//! CPAL input: the callback only copies samples into a bounded lock-free ring.
use crate::config::VoiceConfig;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc, Arc,
};

#[derive(serde::Serialize, Clone, Debug)]
pub(crate) struct InputDevice {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

pub(crate) fn devices() -> Result<Vec<InputDevice>, String> {
    let host = cpal::default_host();
    let default = host.default_input_device().and_then(|d| d.id().ok());
    host.input_devices()
        .map_err(|e| format!("List microphone inputs: {e}"))?
        .map(|device| {
            let id = device
                .id()
                .map_err(|e| format!("Microphone identity: {e}"))?;
            Ok(InputDevice {
                is_default: Some(&id) == default.as_ref(),
                id: id.to_string(),
                name: device
                    .description()
                    .map_err(|e| format!("Microphone description: {e}"))?
                    .name()
                    .to_string(),
            })
        })
        .collect()
}

pub(crate) fn choose(devices: &[InputDevice], config: &VoiceConfig) -> Result<String, String> {
    for preference in &config.input_preferences {
        if let Some(device) = devices.iter().find(|d| &d.id == preference) {
            return Ok(device.id.clone());
        }
        let mut matches = devices.iter().filter(|d| &d.name == preference);
        if let Some(device) = matches.next() {
            if matches.next().is_some() {
                return Err(format!(
                    "Ambiguous microphone name '{preference}'; use its device ID"
                ));
            }
            return Ok(device.id.clone());
        }
    }
    if config.fallback_to_default {
        if let Some(device) = devices.iter().find(|d| d.is_default) {
            return Ok(device.id.clone());
        }
    }
    Err("No preferred microphone available; check voice.input_preferences or enable fallback_to_default".into())
}

pub(crate) struct AudioInput {
    pub samples: rtrb::Consumer<f32>,
    pub rate: u32,
    pub channels: usize,
    pub microphone: String,
    pub overflow: Arc<AtomicU64>,
    pub failed: Arc<AtomicBool>,
}

/// Creates the stream on its owning worker. Cancellation drops it independently
/// of a slow model decode; the inference worker never owns the microphone.
pub(crate) fn spawn(
    config: VoiceConfig,
    cancel: Arc<AtomicBool>,
) -> Result<
    (
        std::thread::JoinHandle<()>,
        mpsc::Receiver<Result<AudioInput, String>>,
    ),
    String,
> {
    let (tx, rx) = mpsc::sync_channel(1);
    let handle = std::thread::Builder::new()
        .name("plexi-voice-microphone".into())
        .spawn(move || {
            let prepare = || -> Result<(cpal::Stream, AudioInput), String> {
                let inputs = devices()?;
                let id = choose(&inputs, &config)?;
                let selected = inputs
                    .iter()
                    .find(|d| d.id == id)
                    .ok_or("Microphone disappeared")?;
                let device = cpal::default_host()
                    .device_by_id(&id.parse().map_err(|e| format!("Microphone ID: {e}"))?)
                    .ok_or("Microphone disconnected")?;
                let supported = device
                    .default_input_config()
                    .map_err(|e| format!("Microphone input configuration: {e}"))?;
                let stream_config = supported.config();
                let rate = stream_config.sample_rate;
                let channels = stream_config.channels as usize;
                if !(16000..=192000).contains(&rate) || !(1..=8).contains(&channels) {
                    return Err("Microphone must provide 16–192 kHz with 1–8 channels".into());
                }
                let (producer, consumer) = rtrb::RingBuffer::new(rate as usize * channels * 3);
                let overflow = Arc::new(AtomicU64::new(0));
                let failed = Arc::new(AtomicBool::new(false));
                let stream = match supported.sample_format() {
                    cpal::SampleFormat::F32 => stream::<f32>(
                        &device,
                        &stream_config,
                        producer,
                        overflow.clone(),
                        failed.clone(),
                    ),
                    cpal::SampleFormat::I16 => stream::<i16>(
                        &device,
                        &stream_config,
                        producer,
                        overflow.clone(),
                        failed.clone(),
                    ),
                    cpal::SampleFormat::U16 => stream::<u16>(
                        &device,
                        &stream_config,
                        producer,
                        overflow.clone(),
                        failed.clone(),
                    ),
                    format => {
                        return Err(format!("Unsupported microphone sample format: {format}"))
                    }
                }?;
                stream.play().map_err(|e| {
                    format!("Start microphone (check OS microphone permission): {e}")
                })?;
                Ok((
                    stream,
                    AudioInput {
                        samples: consumer,
                        rate,
                        channels,
                        microphone: selected.name.clone(),
                        overflow,
                        failed,
                    },
                ))
            };
            match prepare() {
                Ok((stream, input)) => {
                    if tx.send(Ok(input)).is_ok() {
                        while !cancel.load(Ordering::Acquire) {
                            std::thread::sleep(std::time::Duration::from_millis(16));
                        }
                    }
                    drop(stream);
                    log::info!("voice: microphone stopped");
                }
                Err(error) => {
                    log::warn!("voice: capture setup failed: {error}");
                    let _ = tx.send(Err(error));
                }
            }
        })
        .map_err(|e| format!("Start microphone worker: {e}"))?;
    Ok((handle, rx))
}

fn stream<T: cpal::SizedSample + Copy>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut producer: rtrb::Producer<f32>,
    overflow: Arc<AtomicU64>,
    failed: Arc<AtomicBool>,
) -> Result<cpal::Stream, String>
where
    f32: cpal::FromSample<T>,
{
    device
        .build_input_stream(
            config,
            move |data: &[T], _| {
                // Whole callbacks are dropped on overload. The consumer invalidates the
                // entire utterance and waits for silence before admitting more speech.
                if producer.slots() < data.len() {
                    overflow.fetch_add(1, Ordering::Release);
                    return;
                }
                for sample in data {
                    if producer
                        .push(cpal::Sample::to_sample::<f32>(*sample))
                        .is_err()
                    {
                        overflow.fetch_add(1, Ordering::Release);
                        break;
                    }
                }
            },
            move |_| {
                failed.store(true, Ordering::Release);
            },
            None,
        )
        .map_err(|e| format!("Open microphone (check OS microphone permission): {e}"))
}

/// Streaming box-filter resampling: accumulates fractional source-frame area
/// into each 16 kHz output sample. Downmix/resample occur off the audio callback.
pub(crate) struct Normalize {
    channels: usize,
    channel_index: usize,
    mono_sum: f32,
    ratio: f64,
    remaining: f64,
    sum: f64,
}

impl Normalize {
    pub fn new(rate: u32, channels: usize) -> Self {
        let ratio = rate as f64 / 16000.0;
        Self {
            channels,
            channel_index: 0,
            mono_sum: 0.0,
            ratio,
            remaining: ratio,
            sum: 0.0,
        }
    }
    pub fn push(&mut self, value: f32, output: &mut Vec<f32>) {
        self.mono_sum += if value.is_finite() {
            value.clamp(-1.0, 1.0)
        } else {
            0.0
        };
        self.channel_index += 1;
        if self.channel_index != self.channels {
            return;
        }
        let mono = (self.mono_sum / self.channels as f32) as f64;
        self.channel_index = 0;
        self.mono_sum = 0.0;
        let mut available: f64 = 1.0;
        while available > 1e-9 {
            let amount = available.min(self.remaining);
            self.sum += mono * amount;
            self.remaining -= amount;
            available -= amount;
            if self.remaining < 1e-9 {
                output.push((self.sum / self.ratio) as f32);
                self.sum = 0.0;
                self.remaining = self.ratio;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preference_order_and_explicit_fallback() {
        let inputs = vec![
            InputDevice {
                id: "a".into(),
                name: "Built-in".into(),
                is_default: true,
            },
            InputDevice {
                id: "b".into(),
                name: "USB".into(),
                is_default: false,
            },
        ];
        let mut config = VoiceConfig {
            input_preferences: vec!["missing".into(), "USB".into()],
            ..Default::default()
        };
        assert_eq!(choose(&inputs, &config).unwrap(), "b");
        config.input_preferences = vec!["missing".into()];
        config.fallback_to_default = false;
        assert!(choose(&inputs, &config).is_err());
        assert!(choose(&[], &VoiceConfig::default()).is_err());
    }
    #[test]
    fn resample_preserves_duration_and_downmixes_stereo() {
        for rate in [16000, 44100, 48000, 96000] {
            let mut normalizer = Normalize::new(rate, 2);
            let mut out = Vec::new();
            for _ in 0..rate {
                normalizer.push(0.2, &mut out);
                normalizer.push(0.6, &mut out);
            }
            assert_eq!(out.len(), 16000);
            assert!(out.iter().all(|s| (*s - 0.4).abs() < 0.0001));
        }
    }
}
