use egui::RichText;

use crate::ui::style;
use crate::ui::theme::Colors;

pub(crate) fn code_block<R>(
    ui: &mut egui::Ui,
    colors: &Colors,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    egui::Frame::new()
        .fill(colors.bg_darkest)
        .corner_radius(style::RADIUS_SM)
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add_contents(ui)
        })
}

pub(crate) fn copyable_command(
    ui: &mut egui::Ui,
    id: egui::Id,
    command: &str,
    colors: &Colors,
) -> egui::Response {
    code_block(ui, colors, |ui| {
        command_row(ui, id, command, style::TEXT_CAPTION, colors);
    })
    .response
}

pub(crate) fn compact_copyable_command(
    ui: &mut egui::Ui,
    id: egui::Id,
    command: &str,
    colors: &Colors,
) -> egui::Response {
    egui::Frame::new()
        .fill(colors.bg_darkest)
        .corner_radius(style::RADIUS_SM)
        .inner_margin(egui::Margin::symmetric(10, 3))
        .show(ui, |ui| {
            ui.spacing_mut().interact_size.y = crate::ui::toast::TOAST_CONTROL_H - 6.0;
            ui.spacing_mut().button_padding = egui::vec2(4.0, 2.0);
            ui.spacing_mut().item_spacing.x = style::SPACE_SM;
            let text_width = ui
                .fonts_mut(|f| {
                    f.layout_no_wrap(
                        command.to_owned(),
                        egui::FontId::monospace(style::TEXT_HINT),
                        colors.text_primary,
                    )
                })
                .size()
                .x;
            let copy_width =
                (style::BUTTON_H_MD - style::SPACE_SM).max(ui.spacing().interact_size.x);
            ui.set_width(
                ui.available_width()
                    .min(text_width + copy_width + style::SPACE_SM),
            );
            command_row(ui, id, command, style::TEXT_HINT, colors);
        })
        .response
}

// Allocate the copy action first; the command owns only the remaining lane.
fn command_row(ui: &mut egui::Ui, id: egui::Id, command: &str, size: f32, colors: &Colors) {
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            crate::ui::button::copy_button(ui, id, command);
            let label = crate::ui::text::elide(
                ui,
                command,
                egui::FontId::monospace(size),
                ui.available_width(),
                crate::ui::text::Side::Middle,
            );
            ui.add(
                egui::Label::new(
                    RichText::new(label)
                        .size(size)
                        .color(colors.text_primary)
                        .monospace(),
                )
                .wrap_mode(egui::TextWrapMode::Extend),
            )
            .on_hover_text(command);
        });
    });
}

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
        egui::Stroke::new(1.0_f32, colors.border),
        egui::StrokeKind::Inside,
    );
    crate::ui::list::paint_text_centered(
        ui,
        label,
        egui::FontId::proportional(style::TEXT_HINT),
        colors.text_on(fill),
        rect.center(),
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
        .stroke(egui::Stroke::new(1.0_f32, color))
        .corner_radius(style::RADIUS_BADGE)
        .inner_margin(egui::Margin::symmetric(
            style::BADGE_PAD_H as i8,
            style::BADGE_PAD_V as i8,
        ))
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
        .stroke(egui::Stroke::new(1.0_f32, colors.border))
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
        .stroke(egui::Stroke::new(1.0_f32, accent.gamma_multiply(0.45)))
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
