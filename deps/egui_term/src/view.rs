use alacritty_terminal::index::Point as TerminalGridPoint;
use alacritty_terminal::grid::Dimensions;
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
use egui::{Align2, Painter, Pos2, Rect, Response, Stroke, Vec2};
use egui::{Id, PointerButton};
use std::time::{Duration, Instant};

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

#[derive(Clone)]
pub struct TerminalViewState {
    is_dragged: bool,
    scroll_pixels: f32,
    current_mouse_position_on_grid: TerminalGridPoint,
    last_cursor_toggle: Instant,
    cursor_visible: bool,
}

impl Default for TerminalViewState {
    fn default() -> Self {
        Self {
            is_dragged: false,
            scroll_pixels: 0.0,
            current_mouse_position_on_grid: TerminalGridPoint::default(),
            last_cursor_toggle: Instant::now(),
            cursor_visible: true,
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
                    input_actions.push(process_keyboard_event(
                        event,
                        self.backend,
                        &self.bindings_layout,
                        modifiers,
                    ))
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
                } if has_pointer => input_actions.push(process_button_click(
                    state,
                    layout,
                    self.backend,
                    &self.bindings_layout,
                    button,
                    pos,
                    &modifiers,
                    pressed,
                )),
                egui::Event::PointerMoved(pos) if has_pointer => {
                    input_actions = process_mouse_move(
                        state,
                        layout,
                        self.backend,
                        pos,
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

        // Handle file drops — paste paths into PTY
        let dropped_files = layout.ctx.input(|i| i.raw.dropped_files.clone());
        for file in dropped_files {
            if let Some(path) = &file.path {
                let path_str = path.display().to_string();
                let escaped = if path_str
                    .contains(|c: char| c.is_whitespace() || "\"'\\()&|;$`!#".contains(c))
                {
                    format!("'{}'", path_str.replace('\'', "'\\''"))
                } else {
                    path_str
                };
                self.backend
                    .process_command(BackendCommand::Write(escaped.as_bytes().to_vec()));
            }
        }

        // Auto-scroll during drag selection at viewport edges.
        // Runs every frame (not just on PointerMoved) so scrolling continues
        // while the mouse is held stationary near an edge.
        if state.is_dragged {
            if let Some(pos) = layout.ctx.input(|i| i.pointer.latest_pos()) {
                let edge_zone = 20.0_f32;
                let scroll_lines = if pos.y < layout.rect.min.y + edge_zone {
                    1 // scroll up (show earlier content)
                } else if pos.y > layout.rect.max.y - edge_zone {
                    -1 // scroll down (show later content)
                } else {
                    0
                };

                if scroll_lines != 0 {
                    self.backend
                        .process_command(BackendCommand::Scroll(scroll_lines));
                    let cursor_x = pos.x - layout.rect.min.x;
                    let cursor_y = pos.y - layout.rect.min.y;
                    self.backend
                        .process_command(BackendCommand::SelectUpdate(cursor_x, cursor_y));
                    layout
                        .ctx
                        .request_repaint_after(Duration::from_millis(50));
                }
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
        let cols = content.terminal_size.columns() as f32;
        let lines = content.terminal_size.screen_lines() as f32;
        let cell_height = layout.rect.height() / lines;
        let cell_width = layout.rect.width() / cols;
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

            if is_inverse || is_selected {
                std::mem::swap(&mut fg, &mut bg);
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
            #[cfg(not(any(target_os = "ios", target_os = "macos")))]
            if modifiers.contains(Modifiers::COMMAND | Modifiers::SHIFT) {
                let content = backend.selectable_content();
                InputAction::WriteToClipboard(content)
            } else {
                // Hotfix - Send ^C when there's not selection on view.
                InputAction::BackendCall(BackendCommand::Write([0x3].to_vec()))
            }
            #[cfg(any(target_os = "ios", target_os = "macos"))]
            {
                let content = backend.selectable_content();
                InputAction::WriteToClipboard(content)
            }
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
        _ => InputAction::Ignore,
    }
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
                cursor_x, cursor_y,
            ))
        };

        actions.push(cmd);
    }

    // Link hover is handled per-frame in process_input, not per mouse-move.

    actions
}
