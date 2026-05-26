use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use syntect::easy::HighlightLines;
use syntect::highlighting::{Theme, ThemeSet};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId};
use crate::app_trait::{App, AppCommand, AppRenderContext};

static SYNTAX_SET: OnceLock<SyntaxSet> = OnceLock::new();
static THEME: OnceLock<Theme> = OnceLock::new();

fn syntax_set() -> &'static SyntaxSet {
    SYNTAX_SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn theme() -> &'static Theme {
    THEME.get_or_init(|| {
        let theme_set = ThemeSet::load_defaults();
        theme_set.themes["base16-ocean.dark"].clone()
    })
}

pub(crate) struct TextEditorApp {
    path: PathBuf,
    content: String,
    original_content: String,
    dirty: bool,
    should_close: bool,
    dirty_escape_warned: bool,
    focus_requested: bool,
    // Cache: rebuild layout only when content changes
    content_version: u64,
    cached_version: u64,
    cached_layout: Option<LayoutJob>,
}

impl TextEditorApp {
    /// Create from args: args[0] is the file path. If absent, opens a new scratchpad note.
    pub(crate) fn from_args(args: &[String]) -> Self {
        let path = if let Some(p) = args.first() {
            PathBuf::from(p)
        } else {
            scratchpad_file()
        };
        Self::new(path)
    }

    pub(crate) fn new(path: PathBuf) -> Self {
        // Ensure parent directory exists (important for scratchpad notes/).
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                log::warn!("text_editor: could not create dir {:?}: {e}", parent);
            }
        }

        let content = std::fs::read_to_string(&path).unwrap_or_default();
        log::info!("text_editor: opening '{}' ({} bytes)", path.display(), content.len());

        let original_content = content.clone();
        Self {
            path,
            content,
            original_content,
            dirty: false,
            should_close: false,
            dirty_escape_warned: false,
            focus_requested: false,
            content_version: 0,
            cached_version: u64::MAX,
            cached_layout: None,
        }
    }

    fn build_layout(&mut self, font_size: f32) -> LayoutJob {
        let ext = self.path.extension().and_then(|e| e.to_str()).unwrap_or("txt");
        let ss = syntax_set();
        let syntax = ss
            .find_syntax_by_extension(ext)
            .or_else(|| ss.find_syntax_by_name("Plain Text"))
            .unwrap_or_else(|| ss.find_syntax_plain_text());

        let mut highlighter = HighlightLines::new(syntax, theme());
        let mut job = LayoutJob::default();
        let font_id = FontId::monospace(font_size);

        for line in LinesWithEndings::from(&self.content) {
            let ranges = highlighter.highlight_line(line, ss).unwrap_or_default();
            for (style, text) in ranges {
                let fg = style.foreground;
                let color = Color32::from_rgb(fg.r, fg.g, fg.b);
                job.append(text, 0.0, TextFormat {
                    font_id: font_id.clone(),
                    color,
                    ..Default::default()
                });
            }
        }

        // Empty file fallback: at least one section so TextEdit has something to measure.
        if job.sections.is_empty() {
            job.append("", 0.0, TextFormat {
                font_id,
                color: Color32::GRAY,
                ..Default::default()
            });
        }

        self.cached_version = self.content_version;
        self.cached_layout = Some(job.clone());
        job
    }
}

impl App for TextEditorApp {
    fn type_id(&self) -> &'static str {
        "text_editor"
    }

    fn display_name(&self) -> String {
        let filename = self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "untitled".to_string());
        if self.dirty {
            format!("● {filename}")
        } else {
            filename
        }
    }

    fn keyboard_capture(&self) -> bool {
        true
    }

    fn wants_close(&self) -> bool {
        self.should_close
    }

    fn take_pending_commands(&mut self) -> Vec<AppCommand> {
        vec![]
    }

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &AppRenderContext<'_>) {
        let colors = ctx.colors;
        let content_before = self.content.clone();

        // Derive a per-pane unique ID from the enclosing ui region.
        // Using ui.id() avoids static-ID conflicts when multiple editors are open.
        let te_id = ui.id().with("text_editor_main");
        let has_focus = ui.memory(|mem| mem.has_focus(te_id));

        // Guard input behind has_focus so unfocused editors don't react to
        // Escape/Cmd+S pressed in another pane (global input state is shared).
        let cmd_s = has_focus && ui.input(|i| i.key_pressed(egui::Key::S) && i.modifiers.command);
        if cmd_s {
            match std::fs::write(&self.path, &self.content) {
                Ok(()) => {
                    log::info!("text_editor: saved '{}' ({} bytes)", self.path.display(), self.content.len());
                    self.original_content = self.content.clone();
                    self.dirty = false;
                    self.dirty_escape_warned = false;
                }
                Err(e) => {
                    log::error!("text_editor: failed to save '{}': {e}", self.path.display());
                }
            }
        }

        let escape = has_focus && ui.input(|i| i.key_pressed(egui::Key::Escape));
        if escape {
            if !self.dirty {
                log::info!("text_editor: clean close of '{}'", self.path.display());
                self.should_close = true;
            } else if !self.dirty_escape_warned {
                log::info!("text_editor: dirty escape warning for '{}'", self.path.display());
                self.dirty_escape_warned = true;
            } else {
                log::info!("text_editor: discarding changes to '{}' and closing", self.path.display());
                self.should_close = true;
            }
        }

        egui::Frame::new()
            .fill(colors.terminal_bg)
            .show(ui, |ui| {
                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.colored_label(colors.text_dim, egui::RichText::new("⌘S save").size(11.0));
                        ui.colored_label(colors.text_dim, egui::RichText::new("  ").size(11.0));
                        if self.dirty && self.dirty_escape_warned {
                            ui.colored_label(
                                egui::Color32::YELLOW,
                                egui::RichText::new("Unsaved — Esc again to discard").size(11.0),
                            );
                        } else {
                            ui.colored_label(colors.text_dim, egui::RichText::new("Esc close").size(11.0));
                        }
                        if self.dirty {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.colored_label(egui::Color32::YELLOW, egui::RichText::new("●").size(11.0));
                            });
                        }
                    });
                    ui.separator();

                    let available = ui.available_size();
                    let font_size = 13.0_f32;

                    let cached_job = if self.content_version == self.cached_version {
                        self.cached_layout.clone().unwrap_or_else(|| self.build_layout(font_size))
                    } else {
                        self.build_layout(font_size)
                    };

                    let mut layouter = |ui: &egui::Ui, _string: &str, wrap_width: f32| -> Arc<egui::Galley> {
                        let mut job = cached_job.clone();
                        job.wrap.max_width = wrap_width;
                        ui.fonts(|f| f.layout_job(job))
                    };

                    let te_response = ui.add_sized(
                        available,
                        egui::TextEdit::multiline(&mut self.content)
                            .id(te_id)
                            .font(egui::TextStyle::Monospace)
                            .desired_width(f32::INFINITY)
                            .frame(false)
                            .layouter(&mut layouter),
                    );

                    // One-shot focus request after all widgets render (GOTCHAS.md Fix 1).
                    if !self.focus_requested {
                        te_response.request_focus();
                        self.focus_requested = true;
                    }
                });
            });

        if self.content != content_before {
            self.content_version += 1;
            self.dirty = self.content != self.original_content;
            if !self.dirty {
                self.dirty_escape_warned = false;
            }
        }
    }

    fn serialize_state(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "path": self.path.to_string_lossy(),
        }))
    }

    fn restore_state(&mut self, state: &serde_json::Value) {
        if let Some(path) = state.get("path").and_then(|v| v.as_str()) {
            let new_path = PathBuf::from(path);
            if new_path != self.path {
                let content = std::fs::read_to_string(&new_path).unwrap_or_default();
                log::info!("text_editor: restoring state for '{}'", new_path.display());
                self.original_content = content.clone();
                self.content = content;
                self.path = new_path;
                self.dirty = false;
                self.content_version += 1;
            }
        }
    }
}

/// Timestamped scratchpad file: `<config_dir>/notes/YYYY-MM-DD_HHMMSS.md`.
pub(crate) fn scratchpad_file() -> PathBuf {
    use chrono::Local;
    let timestamp = Local::now().format("%Y-%m-%d_%H%M%S").to_string();
    crate::config::config_dir().join("notes").join(format!("{timestamp}.md"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn make_temp_file(content: &str, ext: &str) -> (tempfile::NamedTempFile, PathBuf) {
        let mut f = tempfile::Builder::new().suffix(&format!(".{ext}")).tempfile().unwrap();
        f.write_all(content.as_bytes()).unwrap();
        let path = f.path().to_path_buf();
        (f, path)
    }

    #[test]
    fn display_name_clean() {
        let (_f, path) = make_temp_file("hello", "rs");
        let app = TextEditorApp::new(path.clone());
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        assert_eq!(app.display_name(), name);
    }

    #[test]
    fn display_name_dirty() {
        let (_f, path) = make_temp_file("hello", "rs");
        let mut app = TextEditorApp::new(path.clone());
        app.content = "changed".to_string();
        app.dirty = true;
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        assert_eq!(app.display_name(), format!("● {name}"));
    }

    #[test]
    fn dirty_flag_follows_content() {
        let (_f, path) = make_temp_file("original", "txt");
        let mut app = TextEditorApp::new(path);
        assert!(!app.dirty);
        app.content = "changed".to_string();
        app.dirty = app.content != app.original_content;
        assert!(app.dirty);
        app.content = "original".to_string();
        app.dirty = app.content != app.original_content;
        assert!(!app.dirty);
    }

    #[test]
    fn wants_close_false_by_default() {
        let (_f, path) = make_temp_file("", "txt");
        let app = TextEditorApp::new(path);
        assert!(!app.wants_close());
    }

    #[test]
    fn keyboard_capture_true() {
        let (_f, path) = make_temp_file("", "txt");
        let app = TextEditorApp::new(path);
        assert!(app.keyboard_capture());
    }

    #[test]
    fn serialize_restore_roundtrip() {
        let (_f, path) = make_temp_file("data", "md");
        let app = TextEditorApp::new(path.clone());
        let state = app.serialize_state().unwrap();
        assert_eq!(
            state.get("path").and_then(|v| v.as_str()),
            Some(path.to_str().unwrap())
        );
    }
}
