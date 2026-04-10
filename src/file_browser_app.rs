use crate::app_trait::{App, AppCommand, AppRenderContext};
use crate::theme::Colors;
use egui::{Color32, CornerRadius, Stroke, StrokeKind};
use image::imageops::FilterType;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::SystemTime;

const ROW_HEIGHT: f32 = 58.0;
const MIN_SIDEBAR_WIDTH: f32 = 920.0;

#[derive(Clone)]
struct Entry {
    name: String,
    path: PathBuf,
    is_dir: bool,
    is_image: bool,
    is_audio: bool,
    size_bytes: Option<u64>,
    modified: Option<SystemTime>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SortMode {
    RecentlyTouched,
    Name,
}

#[derive(Clone, Copy)]
struct DirStats {
    file_count: usize,
    dir_count: usize,
    total_bytes: u64,
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
    // Text preview
    text_preview_path: Option<PathBuf>,
    text_preview_body: Option<String>,
    // Dir preview
    dir_preview_path: Option<PathBuf>,
    dir_preview_stats: Option<DirStats>,
    pending_cmds: Vec<AppCommand>,
    /// When true, the next draw_list pass will scroll the selected row into view.
    pending_scroll: bool,
    /// Remembers which entry was selected when leaving a directory,
    /// so navigating back restores the selection.
    directory_selection_memory: std::collections::HashMap<PathBuf, String>,
    // Audio preview
    audio_tx: mpsc::Sender<AudioMsg>,
    audio_playing_path: Option<PathBuf>,
    audio_playing: bool,
    audio_play_started: Option<std::time::Instant>,
    audio_elapsed_before_pause: f32,
    audio_paused: bool,
}

enum AudioMsg {
    Play(PathBuf),
    Pause,
    Resume,
    Stop,
}

impl FileBrowserApp {
    pub fn new(cwd: PathBuf) -> Self {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || audio_thread(rx));

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
            text_preview_path: None,
            text_preview_body: None,
            dir_preview_path: None,
            dir_preview_stats: None,
            pending_cmds: Vec::new(),
            pending_scroll: false,
            directory_selection_memory: std::collections::HashMap::new(),
            audio_tx: tx,
            audio_playing_path: None,
            audio_playing: false,
            audio_play_started: None,
            audio_elapsed_before_pause: 0.0,
            audio_paused: false,
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
                        let is_dir = path.is_dir();
                        let meta = e.metadata().ok();
                        let size_bytes = meta.as_ref().and_then(|m| {
                            if is_dir { None } else { Some(m.len()) }
                        });
                        let modified = meta.as_ref().and_then(|m| m.modified().ok());
                        let ext = path
                            .extension()
                            .and_then(|e| e.to_str())
                            .map(|e| e.to_ascii_lowercase());
                        let is_image = ext
                            .as_deref()
                            .map(|e| matches!(e, "png" | "jpg" | "jpeg" | "gif" | "bmp" | "tiff" | "webp"))
                            .unwrap_or(false);
                        let is_audio = ext
                            .as_deref()
                            .map(|e| matches!(e, "mp3" | "wav" | "flac" | "ogg" | "aiff" | "aif" | "m4a"))
                            .unwrap_or(false);
                        Some(Entry { name, path, is_dir, is_image, is_audio, size_bytes, modified })
                    })
                    .collect();

                match self.sort_mode {
                    SortMode::Name => {
                        entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then(a.name.cmp(&b.name)));
                    }
                    SortMode::RecentlyTouched => {
                        entries.sort_by(|a, b| {
                            b.is_dir.cmp(&a.is_dir).then(
                                b.modified
                                    .cmp(&a.modified)
                                    .then(a.name.cmp(&b.name)),
                            )
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
        // Remember current selection so we can restore it when coming back.
        if let Some(entry) = self.selected_entry() {
            self.directory_selection_memory
                .insert(self.cwd.clone(), entry.name.clone());
        }
        self.cwd = path.clone();
        self.selected = 0;
        self.refresh();
        self.pending_scroll = true;
        self.pending_cmds.push(AppCommand::Cd(path));
    }

    fn navigate_up(&mut self) {
        if let Some(parent) = self.cwd.parent().map(|p| p.to_path_buf()) {
            // Capture the name of the directory we're leaving BEFORE changing cwd.
            let leaving_name = self.cwd
                .file_name()
                .map(|n| n.to_string_lossy().to_string());
            self.cwd = parent.clone();
            self.selected = 0;
            self.refresh();
            // Restore selection: either from memory or the directory we just left.
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
            self.pending_cmds.push(AppCommand::Cd(parent));
        }
    }

    fn selected_entry(&self) -> Option<&Entry> {
        self.entries.get(self.selected)
    }

    /// Called by the host when the linked terminal's CWD changes.
    /// Updates the file browser to show the new directory.
    /// Skips if the directory matches what we already navigated to internally
    /// (avoids resetting selection from our own cd commands).
    pub fn sync_cwd(&mut self, new_cwd: PathBuf) {
        if new_cwd == self.cwd {
            return;
        }
        // Remember selection in the old directory.
        if let Some(entry) = self.selected_entry() {
            self.directory_selection_memory
                .insert(self.cwd.clone(), entry.name.clone());
        }
        // Try to restore selection for the new directory.
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

    fn ensure_text_preview(&mut self, path: &Path) {
        if self.text_preview_path.as_deref() == Some(path) {
            return;
        }
        self.text_preview_path = Some(path.to_path_buf());
        let mut file = match fs::File::open(path) {
            Ok(f) => f,
            Err(e) => {
                self.text_preview_body = Some(format!("Error: {e}"));
                return;
            }
        };
        use std::io::Read;
        let mut buf = [0u8; 4096];
        let n = file.read(&mut buf).unwrap_or(0);
        let text = String::from_utf8_lossy(&buf[..n]).into_owned();
        self.text_preview_body = Some(text);
    }

    fn ensure_dir_preview(&mut self, path: &Path) {
        if self.dir_preview_path.as_deref() == Some(path) {
            return;
        }
        self.dir_preview_path = Some(path.to_path_buf());
        let mut file_count = 0usize;
        let mut dir_count = 0usize;
        let mut total_bytes = 0u64;
        if let Ok(entries) = fs::read_dir(path) {
            for e in entries.flatten() {
                let is_dir = e.path().is_dir();
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
        self.dir_preview_stats = Some(DirStats { file_count, dir_count, total_bytes });
    }

    fn draw_list(&mut self, ui: &mut egui::Ui, colors: &Colors) -> Option<PathBuf> {
        let mut navigate_to: Option<PathBuf> = None;
        let should_scroll = self.pending_scroll;
        self.pending_scroll = false;
        let entry_count = self.entries.len();
        for idx in 0..entry_count {
            let entry = self.entries[idx].clone();
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
                    if is_selected { colors.accent } else { colors.border },
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
                format!("{sub} · {}", format_modified(entry.modified)),
                egui::FontId::proportional(9.5),
                colors.text_dim,
            );

            if resp.clicked() {
                self.selected = idx;
            }
            if resp.double_clicked() {
                self.selected = idx;
                navigate_to = Some(entry.path.clone());
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
        } else if entry.is_audio {
            self.draw_audio_sidebar(ui, colors, &entry);
        } else if entry.is_dir {
            self.draw_dir_sidebar(ui, colors, &entry);
        } else if is_text_file(&entry.path) {
            self.draw_text_sidebar(ui, colors, &entry);
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
                    egui::RichText::new(format!("Image · {}", entry.name))
                        .size(10.5)
                        .color(colors.text_primary)
                        .strong(),
                );
                if let Some([w, h]) = self.preview_size {
                    ui.label(
                        egui::RichText::new(format!("{w}×{h}"))
                            .size(10.0)
                            .color(colors.text_dim),
                    );
                }
                ui.add_space(6.0);
                let preview_max = egui::vec2(ui.available_width(), 220.0);
                let (slot_rect, _) = ui.allocate_exact_size(preview_max, egui::Sense::hover());
                ui.painter().rect_filled(slot_rect, CornerRadius::same(4), colors.bg_darkest.gamma_multiply(0.95));
                ui.painter().rect_stroke(slot_rect, CornerRadius::same(4), Stroke::new(1.0, colors.border), StrokeKind::Inside);
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
                    ui.painter().text(slot_rect.center(), egui::Align2::CENTER_CENTER, err, egui::FontId::proportional(10.0), Color32::from_rgb(0xff, 0xaf, 0xaf));
                } else {
                    ui.painter().text(slot_rect.center(), egui::Align2::CENTER_CENTER, "Loading…", egui::FontId::proportional(10.0), colors.text_dim);
                }
                ui.add_space(6.0);
                if let Some(size) = entry.size_bytes {
                    ui.label(egui::RichText::new(format_size(Some(size))).size(9.5).color(colors.text_dim));
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
                    egui::RichText::new(format!("Folder · {}", entry.name))
                        .size(10.5)
                        .color(colors.text_primary)
                        .strong(),
                );
                ui.separator();
                ui.label(egui::RichText::new(format!("Path: {}", entry.path.display())).size(9.5).color(colors.text_dim));
                ui.label(
                    egui::RichText::new(format!(
                        "Contains: {} folders, {} files",
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

    fn draw_text_sidebar(&mut self, ui: &mut egui::Ui, colors: &Colors, entry: &Entry) {
        let path = entry.path.clone();
        self.ensure_text_preview(&path);
        let body = self.text_preview_body.clone().unwrap_or_default();

        egui::Frame::new()
            .fill(colors.bg_sidebar)
            .stroke(Stroke::new(1.0, colors.border))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new(format!("Preview · {}", entry.name))
                        .size(10.5)
                        .color(colors.text_primary)
                        .strong(),
                );
                ui.separator();
                egui::ScrollArea::vertical()
                    .max_height(280.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.label(
                            egui::RichText::new(&body)
                                .size(9.5)
                                .color(colors.text_dim)
                                .monospace(),
                        );
                    });
            });
    }

    fn draw_audio_sidebar(&mut self, ui: &mut egui::Ui, colors: &Colors, entry: &Entry) {
        let is_this_playing = self.audio_playing_path.as_ref() == Some(&entry.path);

        // Request repaints while playing for elapsed counter.
        if is_this_playing && self.audio_playing {
            ui.ctx().request_repaint();
        }

        egui::Frame::new()
            .fill(colors.bg_sidebar)
            .stroke(Stroke::new(1.0, colors.border))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new(format!("Audio · {}", entry.name))
                        .size(10.5)
                        .color(colors.text_primary)
                        .strong(),
                );
                if let Some(size) = entry.size_bytes {
                    ui.label(egui::RichText::new(format_size(Some(size))).size(9.5).color(colors.text_dim));
                }
                ui.add_space(8.0);

                // Play/pause button
                let button_label = if is_this_playing && self.audio_playing {
                    "⏸ Pause"
                } else if is_this_playing && self.audio_paused {
                    "▶ Resume"
                } else {
                    "▶ Play"
                };
                if ui.button(button_label).clicked() {
                    if is_this_playing && self.audio_playing {
                        self.audio_pause();
                    } else if is_this_playing && self.audio_paused {
                        self.audio_resume();
                    } else {
                        self.audio_play(&entry.path);
                    }
                }

                // Stop button (only when something is playing/paused)
                if is_this_playing && (self.audio_playing || self.audio_paused) {
                    if ui.button("⏹ Stop").clicked() {
                        self.audio_stop();
                    }
                }

                // Elapsed time
                if is_this_playing {
                    let elapsed = self.audio_elapsed();
                    let mins = (elapsed / 60.0) as u32;
                    let secs = (elapsed % 60.0) as u32;
                    let state = if self.audio_playing { "Playing" } else { "Paused" };
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new(format!("{state} — {mins}:{secs:02}"))
                            .size(11.0)
                            .color(colors.accent)
                            .monospace(),
                    );
                }

                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new("Space to play/pause")
                        .size(9.0)
                        .color(colors.text_dim),
                );
            });
    }

    fn audio_play(&mut self, path: &Path) {
        let _ = self.audio_tx.send(AudioMsg::Stop);
        let _ = self.audio_tx.send(AudioMsg::Play(path.to_path_buf()));
        self.audio_playing_path = Some(path.to_path_buf());
        self.audio_playing = true;
        self.audio_paused = false;
        self.audio_play_started = Some(std::time::Instant::now());
        self.audio_elapsed_before_pause = 0.0;
    }

    fn audio_pause(&mut self) {
        let _ = self.audio_tx.send(AudioMsg::Pause);
        self.audio_playing = false;
        self.audio_paused = true;
        if let Some(started) = self.audio_play_started {
            self.audio_elapsed_before_pause += started.elapsed().as_secs_f32();
        }
        self.audio_play_started = None;
    }

    fn audio_resume(&mut self) {
        let _ = self.audio_tx.send(AudioMsg::Resume);
        self.audio_playing = true;
        self.audio_paused = false;
        self.audio_play_started = Some(std::time::Instant::now());
    }

    fn audio_stop(&mut self) {
        let _ = self.audio_tx.send(AudioMsg::Stop);
        self.audio_playing = false;
        self.audio_paused = false;
        self.audio_playing_path = None;
        self.audio_play_started = None;
        self.audio_elapsed_before_pause = 0.0;
    }

    fn audio_elapsed(&self) -> f32 {
        let current = self.audio_play_started
            .map(|s| s.elapsed().as_secs_f32())
            .unwrap_or(0.0);
        self.audio_elapsed_before_pause + current
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
                    ui.label(egui::RichText::new(format_size(Some(size))).size(9.5).color(colors.text_dim));
                }
                if let Some(modified) = entry.modified {
                    ui.label(egui::RichText::new(format!("Modified: {}", format_modified(Some(modified)))).size(9.5).color(colors.text_dim));
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
                // Header bar
                ui.horizontal(|ui| {
                    ui.colored_label(
                        colors.accent,
                        egui::RichText::new(self.cwd.display().to_string())
                            .size(11.0)
                            .monospace(),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let name_label = if self.sort_mode == SortMode::Name { "Name ✓" } else { "Name" };
                        let recent_label = if self.sort_mode == SortMode::RecentlyTouched { "Recent ✓" } else { "Recent" };
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
                let mut navigate_to: Option<PathBuf> = None;

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

                if let Some(path) = navigate_to {
                    if path.is_dir() {
                        self.navigate_into(path);
                    } else {
                        let _ = std::process::Command::new("open").arg(&path).spawn();
                    }
                }
            });
    }

    fn handle_key(&mut self, input: &egui::InputState) -> bool {
        if self.entries.is_empty() {
            // Backspace to go up even when empty.
            if input.key_pressed(egui::Key::Backspace) {
                self.navigate_up();
                return true;
            }
            return false;
        }

        let last = self.entries.len().saturating_sub(1);
        let mut consumed = false;

        // Navigation
        if input.key_pressed(egui::Key::ArrowDown) || input.key_pressed(egui::Key::J) {
            self.selected = (self.selected + 1).min(last);
            self.pending_scroll = true;
            consumed = true;
        }
        if input.key_pressed(egui::Key::ArrowUp) || input.key_pressed(egui::Key::K) {
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

        // Enter / open — but NOT when Cmd is held (Cmd+Enter = Plexi zoom).
        if !input.modifiers.command && (input.key_pressed(egui::Key::Enter) || input.key_pressed(egui::Key::ArrowRight) || input.key_pressed(egui::Key::L)) {
            if let Some(entry) = self.selected_entry().cloned() {
                if entry.is_dir {
                    self.navigate_into(entry.path);
                } else {
                    let _ = std::process::Command::new("open").arg(&entry.path).spawn();
                }
            }
            consumed = true;
        }

        // Back / parent — but NOT when Cmd is held (Cmd+HJKL = pane navigation).
        if !input.modifiers.command && (input.key_pressed(egui::Key::Backspace) || input.key_pressed(egui::Key::ArrowLeft) || input.key_pressed(egui::Key::H)) {
            self.navigate_up();
            consumed = true;
        }

        // Sort toggle
        if input.key_pressed(egui::Key::S) {
            self.sort_mode = match self.sort_mode {
                SortMode::RecentlyTouched => SortMode::Name,
                SortMode::Name => SortMode::RecentlyTouched,
            };
            self.refresh();
            consumed = true;
        }

        // Refresh
        if input.key_pressed(egui::Key::R) {
            self.refresh();
            consumed = true;
        }

        // Space: toggle audio play/pause on selected audio file
        if input.key_pressed(egui::Key::Space) {
            if let Some(entry) = self.selected_entry().cloned() {
                if entry.is_audio {
                    let is_this = self.audio_playing_path.as_ref() == Some(&entry.path);
                    if is_this && self.audio_playing {
                        self.audio_pause();
                    } else if is_this && self.audio_paused {
                        self.audio_resume();
                    } else {
                        self.audio_play(&entry.path);
                    }
                    consumed = true;
                }
            }
        }

        consumed
    }

    fn take_pending_commands(&mut self) -> Vec<AppCommand> {
        std::mem::take(&mut self.pending_cmds)
    }

    fn on_command(&mut self, cmd: &str) -> Option<AppCommand> {
        let parts: Vec<&str> = cmd.trim().splitn(2, ' ').collect();
        match parts.as_slice() {
            ["cd", path] => {
                let target = PathBuf::from(path);
                let target = if target.is_absolute() { target } else { self.cwd.join(target) };
                if target.is_dir() {
                    self.navigate_into(target.clone());
                    return Some(AppCommand::Cd(target));
                }
                None
            }
            _ => Some(AppCommand::RunInTerminal(cmd.to_string())),
        }
    }

    fn sync_cwd(&mut self, new_cwd: &std::path::Path) {
        self.sync_cwd(new_cwd.to_path_buf());
    }

    fn accepted_extensions(&self) -> &[&str] {
        &[]
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

// ─── Icon painting ────────────────────────────────────────────────────────────

enum FileIconKind {
    Image,
    Audio,
    Markdown,
    Text,
    Code,
    Config,
    Pdf,
    Archive,
    Generic,
}

fn file_icon_kind(entry: &Entry) -> FileIconKind {
    if entry.is_image { return FileIconKind::Image; }
    if entry.is_audio { return FileIconKind::Audio; }
    let Some(ext) = entry.path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()) else {
        return FileIconKind::Generic;
    };
    match ext.as_str() {
        "md" | "markdown" | "mdx" | "rst" => FileIconKind::Markdown,
        "txt" | "rtf" | "log" => FileIconKind::Text,
        "rs" | "py" | "js" | "jsx" | "ts" | "tsx" | "go" | "java" | "swift" | "kt"
        | "c" | "h" | "cpp" | "hpp" | "sh" | "zsh" | "bash" | "fish" | "lua" | "rb" => FileIconKind::Code,
        "toml" | "yaml" | "yml" | "json" | "jsonc" | "json5" | "ini" | "cfg" | "conf"
        | "env" | "plist" => FileIconKind::Config,
        "pdf" => FileIconKind::Pdf,
        "zip" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "7z" | "rar" => FileIconKind::Archive,
        _ => FileIconKind::Generic,
    }
}

fn paint_entry_icon(painter: &egui::Painter, rect: egui::Rect, entry: &Entry, colors: &Colors) {
    if entry.is_dir {
        let tab = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 1.0, rect.top() + 2.0),
            egui::vec2(rect.width() * 0.45, rect.height() * 0.3),
        );
        let body = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 1.0, rect.top() + rect.height() * 0.25),
            egui::vec2(rect.width() - 2.0, rect.height() * 0.7),
        );
        painter.rect_filled(tab, CornerRadius::same(2), colors.accent.gamma_multiply(0.7));
        painter.rect_filled(body, CornerRadius::same(2), colors.accent.gamma_multiply(0.9));
        return;
    }

    let sheet = rect.shrink(1.0);
    let fold = (sheet.width().min(sheet.height()) * 0.30).clamp(4.0, 18.0);
    let stroke_w = (sheet.width().min(sheet.height()) * 0.10).clamp(1.0, 2.4);

    painter.rect_filled(sheet, CornerRadius::same(2), colors.text_dim.gamma_multiply(0.34));
    painter.rect_stroke(sheet, CornerRadius::same(2), Stroke::new(1.0, colors.border), StrokeKind::Inside);
    let fold_poly = vec![
        egui::pos2(sheet.right() - fold, sheet.top()),
        egui::pos2(sheet.right(), sheet.top()),
        egui::pos2(sheet.right(), sheet.top() + fold),
    ];
    painter.add(egui::Shape::convex_polygon(fold_poly, colors.bg_active.gamma_multiply(0.75), Stroke::new(1.0, colors.border)));

    let x = |t: f32| sheet.left() + sheet.width() * t;
    let y = |t: f32| sheet.top() + sheet.height() * t;
    let kind = file_icon_kind(entry);

    match kind {
        FileIconKind::Image => {
            let sky = Color32::from_rgb(0x89, 0xb4, 0xfa);
            let points = [(0.18, 0.78), (0.36, 0.52), (0.54, 0.72), (0.80, 0.42)];
            for w in points.windows(2) {
                painter.line_segment(
                    [egui::pos2(x(w[0].0), y(w[0].1)), egui::pos2(x(w[1].0), y(w[1].1))],
                    Stroke::new(stroke_w, sky),
                );
            }
            painter.circle_filled(egui::pos2(x(0.76), y(0.26)), (sheet.width().min(sheet.height()) * 0.09).max(1.5), sky.gamma_multiply(0.9));
        }
        FileIconKind::Audio => {
            let c = Color32::from_rgb(0xa6, 0xe3, 0xa1);
            painter.add(egui::Shape::convex_polygon(
                vec![
                    egui::pos2(x(0.26), y(0.50)), egui::pos2(x(0.36), y(0.40)),
                    egui::pos2(x(0.47), y(0.40)), egui::pos2(x(0.47), y(0.68)),
                    egui::pos2(x(0.36), y(0.68)), egui::pos2(x(0.26), y(0.58)),
                ],
                c,
                Stroke::new(0.0, Color32::TRANSPARENT),
            ));
            painter.line_segment([egui::pos2(x(0.56), y(0.44)), egui::pos2(x(0.66), y(0.54))], Stroke::new(stroke_w, c));
            painter.line_segment([egui::pos2(x(0.56), y(0.64)), egui::pos2(x(0.66), y(0.54))], Stroke::new(stroke_w, c));
            painter.line_segment([egui::pos2(x(0.68), y(0.38)), egui::pos2(x(0.80), y(0.54))], Stroke::new(stroke_w, c.gamma_multiply(0.9)));
            painter.line_segment([egui::pos2(x(0.68), y(0.70)), egui::pos2(x(0.80), y(0.54))], Stroke::new(stroke_w, c.gamma_multiply(0.9)));
        }
        FileIconKind::Markdown | FileIconKind::Text => {
            let c = Color32::from_rgb(0xf9, 0xe2, 0xaf);
            painter.line_segment([egui::pos2(x(0.28), y(0.74)), egui::pos2(x(0.72), y(0.30))], Stroke::new(stroke_w * 1.15, c));
            painter.add(egui::Shape::convex_polygon(
                vec![egui::pos2(x(0.70), y(0.26)), egui::pos2(x(0.80), y(0.20)), egui::pos2(x(0.74), y(0.30))],
                c, Stroke::new(0.0, Color32::TRANSPARENT),
            ));
            if matches!(kind, FileIconKind::Markdown) {
                painter.line_segment([egui::pos2(x(0.26), y(0.26)), egui::pos2(x(0.54), y(0.26))], Stroke::new(stroke_w, c.gamma_multiply(0.95)));
            }
        }
        FileIconKind::Code => {
            let c = Color32::from_rgb(0x94, 0xe2, 0xd5);
            painter.line_segment([egui::pos2(x(0.38), y(0.34)), egui::pos2(x(0.24), y(0.52))], Stroke::new(stroke_w, c));
            painter.line_segment([egui::pos2(x(0.24), y(0.52)), egui::pos2(x(0.38), y(0.70))], Stroke::new(stroke_w, c));
            painter.line_segment([egui::pos2(x(0.62), y(0.34)), egui::pos2(x(0.76), y(0.52))], Stroke::new(stroke_w, c));
            painter.line_segment([egui::pos2(x(0.76), y(0.52)), egui::pos2(x(0.62), y(0.70))], Stroke::new(stroke_w, c));
            painter.line_segment([egui::pos2(x(0.52), y(0.34)), egui::pos2(x(0.46), y(0.70))], Stroke::new(stroke_w * 0.9, c.gamma_multiply(0.85)));
        }
        FileIconKind::Config => {
            let c = Color32::from_rgb(0xb4, 0xbe, 0xfe);
            painter.line_segment([egui::pos2(x(0.22), y(0.38)), egui::pos2(x(0.78), y(0.38))], Stroke::new(stroke_w, c));
            painter.circle_filled(egui::pos2(x(0.42), y(0.38)), (stroke_w * 1.2).max(1.6), c);
            painter.line_segment([egui::pos2(x(0.22), y(0.56)), egui::pos2(x(0.78), y(0.56))], Stroke::new(stroke_w, c));
            painter.circle_filled(egui::pos2(x(0.62), y(0.56)), (stroke_w * 1.2).max(1.6), c);
        }
        FileIconKind::Pdf => {
            let c = Color32::from_rgb(0xf3, 0x8b, 0xa8);
            let band = egui::Rect::from_min_size(
                egui::pos2(x(0.16), y(0.20)),
                egui::vec2(sheet.width() * 0.68, sheet.height() * 0.20),
            );
            painter.rect_filled(band, CornerRadius::same(2), c.gamma_multiply(0.95));
            painter.text(band.center(), egui::Align2::CENTER_CENTER, "PDF", egui::FontId::proportional((sheet.height() * 0.18).max(6.0)), Color32::from_rgb(0x1e, 0x1e, 0x2e));
        }
        FileIconKind::Archive => {
            let c = Color32::from_rgb(0xfa, 0xb3, 0x87);
            let box_rect = egui::Rect::from_min_size(egui::pos2(x(0.26), y(0.30)), egui::vec2(sheet.width() * 0.48, sheet.height() * 0.46));
            painter.rect_stroke(box_rect, CornerRadius::same(2), Stroke::new(stroke_w, c), StrokeKind::Inside);
            painter.line_segment([egui::pos2(box_rect.center().x, box_rect.top()), egui::pos2(box_rect.center().x, box_rect.bottom())], Stroke::new(stroke_w * 0.9, c));
            painter.line_segment([egui::pos2(box_rect.left(), box_rect.center().y), egui::pos2(box_rect.right(), box_rect.center().y)], Stroke::new(stroke_w * 0.9, c));
        }
        FileIconKind::Generic => {
            let c = colors.text_primary.gamma_multiply(0.8);
            painter.line_segment([egui::pos2(x(0.24), y(0.38)), egui::pos2(x(0.70), y(0.38))], Stroke::new(stroke_w, c));
            painter.line_segment([egui::pos2(x(0.24), y(0.58)), egui::pos2(x(0.60), y(0.58))], Stroke::new(stroke_w, c));
        }
    }
}

// ─── Formatting helpers ───────────────────────────────────────────────────────

fn format_size(bytes: Option<u64>) -> String {
    match bytes {
        None => "—".to_string(),
        Some(b) if b < 1024 => format!("{b} B"),
        Some(b) if b < 1024 * 1024 => format!("{:.1} KB", b as f64 / 1024.0),
        Some(b) if b < 1024 * 1024 * 1024 => format!("{:.1} MB", b as f64 / (1024.0 * 1024.0)),
        Some(b) => format!("{:.1} GB", b as f64 / (1024.0 * 1024.0 * 1024.0)),
    }
}

fn format_modified(modified: Option<SystemTime>) -> String {
    let Some(modified) = modified else { return "—".to_string() };
    let Ok(elapsed) = SystemTime::now().duration_since(modified) else { return "—".to_string() };
    let secs = elapsed.as_secs();
    if secs < 60 {
        "just now".to_string()
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else if secs < 86400 * 7 {
        format!("{}d ago", secs / 86400)
    } else if secs < 86400 * 30 {
        format!("{}w ago", secs / (86400 * 7))
    } else {
        format!("{}mo ago", secs / (86400 * 30))
    }
}

fn is_text_file(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()) else {
        return false;
    };
    matches!(
        ext.as_str(),
        "txt" | "md" | "markdown" | "rs" | "py" | "js" | "jsx" | "ts" | "tsx" | "go"
        | "java" | "swift" | "kt" | "c" | "h" | "cpp" | "hpp" | "sh" | "zsh" | "bash"
        | "fish" | "lua" | "rb" | "toml" | "yaml" | "yml" | "json" | "jsonc" | "json5"
        | "ini" | "cfg" | "conf" | "env" | "log" | "rtf" | "rst" | "mdx"
    )
}

/// Audio thread — owns rodio OutputStream and Sink (not Send).
fn audio_thread(rx: mpsc::Receiver<AudioMsg>) {
    let Ok((_stream, handle)) = rodio::OutputStream::try_default() else {
        log::error!("FileBrowser audio: failed to open output stream");
        return;
    };
    let mut sink: Option<rodio::Sink> = None;

    loop {
        let msg = match rx.recv() {
            Ok(m) => m,
            Err(_) => break,
        };
        match msg {
            AudioMsg::Play(path) => {
                if let Some(s) = sink.take() {
                    s.stop();
                }
                let Ok(file) = std::fs::File::open(&path) else { continue };
                let Ok(source) = rodio::Decoder::new(std::io::BufReader::new(file)) else { continue };
                let Ok(new_sink) = rodio::Sink::try_new(&handle) else { continue };
                new_sink.append(source);
                sink = Some(new_sink);
            }
            AudioMsg::Pause => {
                if let Some(s) = &sink { s.pause(); }
            }
            AudioMsg::Resume => {
                if let Some(s) = &sink { s.play(); }
            }
            AudioMsg::Stop => {
                if let Some(s) = sink.take() { s.stop(); }
            }
        }
    }
}
