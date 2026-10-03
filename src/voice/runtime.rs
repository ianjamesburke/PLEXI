//! Worker lifecycle and coalesced, bounded speech mailbox.
use super::*;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use transcribe_rs::{
    onnx::{parakeet::ParakeetModel, Quantization},
    SpeechModel, TranscribeOptions,
};

#[derive(Default)]
pub(crate) struct Mailbox {
    pub origin: Option<Origin>,
    pub partial: String,
    pub microphone: Option<String>,
    pub listening: bool,
    pub error: Option<String>,
    pub rejected: u64,
    pub finals: VecDeque<Utterance>,
}

impl Mailbox {
    pub fn finalize(&mut self, utterance: Utterance) {
        self.partial.clear();
        if self.finals.len() == QUEUE_CAPACITY {
            self.rejected += 1;
            self.error = Some("Voice queue full; utterance rejected".into());
        } else {
            self.finals.push_back(utterance);
        }
    }
}

pub(crate) struct Worker {
    pub cancel: Arc<AtomicBool>,
    pub mailbox: Arc<Mutex<Mailbox>>,
    pub thread: std::thread::JoinHandle<()>,
}

impl Worker {
    pub fn spawn(
        config: crate::config::VoiceConfig,
        generation: u64,
        origin: Origin,
        wake: egui::Context,
    ) -> Result<Self, String> {
        let cancel = Arc::new(AtomicBool::new(false));
        let mailbox = Arc::new(Mutex::new(Mailbox {
            origin: Some(origin),
            ..Default::default()
        }));
        let worker_cancel = cancel.clone();
        let worker_mailbox = mailbox.clone();
        let thread = std::thread::Builder::new()
            .name("plexi-voice-transcription".into())
            .spawn(move || {
                let result = listen(config, generation, &worker_cancel, &worker_mailbox, &wake);
                worker_cancel.store(true, Ordering::Release);
                if let Ok(mut shared) = worker_mailbox.lock() {
                    shared.listening = false;
                    if let Err(error) = result {
                        log::warn!("voice: transcription stopped: {error}");
                        shared.error = Some(error);
                    }
                }
                wake.request_repaint();
            })
            .map_err(|e| format!("Start transcription worker: {e}"))?;
        Ok(Self {
            cancel,
            mailbox,
            thread,
        })
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}

fn listen(
    config: crate::config::VoiceConfig,
    generation: u64,
    cancel: &Arc<AtomicBool>,
    mailbox: &Arc<Mutex<Mailbox>>,
    wake: &egui::Context,
) -> Result<(), String> {
    let capture_config = config.clone();
    let capture_cancel = cancel.clone();
    listen_with_input(config, generation, cancel, mailbox, wake, move || {
        capture::spawn(capture_config, capture_cancel)
    })
}

fn listen_with_input(
    config: crate::config::VoiceConfig,
    generation: u64,
    cancel: &Arc<AtomicBool>,
    mailbox: &Arc<Mutex<Mailbox>>,
    wake: &egui::Context,
    input: impl FnOnce() -> Result<capture::CaptureWorker, String>,
) -> Result<(), String> {
    let started = Instant::now();
    let model_path = config
        .model_path
        .as_ref()
        .ok_or("Set voice.model_path to an extracted Parakeet v3 int8 model directory")?;
    let mut model = ParakeetModel::load(model_path, &Quantization::Int8)
        .map_err(|e| format!("Load local voice model: {e}"))?;
    log::info!(
        "voice: model loaded generation={generation} load_ms={}",
        started.elapsed().as_millis()
    );
    if cancel.load(Ordering::Acquire) {
        return Ok(());
    }
    let (_capture_thread, receiver) = input()?;
    let mut input = loop {
        if cancel.load(Ordering::Acquire) {
            return Ok(());
        }
        match receiver.recv_timeout(std::time::Duration::from_millis(20)) {
            Ok(result) => break result?,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => (),
            Err(_) => return Err("Microphone worker disconnected".into()),
        }
    };
    {
        let mut shared = mailbox.lock().map_err(|_| "Voice mailbox unavailable")?;
        shared.listening = true;
        shared.microphone = Some(input.microphone.clone());
    }
    wake.request_repaint();
    log::info!(
        "voice: listening generation={generation} input={}",
        input.microphone
    );
    let mut normalizer = capture::Normalize::new(input.rate, input.channels);
    let mut vad = earshot::Detector::default();
    let mut frames = Vec::with_capacity(FRAME + 1);
    let mut segmenter = Segmenter::new(config.silence_ms);
    let mut next_partial = 16000;
    let mut id = 0;
    let mut origin = None;
    let mut onset = Instant::now();
    while !cancel.load(Ordering::Acquire) {
        if let Ok(error) = input.errors.pop() {
            return Err(format!(
                "Microphone input failed: {}",
                error.to_string().chars().take(256).collect::<String>()
            ));
        }
        if input.overflow.swap(0, Ordering::AcqRel) != 0 {
            while input.samples.pop().is_ok() {}
            frames.clear();
            normalizer = capture::Normalize::new(input.rate, input.channels);
            vad.reset();
            segmenter.invalidate();
            let mut shared = mailbox.lock().map_err(|_| "Voice mailbox unavailable")?;
            shared.rejected += 1;
            shared.error = Some(
                "Audio fell behind; whole utterance rejected. Please repeat after a pause.".into(),
            );
            shared.partial.clear();
            wake.request_repaint();
        }
        let Ok(sample) = input.samples.pop() else {
            std::thread::sleep(std::time::Duration::from_millis(4));
            continue;
        };
        normalizer.push(sample, &mut frames);
        if frames.len() < FRAME {
            continue;
        }
        let speech = vad.predict_f32(&frames[..FRAME]) >= 0.5;
        if speech && !segmenter.active && !segmenter.rejected {
            id += 1;
            origin = mailbox
                .lock()
                .map_err(|_| "Voice mailbox unavailable")?
                .origin
                .clone();
            onset = Instant::now();
            next_partial = 16000;
        }
        let was_rejected = segmenter.rejected;
        let finalized = segmenter.push(&frames[..FRAME], speech);
        frames.drain(..FRAME);
        if segmenter.rejected && !was_rejected {
            let mut shared = mailbox.lock().map_err(|_| "Voice mailbox unavailable")?;
            shared.rejected += 1;
            shared.error = Some(
                "Utterance exceeded 12 seconds; rejected. Pause, then say one short command."
                    .into(),
            );
            shared.partial.clear();
            wake.request_repaint();
        }
        if let Some(samples) = finalized {
            let finalized = Instant::now();
            let text = model
                .transcribe(&samples, &TranscribeOptions::default())
                .map_err(|e| format!("Transcribe utterance: {e}"))?
                .text;
            if cancel.load(Ordering::Acquire) {
                return Ok(());
            }
            log::info!(
                "voice: finalized generation={generation} utterance={id} final_decode_ms={}",
                finalized.elapsed().as_millis()
            );
            let mut shared = mailbox.lock().map_err(|_| "Voice mailbox unavailable")?;
            shared.partial.clear();
            if text.len() > MAX_TEXT {
                shared.rejected += 1;
                shared.error = Some("Transcription exceeded text limit; utterance rejected".into());
            } else if !text.trim().is_empty() {
                if let Some(origin) = origin.take() {
                    shared.finalize(Utterance {
                        generation,
                        id,
                        origin,
                        text,
                        started: onset,
                        finalized,
                    });
                } else {
                    shared.error = Some("No origin pane at speech onset; command rejected".into());
                    shared.rejected += 1;
                }
            }
            drop(shared);
            wake.request_repaint();
        } else if segmenter.active && segmenter.samples.len() >= next_partial {
            next_partial = segmenter.samples.len() + 16000;
            let text = model
                .transcribe(&segmenter.samples, &TranscribeOptions::default())
                .map_err(|e| format!("Transcribe partial: {e}"))?
                .text;
            if text.len() <= MAX_TEXT {
                mailbox
                    .lock()
                    .map_err(|_| "Voice mailbox unavailable")?
                    .partial = text;
                wake.request_repaint();
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires explicit local model and generated command WAV fixtures"]
    fn local_model_replays_continuous_command_audio_while_decision_is_blocked() {
        let model_path =
            PathBuf::from(std::env::var("PLEXI_VOICE_MODEL_PATH").expect("model directory"));
        let fixture_dir =
            PathBuf::from(std::env::var("PLEXI_VOICE_FIXTURE_DIR").expect("command WAV directory"));
        let mut audio = vec![0.0; 16000];
        for name in ["open-terminal.wav", "open-notes.wav"] {
            audio.extend(transcribe_rs::audio::read_wav_samples(&fixture_dir.join(name)).unwrap());
            audio.extend(std::iter::repeat_n(0.0, 16000));
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let mailbox = Arc::new(Mutex::new(Mailbox {
            origin: Some(Origin {
                window: 1,
                context: 1,
                pane: 1,
                workspace: fixture_dir,
            }),
            ..Default::default()
        }));
        let worker_mailbox = mailbox.clone();
        let worker_cancel = cancel.clone();
        let (release, decision) = std::sync::mpsc::sync_channel::<()>(1);
        let decision_thread = std::thread::spawn(move || decision.recv().unwrap());
        let started = Instant::now();
        let speech_thread = std::thread::spawn(move || {
            let capture_cancel = worker_cancel.clone();
            listen_with_input(
                crate::config::VoiceConfig {
                    model_path: Some(model_path),
                    ..Default::default()
                },
                1,
                &worker_cancel,
                &worker_mailbox,
                &egui::Context::default(),
                move || {
                    let (mut producer, consumer) = rtrb::RingBuffer::new(3 * 16000);
                    let (_errors_tx, errors) = rtrb::RingBuffer::new(2);
                    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
                    sender
                        .send(Ok(capture::AudioInput {
                            samples: consumer,
                            rate: 16000,
                            channels: 1,
                            microphone: "WAV replay".into(),
                            overflow: Arc::new(std::sync::atomic::AtomicU64::new(0)),
                            errors,
                        }))
                        .unwrap();
                    let thread = std::thread::spawn(move || {
                        for sample in audio {
                            while producer.is_full() && !capture_cancel.load(Ordering::Acquire) {
                                std::thread::sleep(std::time::Duration::from_millis(1));
                            }
                            if capture_cancel.load(Ordering::Acquire) {
                                return;
                            }
                            producer.push(sample).unwrap();
                        }
                    });
                    Ok((thread, receiver))
                },
            )
        });
        let deadline =
            Instant::now() + crate::testing::load_aware_timeout(std::time::Duration::from_secs(30));
        let mut partial_seen = false;
        loop {
            let shared = mailbox.lock().unwrap();
            partial_seen |= !shared.partial.is_empty();
            if shared.finals.len() == 2 {
                break;
            }
            drop(shared);
            if speech_thread.is_finished() || Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        cancel.store(true, Ordering::Release);
        speech_thread.join().unwrap().unwrap();
        let shared = mailbox.lock().unwrap();
        let texts: Vec<_> = shared
            .finals
            .iter()
            .map(|u| u.text.to_lowercase())
            .collect();
        eprintln!(
            "local replay elapsed_ms={} partial_seen={} finals={texts:?}",
            started.elapsed().as_millis(),
            partial_seen
        );
        assert_eq!(texts.len(), 2);
        assert!(texts[0].contains("terminal"));
        assert!(texts[1].contains("notes"));
        assert!(
            partial_seen,
            "A provisional transcript must be emitted before finalization"
        );
        assert!(
            !decision_thread.is_finished(),
            "Speech must finish while the decision is blocked"
        );
        release.send(()).unwrap();
        decision_thread.join().unwrap();
    }
}
