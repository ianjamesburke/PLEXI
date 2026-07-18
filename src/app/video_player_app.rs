use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use crossbeam_queue::ArrayQueue;

use crate::app::app_trait::{App, AppRenderContext};
use crate::media::video::{VideoDecoder, VideoHandle, VideoState};

pub struct VideoPlayerApp {
    path: PathBuf,
    decoder: Arc<dyn VideoDecoder>,
    frame_ring: Arc<ArrayQueue<Vec<u8>>>,
    handle: Option<VideoHandle>,
    frame_texture: Option<egui::TextureHandle>,
    frame_size: Option<[usize; 2]>,
    duration_ms: Option<u64>,
    position_ms: u64,
    playing: bool,
    pause_after_seek: bool,
    last_tick: Instant,
    error: Option<String>,
}

impl VideoPlayerApp {
    pub fn new(path: PathBuf) -> Self {
        log::info!(
            "VideoPlayerApp: launch app_id=video-player path={}",
            path.display()
        );
        Self {
            path,
            decoder: crate::media::video::default_video_device(),
            frame_ring: Arc::new(ArrayQueue::new(3)),
            handle: None,
            frame_texture: None,
            frame_size: None,
            duration_ms: None,
            position_ms: 0,
            playing: false,
            pause_after_seek: false,
            last_tick: Instant::now(),
            error: None,
        }
    }

    fn filename(&self) -> String {
        self.path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Video")
            .to_string()
    }

    fn ensure_open(&mut self) -> bool {
        if self.handle.is_some() {
            return true;
        }
        match self
            .decoder
            .open(&self.path.to_string_lossy(), Arc::clone(&self.frame_ring))
        {
            Ok((ack, handle)) => {
                self.duration_ms = Some(ack.duration_ms);
                self.handle = Some(handle);
                log::info!(
                    "VideoPlayerApp: opened path={} handle={} duration_ms={}",
                    self.path.display(),
                    ack.handle_id,
                    ack.duration_ms
                );
                true
            }
            Err(err) => {
                let msg = format!("Unable to open video: {err}");
                log::warn!("VideoPlayerApp: {} path={}", msg, self.path.display());
                self.error = Some(msg);
                false
            }
        }
    }

    fn set_playing(&mut self, playing: bool) {
        if playing && !self.ensure_open() {
            return;
        }
        if let Some(handle) = self.handle.as_mut() {
            let state = if playing {
                VideoState::Play
            } else {
                VideoState::Pause
            };
            if let Err(err) = handle.set_state(state) {
                self.error = Some(format!("Playback control failed: {err}"));
                return;
            }
        }
        self.playing = playing;
        self.pause_after_seek = false;
        self.last_tick = Instant::now();
    }

    fn seek(&mut self, position_ms: u64) {
        self.position_ms = self
            .duration_ms
            .map(|duration| position_ms.min(duration))
            .unwrap_or(position_ms);
        if let Some(handle) = self.handle.as_mut() {
            if let Err(err) = handle.set_state(VideoState::Seek {
                position_ms: self.position_ms,
            }) {
                self.error = Some(format!("Seek failed: {err}"));
            } else if !self.playing {
                self.pause_after_seek = true;
            }
        }
        self.last_tick = Instant::now();
    }

    fn drain_frames(&mut self, ctx: &egui::Context) {
        let Some(handle) = self.handle.as_ref() else {
            return;
        };
        let size = [handle.width as usize, handle.height as usize];
        while let Some(frame) = self.frame_ring.pop() {
            if frame.len() != size[0].saturating_mul(size[1]).saturating_mul(4) {
                log::warn!(
                    "VideoPlayerApp: dropped malformed frame path={} bytes={} expected={}",
                    self.path.display(),
                    frame.len(),
                    size[0].saturating_mul(size[1]).saturating_mul(4)
                );
                continue;
            }
            let image = egui::ColorImage::from_rgba_unmultiplied(size, &frame);
            match &mut self.frame_texture {
                Some(texture) => texture.set(image, egui::TextureOptions::LINEAR),
                None => {
                    self.frame_texture = Some(ctx.load_texture(
                        format!("video-player:{}", self.path.display()),
                        image,
                        egui::TextureOptions::LINEAR,
                    ));
                }
            }
            self.frame_size = Some(size);
        }
    }

    fn tick_position(&mut self) {
        if !self.playing {
            if self.pause_after_seek {
                if let Some(handle) = self.handle.as_mut() {
                    if let Err(err) = handle.set_state(VideoState::Pause) {
                        self.error = Some(format!("Pause after seek failed: {err}"));
                    }
                }
                self.pause_after_seek = false;
            }
            self.last_tick = Instant::now();
            return;
        }
        let now = Instant::now();
        let delta_ms = now.duration_since(self.last_tick).as_millis() as u64;
        self.last_tick = now;
        self.position_ms = self.position_ms.saturating_add(delta_ms);
        if let Some(duration) = self.duration_ms {
            if self.position_ms >= duration {
                self.position_ms = duration;
                self.set_playing(false);
            }
        }
    }
}

impl App for VideoPlayerApp {
    #[cfg(test)]
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn type_id(&self) -> &'static str {
        "video-player"
    }

    fn display_name(&self) -> String {
        format!("Video - {}", self.filename())
    }

    fn ui(&mut self, ui: &mut egui::Ui, _ctx: &AppRenderContext<'_>) {
        self.tick_position();
        self.drain_frames(ui.ctx());
        if self.playing {
            ui.ctx().request_repaint();
        }

        ui.label(self.filename());
        let available = ui.available_size();
        let viewport_height = (available.y - 40.0).max(160.0);
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(available.x.max(1.0), viewport_height),
            egui::Sense::hover(),
        );
        ui.painter()
            .rect_filled(rect, 4.0, ui.visuals().extreme_bg_color);
        if let (Some(texture), Some([w, h])) = (&self.frame_texture, self.frame_size) {
            let tex_size = egui::vec2(w as f32, h as f32);
            let scale = (rect.width() / tex_size.x)
                .min(rect.height() / tex_size.y)
                .max(0.01);
            let draw_rect = egui::Rect::from_center_size(rect.center(), tex_size * scale);
            ui.painter().image(
                texture.id(),
                draw_rect,
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        } else {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                if self.handle.is_some() {
                    "Waiting for video frame"
                } else {
                    "Press Play"
                },
                egui::TextStyle::Body.resolve(ui.style()),
                ui.visuals().weak_text_color(),
            );
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let label = if self.playing { "Pause" } else { "Play" };
            if ui.button(label).clicked() {
                self.set_playing(!self.playing);
            }
            let duration = self.duration_ms.unwrap_or(0).max(self.position_ms);
            let mut pos = self.position_ms.min(duration);
            let response = ui.add(egui::Slider::new(&mut pos, 0..=duration).show_value(false));
            if response.changed() {
                self.seek(pos);
                if !self.playing {
                    ui.ctx().request_repaint();
                }
            }
            ui.label(format!(
                "{} / {}",
                format_time(self.position_ms),
                self.duration_ms
                    .map(format_time)
                    .unwrap_or_else(|| "--:--".to_string())
            ));
        });
        if let Some(error) = &self.error {
            ui.label(error);
        }
    }

    fn serialize_state(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "path": self.path.display().to_string(),
            "position_ms": self.position_ms,
            "duration_ms": self.duration_ms,
        }))
    }
}

fn format_time(ms: u64) -> String {
    let total_seconds = ms / 1000;
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    format!("{minutes:02}:{seconds:02}")
}
