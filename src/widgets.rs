use egui::{Color32, CornerRadius, Pos2, Stroke, StrokeKind, Vec2};

use crate::style;
use crate::theme::Colors;

/// Consistent inner padding applied to every selectable row.
const ROW_PAD_H: f32 = 10.0;
const ROW_PAD_V: f32 = 6.0;

/// Renders a highlighted selectable row whose height is determined by its
/// content. The widget owns horizontal and vertical padding so callers
/// render only raw content without spacing boilerplate.
///
/// Returns `(response, content_return_value)`. Check `response.clicked()` to
/// detect activation and `response.hovered()` to drive keyboard selection.
///
/// Pattern: reserve a shape slot before rendering content, then fill it after
/// measuring — so the highlight sits behind the text in Z-order even though
/// the rect is only known after layout.
pub(crate) fn selectable_row<R>(
    ui: &mut egui::Ui,
    is_selected: bool,
    colors: &Colors,
    content: impl FnOnce(&mut egui::Ui) -> R,
) -> (egui::Response, R) {
    let fill: Color32 = if is_selected {
        colors.bg_active
    } else {
        Color32::TRANSPARENT
    };

    let bg_idx = ui.painter().add(egui::Shape::Noop);

    let scope = ui.scope(|ui| {
        ui.set_width(ui.available_width());
        ui.add_space(ROW_PAD_V);
        // Left-indent the content without using Frame (Frame inner_margin
        // interferes with ScrollArea height measurement).
        let inner = ui
            .horizontal(|ui| {
                ui.add_space(ROW_PAD_H);
                ui.vertical(|ui| content(ui)).inner
            })
            .inner;
        ui.add_space(ROW_PAD_V);
        inner
    });

    let row_rect = scope.response.rect;

    ui.painter().set(
        bg_idx,
        egui::Shape::rect_filled(row_rect, CornerRadius::same(4), fill),
    );

    let response = ui.interact(
        row_rect,
        scope.response.id.with("_selectable_row_hit"),
        egui::Sense::click(),
    );

    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    (response, scope.inner)
}

// ── Keycap primitive ─────────────────────────────────────────────────────
//
// The notification modal shipped with "⌘[/⌘] to cycle" rendered as flat
// text and it looks like run-on typography. Native macOS menus render
// each key as a distinct visual chip; we do the same. One rendering rule,
// one primitive, apply everywhere shortcuts appear.
//
// Usage:
//     key_combo(ui, &["⌘", "["], colors);        // single shortcut
//     key_combo_list(ui, &[["⌘", "["], ["⌘", "]"]], colors, "to cycle");

/// Padding around the key label text inside the chip.
const KEYCAP_PAD_H: f32 = 5.0;
const KEYCAP_PAD_V: f32 = 1.0;
/// Minimum chip width — keeps single-char keys like `[` from looking cramped.
const KEYCAP_MIN_W: f32 = 16.0;

/// Render a single keycap chip. Allocates its own exact-size rect and
/// returns the egui Response so callers can compose with other widgets.
pub(crate) fn key_chip(ui: &mut egui::Ui, label: &str, colors: &Colors) -> egui::Response {
    let font_id = egui::FontId::monospace(style::TEXT_HINT);
    let galley = ui
        .fonts(|f| f.layout_no_wrap(label.to_string(), font_id, colors.text_dim));
    let text_w = galley.size().x;
    let text_h = galley.size().y;
    let chip_w = (text_w + KEYCAP_PAD_H * 2.0).max(KEYCAP_MIN_W);
    let chip_h = text_h + KEYCAP_PAD_V * 2.0;
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(chip_w, chip_h),
        egui::Sense::hover(),
    );
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(3), colors.bg_active);
    painter.rect_stroke(
        rect,
        CornerRadius::same(3),
        Stroke::new(1.0, colors.border),
        StrokeKind::Inside,
    );
    // Centre the text horizontally inside the chip.
    let text_pos = Pos2::new(
        rect.center().x - text_w / 2.0,
        rect.min.y + KEYCAP_PAD_V,
    );
    painter.galley(text_pos, galley, colors.text_dim);
    response
}

/// Gap between chips within a single combo (e.g. ⌘ + [).
const INTRA_COMBO_GAP: f32 = 2.0;
/// Gap between separate combos in a list (e.g. ⌘[ vs ⌘]).
const INTER_COMBO_GAP: f32 = 10.0;
/// Gap between the last combo/chip and the trailing description text.
const TRAILING_GAP: f32 = 10.0;

/// Render several chips forming a single key combo (e.g. ["⌘", "["]).
/// Chips are laid out left-to-right with `INTRA_COMBO_GAP` between them.
pub(crate) fn key_combo(ui: &mut egui::Ui, keys: &[&str], colors: &Colors) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = INTRA_COMBO_GAP;
        for key in keys {
            key_chip(ui, key, colors);
        }
    });
}

/// Render multiple combos inline with an optional trailing description label.
///
/// Layout:    [⌘][[] ··inter·· [⌘][] ··trailing·· "cycle"
pub(crate) fn key_combo_list(
    ui: &mut egui::Ui,
    combos: &[&[&str]],
    trailing: Option<&str>,
    colors: &Colors,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for (i, keys) in combos.iter().enumerate() {
            if i > 0 {
                ui.add_space(INTER_COMBO_GAP);
            }
            // Chips within a combo sit INTRA_COMBO_GAP apart.
            for (j, key) in keys.iter().enumerate() {
                if j > 0 {
                    ui.add_space(INTRA_COMBO_GAP);
                }
                key_chip(ui, key, colors);
            }
        }
        if let Some(text) = trailing {
            ui.add_space(TRAILING_GAP);
            ui.label(
                egui::RichText::new(text)
                    .size(style::TEXT_HINT)
                    .color(colors.text_dim),
            );
        }
    });
}
