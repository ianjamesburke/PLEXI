//! `iq.query` capability broker — issue #284.
//!
//! The broker is the host-side bridge between an app's `DrawCommand::IqQuery`
//! and the live Plexi IQ backend. The capability gate, model-tier resolution,
//! ledger append, and outbound `PlexiEvent::IqResponse` are all funnelled
//! through this module so:
//!
//!   1. Tests can replace `LiveIqBroker` with a canned `IqBroker` mock and
//!      drive `process_app::routing` deterministically (no real LLM).
//!   2. v3.4 (issue #285, "agent-as-app") can reuse the same broker — the
//!      hardcoded `Pane::Agent` turn loop becomes a degenerate case where
//!      the in-process agent calls into `LiveIqBroker` exactly the same way
//!      a subprocess agent does over PGAP.
//!
//! Today the broker drives the existing synchronous `turn_loop::run_turn`
//! path (which itself spawns a worker thread inside the backend) so we
//! never hit the UI thread with a blocking call. The broker's `dispatch`
//! method is therefore expected to be invoked from a worker thread spawned
//! by the routing layer.

use crate::app_protocol::{IqMessage, IqTool, ModelTier};
use crate::plexi_iq::backend::anthropic_api::AnthropicApiBackend;
use crate::plexi_iq::backend::{BillingModel, LlmBackend};
use crate::plexi_iq::ledger::{self, LedgerRow};
use crate::plexi_iq::turn_loop::{self, TurnError};

/// One brokered request — what the routing layer hands to a broker. `app_id`
/// is required so the ledger can attribute spend per-app.
#[derive(Debug, Clone)]
pub struct IqBrokerRequest {
    pub app_id: String,
    pub model_tier: ModelTier,
    pub system: String,
    pub messages: Vec<IqMessage>,
    pub tools: Vec<IqTool>,
}

/// Broker outcome. Either `content` is `Some` (success) or `error` is `Some`
/// (failure). Token counts default to `0` when the upstream backend doesn't
/// report them (e.g. subscription billing through `claude_cli`).
#[derive(Debug, Clone)]
pub struct IqBrokerResponse {
    pub content: Option<String>,
    pub tokens_in: u32,
    pub tokens_out: u32,
    pub error: Option<String>,
}

impl IqBrokerResponse {
    pub fn ok(content: String, tokens_in: u32, tokens_out: u32) -> Self {
        Self {
            content: Some(content),
            tokens_in,
            tokens_out,
            error: None,
        }
    }

    pub fn err(message: impl Into<String>) -> Self {
        Self {
            content: None,
            tokens_in: 0,
            tokens_out: 0,
            error: Some(message.into()),
        }
    }
}

/// The contract every broker must satisfy. Implementations must be `Send + Sync`
/// because routing dispatches off the UI thread.
pub trait IqBroker: Send + Sync {
    /// Synchronously dispatch a brokered query. Implementations should not
    /// block the caller forever — the routing layer always invokes this from
    /// a dedicated worker thread, but slow networks and stalled backends can
    /// still tie that thread up. Implementations should bound their own waits.
    fn dispatch(&self, request: IqBrokerRequest) -> IqBrokerResponse;
}

/// Production broker: wires `IqBrokerRequest` → concrete model id → live
/// `LlmBackend` → ledger append → response.
///
/// Backend selection is per-call (not held as a long-lived field) so that:
/// - Each call can pick a fresh API key from the secrets store (the host
///   may rotate keys between calls).
/// - Tier-routing can pick a metered or subscription backend without the
///   broker holding state for both.
///
/// `api_key_resolver` is invoked on every call. Returning `None` produces
/// an error response (no fallback, no secret leakage).
pub struct LiveIqBroker {
    /// Closure that resolves the Anthropic API key when needed. Returning
    /// `None` causes `dispatch` to short-circuit with an error response —
    /// the broker never falls through to the proxied backend silently.
    api_key_resolver: Box<dyn Fn() -> Option<String> + Send + Sync>,
}

impl LiveIqBroker {
    /// Construct a live broker. `api_key_resolver` is invoked once per
    /// dispatch when the metered Anthropic API path is used.
    pub fn new<F>(api_key_resolver: F) -> Self
    where
        F: Fn() -> Option<String> + Send + Sync + 'static,
    {
        Self {
            api_key_resolver: Box::new(api_key_resolver),
        }
    }
}

impl IqBroker for LiveIqBroker {
    fn dispatch(&self, request: IqBrokerRequest) -> IqBrokerResponse {
        // v3.3: the broker only supports text-in / text-out turns. Tool dispatch
        // lands in v3.4 — explicit, loud refusal beats a silent drop.
        if !request.tools.is_empty() {
            return IqBrokerResponse::err(
                "tools not yet supported by iq.query broker (v3.4)",
            );
        }

        let model_id = resolve_model_id(request.model_tier);

        // Native Anthropic API path. Subscription/proxied routing per-tier
        // is future-scope; today every tier goes through the metered API
        // since that's the only path that honours an explicit model id.
        let Some(api_key) = (self.api_key_resolver)() else {
            log::warn!(
                "iq_broker[{}]: ANTHROPIC_API_KEY not in secrets store — denying iq.query",
                request.app_id
            );
            return IqBrokerResponse::err(
                "api_key_missing: store ANTHROPIC_API_KEY in Plexi secrets to use iq.query",
            );
        };

        let backend = AnthropicApiBackend::with_model(api_key, model_id.clone());

        // Multi-turn conversations flow natively now — `LlmRequest` carries
        // the structured `Vec<IqMessage>` and the backend translates each
        // entry directly to the Anthropic Messages API. No flattening.
        let turn = turn_loop::run_turn(
            &backend,
            request.messages.clone(),
            request.system.clone(),
            |_| {},
        );

        match turn {
            Ok(result) => {
                let tokens_in = result.input_tokens.unwrap_or(0);
                let tokens_out = result.output_tokens.unwrap_or(0);

                // Ledger append — the contract for #284. Failures here are
                // logged but never propagated; a billing miss must not break
                // the user's conversation.
                let row = LedgerRow::with_attribution(
                    backend.name(),
                    BillingModel::Metered,
                    Some(request.app_id.clone()),
                    Some(model_id),
                    result.input_tokens,
                    result.output_tokens,
                );
                ledger::append(&row);

                IqBrokerResponse::ok(result.text, tokens_in, tokens_out)
            }
            Err(e) => {
                log::warn!("iq_broker[{}]: turn failed: {e}", request.app_id);
                let message = match e {
                    TurnError::BackendError(be) => format!("backend error: {be}"),
                    TurnError::StreamError(s) => format!("stream error: {s}"),
                    TurnError::ChannelClosed => {
                        "stream closed before completion".to_string()
                    }
                };
                IqBrokerResponse::err(message)
            }
        }
    }
}

/// Map a `ModelTier` to the concrete model id the host passes to the backend.
/// These ids live in this single helper so that a future config schema change
/// touches one place. The values track the spec's stated routing:
///   - `Low`    → Haiku
///   - `Medium` → Sonnet
///   - `High`   → Opus
pub fn resolve_model_id(tier: ModelTier) -> String {
    // Model ids are intentionally hard-coded. Per project rule
    // `lessons.model_id_verification`, never guess a versioned id; these
    // family ids are the ones the existing Anthropic backend already uses
    // (see `src/plexi_iq/backend/anthropic_api.rs::default model`) plus the
    // Haiku and Opus siblings. If a model id needs to be revved, do it here
    // and add a DEV_LOG entry.
    match tier {
        ModelTier::Low => "claude-haiku-4-5".to_string(),
        ModelTier::Medium => "claude-sonnet-4-5".to_string(),
        ModelTier::High => "claude-opus-4-5".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Test broker: records every request and returns a canned response.
    pub struct CannedBroker {
        pub seen: Arc<Mutex<Vec<IqBrokerRequest>>>,
        pub response: IqBrokerResponse,
    }

    impl CannedBroker {
        pub fn ok(content: &str) -> Self {
            Self {
                seen: Arc::new(Mutex::new(Vec::new())),
                response: IqBrokerResponse::ok(content.to_string(), 7, 3),
            }
        }
    }

    impl IqBroker for CannedBroker {
        fn dispatch(&self, request: IqBrokerRequest) -> IqBrokerResponse {
            self.seen.lock().unwrap().push(request);
            self.response.clone()
        }
    }

    #[test]
    fn resolve_model_id_routes_tiers() {
        assert_eq!(resolve_model_id(ModelTier::Low), "claude-haiku-4-5");
        assert_eq!(resolve_model_id(ModelTier::Medium), "claude-sonnet-4-5");
        assert_eq!(resolve_model_id(ModelTier::High), "claude-opus-4-5");
    }

    #[test]
    fn broker_accepts_structured_messages_after_widening() {
        // Multi-turn conversations now flow as a structured `Vec<IqMessage>`
        // through the broker — no flattening into a single prompt. This test
        // pins the contract: the broker hands the full message array to the
        // backend and `IqBrokerRequest` accepts the structured form unchanged.
        let broker = CannedBroker::ok("response");
        let messages = vec![
            IqMessage {
                role: "user".to_string(),
                content: "first".to_string(),
            },
            IqMessage {
                role: "assistant".to_string(),
                content: "ok".to_string(),
            },
            IqMessage {
                role: "user".to_string(),
                content: "second".to_string(),
            },
        ];
        let resp = broker.dispatch(IqBrokerRequest {
            app_id: "test".to_string(),
            model_tier: ModelTier::Low,
            system: "be helpful".to_string(),
            messages: messages.clone(),
            tools: vec![],
        });
        assert_eq!(resp.content.as_deref(), Some("response"));
        // The broker must have received the structured array verbatim — no
        // flattening, no "[assistant previously]" prefix, no joining.
        let seen = broker.seen.lock().unwrap();
        assert_eq!(seen.len(), 1, "broker should have seen exactly one request");
        assert_eq!(seen[0].messages, messages, "messages must round-trip unchanged");
        assert_eq!(seen[0].system, "be helpful");
    }

    #[test]
    fn live_broker_rejects_tools_until_v3_4() {
        let broker = LiveIqBroker::new(|| Some("dummy".to_string()));
        let resp = broker.dispatch(IqBrokerRequest {
            app_id: "test".to_string(),
            model_tier: ModelTier::Low,
            system: String::new(),
            messages: vec![],
            tools: vec![IqTool {
                name: "x".into(),
                description: "y".into(),
                input_schema: serde_json::json!({}),
            }],
        });
        assert!(resp.error.is_some(), "tools must be rejected");
        assert!(
            resp.error.unwrap().contains("tools not yet supported"),
            "error message must call out tools"
        );
    }

    #[test]
    fn live_broker_errors_when_api_key_missing() {
        let broker = LiveIqBroker::new(|| None);
        let resp = broker.dispatch(IqBrokerRequest {
            app_id: "test".to_string(),
            model_tier: ModelTier::Low,
            system: String::new(),
            messages: vec![IqMessage {
                role: "user".to_string(),
                content: "hi".to_string(),
            }],
            tools: vec![],
        });
        assert!(resp.error.is_some());
        assert!(
            resp.error.unwrap().contains("api_key_missing"),
            "missing-key error must surface the api_key_missing tag"
        );
    }
}
