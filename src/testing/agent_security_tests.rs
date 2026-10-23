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
    let mut ack = Vec::new();
    crate::app::handle_socket_line(&line, &h.ipc_tx, Some(ancestry), &mut ack);
    let deadline = std::time::Instant::now() + crate::testing::load_aware_timeout(std::time::Duration::from_secs(10));
    while !reply.is_file() {
        assert!(std::time::Instant::now() < deadline, "no secret_get reply");
        h.run_frames(1);
    }
    let body = std::fs::read_to_string(&reply).unwrap();
    let _ = std::fs::remove_file(&reply);
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
