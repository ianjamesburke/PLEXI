//! Command view pane. It paints the agents API projection: one row per lead,
//! that lead's latest transcript line, and pending permission requests.
//! Steering stays on the CLI so a click here cannot skip the gate.

use std::path::PathBuf;

use crate::app::app_trait::{App, AppRenderContext};

pub struct CommandViewApp {
    workspace: PathBuf,
}

impl CommandViewApp {
    pub fn new(workspace: PathBuf) -> Self {
        Self { workspace }
    }
}

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
        let lines = crate::agent::leads::pane_lines(&self.workspace);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .id_salt("command-view")
            .show(ui, |ui| {
                for line in lines {
                    let attention = line.starts_with("waiting");
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
        Some(crate::agent::leads::projection(&self.workspace))
    }
}
