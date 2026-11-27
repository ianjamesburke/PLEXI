//! Quit state: one enum and one transition route (stint 1544).
//!
//! Cmd+Q, a window close, a system quit, `plexi host stop`, an update restart
//! and a backend teardown all move through [`QuitState`]. Every move between
//! stages goes through [`QuitCoordinator::go`], which checks it against the
//! one table, [`QuitPhase::allows`], and logs it. Each stage carries only what
//! it needs, so combinations such as "accepted while counting Cmd+Q presses"
//! or "a save retry with no quit" cannot be written down.

use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use super::quit::ExitReason;

/// How long a Cmd+Q press counts toward the next one.
const KEYBOARD_PRESS_WINDOW: Duration = Duration::from_millis(1500);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UserQuitConfirmation {
    Keyboard,
    WorkingAgents,
    Accepted,
}

/// Where a quit stands.
#[derive(Default)]
pub(crate) enum QuitState {
    #[default]
    Idle,
    /// Cmd+Q was pressed `count` times (one or two); the third press quits.
    CountingPresses { count: u8, last: Instant },
    /// A text editor vetoed the quit and asks about its unsaved changes.
    AwaitingEditor {
        reason: ExitReason,
        pane: u64,
        confirmation: UserQuitConfirmation,
    },
    /// The "agents are working" dialog is open.
    ConfirmingAgents { reason: ExitReason },
    /// The quit is decided: save, hide, clean up and exit follow.
    Accepted(AcceptedQuit),
}

/// The stage of a [`QuitState`], without its data: the transition table's
/// rows and columns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum QuitPhase {
    Idle,
    CountingPresses,
    AwaitingEditor,
    ConfirmingAgents,
    Accepted,
}

impl QuitPhase {
    #[cfg(test)]
    pub(crate) const ALL: [Self; 5] = [
        Self::Idle,
        Self::CountingPresses,
        Self::AwaitingEditor,
        Self::ConfirmingAgents,
        Self::Accepted,
    ];

    /// The one transition table. Changes inside the accepted stage (save
    /// progress, hide, restart) are not transitions.
    pub(crate) fn allows(self, to: Self) -> bool {
        use QuitPhase::*;
        matches!(
            (self, to),
            // Anything can be cancelled or reset.
            (_, Idle)
            // A quit starts from rest or from Cmd+Q presses.
            | (
                Idle | CountingPresses,
                CountingPresses | AwaitingEditor | ConfirmingAgents | Accepted
            )
            // `host stop` and a backend teardown overrule an open question.
            | (AwaitingEditor | ConfirmingAgents, Accepted)
            // A save retry leaves the UI usable, so an editor can turn dirty.
            | (Accepted, AwaitingEditor)
        )
    }
}

impl QuitState {
    pub(crate) fn phase(&self) -> QuitPhase {
        match self {
            Self::Idle => QuitPhase::Idle,
            Self::CountingPresses { .. } => QuitPhase::CountingPresses,
            Self::AwaitingEditor { .. } => QuitPhase::AwaitingEditor,
            Self::ConfirmingAgents { .. } => QuitPhase::ConfirmingAgents,
            Self::Accepted(_) => QuitPhase::Accepted,
        }
    }
}

/// A decided quit.
pub(crate) struct AcceptedQuit {
    pub(crate) reason: ExitReason,
    pub(crate) save: SaveStage,
    /// `plexi host stop`'s reply token, answered by `exit_host` with the
    /// process's real exit outcome (stint 1430).
    pub(crate) stop_reply: Option<String>,
    /// A restart stop holds pane cleanup until the CLI detaches its relaunch.
    pub(crate) restart_handoff: Option<Receiver<std::io::Result<()>>>,
    pub(crate) restart_scheduled: bool,
    pub(crate) hide: WindowHide,
}

impl AcceptedQuit {
    pub(crate) fn new(reason: ExitReason, stop_reply: Option<String>) -> Self {
        Self {
            reason,
            save: SaveStage::Pending { failures: 0 },
            stop_reply,
            restart_handoff: None,
            restart_scheduled: false,
            hide: WindowHide::NotRequested,
        }
    }
}

/// The final workspace save of an accepted quit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SaveStage {
    /// No successful save yet, after `failures` consecutive failed attempts.
    Pending {
        failures: u32,
    },
    /// A failed save waits until `at` before trying again.
    Retrying {
        failures: u32,
        at: Instant,
    },
    /// Failures reached the cap: the "Quit without saving" dialog shows
    /// `cause`. No retry runs.
    AskingUser {
        failures: u32,
        cause: String,
    },
    Saved,
    /// Skipped for the rest of this quit: the user chose "Quit without
    /// saving", or `host stop` spent its retries (stint 1540). The file on
    /// disk is not rewritten. `failures` is the count that led here.
    Waived {
        failures: u32,
    },
}

impl SaveStage {
    pub(crate) fn failures(&self) -> u32 {
        match self {
            Self::Pending { failures }
            | Self::Retrying { failures, .. }
            | Self::AskingUser { failures, .. }
            | Self::Waived { failures } => *failures,
            Self::Saved => 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WindowHide {
    NotRequested,
    /// `handed_off` is false after a failed native hide, so the watchdog
    /// reports a visible stall as `[FREEZE]`.
    Requested {
        handed_off: bool,
    },
}

/// A quit that arrived off the logic pass and is taken by the next one.
enum QueuedQuit {
    HostStop {
        reply: Option<String>,
        restart: bool,
    },
    UpdateRestart,
}

#[derive(Default)]
pub(crate) struct QuitCoordinator {
    state: QuitState,
    queued: Option<QueuedQuit>,
}

impl QuitCoordinator {
    pub(crate) fn phase(&self) -> QuitPhase {
        self.state.phase()
    }

    pub(crate) fn state(&self) -> &QuitState {
        &self.state
    }

    /// The one route between stages. An illegal move is logged and refused,
    /// leaving the state as it was.
    pub(crate) fn go(&mut self, next: QuitState, why: &str) -> bool {
        let (from, to) = (self.phase(), next.phase());
        if !from.allows(to) {
            log::error!("quit_state: refused {from:?} -> {to:?} ({why})");
            return false;
        }
        if from != to {
            log::info!("quit_state: {from:?} -> {to:?} ({why})");
        }
        self.state = next;
        true
    }

    pub(crate) fn accepted(&self) -> Option<&AcceptedQuit> {
        match &self.state {
            QuitState::Accepted(quit) => Some(quit),
            _ => None,
        }
    }

    pub(crate) fn accepted_mut(&mut self) -> Option<&mut AcceptedQuit> {
        match &mut self.state {
            QuitState::Accepted(quit) => Some(quit),
            _ => None,
        }
    }

    pub(crate) fn accepted_reason(&self) -> Option<ExitReason> {
        self.accepted().map(|quit| quit.reason)
    }

    pub(crate) fn is_active(&self) -> bool {
        self.accepted().is_some()
    }

    /// An accepted quit freezes input, except while a failed save waits to
    /// retry or asks the user.
    pub(crate) fn blocks_input(&self) -> bool {
        self.accepted().is_some_and(|quit| {
            !matches!(
                quit.save,
                SaveStage::Retrying { .. } | SaveStage::AskingUser { .. }
            )
        })
    }

    pub(crate) fn pending_editor(&self) -> Option<u64> {
        match self.state {
            QuitState::AwaitingEditor { pane, .. } => Some(pane),
            _ => None,
        }
    }

    pub(crate) fn confirming_agents(&self) -> bool {
        matches!(self.state, QuitState::ConfirmingAgents { .. })
    }

    /// The cause shown by the "Quit without saving" dialog.
    pub(crate) fn save_failure(&self) -> Option<&str> {
        match &self.accepted()?.save {
            SaveStage::AskingUser { cause, .. } => Some(cause),
            _ => None,
        }
    }

    /// Whether the quit dialog (working agents, or a failed save) is open.
    pub(crate) fn asks_user(&self) -> bool {
        self.confirming_agents() || self.save_failure().is_some()
    }

    pub(crate) fn keyboard_presses(&self) -> u8 {
        match self.state {
            QuitState::CountingPresses { count, .. } => count,
            _ => 0,
        }
    }

    pub(crate) fn keyboard_presses_timed_out(&self, now: Instant) -> bool {
        match self.state {
            QuitState::CountingPresses { last, .. } => {
                now.duration_since(last) > KEYBOARD_PRESS_WINDOW
            }
            _ => false,
        }
    }

    /// Presses counted so far, with the press at `now` added.
    pub(crate) fn next_keyboard_press(&self, now: Instant) -> u8 {
        if self.keyboard_presses_timed_out(now) {
            1
        } else {
            self.keyboard_presses() + 1
        }
    }

    pub(crate) fn window_handed_off(&self) -> bool {
        self.accepted()
            .is_some_and(|quit| quit.hide == WindowHide::Requested { handed_off: true })
    }

    /// A `host stop` whose CLI is waiting for an answer, queued or accepted.
    pub(crate) fn stop_reply_pending(&self) -> bool {
        matches!(
            self.queued,
            Some(QueuedQuit::HostStop { reply: Some(_), .. })
        ) || self
            .accepted()
            .is_some_and(|quit| quit.stop_reply.is_some())
    }

    pub(crate) fn host_stop_queued(&self) -> bool {
        matches!(self.queued, Some(QueuedQuit::HostStop { .. }))
    }

    /// Queue a `host stop`. It outranks a queued update restart. A second
    /// stop keeps the first one's restart choice and only fills a missing
    /// reply token.
    pub(crate) fn queue_host_stop(&mut self, reply: Option<String>, restart: bool) {
        match &mut self.queued {
            Some(QueuedQuit::HostStop { reply: queued, .. }) => {
                if queued.is_none() {
                    *queued = reply;
                }
            }
            _ => self.queued = Some(QueuedQuit::HostStop { reply, restart }),
        }
    }

    /// Queue an update restart unless a `host stop` is already queued.
    pub(crate) fn queue_update_restart(&mut self) {
        if self.queued.is_none() {
            self.queued = Some(QueuedQuit::UpdateRestart);
        }
    }

    pub(crate) fn update_restart_queued(&self) -> bool {
        matches!(self.queued, Some(QueuedQuit::UpdateRestart))
    }

    /// The queued `host stop` as `(reply, restart)`.
    pub(crate) fn take_host_stop(&mut self) -> Option<(Option<String>, bool)> {
        match self.queued.take() {
            Some(QueuedQuit::HostStop { reply, restart }) => Some((reply, restart)),
            other => {
                self.queued = other;
                None
            }
        }
    }

    pub(crate) fn take_update_restart(&mut self) -> bool {
        if self.update_restart_queued() {
            self.queued = None;
            return true;
        }
        false
    }
}

/// Test seams.
#[cfg(test)]
impl QuitCoordinator {
    /// The reason of a quit that is asking, waiting or accepted.
    pub(crate) fn reason(&self) -> Option<ExitReason> {
        match &self.state {
            QuitState::AwaitingEditor { reason, .. } | QuitState::ConfirmingAgents { reason } => {
                Some(*reason)
            }
            QuitState::Accepted(quit) => Some(quit.reason),
            QuitState::Idle | QuitState::CountingPresses { .. } => None,
        }
    }

    pub(crate) fn retry_pending(&self) -> bool {
        self.accepted()
            .is_some_and(|quit| matches!(quit.save, SaveStage::Retrying { .. }))
    }

    pub(crate) fn make_retry_due(&mut self) {
        if let Some(quit) = self.accepted_mut() {
            if let SaveStage::Retrying { at, .. } = &mut quit.save {
                *at = Instant::now();
            }
        }
    }

    /// Drop a scheduled retry, so the next pass saves without the retry's
    /// veto check.
    pub(crate) fn clear_retry(&mut self) {
        if let Some(quit) = self.accepted_mut() {
            if let SaveStage::Retrying { failures, .. } = quit.save {
                quit.save = SaveStage::Pending { failures };
            }
        }
    }

    pub(crate) fn hide_requested(&self) -> bool {
        self.accepted()
            .is_some_and(|quit| quit.hide != WindowHide::NotRequested)
    }

    pub(crate) fn saved(&self) -> bool {
        self.accepted()
            .is_some_and(|quit| quit.save == SaveStage::Saved)
    }

    pub(crate) fn save_waived(&self) -> bool {
        self.accepted()
            .is_some_and(|quit| matches!(quit.save, SaveStage::Waived { .. }))
    }

    pub(crate) fn save_failures(&self) -> u32 {
        self.accepted().map_or(0, |quit| quit.save.failures())
    }

    pub(crate) fn restart_scheduled(&self) -> bool {
        self.accepted().is_some_and(|quit| quit.restart_scheduled)
    }

    pub(crate) fn restart_handoff_pending(&self) -> bool {
        self.accepted()
            .is_some_and(|quit| quit.restart_handoff.is_some())
    }

    pub(crate) fn stop_reply(&self) -> Option<&str> {
        match &self.queued {
            Some(QueuedQuit::HostStop {
                reply: Some(reply), ..
            }) => Some(reply),
            _ => self.accepted()?.stop_reply.as_deref(),
        }
    }

    /// Show `count` Cmd+Q presses that never time out (0 clears them).
    pub(crate) fn hold_keyboard_presses(&mut self, count: u8) {
        let next = if count == 0 {
            QuitState::Idle
        } else {
            QuitState::CountingPresses {
                count,
                last: Instant::now() + Duration::from_secs(3600),
            }
        };
        assert!(self.go(next, "test seam"));
    }

    /// Make the last Cmd+Q press `age` old.
    pub(crate) fn age_keyboard_presses(&mut self, age: Duration) {
        if let QuitState::CountingPresses { last, .. } = &mut self.state {
            *last = Instant::now() - age;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use QuitPhase::*;

    fn state(phase: QuitPhase) -> QuitState {
        match phase {
            Idle => QuitState::Idle,
            CountingPresses => QuitState::CountingPresses {
                count: 1,
                last: Instant::now(),
            },
            AwaitingEditor => QuitState::AwaitingEditor {
                reason: ExitReason::WindowClosed,
                pane: 7,
                confirmation: UserQuitConfirmation::Keyboard,
            },
            ConfirmingAgents => QuitState::ConfirmingAgents {
                reason: ExitReason::WindowClosed,
            },
            Accepted => QuitState::Accepted(AcceptedQuit::new(ExitReason::WindowClosed, None)),
        }
    }

    /// Every state pair is either allowed or rejected, and `go` obeys the
    /// table: an allowed move lands, a rejected one leaves the state alone.
    #[test]
    fn transition_table_covers_every_state_pair() {
        let allowed = [
            (Idle, Idle),
            (Idle, CountingPresses),
            (Idle, AwaitingEditor),
            (Idle, ConfirmingAgents),
            (Idle, Accepted),
            (CountingPresses, Idle),
            (CountingPresses, CountingPresses),
            (CountingPresses, AwaitingEditor),
            (CountingPresses, ConfirmingAgents),
            (CountingPresses, Accepted),
            (AwaitingEditor, Idle),
            (AwaitingEditor, Accepted),
            (ConfirmingAgents, Idle),
            (ConfirmingAgents, Accepted),
            (Accepted, Idle),
            (Accepted, AwaitingEditor),
        ];
        let mut pairs = 0;
        for from in QuitPhase::ALL {
            for to in QuitPhase::ALL {
                pairs += 1;
                let expected = allowed.contains(&(from, to));
                assert_eq!(from.allows(to), expected, "{from:?} -> {to:?}");
                let mut coordinator = QuitCoordinator {
                    state: state(from),
                    queued: None,
                };
                assert_eq!(coordinator.go(state(to), "table test"), expected);
                let landed = if expected { to } else { from };
                assert_eq!(coordinator.phase(), landed, "{from:?} -> {to:?}");
            }
        }
        assert_eq!(pairs, 25);
    }

    #[test]
    fn only_a_retrying_or_asking_save_lets_input_through() {
        let stages = [
            (SaveStage::Pending { failures: 0 }, true),
            (
                SaveStage::Retrying {
                    failures: 1,
                    at: Instant::now(),
                },
                false,
            ),
            (
                SaveStage::AskingUser {
                    failures: 5,
                    cause: "x".into(),
                },
                false,
            ),
            (SaveStage::Saved, true),
            (SaveStage::Waived { failures: 5 }, true),
        ];
        for (save, blocks) in stages {
            let mut quit = AcceptedQuit::new(ExitReason::WindowClosed, None);
            quit.save = save.clone();
            let coordinator = QuitCoordinator {
                state: QuitState::Accepted(quit),
                queued: None,
            };
            assert_eq!(coordinator.blocks_input(), blocks, "{save:?}");
        }
        assert!(!QuitCoordinator::default().blocks_input());
    }

    #[test]
    fn a_queued_host_stop_outranks_an_update_restart() {
        let mut coordinator = QuitCoordinator::default();
        coordinator.queue_update_restart();
        coordinator.queue_host_stop(None, false);
        coordinator.queue_update_restart();
        coordinator.queue_host_stop(Some("reply".into()), true);
        assert!(!coordinator.take_update_restart());
        assert_eq!(
            coordinator.take_host_stop(),
            Some((Some("reply".into()), false))
        );
        assert!(coordinator.take_host_stop().is_none());
    }
}
