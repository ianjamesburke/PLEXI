use egui::{Align2, Color32, RichText, Vec2};

use crate::ui::style;
use crate::ui::theme::Colors;

#[derive(Clone, Copy)]
pub(crate) enum ButtonKind {
    Primary,
    Secondary,
    Accent,
    Danger,
}

#[derive(Clone, Copy)]
pub(crate) struct ChromeButtonState {
    pub(crate) disabled: bool,
    pub(crate) down: bool,
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
    let visuals = chrome_button_visuals(
        kind,
        colors,
        ChromeButtonState {
            disabled: !ui.is_enabled(),
            down: false,
        },
    );

    tactile_button(
        ui,
        egui::Button::new(
            RichText::new(label)
                .size(style::TEXT_CAPTION)
                .color(visuals.text),
        )
        .fill(visuals.fill)
        .min_size(egui::vec2(min_width, height)),
        chrome_button_visuals(
            kind,
            colors,
            ChromeButtonState {
                disabled: !ui.is_enabled(),
                down: true,
            },
        )
        .fill,
    )
    .on_hover_cursor(egui::CursorIcon::PointingHand)
}

#[derive(Clone, Copy)]
struct ChromeButtonVisuals {
    fill: Color32,
    text: Color32,
}

fn chrome_button_visuals(
    kind: ButtonKind,
    colors: &Colors,
    state: ChromeButtonState,
) -> ChromeButtonVisuals {
    let base_fill = match kind {
        ButtonKind::Primary | ButtonKind::Secondary => colors.bg_active,
        ButtonKind::Accent => colors.accent,
        ButtonKind::Danger => colors.danger,
    };
    let fill = if state.disabled {
        colors.bg_active.gamma_multiply(0.55)
    } else if state.down {
        match kind {
            ButtonKind::Accent | ButtonKind::Danger => base_fill.gamma_multiply(0.85),
            ButtonKind::Primary | ButtonKind::Secondary => colors.bg_hover,
        }
    } else {
        base_fill
    };
    let text = if state.disabled {
        colors.text_dim
    } else {
        match kind {
            ButtonKind::Accent | ButtonKind::Danger => colors.text_on(base_fill),
            ButtonKind::Secondary => colors.text_dim,
            ButtonKind::Primary => colors.text_primary,
        }
    };
    ChromeButtonVisuals { fill, text }
}

pub(crate) fn toolbar_button(
    ui: &mut egui::Ui,
    label: impl Into<egui::WidgetText>,
    hover_text: &str,
) -> egui::Response {
    tactile_button(
        ui,
        egui::Button::new(label)
            .frame(false)
            .min_size(egui::vec2(style::BUTTON_H_MD - style::SPACE_SM, 0.0))
            .sense(egui::Sense::CLICK),
        ui.visuals().widgets.active.bg_fill,
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
        (
            colors.accent,
            Color32::BLACK,
            Color32::from_black_alpha(140),
        )
    } else {
        (colors.bg_hover, colors.text_primary, colors.text_dim)
    };
    let actual_bg = if resp.hovered() && !focused {
        colors.bg_active
    } else {
        bg
    };

    let amount = style::press_amount(&resp);
    let rect = egui::Rect::from_center_size(
        rect.center(),
        rect.size() * (1.0 - (1.0 - style::PRESS_SCALE) * amount),
    );
    let actual_bg = actual_bg.lerp_to_gamma(actual_bg.gamma_multiply(0.85), amount);
    let painter = ui.painter();
    painter.rect_filled(rect, style::RADIUS_MD, actual_bg);
    crate::ui::snap::text_snapped(
        painter,
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(style::TEXT_BODY),
        fg,
    );
    if !shortcut_hint.is_empty() {
        crate::ui::snap::text_snapped(
            painter,
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
    let just_copied = copied_at.is_some_and(|t| now - t < 2.0);
    let feedback = style::micro_motion(
        ui.ctx(),
        id.with("copy_feedback"),
        just_copied,
        style::DURATION_MICRO_MS,
    );
    let resp = ui
        .add(egui::Button::new("").frame(false).min_size(egui::vec2(
            style::BUTTON_H_MD - style::SPACE_SM,
            style::BUTTON_H_MD - style::SPACE_SM,
        )))
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(format!("Copy `{text}`"));
    resp.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), "Copy")
    });
    for (icon, opacity, color) in [
        ("\u{f0c5}", 1.0 - feedback, ui.visuals().text_color()),
        ("✓", feedback, ui.visuals().selection.stroke.color),
    ] {
        if opacity > 0.0 {
            crate::ui::snap::text_snapped(
                ui.painter(),
                resp.rect.center(),
                Align2::CENTER_CENTER,
                icon,
                egui::FontId::proportional(style::TEXT_CAPTION),
                color.gamma_multiply(opacity),
            );
        }
    }
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

/// Keep egui's sizing, accessibility, focus and click semantics, applying the
/// press transform only to the shapes this button painted, never its hit box.
pub(crate) fn tactile_button(
    ui: &mut egui::Ui,
    button: egui::Button<'_>,
    active_fill: Color32,
) -> egui::Response {
    let layer = ui.layer_id();
    let start = ui
        .ctx()
        .graphics(|g| g.get(layer).map_or(0, |p| p.all_entries().len()));
    let backdrop = ui.painter().add(egui::Shape::Noop);
    let response = ui.add(button);
    let amount = style::press_amount(&response);
    if amount > 0.0 {
        let scale = 1.0 - (1.0 - style::PRESS_SCALE) * amount;
        let transform = egui::emath::TSTransform::from_translation(
            response.rect.center().to_vec2() * (1.0 - scale),
        ) * egui::emath::TSTransform::from_scaling(scale);
        ui.ctx().graphics_mut(|g| {
            if let Some(list) = g.get_mut(layer) {
                let end = list.all_entries().len();
                let mut has_fill = false;
                for index in start..end {
                    list.mutate_shape(egui::layers::ShapeIdx(index), |clipped| {
                        if let egui::Shape::Rect(shape) = &mut clipped.shape {
                            has_fill = true;
                            shape.fill = shape.fill.lerp_to_gamma(active_fill, amount);
                        }
                    });
                }
                if !has_fill {
                    list.set(
                        backdrop,
                        ui.clip_rect(),
                        egui::Shape::rect_filled(
                            response.rect,
                            style::RADIUS_SM,
                            active_fill.gamma_multiply(amount),
                        ),
                    );
                }
                list.transform_range(
                    egui::layers::ShapeIdx(start),
                    egui::layers::ShapeIdx(end),
                    transform,
                );
            }
        });
    }
    response
}
