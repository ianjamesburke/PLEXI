//! AI backend abstraction for the Plexi AI broker.
//!
//! Concrete backends: `OpenRouterBackend` (SSE streaming via OpenRouter) and
//! `OllamaBackend` (NDJSON streaming via local Ollama). The host reads API
//! keys from the environment at dispatch time; apps never see keys.
//!
//! Backends deliver events through an `mpsc::Sender<StreamEvent>` so the
//! caller can drain a receiver each frame without blocking the UI.

pub mod ollama;
pub mod openrouter;

use std::sync::mpsc;

use crate::app_protocol::ModelTier;

/// Whether the backend is billed per-token or via a flat subscription.
/// Drives the cost ledger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BillingModel {
    /// Per-token USD. Pre-flight enforcement against a dollar envelope.
    Metered,
    /// Flat-rate upstream (e.g. Claude Code subscription). Rate limits are
    /// enforced by the provider, not Plexi AI.
    Subscription,
}

/// Streaming event delivered to the turn loop.
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// Incremental text chunk — append to the pane's output buffer.
    Text(String),
    /// Incremental reasoning ("thinking") chunk from a reasoning model.
    /// Displayed separately from the answer text; never part of `TurnResult::text`.
    Reasoning(String),
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
    /// The model requested one or more tool calls. The turn loop stores these
    /// and returns them in `TurnResult`; the broker dispatches them and runs
    /// another turn with the results appended.
    ToolCalls(Vec<RawToolCall>),
    /// Terminal error. Surface in the conversation buffer.
    Error(String),
}

/// One tool call as received from the backend (pre-parsed SSE deltas).
#[derive(Debug, Clone)]
pub struct RawToolCall {
    /// Provider-assigned call ID (e.g. `"call_abc123"`).
    pub id: String,
    /// Tool name (e.g. `"increment"`).
    pub name: String,
    /// Raw JSON arguments string (e.g. `"{\"n\": 3}"`).
    pub arguments: String,
}

/// Conversation turn passed to `AiBackend::stream_to_channel`.
///
/// `messages` carries the full conversation history as `serde_json::Value`
/// objects so the broker can inject multi-turn tool-call/tool-result messages
/// directly without an intermediate AiMessage type that can't represent them.
/// Backends forward the array as-is into the request body.
///
/// `system` is the system prompt; injected on every call. Empty string =
/// no system prompt.
#[derive(Debug, Clone, Default)]
pub struct AiBackendRequest {
    /// Full structured conversation history as JSON values.
    /// Normal turns: `{"role": "user"|"assistant", "content": "…"}`.
    /// Tool-call turns: `{"role": "assistant", "content": null, "tool_calls": […]}`.
    /// Tool-result turns: `{"role": "tool", "content": "…", "tool_call_id": "…"}`.
    pub messages: Vec<serde_json::Value>,
    /// System prompt (injected on every call for the native API).
    pub system: String,
    /// Tools to inject into the request when non-empty.
    pub tools: Vec<crate::app_protocol::AiTool>,
    /// Model tier from the broker request. Used by backends to apply
    /// tier-specific request parameters (e.g. disabling reasoning for Low).
    pub model_tier: Option<ModelTier>,
}

/// Error returned when a backend call cannot start.
/// Individual stream failures are delivered as `StreamEvent::Error`.
#[derive(Debug)]
pub enum AiBackendError {
    /// I/O failure before streaming began.
    Io(String),
}

impl std::fmt::Display for AiBackendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AiBackendError::Io(s) => write!(f, "I/O error: {s}"),
        }
    }
}

/// The core backend contract.
///
/// `stream_to_channel` spawns background work and delivers `StreamEvent`s
/// into `tx`. The caller (turn loop) holds the receiver and drains it
/// each UI frame, so the main thread is never blocked.
pub trait AiBackend: Send + Sync {
    /// Human-readable backend name for logs and the pane-header badge.
    fn name(&self) -> &str;

    /// Start streaming a turn. Returns immediately; events arrive on `tx`.
    fn stream_to_channel(
        &self,
        request: AiBackendRequest,
        tx: mpsc::Sender<StreamEvent>,
    ) -> Result<(), AiBackendError>;
}
