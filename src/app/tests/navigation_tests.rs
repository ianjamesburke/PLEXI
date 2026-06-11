use super::super::*;
use crate::host::context::Window;
use crate::testing::HostHarness;

fn second_window(context_id: u64, window_id: u64, pane_id: u64) -> Window {
    let mut tree = egui_tiles::Tree::empty("test_tree_2");
    let tile = tree.tiles.insert_pane(pane_id);
    tree.root = Some(tile);
    Window {
        name: "Context B".into(),
        path: std::env::temp_dir(),
        tree,
        panes: HashMap::new(),
        focused_pane: None,
        zoomed_pane: None,
        grid_x: 0,
        grid_y: 1,
        window_id,
        context_id,
    }
}

fn same_workspace_window_below(window_id: u64, pane_id: u64) -> Window {
    let mut tree = egui_tiles::Tree::empty("test_tree_below");
    let tile = tree.tiles.insert_pane(pane_id);
    tree.root = Some(tile);
    Window {
        name: "Test".into(),
        path: std::env::temp_dir(),
        tree,
        panes: HashMap::new(),
        focused_pane: None,
        zoomed_pane: None,
        grid_x: 0,
        grid_y: 1,
        window_id,
        context_id: 1, // same workspace as window 0
    }
}

fn same_workspace_window_bottom(window_id: u64, pane_id: u64) -> Window {
    let mut tree = egui_tiles::Tree::empty("test_tree_bottom");
    let tile = tree.tiles.insert_pane(pane_id);
    tree.root = Some(tile);
    Window {
        name: "Test".into(),
        path: std::env::temp_dir(),
        tree,
        panes: HashMap::new(),
        focused_pane: None,
        zoomed_pane: None,
        grid_x: 0,
        grid_y: 2,
        window_id,
        context_id: 1,
    }
}

/// Regression guard for #791: pane_navigate with a pane in the active
/// window must set focused_pane and return true.
#[test]
fn pane_navigate_same_window_sets_focused_pane() {
    let mut h = HostHarness::new();
    let pane_id = h.add_test_pane();
    assert!(h.app.pane_navigate(pane_id));
    assert_eq!(h.app.active_window, 0);
    assert!(h.app.windows[0].focused_pane.is_some());
}

/// Regression guard for #791: pane_navigate to a pane in window 1 must
/// update active_window to 1 (the missing piece before the fix).
#[test]
fn pane_navigate_cross_window_updates_active_window() {
    let mut h = HostHarness::new();
    let _pane_a = h.add_test_pane();
    h.app.windows.push(second_window(2, 2, 9901));
    h.app.router.push(crate::host::context::Context {
        name: "Context B".into(),
        path: std::env::temp_dir(),
        root: None,
        description: None,
        context_id: 2,
        parent_id: None,
        depth: 0,
        parked: false,
    });

    assert_eq!(h.app.active_window, 0);
    assert!(h.app.pane_navigate(9901));
    assert_eq!(
        h.app.active_window, 1,
        "active_window must switch to window 1"
    );
    assert!(
        h.app.windows[1].focused_pane.is_some(),
        "focused_pane must be set on target window"
    );
}

/// Regression guard for #823: pane_navigate must also sync router.active_idx
/// so the sidebar context switcher reflects the new active context immediately.
#[test]
fn pane_navigate_cross_window_syncs_router() {
    let mut h = HostHarness::new();
    let _pane_a = h.add_test_pane();
    h.app.windows.push(second_window(2, 2, 9902));
    h.app.router.push(crate::host::context::Context {
        name: "Context B".into(),
        path: std::env::temp_dir(),
        root: None,
        description: None,
        context_id: 2,
        parent_id: None,
        depth: 0,
        parked: false,
    });

    assert_eq!(h.app.router.active_idx(), 0);
    assert!(h.app.pane_navigate(9902));
    assert_eq!(
        h.app.router.active_idx(),
        1,
        "router must reflect new active context after pane_navigate"
    );
}

/// Regression guard for #878: `SendToPane` must search all windows, not just
/// `self.windows[self.active_window]`. Before the fix, a pane in window 1
/// returned "not found" when `active_window == 0`.
///
/// Strategy: insert an App pane into a second window, keep `active_window = 0`,
/// inject `SendToPane` targeting that pane. The response must contain
/// "not a terminal pane" (pane was found across windows but is an App pane),
/// NOT "not found" (which would indicate the pre-fix single-window search).
#[test]
fn send_to_pane_searches_all_windows() {
    let mut h = HostHarness::new();
    // Window 0 exists with a test app pane (from HostHarness::new via add_test_pane).
    // We need a second window with a known pane id.
    let cross_window_pane_id: u64 = 9978;
    let mut win1 = second_window(2, 2, cross_window_pane_id);
    // Insert an App pane (not Terminal) so we can confirm lookup reaches it.
    let app_pane = {
        use crate::app::permissions::AppPermissions;
        use crate::host::pane::{AppPane, AppRuntime};
        use crate::process_app::ProcessApp;
        let (process_app, _draw_tx) =
            ProcessApp::new_for_test(cross_window_pane_id, AppPermissions::builtin());
        AppPane {
            id: cross_window_pane_id,
            runtime: AppRuntime::Process(Box::new(process_app)),
            workspace_root: std::env::temp_dir(),
            permissions: AppPermissions::builtin(),
            manifest_id: "test".to_string(),
            name: "Test App Win1".to_string(),
            pane_group: None,
            linked_pane_id: None,
            overlay_replaced: None,
            hidden: false,
            agent: None,
            slots: std::collections::HashMap::new(),
        }
    };
    win1.panes.insert(
        cross_window_pane_id,
        crate::host::pane::Pane::App(Box::new(app_pane)),
    );
    h.app.windows.push(win1);
    h.app.router.push(crate::host::context::Context {
        name: "Context B".into(),
        path: std::env::temp_dir(),
        root: None,
        description: None,
        context_id: 2,
        parent_id: None,
        depth: 0,
        parked: false,
    });

    // active_window remains 0 — pane is in window 1.
    assert_eq!(h.app.active_window, 0);

    let resp_file = std::env::temp_dir().join("plexi_test_send_878.json");
    h.inject_ipc(crate::app_protocol::AppRequest::SendToPane {
        pane_id: cross_window_pane_id,
        text: "hello".to_string(),
        response_file: Some(resp_file.to_string_lossy().to_string()),
    });

    // drain_pane_cmd_channel processes the IPC queue synchronously.
    h.app.drain_pane_cmd_channel();

    let response = std::fs::read_to_string(&resp_file)
        .expect("response file must be written by SendToPane handler");
    // Pre-fix: response contains "not found" (pane not visible from active window).
    // Post-fix: pane IS found across windows but is an App pane → "is not a terminal pane".
    // The distinct error messages let us confirm the cross-window search reached the pane.
    assert!(
        response.contains("is not a terminal pane"),
        "expected cross-window lookup to find the pane (error contains 'is not a terminal pane'), \
         got: {response}. If 'not found', the single-window regression is back."
    );
    let _ = std::fs::remove_file(&resp_file);
}

/// Regression guard for #996: `pane list` must only return pane_ids that
/// have a corresponding tile in the tree, so every id it emits is navigable
/// via `pane_navigate`. When win.panes and the tile tree are out of sync,
/// the orphaned entry must be omitted rather than surfaced as a broken id.
#[test]
fn pane_list_excludes_orphaned_panes_and_navigate_succeeds() {
    let mut h = HostHarness::new();
    let real_pane_id = h.add_test_pane();

    // Artificially create the desync: insert a pane into win.panes without
    // a corresponding tile in the tree. This simulates corrupted restore state
    // or any create-path bug that leaves win.panes ahead of the tile tree.
    let orphan_id: PaneId = 99991;
    let (orphan_process, _tx) = crate::process_app::ProcessApp::new_for_test(
        orphan_id,
        crate::app::permissions::AppPermissions::builtin(),
    );
    let orphan_pane = crate::host::pane::Pane::App(Box::new(crate::host::pane::AppPane {
        id: orphan_id,
        runtime: crate::host::pane::AppRuntime::Process(Box::new(orphan_process)),
        workspace_root: std::env::temp_dir(),
        permissions: crate::app::permissions::AppPermissions::builtin(),
        manifest_id: "orphan".to_string(),
        name: "Orphan".to_string(),
        pane_group: None,
        linked_pane_id: None,
        overlay_replaced: None,
        hidden: false,
        agent: None,
        slots: std::collections::HashMap::new(),
    }));
    h.app.windows[0].panes.insert(orphan_id, orphan_pane);
    assert!(
        h.app.windows[0].tree.tiles.find_pane(&orphan_id).is_none(),
        "orphan has no tile"
    );

    let resp_file = std::env::temp_dir().join("plexi_test_pane_list_996.json");
    h.inject_ipc(crate::app_protocol::AppRequest::ListPanes {
        response_file: resp_file.to_string_lossy().to_string(),
        context_id: None,
    });
    h.app.drain_pane_cmd_channel();

    let json = std::fs::read_to_string(&resp_file).expect("ListPanes must write response file");
    let panes: Vec<serde_json::Value> = serde_json::from_str(&json).expect("valid JSON");
    let ids: Vec<u64> = panes.iter().filter_map(|p| p["id"].as_u64()).collect();

    assert!(
        ids.contains(&real_pane_id),
        "real pane must appear in pane_list"
    );
    assert!(
        !ids.contains(&orphan_id),
        "orphaned pane (no tile) must NOT appear in pane_list"
    );

    for id in &ids {
        assert!(
            h.app.pane_navigate(*id),
            "pane_navigate must succeed for every id returned by pane_list (failed for {id})"
        );
    }

    let _ = std::fs::remove_file(&resp_file);
}

#[test]
fn pane_info_and_list_include_agent_state() {
    let mut h = HostHarness::new();
    let pane_id = h.add_test_pane();

    h.inject_ipc(crate::app_protocol::AppRequest::SetAgentState {
        pane_id,
        state: crate::app_protocol::AgentState::Working,
        agent: "claude-code".to_string(),
        detail: Some("Bash: cargo test".to_string()),
        session_id: Some("session-33".to_string()),
    });
    h.app.drain_pane_cmd_channel();

    let info_file = std::env::temp_dir().join("plexi_test_pane_info_agent_2119.json");
    h.inject_ipc(crate::app_protocol::AppRequest::GetPaneInfo {
        pane_id,
        response_file: info_file.to_string_lossy().to_string(),
    });
    h.app.drain_pane_cmd_channel();

    let info_json =
        std::fs::read_to_string(&info_file).expect("GetPaneInfo must write response file");
    let info: serde_json::Value = serde_json::from_str(&info_json).expect("valid pane info JSON");
    assert_eq!(info["agent"]["pane_id"], pane_id);
    assert_eq!(info["agent"]["state"], "working");
    assert_eq!(info["agent"]["agent"], "claude-code");
    assert_eq!(info["agent"]["detail"], "Bash: cargo test");
    assert_eq!(info["agent"]["session_id"], "session-33");

    let list_file = std::env::temp_dir().join("plexi_test_pane_list_agent_2119.json");
    h.inject_ipc(crate::app_protocol::AppRequest::ListPanes {
        response_file: list_file.to_string_lossy().to_string(),
        context_id: None,
    });
    h.app.drain_pane_cmd_channel();

    let list_json =
        std::fs::read_to_string(&list_file).expect("ListPanes must write response file");
    let panes: Vec<serde_json::Value> = serde_json::from_str(&list_json).expect("valid JSON");
    let pane = panes
        .iter()
        .find(|pane| pane["id"].as_u64() == Some(pane_id))
        .expect("pane must be present in pane list");
    assert_eq!(pane["agent"]["pane_id"], pane_id);
    assert_eq!(pane["agent"]["state"], "working");
    assert_eq!(pane["agent"]["agent"], "claude-code");
    assert_eq!(pane["agent"]["detail"], "Bash: cargo test");
    assert_eq!(pane["agent"]["session_id"], "session-33");

    let _ = std::fs::remove_file(&info_file);
    let _ = std::fs::remove_file(&list_file);
}

#[test]
fn get_agent_states_collects_state_from_panes() {
    let mut h = HostHarness::new();
    let pane_id = h.add_test_pane();

    h.inject_ipc(crate::app_protocol::AppRequest::SetAgentState {
        pane_id,
        state: crate::app_protocol::AgentState::Blocked,
        agent: "claude-code".to_string(),
        detail: None,
        session_id: None,
    });
    h.app.drain_pane_cmd_channel();

    let states_file = std::env::temp_dir().join("plexi_test_agent_states_2119.json");
    h.inject_ipc(crate::app_protocol::AppRequest::GetAgentStates {
        response_file: states_file.to_string_lossy().to_string(),
    });
    h.app.drain_pane_cmd_channel();

    let states_json =
        std::fs::read_to_string(&states_file).expect("GetAgentStates must write response file");
    let states: Vec<serde_json::Value> = serde_json::from_str(&states_json).expect("valid JSON");
    assert_eq!(states.len(), 1);
    assert_eq!(states[0]["pane_id"], pane_id);
    assert_eq!(states[0]["state"], "blocked");
    assert_eq!(states[0]["agent"], "claude-code");
    assert!(states[0]["detail"].is_null());
    assert!(states[0]["session_id"].is_null());

    let _ = std::fs::remove_file(&states_file);
}

#[test]
fn overlay_panes_preserve_replaced_pane_agent_state() {
    let mut h = HostHarness::new();
    let pane_id = h.add_test_pane();

    h.inject_ipc(crate::app_protocol::AppRequest::SetAgentState {
        pane_id,
        state: crate::app_protocol::AgentState::Working,
        agent: "claude-code".to_string(),
        detail: Some("Edit: main.rs".to_string()),
        session_id: Some("overlay-session".to_string()),
    });
    h.app.drain_pane_cmd_channel();

    let replaced = h.app.windows[0]
        .panes
        .remove(&pane_id)
        .expect("test pane exists before overlay");
    let (overlay_app, _draw_tx) = crate::process_app::ProcessApp::new_for_test(
        pane_id,
        crate::app::permissions::AppPermissions::builtin(),
    );
    h.app.windows[0].panes.insert(
        pane_id,
        crate::host::pane::Pane::App(Box::new(crate::host::pane::AppPane {
            id: pane_id,
            runtime: crate::host::pane::AppRuntime::Process(Box::new(overlay_app)),
            workspace_root: std::env::temp_dir(),
            permissions: crate::app::permissions::AppPermissions::builtin(),
            manifest_id: "overlay".to_string(),
            name: "Overlay".to_string(),
            pane_group: None,
            linked_pane_id: None,
            overlay_replaced: Some(Box::new(replaced)),
            hidden: false,
            agent: None,
            slots: std::collections::HashMap::new(),
        })),
    );

    let states_file = std::env::temp_dir().join("plexi_test_overlay_agent_states_2119.json");
    h.inject_ipc(crate::app_protocol::AppRequest::GetAgentStates {
        response_file: states_file.to_string_lossy().to_string(),
    });
    h.app.drain_pane_cmd_channel();

    let states_json =
        std::fs::read_to_string(&states_file).expect("GetAgentStates must write response file");
    let states: Vec<serde_json::Value> = serde_json::from_str(&states_json).expect("valid JSON");
    assert_eq!(states.len(), 1);
    assert_eq!(states[0]["pane_id"], pane_id);
    assert_eq!(states[0]["state"], "working");
    assert_eq!(states[0]["detail"], "Edit: main.rs");
    assert_eq!(states[0]["session_id"], "overlay-session");

    let info_file = std::env::temp_dir().join("plexi_test_overlay_pane_info_agent_2119.json");
    h.inject_ipc(crate::app_protocol::AppRequest::GetPaneInfo {
        pane_id,
        response_file: info_file.to_string_lossy().to_string(),
    });
    h.app.drain_pane_cmd_channel();

    let info_json =
        std::fs::read_to_string(&info_file).expect("GetPaneInfo must write response file");
    let info: serde_json::Value = serde_json::from_str(&info_json).expect("valid pane info JSON");
    assert_eq!(info["agent"]["pane_id"], pane_id);
    assert_eq!(info["agent"]["state"], "working");
    assert_eq!(info["agent"]["detail"], "Edit: main.rs");
    assert_eq!(info["agent"]["session_id"], "overlay-session");

    let _ = std::fs::remove_file(&states_file);
    let _ = std::fs::remove_file(&info_file);
}

/// #1074: navigate(Down) at the vertical pane boundary jumps to the LAST
/// window in the workspace list — skipping intermediate windows entirely.
#[test]
fn navigate_down_at_vertical_boundary_jumps_to_last_window() {
    let mut h = HostHarness::new();
    let pane_a = h.add_test_pane();
    // Three windows: grid_y 0 (window 0), 1 (window 1), 2 (window 2).
    h.app.windows.push(same_workspace_window_below(2, 9910)); // grid_y=1
    h.app.windows.push(same_workspace_window_bottom(3, 9911)); // grid_y=2

    assert!(
        h.app.pane_navigate(pane_a),
        "pane_navigate must succeed to set up focus"
    );
    assert_eq!(h.app.active_window, 0);

    // Down from window 0 must jump directly to the LAST window (grid_y=2), not step to grid_y=1.
    h.app.navigate(crate::host::keys::Direction::Down);
    assert_eq!(
        h.app.active_window, 2,
        "navigate(Down) at vertical boundary must jump to last window"
    );
}

/// #1074: navigate(Up) at the vertical pane boundary jumps to the FIRST
/// window in the workspace list.
#[test]
fn navigate_up_at_vertical_boundary_jumps_to_first_window() {
    let mut h = HostHarness::new();
    let _pane_a = h.add_test_pane();
    h.app.windows.push(same_workspace_window_below(2, 9910)); // grid_y=1
    h.app.windows.push(same_workspace_window_bottom(3, 9911)); // grid_y=2

    // Start from the middle window.
    assert_eq!(h.app.active_window, 0);
    h.app.active_window = 1;

    // Up from window 1 must jump to the FIRST window (grid_y=0).
    h.app.navigate(crate::host::keys::Direction::Up);
    assert_eq!(
        h.app.active_window, 0,
        "navigate(Up) at vertical boundary must jump to first window"
    );
}

/// Single-window workspace: navigate(Down) at boundary is a no-op —
/// the only window is both first and last.
#[test]
fn navigate_down_single_window_is_noop() {
    let mut h = HostHarness::new();
    let pane_a = h.add_test_pane();
    assert!(h.app.pane_navigate(pane_a), "pane_navigate must succeed");
    assert_eq!(h.app.active_window, 0);
    h.app.navigate(crate::host::keys::Direction::Down);
    assert_eq!(
        h.app.active_window, 0,
        "navigate(Down) in single-window workspace must not change active_window"
    );
}

/// Horizontal boundary (Left/Right) still falls through to page navigation unchanged.
#[test]
fn navigate_left_at_horizontal_boundary_still_page_navigates() {
    let mut h = HostHarness::new();
    let pane_a = h.add_test_pane();
    assert!(h.app.pane_navigate(pane_a), "pane_navigate must succeed");
    assert_eq!(h.app.active_window, 0);
    // No window to the left of (0,0) → Left is a no-op (not wrapped).
    h.app.navigate(crate::host::keys::Direction::Left);
    assert_eq!(
        h.app.active_window, 0,
        "Left at boundary must not change active_window"
    );
}

/// GetPreviousPaneInfo returns the last live entry from pane_focus_history.
#[test]
fn get_previous_pane_info_returns_previous_pane() {
    let mut h = HostHarness::new();
    let pane_a = h.add_test_pane();
    let pane_b = h.add_test_pane();

    // Simulate: user was in pane_a, moved to pane_b (pushes pane_a's tile into history).
    let tile_a = h.app.windows[0]
        .tree
        .tiles
        .find_pane(&pane_a)
        .expect("tile_a must exist");
    let window_id = h.app.windows[0].window_id;
    h.app.pane_focus_history.push((window_id, tile_a));

    let resp_file = std::env::temp_dir().join("plexi_test_prev_pane_info.json");
    let _ = std::fs::remove_file(&resp_file);

    h.inject_ipc(crate::app_protocol::AppRequest::GetPreviousPaneInfo {
        response_file: resp_file.to_string_lossy().to_string(),
    });
    h.app.drain_pane_cmd_channel();

    let json = std::fs::read_to_string(&resp_file).expect("response file must be written");
    let v: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
    assert!(v.get("error").is_none(), "unexpected error: {json}");
    assert_eq!(
        v["id"].as_u64(),
        Some(pane_a),
        "previous pane must be pane_a, not pane_b; got: {json}"
    );
    let _ = std::fs::remove_file(&resp_file);
    let _ = pane_b;
}

/// GetPreviousPaneInfo skips stale tile entries (tile no longer in tree) and
/// falls through to the next valid entry.
#[test]
fn get_previous_pane_info_skips_stale_tile() {
    let mut h = HostHarness::new();
    let pane_a = h.add_test_pane();
    let window_id = h.app.windows[0].window_id;

    // Push a stale (nonexistent) tile first, then a valid one.
    let stale_tile = egui_tiles::TileId::from_u64(99999);
    let tile_a = h.app.windows[0]
        .tree
        .tiles
        .find_pane(&pane_a)
        .expect("tile_a must exist");
    h.app.pane_focus_history.push((window_id, tile_a)); // older: pane_a
    h.app.pane_focus_history.push((window_id, stale_tile)); // newer (but stale)

    let resp_file = std::env::temp_dir().join("plexi_test_prev_pane_stale.json");
    let _ = std::fs::remove_file(&resp_file);

    h.inject_ipc(crate::app_protocol::AppRequest::GetPreviousPaneInfo {
        response_file: resp_file.to_string_lossy().to_string(),
    });
    h.app.drain_pane_cmd_channel();

    let json = std::fs::read_to_string(&resp_file).expect("response file must be written");
    let v: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
    assert!(
        v.get("error").is_none(),
        "unexpected error after stale skip: {json}"
    );
    assert_eq!(
        v["id"].as_u64(),
        Some(pane_a),
        "must fall through to pane_a after skipping stale tile; got: {json}"
    );
    let _ = std::fs::remove_file(&resp_file);
}

/// GetPreviousPaneInfo returns an error JSON when history is empty.
#[test]
fn get_previous_pane_info_empty_history_returns_error() {
    let mut h = HostHarness::new();
    let _pane_a = h.add_test_pane();
    assert!(
        h.app.pane_focus_history.is_empty(),
        "history must start empty"
    );

    let resp_file = std::env::temp_dir().join("plexi_test_prev_pane_empty.json");
    let _ = std::fs::remove_file(&resp_file);

    h.inject_ipc(crate::app_protocol::AppRequest::GetPreviousPaneInfo {
        response_file: resp_file.to_string_lossy().to_string(),
    });
    h.app.drain_pane_cmd_channel();

    let json =
        std::fs::read_to_string(&resp_file).expect("response file must be written even on error");
    let v: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
    assert!(
        v.get("error").is_some(),
        "expected error field in response; got: {json}"
    );
    let _ = std::fs::remove_file(&resp_file);
}
