//! Host-level MCP server — `stints 0214, 0653`.
//!
//! The native transport for MCP-aware agents (Claude Code, Codex). Unlike the
//! rejected per-app-server design, this is a single host-wide server started
//! once at boot. It exposes event subscription tools and the live app tools
//! registered through `ExposeTools`.
//!
//! Tools:
//! - `list_event_streams` — discover the `(app, stream)` pairs running apps declare.
//! - `subscribe_and_wait` — broker-checked subscribe, block for the next event,
//!   return it, then drop the subscription. A long-poll, so an agent can "wait
//!   for the next event and report it" in one tool call.
//! - `<app_id>__<tool>` — workspace-scoped tools registered by live app panes.
//!
//! Discovery: the bound port + bearer token are injected into every pane's PTY
//! env as `PLEXI_HOST_MCP_PORT` / `PLEXI_HOST_MCP_TOKEN`, so an agent in a pane
//! can configure this server without a wrapper subprocess.
//!
//! Persistence: the port is stored in `config_dir/host_mcp.json` and reused on
//! every launch. Bearers are pane-scoped runtime credentials: the host binds
//! each one to the trusted pane id + workspace root while building that pane's
//! environment. Requests never accept a client-provided workspace path.
//!
//! Identity: calls and subscriptions are stamped `mcp:pane:<id>` from the
//! authenticated credential, never from tool arguments.

use crate::app::ui_mailbox::UiMailbox;
use crate::host::event_subscriptions::{HostSubscribeReply, HostSubscribeRequest};
use crate::mcp_http::{read_json_rpc_request, write_http_response, RequestOutcome};
use std::collections::HashMap;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Hard cap on `subscribe_and_wait` blocking, kept under common MCP client
/// request timeouts. The client may request less via `timeout_secs`.
const MAX_WAIT_SECS: u64 = 55;
const DEFAULT_WAIT_SECS: u64 = 25;

/// Process-wide port for the singleton host MCP server.
static DISCOVERY: OnceLock<u16> = OnceLock::new();

fn discovery() -> Option<u16> {
    DISCOVERY.get().copied()
}

#[derive(Clone)]
struct McpCaller {
    pane_id: u64,
    /// Host-established context this pane lives in — stint 0724 Phase C.
    /// `from_namespaced_registry` resolves connector-tool reachability from
    /// this, never from `workspace_root` path-equality.
    context_id: u64,
    workspace_root: PathBuf,
}

#[derive(Default)]
struct CredentialRegistry {
    by_token: HashMap<String, McpCaller>,
    by_pane: HashMap<u64, String>,
}

static CREDENTIALS: OnceLock<Mutex<CredentialRegistry>> = OnceLock::new();

fn credentials() -> &'static Mutex<CredentialRegistry> {
    CREDENTIALS.get_or_init(|| Mutex::new(CredentialRegistry::default()))
}

/// Register or refresh the credential injected into one pane. The caller's
/// context and workspace come from host-owned pane construction, never the
/// MCP request.
fn register_pane_credential(pane_id: u64, context_id: u64, workspace_root: PathBuf) -> String {
    let mut registry = credentials().lock().unwrap();
    if let Some(token) = registry.by_pane.get(&pane_id).cloned() {
        registry.by_token.insert(
            token.clone(),
            McpCaller {
                pane_id,
                context_id,
                workspace_root,
            },
        );
        return token;
    }
    let token = uuid::Uuid::new_v4().to_string();
    registry.by_pane.insert(pane_id, token.clone());
    registry.by_token.insert(
        token.clone(),
        McpCaller {
            pane_id,
            context_id,
            workspace_root: workspace_root.clone(),
        },
    );
    log::info!(
        "host_mcp: registered scoped credential pane={pane_id} context={context_id} workspace={}",
        workspace_root.display()
    );
    token
}

/// Return the singleton endpoint plus a credential bound to this host-owned
/// pane identity. This is the only discovery surface pane environments use.
pub(crate) fn discovery_for_pane(
    pane_id: u64,
    context_id: u64,
    workspace_root: PathBuf,
) -> Option<(u16, String)> {
    discovery().map(|port| {
        let token = register_pane_credential(pane_id, context_id, workspace_root);
        (port, token)
    })
}

pub(crate) fn revoke_pane_credentials(pane_id: u64) {
    let mut registry = credentials().lock().unwrap();
    if let Some(token) = registry.by_pane.remove(&pane_id) {
        registry.by_token.remove(&token);
        log::info!("host_mcp: revoked scoped credential pane={pane_id}");
    }
}

/// Owns a credential between environment construction and installation of the
/// terminal pane. Dropping an uncommitted lease closes the credential, so every
/// early return from `TerminalPane::new` is safe by construction.
pub(crate) struct PendingPaneCredential {
    pane_id: Option<u64>,
}

impl PendingPaneCredential {
    pub(crate) fn new(pane_id: u64) -> Self {
        Self {
            pane_id: Some(pane_id),
        }
    }

    pub(crate) fn mark_live(mut self) {
        self.pane_id = None;
    }
}

impl Drop for PendingPaneCredential {
    fn drop(&mut self) {
        if let Some(pane_id) = self.pane_id.take() {
            revoke_pane_credentials(pane_id);
        }
    }
}

/// Update the trusted workspace attached to a live pane without rotating its
/// credential. Context roots can change while their panes remain alive — the
/// pane's `context_id` never changes on a root move, so it is carried forward
/// from the existing credential rather than taken as a parameter here.
pub(crate) fn rebind_pane_credential(pane_id: u64, workspace_root: PathBuf) {
    let mut registry = credentials().lock().unwrap();
    let Some(token) = registry.by_pane.get(&pane_id).cloned() else {
        return;
    };
    let Some(context_id) = registry
        .by_token
        .get(&token)
        .map(|caller| caller.context_id)
    else {
        return;
    };
    registry.by_token.insert(
        token,
        McpCaller {
            pane_id,
            context_id,
            workspace_root: workspace_root.clone(),
        },
    );
    log::info!(
        "host_mcp: rebound scoped credential pane={pane_id} context={context_id} workspace={}",
        workspace_root.display()
    );
}

fn authenticate(token: &str) -> Option<McpCaller> {
    credentials().lock().unwrap().by_token.get(token).cloned()
}

#[cfg(test)]
pub(crate) fn register_pane_credential_for_test(
    pane_id: u64,
    context_id: u64,
    workspace_root: PathBuf,
) -> String {
    register_pane_credential(pane_id, context_id, workspace_root)
}

#[cfg(test)]
pub(crate) fn authenticated_workspace_for_test(token: &str) -> Option<PathBuf> {
    authenticate(token).map(|caller| caller.workspace_root)
}

/// Persisted host-MCP endpoint (`config_dir/host_mcp.json`). Credentials are
/// deliberately runtime-only because they carry a live pane identity.
#[derive(serde::Serialize, serde::Deserialize)]
struct HostMcpIdentity {
    port: u16,
}

fn identity_path(config_dir: &Path) -> PathBuf {
    config_dir.join("host_mcp.json")
}

/// Load the persisted port if the file exists and parses. A corrupt file is
/// logged and ignored so the next launch rolls a fresh endpoint
/// rather than failing to start the transport.
fn load_identity(config_dir: &Path) -> Option<HostMcpIdentity> {
    let path = identity_path(config_dir);
    let bytes = std::fs::read(&path).ok()?;
    match serde_json::from_slice::<HostMcpIdentity>(&bytes) {
        Ok(id) => Some(id),
        Err(e) => {
            log::warn!("host_mcp: ignoring unparseable {}: {e}", path.display());
            None
        }
    }
}

/// Atomically persist the port. Best-effort: a write failure only means the
/// next launch rolls the endpoint, so it is logged rather than propagated.
fn save_identity(config_dir: &Path, port: u16) {
    let path = identity_path(config_dir);
    let body = match serde_json::to_vec_pretty(&HostMcpIdentity { port }) {
        Ok(b) => b,
        Err(e) => {
            log::warn!("host_mcp: could not serialize identity: {e}");
            return;
        }
    };
    if let Err(e) = crate::platform::fs::atomic_write_with_mode(&path, &body, 0o600) {
        log::warn!("host_mcp: could not write {}: {e}", path.display());
    }
}

/// Start the host MCP server. `subscribe_tx` routes subscribe requests to the
/// UI thread (which owns the grant store); the mailbox wakes the UI so it
/// drains the subscribe channel promptly even while idle. `config_dir` is the
/// profile dir where the port is persisted across restarts.
/// Idempotent at the discovery level — the first successful bind wins.
pub fn start_host_mcp_server(
    subscribe_tx: UiMailbox<HostSubscribeRequest>,
    config_dir: &Path,
) -> std::io::Result<u16> {
    let persisted = load_identity(config_dir);
    // Reuse the persisted port so a static MCP URL keeps resolving. Fall back to
    // an OS-assigned port if it is taken (e.g. another channel grabbed it while
    // this instance was down) and re-persist whatever we actually bound.
    let listener = match persisted.as_ref().map(|id| id.port) {
        Some(p) => TcpListener::bind(("127.0.0.1", p)).or_else(|e| {
            log::warn!(
                "host_mcp: persisted port {p} unavailable ({e}); falling back to an ephemeral port"
            );
            TcpListener::bind("127.0.0.1:0")
        })?,
        None => TcpListener::bind("127.0.0.1:0")?,
    };
    let port = listener.local_addr()?.port();

    save_identity(config_dir, port);

    let _ = DISCOVERY.set(port);
    // The directory this server was started with is the grant profile. Callers
    // pass `config_dir()` in production. Connection threads must not call
    // `config_dir()` themselves: a test's profile override is thread-local.
    let profile = config_dir.to_path_buf();

    std::thread::Builder::new()
        .name(format!("host-mcp-accept-{port}"))
        .spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        let subscribe_tx = subscribe_tx.clone();
                        let profile = profile.clone();
                        std::thread::Builder::new()
                            .name("host-mcp-conn".to_string())
                            .spawn(move || {
                                if let Err(e) = handle_connection(stream, &subscribe_tx, &profile) {
                                    log::warn!("host_mcp: connection error: {e}");
                                }
                            })
                            .ok();
                    }
                    Err(e) => {
                        log::warn!("host_mcp: accept error: {e}");
                        break;
                    }
                }
            }
        })
        .map_err(std::io::Error::other)?;

    log::info!("host_mcp: started on 127.0.0.1:{port}");
    Ok(port)
}

fn tool_defs(dispatcher: &crate::plexi_ai::tool_dispatch::ToolDispatcher) -> serde_json::Value {
    let mut tools = vec![
        serde_json::json!({
            "name": "permissions_list",
            "description": "List live permission decisions from the permission monitor. Same rows as `plexi permissions list`.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        }),
        serde_json::json!({
            "name": "permissions_revoke",
            "description": "Remove an allow or refuse a pending ask. Narrows authority.",
            "inputSchema": {
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"],
                "additionalProperties": false
            }
        }),
        serde_json::json!({
            "name": "permissions_reset",
            "description": "Ask to clear a stored denial. An MCP caller files Needs you and does not clear it.",
            "inputSchema": {
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"],
                "additionalProperties": false
            }
        }),
        serde_json::json!({
            "name": "permissions_allow",
            "description": "Ask to allow a denial or a pending ask. An MCP caller files Needs you and does not grant it.",
            "inputSchema": {
                "type": "object",
                "properties": { "id": { "type": "string" } },
                "required": ["id"],
                "additionalProperties": false
            }
        }),
        serde_json::json!({
            "name": "list_event_streams",
            "description": "List the event streams currently declared by running Plexi apps. Returns an array of {app_id, stream} pairs.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        }),
        serde_json::json!({
            "name": "subscribe_and_wait",
            "description": "Subscribe to a Plexi app's event stream and block until the next event arrives (or the timeout elapses), then return it. The subscription is dropped when the call returns.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "app_id": { "type": "string", "description": "App that publishes the stream, e.g. event-probe." },
                    "stream": { "type": "string", "description": "Stream name, e.g. probe.tick. Omit with all=true to subscribe to every stream." },
                    "all": { "type": "boolean", "description": "Subscribe to all of the app's declared streams." },
                    "payload": { "type": "string", "enum": ["off", "summary", "full", "state_ref"], "description": "How much of the event to deliver (default full)." },
                    "timeout_secs": { "type": "integer", "description": "Max seconds to wait for the next event (default 25, max 55)." }
                },
                "required": ["app_id"],
                "additionalProperties": false
            }
        }),
    ];
    let mut app_tools = dispatcher.all_tools();
    app_tools.sort_by(|a, b| a.name.cmp(&b.name));
    tools.extend(app_tools.into_iter().map(|tool| {
        serde_json::json!({
            "name": tool.name,
            "description": tool.description,
            "inputSchema": tool.input_schema,
            "outputSchema": tool.output_schema,
        })
    }));
    serde_json::Value::Array(tools)
}

fn handle_connection(
    stream: std::net::TcpStream,
    subscribe_tx: &UiMailbox<HostSubscribeRequest>,
    profile: &std::path::Path,
) -> std::io::Result<()> {
    let peer = stream
        .peer_addr()
        .map(|a| a.to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    let mut write_stream = stream.try_clone()?;

    let (json, caller) = match read_json_rpc_request(&stream, authenticate)? {
        RequestOutcome::Json { body, auth } => (body, auth),
        RequestOutcome::Handled => return Ok(()),
    };
    let dispatcher = crate::plexi_ai::tool_dispatch::ToolDispatcher::from_namespaced_registry(
        crate::plexi_ai::tool_dispatch::DispatchScope::new(
            caller.pane_id,
            format!("mcp:pane:{}", caller.pane_id),
            caller.workspace_root.clone(),
            caller.context_id,
        ),
        crate::broker::gate::PermissionMonitor::for_profile(profile),
    );

    let id = json.get("id").cloned().unwrap_or(serde_json::Value::Null);
    let method_name = json.get("method").and_then(|m| m.as_str()).unwrap_or("");

    let response_body = match method_name {
        "initialize" => serde_json::json!({
            "jsonrpc": "2.0", "id": id,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "plexi-host", "version": "1.0.0" }
            }
        }),
        "notifications/initialized" => serde_json::json!({
            "jsonrpc": "2.0", "id": serde_json::Value::Null, "result": serde_json::Value::Null
        }),
        "tools/list" => serde_json::json!({
            "jsonrpc": "2.0", "id": id, "result": { "tools": tool_defs(&dispatcher) }
        }),
        "tools/call" => {
            let params = json.get("params").cloned().unwrap_or_default();
            let tool_name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or(serde_json::Value::Object(Default::default()));
            log::info!(
                "host_mcp: tool_call caller_pane={} workspace={} tool={tool_name} peer={peer}",
                caller.pane_id,
                caller.workspace_root.display()
            );
            let result = match tool_name {
                "permissions_list" => tool_permissions("list", &arguments, profile, &caller),
                "permissions_revoke" => tool_permissions("revoke", &arguments, profile, &caller),
                "permissions_reset" => tool_permissions("reset", &arguments, profile, &caller),
                "permissions_allow" => tool_permissions("allow", &arguments, profile, &caller),
                "list_event_streams" => tool_list_event_streams(&caller),
                "subscribe_and_wait" => tool_subscribe_and_wait(&arguments, subscribe_tx, &caller),
                other => {
                    let input_json = serde_json::to_string(&arguments)
                        .map_err(|error| format!("serialize tool arguments: {error}"));
                    match input_json {
                        Ok(input_json) => {
                            let call_id = format!("mcp-{}", uuid::Uuid::new_v4());
                            let result = dispatcher.dispatch_call(call_id, other, input_json);
                            match (result.output_json, result.error) {
                                (_, Some(error)) => Err(error),
                                (Some(output), None) => Ok(output),
                                (None, None) => Ok("null".to_string()),
                            }
                        }
                        Err(error) => Err(error),
                    }
                }
            };
            match result {
                Ok(text) => serde_json::json!({
                    "jsonrpc": "2.0", "id": id,
                    "result": { "content": [{ "type": "text", "text": text }], "isError": false }
                }),
                Err(msg) => serde_json::json!({
                    "jsonrpc": "2.0", "id": id,
                    "result": { "content": [{ "type": "text", "text": msg }], "isError": true }
                }),
            }
        }
        other => serde_json::json!({
            "jsonrpc": "2.0", "id": id,
            "error": { "code": -32601, "message": format!("method not found: {other}") }
        }),
    };

    let body_bytes = serde_json::to_vec(&response_body).unwrap_or_else(|_| b"{}".to_vec());
    write_http_response(&mut write_stream, 200, &body_bytes)
}

/// Permission-monitor tools. MCP callers are agents: revoke narrows, reset and
/// allow file Needs you and change nothing.
fn tool_permissions(
    op: &str,
    arguments: &serde_json::Value,
    profile: &std::path::Path,
    caller: &McpCaller,
) -> Result<String, String> {
    let monitor = crate::broker::gate::PermissionMonitor::for_profile(profile);
    if op == "list" {
        let entries = monitor.list_entries();
        log::info!(
            "host_mcp: permissions_list count={} pane={}",
            entries.len(),
            caller.pane_id
        );
        return serde_json::to_string(&serde_json::json!({"ok": true, "entries": entries}))
            .map_err(|error| error.to_string());
    }
    let id = arguments
        .get("id")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let outcome = monitor.mutate_entry(
        id,
        op,
        &crate::broker::gate::PermissionCaller {
            human: false,
            actor_id: format!("mcp:pane:{}", caller.pane_id),
        },
    );
    log::info!("host_mcp: permissions_{op} id={id} pane={}", caller.pane_id);
    let mut body = serde_json::to_value(&outcome).unwrap_or(serde_json::Value::Null);
    let ok = matches!(
        outcome,
        crate::broker::gate::PermissionMutation::Applied { .. }
    );
    if let Some(map) = body.as_object_mut() {
        map.insert("ok".to_string(), serde_json::json!(ok));
    }
    let text = serde_json::to_string(&body).unwrap_or_else(|_| "{}".to_string());
    if ok { Ok(text) } else { Err(text) }
}

/// `list_event_streams` — read declared streams from the global timeline,
/// filtered to the caller's own context (stint 0724 Phase D): a stream
/// declared only in another context can never be subscribed to from here
/// anyway (no cross-context grant exists), so listing it would be
/// misleading.
fn tool_list_event_streams(caller: &McpCaller) -> Result<String, String> {
    let streams = crate::host::app_timeline::global()
        .lock()
        .unwrap()
        .all_declared_streams();
    let arr: Vec<serde_json::Value> = streams
        .iter()
        .filter(|(context_id, _, _)| *context_id == caller.context_id)
        .map(|(_, a, s)| serde_json::json!({ "app_id": a, "stream": s }))
        .collect();
    serde_json::to_string(&serde_json::json!({ "streams": arr }))
        .map_err(|e| format!("serialize failed: {e}"))
}

/// `subscribe_and_wait` — broker-checked subscribe, block for the next event,
/// return it, then clear the subscription.
fn tool_subscribe_and_wait(
    args: &serde_json::Value,
    subscribe_tx: &UiMailbox<HostSubscribeRequest>,
    caller: &McpCaller,
) -> Result<String, String> {
    let app_id = args
        .get("app_id")
        .and_then(|v| v.as_str())
        .ok_or("missing required argument: app_id")?
        .to_string();
    let all = args.get("all").and_then(|v| v.as_bool()).unwrap_or(false);
    let stream = args.get("stream").and_then(|v| v.as_str());
    let event_names: Vec<String> = match (all, stream) {
        (true, _) => vec![],
        (false, Some(s)) => vec![s.to_string()],
        (false, None) => {
            return Err("provide a stream name or set all=true".to_string());
        }
    };
    let payload_mode = match args.get("payload").and_then(|v| v.as_str()) {
        Some("off") => crate::protocol::PayloadMode::Off,
        Some("summary") => crate::protocol::PayloadMode::Summary,
        Some("state_ref") => crate::protocol::PayloadMode::StateRef,
        _ => crate::protocol::PayloadMode::Full,
    };
    let timeout_secs = args
        .get("timeout_secs")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_WAIT_SECS)
        .clamp(1, MAX_WAIT_SECS);

    // Route the broker-checked subscribe through the UI thread.
    //
    // The authenticated pane identity owns both broker provenance and workspace
    // scope. A unique delivery suffix prevents concurrent waiters from
    // cross-talking or tearing each other down.
    let actor_id = format!("mcp:pane:{}", caller.pane_id);
    let delivery_id = format!("{actor_id}:{}", uuid::Uuid::new_v4());
    let (reply_tx, reply_rx) = std::sync::mpsc::sync_channel::<HostSubscribeReply>(1);
    let req = HostSubscribeRequest {
        publisher_app_id: app_id.clone(),
        event_names,
        payload_mode,
        trigger_mode: crate::protocol::TriggerMode::Conversation,
        resource_id: None,
        from_pane_id: Some(caller.pane_id),
        subscriber_override: Some(delivery_id),
        subscriber_type_override: None,
        broker_actor_override: Some(actor_id),
        workspace_root_override: Some(caller.workspace_root.clone()),
        context_id_override: Some(caller.context_id),
        // The authenticated bearer credential already establishes this
        // caller's identity (bound to `caller.pane_id`/`context_id` at
        // registration — see `register_pane_credential`); there is no raw
        // socket peer to independently verify here.
        peer_ancestry: None,
        cancelled: None,
        reply: reply_tx,
    };
    subscribe_tx
        .send(req)
        .map_err(|_| "host not accepting subscriptions".to_string())?;

    // A first-time MCP subscribe under default `Ask` posture blocks here until
    // the user answers the host consent modal. Kept short enough that the
    // consent wait plus the long-poll stay under common MCP client timeouts;
    // once the user picks "Always" the grant persists and this is instant.
    let (subscriber_type, subscriber_id) = match reply_rx.recv_timeout(Duration::from_secs(30)) {
        Ok(HostSubscribeReply::Ok {
            subscriber_type,
            subscriber_id,
            ..
        }) => (subscriber_type, subscriber_id),
        Ok(HostSubscribeReply::Err { message }) => return Err(message),
        Err(_) => return Err("subscribe consent timed out".to_string()),
    };

    // Long-poll the global timeline for the next delivery.
    let timeline = crate::host::app_timeline::global();
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    let mut first = None;
    while Instant::now() < deadline {
        let deliveries = timeline
            .lock()
            .unwrap()
            .take_deliveries_for(subscriber_type, &subscriber_id);
        if let Some(d) = deliveries.into_iter().next() {
            first = Some(d);
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    // One-shot: drop the subscription and any extra queued deliveries.
    timeline
        .lock()
        .unwrap()
        .clear_subscriber(subscriber_type, &subscriber_id);

    match first {
        Some(d) => {
            let out = serde_json::json!({
                "app_id": d.app_id,
                "event": d.event,
                "event_id": d.event_id,
                "resource_id": d.resource_id,
                "summary": d.summary,
                "payload": d.payload,
                "state_ref": d.state_ref,
                "created_at": d.created_at,
            });
            log::info!(
                "host_mcp: subscribe_and_wait delivered event {} from {}",
                d.event,
                d.app_id
            );
            serde_json::to_string(&out).map_err(|e| format!("serialize failed: {e}"))
        }
        None => Ok(format!(
            "{{\"timeout\":true,\"message\":\"no event within {timeout_secs}s\"}}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpStream;

    fn post(port: u16, token: Option<&str>, body: &[u8]) -> (u16, Vec<u8>) {
        let mut stream = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
        let auth = match token {
            Some(t) => format!("Authorization: Bearer {t}\r\n"),
            None => String::new(),
        };
        let req = format!(
            "POST /mcp HTTP/1.1\r\nHost: localhost\r\n{auth}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(req.as_bytes()).unwrap();
        stream.write_all(body).unwrap();
        stream.flush().unwrap();
        let mut resp = Vec::new();
        let _ = stream.read_to_end(&mut resp);
        let s = String::from_utf8_lossy(&resp);
        let status = s
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|x| x.parse().ok())
            .unwrap_or(0);
        let start = s.find("\r\n\r\n").map(|i| i + 4).unwrap_or(resp.len());
        (status, resp[start..].to_vec())
    }

    /// A test server whose subscribe channel is serviced by a granted service
    /// bound to the global timeline (the production wiring, minus the UI loop).
    fn start_test_server(grant_target: Option<&str>) -> (u16, String, std::path::PathBuf) {
        static NEXT_PANE_ID: std::sync::atomic::AtomicU64 =
            std::sync::atomic::AtomicU64::new(80_000);
        use crate::broker::{
            ActorScope, ActorType, GrantDuration, GrantRecord, GrantSource,
        };
        let pane_id = NEXT_PANE_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let actor_id = format!("mcp:pane:{pane_id}");
        let workspace_root = PathBuf::from(format!("/workspace/host-mcp-test-{pane_id}"));
        let (tx, rx) = UiMailbox::<HostSubscribeRequest>::channel(
            std::sync::Arc::new(crate::app::ui_mailbox::RecordingWake::new()),
            "host_mcp_test",
        );
        let mut store = crate::broker::GrantStore::default();
        if let Some(target) = grant_target {
            store.record(GrantRecord::event_stream_allow(
                ActorType::Agent,
                &actor_id,
                ActorScope::User,
                target,
                &workspace_root,
                GrantDuration::Always,
                GrantSource::User,
                None,
            ));
        }
        let svc = crate::host::event_subscriptions::HostSubscriptionService::new_for_test(
            store,
            crate::host::app_timeline::global(),
        );
        std::thread::spawn(move || {
            while let Ok(req) = rx.recv() {
                // Pre-granted in these tests, so classify answers immediately;
                // an `Ask` would return a parked consent we have no UI to
                // resolve, so it is dropped (the transport sees a closed reply).
                let _ = svc.classify_subscribe_request(req);
            }
        });
        // Each test server gets an isolated profile dir so its persisted
        // endpoint never collides with a sibling test's port.
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().to_path_buf();
        let port = start_host_mcp_server(tx, &profile).unwrap();
        // The accept thread keeps using this directory after the test returns.
        std::mem::forget(dir);
        let token = register_pane_credential(pane_id, 1, workspace_root);
        (port, token, profile)
    }

    #[test]
    fn identity_round_trips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        save_identity(dir.path(), 4242);
        let loaded = load_identity(dir.path()).expect("identity file written");
        assert_eq!(loaded.port, 4242);
    }

    #[test]
    fn pending_pane_credentials_revoke_on_abort_and_survive_commit() {
        let aborted_ids = [65_300_301u64, 65_300_302u64];
        let aborted_tokens: Vec<_> = aborted_ids
            .iter()
            .map(|pane_id| {
                register_pane_credential(
                    *pane_id,
                    1,
                    PathBuf::from(format!("/workspace/aborted-{pane_id}")),
                )
            })
            .collect();
        let pending: Vec<_> = aborted_ids
            .iter()
            .map(|pane_id| PendingPaneCredential::new(*pane_id))
            .collect();

        drop(pending);

        assert!(aborted_tokens
            .iter()
            .all(|token| authenticate(token).is_none()));

        let live_id = 65_300_303u64;
        let live_root = PathBuf::from("/workspace/live");
        let live_token = register_pane_credential(live_id, 1, live_root.clone());
        PendingPaneCredential::new(live_id).mark_live();

        assert_eq!(
            authenticate(&live_token).map(|caller| caller.workspace_root),
            Some(live_root)
        );
        revoke_pane_credentials(live_id);
    }

    #[test]
    fn persisted_endpoint_contains_no_bearer() {
        let dir = tempfile::tempdir().unwrap();
        save_identity(dir.path(), 4242);
        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(identity_path(dir.path())).unwrap()).unwrap();
        assert_eq!(json["port"], 4242);
        assert!(json.get("token").is_none());
    }

    #[test]
    fn no_auth_returns_401() {
        let (port, _t, _profile) = start_test_server(None);
        let (status, _) = post(
            port,
            None,
            br#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
        );
        assert_eq!(status, 401);
    }

    #[test]
    fn tools_list_exposes_subscription_tools() {
        let (port, token, _profile) = start_test_server(None);
        let (status, body) = post(
            port,
            Some(&token),
            br#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
        );
        assert_eq!(status, 200);
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let names: Vec<&str> = json["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t["name"].as_str())
            .collect();
        assert!(names.contains(&"list_event_streams"));
        assert!(names.contains(&"subscribe_and_wait"));
    }

    /// A `ScopeOrigin` for a tool provider pane — mirrors
    /// `tool_dispatch::tests::origin` (private to that module), duplicated
    /// here since this test lives in a different module. Only `context_id`
    /// and `pane_id` matter to `evaluate_reach`.
    fn test_provider_origin(context_id: u64, pane_id: u64) -> crate::host::scope::ScopeOrigin {
        crate::host::scope::ScopeOrigin {
            context_id,
            context_root: PathBuf::from(format!("/ctx-root-{context_id}")),
            window_id: context_id,
            pane_id,
            app_id: None,
        }
    }

    #[test]
    fn pane_credential_lists_and_calls_only_context_app_tools() {
        use crate::protocol::{AiTool, PlexiEvent};
        use crate::plexi_ai::tool_dispatch::{self, AppEventSender, ToolCallResult};

        let (port, _test_token, profile) = start_test_server(None);
        let context_a = 100u64;
        let context_b = 200u64; // a different context — must stay unreachable
        let workspace_a = PathBuf::from("/workspace/host-mcp-a");
        let caller_token = register_pane_credential(7_001, context_a, workspace_a.clone());
        let (provider_tx, provider_rx) = std::sync::mpsc::channel();
        let (other_tx, _other_rx) = std::sync::mpsc::channel();
        let tool = |name: &str| AiTool {
            name: name.to_string(),
            description: format!("test tool {name}"),
            input_schema: serde_json::json!({"type": "object"}),
            output_schema: serde_json::json!({"type": "object"}),
            timeout_ms: Some(1_000),
            read_only: true,
        };
        tool_dispatch::register(
            7_002,
            "workspace-a-app".to_string(),
            vec![tool("echo")],
            AppEventSender::Channel(provider_tx),
            test_provider_origin(context_a, 7_002),
        );
        tool_dispatch::register(
            7_003,
            "workspace-b-app".to_string(),
            vec![tool("secret")],
            AppEventSender::Channel(other_tx),
            test_provider_origin(context_b, 7_003),
        );

        let (status, body) = post(
            port,
            Some(&caller_token),
            br#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
        );
        assert_eq!(status, 200);
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let names: Vec<&str> = json["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect();
        assert!(names.contains(&"workspace-a-app__echo"));
        assert!(!names.contains(&"workspace-b-app__secret"));

        let (status, body) = post(
            port,
            Some(&caller_token),
            br#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"workspace-b-app__secret","arguments":{"workspace_root":"/workspace/host-mcp-b"}}}"#,
        );
        assert_eq!(status, 200);
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["result"]["isError"], true);
        assert!(
            json["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("tool_not_found"),
            "client arguments cannot override credential workspace: {json}"
        );

        {
            use crate::broker::{
                ActorScope, ActorType, Decision, ExactBinding, GrantDuration, GrantRecord,
                GrantSource, ResourceScope, TargetType,
            };
            let args = r#"{"value":7}"#;
            let binding = ExactBinding {
                actor_type: ActorType::App,
                actor_id: "mcp:pane:7001".to_string(),
                actor_scope: ActorScope::User,
                trust_origin: "host".to_string(),
                workspace_root: workspace_a.clone(),
                target_type: TargetType::AppConnector,
                target_id: "workspace-a-app__echo".to_string(),
                resource_scope: ResourceScope::Workspace,
                resource_id: None,
                args_fingerprint: crate::broker::gate::fingerprint_args(args).unwrap(),
                session_id: None,
                package_id: "workspace-a-app".to_string(),
                instance_id: Some(7_002),
                context_id: Some(context_a),
                call_id: String::new(),
                operation_id: String::new(),
            };
            crate::broker::gate::PermissionMonitor::for_profile(&profile)
                .store()
                .record(GrantRecord::from_binding(
                    &binding,
                    Decision::Allow,
                    GrantDuration::Always,
                    GrantSource::User,
                    "grant-mcp-echo",
                ));
        }
        let responder = std::thread::spawn(move || {
            let line = provider_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("provider receives ToolCall");
            let event: PlexiEvent = serde_json::from_str(line.trim()).unwrap();
            let PlexiEvent::ToolCall {
                call_id,
                name,
                input_json,
                caller_id,
                authorization: _,
            } = event
            else {
                panic!("expected ToolCall");
            };
            assert_eq!(name, "echo");
            assert_eq!(input_json, r#"{"value":7}"#);
            assert_eq!(caller_id, "mcp:pane:7001");
            tool_dispatch::resolve_pending(&call_id, ToolCallResult::ok(r#"{"echo":7}"#));
        });
        let (status, body) = post(
            port,
            Some(&caller_token),
            br#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"workspace-a-app__echo","arguments":{"value":7}}}"#,
        );
        responder.join().unwrap();
        assert_eq!(status, 200);
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            json["result"]["content"][0]["text"],
            serde_json::json!(r#"{"echo":7}"#)
        );
        assert_eq!(json["result"]["isError"], false);

        tool_dispatch::unregister(7_002);
        tool_dispatch::unregister(7_003);
        revoke_pane_credentials(7_001);
        let (status, _) = post(
            port,
            Some(&caller_token),
            br#"{"jsonrpc":"2.0","id":4,"method":"tools/list"}"#,
        );
        assert_eq!(status, 401, "closed-pane credentials must be revoked");
    }

    /// A real Pi session reaches a context app tool through the Plexi-managed
    /// Pi extension and this server — no second protocol. Pi's model is its
    /// scripted `faux` provider, so the run needs no API key; the tool call
    /// still crosses Pi's MCP client, the pane-scoped bearer, and
    /// `ToolDispatcher` into the provider pane.
    ///
    /// Needs a Pi install (not in CI):
    /// `PLEXI_PI_CLI=<pi-coding-agent>/dist/bundle/cli.js PLEXI_PI_PROBE_DIR=<dir with
    /// node_modules/@earendil-works/pi-coding-agent> cargo test --bin plexi
    /// pi_session_calls_context_app_tool -- --ignored`. `PLEXI_PI_RUNTIME`
    /// defaults to `bun` (Pi's bundle needs Node >= 22.8 otherwise).
    #[test]
    #[ignore = "requires a Pi install; see doc comment"]
    fn pi_session_calls_context_app_tool_through_host_mcp() {
        use crate::protocol::{AiTool, PlexiEvent};
        use crate::plexi_ai::tool_dispatch::{self, AppEventSender, ToolCallResult};
        use std::process::{Command, Stdio};

        let pi_cli = std::env::var("PLEXI_PI_CLI").expect("set PLEXI_PI_CLI to Pi's cli.js");
        let probe_dir = PathBuf::from(
            std::env::var("PLEXI_PI_PROBE_DIR")
                .expect("set PLEXI_PI_PROBE_DIR to a dir whose node_modules has Pi"),
        );
        let runtime = std::env::var("PLEXI_PI_RUNTIME").unwrap_or_else(|_| "bun".to_string());

        let (port, _test_token, _profile) = start_test_server(None);
        let context = 300u64;
        let caller_pane = 7_101u64;
        let token = register_pane_credential(
            caller_pane,
            context,
            PathBuf::from("/workspace/host-mcp-pi"),
        );
        let (provider_tx, provider_rx) = std::sync::mpsc::channel();
        let (other_tx, _other_rx) = std::sync::mpsc::channel();
        let tool = |name: &str| AiTool {
            name: name.to_string(),
            description: format!("probe tool {name}"),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {"square": {"type": "string"}},
            }),
            output_schema: serde_json::json!({"type": "object"}),
            timeout_ms: Some(5_000),
            read_only: false,
        };
        tool_dispatch::register(
            7_102,
            "pi-probe".to_string(),
            vec![tool("move")],
            AppEventSender::Channel(provider_tx),
            test_provider_origin(context, 7_102),
        );
        tool_dispatch::register(
            7_103,
            "pi-other".to_string(),
            vec![tool("secret")],
            AppEventSender::Channel(other_tx),
            test_provider_origin(context + 1, 7_103),
        );

        // Pi's MCP client will make this same request on startup. Assert the
        // host boundary first so the real-session half below proves the call
        // crosses that already-scoped boundary rather than a parallel path.
        let (status, body) = post(
            port,
            Some(&token),
            br#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
        );
        assert_eq!(status, 200);
        let listed_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let listed: Vec<&str> = listed_json["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect();
        assert!(listed.contains(&"pi-probe__move"));
        assert!(!listed.contains(&"pi-other__secret"));

        let responder = std::thread::spawn(move || {
            let line = provider_rx
                .recv_timeout(Duration::from_secs(60))
                .expect("provider pane receives ToolCall from Pi");
            let event: PlexiEvent = serde_json::from_str(line.trim()).unwrap();
            let PlexiEvent::ToolCall {
                call_id,
                name,
                input_json,
                caller_id,
                authorization: _,
            } = event
            else {
                panic!("expected ToolCall");
            };
            tool_dispatch::resolve_pending(
                &call_id,
                ToolCallResult::ok(r#"{"moved":"e4","revision":2}"#),
            );
            (name, input_json, caller_id)
        });

        let dir = tempfile::tempdir_in(&probe_dir).unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let plexi_ext = dir.path().join("plexi.ts");
        std::fs::write(&plexi_ext, crate::cli::agent::pi_extension_script("true")).unwrap();
        let faux_ext = dir.path().join("faux.ts");
        std::fs::write(
            &faux_ext,
            r#"import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { fauxProvider, fauxAssistantMessage, fauxToolCall } from "@earendil-works/pi-ai";

export default function (pi: ExtensionAPI) {
  const faux = fauxProvider({ provider: "plexi-probe", models: [{ id: "probe" }] });
  faux.setResponses([
    (context) => {
      return fauxAssistantMessage(
        [fauxToolCall("mcp__plexi__pi_probe__move", { square: "e4" })],
        { stopReason: "toolUse" },
      );
    },
    (context) => {
      const result = [...context.messages].reverse().find((m: any) => m.role === "toolResult") as any;
      const text = result?.content?.map((c: any) => c.text ?? "").join("") ?? "<none>";
      return fauxAssistantMessage(`PROBE_RESULT ${result?.isError ? "error" : "ok"} ${text}`);
    },
  ]);
  pi.registerProvider(faux.provider);
}
"#,
        )
        .unwrap();

        let output = Command::new(&runtime)
            .arg(&pi_cli)
            .args(["-p", "--no-session", "--offline", "-ne", "-e", "builtin:mcp", "-e"])
            .arg(&plexi_ext)
            .arg("-e")
            .arg(&faux_ext)
            .args(["--provider", "plexi-probe", "--model", "probe", "move e4"])
            .current_dir(dir.path())
            .env("HOME", &home)
            .env("PLEXI_HOST_MCP_PORT", port.to_string())
            .env("PLEXI_HOST_MCP_TOKEN", &token)
            .env_remove("PLEXI_SOCKET")
            .env_remove("PLEXI_PANE_ID")
            .stdin(Stdio::null())
            .output()
            .expect("spawn Pi");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "pi failed\nstdout:\n{stdout}\nstderr:\n{stderr}");

        let (name, input_json, caller_id) = responder.join().unwrap();
        assert_eq!(name, "move");
        assert_eq!(input_json, r#"{"square":"e4"}"#);
        assert_eq!(caller_id, format!("mcp:pane:{caller_pane}"));
        assert!(
            stdout.contains(r#"PROBE_RESULT ok {"moved":"e4","revision":2}"#),
            "tool result must reach Pi's model: {stdout}\n{stderr}"
        );

        tool_dispatch::unregister(7_102);
        tool_dispatch::unregister(7_103);
        revoke_pane_credentials(caller_pane);
    }

    /// End-to-end proof the MCP adapter is wired, not just that a host test can
    /// route in-process: subscribe via the tool, emit on the timeline, and the
    /// tool returns the event.
    #[test]
    fn subscribe_and_wait_delivers_emitted_event() {
        use crate::protocol::{AppEventActor, EventStreamDecl};
        use crate::host::app_timeline::EmittedEvent;
        let app = "mcp-it-app";
        let stream = "it.tick";
        // `start_test_server` registers its pane credential with `context_id
        // = 1` (see `register_pane_credential(pane_id, 1, ...)` below), so
        // the stream must be declared under that same context for the
        // subscribe to see it as declared (stint 0724 Phase D).
        crate::host::app_timeline::global()
            .lock()
            .unwrap()
            .declare_streams(
                1,
                app,
                vec![EventStreamDecl {
                    name: stream.to_string(),
                    schema: serde_json::json!({"type": "object"}),
                    description: None,
                }],
            )
            .unwrap();
        let (port, token, _profile) = start_test_server(Some(&format!("{app}::{stream}")));

        // Emit shortly after the tool call begins its long-poll.
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            crate::host::app_timeline::global()
                .lock()
                .unwrap()
                .record_event(
                    1,
                    app,
                    1,
                    EmittedEvent {
                        event: stream.to_string(),
                        actor: AppEventActor::User,
                        actor_id: None,
                        caused_by: None,
                        summary: "it tick 1".to_string(),
                        resource_id: "it-session".to_string(),
                        resource_scope: Some("document".to_string()),
                        revision_after: "tick-1".to_string(),
                        payload: Some(serde_json::json!({"count": 1})),
                        state_ref: None,
                        revision_before: None,
                        rollback_token: None,
                        changed_resources: vec![],
                        suggested_trigger: None,
                    },
                )
                .unwrap();
        });

        let call = format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"subscribe_and_wait","arguments":{{"app_id":"{app}","stream":"{stream}","timeout_secs":5}}}}}}"#
        );
        let (status, body) = post(port, Some(&token), call.as_bytes());
        assert_eq!(status, 200);
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let text = json["result"]["content"][0]["text"].as_str().unwrap();
        let event: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(event["event"], stream);
        assert_eq!(event["summary"], "it tick 1");
        assert_eq!(event["payload"]["count"], 1);
    }
}
