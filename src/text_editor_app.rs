//! Built-in file-backed text editor pane.

use crate::app_trait::{App, AppCommand, AppRenderContext, KeyDisposition};
use crate::style;
use egui::RichText;
use std::path::PathBuf;

pub struct TextEditorApp {
    path: PathBuf,
    content: String,
    dirty: bool,
    wants_close: bool,
    load_error: Option<String>,
}

impl TextEditorApp {
    pub fn new(path: PathBuf) -> Self {
        let (content, load_error) = match std::fs::read_to_string(&path) {
            Ok(s) => (s, None),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (String::new(), None),
            Err(e) => (String::new(), Some(e.to_string())),
        };
        log::info!("TextEditorApp: opened {:?} ({} bytes)", path, content.len());
        Self { path, content, dirty: false, wants_close: false, load_error }
    }

    fn save(&mut self) {
        if !self.dirty {
            return;
        }
        if let Some(parent) = self.path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                log::warn!("TextEditorApp: could not create parent dir {:?}: {e}", parent);
                return;
            }
        }
        match std::fs::write(&self.path, &self.content) {
            Ok(()) => {
                log::info!("TextEditorApp: saved {:?} ({} bytes)", self.path, self.content.len());
                self.dirty = false;
            }
            Err(e) => log::warn!("TextEditorApp: save failed for {:?}: {e}", self.path),
        }
    }
}

impl App for TextEditorApp {
    fn type_id(&self) -> &'static str {
        "text-editor"
    }

    fn display_name(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "text-editor".to_string())
    }

    fn keyboard_capture(&self) -> bool {
        true
    }

    fn handle_key(&mut self, input: &egui::InputState) -> KeyDisposition {
        if input.modifiers.command && input.key_pressed(egui::Key::S) {
            self.save();
            return KeyDisposition::Consumed;
        }
        KeyDisposition::Passthrough
    }

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &AppRenderContext<'_>) {
        let colors = ctx.colors;

        if let Some(err) = &self.load_error {
            ui.centered_and_justified(|ui| {
                ui.label(
                    RichText::new(format!("Failed to open file: {err}"))
                        .size(style::TEXT_BODY)
                        .color(colors.danger),
                );
            });
            return;
        }

        ui.visuals_mut().extreme_bg_color = colors.bg_darkest;
        ui.visuals_mut().override_text_color = Some(colors.text_primary);

        // Reserve space at the bottom for the "Cmd+S to save" hint when dirty.
        let hint_height = style::TEXT_HINT + style::SPACE_SM * 2.0;
        let mut available = ui.available_rect_before_wrap();
        if self.dirty {
            available.max.y -= hint_height;
        }

        let response = ui.add_sized(
            available.size(),
            egui::TextEdit::multiline(&mut self.content)
                .font(egui::TextStyle::Monospace)
                .desired_width(f32::INFINITY)
                .frame(false),
        );

        if response.changed() {
            self.dirty = true;
        }

        if self.dirty {
            ui.add_space(style::SPACE_SM);
            ui.label(
                RichText::new("Cmd+S to save")
                    .size(style::TEXT_HINT)
                    .color(colors.text_dim),
            );
        }
    }

    fn wants_close(&self) -> bool {
        self.wants_close
    }

    fn take_pending_commands(&mut self) -> Vec<AppCommand> {
        vec![]
    }

    fn serialize_state(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({ "path": self.path.to_string_lossy() }))
    }

    fn restore_state(&mut self, state: &serde_json::Value) {
        if let Some(p) = state.get("path").and_then(|v| v.as_str()) {
            let new_path = PathBuf::from(p);
            if new_path != self.path {
                log::info!("TextEditorApp: switching from {:?} to {:?}", self.path, new_path);
                self.save();
                let (content, load_error) = match std::fs::read_to_string(&new_path) {
                    Ok(s) => (s, None),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => (String::new(), None),
                    Err(e) => (String::new(), Some(e.to_string())),
                };
                self.path = new_path;
                self.content = content;
                self.load_error = load_error;
                self.dirty = false;
            }
        }
    }
}

impl Drop for TextEditorApp {
    fn drop(&mut self) {
        self.save();
    }
}
