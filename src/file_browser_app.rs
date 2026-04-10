use crate::app_trait::{App, AppCommand, AppRenderContext};
use std::fs;
use std::path::PathBuf;

pub struct FileBrowserApp {
    pub cwd: PathBuf,
    entries: Vec<Entry>,
    selected: usize,
    error: Option<String>,
}

#[derive(Clone)]
struct Entry {
    name: String,
    path: PathBuf,
    is_dir: bool,
}

impl FileBrowserApp {
    pub fn new(cwd: PathBuf) -> Self {
        let mut app = Self {
            cwd: cwd.clone(),
            entries: Vec::new(),
            selected: 0,
            error: None,
        };
        app.refresh();
        app
    }

    fn refresh(&mut self) {
        self.entries.clear();
        self.error = None;

        match fs::read_dir(&self.cwd) {
            Ok(dir) => {
                let mut entries: Vec<Entry> = dir
                    .filter_map(|e| {
                        let e = e.ok()?;
                        let name = e.file_name().to_string_lossy().to_string();
                        // Hide dotfiles
                        if name.starts_with('.') {
                            return None;
                        }
                        let path = e.path();
                        let is_dir = path.is_dir();
                        Some(Entry { name, path, is_dir })
                    })
                    .collect();

                // Directories first, then files, both alphabetical.
                entries.sort_by(|a, b| {
                    b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name))
                });

                self.entries = entries;
                self.selected = self.selected.min(self.entries.len().saturating_sub(1));
            }
            Err(e) => {
                self.error = Some(format!("Cannot read directory: {e}"));
            }
        }
    }

    fn navigate_into(&mut self, path: PathBuf) {
        self.cwd = path;
        self.selected = 0;
        self.refresh();
    }

    fn navigate_up(&mut self) {
        if let Some(parent) = self.cwd.parent().map(|p| p.to_path_buf()) {
            self.cwd = parent;
            self.selected = 0;
            self.refresh();
        }
    }

    fn current_entry(&self) -> Option<&Entry> {
        self.entries.get(self.selected)
    }
}

impl App for FileBrowserApp {
    fn type_id(&self) -> &'static str {
        "file_browser"
    }

    fn display_name(&self) -> String {
        self.cwd
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "/".to_string())
    }

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &AppRenderContext<'_>) {
        let colors = ctx.colors;

        egui::Frame::new()
            .fill(colors.terminal_bg)
            .inner_margin(egui::Margin::symmetric(12, 8))
            .show(ui, |ui| {
                // Header: current directory
                ui.horizontal(|ui| {
                    ui.colored_label(
                        colors.accent,
                        egui::RichText::new(self.cwd.display().to_string())
                            .size(11.0)
                            .monospace(),
                    );
                });

                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);

                if let Some(err) = &self.error {
                    ui.colored_label(colors.text_dim, err.clone());
                    return;
                }

                if self.entries.is_empty() {
                    ui.colored_label(colors.text_dim, "Empty directory");
                    return;
                }

                // Entry list — scrollable
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        // Up row
                        if self.cwd.parent().is_some() {
                            let row = ui.add(
                                egui::Label::new(
                                    egui::RichText::new("  ..  (parent)")
                                        .size(12.0)
                                        .color(colors.text_dim),
                                )
                                .sense(egui::Sense::click()),
                            );
                            if row.double_clicked() {
                                self.navigate_up();
                            }
                            ui.add_space(2.0);
                        }

                        // Clone entries to avoid borrow issues
                        let entries: Vec<_> = self.entries.iter().cloned().collect();
                        let mut navigate_into: Option<PathBuf> = None;
                        let mut open_command: Option<String> = None;

                        for (i, entry) in entries.iter().enumerate() {
                            let is_selected = i == self.selected;

                            let icon = if entry.is_dir { "▶ " } else { "  " };
                            let label_text = format!("{}{}", icon, entry.name);

                            let text = egui::RichText::new(&label_text)
                                .size(12.0)
                                .monospace()
                                .color(if is_selected {
                                    colors.text_primary
                                } else {
                                    colors.text_dim
                                });

                            let bg = if is_selected {
                                colors.bg_active
                            } else {
                                colors.terminal_bg
                            };

                            let (row_rect, row_resp) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), 20.0),
                                egui::Sense::click(),
                            );

                            ui.painter().rect_filled(row_rect, 2.0, bg);
                            ui.painter().text(
                                egui::pos2(row_rect.min.x + 8.0, row_rect.center().y),
                                egui::Align2::LEFT_CENTER,
                                &label_text,
                                egui::FontId::monospace(12.0),
                                if is_selected { colors.text_primary } else { colors.text_dim },
                            );

                            if row_resp.clicked() {
                                self.selected = i;
                            }

                            if row_resp.double_clicked() {
                                if entry.is_dir {
                                    navigate_into = Some(entry.path.clone());
                                } else {
                                    open_command = Some(format!("open {:?}", entry.path));
                                }
                            }
                        }

                        if let Some(path) = navigate_into {
                            self.navigate_into(path);
                        }
                        drop(open_command); // handled via on_command in future
                    });
            });
    }

    fn handle_key(&mut self, input: &egui::InputState) -> bool {
        let mut consumed = false;

        if input.key_pressed(egui::Key::ArrowDown) {
            self.selected = (self.selected + 1).min(self.entries.len().saturating_sub(1));
            consumed = true;
        }
        if input.key_pressed(egui::Key::ArrowUp) {
            self.selected = self.selected.saturating_sub(1);
            consumed = true;
        }
        if input.key_pressed(egui::Key::ArrowLeft) {
            self.navigate_up();
            consumed = true;
        }
        if input.key_pressed(egui::Key::ArrowRight) {
            if let Some(entry) = self.current_entry().cloned() {
                if entry.is_dir {
                    self.navigate_into(entry.path);
                }
            }
            consumed = true;
        }

        consumed
    }

    fn on_command(&mut self, cmd: &str) -> Option<AppCommand> {
        let parts: Vec<&str> = cmd.trim().splitn(2, ' ').collect();
        match parts.as_slice() {
            ["cd", path] => {
                let target = PathBuf::from(path);
                let target = if target.is_absolute() {
                    target
                } else {
                    self.cwd.join(target)
                };
                if target.is_dir() {
                    self.navigate_into(target.clone());
                    return Some(AppCommand::Cd(target));
                }
                None
            }
            ["open"] | ["enter"] => {
                if let Some(entry) = self.current_entry().cloned() {
                    if entry.is_dir {
                        self.navigate_into(entry.path.clone());
                        Some(AppCommand::Cd(entry.path))
                    } else {
                        Some(AppCommand::RunInTerminal(format!(
                            "open {:?}",
                            entry.path
                        )))
                    }
                } else {
                    None
                }
            }
            _ => {
                // Unknown command — pass through to terminal.
                Some(AppCommand::RunInTerminal(cmd.to_string()))
            }
        }
    }

    fn accepted_extensions(&self) -> &[&str] {
        &[] // directory browser, not file-type driven
    }

    fn serialize_state(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "cwd": self.cwd.display().to_string(),
            "selected": self.selected,
        }))
    }

    fn restore_state(&mut self, state: &serde_json::Value) {
        if let Some(cwd) = state["cwd"].as_str() {
            let path = PathBuf::from(cwd);
            if path.is_dir() {
                self.cwd = path;
                self.refresh();
            }
        }
        if let Some(sel) = state["selected"].as_u64() {
            self.selected = (sel as usize).min(self.entries.len().saturating_sub(1));
        }
    }
}
