use super::super::*;
use crate::app::app_trait::AppCommand;
use crate::host::context::Window;
use crate::testing::HostHarness;

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
    app.pane_focus_history
        .push((window_id, egui_tiles::TileId::from_u64(9999)));
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
    let current_focus = app
        .windows
        .get(app.active_window)
        .and_then(|win| win.focused_pane.map(|tile| (win.window_id, tile)));
    assert_ne!(current_focus, app.last_logged_focus);
    app.last_logged_focus = current_focus;
    app.focus_started_at = Some(std::time::Instant::now());

    assert_eq!(app.last_logged_focus, Some((win_id, tile_a)));

    // Switch to tile_b
    app.windows[0].focused_pane = Some(tile_b);

    let current_focus = app
        .windows
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
    let current_focus = app
        .windows
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
    assert_eq!(
        app.pane_focus_history.len(),
        1,
        "history should stay capped at depth 1"
    );

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
    assert!(
        app.pane_focus_future.len() <= 2,
        "future stack should be capped at depth"
    );
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
    let share = crate::host::command::ShareRatio {
        numerator: 1.0,
        denominator: 1.0,
    };
    let new_tile = app.split_with_new_pane(new_pane_id, true, share, false, false);
    assert!(new_tile.is_some(), "split_with_new_pane should succeed");

    // Focus should now be on the new tile.
    assert_eq!(app.windows[0].focused_pane, new_tile);

    // History should contain tile_a so Cmd+[ works.
    assert!(
        !app.pane_focus_history.is_empty(),
        "focus history should be non-empty after split"
    );
    assert_eq!(app.pane_focus_history.last().unwrap().1, tile_a);

    // Step back should restore focus to tile_a.
    app.step_focus_history_back();
    assert_eq!(app.windows[0].focused_pane, Some(tile_a));
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

/// Regression guard for #2043: switch_workspace must push focus history so
/// Cmd+[ returns to the pane that was active before the context switch.
#[test]
fn switch_workspace_pushes_focus_history() {
    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);

    // Set up a focused pane in context A (the initial context).
    let (tile_a, _) = app.add_test_pane();
    app.windows[0].focused_pane = Some(tile_a);
    let win_a_id = app.windows[0].window_id;
    let ctx_a_id = app.windows[0].context_id;

    // Add a second context B with its own window.
    let ctx_b_id = app.next_window_id;
    app.next_window_id += 1;
    let win_b_id = app.next_window_id;
    app.next_window_id += 1;
    app.router.push(crate::host::context::Context {
        name: "ctx-b".into(),
        path: std::path::PathBuf::from("/tmp"),
        root: None,
        description: None,
        context_id: ctx_b_id,
        parent_id: None,
        depth: 0,
        parked: false,
    });
    app.windows.push(crate::host::context::Window {
        name: String::new(),
        path: std::path::PathBuf::from("/tmp"),
        tree: egui_tiles::Tree::empty("test_ctx_b"),
        panes: std::collections::HashMap::new(),
        focused_pane: None,
        zoomed_pane: None,
        grid_x: 0,
        grid_y: 0,
        window_id: win_b_id,
        context_id: ctx_b_id,
    });
    app.context_active_window.insert(ctx_b_id, win_b_id);
    let ctx_b_idx = app.router.len() - 1;

    // Switch to context B — this should record (win_a_id, tile_a) in history.
    app.switch_workspace(ctx_b_idx);

    assert!(
        !app.pane_focus_history.is_empty(),
        "focus history must be non-empty after switch_workspace"
    );
    let (recorded_win, recorded_tile) = app.pane_focus_history.last().unwrap();
    assert_eq!(
        *recorded_win, win_a_id,
        "recorded window must be context A's window"
    );
    assert_eq!(
        *recorded_tile, tile_a,
        "recorded tile must be the pane focused in context A"
    );

    // Cmd+[ (step_focus_history_back) must restore focus to context A's window and tile.
    app.step_focus_history_back();
    assert_eq!(
        app.windows[app.active_window].window_id, win_a_id,
        "active window must return to context A after step_focus_history_back"
    );
    assert_eq!(
        app.windows[app.active_window].focused_pane,
        Some(tile_a),
        "focused pane must return to tile_a after step_focus_history_back"
    );

    let _ = ctx_a_id; // suppress unused warning
}

/// Regression guard for #2043: repeated switch_workspace calls do not
/// double-push when the navigating_history guard is active.
#[test]
fn switch_workspace_no_double_push_during_history_navigation() {
    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);

    let (tile_a, _) = app.add_test_pane();
    app.windows[0].focused_pane = Some(tile_a);

    let ctx_b_id = app.next_window_id;
    app.next_window_id += 1;
    let win_b_id = app.next_window_id;
    app.next_window_id += 1;
    app.router.push(crate::host::context::Context {
        name: "ctx-b".into(),
        path: std::path::PathBuf::from("/tmp"),
        root: None,
        description: None,
        context_id: ctx_b_id,
        parent_id: None,
        depth: 0,
        parked: false,
    });
    app.windows.push(crate::host::context::Window {
        name: String::new(),
        path: std::path::PathBuf::from("/tmp"),
        tree: egui_tiles::Tree::empty("test_ctx_b2"),
        panes: std::collections::HashMap::new(),
        focused_pane: None,
        zoomed_pane: None,
        grid_x: 0,
        grid_y: 0,
        window_id: win_b_id,
        context_id: ctx_b_id,
    });
    app.context_active_window.insert(ctx_b_id, win_b_id);
    let ctx_b_idx = app.router.len() - 1;

    app.switch_workspace(ctx_b_idx);
    let history_len_after_switch = app.pane_focus_history.len();

    // Simulate navigating_history = true (as happens inside step_focus_history_back).
    app.navigating_history = true;
    app.switch_workspace(0);
    app.navigating_history = false;

    assert_eq!(
        app.pane_focus_history.len(),
        history_len_after_switch,
        "navigating_history guard must prevent push during history traversal"
    );
}

/// Regression guard for #2054: navigate_to activates ancestor Tabs so focused_pane
/// is never pointing at a tab-hidden tile, and assert_focus_invariants catches
/// zoom/focus desync.
#[test]
fn navigate_to_activates_ancestor_tabs() {
    use egui_tiles::{Container, Tile};
    let egui_ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _tx) = PlexiApp::new_for_test(egui_ctx, frame_tick);

    // Build two panes in a Tabs container.
    let (tile_a, _) = app.add_test_pane();
    let (tile_b, _) = app.add_test_pane();
    let ctx = &mut app.windows[0];
    let tab_tile = ctx.tree.tiles.insert_tab_tile(vec![tile_a, tile_b]);
    ctx.tree.root = Some(tab_tile);
    // Start with tile_a active.
    if let Some(Tile::Container(Container::Tabs(tabs))) = ctx.tree.tiles.get_mut(tab_tile) {
        tabs.set_active(tile_a);
    }
    ctx.focused_pane = Some(tile_a);

    // Navigate to tile_b (which is in the same Tabs but currently inactive).
    app.windows[0].navigate_to(tile_b);

    // focused_pane must be tile_b.
    assert_eq!(app.windows[0].focused_pane, Some(tile_b));

    // The Tabs container must have tile_b as active.
    let ctx = &app.windows[0];
    if let Some(Tile::Container(Container::Tabs(tabs))) = ctx.tree.tiles.get(tab_tile) {
        assert_eq!(
            tabs.active,
            Some(tile_b),
            "Tabs must activate tile_b after navigate_to"
        );
    } else {
        panic!("Expected Tabs container at tab_tile");
    }

    // assert_focus_invariants must not panic when zoomed_pane is None.
    #[cfg(debug_assertions)]
    app.windows[0].assert_focus_invariants();

    // Set a consistent zoom state and verify invariant holds.
    app.windows[0].zoomed_pane = Some(tile_b);
    #[cfg(debug_assertions)]
    app.windows[0].assert_focus_invariants();
}
