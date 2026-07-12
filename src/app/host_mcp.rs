//! Host-level event-subscription MCP server — `stint 0214`.
//!
//! The native transport for MCP-aware agents (Claude Code, Codex). Unlike the
//! app-scoped tool bridges, this is a single host-wide
//! server started once at boot. It exposes the same subscription primitive the
//! `plexi events` CLI wraps, backed by the one host subscription core — CLI and
//! MCP are transports, not separate event buses.
//!
//! Tools:
//! - `list_event_streams` — discover the `(app, stream)` pairs running apps declare.
//! - `subscribe_and_wait` — broker-checked subscribe, block for the next event,
//!   return it, then drop the subscription. A long-poll, so an agent can "wait
//!   for the next event and report it" in one tool call.
//!
//! Discovery: the bound port + bearer token are injected into every pane's PTY
//! env as `PLEXI_HOST_MCP_PORT` / `PLEXI_HOST_MCP_TOKEN`, so an agent in a pane
//! can configure this server without a wrapper subprocess.
//!
//! Persistence: the `(port, token)` is stored in `config_dir/host_mcp.json` and
//! reused on every launch. This makes the server install-once — an agent runs
//! `plexi events mcp-config` a single time, bakes the URL + bearer into a static
//! MCP client config, and it keeps resolving across Plexi restarts instead of
//! breaking when a fresh port/token is rolled each boot.
//!
//! Identity: subscriptions are recorded under the host-stamped id `mcp:host`
//! (set by this trusted transport, never from a tool argument), so an MCP client
//! cannot spoof another subscriber.

use crate::host::event_subscriptions::{HostSubscribeReply, HostSubscribeRequest};
use crate::mcp_http::{read_json_rpc_request, write_http_response, RequestOutcome};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// The host-stamped subscriber identity for all MCP-routed subscriptions.
const MCP_SUBSCRIBER_ID: &str = "mcp:host";
/// Hard cap on `subscribe_and_wait` blocking, kept under common MCP client
/// request timeouts. The client may request less via `timeout_secs`.
const MAX_WAIT_SECS: u64 = 55;
const DEFAULT_WAIT_SECS: u64 = 25;

/// Process-wide discovery info for the singleton host MCP server: `(port,
/// token)`. Set once when the server binds; read by the pane PTY env builder so
/// every pane gets `PLEXI_HOST_MCP_PORT` / `PLEXI_HOST_MCP_TOKEN`.
static DISCOVERY: OnceLock<(u16, String)> = OnceLock::new();

/// The bound `(port, token)` once the server has started, else `None`.
pub fn discovery() -> Option<&'static (u16, String)> {
    DISCOVERY.get()
}

/// The persisted host-MCP identity (`config_dir/host_mcp.json`). Survives
/// restarts so a baked MCP client config keeps working — see the module docs.
#[derive(serde::Serialize, serde::Deserialize)]
struct HostMcpIdentity {
    port: u16,
    token: String,
}

fn identity_path(config_dir: &Path) -> PathBuf {
    config_dir.join("host_mcp.json")
}

/// Load the persisted `(port, token)` if the file exists and parses. A corrupt
/// file is logged and ignored so the next launch re-rolls a fresh identity
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

/// Atomically persist `(port, token)` with owner-only permissions — the file
/// carries a bearer secret. Best-effort: a write failure only means the next
/// launch re-rolls the identity, so it is logged rather than propagated.
fn save_identity(config_dir: &Path, port: u16, token: &str) {
    let path = identity_path(config_dir);
    let tmp = path.with_extension("json.tmp");
    let body = match serde_json::to_vec_pretty(&HostMcpIdentity {
        port,
        token: token.to_string(),
    }) {
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
/// UI thread (which owns the grant store); `egui_ctx` is woken so the UI drains
/// the subscribe channel promptly even while idle. `config_dir` is the profile
/// dir where the `(port, token)` identity is persisted across restarts.
/// Idempotent at the discovery level — the first successful bind wins.
pub fn start_host_mcp_server(
    subscribe_tx: Sender<HostSubscribeRequest>,
    egui_ctx: egui::Context,
    config_dir: &Path,
) -> std::io::Result<(u16, String)> {
    let persisted = load_identity(config_dir);
    // Reuse the persisted token so a baked `Authorization: Bearer` keeps
    // authenticating; generate one only on first launch.
    let token = persisted
        .as_ref()
        .map(|id| id.token.clone())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

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

    save_identity(config_dir, port, &token);

    let _ = DISCOVERY.set((port, token.clone()));
    let token_arc = std::sync::Arc::new(token.clone());

    std::thread::Builder::new()
        .name(format!("host-mcp-accept-{port}"))
        .spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        let token = std::sync::Arc::clone(&token_arc);
                        let subscribe_tx = subscribe_tx.clone();
                        let egui_ctx = egui_ctx.clone();
                        std::thread::Builder::new()
                            .name("host-mcp-conn".to_string())
                            .spawn(move || {
                                if let Err(e) =
                                    handle_connection(stream, &token, &subscribe_tx, &egui_ctx)
                                {
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
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

    log::info!("host_mcp: started on 127.0.0.1:{port}");
    Ok((port, token))
}

fn tool_defs() -> serde_json::Value {
    serde_json::json!([
        {
            "name": "list_event_streams",
            "description": "List the event streams currently declared by running Plexi apps. Returns an array of {app_id, stream} pairs.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        },
        {
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
        }
    ])
}

fn handle_connection(
    stream: std::net::TcpStream,
    token: &str,
    subscribe_tx: &Sender<HostSubscribeRequest>,
    egui_ctx: &egui::Context,
) -> std::io::Result<()> {
    let peer = stream
        .peer_addr()
        .map(|a| a.to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    let mut write_stream = stream.try_clone()?;

    let json = match read_json_rpc_request(&stream, token)? {
        RequestOutcome::Json(v) => v,
        RequestOutcome::Handled => return Ok(()),
    };

    let id = json.get("id").cloned().unwrap_or(serde_json::Value::Null);
    let method_name = json.get("method").and_then(|m| m.as_str()).unwrap_or("");

    let response_body = match method_name {
        "initialize" => serde_json::json!({
            "jsonrpc": "2.0", "id": id,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "plexi-host-events", "version": "1.0.0" }
            }
        }),
        "notifications/initialized" => serde_json::json!({
            "jsonrpc": "2.0", "id": serde_json::Value::Null, "result": serde_json::Value::Null
        }),
        "tools/list" => serde_json::json!({
            "jsonrpc": "2.0", "id": id, "result": { "tools": tool_defs() }
        }),
        "tools/call" => {
            let params = json.get("params").cloned().unwrap_or_default();
            let tool_name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or(serde_json::Value::Object(Default::default()));
            log::info!("host_mcp: tool_call tool={tool_name} peer={peer}");
            let result = match tool_name {
                "list_event_streams" => tool_list_event_streams(),
                "subscribe_and_wait" => tool_subscribe_and_wait(&arguments, subscribe_tx, egui_ctx),
                other => Err(format!("unknown tool: {other}")),
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
    subscribe_tx: &Sender<HostSubscribeRequest>,
    egui_ctx: &egui::Context,
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
    // Identity is split (#2290): the broker actor stays the stable `mcp:host`
    // so one "Always" grant covers every host-MCP connection, while the delivery
    // routing key is minted unique per call so two concurrent waiters never
    // cross-talk or tear each other down.
    let delivery_id = format!("{MCP_SUBSCRIBER_ID}:{}", uuid::Uuid::new_v4());
    let (reply_tx, reply_rx) = std::sync::mpsc::sync_channel::<HostSubscribeReply>(1);
    let req = HostSubscribeRequest {
        publisher_app_id: app_id.clone(),
        event_names,
        payload_mode,
        trigger_mode: crate::app_protocol::TriggerMode::Conversation,
        resource_id: None,
        from_pane_id: None,
        subscriber_override: Some(delivery_id),
        broker_actor_override: Some(MCP_SUBSCRIBER_ID.to_string()),
        reply: reply_tx,
    };
    subscribe_tx
        .send(req)
        .map_err(|_| "host not accepting subscriptions".to_string())?;
    egui_ctx.request_repaint();

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
        use crate::broker::{
            ActorScope, ActorType, Decision, GrantDuration, GrantRecord, GrantSource,
            ResourceScope, TargetType,
        };
        let (tx, rx) = std::sync::mpsc::channel::<HostSubscribeRequest>();
        let mut store = crate::broker::GrantStore::default();
        if let Some(target) = grant_target {
            store.record(GrantRecord {
                actor_type: ActorType::Agent,
                actor_id: MCP_SUBSCRIBER_ID.to_string(),
                actor_scope: ActorScope::User,
                workspace_root: None,
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
        // identity never collides with a sibling test's port/token.
        let dir = tempfile::tempdir().unwrap();
        start_host_mcp_server(tx, egui::Context::default(), dir.path()).unwrap()
    }

    #[test]
    fn identity_round_trips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        save_identity(dir.path(), 4242, "tok-abc");
        let loaded = load_identity(dir.path()).expect("identity file written");
        assert_eq!(loaded.port, 4242);
        assert_eq!(loaded.token, "tok-abc");
    }

    #[test]
    fn restart_reuses_persisted_token() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, _rx) = std::sync::mpsc::channel::<HostSubscribeRequest>();
        let (port1, token1) =
            start_host_mcp_server(tx.clone(), egui::Context::default(), dir.path()).unwrap();
        // A second boot against the same profile dir must reuse the token so a
        // baked `Authorization: Bearer` keeps authenticating across restarts.
        let (_port2, token2) =
            start_host_mcp_server(tx, egui::Context::default(), dir.path()).unwrap();
        assert_eq!(token1, token2, "token must persist across restarts");
        assert_eq!(load_identity(dir.path()).unwrap().token, token1);
        assert!(port1 > 0);
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
