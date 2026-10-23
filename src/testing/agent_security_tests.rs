//! Stint 0788: secrets stay out of agent panes, and a pane's `secret get`
//! waits for a desktop click.

use std::path::{Path, PathBuf};

use crate::broker::gate::{ApprovalChoice, PermissionMonitor};
use crate::protocol::AppRequest;
use crate::testing::HostHarness;
use crate::workspace::secrets::NonDestructiveStore;

fn monitor() -> std::sync::Arc<PermissionMonitor> {
    PermissionMonitor::for_profile(&crate::config::config_dir())
}

fn write_workspace(root: &Path, workspace_id: &str, inject: &[&str]) {
    let dir = root.join(crate::config::workspace_channel_dir());
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("workspace.toml"), format!("id = \"{workspace_id}\"\n")).unwrap();
    let names = inject
        .iter()
        .map(|name| format!("\"{name}\""))
        .collect::<Vec<_>>()
        .join(", ");
    std::fs::write(
        dir.join("secrets.toml"),
        format!("fallback = true\n\n[terminal.env]\ninject = [{names}]\n"),
    )
    .unwrap();
}

/// One `secret_get` over the socket, stamped with `ancestry` the way the
/// listener stamps a real peer.
fn secret_get(h: &mut HostHarness, name: &str, root: &Path, ancestry: &[u32]) -> serde_json::Value {
    let reply = PathBuf::from(crate::rpc::response_file("secret-get-test", "json"));
    let request = AppRequest::PaneSecretGet {
        name: name.to_string(),
        global: false,
        workspace_root: Some(root.to_path_buf()),
        peer_ancestry: Vec::new(),
        response_file: reply.to_string_lossy().into_owned(),
    };
    let line = serde_json::to_string(&request).unwrap();
    let (mut server_stream, client_stream) = std::os::unix::net::UnixStream::pair().unwrap();
    let token = reply.to_string_lossy().into_owned();
    crate::rpc::register_client_stream(token.clone(), client_stream);
    crate::app::handle_socket_line(&line, &h.ipc_tx, Some(ancestry), &mut server_stream);
    drop(server_stream);
    h.run_frames(1);
    let body = crate::rpc::poll_string(&token, Some(std::time::Duration::from_secs(2))).unwrap();
    assert!(!reply.exists(), "secret_get must not write a response file to disk");
    serde_json::from_str(&body).unwrap()
}

#[test]
fn secret_get_from_a_pane_files_a_click_approval_not_a_value() {
    let mut h = HostHarness::new();
    let term = h.add_focused_terminal();
    let pid = h.terminal_backend(term).child_pid();
    let root = h.workspace_root();
    write_workspace(&root, "ws-0788-get", &[]);
    let canary = "secret-get-canary-0788-a1";
    crate::workspace::secrets::system_store()
        .set("plexi:ws-0788-get:API_KEY_0788", canary)
        .unwrap();

    let first = secret_get(&mut h, "API_KEY_0788", &root, &[pid]);
    assert_eq!(first["status"], "permission_required", "{first}");
    assert!(!first.to_string().contains(canary), "{first}");
    let pending_id = first["pending_request_id"].as_str().expect("pending id").to_string();
    let pending = monitor().show_pending(&pending_id).expect("banner sees the pending");
    assert_eq!(pending.actor_id, format!("pane:{term}"));
    assert_eq!(pending.tool, crate::workspace::secrets::SECRET_GET_TOOL);

    // The desktop banner's Allow once is `approve_pending(Once)`.
    monitor().approve_pending(&pending_id, ApprovalChoice::Once).unwrap();
    let granted = secret_get(&mut h, "API_KEY_0788", &root, &[pid]);
    assert_eq!(granted["status"], "value", "{granted}");
    assert_eq!(granted["value"], canary);

    let again = secret_get(&mut h, "API_KEY_0788", &root, &[pid]);
    assert_eq!(again["status"], "permission_required", "Allow once is spent: {again}");
}

#[test]
fn secret_get_without_a_host_stamped_pane_is_refused() {
    let mut h = HostHarness::new();
    let root = h.workspace_root();
    write_workspace(&root, "ws-0788-nopane", &[]);
    let canary = "secret-get-canary-0788-b2";
    crate::workspace::secrets::system_store()
        .set("plexi:ws-0788-nopane:API_KEY_0788", canary)
        .unwrap();
    let before = monitor().list_pending().len();

    let reply = secret_get(&mut h, "API_KEY_0788", &root, &[]);
    assert_eq!(reply["status"], "denied", "{reply}");
    assert!(!reply.to_string().contains(canary));
    assert_eq!(monitor().list_pending().len(), before, "no pending without an identity");
}

#[test]
fn agent_pane_env_carries_no_workspace_or_folder_secrets() {
    let h = HostHarness::new();
    let _enabled = crate::release::set_test_experiments(&[crate::release::ReleaseFeature::FolderSecrets]);
    let root = h.workspace_root();
    write_workspace(&root, "ws-0788-env", &["INJECTED_0788"]);
    crate::workspace::secrets::system_store()
        .set("plexi:ws-0788-env:INJECTED_0788", "workspace-env-canary-0788")
        .unwrap();
    crate::workspace::secrets::set_folder_secret(
        crate::workspace::secrets::folder_store(),
        "FOLDER_0788",
        &root,
        "folder-env-canary-0788",
    )
    .unwrap();

    let settings = |agent_pane: bool| {
        let context_env = crate::pane_ops::create::PaneContextEnv {
            context_id: 1,
            name: "ctx".to_string(),
            description: String::new(),
            root: Some(root.clone()),
            depth: 0,
        };
        crate::app::PlexiApp::make_backend_settings(
            7,
            Some(root.clone()),
            &h.app.colors,
            &context_env,
            agent_pane,
        )
        .0
        .env
    };
    let human = settings(false);
    assert_eq!(human.get("INJECTED_0788").map(String::as_str), Some("workspace-env-canary-0788"));
    assert_eq!(human.get("FOLDER_0788").map(String::as_str), Some("folder-env-canary-0788"));

    let agent = settings(true);
    for (key, value) in &agent {
        assert!(
            !value.contains("env-canary-0788"),
            "agent pane env {key} carries a secret value"
        );
    }
    assert!(!agent.contains_key("PLEXI_TERMINAL_ENV_NAMES"));
}

#[test]
fn cli_secret_set_does_not_enable_terminal_injection() {
    let dir = tempfile::tempdir().unwrap();
    let _profile = crate::config::set_test_profile_dir(dir.path().join("profile"));
    let root = dir.path().join("ws");
    write_workspace(&root, "ws-0788-set", &["ALREADY_0788"]);
    let store = crate::workspace::secrets::InMemoryKeychain::new();

    let injected = crate::cli::workspace::store_workspace_secret(
        &store,
        &root,
        "ws-0788-set",
        "NEW_0788",
        "NEW_0788",
        "set-canary-0788",
    )
    .unwrap();

    assert_eq!(store.get("plexi:ws-0788-set:NEW_0788").as_deref().map(String::as_str), Some("set-canary-0788"));
    let router = crate::workspace::secrets::WorkspaceSecrets::load(&root).unwrap().unwrap();
    assert_eq!(router.terminal.env.inject, vec!["ALREADY_0788".to_string()]);
    assert_eq!(injected, vec!["ALREADY_0788".to_string()], "existing injected names are surfaced");
}

fn spawn_second_terminal(h: &mut HostHarness, beside: u64) -> u64 {
    let (_, tile) = h.app.find_pane_in_any_window(beside).unwrap();
    h.app.spawn_terminal_pane_at(crate::pane_ops::create::SpawnTerminalAtSpec {
        win_idx: 0,
        target_tile: tile,
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
    })
}

fn socket_request(
    h: &mut HostHarness,
    req: &AppRequest,
    ancestry: &[u32],
    token: &Path,
) -> serde_json::Value {
    let line = serde_json::to_string(req).unwrap();
    let (mut server_stream, client_stream) = std::os::unix::net::UnixStream::pair().unwrap();
    let token = token.to_string_lossy().into_owned();
    crate::rpc::register_client_stream(token.clone(), client_stream);
    crate::app::handle_socket_line(&line, &h.ipc_tx, Some(ancestry), &mut server_stream);
    drop(server_stream);
    h.run_frames(1);
    let body = crate::rpc::poll_string(&token, Some(std::time::Duration::from_secs(2))).unwrap();
    serde_json::from_str(&body).unwrap()
}

#[test]
fn agent_done_and_ask_from_one_pane_cannot_act_as_another_pane() {
    let mut h = HostHarness::new();
    let attacker = h.add_focused_terminal();
    let victim = spawn_second_terminal(&mut h, attacker);
    let attacker_pid = h.terminal_backend(attacker).child_pid();
    let root = h.workspace_root();

    let done = AppRequest::AgentDone {
        pane_id: victim,
        result: Some("{\"forged\":true}".into()),
        summary: Some("forged".into()),
        peer_ancestry: None,
        response_file: root.join("forged-done.json").to_string_lossy().into_owned(),
    };
    let reply = socket_request(&mut h, &done, &[attacker_pid], &root.join("forged-done.json"));
    assert_eq!(reply["ok"], false, "{reply}");
    assert_eq!(reply["error_code"], "permission_denied", "{reply}");
    assert!(
        h.app.pane_results.get(&victim).is_none_or(|r| r.committed.is_none()),
        "a refused agent done must not commit a result for the victim"
    );

    let ask = AppRequest::AgentAsk {
        pane_id: victim,
        question: "forged question".into(),
        peer_ancestry: None,
        response_file: root.join("forged-ask.json").to_string_lossy().into_owned(),
    };
    let reply = socket_request(&mut h, &ask, &[attacker_pid], &root.join("forged-ask.json"));
    assert_eq!(reply["error_code"], "permission_denied", "{reply}");
    assert!(
        h.app.agent_inputs().iter().all(|a| a.pane_id != victim),
        "a refused agent ask must not mark the victim blocked"
    );

    // The same peer acting as itself is accepted.
    let own = AppRequest::AgentDone {
        pane_id: attacker,
        result: Some("{\"ok\":1}".into()),
        summary: Some("mine".into()),
        peer_ancestry: None,
        response_file: root.join("own-done.json").to_string_lossy().into_owned(),
    };
    let reply = socket_request(&mut h, &own, &[attacker_pid], &root.join("own-done.json"));
    assert_eq!(reply["ok"], true, "{reply}");
    assert!(h.app.pane_results.get(&attacker).is_some_and(|r| r.committed.is_some()));

    // A caller outside every pane (the human at a plain terminal) keeps working.
    let outside = AppRequest::AgentDone {
        pane_id: victim,
        result: Some("{\"by\":\"human\"}".into()),
        summary: None,
        peer_ancestry: None,
        response_file: root.join("human-done.json").to_string_lossy().into_owned(),
    };
    let reply = socket_request(&mut h, &outside, &[999_999], &root.join("human-done.json"));
    assert_eq!(reply["ok"], true, "{reply}");
}

#[test]
fn set_agent_state_from_one_pane_cannot_change_another_pane() {
    let mut h = HostHarness::new();
    let attacker = h.add_focused_terminal();
    let victim = spawn_second_terminal(&mut h, attacker);
    let attacker_pid = h.terminal_backend(attacker).child_pid();
    let report = |pane_id| AppRequest::SetAgentState {
        pane_id,
        state: crate::protocol::AgentState::Failed,
        agent: "claude-code".into(),
        detail: None,
        session_id: None,
        event: None,
        blocked_reason: None,
        peer_ancestry: None,
    };
    for req in [report(victim), report(attacker)] {
        let line = serde_json::to_string(&req).unwrap();
        let mut ack = Vec::new();
        crate::app::handle_socket_line(&line, &h.ipc_tx, Some(&[attacker_pid]), &mut ack);
    }
    h.run_frames(2);
    let inputs = h.app.agent_inputs();
    assert!(inputs.iter().all(|a| a.pane_id != victim), "victim state was forged");
    assert!(inputs.iter().any(|a| a.pane_id == attacker), "own report must land");
}

#[test]
fn permissions_caller_is_derived_from_the_peer_not_the_wire() {
    let mut h = HostHarness::new();
    let pane = h.add_focused_terminal();
    let pid = h.terminal_backend(pane).child_pid();
    // A pane process that claims no pane (unset env) is still that pane's agent.
    assert_eq!(h.app.permissions_caller_pane(Some(&[pid]), None), Some(pane));
    // A wire claim of another pane cannot override the verified peer.
    assert_eq!(h.app.permissions_caller_pane(Some(&[pid]), Some(pane + 100)), Some(pane));
    // A caller outside every pane stays outside; its claim can only lower authority.
    assert_eq!(h.app.permissions_caller_pane(Some(&[999_999]), None), None);
    assert_eq!(h.app.permissions_caller_pane(Some(&[999_999]), Some(7)), Some(7));
}

#[test]
fn caller_context_is_answered_from_the_verified_peer() {
    let mut h = HostHarness::new();
    let pane = h.add_focused_terminal();
    let pid = h.terminal_backend(pane).child_pid();
    let root = h.workspace_root();
    let ask = |h: &mut HostHarness, ancestry: &[u32], name: &str| {
        let token = root.join(name);
        let req = AppRequest::CallerContext {
            peer_ancestry: None,
            response_file: token.to_string_lossy().into_owned(),
        };
        socket_request(h, &req, ancestry, &token)
    };
    let inside = ask(&mut h, &[pid], "ctx-in.json");
    assert_eq!(inside["ok"], true, "{inside}");
    assert_eq!(inside["pane_id"], pane, "{inside}");
    assert!(inside["context_root"].is_string(), "pane has a live context root: {inside}");
    let outside = ask(&mut h, &[999_999], "ctx-out.json");
    assert_eq!(outside["ok"], true, "{outside}");
    assert!(outside["pane_id"].is_null() && outside["context_root"].is_null(), "{outside}");
}

/// 0961 follow-up: a pane from `plexi agent spawn` carries its own routing env,
/// so `agent done` needs no manual `PLEXI_SOCKET` export, and the host accepts
/// the done from that pane's own process.
#[test]
fn agent_done_works_from_a_spawned_pane_without_a_manual_socket_export() {
    let mut h = HostHarness::new();
    let root = h.workspace_root();
    h.app.set_context_root(root.clone(), None);
    let (pane, _) = h
        .app
        .spawn_agent_worker_pane(
            "researcher", "pi", "sleep 30", "Audit auth", None, false, None, None, None, None,
            false, None,
        )
        .unwrap();
    let (pid, env) = {
        let (win, _) = h.app.find_pane_in_any_window(pane).unwrap();
        let term = h.app.windows[win].panes.get(&pane).unwrap().as_terminal().unwrap();
        (term.backend.child_pid(), term.spawn_env.clone())
    };
    assert_eq!(
        env.get("PLEXI_SOCKET").map(String::as_str),
        Some(crate::platform::ipc::endpoint().as_str()),
        "spawned pane must inherit the host socket"
    );
    assert_eq!(env.get("PLEXI_PANE_ID"), Some(&pane.to_string()));

    let reply_path = root.join("spawned-done.json");
    let done = AppRequest::AgentDone {
        pane_id: pane,
        result: Some("{\"answer\":42}".into()),
        summary: Some("done".into()),
        peer_ancestry: None,
        response_file: reply_path.to_string_lossy().into_owned(),
    };
    let reply = socket_request(&mut h, &done, &[pid], &reply_path);
    assert_eq!(reply["ok"], true, "{reply}");
    assert!(h.app.pane_results.get(&pane).is_some_and(|r| r.committed.is_some()));
}

#[test]
fn plain_panes_are_stamped_spawned_by_human_with_a_badge() {
    let mut h = HostHarness::new();
    let pane = h.add_focused_terminal();
    let (win, _) = h.app.find_pane_in_any_window(pane).unwrap();
    let term = h.app.windows[win].panes.get(&pane).unwrap().as_terminal().unwrap();
    assert_eq!(term.spawned_by.as_deref(), Some("human"));
    assert_eq!(term.spawned_by_badge().as_deref(), Some("spawned by you"));
}
