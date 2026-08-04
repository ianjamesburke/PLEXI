//! Global tool registry and dispatcher for the v3.7 tool protocol (#399).
//!
//! The tool registry is a singleton shared across all `WASM app runtime` instances.
//! When an app sends `DrawCommand::ExposeTools`, its pane registers tool
//! definitions + an `AppEventSender` here. When the broker wants to call a
//! tool, it creates a `ToolDispatcher` snapshot, then calls `dispatch_call`
//! which:
//!   1. Looks up the owning pane's `AppEventSender`.
//!   2. Sends `PlexiEvent::ToolCall { call_id, name, input_json }` to that pane.
//!   3. Blocks (up to `timeout_ms`) on a `SyncReceiver` for the result.
//!   4. Returns `ToolCallResult { output_json, error }`.
//!
//! Pending-call state lives in `PENDING_CALLS`. `WASM app runtime::routing` feeds
//! `DrawCommand::ToolResult` back here to unblock the waiting broker thread.
//!
//! # Authorization model (#1182, re-scoped stint 0724 Phase C)
//!
//! Every registry entry records the host-established [`ScopeOrigin`](crate::host::scope::ScopeOrigin)
//! of the pane that exposed the tools, captured once at registration time via
//! `PlexiApp::origin_for_pane` — never the pane's (mutable) `workspace_root`.
//! When building a `ToolDispatcher`, the caller supplies its own
//! `viewer_context_id`, also host-resolved, never read from `router.active()`
//! or any client-supplied path. Reachability is decided by
//! `crate::host::scope::evaluate_reach` against `Scope::AppInstance`: only
//! tools owned in the caller's own context are included in the snapshot — two
//! contexts sharing the same canonical root are NOT mutually reachable,
//! because reachability's runtime dimension is `context_id`, not root. Every
//! `RegistryEntry` is context-owned with no cross-context grant issuance wired
//! yet (`grant: None` everywhere here), so today the rule is simply "same
//! context only." Cross-context tools are invisible to the caller and cannot
//! be invoked — the model never sees them, so confused-deputy calls fail
//! before they can be attempted.
//!
//! Every dispatched call is logged at `info` level with both the caller
//! (app_id, pane_id) and provider (pane_id, tool name) so every invocation is
//! attributable in the audit trail.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use crate::app_protocol::{AiTool, PlexiEvent};
use crate::host::scope::{evaluate_reach, Reach, ScopeOrigin};

// ── AppEventSender ──────────────────────────────────────────────────────────

/// Thin wrapper that lets external code send `PlexiEvent`s into a pane's
/// stdin channel without exposing the `StdinItem` enum publicly.
pub(crate) enum AppEventSender {
    #[cfg(test)]
    Channel(std::sync::mpsc::Sender<String>),
    Python(crate::host::wasm_python::AppendableStdin),
    Wasm(crate::host::wasm_pane::WasmInputSender),
}

impl AppEventSender {
    pub(crate) fn send_event(&self, event: &PlexiEvent) -> Result<(), String> {
        match self {
            #[cfg(test)]
            Self::Channel(tx) => {
                let mut json = serde_json::to_string(event).map_err(|error| error.to_string())?;
                json.push('\n');
                tx.send(json).map_err(|error| error.to_string())?;
                Ok(())
            }
            Self::Python(stdin) => {
                if let PlexiEvent::ToolCall {
                    call_id,
                    name,
                    input_json,
                    caller_id,
                } = event
                {
                    stdin
                        .push_json_line(&serde_json::json!({
                            "type": "tool_call",
                            "call_id": call_id,
                            "name": name,
                            "input_json": input_json,
                            "caller_id": caller_id,
                        }))
                        .map_err(|error| format!("send ToolCall to Python app: {error}"))?;
                }
                Ok(())
            }
            Self::Wasm(sender) => {
                let PlexiEvent::ToolCall {
                    call_id,
                    name,
                    input_json,
                    caller_id,
                } = event
                else {
                    return Err("WASM tool sender only accepts ToolCall events".to_string());
                };
                sender.send_tool_call(
                    call_id.clone(),
                    name.clone(),
                    input_json.clone(),
                    caller_id.clone(),
                )
            }
        }
    }
}

// ── ToolRegistry ────────────────────────────────────────────────────────────

/// One registered app — its tool definitions, how to reach it, and the
/// host-established scope origin it was registered from (used for
/// authorization checks).
struct RegistryEntry {
    tools: Vec<AiTool>,
    sender: AppEventSender,
    /// App type id (manifest `app.id`) — used to group tools by app for `/apps`.
    app_id: String,
    /// Host-established origin of the pane that exposed these tools, captured
    /// once at registration time via `PlexiApp::origin_for_pane` — stint 0724
    /// Phase C. Reachability is decided from `origin.context_id`, never from
    /// the pane's (mutable, `sync_app_cwd`-updated) `workspace_root`.
    origin: ScopeOrigin,
}

impl RegistryEntry {
    /// The owner scope for `evaluate_reach`: context-owned, per the stint's
    /// rule that context-owned resources are visible only inside their
    /// owning context. Two entries with colliding `pane_id`/`app_id` in
    /// different contexts are still distinct scopes — `Scope::AppInstance`
    /// carries `context_id` precisely so a numeric collision across contexts
    /// never grants reach.
    fn owner_scope(&self, pane_id: u64) -> crate::host::scope::Scope {
        crate::host::scope::Scope::AppInstance {
            pane_id,
            app_id: self.app_id.clone(),
            context_id: self.origin.context_id,
        }
    }
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
        app_id: String,
        tools: Vec<AiTool>,
        sender: AppEventSender,
        origin: ScopeOrigin,
    ) {
        self.entries.insert(
            pane_id,
            RegistryEntry {
                tools,
                sender,
                app_id,
                origin,
            },
        );
    }

    fn unregister(&mut self, pane_id: u64) {
        self.entries.remove(&pane_id);
    }

    /// Snapshot of tools visible to a caller in `viewer_context_id`, keyed by
    /// tool name. Only panes owned in that same context are included —
    /// decided by `evaluate_reach`, never by comparing paths. Two contexts
    /// anchored at the same canonical root are NOT mutually visible; only
    /// `context_id` equality (or, in a later stint, a `CrossContextGrant`)
    /// grants reach.
    ///
    /// If two panes expose the same tool name, both are **excluded** from the
    /// snapshot and an `error!` is logged listing the conflicting pane IDs.
    /// Silently picking a winner would dispatch to an arbitrary pane — callers
    /// instead get `tool_not_found`, which is the correct fail-visible signal
    /// until the apps are fixed or namespaced.
    fn snapshot_for_caller(
        &self,
        viewer_context_id: u64,
    ) -> HashMap<String, (u64, String, AiTool)> {
        let mut map: HashMap<String, (u64, String, AiTool)> = HashMap::new();
        let mut pane_ids: Vec<u64> = self
            .entries
            .iter()
            .filter(|(&pane_id, entry)| {
                matches!(
                    evaluate_reach(&entry.owner_scope(pane_id), viewer_context_id, None),
                    Reach::Allowed
                )
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
                map.insert(
                    tool.name.clone(),
                    (pane_id, tool.name.clone(), tool.clone()),
                );
            }
        }
        for (name, mut owners) in owners_by_name {
            if owners.len() > 1 {
                owners.sort_unstable();
                log::error!(
                    "tool_dispatch: tool name {:?} conflict — exposed by panes {:?}; \
                     tool withheld from model until conflict is resolved",
                    name,
                    owners
                );
                map.remove(&name);
            }
        }
        map
    }

    /// Snapshot for the host MCP server, where every app tool is externally
    /// named `<app_id>__<tool>`. The original tool name remains attached to
    /// the target so dispatch sends the provider exactly what it registered.
    ///
    /// Multiple live instances of the same app expose the same external name.
    /// Those collisions are withheld instead of selecting an arbitrary pane.
    fn namespaced_snapshot_for_caller(
        &self,
        viewer_context_id: u64,
    ) -> HashMap<String, (u64, String, AiTool)> {
        let mut pane_ids: Vec<u64> = self
            .entries
            .iter()
            .filter(|(&pane_id, entry)| {
                matches!(
                    evaluate_reach(&entry.owner_scope(pane_id), viewer_context_id, None),
                    Reach::Allowed
                )
            })
            .map(|(&pane_id, _)| pane_id)
            .collect();
        pane_ids.sort_unstable();

        let mut candidates: HashMap<String, Vec<(u64, String, AiTool)>> = HashMap::new();
        for pane_id in pane_ids {
            let Some(entry) = self.entries.get(&pane_id) else {
                continue;
            };
            for tool in &entry.tools {
                let external_name = format!("{}__{}", entry.app_id, tool.name);
                let mut external_tool = tool.clone();
                external_tool.name = external_name.clone();
                candidates.entry(external_name).or_default().push((
                    pane_id,
                    tool.name.clone(),
                    external_tool,
                ));
            }
        }

        let mut snapshot = HashMap::new();
        for (external_name, mut owners) in candidates {
            if owners.len() == 1 {
                snapshot.insert(external_name, owners.pop().unwrap());
            } else {
                let mut pane_ids: Vec<u64> =
                    owners.iter().map(|(pane_id, _, _)| *pane_id).collect();
                pane_ids.sort_unstable();
                log::error!(
                    "tool_dispatch: namespaced tool {:?} conflict — exposed by panes {:?}; \
                     tool withheld from external MCP callers",
                    external_name,
                    pane_ids
                );
            }
        }
        snapshot
    }

    /// Map from app_id to tool list for all panes reachable from
    /// `viewer_context_id`. Used by `/apps` to present tools grouped by the
    /// app that exposed them.
    fn apps_for_context(&self, viewer_context_id: u64) -> Vec<(String, Vec<AiTool>)> {
        let mut by_app: std::collections::BTreeMap<String, Vec<AiTool>> =
            std::collections::BTreeMap::new();
        for (&pane_id, entry) in &self.entries {
            if !matches!(
                evaluate_reach(&entry.owner_scope(pane_id), viewer_context_id, None),
                Reach::Allowed
            ) {
                continue;
            }
            by_app
                .entry(entry.app_id.clone())
                .or_default()
                .extend(entry.tools.iter().cloned());
        }
        by_app.into_iter().collect()
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
/// `DrawCommand::ExposeTools` arrives. `origin` is the host-established
/// `ScopeOrigin` for `pane_id`, resolved by the caller via
/// `PlexiApp::origin_for_pane` — never derived from the pane's own
/// `workspace_root` field, which `sync_app_cwd` can mutate live.
pub(crate) fn register(
    pane_id: u64,
    app_id: String,
    tools: Vec<AiTool>,
    sender: AppEventSender,
    origin: ScopeOrigin,
) {
    let count = tools.len();
    global_registry()
        .lock()
        .unwrap()
        .register(pane_id, app_id, tools, sender, origin);
    log::info!("tool_dispatch: registered {count} tool(s) for pane {pane_id}");
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
type PendingCallMap = Arc<Mutex<HashMap<String, std::sync::mpsc::SyncSender<ToolCallResult>>>>;

static PENDING_CALLS: OnceLock<PendingCallMap> = OnceLock::new();

fn pending_calls() -> &'static PendingCallMap {
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

// ── ToolCallHooks ────────────────────────────────────────────────────────────

/// Per-call hooks the dispatching actor can install on its `ToolDispatcher`
/// snapshot (Phase D: the host Assistant gates ask-tier tools through the
/// permission sheet here). Hooks run on the broker worker thread;
/// `before_call` may block while a decision is collected on the UI thread.
/// Callers that install no hooks (PGAP apps, `AgentHost`) are unaffected.
pub trait ToolCallHooks: Send + Sync {
    /// Called after the registry lookup, before the call is sent to the
    /// providing app. `Err(reason)` blocks the call; the reason is returned
    /// to the model as the tool error.
    fn before_call(&self, name: &str, input_json: &str) -> Result<(), String>;

    /// Called with the call outcome (`error: None` = success, with the
    /// tool's `output_json` when one was produced — the Assistant lifts
    /// render payloads like file-edit diffs from it). Not called when
    /// `before_call` blocked the call or the tool was not found.
    fn after_call(&self, name: &str, error: Option<&str>, output_json: Option<&str>);
}

// ── ToolDispatcher ──────────────────────────────────────────────────────────

/// Snapshot of the tool registry for one broker invocation. Created by the
/// routing layer before spawning the broker thread. Passed into the broker
/// via `AiBrokerRequest::tool_dispatcher`.
///
/// Only contains tools reachable from the caller's context — cross-context
/// tools are excluded at construction time (via `evaluate_reach`) and never
/// visible to the dispatching app or the model it drives.
pub struct ToolDispatcher {
    /// Snapshot: exposed_name → (provider_pane_id, provider_tool_name, AiTool).
    /// Already filtered to the caller's context.
    tools: HashMap<String, (u64, String, AiTool)>,
    /// Caller-local host tools (Phase D3: the Assistant's
    /// `host.events.subscribe`/`unsubscribe`). Dispatched through
    /// `host_handler`, never sent to a pane. Unaffected by `retain_allowed`
    /// — the registering caller owns their gating.
    host_tools: HashMap<String, AiTool>,
    /// Handler for `host_tools` calls. Runs on the broker worker thread.
    host_handler: Option<HostToolHandler>,
    /// Caller identity for audit logging.
    caller_app_id: String,
    /// Caller pane id for audit logging.
    caller_pane_id: u64,
    /// Optional per-call hooks (permission gating + call observation).
    hooks: Option<Arc<dyn ToolCallHooks>>,
}

/// Handler for caller-local host tools: `(tool_name, input_json) → result`.
pub type HostToolHandler = Arc<dyn Fn(&str, &str) -> ToolCallResult + Send + Sync>;

impl std::fmt::Debug for ToolDispatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolDispatcher")
            .field("tools", &self.tools.keys().collect::<Vec<_>>())
            .field("host_tools", &self.host_tools.keys().collect::<Vec<_>>())
            .field("caller_app_id", &self.caller_app_id)
            .field("caller_pane_id", &self.caller_pane_id)
            .field("hooks", &self.hooks.is_some())
            .finish()
    }
}

impl ToolDispatcher {
    /// Build a dispatcher scoped to `viewer_context_id` — the caller's own
    /// host-established context, never `router.active()` or a client-supplied
    /// path. Only tools reachable from that context are included.
    pub fn from_registry(
        caller_pane_id: u64,
        caller_app_id: String,
        viewer_context_id: u64,
    ) -> Self {
        let registry = global_registry().lock().unwrap();
        let tools = registry.snapshot_for_caller(viewer_context_id);
        let visible: Vec<&str> = tools.keys().map(|s| s.as_str()).collect();
        log::info!(
            "tool_dispatch: dispatcher for caller={caller_app_id} pane={caller_pane_id} viewer_context={viewer_context_id} — {} tool(s) visible: {visible:?}",
            tools.len(),
        );
        Self {
            tools,
            host_tools: HashMap::new(),
            host_handler: None,
            caller_app_id,
            caller_pane_id,
            hooks: None,
        }
    }

    /// Build the context-scoped snapshot exposed by the singleton host MCP
    /// server. Definitions are namespaced `<app_id>__<tool>` while dispatch
    /// retains the provider's original tool name.
    pub(crate) fn from_namespaced_registry(caller_pane_id: u64, viewer_context_id: u64) -> Self {
        let caller_app_id = format!("mcp:pane:{caller_pane_id}");
        let registry = global_registry().lock().unwrap();
        let tools = registry.namespaced_snapshot_for_caller(viewer_context_id);
        log::info!(
            "tool_dispatch: external MCP dispatcher caller={caller_app_id} viewer_context={viewer_context_id} — {} tool(s) visible",
            tools.len(),
        );
        Self {
            tools,
            host_tools: HashMap::new(),
            host_handler: None,
            caller_app_id,
            caller_pane_id,
            hooks: None,
        }
    }

    /// Register caller-local host tools (Phase D3). They are visible in
    /// `all_tools()` and dispatched through `handler` on the broker worker
    /// thread — never routed to a pane. Hooks still observe these calls.
    pub fn add_host_tools(&mut self, tools: Vec<AiTool>, handler: HostToolHandler) {
        for tool in tools {
            log::info!(
                "tool_dispatch: caller={} registered host tool '{}'",
                self.caller_app_id,
                tool.name
            );
            self.host_tools.insert(tool.name.clone(), tool);
        }
        self.host_handler = Some(handler);
    }

    /// Install per-call hooks (Phase D: the Assistant's ask-gate). Hooks see
    /// every `dispatch_call` on this snapshot.
    pub fn set_hooks(&mut self, hooks: Arc<dyn ToolCallHooks>) {
        self.hooks = Some(hooks);
    }

    /// All tools visible at snapshot time, for injection into the LLM request.
    pub fn all_tools(&self) -> Vec<AiTool> {
        self.tools
            .values()
            .map(|(_, _, tool)| tool.clone())
            .chain(self.host_tools.values().cloned())
            .collect()
    }

    /// Apps and their tools visible to the caller's context — for `/apps`.
    /// Returns `(app_id, tools)` pairs sorted by app_id.
    pub fn apps_for_context(viewer_context_id: u64) -> Vec<(String, Vec<AiTool>)> {
        global_registry()
            .lock()
            .unwrap()
            .apps_for_context(viewer_context_id)
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

    /// Dispatch a single tool call. Blocks until the app responds or times
    /// out. When hooks are installed, `before_call` runs first (and may block
    /// the call) and `after_call` observes the outcome.
    pub fn dispatch_call(&self, call_id: String, name: &str, input_json: String) -> ToolCallResult {
        // Host tools never reach a pane: hooks observe them, the caller's
        // host handler resolves them in-process.
        if self.host_tools.contains_key(name) {
            let Some(handler) = &self.host_handler else {
                return ToolCallResult {
                    output_json: None,
                    error: Some(format!("host_tool_unhandled: no handler for {name:?}")),
                };
            };
            if let Some(hooks) = &self.hooks {
                if let Err(reason) = hooks.before_call(name, &input_json) {
                    log::info!(
                        "tool_dispatch: caller={} — host tool '{name}' blocked by hook: {reason}",
                        self.caller_app_id
                    );
                    return ToolCallResult {
                        output_json: None,
                        error: Some(reason),
                    };
                }
            }
            log::info!(
                "tool_dispatch: caller={} → host tool {name:?} call_id={call_id:?}",
                self.caller_app_id
            );
            let result = handler(name, &input_json);
            if let Some(hooks) = &self.hooks {
                hooks.after_call(name, result.error.as_deref(), result.output_json.as_deref());
            }
            return result;
        }
        if !self.tools.contains_key(name) {
            // Fall through to dispatch_inner's tool_not_found path without
            // invoking hooks for tools the snapshot does not contain.
            return self.dispatch_inner(call_id, name, input_json);
        }
        if let Some(hooks) = &self.hooks {
            if let Err(reason) = hooks.before_call(name, &input_json) {
                log::info!(
                    "tool_dispatch: caller={} pane={} — tool '{name}' blocked by hook: {reason}",
                    self.caller_app_id,
                    self.caller_pane_id
                );
                return ToolCallResult {
                    output_json: None,
                    error: Some(reason),
                };
            }
        }
        let result = self.dispatch_inner(call_id, name, input_json);
        if let Some(hooks) = &self.hooks {
            hooks.after_call(name, result.error.as_deref(), result.output_json.as_deref());
        }
        result
    }

    fn dispatch_inner(&self, call_id: String, name: &str, input_json: String) -> ToolCallResult {
        let (pane_id, provider_tool_name, tool) = match self.tools.get(name) {
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
                sender
                    .send_event(&PlexiEvent::ToolCall {
                        call_id: call_id.clone(),
                        name: provider_tool_name.clone(),
                        input_json,
                        caller_id: self.caller_app_id.clone(),
                    })
                    .is_ok()
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
            output_schema: serde_json::json!({"type": "object"}),
            timeout_ms: Some(100),
            read_only: false,
        }
    }

    /// A minimal `ScopeOrigin` for tests that only care about `context_id` —
    /// every other field is a deterministic placeholder never inspected by
    /// `evaluate_reach` or `Scope::AppInstance`.
    fn origin(context_id: u64, pane_id: u64) -> ScopeOrigin {
        ScopeOrigin {
            context_id,
            context_root: PathBuf::from(format!("/ctx-root-{context_id}")),
            window_id: context_id,
            pane_id,
            app_id: None,
        }
    }

    // ── ToolRegistry unit tests (private access via same-module #[cfg(test)]) ──

    #[test]
    fn same_context_tools_are_visible() {
        let mut reg = ToolRegistry::new();
        let (tx, _rx) = std::sync::mpsc::channel();
        reg.register(
            1,
            "search-app".to_string(),
            vec![make_tool("search")],
            AppEventSender::Channel(tx),
            origin(10, 1),
        );

        let snap = reg.snapshot_for_caller(10);
        assert!(
            snap.contains_key("search"),
            "same-context tool must be visible"
        );
        assert_eq!(snap["search"].0, 1, "provider pane id must be 1");
    }

    #[test]
    fn cross_context_tools_are_hidden() {
        let mut reg = ToolRegistry::new();
        let (tx, _rx) = std::sync::mpsc::channel();
        reg.register(
            10,
            "attacker-app".to_string(),
            vec![make_tool("dangerous_tool")],
            AppEventSender::Channel(tx.clone()),
            origin(100, 10),
        );
        reg.register(
            20,
            "victim-app".to_string(),
            vec![make_tool("safe_tool")],
            AppEventSender::Channel(tx),
            origin(200, 20),
        );

        // Snapshot from the attacker's context must NOT see the victim's tools.
        let snap_attacker = reg.snapshot_for_caller(100);
        assert!(snap_attacker.contains_key("dangerous_tool"));
        assert!(
            !snap_attacker.contains_key("safe_tool"),
            "cross-context tool must be hidden from attacker"
        );

        // Snapshot from the victim's context must NOT see the attacker's tools.
        let snap_victim = reg.snapshot_for_caller(200);
        assert!(snap_victim.contains_key("safe_tool"));
        assert!(
            !snap_victim.contains_key("dangerous_tool"),
            "cross-context tool must be hidden from victim"
        );
    }

    #[test]
    fn unregistered_pane_tools_disappear() {
        let mut reg = ToolRegistry::new();
        let (tx, _rx) = std::sync::mpsc::channel();
        reg.register(
            5,
            "app-x".to_string(),
            vec![make_tool("tool_a")],
            AppEventSender::Channel(tx),
            origin(1, 5),
        );

        assert!(reg.snapshot_for_caller(1).contains_key("tool_a"));
        reg.unregister(5);
        assert!(
            reg.snapshot_for_caller(1).is_empty(),
            "tools must disappear after pane unregisters"
        );
    }

    #[test]
    fn empty_snapshot_for_unknown_context() {
        let reg = ToolRegistry::new();
        let snap = reg.snapshot_for_caller(999_999);
        assert!(snap.is_empty());
    }

    /// Deliberate tightening vs. pre-Phase-C behavior (stint 0724 Phase C):
    /// two sibling contexts anchored at the SAME canonical root are no longer
    /// mutually visible. Before this phase, reachability was decided by
    /// path-equality on `AppPane::workspace_root`, so two panes whose contexts
    /// happened to share a root were mutually reachable regardless of
    /// `context_id`. Reachability's runtime dimension is now `context_id`
    /// only — a tool registered from a pane in context A must stay invisible
    /// to a caller in a sibling context B that merely shares A's root, unless
    /// a `CrossContextGrant` (not wired until a later stint) says otherwise.
    #[test]
    fn connector_from_same_root_sibling_context_is_unreachable_without_grant() {
        let mut reg = ToolRegistry::new();
        let (tx, _rx) = std::sync::mpsc::channel();
        let shared_root = PathBuf::from("/workspace/shared-root");
        let context_a = 501u64;
        let context_b = 502u64; // sibling context, SAME canonical root as A
        let origin_a = ScopeOrigin {
            context_id: context_a,
            context_root: shared_root.clone(),
            window_id: 1,
            pane_id: 30,
            app_id: Some("root-app".to_string()),
        };
        reg.register(
            30,
            "root-app".to_string(),
            vec![make_tool("shared_root_tool")],
            AppEventSender::Channel(tx),
            origin_a,
        );

        // Sanity: the owning context still sees its own tool.
        let snap_a = reg.snapshot_for_caller(context_a);
        assert!(snap_a.contains_key("shared_root_tool"));

        // The sibling context sharing the same root must NOT see it.
        let snap_b = reg.snapshot_for_caller(context_b);
        assert!(
            !snap_b.contains_key("shared_root_tool"),
            "same-root sibling context must not see another context's tool without a grant"
        );
    }

    // ── ToolDispatcher: unauthorized call returns tool_not_found ──────────────
    //
    // The authorization boundary is: cross-context tools are excluded from the
    // snapshot, so `dispatch_call` returns `tool_not_found` for them — the model
    // never learns they exist.

    #[test]
    fn dispatcher_excludes_cross_context_tools() {
        // Register a tool in context B.
        let (tx_b, _rx_b) = std::sync::mpsc::channel();
        register(
            999,
            "app-b".to_string(),
            vec![make_tool("secret_tool")],
            AppEventSender::Channel(tx_b),
            origin(200, 999),
        );

        // Build a dispatcher for a caller in context A.
        let dispatcher = ToolDispatcher::from_registry(1, "app_a".to_string(), 100);

        // The tool must not appear in the visible set.
        let visible: Vec<String> = dispatcher.all_tools().into_iter().map(|t| t.name).collect();
        assert!(
            !visible.contains(&"secret_tool".to_string()),
            "cross-context tool must not appear in dispatcher: {visible:?}"
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

    /// Two panes in the same context exposing the same tool name must both be
    /// excluded from the snapshot — no silent winner selection.
    #[test]
    fn conflicting_tool_names_excluded_from_snapshot() {
        let mut reg = ToolRegistry::new();
        let (tx1, _rx1) = std::sync::mpsc::channel();
        let (tx2, _rx2) = std::sync::mpsc::channel();
        reg.register(
            10,
            "app-conflict-1".to_string(),
            vec![make_tool("shared_tool"), make_tool("unique_a")],
            AppEventSender::Channel(tx1),
            origin(50, 10),
        );
        reg.register(
            20,
            "app-conflict-2".to_string(),
            vec![make_tool("shared_tool"), make_tool("unique_b")],
            AppEventSender::Channel(tx2),
            origin(50, 20),
        );

        let snap = reg.snapshot_for_caller(50);
        assert!(
            !snap.contains_key("shared_tool"),
            "conflicting tool must be excluded from snapshot, not silently picked"
        );
        assert!(
            snap.contains_key("unique_a"),
            "non-conflicting tool from pane 10 must remain visible"
        );
        assert!(
            snap.contains_key("unique_b"),
            "non-conflicting tool from pane 20 must remain visible"
        );
    }

    #[test]
    fn namespaced_snapshot_withholds_duplicate_app_instances() {
        let mut reg = ToolRegistry::new();
        let (tx1, _rx1) = std::sync::mpsc::channel();
        let (tx2, _rx2) = std::sync::mpsc::channel();
        reg.register(
            30,
            "same-app".to_string(),
            vec![make_tool("echo")],
            AppEventSender::Channel(tx1),
            origin(60, 30),
        );
        reg.register(
            20,
            "same-app".to_string(),
            vec![make_tool("echo"), make_tool("unique")],
            AppEventSender::Channel(tx2),
            origin(60, 20),
        );

        let snapshot = reg.namespaced_snapshot_for_caller(60);
        assert!(
            !snapshot.contains_key("same-app__echo"),
            "two live instances must not select an arbitrary provider"
        );
        let (pane_id, provider_name, tool) = &snapshot["same-app__unique"];
        assert_eq!(*pane_id, 20);
        assert_eq!(provider_name, "unique");
        assert_eq!(tool.name, "same-app__unique");
    }

    /// A `before_call` error must block the call and skip `after_call`;
    /// unknown tools must return `tool_not_found` without invoking hooks.
    #[test]
    fn hooks_block_calls_and_skip_unknown_tools() {
        struct DenyHook;
        impl ToolCallHooks for DenyHook {
            fn before_call(&self, _name: &str, _input: &str) -> Result<(), String> {
                Err("permission_denied: test gate".to_string())
            }
            fn after_call(&self, _name: &str, _error: Option<&str>, _output: Option<&str>) {
                panic!("after_call must not run for blocked or unknown calls");
            }
        }

        let (tx, _rx) = std::sync::mpsc::channel();
        register(
            998,
            "hooks-app".to_string(),
            vec![make_tool("gated_tool")],
            AppEventSender::Channel(tx),
            origin(70, 998),
        );
        let mut dispatcher = ToolDispatcher::from_registry(2, "agent:assistant".to_string(), 70);
        dispatcher.set_hooks(Arc::new(DenyHook));

        let blocked = dispatcher.dispatch_call("c1".to_string(), "gated_tool", "{}".to_string());
        assert!(
            blocked
                .error
                .as_deref()
                .unwrap_or("")
                .contains("permission_denied"),
            "hook denial must surface as the tool error: {:?}",
            blocked.error
        );

        let unknown = dispatcher.dispatch_call("c2".to_string(), "nope", "{}".to_string());
        assert!(
            unknown
                .error
                .as_deref()
                .unwrap_or("")
                .contains("tool_not_found"),
            "unknown tool must bypass hooks: {:?}",
            unknown.error
        );

        unregister(998);
    }

    #[test]
    fn wasm_provider_receives_tool_call_and_returns_result() {
        let (sender, queue) = crate::host::wasm_pane::WasmInputSender::new_for_test();
        register(
            997,
            "wasm-tools".to_string(),
            vec![make_tool("wasm.echo")],
            AppEventSender::Wasm(sender),
            origin(80, 997),
        );
        let dispatcher = ToolDispatcher::from_registry(2, "agent:assistant".to_string(), 80);
        let worker = std::thread::spawn(move || {
            dispatcher.dispatch_call(
                "wasm-call-1".to_string(),
                "wasm.echo",
                r#"{"value":7}"#.to_string(),
            )
        });

        let event = (0..100)
            .find_map(|_| {
                let event = queue.pop();
                if event.is_none() {
                    std::thread::sleep(Duration::from_millis(2));
                }
                event
            })
            .expect("WASM app must receive ToolCall");
        let crate::host::wasm_app::InputEvent::ToolCall(call) = event else {
            panic!("expected ToolCall, got {event:?}");
        };
        assert_eq!(call.call_id, "wasm-call-1");
        assert_eq!(call.name, "wasm.echo");
        assert_eq!(call.input_json, r#"{"value":7}"#);
        assert_eq!(call.caller_id, "agent:assistant");

        resolve_pending(
            &call.call_id,
            ToolCallResult {
                output_json: Some(r#"{"value":7}"#.to_string()),
                error: None,
            },
        );
        let result = worker.join().unwrap();
        assert_eq!(result.output_json.as_deref(), Some(r#"{"value":7}"#));
        assert!(result.error.is_none());
        unregister(997);
    }

    #[test]
    fn denied_wasm_tool_call_never_reaches_the_guest() {
        struct DenyHook;
        impl ToolCallHooks for DenyHook {
            fn before_call(&self, _name: &str, _input: &str) -> Result<(), String> {
                Err("permission_denied: app connector denied".to_string())
            }
            fn after_call(&self, _name: &str, _error: Option<&str>, _output: Option<&str>) {}
        }

        let (sender, queue) = crate::host::wasm_pane::WasmInputSender::new_for_test();
        register(
            996,
            "wasm-denied".to_string(),
            vec![make_tool("wasm.write")],
            AppEventSender::Wasm(sender),
            origin(90, 996),
        );
        let mut dispatcher = ToolDispatcher::from_registry(2, "agent:assistant".to_string(), 90);
        dispatcher.set_hooks(Arc::new(DenyHook));

        let result = dispatcher.dispatch_call(
            "wasm-call-denied".to_string(),
            "wasm.write",
            "{}".to_string(),
        );
        assert_eq!(
            result.error.as_deref(),
            Some("permission_denied: app connector denied")
        );
        assert!(
            queue.is_empty(),
            "denied calls must not enter WASM update()"
        );
        unregister(996);
    }
}
