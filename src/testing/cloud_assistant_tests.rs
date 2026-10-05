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
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::agent::{AgentDefinition, AgentHost};
use crate::broker::{
    ActorScope, ActorType, Decision, GrantDuration, GrantRecord, GrantSource, ResourceScope,
    TargetType,
};
use crate::plexi_ai::broker::{AiBroker, AiBrokerRequest, AiBrokerResponse};
use crate::plexi_ai::turn_loop::TurnDelta;
use crate::protocol::AppRequest;

use super::HostHarness;

// Ids unique to this test: the timeline and tool registry are process-global.
const WHITE: &str = "agent:p1-white";
const BLACK_ID: &str = "p1-black";
const KIBITZER_ID: &str = "p1-kibitzer";

/// One tool call a scripted model made, with what the host answered.
#[derive(Debug, Clone)]
struct Recorded {
    agent: String,
    tool: String,
    input: serde_json::Value,
    output: Option<serde_json::Value>,
    error: Option<String>,
}

/// Scripted model: on each turn, reads the newest committed revision from the
/// delivered event lines and issues a fixed sequence of `chess.play` calls
/// through the broker-filtered dispatcher the real agent loop hands it.
struct ScriptedChessModel {
    log: Arc<Mutex<Vec<Recorded>>>,
}

fn revision_from(messages: &str) -> Option<i64> {
    let at = messages.rfind("\"revision_after\":")?;
    let rest = &messages[at + "\"revision_after\":".len()..];
    let digits: String = rest
        .trim_start()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

impl AiBroker for ScriptedChessModel {
    fn dispatch(
        &self,
        request: AiBrokerRequest,
        _on_delta: &mut dyn FnMut(TurnDelta<'_>),
    ) -> AiBrokerResponse {
        let agent = request.app_id.clone();
        let last = request
            .messages
            .last()
            .map(|m| m.content.clone())
            .unwrap_or_default();
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
            self.log.lock().unwrap().push(Recorded {
                agent: agent.clone(),
                tool: "chess.play".to_string(),
                input,
                output: result
                    .output_json
                    .as_deref()
                    .and_then(|o| serde_json::from_str(o).ok()),
                error: result.error,
            });
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

fn subscription_grant(agent_id: &str) -> GrantRecord {
    GrantRecord {
        actor_type: ActorType::Agent,
        actor_id: agent_id.to_string(),
        actor_scope: ActorScope::User,
        workspace_root: None,
        target_type: TargetType::AppEventStream,
        target_id: "chess::chess.move_committed".to_string(),
        resource_scope: ResourceScope::Workspace,
        resource_id: None,
        decision: Decision::Allow,
        duration: GrantDuration::Always,
        source: GrantSource::User,
        created_at: crate::platform::clock::now_secs() as i64,
        expires_at: None,
    }
}

/// Send `AppRequest::CallAppTool` and pump frames until the worker thread
/// writes the reply file. Returns the parsed reply object.
fn call_tool(
    h: &mut HostHarness,
    caller_pane_id: Option<u64>,
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
        .filter(|r| r.event == "chess.move_committed" && r.caused_by.as_deref() == Some(op))
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
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut agents = AgentHost::new_for_test(
        crate::host::app_timeline::global(),
        Arc::new(ScriptedChessModel {
            log: Arc::clone(&log),
        }),
        h.workspace_root(),
    );
    agents.grant_store.record(subscription_grant(BLACK_ID));
    agents.grant_store.record(subscription_grant(KIBITZER_ID));
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

    let pane = h.launch_repo_app(
        "apps/chess",
        &[
            format!("--white={WHITE}"),
            format!("--black=agent:{BLACK_ID}"),
        ],
    );

    // Discovery through the real tool path: revision 0, seats as launched.
    let state = call_tool(&mut h, None, "chess.state", serde_json::json!({}));
    assert_eq!(state["output"]["revision"], 0, "{state}");
    assert_eq!(
        state["output"]["seats"]["black"],
        format!("agent:{BLACK_ID}")
    );

    // An unseated caller (a pane, identified by the host) is refused, and a
    // forged identity in the input does not help.
    let intruder = h.add_test_pane();
    let mut forged = play(0, "pane-op", "e2e4");
    forged["caller_id"] = serde_json::json!(WHITE);
    let refused = call_tool(&mut h, Some(intruder), "chess.play", forged);
    let err = refused["error"].as_str().unwrap_or_default();
    assert!(
        err.contains("unauthorized"),
        "pane caller must be refused: {refused}"
    );
    let missing = call_tool(
        &mut h,
        Some(9_999_999),
        "chess.play",
        play(0, "ghost", "e2e4"),
    );
    assert!(
        missing["error"]
            .as_str()
            .unwrap_or_default()
            .contains("not found"),
        "an unknown caller pane is refused, not downgraded to user: {missing}"
    );

    // The local user moves White through the same tool; the app commits a
    // receipt and publishes a scoped event.
    let white = call_tool(&mut h, None, "chess.play", play(0, "user-op-1", "e2e4"));
    assert_eq!(white["output"]["revision_after"], 1, "{white}");
    assert_eq!(white["output"]["actor"], "user");
    let event = h
        .wait_for_app_event(
            "chess.move_committed",
            Some("user-op-1"),
            Duration::from_secs(10),
        )
        .expect("move event recorded on the bus");
    assert_eq!(event.app_id, "chess");
    assert_eq!(event.resource_id, "game-1");
    assert_eq!(event.revision_after, "1");
    assert_eq!(
        event.owner_context_id, ctx_id,
        "event is scoped to the app's context"
    );

    // Black's real agent loop sees the event and answers through the tool.
    let started = Instant::now();
    loop {
        h.run_frames(1);
        let done = log
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.agent.ends_with(BLACK_ID))
            .count()
            >= 3
            && log
                .lock()
                .unwrap()
                .iter()
                .any(|r| r.agent.ends_with(KIBITZER_ID));
        if done {
            break;
        }
        assert!(
            started.elapsed() < super::load_aware_timeout(Duration::from_secs(60)),
            "agents did not finish their turns: {:?}",
            log.lock().unwrap()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let calls = log.lock().unwrap().clone();
    let black: Vec<&Recorded> = calls
        .iter()
        .filter(|r| r.agent.ends_with(BLACK_ID))
        .collect();
    let committed = black[0].output.as_ref().expect("black's move committed");
    assert_eq!(committed["revision_after"], 2, "{black:?}");
    assert_eq!(committed["actor"], format!("agent:{BLACK_ID}"));
    let dup = black[1]
        .output
        .as_ref()
        .expect("duplicate returns a receipt");
    assert_eq!(dup["duplicate"], true);
    assert_eq!(dup["revision_after"], 2);
    let again = black[2].error.as_deref().unwrap_or_default();
    assert!(
        again.contains("wrong_side"),
        "a second black move is refused: {again}"
    );
    for kib in calls.iter().filter(|r| r.agent.ends_with(KIBITZER_ID)) {
        let e = kib.error.as_deref().unwrap_or_default();
        assert!(
            e.contains("tool_not_found"),
            "broker withholds chess.play: {e}"
        );
        assert_eq!(kib.tool, "chess.play");
        let _ = &kib.input;
    }

    // Stale and duplicate user calls after the agent's move.
    let stale = call_tool(&mut h, None, "chess.play", play(1, "user-op-2", "g1f3"));
    assert!(
        stale["error"]
            .as_str()
            .unwrap_or_default()
            .contains("stale_revision"),
        "{stale}"
    );
    let dup_user = call_tool(&mut h, None, "chess.play", play(0, "user-op-1", "e2e4"));
    assert_eq!(dup_user["output"]["duplicate"], true, "{dup_user}");

    // Exactly one board mutation and one event per accepted operation.
    let final_state = call_tool(&mut h, None, "chess.state", serde_json::json!({}));
    assert_eq!(final_state["output"]["revision"], 2, "{final_state}");
    let moves = final_state["output"]["moves"].as_array().unwrap();
    let ucis: Vec<&str> = moves.iter().map(|m| m["uci"].as_str().unwrap()).collect();
    assert_eq!(ucis, ["e2e4", "e7e5"]);
    assert_eq!(committed_events("user-op-1"), 1);
    assert_eq!(committed_events(&format!("agent:{BLACK_ID}-r1")), 1);
    assert_eq!(committed_events("pane-op"), 0);
    assert_eq!(committed_events("user-op-2"), 0);
    log::info!("cloud_assistant_p1: pane={pane} final={final_state}");
}

#[test]
fn revision_parser_reads_the_newest_event_line() {
    let text = "[chess] chess.move_committed x payload={\"revision_after\":1}\n\
                [chess] chess.move_committed y payload={\"revision_after\": 12}";
    assert_eq!(revision_from(text), Some(12));
    assert_eq!(revision_from("no revision"), None);
}
