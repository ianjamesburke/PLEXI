use crate::app_trait::{AppRenderContext, SurfaceLayer, SurfaceMode, APP_DIM_OPACITY};
use crate::pane::Pane;
use crate::theme::{self, Colors};
use egui::{Color32, Vec2};
use egui_term::{BackendCommand, TerminalTheme, TerminalView};
use egui_tiles::{Behavior, SimplificationOptions, TabState, TileId, Tiles, UiResponse};
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
    pub panes: &'a mut HashMap<PaneId, Pane>,
    pub focused_tile: Option<TileId>,
    pub theme: TerminalTheme,
    pub new_focused: Option<TileId>,
    pub close_exited: Option<TileId>,
    pub tab_info: HashMap<TileId, (usize, usize)>, // tile_id -> (index, count)
    pub zoomed_pane: Option<TileId>,
    pub colors: Colors,
    pub pane_names: HashMap<PaneId, String>,
    pub drag_cursor_pos: Option<egui::Pos2>,
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
        if is_drag_hovering {
            let dropped = ui.input(|i| i.raw.dropped_files.clone());
            if !dropped.is_empty() {
                if let Some(pane) = self.panes.get_mut(pane_id) {
                    if let Some(t) = pane.as_terminal_mut() {
                        for file in dropped {
                            if let Some(path) = &file.path {
                                let path_str = path.display().to_string();
                                let escaped = if path_str.contains(|c: char| {
                                    c.is_whitespace() || "\"'\\()&|;$`!#".contains(c)
                                }) {
                                    format!("'{}'", path_str.replace('\'', "'\\''"))
                                } else {
                                    path_str
                                };
                                t.backend.process_command(BackendCommand::Write(
                                    escaped.as_bytes().to_vec(),
                                ));
                            }
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
                    // Agent panes: full turn loop UI.
                    if let Some(agent) = pane.as_agent_mut() {
                        // Drain any streamed tokens from background thread.
                        if let Some(rx) = &agent.turn_rx {
                            let mut done = false;
                            loop {
                                match rx.try_recv() {
                                    Ok(crate::pane::TurnMsg::Token(chunk)) => {
                                        if let Some(last) = agent.transcript.last_mut() {
                                            last.push_str(&chunk);
                                        }
                                    }
                                    Ok(crate::pane::TurnMsg::Done { session_id, token_count }) => {
                                        agent.session_id = session_id;
                                        agent.transcript.push(String::new()); // separator
                                        crate::event_log::emit(crate::event_log::HostEvent::AgentTurn {
                                            session_id: agent.session_id.clone(),
                                            token_count,
                                            timestamp: crate::event_log::now_timestamp(),
                                        });
                                        done = true;
                                        break;
                                    }
                                    Ok(crate::pane::TurnMsg::Error(e)) => {
                                        agent.transcript.push(format!("[error] {e}"));
                                        done = true;
                                        break;
                                    }
                                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                                        done = true;
                                        break;
                                    }
                                }
                            }
                            if done {
                                agent.turn_rx = None;
                            }
                        }

                        let rect = ui.max_rect();
                        // Title bar.
                        let title_rect = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), 24.0));
                        ui.painter().rect_filled(title_rect, 0.0, self.colors.bg_toolbar);
                        let status = if agent.turn_rx.is_some() { " ⏳" } else { "" };
                        ui.painter().text(
                            egui::pos2(title_rect.min.x + 8.0, title_rect.center().y),
                            egui::Align2::LEFT_CENTER,
                            format!("🤖 {}{status}", agent.label),
                            egui::FontId::proportional(12.0),
                            self.colors.text_primary,
                        );

                        // Input bar at the bottom.
                        let input_h = 32.0;
                        let input_rect = egui::Rect::from_min_max(
                            egui::pos2(rect.min.x, rect.max.y - input_h),
                            rect.max,
                        );
                        // Transcript area.
                        let body_rect = egui::Rect::from_min_max(
                            egui::pos2(rect.min.x, title_rect.max.y),
                            egui::pos2(rect.max.x, input_rect.min.y),
                        );
                        ui.painter().rect_filled(body_rect, 0.0, self.colors.terminal_bg);

                        // Render transcript.
                        let mut y = body_rect.min.y + 4.0;
                        let line_h = 14.0;
                        for line in &agent.transcript {
                            if y + line_h > body_rect.max.y { break; }
                            ui.painter().text(
                                egui::pos2(body_rect.min.x + 6.0, y),
                                egui::Align2::LEFT_TOP,
                                line,
                                egui::FontId::monospace(11.0),
                                self.colors.text_primary,
                            );
                            y += line_h;
                        }

                        if agent.transcript.is_empty() && agent.turn_rx.is_none() {
                            ui.painter().text(
                                body_rect.center(),
                                egui::Align2::CENTER_CENTER,
                                "Type a message below and press Enter",
                                egui::FontId::proportional(11.0),
                                self.colors.text_dim,
                            );
                        }

                        // Input bar.
                        ui.painter().rect_filled(input_rect, 0.0, self.colors.bg_active);
                        let input_id = egui::Id::new(("agent_input", agent.id));
                        let mut te = egui::TextEdit::singleline(&mut agent.input_buf)
                            .desired_width(input_rect.width() - 12.0)
                            .font(egui::FontId::monospace(12.0))
                            .hint_text("Send a message…");
                        let te_resp = ui.put(
                            egui::Rect::from_min_size(
                                egui::pos2(input_rect.min.x + 6.0, input_rect.min.y + 4.0),
                                egui::vec2(input_rect.width() - 12.0, input_h - 8.0),
                            ),
                            te,
                        );

                        // Submit on Enter.
                        if te_resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            let msg = std::mem::take(&mut agent.input_buf);
                            if !msg.is_empty() && agent.turn_rx.is_none() {
                                agent.transcript.push(format!("> {msg}"));
                                agent.transcript.push(String::new()); // assistant turn accumulates here
                                let session_id = agent.session_id.clone();

                                // Fire off the turn on a background thread.
                                let (tx, rx) = std::sync::mpsc::channel::<crate::pane::TurnMsg>();
                                agent.turn_rx = Some(rx);
                                std::thread::spawn(move || {
                                    use crate::plexi_iq::backend::ClaudeCliBackend;
                                    let backend = ClaudeCliBackend::new();
                                    let tx_tok = tx.clone();
                                    let result = crate::plexi_iq::turn_loop::run_turn(
                                        &backend,
                                        msg,
                                        "You are Plexi IQ, a helpful terminal-native assistant.",
                                        session_id,
                                        move |chunk| {
                                            let _ = tx_tok.send(crate::pane::TurnMsg::Token(chunk.to_string()));
                                        },
                                    );
                                    match result {
                                        Ok(r) => {
                                            let tok_count = r.output_tokens.unwrap_or(r.text.split_whitespace().count() as u32) as usize;
                                            let _ = tx.send(crate::pane::TurnMsg::Done {
                                                session_id: r.session_id,
                                                token_count: tok_count,
                                            });
                                        }
                                        Err(e) => {
                                            let _ = tx.send(crate::pane::TurnMsg::Error(e.to_string()));
                                        }
                                    }
                                });
                            }
                        }

                        // Request repaint while a turn is in progress.
                        if agent.turn_rx.is_some() {
                            ui.ctx().request_repaint();
                        }

                        return;
                    }

                    // App panes: handled by their own subsystems (Layer 3b).
                    let Some(t) = pane.as_terminal_mut() else { return };

                    if t.exited {
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

                    match t.surface_mode {
                        SurfaceMode::FullTerminal => {
                            render_name_bar_and_dots(
                                ui,
                                tile_id,
                                pane_id,
                                &self.tab_info,
                                &self.pane_names,
                                &self.colors,
                            );
                            let font_size = t.font_size;
                            let terminal = TerminalView::new(ui, &mut t.backend)
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
                            if let Some(app) = t.active_app.as_mut() {
                                // App renders full height — the terminal is a
                                // separate pane below (created by auto-split).
                                let app_ctx = AppRenderContext {
                                    colors: &self.colors,
                                    is_focused,
                                    linked_terminal: *pane_id,
                                };
                                app.ui(ui, &app_ctx);
                            } else {
                                // App was dropped — fall back to full terminal.
                                let font_size = t.font_size;
                                let terminal = TerminalView::new(ui, &mut t.backend)
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
