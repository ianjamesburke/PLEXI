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
fn key_str_to_egui_raw_input_maps_named_keys_without_text() {
    let raw = key_str_to_egui_raw_input("enter").expect("enter must map");
    assert_eq!(raw.events.len(), 2, "named keys emit press and release");
    assert!(matches!(
        raw.events[0],
        egui::Event::Key {
            key: egui::Key::Enter,
            pressed: true,
            repeat: false,
            ..
        }
    ));
    assert!(matches!(
        raw.events[1],
        egui::Event::Key { pressed: false, .. }
    ));

    let raw = key_str_to_egui_raw_input("ArrowDown").expect("ArrowDown must map");
    assert!(matches!(
        raw.events[0],
        egui::Event::Key {
            key: egui::Key::ArrowDown,
            ..
        }
    ));
}

#[test]
fn key_str_to_egui_raw_input_emits_text_for_printable_chars() {
    let raw = key_str_to_egui_raw_input("a").expect("a must map");
    assert!(matches!(
        raw.events[0],
        egui::Event::Key {
            key: egui::Key::A,
            ..
        }
    ));
    assert!(
        matches!(&raw.events[1], egui::Event::Text(t) if t == "a"),
        "printable chars must also emit Text for search fields; got {:?}",
        raw.events
    );
    assert!(matches!(
        raw.events[2],
        egui::Event::Key { pressed: false, .. }
    ));

    let raw = key_str_to_egui_raw_input("space").expect("space must map");
    assert!(matches!(&raw.events[1], egui::Event::Text(t) if t == " "));
}

#[test]
fn clipboard_chord_matches_egui_winit_vocabulary() {
    let cmd = egui::Modifiers::COMMAND;
    assert_eq!(clipboard_chord(cmd, egui::Key::X), Some(ClipboardChord::Cut));
    assert_eq!(
        clipboard_chord(cmd, egui::Key::C),
        Some(ClipboardChord::Copy)
    );
    assert_eq!(
        clipboard_chord(cmd, egui::Key::V),
        Some(ClipboardChord::Paste)
    );
    assert_eq!(clipboard_chord(cmd, egui::Key::A), None);
    assert_eq!(
        clipboard_chord(egui::Modifiers::default(), egui::Key::V),
        None,
        "bare v is ordinary typing, never a paste chord"
    );
}

#[test]
fn key_str_clipboard_chords_translate_like_physical_input() {
    // Physical cmd+c/cmd+x reach widgets as Copy/Cut events (egui-winit
    // translates before egui sees them); the synthetic path must match or
    // every text surface silently drops the replayed raw chord.
    let raw = key_str_to_egui_raw_input("cmd+c").expect("cmd+c must map");
    assert_eq!(raw.events, vec![egui::Event::Copy]);

    let raw = key_str_to_egui_raw_input("cmd+x").expect("cmd+x must map");
    assert_eq!(raw.events, vec![egui::Event::Cut]);

    // cmd+v reads the live system clipboard, so only assert the shape: never
    // raw Key/Text events, at most one Paste carrying the clipboard text.
    let raw = key_str_to_egui_raw_input("cmd+v").expect("cmd+v must map");
    assert!(raw.events.len() <= 1, "got {:?}", raw.events);
    assert!(
        raw.events
            .iter()
            .all(|e| matches!(e, egui::Event::Paste(_))),
        "cmd+v must translate to Paste, never raw Key events; got {:?}",
        raw.events
    );
}

#[test]
fn key_str_to_egui_raw_input_chords_set_modifiers_and_suppress_text() {
    let raw = key_str_to_egui_raw_input("ctrl+c").expect("ctrl+c must map");
    assert!(raw.modifiers.ctrl);
    assert_eq!(
        raw.events.len(),
        2,
        "chord keys must not emit Text; got {:?}",
        raw.events
    );

    let raw = key_str_to_egui_raw_input("cmd+enter").expect("cmd+enter must map");
    assert!(raw.modifiers.command);
}

#[test]
fn key_str_to_egui_raw_input_rejects_unmappable_strings() {
    assert!(key_str_to_egui_raw_input("notakey").is_none());
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

#[test]
fn media_files_route_to_native_viewer_apps() {
    let ctx = egui::Context::default();
    let ft = crate::platform::logging::new_frame_tick();
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, ft);
    let (_tile, _pane) = app.add_test_pane();

    for (filename, expected_id) in [
        ("photo.png", "image-viewer"),
        ("clip.mp4", "video-player"),
        ("song.mp3", "audio-player"),
    ] {
        let panes_before: usize = app.windows.iter().map(|w| w.panes.len()).sum();
        let path = std::env::temp_dir()
            .join(filename)
            .to_string_lossy()
            .to_string();

        app.dispatch_open_artifact(0, path, crate::app_protocol::ArtifactOpenMode::OpenInPane);

        let panes_after: usize = app.windows.iter().map(|w| w.panes.len()).sum();
        assert_eq!(
            panes_after,
            panes_before + 1,
            "{filename} should open exactly one in-Plexi pane"
        );
        assert!(
            app.windows.iter().any(|w| {
                w.panes.values().any(|p| {
                    p.as_app()
                        .map(|a| a.runtime.type_id() == expected_id)
                        .unwrap_or(false)
                })
            }),
            "{filename} should route to {expected_id}"
        );
    }
}

#[test]
fn explorer_media_viewer_close_returns_focus_and_preserves_selection() {
    let ctx = egui::Context::default();
    let ft = crate::platform::logging::new_frame_tick();
    let (mut app, _tx) = PlexiApp::new_for_test(ctx, ft);
    let dir = tempfile::tempdir().expect("tempdir");
    let selected_path = dir.path().join("photo.png");
    std::fs::write(&selected_path, b"not decoded in this test").expect("write image");

    let browser: Box<dyn crate::app::app_trait::App> = Box::new(
        crate::file_browser::FileBrowserApp::new(dir.path().to_path_buf()),
    );
    app.open_builtin_app_pane(
        browser,
        crate::app::permissions::AppPermissions::builtin(),
        dir.path().to_path_buf(),
        Some("cwd".to_string()),
        Some("split_v"),
        Some(0.5),
    );

    let browser_tile = app.windows[0].focused_pane.expect("browser focused");
    let browser_pane_id = match app.windows[0].tree.tiles.get(browser_tile) {
        Some(egui_tiles::Tile::Pane(id)) => *id,
        other => panic!("expected browser pane tile, got {other:?}"),
    };
    let browser_state_before = app.windows[0]
        .panes
        .get(&browser_pane_id)
        .and_then(|p| p.as_app())
        .and_then(|a| a.runtime.serialize_state())
        .expect("browser state before");
    assert_eq!(browser_state_before["selected"], 0);

    app.dispatch_open_artifact(
        browser_pane_id,
        selected_path.to_string_lossy().to_string(),
        crate::app_protocol::ArtifactOpenMode::OpenInPane,
    );

    let viewer_tile = app.windows[0].focused_pane.expect("viewer focused");
    assert_ne!(
        viewer_tile, browser_tile,
        "viewer should open as split sibling"
    );
    let viewer_pane_id = match app.windows[0].tree.tiles.get(viewer_tile) {
        Some(egui_tiles::Tile::Pane(id)) => *id,
        other => panic!("expected viewer pane tile, got {other:?}"),
    };
    assert!(
        app.windows[0]
            .panes
            .get(&viewer_pane_id)
            .and_then(|p| p.as_app())
            .map(|a| a.runtime.type_id() == "image-viewer")
            .unwrap_or(false),
        "selected image should open in native image viewer"
    );

    app.close_focused();

    assert_eq!(
        app.windows[0].focused_pane,
        Some(browser_tile),
        "closing viewer should return focus to explorer sibling"
    );
    let browser_state_after = app.windows[0]
        .panes
        .get(&browser_pane_id)
        .and_then(|p| p.as_app())
        .and_then(|a| a.runtime.serialize_state())
        .expect("browser state after");
    assert_eq!(
        browser_state_after["selected"], browser_state_before["selected"],
        "browser selection should survive viewer open and close"
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

/// When `plexi host start` boots windowless (active window has no root pane and
/// no focused pane), a `plexi pane new` split request must still materialize a
/// pane. Regression for stint 0348: the empty-window fallback routed through
/// `split_focused`, which returns early with no focused pane to split, so the
/// spawn was silently dropped — `pane list` returned `[]` and the requested
/// name never applied. The fallback must seed a root pane instead, apply the
/// requested name, and write the actually-created pane id to the response file.
#[test]
fn spawn_pane_seeds_root_in_empty_window() {
    let ctx = egui::Context::default();
    let ft = crate::platform::logging::new_frame_tick();
    let (mut app, ipc_tx) = PlexiApp::new_for_test(ctx, ft);

    // new_for_test's sole window boots empty — no root, no focused pane —
    // exactly the windowless-boot state.
    assert!(
        app.windows[0].tree.root.is_none(),
        "precondition: window boots with no root pane"
    );
    assert!(
        app.windows[0].focused_pane.is_none(),
        "precondition: window boots with no focused pane"
    );

    let response_file =
        std::env::temp_dir().join(format!("plexi_test_seed_root_{}.json", std::process::id()));
    let _ = std::fs::remove_file(&response_file);

    // Split spawn (not --window), no from_pane_id: falls into the empty-window
    // fallback branch.
    let _ = ipc_tx.send(crate::app_protocol::AppRequest::SpawnPane {
        type_id: "terminal".to_string(),
        layout: Some("split_h".to_string()),
        args: vec![],
        from_pane_id: None,
        request_id: None,
        response_file: Some(response_file.to_string_lossy().to_string()),
        ephemeral: false,
        cwd: None,
        no_focus: false,
        path: None,
        workspace_root: None,
        target_context: None,
        name: Some("seeded".to_string()),
    });
    app.drain_pane_cmd_channel();

    // Determine PTY availability independently of the code under test:
    // `create_page_at` spawns a terminal via the same `TerminalPane::new` path.
    // If it can't create one here (a headless CI without a PTY), the fallback
    // likewise can't, so skip. When it CAN, the empty-window fallback must
    // produce a pane — so we assert unconditionally below. This split is
    // deliberate: guarding the real assertions on "panes.is_empty()" would let
    // the very bug under test (a silently dropped spawn) masquerade as a
    // skipped PTY failure.
    let pty_available = {
        let ctx = egui::Context::default();
        let ft = crate::platform::logging::new_frame_tick();
        let (mut probe, _tx) = PlexiApp::new_for_test(ctx, ft);
        let before = probe.windows.len();
        probe.create_page_at(9, 9, 1, None, false, None);
        probe.windows.len() > before
    };
    if !pty_available {
        let _ = std::fs::remove_file(&response_file);
        return; // no PTY in this environment; cannot exercise terminal spawn
    }

    assert_eq!(
        app.windows[0].panes.len(),
        1,
        "empty-window spawn must seed exactly one root pane"
    );
    assert!(
        app.windows[0].tree.root.is_some(),
        "seeded window must have a tree root"
    );

    let pane_id = *app.windows[0]
        .panes
        .keys()
        .next()
        .expect("seeded pane must exist");

    // Requested name must have applied to the seeded pane.
    let name = app.windows[0]
        .panes
        .get(&pane_id)
        .and_then(|p| p.as_terminal())
        .and_then(|t| t.name.clone());
    assert_eq!(
        name.as_deref(),
        Some("seeded"),
        "requested pane name must apply to the seeded root pane"
    );

    // Response file must report the id of the pane actually created.
    let raw =
        std::fs::read_to_string(&response_file).expect("spawn_pane must write a response file");
    let _ = std::fs::remove_file(&response_file);
    let json: serde_json::Value =
        serde_json::from_str(&raw).expect("response file must be valid JSON");
    assert_eq!(
        json["pane_id"].as_u64(),
        Some(pane_id),
        "response file pane_id must match the seeded pane's id"
    );
}

/// The notify-socket thread must wake an idle UI thread promptly: one queued
/// request, one repaint callback with the small nonzero IPC wake delay (a
/// zero delay would schedule an extra settling paint; a missing or large
/// delay leaves the request parked until an unrelated frame — stint 0479).
#[test]
fn socket_lines_after_idle_each_request_a_prompt_repaint() {
    use std::sync::{Arc, Mutex};

    let ctx = egui::Context::default();
    // Match a real eframe host after its first pass. Egui advances delayed
    // repaint deadlines by this predicted frame time before invoking the
    // native callback; a wake shorter than `predicted_dt` silently becomes an
    // immediate RepaintNow event instead of the intended one-shot delay.
    for _ in 0..3 {
        let _ = ctx.run_ui(
            egui::RawInput {
                predicted_dt: 1.0 / 60.0,
                ..Default::default()
            },
            |_| {},
        );
    }
    let delays: Arc<Mutex<Vec<std::time::Duration>>> = Arc::new(Mutex::new(Vec::new()));
    let delays_cb = delays.clone();
    ctx.set_request_repaint_callback(move |info| {
        delays_cb.lock().unwrap().push(info.delay);
    });

    let (tx, rx) = std::sync::mpsc::channel::<crate::app_protocol::AppRequest>();
    let line = r#"{"type":"log_marker","source":"test","message":"wake"}"#;
    handle_socket_line(line, &tx, &ctx);

    match rx.try_recv() {
        Ok(crate::app_protocol::AppRequest::LogMarker { source, .. }) => {
            assert_eq!(source, "test");
        }
        other => panic!("expected queued LogMarker request, got {other:?}"),
    }
    // Simulate the host returning to idle after the ready/startup request,
    // then issue the same response-file style command a later `pane list`
    // sends. Both external arrivals must cross the repaint callback with a
    // nonzero delay; eframe rejects a raw immediate wake during a stale pass.
    let _ = ctx.run_ui(
        egui::RawInput {
            predicted_dt: 1.0 / 60.0,
            ..Default::default()
        },
        |_| {},
    );
    handle_socket_line(
        r#"{"type":"list_panes","response_file":"/tmp/second-pane-list.json"}"#,
        &tx,
        &ctx,
    );
    assert!(
        matches!(
            rx.try_recv(),
            Ok(crate::app_protocol::AppRequest::ListPanes { .. })
        ),
        "post-idle pane list must queue on the pane-IPC channel"
    );

    let delays = delays.lock().unwrap();
    assert_eq!(
        delays.len(),
        2,
        "each socket line, including a post-idle pane list, must trigger one wake callback"
    );
    // The callback delay must remain nonzero after egui subtracts
    // `predicted_dt`. A zero callback is RepaintNow in eframe; its stale-pass
    // rejection can strand the queued IPC request until unrelated input.
    assert!(
        delays
            .iter()
            .all(|delay| *delay > std::time::Duration::ZERO),
        "IPC wake collapsed to RepaintNow after egui predicted_dt adjustment: {delays:?}"
    );
    assert!(
        delays.iter().all(|delay| *delay <= IPC_WAKE_DELAY),
        "wake delay must be prompt (requested {IPC_WAKE_DELAY:?}, got {delays:?})"
    );
}

/// A malformed socket line must neither queue a request nor wake the UI.
#[test]
fn socket_line_parse_error_does_not_wake() {
    use std::sync::{Arc, Mutex};

    let ctx = egui::Context::default();
    let woke: Arc<Mutex<u32>> = Arc::new(Mutex::new(0));
    let woke_cb = woke.clone();
    ctx.set_request_repaint_callback(move |_| *woke_cb.lock().unwrap() += 1);

    let (tx, rx) = std::sync::mpsc::channel::<crate::app_protocol::AppRequest>();
    handle_socket_line("not json", &tx, &ctx);

    assert!(rx.try_recv().is_err(), "malformed line must not queue");
    assert_eq!(*woke.lock().unwrap(), 0, "malformed line must not wake");
}
