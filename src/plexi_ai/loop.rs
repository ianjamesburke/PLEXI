//! Turn loop driver — stream → collect text → ledger → return.
//!
//! Drives a single user→assistant exchange against an `AiBackend`. The loop:
//!   1. Builds an `AiBackendRequest`.
//!   2. Opens an mpsc channel and calls `backend.stream_to_channel(request, tx)`.
//!   3. Drains `StreamEvent`s from the receiver, accumulating text.
//!   4. On `Done`, returns the assembled text plus optional token counts.
//!   5. On `Error`, returns `Err` so the caller can surface it.
//!
//! Tool dispatch and cost-ledger writes both live in the broker (`broker.rs`);
//! the turn loop is the lower-level primitive for "one streaming exchange".
//!
//! NOTE: the file is named `loop.rs` to match the spec, but `loop` is a
//! Rust keyword, so `mod.rs` mounts it as `pub mod turn_loop` via
//! `#[path = "loop.rs"]`.

use std::sync::mpsc;

use crate::app_protocol::AiMessage;
use crate::plexi_ai::backend::{AiBackend, AiBackendError, AiBackendRequest, StreamEvent};

/// Outcome of a completed turn.
pub struct TurnResult {
    /// Full assistant response text accumulated from streamed chunks.
    pub text: String,
    /// Input token count — `Some` only for metered (native API) backend.
    pub input_tokens: Option<u32>,
    /// Output token count — `Some` only for metered (native API) backend.
    pub output_tokens: Option<u32>,
    /// OpenRouter generation ID from `X-Generation-Id` response header.
    /// `Some` when the OpenRouter backend is used; `None` otherwise.
    /// The broker uses this to fetch the real per-call cost after the turn.
    pub generation_id: Option<String>,
}

/// Error variants for a failed turn.
#[derive(Debug)]
pub enum TurnError {
    /// Backend could not start streaming.
    BackendError(AiBackendError),
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
/// Synchronous — blocks until the backend delivers `StreamEvent::Done` or
/// `StreamEvent::Error`. For UI use, call from a dedicated worker thread.
///
/// `messages` is the full structured conversation history (Anthropic shape:
/// alternating user/assistant turns). Multi-turn conversations now flow
/// natively through this path — no flattening at the broker.
///
/// `on_token` is called with each text chunk as it arrives. Pass a no-op
/// closure if streaming is not needed (e.g. in tests).
pub fn run_turn(
    backend: &dyn AiBackend,
    messages: Vec<AiMessage>,
    system: impl Into<String>,
    mut on_token: impl FnMut(&str),
) -> Result<TurnResult, TurnError> {
    let request = AiBackendRequest {
        messages,
        system: system.into(),
    };

    let (tx, rx) = mpsc::channel::<StreamEvent>();

    backend
        .stream_to_channel(request, tx)
        .map_err(TurnError::BackendError)?;

    let mut text = String::new();
    let mut input_tokens: Option<u32> = None;
    let mut output_tokens: Option<u32> = None;
    let mut generation_id: Option<String> = None;

    loop {
        match rx.recv() {
            Ok(StreamEvent::Text(chunk)) => {
                on_token(&chunk);
                text.push_str(&chunk);
            }
            Ok(StreamEvent::Done {
                input_tokens: in_tok,
                output_tokens: out_tok,
                generation_id: gen_id,
            }) => {
                input_tokens = in_tok;
                output_tokens = out_tok;
                generation_id = gen_id;
                break;
            }
            Ok(StreamEvent::Error(msg)) => {
                return Err(TurnError::StreamError(msg));
            }
            Err(_) => {
                // Sender dropped without sending Done — treat as unexpected close.
                if text.is_empty() {
                    return Err(TurnError::ChannelClosed);
                }
                log::warn!("plexi_ai: stream channel closed before Done; returning partial text");
                break;
            }
        }
    }

    Ok(TurnResult {
        text,
        input_tokens,
        output_tokens,
        generation_id,
    })
}
