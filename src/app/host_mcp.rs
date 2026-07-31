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
/// workspace comes from host-owned pane construction, never the MCP request.
fn register_pane_credential(pane_id: u64, workspace_root: PathBuf) -> String {
    let mut registry = credentials().lock().unwrap();
    if let Some(token) = registry.by_pane.get(&pane_id).cloned() {
        registry.by_token.insert(
            token.clone(),
            McpCaller {
                pane_id,
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
            workspace_root: workspace_root.clone(),
        },
    );
    log::info!(
        "host_mcp: registered scoped credential pane={pane_id} workspace={}",
        workspace_root.display()
    );
    token
}

/// Return the singleton endpoint plus a credential bound to this host-owned
/// pane identity. This is the only discovery surface pane environments use.
pub(crate) fn discovery_for_pane(pane_id: u64, workspace_root: PathBuf) -> Option<(u16, String)> {
    discovery().map(|port| {
        let token = register_pane_credential(pane_id, workspace_root);
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
/// credential. Context roots can change while their panes remain alive.
pub(crate) fn rebind_pane_credential(pane_id: u64, workspace_root: PathBuf) {
    let mut registry = credentials().lock().unwrap();
    let Some(token) = registry.by_pane.get(&pane_id).cloned() else {
        return;
    };
    registry.by_token.insert(
        token,
        McpCaller {
            pane_id,
            workspace_root: workspace_root.clone(),
        },
    );
    log::info!(
        "host_mcp: rebound scoped credential pane={pane_id} workspace={}",
        workspace_root.display()
    );
}

fn authenticate(token: &str) -> Option<McpCaller> {
    credentials().lock().unwrap().by_token.get(token).cloned()
}

#[cfg(test)]
pub(crate) fn register_pane_credential_for_test(pane_id: u64, workspace_root: PathBuf) -> String {
    register_pane_credential(pane_id, workspace_root)
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
    let tmp = path.with_extension("json.tmp");
    let body = match serde_json::to_vec_pretty(&HostMcpIdentity { port }) {
        Ok(b) => b,
        Err(e) => {
            log::warn!("host_mcp: could not serialize identity: {e}");
            return;
        }
    };
    if let Err(e) = std::fs::write(&tmp, &body) {
        log::warn!("host_mcp: could not write {}: {e}", tmp.display());
        return;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    if let Err(e) = std::fs::rename(&tmp, &path) {
        log::warn!("host_mcp: could not finalize {}: {e}", path.display());
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

    std::thread::Builder::new()
        .name(format!("host-mcp-accept-{port}"))
        .spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        let subscribe_tx = subscribe_tx.clone();
                        std::thread::Builder::new()
                            .name("host-mcp-conn".to_string())
                            .spawn(move || {
                                if let Err(e) = handle_connection(stream, &subscribe_tx) {
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
        caller.pane_id,
        caller.workspace_root.clone(),
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
                "list_event_streams" => tool_list_event_streams(),
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

/// `list_event_streams` — read declared streams from the global timeline.
fn tool_list_event_streams() -> Result<String, String> {
    let streams = crate::host::app_timeline::global()
        .lock()
        .unwrap()
        .all_declared_streams();
    let arr: Vec<serde_json::Value> = streams
        .iter()
        .map(|(a, s)| serde_json::json!({ "app_id": a, "stream": s }))
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
        Some("off") => crate::app_protocol::PayloadMode::Off,
        Some("summary") => crate::app_protocol::PayloadMode::Summary,
        Some("state_ref") => crate::app_protocol::PayloadMode::StateRef,
        _ => crate::app_protocol::PayloadMode::Full,
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
        trigger_mode: crate::app_protocol::TriggerMode::Conversation,
        resource_id: None,
        from_pane_id: Some(caller.pane_id),
        subscriber_override: Some(delivery_id),
        subscriber_type_override: None,
        broker_actor_override: Some(actor_id),
        workspace_root_override: Some(caller.workspace_root.clone()),
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
    fn start_test_server(grant_target: Option<&str>) -> (u16, String) {
        static NEXT_PANE_ID: std::sync::atomic::AtomicU64 =
            std::sync::atomic::AtomicU64::new(80_000);
        use crate::broker::{
            ActorScope, ActorType, Decision, GrantDuration, GrantRecord, GrantSource,
            ResourceScope, TargetType,
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
            store.record(GrantRecord {
                actor_type: ActorType::Agent,
                actor_id,
                actor_scope: ActorScope::User,
                workspace_root: Some(workspace_root.clone()),
                target_type: TargetType::AppEventStream,
                target_id: target.to_string(),
                resource_scope: ResourceScope::Global,
                resource_id: None,
                decision: Decision::Allow,
                duration: GrantDuration::Session,
                source: GrantSource::Session,
                created_at: 0,
                expires_at: None,
            });
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
        let port = start_host_mcp_server(tx, dir.path()).unwrap();
        let token = register_pane_credential(pane_id, workspace_root);
        (port, token)
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
        let live_token = register_pane_credential(live_id, live_root.clone());
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
        let (port, _t) = start_test_server(None);
        let (status, _) = post(
            port,
            None,
            br#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
        );
        assert_eq!(status, 401);
    }

    #[test]
    fn tools_list_exposes_subscription_tools() {
        let (port, token) = start_test_server(None);
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

    #[test]
    fn pane_credential_lists_and_calls_only_workspace_app_tools() {
        use crate::app_protocol::{AiTool, PlexiEvent};
        use crate::plexi_ai::tool_dispatch::{self, AppEventSender, ToolCallResult};

        let (port, _test_token) = start_test_server(None);
        let workspace_a = PathBuf::from("/workspace/host-mcp-a");
        let workspace_b = PathBuf::from("/workspace/host-mcp-b");
        let caller_token = register_pane_credential(7_001, workspace_a.clone());
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
            workspace_a,
        );
        tool_dispatch::register(
            7_003,
            "workspace-b-app".to_string(),
            vec![tool("secret")],
            AppEventSender::Channel(other_tx),
            workspace_b,
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
            } = event
            else {
                panic!("expected ToolCall");
            };
            assert_eq!(name, "echo");
            assert_eq!(input_json, r#"{"value":7}"#);
            assert_eq!(caller_id, "mcp:pane:7001");
            tool_dispatch::resolve_pending(
                &call_id,
                ToolCallResult {
                    output_json: Some(r#"{"echo":7}"#.to_string()),
                    error: None,
                },
            );
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

    /// End-to-end proof the MCP adapter is wired, not just that a host test can
    /// route in-process: subscribe via the tool, emit on the timeline, and the
    /// tool returns the event.
    #[test]
    fn subscribe_and_wait_delivers_emitted_event() {
        use crate::app_protocol::{AppEventActor, EventStreamDecl};
        use crate::host::app_timeline::EmittedEvent;
        let app = "mcp-it-app";
        let stream = "it.tick";
        crate::host::app_timeline::global()
            .lock()
            .unwrap()
            .declare_streams(
                app,
                vec![EventStreamDecl {
                    name: stream.to_string(),
                    schema: serde_json::json!({"type": "object"}),
                    description: None,
                }],
            )
            .unwrap();
        let (port, token) = start_test_server(Some(&format!("{app}::{stream}")));

        // Emit shortly after the tool call begins its long-poll.
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            crate::host::app_timeline::global()
                .lock()
                .unwrap()
                .record_event(
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
