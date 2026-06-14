//! Plexi AI — agent runtime + brokered LLM capability.
//!
//! Public surface today (#284):
//!   - `backend` — `AiBackend` trait + concrete `OpenRouterBackend` + `OllamaBackend`.
//!   - `broker`  — `AiBroker` trait + `LiveAiBroker`. The host-side bridge
//!     that fields `DrawCommand::AiQuery`, gates on the `ai.query`
//!     capability, dispatches to a backend, and writes the ledger.
//!   - `ledger`  — append-only JSONL row of every brokered call.
//!   - `turn_loop` — single-turn driver consumed by the broker.

pub mod backend;
pub mod broker;
pub mod ledger;
pub mod tool_dispatch;
#[path = "loop.rs"]
pub mod turn_loop;

use std::sync::atomic::{AtomicBool, Ordering};
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
/// [`cancel`]: CancelToken::cancel
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    /// A fresh token that has not been cancelled.
    pub fn new() -> Self {
        Self::default()
    }

    /// Trip the token. Every clone observes the cancellation. Idempotent.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    /// True once any clone of this token has been cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}
