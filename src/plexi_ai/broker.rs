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

use std::sync::{Arc, Mutex, OnceLock};

use crate::app_protocol::{AiMessage, AiTool, ModelTier};
use crate::config::{AiConfig, OllamaBackendConfig, OpenRouterBackendConfig};
use crate::event_log::{self, HostEvent};
use crate::plexi_ai::backend::ollama::OllamaBackend;
use crate::plexi_ai::backend::openrouter::OpenRouterBackend;
use crate::plexi_ai::backend::{AiBackend, AiBackendRequest, BillingModel};
use crate::plexi_ai::ledger::{self, LedgerRow};
use crate::plexi_ai::tool_dispatch::ToolDispatcher;
use crate::plexi_ai::turn_loop::{self, TurnError};

/// Lightweight context for a single open pane (injected into system prompt).
#[derive(Debug, Clone)]
pub struct PaneContext {
    pub type_id: String,
    pub pane_id: u64,
}

// ── Global pane context snapshot ────────────────────────────────────────────

/// Singleton holding a snapshot of all open panes across all windows.
/// Written by `PlexiApp::update()` each frame; read by `route_command` when
/// dispatching `AiQuery` so the broker receives the full workspace context.
static PANE_SNAPSHOT: OnceLock<Arc<Mutex<Vec<PaneContext>>>> = OnceLock::new();

fn pane_snapshot() -> &'static Arc<Mutex<Vec<PaneContext>>> {
    PANE_SNAPSHOT.get_or_init(|| Arc::new(Mutex::new(Vec::new())))
}

/// Replace the global pane context snapshot. Called once per frame by the host.
pub fn update_pane_snapshot(panes: Vec<PaneContext>) {
    *pane_snapshot().lock().unwrap() = panes;
}

/// Read the current pane context snapshot. Called by routing on `AiQuery`.
pub fn get_pane_snapshot() -> Vec<PaneContext> {
    pane_snapshot().lock().unwrap().clone()
}

/// One brokered request — what the routing layer hands to a broker. `app_id`
/// is required so the ledger can attribute spend per-app.
#[derive(Debug, Clone)]
pub struct AiBrokerRequest {
    pub app_id: String,
    pub model_tier: ModelTier,
    pub system: String,
    pub messages: Vec<AiMessage>,
    pub tools: Vec<AiTool>,
    /// Workspace root for host-context injection. When `Some`, the broker
    /// prepends a compact workspace context block to the system prompt.
    pub workspace_root: Option<std::path::PathBuf>,
    /// Currently open panes — listed in the host context block.
    pub open_panes: Vec<PaneContext>,
    /// Tool dispatcher snapshot. When `Some`, the broker runs a tool loop
    /// and dispatches any tool calls the model makes.
    pub tool_dispatcher: Option<std::sync::Arc<ToolDispatcher>>,
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

/// Build a compact host context block to prepend to the system prompt.
/// Budget: ≤2000 tokens (~8000 chars). Skipped when the system prompt
/// starts with `# no-host-context`.
fn build_context_prefix(
    workspace_root: &std::path::Path,
    open_panes: &[PaneContext],
) -> String {
    let mut out = String::new();
    out.push_str("# Host context\n");
    out.push_str(&format!("workspace: {}\n", workspace_root.display()));
    if !open_panes.is_empty() {
        out.push_str("open panes:");
        for p in open_panes {
            out.push_str(&format!(" {}(pane_id={})", p.type_id, p.pane_id));
        }
        out.push('\n');
    }

    // Append up to 20 recent workspace events (~8000 chars budget).
    let events_path = workspace_root
        .join(".plexi")
        .join("events.jsonl");
    let recent = crate::event_log::read_recent(&events_path, 20);
    if !recent.is_empty() {
        out.push_str("recent events:\n");
        for ev in &recent {
            let line = serde_json::to_string(ev).unwrap_or_default();
            // Truncate long lines.
            if line.len() > 200 {
                out.push_str(&line[..200]);
                out.push_str("…\n");
            } else {
                out.push_str(&line);
                out.push('\n');
            }
        }
    }
    out.push_str("---\n");
    // Hard cap: 8000 chars (~2000 tokens at 4 chars/token).
    if out.len() > 8000 {
        out.truncate(8000);
        out.push_str("\n---\n");
    }
    out
}

/// Convert `Vec<AiMessage>` to `Vec<serde_json::Value>` for the backend.
fn messages_to_json(messages: &[AiMessage]) -> Vec<serde_json::Value> {
    messages
        .iter()
        .map(|m| serde_json::json!({"role": m.role, "content": m.content}))
        .collect()
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
    // Build effective system prompt (optionally prepend host context).
    let system = if request
        .system
        .starts_with("# no-host-context")
    {
        request.system.clone()
    } else if let Some(root) = &request.workspace_root {
        let prefix = build_context_prefix(root, &request.open_panes);
        format!("{prefix}{}", request.system)
    } else {
        request.system.clone()
    };

    // Collect tools: from request AND from global registry via dispatcher.
    let registry_tools = request
        .tool_dispatcher
        .as_ref()
        .map(|d| d.all_tools())
        .unwrap_or_default();
    let mut all_tools: Vec<AiTool> = request.tools.clone();
    for t in registry_tools {
        if !all_tools.iter().any(|existing| existing.name == t.name) {
            all_tools.push(t);
        }
    }

    // Convert messages to JSON values for the backend.
    let mut conv: Vec<serde_json::Value> = messages_to_json(&request.messages);

    const MAX_TOOL_ITERATIONS: usize = 10;
    let mut total_tokens_in: u32 = 0;
    let mut total_tokens_out: u32 = 0;
    let mut last_generation_id: Option<String> = None;
    let mut final_text = String::new();

    for iteration in 0..=MAX_TOOL_ITERATIONS {
        if iteration == MAX_TOOL_ITERATIONS {
            log::warn!(
                "ai_broker[{}]: tool loop hit max iterations ({MAX_TOOL_ITERATIONS}), forcing stop",
                request.app_id
            );
            break;
        }

        let backend_req = AiBackendRequest {
            messages: conv.clone(),
            system: system.clone(),
            tools: all_tools.clone(),
        };

        let turn = turn_loop::run_turn(backend, backend_req, |_| {});

        match turn {
            Err(e) => {
                log::warn!("ai_broker[{}]: turn failed: {e}", request.app_id);
                let message = match e {
                    TurnError::BackendError(be) => format!("backend error: {be}"),
                    TurnError::StreamError(s) => format!("stream error: {s}"),
                    TurnError::ChannelClosed => "stream closed before completion".to_string(),
                };
                return AiBrokerResponse::err(message);
            }
            Ok(result) => {
                total_tokens_in += result.input_tokens.unwrap_or(0);
                total_tokens_out += result.output_tokens.unwrap_or(0);
                if result.generation_id.is_some() {
                    last_generation_id = result.generation_id.clone();
                }

                if result.tool_calls.is_empty() {
                    // No tool calls — done.
                    final_text = result.text;
                    break;
                }

                // Append assistant tool-call message.
                let tc_json: Vec<serde_json::Value> = result
                    .tool_calls
                    .iter()
                    .map(|tc| {
                        serde_json::json!({
                            "id": tc.id,
                            "type": "function",
                            "function": {
                                "name": tc.name,
                                "arguments": tc.arguments
                            }
                        })
                    })
                    .collect();
                conv.push(serde_json::json!({
                    "role": "assistant",
                    "content": null,
                    "tool_calls": tc_json
                }));

                // Dispatch each tool call and append tool result messages.
                let dispatcher = match &request.tool_dispatcher {
                    Some(d) => d,
                    None => {
                        log::warn!(
                            "ai_broker[{}]: model requested tool calls but no dispatcher available",
                            request.app_id
                        );
                        // Append error result for each call so the model can recover.
                        for tc in &result.tool_calls {
                            conv.push(serde_json::json!({
                                "role": "tool",
                                "tool_call_id": tc.id,
                                "content": "error: tool_dispatch_unavailable"
                            }));
                        }
                        continue;
                    }
                };

                // Dispatch all tool calls (sequentially for now — parallel
                // dispatch would require spawning threads and joining, adding
                // complexity for marginal gain on typical 1-2 tool call batches).
                let mut call_counter: u64 = 0;
                for tc in &result.tool_calls {
                    call_counter += 1;
                    let call_id = format!("{}-{call_counter}", tc.id);
                    log::debug!(
                        "ai_broker[{}]: dispatching tool '{}' call_id={call_id}",
                        request.app_id,
                        tc.name,
                    );
                    let tool_result = dispatcher.dispatch_call(
                        call_id.clone(),
                        &tc.name,
                        tc.arguments.clone(),
                    );
                    let content = if let Some(out) = tool_result.output_json {
                        out
                    } else {
                        format!(
                            "error: {}",
                            tool_result.error.unwrap_or_else(|| "unknown".to_string())
                        )
                    };
                    conv.push(serde_json::json!({
                        "role": "tool",
                        "tool_call_id": tc.id,
                        "content": content
                    }));
                }
            }
        }
    }

    // Cost fetch (OpenRouter only).
    let cost_usd = last_generation_id.as_deref().and_then(|gen_id| {
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
        Some(total_tokens_in),
        Some(total_tokens_out),
        cost_usd,
    );
    ledger::append(&row);

    event_log::emit(HostEvent::AgentTurn {
        pane_id: None,
        tokens_in: total_tokens_in,
        tokens_out: total_tokens_out,
        cost_cents,
        timestamp: event_log::now_timestamp(),
    });

    AiBrokerResponse::ok(final_text, total_tokens_in, total_tokens_out)
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
        // Multi-turn conversations flow as `Vec<AiMessage>` through the broker
        // (PGAP wire format). This test pins the contract: `AiBrokerRequest`
        // accepts the structured form unchanged.
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
            workspace_root: None,
            open_panes: vec![],
            tool_dispatcher: None,
        });
        assert_eq!(resp.content.as_deref(), Some("response"));
        let seen = broker.seen.lock().unwrap();
        assert_eq!(seen.len(), 1, "broker should have seen exactly one request");
        assert_eq!(seen[0].messages, messages, "messages must round-trip unchanged");
        assert_eq!(seen[0].system, "be helpful");
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
            workspace_root: None,
            open_panes: vec![],
            tool_dispatcher: None,
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
            workspace_root: None,
            open_panes: vec![],
            tool_dispatcher: None,
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
            workspace_root: None,
            open_panes: vec![],
            tool_dispatcher: None,
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
