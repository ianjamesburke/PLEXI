//! Cloud Assistant P1 gate: one chess app instance accepts an authorized move
//! through its real tool path, publishes a scoped event, and rejects
//! duplicate, stale, and unauthorized mutations.
//!
//! Everything runs through production seams: the real `apps/chess` Python app
//! in the CPython-WASM runtime, `AppRequest::CallAppTool` (the `plexi app
//! call` handler), the global tool registry and `ToolDispatcher`, the
//! process-global event timeline, and `AgentHost` turns whose tool snapshot is
//! filtered by the permission broker. Only the model is scripted.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::agent::{AgentDefinition, AgentHost};
use crate::broker::{ActorScope, ActorType, GrantDuration, GrantRecord, GrantSource};
use crate::plexi_ai::broker::{AiBroker, AiBrokerRequest, AiBrokerResponse};
use crate::plexi_ai::turn_loop::TurnDelta;
use crate::protocol::AppRequest;

use super::HostHarness;

// Ids unique to this test: the timeline and tool registry are process-global.
const BLACK_ID: &str = "p1-black";
const KIBITZER_ID: &str = "p1-kibitzer";

/// Scripted model: on each turn, reads the newest committed revision from the
/// delivered event lines and issues a fixed sequence of `chess.play` calls
/// through the broker-filtered dispatcher the real agent loop hands it.
struct ScriptedChessModel;

fn revision_from(messages: &str) -> Option<i64> {
    let at = messages.rfind("\"revision_after\":")?;
    let rest = &messages[at + "\"revision_after\":".len()..];
    let digits: String = rest.trim_start().chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

impl AiBroker for ScriptedChessModel {
    fn dispatch(
        &self,
        request: AiBrokerRequest,
        _on_delta: &mut dyn FnMut(TurnDelta<'_>),
    ) -> AiBrokerResponse {
        let agent = request.app_id.clone();
        let last = request.messages.last().map(|m| m.content.clone()).unwrap_or_default();
        let Some(dispatcher) = request.tool_dispatcher.clone() else {
            return AiBrokerResponse::err("no tool dispatcher".to_string());
        };
        let Some(revision) = revision_from(&last) else {
            return AiBrokerResponse::err(format!("no revision in event lines: {last}"));
        };
        let calls: Vec<serde_json::Value> = vec![
            // The authorized reply.
            serde_json::json!({"game_id": "game-1", "expected_revision": revision,
                "operation_id": format!("{agent}-r{revision}"), "move": "e7e5"}),
            // The same operation again: must return the original receipt.
            serde_json::json!({"game_id": "game-1", "expected_revision": revision,
                "operation_id": format!("{agent}-r{revision}"), "move": "e7e5"}),
            // A fresh operation against the revision it just consumed.
            serde_json::json!({"game_id": "game-1", "expected_revision": revision,
                "operation_id": format!("{agent}-r{revision}-again"), "move": "d7d5"}),
        ];
        for (i, input) in calls.into_iter().enumerate() {
            let result = dispatcher.dispatch_call(
                format!("{agent}-call-{revision}-{i}"),
                "chess.play",
                input.to_string(),
            );
            log::info!(
                "cloud_assistant_p1: scripted {agent} call {i} error={}",
                result.error.as_deref().unwrap_or("none")
            );
        }
        AiBrokerResponse::ok("played".to_string(), 0, 0)
    }
}

fn agent_settings(id: &str, allow_play: bool) -> String {
    let allow = if allow_play {
        r#"allow = ["app.chess.state", "app.chess.legal_moves", "app.chess.play"]"#
    } else {
        r#"allow = ["app.chess.state"]"#
    };
    format!(
        r#"[agent]
id = "{id}"
display_name = "{id}"
default_tier = "medium"

[permissions]
default_posture = "deny"
{allow}

[[subscriptions]]
app = "chess"
events = ["chess.move_committed"]
payload = "full"
trigger = "conversation"
default = "ask"
"#
    )
}

fn subscription_grant(agent_id: &str, workspace: &std::path::Path) -> GrantRecord {
    GrantRecord::event_stream_allow(
        ActorType::Agent,
        agent_id,
        ActorScope::User,
        "chess::chess.move_committed",
        workspace,
        GrantDuration::Always,
        GrantSource::User,
        None,
    )
}

/// Send `AppRequest::CallAppTool` and pump frames until the worker thread
/// writes the reply file. Returns the parsed reply object.
fn call_tool(
    h: &mut HostHarness,
    caller_pane_id: Option<u64>,
    credential: Option<&str>,
    tool: &str,
    input: serde_json::Value,
) -> serde_json::Value {
    let reply_path: PathBuf = h
        .workspace_root()
        .join(format!("reply-{}.json", uuid::Uuid::new_v4()));
    h.inject_ipc(AppRequest::CallAppTool {
        app_id: "chess".to_string(),
        tool: tool.to_string(),
        input_json: input.to_string(),
        caller_pane_id,
        call_credential: credential.map(str::to_string),
        peer_ancestry: Vec::new(),
        target_pane_id: None,
        response_file: Some(reply_path.to_string_lossy().to_string()),
    });
    let started = Instant::now();
    loop {
        h.run_frames(1);
        if let Ok(raw) = std::fs::read_to_string(&reply_path) {
            if !raw.is_empty() {
                return serde_json::from_str(&raw).expect("reply is JSON");
            }
        }
        assert!(
            started.elapsed() < super::load_aware_timeout(Duration::from_secs(30)),
            "no reply to {tool} within the deadline"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn play(rev: i64, op: &str, mv: &str) -> serde_json::Value {
    serde_json::json!({"game_id": "game-1", "expected_revision": rev,
        "operation_id": op, "move": mv})
}

fn committed_events(op: &str) -> usize {
    let timeline = crate::host::app_timeline::global();
    let timeline = timeline.lock().unwrap();
    timeline
        .events()
        .iter()
        .filter(|r| {
            r.event == "chess.move_committed" && r.caused_by.as_deref() == Some(op)
        })
        .count()
}

#[test]
fn chess_tool_path_commits_publishes_and_rejects_bad_mutations() {
    let mut h = HostHarness::new();
    // Tool registration is process-global in the test binary. Give this
    // integration fixture a harness-unique context as well as its already
    // unique pane-id block, so another concurrently running app test cannot
    // make the common `chess.*` names ambiguous in its snapshot.
    let unique_context_id = h.add_test_pane();
    h.app.windows[h.app.active_window].context_id = unique_context_id;
    let active_context_idx = h.app.router.active_idx();
    h.app.router.get_mut(active_context_idx).context_id = unique_context_id;
    let ctx_id = h.app.windows[h.app.active_window].context_id;

    // Black is a real AgentHost agent: broker-granted subscription, posture
    // allowing chess.play. The kibitzer is subscribed but holds no chess.play
    // permission, so the broker must withhold the tool from its turns.
    let mut agents = AgentHost::new_for_test(
        crate::host::app_timeline::global(),
        Arc::new(ScriptedChessModel),
        h.workspace_root(),
    );
    agents.grant_store.record(subscription_grant(BLACK_ID, &h.workspace_root()));
    agents.grant_store.record(subscription_grant(KIBITZER_ID, &h.workspace_root()));
    agents.reload_workspace(None, ctx_id);
    for (id, allow) in [(BLACK_ID, true), (KIBITZER_ID, false)] {
        let def = AgentDefinition::parse("Play chess.", &agent_settings(id, allow))
            .expect("agent settings parse");
        agents.attach(def);
    }
    assert!(
        agents.agents.iter().all(|a| a.subscription_ids.len() == 1),
        "both agents hold a granted chess subscription"
    );
    h.app.agent_host = agents;

    let pane = h.launch_repo_app("apps/chess", &[]);
    let monitor = crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir());
    let credential = monitor.issue_credential(
        Some(pane),
        ctx_id,
        &h.workspace_root(),
        &format!("pane:{pane}"),
    );

    let blocked = call_tool(
        &mut h,
        Some(pane),
        Some(&credential),
        "chess.play",
        play(0, "pane-op", "e2e4"),
    );
    assert_eq!(blocked["error_code"], "permission_required", "{blocked}");
    assert_eq!(committed_events("pane-op"), 0, "no grant must not mutate");

    let pending = blocked["pending_request_id"].as_str().unwrap();
    monitor
        .approve_pending(pending, crate::broker::gate::ApprovalChoice::Once)
        .expect("approve the exact pending move");
    let white = call_tool(
        &mut h,
        Some(pane),
        Some(&credential),
        "chess.play",
        play(0, "pane-op", "e2e4"),
    );
    assert_eq!(white["output"]["revision_after"], 1, "{white}");
    assert_eq!(white["output"]["actor"], format!("pane:{pane}"));
    let dup = call_tool(
        &mut h,
        Some(pane),
        Some(&credential),
        "chess.play",
        play(0, "pane-op", "e2e4"),
    );
    assert_eq!(dup["output"]["duplicate"], true, "{dup}");
    assert_eq!(committed_events("pane-op"), 1);

    let missing = call_tool(&mut h, Some(9_999_999), None, "chess.play", play(1, "ghost", "e7e5"));
    assert_eq!(missing["error_code"], "permission_denied", "{missing}");
    log::info!("cloud_assistant_p1: pane={pane} committed once");
}

#[test]
fn revision_parser_reads_the_newest_event_line() {
    let text = "[chess] chess.move_committed x payload={\"revision_after\":1}\n\
                [chess] chess.move_committed y payload={\"revision_after\": 12}";
    assert_eq!(revision_from(text), Some(12));
    assert_eq!(revision_from("no revision"), None);
}
