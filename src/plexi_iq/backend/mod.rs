//! LLM backend abstraction for Plexi IQ.
//!
//! Two concrete backends land in Stage 1 behind this trait (spec §3.6):
//!
//! - `AnthropicApiBackend` — native mode, direct Messages API via
//!   `async-anthropic`. Plexi IQ owns the tool loop.
//! - `ClaudeCliBackend` — proxied mode, slots the existing
//!   `src/agent_llm.rs` `claude -p --resume` wrapper in behind the trait.
//!   Claude Code owns the tool loop internally; Plexi IQ sees only
//!   streamed text.
//!
//! Stage 0 is just the trait shape. Both implementations are `todo!()`
//! stubs so Stage 1 can fill them in without touching the trait.

pub mod anthropic_api;
pub mod claude_cli;

pub use anthropic_api::AnthropicApiBackend;
pub use claude_cli::ClaudeCliBackend;

/// Whether the backend is billed per-token or via a flat subscription.
/// Drives the cost ledger (§10) and the pre-flight budget gate (§8, risk #10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BillingModel {
    /// Per-token USD. Pre-flight enforcement against a dollar envelope.
    Metered,
    /// Flat-rate upstream (e.g. Claude Code subscription). Rate limits are
    /// enforced by the provider, not Plexi IQ.
    Subscription,
}

/// Streaming event yielded by `LlmBackend::stream`. Stage 1 will flesh this
/// out to cover text deltas, tool-use start/delta/stop, and message stop with
/// a stop reason; for now it's a single opaque variant so the trait signature
/// can be stable.
#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// Placeholder — replaced in Stage 1 with the full event taxonomy
    /// described in spec §3.2.
    Placeholder,
}

/// Request object passed to `LlmBackend::stream`. Stage 1 will expand this
/// into the full Messages API envelope (system prompt, messages, tools,
/// cache breakpoints, max_thinking_tokens, etc.).
#[derive(Debug, Clone, Default)]
pub struct LlmRequest {
    /// Placeholder — real fields land in Stage 1.
    pub _placeholder: (),
}

/// Error type for backend streaming. Stage 1 will split this into
/// transport / rate-limit / budget / abort variants.
#[derive(Debug)]
pub enum LlmError {
    /// Placeholder — real variants land in Stage 1.
    Todo,
}

/// The core backend contract. See spec §3.6 for the capability split and
/// §7 for why this is Anthropic-only (no multi-provider abstraction).
///
/// Stage 1 will almost certainly change the `stream` return type to a
/// proper `Stream<Item = Result<StreamEvent, LlmError>>` once we settle on
/// whether to use `futures::Stream` or a custom channel.
#[async_trait::async_trait]
pub trait LlmBackend: Send + Sync {
    /// Human-readable backend name for logs and the pane-header badge.
    fn name(&self) -> &str;

    /// Whether this backend exposes raw tool_use events to Plexi IQ's
    /// turn loop. Native mode returns `true`; proxied mode returns
    /// `false` because Claude Code owns its own tool loop.
    fn supports_tool_dispatch(&self) -> bool;

    /// How this backend bills — drives the cost ledger and pre-flight
    /// budget gate.
    fn billing_model(&self) -> BillingModel;

    /// Stream a single LLM request. Stage 1 will change the return type
    /// to a real async stream; for now this is a placeholder signature
    /// so the trait compiles.
    async fn stream(&self, request: LlmRequest) -> Result<StreamEvent, LlmError>;
}
