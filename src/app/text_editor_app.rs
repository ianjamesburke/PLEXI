//! Built-in file-backed text editor pane.
//!
//! Thin adapter over the shared editor core (`src/editor/`): all editing —
//! movement, selection, clipboard, undo/redo, IME, mouse placement, indent,
//! smart backspace — flows through [`Document`] / [`EditorCommand`] via
//! [`EditorWidget`]. This file owns only file/note loading, frontmatter,
//! autosave, save errors, the find/replace bar, focus routing, and host
//! command surface.

use crate::app::app_trait::{App, AppCommand, AppRenderContext, KeyDisposition};
use crate::editor::widget::EditorWidget;
use crate::editor::{movement, Document, EditorCommand, ViewState};
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
    replace: String,
    /// Char-offset `(start, end)` ranges of each match in the document body.
    matches: Vec<(usize, usize)>,
    /// Index into `matches` for the current (highlighted) match.
    current: usize,
    /// One-shot: claim the find input as this pane's focused surface on the
    /// next render (via `claim_text_surface`; the reconciler grants it).
    claim_focus: bool,
}

impl FindBar {
    fn new() -> Self {
        Self {
            query: String::new(),
            replace: String::new(),
            matches: Vec::new(),
            current: 0,
            claim_focus: true,
        }
    }

    fn recompute(&mut self, content: &str) {
        self.matches.clear();
        if self.query.is_empty() {
            return;
        }
        // Case folding per char (first lowercase mapping) keeps the folded
        // sequence the same length as the original, so match offsets are
        // valid char offsets into `content`.
        let fold = |s: &str| -> Vec<char> {
            s.chars()
                .map(|c| c.to_lowercase().next().unwrap_or(c))
                .collect()
        };
        let haystack = fold(content);
        let needle = fold(&self.query);
        if needle.is_empty() || needle.len() > haystack.len() {
            return;
        }
        let mut i = 0;
        while i + needle.len() <= haystack.len() {
            if haystack[i..i + needle.len()] == needle[..] {
                self.matches.push((i, i + needle.len()));
                i += needle.len();
            } else {
                i += 1;
            }
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
            self.current = self
                .current
                .checked_sub(1)
                .unwrap_or(self.matches.len() - 1);
        }
    }
}

pub struct TextEditorApp {
    path: PathBuf,
    /// Editable document. For notes this holds the body only — the
    /// frontmatter block is held in `note_header` and never shown.
    doc: Document,
    /// Viewport/scroll state for the editor widget.
    view: ViewState,
    /// Raw frontmatter block (both `---` fences, trailing newline) for files
    /// under the notes dir. Recomposed in front of the body on save.
    note_header: Option<String>,
    /// Display title parsed from `note_header` (empty when unset).
    note_title: String,
    /// True when `path` lives under `<config_dir>/notes/`.
    is_note: bool,
    last_edit: Option<Instant>,
    wants_close: bool,
    load_error: Option<String>,
    font_size: f32,
    /// Active find/replace bar, or `None` when dismissed.
    find_bar: Option<FindBar>,
    editor_focused: bool,
    last_save_result: Option<String>,
    last_drop_result: Option<serde_json::Value>,
}

impl TextEditorApp {
    pub fn new(path: PathBuf) -> Self {
        let (raw, load_error) = match std::fs::read_to_string(&path) {
            Ok(s) => (s, None),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (String::new(), None),
            Err(e) => (String::new(), Some(e.to_string())),
        };
        log::info!("notes_editor: editor created for {:?} ({} bytes)", path, raw.len());
        let notes_dir = crate::config::config_dir().join("notes");
        let is_note = path.starts_with(&notes_dir);
        let (note_header, content, note_title) = split_note(is_note, raw);
        Self {
            path,
            doc: Document::new(&content),
            view: ViewState::default(),
            note_header,
            note_title,
            is_note,
            last_edit: None,
            wants_close: false,
            load_error,
            font_size: FONT_SIZE_DEFAULT,
            find_bar: None,
            editor_focused: false,
            last_save_result: None,
            last_drop_result: None,
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
        app.doc = Document::new(&content);
        app.note_title = note_title;
        app
    }

    /// Full on-disk document: frontmatter (when held out) + editable body.
    fn composed(&self) -> String {
        match &self.note_header {
            Some(header) => format!("{header}{}", self.doc.text()),
            None => self.doc.text(),
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
        if content != self.doc.text() {
            // Title rewrites only touch the header in practice; rebuild the
            // document (losing undo history) only if the body itself moved.
            self.doc = Document::new(&content);
        }
        self.note_title = note_title;
        self.last_edit = Some(Instant::now());
        log::info!(
            "notes_editor: note title set to {:?} for {:?}",
            self.note_title,
            self.path
        );

        if !title.trim().is_empty() {
            if let Some(parent) = self.path.parent() {
                let slug = slugify_title(title);
                let new_path = parent.join(format!("{slug}.md"));
                if new_path != self.path && !new_path.exists() {
                    match std::fs::rename(&self.path, &new_path) {
                        Ok(()) => {
                            log::info!(
                                "notes_editor: renamed note {:?} -> {:?}",
                                self.path,
                                new_path
                            );
                            self.path = new_path;
                        }
                        Err(e) => {
                            log::warn!(
                                "notes_editor: failed to rename {:?} -> {:?}: {e}",
                                self.path,
                                new_path
                            );
                        }
                    }
                } else if new_path.exists() && new_path != self.path {
                    log::warn!(
                        "notes_editor: skipping rename, target already exists: {:?}",
                        new_path
                    );
                }
            }
        }
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

    /// Runs the debounced autosave when the last edit is old enough. Called
    /// every rendered frame; extracted so tests can drive the race directly.
    fn maybe_autosave(&mut self) {
        if let Some(t) = self.last_edit {
            if t.elapsed() >= DEBOUNCE {
                self.autosave();
            }
        }
    }

    fn flush_with(&mut self, durability: Durability) {
        // Empty content → delete the file rather than writing an empty document.
        if self.is_effectively_empty() {
            if self.path.exists() {
                if let Err(e) = std::fs::remove_file(&self.path) {
                    log::warn!(
                        "notes_editor: failed to delete empty note {:?}: {e}",
                        self.path
                    );
                    self.last_edit = Some(Instant::now());
                    self.last_save_result = Some(format!("error: {e}"));
                    return;
                } else {
                    log::info!("notes_editor: deleted empty note {:?}", self.path);
                    self.last_edit = None;
                }
            } else {
                self.last_edit = None;
            }
            self.last_save_result = Some("ok".to_string());
            log::info!(
                "notes_editor: semantic save completed path={:?} bytes=0 result=ok",
                self.path
            );
            return;
        }
        let document = self.composed();
        match write_note_atomically(&self.path, document.as_bytes(), durability) {
            Ok(()) => {
                self.last_edit = None;
                self.last_save_result = Some("ok".to_string());
                log::info!(
                    "notes_editor: semantic save completed path={:?} bytes={} result=ok",
                    self.path,
                    document.len()
                );
            }
            Err(e) => {
                self.last_edit = Some(Instant::now());
                self.last_save_result = Some(format!("error: {e}"));
                log::warn!("notes_editor: save failed for {:?}: {e}", self.path);
            }
        }
    }

    /// Selects match `range` in the document and scrolls it into view.
    /// Pure selection commands — never dirties the buffer.
    fn select_match(&mut self, range: (usize, usize)) {
        let start = movement::char_to_cursor(self.doc.buffer(), range.0);
        let end = movement::char_to_cursor(self.doc.buffer(), range.1);
        self.doc.apply(EditorCommand::SetCursor(start));
        self.doc.apply(EditorCommand::ExtendTo(end));
        let line_count = self.doc.buffer().line_count();
        self.view.scroll_to_line(end.line, line_count);
    }

    /// Replaces the current match with the bar's replacement text through the
    /// shared model (selection replace = one undoable transaction).
    fn replace_current(&mut self) {
        let Some(bar) = &mut self.find_bar else {
            return;
        };
        let Some(&range) = bar.matches.get(bar.current) else {
            return;
        };
        let replacement = bar.replace.clone();
        self.select_match(range);
        self.doc.apply(EditorCommand::InsertText(replacement));
        self.last_edit = Some(Instant::now());
        let text = self.doc.text();
        if let Some(bar) = &mut self.find_bar {
            bar.recompute(&text);
        }
        log::info!("notes_editor: replaced current find match");
    }

    /// Replaces every match, back to front so earlier offsets stay valid.
    fn replace_all(&mut self) {
        let Some(bar) = &mut self.find_bar else {
            return;
        };
        if bar.matches.is_empty() {
            return;
        }
        let replacement = bar.replace.clone();
        let ranges: Vec<(usize, usize)> = bar.matches.iter().copied().rev().collect();
        let count = ranges.len();
        for range in ranges {
            self.select_match(range);
            self.doc
                .apply(EditorCommand::InsertText(replacement.clone()));
        }
        self.last_edit = Some(Instant::now());
        let text = self.doc.text();
        if let Some(bar) = &mut self.find_bar {
            bar.recompute(&text);
        }
        log::info!("notes_editor: replace-all rewrote {count} matches");
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

/// Convert a note title into a safe lowercase filename slug (no `.md` suffix).
/// Non-alphanumeric characters become `-`; leading/trailing and consecutive
/// dashes are collapsed; falls back to `"note"` for an all-symbol title.
fn slugify_title(title: &str) -> String {
    let slug: String = title
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    // Collapse runs of dashes and trim edges.
    let slug = slug
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if slug.is_empty() {
        "note".to_string()
    } else {
        slug
    }
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
    #[cfg(test)]
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

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

    fn handle_key(&mut self, input: &crate::app::input_router::PlexiInput) -> KeyDisposition {
        if input.key_pressed(egui::Key::S)
            && input
                .modifiers()
                .matches_logically(egui::Modifiers::COMMAND)
        {
            self.flush();
            return KeyDisposition::Consumed;
        }
        // Cmd+F: open find bar (or re-focus if already open).
        if input.key_pressed(egui::Key::F)
            && input
                .modifiers()
                .matches_logically(egui::Modifiers::COMMAND)
        {
            log::info!("notes_editor: Cmd+F — opening find bar");
            match &mut self.find_bar {
                Some(bar) => bar.claim_focus = true,
                None => {
                    let mut bar = FindBar::new();
                    bar.recompute(&self.doc.text());
                    self.find_bar = Some(bar);
                }
            }
            return KeyDisposition::Consumed;
        }

        if let Some(bar) = &mut self.find_bar {
            // Escape: close the find bar.
            if input.key_pressed(egui::Key::Escape) {
                log::info!("notes_editor: Escape — closing find bar");
                self.find_bar = None;
                return KeyDisposition::Consumed;
            }
            // Enter: next match. Shift+Enter: previous match.
            if input.key_pressed(egui::Key::Enter) {
                let forward = !input.modifiers().shift;
                bar.advance(forward);
                if let Some(&range) = bar.matches.get(bar.current) {
                    self.select_match(range);
                }
                return KeyDisposition::Consumed;
            }
        }

        KeyDisposition::Passthrough
    }

    fn adjust_font_size(&mut self, delta: f32) {
        self.font_size = (self.font_size + delta).clamp(FONT_SIZE_MIN, FONT_SIZE_MAX);
        log::info!("notes_editor: font_size -> {}", self.font_size);
    }

    fn ui(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &AppRenderContext<'_>,
        _pending_click: Option<crate::host::pane::PendingPaneClick>,
    ) {
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
        // The editor is this pane's default text surface (stint 0429): the
        // post-frame reconciler grants it focus while the pane owns input.
        crate::ui::focus::register_default_text_surface(
            ui.ctx(),
            crate::ui::focus::SurfaceKey::Pane(ctx.pane_id),
            te_id,
        );

        // The memory entry is the production focus authority used by keyboard
        // dispatch and survives CLI focus while the OS window is blurred.
        let editor_focused = ui.ctx().memory(|memory| memory.has_focus(te_id));
        let find_input_id = egui::Id::new("text_editor_find_input").with(&self.path);
        let replace_input_id = egui::Id::new("text_editor_replace_input").with(&self.path);
        let find_focused = self.find_bar.is_some()
            && ui.ctx().memory(|memory| {
                memory.has_focus(find_input_id) || memory.has_focus(replace_input_id)
            });

        // App-level shortcuts, consumed out of the frame's event queue before
        // the editor widget (which reads the same queue) can see them. Gated
        // on this pane's surfaces owning input so an unfocused notes pane
        // never steals another pane's keys.
        if editor_focused || find_focused {
            if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S)) {
                self.flush();
            }
            if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::F)) {
                log::info!("notes_editor: Cmd+F — opening find bar");
                match &mut self.find_bar {
                    Some(bar) => bar.claim_focus = true,
                    None => {
                        let mut bar = FindBar::new();
                        bar.recompute(&self.doc.text());
                        self.find_bar = Some(bar);
                    }
                }
            }
            if self.find_bar.is_some() {
                if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                    log::info!("notes_editor: Escape — closing find bar");
                    self.find_bar = None;
                } else {
                    let back =
                        ui.input_mut(|i| i.consume_key(egui::Modifiers::SHIFT, egui::Key::Enter));
                    let forward =
                        ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
                    if back || forward {
                        if let Some(bar) = &mut self.find_bar {
                            bar.advance(forward);
                            if let Some(&range) = bar.matches.get(bar.current) {
                                self.select_match(range);
                            }
                        }
                    }
                }
            }
        }

        // When the find bar is open, reserve its height at the bottom before
        // laying out the editor so it doesn't overlap the bar.
        let find_bar_height = if self.find_bar.is_some() {
            FIND_BAR_HEIGHT
        } else {
            0.0
        };
        let editor_height = (ui.available_height() - find_bar_height).max(1.0);
        let editor_rect = egui::Rect::from_min_size(
            ui.cursor().min,
            egui::vec2(ui.available_width(), editor_height),
        );

        let (highlights, current_highlight) = match &self.find_bar {
            Some(bar) => (bar.matches.clone(), Some(bar.current)),
            None => (Vec::new(), None),
        };
        let revision_before = self.doc.revision();
        // Same 4px horizontal text inset the old TextEdit margin provided.
        let mut editor_ui = ui.new_child(
            egui::UiBuilder::new().max_rect(editor_rect.shrink2(egui::vec2(4.0, 0.0))),
        );
        editor_ui.visuals_mut().override_text_color = Some(colors.text_primary);
        editor_ui.visuals_mut().selection.stroke.color = colors.accent;
        // Markdown-aware keyboard behavior for notes and .md/.markdown files.
        let markdown = self.is_note
            || self
                .path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("md") || e.eq_ignore_ascii_case("markdown"));
        let response = EditorWidget::new(&mut self.doc, &mut self.view)
            .id(te_id)
            .active(editor_focused)
            .font_size(self.font_size)
            .markdown(markdown)
            .highlights(
                highlights,
                current_highlight,
                colors.warning.gamma_multiply(0.45),
                colors.accent.gamma_multiply(0.55),
            )
            .show(&mut editor_ui);
        ui.advance_cursor_after_rect(editor_rect);

        // Clicking the editor while the find input holds focus claims the
        // editor surface back (the reconciler grants it post-frame).
        if response.clicked() && !editor_focused {
            crate::ui::focus::claim_text_surface(
                ui.ctx(),
                crate::ui::focus::SurfaceKey::Pane(ctx.pane_id),
                te_id,
            );
        }

        if self.doc.revision() != revision_before {
            self.last_edit = Some(Instant::now());
            let text = self.doc.text();
            if let Some(bar) = &mut self.find_bar {
                bar.recompute(&text);
            }
        }

        if editor_focused != self.editor_focused {
            log::info!("notes_editor: focus transition focused={editor_focused}");
            self.editor_focused = editor_focused;
        }

        self.maybe_autosave();

        // Render the find/replace bar below the editor.
        if self.find_bar.is_some() {
            let find_rect = egui::Rect::from_min_size(
                ui.cursor().min,
                egui::vec2(ui.available_width(), FIND_BAR_HEIGHT),
            );
            ui.advance_cursor_after_rect(find_rect);

            ui.painter()
                .rect_filled(find_rect, 0.0, colors.pane_header_bg());

            let mut replace_one = false;
            let mut replace_every = false;
            let mut find_ui = ui.new_child(egui::UiBuilder::new().max_rect(find_rect));
            find_ui.horizontal_centered(|ui| {
                let Some(bar) = &mut self.find_bar else {
                    return;
                };
                ui.add_space(8.0);

                let input_width = ((find_rect.width() - 260.0) / 2.0).max(60.0);
                let response = ui.add(
                    egui::TextEdit::singleline(&mut bar.query)
                        .id(find_input_id)
                        .desired_width(input_width)
                        .font(egui::FontId::proportional(FIND_BAR_FONT_SIZE))
                        .hint_text("Find…")
                        .frame(egui::Frame::NONE),
                );

                crate::ui::focus::register_text_surface(
                    ui.ctx(),
                    crate::ui::focus::SurfaceKey::Pane(ctx.pane_id),
                    find_input_id,
                );
                if bar.claim_focus {
                    crate::ui::focus::claim_text_surface(
                        ui.ctx(),
                        crate::ui::focus::SurfaceKey::Pane(ctx.pane_id),
                        find_input_id,
                    );
                    bar.claim_focus = false;
                }

                let query_changed = response.changed();

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

                ui.add_space(8.0);
                ui.add(
                    egui::TextEdit::singleline(&mut bar.replace)
                        .id(replace_input_id)
                        .desired_width(input_width)
                        .font(egui::FontId::proportional(FIND_BAR_FONT_SIZE))
                        .hint_text("Replace…")
                        .frame(egui::Frame::NONE),
                );
                // Registered under the pane so the post-frame focus
                // reconciler keeps a clicked replace field focused instead of
                // snapping focus back to the editor.
                crate::ui::focus::register_text_surface(
                    ui.ctx(),
                    crate::ui::focus::SurfaceKey::Pane(ctx.pane_id),
                    replace_input_id,
                );
                let has_matches = !bar.matches.is_empty();
                replace_one = ui
                    .add_enabled(has_matches, egui::Button::new("Replace"))
                    .clicked();
                replace_every = ui
                    .add_enabled(has_matches, egui::Button::new("All"))
                    .clicked();

                if query_changed {
                    let text = self.doc.text();
                    if let Some(bar) = &mut self.find_bar {
                        bar.recompute(&text);
                        log::info!("notes_editor: find query changed — {} matches", bar.matches.len());
                    }
                }
            });
            if replace_one {
                self.replace_current();
            }
            if replace_every {
                self.replace_all();
            }
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
                    "notes_editor: switching from {:?} to {:?}",
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
                self.doc = Document::new(&content);
                self.view = ViewState::default();
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

    fn semantic_state(&self) -> Option<serde_json::Value> {
        let sem = self.doc.semantic_state(self.view.scroll_y);
        let buffer = self.doc.buffer();
        let anchor_char = movement::cursor_to_char(buffer, sem.selection.anchor);
        let caret_char = movement::cursor_to_char(buffer, sem.cursor);
        let cursor = movement::clamp(buffer, sem.cursor);
        let line_start_char = buffer.line_to_char(cursor.line);
        let active = movement::line_text(buffer, cursor.line);
        let line_end_char = line_start_char + active.chars().count();
        let line_count = buffer.line_count();
        let visible = self.view.visible_lines(line_count);
        let visible_start = if visible.start < line_count {
            buffer.line_to_char(visible.start)
        } else {
            buffer.len()
        };
        let visible_end = if visible.end < line_count {
            buffer.line_to_char(visible.end)
        } else {
            buffer.len()
        };
        let visible_source = buffer.slice(visible_start, visible_end);
        let links = markdown_targets(&visible_source, false);
        let images = markdown_targets(&visible_source, true);
        Some(serde_json::json!({
            "kind": "notes_editor",
            "source_text": sem.text,
            "primary_selection": {"anchor": anchor_char, "caret": caret_char},
            "caret": caret_char,
            "scroll": {"y": sem.scroll_y},
            "dirty": self.last_edit.is_some(),
            "last_save_result": self.last_save_result,
            "active_markdown_block": {"start": line_start_char, "end": line_end_char, "source": active, "granularity": "source_line"},
            "visible_link_targets": links,
            "visible_images": images,
            "undo_available": sem.can_undo,
            "redo_available": sem.can_redo,
            "focused": self.editor_focused,
            "last_drop_result": self.last_drop_result,
            "path": self.path,
        }))
    }

    fn drop_file(&mut self, path_or_url: &str) -> Result<serde_json::Value, String> {
        let source_kind = if path_or_url.contains("://") {
            "url"
        } else {
            "file"
        };
        self.last_drop_result = Some(serde_json::json!({
            "result": "rejected",
            "source_kind": source_kind,
        }));
        Err("Notes does not yet accept file drops".to_string())
    }
}

fn markdown_targets(content: &str, images: bool) -> Vec<String> {
    let prefix = if images { "![" } else { "[" };
    content
        .match_indices(prefix)
        .filter_map(|(start, _)| {
            if !images && start > 0 && content.as_bytes()[start - 1] == b'!' {
                return None;
            }
            let rest = &content[start + prefix.len()..];
            let open = rest.find("](")? + start + prefix.len() + 2;
            let end = content[open..].find(')')? + open;
            Some(content[open..end].to_string())
        })
        .collect()
}

impl Drop for TextEditorApp {
    fn drop(&mut self) {
        // Save unsaved edits; also clean up empty notes (flush deletes them).
        if self.last_edit.is_some() || self.is_effectively_empty() {
            self.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::Cursor;

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
    fn semantic_active_block_handles_unicode_caret_positions() {
        let dir = unique_temp_dir("notes-semantic-unicode");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("unicode.md");
        std::fs::write(&path, "😀 café\nsecond").unwrap();
        let mut app = TextEditorApp::new_for_test_note(path);
        app.doc.apply(EditorCommand::SetCursor(Cursor::new(0, 6)));
        let state = crate::app::app_trait::App::semantic_state(&app).unwrap();
        assert_eq!(state["active_markdown_block"]["source"], "😀 café");
        assert_eq!(state["caret"], 6);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn semantic_state_reports_multiline_selection_as_char_offsets() {
        let dir = unique_temp_dir("notes-semantic-multiline");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("multi.md");
        std::fs::write(&path, "one\ntwo\nthree").unwrap();
        let mut app = TextEditorApp::new_for_test_note(path);
        app.doc.apply(EditorCommand::SetCursor(Cursor::new(0, 2)));
        app.doc.apply(EditorCommand::ExtendTo(Cursor::new(2, 3)));
        let state = crate::app::app_trait::App::semantic_state(&app).unwrap();
        assert_eq!(state["primary_selection"]["anchor"], 2);
        assert_eq!(state["primary_selection"]["caret"], 11);
        assert_eq!(app.doc.selected_text(), "e\ntwo\nthr");
        let _ = std::fs::remove_dir_all(dir);
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
    fn find_bar_recompute_finds_case_insensitive_char_ranges() {
        let mut bar = FindBar::new();
        bar.query = "hello".to_string();
        let content = "Hello world, hello there, HELLO!";
        bar.recompute(content);
        assert_eq!(
            bar.matches,
            vec![(0, 5), (13, 18), (26, 31)],
            "char-offset ranges for each case-insensitive match"
        );
        // Unicode before a match: offsets are chars, not bytes.
        let mut bar = FindBar::new();
        bar.query = "x".to_string();
        bar.recompute("émoji 😀 x");
        assert_eq!(bar.matches, vec![(8, 9)]);
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

    #[test]
    fn replace_current_and_replace_all_route_through_document() {
        let dir = unique_temp_dir("notes-replace");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("replace.md");
        std::fs::write(&path, "foo bar foo baz foo").unwrap();
        let mut app = TextEditorApp::new_for_test_note(path);
        let mut bar = FindBar::new();
        bar.query = "foo".to_string();
        bar.replace = "qux".to_string();
        bar.recompute(&app.doc.text());
        app.find_bar = Some(bar);

        app.replace_current();
        assert_eq!(app.doc.text(), "qux bar foo baz foo");
        assert!(app.last_edit.is_some(), "replace marks the buffer dirty");
        // Each replace is undoable through the shared history.
        app.doc.apply(EditorCommand::Undo);
        assert_eq!(app.doc.text(), "foo bar foo baz foo");
        if let Some(bar) = &mut app.find_bar {
            let text = "foo bar foo baz foo".to_string();
            bar.recompute(&text);
        }

        app.replace_all();
        assert_eq!(app.doc.text(), "qux bar qux baz qux");
        assert!(app.find_bar.as_ref().unwrap().matches.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn select_match_selects_without_dirtying_the_buffer() {
        let dir = unique_temp_dir("notes-select-match");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sel.md");
        std::fs::write(&path, "alpha beta gamma").unwrap();
        let mut app = TextEditorApp::new_for_test_note(path);
        let before = app.doc.revision();
        app.select_match((6, 10));
        assert_eq!(app.doc.selected_text(), "beta");
        assert_eq!(app.doc.revision(), before, "selection is not an edit");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn autosave_debounce_only_fires_after_the_delay() {
        let dir = unique_temp_dir("notes-autosave-debounce");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("debounce.md");
        std::fs::write(&path, "seed").unwrap();
        let mut app = TextEditorApp::new_for_test_note(path.clone());
        app.doc.apply(EditorCommand::Move {
            movement: crate::editor::commands::Movement::DocEnd,
            extend: false,
        });
        app.doc.apply(EditorCommand::InsertText(" more".into()));
        app.last_edit = Some(Instant::now());

        // Within the debounce window: no write happens.
        app.maybe_autosave();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "seed");
        assert!(app.last_edit.is_some());

        // A keystroke mid-window pushes the deadline out (autosave race:
        // the save must reflect the latest edit, never a stale buffer).
        app.doc.apply(EditorCommand::InsertText("!".into()));
        app.last_edit = Some(Instant::now() - DEBOUNCE);
        app.maybe_autosave();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "seed more!");
        assert!(app.last_edit.is_none(), "successful save clears dirty");
        assert_eq!(app.last_save_result.as_deref(), Some("ok"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn save_failure_surfaces_error_and_stays_dirty() {
        let dir = unique_temp_dir("notes-save-failure");
        std::fs::create_dir_all(&dir).unwrap();
        // Parent "note.md" is a *file*, so create_dir_all for the child's
        // parent fails and the save errors.
        let blocker = dir.join("blocker");
        std::fs::write(&blocker, "in the way").unwrap();
        let path = blocker.join("child.md");
        let mut app = TextEditorApp::new(path);
        app.doc.apply(EditorCommand::InsertText("content".into()));
        app.last_edit = Some(Instant::now() - DEBOUNCE);
        app.maybe_autosave();
        assert!(
            app.last_save_result
                .as_deref()
                .is_some_and(|r| r.starts_with("error:")),
            "failed save surfaces an error result, got {:?}",
            app.last_save_result
        );
        assert!(app.last_edit.is_some(), "failed save stays dirty for retry");
        // Silence the Drop-flush retry against the same broken path.
        app.doc.apply(EditorCommand::Undo);
        app.last_edit = None;
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn reopen_round_trips_body_and_frontmatter() {
        let dir = unique_temp_dir("notes-reopen");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("note.md");
        std::fs::write(
            &path,
            "---\ntitle: \"Trip\"\nsource: \"scratchpad\"\n---\npacking list\n",
        )
        .unwrap();
        {
            let mut app = TextEditorApp::new_for_test_note(path.clone());
            assert_eq!(app.doc.text(), "packing list\n");
            app.doc.apply(EditorCommand::Move {
                movement: crate::editor::commands::Movement::DocEnd,
                extend: false,
            });
            app.doc.apply(EditorCommand::InsertText("tent\n".into()));
            app.last_edit = Some(Instant::now());
            app.flush();
        }
        let reopened = TextEditorApp::new_for_test_note(path.clone());
        assert_eq!(reopened.doc.text(), "packing list\ntent\n");
        assert_eq!(reopened.note_title, "Trip");
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .starts_with("---\ntitle: \"Trip\""));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn slugify_basic() {
        assert_eq!(slugify_title("My Note Title"), "my-note-title");
    }

    #[test]
    fn slugify_collapses_special_chars() {
        assert_eq!(slugify_title("Hello, World!"), "hello-world");
    }

    #[test]
    fn slugify_empty_falls_back_to_note() {
        assert_eq!(slugify_title("---"), "note");
    }
}
