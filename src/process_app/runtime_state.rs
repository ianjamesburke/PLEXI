//! PGAP app runtime state machine.
//!
//! This module owns the render transaction contract for a process app. The
//! host code asks for state transitions; it does not carry separate booleans
//! for "dirty", "in flight", and "deadline".

use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ExitReason {
    StdoutClosed,
    ProcessExited,
    ExitedDuringRender { frame_id: u64 },
    FatalError,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RuntimeState {
    Spawned {
        pending_deadline: Option<Instant>,
    },
    Ready,
    Waiting {
        next_deadline: Instant,
    },
    Idle,
    Rendering {
        frame_id: u64,
        started_at: Instant,
        followup_deadline: Option<Instant>,
    },
    Closing,
    Exited {
        reason: ExitReason,
    },
    ProtocolError {
        reason: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FrameDoneOutcome {
    Matched,
    Unexpected { expected: Option<u64>, got: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RenderPoll {
    Send { frame_id: u64 },
    Waiting { remaining: Duration },
    InFlight,
    NotReady,
    Idle,
    Terminal,
}

#[derive(Debug)]
pub(crate) struct PgapRuntime {
    state: RuntimeState,
    frame_counter: u64,
    /// Frame id of a Render transaction abandoned by the in-flight timeout
    /// (issue #2208). A late FrameDone for this id is tolerated — the app
    /// recovered, slowly — instead of being treated as a protocol error.
    abandoned_frame_id: Option<u64>,
}

impl PgapRuntime {
    pub(crate) fn spawned_with_initial_render() -> Self {
        Self {
            state: RuntimeState::Spawned {
                pending_deadline: Some(Instant::now()),
            },
            frame_counter: 0,
            abandoned_frame_id: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn ready_for_test_with_initial_render() -> Self {
        Self {
            state: RuntimeState::Waiting {
                next_deadline: Instant::now(),
            },
            frame_counter: 0,
            abandoned_frame_id: None,
        }
    }

    pub(crate) fn mark_ready(&mut self) {
        let deadline = match &self.state {
            RuntimeState::Spawned { pending_deadline } => *pending_deadline,
            RuntimeState::Ready
            | RuntimeState::Waiting { .. }
            | RuntimeState::Idle
            | RuntimeState::Rendering { .. } => return,
            RuntimeState::Closing
            | RuntimeState::Exited { .. }
            | RuntimeState::ProtocolError { .. } => {
                return;
            }
        };
        self.state = match deadline {
            Some(next_deadline) => RuntimeState::Waiting { next_deadline },
            None => RuntimeState::Ready,
        };
    }

    pub(crate) fn request_render_now(&mut self) -> bool {
        self.request_render_at(Instant::now())
    }

    pub(crate) fn request_render_after(&mut self, delay: Duration) -> bool {
        self.request_render_at(Instant::now() + delay)
    }

    /// Request a render at an absolute deadline. Used by the continuous
    /// `AnimationClock`, whose deadlines are phase-anchored — converting to a
    /// relative delay and back would reintroduce send-time rebasing.
    pub(crate) fn request_render_at(&mut self, deadline: Instant) -> bool {
        match &mut self.state {
            RuntimeState::Spawned { pending_deadline } => {
                let was_idle = pending_deadline.is_none();
                *pending_deadline =
                    Some(pending_deadline.map_or(deadline, |old| old.min(deadline)));
                was_idle
            }
            RuntimeState::Ready | RuntimeState::Idle => {
                self.state = RuntimeState::Waiting {
                    next_deadline: deadline,
                };
                true
            }
            RuntimeState::Waiting { next_deadline } => {
                *next_deadline = (*next_deadline).min(deadline);
                false
            }
            RuntimeState::Rendering {
                followup_deadline, ..
            } => {
                let was_idle = followup_deadline.is_none();
                *followup_deadline =
                    Some(followup_deadline.map_or(deadline, |old| old.min(deadline)));
                was_idle
            }
            RuntimeState::Closing
            | RuntimeState::Exited { .. }
            | RuntimeState::ProtocolError { .. } => false,
        }
    }

    pub(crate) fn poll_render(&mut self, now: Instant) -> RenderPoll {
        match &self.state {
            RuntimeState::Waiting { next_deadline } if *next_deadline <= now => {
                self.frame_counter = self.frame_counter.saturating_add(1);
                let frame_id = self.frame_counter;
                self.state = RuntimeState::Rendering {
                    frame_id,
                    started_at: now,
                    followup_deadline: None,
                };
                RenderPoll::Send { frame_id }
            }
            RuntimeState::Waiting { next_deadline } => RenderPoll::Waiting {
                remaining: next_deadline.saturating_duration_since(now),
            },
            RuntimeState::Rendering { .. } => RenderPoll::InFlight,
            RuntimeState::Spawned { .. } | RuntimeState::Ready => RenderPoll::NotReady,
            RuntimeState::Idle => RenderPoll::Idle,
            RuntimeState::Closing
            | RuntimeState::Exited { .. }
            | RuntimeState::ProtocolError { .. } => RenderPoll::Terminal,
        }
    }

    /// If the in-flight render has been outstanding longer than `timeout`,
    /// abandon the transaction so the host stops repaint-polling for a
    /// FrameDone that may never come (issue #2208). Returns the abandoned
    /// frame id when the timeout fires, `None` otherwise.
    ///
    /// Any followup deadline is dropped — re-arming a render against an app
    /// that has stopped answering would just restart the poll loop. A late
    /// FrameDone for the abandoned frame is still committed (see
    /// `complete_frame`), and fresh input re-arms rendering normally.
    pub(crate) fn abandon_render_if_stalled(
        &mut self,
        now: Instant,
        timeout: Duration,
    ) -> Option<u64> {
        let RuntimeState::Rendering {
            frame_id,
            started_at,
            ..
        } = self.state
        else {
            return None;
        };
        if now.saturating_duration_since(started_at) < timeout {
            return None;
        }
        self.abandoned_frame_id = Some(frame_id);
        self.state = RuntimeState::Idle;
        Some(frame_id)
    }

    pub(crate) fn complete_frame(&mut self, frame_id: u64) -> FrameDoneOutcome {
        if self.abandoned_frame_id == Some(frame_id) {
            // The app finally answered a render the timeout already gave up
            // on. The state machine has moved on (Idle, or a newer request);
            // commit the late frame without disturbing it.
            self.abandoned_frame_id = None;
            return FrameDoneOutcome::Matched;
        }

        let RuntimeState::Rendering {
            frame_id: expected,
            followup_deadline,
            ..
        } = &self.state
        else {
            let expected = self.in_flight_frame_id();
            self.state = RuntimeState::ProtocolError {
                reason: format!("FrameDone({frame_id}) arrived while no render was in flight"),
            };
            return FrameDoneOutcome::Unexpected {
                expected,
                got: frame_id,
            };
        };

        if *expected != frame_id {
            let expected_frame_id = *expected;
            self.state = RuntimeState::ProtocolError {
                reason: format!(
                    "FrameDone({frame_id}) did not match in-flight Render({expected_frame_id})"
                ),
            };
            return FrameDoneOutcome::Unexpected {
                expected: Some(expected_frame_id),
                got: frame_id,
            };
        }

        self.state = match followup_deadline {
            Some(next_deadline) => RuntimeState::Waiting {
                next_deadline: *next_deadline,
            },
            None => RuntimeState::Idle,
        };
        FrameDoneOutcome::Matched
    }

    /// Test hook: shift the in-flight render's start time into the past so a
    /// wall-clock timeout can be exercised without sleeping.
    #[cfg(test)]
    pub(crate) fn backdate_in_flight_render(&mut self, by: Duration) {
        if let RuntimeState::Rendering { started_at, .. } = &mut self.state {
            *started_at = started_at
                .checked_sub(by)
                .expect("backdate underflowed the Instant epoch");
        }
    }

    pub(crate) fn in_flight_frame_id(&self) -> Option<u64> {
        match self.state {
            RuntimeState::Rendering { frame_id, .. } => Some(frame_id),
            _ => None,
        }
    }

    pub(crate) fn has_pending_render(&self) -> bool {
        matches!(
            self.state,
            RuntimeState::Waiting { .. }
                | RuntimeState::Rendering {
                    followup_deadline: Some(_),
                    ..
                }
                | RuntimeState::Spawned {
                    pending_deadline: Some(_)
                }
        )
    }

    pub(crate) fn is_rendering(&self) -> bool {
        matches!(self.state, RuntimeState::Rendering { .. })
    }

    pub(crate) fn pending_repaint_delay(&self) -> Option<Duration> {
        match self.state {
            RuntimeState::Waiting { next_deadline } => {
                Some(next_deadline.saturating_duration_since(Instant::now()))
            }
            RuntimeState::Rendering {
                followup_deadline: Some(next_deadline),
                ..
            } => {
                let delay = next_deadline.saturating_duration_since(Instant::now());
                if delay.is_zero() {
                    None
                } else {
                    Some(delay)
                }
            }
            _ => None,
        }
    }

    pub(crate) fn mark_closing(&mut self) {
        self.state = RuntimeState::Closing;
    }

    pub(crate) fn mark_stdout_closed(&mut self) -> ExitReason {
        let reason = match self.state {
            RuntimeState::Rendering { frame_id, .. } => ExitReason::ExitedDuringRender { frame_id },
            _ => ExitReason::StdoutClosed,
        };
        self.state = RuntimeState::Exited {
            reason: reason.clone(),
        };
        reason
    }

    pub(crate) fn mark_process_exited(&mut self) {
        if matches!(self.state, RuntimeState::Exited { .. }) {
            return;
        }
        self.state = RuntimeState::Exited {
            reason: ExitReason::ProcessExited,
        };
    }

    pub(crate) fn mark_fatal_error(&mut self) {
        self.state = RuntimeState::Exited {
            reason: ExitReason::FatalError,
        };
    }

    pub(crate) fn mark_protocol_error(&mut self, reason: impl Into<String>) {
        self.state = RuntimeState::ProtocolError {
            reason: reason.into(),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_waits_for_ready_handshake() {
        let mut runtime = PgapRuntime::spawned_with_initial_render();
        assert_eq!(runtime.poll_render(Instant::now()), RenderPoll::NotReady);

        runtime.mark_ready();
        assert_eq!(
            runtime.poll_render(Instant::now()),
            RenderPoll::Send { frame_id: 1 }
        );
    }

    #[test]
    fn render_is_a_strict_transaction() {
        let mut runtime = PgapRuntime::ready_for_test_with_initial_render();
        assert_eq!(
            runtime.poll_render(Instant::now()),
            RenderPoll::Send { frame_id: 1 }
        );
        assert_eq!(runtime.poll_render(Instant::now()), RenderPoll::InFlight);
        assert_eq!(runtime.complete_frame(1), FrameDoneOutcome::Matched);
        assert_eq!(runtime.poll_render(Instant::now()), RenderPoll::Idle);
    }

    #[test]
    fn schedule_during_render_becomes_followup_frame() {
        let mut runtime = PgapRuntime::ready_for_test_with_initial_render();
        assert_eq!(
            runtime.poll_render(Instant::now()),
            RenderPoll::Send { frame_id: 1 }
        );
        assert!(runtime.request_render_after(Duration::from_millis(0)));
        assert_eq!(runtime.poll_render(Instant::now()), RenderPoll::InFlight);
        assert_eq!(runtime.complete_frame(1), FrameDoneOutcome::Matched);
        assert_eq!(
            runtime.poll_render(Instant::now()),
            RenderPoll::Send { frame_id: 2 }
        );
    }

    #[test]
    fn absolute_followup_deadline_due_at_commit_sends_in_same_pass() {
        // Regression: a continuous app's followup armed via an absolute
        // deadline must produce an immediate Send when FrameDone commits at
        // or after that deadline — no extra host repaint in between.
        let mut runtime = PgapRuntime::ready_for_test_with_initial_render();
        let t0 = Instant::now();
        assert_eq!(runtime.poll_render(t0), RenderPoll::Send { frame_id: 1 });
        let deadline = t0 + Duration::from_micros(16_667);
        runtime.request_render_at(deadline);
        // FrameDone commits one host frame later — past the deadline.
        let commit = t0 + Duration::from_micros(21_000);
        assert_eq!(runtime.complete_frame(1), FrameDoneOutcome::Matched);
        assert_eq!(
            runtime.poll_render(commit),
            RenderPoll::Send { frame_id: 2 }
        );
    }

    #[test]
    fn stalled_render_is_abandoned_after_timeout() {
        let mut runtime = PgapRuntime::ready_for_test_with_initial_render();
        let t0 = Instant::now();
        assert_eq!(runtime.poll_render(t0), RenderPoll::Send { frame_id: 1 });

        let timeout = Duration::from_secs(5);
        // Before the timeout: still a live transaction.
        assert_eq!(
            runtime.abandon_render_if_stalled(t0 + Duration::from_secs(4), timeout),
            None
        );
        assert!(runtime.is_rendering());

        // Past the timeout: abandoned, in-flight cleared, polling stops.
        assert_eq!(
            runtime.abandon_render_if_stalled(t0 + Duration::from_secs(6), timeout),
            Some(1)
        );
        assert!(!runtime.is_rendering());
        assert_eq!(
            runtime.poll_render(t0 + Duration::from_secs(6)),
            RenderPoll::Idle
        );
        // Idempotent — no second abandon for the same stall.
        assert_eq!(
            runtime.abandon_render_if_stalled(t0 + Duration::from_secs(7), timeout),
            None
        );
    }

    #[test]
    fn late_frame_done_after_abandonment_is_tolerated() {
        let mut runtime = PgapRuntime::ready_for_test_with_initial_render();
        let t0 = Instant::now();
        assert_eq!(runtime.poll_render(t0), RenderPoll::Send { frame_id: 1 });
        runtime.abandon_render_if_stalled(t0 + Duration::from_secs(6), Duration::from_secs(5));

        // The app finally answers — commit, don't flag a protocol error.
        assert_eq!(runtime.complete_frame(1), FrameDoneOutcome::Matched);

        // The runtime is healthy again: a fresh render round-trips normally.
        assert!(runtime.request_render_now());
        assert_eq!(
            runtime.poll_render(Instant::now()),
            RenderPoll::Send { frame_id: 2 }
        );
        assert_eq!(runtime.complete_frame(2), FrameDoneOutcome::Matched);
    }

    #[test]
    fn eof_during_render_is_classified() {
        let mut runtime = PgapRuntime::ready_for_test_with_initial_render();
        assert_eq!(
            runtime.poll_render(Instant::now()),
            RenderPoll::Send { frame_id: 1 }
        );
        assert_eq!(
            runtime.mark_stdout_closed(),
            ExitReason::ExitedDuringRender { frame_id: 1 }
        );
    }
}
