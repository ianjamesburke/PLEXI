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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Normal,
    Search,
    Empty,
}

#[derive(Debug)]
enum FileBrowserAction {
    SelectNext,
    SelectPrev,
    SelectFirst,
    SelectLast,
    PageDown,
    PageUp,
    Activate,
    NavigateUp,
    Backspace,
    Escape,
    EnterSearch,
    ToggleSort,
    Refresh,
    AppendText(String),
}

fn key_pressed_no_repeat(input: &egui::InputState, key: egui::Key) -> bool {
    input.events.iter().any(|e| {
        matches!(e, egui::Event::Key { key: k, pressed: true, repeat: false, .. } if *k == key)
    })
}

// in_search gates all letter keys so that j/k/h/l/s/r/slash fall through
// to AppendText instead of firing navigation/action commands while the user
// is typing a query. Arrow keys, Enter, Escape, and Backspace always work.
fn classify_key(input: &egui::InputState, in_search: bool) -> Option<FileBrowserAction> {
    if key_pressed_no_repeat(input, egui::Key::Escape) {
        return Some(FileBrowserAction::Escape);
    }
    if key_pressed_no_repeat(input, egui::Key::Backspace) {
        return Some(FileBrowserAction::Backspace);
    }
    if input.key_pressed(egui::Key::ArrowDown)
        || (!in_search && input.key_pressed(egui::Key::J) && !input.modifiers.any())
    {
        return Some(FileBrowserAction::SelectNext);
    }
    if input.key_pressed(egui::Key::ArrowUp)
        || (!in_search && input.key_pressed(egui::Key::K) && !input.modifiers.any())
    {
        return Some(FileBrowserAction::SelectPrev);
    }
    if input.key_pressed(egui::Key::Home) {
        return Some(FileBrowserAction::SelectFirst);
    }
    if input.key_pressed(egui::Key::End) {
        return Some(FileBrowserAction::SelectLast);
    }
    if input.key_pressed(egui::Key::PageDown) {
        return Some(FileBrowserAction::PageDown);
    }
    if input.key_pressed(egui::Key::PageUp) {
        return Some(FileBrowserAction::PageUp);
    }
    if !input.modifiers.command
        && (key_pressed_no_repeat(input, egui::Key::Enter)
            || key_pressed_no_repeat(input, egui::Key::ArrowRight)
            || (!in_search && key_pressed_no_repeat(input, egui::Key::L)))
    {
        return Some(FileBrowserAction::Activate);
    }
    if !input.modifiers.command
        && (key_pressed_no_repeat(input, egui::Key::ArrowLeft)
            || (!in_search && key_pressed_no_repeat(input, egui::Key::H)))
    {
        return Some(FileBrowserAction::NavigateUp);
    }
    if !in_search && key_pressed_no_repeat(input, egui::Key::Slash) && !input.modifiers.command {
        return Some(FileBrowserAction::EnterSearch);
    }
    if !in_search && key_pressed_no_repeat(input, egui::Key::S) && !input.modifiers.any() {
        return Some(FileBrowserAction::ToggleSort);
    }
    if !in_search && key_pressed_no_repeat(input, egui::Key::R) && !input.modifiers.any() {
        return Some(FileBrowserAction::Refresh);
    }
    let mut text = String::new();
    for event in &input.events {
        if let egui::Event::Text(t) = event {
            text.push_str(t);
        }
    }
    if !text.is_empty() {
        return Some(FileBrowserAction::AppendText(text));
    }
    None
}

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
    /// In tests: collects paths passed to open_file instead of spawning the system opener.
    #[cfg(test)]
    pub(crate) opened_files: Vec<PathBuf>,
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
            #[cfg(test)]
            opened_files: Vec::new(),
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
        #[cfg(test)]
        {
            self.opened_files.push(path.to_path_buf());
            return;
        }
        #[cfg(not(test))]
        match std::process::Command::new(Self::system_opener()).arg(path).status() {
            Ok(status) if status.success() => log::info!("file_browser: opened '{}'", path.display()),
            Ok(status) => log::error!(
                "file_browser: system-open failed for '{}': {status}",
                path.display()
            ),
            Err(e) => log::error!(
                "file_browser: system-open failed to spawn for '{}': {e}",
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
                colors.bg_sidebar_hover
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

    fn handle_key(&mut self, input: &egui::InputState) -> crate::app_trait::KeyDisposition {
        use crate::app_trait::KeyDisposition;
        let mode = if self.in_search {
            Mode::Search
        } else if self.entries.is_empty() {
            Mode::Empty
        } else {
            Mode::Normal
        };
        let action = classify_key(input, self.in_search);
        log::debug!("file_browser: handle_key mode={mode:?} action={action:?}");

        match (mode, action) {
            // Escape: closes browser in normal/empty, exits search in search mode
            (Mode::Search, Some(FileBrowserAction::Escape)) => {
                self.exit_search();
                KeyDisposition::Consumed
            }
            (_, Some(FileBrowserAction::Escape)) => {
                self.should_close = true;
                KeyDisposition::Consumed
            }
            // NavigateUp (← / H): global — no modal conflict
            (_, Some(FileBrowserAction::NavigateUp)) => {
                self.navigate_up();
                KeyDisposition::Consumed
            }
            // Backspace: delete char in search, navigate up elsewhere
            (Mode::Search, Some(FileBrowserAction::Backspace)) => {
                self.search_query.pop();
                self.refilter();
                KeyDisposition::Consumed
            }
            (_, Some(FileBrowserAction::Backspace)) => {
                self.navigate_up();
                KeyDisposition::Consumed
            }
            // Search mode actions
            (Mode::Search, Some(FileBrowserAction::Activate)) => {
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
                KeyDisposition::Consumed
            }
            (Mode::Search, Some(FileBrowserAction::SelectNext)) => {
                let last = self.search_indices.len().saturating_sub(1);
                self.selected = (self.selected + 1).min(last);
                self.pending_scroll = true;
                KeyDisposition::Consumed
            }
            (Mode::Search, Some(FileBrowserAction::SelectPrev)) => {
                self.selected = self.selected.saturating_sub(1);
                self.pending_scroll = true;
                KeyDisposition::Consumed
            }
            (Mode::Search, Some(FileBrowserAction::AppendText(text))) => {
                self.search_query.push_str(&text);
                self.refilter();
                KeyDisposition::Consumed
            }
            // Search mode consumes all unhandled input
            (Mode::Search, _) => KeyDisposition::Consumed,
            // Empty mode: no further keys handled
            (Mode::Empty, _) => KeyDisposition::Passthrough,
            // Normal mode actions
            (Mode::Normal, Some(FileBrowserAction::EnterSearch)) => {
                self.in_search = true;
                self.search_query.clear();
                self.refilter();
                log::info!("file_browser: entering search mode");
                KeyDisposition::Consumed
            }
            (Mode::Normal, Some(FileBrowserAction::SelectNext)) => {
                let last = self.entries.len().saturating_sub(1);
                self.selected = (self.selected + 1).min(last);
                self.pending_scroll = true;
                KeyDisposition::Consumed
            }
            (Mode::Normal, Some(FileBrowserAction::SelectPrev)) => {
                self.selected = self.selected.saturating_sub(1);
                self.pending_scroll = true;
                KeyDisposition::Consumed
            }
            (Mode::Normal, Some(FileBrowserAction::SelectFirst)) => {
                self.selected = 0;
                self.pending_scroll = true;
                KeyDisposition::Consumed
            }
            (Mode::Normal, Some(FileBrowserAction::SelectLast)) => {
                self.selected = self.entries.len().saturating_sub(1);
                self.pending_scroll = true;
                KeyDisposition::Consumed
            }
            (Mode::Normal, Some(FileBrowserAction::PageDown)) => {
                let last = self.entries.len().saturating_sub(1);
                self.selected = (self.selected + 10).min(last);
                self.pending_scroll = true;
                KeyDisposition::Consumed
            }
            (Mode::Normal, Some(FileBrowserAction::PageUp)) => {
                self.selected = self.selected.saturating_sub(10);
                self.pending_scroll = true;
                KeyDisposition::Consumed
            }
            (Mode::Normal, Some(FileBrowserAction::Activate)) => {
                if let Some(entry) = self.selected_entry().cloned() {
                    if entry.is_dir {
                        self.navigate_into(entry.path);
                    } else {
                        self.open_file(&entry.path);
                    }
                }
                KeyDisposition::Consumed
            }
            (Mode::Normal, Some(FileBrowserAction::ToggleSort)) => {
                self.sort_mode = match self.sort_mode {
                    SortMode::RecentlyTouched => SortMode::Name,
                    SortMode::Name => SortMode::RecentlyTouched,
                };
                self.refresh();
                KeyDisposition::Consumed
            }
            (Mode::Normal, Some(FileBrowserAction::Refresh)) => {
                self.refresh();
                KeyDisposition::Consumed
            }
            _ => KeyDisposition::Passthrough,
        }
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

    fn current_cwd(&self) -> Option<std::path::PathBuf> {
        Some(self.cwd.clone())
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

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Key, Modifiers, RawInput};

    fn key_event(key: Key, modifiers: Modifiers) -> Event {
        Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
    }

    fn run_handle_key(app: &mut FileBrowserApp, events: Vec<Event>) -> bool {
        use crate::app_trait::KeyDisposition;
        let ctx = egui::Context::default();
        let raw = RawInput { events, ..Default::default() };
        let mut consumed = false;
        let _ = ctx.run(raw, |ctx| {
            ctx.input(|i| {
                consumed = app.handle_key(i) == KeyDisposition::Consumed;
            });
        });
        consumed
    }

    fn make_empty_dir_app() -> (FileBrowserApp, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let app = FileBrowserApp::new(dir.path().to_path_buf());
        (app, dir)
    }

    fn make_populated_dir_app() -> (FileBrowserApp, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("file.txt"), b"hi").expect("write");
        let app = FileBrowserApp::new(dir.path().to_path_buf());
        (app, dir)
    }

    // ── Regression tests for issue #926 ──────────────────────────────────────
    // The empty-dir guard previously swallowed Escape, H, and ← without acting.

    #[test]
    fn empty_dir_escape_closes() {
        let (mut app, _dir) = make_empty_dir_app();
        assert!(app.entries.is_empty());
        let consumed = run_handle_key(&mut app, vec![key_event(Key::Escape, Modifiers::default())]);
        assert!(consumed, "Escape must be consumed in empty dir");
        assert!(app.should_close, "Escape must set should_close in empty dir");
    }

    #[test]
    fn empty_dir_arrow_left_navigates_up() {
        let (mut app, _dir) = make_empty_dir_app();
        let parent = app.cwd.parent().map(|p| p.to_path_buf()).unwrap();
        let consumed = run_handle_key(&mut app, vec![key_event(Key::ArrowLeft, Modifiers::default())]);
        assert!(consumed, "← must be consumed in empty dir");
        assert_eq!(app.cwd, parent, "← must navigate to parent in empty dir");
    }

    #[test]
    fn empty_dir_h_navigates_up() {
        let (mut app, _dir) = make_empty_dir_app();
        let parent = app.cwd.parent().map(|p| p.to_path_buf()).unwrap();
        let consumed = run_handle_key(&mut app, vec![key_event(Key::H, Modifiers::default())]);
        assert!(consumed, "H must be consumed in empty dir");
        assert_eq!(app.cwd, parent, "H must navigate to parent in empty dir");
    }

    #[test]
    fn empty_dir_backspace_navigates_up() {
        let (mut app, _dir) = make_empty_dir_app();
        let parent = app.cwd.parent().map(|p| p.to_path_buf()).unwrap();
        let consumed = run_handle_key(&mut app, vec![key_event(Key::Backspace, Modifiers::default())]);
        assert!(consumed);
        assert_eq!(app.cwd, parent);
    }

    #[test]
    fn empty_dir_other_keys_not_consumed() {
        let (mut app, _dir) = make_empty_dir_app();
        let consumed = run_handle_key(&mut app, vec![key_event(Key::ArrowDown, Modifiers::default())]);
        assert!(!consumed, "↓ must not be consumed in empty dir");
    }

    // ── Normal mode smoke tests ───────────────────────────────────────────────

    #[test]
    fn normal_mode_escape_closes() {
        let (mut app, _dir) = make_populated_dir_app();
        run_handle_key(&mut app, vec![key_event(Key::Escape, Modifiers::default())]);
        assert!(app.should_close);
    }

    #[test]
    fn normal_mode_slash_enters_search() {
        let (mut app, _dir) = make_populated_dir_app();
        run_handle_key(&mut app, vec![key_event(Key::Slash, Modifiers::default())]);
        assert!(app.in_search);
    }

    // ── Search mode smoke tests ───────────────────────────────────────────────

    #[test]
    fn search_escape_exits_search() {
        let (mut app, _dir) = make_populated_dir_app();
        app.in_search = true;
        app.search_query = "abc".to_string();
        run_handle_key(&mut app, vec![key_event(Key::Escape, Modifiers::default())]);
        assert!(!app.in_search);
        assert!(!app.should_close, "Escape in search must not close the browser");
    }

    #[test]
    fn handle_key_has_no_key_pressed_calls() {
        // Structural invariant: all key_pressed calls live in classify_key, not handle_key.
        // This test fails at compile time if violated — it's a documentation check.
        // The real enforcement is: grep the source for key_pressed outside classify_key.
        let src = include_str!("mod.rs");
        let classify_start = src.find("fn classify_key").expect("classify_key must exist");
        let handle_key_start = src.find("fn handle_key").expect("handle_key must exist");
        let handle_key_body = &src[handle_key_start..];
        let after_classify = &src[classify_start..handle_key_start];
        // classify_key should contain key_pressed calls (for scroll keys)
        assert!(after_classify.contains("key_pressed"), "classify_key must contain key_pressed calls");
        // classify_key must also use key_pressed_no_repeat for action/activation keys (Escape, Enter, etc.)
        // scroll keys (ArrowDown, ArrowUp, j, k) still use key_pressed to allow repeating
        assert!(after_classify.contains("key_pressed_no_repeat"), "classify_key must use key_pressed_no_repeat for action/activation keys");
        // handle_key body must not contain key_pressed calls
        let handle_body_end = handle_key_body[10..].find("fn ").map(|i| i + 10).unwrap_or(handle_key_body.len());
        let handle_body = &handle_key_body[..handle_body_end];
        assert!(!handle_body.contains("key_pressed"), "handle_key must not contain key_pressed calls — they belong in classify_key");
    }

    #[test]
    fn key_pressed_no_repeat_requires_non_repeat_event() {
        // key_pressed_no_repeat returns false when no events are present.
        let ctx = egui::Context::default();
        let _ = ctx.run(RawInput::default(), |ctx| {
            ctx.input(|i| {
                assert!(!super::key_pressed_no_repeat(i, Key::ArrowRight));
            });
        });
    }

    #[test]
    fn key_pressed_no_repeat_returns_true_for_fresh_press() {
        // egui marks the first press of a key (repeat=false), so key_pressed_no_repeat
        // must return true for a fresh key press event.
        let ctx = egui::Context::default();
        let raw = RawInput {
            events: vec![key_event(Key::ArrowRight, Modifiers::default())],
            ..Default::default()
        };
        let _ = ctx.run(raw, |ctx| {
            ctx.input(|i| {
                assert!(super::key_pressed_no_repeat(i, Key::ArrowRight));
            });
        });
    }

    // ── Enter / l / → open non-directory files (#138) ──────────────────────

    fn make_file_only_dir_app() -> (FileBrowserApp, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("readme.txt"), b"hi").expect("write file");
        let app = FileBrowserApp::new(dir.path().to_path_buf());
        (app, dir)
    }

    fn make_mixed_dir_app() -> (FileBrowserApp, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(dir.path().join("subdir")).expect("create subdir");
        std::fs::write(dir.path().join("notes.txt"), b"hi").expect("write file");
        let app = FileBrowserApp::new(dir.path().to_path_buf());
        (app, dir)
    }

    #[test]
    fn enter_on_file_opens_file_and_browser_stays_open() {
        let (mut app, _dir) = make_file_only_dir_app();
        assert!(!app.entries.is_empty(), "entries must be populated after new()");
        assert!(!app.entries[0].is_dir, "first entry must be a file (no dirs in fixture)");
        let consumed = run_handle_key(&mut app, vec![key_event(Key::Enter, Modifiers::default())]);
        assert!(consumed, "Enter on a file must be consumed");
        assert_eq!(app.opened_files.len(), 1, "Enter on a file must call open_file");
        assert!(app.opened_files[0].ends_with("readme.txt"), "must open the selected file");
        assert!(!app.should_close, "browser must stay open after opening a file");
    }

    #[test]
    fn l_key_on_file_opens_file() {
        let (mut app, _dir) = make_file_only_dir_app();
        let consumed = run_handle_key(&mut app, vec![key_event(Key::L, Modifiers::default())]);
        assert!(consumed);
        assert_eq!(app.opened_files.len(), 1, "l on a file must call open_file");
    }

    #[test]
    fn arrow_right_on_file_opens_file() {
        let (mut app, _dir) = make_file_only_dir_app();
        let consumed = run_handle_key(&mut app, vec![key_event(Key::ArrowRight, Modifiers::default())]);
        assert!(consumed);
        assert_eq!(app.opened_files.len(), 1, "→ on a file must call open_file");
    }

    #[test]
    fn enter_on_dir_navigates_not_opens() {
        let (mut app, _dir) = make_mixed_dir_app();
        // dirs sort first — selected=0 is the subdir
        assert!(app.entries[0].is_dir, "first entry must be the subdirectory");
        let consumed = run_handle_key(&mut app, vec![key_event(Key::Enter, Modifiers::default())]);
        assert!(consumed);
        assert!(app.opened_files.is_empty(), "Enter on a dir must navigate, not open_file");
    }

    #[test]
    fn search_enter_on_file_opens_and_exits_search() {
        let (mut app, _dir) = make_file_only_dir_app();
        // enter search mode
        run_handle_key(&mut app, vec![key_event(Key::Slash, Modifiers::default())]);
        assert!(app.in_search, "slash must enter search mode");
        // press Enter to open the (only) filtered result
        let consumed = run_handle_key(&mut app, vec![key_event(Key::Enter, Modifiers::default())]);
        assert!(consumed);
        assert_eq!(app.opened_files.len(), 1, "Enter in search mode on a file must call open_file");
        assert!(!app.in_search, "search mode must exit after opening a file");
    }

    // ── Search mode: letter keys must append to query, not fire vim actions ──
    // Regression tests for issue #258.

    #[test]
    fn search_mode_h_does_not_navigate_up() {
        let (mut app, _dir) = make_populated_dir_app();
        let original_cwd = app.cwd.clone();
        app.in_search = true;
        app.refilter();
        run_handle_key(&mut app, vec![key_event(Key::H, Modifiers::default())]);
        assert_eq!(app.cwd, original_cwd, "H in search mode must not navigate to parent");
    }

    #[test]
    fn search_mode_h_appends_to_query() {
        let (mut app, _dir) = make_populated_dir_app();
        app.in_search = true;
        app.refilter();
        run_handle_key(&mut app, vec![
            key_event(Key::H, Modifiers::default()),
            egui::Event::Text("h".to_string()),
        ]);
        assert_eq!(app.search_query, "h", "H in search mode must append to query");
    }

    #[test]
    fn search_mode_l_does_not_activate() {
        let (mut app, _dir) = make_file_only_dir_app();
        app.in_search = true;
        app.refilter();
        run_handle_key(&mut app, vec![key_event(Key::L, Modifiers::default())]);
        assert!(app.opened_files.is_empty(), "L in search mode must not open a file");
    }

    #[test]
    fn search_mode_l_appends_to_query() {
        let (mut app, _dir) = make_populated_dir_app();
        app.in_search = true;
        app.refilter();
        run_handle_key(&mut app, vec![
            key_event(Key::L, Modifiers::default()),
            egui::Event::Text("l".to_string()),
        ]);
        assert_eq!(app.search_query, "l", "L in search mode must append to query");
    }

    #[test]
    fn search_mode_j_appends_to_query() {
        let (mut app, _dir) = make_populated_dir_app();
        app.in_search = true;
        app.refilter();
        run_handle_key(&mut app, vec![
            key_event(Key::J, Modifiers::default()),
            egui::Event::Text("j".to_string()),
        ]);
        assert_eq!(app.search_query, "j", "J in search mode must append to query");
    }

    #[test]
    fn search_mode_k_appends_to_query() {
        let (mut app, _dir) = make_populated_dir_app();
        app.in_search = true;
        app.refilter();
        run_handle_key(&mut app, vec![
            key_event(Key::K, Modifiers::default()),
            egui::Event::Text("k".to_string()),
        ]);
        assert_eq!(app.search_query, "k", "K in search mode must append to query");
    }

    #[test]
    fn search_mode_s_does_not_toggle_sort() {
        let (mut app, _dir) = make_populated_dir_app();
        let original_sort = app.sort_mode;
        app.in_search = true;
        app.refilter();
        run_handle_key(&mut app, vec![
            key_event(Key::S, Modifiers::default()),
            egui::Event::Text("s".to_string()),
        ]);
        assert_eq!(app.sort_mode, original_sort, "S in search mode must not toggle sort");
        assert_eq!(app.search_query, "s", "S in search mode must append to query");
    }

    #[test]
    fn search_mode_r_appends_to_query() {
        let (mut app, _dir) = make_populated_dir_app();
        app.in_search = true;
        app.refilter();
        run_handle_key(&mut app, vec![
            key_event(Key::R, Modifiers::default()),
            egui::Event::Text("r".to_string()),
        ]);
        assert_eq!(app.search_query, "r", "R in search mode must append to query");
    }

    #[test]
    fn arrow_down_still_selects_next_in_search() {
        let (mut app, _dir) = make_populated_dir_app();
        // add a second file so there's something to move to
        std::fs::write(app.cwd.join("other.txt"), b"x").expect("write");
        app.refresh();
        app.in_search = true;
        app.refilter();
        assert!(app.search_indices.len() >= 2, "need at least 2 entries for this test");
        let before = app.selected;
        run_handle_key(&mut app, vec![key_event(Key::ArrowDown, Modifiers::default())]);
        assert_eq!(app.selected, before + 1, "↓ must still move selection in search mode");
        assert_eq!(app.search_query, "", "↓ must not append to query");
    }
}
