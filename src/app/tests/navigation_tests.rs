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
        h.app
            .find_app_pane_by_type("singleton", None)
            .map(|(p, _)| p)
            == Some(instance_pane),
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
        h.app
            .find_app_pane_by_type("percontext", Some(ctx1))
            .map(|(p, _)| p),
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
        h.app
            .find_app_pane_by_type("wasm_singleton", None)
            .map(|(p, _)| p),
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
