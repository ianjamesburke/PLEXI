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
    pump_until(h, |_| {
        std::fs::metadata(reply).map(|meta| meta.len() > 0).unwrap_or(false)
    });
    serde_json::from_str(&std::fs::read_to_string(reply).unwrap()).unwrap()
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
    // The guest writes the receipt and the move event as separate messages.
    // The receipt can be visible one frame before the timeline records it.
    pump_until(&mut h, |_| commits_for(start, &op) >= 1);
    assert_eq!(commits_for(start, &op), 1, "one commit");
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
    assert_eq!(commits_for(start, op), 1);

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
    assert_eq!(
        ambiguous["error_code"], "ambiguous_instance",
        "bare chess.play must not guess a pane: {ambiguous}"
    );
    assert!(!ambiguous.to_string().contains("tool_not_found"), "{ambiguous}");
    let named = ambiguous["panes"]
        .as_array()
        .expect("ambiguous reply names the panes");
    assert!(
        named.iter().any(|pane| pane.as_u64() == Some(first))
            && named.iter().any(|pane| pane.as_u64() == Some(second)),
        "ambiguity must name both instances: {ambiguous}"
    );
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
    assert_eq!(commits_for(start, "route-e4"), 1);
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
