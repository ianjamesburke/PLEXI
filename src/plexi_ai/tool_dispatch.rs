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
//!
//! # Authorization model (#1182)
//!
//! Every registry entry records the `workspace_root` of the pane that exposed
//! the tools. When building a `ToolDispatcher`, the caller supplies its own
//! `workspace_root`. Only tools from panes in the **same workspace** are
//! included in the snapshot. Cross-workspace tools are invisible to the caller
//! and cannot be invoked — the model never sees them, so confused-deputy calls
//! fail before they can be attempted.
//!
//! Every dispatched call is logged at `info` level with both the caller
//! (app_id, pane_id) and provider (pane_id, tool name) so every invocation is
//! attributable in the audit trail.

use std::collections::HashMap;
use std::path::Path;
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
            let _ = self.tx.send(crate::process_app::StdinItem::Event(json));
        }
    }
}

// ── ToolRegistry ────────────────────────────────────────────────────────────

/// One registered app — its tool definitions, how to reach it, and the
/// workspace it belongs to (used for authorization checks).
struct RegistryEntry {
    tools: Vec<AiTool>,
    sender: AppEventSender,
    /// Workspace root of the pane that exposed these tools. Used to restrict
    /// cross-workspace tool invocation.
    workspace_root: std::path::PathBuf,
}

/// Global map from `pane_id` → `RegistryEntry`.
struct ToolRegistry {
    entries: HashMap<u64, RegistryEntry>,
}

impl ToolRegistry {
    fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    fn register(
        &mut self,
        pane_id: u64,
        tools: Vec<AiTool>,
        sender: AppEventSender,
        workspace_root: std::path::PathBuf,
    ) {
        self.entries.insert(
            pane_id,
            RegistryEntry {
                tools,
                sender,
                workspace_root,
            },
        );
    }

    fn unregister(&mut self, pane_id: u64) {
        self.entries.remove(&pane_id);
    }

    /// Snapshot of tools visible to a caller in `caller_workspace`, keyed by
    /// tool name. Only panes in the same workspace are included.
    ///
    /// Duplicate names (within the same workspace) are resolved
    /// deterministically: highest pane_id wins. This avoids hash-iteration
    /// nondeterminism when multiple panes expose the same tool name.
    fn snapshot_for_caller(&self, caller_workspace: &Path) -> HashMap<String, (u64, AiTool)> {
        let mut map = HashMap::new();
        // Use component-by-component comparison to avoid false mismatches from
        // trailing separators or platform path normalization differences.
        let mut pane_ids: Vec<u64> = self
            .entries
            .iter()
            .filter(|(_, e)| {
                e.workspace_root
                    .components()
                    .eq(caller_workspace.components())
            })
            .map(|(&id, _)| id)
            .collect();
        pane_ids.sort_unstable();

        let mut owners_by_name: HashMap<String, Vec<u64>> = HashMap::new();
        for &pane_id in &pane_ids {
            let Some(entry) = self.entries.get(&pane_id) else {
                continue;
            };
            for tool in &entry.tools {
                owners_by_name
                    .entry(tool.name.clone())
                    .or_default()
                    .push(pane_id);
                map.insert(tool.name.clone(), (pane_id, tool.clone()));
            }
        }
        for (name, mut owners) in owners_by_name {
            if owners.len() > 1 {
                owners.sort_unstable();
                log::warn!(
                    "tool_dispatch: duplicate tool name {:?} exposed by panes {:?}; dispatch will target highest pane_id",
                    name,
                    owners
                );
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
pub(crate) fn register(
    pane_id: u64,
    tools: Vec<AiTool>,
    sender: AppEventSender,
    workspace_root: std::path::PathBuf,
) {
    let count = tools.len();
    global_registry()
        .lock()
        .unwrap()
        .register(pane_id, tools, sender, workspace_root);
    log::info!("tool_dispatch: registered {count} tool(s) for pane {pane_id}");
}

/// Remove all tools for `pane_id`. Called when a pane is closed.
pub(crate) fn unregister(pane_id: u64) {
    global_registry().lock().unwrap().unregister(pane_id);
}

/// True when `pane_id` has a registry entry (it exposed tools and is still
/// open) — i.e. host-routed events can reach it.
pub(crate) fn pane_reachable(pane_id: u64) -> bool {
    global_registry().lock().unwrap().sender_for(pane_id).is_some()
}

/// Send a `PlexiEvent` to a registered pane's stdin channel (Phase C: the
/// host agent runtime delivers cross-pane `RollbackVerify` this way).
/// Returns false when the pane has no registry entry (closed, or it never
/// exposed tools).
pub(crate) fn send_event_to_pane(pane_id: u64, event: &PlexiEvent) -> bool {
    let registry = global_registry().lock().unwrap();
    match registry.sender_for(pane_id) {
        Some(sender) => {
            log::info!("tool_dispatch: host-routed event to pane {pane_id}");
            sender.send_event(event);
            true
        }
        None => {
            log::warn!("tool_dispatch: no registered sender for pane {pane_id} — event dropped");
            false
        }
    }
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
static PENDING_CALLS: OnceLock<
    Arc<Mutex<HashMap<String, std::sync::mpsc::SyncSender<ToolCallResult>>>>,
> = OnceLock::new();

fn pending_calls(
) -> &'static Arc<Mutex<HashMap<String, std::sync::mpsc::SyncSender<ToolCallResult>>>> {
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
///
/// Only contains tools from panes in the same workspace as the caller — cross-
/// workspace tools are excluded at construction time and never visible to the
/// dispatching app or the model it drives.
#[derive(Debug)]
pub struct ToolDispatcher {
    /// Snapshot: tool_name → (provider_pane_id, AiTool).
    /// Already filtered to the caller's workspace.
    tools: HashMap<String, (u64, AiTool)>,
    /// Caller identity for audit logging.
    caller_app_id: String,
    /// Caller pane id for audit logging.
    caller_pane_id: u64,
}

impl ToolDispatcher {
    /// Build a dispatcher scoped to `caller_workspace`. Only tools from panes
    /// in that workspace are included in the snapshot.
    pub fn from_registry(
        caller_pane_id: u64,
        caller_app_id: String,
        caller_workspace: std::path::PathBuf,
    ) -> Self {
        let registry = global_registry().lock().unwrap();
        let tools = registry.snapshot_for_caller(&caller_workspace);
        let visible: Vec<&str> = tools.keys().map(|s| s.as_str()).collect();
        log::info!(
            "tool_dispatch: dispatcher for caller={caller_app_id} pane={caller_pane_id} workspace={} — {} tool(s) visible: {visible:?}",
            caller_workspace.display(),
            tools.len(),
        );
        Self {
            tools,
            caller_app_id,
            caller_pane_id,
        }
    }

    /// All tools visible at snapshot time, for injection into the LLM request.
    pub fn all_tools(&self) -> Vec<AiTool> {
        self.tools.values().map(|(_, t)| t.clone()).collect()
    }

    /// Restrict the snapshot to `allowed` tool names (Phase C: the agent
    /// runtime applies broker `app_connector` decisions here). Removed tools
    /// are invisible to the model and `dispatch_call` returns
    /// `tool_not_found` for them — gating both visibility and invocation.
    pub fn retain_allowed(&mut self, allowed: &std::collections::HashSet<String>) {
        let before: Vec<String> = self.tools.keys().cloned().collect();
        self.tools.retain(|name, _| allowed.contains(name));
        for name in before {
            if !self.tools.contains_key(&name) {
                log::info!(
                    "tool_dispatch: caller={} pane={} — tool '{name}' removed by broker gate",
                    self.caller_app_id,
                    self.caller_pane_id
                );
            }
        }
    }

    /// Dispatch a single tool call. Blocks until the app responds or times out.
    pub fn dispatch_call(&self, call_id: String, name: &str, input_json: String) -> ToolCallResult {
        let (pane_id, tool) = match self.tools.get(name) {
            Some(entry) => entry,
            None => {
                log::warn!(
                    "tool_dispatch: caller={} pane={} requested unknown tool {name:?} call_id={call_id:?}",
                    self.caller_app_id, self.caller_pane_id,
                );
                return ToolCallResult {
                    output_json: None,
                    error: Some(format!(
                        "tool_not_found: no tool named {name:?} in registry"
                    )),
                };
            }
        };

        let timeout_ms = tool.timeout_ms.unwrap_or(30_000);

        log::info!(
            "tool_dispatch: caller={} caller_pane={} → tool={name:?} provider_pane={pane_id} call_id={call_id:?}",
            self.caller_app_id, self.caller_pane_id,
        );

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
            log::warn!(
                "tool_dispatch: provider pane {pane_id} gone for tool {name:?} call_id={call_id:?}"
            );
            return ToolCallResult {
                output_json: None,
                error: Some(format!(
                    "tool_pane_gone: pane {pane_id} for tool {name:?} is no longer registered"
                )),
            };
        }

        // Block until the app sends ToolResult or the timeout fires.
        match result_rx.recv_timeout(Duration::from_millis(timeout_ms)) {
            Ok(result) => {
                log::info!(
                    "tool_dispatch: tool={name:?} call_id={call_id:?} result={}",
                    if result.error.is_none() {
                        "ok"
                    } else {
                        "error"
                    }
                );
                result
            }
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

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_protocol::AiTool;
    use std::path::PathBuf;

    fn make_tool(name: &str) -> AiTool {
        AiTool {
            name: name.to_string(),
            description: format!("test tool {name}"),
            input_schema: serde_json::json!({"type": "object", "properties": {}}),
            timeout_ms: Some(100),
        }
    }

    // ── ToolRegistry unit tests (private access via same-module #[cfg(test)]) ──

    #[test]
    fn same_workspace_tools_are_visible() {
        let mut reg = ToolRegistry::new();
        let ws = PathBuf::from("/workspace/a");
        let (tx, _rx) = std::sync::mpsc::channel();
        reg.register(
            1,
            vec![make_tool("search")],
            AppEventSender { tx: tx.clone() },
            ws.clone(),
        );

        let snap = reg.snapshot_for_caller(&ws);
        assert!(
            snap.contains_key("search"),
            "same-workspace tool must be visible"
        );
        assert_eq!(snap["search"].0, 1, "provider pane id must be 1");
    }

    #[test]
    fn cross_workspace_tools_are_hidden() {
        let mut reg = ToolRegistry::new();
        let (tx, _rx) = std::sync::mpsc::channel();
        reg.register(
            10,
            vec![make_tool("dangerous_tool")],
            AppEventSender { tx: tx.clone() },
            PathBuf::from("/workspace/attacker"),
        );
        reg.register(
            20,
            vec![make_tool("safe_tool")],
            AppEventSender { tx: tx.clone() },
            PathBuf::from("/workspace/victim"),
        );

        // Snapshot from attacker workspace must NOT see victim's tools.
        let snap_attacker = reg.snapshot_for_caller(Path::new("/workspace/attacker"));
        assert!(snap_attacker.contains_key("dangerous_tool"));
        assert!(
            !snap_attacker.contains_key("safe_tool"),
            "cross-workspace tool must be hidden from attacker"
        );

        // Snapshot from victim workspace must NOT see attacker's tools.
        let snap_victim = reg.snapshot_for_caller(Path::new("/workspace/victim"));
        assert!(snap_victim.contains_key("safe_tool"));
        assert!(
            !snap_victim.contains_key("dangerous_tool"),
            "cross-workspace tool must be hidden from victim"
        );
    }

    #[test]
    fn unregistered_pane_tools_disappear() {
        let mut reg = ToolRegistry::new();
        let ws = PathBuf::from("/workspace/x");
        let (tx, _rx) = std::sync::mpsc::channel();
        reg.register(
            5,
            vec![make_tool("tool_a")],
            AppEventSender { tx },
            ws.clone(),
        );

        assert!(reg.snapshot_for_caller(&ws).contains_key("tool_a"));
        reg.unregister(5);
        assert!(
            reg.snapshot_for_caller(&ws).is_empty(),
            "tools must disappear after pane unregisters"
        );
    }

    #[test]
    fn empty_workspace_snapshot_for_unknown_workspace() {
        let reg = ToolRegistry::new();
        let snap = reg.snapshot_for_caller(Path::new("/workspace/unknown"));
        assert!(snap.is_empty());
    }

    // ── ToolDispatcher: unauthorized call returns tool_not_found ──────────────
    //
    // The authorization boundary is: cross-workspace tools are excluded from the
    // snapshot, so `dispatch_call` returns `tool_not_found` for them — the model
    // never learns they exist.

    #[test]
    fn dispatcher_excludes_cross_workspace_tools() {
        // Register a tool in workspace B.
        let (tx_b, _rx_b) = std::sync::mpsc::channel();
        register(
            999,
            vec![make_tool("secret_tool")],
            AppEventSender { tx: tx_b },
            PathBuf::from("/workspace/b"),
        );

        // Build a dispatcher for a caller in workspace A.
        let dispatcher =
            ToolDispatcher::from_registry(1, "app_a".to_string(), PathBuf::from("/workspace/a"));

        // The tool must not appear in the visible set.
        let visible: Vec<String> = dispatcher.all_tools().into_iter().map(|t| t.name).collect();
        assert!(
            !visible.contains(&"secret_tool".to_string()),
            "cross-workspace tool must not appear in dispatcher: {visible:?}"
        );

        // A direct dispatch attempt must return tool_not_found.
        let result =
            dispatcher.dispatch_call("call-x".to_string(), "secret_tool", "{}".to_string());
        assert!(result.error.is_some());
        assert!(
            result
                .error
                .as_deref()
                .unwrap_or("")
                .contains("tool_not_found"),
            "unauthorized call must return tool_not_found: {:?}",
            result.error
        );

        // Clean up global registry.
        unregister(999);
    }
}
