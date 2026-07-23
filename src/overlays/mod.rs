use crate::app::app_trait::AppCommand;
use crate::spatial::tiling::PaneId;
use crate::ui::style;
use crate::ui::theme::Colors;
use egui::{Align, Align2, Color32, CornerRadius, Layout, RichText, Stroke, Vec2};

pub(crate) mod command_palette;
pub(crate) mod confirmations;
pub(crate) mod misc;
pub(crate) mod notes_picker;
pub(crate) mod notes_triage;
pub(crate) mod notification_modal;
pub(crate) mod quick_note;
pub(crate) mod setup;
pub(crate) mod toolbar;
pub(crate) mod ui_gallery;

use crate::app::PlexiApp;

pub(crate) const MODAL_WIDTH: f32 = 400.0;
pub(crate) const R6: CornerRadius = CornerRadius::same(6);

/// Show a native folder picker through the shared `PickerService` seam.
/// Blocks the calling (background) thread; must NOT be called on the main thread.
pub(crate) fn pick_folder() -> Option<std::path::PathBuf> {
    use crate::host::services::{default_picker_service, FilePickOutcome, FilePickRequest};
    let outcome = default_picker_service().pick(&FilePickRequest {
        filter: Vec::new(),
        multiple: false,
        mode: crate::app_protocol::FilePickerMode::Folder,
    });
    match outcome {
        FilePickOutcome::Picked(paths) => paths.into_iter().next(),
        FilePickOutcome::Cancelled => None,
    }
}

pub(crate) fn draw_contact_footer(ui: &mut egui::Ui, colors: &Colors) {
    ui.vertical_centered(|ui| {
        ui.label(
            RichText::new("If you have any ideas, want to help, or just want to say what's up...")
                .size(style::TEXT_CAPTION)
                .color(colors.text_dim),
        );
        ui.add_space(style::SPACE_SM / 2.0);
        {
            let email = "ADHDISNTREAL@GMAIL.COM";
            let mailto = "mailto:ADHDisntreal@gmail.com";
            let font_id = egui::FontId::proportional(style::TEXT_CAPTION);
            let email_w = ui.fonts_mut(|f| {
                f.layout_no_wrap(email.to_string(), font_id, colors.text_dim)
                    .size()
                    .x
            });
            let btn_w = 24.0;
            let gap = ui.spacing().item_spacing.x;
            let pad = ((ui.available_width() - email_w - gap - btn_w) / 2.0).max(0.0);
            ui.horizontal(|ui| {
                ui.add_space(pad);
                ui.hyperlink_to(
                    RichText::new(email)
                        .size(style::TEXT_CAPTION)
                        .color(colors.text_dim),
                    mailto,
                );
                crate::ui::button::copy_button(
                    ui,
                    egui::Id::new("shortcuts_email_copy"),
                    "ADHDisntreal@gmail.com",
                );
            });
        }
        ui.add_space(style::SPACE_SM / 2.0);
        ui.hyperlink_to(
            RichText::new("❤️  Support the project")
                .size(style::TEXT_CAPTION)
                .color(colors.text_dim),
            "https://buymeacoffee.com/ianjamesbu8",
        );
    });
}
