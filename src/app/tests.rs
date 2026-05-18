use super::*;
use crate::app_trait::AppCommand;
use crate::context::Window;
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
    h.app.router.push(crate::context::Context {
        name: "Context B".into(),
        path: std::env::temp_dir(),
        root: None,
        context_id: 2,
        parent_id: None,
        depth: 0,
    });

    assert_eq!(h.app.active_window, 0);
    assert!(h.app.pane_navigate(9901));
    assert_eq!(h.app.active_window, 1, "active_window must switch to window 1");
    assert!(h.app.windows[1].focused_pane.is_some(), "focused_pane must be set on target window");
}

/// Regression guard for #823: pane_navigate must also sync router.active_idx
/// so the sidebar context switcher reflects the new active context immediately.
#[test]
fn pane_navigate_cross_window_syncs_router() {
    let mut h = HostHarness::new();
    let _pane_a = h.add_test_pane();
    h.app.windows.push(second_window(2, 2, 9902));
    h.app.router.push(crate::context::Context {
        name: "Context B".into(),
        path: std::env::temp_dir(),
        root: None,
        context_id: 2,
        parent_id: None,
        depth: 0,
    });

    assert_eq!(h.app.router.active_idx(), 0);
    assert!(h.app.pane_navigate(9902));
    assert_eq!(h.app.router.active_idx(), 1, "router must reflect new active context after pane_navigate");
}

/// Regression guard for #791: dispatch_notify_action_cmds with pane_focus
/// host_action must call pane_navigate synchronously before writing the
/// response file (so navigation is complete before the shell unblocks).
#[test]
fn dispatch_notify_action_pane_focus_navigates() {
    let mut h = HostHarness::new();
    let sender_id = h.add_test_pane();
    h.app.windows.push(second_window(2, 2, 9903));
    h.app.router.push(crate::context::Context {
        name: "Context B".into(),
        path: std::env::temp_dir(),
        root: None,
        context_id: 2,
        parent_id: None,
        depth: 0,
    });

    assert_eq!(h.app.active_window, 0);
    h.app.dispatch_notify_action_cmds(vec![AppCommand::DeliverNotifyAction {
        pane_id: sender_id,
        notify_id: "n1".into(),
        action_label: "Go".into(),
        value: None,
        response_file: None,
        host_action: Some("pane_focus:9903".into()),
    }]);
    assert_eq!(h.app.active_window, 1, "pane_focus host_action must navigate to the target window");
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
        use crate::pane::{AppPane, AppRuntime};
        use crate::process_app::ProcessApp;
        use crate::app_permissions::AppPermissions;
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
        }
    };
    win1.panes.insert(cross_window_pane_id, crate::pane::Pane::App(Box::new(app_pane)));
    h.app.windows.push(win1);
    h.app.router.push(crate::context::Context {
        name: "Context B".into(),
        path: std::env::temp_dir(),
        root: None,
        context_id: 2,
        parent_id: None,
        depth: 0,
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
    let (orphan_process, _tx) =
        crate::process_app::ProcessApp::new_for_test(orphan_id, crate::app_permissions::AppPermissions::builtin());
    let orphan_pane = crate::pane::Pane::App(Box::new(crate::pane::AppPane {
        id: orphan_id,
        runtime: crate::pane::AppRuntime::Process(Box::new(orphan_process)),
        workspace_root: std::env::temp_dir(),
        permissions: crate::app_permissions::AppPermissions::builtin(),
        manifest_id: "orphan".to_string(),
        name: "Orphan".to_string(),
        pane_group: None,
        linked_pane_id: None,
        overlay_replaced: None,
    }));
    h.app.windows[0].panes.insert(orphan_id, orphan_pane);
    assert!(h.app.windows[0].tree.tiles.find_pane(&orphan_id).is_none(), "orphan has no tile");

    let resp_file = std::env::temp_dir().join("plexi_test_pane_list_996.json");
    h.inject_ipc(crate::app_protocol::AppRequest::ListPanes {
        response_file: resp_file.to_string_lossy().to_string(),
    });
    h.app.drain_pane_cmd_channel();

    let json = std::fs::read_to_string(&resp_file).expect("ListPanes must write response file");
    let panes: Vec<serde_json::Value> = serde_json::from_str(&json).expect("valid JSON");
    let ids: Vec<u64> = panes.iter().filter_map(|p| p["id"].as_u64()).collect();

    assert!(ids.contains(&real_pane_id), "real pane must appear in pane_list");
    assert!(!ids.contains(&orphan_id), "orphaned pane (no tile) must NOT appear in pane_list");

    for id in &ids {
        assert!(
            h.app.pane_navigate(*id),
            "pane_navigate must succeed for every id returned by pane_list (failed for {id})"
        );
    }

    let _ = std::fs::remove_file(&resp_file);
}

/// #840: A snoozed notification must become invisible immediately after
/// deliver_after is set, and visible again once the instant has elapsed.
#[test]
fn snoozed_notification_invisible_then_visible() {
    let mut h = HostHarness::new();

    // Push a notification that is already past its snooze window.
    let wake_past = std::time::Instant::now() - std::time::Duration::from_secs(1);
    h.app.pending_notifications.push(PendingNotification {
        notify_id: "snoozed-woken".into(),
        sender_pane_id: 0,
        source_context_id: h.app.router.active().context_id,
        level: "info".into(),
        title: "Snoozed".into(),
        body: "body".into(),
        kind: crate::app_protocol::NotifyKind::Message,
        options: vec![],
        input_prompt: None,
        required: false,
        priority: 100,
        scope: crate::app_protocol::NotifyScope::Global,
        image_inline: None,
        image_pipe_id: None,
        response_file: None,
        timeout_secs: None,
        on_dismiss: None,
        enqueued_at: std::time::Instant::now(),
        tombstoned: false,
        deliver_after: Some(wake_past), // already elapsed → visible
    });

    assert_eq!(h.app.visible_notification_count(), 1, "past-snooze notification must be visible");

    // Now set deliver_after to the future (active snooze).
    let wake_future = std::time::Instant::now() + std::time::Duration::from_secs(300);
    h.app.pending_notifications[0].deliver_after = Some(wake_future);

    assert_eq!(h.app.visible_notification_count(), 0, "future-snooze notification must be invisible");
}

/// #840: tick_notification_timeouts must not time out a snoozed notification.
#[test]
fn snoozed_notification_exempt_from_timeout() {
    let mut h = HostHarness::new();

    let sender_id = h.add_test_pane();
    // Enqueued 10 minutes ago with a 60s timeout, but snoozed into the future.
    h.app.pending_notifications.push(PendingNotification {
        notify_id: "snoozed-no-timeout".into(),
        sender_pane_id: sender_id,
        source_context_id: h.app.router.active().context_id,
        level: "info".into(),
        title: "ShouldNotTimeout".into(),
        body: "body".into(),
        kind: crate::app_protocol::NotifyKind::Message,
        options: vec![],
        input_prompt: None,
        required: false,
        priority: 100,
        scope: crate::app_protocol::NotifyScope::Global,
        image_inline: None,
        image_pipe_id: None,
        response_file: None,
        timeout_secs: Some(60),
        on_dismiss: None,
        enqueued_at: std::time::Instant::now() - std::time::Duration::from_secs(600),
        tombstoned: false,
        deliver_after: Some(std::time::Instant::now() + std::time::Duration::from_secs(300)),
    });

    h.app.tick_notification_timeouts();

    assert_eq!(
        h.app.pending_notifications.len(), 1,
        "snoozed notification must survive tick_notification_timeouts"
    );
}

/// Build a second window in the SAME workspace (context_id = 1) placed
/// directly below window 0 on the spatial grid (grid_y = 1).
/// This is different from `second_window()` which uses context_id = 2
/// (a separate workspace used for cross-context tests).
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

/// Build a third window in the same workspace at grid_y=2, directly below
/// `same_workspace_window_below` (which is at grid_y=1).
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

/// #1074: navigate(Down) at the vertical pane boundary jumps to the LAST
/// window in the workspace list — skipping intermediate windows entirely.
#[test]
fn navigate_down_at_vertical_boundary_jumps_to_last_window() {
    let mut h = HostHarness::new();
    let pane_a = h.add_test_pane();
    // Three windows: grid_y 0 (window 0), 1 (window 1), 2 (window 2).
    h.app.windows.push(same_workspace_window_below(2, 9910));  // grid_y=1
    h.app.windows.push(same_workspace_window_bottom(3, 9911)); // grid_y=2

    assert!(h.app.pane_navigate(pane_a), "pane_navigate must succeed to set up focus");
    assert_eq!(h.app.active_window, 0);

    // Down from window 0 must jump directly to the LAST window (grid_y=2), not step to grid_y=1.
    h.app.navigate(crate::keys::Direction::Down);
    assert_eq!(h.app.active_window, 2, "navigate(Down) at vertical boundary must jump to last window");
}

/// #1074: navigate(Up) at the vertical pane boundary jumps to the FIRST
/// window in the workspace list.
#[test]
fn navigate_up_at_vertical_boundary_jumps_to_first_window() {
    let mut h = HostHarness::new();
    let pane_a = h.add_test_pane();
    h.app.windows.push(same_workspace_window_below(2, 9910));  // grid_y=1
    h.app.windows.push(same_workspace_window_bottom(3, 9911)); // grid_y=2

    // Start from the middle window.
    assert_eq!(h.app.active_window, 0);
    h.app.active_window = 1;

    // Up from window 1 must jump to the FIRST window (grid_y=0).
    h.app.navigate(crate::keys::Direction::Up);
    assert_eq!(h.app.active_window, 0, "navigate(Up) at vertical boundary must jump to first window");
}

/// Single-window workspace: navigate(Down) at boundary is a no-op —
/// the only window is both first and last.
#[test]
fn navigate_down_single_window_is_noop() {
    let mut h = HostHarness::new();
    let pane_a = h.add_test_pane();
    assert!(h.app.pane_navigate(pane_a), "pane_navigate must succeed");
    assert_eq!(h.app.active_window, 0);
    h.app.navigate(crate::keys::Direction::Down);
    assert_eq!(h.app.active_window, 0, "navigate(Down) in single-window workspace must not change active_window");
}

/// Horizontal boundary (Left/Right) still falls through to page navigation unchanged.
#[test]
fn navigate_left_at_horizontal_boundary_still_page_navigates() {
    let mut h = HostHarness::new();
    let pane_a = h.add_test_pane();
    assert!(h.app.pane_navigate(pane_a), "pane_navigate must succeed");
    assert_eq!(h.app.active_window, 0);
    // No window to the left of (0,0) → Left is a no-op (not wrapped).
    h.app.navigate(crate::keys::Direction::Left);
    assert_eq!(h.app.active_window, 0, "Left at boundary must not change active_window");
}

/// Regression guard for #1110: setting renaming_pane + sync_rename_pane_focus()
/// must push FocusLayer::RenamePane in the same call so input_captured_by_overlay()
/// is accurate immediately — not deferred to the next frame.
#[test]
fn rename_pane_focus_layer_syncs_immediately() {
    let mut h = HostHarness::new();
    let pane_a = h.add_test_pane();

    assert!(h.app.focus_stack.is_empty(), "focus stack must start empty");
    assert!(!h.app.input_captured_by_overlay(), "no overlay on start");

    // Simulate what the fixed Action::RenamePane handler now does.
    h.app.renaming_pane = Some(pane_a);
    h.app.rename_pane_focus_requested = false;
    h.app.sync_rename_pane_focus();

    assert_eq!(
        h.app.focus_stack.last(),
        Some(&FocusLayer::RenamePane),
        "FocusLayer::RenamePane must be on the stack after sync"
    );
    assert!(
        h.app.input_captured_by_overlay(),
        "input_captured_by_overlay must be true while rename modal is open"
    );
}

// ── Focus history tests ───────────────────────────────────────────────────

#[test]
fn focus_history_records_on_navigate() {
    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _) = PlexiApp::new_for_test(ctx, frame_tick);
    let (tile_a, _) = app.add_test_pane();
    let (_tile_b, _) = app.add_test_pane();
    app.windows[0].focused_pane = Some(tile_a);

    let old_window_id = app.windows[0].window_id;
    app.push_focus_history(old_window_id, Some(tile_a));

    assert_eq!(app.pane_focus_history.len(), 1);
    assert_eq!(app.pane_focus_history[0].1, tile_a);
    assert!(app.pane_focus_future.is_empty());
}

#[test]
fn focus_history_back_restores_previous_pane() {
    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _) = PlexiApp::new_for_test(ctx, frame_tick);
    let (tile_a, _) = app.add_test_pane();
    let (tile_b, _) = app.add_test_pane();
    let window_id = app.windows[0].window_id;

    app.windows[0].focused_pane = Some(tile_a);
    app.push_focus_history(window_id, Some(tile_a));
    app.windows[0].focused_pane = Some(tile_b);

    app.step_focus_history_back();

    assert_eq!(app.windows[0].focused_pane, Some(tile_a));
    assert_eq!(app.pane_focus_future.len(), 1);
    assert_eq!(app.pane_focus_future[0].1, tile_b);
}

#[test]
fn focus_history_forward_re_applies_undone_move() {
    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _) = PlexiApp::new_for_test(ctx, frame_tick);
    let (tile_a, _) = app.add_test_pane();
    let (tile_b, _) = app.add_test_pane();
    let window_id = app.windows[0].window_id;

    app.windows[0].focused_pane = Some(tile_a);
    app.push_focus_history(window_id, Some(tile_a));
    app.windows[0].focused_pane = Some(tile_b);
    app.step_focus_history_back();
    app.step_focus_history_forward();

    assert_eq!(app.windows[0].focused_pane, Some(tile_b));
}

#[test]
fn focus_history_organic_focus_clears_future() {
    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _) = PlexiApp::new_for_test(ctx, frame_tick);
    let (tile_a, _) = app.add_test_pane();
    let (tile_b, _) = app.add_test_pane();
    let (_tile_c, _) = app.add_test_pane();
    let window_id = app.windows[0].window_id;

    app.windows[0].focused_pane = Some(tile_a);
    app.push_focus_history(window_id, Some(tile_a));
    app.windows[0].focused_pane = Some(tile_b);
    app.step_focus_history_back(); // now future has tile_b

    // organic focus change to tile_c should clear future
    app.push_focus_history(window_id, Some(tile_a));

    assert!(app.pane_focus_future.is_empty());
}

#[test]
fn focus_history_skips_stale_tile() {
    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _) = PlexiApp::new_for_test(ctx, frame_tick);
    let (tile_a, _) = app.add_test_pane();
    let (tile_b, _) = app.add_test_pane();
    let window_id = app.windows[0].window_id;

    app.windows[0].focused_pane = Some(tile_b);
    // Push a stale tile_id that doesn't exist in the tree.
    app.pane_focus_history.push((window_id, egui_tiles::TileId::from_u64(9999)));
    app.pane_focus_history.push((window_id, tile_a));

    app.step_focus_history_back();

    assert_eq!(app.windows[0].focused_pane, Some(tile_a));
}

#[test]
fn focus_changed_detected_on_pane_switch() {
    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _) = PlexiApp::new_for_test(ctx, frame_tick);
    let (tile_a, _) = app.add_test_pane();
    let (tile_b, _) = app.add_test_pane();
    let win_id = app.windows[0].window_id;

    // Simulate first focus on tile_a
    app.windows[0].focused_pane = Some(tile_a);
    app.last_logged_focus = None;
    app.focus_started_at = None;

    // Mirrors the frame-end detection in update()
    let current_focus = app.windows
        .get(app.active_window)
        .and_then(|win| win.focused_pane.map(|tile| (win.window_id, tile)));
    assert_ne!(current_focus, app.last_logged_focus);
    app.last_logged_focus = current_focus;
    app.focus_started_at = Some(std::time::Instant::now());

    assert_eq!(app.last_logged_focus, Some((win_id, tile_a)));

    // Switch to tile_b
    app.windows[0].focused_pane = Some(tile_b);

    let current_focus = app.windows
        .get(app.active_window)
        .and_then(|win| win.focused_pane.map(|tile| (win.window_id, tile)));
    assert_ne!(current_focus, app.last_logged_focus);

    app.last_logged_focus = current_focus;
    app.focus_started_at = Some(std::time::Instant::now());

    assert_eq!(app.last_logged_focus, Some((win_id, tile_b)));
}

#[test]
fn focus_changed_not_emitted_for_same_pane() {
    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _) = PlexiApp::new_for_test(ctx, frame_tick);
    let (tile_a, _) = app.add_test_pane();
    let win_id = app.windows[0].window_id;

    app.windows[0].focused_pane = Some(tile_a);
    app.last_logged_focus = Some((win_id, tile_a));

    // Same focus — current_focus == last_logged_focus, no emission expected.
    let current_focus = app.windows
        .get(app.active_window)
        .and_then(|win| win.focused_pane.map(|tile| (win.window_id, tile)));
    assert_eq!(current_focus, app.last_logged_focus);
}

#[test]
fn focus_history_depth_caps_at_configured_limit() {
    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);
    app.focus_history_depth = 1;

    let (tile_a, _) = app.add_test_pane();
    let (tile_b, _) = app.add_test_pane();
    let (tile_c, _) = app.add_test_pane();
    let win_id = app.windows[0].window_id;

    app.windows[0].focused_pane = Some(tile_a);
    app.push_focus_history(win_id, Some(tile_a));
    assert_eq!(app.pane_focus_history.len(), 1);

    app.windows[0].focused_pane = Some(tile_b);
    app.push_focus_history(win_id, Some(tile_b));
    assert_eq!(app.pane_focus_history.len(), 1, "history should stay capped at depth 1");

    app.windows[0].focused_pane = Some(tile_c);
    app.push_focus_history(win_id, Some(tile_c));
    assert_eq!(app.pane_focus_history.len(), 1);
    assert_eq!(app.pane_focus_history[0], (win_id, tile_c));
}

#[test]
fn focus_future_caps_at_configured_depth() {
    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);
    app.focus_history_depth = 2;

    let (tile_a, _) = app.add_test_pane();
    let (tile_b, _) = app.add_test_pane();
    let (tile_c, _) = app.add_test_pane();
    let win_id = app.windows[0].window_id;

    app.windows[0].focused_pane = Some(tile_a);
    app.push_focus_history(win_id, Some(tile_a));
    app.windows[0].focused_pane = Some(tile_b);
    app.push_focus_history(win_id, Some(tile_b));
    app.windows[0].focused_pane = Some(tile_c);
    app.push_focus_history(win_id, Some(tile_c));

    app.step_focus_history_back();
    app.step_focus_history_back();
    app.step_focus_history_back();
    assert!(app.pane_focus_future.len() <= 2, "future stack should be capped at depth");
}

#[test]
fn split_with_new_pane_pushes_focus_history() {
    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);

    let (tile_a, _pane_a) = app.add_test_pane();
    app.windows[0].focused_pane = Some(tile_a);

    // Split — this should record tile_a in focus history before moving focus.
    let new_pane_id = 50000;
    let share = crate::host::command::ShareRatio { numerator: 1.0, denominator: 1.0 };
    let new_tile = app.split_with_new_pane(new_pane_id, true, share, false);
    assert!(new_tile.is_some(), "split_with_new_pane should succeed");

    // Focus should now be on the new tile.
    assert_eq!(app.windows[0].focused_pane, new_tile);

    // History should contain tile_a so Cmd+[ works.
    assert!(!app.pane_focus_history.is_empty(), "focus history should be non-empty after split");
    assert_eq!(app.pane_focus_history.last().unwrap().1, tile_a);

    // Step back should restore focus to tile_a.
    app.step_focus_history_back();
    assert_eq!(app.windows[0].focused_pane, Some(tile_a));
}

/// Regression guard for #1384: creating a child context should adopt the focused
/// pane from the parent, not start with a blank terminal.
#[test]
fn new_child_context_adopts_focused_pane() {
    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);

    // Add a pane and focus it in the parent.
    let (tile_id, pane_id) = app.add_test_pane();
    app.windows[0].focused_pane = Some(tile_id);

    let parent_name = "Test"; // initial context name from new_for_test
    app.new_child_context(parent_name, std::path::PathBuf::from("/tmp/child"))
        .expect("new_child_context should succeed");

    // Parent's focused tile must now be a SubContext pointing at the child.
    let parent_focused_tile = app.windows[0].focused_pane.expect("parent has focused tile");
    let parent_sub_pane_id = match app.windows[0].tree.tiles.get(parent_focused_tile) {
        Some(egui_tiles::Tile::Pane(id)) => *id,
        other => panic!("expected Tile::Pane, got {other:?}"),
    };
    let child_ctx_id = app.windows[0].panes.get(&parent_sub_pane_id)
        .and_then(|p| p.as_sub_context())
        .expect("parent focused pane is SubContext");

    // The original pane must not remain in the parent.
    assert!(
        app.windows[0].panes.get(&pane_id).is_none(),
        "adopted pane must be removed from parent"
    );

    // The child context and window must exist.
    assert!(
        app.router.position(|c| c.context_id == child_ctx_id).is_some(),
        "child context must be in router"
    );
    let child_win_idx = app.windows.iter().position(|w| w.context_id == child_ctx_id)
        .expect("child window must exist");

    // The child window must have exactly the adopted pane.
    assert_eq!(app.windows[child_win_idx].panes.len(), 1, "child has exactly 1 pane");
    assert!(
        app.windows[child_win_idx].panes.contains_key(&pane_id),
        "child window must contain the adopted pane"
    );

    // Child's focused tile must point at the adopted pane.
    let child_focused_tile = app.windows[child_win_idx].focused_pane.expect("child has focused tile");
    let child_focused_pane_id = match app.windows[child_win_idx].tree.tiles.get(child_focused_tile) {
        Some(egui_tiles::Tile::Pane(id)) => *id,
        other => panic!("expected Tile::Pane in child, got {other:?}"),
    };
    assert_eq!(child_focused_pane_id, pane_id, "child focuses the adopted pane");
}

/// Regression guard for #1384: when no pane is focused, the fallback path must
/// create a terminal in the child and add a SubContext tile alongside the parent's
/// existing panes (legacy behavior preserved).
#[test]
fn new_child_context_fallback_when_no_focused_pane() {
    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);

    // Explicitly clear focused pane so fallback triggers.
    app.windows[0].focused_pane = None;

    let parent_pane_count_before = app.windows[0].panes.len();

    // new_child_context returns an error when create_single_pane_tree fails (no PTY
    // in test environment), so we only assert on the state mutations that happen
    // before the terminal spawn — specifically that the adoption path was NOT taken.
    // In prod this path creates a terminal; in tests it errors out at PTY spawn.
    let result = app.new_child_context("Test", std::path::PathBuf::from("/tmp/child2"));

    // Whether success or failure, the parent's focused_pane must remain None
    // (we did not adopt anything).
    assert_eq!(app.windows[0].focused_pane, None, "parent focused_pane untouched on fallback");

    // Parent pane count must be unchanged if the terminal fallback failed.
    if result.is_err() {
        assert_eq!(
            app.windows[0].panes.len(), parent_pane_count_before,
            "parent panes unchanged on failed fallback"
        );
    }
}

/// Issue #1409: parent name lookup must be case-insensitive.
#[test]
fn new_child_context_case_insensitive_parent() {
    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);

    // Set up a focused pane so the adoption path is taken (not the PTY fallback).
    let (tile_id, _pane_id) = app.add_test_pane();
    app.windows[0].focused_pane = Some(tile_id);

    // The initial context is named "Test" (from new_for_test).
    // Lookup with lowercase "test" must succeed.
    let result = app.new_child_context("test", std::path::PathBuf::from("/tmp/child_ci"));
    assert!(result.is_ok(), "case-insensitive lookup should succeed: {result:?}");
}

/// Issue #1409: creating a child context with a parent should auto-zoom into it.
#[test]
fn create_child_context_auto_zooms() {
    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);

    let (tile_id, _pane_id) = app.add_test_pane();
    app.windows[0].focused_pane = Some(tile_id);

    let parent_ctx_id = app.router.active().context_id;

    // Simulate what the CreateContext handler does: capture current state,
    // call new_child_context, then push_depth + switch_workspace.
    let current_win_id = app.windows[app.active_window].window_id;
    let current_focused = app.windows[app.active_window].focused_pane;
    app.new_child_context("Test", std::path::PathBuf::from("/tmp/child_zoom"))
        .expect("should succeed");
    let new_ctx_idx = app.router.len() - 1;
    let new_ctx_id = app.router.get(new_ctx_idx).context_id;
    app.router.push_depth(parent_ctx_id, current_win_id, current_focused);
    app.switch_workspace(new_ctx_idx);

    // Active context must now be the child.
    assert_eq!(
        app.router.active().context_id,
        new_ctx_id,
        "should be zoomed into child context after creation"
    );

    // Depth stack must have one entry (parent pushed).
    assert_eq!(app.router.current_depth(), 1);
}

#[test]
fn persist_roundtrip() {
    let dir = std::env::temp_dir().join(format!("plexi_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("notifications.json");

    let n = PendingNotification {
        notify_id: "test-id".into(),
        sender_pane_id: 0,
        source_context_id: 1,
        level: "info".into(),
        title: "Test".into(),
        body: "body".into(),
        kind: crate::app_protocol::NotifyKind::Message,
        options: vec![],
        input_prompt: None,
        required: false,
        priority: 50,
        scope: crate::app_protocol::NotifyScope::Global,
        image_inline: None,
        image_pipe_id: None,
        response_file: Some("/tmp/resp".into()),
        timeout_secs: None,
        on_dismiss: None,
        enqueued_at: std::time::Instant::now(),
        tombstoned: false,
        deliver_after: None,
    };

    save_pending_notifications_to(&[n], &path);
    let restored = load_pending_notifications_from(&path);

    assert_eq!(restored.len(), 1);
    assert_eq!(restored[0].notify_id, "test-id");
    assert_eq!(restored[0].title, "Test");
    assert!(restored[0].tombstoned, "restored notification must be tombstoned");
    assert!(restored[0].response_file.is_none(), "response_file must be cleared on restore");
    assert!(restored[0].image_pipe_id.is_none(), "image_pipe_id must be cleared on restore");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn persist_ttl_drops_old_notifications() {
    let dir = std::env::temp_dir().join(format!("plexi_test_ttl_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("notifications.json");

    let now_sys = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let eight_days_ago = now_sys.saturating_sub(8 * 24 * 3600);
    let json = format!(
        r#"[{{"notify_id":"old","sender_pane_id":0,"source_context_id":1,"level":"info","title":"Old","body":"","kind":"message","options":[],"required":false,"priority":0,"scope":"global","enqueued_at_secs":{},"tombstoned":false}}]"#,
        eight_days_ago
    );
    std::fs::write(&path, json).unwrap();

    let restored = load_pending_notifications_from(&path);
    assert!(restored.is_empty(), "notification older than 7 days must be dropped");

    let _ = std::fs::remove_dir_all(&dir);
}
