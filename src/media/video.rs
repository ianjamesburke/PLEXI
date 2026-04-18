// PROD IMPL — AvfVideoDecoder is stubbed until Layer 4.
// MockVideoDecoder is a fully working implementation that generates
// procedural RGBA8 frames with a time-varying gradient.

use std::collections::HashMap;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Public supporting types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum VideoSource {
    File(PathBuf),
}

#[derive(Debug, Clone)]
pub struct VideoHandle {
    pub id: u64,
    pub width: u32,
    pub height: u32,
    /// Total duration in milliseconds.
    pub duration_ms: u64,
}

#[derive(Debug, Clone)]
pub enum PlaybackState {
    Play,
    Pause,
    /// Seek to position in milliseconds.
    Seek(u64),
}

#[derive(Debug, Clone)]
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    /// Presentation timestamp in milliseconds.
    pub pts_ms: u64,
    /// Raw RGBA8 pixel data, row-major, 4 bytes per pixel.
    pub rgba: Vec<u8>,
}

#[derive(Debug)]
pub enum VideoError {
    CodecUnsupported,
    FileNotFound,
    DecodeError(String),
}

impl std::fmt::Display for VideoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VideoError::CodecUnsupported => write!(f, "codec unsupported"),
            VideoError::FileNotFound => write!(f, "file not found"),
            VideoError::DecodeError(msg) => write!(f, "decode error: {msg}"),
        }
    }
}

impl std::error::Error for VideoError {}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

pub trait VideoDecoder: Send {
    /// Open a video source. Returns a handle describing the stream geometry.
    fn open(&mut self, source: VideoSource) -> Result<VideoHandle, VideoError>;

    /// Update the playback state for a handle (Play / Pause / Seek).
    fn set_state(&mut self, handle_id: u64, state: PlaybackState);

    /// Pull the next decoded RGBA8 frame. Returns `None` when the stream ends
    /// or the handle is paused with no buffered frame.
    fn next_frame(&mut self, handle_id: u64) -> Option<VideoFrame>;

    /// Release all resources associated with the handle.
    fn close(&mut self, handle_id: u64);
}

// ---------------------------------------------------------------------------
// AvfVideoDecoder — prod stub (Layer 4)
// ---------------------------------------------------------------------------

pub struct AvfVideoDecoder;

impl VideoDecoder for AvfVideoDecoder {
    fn open(&mut self, _source: VideoSource) -> Result<VideoHandle, VideoError> {
        log::warn!(
            "AvfVideoDecoder: open not yet implemented (Layer 4). \
                    Set PLEXI_VIDEO=mock://... to use the mock decoder."
        );
        Err(VideoError::DecodeError(
            "AvfVideoDecoder not implemented".into(),
        ))
    }

    fn set_state(&mut self, _handle_id: u64, _state: PlaybackState) {
        log::warn!("AvfVideoDecoder: set_state not yet implemented (Layer 4) — no-op");
    }

    fn next_frame(&mut self, _handle_id: u64) -> Option<VideoFrame> {
        None
    }

    fn close(&mut self, _handle_id: u64) {
        log::warn!("AvfVideoDecoder: close not yet implemented (Layer 4) — no-op");
    }
}

// ---------------------------------------------------------------------------
// MockVideoDecoder — working implementation
//
// Configuration via PLEXI_VIDEO env var:
//   mock://<path>?duration=<ms>&w=<width>&h=<height>
// Defaults: duration=5000, w=640, h=360
//
// Frame generation: 30 fps, time-varying gradient.
// State machine: Play / Pause / Seek all work deterministically.
// ---------------------------------------------------------------------------

const MOCK_FPS: u64 = 30;
const FRAME_DURATION_MS: u64 = 1000 / MOCK_FPS; // ≈33 ms

#[derive(Debug)]
struct MockStream {
    width: u32,
    height: u32,
    duration_ms: u64,
    pts_ms: u64,
    playing: bool,
    last_tick: std::time::Instant,
}

pub struct MockVideoDecoder {
    next_id: u64,
    streams: HashMap<u64, MockStream>,
    /// Default parameters parsed from the env var at construction time.
    default_duration_ms: u64,
    default_width: u32,
    default_height: u32,
}

impl MockVideoDecoder {
    pub fn new(duration_ms: u64, width: u32, height: u32) -> Self {
        Self {
            next_id: 1,
            streams: HashMap::new(),
            default_duration_ms: duration_ms,
            default_width: width,
            default_height: height,
        }
    }

    fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn generate_frame(width: u32, height: u32, pts_ms: u64) -> Vec<u8> {
        let pixel_count = (width * height) as usize;
        let mut rgba = Vec::with_capacity(pixel_count * 4);
        let t = pts_ms as u8; // wraps every 255 ms — gives visible motion

        for row in 0..height {
            for col in 0..width {
                let r = ((col * 255 / width.max(1)) as u8).wrapping_add(t);
                let g = ((row * 255 / height.max(1)) as u8).wrapping_add(t / 2);
                let b = t.wrapping_add(128);
                let a = 255u8;
                rgba.push(r);
                rgba.push(g);
                rgba.push(b);
                rgba.push(a);
            }
        }

        rgba
    }
}

impl VideoDecoder for MockVideoDecoder {
    fn open(&mut self, source: VideoSource) -> Result<VideoHandle, VideoError> {
        // The mock accepts any path — it doesn't read from disk.
        let _path = match &source {
            VideoSource::File(p) => p.clone(),
        };

        let id = self.alloc_id();
        let stream = MockStream {
            width: self.default_width,
            height: self.default_height,
            duration_ms: self.default_duration_ms,
            pts_ms: 0,
            playing: false,
            last_tick: std::time::Instant::now(),
        };

        let handle = VideoHandle {
            id,
            width: stream.width,
            height: stream.height,
            duration_ms: stream.duration_ms,
        };

        self.streams.insert(id, stream);
        Ok(handle)
    }

    fn set_state(&mut self, handle_id: u64, state: PlaybackState) {
        if let Some(stream) = self.streams.get_mut(&handle_id) {
            match state {
                PlaybackState::Play => {
                    stream.playing = true;
                    stream.last_tick = std::time::Instant::now();
                }
                PlaybackState::Pause => {
                    stream.playing = false;
                }
                PlaybackState::Seek(ms) => {
                    stream.pts_ms = ms.min(stream.duration_ms);
                    stream.last_tick = std::time::Instant::now();
                }
            }
        }
    }

    fn next_frame(&mut self, handle_id: u64) -> Option<VideoFrame> {
        let stream = self.streams.get_mut(&handle_id)?;

        if !stream.playing {
            return None;
        }

        // Advance pts by real elapsed time, clamped to frame boundaries.
        let elapsed_ms = stream.last_tick.elapsed().as_millis() as u64;
        if elapsed_ms < FRAME_DURATION_MS {
            return None; // Not yet time for the next frame.
        }

        // Advance by whole frames.
        let frames_elapsed = elapsed_ms / FRAME_DURATION_MS;
        stream.pts_ms += frames_elapsed * FRAME_DURATION_MS;
        stream.last_tick = std::time::Instant::now();

        if stream.pts_ms >= stream.duration_ms {
            stream.pts_ms = stream.duration_ms;
            stream.playing = false;
            return None; // Stream ended.
        }

        let pts_ms = stream.pts_ms;
        let (w, h) = (stream.width, stream.height);
        let rgba = Self::generate_frame(w, h, pts_ms);

        Some(VideoFrame {
            width: w,
            height: h,
            pts_ms,
            rgba,
        })
    }

    fn close(&mut self, handle_id: u64) {
        self.streams.remove(&handle_id);
    }
}

#[cfg(test)]
mod prod_stub_tests {
    use super::*;

    #[test]
    fn avf_open_returns_err_not_panic() {
        let mut d = AvfVideoDecoder;
        assert!(d
            .open(VideoSource::File(std::path::PathBuf::from(
                "/nonexistent.mp4"
            )))
            .is_err());
    }

    #[test]
    fn avf_set_state_is_noop_not_panic() {
        let mut d = AvfVideoDecoder;
        d.set_state(0, PlaybackState::Play);
    }

    #[test]
    fn avf_next_frame_returns_none_not_panic() {
        let mut d = AvfVideoDecoder;
        assert!(d.next_frame(0).is_none());
    }

    #[test]
    fn avf_close_is_noop_not_panic() {
        let mut d = AvfVideoDecoder;
        d.close(0);
    }
}
