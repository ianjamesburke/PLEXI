use super::*;

fn click_pane_center(h: &mut PlexiUiHarness, pane: u64) {
    let pos = h
        .with_app(|app| app.visible_pane_rect(pane))
        .expect("pane is laid out")
        .center();
    for pressed in [true, false] {
        h.harness().input_mut().events.extend([
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        h.step();
    }
    h.run_steps(2);
}

/// Stint 1251: closing the focused rightmost column focuses the column
/// beside it, not the newer focus-history entry two columns away.
#[test]
fn screenshot_close_focused_pane_focuses_spatial_neighbour() {
    let (mut h, [a, b, c]) = three_column_fixture();
    h.focus_pane(a).unwrap();
    h.run_steps(1);
    h.focus_pane(c).unwrap();
    h.run_steps(2);
    h.save_screenshot(&evidence_png!("_before")).unwrap();
    h.close_pane(c).unwrap();
    h.run_steps(3);
    assert_eq!(
        h.with_app(|app| app.active_focused_pane_id()),
        Some(b),
        "focus lands on the adjacent column, not the history entry"
    );
    h.save_screenshot(&evidence_png!("_after")).unwrap();
}

/// Stint 1340: a click on a pane clears the multi-pane selection and
/// focuses the clicked pane; the selection rings disappear.
#[test]
fn screenshot_pane_click_clears_multi_selection() {
    let (mut h, [a, b, c]) = three_column_fixture();
    h.focus_pane(a).unwrap();
    h.run_steps(1);
    h.with_app_mut(|app| app.extend_pane_selection(crate::host::keys::Direction::Right));
    h.run_steps(2);
    let window = h.with_app(|app| app.windows[app.active_window].window_id);
    assert_eq!(
        h.with_app(|app| app.host.pane_selection(window).map(|s| s.panes.clone())),
        Some(vec![a, b]),
        "fixture selects a and b"
    );
    h.save_screenshot(&evidence_png!("_selected")).unwrap();
    click_pane_center(&mut h, c);
    assert!(
        h.with_app(|app| app.host.pane_selection(window).is_none()),
        "a pane click clears the selection"
    );
    assert_eq!(h.with_app(|app| app.active_focused_pane_id()), Some(c));
    h.save_screenshot(&evidence_png!("_cleared")).unwrap();
}

fn click_pane_center_with(h: &mut PlexiUiHarness, pane: u64, modifiers: egui::Modifiers) {
    let pos = h
        .with_app(|app| app.visible_pane_rect(pane))
        .expect("pane is laid out")
        .center();
    for pressed in [true, false] {
        h.harness().input_mut().modifiers = modifiers;
        h.harness().input_mut().events.extend([
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers,
            },
        ]);
        h.step();
    }
    h.harness().input_mut().modifiers = egui::Modifiers::NONE;
    h.run_steps(2);
}

/// Stint 1458: Cmd+Shift+click adds panes to the selection, and the
/// close shortcut closes all three selected panes at once; the fourth
/// pane is left, focused.
#[test]
fn screenshot_cmd_shift_click_selects_and_close_shortcut_closes_all_selected() {
    let mut h = PlexiUiHarness::new_sized(1200.0, 600.0);
    h.step();
    let panes = [
        add_focused_pane(&mut h),
        add_focused_pane(&mut h),
        add_focused_pane(&mut h),
        add_focused_pane(&mut h),
    ];
    let [a, b, c, d] = panes;
    h.with_app_mut(|app| {
        app.sidebar_visible = false;
        let context_id = app.windows[app.active_window].context_id;
        app.set_context_columns(context_id, panes.iter().map(|p| vec![*p]).collect());
    });
    h.run_steps(3);
    h.focus_pane(a).unwrap();
    h.run_steps(1);
    let extend = egui::Modifiers::COMMAND | egui::Modifiers::MAC_CMD | egui::Modifiers::SHIFT;
    click_pane_center_with(&mut h, b, extend);
    click_pane_center_with(&mut h, c, extend);
    let window = h.with_app(|app| app.windows[app.active_window].window_id);
    assert_eq!(
        h.with_app(|app| app.host.pane_selection(window).map(|s| s.panes.clone())),
        Some(vec![a, b, c]),
        "Cmd+Shift+click extends the selection from the focused pane"
    );
    h.save_screenshot(&evidence_png!("_selected")).unwrap();
    h.with_app_mut(|app| {
        let ctx = app.ctx.clone();
        app.dispatch_host_action(crate::host::keys::Action::ClosePane, &ctx);
    });
    h.run_steps(1);
    let open: Vec<u64> = h.with_app(|app| app.windows[app.active_window].panes.keys().copied().collect());
    assert_eq!(open, vec![d], "all three selected panes closed");
    assert_eq!(h.with_app(|app| app.active_focused_pane_id()), Some(d));
    assert!(h.with_app(|app| app.host.pane_selection(window).is_none()));
    h.save_screenshot(&evidence_png!("_closed")).unwrap();
}

/// Stint 1458 critic: the pane-click layer rule must not cost a pane its
/// own popups. A click on a control in pane B focuses B; a click inside
/// B's own popup (the file browser's View menu, an egui Area) keeps B
/// focused, so keyboard input stays with the pane the user is working in.
#[test]
fn click_in_a_panes_own_popup_keeps_that_pane_focused() {
    let (mut h, [a, b, _c]) = three_column_fixture();
    h.focus_pane(a).unwrap();
    h.run_steps(2);
    let b_rect = h.with_app(|app| app.visible_pane_rect(b)).unwrap();
    let view = h
        .harness()
        .get_all_by_label("View \u{2304}")
        .map(|node| node.rect())
        .find(|rect| b_rect.contains_rect(*rect))
        .expect("B's View menu button");
    press_at(&mut h, view.center());
    assert_eq!(
        h.with_app(|app| app.active_focused_pane_id()),
        Some(b),
        "(a) a click on a control in B focuses B"
    );
    // The popup is wider than B's column, so its item sits over pane C.
    let item = h.harness().get_by_label("Show hidden files").rect();
    let c_rect = h.with_app(|app| app.visible_pane_rect(_c)).unwrap();
    let pos = egui::pos2(c_rect.left() + 8.0, item.center().y);
    assert!(item.contains(pos) && c_rect.contains(pos), "the press lands over C");
    press_at(&mut h, pos);
    assert_eq!(
        h.with_app(|app| app.active_focused_pane_id()),
        Some(b),
        "(b) a click in B's popup keeps B focused; it does not reach C under it"
    );
    assert_eq!(
        h.with_app(|app| app.host_input_owner()),
        crate::app::input_owner::InputOwner::Pane(b),
        "keyboard input stays with B"
    );
}
