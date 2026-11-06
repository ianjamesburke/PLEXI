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

fn assert_zoomed_split(vertical: bool) {
    let mut h = HostHarness::new();
    let panes = grid(&mut h);
    let parent = h.app.router.active().context_id;
    h.app.toggle_pane_zoom();
    let modifiers = if vertical {
        command() | egui::Modifiers::SHIFT
    } else {
        command()
    };
    h.press_key(egui::Key::D, modifiers);
    h.run_frames(2);
    let child_id = h.app.router.active().context_id;
    assert_ne!(child_id, parent);
    let child = &h.app.windows[h.app.active_window];
    assert_eq!(child.panes.len(), 2);
    assert!(child.panes.contains_key(&panes[0]));
    assert!(child.zoomed_pane.is_none());
    let second = *child.panes.keys().find(|&&id| id != panes[0]).unwrap();
    let rect = |id| {
        child
            .tree
            .tiles
            .rect(child.tree.tiles.find_pane(&id).unwrap())
            .unwrap()
    };
    let (first, second) = (rect(panes[0]), rect(second));
    if vertical {
        assert!(first.bottom() <= second.top());
        assert!((first.left() - second.left()).abs() < 1.0);
    } else {
        assert!(first.right() <= second.left());
        assert!((first.top() - second.top()).abs() < 1.0);
    }
    let parent_window = h
        .app
        .windows
        .iter()
        .find(|w| w.context_id == parent)
        .unwrap();
    assert_eq!(parent_window.panes.len(), 4);
    for pane in &panes[1..] {
        assert!(parent_window.panes.contains_key(pane));
    }
    assert!(parent_window.panes.values().any(|p| p
        .as_portal()
        .is_some_and(|p| p.target_context_id == child_id)));
    h.press_key(egui::Key::Escape, command());
    assert_eq!(h.app.router.active().context_id, parent);
}

#[test]
fn wrap_zoomed_split_enters_child_with_two_panes() {
    assert_zoomed_split(false);
}

#[test]
fn wrap_zoomed_vertical_split_enters_child_with_stacked_panes() {
    assert_zoomed_split(true);
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
    for modifiers in [command(), command() | egui::Modifiers::SHIFT] {
        let mut h = HostHarness::new();
        let pane = h.add_test_pane();
        h.app.focus_pane_in_strip(pane);
        // A zoomed window can lose its other panes through background closure.
        let tile = h.app.windows[0].focused_pane.unwrap();
        h.app.windows[0].zoom_to(tile);
        h.press_key(egui::Key::D, modifiers);
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
}

#[test]
fn wrap_zoomed_portal_is_rejected_without_splitting() {
    for modifiers in [command(), command() | egui::Modifiers::SHIFT] {
        let mut h = HostHarness::new();
        let panes = grid(&mut h);
        let parent = h.app.router.active().context_id;
        assert!(h.app.wrap_panes_to_subcontext(&[panes[0]], panes[0], None));
        h.press_key(egui::Key::Escape, command());
        let portal = *h.app.windows[h.app.active_window]
            .panes
            .iter()
            .find(|(_, pane)| pane.as_portal().is_some())
            .unwrap()
            .0;
        h.app.focus_pane_in_strip(portal);
        let window = &mut h.app.windows[h.app.active_window];
        window.zoom_to(window.tree.tiles.find_pane(&portal).unwrap());
        h.press_key(egui::Key::D, modifiers);
        assert_eq!(h.app.router.active().context_id, parent);
        assert_eq!(h.app.router.len(), 2);
        assert_eq!(h.app.windows[h.app.active_window].panes.len(), 4);
        assert!(h.app.windows[h.app.active_window].zoomed_pane.is_some());
        assert!(h
            .app
            .attention_queue
            .transient_hint
            .as_ref()
            .unwrap()
            .0
            .contains("portal"));
    }
}

#[test]
fn wrap_unzoomed_vertical_split_stays_in_parent() {
    let mut h = HostHarness::new();
    let panes = grid(&mut h);
    let parent = h.app.router.active().context_id;
    h.press_key(egui::Key::D, command() | egui::Modifiers::SHIFT);
    h.run_frames(2);
    assert_eq!(h.app.router.active().context_id, parent);
    assert_eq!(h.app.router.len(), 1);
    let window = &h.app.windows[h.app.active_window];
    assert_eq!(window.panes.len(), 5);
    let second = *window.panes.keys().find(|id| !panes.contains(id)).unwrap();
    let rect = |id| {
        window
            .tree
            .tiles
            .rect(window.tree.tiles.find_pane(&id).unwrap())
            .unwrap()
    };
    assert!(rect(panes[0]).bottom() <= rect(second).top());
    assert!((rect(panes[0]).left() - rect(second).left()).abs() < 1.0);
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

#[test]
fn closing_last_sibling_reveals_child_without_moving_panes() {
    let mut h = HostHarness::new();
    let [a, b, c, outside] = grid(&mut h);
    let parent = h.app.router.active().context_id;
    assert!(h.app.wrap_panes_to_subcontext(&[a, b, c], a, None));
    let child = h.app.router.active().context_id;
    let before = h.app.strip_for_context(child).columns;
    let origin = h.app.origin_for_pane(a).unwrap();
    let resource = crate::host::scope::Scope::Pane { pane_id: a, context_id: child };
    let grant = crate::host::scope::CrossContextGrant {
        granting_context_id: child,
        grantee_context_id: parent,
        reason: "test".into(),
    };
    let before_reach = crate::host::scope::evaluate_reach(&resource, parent, Some(&grant));
    let child_window = h.app.windows.iter().find(|w| w.context_id == child).unwrap();
    let runtime = &child_window.panes[&a].as_app().unwrap().runtime as *const _ as *const ();
    assert!(h.app.zoom_out_of_context());
    assert_eq!(h.app.router.active().context_id, parent);
    h.app.close_pane_by_id(outside);
    assert_eq!(h.app.router.active().context_id, child, "closing the last sibling should reveal the child");
    assert_eq!(h.app.strip_for_context(child).columns, before);
    let child_window = h.app.windows.iter().find(|w| w.context_id == child).unwrap();
    assert!([a, b, c].iter().all(|id| child_window.panes.contains_key(id)));
    assert_eq!(h.app.origin_for_pane(a), Some(origin));
    assert_eq!(crate::host::scope::evaluate_reach(&resource, parent, Some(&grant)), before_reach);
    assert_eq!(&child_window.panes[&a].as_app().unwrap().runtime as *const _ as *const (), runtime);
    h.app.close_pane_by_id(b);
    assert_eq!(h.app.router.active().context_id, child);
    assert!(h.app.origin_for_pane(a).is_some());
}

#[test]
fn close_reveal_preserves_background_focus_and_skips_redundant_ancestor() {
    let mut h = HostHarness::new();
    let [a, b, c, _outside] = grid(&mut h);
    let root = h.app.router.active().context_id;
    assert!(h.app.wrap_panes_to_subcontext(&[a, b, c], a, None));
    let child = h.app.router.active().context_id;
    assert!(h.app.wrap_panes_to_subcontext(&[a, b], a, None));
    let grandchild = h.app.router.active().context_id;
    let before_window = h.app.active_window;
    h.app.close_pane_by_id(c);
    assert_eq!(h.app.router.active().context_id, grandchild);
    assert_eq!(h.app.active_window, before_window, "background close must not steal focus");
    assert!(h.app.zoom_out_of_context());
    assert_eq!(h.app.router.active().context_id, root, "back should skip the redundant child");
    assert!(h.app.router.iter().any(|ctx| ctx.context_id == child && ctx.parent_id == Some(root)));
    assert!(h.app.router.iter().any(|ctx| ctx.context_id == grandchild && ctx.parent_id == Some(child)));
}

#[test]
fn close_reveal_does_not_ignore_hidden_owned_panes() {
    let mut h = HostHarness::new();
    let [a, b, hidden, outside] = grid(&mut h);
    let parent = h.app.router.active().context_id;
    assert!(h.app.wrap_panes_to_subcontext(&[a, b], a, None));
    assert!(h.app.zoom_out_of_context());
    let parent_win = h.app.windows.iter_mut().find(|w| w.context_id == parent).unwrap();
    parent_win.panes.get_mut(&hidden).unwrap().set_hidden(true);
    h.app.close_pane_by_id(outside);
    assert_eq!(h.app.router.active().context_id, parent, "hidden panes still belong to the parent");
}

#[test]
fn close_reveal_survives_workspace_restore() {
    let mut h = HostHarness::new();
    let [a, b, c, outside] = grid(&mut h);
    let parent = h.app.router.active().context_id;
    assert!(h.app.wrap_panes_to_subcontext(&[a, b, c], a, None));
    let child = h.app.router.active().context_id;
    assert!(h.app.zoom_out_of_context());
    h.app.close_pane_by_id(outside);
    assert!(h.app.save_workspace_now());
    let mut restored = HostHarness::from_profile_dir(h.profile_dir().to_path_buf());
    let parent_idx = restored.app.router.position(|ctx| ctx.context_id == parent).unwrap();
    restored.app.switch_workspace(parent_idx);
    assert_eq!(restored.app.router.active().context_id, child);
    assert!(!restored.app.zoom_out_of_context(), "no meaningful ancestor remains");
}

#[test]
fn close_reveal_respects_other_pages_and_missing_targets() {
    let mut h = HostHarness::new();
    let [a, b, c, outside] = grid(&mut h);
    let parent = h.app.router.active().context_id;
    assert!(h.app.wrap_panes_to_subcontext(&[a, b, c], a, None));
    assert!(h.app.zoom_out_of_context());
    let parent_window = h.app.windows.iter().find(|w| w.context_id == parent).unwrap();
    h.app.windows.push(crate::host::context::Window {
        name: String::new(),
        path: parent_window.path.clone(),
        tree: egui_tiles::Tree::empty("empty-second-page"),
        panes: std::collections::HashMap::new(),
        focused_pane: None,
        zoomed_pane: None,
        grid_x: 1,
        grid_y: 0,
        window_id: 987_654_321,
        context_id: parent,
    });
    h.app.close_pane_by_id(outside);
    assert_eq!(h.app.router.active().context_id, parent, "a second page is meaningful");
    h.app.windows.pop();
    let portal = h.app.windows.iter_mut().find(|w| w.context_id == parent).unwrap().panes.values_mut().find_map(|p| p.as_portal_mut()).unwrap();
    portal.target_context_id = u64::MAX;
    h.app.reveal_sole_portal_after_close(parent);
    assert_eq!(h.app.router.active().context_id, parent, "missing targets cannot be revealed");
}

#[test]
fn close_reveal_via_focused_close_preserves_child_page() {
    let mut h = HostHarness::new();
    let [a, b, c, outside] = grid(&mut h);
    let parent = h.app.router.active().context_id;
    assert!(h.app.wrap_panes_to_subcontext(&[a, b, c], a, None));
    let child = h.app.router.active().context_id;
    assert!(h.app.zoom_out_of_context());
    let index = h.app.windows.iter().position(|w| w.context_id == parent).unwrap();
    let tile = h.app.windows[index].tree.tiles.find_pane(&outside).unwrap();
    h.app.windows[index].navigate_to(tile);
    h.app.execute_close_pane();
    assert_eq!(h.app.router.active().context_id, child);
    assert!([a, b, c].iter().all(|id| h.app.windows[h.app.active_window].panes.contains_key(id)));
}

#[test]
fn close_reveal_focus_history_skips_redundant_portal() {
    let mut h = HostHarness::new();
    let [a, b, c, outside] = grid(&mut h);
    let parent = h.app.router.active().context_id;
    assert!(h.app.wrap_panes_to_subcontext(&[a, b, c], a, None));
    let child = h.app.router.active().context_id;
    assert!(h.app.zoom_out_of_context());
    h.app.close_pane_by_id(outside);
    let page = h.app.windows.iter().find(|w| w.context_id == parent).unwrap();
    let tile = page.tree.root.and_then(|root| page.find_first_pane_in(root)).unwrap();
    let entry = (page.window_id, tile);
    h.app.pane_focus_history.clear();
    h.app.pane_focus_history.push(entry);
    h.app.step_focus_history_back();
    assert_eq!(h.app.router.active().context_id, child);
    h.app.pane_focus_future.clear();
    h.app.pane_focus_future.push(entry);
    h.app.step_focus_history_forward();
    assert_eq!(h.app.router.active().context_id, child);
}
