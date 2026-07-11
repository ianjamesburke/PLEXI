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
/// inject `SendToPane` targeting that pane. App panes accept text through the
/// production egui input path, so success plus cross-window focus proves the
/// lookup reached the target instead of returning "not found".
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
            pip_status: None,
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
            semantic_state: Default::default(),
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
    assert_eq!(response, r#"{"ok":true}"#);
    assert_eq!(h.app.active_window, 1, "target window must become active");
    let focused_pane = h.app.windows[1]
        .focused_pane
        .and_then(|tile_id| h.app.windows[1].tree.tiles.get(tile_id))
        .and_then(|tile| match tile {
            egui_tiles::Tile::Pane(pane_id) => Some(*pane_id),
            _ => None,
        });
    assert_eq!(focused_pane, Some(cross_window_pane_id));
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
        pip_status: None,
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
        semantic_state: Default::default(),
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
            pip_status: None,
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
            semantic_state: Default::default(),
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
        steps: 1,
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
        steps: 1,
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
        steps: 1,
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

/// GetPreviousPaneInfo with steps=2 skips the immediately previous pane and
/// returns the one two hops back.
#[test]
fn get_previous_pane_info_steps_two_returns_second_pane() {
    let mut h = HostHarness::new();
    let pane_a = h.add_test_pane();
    let pane_b = h.add_test_pane();
    let window_id = h.app.windows[0].window_id;

    let tile_a = h.app.windows[0]
        .tree
        .tiles
        .find_pane(&pane_a)
        .expect("tile_a must exist");
    let tile_b = h.app.windows[0]
        .tree
        .tiles
        .find_pane(&pane_b)
        .expect("tile_b must exist");

    // History order (oldest first): pane_a, pane_b.
    // Reversed: pane_b is step 1, pane_a is step 2.
    h.app.pane_focus_history.push((window_id, tile_a));
    h.app.pane_focus_history.push((window_id, tile_b));

    let resp_file = std::env::temp_dir().join("plexi_test_prev_pane_steps2.json");
    let _ = std::fs::remove_file(&resp_file);

    h.inject_ipc(crate::app_protocol::AppRequest::GetPreviousPaneInfo {
        response_file: resp_file.to_string_lossy().to_string(),
        steps: 2,
    });
    h.app.drain_pane_cmd_channel();

    let json = std::fs::read_to_string(&resp_file).expect("response file must be written");
    let v: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
    assert!(v.get("error").is_none(), "unexpected error: {json}");
    assert_eq!(
        v["id"].as_u64(),
        Some(pane_a),
        "step 2 must return pane_a (2 hops back); got: {json}"
    );
    let _ = std::fs::remove_file(&resp_file);
}

/// GetPreviousPaneInfo returns an error when steps exceeds available valid history.
#[test]
fn get_previous_pane_info_steps_exceeds_history_returns_error() {
    let mut h = HostHarness::new();
    let pane_a = h.add_test_pane();
    let window_id = h.app.windows[0].window_id;

    let tile_a = h.app.windows[0]
        .tree
        .tiles
        .find_pane(&pane_a)
        .expect("tile_a must exist");
    h.app.pane_focus_history.push((window_id, tile_a));

    let resp_file = std::env::temp_dir().join("plexi_test_prev_pane_steps_overflow.json");
    let _ = std::fs::remove_file(&resp_file);

    // Only 1 valid entry in history; requesting step 5 should error.
    h.inject_ipc(crate::app_protocol::AppRequest::GetPreviousPaneInfo {
        response_file: resp_file.to_string_lossy().to_string(),
        steps: 5,
    });
    h.app.drain_pane_cmd_channel();

    let json =
        std::fs::read_to_string(&resp_file).expect("response file must be written even on error");
    let v: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
    assert!(
        v.get("error").is_some(),
        "expected error when steps exceeds history; got: {json}"
    );
    let _ = std::fs::remove_file(&resp_file);
}

// ── `[launch] on_launch` dedup policy (#0336) ───────────────────────────────

/// An empty window in `context_id`, ready for `add_app_pane_in_window`.
fn empty_window(context_id: u64, window_id: u64) -> Window {
    Window {
        name: "Ctx".into(),
        path: std::env::temp_dir(),
        tree: egui_tiles::Tree::empty("on_launch_test"),
        panes: HashMap::new(),
        focused_pane: None,
        zoomed_pane: None,
        grid_x: 0,
        grid_y: 1,
        window_id,
        context_id,
    }
}

fn context_b(context_id: u64) -> crate::host::context::Context {
    crate::host::context::Context {
        name: "Context B".into(),
        path: std::env::temp_dir(),
        root: None,
        description: None,
        context_id,
        parent_id: None,
        depth: 0,
        parked: false,
    }
}

/// Build an `AppRegistry` holding a single app `id` with the given `on_launch`
/// policy, staged from a real manifest so it exercises the same load path as
/// production. The returned `TempDir` must be kept alive by the caller.
fn registry_with_on_launch(
    id: &str,
    on_launch: &str,
) -> (tempfile::TempDir, crate::app::registry::AppRegistry) {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let global = tmp.path().join("global-apps");
    let app_dir = global.join(id);
    std::fs::create_dir_all(&app_dir).expect("app dir");
    std::fs::write(
        app_dir.join("manifest.toml"),
        format!(
            "schema_version = 1\n\n[app]\nid = \"{id}\"\ntype = \"app\"\nname = \"{id}\"\n\
             version = \"0.0.1\"\nentry = \"main.py\"\n\n[launch]\non_launch = \"{on_launch}\"\n"
        ),
    )
    .expect("manifest");
    std::fs::write(app_dir.join("main.py"), "#!/usr/bin/env python3\n").expect("entry");
    let registry = crate::app::registry::AppRegistry::load_with_global(tmp.path(), &global);
    assert!(
        registry.get(id).is_some(),
        "staged app '{id}' must load into the registry"
    );
    (tmp, registry)
}

/// `focus_existing`: relaunching an app that is already open in a *different*
/// context focuses the existing pane and jumps to its context instead of
/// spawning a second instance.
#[test]
fn on_launch_focus_existing_focuses_cross_context_instead_of_spawning() {
    let mut h = HostHarness::new();
    let _pane_a = h.add_test_pane(); // window 0, context 1

    h.app.windows.push(empty_window(2, 2)); // window index 1, context 2
    let (_tile, instance_pane) = h.app.add_app_pane_in_window(1, "singleton");
    h.app.router.push(context_b(2));

    let (_tmp, registry) = registry_with_on_launch("singleton", "focus_existing");
    h.app.registry = registry;

    assert_eq!(h.app.active_window, 0);
    let caller_ctx = h.app.windows[0].context_id;
    let focused = h
        .app
        .resolve_on_launch_policy("singleton", caller_ctx, &[], None);

    assert_eq!(
        focused,
        Some(instance_pane),
        "focus_existing must satisfy the launch by focusing the existing instance and \
         return that instance's real pane id (never a predicted spawn id)"
    );
    assert_eq!(
        h.app.active_window, 1,
        "must jump to the window that holds the existing instance"
    );
    assert!(
        h.app.windows[1].focused_pane.is_some(),
        "the existing instance's pane must be focused"
    );
    assert!(
        h.app.find_app_pane_by_type("singleton", None).map(|(p, _)| p) == Some(instance_pane),
        "no second instance should have been spawned"
    );
}

/// `focus_existing_in_context`: relaunching in a context that has no instance
/// spawns there (resolver returns false = caller spawns); relaunching in a
/// context that already has one focuses it (resolver returns true).
#[test]
fn on_launch_focus_existing_in_context_is_per_context() {
    let mut h = HostHarness::new();
    let _pane_a = h.add_test_pane(); // window 0, context 1

    // An instance living in context 2 must NOT satisfy a launch from context 1.
    h.app.windows.push(empty_window(2, 2)); // window index 1, context 2
    let _other = h.app.add_app_pane_in_window(1, "percontext");
    h.app.router.push(context_b(2));

    let (_tmp, registry) = registry_with_on_launch("percontext", "focus_existing_in_context");
    h.app.registry = registry;

    let ctx1 = h.app.windows[0].context_id;
    assert!(
        h.app
            .resolve_on_launch_policy("percontext", ctx1, &[], None)
            .is_none(),
        "an instance in another context must not satisfy focus_existing_in_context — caller spawns"
    );
    assert_eq!(h.app.active_window, 0, "no cross-context jump should occur");

    // Now plant an instance in context 1 (window 0); the relaunch must focus it.
    let (_tile, same_ctx_pane) = h.app.add_app_pane_in_window(0, "percontext");
    assert_eq!(
        h.app
            .resolve_on_launch_policy("percontext", ctx1, &[], None),
        Some(same_ctx_pane),
        "an instance in the caller's context must be focused and its real id returned"
    );
    assert_eq!(h.app.active_window, 0);
    assert_eq!(
        h.app.find_app_pane_by_type("percontext", Some(ctx1)).map(|(p, _)| p),
        Some(same_ctx_pane),
        "the in-context instance is the focus target"
    );
}

/// Regression: dedup identity is the pane's `manifest_id`, not its runtime
/// `type_id`. WASM panes all report `type_id() == "wasm"`, so a resolver that
/// matched on runtime type id would fail to focus an existing WASM instance
/// (and would spawn duplicates). Simulate the mismatch and confirm the lookup
/// still finds the instance by manifest id.
#[test]
fn on_launch_matches_by_manifest_id_not_runtime_type_id() {
    let mut h = HostHarness::new();
    let _pane_a = h.add_test_pane(); // window 0, context 1

    // Instance whose runtime type_id is the generic "wasm" but whose manifest
    // identity is the real app id — as a WASM app pane is stored.
    let (_tile, wasm_pane) =
        h.app
            .add_app_pane_in_window_with_runtime_id(0, "wasm_singleton", "wasm");

    let (_tmp, registry) = registry_with_on_launch("wasm_singleton", "focus_existing");
    h.app.registry = registry;

    assert_eq!(
        h.app.find_app_pane_by_type("wasm_singleton", None).map(|(p, _)| p),
        Some(wasm_pane),
        "lookup must match the WASM instance by manifest id"
    );
    let ctx1 = h.app.windows[0].context_id;
    assert_eq!(
        h.app
            .resolve_on_launch_policy("wasm_singleton", ctx1, &[], None),
        Some(wasm_pane),
        "focus_existing must focus the existing WASM instance (by real id) instead of spawning"
    );
}

/// Regression: an instance sitting behind an overlay is still deduped.
/// Overlay launches move the covered pane into the overlay app's
/// `overlay_replaced` (it leaves `win.panes`), so a naive top-level scan would
/// miss it and spawn a duplicate. `focus_existing` must find the buried
/// instance, pop the overlay to reveal it, and focus it.
#[test]
fn on_launch_focus_existing_reveals_instance_behind_overlay() {
    let mut h = HostHarness::new();

    // Slot pane holds the singleton "buried"; then an overlay app takes over
    // the same slot, boxing the singleton into its `overlay_replaced`.
    let (tile, slot) = h.app.add_app_pane_in_window(0, "buried");
    h.app.windows[0].focused_pane = Some(tile);

    let overlay_pane = {
        use crate::app::permissions::AppPermissions;
        use crate::host::pane::{AppPane, AppRuntime, Pane};
        use crate::process_app::ProcessApp;
        let covered = h.app.windows[0]
            .panes
            .remove(&slot)
            .expect("singleton pane present before overlay");
        let (mut proc, _tx) = ProcessApp::new_for_test(slot, AppPermissions::builtin());
        proc.type_id = "overlay_app".to_string();
        Pane::App(Box::new(AppPane {
            pip_status: None,
            id: slot,
            runtime: AppRuntime::Process(Box::new(proc)),
            workspace_root: std::env::temp_dir(),
            permissions: AppPermissions::builtin(),
            manifest_id: "overlay_app".to_string(),
            name: "Overlay".to_string(),
            pane_group: None,
            linked_pane_id: None,
            overlay_replaced: Some(Box::new(covered)),
            hidden: false,
            agent: None,
            slots: std::collections::HashMap::new(),
            semantic_state: Default::default(),
        }))
    };
    h.app.windows[0].panes.insert(slot, overlay_pane);

    let (_tmp, registry) = registry_with_on_launch("buried", "focus_existing");
    h.app.registry = registry;

    // The buried instance must be located (depth 1) despite not being a
    // top-level pane, and the resolver must reveal + focus it, not spawn.
    assert_eq!(
        h.app.locate_app_instance("buried", None),
        Some((0, slot, 1)),
        "buried instance must be found one overlay deep"
    );
    let ctx1 = h.app.windows[0].context_id;
    assert_eq!(
        h.app.resolve_on_launch_policy("buried", ctx1, &[], None),
        Some(slot),
        "focus_existing must reveal the overlay-covered instance (its real id) instead of spawning"
    );
    // After popping the overlay, the slot holds the singleton again.
    assert_eq!(
        h.app.windows[0]
            .panes
            .get(&slot)
            .and_then(|p| p.as_app())
            .map(|a| a.manifest_id.as_str()),
        Some("buried"),
        "the overlay must have been popped to reveal the singleton"
    );
}

/// `always_new` (and an unset policy) never dedups: the resolver returns false
/// even when an instance is already open, so the caller spawns a fresh one.
#[test]
fn on_launch_always_new_never_dedups() {
    let mut h = HostHarness::new();
    let _pane_a = h.add_test_pane(); // window 0, context 1
    let _existing = h.app.add_app_pane_in_window(0, "stacker");

    let (_tmp, registry) = registry_with_on_launch("stacker", "always_new");
    h.app.registry = registry;

    let ctx1 = h.app.windows[0].context_id;
    assert!(
        h.app
            .resolve_on_launch_policy("stacker", ctx1, &[], None)
            .is_none(),
        "always_new must always let the caller spawn a fresh instance"
    );

    // An app with no [launch] on_launch at all defaults to always_new.
    let _unset = h.app.add_app_pane_in_window(0, "test");
    assert!(
        h.app
            .resolve_on_launch_policy("test", ctx1, &[], None)
            .is_none(),
        "an unset on_launch defaults to always_new (no dedup)"
    );
}

/// The split-mirror path duplicates the focused pane on purpose, so it must
/// bypass the on_launch dedup policy: mirror-splitting a `focus_existing`
/// (singleton) app spawns a SECOND instance instead of focusing the first and
/// silently no-oping. Without the bypass the policy would win and pane_count
/// would stay at 1.
#[test]
fn split_mirror_bypasses_on_launch_dedup() {
    let mut h = HostHarness::new();

    // Launch a builtin app into the empty context → one focused App pane.
    // (text-editor is a builtin, so it launches without an external process.)
    h.app
        .launch_app_by_id_with_layout("text-editor", None, &[], None)
        .expect("text-editor launch must succeed");
    assert_eq!(h.pane_count(), 1, "one instance after the first launch");

    // Make the app a singleton. The registry entry names the same id as the
    // builtin, so the dedup policy is now live for "text-editor".
    let (_tmp, registry) = registry_with_on_launch("text-editor", "focus_existing");
    h.app.registry = registry;
    assert_eq!(
        h.app.registry.on_launch_for("text-editor"),
        crate::app::registry::OnLaunchPolicy::FocusExisting,
        "policy must be active for the mirror to have something to bypass"
    );

    // Mirror-split must still produce a second instance despite the singleton
    // policy — the mirror uses the forced (dedup-bypassing) launch path.
    h.app
        .split_focused_mirror(crate::host::command::Placement::Right);
    assert_eq!(
        h.pane_count(),
        2,
        "mirror-split of a focus_existing app must spawn a second instance, not dedup"
    );
}
