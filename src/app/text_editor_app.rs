//! Built-in file-backed text editor pane.

use crate::app::app_trait::{App, AppCommand, AppRenderContext, KeyDisposition};
use std::path::PathBuf;
use std::time::{Duration, Instant};

const DEBOUNCE: Duration = Duration::from_secs(2);

pub struct TextEditorApp {
    path: PathBuf,
    content: String,
    last_edit: Option<Instant>,
    wants_close: bool,
    load_error: Option<String>,
    focus_requested: bool,
}

impl TextEditorApp {
    pub fn new(path: PathBuf) -> Self {
        let (content, load_error) = match std::fs::read_to_string(&path) {
            Ok(s) => (s, None),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (String::new(), None),
            Err(e) => (String::new(), Some(e.to_string())),
        };
        log::info!("TextEditorApp: opened {:?} ({} bytes)", path, content.len());
        Self { path, content, last_edit: None, wants_close: false, load_error, focus_requested: false }
    }

    fn flush(&mut self) {
        if let Some(parent) = self.path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                log::warn!("TextEditorApp: could not create parent dir {:?}: {e}", parent);
                return;
            }
        }
        match std::fs::write(&self.path, &self.content) {
            Ok(()) => {
                log::info!("TextEditorApp: saved {:?} ({} bytes)", self.path, self.content.len());
                self.last_edit = None;
            }
            Err(e) => {
                log::warn!("TextEditorApp: save failed for {:?}: {e}", self.path);
            }
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
        false
    }

    fn handle_key(&mut self, _input: &egui::InputState) -> KeyDisposition {
        KeyDisposition::Passthrough
    }

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &AppRenderContext<'_>) {
        let colors = ctx.colors;

        if let Some(err) = &self.load_error {
            ui.centered_and_justified(|ui| {
                ui.label(
                    egui::RichText::new(format!("Failed to open file: {err}"))
                        .size(crate::ui::style::TEXT_BODY)
                        .color(colors.danger),
                );
            });
            return;
        }

        ui.visuals_mut().extreme_bg_color = colors.bg_darkest;
        ui.visuals_mut().override_text_color = Some(colors.text_primary);

        let response = ui.add_sized(
            ui.available_size(),
            egui::TextEdit::multiline(&mut self.content)
                .font(egui::TextStyle::Monospace)
                .desired_width(f32::INFINITY)
                .frame(false),
        );

        if response.changed() {
            self.last_edit = Some(Instant::now());
        }

        if let Some(t) = self.last_edit {
            if t.elapsed() >= DEBOUNCE {
                self.flush();
            }
        }

        if !self.focus_requested {
            response.request_focus();
            self.focus_requested = true;
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
                self.flush();
                let (content, load_error) = match std::fs::read_to_string(&new_path) {
                    Ok(s) => (s, None),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => (String::new(), None),
                    Err(e) => (String::new(), Some(e.to_string())),
                };
                self.path = new_path;
                self.content = content;
                self.load_error = load_error;
                self.last_edit = None;
                self.focus_requested = false;
            }
        }
    }
}

impl Drop for TextEditorApp {
    fn drop(&mut self) {
        if self.last_edit.is_some() {
            self.flush();
        }
    }
}
