//! Host repaint scheduling for PGAP panes.

use crate::platform::frame_diag::{self, RepaintCause};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AppSchedulerMode {
    Idle,
    Scheduled,
    Continuous { interval: Duration },
}

impl Default for AppSchedulerMode {
    fn default() -> Self {
        Self::Scheduled
    }
}

impl AppSchedulerMode {
    pub(crate) fn from_wire(mode: &str, fps: Option<u32>) -> Result<Self, String> {
        match mode {
            "idle" => Ok(Self::Idle),
            "scheduled" => Ok(Self::Scheduled),
            "continuous" => {
                let fps = fps.unwrap_or(60).clamp(1, 240);
                Ok(Self::Continuous {
                    interval: Duration::from_secs_f64(1.0 / fps as f64),
                })
            }
            other => Err(format!("unknown scheduler mode {other:?}")),
        }
    }

    pub(crate) fn next_frame_delay(self) -> Option<Duration> {
        match self {
            Self::Continuous { interval } => Some(interval),
            Self::Idle | Self::Scheduled => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RepaintDecision {
    Now,
    After(Duration),
    None,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct RepaintInputs {
    pub(crate) click_now: bool,
    pub(crate) click_awaiting_frame: bool,
    pub(crate) pointer_tracking: bool,
    pub(crate) render_delay: Option<Duration>,
    pub(crate) render_in_flight: bool,
    pub(crate) async_wake: bool,
}

pub(crate) fn decide_repaint(input: RepaintInputs) -> RepaintDecision {
    if input.click_now || input.click_awaiting_frame {
        frame_diag::note(RepaintCause::AppClick);
        return RepaintDecision::Now;
    }

    if input.pointer_tracking {
        frame_diag::note(RepaintCause::PointerTracking);
        return RepaintDecision::After(Duration::from_millis(16));
    }

    if let Some(delay) = input.render_delay {
        if delay.is_zero() {
            return RepaintDecision::Now;
        }
        return RepaintDecision::After(delay);
    }

    if input.render_in_flight || input.async_wake {
        frame_diag::note(RepaintCause::AppIdlePoll);
        return RepaintDecision::After(Duration::from_millis(100));
    }

    RepaintDecision::None
}
