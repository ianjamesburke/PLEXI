use egui::{Color32, CornerRadius, Margin};

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
        // Use a transparent Frame purely for its inner_margin so callers
        // never have to think about row padding.
        egui::Frame::new()
            .inner_margin(Margin::symmetric(ROW_PAD_H as i8, ROW_PAD_V as i8))
            .show(ui, |ui| content(ui))
            .inner
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
