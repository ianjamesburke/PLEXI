//! AT-P1 acceptance coverage for the single permission monitor.
//!
//! Chess cases load `apps/chess` and drive the production assistant turn,
//! host MCP parser, and socket identity handler. They do not call
//! `chess_domain.play` directly.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::app::app_trait::App;
use crate::broker::gate::{
    fingerprint_args, gate_log_contains, AdmitRequest, Admission, ApprovalChoice, PermissionMonitor,
};
use crate::broker::{
    ActorScope, ActorType, Decision, ExactBinding, GrantDuration, GrantRecord, GrantSource,
    ResourceScope, TargetType,
};
use crate::plexi_ai::tool_dispatch::{self, AppEventSender, DispatchScope, ToolCallResult, ToolDispatcher};
use crate::protocol::{AiTool, AppRequest};
use crate::testing::HostHarness;

fn play(rev: i64, op: &str, mv: &str, game: &str) -> serde_json::Value {
    serde_json::json!({
        "game_id": game,
        "expected_revision": rev,
        "operation_id": op,
        "move": mv,
    })
}

fn events_since(start: usize) -> Vec<crate::host::app_timeline::AppEventRecord> {
    let timeline = crate::host::app_timeline::global();
    let timeline = timeline.lock().unwrap();
    timeline.events().iter().skip(start).cloned().collect()
}

fn event_len() -> usize {
    crate::host::app_timeline::global()
        .lock()
        .unwrap()
        .events()
        .len()
}

fn commits_for(start: usize, op: &str) -> usize {
    events_since(start)
        .iter()
        .filter(|record| {
            record.event == "chess.move_committed"
                && (record.caused_by.as_deref() == Some(op)
                    || record
                        .payload
                        .as_ref()
                        .and_then(|payload| payload.get("operation_id"))
                        .and_then(|value| value.as_str())
                        == Some(op))
        })
        .count()
}

/// The socket reply can return before `chess.move_committed` is on the
/// process-global timeline. A slow runner (macos-x64) observed revision 1
/// with `commits_for` still 0.
fn expect_commits(h: &mut HostHarness, start: usize, op: &str, n: usize) {
    if n > 0 && commits_for(start, op) < n {
        pump_until(h, |_| commits_for(start, op) >= n);
    }
    assert_eq!(commits_for(start, op), n, "commits for {op}");
}

fn mcp_error_code(body: &serde_json::Value) -> Option<String> {
    let text = body.pointer("/result/content/0/text")?.as_str()?;
    let parsed: serde_json::Value = serde_json::from_str(text).ok()?;
    parsed
        .pointer("/error/code")
        .and_then(|value| value.as_str())
        .map(str::to_string)
}

fn post_mcp(port: u16, token: &str, body: &[u8]) -> serde_json::Value {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    let mut stream = TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
    let req = format!(
        "POST /mcp HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(req.as_bytes()).unwrap();
    stream.write_all(body).unwrap();
    stream.flush().unwrap();
    let mut resp = Vec::new();
    let _ = stream.read_to_end(&mut resp);
    let text = String::from_utf8_lossy(&resp);
    let start = text.find("\r\n\r\n").map(|index| index + 4).unwrap_or(0);
    serde_json::from_str(&text[start..]).unwrap_or_else(|error| {
        panic!("mcp reply was not JSON ({error}): {text}")
    })
}

fn pump_until(h: &mut HostHarness, mut done: impl FnMut(&mut HostHarness) -> bool) {
    let started = Instant::now();
    while !done(h) {
        h.run_frames(1);
        assert!(
            started.elapsed() < super::load_aware_timeout(Duration::from_secs(40)),
            "timed out waiting for a chess reply"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn socket_call(
    h: &mut HostHarness,
    caller: Option<u64>,
    credential: Option<&str>,
    ancestry: &[u32],
    input: serde_json::Value,
) -> serde_json::Value {
    socket_call_pane(h, caller, credential, ancestry, None, input)
}

fn socket_call_pane(
    h: &mut HostHarness,
    caller: Option<u64>,
    credential: Option<&str>,
    ancestry: &[u32],
    target_pane: Option<u64>,
    input: serde_json::Value,
) -> serde_json::Value {
    let reply = h.workspace_root().join(format!("socket-{}.json", uuid::Uuid::new_v4()));
    let request = AppRequest::CallAppTool {
        wait_approved: false,
        app_id: "chess".to_string(),
        tool: "chess.play".to_string(),
        input_json: input.to_string(),
        caller_pane_id: caller,
        call_credential: credential.map(str::to_string),
        peer_ancestry: ancestry.to_vec(),
        target_pane_id: target_pane,
        response_file: Some(reply.to_string_lossy().to_string()),
    };
    let line = serde_json::to_string(&request).unwrap();
    let mut ack = Vec::new();
    crate::app::handle_socket_line(&line, &h.ipc_tx, Some(ancestry), &mut ack);
    pump_until(h, |_| reply.is_file() && std::fs::metadata(&reply).map(|meta| meta.len() > 0).unwrap_or(false));
    serde_json::from_str(&std::fs::read_to_string(&reply).unwrap()).unwrap()
}

fn assistant_begin(h: &mut HostHarness, assistant: u64, input: &serde_json::Value) -> PathBuf {
    let reply = h
        .workspace_root()
        .join(format!("assistant-{}.json", uuid::Uuid::new_v4()));
    h.assistant_mut(assistant)
        .queue_scripted_tool("chess.play", &input.to_string());
    App::submit_external_turn(
        h.assistant_mut(assistant),
        "play the move".to_string(),
        format!("req-{}", uuid::Uuid::new_v4()),
        reply.display().to_string(),
        crate::app::app_trait::ExternalTurnOpts {
            conversation_id: None,
            join_desktop: true,
            status_for: None,
            client: None,
            kind: None,
        },
    )
    .unwrap();
    reply
}

fn assistant_pending(h: &mut HostHarness, assistant: u64) -> String {
    pump_until(h, |h| {
        h.assistant_mut(assistant)
            .model
            .pending_permission
            .is_some()
    });
    h.assistant_mut(assistant)
        .model
        .pending_permission
        .as_ref()
        .unwrap()
        .pending_request_id
        .clone()
}

fn assistant_resolve(h: &mut HostHarness, assistant: u64, reply: &Path, choice: crate::assistant::model::PermissionChoice) -> serde_json::Value {
    h.assistant_mut(assistant).resolve_permission(choice);
    pump_until(h, |_| reply_is_terminal(reply));
    serde_json::from_str(&std::fs::read_to_string(reply).unwrap()).unwrap()
}

/// The phone ack writes `waiting_for_permission` as soon as the sheet opens.
/// The same file is replaced when the turn finishes.
fn reply_is_terminal(reply: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(reply) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return false;
    };
    value.get("state").and_then(|state| state.as_str()) != Some("waiting_for_permission")
}

fn human_e7e5(h: &mut HostHarness, pane: u64) {
    for key in ["up", "up", "up", "up", "up", "enter", "down", "down", "enter"] {
        let response = h.workspace_root().join(format!("key-{key}-{}.json", uuid::Uuid::new_v4()));
        h.inject_ipc(AppRequest::KeyPane {
            pane_id: pane,
            key: key.to_string(),
            response_file: Some(response.to_string_lossy().to_string()),
        });
        pump_until(h, |_| response.is_file());
        h.run_frames(4);
    }
}

fn monitor() -> Arc<PermissionMonitor> {
    PermissionMonitor::for_profile(&crate::config::config_dir())
}

/// `PlexiApp::new_for_test` gives every harness the same context id, and the
/// process-global tool registry treats that id as one audience. Pane-id blocks
/// stop two tests from overwriting the same key; they do not stop a second
/// chess pane in another harness from withdrawing the bare `chess.play` name
/// or from receiving a call this harness is not pumping. Move this harness's
/// window and its router context together, before any app registers tools.
fn isolate_tool_context(h: &mut HostHarness) {
    static NEXT: AtomicU64 = AtomicU64::new(80_000);
    let context_id = NEXT.fetch_add(1, Ordering::SeqCst);
    let window_idx = h.app.active_window;
    h.app.windows[window_idx].context_id = context_id;
    let router_idx = h.app.router.active_idx();
    h.app.router.get_mut(router_idx).context_id = context_id;
    log::info!("permission_gate: isolated tool context {context_id}");
}

#[derive(Clone, Copy)]
enum Ingress {
    Assistant,
    Mcp,
    Socket,
}

#[test]
fn permission_gate_real_chess_all_ingresses() {
    for ingress in [Ingress::Assistant, Ingress::Mcp, Ingress::Socket] {
        run_ingress(ingress);
    }
}

fn run_ingress(ingress: Ingress) {
    let mut h = HostHarness::new();
    isolate_tool_context(&mut h);
    let pane = h.launch_repo_app("apps/chess", &[]);
    let ctx = h.app.windows[h.app.active_window].context_id;
    let workspace = h.workspace_root();
    let assistant = h.add_assistant_pane();
    let mon = monitor();
    let credential = mon.issue_credential(Some(pane), ctx, &workspace, &format!("pane:{pane}"));
    let (mcp_port, mcp_token) = start_mcp(&h, ctx, &workspace);
    let start = event_len();
    let op = format!("gate-{}-e4", uuid::Uuid::new_v4());
    let input = play(0, &op, "e2e4", "game-1");

    // No grant: the board does not change.
    match ingress {
        Ingress::Assistant => {
            let reply = assistant_begin(&mut h, assistant, &input);
            let pending = assistant_pending(&mut h, assistant);
            assert!(!pending.is_empty());
            assert_eq!(commits_for(start, &op), 0);
            let _ = assistant_resolve(
                &mut h,
                assistant,
                &reply,
                crate::assistant::model::PermissionChoice::Deny,
            );
        }
        Ingress::Mcp => {
            let body = mcp_call(&mut h, mcp_port, &mcp_token, &input);
            assert_eq!(mcp_error_code(&body).as_deref(), Some("permission_required"), "{body}");
            assert_eq!(commits_for(start, &op), 0);
        }
        Ingress::Socket => {
            let body = socket_call(&mut h, Some(pane), Some(&credential), &[], input.clone());
            assert_eq!(body["error_code"], "permission_required", "{body}");
            assert_eq!(commits_for(start, &op), 0);
            let _ = body["pending_request_id"].as_str().unwrap();
        }
    }

    // Wrong resource stays a question and does not move.
    let wrong_game = play(0, &format!("{op}-game"), "e2e4", "game-missing");
    refuse_without_commit(&mut h, ingress, assistant, mcp_port, &mcp_token, pane, &credential, &wrong_game, start);

    // Exact approval commits once, with the ingress actor on the receipt.
    let approved = play(0, &op, "e2e4", "game-1");
    let committed = approve_and_commit(
        &mut h,
        ingress,
        assistant,
        mcp_port,
        &mcp_token,
        pane,
        &credential,
        &approved,
    );
    let output = committed_output(&committed);
    assert_eq!(output["revision_after"], 1, "{committed}");
    assert_eq!(output["move"], "e2e4", "{committed}");
    assert!(
        output["fen"].as_str().unwrap_or("").contains("4P3"),
        "board fen must show e4: {output}"
    );
    let actor = output["actor"].as_str().unwrap_or("");
    match ingress {
        Ingress::Assistant => assert_eq!(actor, "agent:default", "{output}"),
        Ingress::Mcp => assert!(actor.starts_with("mcp:pane:"), "{output}"),
        Ingress::Socket => assert_eq!(actor, format!("pane:{pane}"), "{output}"),
    }
    expect_commits(&mut h, start, &op, 1);
    let intruder = h.add_test_pane();
    let intruder_credential =
        mon.issue_credential(Some(intruder), ctx, &workspace, &format!("pane:{intruder}"));
    let wrong_actor = socket_call(
        &mut h,
        Some(intruder),
        Some(&intruder_credential),
        &[],
        approved.clone(),
    );
    assert_eq!(
        wrong_actor["error_code"], "permission_required",
        "a different actor cannot use the grant: {wrong_actor}"
    );
    assert_eq!(commits_for(start, &op), 1, "wrong actor must not move");
    let facts = mon.audit_records();
    assert!(
        facts.iter().any(|fact| {
            fact.kind == "use" && fact.actor == actor && fact.operation_id == op
        }),
        "audit links the actor and operation: {facts:?}"
    );

    // The same operation returns the original receipt and does not move again.
    let duplicate = approve_and_commit(
        &mut h,
        ingress,
        assistant,
        mcp_port,
        &mcp_token,
        pane,
        &credential,
        &approved,
    );
    let dup_output = committed_output(&duplicate);
    assert_eq!(dup_output["duplicate"], true, "{duplicate}");
    assert_eq!(commits_for(start, &op), 1, "duplicate must not move twice");

    // A changed argument is a different call.
    let changed = play(1, &format!("{op}-changed"), "d2d4", "game-1");
    refuse_without_commit(&mut h, ingress, assistant, mcp_port, &mcp_token, pane, &credential, &changed, start);

    // Revoke the grant that would have authorized a repeat, then refuse.
    let grant_id = mon
        .store()
        .records()
        .iter()
        .find(|record| record.actor_id == actor && record.decision == Decision::Allow)
        .map(|record| record.grant_id.clone());
    if let Some(grant_id) = grant_id {
        assert!(mon.revoke_grant_id(&grant_id), "revoke {grant_id}");
    }
    refuse_without_commit(&mut h, ingress, assistant, mcp_port, &mcp_token, pane, &credential, &changed, start);

    // A pending move goes stale when a human plays while approval waits.
    let stale_op = format!("{op}-stale");
    let stale_input = play(1, &stale_op, "e7e5", "game-1");
    match ingress {
        Ingress::Assistant => {
            let reply = assistant_begin(&mut h, assistant, &stale_input);
            let _ = assistant_pending(&mut h, assistant);
            human_e7e5(&mut h, pane);
            let settled = assistant_resolve(
                &mut h,
                assistant,
                &reply,
                crate::assistant::model::PermissionChoice::AllowOnce,
            );
            let text = settled.to_string();
            assert!(text.contains("stale_revision"), "{settled}");
        }
        Ingress::Mcp | Ingress::Socket => {
            let first = match ingress {
                Ingress::Mcp => mcp_call(&mut h, mcp_port, &mcp_token, &stale_input),
                Ingress::Socket => socket_call(&mut h, Some(pane), Some(&credential), &[], stale_input.clone()),
                Ingress::Assistant => unreachable!(),
            };
            human_e7e5(&mut h, pane);
            mon.approve_pending(&pending_id_of(&first), ApprovalChoice::Once)
                .expect("approve the pending stale move");
            let second = match ingress {
                Ingress::Mcp => mcp_call(&mut h, mcp_port, &mcp_token, &stale_input),
                Ingress::Socket => socket_call(&mut h, Some(pane), Some(&credential), &[], stale_input),
                Ingress::Assistant => unreachable!(),
            };
            let text = second.to_string();
            assert!(text.contains("stale_revision"), "{second}");
        }
    }
    assert_eq!(commits_for(start, &stale_op), 0, "stale move must not commit");
    log::info!("permission_gate: ingress case finished");
}

fn pending_id_of(reply: &serde_json::Value) -> String {
    if let Some(id) = reply.get("pending_request_id").and_then(|value| value.as_str()) {
        return id.to_string();
    }
    let text = reply
        .pointer("/result/content/0/text")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let parsed: serde_json::Value = serde_json::from_str(text).unwrap_or(serde_json::Value::Null);
    parsed
        .pointer("/error/pending_request_id")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .to_string()
}

fn committed_output(reply: &serde_json::Value) -> serde_json::Value {
    if let Some(output) = reply.get("output") {
        return output.clone();
    }
    if let Some(reply_text) = reply.get("reply").and_then(|value| value.as_str()) {
        return serde_json::from_str(reply_text).unwrap_or(serde_json::Value::String(reply_text.to_string()));
    }
    if let Some(text) = reply.pointer("/result/content/0/text").and_then(|value| value.as_str()) {
        return serde_json::from_str(text).unwrap_or(serde_json::Value::String(text.to_string()));
    }
    reply.clone()
}

fn refuse_without_commit(
    h: &mut HostHarness,
    ingress: Ingress,
    assistant: u64,
    port: u16,
    token: &str,
    pane: u64,
    credential: &str,
    input: &serde_json::Value,
    start: usize,
) {
    let before = commits_for(start, input["operation_id"].as_str().unwrap_or(""));
    match ingress {
        Ingress::Assistant => {
            let reply = assistant_begin(h, assistant, input);
            let _ = assistant_pending(h, assistant);
            let settled = assistant_resolve(h, assistant, &reply, crate::assistant::model::PermissionChoice::Deny);
            let text = settled.to_string();
            assert!(
                text.contains("permission_denied") || text.contains("denied"),
                "{settled}"
            );
        }
        Ingress::Mcp => {
            let body = mcp_call(h, port, token, input);
            let code = mcp_error_code(&body).unwrap_or_default();
            assert!(
                code == "permission_required" || code == "permission_denied",
                "{body}"
            );
        }
        Ingress::Socket => {
            let body = socket_call(h, Some(pane), Some(credential), &[], input.clone());
            let code = body["error_code"].as_str().unwrap_or("");
            assert!(
                code == "permission_required" || code == "permission_denied",
                "{body}"
            );
        }
    }
    assert_eq!(commits_for(start, input["operation_id"].as_str().unwrap_or("")), before);
}

fn approve_and_commit(
    h: &mut HostHarness,
    ingress: Ingress,
    assistant: u64,
    port: u16,
    token: &str,
    pane: u64,
    credential: &str,
    input: &serde_json::Value,
) -> serde_json::Value {
    match ingress {
        Ingress::Assistant => {
            let reply = assistant_begin(h, assistant, input);
            if h.assistant_mut(assistant).model.pending_permission.is_none() {
                pump_until(h, |h| {
                    h.assistant_mut(assistant).model.pending_permission.is_some()
                        || std::fs::metadata(&reply).map(|meta| meta.len() > 0).unwrap_or(false)
                });
            }
            if h.assistant_mut(assistant).model.pending_permission.is_some() {
                assistant_resolve(h, assistant, &reply, crate::assistant::model::PermissionChoice::AllowOnce)
            } else {
                serde_json::from_str(&std::fs::read_to_string(&reply).unwrap()).unwrap()
            }
        }
        Ingress::Mcp => {
            let first = mcp_call(h, port, token, input);
            if let Some(id) = nonempty(&pending_id_of(&first)) {
                monitor()
                    .approve_pending(&id, ApprovalChoice::Once)
                    .expect("approve mcp move");
                mcp_call(h, port, token, input)
            } else {
                first
            }
        }
        Ingress::Socket => {
            let first = socket_call(h, Some(pane), Some(credential), &[], input.clone());
            if let Some(id) = nonempty(&pending_id_of(&first)) {
                monitor()
                    .approve_pending(&id, ApprovalChoice::Once)
                    .expect("approve socket move");
                socket_call(h, Some(pane), Some(credential), &[], input.clone())
            } else {
                first
            }
        }
    }
}

fn nonempty(id: &str) -> Option<String> {
    if id.is_empty() { None } else { Some(id.to_string()) }
}

fn start_mcp(h: &HostHarness, context_id: u64, workspace: &Path) -> (u16, String) {
    use crate::app::ui_mailbox::UiMailbox;
    use crate::host::event_subscriptions::HostSubscribeRequest;
    let (tx, _rx) = UiMailbox::<HostSubscribeRequest>::channel(
        Arc::new(crate::app::ui_mailbox::RecordingWake::new()),
        "permission-gate-mcp",
    );
    let port = crate::app::host_mcp::start_host_mcp_server(tx, &crate::config::config_dir()).unwrap();
    let token = crate::app::host_mcp::register_pane_credential_for_test(
        880_000 + (context_id % 1000),
        context_id,
        workspace.to_path_buf(),
    );
    let _ = h;
    (port, token)
}

fn mcp_call(
    h: &mut HostHarness,
    port: u16,
    token: &str,
    input: &serde_json::Value,
) -> serde_json::Value {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": { "name": "chess__chess.play", "arguments": input }
    });
    let (sender, receiver) = std::sync::mpsc::channel();
    let token = token.to_string();
    let bytes = body.to_string();
    std::thread::spawn(move || {
        sender.send(post_mcp(port, &token, bytes.as_bytes())).ok();
    });
    let started = Instant::now();
    loop {
        if let Ok(value) = receiver.try_recv() {
            return value;
        }
        h.run_frames(1);
        assert!(
            started.elapsed() < super::load_aware_timeout(Duration::from_secs(40)),
            "mcp call timed out"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn app_call_waits_for_human_then_dispatches_exactly_once() {
    assert_waiting_app_call(ApprovalChoice::Once);
}

#[test]
fn app_call_wait_ends_on_human_denial_without_execution() {
    assert_waiting_app_call(ApprovalChoice::Deny);
}

fn assert_waiting_app_call(choice: ApprovalChoice) {
    let mut h = HostHarness::new();
    let caller = h.add_test_pane();
    let target = h.add_test_pane();
    let workspace = h.workspace_root();
    let context_id = h.app.windows[h.app.active_window].context_id;
    let mon = monitor();
    let credential = mon.issue_credential(Some(caller), context_id, &workspace, &format!("pane:{caller}"));
    let (tx, rx) = std::sync::mpsc::channel();
    tool_dispatch::register(target, "wait-probe".into(), vec![AiTool {
        name: "probe.add".into(), description: "add".into(),
        input_schema: serde_json::json!({"type":"object"}),
        output_schema: serde_json::json!({"type":"object"}),
        timeout_ms: Some(1000), read_only: false,
    }], AppEventSender::Channel(tx), crate::host::scope::ScopeOrigin {
        context_id, context_root: workspace.clone(), window_id: 1,
        pane_id: target, app_id: Some("wait-probe".into()),
    });
    let reply = workspace.join("wait-reply.json");
    let input = r#"{"text":"Buy groceries"}"#;
    let request: AppRequest = serde_json::from_value(serde_json::json!({
        "type":"call_app_tool", "app_id":"wait-probe", "tool":"probe.add",
        "input_json":input, "caller_pane_id":caller, "call_credential":credential,
        "peer_ancestry":[std::process::id()], "target_pane_id":target,
        "wait_approved":true, "response_file":reply,
    })).unwrap();
    h.inject_ipc(request);
    pump_until(&mut h, |_| !mon.list_pending().is_empty());
    std::thread::sleep(Duration::from_millis(50));
    assert!(!reply.exists(), "waiting call must not return permission_required");
    assert!(rx.try_recv().is_err(), "no execution before human approval");
    mon.approve_pending(&mon.list_pending()[0].pending_request_id, choice).unwrap();
    if choice == ApprovalChoice::Deny {
        pump_until(&mut h, |_| reply.is_file());
        let body: serde_json::Value = serde_json::from_slice(&std::fs::read(reply).unwrap()).unwrap();
        assert_eq!(body["error_code"], "permission_denied", "{body}");
        assert!(rx.try_recv().is_err());
        tool_dispatch::unregister(target);
        return;
    }
    let event: crate::protocol::PlexiEvent = serde_json::from_str(&rx.recv_timeout(Duration::from_secs(2)).unwrap()).unwrap();
    let crate::protocol::PlexiEvent::ToolCall { call_id, input_json, .. } = event else {
        panic!("expected tool call");
    };
    assert_eq!(input_json, input);
    tool_dispatch::resolve_pending(&call_id, ToolCallResult::ok(r#"{"added":true}"#));
    pump_until(&mut h, |_| reply.is_file());
    let body: serde_json::Value = serde_json::from_slice(&std::fs::read(reply).unwrap()).unwrap();
    assert_eq!(body["ok"], true, "{body}; attention={:?}", mon.open_needs_you());
    assert!(rx.try_recv().is_err(), "exactly one execution");
    tool_dispatch::unregister(target);
}

#[test]
fn all_dispatchers_require_monitor() {
    let mut h = HostHarness::new();
    let workspace = h.workspace_root();
    let context_id = h.app.windows[h.app.active_window].context_id;
    let mon = PermissionMonitor::ephemeral();
    let hits = Arc::new(AtomicU64::new(0));
    let hits_for_handler = Arc::clone(&hits);
    let mut assistant = ToolDispatcher::from_registry(
        DispatchScope::new(1, "agent:assistant", workspace.clone(), context_id),
        Arc::clone(&mon),
    );
    assistant.add_host_tools(
        vec![AiTool {
            name: "host.apps.open".to_string(),
            description: "open an app".to_string(),
            input_schema: serde_json::json!({"type":"object"}),
            output_schema: serde_json::json!({"type":"object"}),
            timeout_ms: Some(1_000),
            read_only: true,
        }],
        Arc::new(move |_name, _input| {
            hits_for_handler.fetch_add(1, Ordering::SeqCst);
            ToolCallResult::ok(r#"{"ok":true}"#)
        }),
    );
    let blocked = assistant.dispatch_call(
        "launch-1".to_string(),
        "host.apps.open",
        r#"{"type_id":"chess"}"#.to_string(),
    );
    assert_eq!(blocked.error_code.as_deref(), Some("permission_required"));
    assert_eq!(hits.load(Ordering::SeqCst), 0, "read_only must not skip the prompt");

    let (tx, _rx) = std::sync::mpsc::channel();
    tool_dispatch::register(
        910_101,
        "probe".to_string(),
        vec![AiTool {
            name: "probe.ping".to_string(),
            description: "ping".to_string(),
            input_schema: serde_json::json!({"type":"object"}),
            output_schema: serde_json::json!({"type":"object"}),
            timeout_ms: Some(1_000),
            read_only: true,
        }],
        AppEventSender::Channel(tx),
        crate::host::scope::ScopeOrigin {
            context_id,
            context_root: workspace.clone(),
            window_id: 1,
            pane_id: 910_101,
            app_id: Some("probe".to_string()),
        },
    );
    // AgentHost::gated_dispatcher is private and builds this same constructor.
    // Discovery is the snapshot taken when the dispatcher is built.
    let agent = ToolDispatcher::from_registry(
        DispatchScope::new(2, "agent:chess-opponent", workspace.clone(), context_id),
        Arc::clone(&mon),
    );
    let discovered = agent.all_tools().iter().any(|tool| tool.name.contains("probe"));
    assert!(discovered, "discovery lists the tool");
    let args = r#"{"n":1}"#;
    let binding = ExactBinding {
        actor_type: ActorType::Agent,
        actor_id: "agent:chess-opponent".to_string(),
        actor_scope: ActorScope::User,
        trust_origin: "host".to_string(),
        workspace_root: workspace.clone(),
        target_type: TargetType::AppConnector,
        target_id: "probe.ping".to_string(),
        resource_scope: ResourceScope::Workspace,
        resource_id: None,
        args_fingerprint: fingerprint_args(args).unwrap(),
        session_id: None,
        package_id: "probe".to_string(),
        instance_id: Some(910_101),
        context_id: Some(context_id),
        call_id: String::new(),
        operation_id: String::new(),
    };
    mon.store().record(GrantRecord::from_binding(
        &binding,
        Decision::Allow,
        GrantDuration::Always,
        GrantSource::User,
        "grant-probe",
    ));
    assert!(mon.revoke_grant_id("grant-probe"));
    let refused = agent.dispatch_call("probe-1".to_string(), "probe.ping", args.to_string());
    assert_eq!(refused.error_code.as_deref(), Some("permission_required"));

    let decision = crate::host::event_subscriptions::evaluate_subscription(
        &mon.store(),
        None,
        Some(&workspace),
        "chess",
        ActorType::Agent,
        "agent:assistant",
        ActorScope::User,
        None,
        &["chess.move_committed".to_string()],
    );
    assert_ne!(decision, Decision::Allow, "a subscription without an exact grant is not allowed");

    let credential = monitor().issue_credential(Some(1), context_id, &workspace, "pane:1");
    let reply = h.workspace_root().join("app-call-blocked.json");
    h.inject_ipc(AppRequest::CallAppTool {
        wait_approved: false,
        app_id: "probe".to_string(),
        tool: "probe.ping".to_string(),
        input_json: args.to_string(),
        caller_pane_id: Some(1),
        call_credential: Some(credential),
        peer_ancestry: Vec::new(),
        target_pane_id: None,
        response_file: Some(reply.to_string_lossy().to_string()),
    });
    pump_until(&mut h, |_| reply.is_file());
    let body: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&reply).unwrap()).unwrap();
    assert_eq!(body["error_code"], "permission_required", "{body}");

    let (mcp_port, mcp_token) = start_mcp(&h, context_id, &workspace);
    let probe_body = {
        let raw = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "tools/call",
            "params": { "name": "probe__probe.ping", "arguments": {"n": 1} }
        });
        post_mcp(mcp_port, &mcp_token, raw.to_string().as_bytes())
    };
    assert_eq!(
        mcp_error_code(&probe_body).as_deref(),
        Some("permission_required"),
        "host MCP tools/call must ask: {probe_body}"
    );
    tool_dispatch::unregister(910_101);
    log::info!("permission_gate: every dispatcher asked");
}

#[test]
fn app_call_rejects_forged_or_missing_identity() {
    let mut h = HostHarness::new();
    isolate_tool_context(&mut h);
    let pane = h.launch_repo_app("apps/chess", &[]);
    let ctx = h.app.windows[h.app.active_window].context_id;
    let workspace = h.workspace_root();
    let start = event_len();
    let input = play(0, "forged-op", "e2e4", "game-1");
    let other = h.add_test_pane();

    let forged = socket_call(&mut h, Some(other), None, &[], input.clone());
    assert_eq!(forged["error_code"], "permission_denied", "{forged}");

    let missing_pane = socket_call(&mut h, Some(9_999_999), None, &[], input.clone());
    assert_eq!(missing_pane["error_code"], "permission_denied", "{missing_pane}");

    let absent = socket_call(&mut h, None, None, &[], input.clone());
    assert_eq!(absent["error_code"], "permission_denied", "{absent}");

    let stale = socket_call(&mut h, Some(pane), Some("cred_not_issued"), &[], input.clone());
    assert_eq!(stale["error_code"], "permission_denied", "{stale}");

    let term = h.add_focused_terminal();
    let pid = h.terminal_backend(term).child_pid();
    let mismatched = socket_call(&mut h, Some(pane), None, &[pid], input.clone());
    assert_eq!(mismatched["error_code"], "permission_denied", "{mismatched}");

    let session = monitor().issue_credential(None, ctx, &workspace, "session:phone");
    let asked = socket_call(&mut h, None, Some(&session), &[], input.clone());
    assert_eq!(asked["error_code"], "permission_required", "{asked}");
    let pending = monitor().show_pending(asked["pending_request_id"].as_str().unwrap()).unwrap();
    assert_eq!(pending.actor_id, "session:phone");
    assert_ne!(pending.actor_id, "user");
    assert_eq!(commits_for(start, "forged-op"), 0);
    log::info!("permission_gate: forged identities denied");
}

#[test]
fn chess_receipt_survives_lost_reply() {
    let mut h = HostHarness::new();
    isolate_tool_context(&mut h);
    let pane = h.launch_repo_app("apps/chess", &[]);
    let ctx = h.app.windows[h.app.active_window].context_id;
    let workspace = h.workspace_root();
    let mon = monitor();
    let credential = mon.issue_credential(Some(pane), ctx, &workspace, &format!("pane:{pane}"));
    let start = event_len();
    let op = "lost-reply-op";
    let input = play(0, op, "e2e4", "game-1");
    let first = socket_call(&mut h, Some(pane), Some(&credential), &[], input.clone());
    assert_eq!(first["error_code"], "permission_required", "{first}");
    mon.approve_pending(first["pending_request_id"].as_str().unwrap(), ApprovalChoice::Once)
        .unwrap();
    let committed = socket_call(&mut h, Some(pane), Some(&credential), &[], input.clone());
    assert_eq!(committed["output"]["revision_after"], 1, "{committed}");
    // The caller drops that reply. The retry is the same operation.
    let recovered = socket_call(&mut h, Some(pane), Some(&credential), &[], input.clone());
    assert_eq!(recovered["output"]["duplicate"], true, "{recovered}");
    expect_commits(&mut h, start, op, 1);

    mon.fail_audit(true);
    let blocked = socket_call(&mut h, Some(pane), Some(&credential), &[], input.clone());
    mon.fail_audit(false);
    assert_eq!(
        blocked["error_code"], "permission_denied",
        "audit failure before a consumed-once retry blocks execution: {blocked}"
    );
    assert_eq!(commits_for(start, op), 1, "the blocked retry must not move again");

    let encoded = crate::broker::gate::structured_error("stale_revision", "call-1", None);
    let parsed: serde_json::Value = serde_json::from_str(&encoded).unwrap();
    assert_eq!(parsed["error"]["code"], "stale_revision");
    let cli = serde_json::json!({
        "ok": false,
        "error": encoded,
        "error_code": "edit_conflict",
        "pending_request_id": "req_1",
    });
    let round: serde_json::Value = serde_json::from_str(&cli.to_string()).unwrap();
    assert_eq!(round["error_code"], "edit_conflict");
    assert_eq!(round["pending_request_id"], "req_1");
    let mcp = serde_json::json!({
        "result": { "content": [{ "type": "text", "text": crate::broker::gate::structured_error("operation_conflict", "c", None) }], "isError": true }
    });
    assert_eq!(mcp_error_code(&mcp).as_deref(), Some("operation_conflict"));
    let tool = ToolCallResult::coded("outcome_unknown", "c2", None);
    assert_eq!(tool.error_code.as_deref(), Some("outcome_unknown"));
    log::info!("permission_gate: lost reply recovered as one mutation");
}

#[test]
fn app_call_and_mcp_are_gated_and_audited() {
    let mut h = HostHarness::new();
    isolate_tool_context(&mut h);
    let pane = h.launch_repo_app("apps/chess", &[]);
    let ctx = h.app.windows[h.app.active_window].context_id;
    let workspace = h.workspace_root();
    let mon = monitor();
    let credential = mon.issue_credential(Some(pane), ctx, &workspace, &format!("pane:{pane}"));
    let start = event_len();
    let input = play(0, "ungated-bc4", "b1c3", "game-1");

    let called = socket_call(&mut h, Some(pane), Some(&credential), &[], input.clone());
    assert_eq!(called["error_code"], "permission_required", "{called}");
    assert!(!called.to_string().contains("holds no seat"), "{called}");
    assert!(!called.to_string().contains("\"user\""), "{called}");
    assert_eq!(commits_for(start, "ungated-bc4"), 0);
    assert!(
        mon.audit_records().iter().any(|fact| fact.kind == "ask" && fact.actor == format!("pane:{pane}")),
        "app call must write an ask audit row: {:?}",
        mon.audit_records()
    );
    assert!(
        gate_log_contains("op=ungated-bc4"),
        "the ask for this call must reach the host log"
    );

    let (port, token) = start_mcp(&h, ctx, &workspace);
    let mcp = mcp_call(&mut h, port, &token, &input);
    assert_eq!(mcp_error_code(&mcp).as_deref(), Some("permission_required"), "{mcp}");
    assert!(!mcp.to_string().contains("holds no seat"), "{mcp}");
    assert!(
        mon.audit_records().iter().any(|fact| fact.kind == "ask" && fact.actor.starts_with("mcp:pane:")),
        "MCP must write an ask audit row: {:?}",
        mon.audit_records()
    );
    log::info!("permission_gate: app call and MCP asked and audited");
}

#[test]
fn second_chess_pane_routes_by_instance() {
    let mut h = HostHarness::new();
    isolate_tool_context(&mut h);
    let first = h.launch_repo_app("apps/chess", &[]);
    let again = h.launch_repo_app("apps/chess", &[]);
    assert_eq!(again, first, "app open chess focuses the open board");
    assert_eq!(chess_panes(&h).len(), 1, "one chess pane per context");

    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("apps/chess");
    let second = h
        .app
        .launch_app_by_path_forced(&path.to_string_lossy(), Some(h.workspace_root()), &[])
        .expect("forced second chess pane")
        .expect("pane id");
    h.wait_for_first_render(second);
    h.wait_for_exposed_tools(second);
    pump_until(&mut h, |harness| {
        chess_play_names(harness)
            .iter()
            .filter(|name| name.contains(':'))
            .count()
            >= 2
    });
    let panes = chess_panes(&h);
    assert_eq!(panes.len(), 2, "a forced second instance stays open: {panes:?}");
    assert!(panes.contains(&first) && panes.contains(&second));

    let ctx = h.app.windows[h.app.active_window].context_id;
    let workspace = h.workspace_root();
    let mon = monitor();
    let credential = mon.issue_credential(Some(first), ctx, &workspace, &format!("pane:{first}"));
    let start = event_len();
    let input = play(0, "route-e4", "e2e4", "game-1");
    let ambiguous = socket_call(&mut h, Some(first), Some(&credential), &[], input.clone());
    assert!(
        ambiguous.to_string().contains("ambiguous_instance"),
        "bare chess.play must not guess a pane: {ambiguous}"
    );
    assert!(!ambiguous.to_string().contains("tool_not_found"), "{ambiguous}");
    assert_eq!(commits_for(start, "route-e4"), 0);
    assert!(gate_log_contains("ambiguous"), "ambiguous route is logged");

    let asked = socket_call_pane(
        &mut h,
        Some(first),
        Some(&credential),
        &[],
        Some(first),
        input.clone(),
    );
    assert_eq!(asked["error_code"], "permission_required", "{asked}");
    assert!(!asked.to_string().contains("holds no seat"), "{asked}");
    mon.approve_pending(asked["pending_request_id"].as_str().unwrap(), ApprovalChoice::Once)
        .unwrap();
    let committed = socket_call_pane(
        &mut h,
        Some(first),
        Some(&credential),
        &[],
        Some(first),
        input,
    );
    assert_eq!(committed["output"]["revision_after"], 1, "{committed}");
    assert_eq!(committed["output"]["actor"], format!("pane:{first}"));
    expect_commits(&mut h, start, "route-e4", 1);
    log::info!("permission_gate: second chess pane addressed by instance");
}

fn chess_play_names(h: &HostHarness) -> Vec<String> {
    let ctx = h.app.windows[h.app.active_window].context_id;
    let scope = DispatchScope::new(0, "probe", h.workspace_root(), ctx);
    ToolDispatcher::namespaced_for(scope, monitor())
        .all_tools()
        .into_iter()
        .map(|tool| tool.name)
        .filter(|name| name.contains("chess.play"))
        .collect()
}

fn chess_panes(h: &HostHarness) -> Vec<u64> {
    h.app
        .windows
        .iter()
        .flat_map(|window| window.panes.iter())
        .filter_map(|(id, pane)| {
            pane.as_app()
                .filter(|app| app.manifest_id == "chess")
                .map(|_| *id)
        })
        .collect()
}

#[test]
fn gate_denial_reaches_the_host_log() {
    let mut h = HostHarness::new();
    isolate_tool_context(&mut h);
    let pane = h.launch_repo_app("apps/chess", &[]);
    let start = event_len();
    let denied = socket_call(&mut h, Some(9_999_999), None, &[], play(0, "ghost-log", "e2e4", "game-1"));
    assert_eq!(denied["error_code"], "permission_denied", "{denied}");
    assert!(!denied.to_string().contains("holds no seat"), "{denied}");
    assert!(
        gate_log_contains("actor=pane:9999999"),
        "a gate denial must be an info log line"
    );
    assert!(
        monitor().audit_records().iter().any(|fact| fact.kind == "deny"),
        "a gate denial must be an audit row"
    );
    assert_eq!(commits_for(start, "ghost-log"), 0);
    let _ = pane;
    log::info!("permission_gate: denial logged");
}

fn refuse_rows(mon: &PermissionMonitor, pending_id: &str) -> usize {
    mon.audit_records()
        .iter()
        .filter(|fact| {
            fact.kind == "refuse"
                && fact.decision == "refused_resolve"
                && fact.call_id == pending_id
        })
        .count()
}

/// Socket resolve, a sheet key, and a click on the approval button are
/// refused. The pending stays, and the desktop `approve_pending` path still
/// grants.
#[test]
fn socket_and_synthetic_resolve_are_refused() {
    let mut h = HostHarness::new();
    let assistant = h.add_assistant_pane();
    h.run_frames(2);
    let workspace = h.workspace_root();
    let args = r#"{"game_id":"game-1","move":"e2e4","expected_revision":0,"operation_id":"op-socket-refuse"}"#;
    let mon = monitor();
    let pending_id = match mon.admit(AdmitRequest {
        call_id: "call-socket-refuse",
        tool: "chess.play",
        input_json: args,
        actor_type: ActorType::Agent,
        actor_id: "agent:chess",
        actor_scope: ActorScope::User,
        trust_origin: "host",
        workspace_root: &workspace,
        context_id: 1,
        package_id: "chess",
        instance_id: 1,
        target_type: TargetType::AppConnector,
    }) {
        Admission::Required { pending_request_id } => pending_request_id,
        Admission::Proceed { .. } => panic!("expected a pending ask, grant matched"),
        Admission::Denied { code } => panic!("expected a pending ask, denied {code}"),
    };

    let resolve_path = workspace.join("resolve.json");
    h.inject_ipc(AppRequest::ResolvePermissionRequest {
        pending_request_id: pending_id.clone(),
        choice: "always".to_string(),
        response_file: resolve_path.to_string_lossy().to_string(),
    });
    pump_until(&mut h, |_| resolve_path.is_file());
    let resolve_body = std::fs::read_to_string(&resolve_path).unwrap();
    assert!(
        resolve_body.contains("permission_denied"),
        "{resolve_body}"
    );
    assert!(mon.show_pending(&pending_id).is_some(), "resolve leaves the pending");

    let key_path = workspace.join("key.json");
    h.inject_ipc(AppRequest::KeyPane {
        pane_id: assistant,
        key: "enter".to_string(),
        response_file: Some(key_path.to_string_lossy().to_string()),
    });
    pump_until(&mut h, |_| key_path.is_file());
    let key_body = std::fs::read_to_string(&key_path).unwrap();
    assert!(key_body.contains("permission_denied"), "{key_body}");

    let (win, tile) = h
        .app
        .find_pane_in_any_window(assistant)
        .expect("assistant tile");
    let rect = h.app.windows[win]
        .tree
        .tiles
        .rect(tile)
        .expect("assistant rect");
    let abs = rect.min + egui::vec2(12.0, 8.0);
    h.app.approval_buttons.push(crate::app::ApprovalButton {
        label: "Allow once".to_string(),
        bounds: [abs.x - 4.0, abs.y - 4.0, abs.x + 24.0, abs.y + 12.0],
        pending_request_id: pending_id.clone(),
    });
    let click_path = workspace.join("click.json");
    h.inject_ipc(AppRequest::ClickPane {
        pane_id: assistant,
        x: 12.0,
        y: 8.0,
        button: Some("left".to_string()),
        response_file: Some(click_path.to_string_lossy().to_string()),
    });
    pump_until(&mut h, |_| click_path.is_file());
    let click_body = std::fs::read_to_string(&click_path).unwrap();
    assert!(click_body.contains("permission_denied"), "{click_body}");

    assert!(
        refuse_rows(&mon, &pending_id) >= 3,
        "one refuse row per attempt, got {}",
        refuse_rows(&mon, &pending_id)
    );
    assert!(mon.show_pending(&pending_id).is_some(), "the pending stays");
    assert!(
        mon.approve_pending(&pending_id, ApprovalChoice::Once).is_ok(),
        "the desktop path can still approve"
    );
    log::info!("permission_gate: socket and synthetic resolve refused");
}

#[test]
fn agents_chess_approval_scene() {
    let scene = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/scenes/agents-chess-approval.toml");
    let out = std::env::temp_dir().join(format!("plexi-chess-approval-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&out);
    let report = crate::scenes::run_scene(&scene, &out, false);
    assert!(
        report.passed,
        "agents-chess-approval scene failed: {}",
        serde_json::to_string_pretty(&report).unwrap_or_default()
    );
    log::info!("permission_gate: chess approval scene passed");
}

#[test]
fn list_app_tools_snapshots_live_registry_for_context() {
    let mut h = HostHarness::new();
    isolate_tool_context(&mut h);
    let pane = h.launch_repo_app("apps/chess", &[]);
    let ctx = h.app.windows[h.app.active_window].context_id;
    pump_until(&mut h, |_| {
        crate::plexi_ai::tool_dispatch::pane_has_registered_tools(pane)
    });

    let query = |h: &mut HostHarness, app_id: Option<&str>, context_id: Option<u64>| -> serde_json::Value {
        let reply = h
            .workspace_root()
            .join(format!("app-tools-{}.json", uuid::Uuid::new_v4()));
        let req = AppRequest::ListAppTools {
            app_id: app_id.map(str::to_string),
            context_id,
            caller_pane_id: Some(pane),
            caller_context_id: Some(ctx),
            peer_ancestry: Vec::new(),
            response_file: reply.to_string_lossy().to_string(),
        };
        let line = serde_json::to_string(&req).unwrap();
        let mut ack = Vec::new();
        crate::app::handle_socket_line(&line, &h.ipc_tx, None, &mut ack);
        pump_until(h, |_| {
            reply.is_file()
                && std::fs::metadata(&reply)
                    .map(|meta| meta.len() > 0)
                    .unwrap_or(false)
        });
        serde_json::from_str(&std::fs::read_to_string(&reply).unwrap()).unwrap()
    };

    let all = query(&mut h, None, None);
    assert_eq!(all["ok"], true, "{all}");
    assert_eq!(all["context_id"], ctx, "{all}");
    let tools = all["tools"].as_array().expect("tools array");
    let play = tools
        .iter()
        .find(|t| t["name"] == "chess.play")
        .expect("chess.play in live tool registry snapshot");
    assert_eq!(play["provider_name"], "chess.play");
    assert_eq!(play["app_id"], "chess");
    assert_eq!(play["pane_id"], pane);
    assert_eq!(play["context_id"], ctx);
    assert!(!play["description"].as_str().unwrap_or("").is_empty());
    assert_eq!(play["input_schema"]["type"], "object");
    assert_eq!(play["output_schema"]["type"], "object");

    let filtered_empty = query(&mut h, Some("nonexistent-app"), None);
    assert_eq!(filtered_empty["ok"], true);
    assert_eq!(filtered_empty["tools"].as_array().unwrap().len(), 0);

    let unknown_ctx = query(&mut h, None, Some(999_999));
    assert_eq!(unknown_ctx["ok"], false);
    assert!(unknown_ctx["error"]
        .as_str()
        .unwrap_or("")
        .contains("context 999999 not found"));
}

#[test]
fn create_head_approve_mints_persisted_pane_scoped_grant_and_retry_succeeds() {
    use crate::broker::gate::{NeedsYouDetails, NeedsYouFile, NeedsYouKind, ResolveSurface, RunOutcome};
    use sha2::{Digest, Sha256};

    let mut h = HostHarness::new();
    let pane = h.add_test_pane();
    let other_pane = h.add_test_pane();
    let workspace = h.workspace_root();
    let profile_dir = crate::config::config_dir();

    let call_create_head =
        |h: &mut HostHarness, head_name: &str, caller_pane: u64| -> serde_json::Value {
            let reply = h
                .workspace_root()
                .join(format!("create-head-{}.json", uuid::Uuid::new_v4()));
            let req = AppRequest::AgentsApi {
                op: "create_head".to_string(),
                payload: serde_json::json!({
                    "workspace": h.workspace_root(),
                    "name": head_name,
                    "grants": ["agents.ping=allow"],
                    "caller_pane": caller_pane,
                }),
                response_file: reply.to_string_lossy().to_string(),
            };
            let line = serde_json::to_string(&req).unwrap();
            let mut ack = Vec::new();
            crate::app::handle_socket_line(&line, &h.ipc_tx, None, &mut ack);
            pump_until(h, |_| {
                reply.is_file()
                    && std::fs::metadata(&reply)
                        .map(|meta| meta.len() > 0)
                        .unwrap_or(false)
            });
            serde_json::from_str(&std::fs::read_to_string(&reply).unwrap()).unwrap()
        };

    // 1. From a head-less pane, `create_head` with `agents.ping=allow` is denied
    //    and files exactly one `permission_request` needs-you item.
    let first = call_create_head(&mut h, "lead", pane);
    assert_eq!(first["ok"], false, "{first}");
    assert_eq!(first["error_code"], "permission_denied", "{first}");
    let needs_you_id = first["needs_you_id"]
        .as_str()
        .expect("refusal must return needs_you_id")
        .to_string();

    let mon = monitor();
    let open_before = mon.list_needs_you();
    assert_eq!(open_before.len(), 1, "{open_before:?}");
    assert_eq!(open_before[0].id, needs_you_id);
    assert_eq!(open_before[0].kind, NeedsYouKind::PermissionRequest);

    // 2. Approve the permission request from the desktop surface.
    let receipt = mon
        .resolve_needs_you_from(&needs_you_id, true, ResolveSurface::Desktop)
        .expect("desktop approve must succeed");
    assert_eq!(receipt.run_outcome, RunOutcome::Unblocked);

    // Grants file is written and `permission-integrity.toml` has matching `grants_sha256` != "absent".
    let grants_path = profile_dir.join("grants.toml");
    assert!(grants_path.is_file(), "grants.toml must be written on approve");
    let grants_bytes = std::fs::read(&grants_path).unwrap();
    let expected_sha = format!("{:x}", Sha256::digest(&grants_bytes));
    let stamp_raw =
        std::fs::read_to_string(profile_dir.join("permission-integrity.toml")).unwrap();
    let stamp_val: toml::Value = toml::from_str(&stamp_raw).unwrap();
    let stamped_grants_sha = stamp_val
        .get("grants_sha256")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert_ne!(stamped_grants_sha, "absent", "{stamp_raw}");
    assert_eq!(stamped_grants_sha, expected_sha, "{stamp_raw}");

    // Audit `grant` fact (and `needs_you` fact) carries a non-empty `grant_id`.
    let audit_rows = mon.audit_records();
    let grant_fact = audit_rows
        .iter()
        .find(|row| row.kind == "grant" && row.resource_id == "agents.ping")
        .expect("expected grant audit fact for agents.ping");
    assert!(
        !grant_fact.grant_id.is_empty(),
        "audit grant fact must carry non-empty grant_id: {grant_fact:?}"
    );
    let ny_fact = audit_rows
        .iter()
        .find(|row| row.kind == "needs_you" && row.operation_id == needs_you_id)
        .expect("expected needs_you audit fact");
    assert_eq!(ny_fact.grant_id, grant_fact.grant_id);

    // 3. Retry the same `create_head` from the same pane -> `ok: true`, no second needs-you filed.
    let needs_you_total_before_retry = mon.all_needs_you().len();
    let retry = call_create_head(&mut h, "lead", pane);
    assert_eq!(retry["ok"], true, "retry after approve must succeed: {retry}");
    assert!(
        mon.list_needs_you().is_empty(),
        "no open needs-you after successful retry: {:?}",
        mon.list_needs_you()
    );
    assert_eq!(
        mon.all_needs_you().len(),
        needs_you_total_before_retry,
        "retry must not file a second needs-you item"
    );

    // 4. The minted grant does not leak to a different pane.
    let other_pane_try = call_create_head(&mut h, "lead-other-pane", other_pane);
    assert_eq!(
        other_pane_try["error_code"], "permission_denied",
        "different pane must not inherit pane-scoped grant: {other_pane_try}"
    );

    // 5. The minted grant does not leak across a new host session.
    let reloaded = PermissionMonitor::reload_for_test(&profile_dir);
    assert_ne!(reloaded.session_id(), mon.session_id());
    let next_session_try = call_create_head(&mut h, "lead-next-session", pane);
    assert_eq!(
        next_session_try["error_code"], "permission_denied",
        "new host session must not inherit session-scoped pane grant: {next_session_try}"
    );

    // 6. Approving a PermissionRequest with neither run nor pane does not claim `unblocked`.
    let orphan_id = reloaded
        .file_needs_you_with_details(
            NeedsYouFile {
                kind: NeedsYouKind::PermissionRequest,
                actor: "pane".to_string(),
                resource: "agents.ping".to_string(),
                summary: "orphan request without run or pane".to_string(),
                expires_at: None,
                run_tag: None,
            },
            Some(NeedsYouDetails {
                workspace: Some(workspace.display().to_string()),
                tool: Some("agents.ping".to_string()),
                ..Default::default()
            }),
        )
        .unwrap()
        .0;
    let orphan_receipt = reloaded
        .resolve_needs_you_from(&orphan_id, true, ResolveSurface::Desktop)
        .expect("resolve orphan permission_request");
    assert_eq!(
        orphan_receipt.run_outcome,
        RunOutcome::OutcomeUnknown,
        "approve with neither run nor pane must not claim unblocked: {orphan_receipt:?}"
    );
}

#[test]
fn human_pane_creates_head_in_own_context_and_spawned_head_is_narrowed_by_grants() {
    let mut h = HostHarness::new();
    let human_pane = h.add_focused_terminal();
    let human_pid = h.terminal_backend(human_pane).child_pid();
    assert!(
        h.app.is_human_owned_pane(human_pane),
        "host-spawned user terminal must be human-owned"
    );
    let workspace = h.workspace_root();
    let mon = monitor();

    let call_agents_api = |h: &mut HostHarness,
                           op: &str,
                           payload: serde_json::Value,
                           ancestry: Option<&[u32]>|
     -> serde_json::Value {
        let reply = h
            .workspace_root()
            .join(format!("agents-api-{op}-{}.json", uuid::Uuid::new_v4()));
        let req = AppRequest::AgentsApi {
            op: op.to_string(),
            payload,
            response_file: reply.to_string_lossy().to_string(),
        };
        let line = serde_json::to_string(&req).unwrap();
        let mut ack = Vec::new();
        crate::app::handle_socket_line(&line, &h.ipc_tx, ancestry, &mut ack);
        pump_until(h, |_| {
            reply.is_file()
                && std::fs::metadata(&reply)
                    .map(|meta| meta.len() > 0)
                    .unwrap_or(false)
        });
        serde_json::from_str(&std::fs::read_to_string(&reply).unwrap()).unwrap()
    };

    // 1. Human pane in context A runs `agent head create lead --grant agents.ping=allow`
    //    -> `ok: true`, no needs-you filed.
    let created = call_agents_api(
        &mut h,
        "create_head",
        serde_json::json!({
            "workspace": workspace,
            "name": "lead",
            "grants": ["agents.ping=allow"],
            "caller_pane": human_pane,
        }),
        Some(&[human_pid]),
    );
    assert_eq!(created["ok"], true, "human pane create_head must succeed: {created}");
    assert!(
        mon.list_needs_you().is_empty(),
        "human pane create_head must not file needs-you: {:?}",
        mon.list_needs_you()
    );

    // 2. Spawn a run on `lead` and verify it only has `agents.ping` (narrowed by grants).
    let run_reply = call_agents_api(
        &mut h,
        "spawn_run",
        serde_json::json!({
            "workspace": workspace,
            "head": "lead",
            "admission": "adm-0888-lead",
            "journal_only": true,
        }),
        Some(&[human_pid]),
    );
    assert_eq!(run_reply["ok"], true, "{run_reply}");
    let run_id = run_reply["run"]["id"].as_str().unwrap().to_string();

    let ping_call = call_agents_api(
        &mut h,
        "call",
        serde_json::json!({
            "workspace": workspace,
            "run_id": run_id,
            "tool": "agents.ping",
            "input_json": "{}",
            "caller_pane": human_pane,
        }),
        None,
    );
    assert_eq!(ping_call["ok"], true, "granted tool must succeed: {ping_call}");

    let admin_call = call_agents_api(
        &mut h,
        "call",
        serde_json::json!({
            "workspace": workspace,
            "run_id": run_id,
            "tool": "agents.admin",
            "input_json": "{}",
            "caller_pane": human_pane,
        }),
        None,
    );
    assert_eq!(admin_call["ok"], false, "ungranted tool must be refused: {admin_call}");
    assert_eq!(admin_call["error_code"], "permission_denied", "{admin_call}");

    // 3. Delegating a sub-head with a wider grant (`agents.admin=allow`) is refused by subset check.
    let wider_delegate = call_agents_api(
        &mut h,
        "delegate",
        serde_json::json!({
            "workspace": workspace,
            "parent_run": run_id,
            "name": "sub-wider",
            "grants": ["agents.admin=allow"],
            "caller_pane": human_pane,
        }),
        None,
    );
    assert_eq!(wider_delegate["ok"], false, "{wider_delegate}");
    assert_eq!(wider_delegate["error_code"], "permission_denied", "{wider_delegate}");
}

#[test]
fn human_pane_has_no_authority_over_other_or_nested_context_secrets_and_grants() {
    use crate::workspace::secrets::{
        self,
        manager::{mutate_secret, secret_inventory, Mutation, SecretTarget},
    };

    let mut h = HostHarness::new();
    let human_pane = h.add_focused_terminal();
    let human_pid = h.terminal_backend(human_pane).child_pid();
    let root_a = crate::platform::path::canonical_or_self(&h.workspace_root());
    let dir_b = tempfile::tempdir().unwrap();
    let root_b = crate::platform::path::canonical_or_self(dir_b.path());
    let nested_b = root_a.join("nested-b");
    std::fs::create_dir_all(&nested_b).unwrap();

    let channel_dir = crate::config::workspace_channel_dir();
    let cfg_a = secrets::init_workspace(&root_a, &channel_dir).unwrap();
    let cfg_b = secrets::init_workspace(&root_b, &channel_dir).unwrap();
    let cfg_nested = secrets::init_workspace(&nested_b, &channel_dir).unwrap();

    let _exp =
        crate::release::set_test_experiments(&[crate::release::ReleaseFeature::FolderSecrets]);
    let mon = monitor();
    let sys_store = secrets::system_store();
    let fld_store = secrets::folder_store();

    sys_store
        .set(
            &secrets::keychain_workspace_name(&cfg_a.id, "SECRET_A"),
            "canary-a-0888",
        )
        .unwrap();
    sys_store
        .set(
            &secrets::keychain_workspace_name(&cfg_b.id, "SECRET_B"),
            "canary-b-0888",
        )
        .unwrap();
    sys_store
        .set(
            &secrets::keychain_workspace_name(&cfg_nested.id, "SECRET_NESTED"),
            "canary-nested-0888",
        )
        .unwrap();

    mutate_secret(
        fld_store,
        &mon,
        &SecretTarget::Folder(root_a.clone()),
        "FOLDER_SECRET_A",
        Mutation::Set("folder-canary-a"),
    )
    .unwrap();
    mutate_secret(
        fld_store,
        &mon,
        &SecretTarget::Folder(root_b.clone()),
        "FOLDER_SECRET_B",
        Mutation::Set("folder-canary-b"),
    )
    .unwrap();
    mutate_secret(
        fld_store,
        &mon,
        &SecretTarget::Folder(nested_b.clone()),
        "FOLDER_SECRET_NESTED",
        Mutation::Set("folder-canary-nested"),
    )
    .unwrap();

    let call_agents_api = |h: &mut HostHarness, op: &str, payload: serde_json::Value| {
        let reply = h
            .workspace_root()
            .join(format!("ctx-scope-{op}-{}.json", uuid::Uuid::new_v4()));
        let req = AppRequest::AgentsApi {
            op: op.to_string(),
            payload,
            response_file: reply.to_string_lossy().to_string(),
        };
        let line = serde_json::to_string(&req).unwrap();
        let mut ack = Vec::new();
        crate::app::handle_socket_line(&line, &h.ipc_tx, Some(&[human_pid]), &mut ack);
        pump_until(h, |_| {
            reply.is_file()
                && std::fs::metadata(&reply)
                    .map(|meta| meta.len() > 0)
                    .unwrap_or(false)
        });
        serde_json::from_str::<serde_json::Value>(&std::fs::read_to_string(&reply).unwrap())
            .unwrap()
    };

    // 1. Human pane in context A cannot create heads or list heads/grants in sibling context B
    //    or nested context `nested_b` inside A's directory.
    for target_ws in [&root_b, &nested_b] {
        let create_resp = call_agents_api(
            &mut h,
            "create_head",
            serde_json::json!({
                "workspace": target_ws,
                "name": "intruder",
                "grants": ["agents.ping=allow"],
                "caller_pane": human_pane,
            }),
        );
        assert_eq!(create_resp["ok"], false, "must refuse target {}: {create_resp}", target_ws.display());
        assert_eq!(create_resp["error_code"], "permission_denied", "{create_resp}");

        let list_heads_resp = call_agents_api(
            &mut h,
            "list_heads",
            serde_json::json!({
                "workspace": target_ws,
                "caller_pane": human_pane,
            }),
        );
        assert_eq!(list_heads_resp["ok"], false, "{list_heads_resp}");
        assert_eq!(list_heads_resp["error_code"], "permission_denied", "{list_heads_resp}");

        let list_grants_resp = call_agents_api(
            &mut h,
            "list_grants",
            serde_json::json!({
                "workspace": target_ws,
                "caller_pane": human_pane,
            }),
        );
        assert_eq!(list_grants_resp["ok"], false, "{list_grants_resp}");
        assert_eq!(list_grants_resp["error_code"], "permission_denied", "{list_grants_resp}");
    }
    assert!(
        mon.list_needs_you().is_empty(),
        "cross-context refusal must not file needs-you: {:?}",
        mon.list_needs_you()
    );

    // 2. Human pane in context A cannot read context B's or nested context B's secrets via PaneSecretGet.
    for (target_ws, secret_name, canary) in [
        (&root_b, "SECRET_B", "canary-b-0888"),
        (&nested_b, "SECRET_NESTED", "canary-nested-0888"),
    ] {
        let reply = std::path::PathBuf::from(crate::rpc::response_file(
            &format!("secret-get-0888-{}", uuid::Uuid::new_v4()),
            "json",
        ));
        let req = AppRequest::PaneSecretGet {
            name: secret_name.to_string(),
            global: false,
            workspace_root: Some(target_ws.clone()),
            peer_ancestry: Vec::new(),
            response_file: reply.to_string_lossy().into_owned(),
        };
        let line = serde_json::to_string(&req).unwrap();
        let mut ack = Vec::new();
        crate::app::handle_socket_line(&line, &h.ipc_tx, Some(&[human_pid]), &mut ack);
        pump_until(&mut h, |_| reply.is_file());
        let body: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&reply).unwrap()).unwrap();
        let _ = std::fs::remove_file(&reply);
        assert_eq!(body["status"], "denied", "must deny cross-context secret_get: {body}");
        assert_eq!(body["error_code"], "permission_denied", "{body}");
        assert!(!body.to_string().contains(canary));
    }

    // 3. CLI context-scope enforcement and inventory scoping from context A:
    let _ctx_guard = crate::cli::workspace::set_test_pane_context_root(&root_a);
    let inv_a = secret_inventory(&root_a, false).unwrap();
    assert!(inv_a.iter().any(|e| e.name == "SECRET_A"));
    assert!(inv_a.iter().any(|e| e.name == "FOLDER_SECRET_A"));
    assert!(
        !inv_a.iter().any(|e| e.name == "SECRET_B"
            || e.name == "SECRET_NESTED"
            || e.name == "FOLDER_SECRET_B"
            || e.name == "FOLDER_SECRET_NESTED"),
        "context A inventory must not include sibling or nested context secrets"
    );
    assert!(
        secret_inventory(&root_b, false).is_err(),
        "listing sibling context B from pane in context A must be refused"
    );
    assert!(
        secret_inventory(&nested_b, false).is_err(),
        "listing nested context B from pane in context A must be refused"
    );
    assert_eq!(
        crate::cli::workspace::folder_secret_read(
            "FOLDER_SECRET_B",
            Some("lead"),
            None,
            Some(&root_b)
        ),
        1
    );
    assert_eq!(
        crate::cli::workspace::folder_secret_read(
            "FOLDER_SECRET_NESTED",
            Some("lead"),
            None,
            Some(&nested_b)
        ),
        1
    );
    assert_eq!(
        crate::cli::workspace::folder_secret_rm("FOLDER_SECRET_NESTED", &nested_b),
        1
    );
}

#[test]
fn agent_spawned_pane_does_not_get_human_authority() {
    use crate::broker::gate::{NeedsYouKind, ResolveSurface};

    let mut h = HostHarness::new();
    let human_pane = h.add_focused_terminal();
    assert!(h.app.is_human_owned_pane(human_pane));

    // 1. Spawn an agent pane (`agent_pane: true`).
    let (_, human_tile) = h.app.find_pane_in_any_window(human_pane).unwrap();
    let agent_pane = h.app.spawn_terminal_pane_at(
        crate::pane_ops::create::SpawnTerminalAtSpec {
            win_idx: 0,
            target_tile: human_tile,
            vertical: true,
            new_pane_first: false,
            keep_focus: true,
            options: crate::pane_ops::create::TerminalSpawnOptions {
                initial_cmd: None,
                close_on_exit: false,
                cwd_override: None,
                agent_pane: true,
                inject_folder_secrets: false,
            },
        },
    );
    assert!(
        !h.app.is_human_owned_pane(agent_pane),
        "agent pane must not be human-owned"
    );

    // 2. Agent pane spawns a plain terminal pane via `pane new` (no `agent_cmd`,
    //    only `from_pane_id: Some(agent_pane)` / peer ancestry). It must ALSO not be human-owned.
    let agent_pid = h.terminal_backend(agent_pane).child_pid();
    let spawn_child_reply = h
        .workspace_root()
        .join(format!("spawn-child-{}.json", uuid::Uuid::new_v4()));
    let child_line = serde_json::json!({
        "type": "spawn_pane",
        "type_id": "terminal",
        "from_pane_id": agent_pane,
        "response_file": spawn_child_reply.to_string_lossy().to_string(),
    })
    .to_string();
    let mut ack = Vec::new();
    crate::app::handle_socket_line(&child_line, &h.ipc_tx, Some(&[agent_pid]), &mut ack);
    pump_until(&mut h, |_| spawn_child_reply.is_file());
    let child_spawn_json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&spawn_child_reply).unwrap()).unwrap();
    let child_shell_pane = child_spawn_json["pane_id"]
        .as_u64()
        .expect("child shell pane_id");
    assert!(
        !h.app.is_human_owned_pane(child_shell_pane),
        "plain shell spawned by an agent pane via `pane new` must never qualify as human-owned"
    );

    // 3. `create_head` from `child_shell_pane` is denied and files a `permission_request` needs-you item.
    let child_pid = h.terminal_backend(child_shell_pane).child_pid();
    let call_create_head = |h: &mut HostHarness, name: &str, grant: &str| -> serde_json::Value {
        let reply = h
            .workspace_root()
            .join(format!("agent-child-head-{}.json", uuid::Uuid::new_v4()));
        let req = AppRequest::AgentsApi {
            op: "create_head".to_string(),
            payload: serde_json::json!({
                "workspace": h.workspace_root(),
                "name": name,
                "grants": [grant],
                "caller_pane": child_shell_pane,
            }),
            response_file: reply.to_string_lossy().to_string(),
        };
        let line = serde_json::to_string(&req).unwrap();
        let mut ack = Vec::new();
        crate::app::handle_socket_line(&line, &h.ipc_tx, Some(&[child_pid]), &mut ack);
        pump_until(h, |_| {
            reply.is_file()
                && std::fs::metadata(&reply)
                    .map(|meta| meta.len() > 0)
                    .unwrap_or(false)
        });
        serde_json::from_str(&std::fs::read_to_string(&reply).unwrap()).unwrap()
    };

    let denied = call_create_head(&mut h, "worker-lead", "agents.ping=allow");
    assert_eq!(denied["ok"], false, "{denied}");
    assert_eq!(denied["error_code"], "permission_denied", "{denied}");
    let needs_you_id = denied["needs_you_id"]
        .as_str()
        .expect("agent-spawned pane must file needs-you")
        .to_string();

    let mon = monitor();
    let open = mon.list_needs_you();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].id, needs_you_id);
    assert_eq!(open[0].kind, NeedsYouKind::PermissionRequest);

    // 4. Desktop approval mints a pane-session grant for `agents.ping` on `child_shell_pane`,
    //    allowing retry of `agents.ping=allow` while still refusing `agents.admin=allow`.
    mon.resolve_needs_you_from(&needs_you_id, true, ResolveSurface::Desktop)
        .unwrap();
    let retry = call_create_head(&mut h, "worker-lead", "agents.ping=allow");
    assert_eq!(retry["ok"], true, "retry after pane-session grant must succeed: {retry}");

    let ungranted = call_create_head(&mut h, "worker-admin", "agents.admin=allow");
    assert_eq!(ungranted["ok"], false, "{ungranted}");
    assert_eq!(ungranted["error_code"], "permission_denied", "{ungranted}");
}
