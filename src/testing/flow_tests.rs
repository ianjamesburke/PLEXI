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
    h.press_key(
        egui::Key::D,
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
    );
    h.run_frames(2);
    let bottom = h
        .app
        .active_focused_pane_id()
        .expect("stacked pane focused");
    assert_ne!(bottom, first);
    h.press_key(egui::Key::D, egui::Modifiers::COMMAND);
    h.run_frames(2);
    let win = &h.app.windows[h.app.active_window];
    let root = win.tree.root().unwrap();
    let Some(Tile::Container(Container::Linear(rows))) = win.tree.tiles.get(root) else {
        panic!("expected stacked root");
    };
    assert_eq!(
        rows.dir,
        LinearDir::Vertical,
        "splitting bottom pane must preserve the top/bottom root"
    );
    assert_eq!(rows.children.len(), 2);
    let first_tile = win.find_first_pane_in(rows.children[0]).unwrap();
    assert!(matches!(win.tree.tiles.get(first_tile), Some(Tile::Pane(id)) if *id == first));
    let Some(Tile::Container(Container::Linear(bottom_pair))) =
        win.tree.tiles.get(rows.children[1])
    else {
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
    let Some(Tile::Container(Container::Linear(pair))) =
        win.tree.tiles.get(win.tree.root().unwrap())
    else {
        panic!("expected side-by-side split");
    };
    assert_eq!(pair.children.len(), 2);
    let widths = pair.shares.split(&pair.children, 1000.0);
    assert!(
        (widths[0] - widths[1]).abs() < 1.0,
        "ordinary first split must be equal: {widths:?}"
    );
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
    assert_eq!(
        serde_json::to_value(&h.app.windows[original].tree).unwrap(),
        tree
    );
    h.press_key(egui::Key::H, egui::Modifiers::COMMAND);
    h.run_frames(2);
    assert_eq!(h.app.active_window, original);
    assert!(h
        .app
        .pane_qualifies_for_input(h.app.active_focused_pane_id().unwrap()));
}

#[test]
fn ordinary_splits_resized_nested_tree_survives_save_reload() {
    use egui_tiles::{Container, Tile};
    let mut h = HostHarness::new();
    let first = h.add_test_pane();
    h.app.windows[0]
        .panes
        .get_mut(&first)
        .unwrap()
        .as_view_mut()
        .unwrap()
        .manifest_id = "agents".into();
    h.focus_pane(first);
    h.run_frames(2);
    h.press_key(
        egui::Key::D,
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
    );
    h.run_frames(2);
    h.press_key(egui::Key::D, egui::Modifiers::COMMAND);
    h.run_frames(2);
    let win = &mut h.app.windows[h.app.active_window];
    let root = win.tree.root().unwrap();
    let Some(Tile::Container(Container::Linear(rows))) = win.tree.tiles.get_mut(root) else {
        panic!("rows")
    };
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
    assert_eq!(
        serde_json::to_value(&after.windows[0].tree).unwrap(),
        serde_json::to_value(&before.windows[0].tree).unwrap()
    );
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

// ── Stint 1230: every tree mutation leaves its window settled ─────────────

/// Window 0 holds `[a | b]` as bare panes with `a` focused, the shape a
/// mutation sees before any frame wraps panes in tabs. Window 1, to its right
/// in the same context, holds one pane `c`. Returns `(a, b, c)`.
fn two_window_fixture(h: &mut HostHarness) -> (PaneId, PaneId, PaneId) {
    let a = h.add_test_pane();
    let b = h.add_test_pane();
    let window = &mut h.app.windows[0];
    let a_tile = window.tree.tiles.find_pane(&a).expect("a has a tile");
    let b_tile = window.tree.tiles.find_pane(&b).expect("b has a tile");
    let root = window
        .tree
        .tiles
        .insert_horizontal_tile(vec![a_tile, b_tile]);
    window.tree.root = Some(root);
    window.focused_pane = Some(a_tile);
    let context_id = window.context_id;
    let window_id = h.app.next_window_id;
    h.app.next_window_id += 1;
    h.app.windows.push(crate::host::context::Window {
        name: "right".into(),
        path: crate::testing::scratch_context_root("stint-1230-right"),
        tree: egui_tiles::Tree::empty("stint_1230_right"),
        panes: std::collections::HashMap::new(),
        focused_pane: None,
        zoomed_pane: None,
        grid_x: 1,
        grid_y: 0,
        window_id,
        context_id,
    });
    let (c_tile, c) = h.app.add_view_pane_in_window(1, "right");
    h.app.windows[1].focused_pane = Some(c_tile);
    h.app.active_window = 0;
    (a, b, c)
}

#[test]
fn split_focused_leaves_window_settled() {
    let mut h = HostHarness::new();
    let first = h.add_test_pane();
    h.focus_pane(first);
    h.run_frames(2);
    h.app
        .split_focused(true, None, false, false, None)
        .expect("split spawns a terminal");
    let w = h.app.active_window;
    h.assert_window_settled(w, "split_focused");
}

/// A view launched into an empty context becomes the tree root (the
/// `place_view_pane` root path in `src/pane_ops/create.rs`).
#[test]
fn view_launched_into_an_empty_context_leaves_window_settled() {
    let mut h = HostHarness::new();
    assert!(
        h.app.windows[0].focused_pane.is_none(),
        "precondition: the harness window starts empty"
    );
    h.app
        .launch_view_by_id_with_layout("agents", None, &[], None)
        .expect("agents view launches");
    h.assert_window_settled(0, "view launched as root pane");
}

/// An agent worker spawned into an empty context takes focus through the
/// `spawn_agent_worker_pane` split-insert path.
#[test]
fn agent_worker_spawned_into_an_empty_context_leaves_window_settled() {
    let mut h = HostHarness::new();
    let ws = h.workspace_root();
    let agent = ws.join(".plexi/agents/settle-test");
    std::fs::create_dir_all(&agent).expect("agent dir");
    std::fs::write(agent.join("AGENT.md"), "Test agent").expect("AGENT.md");
    std::fs::write(
        agent.join("settings.toml"),
        "[agent]\nid = \"settle-test\"\ndisplay_name = \"Settle test\"\ndefault_tier = \"medium\"\n[harness]\ncli = \"claude\"\n",
    )
    .expect("settings.toml");
    assert!(
        h.app.windows[0].focused_pane.is_none(),
        "precondition: the harness window starts empty"
    );
    h.app
        .spawn_agent_worker_pane(
            "settle-test",
            "claude",
            "sleep 30",
            "test",
            None,
            false,
            None,
            Some(ws),
            None,
            None,
            false,
            None,
        )
        .expect("agent worker spawns");
    h.assert_window_settled(0, "agent worker spawned into an empty context");
}

#[test]
fn new_tab_leaves_window_settled() {
    let mut h = HostHarness::new();
    let (a, _b, _c) = two_window_fixture(&mut h);
    assert!(h.app.windows[0].tree.tiles.find_pane(&a).is_some());
    h.app.new_tab(0, None, false, None, false, false);
    assert_eq!(h.app.windows[0].panes.len(), 3, "new_tab adds a pane");
    h.assert_window_settled(0, "new_tab");
}

#[test]
fn move_focused_pane_to_adjacent_window_leaves_both_windows_settled() {
    let mut h = HostHarness::new();
    let (a, _b, _c) = two_window_fixture(&mut h);
    assert!(h
        .app
        .move_focused_pane_to_adjacent_window(crate::host::keys::Direction::Right));
    assert_eq!(h.app.active_window, 1);
    assert!(h.app.windows[1].panes.contains_key(&a));
    h.assert_window_settled(1, "move_focused_pane_to_adjacent_window destination");
    h.assert_window_settled(0, "move_focused_pane_to_adjacent_window source");
}

#[test]
fn send_pane_to_adjacent_window_leaves_source_focus_on_a_pane() {
    let mut h = HostHarness::new();
    let (a, b, _c) = two_window_fixture(&mut h);
    assert!(h
        .app
        .send_pane_to_adjacent_window(crate::host::keys::Direction::Right));
    assert_eq!(h.app.active_window, 0, "send keeps focus in the source");
    assert!(h.app.windows[1].panes.contains_key(&a));
    h.assert_window_settled(0, "send_pane_to_adjacent_window source");
    assert_eq!(h.app.active_focused_pane_id(), Some(b));
    h.assert_window_settled(1, "send_pane_to_adjacent_window destination");
}

#[test]
fn move_focused_pane_to_row_boundary_leaves_new_window_settled() {
    let mut h = HostHarness::new();
    let (a, _b, _c) = two_window_fixture(&mut h);
    assert!(h.app.move_focused_pane_to_row_boundary(true));
    let w = h.app.active_window;
    assert!(h.app.windows[w].panes.contains_key(&a));
    h.assert_window_settled(w, "move_focused_pane_to_row_boundary new window");
    h.assert_window_settled(0, "move_focused_pane_to_row_boundary source");
}

#[test]
fn set_context_columns_leaves_window_settled() {
    let mut h = HostHarness::new();
    let a = h.add_test_pane();
    let b = h.add_test_pane();
    let c = h.add_test_pane();
    let context_id = h.app.windows[0].context_id;
    h.app
        .set_context_columns(context_id, vec![vec![a], vec![b, c]]);
    h.assert_window_settled(0, "set_context_columns");
}

/// egui simplifies the tree at the start of `Tree::ui`. A bare pane focused by
/// a path outside the settling method must still be focused by pane, not by
/// the Tabs container that took over its tile id, when the frame ends.
#[test]
fn render_frame_leaves_focus_on_a_pane_after_egui_simplify() {
    let mut h = HostHarness::new();
    let first = h.add_test_pane();
    h.focus_pane(first);
    h.run_frames(1);
    let second = h.add_test_pane();
    let window = &mut h.app.windows[0];
    let second_tile = window
        .tree
        .tiles
        .find_pane(&second)
        .expect("second has a tile");
    let root = window.tree.root.expect("window has a root");
    let split = window
        .tree
        .tiles
        .insert_horizontal_tile(vec![root, second_tile]);
    window.tree.root = Some(split);
    window.focused_pane = Some(second_tile);
    h.run_frames(1);
    assert!(
        focus_is_pane(&h, 0),
        "focus must name a pane after the frame's simplify"
    );
    assert_eq!(h.app.active_focused_pane_id(), Some(second));
}

// ── Stint 1340: a pane click clears the multi-pane selection ──────────────

/// Window 0 holds three side-by-side panes `[a | b | c]`, laid out, with `a`
/// and `b` selected (Cmd+Shift+L from `a`) and `b` focused.
fn selection_fixture(h: &mut HostHarness) -> [PaneId; 3] {
    let panes = [h.add_test_pane(), h.add_test_pane(), h.add_test_pane()];
    let context_id = h.app.windows[0].context_id;
    h.app
        .set_context_columns(context_id, panes.iter().map(|pane| vec![*pane]).collect());
    h.focus_pane(panes[0]);
    h.run_frames(3);
    h.app
        .extend_pane_selection(crate::host::keys::Direction::Right);
    let window_id = h.app.windows[0].window_id;
    let selected = &h
        .app
        .host
        .pane_selection(window_id)
        .expect("selection")
        .panes;
    assert_eq!(
        selected,
        &vec![panes[0], panes[1]],
        "fixture selects a and b"
    );
    h.run_frames(1);
    panes
}

fn pane_center(h: &HostHarness, pane: PaneId) -> egui::Pos2 {
    let tree = &h.app.windows[0].tree;
    let tile = tree.tiles.find_pane(&pane).expect("pane tile");
    tree.tiles.rect(tile).expect("pane laid out").center()
}

fn pointer_frame(h: &mut HostHarness, pos: egui::Pos2, pressed: Option<bool>, hovering: bool) {
    let mut events = vec![egui::Event::PointerMoved(pos)];
    if let Some(pressed) = pressed {
        events.push(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
    }
    h.frame(egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1280.0, 800.0),
        )),
        events,
        hovered_files: if hovering {
            vec![egui::HoveredFile::default()]
        } else {
            Vec::new()
        },
        ..Default::default()
    });
}

fn click_pane(h: &mut HostHarness, pane: PaneId) {
    let pos = pane_center(h, pane);
    pointer_frame(h, pos, Some(true), false);
    pointer_frame(h, pos, Some(false), false);
}

#[test]
fn clicking_a_selected_pane_clears_selection_and_focuses_it() {
    let mut h = HostHarness::new();
    let [a, _b, _c] = selection_fixture(&mut h);
    click_pane(&mut h, a);
    let window_id = h.app.windows[0].window_id;
    assert!(
        h.app.host.pane_selection(window_id).is_none(),
        "a click on a selected pane clears the multi-selection"
    );
    assert_eq!(h.app.active_focused_pane_id(), Some(a));
}

#[test]
fn clicking_an_unselected_pane_clears_selection_and_focuses_it() {
    let mut h = HostHarness::new();
    let [_a, _b, c] = selection_fixture(&mut h);
    click_pane(&mut h, c);
    let window_id = h.app.windows[0].window_id;
    assert!(
        h.app.host.pane_selection(window_id).is_none(),
        "a click on an unselected pane clears the multi-selection"
    );
    assert_eq!(h.app.active_focused_pane_id(), Some(c));
}

#[test]
fn file_drag_hover_over_a_pane_keeps_selection() {
    let mut h = HostHarness::new();
    let [_a, _b, c] = selection_fixture(&mut h);
    let pos = pane_center(&h, c);
    pointer_frame(&mut h, pos, None, true);
    pointer_frame(&mut h, pos, None, true);
    let window_id = h.app.windows[0].window_id;
    assert!(
        h.app.host.pane_selection(window_id).is_some(),
        "hovering a file drag over a pane must not clear the selection"
    );
}
