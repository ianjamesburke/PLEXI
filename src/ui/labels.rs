use egui::RichText;

use crate::ui::style;
use crate::ui::theme::Colors;

pub(crate) fn section_header(ui: &mut egui::Ui, label: &str, colors: &Colors) {
    ui.label(
        RichText::new(label)
            .size(style::TEXT_CAPTION)
            .color(colors.accent),
    );
}

pub(crate) fn chrome_section<R>(
    ui: &mut egui::Ui,
    title: &str,
    colors: &Colors,
    body: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    section_header(ui, title, colors);
    ui.add_space(style::SPACE_XS);
    let result = body(ui);
    ui.add_space(style::SPACE_XL);
    result
}

pub(crate) fn description_label(ui: &mut egui::Ui, text: &str, colors: &Colors) {
    ui.add(
        egui::Label::new(
            RichText::new(text)
                .size(style::TEXT_HINT)
                .color(colors.text_dim),
        )
        .truncate(),
    );
}
