//! Terminal authority across a host restart (stint 1158).
//!
//! Every test saves a live layout, starts a second harness on the same
//! profile, and reads the authority the restored terminals carry. Broker
//! effects are counted from the permission monitor, never inferred from flags.

use crate::host::pane::TerminalPane;
use crate::testing::HostHarness;

/// The host-sealed file that carries terminal authority beside the layout.
const LEDGER_FILE: &str = "terminal-authority.toml";

fn terminal(h: &HostHarness, pane: u64) -> &TerminalPane {
    h.app
        .windows
        .iter()
        .find_map(|window| window.panes.get(&pane))
        .and_then(|pane| pane.as_terminal())
        .expect("pane is a live terminal")
}

fn terminal_mut(h: &mut HostHarness, pane: u64) -> &mut TerminalPane {
    h.app
        .windows
        .iter_mut()
        .find_map(|window| window.panes.get_mut(&pane))
        .and_then(|pane| pane.as_terminal_mut())
        .expect("pane is a live terminal")
}

fn terminal_ids(h: &HostHarness) -> Vec<u64> {
    let mut ids: Vec<u64> = h
        .app
        .windows
        .iter()
        .flat_map(|window| window.panes.iter())
        .filter(|(_, pane)| pane.as_terminal().is_some())
        .map(|(id, _)| *id)
        .collect();
    ids.sort_unstable();
    ids
}

/// Mark a live terminal the way the host marks a plain shell an agent opened.
fn mark_agent_created(h: &mut HostHarness, pane: u64) {
    let term = terminal_mut(h, pane);
    term.agent_spawned = true;
    term.spawned_by = Some("agent:worker".to_string());
}

fn save(h: &mut HostHarness) -> std::path::PathBuf {
    assert!(h.app.save_workspace_now(), "workspace save must succeed");
    h.profile_dir().to_path_buf()
}

fn restart(h: &mut HostHarness) -> HostHarness {
    let profile = save(h);
    HostHarness::from_profile_dir(profile)
}

/// Ask the host to send text from `caller` to `target`. Returns the reply path.
fn send_between(h: &mut HostHarness, caller: u64, target: u64, tag: &str) -> String {
    let caller_pid = terminal(h, caller).backend.child_pid();
    let reply = crate::rpc::response_file(tag, "json");
    let request = crate::protocol::HostRequest::SendToPane {
        pane_id: target,
        text: "restored terminal message".into(),
        submit: false,
        paste: false,
        response_file: Some(reply.clone()),
        operation_id: None,
        generated_operation_id: false,
        peer_ancestry: None,
    };
    let line = serde_json::to_string(&request).unwrap();
    crate::app::handle_socket_line(&line, &h.ipc_tx, Some(&[caller_pid]), &mut Vec::new());
    h.hidden_frame();
    reply
}

fn pending_sends(h: &HostHarness) -> Vec<crate::broker::gate::PendingView> {
    crate::broker::gate::PermissionMonitor::for_profile(h.profile_dir())
        .list_pending()
        .into_iter()
        .filter(|row| row.tool == "pane.send")
        .collect()
}

fn restrictive_posture() -> crate::broker::PermissionPosture {
    crate::broker::PermissionPosture {
        default_posture: crate::broker::Decision::Deny,
        allow: Vec::new(),
        ask: vec!["secret.get".to_string()],
        deny: vec!["pane.send".to_string()],
    }
}

#[test]
fn desktop_terminal_keeps_its_identity_and_owner_after_restart() {
    let mut h = HostHarness::new();
    let pane = h.add_focused_terminal();
    assert!(
        h.app.is_human_owned_pane(pane),
        "fixture is a desktop terminal"
    );

    let restored = restart(&mut h);

    assert!(
        restored.app.find_pane_in_any_window(pane).is_some(),
        "a terminal whose sealed record matches the layout keeps its pane id"
    );
    assert!(
        restored.app.is_human_owned_pane(pane),
        "a desktop terminal with a sealed record is still the person's terminal"
    );
    assert_eq!(
        terminal(&restored, pane).spawned_by.as_deref(),
        Some("human")
    );
}

#[test]
fn agent_created_plain_terminal_is_not_human_owned_after_restart() {
    let mut h = HostHarness::new();
    let target = h.add_focused_terminal();
    let child = h.add_focused_terminal();
    mark_agent_created(&mut h, child);
    assert!(!h.app.is_human_owned_pane(child));

    let mut restored = restart(&mut h);

    assert!(
        restored.app.find_pane_in_any_window(child).is_some(),
        "a verified terminal keeps its pane id"
    );
    let term = terminal(&restored, child);
    assert!(
        term.agent_spawned,
        "a plain terminal an agent opened has no agent session; its lineage must still survive"
    );
    assert_eq!(term.spawned_by.as_deref(), Some("agent:worker"));
    assert_eq!(
        term.spawn_env
            .get("PLEXI_AGENT_SPAWNED")
            .map(String::as_str),
        Some("1"),
        "the restored shell must be built as a withheld-secrets pane"
    );
    assert!(!restored.app.is_human_owned_pane(child));

    // Real broker effect: the restored child cannot drive another pane ungated.
    let reply = send_between(&mut restored, child, target, "restored-child-send");
    assert!(
        !std::path::Path::new(&reply).exists(),
        "the send must park on an approval instead of being delivered"
    );
    let pending = pending_sends(&restored);
    assert_eq!(pending.len(), 1, "exactly one pane.send approval is filed");
    assert_eq!(pending[0].actor_id, format!("pane:{child}"));
}

#[test]
fn plain_terminal_restrictive_posture_survives_restart() {
    let mut h = HostHarness::new();
    let child = h.add_focused_terminal();
    mark_agent_created(&mut h, child);
    terminal_mut(&mut h, child).agent_permission_posture = Some(restrictive_posture());

    let restored = restart(&mut h);

    assert_eq!(
        restored.app.pane_permission_posture(child),
        Some(restrictive_posture()),
        "a posture with no agent session to ride on must still be restored"
    );
}

#[test]
fn agent_terminal_stays_restricted_when_session_restore_is_off() {
    let mut h = HostHarness::new();
    let pane = h.add_focused_terminal();
    {
        let term = terminal_mut(&mut h, pane);
        term.agent_harness = Some("pi".to_string());
        term.agent_spawned = true;
        term.spawned_by = Some("agent:worker".to_string());
        term.agent_permission_posture = Some(restrictive_posture());
    }
    let profile = save(&mut h);
    std::fs::write(
        profile.join("config.toml"),
        "[agents]\nrestore_mode = \"off\"\n",
    )
    .unwrap();

    let restored = HostHarness::from_profile_dir(profile);

    assert!(
        !restored.app.is_human_owned_pane(pane),
        "not resuming the agent must not turn its terminal into the person's terminal"
    );
    assert_eq!(
        restored.app.pane_permission_posture(pane),
        Some(restrictive_posture())
    );
}

/// The restored terminals when none of them could be verified: every one has
/// a new pane id, and none is the person's terminal.
fn assert_all_terminals_reissued_and_restricted(restored: &HostHarness, former: &[u64]) {
    let ids = terminal_ids(restored);
    assert_eq!(ids.len(), former.len(), "every terminal is still restored");
    for old in former {
        assert!(
            restored.app.find_pane_in_any_window(*old).is_none(),
            "an unverified terminal must not come back under pane id {old}"
        );
    }
    for id in ids {
        let term = terminal(restored, id);
        assert!(!restored.app.is_human_owned_pane(id));
        assert!(term.agent_spawned);
        assert_eq!(term.parent_pane_id, None);
        assert_eq!(term.spawned_by.as_deref(), Some("unverified"));
        assert_eq!(
            term.spawned_by_badge().as_deref(),
            Some("owner not verified")
        );
        assert_eq!(
            term.spawn_env
                .get("PLEXI_AGENT_SPAWNED")
                .map(String::as_str),
            Some("1")
        );
    }
}

#[test]
fn terminal_with_no_sealed_record_is_restored_as_a_new_restricted_pane() {
    let mut h = HostHarness::new();
    let pane = h.add_focused_terminal();
    let profile = save(&mut h);
    // A layout written before sealed records existed has no ledger beside it.
    match std::fs::remove_file(profile.join(LEDGER_FILE)) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => panic!("remove ledger: {error}"),
    }

    let restored = HostHarness::from_profile_dir(profile);

    assert_all_terminals_reissued_and_restricted(&restored, &[pane]);
}

#[test]
fn unsigned_record_cannot_claim_a_desktop_terminal() {
    let mut h = HostHarness::new();
    let child = h.add_focused_terminal();
    mark_agent_created(&mut h, child);
    let profile = save(&mut h);
    let layout = std::fs::read(crate::workspace::workspace_path_in(&profile)).unwrap();
    let digest = {
        use sha2::Digest;
        format!("{:x}", sha2::Sha256::digest(&layout))
    };
    std::fs::write(
        profile.join(LEDGER_FILE),
        format!(
            "schema = 1\nlayout_sha256 = \"{digest}\"\n\n[[terminals]]\npane_id = {child}\nopener = \"desktop\"\nspawned_by = \"human\"\n"
        ),
    )
    .unwrap();

    let restored = HostHarness::from_profile_dir(profile);

    assert_all_terminals_reissued_and_restricted(&restored, &[child]);
}

#[test]
fn altered_sealed_record_cannot_claim_a_desktop_terminal() {
    let mut h = HostHarness::new();
    let child = h.add_focused_terminal();
    mark_agent_created(&mut h, child);
    let profile = save(&mut h);
    let path = profile.join(LEDGER_FILE);
    let sealed = std::fs::read_to_string(&path).expect("save writes a sealed ledger");
    assert!(sealed.contains("opener = \"request\""));
    std::fs::write(
        &path,
        sealed.replace("opener = \"request\"", "opener = \"desktop\""),
    )
    .unwrap();

    let restored = HostHarness::from_profile_dir(profile);

    assert_all_terminals_reissued_and_restricted(&restored, &[child]);
}

#[test]
fn edited_layout_does_not_keep_the_sealed_owner() {
    let mut h = HostHarness::new();
    let pane = h.add_focused_terminal();
    assert!(h.app.is_human_owned_pane(pane));
    let profile = save(&mut h);
    // Point the person's terminal at a different folder without the host.
    let elsewhere = tempfile::tempdir().unwrap();
    let path = crate::workspace::workspace_path_in(&profile);
    let mut layout: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    for window in layout["windows"].as_array_mut().unwrap() {
        for saved in window["panes"].as_array_mut().unwrap() {
            if saved["id"] == pane {
                saved["cwd"] = serde_json::json!(elsewhere.path());
            }
        }
    }
    std::fs::write(&path, serde_json::to_vec_pretty(&layout).unwrap()).unwrap();

    let restored = HostHarness::from_profile_dir(profile);

    assert_all_terminals_reissued_and_restricted(&restored, &[pane]);
}

#[test]
fn earlier_sealed_record_does_not_apply_to_a_later_layout() {
    let mut h = HostHarness::new();
    let pane = h.add_focused_terminal();
    let profile = save(&mut h);
    let ledger = profile.join(LEDGER_FILE);
    let earlier = std::fs::read(&ledger).expect("save writes a sealed ledger");
    // The layout moves on: a second terminal appears and is saved.
    let later = h.add_focused_terminal();
    save(&mut h);
    std::fs::write(&ledger, earlier).unwrap();

    let restored = HostHarness::from_profile_dir(profile);

    assert_all_terminals_reissued_and_restricted(&restored, &[pane, later]);
}

#[test]
fn unreadable_seal_key_restores_terminals_restricted() {
    let mut h = HostHarness::new();
    let pane = h.add_focused_terminal();
    let profile = save(&mut h);

    let restored = {
        let _key = crate::broker::host_key::UnavailableHostKeyStore::acquire();
        HostHarness::from_profile_dir(profile)
    };

    assert_all_terminals_reissued_and_restricted(&restored, &[pane]);
}

#[test]
fn stored_grant_does_not_follow_an_unverified_terminal() {
    let mut h = HostHarness::new();
    let target = h.add_focused_terminal();
    let child = h.add_focused_terminal();
    mark_agent_created(&mut h, child);

    // The person approves this pane's sends for good.
    let first = send_between(&mut h, child, target, "grant-before-restart");
    let monitor = crate::broker::gate::PermissionMonitor::for_profile(h.profile_dir());
    let pending = pending_sends(&h);
    assert_eq!(pending.len(), 1);
    monitor
        .approve_pending(
            &pending[0].pending_request_id,
            crate::broker::gate::ApprovalChoice::Always,
        )
        .unwrap();
    h.hidden_frame();
    assert!(std::path::Path::new(&first).exists());
    assert!(pending_sends(&h).is_empty());

    let profile = save(&mut h);
    std::fs::remove_file(profile.join(LEDGER_FILE)).expect("save writes a sealed ledger");
    let mut restored = HostHarness::from_profile_dir(profile);

    let ids = terminal_ids(&restored);
    assert_eq!(ids.len(), 2);
    assert!(!ids.contains(&child) && !ids.contains(&target));
    for id in &ids {
        assert!(
            *id > child.max(target),
            "a reissued pane id is above every id the layout used"
        );
    }
    // Whichever restored terminal sends, the old Always grant does not answer.
    let reply = send_between(&mut restored, ids[1], ids[0], "grant-after-restart");
    assert!(
        !std::path::Path::new(&reply).exists(),
        "a stored grant for the former pane id must not admit the new pane"
    );
    let pending = pending_sends(&restored);
    assert_eq!(pending.len(), 1, "the send asks again");
    assert_eq!(pending[0].actor_id, format!("pane:{}", ids[1]));
}
