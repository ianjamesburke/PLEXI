use super::*;

#[test]
fn split_vertical_adds_pane() {
    let mut h = HostHarness::new();
    let pane_id = h.add_test_pane();
    h.focus_pane(pane_id);
    h.run_frames(1);

    assert_eq!(h.pane_count(), 1);
    h.app.split_focused(true, None, false, false, None);
    assert_eq!(
        h.pane_count(),
        2,
        "SplitVertical should add a terminal pane"
    );
}

#[test]
fn split_horizontal_adds_pane() {
    let mut h = HostHarness::new();
    let pane_id = h.add_test_pane();
    h.focus_pane(pane_id);
    h.run_frames(1);

    assert_eq!(h.pane_count(), 1);
    h.app.split_focused(false, None, false, false, None);
    assert_eq!(
        h.pane_count(),
        2,
        "SplitHorizontal should add a terminal pane"
    );
}

#[test]
fn new_context_adds_window() {
    let mut h = HostHarness::new();
    assert_eq!(h.window_count(), 1);
    h.app.new_context();
    assert_eq!(
        h.window_count(),
        2,
        "new_context should add a second window"
    );
}

#[test]
fn ordinary_splits_keep_nested_split_inside_selected_pane() {
    use egui_tiles::{Container, LinearDir, Tile};
    let mut h = HostHarness::new();
    let first = h.add_test_pane();
    h.focus_pane(first);
    h.run_frames(2);
    h.press_key(egui::Key::D, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT);
    h.run_frames(2);
    let bottom = h.app.active_focused_pane_id().expect("stacked pane focused");
    assert_ne!(bottom, first);
    h.press_key(egui::Key::D, egui::Modifiers::COMMAND);
    h.run_frames(2);
    let win = &h.app.windows[h.app.active_window];
    let root = win.tree.root().unwrap();
    let Some(Tile::Container(Container::Linear(rows))) = win.tree.tiles.get(root) else {
        panic!("expected stacked root");
    };
    assert_eq!(rows.dir, LinearDir::Vertical, "splitting bottom pane must preserve the top/bottom root");
    assert_eq!(rows.children.len(), 2);
    let first_tile = win.find_first_pane_in(rows.children[0]).unwrap();
    assert!(matches!(win.tree.tiles.get(first_tile), Some(Tile::Pane(id)) if *id == first));
    let Some(Tile::Container(Container::Linear(bottom_pair))) = win.tree.tiles.get(rows.children[1]) else {
        panic!("bottom pane must become a nested split");
    };
    assert_eq!(bottom_pair.dir, LinearDir::Horizontal);
    assert_eq!(bottom_pair.children.len(), 2);
}

#[test]
fn ordinary_splits_first_pair_has_equal_shares_after_frames() {
    use egui_tiles::{Container, Tile};
    let mut h = HostHarness::new();
    let first = h.add_test_pane();
    h.focus_pane(first);
    h.run_frames(2);
    h.press_key(egui::Key::D, egui::Modifiers::COMMAND);
    h.run_frames(4);
    let win = &h.app.windows[h.app.active_window];
    let Some(Tile::Container(Container::Linear(pair))) = win.tree.tiles.get(win.tree.root().unwrap()) else {
        panic!("expected side-by-side split");
    };
    assert_eq!(pair.children.len(), 2);
    let widths = pair.shares.split(&pair.children, 1000.0);
    assert!((widths[0] - widths[1]).abs() < 1.0, "ordinary first split must be equal: {widths:?}");
}

#[test]
fn ordinary_splits_cmd_n_preserves_previous_window_and_returns_to_it() {
    let mut h = HostHarness::new();
    let first = h.add_test_pane();
    h.focus_pane(first);
    h.run_frames(2);
    h.press_key(egui::Key::D, egui::Modifiers::COMMAND);
    h.run_frames(3);
    let original = h.app.active_window;
    let tree = serde_json::to_value(&h.app.windows[original].tree).unwrap();
    let context = h.app.windows[original].context_id;
    h.press_key(egui::Key::N, egui::Modifiers::COMMAND);
    h.run_frames(3);
    assert_eq!(h.window_count(), 2);
    assert_ne!(h.app.active_window, original);
    let new_window = &h.app.windows[h.app.active_window];
    assert_eq!(new_window.context_id, context);
    assert_eq!(new_window.panes.len(), 1);
    let pane = h.app.active_focused_pane_id().unwrap();
    assert!(h.app.pane_qualifies_for_input(pane));
    assert_eq!(serde_json::to_value(&h.app.windows[original].tree).unwrap(), tree);
    h.press_key(egui::Key::H, egui::Modifiers::COMMAND);
    h.run_frames(2);
    assert_eq!(h.app.active_window, original);
    assert!(h.app.pane_qualifies_for_input(h.app.active_focused_pane_id().unwrap()));
}

#[test]
fn ordinary_splits_resized_nested_tree_survives_save_reload() {
    use egui_tiles::{Container, Tile};
    let mut h = HostHarness::new();
    let first = h.add_test_pane();
    h.app.windows[0].panes.get_mut(&first).unwrap().as_app_mut().unwrap().manifest_id = "agents".into();
    h.focus_pane(first);
    h.run_frames(2);
    h.press_key(egui::Key::D, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT);
    h.run_frames(2);
    h.press_key(egui::Key::D, egui::Modifiers::COMMAND);
    h.run_frames(2);
    let win = &mut h.app.windows[h.app.active_window];
    let root = win.tree.root().unwrap();
    let Some(Tile::Container(Container::Linear(rows))) = win.tree.tiles.get_mut(root) else { panic!("rows") };
    rows.shares.set_share(rows.children[0], 3.0);
    rows.shares.set_share(rows.children[1], 2.0);
    h.run_frames(3);
    let profile = h.profile_dir().to_path_buf();
    assert!(h.app.save_workspace_now());
    let before = crate::workspace::WorkspaceFile::load_from(&profile).unwrap();
    let focus = h.app.active_focused_pane_id();
    let mut restored = HostHarness::from_profile_dir(profile.clone());
    restored.run_frames(3);
    assert_eq!(restored.app.active_focused_pane_id(), focus);
    assert!(restored.app.save_workspace_now());
    let after = crate::workspace::WorkspaceFile::load_from(&profile).unwrap();
    assert_eq!(serde_json::to_value(&after.windows[0].tree).unwrap(), serde_json::to_value(&before.windows[0].tree).unwrap());
    assert_eq!(after.windows[0].panes.len(), 3);
}

/// Saves the workspace and fails the test, naming `step`, if the save is refused.
fn assert_save_ok(h: &mut HostHarness, step: &str) {
    assert!(h.app.save_workspace_now(), "workspace save failed after {step}");
}

fn focus_is_pane(h: &HostHarness, window: usize) -> bool {
    let w = &h.app.windows[window];
    w.focused_pane
        .is_some_and(|t| matches!(w.tree.tiles.get(t), Some(egui_tiles::Tile::Pane(_))))
}

/// Saves a three-pane workspace, then rewrites its file so the window's
/// `focused_pane` names the root container, as an older build could leave it.
fn save_workspace_with_container_focus(h: &mut HostHarness) -> std::path::PathBuf {
    let first = h.add_test_pane();
    h.focus_pane(first);
    h.run_frames(2);
    h.app.split_focused(true, None, false, false, None);
    h.app.split_focused(false, None, false, false, None);
    h.run_frames(2);
    assert_save_ok(h, "building the fixture workspace");
    let path = crate::workspace::workspace_path_in(h.profile_dir());
    let mut ws: crate::workspace::WorkspaceFile =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let root = ws.windows[0].tree.root().unwrap();
    assert!(matches!(
        ws.windows[0].tree.tiles.get(root),
        Some(egui_tiles::Tile::Container(_))
    ));
    ws.windows[0].focused_pane = Some(root);
    std::fs::write(&path, serde_json::to_string_pretty(&ws).unwrap()).unwrap();
    path
}

#[test]
fn save_workspace_after_zoom_and_split_has_valid_focused_tile() {
    let mut h = HostHarness::new();
    let first = h.add_test_pane();
    h.focus_pane(first);
    h.run_frames(2);
    h.app.split_focused(true, None, false, false, None);
    h.app.split_focused(false, None, false, false, None);
    let w = h.app.active_window;
    let tile = h.app.windows[w].focused_pane.unwrap();
    h.app.windows[w].zoom_to(tile);
    // Save immediately after closing the zoomed focused pane, before any frame.
    let focused = h.app.active_focused_pane_id().unwrap();
    h.app.close_pane_by_id(focused);
    let focus = h.app.windows[w].focused_pane.unwrap();
    assert!(matches!(h.app.windows[w].tree.tiles.get(focus), Some(egui_tiles::Tile::Pane(_))));
    assert_save_ok(&mut h, "close focused pane while zoomed, before any frame");
    // A background window must also be saveable without an active render pass.
    h.app.split_focused(true, None, false, false, None);
    h.app.split_focused(false, None, false, false, None);
    h.run_frames(2);
    let background = h.app.active_window;
    h.app.new_context();
    h.run_frames(3);
    assert_ne!(h.app.active_window, background);
    let focused = h.app.windows[background].focused_pane.and_then(|t| match h.app.windows[background].tree.tiles.get(t) {
        Some(egui_tiles::Tile::Pane(p)) => Some(*p), _ => None }).unwrap();
    h.app.close_pane_by_id(focused);
    // Before any frame or save: only the close_tile heal can have fixed this.
    assert!(
        focus_is_pane(&h, background),
        "close_tile must leave a pane focused in a background window"
    );
    h.run_frames(5);
    assert_save_ok(&mut h, "close focused pane in a background window, after frames");
}

#[test]
fn save_reconciles_stale_focus_in_background_window() {
    let mut h = HostHarness::new();
    let pane = h.add_test_pane();
    h.focus_pane(pane);
    h.app.split_focused(true, None, false, false, None);
    h.run_frames(2);
    let background = h.app.active_window;
    let root = h.app.windows[background].tree.root.unwrap();
    assert!(matches!(h.app.windows[background].tree.tiles.get(root), Some(egui_tiles::Tile::Container(_))));
    h.app.new_context();
    assert_ne!(h.app.active_window, background);
    h.app.windows[background].focused_pane = Some(root);
    assert_save_ok(&mut h, "stale background focus without a frame");
    let focus = h.app.windows[background].focused_pane.unwrap();
    assert!(matches!(h.app.windows[background].tree.tiles.get(focus), Some(egui_tiles::Tile::Pane(_))));
}

#[test]
fn closing_a_non_focused_pane_before_a_frame_leaves_a_pane_focused() {
    let mut h = HostHarness::new();
    let first = h.add_test_pane();
    h.focus_pane(first);
    h.run_frames(2);
    // No frame between splits: bare panes are wrapped by the next simplify.
    h.app.split_focused(true, None, false, false, None);
    h.app.split_focused(false, None, false, false, None);
    let w = h.app.active_window;
    let focused = h.app.windows[w].focused_pane.unwrap();
    let victim = h.app.windows[w]
        .tree
        .tiles
        .iter()
        .find_map(|(id, tile)| {
            (*id != focused && matches!(tile, egui_tiles::Tile::Pane(_))).then_some(*id)
        })
        .expect("a non-focused pane");
    h.app.close_tile(w, victim);
    assert!(
        focus_is_pane(&h, w),
        "focused_pane must be a Tile::Pane before any frame"
    );
}

#[test]
fn invalid_focus_on_disk_loads_healed() {
    let mut h = HostHarness::new();
    save_workspace_with_container_focus(&mut h);
    let loaded = crate::workspace::WorkspaceFile::load_from(h.profile_dir())
        .expect("invalid focus must heal, not be rejected");
    let window = &loaded.windows[0];
    let focus = window.focused_pane.expect("healed focus");
    assert!(matches!(
        window.tree.tiles.get(focus),
        Some(egui_tiles::Tile::Pane(_))
    ));
}

#[test]
fn invalid_focus_on_disk_does_not_block_later_saves_and_focus_survives_reload() {
    let mut h = HostHarness::new();
    let path = save_workspace_with_container_focus(&mut h);
    let on_disk_before = std::fs::read(&path).unwrap();
    assert_save_ok(&mut h, "a file on disk with container focus");
    assert_ne!(
        std::fs::read(&path).unwrap(),
        on_disk_before,
        "the save rewrote the file"
    );
    let w = h.app.active_window;
    let live = h.app.windows[w].focused_pane.expect("live focus");
    let reloaded = crate::workspace::WorkspaceFile::load_from(h.profile_dir()).expect("reload");
    assert_eq!(
        reloaded.windows[w].focused_pane,
        Some(live),
        "focused pane id survives save and reload"
    );
    assert!(matches!(
        reloaded.windows[w].tree.tiles.get(live),
        Some(egui_tiles::Tile::Pane(_))
    ));
}
