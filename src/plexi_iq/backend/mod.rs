//! LLM backend abstraction for Plexi IQ — Stage 1.
//!
//! Two concrete backends (spec §3.6):
//!
//! - `ClaudeCliBackend` — proxied mode. Slots the existing `src/agent_llm.rs`
//!   `claude -p --resume --output-format stream-json` subprocess in behind the
//!   trait. Claude Code owns the tool loop; Plexi IQ sees only streamed text.
//!
//! - `AnthropicApiBackend` — native mode. Direct Messages API via
//!   `async-anthropic`. Plexi IQ owns the tool loop; raw tool_use events are
//!   visible. Requires `ANTHROPIC_API_KEY` or a key from the Plexi secrets store.
//!
//! Both backends deliver events through an `mpsc::Sender<StreamEvent>` so the
//! UI thread can drain a receiver each frame without blocking.

pub mod anthropic_api;
pub mod claude_cli;

pub use anthropic_api::AnthropicApiBackend;
pub use claude_cli::ClaudeCliBackend;

use std::sync::mpsc;

/// Invoke `claude -p --resume <session_id>` with the given prompt.
/// Writes prompt to stdin, reads response from stdout.
/// This is a blocking call — run in a spawned thread if needed.
pub async fn run_claude_proxy(
    session_id: &str,
    prompt: &str,
    workspace_dir: &std::path::Path,
) -> Result<String, String> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let mut child = Command::new("claude")
        .arg("-p")
        .arg("--resume")
        .arg(session_id)
        .current_dir(workspace_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to spawn claude: {e}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(prompt.as_bytes())
            .map_err(|e| format!("stdin write: {e}"))?;
        stdin
            .write_all(b"\n")
            .map_err(|e| format!("stdin newline: {e}"))?;
    }

    let output = child
        .wait_with_output()
        .map_err(|e| format!("wait: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        log::warn!("plexi_iq: claude exited non-zero: {stderr}");
    }

    let text = String::from_utf8_lossy(&output.stdout).to_string();
    Ok(text.trim().to_string())
}

/// Whether the backend is billed per-token or via a flat subscription.
/// Drives the cost ledger (§10) and the pre-flight budget gate (§8, risk #10).
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
        /// Session ID returned by the `claude -p` process (proxied mode).
        /// Used to `--resume` the next turn.
        session_id: Option<String>,
    },
    /// Terminal error. Surface in the conversation buffer.
    Error(String),
}

/// Conversation turn passed to `LlmBackend::stream_to_channel`.
#[derive(Debug, Clone, Default)]
pub struct LlmRequest {
    /// User message for this turn.
    pub prompt: String,
    /// System prompt (injected on turn 0 for proxied mode; every call for native).
    pub system: String,
    /// Resume token from a previous turn (`None` on the first turn).
    pub session_id: Option<String>,
}

/// Error returned when a backend call cannot start.
/// Individual stream failures are delivered as `StreamEvent::Error`.
#[derive(Debug)]
pub enum LlmError {
    /// Binary not found or API client cannot be constructed.
    NotAvailable(String),
    /// I/O failure before streaming began.
    Io(String),
    /// API-level rejection (auth, quota, bad request).
    Api(String),
}

impl std::fmt::Display for LlmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LlmError::NotAvailable(s) => write!(f, "backend not available: {s}"),
            LlmError::Io(s) => write!(f, "I/O error: {s}"),
            LlmError::Api(s) => write!(f, "API error: {s}"),
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

    /// Whether this backend exposes raw tool_use events to Plexi IQ's
    /// turn loop. `true` → native mode (Plexi IQ owns tool dispatch).
    /// `false` → proxied mode (Claude Code owns its own tool loop).
    fn supports_tool_dispatch(&self) -> bool;

    /// How this backend bills — drives the cost ledger and pre-flight gate.
    fn billing_model(&self) -> BillingModel;

    /// Start streaming a turn. Returns immediately; events arrive on `tx`.
    fn stream_to_channel(
        &self,
        request: LlmRequest,
        tx: mpsc::Sender<StreamEvent>,
    ) -> Result<(), LlmError>;
}
