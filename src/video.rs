//! Video subsystem substrate (#345).
//!
//! Three layers, mirroring [`crate::audio`] and [`crate::midi`]:
//!
//!   1. [`VideoDecoder`] — host-facing trait. `open()` returns a
//!      [`VideoOpenAck`] carrying the negotiated `width`/`height`/`fps`/
//!      `duration_ms` plus an opaque `handle_id`. Subsequent operations
//!      (`set_state`, `close`) take that `handle_id`. The trait lives behind
//!      `Arc<dyn VideoDecoder>` on each `ProcessApp` instance.
//!
//!   2. [`AvfVideoDecoder`] — production stub. Every method returns
//!      [`VideoError::NotImplemented`] cleanly. Real AVFoundation backing
//!      lands in #346 (split out of #278). The factory selects this impl
//!      unless `PLEXI_VIDEO=mock://...` is set.
//!
//!   3. [`MockVideoDecoder`] — procedural RGBA gradient at configurable fps.
//!      Frames are pushed via the binary-pipe ring (one frame per packet).
//!      Used by tests and by the POC `examples/video-player/` app driven
//!      with `PLEXI_VIDEO=mock://...`.
//!
//! Wire shape: apps `OpenVideo { request_id, source, pipe_id }` and the host
//! replies with `VideoOpenAck { request_id, handle_id, width, height, fps,
//! duration_ms }` on success or `VideoOpenError { request_id, error }` on
//! failure. Frames travel on the binary pipe as raw RGBA8 packed
//! `[R,G,B,A,R,G,B,A,...]` of length `width * height * 4`. One pipe frame =
//! one video frame; consumers read whole frames using the existing
//! `Pipe.read_frame()` length-prefix protocol.
//!
//! Out of scope (deferred to #346):
//!   - Real AVFoundation decoding.
//!   - PTS-accurate scheduling.
//!   - A/V sync.
//!   - Codec coverage (H.264, HEVC, ProRes, etc.).
//!   - Streaming sources (HTTP, RTSP).
//!   - Subtitles.

use std::sync::Arc;

use crossbeam_queue::ArrayQueue;

// ─── Public types ────────────────────────────────────────────────────────────

/// Playback state for a video handle. Encoded on the wire as
/// `{"play": null}` / `{"pause": null}` / `{"seek": <ms>}` via serde's
/// default `untagged`-friendly encoding. The PGAP wire serialises this as a
/// nested struct under `state` in `DrawCommand::SetVideoState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VideoState {
    Play,
    Pause,
    /// Absolute position in milliseconds from the start of the video.
    Seek { position_ms: u64 },
}

/// Result of `VideoDecoder::open` on success. Reported back to the app as
/// `PlexiEvent::VideoOpenAck`.
#[derive(Debug, Clone, PartialEq)]
pub struct VideoOpenAck {
    pub handle_id: u64,
    pub width: u32,
    pub height: u32,
    pub fps: f32,
    pub duration_ms: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum VideoError {
    /// Production stub returns this until #346 ships AVFoundation backing.
    /// Test and routing code can match on this variant directly.
    #[error("video decoder not implemented")]
    NotImplemented,
    #[error("invalid video source url: {0}")]
    InvalidSource(String),
    /// Wraps any underlying decoder failure.
    #[error("decoder: {0}")]
    Decoder(String),
}

/// Opaque video session. Drop tears down the worker thread (mock) or the
/// AVFoundation player (#346). Owned by `ProcessApp::video_handles`.
pub struct VideoHandle {
    pub handle_id: u64,
    pub width: u32,
    pub height: u32,
    pub fps: f32,
    pub duration_ms: u64,
    /// Drop-in cleanup. Mock spawns a worker thread; production stub is empty.
    _guard: Box<dyn VideoHandleGuard>,
    /// Per-handle inner control surface for `set_state`. Trait-object so the
    /// production stub (which never opens anything) and the mock (which
    /// drives a worker thread) can coexist behind one `VideoHandle` shape.
    inner: Box<dyn VideoHandleInner>,
}

impl std::fmt::Debug for VideoHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VideoHandle")
            .field("handle_id", &self.handle_id)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("fps", &self.fps)
            .field("duration_ms", &self.duration_ms)
            .finish()
    }
}

impl VideoHandle {
    pub fn set_state(&mut self, state: VideoState) -> Result<(), VideoError> {
        self.inner.set_state(state)
    }
}

trait VideoHandleGuard: Send {}
trait VideoHandleInner: Send {
    fn set_state(&mut self, state: VideoState) -> Result<(), VideoError>;
}

// ─── Device trait ────────────────────────────────────────────────────────────

pub trait VideoDecoder: Send + Sync {
    /// Open `source` and return its negotiated dimensions / fps / duration
    /// plus an opaque handle id. The handle is returned via the `VideoHandle`
    /// the caller must store; the returned `VideoOpenAck` is what we send to
    /// the app on the wire.
    ///
    /// `frame_ring` is where decoded RGBA frames are pushed. Each push is
    /// `width * height * 4` bytes, packed RGBA8. The mock spawns a worker
    /// thread that pumps frames into the ring at the configured fps; the
    /// production stub returns `Err(NotImplemented)` before touching the ring.
    fn open(
        &self,
        source: &str,
        frame_ring: Arc<ArrayQueue<Vec<u8>>>,
    ) -> Result<(VideoOpenAck, VideoHandle), VideoError>;
}

// ─── Production impl: AvfVideoDecoder ────────────────────────────────────────

/// Production video decoder shell. Every method returns
/// [`VideoError::NotImplemented`] cleanly until #346 swaps in real
/// AVFoundation backing. Per the panic-discipline rule, this stub MUST NOT
/// `todo!()` / `unimplemented!()` — those panic at runtime and freeze the
/// host UI thread.
pub struct AvfVideoDecoder;

impl AvfVideoDecoder {
    pub fn new() -> Self {
        Self
    }
}

impl Default for AvfVideoDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl VideoDecoder for AvfVideoDecoder {
    fn open(
        &self,
        _source: &str,
        _frame_ring: Arc<ArrayQueue<Vec<u8>>>,
    ) -> Result<(VideoOpenAck, VideoHandle), VideoError> {
        // #346 will replace this with real AVFoundation backing. Returning
        // an explicit error variant — never panic.
        Err(VideoError::NotImplemented)
    }
}

// ─── Mock impl: MockVideoDecoder ─────────────────────────────────────────────

/// Configuration for the mock decoder. Parsed out of `PLEXI_VIDEO=mock://...`
/// at factory time; constructed directly in tests.
#[derive(Debug, Clone, PartialEq)]
pub struct MockVideoDecoderConfig {
    pub width: u32,
    pub height: u32,
    pub fps: f32,
    /// Reported back as `duration_ms` on `open`. The mock never stops
    /// streaming on its own — the caller closes the handle when done. The
    /// duration is advisory metadata for UI seekbars.
    pub duration_ms: u64,
}

impl Default for MockVideoDecoderConfig {
    fn default() -> Self {
        Self {
            width: 320,
            height: 180,
            fps: 30.0,
            duration_ms: 30_000,
        }
    }
}

/// Procedural-gradient video decoder. Drives a worker thread that paints an
/// animated RGBA gradient and pushes frames into the binary pipe ring at
/// `fps`. Used by the POC and tests.
pub struct MockVideoDecoder {
    config: MockVideoDecoderConfig,
    next_handle_id: std::sync::atomic::AtomicU64,
}

impl MockVideoDecoder {
    pub fn new(config: MockVideoDecoderConfig) -> Self {
        Self {
            config,
            next_handle_id: std::sync::atomic::AtomicU64::new(1),
        }
    }

    /// Validate a `mock://...` source URL and parse out any per-call
    /// overrides. Today the URL is opaque ("mock://gradient") and we simply
    /// reject empty / non-`mock://` strings; the config carries the
    /// dimensions. Future: accept query-string fps/width/height overrides.
    fn validate_source(source: &str) -> Result<(), VideoError> {
        if source.is_empty() {
            return Err(VideoError::InvalidSource(
                "source URL is empty".to_owned(),
            ));
        }
        // Mock decoder accepts only mock:// URLs. Real-file paths must be
        // routed through AvfVideoDecoder (which today returns NotImplemented).
        if !source.starts_with("mock://") {
            return Err(VideoError::InvalidSource(format!(
                "MockVideoDecoder requires a mock:// URL, got {source:?}"
            )));
        }
        Ok(())
    }
}

impl VideoDecoder for MockVideoDecoder {
    fn open(
        &self,
        source: &str,
        frame_ring: Arc<ArrayQueue<Vec<u8>>>,
    ) -> Result<(VideoOpenAck, VideoHandle), VideoError> {
        Self::validate_source(source)?;

        let handle_id = self
            .next_handle_id
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let width = self.config.width;
        let height = self.config.height;
        let fps = self.config.fps.max(1.0);
        let duration_ms = self.config.duration_ms;

        // Per-handle control surface. The worker thread reads `state` on each
        // tick; the host writes via `set_state`.
        let state = Arc::new(std::sync::Mutex::new(VideoState::Play));
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));

        let state_thread = Arc::clone(&state);
        let stop_thread = Arc::clone(&stop);
        let frame_period = std::time::Duration::from_secs_f32(1.0 / fps);

        let handle = std::thread::Builder::new()
            .name(format!("mock-video-{handle_id}"))
            .spawn(move || {
                let mut frame_index: u64 = 0;
                loop {
                    if stop_thread.load(std::sync::atomic::Ordering::Acquire) {
                        return;
                    }
                    // Snapshot the current state and apply any seek. We
                    // drain the seek to avoid re-applying it on the next tick.
                    let paused = {
                        let mut st = state_thread
                            .lock()
                            .expect("mock-video state mutex poisoned");
                        match *st {
                            VideoState::Play => false,
                            VideoState::Pause => true,
                            VideoState::Seek { position_ms } => {
                                // Convert ms → frame index at `fps`; clamp to
                                // duration so we don't overshoot.
                                let total_frames = ((duration_ms as f32 / 1000.0) * fps) as u64;
                                let target = ((position_ms as f32 / 1000.0) * fps) as u64;
                                frame_index = target.min(total_frames.max(1) - 1);
                                // After applying the seek, resume play.
                                *st = VideoState::Play;
                                false
                            }
                        }
                    };
                    if !paused {
                        let buf = render_gradient_frame(width, height, frame_index);
                        // Drop on full ring — never block the worker.
                        let _ = frame_ring.push(buf);
                        frame_index = frame_index.wrapping_add(1);
                    }
                    std::thread::sleep(frame_period);
                }
            })
            .map_err(|e| VideoError::Decoder(format!("mock thread spawn: {e}")))?;

        struct MockGuard {
            stop: Arc<std::sync::atomic::AtomicBool>,
            handle: Option<std::thread::JoinHandle<()>>,
        }
        impl VideoHandleGuard for MockGuard {}
        impl Drop for MockGuard {
            fn drop(&mut self) {
                self.stop
                    .store(true, std::sync::atomic::Ordering::Release);
                if let Some(h) = self.handle.take() {
                    let _ = h.join();
                }
            }
        }

        struct MockInner {
            state: Arc<std::sync::Mutex<VideoState>>,
        }
        impl VideoHandleInner for MockInner {
            fn set_state(&mut self, new_state: VideoState) -> Result<(), VideoError> {
                let mut s = self
                    .state
                    .lock()
                    .expect("mock-video state mutex poisoned on set_state");
                *s = new_state;
                Ok(())
            }
        }

        let ack = VideoOpenAck {
            handle_id,
            width,
            height,
            fps,
            duration_ms,
        };

        let video_handle = VideoHandle {
            handle_id,
            width,
            height,
            fps,
            duration_ms,
            _guard: Box::new(MockGuard {
                stop,
                handle: Some(handle),
            }),
            inner: Box::new(MockInner { state }),
        };

        Ok((ack, video_handle))
    }
}

/// Paint a procedural RGBA8 gradient that animates with `frame_index`. The
/// horizontal axis encodes a hue sweep; the vertical axis encodes the frame
/// index modulated through brightness. Pure CPU, no allocations beyond the
/// returned buffer.
fn render_gradient_frame(width: u32, height: u32, frame_index: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity((width * height * 4) as usize);
    let phase = (frame_index % 256) as u8;
    for y in 0..height {
        let row_base = ((y as u32 * 255) / height.max(1)) as u8;
        for x in 0..width {
            let r = ((x as u32 * 255) / width.max(1)) as u8;
            let g = row_base.wrapping_add(phase);
            let b = phase.wrapping_add(row_base / 2);
            buf.push(r);
            buf.push(g);
            buf.push(b);
            buf.push(0xff);
        }
    }
    buf
}

// ─── Factory ────────────────────────────────────────────────────────────────

/// Build the production video decoder. Reads `PLEXI_VIDEO` for a
/// `mock://...` opt-in (parametrised via `PLEXI_VIDEO_FPS`,
/// `PLEXI_VIDEO_WIDTH`, `PLEXI_VIDEO_HEIGHT`, `PLEXI_VIDEO_DURATION_MS`);
/// otherwise instantiates the production [`AvfVideoDecoder`] stub.
///
/// Tests inject `Arc::new(MockVideoDecoder::new(cfg))` directly into
/// `ProcessApp::video_device` and skip the env var altogether.
pub fn default_video_device() -> Arc<dyn VideoDecoder> {
    if let Ok(url) = std::env::var("PLEXI_VIDEO") {
        if url.starts_with("mock://") {
            let cfg = MockVideoDecoderConfig {
                width: parse_env("PLEXI_VIDEO_WIDTH", 320),
                height: parse_env("PLEXI_VIDEO_HEIGHT", 180),
                fps: parse_env_f32("PLEXI_VIDEO_FPS", 30.0),
                duration_ms: parse_env("PLEXI_VIDEO_DURATION_MS", 30_000),
            };
            log::info!(
                "video: PLEXI_VIDEO={url} — using MockVideoDecoder ({}x{} @ {} fps)",
                cfg.width,
                cfg.height,
                cfg.fps
            );
            return Arc::new(MockVideoDecoder::new(cfg));
        }
    }
    Arc::new(AvfVideoDecoder::new())
}

fn parse_env<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse::<T>().ok())
        .unwrap_or(default)
}

fn parse_env_f32(key: &str, default: f32) -> f32 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
        .unwrap_or(default)
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_ring() -> Arc<ArrayQueue<Vec<u8>>> {
        Arc::new(ArrayQueue::new(64))
    }

    /// Convenience: pull whatever the ring drained in `wait_ms` and return
    /// the count of frames + the first frame for shape checks.
    fn drain_after(
        ring: &Arc<ArrayQueue<Vec<u8>>>,
        wait_ms: u64,
    ) -> (usize, Option<Vec<u8>>) {
        std::thread::sleep(std::time::Duration::from_millis(wait_ms));
        let mut first: Option<Vec<u8>> = None;
        let mut count = 0;
        while let Some(f) = ring.pop() {
            if first.is_none() {
                first = Some(f);
            } else {
                let _ = f;
            }
            count += 1;
        }
        (count, first)
    }

    #[test]
    fn open_with_invalid_path_returns_error_not_panic() {
        // Mock decoder rejects non-mock URLs. The contract: bad source → Err,
        // never panic. (Production stub returns NotImplemented for any
        // source, which is its own test below.)
        let dev = MockVideoDecoder::new(MockVideoDecoderConfig::default());
        let ring = make_ring();
        let res = dev.open("/no/such/file.mp4", Arc::clone(&ring));
        match res {
            Err(VideoError::InvalidSource(msg)) => {
                assert!(
                    msg.contains("/no/such/file.mp4"),
                    "error must name the bad source: {msg}"
                );
            }
            other => panic!("expected InvalidSource, got {other:?}"),
        }
        let res = dev.open("", Arc::clone(&ring));
        match res {
            Err(VideoError::InvalidSource(_)) => {}
            other => panic!("expected InvalidSource for empty source, got {other:?}"),
        }
    }

    #[test]
    fn mock_decoder_round_trips_frames() {
        // Open the mock at a small size and a high fps so we get several
        // frames in a short test window. Drain the ring and assert each
        // frame is exactly width*height*4 bytes RGBA8.
        let cfg = MockVideoDecoderConfig {
            width: 16,
            height: 8,
            fps: 60.0,
            duration_ms: 1_000,
        };
        let dev = MockVideoDecoder::new(cfg.clone());
        let ring = make_ring();
        let (ack, handle) = dev
            .open("mock://gradient", Arc::clone(&ring))
            .expect("mock open must succeed");
        assert_eq!(ack.width, cfg.width);
        assert_eq!(ack.height, cfg.height);
        assert!((ack.fps - cfg.fps).abs() < 0.01);
        assert_eq!(ack.duration_ms, cfg.duration_ms);
        assert_eq!(ack.handle_id, 1);

        let (count, first) = drain_after(&ring, 250);
        drop(handle); // join worker
        assert!(count > 0, "mock must deliver at least one frame in 250ms");
        let first = first.expect("first frame must be present");
        assert_eq!(
            first.len(),
            (cfg.width * cfg.height * 4) as usize,
            "frame size must equal width*height*4 (RGBA8)"
        );
    }

    #[test]
    fn avf_decoder_open_returns_not_implemented() {
        // Production stub: open MUST return Err(NotImplemented). Never panic.
        // The #![deny(clippy::todo)] lint at the crate root catches todo!() /
        // unimplemented!() — this test pins the runtime contract.
        let dev = AvfVideoDecoder::new();
        let ring = make_ring();
        let res = dev.open("file:///tmp/anything.mp4", ring);
        match res {
            Err(VideoError::NotImplemented) => {}
            other => panic!("expected NotImplemented, got {other:?}"),
        }
    }

    #[test]
    fn set_state_pause_then_play_round_trips() {
        // Open the mock; pause it; assert the ring drains down. Resume; assert
        // frames flow again. This pins the pause path through the worker
        // thread without flaking on tight timing.
        let cfg = MockVideoDecoderConfig {
            width: 4,
            height: 4,
            fps: 60.0,
            duration_ms: 5_000,
        };
        let dev = MockVideoDecoder::new(cfg);
        let ring = make_ring();
        let (_ack, mut handle) = dev
            .open("mock://gradient", Arc::clone(&ring))
            .expect("mock open must succeed");

        // Let it produce a few frames.
        std::thread::sleep(std::time::Duration::from_millis(120));
        handle
            .set_state(VideoState::Pause)
            .expect("pause must succeed");
        // Drain whatever was already queued; further frames should not arrive.
        std::thread::sleep(std::time::Duration::from_millis(60));
        while ring.pop().is_some() {}
        std::thread::sleep(std::time::Duration::from_millis(120));
        let stalled = {
            let mut n = 0;
            while ring.pop().is_some() {
                n += 1;
            }
            n
        };
        assert_eq!(
            stalled, 0,
            "while paused the ring must not gain frames, got {stalled}"
        );

        // Resume.
        handle
            .set_state(VideoState::Play)
            .expect("play must succeed");
        std::thread::sleep(std::time::Duration::from_millis(120));
        let resumed = {
            let mut n = 0;
            while ring.pop().is_some() {
                n += 1;
            }
            n
        };
        assert!(resumed > 0, "after resume frames must flow again");

        // Seek anywhere — must not error.
        handle
            .set_state(VideoState::Seek { position_ms: 1_000 })
            .expect("seek must succeed");
        drop(handle);
    }

    #[test]
    fn frame_layout_is_rgba8_packed() {
        // Spot-check that the gradient produces a buffer of the expected
        // shape and that the alpha byte is fully opaque (0xff). This pins
        // the wire format consumers parse against.
        let buf = render_gradient_frame(8, 4, 0);
        assert_eq!(buf.len(), 8 * 4 * 4);
        // Every 4th byte (the A channel) must be 0xff.
        for i in 0..(8 * 4) {
            assert_eq!(buf[i * 4 + 3], 0xff, "alpha at pixel {i} must be 0xff");
        }
    }

    #[test]
    fn factory_default_returns_avf_when_no_env() {
        // Sanity: with no env var, the factory must return the production
        // stub (which fails open() with NotImplemented). Tests run in
        // arbitrary process state — clear PLEXI_VIDEO first.
        // SAFETY: tests in this binary share a process; clearing here is safe
        // because no other test in this module reads PLEXI_VIDEO.
        unsafe {
            std::env::remove_var("PLEXI_VIDEO");
        }
        let dev = default_video_device();
        let ring = make_ring();
        match dev.open("file:///tmp/x.mp4", ring) {
            Err(VideoError::NotImplemented) => {}
            other => panic!("expected NotImplemented from default factory, got {other:?}"),
        }
    }

    #[test]
    fn prod_stub_methods_no_panic_smoke() {
        // Production-stub no-panic guarantee, mirrors the audio/midi pattern:
        // calling every trait method must surface an error, never panic.
        let dev = AvfVideoDecoder::new();
        let _ = dev.open("file:///x.mp4", make_ring());
        let _ = dev.open("", make_ring());
        let _ = dev.open("mock://gradient", make_ring());
    }
}
