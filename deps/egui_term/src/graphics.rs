use egui::emath::GuiRounding;
use egui::epaint::RectShape;
use egui::{pos2, Color32, CornerRadius, Rect, Shape};

#[derive(Clone, Copy, Debug, PartialEq)]
struct RectFraction {
    x0: f32,
    x1: f32,
    y0: f32,
    y1: f32,
}

const ONE_EIGHTH: f32 = 0.125;
const ONE_QUARTER: f32 = 0.25;
const THREE_EIGHTHS: f32 = 0.375;
const HALF: f32 = 0.5;
const FIVE_EIGHTHS: f32 = 0.625;
const THREE_QUARTERS: f32 = 0.75;
const SEVEN_EIGHTHS: f32 = 0.875;

const FULL_BLOCK: [RectFraction; 1] = [RectFraction::new(0.0, 1.0, 0.0, 1.0)];
const UPPER_HALF: [RectFraction; 1] = [RectFraction::new(0.0, 1.0, 0.0, HALF)];
const LOWER_HALF: [RectFraction; 1] = [RectFraction::new(0.0, 1.0, HALF, 1.0)];
const LOWER_ONE_EIGHTH: [RectFraction; 1] =
    [RectFraction::new(0.0, 1.0, 1.0 - ONE_EIGHTH, 1.0)];
const LOWER_ONE_QUARTER: [RectFraction; 1] =
    [RectFraction::new(0.0, 1.0, 1.0 - ONE_QUARTER, 1.0)];
const LOWER_THREE_EIGHTHS: [RectFraction; 1] =
    [RectFraction::new(0.0, 1.0, 1.0 - THREE_EIGHTHS, 1.0)];
const LOWER_FIVE_EIGHTHS: [RectFraction; 1] =
    [RectFraction::new(0.0, 1.0, 1.0 - FIVE_EIGHTHS, 1.0)];
const LOWER_THREE_QUARTERS: [RectFraction; 1] =
    [RectFraction::new(0.0, 1.0, 1.0 - THREE_QUARTERS, 1.0)];
const LOWER_SEVEN_EIGHTHS: [RectFraction; 1] =
    [RectFraction::new(0.0, 1.0, 1.0 - SEVEN_EIGHTHS, 1.0)];
const LEFT_ONE_EIGHTH: [RectFraction; 1] =
    [RectFraction::new(0.0, ONE_EIGHTH, 0.0, 1.0)];
const LEFT_ONE_QUARTER: [RectFraction; 1] =
    [RectFraction::new(0.0, ONE_QUARTER, 0.0, 1.0)];
const LEFT_THREE_EIGHTHS: [RectFraction; 1] =
    [RectFraction::new(0.0, THREE_EIGHTHS, 0.0, 1.0)];
const LEFT_HALF: [RectFraction; 1] = [RectFraction::new(0.0, HALF, 0.0, 1.0)];
const LEFT_FIVE_EIGHTHS: [RectFraction; 1] =
    [RectFraction::new(0.0, FIVE_EIGHTHS, 0.0, 1.0)];
const LEFT_THREE_QUARTERS: [RectFraction; 1] =
    [RectFraction::new(0.0, THREE_QUARTERS, 0.0, 1.0)];
const LEFT_SEVEN_EIGHTHS: [RectFraction; 1] =
    [RectFraction::new(0.0, SEVEN_EIGHTHS, 0.0, 1.0)];
const RIGHT_HALF: [RectFraction; 1] = [RectFraction::new(HALF, 1.0, 0.0, 1.0)];
const UPPER_ONE_EIGHTH: [RectFraction; 1] =
    [RectFraction::new(0.0, 1.0, 0.0, ONE_EIGHTH)];
const RIGHT_ONE_EIGHTH: [RectFraction; 1] =
    [RectFraction::new(1.0 - ONE_EIGHTH, 1.0, 0.0, 1.0)];
const QUADRANT_BL: [RectFraction; 1] = [RectFraction::new(0.0, HALF, HALF, 1.0)];
const QUADRANT_BR: [RectFraction; 1] = [RectFraction::new(HALF, 1.0, HALF, 1.0)];
const QUADRANT_TL: [RectFraction; 1] = [RectFraction::new(0.0, HALF, 0.0, HALF)];
const QUADRANT_TR: [RectFraction; 1] = [RectFraction::new(HALF, 1.0, 0.0, HALF)];
const QUADRANT_TL_BL_BR: [RectFraction; 3] = [
    RectFraction::new(0.0, HALF, 0.0, HALF),
    RectFraction::new(0.0, HALF, HALF, 1.0),
    RectFraction::new(HALF, 1.0, HALF, 1.0),
];
const QUADRANT_TL_BR: [RectFraction; 2] = [
    RectFraction::new(0.0, HALF, 0.0, HALF),
    RectFraction::new(HALF, 1.0, HALF, 1.0),
];
const QUADRANT_TL_TR_BL: [RectFraction; 3] = [
    RectFraction::new(0.0, HALF, 0.0, HALF),
    RectFraction::new(HALF, 1.0, 0.0, HALF),
    RectFraction::new(0.0, HALF, HALF, 1.0),
];
const QUADRANT_TL_TR_BR: [RectFraction; 3] = [
    RectFraction::new(0.0, HALF, 0.0, HALF),
    RectFraction::new(HALF, 1.0, 0.0, HALF),
    RectFraction::new(HALF, 1.0, HALF, 1.0),
];
const QUADRANT_TR_BL: [RectFraction; 2] = [
    RectFraction::new(HALF, 1.0, 0.0, HALF),
    RectFraction::new(0.0, HALF, HALF, 1.0),
];
const QUADRANT_TR_BL_BR: [RectFraction; 3] = [
    RectFraction::new(HALF, 1.0, 0.0, HALF),
    RectFraction::new(0.0, HALF, HALF, 1.0),
    RectFraction::new(HALF, 1.0, HALF, 1.0),
];

impl RectFraction {
    const fn new(x0: f32, x1: f32, y0: f32, y1: f32) -> Self {
        Self { x0, x1, y0, y1 }
    }
}

pub(crate) fn maybe_push_graphics_element(
    shapes: &mut Vec<Shape>,
    c: char,
    cell_rect: Rect,
    fg: Color32,
    pixels_per_point: f32,
) -> bool {
    let Some(rects) = block_element_rects(c, cell_rect, pixels_per_point) else {
        return false;
    };

    for rect in rects {
        shapes.push(Shape::Rect(RectShape::filled(
            rect,
            CornerRadius::ZERO,
            fg,
        )));
    }

    true
}

fn block_element_rects(
    c: char,
    cell_rect: Rect,
    pixels_per_point: f32,
) -> Option<Vec<Rect>> {
    let fractions = match c {
        '\u{2580}' => &UPPER_HALF[..],
        '\u{2581}' => &LOWER_ONE_EIGHTH[..],
        '\u{2582}' => &LOWER_ONE_QUARTER[..],
        '\u{2583}' => &LOWER_THREE_EIGHTHS[..],
        '\u{2584}' => &LOWER_HALF[..],
        '\u{2585}' => &LOWER_FIVE_EIGHTHS[..],
        '\u{2586}' => &LOWER_THREE_QUARTERS[..],
        '\u{2587}' => &LOWER_SEVEN_EIGHTHS[..],
        '\u{2588}' => &FULL_BLOCK[..],
        '\u{2589}' => &LEFT_SEVEN_EIGHTHS[..],
        '\u{258A}' => &LEFT_THREE_QUARTERS[..],
        '\u{258B}' => &LEFT_FIVE_EIGHTHS[..],
        '\u{258C}' => &LEFT_HALF[..],
        '\u{258D}' => &LEFT_THREE_EIGHTHS[..],
        '\u{258E}' => &LEFT_ONE_QUARTER[..],
        '\u{258F}' => &LEFT_ONE_EIGHTH[..],
        '\u{2590}' => &RIGHT_HALF[..],
        '\u{2594}' => &UPPER_ONE_EIGHTH[..],
        '\u{2595}' => &RIGHT_ONE_EIGHTH[..],
        '\u{2596}' => &QUADRANT_BL[..],
        '\u{2597}' => &QUADRANT_BR[..],
        '\u{2598}' => &QUADRANT_TL[..],
        '\u{2599}' => &QUADRANT_TL_BL_BR[..],
        '\u{259A}' => &QUADRANT_TL_BR[..],
        '\u{259B}' => &QUADRANT_TL_TR_BL[..],
        '\u{259C}' => &QUADRANT_TL_TR_BR[..],
        '\u{259D}' => &QUADRANT_TR[..],
        '\u{259E}' => &QUADRANT_TR_BL[..],
        '\u{259F}' => &QUADRANT_TR_BL_BR[..],
        _ => return None,
    };

    Some(
        fractions
            .iter()
            .filter_map(|fraction| fraction.to_rect(cell_rect, pixels_per_point))
            .collect(),
    )
}

impl RectFraction {
    fn to_rect(self, cell_rect: Rect, pixels_per_point: f32) -> Option<Rect> {
        let min = pos2(
            lerp(cell_rect.min.x, cell_rect.max.x, self.x0)
                .round_to_pixels(pixels_per_point),
            lerp(cell_rect.min.y, cell_rect.max.y, self.y0)
                .round_to_pixels(pixels_per_point),
        );
        let max = pos2(
            lerp(cell_rect.min.x, cell_rect.max.x, self.x1)
                .round_to_pixels(pixels_per_point),
            lerp(cell_rect.min.y, cell_rect.max.y, self.y1)
                .round_to_pixels(pixels_per_point),
        );

        (max.x > min.x && max.y > min.y).then_some(Rect::from_min_max(min, max))
    }
}

fn lerp(min: f32, max: f32, t: f32) -> f32 {
    min + (max - min) * t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_blocks_share_an_identical_edge() {
        let cell_rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(15.0, 15.0));
        let upper = block_element_rects('\u{2580}', cell_rect, 1.0).unwrap();
        let lower = block_element_rects('\u{2584}', cell_rect, 1.0).unwrap();

        assert_eq!(upper[0].max.y, lower[0].min.y);
    }

    #[test]
    fn full_block_fills_the_entire_cell() {
        let cell_rect = Rect::from_min_max(pos2(3.0, 5.0), pos2(19.0, 21.0));
        let rects = block_element_rects('\u{2588}', cell_rect, 1.0).unwrap();

        assert_eq!(rects, vec![cell_rect]);
    }

    #[test]
    fn quadrant_block_produces_multiple_rects() {
        let cell_rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(16.0, 16.0));
        let rects = block_element_rects('\u{259F}', cell_rect, 1.0).unwrap();

        assert_eq!(rects.len(), 3);
        assert_eq!(rects[0], Rect::from_min_max(pos2(8.0, 0.0), pos2(16.0, 8.0)));
        assert_eq!(rects[1], Rect::from_min_max(pos2(0.0, 8.0), pos2(8.0, 16.0)));
        assert_eq!(rects[2], Rect::from_min_max(pos2(8.0, 8.0), pos2(16.0, 16.0)));
    }
}
