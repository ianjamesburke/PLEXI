use std::path::PathBuf;
use std::time::Duration;

use crate::app::app_trait::{App, AppRenderContext};
use crate::media::audio::{PlaybackRequest, PlaybackSession};

pub struct AudioPlayerApp {
    path: PathBuf,
    session: Option<PlaybackSession>,
    duration_ms: Option<u64>,
    position_ms: u64,
    playing: bool,
    volume: f32,
    error: Option<String>,
}

impl AudioPlayerApp {
    pub fn new(path: PathBuf) -> Self {
        log::info!(
            "AudioPlayerApp: launch app_id=audio-player path={}",
            path.display()
        );
        let duration_ms = crate::media::audio::source_duration_ms(&path.to_string_lossy());
        Self {
            path,
            session: None,
            duration_ms,
            position_ms: 0,
            playing: false,
            volume: 1.0,
            error: None,
        }
    }

    fn filename(&self) -> String {
        self.path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Audio")
            .to_string()
    }

    fn ensure_session(&mut self) -> bool {
        if self.session.is_some() {
            return true;
        }
        let request = PlaybackRequest {
            source: self.path.to_string_lossy().to_string(),
            volume: self.volume,
        };
        match crate::media::audio::start_playback(request) {
            Ok(session) => {
                self.session = Some(session);
                true
            }
            Err(err) => {
                let msg = format!("Unable to play audio: {err}");
                log::warn!("AudioPlayerApp: {} path={}", msg, self.path.display());
                self.error = Some(msg);
                false
            }
        }
    }

    fn set_playing(&mut self, playing: bool) {
        if playing && !self.ensure_session() {
            return;
        }
        if let Some(session) = &self.session {
            if playing {
                session.resume();
            } else {
                session.pause();
            }
        }
        self.playing = playing;
    }

    fn seek(&mut self, position_ms: u64) {
        self.position_ms = self
            .duration_ms
            .map(|duration| position_ms.min(duration))
            .unwrap_or(position_ms);
        if let Some(session) = &self.session {
            if let Err(err) = session.seek(Duration::from_millis(self.position_ms)) {
                self.error = Some(format!("Seek failed: {err}"));
            }
        }
    }

    fn sync_position(&mut self) {
        if let Some(session) = &self.session {
            self.position_ms = session.position().as_millis() as u64;
        }
    }
}

impl App for AudioPlayerApp {
    fn type_id(&self) -> &'static str {
        "audio-player"
    }

    fn display_name(&self) -> String {
        format!("Audio - {}", self.filename())
    }

    fn ui(&mut self, ui: &mut egui::Ui, _ctx: &AppRenderContext<'_>) {
        self.sync_position();
        if self.playing {
            ui.ctx().request_repaint();
        }
        ui.heading(self.filename());
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
            }
            ui.label(format!(
                "{} / {}",
                format_time(self.position_ms),
                self.duration_ms
                    .map(format_time)
                    .unwrap_or_else(|| "--:--".to_string())
            ));
        });
        ui.horizontal(|ui| {
            ui.label("Volume");
            if ui
                .add(egui::Slider::new(&mut self.volume, 0.0..=2.0))
                .changed()
            {
                if let Some(session) = &self.session {
                    session.set_volume(self.volume);
                }
            }
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
            "volume": self.volume,
        }))
    }
}

fn format_time(ms: u64) -> String {
    let total_seconds = ms / 1000;
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    format!("{minutes:02}:{seconds:02}")
}
