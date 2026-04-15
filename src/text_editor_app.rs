use crate::app_trait::{App, AppCommand, AppRenderContext};
use crate::theme::Colors;
use egui::text::{LayoutJob, TextFormat};
use egui::FontId;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use self::autosave::Autosave;
use self::highlight::{detect_language, HighlightCache, SyntaxKind};
use self::search::{FindState, FindUiMode};
use self::undo::UndoStack;

/// Maximum file size for syntax highlighting. Above this we fall back to plain.
const MAX_HIGHLIGHT_BYTES: usize = 1_000_000;
/// Max number of undo entries retained.
const UNDO_CAP: usize = 200;
/// Auto-save cadence for unnamed scratch files.
const AUTOSAVE_INTERVAL: Duration = Duration::from_secs(30);
/// Auto-save after this many keystrokes for unnamed scratch files.
const AUTOSAVE_KEY_THRESHOLD: u32 = 100;
/// How long a transient status message remains visible.
const STATUS_MSG_TTL: Duration = Duration::from_secs(3);

/// Built-in Plexi text editor with find/replace, syntax highlighting, undo,
/// line numbers, goto line, save-as, and auto-save scratch files.
pub struct TextEditorApp {
    file_path: Option<PathBuf>,
    content: String,
    saved_content: String,
    edit_mode: bool,
    dirty: bool,
    was_focused: bool,
    cursor_init_pending: bool,
    pending_cmds: Vec<AppCommand>,

    // Syntax / rendering
    highlight_cache: HighlightCache,
    show_line_numbers: bool,
    word_wrap: bool,
    word_wrap_user_set: bool,

    // Undo
    undo: UndoStack,

    // Find & replace
    find: FindState,

    // Goto line
    goto_open: bool,
    goto_input: String,

    // Save As modal
    save_as_open: bool,
    save_as_input: String,
    save_as_overwrite_prompt: Option<PathBuf>,

    // File-changed-on-disk
    disk_mtime: Option<SystemTime>,
    disk_conflict_prompt: bool,

    // Status bar transient message
    status_msg: Option<(String, Instant)>,

    // Auto-save tracking (scratch only)
    autosave: Autosave,

    // Latest info about the last frame (cached for status bar)
    last_cursor_line_col: (usize, usize),
}

impl TextEditorApp {
    /// Construct a fresh scratch editor (no file backing yet).
    pub fn new_scratch(cwd: PathBuf) -> Self {
        let _ = cwd;
        Self::build(None, String::new(), true)
    }

    /// Load an editor from disk. On read failure logs and opens an empty buffer.
    pub fn from_file(path: PathBuf) -> Self {
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                log::error!("TextEditorApp: failed to read {}: {e}", path.display());
                String::new()
            }
        };
        let mut app = Self::build(Some(path.clone()), content, false);
        app.disk_mtime = read_mtime(&path);
        app
    }

    fn build(file_path: Option<PathBuf>, content: String, edit_mode: bool) -> Self {
        let lang = detect_language(file_path.as_deref());
        // Default wrap: ON for plain text, OFF for code.
        let word_wrap = matches!(lang, SyntaxKind::Plain | SyntaxKind::Markdown);
        Self {
            file_path,
            content: content.clone(),
            saved_content: content,
            edit_mode,
            dirty: false,
            was_focused: false,
            cursor_init_pending: true,
            pending_cmds: Vec::new(),
            highlight_cache: HighlightCache::new(),
            show_line_numbers: true,
            word_wrap,
            word_wrap_user_set: false,
            undo: UndoStack::new(UNDO_CAP),
            find: FindState::default(),
            goto_open: false,
            goto_input: String::new(),
            save_as_open: false,
            save_as_input: String::new(),
            save_as_overwrite_prompt: None,
            disk_mtime: None,
            disk_conflict_prompt: false,
            status_msg: None,
            autosave: Autosave::new(),
            last_cursor_line_col: (1, 1),
        }
    }

    /// Save to the current file path, or to a scratch file if unnamed.
    fn save(&mut self) {
        let path = match &self.file_path {
            Some(p) => p.clone(),
            None => {
                // Promote to autosave-style scratch path (manual save, not auto).
                let scratch_dir = crate::config::config_dir().join("scratch");
                if let Err(e) = std::fs::create_dir_all(&scratch_dir) {
                    log::error!("TextEditorApp: failed to create scratch dir: {e}");
                    self.set_status(format!("Save failed: {e}"));
                    return;
                }
                let ts = SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let p = scratch_dir.join(format!("scratch-{ts}.txt"));
                self.file_path = Some(p.clone());
                p
            }
        };
        self.write_to(path);
    }

    fn write_to(&mut self, path: PathBuf) {
        match std::fs::write(&path, &self.content) {
            Ok(()) => {
                self.saved_content = self.content.clone();
                self.dirty = false;
                self.disk_mtime = read_mtime(&path);
                log::info!("TextEditorApp: saved {}", path.display());
                self.set_status(format!("Saved {}", path.display()));
                self.pending_cmds
                    .push(AppCommand::Notify(format!("Saved {}", path.display())));
                self.file_path = Some(path);
                self.autosave.reset();
            }
            Err(e) => {
                log::error!("TextEditorApp: save failed for {}: {e}", path.display());
                self.set_status(format!("Save failed: {e}"));
                self.pending_cmds
                    .push(AppCommand::Notify(format!("Save failed: {e}")));
            }
        }
    }

    fn discard_changes(&mut self) {
        self.content = self.saved_content.clone();
        self.dirty = false;
        self.undo.clear();
        self.highlight_cache.invalidate();
    }

    fn set_status(&mut self, msg: impl Into<String>) {
        self.status_msg = Some((msg.into(), Instant::now()));
    }

    fn detect_and_cache_edits(&mut self) {
        // Rebuild dirty flag from content diff, track autosave keystroke counter.
        let is_dirty = self.content != self.saved_content;
        if is_dirty != self.dirty {
            self.dirty = is_dirty;
        }
    }

    fn handle_file_changed_on_disk(&mut self) {
        let Some(path) = self.file_path.clone() else {
            return;
        };
        let Some(current) = read_mtime(&path) else {
            return;
        };
        if let Some(prev) = self.disk_mtime {
            if current != prev {
                if !self.dirty {
                    // Clean buffer — silent reload.
                    match std::fs::read_to_string(&path) {
                        Ok(c) => {
                            self.content = c.clone();
                            self.saved_content = c;
                            self.disk_mtime = Some(current);
                            self.highlight_cache.invalidate();
                            self.undo.clear();
                            self.set_status("Reloaded from disk");
                            log::info!(
                                "TextEditorApp: silently reloaded {} after external change",
                                path.display()
                            );
                        }
                        Err(e) => {
                            log::error!(
                                "TextEditorApp: reload failed for {}: {e}",
                                path.display()
                            );
                        }
                    }
                } else {
                    self.disk_conflict_prompt = true;
                }
            }
        } else {
            self.disk_mtime = Some(current);
        }
    }

    fn reload_from_disk(&mut self) {
        if let Some(path) = self.file_path.clone() {
            match std::fs::read_to_string(&path) {
                Ok(c) => {
                    self.content = c.clone();
                    self.saved_content = c;
                    self.dirty = false;
                    self.disk_mtime = read_mtime(&path);
                    self.highlight_cache.invalidate();
                    self.undo.clear();
                    self.set_status("Reloaded from disk");
                }
                Err(e) => {
                    log::error!("TextEditorApp: reload failed for {}: {e}", path.display());
                    self.set_status(format!("Reload failed: {e}"));
                }
            }
        }
        self.disk_conflict_prompt = false;
    }
}

impl App for TextEditorApp {
    fn type_id(&self) -> &'static str {
        "text_editor"
    }

    fn display_name(&self) -> String {
        self.file_path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Untitled".to_string())
    }

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &AppRenderContext<'_>) {
        let colors = ctx.colors;
        let is_focused = ctx.is_focused;

        // Focus transition: check disk for external changes.
        if is_focused && !self.was_focused {
            self.handle_file_changed_on_disk();
        }

        // ─── Consume app-level shortcuts BEFORE TextEdit renders ──────────
        let shortcut = ui.input_mut(consume_editor_shortcuts);
        self.apply_shortcut(shortcut);

        // Expire transient status messages.
        if let Some((_, when)) = &self.status_msg {
            if when.elapsed() > STATUS_MSG_TTL {
                self.status_msg = None;
            }
        }

        // ─── Top bar ──────────────────────────────────────────────────────
        self.draw_top_bar(ui, colors);

        // ─── Find / replace bar ───────────────────────────────────────────
        if self.find.mode != FindUiMode::Hidden {
            self.draw_find_bar(ui, colors);
        }

        // ─── Goto-line prompt ─────────────────────────────────────────────
        if self.goto_open {
            self.draw_goto_bar(ui, colors);
        }

        // ─── Save As modal ────────────────────────────────────────────────
        if self.save_as_open {
            self.draw_save_as_bar(ui, colors);
        }

        // ─── Disk-conflict prompt ─────────────────────────────────────────
        if self.disk_conflict_prompt {
            self.draw_disk_conflict_prompt(ui, colors);
        }

        ui.add_space(2.0);
        ui.separator();

        // Reserve status bar height at the bottom.
        let status_h = 20.0;
        let editor_region = {
            let mut r = ui.available_rect_before_wrap();
            r.max.y -= status_h;
            r
        };

        // ─── Editor body ──────────────────────────────────────────────────
        let mut editor_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(editor_region)
                .layout(egui::Layout::top_down(egui::Align::LEFT)),
        );
        self.draw_editor_body(&mut editor_ui, colors, is_focused);

        // ─── Status bar ───────────────────────────────────────────────────
        let status_rect = egui::Rect::from_min_max(
            egui::pos2(ui.max_rect().left(), ui.max_rect().bottom() - status_h),
            ui.max_rect().max,
        );
        let mut status_ui = ui.new_child(egui::UiBuilder::new().max_rect(status_rect));
        self.draw_status_bar(&mut status_ui, colors);

        self.was_focused = is_focused;

        // Auto-save scratch files after edits.
        self.detect_and_cache_edits();
        if self.file_path.is_none() || self.autosave.owns(&self.file_path) {
            if let Some(msg) = self.autosave.maybe_save(
                &self.content,
                self.dirty,
                AUTOSAVE_INTERVAL,
                AUTOSAVE_KEY_THRESHOLD,
                &mut self.file_path,
                &mut self.saved_content,
            ) {
                self.dirty = false;
                self.disk_mtime = self
                    .file_path
                    .as_ref()
                    .and_then(|p| read_mtime(p));
                self.set_status(msg);
            }
        }
    }

    fn handle_key(&mut self, _input: &egui::InputState) -> bool {
        false
    }

    fn take_pending_commands(&mut self) -> Vec<AppCommand> {
        std::mem::take(&mut self.pending_cmds)
    }

    fn accepted_extensions(&self) -> &[&str] {
        &[
            "txt", "md", "markdown", "toml", "yaml", "yml", "json", "csv", "log", "conf", "cfg",
            "ini", "sh", "bash", "zsh", "fish", "env", "gitignore", "dockerignore", "editorconfig",
            "rs", "py", "js", "ts", "tsx", "jsx", "html", "css", "scss", "xml", "svg", "sql",
            "rb", "go", "c", "h", "cpp", "hpp", "java", "kt", "swift", "lua", "vim", "el",
            "lisp", "zig", "nim", "r", "jl", "ex", "exs", "erl", "hs", "ml", "mli", "nix",
            "tf", "hcl", "just", "makefile", "cmake",
        ]
    }

    fn wants_close(&self) -> bool {
        false
    }

    fn serialize_state(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "file_path": self.file_path.as_ref().map(|p| p.to_string_lossy().to_string()),
            "content": self.content,
            "edit_mode": self.edit_mode,
            "show_line_numbers": self.show_line_numbers,
            "word_wrap": self.word_wrap,
        }))
    }

    fn restore_state(&mut self, state: &serde_json::Value) {
        if let Some(path_str) = state.get("file_path").and_then(|v| v.as_str()) {
            let path = PathBuf::from(path_str);
            match std::fs::read_to_string(&path) {
                Ok(disk_content) => {
                    self.content = disk_content.clone();
                    self.saved_content = disk_content;
                    self.disk_mtime = read_mtime(&path);
                }
                Err(_) => {
                    if let Some(c) = state.get("content").and_then(|v| v.as_str()) {
                        self.content = c.to_string();
                        self.saved_content = c.to_string();
                    }
                }
            }
            self.file_path = Some(path);
        } else if let Some(c) = state.get("content").and_then(|v| v.as_str()) {
            self.content = c.to_string();
            self.saved_content = c.to_string();
        }
        if let Some(em) = state.get("edit_mode").and_then(|v| v.as_bool()) {
            self.edit_mode = em;
        }
        if let Some(ln) = state.get("show_line_numbers").and_then(|v| v.as_bool()) {
            self.show_line_numbers = ln;
        }
        if let Some(ww) = state.get("word_wrap").and_then(|v| v.as_bool()) {
            self.word_wrap = ww;
            self.word_wrap_user_set = true;
        }
        self.dirty = false;
        self.highlight_cache.invalidate();
    }
}

// ---------------------------------------------------------------------------
// Shortcut handling
// ---------------------------------------------------------------------------

/// Editor-level keyboard shortcuts, consumed before TextEdit renders.
#[derive(Clone, Copy)]
enum EditorShortcut {
    ToggleFind,
    ToggleReplace,
    FindNext,
    FindPrev,
    ToggleLineNumbers,
    ToggleWordWrap,
    GotoLine,
    SaveAs,
    Undo,
    Redo,
    ToggleCase,
}

fn consume_editor_shortcuts(input: &mut egui::InputState) -> Vec<EditorShortcut> {
    use egui::{Key, Modifiers};
    let mut out = Vec::new();
    let cmd_only = Modifiers::COMMAND;
    let cmd_shift = Modifiers {
        shift: true,
        ..Modifiers::COMMAND
    };
    let cmd_alt = Modifiers {
        alt: true,
        ..Modifiers::COMMAND
    };

    // Exact-modifier checks so combinations with Shift/Alt don't collide.
    let mods = input.modifiers;
    let cmd_bare = mods.command && !mods.shift && !mods.alt;
    let cmd_shift_only = mods.command && mods.shift && !mods.alt;
    let cmd_alt_only = mods.command && mods.alt && !mods.shift;

    if cmd_bare && input.consume_key(cmd_only, Key::F) {
        out.push(EditorShortcut::ToggleFind);
    }
    if cmd_bare && input.consume_key(cmd_only, Key::R) {
        out.push(EditorShortcut::ToggleReplace);
    }
    if cmd_bare && input.consume_key(cmd_only, Key::G) {
        out.push(EditorShortcut::FindNext);
    }
    if cmd_shift_only && input.consume_key(cmd_shift, Key::G) {
        out.push(EditorShortcut::FindPrev);
    }
    if cmd_shift_only && input.consume_key(cmd_shift, Key::L) {
        out.push(EditorShortcut::ToggleLineNumbers);
    }
    if cmd_shift_only && input.consume_key(cmd_shift, Key::W) {
        out.push(EditorShortcut::ToggleWordWrap);
    }
    if cmd_bare && input.consume_key(cmd_only, Key::Semicolon) {
        out.push(EditorShortcut::GotoLine);
    }
    if cmd_alt_only && input.consume_key(cmd_alt, Key::S) {
        out.push(EditorShortcut::SaveAs);
    }
    if cmd_bare && input.consume_key(cmd_only, Key::Z) {
        out.push(EditorShortcut::Undo);
    }
    if cmd_shift_only && input.consume_key(cmd_shift, Key::Z) {
        out.push(EditorShortcut::Redo);
    }
    if cmd_alt_only && input.consume_key(cmd_alt, Key::C) {
        out.push(EditorShortcut::ToggleCase);
    }

    // Replace-current / replace-all are handled via plain Enter inside the
    // replace field (not via a global shortcut) because Cmd+Enter collides
    // with Plexi's zoom toggle at the top-level input layer.
    out
}

impl TextEditorApp {
    fn apply_shortcut(&mut self, shortcuts: Vec<EditorShortcut>) {
        for sc in shortcuts {
            match sc {
                EditorShortcut::ToggleFind => {
                    self.find.toggle_find();
                }
                EditorShortcut::ToggleReplace => {
                    self.find.toggle_replace();
                }
                EditorShortcut::FindNext => {
                    if self.find.mode == FindUiMode::Hidden {
                        self.find.toggle_find();
                    } else {
                        self.find.advance(&self.content, 1);
                    }
                }
                EditorShortcut::FindPrev => {
                    self.find.advance(&self.content, -1);
                }
                EditorShortcut::ToggleLineNumbers => {
                    self.show_line_numbers = !self.show_line_numbers;
                }
                EditorShortcut::ToggleWordWrap => {
                    self.word_wrap = !self.word_wrap;
                    self.word_wrap_user_set = true;
                }
                EditorShortcut::GotoLine => {
                    self.goto_open = !self.goto_open;
                    if self.goto_open {
                        self.goto_input.clear();
                    }
                }
                EditorShortcut::SaveAs => {
                    self.save_as_open = true;
                    self.save_as_input = self
                        .file_path
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default();
                }
                EditorShortcut::Undo => {
                    if let Some(new_content) = self.undo.undo(&self.content) {
                        self.content = new_content;
                        self.highlight_cache.invalidate();
                        self.dirty = self.content != self.saved_content;
                    }
                }
                EditorShortcut::Redo => {
                    if let Some(new_content) = self.undo.redo(&self.content) {
                        self.content = new_content;
                        self.highlight_cache.invalidate();
                        self.dirty = self.content != self.saved_content;
                    }
                }
                EditorShortcut::ToggleCase => {
                    self.find.case_sensitive = !self.find.case_sensitive;
                    self.find.rebuild_matches(&self.content);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Top bar / find bar / goto bar / save-as bar / disk conflict
// ---------------------------------------------------------------------------

impl TextEditorApp {
    fn draw_top_bar(&mut self, ui: &mut egui::Ui, colors: &Colors) {
        ui.horizontal(|ui| {
            let mode_label = if self.edit_mode { "Edit" } else { "View" };
            if ui
                .button(
                    egui::RichText::new(mode_label)
                        .size(11.0)
                        .color(colors.accent),
                )
                .clicked()
            {
                self.edit_mode = !self.edit_mode;
            }
            if self.dirty {
                if ui
                    .button(egui::RichText::new("Save").size(11.0).color(colors.accent))
                    .clicked()
                {
                    self.save();
                }
                if ui
                    .button(
                        egui::RichText::new("Discard")
                            .size(11.0)
                            .color(colors.text_dim),
                    )
                    .clicked()
                {
                    self.discard_changes();
                }
            }
            let label = self
                .file_path
                .as_ref()
                .map(|p| {
                    let d = p.display().to_string();
                    dirs::home_dir()
                        .and_then(|h| {
                            d.strip_prefix(&h.display().to_string())
                                .map(|r| format!("~{r}"))
                        })
                        .unwrap_or(d)
                })
                .unwrap_or_else(|| "(scratch)".into());
            ui.label(egui::RichText::new(label).size(10.0).color(colors.text_dim));
        });
    }

    fn draw_find_bar(&mut self, ui: &mut egui::Ui, colors: &Colors) {
        ui.add_space(2.0);
        let frame = egui::Frame::default()
            .fill(colors.bg_sidebar)
            .inner_margin(egui::Margin::symmetric(6, 4));
        frame.show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("Find")
                        .size(10.0)
                        .color(colors.text_dim),
                );
                let find_id = ui.make_persistent_id("text_editor_find_input");
                let find_resp = ui.add(
                    egui::TextEdit::singleline(&mut self.find.query)
                        .id(find_id)
                        .font(FontId::monospace(12.0))
                        .desired_width(180.0),
                );
                if self.find.focus_request {
                    find_resp.request_focus();
                    self.find.focus_request = false;
                }
                if find_resp.changed() {
                    self.find.rebuild_matches(&self.content);
                }
                // Enter = next, Shift+Enter = prev, Esc = close (handled while focused).
                if find_resp.has_focus() {
                    let (enter, shift_enter, esc) = ui.input(|i| {
                        (
                            i.key_pressed(egui::Key::Enter) && !i.modifiers.shift,
                            i.key_pressed(egui::Key::Enter) && i.modifiers.shift,
                            i.key_pressed(egui::Key::Escape),
                        )
                    });
                    if enter {
                        self.find.advance(&self.content, 1);
                    }
                    if shift_enter {
                        self.find.advance(&self.content, -1);
                    }
                    if esc {
                        self.find.close();
                    }
                }

                let match_count = self.find.matches.len();
                let counter = if match_count == 0 {
                    "no matches".to_string()
                } else {
                    format!("{}/{}", self.find.current.saturating_add(1), match_count)
                };
                ui.label(
                    egui::RichText::new(counter)
                        .size(10.0)
                        .color(colors.text_dim),
                );

                let case_btn_color = if self.find.case_sensitive {
                    colors.accent
                } else {
                    colors.text_dim
                };
                if ui
                    .button(egui::RichText::new("Aa").size(10.0).color(case_btn_color))
                    .on_hover_text("Case sensitive (Cmd+Alt+C)")
                    .clicked()
                {
                    self.find.case_sensitive = !self.find.case_sensitive;
                    self.find.rebuild_matches(&self.content);
                }

                if ui
                    .button(egui::RichText::new("×").size(11.0).color(colors.text_dim))
                    .on_hover_text("Close (Esc)")
                    .clicked()
                {
                    self.find.close();
                }
            });

            if self.find.mode == FindUiMode::Replace {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("Replace")
                            .size(10.0)
                            .color(colors.text_dim),
                    );
                    let replace_id = ui.make_persistent_id("text_editor_replace_input");
                    let resp = ui.add(
                        egui::TextEdit::singleline(&mut self.find.replacement)
                            .id(replace_id)
                            .font(FontId::monospace(12.0))
                            .desired_width(180.0),
                    );
                    // Enter = replace current, Shift+Enter = replace all.
                    if resp.has_focus() {
                        let (enter, shift_enter) = ui.input(|i| {
                            (
                                i.key_pressed(egui::Key::Enter) && !i.modifiers.shift,
                                i.key_pressed(egui::Key::Enter) && i.modifiers.shift,
                            )
                        });
                        if enter {
                            self.replace_current();
                        }
                        if shift_enter {
                            self.replace_all();
                        }
                    }
                    if ui
                        .button(egui::RichText::new("Replace").size(10.0).color(colors.accent))
                        .clicked()
                    {
                        self.replace_current();
                    }
                    if ui
                        .button(
                            egui::RichText::new("Replace All")
                                .size(10.0)
                                .color(colors.accent),
                        )
                        .clicked()
                    {
                        self.replace_all();
                    }
                });
            }
        });
    }

    fn replace_current(&mut self) {
        if self.find.query.is_empty() || self.find.matches.is_empty() {
            return;
        }
        let idx = self.find.current;
        if let Some(range) = self.find.matches.get(idx).copied() {
            let before = &self.content[..range.0];
            let after = &self.content[range.1..];
            let new_content = format!("{before}{}{after}", self.find.replacement);
            self.undo.push_edit(&self.content, &new_content);
            self.content = new_content;
            self.highlight_cache.invalidate();
            self.find.rebuild_matches(&self.content);
            if !self.find.matches.is_empty() {
                self.find.current = idx.min(self.find.matches.len() - 1);
            }
            self.set_status("Replaced 1 match");
        }
    }

    fn replace_all(&mut self) {
        if self.find.query.is_empty() {
            return;
        }
        let (new_content, count) =
            replace_all_in(&self.content, &self.find.query, &self.find.replacement, self.find.case_sensitive);
        if count == 0 {
            self.set_status("No matches");
            return;
        }
        self.undo.push_edit(&self.content, &new_content);
        self.content = new_content;
        self.highlight_cache.invalidate();
        self.find.rebuild_matches(&self.content);
        self.set_status(format!("Replaced {count} matches"));
    }

    fn draw_goto_bar(&mut self, ui: &mut egui::Ui, colors: &Colors) {
        let frame = egui::Frame::default()
            .fill(colors.bg_sidebar)
            .inner_margin(egui::Margin::symmetric(6, 4));
        frame.show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("Go to line")
                        .size(10.0)
                        .color(colors.text_dim),
                );
                let id = ui.make_persistent_id("text_editor_goto_input");
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.goto_input)
                        .id(id)
                        .font(FontId::monospace(12.0))
                        .desired_width(80.0),
                );
                resp.request_focus();
                if resp.has_focus() {
                    let (enter, esc) = ui.input(|i| {
                        (
                            i.key_pressed(egui::Key::Enter),
                            i.key_pressed(egui::Key::Escape),
                        )
                    });
                    if enter {
                        if let Ok(line) = self.goto_input.trim().parse::<usize>() {
                            self.goto_line(ui.ctx(), line.saturating_sub(1));
                        }
                        self.goto_open = false;
                    }
                    if esc {
                        self.goto_open = false;
                    }
                }
            });
        });
    }

    fn goto_line(&mut self, ctx: &egui::Context, zero_indexed_line: usize) {
        // Find the byte offset of that line.
        let mut offset = 0usize;
        for (i, line) in self.content.split_inclusive('\n').enumerate() {
            if i == zero_indexed_line {
                break;
            }
            offset += line.len();
        }
        let editor_id = self.editor_persistent_id();
        if let Some(mut state) = egui::TextEdit::load_state(ctx, editor_id) {
            state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::one(
                    egui::text::CCursor::new(offset),
                )));
            state.store(ctx, editor_id);
        }
        self.set_status(format!("Line {}", zero_indexed_line + 1));
    }

    fn editor_persistent_id(&self) -> egui::Id {
        egui::Id::new((
            "text_editor_app",
            self.file_path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| "scratch".to_string()),
        ))
    }

    fn draw_save_as_bar(&mut self, ui: &mut egui::Ui, colors: &Colors) {
        let frame = egui::Frame::default()
            .fill(colors.bg_sidebar)
            .inner_margin(egui::Margin::symmetric(6, 4));
        frame.show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("Save as")
                        .size(10.0)
                        .color(colors.text_dim),
                );
                let id = ui.make_persistent_id("text_editor_save_as_input");
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.save_as_input)
                        .id(id)
                        .font(FontId::monospace(12.0))
                        .desired_width(320.0),
                );
                resp.request_focus();
                if resp.has_focus() {
                    let (enter, esc) = ui.input(|i| {
                        (
                            i.key_pressed(egui::Key::Enter),
                            i.key_pressed(egui::Key::Escape),
                        )
                    });
                    if enter {
                        self.save_as_confirm();
                    }
                    if esc {
                        self.save_as_open = false;
                        self.save_as_overwrite_prompt = None;
                    }
                }
                if ui
                    .button(egui::RichText::new("Save").size(10.0).color(colors.accent))
                    .clicked()
                {
                    self.save_as_confirm();
                }
                if ui
                    .button(egui::RichText::new("Cancel").size(10.0).color(colors.text_dim))
                    .clicked()
                {
                    self.save_as_open = false;
                    self.save_as_overwrite_prompt = None;
                }
            });
            if let Some(path) = self.save_as_overwrite_prompt.clone() {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!("Overwrite {}?", path.display()))
                            .size(10.0)
                            .color(colors.text_dim),
                    );
                    if ui
                        .button(egui::RichText::new("Yes").size(10.0).color(colors.accent))
                        .clicked()
                    {
                        self.write_to(path.clone());
                        self.save_as_open = false;
                        self.save_as_overwrite_prompt = None;
                    }
                    if ui
                        .button(egui::RichText::new("No").size(10.0).color(colors.text_dim))
                        .clicked()
                    {
                        self.save_as_overwrite_prompt = None;
                    }
                });
            }
        });
    }

    fn save_as_confirm(&mut self) {
        let raw = self.save_as_input.trim();
        if raw.is_empty() {
            return;
        }
        let expanded = if let Some(stripped) = raw.strip_prefix("~/") {
            dirs::home_dir()
                .map(|h| h.join(stripped))
                .unwrap_or_else(|| PathBuf::from(raw))
        } else if raw == "~" {
            dirs::home_dir().unwrap_or_else(|| PathBuf::from(raw))
        } else {
            PathBuf::from(raw)
        };
        if expanded.exists() {
            self.save_as_overwrite_prompt = Some(expanded);
        } else {
            self.write_to(expanded);
            self.save_as_open = false;
        }
    }

    fn draw_disk_conflict_prompt(&mut self, ui: &mut egui::Ui, colors: &Colors) {
        let frame = egui::Frame::default()
            .fill(colors.bg_active)
            .inner_margin(egui::Margin::symmetric(6, 4));
        frame.show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("File changed on disk.")
                        .size(10.0)
                        .color(colors.accent),
                );
                if ui
                    .button(egui::RichText::new("Reload").size(10.0).color(colors.accent))
                    .clicked()
                {
                    self.reload_from_disk();
                }
                if ui
                    .button(
                        egui::RichText::new("Keep mine")
                            .size(10.0)
                            .color(colors.text_dim),
                    )
                    .clicked()
                {
                    // Accept the new disk mtime without reloading; keep buffer dirty.
                    if let Some(path) = &self.file_path {
                        self.disk_mtime = read_mtime(path);
                    }
                    self.disk_conflict_prompt = false;
                }
                ui.add_enabled(
                    false,
                    egui::Button::new(
                        egui::RichText::new("View diff")
                            .size(10.0)
                            .color(colors.text_dim),
                    ),
                );
            });
        });
    }

    fn draw_editor_body(&mut self, ui: &mut egui::Ui, colors: &Colors, is_focused: bool) {
        let editor_id = self.editor_persistent_id();
        let lang = detect_language(self.file_path.as_deref());
        let over_limit = self.content.len() > MAX_HIGHLIGHT_BYTES;
        if over_limit && !self.highlight_cache.warned_large {
            self.set_status("File too large for syntax highlighting (plain mode)");
            self.highlight_cache.warned_large = true;
        }
        let use_syntax = !over_limit;
        let show_numbers = self.show_line_numbers;
        let wrap = self.word_wrap;
        let match_ranges = self.find.matches.clone();
        let current_match = self.find.current;

        let scroll = egui::ScrollArea::vertical()
            .id_salt("text_editor_scroll")
            .auto_shrink([false, false]);
        scroll.show(ui, |ui| {
            ui.horizontal_top(|ui| {
                if show_numbers {
                    draw_line_numbers(ui, &self.content, colors);
                }

                // Snapshot pre-edit content so we can record an undo entry if
                // TextEdit reports a change this frame.
                let pre_edit = self.content.clone();

                // Drive TextEdit with a custom layouter that performs syntax
                // highlighting + search-match underlay via the cache.
                let cache = &mut self.highlight_cache;
                let colors_copy = *colors;
                let query = self.find.query.clone();
                let case_sensitive = self.find.case_sensitive;
                let mut layouter = |ui: &egui::Ui, text: &str, wrap_width: f32| {
                    let job = cache.layout(
                        text,
                        lang,
                        &colors_copy,
                        use_syntax,
                        &query,
                        case_sensitive,
                        &match_ranges,
                        current_match,
                    );
                    let mut job = job;
                    if wrap {
                        job.wrap.max_width = wrap_width;
                    } else {
                        job.wrap.max_width = f32::INFINITY;
                    }
                    ui.fonts(|f| f.layout_job(job))
                };

                let te = egui::TextEdit::multiline(&mut self.content)
                    .id(editor_id)
                    .desired_width(f32::INFINITY)
                    .font(egui::TextStyle::Monospace)
                    .code_editor()
                    .interactive(self.edit_mode)
                    .layouter(&mut layouter);

                let response = ui.add(te);

                // Focus handling.
                if is_focused && !self.was_focused {
                    response.request_focus();
                }
                if is_focused && self.cursor_init_pending {
                    if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), editor_id) {
                        state
                            .cursor
                            .set_char_range(Some(egui::text::CCursorRange::one(
                                egui::text::CCursor::new(0),
                            )));
                        state.store(ui.ctx(), editor_id);
                    }
                    self.cursor_init_pending = false;
                }

                // Capture cursor line/col for status bar.
                if let Some(state) = egui::TextEdit::load_state(ui.ctx(), editor_id) {
                    if let Some(range) = state.cursor.char_range() {
                        let byte_offset = char_index_to_byte_offset(
                            &self.content,
                            range.primary.index,
                        );
                        self.last_cursor_line_col = byte_offset_to_line_col(&self.content, byte_offset);
                    }
                }

                if response.changed() {
                    // Record the pre-edit snapshot as a new undo entry. The
                    // stack coalesces single-character inserts that extend
                    // the previous entry to avoid one undo slot per keystroke.
                    self.undo.record(&pre_edit, &self.content);
                    self.dirty = self.content != self.saved_content;
                    self.highlight_cache.invalidate();
                    self.find.rebuild_matches(&self.content);
                    self.autosave.note_keystroke();
                }
            });
        });
    }

    fn draw_status_bar(&mut self, ui: &mut egui::Ui, colors: &Colors) {
        let rect = ui.max_rect();
        ui.painter().rect_filled(rect, 0.0, colors.bg_sidebar);
        let painter = ui.painter().clone();
        let left = egui::pos2(rect.left() + 8.0, rect.center().y);
        let right_anchor = egui::pos2(rect.right() - 8.0, rect.center().y);

        // Left: path + dirty indicator.
        let path_label = self
            .file_path
            .as_ref()
            .map(|p| {
                let d = p.display().to_string();
                dirs::home_dir()
                    .and_then(|h| {
                        d.strip_prefix(&h.display().to_string())
                            .map(|r| format!("~{r}"))
                    })
                    .unwrap_or(d)
            })
            .unwrap_or_else(|| "(scratch)".into());
        let left_text = if self.dirty {
            format!("● {path_label}")
        } else {
            path_label
        };
        painter.text(
            left,
            egui::Align2::LEFT_CENTER,
            left_text,
            FontId::monospace(11.0),
            colors.text_dim,
        );

        // Right: line:col · N lines · N bytes · lang.
        let (line, col) = self.last_cursor_line_col;
        let total_lines = self.content.matches('\n').count() + 1;
        let bytes = self.content.len();
        let lang = detect_language(self.file_path.as_deref()).name();
        let right_text = format!(
            "{line}:{col}  {total_lines}L  {bytes}B  {lang}"
        );
        painter.text(
            right_anchor,
            egui::Align2::RIGHT_CENTER,
            right_text,
            FontId::monospace(11.0),
            colors.text_dim,
        );

        // Center: transient status message (drawn on top of background).
        if let Some((msg, _)) = &self.status_msg {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                msg,
                FontId::monospace(11.0),
                colors.accent,
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Line numbers gutter
// ---------------------------------------------------------------------------

/// Draw a right-aligned monospace column of 1-indexed line numbers that
/// auto-grows to fit the largest digit count.
fn draw_line_numbers(ui: &mut egui::Ui, content: &str, colors: &Colors) {
    let total_lines = content.matches('\n').count() + 1;
    let digit_count = total_lines.to_string().len().max(2);
    let char_w = 8.0;
    let gutter_w = char_w * digit_count as f32 + 10.0;

    let mut job = LayoutJob::default();
    for i in 0..total_lines {
        let label = format!("{:>width$} \n", i + 1, width = digit_count);
        job.append(
            &label,
            0.0,
            TextFormat {
                font_id: FontId::monospace(13.0),
                color: colors.text_dim,
                ..Default::default()
            },
        );
    }
    let galley = ui.fonts(|f| f.layout_job(job));
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(gutter_w, galley.size().y),
        egui::Sense::hover(),
    );
    ui.painter().galley(rect.min, galley, colors.text_dim);
}

// ---------------------------------------------------------------------------
// Helpers: mtime, cursor, byte/line conversions
// ---------------------------------------------------------------------------

fn read_mtime(path: &std::path::Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn char_index_to_byte_offset(text: &str, char_idx: usize) -> usize {
    text.char_indices()
        .nth(char_idx)
        .map(|(b, _)| b)
        .unwrap_or(text.len())
}

fn byte_offset_to_line_col(text: &str, byte: usize) -> (usize, usize) {
    let clamped = byte.min(text.len());
    let prefix = &text[..clamped];
    let line = prefix.matches('\n').count() + 1;
    let last_nl = prefix.rfind('\n').map(|i| i + 1).unwrap_or(0);
    let col_bytes = &prefix[last_nl..];
    let col = col_bytes.chars().count() + 1;
    (line, col)
}

fn replace_all_in(
    haystack: &str,
    needle: &str,
    replacement: &str,
    case_sensitive: bool,
) -> (String, usize) {
    if needle.is_empty() {
        return (haystack.to_string(), 0);
    }
    if case_sensitive {
        let count = haystack.matches(needle).count();
        (haystack.replace(needle, replacement), count)
    } else {
        let lower_needle = needle.to_lowercase();
        let mut out = String::with_capacity(haystack.len());
        let mut count = 0usize;
        let mut i = 0usize;
        let lower_hay = haystack.to_lowercase();
        let bytes_lower = lower_hay.as_bytes();
        let needle_len = lower_needle.len();
        while i < haystack.len() {
            if i + needle_len <= bytes_lower.len()
                && &bytes_lower[i..i + needle_len] == lower_needle.as_bytes()
            {
                out.push_str(replacement);
                i += needle_len;
                count += 1;
            } else {
                // Advance by one char (UTF-8 safe). `i < haystack.len()` holds
                // here so `chars().next()` always yields Some, but we match
                // defensively to avoid any panic on pathological inputs.
                match haystack[i..].chars().next() {
                    Some(ch) => {
                        out.push(ch);
                        i += ch.len_utf8();
                    }
                    None => break,
                }
            }
        }
        (out, count)
    }
}

// ---------------------------------------------------------------------------
// Submodules (inline to avoid touching main.rs for registration)
// ---------------------------------------------------------------------------

mod undo {
    use std::time::{Duration, Instant};

    /// Idle gap after which consecutive edits start a new undo entry. Edits
    /// closer together than this (or that cross a whitespace boundary) are
    /// coalesced into the previous entry so typing a word is one undo step.
    const COALESCE_GAP: Duration = Duration::from_millis(500);

    /// A content-snapshot undo stack. Each entry stores the pre-edit string.
    /// Adjacent fast single-character edits coalesce into the previous entry
    /// to avoid one undo slot per keystroke. Capped at `cap`.
    pub struct UndoStack {
        past: Vec<String>,
        future: Vec<String>,
        cap: usize,
        last_record: Option<Instant>,
    }

    impl UndoStack {
        pub fn new(cap: usize) -> Self {
            Self {
                past: Vec::new(),
                future: Vec::new(),
                cap,
                last_record: None,
            }
        }

        pub fn clear(&mut self) {
            self.past.clear();
            self.future.clear();
            self.last_record = None;
        }

        /// Record an edit from `before` to `after`. No-op if unchanged.
        /// Coalesces edits within COALESCE_GAP into the previous entry.
        pub fn record(&mut self, before: &str, after: &str) {
            if before == after {
                return;
            }
            self.future.clear();
            let now = Instant::now();
            let hot = self
                .last_record
                .map(|t| now.duration_since(t) < COALESCE_GAP)
                .unwrap_or(false);
            // Crossing a whitespace boundary forces a new undo step even
            // during hot typing, so word-by-word undo works.
            let crossed_whitespace = last_char_differs_in_kind(before, after);
            if hot && !self.past.is_empty() && !crossed_whitespace {
                // Coalesce: leave the existing "before" snapshot as the undo
                // target. The new `after` just shifts further from it.
            } else {
                self.past.push(before.to_string());
                if self.past.len() > self.cap {
                    let excess = self.past.len() - self.cap;
                    self.past.drain(0..excess);
                }
            }
            self.last_record = Some(now);
        }

        /// Push an explicit edit pair, used for programmatic edits
        /// (replace, replace-all). Always creates a new undo entry and
        /// resets the coalesce timer.
        pub fn push_edit(&mut self, before: &str, after: &str) {
            if before == after {
                return;
            }
            self.past.push(before.to_string());
            self.future.clear();
            if self.past.len() > self.cap {
                let excess = self.past.len() - self.cap;
                self.past.drain(0..excess);
            }
            self.last_record = None;
        }

        /// Undo: returns the previous snapshot and pushes `current` onto redo.
        pub fn undo(&mut self, current: &str) -> Option<String> {
            let prev = self.past.pop()?;
            self.future.push(current.to_string());
            self.last_record = None;
            Some(prev)
        }

        /// Redo: returns the next snapshot and pushes `current` onto undo.
        pub fn redo(&mut self, current: &str) -> Option<String> {
            let next = self.future.pop()?;
            self.past.push(current.to_string());
            self.last_record = None;
            Some(next)
        }
    }

    /// Heuristic: returns true if the last edited character's kind (word /
    /// whitespace / punctuation) changed between `before` and `after`. Used
    /// to break coalescing at word boundaries.
    fn last_char_differs_in_kind(before: &str, after: &str) -> bool {
        let a = before.chars().last();
        let b = after.chars().last();
        match (a, b) {
            (Some(x), Some(y)) => char_kind(x) != char_kind(y),
            _ => false,
        }
    }

    fn char_kind(c: char) -> u8 {
        if c.is_whitespace() {
            0
        } else if c.is_alphanumeric() || c == '_' {
            1
        } else {
            2
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn undo_redo_roundtrip() {
            let mut u = UndoStack::new(10);
            u.push_edit("", "hello");
            u.push_edit("hello", "hello world");
            let u1 = u.undo("hello world").unwrap();
            assert_eq!(u1, "hello");
            let u2 = u.undo("hello").unwrap();
            assert_eq!(u2, "");
            let r1 = u.redo("").unwrap();
            assert_eq!(r1, "hello");
            let r2 = u.redo("hello").unwrap();
            assert_eq!(r2, "hello world");
        }

        #[test]
        fn push_edit_respects_cap() {
            let mut u = UndoStack::new(3);
            u.push_edit("a", "b");
            u.push_edit("b", "c");
            u.push_edit("c", "d");
            u.push_edit("d", "e");
            // Oldest entry dropped; only last 3 remain.
            assert_eq!(u.past.len(), 3);
        }
    }
}

mod search {
    /// Find bar visibility / mode.
    #[derive(Clone, Copy, PartialEq, Eq, Default)]
    pub enum FindUiMode {
        #[default]
        Hidden,
        Find,
        Replace,
    }

    /// Search state + matches + replacement input.
    #[derive(Default)]
    pub struct FindState {
        pub mode: FindUiMode,
        pub query: String,
        pub replacement: String,
        pub case_sensitive: bool,
        pub matches: Vec<(usize, usize)>, // byte ranges
        pub current: usize,
        pub focus_request: bool,
    }

    impl FindState {
        pub fn toggle_find(&mut self) {
            self.mode = match self.mode {
                FindUiMode::Hidden => {
                    self.focus_request = true;
                    FindUiMode::Find
                }
                FindUiMode::Find | FindUiMode::Replace => FindUiMode::Hidden,
            };
        }

        pub fn toggle_replace(&mut self) {
            self.mode = match self.mode {
                FindUiMode::Hidden | FindUiMode::Find => {
                    self.focus_request = true;
                    FindUiMode::Replace
                }
                FindUiMode::Replace => FindUiMode::Hidden,
            };
        }

        pub fn close(&mut self) {
            self.mode = FindUiMode::Hidden;
            self.matches.clear();
            self.current = 0;
        }

        /// Rebuild the `matches` vector from the current query and content.
        pub fn rebuild_matches(&mut self, content: &str) {
            self.matches.clear();
            self.current = 0;
            if self.query.is_empty() {
                return;
            }
            if self.case_sensitive {
                let mut start = 0usize;
                while let Some(pos) = content[start..].find(&self.query) {
                    let abs = start + pos;
                    self.matches.push((abs, abs + self.query.len()));
                    start = abs + self.query.len().max(1);
                }
            } else {
                let lower_hay = content.to_lowercase();
                let lower_needle = self.query.to_lowercase();
                let mut start = 0usize;
                while let Some(pos) = lower_hay[start..].find(&lower_needle) {
                    let abs = start + pos;
                    // Map lowered byte positions back — lowercasing ASCII is
                    // length-preserving. Non-ASCII case folding can shift
                    // byte lengths; in that case we degrade to ASCII-ish.
                    if lower_hay.len() == content.len() {
                        self.matches.push((abs, abs + lower_needle.len()));
                    } else {
                        // Non-ASCII: fall back to case-sensitive style.
                        break;
                    }
                    start = abs + lower_needle.len().max(1);
                }
                if lower_hay.len() != content.len() {
                    // Re-run case-sensitive as a fallback.
                    self.matches.clear();
                    let mut start = 0usize;
                    while let Some(pos) = content[start..].find(&self.query) {
                        let abs = start + pos;
                        self.matches.push((abs, abs + self.query.len()));
                        start = abs + self.query.len().max(1);
                    }
                }
            }
        }

        /// Advance `current` by delta (wraps). `delta > 0` = next.
        pub fn advance(&mut self, _content: &str, delta: i32) {
            if self.matches.is_empty() {
                return;
            }
            let len = self.matches.len() as i32;
            let mut cur = self.current as i32 + delta;
            cur = ((cur % len) + len) % len;
            self.current = cur as usize;
        }
    }
}

mod highlight {
    use super::Colors;
    use egui::text::{LayoutJob, TextFormat};
    use egui::{Color32, FontId, Stroke};
    use std::path::Path;
    use std::sync::OnceLock;
    use syntect::easy::HighlightLines;
    use syntect::highlighting::{Style, ThemeSet};
    use syntect::parsing::{SyntaxReference, SyntaxSet};
    use syntect::util::LinesWithEndings;

    /// Minimal language classification used for selecting the syntect syntax
    /// and displaying a short label in the status bar.
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub enum SyntaxKind {
        Plain,
        Markdown,
        Toml,
        Rust,
        Python,
        Javascript,
        Typescript,
        Json,
        Yaml,
        Html,
        Css,
        Shell,
    }

    impl SyntaxKind {
        pub fn name(self) -> &'static str {
            match self {
                SyntaxKind::Plain => "text",
                SyntaxKind::Markdown => "md",
                SyntaxKind::Toml => "toml",
                SyntaxKind::Rust => "rust",
                SyntaxKind::Python => "py",
                SyntaxKind::Javascript => "js",
                SyntaxKind::Typescript => "ts",
                SyntaxKind::Json => "json",
                SyntaxKind::Yaml => "yaml",
                SyntaxKind::Html => "html",
                SyntaxKind::Css => "css",
                SyntaxKind::Shell => "sh",
            }
        }

        fn syntect_name(self) -> &'static str {
            match self {
                SyntaxKind::Rust => "Rust",
                SyntaxKind::Python => "Python",
                SyntaxKind::Javascript => "JavaScript",
                SyntaxKind::Typescript => "TypeScript",
                SyntaxKind::Json => "JSON",
                SyntaxKind::Yaml => "YAML",
                SyntaxKind::Toml => "TOML",
                SyntaxKind::Markdown => "Markdown",
                SyntaxKind::Html => "HTML",
                SyntaxKind::Css => "CSS",
                SyntaxKind::Shell => "Bourne Again Shell (bash)",
                SyntaxKind::Plain => "Plain Text",
            }
        }
    }

    /// Detect a language from path extension (case-insensitive). Falls back to
    /// `Plain` for unknown extensions.
    pub fn detect_language(path: Option<&Path>) -> SyntaxKind {
        let Some(path) = path else {
            return SyntaxKind::Plain;
        };
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_default();
        match ext.as_str() {
            "rs" => SyntaxKind::Rust,
            "py" => SyntaxKind::Python,
            "js" | "mjs" | "cjs" | "jsx" => SyntaxKind::Javascript,
            "ts" | "tsx" => SyntaxKind::Typescript,
            "json" => SyntaxKind::Json,
            "yaml" | "yml" => SyntaxKind::Yaml,
            "toml" => SyntaxKind::Toml,
            "md" | "markdown" => SyntaxKind::Markdown,
            "html" | "htm" => SyntaxKind::Html,
            "css" | "scss" => SyntaxKind::Css,
            "sh" | "bash" | "zsh" | "fish" | "env" => SyntaxKind::Shell,
            _ => SyntaxKind::Plain,
        }
    }

    struct SyntectBundle {
        syntaxes: SyntaxSet,
        themes: ThemeSet,
    }

    fn bundle() -> &'static SyntectBundle {
        static BUNDLE: OnceLock<SyntectBundle> = OnceLock::new();
        BUNDLE.get_or_init(|| SyntectBundle {
            syntaxes: SyntaxSet::load_defaults_newlines(),
            themes: ThemeSet::load_defaults(),
        })
    }

    fn pick_syntax(kind: SyntaxKind) -> Option<&'static SyntaxReference> {
        let bundle = bundle();
        let target = kind.syntect_name();
        bundle.syntaxes.find_syntax_by_name(target)
    }

    fn pick_theme() -> &'static syntect::highlighting::Theme {
        let bundle = bundle();
        // `base16-eighties.dark` is the lowest-contrast built-in theme and
        // blends well with Plexi's muted palette.
        bundle
            .themes
            .themes
            .get("base16-eighties.dark")
            .or_else(|| bundle.themes.themes.get("base16-ocean.dark"))
            .or_else(|| bundle.themes.themes.values().next())
            .expect("syntect ships at least one default theme")
    }

    /// Cached syntax highlighting output. Recomputed whenever `text` or
    /// highlighting parameters change.
    pub struct HighlightCache {
        last_text_hash: u64,
        last_lang: Option<SyntaxKind>,
        last_use_syntax: bool,
        last_query: String,
        last_case: bool,
        last_matches_hash: u64,
        last_current: usize,
        cached_job: Option<LayoutJob>,
        pub warned_large: bool,
    }

    impl HighlightCache {
        pub fn new() -> Self {
            Self {
                last_text_hash: 0,
                last_lang: None,
                last_use_syntax: true,
                last_query: String::new(),
                last_case: false,
                last_matches_hash: 0,
                last_current: usize::MAX,
                cached_job: None,
                warned_large: false,
            }
        }

        pub fn invalidate(&mut self) {
            self.cached_job = None;
            self.last_text_hash = 0;
        }

        #[allow(clippy::too_many_arguments)]
        pub fn layout(
            &mut self,
            text: &str,
            lang: SyntaxKind,
            colors: &Colors,
            use_syntax: bool,
            query: &str,
            case_sensitive: bool,
            matches: &[(usize, usize)],
            current_match: usize,
        ) -> LayoutJob {
            let text_hash = fast_hash(text.as_bytes());
            let matches_hash = fast_hash_matches(matches);
            let cache_hit = self.cached_job.is_some()
                && self.last_text_hash == text_hash
                && self.last_lang == Some(lang)
                && self.last_use_syntax == use_syntax
                && self.last_query == query
                && self.last_case == case_sensitive
                && self.last_matches_hash == matches_hash
                && self.last_current == current_match;
            if cache_hit {
                if let Some(job) = &self.cached_job {
                    return job.clone();
                }
            }

            let job = build_layout_job(text, lang, colors, use_syntax, matches, current_match);
            self.last_text_hash = text_hash;
            self.last_lang = Some(lang);
            self.last_use_syntax = use_syntax;
            self.last_query = query.to_string();
            self.last_case = case_sensitive;
            self.last_matches_hash = matches_hash;
            self.last_current = current_match;
            self.cached_job = Some(job.clone());
            job
        }
    }

    fn fast_hash(bytes: &[u8]) -> u64 {
        // FNV-1a (64-bit). Not cryptographic; good enough for cache keying.
        let mut h: u64 = 0xcbf29ce484222325;
        for &b in bytes {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        h
    }

    fn fast_hash_matches(m: &[(usize, usize)]) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        for (a, b) in m {
            h ^= *a as u64;
            h = h.wrapping_mul(0x100000001b3);
            h ^= *b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        h
    }

    fn build_layout_job(
        text: &str,
        lang: SyntaxKind,
        colors: &Colors,
        use_syntax: bool,
        matches: &[(usize, usize)],
        current_match: usize,
    ) -> LayoutJob {
        let mut job = LayoutJob::default();
        let font = FontId::monospace(13.0);
        if !use_syntax || matches!(lang, SyntaxKind::Plain) {
            append_with_matches(&mut job, text, colors.text_primary, &font, matches, current_match, colors);
            return job;
        }
        let Some(syntax) = pick_syntax(lang) else {
            append_with_matches(&mut job, text, colors.text_primary, &font, matches, current_match, colors);
            return job;
        };
        let theme = pick_theme();
        let bundle = bundle();
        let mut highlighter = HighlightLines::new(syntax, theme);
        let mut byte_cursor = 0usize;
        for line in LinesWithEndings::from(text) {
            let regions = match highlighter.highlight_line(line, &bundle.syntaxes) {
                Ok(r) => r,
                Err(_) => {
                    append_with_matches(
                        &mut job,
                        line,
                        colors.text_primary,
                        &font,
                        matches,
                        current_match,
                        colors,
                    );
                    byte_cursor += line.len();
                    continue;
                }
            };
            for (style, chunk) in regions {
                let color = blend_for_theme(style, colors);
                let chunk_start = byte_cursor;
                let chunk_end = byte_cursor + chunk.len();
                // Slice of matches overlapping this chunk, in chunk-local coords.
                append_chunk(
                    &mut job,
                    chunk,
                    color,
                    &font,
                    matches,
                    current_match,
                    chunk_start,
                    chunk_end,
                    colors,
                );
                byte_cursor = chunk_end;
            }
        }
        job
    }

    /// Blend syntect's per-token color toward Plexi's primary text color so
    /// vivid themes don't overwhelm the muted palette.
    fn blend_for_theme(style: Style, colors: &Colors) -> Color32 {
        let fg = Color32::from_rgb(style.foreground.r, style.foreground.g, style.foreground.b);
        // Mix 65% syntect / 35% plexi primary to tame saturation.
        mix(fg, colors.text_primary, 0.65)
    }

    fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
        let t = t.clamp(0.0, 1.0);
        let inv = 1.0 - t;
        Color32::from_rgb(
            (a.r() as f32 * t + b.r() as f32 * inv) as u8,
            (a.g() as f32 * t + b.g() as f32 * inv) as u8,
            (a.b() as f32 * t + b.b() as f32 * inv) as u8,
        )
    }

    fn append_with_matches(
        job: &mut LayoutJob,
        text: &str,
        color: Color32,
        font: &FontId,
        matches: &[(usize, usize)],
        current_match: usize,
        colors: &Colors,
    ) {
        append_chunk(job, text, color, font, matches, current_match, 0, text.len(), colors);
    }

    #[allow(clippy::too_many_arguments)]
    fn append_chunk(
        job: &mut LayoutJob,
        chunk: &str,
        color: Color32,
        font: &FontId,
        matches: &[(usize, usize)],
        current_match: usize,
        chunk_start: usize,
        chunk_end: usize,
        colors: &Colors,
    ) {
        if chunk.is_empty() {
            return;
        }
        let mut cursor = chunk_start;
        for (i, (m_start, m_end)) in matches.iter().copied().enumerate() {
            if m_end <= chunk_start || m_start >= chunk_end {
                continue;
            }
            let start = m_start.max(chunk_start);
            let end = m_end.min(chunk_end);
            if start > cursor {
                push_piece(
                    job,
                    &chunk[cursor - chunk_start..start - chunk_start],
                    color,
                    font,
                    None,
                );
            }
            let bg = if i == current_match {
                colors.accent
            } else {
                colors.bg_active
            };
            push_piece(
                job,
                &chunk[start - chunk_start..end - chunk_start],
                if i == current_match {
                    colors.bg_darkest
                } else {
                    color
                },
                font,
                Some(bg),
            );
            cursor = end;
        }
        if cursor < chunk_end {
            push_piece(
                job,
                &chunk[cursor - chunk_start..],
                color,
                font,
                None,
            );
        }
    }

    fn push_piece(
        job: &mut LayoutJob,
        text: &str,
        color: Color32,
        font: &FontId,
        background: Option<Color32>,
    ) {
        if text.is_empty() {
            return;
        }
        job.append(
            text,
            0.0,
            TextFormat {
                font_id: font.clone(),
                color,
                background: background.unwrap_or(Color32::TRANSPARENT),
                underline: Stroke::NONE,
                ..Default::default()
            },
        );
    }
}

mod autosave {
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    /// Auto-save for unnamed scratch files. Tracks keystrokes + elapsed time
    /// and writes to `~/.plexi-alpha/scratch/autosave-<id>.txt` once either
    /// threshold is crossed. After the first save the editor's `file_path` is
    /// pointed at the autosave file and subsequent saves use the normal path.
    pub struct Autosave {
        last_save: Instant,
        keystrokes: u32,
        /// The autosave path we own, once created.
        owned_path: Option<PathBuf>,
    }

    impl Autosave {
        pub fn new() -> Self {
            Self {
                last_save: Instant::now(),
                keystrokes: 0,
                owned_path: None,
            }
        }

        pub fn reset(&mut self) {
            self.last_save = Instant::now();
            self.keystrokes = 0;
        }

        pub fn note_keystroke(&mut self) {
            self.keystrokes = self.keystrokes.saturating_add(1);
        }

        /// True if the editor's current `file_path` is an autosave file we created.
        pub fn owns(&self, current: &Option<PathBuf>) -> bool {
            match (&self.owned_path, current) {
                (Some(a), Some(b)) => a == b,
                _ => false,
            }
        }

        /// Consider saving. Returns `Some(status_message)` if a write happened.
        #[allow(clippy::too_many_arguments)]
        pub fn maybe_save(
            &mut self,
            content: &str,
            dirty: bool,
            interval: Duration,
            key_threshold: u32,
            file_path: &mut Option<PathBuf>,
            saved_content: &mut String,
        ) -> Option<String> {
            // Only auto-save scratch (no explicit path) or our own autosave file.
            let is_scratch = file_path.is_none() || self.owns(file_path);
            if !is_scratch {
                return None;
            }
            if !dirty && self.keystrokes == 0 {
                return None;
            }
            let time_triggered = self.last_save.elapsed() >= interval && dirty;
            let key_triggered = self.keystrokes >= key_threshold;
            if !(time_triggered || key_triggered) {
                return None;
            }
            // Resolve/create path.
            let path = if let Some(p) = &self.owned_path {
                p.clone()
            } else {
                let scratch_dir = crate::config::config_dir().join("scratch");
                if let Err(e) = std::fs::create_dir_all(&scratch_dir) {
                    log::error!("TextEditorApp: autosave dir failed: {e}");
                    return None;
                }
                let ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let p = scratch_dir.join(format!("autosave-{ts}.txt"));
                self.owned_path = Some(p.clone());
                p
            };

            match std::fs::write(&path, content) {
                Ok(()) => {
                    log::info!("TextEditorApp: autosaved scratch to {}", path.display());
                    *file_path = Some(path.clone());
                    *saved_content = content.to_string();
                    self.last_save = Instant::now();
                    self.keystrokes = 0;
                    Some(format!("Autosaved {}", path.display()))
                }
                Err(e) => {
                    log::error!("TextEditorApp: autosave write failed: {e}");
                    None
                }
            }
        }
    }
}
