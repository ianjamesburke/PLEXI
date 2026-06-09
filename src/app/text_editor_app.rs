//! Built-in file-backed text editor pane.

use crate::app::app_trait::{App, AppCommand, AppRenderContext, KeyDisposition};
use std::path::PathBuf;
use std::time::{Duration, Instant};

const DEBOUNCE: Duration = Duration::from_secs(2);
const FONT_SIZE_DEFAULT: f32 = 14.0;
const FONT_SIZE_MIN: f32 = 9.0;
const FONT_SIZE_MAX: f32 = 32.0;

pub struct TextEditorApp {
    path: PathBuf,
    content: String,
    last_edit: Option<Instant>,
    wants_close: bool,
    load_error: Option<String>,
    font_size: f32,
}

impl TextEditorApp {
    pub fn new(path: PathBuf) -> Self {
        let (content, load_error) = match std::fs::read_to_string(&path) {
            Ok(s) => (s, None),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (String::new(), None),
            Err(e) => (String::new(), Some(e.to_string())),
        };
        log::info!("TextEditorApp: opened {:?} ({} bytes)", path, content.len());
        Self {
            path,
            content,
            last_edit: None,
            wants_close: false,
            load_error,
            font_size: FONT_SIZE_DEFAULT,
        }
    }

    fn flush(&mut self) {
        self.last_edit = None;
        // Empty content → delete the file rather than writing an empty document.
        if self.content.is_empty() {
            if self.path.exists() {
                if let Err(e) = std::fs::remove_file(&self.path) {
                    log::warn!("TextEditorApp: failed to delete empty note {:?}: {e}", self.path);
                } else {
                    log::info!("TextEditorApp: deleted empty note {:?}", self.path);
                }
            }
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

    fn adjust_font_size(&mut self, delta: f32) {
        self.font_size = (self.font_size + delta).clamp(FONT_SIZE_MIN, FONT_SIZE_MAX);
        log::info!("TextEditorApp: font_size -> {}", self.font_size);
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

        // Fill the entire pane rect for consistent background in both tiled and zoomed modes.
        ui.painter().rect_filled(ui.max_rect(), 0.0, colors.bg_darkest);

        ui.visuals_mut().extreme_bg_color = colors.bg_darkest;
        ui.visuals_mut().override_text_color = Some(colors.text_primary);

        let te_id = egui::Id::new("text_editor_content").with(&self.path);
        let font_id = egui::FontId::monospace(self.font_size);

        // Minimum rows = fill the pane without overflow so the scroll area starts inactive.
        let min_rows = ((ui.available_height() / self.font_size) as usize).max(1);

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let output = egui::TextEdit::multiline(&mut self.content)
                    .id(te_id)
                    .font(font_id)
                    .desired_width(f32::INFINITY)
                    .desired_rows(min_rows)
                    .frame(false)
                    .show(ui);

                if output.response.changed() {
                    self.last_edit = Some(Instant::now());
                }

                if let Some(t) = self.last_edit {
                    if t.elapsed() >= DEBOUNCE {
                        self.flush();
                    }
                }

                // Down arrow at end of content → append a newline so the cursor
                // can move past the last line without requiring an explicit Enter.
                if output.response.has_focus() {
                    let at_end = output
                        .cursor_range
                        .map(|r| r.primary.ccursor.index >= self.content.len())
                        .unwrap_or(false);
                    if at_end {
                        let down_pressed = ui.input_mut(|i| {
                            i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)
                        });
                        if down_pressed {
                            self.content.push('\n');
                            self.last_edit = Some(Instant::now());
                            // Reposition cursor to the new end.
                            let mut state = egui::TextEdit::load_state(ui.ctx(), te_id)
                                .unwrap_or_default();
                            let end = egui::text::CCursor::new(self.content.len());
                            state
                                .cursor
                                .set_char_range(Some(egui::text::CCursorRange::one(end)));
                            egui::TextEdit::store_state(ui.ctx(), te_id, state);
                        }
                    }
                }

                // Request focus whenever this pane is active but the TextEdit doesn't
                // have it — handles initial open, zoom/fullscreen toggles, and pane
                // switches without a one-shot guard that breaks after focus changes.
                if ctx.is_focused && !output.response.has_focus() {
                    output.response.request_focus();
                }
            });
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
            }
        }
    }
}

impl Drop for TextEditorApp {
    fn drop(&mut self) {
        // Save unsaved edits; also clean up if content was cleared (flush deletes empty files).
        if self.last_edit.is_some() || self.content.is_empty() {
            self.flush();
        }
    }
}
