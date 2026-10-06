//! Outbound phone-relay client.
//!
//! `plexi relay connect` dials the relay, shows a one-time pairing code, and
//! forwards each phone message into `assistant send` on that turn's
//! `request_id` and the phone's own conversation. The relay is the default
//! path. A local `--tailscale` phone shell remains a direct alternative.

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::relay_crypto;
use super::relay_ws::{self, Incoming, WsConn};

const STATUS_FILE: &str = "relay-status.json";
const IDENTITY_FILE: &str = "relay-host.json";
const PROTOCOL_VERSION: i64 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dispatch {
    /// `plexi assistant send` against the running host.
    Host,
    /// Local stand-in that speaks the assistant-send JSON shape. Tests and the
    /// e2e script opt in. It is not the default.
    Echo,
}

struct Identity {
    host_id: String,
    host_token: String,
    label: String,
}

struct PairedDevice {
    device_id: String,
    label: String,
    fingerprint: String,
}

struct PendingTurn {
    delivery_id: String,
    request_id: String,
    conversation_id: String,
    device_id: String,
    text: String,
    join_desktop: bool,
}

struct Inflight {
    delivery_id: String,
    request_id: String,
    device_id: String,
    rx: Receiver<Value>,
    acked: bool,
}

struct Session {
    url: relay_ws::RelayUrl,
    identity: Identity,
    dispatch: Dispatch,
    pairing_id: Option<String>,
    devices: Vec<PairedDevice>,
    /// `host` when the host process owns the socket, `cli` for `relay connect`.
    owner: &'static str,
    control: String,
    queued: VecDeque<PendingTurn>,
    inflight: Option<Inflight>,
    last_ping: Instant,
    pending_key: Option<relay_crypto::PendingDesktopKey>,
    phone_sessions: HashMap<String, relay_crypto::PhoneSession>,
}

pub fn relay_connect_cli(url: Option<String>) -> i32 {
    let url = match url.or_else(configured_url) {
        Some(url) => url,
        None => {
            eprintln!("error: pass --url or set PLEXI_RELAY_URL or [url] in relay.toml");
            return 1;
        }
    };
    let parsed = match relay_ws::parse_relay_url(&url) {
        Ok(parsed) => parsed,
        Err(error) => {
            eprintln!("error: {error}");
            log::info!("relay: rejected url outcome=insecure_or_invalid");
            return 1;
        }
    };
    let echo_requested = std::env::var("PLEXI_RELAY_ASSISTANT")
        .ok()
        .is_some_and(|value| value == "echo");
    let dispatch = dispatch_from_env(echo_requested, relay_ws::is_loopback(&parsed.host));
    if echo_requested && !matches!(dispatch, Dispatch::Echo) {
        log::info!("relay: ignored echo dispatch outcome=not_loopback");
    }
    match live_connection() {
        Some(LiveConnection::Host) => {
            println!("attached to the host relay");
            if let Ok(text) = fs::read_to_string(status_path()) {
                println!("{text}");
            }
            log::info!("relay: attached to the host connection");
            return 0;
        }
        Some(LiveConnection::Other) => {
            eprintln!(
                "error: a relay connection is already running. Stop it before starting another. A second connection would replace this desktop on the relay."
            );
            log::info!("relay: refused a second connection outcome=already_running");
            return 1;
        }
        None => {}
    }
    if let Err(error) = run_session(&url, dispatch, "cli") {
        if error == ALREADY_RUNNING {
            if matches!(live_connection(), Some(LiveConnection::Host)) {
                println!("attached to the host relay");
                log::info!("relay: attached to the host connection");
                return 0;
            }
            eprintln!(
                "error: a relay connection is already running. Stop it before starting another. A second connection would replace this desktop on the relay."
            );
            log::info!("relay: refused a second connection outcome=already_running");
            return 1;
        }
        eprintln!("error: {error}");
        log::error!("relay: connect failed outcome={error}");
        return 1;
    }
    0
}

pub fn relay_enable_cli(url: Option<String>) -> i32 {
    let url = match url.or_else(configured_url) {
        Some(url) => url,
        None => {
            eprintln!("error: pass --url or set PLEXI_RELAY_URL");
            return 1;
        }
    };
    if let Err(error) = relay_ws::parse_relay_url(&url) {
        eprintln!("error: {error}");
        log::info!("relay: rejected url outcome=insecure_or_invalid");
        return 1;
    }
    if let Err(error) = write_relay_config(true, &url) {
        eprintln!("error: {error}");
        return 1;
    }
    log::info!("relay: enabled");
    println!("Relay enabled. It connects when the host starts.");
    0
}

pub fn relay_disable_cli() -> i32 {
    let url = configured_url().unwrap_or_default();
    if let Err(error) = write_relay_config(false, &url) {
        eprintln!("error: {error}");
        return 1;
    }
    log::info!("relay: disabled");
    println!("Relay disabled. The host will not connect on the next start.");
    0
}

pub fn start_host_relay() {
    if !relay_enabled() {
        return;
    }
    let Some(url) = configured_url() else {
        log::warn!("relay: enabled but no url is configured");
        return;
    };
    if live_connection().is_some() {
        log::info!("relay: host left the existing connection in place");
        return;
    }
    log::info!("relay: host connecting");
    thread::spawn(move || {
        if let Err(error) = run_session(&url, Dispatch::Host, "host") {
            if error == ALREADY_RUNNING {
                log::info!("relay: host did not open a second connection");
            } else {
                log::error!("relay: host session ended outcome={error}");
            }
        }
    });
}

pub fn relay_confirm_cli(pairing_id: Option<String>) -> i32 {
    control_roundtrip(json!({"type": "confirm", "pairing_id": pairing_id}))
}

pub fn relay_revoke_cli(device_id: &str) -> i32 {
    control_roundtrip(json!({"type": "revoke", "device_id": device_id}))
}

pub fn relay_pair_cli() -> i32 {
    control_roundtrip(json!({"type": "pair"}))
}

pub fn relay_status_cli() -> i32 {
    let path = status_path();
    match fs::read_to_string(&path) {
        Ok(text) => {
            println!("{text}");
            0
        }
        Err(error) => {
            eprintln!("error: no relay status at {}: {error}", path.display());
            1
        }
    }
}

fn run_session(url: &str, dispatch: Dispatch, owner: &'static str) -> Result<(), String> {
    let parsed = relay_ws::parse_relay_url(url)?;
    fs::create_dir_all(crate::config::config_dir())
        .map_err(|error| format!("relay profile: {error}"))?;
    let _lock = try_session_lock()?;
    let (identity, devices) = load_identity(crate::workspace::secrets::system_store())?;
    let phone_sessions = load_phone_sessions(&devices);
    let pending_key = load_desktop_key(&identity.host_id);
    let listener =
        TcpListener::bind("127.0.0.1:0").map_err(|error| format!("relay control: {error}"))?;
    let control = listener
        .local_addr()
        .map_err(|error| format!("relay control: {error}"))?
        .to_string();
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("relay control: {error}"))?;
    let mut session = Session {
        url: parsed,
        identity,
        dispatch,
        pairing_id: None,
        devices,
        owner,
        control: control.clone(),
        queued: VecDeque::new(),
        inflight: None,
        last_ping: Instant::now(),
        pending_key,
        phone_sessions,
    };
    write_status(&session, "connecting", None, None, None)?;
    log::info!(
        "relay: connecting host_id={} host={} port={} tls={}",
        session.identity.host_id,
        session.url.host,
        session.url.port,
        session.url.tls
    );
    println!("relay control {}", session.control);
    let mut attempt = 0u32;
    let mut socket: Option<WsConn> = None;
    loop {
        poll_control(&listener, &mut session, &mut socket)?;
        if session_stopped() {
            log::info!("relay: stopping host_id={}", session.identity.host_id);
            return Ok(());
        }
        if session.owner != "host"
            && matches!(session.dispatch, Dispatch::Host)
            && !host_socket_open()
        {
            log::info!(
                "relay: host gone, closing desktop link host_id={}",
                session.identity.host_id
            );
            return Ok(());
        }
        pump_assistant(&mut session, &mut socket)?;
        if socket.is_none() {
            match WsConn::connect(&session.url) {
                Ok(mut connected) => {
                    send_json(
                        &mut connected,
                        &json!({
                            "type": "hello",
                            "protocol": PROTOCOL_VERSION,
                            "host_id": session.identity.host_id,
                            "host_token": session.identity.host_token,
                            "host_label": session.identity.label,
                            "devices": session.devices.iter().map(|device| json!({
                                "device_id": device.device_id,
                                "fingerprint": device.fingerprint,
                            })).collect::<Vec<_>>(),
                        }),
                    )?;
                    attempt = 0;
                    session.last_ping = Instant::now();
                    socket = Some(connected);
                    log::info!(
                        "relay: desktop socket open host_id={}",
                        session.identity.host_id
                    );
                }
                Err(_) => {
                    log::info!(
                        "relay: connect retry outcome=failed host={}",
                        session.url.host
                    );
                    backoff(attempt);
                    attempt = attempt.saturating_add(1);
                    continue;
                }
            }
        }
        let incoming = {
            let Some(conn) = socket.as_mut() else {
                continue;
            };
            if session.last_ping.elapsed() >= Duration::from_secs(15) {
                send_json(conn, &json!({"type": "ping"}))?;
                session.last_ping = Instant::now();
            }
            conn.recv()
        };
        match incoming {
            Ok(Incoming::Timeout) => {}
            Ok(Incoming::Closed) => {
                log::info!(
                    "relay: desktop socket closed host_id={}",
                    session.identity.host_id
                );
                socket = None;
                backoff(attempt);
                attempt = attempt.saturating_add(1);
            }
            Ok(Incoming::Text(text)) => {
                let message: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                if let Some(conn) = socket.as_mut() {
                    handle_relay_message(&mut session, conn, &message)?;
                }
            }
            Err(_) => {
                log::info!(
                    "relay: desktop socket error host_id={} outcome=read_failed",
                    session.identity.host_id
                );
                socket = None;
            }
        }
    }
}

/// Answer a paired phone's needs-you list or resolve.
///
/// `id` is `None` for a list. A resolve always counts as from the phone:
/// an approval click cannot be approved here.
fn answer_needs_you(
    session: &Session,
    conn: &mut WsConn,
    message: &Value,
    id: Option<String>,
    approve: bool,
) -> Result<(), String> {
    let request_id = json_str(message, "request_id");
    let monitor =
        crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir());
    let body = if let Some(id) = id {
        if approve
            && monitor
                .open_needs_you()
                .iter()
                .any(|row| row.id == id && !row.kind.phone_may_approve())
        {
            log::info!(
                "relay: phone refused irreversible needs-you host_id={} id={id}",
                session.identity.host_id
            );
            json!({
                "type": "needs_you_result",
                "request_id": request_id,
                "ok": false,
                "error": "waiting on desktop",
            })
        } else {
            log::info!(
                "relay: phone needs-you resolve host_id={} id={id} approve={approve}",
                session.identity.host_id
            );
            match monitor.resolve_needs_you(&id, approve) {
                Ok(receipt) => json!({
                    "type": "needs_you_result",
                    "request_id": request_id,
                    "ok": !receipt.already,
                    "id": receipt.id,
                    "resolution": receipt.resolution.as_str(),
                    "already": receipt.already,
                }),
                Err(error) => json!({
                    "type": "needs_you_result",
                    "request_id": request_id,
                    "ok": false,
                    "error": error,
                }),
            }
        }
    } else {
        let items = crate::broker::gate::needs_you_phone_items(&monitor.list_needs_you());
        log::info!(
            "relay: phone needs-you list host_id={} count={}",
            session.identity.host_id,
            items.len()
        );
        json!({
            "type": "needs_you_result",
            "request_id": request_id,
            "ok": true,
            "items": items,
        })
    };
    send_json(conn, &body)
}

fn handle_relay_message(
    session: &mut Session,
    conn: &mut WsConn,
    message: &Value,
) -> Result<(), String> {
    if let Some(problem) = protocol_problem(message) {
        log::info!(
            "relay: protocol mismatch host_id={} outcome=protocol_mismatch",
            session.identity.host_id
        );
        return Err(problem);
    }
    let kind = message
        .get("type")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    match kind {
        "hello_ok" => {
            session.devices = reconcile_paired(&session.devices, &devices_from_message(message));
            persist_record(&session.identity, &session.devices)?;
            log::info!(
                "relay: hello accepted host_id={} devices={}",
                session.identity.host_id,
                session.devices.len()
            );
            if session.devices.is_empty() {
                send_json(conn, &json!({"type": "pair_start"}))?;
            } else {
                write_status(session, "paired", None, None, None)?;
                log::info!(
                    "relay: resumed paired devices host_id={} devices={}",
                    session.identity.host_id,
                    session.devices.len()
                );
            }
        }
        "pair_code" => {
            let pairing_id = message
                .get("pairing_id")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let code = message
                .get("code")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let qr_url = message
                .get("qr_url")
                .and_then(|value| value.as_str())
                .unwrap_or("/");
            let pending = relay_crypto::generate_desktop_key().map_err(|_| {
                "could not generate a pairing key".to_string()
            })?;
            persist_desktop_key(&session.identity.host_id, &pending)?;
            let qr_url = relay_crypto::qr_with_public_key(qr_url, &pending.public);
            let key_fingerprint = relay_crypto::key_fingerprint(&pending.public);
            session.pending_key = Some(pending);
            session.pairing_id = Some(pairing_id.to_string());
            write_status(session, "waiting_for_phone", Some(code), Some(&qr_url), None)?;
            // The code and the key fingerprint are shown to the person at this
            // desktop. Neither is logged. The public key is only in the URL
            // fragment, which the relay never receives.
            println!("Pair this phone");
            println!("Code: {code}");
            println!("Key fingerprint: {key_fingerprint}");
            println!("QR (no code, no session token): {qr_url}");
            println!("The phone enters the code. Then run: plexi relay confirm");
            log::info!(
                "relay: pairing key displayed host_id={} pairing_id={pairing_id} outcome=key_displayed",
                session.identity.host_id
            );
        }
        "pair_pending" => {
            let pairing_id = message
                .get("pairing_id")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let fingerprint = message
                .get("fingerprint")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let label = message
                .get("device_label")
                .and_then(|value| value.as_str())
                .unwrap_or("phone");
            session.pairing_id = Some(pairing_id.to_string());
            write_status(session, "pending_confirm", None, None, Some(fingerprint))?;
            println!("Phone '{label}' wants to pair. Fingerprint {fingerprint}");
            println!("Confirm with: plexi relay confirm");
            log::info!(
                "relay: pairing awaiting confirm host_id={} pairing_id={pairing_id}",
                session.identity.host_id
            );
        }
        "pair_confirmed" => {
            let device_id = message
                .get("device_id")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            let fingerprint = message
                .get("fingerprint")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .to_string();
            if !device_id.is_empty()
                && !session
                    .devices
                    .iter()
                    .any(|device| device.device_id == device_id)
            {
                session.devices.push(PairedDevice {
                    device_id: device_id.to_string(),
                    label: "phone".to_string(),
                    fingerprint: fingerprint.clone(),
                });
            }
            persist_record(&session.identity, &session.devices)?;
            write_status(session, "confirmed", None, None, Some(&fingerprint))?;
            println!("Paired device {device_id}. Revoke with: plexi relay revoke {device_id}");
            log::info!(
                "relay: phone paired host_id={} device_id={device_id} devices={}",
                session.identity.host_id,
                session.devices.len()
            );
        }
        "revoked" => {
            let device_id = message
                .get("device_id")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            session
                .devices
                .retain(|device| device.device_id != device_id);
            session.phone_sessions.remove(device_id);
            if !device_id.is_empty() {
                let _ = crate::workspace::secrets::system_store()
                    .delete(&phone_session_account(device_id));
            }
            persist_record(&session.identity, &session.devices)?;
            let phase = if session.devices.is_empty() {
                "revoked"
            } else {
                "paired"
            };
            write_status(session, phase, None, None, None)?;
            println!("Revoked {device_id}");
            log::info!(
                "relay: phone revoked host_id={} device_id={device_id} devices={}",
                session.identity.host_id,
                session.devices.len()
            );
        }
        "deliver" => {
            let delivery_id = json_str(message, "delivery_id");
            let request_id = json_str(message, "request_id");
            let conversation_id = json_str(message, "conversation_id");
            let device_id = json_str(message, "device_id");
            let sealed = json_str(message, "text");
            // The outer join_desktop flag is the relay's copy. The phone's
            // choice is inside the seal. A relay that flips the outer flag
            // does not change the turn.
            match accept_sealed(session, &device_id, &request_id, &sealed) {
                Ok((text, join_desktop)) => {
                    log::info!(
                        "relay: phone seal accepted host_id={} request_id={request_id} delivery_id={delivery_id} device_id={device_id} bytes={}",
                        session.identity.host_id,
                        sealed.len()
                    );
                    session.queued.push_back(PendingTurn {
                        delivery_id,
                        request_id,
                        conversation_id,
                        device_id,
                        text,
                        join_desktop,
                    });
                }
                Err(error) => {
                    log::info!(
                        "relay: phone seal rejected host_id={} request_id={request_id} delivery_id={delivery_id} device_id={device_id} outcome={error:?}",
                        session.identity.host_id
                    );
                }
            }
        }
        "needs_you_list" => {
            answer_needs_you(session, conn, message, None, false)?;
        }
        "needs_you_resolve" => {
            let id = json_str(message, "id");
            let approve = message
                .get("approve")
                .and_then(|value| value.as_bool())
                .unwrap_or(false);
            answer_needs_you(session, conn, message, Some(id), approve)?;
        }
        "pong" | "error" => {
            let outcome = message
                .get("error")
                .and_then(|value| value.as_str())
                .unwrap_or(kind);
            log::info!(
                "relay: desktop notice host_id={} outcome={outcome}",
                session.identity.host_id
            );
        }
        _ => {
            log::info!(
                "relay: ignored desktop frame host_id={} outcome=unknown_type",
                session.identity.host_id
            );
        }
    }
    Ok(())
}

fn pump_assistant(session: &mut Session, socket: &mut Option<WsConn>) -> Result<(), String> {
    let drained = if let Some(job) = session.inflight.as_mut() {
        let mut batch = Vec::new();
        let mut finished = false;
        loop {
            match job.rx.try_recv() {
                Ok(value) => batch.push(value),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    finished = true;
                    break;
                }
            }
        }
        let ack = !job.acked && !batch.is_empty();
        if ack {
            job.acked = true;
        }
        Some((
            job.delivery_id.clone(),
            job.request_id.clone(),
            job.device_id.clone(),
            ack,
            batch,
            finished,
        ))
    } else {
        None
    };
    if let Some((delivery_id, request_id, device_id, ack, batch, finished)) = drained {
        if ack {
            if let Some(conn) = socket.as_mut() {
                send_json(conn, &json!({"type": "ack", "delivery_id": delivery_id}))?;
            }
        }
        for value in batch {
            forward_assistant_reply(
                session,
                socket,
                &delivery_id,
                &request_id,
                &device_id,
                &value,
            )?;
        }
        if finished {
            session.inflight = None;
        }
    }
    if session.inflight.is_none() {
        if let Some(pending) = session.queued.pop_front() {
            let dispatch = session.dispatch;
            let host_id = session.identity.host_id.clone();
            log::info!(
                "relay: dispatching phone turn host_id={host_id} request_id={} conversation_id={} bytes={}",
                pending.request_id,
                pending.conversation_id,
                pending.text.len()
            );
            let (tx, rx) = mpsc::channel();
            let delivery_id = pending.delivery_id.clone();
            let request_id = pending.request_id.clone();
            let device_id = pending.device_id.clone();
            thread::spawn(move || {
                dispatch_turn(
                    dispatch,
                    &pending.text,
                    &pending.request_id,
                    &pending.conversation_id,
                    pending.join_desktop,
                    tx,
                );
            });
            session.inflight = Some(Inflight {
                delivery_id,
                request_id,
                device_id,
                rx,
                acked: false,
            });
        }
    }
    Ok(())
}

fn forward_assistant_reply(
    session: &mut Session,
    socket: &mut Option<WsConn>,
    delivery_id: &str,
    request_id: &str,
    device_id: &str,
    value: &Value,
) -> Result<(), String> {
    let state = value
        .get("state")
        .and_then(|item| item.as_str())
        .unwrap_or("failed");
    if let Some(returned) = value.get("request_id").and_then(|item| item.as_str()) {
        if returned != request_id && value.get("turn_id").is_none() {
            log::info!(
                "relay: dropped mismatched assistant reply request_id={request_id} outcome=request_mismatch"
            );
            return Ok(());
        }
    }
    log::info!(
        "relay: phone turn update host_id={} request_id={request_id} state={state}",
        session.identity.host_id
    );
    let Some(conn) = socket.as_mut() else {
        return Ok(());
    };
    let mut reply = json!({
        "type": "reply",
        "delivery_id": delivery_id,
        "request_id": request_id,
        "state": state,
    });
    let reply_text = value.get("reply").and_then(|item| item.as_str());
    let error_text = value.get("error").and_then(|item| item.as_str());
    if reply_text.is_some() || error_text.is_some() {
        if let Some(phone) = session.phone_sessions.get_mut(device_id) {
            match relay_crypto::seal_reply(
                phone,
                reply_text.unwrap_or(""),
                error_text,
                request_id,
            ) {
                Ok(sealed) => {
                    if let Err(error) = persist_phone_session(device_id, phone) {
                        log::error!("relay: phone session was not stored outcome={error}");
                    }
                    reply["reply"] = json!(sealed);
                }
                Err(error) => {
                    log::info!(
                        "relay: phone reply seal failed request_id={request_id} outcome={error:?}"
                    );
                    reply["state"] = json!("failed");
                }
            }
        } else {
            log::info!(
                "relay: phone reply dropped request_id={request_id} outcome=no_session"
            );
            reply["state"] = json!("failed");
        }
    }
    if let Some(turn_id) = value.get("turn_id").and_then(|item| item.as_str()) {
        reply["turn_id"] = json!(turn_id);
    }
    if let Some(conversation_id) = value.get("conversation_id").and_then(|item| item.as_str()) {
        reply["conversation_id"] = json!(conversation_id);
    }
    if let Some(status) = value.get("status").and_then(|item| item.as_str()) {
        reply["status"] = json!(status);
    }
    send_json(conn, &reply)
}

/// Phone turns stay on `phone-<host>` unless the phone opted into the desktop.
pub(crate) fn desktop_turn_target(
    conversation_id: &str,
    join_desktop: bool,
) -> (Option<&str>, bool) {
    if join_desktop {
        (None, true)
    } else {
        (Some(conversation_id), false)
    }
}

/// `echo` is the local install check. It never runs against a remote relay,
/// and the host-owned connection does not consult this env var at all.
fn dispatch_from_env(echo_requested: bool, loopback: bool) -> Dispatch {
    if echo_requested && loopback {
        Dispatch::Echo
    } else {
        Dispatch::Host
    }
}

fn dispatch_turn(
    dispatch: Dispatch,
    text: &str,
    request_id: &str,
    conversation_id: &str,
    join_desktop: bool,
    tx: mpsc::Sender<Value>,
) {
    let (conversation, join_desktop) = desktop_turn_target(conversation_id, join_desktop);
    match dispatch {
        Dispatch::Echo => {
            let _ = tx.send(json!({
                "request_id": request_id,
                "turn_id": format!("turn-{request_id}"),
                "conversation_id": conversation.unwrap_or("desktop"),
                "state": "succeeded",
                "reply": format!("echo:{text}"),
            }));
        }
        Dispatch::Host => {
            if join_desktop {
                log::info!(
                    "relay: phone turn continues the desktop conversation request_id={request_id}"
                );
            }
            let first =
                host_assistant_turn(Some(text), request_id, conversation, join_desktop, None);
            let state = first
                .get("state")
                .and_then(|item| item.as_str())
                .unwrap_or("failed")
                .to_string();
            let turn_id = first
                .get("turn_id")
                .and_then(|item| item.as_str())
                .or_else(|| {
                    first
                        .get("pending_request_id")
                        .and_then(|item| item.as_str())
                })
                .unwrap_or("")
                .to_string();
            let _ = tx.send(first);
            if state != "waiting_for_permission" || turn_id.is_empty() {
                return;
            }
            log::info!("relay: polling desktop approval request_id={request_id} turn_id={turn_id}");
            for _ in 0..240 {
                if !host_socket_open() {
                    log::info!("relay: stopped approval poll, host gone turn_id={turn_id}");
                    return;
                }
                thread::sleep(Duration::from_millis(500));
                let polled = host_assistant_turn(
                    None,
                    &uuid::Uuid::new_v4().to_string(),
                    None,
                    false,
                    Some(&turn_id),
                );
                let polled_state = polled
                    .get("state")
                    .and_then(|item| item.as_str())
                    .unwrap_or("failed");
                if polled_state != "waiting_for_permission" {
                    let _ = tx.send(polled);
                    return;
                }
            }
        }
    }
}

fn host_assistant_turn(
    text: Option<&str>,
    request_id: &str,
    conversation_id: Option<&str>,
    join_desktop: bool,
    status_for: Option<&str>,
) -> Value {
    match super::app::assistant_send_result(
        text,
        Some(request_id),
        None,
        None,
        conversation_id,
        join_desktop,
        status_for,
    ) {
        Ok(mut value) => {
            if value.get("request_id").is_none() {
                value["request_id"] = json!(request_id);
            }
            if let Some(conversation_id) = conversation_id {
                if value.get("conversation_id").is_none() {
                    value["conversation_id"] = json!(conversation_id);
                }
            }
            value
        }
        Err(error) => json!({
            "request_id": request_id,
            "conversation_id": conversation_id,
            "state": "failed",
            "error": error,
        }),
    }
}

fn host_socket_open() -> bool {
    let path = crate::config::config_dir().join("notify.sock");
    #[cfg(unix)]
    {
        std::os::unix::net::UnixStream::connect(&path).is_ok()
    }
    #[cfg(not(unix))]
    {
        path.exists()
    }
}

fn poll_control(
    listener: &TcpListener,
    session: &mut Session,
    socket: &mut Option<WsConn>,
) -> Result<(), String> {
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                if let Some(command) = read_control(stream) {
                    apply_control(session, command, socket.as_mut())?;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(error) => return Err(format!("relay control accept: {error}")),
        }
    }
    Ok(())
}

fn apply_control(
    session: &mut Session,
    command: Value,
    conn: Option<&mut WsConn>,
) -> Result<(), String> {
    let kind = command
        .get("type")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    match kind {
        "confirm" => {
            let pairing_id = command
                .get("pairing_id")
                .and_then(|value| value.as_str())
                .map(str::to_string)
                .filter(|id| !id.is_empty())
                .or_else(|| session.pairing_id.clone());
            let Some(pairing_id) = pairing_id else {
                return Ok(());
            };
            if let Some(conn) = conn {
                send_json(
                    conn,
                    &json!({"type": "pair_confirm", "pairing_id": pairing_id}),
                )?;
                log::info!(
                    "relay: confirm sent host_id={} pairing_id={pairing_id}",
                    session.identity.host_id
                );
            }
        }
        "revoke" => {
            let device_id = json_str(&command, "device_id");
            if let Some(conn) = conn {
                send_json(conn, &json!({"type": "revoke", "device_id": device_id}))?;
                log::info!(
                    "relay: revoke sent host_id={} device_id={device_id}",
                    session.identity.host_id
                );
            }
        }
        "pair" => {
            if let Some(conn) = conn {
                send_json(conn, &json!({"type": "pair_start"}))?;
                log::info!(
                    "relay: additional pairing requested host_id={}",
                    session.identity.host_id
                );
            }
        }
        "stop" => {
            STOP.store(true, Ordering::SeqCst);
        }
        _ => {}
    }
    Ok(())
}

fn read_control(mut stream: TcpStream) -> Option<Value> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let mut buf = String::new();
    let mut byte = [0u8; 1];
    loop {
        match stream.read(&mut byte) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                if byte[0] == b'\n' {
                    break;
                }
                buf.push(byte[0] as char);
                if buf.len() > 4096 {
                    break;
                }
            }
        }
    }
    let value = serde_json::from_str::<Value>(&buf).unwrap_or(Value::Null);
    let response = if value.get("type").and_then(|item| item.as_str()).is_some() {
        json!({"ok": true})
    } else {
        json!({"ok": false, "error": "invalid_control"})
    };
    let _ = stream.write_all(format!("{response}\n").as_bytes());
    let _ = stream.shutdown(std::net::Shutdown::Both);
    if value.get("type").is_some() {
        Some(value)
    } else {
        None
    }
}

static STOP: AtomicBool = AtomicBool::new(false);

fn session_stopped() -> bool {
    STOP.load(Ordering::SeqCst)
}

fn control_roundtrip(command: Value) -> i32 {
    let Ok(text) = fs::read_to_string(status_path()) else {
        eprintln!("error: relay connect is not running");
        return 1;
    };
    let status: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    let Some(control) = status.get("control").and_then(|value| value.as_str()) else {
        eprintln!("error: relay connect is not running");
        return 1;
    };
    let mut stream = match TcpStream::connect(control) {
        Ok(stream) => stream,
        Err(error) => {
            eprintln!("error: relay connect is not running ({error})");
            return 1;
        }
    };
    if stream.write_all(format!("{command}\n").as_bytes()).is_err() {
        eprintln!("error: could not reach relay connect");
        return 1;
    }
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let mut buf = String::new();
    let _ = stream.read_to_string(&mut buf);
    println!("{buf}");
    0
}

fn write_status(
    session: &Session,
    phase: &str,
    code: Option<&str>,
    qr_url: Option<&str>,
    fingerprint: Option<&str>,
) -> Result<(), String> {
    let mut value = json!({
        "host_id": session.identity.host_id,
        "control": session.control,
        "phase": phase,
        "owner": session.owner,
        "url_host": session.url.host,
        "url_port": session.url.port,
        "tls": session.url.tls,
        "conversation_id": format!("phone-{}", session.identity.host_id),
    });
    if let Some(pairing_id) = &session.pairing_id {
        value["pairing_id"] = json!(pairing_id);
    }
    value["devices"] = json!(session
        .devices
        .iter()
        .map(|device| json!({
            "device_id": device.device_id,
            "label": device.label,
            "fingerprint": device.fingerprint,
        }))
        .collect::<Vec<_>>());
    if let Some(device) = session.devices.last() {
        value["device_id"] = json!(device.device_id);
    }
    if let Some(code) = code {
        value["code"] = json!(code);
    }
    if let Some(qr_url) = qr_url {
        value["qr_url"] = json!(qr_url);
    }
    if let Some(pending) = &session.pending_key {
        value["key_fingerprint"] = json!(relay_crypto::key_fingerprint(&pending.public));
    }
    if let Some(fingerprint) = fingerprint {
        value["fingerprint"] = json!(fingerprint);
    }
    // Keep a previously issued code when a later status update omits it.
    if code.is_none() {
        if let Ok(previous) = fs::read_to_string(status_path()) {
            if let Ok(previous) = serde_json::from_str::<Value>(&previous) {
                if value.get("code").is_none() {
                    if let Some(existing) = previous.get("code") {
                        value["code"] = existing.clone();
                    }
                }
                if value.get("qr_url").is_none() {
                    if let Some(existing) = previous.get("qr_url") {
                        value["qr_url"] = existing.clone();
                    }
                }
                if fingerprint.is_none() {
                    if let Some(existing) = previous.get("fingerprint") {
                        value["fingerprint"] = existing.clone();
                    }
                }
                if value.get("key_fingerprint").is_none() {
                    if let Some(existing) = previous.get("key_fingerprint") {
                        value["key_fingerprint"] = existing.clone();
                    }
                }
            }
        }
    }
    write_private(
        &status_path(),
        &serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".to_string()),
    )
}

fn token_account(host_id: &str) -> String {
    format!("plexi:user:relay-host-token:{host_id}")
}

fn phone_session_account(device_id: &str) -> String {
    format!("plexi:user:relay-phone-session:{device_id}")
}

fn desktop_key_account(host_id: &str) -> String {
    format!("plexi:user:relay-desktop-key:{host_id}")
}

fn load_desktop_key(host_id: &str) -> Option<relay_crypto::PendingDesktopKey> {
    let raw = crate::workspace::secrets::system_store().get(&desktop_key_account(host_id))?;
    let key = relay_crypto::desktop_key_from_record(&raw)?;
    log::info!("relay: desktop key restored host_id={host_id}");
    Some(key)
}

fn persist_desktop_key(host_id: &str, key: &relay_crypto::PendingDesktopKey) -> Result<(), String> {
    crate::workspace::secrets::system_store()
        .set(
            &desktop_key_account(host_id),
            &relay_crypto::desktop_key_record(key),
        )
        .map_err(|error| format!("relay keychain: {error}"))
}

fn load_phone_sessions(
    devices: &[PairedDevice],
) -> HashMap<String, relay_crypto::PhoneSession> {
    let store = crate::workspace::secrets::system_store();
    let mut sessions = HashMap::new();
    for device in devices {
        let Some(raw) = store.get(&phone_session_account(&device.device_id)) else {
            continue;
        };
        match relay_crypto::PhoneSession::from_record(&raw) {
            Some(phone) => {
                sessions.insert(device.device_id.clone(), phone);
            }
            None => {
                log::info!(
                    "relay: phone session rejected device_id={} outcome=bad_record",
                    device.device_id
                );
            }
        }
    }
    sessions
}

fn persist_phone_session(device_id: &str, phone: &relay_crypto::PhoneSession) -> Result<(), String> {
    crate::workspace::secrets::system_store()
        .set(&phone_session_account(device_id), &phone.to_record())
        .map_err(|error| format!("relay keychain: {error}"))
}

fn accept_sealed(
    session: &mut Session,
    device_id: &str,
    request_id: &str,
    body: &str,
) -> Result<(String, bool), relay_crypto::SealError> {
    if device_id.is_empty() {
        return Err(relay_crypto::SealError::Malformed);
    }
    let opened = relay_crypto::open_phone_body(
        body,
        request_id,
        &mut session.pending_key,
        session.phone_sessions.get_mut(device_id),
    )?;
    match opened {
        relay_crypto::PhoneOpen::New { opened, session: phone } => {
            if let Err(error) = persist_phone_session(device_id, &phone) {
                log::error!("relay: phone session was not stored outcome={error}");
            }
            session.phone_sessions.insert(device_id.to_string(), phone);
            Ok((opened.text, opened.join_desktop))
        }
        relay_crypto::PhoneOpen::Existing { opened } => {
            if let Some(phone) = session.phone_sessions.get(device_id) {
                if let Err(error) = persist_phone_session(device_id, phone) {
                    log::error!("relay: phone session was not stored outcome={error}");
                }
            }
            Ok((opened.text, opened.join_desktop))
        }
    }
}

fn load_identity(
    store: &dyn crate::workspace::secrets::SecretStore,
) -> Result<(Identity, Vec<PairedDevice>), String> {
    let path = identity_path();
    if let Ok(text) = fs::read_to_string(&path) {
        if let Ok(value) = serde_json::from_str::<Value>(&text) {
            if let Some(host_id) = value.get("host_id").and_then(|item| item.as_str()) {
                let label = value
                    .get("label")
                    .and_then(|item| item.as_str())
                    .unwrap_or("desktop")
                    .to_string();
                let devices = devices_from_message(&value);
                let account = token_account(host_id);
                if let Some(file_token) = value.get("host_token").and_then(|item| item.as_str()) {
                    store
                        .set(&account, file_token)
                        .map_err(|error| format!("relay keychain: {error}"))?;
                    let identity = Identity {
                        host_id: host_id.to_string(),
                        host_token: file_token.to_string(),
                        label,
                    };
                    persist_record(&identity, &devices)?;
                    log::info!("relay: moved host token into the keychain host_id={host_id}");
                    return Ok((identity, devices));
                }
                if let Some(token) = store.get(&account) {
                    return Ok((
                        Identity {
                            host_id: host_id.to_string(),
                            host_token: token.to_string(),
                            label,
                        },
                        devices,
                    ));
                }
                return Err(format!(
                    "relay host token for {host_id} is not in the keychain"
                ));
            }
        }
    }
    let identity = Identity {
        host_id: format!("host-{}", uuid::Uuid::new_v4()),
        host_token: uuid::Uuid::new_v4().to_string(),
        label: machine_label(),
    };
    store
        .set(&token_account(&identity.host_id), &identity.host_token)
        .map_err(|error| format!("relay keychain: {error}"))?;
    persist_record(&identity, &[])?;
    log::info!(
        "relay: created desktop identity host_id={}",
        identity.host_id
    );
    Ok((identity, Vec::new()))
}

fn persist_record(identity: &Identity, devices: &[PairedDevice]) -> Result<(), String> {
    let body = json!({
        "host_id": identity.host_id,
        "label": identity.label,
        "devices": devices.iter().map(|device| json!({
            "device_id": device.device_id,
            "label": device.label,
            "fingerprint": device.fingerprint,
        })).collect::<Vec<_>>(),
    });
    write_private(&identity_path(), &body.to_string())
}

fn protocol_problem(message: &Value) -> Option<String> {
    let kind = message
        .get("type")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let mismatch = kind == "error"
        && message.get("error").and_then(|value| value.as_str()) == Some("protocol_mismatch");
    let hello_ok_mismatch = kind == "hello_ok"
        && message.get("protocol").and_then(|value| value.as_i64()) != Some(PROTOCOL_VERSION);
    if mismatch || hello_ok_mismatch {
        let text = message
            .get("message")
            .and_then(|value| value.as_str())
            .filter(|text| !text.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| {
                format!(
                    "This desktop speaks relay protocol {PROTOCOL_VERSION}. The relay speaks a different version. Update both so they match, then pair again."
                )
            });
        Some(text)
    } else {
        None
    }
}

fn reconcile_paired(local: &[PairedDevice], remote: &[PairedDevice]) -> Vec<PairedDevice> {
    remote
        .iter()
        .map(|remote_device| {
            let kept = local
                .iter()
                .find(|device| device.device_id == remote_device.device_id)
                .map(|device| device.label.clone())
                .filter(|label| !label.is_empty());
            let label = if let Some(label) = kept {
                label
            } else if remote_device.label.is_empty() {
                "phone".to_string()
            } else {
                remote_device.label.clone()
            };
            PairedDevice {
                device_id: remote_device.device_id.clone(),
                label,
                fingerprint: remote_device.fingerprint.clone(),
            }
        })
        .collect()
}

fn devices_from_message(message: &Value) -> Vec<PairedDevice> {
    message
        .get("devices")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let device_id = item.get("device_id").and_then(|value| value.as_str())?;
                    if device_id.is_empty() {
                        return None;
                    }
                    Some(PairedDevice {
                        device_id: device_id.to_string(),
                        label: item
                            .get("label")
                            .and_then(|value| value.as_str())
                            .unwrap_or("phone")
                            .to_string(),
                        fingerprint: item
                            .get("fingerprint")
                            .and_then(|value| value.as_str())
                            .unwrap_or("")
                            .to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

const ALREADY_RUNNING: &str = "relay_already_running";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LiveConnection {
    Host,
    Other,
}

pub(crate) fn classify_connection(
    control_open: bool,
    owner: Option<&str>,
) -> Option<LiveConnection> {
    if !control_open {
        None
    } else if owner == Some("host") {
        Some(LiveConnection::Host)
    } else {
        Some(LiveConnection::Other)
    }
}

fn live_connection() -> Option<LiveConnection> {
    let text = fs::read_to_string(status_path()).ok()?;
    let status: Value = serde_json::from_str(&text).ok()?;
    let control = status.get("control").and_then(|value| value.as_str())?;
    let open = TcpStream::connect_timeout(
        &control
            .parse()
            .unwrap_or_else(|_| "127.0.0.1:0".parse().unwrap()),
        Duration::from_millis(200),
    )
    .is_ok();
    classify_connection(open, status.get("owner").and_then(|value| value.as_str()))
}

fn try_session_lock() -> Result<fs::File, String> {
    let path = crate::config::config_dir().join("relay.lock");
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|error| format!("relay lock: {error}"))?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(std::fs::TryLockError::WouldBlock) => Err(ALREADY_RUNNING.to_string()),
        Err(std::fs::TryLockError::Error(error)) => Err(format!("relay lock: {error}")),
    }
}

fn write_relay_config(enabled: bool, url: &str) -> Result<(), String> {
    let body = format!("enabled = {enabled}\nurl = \"{url}\"\n");
    write_private(&crate::config::config_dir().join("relay.toml"), &body)
}

pub(crate) fn relay_enabled() -> bool {
    let Ok(text) = fs::read_to_string(crate::config::config_dir().join("relay.toml")) else {
        return false;
    };
    text.lines().any(|line| {
        let rest = line.trim().strip_prefix("enabled");
        rest.is_some_and(|rest| rest.trim().trim_start_matches('=').trim() == "true")
    })
}

fn configured_url() -> Option<String> {
    if let Ok(url) = std::env::var("PLEXI_RELAY_URL") {
        let url = url.trim();
        if !url.is_empty() {
            return Some(url.to_string());
        }
    }
    let text = fs::read_to_string(crate::config::config_dir().join("relay.toml")).ok()?;
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("url") else {
            continue;
        };
        let rest = rest
            .trim()
            .trim_start_matches('=')
            .trim()
            .trim_matches('"')
            .trim_matches('\'');
        if !rest.is_empty() {
            return Some(rest.to_string());
        }
    }
    None
}

fn write_private(path: &Path, body: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("relay write {}: {error}", path.display()))?;
    }
    fs::write(path, body).map_err(|error| format!("relay write {}: {error}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

fn status_path() -> PathBuf {
    crate::config::config_dir().join(STATUS_FILE)
}

fn identity_path() -> PathBuf {
    crate::config::config_dir().join(IDENTITY_FILE)
}

fn send_json(conn: &mut WsConn, value: &Value) -> Result<(), String> {
    let text = serde_json::to_string(value).map_err(|error| format!("relay encode: {error}"))?;
    conn.send_text(text.as_bytes())
}

fn json_str(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(|item| item.as_str())
        .unwrap_or("")
        .to_string()
}

fn backoff(attempt: u32) {
    let shift = attempt.min(4);
    let base = 500u64.saturating_mul(1u64 << shift).min(8_000);
    let jitter = (std::process::id() as u64)
        .wrapping_mul(17)
        .wrapping_add(attempt as u64 * 13)
        % 400;
    thread::sleep(Duration::from_millis(base + jitter));
}

fn machine_label() -> String {
    let label = hostname_from_os();
    let label = label.trim();
    if label.is_empty() {
        "desktop".to_string()
    } else {
        label.to_string()
    }
}

/// The desktop's machine name, used only as a relay host label.
/// Windows libc does not export `gethostname`; `COMPUTERNAME` is the
/// name the OS already published for this process.
#[cfg(unix)]
fn hostname_from_os() -> String {
    let mut buf = [0u8; 256];
    // SAFETY: gethostname writes a NUL-terminated name into `buf`, which we
    // bound by its length and never read past the first NUL.
    let rc = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
    if rc != 0 {
        return String::new();
    }
    let end = buf.iter().position(|byte| *byte == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}

#[cfg(not(unix))]
fn hostname_from_os() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};
    use std::sync::Mutex;
    use std::time::Duration;

    /// `STOP` and the in-memory test keychain are process-global.
    static SESSION_TEST: Mutex<()> = Mutex::new(());

    #[test]
    fn protocol_mismatch_stops_and_reconcile_keeps_local_labels() {
        let mismatch = json!({
            "type": "error",
            "error": "protocol_mismatch",
            "message": "desktop and relay differ",
        });
        assert!(protocol_problem(&mismatch)
            .unwrap()
            .contains("desktop and relay differ"));
        assert!(protocol_problem(&json!({"type": "hello_ok", "devices": []})).is_some());
        assert!(protocol_problem(&json!({
            "type": "hello_ok",
            "protocol": PROTOCOL_VERSION,
            "devices": [],
        }))
        .is_none());
        let local = vec![PairedDevice {
            device_id: "dev-a".to_string(),
            label: "pixel".to_string(),
            fingerprint: "old".to_string(),
        }];
        let remote = vec![
            PairedDevice {
                device_id: "dev-a".to_string(),
                label: "phone".to_string(),
                fingerprint: "fp-a".to_string(),
            },
            PairedDevice {
                device_id: "dev-b".to_string(),
                label: "phone".to_string(),
                fingerprint: "fp-b".to_string(),
            },
        ];
        let merged = reconcile_paired(&local, &remote);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].label, "pixel");
        assert_eq!(merged[0].fingerprint, "fp-a");
        assert_eq!(merged[1].device_id, "dev-b");
        assert!(reconcile_paired(&local, &[]).is_empty());
    }

    #[test]
    fn phone_turns_stay_off_the_desktop_unless_opted_in() {
        assert_eq!(
            desktop_turn_target("phone-host", false),
            (Some("phone-host"), false)
        );
        assert_eq!(desktop_turn_target("phone-host", true), (None, true));
    }

    #[test]
    fn echo_dispatch_is_loopback_only_and_the_payload_grants_nothing() {
        assert_eq!(dispatch_from_env(true, true), Dispatch::Echo);
        assert_eq!(dispatch_from_env(true, false), Dispatch::Host);
        assert_eq!(dispatch_from_env(false, true), Dispatch::Host);
        let payload = super::super::app::assistant_send_payload(
            &super::super::app::AssistantSendFields {
                text: "hello from the phone",
                request_id: "req-1",
                response_file: "/tmp/response",
                pane_id: None,
                context_id: None,
                conversation: Some("phone-host"),
                join_desktop: false,
                status_for: None,
            },
        );
        assert_eq!(payload["type"], "submit_assistant_turn");
        assert!(payload.get("approved").is_none());
        assert!(payload.get("grant").is_none());
        assert!(payload.get("permission").is_none());
        assert_eq!(payload["join_desktop"], false);
        assert_eq!(payload["conversation_id"], "phone-host");
    }

    #[test]
    fn host_token_moves_out_of_the_profile_file() {
        let profile = tempfile::tempdir().unwrap();
        let _guard = crate::config::set_test_profile_dir(profile.path().to_path_buf());
        let store = crate::workspace::secrets::InMemoryKeychain::new();
        fs::write(
            profile.path().join(IDENTITY_FILE),
            r#"{"host_id":"host-legacy","host_token":"secret-token","label":"desk"}"#,
        )
        .unwrap();
        let (identity, devices) = load_identity(&store).unwrap();
        assert_eq!(identity.host_id, "host-legacy");
        assert_eq!(identity.host_token, "secret-token");
        assert!(devices.is_empty());
        let text = fs::read_to_string(profile.path().join(IDENTITY_FILE)).unwrap();
        assert!(!text.contains("secret-token"));
        assert!(!text.contains("host_token"));
        let (again, _) = load_identity(&store).unwrap();
        assert_eq!(again.host_id, "host-legacy");
        assert_eq!(again.host_token, "secret-token");
    }

    #[test]
    fn a_live_host_connection_attaches_and_a_cli_connection_is_busy() {
        assert_eq!(
            classify_connection(true, Some("host")),
            Some(LiveConnection::Host)
        );
        assert_eq!(
            classify_connection(true, Some("cli")),
            Some(LiveConnection::Other)
        );
        assert_eq!(classify_connection(false, Some("host")), None);
    }

    #[test]
    fn enable_persists_the_url_the_host_reads() {
        let profile = tempfile::tempdir().unwrap();
        let _guard = crate::config::set_test_profile_dir(profile.path().to_path_buf());
        assert!(!relay_enabled());
        assert_eq!(
            relay_enable_cli(Some("ws://127.0.0.1:9/v1/desktop".to_string())),
            0
        );
        assert!(relay_enabled());
        assert_eq!(
            configured_url().as_deref(),
            Some("ws://127.0.0.1:9/v1/desktop")
        );
        assert_eq!(relay_disable_cli(), 0);
        assert!(!relay_enabled());
    }

    #[test]
    fn a_second_session_does_not_open_another_socket() {
        let _session = SESSION_TEST.lock().unwrap();
        let profile = tempfile::tempdir().unwrap();
        let _guard = crate::config::set_test_profile_dir(profile.path().to_path_buf());
        STOP.store(false, Ordering::SeqCst);
        let profile_path = profile.path().to_path_buf();
        let worker = thread::spawn(move || {
            let _guard = crate::config::set_test_profile_dir(profile_path);
            run_session("ws://127.0.0.1:1/v1/desktop", Dispatch::Echo, "cli")
        });
        let status = wait_status(profile.path(), "control");
        assert_eq!(status["owner"], "cli");
        let profile_path = profile.path().to_path_buf();
        let error = thread::spawn(move || {
            let _guard = crate::config::set_test_profile_dir(profile_path);
            run_session("ws://127.0.0.1:1/v1/desktop", Dispatch::Echo, "cli")
                .expect_err("second session")
        })
        .join()
        .unwrap();
        assert_eq!(error, ALREADY_RUNNING);
        assert_eq!(
            relay_connect_cli(Some("ws://127.0.0.1:1/v1/desktop".to_string())),
            1
        );
        assert_eq!(control_roundtrip(json!({"type": "stop"})), 0);
        worker.join().unwrap().unwrap();
    }

    #[test]
    fn echo_round_trip_correlates_request_and_conversation() {
        let _session = SESSION_TEST.lock().unwrap();
        let profile = tempfile::tempdir().unwrap();
        let _guard = crate::config::set_test_profile_dir(profile.path().to_path_buf());
        STOP.store(false, Ordering::SeqCst);
        let port = free_port();
        let mut child = Command::new("python3")
            .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("services/relay/relay.py"))
            .arg("--host")
            .arg("127.0.0.1")
            .arg("--port")
            .arg(port.to_string())
            .arg("--public-origin")
            .arg(format!("http://127.0.0.1:{port}"))
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("python3");
        wait_until(Duration::from_secs(5), || health(port));
        let profile_path = profile.path().to_path_buf();
        let url = format!("ws://127.0.0.1:{port}/v1/desktop");
        let worker = thread::spawn(move || {
            let _guard = crate::config::set_test_profile_dir(profile_path);
            run_session(&url, Dispatch::Echo, "cli").expect("session");
        });
        let status = wait_status(profile.path(), "code");
        let code = status["code"].as_str().unwrap().to_string();
        let pairing_id = status["pairing_id"].as_str().unwrap().to_string();
        let (pair_status, pending, _) = http(
            "POST",
            &format!("http://127.0.0.1:{port}/api/pair"),
            Some(json!({"code": code, "label": "curl-phone"})),
            None,
        );
        assert_eq!(pair_status, 202, "{pending}");
        let _ = wait_status(profile.path(), "fingerprint");
        let confirm = control_roundtrip(json!({"type": "confirm", "pairing_id": pairing_id}));
        assert_eq!(confirm, 0);
        let mut cookie = None;
        for _ in 0..50 {
            let (status, body, token) = http(
                "GET",
                &format!("http://127.0.0.1:{port}/api/pair/{pairing_id}"),
                None,
                None,
            );
            if status == 200 && body["status"] == "confirmed" {
                cookie = token;
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        let cookie = cookie.expect("session cookie");
        let marker = "CANARY-host-roundtrip-6c1e";
        let state = profile.path().join("phone.json");
        let (turn_status, queued, _) = http(
            "POST",
            &format!("http://127.0.0.1:{port}/api/turns"),
            Some(seal_turn(profile.path(), &state, "req-echo", marker)),
            Some(&cookie),
        );
        assert_eq!(turn_status, 202, "{queued}");
        let mut saw = false;
        for _ in 0..50 {
            let (_status, page, _) = http(
                "GET",
                &format!("http://127.0.0.1:{port}/api/conversation?after=0"),
                None,
                Some(&cookie),
            );
            if reply_has(&page, &state, "req-echo", &format!("echo:{marker}")) {
                saw = true;
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        assert!(saw, "phone did not see the correlated echo");
        let _ = control_roundtrip(json!({"type": "stop"}));
        let _ = worker.join();
        let _ = child.kill();
        let _ = child.wait();
    }

    #[test]
    fn reconnect_resumes_the_paired_device_without_a_new_code() {
        let _session = SESSION_TEST.lock().unwrap();
        let profile = tempfile::tempdir().unwrap();
        let _guard = crate::config::set_test_profile_dir(profile.path().to_path_buf());
        STOP.store(false, Ordering::SeqCst);
        let port = free_port();
        let mut child = Command::new("python3")
            .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("services/relay/relay.py"))
            .arg("--host")
            .arg("127.0.0.1")
            .arg("--port")
            .arg(port.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("python3");
        wait_until(Duration::from_secs(5), || health(port));
        let url = format!("ws://127.0.0.1:{port}/v1/desktop");
        let profile_path = profile.path().to_path_buf();
        let worker = thread::spawn({
            let url = url.clone();
            let profile_path = profile_path.clone();
            move || {
                let _guard = crate::config::set_test_profile_dir(profile_path);
                run_session(&url, Dispatch::Echo, "cli").expect("session");
            }
        });
        let status = wait_status(profile.path(), "code");
        let code = status["code"].as_str().unwrap().to_string();
        let pairing_id = status["pairing_id"].as_str().unwrap().to_string();
        let (pair_status, _, _) = http(
            "POST",
            &format!("http://127.0.0.1:{port}/api/pair"),
            Some(json!({"code": code, "label": "first"})),
            None,
        );
        assert_eq!(pair_status, 202);
        let _ = wait_status(profile.path(), "fingerprint");
        assert_eq!(
            control_roundtrip(json!({"type": "confirm", "pairing_id": pairing_id})),
            0
        );
        let mut cookie = None;
        for _ in 0..50 {
            let (status, body, token) = http(
                "GET",
                &format!("http://127.0.0.1:{port}/api/pair/{pairing_id}"),
                None,
                None,
            );
            if status == 200 && body["status"] == "confirmed" {
                cookie = token;
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        let cookie = cookie.expect("cookie");
        let device_id = wait_status(profile.path(), "device_id")["device_id"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(control_roundtrip(json!({"type": "pair"})), 0);
        let second = wait_for_phase(profile.path(), "waiting_for_phone");
        let second_code = second["code"].as_str().unwrap().to_string();
        let second_pairing = second["pairing_id"].as_str().unwrap().to_string();
        assert_ne!(second_code, code);
        let (second_status, _, _) = http(
            "POST",
            &format!("http://127.0.0.1:{port}/api/pair"),
            Some(json!({"code": second_code, "label": "second"})),
            None,
        );
        assert_eq!(second_status, 202);
        let _ = wait_for_phase(profile.path(), "pending_confirm");
        assert_eq!(
            control_roundtrip(json!({"type": "confirm", "pairing_id": second_pairing})),
            0
        );
        let mut second_cookie = None;
        for _ in 0..50 {
            let (status, body, token) = http(
                "GET",
                &format!("http://127.0.0.1:{port}/api/pair/{second_pairing}"),
                None,
                None,
            );
            if status == 200 && body["status"] == "confirmed" {
                second_cookie = token;
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        let second_cookie = second_cookie.expect("second cookie");
        assert_eq!(control_roundtrip(json!({"type": "stop"})), 0);
        worker.join().unwrap();
        STOP.store(false, Ordering::SeqCst);
        let worker = thread::spawn(move || {
            let _guard = crate::config::set_test_profile_dir(profile_path);
            run_session(&url, Dispatch::Echo, "cli").expect("resume");
        });
        let resumed = wait_for_phase(profile.path(), "paired");
        let listed = resumed["devices"].as_array().unwrap();
        let ids: Vec<&str> = listed
            .iter()
            .filter_map(|device| device["device_id"].as_str())
            .collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&device_id.as_str()));
        let identity = fs::read_to_string(profile.path().join(IDENTITY_FILE)).unwrap();
        assert!(!identity.contains("host_token"));
        let first_state = profile.path().join("phone-first.json");
        let second_state = profile.path().join("phone-second.json");
        let (turn_status, _, _) = http(
            "POST",
            &format!("http://127.0.0.1:{port}/api/turns"),
            Some(seal_turn(profile.path(), &first_state, "req-resume", "still paired")),
            Some(&cookie),
        );
        assert_eq!(turn_status, 202);
        let (other_status, _, _) = http(
            "POST",
            &format!("http://127.0.0.1:{port}/api/turns"),
            Some(seal_turn(
                profile.path(),
                &second_state,
                "req-other",
                "second phone",
            )),
            Some(&second_cookie),
        );
        assert_eq!(other_status, 202);
        assert_eq!(
            control_roundtrip(json!({"type": "revoke", "device_id": device_id})),
            0
        );
        thread::sleep(Duration::from_millis(200));
        let (revoked_status, _, _) = http(
            "POST",
            &format!("http://127.0.0.1:{port}/api/turns"),
            Some(seal_turn(profile.path(), &first_state, "req-revoked", "gone")),
            Some(&cookie),
        );
        assert_eq!(revoked_status, 401);
        let (kept_status, _, _) = http(
            "POST",
            &format!("http://127.0.0.1:{port}/api/turns"),
            Some(seal_turn(profile.path(), &second_state, "req-kept", "kept")),
            Some(&second_cookie),
        );
        assert_eq!(kept_status, 202);
        let _ = control_roundtrip(json!({"type": "stop"}));
        let _ = worker.join();
        let _ = child.kill();
        let _ = child.wait();
    }

    fn seal_turn(profile: &Path, state: &Path, request_id: &str, text: &str) -> Value {
        let output = Command::new("python3")
            .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("services/relay/phone_crypto.py"))
            .arg("seal")
            .arg("--status")
            .arg(profile.join(STATUS_FILE))
            .arg("--state")
            .arg(state)
            .arg("--request-id")
            .arg(request_id)
            .arg("--text")
            .arg(text)
            .output()
            .expect("python phone_crypto");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).expect("seal json")
    }

    fn reply_has(page: &Value, state: &Path, request_id: &str, needle: &str) -> bool {
        let mut child = Command::new("python3")
            .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("services/relay/phone_crypto.py"))
            .arg("saw")
            .arg("--state")
            .arg(state)
            .arg("--request-id")
            .arg(request_id)
            .arg("--needle")
            .arg(needle)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("python phone_crypto");
        {
            let stdin = child.stdin.as_mut().expect("stdin");
            serde_json::to_writer(stdin, page).expect("page");
        }
        child.wait().expect("saw").success()
    }

    fn wait_for_phase(profile: &Path, phase: &str) -> Value {
        let path = profile.join(STATUS_FILE);
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            if let Ok(text) = fs::read_to_string(&path) {
                if let Ok(value) = serde_json::from_str::<Value>(&text) {
                    if value.get("phase").and_then(|item| item.as_str()) == Some(phase) {
                        return value;
                    }
                }
            }
            thread::sleep(Duration::from_millis(30));
        }
        panic!("status never reached {phase} at {}", path.display());
    }

    fn free_port() -> u16 {
        TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    fn health(port: u16) -> bool {
        let (status, body, _) = http(
            "GET",
            &format!("http://127.0.0.1:{port}/healthz"),
            None,
            None,
        );
        status == 200 && body["ok"] == true
    }

    fn wait_until(budget: Duration, mut ready: impl FnMut() -> bool) {
        let start = Instant::now();
        while start.elapsed() < budget {
            if ready() {
                return;
            }
            thread::sleep(Duration::from_millis(30));
        }
        panic!("timed out");
    }

    fn wait_status(profile: &Path, key: &str) -> Value {
        let path = profile.join(STATUS_FILE);
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            if let Ok(text) = fs::read_to_string(&path) {
                if let Ok(value) = serde_json::from_str::<Value>(&text) {
                    if value.get(key).and_then(|item| item.as_str()).is_some() {
                        return value;
                    }
                }
            }
            thread::sleep(Duration::from_millis(30));
        }
        panic!("status missing {key} at {}", path.display());
    }

    fn http(
        method: &str,
        url: &str,
        body: Option<Value>,
        cookie: Option<&str>,
    ) -> (u16, Value, Option<String>) {
        let mut request = match method {
            "POST" => ureq::post(url),
            _ => ureq::get(url),
        };
        if let Some(cookie) = cookie {
            request = request.set("Cookie", &format!("plexi_phone={cookie}"));
        }
        let result = if let Some(body) = body {
            request
                .set("Content-Type", "application/json")
                .send_string(&body.to_string())
        } else if method == "POST" {
            request.send_string("")
        } else {
            request.call()
        };
        match result {
            Ok(response) => read_ureq(response.status(), response),
            Err(ureq::Error::Status(code, response)) => read_ureq(code, response),
            Err(error) => (0, json!({"error": error.to_string()}), None),
        }
    }

    fn read_ureq(status: u16, response: ureq::Response) -> (u16, Value, Option<String>) {
        let cookie = response
            .header("set-cookie")
            .and_then(|header| header.split(';').next())
            .and_then(|part| part.trim().strip_prefix("plexi_phone="))
            .map(str::to_string);
        let text = response.into_string().unwrap_or_default();
        let value = serde_json::from_str(&text).unwrap_or(Value::Null);
        (status, value, cookie)
    }
}
