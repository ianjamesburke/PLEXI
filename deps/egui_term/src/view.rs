use alacritty_terminal::index::Point as TerminalGridPoint;
use alacritty_terminal::term::cell;
use alacritty_terminal::term::TermMode;
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor};
use egui::emath::GuiRounding;
use egui::epaint::RectShape;
use egui::{CornerRadius, Key};
use egui::Modifiers;
use egui::MouseWheelUnit;
use egui::Shape;
use egui::Widget;
use egui::{Align2, Color32, Painter, Pos2, Rect, Response, Stroke, Vec2};
use egui::{Id, PointerButton};
use std::time::{Duration, Instant};

use alacritty_terminal::grid::Dimensions;
use crate::backend::BackendCommand;
use crate::backend::TerminalBackend;
use crate::backend::{LinkAction, MouseButton, SelectionType};
use crate::bindings::Binding;
use crate::bindings::{BindingAction, BindingsLayout, InputKind};
use crate::font::TerminalFont;
use crate::graphics;
use crate::theme::TerminalTheme;
use crate::types::Size;

const EGUI_TERM_WIDGET_ID_PREFIX: &str = "egui_term::instance::";

#[derive(Debug, Clone)]
enum InputAction {
    BackendCall(BackendCommand),
    WriteToClipboard(String),
    Ignore,
}

#[derive(Debug, Clone)]
struct CopyModeState {
    /// Row index from the top of the visible viewport (0 = top row).
    line_in_viewport: usize,
    col: usize,
    /// Set when visual selection is active; holds the anchor (row, col).
    selection_start: Option<(usize, usize)>,
    /// True when line-wise selection (V) is active.
    line_select: bool,
}

#[derive(Clone)]
pub struct TerminalViewState {
    is_dragged: bool,
    scroll_pixels: f32,
    current_mouse_position_on_grid: TerminalGridPoint,
    last_cursor_toggle: Instant,
    cursor_visible: bool,
    copy_mode: Option<CopyModeState>,
}

impl Default for TerminalViewState {
    fn default() -> Self {
        Self {
            is_dragged: false,
            scroll_pixels: 0.0,
            current_mouse_position_on_grid: TerminalGridPoint::default(),
            last_cursor_toggle: Instant::now(),
            cursor_visible: true,
            copy_mode: None,
        }
    }
}

pub struct TerminalView<'a> {
    widget_id: Id,
    has_focus: bool,
    size: Vec2,
    backend: &'a mut TerminalBackend,
    font: TerminalFont,
    theme: TerminalTheme,
    bindings_layout: BindingsLayout,
}

impl Widget for TerminalView<'_> {
    fn ui(self, ui: &mut egui::Ui) -> Response {
        let (layout, painter) =
            ui.allocate_painter(self.size, egui::Sense::click());

        let widget_id = self.widget_id;
        let mut state = ui.memory(|m| {
            m.data
                .get_temp::<TerminalViewState>(widget_id)
                .unwrap_or_default()
        });

        self.focus(&layout)
            .resize(&layout)
            .process_input(&layout, &mut state)
            .show(&mut state, &layout, &painter);

        ui.memory_mut(|m| m.data.insert_temp(widget_id, state));
        layout
    }
}

impl<'a> TerminalView<'a> {
    pub fn new(ui: &mut egui::Ui, backend: &'a mut TerminalBackend) -> Self {
        let widget_id = ui.make_persistent_id(format!(
            "{}{}",
            EGUI_TERM_WIDGET_ID_PREFIX, backend.id
        ));

        Self {
            widget_id,
            has_focus: false,
            size: ui.available_size(),
            backend,
            font: TerminalFont::default(),
            theme: TerminalTheme::default(),
            bindings_layout: BindingsLayout::new(),
        }
    }

    #[inline]
    pub fn set_theme(mut self, theme: TerminalTheme) -> Self {
        self.theme = theme;
        self
    }

    #[inline]
    pub fn set_font(mut self, font: TerminalFont) -> Self {
        self.font = font;
        self
    }

    #[inline]
    pub fn set_focus(mut self, has_focus: bool) -> Self {
        self.has_focus = has_focus;
        self
    }

    #[inline]
    pub fn set_size(mut self, size: Vec2) -> Self {
        self.size = size;
        self
    }

    #[inline]
    pub fn add_bindings(
        mut self,
        bindings: Vec<(Binding<InputKind>, BindingAction)>,
    ) -> Self {
        self.bindings_layout.add_bindings(bindings);
        self
    }

    fn focus(self, layout: &Response) -> Self {
        if self.has_focus {
            layout.request_focus();
        } else {
            layout.surrender_focus();
        }

        self
    }

    fn resize(self, layout: &Response) -> Self {
        self.backend.process_command(BackendCommand::Resize(
            Size::from(layout.rect.size()),
            self.font.font_measure(&layout.ctx),
        ));

        self
    }

    fn process_input(
        self,
        layout: &Response,
        state: &mut TerminalViewState,
    ) -> Self {
        let has_focus = layout.has_focus();
        let has_pointer = layout.contains_pointer();

        // Process mouse wheel for any pane the cursor is over, regardless of focus.
        // All other input (keyboard, pointer clicks/moves) still requires focus.
        if !has_focus && !has_pointer {
            return self;
        }

        let modifiers = layout.ctx.input(|i| i.modifiers);
        let events = layout.ctx.input(|i| i.events.clone());
        for event in events {
            let mut input_actions = vec![];

            match event {
                egui::Event::Text(_)
                | egui::Event::Key { .. }
                | egui::Event::Copy
                | egui::Event::Paste(_) => {
                    if !has_focus {
                        continue;
                    }
                    state.cursor_visible = true;
                    state.last_cursor_toggle = Instant::now();

                    if state.copy_mode.is_some() {
                        input_actions.extend(process_copy_mode_event(
                            &event,
                            state,
                            self.backend,
                            &layout,
                        ));
                    } else if let egui::Event::Key {
                        key: Key::OpenBracket,
                        pressed: true,
                        modifiers: key_mods,
                        ..
                    } = &event
                    {
                        if key_mods.command && key_mods.shift {
                            let content = self.backend.last_content();
                            if !content.terminal_mode.contains(TermMode::ALT_SCREEN) {
                                let display_offset = content.grid.display_offset();
                                let cursor_line = content.grid.cursor.point.line.0;
                                let col = content.grid.cursor.point.column.0;
                                let screen_lines = content.terminal_size.screen_lines();
                                let line_in_viewport =
                                    ((cursor_line + display_offset as i32).max(0) as usize)
                                        .min(screen_lines.saturating_sub(1));
                                let num_cols = content.terminal_size.columns();
                                state.copy_mode = Some(CopyModeState {
                                    line_in_viewport,
                                    col: col.min(num_cols.saturating_sub(1)),
                                    selection_start: None,
                                    line_select: false,
                                });
                                log::info!("[copy-mode] entered at viewport row {line_in_viewport}, col {col}");
                            }
                            input_actions.push(InputAction::Ignore);
                        } else {
                            input_actions.push(process_keyboard_event(
                                event,
                                self.backend,
                                &self.bindings_layout,
                                modifiers,
                            ));
                        }
                    } else {
                        input_actions.push(process_keyboard_event(
                            event,
                            self.backend,
                            &self.bindings_layout,
                            modifiers,
                        ));
                    }
                },
                egui::Event::MouseWheel { unit, delta, .. } if has_pointer => {
                    input_actions.push(process_mouse_wheel(
                        state,
                        self.font.font_type().size,
                        unit,
                        delta,
                    ))
                },
                egui::Event::PointerButton {
                    button,
                    pressed,
                    modifiers,
                    pos,
                    ..
                } if has_pointer || (!pressed && state.is_dragged) => {
                    // Allow button release to reach the pane even when the
                    // pointer is outside, so is_dragged gets cleared.
                    input_actions.push(process_button_click(
                        state,
                        layout,
                        self.backend,
                        &self.bindings_layout,
                        button,
                        pos,
                        &modifiers,
                        pressed,
                    ))
                },
                egui::Event::PointerMoved(pos)
                    if has_pointer || state.is_dragged =>
                {
                    // Clamp position to pane rect so selection extends to
                    // the nearest edge when the cursor leaves the pane.
                    let clamped = Pos2::new(
                        pos.x.clamp(layout.rect.min.x, layout.rect.max.x),
                        pos.y.clamp(layout.rect.min.y, layout.rect.max.y),
                    );
                    input_actions = process_mouse_move(
                        state,
                        layout,
                        self.backend,
                        clamped,
                        &modifiers,
                    )
                },
                _ => {},
            };

            for action in input_actions {
                match action {
                    InputAction::BackendCall(cmd) => {
                        self.backend.process_command(cmd);
                    },
                    InputAction::WriteToClipboard(data) => {
                        layout.ctx.copy_text(data);
                    },
                    InputAction::Ignore => {},
                }
            }
        }

        // Auto-scroll during drag selection at viewport edges.
        // Runs every frame (not just on PointerMoved) so scrolling continues
        // while the mouse is held stationary near an edge.
        if state.is_dragged {
            if let Some(pos) = layout.ctx.input(|i| i.pointer.latest_pos()) {
                let cell_height = (self.backend.last_content().terminal_size.cell_height as f32).max(1.0);
                let scroll_lines = if pos.y < layout.rect.min.y {
                    let overshoot = layout.rect.min.y - pos.y;
                    ((overshoot / cell_height).ceil() as i32).max(1)
                } else if pos.y > layout.rect.max.y {
                    let overshoot = pos.y - layout.rect.max.y;
                    -(((overshoot / cell_height).ceil() as i32).max(1))
                } else {
                    0
                };

                if scroll_lines != 0 {
                    self.backend
                        .process_command(BackendCommand::Scroll(scroll_lines));
                    layout
                        .ctx
                        .request_repaint_after(Duration::from_millis(150));
                }

                // Always update selection while dragging, clamping to
                // the pane rect so it extends to the nearest edge.
                let clamped_x = pos.x.clamp(layout.rect.min.x, layout.rect.max.x);
                let clamped_y = pos.y.clamp(layout.rect.min.y, layout.rect.max.y);
                self.backend.process_command(BackendCommand::SelectUpdate(
                    clamped_x - layout.rect.min.x,
                    clamped_y - layout.rect.min.y,
                    layout.ctx.pixels_per_point(),
                ));
            }
        }

        // Check for link hover every frame (not just on mouse move) so that
        // pressing/releasing Cmd updates the hyperlink state immediately.
        if has_pointer && modifiers.command_only() {
            self.backend.process_command(BackendCommand::ProcessLink(
                LinkAction::Hover,
                state.current_mouse_position_on_grid,
            ));
        } else if has_pointer {
            self.backend.process_command(BackendCommand::ProcessLink(
                LinkAction::Clear,
                state.current_mouse_position_on_grid,
            ));
        }

        self
    }

    fn show(
        self,
        state: &mut TerminalViewState,
        layout: &Response,
        painter: &Painter,
    ) {
        let content = self.backend.sync();
        let layout_min = layout.rect.min;
        let layout_max = layout.rect.max;
        // Use font-metric-based cell dimensions from the committed terminal
        // size rather than recomputing from the pane rect. Recomputing gives
        // pane_width / committed_cols, which stretches cells continuously as
        // the pane resizes between SIGWINCH events — causing the jitter.
        let cell_width = content.terminal_size.cell_width as f32;
        let cell_height = content.terminal_size.cell_height as f32;
        let global_bg =
            self.theme.get_color(Color::Named(NamedColor::Background));

        // Show pointer cursor when hovering a hyperlink
        if content.hovered_hyperlink.as_ref().is_some_and(|r| {
            r.contains(&state.current_mouse_position_on_grid)
        }) {
            layout.ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
        }

        let mut shapes = vec![Shape::Rect(RectShape::filled(
            Rect::from_min_max(layout_min, layout_max),
            CornerRadius::ZERO,
            global_bg,
        ))];

        for indexed in content.grid.display_iter() {
            let flags = indexed.cell.flags;
            let is_wide_char_spacer =
                flags.contains(cell::Flags::WIDE_CHAR_SPACER);
            if is_wide_char_spacer {
                continue;
            }

            let is_wide_char = flags.contains(cell::Flags::WIDE_CHAR);
            let is_inverse = flags.contains(cell::Flags::INVERSE);
            let is_dim =
                flags.intersects(cell::Flags::DIM | cell::Flags::DIM_BOLD);
            let is_selected = content
                .selectable_range
                .is_some_and(|r| r.contains(indexed.point));
            let is_hovered_hyperling =
                content.hovered_hyperlink.as_ref().is_some_and(|r| {
                    r.contains(&indexed.point)
                        && r.contains(&state.current_mouse_position_on_grid)
                });

            let x = layout_min.x + (cell_width * indexed.point.column.0 as f32);
            let line_num =
                indexed.point.line.0 + content.grid.display_offset() as i32;
            let y = layout_min.y + (cell_height * line_num as f32);

            let mut fg = self.theme.get_color(indexed.fg);
            let mut bg = self.theme.get_color(indexed.bg);
            let cell_width = if is_wide_char {
                cell_width * 2.0
            } else {
                cell_width
            };
            let cell_rect = Rect::from_min_max(
                Pos2::new(x, y),
                Pos2::new(
                    (x + cell_width).min(layout_max.x),
                    (y + cell_height).min(layout_max.y),
                ),
            )
            .round_to_pixels(painter.pixels_per_point());

            if is_dim {
                fg = fg.linear_multiply(0.7);
            }

            if is_inverse {
                std::mem::swap(&mut fg, &mut bg);
            }

            if is_selected {
                // Uniform selection highlight: fixed tint over the global
                // background so the band looks consistent regardless of
                // per-cell fg/bg colors from ANSI escapes.
                bg = Color32::from_rgba_unmultiplied(70, 130, 210, 90);
            }

            if global_bg != bg {
                shapes.push(Shape::Rect(RectShape::filled(
                    cell_rect,
                    CornerRadius::ZERO,
                    bg,
                )));
            }

            // Handle hovered hyperlink underline
            if is_hovered_hyperling {
                let underline_height = y + cell_height;
                shapes.push(Shape::LineSegment {
                    points: [
                        Pos2::new(x, underline_height),
                        Pos2::new(x + cell_width, underline_height),
                    ],
                    stroke: Stroke::new(cell_height * 0.15, fg).into(),
                });
            }

            // Handle cursor rendering
            if content.grid.cursor.point == indexed.point
                && content.cursor_visible
                && content.cursor_shape != CursorShape::Hidden
            {
                let cursor_color = self.theme.get_color(content.cursor.fg);

                if self.has_focus {
                    // Focused: blink the cursor
                    let blink_interval = Duration::from_millis(530);
                    let elapsed = state.last_cursor_toggle.elapsed();
                    if elapsed >= blink_interval {
                        state.cursor_visible = !state.cursor_visible;
                        state.last_cursor_toggle = Instant::now();
                    }
                    if state.cursor_visible {
                        match content.cursor_shape {
                            CursorShape::Block | CursorShape::HollowBlock => {
                                shapes.push(Shape::Rect(RectShape::filled(
                                    cell_rect,
                                    CornerRadius::default(),
                                    cursor_color,
                                )));
                            }
                            CursorShape::Beam => {
                                shapes.push(Shape::Rect(RectShape::filled(
                                    Rect::from_min_size(
                                        Pos2::new(x, y),
                                        Vec2::new(2.0, cell_height),
                                    ),
                                    CornerRadius::default(),
                                    cursor_color,
                                )));
                            }
                            CursorShape::Underline => {
                                shapes.push(Shape::Rect(RectShape::filled(
                                    Rect::from_min_size(
                                        Pos2::new(x, y + cell_height - 2.0),
                                        Vec2::new(cell_width, 2.0),
                                    ),
                                    CornerRadius::default(),
                                    cursor_color,
                                )));
                            }
                            CursorShape::Hidden => {}
                        }
                    }
                    let remaining = blink_interval.saturating_sub(state.last_cursor_toggle.elapsed());
                    painter.ctx().request_repaint_after(remaining);
                } else {
                    // Unfocused: hollow outline, no blink
                    shapes.push(Shape::Rect(RectShape::stroke(
                        cell_rect,
                        CornerRadius::default(),
                        Stroke::new(1.0, cursor_color),
                        egui::StrokeKind::Inside,
                    )));
                }
            }

            // Draw text content
            if indexed.c != ' ' && indexed.c != '\t' {
                let glyph_fg = if content.grid.cursor.point == indexed.point
                    && self.has_focus
                    && content.cursor_visible
                    && matches!(content.cursor_shape, CursorShape::Block | CursorShape::HollowBlock)
                    && state.cursor_visible
                {
                    bg
                } else {
                    fg
                };

                if graphics::maybe_push_graphics_element(
                    &mut shapes,
                    indexed.c,
                    cell_rect,
                    glyph_fg,
                    painter.pixels_per_point(),
                ) {
                    continue;
                }

                shapes.push(Shape::text(
                    &painter.fonts(|c| c.clone()),
                    Pos2 {
                        x: cell_rect.center().x,
                        y: cell_rect.min.y,
                    },
                    Align2::CENTER_TOP,
                    indexed.c,
                    self.font.font_type(),
                    glyph_fg,
                ));
            }
        }

        painter.extend(shapes);

        // Copy-mode: block cursor highlight
        if let Some(ref cm) = state.copy_mode {
            let cx = layout_min.x + cm.col as f32 * cell_width;
            let cy = layout_min.y + cm.line_in_viewport as f32 * cell_height;
            let cursor_rect = Rect::from_min_size(
                Pos2::new(cx, cy),
                Vec2::new(cell_width, cell_height),
            );
            painter.rect_filled(
                cursor_rect,
                CornerRadius::ZERO,
                Color32::from_rgba_unmultiplied(220, 220, 220, 140),
            );

            // [COPY] badge — top-right corner
            let badge_label = "[COPY]";
            let badge_font = egui::FontId::monospace(11.0);
            let badge_color = Color32::from_rgba_unmultiplied(230, 230, 160, 200);
            let galley = painter.layout_no_wrap(
                badge_label.to_string(),
                badge_font.clone(),
                badge_color,
            );
            let pad = 4.0;
            let bw = galley.size().x + pad * 2.0;
            let bh = galley.size().y + pad;
            let badge_rect = Rect::from_min_size(
                Pos2::new(layout_max.x - bw - 4.0, layout_min.y + 2.0),
                Vec2::new(bw, bh),
            );
            painter.rect_filled(
                badge_rect,
                egui::CornerRadius::same(3),
                Color32::from_rgba_unmultiplied(40, 40, 20, 200),
            );
            painter.text(
                badge_rect.center(),
                Align2::CENTER_CENTER,
                badge_label,
                badge_font,
                badge_color,
            );
        }

        let offset = content.grid.display_offset();
        if offset > 0 {
            painter.text(
                Pos2::new(layout_max.x - 6.0, layout_max.y - 4.0),
                Align2::RIGHT_BOTTOM,
                format!("↑ {}", offset),
                egui::FontId::monospace(11.0),
                Color32::from_rgba_unmultiplied(200, 200, 200, 110),
            );
        }
    }
}

fn process_keyboard_event(
    event: egui::Event,
    backend: &TerminalBackend,
    bindings_layout: &BindingsLayout,
    modifiers: Modifiers,
) -> InputAction {
    match event {
        egui::Event::Text(text) => {
            process_text_event(&text, modifiers, backend, bindings_layout)
        },
        egui::Event::Paste(text) => InputAction::BackendCall(
            #[cfg(not(any(target_os = "ios", target_os = "macos")))]
            if modifiers.contains(Modifiers::COMMAND | Modifiers::SHIFT) {
                BackendCommand::Write(text.as_bytes().to_vec())
            } else {
                // Hotfix - Send ^V when there's not selection on view.
                BackendCommand::Write([0x16].to_vec())
            },
            #[cfg(any(target_os = "ios", target_os = "macos"))]
            {
                BackendCommand::Write(text.as_bytes().to_vec())
            },
        ),
        egui::Event::Copy => {
            let copy_if_nonempty = |content: String| -> InputAction {
                if content.trim().is_empty() { InputAction::Ignore } else { InputAction::WriteToClipboard(content) }
            };
            #[cfg(not(any(target_os = "ios", target_os = "macos")))]
            if modifiers.contains(Modifiers::COMMAND | Modifiers::SHIFT) {
                copy_if_nonempty(backend.selectable_content())
            } else {
                // Hotfix - Send ^C when there's not selection on view.
                InputAction::BackendCall(BackendCommand::Write([0x3].to_vec()))
            }
            #[cfg(any(target_os = "ios", target_os = "macos"))]
            copy_if_nonempty(backend.selectable_content())
        },
        egui::Event::Key {
            key,
            pressed,
            modifiers,
            ..
        } => process_keyboard_key(
            backend,
            bindings_layout,
            key,
            modifiers,
            pressed,
        ),
        _ => InputAction::Ignore,
    }
}

fn process_text_event(
    text: &str,
    modifiers: Modifiers,
    backend: &TerminalBackend,
    bindings_layout: &BindingsLayout,
) -> InputAction {
    // On macOS, Option+letter fires a Text event with a unicode character (e.g. ƒ for Option+f).
    // The Key event with Modifiers::ALT handles sending the correct escape sequence, so suppress
    // the Text event here to avoid writing the unicode character to the PTY.
    if modifiers.alt {
        return InputAction::Ignore;
    }
    // Suppress Text events when Cmd is held — these are shortcut chars (e.g. '{' for Cmd+Shift+[)
    // and should never be written to the PTY. The corresponding Key event handles the binding.
    if modifiers.command {
        return InputAction::Ignore;
    }

    if let Some(key) = Key::from_name(text) {
        if bindings_layout.get_action(
            InputKind::KeyCode(key),
            modifiers,
            backend.last_content().terminal_mode,
        ) == BindingAction::Ignore
        {
            InputAction::BackendCall(BackendCommand::Write(
                text.as_bytes().to_vec(),
            ))
        } else {
            InputAction::Ignore
        }
    } else {
        InputAction::BackendCall(BackendCommand::Write(
            text.as_bytes().to_vec(),
        ))
    }
}

fn process_keyboard_key(
    backend: &TerminalBackend,
    bindings_layout: &BindingsLayout,
    key: Key,
    modifiers: Modifiers,
    pressed: bool,
) -> InputAction {
    if !pressed {
        return InputAction::Ignore;
    }

    let terminal_mode = backend.last_content().terminal_mode;
    let binding_action = bindings_layout.get_action(
        InputKind::KeyCode(key),
        modifiers,
        terminal_mode,
    );

    match binding_action {
        BindingAction::Char(c) => {
            let mut buf = [0, 0, 0, 0];
            let str = c.encode_utf8(&mut buf);
            InputAction::BackendCall(BackendCommand::Write(
                str.as_bytes().to_vec(),
            ))
        },
        BindingAction::Esc(seq) => InputAction::BackendCall(
            BackendCommand::Write(seq.as_bytes().to_vec()),
        ),
        BindingAction::ScrollLines(delta) => {
            InputAction::BackendCall(BackendCommand::Scroll(delta))
        },
        BindingAction::ScrollPage(dir) => {
            let page = backend.last_content().terminal_size.screen_lines() as i32 - 1;
            InputAction::BackendCall(BackendCommand::Scroll(page * dir))
        },
        BindingAction::ScrollToTop => {
            InputAction::BackendCall(BackendCommand::ScrollToTop)
        },
        BindingAction::ScrollToBottom => {
            InputAction::BackendCall(BackendCommand::ScrollToBottom)
        },
        _ => InputAction::Ignore,
    }
}

fn process_copy_mode_event(
    event: &egui::Event,
    state: &mut TerminalViewState,
    backend: &TerminalBackend,
    layout: &Response,
) -> Vec<InputAction> {
    let content = backend.last_content();
    let screen_lines = content.terminal_size.screen_lines();
    let num_cols = content.terminal_size.columns();
    let cell_width = content.terminal_size.cell_width as f32;
    let cell_height = content.terminal_size.cell_height as f32;
    let ppp = layout.ctx.pixels_per_point();
    let page = (screen_lines as i32 - 1).max(1);

    let cursor_px = |cm: &CopyModeState| -> (f32, f32) {
        (cm.col as f32 * cell_width, cm.line_in_viewport as f32 * cell_height)
    };

    let mut actions: Vec<InputAction> = vec![];

    let advance_selection = |cm: &CopyModeState, actions: &mut Vec<InputAction>, ppp: f32, cell_width: f32, cell_height: f32| {
        if cm.selection_start.is_some() {
            let (x, y) = (cm.col as f32 * cell_width, cm.line_in_viewport as f32 * cell_height);
            actions.push(InputAction::BackendCall(BackendCommand::SelectUpdate(x, y, ppp)));
        }
    };

    match event {
        // ---- Exit ----
        egui::Event::Text(t) if t == "q" => {
            log::info!("[copy-mode] exit via q");
            state.copy_mode = None;
            actions.push(InputAction::BackendCall(BackendCommand::ClearSelection));
        }
        egui::Event::Key { key: Key::Escape, pressed: true, .. } => {
            let cm = state.copy_mode.as_mut().unwrap();
            if cm.selection_start.is_some() {
                cm.selection_start = None;
                cm.line_select = false;
                actions.push(InputAction::BackendCall(BackendCommand::ClearSelection));
                log::info!("[copy-mode] selection cancelled");
            } else {
                log::info!("[copy-mode] exit via Esc");
                state.copy_mode = None;
                actions.push(InputAction::BackendCall(BackendCommand::ClearSelection));
            }
        }
        // ---- Yank ----
        egui::Event::Text(t) if t == "y" => {
            let text = backend.selectable_content();
            if !text.trim().is_empty() {
                actions.push(InputAction::WriteToClipboard(text));
                log::info!("[copy-mode] yanked selection to clipboard");
            }
            state.copy_mode = None;
            actions.push(InputAction::BackendCall(BackendCommand::ClearSelection));
        }
        // ---- Movement: left ----
        egui::Event::Text(t) if t == "h" => {
            let cm = state.copy_mode.as_mut().unwrap();
            if cm.col > 0 { cm.col -= 1; }
            advance_selection(cm, &mut actions, ppp, cell_width, cell_height);
        }
        egui::Event::Key { key: Key::ArrowLeft, pressed: true, .. } => {
            let cm = state.copy_mode.as_mut().unwrap();
            if cm.col > 0 { cm.col -= 1; }
            advance_selection(cm, &mut actions, ppp, cell_width, cell_height);
        }
        // ---- Movement: right ----
        egui::Event::Text(t) if t == "l" => {
            let cm = state.copy_mode.as_mut().unwrap();
            if cm.col + 1 < num_cols { cm.col += 1; }
            advance_selection(cm, &mut actions, ppp, cell_width, cell_height);
        }
        egui::Event::Key { key: Key::ArrowRight, pressed: true, .. } => {
            let cm = state.copy_mode.as_mut().unwrap();
            if cm.col + 1 < num_cols { cm.col += 1; }
            advance_selection(cm, &mut actions, ppp, cell_width, cell_height);
        }
        // ---- Movement: up ----
        egui::Event::Text(t) if t == "k" => {
            let cm = state.copy_mode.as_mut().unwrap();
            if cm.line_in_viewport > 0 {
                cm.line_in_viewport -= 1;
            } else {
                actions.push(InputAction::BackendCall(BackendCommand::Scroll(1)));
            }
            advance_selection(cm, &mut actions, ppp, cell_width, cell_height);
        }
        egui::Event::Key { key: Key::ArrowUp, pressed: true, modifiers: Modifiers { command: false, shift: false, .. }, .. } => {
            let cm = state.copy_mode.as_mut().unwrap();
            if cm.line_in_viewport > 0 {
                cm.line_in_viewport -= 1;
            } else {
                actions.push(InputAction::BackendCall(BackendCommand::Scroll(1)));
            }
            advance_selection(cm, &mut actions, ppp, cell_width, cell_height);
        }
        // ---- Movement: down ----
        egui::Event::Text(t) if t == "j" => {
            let cm = state.copy_mode.as_mut().unwrap();
            if cm.line_in_viewport + 1 < screen_lines {
                cm.line_in_viewport += 1;
            } else {
                actions.push(InputAction::BackendCall(BackendCommand::Scroll(-1)));
            }
            advance_selection(cm, &mut actions, ppp, cell_width, cell_height);
        }
        egui::Event::Key { key: Key::ArrowDown, pressed: true, modifiers: Modifiers { command: false, shift: false, .. }, .. } => {
            let cm = state.copy_mode.as_mut().unwrap();
            if cm.line_in_viewport + 1 < screen_lines {
                cm.line_in_viewport += 1;
            } else {
                actions.push(InputAction::BackendCall(BackendCommand::Scroll(-1)));
            }
            advance_selection(cm, &mut actions, ppp, cell_width, cell_height);
        }
        // ---- Page up ----
        egui::Event::Key { key: Key::PageUp, pressed: true, .. } => {
            let cm = state.copy_mode.as_mut().unwrap();
            actions.push(InputAction::BackendCall(BackendCommand::Scroll(page)));
            cm.line_in_viewport = cm.line_in_viewport.saturating_sub(page as usize);
            advance_selection(cm, &mut actions, ppp, cell_width, cell_height);
        }
        // ---- Page down ----
        egui::Event::Key { key: Key::PageDown, pressed: true, .. } => {
            let cm = state.copy_mode.as_mut().unwrap();
            actions.push(InputAction::BackendCall(BackendCommand::Scroll(-page)));
            cm.line_in_viewport = (cm.line_in_viewport + page as usize).min(screen_lines.saturating_sub(1));
            advance_selection(cm, &mut actions, ppp, cell_width, cell_height);
        }
        // ---- Jump to top (g) ----
        egui::Event::Text(t) if t == "g" => {
            let cm = state.copy_mode.as_mut().unwrap();
            actions.push(InputAction::BackendCall(BackendCommand::ScrollToTop));
            cm.line_in_viewport = 0;
            cm.col = 0;
            advance_selection(cm, &mut actions, ppp, cell_width, cell_height);
            log::info!("[copy-mode] jumped to top");
        }
        // ---- Jump to bottom (G) ----
        egui::Event::Text(t) if t == "G" => {
            let cm = state.copy_mode.as_mut().unwrap();
            actions.push(InputAction::BackendCall(BackendCommand::ScrollToBottom));
            cm.line_in_viewport = screen_lines.saturating_sub(1);
            advance_selection(cm, &mut actions, ppp, cell_width, cell_height);
            log::info!("[copy-mode] jumped to bottom");
        }
        // ---- Start visual selection (v) ----
        egui::Event::Text(t) if t == "v" => {
            let cm = state.copy_mode.as_mut().unwrap();
            cm.selection_start = Some((cm.line_in_viewport, cm.col));
            cm.line_select = false;
            let (x, y) = cursor_px(cm);
            actions.push(InputAction::BackendCall(BackendCommand::SelectStart(
                SelectionType::Simple, x, y, ppp,
            )));
            log::info!("[copy-mode] visual selection started at ({}, {})", cm.line_in_viewport, cm.col);
        }
        // ---- Start line selection (V) ----
        egui::Event::Text(t) if t == "V" => {
            let cm = state.copy_mode.as_mut().unwrap();
            cm.selection_start = Some((cm.line_in_viewport, 0));
            cm.line_select = true;
            let (x, y) = cursor_px(cm);
            actions.push(InputAction::BackendCall(BackendCommand::SelectStart(
                SelectionType::Lines, x, y, ppp,
            )));
            log::info!("[copy-mode] line selection started at row {}", cm.line_in_viewport);
        }
        _ => {}
    }

    actions
}

fn process_mouse_wheel(
    state: &mut TerminalViewState,
    font_size: f32,
    unit: MouseWheelUnit,
    delta: Vec2,
) -> InputAction {
    match unit {
        MouseWheelUnit::Line => {
            let lines = delta.y.signum() * delta.y.abs().ceil();
            InputAction::BackendCall(BackendCommand::Scroll(lines as i32))
        },
        MouseWheelUnit::Point => {
            state.scroll_pixels -= delta.y;
            let lines = (state.scroll_pixels / font_size).trunc();
            state.scroll_pixels %= font_size;
            if lines != 0.0 {
                InputAction::BackendCall(BackendCommand::Scroll(-lines as i32))
            } else {
                InputAction::Ignore
            }
        },
        MouseWheelUnit::Page => InputAction::Ignore,
    }
}

fn process_button_click(
    state: &mut TerminalViewState,
    layout: &Response,
    backend: &TerminalBackend,
    bindings_layout: &BindingsLayout,
    button: PointerButton,
    position: Pos2,
    modifiers: &Modifiers,
    pressed: bool,
) -> InputAction {
    match button {
        PointerButton::Primary => process_left_button(
            state,
            layout,
            backend,
            bindings_layout,
            position,
            modifiers,
            pressed,
        ),
        _ => InputAction::Ignore,
    }
}

fn process_left_button(
    state: &mut TerminalViewState,
    layout: &Response,
    backend: &TerminalBackend,
    bindings_layout: &BindingsLayout,
    position: Pos2,
    modifiers: &Modifiers,
    pressed: bool,
) -> InputAction {
    let terminal_mode = backend.last_content().terminal_mode;
    if terminal_mode.intersects(TermMode::MOUSE_MODE) {
        InputAction::BackendCall(BackendCommand::MouseReport(
            MouseButton::LeftButton,
            *modifiers,
            state.current_mouse_position_on_grid,
            pressed,
        ))
    } else if pressed {
        process_left_button_pressed(state, layout, position)
    } else {
        process_left_button_released(
            state,
            layout,
            backend,
            bindings_layout,
            position,
            modifiers,
        )
    }
}

fn process_left_button_pressed(
    state: &mut TerminalViewState,
    layout: &Response,
    position: Pos2,
) -> InputAction {
    state.is_dragged = true;
    InputAction::BackendCall(build_start_select_command(layout, position))
}

fn process_left_button_released(
    state: &mut TerminalViewState,
    layout: &Response,
    backend: &TerminalBackend,
    bindings_layout: &BindingsLayout,
    position: Pos2,
    modifiers: &Modifiers,
) -> InputAction {
    state.is_dragged = false;
    if layout.double_clicked() || layout.triple_clicked() {
        InputAction::BackendCall(build_start_select_command(layout, position))
    } else {
        let terminal_content = backend.last_content();
        let binding_action = bindings_layout.get_action(
            InputKind::Mouse(PointerButton::Primary),
            *modifiers,
            terminal_content.terminal_mode,
        );

        if binding_action == BindingAction::LinkOpen {
            InputAction::BackendCall(BackendCommand::ProcessLink(
                LinkAction::Open,
                state.current_mouse_position_on_grid,
            ))
        } else {
            InputAction::Ignore
        }
    }
}

fn build_start_select_command(
    layout: &Response,
    cursor_position: Pos2,
) -> BackendCommand {
    let selection_type = if layout.double_clicked() {
        SelectionType::Semantic
    } else if layout.triple_clicked() {
        SelectionType::Lines
    } else {
        SelectionType::Simple
    };

    BackendCommand::SelectStart(
        selection_type,
        cursor_position.x - layout.rect.min.x,
        cursor_position.y - layout.rect.min.y,
        layout.ctx.pixels_per_point(),
    )
}

fn process_mouse_move(
    state: &mut TerminalViewState,
    layout: &Response,
    backend: &TerminalBackend,
    position: Pos2,
    modifiers: &Modifiers,
) -> Vec<InputAction> {
    let terminal_content = backend.last_content();
    let cursor_x = position.x - layout.rect.min.x;
    let cursor_y = position.y - layout.rect.min.y;
    state.current_mouse_position_on_grid = TerminalBackend::selection_point(
        cursor_x,
        cursor_y,
        &terminal_content.terminal_size,
        terminal_content.grid.display_offset(),
        layout.ctx.pixels_per_point(),
    );

    let mut actions = vec![];
    // Handle command or selection update based on terminal mode and modifiers
    if state.is_dragged {
        let terminal_mode = terminal_content.terminal_mode;
        let cmd = if terminal_mode.contains(TermMode::MOUSE_MOTION)
            && modifiers.is_none()
        {
            InputAction::BackendCall(BackendCommand::MouseReport(
                MouseButton::LeftMove,
                *modifiers,
                state.current_mouse_position_on_grid,
                true,
            ))
        } else {
            InputAction::BackendCall(BackendCommand::SelectUpdate(
                cursor_x,
                cursor_y,
                layout.ctx.pixels_per_point(),
            ))
        };

        actions.push(cmd);
    }

    // Link hover is handled per-frame in process_input, not per mouse-move.

    actions
}
