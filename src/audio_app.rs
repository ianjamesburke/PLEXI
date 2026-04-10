/// Audio player app — plays audio files scoped to the launch directory.
///
/// Demonstrates the app infrastructure with:
/// - Capability-scoped permissions (filesystem read_only, no terminal write)
/// - serialize_state / restore_state for persistence
/// - Proper keyboard handling (respects reserved Plexi shortcuts)
/// - Real audio playback via rodio (on a dedicated audio thread)

use crate::app_trait::{App, AppCommand, AppRenderContext};
use crate::theme::Colors;
use egui::{CornerRadius, Stroke, StrokeKind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Instant;

const STATE_STOPPED: u8 = 0;
const STATE_PLAYING: u8 = 1;
const STATE_PAUSED: u8 = 2;

/// Messages sent to the audio thread.
enum AudioMsg {
    Play(PathBuf),
    Pause,
    Resume,
    Stop,
    Shutdown,
}

pub struct AudioApp {
    scope_dir: PathBuf,
    files: Vec<PathBuf>,
    selected: usize,
    loaded_file: Option<PathBuf>,
    /// Shared state with audio thread.
    playback_state: Arc<AtomicU8>,
    /// Elapsed millis reported by the audio thread.
    elapsed_ms: Arc<AtomicU64>,
    /// Channel to send commands to audio thread.
    audio_tx: mpsc::Sender<AudioMsg>,
    play_started: Option<Instant>,
    elapsed_before_pause: f32,
    pending_cmds: Vec<AppCommand>,
    pending_scroll: bool,
}

impl AudioApp {
    pub fn new(scope_dir: PathBuf) -> Self {
        let files = scan_audio_files(&scope_dir);
        let playback_state = Arc::new(AtomicU8::new(STATE_STOPPED));
        let elapsed_ms = Arc::new(AtomicU64::new(0));
        let (tx, rx) = mpsc::channel();

        // Spawn audio thread — owns OutputStream and Sink (not Send).
        let state_clone = playback_state.clone();
        std::thread::spawn(move || {
            audio_thread(rx, state_clone);
        });

        Self {
            scope_dir,
            files,
            selected: 0,
            loaded_file: None,
            playback_state,
            elapsed_ms,
            audio_tx: tx,
            play_started: None,
            elapsed_before_pause: 0.0,
            pending_cmds: Vec::new(),
            pending_scroll: false,
        }
    }

    fn play_selected(&mut self) {
        let Some(path) = self.files.get(self.selected).cloned() else {
            return;
        };
        let _ = self.audio_tx.send(AudioMsg::Play(path.clone()));
        self.loaded_file = Some(path);
        self.play_started = Some(Instant::now());
        self.elapsed_before_pause = 0.0;
    }

    fn toggle_pause(&mut self) {
        let state = self.playback_state.load(Ordering::Relaxed);
        match state {
            STATE_PLAYING => {
                let _ = self.audio_tx.send(AudioMsg::Pause);
                if let Some(started) = self.play_started {
                    self.elapsed_before_pause += started.elapsed().as_secs_f32();
                }
                self.play_started = None;
            }
            STATE_PAUSED => {
                let _ = self.audio_tx.send(AudioMsg::Resume);
                self.play_started = Some(Instant::now());
            }
            _ => {
                self.play_selected();
            }
        }
    }

    fn stop(&mut self) {
        let _ = self.audio_tx.send(AudioMsg::Stop);
        self.loaded_file = None;
        self.play_started = None;
        self.elapsed_before_pause = 0.0;
    }

    fn elapsed_secs(&self) -> f32 {
        let current = self.play_started
            .map(|s| s.elapsed().as_secs_f32())
            .unwrap_or(0.0);
        self.elapsed_before_pause + current
    }

    fn state_label(&self) -> &'static str {
        match self.playback_state.load(Ordering::Relaxed) {
            STATE_PLAYING => "PLAYING",
            STATE_PAUSED => "PAUSED",
            _ => "STOPPED",
        }
    }

    fn is_playing(&self) -> bool {
        self.playback_state.load(Ordering::Relaxed) == STATE_PLAYING
    }
}

impl Drop for AudioApp {
    fn drop(&mut self) {
        let _ = self.audio_tx.send(AudioMsg::Shutdown);
    }
}

impl App for AudioApp {
    fn type_id(&self) -> &'static str {
        "audio_player"
    }

    fn display_name(&self) -> String {
        "Audio".to_string()
    }

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &AppRenderContext<'_>) {
        let colors = ctx.colors;

        // Request repaints while playing (for elapsed time counter).
        if self.is_playing() {
            ui.ctx().request_repaint();
        }

        egui::Frame::new()
            .fill(colors.terminal_bg)
            .inner_margin(egui::Margin::symmetric(16, 12))
            .show(ui, |ui| {
                // Header
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("Audio Player")
                            .size(13.0)
                            .color(colors.accent)
                            .strong(),
                    );
                    ui.label(
                        egui::RichText::new(format!("  {} files", self.files.len()))
                            .size(10.0)
                            .color(colors.text_dim),
                    );
                });
                ui.add_space(4.0);

                // Now playing bar
                if let Some(path) = &self.loaded_file.clone() {
                    let name = path.file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    let elapsed = self.elapsed_secs();
                    let mins = (elapsed / 60.0) as u32;
                    let secs = (elapsed % 60.0) as u32;
                    let state_label = self.state_label();

                    let bar_rect = ui.available_rect_before_wrap();
                    let bar_rect = egui::Rect::from_min_size(
                        bar_rect.min,
                        egui::vec2(bar_rect.width(), 36.0),
                    );
                    ui.painter().rect_filled(
                        bar_rect,
                        CornerRadius::same(4),
                        colors.bg_active,
                    );
                    ui.painter().text(
                        egui::pos2(bar_rect.left() + 10.0, bar_rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        format!("{state_label}  {name}  {mins}:{secs:02}"),
                        egui::FontId::monospace(11.0),
                        colors.text_primary,
                    );
                    ui.advance_cursor_after_rect(bar_rect);
                    ui.add_space(4.0);
                }

                // Controls hint
                ui.label(
                    egui::RichText::new("Space play/pause · S stop · Enter play selected · Esc close")
                        .size(9.5)
                        .color(colors.text_dim),
                );
                ui.add_space(6.0);
                ui.separator();
                ui.add_space(4.0);

                // File list
                if self.files.is_empty() {
                    ui.label(
                        egui::RichText::new("No audio files found in this directory")
                            .size(11.0)
                            .color(colors.text_dim),
                    );
                } else {
                    let should_scroll = self.pending_scroll;
                    self.pending_scroll = false;
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            for (idx, path) in self.files.clone().iter().enumerate() {
                                let name = path.file_name()
                                    .map(|n| n.to_string_lossy().to_string())
                                    .unwrap_or_default();
                                let is_selected = idx == self.selected;
                                let is_loaded = self.loaded_file.as_ref() == Some(path);
                                let row_h = 32.0;
                                let (rect, resp) = ui.allocate_exact_size(
                                    egui::vec2(ui.available_width(), row_h),
                                    egui::Sense::click(),
                                );

                                if is_selected && should_scroll {
                                    resp.scroll_to_me(None);
                                }

                                let fill = if is_selected {
                                    colors.bg_active
                                } else if resp.hovered() {
                                    colors.bg_hover
                                } else {
                                    colors.terminal_bg
                                };
                                ui.painter().rect_filled(rect, CornerRadius::same(4), fill);
                                if is_selected {
                                    ui.painter().rect_stroke(
                                        rect,
                                        CornerRadius::same(4),
                                        Stroke::new(1.0, colors.accent),
                                        StrokeKind::Inside,
                                    );
                                }

                                // Audio icon
                                let icon_color = if is_loaded {
                                    colors.accent
                                } else {
                                    colors.text_dim
                                };
                                let icon = if is_loaded && self.is_playing() {
                                    ">"
                                } else {
                                    "#"
                                };
                                ui.painter().text(
                                    egui::pos2(rect.left() + 10.0, rect.center().y),
                                    egui::Align2::LEFT_CENTER,
                                    icon,
                                    egui::FontId::monospace(11.0),
                                    icon_color,
                                );

                                ui.painter().text(
                                    egui::pos2(rect.left() + 28.0, rect.center().y),
                                    egui::Align2::LEFT_CENTER,
                                    &name,
                                    egui::FontId::proportional(11.0),
                                    if is_loaded { colors.accent } else { colors.text_primary },
                                );

                                if resp.clicked() {
                                    self.selected = idx;
                                }
                                if resp.double_clicked() {
                                    self.selected = idx;
                                    self.play_selected();
                                }
                                ui.add_space(2.0);
                            }
                        });
                }
            });
    }

    fn handle_key(&mut self, input: &egui::InputState) -> bool {
        // Never consume Cmd-modified keys.
        if input.modifiers.command {
            return false;
        }

        if input.key_pressed(egui::Key::ArrowDown) || input.key_pressed(egui::Key::J) {
            self.selected = (self.selected + 1).min(self.files.len().saturating_sub(1));
            self.pending_scroll = true;
            return true;
        }
        if input.key_pressed(egui::Key::ArrowUp) || input.key_pressed(egui::Key::K) {
            self.selected = self.selected.saturating_sub(1);
            self.pending_scroll = true;
            return true;
        }
        if input.key_pressed(egui::Key::Enter) {
            self.play_selected();
            return true;
        }
        if input.key_pressed(egui::Key::Space) {
            self.toggle_pause();
            return true;
        }
        if input.key_pressed(egui::Key::S) {
            self.stop();
            return true;
        }

        false
    }

    fn take_pending_commands(&mut self) -> Vec<AppCommand> {
        std::mem::take(&mut self.pending_cmds)
    }

    fn accepted_extensions(&self) -> &[&str] {
        &["mp3", "wav", "flac", "ogg", "aiff", "aif", "m4a"]
    }

    fn serialize_state(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "scope_dir": self.scope_dir.display().to_string(),
            "selected": self.selected,
            "loaded_file": self.loaded_file.as_ref().map(|p| p.display().to_string()),
        }))
    }

    fn restore_state(&mut self, state: &serde_json::Value) {
        if let Some(sel) = state["selected"].as_u64() {
            self.selected = (sel as usize).min(self.files.len().saturating_sub(1));
        }
    }
}

fn scan_audio_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            if path.is_file() {
                let ext = path.extension()?.to_str()?.to_ascii_lowercase();
                if matches!(ext.as_str(), "mp3" | "wav" | "flac" | "ogg" | "aiff" | "aif" | "m4a") {
                    return Some(path);
                }
            }
            None
        })
        .collect();
    files.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    files
}

/// Audio thread — owns the rodio OutputStream and Sink (which are !Send).
fn audio_thread(rx: mpsc::Receiver<AudioMsg>, state: Arc<AtomicU8>) {
    let Ok((_stream, handle)) = rodio::OutputStream::try_default() else {
        log::error!("AudioApp: audio thread failed to open output stream");
        return;
    };
    let mut sink: Option<rodio::Sink> = None;

    loop {
        let msg = match rx.recv() {
            Ok(m) => m,
            Err(_) => break, // channel closed
        };
        match msg {
            AudioMsg::Play(path) => {
                // Stop existing playback.
                if let Some(s) = sink.take() {
                    s.stop();
                }
                let file = match std::fs::File::open(&path) {
                    Ok(f) => f,
                    Err(e) => {
                        log::error!("AudioApp: failed to open {}: {e}", path.display());
                        state.store(STATE_STOPPED, Ordering::Relaxed);
                        continue;
                    }
                };
                let source = match rodio::Decoder::new(std::io::BufReader::new(file)) {
                    Ok(s) => s,
                    Err(e) => {
                        log::error!("AudioApp: failed to decode {}: {e}", path.display());
                        state.store(STATE_STOPPED, Ordering::Relaxed);
                        continue;
                    }
                };
                let new_sink = match rodio::Sink::try_new(&handle) {
                    Ok(s) => s,
                    Err(e) => {
                        log::error!("AudioApp: failed to create sink: {e}");
                        state.store(STATE_STOPPED, Ordering::Relaxed);
                        continue;
                    }
                };
                new_sink.append(source);
                sink = Some(new_sink);
                state.store(STATE_PLAYING, Ordering::Relaxed);
            }
            AudioMsg::Pause => {
                if let Some(s) = &sink {
                    s.pause();
                }
                state.store(STATE_PAUSED, Ordering::Relaxed);
            }
            AudioMsg::Resume => {
                if let Some(s) = &sink {
                    s.play();
                }
                state.store(STATE_PLAYING, Ordering::Relaxed);
            }
            AudioMsg::Stop => {
                if let Some(s) = sink.take() {
                    s.stop();
                }
                state.store(STATE_STOPPED, Ordering::Relaxed);
            }
            AudioMsg::Shutdown => {
                if let Some(s) = sink.take() {
                    s.stop();
                }
                break;
            }
        }
    }
}
