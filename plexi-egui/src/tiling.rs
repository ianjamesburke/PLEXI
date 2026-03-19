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
}

impl Behavior<PaneId> for PlexiBehavior<'_> {
    fn pane_ui(
        &mut self,
        ui: &mut egui::Ui,
        tile_id: TileId,
        pane_id: &mut PaneId,
    ) -> UiResponse {
        // Detect clicks for focus
        if ui.input(|i| i.pointer.any_pressed()) && ui.rect_contains_pointer(ui.max_rect()) {
            self.new_focused = Some(tile_id);
        }

        let is_focused = self.focused_tile == Some(tile_id);

        // If this pane is zoomed, render a dark placeholder instead of the terminal
        if self.zoomed_pane == Some(tile_id) {
            egui::Frame::new()
                .fill(egui::Color32::from_rgb(0x11, 0x11, 0x1b))
                .inner_margin(egui::Margin::same(8))
                .show(ui, |_ui| {});
            return UiResponse::None;
        }

        if let Some(pane) = self.panes.get_mut(pane_id) {
            egui::Frame::new()
                .fill(egui::Color32::from_rgb(0x1e, 0x1e, 0x2e))
                .inner_margin(egui::Margin::same(8))
                .show(ui, |ui| {
                    if pane.exited {
                        // Show exit message centered, auto-close on any key
                        let rect = ui.max_rect();
                        ui.painter().rect_filled(
                            rect,
                            0.0,
                            egui::Color32::from_rgb(0x1e, 0x1e, 0x2e),
                        );
                        ui.allocate_new_ui(egui::UiBuilder::new().max_rect(rect), |ui| {
                            ui.centered_and_justified(|ui| {
                                ui.colored_label(
                                    egui::Color32::from_rgb(0x6c, 0x70, 0x86),
                                    "[process exited]",
                                );
                            });
                        });
                        if is_focused && ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. }))) {
                            self.close_exited = Some(tile_id);
                        }
                    } else {
                        let has_tabs = self.tab_info.contains_key(&tile_id);
                        // Reserve space for dot indicators above terminal
                        if has_tabs {
                            ui.add_space(14.0);
                        }
                        let terminal = TerminalView::new(ui, &mut pane.backend)
                            .set_focus(is_focused)
                            .set_theme(self.theme.clone())
                            .set_font(self.font.clone())
                            .set_size(Vec2::new(ui.available_width(), ui.available_height()));
                        ui.add(terminal);
                    }

                    // Draw tab indicator dots (top-left) when 2+ tabs
                    if let Some(&(active_idx, count)) = self.tab_info.get(&tile_id) {
                        let dot_radius = 4.0;
                        let dot_spacing = 12.0;
                        let rect = ui.max_rect();
                        let start_x = rect.left() + 2.0;
                        let y = rect.top() + 2.0 + dot_radius;

                        let accent = egui::Color32::from_rgb(137, 180, 250); // Catppuccin blue
                        let dim = egui::Color32::from_rgb(0x45, 0x47, 0x5a);

                        for i in 0..count {
                            let cx = start_x + (i as f32) * dot_spacing + dot_radius;
                            let color = if i == active_idx { accent } else { dim };
                            ui.painter().circle_filled(egui::pos2(cx, y), dot_radius, color);
                        }
                    }
                });
        }

        UiResponse::None
    }

    fn tab_title_for_pane(&mut self, pane: &PaneId) -> egui::WidgetText {
        let label = format!("Terminal {}", pane + 1);
        egui::RichText::new(label)
            .size(11.0)
            .color(Colors::TEXT_DIM)
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
            // Catppuccin Mocha blue
            let stroke = egui::Stroke::new(1.5, egui::Color32::from_rgb(137, 180, 250));
            let rect = rect.shrink(0.75);
            painter.rect_stroke(rect, 0.0, stroke, egui::StrokeKind::Inside);
        }
    }
}
