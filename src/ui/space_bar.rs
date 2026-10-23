//! Coordinate bar and the compact summary a pane shows for the space it holds.
//!
//! Team runs, agent calls, and pane-created sub-contexts all paint
//! [`paint_space_summary`]. The coordinate bar is the path of human names.

use crate::host::context_state::ContextStatus;
use crate::host::space_coord::{SpaceFace, UrgentKind};
use crate::ui::style;
use crate::ui::theme::Colors;

/// Height of the coordinate bar reserved above a nested space.
pub const BAR_H: f32 = style::TEXT_BODY + style::SPACE_SM + style::SPACE_XS;

/// Height of the compact space summary (title plus the counts line).
pub const SUMMARY_H: f32 = style::TEXT_BODY + style::TEXT_HINT + style::SPACE_MD;

pub fn paint_coordinate_bar(ui: &mut egui::Ui, rect: egui::Rect, path: &str, colors: &Colors) {
    ui.painter().rect_filled(rect, 0.0, colors.bg_toolbar);
    ui.painter().line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        egui::Stroke::new(1.0_f32, colors.border),
    );
    let inner = rect.shrink2(egui::vec2(style::SPACE_SM, style::SPACE_XS));
    if !inner.is_positive() {
        return;
    }
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .id_salt("space-coordinate"),
    );
    let response = child.add(
        egui::Label::new(
            egui::RichText::new(path)
                .size(style::TEXT_BODY)
                .strong()
                .color(colors.text_primary),
        )
        .wrap_mode(egui::TextWrapMode::Truncate),
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, false, path));
}

/// The summary strip. Callers advance with the returned response's rect.
pub fn paint_space_summary(ui: &mut egui::Ui, face: &SpaceFace, colors: &Colors) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), SUMMARY_H),
        egui::Sense::click(),
    );
    ui.painter().rect_filled(rect, 0.0, colors.pane_header_bg());
    let inner = rect.shrink2(egui::vec2(style::SPACE_SM, style::SPACE_XS));
    if inner.is_positive() {
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(inner)
                .id_salt("space-summary"),
        );
        child.vertical(|ui| {
            ui.set_width(inner.width());
            crate::ui::typography::body_strong(ui, &face.title, colors);
            let color = urgent_color(face.urgent, colors);
            ui.add(egui::Label::new(
                egui::RichText::new(face.summary_line())
                    .size(style::TEXT_HINT)
                    .strong()
                    .color(color),
            ));
        });
    }
    let label = face.label();
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
    response
        .on_hover_text("Dive into this space (Cmd+Enter). Surface with Cmd+Esc or Cmd+Backspace.")
}

fn urgent_color(kind: UrgentKind, colors: &Colors) -> egui::Color32 {
    let status = match kind {
        UrgentKind::NeedsYou => ContextStatus::NeedsYou,
        UrgentKind::Blocked => ContextStatus::Blocked,
        UrgentKind::Error => ContextStatus::Error,
        UrgentKind::Working => ContextStatus::Working,
        UrgentKind::Done => ContextStatus::Done,
        UrgentKind::Idle => ContextStatus::Idle,
    };
    crate::ui::activity::context_status_color(&status, colors).unwrap_or(colors.text_dim)
}
