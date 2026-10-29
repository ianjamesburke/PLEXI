use crate::testing::HostHarness;

fn command() -> egui::Modifiers {
    egui::Modifiers::MAC_CMD | egui::Modifiers::COMMAND
}

fn grid(h: &mut HostHarness) -> [u64; 4] {
    let panes = [
        h.add_test_pane(),
        h.add_test_pane(),
        h.add_test_pane(),
        h.add_test_pane(),
    ];
    for id in panes {
        h.app.windows[0]
            .panes
            .get_mut(&id)
            .unwrap()
            .as_app_mut()
            .unwrap()
            .manifest_id = "agents".into();
    }
    let context = h.app.router.active().context_id;
    h.app.set_context_columns(
        context,
        vec![vec![panes[0], panes[1]], vec![panes[2], panes[3]]],
    );
    h.app.focus_pane_in_strip(panes[0]);
    h.run_frames(2);
    panes
}

#[test]
fn wrap_selection_preserves_layout_portal_and_workspace_roundtrip() {
    let mut h = HostHarness::new();
    let [a, b, c, d] = grid(&mut h);
    let parent = h.app.router.active().context_id;
    let anchor_tile = h.app.windows[0].tree.tiles.find_pane(&a).unwrap();
    let select = command() | egui::Modifiers::SHIFT;
    for key in [egui::Key::J, egui::Key::K, egui::Key::L] {
        h.press_key(key, select);
    }
    h.press_key(egui::Key::Enter, command());
    let child = h.app.router.active().context_id;
    assert_ne!(child, parent, "Cmd+Enter must enter the wrapped child");
    let child_strip = h.app.strip_for_context(child);
    assert_eq!(
        child_strip
            .columns
            .iter()
            .map(|col| col.panes.clone())
            .collect::<Vec<_>>(),
        vec![vec![a, b], vec![c]]
    );
    let parent_window = h
        .app
        .windows
        .iter()
        .find(|w| w.context_id == parent)
        .unwrap();
    let egui_tiles::Tile::Pane(portal) = parent_window.tree.tiles.get(anchor_tile).unwrap() else {
        panic!("anchor must remain a pane slot")
    };
    assert_eq!(
        parent_window.panes[portal]
            .as_portal()
            .unwrap()
            .target_context_id,
        child
    );
    assert_eq!(parent_window.panes.len(), 2);
    assert!(parent_window.panes.contains_key(&d));
    assert_eq!(
        h.app
            .strip_for_context(parent)
            .columns
            .iter()
            .map(|col| col.panes.clone())
            .collect::<Vec<_>>(),
        vec![vec![d]]
    );
    // Compatibility strips exclude portals; the anchor tile above verifies the parent portal.
    let expected_parent = h.app.strip_for_context(parent);
    assert!(
        h.app.save_workspace_now(),
        "wrap must remain immediately saveable"
    );
    let profile = h.profile_dir().to_path_buf();
    let saved =
        crate::workspace::WorkspaceFile::load_from(&profile).expect("valid wrapped workspace");
    assert_eq!(saved.strips.len(), saved.contexts.len());
    let restored = HostHarness::from_profile_dir(profile);
    assert_eq!(
        restored.app.strip_for_context(child).columns,
        child_strip.columns
    );
    assert_eq!(
        restored.app.strip_for_context(parent).columns,
        expected_parent.columns
    );
}

#[test]
fn wrap_zoomed_split_enters_child_with_two_panes() {
    let mut h = HostHarness::new();
    let panes = grid(&mut h);
    let parent = h.app.router.active().context_id;
    h.app.toggle_pane_zoom();
    h.press_key(egui::Key::D, command());
    assert_ne!(h.app.router.active().context_id, parent);
    let child = &h.app.windows[h.app.active_window];
    assert_eq!(child.panes.len(), 2);
    assert!(child.panes.contains_key(&panes[0]));
    assert!(child.zoomed_pane.is_none());
}

#[test]
fn wrap_rejects_invalid_sets_without_mutation_and_explains_why() {
    let mut h = HostHarness::new();
    let panes = grid(&mut h);
    let parent = h.app.router.active().context_id;
    let before = serde_json::to_value(&h.app.windows[0].tree).unwrap();
    let next = h.app.next_window_id;
    for (selected, anchor, message) in [
        (panes.to_vec(), panes[0], "Leave at least one"),
        (vec![panes[0], u64::MAX], panes[0], "same window"),
        (vec![panes[0]], panes[1], "anchor"),
    ] {
        assert!(!h.app.wrap_panes_to_subcontext(&selected, anchor, None));
        assert!(h
            .app
            .attention_queue
            .transient_hint
            .as_ref()
            .unwrap()
            .0
            .contains(message));
        assert_eq!(h.app.next_window_id, next);
        assert_eq!(h.app.router.active().context_id, parent);
        assert_eq!(
            serde_json::to_value(&h.app.windows[0].tree).unwrap(),
            before
        );
        assert_eq!(h.app.windows[0].panes.len(), 4);
    }
    assert!(h.app.wrap_panes_to_subcontext(&[panes[0]], panes[0], None));
    let portal = *h.app.windows[0]
        .panes
        .iter()
        .find(|(_, pane)| pane.as_portal().is_some())
        .unwrap()
        .0;
    assert!(!h
        .app
        .wrap_panes_to_subcontext(&[portal, panes[1]], portal, None));
    assert!(h
        .app
        .attention_queue
        .transient_hint
        .as_ref()
        .unwrap()
        .0
        .contains("portal"));
    assert_eq!(h.app.windows[0].panes.len(), 4);
}

#[test]
fn wrap_selection_cancels_on_escape_navigation_enter_and_selected_close() {
    let mut h = HostHarness::new();
    let [a, b, c, d] = grid(&mut h);
    let window = h.app.windows[0].window_id;
    let select = command() | egui::Modifiers::SHIFT;
    h.press_key(egui::Key::J, select);
    assert_eq!(h.app.host.pane_selection(window).unwrap().anchor, a);
    assert_eq!(h.app.host.pane_selection(window).unwrap().panes, vec![a, b]);
    assert_eq!(h.app.focused_pane_id(), Some(b));
    h.press_key(egui::Key::Escape, egui::Modifiers::NONE);
    assert!(h.app.host.pane_selection(window).is_none());
    assert_eq!(
        h.app.windows[0].panes.len(),
        4,
        "Escape must not close the selected app"
    );
    h.press_key(egui::Key::K, select);
    h.press_key(egui::Key::L, command());
    assert!(h.app.host.pane_selection(window).is_none());
    assert_eq!(h.app.focused_pane_id(), Some(c));
    h.press_key(egui::Key::J, select);
    h.app.close_pane_by_id(a);
    assert!(
        h.app.host.pane_selection(window).is_some(),
        "unselected close retains selection"
    );
    h.app.close_pane_by_id(d);
    assert!(h.app.host.pane_selection(window).is_none());
}

#[test]
fn wrap_selection_single_pane_enter_keeps_zoom_behavior_and_clears_selection() {
    let mut h = HostHarness::new();
    let [a, ..] = grid(&mut h);
    let window = h.app.windows[0].window_id;
    h.press_key(egui::Key::H, command() | egui::Modifiers::SHIFT);
    assert_eq!(h.app.host.pane_selection(window).unwrap().panes, vec![a]);
    h.press_key(egui::Key::Enter, command());
    assert!(h.app.host.pane_selection(window).is_none());
    assert!(h.app.windows[0].zoomed_pane.is_some());
    assert_eq!(h.app.router.len(), 1);
}

#[test]
fn wrap_zoomed_only_pane_is_rejected_without_splitting() {
    let mut h = HostHarness::new();
    let pane = h.add_test_pane();
    h.app.focus_pane_in_strip(pane);
    // A zoomed window can lose its other panes through background closure.
    let tile = h.app.windows[0].focused_pane.unwrap();
    h.app.windows[0].zoom_to(tile);
    h.press_key(egui::Key::D, command());
    assert_eq!(h.app.router.len(), 1);
    assert_eq!(h.app.windows[0].panes.len(), 1);
    assert!(h.app.windows[0].zoomed_pane.is_some());
    assert!(h
        .app
        .attention_queue
        .transient_hint
        .as_ref()
        .unwrap()
        .0
        .contains("Leave at least one"));
}

#[test]
fn wrap_selection_stays_window_local_and_does_not_cross_boundaries() {
    let mut h = HostHarness::new();
    let [a, b, c, d] = grid(&mut h);
    let first_window = h.app.windows[0].window_id;
    h.press_key(egui::Key::J, command() | egui::Modifiers::SHIFT);
    h.app.new_page_right();
    h.run_frames(2);
    let second_window = h.app.windows[h.app.active_window].window_id;
    assert!(h.app.host.pane_selection(second_window).is_none());
    h.press_key(egui::Key::H, command() | egui::Modifiers::SHIFT);
    assert_eq!(h.app.windows[h.app.active_window].window_id, second_window);
    assert_eq!(
        h.app.host.pane_selection(first_window).unwrap().panes,
        vec![a, b]
    );
    h.app.focus_pane_in_strip(c);
    h.press_key(egui::Key::J, command() | egui::Modifiers::SHIFT);
    assert_eq!(h.app.host.pane_selection(first_window).unwrap().anchor, a);
    assert_eq!(
        h.app.host.pane_selection(first_window).unwrap().panes,
        vec![a, b, d]
    );
    assert_eq!(
        h.app
            .host
            .pane_selection(second_window)
            .unwrap()
            .panes
            .len(),
        1
    );
}

#[test]
fn wrap_keeps_split_proportions_and_nonfirst_anchor() {
    let mut h = HostHarness::new();
    let [a, b, c, d] = grid(&mut h);
    let tree = &mut h.app.windows[0].tree;
    let egui_tiles::Tile::Container(egui_tiles::Container::Linear(root)) =
        tree.tiles.get_mut(tree.root.unwrap()).unwrap()
    else {
        panic!("horizontal grid root")
    };
    root.shares[root.children[0]] = 3.0;
    root.shares[root.children[1]] = 2.0;
    let left = root.children[0];
    let egui_tiles::Tile::Container(egui_tiles::Container::Linear(stack)) =
        tree.tiles.get_mut(left).unwrap()
    else {
        panic!("vertical stack")
    };
    stack.shares[stack.children[0]] = 2.0;
    stack.shares[stack.children[1]] = 1.0;
    let anchor_slot = tree.tiles.find_pane(&c).unwrap();
    assert!(h.app.wrap_panes_to_subcontext(&[c, b, a], c, None));
    assert_eq!(h.app.focused_pane_id(), Some(c));
    assert!(
        matches!(h.app.windows[0].tree.tiles.get(anchor_slot), Some(egui_tiles::Tile::Pane(id)) if h.app.windows[0].panes[id].as_portal().is_some())
    );
    assert!(h.app.windows[0].panes.contains_key(&d));
    assert!(h.app.save_workspace_now(), "save before rendering child");
    h.run_frames(2);
    let child = &h.app.windows[h.app.active_window];
    let rect = |id| {
        child
            .tree
            .tiles
            .rect(child.tree.tiles.find_pane(&id).unwrap())
            .unwrap()
    };
    let (ra, rb, rc) = (rect(a), rect(b), rect(c));
    assert!((ra.width() / rc.width() - 1.5).abs() < 0.02);
    assert!((ra.height() / rb.height() - 2.0).abs() < 0.02);
    assert!(ra.bottom() <= rb.top());
    assert!(ra.right() <= rc.left());
    assert!(h.app.save_workspace_now(), "save after rendering child");
}

#[test]
fn bracket_shortcuts_cycle_tabs_without_using_focus_history() {
    let mut h = HostHarness::new();
    let a = h.add_test_pane();
    let b = h.add_test_pane();
    let (ta, tb) = {
        let tree = &mut h.app.windows[0].tree;
        let ta = tree.tiles.find_pane(&a).unwrap();
        let tb = tree.tiles.find_pane(&b).unwrap();
        tree.root = Some(tree.tiles.insert_tab_tile(vec![ta, tb]));
        (ta, tb)
    };
    h.app.windows[0].navigate_to(ta);
    h.run_frames(2);
    h.press_key(egui::Key::CloseBracket, command());
    assert_eq!(h.app.windows[0].focused_pane, Some(tb));
    h.press_key(egui::Key::OpenBracket, command());
    assert_eq!(h.app.windows[0].focused_pane, Some(ta));
}
