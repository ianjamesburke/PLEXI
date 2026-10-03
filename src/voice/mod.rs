//! Continuous voice control. Audio and interpretation have independent workers.
//! Only finalized utterances enter the bounded host-owned FIFO.
use serde::Serialize;
use std::{collections::VecDeque, path::PathBuf, time::Instant};

pub(crate) mod capture;
pub(crate) mod decisions;
pub(crate) mod runtime;

pub(crate) const QUEUE_CAPACITY: usize = 8;
pub(crate) const FRAME: usize = 256;
pub(crate) const MAX_SAMPLES: usize = 12 * 16000;
pub(crate) const MAX_TEXT: usize = 2048;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Origin {
    pub window: u64,
    pub context: u64,
    pub pane: u64,
    pub workspace: PathBuf,
}

#[derive(Clone)]
pub(crate) struct Utterance {
    pub generation: u64,
    pub id: u64,
    pub origin: Origin,
    pub text: String,
    pub started: Instant,
    pub finalized: Instant,
}

#[derive(Default, Serialize)]
pub(crate) struct Status {
    pub enabled: bool,
    pub listening: bool,
    pub microphone: Option<String>,
    pub partial: String,
    pub processing: Option<u64>,
    pub queued: usize,
    pub outcome: String,
    pub rejected: u64,
}

#[derive(Default)]
pub(crate) struct Session {
    pub generation: u64,
    pub status: Status,
    queue: VecDeque<Utterance>,
    last_id: u64,
}

impl Session {
    pub fn start(&mut self) {
        self.queue.clear();
        self.generation += 1;
        self.last_id = 0;
        self.status = Status {
            enabled: true,
            outcome: "Loading local speech model…".into(),
            ..Default::default()
        };
    }

    pub fn stop(&mut self) {
        self.generation += 1;
        self.queue.clear();
        self.status = Status {
            outcome: "Voice mode off".into(),
            ..Default::default()
        };
    }

    pub fn enqueue(&mut self, utterance: Utterance) -> Result<(), &'static str> {
        if !self.status.enabled || utterance.generation != self.generation {
            return Err("Voice session has ended");
        }
        if utterance.id <= self.last_id {
            return Err("Duplicate utterance");
        }
        self.last_id = utterance.id;
        if self.queue.len() == QUEUE_CAPACITY {
            self.status.rejected += 1;
            return Err("Voice command queue full; utterance rejected");
        }
        if utterance.text.len() > MAX_TEXT {
            return Err("Utterance too long");
        }
        self.queue.push_back(utterance);
        self.status.queued = self.queue.len();
        Ok(())
    }

    pub fn next(&mut self) -> Option<Utterance> {
        if !self.status.enabled || self.status.processing.is_some() {
            return None;
        }
        let utterance = self.queue.pop_front()?;
        self.status.queued = self.queue.len();
        self.status.processing = Some(utterance.id);
        Some(utterance)
    }

    pub fn complete(&mut self, generation: u64, id: u64, outcome: String) -> bool {
        if !self.status.enabled
            || generation != self.generation
            || self.status.processing != Some(id)
        {
            return false;
        }
        self.status.processing = None;
        self.status.outcome = outcome;
        true
    }
}

/// Bounded silence endpoint detector. A damaged/overlong utterance is discarded
/// until a full silence boundary, never split into independently actionable text.
pub(crate) struct Segmenter {
    pub samples: Vec<f32>,
    pub active: bool,
    pub rejected: bool,
    silence: usize,
    end_frames: usize,
    pre_roll: VecDeque<f32>,
}

impl Segmenter {
    pub fn new(silence_ms: u64) -> Self {
        Self {
            samples: Vec::new(),
            active: false,
            rejected: false,
            silence: 0,
            end_frames: silence_ms.div_ceil(16) as usize,
            pre_roll: VecDeque::new(),
        }
    }

    pub fn invalidate(&mut self) {
        self.samples.clear();
        self.pre_roll.clear();
        self.rejected = true;
        self.active = false;
        self.silence = 0;
    }

    pub fn push(&mut self, frame: &[f32], speech: bool) -> Option<Vec<f32>> {
        self.silence = if speech { 0 } else { self.silence + 1 };
        if self.rejected {
            if self.silence >= self.end_frames {
                self.rejected = false;
            }
            return None;
        }
        if speech && !self.active {
            self.active = true;
            self.samples.extend(self.pre_roll.drain(..));
        }
        if self.active {
            if self.samples.len() + frame.len() > MAX_SAMPLES {
                self.invalidate();
                return None;
            }
            self.samples.extend_from_slice(frame);
            if self.silence >= self.end_frames {
                self.active = false;
                return Some(std::mem::take(&mut self.samples));
            }
        } else {
            self.pre_roll.extend(frame);
            while self.pre_roll.len() > 3200 {
                self.pre_roll.pop_front();
            }
        }
        None
    }
}

#[cfg(test)]
mod tests;
