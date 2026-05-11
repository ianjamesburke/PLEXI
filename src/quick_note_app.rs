use crate::app_trait::{App, AppCommand, AppRenderContext};
use crate::style;
use crate::widgets;
use std::path::PathBuf;

#[derive(Default)]
enum Phase {
    #[default]
    Composing,
    PickingDestination,
}

struct Destination {
    key: &'static str,
    label: &'static str,
}

const DESTINATIONS: &[Destination] = &[
    Destination { key: "0", label: "Backlog" },
    Destination { key: "1", label: "Project backlog" },
    Destination { key: "2", label: "Claude Code — current dir" },
    Destination { key: "3", label: "Claude Code — workspace" },
    Destination { key: "4", label: "GitHub issue" },
];

pub struct QuickNoteApp {
    text: String,
    saved: bool,
    pending_cmds: Vec<AppCommand>,
    focus_set: bool,
    phase: Phase,
    /// Set to true after save — signals the host to close the app.
    pub should_close: bool,
}

impl QuickNoteApp {
    pub fn new(_cwd: PathBuf) -> Self {
        Self {
            text: String::new(),
            saved: false,
            pending_cmds: Vec::new(),
            focus_set: false,
            phase: Phase::Composing,
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

    fn draw_composing(&mut self, ui: &mut egui::Ui, ctx: &AppRenderContext<'_>) {
        let colors = ctx.colors;

        // Only intercept plain Enter (no shift) to advance to destination picker.
        // Shift+Enter passes through to TextEdit which handles newlines naturally.
        let plain_enter = ui.input_mut(|i| {
            if !i.modifiers.shift && !i.modifiers.command {
                i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
            } else {
                false
            }
        });

        if plain_enter && !self.text.trim().is_empty() {
            log::info!("QuickNote: showing destination picker");
            self.phase = Phase::PickingDestination;
            return;
        }

        // Subtle hint
        ui.label(
            egui::RichText::new("Enter to send · Shift+Enter for new line · Esc to discard")
                .color(colors.text_dim.linear_multiply(0.4))
                .size(style::TEXT_HINT)
                .family(egui::FontFamily::Monospace),
        );
        ui.add_space(style::SPACE_SM);

        // Text box
        let response = ui.add_sized(
            ui.available_size(),
            egui::TextEdit::multiline(&mut self.text)
                .font(egui::FontId::monospace(style::TEXT_BODY))
                .text_color(colors.text_primary)
                .desired_width(f32::INFINITY)
                .frame(false)
                .hint_text(
                    egui::RichText::new("What's on your mind?")
                        .color(colors.text_dim.linear_multiply(0.3))
                        .size(style::TEXT_BODY),
                ),
        );

        if !self.focus_set {
            response.request_focus();
            self.focus_set = true;
        }
    }

    fn draw_destination_picker(&mut self, ui: &mut egui::Ui, ctx: &AppRenderContext<'_>) {
        let colors = ctx.colors;

        let esc = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        if esc {
            log::info!("QuickNote: destination picker dismissed, back to composing");
            self.phase = Phase::Composing;
            self.focus_set = false;
            return;
        }

        let enter = ui.input_mut(|i| {
            if !i.modifiers.shift && !i.modifiers.command {
                i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
            } else {
                false
            }
        });

        // Digit keys 0–4 select a destination; bare Enter routes to destination 0.
        let selected_key = ui.input_mut(|i| {
            let digits = [
                (egui::Key::Num0, "0"),
                (egui::Key::Num1, "1"),
                (egui::Key::Num2, "2"),
                (egui::Key::Num3, "3"),
                (egui::Key::Num4, "4"),
            ];
            for (key, label) in digits {
                if i.consume_key(egui::Modifiers::NONE, key) {
                    return Some(label);
                }
            }
            None
        });

        if enter || selected_key.is_some() {
            let dest = selected_key.unwrap_or("0");
            log::info!("QuickNote: destination selected: {dest}");
            self.save();
            return;
        }

        // --- Note preview (dimmed) ---
        let preview = {
            let t = self.text.trim();
            if let Some((idx, _)) = t.char_indices().nth(80) {
                format!("{}…", &t[..idx])
            } else {
                t.to_string()
            }
        };
        ui.label(
            egui::RichText::new(&preview)
                .color(colors.text_dim.linear_multiply(0.5))
                .size(style::TEXT_BODY)
                .family(egui::FontFamily::Monospace),
        );

        ui.add_space(style::SPACE_XL);

        // --- Heading ---
        ui.label(
            egui::RichText::new("Send to…")
                .color(colors.text_primary)
                .size(style::TEXT_BODY),
        );

        ui.add_space(style::SPACE_MD);

        // --- Destination rows ---
        for dest in DESTINATIONS {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = style::SPACE_SM;
                widgets::key_chip(ui, dest.key, colors);
                ui.label(
                    egui::RichText::new(dest.label)
                        .color(colors.text_dim)
                        .size(style::TEXT_BODY),
                );
            });
            ui.add_space(style::SPACE_SM);
        }

        ui.add_space(style::SPACE_XL);

        // --- Footer hint ---
        ui.label(
            egui::RichText::new("Esc to go back")
                .color(colors.text_dim.linear_multiply(0.4))
                .size(style::TEXT_HINT)
                .family(egui::FontFamily::Monospace),
        );
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

        let rect = ui.max_rect();
        ui.painter().rect_filled(rect, 0.0, colors.terminal_bg);

        let margin_x = (rect.width() * 0.15).max(32.0);
        let margin_top = (rect.height() * 0.3).max(40.0);
        let inner = egui::Rect::from_min_max(
            egui::pos2(rect.left() + margin_x, rect.top() + margin_top),
            egui::pos2(rect.right() - margin_x, rect.bottom() - 40.0),
        );

        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(inner));
        child.vertical_centered(|ui| match &self.phase {
            Phase::Composing => self.draw_composing(ui, ctx),
            Phase::PickingDestination => self.draw_destination_picker(ui, ctx),
        });
    }

    fn wants_close(&self) -> bool {
        self.should_close
    }

    fn handle_key(&mut self, _input: &egui::InputState) -> bool {
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
