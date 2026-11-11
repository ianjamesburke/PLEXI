//! Shared single-choice dropdown. Egui owns popup placement and dismissal;
//! the kit owns the trigger sizing and every option's state-stable geometry.

use super::{list::ListRow, style, theme::Colors};

#[expect(
    clippy::disallowed_methods,
    reason = "the kit dropdown is the sole ComboBox construction boundary"
)]
/// `options` is built only while the popup is open, so callers may resolve
/// them from disk without paying for it on every closed frame.
pub(crate) fn select_value<'a, T: PartialEq, O: IntoIterator<Item = (T, &'a str)>>(
    ui: &mut egui::Ui,
    id_salt: impl std::hash::Hash,
    selected_text: &str,
    value: &mut T,
    options: impl FnOnce() -> O,
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
                for (option, label) in options() {
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

#[cfg(test)]
mod tests {
    use egui_kittest::kittest::Queryable;
    use std::cell::Cell;

    #[test]
    fn options_are_resolved_only_while_the_popup_is_open() {
        let colors = crate::ui::theme::Colors::from_config(
            &crate::ui::theme::preset_colors("plexi-dusk").unwrap(),
        );
        let calls = Cell::new(0_u32);
        let mut initialized = false;
        let mut h = egui_kittest::Harness::new_ui_state(
            |ui, value: &mut &'static str| {
                if !initialized {
                    crate::ui::theme::setup_fonts(ui.ctx());
                    crate::ui::theme::setup_style(ui.ctx(), &colors);
                    initialized = true;
                    return;
                }
                let selected = *value;
                super::select_value(
                    ui,
                    "d",
                    selected,
                    value,
                    || {
                        calls.set(calls.get() + 1);
                        [("A", "A"), ("B", "B")]
                    },
                    &colors,
                );
            },
            "A",
        );
        h.run();
        h.run();
        assert_eq!(calls.get(), 0, "closed dropdown must not build options");
        h.get_by_role(egui::accesskit::Role::ComboBox).click();
        h.run();
        assert!(calls.get() > 0, "open dropdown builds options");
    }
}
