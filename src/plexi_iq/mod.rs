//! Plexi IQ — in-process agent runtime (v3).
//!
//! This module is the agent runtime for `Pane::Agent` as specified in
//! `docs/specs/releases/plexi-v3.0.md §9`. It provides:
//!
//! - A **turn loop** (`turn_loop::run_turn`) that drives a single user→assistant
//!   exchange, delivering streamed text via a closure and writing a ledger row.
//! - A **backend abstraction** (`LlmBackend` trait) with two concrete implementations:
//!   `ClaudeCliBackend` (proxied, subscription-billed) and `AnthropicApiBackend`
//!   (native Messages API, metered).
//! - A **cost ledger** (`ledger`) that appends JSONL rows to the build-specific
//!   config dir on every completed turn.
//! - A **tool context** (`context.rs`) — minimal `pane_id` + `directory_scope`
//!   pair every tool receives. Stage 2 fields (session, app bus) land with
//!   the first tool that needs them.
//!
//! Call sites into `Pane::Agent` are wired in Layer 3 — not here.

pub mod backend;
pub mod context;
pub mod ledger;
#[path = "loop.rs"]
pub mod turn_loop;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Top-level Plexi IQ handle — owns shared configuration and spawns per-pane
/// `PlexiIqInstance`s. Stage 1 will give this real fields (backend factory,
/// global budget ledger handle, MCP client pool, etc.). For now it is a
/// zero-sized marker so downstream code can name the type.
#[derive(Debug, Default)]
pub struct PlexiIq {}

/// Configuration passed when constructing a `PlexiIq`. See spec §3.6 for the
/// backend-selection logic and §10 for budget fields that will land here.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct PlexiIqConfig {
    /// Optional override for the directory scope used when bootstrapping
    /// instances. When `None`, instances will inherit the pane's cwd.
    pub default_directory_scope: Option<PathBuf>,
    /// Which backend to use for LLM calls.
    #[serde(default)]
    pub backend: IqBackend,
    /// API key for the native Anthropic backend (when `backend = "native_api"`).
    #[serde(default)]
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IqBackend {
    #[default]
    ClaudeProxy,
    NativeApi,
}

/// Per-pane agent instance. Owns its own conversation, tool registry, and
/// session state. Stage 1 will add the turn loop driver and the channels that
/// connect it to the pane's `AgentMode` state machine.
#[derive(Debug, Default)]
pub struct PlexiIqInstance {}

/// Per-pane IQ session. Tracks session state for `claude -p --resume`.
pub struct IqSession {
    pub session_id: String,
    pub run_id: Option<String>,
    pub workspace_dir: PathBuf,
}

impl IqSession {
    pub fn new(workspace_dir: PathBuf) -> Self {
        // Use a timestamp-based ID to avoid uuid dependency.
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        Self {
            session_id: format!("plexi-iq-{ts:x}"),
            run_id: None,
            workspace_dir,
        }
    }

    pub async fn send(&self, prompt: &str) -> Result<String, String> {
        backend::run_claude_proxy(&self.session_id, prompt, &self.workspace_dir).await
    }
}
