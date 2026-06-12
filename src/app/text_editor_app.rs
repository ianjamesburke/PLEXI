//! Built-in file-backed text editor pane.

use crate::app::app_trait::{App, AppCommand, AppRenderContext, KeyDisposition};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

const DEBOUNCE: Duration = Duration::from_secs(2);
const FONT_SIZE_DEFAULT: f32 = 14.0;
const FONT_SIZE_MIN: f32 = 9.0;
const FONT_SIZE_MAX: f32 = 32.0;

pub struct TextEditorApp {
    path: PathBuf,
    /// Editable buffer. For notes this is the body only — the frontmatter
    /// block is held in `note_header` and never shown in the editor.
    content: String,
    /// Raw frontmatter block (both `---` fences, trailing newline) for files
    /// under the notes dir. Recomposed in front of `content` on save.
    note_header: Option<String>,
    /// Display title parsed from `note_header` (empty when unset).
    note_title: String,
    /// True when `path` lives under `<config_dir>/notes/`.
    is_note: bool,
    last_edit: Option<Instant>,
    wants_close: bool,
    load_error: Option<String>,
    font_size: f32,
}

impl TextEditorApp {
    pub fn new(path: PathBuf) -> Self {
        let (raw, load_error) = match std::fs::read_to_string(&path) {
            Ok(s) => (s, None),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (String::new(), None),
            Err(e) => (String::new(), Some(e.to_string())),
        };
        log::info!("TextEditorApp: opened {:?} ({} bytes)", path, raw.len());
        let notes_dir = crate::config::config_dir().join("notes");
        let is_note = path.starts_with(&notes_dir);
        let (note_header, content, note_title) = split_note(is_note, raw);
        Self {
            path,
            content,
            note_header,
            note_title,
            is_note,
            last_edit: None,
            wants_close: false,
            load_error,
            font_size: FONT_SIZE_DEFAULT,
        }
    }

    /// Test constructor: treat `path` as a note regardless of the machine's
    /// config dir, so note rendering is testable against temp files.
    #[cfg(test)]
    pub(crate) fn new_for_test_note(path: PathBuf) -> Self {
        let mut app = Self::new(path);
        let raw = app.composed();
        let (note_header, content, note_title) = split_note(true, raw);
        app.is_note = true;
        app.note_header = note_header;
        app.content = content;
        app.note_title = note_title;
        app
    }

    /// Full on-disk document: frontmatter (when held out) + editable body.
    fn composed(&self) -> String {
        match &self.note_header {
            Some(header) => format!("{header}{}", self.content),
            None => self.content.clone(),
        }
    }

    /// Rewrite the frontmatter `title:` for this note and mark the buffer
    /// dirty so the next flush persists it. No-op for non-note files —
    /// renaming a pane must never inject YAML into arbitrary documents.
    fn apply_note_title(&mut self, title: &str) {
        if !self.is_note {
            return;
        }
        let updated = crate::notes::set_title_in_content(&self.composed(), title);
        let (note_header, content, note_title) = split_note(true, updated);
        self.note_header = note_header;
        self.content = content;
        self.note_title = note_title;
        self.last_edit = Some(Instant::now());
        log::info!(
            "TextEditorApp: note title set to {:?} for {:?}",
            self.note_title,
            self.path
        );
    }

    fn is_effectively_empty(&self) -> bool {
        let notes_dir = crate::config::config_dir().join("notes");
        content_is_effectively_empty(&self.path, &notes_dir, &self.composed())
    }

    fn flush(&mut self) {
        // Empty content → delete the file rather than writing an empty document.
        if self.is_effectively_empty() {
            if self.path.exists() {
                if let Err(e) = std::fs::remove_file(&self.path) {
                    log::warn!(
                        "TextEditorApp: failed to delete empty note {:?}: {e}",
                        self.path
                    );
                    self.last_edit = Some(Instant::now());
                } else {
                    log::info!("TextEditorApp: deleted empty note {:?}", self.path);
                    self.last_edit = None;
                }
            } else {
                self.last_edit = None;
            }
            return;
        }
        let document = self.composed();
        match write_note_atomically(&self.path, document.as_bytes()) {
            Ok(()) => {
                self.last_edit = None;
                log::info!(
                    "TextEditorApp: saved {:?} ({} bytes)",
                    self.path,
                    document.len()
                );
            }
            Err(e) => {
                self.last_edit = Some(Instant::now());
                log::warn!("TextEditorApp: save failed for {:?}: {e}", self.path);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_temp_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("{name}-{}-{}", std::process::id(), unique_suffix()))
    }

    fn unique_suffix() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    }

    #[test]
    fn frontmatter_only_note_counts_as_empty_inside_notes_dir() {
        let notes_dir = PathBuf::from("/fake-home/.plexi/notes");
        let inbox_note = notes_dir.join("inbox").join("note-1.md");
        let outside = PathBuf::from("/projects/readme.md");
        let frontmatter_only = "---\ntitle: \"\"\nsource: \"scratchpad\"\n---\n\n";
        let with_body = "---\nsource: \"scratchpad\"\n---\nactual content\n";

        // Frontmatter-only is empty only for files under the notes dir.
        assert!(content_is_effectively_empty(
            &inbox_note,
            &notes_dir,
            frontmatter_only
        ));
        assert!(!content_is_effectively_empty(
            &outside,
            &notes_dir,
            frontmatter_only
        ));

        // A real body is never empty; zero bytes always is.
        assert!(!content_is_effectively_empty(
            &inbox_note,
            &notes_dir,
            with_body
        ));
        assert!(content_is_effectively_empty(&outside, &notes_dir, ""));
    }

    #[test]
    fn split_note_hides_frontmatter_and_extracts_title() {
        let raw = "---\ntitle: \"Groceries\"\nsource: \"scratchpad\"\n---\nmilk\neggs\n";
        let (header, body, title) = split_note(true, raw.to_string());
        assert_eq!(
            header.as_deref(),
            Some("---\ntitle: \"Groceries\"\nsource: \"scratchpad\"\n---\n")
        );
        assert_eq!(body, "milk\neggs\n");
        assert_eq!(title, "Groceries");
        // Recomposition is lossless.
        assert_eq!(format!("{}{body}", header.unwrap()), raw);
    }

    #[test]
    fn split_note_passes_through_non_notes_and_headerless_content() {
        let raw = "---\ntitle: \"x\"\n---\nbody\n";
        let (header, body, title) = split_note(false, raw.to_string());
        assert!(header.is_none());
        assert_eq!(body, raw);
        assert!(title.is_empty());

        let plain = "just text, no frontmatter\n";
        let (header, body, _) = split_note(true, plain.to_string());
        assert!(header.is_none());
        assert_eq!(body, plain);
    }

    #[test]
    fn title_rewrite_roundtrips_through_split() {
        let raw = "---\ntitle: \"\"\nsource: \"scratchpad\"\n---\nbody line\n";
        let updated = crate::notes::set_title_in_content(raw, "New Title");
        let (header, body, title) = split_note(true, updated);
        assert_eq!(title, "New Title");
        assert_eq!(body, "body line\n");
        assert!(header.unwrap().contains("source: \"scratchpad\""));
    }

    #[test]
    fn note_path_identity_matches_existing_file_aliases() {
        let dir = unique_temp_dir("plexi-note-identity");
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("note.md");
        std::fs::write(&path, "hello").expect("write note");
        let alias = dir.join(".").join("note.md");

        assert_eq!(note_path_identity(&path), note_path_identity(&alias));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn note_path_identity_matches_missing_file_when_parent_exists() {
        let dir = unique_temp_dir("plexi-note-missing-identity");
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("missing.md");
        let alias = dir.join(".").join("missing.md");

        assert_eq!(note_path_identity(&path), note_path_identity(&alias));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_note_atomically_replaces_existing_file_contents() {
        let dir = unique_temp_dir("plexi-note-atomic-write");
        let path = dir.join("note.md");
        std::fs::create_dir_all(&dir).expect("create temp dir");
        std::fs::write(&path, "old contents").expect("seed note");

        write_note_atomically(&path, b"new contents").expect("atomic write");

        assert_eq!(
            std::fs::read_to_string(&path).expect("read note"),
            "new contents"
        );
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .expect("read temp dir")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp files should be cleaned up");

        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Split a note document into its raw frontmatter block (kept out of the
/// editable buffer), the body, and the display title. Non-note files and
/// notes without a frontmatter block pass through unchanged.
fn split_note(is_note: bool, raw: String) -> (Option<String>, String, String) {
    if !is_note {
        return (None, raw, String::new());
    }
    if let Some(rest) = raw.strip_prefix("---\n") {
        if let Some(end) = rest.find("\n---\n") {
            let header = raw[..4 + end + 5].to_string(); // "---\n" + header + "\n---\n"
            let body = rest[end + 5..].to_string();
            let title = crate::notes::parse_note(&raw).0.title.unwrap_or_default();
            return (Some(header), body, title);
        }
    }
    (None, raw, String::new())
}

/// A note is empty when it has no content at all, or — for files under
/// `notes_dir` — when only capture frontmatter remains (scratch/quick notes the
/// user never typed into). Empty notes are deleted instead of saved.
fn content_is_effectively_empty(path: &Path, notes_dir: &Path, content: &str) -> bool {
    if content.is_empty() {
        return true;
    }
    path.starts_with(notes_dir) && crate::notes::parse_note(content).1.trim().is_empty()
}

pub(crate) fn note_path_identity(path: &Path) -> PathBuf {
    if let Ok(canonical) = path.canonicalize() {
        return canonical;
    }

    let path = normalize_lexically(path);
    let Some(parent) = path.parent() else {
        return path;
    };
    match parent.canonicalize() {
        Ok(parent) => path
            .file_name()
            .map(|name| parent.join(name))
            .unwrap_or(path),
        Err(_) => path,
    }
}

fn normalize_lexically(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    normalized.push(component.as_os_str());
                }
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

fn write_note_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("note");
    let temp_path = parent.join(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));

    let write_result = (|| {
        let mut temp_file = std::fs::File::create(&temp_path)?;
        temp_file.write_all(bytes)?;
        temp_file.sync_all()?;
        drop(temp_file);
        std::fs::rename(&temp_path, path)
    })();

    if write_result.is_err() {
        let _ = std::fs::remove_file(&temp_path);
    }

    write_result
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
        ui.painter()
            .rect_filled(ui.max_rect(), 0.0, colors.bg_darkest);

        ui.visuals_mut().extreme_bg_color = colors.bg_darkest;
        ui.visuals_mut().override_text_color = Some(colors.text_primary);

        // Notes show their frontmatter title as a header row; the YAML block
        // itself is held out of the buffer and never rendered.
        if self.is_note {
            let (text, color) = if self.note_title.is_empty() {
                let placeholder = self
                    .path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "untitled".to_string());
                (placeholder, colors.text_dim)
            } else {
                (self.note_title.clone(), colors.text_primary)
            };
            ui.add_space(crate::ui::style::SPACE_SM);
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(text)
                        .size(crate::ui::style::TEXT_BODY)
                        .strong()
                        .color(color),
                );
            });
            ui.add_space(crate::ui::style::SPACE_SM);
        }

        let te_id = egui::Id::new("text_editor_content").with(&self.path);
        let font_id = egui::FontId::monospace(self.font_size);

        // Use the actual rendered row height (not font em-size) so desired_rows fills
        // the viewport exactly. font_size alone ignores line leading, causing the TextEdit
        // to be taller than the viewport and triggering an unwanted scrollbar.
        let row_height = ui.fonts(|f| f.row_height(&font_id));
        let min_rows = ((ui.available_height() / row_height).floor() as usize).max(1);

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let output = egui::TextEdit::multiline(&mut self.content)
                    .id(te_id)
                    .font(font_id)
                    .desired_width(f32::INFINITY)
                    .desired_rows(min_rows)
                    .margin(egui::vec2(4.0, 0.0))
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
                            let mut state =
                                egui::TextEdit::load_state(ui.ctx(), te_id).unwrap_or_default();
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
            if note_path_identity(&new_path) != note_path_identity(&self.path) {
                log::info!(
                    "TextEditorApp: switching from {:?} to {:?}",
                    self.path,
                    new_path
                );
                self.flush();
                let (raw, load_error) = match std::fs::read_to_string(&new_path) {
                    Ok(s) => (s, None),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => (String::new(), None),
                    Err(e) => (String::new(), Some(e.to_string())),
                };
                let notes_dir = crate::config::config_dir().join("notes");
                self.is_note = new_path.starts_with(&notes_dir);
                let (note_header, content, note_title) = split_note(self.is_note, raw);
                self.path = new_path;
                self.content = content;
                self.note_header = note_header;
                self.note_title = note_title;
                self.load_error = load_error;
                self.last_edit = None;
            }
        }
    }

    fn rename_seed(&self) -> Option<String> {
        self.is_note.then(|| self.note_title.clone())
    }

    fn on_pane_renamed(&mut self, name: &str) {
        self.apply_note_title(name);
    }
}

impl Drop for TextEditorApp {
    fn drop(&mut self) {
        // Save unsaved edits; also clean up empty notes (flush deletes them).
        if self.last_edit.is_some() || self.is_effectively_empty() {
            self.flush();
        }
    }
}
