//! Shared single-choice dropdown. Egui owns popup placement and dismissal;
//! the kit owns the trigger sizing and every option's state-stable geometry.

use super::{list::ListRow, style, theme::Colors};

#[expect(
    clippy::disallowed_methods,
    reason = "the kit dropdown is the sole ComboBox construction boundary"
)]
pub(crate) fn select_value<'a, T: PartialEq>(
    ui: &mut egui::Ui,
    id_salt: impl std::hash::Hash,
    selected_text: &str,
    value: &mut T,
    options: impl IntoIterator<Item = (T, &'a str)>,
    colors: &Colors,
) -> egui::Response {
    ui.scope(|ui| {
        ui.spacing_mut().interact_size.y = style::BUTTON_H_MD;
        let mut changed = false;
        let mut response = egui::ComboBox::from_id_salt(id_salt)
            .width(style::MENU_MIN_W)
            .selected_text(
                egui::RichText::new(selected_text)
                    .font(super::theme::font_medium(style::TEXT_CONTROL)),
            )
            .truncate()
            .show_ui(ui, |ui| {
                ui.set_width(style::MENU_MIN_W);
                for (option, label) in options {
                    if option_row(ui, label, *value == option, colors).clicked() {
                        changed = *value != option;
                        *value = option;
                        ui.close();
                    }
                }
            })
            .response;
        if changed {
            response.mark_changed();
            log::info!("ui: kit dropdown selection changed id={:?}", response.id);
        }
        response
    })
    .inner
}

pub(super) fn option_row(
    ui: &mut egui::Ui,
    label: &str,
    selected: bool,
    colors: &Colors,
) -> egui::Response {
    ListRow::new(label)
        .dense()
        .selected(selected)
        .show(ui, colors)
        .response()
}
