// Jukebox playlist + transport model (stint 0513).
//
// Pure state machine: no host imports, no I/O, no allocation on the RT path
// (`fill_output`). The WASM shell (`lib.rs`) owns every host effect — picking
// files, reading bytes, opening the audio stream — and feeds decoded PCM in
// through `add_track`. Everything the app knows about "what is playing and
// where" lives here so it is unit-testable without a host or a device.
//
// Sample storage is normalized to the audio stream's negotiated
// (sample_rate, channels) at load time (`normalize`), so `fill_output` is a
// straight interleaved copy scaled by volume — the RT thread never resamples
// or mixes channels.

use serde_json::{json, Value};

/// Load state of one track's PCM. A track appears in the playlist the moment
/// it is picked; its bytes arrive later (or never, on a read/decode failure).
#[derive(Debug, Clone, PartialEq)]
pub enum Load {
    Loaded,
    Pending,
    Failed(String),
}

impl Load {
    fn label(&self) -> &'static str {
        match self {
            Load::Loaded => "loaded",
            Load::Pending => "pending",
            Load::Failed(_) => "failed",
        }
    }
}

/// One playlist entry. `samples` is interleaved f32 already normalized to the
/// jukebox's stream (rate, channels); it is empty until the track loads.
#[derive(Debug, Clone)]
pub struct Track {
    /// Display name — the file stem, or the demo label.
    pub name: String,
    /// `demo:<id>` for synthesized content, or the absolute picked path.
    pub source: String,
    pub load: Load,
    /// Interleaved f32 at the stream's (rate, channels); empty unless Loaded.
    pub samples: Vec<f32>,
}

impl Track {
    /// Frame count at the stream's channel layout.
    fn frames(&self, channels: u32) -> u64 {
        if channels == 0 {
            0
        } else {
            (self.samples.len() / channels as usize) as u64
        }
    }
}

/// Playlist + transport. `current` indexes `tracks`; `cursor` is the play
/// position in frames within the current track.
pub struct Jukebox {
    tracks: Vec<Track>,
    current: usize,
    playing: bool,
    /// Output gain, `[0.0, 2.0]`. 1.0 is unity.
    volume: f32,
    cursor: u64,
    stream_rate: u32,
    stream_channels: u32,
}

impl Jukebox {
    #[must_use]
    pub fn new(stream_rate: u32, stream_channels: u32) -> Self {
        Jukebox {
            tracks: Vec::new(),
            current: 0,
            playing: false,
            volume: 1.0,
            cursor: 0,
            stream_rate: stream_rate.max(1),
            stream_channels: stream_channels.clamp(1, 2),
        }
    }

    #[must_use]
    pub fn stream_rate(&self) -> u32 {
        self.stream_rate
    }

    #[must_use]
    pub fn stream_channels(&self) -> u32 {
        self.stream_channels
    }

    #[must_use]
    pub fn track_count(&self) -> usize {
        self.tracks.len()
    }

    #[must_use]
    pub fn is_playing(&self) -> bool {
        self.playing
    }

    #[must_use]
    pub fn volume(&self) -> f32 {
        self.volume
    }

    #[must_use]
    pub fn current_index(&self) -> usize {
        self.current
    }

    #[must_use]
    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    /// Appends a pending track (name + source), returning its index. Bytes are
    /// filled later via [`Jukebox::load_track`].
    pub fn add_pending(&mut self, name: impl Into<String>, source: impl Into<String>) -> usize {
        self.tracks.push(Track {
            name: name.into(),
            source: source.into(),
            load: Load::Pending,
            samples: Vec::new(),
        });
        self.tracks.len() - 1
    }

    /// Adds an already-decoded track (demo content synthesized in-guest).
    /// `samples` must already be at this jukebox's stream (rate, channels).
    pub fn add_loaded(
        &mut self,
        name: impl Into<String>,
        source: impl Into<String>,
        samples: Vec<f32>,
    ) -> usize {
        self.tracks.push(Track {
            name: name.into(),
            source: source.into(),
            load: Load::Loaded,
            samples,
        });
        self.tracks.len() - 1
    }

    /// Resolves a pending track to Loaded, normalizing `samples` from its
    /// native (rate, channels) to the stream's. A track whose index no longer
    /// exists (never happens today — ids never rebind) is ignored.
    pub fn load_track(
        &mut self,
        index: usize,
        samples: &[f32],
        src_rate: u32,
        src_channels: u32,
    ) {
        let (dst_rate, dst_channels) = (self.stream_rate, self.stream_channels);
        if let Some(track) = self.tracks.get_mut(index) {
            track.samples = normalize(samples, src_rate, src_channels, dst_rate, dst_channels);
            track.load = Load::Loaded;
        }
    }

    /// Marks a pending track as failed with a human-readable reason.
    pub fn fail_track(&mut self, index: usize, reason: impl Into<String>) {
        if let Some(track) = self.tracks.get_mut(index) {
            track.load = Load::Failed(reason.into());
            track.samples = Vec::new();
        }
    }

    /// Source path of a pending track, for the shell's read queue.
    #[must_use]
    pub fn source_of(&self, index: usize) -> Option<&str> {
        self.tracks.get(index).map(|t| t.source.as_str())
    }

    // ── Transport ────────────────────────────────────────────────────────────

    pub fn play(&mut self) {
        if !self.tracks.is_empty() {
            self.playing = true;
        }
    }

    pub fn pause(&mut self) {
        self.playing = false;
    }

    pub fn toggle(&mut self) {
        if self.playing {
            self.pause();
        } else {
            self.play();
        }
    }

    /// Advances to the next track (wraps), rewinding the play cursor.
    pub fn next(&mut self) {
        if self.tracks.is_empty() {
            return;
        }
        self.current = (self.current + 1) % self.tracks.len();
        self.cursor = 0;
    }

    /// Steps to the previous track (wraps), rewinding the play cursor.
    pub fn prev(&mut self) {
        if self.tracks.is_empty() {
            return;
        }
        self.current = (self.current + self.tracks.len() - 1) % self.tracks.len();
        self.cursor = 0;
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume.clamp(0.0, 2.0);
    }

    /// Seeks the current track to `frame`, clamped to its length.
    pub fn seek(&mut self, frame: u64) {
        let len = self
            .tracks
            .get(self.current)
            .map(|t| t.frames(self.stream_channels))
            .unwrap_or(0);
        self.cursor = frame.min(len);
    }

    #[must_use]
    pub fn cursor(&self) -> u64 {
        self.cursor
    }

    // ── Real-time output ───────────────────────────────────────────────────
    //
    // Called from the host audio thread through `process-output`. Allocation-
    // free: it only writes into the caller's `out`. Fills silence when stopped
    // or when the current track is not loaded, and auto-advances to the next
    // track when the current one ends (a track that ends mid-buffer is
    // followed by the next track's head in the same buffer).

    /// Writes `out.len()` interleaved samples for the current transport state.
    /// `out.len()` must be a multiple of the stream channel count.
    pub fn fill_output(&mut self, out: &mut [f32]) {
        for s in out.iter_mut() {
            *s = 0.0;
        }
        if !self.playing || self.tracks.is_empty() {
            return;
        }
        let channels = self.stream_channels as usize;
        if channels == 0 {
            return;
        }
        let gain = self.volume;
        let mut written = 0usize;
        // Bound the walk by the playlist length so an all-empty playlist can
        // never spin here.
        let mut guard = self.tracks.len() + 1;
        while written < out.len() && guard > 0 {
            let Some(track) = self.tracks.get(self.current) else {
                return;
            };
            if track.load != Load::Loaded || track.samples.is_empty() {
                // Skip an unloaded/empty current track rather than stalling on
                // silence forever; if the whole playlist is unloaded the guard
                // stops the walk.
                self.advance_track();
                guard -= 1;
                continue;
            }
            let frames = track.samples.len() / channels;
            while written < out.len() && (self.cursor as usize) < frames {
                let base = self.cursor as usize * channels;
                for ch in 0..channels {
                    out[written] = track.samples[base + ch] * gain;
                    written += 1;
                }
                self.cursor += 1;
            }
            if (self.cursor as usize) >= frames {
                // Track finished: roll to the next one and keep filling.
                self.advance_track();
                guard -= 1;
            }
        }
    }

    /// Auto-advance at end of track: wrap to the next entry and rewind. Unlike
    /// `next`, this is the RT-thread continuation, kept private.
    fn advance_track(&mut self) {
        if self.tracks.is_empty() {
            return;
        }
        self.current = (self.current + 1) % self.tracks.len();
        self.cursor = 0;
    }

    // ── Serialization / tool read surface ─────────────────────────────────

    /// Milliseconds of playback elapsed in the current track.
    #[must_use]
    pub fn position_ms(&self) -> u64 {
        frames_to_ms(self.cursor, self.stream_rate)
    }

    fn duration_ms(&self, index: usize) -> u64 {
        self.tracks
            .get(index)
            .map(|t| frames_to_ms(t.frames(self.stream_channels), self.stream_rate))
            .unwrap_or(0)
    }

    /// Full transport + playlist snapshot (no PCM). The app's persisted state
    /// and the `now_playing` tool both derive from this shape.
    #[must_use]
    pub fn serialize_state(&self) -> String {
        serde_json::to_string(&self.state_value()).unwrap_or_else(|_| "{}".to_string())
    }

    fn state_value(&self) -> Value {
        let tracks: Vec<Value> = self
            .tracks
            .iter()
            .enumerate()
            .map(|(i, t)| {
                json!({
                    "index": i,
                    "name": t.name,
                    "source": t.source,
                    "state": t.load.label(),
                    "error": match &t.load {
                        Load::Failed(reason) => Value::String(reason.clone()),
                        _ => Value::Null,
                    },
                    "duration_ms": self.duration_ms(i),
                })
            })
            .collect();
        json!({
            "current": self.current,
            "playing": self.playing,
            "volume": self.volume,
            "position_frames": self.cursor,
            "position_ms": self.position_ms(),
            "stream_rate": self.stream_rate,
            "stream_channels": self.stream_channels,
            "tracks": tracks,
        })
    }

    /// Read-only `jukebox.list_files` output: the playlist as index/name/
    /// source/state rows.
    #[must_use]
    pub fn list_files_value(&self) -> Value {
        let tracks: Vec<Value> = self
            .tracks
            .iter()
            .enumerate()
            .map(|(i, t)| {
                json!({
                    "index": i,
                    "name": t.name,
                    "source": t.source,
                    "state": t.load.label(),
                    "duration_ms": self.duration_ms(i),
                })
            })
            .collect();
        json!({ "count": self.tracks.len(), "tracks": tracks })
    }

    /// Read-only `jukebox.now_playing` output.
    #[must_use]
    pub fn now_playing_value(&self) -> Value {
        match self.tracks.get(self.current) {
            Some(track) => json!({
                "index": self.current,
                "name": track.name,
                "source": track.source,
                "state": track.load.label(),
                "playing": self.playing,
                "volume": self.volume,
                "position_ms": self.position_ms(),
                "duration_ms": self.duration_ms(self.current),
                "track_count": self.tracks.len(),
            }),
            None => json!({
                "index": self.current,
                "name": Value::Null,
                "playing": false,
                "volume": self.volume,
                "track_count": 0,
            }),
        }
    }
}

fn frames_to_ms(frames: u64, rate: u32) -> u64 {
    if rate == 0 {
        0
    } else {
        frames.saturating_mul(1000) / rate as u64
    }
}

/// Resamples/remixes interleaved `samples` from `(src_rate, src_channels)` to
/// `(dst_rate, dst_channels)`. Rate conversion is linear interpolation;
/// channel conversion averages down to mono and duplicates mono up to stereo.
/// Returns interleaved f32 at the destination layout. Empty in ⇒ empty out.
#[must_use]
pub fn normalize(
    samples: &[f32],
    src_rate: u32,
    src_channels: u32,
    dst_rate: u32,
    dst_channels: u32,
) -> Vec<f32> {
    let src_channels = src_channels.max(1) as usize;
    let dst_channels = dst_channels.clamp(1, 2) as usize;
    let src_rate = src_rate.max(1);
    let dst_rate = dst_rate.max(1);
    let src_frames = samples.len() / src_channels;
    if src_frames == 0 {
        return Vec::new();
    }

    // Collapse each source frame to mono, then fan out to dst channels. A
    // jukebox does not need per-channel fidelity; correctness of pitch and
    // duration matters, channel imaging does not.
    let mono: Vec<f32> = (0..src_frames)
        .map(|f| {
            let base = f * src_channels;
            let sum: f32 = samples[base..base + src_channels].iter().sum();
            sum / src_channels as f32
        })
        .collect();

    let dst_frames = ((src_frames as u64 * dst_rate as u64) / src_rate as u64).max(1) as usize;
    let mut out = Vec::with_capacity(dst_frames * dst_channels);
    for f in 0..dst_frames {
        // Position in source frames for this destination frame.
        let src_pos = f as f64 * src_rate as f64 / dst_rate as f64;
        let i = src_pos.floor() as usize;
        let frac = (src_pos - i as f64) as f32;
        let a = mono[i.min(src_frames - 1)];
        let b = mono[(i + 1).min(src_frames - 1)];
        let v = a + (b - a) * frac;
        for _ in 0..dst_channels {
            out.push(v);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms(samples: &[f32]) -> f32 {
        if samples.is_empty() {
            return 0.0;
        }
        let sum: f32 = samples.iter().map(|s| s * s).sum();
        (sum / samples.len() as f32).sqrt()
    }

    /// A short constant-1.0 stereo track at the stream layout.
    fn loud_track(frames: usize) -> Vec<f32> {
        vec![1.0; frames * 2]
    }

    #[test]
    fn new_jukebox_is_empty_and_stopped() {
        let jb = Jukebox::new(48_000, 2);
        assert_eq!(jb.track_count(), 0);
        assert!(!jb.is_playing());
        assert_eq!(jb.volume(), 1.0);
        assert_eq!(jb.current_index(), 0);
    }

    #[test]
    fn play_is_a_noop_without_tracks() {
        let mut jb = Jukebox::new(48_000, 2);
        jb.play();
        assert!(!jb.is_playing(), "cannot play an empty playlist");
    }

    #[test]
    fn transport_play_pause_toggle() {
        let mut jb = Jukebox::new(48_000, 2);
        jb.add_loaded("a", "demo:a", loud_track(10));
        jb.play();
        assert!(jb.is_playing());
        jb.pause();
        assert!(!jb.is_playing());
        jb.toggle();
        assert!(jb.is_playing());
        jb.toggle();
        assert!(!jb.is_playing());
    }

    #[test]
    fn next_prev_wrap_and_rewind_cursor() {
        let mut jb = Jukebox::new(48_000, 2);
        jb.add_loaded("a", "demo:a", loud_track(10));
        jb.add_loaded("b", "demo:b", loud_track(10));
        jb.add_loaded("c", "demo:c", loud_track(10));
        jb.play();
        jb.seek(5);
        assert_eq!(jb.cursor(), 5);
        jb.next();
        assert_eq!(jb.current_index(), 1);
        assert_eq!(jb.cursor(), 0, "next rewinds");
        jb.prev();
        assert_eq!(jb.current_index(), 0);
        jb.prev();
        assert_eq!(jb.current_index(), 2, "prev wraps to last");
    }

    #[test]
    fn set_volume_clamps_to_range() {
        let mut jb = Jukebox::new(48_000, 2);
        jb.set_volume(5.0);
        assert_eq!(jb.volume(), 2.0);
        jb.set_volume(-1.0);
        assert_eq!(jb.volume(), 0.0);
        jb.set_volume(0.75);
        assert_eq!(jb.volume(), 0.75);
    }

    #[test]
    fn fill_output_is_silent_when_stopped() {
        let mut jb = Jukebox::new(48_000, 2);
        jb.add_loaded("a", "demo:a", loud_track(64));
        let mut buf = vec![0.0f32; 32];
        jb.fill_output(&mut buf);
        assert_eq!(rms(&buf), 0.0, "stopped output must be silence");
    }

    #[test]
    fn fill_output_emits_audio_when_playing() {
        let mut jb = Jukebox::new(48_000, 2);
        jb.add_loaded("a", "demo:a", loud_track(64));
        jb.play();
        let mut buf = vec![0.0f32; 32];
        jb.fill_output(&mut buf);
        assert!(rms(&buf) > 0.5, "playing output must be non-silent: {}", rms(&buf));
        assert_eq!(jb.cursor(), 16, "16 stereo frames consumed");
    }

    #[test]
    fn fill_output_scales_by_volume() {
        let mut jb = Jukebox::new(48_000, 2);
        jb.add_loaded("a", "demo:a", loud_track(64));
        jb.play();
        jb.set_volume(0.5);
        let mut buf = vec![0.0f32; 8];
        jb.fill_output(&mut buf);
        assert!(buf.iter().all(|&s| (s - 0.5).abs() < 1e-6), "0.5 gain on a 1.0 track");
    }

    #[test]
    fn fill_output_auto_advances_at_track_end() {
        let mut jb = Jukebox::new(48_000, 2);
        jb.add_loaded("a", "demo:a", loud_track(4)); // 4 frames
        jb.add_loaded("b", "demo:b", loud_track(64));
        jb.play();
        // 4 frames * 2 ch = 8 samples of track a, then track b begins.
        let mut buf = vec![0.0f32; 16];
        jb.fill_output(&mut buf);
        assert_eq!(jb.current_index(), 1, "rolled onto track b");
        assert!(rms(&buf) > 0.5, "no silent gap across the track boundary");
    }

    #[test]
    fn fill_output_skips_unloaded_current_track() {
        let mut jb = Jukebox::new(48_000, 2);
        jb.add_pending("pending", "/music/x.wav"); // never loaded
        jb.add_loaded("b", "demo:b", loud_track(64));
        jb.play();
        let mut buf = vec![0.0f32; 16];
        jb.fill_output(&mut buf);
        assert_eq!(jb.current_index(), 1, "skipped the unloaded track");
        assert!(rms(&buf) > 0.5);
    }

    #[test]
    fn fill_output_all_unloaded_does_not_spin() {
        let mut jb = Jukebox::new(48_000, 2);
        jb.add_pending("a", "/music/a.wav");
        jb.add_pending("b", "/music/b.wav");
        jb.play();
        let mut buf = vec![0.0f32; 16];
        jb.fill_output(&mut buf); // must terminate
        assert_eq!(rms(&buf), 0.0);
    }

    #[test]
    fn load_track_resolves_pending_and_normalizes_rate() {
        let mut jb = Jukebox::new(48_000, 2);
        let idx = jb.add_pending("x", "/music/x.wav");
        // 24_000 Hz mono, 10 frames of 1.0 → upsampled to 48k stereo.
        jb.load_track(idx, &vec![1.0; 10], 24_000, 1);
        assert!(matches!(jb.tracks()[idx].load, Load::Loaded));
        assert!(jb.tracks()[idx].samples.len() >= 20 * 2, "roughly 2x frames, stereo");
    }

    #[test]
    fn fail_track_marks_reason_and_clears_samples() {
        let mut jb = Jukebox::new(48_000, 2);
        let idx = jb.add_pending("x", "/music/x.wav");
        jb.fail_track(idx, "wav: not a RIFF/WAVE stream");
        match &jb.tracks()[idx].load {
            Load::Failed(reason) => assert!(reason.contains("RIFF")),
            other => panic!("expected Failed, got {other:?}"),
        }
        assert!(jb.tracks()[idx].samples.is_empty());
    }

    #[test]
    fn serialize_state_round_trips_transport_and_playlist() {
        let mut jb = Jukebox::new(44_100, 2);
        jb.add_loaded("Track A", "demo:a", loud_track(44_100)); // 1s
        jb.add_pending("song", "/music/song.wav");
        jb.play();
        jb.set_volume(0.8);
        let v: Value = serde_json::from_str(&jb.serialize_state()).unwrap();
        assert_eq!(v["playing"], true);
        assert!((v["volume"].as_f64().unwrap() - 0.8).abs() < 1e-6);
        assert_eq!(v["stream_rate"], 44_100);
        assert_eq!(v["tracks"].as_array().unwrap().len(), 2);
        assert_eq!(v["tracks"][0]["state"], "loaded");
        assert_eq!(v["tracks"][0]["duration_ms"], 1000);
        assert_eq!(v["tracks"][1]["state"], "pending");
    }

    #[test]
    fn now_playing_reports_current_track() {
        let mut jb = Jukebox::new(48_000, 2);
        jb.add_loaded("First", "demo:a", loud_track(48_000));
        jb.add_loaded("Second", "demo:b", loud_track(48_000));
        jb.next();
        jb.play();
        let v = jb.now_playing_value();
        assert_eq!(v["index"], 1);
        assert_eq!(v["name"], "Second");
        assert_eq!(v["playing"], true);
        assert_eq!(v["track_count"], 2);
    }

    #[test]
    fn list_files_lists_every_track() {
        let mut jb = Jukebox::new(48_000, 2);
        jb.add_loaded("A", "demo:a", loud_track(10));
        jb.add_pending("B", "/music/b.wav");
        let v = jb.list_files_value();
        assert_eq!(v["count"], 2);
        assert_eq!(v["tracks"][0]["name"], "A");
        assert_eq!(v["tracks"][1]["state"], "pending");
    }

    #[test]
    fn normalize_mono_to_stereo_duplicates() {
        let out = normalize(&[0.4, 0.6], 48_000, 1, 48_000, 2);
        assert_eq!(out, vec![0.4, 0.4, 0.6, 0.6]);
    }

    #[test]
    fn normalize_stereo_to_mono_averages() {
        let out = normalize(&[0.2, 0.8, -1.0, 1.0], 48_000, 2, 48_000, 1);
        assert_eq!(out, vec![0.5, 0.0]);
    }

    #[test]
    fn normalize_doubles_frame_count_on_2x_upsample() {
        // 4 mono frames at 24k → ~8 frames at 48k, stereo.
        let out = normalize(&[0.0, 1.0, 0.0, -1.0], 24_000, 1, 48_000, 2);
        assert_eq!(out.len(), 8 * 2);
    }

    #[test]
    fn normalize_empty_is_empty() {
        assert!(normalize(&[], 48_000, 2, 48_000, 2).is_empty());
    }
}
