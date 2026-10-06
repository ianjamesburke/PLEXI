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

use crate::protocol::{AiTool, PlexiEvent};
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
                    authorization,
                } = event
                {
                    stdin
                        .push_json_line(&serde_json::json!({
                            "type": "tool_call",
                            "call_id": call_id,
                            "name": name,
                            "input_json": input_json,
                            "caller_id": caller_id,
                            "authorization": authorization,
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
                    authorization: _,
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
    /// If two panes expose the same tool name, the bare name is not offered.
    /// Each instance stays addressable as `<tool>@<pane_id>`. Silently picking
    /// a pane would let a grant for one board reach the other.
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

        let mut owners_by_name: HashMap<String, Vec<(u64, AiTool)>> = HashMap::new();
        for &pane_id in &pane_ids {
            let Some(entry) = self.entries.get(&pane_id) else {
                continue;
            };
            for tool in &entry.tools {
                owners_by_name
                    .entry(tool.name.clone())
                    .or_default()
                    .push((pane_id, tool.clone()));
            }
        }
        for (name, mut owners) in owners_by_name {
            owners.sort_by_key(|(pane_id, _)| *pane_id);
            if owners.len() == 1 {
                let (pane_id, tool) = owners.pop().unwrap();
                map.insert(name, (pane_id, tool.name.clone(), tool));
                continue;
            }
            let panes: Vec<u64> = owners.iter().map(|(pane_id, _)| *pane_id).collect();
            log::info!(
                "tool_dispatch: tool {name} is live on panes {panes:?}; address {name}@<pane>"
            );
            for (pane_id, tool) in owners {
                let qualified = format!("{name}@{pane_id}");
                let mut exposed = tool.clone();
                exposed.name = qualified.clone();
                map.insert(qualified, (pane_id, tool.name, exposed));
            }
        }
        map
    }

    /// Snapshot for the host MCP server, where every app tool is externally
    /// named `<app_id>__<tool>`. The original tool name remains attached to
    /// the target so dispatch sends the provider exactly what it registered.
    ///
    /// Multiple live instances keep distinct names `<app>:<pane>__<tool>`.
    /// The unqualified `<app>__<tool>` name is offered only when one instance
    /// is live, so a call cannot land on an arbitrary pane.
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
            owners.sort_by_key(|(pane_id, _, _)| *pane_id);
            if owners.len() == 1 {
                snapshot.insert(external_name, owners.pop().unwrap());
                continue;
            }
            let panes: Vec<u64> = owners.iter().map(|(pane_id, _, _)| *pane_id).collect();
            log::info!(
                "tool_dispatch: {external_name} is live on panes {panes:?}; address <app>:<pane>__<tool>"
            );
            let app_id = external_name
                .split_once("__")
                .map(|(app, _)| app)
                .unwrap_or(external_name.as_str());
            for (pane_id, provider_name, mut tool) in owners {
                let qualified = format!("{app_id}:{pane_id}__{provider_name}");
                tool.name = qualified.clone();
                snapshot.insert(qualified, (pane_id, provider_name, tool));
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

/// Tools currently exposed by live app panes: app id, pane id, tool name, description.
pub(crate) fn live_exposed_tools() -> Vec<(String, u64, String, String)> {
    let registry = global_registry().lock().unwrap_or_else(|error| error.into_inner());
    let mut tools = Vec::new();
    for (pane_id, entry) in &registry.entries {
        for tool in &entry.tools {
            tools.push((
                entry.app_id.clone(),
                *pane_id,
                tool.name.clone(),
                tool.description.clone(),
            ));
        }
    }
    tools.sort_by(|left, right| left.0.cmp(&right.0).then(left.2.cmp(&right.2)));
    tools
}

/// Whether `pane_id` currently has tools in the process-global registry.
/// Harnesses wait on this after the first render: `ExposeTools` can land a
/// frame later when several guests start at once, and a call before that
/// answers `tool_not_found`.
#[cfg(test)]
pub(crate) fn pane_has_registered_tools(pane_id: u64) -> bool {
    global_registry()
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .entries
        .contains_key(&pane_id)
}

// ── Pending calls ────────────────────────────────────────────────────────────

/// Result returned to the broker by a completed tool call.
#[derive(Debug, Clone)]
pub struct ToolCallResult {
    pub output_json: Option<String>,
    pub error: Option<String>,
    /// Stable code: `permission_required`, `permission_denied`, `stale_revision`,
    /// `edit_conflict`, `operation_conflict`, `outcome_unknown`, or `None`.
    pub error_code: Option<String>,
    pub pending_request_id: Option<String>,
}

impl ToolCallResult {
    /// A successful call whose output is already serialized JSON.
    pub fn ok(output_json: impl Into<String>) -> Self {
        Self {
            output_json: Some(output_json.into()),
            error: None,
            error_code: None,
            pending_request_id: None,
        }
    }

    /// A successful call carrying a JSON value to serialize.
    pub fn ok_value(value: serde_json::Value) -> Self {
        Self::ok(value.to_string())
    }

    /// A failed call. `error` is the model-facing reason, tagged with the
    /// convention every tool site uses (`invalid_input:`, `tool_timeout:`, …).
    pub fn err(error: impl Into<String>) -> Self {
        Self {
            output_json: None,
            error: Some(error.into()),
            error_code: None,
            pending_request_id: None,
        }
    }

    pub fn coded(code: &str, call_id: &str, pending: Option<&str>) -> Self {
        Self {
            output_json: None,
            error: Some(crate::broker::gate::structured_error(code, call_id, pending)),
            error_code: Some(code.to_string()),
            pending_request_id: pending.map(str::to_string),
        }
    }
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
    /// Called after the monitor admits the call, before it is sent. This
    /// cannot allow or deny the call.
    fn before_call(&self, name: &str, input_json: &str);

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
/// Who is dispatching. The monitor binds the grant to these fields.
#[derive(Debug, Clone)]
pub struct DispatchScope {
    pub caller_pane_id: u64,
    pub caller_app_id: String,
    pub actor_type: crate::broker::ActorType,
    pub actor_scope: crate::broker::ActorScope,
    pub workspace_root: std::path::PathBuf,
    pub context_id: u64,
    pub trust_origin: String,
}

impl DispatchScope {
    pub fn new(
        caller_pane_id: u64,
        caller_app_id: impl Into<String>,
        workspace_root: impl Into<std::path::PathBuf>,
        context_id: u64,
    ) -> Self {
        let caller_app_id = caller_app_id.into();
        let actor_type = if caller_app_id.starts_with("agent:") || caller_app_id.starts_with("pane:")
        {
            crate::broker::ActorType::Agent
        } else {
            crate::broker::ActorType::App
        };
        Self {
            caller_pane_id,
            caller_app_id,
            actor_type,
            actor_scope: crate::broker::ActorScope::User,
            workspace_root: workspace_root.into(),
            context_id,
            trust_origin: "host".to_string(),
        }
    }
}

/// Desktop sheet (or another in-process presenter). MCP, CLI, and socket
/// callers leave this empty and receive `permission_required`.
pub trait PermissionPresenter: Send + Sync {
    fn present(
        &self,
        pending_id: &str,
        tool: &str,
        summary: &str,
        actor: &str,
        resource: &str,
    ) -> crate::broker::gate::ApprovalChoice;
}

/// Why `app call` could not choose a single tool provider.
#[derive(Debug)]
pub(crate) enum AppToolRouteError {
    /// Two or more panes expose the tool and the caller did not name one.
    Ambiguous { message: String, panes: Vec<u64> },
    /// The named pane does not expose the tool.
    Missing { message: String },
}

impl AppToolRouteError {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::Ambiguous { .. } => "ambiguous_instance",
            Self::Missing { .. } => "tool_not_found",
        }
    }

    pub(crate) fn message(&self) -> &str {
        match self {
            Self::Ambiguous { message, .. } | Self::Missing { message } => message,
        }
    }

    pub(crate) fn panes(&self) -> &[u64] {
        match self {
            Self::Ambiguous { panes, .. } => panes,
            Self::Missing { .. } => &[],
        }
    }
}

fn pane_ids_in_qualified_names(prefix: &str, names: &[String]) -> Vec<u64> {
    let mut panes = Vec::new();
    for name in names {
        let Some(rest) = name.strip_prefix(prefix) else {
            continue;
        };
        let Some((id, _)) = rest.split_once("__") else {
            continue;
        };
        if let Ok(pane) = id.parse::<u64>() {
            panes.push(pane);
        }
    }
    panes
}

pub struct ToolDispatcher {
    /// Snapshot: exposed_name → (provider_pane_id, provider_tool_name, AiTool).
    /// Already filtered to the caller's context.
    tools: HashMap<String, (u64, String, AiTool)>,
    /// Caller-local host tools. Dispatched through `host_handler`, never sent
    /// to a pane. Still admitted by the permission monitor.
    host_tools: HashMap<String, AiTool>,
    /// Handler for `host_tools` calls. Runs on the broker worker thread.
    host_handler: Option<HostToolHandler>,
    /// Caller identity for audit logging.
    caller_app_id: String,
    /// Caller pane id for audit logging.
    caller_pane_id: u64,
    scope: DispatchScope,
    monitor: Arc<crate::broker::gate::PermissionMonitor>,
    /// Observation only. A hook cannot allow or deny a call.
    hooks: Option<Arc<dyn ToolCallHooks>>,
    presenter: Option<Arc<dyn PermissionPresenter>>,
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
            .field("presenter", &self.presenter.is_some())
            .finish()
    }
}

impl ToolDispatcher {
    /// Build a dispatcher scoped to `viewer_context_id` — the caller's own
    /// host-established context, never `router.active()` or a client-supplied
    /// path. Only tools reachable from that context are included.
    pub fn from_registry(scope: DispatchScope, monitor: Arc<crate::broker::gate::PermissionMonitor>) -> Self {
        let registry = global_registry().lock().unwrap();
        let tools = registry.snapshot_for_caller(scope.context_id);
        let visible: Vec<&str> = tools.keys().map(|s| s.as_str()).collect();
        log::info!(
            "tool_dispatch: dispatcher for caller={} pane={} viewer_context={} — {} tool(s) visible: {visible:?}",
            scope.caller_app_id,
            scope.caller_pane_id,
            scope.context_id,
            tools.len(),
        );
        Self::from_parts(scope, monitor, tools)
    }

    /// Build the context-scoped snapshot exposed by the singleton host MCP
    /// server. Definitions are namespaced `<app_id>__<tool>` while dispatch
    /// retains the provider's original tool name.
    pub(crate) fn from_namespaced_registry(
        scope: DispatchScope,
        monitor: Arc<crate::broker::gate::PermissionMonitor>,
    ) -> Self {
        Self::namespaced_for(scope, monitor)
    }

    /// The same namespaced, context-scoped snapshot for a host-identified
    /// caller. The identity is stamped by the host, never by the caller.
    pub(crate) fn namespaced_for(
        scope: DispatchScope,
        monitor: Arc<crate::broker::gate::PermissionMonitor>,
    ) -> Self {
        let registry = global_registry().lock().unwrap();
        let tools = registry.namespaced_snapshot_for_caller(scope.context_id);
        log::info!(
            "tool_dispatch: namespaced dispatcher caller={} viewer_context={} — {} tool(s) visible",
            scope.caller_app_id,
            scope.context_id,
            tools.len(),
        );
        Self::from_parts(scope, monitor, tools)
    }

    fn from_parts(
        scope: DispatchScope,
        monitor: Arc<crate::broker::gate::PermissionMonitor>,
        tools: HashMap<String, (u64, String, AiTool)>,
    ) -> Self {
        let caller_app_id = scope.caller_app_id.clone();
        let caller_pane_id = scope.caller_pane_id;
        Self {
            tools,
            host_tools: HashMap::new(),
            host_handler: None,
            caller_app_id,
            caller_pane_id,
            scope,
            monitor,
            hooks: None,
            presenter: None,
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

    /// Observation and presentation only. Hooks cannot allow or deny a call;
    /// the permission monitor inside `dispatch_call` is the only authorizer.
    pub fn set_hooks(&mut self, hooks: Arc<dyn ToolCallHooks>) {
        self.hooks = Some(hooks);
    }

    pub fn set_presenter(&mut self, presenter: Arc<dyn PermissionPresenter>) {
        self.presenter = Some(presenter);
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

    /// Dispatch a single tool call. The permission monitor admits the call
    /// before any handler runs. Hooks only observe a call that was admitted.
    pub fn dispatch_call(&self, call_id: String, name: &str, input_json: String) -> ToolCallResult {
        let known = self.host_tools.contains_key(name) || self.tools.contains_key(name);
        if !known {
            crate::broker::gate::trace_gate(format!(
                "tool_dispatch: tool_not_found caller={} tool={name} call_id={call_id}",
                self.caller_app_id
            ));
            return self.dispatch_inner(call_id, name, input_json, None);
        }
        let (package, instance) = self.provider_of(name);
        let target_type = if self.host_tools.contains_key(name) {
            crate::broker::TargetType::HostTool
        } else {
            crate::broker::TargetType::AppConnector
        };
        let mut admission = self.monitor.admit(crate::broker::gate::AdmitRequest {
            call_id: &call_id,
            tool: name,
            input_json: &input_json,
            actor_type: self.scope.actor_type,
            actor_id: &self.scope.caller_app_id,
            actor_scope: self.scope.actor_scope,
            trust_origin: &self.scope.trust_origin,
            workspace_root: &self.scope.workspace_root,
            context_id: self.scope.context_id,
            package_id: &package,
            instance_id: instance,
            target_type,
        });
        if let crate::broker::gate::Admission::Required { pending_request_id } = &admission {
            if let Some(presenter) = &self.presenter {
                let (_, resource_id) = crate::broker::gate::resource_of(name, &input_json);
                let choice = presenter.present(
                    pending_request_id,
                    name,
                    &input_json,
                    &self.scope.caller_app_id,
                    resource_id.as_deref().unwrap_or(""),
                );
                if matches!(
                    choice,
                    crate::broker::gate::ApprovalChoice::Deny
                        | crate::broker::gate::ApprovalChoice::DenyAlways
                ) {
                    let _ = self.monitor.approve_pending(pending_request_id, choice);
                    return ToolCallResult::coded(
                        "permission_denied",
                        &call_id,
                        Some(pending_request_id),
                    );
                }
                if self
                    .monitor
                    .approve_pending(pending_request_id, choice)
                    .is_err()
                {
                    return ToolCallResult::coded("permission_denied", &call_id, Some(pending_request_id));
                }
                admission = self.monitor.admit(crate::broker::gate::AdmitRequest {
                    call_id: &call_id,
                    tool: name,
                    input_json: &input_json,
                    actor_type: self.scope.actor_type,
                    actor_id: &self.scope.caller_app_id,
                    actor_scope: self.scope.actor_scope,
                    trust_origin: &self.scope.trust_origin,
                    workspace_root: &self.scope.workspace_root,
                    context_id: self.scope.context_id,
                    package_id: &package,
                    instance_id: instance,
                    target_type,
                });
            } else {
                log::info!(
                    "tool_dispatch: permission_required caller={} tool={name} pending={pending_request_id}",
                    self.caller_app_id
                );
                return ToolCallResult::coded(
                    "permission_required",
                    &call_id,
                    Some(pending_request_id),
                );
            }
        }
        let crate::broker::gate::Admission::Proceed {
            grant_id,
            fingerprint,
            resource_id,
            resource_scope,
        } = admission
        else {
            let code = match admission {
                crate::broker::gate::Admission::Denied { code } => code,
                crate::broker::gate::Admission::Required { .. } => "permission_required",
                crate::broker::gate::Admission::Proceed { .. } => "permission_denied",
            };
            return ToolCallResult::coded(code, &call_id, None);
        };
        let operation_id = crate::broker::gate::operation_id_of(&input_json);
        let resource = resource_id.clone().unwrap_or_default();
        if self
            .monitor
            .note_use(
                &self.scope.caller_app_id,
                &call_id,
                &grant_id,
                &fingerprint,
                &resource,
                &operation_id,
            )
            .is_err()
        {
            log::info!(
                "tool_dispatch: blocked before execution caller={} tool={name} call_id={call_id}",
                self.caller_app_id
            );
            return ToolCallResult::coded("permission_denied", &call_id, None);
        }
        if let Some(hooks) = &self.hooks {
            hooks.before_call(name, &input_json);
        }
        let envelope = authorization_envelope(
            &grant_id,
            &self.scope.caller_app_id,
            &package,
            resource_id.as_deref(),
            &call_id,
            resource_scope,
        );
        let result = if self.host_tools.contains_key(name) {
            let Some(handler) = &self.host_handler else {
                return ToolCallResult::err(format!("host_tool_unhandled: no handler for {name:?}"));
            };
            log::info!(
                "tool_dispatch: caller={} → host tool {name:?} call_id={call_id:?} grant={grant_id}",
                self.caller_app_id
            );
            handler(name, &input_json)
        } else {
            self.dispatch_inner(call_id.clone(), name, input_json, Some(envelope))
        };
        if let Some(error) = &result.error {
            let code = result.error_code.as_deref().unwrap_or("error");
            let detail: String = error.chars().take(180).collect();
            crate::broker::gate::trace_gate(format!(
                "tool_dispatch: result code={code} caller={} tool={name} call_id={call_id} detail={detail}",
                self.scope.caller_app_id
            ));
        }
        let outcome = if result.error.is_none() { "ok" } else { "error" };
        let (revision_before, revision_after) =
            crate::broker::gate::revisions_from_output(result.output_json.as_deref());
        if self
            .monitor
            .note_outcome(
                &self.scope.caller_app_id,
                &call_id,
                &grant_id,
                &fingerprint,
                &resource,
                &operation_id,
                outcome,
                &revision_before,
                &revision_after,
            )
            .is_err()
            && result.error.is_none()
        {
            self.monitor
                .consume_once(&grant_id, Some(operation_id.as_str()).filter(|s| !s.is_empty()));
            let mut unknown = result;
            unknown.error_code = Some("outcome_unknown".to_string());
            unknown.error = Some(crate::broker::gate::structured_error(
                "outcome_unknown",
                &call_id,
                None,
            ));
            return unknown;
        }
        if result.error.is_none() {
            self.monitor
                .consume_once(&grant_id, Some(operation_id.as_str()).filter(|s| !s.is_empty()));
        }
        if let Some(hooks) = &self.hooks {
            hooks.after_call(name, result.error.as_deref(), result.output_json.as_deref());
        }
        result
    }

    /// Pick the exposed name for an app tool. One live instance keeps
    /// `<app>__<tool>`. Several instances require `<app>:<pane>__<tool>`.
    pub(crate) fn select_app_tool(
        &self,
        app_id: &str,
        tool: &str,
        target_pane: Option<u64>,
    ) -> Result<String, AppToolRouteError> {
        let plain = format!("{app_id}__{tool}");
        if let Some(pane) = target_pane {
            let qualified = format!("{app_id}:{pane}__{tool}");
            if self.tools.contains_key(&qualified) {
                log::info!("tool_dispatch: routed {plain} to pane {pane}");
                return Ok(qualified);
            }
            if let Some((provider, _, _)) = self.tools.get(&plain) {
                if *provider == pane {
                    log::info!("tool_dispatch: routed {plain} to pane {pane}");
                    return Ok(plain);
                }
            }
            log::info!("tool_dispatch: pane {pane} does not expose {plain}");
            return Err(AppToolRouteError::Missing {
                message: format!("tool_not_found: pane {pane} does not expose {tool}"),
            });
        }
        if self.tools.contains_key(&plain) {
            return Ok(plain);
        }
        let prefix = format!("{app_id}:");
        let suffix = format!("__{tool}");
        let mut matches: Vec<String> = self
            .tools
            .keys()
            .filter(|name| name.starts_with(&prefix) && name.ends_with(&suffix))
            .cloned()
            .collect();
        matches.sort();
        if matches.is_empty() {
            return Ok(plain);
        }
        let panes = pane_ids_in_qualified_names(&prefix, &matches);
        log::info!(
            "tool_dispatch: ambiguous_instance {plain} panes={panes:?}; address {}",
            matches.join(", ")
        );
        Err(AppToolRouteError::Ambiguous {
            message: format!(
                "ambiguous_instance: {plain} is live on multiple panes; address {}",
                matches.join(", ")
            ),
            panes,
        })
    }

    fn provider_of(&self, name: &str) -> (String, u64) {
        if let Some((pane, provider_name, _)) = self.tools.get(name) {
            let package = name
                .split_once("__")
                .map(|(app, _)| {
                    app.split(':')
                        .next()
                        .unwrap_or(app)
                        .to_string()
                })
                .unwrap_or_else(|| {
                    provider_name
                        .split('.')
                        .next()
                        .unwrap_or("app")
                        .to_string()
                });
            return (package, *pane);
        }
        ("host".to_string(), self.caller_pane_id)
    }

    fn dispatch_inner(
        &self,
        call_id: String,
        name: &str,
        input_json: String,
        authorization: Option<String>,
    ) -> ToolCallResult {
        let (pane_id, provider_tool_name, tool) = match self.tools.get(name) {
            Some(entry) => entry,
            None => {
                log::warn!(
                    "tool_dispatch: caller={} pane={} requested unknown tool {name:?} call_id={call_id:?}",
                    self.caller_app_id, self.caller_pane_id,
                );
                return ToolCallResult::err(format!(
                    "tool_not_found: no tool named {name:?} in registry"
                ));
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
                        authorization,
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
            return ToolCallResult::err(format!(
                "tool_pane_gone: pane {pane_id} for tool {name:?} is no longer registered"
            ));
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
                ToolCallResult::err(format!(
                    "tool_timeout: tool {name:?} did not respond within {timeout_ms}ms"
                ))
            }
        }
    }
}

fn authorization_envelope(
    grant_id: &str,
    actor: &str,
    package: &str,
    resource_id: Option<&str>,
    call_id: &str,
    resource_scope: crate::broker::ResourceScope,
) -> String {
    serde_json::json!({
        "schema_version": 1,
        "grant_id": grant_id,
        "actor": actor,
        "package": package,
        "game_id": resource_id,
        "resource_scope": format!("{resource_scope:?}").to_ascii_lowercase(),
        "call_id": call_id,
        "human_interaction": false,
    })
    .to_string()
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::AiTool;
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
        let dispatcher = ToolDispatcher::from_registry(
            DispatchScope::new(1, "app_a", "/ws-a", 100),
            crate::broker::gate::PermissionMonitor::ephemeral(),
        );

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

    /// Two panes exposing the same tool name stay addressable by pane id.
    /// The bare name is not offered, so dispatch cannot pick a winner.
    #[test]
    fn conflicting_tool_names_are_addressable_by_pane() {
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
            "the bare name must not pick a pane"
        );
        assert_eq!(snap["shared_tool@10"].0, 10);
        assert_eq!(snap["shared_tool@20"].0, 20);
        assert_eq!(snap["shared_tool@10"].1, "shared_tool");
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
    fn namespaced_snapshot_addresses_duplicate_app_instances_by_pane() {
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
        assert_eq!(snapshot["same-app:30__echo"].0, 30);
        assert_eq!(snapshot["same-app:20__echo"].0, 20);
        let (pane_id, provider_name, tool) = &snapshot["same-app__unique"];
        assert_eq!(*pane_id, 20);
        assert_eq!(provider_name, "unique");
        assert_eq!(tool.name, "same-app__unique");
    }

    /// A `before_call` error must block the call and skip `after_call`;
    /// unknown tools must return `tool_not_found` without invoking hooks.
    #[test]
    fn hooks_block_calls_and_skip_unknown_tools() {
        struct ObserveHook {
            seen: std::sync::Mutex<u32>,
        }
        impl ToolCallHooks for ObserveHook {
            fn before_call(&self, _name: &str, _input: &str) {
                *self.seen.lock().unwrap() += 1;
            }
            fn after_call(&self, _name: &str, _error: Option<&str>, _output: Option<&str>) {}
        }

        let (tx, _rx) = std::sync::mpsc::channel();
        register(
            998,
            "hooks-app".to_string(),
            vec![make_tool("gated_tool")],
            AppEventSender::Channel(tx),
            origin(70, 998),
        );
        let hook = Arc::new(ObserveHook {
            seen: std::sync::Mutex::new(0),
        });
        let mut dispatcher = ToolDispatcher::from_registry(
            DispatchScope::new(2, "agent:assistant", "/ws-hooks", 70),
            crate::broker::gate::PermissionMonitor::ephemeral(),
        );
        dispatcher.set_hooks(hook.clone());

        let blocked = dispatcher.dispatch_call("c1".to_string(), "gated_tool", "{}".to_string());
        assert_eq!(blocked.error_code.as_deref(), Some("permission_required"));
        assert_eq!(*hook.seen.lock().unwrap(), 0, "a refused call is not observed as started");

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
        let input = r#"{"value":7}"#;
        let monitor = crate::broker::gate::PermissionMonitor::ephemeral();
        let fp = crate::broker::gate::fingerprint_args(input).unwrap();
        monitor.store().record(crate::broker::GrantRecord::from_binding(
            &crate::broker::ExactBinding {
                actor_type: crate::broker::ActorType::Agent,
                actor_id: "agent:assistant".to_string(),
                actor_scope: crate::broker::ActorScope::User,
                trust_origin: "host".to_string(),
                workspace_root: PathBuf::from("/ws-wasm"),
                target_type: crate::broker::TargetType::AppConnector,
                target_id: "wasm.echo".to_string(),
                resource_scope: crate::broker::ResourceScope::Workspace,
                resource_id: None,
                args_fingerprint: fp,
                session_id: Some(monitor.session_id().to_string()),
                package_id: "wasm".to_string(),
                instance_id: Some(997),
                context_id: Some(80),
                call_id: "wasm-call-1".to_string(),
                operation_id: String::new(),
            },
            crate::broker::Decision::Allow,
            crate::broker::GrantDuration::Always,
            crate::broker::GrantSource::User,
            "g-wasm",
        ));
        let dispatcher = ToolDispatcher::from_registry(
            DispatchScope::new(2, "agent:assistant", "/ws-wasm", 80),
            monitor,
        );
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

        resolve_pending(&call.call_id, ToolCallResult::ok(r#"{"value":7}"#));
        let result = worker.join().unwrap();
        assert_eq!(result.output_json.as_deref(), Some(r#"{"value":7}"#));
        assert!(result.error.is_none());
        unregister(997);
    }

    #[test]
    fn denied_wasm_tool_call_never_reaches_the_guest() {
        let (sender, queue) = crate::host::wasm_pane::WasmInputSender::new_for_test();
        register(
            996,
            "wasm-denied".to_string(),
            vec![make_tool("wasm.write")],
            AppEventSender::Wasm(sender),
            origin(90, 996),
        );
        let dispatcher = ToolDispatcher::from_registry(
            DispatchScope::new(2, "agent:assistant", "/ws-denied", 90),
            crate::broker::gate::PermissionMonitor::ephemeral(),
        );

        let result = dispatcher.dispatch_call(
            "wasm-call-denied".to_string(),
            "wasm.write",
            "{}".to_string(),
        );
        assert_eq!(result.error_code.as_deref(), Some("permission_required"));
        assert!(
            queue.is_empty(),
            "denied calls must not enter WASM update()"
        );
        unregister(996);
    }
}
