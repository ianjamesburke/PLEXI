//! Audio playback (#341) + real-time output streaming (WASM audio).
//!
//! [`AudioDeviceInfo`] is the on-the-wire device row; it is still referenced by
//! `protocol::AudioDeviceWire`. The `AudioDevice` trait, its cpal-backed
//! `CoreAudioDevice` impl and the mic-capture path they served were never
//! reachable from any build configuration and were deleted by stint 0750, along
//! with the CoreMIDI stack in `crate::media::midi`. The `list_audio_devices` /
//! `audio_devices_listed` protocol pair is still declared, still described by
//! the generated schema and the Python SDK, and still has no host handler.
//!
//! Playback is provided via the `start_playback` free function (#341). rodio
//! manages the output stream and decoder. WAV/MP3/FLAC/OGG are supported
//! through rodio's default symphonia feature set.
//!
//! `start_output_stream` is the synthesised-audio path: the host UI thread tops
//! up a lock-free ring that the cpal output callback drains on the RT thread.

use std::sync::Arc;

#[cfg(not(test))]
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

// ─── Public types ────────────────────────────────────────────────────────────

/// Stable info row for one audio device. Returned by enumeration; consumed by
/// apps populating an input-device dropdown.
///
/// `id` is the cpal `DeviceId` rendered to a string. It is stable across
/// reboots on macOS (CoreAudio device UID) but NOT across machines. Apps that
/// persist a "last selected device" should fall back to `default = true`
/// when the saved id is no longer present.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AudioDeviceInfo {
    pub id: String,
    pub name: String,
    pub default: bool,
}

/// Failure modes of the production audio paths. Both variants are raised only
/// by `cfg(not(test))` code — the test-build stubs never touch hardware, so
/// under `cfg(test)` this enum is deliberately uninhabited.
#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("no output devices found")]
    #[cfg(not(test))]
    NoDevicesAvailable,
    /// Wraps cpal stream-build / play errors. macOS TCC denials surface here
    /// too — cpal returns a generic `BackendSpecific` error rather than a
    /// dedicated variant. The error message includes the cpal context, e.g.
    /// "BuildStreamError: device unavailable" when the user hasn't granted
    /// microphone access.
    #[error("cpal: {0}")]
    #[cfg(not(test))]
    Cpal(String),
}

// ─── Playback types (#341) ───────────────────────────────────────────────────

/// Request parameters for audio playback.
#[derive(Debug, Clone)]
pub struct PlaybackRequest {
    /// Path to an audio file (WAV, MP3, FLAC, OGG supported via rodio).
    pub source: String,
    /// Playback volume in [0.0, 2.0]. Values are clamped on use.
    pub volume: f32,
}

/// Opaque handle returned from `start_playback`. Dropping it stops playback.
#[cfg(not(test))]
pub struct PlaybackSession {
    player: rodio::Player,
    _handle: rodio::MixerDeviceSink,
}

#[cfg(not(test))]
impl PlaybackSession {
    pub fn pause(&self) {
        self.player.pause();
    }
    pub fn resume(&self) {
        self.player.play();
    }
    pub fn set_volume(&self, v: f32) {
        self.player.set_volume(v);
    }
    pub fn seek(&self, position: std::time::Duration) -> Result<(), String> {
        self.player.try_seek(position).map_err(|e| e.to_string())
    }
    pub fn position(&self) -> std::time::Duration {
        self.player.get_pos()
    }
}

#[cfg(not(test))]
impl std::fmt::Debug for PlaybackSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlaybackSession").finish()
    }
}

/// Production playback implementation via rodio (#341).
///
/// Opens the system default output device, decodes the file at
/// `request.source`, and begins playback immediately. The returned
/// `PlaybackSession` keeps the output stream alive — drop it to stop.
#[cfg(not(test))]
pub fn start_playback(request: PlaybackRequest) -> Result<PlaybackSession, AudioError> {
    use std::fs::File;
    use std::io::BufReader;

    let handle = rodio::DeviceSinkBuilder::open_default_sink()
        .map_err(|e| AudioError::Cpal(format!("rodio: output stream: {e}")))?;
    let file = File::open(&request.source)
        .map_err(|e| AudioError::Cpal(format!("open {}: {e}", request.source)))?;
    let player = rodio::play(handle.mixer(), BufReader::new(file))
        .map_err(|e| AudioError::Cpal(format!("decode/play {}: {e}", request.source)))?;
    player.set_volume(request.volume.clamp(0.0, 2.0));
    Ok(PlaybackSession {
        player,
        _handle: handle,
    })
}

#[cfg(not(test))]
pub fn source_duration_ms(source: &str) -> Option<u64> {
    use rodio::Source;

    let file = std::fs::File::open(source).ok()?;
    let decoder = rodio::Decoder::try_from(file).ok()?;
    decoder
        .total_duration()
        .map(|duration| duration.as_millis() as u64)
}

/// Test stub — playback is not exercised in unit tests; real hardware is not
/// available in CI. The stub returns `Ok` unconditionally so routing tests
/// can exercise the `AudioPlay` handler path without hardware.
#[cfg(test)]
pub struct PlaybackSession {
    _phantom: (),
}

#[cfg(test)]
impl PlaybackSession {
    pub fn pause(&self) {}
    pub fn resume(&self) {}
    pub fn set_volume(&self, _v: f32) {}
    pub fn seek(&self, _position: std::time::Duration) -> Result<(), String> {
        Ok(())
    }
    pub fn position(&self) -> std::time::Duration {
        std::time::Duration::ZERO
    }
}

#[cfg(test)]
impl std::fmt::Debug for PlaybackSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PlaybackSession(mock)")
    }
}

#[cfg(test)]
pub fn start_playback(request: PlaybackRequest) -> Result<PlaybackSession, AudioError> {
    log::info!(
        "audio(mock): start_playback source={} volume={}",
        request.source,
        request.volume
    );
    Ok(PlaybackSession { _phantom: () })
}

#[cfg(test)]
pub fn source_duration_ms(_source: &str) -> Option<u64> {
    None
}

// ─── Real-time output stream (WASM audio, G12) ────────────────────────────────
//
// A pull-based output stream for synthesised audio. The producer (the host UI
// thread, which owns the wasmtime Store) tops up a lock-free `ArrayQueue<f32>`
// ring by calling the guest's `process-output`; the cpal output callback (RT
// thread) pops interleaved f32 samples from the same ring. The RT thread never
// touches the Store and never locks — an empty ring yields silence, never a
// stall. This is the live counterpart to the synchronous G12 gate.

use crossbeam_queue::ArrayQueue;

/// Newtype wrapper to make a `cpal::Stream` `Send`. cpal documents that
/// `Stream::drop` is safe from any thread on the platforms we support; the
/// trait bound is conservative on the upstream type.
#[cfg(not(test))]
struct SendStream {
    _stream: cpal::Stream,
}

#[cfg(not(test))]
unsafe impl Send for SendStream {}

/// Handle owning a live output stream. Drop to stop playback.
#[cfg(not(test))]
pub struct OutputSession {
    _stream: SendStream,
    /// Config the device actually negotiated. The producer must synthesise at
    /// these values, not the requested ones, or pitch/throughput will drift.
    pub sample_rate: u32,
    pub channels: u32,
}

/// Open the default output device, negotiate the nearest config to the request,
/// and start a callback that drains `ring`. Returns the negotiated config so the
/// caller can synthesise at the right rate/channel count.
#[cfg(not(test))]
pub fn start_output_stream(
    requested_sample_rate: u32,
    requested_channels: u16,
    ring: Arc<ArrayQueue<f32>>,
) -> Result<OutputSession, AudioError> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or(AudioError::NoDevicesAvailable)?;

    let supported = pick_output_config(&device, requested_sample_rate, requested_channels)?;
    let sample_format = supported.sample_format();
    if sample_format != cpal::SampleFormat::F32 {
        // CoreAudio is natively f32; other formats would need conversion in the
        // RT callback. Fail clearly rather than emit garbage.
        return Err(AudioError::Cpal(format!(
            "unsupported output sample format: {sample_format:?}"
        )));
    }
    let sample_rate = supported.sample_rate();
    let channels = supported.channels();
    let config = cpal::StreamConfig {
        channels,
        sample_rate,
        buffer_size: cpal::BufferSize::Default,
    };
    let err_fn = |e| log::warn!("audio: output stream error: {e}");

    let stream = device
        .build_output_stream(
            &config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                // RT thread: pop pre-rendered samples; underrun → silence.
                for s in data.iter_mut() {
                    *s = ring.pop().unwrap_or(0.0);
                }
            },
            err_fn,
            None,
        )
        .map_err(|e| AudioError::Cpal(format!("build_output_stream: {e}")))?;
    stream
        .play()
        .map_err(|e| AudioError::Cpal(format!("output stream.play: {e}")))?;
    log::info!("audio: output stream playing ({sample_rate} Hz, {channels} ch)");

    Ok(OutputSession {
        _stream: SendStream { _stream: stream },
        sample_rate,
        channels: channels as u32,
    })
}

#[cfg(not(test))]
fn pick_output_config(
    device: &cpal::Device,
    rate: u32,
    channels: u16,
) -> Result<cpal::SupportedStreamConfig, AudioError> {
    let configs = device
        .supported_output_configs()
        .map_err(|e| AudioError::Cpal(format!("supported_output_configs: {e}")))?;

    // Prefer a range that supports the requested channel count, then the one
    // whose sample-rate window is nearest the request.
    let mut best: Option<(u32, cpal::SupportedStreamConfigRange)> = None;
    for range in configs {
        let min = range.min_sample_rate();
        let max = range.max_sample_rate();
        let clamped = rate.clamp(min, max);
        let channel_penalty = if range.channels() == channels {
            0
        } else {
            100_000
        };
        let dist = clamped.abs_diff(rate) + channel_penalty;
        if best.as_ref().is_none_or(|(d, _)| dist < *d) {
            best = Some((dist, range));
        }
    }
    let range = best
        .ok_or_else(|| AudioError::Cpal("device has no supported output configs".to_owned()))?
        .1;
    let chosen = rate.clamp(range.min_sample_rate(), range.max_sample_rate());
    Ok(range.with_sample_rate(chosen))
}

/// Test stub — output playback touches CoreAudio, which is unavailable in CI.
/// Returns a no-op session reporting the requested config so producer code can
/// be exercised without hardware.
#[cfg(test)]
pub struct OutputSession {
    pub sample_rate: u32,
    pub channels: u32,
}

#[cfg(test)]
pub fn start_output_stream(
    requested_sample_rate: u32,
    requested_channels: u16,
    _ring: Arc<ArrayQueue<f32>>,
) -> Result<OutputSession, AudioError> {
    log::info!(
        "audio(mock): start_output_stream rate={requested_sample_rate} ch={requested_channels}"
    );
    Ok(OutputSession {
        sample_rate: requested_sample_rate,
        channels: requested_channels as u32,
    })
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playback_stub_does_not_panic() {
        // Production-stub assertion (#341): start_playback with any path must
        // return Ok (test stub) and never panic. The production impl opens
        // real hardware; the test stub returns Ok unconditionally so routing
        // tests can exercise the AudioPlay handler path without hardware.
        let req = PlaybackRequest {
            source: "/nonexistent.wav".to_owned(),
            volume: 1.0,
        };
        let result = start_playback(req);
        // In test mode the stub returns Ok unconditionally — no hardware needed.
        assert!(result.is_ok(), "playback stub must return Ok: {result:?}");
    }
}
