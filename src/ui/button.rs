use egui::{Align2, Color32, RichText, Vec2};

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

/// Full-width option button for `NotifyKind::Choice` lists.
///
/// Paints a fixed-height rect with a centered label and an optional
/// right-gutter shortcut hint. The focused option receives an accent fill;
/// non-focused options lift slightly on hover.
pub(crate) fn choice_button(
    ui: &mut egui::Ui,
    label: &str,
    shortcut_hint: &str,
    focused: bool,
    colors: &Colors,
) -> egui::Response {
    let width = ui.available_width();
    let (rect, resp) =
        ui.allocate_exact_size(Vec2::new(width, style::BUTTON_H_LG), egui::Sense::click());

    let (bg, fg, hint_color) = if focused {
        (colors.accent, Color32::BLACK, Color32::from_black_alpha(140))
    } else {
        (colors.bg_hover, colors.text_primary, colors.text_dim)
    };
    let actual_bg = if resp.hovered() && !focused {
        colors.bg_active
    } else {
        bg
    };

    let painter = ui.painter();
    painter.rect_filled(rect, style::RADIUS_MD, actual_bg);
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(style::TEXT_BODY),
        fg,
    );
    if !shortcut_hint.is_empty() {
        painter.text(
            egui::pos2(rect.right() - 18.0, rect.center().y),
            Align2::RIGHT_CENTER,
            shortcut_hint,
            egui::FontId::proportional(style::TEXT_CAPTION),
            hint_color,
        );
    }
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp
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
