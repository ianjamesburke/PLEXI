use crate::ui::style;
use crate::ui::theme::Colors;

const ROW_PAD_H: f32 = style::LIST_ROW_PAD_H;
const ROW_PAD_V: f32 = 8.0;

pub(crate) fn selectable_row<R>(
    ui: &mut egui::Ui,
    is_selected: bool,
    colors: &Colors,
    content: impl FnOnce(&mut egui::Ui) -> R,
) -> (egui::Response, R) {
    let bg_idx = ui.painter().add(egui::Shape::Noop);
    let outline_idx = ui.painter().add(egui::Shape::Noop);

    let scope = ui.scope(|ui| {
        ui.set_width(ui.available_width());
        ui.add_space(ROW_PAD_V);
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

    if is_selected {
        let [fill, outline] = crate::ui::list::selection_shapes(row_rect, colors);
        ui.painter().set(bg_idx, fill);
        ui.painter().set(outline_idx, outline);
    }

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
