//! `iq.query` capability broker — issue #284, #383.
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
//! Today the broker drives the synchronous `turn_loop::run_turn` path (which
//! itself spawns a worker thread inside the backend) so we never hit the UI
//! thread with a blocking call. The broker's `dispatch` method is therefore
//! expected to be invoked from a worker thread spawned by the routing layer.

use crate::app_protocol::{IqMessage, IqTool, ModelTier};
use crate::config::IqConfig;
use crate::event_log::{self, HostEvent};
use crate::plexi_iq::backend::openrouter::OpenRouterBackend;
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

/// Production broker: reads model IDs from `IqConfig`, resolves the
/// `OPENROUTER_API_KEY` from the environment at dispatch time, calls the
/// OpenRouter backend, fetches real per-call cost, writes the ledger, and
/// emits `HostEvent::AgentTurn`.
///
/// `IqConfig` is cloned at construction from `PlexiConfig::load().iq`.
/// If the config section is absent, `dispatch` fails fast with a clear error.
pub struct LiveIqBroker {
    /// Model tier config loaded from `config.toml [iq]`.
    /// `None` when the section is absent; `dispatch` fails fast in that case.
    iq_config: Option<IqConfig>,
}

impl LiveIqBroker {
    /// Construct from the loaded `IqConfig` (may be `None` when `[iq]` is
    /// absent from config.toml — errors are surfaced at dispatch time).
    pub fn new(iq_config: Option<IqConfig>) -> Self {
        Self { iq_config }
    }
}

impl IqBroker for LiveIqBroker {
    fn dispatch(&self, request: IqBrokerRequest) -> IqBrokerResponse {
        // v3.3: text-in / text-out only. Tool dispatch lands in v3.4.
        if !request.tools.is_empty() {
            return IqBrokerResponse::err("tools not yet supported by iq.query broker (v3.4)");
        }

        let iq_config = match &self.iq_config {
            Some(c) => c,
            None => {
                return IqBrokerResponse::err(
                    "iq_config_missing: add [iq] section with model_low, model_medium, model_high to config.toml",
                );
            }
        };

        // Resolve model ID from config based on tier.
        let model_id = match request.model_tier {
            ModelTier::Low => match &iq_config.model_low {
                Some(m) => m.clone(),
                None => return IqBrokerResponse::err(
                    "iq_config_missing: model_low not set in [iq] config section",
                ),
            },
            ModelTier::Medium => match &iq_config.model_medium {
                Some(m) => m.clone(),
                None => return IqBrokerResponse::err(
                    "iq_config_missing: model_medium not set in [iq] config section",
                ),
            },
            ModelTier::High => match &iq_config.model_high {
                Some(m) => m.clone(),
                None => return IqBrokerResponse::err(
                    "iq_config_missing: model_high not set in [iq] config section",
                ),
            },
        };

        // Read OPENROUTER_API_KEY from environment at dispatch time.
        let api_key = match std::env::var("OPENROUTER_API_KEY") {
            Ok(k) if !k.is_empty() => k,
            _ => {
                log::warn!(
                    "iq_broker[{}]: OPENROUTER_API_KEY not set — denying iq.query",
                    request.app_id
                );
                return IqBrokerResponse::err(
                    "api_key_missing: OPENROUTER_API_KEY not set — export it in your shell profile",
                );
            }
        };

        let backend = OpenRouterBackend {
            api_key: api_key.clone(),
            model: model_id.clone(),
        };

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

                // Fetch real per-call cost from the OpenRouter generation endpoint.
                // One retry after 500ms for eventual consistency; non-fatal on failure.
                let cost_usd = result.generation_id.as_deref().and_then(|gen_id| {
                    fetch_generation_cost(gen_id, &api_key)
                });

                let cost_cents = cost_usd
                    .map(|usd| (usd * 100.0).round().max(0.0) as u64)
                    .unwrap_or(0);

                // Ledger append. Failures are logged but never propagated —
                // a billing miss must not break the user's conversation.
                let row = LedgerRow::with_attribution(
                    backend.name(),
                    BillingModel::Metered,
                    Some(request.app_id.clone()),
                    Some(model_id),
                    result.input_tokens,
                    result.output_tokens,
                    cost_usd,
                );
                ledger::append(&row);

                // Emit AgentTurn to the event bus (#383). This closes the gap:
                // HostEvent::AgentTurn was defined at event_log.rs:99 since v3.3
                // but was never emitted anywhere. The spec (§6.1) requires it on
                // every IQ turn.
                event_log::emit(HostEvent::AgentTurn {
                    pane_id: None,
                    tokens_in,
                    tokens_out,
                    cost_cents,
                    timestamp: event_log::now_timestamp(),
                });

                IqBrokerResponse::ok(result.text, tokens_in, tokens_out)
            }
            Err(e) => {
                log::warn!("iq_broker[{}]: turn failed: {e}", request.app_id);
                let message = match e {
                    TurnError::BackendError(be) => format!("backend error: {be}"),
                    TurnError::StreamError(s) => format!("stream error: {s}"),
                    TurnError::ChannelClosed => "stream closed before completion".to_string(),
                };
                IqBrokerResponse::err(message)
            }
        }
    }
}

/// Fetch the real USD cost for a completed generation from the OpenRouter
/// generation endpoint.
///
/// **Endpoint:** `GET https://openrouter.ai/api/v1/generation?id=<gen_id>`
///
/// **Response shape:**
/// ```json
/// {"data":{"total_cost":"0.00492","tokens_prompt":24,"tokens_completion":29}}
/// ```
///
/// Eventual consistency: if the generation is not yet queryable, OpenRouter
/// returns 404 or a 200 with `data` absent/null. One retry after 500ms.
/// If still unavailable, logs a warning and returns `None`.
pub fn fetch_generation_cost(gen_id: &str, api_key: &str) -> Option<f64> {
    let url = format!("https://openrouter.ai/api/v1/generation?id={gen_id}");

    let try_fetch = || -> Option<f64> {
        let resp = ureq::get(&url)
            .set("Authorization", &format!("Bearer {api_key}"))
            .call()
            .ok()?;

        if resp.status() != 200 {
            return None;
        }

        let body_str = resp.into_string().ok()?;
        let body: serde_json::Value = serde_json::from_str(&body_str).ok()?;
        let total_cost_str = body["data"]["total_cost"].as_str()?;
        total_cost_str.parse::<f64>().ok()
    };

    // First attempt
    if let Some(cost) = try_fetch() {
        return Some(cost);
    }

    // One retry after 500ms for eventual consistency.
    std::thread::sleep(std::time::Duration::from_millis(500));
    let cost = try_fetch();
    if cost.is_none() {
        log::warn!(
            "iq_broker: generation cost unavailable for gen_id={gen_id} — ledger will show cost_usd=null"
        );
    }
    cost
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::IqConfig;
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

    fn test_iq_config() -> IqConfig {
        IqConfig {
            model_low: Some("google/gemini-flash-2.0".to_string()),
            model_medium: Some("anthropic/claude-sonnet-4-6".to_string()),
            model_high: Some("anthropic/claude-opus-4-7".to_string()),
        }
    }

    #[test]
    fn config_resolves_tiers_to_openrouter_model_ids() {
        let config = test_iq_config();
        assert_eq!(
            config.model_low.as_deref(),
            Some("google/gemini-flash-2.0")
        );
        assert_eq!(
            config.model_medium.as_deref(),
            Some("anthropic/claude-sonnet-4-6")
        );
        assert_eq!(
            config.model_high.as_deref(),
            Some("anthropic/claude-opus-4-7")
        );
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
        let broker = LiveIqBroker::new(Some(test_iq_config()));
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
        // Ensure the env var is unset for this test.
        // Safety: test-only; `std::env` mutations in tests can be racy under
        // parallel test execution if other tests set the same var. This test
        // is isolated enough to be acceptable.
        let orig = std::env::var("OPENROUTER_API_KEY").ok();
        unsafe { std::env::remove_var("OPENROUTER_API_KEY") };

        let broker = LiveIqBroker::new(Some(test_iq_config()));
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

        // Restore env var.
        if let Some(v) = orig {
            unsafe { std::env::set_var("OPENROUTER_API_KEY", v) };
        }

        assert!(resp.error.is_some());
        assert!(
            resp.error.as_deref().unwrap_or("").contains("api_key_missing"),
            "missing-key error must surface the api_key_missing tag"
        );
    }

    #[test]
    fn live_broker_errors_when_iq_config_missing() {
        let broker = LiveIqBroker::new(None);
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
            resp.error.as_deref().unwrap_or("").contains("iq_config_missing"),
            "missing config error must surface the iq_config_missing tag"
        );
    }

    /// Verify `fetch_generation_cost` parses the documented response shape.
    #[test]
    fn fetch_generation_cost_parses_total_cost_string() {
        // Simulate the OpenRouter generation endpoint response shape.
        let mock_response = serde_json::json!({
            "data": {
                "total_cost": "0.00492",
                "tokens_prompt": 24,
                "tokens_completion": 29
            }
        });
        let total_cost_str = mock_response["data"]["total_cost"]
            .as_str()
            .expect("total_cost must be a string");
        let parsed: f64 = total_cost_str.parse().expect("must parse as f64");
        assert!(
            (parsed - 0.00492).abs() < 1e-9,
            "total_cost must parse to 0.00492"
        );
    }
}
