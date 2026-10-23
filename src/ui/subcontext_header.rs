//! One header for every sub-context: portal cards and the bar after entering.
//!
//! A later flap or dive can reuse [`paint`] without taking this zoom model.

use crate::host::context_state::SubcontextHeader;
use crate::ui::style;
use crate::ui::theme::Colors;
use std::time::Duration;

pub fn format_elapsed(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    }
}

fn badge_fill(label: &str, colors: &Colors) -> egui::Color32 {
    if label.ends_with("needs you") {
        colors.state_needs_you
    } else if label.ends_with("failed") {
        colors.state_error
    } else if label.ends_with("done") {
        colors.state_waiting
    } else {
        colors.state_working
    }
}

/// Paint the shared header. The name, owner, each count, and the elapsed time
/// are accessibility labels.
pub fn paint(ui: &mut egui::Ui, header: &SubcontextHeader, colors: &Colors) {
    let logged_id = egui::Id::new(("subcontext_header", header.context_id));
    let logged = ui.ctx().data(|data| data.get_temp::<bool>(logged_id).unwrap_or(false));
    if !logged {
        log::info!(
            "subcontext_header: ctx={} name={} owner={:?} counts={:?} elapsed={:?}",
            header.context_id,
            header.name,
            header.owner,
            header.counts_label(),
            header.elapsed_secs,
        );
        ui.ctx().data_mut(|data| data.insert_temp(logged_id, true));
    }
    if header.elapsed_secs.is_some() {
        let predicted = ui.input(|input| input.predicted_dt).max(0.0);
        ui.ctx().request_repaint_after(
            Duration::from_secs(1) + Duration::from_secs_f32(predicted),
        );
    }
    ui.horizontal(|ui| {
        ui.set_min_height(style::TEXT_BODY + style::SPACE_SM);
        let name = ui.label(
            egui::RichText::new(&header.name)
                .strong()
                .size(style::TEXT_BODY)
                .color(colors.text_primary),
        );
        name.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &header.name)
        });
        if let Some(owner) = &header.owner {
            let text = format!("owner {owner}");
            let response = ui.label(
                egui::RichText::new(&text)
                    .size(style::TEXT_HINT)
                    .color(colors.text_dim),
            );
            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &text));
        }
        for part in header.counts_label().unwrap_or_default().split(" \u{b7} ") {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let fill = badge_fill(part, colors);
            let response = crate::ui::badge::badge(ui, part, fill, colors);
            let label = part.to_string();
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &label)
            });
        }
        if let Some(secs) = header.elapsed_secs {
            let text = format_elapsed(secs);
            let response = ui.label(
                egui::RichText::new(&text)
                    .size(style::TEXT_HINT)
                    .color(colors.text_dim),
            );
            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &text));
        }
    });
}
