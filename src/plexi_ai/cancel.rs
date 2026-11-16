use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

/// Cooperative cancellation handle for an in-flight turn.
///
/// Cloned from the caller (e.g. the host Assistant) down into the broker tool
/// loop and the backend's streaming read loop. Tripping it via [`cancel`]
/// makes both bail at their next checkpoint: the backend stops reading the SSE
/// body and drops its sender, and the broker stops before starting another
/// tool-call round. This is the seam that turns ESC / event-preempt into a
/// real network abort instead of a UI-only "move on".
///
/// [`cancel`]: CancelToken::cancel_with_reason
#[derive(Debug, Clone)]
pub struct CancelToken {
    state: Arc<AtomicU8>,
    turn_id: Arc<str>,
}

impl Default for CancelToken {
    fn default() -> Self {
        Self::for_turn(uuid::Uuid::new_v4().to_string())
    }
}

impl CancelToken {
    /// A fresh token that has not been cancelled.
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn for_turn(turn_id: impl Into<Arc<str>>) -> Self {
        Self {
            state: Arc::new(AtomicU8::new(0)),
            turn_id: turn_id.into(),
        }
    }

    /// Trip the token. Every clone observes the cancellation. Idempotent.
    #[cfg(test)]
    pub fn cancel(&self) {
        self.cancel_with_reason("caller cancelled turn");
    }

    pub(crate) fn cancel_with_reason(&self, reason: &str) {
        // Bit 0 rejects stale work; bit 1 records worker completion. Keep
        // rejecting stale work after completion, but only log a live cancel.
        if self.state.fetch_or(1, Ordering::AcqRel) == 0 {
            log::info!("turn cancelled turn_id={} reason={reason}", self.turn_id);
        }
    }

    pub(crate) fn mark_finished(&self) {
        self.state.fetch_or(2, Ordering::AcqRel);
    }

    pub(crate) fn finished() -> Self {
        let token = Self::new();
        token.mark_finished();
        token
    }

    /// True once any clone of this token has been cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.state.load(Ordering::Acquire) & 1 != 0
    }

    pub(crate) fn same_turn(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.state, &other.state)
    }

    /// A queued request must not keep its worker alive after the owner cancels.
    pub(crate) fn recv<T>(&self, receiver: &std::sync::mpsc::Receiver<T>) -> Option<T> {
        self.recv_timeout(receiver, std::time::Duration::MAX).ok()
    }

    pub(crate) fn recv_timeout<T>(
        &self,
        receiver: &std::sync::mpsc::Receiver<T>,
        timeout: std::time::Duration,
    ) -> Result<T, std::sync::mpsc::RecvTimeoutError> {
        use std::sync::mpsc::RecvTimeoutError;
        let started = std::time::Instant::now();
        while !self.is_cancelled() {
            let remaining = timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return Err(RecvTimeoutError::Timeout);
            }
            match receiver.recv_timeout(remaining.min(std::time::Duration::from_millis(20))) {
                Ok(value) => {
                    return if self.is_cancelled() {
                        Err(RecvTimeoutError::Disconnected)
                    } else {
                        Ok(value)
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(error) => return Err(error),
            }
        }
        Err(RecvTimeoutError::Disconnected)
    }
}
