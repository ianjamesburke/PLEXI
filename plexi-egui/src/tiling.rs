use crate::pane::TerminalPane;
use crate::theme::Colors;
use egui::Vec2;
use egui_term::{TerminalFont, TerminalTheme, TerminalView};
use egui_tiles::{Behavior, TabState, TileId, Tiles, UiResponse};
use std::collections::HashMap;

pub type PaneId = u64;

pub struct PlexiBehavior<'a> {
    pub panes: &'a mut HashMap<PaneId, TerminalPane>,
    pub focused_tile: Option<TileId>,
    pub theme: TerminalTheme,
    pub font: TerminalFont,
    pub new_focused: Option<TileId>,
    pub close_exited: Option<TileId>,
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
                        let terminal = TerminalView::new(ui, &mut pane.backend)
                            .set_focus(is_focused)
                            .set_theme(self.theme.clone())
                            .set_font(self.font.clone())
                            .set_size(Vec2::new(ui.available_width(), ui.available_height()));
                        ui.add(terminal);
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

    fn tab_bar_height(&self, _style: &egui::Style) -> f32 {
        24.0
    }

    fn tab_bar_color(&self, _visuals: &egui::Visuals) -> egui::Color32 {
        Colors::BG_DARKEST
    }

    fn tab_bg_color(
        &self,
        _visuals: &egui::Visuals,
        _tiles: &Tiles<PaneId>,
        _tile_id: TileId,
        state: &TabState,
    ) -> egui::Color32 {
        if state.active {
            egui::Color32::from_rgb(0x1e, 0x1e, 0x2e) // terminal bg
        } else {
            Colors::BG_DARKEST
        }
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
