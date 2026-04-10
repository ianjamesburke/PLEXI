use crate::app_trait::{App, AppCommand, AppRenderContext};
use std::path::PathBuf;

pub struct QuickNoteApp {
    text: String,
    save_dir: PathBuf,
    saved: bool,
    pending_cmds: Vec<AppCommand>,
    /// Track whether we've requested focus on the first frame.
    focus_set: bool,
}

impl QuickNoteApp {
    pub fn new(save_dir: PathBuf) -> Self {
        Self {
            text: String::new(),
            save_dir,
            saved: false,
            pending_cmds: Vec::new(),
            focus_set: false,
        }
    }

    fn save(&mut self) {
        if self.saved || self.text.trim().is_empty() {
            return;
        }

        let timestamp = std::process::Command::new("date")
            .arg("+%Y-%m-%d-%H%M%S")
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_else(|_| "unknown".to_string());

        let display_time = std::process::Command::new("date")
            .arg("+%Y-%m-%d %H:%M:%S")
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_else(|_| timestamp.clone());

        let filename = format!("note-{}.md", timestamp);
        let header = format!("# Quick Note — {}", display_time);
        let content = format!("{}\n\n{}\n", header, self.text.trim());

        // Save to ~/.plexi/backlog/ by default.
        let backlog_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".plexi")
            .join("backlog");
        if let Err(e) = std::fs::create_dir_all(&backlog_dir) {
            log::error!("QuickNote: failed to create backlog dir: {e}");
        }
        let path = backlog_dir.join(&filename);

        match std::fs::write(&path, &content) {
            Ok(()) => {
                self.saved = true;
                self.pending_cmds
                    .push(AppCommand::Notify(format!("Saved {}", filename)));
            }
            Err(e) => {
                self.pending_cmds
                    .push(AppCommand::Notify(format!("Save failed: {}", e)));
            }
        }
    }
}

impl App for QuickNoteApp {
    fn type_id(&self) -> &'static str {
        "quick_note"
    }

    fn display_name(&self) -> String {
        "Quick Note".to_string()
    }

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &AppRenderContext<'_>) {
        let colors = ctx.colors;

        // Fill background
        let rect = ui.max_rect();
        ui.painter()
            .rect_filled(rect, 0.0, colors.terminal_bg);

        let margin = 32.0;
        let inner = rect.shrink(margin);

        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner));

        // Header
        child.vertical(|ui| {
            ui.add_space(8.0);

            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("Quick Note")
                        .color(colors.text_dim)
                        .size(14.0)
                        .family(egui::FontFamily::Monospace),
                );
                ui.add_space(16.0);
                ui.label(
                    egui::RichText::new("⌘S to save · Esc to discard")
                        .color(colors.text_dim.linear_multiply(0.6))
                        .size(12.0)
                        .family(egui::FontFamily::Monospace),
                );
            });

            // Check for Cmd+S BEFORE the TextEdit consumes key events.
            let should_save = ui.input_mut(|i| {
                i.consume_key(egui::Modifiers::COMMAND, egui::Key::S)
            });
            if should_save {
                self.save();
            }

            ui.add_space(12.0);

            // Text editor
            let available = ui.available_size();
            let response = ui.add_sized(
                available,
                egui::TextEdit::multiline(&mut self.text)
                    .font(egui::FontId::monospace(13.0))
                    .text_color(colors.text_primary)
                    .desired_width(f32::INFINITY)
                    .frame(false),
            );

            // Auto-focus on first frame
            if !self.focus_set {
                response.request_focus();
                self.focus_set = true;
            }
        });
    }

    fn handle_key(&mut self, input: &egui::InputState) -> bool {
        let cmd = input.modifiers.mac_cmd || input.modifiers.command;

        // Cmd+S → save (Cmd+Enter is reserved for Plexi zoom toggle)
        if cmd && input.key_pressed(egui::Key::S) {
            self.save();
            return true;
        }

        // Don't consume Cmd-modified keys (belong to Plexi)
        if cmd {
            return false;
        }

        false
    }

    fn take_pending_commands(&mut self) -> Vec<AppCommand> {
        std::mem::take(&mut self.pending_cmds)
    }

    fn serialize_state(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!(self.text))
    }

    fn restore_state(&mut self, state: &serde_json::Value) {
        if let Some(s) = state.as_str() {
            self.text = s.to_string();
        }
    }
}
