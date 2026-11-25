//! The one consent guard and the evidence it issues (stints 1498, 1285).
//!
//! A decision that widens authority is applied only with a
//! [`DecisionEvidence`]. Its fields are private to this module, and the only
//! constructor outside tests is [`ConsentGuard::witness`]: the guard of the
//! surface showing that exact request, on real input, once armed. It is not
//! `Clone` or serializable, it names one request and one choice, and the sink
//! that applies the decision consumes it. A gesture elsewhere (the palette,
//! an earlier frame, an assistant turn) has none to pass.

use std::time::{Duration, Instant};

/// Proof the person chose `choice` on `request_id` at the surface showing it.
#[derive(Debug)]
pub struct DecisionEvidence {
    request_id: String,
    choice: String,
}

impl DecisionEvidence {
    /// Whether this evidence is for applying `choice` to `request_id`.
    pub fn covers(&self, request_id: &str, choice: &str) -> bool {
        self.request_id == request_id && self.choice == choice
    }

    /// This evidence, checked against the exact request and choice a sink is
    /// about to apply. The only way to get a [`CoveredDecision`].
    pub(crate) fn admit(
        &self,
        request_id: &str,
        choice: &str,
    ) -> Result<CoveredDecision<'_>, String> {
        if !self.covers(request_id, choice) {
            log::info!(
                "consent: refused {choice:?} on {request_id}: evidence is for {:?} on {}",
                self.choice,
                self.request_id
            );
            return Err("the decision was not made for this request".to_string());
        }
        Ok(CoveredDecision {
            evidence: std::marker::PhantomData,
        })
    }

    /// Evidence as a test fixture standing in for a click.
    #[cfg(test)]
    pub(crate) fn for_test(request_id: &str, choice: &str) -> Self {
        Self {
            request_id: request_id.to_string(),
            choice: choice.to_string(),
        }
    }
}

/// Evidence a sink has matched to the request and choice it is applying.
/// A grant row that says the person decided it is written only with this.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CoveredDecision<'a> {
    evidence: std::marker::PhantomData<&'a DecisionEvidence>,
}

/// One decision being asked of a [`ConsentGuard`].
pub(crate) struct ConsentDecision<'a> {
    /// The guard key of the request the surface is showing.
    pub displayed: u64,
    /// The record the decision applies to.
    pub target: &'a str,
    pub choice: &'a str,
    /// Deny and dismiss count at once; an approval waits for the arm delay.
    pub refuses: bool,
    pub now: Instant,
    pub synthetic: bool,
}

/// Why a [`ConsentGuard`] issued no evidence (stint 1735).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConsentRefusal {
    NotDisplayed,
    Synthetic,
    /// A real press before the arm delay ended: the person should retry.
    NotArmed,
}

impl std::fmt::Display for ConsentRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotDisplayed => "Ignored: this request is not the one on screen.",
            Self::Synthetic => "Ignored: only the person at the desktop can do this.",
            Self::NotArmed => "Wait a moment, then press again.",
        })
    }
}

/// What a decision surface shows when a decision did not apply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DecisionFailure {
    Refused(ConsentRefusal),
    Failed(String),
}

impl DecisionFailure {
    /// The press was early, not wrong: nothing failed.
    pub(crate) fn is_wait(&self) -> bool {
        matches!(self, Self::Refused(ConsentRefusal::NotArmed))
    }
}

impl From<ConsentRefusal> for DecisionFailure {
    fn from(refusal: ConsentRefusal) -> Self {
        Self::Refused(refusal)
    }
}

impl From<String> for DecisionFailure {
    fn from(error: String) -> Self {
        Self::Failed(error)
    }
}

impl From<&str> for DecisionFailure {
    fn from(error: &str) -> Self {
        Self::Failed(error.to_string())
    }
}

impl std::fmt::Display for DecisionFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(refusal) => refusal.fmt(f),
            Self::Failed(error) => f.write_str(error),
        }
    }
}

/// The guard key of a request known by a string id.
pub(crate) fn consent_request_key(id: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    id.hash(&mut hasher);
    hasher.finish()
}

pub(crate) const ARM_DELAY: Duration = Duration::from_secs(1);

/// The one consent input guard (stint 1498), shared by the permission banner
/// and every consent dialog: a decision input counts only on a real
/// (non-synthetic) frame, once the surface has been on screen for
/// [`ARM_DELAY`]. Stint 1283 replaces this function with the human-input
/// witness.
pub(crate) fn consent_input_admitted(
    shown_since: Option<Instant>,
    now: Instant,
    synthetic_frame: bool,
) -> bool {
    !synthetic_frame
        && shown_since.is_some_and(|since| now.saturating_duration_since(since) >= ARM_DELAY)
}

/// A fresh id for one consent request (stint 1498). The guard keys on it, so
/// a request that repeats an earlier one, field for field, still arms anew.
pub(crate) fn next_consent_request_id() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// What a consent dialog may take this frame (stint 1498).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ConsentInput {
    /// The request has been on screen, without a break, for [`ARM_DELAY`].
    pub armed: bool,
    /// This frame carries injected input.
    pub synthetic: bool,
}

impl ConsentInput {
    /// Whether an action decides this frame: nothing on a synthetic frame;
    /// a refusal (deny, dismiss) any time; an approval only once armed.
    pub(crate) fn admits(self, refuses: bool) -> bool {
        !self.synthetic && (refuses || self.armed)
    }
}

/// Arms one consent dialog's current request: the delay starts when the
/// request is first shown, and again whenever it comes back after a frame in
/// which it was not shown (hidden, unfocused, covered).
#[derive(Clone, Default, Debug)]
pub(crate) struct ConsentGuard {
    request: Option<u64>,
    since: Option<Instant>,
    last_shown_pass: Option<u64>,
}

/// Whether a surface shown on `pass` was also shown on the pass before.
fn shown_without_break(last_shown_pass: Option<u64>, pass: u64) -> bool {
    last_shown_pass.is_some_and(|last| pass <= last + 1)
}

impl ConsentGuard {
    /// The dialog is showing `request` on egui pass `pass` at `now`.
    pub(crate) fn present(&mut self, request: u64, pass: u64, now: Instant) {
        if self.request != Some(request) || !shown_without_break(self.last_shown_pass, pass) {
            log::info!("consent_guard: arming request {request}");
            self.request = Some(request);
            self.since = Some(now);
        }
        self.last_shown_pass = Some(pass);
    }

    pub(crate) fn input(&self, now: Instant, synthetic: bool) -> ConsentInput {
        ConsentInput {
            armed: consent_input_admitted(self.since, now, false),
            synthetic,
        }
    }

    /// Time left before approvals count, for scheduling the repaint that arms.
    pub(crate) fn remaining(&self, now: Instant) -> Duration {
        self.since
            .map(|since| ARM_DELAY.saturating_sub(now.saturating_duration_since(since)))
            .unwrap_or_default()
    }

    /// A keyboard-owned surface took focus on `request`: the delay starts
    /// now. Such a surface has no per-frame `present`.
    pub(crate) fn focus(&mut self, request: u64, now: Instant) {
        log::info!("consent_guard: arming focused request {request}");
        self.request = Some(request);
        self.since = Some(now);
        self.last_shown_pass = None;
    }

    /// The surface no longer shows a request.
    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }

    /// Require a second, deliberate input after another arm interval.
    pub(crate) fn rearm(&mut self, now: Instant) {
        self.since = Some(now);
    }

    /// Evidence for one decision, only when this guard is showing exactly
    /// the request being decided and its input is admitted: real input, and
    /// for an approval, armed. The one way to get a [`DecisionEvidence`].
    pub(crate) fn witness(
        &self,
        decision: ConsentDecision<'_>,
    ) -> Result<DecisionEvidence, ConsentRefusal> {
        let ConsentDecision {
            displayed,
            target,
            choice,
            refuses,
            now,
            synthetic,
        } = decision;
        if self.request != Some(displayed) {
            log::info!("consent_guard: refused id={target} reason=NotDisplayed");
            return Err(ConsentRefusal::NotDisplayed);
        }
        let input = self.input(now, synthetic);
        if !input.admits(refuses) {
            let refusal = if input.synthetic {
                ConsentRefusal::Synthetic
            } else {
                ConsentRefusal::NotArmed
            };
            log::info!("consent_guard: refused id={target} reason={refusal:?}");
            return Err(refusal);
        }
        log::info!("consent_guard: witnessed id={target}");
        Ok(DecisionEvidence {
            request_id: target.to_string(),
            choice: choice.to_string(),
        })
    }

    /// Test hook: the request appeared at `since`.
    #[cfg(test)]
    pub(crate) fn set_since(&mut self, since: Instant) {
        self.since = Some(since);
    }

    /// Test hook: the request appeared `ago` before now.
    #[cfg(test)]
    pub(crate) fn backdate(&mut self, ago: Duration) {
        if let Some(since) = self.since.as_mut() {
            *since -= ago;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decision(displayed: u64, refuses: bool, synthetic: bool, now: Instant) -> ConsentDecision<'static> {
        ConsentDecision { displayed, target: "req", choice: "Once", refuses, now, synthetic }
    }

    #[test]
    fn evidence_needs_the_displayed_request_real_input_and_the_arm_delay() {
        let now = Instant::now();
        let mut guard = ConsentGuard::default();
        assert!(guard.witness(decision(7, false, false, now)).is_err(), "nothing shown");
        guard.present(7, 1, now);
        assert_eq!(
            guard.witness(decision(7, false, false, now)).unwrap_err(),
            ConsentRefusal::NotArmed,
            "not armed"
        );
        assert_eq!(
            guard.witness(decision(7, false, true, now)).unwrap_err(),
            ConsentRefusal::Synthetic,
            "synthetic outranks not armed"
        );
        assert!(guard.witness(decision(7, true, false, now)).is_ok(), "refusal counts at once");
        let armed = now + ARM_DELAY;
        assert!(guard.witness(decision(8, false, false, armed)).is_err(), "another request");
        assert!(guard.witness(decision(7, false, true, armed)).is_err(), "synthetic");
        let evidence = guard.witness(decision(7, false, false, armed)).unwrap();
        assert!(evidence.covers("req", "Once"));
        assert!(!evidence.covers("req", "Always"), "bound to the choice");
        assert!(!evidence.covers("other", "Once"), "bound to the request");
        assert!(evidence.admit("req", "Once").is_ok());
        assert!(evidence.admit("req", "Always").is_err(), "a sink cannot widen the choice");
        assert!(evidence.admit("other", "Once").is_err(), "a sink cannot retarget it");
    }

    #[test]
    fn a_request_that_left_the_screen_arms_again() {
        let now = Instant::now();
        let mut guard = ConsentGuard::default();
        guard.present(7, 1, now);
        let later = now + ARM_DELAY;
        guard.present(7, 5, later);
        assert!(guard.witness(decision(7, false, false, later)).is_err());
    }
}
