use crate::pane::TerminalPane;
use crate::theme::Colors;
use egui::Vec2;
use egui_term::{TerminalFont, TerminalTheme, TerminalView};
use egui_tiles::{Behavior, SimplificationOptions, TabState, TileId, Tiles, UiResponse};
use std::collections::HashMap;

pub type PaneId = u64;

pub struct PlexiBehavior<'a> {
    pub panes: &'a mut HashMap<PaneId, TerminalPane>,
    pub focused_tile: Option<TileId>,
    pub theme: TerminalTheme,
    pub font: TerminalFont,
    pub new_focused: Option<TileId>,
    pub close_exited: Option<TileId>,
    pub tab_info: HashMap<TileId, (usize, usize)>, // tile_id -> (index, count)
    pub zoomed_pane: Option<TileId>,
    pub colors: Colors,
    pub pane_names: HashMap<PaneId, String>,
}

impl Behavior<PaneId> for PlexiBehavior<'_> {
    fn pane_ui(
        &mut self,
        ui: &mut egui::Ui,
        tile_id: TileId,
        pane_id: &mut PaneId,
    ) -> UiResponse {
        // Detect clicks for focus (skip when a pane is zoomed — input belongs to the overlay)
        if self.zoomed_pane.is_none()
            && ui.input(|i| i.pointer.any_pressed())
            && ui.rect_contains_pointer(ui.max_rect())
        {
            self.new_focused = Some(tile_id);
        }

        let is_focused = self.focused_tile == Some(tile_id);

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
                                let dot_radius = 4.0;
                                let dot_spacing = 12.0;
                                let start_x = bar_rect.left() - 6.0;
                                let y = bar_rect.center().y;
                                let dim = self.colors.bg_active;

                                for i in 0..count {
                                    let cx = start_x + (i as f32) * dot_spacing + dot_radius;
                                    let color = if i == active_idx { self.colors.accent } else { dim };
                                    ui.painter().circle_filled(egui::pos2(cx, y), dot_radius, color);
                                }
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
                            ui.add_space(14.0);
                        }

                        let terminal = TerminalView::new(ui, &mut pane.backend)
                            .set_focus(is_focused)
                            .set_theme(self.theme.clone())
                            .set_font(self.font.clone())
                            .set_size(Vec2::new(ui.available_width(), ui.available_height()));
                        ui.add(terminal);
                    }

                    // Draw tab indicator dots (top-left) when 2+ tabs and NO name bar
                    // (when a name bar exists, dots are already drawn inside it)
                    if !self.pane_names.contains_key(pane_id) {
                        if let Some(&(active_idx, count)) = self.tab_info.get(&tile_id) {
                            let dot_radius = 4.0;
                            let dot_spacing = 12.0;
                            let rect = ui.max_rect();
                            let start_x = rect.left() + 2.0;
                            let y = rect.top() + 2.0 + dot_radius;

                            let dim = self.colors.bg_active;

                            for i in 0..count {
                                let cx = start_x + (i as f32) * dot_spacing + dot_radius;
                                let color = if i == active_idx { self.colors.accent } else { dim };
                                ui.painter().circle_filled(egui::pos2(cx, y), dot_radius, color);
                            }
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
