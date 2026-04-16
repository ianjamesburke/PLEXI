use crate::app_trait::{App, AppCommand, AppRenderContext};
use std::path::PathBuf;

pub struct QuickNoteApp {
    text: String,
    saved: bool,
    pending_cmds: Vec<AppCommand>,
    /// Tracks whether the pane was focused on the previous frame. When the pane
    /// gains focus (transitions from unfocused → focused), we re-request egui
    /// focus on the TextEdit so typing works immediately after Cmd+H/J/K/L.
    was_pane_focused: bool,
    /// Set to true after save — signals the host to close the app.
    pub should_close: bool,
}

impl QuickNoteApp {
    pub fn new(_cwd: PathBuf) -> Self {
        Self {
            text: String::new(),
            saved: false,
            pending_cmds: Vec::new(),
            was_pane_focused: false,
            should_close: false,
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

        let backlog_dir = crate::config::config_dir().join("backlog");
        if let Err(e) = std::fs::create_dir_all(&backlog_dir) {
            log::error!("QuickNote: failed to create backlog dir: {e}");
        }
        let path = backlog_dir.join(&filename);

        match std::fs::write(&path, &content) {
            Ok(()) => {
                self.saved = true;
                self.should_close = true;
                log::info!("QuickNote: saved to {}", path.display());
            }
            Err(e) => {
                log::error!("QuickNote: save failed: {e}");
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

        // Re-request egui focus whenever the pane becomes the active one.
        // `was_pane_focused` lets us detect the rising edge (unfocused → focused)
        // so we don't fight egui every frame — only on the first frame after focus
        // moves here (e.g. via Cmd+H/J/K/L).
        let pane_just_focused = ctx.is_focused && !self.was_pane_focused;
        self.was_pane_focused = ctx.is_focused;

        let rect = ui.max_rect();
        ui.painter().rect_filled(rect, 0.0, colors.terminal_bg);

        // Center the text box vertically and horizontally with generous margins.
        let margin_x = (rect.width() * 0.15).max(32.0);
        let margin_top = (rect.height() * 0.3).max(40.0);
        let inner = egui::Rect::from_min_max(
            egui::pos2(rect.left() + margin_x, rect.top() + margin_top),
            egui::pos2(rect.right() - margin_x, rect.bottom() - 40.0),
        );

        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner));
        child.vertical_centered(|ui| {
            // Only intercept plain Enter (no shift) for save.
            // Shift+Enter passes through to TextEdit which handles newlines naturally.
            let plain_enter = ui.input_mut(|i| {
                if !i.modifiers.shift && !i.modifiers.command {
                    i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                } else {
                    false
                }
            });

            if plain_enter && !self.text.trim().is_empty() {
                self.save();
                return;
            }

            // Subtle hint
            ui.label(
                egui::RichText::new("Enter to save · Shift+Enter for new line · Esc to discard")
                    .color(colors.text_dim.linear_multiply(0.4))
                    .size(10.0)
                    .family(egui::FontFamily::Monospace),
            );
            ui.add_space(8.0);

            // Text box
            let response = ui.add_sized(
                ui.available_size(),
                egui::TextEdit::multiline(&mut self.text)
                    .font(egui::FontId::monospace(14.0))
                    .text_color(colors.text_primary)
                    .desired_width(f32::INFINITY)
                    .frame(false)
                    .hint_text(
                        egui::RichText::new("What's on your mind?")
                            .color(colors.text_dim.linear_multiply(0.3))
                            .size(14.0),
                    ),
            );

            // Request focus on first render OR whenever the pane regains focus
            // (e.g. after Cmd+H/J/K/L navigates back to this pane).
            if pane_just_focused {
                response.request_focus();
            }
        });
    }

    fn wants_close(&self) -> bool {
        self.should_close
    }

    fn handle_key(&mut self, _input: &egui::InputState) -> bool {
        // All key handling is done in ui() before TextEdit.
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
