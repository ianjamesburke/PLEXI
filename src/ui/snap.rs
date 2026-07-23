//! Pixel-grid snapping for hand-painted text (stint 0530).
//!
//! Plain egui `Label`s land on the physical pixel grid via egui's own widget
//! layout; raw `painter.galley` / `painter.text` calls do not — centering
//! math like `(band_h - block_h) / 2.0` routinely produces half-pixel
//! origins that rasterize fuzzy on every display. All hand-painted chrome
//! text goes through these helpers so the galley origin is rounded to the
//! grid for the pane's actual `pixels_per_point`.

use std::sync::Arc;

use egui::emath::GuiRounding;
use egui::{Align2, Color32, FontId, Galley, Painter, Pos2, Rect};

/// `Painter::galley` with the origin snapped to the physical pixel grid.
pub fn galley_snapped(painter: &Painter, pos: Pos2, galley: Arc<Galley>, fallback_color: Color32) {
    painter.galley(
        pos.round_to_pixels(painter.pixels_per_point()),
        galley,
        fallback_color,
    );
}

/// `Painter::text` with the anchored galley origin snapped to the physical
/// pixel grid. Returns the rect the text was painted into.
pub fn text_snapped(
    painter: &Painter,
    pos: Pos2,
    anchor: Align2,
    text: impl ToString,
    font_id: FontId,
    color: Color32,
) -> Rect {
    let galley = painter.layout_no_wrap(text.to_string(), font_id, color);
    let min = anchor
        .anchor_size(pos, galley.size())
        .min
        .round_to_pixels(painter.pixels_per_point());
    let rect = Rect::from_min_size(min, galley.size());
    painter.galley(rect.min, galley, color);
    rect
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every anchor lands the galley origin on the physical pixel grid, at
    /// integer and HiDPI scales.
    #[test]
    fn snapped_text_origin_is_on_the_physical_pixel_grid() {
        for ppp in [1.0_f32, 2.0] {
            let ctx = egui::Context::default();
            ctx.set_pixels_per_point(ppp);
            let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                let painter = ui.painter();
                for anchor in [
                    Align2::LEFT_TOP,
                    Align2::CENTER_CENTER,
                    Align2::RIGHT_BOTTOM,
                    Align2::LEFT_CENTER,
                ] {
                    let rect = text_snapped(
                        painter,
                        egui::pos2(10.3, 7.7),
                        anchor,
                        "workspace",
                        FontId::proportional(12.0),
                        Color32::WHITE,
                    );
                    for coord in [rect.min.x, rect.min.y] {
                        let phys = coord * ppp;
                        assert!(
                            (phys - phys.round()).abs() < 1e-3,
                            "origin {coord} not on the pixel grid at ppp {ppp} (anchor {anchor:?})"
                        );
                    }
                }
            });
        }
    }
}
