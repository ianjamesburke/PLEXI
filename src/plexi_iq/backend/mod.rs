//! LLM backend abstraction for the Plexi IQ broker.
//!
//! Today (#284) the only concrete backend is `AnthropicApiBackend` — direct
//! Messages API via `async-anthropic`. The `claude_cli` proxied backend
//! lands with #285 (agent-as-app) when the v2 in-process turn loop is
//! retired and Plexi IQ becomes a thin broker on top of subprocess agents.
//!
//! Backends deliver events through an `mpsc::Sender<StreamEvent>` so the
//! caller can drain a receiver each frame without blocking the UI.

pub mod anthropic_api;

use std::sync::mpsc;

/// Whether the backend is billed per-token or via a flat subscription.
/// Drives the cost ledger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BillingModel {
    /// Per-token USD. Pre-flight enforcement against a dollar envelope.
    Metered,
    /// Flat-rate upstream (e.g. Claude Code subscription). Rate limits are
    /// enforced by the provider, not Plexi IQ.
    Subscription,
}

/// Streaming event delivered to the turn loop.
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// Incremental text chunk — append to the pane's output buffer.
    Text(String),
    /// Turn complete. Token counts are `Some` only for metered backends.
    Done {
        input_tokens: Option<u32>,
        output_tokens: Option<u32>,
    },
    /// Terminal error. Surface in the conversation buffer.
    Error(String),
}

/// Conversation turn passed to `LlmBackend::stream_to_channel`.
#[derive(Debug, Clone, Default)]
pub struct LlmRequest {
    /// User message for this turn.
    pub prompt: String,
    /// System prompt (injected on every call for the native API).
    pub system: String,
}

/// Error returned when a backend call cannot start.
/// Individual stream failures are delivered as `StreamEvent::Error`.
#[derive(Debug)]
pub enum LlmError {
    /// I/O failure before streaming began.
    Io(String),
}

impl std::fmt::Display for LlmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LlmError::Io(s) => write!(f, "I/O error: {s}"),
        }
    }
}

/// The core backend contract.
///
/// `stream_to_channel` spawns background work and delivers `StreamEvent`s
/// into `tx`. The caller (turn loop) holds the receiver and drains it
/// each UI frame, so the main thread is never blocked.
pub trait LlmBackend: Send + Sync {
    /// Human-readable backend name for logs and the pane-header badge.
    fn name(&self) -> &str;

    /// Start streaming a turn. Returns immediately; events arrive on `tx`.
    fn stream_to_channel(
        &self,
        request: LlmRequest,
        tx: mpsc::Sender<StreamEvent>,
    ) -> Result<(), LlmError>;
}
