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
