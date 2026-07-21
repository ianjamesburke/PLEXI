//! `plexi events` — subscribe to app event streams from a terminal pane.
//!
//! The lowest-common-denominator agent transport: any process that can read
//! subprocess stdout can subscribe. Both subcommands open the host socket,
//! send one control line, and stream newline-delimited JSON back:
//!
//! - `subscribe` keeps the connection open and prints a `subscribed` ack line
//!   followed by one line per delivered event, until interrupted.
//! - `list` prints the apps' declared streams once and exits.
//!
//! `declare` and `emit` are the publish side: they send one control line and
//! read a single `ok`/`error` reply. A first-time publish under an app-id
//! namespace the caller does not own prompts for host consent (broker `Ask`).
//!
//! Identity is host-stamped from `PLEXI_PANE_ID`; there is deliberately no flag
//! to set the subscriber/emitter identity, so a CLI agent cannot spoof another.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;

/// Connect to the running host's command socket. Mirrors the connect/cleanup
/// behaviour of `send_to_socket`, but returns the live stream so the caller can
/// stream NDJSON responses back.
fn connect_socket() -> Result<UnixStream, i32> {
    let socket_path = match super::resolve_command_socket() {
        Some(path) => path,
        None => {
            eprintln!("error: PLEXI_SOCKET is not set — run this inside a Plexi terminal pane");
            return Err(1);
        }
    };
    match UnixStream::connect(&socket_path) {
        Ok(s) => Ok(s),
        Err(e) if e.kind() == std::io::ErrorKind::ConnectionRefused => {
            let _ = std::fs::remove_file(&socket_path);
            eprintln!("error: Plexi is not responding (stale socket removed). Is Plexi running?");
            Err(1)
        }
        Err(e) => {
            eprintln!("error: could not connect to PLEXI_SOCKET {socket_path:?}: {e}");
            Err(1)
        }
    }
}

fn from_pane_id() -> Option<u64> {
    std::env::var("PLEXI_PANE_ID").ok()?.parse().ok()
}

/// Connect to the host socket and write one control line + flush, logging the
/// sent type. Returns the live stream so the caller can read its reply (a
/// single line for publish, an open NDJSON stream for subscribe). Shared
/// prologue for [`stream_control_line`] and [`send_and_read_reply`].
fn connect_and_send(payload: &serde_json::Value) -> Result<UnixStream, i32> {
    let mut stream = connect_socket()?;
    let line = format!("{payload}\n");
    if let Err(e) = stream.write_all(line.as_bytes()) {
        eprintln!("error: could not write to socket: {e}");
        return Err(1);
    }
    let _ = stream.flush();
    log::info!("events: sent control line type={}", payload["type"]);
    Ok(stream)
}

/// Send one control line and stream every NDJSON line the host returns to
/// stdout until the connection closes. Returns the process exit code.
fn stream_control_line(payload: serde_json::Value) -> i32 {
    let stream = match connect_and_send(&payload) {
        Ok(s) => s,
        Err(code) => return code,
    };
    let reader = BufReader::new(stream);
    let stdout = std::io::stdout();
    for line in reader.lines() {
        match line {
            Ok(line) => {
                let mut out = stdout.lock();
                if writeln!(out, "{line}").is_err() {
                    return 0; // downstream consumer went away
                }
                let _ = out.flush();
            }
            Err(e) => {
                eprintln!("error: reading event stream: {e}");
                return 1;
            }
        }
    }
    0
}

/// `plexi events subscribe <app_id> <stream>` — stream deliveries as NDJSON.
pub fn events_subscribe_cli(
    app_id: &str,
    stream: Option<&str>,
    all: bool,
    payload: &str,
    trigger: &str,
    resource: Option<&str>,
) -> i32 {
    let event_names: Vec<String> = match (all, stream) {
        (true, _) => vec![],
        (false, Some(s)) => vec![s.to_string()],
        (false, None) => {
            eprintln!(
                "error: specify a stream name (e.g. `plexi events subscribe {app_id} probe.tick`) \
                 or pass --all to subscribe to every stream"
            );
            return 2;
        }
    };
    let payload_mode = payload_mode_json(payload);
    let trigger_mode = trigger_mode_json(trigger);
    let req = serde_json::json!({
        "type": "events_subscribe",
        "app_id": app_id,
        "event_names": event_names,
        "payload_mode": payload_mode,
        "trigger_mode": trigger_mode,
        "resource_id": resource,
        "from_pane_id": from_pane_id(),
    });
    stream_control_line(req)
}

/// Send one control line, read exactly one JSON reply line, print a
/// human-readable result, and return the process exit code. Used by the
/// one-shot publish commands (`declare`, `emit`): the host answers with a
/// single `ok`/`error` object rather than an open NDJSON stream.
fn send_and_read_reply(payload: serde_json::Value) -> i32 {
    let stream = match connect_and_send(&payload) {
        Ok(s) => s,
        Err(code) => return code,
    };
    let mut reader = BufReader::new(stream);
    let mut reply = String::new();
    if let Err(e) = reader.read_line(&mut reply) {
        eprintln!("error: reading reply: {e}");
        return 1;
    }
    let reply = reply.trim();
    if reply.is_empty() {
        eprintln!("error: host closed the connection without replying");
        return 1;
    }
    match serde_json::from_str::<serde_json::Value>(reply) {
        Ok(val) => match val["type"].as_str() {
            Some("error") => {
                eprintln!(
                    "error: {}",
                    val["message"].as_str().unwrap_or("unknown error")
                );
                1
            }
            _ => {
                println!("{}", val["detail"].as_str().unwrap_or(reply));
                0
            }
        },
        Err(_) => {
            eprintln!("error: host reply was not valid JSON: {reply}");
            1
        }
    }
}

/// `plexi events declare <app_id> <stream>` — register a stream schema.
pub fn events_declare_cli(
    app_id: &str,
    stream: &str,
    schema: &str,
    description: Option<&str>,
) -> i32 {
    let schema_json: serde_json::Value = match serde_json::from_str(schema) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: --schema is not valid JSON: {e}");
            return 2;
        }
    };
    let mut decl = serde_json::json!({ "name": stream, "schema": schema_json });
    if let Some(d) = description {
        decl["description"] = serde_json::Value::String(d.to_string());
    }
    let req = serde_json::json!({
        "type": "events_declare",
        "app_id": app_id,
        "streams": [decl],
        "from_pane_id": from_pane_id(),
    });
    send_and_read_reply(req)
}

/// Arguments for `plexi events emit`, mirroring the `EventsCmd::Emit` fields.
pub struct EmitArgs<'a> {
    pub app_id: &'a str,
    pub event: &'a str,
    pub summary: &'a str,
    pub resource: &'a str,
    pub revision_after: &'a str,
    pub actor: &'a str,
    pub resource_scope: Option<&'a str>,
    pub payload: Option<&'a str>,
    pub state_ref: Option<&'a str>,
    pub revision_before: Option<&'a str>,
    pub rollback_token: Option<&'a str>,
    pub changed_resources: &'a [String],
}

/// `plexi events emit <app_id> <event>` — record + fan out one event.
pub fn events_emit_cli(args: EmitArgs) -> i32 {
    let payload_json = match args.payload {
        Some(p) => match serde_json::from_str::<serde_json::Value>(p) {
            Ok(v) => Some(v),
            Err(e) => {
                eprintln!("error: --payload is not valid JSON: {e}");
                return 2;
            }
        },
        None => None,
    };
    let req = serde_json::json!({
        "type": "events_emit",
        "app_id": args.app_id,
        "event": args.event,
        "actor": args.actor,
        "summary": args.summary,
        "resource_id": args.resource,
        "resource_scope": args.resource_scope,
        "revision_after": args.revision_after,
        "payload": payload_json,
        "state_ref": args.state_ref,
        "revision_before": args.revision_before,
        "rollback_token": args.rollback_token,
        "changed_resources": args.changed_resources,
        "from_pane_id": from_pane_id(),
    });
    send_and_read_reply(req)
}

/// `plexi events list` — print declared streams once and exit.
pub fn events_list_cli(json: bool) -> i32 {
    let req = serde_json::json!({
        "type": "events_list",
        "json": json,
    });
    stream_control_line(req)
}

/// `plexi events mcp-config` — print the host MCP server config block.
pub fn events_mcp_config_cli() -> i32 {
    let port = std::env::var("PLEXI_HOST_MCP_PORT").ok();
    let token = std::env::var("PLEXI_HOST_MCP_TOKEN").ok();
    let (port, token) = match (port, token) {
        (Some(p), Some(t)) if !p.is_empty() && !t.is_empty() => (p, t),
        _ => {
            eprintln!(
                "error: PLEXI_HOST_MCP_PORT / PLEXI_HOST_MCP_TOKEN not set — run this inside a \
                 Plexi terminal pane on a build with the host event MCP server"
            );
            return 1;
        }
    };
    let config = serde_json::json!({
        "mcpServers": {
            "plexi-events": {
                "type": "http",
                "url": format!("http://127.0.0.1:{port}/mcp"),
                "headers": { "Authorization": format!("Bearer {token}") }
            }
        }
    });
    match serde_json::to_string_pretty(&config) {
        Ok(s) => {
            println!("{s}");
            log::info!("events: printed host MCP config for port {port}");
            0
        }
        Err(e) => {
            eprintln!("error: could not serialize MCP config: {e}");
            1
        }
    }
}

/// Map the CLI `--payload` value to the wire `PayloadMode` (snake_case serde).
fn payload_mode_json(s: &str) -> &'static str {
    match s {
        "off" => "off",
        "summary" => "summary",
        "state-ref" => "state_ref",
        _ => "full",
    }
}

/// Map the CLI `--trigger` value to the wire `TriggerMode` (snake_case serde).
fn trigger_mode_json(s: &str) -> &'static str {
    match s {
        "never" => "never",
        "ambient" => "ambient",
        "ask" => "ask",
        _ => "conversation",
    }
}
