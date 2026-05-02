//! Global tool registry and dispatcher for the v3.7 tool protocol (#399).
//!
//! The tool registry is a singleton shared across all `ProcessApp` instances.
//! When an app sends `DrawCommand::ExposeTools`, its pane registers tool
//! definitions + an `AppEventSender` here. When the broker wants to call a
//! tool, it creates a `ToolDispatcher` snapshot, then calls `dispatch_call`
//! which:
//!   1. Looks up the owning pane's `AppEventSender`.
//!   2. Sends `PlexiEvent::ToolCall { call_id, name, input_json }` to that pane.
//!   3. Blocks (up to `timeout_ms`) on a `SyncReceiver` for the result.
//!   4. Returns `ToolCallResult { output_json, error }`.
//!
//! Pending-call state lives in `PENDING_CALLS`. `ProcessApp::routing` feeds
//! `DrawCommand::ToolResult` back here to unblock the waiting broker thread.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use crate::app_protocol::{AiTool, PlexiEvent};

// ── AppEventSender ──────────────────────────────────────────────────────────

/// Thin wrapper that lets external code send `PlexiEvent`s into a pane's
/// stdin channel without exposing the `StdinItem` enum publicly.
pub(crate) struct AppEventSender {
    pub(crate) tx: std::sync::mpsc::Sender<crate::process_app::StdinItem>,
}

impl AppEventSender {
    pub(crate) fn send_event(&self, event: &PlexiEvent) {
        if let Ok(mut json) = serde_json::to_string(event) {
            json.push('\n');
            let _ = self
                .tx
                .send(crate::process_app::StdinItem::Event(json));
        }
    }
}

// ── ToolRegistry ────────────────────────────────────────────────────────────

/// One registered app — its tool definitions and how to reach it.
struct RegistryEntry {
    tools: Vec<AiTool>,
    sender: AppEventSender,
}

/// Global map from `(pane_id)` → `RegistryEntry`.
struct ToolRegistry {
    entries: HashMap<u64, RegistryEntry>,
}

impl ToolRegistry {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    fn register(&mut self, pane_id: u64, tools: Vec<AiTool>, sender: AppEventSender) {
        self.entries.insert(pane_id, RegistryEntry { tools, sender });
    }

    fn unregister(&mut self, pane_id: u64) {
        self.entries.remove(&pane_id);
    }

    /// Snapshot of all tools visible right now, keyed by tool name.
    /// When the same tool name appears in multiple panes, the last one wins
    /// (non-deterministic order — callers should ensure unique names).
    fn snapshot(&self) -> HashMap<String, (u64, AiTool)> {
        let mut map = HashMap::new();
        for (&pane_id, entry) in &self.entries {
            for tool in &entry.tools {
                map.insert(tool.name.clone(), (pane_id, tool.clone()));
            }
        }
        map
    }

    /// Get the `AppEventSender` for a pane without moving it.
    fn sender_for(&self, pane_id: u64) -> Option<&AppEventSender> {
        self.entries.get(&pane_id).map(|e| &e.sender)
    }
}

static GLOBAL_REGISTRY: OnceLock<Arc<Mutex<ToolRegistry>>> = OnceLock::new();

fn global_registry() -> &'static Arc<Mutex<ToolRegistry>> {
    GLOBAL_REGISTRY.get_or_init(|| Arc::new(Mutex::new(ToolRegistry::new())))
}

/// Register (or replace) the tools for `pane_id`. Called by routing when
/// `DrawCommand::ExposeTools` arrives.
pub(crate) fn register(pane_id: u64, tools: Vec<AiTool>, sender: AppEventSender) {
    let count = tools.len();
    global_registry()
        .lock()
        .unwrap()
        .register(pane_id, tools, sender);
    log::debug!("tool_dispatch: registered {count} tool(s) for pane {pane_id}");
}

/// Remove all tools for `pane_id`. Called when a pane is closed.
pub(crate) fn unregister(pane_id: u64) {
    global_registry().lock().unwrap().unregister(pane_id);
}

// ── Pending calls ────────────────────────────────────────────────────────────

/// Result returned to the broker by a completed tool call.
#[derive(Debug, Clone)]
pub struct ToolCallResult {
    pub output_json: Option<String>,
    pub error: Option<String>,
}

/// Map from `call_id` → sender that will receive the `ToolCallResult` once
/// `DrawCommand::ToolResult` arrives.
static PENDING_CALLS: OnceLock<Arc<Mutex<HashMap<String, std::sync::mpsc::SyncSender<ToolCallResult>>>>> =
    OnceLock::new();

fn pending_calls() -> &'static Arc<Mutex<HashMap<String, std::sync::mpsc::SyncSender<ToolCallResult>>>> {
    PENDING_CALLS.get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
}

/// Called by `routing.rs` when `DrawCommand::ToolResult` arrives for a pane.
/// Resolves the pending `call_id` so the blocking broker thread can continue.
pub(crate) fn resolve_pending(call_id: &str, result: ToolCallResult) {
    let tx = pending_calls().lock().unwrap().remove(call_id);
    match tx {
        Some(t) => {
            let _ = t.send(result);
        }
        None => {
            log::warn!("tool_dispatch: ToolResult for unknown call_id={call_id:?} — dropped");
        }
    }
}

// ── ToolDispatcher ──────────────────────────────────────────────────────────

/// Snapshot of the tool registry for one broker invocation. Created by the
/// routing layer before spawning the broker thread. Passed into the broker
/// via `AiBrokerRequest::tool_dispatcher`.
#[derive(Debug)]
pub struct ToolDispatcher {
    /// Snapshot: tool_name → (pane_id, AiTool)
    tools: HashMap<String, (u64, AiTool)>,
}

impl ToolDispatcher {
    /// Build a dispatcher from the current global registry state.
    pub fn from_registry() -> Self {
        let tools = global_registry().lock().unwrap().snapshot();
        Self { tools }
    }

    /// All tools visible at snapshot time, for injection into the LLM request.
    pub fn all_tools(&self) -> Vec<AiTool> {
        self.tools.values().map(|(_, t)| t.clone()).collect()
    }

    /// Dispatch a single tool call. Blocks until the app responds or times out.
    pub fn dispatch_call(
        &self,
        call_id: String,
        name: &str,
        input_json: String,
    ) -> ToolCallResult {
        let (pane_id, tool) = match self.tools.get(name) {
            Some(entry) => entry,
            None => {
                return ToolCallResult {
                    output_json: None,
                    error: Some(format!("tool_not_found: no tool named {name:?} in registry")),
                };
            }
        };

        let timeout_ms = tool.timeout_ms.unwrap_or(30_000);

        // Register the pending call before sending the event to avoid a race.
        let (result_tx, result_rx) = std::sync::mpsc::sync_channel::<ToolCallResult>(1);
        pending_calls()
            .lock()
            .unwrap()
            .insert(call_id.clone(), result_tx);

        // Send ToolCall to the owning pane.
        let sent = {
            let registry = global_registry().lock().unwrap();
            if let Some(sender) = registry.sender_for(*pane_id) {
                sender.send_event(&PlexiEvent::ToolCall {
                    call_id: call_id.clone(),
                    name: name.to_string(),
                    input_json,
                });
                true
            } else {
                false
            }
        };

        if !sent {
            // Pane went away after we built the snapshot — clean up and return error.
            pending_calls().lock().unwrap().remove(&call_id);
            return ToolCallResult {
                output_json: None,
                error: Some(format!(
                    "tool_pane_gone: pane {pane_id} for tool {name:?} is no longer registered"
                )),
            };
        }

        // Block until the app sends ToolResult or the timeout fires.
        match result_rx.recv_timeout(Duration::from_millis(timeout_ms)) {
            Ok(result) => result,
            Err(_) => {
                // Clean up the stale pending entry.
                pending_calls().lock().unwrap().remove(&call_id);
                log::warn!(
                    "tool_dispatch: tool {name:?} call_id={call_id:?} timed out after {timeout_ms}ms"
                );
                ToolCallResult {
                    output_json: None,
                    error: Some(format!(
                        "tool_timeout: tool {name:?} did not respond within {timeout_ms}ms"
                    )),
                }
            }
        }
    }
}
