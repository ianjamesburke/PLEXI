use super::super::*;

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
