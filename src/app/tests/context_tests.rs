use super::super::*;
/// Issue #1392: creating a child context must NOT remove or replace the parent's
/// focused pane (adoption branch was removed). The parent should have:
/// (count_before + 1) panes — its original pane(s) plus the new Portal tile.
#[test]
fn new_child_context_does_not_adopt_focused_pane() {
    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);

    let (tile_id, orig_pane_id) = app.add_test_pane();
    app.windows[0].focused_pane = Some(tile_id);
    let orig_count = app.windows[0].panes.len();
    let parent_id = app.router.active().context_id;
    let parent_name = app.router.active().name.clone();

    app.new_child_context(
        &parent_name,
        std::path::PathBuf::from("/tmp/no_adopt"),
        true,
        false,
        None,
    )
    .expect("child create should succeed");

    let parent_win = app
        .windows
        .iter()
        .find(|w| w.context_id == parent_id)
        .expect("parent window must still exist");

    // Original pane is still present.
    assert!(
        parent_win.panes.contains_key(&orig_pane_id),
        "original focused pane must NOT be adopted away"
    );

    // Pane count grew by exactly 1 (the new Portal tile).
    assert_eq!(
        parent_win.panes.len(),
        orig_count + 1,
        "parent should have orig_count + 1 panes after new_child_context"
    );

    // The new pane is a Portal.
    let has_sub_ctx = parent_win
        .panes
        .values()
        .any(|p| p.portal_target().is_some());
    assert!(has_sub_ctx, "parent must contain a Portal tile");
}

/// When no pane is focused, new_child_context still creates the child context with
/// a fresh terminal. The parent gets a Portal tile inserted as root (no focused
/// split target). The parent's focused_pane remains None.
#[test]
fn new_child_context_no_focused_pane_inserts_sub_ctx() {
    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);

    // Explicitly clear focused pane.
    app.windows[0].focused_pane = None;

    let parent_pane_count_before = app.windows[0].panes.len();
    let parent_id = app.router.active().context_id;

    let result = app.new_child_context(
        "Test",
        std::path::PathBuf::from("/tmp/child2"),
        true,
        false,
        None,
    );

    // Whether success or failure, the parent's focused_pane must remain None.
    assert_eq!(
        app.windows[0].focused_pane, None,
        "parent focused_pane untouched"
    );

    if result.is_ok() {
        // Parent gained exactly 1 Portal pane.
        let parent_win = app
            .windows
            .iter()
            .find(|w| w.context_id == parent_id)
            .unwrap();
        assert_eq!(
            parent_win.panes.len(),
            parent_pane_count_before + 1,
            "parent pane count grew by 1"
        );
        let has_sub_ctx = parent_win
            .panes
            .values()
            .any(|p| p.portal_target().is_some());
        assert!(has_sub_ctx, "parent has a Portal tile");
    } else {
        // PTY failed in test env — parent panes unchanged.
        assert_eq!(
            app.windows[0].panes.len(),
            parent_pane_count_before,
            "parent panes unchanged on failed terminal create"
        );
    }
}

/// `plexi context new --from <id>`: the portal split must anchor at the given
/// pane, not the parent context's focused pane. Two panes exist; A is focused;
/// the anchor names B — the portal must land as B's sibling.
#[test]
fn new_child_context_anchor_pane_overrides_focused() {
    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);

    let (tile_a, _pane_a) = app.add_test_pane();
    let (tile_b, pane_b) = app.add_test_pane();
    app.windows[0].focused_pane = Some(tile_a);
    let parent_id = app.router.active().context_id;
    let parent_name = app.router.active().name.clone();

    app.new_child_context(
        &parent_name,
        std::path::PathBuf::from("/tmp/anchor_pane"),
        true,
        false,
        Some(pane_b),
    )
    .expect("child create should succeed");

    let parent_win = app
        .windows
        .iter()
        .find(|w| w.context_id == parent_id)
        .expect("parent window must still exist");
    let portal_pane_id = parent_win
        .panes
        .iter()
        .find(|(_, p)| p.portal_target().is_some())
        .map(|(id, _)| *id)
        .expect("parent must contain a Portal tile");
    let portal_tile = parent_win
        .tree
        .tiles
        .find_pane(&portal_pane_id)
        .expect("portal tile in tree");
    let container = parent_win
        .tree
        .tiles
        .parent_of(portal_tile)
        .expect("portal tile has a parent container");
    let children: Vec<_> = match parent_win.tree.tiles.get(container) {
        Some(egui_tiles::Tile::Container(c)) => c.children().copied().collect(),
        other => panic!("portal parent is not a container: {other:?}"),
    };
    assert!(
        children.contains(&tile_b),
        "portal must split against the anchor pane's tile"
    );
    assert!(
        !children.contains(&tile_a),
        "portal must NOT split against the focused pane when an anchor is given"
    );
}

/// End-to-end over the real IPC path: the exact JSON `plexi context new --parent`
/// sends (including `anchor_pane`) deserializes into `AppRequest::CreateContext`
/// and the handler anchors the portal at that pane, not the focused one.
#[test]
fn create_context_ipc_anchor_pane_places_portal() {
    let mut h = crate::testing::HostHarness::new();
    let pane_a = h.add_test_pane();
    let pane_b = h.add_test_pane();
    let tile_a = h.app.windows[0].tree.tiles.find_pane(&pane_a).unwrap();
    let tile_b = h.app.windows[0].tree.tiles.find_pane(&pane_b).unwrap();
    h.app.windows[0].focused_pane = Some(tile_a);
    let parent_id = h.app.router.active().context_id;
    let parent_name = h.app.router.active().name.clone();

    let payload = serde_json::json!({
        "type": "create_context",
        "name": "anchored",
        "parent_name": parent_name,
        "anchor_pane": pane_b,
        "portal_direction": "down",
    });
    let req: crate::app_protocol::AppRequest =
        serde_json::from_value(payload).expect("CLI payload must deserialize");
    h.inject_ipc(req);
    h.app.drain_pane_cmd_channel();

    let parent_win = h
        .app
        .windows
        .iter()
        .find(|w| w.context_id == parent_id)
        .expect("parent window must still exist");
    let portal_pane_id = parent_win
        .panes
        .iter()
        .find(|(_, p)| p.portal_target().is_some())
        .map(|(id, _)| *id)
        .expect("handler must insert a Portal tile");
    let portal_tile = parent_win.tree.tiles.find_pane(&portal_pane_id).unwrap();
    let container = parent_win
        .tree
        .tiles
        .parent_of(portal_tile)
        .expect("portal tile has a parent container");
    let children: Vec<_> = match parent_win.tree.tiles.get(container) {
        Some(egui_tiles::Tile::Container(c)) => c.children().copied().collect(),
        other => panic!("portal parent is not a container: {other:?}"),
    };
    assert!(
        children.contains(&tile_b),
        "portal must split against the anchor pane sent over IPC"
    );
    assert!(
        !children.contains(&tile_a),
        "portal must NOT split against the focused pane"
    );
}

/// An anchor pane id that doesn't exist in the parent context falls back to the
/// focused pane — creation must still succeed.
#[test]
fn new_child_context_unknown_anchor_falls_back_to_focused() {
    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);

    let (tile_a, _pane_a) = app.add_test_pane();
    app.windows[0].focused_pane = Some(tile_a);
    let parent_id = app.router.active().context_id;
    let parent_name = app.router.active().name.clone();

    app.new_child_context(
        &parent_name,
        std::path::PathBuf::from("/tmp/anchor_missing"),
        true,
        false,
        Some(999_999),
    )
    .expect("child create should succeed despite unknown anchor");

    let parent_win = app
        .windows
        .iter()
        .find(|w| w.context_id == parent_id)
        .expect("parent window must still exist");
    let portal_pane_id = parent_win
        .panes
        .iter()
        .find(|(_, p)| p.portal_target().is_some())
        .map(|(id, _)| *id)
        .expect("parent must contain a Portal tile");
    let portal_tile = parent_win
        .tree
        .tiles
        .find_pane(&portal_pane_id)
        .expect("portal tile in tree");
    let container = parent_win
        .tree
        .tiles
        .parent_of(portal_tile)
        .expect("portal tile has a parent container");
    let children: Vec<_> = match parent_win.tree.tiles.get(container) {
        Some(egui_tiles::Tile::Container(c)) => c.children().copied().collect(),
        other => panic!("portal parent is not a container: {other:?}"),
    };
    assert!(
        children.contains(&tile_a),
        "unknown anchor must fall back to the focused pane"
    );
}

/// Issue #1409: parent name lookup must be case-insensitive.
#[test]
fn new_child_context_case_insensitive_parent() {
    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);

    // The initial context is named "Test" (from new_for_test).
    // Lookup with lowercase "test" must not return "no context named" error.
    // It may fail for another reason (PTY unavailable in test env), but not lookup.
    let result = app.new_child_context(
        "test",
        std::path::PathBuf::from("/tmp/child_ci"),
        true,
        false,
        None,
    );
    match result {
        Ok(_) => {}
        Err(e) => {
            assert!(
                !e.contains("no context named"),
                "case-insensitive lookup should succeed, got: {e}"
            );
        }
    }
}

/// Issue #1409: creating a child context with a parent should auto-zoom into it.
/// Note: new_child_context always creates a fresh terminal (portal model, no adoption).
/// In test environments without a PTY, new_child_context returns Err — skip the zoom
/// assertion in that case and focus only on router state (push_depth) which is caller-side.
#[test]
fn create_child_context_auto_zooms() {
    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);

    let parent_ctx_id = app.router.active().context_id;
    let current_win_id = app.windows[app.active_window].window_id;
    let current_focused = app.windows[app.active_window].focused_pane;

    // Simulate what the CreateContext handler does: capture current state,
    // call new_child_context, then push_depth + switch_workspace.
    let result = app.new_child_context(
        "Test",
        std::path::PathBuf::from("/tmp/child_zoom"),
        true,
        false,
        None,
    );

    if result.is_err() {
        // PTY unavailable in test env — verify caller-side depth push still works.
        app.router
            .push_depth(parent_ctx_id, current_win_id, current_focused);
        assert_eq!(
            app.router.current_depth(),
            1,
            "depth stack grows even on Err"
        );
        return;
    }

    let new_ctx_idx = app.router.len() - 1;
    let new_ctx_id = app.router.get(new_ctx_idx).context_id;
    app.router
        .push_depth(parent_ctx_id, current_win_id, current_focused);
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

/// Issue #1833: new_child_context_from_keyboard creates a child of the active context
/// and auto-zooms into it (push_depth + switch_workspace). In PTY-less test environments
/// the terminal creation fails — verify the depth stack is unchanged and the active context
/// is unmodified in that case.
#[test]
fn new_child_context_from_keyboard_zooms_into_child() {
    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);

    let parent_ctx_id = app.router.active().context_id;
    let initial_depth = app.router.current_depth();
    let initial_ctx_count = app.router.len();

    app.new_child_context_from_keyboard();

    if app.router.len() == initial_ctx_count {
        // PTY unavailable — no child created, state unchanged.
        assert_eq!(
            app.router.current_depth(),
            initial_depth,
            "depth stack must be unchanged when child creation fails"
        );
        assert_eq!(
            app.router.active().context_id,
            parent_ctx_id,
            "active context must not change when child creation fails"
        );
    } else {
        // Child was created and we zoomed in.
        assert_eq!(
            app.router.len(),
            initial_ctx_count + 1,
            "exactly one new context added"
        );
        assert_ne!(
            app.router.active().context_id,
            parent_ctx_id,
            "active context must switch to the new child"
        );
        assert_eq!(
            app.router.active().parent_id,
            Some(parent_ctx_id),
            "child's parent_id must be the original context"
        );
        assert_eq!(
            app.router.current_depth(),
            initial_depth + 1,
            "depth stack must grow by one after auto-zoom"
        );
    }
}

/// Issue #1392: unlimited nesting after killing the depth cap and adoption branch.
/// Build a 4-level chain (root → A → B → C → D) and verify that each parent's
/// window contains a Portal tile pointing at the corresponding child.
/// Note: new_child_context always creates a fresh terminal. In test envs without PTY
/// this returns Err — if so, verify depth metadata still incremented correctly for
/// any contexts that were created before the first failure.
#[test]
fn depth_four_chain_has_portal_tiles() {
    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);

    // Rename the initial context to "Root" for predictability.
    let root_idx = app.router.active_idx();
    app.router.get_mut(root_idx).name = "Root".to_string();

    let names = ["A", "B", "C", "D"];
    let mut parent_name = "Root".to_string();
    let mut chain_ids: Vec<u64> = vec![app.router.active().context_id];

    for &child in &names {
        let path = std::path::PathBuf::from(format!("/tmp/depth_test_{child}"));
        let result = app.new_child_context(&parent_name, path, true, false, None);
        if result.is_err() {
            // PTY unavailable — can't build the full chain in test env. Stop here.
            break;
        }
        let new_idx = app.router.len() - 1;
        app.router.get_mut(new_idx).name = child.to_string();
        chain_ids.push(app.router.get(new_idx).context_id);
        parent_name = child.to_string();
    }

    // Verify depth metadata for whatever was created.
    for (level, &cid) in chain_ids.iter().enumerate() {
        let c = app.router.iter().find(|c| c.context_id == cid).unwrap();
        assert_eq!(
            c.depth as usize, level,
            "context at chain[{level}] should have depth {level}"
        );
    }

    // Verify each parent's window has a Portal tile pointing at its child.
    for i in 0..chain_ids.len().saturating_sub(1) {
        let parent_id = chain_ids[i];
        let child_id = chain_ids[i + 1];
        let parent_win = app
            .windows
            .iter()
            .find(|w| w.context_id == parent_id)
            .expect("parent window must exist");
        let found = parent_win
            .panes
            .values()
            .any(|p| p.portal_target() == Some(child_id));
        assert!(
            found,
            "parent ctx_id={parent_id} must contain a Portal tile pointing at child {child_id}"
        );
    }
}

/// Issue #1854: closing a context portal must reset the parent window's focused_pane
/// if it was pointing at the now-deleted Portal tile. Otherwise focus is permanently lost.
#[test]
fn delete_context_portal_resets_stale_focused_pane() {
    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);

    // Add a real pane to the root window and record its tile ID.
    let (root_pane_tile, _root_pane_id) = app.add_test_pane();
    let root_id = app.router.active().context_id;

    // Create a child context.
    let child_id = 9901u64;
    let child_win_id = 9902u64;
    app.router.push(crate::host::context::Context {
        name: "Child".to_string(),
        path: std::path::PathBuf::from("/tmp/child_1854"),
        root: None,
        description: None,
        context_id: child_id,
        parent_id: Some(root_id),
        depth: 1,
        parked: false,
    });
    app.context_active_window.insert(child_id, child_win_id);
    {
        let mut tiles = egui_tiles::Tiles::default();
        let r = tiles.insert_pane(99991u64);
        app.windows.push(crate::host::context::Window {
            name: String::new(),
            path: std::path::PathBuf::from("/tmp/child_1854"),
            tree: egui_tiles::Tree::new("child_1854", r, tiles),
            panes: std::collections::HashMap::new(),
            focused_pane: None,
            zoomed_pane: None,
            grid_x: 0,
            grid_y: 0,
            window_id: child_win_id,
            context_id: child_id,
        });
    }

    // Insert a Portal pane into the root window pointing at the child context.
    let portal_pane_id = 88801u64;
    let portal_tile;
    {
        let root_win = &mut app.windows[0];
        portal_tile = root_win.tree.tiles.insert_pane(portal_pane_id);
        root_win.panes.insert(
            portal_pane_id,
            crate::host::pane::Pane::Portal(Box::new(crate::host::pane::PortalPane {
                pane_id: portal_pane_id,
                target_context_id: child_id,
                context_state: None,
                hidden: false,
            })),
        );
    }

    // Simulate focus being on the Portal tile (the buggy pre-delete state).
    app.windows[0].focused_pane = Some(portal_tile);

    // Delete the child context — this removes the Portal tile from the root window.
    let child_idx = app.router.position(|c| c.context_id == child_id).unwrap();
    app.delete_context(child_idx);

    // focused_pane must NOT still point to the now-deleted Portal tile.
    let root_win = app
        .windows
        .iter()
        .find(|w| w.context_id == root_id)
        .unwrap();
    assert_ne!(
        root_win.focused_pane,
        Some(portal_tile),
        "focused_pane must not point to the deleted Portal tile"
    );
    // And it must be reset to the surviving pane.
    assert_eq!(
        root_win.focused_pane,
        Some(root_pane_tile),
        "focused_pane must be reset to the surviving root pane tile"
    );
}

/// Issue #1392: deleting a context must cascade to all descendants AND
/// clean up router.depth_stack entries pointing to deleted contexts.
#[test]
fn delete_context_cascades_and_cleans_depth_stack() {
    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);

    let root_idx = app.router.active_idx();
    app.router.get_mut(root_idx).name = "Root".to_string();
    let root_id = app.router.active().context_id;

    // Manually insert child context A and grandchild B without PTY.
    // We insert them directly into the router/windows to avoid PTY dependency.
    let a_id = 9001u64;
    let b_id = 9002u64;
    let a_win_id = 9003u64;
    let b_win_id = 9004u64;

    app.router.push(crate::host::context::Context {
        name: "A".to_string(),
        path: std::path::PathBuf::from("/tmp/a"),
        root: None,
        description: None,
        context_id: a_id,
        parent_id: Some(root_id),
        depth: 1,
        parked: false,
    });
    app.router.push(crate::host::context::Context {
        name: "B".to_string(),
        path: std::path::PathBuf::from("/tmp/b"),
        root: None,
        description: None,
        context_id: b_id,
        parent_id: Some(a_id),
        depth: 2,
        parked: false,
    });

    // Push minimal windows for A and B without PTY dependency.
    app.context_active_window.insert(a_id, a_win_id);
    app.context_active_window.insert(b_id, b_win_id);
    {
        let mut tiles_a = egui_tiles::Tiles::default();
        let r_a = tiles_a.insert_pane(88881);
        app.windows.push(crate::host::context::Window {
            name: String::new(),
            path: std::path::PathBuf::from("/tmp/a"),
            tree: egui_tiles::Tree::new("plexi_a", r_a, tiles_a),
            panes: std::collections::HashMap::new(),
            focused_pane: None,
            zoomed_pane: None,
            grid_x: 0,
            grid_y: 0,
            window_id: a_win_id,
            context_id: a_id,
        });
    }
    {
        let mut tiles_b = egui_tiles::Tiles::default();
        let r_b = tiles_b.insert_pane(88882);
        app.windows.push(crate::host::context::Window {
            name: String::new(),
            path: std::path::PathBuf::from("/tmp/b"),
            tree: egui_tiles::Tree::new("plexi_b", r_b, tiles_b),
            panes: std::collections::HashMap::new(),
            focused_pane: None,
            zoomed_pane: None,
            grid_x: 0,
            grid_y: 0,
            window_id: b_win_id,
            context_id: b_id,
        });
    }

    // Simulate user zoomed Root → A → B.
    app.router.push_depth(root_id, 0, None);
    app.router.push_depth(a_id, 0, None);
    assert_eq!(app.router.current_depth(), 2);

    // Delete A (find it by id).
    let a_idx_now = app.router.position(|c| c.context_id == a_id).unwrap();
    app.delete_context(a_idx_now);

    // A and B should both be gone from the router.
    assert!(
        app.router.iter().find(|c| c.context_id == a_id).is_none(),
        "A should be deleted"
    );
    assert!(
        app.router.iter().find(|c| c.context_id == b_id).is_none(),
        "B should be cascade-deleted"
    );

    // Depth stack should no longer contain A or B.
    assert!(
        app.router
            .depth_stack
            .iter()
            .all(|(cid, _, _)| *cid != a_id),
        "depth_stack must not contain deleted ctx_id={a_id}"
    );
    assert!(
        app.router
            .depth_stack
            .iter()
            .all(|(cid, _, _)| *cid != b_id),
        "depth_stack must not contain deleted ctx_id={b_id}"
    );

    // No Portal tile pointing to A or B should remain in any window.
    for win in &app.windows {
        for pane in win.panes.values() {
            if let Some(target) = pane.portal_target() {
                assert_ne!(target, a_id, "stale Portal tile pointing to deleted A");
                assert_ne!(target, b_id, "stale Portal tile pointing to deleted B");
            }
        }
    }
}

/// Issue #1801: context transition must rescan the registry and restart the watcher.
///
/// Creates two temp roots each with a distinct workspace-local app, switches the
/// active context between them, and asserts that the registry always reflects the
/// current root — no stale apps from the previous root survive the switch.
#[test]
fn context_transition_rescans_registry() {
    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);

    // Two isolated temp roots with different workspace-local apps.
    let dir_a = std::env::temp_dir().join("plexi_test_1801_ctx_a");
    let dir_b = std::env::temp_dir().join("plexi_test_1801_ctx_b");
    // Clean up from any prior failed run before creating fixtures.
    let _ = std::fs::remove_dir_all(&dir_a);
    let _ = std::fs::remove_dir_all(&dir_b);

    let manifest_a = concat!(
        "schema_version = 1\n",
        "[app]\n",
        "id = \"test-1801-app-a\"\n",
        "name = \"Test App A\"\n",
        "entry = \"app.py\"\n",
        "type = \"app\"\n",
    );
    let manifest_b = concat!(
        "schema_version = 1\n",
        "[app]\n",
        "id = \"test-1801-app-b\"\n",
        "name = \"Test App B\"\n",
        "entry = \"app.py\"\n",
        "type = \"app\"\n",
    );

    let apps_a = crate::app::registry::workspace_apps_dir(&dir_a);
    std::fs::create_dir_all(apps_a.join("test-1801-app-a")).unwrap();
    std::fs::write(
        apps_a.join("test-1801-app-a").join("manifest.toml"),
        manifest_a,
    )
    .unwrap();
    std::fs::write(apps_a.join("test-1801-app-a").join("app.py"), b"").unwrap();

    let apps_b = crate::app::registry::workspace_apps_dir(&dir_b);
    std::fs::create_dir_all(apps_b.join("test-1801-app-b")).unwrap();
    std::fs::write(
        apps_b.join("test-1801-app-b").join("manifest.toml"),
        manifest_b,
    )
    .unwrap();
    std::fs::write(apps_b.join("test-1801-app-b").join("app.py"), b"").unwrap();

    // Switch context 0 to root A and verify registry picks up app-a.
    let idx0 = app.router.active_idx();
    app.router.get_mut(idx0).root = Some(dir_a.clone());
    app.apply_context_transition_effects();

    let ids: Vec<String> = app
        .registry
        .list()
        .into_iter()
        .map(|a| a.manifest.id.clone())
        .collect();
    assert!(
        ids.contains(&"test-1801-app-a".to_string()),
        "registry should contain test-1801-app-a after setting root A, got: {ids:?}"
    );
    assert!(
        !ids.contains(&"test-1801-app-b".to_string()),
        "registry should not contain test-1801-app-b while on root A, got: {ids:?}"
    );

    // Add a second context pointing at root B and switch to it.
    let ctx_b_id = app.next_window_id;
    app.next_window_id += 1;
    let win_b_id = app.next_window_id;
    app.next_window_id += 1;
    app.router.push(crate::host::context::Context {
        name: "Context B".into(),
        path: dir_b.clone(),
        root: Some(dir_b.clone()),
        description: None,
        context_id: ctx_b_id,
        parent_id: None,
        depth: 0,
        parked: false,
    });
    app.windows.push(Window {
        name: "Context B".into(),
        path: dir_b.clone(),
        tree: egui_tiles::Tree::empty("test_tree_b"),
        panes: HashMap::new(),
        focused_pane: None,
        zoomed_pane: None,
        grid_x: 0,
        grid_y: 0,
        window_id: win_b_id,
        context_id: ctx_b_id,
    });
    let ctx_b_idx = app.router.len() - 1;
    app.switch_workspace(ctx_b_idx);

    let ids: Vec<String> = app
        .registry
        .list()
        .into_iter()
        .map(|a| a.manifest.id.clone())
        .collect();
    assert!(
        ids.contains(&"test-1801-app-b".to_string()),
        "registry should contain test-1801-app-b after switching to root B, got: {ids:?}"
    );
    assert!(
        !ids.contains(&"test-1801-app-a".to_string()),
        "registry should not contain test-1801-app-a while on root B, got: {ids:?}"
    );

    let _ = std::fs::remove_dir_all(&dir_a);
    let _ = std::fs::remove_dir_all(&dir_b);
}

/// Issue #1912: `new_context` always creates a new top-level empty context.
#[test]
fn new_context_creates_top_level_empty_context() {
    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);

    let original_ctx_id = app.router.active().context_id;

    let (_tile_id, _pane_id) = app.add_test_pane();
    app.windows[0].focused_pane = Some(_tile_id);

    assert_eq!(app.router.len(), 1, "setup: 1 context before new_context");
    assert_eq!(
        app.windows[0].panes.len(),
        1,
        "setup: 1 pane before new_context"
    );

    app.new_context();

    // PTY may be unavailable in test env -- new_context_empty returns early.
    if app.router.len() == 1 {
        return;
    }

    assert_eq!(app.router.len(), 2, "new top-level context registered");

    let new_ctx_id = app.router.active().context_id;
    assert_ne!(
        new_ctx_id, original_ctx_id,
        "active context switched to new one"
    );

    // No depth change -- new context is top-level, not a child.
    assert_eq!(app.router.current_depth(), 0, "depth stack unchanged");

    // New context has depth 0 and no parent.
    let new_ctx = app.router.active();
    assert_eq!(new_ctx.depth, 0, "new context is top-level");
    assert!(new_ctx.parent_id.is_none(), "new context has no parent");

    // Original context's panes are untouched.
    let orig_win_idx = app
        .windows
        .iter()
        .position(|w| w.context_id == original_ctx_id)
        .expect("original context still has a window");
    assert_eq!(
        app.windows[orig_win_idx].panes.len(),
        1,
        "original panes untouched"
    );
    assert!(
        app.windows[orig_win_idx].panes.contains_key(&_pane_id),
        "original pane still present"
    );

    // Inline rename was opened for the new context.
    assert_eq!(
        app.renaming_window,
        Some(app.router.len() - 1),
        "inline rename opened for new context"
    );
}

/// Issue #2029: closing a sub-context portal that is the sole pane on a window must
/// delete that window. Previously the window survived empty, showing the welcome screen.
#[test]
fn delete_context_collapses_empty_portal_window() {
    use std::collections::HashMap;

    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);

    // Window 0 already exists (the initial window for the root context).
    let root_id = app.router.active().context_id;
    let (root_tile, _root_pane_id) = app.add_test_pane();
    app.windows[0].focused_pane = Some(root_tile);

    // Add a second window to the root context (simulates a two-page layout).
    let win2_id = 77701u64;
    {
        let mut tiles = egui_tiles::Tiles::default();
        // Intentionally no real pane tile yet — the portal will be inserted below.
        let placeholder = tiles.insert_pane(77702u64);
        app.windows.push(Window {
            name: String::new(),
            path: std::path::PathBuf::from("/tmp/test_2029"),
            tree: egui_tiles::Tree::new("test_2029_win2", placeholder, tiles),
            panes: HashMap::new(),
            focused_pane: None,
            zoomed_pane: None,
            grid_x: 1,
            grid_y: 0,
            window_id: win2_id,
            context_id: root_id,
        });
    }

    // Register a child context (no PTY needed — manual insertion).
    let child_id = 77710u64;
    let child_win_id = 77711u64;
    app.router.push(crate::host::context::Context {
        name: "Child2029".to_string(),
        path: std::path::PathBuf::from("/tmp/test_2029_child"),
        root: None,
        description: None,
        context_id: child_id,
        parent_id: Some(root_id),
        depth: 1,
        parked: false,
    });
    app.context_active_window.insert(child_id, child_win_id);
    {
        let mut tiles = egui_tiles::Tiles::default();
        let r = tiles.insert_pane(77712u64);
        app.windows.push(Window {
            name: String::new(),
            path: std::path::PathBuf::from("/tmp/test_2029_child"),
            tree: egui_tiles::Tree::new("test_2029_child", r, tiles),
            panes: HashMap::new(),
            focused_pane: None,
            zoomed_pane: None,
            grid_x: 0,
            grid_y: 0,
            window_id: child_win_id,
            context_id: child_id,
        });
    }

    // Insert a Portal pane into window 1 (the second root window) as its ONLY pane.
    let portal_pane_id = 77720u64;
    {
        // Replace the placeholder pane map entry with the portal.
        let win2 = app
            .windows
            .iter_mut()
            .find(|w| w.window_id == win2_id)
            .unwrap();
        // Clear the placeholder from the pane map (tree already has the tile; pane map is separate).
        // Insert the portal pane.
        let portal_tile = win2.tree.tiles.insert_pane(portal_pane_id);
        win2.tree.root = Some(portal_tile);
        win2.panes.clear();
        win2.panes.insert(
            portal_pane_id,
            crate::host::pane::Pane::Portal(Box::new(crate::host::pane::PortalPane {
                pane_id: portal_pane_id,
                target_context_id: child_id,
                context_state: None,
                hidden: false,
            })),
        );
        win2.focused_pane = Some(portal_tile);
    }

    // Preconditions.
    assert_eq!(
        app.windows
            .iter()
            .filter(|w| w.context_id == root_id)
            .count(),
        2,
        "setup: root context has 2 windows"
    );

    // Delete the child context (removes the portal from win2, leaving it empty).
    let child_idx = app.router.position(|c| c.context_id == child_id).unwrap();
    app.delete_context(child_idx);

    // win2 must be gone — root context now has exactly 1 window.
    let root_windows: Vec<_> = app
        .windows
        .iter()
        .filter(|w| w.context_id == root_id)
        .collect();
    assert_eq!(
        root_windows.len(),
        1,
        "empty portal window must be deleted after delete_context"
    );

    // The surviving window still has its pane.
    assert!(
        root_windows[0].panes.contains_key(&_root_pane_id),
        "original root pane must survive"
    );
}

/// Issue #2029 (edge case): when ALL windows in a context each contain only a portal
/// to the deleted child, none has a non-empty sibling — so ALL must be preserved.
/// Deleting all of them would strip the context of every window, violating the invariant.
#[test]
fn delete_context_keeps_all_empty_windows_when_no_nonempty_sibling() {
    use std::collections::HashMap;

    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);

    let root_id = app.router.active().context_id;
    // Window 0 already exists for root_id but has NO real pane — it will also be empty after portal removal.
    // Add window 1 to root_id; both will have only a portal pane.
    let win2_id = 88801u64;
    {
        let mut tiles = egui_tiles::Tiles::default();
        let placeholder = tiles.insert_pane(88802u64);
        app.windows.push(Window {
            name: String::new(),
            path: std::path::PathBuf::from("/tmp/test_2029_edge"),
            tree: egui_tiles::Tree::new("test_2029_edge_win2", placeholder, tiles),
            panes: HashMap::new(),
            focused_pane: None,
            zoomed_pane: None,
            grid_x: 1,
            grid_y: 0,
            window_id: win2_id,
            context_id: root_id,
        });
    }

    // Child context.
    let child_id = 88810u64;
    let child_win_id = 88811u64;
    app.router.push(crate::host::context::Context {
        name: "ChildEdge".to_string(),
        path: std::path::PathBuf::from("/tmp/test_2029_edge_child"),
        root: None,
        description: None,
        context_id: child_id,
        parent_id: Some(root_id),
        depth: 1,
        parked: false,
    });
    app.context_active_window.insert(child_id, child_win_id);
    {
        let mut tiles = egui_tiles::Tiles::default();
        let r = tiles.insert_pane(88812u64);
        app.windows.push(Window {
            name: String::new(),
            path: std::path::PathBuf::from("/tmp/test_2029_edge_child"),
            tree: egui_tiles::Tree::new("test_2029_edge_child", r, tiles),
            panes: HashMap::new(),
            focused_pane: None,
            zoomed_pane: None,
            grid_x: 0,
            grid_y: 0,
            window_id: child_win_id,
            context_id: child_id,
        });
    }

    // Insert a portal into window 0 (initially has no panes) and window 1 — both portal-only.
    for (win_id, portal_pane_id) in [(app.windows[0].window_id, 88820u64), (win2_id, 88821u64)] {
        let win = app
            .windows
            .iter_mut()
            .find(|w| w.window_id == win_id)
            .unwrap();
        let portal_tile = win.tree.tiles.insert_pane(portal_pane_id);
        win.tree.root = Some(portal_tile);
        win.panes.clear();
        win.panes.insert(
            portal_pane_id,
            crate::host::pane::Pane::Portal(Box::new(crate::host::pane::PortalPane {
                pane_id: portal_pane_id,
                target_context_id: child_id,
                context_state: None,
                hidden: false,
            })),
        );
    }

    assert_eq!(
        app.windows
            .iter()
            .filter(|w| w.context_id == root_id)
            .count(),
        2,
        "setup: 2 windows in root context, both portal-only"
    );

    let child_idx = app.router.position(|c| c.context_id == child_id).unwrap();
    app.delete_context(child_idx);

    // Both windows became empty but neither had a non-empty sibling — both must survive.
    // The root context must still have windows (invariant: context always has >= 1 window).
    let root_windows = app
        .windows
        .iter()
        .filter(|w| w.context_id == root_id)
        .count();
    assert!(
        root_windows >= 1,
        "root context must retain at least 1 window; got {root_windows}"
    );
    // Specifically: neither empty window had a non-empty sibling, so both are kept.
    assert_eq!(
        root_windows, 2,
        "both portal-only windows must survive when no non-empty sibling exists"
    );
}

fn test_app_pane(pane_id: u64) -> crate::host::pane::Pane {
    use crate::app::permissions::AppPermissions;
    use crate::host::pane::{AppPane, AppRuntime};
    use crate::process_app::ProcessApp;

    let permissions = AppPermissions::builtin();
    let (process_app, _draw_tx) = ProcessApp::new_for_test(pane_id, permissions.clone());
    crate::host::pane::Pane::App(Box::new(AppPane {
        id: pane_id,
        runtime: AppRuntime::Process(Box::new(process_app)),
        workspace_root: std::env::temp_dir(),
        permissions,
        manifest_id: format!("test-{pane_id}"),
        name: format!("Test App {pane_id}"),
        pane_group: None,
        linked_pane_id: None,
        overlay_replaced: None,
        hidden: false,
        agent: None,
        slots: std::collections::HashMap::new(),
    }))
}

fn test_context(id: u64, parent_id: u64, name: &str) -> crate::host::context::Context {
    crate::host::context::Context {
        name: name.to_string(),
        path: std::path::PathBuf::from(format!("/tmp/{name}")),
        root: None,
        description: None,
        context_id: id,
        parent_id: Some(parent_id),
        depth: 1,
        parked: false,
    }
}

/// Issue #2108: dissolving a one-window sub-context should graft that child's
/// tile tree into the exact Portal slot instead of flattening child panes into
/// the parent container.
#[test]
fn dissolve_portal_grafts_single_child_window_tree_in_place() {
    use egui_tiles::{Container, LinearDir, Tile};
    use std::collections::HashMap;

    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);

    let parent_ctx_id = app.router.active().context_id;
    let parent_win_id = app.windows[0].window_id;
    let child_ctx_id = 210810u64;
    let child_win_id = 210811u64;

    let parent_left = 210801u64;
    let parent_top_right = 210802u64;
    let portal_pane_id = 210803u64;
    let child_left = 210804u64;
    let child_right = 210805u64;

    let mut parent_tiles = egui_tiles::Tiles::default();
    let parent_left_tile = parent_tiles.insert_pane(parent_left);
    let parent_top_right_tile = parent_tiles.insert_pane(parent_top_right);
    let portal_tile = parent_tiles.insert_pane(portal_pane_id);
    let right_col = parent_tiles.insert_vertical_tile(vec![parent_top_right_tile, portal_tile]);
    let parent_root = parent_tiles.insert_horizontal_tile(vec![parent_left_tile, right_col]);
    let mut parent_panes = HashMap::new();
    parent_panes.insert(parent_left, test_app_pane(parent_left));
    parent_panes.insert(parent_top_right, test_app_pane(parent_top_right));
    parent_panes.insert(
        portal_pane_id,
        crate::host::pane::Pane::Portal(Box::new(crate::host::pane::PortalPane {
            pane_id: portal_pane_id,
            target_context_id: child_ctx_id,
            context_state: None,
            hidden: false,
        })),
    );
    app.windows[0].tree =
        egui_tiles::Tree::new("dissolve_single_parent", parent_root, parent_tiles);
    app.windows[0].panes = parent_panes;
    app.windows[0].focused_pane = Some(portal_tile);
    app.windows[0].zoomed_pane = Some(portal_tile);

    app.router.push(test_context(
        child_ctx_id,
        parent_ctx_id,
        "dissolve_single_child",
    ));
    app.context_active_window.insert(child_ctx_id, child_win_id);

    let mut child_tiles = egui_tiles::Tiles::default();
    let child_left_tile = child_tiles.insert_pane(child_left);
    let child_right_tile = child_tiles.insert_pane(child_right);
    let child_root = child_tiles.insert_horizontal_tile(vec![child_left_tile, child_right_tile]);
    let mut child_panes = HashMap::new();
    child_panes.insert(child_left, test_app_pane(child_left));
    child_panes.insert(child_right, test_app_pane(child_right));
    app.windows.push(Window {
        name: "child".to_string(),
        path: std::path::PathBuf::from("/tmp/dissolve_single_child"),
        tree: egui_tiles::Tree::new("dissolve_single_child", child_root, child_tiles),
        panes: child_panes,
        focused_pane: Some(child_right_tile),
        zoomed_pane: Some(child_right_tile),
        grid_x: 0,
        grid_y: 0,
        window_id: child_win_id,
        context_id: child_ctx_id,
    });
    app.router
        .push_depth(child_ctx_id, child_win_id, Some(child_right_tile));

    app.dissolve_portal(child_ctx_id);

    assert!(
        app.router.iter().all(|ctx| ctx.context_id != child_ctx_id),
        "dissolved child context must be removed from router"
    );
    assert!(
        !app.context_active_window.contains_key(&child_ctx_id),
        "dissolved child context must not keep an active-window entry"
    );
    assert!(
        app.router
            .depth_stack
            .iter()
            .all(|(ctx_id, _, _)| *ctx_id != child_ctx_id),
        "dissolved child context must be removed from depth stack"
    );
    assert!(
        app.windows.iter().all(|w| w.context_id != child_ctx_id),
        "no window should still belong to the dissolved child context"
    );

    let parent = app
        .windows
        .iter()
        .find(|w| w.window_id == parent_win_id)
        .expect("parent window should survive");
    assert!(parent.panes.contains_key(&parent_left));
    assert!(parent.panes.contains_key(&parent_top_right));
    assert!(parent.panes.contains_key(&child_left));
    assert!(parent.panes.contains_key(&child_right));
    assert!(
        !parent.panes.contains_key(&portal_pane_id),
        "Portal pane must be removed after dissolve"
    );
    assert!(
        parent
            .panes
            .values()
            .all(|pane| pane.portal_target() != Some(child_ctx_id)),
        "no surviving PortalPane may point at the dissolved context"
    );

    let grafted_tile = match parent.tree.tiles.get(right_col) {
        Some(Tile::Container(Container::Linear(linear))) => {
            assert_eq!(
                linear.dir,
                LinearDir::Vertical,
                "portal lived in the lower half of the right column"
            );
            assert_eq!(
                linear.children.len(),
                2,
                "child split should replace the portal slot, not become extra siblings"
            );
            linear.children[1]
        }
        other => panic!("expected right column linear container, got {other:?}"),
    };

    match parent.tree.tiles.get(grafted_tile) {
        Some(Tile::Container(Container::Linear(linear))) => {
            assert_eq!(linear.dir, LinearDir::Horizontal);
            let grafted_panes: Vec<_> = linear
                .children
                .iter()
                .map(|tile| match parent.tree.tiles.get(*tile) {
                    Some(Tile::Pane(pane_id)) => *pane_id,
                    other => panic!("expected child pane tile, got {other:?}"),
                })
                .collect();
            assert_eq!(
                grafted_panes,
                vec![child_left, child_right],
                "grafted tile must preserve the child split order"
            );
        }
        other => panic!("expected child split grafted into portal slot, got {other:?}"),
    }

    let mapped_child_focus = parent
        .tree
        .tiles
        .find_pane(&child_right)
        .expect("focused child pane should be present in graft");
    assert_eq!(
        parent.focused_pane,
        Some(mapped_child_focus),
        "child focus should map to the grafted tile"
    );
    assert_eq!(
        parent.zoomed_pane,
        Some(mapped_child_focus),
        "child zoom should map to the grafted tile"
    );
}

/// Issue #2108: a multi-window child context must not be flattened into the
/// active parent window. The active child window is grafted into the Portal
/// slot; the remaining child windows are promoted to parent-context windows.
#[test]
fn dissolve_portal_preserves_multi_window_child_boundaries() {
    use egui_tiles::{Container, Tile};
    use std::collections::{HashMap, HashSet};

    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);

    let parent_ctx_id = app.router.active().context_id;
    let parent_win_id = app.windows[0].window_id;
    let child_ctx_id = 210860u64;
    let child_win_a = 210861u64;
    let child_win_primary = 210862u64;
    let child_win_c = 210863u64;

    let parent_pane = 210850u64;
    let portal_pane_id = 210851u64;
    let secondary_a_pane = 210852u64;
    let primary_left = 210853u64;
    let primary_right = 210854u64;
    let secondary_c_pane = 210855u64;

    let mut parent_tiles = egui_tiles::Tiles::default();
    let parent_tile = parent_tiles.insert_pane(parent_pane);
    let portal_tile = parent_tiles.insert_pane(portal_pane_id);
    let parent_root = parent_tiles.insert_horizontal_tile(vec![parent_tile, portal_tile]);
    let mut parent_panes = HashMap::new();
    parent_panes.insert(parent_pane, test_app_pane(parent_pane));
    parent_panes.insert(
        portal_pane_id,
        crate::host::pane::Pane::Portal(Box::new(crate::host::pane::PortalPane {
            pane_id: portal_pane_id,
            target_context_id: child_ctx_id,
            context_state: None,
            hidden: false,
        })),
    );
    app.windows[0].tree = egui_tiles::Tree::new("dissolve_multi_parent", parent_root, parent_tiles);
    app.windows[0].panes = parent_panes;
    app.windows[0].focused_pane = Some(portal_tile);
    app.windows[0].grid_x = 0;
    app.windows[0].grid_y = 0;

    app.router.push(test_context(
        child_ctx_id,
        parent_ctx_id,
        "dissolve_multi_child",
    ));
    app.context_active_window
        .insert(child_ctx_id, child_win_primary);

    let mut tiles_a = egui_tiles::Tiles::default();
    let tile_a = tiles_a.insert_pane(secondary_a_pane);
    let mut panes_a = HashMap::new();
    panes_a.insert(secondary_a_pane, test_app_pane(secondary_a_pane));
    app.windows.push(Window {
        name: "secondary-a".to_string(),
        path: std::path::PathBuf::from("/tmp/dissolve_multi_a"),
        tree: egui_tiles::Tree::new("dissolve_multi_a", tile_a, tiles_a),
        panes: panes_a,
        focused_pane: Some(tile_a),
        zoomed_pane: None,
        grid_x: 0,
        grid_y: 0,
        window_id: child_win_a,
        context_id: child_ctx_id,
    });

    let mut tiles_primary = egui_tiles::Tiles::default();
    let primary_left_tile = tiles_primary.insert_pane(primary_left);
    let primary_right_tile = tiles_primary.insert_pane(primary_right);
    let primary_root =
        tiles_primary.insert_horizontal_tile(vec![primary_left_tile, primary_right_tile]);
    let mut panes_primary = HashMap::new();
    panes_primary.insert(primary_left, test_app_pane(primary_left));
    panes_primary.insert(primary_right, test_app_pane(primary_right));
    app.windows.push(Window {
        name: "primary".to_string(),
        path: std::path::PathBuf::from("/tmp/dissolve_multi_primary"),
        tree: egui_tiles::Tree::new("dissolve_multi_primary", primary_root, tiles_primary),
        panes: panes_primary,
        focused_pane: Some(primary_right_tile),
        zoomed_pane: Some(primary_right_tile),
        grid_x: 1,
        grid_y: 0,
        window_id: child_win_primary,
        context_id: child_ctx_id,
    });

    let mut tiles_c = egui_tiles::Tiles::default();
    let tile_c = tiles_c.insert_pane(secondary_c_pane);
    let mut panes_c = HashMap::new();
    panes_c.insert(secondary_c_pane, test_app_pane(secondary_c_pane));
    app.windows.push(Window {
        name: "secondary-c".to_string(),
        path: std::path::PathBuf::from("/tmp/dissolve_multi_c"),
        tree: egui_tiles::Tree::new("dissolve_multi_c", tile_c, tiles_c),
        panes: panes_c,
        focused_pane: Some(tile_c),
        zoomed_pane: None,
        grid_x: 1,
        grid_y: 0,
        window_id: child_win_c,
        context_id: child_ctx_id,
    });

    app.minimap.visible = false;
    app.minimap_visible_per_context.insert(parent_ctx_id, false);

    app.dissolve_portal(child_ctx_id);

    assert!(
        app.router.iter().all(|ctx| ctx.context_id != child_ctx_id),
        "dissolved child context must be removed from router"
    );
    assert!(
        !app.context_active_window.contains_key(&child_ctx_id),
        "dissolved child context must not keep an active-window entry"
    );
    assert!(
        app.windows.iter().all(|w| w.context_id != child_ctx_id),
        "all child windows should be reparented or removed"
    );

    let active_parent = app
        .windows
        .iter()
        .find(|w| w.window_id == parent_win_id)
        .expect("parent window should survive");
    assert!(active_parent.panes.contains_key(&parent_pane));
    assert!(active_parent.panes.contains_key(&primary_left));
    assert!(active_parent.panes.contains_key(&primary_right));
    assert!(
        !active_parent.panes.contains_key(&secondary_a_pane)
            && !active_parent.panes.contains_key(&secondary_c_pane),
        "secondary child windows must not be flattened into the active parent window"
    );
    assert!(
        !active_parent.panes.contains_key(&portal_pane_id),
        "Portal pane must be removed after dissolve"
    );
    let portal_slot = match active_parent
        .tree
        .root
        .and_then(|root| active_parent.tree.tiles.get(root))
    {
        Some(Tile::Container(Container::Linear(linear))) => {
            assert_eq!(
                linear.children.len(),
                2,
                "primary child tree should replace the portal slot"
            );
            linear.children[1]
        }
        other => panic!("expected parent root split, got {other:?}"),
    };
    assert!(
        matches!(
            active_parent.tree.tiles.get(portal_slot),
            Some(Tile::Container(Container::Linear(_)))
        ),
        "primary child split should be grafted as a container"
    );

    let promoted_a = app
        .windows
        .iter()
        .find(|w| w.window_id == child_win_a)
        .expect("secondary child window A should be promoted");
    let promoted_c = app
        .windows
        .iter()
        .find(|w| w.window_id == child_win_c)
        .expect("secondary child window C should be promoted");
    assert_eq!(promoted_a.context_id, parent_ctx_id);
    assert_eq!(promoted_c.context_id, parent_ctx_id);
    assert!(promoted_a.panes.contains_key(&secondary_a_pane));
    assert!(promoted_c.panes.contains_key(&secondary_c_pane));
    assert!(
        promoted_a
            .tree
            .root
            .and_then(|root| promoted_a.tree.tiles.get(root))
            .is_some(),
        "promoted window A should keep its tile tree"
    );
    assert!(
        promoted_c
            .tree
            .root
            .and_then(|root| promoted_c.tree.tiles.get(root))
            .is_some(),
        "promoted window C should keep its tile tree"
    );

    let parent_window_coords: HashSet<_> = app
        .windows
        .iter()
        .filter(|w| w.context_id == parent_ctx_id)
        .map(|w| (w.grid_x, w.grid_y))
        .collect();
    let parent_window_count = app
        .windows
        .iter()
        .filter(|w| w.context_id == parent_ctx_id)
        .count();
    assert_eq!(
        parent_window_coords.len(),
        parent_window_count,
        "promoted child windows should have deterministic non-colliding grid coordinates"
    );
    assert!(
        app.minimap.visible,
        "dissolving a multi-window child context should show the parent minimap"
    );
    assert_eq!(
        app.minimap_visible_per_context.get(&parent_ctx_id),
        Some(&true),
        "parent minimap state should be saved as visible after promoted child windows"
    );
    assert!(
        app.windows
            .iter()
            .flat_map(|w| w.panes.values())
            .all(|pane| pane.portal_target() != Some(child_ctx_id)),
        "no surviving PortalPane may point at the dissolved context"
    );
}

/// Issue #2121: CreateContext with windows creates anchor + N extra windows.
/// Simulates what the CreateContext handler does after this change.
#[test]
fn create_context_with_windows_adds_extra_pages() {
    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);

    // Create a new context (simulates the no-parent branch of CreateContext).
    let ctx_count_before = app.router.len();
    app.new_context();
    // PTY may be unavailable in CI — new_context returns early without creating a context.
    if app.router.len() == ctx_count_before {
        return;
    }
    let ctx_id = app.router.active().context_id;
    let initial_window_count = app
        .windows
        .iter()
        .filter(|w| w.context_id == ctx_id)
        .count();
    assert_eq!(
        initial_window_count, 1,
        "new_context starts with 1 anchor window"
    );

    // Now simulate the windows loop from the CreateContext handler.
    let cmds = vec!["echo a".to_string(), "echo b".to_string()];
    let active_y = app.windows[app.active_window].grid_y;
    let mut new_x = app
        .windows
        .iter()
        .filter(|w| w.context_id == ctx_id && w.grid_y == active_y)
        .map(|w| w.grid_x)
        .max()
        .map(|x| x + 1)
        .unwrap_or(1);
    for cmd in &cmds {
        app.create_page_at(new_x, active_y, Some(cmd.as_str()), false);
        new_x += 1;
    }

    let final_window_count = app
        .windows
        .iter()
        .filter(|w| w.context_id == ctx_id)
        .count();
    assert_eq!(
        final_window_count, 3,
        "anchor + 2 --window args = 3 windows total"
    );
}

/// Cmd+R on a focused portal pane falls through to renaming the portal's
/// target subcontext (same as Cmd+Shift+R from inside it) — it must not
/// open the pane rename modal.
#[test]
fn rename_on_focused_portal_falls_through_to_subcontext() {
    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);

    let root_id = app.router.active().context_id;
    let child_id = 7701u64;
    app.router.push(crate::host::context::Context {
        name: "Child".to_string(),
        path: std::path::PathBuf::from("/tmp/child_rename"),
        root: None,
        description: None,
        context_id: child_id,
        parent_id: Some(root_id),
        depth: 1,
        parked: false,
    });

    // Insert a portal pane targeting the child and focus it.
    let portal_pane_id = 77011u64;
    let portal_tile;
    {
        let win = &mut app.windows[0];
        portal_tile = win.tree.tiles.insert_pane(portal_pane_id);
        win.panes.insert(
            portal_pane_id,
            crate::host::pane::Pane::Portal(Box::new(crate::host::pane::PortalPane {
                pane_id: portal_pane_id,
                target_context_id: child_id,
                context_state: None,
                hidden: false,
            })),
        );
        win.focused_pane = Some(portal_tile);
    }

    app.open_rename_for_focused();

    let child_idx = app.router.position(|c| c.context_id == child_id).unwrap();
    assert_eq!(
        app.renaming_window,
        Some(child_idx),
        "portal Cmd+R must open the context rename targeting the child"
    );
    assert_eq!(
        app.renaming_pane, None,
        "portal Cmd+R must not open the pane rename modal"
    );
    assert_eq!(app.rename_buffer, "Child");
}

/// Cmd+R on a non-portal pane still opens the pane rename modal.
#[test]
fn rename_on_focused_pane_opens_pane_rename() {
    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);

    let (pane_tile, pane_id) = app.add_test_pane();
    app.windows[0].focused_pane = Some(pane_tile);

    app.open_rename_for_focused();

    assert_eq!(app.renaming_pane, Some(pane_id));
    assert_eq!(app.renaming_window, None);
}

/// Closing the last pane in a subcontext must delete the subcontext and zoom
/// back out to the parent — the exact landing Cmd+Escape (ContextZoomOut)
/// would produce: parent context, parent window, previously focused tile.
#[test]
fn closing_last_pane_in_subcontext_collapses_and_zooms_out() {
    use std::collections::HashMap;

    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);

    let root_id = app.router.active().context_id;
    let root_win_id = app.windows[0].window_id;
    let (root_tile, _root_pane) = app.add_test_pane();
    app.windows[0].focused_pane = Some(root_tile);

    // Child subcontext with a single pane.
    let child_id = 9101u64;
    let child_win_id = 9102u64;
    let child_pane = 88991u64;
    app.router
        .push(test_context(child_id, root_id, "collapse_child"));
    app.context_active_window.insert(child_id, child_win_id);

    let mut child_tiles = egui_tiles::Tiles::default();
    let child_tile = child_tiles.insert_pane(child_pane);
    let mut child_panes = HashMap::new();
    child_panes.insert(child_pane, test_app_pane(child_pane));
    app.windows.push(Window {
        name: "collapse_child".to_string(),
        path: std::path::PathBuf::from("/tmp/collapse_child"),
        tree: egui_tiles::Tree::new("collapse_child", child_tile, child_tiles),
        panes: child_panes,
        focused_pane: Some(child_tile),
        zoomed_pane: None,
        grid_x: 0,
        grid_y: 0,
        window_id: child_win_id,
        context_id: child_id,
    });

    // Zoom Root → child, as sidebar/portal zoom does.
    app.router
        .push_depth(root_id, root_win_id, Some(root_tile));
    let child_idx = app.router.position(|c| c.context_id == child_id).unwrap();
    app.switch_workspace(child_idx);
    assert_eq!(app.router.active().context_id, child_id);

    // Close the only pane in the subcontext (Cmd+W path).
    app.execute_close_pane();

    assert!(
        app.router.iter().all(|c| c.context_id != child_id),
        "emptied subcontext must be removed from the router"
    );
    assert!(
        app.windows.iter().all(|w| w.context_id != child_id),
        "emptied subcontext must have no remaining windows"
    );
    assert_eq!(
        app.router.active().context_id,
        root_id,
        "closing the last subcontext pane must land on the parent context"
    );
    assert_eq!(
        app.windows[app.active_window].window_id, root_win_id,
        "must land on the same parent window Cmd+Escape would restore"
    );
    assert_eq!(
        app.windows[app.active_window].focused_pane,
        Some(root_tile),
        "must restore the parent's previously focused tile"
    );
    assert_eq!(
        app.router.current_depth(),
        0,
        "the depth-stack entry for the zoom-in must be consumed"
    );
}

/// Closing the last pane in a ROOT context must NOT delete the context —
/// the sole window stays alive so the welcome screen renders.
#[test]
fn closing_last_pane_in_root_context_keeps_context() {
    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);

    let root_id = app.router.active().context_id;
    let (root_tile, _pane) = app.add_test_pane();
    app.windows[0].focused_pane = Some(root_tile);

    app.execute_close_pane();

    assert_eq!(
        app.router.active().context_id,
        root_id,
        "root context must survive closing its last pane"
    );
    assert!(
        app.windows.iter().any(|w| w.context_id == root_id),
        "root context must keep its welcome-screen window"
    );
}

/// Closing the last pane of a NON-active subcontext (e.g. via CLI
/// `plexi pane close`) removes the subcontext without switching the
/// user's active context.
#[test]
fn closing_last_pane_in_background_subcontext_removes_it_without_switching() {
    use std::collections::HashMap;

    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);

    let root_id = app.router.active().context_id;
    let (root_tile, _root_pane) = app.add_test_pane();
    app.windows[0].focused_pane = Some(root_tile);

    let child_id = 9201u64;
    let child_win_id = 9202u64;
    let child_pane = 88992u64;
    app.router
        .push(test_context(child_id, root_id, "bg_collapse_child"));
    app.context_active_window.insert(child_id, child_win_id);

    let mut child_tiles = egui_tiles::Tiles::default();
    let child_tile = child_tiles.insert_pane(child_pane);
    let mut child_panes = HashMap::new();
    child_panes.insert(child_pane, test_app_pane(child_pane));
    app.windows.push(Window {
        name: "bg_collapse_child".to_string(),
        path: std::path::PathBuf::from("/tmp/bg_collapse_child"),
        tree: egui_tiles::Tree::new("bg_collapse_child", child_tile, child_tiles),
        panes: child_panes,
        focused_pane: Some(child_tile),
        zoomed_pane: None,
        grid_x: 0,
        grid_y: 0,
        window_id: child_win_id,
        context_id: child_id,
    });

    // Root stays active; close the subcontext's only pane by id.
    app.close_pane_by_id(child_pane);

    assert!(
        app.router.iter().all(|c| c.context_id != child_id),
        "emptied background subcontext must be removed from the router"
    );
    assert_eq!(
        app.router.active().context_id,
        root_id,
        "active context must not change when a background subcontext collapses"
    );
}

/// End-to-end over the real IPC path: `plexi context push` sends `pane_id`
/// (the caller's PLEXI_PANE_ID) and the host pushes THAT pane into the new
/// sub-context — not the focused one.
#[test]
fn push_pane_ipc_targets_caller_pane_not_focused() {
    let mut h = crate::testing::HostHarness::new();
    let pane_a = h.add_test_pane();
    let pane_b = h.add_test_pane();
    let tile_a = h.app.windows[0].tree.tiles.find_pane(&pane_a).unwrap();
    h.app.windows[0].focused_pane = Some(tile_a);
    let parent_id = h.app.router.active().context_id;
    let ctx_count_before = h.app.router.len();

    let payload = serde_json::json!({
        "type": "push_pane_to_subcontext",
        "name": "pushed",
        "pane_id": pane_b,
    });
    let req: crate::app_protocol::AppRequest =
        serde_json::from_value(payload).expect("CLI payload must deserialize");
    h.inject_ipc(req);
    h.app.drain_pane_cmd_channel();

    assert_eq!(
        h.app.router.len(),
        ctx_count_before + 1,
        "push must create one child context"
    );
    let parent_win = h
        .app
        .windows
        .iter()
        .find(|w| w.context_id == parent_id)
        .expect("parent window must still exist");
    assert!(
        parent_win.panes.contains_key(&pane_a),
        "focused pane A must stay in the parent window"
    );
    assert!(
        !parent_win.panes.contains_key(&pane_b),
        "caller pane B must have moved into the child context"
    );
    let child_win = h
        .app
        .windows
        .iter()
        .find(|w| w.context_id != parent_id)
        .expect("child window must exist");
    assert!(
        child_win.panes.contains_key(&pane_b),
        "caller pane B must live in the child window"
    );
}

/// `plexi context push` with an unknown pane id falls back to the focused pane.
#[test]
fn push_pane_ipc_unknown_pane_falls_back_to_focused() {
    let mut h = crate::testing::HostHarness::new();
    let pane_a = h.add_test_pane();
    let tile_a = h.app.windows[0].tree.tiles.find_pane(&pane_a).unwrap();
    h.app.windows[0].focused_pane = Some(tile_a);
    let parent_id = h.app.router.active().context_id;

    let payload = serde_json::json!({
        "type": "push_pane_to_subcontext",
        "pane_id": 999_999u64,
    });
    let req: crate::app_protocol::AppRequest =
        serde_json::from_value(payload).expect("CLI payload must deserialize");
    h.inject_ipc(req);
    h.app.drain_pane_cmd_channel();

    let parent_win = h
        .app
        .windows
        .iter()
        .find(|w| w.context_id == parent_id)
        .expect("parent window must still exist");
    assert!(
        !parent_win.panes.contains_key(&pane_a),
        "fallback must push the focused pane"
    );
}

/// `plexi context describe` sends the caller's PLEXI_CONTEXT_ID — the host
/// must set the description on THAT context, not the active one.
#[test]
fn set_context_description_ipc_targets_caller_context() {
    let mut h = crate::testing::HostHarness::new();
    let active_id = h.app.router.active().context_id;
    let other_id = active_id + 100;
    h.app
        .router
        .push(test_context(other_id, active_id, "background"));

    let payload = serde_json::json!({
        "type": "set_context_description",
        "description": "set from a background pane",
        "context_id": other_id,
    });
    let req: crate::app_protocol::AppRequest =
        serde_json::from_value(payload).expect("CLI payload must deserialize");
    h.inject_ipc(req);
    h.app.drain_pane_cmd_channel();

    let other_idx = h.app.router.position(|c| c.context_id == other_id).unwrap();
    assert_eq!(
        h.app.router.get(other_idx).description.as_deref(),
        Some("set from a background pane"),
        "description must land on the caller's context"
    );
    let active_idx = h
        .app
        .router
        .position(|c| c.context_id == active_id)
        .unwrap();
    assert_eq!(
        h.app.router.get(active_idx).description,
        None,
        "active context must be untouched"
    );
}

/// `plexi context set-root` sends the caller's PLEXI_CONTEXT_ID — the host
/// must re-root THAT context, not the active one.
#[test]
fn set_context_root_ipc_targets_caller_context() {
    let mut h = crate::testing::HostHarness::new();
    let active_id = h.app.router.active().context_id;
    let active_root_before = h.app.router.active().root.clone();
    let other_id = active_id + 100;
    h.app
        .router
        .push(test_context(other_id, active_id, "background"));

    let tmp = tempfile::tempdir().expect("tempdir");
    let payload = serde_json::json!({
        "type": "set_context_root",
        "root": tmp.path(),
        "context_id": other_id,
    });
    let req: crate::app_protocol::AppRequest =
        serde_json::from_value(payload).expect("CLI payload must deserialize");
    h.inject_ipc(req);
    h.app.drain_pane_cmd_channel();

    let other_idx = h.app.router.position(|c| c.context_id == other_id).unwrap();
    assert_eq!(
        h.app.router.get(other_idx).root.as_deref(),
        Some(tmp.path()),
        "root must land on the caller's context"
    );
    let active_idx = h
        .app
        .router
        .position(|c| c.context_id == active_id)
        .unwrap();
    assert_eq!(
        h.app.router.get(active_idx).root,
        active_root_before,
        "active context root must be untouched"
    );
}
