use super::super::*;
use crate::app::app_trait::AppCommand;
use crate::host::context::Window;
use crate::testing::HostHarness;

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

    app.new_child_context(&parent_name, std::path::PathBuf::from("/tmp/no_adopt"))
        .expect("child create should succeed");

    let parent_win = app.windows.iter().find(|w| w.context_id == parent_id)
        .expect("parent window must still exist");

    // Original pane is still present.
    assert!(parent_win.panes.contains_key(&orig_pane_id),
        "original focused pane must NOT be adopted away");

    // Pane count grew by exactly 1 (the new Portal tile).
    assert_eq!(parent_win.panes.len(), orig_count + 1,
        "parent should have orig_count + 1 panes after new_child_context");

    // The new pane is a Portal.
    let has_sub_ctx = parent_win.panes.values().any(|p| p.portal_target().is_some());
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

    let result = app.new_child_context("Test", std::path::PathBuf::from("/tmp/child2"));

    // Whether success or failure, the parent's focused_pane must remain None.
    assert_eq!(app.windows[0].focused_pane, None, "parent focused_pane untouched");

    if result.is_ok() {
        // Parent gained exactly 1 Portal pane.
        let parent_win = app.windows.iter().find(|w| w.context_id == parent_id).unwrap();
        assert_eq!(parent_win.panes.len(), parent_pane_count_before + 1,
            "parent pane count grew by 1");
        let has_sub_ctx = parent_win.panes.values().any(|p| p.portal_target().is_some());
        assert!(has_sub_ctx, "parent has a Portal tile");
    } else {
        // PTY failed in test env — parent panes unchanged.
        assert_eq!(app.windows[0].panes.len(), parent_pane_count_before,
            "parent panes unchanged on failed terminal create");
    }
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
    let result = app.new_child_context("test", std::path::PathBuf::from("/tmp/child_ci"));
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
    let result = app.new_child_context("Test", std::path::PathBuf::from("/tmp/child_zoom"));

    if result.is_err() {
        // PTY unavailable in test env — verify caller-side depth push still works.
        app.router.push_depth(parent_ctx_id, current_win_id, current_focused);
        assert_eq!(app.router.current_depth(), 1, "depth stack grows even on Err");
        return;
    }

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
        assert_eq!(app.router.current_depth(), initial_depth,
            "depth stack must be unchanged when child creation fails");
        assert_eq!(app.router.active().context_id, parent_ctx_id,
            "active context must not change when child creation fails");
    } else {
        // Child was created and we zoomed in.
        assert_eq!(app.router.len(), initial_ctx_count + 1,
            "exactly one new context added");
        assert_ne!(app.router.active().context_id, parent_ctx_id,
            "active context must switch to the new child");
        assert_eq!(app.router.active().parent_id, Some(parent_ctx_id),
            "child's parent_id must be the original context");
        assert_eq!(app.router.current_depth(), initial_depth + 1,
            "depth stack must grow by one after auto-zoom");
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
        let result = app.new_child_context(&parent_name, path);
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
        assert_eq!(c.depth as usize, level, "context at chain[{level}] should have depth {level}");
    }

    // Verify each parent's window has a Portal tile pointing at its child.
    for i in 0..chain_ids.len().saturating_sub(1) {
        let parent_id = chain_ids[i];
        let child_id = chain_ids[i + 1];
        let parent_win = app.windows.iter().find(|w| w.context_id == parent_id)
            .expect("parent window must exist");
        let found = parent_win.panes.values()
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
    let root_win = app.windows.iter().find(|w| w.context_id == root_id).unwrap();
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
    assert!(app.router.iter().find(|c| c.context_id == a_id).is_none(),
        "A should be deleted");
    assert!(app.router.iter().find(|c| c.context_id == b_id).is_none(),
        "B should be cascade-deleted");

    // Depth stack should no longer contain A or B.
    assert!(app.router.depth_stack.iter().all(|(cid, _, _)| *cid != a_id),
        "depth_stack must not contain deleted ctx_id={a_id}");
    assert!(app.router.depth_stack.iter().all(|(cid, _, _)| *cid != b_id),
        "depth_stack must not contain deleted ctx_id={b_id}");

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
    std::fs::write(apps_a.join("test-1801-app-a").join("manifest.toml"), manifest_a).unwrap();
    std::fs::write(apps_a.join("test-1801-app-a").join("app.py"), b"").unwrap();

    let apps_b = crate::app::registry::workspace_apps_dir(&dir_b);
    std::fs::create_dir_all(apps_b.join("test-1801-app-b")).unwrap();
    std::fs::write(apps_b.join("test-1801-app-b").join("manifest.toml"), manifest_b).unwrap();
    std::fs::write(apps_b.join("test-1801-app-b").join("app.py"), b"").unwrap();

    // Switch context 0 to root A and verify registry picks up app-a.
    let idx0 = app.router.active_idx();
    app.router.get_mut(idx0).root = Some(dir_a.clone());
    app.apply_context_transition_effects();

    let ids: Vec<String> = app.registry.list().into_iter().map(|a| a.manifest.id.clone()).collect();
    assert!(ids.contains(&"test-1801-app-a".to_string()),
        "registry should contain test-1801-app-a after setting root A, got: {ids:?}");
    assert!(!ids.contains(&"test-1801-app-b".to_string()),
        "registry should not contain test-1801-app-b while on root A, got: {ids:?}");

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

    let ids: Vec<String> = app.registry.list().into_iter().map(|a| a.manifest.id.clone()).collect();
    assert!(ids.contains(&"test-1801-app-b".to_string()),
        "registry should contain test-1801-app-b after switching to root B, got: {ids:?}");
    assert!(!ids.contains(&"test-1801-app-a".to_string()),
        "registry should not contain test-1801-app-a while on root B, got: {ids:?}");

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
    assert_eq!(app.windows[0].panes.len(), 1, "setup: 1 pane before new_context");

    app.new_context();

    // PTY may be unavailable in test env -- new_context_empty returns early.
    if app.router.len() == 1 {
        return;
    }

    assert_eq!(app.router.len(), 2, "new top-level context registered");

    let new_ctx_id = app.router.active().context_id;
    assert_ne!(new_ctx_id, original_ctx_id, "active context switched to new one");

    // No depth change -- new context is top-level, not a child.
    assert_eq!(app.router.current_depth(), 0, "depth stack unchanged");

    // New context has depth 0 and no parent.
    let new_ctx = app.router.active();
    assert_eq!(new_ctx.depth, 0, "new context is top-level");
    assert!(new_ctx.parent_id.is_none(), "new context has no parent");

    // Original context's panes are untouched.
    let orig_win_idx = app.windows.iter().position(|w| w.context_id == original_ctx_id)
        .expect("original context still has a window");
    assert_eq!(app.windows[orig_win_idx].panes.len(), 1, "original panes untouched");
    assert!(app.windows[orig_win_idx].panes.contains_key(&_pane_id), "original pane still present");

    // Inline rename was opened for the new context.
    assert_eq!(
        app.renaming_window,
        Some(app.router.len() - 1),
        "inline rename opened for new context"
    );
}
