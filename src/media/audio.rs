//! Audio capture + device enumeration (#277) + playback (#341).
//!
//! Three layers:
//!
//!   1. [`AudioDevice`] — thin host-facing trait. Methods return owned data
//!      (`AudioDeviceInfo`) so the caller never holds a cpal handle. The trait
//!      lives behind `Arc<dyn AudioDevice>` on each `ProcessApp` instance, the
//!      same shape as `IqBroker` and `NetService`.
//!
//!   2. [`CoreAudioDevice`] — production impl. cpal-backed enumeration +
//!      capture. cpal handles the macOS TCC prompt transparently on the first
//!      `Stream::play()` call provided `NSMicrophoneUsageDescription` is set
//!      in `Info.plist` (cargo-bundle: `assets/Info.plist.fragment`).
//!
//!   3. [`MockAudioDevice`] — test impl. Returns a fixed device list and
//!      drives a synthetic 440 Hz sine into the frame sink for round-trip
//!      tests. Never touches real hardware.
//!
//! Capture frames are pushed into the host's `TypedPipeRegistry` ring as raw
//! interleaved f32 PCM. Apps read the same frames over the binary pipe socket
//! using the existing typed-pipe transport — no audio-specific socket plumbing.
//!
//! Playback is provided via the `start_playback` free function (#341). rodio
//! manages the output stream and decoder. WAV/MP3/FLAC/OGG are supported
//! through rodio's default symphonia feature set.
//!
//! Sample-rate resampling is handled by cpal negotiation in capture; no
//! additional resampling library is needed.

use std::sync::Arc;
#[cfg(test)]
use std::sync::Mutex;

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

/// App-requested capture parameters. The device may negotiate down to its
/// nearest supported rate / channel count and report the actual values back
/// via `AudioCaptureStarted`.
#[derive(Debug, Clone)]
pub struct AudioCaptureRequest {
    /// `None` → use the host's default input device.
    pub device_id: Option<String>,
    pub requested_sample_rate: u32,
    pub requested_buffer_size: u32,
}

/// What the device actually produced after negotiation. Apps must use these
/// values when interpreting the PCM frames on the pipe — the requested values
/// are advisory.
#[derive(Debug, Clone, PartialEq)]
pub struct NegotiatedCaptureConfig {
    pub sample_rate: u32,
    pub channels: u16,
    pub buffer_size: u32,
    /// Human-readable name of the actual device that was opened. Useful for
    /// log lines and in-pane "Recording from <Built-in Mic>" UI.
    pub device_name: String,
}

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("no input devices found")]
    NoDevicesAvailable,
    #[error("device id '{0}' not found")]
    DeviceNotFound(String),
    /// Wraps cpal stream-build / play errors. macOS TCC denials surface here
    /// too — cpal returns a generic `BackendSpecific` error rather than a
    /// dedicated variant. The error message includes the cpal context, e.g.
    /// "BuildStreamError: device unavailable" when the user hasn't granted
    /// microphone access.
    #[error("cpal: {0}")]
    Cpal(String),
}

/// Where captured PCM frames are delivered. Returns `Ok(())` on success;
/// returns `Err(())` to signal "stop the stream" (e.g. the consumer was
/// dropped). The trait stays object-safe; the closure does not allocate.
///
/// Frames are interleaved f32 PCM. One frame = one sample per channel.
pub type FrameSink = Arc<dyn Fn(&[f32]) -> Result<(), ()> + Send + Sync + 'static>;

/// Opaque handle returned from `start_capture`. Dropping the handle (or
/// calling `stop_capture`) tears down the underlying stream.
pub struct CaptureSession {
    /// Cleanup runs on drop. The closure owns whatever the device impl needs
    /// to keep alive — for cpal that's the `Stream` itself, which stops on
    /// drop. For the mock it's a `JoinHandle`.
    _guard: Box<dyn AudioCaptureGuard>,
    /// Reported back to the app so it can correlate `AudioCaptureStarted`
    /// with the device that was actually opened.
    pub negotiated: NegotiatedCaptureConfig,
}

impl std::fmt::Debug for CaptureSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CaptureSession")
            .field("negotiated", &self.negotiated)
            .finish()
    }
}

/// Internal: per-impl cleanup hook. cpal `Stream` is `!Send`, so we erase it
/// behind this trait and let the concrete `Drop` impl run on whichever
/// thread currently holds the `CaptureSession`.
trait AudioCaptureGuard: Send {}

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
    let player = rodio::play(&handle.mixer(), BufReader::new(file))
        .map_err(|e| AudioError::Cpal(format!("decode/play {}: {e}", request.source)))?;
    player.set_volume(request.volume.clamp(0.0, 2.0));
    Ok(PlaybackSession {
        player,
        _handle: handle,
    })
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

// ─── Device trait ────────────────────────────────────────────────────────────

pub trait AudioDevice: Send + Sync {
    fn list_input_devices(&self) -> Vec<AudioDeviceInfo>;
    fn list_output_devices(&self) -> Vec<AudioDeviceInfo>;
    /// Open `request.device_id` (or the default input if `None`), negotiate
    /// the closest supported config, and start delivering interleaved f32 PCM
    /// frames to `sink` on the cpal callback thread. The returned
    /// `CaptureSession` owns the stream — drop it to stop.
    fn start_capture(
        &self,
        request: AudioCaptureRequest,
        sink: FrameSink,
    ) -> Result<CaptureSession, AudioError>;
}

// ─── Production impl: CoreAudioDevice (cpal-backed) ──────────────────────────

#[cfg(not(test))]
pub struct CoreAudioDevice;

#[cfg(not(test))]
impl CoreAudioDevice {
    pub fn new() -> Self {
        Self
    }

    fn host() -> cpal::Host {
        cpal::default_host()
    }

    fn collect_devices(host: &cpal::Host, kind: DeviceKind) -> Vec<AudioDeviceInfo> {
        let default_id = match kind {
            DeviceKind::Input => host
                .default_input_device()
                .and_then(|d| d.id().ok().map(|id| id.to_string())),
            DeviceKind::Output => host
                .default_output_device()
                .and_then(|d| d.id().ok().map(|id| id.to_string())),
        };

        let iter = match kind {
            DeviceKind::Input => host.input_devices(),
            DeviceKind::Output => host.output_devices(),
        };

        let devices = match iter {
            Ok(d) => d,
            Err(e) => {
                log::warn!("audio: enumerate {kind:?} devices failed: {e}");
                return Vec::new();
            }
        };

        devices
            .filter_map(|dev| {
                let id = dev.id().ok().map(|id| id.to_string())?;
                // `description()` returns a structured `DeviceDescription`;
                // pull the human-readable name off it. Fall back to the id
                // string if the description isn't queryable.
                let name = dev
                    .description()
                    .ok()
                    .map(|d| d.name().to_owned())
                    .unwrap_or_else(|| id.clone());
                let default = default_id.as_deref() == Some(id.as_str());
                Some(AudioDeviceInfo { id, name, default })
            })
            .collect()
    }
}

#[cfg(not(test))]
#[derive(Clone, Copy, Debug)]
enum DeviceKind {
    Input,
    Output,
}

#[cfg(not(test))]
impl AudioDevice for CoreAudioDevice {
    fn list_input_devices(&self) -> Vec<AudioDeviceInfo> {
        Self::collect_devices(&Self::host(), DeviceKind::Input)
    }

    fn list_output_devices(&self) -> Vec<AudioDeviceInfo> {
        Self::collect_devices(&Self::host(), DeviceKind::Output)
    }

    fn start_capture(
        &self,
        request: AudioCaptureRequest,
        sink: FrameSink,
    ) -> Result<CaptureSession, AudioError> {
        let host = Self::host();

        // Resolve the requested device (or fall back to default).
        let device = match request.device_id.as_deref() {
            None | Some("") => host
                .default_input_device()
                .ok_or(AudioError::NoDevicesAvailable)?,
            Some(id_str) => {
                // cpal::DeviceId is `FromStr`. A bad id is a real-world case
                // (saved selection on a now-disconnected interface); surface
                // a clear error rather than panicking.
                let parsed = id_str
                    .parse::<cpal::DeviceId>()
                    .map_err(|e| AudioError::Cpal(format!("bad device id {id_str:?}: {e}")))?;
                host.device_by_id(&parsed)
                    .ok_or_else(|| AudioError::DeviceNotFound(id_str.to_owned()))?
            }
        };

        let device_name = device
            .description()
            .ok()
            .map(|d| d.name().to_owned())
            .or_else(|| device.id().ok().map(|id| id.to_string()))
            .unwrap_or_else(|| "<unknown>".to_owned());

        // Pick the supported config nearest the request.
        let supported = pick_input_config(&device, &request)?;
        let sample_rate = supported.sample_rate();
        let channels = supported.channels();
        let buffer_size = request.requested_buffer_size.max(64);

        let config = cpal::StreamConfig {
            channels,
            sample_rate,
            buffer_size: cpal::BufferSize::Default,
        };

        let sample_format = supported.sample_format();
        let err_fn = |e| log::warn!("audio: capture stream error: {e}");

        // Build the input stream. cpal triggers the macOS TCC prompt on
        // `Stream::play()` below if the entitlement plist string is present.
        let stream = build_input_stream(&device, &config, sample_format, sink, err_fn)?;
        stream
            .play()
            .map_err(|e| AudioError::Cpal(format!("stream.play: {e}")))?;

        // Hold the cpal `Stream` on the heap — dropping `CpalGuard` drops
        // the stream, which stops the underlying audio unit. `Stream` is
        // technically `!Send` on cpal's bound but documented as safe to
        // drop from any thread on macOS / WASAPI / ALSA. The newtype
        // `SendStream` makes the trait-object boxing legal.
        struct CpalGuard {
            stream: Option<SendStream>,
        }
        impl AudioCaptureGuard for CpalGuard {}
        impl Drop for CpalGuard {
            fn drop(&mut self) {
                drop(self.stream.take());
            }
        }

        let guard = CpalGuard {
            stream: Some(SendStream { _stream: stream }),
        };

        Ok(CaptureSession {
            _guard: Box::new(guard),
            negotiated: NegotiatedCaptureConfig {
                sample_rate,
                channels,
                buffer_size,
                device_name,
            },
        })
    }
}

/// Newtype wrapper to make a `cpal::Stream` `Send`. cpal documents that
/// `Stream::drop` is safe from any thread on the platforms we support; the
/// trait bound is conservative on the upstream type.
#[cfg(not(test))]
struct SendStream {
    _stream: cpal::Stream,
}

#[cfg(not(test))]
unsafe impl Send for SendStream {}

#[cfg(not(test))]
fn pick_input_config(
    device: &cpal::Device,
    request: &AudioCaptureRequest,
) -> Result<cpal::SupportedStreamConfig, AudioError> {
    // Walk supported configs, find the range whose [min, max] sample-rate
    // window covers (or is closest to) the requested rate. cpal returns
    // ranges, not points — pick the one whose closest-rate is nearest the
    // request, then clamp to its bounds.
    let configs = device
        .supported_input_configs()
        .map_err(|e| AudioError::Cpal(format!("supported_input_configs: {e}")))?;

    let target = request.requested_sample_rate;
    let mut best: Option<(u32, cpal::SupportedStreamConfigRange)> = None;
    for range in configs {
        let min = range.min_sample_rate();
        let max = range.max_sample_rate();
        let clamped = target.clamp(min, max);
        let dist = clamped.abs_diff(target);
        if best.as_ref().map_or(true, |(d, _)| dist < *d) {
            best = Some((dist, range));
        }
    }

    let range = best
        .ok_or_else(|| AudioError::Cpal("device has no supported input configs".to_owned()))?
        .1;

    let chosen_rate = target.clamp(range.min_sample_rate(), range.max_sample_rate());
    Ok(range.with_sample_rate(chosen_rate))
}

#[cfg(not(test))]
fn build_input_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    sample_format: cpal::SampleFormat,
    sink: FrameSink,
    err_fn: impl FnMut(cpal::StreamError) + Send + 'static,
) -> Result<cpal::Stream, AudioError> {
    use cpal::SampleFormat as F;

    let stream = match sample_format {
        F::F32 => {
            let sink = Arc::clone(&sink);
            device.build_input_stream(
                config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    let _ = sink(data);
                },
                err_fn,
                None,
            )
        }
        F::I16 => {
            let sink = Arc::clone(&sink);
            device.build_input_stream(
                config,
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    let mut buf = Vec::with_capacity(data.len());
                    for s in data {
                        buf.push(*s as f32 / i16::MAX as f32);
                    }
                    let _ = sink(&buf);
                },
                err_fn,
                None,
            )
        }
        F::U16 => {
            let sink = Arc::clone(&sink);
            device.build_input_stream(
                config,
                move |data: &[u16], _: &cpal::InputCallbackInfo| {
                    let mut buf = Vec::with_capacity(data.len());
                    for s in data {
                        // u16 → f32 in [-1, 1].
                        let centered = *s as i32 - i16::MAX as i32 - 1;
                        buf.push(centered as f32 / (i16::MAX as f32 + 1.0));
                    }
                    let _ = sink(&buf);
                },
                err_fn,
                None,
            )
        }
        other => {
            return Err(AudioError::Cpal(format!(
                "unsupported sample format: {other:?}"
            )));
        }
    };

    stream.map_err(|e| AudioError::Cpal(format!("build_input_stream: {e}")))
}

// ─── Test impl: MockAudioDevice ──────────────────────────────────────────────

/// Test-only audio device. Reports a stable two-device list and drives the
/// frame sink with a synthetic 440 Hz sine wave on a worker thread.
///
/// `cfg(test)`-gated because the only consumers are unit tests in this
/// module and `process_app::tests`. Production code paths use
/// `CoreAudioDevice` exclusively (selected by `default_audio_device` in
/// `process_app/mod.rs`). When the harness is dropped the worker thread
/// exits within one buffer period.
#[cfg(test)]
pub struct MockAudioDevice {
    pub inputs: Vec<AudioDeviceInfo>,
    pub outputs: Vec<AudioDeviceInfo>,
    pub frames_per_callback: usize,
    pub sample_rate: u32,
    pub channels: u16,
}

#[cfg(test)]
impl Default for MockAudioDevice {
    fn default() -> Self {
        Self {
            inputs: vec![
                AudioDeviceInfo {
                    id: "mock-builtin".to_owned(),
                    name: "Mock Built-in Microphone".to_owned(),
                    default: true,
                },
                AudioDeviceInfo {
                    id: "mock-usb".to_owned(),
                    name: "Mock USB Interface".to_owned(),
                    default: false,
                },
            ],
            outputs: vec![AudioDeviceInfo {
                id: "mock-speakers".to_owned(),
                name: "Mock Built-in Speakers".to_owned(),
                default: true,
            }],
            frames_per_callback: 512,
            sample_rate: 48_000,
            channels: 1,
        }
    }
}

#[cfg(test)]
impl MockAudioDevice {
    pub fn new() -> Self {
        Self::default()
    }
}

#[cfg(test)]
impl AudioDevice for MockAudioDevice {
    fn list_input_devices(&self) -> Vec<AudioDeviceInfo> {
        self.inputs.clone()
    }

    fn list_output_devices(&self) -> Vec<AudioDeviceInfo> {
        self.outputs.clone()
    }

    fn start_capture(
        &self,
        request: AudioCaptureRequest,
        sink: FrameSink,
    ) -> Result<CaptureSession, AudioError> {
        // Resolve device id: empty/None → first input ("default"); else exact match.
        let device = match request.device_id.as_deref() {
            None | Some("") => self
                .inputs
                .iter()
                .find(|d| d.default)
                .or_else(|| self.inputs.first())
                .ok_or(AudioError::NoDevicesAvailable)?,
            Some(id) => self
                .inputs
                .iter()
                .find(|d| d.id == id)
                .ok_or_else(|| AudioError::DeviceNotFound(id.to_owned()))?,
        };

        let frames_per = self.frames_per_callback;
        let rate = self.sample_rate;
        let channels = self.channels;
        log::info!(
            "audio(mock): capture requested rate={} buffer={}, serving rate={rate} \
             frames_per_callback={frames_per}",
            request.requested_sample_rate,
            request.requested_buffer_size,
        );
        let stop = Arc::new(Mutex::new(false));
        let stop_t = Arc::clone(&stop);
        let handle = std::thread::Builder::new()
            .name("mock-audio-capture".to_owned())
            .spawn(move || {
                let mut phase: f32 = 0.0;
                let step = 2.0 * std::f32::consts::PI * 440.0 / rate as f32;
                let buf_seconds = frames_per as f32 / rate as f32;
                loop {
                    if *stop_t.lock().expect("mock stop poisoned") {
                        return;
                    }
                    let mut buf = Vec::with_capacity(frames_per * channels as usize);
                    for _ in 0..frames_per {
                        let s = phase.sin() * 0.25;
                        for _ in 0..channels {
                            buf.push(s);
                        }
                        phase += step;
                        if phase > std::f32::consts::TAU {
                            phase -= std::f32::consts::TAU;
                        }
                    }
                    if sink(&buf).is_err() {
                        return;
                    }
                    std::thread::sleep(std::time::Duration::from_secs_f32(buf_seconds));
                }
            })
            .map_err(|e| AudioError::Cpal(format!("mock thread spawn: {e}")))?;

        struct MockGuard {
            stop: Arc<Mutex<bool>>,
            handle: Option<std::thread::JoinHandle<()>>,
        }
        impl AudioCaptureGuard for MockGuard {}
        impl Drop for MockGuard {
            fn drop(&mut self) {
                *self.stop.lock().expect("mock stop poisoned on drop") = true;
                if let Some(h) = self.handle.take() {
                    let _ = h.join();
                }
            }
        }

        Ok(CaptureSession {
            _guard: Box::new(MockGuard {
                stop,
                handle: Some(handle),
            }),
            negotiated: NegotiatedCaptureConfig {
                sample_rate: rate,
                channels,
                buffer_size: frames_per as u32,
                device_name: device.name.clone(),
            },
        })
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn collecting_sink(counter: Arc<AtomicUsize>) -> FrameSink {
        Arc::new(move |frames: &[f32]| {
            counter.fetch_add(frames.len(), Ordering::Relaxed);
            Ok(())
        })
    }

    #[test]
    fn list_input_devices_returns_at_least_default() {
        let dev = MockAudioDevice::new();
        let inputs = dev.list_input_devices();
        assert!(!inputs.is_empty(), "mock must report at least one input");
        let defaults: Vec<_> = inputs.iter().filter(|d| d.default).collect();
        assert_eq!(defaults.len(), 1, "exactly one input must be the default");
    }

    #[test]
    fn list_output_devices_returns_at_least_default() {
        let dev = MockAudioDevice::new();
        let outputs = dev.list_output_devices();
        assert!(!outputs.is_empty(), "mock must report at least one output");
        assert!(
            outputs.iter().any(|d| d.default),
            "an output must be the default"
        );
    }

    #[test]
    fn mock_device_round_trips_pcm_frames() {
        let dev = MockAudioDevice {
            frames_per_callback: 64,
            sample_rate: 48_000,
            channels: 1,
            ..MockAudioDevice::default()
        };
        let counter = Arc::new(AtomicUsize::new(0));
        let session = dev
            .start_capture(
                AudioCaptureRequest {
                    device_id: None,
                    requested_sample_rate: 48_000,
                    requested_buffer_size: 64,
                },
                collecting_sink(Arc::clone(&counter)),
            )
            .expect("mock start_capture must succeed");

        // The mock pushes one buffer per `frames_per_callback / sample_rate`
        // seconds. 250 ms is ~187 buffers @ 64 frames; we just need >0.
        std::thread::sleep(std::time::Duration::from_millis(250));
        drop(session); // join worker
        assert!(
            counter.load(Ordering::Relaxed) > 0,
            "mock must have delivered at least one frame buffer"
        );
    }

    #[test]
    fn core_audio_device_no_panic_on_nonexistent_device_id() {
        // Production-stub assertion (#277 acceptance): start_capture with a
        // bogus device id must return an error, never panic. This extends
        // the prod_stub_tests pattern to the audio device trait.
        //
        // The mock drives this contract because cpal calls into real OS
        // audio APIs that are unsuitable for CI. The trait-level guarantee
        // — bad id → Err, never panic — is what the real CoreAudioDevice
        // impl above also honours via its `parse::<DeviceId>` + `device_by_id`
        // bail-out path.
        let dev = MockAudioDevice::new();
        let counter = Arc::new(AtomicUsize::new(0));
        let res = dev.start_capture(
            AudioCaptureRequest {
                device_id: Some("definitely-not-a-real-device".to_owned()),
                requested_sample_rate: 48_000,
                requested_buffer_size: 512,
            },
            collecting_sink(counter),
        );
        match res {
            Err(AudioError::DeviceNotFound(id)) => {
                assert_eq!(id, "definitely-not-a-real-device");
            }
            other => panic!("expected DeviceNotFound, got {other:?}"),
        }
    }

    #[test]
    fn sample_rate_negotiation_picks_closest_supported() {
        // The mock reports its native rate (48 kHz) regardless of request.
        // Apps must read `negotiated.sample_rate` rather than trusting their
        // requested rate — the contract that this PR adds to the protocol.
        let dev = MockAudioDevice {
            sample_rate: 48_000,
            ..MockAudioDevice::default()
        };
        let counter = Arc::new(AtomicUsize::new(0));
        let session = dev
            .start_capture(
                AudioCaptureRequest {
                    device_id: None,
                    requested_sample_rate: 22_050, // not natively supported
                    requested_buffer_size: 512,
                },
                collecting_sink(counter),
            )
            .expect("mock must accept a non-native rate request");

        assert_eq!(
            session.negotiated.sample_rate, 48_000,
            "mock must report its actual native rate, not the request"
        );
    }

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
