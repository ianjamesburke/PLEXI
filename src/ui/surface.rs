use egui::{Align2, RichText};

use crate::ui::style;
use crate::ui::theme::Colors;

pub(crate) fn color_swatch(
    ui: &mut egui::Ui,
    label: &str,
    fill: egui::Color32,
    colors: &Colors,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(84.0, 30.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, style::RADIUS_MD, fill);
    ui.painter().rect_stroke(
        rect,
        style::RADIUS_MD,
        egui::Stroke::new(1.0, colors.border),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(style::TEXT_HINT),
        colors.text_on(fill),
    );
    response
}

pub(crate) fn status_chip(ui: &mut egui::Ui, status: &str, colors: &Colors) -> egui::Response {
    let color = match status.to_ascii_lowercase().as_str() {
        "busy" | "running" => colors.accent,
        "crashed" | "hung" | "error" | "exited" => colors.danger,
        _ => colors.text_dim,
    };
    status_chip_with_color(ui, status, color, colors)
}

pub(crate) fn status_chip_with_color(
    ui: &mut egui::Ui,
    label: &str,
    color: egui::Color32,
    colors: &Colors,
) -> egui::Response {
    egui::Frame::new()
        .fill(colors.bg_active)
        .stroke(egui::Stroke::new(1.0, color))
        .corner_radius(style::RADIUS_BADGE)
        .inner_margin(egui::Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.label(RichText::new(label).size(style::TEXT_HINT).color(color));
        })
        .response
}

pub(crate) fn empty_state_panel(
    ui: &mut egui::Ui,
    title: &str,
    detail: Option<&str>,
    colors: &Colors,
) -> egui::Response {
    egui::Frame::new()
        .fill(colors.bg_toolbar)
        .stroke(egui::Stroke::new(1.0, colors.border))
        .corner_radius(style::RADIUS_MD)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new(title)
                        .size(style::TEXT_BODY)
                        .color(colors.text_primary),
                );
                if let Some(detail) = detail {
                    ui.label(
                        RichText::new(detail)
                            .size(style::TEXT_HINT)
                            .color(colors.text_dim),
                    );
                }
            });
        })
        .response
}

pub(crate) enum TrustTone {
    Neutral,
    Warning,
    Danger,
}

pub(crate) fn trust_decision_panel(
    ui: &mut egui::Ui,
    title: &str,
    detail: &str,
    tone: TrustTone,
    colors: &Colors,
) -> egui::Response {
    let accent = match tone {
        TrustTone::Neutral => colors.accent,
        TrustTone::Warning => colors.warning,
        TrustTone::Danger => colors.danger,
    };
    egui::Frame::new()
        .fill(colors.bg_toolbar)
        .stroke(egui::Stroke::new(1.0, accent.gamma_multiply(0.45)))
        .corner_radius(style::RADIUS_MD)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (dot_rect, _) =
                    ui.allocate_exact_size(egui::vec2(9.0, 9.0), egui::Sense::hover());
                ui.painter().circle_filled(dot_rect.center(), 4.5, accent);
                ui.add_space(style::SPACE_SM);
                ui.vertical(|ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new(title)
                                .size(style::TEXT_CAPTION)
                                .color(colors.text_primary)
                                .strong(),
                        )
                        .wrap(),
                    );
                    ui.add(
                        egui::Label::new(
                            RichText::new(detail)
                                .size(style::TEXT_HINT)
                                .color(colors.text_dim),
                        )
                        .wrap(),
                    );
                });
            });
        })
        .response
}
