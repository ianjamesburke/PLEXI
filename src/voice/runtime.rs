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
    let (_capture_thread, receiver) = capture::spawn(config.clone(), cancel.clone())?;
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
        if input.failed.load(Ordering::Acquire) {
            return Err("Microphone disconnected or input failed; voice mode stopped".into());
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
            if !text.trim().is_empty() && text.len() <= MAX_TEXT {
                let mut shared = mailbox.lock().map_err(|_| "Voice mailbox unavailable")?;
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
