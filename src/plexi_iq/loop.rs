//! Turn loop driver — stream → collect text → ledger → return.
//!
//! Stage 1: implements the minimum viable streaming turn loop for both
//! backends. The loop:
//!  1. Builds an `LlmRequest` from the user input and current session state.
//!  2. Opens an mpsc channel and calls `backend.stream_to_channel(request, tx)`.
//!  3. Drains `StreamEvent`s from the receiver, accumulating text.
//!  4. On `StreamEvent::Done`, appends a ledger row and returns the full
//!     assistant turn text plus an updated session ID (proxied backend only).
//!  5. On `StreamEvent::Error`, returns `Err` so the caller can surface it.
//!
//! The turn loop does NOT own tool dispatch in Stage 1. That comes in Stage 2
//! when `supports_tool_dispatch()` returns `true` and `tool_use` events are
//! wired up. For now, native mode behaves like proxied mode from the loop's
//! perspective: text in, text out.
//!
//! NOTE: the file is named `loop.rs` to match the spec, but `loop` is a
//! Rust keyword, so `mod.rs` mounts it as `pub mod turn_loop` via
//! `#[path = "loop.rs"]`.

use std::sync::mpsc;

use crate::plexi_iq::backend::{LlmBackend, LlmError, LlmRequest, StreamEvent};
use crate::plexi_iq::ledger::{self, LedgerRow};

/// Outcome of a completed turn.
pub struct TurnResult {
    /// Full assistant response text accumulated from streamed chunks.
    pub text: String,
    /// Session ID returned by `claude -p` (proxied backend). `None` for native
    /// backend (conversation state is managed server-side).
    pub session_id: Option<String>,
    /// Input token count — `Some` only for metered (native API) backend.
    pub input_tokens: Option<u32>,
    /// Output token count — `Some` only for metered (native API) backend.
    pub output_tokens: Option<u32>,
}

/// Error variants for a failed turn.
#[derive(Debug)]
pub enum TurnError {
    /// Backend could not start streaming (binary missing, bad API key, etc.).
    BackendError(LlmError),
    /// Streaming started but the backend reported an error mid-stream.
    StreamError(String),
    /// The stream channel closed unexpectedly before `Done` was received.
    ChannelClosed,
}

impl std::fmt::Display for TurnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TurnError::BackendError(e) => write!(f, "backend error: {e}"),
            TurnError::StreamError(s) => write!(f, "stream error: {s}"),
            TurnError::ChannelClosed => write!(f, "stream channel closed unexpectedly"),
        }
    }
}

/// Run a single user turn through the backend and return the full response.
///
/// This is a synchronous call — it blocks until the backend delivers
/// `StreamEvent::Done` or `StreamEvent::Error`. For UI use, run this on a
/// dedicated thread (the same pattern used by `agent_llm.rs`).
///
/// `on_token` is called with each text chunk as it arrives. Pass a no-op
/// closure if streaming is not needed (e.g. in tests).
pub fn run_turn(
    backend: &dyn LlmBackend,
    prompt: impl Into<String>,
    system: impl Into<String>,
    session_id: Option<String>,
    mut on_token: impl FnMut(&str),
) -> Result<TurnResult, TurnError> {
    let request = LlmRequest {
        prompt: prompt.into(),
        system: system.into(),
        session_id,
    };

    let (tx, rx) = mpsc::channel::<StreamEvent>();

    backend
        .stream_to_channel(request, tx)
        .map_err(TurnError::BackendError)?;

    let mut text = String::new();
    let mut result_session_id: Option<String> = None;
    let mut input_tokens: Option<u32> = None;
    let mut output_tokens: Option<u32> = None;

    loop {
        match rx.recv() {
            Ok(StreamEvent::Text(chunk)) => {
                on_token(&chunk);
                text.push_str(&chunk);
            }
            Ok(StreamEvent::Done {
                input_tokens: in_tok,
                output_tokens: out_tok,
                session_id: sid,
            }) => {
                input_tokens = in_tok;
                output_tokens = out_tok;
                result_session_id = sid;
                break;
            }
            Ok(StreamEvent::Error(msg)) => {
                return Err(TurnError::StreamError(msg));
            }
            Err(_) => {
                // Sender dropped without sending Done — treat as unexpected close.
                // If we have partial text, still surface it as an error so the
                // caller knows the turn was incomplete.
                if text.is_empty() {
                    return Err(TurnError::ChannelClosed);
                }
                // Partial text received before unexpected close — log and break
                // so the caller gets what we have rather than losing it entirely.
                log::warn!("plexi_iq: stream channel closed before Done; returning partial text");
                break;
            }
        }
    }

    // Append a ledger row. Errors here are non-fatal — a billing ledger failure
    // must never interrupt the conversation.
    let row = LedgerRow::new(
        backend.name(),
        backend.billing_model(),
        input_tokens,
        output_tokens,
    );
    ledger::append(&row);

    Ok(TurnResult {
        text,
        session_id: result_session_id,
        input_tokens,
        output_tokens,
    })
}
