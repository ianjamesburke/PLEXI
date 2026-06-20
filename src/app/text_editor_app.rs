//! Built-in file-backed text editor pane.

use crate::app::app_trait::{App, AppCommand, AppRenderContext, KeyDisposition};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

const DEBOUNCE: Duration = Duration::from_secs(2);
/// Mirrors the terminal pane name bar (`render_name_bar_and_dots`): same
/// height, fill, and centered dim 11px text, so note panes match terminal chrome.
const NOTE_HEADER_BAR_HEIGHT: f32 = 20.0;
const NOTE_HEADER_FONT_SIZE: f32 = 11.0;
const FONT_SIZE_DEFAULT: f32 = 14.0;
const FONT_SIZE_MIN: f32 = 9.0;
const FONT_SIZE_MAX: f32 = 32.0;
const FIND_BAR_HEIGHT: f32 = 28.0;
const FIND_BAR_FONT_SIZE: f32 = 13.0;

struct FindBar {
    query: String,
    /// Byte-offset positions of each match start in `content`.
    matches: Vec<usize>,
    /// Index into `matches` for the current (highlighted) match.
    current: usize,
    /// Requests focus on the find input on the next frame.
    focus_requested: bool,
}

impl FindBar {
    fn new() -> Self {
        Self {
            query: String::new(),
            matches: Vec::new(),
            current: 0,
            focus_requested: true,
        }
    }

    fn recompute(&mut self, content: &str) {
        self.matches.clear();
        if self.query.is_empty() {
            return;
        }
        let lower_content = content.to_lowercase();
        let lower_query = self.query.to_lowercase();
        let mut pos = 0;
        while let Some(idx) = lower_content[pos..].find(&lower_query) {
            let abs = pos + idx;
            self.matches.push(abs);
            pos = abs + lower_query.len().max(1);
        }
        if self.current >= self.matches.len() && !self.matches.is_empty() {
            self.current = self.matches.len() - 1;
        }
    }

    fn advance(&mut self, forward: bool) {
        if self.matches.is_empty() {
            return;
        }
        if forward {
            self.current = (self.current + 1) % self.matches.len();
        } else {
            self.current = self.current.checked_sub(1).unwrap_or(self.matches.len() - 1);
        }
    }
}

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
    /// Whether the text cursor sat at the end of the buffer on the previous
    /// rendered frame. Gates the down-arrow-appends-newline behavior: the
    /// press that *moves* the cursor to the end must not also append.
    cursor_was_at_end: bool,
    /// Active find bar, or `None` when dismissed.
    find_bar: Option<FindBar>,
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
            cursor_was_at_end: false,
            find_bar: None,
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

    /// Durable save: full atomic write with fsync. Used when the document
    /// lifecycle ends (pane close, file switch).
    fn flush(&mut self) {
        self.flush_with(Durability::Fsync);
    }

    /// Debounced autosave: atomic temp+rename without fsync, so the render
    /// thread never blocks on a 1-10ms APFS fsync between keystrokes. Crash
    /// safety is preserved (rename is atomic — readers see old or new, never
    /// partial); only power-loss durability is deferred to the next durable
    /// flush.
    fn autosave(&mut self) {
        self.flush_with(Durability::Fast);
    }

    fn flush_with(&mut self, durability: Durability) {
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
        match write_note_atomically(&self.path, document.as_bytes(), durability) {
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
    fn split_note_absorbs_blank_lines_after_fence_into_header() {
        // Captures write "---\n...\n---\n\n" — the blank line must live in the
        // hidden header, not the editable body, or it renders as dead space.
        let raw = "---\ntitle: \"\"\nsource: \"scratchpad\"\n---\n\nbody text\n";
        let (header, body, _) = split_note(true, raw.to_string());
        assert_eq!(
            header.as_deref(),
            Some("---\ntitle: \"\"\nsource: \"scratchpad\"\n---\n\n")
        );
        assert_eq!(body, "body text\n");
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

        write_note_atomically(&path, b"new contents", Durability::Fsync).expect("atomic write");

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

    #[test]
    fn find_bar_recompute_finds_case_insensitive_matches() {
        let mut bar = FindBar::new();
        bar.query = "hello".to_string();
        let content = "Hello world, hello there, HELLO!";
        bar.recompute(content);
        assert_eq!(bar.matches.len(), 3);
        assert_eq!(bar.matches[0], 0);
        assert_eq!(bar.matches[1], 13);
        assert_eq!(bar.matches[2], 26);
    }

    #[test]
    fn find_bar_advance_wraps_around() {
        let mut bar = FindBar::new();
        bar.query = "x".to_string();
        let content = "x foo x bar x";
        bar.recompute(content);
        assert_eq!(bar.matches.len(), 3);
        assert_eq!(bar.current, 0);
        bar.advance(true);
        assert_eq!(bar.current, 1);
        bar.advance(true);
        assert_eq!(bar.current, 2);
        bar.advance(true);
        assert_eq!(bar.current, 0); // wraps
        bar.advance(false);
        assert_eq!(bar.current, 2); // wraps backward
    }

    #[test]
    fn find_bar_empty_query_produces_no_matches() {
        let mut bar = FindBar::new();
        bar.recompute("anything");
        assert!(bar.matches.is_empty());
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
            // "---\n" + header + "\n---\n", plus any blank lines between the
            // fence and the body — captures write "---\n\n" and that leading
            // blank line must not render as dead space under the title bar.
            let mut header_end = 4 + end + 5;
            while raw[header_end..].starts_with('\n') {
                header_end += 1;
            }
            let header = raw[..header_end].to_string();
            let body = raw[header_end..].to_string();
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

/// Whether an atomic note write fsyncs before the rename.
#[derive(Clone, Copy, PartialEq)]
enum Durability {
    /// fsync the temp file before renaming — survives power loss.
    Fsync,
    /// Skip fsync — still atomic (rename), but durability is deferred.
    /// For debounced autosaves on the render thread, where an APFS fsync
    /// (1-10ms) can land exactly on the next keystroke.
    Fast,
}

fn write_note_atomically(path: &Path, bytes: &[u8], durability: Durability) -> std::io::Result<()> {
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
        if durability == Durability::Fsync {
            temp_file.sync_all()?;
        }
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

    fn handle_key(&mut self, input: &egui::InputState) -> KeyDisposition {
        // Cmd+F: open find bar (or re-focus if already open).
        if input.key_pressed(egui::Key::F)
            && input.modifiers.matches_logically(egui::Modifiers::COMMAND)
        {
            log::info!("TextEditorApp: Cmd+F — opening find bar");
            match &mut self.find_bar {
                Some(bar) => bar.focus_requested = true,
                None => {
                    let mut bar = FindBar::new();
                    bar.recompute(&self.content);
                    self.find_bar = Some(bar);
                }
            }
            return KeyDisposition::Consumed;
        }

        if let Some(bar) = &mut self.find_bar {
            // Escape: close the find bar.
            if input.key_pressed(egui::Key::Escape) {
                log::info!("TextEditorApp: Escape — closing find bar");
                self.find_bar = None;
                return KeyDisposition::Consumed;
            }
            // Enter: next match. Shift+Enter: previous match.
            if input.key_pressed(egui::Key::Enter) {
                let forward = !input.modifiers.shift;
                bar.advance(forward);
                return KeyDisposition::Consumed;
            }
        }

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

        // Fill only the remaining rect, not `max_rect()`: when this editor
        // overtakes another pane, `app_pane::render` has already allocated the
        // overtake bar above us, and filling max_rect would paint over it —
        // leaving an invisible bar-sized gap. Matches the terminal pane
        // background so note panes and terminals read as the same surface.
        ui.painter()
            .rect_filled(ui.available_rect_before_wrap(), 0.0, colors.terminal_bg);

        ui.visuals_mut().extreme_bg_color = colors.terminal_bg;
        ui.visuals_mut().override_text_color = Some(colors.text_primary);

        // Notes show their frontmatter title in a header bar styled exactly
        // like the terminal pane name bar (same height, fill, and centered dim
        // text — whether the title is custom or the file-name fallback). The
        // YAML block itself is held out of the buffer and never rendered.
        if self.is_note {
            let title = if self.note_title.is_empty() {
                self.path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "untitled".to_string())
            } else {
                self.note_title.clone()
            };
            let bar_rect = egui::Rect::from_min_size(
                ui.cursor().min,
                egui::vec2(ui.available_width(), NOTE_HEADER_BAR_HEIGHT),
            );
            ui.advance_cursor_after_rect(bar_rect);
            ui.painter()
                .rect_filled(bar_rect, 0.0, colors.pane_header_bg());
            // A real label (not painter text) so the title stays in the
            // accessibility tree for UI-harness queries.
            let mut bar_ui = ui.new_child(egui::UiBuilder::new().max_rect(bar_rect));
            bar_ui.centered_and_justified(|ui| {
                ui.label(
                    egui::RichText::new(title)
                        .size(NOTE_HEADER_FONT_SIZE)
                        .color(colors.text_dim),
                );
            });
            ui.add_space(crate::ui::style::SPACE_XS);
        }

        let te_id = egui::Id::new("text_editor_content").with(&self.path);
        let font_id = egui::FontId::monospace(self.font_size);

        // When the find bar is open, reserve its height at the bottom before
        // laying out the scroll area so the editor doesn't overlap the bar.
        let find_bar_height = if self.find_bar.is_some() {
            FIND_BAR_HEIGHT
        } else {
            0.0
        };
        let editor_height = (ui.available_height() - find_bar_height).max(1.0);

        // Use the actual rendered row height (not font em-size) so desired_rows fills
        // the viewport exactly. font_size alone ignores line leading, causing the TextEdit
        // to be taller than the viewport and triggering an unwanted scrollbar.
        let row_height = ui.fonts(|f| f.row_height(&font_id));
        let min_rows = ((editor_height / row_height).floor() as usize).max(1);
        const OVERSCROLL_ROWS: usize = 100;

        // Snapshot match positions for the layouter closure (can't borrow self there).
        let match_positions: Vec<usize> = self
            .find_bar
            .as_ref()
            .map(|b| b.matches.clone())
            .unwrap_or_default();
        let current_match: Option<usize> = self
            .find_bar
            .as_ref()
            .and_then(|b| b.matches.get(b.current).copied());
        let query_len = self
            .find_bar
            .as_ref()
            .map(|b| b.query.len())
            .unwrap_or(0);

        let match_bg = colors.warning.gamma_multiply(0.45);
        let current_match_bg = colors.accent.gamma_multiply(0.55);
        let text_color = colors.text_primary;
        let font_id_clone = font_id.clone();

        let mut layouter = move |ui: &egui::Ui, text: &str, wrap_width: f32| {
            let mut job = egui::text::LayoutJob::default();
            if match_positions.is_empty() || query_len == 0 {
                job.append(
                    text,
                    0.0,
                    egui::TextFormat {
                        font_id: font_id_clone.clone(),
                        color: text_color,
                        ..Default::default()
                    },
                );
            } else {
                let mut pos = 0usize;
                for &start in &match_positions {
                    if start > text.len() {
                        break;
                    }
                    let end = (start + query_len).min(text.len());
                    if start > pos {
                        job.append(
                            &text[pos..start],
                            0.0,
                            egui::TextFormat {
                                font_id: font_id_clone.clone(),
                                color: text_color,
                                ..Default::default()
                            },
                        );
                    }
                    let bg = if Some(start) == current_match {
                        current_match_bg
                    } else {
                        match_bg
                    };
                    job.append(
                        &text[start..end],
                        0.0,
                        egui::TextFormat {
                            font_id: font_id_clone.clone(),
                            color: text_color,
                            background: bg,
                            ..Default::default()
                        },
                    );
                    pos = end;
                }
                if pos < text.len() {
                    job.append(
                        &text[pos..],
                        0.0,
                        egui::TextFormat {
                            font_id: font_id_clone.clone(),
                            color: text_color,
                            ..Default::default()
                        },
                    );
                }
            }
            job.wrap.max_width = wrap_width;
            ui.fonts(|f| f.layout_job(job))
        };

        egui::ScrollArea::vertical()
            .id_salt(egui::Id::new("text_editor_scroll").with(&self.path))
            .auto_shrink([false, false])
            .max_height(editor_height)
            .show(ui, |ui| {
                // egui's caret is hidden (transparent, non-blinking) and
                // draw_text_caret paints a glyph-height replacement on top.
                let output = ui
                    .scope(|ui| {
                        ui.visuals_mut().text_cursor.blink = false;
                        ui.visuals_mut().text_cursor.stroke.color = egui::Color32::TRANSPARENT;
                        egui::TextEdit::multiline(&mut self.content)
                            .id(te_id)
                            .font(font_id)
                            .desired_width(f32::INFINITY)
                            .desired_rows(min_rows + OVERSCROLL_ROWS)
                            .margin(egui::vec2(4.0, 0.0))
                            .frame(false)
                            .layouter(&mut layouter)
                            .show(ui)
                    })
                    .inner;
                crate::ui::text_field::draw_text_caret(
                    ui,
                    &output,
                    self.font_size,
                    row_height,
                    egui::Stroke::new(1.0, colors.accent),
                );

                if output.response.changed() {
                    self.last_edit = Some(Instant::now());
                    // Recompute matches when content changes.
                    if let Some(bar) = &mut self.find_bar {
                        bar.recompute(&self.content);
                    }
                }

                if let Some(t) = self.last_edit {
                    if t.elapsed() >= DEBOUNCE {
                        self.autosave();
                    }
                }

                // Down arrow at end of content → append a newline so the cursor
                // can move past the last line without requiring an explicit Enter.
                //
                // Two constraints:
                // * Gate on the cursor having been at the end on the PREVIOUS
                //   frame. TextEdit has already processed this frame's presses
                //   (egui reads but does not consume them), so the press that
                //   moved the cursor TO the end must not also append.
                // * Append one newline per queued press, not per frame —
                //   key-repeat can deliver several presses in one frame and
                //   consume_key would silently swallow the extras, making Down
                //   feel slower than Enter.
                if output.response.has_focus() {
                    let at_end = output
                        .cursor_range
                        .map(|r| r.primary.ccursor.index >= self.content.len())
                        .unwrap_or(false);
                    if at_end && self.cursor_was_at_end {
                        let presses = ui.input_mut(|i| {
                            i.count_and_consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)
                        });
                        if presses > 0 {
                            for _ in 0..presses {
                                self.content.push('\n');
                            }
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
                    self.cursor_was_at_end = at_end;
                }

                // Request focus whenever this pane is active but the TextEdit doesn't
                // have it — handles initial open, zoom/fullscreen toggles, and pane
                // switches without a one-shot guard that breaks after focus changes.
                // Skip when the find bar is open: the find input owns focus then.
                if ctx.is_focused && !output.response.has_focus() && self.find_bar.is_none() {
                    output.response.request_focus();
                }
            });

        // Render the find bar below the scroll area.
        if let Some(bar) = &mut self.find_bar {
            let find_rect = egui::Rect::from_min_size(
                ui.cursor().min,
                egui::vec2(ui.available_width(), FIND_BAR_HEIGHT),
            );
            ui.advance_cursor_after_rect(find_rect);

            ui.painter()
                .rect_filled(find_rect, 0.0, colors.pane_header_bg());

            let mut find_ui = ui.new_child(egui::UiBuilder::new().max_rect(find_rect));
            find_ui.horizontal_centered(|ui| {
                ui.add_space(8.0);

                let input_id = egui::Id::new("text_editor_find_input").with(&self.path);
                let input_width = (find_rect.width() - 140.0).max(80.0);
                let response = ui.add(
                    egui::TextEdit::singleline(&mut bar.query)
                        .id(input_id)
                        .desired_width(input_width)
                        .font(egui::FontId::proportional(FIND_BAR_FONT_SIZE))
                        .hint_text("Find…")
                        .frame(false),
                );

                if bar.focus_requested {
                    response.request_focus();
                    bar.focus_requested = false;
                }

                if response.changed() {
                    bar.recompute(&self.content);
                    log::info!(
                        "TextEditorApp: find query {:?} — {} matches",
                        bar.query,
                        bar.matches.len()
                    );
                }

                ui.add_space(8.0);
                let count_text = if bar.matches.is_empty() {
                    if bar.query.is_empty() {
                        String::new()
                    } else {
                        "No results".to_string()
                    }
                } else {
                    format!("{} / {}", bar.current + 1, bar.matches.len())
                };
                ui.label(
                    egui::RichText::new(count_text)
                        .size(FIND_BAR_FONT_SIZE)
                        .color(colors.text_dim),
                );
            });
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
                self.find_bar = None;
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
