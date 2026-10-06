//! Command view pane. It paints the same projection `plexi command-view --json`
//! returns, one row per line, so a waiting lead stays on screen. Steering stays
//! on the CLI so a click here cannot skip the gate.

use crate::app::app_trait::{App, AppRenderContext};

pub struct CommandViewApp;

impl App for CommandViewApp {
    #[cfg(test)]
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn type_id(&self) -> &'static str {
        "command-view"
    }

    fn display_name(&self) -> String {
        "Command".to_string()
    }

    fn ui(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &AppRenderContext<'_>,
        _pending_click: Option<crate::host::pane::PendingPaneClick>,
    ) {
        let colors = ctx.colors;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .id_salt("command-view")
            .show(ui, |ui| {
                for line in crate::host::command_view::pane_text().lines() {
                    let attention = line.contains("waiting on you") || line.contains("needs you ");
                    let color = if attention {
                        colors.warning
                    } else {
                        colors.text_primary
                    };
                    ui.label(
                        egui::RichText::new(line)
                            .family(egui::FontFamily::Monospace)
                            .size(crate::ui::style::TEXT_BODY)
                            .color(color),
                    );
                }
            });
    }

    fn semantic_state(&self) -> Option<serde_json::Value> {
        Some(crate::host::command_view::pane_state())
    }
}
