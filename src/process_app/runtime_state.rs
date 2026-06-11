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
}

impl PgapRuntime {
    pub(crate) fn spawned_with_initial_render() -> Self {
        Self {
            state: RuntimeState::Spawned {
                pending_deadline: Some(Instant::now()),
            },
            frame_counter: 0,
        }
    }

    #[cfg(test)]
    pub(crate) fn ready_for_test_with_initial_render() -> Self {
        Self {
            state: RuntimeState::Waiting {
                next_deadline: Instant::now(),
            },
            frame_counter: 0,
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

    fn request_render_at(&mut self, deadline: Instant) -> bool {
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

    pub(crate) fn complete_frame(&mut self, frame_id: u64) -> FrameDoneOutcome {
        let RuntimeState::Rendering {
            frame_id: expected,
            followup_deadline,
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
