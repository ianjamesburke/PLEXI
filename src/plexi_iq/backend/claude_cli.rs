//! Proxied mode — slot-in of the existing `src/agent_llm.rs` wrapper.
//!
//! Stage 0: empty struct + stub `LlmBackend` impl that `todo!()`s. Stage 1
//! will move the existing `claude -p --resume <session_id>` subprocess
//! logic (currently in `src/agent_llm.rs` on the `dev` branch, not yet in
//! this alpha worktree) behind this backend.
//!
//! Capability note (spec §3.6, risk #12): in proxied mode, Claude Code
//! owns the tool loop internally. Plexi IQ sees only prompts + streamed
//! text. `supports_tool_dispatch()` therefore returns `false`, and the
//! turn loop will skip building a tool schema and collecting tool_use
//! blocks when this backend is active. The pane-header badge must show
//! "proxied — tool dispatch disabled" so users don't silently lose
//! app-protocol tools, MCP, or subagents-as-panes.

use super::{BillingModel, LlmBackend, LlmError, LlmRequest, StreamEvent};

/// `claude -p --resume` subprocess backend. Wraps the existing
/// `agent_llm.rs` in the `LlmBackend` trait. Stage 1 plumbs the real
/// subprocess in.
#[derive(Debug, Default)]
pub struct ClaudeCliBackend {}

impl ClaudeCliBackend {
    /// Placeholder constructor. Stage 1 will accept the path to the
    /// `claude` binary and an optional session ID for `--resume`.
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
impl LlmBackend for ClaudeCliBackend {
    fn name(&self) -> &str {
        "claude-cli"
    }

    fn supports_tool_dispatch(&self) -> bool {
        // Proxied mode — Claude Code owns the tool loop. See spec §3.6.
        false
    }

    fn billing_model(&self) -> BillingModel {
        BillingModel::Subscription
    }

    async fn stream(&self, _request: LlmRequest) -> Result<StreamEvent, LlmError> {
        todo!("Plexi IQ Stage 1: slot in src/agent_llm.rs behind this trait (spec §3.6 proxied mode).")
    }
}
