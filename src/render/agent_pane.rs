use crate::pane::AgentPane;
use crate::theme::Colors;

/// Render an agent conversation pane. Placeholder UI — full turn loop wired in #288.
pub fn render_agent_pane(ui: &mut egui::Ui, pane: &mut AgentPane, _colors: &Colors) {
    ui.vertical(|ui| {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for line in &pane.transcript {
                    ui.label(line);
                }
                if pane.transcript.is_empty() {
                    ui.colored_label(
                        egui::Color32::from_gray(120),
                        "Agent pane — type a message below to begin.",
                    );
                }
            });
        ui.separator();
        ui.text_edit_singleline(&mut pane.input_buf);
    });
}
