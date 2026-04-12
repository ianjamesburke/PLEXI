use crate::agent_mode::AgentMode;
use crate::app_trait::{AppRenderContext, SurfaceLayer, SurfaceMode, APP_DIM_OPACITY};
use crate::media_cache::MediaCache;
use crate::pane::TerminalPane;
use crate::theme::{self, Colors};
use egui::{Color32, Stroke, Vec2};
use egui_term::{BackendCommand, TerminalTheme, TerminalView};
use egui_tiles::{Behavior, SimplificationOptions, TabState, TileId, Tiles, UiResponse};
use std::cell::RefCell;
use std::collections::HashMap;

/// Default height in logical pixels for the terminal command bar when an app is active.
const COMMAND_BAR_HEIGHT: f32 = 140.0;

pub type PaneId = u64;

const DOT_RADIUS: f32 = 4.0;
const DOT_SPACING: f32 = 12.0;
const DOT_LEFT_MARGIN: f32 = 6.0;
pub(crate) const TAB_DOT_RESERVED_HEIGHT: f32 = 14.0;

pub(crate) fn paint_tab_dots(
    painter: &egui::Painter,
    left_x: f32,
    center_y: f32,
    active_idx: usize,
    count: usize,
    active_color: Color32,
    inactive_color: Color32,
) {
    let start_x = left_x + DOT_LEFT_MARGIN;
    for i in 0..count {
        let cx = start_x + (i as f32) * DOT_SPACING + DOT_RADIUS;
        let color = if i == active_idx { active_color } else { inactive_color };
        painter.circle_filled(egui::pos2(cx, center_y), DOT_RADIUS, color);
    }
}

pub struct PlexiBehavior<'a> {
    pub panes: &'a mut HashMap<PaneId, TerminalPane>,
    pub focused_tile: Option<TileId>,
    pub theme: TerminalTheme,
    pub new_focused: Option<TileId>,
    pub close_exited: Option<TileId>,
    pub tab_info: HashMap<TileId, (usize, usize)>, // tile_id -> (index, count)
    pub zoomed_pane: Option<TileId>,
    pub colors: Colors,
    pub pane_names: HashMap<PaneId, String>,
    pub drag_cursor_pos: Option<egui::Pos2>,
    /// Shared media cache borrowed from `PlexiApp`.
    pub media_cache: &'a RefCell<MediaCache>,
}

impl Behavior<PaneId> for PlexiBehavior<'_> {
    fn pane_ui(
        &mut self,
        ui: &mut egui::Ui,
        tile_id: TileId,
        pane_id: &mut PaneId,
    ) -> UiResponse {
        // Detect clicks or file drags for focus (skip when a pane is zoomed — input belongs to the overlay)
        let is_click = ui.input(|i| i.pointer.any_pressed()) && ui.rect_contains_pointer(ui.max_rect());
        let is_drag_hovering = match self.drag_cursor_pos {
            Some(pos) => ui.max_rect().contains(pos),
            None => ui.input(|i| !i.raw.hovered_files.is_empty()) && ui.rect_contains_pointer(ui.max_rect()),
        };
        if self.zoomed_pane.is_none() && (is_click || is_drag_hovering) {
            self.new_focused = Some(tile_id);
        }

        let is_focused = self.focused_tile == Some(tile_id);

        // Handle file drops — use the same geometric hit test as hover detection
        // so the drop target matches the visual focus with no frame delay.
        //
        // Apps in AppActive mode get first crack at the drop via their declared
        // DropTargets (see App::handle_drop). If no drop target claims the
        // position, or the pane is in FullTerminal mode, fall back to the
        // default behaviour: paste the shell-escaped path(s) into the terminal.
        if is_drag_hovering {
            let dropped = ui.input(|i| i.raw.dropped_files.clone());
            if !dropped.is_empty() {
                let paths: Vec<std::path::PathBuf> = dropped
                    .iter()
                    .filter_map(|f| f.path.clone())
                    .collect();

                // Try the active app first. The Frame applied below uses an
                // inner_margin of 8, so the app's local origin is offset from
                // the pane's outer rect by (8, 8). Mirror that offset here so
                // the position we pass to handle_drop matches the coordinate
                // space of the DropTarget commands the app emitted.
                let app_consumed = if let Some(pane) = self.panes.get_mut(pane_id) {
                    if matches!(pane.surface_mode, SurfaceMode::AppActive)
                        && pane.active_app.is_some()
                    {
                        let drop_pos = self
                            .drag_cursor_pos
                            .or_else(|| ui.input(|i| i.pointer.interact_pos()))
                            .unwrap_or_else(|| ui.max_rect().center());
                        let inner_origin = ui.max_rect().min + egui::vec2(8.0, 8.0);
                        let local_pos = drop_pos - inner_origin.to_vec2();
                        let app = pane.active_app.as_mut().unwrap();
                        app.handle_drop(local_pos, &paths)
                    } else {
                        false
                    }
                } else {
                    false
                };

                if !app_consumed {
                    if let Some(pane) = self.panes.get_mut(pane_id) {
                        for path in &paths {
                            let path_str = path.display().to_string();
                            let escaped = if path_str.contains(|c: char| {
                                c.is_whitespace() || "\"'\\()&|;$`!#".contains(c)
                            }) {
                                format!("'{}'", path_str.replace('\'', "'\\''"))
                            } else {
                                path_str
                            };
                            pane.backend.process_command(BackendCommand::Write(
                                escaped.as_bytes().to_vec(),
                            ));
                        }
                    }
                }
            }
        }

        // When any pane is zoomed, render ALL panes as dark placeholders.
        // The zoomed pane is rendered separately in the overlay (app.rs).
        if self.zoomed_pane.is_some() {
            egui::Frame::new()
                .fill(self.colors.bg_darkest)
                .inner_margin(egui::Margin::same(8))
                .show(ui, |_ui| {});
            return UiResponse::None;
        }

        if let Some(pane) = self.panes.get_mut(pane_id) {
            egui::Frame::new()
                .fill(self.colors.terminal_bg)
                .inner_margin(egui::Margin::same(8))
                .show(ui, |ui| {
                    if pane.exited {
                        // Show exit message centered, auto-close on any key
                        let rect = ui.max_rect();
                        ui.painter().rect_filled(rect, 0.0, self.colors.terminal_bg);
                        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(rect), |ui| {
                            ui.centered_and_justified(|ui| {
                                ui.colored_label(self.colors.text_dim, "[process exited]");
                            });
                        });
                        if is_focused
                            && ui.input(|i| {
                                i.events.iter().any(|e| {
                                    matches!(e, egui::Event::Key { pressed: true, .. })
                                })
                            })
                        {
                            self.close_exited = Some(tile_id);
                        }
                        return;
                    }

                    match pane.surface_mode {
                        SurfaceMode::FullTerminal => {
                            // Agent mode: inline (Warp-style) rendering.
                            //
                            // We keep the normal TerminalView so shell
                            // scrollback stays visible and mouse selection /
                            // scroll still work. Two extra steps:
                            //   1. Drain key/text events into AgentMode
                            //      BEFORE TerminalView sees them, so the PTY
                            //      never hears agent-mode keystrokes.
                            //   2. Drain AgentMode's pending ANSI bytes into
                            //      the terminal backend via write_agent_bytes,
                            //      so the `>>> ` indicator and LLM replies
                            //      render into the same grid as shell output.
                            if is_focused && pane.agent_mode.is_active() {
                                intercept_agent_keys(ui, &mut pane.agent_mode);
                            }
                            if pane.agent_mode.poll_llm() {
                                ui.ctx().request_repaint_after(
                                    std::time::Duration::from_millis(50),
                                );
                            }
                            for chunk in pane.agent_mode.drain_output() {
                                pane.backend.write_agent_bytes(&chunk);
                            }
                            if matches!(
                                pane.agent_mode.state,
                                crate::agent_mode::AgentModeState::Processing
                            ) {
                                // Keep polling the LLM worker while a request
                                // is in flight even without new key input.
                                ui.ctx().request_repaint_after(
                                    std::time::Duration::from_millis(50),
                                );
                            }

                            render_name_bar_and_dots(
                                ui,
                                tile_id,
                                pane_id,
                                &self.tab_info,
                                &self.pane_names,
                                &self.colors,
                            );
                            let font_size = pane.font_size;
                            let terminal = TerminalView::new(ui, &mut pane.backend)
                                .set_focus(is_focused)
                                .set_theme(self.theme.clone())
                                .set_font(theme::terminal_font(font_size))
                                .set_size(Vec2::new(
                                    ui.available_width(),
                                    ui.available_height(),
                                ));
                            ui.add(terminal);
                        }

                        SurfaceMode::AppActive => {
                            if let Some(app) = pane.active_app.as_mut() {
                                // App renders full height — the terminal is a
                                // separate pane below (created by auto-split).
                                let app_ctx = AppRenderContext {
                                    colors: &self.colors,
                                    is_focused,
                                    linked_terminal: *pane_id,
                                    media_cache: self.media_cache,
                                };
                                app.ui(ui, &app_ctx);

                                // Forward mouse events to the focused app.
                                // The app surface origin is the inner margin (8px) from the Frame.
                                if is_focused {
                                    let inner_origin = ui.min_rect().min + egui::vec2(8.0, 8.0);

                                    ui.input(|i| {
                                        // Mouse button presses and releases.
                                        for event in &i.events {
                                            match event {
                                                egui::Event::PointerButton { pos, button, pressed, .. } => {
                                                    if ui.max_rect().contains(*pos) {
                                                        let lx = pos.x - inner_origin.x;
                                                        let ly = pos.y - inner_origin.y;
                                                        let btn = match button {
                                                            egui::PointerButton::Primary   => "left",
                                                            egui::PointerButton::Secondary => "right",
                                                            egui::PointerButton::Middle    => "middle",
                                                            _                              => "left",
                                                        };
                                                        if *pressed {
                                                            app.send_mouse_down(lx, ly, btn);
                                                        } else {
                                                            app.send_mouse_up(lx, ly, btn);
                                                        }
                                                    }
                                                }
                                                egui::Event::MouseMoved(_) => {
                                                    // MouseMove: only forward if the app opted in.
                                                    if app.mouse_tracking_enabled() {
                                                        if let Some(pos) = i.pointer.hover_pos() {
                                                            if ui.max_rect().contains(pos) {
                                                                let lx = pos.x - inner_origin.x;
                                                                let ly = pos.y - inner_origin.y;
                                                                app.send_mouse_move(lx, ly);
                                                            }
                                                        }
                                                    }
                                                }
                                                _ => {}
                                            }
                                        }

                                        // Scroll delta — send whenever there is a non-zero scroll.
                                        let scroll = i.smooth_scroll_delta;
                                        if scroll.length_sq() > 0.0 {
                                            if let Some(pos) = i.pointer.hover_pos() {
                                                if ui.max_rect().contains(pos) {
                                                    let lx = pos.x - inner_origin.x;
                                                    let ly = pos.y - inner_origin.y;
                                                    app.send_scroll(lx, ly, scroll.x, scroll.y);
                                                }
                                            }
                                        }
                                    });
                                }
                            } else {
                                // App was dropped — fall back to full terminal.
                                let font_size = pane.font_size;
                                let terminal = TerminalView::new(ui, &mut pane.backend)
                                    .set_focus(is_focused)
                                    .set_theme(self.theme.clone())
                                    .set_font(theme::terminal_font(font_size))
                                    .set_size(Vec2::new(
                                        ui.available_width(),
                                        ui.available_height(),
                                    ));
                                ui.add(terminal);
                            }
                        }

                        SurfaceMode::AppWithCompanion { companion_fraction } => {
                            // Render the app on the top portion of the pane
                            // and the existing (preserved) terminal on the
                            // bottom portion. Both share the same pane id.
                            let pane_rect = ui.max_rect();
                            let frac = companion_fraction.clamp(0.10, 0.90);
                            let separator_h = 1.0;
                            let term_h = (pane_rect.height() * frac).max(40.0);
                            let app_h = (pane_rect.height() - term_h - separator_h).max(40.0);

                            let app_rect = egui::Rect::from_min_size(
                                pane_rect.min,
                                egui::vec2(pane_rect.width(), app_h),
                            );
                            let sep_rect = egui::Rect::from_min_size(
                                egui::pos2(pane_rect.left(), pane_rect.top() + app_h),
                                egui::vec2(pane_rect.width(), separator_h),
                            );
                            let term_rect = egui::Rect::from_min_size(
                                egui::pos2(
                                    pane_rect.left(),
                                    pane_rect.top() + app_h + separator_h,
                                ),
                                egui::vec2(pane_rect.width(), term_h),
                            );

                            // Separator line between app and terminal.
                            ui.painter().rect_filled(
                                sep_rect,
                                0.0,
                                self.colors.border,
                            );

                            // Track which surface the pointer is currently
                            // over so a click promotes that surface inside
                            // the pane (handled below the per-surface UIs).
                            let pointer_over_app = ui
                                .input(|i| i.pointer.hover_pos())
                                .map(|p| app_rect.contains(p))
                                .unwrap_or(false);
                            let pointer_over_term = ui
                                .input(|i| i.pointer.hover_pos())
                                .map(|p| term_rect.contains(p))
                                .unwrap_or(false);
                            let click_pressed =
                                ui.input(|i| i.pointer.any_pressed());

                            // App surface (top).
                            let app_focused = is_focused
                                && pane.focused_surface == SurfaceLayer::App;
                            if let Some(app) = pane.active_app.as_mut() {
                                let mut app_ui = ui.new_child(
                                    egui::UiBuilder::new().max_rect(app_rect),
                                );
                                let app_ctx = AppRenderContext {
                                    colors: &self.colors,
                                    is_focused: app_focused,
                                    linked_terminal: *pane_id,
                                };
                                app.ui(&mut app_ui, &app_ctx);
                            }

                            // Companion terminal surface (bottom). This is
                            // the SAME terminal backend that was running in
                            // this pane before the app opened — history,
                            // running processes, agent sessions all intact.
                            let term_focused = is_focused
                                && pane.focused_surface == SurfaceLayer::Terminal;
                            let mut term_ui = ui.new_child(
                                egui::UiBuilder::new().max_rect(term_rect),
                            );
                            let font_size = pane.font_size;
                            let terminal = TerminalView::new(&mut term_ui, &mut pane.backend)
                                .set_focus(term_focused)
                                .set_theme(self.theme.clone())
                                .set_font(theme::terminal_font(font_size))
                                .set_size(Vec2::new(term_rect.width(), term_rect.height()));
                            term_ui.add(terminal);

                            // Subtle highlight on the focused surface so the
                            // user can see which one will receive keys.
                            let highlight = Stroke::new(1.0, self.colors.accent);
                            if app_focused {
                                ui.painter().rect_stroke(
                                    app_rect.shrink(0.5),
                                    0.0,
                                    highlight,
                                    egui::StrokeKind::Inside,
                                );
                            } else if term_focused {
                                ui.painter().rect_stroke(
                                    term_rect.shrink(0.5),
                                    0.0,
                                    highlight,
                                    egui::StrokeKind::Inside,
                                );
                            }

                            // Click-to-focus inside the embedded layout: a
                            // press inside the app rect promotes the app
                            // surface; a press inside the terminal rect
                            // promotes the terminal surface.
                            if is_focused && click_pressed {
                                if pointer_over_app
                                    && pane.focused_surface != SurfaceLayer::App
                                {
                                    pane.focused_surface = SurfaceLayer::App;
                                } else if pointer_over_term
                                    && pane.focused_surface != SurfaceLayer::Terminal
                                {
                                    pane.focused_surface = SurfaceLayer::Terminal;
                                }
                            }

                            // If the pointer is over a surface that isn't
                            // the focused one, dim that surface slightly so
                            // the user knows keys won't go there until they
                            // click. Skipping for MVP — left intentionally
                            // simple. The accent border is enough.
                            let _ = APP_DIM_OPACITY;
                        }
                    }

                    // Draw tab indicator dots (top-left) when 2+ tabs and NO name bar
                    if !self.pane_names.contains_key(pane_id) {
                        if let Some(&(active_idx, count)) = self.tab_info.get(&tile_id) {
                            let rect = ui.max_rect();
                            paint_tab_dots(
                                ui.painter(),
                                rect.left(),
                                rect.top() + 2.0 + DOT_RADIUS,
                                active_idx,
                                count,
                                self.colors.accent,
                                self.colors.bg_active,
                            );
                        }
                    }
                });
        }

        UiResponse::None
    }

    fn tab_title_for_pane(&mut self, pane: &PaneId) -> egui::WidgetText {
        let label = if let Some(name) = self.pane_names.get(pane) {
            name.clone()
        } else {
            format!("Terminal {}", pane + 1)
        };
        egui::RichText::new(label)
            .size(11.0)
            .color(self.colors.text_dim)
            .into()
    }

    fn simplification_options(&self) -> SimplificationOptions {
        SimplificationOptions {
            all_panes_must_have_tabs: true,
            ..SimplificationOptions::default()
        }
    }

    fn tab_ui(
        &mut self,
        _tiles: &mut Tiles<PaneId>,
        ui: &mut egui::Ui,
        id: egui::Id,
        _tile_id: TileId,
        _state: &TabState,
    ) -> egui::Response {
        // During zoom, suppress all tab label rendering so they don't bleed
        // through the semi-transparent scrim over background panes.
        let (_, rect) = ui.allocate_space(egui::Vec2::ZERO);
        ui.interact(rect, id, egui::Sense::hover())
    }

    fn tab_bar_height(&self, _style: &egui::Style) -> f32 {
        0.0
    }

    fn gap_width(&self, _style: &egui::Style) -> f32 {
        4.0
    }

    fn paint_on_top_of_tile(
        &self,
        painter: &egui::Painter,
        _style: &egui::Style,
        tile_id: TileId,
        rect: egui::Rect,
    ) {
        if self.focused_tile == Some(tile_id) {
            let stroke = egui::Stroke::new(1.5, self.colors.accent);
            let rect = rect.shrink(0.75);
            painter.rect_stroke(rect, 0.0, stroke, egui::StrokeKind::Inside);
        }
    }
}

/// Route every key / text event for a focused, agent-mode pane into
/// `AgentMode::handle_key_event`, draining the matched events from the frame
/// so the terminal widget rendered afterward never sees them and nothing
/// reaches the PTY.
///
/// This runs BEFORE `TerminalView::new().add(...)` renders, which is the
/// lesson from `~/.claude/CLAUDE.md`: TextEdit/terminal widgets consume key
/// events during their own rendering pass, so any app-level interception has
/// to happen first. Filtering `input.events` here is equivalent to calling
/// `consume_key` for every relevant event.
fn intercept_agent_keys(ui: &mut egui::Ui, agent: &mut AgentMode) {
    // Take the current events out of egui's input state, hand each eligible
    // one to AgentMode, and put the non-consumed events back. The terminal
    // widget rendered afterward clones `i.events` in its process_input pass,
    // so anything we drop here never reaches the PTY.
    let mut changed = false;
    ui.input_mut(|input| {
        let old = std::mem::take(&mut input.events);
        let mut kept = Vec::with_capacity(old.len());
        for event in old {
            let eligible = matches!(
                event,
                egui::Event::Text(_) | egui::Event::Key { pressed: true, .. }
            );
            if eligible && agent.handle_key_event(&event) {
                changed = true;
                continue;
            }
            kept.push(event);
        }
        input.events = kept;
    });
    if changed {
        ui.ctx().request_repaint();
    }
}

/// Render the pane name bar (if named) and tab dot indicators for a terminal in FullTerminal mode.
fn render_name_bar_and_dots(
    ui: &mut egui::Ui,
    tile_id: TileId,
    pane_id: &PaneId,
    tab_info: &HashMap<TileId, (usize, usize)>,
    pane_names: &HashMap<PaneId, String>,
    colors: &Colors,
) {
    let name_bar_height = 20.0;
    let has_name = pane_names.contains_key(pane_id);
    let has_tabs = tab_info.contains_key(&tile_id);

    if has_name {
        let bar_rect = egui::Rect::from_min_size(
            ui.cursor().min,
            egui::vec2(ui.available_width(), name_bar_height),
        );
        ui.advance_cursor_after_rect(bar_rect);

        let name = &pane_names[pane_id];

        if let Some(&(active_idx, count)) = tab_info.get(&tile_id) {
            paint_tab_dots(
                ui.painter(),
                bar_rect.left(),
                bar_rect.center().y,
                active_idx,
                count,
                colors.accent,
                colors.bg_active,
            );
        }

        ui.painter().text(
            bar_rect.center(),
            egui::Align2::CENTER_CENTER,
            name,
            egui::FontId::proportional(11.0),
            colors.text_dim,
        );
    } else if has_tabs {
        ui.add_space(TAB_DOT_RESERVED_HEIGHT);
    }
}
