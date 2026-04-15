//! Native mode — direct Anthropic Messages API via `async-anthropic`.
//!
//! Stage 0: empty struct + stub `LlmBackend` impl that `todo!()`s. Stage 1
//! will:
//!
//! - Construct an `async_anthropic::Client` from the configured API key
//!   (`ANTHROPIC_API_KEY` env var or Plexi secrets).
//! - Translate `LlmRequest` into an Anthropic `MessagesRequest` with
//!   `cache_control` markers on the system prefix + last stable user turn
//!   (spec §4).
//! - Enable the 1M context beta header behind a config flag (spec §8
//!   risk #6).
//! - Expose `max_thinking_tokens` per call (spec §3.6).
//!
//! No `async-anthropic` calls in Stage 0 — just the trait surface.

use super::{BillingModel, LlmBackend, LlmError, LlmRequest, StreamEvent};

/// Direct Anthropic Messages API backend. Owns the tool loop in Plexi IQ.
#[derive(Debug, Default)]
pub struct AnthropicApiBackend {}

impl AnthropicApiBackend {
    /// Placeholder constructor. Stage 1 will take an API key + model routing
    /// config and build a real `async_anthropic::Client`.
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
impl LlmBackend for AnthropicApiBackend {
    fn name(&self) -> &str {
        "anthropic-api"
    }

    fn supports_tool_dispatch(&self) -> bool {
        // Native mode — Plexi IQ owns the tool loop. See spec §3.6.
        true
    }

    fn billing_model(&self) -> BillingModel {
        BillingModel::Metered
    }

    async fn stream(&self, _request: LlmRequest) -> Result<StreamEvent, LlmError> {
        todo!("Plexi IQ Stage 1: stream via async-anthropic (spec §3.6 native mode).")
    }
}
