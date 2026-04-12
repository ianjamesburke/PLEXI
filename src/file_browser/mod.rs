mod audio;
mod helpers;
mod icons;

use crate::app_trait::{App, AppCommand, AppRenderContext};
use crate::theme::Colors;
use egui::{Color32, CornerRadius, Stroke, StrokeKind};
use image::imageops::FilterType;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::SystemTime;

use audio::AudioMsg;
use helpers::{format_modified, format_size, is_text_file, DirStats, Entry, SortMode};
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
    // Text preview / inline editor
    text_preview_path: Option<PathBuf>,
    text_preview_body: Option<String>,
    text_preview_dirty: bool,
    text_preview_saved_body: Option<String>,
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

impl FileBrowserApp {
    pub fn new(cwd: PathBuf) -> Self {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || audio::audio_thread(rx));

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
            text_preview_dirty: false,
            text_preview_saved_body: None,
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
                        let is_dir = e.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
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
            let leaving_name = self.cwd
                .file_name()
                .map(|n| n.to_string_lossy().to_string());
            self.cwd = parent.clone();
            self.selected = 0;
            self.refresh();
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

    fn ensure_text_preview(&mut self, path: &Path) {
        if self.text_preview_path.as_deref() == Some(path) {
            return;
        }
        // Save any dirty content before switching files.
        self.save_text_preview_if_dirty();
        self.text_preview_path = Some(path.to_path_buf());
        self.text_preview_dirty = false;
        match fs::read_to_string(path) {
            Ok(text) => {
                self.text_preview_saved_body = Some(text.clone());
                self.text_preview_body = Some(text);
            }
            Err(e) => {
                self.text_preview_body = Some(format!("Error: {e}"));
                self.text_preview_saved_body = None;
            }
        }
    }

    fn save_text_preview_if_dirty(&mut self) {
        if !self.text_preview_dirty {
            return;
        }
        if let (Some(path), Some(content)) = (&self.text_preview_path, &self.text_preview_body) {
            if let Err(e) = fs::write(path, content) {
                log::error!("FileBrowser: failed to save {}: {e}", path.display());
            } else {
                self.text_preview_saved_body = Some(content.clone());
                self.text_preview_dirty = false;
            }
        }
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
        self.dir_preview_stats = Some(DirStats { file_count, dir_count, total_bytes, truncated });
    }

    // ─── Drawing ─────────────────────────────────────────────────────────────

    fn draw_list(&mut self, ui: &mut egui::Ui, colors: &Colors) -> Option<(PathBuf, bool)> {
        let mut navigate_to: Option<(PathBuf, bool)> = None;
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
                colors.list_item_hover
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
                    ui.painter().text(slot_rect.center(), egui::Align2::CENTER_CENTER, "Loading\u{2026}", egui::FontId::proportional(10.0), colors.text_dim);
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
                    egui::RichText::new(format!("Folder \u{00b7} {}", entry.name))
                        .size(10.5)
                        .color(colors.text_primary)
                        .strong(),
                );
                ui.separator();
                ui.label(egui::RichText::new(format!("Path: {}", entry.path.display())).size(9.5).color(colors.text_dim));
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

    fn draw_text_sidebar(&mut self, ui: &mut egui::Ui, colors: &Colors, entry: &Entry) {
        let path = entry.path.clone();
        self.ensure_text_preview(&path);

        // Cmd+S saves the file.
        let should_save = ui.input_mut(|i| {
            i.consume_key(egui::Modifiers::COMMAND, egui::Key::S)
        });
        if should_save {
            self.save_text_preview_if_dirty();
        }

        egui::Frame::new()
            .fill(colors.bg_sidebar)
            .stroke(Stroke::new(1.0, colors.border))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(entry.name.clone())
                            .size(10.5)
                            .color(colors.text_primary)
                            .strong(),
                    );
                    if self.text_preview_dirty {
                        ui.label(
                            egui::RichText::new("modified")
                                .size(9.0)
                                .color(colors.accent),
                        );
                    }
                });
                ui.label(
                    egui::RichText::new("Cmd+S to save")
                        .size(9.0)
                        .color(colors.text_dim.linear_multiply(0.5)),
                );
                ui.add_space(4.0);

                if let Some(body) = &mut self.text_preview_body {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            let response = ui.add(
                                egui::TextEdit::multiline(body)
                                    .font(egui::FontId::monospace(11.0))
                                    .text_color(colors.text_primary)
                                    .desired_width(f32::INFINITY)
                                    .frame(false)
                                    .code_editor(),
                            );
                            if response.changed() {
                                self.text_preview_dirty = true;
                            }
                        });
                }
            });
    }

    fn draw_audio_sidebar(&mut self, ui: &mut egui::Ui, colors: &Colors, entry: &Entry) {
        let is_this_playing = self.audio_playing_path.as_ref() == Some(&entry.path);

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
                    egui::RichText::new(format!("Audio \u{00b7} {}", entry.name))
                        .size(10.5)
                        .color(colors.text_primary)
                        .strong(),
                );
                if let Some(size) = entry.size_bytes {
                    ui.label(egui::RichText::new(format_size(Some(size))).size(9.5).color(colors.text_dim));
                }
                ui.add_space(8.0);

                let button_label = if is_this_playing && self.audio_playing {
                    "\u{23f8} Pause"
                } else if is_this_playing && self.audio_paused {
                    "\u{25b6} Resume"
                } else {
                    "\u{25b6} Play"
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

                if is_this_playing && (self.audio_playing || self.audio_paused) {
                    if ui.button("\u{23f9} Stop").clicked() {
                        self.audio_stop();
                    }
                }

                if is_this_playing {
                    let elapsed = self.audio_elapsed();
                    let mins = (elapsed / 60.0) as u32;
                    let secs = (elapsed % 60.0) as u32;
                    let state = if self.audio_playing { "Playing" } else { "Paused" };
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new(format!("{state} \u{2014} {mins}:{secs:02}"))
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

    // ─── Audio control ───────────────────────────────────────────────────────

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
                        let name_label = if self.sort_mode == SortMode::Name { "Name \u{2713}" } else { "Name" };
                        let recent_label = if self.sort_mode == SortMode::RecentlyTouched { "Recent \u{2713}" } else { "Recent" };
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
                        let _ = std::process::Command::new("open").arg(&path).spawn();
                    }
                }
            });
    }

    fn handle_key(&mut self, input: &egui::InputState) -> bool {
        if self.entries.is_empty() {
            if input.key_pressed(egui::Key::Backspace) {
                self.navigate_up();
                return true;
            }
            return false;
        }

        let last = self.entries.len().saturating_sub(1);
        let mut consumed = false;

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

        if !input.modifiers.command && (input.key_pressed(egui::Key::Backspace) || input.key_pressed(egui::Key::ArrowLeft) || input.key_pressed(egui::Key::H)) {
            self.navigate_up();
            consumed = true;
        }

        if input.key_pressed(egui::Key::S) {
            self.sort_mode = match self.sort_mode {
                SortMode::RecentlyTouched => SortMode::Name,
                SortMode::Name => SortMode::RecentlyTouched,
            };
            self.refresh();
            consumed = true;
        }

        if input.key_pressed(egui::Key::R) {
            self.refresh();
            consumed = true;
        }

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

    fn current_dir(&self) -> Option<&std::path::Path> {
        Some(&self.cwd)
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
