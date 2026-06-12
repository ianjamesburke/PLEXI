//! Host repaint scheduling for PGAP panes.

use crate::platform::frame_diag::{self, RepaintCause};
use std::time::{Duration, Instant};

/// Poll cadence while a Render is in flight, waiting for FrameDone.
///
/// Must stay comfortably above one host frame time: egui's
/// `request_repaint_after` subtracts the predicted frame time (~16.7ms at
/// 60Hz) from the requested delay, so a 16ms poll saturated to zero and
/// repainted the host at full frame rate (issue #2208). 50ms survives the
/// subtraction with margin (even at 24Hz, predicted ~41.7ms) and a 20Hz
/// FrameDone pickup adds at most one host frame of latency to a commit.
const RENDER_IN_FLIGHT_POLL: Duration = Duration::from_millis(50);
const ASYNC_WAKE_POLL: Duration = Duration::from_millis(100);

/// How long a Render may stay in flight with no FrameDone before the host
/// abandons the transaction, marks the app hung, and stops repaint polling.
///
/// 5s matches `lifecycle::HUNG_FRAME_GAP` — there is exactly one notion of
/// "this app has stopped answering" so the repaint scheduler and the status
/// pill flip at the same moment.
pub(crate) const RENDER_IN_FLIGHT_TIMEOUT: Duration = super::lifecycle::HUNG_FRAME_GAP;

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

/// Absolute frame clock for `Continuous` scheduler mode.
///
/// Deadlines advance by exactly `interval` from the *previous deadline*, never
/// from the send timestamp. Rebasing to `send + interval` phase-locks a 60Hz
/// host to ~30fps: FrameDone commits one vsync (~16.6ms) after the send, the
/// rebased deadline (send + 16.7ms) misses being due by a hair, and the next
/// Render waits for another full host frame.
///
/// If the cadence falls more than one interval behind (slow app, hidden pane),
/// the clock rebases to `now` instead of bursting to catch up — the strict
/// Render/FrameDone transaction means there is never more than one frame in
/// flight, so debt must be dropped, not repaid.
#[derive(Debug, Clone, Copy)]
pub(crate) struct AnimationClock {
    interval: Duration,
    next_deadline: Instant,
}

impl AnimationClock {
    pub(crate) fn new(interval: Duration, now: Instant) -> Self {
        Self {
            interval,
            next_deadline: now,
        }
    }

    /// Record that a Render was sent at `now`; returns the absolute deadline
    /// for the frame after it. A send before the scheduled tick (e.g. a
    /// click-driven render) does not advance the phase.
    pub(crate) fn deadline_after_send(&mut self, now: Instant) -> Instant {
        if now >= self.next_deadline {
            self.next_deadline += self.interval;
            if self.next_deadline < now {
                self.next_deadline = now;
            }
        }
        self.next_deadline
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

    if input.render_in_flight {
        frame_diag::note(RepaintCause::AppIdlePoll);
        return RepaintDecision::After(RENDER_IN_FLIGHT_POLL);
    }

    if input.async_wake {
        frame_diag::note(RepaintCause::AppIdlePoll);
        return RepaintDecision::After(ASYNC_WAKE_POLL);
    }

    RepaintDecision::None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idle_inputs() -> RepaintInputs {
        RepaintInputs {
            click_now: false,
            click_awaiting_frame: false,
            pointer_tracking: false,
            render_delay: None,
            render_in_flight: false,
            async_wake: false,
        }
    }

    #[test]
    fn render_delay_takes_precedence_over_in_flight_poll() {
        let mut input = idle_inputs();
        input.render_delay = Some(Duration::from_millis(8));
        input.render_in_flight = true;
        assert_eq!(
            decide_repaint(input),
            RepaintDecision::After(Duration::from_millis(8))
        );
    }

    #[test]
    fn in_flight_render_polls_at_bounded_cadence() {
        let mut input = idle_inputs();
        input.render_in_flight = true;
        assert_eq!(
            decide_repaint(input),
            RepaintDecision::After(RENDER_IN_FLIGHT_POLL)
        );
    }

    /// Issue #2208: egui's `request_repaint_after` subtracts the predicted
    /// frame time (~16.7ms at 60Hz) from the requested delay. A poll at or
    /// below one frame time saturates to zero = immediate repaint = the host
    /// pinned at full frame rate while any render is in flight.
    #[test]
    fn in_flight_poll_survives_predicted_frame_time_subtraction() {
        let predicted_60hz_frame = Duration::from_secs_f32(1.0 / 60.0);
        assert!(
            !RENDER_IN_FLIGHT_POLL
                .saturating_sub(predicted_60hz_frame)
                .is_zero(),
            "RENDER_IN_FLIGHT_POLL ({RENDER_IN_FLIGHT_POLL:?}) must exceed one \
             60Hz frame time or egui turns it into an immediate repaint"
        );
    }

    const FRAME: Duration = Duration::from_micros(16_667);

    #[test]
    fn animation_clock_keeps_absolute_phase() {
        let t0 = Instant::now();
        let mut clock = AnimationClock::new(FRAME, t0);
        // The send fires a few ms after the deadline (host vsync
        // quantization) — the next deadline must stay anchored to the
        // previous deadline, not the send timestamp.
        let d1 = clock.deadline_after_send(t0 + Duration::from_millis(5));
        assert_eq!(d1, t0 + FRAME);
    }

    #[test]
    fn deadline_is_due_when_frame_done_commits_one_vsync_after_a_late_send() {
        // Regression: rebasing the followup to send + interval phase-locked a
        // 60Hz host to ~30-37fps — FrameDone (send + ~16.6ms) missed the
        // deadline (send + 16.7ms) by ~0.1ms every frame.
        let t0 = Instant::now();
        let mut clock = AnimationClock::new(FRAME, t0);
        let send = t0 + Duration::from_millis(5);
        let d1 = clock.deadline_after_send(send);
        let frame_done = send + Duration::from_micros(16_600);
        assert!(
            d1 <= frame_done,
            "followup deadline must already be due when FrameDone commits"
        );
    }

    #[test]
    fn early_send_does_not_advance_phase() {
        let t0 = Instant::now();
        let mut clock = AnimationClock::new(FRAME, t0);
        let d1 = clock.deadline_after_send(t0);
        // A click-driven render fires mid-tick; the scheduled tick stands.
        let d2 = clock.deadline_after_send(t0 + Duration::from_millis(4));
        assert_eq!(d1, d2);
    }

    #[test]
    fn falling_behind_rebases_instead_of_bursting() {
        let t0 = Instant::now();
        let mut clock = AnimationClock::new(FRAME, t0);
        // Pane hidden for 500ms — next deadline rebases to now, no burst.
        let late = t0 + Duration::from_millis(500);
        assert_eq!(clock.deadline_after_send(late), late);
    }

    #[test]
    fn generic_async_wake_uses_idle_poll_cadence() {
        let mut input = idle_inputs();
        input.async_wake = true;
        assert_eq!(
            decide_repaint(input),
            RepaintDecision::After(ASYNC_WAKE_POLL)
        );
    }
}
