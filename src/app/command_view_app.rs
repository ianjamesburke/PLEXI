//! Command view pane. It paints the same projection `plexi command-view --json`
//! returns. Steering stays on the CLI so a click here cannot skip the gate.

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
        _ctx: &AppRenderContext<'_>,
        _pending_click: Option<crate::host::pane::PendingPaneClick>,
    ) {
        ui.label(crate::host::command_view::pane_text());
    }

    fn semantic_state(&self) -> Option<serde_json::Value> {
        Some(crate::host::command_view::pane_state())
    }
}
