//! Stint 1285: one typed caller at the socket boundary. Each test drives a
//! real socket line through the admission entry with a kernel-shaped
//! witness, and pins one former "is this the human" decision.

use crate::broker::caller::{AncestryEnd, PeerHop, PeerWitness, SocketCaller};
use crate::testing::HostHarness;

/// A witness whose nearest process is `pid` (a live process), ending how
/// `end` says.
fn witness(pid: u32, end: AncestryEnd) -> PeerWitness {
    let mut peer = PeerWitness::for_live_pids(&[pid]);
    peer.end = end;
    peer
}

/// A witness for a process outside every pane: this test process, whose
/// lineage the kernel walks to the root.
fn outside() -> PeerWitness {
    let peer = PeerWitness::capture(std::process::id());
    assert_eq!(peer.end, AncestryEnd::ReachedRoot, "{peer:?}");
    peer
}

/// Read the client half of a reply connection to its end, as the CLI does:
/// the host's ack byte is dropped and the JSON body returned. There is no
/// file fallback on this path, so a reply that took one is never seen.
fn read_client(mut client: std::os::unix::net::UnixStream, line: &str) -> serde_json::Value {
    use std::io::Read as _;
    // macOS refuses a timeout (EINVAL) once the host half is closed; the
    // read then ends at once anyway.
    let _ = client.set_read_timeout(Some(std::time::Duration::from_secs(3)));
    let mut bytes = Vec::new();
    client
        .read_to_end(&mut bytes)
        .unwrap_or_else(|error| panic!("no reply on the connection to {line}: {error}"));
    let body = String::from_utf8_lossy(&bytes);
    let body = body.trim_start_matches('\u{6}').trim();
    assert!(!body.is_empty(), "no reply on the connection to {line}");
    serde_json::from_str(body).unwrap_or_else(|_| serde_json::json!({ "raw": body }))
}

/// Send one request line as the socket listener would, from `peer`, and
/// return the reply read from the client's end of that connection.
fn send(h: &mut HostHarness, mut request: serde_json::Value, peer: Option<PeerWitness>) -> serde_json::Value {
    let token = crate::rpc::response_file("socket-policy-test", "json");
    request["response_file"] = serde_json::json!(token);
    let line = request.to_string();
    let (mut server, client) = std::os::unix::net::UnixStream::pair().unwrap();
    crate::app::handle_socket_line_in(
        &crate::config::config_dir(),
        &line,
        &h.ipc_tx,
        peer.as_ref(),
        &mut server,
    );
    drop(server);
    h.run_frames(2);
    let reply = read_client(client, &line);
    assert!(
        !std::path::Path::new(&token).exists(),
        "the reply took the file fallback: {line}"
    );
    reply
}

fn pane_witness(h: &HostHarness, pane: u64) -> PeerWitness {
    PeerWitness::for_live_pids(&[h.shell_pid(pane)])
}

#[test]
fn a_terminal_shell_is_a_pane_caller_only_at_its_own_incarnation() {
    let mut h = HostHarness::new();
    let pane = h.add_focused_terminal();
    let pid = h.shell_pid(pane);
    let caller = h.app.classify_socket_peer(Some(&pane_witness(&h, pane)));
    let SocketCaller::Pane(found) = caller else {
        panic!("expected a pane caller, got {caller:?}");
    };
    assert_eq!(found.terminal_pane, pane);
    // The same pid at another start time is a reused pid, not this pane.
    let mut reused = pane_witness(&h, pane);
    reused.hops[0] = PeerHop {
        pid,
        start: crate::platform::process::process_info(std::process::id())
            .unwrap()
            .start,
    };
    reused.end = AncestryEnd::LookupFailed;
    assert!(matches!(
        h.app.classify_socket_peer(Some(&reused)),
        SocketCaller::Unidentified(_)
    ));
}

#[test]
fn an_outside_process_is_external_and_an_incomplete_walk_is_unidentified() {
    let h = HostHarness::new();
    assert!(matches!(
        h.app.classify_socket_peer(Some(&outside())),
        SocketCaller::External(_)
    ));
    assert!(matches!(
        h.app
            .classify_socket_peer(Some(&witness(std::process::id(), AncestryEnd::LookupFailed))),
        SocketCaller::Unidentified(_)
    ));
    assert!(matches!(
        h.app.classify_socket_peer(None),
        SocketCaller::Unidentified(_)
    ));
}

/// Inventory 15: an own-pane report from another pane, from outside, or
/// from nobody is refused (round3 6b).
#[test]
fn agent_reports_only_for_the_callers_own_pane() {
    let mut h = HostHarness::new();
    let caller = h.add_focused_terminal();
    let target = h.add_focused_terminal();
    let done = |pane: u64| {
        serde_json::json!({"type": "agent_done", "pane_id": pane, "result": "{}", "summary": "s"})
    };
    let peer = pane_witness(&h, caller);
    let from_other = send(&mut h, done(target), Some(peer));
    assert_eq!(from_other["error_code"], "permission_denied", "{from_other}");
    let from_outside = send(&mut h, done(target), Some(outside()));
    assert_eq!(from_outside["error_code"], "permission_denied", "{from_outside}");
    let from_nobody = send(&mut h, done(target), None);
    assert_eq!(from_nobody["error_code"], "permission_denied", "{from_nobody}");
}

/// Inventory 12: a socket answer never answers; the question stays pending
/// and the asker never holds a token that would.
#[test]
fn a_socket_caller_cannot_answer_a_question() {
    let mut h = HostHarness::new();
    let pane = h.add_focused_terminal();
    let peer = pane_witness(&h, pane);
    let asked = send(
        &mut h,
        serde_json::json!({"type": "needs_you_api", "op": "ask", "payload": {"question": "ship?"}}),
        Some(peer),
    );
    assert_eq!(asked["ok"], true, "{asked}");
    assert!(asked["ui_token"].is_null(), "a socket asker got a token: {asked}");
    let id = asked["id"].as_str().unwrap().to_string();
    let own = pane_witness(&h, pane);
    for peer in [Some(own), Some(outside()), None] {
        let answered = send(
            &mut h,
            serde_json::json!({"type": "needs_you_api", "op": "answer",
                "payload": {"id": id, "answer": "yes"}}),
            peer,
        );
        assert_eq!(answered["error_code"], "approval_required", "{answered}");
    }
    let monitor =
        crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir()).unwrap();
    assert!(monitor.get_needs_you(&id).is_some_and(|item| item.answer.is_none()));
}

/// Inventory 23: the asker is the classified caller, never a payload actor.
#[test]
fn a_question_is_asked_as_the_caller_not_the_payload_actor() {
    let mut h = HostHarness::new();
    let pane = h.add_focused_terminal();
    let peer = pane_witness(&h, pane);
    let asked = send(
        &mut h,
        serde_json::json!({"type": "needs_you_api", "op": "ask",
            "payload": {"question": "q", "actor": "user", "pane_id": 999, "context_id": 999}}),
        Some(peer),
    );
    assert_eq!(asked["item"]["actor"], format!("pane:{pane}"), "{asked}");
}

/// Inventory 6 and 19: no caller gets "operator" or "*" authority; widening
/// beyond what the caller holds files a request and writes no grant.
#[test]
fn create_agent_beyond_the_callers_grants_files_a_request() {
    let _mesh = crate::release::set_test_experiments(&[crate::release::ReleaseFeature::AgentMesh]);
    let mut h = HostHarness::new();
    let pane = h.add_focused_terminal();
    let workspace = h.workspace_root();
    let create = |name: &str| {
        serde_json::json!({"type": "agents_api", "op": "create_agent", "payload": {
            "workspace": workspace, "name": name, "grants": ["agents.ping=allow"]}})
    };
    let own = pane_witness(&h, pane);
    for (name, peer) in [
        ("from-desktop-pane", Some(own)),
        ("from-outside", Some(outside())),
    ] {
        let reply = send(&mut h, create(name), peer);
        assert_eq!(reply["ok"], false, "{name}: {reply}");
        assert!(reply["needs_you_id"].is_string(), "{name}: {reply}");
    }
    let unidentified = send(&mut h, create("from-nobody"), None);
    assert_eq!(unidentified["error_code"], "permission_denied", "{unidentified}");
    let monitor =
        crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir()).unwrap();
    assert!(
        !monitor
            .store()
            .records()
            .iter()
            .any(|row| row.actor_id.starts_with("agent:from-")),
        "a refused create wrote a grant"
    );
}

/// P1-7: a team run names no caller in its payload; a claim is refused by name.
#[test]
fn a_team_run_claiming_a_caller_pane_is_refused_by_name() {
    let mut h = HostHarness::new();
    let pane = h.add_focused_terminal();
    let workspace = h.workspace_root();
    let reply = send(
        &mut h,
        serde_json::json!({"type": "agents_api", "op": "team_run", "payload": {
            "workspace": workspace, "name": "t", "question": "q",
            "caller_pane": pane}}),
        Some(outside()),
    );
    assert_eq!(reply["error_code"], "removed_field", "{reply}");
    assert_eq!(reply["field"], "caller_pane", "{reply}");
    assert!(h.app.team_runs.is_empty(), "nothing started");
}

/// Inventory 8: an outside process types into a pane as itself, like
/// `pane new` (ruling 192); a pane the person opened is brokered like any
/// pane; a caller with no identity sends nothing.
#[test]
fn pane_send_from_outside_reaches_the_pane_and_from_a_pane_needs_the_broker() {
    let mut h = HostHarness::new();
    let caller = h.add_focused_terminal();
    let target = h.add_focused_terminal();
    h.terminal_backend(target).enable_input_tap();
    let send_to = serde_json::json!({"type": "send_to_pane", "pane_id": target, "text": "echo hi",
        "submit": false, "paste": false, "operation_id": "pane-send-outside",
        "generated_operation_id": true});
    let nobody_reply = send(&mut h, send_to.clone(), None);
    assert_eq!(nobody_reply["error_code"], "permission_denied", "{nobody_reply}");
    assert!(
        !h.terminal_backend(target).take_input_tap().ends_with(b"echo hi"),
        "an unidentified caller typed into the target"
    );
    let outside_reply = send(&mut h, send_to.clone(), Some(outside()));
    assert_eq!(outside_reply["ok"], true, "{outside_reply}");
    assert!(
        h.terminal_backend(target).take_input_tap().ends_with(b"echo hi"),
        "an outside pane send must reach the target"
    );
    assert!(
        h.app.pending_send_approvals.is_empty(),
        "an outside pane send is attributed, not brokered"
    );
    // From the person's own pane: the broker asks; nothing is typed until a
    // desktop click approves.
    let mut request: crate::protocol::HostRequest = serde_json::from_value(send_to).unwrap();
    if let crate::protocol::HostRequest::SendToPane { response_file, .. } = &mut request {
        *response_file = Some(crate::rpc::response_file("socket-policy-send", "json"));
    }
    h.inject_from_pane(caller, request);
    h.run_frames(2);
    assert!(
        h.app.pending_send_approvals.iter().any(|item| item.pane_id == target),
        "a pane send from a desktop pane must wait for approval"
    );
    assert!(
        !h.terminal_backend(target).take_input_tap().ends_with(b"echo hi"),
        "the text reached the target without admission"
    );
}

/// Inventory 22: the event bus ignores a claimed pane.
#[test]
fn event_bus_caller_is_never_the_claimed_pane() {
    let mut h = HostHarness::new();
    let pane = h.add_focused_terminal();
    assert_eq!(h.app.resolve_event_bus_caller(None), (None, None, None));
    let caller = h.app.classify_socket_peer(Some(&pane_witness(&h, pane)));
    assert_eq!(h.app.resolve_event_bus_caller(Some(&caller)).0, Some(pane));
    let _ = &mut h;
}

/// Inventory 9 and 10: the person's own pane gets no secret inventory
/// shortcut.
#[test]
fn secret_list_admits_every_pane_as_itself() {
    let mut h = HostHarness::new();
    let pane = h.add_focused_terminal();
    let context = h.app.windows[h.app.active_window].context_id;
    let peer = pane_witness(&h, pane);
    let reply = send(
        &mut h,
        serde_json::json!({"type": "pane_secret_list", "context": context.to_string()}),
        Some(peer),
    );
    assert_eq!(reply["error_code"], "permission_denied", "{reply}");
}

/// Stream frames outside `HostRequest` pass the same entry.
#[test]
fn stream_frames_are_admitted_on_the_host_thread() {
    let mut h = HostHarness::new();
    let (reply, admitted) = std::sync::mpsc::sync_channel(1);
    h.ipc_tx
        .send(crate::app::socket_caller::PaneIpcMessage::Admit {
            peer: None,
            frame: crate::app::socket_policy::StreamFrame::EventsSubscribe,
            scope: None,
            reply,
        })
        .unwrap();
    h.run_frames(1);
    let refusal = admitted.recv().unwrap().unwrap_err();
    assert_eq!(refusal.code, "permission_denied");
}

/// A delegated row is never the person's, and an outside-process row lasts
/// at most the session, on write and on lookup.
#[test]
fn outside_process_grants_are_once_or_session_only() {
    use crate::broker::GrantDuration;
    for duration in [
        GrantDuration::Once,
        GrantDuration::Session,
        GrantDuration::Workspace,
        GrantDuration::Document,
        GrantDuration::Game,
        GrantDuration::Path,
        GrantDuration::PackageIdentity,
        GrantDuration::Always,
    ] {
        assert_eq!(
            duration.allowed_for_external(),
            matches!(duration, GrantDuration::Once | GrantDuration::Session),
            "{duration:?}"
        );
    }
    let principal = crate::broker::principal::Principal::External;
    assert_eq!(principal.actor_type(), crate::broker::ActorType::External);
    assert_ne!(
        (principal.actor_type(), principal.actor_id()),
        (
            crate::broker::ActorType::Agent,
            crate::broker::principal::Principal::agent("agent:external").actor_id()
        ),
        "an agent named external shares the outside-process key"
    );
}

/// The resource lease holder must be in the caller's witnessed lineage.
#[test]
fn a_lease_holder_outside_the_callers_lineage_is_refused() {
    let mut h = HostHarness::new();
    let reply = send(
        &mut h,
        serde_json::json!({"type": "resource_lease", "op": "acquire", "name": "gpu",
            "max": 1, "holder_pid": 2}),
        Some(outside()),
    );
    assert_eq!(reply["error_code"], "permission_denied", "{reply}");
}


/// R7: `agent spawn --from` is placement. A pane places beside a pane of its
/// own context and stays the parent; another context's pane is refused.
#[test]
fn agent_spawn_anchor_is_placement_and_never_identity() {
    let _mesh = crate::release::set_test_experiments(&[crate::release::ReleaseFeature::AgentMesh]);
    let mut h = HostHarness::new();
    let caller = h.add_focused_terminal();
    let anchor = h.add_focused_terminal();
    let workspace = h.workspace_root();
    let spawn = |anchor_pane: u64, name: &str| {
        serde_json::json!({"type": "agent_spawn", "role": name, "harness": "script",
            "command": "sleep 30", "cwd": workspace, "anchor_pane": anchor_pane})
    };
    let witness = pane_witness(&h, caller);
    let placed = send(&mut h, spawn(anchor, "placed"), Some(witness.clone()));
    assert_eq!(placed["ok"], true, "{placed}");
    let worker = placed["pane_id"].as_u64().unwrap();
    let grant = h.app.pane_authority(worker).unwrap().grant().clone();
    assert_eq!(grant.parent, Some(caller), "the anchor is not the parent");
    assert_ne!(grant.parent, Some(anchor));

    // A worker in a sub-context is a pane of another context.
    let (foreign, foreign_context) = h
        .app
        .spawn_agent_worker_pane(
            "elsewhere", "script", "sleep 30", "", Some("other-context"), false,
            Some(caller), None, None, None, false, None,
        )
        .unwrap();
    assert_ne!(foreign_context, h.app.windows[0].context_id);
    let refused = send(&mut h, spawn(foreign, "refused"), Some(witness));
    assert_eq!(refused["error_code"], "permission_denied", "{refused}");
}

/// An outside process goes through the broker: its message files a request,
/// an always answer is refused, and one approval delivers that exact message.
#[test]
fn an_outside_agent_message_is_brokered_once() {
    let _mesh = crate::release::set_test_experiments(&[crate::release::ReleaseFeature::AgentMesh]);
    let mut h = HostHarness::new();
    let parent = h.add_focused_terminal();
    let (child, _) = h
        .app
        .spawn_agent_worker_pane(
            "tester", "pi", "sleep 30", "Verify", None, false, Some(parent), None, None, None,
            false, None,
        )
        .unwrap();
    let message = serde_json::json!({"type": "agent_message", "pane_id": child, "text": "hello"});
    let first = send(&mut h, message.clone(), Some(outside()));
    assert!(first["error"].as_str().unwrap_or("").contains("Needs you"), "{first}");
    assert!(h.app.pane_mailboxes.get(&child).is_none_or(|queue| queue.is_empty()));
    let monitor =
        crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir()).unwrap();
    let pending = monitor
        .list_pending()
        .into_iter()
        .find(|row| row.tool == "agent.message")
        .expect("a pending request")
        .pending_request_id;
    assert!(monitor
        .approve_pending(&pending, crate::broker::gate::ApprovalChoice::Always)
        .is_err());
    monitor
        .approve_pending(&pending, crate::broker::gate::ApprovalChoice::Once)
        .unwrap();
    let second = send(&mut h, message.clone(), Some(outside()));
    assert!(second.get("error").is_none(), "{second}");
    assert_eq!(h.app.pane_mailboxes[&child].len(), 1);
    let third = send(&mut h, message, Some(outside()));
    assert!(third["error"].as_str().unwrap_or("").contains("Needs you"), "one use: {third}");
    assert_eq!(h.app.pane_mailboxes[&child].len(), 1);
}

/// The event bus names an outside process with the external identity, never
/// an agent id: an agent named `external` holds a disjoint row.
#[test]
fn an_outside_event_bus_caller_has_the_external_identity() {
    use crate::broker::ActorType;
    use crate::host::event_subscriptions::HostSubscriptionService as Service;
    let (actor_type, actor_id) = Service::resolve_cli_subscriber(None);
    assert_eq!(actor_type, ActorType::External);
    assert_eq!(actor_id, crate::broker::principal::EXTERNAL_ID);
    assert_ne!(
        crate::broker::principal::Principal::External.actor_type(),
        crate::broker::principal::Principal::agent("agent:external").actor_type()
    );
}

/// A socket caller never borrows the Assistant's grants: an unlisted tool is
/// refused, a tool runs as the caller, and permission rows stay its own.
#[test]
fn an_assistant_tool_over_the_socket_runs_as_the_caller() {
    let mut h = HostHarness::new();
    let pane = h.add_focused_terminal();
    let peer = pane_witness(&h, pane);
    let tool = |name: &str, input: &str| {
        serde_json::json!({"type": "assistant_host_tool", "name": name, "input_json": input})
    };
    let unknown = send(&mut h, tool("host.not.a.tool", "{}"), Some(peer.clone()));
    assert_eq!(unknown["error_code"], "invalid_argument", "{unknown}");

    let monitor =
        crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir()).unwrap();
    let path = h.workspace_root().join("note.txt");
    let input = serde_json::json!({"path": path, "content": "x"}).to_string();
    let write = send(&mut h, tool("host.files.write", &input), Some(peer.clone()));
    assert_eq!(write["ok"], false, "{write}");
    assert!(!path.exists(), "nothing written before a decision");
    assert!(
        monitor
            .list_pending()
            .iter()
            .any(|row| row.tool == "host.files.write" && row.actor_id == format!("pane:{pane}")),
        "the request is the pane's, not the Assistant's"
    );
    let listed = send(&mut h, tool("host.permissions.list", "{}"), Some(peer));
    assert_eq!(listed["ok"], true, "{listed}");
    assert!(
        listed["output"]["entries"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["actor_id"] == format!("pane:{pane}")),
        "{listed}"
    );
}

/// Inventory: a socket caller gets no plugin rows and only the items it filed.
#[test]
fn permission_inventory_and_needs_you_lists_are_scoped_to_the_caller() {
    use crate::broker::gate::{NeedsYouFile, NeedsYouKind};
    let mut h = HostHarness::new();
    let pane = h.add_focused_terminal();
    let peer = pane_witness(&h, pane);
    let monitor =
        crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir()).unwrap();
    let file = |actor: &str, tag: &str| {
        monitor
            .file_needs_you(NeedsYouFile {
                kind: NeedsYouKind::Question,
                actor: actor.to_string(),
                resource: "r".into(),
                summary: "s".into(),
                expires_at: None,
                run_tag: Some(tag.to_string()),
            })
            .unwrap()
    };
    let own = file(&format!("pane:{pane}"), "own");
    let other = file("pane:999999", "other");
    let listed = send(&mut h, serde_json::json!({"type": "list_needs_you"}), Some(peer.clone()));
    let ids: Vec<_> = listed["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|row| row["id"].as_str())
        .collect();
    assert!(ids.contains(&own.as_str()), "{listed}");
    assert!(!ids.contains(&other.as_str()), "{listed}");
    let inventory = send(&mut h, serde_json::json!({"type": "list_permissions"}), Some(peer));
    assert_eq!(inventory["permissions"], serde_json::json!([]), "{inventory}");
}

// ── SOCKET_OPERATIONS: one row per socket operation ──────────────────────

use crate::app::socket_policy::{
    AgentsOp, DashboardOp, NeedsYouOp, PermissionsOp, StreamFrame, VerbPolicy, ASSISTANT_TOOLS,
};

/// Where a row's operation lives on the request: a whole `HostRequest`
/// variant, one suboperation of a multiplexed one, or a stream frame.
#[derive(Debug, Clone, Copy)]
enum Op {
    Request(&'static str),
    Sub(&'static str, &'static str, &'static str),
    Frame(&'static str),
}

/// The expected policy of every socket operation. `every_socket_operation_is_classified`
/// fails when a `HostRequest` variant, suboperation or stream frame has no row.
const SOCKET_OPERATIONS: &[(Op, VerbPolicy)] = {
    use Op::{Frame, Request, Sub};
    use VerbPolicy::*;
    &[
        (Request("get_build_info"), Open),
        (Request("wake"), Open),
        (Request("file_open"), Open),
        (Request("shutdown"), Open),
        (Request("show_host"), Operator),
        (Request("spawn_receipt"), Operator),
        (Request("context_rename"), Operator),
        (Request("context_close"), Operator),
        (Request("context_park"), Operator),
        (Request("pane_zoom"), Operator),
        (Request("pane_swap"), Operator),
        (Request("workspace_snapshot"), Operator),
        (Request("reload_config"), Operator),
        (Request("focus_history"), Operator),
        (Request("resource_lease"), Operator),
        (Request("notify"), Operator),
        (Request("dismiss_notification"), Operator),
        (Request("set_pip_status"), Operator),
        (Request("get_agent_states"), Operator),
        (Request("log_marker"), Operator),
        (Request("spawn_pane"), Operator),
        (Request("set_pane_title"), Operator),
        (Request("list_panes"), Operator),
        (Request("list_contexts"), Operator),
        (Request("get_pane_info"), Operator),
        (Request("get_previous_pane_info"), Operator),
        (Request("slot_write"), Operator),
        (Request("slot_read"), Operator),
        (Request("slot_wait"), Operator),
        (Request("slot_list"), Operator),
        (Request("slot_delete"), Operator),
        (Request("workspace_clean_slots"), Operator),
        (Request("pane_message_send"), Operator),
        (Request("pane_message_recv"), Operator),
        (Request("pane_result_schema_set"), Operator),
        (Request("pane_result_write"), Operator),
        (Request("pane_result_read"), Operator),
        (Request("agent_spawn"), Operator),
        (Request("agent_wait"), Operator),
        (Request("agent_next"), Operator),
        (Request("agent_fleet"), Operator),
        (Request("agent_fleet_dismiss"), Operator),
        (Request("agent_open"), Operator),
        (Request("agent_new_chat"), Operator),
        (Request("agent_reopen"), Operator),
        (Request("focus_pane"), Operator),
        (Request("close_pane"), Operator),
        (Request("pane_heartbeat"), Operator),
        (Request("key_pane"), Operator),
        (Request("drop_file"), Operator),
        (Request("click_pane"), Operator),
        (Request("click_pane_node"), Operator),
        (Request("drag_pane"), Operator),
        (Request("capture_pane"), Operator),
        (Request("pane_status"), Operator),
        (Request("get_pane_state"), Operator),
        (Request("screenshot"), Operator),
        (Request("host_key"), Operator),
        (Request("send_pane_action"), Operator),
        (Request("list_needs_you"), Operator),
        (Request("caller_context"), Operator),
        (Request("create_context"), Operator),
        (Request("create_sub_context"), Operator),
        (Request("focus_context"), Operator),
        (Request("set_context_description"), Operator),
        (Request("zoom_into_context"), Operator),
        (Request("zoom_out_of_context"), Operator),
        (Request("push_pane_to_subcontext"), Operator),
        (Request("pane_secret_list"), Protected),
        (Request("secret_get"), Protected),
        (Request("secret_set"), Protected),
        (Request("secret_delete"), Protected),
        (Request("call_plugin_tool"), Protected),
        (Request("list_plugin_tools"), Protected),
        (Request("set_context_root"), Protected),
        (Request("agent_call"), Protected),
        (Request("agent_message"), Protected),
        (Request("submit_assistant_turn"), Protected),
        (Request("list_permissions"), Protected),
        (Request("list_permission_requests"), Protected),
        (Request("show_permission_request"), Protected),
        (Sub("permissions", "op", "list"), Protected),
        (Sub("permissions", "op", "revoke"), Protected),
        (Sub("permissions", "op", "allow"), Widening),
        (Sub("permissions", "op", "reset"), Widening),
        (Sub("needs_you_api", "op", "ask"), Protected),
        (Sub("needs_you_api", "op", "show"), Protected),
        (Sub("needs_you_api", "op", "reconcile"), Protected),
        (Sub("needs_you_api", "op", "complete"), Protected),
        (Sub("needs_you_api", "op", "answer"), Decision),
        (Sub("agents_api", "op", "list_agents"), Protected),
        (Sub("agents_api", "op", "list_grants"), Protected),
        (Sub("agents_api", "op", "list_runs"), Protected),
        (Sub("agents_api", "op", "show_run"), Protected),
        (Sub("agents_api", "op", "finish_run"), Protected),
        (Sub("agents_api", "op", "spawn_run"), Protected),
        (Sub("agents_api", "op", "call"), Protected),
        (Sub("agents_api", "op", "create_agent"), Widening),
        (Sub("agents_api", "op", "delegate"), Widening),
        (Sub("agents_api", "op", "team_run"), Widening),
        (Sub("agents_dashboard", "op", "open"), Operator),
        (Sub("agents_dashboard", "op", "list"), Operator),
        (Sub("agents_dashboard", "op", "cancel"), Protected),
        (Sub("assistant_host_tool", "name", "host.editors.list"), Protected),
        (Sub("assistant_host_tool", "name", "host.introspect"), Protected),
        (Sub("assistant_host_tool", "name", "host.permissions.list"), Protected),
        (Sub("assistant_host_tool", "name", "host.permissions.revoke"), Protected),
        (Sub("assistant_host_tool", "name", "host.permissions.reset"), Widening),
        (Sub("assistant_host_tool", "name", "host.permissions.allow"), Widening),
        (Sub("assistant_host_tool", "name", "host.panes.list"), Protected),
        (Sub("assistant_host_tool", "name", "host.panes.state"), Protected),
        (Sub("assistant_host_tool", "name", "host.panes.open"), Protected),
        (Sub("assistant_host_tool", "name", "host.panes.focus"), Protected),
        (Sub("assistant_host_tool", "name", "host.panes.close"), Protected),
        (Sub("assistant_host_tool", "name", "host.views.open"), Protected),
        (Sub("assistant_host_tool", "name", "host.terminals.open"), Protected),
        (Sub("assistant_host_tool", "name", "host.terminals.run"), Protected),
        (Sub("assistant_host_tool", "name", "host.terminals.read"), Protected),
        (Sub("assistant_host_tool", "name", "host.files.read"), Protected),
        (Sub("assistant_host_tool", "name", "host.files.grep"), Protected),
        (Sub("assistant_host_tool", "name", "host.files.list"), Protected),
        (Sub("assistant_host_tool", "name", "host.files.write"), Protected),
        (Sub("assistant_host_tool", "name", "host.files.edit"), Protected),
        (Request("resolve_permission_request"), Decision),
        (Request("resolve_needs_you"), Decision),
        (Request("set_agent_state"), OwnPane),
        (Request("agent_done"), OwnPane),
        (Request("agent_ask"), OwnPane),
        (Request("send_to_pane"), Protected),
        (Request("capability_request"), NotOnSocket),
        (Request("plugin_secret_get"), NotOnSocket),
        (Request("file_read"), NotOnSocket),
        (Request("file_list"), NotOnSocket),
        (Request("save_plugin_state"), NotOnSocket),
        (Request("run_get"), NotOnSocket),
        (Request("run_complete"), NotOnSocket),
        (Request("pipe_open"), NotOnSocket),
        (Request("pipe_open_directed"), NotOnSocket),
        (Request("pipe_send"), NotOnSocket),
        (Request("status_summary"), NotOnSocket),
        (Request("spawn_view"), NotOnSocket),
        (Request("query_context_state"), NotOnSocket),
        (Request("http_request"), NotOnSocket),
        (Request("open_url"), NotOnSocket),
        (Request("ai_query"), NotOnSocket),
        (Request("expose_tools"), NotOnSocket),
        (Request("tool_result"), NotOnSocket),
        (Request("audio_play"), NotOnSocket),
        (Request("audio_capture"), NotOnSocket),
        (Request("list_audio_devices"), NotOnSocket),
        (Request("list_midi_devices"), NotOnSocket),
        (Request("open_midi_input"), NotOnSocket),
        (Request("close_midi_input"), NotOnSocket),
        (Request("send_midi"), NotOnSocket),
        (Request("open_video"), NotOnSocket),
        (Request("set_video_state"), NotOnSocket),
        (Request("close_video"), NotOnSocket),
        (Request("cd_request"), NotOnSocket),
        (Request("set_timer"), NotOnSocket),
        (Request("cancel_timer"), NotOnSocket),
        (Request("load_image"), NotOnSocket),
        (Request("request_linked_terminal"), NotOnSocket),
        (Request("run_in_linked_terminal"), NotOnSocket),
        (Request("insert_path_token"), NotOnSocket),
        (Request("request_command_preview"), NotOnSocket),
        (Request("open_artifact"), NotOnSocket),
        (Request("push_nav"), NotOnSocket),
        (Request("pop_nav"), NotOnSocket),
        (Request("set_mouse_tracking"), NotOnSocket),
        (Request("stream_process"), NotOnSocket),
        (Request("cancel_process"), NotOnSocket),
        (Request("open_file_picker"), NotOnSocket),
        (Request("declare_event_streams"), NotOnSocket),
        (Request("emit_event"), NotOnSocket),
        (Request("rollback_verify_result"), NotOnSocket),
        (Request("subscribe_plugin_events"), NotOnSocket),
        (Request("unsubscribe_plugin_events"), NotOnSocket),
        (Request("list_undo_checkpoints"), NotOnSocket),
        (Request("request_rollback"), NotOnSocket),
        (Frame("cli_trace"), Operator),
        (Frame("events_list"), Operator),
        (Frame("pane_lifecycle_wait"), Operator),
        (Frame("pane_lifecycle_follow"), Operator),
        (Frame("events_subscribe"), Protected),
        (Frame("events_declare"), Protected),
        (Frame("events_emit"), Protected),
        (Frame("command_view_follow"), Protected),
    ]
};

/// The caller axis of the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Caller {
    /// A process in a Plexi pane, at that pane's shell incarnation.
    PaneSelf,
    /// The same, acting on another pane. The entry admits it as a pane;
    /// the own-pane handlers refuse it
    /// (`own_pane_operations_from_another_pane_reach_no_effect`).
    PaneOther,
    /// A process whose walk reached the OS root with no pane on it.
    External,
    /// No kernel credential, or a walk that could not be completed.
    Unidentified,
}

/// Admission by policy and caller class: `None` dispatches, `Some(code)` is
/// the refusal the entry writes before any handler runs. Stated cell by cell
/// rather than derived from `admit`, so a change there fails here.
fn expected_admission(policy: VerbPolicy, caller: Caller) -> Option<&'static str> {
    use Caller::*;
    use VerbPolicy::*;
    match (policy, caller) {
        (Open, _) => None,
        (Operator, _) => None,
        (Protected | Widening, PaneSelf | PaneOther | External) => None,
        (Protected | Widening, Unidentified) => Some("permission_denied"),
        (Decision, _) => Some("approval_required"),
        (OwnPane, PaneSelf | PaneOther) => None,
        (OwnPane, External | Unidentified) => Some("permission_denied"),
        (NotOnSocket, _) => Some("not_on_socket"),
    }
}

/// Each suboperation's wire name, by exhaustive matches: a new variant does
/// not compile until it is named here, and then needs a table row.
fn suboperations() -> Vec<(&'static str, &'static str)> {
    let permissions = |op: PermissionsOp| match op {
        PermissionsOp::List => "list",
        PermissionsOp::Revoke => "revoke",
        PermissionsOp::Allow => "allow",
        PermissionsOp::Reset => "reset",
    };
    let needs_you = |op: NeedsYouOp| match op {
        NeedsYouOp::Ask => "ask",
        NeedsYouOp::Show => "show",
        NeedsYouOp::Answer => "answer",
        NeedsYouOp::Reconcile => "reconcile",
        NeedsYouOp::Complete => "complete",
    };
    let agents = |op: AgentsOp| match op {
        AgentsOp::CreateAgent => "create_agent",
        AgentsOp::ListAgents => "list_agents",
        AgentsOp::ListGrants => "list_grants",
        AgentsOp::SpawnRun => "spawn_run",
        AgentsOp::ListRuns => "list_runs",
        AgentsOp::ShowRun => "show_run",
        AgentsOp::FinishRun => "finish_run",
        AgentsOp::Delegate => "delegate",
        AgentsOp::Call => "call",
        AgentsOp::TeamRun => "team_run",
    };
    let dashboard = |op: DashboardOp| match op {
        DashboardOp::Open => "open",
        DashboardOp::Cancel => "cancel",
        DashboardOp::List => "list",
    };
    let mut all = Vec::new();
    for op in [PermissionsOp::List, PermissionsOp::Revoke, PermissionsOp::Allow, PermissionsOp::Reset] {
        assert_eq!(PermissionsOp::parse(permissions(op)), Some(op));
        all.push(("permissions", permissions(op)));
    }
    for op in [
        NeedsYouOp::Ask,
        NeedsYouOp::Show,
        NeedsYouOp::Answer,
        NeedsYouOp::Reconcile,
        NeedsYouOp::Complete,
    ] {
        assert_eq!(NeedsYouOp::parse(needs_you(op)), Some(op));
        all.push(("needs_you_api", needs_you(op)));
    }
    for op in [
        AgentsOp::CreateAgent,
        AgentsOp::ListAgents,
        AgentsOp::ListGrants,
        AgentsOp::SpawnRun,
        AgentsOp::ListRuns,
        AgentsOp::ShowRun,
        AgentsOp::FinishRun,
        AgentsOp::Delegate,
        AgentsOp::Call,
        AgentsOp::TeamRun,
    ] {
        assert_eq!(AgentsOp::parse(agents(op)), Some(op));
        all.push(("agents_api", agents(op)));
    }
    for op in [DashboardOp::Open, DashboardOp::Cancel, DashboardOp::List] {
        assert_eq!(DashboardOp::parse(dashboard(op)), Some(op));
        all.push(("agents_dashboard", dashboard(op)));
    }
    for (tool, _) in ASSISTANT_TOOLS {
        all.push(("assistant_host_tool", *tool));
    }
    all
}

fn stream_frames() -> Vec<StreamFrame> {
    // Exhaustive: a new frame does not compile until it is listed.
    let all = [
        StreamFrame::CliTrace,
        StreamFrame::EventsList,
        StreamFrame::EventsSubscribe,
        StreamFrame::EventsDeclare,
        StreamFrame::EventsEmit,
        StreamFrame::PaneLifecycleWait,
        StreamFrame::PaneLifecycleFollow,
        StreamFrame::CommandViewFollow,
    ];
    for frame in all {
        match frame {
            StreamFrame::CliTrace
            | StreamFrame::EventsList
            | StreamFrame::EventsSubscribe
            | StreamFrame::EventsDeclare
            | StreamFrame::EventsEmit
            | StreamFrame::PaneLifecycleWait
            | StreamFrame::PaneLifecycleFollow
            | StreamFrame::CommandViewFollow => {}
        }
    }
    all.to_vec()
}

/// A minimal value satisfying `schema`: required properties only, the first
/// alternative of a union, the first value of an enum.
fn minimal_instance(schema: &serde_json::Value, defs: &serde_json::Value) -> serde_json::Value {
    use serde_json::{json, Value};
    if let Some(reference) = schema["$ref"].as_str() {
        let name = reference.rsplit('/').next().unwrap();
        return minimal_instance(&defs[name], defs);
    }
    if let Some(value) = schema.get("const") {
        return value.clone();
    }
    if let Some(values) = schema["enum"].as_array() {
        return values[0].clone();
    }
    for union in ["oneOf", "anyOf"] {
        if let Some(options) = schema[union].as_array() {
            let option = options
                .iter()
                .find(|option| option["type"] != "null")
                .unwrap_or(&options[0]);
            return minimal_instance(option, defs);
        }
    }
    if let Some(parts) = schema["allOf"].as_array() {
        let mut merged = serde_json::Map::new();
        for part in parts {
            if let Value::Object(object) = minimal_instance(part, defs) {
                merged.extend(object);
            }
        }
        return Value::Object(merged);
    }
    let kind = match &schema["type"] {
        Value::Array(kinds) => kinds
            .iter()
            .filter_map(Value::as_str)
            .find(|kind| *kind != "null")
            .unwrap_or("null")
            .to_string(),
        Value::String(kind) => kind.clone(),
        _ => "object".to_string(),
    };
    match kind.as_str() {
        "string" => json!(""),
        "integer" | "number" => json!(0),
        "boolean" => json!(false),
        "array" => json!([]),
        "null" => Value::Null,
        _ => {
            let mut object = serde_json::Map::new();
            for name in schema["required"].as_array().into_iter().flatten() {
                let name = name.as_str().unwrap();
                object.insert(name.to_string(), minimal_instance(&schema["properties"][name], defs));
            }
            Value::Object(object)
        }
    }
}

/// Every `HostRequest` wire tag with a minimal request of that variant.
fn host_request_variants() -> Vec<(String, serde_json::Value)> {
    let schema = serde_json::to_value(schemars::schema_for!(crate::protocol::HostRequest)).unwrap();
    let defs = &schema["$defs"];
    schema["oneOf"]
        .as_array()
        .expect("HostRequest is a tagged union")
        .iter()
        .map(|variant| {
            let tag = minimal_instance(&variant["properties"]["type"], defs);
            (tag.as_str().unwrap().to_string(), minimal_instance(variant, defs))
        })
        .collect()
}

/// The request a row names, with any suboperation set.
fn row_request(op: Op, variants: &[(String, serde_json::Value)]) -> Option<crate::protocol::HostRequest> {
    let (tag, field, value) = match op {
        Op::Request(tag) => (tag, None, None),
        Op::Sub(tag, field, value) => (tag, Some(field), Some(value)),
        Op::Frame(_) => return None,
    };
    let (_, base) = variants
        .iter()
        .find(|(name, _)| name == tag)
        .unwrap_or_else(|| panic!("row {op:?} names no HostRequest variant"));
    let mut request = base.clone();
    if let (Some(field), Some(value)) = (field, value) {
        request[field] = serde_json::json!(value);
    }
    Some(
        serde_json::from_value(request.clone())
            .unwrap_or_else(|error| panic!("row {op:?}: {request} does not parse: {error}")),
    )
}

#[test]
fn every_socket_operation_is_classified() {
    let variants = host_request_variants();
    let mut missing = Vec::new();
    let multiplexed: std::collections::BTreeSet<&str> =
        suboperations().iter().map(|(tag, _)| *tag).collect();
    for (tag, _) in &variants {
        if multiplexed.contains(tag.as_str()) {
            continue;
        }
        if !SOCKET_OPERATIONS
            .iter()
            .any(|(op, _)| matches!(op, Op::Request(name) if name == tag))
        {
            missing.push(tag.clone());
        }
    }
    for (tag, value) in suboperations() {
        if !SOCKET_OPERATIONS
            .iter()
            .any(|(op, _)| matches!(op, Op::Sub(t, _, v) if *t == tag && *v == value))
        {
            missing.push(format!("{tag} {value}"));
        }
    }
    for frame in stream_frames() {
        if !SOCKET_OPERATIONS
            .iter()
            .any(|(op, _)| matches!(op, Op::Frame(name) if *name == frame.as_str()))
        {
            missing.push(frame.as_str().to_string());
        }
    }
    assert!(missing.is_empty(), "socket operations without a SOCKET_OPERATIONS row: {missing:?}");

    // Every row states the policy the entry applies.
    for (op, expected) in SOCKET_OPERATIONS {
        let actual = match *op {
            Op::Frame(name) => StreamFrame::parse(name)
                .unwrap_or_else(|| panic!("row {op:?} names no stream frame"))
                .policy(),
            _ => crate::app::socket_policy::socket_policy(&row_request(*op, &variants).unwrap())
                .unwrap_or_else(|refusal| panic!("row {op:?} refused: {refusal:?}")),
        };
        assert_eq!(actual, *expected, "{op:?}");
    }

    // An unknown suboperation is refused, never dispatched.
    for (tag, field) in [
        ("permissions", "op"),
        ("needs_you_api", "op"),
        ("agents_api", "op"),
        ("agents_dashboard", "op"),
        ("assistant_host_tool", "name"),
    ] {
        let request = row_request(Op::Sub(tag, field, "not-an-operation"), &variants).unwrap();
        let refusal = crate::app::socket_policy::socket_policy(&request).unwrap_err();
        assert_eq!(refusal.code, "invalid_argument", "{tag}");
    }
}

/// Drive one request through the admission entry from `caller` and return
/// the refusal code it wrote, or `None` when it was admitted. Refused cells
/// run the real entry; an admitted cell is checked at `admit` so no handler
/// (shutdown, close, spawn) runs.
fn entry_outcome(
    h: &mut HostHarness,
    request: crate::protocol::HostRequest,
    peer: Option<PeerWitness>,
    dispatch: bool,
) -> Option<String> {
    let caller = h.app.classify_socket_peer(peer.as_ref());
    let policy = crate::app::socket_policy::socket_policy(&request).unwrap();
    if crate::app::socket_policy::admit_request(&caller, &request, policy).is_ok() && !dispatch {
        return None;
    }
    let token = serde_json::to_value(&request)
        .ok()
        .and_then(|value| value["response_file"].as_str().map(str::to_string))
        .filter(|token| !token.is_empty())
        .unwrap_or_else(|| crate::rpc::response_file("socket-operations", "json"));
    let (server, client) = std::os::unix::net::UnixStream::pair().unwrap();
    // The host half is registered as the listener registers it; the reply is
    // read from the client half, so the test-only file fallback cannot
    // satisfy a cell.
    crate::rpc::ReplyRegistry::register(token.clone(), server);
    h.app.service_pane_ipc_message(crate::app::socket_caller::PaneIpcMessage::Request {
        ingress: crate::app::socket_caller::Ingress::Socket {
            peer,
            reply: Some(token.clone()),
        },
        request: Box::new(request),
    });
    h.run_frames(1);
    let reply = read_client(client, &token);
    assert!(!std::path::Path::new(&token).exists(), "the reply took the file fallback");
    if reply["ok"] == true || reply.get("error_code").is_none() {
        return None;
    }
    Some(reply["error_code"].as_str().unwrap().to_string())
}

#[test]
fn socket_operations_admit_by_caller_class() {
    let mut h = HostHarness::new();
    let pane = h.add_focused_terminal();
    let foreign = h.add_focused_terminal();
    let variants = host_request_variants();
    for (op, policy) in SOCKET_OPERATIONS {
        for caller in [Caller::PaneSelf, Caller::PaneOther, Caller::External, Caller::Unidentified] {
            let peer = match caller {
                Caller::PaneSelf | Caller::PaneOther => Some(pane_witness(&h, pane)),
                Caller::External => Some(outside()),
                Caller::Unidentified => None,
            };
            let expected = if matches!(caller, Caller::Unidentified)
                && matches!(
                    op,
                    Op::Request("secret_get" | "secret_set" | "secret_delete")
                ) {
                None
            } else {
                expected_admission(*policy, caller)
            };
            let actual = match *op {
                Op::Frame(name) => {
                    let classified = h.app.classify_socket_peer(peer.as_ref());
                    crate::app::socket_policy::admit(&classified, StreamFrame::parse(name).unwrap().policy())
                        .err()
                        .map(|refusal| refusal.code.to_string())
                }
                _ => {
                    // A request naming a pane names the caller's own for
                    // PaneSelf and a real foreign pane for PaneOther.
                    let mut request = serde_json::to_value(row_request(*op, &variants).unwrap()).unwrap();
                    if request.get("pane_id").is_some() {
                        let target = if caller == Caller::PaneOther { foreign } else { pane };
                        request["pane_id"] = serde_json::json!(target);
                    }
                    let request = serde_json::from_value(request).unwrap();
                    entry_outcome(&mut h, request, peer, false)
                }
            };
            assert_eq!(actual.as_deref(), expected, "{op:?} from {caller:?}");
        }
    }
    // Past the entry, an own-pane report aimed at the foreign pane is refused
    // by its handler and changes nothing on that pane.
    for tag in ["agent_done", "agent_ask"] {
        let mut request = serde_json::to_value(row_request(Op::Request(tag), &variants).unwrap()).unwrap();
        request["pane_id"] = serde_json::json!(foreign);
        request["response_file"] = serde_json::json!(crate::rpc::response_file("pane-other", "json"));
        let peer = pane_witness(&h, pane);
        let outcome = entry_outcome(&mut h, serde_json::from_value(request).unwrap(), Some(peer), true);
        assert_eq!(outcome.as_deref(), Some("permission_denied"), "{tag} at a foreign pane");
    }
    let foreign_terminal = h.app.terminal_incarnation(foreign).unwrap();
    assert!(!foreign_terminal.agent_completed && foreign_terminal.agent.is_none());
}

// Exercise the real decoder, classification and dispatch for metadata replies.
// cfg(test) writes those to the profile when the writer has no socket stream.
fn secret_metadata(
    h: &mut HostHarness,
    mut request: serde_json::Value,
    peer: &PeerWitness,
) -> serde_json::Value {
    let token = crate::rpc::response_file("secret-metadata", "json");
    request["response_file"] = token.as_str().into();
    crate::app::handle_socket_line_in(
        &crate::config::config_dir(),
        &request.to_string(),
        &h.ipc_tx,
        Some(peer),
        &mut Vec::new(),
    );
    h.hidden_frame();
    serde_json::from_slice(&std::fs::read(token).expect("metadata reply")).unwrap()
}

fn secret_ask(
    h: &mut HostHarness,
    request: &serde_json::Value,
    peer: &PeerWitness,
) -> crate::broker::gate::PendingView {
    let reply = secret_metadata(h, request.clone(), peer);
    assert_eq!(reply["status"], "permission_required", "{reply}");
    assert!(reply.get("value").is_none());
    let monitor =
        crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir()).unwrap();
    monitor
        .show_pending(reply["pending_request_id"].as_str().unwrap())
        .unwrap()
}

#[test]
fn external_secret_get_asks_and_returns_no_value() {
    use crate::broker::gate::{ApprovalChoice, PermissionMonitor};
    let mut h = HostHarness::new();
    let root = workspace_secret(&h, "ws-1280-get", "READ_1280", "test-read-value");
    let get = serde_json::json!({"type":"secret_get","name":"READ_1280","workspace_root":root});
    let peer = witness(std::process::id(), AncestryEnd::ReachedRoot);
    let ask = secret_ask(&mut h, &get, &peer);
    assert_eq!(ask.actor_id, crate::broker::principal::EXTERNAL_ID);
    assert!(ask.input_summary.contains("terminal outside Plexi"));
    PermissionMonitor::for_profile(&crate::config::config_dir())
        .unwrap()
        .approve_pending(&ask.pending_request_id, ApprovalChoice::Once)
        .unwrap();
    assert_eq!(
        send(&mut h, get.clone(), Some(peer.clone()))["value"],
        "test-read-value"
    );
    secret_ask(&mut h, &get, &peer);
}

#[test]
fn detached_secret_get_asks() {
    let mut h = HostHarness::new();
    let peer = witness(std::process::id(), AncestryEnd::LookupFailed);
    let get = serde_json::json!({"type":"secret_get","name":"DETACHED_1280","global":true});
    let ask = secret_ask(&mut h, &get, &peer);
    assert_eq!(ask.actor_id, crate::broker::principal::EXTERNAL_ID);
    h.app.sync_attention_queue();
    let card = h
        .app
        .attention_queue
        .items
        .iter()
        .find(|card| card.id == ask.pending_request_id)
        .unwrap();
    assert!(card.summary.contains("detached"));
    assert_eq!(
        card.options
            .iter()
            .map(|o| o.key.as_str())
            .collect::<Vec<_>>(),
        ["allow_once", "deny"]
    );
    assert!(ask.input_summary.contains("detached"));
}

#[test]
fn external_caller_is_not_admitted_by_a_session_grant() {
    use crate::broker::gate::{ApprovalChoice, PermissionMonitor};
    let mut h = HostHarness::new();
    let peer = witness(std::process::id(), AncestryEnd::ReachedRoot);
    let get = serde_json::json!({"type":"secret_get","name":"SESSION_1280","global":true});
    let ask = secret_ask(&mut h, &get, &peer);
    let monitor = PermissionMonitor::for_profile(&crate::config::config_dir()).unwrap();
    assert!(monitor
        .approve_pending(&ask.pending_request_id, ApprovalChoice::Session)
        .is_err());
    // Simulate a valid session row inherited from an earlier host. Rejection
    // at the click alone cannot prove that admission ignores standing rows.
    monitor
        .approve_pending(&ask.pending_request_id, ApprovalChoice::Once)
        .unwrap();
    {
        let mut store = monitor.store();
        let row = store
            .records_mut()
            .iter_mut()
            .find(|row| row.target_id == "secret.get")
            .unwrap();
        row.duration = crate::broker::GrantDuration::Session;
        row.session_id = Some(monitor.session_id().into());
    }
    secret_ask(&mut h, &get, &peer);
}

#[test]
fn external_secret_set_and_delete_ask_then_apply_once() {
    use crate::broker::gate::{ApprovalChoice, PermissionMonitor};
    use crate::workspace::secrets::{keychain_user_name, system_store};
    let mut h = HostHarness::new();
    let monitor = PermissionMonitor::for_profile(&crate::config::config_dir()).unwrap();
    let name = format!("MUTATE_{}", uuid::Uuid::new_v4().simple());
    let logs = crate::testing::test_log::capture_with_debug();
    let account = keychain_user_name(&name);
    let peer = witness(std::process::id(), AncestryEnd::ReachedRoot);
    for op in ["set", "delete"] {
        let mut request =
            serde_json::json!({"type":format!("secret_{op}"), "name":name, "global":true});
        if op == "set" {
            request["value"] = "mutation-canary-before-click".into();
        }
        let ask = secret_ask(&mut h, &request, &peer);
        assert_eq!(ask.tool, format!("secret.{op}"));
        assert_eq!(ask.resource_id.unwrap(), format!("{name}@global"));
        assert!(!ask.input_summary.contains("mutation-canary"));
        assert_eq!(system_store().get(&account).is_some(), op == "delete");
        monitor
            .approve_pending(&ask.pending_request_id, ApprovalChoice::Once)
            .unwrap();
        if op == "set" {
            // Only the name and scope are approved. The retry supplies the value.
            request["value"] = "mutation-canary".into();
        }
        assert_eq!(
            secret_metadata(&mut h, request.clone(), &peer)["status"],
            "ok"
        );
        assert_eq!(
            system_store().get(&account).as_deref().map(String::as_str),
            (op == "set").then_some("mutation-canary")
        );
        secret_ask(&mut h, &request, &peer);
        assert!(monitor
            .audit_records()
            .iter()
            .any(|r| r.kind == "secret_mutation" && r.operation_id == op && r.decision == "ok"));
    }
    assert!(logs.lines().iter().any(|line| line.contains("secret.set")));
    assert!(!logs
        .lines()
        .iter()
        .any(|line| line.contains("mutation-canary")));
    assert!(
        !std::fs::read_to_string(crate::config::config_dir().join("permission-audit.jsonl"))
            .unwrap()
            .contains("mutation-canary")
    );
}

#[test]
fn pane_secret_mutations_ask_as_the_pane_within_its_root() {
    let mut h = HostHarness::new();
    let root = workspace_secret(&h, "ws-1280-pane", "PANE_1280", "initial");
    let pane = h.add_focused_terminal();
    let peer = pane_witness(&h, pane);
    let other = tempfile::tempdir().unwrap();
    for op in ["set", "delete"] {
        let mut req = serde_json::json!({"type":format!("secret_{op}"), "name":"PANE_1280","workspace_root":root});
        if op == "set" {
            req["value"] = "fixture".into();
        }
        let ask = secret_ask(&mut h, &req, &peer);
        assert_eq!(ask.actor_id, format!("pane:{pane}"));
        let mut foreign = req.clone();
        foreign["workspace_root"] = serde_json::json!(other.path());
        assert_eq!(
            secret_metadata(&mut h, foreign, &peer)["error_code"],
            "permission_denied"
        );
        crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir())
            .unwrap()
            .approve_pending(
                &ask.pending_request_id,
                crate::broker::gate::ApprovalChoice::Once,
            )
            .unwrap();
        let reply = secret_metadata(&mut h, req, &peer);
        assert_eq!(reply["target"]["Workspace"]["id"], "ws-1280-pane");
        let value = crate::workspace::secrets::system_store().get("plexi:ws-1280-pane:PANE_1280");
        assert_eq!(
            value.as_deref().map(String::as_str),
            (op == "set").then_some("fixture")
        );
    }
}

/// Old CLI -> new host: each frame shape the pre-1285 CLI sent with an
/// identity claim (taken from `origin/alpha` at e4259f67e: `notify
/// dismiss`, `pane send`, `agent open`, `agent spawn`, `agent team run`,
/// `plugin tools`) gets one `removed_field` reply naming the claim, on the
/// connection the client is reading, fire-and-forget frames included, and
/// reaches no handler.
#[test]
fn an_older_cli_frame_with_an_identity_claim_is_refused_by_name() {
    let mut h = HostHarness::new();
    let pane = h.add_focused_terminal();
    let target = h.add_focused_terminal();
    let context = h.app.windows[h.app.active_window].context_id;
    let workspace = h.workspace_root();
    let token = || crate::rpc::response_file("old-cli", "json");
    let frames = [
        (
            serde_json::json!({"type": "dismiss_notification", "notify_id": "n",
                "source_context_id": context, "source_pane_id": pane, "response_file": token()}),
            "source_pane_id",
        ),
        (
            serde_json::json!({"type": "send_to_pane", "pane_id": target, "text": "echo hi",
                "submit": false, "paste": false, "operation_id": "op",
                "generated_operation_id": true, "peer_ancestry": []}),
            "peer_ancestry",
        ),
        (
            serde_json::json!({"type": "agent_open", "name": "n", "cwd": workspace,
                "caller_pane_id": pane}),
            "caller_pane_id",
        ),
        (
            serde_json::json!({"type": "agent_spawn", "role": "r", "harness": "script",
                "command": "sleep 30", "task": "", "ephemeral": false, "cwd": workspace,
                "parent_pane_id": pane}),
            "parent_pane_id",
        ),
        (
            serde_json::json!({"type": "agents_api", "op": "team_run", "response_file": token(),
                "payload": {"execution_cwd": workspace, "name": "t", "question": "q",
                    "caller_pane": pane}}),
            "caller_pane",
        ),
        (
            serde_json::json!({"type": "list_plugin_tools", "caller_pane_id": pane,
                "caller_context_id": context}),
            "caller_pane_id",
        ),
    ];
    let panes_before: usize = h.app.windows.iter().map(|w| w.panes.len()).sum();
    for (frame, field) in frames {
        let mut ack = Vec::new();
        crate::app::handle_socket_line_in(
            &crate::config::config_dir(),
            &frame.to_string(),
            &h.ipc_tx,
            Some(&pane_witness(&h, pane)),
            &mut ack,
        );
        h.run_frames(2);
        let reply: serde_json::Value = serde_json::from_slice(&ack)
            .unwrap_or_else(|_| panic!("{frame}: no typed reply: {:?}", String::from_utf8_lossy(&ack)));
        assert_eq!(reply["error_code"], "removed_field", "{frame}: {reply}");
        assert_eq!(reply["field"], field, "{frame}: {reply}");
    }
    let panes_after: usize = h.app.windows.iter().map(|w| w.panes.len()).sum();
    assert_eq!(panes_before, panes_after, "a refused frame reached a handler");
    assert!(h.app.team_runs.is_empty());
    assert!(
        !h.terminal_backend(target).take_input_tap().ends_with(b"echo hi"),
        "a refused pane send typed into the target"
    );
}

// ── Stint 1673: the typed caller under cover and across ids ──────────────

/// A `tester` worker under its role's ceiling (`deny` lists tools), its
/// terminal then covered by a builtin overlay, and the witness of its shell.
fn covered_worker(h: &mut HostHarness, deny: &str) -> (u64, PeerWitness) {
    let root = h.workspace_root();
    h.app.set_context_root(root.clone(), None);
    let dir = crate::agent::workspace_agents_dir(&root).join("tester");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("AGENT.md"), "Test").unwrap();
    std::fs::write(
        dir.join("settings.toml"),
        format!(
            "[agent]\nid='tester'\ndisplay_name='Tester'\n[harness]\ncli='pi'\nephemeral=false\n[permissions]\ndefault_posture='ask'\ndeny=[{deny}]\n"
        ),
    )
    .unwrap();
    let (worker, _) = h
        .app
        .spawn_agent_worker_pane(
            "tester", "pi", "sleep 30", "Run", None, false, None, Some(root), None, None, false,
            None,
        )
        .expect("tester worker");
    let pid = h.app.terminal_incarnation(worker).unwrap().backend.child_pid();
    h.focus_pane(worker);
    h.app
        .launch_view_by_id_with_layout("file_browser", Some("overlay".into()), &[], None)
        .unwrap();
    assert!(
        h.app.windows.iter().any(|w| w.panes.get(&worker).is_some_and(|p| p
            .as_view()
            .is_some_and(|v| v.overlay_replaced.is_some()))),
        "fixture: the worker is covered"
    );
    (worker, PeerWitness::for_live_pids(&[pid]))
}

/// A workspace with an id and one stored secret.
fn workspace_secret(h: &HostHarness, id: &str, name: &str, value: &str) -> std::path::PathBuf {
    let root = h.workspace_root();
    let dir = root.join(crate::config::workspace_channel_dir());
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("workspace.toml"), format!("id = \"{id}\"\n")).unwrap();
    crate::workspace::secrets::system_store()
        .set(&format!("plexi:{id}:{name}"), value)
        .unwrap();
    root
}

/// Under cover, an exact approval satisfies the worker's Ask.
#[test]
fn a_covered_worker_secret_get_is_satisfied_by_its_exact_approval() {
    let mut h = HostHarness::new();
    let (worker, peer) = covered_worker(&mut h, "");
    let canary = "covered-ask-canary-1673";
    let root = workspace_secret(&h, "ws-1673-ask", "KEY_1673", canary);
    let get = serde_json::json!({"type": "secret_get", "name": "KEY_1673",
        "workspace_root": root});
    let first = send(&mut h, get.clone(), Some(peer.clone()));
    assert_eq!(first["status"], "permission_required", "{first}");
    let monitor =
        crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir()).unwrap();
    let pending = first["pending_request_id"].as_str().unwrap();
    assert_eq!(
        monitor.show_pending(pending).unwrap().actor_id,
        format!("pane:{worker}"),
        "the request is the covered worker's"
    );
    monitor
        .approve_pending(pending, crate::broker::gate::ApprovalChoice::Once)
        .unwrap();
    let granted = send(&mut h, get, Some(peer));
    assert_eq!(granted["status"], "value", "{granted}");
    assert_eq!(granted["value"], canary);
}

/// Under cover, the worker's explicit Deny beats an exact stored allow.
#[test]
fn a_covered_worker_deny_beats_an_exact_stored_allow() {
    let mut h = HostHarness::new();
    let (worker, peer) = covered_worker(&mut h, "'secret.get'");
    let canary = "covered-deny-canary-1673";
    let root = workspace_secret(&h, "ws-1673-deny", "KEY_1673", canary);
    // An exact Always row for this pane, written as an unrestricted read
    // of the same secret would be.
    let monitor =
        crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir()).unwrap();
    let actor = crate::workspace::secrets::SecretActor {
        kind: crate::broker::ActorType::Agent,
        id: format!("pane:{worker}"),
    };
    let scope = crate::workspace::secrets::SecretScope::Workspace {
        id: "ws-1673-deny".to_string(),
        root: crate::platform::path::canonical_or_self(&root),
    };
    let read = || {
        crate::workspace::secrets::read_pane_secret_with_posture(
            &monitor,
            crate::workspace::secrets::system_store(),
            &actor,
            "KEY_1673",
            &scope,
            &crate::config::config_dir(),
            None,
            None,
        )
        .unwrap()
        .result
    };
    let crate::workspace::secrets::ReadResult::PermissionRequired { pending_id } = read() else {
        panic!("expected a pending request");
    };
    monitor
        .approve_pending(&pending_id, crate::broker::gate::ApprovalChoice::Always)
        .unwrap();
    assert!(
        matches!(read(), crate::workspace::secrets::ReadResult::Value(_)),
        "fixture: the stored allow opens the secret without a posture"
    );
    let reply = send(
        &mut h,
        serde_json::json!({"type": "secret_get", "name": "KEY_1673",
            "workspace_root": root}),
        Some(peer),
    );
    assert_eq!(reply["status"], "denied", "{reply}");
    assert!(!reply.to_string().contains(canary), "{reply}");
}

/// A covered worker's own `agent done` reaches its terminal: the completion
/// flags are set on the worker under the overlay.
#[test]
fn a_covered_workers_agent_done_completes_its_terminal() {
    let mut h = HostHarness::new();
    let (worker, peer) = covered_worker(&mut h, "");
    let reply = send(
        &mut h,
        serde_json::json!({"type": "agent_done", "pane_id": worker,
            "result": "{\"ok\":true}", "summary": "done"}),
        Some(peer),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    let terminal = h.app.terminal_incarnation(worker).unwrap();
    assert!(terminal.agent_completed, "the covered terminal is completed");
    assert!(terminal.fleet_result.is_some());
    assert!(terminal.fleet_done_unseen);
}

/// A covered worker that spawns an agent is its parent, and the child
/// inherits the worker's restrictions.
#[test]
fn a_covered_worker_spawn_keeps_its_provenance_and_restrictions() {
    let _mesh = crate::release::set_test_experiments(&[crate::release::ReleaseFeature::AgentMesh]);
    let mut h = HostHarness::new();
    let (worker, peer) = covered_worker(&mut h, "'tool.secret'");
    let workspace = h.workspace_root();
    let reply = send(
        &mut h,
        serde_json::json!({"type": "agent_spawn", "role": "child", "harness": "script",
            "command": "sleep 30", "cwd": workspace}),
        Some(peer),
    );
    assert_eq!(reply["ok"], true, "{reply}");
    let child = reply["pane_id"].as_u64().unwrap();
    let grant = h.app.pane_authority(child).unwrap().grant().clone();
    assert_eq!(grant.parent, Some(worker), "the covered worker is the parent");
    assert_eq!(reply["parent_pane_id"], worker, "{reply}");
    assert_eq!(reply["spawned_by"], "agent:tester", "the worker's agent, as uncovered: {reply}");
    let posture = h.app.pane_ceiling_or_none(child).expect("the child is restricted");
    assert!(
        posture.deny.iter().any(|tool| tool == "tool.secret"),
        "the child inherits the worker's deny: {posture:?}"
    );
}

/// A caller whose context id and window id differ: its question and its
/// plugin tool list are scoped to the context, never the window id.
#[test]
fn a_callers_context_is_its_context_id_never_its_window_id() {
    let mut h = HostHarness::new();
    let pane = h.add_focused_terminal();
    let window = h.app.windows.iter().position(|w| w.panes.contains_key(&pane)).unwrap();
    let context = h.app.windows[window].context_id;
    let distinct = h.app.windows.iter().map(|w| w.window_id.max(w.context_id)).max().unwrap() + 100;
    h.app.windows[window].window_id = distinct;
    h.app.next_window_id = h.app.next_window_id.max(distinct + 1);
    assert_ne!(h.app.windows[window].window_id, context);
    let peer = pane_witness(&h, pane);
    let asked = send(
        &mut h,
        serde_json::json!({"type": "needs_you_api", "op": "ask", "payload": {"question": "q"}}),
        Some(peer.clone()),
    );
    assert_eq!(asked["ok"], true, "{asked}");
    assert_eq!(asked["item"]["context_id"], context, "{asked}");
    let tools = send(&mut h, serde_json::json!({"type": "list_plugin_tools"}), Some(peer));
    assert_eq!(tools["context_id"], context, "{tools}");
}

/// A Needs-you continuation or item is read back only by the caller that
/// filed it: `show` and `reconcile` never hand one principal's questions,
/// answers or continuations to another socket caller, whatever filters the
/// payload names.
#[test]
fn needs_you_show_and_reconcile_are_scoped_to_the_asker() {
    let mut h = HostHarness::new();
    let asker = h.add_focused_terminal();
    let other = h.add_focused_terminal();
    let (asker_peer, other_peer) = (pane_witness(&h, asker), pane_witness(&h, other));
    let asked = send(
        &mut h,
        serde_json::json!({"type": "needs_you_api", "op": "ask",
            "payload": {"question": "private?", "session_id": "s-1285"}}),
        Some(asker_peer.clone()),
    );
    let id = asked["id"].as_str().unwrap().to_string();
    let ids = |reply: &serde_json::Value| -> Vec<String> {
        reply["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|item| item["id"].as_str().map(str::to_string))
            .collect()
    };
    let reconcile = |pane: u64| {
        serde_json::json!({"type": "needs_you_api", "op": "reconcile",
            "payload": {"pane_id": pane, "session_id": "s-1285"}})
    };
    let show = serde_json::json!({"type": "needs_you_api", "op": "show", "payload": {"id": id}});
    for peer in [Some(other_peer), Some(outside())] {
        let listed = send(&mut h, reconcile(asker), peer.clone());
        assert!(!ids(&listed).contains(&id), "{listed}");
        let unfiltered = send(
            &mut h,
            serde_json::json!({"type": "needs_you_api", "op": "reconcile", "payload": {}}),
            peer.clone(),
        );
        assert!(!ids(&unfiltered).contains(&id), "{unfiltered}");
        let shown = send(&mut h, show.clone(), peer);
        assert_eq!(shown["ok"], false, "{shown}");
        assert!(shown["item"].is_null(), "{shown}");
    }
    let own = send(&mut h, reconcile(asker), Some(asker_peer.clone()));
    assert!(ids(&own).contains(&id), "{own}");
    let shown = send(&mut h, show, Some(asker_peer));
    assert_eq!(shown["ok"], true, "{shown}");
}

/// Codex 3290 r1 P1: a context root moves only for the caller's own context
/// and only once the caller is admitted for it as itself. A pane cannot name
/// another context, an outside process names none, and nothing moves before
/// an approval.
#[test]
fn a_once_set_root_is_spent_by_one_change() {
    let mut h = HostHarness::new();
    let pane = h.add_focused_terminal();
    let peer = pane_witness(&h, pane);
    let context = h.app.windows[h.app.active_window].context_id;
    let before = h.app.context_root_for(context);
    let target = tempfile::tempdir().unwrap();
    let set = |context_id: Option<u64>| {
        let mut request = serde_json::json!({"type": "set_context_root", "root": target.path()});
        if let Some(id) = context_id {
            request["context_id"] = serde_json::json!(id);
        }
        request
    };
    let foreign = send(&mut h, set(Some(context + 999)), Some(peer.clone()));
    assert!(foreign["error"].as_str().unwrap_or("").contains("only its own context"), "{foreign}");
    let outside_unnamed = send(&mut h, set(None), Some(outside()));
    assert!(outside_unnamed["error"].as_str().unwrap_or("").contains("permission_denied"), "{outside_unnamed}");
    let outside_named = send(&mut h, set(Some(context)), Some(outside()));
    assert!(outside_named["error"].as_str().unwrap_or("").contains("needs approval"), "{outside_named}");
    let asked = send(&mut h, set(None), Some(peer.clone()));
    assert!(asked["error"].as_str().unwrap_or("").contains("needs approval"), "{asked}");
    assert_eq!(h.app.context_root_for(context), before, "nothing moved before approval");

    let monitor =
        crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir()).unwrap();
    let pending = monitor
        .list_pending()
        .into_iter()
        .find(|row| row.tool == "context.set_root" && row.actor_id == format!("pane:{pane}"))
        .expect("the pane's request is pending")
        .pending_request_id;
    monitor
        .approve_pending(&pending, crate::broker::gate::ApprovalChoice::Once)
        .unwrap();
    let applied = send(&mut h, set(None), Some(peer.clone()));
    assert_eq!(applied["context_id"], context, "{applied}");
    assert_eq!(
        h.app.context_root_for(context).map(|p| crate::platform::path::canonical_or_self(&p)),
        Some(crate::platform::path::canonical_or_self(target.path()))
    );
    h.app.set_context_root(before.unwrap(), Some(context));
    let replay = send(&mut h, set(None), Some(peer));
    assert!(
        replay["error"].as_str().unwrap().contains("needs approval"),
        "{replay}"
    );
    assert!(
        monitor.store().records().iter().any(|row| {
            row.target_id == "context.set_root" && row.consumed && row.bound_operation_id.is_some()
        }),
        "one broker-claimed use"
    );
}

/// Codex 3290 r1 P2: a pre-1285 `plexi notify` (its exact wire bytes from
/// `origin/alpha` e4259f67e, a forged pane claim included) reads only the ack
/// byte. The host acks and posts the notification, owned by the kernel-named
/// peer, never by the claimed pane.
#[test]
fn an_older_cli_notify_posts_as_its_peer_and_the_client_reads_the_ack() {
    let mut h = HostHarness::new();
    let sender = h.add_focused_terminal();
    let claimed = h.add_focused_terminal();
    let context = h.app.windows[h.app.active_window].context_id;
    h.app.notifications_enabled = true;
    let before = h.app.pending_notifications.len();
    let line = format!(
        "{{\"type\":\"notify\",\"notify_id\":\"cli:{claimed}:legacy\",\"title\":\"Done\",\"body\":\"\",\"kind\":\"message\",\"options\":[],\"source_context_id\":{context},\"source_pane_id\":{claimed}}}"
    );
    let mut ack = Vec::new();
    crate::app::handle_socket_line_in(
        &crate::config::config_dir(),
        &line,
        &h.ipc_tx,
        Some(&pane_witness(&h, sender)),
        &mut ack,
    );
    h.run_frames(2);
    // The old client's read: one byte, the identity ack.
    assert_eq!(ack.first().copied(), Some(0x06), "{:?}", String::from_utf8_lossy(&ack));
    assert_eq!(h.app.pending_notifications.len(), before + 1, "the notification was posted");
    let posted = h.app.pending_notifications.last().unwrap();
    assert_eq!(posted.dismiss_owner_pane_id, sender, "owned by the peer");
    assert_ne!(posted.dismiss_owner_pane_id, claimed);
}

/// Codex 3290 r1 P2: deferred work keeps the caller it was admitted for. A
/// pane closed after classification subscribes as that closed pane (and is
/// refused), never as an outside process; a secret read for it is withheld.
#[test]
fn deferred_work_keeps_the_admitted_pane_and_refuses_it_once_closed() {
    use crate::host::event_subscriptions::SubscriberAuthority;
    let mut h = HostHarness::new();
    let pane = h.add_focused_terminal();
    let caller = h.app.classify_socket_peer(Some(&pane_witness(&h, pane)));
    assert!(matches!(caller, SocketCaller::Pane(_)));
    h.app.close_pane_by_id(pane);
    match crate::app::PlexiApp::subscriber_authority(Some(&caller)) {
        SubscriberAuthority::Pane(admitted) => {
            assert!(admitted.current().is_err(), "a closed pane is no longer admitted")
        }
        other => panic!("a closed pane subscribed as {other:?}"),
    }
    assert!(matches!(
        crate::app::PlexiApp::subscriber_authority(None),
        SubscriberAuthority::Sealed
    ));

    let token = crate::rpc::response_file("deferred-secret", "json");
    let (server, client) = std::os::unix::net::UnixStream::pair().unwrap();
    crate::rpc::ReplyRegistry::register(token.clone(), server);
    h.app.serve_secret_get(
        "DEFERRED_1285",
        true,
        None,
        &crate::broker::caller::RequestOrigin::Socket(caller),
        &token,
    );
    let reply = read_client(client, &token);
    assert_eq!(reply["status"], "denied", "{reply}");
    assert!(reply.get("value").is_none(), "{reply}");
}

/// The PaneOther cells past the entry: a pane reporting for another pane
/// changes nothing on it.
#[test]
fn own_pane_operations_from_another_pane_reach_no_effect() {
    let mut h = HostHarness::new();
    let caller = h.add_focused_terminal();
    let target = h.add_focused_terminal();
    let peer = pane_witness(&h, caller);
    let state = serde_json::json!({"type": "set_agent_state", "pane_id": target,
        "state": "working", "agent": "claude"});
    let mut ack = Vec::new();
    crate::app::handle_socket_line_in(
        &crate::config::config_dir(),
        &state.to_string(),
        &h.ipc_tx,
        Some(&peer),
        &mut ack,
    );
    h.run_frames(2);
    assert!(
        h.app.terminal_incarnation(target).unwrap().agent.is_none(),
        "another pane's state was set"
    );
    let done = send(
        &mut h,
        serde_json::json!({"type": "agent_done", "pane_id": target, "result": "{}", "summary": "s"}),
        Some(peer.clone()),
    );
    assert_eq!(done["error_code"], "permission_denied", "{done}");
    assert!(!h.app.terminal_incarnation(target).unwrap().agent_completed);
    let asked = send(
        &mut h,
        serde_json::json!({"type": "agent_ask", "pane_id": target, "question": "q"}),
        Some(peer),
    );
    assert_eq!(asked["error_code"], "permission_denied", "{asked}");
}

/// Codex 3290 r1/r3: the dashboard's cancel takes the caller. Another pane's
/// cancel leaves a live run it did not start running; the owning pane's
/// ordinary cancel (the frame `plexi agents-dashboard cancel` sends)
/// finishes it. Follow is checked through the stream in
/// `app::tests::misc_tests::dashboard_follow_stream_serves_only_the_callers_workspace`.
#[test]
fn dashboard_cancel_stops_only_the_callers_own_run() {
    let _experiments = crate::release::set_test_experiments(&[
        crate::release::ReleaseFeature::AgentsDashboard,
        crate::release::ReleaseFeature::AgentMesh,
    ]);
    let mut h = HostHarness::new();
    let owner = h.add_focused_terminal();
    let other = h.add_focused_terminal();
    let root = h.workspace_root();
    h.app.set_context_root(root.clone(), None);
    let host = crate::agent::runs::AgentsCaller::Host { ceiling: Vec::new() };
    let created = crate::agent::runs::handle_request(
        "create_agent",
        &serde_json::json!({"workspace": root, "name": "worker", "grants": ["agents.ping=ask"]}),
        &host,
    );
    assert_eq!(created["ok"], true, "{created}");
    let (owner_peer, other_peer) = (pane_witness(&h, owner), pane_witness(&h, other));
    let started = send(
        &mut h,
        serde_json::json!({"type": "agents_api", "op": "spawn_run",
            "payload": {"workspace": root, "agent": "worker", "admission": "adm-dash"}}),
        Some(owner_peer.clone()),
    );
    assert_eq!(started["ok"], true, "{started}");
    let run = started["run"]["id"].as_str().unwrap().to_string();
    let active = |run: &str| {
        crate::agent::runs::handle_request(
            "show_run",
            &serde_json::json!({"workspace": root, "id": run}),
            &crate::agent::runs::AgentsCaller::Host { ceiling: Vec::new() },
        )["run"]["active"]
            .as_bool()
            .unwrap()
    };
    let cancel = serde_json::json!({"type": "agents_dashboard", "op": "cancel",
        "payload": {"workspace": root, "run": run}});
    let foreign = send(&mut h, cancel.clone(), Some(other_peer));
    assert_eq!(foreign["error_code"], "permission_denied", "{foreign}");
    assert!(active(&run), "another pane's cancel stopped the run");
    let own = send(&mut h, cancel, Some(owner_peer));
    assert_eq!(own["ok"], true, "{own}");
    assert!(!active(&run), "the owner's cancel left the run running");
}

/// Design 1285 section 4, condition 3: with the dashboard experiment off,
/// both raw host entries refuse: the list request and the follow stream.
#[test]
fn the_dashboard_experiment_gates_both_raw_host_entries() {
    let _none = crate::release::set_test_experiments(&[]);
    let mut h = HostHarness::new();
    let pane = h.add_focused_terminal();
    let root = h.workspace_root();
    h.app.set_context_root(root.clone(), None);
    let peer = pane_witness(&h, pane);
    let caller = h.app.classify_socket_peer(Some(&peer));
    let listed = send(
        &mut h,
        serde_json::json!({"type": "agents_dashboard", "op": "list",
            "payload": {"workspace": root}}),
        Some(peer.clone()),
    );
    assert_eq!(listed["error_code"], "experiment_disabled", "{listed}");
    assert!(listed.get("runs").is_none() && listed.get("needs_you").is_none(), "{listed}");
    let follow = h
        .app
        .admit_dashboard_workspace(&caller, &root.to_string_lossy())
        .unwrap_err();
    assert_eq!(follow.code, "experiment_disabled");

    drop(_none);
    let _on =
        crate::release::set_test_experiments(&[crate::release::ReleaseFeature::AgentsDashboard]);
    let listed = send(
        &mut h,
        serde_json::json!({"type": "agents_dashboard", "op": "list",
            "payload": {"workspace": root}}),
        Some(peer),
    );
    assert_eq!(listed["ok"], true, "{listed}");
    assert!(h.app.admit_dashboard_workspace(&caller, &root.to_string_lossy()).is_ok());
}

/// Codex 3290 r3 B1: an explicit context or window scope the host cannot
/// resolve (an outside caller) is refused and posts nothing; it is never
/// widened to global. An unscoped notify from outside still posts.
#[test]
fn an_explicit_notify_scope_the_host_cannot_resolve_is_refused_not_widened() {
    let mut h = HostHarness::new();
    h.app.notifications_enabled = true;
    let before = h.app.pending_notifications.len();
    for scope in ["context", "window"] {
        let reply = send(
            &mut h,
            serde_json::json!({"type": "notify", "notify_id": format!("n-{scope}"),
                "title": "private", "body": "", "kind": "message", "options": [],
                "scope": scope}),
            Some(outside()),
        );
        assert_eq!(reply["error_code"], "scope_unresolvable", "{scope}: {reply}");
    }
    assert_eq!(h.app.pending_notifications.len(), before, "a refused notify was posted");
    let mut ack = Vec::new();
    crate::app::handle_socket_line_in(
        &crate::config::config_dir(),
        &serde_json::json!({"type": "notify", "notify_id": "n-unscoped", "title": "t",
            "body": "", "kind": "message", "options": []})
        .to_string(),
        &h.ipc_tx,
        Some(&outside()),
        &mut ack,
    );
    h.run_frames(2);
    assert_eq!(h.app.pending_notifications.len(), before + 1, "an unscoped notify posts");
}

/// Codex 3290 r3 B2: a pane whose shell start time could not be read at
/// spawn is still that pane's caller (read again lazily, matched by pid
/// while unreadable, as alpha did); it never becomes an outside process.
/// A walk that cannot be completed still drives panes (operator requests,
/// as alpha allowed) and is refused anything protected.
#[test]
fn a_pane_shell_without_a_recorded_start_is_still_its_pane() {
    let mut h = HostHarness::new();
    for late_read_fails in [false, true] {
        let pane = h.add_focused_terminal();
        let shell = h.shell_pid(pane);
        h.app
            .windows
            .iter_mut()
            .find_map(|w| w.panes.get_mut(&pane))
            .and_then(|p| p.as_terminal_mut())
            .unwrap()
            .forget_shell_start_for_test(late_read_fails);
        // With the late read failing too, the live shell matches by pid, as
        // on alpha; with it succeeding, by pid and start.
        let caller = h.app.classify_socket_peer(Some(&PeerWitness::for_live_pids(&[shell])));
        assert!(
            matches!(caller, SocketCaller::Pane(ref found) if found.terminal_pane == pane),
            "late_read_fails={late_read_fails}: {caller:?}"
        );
    }

    let caller = h.add_focused_terminal();
    let target = h.add_focused_terminal();
    h.app
        .windows
        .iter_mut()
        .find_map(|window| window.panes.get_mut(&caller))
        .and_then(|pane| pane.as_terminal_mut())
        .unwrap()
        .forget_shell_start_for_test(true);
    let peer = pane_witness(&h, caller);
    let refusal = send(
        &mut h,
        serde_json::json!({"type": "agent_done", "pane_id": target, "result": "{}", "summary": "s"}),
        Some(peer),
    );
    assert_eq!(refusal["error_code"], "permission_denied", "{refusal}");
    assert_eq!(
        refusal["error"],
        format!("pane {caller} cannot act as pane {target}")
    );
    assert!(!h.app.terminal_incarnation(target).unwrap().agent_completed);

    let incomplete = Some(witness(std::process::id(), AncestryEnd::LookupFailed));
    let listed = send(&mut h, serde_json::json!({"type": "list_panes"}), incomplete.clone());
    assert!(listed.get("error_code").is_none(), "an operator request is served: {listed}");
    let secret = send(
        &mut h,
        serde_json::json!({"type": "pane_secret_list", "context": "1"}),
        incomplete,
    );
    assert_eq!(secret["error_code"], "permission_denied", "{secret}");
}

/// Codex 3290 r3 B5: `host stop` in alpha's exact frame, from a caller the
/// host cannot identify, and with a legacy identity claim, is served: an
/// older client must never read a policy refusal (it would kill the host).
#[test]
fn an_older_host_stop_is_served_to_any_same_user_caller() {
    // Alpha's `host stop` frame names its reply token (`request_on_stream`).
    for frame in [
        serde_json::json!({"type": "shutdown",
            "response_file": crate::rpc::response_file("host-stop", "json")}),
        serde_json::json!({"type": "shutdown", "peer_ancestry": [],
            "response_file": crate::rpc::response_file("host-stop", "json")}),
    ] {
        let mut h = HostHarness::new();
        let mut ack = Vec::new();
        crate::app::handle_socket_line_in(
            &crate::config::config_dir(),
            &frame.to_string(),
            &h.ipc_tx,
            None,
            &mut ack,
        );
        h.run_frames(1);
        assert!(
            !String::from_utf8_lossy(&ack).contains("error_code"),
            "{frame}: {}",
            String::from_utf8_lossy(&ack)
        );
        assert!(
            h.app.quit_coordinator.stop_reply.is_some(),
            "{frame}: stop was not served"
        );
    }
}

/// Codex 3290 r3 B6: an older short-lived client (here `pane rename`) that
/// exits before its credential is read reaches the host with no witness. Its
/// identity-independent request is still applied, as on alpha; only
/// protected requests need an identified caller.
#[test]
fn a_short_lived_clients_identity_independent_request_is_applied() {
    let mut h = HostHarness::new();
    let pane = h.add_focused_terminal();
    let mut ack = Vec::new();
    crate::app::handle_socket_line_in(
        &crate::config::config_dir(),
        &serde_json::json!({"type": "set_pane_title", "pane_id": pane, "name": "renamed-1285"})
            .to_string(),
        &h.ipc_tx,
        None,
        &mut ack,
    );
    h.run_frames(2);
    let name = h.app.terminal_incarnation(pane).and_then(|t| t.name.clone());
    assert_eq!(name.as_deref(), Some("renamed-1285"));
}
