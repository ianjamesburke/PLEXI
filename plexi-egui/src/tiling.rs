use crate::pane::TerminalPane;
use egui::Vec2;
use egui_term::{TerminalFont, TerminalTheme, TerminalView};
use egui_tiles::{Behavior, TileId, UiResponse};
use std::collections::HashMap;

pub type PaneId = u64;

pub struct PlexiBehavior<'a> {
    pub panes: &'a mut HashMap<PaneId, TerminalPane>,
    pub focused_tile: Option<TileId>,
    pub theme: TerminalTheme,
    pub font: TerminalFont,
    pub new_focused: Option<TileId>,
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
            let terminal = TerminalView::new(ui, &mut pane.backend)
                .set_focus(is_focused)
                .set_theme(self.theme.clone())
                .set_font(self.font.clone())
                .set_size(Vec2::new(ui.available_width(), ui.available_height()));
            ui.add(terminal);
        }

        UiResponse::None
    }

    fn tab_title_for_pane(&mut self, _pane: &PaneId) -> egui::WidgetText {
        "Terminal".into()
    }

    fn tab_bar_height(&self, _style: &egui::Style) -> f32 {
        0.0
    }

    fn gap_width(&self, _style: &egui::Style) -> f32 {
        2.0
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
