use crate::pane::TerminalPane;
use crate::theme::{self, Colors};
use egui::{Color32, Vec2};
use egui_term::{BackendCommand, TerminalTheme, TerminalView};
use egui_tiles::{Behavior, SimplificationOptions, TabState, TileId, Tiles, UiResponse};
use std::collections::HashMap;

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
                        ui.painter().rect_filled(
                            rect,
                            0.0,
                            self.colors.terminal_bg,
                        );
                        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(rect), |ui| {
                            ui.centered_and_justified(|ui| {
                                ui.colored_label(
                                    self.colors.text_dim,
                                    "[process exited]",
                                );
                            });
                        });
                        if is_focused && ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. }))) {
                            self.close_exited = Some(tile_id);
                        }
                    } else {
                        let has_tabs = self.tab_info.contains_key(&tile_id);
                        let has_name = self.pane_names.contains_key(pane_id);
                        let name_bar_height = 20.0;

                        // Reserve space for name bar or dot indicators above terminal
                        if has_name {
                            // Draw centered name bar
                            let bar_rect = egui::Rect::from_min_size(
                                ui.cursor().min,
                                egui::vec2(ui.available_width(), name_bar_height),
                            );
                            ui.advance_cursor_after_rect(bar_rect);

                            let name = &self.pane_names[pane_id];

                            // If tabs exist, draw dots on the left side of the bar
                            if let Some(&(active_idx, count)) = self.tab_info.get(&tile_id) {
                                paint_tab_dots(
                                    ui.painter(),
                                    bar_rect.left(),
                                    bar_rect.center().y,
                                    active_idx,
                                    count,
                                    self.colors.accent,
                                    self.colors.bg_active,
                                );
                            }

                            // Center the name text in the bar
                            ui.painter().text(
                                bar_rect.center(),
                                egui::Align2::CENTER_CENTER,
                                name,
                                egui::FontId::proportional(11.0),
                                self.colors.text_dim,
                            );
                        } else if has_tabs {
                            ui.add_space(TAB_DOT_RESERVED_HEIGHT);
                        }

                        let font_size = pane.font_size;
                        let terminal = TerminalView::new(ui, &mut pane.backend)
                            .set_focus(is_focused)
                            .set_theme(self.theme.clone())
                            .set_font(theme::terminal_font(font_size))
                            .set_size(Vec2::new(ui.available_width(), ui.available_height()));
                        ui.add(terminal);
                    }

                    // Draw tab indicator dots (top-left) when 2+ tabs and NO name bar
                    // (when a name bar exists, dots are already drawn inside it)
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
