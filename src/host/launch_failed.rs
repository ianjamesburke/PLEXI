//! Built-in error tile rendered when a capability pre-flight check fails.

use crate::app::app_trait::{App, AppCommand, AppRenderContext};
use crate::ui::style;
use egui::RichText;

/// Rendered as a `Pane::App(AppRuntime::Builtin(...))` when the host detects
/// that an app's declared capabilities cannot be satisfied by the current
/// config. No Python process is ever spawned for this pane.
pub struct LaunchFailedApp {
    pub app_id: String,
    pub missing: Vec<String>,
    /// Replaces the default "plexi config edit" footer. Use for Terminal apps
    /// where the fix is an install command, not a config change.
    pub footer: Option<String>,
}

impl App for LaunchFailedApp {
    fn type_id(&self) -> &'static str {
        "launch_failed"
    }

    fn display_name(&self) -> String {
        format!("Cannot launch {}", self.app_id)
    }

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &AppRenderContext<'_>) {
        let colors = ctx.colors;
        ui.vertical_centered(|ui| {
            ui.add_space(style::SPACE_XL * 2.0);
            ui.label(
                RichText::new(format!("Cannot launch {}", self.app_id))
                    .size(style::TEXT_BODY)
                    .color(colors.text_primary)
                    .strong(),
            );
            ui.add_space(style::SPACE_MD);
            for reason in &self.missing {
                ui.label(
                    RichText::new(format!("Missing: {reason}"))
                        .size(style::TEXT_CAPTION)
                        .color(colors.text_dim),
                );
            }
            ui.add_space(style::SPACE_SM);
            let footer_text = self
                .footer
                .as_deref()
                .unwrap_or("plexi config edit");
            ui.label(
                RichText::new(footer_text)
                    .size(style::TEXT_CAPTION)
                    .color(colors.text_dim)
                    .monospace(),
            );
        });
    }

    fn take_pending_commands(&mut self) -> Vec<AppCommand> {
        vec![]
    }
}
