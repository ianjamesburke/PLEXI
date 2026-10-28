use crate::testing::HostHarness;

fn request(h: &mut HostHarness, value: serde_json::Value) -> serde_json::Value {
    let path = h.profile_dir().join("parity-response.json");
    let mut value = value;
    value["response_file"] = path.to_string_lossy().as_ref().into();
    let request = serde_json::from_value(value).expect("CLI request must be recognized");
    h.inject_ipc(request);
    h.hidden_frame();
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn cli_parity_hide_reveal_and_zoom_use_keyboard_handlers() {
    let mut h = HostHarness::new();
    let pane = h.add_test_pane();
    let other = h.add_test_pane();
    tile_pair(&mut h, pane, other);
    h.focus_pane(pane).run_frames(1);
    for hidden in [true, false] {
        let reply = request(
            &mut h,
            serde_json::json!({"type":"pane_visibility", "pane_id":pane, "hidden":hidden}),
        );
        assert_eq!(reply["ok"], true);
        assert_eq!(reply["pane_id"], pane);
        assert_eq!(reply["hidden"], hidden);
        assert_eq!(h.app.windows[0].panes[&pane].is_hidden(), hidden);
    }
    h.press_key(egui::Key::U, egui::Modifiers::COMMAND);
    assert!(h.app.windows[0].panes[&pane].is_hidden());
    let reveal_reply = request(
        &mut h,
        serde_json::json!({"type":"pane_visibility", "pane_id":pane, "hidden":false}),
    );
    assert_eq!(reveal_reply["ok"], true);
    assert_eq!(reveal_reply["pane_id"], pane);
    assert_eq!(reveal_reply["hidden"], false);
    assert!(!h.app.windows[0].panes[&pane].is_hidden());
    let reply = request(
        &mut h,
        serde_json::json!({"type":"pane_zoom", "pane_id":pane}),
    );
    assert_eq!(reply["ok"], true);
    assert!(h.app.windows[0].zoomed_pane.is_some());
    request(
        &mut h,
        serde_json::json!({"type":"pane_zoom", "pane_id":pane}),
    );
    assert!(h.app.windows[0].zoomed_pane.is_none());
    h.press_key(
        egui::Key::Enter,
        egui::Modifiers {
            command: true,
            shift: true,
            ..Default::default()
        },
    );
    assert!(h.app.windows[0].zoomed_pane.is_some());
    request(
        &mut h,
        serde_json::json!({"type":"pane_zoom", "pane_id":pane}),
    );
    assert!(h.app.windows[0].zoomed_pane.is_none());
}

#[test]
fn cli_parity_context_rename_park_unpark_and_close() {
    let mut h = HostHarness::new();
    let id = h.app.router.active().context_id;
    assert_eq!(
        request(
            &mut h,
            serde_json::json!({"type":"context_rename", "context_id":id, "name":"renamed"})
        )["ok"],
        true
    );
    assert_eq!(h.app.router.active().name.displayed(), "renamed");
    for parked in [true, false] {
        assert_eq!(
            request(
                &mut h,
                serde_json::json!({"type":"context_park", "context_id":id, "parked":parked})
            )["ok"],
            true
        );
        assert_eq!(h.app.router.active().parked, parked);
    }
    // The last context is protected by the same close policy as the shortcut.
    assert!(request(
        &mut h,
        serde_json::json!({"type":"context_close", "context_id":id})
    )["error"]
        .is_string());
    assert_eq!(h.app.router.len(), 1);
    assert!(request(
        &mut h,
        serde_json::json!({"type":"context_rename", "context_id":u64::MAX, "name":"wrong"})
    )["error"]
        .is_string());
    assert_eq!(h.app.router.active().name.displayed(), "renamed");
}

#[test]
fn cli_parity_reload_focus_history_and_swap() {
    let mut h = HostHarness::new();
    let a = h.add_test_pane();
    let b = h.add_test_pane();
    tile_pair(&mut h, a, b);
    h.focus_pane(a).run_frames(1);
    let tile_a = h.app.windows[0].tree.tiles.find_pane(&a).unwrap();
    let tile_b = h.app.windows[0].tree.tiles.find_pane(&b).unwrap();
    h.app.pane_navigate(b);
    assert_eq!(
        request(
            &mut h,
            serde_json::json!({"type":"focus_history", "forward":false})
        )["ok"],
        true
    );
    assert_eq!(h.app.fleet_focused_pane(), Some(a));
    assert_eq!(
        request(
            &mut h,
            serde_json::json!({"type":"focus_history", "forward":true})
        )["ok"],
        true
    );
    assert_eq!(h.app.fleet_focused_pane(), Some(b));
    let context = h.app.router.active().context_id;
    let before: Vec<_> = h.app.strip_for_context(context).columns.iter().map(|column| column.panes.clone()).collect();
    assert_eq!(
        request(
            &mut h,
            serde_json::json!({"type":"pane_swap", "pane_id":b, "direction":"left"})
        )["ok"],
        true
    );
    assert_eq!(h.app.windows[0].tree.tiles.find_pane(&b), Some(tile_a));
    assert_eq!(h.app.strip_for_context(context).focused_pane, Some(b));
    assert_eq!(h.app.strip_for_context(context).columns[0].panes, vec![b]);
    h.press_key(
        egui::Key::L,
        egui::Modifiers {
            command: true,
            ctrl: true,
            ..Default::default()
        },
    );
    assert_eq!(h.app.windows[0].tree.tiles.find_pane(&b), Some(tile_b));
    let after = h.app.strip_for_context(context);
    assert_eq!(after.columns.iter().map(|column| column.panes.clone()).collect::<Vec<_>>(), before);
    assert_eq!(after.focused_pane, Some(b));
    std::fs::write(
        h.profile_dir().join("config.toml"),
        "[theme]\naccent = \"#12ab34\"\n",
    )
    .unwrap();
    assert_eq!(
        request(&mut h, serde_json::json!({"type":"reload_config"}))["ok"],
        true
    );
    assert_eq!(
        h.app.colors.accent,
        egui::Color32::from_rgb(0x12, 0xab, 0x34)
    );
    h.app.colors.accent = egui::Color32::BLACK;
    h.press_key(
        egui::Key::Comma,
        egui::Modifiers {
            command: true,
            shift: true,
            ..Default::default()
        },
    );
    assert_eq!(
        h.app.colors.accent,
        egui::Color32::from_rgb(0x12, 0xab, 0x34)
    );
}

fn tile_pair(h: &mut HostHarness, a: u64, b: u64) -> (egui_tiles::TileId, egui_tiles::TileId) {
    let tree = &mut h.app.windows[0].tree;
    let a = tree.tiles.find_pane(&a).unwrap();
    let b = tree.tiles.find_pane(&b).unwrap();
    tree.root = Some(tree.tiles.insert_horizontal_tile(vec![a, b]));
    (a, b)
}

#[test]
fn cli_parity_context_close_matches_shortcut_confirmation_and_empty_close() {
    let mut h = HostHarness::new();
    let pane = h.add_test_pane();
    h.focus_pane(pane).run_frames(1);
    let current = h.app.router.active().context_id;
    let other = crate::testing::reserve_test_context_id();
    h.app.router.push(crate::host::context::Context {
        name: "Other".into(),
        root: h.workspace_root(),
        description: None,
        context_id: other,
        parent_id: None,
        depth: 0,
        parked: false,
    });
    h.app.config.confirm_context_close = Some(true);
    let reply = request(
        &mut h,
        serde_json::json!({"type":"context_close", "context_id":current}),
    );
    assert_eq!(reply["pending_confirmation"], true);
    assert!(h.app.pending_context_close.is_some());
    h.app.pending_context_close = None;
    h.press_key(
        egui::Key::W,
        egui::Modifiers {
            command: true,
            shift: true,
            ..Default::default()
        },
    );
    assert!(h.app.pending_context_close.is_some());
    h.app.pending_context_close = None;
    let reply = request(
        &mut h,
        serde_json::json!({"type":"context_close", "context_id":other}),
    );
    assert_eq!(reply["pending_confirmation"], false);
    assert_eq!(h.app.router.len(), 1);
    assert_eq!(h.app.router.active().context_id, current);
}

#[test]
fn workspace_snapshot_reports_strip_geometry_without_pane_text() {
    let mut h = HostHarness::new();
    let a = h.add_test_pane();
    let b = h.add_test_pane();
    let context = h.app.router.active().context_id;
    h.app.windows[0].panes.get_mut(&a).unwrap().as_app_mut().unwrap().name = "private sentinel text".into();
    h.app.set_context_columns(context, vec![vec![a], vec![b]]);
    h.app.focus_pane_in_strip(a);
    h.run_frames(2);
    let snapshot = request(&mut h, serde_json::json!({"type":"workspace_snapshot"}));
    assert_eq!(snapshot["context_id"], context);
    assert!(snapshot["geometry_generation"].as_u64().unwrap() > 0);
    assert_eq!(snapshot["columns"][0]["panes"][0], a);
    assert!(snapshot["panes"].as_array().unwrap().iter().any(|pane| pane["pane_id"] == a && pane["bounds"].is_object()));
    assert!(snapshot["input_owner"]["kind"].is_string());
    assert!(!snapshot.to_string().contains("private sentinel text"));
    let generation = snapshot["geometry_generation"].as_u64().unwrap();
    h.app.focus_pane_in_strip(b);
    let next = request(&mut h, serde_json::json!({"type":"workspace_snapshot"}));
    assert!(next["geometry_generation"].as_u64().unwrap() > generation);
    assert_eq!(next["focused_pane"], b);
}

#[test]
fn cli_swap_and_shortcut_exchange_stacked_panes_by_identity() {
    let mut h = HostHarness::new();
    let a = h.add_test_pane();
    let b = h.add_test_pane();
    let c = h.add_test_pane();
    let context = h.app.router.active().context_id;
    h.app.set_context_columns(context, vec![vec![a, b], vec![c]]);
    h.app.focus_pane_in_strip(b);
    h.run_frames(2);
    assert_eq!(request(&mut h, serde_json::json!({"type":"pane_swap", "pane_id":b, "direction":"right"}))["ok"], true);
    let swapped = h.app.strip_for_context(context);
    assert_eq!(swapped.focused_pane, Some(b));
    let cli_columns: Vec<_> = swapped.columns.iter().map(|column| column.panes.clone()).collect();
    assert_eq!(cli_columns, vec![vec![a, c], vec![b]]);
    h.app.set_context_columns(context, vec![vec![a, b], vec![c]]);
    h.app.focus_pane_in_strip(b);
    h.run_frames(2);
    h.press_key(egui::Key::L, egui::Modifiers { command: true, ctrl: true, ..Default::default() });
    let shortcut = h.app.strip_for_context(context);
    assert_eq!(shortcut.columns.iter().map(|column| column.panes.clone()).collect::<Vec<_>>(), cli_columns);
    assert_eq!(shortcut.focused_pane, Some(b));
}

#[test]
fn workspace_snapshot_marks_other_windows_outside_active_view() {
    let mut h = HostHarness::new();
    let first = h.add_test_pane();
    let context = h.app.router.active().context_id;
    h.app.set_context_columns(context, vec![vec![first]]);
    let window_id = h.app.next_window_id;
    h.app.next_window_id += 1;
    h.app.windows.push(crate::host::context::Window {
        name: "second".into(), path: h.workspace_root(), tree: egui_tiles::Tree::empty("second"),
        panes: std::collections::HashMap::new(), focused_pane: None, zoomed_pane: None,
        grid_x: 1, grid_y: 0, window_id, context_id: context,
    });
    let (tile, second) = h.app.add_app_pane_in_window(1, "second");
    h.app.windows[1].focused_pane = Some(tile);
    h.app.active_window = 1;
    h.app.context_active_window.insert(context, window_id);
    h.app.focus_pane_in_strip(second);
    h.run_frames(2);
    let snapshot = request(&mut h, serde_json::json!({"type":"workspace_snapshot"}));
    let panes = snapshot["panes"].as_array().unwrap();
    let other = panes.iter().find(|pane| pane["pane_id"] == first).expect("other-window pane");
    assert_eq!(other["visible"], false);
    assert!(other["bounds"].is_null());
    let active = panes.iter().find(|pane| pane["pane_id"] == second).expect("active-window pane");
    assert_eq!(active["window_id"], window_id);
    assert!(active["bounds"].is_object());
}
