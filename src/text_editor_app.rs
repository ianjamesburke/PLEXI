use crate::app_trait::{App, AppCommand, AppRenderContext};
use crate::theme::Colors;
use egui::text::{LayoutJob, TextFormat};
use egui::FontId;
use std::path::PathBuf;

pub struct TextEditorApp {
    file_path: Option<PathBuf>,
    content: String,
    saved_content: String,
    edit_mode: bool,
    dirty: bool,
    was_focused: bool,
    cursor_init_pending: bool,
    pending_cmds: Vec<AppCommand>,
}

impl TextEditorApp {
    pub fn new_scratch(cwd: PathBuf) -> Self {
        let _ = cwd; // available for future use
        let content = String::new();
        Self {
            file_path: None,
            content: content.clone(),
            saved_content: content,
            edit_mode: true,
            dirty: false,
            was_focused: false,
            cursor_init_pending: true,
            pending_cmds: Vec::new(),
        }
    }

    pub fn from_file(path: PathBuf) -> Self {
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                log::error!("TextEditorApp: failed to read {}: {e}", path.display());
                String::new()
            }
        };
        Self {
            file_path: Some(path),
            content: content.clone(),
            saved_content: content,
            edit_mode: false,
            dirty: false,
            was_focused: false,
            cursor_init_pending: true,
            pending_cmds: Vec::new(),
        }
    }

    fn save(&mut self) {
        let path = match &self.file_path {
            Some(p) => p.clone(),
            None => {
                let notes_dir = dirs::home_dir()
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join(".plexi")
                    .join("scratch");
                if let Err(e) = std::fs::create_dir_all(&notes_dir) {
                    log::error!("TextEditorApp: failed to create scratch dir: {e}");
                    return;
                }
                let ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let p = notes_dir.join(format!("scratch-{ts}.txt"));
                self.file_path = Some(p.clone());
                p
            }
        };
        match std::fs::write(&path, &self.content) {
            Ok(()) => {
                self.saved_content = self.content.clone();
                self.dirty = false;
                self.pending_cmds
                    .push(AppCommand::Notify(format!("Saved {}", path.display())));
                log::info!("TextEditorApp: saved {}", path.display());
            }
            Err(e) => {
                log::error!("TextEditorApp: save failed for {}: {e}", path.display());
                self.pending_cmds
                    .push(AppCommand::Notify(format!("Save failed: {e}")));
            }
        }
    }

    fn discard_changes(&mut self) {
        self.content = self.saved_content.clone();
        self.dirty = false;
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

        // Consume Cmd+S BEFORE TextEdit renders, so it doesn't insert "s".
        let cmd_s = ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S));
        if cmd_s {
            self.save();
        }

        // Top bar
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
                ui.label(
                    egui::RichText::new("modified")
                        .size(10.0)
                        .color(colors.accent.gamma_multiply(0.7)),
                );
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
        ui.add_space(4.0);
        ui.separator();
        ui.add_space(4.0);

        let syntax_mode = detect_syntax_mode(self.file_path.as_ref());
        let editor_id = ui.make_persistent_id((
            "text_editor_app",
            self.file_path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| "scratch".to_string()),
        ));

        egui::ScrollArea::vertical().show(ui, |ui| {
            let colors_copy = *colors;
            let mut layouter = |ui: &egui::Ui, text: &str, wrap_width: f32| {
                let mut job = highlight_layout(text, syntax_mode, &colors_copy);
                job.wrap.max_width = wrap_width;
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
            if response.changed() {
                self.dirty = true;
            }
        });
        self.was_focused = is_focused;
    }

    fn handle_key(&mut self, input: &egui::InputState) -> bool {
        // Cmd+S is handled in ui() via input_mut. Don't consume anything else with Cmd held.
        // GOTCHA: consume_key(Modifiers::NONE, Key) matches even with modifiers held,
        // so always check modifier state manually before consuming.
        if input.modifiers.command {
            // Let Cmd-modified keys pass through to Plexi (except Cmd+S handled in ui()).
            return false;
        }
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
        }))
    }

    fn restore_state(&mut self, state: &serde_json::Value) {
        if let Some(path_str) = state.get("file_path").and_then(|v| v.as_str()) {
            let path = PathBuf::from(path_str);
            // Re-read from disk if possible; fall back to serialised content.
            match std::fs::read_to_string(&path) {
                Ok(disk_content) => {
                    self.content = disk_content.clone();
                    self.saved_content = disk_content;
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
        self.dirty = false;
    }
}

// ---------------------------------------------------------------------------
// Syntax highlighting (ported from beta/v2 markdown_pane.rs)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum SyntaxMode {
    Plain,
    Markdown,
    Toml,
}

fn detect_syntax_mode(path: Option<&PathBuf>) -> SyntaxMode {
    let Some(path) = path else {
        return SyntaxMode::Plain;
    };
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref()
    {
        Some("md") | Some("markdown") => SyntaxMode::Markdown,
        Some("toml") => SyntaxMode::Toml,
        _ => SyntaxMode::Plain,
    }
}

fn mono_format(color: egui::Color32) -> TextFormat {
    TextFormat {
        font_id: FontId::monospace(13.0),
        color,
        ..Default::default()
    }
}

fn append(job: &mut LayoutJob, text: &str, color: egui::Color32) {
    if text.is_empty() {
        return;
    }
    job.append(text, 0.0, mono_format(color));
}

fn highlight_layout(text: &str, mode: SyntaxMode, colors: &Colors) -> LayoutJob {
    match mode {
        SyntaxMode::Markdown => highlight_markdown(text, colors),
        SyntaxMode::Toml => highlight_toml(text, colors),
        SyntaxMode::Plain => {
            let mut job = LayoutJob::default();
            append(&mut job, text, colors.text_primary);
            job
        }
    }
}

fn highlight_markdown(text: &str, colors: &Colors) -> LayoutJob {
    let mut job = LayoutJob::default();
    let mut in_code_block = false;
    for line in text.split_inclusive('\n') {
        let trim = line.trim_start();
        if trim.starts_with("```") {
            in_code_block = !in_code_block;
            append(&mut job, line, colors.accent);
            continue;
        }
        if in_code_block {
            append(&mut job, line, colors.text_section);
            continue;
        }
        if trim.starts_with("<!--") || trim.starts_with("-->") || trim.contains("<!--") {
            append(&mut job, line, colors.text_dim);
            continue;
        }
        if trim.starts_with('#') {
            append(&mut job, line, colors.accent);
            continue;
        }
        if trim.starts_with('>') {
            append(&mut job, line, colors.text_dim);
            continue;
        }

        let mut cursor = 0usize;
        let mut in_backticks = false;
        let chars: Vec<(usize, char)> = line.char_indices().collect();
        for (idx, ch) in chars {
            if ch == '`' {
                append(
                    &mut job,
                    &line[cursor..idx],
                    if in_backticks {
                        colors.text_section
                    } else {
                        colors.text_primary
                    },
                );
                append(&mut job, "`", colors.accent.gamma_multiply(0.95));
                cursor = idx + ch.len_utf8();
                in_backticks = !in_backticks;
            }
        }
        append(
            &mut job,
            &line[cursor..],
            if in_backticks {
                colors.text_section
            } else {
                colors.text_primary
            },
        );
    }
    job
}

fn highlight_toml(text: &str, colors: &Colors) -> LayoutJob {
    let mut job = LayoutJob::default();
    for line in text.split_inclusive('\n') {
        let trim = line.trim_start();
        if trim.starts_with('#') {
            append(&mut job, line, colors.text_dim);
            continue;
        }

        let comment_idx = find_unquoted_hash(line);
        let (code_part, comment_part) = match comment_idx {
            Some(idx) => (&line[..idx], Some(&line[idx..])),
            None => (line, None),
        };

        if let Some(eq_idx) = code_part.find('=') {
            let (key, value) = code_part.split_at(eq_idx);
            append(&mut job, key, colors.accent);
            append(&mut job, "=", colors.text_primary);
            highlight_toml_values(&mut job, &value[1..], colors);
        } else if trim.starts_with('[') {
            append(&mut job, code_part, colors.accent.gamma_multiply(0.92));
        } else {
            append(&mut job, code_part, colors.text_primary);
        }

        if let Some(comment) = comment_part {
            append(&mut job, comment, colors.text_dim);
        }
    }
    job
}

fn highlight_toml_values(job: &mut LayoutJob, text: &str, colors: &Colors) {
    let mut in_string = false;
    let mut start = 0usize;
    for (idx, ch) in text.char_indices() {
        if ch == '"' && !is_escaped(text, idx) {
            append(
                job,
                &text[start..idx],
                if in_string {
                    colors.accent.gamma_multiply(0.85)
                } else {
                    colors.text_primary
                },
            );
            append(job, "\"", colors.accent.gamma_multiply(0.9));
            in_string = !in_string;
            start = idx + 1;
        }
    }
    append(
        job,
        &text[start..],
        if in_string {
            colors.accent.gamma_multiply(0.85)
        } else {
            colors.text_primary
        },
    );
}

fn is_escaped(text: &str, idx: usize) -> bool {
    let bytes = text.as_bytes();
    if idx == 0 {
        return false;
    }
    let mut backslashes = 0usize;
    let mut i = idx;
    while i > 0 {
        i -= 1;
        if bytes[i] == b'\\' {
            backslashes += 1;
        } else {
            break;
        }
    }
    backslashes % 2 == 1
}

fn find_unquoted_hash(line: &str) -> Option<usize> {
    let mut in_string = false;
    for (idx, ch) in line.char_indices() {
        if ch == '"' && !is_escaped(line, idx) {
            in_string = !in_string;
            continue;
        }
        if ch == '#' && !in_string {
            return Some(idx);
        }
    }
    None
}
