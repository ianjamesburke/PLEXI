use egui::{Label, Response, RichText, Ui};

use crate::ui::style;
use crate::ui::theme::{self, Colors};

fn wrapped_label(text: RichText) -> Label {
    Label::new(text).wrap()
}

pub(crate) fn modal_title(ui: &mut Ui, text: &str, colors: &Colors) -> Response {
    ui.add(wrapped_label(
        RichText::new(text)
            .font(theme::font_medium(style::TEXT_TITLE))
            .color(colors.text_primary),
    ))
}

pub(crate) fn body(ui: &mut Ui, text: impl Into<String>, colors: &Colors) -> Response {
    ui.add(wrapped_label(
        RichText::new(text.into())
            .size(style::TEXT_HINT)
            .color(colors.text_primary),
    ))
}

pub(crate) fn body_strong(ui: &mut Ui, text: impl Into<String>, colors: &Colors) -> Response {
    ui.add(wrapped_label(
        RichText::new(text.into())
            .size(style::TEXT_BODY)
            .color(colors.text_primary)
            .strong(),
    ))
}

pub(crate) fn caption(ui: &mut Ui, text: impl Into<String>, colors: &Colors) -> Response {
    ui.add(wrapped_label(
        RichText::new(text.into())
            .size(style::TEXT_HINT)
            .color(colors.text_dim),
    ))
}

pub(crate) fn caption_strong(ui: &mut Ui, text: impl Into<String>, colors: &Colors) -> Response {
    ui.add(wrapped_label(
        RichText::new(text.into())
            .size(style::TEXT_HINT)
            .color(colors.text_dim)
            .strong(),
    ))
}
