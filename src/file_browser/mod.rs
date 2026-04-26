mod helpers;
mod icons;

use crate::app_trait::{App, AppCommand, AppRenderContext};
use crate::theme::Colors;
use egui::{Color32, CornerRadius, Stroke, StrokeKind};
use image::imageops::FilterType;
use std::fs;
use std::path::{Path, PathBuf};

use helpers::{format_modified, format_size, DirStats, Entry, MediaKind, SortMode};
use icons::paint_entry_icon;

const ROW_HEIGHT: f32 = 58.0;
const MIN_SIDEBAR_WIDTH: f32 = 920.0;
const DIR_PREVIEW_CAP: usize = 500;

pub struct FileBrowserApp {
    pub cwd: PathBuf,
    entries: Vec<Entry>,
    selected: usize,
    sort_mode: SortMode,
    error: Option<String>,
    // Image preview
    preview_texture: Option<egui::TextureHandle>,
    preview_texture_path: Option<PathBuf>,
    preview_size: Option<[usize; 2]>,
    preview_error: Option<String>,
    // Dir preview
    dir_preview_path: Option<PathBuf>,
    dir_preview_stats: Option<DirStats>,
    pending_cmds: Vec<AppCommand>,
    /// When true, the next draw_list pass will scroll the selected row into view.
    pending_scroll: bool,
    /// Remembers which entry was selected when leaving a directory,
    /// so navigating back restores the selection.
    directory_selection_memory: std::collections::HashMap<PathBuf, String>,
    // Fuzzy search
    in_search: bool,
    search_query: String,
    search_indices: Vec<usize>, // indices into `entries` that match the query
    should_close: bool,
}

impl FileBrowserApp {
    pub fn new(cwd: PathBuf) -> Self {
        let mut app = Self {
            cwd,
            entries: Vec::new(),
            selected: 0,
            sort_mode: SortMode::RecentlyTouched,
            error: None,
            preview_texture: None,
            preview_texture_path: None,
            preview_size: None,
            preview_error: None,
            dir_preview_path: None,
            dir_preview_stats: None,
            pending_cmds: Vec::new(),
            pending_scroll: false,
            directory_selection_memory: std::collections::HashMap::new(),
            in_search: false,
            search_query: String::new(),
            search_indices: Vec::new(),
            should_close: false,
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
                        if name.starts_with('.') {
                            return None;
                        }
                        let path = e.path();
                        let is_dir = e.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
                        let meta = e.metadata().ok();
                        let size_bytes =
                            meta.as_ref()
                                .and_then(|m| if is_dir { None } else { Some(m.len()) });
                        let modified = meta.as_ref().and_then(|m| m.modified().ok());
                        let ext = path
                            .extension()
                            .and_then(|e| e.to_str())
                            .map(|e| e.to_ascii_lowercase());
                        let is_image = ext
                            .as_deref()
                            .map(|e| {
                                matches!(
                                    e,
                                    "png" | "jpg" | "jpeg" | "gif" | "bmp" | "tiff" | "webp"
                                )
                            })
                            .unwrap_or(false);
                        Some(Entry {
                            name,
                            path,
                            is_dir,
                            is_image,
                            size_bytes,
                            modified,
                        })
                    })
                    .collect();

                match self.sort_mode {
                    SortMode::Name => {
                        entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));
                    }
                    SortMode::RecentlyTouched => {
                        entries.sort_by(|a, b| {
                            b.is_dir
                                .cmp(&a.is_dir)
                                .then(b.modified.cmp(&a.modified).then(a.name.cmp(&b.name)))
                        });
                    }
                }
                self.entries = entries;
                self.selected = self.selected.min(self.entries.len().saturating_sub(1));
            }
            Err(e) => {
                self.error = Some(format!("Cannot read directory: {e}"));
            }
        }
    }

    fn navigate_into(&mut self, path: PathBuf) {
        if let Some(entry) = self.selected_entry() {
            self.directory_selection_memory
                .insert(self.cwd.clone(), entry.name.clone());
        }
        self.cwd = path.clone();
        self.selected = 0;
        self.refresh();
        self.pending_scroll = true;
        self.pending_cmds.push(AppCommand::CdRequest {
            cwd: self.cwd.to_string_lossy().to_string(),
            sender_pane_id: 0, // dispatch.rs stamps the real pane_id
        });
    }

    /// Open a file the user activated (Enter, double-click, search-Enter).
    /// GUI↔Terminal media bridge (#79): recognised video/audio extensions
    /// route to the in-app players (`video-player` / `audio-player`) so
    /// the user stays inside the canvas. Everything else falls through
    /// to the system default opener (`open` on macOS, `xdg-open` on
    /// Linux). Failures to spawn the system opener are logged and
    /// silently swallowed — they're not user-recoverable from the file
    /// browser.
    fn open_file(&mut self, path: &Path) {
        let kind = MediaKind::for_path(path);
        if let Some(app_id) = kind.player_app_id() {
            log::info!(
                "file_browser: routing {kind:?} file '{}' to in-app player '{app_id}'",
                path.display()
            );
            self.pending_cmds.push(AppCommand::SpawnApp {
                type_id: app_id.to_string(),
                layout: None, // respect the player's manifest layout_hint
                args: vec![path.to_string_lossy().to_string()],
            });
            return;
        }
        match std::process::Command::new(Self::system_opener()).arg(path).spawn() {
            Ok(_) => log::debug!("file_browser: system-open spawned for {}", path.display()),
            Err(e) => log::error!(
                "file_browser: system-open failed for {}: {e}",
                path.display()
            ),
        }
    }

    /// Platform-appropriate fallback opener. macOS / Linux only — Windows
    /// callers fall through to a no-op since the media-bridge surface is
    /// unix-first for v3.4 (mirrors `canvas_bindings::shell_open`).
    fn system_opener() -> &'static str {
        #[cfg(target_os = "macos")]
        {
            "open"
        }
        #[cfg(target_os = "linux")]
        {
            "xdg-open"
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            // The Command will fail to spawn; the error log makes the
            // platform gap visible without panicking.
            "open"
        }
    }

    fn navigate_up(&mut self) {
        if let Some(parent) = self.cwd.parent().map(|p| p.to_path_buf()) {
            let leaving_name = self
                .cwd
                .file_name()
                .map(|n| n.to_string_lossy().to_string());
            self.cwd = parent.clone();
            self.selected = 0;
            self.refresh();
            self.pending_cmds.push(AppCommand::CdRequest {
                cwd: self.cwd.to_string_lossy().to_string(),
                sender_pane_id: 0, // dispatch.rs stamps the real pane_id
            });
            let restore_name = self
                .directory_selection_memory
                .remove(&self.cwd)
                .or(leaving_name);
            if let Some(name) = restore_name {
                if let Some(idx) = self.entries.iter().position(|e| e.name == name) {
                    self.selected = idx;
                }
            }
            self.pending_scroll = true;
        }
    }

    fn selected_entry(&self) -> Option<&Entry> {
        if self.in_search {
            self.search_indices.get(self.selected).and_then(|&i| self.entries.get(i))
        } else {
            self.entries.get(self.selected)
        }
    }

    fn refilter(&mut self) {
        let q = self.search_query.to_lowercase();
        self.search_indices = (0..self.entries.len())
            .filter(|&i| {
                if q.is_empty() {
                    return true;
                }
                let name = self.entries[i].name.to_lowercase();
                let mut qi = q.chars().peekable();
                for c in name.chars() {
                    if qi.peek() == Some(&c) {
                        qi.next();
                    }
                }
                qi.peek().is_none()
            })
            .collect();
        self.selected = 0;
        self.pending_scroll = true;
    }

    fn exit_search(&mut self) {
        self.in_search = false;
        self.search_query.clear();
        self.search_indices.clear();
        self.selected = 0;
        self.pending_scroll = true;
    }

    /// Called by the host when the linked terminal's CWD changes.
    pub fn sync_cwd(&mut self, new_cwd: PathBuf) {
        if new_cwd == self.cwd {
            return;
        }
        if let Some(entry) = self.selected_entry() {
            self.directory_selection_memory
                .insert(self.cwd.clone(), entry.name.clone());
        }
        let restore_name = self.directory_selection_memory.get(&new_cwd).cloned();
        self.cwd = new_cwd;
        self.selected = 0;
        self.refresh();
        if let Some(name) = restore_name {
            if let Some(idx) = self.entries.iter().position(|e| e.name == name) {
                self.selected = idx;
            }
        }
        self.pending_scroll = true;
    }

    // ─── Preview ensure methods ──────────────────────────────────────────────

    fn ensure_image_preview(&mut self, ctx: &egui::Context, path: &Path) {
        if self.preview_texture_path.as_deref() == Some(path) {
            return;
        }
        self.preview_texture = None;
        self.preview_texture_path = Some(path.to_path_buf());
        self.preview_size = None;
        self.preview_error = None;

        let decoded = match image::open(path) {
            Ok(img) => img,
            Err(e) => {
                self.preview_error = Some(format!("Unable to decode image: {e}"));
                return;
            }
        };
        let decoded = if decoded.width().max(decoded.height()) > 2048 {
            decoded.resize(2048, 2048, FilterType::Triangle)
        } else {
            decoded
        };
        let rgba = decoded.to_rgba8();
        let size = [rgba.width() as usize, rgba.height() as usize];
        let color_image = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
        let texture = ctx.load_texture(
            format!("fb-preview:{}", path.display()),
            color_image,
            egui::TextureOptions::LINEAR,
        );
        self.preview_size = Some(size);
        self.preview_texture = Some(texture);
    }

    fn ensure_dir_preview(&mut self, path: &Path) {
        if self.dir_preview_path.as_deref() == Some(path) {
            return;
        }
        self.dir_preview_path = Some(path.to_path_buf());
        let mut file_count = 0usize;
        let mut dir_count = 0usize;
        let mut total_bytes = 0u64;
        let mut truncated = false;
        if let Ok(entries) = fs::read_dir(path) {
            for e in entries.flatten() {
                if file_count + dir_count >= DIR_PREVIEW_CAP {
                    truncated = true;
                    break;
                }
                let is_dir = e.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
                if is_dir {
                    dir_count += 1;
                } else {
                    file_count += 1;
                    if let Ok(m) = e.metadata() {
                        total_bytes += m.len();
                    }
                }
            }
        }
        self.dir_preview_stats = Some(DirStats {
            file_count,
            dir_count,
            total_bytes,
            truncated,
        });
    }

    // ─── Drawing ─────────────────────────────────────────────────────────────

    fn draw_list(&mut self, ui: &mut egui::Ui, colors: &Colors) -> Option<(PathBuf, bool)> {
        let mut navigate_to: Option<(PathBuf, bool)> = None;
        let should_scroll = self.pending_scroll;
        self.pending_scroll = false;
        let display_count = if self.in_search { self.search_indices.len() } else { self.entries.len() };
        for idx in 0..display_count {
            let actual_idx = if self.in_search { self.search_indices[idx] } else { idx };
            let entry = self.entries[actual_idx].clone();
            let (rect, resp) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), ROW_HEIGHT),
                egui::Sense::click(),
            );
            let is_selected = self.selected == idx;
            if is_selected && should_scroll {
                resp.scroll_to_me(None);
            }
            let fill = if is_selected {
                colors.bg_active
            } else if resp.hovered() {
                colors.bg_hover
            } else {
                colors.bg_sidebar
            };
            ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
            ui.painter().rect_stroke(
                rect,
                CornerRadius::same(6),
                Stroke::new(
                    if is_selected { 1.5 } else { 1.0 },
                    if is_selected {
                        colors.accent
                    } else {
                        colors.border
                    },
                ),
                StrokeKind::Inside,
            );

            let icon_rect = egui::Rect::from_min_size(
                egui::pos2(rect.left() + 8.0, rect.center().y - 12.0),
                egui::vec2(24.0, 24.0),
            );
            paint_entry_icon(ui.painter(), icon_rect, &entry, colors);

            let title = if entry.is_dir {
                format!("{}/", entry.name)
            } else {
                entry.name.clone()
            };
            let sub = if entry.is_dir {
                "directory".to_string()
            } else {
                format_size(entry.size_bytes)
            };
            ui.painter().text(
                egui::pos2(rect.left() + 40.0, rect.top() + 10.0),
                egui::Align2::LEFT_TOP,
                title,
                egui::FontId::proportional(11.5),
                colors.text_primary,
            );
            ui.painter().text(
                egui::pos2(rect.left() + 40.0, rect.top() + 32.0),
                egui::Align2::LEFT_TOP,
                format!("{sub} \u{00b7} {}", format_modified(entry.modified)),
                egui::FontId::proportional(9.5),
                colors.text_dim,
            );

            if resp.clicked() {
                self.selected = idx;
            }
            if resp.double_clicked() {
                self.selected = idx;
                navigate_to = Some((entry.path.clone(), entry.is_dir));
            }
            ui.add_space(4.0);
        }
        navigate_to
    }

    fn draw_sidebar_preview(&mut self, ui: &mut egui::Ui, colors: &Colors) {
        let entry = match self.selected_entry().cloned() {
            Some(e) => e,
            None => return,
        };

        if entry.is_image {
            self.draw_image_sidebar(ui, colors, &entry);
        } else if entry.is_dir {
            self.draw_dir_sidebar(ui, colors, &entry);
        } else {
            self.draw_generic_sidebar(ui, colors, &entry);
        }
    }

    fn draw_image_sidebar(&mut self, ui: &mut egui::Ui, colors: &Colors, entry: &Entry) {
        let path = entry.path.clone();
        self.ensure_image_preview(ui.ctx(), &path);

        egui::Frame::new()
            .fill(colors.bg_sidebar)
            .stroke(Stroke::new(1.0, colors.border))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new(format!("Image \u{00b7} {}", entry.name))
                        .size(10.5)
                        .color(colors.text_primary)
                        .strong(),
                );
                if let Some([w, h]) = self.preview_size {
                    ui.label(
                        egui::RichText::new(format!("{w}\u{00d7}{h}"))
                            .size(10.0)
                            .color(colors.text_dim),
                    );
                }
                ui.add_space(6.0);
                let preview_max = egui::vec2(ui.available_width(), 220.0);
                let (slot_rect, _) = ui.allocate_exact_size(preview_max, egui::Sense::hover());
                ui.painter().rect_filled(
                    slot_rect,
                    CornerRadius::same(4),
                    colors.bg_darkest.gamma_multiply(0.95),
                );
                ui.painter().rect_stroke(
                    slot_rect,
                    CornerRadius::same(4),
                    Stroke::new(1.0, colors.border),
                    StrokeKind::Inside,
                );
                if let Some(texture) = &self.preview_texture {
                    let tex_size = egui::vec2(texture.size()[0] as f32, texture.size()[1] as f32);
                    let scale = (slot_rect.width() / tex_size.x)
                        .min(slot_rect.height() / tex_size.y)
                        .min(1.0);
                    let draw_size = tex_size * scale.max(0.01);
                    let image_rect = egui::Rect::from_center_size(slot_rect.center(), draw_size);
                    ui.painter().image(
                        texture.id(),
                        image_rect,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                } else if let Some(err) = &self.preview_error {
                    ui.painter().text(
                        slot_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        err,
                        egui::FontId::proportional(10.0),
                        Color32::from_rgb(0xff, 0xaf, 0xaf),
                    );
                } else {
                    ui.painter().text(
                        slot_rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "Loading\u{2026}",
                        egui::FontId::proportional(10.0),
                        colors.text_dim,
                    );
                }
                ui.add_space(6.0);
                if let Some(size) = entry.size_bytes {
                    ui.label(
                        egui::RichText::new(format_size(Some(size)))
                            .size(9.5)
                            .color(colors.text_dim),
                    );
                }
            });
    }

    fn draw_dir_sidebar(&mut self, ui: &mut egui::Ui, colors: &Colors, entry: &Entry) {
        let path = entry.path.clone();
        self.ensure_dir_preview(&path);
        let stats = self.dir_preview_stats;

        egui::Frame::new()
            .fill(colors.bg_sidebar)
            .stroke(Stroke::new(1.0, colors.border))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new(format!("Folder \u{00b7} {}", entry.name))
                        .size(10.5)
                        .color(colors.text_primary)
                        .strong(),
                );
                ui.separator();
                ui.label(
                    egui::RichText::new(format!("Path: {}", entry.path.display()))
                        .size(9.5)
                        .color(colors.text_dim),
                );
                let truncated = stats.map(|s| s.truncated).unwrap_or(false);
                let suffix = if truncated { "+" } else { "" };
                ui.label(
                    egui::RichText::new(format!(
                        "Contains: {}{suffix} folders, {}{suffix} files",
                        stats.map(|s| s.dir_count).unwrap_or(0),
                        stats.map(|s| s.file_count).unwrap_or(0),
                    ))
                    .size(9.5)
                    .color(colors.text_primary),
                );
                ui.label(
                    egui::RichText::new(format!(
                        "Immediate file size: {}",
                        format_size(stats.map(|s| s.total_bytes))
                    ))
                    .size(9.5)
                    .color(colors.text_primary),
                );
            });
    }

    fn draw_generic_sidebar(&mut self, ui: &mut egui::Ui, colors: &Colors, entry: &Entry) {
        egui::Frame::new()
            .fill(colors.bg_sidebar)
            .stroke(Stroke::new(1.0, colors.border))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new(entry.name.clone())
                        .size(10.5)
                        .color(colors.text_primary)
                        .strong(),
                );
                ui.separator();
                if let Some(size) = entry.size_bytes {
                    ui.label(
                        egui::RichText::new(format_size(Some(size)))
                            .size(9.5)
                            .color(colors.text_dim),
                    );
                }
                if let Some(modified) = entry.modified {
                    ui.label(
                        egui::RichText::new(format!(
                            "Modified: {}",
                            format_modified(Some(modified))
                        ))
                        .size(9.5)
                        .color(colors.text_dim),
                    );
                }
            });
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
                ui.horizontal(|ui| {
                    ui.colored_label(
                        colors.accent,
                        egui::RichText::new(self.cwd.display().to_string())
                            .size(11.0)
                            .monospace(),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let name_label = if self.sort_mode == SortMode::Name {
                            "Name \u{2713}"
                        } else {
                            "Name"
                        };
                        let recent_label = if self.sort_mode == SortMode::RecentlyTouched {
                            "Recent \u{2713}"
                        } else {
                            "Recent"
                        };
                        if ui.small_button(name_label).clicked() {
                            self.sort_mode = SortMode::Name;
                            self.refresh();
                        }
                        if ui.small_button(recent_label).clicked() {
                            self.sort_mode = SortMode::RecentlyTouched;
                            self.refresh();
                        }
                    });
                });

                ui.add_space(4.0);

                if self.in_search {
                    ui.horizontal(|ui| {
                        ui.colored_label(colors.accent, "/");
                        ui.colored_label(colors.text_primary,
                            if self.search_query.is_empty() { "type to filter…" } else { &self.search_query });
                        let count = self.search_indices.len();
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.colored_label(colors.text_dim,
                                format!("{count} match{}", if count == 1 { "" } else { "es" }));
                        });
                    });
                }

                ui.separator();
                ui.add_space(4.0);

                if let Some(err) = &self.error.clone() {
                    ui.colored_label(colors.text_dim, err);
                    return;
                }

                if self.entries.is_empty() {
                    ui.colored_label(colors.text_dim, "Empty directory");
                    return;
                }

                let show_sidebar = ui.available_width() >= MIN_SIDEBAR_WIDTH;
                let mut navigate_to: Option<(PathBuf, bool)> = None;

                if show_sidebar {
                    ui.columns(2, |columns| {
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show(&mut columns[0], |ui| {
                                navigate_to = self.draw_list(ui, colors);
                            });
                        self.draw_sidebar_preview(&mut columns[1], colors);
                    });
                } else {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            navigate_to = self.draw_list(ui, colors);
                        });
                }

                if let Some((path, is_dir)) = navigate_to {
                    if is_dir {
                        self.navigate_into(path);
                    } else {
                        self.open_file(&path);
                    }
                }
            });
    }

    fn handle_key(&mut self, input: &egui::InputState) -> bool {
        // Search mode: handle all input here and return.
        if self.in_search {
            if input.key_pressed(egui::Key::Escape) {
                self.exit_search();
                return true;
            }
            if input.key_pressed(egui::Key::Backspace) {
                self.search_query.pop();
                self.refilter();
                return true;
            }
            if input.key_pressed(egui::Key::Enter) {
                if let Some(&entry_idx) = self.search_indices.get(self.selected) {
                    let entry = self.entries[entry_idx].clone();
                    if entry.is_dir {
                        self.exit_search();
                        self.navigate_into(entry.path);
                    } else {
                        self.open_file(&entry.path);
                        self.exit_search();
                    }
                }
                return true;
            }
            let last_filtered = self.search_indices.len().saturating_sub(1);
            if input.key_pressed(egui::Key::ArrowDown) || input.key_pressed(egui::Key::J) {
                self.selected = (self.selected + 1).min(last_filtered);
                self.pending_scroll = true;
                return true;
            }
            if input.key_pressed(egui::Key::ArrowUp) || input.key_pressed(egui::Key::K) {
                self.selected = self.selected.saturating_sub(1);
                self.pending_scroll = true;
                return true;
            }
            for event in &input.events {
                if let egui::Event::Text(text) = event {
                    self.search_query.push_str(text);
                    self.refilter();
                }
            }
            return true;
        }

        if self.entries.is_empty() {
            if input.key_pressed(egui::Key::Backspace) {
                self.navigate_up();
                return true;
            }
            return false;
        }

        // Escape closes the file browser.
        if input.key_pressed(egui::Key::Escape) {
            self.should_close = true;
            return true;
        }

        // '/' enters search mode.
        if input.key_pressed(egui::Key::Slash) && !input.modifiers.command {
            self.in_search = true;
            self.search_query.clear();
            self.refilter();
            return true;
        }

        let last = self.entries.len().saturating_sub(1);
        let mut consumed = false;

        if input.key_pressed(egui::Key::ArrowDown)
            || (input.key_pressed(egui::Key::J) && !input.modifiers.any())
        {
            self.selected = (self.selected + 1).min(last);
            self.pending_scroll = true;
            consumed = true;
        }
        if input.key_pressed(egui::Key::ArrowUp)
            || (input.key_pressed(egui::Key::K) && !input.modifiers.any())
        {
            self.selected = self.selected.saturating_sub(1);
            self.pending_scroll = true;
            consumed = true;
        }
        if input.key_pressed(egui::Key::Home) {
            self.selected = 0;
            self.pending_scroll = true;
            consumed = true;
        }
        if input.key_pressed(egui::Key::End) {
            self.selected = last;
            self.pending_scroll = true;
            consumed = true;
        }
        if input.key_pressed(egui::Key::PageDown) {
            self.selected = (self.selected + 10).min(last);
            self.pending_scroll = true;
            consumed = true;
        }
        if input.key_pressed(egui::Key::PageUp) {
            self.selected = self.selected.saturating_sub(10);
            self.pending_scroll = true;
            consumed = true;
        }

        if !input.modifiers.command
            && (input.key_pressed(egui::Key::Enter)
                || input.key_pressed(egui::Key::ArrowRight)
                || input.key_pressed(egui::Key::L))
        {
            if let Some(entry) = self.selected_entry().cloned() {
                if entry.is_dir {
                    self.navigate_into(entry.path);
                } else {
                    self.open_file(&entry.path);
                }
            }
            consumed = true;
        }

        if !input.modifiers.command
            && (input.key_pressed(egui::Key::Backspace)
                || input.key_pressed(egui::Key::ArrowLeft)
                || input.key_pressed(egui::Key::H))
        {
            self.navigate_up();
            consumed = true;
        }

        if input.key_pressed(egui::Key::S) && !input.modifiers.any() {
            self.sort_mode = match self.sort_mode {
                SortMode::RecentlyTouched => SortMode::Name,
                SortMode::Name => SortMode::RecentlyTouched,
            };
            self.refresh();
            consumed = true;
        }

        if input.key_pressed(egui::Key::R) && !input.modifiers.any() {
            self.refresh();
            consumed = true;
        }

        consumed
    }

    fn take_pending_commands(&mut self) -> Vec<AppCommand> {
        std::mem::take(&mut self.pending_cmds)
    }

    fn wants_close(&self) -> bool {
        self.should_close
    }

    fn sync_cwd(&mut self, new_cwd: &std::path::Path) {
        self.sync_cwd(new_cwd.to_path_buf());
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
