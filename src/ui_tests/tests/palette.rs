use super::*;

#[test]
fn screenshot_command_palette_metadata_lane() {
    let mut h = PlexiUiHarness::new_sized(1168.0, 720.0);
    let first_pane_id = add_focused_pane(&mut h);
    let browser_root = h.workspace.path().to_path_buf();
    h.with_app_mut(|app| {
        let second_pane_id = app.host.alloc_pane_id();
        let view_pane = ViewPane {
            incarnation: Default::default(),
            pip_status: None,
            id: second_pane_id,
            runtime: ViewRuntime::Builtin(Box::new(crate::file_browser::FileBrowserView::new(
                browser_root.clone(),
            ))),
            workspace_root: browser_root.clone(),
            permissions: PluginPermissions::builtin(),
            manifest_id: "hidden-test".to_string(),
            name: "Hidden Test App".to_string(),
            pane_group: None,
            linked_pane_id: None,
            overlay_replaced: None,
            agent: None,
            slots: std::collections::HashMap::new(),
            semantic_state: Default::default(),
        };
        let win = &mut app.windows[app.active_window];
        win.panes
            .insert(second_pane_id, Pane::View(Box::new(view_pane)))
            .unwrap();
        let second_tile = win.tree.tiles.insert_pane(second_pane_id);
        if let Some(first_tile) = win.tree.root {
            let root = win
                .tree
                .tiles
                .insert_horizontal_tile(vec![first_tile, second_tile]);
            win.tree.root = Some(root);
            win.focused_pane = Some(first_tile);
        }
        let ctx_idx = app.router.active_idx();
        app.router.get_mut(ctx_idx).name = "Command Palette Metadata".to_string().into();
        let ctx_id = app.router.get(ctx_idx).context_id;
        let extra_a = app.host.alloc_pane_id();
        let extra_b = app.host.alloc_pane_id();
        let mut extra_panes = std::collections::HashMap::new();
        extra_panes.insert(
            extra_a,
            Pane::Portal(Box::new(PortalPane {
                pane_id: extra_a,
                target_context_id: extra_a + 10_000,
                context_state: None,
            })),
        );
        extra_panes.insert(
            extra_b,
            Pane::Portal(Box::new(PortalPane {
                pane_id: extra_b,
                target_context_id: extra_b + 10_000,
                context_state: None,
            })),
        );
        let mut extra_tiles = egui_tiles::Tiles::default();
        let extra_tile_a = extra_tiles.insert_pane(extra_a);
        let extra_tile_b = extra_tiles.insert_pane(extra_b);
        let extra_root = extra_tiles.insert_horizontal_tile(vec![extra_tile_a, extra_tile_b]);
        let extra_window_id = app.next_window_id;
        app.next_window_id += 1;
        app.windows.push(Window {
            name: "palette metadata extra".to_string(),
            path: crate::testing::scratch_context_root("ui"),
            tree: egui_tiles::Tree::new("palette metadata extra", extra_root, extra_tiles),
            panes: (extra_panes).into(),
            focused_pane: Some(extra_tile_a),
            zoomed_pane: None,
            grid_x: 1,
            grid_y: 0,
            window_id: extra_window_id,
            context_id: ctx_id,
            tree_history: Default::default(),
        });
        app.show_command_palette = true;
        app.sync_command_palette_focus();
    });
    h.run_steps(3);
    h.save_screenshot(&evidence_png!()).expect("render failed");
    assert!(
        h.with_app(|app| app.show_command_palette
            && app.windows[app.active_window]
                .panes
                .contains_key(&first_pane_id)),
        "command palette should remain open over the populated context"
    );
    println!("Screenshot saved to {}", evidence_png!());
}

/// Stint 1458 critic blocker 4: a pointer press on the palette row
/// "Close selected panes" must not reach the pane under the palette (which
/// would clear the selection as a pane click), and closes exactly the
/// selected panes.
#[test]
fn palette_close_selected_row_by_pointer_closes_the_selection() {
    let (mut h, [a, b, c]) = three_column_fixture();
    h.focus_pane(a).unwrap();
    h.run_steps(1);
    h.with_app_mut(|app| app.extend_pane_selection(crate::host::keys::Direction::Right));
    h.run_steps(1);
    let window = h.with_app(|app| app.windows[app.active_window].window_id);
    h.with_app_mut(|app| {
        app.show_command_palette = true;
        app.palette_query = "close selected".into();
        app.palette_selected = 0;
        app.palette_selected_key = None;
        app.sync_command_palette_focus();
    });
    h.run_steps(3);
    let row = h.harness().get_by_label("Close selected panes").rect();
    let under = h.with_app(|app| {
        [a, b, c]
            .into_iter()
            .find(|p| app.visible_pane_rect(*p).is_some_and(|r| r.contains(row.center())))
    });
    assert!(under.is_some(), "the row sits over a pane, so a leaked press would hit it");
    let pos = row.center();
    h.harness().input_mut().events.push(egui::Event::PointerMoved(pos));
    h.step();
    h.harness().input_mut().events.push(egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    h.step();
    assert_eq!(
        h.with_app(|app| app.host.pane_selection(window).map(|s| s.panes.clone())),
        Some(vec![a, b]),
        "the press on the palette row did not reach the pane under it"
    );
    h.harness().input_mut().events.push(egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(3);
    let open: Vec<u64> =
        h.with_app(|app| app.windows[app.active_window].panes.keys().copied().collect());
    assert_eq!(open, vec![c], "exactly the selected panes closed");
}

/// Stint 1439: a pointer click on the palette row "Wrap selection or
/// enter pane" wraps the selection; the press does not reach the pane
/// under the palette and clear it first. Keyboard activation of the same
/// row is pinned by `wrap_tests`.
#[test]
fn palette_wrap_row_by_pointer_wraps_the_selection() {
    let (mut h, [a, b, _c]) = three_column_fixture();
    h.focus_pane(a).unwrap();
    h.run_steps(1);
    h.with_app_mut(|app| app.extend_pane_selection(crate::host::keys::Direction::Right));
    h.run_steps(1);
    let parent = h.with_app(|app| app.router.active().context_id);
    h.with_app_mut(|app| {
        app.show_command_palette = true;
        app.palette_query = "wrap selection".into();
        app.palette_selected = 0;
        app.palette_selected_key = None;
        app.sync_command_palette_focus();
    });
    h.run_steps(3);
    let row = h.harness().get_by_label("Wrap selection or enter pane").rect();
    assert!(
        h.with_app(|app| [a, b, _c]
            .into_iter()
            .any(|p| app.visible_pane_rect(p).is_some_and(|r| r.contains(row.center())))),
        "the row sits over a pane"
    );
    press_at(&mut h, row.center());
    h.run_steps(2);
    let child = h.with_app(|app| app.router.active().context_id);
    assert_ne!(child, parent, "the click wrapped the selection and entered the child");
    let wrapped: Vec<u64> = h.with_app(|app| {
        let mut v: Vec<u64> = app.windows[app.active_window].panes.keys().copied().collect();
        v.sort_unstable();
        v
    });
    assert_eq!(wrapped, vec![a, b]);
}

#[test]
fn config_chooser_offers_not_created_repo_row_for_plain_repo() {
    let mut h = PlexiUiHarness::new_sized(900.0, 700.0);
    let root = h.workspace_root().to_path_buf();
    assert!(
        !root.join(crate::config::workspace_channel_dir()).exists(),
        "plain repo has no channel dir"
    );
    h.with_app_mut(|app| {
        let index = app.router.active_idx();
        app.router.get_mut(index).root = root.clone();
        app.windows[app.active_window].path = root;
    });
    h.harness()
        .key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Comma);
    h.run_steps(3);
    assert!(h
        .harness()
        .query_by_label("Open config: this repo (not created yet)")
        .is_some());
}

#[test]
fn config_chooser_omits_repo_row_outside_workspace() {
    let mut h = PlexiUiHarness::new_sized(900.0, 700.0);
    let root = h.workspace_root().join("does-not-exist");
    h.with_app_mut(|app| {
        let index = app.router.active_idx();
        app.router.get_mut(index).root = root.clone();
        app.windows[app.active_window].path = root;
    });
    h.harness()
        .key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Comma);
    h.run_steps(3);
    assert!(h
        .harness()
        .query_by_label("Open config: everywhere")
        .is_some());
    assert!(h
        .harness()
        .query_by_label("Open config: this repo (not created yet)")
        .is_none());
    assert!(h
        .harness()
        .query_by_label("Open config: this repo")
        .is_none());
}

/// Stint 1621 fixture: an ordinary pane plus an agent pane titled
/// "plexi-pm" in the active context, and an agent pane titled
/// "night-shift" alone in a second, inactive context. Returns
/// `(active_agent_pane, inactive_agent_pane, inactive_context_id)`.
fn seed_palette_agent_panes(h: &mut PlexiUiHarness) -> (u64, u64, u64) {
    add_focused_pane(h);
    let root = h.workspace.path().to_path_buf();
    h.with_app_mut(|app| {
        let agent_pane = |app: &mut crate::app::PlexiApp, name: &str| {
            let pane_id = app.host.alloc_pane_id();
            let pane = Pane::View(Box::new(ViewPane {
                incarnation: Default::default(),
                pip_status: None,
                id: pane_id,
                runtime: ViewRuntime::Builtin(Box::new(
                    crate::file_browser::FileBrowserView::new(root.clone()),
                )),
                workspace_root: root.clone(),
                permissions: PluginPermissions::builtin(),
                manifest_id: "terminal".to_string(),
                name: name.to_string(),
                pane_group: None,
                linked_pane_id: None,
                overlay_replaced: None,
                agent: Some(crate::protocol::PaneAgentState {
                    pane_id,
                    state: crate::protocol::AgentState::Working,
                    agent: "claude-code".to_string(),
                    detail: None,
                    session_id: None,
                }),
                slots: std::collections::HashMap::new(),
                semantic_state: Default::default(),
            }));
            (pane_id, pane)
        };

        let (active_id, active_pane) = agent_pane(app, "plexi-pm");
        let win = &mut app.windows[app.active_window];
        win.panes.insert(active_id, active_pane).unwrap();
        let agent_tile = win.tree.tiles.insert_pane(active_id);
        let first_tile = win.tree.root.expect("fixture pane is the root");
        let root_tile = win
            .tree
            .tiles
            .insert_horizontal_tile(vec![first_tile, agent_tile]);
        win.tree.root = Some(root_tile);
        win.focused_pane = Some(first_tile);

        let ctx_id = app.next_window_id;
        app.next_window_id += 1;
        app.router.push(crate::host::context::Context {
            name: "night-ctx".to_string().into(),
            root: root.clone(),
            description: None,
            context_id: ctx_id,
            parent_id: None,
            depth: 0,
            parked: false,
        });
        let (inactive_id, inactive_pane) = agent_pane(app, "night-shift");
        let mut tiles = egui_tiles::Tiles::default();
        let tile = tiles.insert_pane(inactive_id);
        let window_id = app.next_window_id;
        app.next_window_id += 1;
        app.windows.push(Window {
            name: "night-ctx".to_string(),
            path: root.clone(),
            tree: egui_tiles::Tree::new("night-ctx", tile, tiles),
            panes: (std::collections::HashMap::from([(inactive_id, inactive_pane)])).into(),
            focused_pane: Some(tile),
            zoomed_pane: None,
            grid_x: 1,
            grid_y: 0,
            window_id,
            context_id: ctx_id,
            tree_history: Default::default(),
        });
        app.context_active_window.insert(ctx_id, window_id);
        (active_id, inactive_id, ctx_id)
    })
}

/// Every palette result's selection key for `query`, in display order,
/// read by walking the selection down the list.
fn palette_result_keys(h: &mut PlexiUiHarness, query: &str) -> Vec<String> {
    h.with_app_mut(|app| {
        app.show_command_palette = true;
        app.palette_query = query.to_string();
        app.palette_selected = 0;
        app.palette_selected_key = None;
        app.sync_command_palette_focus();
    });
    h.run_steps(2);
    let mut keys: Vec<String> = Vec::new();
    while let Some(key) = h.with_app(|app| app.palette_selected_key.clone()) {
        if keys.last() == Some(&key) {
            break;
        }
        assert!(keys.len() < 500, "palette walk did not terminate");
        keys.push(key);
        h.harness().key_press(egui::Key::ArrowDown);
        h.step();
    }
    keys
}

fn keys_for_pane(keys: &[String], pane_id: u64) -> Vec<&String> {
    keys.iter()
        .filter(|key| key.rsplit(':').next() == Some(pane_id.to_string().as_str()))
        .collect()
}

/// Stint 1621: with default config (`agents.palette_agent_results` unset,
/// so no agent rows), an agent pane is still an ordinary pane row,
/// findable by its name in both the active-context unpack and the
/// inactive-context search pierce.
#[test]
fn palette_finds_agent_pane_by_name_with_default_config() {
    let mut h = PlexiUiHarness::new_sized(1168.0, 800.0);
    let (active_id, inactive_id, inactive_ctx) = seed_palette_agent_panes(&mut h);
    assert_eq!(
        h.with_app(|app| app
            .config
            .agents
            .as_ref()
            .and_then(|a| a.palette_agent_results)),
        None,
        "the harness runs default agents config"
    );

    let keys = palette_result_keys(&mut h, "");
    assert_eq!(
        keys_for_pane(&keys, active_id),
        vec![&format!(
            "context:{}:{}:{active_id}",
            h.with_app(|app| app.router.active().context_id),
            h.with_app(|app| app.windows[app.active_window].window_id)
        )],
        "active-context unpack lists the agent pane as a pane row: {keys:?}"
    );

    let keys = palette_result_keys(&mut h, "plexi-pm");
    assert_eq!(keys_for_pane(&keys, active_id).len(), 1, "{keys:?}");

    let keys = palette_result_keys(&mut h, "night-shift");
    let hits = keys_for_pane(&keys, inactive_id);
    assert_eq!(hits.len(), 1, "inactive agent pane found by name: {keys:?}");
    assert!(
        hits[0].starts_with(&format!("context:{inactive_ctx}:")),
        "as a pane row in its own context: {hits:?}"
    );
    for query in ["plexi-pm", "night-shift"] {
        palette_result_keys(&mut h, query);
        h.with_app_mut(|app| {
            app.palette_selected = 0;
            app.palette_selected_key = None;
        });
        h.run_steps(3);
        h.save_screenshot(&evidence_png!(format!("_{query}")))
            .expect("render failed");
    }
}

/// Stint 1621: with `agents.palette_agent_results = true`, an agent pane
/// is listed exactly once, as its Agent row, never also as a pane row.
#[test]
fn palette_lists_agent_pane_once_when_agent_rows_enabled() {
    let mut h = PlexiUiHarness::new_sized(1168.0, 800.0);
    h.with_app_mut(|app| {
        app.config
            .agents
            .get_or_insert_with(Default::default)
            .palette_agent_results = Some(true)
    });
    let (active_id, inactive_id, _) = seed_palette_agent_panes(&mut h);

    for (query, pane_id) in [
        ("", active_id),
        ("plexi-pm", active_id),
        ("night-shift", inactive_id),
    ] {
        let keys = palette_result_keys(&mut h, query);
        let hits = keys_for_pane(&keys, pane_id);
        assert_eq!(hits.len(), 1, "query {query:?}: exactly one row: {keys:?}");
        assert!(
            hits[0].starts_with("agent:"),
            "query {query:?}: the agent row: {hits:?}"
        );
    }
    palette_result_keys(&mut h, "plexi-pm");
    h.with_app_mut(|app| {
        app.palette_selected = 0;
        app.palette_selected_key = None;
    });
    h.run_steps(3);
    h.save_screenshot(&evidence_png!()).expect("render failed");
}

/// Visual review surface for stint 0565: the palette rendering an agent
/// fleet spread across two contexts, including a stint-0568-style squad
/// whose three panes share one title. Empty query shows the whole fleet;
/// the typed query proves the squad stays separable by agent name.
#[test]
fn screenshot_command_palette_agent_fleet() {
    let mut h = PlexiUiHarness::new_sized(1168.0, 800.0);
    h.with_app_mut(|app| {
        app.config
            .agents
            .get_or_insert_with(Default::default)
            .palette_agent_results = Some(true)
    });
    add_focused_pane(&mut h);
    let browser_root = h.workspace.path().to_path_buf();

    h.with_app_mut(|app| {
        let ctx_idx = app.router.active_idx();
        app.router.get_mut(ctx_idx).name = "plexi".to_string().into();

        let agent_pane = |app: &mut crate::app::PlexiApp,
                          pane_name: &str,
                          agent: &str,
                          state: crate::protocol::AgentState,
                          detail: Option<&str>| {
            let pane_id = app.host.alloc_pane_id();
            let pane = Pane::View(Box::new(ViewPane {
                incarnation: Default::default(),
                pip_status: None,
                id: pane_id,
                runtime: ViewRuntime::Builtin(Box::new(
                    crate::file_browser::FileBrowserView::new(browser_root.clone()),
                )),
                workspace_root: browser_root.clone(),
                permissions: PluginPermissions::builtin(),
                manifest_id: "terminal".to_string(),
                name: pane_name.to_string(),
                pane_group: None,
                linked_pane_id: None,
                overlay_replaced: None,
                agent: Some(crate::protocol::PaneAgentState {
                    pane_id,
                    state,
                    agent: agent.to_string(),
                    detail: detail.map(str::to_string),
                    session_id: None,
                }),
                slots: std::collections::HashMap::new(),
                semantic_state: Default::default(),
            }));
            (pane_id, pane)
        };

        // One agent alongside the ordinary pane in the active context.
        let (solo_id, solo) = agent_pane(
            app,
            "codex",
            "reviewer",
            crate::protocol::AgentState::Working,
            Some("Read"),
        );
        let win = &mut app.windows[app.active_window];
        win.panes.insert(solo_id, solo).unwrap();
        let solo_tile = win.tree.tiles.insert_pane(solo_id);
        if let Some(first_tile) = win.tree.root {
            let root = win
                .tree
                .tiles
                .insert_horizontal_tile(vec![first_tile, solo_tile]);
            win.tree.root = Some(root);
            win.focused_pane = Some(first_tile);
        }

        // A three-pane squad in its own subcontext, all panes titled
        // "claude" — the stint 0568 shape this palette has to navigate.
        let squad_ctx_id = app.next_window_id;
        app.next_window_id += 1;
        app.router.push(crate::host::context::Context {
            name: "squad-alpha".to_string().into(),
            root: browser_root.clone(),
            description: None,
            context_id: squad_ctx_id,
            parent_id: None,
            depth: 0,
            parked: true,
        });

        let mut squad: Vec<(u64, Pane)> = [
            (
                "claude-code",
                crate::protocol::AgentState::Working,
                Some("Edit"),
            ),
            ("claude-code", crate::protocol::AgentState::Idle, None),
            (
                "claude-code",
                crate::protocol::AgentState::Blocked,
                Some("Bash(just pr-install)"),
            ),
        ]
        .into_iter()
        .map(|(agent, state, detail)| agent_pane(app, "claude", agent, state, detail))
        .collect();
        let ordinary_id = app.host.alloc_pane_id();
        squad.push((
            ordinary_id,
            Pane::View(Box::new(ViewPane {
                incarnation: Default::default(),
                pip_status: None,
                id: ordinary_id,
                runtime: ViewRuntime::Builtin(Box::new(
                    crate::file_browser::FileBrowserView::new(browser_root.clone()),
                )),
                workspace_root: browser_root.clone(),
                permissions: PluginPermissions::builtin(),
                manifest_id: "file-browser".to_string(),
                name: "parked build log".to_string(),
                pane_group: None,
                linked_pane_id: None,
                overlay_replaced: None,
                agent: None,
                slots: std::collections::HashMap::new(),
                semantic_state: Default::default(),
            })),
        ));

        let mut squad_panes = std::collections::HashMap::new();
        let mut squad_tiles = egui_tiles::Tiles::default();
        let mut squad_tile_ids = Vec::new();
        for (pane_id, pane) in squad {
            squad_panes.insert(pane_id, pane);
            squad_tile_ids.push(squad_tiles.insert_pane(pane_id));
        }
        let squad_focused = squad_tile_ids[0];
        let squad_root = squad_tiles.insert_horizontal_tile(squad_tile_ids);
        let squad_window_id = app.next_window_id;
        app.next_window_id += 1;
        app.windows.push(Window {
            name: "squad-alpha".to_string(),
            path: browser_root.clone(),
            tree: egui_tiles::Tree::new("squad-alpha", squad_root, squad_tiles),
            panes: (squad_panes).into(),
            focused_pane: Some(squad_focused),
            zoomed_pane: None,
            grid_x: 1,
            grid_y: 0,
            window_id: squad_window_id,
            context_id: squad_ctx_id,
            tree_history: Default::default(),
        });
        app.context_active_window
            .insert(squad_ctx_id, squad_window_id);

        // A stale parked context is still a discoverable palette record,
        // but intentionally has no row action because there is no live
        // window to restore without desynchronizing router focus.
        app.router.push(crate::host::context::Context {
            name: "archived-without-window".to_string().into(),
            root: browser_root.clone(),
            description: None,
            context_id: app.next_window_id,
            parent_id: None,
            depth: 0,
            parked: true,
        });
        app.next_window_id += 1;

        app.show_command_palette = true;
        app.palette_selected = 0;
        app.sync_command_palette_focus();
    });

    h.run_steps(3);
    h.save_screenshot(&evidence_png!()).expect("render failed");

    h.with_app_mut(|app| {
        app.palette_query = "squad-alpha".to_string();
        app.palette_selected = 0;
        app.palette_selected_key = None;
    });
    h.run_steps(3);
    h.save_screenshot(&evidence_png!("_context"))
        .expect("render failed");

    h.with_app_mut(|app| {
        app.palette_query = "build log".to_string();
        app.palette_selected = 0;
        app.palette_selected_key = None;
    });
    h.run_steps(3);
    h.save_screenshot(&evidence_png!("_pane"))
        .expect("render failed");

    h.with_app_mut(|app| {
        app.palette_query = " blocked ".to_string();
        app.palette_selected = 0;
        app.palette_selected_key = None;
    });
    h.run_steps(3);
    h.save_screenshot(&evidence_png!("_agent"))
        .expect("render failed");

    // PAL-05 visual evidence uses the same seeded, live-agent corpus at
    // both responsive extremes. The narrow pass catches clipped headings,
    // chips, and search rows that a wide modal can hide.
    h.harness().set_size(egui::vec2(520.0, 400.0));
    h.run_steps(3);
    h.save_screenshot(&evidence_png!("_agent_narrow"))
        .expect("render failed");

    assert!(
        h.with_app(|app| app.show_command_palette && app.palette_agent_count_logged == Some(4)),
        "the palette should be open over all four agent panes"
    );
    println!(
        "Screenshots saved to /tmp/plexi_command_palette_agent_fleet*.png (wide + narrow)"
    );
}

#[test]
fn screenshot_catppuccin_latte_command_palette() {
    let mut h = PlexiUiHarness::new_sized(1168.0, 720.0);
    add_focused_pane(&mut h);
    h.with_app_mut(|app| {
        let cfg = crate::ui::theme::preset_colors("catppuccin-latte")
            .expect("catppuccin latte preset must exist");
        app.colors = crate::ui::theme::Colors::from_config(&cfg);
        app.theme = crate::ui::theme::terminal_theme(
            &cfg,
            crate::config::DEFAULT_TERMINAL_MINIMUM_CONTRAST,
        );
        crate::ui::theme::setup_style(&app.ctx, &app.colors);
        app.show_command_palette = true;
        app.sync_command_palette_focus();
    });
    h.run_steps(3);
    let img = h.render().expect("render failed");
    img.save(evidence_png!()).expect("save screenshot");
    // Probe the blank right gutter inside the modal. The image center can
    // land on a command description as the registered plugin list changes.
    let surface = img.get_pixel(img.width() * 3 / 4, img.height() / 2).0;
    let center_luma =
        (u16::from(surface[0]) + u16::from(surface[1]) + u16::from(surface[2])) / 3;
    assert!(
        center_luma > 180,
        "Catppuccin Latte palette surface should be light, got rgba={surface:?}"
    );
    println!("Screenshot saved to {}", evidence_png!());
}

#[test]
fn screenshot_command_palette_user_commands() {
    let mut h = PlexiUiHarness::new_sized(1168.0, 1000.0);
    add_focused_pane(&mut h);
    h.with_app_mut(|app| {
        app.palette_commands = vec![
            crate::cli::ResolvedUserCommand {
                name: "build".to_string(),
                run: Some("cargo build".to_string()),
                steps: Vec::new(),
                on_load: Vec::new(),
                description: Some("Build the project".to_string()),
                scope: crate::cli::UserCommandScope::Workspace,
                secrets: Vec::new(),
                split: false,
            },
            crate::cli::ResolvedUserCommand {
                name: "test".to_string(),
                run: Some("cargo test".to_string()),
                steps: Vec::new(),
                on_load: Vec::new(),
                description: None,
                scope: crate::cli::UserCommandScope::Workspace,
                secrets: Vec::new(),
                split: false,
            },
            crate::cli::ResolvedUserCommand {
                name: "deploy".to_string(),
                run: None,
                steps: Vec::new(),
                on_load: Vec::new(),
                description: None,
                scope: crate::cli::UserCommandScope::Global,
                secrets: Vec::new(),
                split: false,
            },
        ];
        app.show_command_palette = true;
        app.palette_selected = 0;
        app.sync_command_palette_focus();
    });
    h.run_steps(3);
    h.save_screenshot(&evidence_png!()).expect("render failed");
    assert!(
        h.with_app(|app| app.show_command_palette && app.palette_commands.len() == 3),
        "palette should be open with the injected user commands"
    );
    println!("Screenshot saved to {}", evidence_png!());
}

#[test]
fn command_palette_keeps_search_focus_after_scrolled_down_nav() {
    let mut h = PlexiUiHarness::new_sized(1168.0, 720.0);
    add_focused_pane(&mut h);
    h.with_app_mut(|app| {
        app.palette_notes = (0..40)
            .map(|idx| crate::notes::NotePickerEntry {
                path: std::env::temp_dir().join(format!("palette-note-{idx}.md")),
                title: format!("Palette note {idx:02}"),
                preview: "Deterministic row for palette scrolling".to_string(),
                search_text: format!("palette note {idx:02} deterministic row"),
                tier_label: None,
                modified: None,
            })
            .collect();
        app.show_command_palette = true;
        app.palette_selected = 0;
        app.sync_command_palette_focus();
    });

    h.run_steps(2);
    for _ in 0..18 {
        h.harness().key_press(egui::Key::ArrowDown);
        h.step();
    }
    h.run_steps(2);

    let (open, selected, focused) = h.with_app(|app| {
        (
            app.show_command_palette,
            app.palette_selected,
            app.ctx.memory(|m| m.focused()),
        )
    });
    assert!(
        open,
        "command palette should remain open during keyboard nav"
    );
    assert!(
        selected >= 18,
        "ArrowDown should advance selection into the scrolled list"
    );
    assert_eq!(
        focused,
        Some(egui::Id::new("palette_search")),
        "palette search must keep focus after scrolling down past the first page"
    );
}
