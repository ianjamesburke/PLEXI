use super::super::*;

#[test]
fn parse_key_str_to_event_uses_sdk_key_names_for_app_panes() {
    let (key, modifiers) = parse_key_str_to_event("plus");
    assert_eq!(key, "plus");
    assert!(!modifiers.ctrl);

    let (key, _) = parse_key_str_to_event("equals");
    assert_eq!(key, "equals");

    let (key, _) = parse_key_str_to_event("ArrowDown");
    assert_eq!(key, "down");

    let (key, _) = parse_key_str_to_event("A");
    assert_eq!(key, "a");
}

#[test]
fn parse_key_str_to_event_preserves_modifiers() {
    let (key, modifiers) = parse_key_str_to_event("ctrl+shift+plus");

    assert_eq!(key, "plus");
    assert!(modifiers.ctrl);
    assert!(modifiers.shift);
    assert!(!modifiers.alt);
    assert!(!modifiers.cmd);
}

#[test]
fn test_spawn_pane_targets_correct_window_with_from_pane_id() {
    let ctx = egui::Context::default();
    let ft = crate::platform::logging::new_frame_tick();
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, ft);

    // Window 0 is created by new_for_test. Add a pane to it.
    let (tile_in_w0, pane_in_w0) = app.add_test_pane();
    app.windows[0].focused_pane = Some(tile_in_w0);

    // Add a second window and focus it.
    let ctx_id = app.router.active().context_id;
    app.windows.push(crate::host::context::Window {
        name: "Window 1".into(),
        path: std::env::temp_dir(),
        tree: egui_tiles::Tree::empty("w1"),
        panes: std::collections::HashMap::new(),
        focused_pane: None,
        zoomed_pane: None,
        grid_x: 1,
        grid_y: 0,
        window_id: 2,
        context_id: ctx_id,
    });
    app.active_window = 1; // focus second window

    // Verify pane is in window 0's tree, not window 1's.
    assert!(app.windows[0].tree.tiles.find_pane(&pane_in_w0).is_some());
    assert!(app.windows[1].tree.tiles.find_pane(&pane_in_w0).is_none());

    // find_pane_in_any_window should locate it in window 0.
    let loc = app.find_pane_in_any_window(pane_in_w0);
    assert!(loc.is_some(), "pane should be found in any window");
    let (found_win_idx, found_tile) = loc.unwrap();
    assert_eq!(found_win_idx, 0, "pane should be in window 0");
    assert_eq!(found_tile, tile_in_w0);
}

/// A valid socket line must queue the request AND leave a repaint pending —
/// with zero-frame idle, the repaint is what gets the queued request drained.
#[test]
fn socket_line_queues_request_and_requests_repaint() {
    let (tx, rx) = std::sync::mpsc::channel();
    let ctx = egui::Context::default();
    handle_socket_line(r#"{"type":"wake"}"#, &tx, &ctx);
    assert!(
        matches!(rx.try_recv(), Ok(crate::app_protocol::AppRequest::Wake)),
        "request must be queued on the pane-IPC channel"
    );
    assert!(
        ctx.has_requested_repaint(),
        "socket line must request a repaint so the idle UI thread wakes"
    );
}

/// A malformed socket line must queue nothing (and therefore wake nothing).
#[test]
fn socket_line_parse_error_queues_nothing() {
    let (tx, rx) = std::sync::mpsc::channel();
    let ctx = egui::Context::default();
    handle_socket_line("definitely not json", &tx, &ctx);
    assert!(
        rx.try_recv().is_err(),
        "parse failures must not queue anything"
    );
}

/// `AppRequest::Wake` through the host handler is a pure no-op: no panic,
/// no window/pane state change.
#[test]
fn wake_request_is_noop_on_host() {
    let mut h = crate::testing::HostHarness::new();
    let windows_before = h.app.windows.len();
    let panes_before: usize = h.app.windows.iter().map(|w| w.panes.len()).sum();

    h.inject_ipc(crate::app_protocol::AppRequest::Wake);
    h.app.drain_pane_cmd_channel();

    assert_eq!(
        h.app.windows.len(),
        windows_before,
        "wake must not touch windows"
    );
    let panes_after: usize = h.app.windows.iter().map(|w| w.panes.len()).sum();
    assert_eq!(panes_after, panes_before, "wake must not touch panes");
}

/// #2283: a `[file_handlers]` entry routes a matching extension into the named
/// Plexi app via the host open resolver (OpenInPane), rather than falling
/// through to the OS opener. Proves resolution tier (a) + the launch path.
#[test]
fn file_handler_config_routes_extension_to_app() {
    let ctx = egui::Context::default();
    let ft = crate::platform::logging::new_frame_tick();
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, ft);

    // Map .md -> the builtin text-editor.
    let mut handlers = std::collections::HashMap::new();
    handlers.insert("md".to_string(), "app:text-editor".to_string());
    app.config.file_handlers = Some(handlers);

    let panes_before: usize = app.windows.iter().map(|w| w.panes.len()).sum();

    // sender_pane_id = 0 -> no pane -> workspace path check is bypassed
    // (trusted host path), so a temp .md path opens cleanly.
    let path = std::env::temp_dir()
        .join("note.md")
        .to_string_lossy()
        .to_string();
    app.dispatch_open_artifact(0, path, crate::app_protocol::ArtifactOpenMode::OpenInPane);

    let panes_after: usize = app.windows.iter().map(|w| w.panes.len()).sum();
    assert_eq!(
        panes_after,
        panes_before + 1,
        "the md handler should open exactly one new in-Plexi pane"
    );

    let opened_text_editor = app.windows.iter().any(|w| {
        w.panes.values().any(|p| {
            p.as_app()
                .map(|a| a.runtime.type_id() == "text-editor")
                .unwrap_or(false)
        })
    });
    assert!(
        opened_text_editor,
        "the file_handler-routed pane should be the text-editor app"
    );
}

/// The file browser must open at the context root when one is set, not the
/// focused pane's CWD. Mirrors `resolve_new_pane_cwd`'s precedence
/// (context.root → focused cwd → home) that every other new-pane path uses.
#[test]
fn file_browser_opens_at_context_root() {
    let ctx = egui::Context::default();
    let ft = crate::platform::logging::new_frame_tick();
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, ft);

    let root = std::env::temp_dir().join("plexi-fb-root-test");
    std::fs::create_dir_all(&root).expect("mkdir root");
    app.set_context_root(root.clone(), None);

    app.open_file_browser();

    let fb_root = app.windows.iter().find_map(|w| {
        w.panes.values().find_map(|p| {
            p.as_app()
                .filter(|a| a.runtime.type_id() == "file_browser")
                .map(|a| a.workspace_root.clone())
        })
    });

    assert_eq!(
        fb_root.as_deref(),
        Some(root.as_path()),
        "file browser should open at the context root, not the focused pane CWD"
    );
}

/// When `plexi pane new --window` is called with a `from_pane_id` in a non-active
/// context, the new window must land in that pane's context — not the active one.
/// Regression for: `new_window` derived ws_id and grid_y from active_window instead
/// of from the calling pane's window.
#[test]
fn spawn_pane_new_window_uses_caller_context_not_active() {
    let ctx = egui::Context::default();
    let ft = crate::platform::logging::new_frame_tick();
    let (mut app, ipc_tx) = PlexiApp::new_for_test(ctx, ft);

    // Context 1 (id=1) is created by new_for_test at grid_y=0.
    // Add a pane to context 1's window so we have a valid from_pane_id.
    let (tile_ctx1, pane_id_ctx1) = app.add_test_pane();
    app.windows[0].focused_pane = Some(tile_ctx1);

    // Add context 2 at grid_y=1 and make it the active context.
    let ctx2_id: u64 = 2;
    app.router.push(crate::host::context::Context {
        name: "Context 2".into(),
        path: std::env::temp_dir(),
        root: None,
        description: None,
        context_id: ctx2_id,
        parent_id: None,
        depth: 0,
        parked: false,
    });
    app.windows.push(crate::host::context::Window {
        name: "Context 2".into(),
        path: std::env::temp_dir(),
        tree: egui_tiles::Tree::empty("ctx2_tree"),
        panes: std::collections::HashMap::new(),
        focused_pane: None,
        zoomed_pane: None,
        grid_x: 0,
        grid_y: 1,
        window_id: 10,
        context_id: ctx2_id,
    });
    app.active_window = 1;
    app.router.set_active(1);

    let windows_before = app.windows.len();

    // Inject a spawn-pane IPC from context 1's pane (pane_id_ctx1) with layout=new_window.
    let _ = ipc_tx.send(crate::app_protocol::AppRequest::SpawnPane {
        type_id: "terminal".to_string(),
        layout: Some("new_window".to_string()),
        args: vec![],
        pipe_id: None,
        from_pane_id: Some(pane_id_ctx1),
        request_id: None,
        response_file: None,
        ephemeral: false,
        cwd: None,
        no_focus: false,
        path: None,
        workspace_root: None,
        target_context: None,
        name: None,
    });
    app.drain_pane_cmd_channel();

    // PTY creation may fail in some CI environments; guard before asserting.
    if app.windows.len() == windows_before {
        return; // terminal spawn failed; skip remainder
    }

    let new_win = app.windows.last().expect("new window must exist");
    assert_eq!(
        new_win.context_id, 1,
        "new window must be in context 1 (caller's context), not context 2 (active)"
    );
    assert_eq!(
        new_win.grid_y, 0,
        "new window must be in row 0 (caller's grid row), not row 1 (active context's row)"
    );
}

/// When `plexi pane new --tab` is called with a `from_pane_id` in a non-active
/// window, the new tab must land in that pane's window — not the active window.
/// Regression for stint 0337: the `tab` IPC branch called `new_tab()` against
/// `self.active_window` unconditionally, ignoring `from_pane_id` entirely.
#[test]
fn spawn_pane_tab_anchors_to_from_pane_window_not_active() {
    let ctx = egui::Context::default();
    let ft = crate::platform::logging::new_frame_tick();
    let (mut app, ipc_tx) = PlexiApp::new_for_test(ctx, ft);

    // Window 0 is created by new_for_test; give it a caller pane.
    let (tile_in_w0, pane_id_ctx1) = app.add_test_pane();
    app.windows[0].focused_pane = Some(tile_in_w0);

    // Add a second window and make it the active one.
    let ctx2_id: u64 = 2;
    app.router.push(crate::host::context::Context {
        name: "Context 2".into(),
        path: std::env::temp_dir(),
        root: None,
        description: None,
        context_id: ctx2_id,
        parent_id: None,
        depth: 0,
        parked: false,
    });
    app.windows.push(crate::host::context::Window {
        name: "Context 2".into(),
        path: std::env::temp_dir(),
        tree: egui_tiles::Tree::empty("ctx2_tree"),
        panes: std::collections::HashMap::new(),
        focused_pane: None,
        zoomed_pane: None,
        grid_x: 0,
        grid_y: 1,
        window_id: 10,
        context_id: ctx2_id,
    });
    app.active_window = 1;
    app.router.set_active(1);

    let panes_in_w0_before = app.windows[0].panes.len();
    let panes_in_w1_before = app.windows[1].panes.len();

    let requested_cwd = std::env::temp_dir();

    // Inject a spawn-pane IPC from window 0's pane (pane_id_ctx1) with layout=tab,
    // while window 1 is active.
    let _ = ipc_tx.send(crate::app_protocol::AppRequest::SpawnPane {
        type_id: "terminal".to_string(),
        layout: Some("tab".to_string()),
        args: vec![],
        pipe_id: None,
        from_pane_id: Some(pane_id_ctx1),
        request_id: None,
        response_file: None,
        ephemeral: false,
        cwd: Some(requested_cwd.to_string_lossy().to_string()),
        no_focus: false,
        path: None,
        workspace_root: None,
        target_context: None,
        name: None,
    });
    app.drain_pane_cmd_channel();

    // PTY creation may fail in some CI environments; guard on total pane count
    // (not just window 0's) so a regression that lands the tab in window 1
    // instead of window 0 doesn't get mistaken for a skipped spawn failure.
    let total_before = panes_in_w0_before + panes_in_w1_before;
    let total_after = app.windows[0].panes.len() + app.windows[1].panes.len();
    if total_after == total_before {
        return; // terminal spawn failed; skip remainder
    }

    assert_eq!(
        app.windows[0].panes.len(),
        panes_in_w0_before + 1,
        "new tab must be added to window 0 (caller's window), not window 1 (active)"
    );
    assert_eq!(
        app.windows[1].panes.len(),
        panes_in_w1_before,
        "active window 1 must be untouched by a tab anchored to a different caller pane"
    );

    let new_pane_id = *app.windows[0]
        .panes
        .keys()
        .find(|id| **id != pane_id_ctx1)
        .expect("a new pane must have been added to window 0");
    let new_tile = app.windows[0]
        .tree
        .tiles
        .find_pane(&new_pane_id)
        .expect("new pane must be present in window 0's tile tree");
    // lsof-based cwd lookup races the freshly spawned child reporting its own
    // cwd; only assert when it resolved in time.
    if let Some(new_pane_cwd) = app.windows[0].get_focused_pane_cwd(new_tile) {
        assert_eq!(
            new_pane_cwd.canonicalize().unwrap_or(new_pane_cwd),
            requested_cwd.canonicalize().unwrap_or(requested_cwd),
            "new tab must honor the requested --cwd"
        );
    }
}
