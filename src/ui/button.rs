use egui::RichText;

use crate::ui::style;
use crate::ui::theme::Colors;

pub(crate) enum ButtonKind {
    Primary,
    Secondary,
    Accent,
    Danger,
}

pub(crate) fn chrome_button(
    ui: &mut egui::Ui,
    label: &str,
    kind: ButtonKind,
    colors: &Colors,
    min_width: f32,
) -> egui::Response {
    chrome_button_sized(ui, label, kind, colors, min_width, style::BUTTON_H_MD)
}

pub(crate) fn chrome_button_sized(
    ui: &mut egui::Ui,
    label: &str,
    kind: ButtonKind,
    colors: &Colors,
    min_width: f32,
    height: f32,
) -> egui::Response {
    let (fill, text_color) = match kind {
        ButtonKind::Primary => (colors.bg_active, colors.text_primary),
        ButtonKind::Secondary => (colors.bg_active, colors.text_dim),
        ButtonKind::Accent => (colors.accent, colors.text_on(colors.accent)),
        ButtonKind::Danger => (colors.danger, colors.text_on(colors.danger)),
    };

    ui.add(
        egui::Button::new(
            RichText::new(label)
                .size(style::TEXT_CAPTION)
                .color(text_color),
        )
        .fill(fill)
        .min_size(egui::vec2(min_width, height)),
    )
    .on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub(crate) fn toolbar_button(
    ui: &mut egui::Ui,
    label: impl Into<egui::WidgetText>,
    hover_text: &str,
) -> egui::Response {
    ui.add(
        egui::Button::new(label)
            .frame(false)
            .min_size(egui::vec2(style::BUTTON_H_MD - style::SPACE_SM, 0.0)),
    )
    .on_hover_cursor(egui::CursorIcon::PointingHand)
    .on_hover_text(hover_text)
}

pub(crate) fn icon_button(
    ui: &mut egui::Ui,
    icon: &str,
    hover_text: &str,
    colors: &Colors,
) -> egui::Response {
    toolbar_button(
        ui,
        RichText::new(icon)
            .size(style::TEXT_CAPTION)
            .color(colors.text_dim),
        hover_text,
    )
}

pub(crate) fn copy_button(ui: &mut egui::Ui, id: egui::Id, text: &str) -> egui::Response {
    let now = ui.ctx().input(|i| i.time);
    let copied_at: Option<f64> = ui.ctx().memory(|m| m.data.get_temp(id));
    let just_copied = copied_at.map_or(false, |t| now - t < 2.0);
    let icon = if just_copied { "✓" } else { "\u{f0c5}" };
    let resp = ui
        .add(egui::Button::new(RichText::new(icon).size(style::TEXT_CAPTION)).frame(false))
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(format!("Copy `{text}`"));
    if resp.clicked() && !just_copied {
        ui.ctx().copy_text(text.to_string());
        ui.ctx().memory_mut(|m| m.data.insert_temp(id, now));
    }
    if just_copied {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(100));
    }
    resp
}
