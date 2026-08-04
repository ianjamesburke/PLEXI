use super::super::*;
use crate::host::context::Window;

#[test]
fn workspace_config_applies_when_switching_contexts() {
    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);

    let root_a = tempfile::tempdir().expect("root a");
    let root_b = tempfile::tempdir().expect("root b");
    write_workspace_config(root_a.path(), "#111111");
    write_workspace_config(root_b.path(), "#22aa44");

    app.router.get_mut(0).root = root_a.path().to_path_buf();
    app.windows[0].path = root_a.path().to_path_buf();
    app.reload_config();
    assert_eq!(
        app.colors.accent,
        egui::Color32::from_rgb(0x11, 0x11, 0x11),
        "initial active context should load root A config"
    );

    let ctx_b_id = 2;
    let win_b_id = 2;
    app.router.push(crate::host::context::Context {
        name: "Context B".into(),
        root: root_b.path().to_path_buf(),
        description: None,
        context_id: ctx_b_id,
        parent_id: None,
        depth: 0,
        parked: false,
    });
    app.windows.push(Window {
        name: "Context B".into(),
        path: root_b.path().to_path_buf(),
        tree: egui_tiles::Tree::empty("workspace_config_b"),
        panes: HashMap::new(),
        focused_pane: None,
        zoomed_pane: None,
        grid_x: 1,
        grid_y: 0,
        window_id: win_b_id,
        context_id: ctx_b_id,
    });

    app.switch_workspace(1);

    assert_eq!(
        app.colors.accent,
        egui::Color32::from_rgb(0x22, 0xaa, 0x44),
        "switching contexts should immediately load root B config"
    );
}

#[test]
fn switching_contexts_refreshes_workspace_app_registry() {
    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);

    let root_a = tempfile::tempdir().expect("root a");
    let root_b = tempfile::tempdir().expect("root b");
    std::fs::create_dir_all(root_a.path().join(crate::config::workspace_channel_dir()))
        .expect("create root a workspace dir");
    write_registry_app(root_b.path(), "local-tool", "Local Tool");

    app.router.get_mut(0).root = root_a.path().to_path_buf();
    app.windows[0].path = root_a.path().to_path_buf();
    app.reload_config();
    let ctx_a_id = app.router.active().context_id;
    assert!(
        app.registries
            .view_for_context(ctx_a_id, &app.router)
            .get("local-tool")
            .is_none(),
        "root B app must not leak into root A registry"
    );

    let ctx_b_id = 2;
    let win_b_id = 2;
    app.router.push(crate::host::context::Context {
        name: "Context B".into(),
        root: root_b.path().to_path_buf(),
        description: None,
        context_id: ctx_b_id,
        parent_id: None,
        depth: 0,
        parked: false,
    });
    app.windows.push(Window {
        name: "Context B".into(),
        path: root_b.path().to_path_buf(),
        tree: egui_tiles::Tree::empty("workspace_app_registry_b"),
        panes: HashMap::new(),
        focused_pane: None,
        zoomed_pane: None,
        grid_x: 1,
        grid_y: 0,
        window_id: win_b_id,
        context_id: ctx_b_id,
    });

    app.switch_workspace(1);
    // Registry resolution is a synchronous per-root cache (stint 0724 Phase
    // B) — `apply_context_transition_effects` (called by `switch_workspace`)
    // resolves the new root's view inline, so no background-load drain is
    // needed here anymore.

    assert!(
        app.registries
            .view_for_context(ctx_b_id, &app.router)
            .get("local-tool")
            .is_some(),
        "switching into a workspace should refresh palette-visible local apps immediately"
    );
}

fn write_workspace_config(root: &std::path::Path, accent: &str) {
    let config_dir = root.join(crate::config::workspace_channel_dir());
    std::fs::create_dir_all(&config_dir).expect("create workspace config dir");
    std::fs::write(
        config_dir.join("config.toml"),
        format!("[theme]\naccent = \"{accent}\"\n"),
    )
    .expect("write workspace config");
}

fn write_registry_app(root: &std::path::Path, id: &str, name: &str) {
    let app_dir = root
        .join(crate::config::workspace_channel_dir())
        .join("apps")
        .join(id);
    std::fs::create_dir_all(&app_dir).expect("create app dir");
    std::fs::write(
        app_dir.join("manifest.toml"),
        format!(
            "schema_version = 1\n\n[app]\nid = \"{id}\"\ntype = \"app\"\nname = \"{name}\"\nversion = \"0.0.1\"\nentry = \"main.py\"\n"
        ),
    )
    .expect("write manifest");
    std::fs::write(app_dir.join("main.py"), "#!/usr/bin/env python3\n").expect("write entry");
}

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

    app.new_child_context(crate::pane_ops::ChildContextSpec::single_terminal(
        None,
        parent_name.to_string(),
        std::path::PathBuf::from("/tmp/no_adopt"),
        true,
        false,
        None,
    ))
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

    let result = app.new_child_context(crate::pane_ops::ChildContextSpec::single_terminal(
        None,
        "Test".to_string(),
        std::path::PathBuf::from("/tmp/child2"),
        true,
        false,
        None,
    ));

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

    app.new_child_context(crate::pane_ops::ChildContextSpec::single_terminal(
        None,
        parent_name.to_string(),
        std::path::PathBuf::from("/tmp/anchor_pane"),
        true,
        false,
        Some(pane_b),
    ))
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
    let parent_name = h.app.router.active().name.to_string();

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

    app.new_child_context(crate::pane_ops::ChildContextSpec::single_terminal(
        None,
        parent_name.to_string(),
        std::path::PathBuf::from("/tmp/anchor_missing"),
        true,
        false,
        Some(999_999),
    ))
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
    let result = app.new_child_context(crate::pane_ops::ChildContextSpec::single_terminal(
        None,
        "test".to_string(),
        std::path::PathBuf::from("/tmp/child_ci"),
        true,
        false,
        None,
    ));
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
    let result = app.new_child_context(crate::pane_ops::ChildContextSpec::single_terminal(
        None,
        "Test".to_string(),
        std::path::PathBuf::from("/tmp/child_zoom"),
        true,
        false,
        None,
    ));

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
    app.router.get_mut(root_idx).name = "Root".to_string().into();

    let names = ["A", "B", "C", "D"];
    let mut parent_name = "Root".to_string();
    let mut chain_ids: Vec<u64> = vec![app.router.active().context_id];

    for &child in &names {
        let path = std::path::PathBuf::from(format!("/tmp/depth_test_{child}"));
        let result = app.new_child_context(crate::pane_ops::ChildContextSpec::single_terminal(
            None,
            parent_name.clone(),
            path,
            true,
            false,
            None,
        ));
        if result.is_err() {
            // PTY unavailable — can't build the full chain in test env. Stop here.
            break;
        }
        let new_idx = app.router.len() - 1;
        app.router.get_mut(new_idx).name = child.to_string().into();
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
        name: "Child".to_string().into(),
        root: std::path::PathBuf::from("/tmp/child_1854"),
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
fn delete_context_cascades_cleans_depth_stack_and_revokes_credentials() {
    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);

    let root_idx = app.router.active_idx();
    app.router.get_mut(root_idx).name = "Root".to_string().into();
    let root_id = app.router.active().context_id;

    // Manually insert child context A and grandchild B without PTY.
    // We insert them directly into the router/windows to avoid PTY dependency.
    let a_id = 9001u64;
    let b_id = 9002u64;
    let a_win_id = 9003u64;
    let b_win_id = 9004u64;
    let a_pane_id = 65_300_111u64;
    let b_pane_id = 65_300_112u64;
    let a_token = crate::app::host_mcp::register_pane_credential_for_test(
        a_pane_id,
        a_id,
        std::path::PathBuf::from("/tmp/a"),
    );
    let b_token = crate::app::host_mcp::register_pane_credential_for_test(
        b_pane_id,
        b_id,
        std::path::PathBuf::from("/tmp/b"),
    );

    app.router.push(crate::host::context::Context {
        name: "A".to_string().into(),
        root: std::path::PathBuf::from("/tmp/a"),
        description: None,
        context_id: a_id,
        parent_id: Some(root_id),
        depth: 1,
        parked: false,
    });
    app.router.push(crate::host::context::Context {
        name: "B".to_string().into(),
        root: std::path::PathBuf::from("/tmp/b"),
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
        let r_a = tiles_a.insert_pane(a_pane_id);
        let mut panes = std::collections::HashMap::new();
        panes.insert(a_pane_id, test_app_pane(a_pane_id));
        app.windows.push(crate::host::context::Window {
            name: String::new(),
            path: std::path::PathBuf::from("/tmp/a"),
            tree: egui_tiles::Tree::new("plexi_a", r_a, tiles_a),
            panes,
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
        let r_b = tiles_b.insert_pane(b_pane_id);
        let mut panes = std::collections::HashMap::new();
        panes.insert(b_pane_id, test_app_pane(b_pane_id));
        app.windows.push(crate::host::context::Window {
            name: String::new(),
            path: std::path::PathBuf::from("/tmp/b"),
            tree: egui_tiles::Tree::new("plexi_b", r_b, tiles_b),
            panes,
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

    assert_eq!(
        crate::app::host_mcp::authenticated_workspace_for_test(&a_token),
        None,
        "deleting a context must revoke credentials for its panes"
    );
    assert_eq!(
        crate::app::host_mcp::authenticated_workspace_for_test(&b_token),
        None,
        "cascade deletion must revoke credentials for descendant panes"
    );
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

#[test]
fn delete_window_revokes_removed_pane_credentials() {
    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);

    let context_id = app.router.active().context_id;
    let removed_pane_id = 65_300_101u64;
    let removed_token = crate::app::host_mcp::register_pane_credential_for_test(
        removed_pane_id,
        context_id,
        std::path::PathBuf::from("/tmp/removed"),
    );

    let mut tiles = egui_tiles::Tiles::default();
    let root = tiles.insert_pane(removed_pane_id);
    let mut panes = std::collections::HashMap::new();
    panes.insert(removed_pane_id, test_app_pane(removed_pane_id));
    app.windows.push(Window {
        name: "removed".to_string(),
        path: std::path::PathBuf::from("/tmp/removed"),
        tree: egui_tiles::Tree::new("removed_window", root, tiles),
        panes,
        focused_pane: Some(root),
        zoomed_pane: None,
        grid_x: 1,
        grid_y: 0,
        window_id: 65_300_102,
        context_id,
    });

    app.delete_window(1);

    assert_eq!(
        crate::app::host_mcp::authenticated_workspace_for_test(&removed_token),
        None
    );
}

/// Stint 0454: deleting a root context whose descendant tree covers every
/// other context must not empty the `WorkspaceRouter` — `remove_at` clamps
/// `active` to 0 on an empty vec, and the next `router.active()` call panics.
/// The delete must be refused, same as the pre-existing single-context guard.
#[test]
fn delete_context_refuses_cascade_that_would_empty_router() {
    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);

    let root_idx = app.router.active_idx();
    app.router.get_mut(root_idx).name = "Root".to_string().into();
    let root_id = app.router.active().context_id;

    let child_id = 9101u64;
    let grandchild_id = 9102u64;

    app.router.push(crate::host::context::Context {
        name: "Child".to_string().into(),
        root: std::path::PathBuf::from("/tmp/child"),
        description: None,
        context_id: child_id,
        parent_id: Some(root_id),
        depth: 1,
        parked: false,
    });
    app.router.push(crate::host::context::Context {
        name: "Grandchild".to_string().into(),
        root: std::path::PathBuf::from("/tmp/grandchild"),
        description: None,
        context_id: grandchild_id,
        parent_id: Some(child_id),
        depth: 2,
        parked: false,
    });

    assert_eq!(app.router.len(), 3, "setup: root + child + grandchild");

    let root_idx_now = app.router.position(|c| c.context_id == root_id).unwrap();
    app.delete_context(root_idx_now);

    // The cascade (root + child + grandchild) would have covered every
    // context in the router — the delete must be refused entirely.
    assert_eq!(
        app.router.len(),
        3,
        "delete must be refused — router must be unchanged"
    );
    assert!(app.router.iter().any(|c| c.context_id == root_id));
    assert!(app.router.iter().any(|c| c.context_id == child_id));
    assert!(app.router.iter().any(|c| c.context_id == grandchild_id));

    // Must not panic.
    let _ = app.router.active();
}

/// Stint 0454 companion: a cascade that leaves an unrelated sibling context
/// standing must still succeed exactly as before the guard was added.
#[test]
fn delete_context_cascade_allowed_when_unrelated_sibling_survives() {
    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);

    let root_idx = app.router.active_idx();
    app.router.get_mut(root_idx).name = "Root".to_string().into();
    let root_id = app.router.active().context_id;

    let a_id = 9201u64;
    let b_id = 9202u64;
    let sibling_id = 9203u64;

    app.router.push(crate::host::context::Context {
        name: "A".to_string().into(),
        root: std::path::PathBuf::from("/tmp/a"),
        description: None,
        context_id: a_id,
        parent_id: Some(root_id),
        depth: 1,
        parked: false,
    });
    app.router.push(crate::host::context::Context {
        name: "B".to_string().into(),
        root: std::path::PathBuf::from("/tmp/b"),
        description: None,
        context_id: b_id,
        parent_id: Some(a_id),
        depth: 2,
        parked: false,
    });
    app.router.push(crate::host::context::Context {
        name: "Sibling".to_string().into(),
        root: std::path::PathBuf::from("/tmp/sibling"),
        description: None,
        context_id: sibling_id,
        parent_id: Some(root_id),
        depth: 1,
        parked: false,
    });

    assert_eq!(app.router.len(), 4, "setup: root + A + B + sibling");

    let a_idx_now = app.router.position(|c| c.context_id == a_id).unwrap();
    app.delete_context(a_idx_now);

    assert!(
        app.router.iter().find(|c| c.context_id == a_id).is_none(),
        "A should be deleted"
    );
    assert!(
        app.router.iter().find(|c| c.context_id == b_id).is_none(),
        "B should be cascade-deleted"
    );
    assert!(
        app.router.iter().any(|c| c.context_id == root_id),
        "root must survive"
    );
    assert!(
        app.router.iter().any(|c| c.context_id == sibling_id),
        "unrelated sibling must survive"
    );
    assert_eq!(app.router.len(), 2, "root + sibling remain");

    // Must not panic.
    let _ = app.router.active();
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
    app.router.get_mut(idx0).root = dir_a.clone();
    app.apply_context_transition_effects();
    // Registry resolution is a synchronous per-root cache (stint 0724 Phase
    // B) — `apply_context_transition_effects` resolves it inline, so no
    // background-load drain is needed here anymore.
    let ctx_a_id = app.router.active().context_id;

    let ids: Vec<String> = app
        .registries
        .view_for_context(ctx_a_id, &app.router)
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
        root: dir_b.clone(),
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
        .registries
        .view_for_context(ctx_b_id, &app.router)
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

    // New contexts are immediately usable; the rename overlay is strictly
    // user-initiated now.
    assert_eq!(
        app.renaming_window, None,
        "new context must not open a rename overlay"
    );
    assert!(matches!(
        new_ctx.name,
        crate::host::context::ContextName::Auto(_)
    ));
}

#[test]
fn context_rename_switches_between_custom_and_auto_name() {
    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);

    app.rename_context(0, "Manual workspace");
    assert!(matches!(
        app.router.active().name,
        crate::host::context::ContextName::Custom(ref name) if name == "Manual workspace"
    ));

    app.rename_context(0, "  ");
    assert!(matches!(
        app.router.active().name,
        crate::host::context::ContextName::Auto(ref name) if !name.is_empty()
    ));
}

#[test]
fn unresolved_auto_name_uses_first_focused_pane_cwd_then_freezes() {
    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);
    let context_idx = app.router.active_idx();
    let context_id = app.router.active().context_id;
    app.router.get_mut(context_idx).name = crate::host::context::ContextName::auto("");
    let pane_id = app.host.alloc_pane_id();

    let window = app
        .windows
        .iter_mut()
        .find(|window| window.context_id == context_id)
        .expect("active context window");
    let tile_id = window.tree.tiles.insert_pane(pane_id);
    window.focused_pane = Some(tile_id);
    let mut pane = test_app_pane(pane_id);
    pane.as_app_mut().expect("app pane").workspace_root =
        std::path::PathBuf::from("/projects/first-project");
    window.panes.insert(pane_id, pane);

    app.resolve_auto_context_names();
    assert_eq!(app.router.active().name.displayed(), "first-project");

    app.windows
        .iter_mut()
        .find(|window| window.context_id == context_id)
        .and_then(|window| window.panes.get_mut(&pane_id))
        .and_then(|pane| pane.as_app_mut())
        .expect("focused app pane")
        .workspace_root = std::path::PathBuf::from("/projects/later-project");
    app.resolve_auto_context_names();

    assert_eq!(app.router.active().name.displayed(), "first-project");
}

#[test]
fn contexts_with_matching_auto_names_are_disambiguated() {
    let mut h = crate::testing::HostHarness::new();
    let left = tempfile::tempdir().expect("left root");
    let right = tempfile::tempdir().expect("right root");
    let first = left.path().join("project");
    let second = right.path().join("project");
    std::fs::create_dir(&first).expect("first project");
    std::fs::create_dir(&second).expect("second project");

    let before = h.app.router.len();
    h.app.new_context_at_path(first);
    if h.app.router.len() == before {
        return; // PTY unavailable in this test environment.
    }
    h.app.new_context_at_path(second);
    if h.app.router.len() == before + 1 {
        return; // PTY became unavailable between the two requests.
    }

    assert_eq!(h.app.router.get(before).name.displayed(), "project");
    assert_eq!(h.app.router.get(before + 1).name.displayed(), "project (2)");
}

/// Regression for stint 0607: a top-level context's root PTY must receive the
/// new context identity, even though the previously active context remains
/// selected while the new window is being seeded.
#[test]
fn new_context_root_pty_env_uses_its_own_context_identity() {
    let mut h = crate::testing::HostHarness::new();
    let previous_context_id = h.app.router.active().context_id;

    h.app.new_context();
    h.app.resolve_auto_context_names();

    let new_context = h.app.router.active().clone();
    if new_context.context_id == previous_context_id {
        return; // PTY unavailable in this test environment.
    }
    let window = h
        .app
        .windows
        .iter()
        .find(|window| window.context_id == new_context.context_id)
        .expect("new context window");
    let terminal = window
        .panes
        .values()
        .find_map(|pane| pane.as_terminal())
        .expect("new context root terminal");

    assert_eq!(
        terminal.spawn_env.get("PLEXI_CONTEXT_ID"),
        Some(&new_context.context_id.to_string())
    );
    let new_context_name = new_context.name.to_string();
    assert_eq!(
        terminal.spawn_env.get("PLEXI_CONTEXT_NAME"),
        Some(&new_context_name)
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
        name: "Child2029".to_string().into(),
        root: std::path::PathBuf::from("/tmp/test_2029_child"),
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
        name: "ChildEdge".to_string().into(),
        root: std::path::PathBuf::from("/tmp/test_2029_edge_child"),
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
    let permissions = AppPermissions::builtin();
    crate::host::pane::Pane::App(Box::new(AppPane {
        pip_status: None,
        id: pane_id,
        runtime: AppRuntime::Builtin(Box::new(crate::file_browser::FileBrowserApp::new(
            std::env::temp_dir(),
        ))),
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
        semantic_state: Default::default(),
    }))
}

fn test_context(id: u64, parent_id: u64, name: &str) -> crate::host::context::Context {
    crate::host::context::Context {
        name: name.to_string().into(),
        root: std::path::PathBuf::from(format!("/tmp/{name}")),
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
    let moved_token = crate::app::host_mcp::register_pane_credential_for_test(
        child_left,
        child_ctx_id,
        std::path::PathBuf::from("/tmp/dissolve_single_child"),
    );

    app.dissolve_portal(child_ctx_id);

    assert_eq!(
        crate::app::host_mcp::authenticated_workspace_for_test(&moved_token),
        Some(std::path::PathBuf::from("/tmp/dissolve_single_child")),
        "dissolving a context moves its panes and must not revoke their credentials"
    );
    crate::app::host_mcp::revoke_pane_credentials(child_left);

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
        app.create_page_at(new_x, active_y, ctx_id, Some(cmd.as_str()), false, None);
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
        name: "Child".to_string().into(),
        root: std::path::PathBuf::from("/tmp/child_rename"),
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
    app.router.push_depth(root_id, root_win_id, Some(root_tile));
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

/// Regression for #2255: pushing a pane to a subcontext from within a subcontext
/// must insert the grandchild immediately after its parent (child1) in the router,
/// not at the end — even when another sibling context (child2) already exists.
#[test]
fn push_pane_to_subcontext_inserts_grandchild_after_parent_not_at_end() {
    fn push_ipc(h: &mut crate::testing::HostHarness, pane_id: crate::spatial::tiling::PaneId) {
        let req: crate::app_protocol::AppRequest = serde_json::from_value(serde_json::json!({
            "type": "push_pane_to_subcontext",
            "pane_id": pane_id,
        }))
        .unwrap();
        h.inject_ipc(req);
        h.app.drain_pane_cmd_channel();
    }

    let mut h = crate::testing::HostHarness::new();
    let pane_a = h.add_test_pane();
    let pane_b = h.add_test_pane();
    let root_id = h.app.router.active().context_id;

    // Push pane_a from root -> child1(d1). Router: [root, child1]. Active: child1.
    push_ipc(&mut h, pane_a);
    let child1_id = h.app.router.active().context_id;

    // Push pane_b from root's window (IPC targets pane_b explicitly) -> child2(d1).
    // With insert_after_subtree(root), child2 lands after child1: [root, child1, child2].
    push_ipc(&mut h, pane_b);
    let child2_id = h.app.router.active().context_id;
    assert_eq!(h.app.router.len(), 3, "root + child1 + child2 = 3 contexts");
    assert_ne!(child2_id, child1_id);

    // Push pane_a from child1's window -> grandchild(d2).
    // With insert_after_subtree(child1), grandchild lands between child1 and child2:
    // [root, child1, grandchild, child2].
    // Without the fix, grandchild would append at the end: [root, child1, child2, grandchild].
    push_ipc(&mut h, pane_a);
    assert_eq!(
        h.app.router.len(),
        4,
        "root + child1 + grandchild + child2 = 4 contexts"
    );

    let ids: Vec<u64> = h.app.router.iter().map(|c| c.context_id).collect();
    let grandchild = h
        .app
        .router
        .iter()
        .find(|c| c.parent_id == Some(child1_id))
        .expect("grandchild must exist");
    assert_eq!(grandchild.depth, 2, "grandchild depth must be 2");

    let child1_pos = ids.iter().position(|&id| id == child1_id).unwrap();
    let grandchild_pos = ids
        .iter()
        .position(|&id| id == grandchild.context_id)
        .unwrap();
    let child2_pos = ids.iter().position(|&id| id == child2_id).unwrap();
    let root_pos = ids.iter().position(|&id| id == root_id).unwrap();

    assert!(root_pos < child1_pos, "root before child1");
    assert_eq!(
        grandchild_pos,
        child1_pos + 1,
        "grandchild must be immediately after child1 (not at end)"
    );
    assert!(
        grandchild_pos < child2_pos,
        "grandchild must precede sibling child2"
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
    let pane_id = 65_300_201u64;
    let mut tiles = egui_tiles::Tiles::default();
    let pane_tile = tiles.insert_pane(pane_id);
    let mut panes = std::collections::HashMap::new();
    panes.insert(pane_id, test_app_pane(pane_id));
    h.app.windows.push(Window {
        name: "background".to_string(),
        path: std::path::PathBuf::from("/tmp/background"),
        tree: egui_tiles::Tree::new("background", pane_tile, tiles),
        panes,
        focused_pane: Some(pane_tile),
        zoomed_pane: None,
        grid_x: 1,
        grid_y: 0,
        window_id: 65_300_202,
        context_id: other_id,
    });
    let old_root = std::path::PathBuf::from("/tmp/background");
    let token = crate::app::host_mcp::register_pane_credential_for_test(
        pane_id,
        other_id,
        old_root.clone(),
    );
    assert_eq!(
        crate::app::host_mcp::authenticated_workspace_for_test(&token),
        Some(old_root)
    );

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
        Some(h.app.router.get(other_idx).root.as_path()),
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
    assert_eq!(
        crate::app::host_mcp::authenticated_workspace_for_test(&token),
        Some(tmp.path().to_path_buf()),
        "set-root must rebind live pane credentials to the new workspace"
    );
    crate::app::host_mcp::revoke_pane_credentials(pane_id);
}

/// Setting a context root must ensure `<root>/.plexi/.gitignore` covers
/// `app_states/` so a user can never accidentally commit their app state
/// (standing ruling: app state is personal, single-user, local data).
#[test]
fn set_context_root_ensures_app_state_gitignore() {
    let mut h = crate::testing::HostHarness::new();
    let fresh_root = tempfile::tempdir().expect("fresh context root");

    h.app
        .set_context_root(fresh_root.path().to_path_buf(), None);

    let gitignore = fresh_root.path().join(".plexi/.gitignore");
    let contents = std::fs::read_to_string(&gitignore)
        .expect("setting a context root must create .plexi/.gitignore");
    assert!(
        contents.lines().any(|line| line.trim() == "app_states/"),
        "gitignore must cover app_states/: {contents:?}"
    );
}

/// Stint 0726: `set_context_root` is the shared invariant boundary for every
/// explicit root-change caller (Cmd+Shift+I, sidebar Set Root, the
/// context-root text overlay, `SetContextRoot` IPC) — an `Auto`-named
/// context must reflect the new root's basename immediately, not only after
/// a manual blank-rename round trip.
#[test]
fn set_context_root_updates_auto_name_immediately() {
    let mut h = crate::testing::HostHarness::new();
    let idx = h.app.router.active_idx();
    h.app.router.get_mut(idx).name = crate::host::context::ContextName::auto("stale-name");

    let parent = tempfile::tempdir().expect("tempdir");
    let new_root = parent.path().join("new-project");
    std::fs::create_dir(&new_root).expect("new-project dir");

    h.app.set_context_root(new_root, None);

    assert_eq!(
        h.app.router.get(idx).name.displayed(),
        "new-project",
        "auto name must follow the new root's basename immediately"
    );
}

/// A `Custom` name is user-owned and must never be overwritten by a root
/// change — only a blank rename submission returns a context to `Auto`.
#[test]
fn set_context_root_preserves_custom_name() {
    let mut h = crate::testing::HostHarness::new();
    let idx = h.app.router.active_idx();
    h.app.router.get_mut(idx).name = crate::host::context::ContextName::custom("My Workspace");

    let new_root = tempfile::tempdir().expect("tempdir");
    h.app.set_context_root(new_root.path().to_path_buf(), None);

    assert!(
        matches!(
            h.app.router.get(idx).name,
            crate::host::context::ContextName::Custom(ref name) if name == "My Workspace"
        ),
        "custom name must survive an explicit root change: {:?}",
        h.app.router.get(idx).name
    );
}

/// An auto name colliding with another context's displayed name after a
/// root change must be disambiguated through the same deterministic suffix
/// path as auto-naming a brand-new context, never silently duplicated.
#[test]
fn set_context_root_disambiguates_auto_name_collision() {
    let mut h = crate::testing::HostHarness::new();
    let active_id = h.app.router.active().context_id;
    let other_id = active_id + 500;
    h.app
        .router
        .push(test_context(other_id, active_id, "project"));
    let other_idx = h.app.router.position(|c| c.context_id == other_id).unwrap();
    h.app.router.get_mut(other_idx).name = crate::host::context::ContextName::auto("project");

    let idx = h.app.router.active_idx();
    h.app.router.get_mut(idx).name = crate::host::context::ContextName::auto("stale-name");

    let parent = tempfile::tempdir().expect("tempdir");
    let colliding_root = parent.path().join("project");
    std::fs::create_dir(&colliding_root).expect("project dir");

    h.app.set_context_root(colliding_root, None);

    assert_eq!(
        h.app.router.get(idx).name.displayed(),
        "project (2)",
        "colliding auto name must be disambiguated deterministically"
    );
}

/// Cmd+Shift+I (`SetContextRootFromCwd`) must persist the updated root and
/// automatic name exactly once, through `set_context_root`'s own dirty mark,
/// so the sidebar state survives a restart rather than requiring a
/// caller-specific `mark_workspace_dirty()` call.
#[test]
fn set_context_root_marks_workspace_dirty_for_persistence() {
    let mut h = crate::testing::HostHarness::new();
    h.app.workspace_dirty = false;

    let new_root = tempfile::tempdir().expect("tempdir");
    h.app.set_context_root(new_root.path().to_path_buf(), None);

    assert!(
        h.app.workspace_dirty,
        "set_context_root must mark the workspace dirty so the new root and \
         name survive a restart, regardless of caller"
    );
}

/// Dissolve is only reachable for a context that hangs off a Portal tile.
/// `dissolve_portal` early-returns without one, so the close-confirm modal
/// must not offer the action for a top-level context (stint 0542).
#[test]
fn context_close_offers_dissolve_only_for_portal_backed_context() {
    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);

    let top_level_id = app.router.active().context_id;
    assert!(
        !app.build_context_close_state(top_level_id).can_dissolve,
        "a top-level context has no parent portal — Dissolve would do nothing"
    );

    // Give the active window a Portal tile pointing at a child context.
    let child_ctx_id = top_level_id + 1_000;
    let portal_pane_id = app.host.alloc_pane_id();
    let win = &mut app.windows[app.active_window];
    win.panes.insert(
        portal_pane_id,
        crate::host::pane::Pane::Portal(Box::new(crate::host::pane::PortalPane {
            pane_id: portal_pane_id,
            target_context_id: child_ctx_id,
            context_state: None,
            hidden: false,
        })),
    );

    assert!(
        app.build_context_close_state(child_ctx_id).can_dissolve,
        "a context reached through a Portal tile must offer Dissolve"
    );
    assert!(
        !app.build_context_close_state(child_ctx_id + 1).can_dissolve,
        "an unrelated context id must not inherit the portal's dissolvability"
    );
}

// ---------------------------------------------------------------------------
// stint 0568 — `plexi context sub`: one command, N agent panes, one subcontext.
//
// The four defects this route fixes, each with a guard below:
//   1. N commands produced N+1 panes (a spare seeded terminal).
//   2. Panes landed as sibling *pages*, not tiles in one window.
//   3. The root defaulted to the parent context's path, not the caller's cwd.
//   4. The parent was resolved by name, so duplicate names nested wrongly.
// ---------------------------------------------------------------------------

/// Defect 1: the squad window holds exactly the requested pane count — the
/// unconditional seeded terminal is gone from this route.
#[test]
fn context_sub_creates_exactly_n_panes() {
    let mut h = crate::testing::HostHarness::new();
    let anchor = h.add_test_pane();
    let parent_id = h.app.router.active().context_id;
    let root = tempfile::tempdir().expect("squad root");

    let payload = serde_json::json!({
        "type": "create_sub_context",
        "name": "agentsquad",
        "root": root.path(),
        "parent_context_id": parent_id,
        "panes": ["echo pane-a", "echo pane-b", "echo pane-c"],
        "anchor_pane": anchor,
    });
    let req: crate::app_protocol::AppRequest =
        serde_json::from_value(payload).expect("CLI payload must deserialize");
    h.inject_ipc(req);
    h.app.drain_pane_cmd_channel();

    let child = h
        .app
        .router
        .iter()
        .find(|c| c.parent_id == Some(parent_id))
        .expect("sub-context must exist");
    assert_eq!(child.name, "agentsquad", "explicit CLI name must win");
    let child_windows: Vec<_> = h
        .app
        .windows
        .iter()
        .filter(|w| w.context_id == child.context_id)
        .collect();

    // Defect 2: one window, not one page per command.
    assert_eq!(
        child_windows.len(),
        1,
        "3 agents must live in ONE window, not N sibling pages"
    );
    // Defect 1: three agents, three panes — no spare terminal.
    assert_eq!(
        child_windows[0].panes.len(),
        3,
        "--agents 3 must yield exactly 3 panes, not 3 + a seeded terminal"
    );
    // Defect 3: rooted at the path the caller passed (its cwd), not the parent's.
    assert_eq!(
        Some(child.root.as_path()),
        Some(root.path()),
        "sub-context must root at the caller-supplied path"
    );
}

/// Defect 2, structurally: the squad's panes are tiles of a single container in
/// one tree, so they render side by side rather than as separate pages.
#[test]
fn context_sub_panes_share_one_tiled_window() {
    let mut h = crate::testing::HostHarness::new();
    let parent_id = h.app.router.active().context_id;
    let root = tempfile::tempdir().expect("squad root");

    let req: crate::app_protocol::AppRequest = serde_json::from_value(serde_json::json!({
        "type": "create_sub_context",
        "name": "tiled",
        "root": root.path(),
        "parent_context_id": parent_id,
        "panes": [null, null, null, null],
        "layout": "tiled",
    }))
    .expect("payload must deserialize");
    h.inject_ipc(req);
    h.app.drain_pane_cmd_channel();

    let child_ctx = h
        .app
        .router
        .iter()
        .find(|c| c.parent_id == Some(parent_id))
        .expect("sub-context must exist")
        .context_id;
    let win = h
        .app
        .windows
        .iter()
        .find(|w| w.context_id == child_ctx)
        .expect("child window");
    let root_tile = win.tree.root.expect("child tree must have a root");
    let children = match win.tree.tiles.get(root_tile) {
        Some(egui_tiles::Tile::Container(c)) => c.children().copied().collect::<Vec<_>>(),
        other => panic!("tiled squad root must be a container, got {other:?}"),
    };
    assert_eq!(
        children.len(),
        4,
        "all 4 panes must be tiles under one container"
    );
    assert!(
        matches!(
            win.tree.tiles.get(root_tile),
            Some(egui_tiles::Tile::Container(egui_tiles::Container::Grid(_)))
        ),
        "the default tiled layout must build a Grid container"
    );
}

/// Defect 4: two contexts sharing a name must not make the parent ambiguous —
/// `PLEXI_CONTEXT_ID` decides.
#[test]
fn resolve_parent_context_prefers_id_over_duplicate_name() {
    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);

    let first_id = app.router.active().context_id;
    let duplicate_id = first_id + 500;
    app.router.push(crate::host::context::Context {
        name: app.router.active().name.clone(),
        root: std::path::PathBuf::from("/tmp/dupe"),
        description: None,
        context_id: duplicate_id,
        parent_id: None,
        depth: 0,
        parked: false,
    });
    let name = app.router.active().name.clone();

    let by_id = app
        .resolve_parent_context(Some(duplicate_id), name.displayed())
        .expect("id must resolve");
    assert_eq!(
        app.router.get(by_id).context_id,
        duplicate_id,
        "the id must select the second context even though the first shares its name"
    );

    // No id → first name match, the historical behaviour.
    let by_name = app
        .resolve_parent_context(None, name.displayed())
        .expect("name must resolve");
    assert_eq!(app.router.get(by_name).context_id, first_id);

    // A stale id must NOT fall back to the name. The pane's context was
    // deleted; resolving its name would silently attach the child to whichever
    // unrelated context happens to share it.
    assert_eq!(
        app.resolve_parent_context(Some(999_999), name.displayed()),
        None,
        "an id naming no live context must fail, not guess by name"
    );
}

/// The child is registered one level below its parent and rooted where the
/// caller asked, which is what `create_context_pane_set` stamps into each
/// squad pane's `PLEXI_CONTEXT_*` env.
///
/// `TerminalBackend` moves its `BackendSettings` into the PTY options rather
/// than retaining them, so the env itself is not observable after spawn; the
/// two halves of that contract are pinned here and in
/// `make_backend_settings_stamps_the_context_it_is_given` below.
#[test]
fn context_sub_child_is_registered_one_level_below_parent() {
    let mut h = crate::testing::HostHarness::new();
    let parent_id = h.app.router.active().context_id;
    let parent_depth = h.app.router.active().depth;
    let root = tempfile::tempdir().expect("squad root");

    let req: crate::app_protocol::AppRequest = serde_json::from_value(serde_json::json!({
        "type": "create_sub_context",
        "name": "envcheck",
        "root": root.path(),
        "parent_context_id": parent_id,
        "panes": [null, null],
    }))
    .expect("payload must deserialize");
    h.inject_ipc(req);
    h.app.drain_pane_cmd_channel();

    let child = h
        .app
        .router
        .iter()
        .find(|c| c.parent_id == Some(parent_id))
        .expect("sub-context must exist")
        .clone();
    assert_eq!(child.depth, parent_depth + 1, "child sits one level deeper");
    assert_eq!(child.root.as_path(), root.path());
    assert_eq!(
        h.app.context_depth_for(child.context_id),
        parent_depth + 1,
        "depth must be resolvable by id — this is what panes stamp as PLEXI_CONTEXT_DEPTH"
    );
}

/// The env seam `create_context_pane_set` feeds: whatever context identity it is
/// handed is what lands in the pane's environment. Seeding the squad *before*
/// the child context is registered is only safe because this takes the identity
/// explicitly instead of reading the active window.
#[test]
fn make_backend_settings_stamps_the_context_it_is_given() {
    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);
    let (settings, _pending_credential) = PlexiApp::make_backend_settings(
        7,
        Some(std::path::PathBuf::from("/tmp")),
        &app.colors,
        4242,
        "agentsquad",
        "",
        Some(&std::path::PathBuf::from("/tmp/squad")),
        3,
    );
    assert_eq!(
        settings.env.get("PLEXI_CONTEXT_ID"),
        Some(&"4242".to_string())
    );
    assert_eq!(
        settings.env.get("PLEXI_CONTEXT_NAME"),
        Some(&"agentsquad".to_string())
    );
    assert_eq!(
        settings.env.get("PLEXI_CONTEXT_DEPTH"),
        Some(&"3".to_string())
    );
    assert_eq!(settings.env.get("PLEXI_PANE_ID"), Some(&"7".to_string()));
}

/// An unresolvable parent fails before any pane is spawned — no orphan context,
/// no half-built squad.
#[test]
fn context_sub_unknown_parent_creates_nothing() {
    let mut h = crate::testing::HostHarness::new();
    let contexts_before = h.app.router.len();
    let windows_before = h.app.windows.len();

    let req: crate::app_protocol::AppRequest = serde_json::from_value(serde_json::json!({
        "type": "create_sub_context",
        "name": "orphan",
        "root": "/tmp",
        "parent_context_id": 987_654_321_u64,
        "parent_name": "no-such-context",
        "panes": [null, null],
    }))
    .expect("payload must deserialize");
    h.inject_ipc(req);
    h.app.drain_pane_cmd_channel();

    assert_eq!(
        h.app.router.len(),
        contexts_before,
        "no context may be added"
    );
    assert_eq!(
        h.app.windows.len(),
        windows_before,
        "no window may be added"
    );
}

/// The name must be carried *into* `new_child_context` via the spec, not
/// applied by renaming the router entry afterwards.
///
/// This calls `new_child_context` directly, so nothing downstream can patch the
/// name up: if the spec's name were ignored, the context would come back named
/// after its directory. That ordering is what matters — the panes are spawned
/// inside this call with the name in `PLEXI_CONTEXT_NAME`, so a rename applied
/// by the caller afterwards would leave running agents advertising a name the
/// router no longer uses.
#[test]
fn child_context_takes_its_name_from_the_spec_not_the_path() {
    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);
    let parent_id = app.router.active().context_id;
    // A root whose basename cannot be confused with the requested name.
    let root = tempfile::tempdir().expect("squad root");
    let derived = root
        .path()
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .expect("tempdir has a basename");

    let child = app
        .new_child_context(crate::pane_ops::ChildContextSpec {
            parent_id: Some(parent_id),
            parent_name: String::new(),
            name: Some("agentsquad".to_string()),
            path: root.path().to_path_buf(),
            portal_vertical: true,
            portal_first: false,
            anchor_pane: None,
            panes: vec![None, None],
            layout: crate::app_protocol::SubContextLayout::Tiled,
        })
        .expect("child create should succeed");

    assert_eq!(
        app.context_name_for(child.context_id),
        "agentsquad",
        "the spec's name must be the context's name the moment it is created"
    );
    assert_ne!(
        app.context_name_for(child.context_id),
        derived,
        "the path-derived name must not win over an explicit one"
    );

    // And with no explicit name, the derived name is still the fallback.
    let plain = app
        .new_child_context(crate::pane_ops::ChildContextSpec::single_terminal(
            Some(parent_id),
            String::new(),
            root.path().to_path_buf(),
            true,
            false,
            None,
        ))
        .expect("child create should succeed");
    assert_eq!(app.context_name_for(plain.context_id), derived);
}

/// `--focus` must record the *caller's* window as the zoom-out target. A
/// background pane can create a squad while the user is looking at a different
/// context; a depth entry naming the globally active window could not restore
/// the caller's location.
#[test]
fn context_sub_focus_returns_to_the_callers_window_not_the_active_one() {
    let mut h = crate::testing::HostHarness::new();
    let caller_pane = h.add_test_pane();
    let caller_ctx_id = h.app.router.active().context_id;
    let caller_win_id = h.app.windows[0].window_id;

    // Make a second context active, so `active_window` is NOT the caller's.
    let other_ctx_id = 9_001;
    let other_win_id = 9_002;
    h.app.router.push(crate::host::context::Context {
        name: "Elsewhere".into(),
        root: std::path::PathBuf::from("/tmp/elsewhere"),
        description: None,
        context_id: other_ctx_id,
        parent_id: None,
        depth: 0,
        parked: false,
    });
    h.app.windows.push(Window {
        name: String::new(),
        path: std::path::PathBuf::from("/tmp/elsewhere"),
        tree: egui_tiles::Tree::empty("plexi"),
        panes: std::collections::HashMap::new(),
        focused_pane: None,
        zoomed_pane: None,
        grid_x: 0,
        grid_y: 0,
        window_id: other_win_id,
        context_id: other_ctx_id,
    });
    h.app.active_window = h.app.windows.len() - 1;
    assert_ne!(
        h.app.windows[h.app.active_window].window_id, caller_win_id,
        "precondition: the active window is not the caller's"
    );

    let root = tempfile::tempdir().expect("squad root");
    let req: crate::app_protocol::AppRequest = serde_json::from_value(serde_json::json!({
        "type": "create_sub_context",
        "name": "bgsquad",
        "root": root.path(),
        "parent_context_id": caller_ctx_id,
        "anchor_pane": caller_pane,
        "panes": [null],
        "focus": true,
    }))
    .expect("payload must deserialize");
    h.inject_ipc(req);
    h.app.drain_pane_cmd_channel();

    let (depth_ctx, depth_win, _) = h
        .app
        .router
        .depth_stack
        .last()
        .copied()
        .expect("--focus must push a depth entry");
    assert_eq!(depth_ctx, caller_ctx_id, "return context is the caller's");
    assert_eq!(
        depth_win, caller_win_id,
        "return window must be the caller's, not the globally active one"
    );
}

// ── Stint 0678 audit evidence ────────────────────────────────────────────────
// Part of the reproduction half of the context-scoped state persistence audit
// (docs/context-state-persistence-audit.md). The rest of the audit's evidence
// lives in `host::wasm_python::tests::audit_0678`, next to the function that
// computes a state address; that module's doc comment carries the assertion
// discipline every audit test follows.

/// Q1: nothing rejects two contexts pointing at the same root. `set_context_root`
/// has no uniqueness check, and `new_context_empty` anchors every new context at
/// the home directory, so duplicates are the default outcome rather than an edge
/// case. Every context-scoped state path derived from a shared root collides.
#[test]
fn audit_0678_two_contexts_accept_the_same_root() {
    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);

    let shared = tempfile::tempdir().expect("shared root");
    let second_ctx_id = 4242;
    app.router.push(crate::host::context::Context {
        name: "Second".into(),
        root: shared.path().to_path_buf(),
        description: None,
        context_id: second_ctx_id,
        parent_id: None,
        depth: 0,
        parked: false,
    });

    let first_ctx_id = app.router.get(0).context_id;
    app.set_context_root(shared.path().to_path_buf(), Some(first_ctx_id));
    app.set_context_root(shared.path().to_path_buf(), Some(second_ctx_id));

    let roots: Vec<_> = app.router.iter().map(|c| c.root.clone()).collect();
    assert_eq!(
        roots,
        vec![shared.path().to_path_buf(), shared.path().to_path_buf()],
        "set_context_root accepts a root already held by another context"
    );
}

// Q1's second half — a sub-context created from the keyboard inheriting the
// parent's stale `path` instead of its live `root` — documented a divergence
// between two separate fields on `Context`. Stints 0651/0652 collapsed
// `Context.path` into the single non-optional `root`, so the two fields this
// test existed to catch drifting apart can no longer diverge: there is only
// one field. The test was deleted rather than adapted; nothing here survives
// the fix to assert.

// ── Perf gate: long host operations off the UI thread (stint 0548) ────────
//
// `delete_context`/`delete_window` used to drop removed `Window`s (and their
// `WasmPythonRuntime` panes) synchronously on the UI thread, which blocks on
// `WasmPythonRuntime::drop`'s `thread.join()` shutdown handshake. The fix
// hands the doomed windows to a background thread and returns immediately.
// These gates lock that: the synchronous portion of each call must return in
// single-digit milliseconds regardless of how many windows/panes are queued
// for disposal. Timing assertions are inherently machine-dependent, so both
// are `#[ignore]`d — run explicitly on a quiet machine, not in CI.

#[test]
#[ignore = "perf-gate: run explicitly on a quiet machine"]
fn perf_gate_delete_context_does_not_block_ui_thread() {
    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);

    let root_id = app.router.active().context_id;

    // A handful of child contexts, each with its own window+pane, all
    // parented under root so a single `delete_context(root_idx)` cascades
    // and disposes every one of them.
    for i in 0..8u64 {
        let ctx_id = 9_100 + i;
        let win_id = 9_200 + i;
        let pane_id = 9_300 + i;
        app.router.push(crate::host::context::Context {
            name: format!("child-{i}").into(),
            root: std::path::PathBuf::from(format!("/tmp/perf-gate-delete-context-{i}")),
            description: None,
            context_id: ctx_id,
            parent_id: Some(root_id),
            depth: 1,
            parked: false,
        });
        let mut tiles = egui_tiles::Tiles::default();
        let root_tile = tiles.insert_pane(pane_id);
        let mut panes = std::collections::HashMap::new();
        panes.insert(pane_id, test_app_pane(pane_id));
        app.windows.push(crate::host::context::Window {
            name: String::new(),
            path: std::path::PathBuf::from(format!("/tmp/perf-gate-delete-context-{i}")),
            tree: egui_tiles::Tree::new(format!("plexi_perf_gate_{i}"), root_tile, tiles),
            panes,
            focused_pane: None,
            zoomed_pane: None,
            grid_x: 0,
            grid_y: 0,
            window_id: win_id,
            context_id: ctx_id,
        });
    }

    let root_idx = app.router.position(|c| c.context_id == root_id).unwrap();
    let start = std::time::Instant::now();
    app.delete_context(root_idx);
    let elapsed = start.elapsed();
    eprintln!("perf_gate_delete_context_does_not_block_ui_thread: elapsed={elapsed:?}");

    assert!(
        elapsed < std::time::Duration::from_millis(20),
        "delete_context must not block the UI thread on window disposal; took {elapsed:?}"
    );
}

// `perf_gate_context_transition_does_not_block_ui_thread` (the background-
// thread registry rescan gate from stint 0548) was removed in stint 0724
// Phase B: `RegistryViews` resolves per-root synchronously by design — a
// never-before-seen root must resolve correctly at the point of use (e.g.
// launching into a context nobody has visited yet), which a background
// load racing the caller cannot guarantee. The perf property this gate
// checked (context transitions don't stall the UI thread on a large
// registry scan) no longer holds as an architectural claim; the tradeoff is
// deliberate — see `crate::app::registry_views` module docs.

// ── Stint 0724 Phase B: per-context registry resolution ─────────────────────

fn counter_wasm_fixture() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/wasm-fixtures/counter.wasm")
}

/// Install a WASM-manifest app (a copy of the `counter.wasm` POC fixture,
/// which needs no external process and is already exercised elsewhere in the
/// suite) under `root`'s workspace-local apps dir.
fn write_registry_wasm_app(root: &std::path::Path, id: &str) {
    let app_dir = root
        .join(crate::config::workspace_channel_dir())
        .join("apps")
        .join(id);
    std::fs::create_dir_all(&app_dir).expect("create app dir");
    std::fs::write(
        app_dir.join("manifest.toml"),
        format!(
            "schema_version = 1\n\n[app]\nid = \"{id}\"\ntype = \"wasm\"\nname = \"{id}\"\n\
             version = \"0.0.1\"\nentry = \"counter.wasm\"\n\n[app.capabilities]\n\
             capabilities = []\n"
        ),
    )
    .expect("write manifest");
    std::fs::copy(counter_wasm_fixture(), app_dir.join("counter.wasm")).expect("copy fixture");
}

/// The falsifying regression test for the stint 0724 bug: before Phase B,
/// `PlexiApp` held exactly one `AppRegistry`, refreshed only when the
/// *active* context transitioned (`apply_context_transition_effects`). A
/// launch redirected at a DIFFERENT context — via the sanctioned
/// `active_window`-redirect convention `pane_ops::dispatch` already uses for
/// an explicit target (never by mutating `router.active()`) — never
/// triggered that refresh, so it resolved against whichever registry the
/// last real transition happened to load. An app installed only in another
/// context's root was invisible there no matter how the launch was routed.
///
/// `RegistryViews::view_for_context` fixes this structurally: every launch
/// resolves its OWN target context's root at the point of use, independent
/// of transition history.
#[test]
fn launch_into_an_inactive_context_resolves_that_contexts_own_registry() {
    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);
    let app_id = "com.plexi.counter-cross-context-test";

    // Context A (the harness default, active) has no such app installed.
    let ctx_a_id = app.router.active().context_id;
    assert!(
        app.registries
            .view_for_context(ctx_a_id, &app.router)
            .get(app_id)
            .is_none(),
        "setup: context A must not have the app installed"
    );

    // Context B, NOT active, with the app installed only in its own root.
    let root_b = tempfile::tempdir().expect("root b");
    write_registry_wasm_app(root_b.path(), app_id);
    let ctx_b_id = 2;
    let win_b_id = 2;
    app.router.push(crate::host::context::Context {
        name: "Context B".into(),
        root: root_b.path().to_path_buf(),
        description: None,
        context_id: ctx_b_id,
        parent_id: None,
        depth: 0,
        parked: false,
    });
    let win_b_idx = app.windows.len();
    app.windows.push(Window {
        name: "Context B".into(),
        path: root_b.path().to_path_buf(),
        tree: egui_tiles::Tree::empty("cross_context_launch_test"),
        panes: HashMap::new(),
        focused_pane: None,
        zoomed_pane: None,
        grid_x: 1,
        grid_y: 0,
        window_id: win_b_id,
        context_id: ctx_b_id,
    });

    // Context A is still active — no `switch_workspace`/context transition
    // has happened. Launching by id while active_window targets A must fail:
    // the app genuinely isn't installed there.
    assert_eq!(app.active_window, 0, "context A must still be active");
    let result_from_a = app.launch_app_by_id_with_layout(app_id, None, &[], None);
    assert!(
        result_from_a.is_err(),
        "the app is not installed in context A's root — launch must fail, got {result_from_a:?}"
    );

    // Redirect `active_window` to context B's window — the sanctioned
    // explicit-target convention (mirrors `pane_ops::dispatch`'s spawn_pane
    // handling of `from_pane_id`), never a real context switch. Context A
    // remains the router's active context throughout.
    app.active_window = win_b_idx;
    let result_from_b = app.launch_app_by_id_with_layout(app_id, None, &[], None);
    assert!(
        result_from_b.is_ok(),
        "launching into context B's window must resolve context B's OWN \
         registry view, not whatever context last transitioned — got {result_from_b:?}"
    );
    assert_eq!(
        app.windows[win_b_idx].panes.len(),
        1,
        "the app must have spawned a pane in context B's window"
    );
    let spawned = app.windows[win_b_idx]
        .panes
        .values()
        .next()
        .and_then(|p| p.as_app())
        .expect("spawned pane must be an app pane");
    assert_eq!(spawned.manifest_id, app_id);
}

/// `PlexiApp::set_context_root` on a context that is NOT the active one used
/// to silently do nothing to the registry: the old `apply_context_transition_effects`
/// (registry rescan + watcher restart) only ran when `idx == router.active_idx()`.
/// `emit_scope_invalidation`'s `ContextRootChanged` handling now rescans
/// `RegistryViews` unconditionally, regardless of which context is active.
#[test]
fn set_context_root_on_an_inactive_context_rescans_its_registry() {
    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);
    let app_id = "com.plexi.inactive-set-root-test";

    // A second, inactive context.
    let ctx_b_id = 2;
    app.router.push(crate::host::context::Context {
        name: "Context B".into(),
        root: tempfile::tempdir().expect("root b placeholder").keep(),
        description: None,
        context_id: ctx_b_id,
        parent_id: None,
        depth: 0,
        parked: false,
    });
    app.windows.push(Window {
        name: "Context B".into(),
        path: std::env::temp_dir(),
        tree: egui_tiles::Tree::empty("inactive_set_root_test"),
        panes: HashMap::new(),
        focused_pane: None,
        zoomed_pane: None,
        grid_x: 1,
        grid_y: 0,
        window_id: 2,
        context_id: ctx_b_id,
    });
    assert_ne!(
        app.router.active().context_id,
        ctx_b_id,
        "setup: context B must not be active"
    );

    let new_root = tempfile::tempdir().expect("new root");
    write_registry_wasm_app(new_root.path(), app_id);

    app.set_context_root(new_root.path().to_path_buf(), Some(ctx_b_id));

    assert_ne!(
        app.router.active().context_id,
        ctx_b_id,
        "set_context_root(Some(ctx_b_id)) must not itself change the active context"
    );
    assert!(
        app.registries
            .view_for_context(ctx_b_id, &app.router)
            .get(app_id)
            .is_some(),
        "setting an INACTIVE context's root must rescan its registry view immediately, \
         not leave it stale until the user happens to navigate there"
    );
}

/// The registry watcher must cover every root `RegistryViews` holds a view
/// for, not just the active context's — `reconcile_registry_watchers` starts
/// a watcher the first time any context's root is resolved, and firing that
/// specific root's `SourceGenerationChanged` (exactly what the per-root
/// watcher drain loop in `PlexiApp::logic` does) must rescan only that root.
#[test]
fn registry_watcher_covers_an_inactive_contexts_root_and_rescans_it_alone() {
    let ctx = egui::Context::default();
    let frame_tick = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, frame_tick);

    let ctx_a_id = app.router.active().context_id;
    let root_a = app.router.active().root.clone();

    // A second, inactive context with its own root.
    let root_b = tempfile::tempdir().expect("root b");
    let ctx_b_id = 2;
    app.router.push(crate::host::context::Context {
        name: "Context B".into(),
        root: root_b.path().to_path_buf(),
        description: None,
        context_id: ctx_b_id,
        parent_id: None,
        depth: 0,
        parked: false,
    });
    app.windows.push(Window {
        name: "Context B".into(),
        path: root_b.path().to_path_buf(),
        tree: egui_tiles::Tree::empty("watcher_inactive_root_test"),
        panes: HashMap::new(),
        focused_pane: None,
        zoomed_pane: None,
        grid_x: 1,
        grid_y: 0,
        window_id: 2,
        context_id: ctx_b_id,
    });

    // Resolve BOTH contexts' views (context A's happens automatically at
    // harness construction; context B's here simulates a cross-context
    // launch, palette open, or any other resolution).
    let _ = app.registries.view_for_context(ctx_a_id, &app.router);
    let _ = app.registries.view_for_context(ctx_b_id, &app.router);

    app.reconcile_registry_watchers();
    let root_a_canon = root_a.canonicalize().unwrap_or(root_a.clone());
    let root_b_canon = root_b
        .path()
        .canonicalize()
        .unwrap_or(root_b.path().to_path_buf());
    assert!(
        app.registry_watchers.contains_key(&root_b_canon),
        "the inactive context's root must be watched too, not just the active one"
    );
    assert!(
        app.registry_watchers.contains_key(&root_a_canon),
        "the active context's root must still be watched (no regression)"
    );

    // Simulate the inactive root's watcher firing (exactly what the per-root
    // drain loop in `PlexiApp::logic` does when THAT root's receiver has a
    // signal) and confirm only root B gets rescanned.
    let app_id = "com.plexi.watcher-inactive-test";
    write_registry_wasm_app(root_b.path(), app_id);
    app.emit_scope_invalidation(
        crate::host::scope::ScopeInvalidation::SourceGenerationChanged {
            source_path: root_b.path().to_path_buf(),
        },
    );

    assert!(
        app.registries
            .view_for_context(ctx_b_id, &app.router)
            .get(app_id)
            .is_some(),
        "firing the inactive root's own invalidation must rescan it and pick up the new app"
    );
    assert!(
        app.registries
            .view_for_context(ctx_a_id, &app.router)
            .get(app_id)
            .is_none(),
        "root A must be untouched by root B's invalidation"
    );
}
