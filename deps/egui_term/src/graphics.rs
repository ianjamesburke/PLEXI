use egui::emath::GuiRounding;
use egui::epaint::{CircleShape, RectShape};
use egui::{pos2, Color32, CornerRadius, Rect, Shape, Stroke};

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
const QUADRANT_BL: [RectFraction; 1] =
    [RectFraction::new(0.0, HALF, HALF, 1.0)];
const QUADRANT_BR: [RectFraction; 1] =
    [RectFraction::new(HALF, 1.0, HALF, 1.0)];
const QUADRANT_TL: [RectFraction; 1] =
    [RectFraction::new(0.0, HALF, 0.0, HALF)];
const QUADRANT_TR: [RectFraction; 1] =
    [RectFraction::new(HALF, 1.0, 0.0, HALF)];
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

// Render corner/line-drawing characters that are commonly missing from fonts as
// precise geometric shapes. This covers characters like U+23BF (⎿, used by Claude
// Code for conversation tree connectors) that aren't in most monospace fonts.
fn corner_shapes(c: char, cell_rect: Rect, fg: Color32) -> Option<Vec<Shape>> {
    let cx = cell_rect.center().x;
    let cy = cell_rect.center().y;
    let top = cell_rect.min.y;
    let bottom = cell_rect.max.y;
    let left = cell_rect.min.x;
    let right = cell_rect.max.x;
    let stroke = Stroke::new(1.0, fg);

    // Each arm goes from the cell center to the midpoint of an edge.
    // Names: U=up (center→top), D=down (center→bottom), L=left, R=right.
    let seg_u = || Shape::line_segment([pos2(cx, cy), pos2(cx, top)], stroke);
    let seg_d =
        || Shape::line_segment([pos2(cx, cy), pos2(cx, bottom)], stroke);
    let seg_l = || Shape::line_segment([pos2(cx, cy), pos2(left, cy)], stroke);
    let seg_r = || Shape::line_segment([pos2(cx, cy), pos2(right, cy)], stroke);

    let shapes = match c {
        // U+23BF ⎿ BOTTOM RIGHT CORNER — Claude Code conversation tree tip (L-bracket)
        // Vertical arm goes UP, horizontal arm goes RIGHT.
        '\u{23BF}' => vec![seg_u(), seg_r()],

        // Arc box-drawing corners (╭╮╯╰) — rendered as two straight arms.
        // These are in JBM Nerd Font but rendering geometrically gives pixel-perfect results.
        '\u{256D}' => vec![seg_d(), seg_r()], // ╭ down + right
        '\u{256E}' => vec![seg_d(), seg_l()], // ╮ down + left
        '\u{256F}' => vec![seg_u(), seg_l()], // ╯ up   + left
        '\u{2570}' => vec![seg_u(), seg_r()], // ╰ up   + right

        _ => return None,
    };

    Some(shapes)
}

// Render circle/dot characters geometrically so they stay within cell bounds.
// Fallback fonts often render these wider than monospace cell width, causing
// clipping at column 0 where the painter's clip rect starts.
fn circle_shape(c: char, cell_rect: Rect, fg: Color32) -> Option<Shape> {
    let center = cell_rect.center();
    // Use cell width as the constraining dimension (cells are taller than wide).
    let half_w = cell_rect.width() / 2.0;

    match c {
        // ● BLACK CIRCLE — Claude Code status indicators, Starship prompt dots
        '\u{25CF}' => {
            let radius = half_w * 0.72;
            Some(Shape::Circle(CircleShape::filled(center, radius, fg)))
        },
        // ○ WHITE CIRCLE — outline variant
        '\u{25CB}' => {
            let radius = half_w * 0.72;
            let stroke = Stroke::new(1.0, fg);
            Some(Shape::Circle(CircleShape::stroke(center, radius, stroke)))
        },
        // ◉ FISHEYE — filled circle with smaller inner circle
        '\u{25C9}' => {
            let radius = half_w * 0.72;
            let stroke = Stroke::new(1.0, fg);
            Some(Shape::Circle(CircleShape::stroke(center, radius, stroke)))
        },
        // ⏺ BLACK CIRCLE FOR RECORD
        '\u{23FA}' => {
            let radius = half_w * 0.72;
            Some(Shape::Circle(CircleShape::filled(center, radius, fg)))
        },
        // • BULLET (smaller dot)
        '\u{2022}' => {
            let radius = half_w * 0.38;
            Some(Shape::Circle(CircleShape::filled(center, radius, fg)))
        },
        // ◦ WHITE BULLET (smaller outline dot)
        '\u{25E6}' => {
            let radius = half_w * 0.38;
            let stroke = Stroke::new(1.0, fg);
            Some(Shape::Circle(CircleShape::stroke(center, radius, stroke)))
        },
        _ => None,
    }
}

pub(crate) fn maybe_push_graphics_element(
    shapes: &mut Vec<Shape>,
    c: char,
    cell_rect: Rect,
    fg: Color32,
    pixels_per_point: f32,
) -> bool {
    if let Some(corner_shapes) = corner_shapes(c, cell_rect, fg) {
        shapes.extend(corner_shapes);
        return true;
    }

    if let Some(circle) = circle_shape(c, cell_rect, fg) {
        shapes.push(circle);
        return true;
    }

    let Some(rects) = block_element_rects(c, cell_rect, pixels_per_point)
    else {
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
            .filter_map(|fraction| {
                fraction.to_rect(cell_rect, pixels_per_point)
            })
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
