//! Host-level event-subscription MCP server — `stint 0214`.
//!
//! The native transport for MCP-aware agents (Claude Code, Codex). Unlike the
//! per-app MCP bridge (`process_app::mcp_server`), this is a single host-wide
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
//! Identity: subscriptions are recorded under the host-stamped id `mcp:host`
//! (set by this trusted transport, never from a tool argument), so an MCP client
//! cannot spoof another subscriber.

use crate::host::event_subscriptions::{HostSubscribeReply, HostSubscribeRequest};
use crate::mcp_http::{read_json_rpc_request, write_http_response, RequestOutcome};
use std::net::TcpListener;
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

/// Start the host MCP server. `subscribe_tx` routes subscribe requests to the
/// UI thread (which owns the grant store); `egui_ctx` is woken so the UI drains
/// the subscribe channel promptly even while idle. Idempotent at the discovery
/// level — the first successful bind wins.
pub fn start_host_mcp_server(
    subscribe_tx: Sender<HostSubscribeRequest>,
    egui_ctx: egui::Context,
) -> std::io::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let token = uuid::Uuid::new_v4().to_string();
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
    Ok(())
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
                "subscribe_and_wait" => {
                    tool_subscribe_and_wait(&arguments, subscribe_tx, egui_ctx)
                }
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
    let (reply_tx, reply_rx) = std::sync::mpsc::sync_channel::<HostSubscribeReply>(1);
    let req = HostSubscribeRequest {
        publisher_app_id: app_id.clone(),
        event_names,
        payload_mode,
        trigger_mode: crate::app_protocol::TriggerMode::Conversation,
        resource_id: None,
        from_pane_id: None,
        subscriber_override: Some(MCP_SUBSCRIBER_ID.to_string()),
        reply: reply_tx,
    };
    subscribe_tx
        .send(req)
        .map_err(|_| "host not accepting subscriptions".to_string())?;
    egui_ctx.request_repaint();

    let (subscriber_type, subscriber_id) = match reply_rx.recv_timeout(Duration::from_secs(5)) {
        Ok(HostSubscribeReply::Ok {
            subscriber_type,
            subscriber_id,
            ..
        }) => (subscriber_type, subscriber_id),
        Ok(HostSubscribeReply::Err { message }) => return Err(message),
        Err(_) => return Err("subscribe timed out".to_string()),
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
            log::info!("host_mcp: subscribe_and_wait delivered event {} from {}", d.event, d.app_id);
            serde_json::to_string(&out).map_err(|e| format!("serialize failed: {e}"))
        }
        None => Ok(format!(
            "{{\"timeout\":true,\"message\":\"no event within {timeout_secs}s\"}}"
        )),
    }
}
