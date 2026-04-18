//! Media subsystem — host-owned audio and video primitives.
//!
//! Factory functions read env vars at call time and return the appropriate
//! implementation (prod or mock). Prod impls are stubbed until Layer 4.
//!
//! # Environment variables
//!
//! | Var            | Format                            | Effect                         |
//! |----------------|-----------------------------------|--------------------------------|
//! | `PLEXI_AUDIO`  | `mock://<in.wav>,<out.wav>`       | Returns `MockAudioDevice`      |
//! | `PLEXI_VIDEO`  | `mock://<path>?duration=<ms>&w=<width>&h=<height>` | Returns `MockVideoDecoder` |
//!
//! Any other value (or absent) returns the prod impl.

pub mod audio;
pub mod video;

pub use audio::{AudioCaptureHandle, AudioDevice, AudioPlaybackHandle, AudioSource,
                CoreAudioDevice, MockAudioDevice};
pub use video::{AvfVideoDecoder, MockVideoDecoder, PlaybackState, VideoDecoder,
                VideoFrame, VideoHandle, VideoSource};

/// Create an `AudioDevice` appropriate for the current environment.
///
/// - `PLEXI_AUDIO=mock://<in_wav>,<out_wav>` → `MockAudioDevice`
/// - Otherwise → `CoreAudioDevice` (prod stub until Layer 4)
pub fn audio_device() -> Box<dyn AudioDevice> {
    match std::env::var("PLEXI_AUDIO") {
        Ok(val) if val.starts_with("mock://") => {
            let rest = val.trim_start_matches("mock://");
            let mut parts = rest.splitn(2, ',');
            let in_wav = parts.next().unwrap_or("").trim().into();
            let out_wav = parts.next().unwrap_or("").trim().into();
            log::info!("media: using MockAudioDevice (in={in_wav:?}, out={out_wav:?})");
            Box::new(MockAudioDevice::new(in_wav, out_wav))
        }
        _ => {
            log::info!("media: using CoreAudioDevice (prod)");
            Box::new(CoreAudioDevice)
        }
    }
}

/// Create a `VideoDecoder` appropriate for the current environment.
///
/// - `PLEXI_VIDEO=mock://<path>?duration=<ms>&w=<w>&h=<h>` → `MockVideoDecoder`
/// - Otherwise → `AvfVideoDecoder` (prod stub until Layer 4)
///
/// Query parameters are all optional; defaults: `duration=5000`, `w=640`, `h=360`.
pub fn video_decoder() -> Box<dyn VideoDecoder> {
    match std::env::var("PLEXI_VIDEO") {
        Ok(val) if val.starts_with("mock://") => {
            let rest = val.trim_start_matches("mock://");
            // Split path from query string.
            let query = if let Some(q) = rest.split_once('?') { q.1 } else { "" };

            let mut duration_ms: u64 = 5000;
            let mut width: u32 = 640;
            let mut height: u32 = 360;

            for kv in query.split('&') {
                let mut it = kv.splitn(2, '=');
                match (it.next(), it.next()) {
                    (Some("duration"), Some(v)) => {
                        if let Ok(n) = v.parse() { duration_ms = n; }
                    }
                    (Some("w"), Some(v)) => {
                        if let Ok(n) = v.parse() { width = n; }
                    }
                    (Some("h"), Some(v)) => {
                        if let Ok(n) = v.parse() { height = n; }
                    }
                    _ => {}
                }
            }

            log::info!(
                "media: using MockVideoDecoder (duration={duration_ms}ms, {width}x{height})"
            );
            Box::new(MockVideoDecoder::new(duration_ms, width, height))
        }
        _ => {
            log::info!("media: using AvfVideoDecoder (prod)");
            Box::new(AvfVideoDecoder)
        }
    }
}
