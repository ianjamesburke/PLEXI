//! Built-in file-backed text editor pane.
//!
//! Thin adapter over the shared editor core (`src/editor/`): all editing —
//! movement, selection, clipboard, undo/redo, IME, mouse placement, indent,
//! smart backspace — flows through [`Document`] / [`EditorCommand`] via
//! [`EditorWidget`]. This file owns only file/note loading, frontmatter,
//! autosave, save errors, the find/replace bar, focus routing, and host
//! command surface.

use crate::app::app_trait::{App, AppCommand, AppRenderContext, KeyDisposition};
use crate::editor::preview::{self, LinkKind, LinkTarget, MarkdownLayoutCache};
use crate::editor::widget::{CodeTheme, EditorWidget, ImageCache, MarkdownTheme};
use crate::editor::{movement, Document, EditorCommand, EditorMode, SyntaxHighlighter, ViewState};
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
    /// Editor presentation/input mode, detected from `path` (extension) and
    /// note-ness. The core never inspects file metadata itself.
    mode: EditorMode,
    /// Syntax span source for code mode; `None` when the language is unknown
    /// to the bundled syntax set (plain-text fallback).
    highlighter: Option<SyntaxHighlighter>,
    /// Markdown block/inline layout cache for Live Preview; present only in
    /// Markdown mode.
    md_cache: Option<MarkdownLayoutCache>,
    last_edit: Option<Instant>,
    wants_close: bool,
    load_error: Option<String>,
    font_size: f32,
    /// Active find/replace bar, or `None` when dismissed.
    find_bar: Option<FindBar>,
    editor_focused: bool,
    /// True after Escape released the body editor back to pane-level
    /// navigation: the pane stays open and focused, but the editor stops
    /// claiming the pane's text surface so host pane-nav keys are no longer
    /// swallowed. Cleared when the user clicks back into the editor.
    input_released: bool,
    last_save_result: Option<String>,
    last_drop_result: Option<serde_json::Value>,
    /// Result of the most recent link activation (kind, target, outcome).
    last_link_activation: Option<serde_json::Value>,
    /// Host commands queued by link activation, drained by the host each
    /// frame via `take_pending_commands`.
    pending_commands: Vec<AppCommand>,
    /// Inline Live Preview image textures (mtime-keyed reload inside).
    image_cache: ImageCache,
    /// This pane's host id, observed from the render context each frame.
    /// Anchors spawned link-target panes next to this note, not whatever
    /// pane happens to be focused at dispatch time.
    host_pane_id: Option<u64>,
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
        let mode = detect_mode(&path, is_note);
        log::info!("notes_editor: mode selected {} for {:?}", mode.describe(), path);
        let highlighter = mode.language().and_then(SyntaxHighlighter::new);
        let md_cache = mode.is_markdown().then(MarkdownLayoutCache::default);
        let mut doc = Document::new(&content);
        let mut view = ViewState::default();
        if is_note {
            position_caret_at_end(&mut doc, &mut view);
        }
        Self {
            path,
            doc,
            view,
            note_header,
            note_title,
            is_note,
            mode,
            highlighter,
            md_cache,
            last_edit: None,
            wants_close: false,
            load_error,
            font_size: FONT_SIZE_DEFAULT,
            find_bar: None,
            editor_focused: false,
            input_released: false,
            last_save_result: None,
            last_drop_result: None,
            last_link_activation: None,
            pending_commands: Vec::new(),
            image_cache: ImageCache::default(),
            host_pane_id: None,
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
        app.mode = detect_mode(&app.path, true);
        app.highlighter = app.mode.language().and_then(SyntaxHighlighter::new);
        app.md_cache = app.mode.is_markdown().then(MarkdownLayoutCache::default);
        app
    }

    /// The row metric the widget quantized to the physical pixel grid on its
    /// last `show()` (stint 0529). Reads back the composited pane's value so
    /// the crispness gate can assert it is integer-physical after a real host
    /// render, not just in isolation.
    #[cfg(test)]
    pub(crate) fn test_view_line_height(&self) -> f32 {
        self.view.line_height
    }

    /// Park the viewport at a fractional scroll offset so the crispness gate
    /// can render the note mid-scroll — the case where an unsnapped scroll_y
    /// would push painted rows off the physical pixel grid (stint 0529).
    #[cfg(test)]
    pub(crate) fn test_set_scroll_y(&mut self, y: f32) {
        self.view.scroll_y = y;
    }

    /// Flips Markdown presentation between Live Preview and source mode.
    /// Presentation only: document, selection, history, IME, and scroll
    /// anchor are untouched. No-op outside Markdown mode.
    fn toggle_preview_mode(&mut self) {
        if let EditorMode::Markdown { live_preview } = &mut self.mode {
            *live_preview = !*live_preview;
            log::info!(
                "notes_editor: preview mode changed -> {} for {:?}",
                self.mode.describe(),
                self.path
            );
        }
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

    /// Caret position as a byte offset into the document body.
    fn caret_byte(&self) -> usize {
        let text = self.doc.text();
        let caret_char = movement::cursor_to_char(self.doc.buffer(), self.doc.cursor());
        text.char_indices()
            .nth(caret_char)
            .map_or(text.len(), |(i, _)| i)
    }

    /// Selects `range` (document byte offsets) and scrolls it into view.
    fn select_byte_range(&mut self, range: &std::ops::Range<usize>) {
        let text = self.doc.text();
        let start_char = text[..range.start].chars().count();
        let end_char = start_char + text[range.start..range.end].chars().count();
        self.select_match((start_char, end_char));
    }

    /// Inserts `text` at the caret (replacing any selection) as exactly one
    /// undo step: the surrounding `SetCursor`s break typing coalescing on
    /// both sides without moving the caret.
    fn insert_isolated(&mut self, text: String) {
        if !self.doc.selection().is_range() {
            // Break typing coalescing without disturbing a selection (a
            // selection replace never coalesces on its own).
            self.doc.apply(EditorCommand::SetCursor(self.doc.cursor()));
        }
        self.doc.apply(EditorCommand::InsertText(text));
        self.doc.apply(EditorCommand::SetCursor(self.doc.cursor()));
        self.last_edit = Some(Instant::now());
    }

    /// Ctrl+K: with the caret inside an existing link, selects its
    /// destination for editing; otherwise wraps the selection (or an empty
    /// caret) in `[…](url)` with the `url` placeholder selected. One undo
    /// step either way.
    fn create_or_edit_link(&mut self) {
        let text = self.doc.text();
        let caret_b = self.caret_byte();
        let existing = preview::link_targets(&text)
            .into_iter()
            .find(|l| l.bytes.contains(&caret_b) || l.bytes.end == caret_b);
        if let Some(link) = existing {
            let slice = &text[link.bytes.clone()];
            let dest_range = match link.kind {
                LinkKind::Wiki => link.bytes.start + 2..link.bytes.end - 2,
                LinkKind::Markdown => match slice.find("](") {
                    Some(i) => link.bytes.start + i + 2..link.bytes.end - 1,
                    None => link.bytes.clone(),
                },
                LinkKind::Autolink => link.bytes.clone(),
            };
            self.select_byte_range(&dest_range);
            log::info!("notes_editor: link edit — selected destination of {:?}", link.dest);
            return;
        }
        let selected = self.doc.selected_text();
        let insert = format!("[{selected}](url)");
        self.insert_isolated(insert);
        // Select the `url` placeholder (3 chars before the closing paren).
        let caret_char = movement::cursor_to_char(self.doc.buffer(), self.doc.cursor());
        self.select_match((caret_char - 4, caret_char - 1));
        log::info!(
            "notes_editor: link created around {} selected chars",
            selected.chars().count()
        );
    }

    /// Selects the next/previous link relative to the caret, wrapping.
    fn focus_visible_link(&mut self, forward: bool) {
        let text = self.doc.text();
        let links = preview::link_targets(&text);
        if links.is_empty() {
            log::info!("notes_editor: focus link — no links in document");
            return;
        }
        let caret_b = self.caret_byte();
        let target = if forward {
            links
                .iter()
                .find(|l| l.bytes.start >= caret_b)
                .unwrap_or(&links[0])
        } else {
            links
                .iter()
                .rev()
                .find(|l| l.bytes.end < caret_b)
                .unwrap_or_else(|| links.last().expect("non-empty"))
        };
        let bytes = target.bytes.clone();
        log::info!(
            "notes_editor: focus {} link -> {:?}",
            if forward { "next" } else { "prev" },
            target.dest
        );
        self.select_byte_range(&bytes);
    }

    /// Keyboard link activation: the link containing the caret, if any.
    fn activate_link_at_caret(&mut self) {
        let text = self.doc.text();
        let caret_b = self.caret_byte();
        let link = preview::link_targets(&text)
            .into_iter()
            .find(|l| l.bytes.contains(&caret_b) || l.bytes.end == caret_b);
        match link {
            Some(link) => self.activate_link(&link),
            None => {
                log::info!("notes_editor: link activation — no link at caret");
                self.last_link_activation = Some(serde_json::json!({
                    "outcome": "no_link_at_caret",
                }));
            }
        }
    }

    /// Activates a link: external http(s) URLs go through the scheme-validated
    /// host opener; wiki and relative links resolve against the notes
    /// collection / note directory and open the target note. Missing or
    /// ambiguous targets surface deterministically — a file is never created.
    fn activate_link(&mut self, link: &LinkTarget) {
        let kind = match link.kind {
            LinkKind::Markdown => "markdown",
            LinkKind::Autolink => "autolink",
            LinkKind::Wiki => "wiki",
        };
        let (outcome, detail) = if link.dest.contains("://") {
            match crate::app::canvas_bindings::open_http_url(&link.dest) {
                Ok(()) => ("opened_external".to_string(), None),
                Err(e) => ("open_failed".to_string(), Some(e)),
            }
        } else if link.kind == LinkKind::Wiki {
            self.resolve_wiki_link(&link.dest)
        } else {
            self.resolve_relative_link(&link.dest)
        };
        log::info!(
            "notes_editor: link activation kind={kind} target={:?} outcome={outcome}{}",
            link.dest,
            detail
                .as_deref()
                .map(|d| format!(" detail={d}"))
                .unwrap_or_default()
        );
        self.last_link_activation = Some(serde_json::json!({
            "kind": kind,
            "target": link.dest,
            "outcome": outcome,
            "detail": detail,
        }));
    }

    /// Resolves `[[name]]` against `<config_dir>/notes/**/<name>.md`. A unique
    /// match opens that note; multiple matches surface deterministically; a
    /// missing target is created as a blank note under `<config_dir>/notes/`
    /// and opened (standard wiki behavior — see [`Self::create_wiki_note`]).
    fn resolve_wiki_link(&mut self, name: &str) -> (String, Option<String>) {
        let notes_dir = crate::config::config_dir().join("notes");
        let mut matches: Vec<PathBuf> = Vec::new();
        let mut stack = vec![notes_dir.clone()];
        while let Some(dir) = stack.pop() {
            let entries = match std::fs::read_dir(&dir) {
                Ok(entries) => entries,
                Err(e) => {
                    if dir == notes_dir {
                        // A not-yet-created notes root is an empty collection,
                        // not an error — the first `[[link]]` should still
                        // create and open its target (create_wiki_note makes
                        // the root). Only a genuine read failure surfaces.
                        if e.kind() == std::io::ErrorKind::NotFound {
                            break;
                        }
                        return (
                            "missing".to_string(),
                            Some(format!("notes dir unreadable: {e}")),
                        );
                    }
                    log::warn!("notes_editor: wiki resolve skipping {dir:?}: {e}");
                    continue;
                }
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let file_type = match entry.file_type() {
                    Ok(ft) => ft,
                    Err(e) => {
                        log::warn!("notes_editor: wiki resolve skipping {path:?}: {e}");
                        continue;
                    }
                };
                // Never descend symlinked directories: a cycle (notes/loop →
                // notes) would spin the UI thread forever.
                if file_type.is_dir() {
                    stack.push(path);
                } else if file_type.is_file()
                    && path.extension().and_then(|e| e.to_str()) == Some("md")
                    && path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .is_some_and(|stem| stem.eq_ignore_ascii_case(name))
                {
                    matches.push(path);
                }
            }
        }
        matches.sort();
        match matches.len() {
            0 => self.create_wiki_note(&notes_dir, name),
            1 => {
                self.open_note_pane(&matches[0]);
                ("opened_note".to_string(), None)
            }
            n => (
                "ambiguous".to_string(),
                Some(format!(
                    "{n} notes named {name:?}: {}",
                    matches
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
            ),
        }
    }

    /// Creates the missing wiki target `<notes_dir>/<name>.md` as a blank note
    /// and opens it. `name` may carry subdirectories (`[[project/idea]]`); the
    /// parent is created as needed. The write is contained to `notes_dir` — a
    /// target whose real parent escapes the notes collection (e.g. `../`) is
    /// refused rather than written outside the sandbox. On any I/O failure the
    /// operation, path, and error are logged and surfaced as `create_failed`.
    fn create_wiki_note(&mut self, notes_dir: &Path, name: &str) -> (String, Option<String>) {
        let target = notes_dir.join(format!("{name}.md"));
        let Some(parent) = target.parent().map(Path::to_path_buf) else {
            return (
                "create_failed".to_string(),
                Some(format!("wiki target {target:?} has no parent directory")),
            );
        };
        // Containment: the new note must live inside the notes collection.
        let normalized_parent = note_path_identity(&parent);
        let notes_root = note_path_identity(notes_dir);
        if !normalized_parent.starts_with(&notes_root) {
            return (
                "create_failed".to_string(),
                Some(format!(
                    "wiki target {name:?} escapes the notes directory {notes_dir:?}"
                )),
            );
        }
        // A nested target (`[[project/idea]]`) is not found by the stem-only
        // scan even when `project/idea.md` already exists, so guard against
        // truncating a real note: if the file is already there, open it.
        if target.exists() {
            log::info!("notes_editor: wiki target {target:?} already exists — opening it");
            self.open_note_pane(&target);
            return ("opened_note".to_string(), None);
        }
        if let Err(e) = std::fs::create_dir_all(&parent) {
            log::error!("notes_editor: create_wiki_note mkdir {parent:?} failed: {e}");
            return (
                "create_failed".to_string(),
                Some(format!("create dir {parent:?}: {e}")),
            );
        }
        if let Err(e) = std::fs::write(&target, "") {
            log::error!("notes_editor: create_wiki_note write {target:?} failed: {e}");
            return (
                "create_failed".to_string(),
                Some(format!("write {target:?}: {e}")),
            );
        }
        log::info!("notes_editor: created blank wiki note {target:?} for [[{name}]]");
        self.open_note_pane(&target);
        ("created_note".to_string(), None)
    }

    /// Resolves a relative/absolute non-URL link against this note's
    /// directory. Never creates the target.
    fn resolve_relative_link(&mut self, dest: &str) -> (String, Option<String>) {
        let base = self.path.parent().unwrap_or_else(|| Path::new("."));
        let candidate = if Path::new(dest).is_absolute() {
            PathBuf::from(dest)
        } else {
            base.join(dest)
        };
        let resolved = note_path_identity(&candidate);
        if resolved.is_file() {
            self.open_note_pane(&resolved);
            ("opened_note".to_string(), None)
        } else {
            (
                "missing".to_string(),
                Some(format!("no file at {}", resolved.display())),
            )
        }
    }

    /// Queues a host command to open `path` in a new text-editor pane,
    /// anchored to this pane so the split lands next to the source note.
    fn open_note_pane(&mut self, path: &Path) {
        self.pending_commands.push(AppCommand::SpawnPane {
            type_id: "text-editor".to_string(),
            layout: "split_h".to_string(),
            args: vec![path.to_string_lossy().into_owned()],
            from_pane_id: self.host_pane_id,
            request_id: None,
            target_context: None,
        });
    }
}

/// Detects the editor mode from file metadata: notes and Markdown extensions
/// get [`EditorMode::Markdown`]; recognized source-code extensions get
/// [`EditorMode::Code`] with the extension as the language identifier;
/// everything else is plain text. The editor core only ever receives the
/// resulting mode/identifier — it never inspects paths.
/// Source-code extensions the editor opens in [`EditorMode::Code`] with syntax
/// chrome. Also part of the builtin text-editor's file-open claim
/// ([`is_text_editable_ext`]).
const CODE_EXTENSIONS: &[&str] = &[
    "rs", "py", "js", "ts", "jsx", "tsx", "json", "toml", "yaml", "yml", "sh", "bash", "zsh", "c",
    "h", "cpp", "hpp", "cc", "go", "rb", "java", "html", "css", "xml", "lua", "swift", "sql", "php",
];

/// Plain-text/prose extensions the editor opens in Markdown or plain mode, in
/// addition to [`CODE_EXTENSIONS`].
const TEXT_EXTENSIONS: &[&str] = &["md", "markdown", "txt", "text", "log", "csv", "conf", "ini"];

/// Whether the builtin text-editor should own opens for `ext` (case-insensitive,
/// no leading dot). Union of the prose and source-code sets the editor already
/// renders. Used by the single file-open resolver so double-clicking a text
/// file in the explorer lands in the editor instead of the OS opener.
pub(crate) fn is_text_editable_ext(ext: &str) -> bool {
    let ext = ext.to_ascii_lowercase();
    TEXT_EXTENSIONS.contains(&ext.as_str()) || CODE_EXTENSIONS.contains(&ext.as_str())
}

fn detect_mode(path: &Path, is_note: bool) -> EditorMode {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    let ext = ext.as_deref();
    if is_note || matches!(ext, Some("md" | "markdown")) {
        // Live Preview is the default Markdown presentation (Obsidian
        // convention); Cmd+E toggles to source mode.
        return EditorMode::Markdown { live_preview: true };
    }
    match ext {
        Some(ext) if CODE_EXTENSIONS.contains(&ext) => EditorMode::Code {
            language: ext.to_string(),
        },
        _ => EditorMode::PlainText,
    }
}

/// Split a note document into its raw frontmatter block (kept out of the
/// editable buffer), the body, and the display title. Non-note files and
/// notes without a frontmatter block pass through unchanged.
/// Continue-writing is the dominant intent for reopening a note (especially
/// one started in the Quick Note editor): land the caret at the end of the
/// document with the viewport anchored to the tail, rather than offset 0.
fn position_caret_at_end(doc: &mut Document, view: &mut ViewState) {
    let line_count = doc.buffer().line_count();
    doc.apply(EditorCommand::Move {
        movement: crate::editor::commands::Movement::DocEnd,
        extend: false,
    });
    view.scroll_to_line(doc.cursor().line, line_count);
}

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

    fn open_note_path(&self) -> Option<&Path> {
        self.is_note.then_some(self.path.as_path())
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
        // Cmd+G: toggle Live Preview / source. (Obsidian uses Cmd+E, but the
        // host binds that to open_file_browser, and host bindings are
        // non-exact so any Cmd+E-with-extra-modifiers triggers it too.)
        if input.key_pressed(egui::Key::G)
            && input
                .modifiers()
                .matches_logically(egui::Modifiers::COMMAND)
            && self.mode.is_markdown()
        {
            self.toggle_preview_mode();
            return KeyDisposition::Consumed;
        }
        // Link commands (Markdown only). Ctrl-based: the host reserves most
        // Cmd single-letter combos (see src/host/keys.rs header).
        if self.mode.is_markdown() {
            let ctrl = input
                .modifiers()
                .matches_logically(egui::Modifiers::CTRL);
            let ctrl_shift = input
                .modifiers()
                .matches_logically(egui::Modifiers::CTRL | egui::Modifiers::SHIFT);
            // Ctrl+K: create link around selection / edit link at caret.
            if input.key_pressed(egui::Key::K) && ctrl {
                self.create_or_edit_link();
                return KeyDisposition::Consumed;
            }
            // Ctrl+L / Ctrl+Shift+L: focus next / previous link.
            if input.key_pressed(egui::Key::L) && ctrl_shift {
                self.focus_visible_link(false);
                return KeyDisposition::Consumed;
            }
            if input.key_pressed(egui::Key::L) && ctrl {
                self.focus_visible_link(true);
                return KeyDisposition::Consumed;
            }
            // Ctrl+Enter: activate the link at the caret.
            if input.key_pressed(egui::Key::Enter) && ctrl {
                self.activate_link_at_caret();
                return KeyDisposition::Consumed;
            }
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

        // Escape with no find bar releases the body editor to pane-level
        // navigation: the pane stays open and focused, but the editor stops
        // owning the keyboard so host pane-nav keys work. egui surrenders the
        // editor's egui focus on this same Escape press (its EventFilter allows
        // escape), which flips the dispatch text-surface gate off and routes
        // the Escape here (stint 0460 delivery) rather than to the render pass.
        // Returning `Consumed` suppresses the AppActive CloseApp binding so the
        // pane is not closed. A click re-enters the editor. (An L1 declarative
        // `TextInput` is a different surface with its own Escape-to-leave-field
        // behavior; this only fires for the focused body editor.)
        if !self.input_released && input.key_pressed(egui::Key::Escape) {
            log::info!("notes_editor: Escape — releasing editor to pane-level navigation");
            self.input_released = true;
            return KeyDisposition::Consumed;
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
        self.host_pane_id = Some(ctx.pane_id);

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
        // While released to pane-level navigation (Escape), it deliberately
        // does NOT register — so the reconciler surrenders its focus and host
        // pane-nav keys reach `poll_actions` instead of the editor.
        if !self.input_released {
            crate::ui::focus::register_default_text_surface(
                ui.ctx(),
                crate::ui::focus::SurfaceKey::Pane(ctx.pane_id),
                te_id,
            );
        }

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
            if self.mode.is_markdown()
                && ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::G))
            {
                self.toggle_preview_mode();
            }
            if self.mode.is_markdown() {
                if ui.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::K)) {
                    self.create_or_edit_link();
                }
                if ui.input_mut(|i| {
                    i.consume_key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::L)
                }) {
                    self.focus_visible_link(false);
                } else if ui.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::L)) {
                    self.focus_visible_link(true);
                }
                if ui.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::Enter)) {
                    self.activate_link_at_caret();
                }
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
            crate::ui::embedded_bar::BAR_TOTAL_H
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
        let mut widget = EditorWidget::new(&mut self.doc, &mut self.view)
            .id(te_id)
            .active(editor_focused && !self.input_released)
            .font_size(self.font_size)
            .mode(self.mode.clone())
            .highlights(
                highlights,
                current_highlight,
                colors.warning.gamma_multiply(0.45),
                colors.accent.gamma_multiply(0.55),
            );
        if self.mode.is_code() {
            // Syntect styles map onto host design tokens — never a syntect
            // theme's raw colors.
            widget = widget.code_theme(CodeTheme {
                gutter_text: colors.text_dim,
                current_line_bg: colors.bg_hover.gamma_multiply(0.55),
                keyword: colors.accent,
                string: colors.success,
                comment: colors.text_dim,
                number: colors.warning,
                ty: colors.warning,
                function: colors.text_primary,
                punctuation: colors.text_section,
            });
            if let Some(highlighter) = &mut self.highlighter {
                widget = widget.span_provider(highlighter);
            }
        }
        if self.mode.is_markdown() {
            // Live Preview styles map onto host design tokens; styling only
            // changes color/underline within lines — inline image strips add
            // per-line extra height through the shared view layout
            // (see src/editor/preview.rs and src/editor/view.rs).
            widget = widget.markdown_theme(MarkdownTheme {
                marker: colors.text_dim,
                heading: colors.accent,
                strong: colors.warning,
                emphasis: colors.text_primary,
                code: colors.success,
                quote: colors.text_section,
                rule: colors.text_dim,
                link: colors.accent,
            });
            // Cache attached in both presentations: source mode still needs
            // link spans for modifier-click activation.
            if let Some(cache) = &mut self.md_cache {
                widget = widget.markdown_preview(cache);
            }
            if self.mode.is_live_preview() {
                let base = self
                    .path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .to_path_buf();
                widget = widget.images(&mut self.image_cache, base);
            }
        }
        let output = widget.show(&mut editor_ui);
        let response = output.response;
        ui.advance_cursor_after_rect(editor_rect);

        if let Some(link) = output.link_activation {
            self.activate_link(&link);
        }

        // Clicking the editor re-enters it: clears any Escape release and, when
        // the find input (or nothing) holds focus, claims the editor surface
        // back (the reconciler grants it post-frame). The claim must register
        // this frame, so re-register the default surface too if it was skipped
        // above while released.
        if response.clicked() {
            if self.input_released {
                log::info!("notes_editor: click — re-entering editor from pane-level navigation");
                self.input_released = false;
                crate::ui::focus::register_default_text_surface(
                    ui.ctx(),
                    crate::ui::focus::SurfaceKey::Pane(ctx.pane_id),
                    te_id,
                );
            }
            if !editor_focused {
                crate::ui::focus::claim_text_surface(
                    ui.ctx(),
                    crate::ui::focus::SurfaceKey::Pane(ctx.pane_id),
                    te_id,
                );
            }
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

        // Render the find/replace bar below the editor. The embedded-bar
        // primitive owns the band geometry and safe insets so the controls
        // never clip against the pane's bottom edge.
        if self.find_bar.is_some() {
            let find_rect = egui::Rect::from_min_size(
                ui.cursor().min,
                egui::vec2(ui.available_width(), crate::ui::embedded_bar::BAR_TOTAL_H),
            );
            ui.advance_cursor_after_rect(find_rect);

            let mut replace_one = false;
            let mut replace_every = false;
            crate::ui::embedded_bar::embedded_bottom_bar(
                ui,
                find_rect,
                colors.pane_header_bg(),
                |ui| {
                    let Some(bar) = &mut self.find_bar else {
                        return;
                    };

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

                    ui.add_space(crate::ui::style::SPACE_SM);
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

                    ui.add_space(crate::ui::style::SPACE_SM);
                    ui.add(
                        egui::TextEdit::singleline(&mut bar.replace)
                            .id(replace_input_id)
                            .desired_width(input_width)
                            .font(egui::FontId::proportional(FIND_BAR_FONT_SIZE))
                            .hint_text("Replace…")
                            .frame(egui::Frame::NONE),
                    );
                    // Registered under the pane so the post-frame focus
                    // reconciler keeps a clicked replace field focused instead
                    // of snapping focus back to the editor.
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
                            log::info!(
                                "notes_editor: find query changed — {} matches",
                                bar.matches.len()
                            );
                        }
                    }
                },
            );
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
        std::mem::take(&mut self.pending_commands)
    }

    fn serialize_state(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "path": self.path.to_string_lossy(),
            "font_size": self.font_size,
        }))
    }

    fn restore_state(&mut self, state: &serde_json::Value) {
        if let Some(size) = state.get("font_size").and_then(|v| v.as_f64()) {
            self.font_size = (size as f32).clamp(FONT_SIZE_MIN, FONT_SIZE_MAX);
            log::info!("notes_editor: font_size restored -> {}", self.font_size);
        }
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
                self.mode = detect_mode(&new_path, self.is_note);
                log::info!(
                    "notes_editor: mode selected {} for {:?}",
                    self.mode.describe(),
                    new_path
                );
                self.highlighter = self.mode.language().and_then(SyntaxHighlighter::new);
                self.md_cache = self.mode.is_markdown().then(MarkdownLayoutCache::default);
                self.path = new_path;
                self.doc = Document::new(&content);
                self.view = ViewState::default();
                if self.is_note {
                    position_caret_at_end(&mut self.doc, &mut self.view);
                }
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
        let line_count = buffer.line_count();
        // Markdown mode reports the caret's parsed block (Live Preview
        // granularity); other modes fall back to the caret's source line.
        let (block_start_line, block_end_line, block_kind) = if self.mode.is_markdown() {
            let layout = preview::parse_markdown_layout(&sem.text);
            match layout.block_at_line(cursor.line) {
                Some(block) => (
                    block.lines.start,
                    block.lines.end,
                    format!("{:?}", block.kind),
                ),
                None => (cursor.line, cursor.line + 1, "Blank".to_string()),
            }
        } else {
            (cursor.line, cursor.line + 1, "SourceLine".to_string())
        };
        let line_start_char = buffer.line_to_char(block_start_line.min(line_count - 1));
        let block_end_char = if block_end_line < line_count {
            buffer.line_to_char(block_end_line)
        } else {
            buffer.len()
        };
        let active = buffer
            .slice(line_start_char, block_end_char)
            .trim_end_matches('\n')
            .to_string();
        // Report the range that slices back to exactly `active`: the trailing
        // block-separator newline(s) trimmed above are excluded from `end`.
        let line_end_char = line_start_char + active.chars().count();
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
        let link_details = preview::link_targets(&visible_source);
        let image_details = preview::image_spans(&visible_source);
        let links: Vec<String> = link_details.iter().map(|l| l.dest.clone()).collect();
        let images: Vec<String> = image_details.iter().map(|i| i.dest.clone()).collect();
        let visible_links: Vec<serde_json::Value> = link_details
            .iter()
            .map(|l| {
                serde_json::json!({
                    "kind": match l.kind {
                        LinkKind::Markdown => "markdown",
                        LinkKind::Autolink => "autolink",
                        LinkKind::Wiki => "wiki",
                    },
                    "target": l.dest,
                    "display": l.display,
                    "start": l.bytes.start,
                    "end": l.bytes.end,
                })
            })
            .collect();
        let visible_image_details: Vec<serde_json::Value> = image_details
            .iter()
            .map(|i| {
                serde_json::json!({
                    "target": i.dest,
                    "alt": i.alt,
                    "start": i.bytes.start,
                    "end": i.bytes.end,
                })
            })
            .collect();
        Some(serde_json::json!({
            "kind": "notes_editor",
            "source_text": sem.text,
            "primary_selection": {"anchor": anchor_char, "caret": caret_char},
            "caret": caret_char,
            "scroll": {"y": sem.scroll_y},
            "dirty": self.last_edit.is_some(),
            "last_save_result": self.last_save_result,
            "active_markdown_block": {
                "start": line_start_char,
                "end": line_end_char,
                "source": active,
                "kind": block_kind,
                "granularity": if self.mode.is_markdown() { "block" } else { "source_line" },
            },
            "editor_mode": self.mode.describe(),
            "preview_mode": match &self.mode {
                EditorMode::Markdown { live_preview: true } => Some("live_preview"),
                EditorMode::Markdown { live_preview: false } => Some("source"),
                _ => None,
            },
            "visible_link_targets": links,
            "visible_links": visible_links,
            "visible_images": images,
            "visible_image_details": visible_image_details,
            "last_link_activation": self.last_link_activation,
            "undo_available": sem.can_undo,
            "redo_available": sem.can_redo,
            "focused": self.editor_focused,
            "input_released": self.input_released,
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
        let result = self.ingest_drop(path_or_url, source_kind);
        match &result {
            Ok(accepted) => {
                log::info!(
                    "notes_editor: drop ingest source_kind={source_kind} outcome=accepted \
                     detail={accepted}"
                );
                self.last_drop_result = Some(accepted.clone());
            }
            Err(reason) => {
                log::info!(
                    "notes_editor: drop ingest source_kind={source_kind} outcome=rejected \
                     reason={reason}"
                );
                self.last_drop_result = Some(serde_json::json!({
                    "result": "rejected",
                    "source_kind": source_kind,
                    "reason": reason,
                }));
            }
        }
        result
    }
}

/// Stable 64-bit FNV-1a content hash: deterministic across builds/releases,
/// so asset names derived from it stay stable forever.
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Lexical relative path from `from_dir` to `target` (both absolute,
/// lexically normalized), as a `/`-separated Markdown-ready string.
fn relative_reference(from_dir: &Path, target: &Path) -> String {
    let target = normalize_lexically(target);
    let from: Vec<_> = from_dir.components().collect();
    let to: Vec<_> = target.components().collect();
    let common = from
        .iter()
        .zip(to.iter())
        .take_while(|(a, b)| a == b)
        .count();
    let mut parts: Vec<String> = vec!["..".to_string(); from.len() - common];
    parts.extend(
        to[common..]
            .iter()
            .map(|c| c.as_os_str().to_string_lossy().into_owned()),
    );
    parts.join("/")
}

/// A safe lowercase asset-name stem from the dropped file's name.
fn asset_stem(source: &Path) -> String {
    let stem = source
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("image");
    let slug = slugify_title(stem);
    if slug == "note" && !stem.eq_ignore_ascii_case("note") {
        "image".to_string()
    } else {
        slug
    }
}

impl TextEditorApp {
    /// 0478 drop ingest: local decodable images copy into
    /// `<note-parent>/assets/` under a content-hash stable name and insert a
    /// relative reference; http(s) image URLs insert a remote reference
    /// without downloading. Every rejection returns a reason without
    /// mutating the note.
    fn ingest_drop(
        &mut self,
        path_or_url: &str,
        source_kind: &str,
    ) -> Result<serde_json::Value, String> {
        if !self.mode.is_markdown() {
            return Err("image drops are only supported in Markdown documents".to_string());
        }
        if source_kind == "url" {
            let url = url::Url::parse(path_or_url)
                .map_err(|e| format!("invalid URL {path_or_url:?}: {e}"))?;
            if let Ok(path) = url.to_file_path() {
                return self.ingest_local_image(&path);
            }
            if !matches!(url.scheme(), "http" | "https") {
                return Err(format!("unsupported URL scheme {:?}", url.scheme()));
            }
            let markdown = format!("![]({path_or_url})");
            self.insert_isolated(markdown.clone());
            return Ok(serde_json::json!({
                "result": "accepted",
                "source_kind": "url",
                "markdown": markdown,
            }));
        }
        self.ingest_local_image(Path::new(path_or_url))
    }

    fn ingest_local_image(&mut self, source: &Path) -> Result<serde_json::Value, String> {
        let bytes = std::fs::read(source)
            .map_err(|e| format!("failed to read dropped file {}: {e}", source.display()))?;
        // Validate by decoding the content — never by extension.
        image::load_from_memory(&bytes)
            .map_err(|e| format!("not a decodable image ({}): {e}", source.display()))?;
        let format = image::guess_format(&bytes)
            .map_err(|e| format!("unrecognized image container ({}): {e}", source.display()))?;
        let ext = format.extensions_str().first().copied().unwrap_or("img");

        let parent = self
            .path
            .parent()
            .ok_or_else(|| format!("note {} has no parent directory", self.path.display()))?;
        // Notes in the collection share one collection-level `notes/assets/`
        // directory (contract: docs/notes-editor.md) so identical content
        // dedupes across note folders; arbitrary Markdown files outside the
        // collection keep attachments next to themselves.
        let notes_dir = crate::config::config_dir().join("notes");
        let assets_dir = if self.path.starts_with(&notes_dir) {
            notes_dir.join("assets")
        } else {
            parent.join("assets")
        };
        std::fs::create_dir_all(&assets_dir)
            .map_err(|e| format!("failed to create {}: {e}", assets_dir.display()))?;

        let hash = fnv1a64(&bytes);
        let stem = asset_stem(source);
        let mut name = format!("{stem}-{hash:016x}.{ext}");
        let mut deduped = false;
        let mut counter = 2usize;
        loop {
            let candidate = assets_dir.join(&name);
            if !candidate.exists() {
                std::fs::write(&candidate, &bytes)
                    .map_err(|e| format!("failed to save {}: {e}", candidate.display()))?;
                break;
            }
            match std::fs::read(&candidate) {
                Ok(existing) if existing == bytes => {
                    deduped = true; // identical content already stored
                    break;
                }
                Ok(_) => {
                    name = format!("{stem}-{hash:016x}-{counter}.{ext}");
                    counter += 1;
                }
                Err(e) => {
                    return Err(format!(
                        "failed to inspect existing asset {}: {e}",
                        candidate.display()
                    ));
                }
            }
        }

        // Reference relative to the note's own directory so standard
        // Markdown resolution finds the asset (`assets/…` next to the file,
        // `../assets/…` from a collection subfolder).
        let reference = relative_reference(&normalize_lexically(parent), &assets_dir.join(&name));
        let markdown = format!("![]({reference})");
        self.insert_isolated(markdown.clone());
        log::info!(
            "notes_editor: drop saved asset {reference} ({} bytes, hash={hash:016x}, deduped={deduped})",
            bytes.len()
        );
        Ok(serde_json::json!({
            "result": "accepted",
            "source_kind": "file",
            "asset": reference,
            "markdown": markdown,
            "deduped": deduped,
        }))
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
        // Markdown block granularity: the two lines are one paragraph block.
        assert_eq!(state["active_markdown_block"]["source"], "😀 café\nsecond");
        assert_eq!(state["active_markdown_block"]["kind"], "Paragraph");
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
    fn detect_mode_routes_notes_markdown_code_and_plain() {
        let p = |s: &str| PathBuf::from(s);
        assert_eq!(
            detect_mode(&p("/x/scratch.txt"), true),
            EditorMode::Markdown { live_preview: true }
        );
        assert_eq!(
            detect_mode(&p("/x/readme.MD"), false),
            EditorMode::Markdown { live_preview: true }
        );
        assert_eq!(
            detect_mode(&p("/x/main.rs"), false),
            EditorMode::Code {
                language: "rs".into()
            }
        );
        assert_eq!(
            detect_mode(&p("/x/script.PY"), false),
            EditorMode::Code {
                language: "py".into()
            }
        );
        assert_eq!(detect_mode(&p("/x/notes.txt"), false), EditorMode::PlainText);
        assert_eq!(detect_mode(&p("/x/no-extension"), false), EditorMode::PlainText);
        assert_eq!(detect_mode(&p("/x/data.xyz"), false), EditorMode::PlainText);
    }

    #[test]
    fn is_text_editable_ext_claims_prose_and_code_but_not_binary() {
        for ext in ["md", "MD", "markdown", "txt", "rs", "py", "TOML", "json", "log", "csv"] {
            assert!(is_text_editable_ext(ext), "{ext} should be text-editable");
        }
        for ext in ["png", "jpg", "mp4", "pdf", "wasm", "zip", ""] {
            assert!(!is_text_editable_ext(ext), "{ext} should not be text-editable");
        }
    }

    #[test]
    fn code_mode_file_saves_source_text_verbatim() {
        let dir = unique_temp_dir("code-mode-save");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("main.rs");
        std::fs::write(&path, "fn main() {}\n").unwrap();
        let mut app = TextEditorApp::new(path.clone());
        assert!(app.mode.is_code());
        assert!(app.highlighter.is_some(), "rust syntax is bundled");
        app.doc.apply(EditorCommand::SetCursor(Cursor::new(0, 0)));
        app.doc
            .apply(EditorCommand::InsertText("// unicode: café 😀\n".into()));
        app.last_edit = Some(Instant::now());
        app.flush();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "// unicode: café 😀\nfn main() {}\n",
            "code mode never rewrites source text"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn preview_toggle_preserves_caret_selection_scroll_and_history() {
        let dir = unique_temp_dir("notes-preview-toggle");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("preview.md");
        std::fs::write(&path, "# Title\n\npara **bold** text\n\n- item").unwrap();
        let mut app = TextEditorApp::new_for_test_note(path);
        assert!(app.mode.is_live_preview(), "markdown defaults to live preview");
        assert!(app.md_cache.is_some());

        app.doc.apply(EditorCommand::SetCursor(Cursor::new(2, 1)));
        app.doc.apply(EditorCommand::ExtendTo(Cursor::new(2, 4)));
        app.view.scroll_y = 12.5;
        let before = app.doc.semantic_state(app.view.scroll_y);

        app.toggle_preview_mode();
        assert_eq!(app.mode, EditorMode::Markdown { live_preview: false });
        assert_eq!(app.doc.semantic_state(app.view.scroll_y), before);
        let state = crate::app::app_trait::App::semantic_state(&app).unwrap();
        assert_eq!(state["preview_mode"], "source");
        assert_eq!(state["editor_mode"], "markdown:source");

        app.toggle_preview_mode();
        assert_eq!(app.doc.semantic_state(app.view.scroll_y), before);
        let state = crate::app::app_trait::App::semantic_state(&app).unwrap();
        assert_eq!(state["preview_mode"], "live_preview");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn semantic_active_block_tracks_caret_across_blocks_without_edits() {
        let dir = unique_temp_dir("notes-active-block");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("blocks.md");
        std::fs::write(&path, "# Head\n\n- one\n- two\n\n```\ncode\n```").unwrap();
        let mut app = TextEditorApp::new_for_test_note(path);
        let revision = app.doc.revision();

        let block_at = |app: &TextEditorApp| {
            let s = crate::app::app_trait::App::semantic_state(app).unwrap();
            (
                s["active_markdown_block"]["kind"].as_str().unwrap().to_string(),
                s["active_markdown_block"]["source"].as_str().unwrap().to_string(),
            )
        };
        app.doc.apply(EditorCommand::SetCursor(Cursor::new(0, 2)));
        assert_eq!(block_at(&app), ("Heading(1)".into(), "# Head".into()));
        app.doc.apply(EditorCommand::SetCursor(Cursor::new(3, 1)));
        assert_eq!(block_at(&app), ("ListItem".into(), "- two".into()));
        app.doc.apply(EditorCommand::SetCursor(Cursor::new(6, 0)));
        let (kind, source) = block_at(&app);
        assert_eq!(kind, "CodeFence");
        assert_eq!(source, "```\ncode\n```");

        // Active-block transitions are pure reads: no text, selection-history,
        // or undo mutation.
        assert_eq!(app.doc.revision(), revision);
        assert!(!app.doc.semantic_state(0.0).can_undo);
        let _ = std::fs::remove_dir_all(dir);
    }

    fn fixture_png() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("notes-drop.png")
    }

    #[test]
    fn drop_file_copies_image_to_assets_and_inserts_one_undo_step() {
        let dir = unique_temp_dir("notes-drop-image");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("note.md");
        std::fs::write(&path, "seed\n").unwrap();
        let mut app = TextEditorApp::new_for_test_note(path);
        app.doc.apply(EditorCommand::Move {
            movement: crate::editor::commands::Movement::DocEnd,
            extend: false,
        });

        let result = crate::app::app_trait::App::drop_file(
            &mut app,
            &fixture_png().to_string_lossy(),
        )
        .expect("decodable image drop accepted");
        assert_eq!(result["result"], "accepted");
        let asset = result["asset"].as_str().unwrap().to_string();
        assert!(asset.starts_with("assets/notes-drop-"), "{asset}");
        let asset_path = dir.join(&asset);
        assert!(asset_path.is_file(), "asset copied to {asset_path:?}");
        assert_eq!(
            std::fs::read(&asset_path).unwrap(),
            std::fs::read(fixture_png()).unwrap()
        );
        assert_eq!(app.doc.text(), format!("seed\n![]({asset})"));
        assert!(app.last_edit.is_some(), "drop dirties the note");

        // Exactly one undo step removes the reference; the asset survives.
        app.doc.apply(EditorCommand::Undo);
        assert_eq!(app.doc.text(), "seed\n");
        assert!(asset_path.is_file(), "undo never deletes the copied asset");
        app.last_edit = None;
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn drop_into_collection_note_uses_collection_assets_dir() {
        let profile = unique_temp_dir("notes-drop-collection");
        std::fs::create_dir_all(&profile).unwrap();
        let _guard = crate::config::set_test_profile_dir(profile.clone());
        let notes_dir = crate::config::config_dir().join("notes");
        std::fs::create_dir_all(notes_dir.join("inbox")).unwrap();
        let path = notes_dir.join("inbox").join("current.md");
        std::fs::write(&path, "").unwrap();
        let mut app = TextEditorApp::new_for_test_note(path);

        let result = crate::app::app_trait::App::drop_file(
            &mut app,
            &fixture_png().to_string_lossy(),
        )
        .expect("collection drop accepted");
        let asset = result["asset"].as_str().unwrap();
        // Collection notes share one notes/assets/ dir; the reference is
        // relative to the note's own folder.
        assert!(asset.starts_with("../assets/notes-drop-"), "{asset}");
        let stored = notes_dir.join("assets").join(asset.rsplit('/').next().unwrap());
        assert!(stored.is_file(), "asset stored at {stored:?}");
        assert!(!notes_dir.join("inbox").join("assets").exists());
        app.last_edit = None;
        let _ = std::fs::remove_dir_all(profile);
    }

    #[test]
    fn drop_file_dedupes_identical_content() {
        let dir = unique_temp_dir("notes-drop-dedupe");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("note.md");
        std::fs::write(&path, "").unwrap();
        let mut app = TextEditorApp::new_for_test_note(path);
        let source = fixture_png().to_string_lossy().into_owned();

        let first = crate::app::app_trait::App::drop_file(&mut app, &source).unwrap();
        let second = crate::app::app_trait::App::drop_file(&mut app, &source).unwrap();
        assert_eq!(first["asset"], second["asset"]);
        assert_eq!(second["deduped"], true);
        let assets: Vec<_> = std::fs::read_dir(dir.join("assets"))
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        assert_eq!(assets.len(), 1, "identical content stored once");
        app.last_edit = None;
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn drop_file_rejects_undecodable_content_without_mutating_note() {
        let dir = unique_temp_dir("notes-drop-reject");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("note.md");
        std::fs::write(&path, "body\n").unwrap();
        // A .png extension with non-image bytes must still be rejected:
        // validation is by decodable content, never extension.
        let fake = dir.join("fake.png");
        std::fs::write(&fake, b"not an image at all").unwrap();
        let mut app = TextEditorApp::new_for_test_note(path);

        let err = crate::app::app_trait::App::drop_file(&mut app, &fake.to_string_lossy())
            .expect_err("undecodable content rejected");
        assert!(err.contains("not a decodable image"), "{err}");
        assert_eq!(app.doc.text(), "body\n", "note unchanged");
        assert!(!dir.join("assets").exists(), "no asset dir created");
        assert_eq!(app.last_drop_result.as_ref().unwrap()["result"], "rejected");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn drop_file_inserts_remote_image_url_without_downloading() {
        let dir = unique_temp_dir("notes-drop-url");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("note.md");
        std::fs::write(&path, "").unwrap();
        let mut app = TextEditorApp::new_for_test_note(path);

        let url = "https://example.com/pic.png";
        let result = crate::app::app_trait::App::drop_file(&mut app, url).unwrap();
        assert_eq!(result["source_kind"], "url");
        assert_eq!(app.doc.text(), format!("![]({url})"));
        assert!(!dir.join("assets").exists(), "no download, no asset");

        // Non-http(s) schemes are rejected.
        let err = crate::app::app_trait::App::drop_file(&mut app, "ftp://example.com/x.png")
            .expect_err("non-http scheme rejected");
        assert!(err.contains("unsupported URL scheme"), "{err}");
        app.last_edit = None;
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn create_or_edit_link_wraps_selection_in_one_undo_step() {
        let dir = unique_temp_dir("notes-link-create");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("note.md");
        std::fs::write(&path, "visit plexi today").unwrap();
        let mut app = TextEditorApp::new_for_test_note(path);
        app.doc.apply(EditorCommand::SetCursor(Cursor::new(0, 6)));
        app.doc.apply(EditorCommand::ExtendTo(Cursor::new(0, 11)));

        app.create_or_edit_link();
        assert_eq!(app.doc.text(), "visit [plexi](url) today");
        // The `url` placeholder is selected for immediate typing.
        assert_eq!(app.doc.selected_text(), "url");
        app.doc.apply(EditorCommand::Undo);
        assert_eq!(app.doc.text(), "visit plexi today", "one undo step");

        // Ctrl+K inside an existing link selects its destination (no edit).
        app.doc.apply(EditorCommand::Redo);
        app.doc.apply(EditorCommand::SetCursor(Cursor::new(0, 8)));
        let revision = app.doc.revision();
        app.create_or_edit_link();
        assert_eq!(app.doc.selected_text(), "url");
        assert_eq!(app.doc.revision(), revision, "edit mode is selection-only");
        app.last_edit = None;
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn focus_visible_link_walks_and_wraps() {
        let dir = unique_temp_dir("notes-link-focus");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("note.md");
        std::fs::write(&path, "[A](http://a.test) mid [[wiki b]] end").unwrap();
        let mut app = TextEditorApp::new_for_test_note(path);
        app.doc.apply(EditorCommand::SetCursor(Cursor::new(0, 0)));

        app.focus_visible_link(true);
        assert_eq!(app.doc.selected_text(), "[A](http://a.test)");
        app.focus_visible_link(true);
        assert_eq!(app.doc.selected_text(), "[[wiki b]]");
        app.focus_visible_link(true); // wraps
        assert_eq!(app.doc.selected_text(), "[A](http://a.test)");
        app.focus_visible_link(false); // wraps backward
        assert_eq!(app.doc.selected_text(), "[[wiki b]]");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn activate_link_rejects_non_http_schemes() {
        let dir = unique_temp_dir("notes-link-scheme");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("note.md");
        std::fs::write(&path, "x").unwrap();
        let mut app = TextEditorApp::new_for_test_note(path);
        let link = preview::LinkTarget {
            kind: LinkKind::Markdown,
            bytes: 0..1,
            dest: "javascript://alert(1)".to_string(),
            display: "x".to_string(),
        };
        app.activate_link(&link);
        let activation = app.last_link_activation.clone().unwrap();
        assert_eq!(activation["outcome"], "open_failed");
        assert!(activation["detail"]
            .as_str()
            .unwrap()
            .contains("non-http(s)"));
        assert!(app.pending_commands.is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn internal_link_resolution_is_deterministic_and_never_creates_files() {
        let dir = unique_temp_dir("notes-link-internal");
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("other.md");
        std::fs::write(&target, "target note").unwrap();
        let path = dir.join("note.md");
        std::fs::write(&path, "[other](other.md) [gone](missing.md)").unwrap();
        let mut app = TextEditorApp::new_for_test_note(path);

        // Existing relative target opens via a queued SpawnPane command.
        let link = preview::link_targets(&app.doc.text())[0].clone();
        app.activate_link(&link);
        assert_eq!(app.last_link_activation.as_ref().unwrap()["outcome"], "opened_note");
        let commands = crate::app::app_trait::App::take_pending_commands(&mut app);
        assert_eq!(commands.len(), 1);
        match &commands[0] {
            AppCommand::SpawnPane { type_id, args, .. } => {
                assert_eq!(type_id, "text-editor");
                assert!(args[0].ends_with("other.md"));
            }
            _ => panic!("expected SpawnPane command"),
        }
        assert!(
            crate::app::app_trait::App::take_pending_commands(&mut app).is_empty(),
            "commands drain once"
        );

        // Missing target: deterministic missing outcome, no file created.
        let link = preview::link_targets(&app.doc.text())[1].clone();
        app.activate_link(&link);
        assert_eq!(app.last_link_activation.as_ref().unwrap()["outcome"], "missing");
        assert!(!dir.join("missing.md").exists(), "never silently creates a file");
        assert!(crate::app::app_trait::App::take_pending_commands(&mut app).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn wiki_link_resolution_creates_missing_and_opens_unique_match() {
        let profile = unique_temp_dir("notes-wiki-profile");
        std::fs::create_dir_all(&profile).unwrap();
        let _guard = crate::config::set_test_profile_dir(profile.clone());
        let notes_dir = crate::config::config_dir().join("notes");
        std::fs::create_dir_all(notes_dir.join("inbox")).unwrap();
        std::fs::write(notes_dir.join("inbox").join("trip-ideas.md"), "packing").unwrap();

        let note_path = notes_dir.join("current.md");
        std::fs::write(&note_path, "[[trip-ideas]] and [[nowhere]]").unwrap();
        let mut app = TextEditorApp::new_for_test_note(note_path);

        let links = preview::link_targets(&app.doc.text());
        app.activate_link(&links[0]);
        assert_eq!(app.last_link_activation.as_ref().unwrap()["outcome"], "opened_note");
        let commands = crate::app::app_trait::App::take_pending_commands(&mut app);
        assert_eq!(commands.len(), 1);

        // Missing wiki target: created as a blank note under the notes dir and
        // opened (standard wiki behavior, stint 0506).
        app.activate_link(&links[1]);
        assert_eq!(app.last_link_activation.as_ref().unwrap()["outcome"], "created_note");
        let created = notes_dir.join("nowhere.md");
        assert!(created.exists(), "missing wiki target must be created");
        assert_eq!(std::fs::read_to_string(&created).unwrap(), "");
        let commands = crate::app::app_trait::App::take_pending_commands(&mut app);
        assert_eq!(commands.len(), 1, "creation opens the new note in a pane");
        match &commands[0] {
            AppCommand::SpawnPane { type_id, args, .. } => {
                assert_eq!(type_id, "text-editor");
                assert!(args[0].ends_with("nowhere.md"));
            }
            _ => panic!("expected SpawnPane command"),
        }
        let _ = std::fs::remove_dir_all(profile);
    }

    #[test]
    fn wiki_link_nested_existing_target_opens_without_truncating() {
        let profile = unique_temp_dir("notes-wiki-nested");
        std::fs::create_dir_all(&profile).unwrap();
        let _guard = crate::config::set_test_profile_dir(profile.clone());
        let notes_dir = crate::config::config_dir().join("notes");
        std::fs::create_dir_all(notes_dir.join("project")).unwrap();
        let nested = notes_dir.join("project").join("idea.md");
        std::fs::write(&nested, "precious contents").unwrap();

        let note_path = notes_dir.join("current.md");
        std::fs::write(&note_path, "[[project/idea]]").unwrap();
        let mut app = TextEditorApp::new_for_test_note(note_path);

        let links = preview::link_targets(&app.doc.text());
        app.activate_link(&links[0]);
        assert_eq!(app.last_link_activation.as_ref().unwrap()["outcome"], "opened_note");
        assert_eq!(
            std::fs::read_to_string(&nested).unwrap(),
            "precious contents",
            "an existing nested target must never be truncated"
        );
        let _ = std::fs::remove_dir_all(profile);
    }

    #[test]
    fn wiki_link_creates_when_notes_root_absent() {
        let profile = unique_temp_dir("notes-wiki-fresh");
        std::fs::create_dir_all(&profile).unwrap();
        let _guard = crate::config::set_test_profile_dir(profile.clone());
        let notes_dir = crate::config::config_dir().join("notes");
        // Deliberately do NOT create notes_dir — a fresh profile.
        assert!(!notes_dir.exists());
        // The source note lives elsewhere; only the wiki resolution matters.
        let note_path = profile.join("current.md");
        std::fs::write(&note_path, "[[first-note]]").unwrap();
        let mut app = TextEditorApp::new(note_path);
        // Force note-mode resolution against the (absent) notes root.
        app.is_note = true;

        let (outcome, _) = app.resolve_wiki_link("first-note");
        assert_eq!(outcome, "created_note");
        assert!(notes_dir.join("first-note.md").exists());
        let _ = std::fs::remove_dir_all(profile);
    }

    #[test]
    fn wiki_link_create_refuses_paths_escaping_notes_dir() {
        let profile = unique_temp_dir("notes-wiki-escape");
        std::fs::create_dir_all(&profile).unwrap();
        let _guard = crate::config::set_test_profile_dir(profile.clone());
        let notes_dir = crate::config::config_dir().join("notes");
        std::fs::create_dir_all(&notes_dir).unwrap();
        let note_path = notes_dir.join("current.md");
        std::fs::write(&note_path, "[[../escape]]").unwrap();
        let mut app = TextEditorApp::new_for_test_note(note_path);

        let links = preview::link_targets(&app.doc.text());
        app.activate_link(&links[0]);
        assert_eq!(
            app.last_link_activation.as_ref().unwrap()["outcome"],
            "create_failed"
        );
        assert!(!notes_dir.parent().unwrap().join("escape.md").exists());
        assert!(crate::app::app_trait::App::take_pending_commands(&mut app).is_empty());
        let _ = std::fs::remove_dir_all(profile);
    }

    #[test]
    fn semantic_state_reports_link_and_image_targets_from_preview_parser() {
        let dir = unique_temp_dir("notes-semantic-links");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("note.md");
        // Everything on the first line: with a zero-height test viewport only
        // the first visible line window is reported.
        std::fs::write(
            &path,
            "[Plexi](https://plexiapp.com) [[wiki page]] ![alt](assets/p.png)",
        )
        .unwrap();
        let app = TextEditorApp::new_for_test_note(path);
        let state = crate::app::app_trait::App::semantic_state(&app).unwrap();
        let targets: Vec<&str> = state["visible_link_targets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(targets, vec!["https://plexiapp.com", "wiki page"]);
        assert_eq!(state["visible_links"][0]["kind"], "markdown");
        assert_eq!(state["visible_links"][1]["kind"], "wiki");
        assert_eq!(state["visible_images"][0], "assets/p.png");
        assert_eq!(state["visible_image_details"][0]["alt"], "alt");
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

    #[test]
    fn font_size_survives_serialize_restore_cycle() {
        let dir = unique_temp_dir("notes-font-size-persist");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("note.md");
        std::fs::write(&path, "hello").unwrap();

        let mut app = TextEditorApp::new(path.clone());
        app.adjust_font_size(FONT_SIZE_MAX);
        assert_eq!(app.font_size, FONT_SIZE_MAX);
        let state = crate::app::app_trait::App::serialize_state(&app).unwrap();
        assert_eq!(state["font_size"], FONT_SIZE_MAX as f64);

        let mut restored = TextEditorApp::new(path.clone());
        assert_eq!(restored.font_size, FONT_SIZE_DEFAULT);
        crate::app::app_trait::App::restore_state(&mut restored, &state);
        assert_eq!(restored.font_size, FONT_SIZE_MAX);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn font_size_restore_clamps_out_of_range_value() {
        let dir = unique_temp_dir("notes-font-size-clamp");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("note.md");
        std::fs::write(&path, "hello").unwrap();

        let mut app = TextEditorApp::new(path);
        let state = serde_json::json!({ "font_size": FONT_SIZE_MAX + 100.0 });
        crate::app::app_trait::App::restore_state(&mut app, &state);
        assert_eq!(app.font_size, FONT_SIZE_MAX);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn font_size_restore_missing_key_keeps_default() {
        let dir = unique_temp_dir("notes-font-size-missing");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("note.md");
        std::fs::write(&path, "hello").unwrap();

        let mut app = TextEditorApp::new(path.clone());
        let state = serde_json::json!({ "path": path.to_string_lossy() });
        crate::app::app_trait::App::restore_state(&mut app, &state);
        assert_eq!(app.font_size, FONT_SIZE_DEFAULT);

        let _ = std::fs::remove_dir_all(dir);
    }
}
