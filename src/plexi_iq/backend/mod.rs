//! LLM backend abstraction for the Plexi IQ broker.
//!
//! The concrete backend is `OpenRouterBackend` — routes to any OpenRouter
//! model via SSE streaming. The host reads `OPENROUTER_API_KEY` from the
//! environment at dispatch time; apps never see the key.
//!
//! Backends deliver events through an `mpsc::Sender<StreamEvent>` so the
//! caller can drain a receiver each frame without blocking the UI.

pub mod openrouter;

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
    /// `generation_id` carries the `X-Generation-Id` response header value
    /// (OpenRouter-specific); the broker uses it to fetch the real cost after
    /// the turn completes.
    Done {
        input_tokens: Option<u32>,
        output_tokens: Option<u32>,
        /// OpenRouter generation ID captured from the `X-Generation-Id`
        /// response header before reading the SSE body. Used to fetch
        /// real per-call cost via the generation endpoint.
        generation_id: Option<String>,
    },
    /// Terminal error. Surface in the conversation buffer.
    Error(String),
}

/// Conversation turn passed to `LlmBackend::stream_to_channel`.
///
/// `messages` carries the full conversation history as a structured array —
/// the same shape the Anthropic Messages API uses. Each entry has `role` ∈
/// {`"user"`, `"assistant"`} and a plain-text `content`. Backends MUST honour
/// the array (no flattening to a single prompt string) so multi-turn agent
/// conversations work correctly.
///
/// `system` is the system prompt; injected on every call. Empty string =
/// no system prompt.
#[derive(Debug, Clone, Default)]
pub struct LlmRequest {
    /// Full structured conversation history. Wire shape mirrors the Anthropic
    /// Messages API — see `app_protocol::IqMessage`.
    pub messages: Vec<crate::app_protocol::IqMessage>,
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
