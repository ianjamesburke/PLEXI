// PROD IMPL — CoreAudioDevice is stubbed until Layer 4.
// MockAudioDevice is a fully working implementation driven by WAV files.

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

// ---------------------------------------------------------------------------
// Public supporting types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum AudioSource {
    File(PathBuf),
    /// Binary pipe handle by ID (Layer 4+ plumbing; stub for now).
    Pipe(String),
}

#[derive(Debug)]
pub enum AudioError {
    DeviceUnavailable,
    UnsupportedFormat,
    StreamError(String),
}

impl std::fmt::Display for AudioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AudioError::DeviceUnavailable => write!(f, "audio device unavailable"),
            AudioError::UnsupportedFormat => write!(f, "unsupported audio format"),
            AudioError::StreamError(msg) => write!(f, "stream error: {msg}"),
        }
    }
}

impl std::error::Error for AudioError {}

/// Owning handle to an active capture stream. Interleaved f32 samples arrive
/// through `receiver`. Dropping this handle does NOT automatically stop the
/// underlying stream — call `AudioDevice::stop(handle.id)` first.
pub struct AudioCaptureHandle {
    pub id: u64,
    pub receiver: mpsc::Receiver<Vec<f32>>,
}

/// Owning handle to an active playback stream. Send `()` on `stop_tx` to
/// request early termination.
pub struct AudioPlaybackHandle {
    pub id: u64,
    pub stop_tx: mpsc::Sender<()>,
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

pub trait AudioDevice {
    /// Begin capturing audio at the given sample rate. Returns a handle whose
    /// `receiver` yields interleaved f32 PCM buffers of `buffer_size` frames.
    fn start_capture(
        &mut self,
        sample_rate: u32,
        buffer_size: u32,
    ) -> Result<AudioCaptureHandle, AudioError>;

    /// Begin playback from `source` at the given `volume` (0.0 – 1.0).
    fn start_playback(
        &mut self,
        source: AudioSource,
        volume: f32,
    ) -> Result<AudioPlaybackHandle, AudioError>;

    /// Stop a capture or playback stream by handle id.
    fn stop(&mut self, handle_id: u64);
}

// ---------------------------------------------------------------------------
// CoreAudioDevice — prod stub (Layer 4)
// ---------------------------------------------------------------------------

pub struct CoreAudioDevice;

impl AudioDevice for CoreAudioDevice {
    fn start_capture(
        &mut self,
        _sample_rate: u32,
        _buffer_size: u32,
    ) -> Result<AudioCaptureHandle, AudioError> {
        log::warn!(
            "CoreAudioDevice: start_capture not yet implemented (Layer 4). \
                    Set PLEXI_AUDIO=mock://<in.wav>,<out.wav> to use the mock device."
        );
        Err(AudioError::DeviceUnavailable)
    }

    fn start_playback(
        &mut self,
        _source: AudioSource,
        _volume: f32,
    ) -> Result<AudioPlaybackHandle, AudioError> {
        log::warn!(
            "CoreAudioDevice: start_playback not yet implemented (Layer 4). \
                    Set PLEXI_AUDIO=mock://<in.wav>,<out.wav> to use the mock device."
        );
        Err(AudioError::DeviceUnavailable)
    }

    fn stop(&mut self, _handle_id: u64) {
        log::warn!("CoreAudioDevice: stop not yet implemented (Layer 4) — no-op");
    }
}

// ---------------------------------------------------------------------------
// MockAudioDevice — working implementation backed by WAV files
// ---------------------------------------------------------------------------

/// A mock audio device driven entirely by WAV files.
///
/// - Capture: reads `in_wav`, feeds f32 sample buffers through an mpsc channel
///   at realtime pace (sleeping `buffer_size / sample_rate` seconds between sends).
/// - Playback: reads from the configured source WAV and writes decoded samples
///   to `out_wav`.
pub struct MockAudioDevice {
    /// Path to the WAV file used as the capture source.
    pub in_wav: PathBuf,
    /// Path where playback output is written.
    pub out_wav: PathBuf,
    next_id: u64,
}

impl MockAudioDevice {
    pub fn new(in_wav: PathBuf, out_wav: PathBuf) -> Self {
        Self {
            in_wav,
            out_wav,
            next_id: 1,
        }
    }

    fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
}

impl AudioDevice for MockAudioDevice {
    fn start_capture(
        &mut self,
        sample_rate: u32,
        buffer_size: u32,
    ) -> Result<AudioCaptureHandle, AudioError> {
        let id = self.alloc_id();
        let in_wav = self.in_wav.clone();

        let (tx, rx) = mpsc::channel::<Vec<f32>>();

        std::thread::spawn(move || {
            let reader = match hound::WavReader::open(&in_wav) {
                Ok(r) => r,
                Err(e) => {
                    log::warn!("MockAudioDevice: failed to open capture WAV {in_wav:?}: {e}");
                    return;
                }
            };

            let spec = reader.spec();
            let frame_duration_ms =
                (buffer_size as f64 / sample_rate.max(1) as f64 * 1000.0) as u64;

            // Collect all samples converted to f32.
            let samples: Vec<f32> = match spec.sample_format {
                hound::SampleFormat::Float => reader
                    .into_samples::<f32>()
                    .filter_map(|s| s.ok())
                    .collect(),
                hound::SampleFormat::Int => {
                    let max_val = (1i64 << (spec.bits_per_sample - 1)) as f32;
                    match spec.bits_per_sample {
                        16 => reader
                            .into_samples::<i16>()
                            .filter_map(|s| s.ok())
                            .map(|s| s as f32 / max_val)
                            .collect(),
                        24 | 32 => reader
                            .into_samples::<i32>()
                            .filter_map(|s| s.ok())
                            .map(|s| s as f32 / max_val)
                            .collect(),
                        _ => {
                            log::warn!(
                                "MockAudioDevice: unsupported bit depth {}",
                                spec.bits_per_sample
                            );
                            return;
                        }
                    }
                }
            };

            let chunk = (buffer_size * spec.channels as u32) as usize;
            for window in samples.chunks(chunk) {
                if tx.send(window.to_vec()).is_err() {
                    break; // receiver dropped
                }
                std::thread::sleep(Duration::from_millis(frame_duration_ms));
            }
        });

        Ok(AudioCaptureHandle { id, receiver: rx })
    }

    fn start_playback(
        &mut self,
        source: AudioSource,
        _volume: f32,
    ) -> Result<AudioPlaybackHandle, AudioError> {
        let id = self.alloc_id();
        let out_wav = self.out_wav.clone();

        let src_path = match source {
            AudioSource::File(p) => p,
            AudioSource::Pipe(id) => {
                log::warn!(
                    "MockAudioDevice: Pipe source mode not yet implemented (Layer 4+) \
                            — pipe='{id}'"
                );
                return Err(AudioError::UnsupportedFormat);
            }
        };

        let (stop_tx, stop_rx) = mpsc::channel::<()>();

        std::thread::spawn(move || {
            let reader = match hound::WavReader::open(&src_path) {
                Ok(r) => r,
                Err(e) => {
                    log::warn!("MockAudioDevice: failed to open playback source {src_path:?}: {e}");
                    return;
                }
            };

            let spec = reader.spec();
            let writer_spec = hound::WavSpec {
                channels: spec.channels,
                sample_rate: spec.sample_rate,
                bits_per_sample: 32,
                sample_format: hound::SampleFormat::Float,
            };

            let mut writer = match hound::WavWriter::create(&out_wav, writer_spec) {
                Ok(w) => w,
                Err(e) => {
                    log::warn!("MockAudioDevice: failed to create output WAV {out_wav:?}: {e}");
                    return;
                }
            };

            let samples: Vec<f32> = match spec.sample_format {
                hound::SampleFormat::Float => reader
                    .into_samples::<f32>()
                    .filter_map(|s| s.ok())
                    .collect(),
                hound::SampleFormat::Int => {
                    let max_val = (1i64 << (spec.bits_per_sample - 1)) as f32;
                    match spec.bits_per_sample {
                        16 => reader
                            .into_samples::<i16>()
                            .filter_map(|s| s.ok())
                            .map(|s| s as f32 / max_val)
                            .collect(),
                        24 | 32 => reader
                            .into_samples::<i32>()
                            .filter_map(|s| s.ok())
                            .map(|s| s as f32 / max_val)
                            .collect(),
                        _ => {
                            log::warn!("MockAudioDevice: unsupported bit depth during playback");
                            return;
                        }
                    }
                }
            };

            for sample in samples {
                // Check for stop signal without blocking.
                if stop_rx.try_recv().is_ok() {
                    break;
                }
                if let Err(e) = writer.write_sample(sample) {
                    log::warn!("MockAudioDevice: write_sample error: {e}");
                    break;
                }
            }

            if let Err(e) = writer.finalize() {
                log::warn!("MockAudioDevice: failed to finalize output WAV: {e}");
            }
        });

        Ok(AudioPlaybackHandle { id, stop_tx })
    }

    fn stop(&mut self, _handle_id: u64) {
        // Stopping is signalled via the handle's stop_tx channel.
        // Callers should send on the handle before dropping it.
        // No registry of active handles is kept in the mock.
    }
}

// ---------------------------------------------------------------------------
// Panic-path protection for prod stubs.
// Any method the host calls on a factory-returned impl must return an error
// instead of panicking — a panic here freezes the UI thread.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod prod_stub_tests {
    use super::*;

    #[test]
    fn core_audio_start_capture_returns_err_not_panic() {
        let mut d = CoreAudioDevice;
        assert!(d.start_capture(48000, 512).is_err());
    }

    #[test]
    fn core_audio_start_playback_returns_err_not_panic() {
        let mut d = CoreAudioDevice;
        let src = AudioSource::File(PathBuf::from("/nonexistent.wav"));
        assert!(d.start_playback(src, 1.0).is_err());
    }

    #[test]
    fn core_audio_stop_is_noop_not_panic() {
        let mut d = CoreAudioDevice;
        d.stop(0); // must not panic
    }

    #[test]
    fn mock_audio_pipe_playback_returns_err_not_panic() {
        let mut d = MockAudioDevice::new(
            PathBuf::from("/tmp/nonexistent_in.wav"),
            PathBuf::from("/tmp/nonexistent_out.wav"),
        );
        let src = AudioSource::Pipe("rec".into());
        assert!(d.start_playback(src, 1.0).is_err());
    }
}
