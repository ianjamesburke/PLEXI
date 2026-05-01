//! `ai.query` capability broker — issue #284, #383.
//!
//! The broker is the host-side bridge between an app's `DrawCommand::AiQuery`
//! and the live Plexi AI backend. The capability gate, model-tier resolution,
//! ledger append, and outbound `PlexiEvent::AiResponse` are all funnelled
//! through this module so:
//!
//!   1. Tests can replace `LiveAiBroker` with a canned `AiBroker` mock and
//!      drive `process_app::routing` deterministically (no real LLM).
//!   2. v3.4 (issue #285, "agent-as-app") can reuse the same broker — the
//!      hardcoded `Pane::Agent` turn loop becomes a degenerate case where
//!      the in-process agent calls into `LiveAiBroker` exactly the same way
//!      a subprocess agent does over PGAP.
//!
//! Today the broker drives the synchronous `turn_loop::run_turn` path (which
//! itself spawns a worker thread inside the backend) so we never hit the UI
//! thread with a blocking call. The broker's `dispatch` method is therefore
//! expected to be invoked from a worker thread spawned by the routing layer.

use crate::app_protocol::{AiMessage, AiTool, ModelTier};
use crate::config::{AiConfig, OllamaBackendConfig, OpenRouterBackendConfig};
use crate::event_log::{self, HostEvent};
use crate::plexi_ai::backend::ollama::OllamaBackend;
use crate::plexi_ai::backend::openrouter::OpenRouterBackend;
use crate::plexi_ai::backend::{BillingModel, AiBackend};
use crate::plexi_ai::ledger::{self, LedgerRow};
use crate::plexi_ai::turn_loop::{self, TurnError};

/// One brokered request — what the routing layer hands to a broker. `app_id`
/// is required so the ledger can attribute spend per-app.
#[derive(Debug, Clone)]
pub struct AiBrokerRequest {
    pub app_id: String,
    pub model_tier: ModelTier,
    pub system: String,
    pub messages: Vec<AiMessage>,
    pub tools: Vec<AiTool>,
}

/// Broker outcome. Either `content` is `Some` (success) or `error` is `Some`
/// (failure). Token counts default to `0` when the upstream backend doesn't
/// report them (e.g. subscription billing).
#[derive(Debug, Clone)]
pub struct AiBrokerResponse {
    pub content: Option<String>,
    pub tokens_in: u32,
    pub tokens_out: u32,
    pub error: Option<String>,
}

impl AiBrokerResponse {
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
pub trait AiBroker: Send + Sync {
    /// Synchronously dispatch a brokered query. Implementations should not
    /// block the caller forever — the routing layer always invokes this from
    /// a dedicated worker thread, but slow networks and stalled backends can
    /// still tie that thread up. Implementations should bound their own waits.
    fn dispatch(&self, request: AiBrokerRequest) -> AiBrokerResponse;
}

/// Production broker: reads config from `AiConfig`, routes to the configured
/// backend (OpenRouter or Ollama), fetches real per-call cost (OpenRouter only),
/// writes the ledger, and emits `HostEvent::AgentTurn`.
///
/// `AiConfig` is cloned at construction from `PlexiConfig::load().ai`.
/// If the config section is absent, `dispatch` fails fast with a clear error.
pub struct LiveAiBroker {
    /// AI broker configuration loaded from `config.toml [ai]`.
    /// `None` when the section is absent; `dispatch` fails fast in that case.
    ai_config: Option<AiConfig>,
}

impl LiveAiBroker {
    /// Construct from the loaded `AiConfig` (may be `None` when `[ai]` is
    /// absent from config.toml — errors are surfaced at dispatch time).
    pub fn new(ai_config: Option<AiConfig>) -> Self {
        Self { ai_config }
    }
}

impl AiBroker for LiveAiBroker {
    fn dispatch(&self, request: AiBrokerRequest) -> AiBrokerResponse {
        // v3.3: text-in / text-out only. Tool dispatch lands in v3.4.
        if !request.tools.is_empty() {
            let msg = "tools not yet supported by ai.query broker (v3.4)";
            log::warn!("ai_broker[{}]: dispatch failed — {}", request.app_id, msg);
            return AiBrokerResponse::err(msg);
        }

        let ai_config = match &self.ai_config {
            Some(c) => c,
            None => {
                let msg = "ai_config_missing: add [ai] section with model_low, model_medium, model_high to config.toml";
                log::warn!("ai_broker[{}]: dispatch failed — {}", request.app_id, msg);
                return AiBrokerResponse::err(msg);
            }
        };

        let backend_name = ai_config.backend.as_deref().unwrap_or("openrouter");

        match backend_name {
            "ollama" => dispatch_ollama(request, ai_config),
            _ => dispatch_openrouter(request, ai_config),
        }
    }
}

/// Dispatch through the OpenRouter backend.
fn dispatch_openrouter(request: AiBrokerRequest, ai_config: &AiConfig) -> AiBrokerResponse {
    let or_config = match ai_config.openrouter.as_ref() {
        Some(c) => c,
        None => {
            let msg = "ai_config_missing: [ai.openrouter] section required for openrouter backend";
            log::warn!("ai_broker[{}]: dispatch failed — {}", request.app_id, msg);
            return AiBrokerResponse::err(msg);
        }
    };

    let model_id = resolve_model_tier(&request.model_tier, or_config);
    let model_id = match model_id {
        Some(m) => m,
        None => {
            let msg = format!(
                "ai_config_missing: model_{} not set in [ai.openrouter] config section",
                tier_name(&request.model_tier)
            );
            log::warn!("ai_broker[{}]: dispatch failed — {}", request.app_id, msg);
            return AiBrokerResponse::err(msg);
        }
    };

    let api_key_env = or_config.api_key_env.as_deref().unwrap_or("OPENROUTER_API_KEY");
    let api_key = match std::env::var(api_key_env) {
        Ok(k) if !k.is_empty() => k,
        _ => {
            log::warn!(
                "ai_broker[{}]: {} not set — denying ai.query",
                request.app_id,
                api_key_env
            );
            return AiBrokerResponse::err(format!(
                "api_key_missing: {api_key_env} not set — export it in your shell profile"
            ));
        }
    };

    let backend = OpenRouterBackend {
        api_key: api_key.clone(),
        model: model_id.clone(),
    };

    run_turn_and_respond(request, &backend, BillingModel::Metered, model_id, api_key)
}

/// Dispatch through the Ollama backend.
fn dispatch_ollama(request: AiBrokerRequest, ai_config: &AiConfig) -> AiBrokerResponse {
    let ollama_config = match ai_config.ollama.as_ref() {
        Some(c) => c,
        None => {
            let msg = "Ollama backend selected but [ai.ollama] config section is missing";
            log::warn!("ai_broker[{}]: dispatch failed — {}", request.app_id, msg);
            return AiBrokerResponse::err(msg);
        }
    };

    let model_id = resolve_ollama_model_tier(&request.model_tier, ollama_config);
    let model_id = match model_id {
        Some(m) => m,
        None => {
            let msg = format!(
                "ai_config_missing: model_{} not set in [ai.ollama] config section",
                tier_name(&request.model_tier)
            );
            log::warn!("ai_broker[{}]: dispatch failed — {}", request.app_id, msg);
            return AiBrokerResponse::err(msg);
        }
    };

    let host = ollama_config.host.as_deref().unwrap_or("http://localhost:11434").to_string();

    let backend = OllamaBackend {
        host,
        model: model_id.clone(),
    };

    run_turn_and_respond(request, &backend, BillingModel::Subscription, model_id, String::new())
}

/// Run a turn through the given backend and build an `AiBrokerResponse`.
/// The `api_key` is only needed for the cost-fetch step (OpenRouter); pass
/// an empty string for other backends.
fn run_turn_and_respond(
    request: AiBrokerRequest,
    backend: &dyn AiBackend,
    billing: BillingModel,
    model_id: String,
    api_key: String,
) -> AiBrokerResponse {
    let turn = turn_loop::run_turn(
        backend,
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
                if api_key.is_empty() {
                    None
                } else {
                    fetch_generation_cost(gen_id, &api_key)
                }
            });

            let cost_cents = cost_usd
                .map(|usd| (usd * 100.0).round().max(0.0) as u64)
                .unwrap_or(0);

            let row = LedgerRow::with_attribution(
                backend.name(),
                billing,
                Some(request.app_id.clone()),
                Some(model_id),
                result.input_tokens,
                result.output_tokens,
                cost_usd,
            );
            ledger::append(&row);

            event_log::emit(HostEvent::AgentTurn {
                pane_id: None,
                tokens_in,
                tokens_out,
                cost_cents,
                timestamp: event_log::now_timestamp(),
            });

            AiBrokerResponse::ok(result.text, tokens_in, tokens_out)
        }
        Err(e) => {
            log::warn!("ai_broker[{}]: turn failed: {e}", request.app_id);
            let message = match e {
                TurnError::BackendError(be) => format!("backend error: {be}"),
                TurnError::StreamError(s) => format!("stream error: {s}"),
                TurnError::ChannelClosed => "stream closed before completion".to_string(),
            };
            AiBrokerResponse::err(message)
        }
    }
}

fn tier_name(tier: &ModelTier) -> &'static str {
    match tier {
        ModelTier::Low => "low",
        ModelTier::Medium => "medium",
        ModelTier::High => "high",
    }
}

fn resolve_model_tier(tier: &ModelTier, config: &OpenRouterBackendConfig) -> Option<String> {
    match tier {
        ModelTier::Low => config.model_low.clone(),
        ModelTier::Medium => config.model_medium.clone(),
        ModelTier::High => config.model_high.clone(),
    }
}

fn resolve_ollama_model_tier(tier: &ModelTier, config: &OllamaBackendConfig) -> Option<String> {
    match tier {
        ModelTier::Low => config.model_low.clone(),
        ModelTier::Medium => config.model_medium.clone(),
        ModelTier::High => config.model_high.clone(),
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

    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(30))
        .build();

    let try_fetch = || -> Option<f64> {
        let resp = agent
            .get(&url)
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

    if let Some(cost) = try_fetch() {
        return Some(cost);
    }

    // One retry after 500ms for eventual consistency.
    std::thread::sleep(std::time::Duration::from_millis(500));
    let cost = try_fetch();
    if cost.is_none() {
        log::warn!(
            "ai_broker: generation cost unavailable for gen_id={gen_id} — ledger will show cost_usd=null"
        );
    }
    cost
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AiConfig, OpenRouterBackendConfig};
    use std::sync::{Arc, Mutex};

    /// Test broker: records every request and returns a canned response.
    pub struct CannedBroker {
        pub seen: Arc<Mutex<Vec<AiBrokerRequest>>>,
        pub response: AiBrokerResponse,
    }

    impl CannedBroker {
        pub fn ok(content: &str) -> Self {
            Self {
                seen: Arc::new(Mutex::new(Vec::new())),
                response: AiBrokerResponse::ok(content.to_string(), 7, 3),
            }
        }
    }

    impl AiBroker for CannedBroker {
        fn dispatch(&self, request: AiBrokerRequest) -> AiBrokerResponse {
            self.seen.lock().unwrap().push(request);
            self.response.clone()
        }
    }

    fn test_ai_config() -> AiConfig {
        AiConfig {
            backend: Some("openrouter".to_string()),
            openrouter: Some(OpenRouterBackendConfig {
                api_key_env: None,
                model_low: Some("google/gemini-2.0-flash-001".to_string()),
                model_medium: Some("anthropic/claude-sonnet-4-6".to_string()),
                model_high: Some("anthropic/claude-opus-4-7".to_string()),
            }),
            ollama: None,
        }
    }

    #[test]
    fn config_resolves_tiers_to_openrouter_model_ids() {
        let config = test_ai_config();
        let or_config = config.openrouter.as_ref().unwrap();
        assert_eq!(
            or_config.model_low.as_deref(),
            Some("google/gemini-2.0-flash-001")
        );
        assert_eq!(
            or_config.model_medium.as_deref(),
            Some("anthropic/claude-sonnet-4-6")
        );
        assert_eq!(
            or_config.model_high.as_deref(),
            Some("anthropic/claude-opus-4-7")
        );
    }

    #[test]
    fn broker_accepts_structured_messages_after_widening() {
        // Multi-turn conversations now flow as a structured `Vec<AiMessage>`
        // through the broker — no flattening into a single prompt. This test
        // pins the contract: the broker hands the full message array to the
        // backend and `AiBrokerRequest` accepts the structured form unchanged.
        let broker = CannedBroker::ok("response");
        let messages = vec![
            AiMessage {
                role: "user".to_string(),
                content: "first".to_string(),
            },
            AiMessage {
                role: "assistant".to_string(),
                content: "ok".to_string(),
            },
            AiMessage {
                role: "user".to_string(),
                content: "second".to_string(),
            },
        ];
        let resp = broker.dispatch(AiBrokerRequest {
            app_id: "test".to_string(),
            model_tier: ModelTier::Low,
            system: "be helpful".to_string(),
            messages: messages.clone(),
            tools: vec![],
        });
        assert_eq!(resp.content.as_deref(), Some("response"));
        let seen = broker.seen.lock().unwrap();
        assert_eq!(seen.len(), 1, "broker should have seen exactly one request");
        assert_eq!(seen[0].messages, messages, "messages must round-trip unchanged");
        assert_eq!(seen[0].system, "be helpful");
    }

    #[test]
    fn live_broker_rejects_tools_until_v3_4() {
        let broker = LiveAiBroker::new(Some(test_ai_config()));
        let resp = broker.dispatch(AiBrokerRequest {
            app_id: "test".to_string(),
            model_tier: ModelTier::Low,
            system: String::new(),
            messages: vec![],
            tools: vec![AiTool {
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
        let orig = std::env::var("OPENROUTER_API_KEY").ok();
        unsafe { std::env::remove_var("OPENROUTER_API_KEY") };

        let broker = LiveAiBroker::new(Some(test_ai_config()));
        let resp = broker.dispatch(AiBrokerRequest {
            app_id: "test".to_string(),
            model_tier: ModelTier::Low,
            system: String::new(),
            messages: vec![AiMessage {
                role: "user".to_string(),
                content: "hi".to_string(),
            }],
            tools: vec![],
        });

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
    fn live_broker_errors_when_ai_config_missing() {
        let broker = LiveAiBroker::new(None);
        let resp = broker.dispatch(AiBrokerRequest {
            app_id: "test".to_string(),
            model_tier: ModelTier::Low,
            system: String::new(),
            messages: vec![AiMessage {
                role: "user".to_string(),
                content: "hi".to_string(),
            }],
            tools: vec![],
        });
        assert!(resp.error.is_some());
        assert!(
            resp.error.as_deref().unwrap_or("").contains("ai_config_missing"),
            "missing config error must surface the ai_config_missing tag"
        );
    }

    #[test]
    fn live_broker_errors_when_ollama_selected_but_config_missing() {
        let broker = LiveAiBroker::new(Some(AiConfig {
            backend: Some("ollama".to_string()),
            openrouter: None,
            ollama: None,
        }));
        let resp = broker.dispatch(AiBrokerRequest {
            app_id: "test".to_string(),
            model_tier: ModelTier::Low,
            system: String::new(),
            messages: vec![AiMessage {
                role: "user".to_string(),
                content: "hi".to_string(),
            }],
            tools: vec![],
        });
        assert!(resp.error.is_some());
        assert!(
            resp.error.as_deref().unwrap_or("").contains("[ai.ollama]"),
            "error must mention missing [ai.ollama] section"
        );
    }

    /// Verify `fetch_generation_cost` parses the documented response shape.
    #[test]
    fn fetch_generation_cost_parses_total_cost_string() {
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
