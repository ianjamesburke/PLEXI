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
        app.open_command_palette("");
    });
    h.run_steps(3);
    h.save_screenshot(&evidence_png!()).expect("render failed");
    assert!(
        h.with_app(|app| app.command_palette_open()
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
        app.open_command_palette("close selected");
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
        app.open_command_palette("wrap selection");
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
        app.open_command_palette(query);
    });
    h.run_steps(2);
    let mut keys: Vec<String> = Vec::new();
    while let Some(key) = h.with_app(|app| app.palette_selected_key()) {
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
        .filter(|key| key.rsplit('/').next() == Some(pane_id.to_string().as_str()))
        .collect()
}

/// Agent panes are findable by name in the active and inactive contexts.
#[test]
fn palette_finds_agent_pane_by_name_with_default_config() {
    let mut h = PlexiUiHarness::new_sized(1168.0, 800.0);
    let (active_id, inactive_id, inactive_ctx) = seed_palette_agent_panes(&mut h);

    let keys = palette_result_keys(&mut h, "");
    assert_eq!(
        keys_for_pane(&keys, active_id),
        vec![&format!(
            "commands:focus/{}/{}/{active_id}",
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
        hits[0].starts_with(&format!("commands:focus/{inactive_ctx}/")),
        "as a pane row in its own context: {hits:?}"
    );
    for query in ["plexi-pm", "night-shift"] {
        palette_result_keys(&mut h, query);
        h.with_app_mut(|app| {
            let rows = app
                .host
                .picker(crate::host::picker::PickerId::Palette)
                .map_or(0, |picker| picker.rows().len());
            for _ in 0..rows {
                app.apply_palette_command(crate::host::picker::PickerCommand::Move(
                    crate::host::picker::Step::Up,
                ));
            }
        });
        h.run_steps(3);
        h.save_screenshot(&evidence_png!(format!("_{query}")))
            .expect("render failed");
    }
}

/// Commands still finds pane destinations through its own source.
#[test]
fn commands_stop_never_lists_files_or_agents() {
    let mut h = PlexiUiHarness::new_sized(1168.0, 800.0);
    let (active_id, inactive_id, _) = seed_palette_agent_panes(&mut h);

    for (query, pane_id) in [
        ("", active_id),
        ("plexi-pm", active_id),
        ("night-shift", inactive_id),
    ] {
        let keys = palette_result_keys(&mut h, query);
        assert!(
            keys.iter().all(|key| key.starts_with("commands:")),
            "query {query:?}: only Commands-source rows: {keys:?}"
        );
        let hits = keys_for_pane(&keys, pane_id);
        assert_eq!(hits.len(), 1, "query {query:?}: exactly one row: {keys:?}");
        assert!(
            hits[0].starts_with("commands:focus/"),
            "query {query:?}: the pane row: {hits:?}"
        );
    }
    h.with_app(|app| {
        let picker = app
            .host
            .picker(crate::host::picker::PickerId::Palette)
            .expect("palette picker");
        assert_eq!(picker.active_stop().id, "commands");
    });
}

/// Visual review surface for stint 0565: the palette rendering an agent
/// fleet spread across two contexts, including a stint-0568-style squad
/// whose three panes share one title. Empty query shows the whole fleet;
/// the typed query proves the squad stays separable by agent name.
#[test]
fn screenshot_command_palette_agent_fleet() {
    let mut h = PlexiUiHarness::new_sized(1168.0, 800.0);
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

        app.open_command_palette("");
    });

    h.run_steps(3);
    h.save_screenshot(&evidence_png!()).expect("render failed");

    h.with_app_mut(|app| {
        app.set_palette_query("squad-alpha");
    });
    h.run_steps(3);
    h.save_screenshot(&evidence_png!("_context"))
        .expect("render failed");

    h.with_app_mut(|app| {
        app.set_palette_query("build log");
    });
    h.run_steps(3);
    h.save_screenshot(&evidence_png!("_pane"))
        .expect("render failed");

    h.with_app_mut(|app| {
        app.set_palette_query(" blocked ");
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
        h.with_app(|app| app.command_palette_open()),
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
        app.open_command_palette("");
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
        app.open_command_palette("");
    });
    h.run_steps(3);
    h.save_screenshot(&evidence_png!()).expect("render failed");
    assert!(
        h.with_app(|app| app.command_palette_open() && app.palette_commands.len() == 3),
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
        app.open_command_palette("");
    });

    h.run_steps(2);
    for _ in 0..18 {
        h.harness().key_press(egui::Key::ArrowDown);
        h.step();
    }
    h.run_steps(2);

    let (open, selected, focused) = h.with_app(|app| {
        (
            app.command_palette_open(),
            app.palette_selected_index().unwrap_or(0),
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

// ── Stint 1391: the picker through the production input path ──

fn palette_picker(h: &PlexiUiHarness) -> (String, &'static str, Option<String>) {
    h.with_app(|app| {
        let picker = app
            .host
            .picker(crate::host::picker::PickerId::Palette)
            .expect("palette picker");
        (
            picker.query().to_owned(),
            picker.active_stop().id,
            app.palette_selected_key(),
        )
    })
}

fn type_palette_text(h: &mut PlexiUiHarness, text: &str) {
    for ch in text.chars() {
        let key = egui::Key::from_name(&ch.to_string()).expect("letter key");
        h.harness().key_press(key);
        h.harness()
            .input_mut()
            .events
            .push(egui::Event::Text(ch.to_string()));
        h.step();
    }
}

/// Stint 1391: Cmd+P then typing a, b, c reads "abc" in the field and in
/// the picker's one query; no key the picker claims writes into it.
#[test]
fn palette_typing_abc_reads_abc() {
    let mut h = PlexiUiHarness::new_sized(1168.0, 800.0);
    add_focused_pane(&mut h);
    h.step();
    h.harness()
        .key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::P);
    h.run_steps(2);
    assert!(h.with_app(|app| app.command_palette_open()));
    type_palette_text(&mut h, "abc");
    h.run_steps(2);
    assert_eq!(palette_picker(&h).0, "abc");
    assert!(
        h.harness().query_all_by_value("abc").next().is_some(),
        "the field shows the picker's query"
    );
    for (modifiers, key) in [
        (egui::Modifiers::NONE, egui::Key::ArrowDown),
        (egui::Modifiers::NONE, egui::Key::ArrowUp),
        (egui::Modifiers::NONE, egui::Key::Tab),
        (egui::Modifiers::SHIFT, egui::Key::Tab),
        (egui::Modifiers::COMMAND, egui::Key::J),
        (egui::Modifiers::COMMAND, egui::Key::K),
    ] {
        h.harness().key_press_modifiers(modifiers, key);
        h.step();
        assert_eq!(palette_picker(&h).0, "abc", "{modifiers:?}+{key:?}");
        assert_eq!(
            h.with_app(|app| app.ctx.memory(|m| m.focused())),
            Some(egui::Id::new("palette_search")),
            "{modifiers:?}+{key:?} keeps focus in the palette field"
        );
        assert!(
            h.harness().query_by_label("Empty directory").is_some(),
            "{modifiers:?}+{key:?} never reaches the explorer behind the palette"
        );
    }
    h.save_screenshot(&evidence_png!()).expect("render failed");
}

const KEYBOARD_ROW: &str = "commands:cmd/Keyboard shortcuts";

fn press(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

fn enter() -> egui::Event {
    press(egui::Key::Enter, egui::Modifiers::NONE)
}

fn down() -> egui::Event {
    press(egui::Key::ArrowDown, egui::Modifiers::NONE)
}

fn up() -> egui::Event {
    press(egui::Key::ArrowUp, egui::Modifiers::NONE)
}

fn pointer_button(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

/// One delivery: every event reaches the host in one egui pass, in order.
fn deliver(h: &mut PlexiUiHarness, events: Vec<egui::Event>) {
    h.harness().input_mut().events.extend(events);
    h.step();
}

fn palette_activations() -> Vec<String> {
    crate::overlays::command_palette::palette_test_log::take_activations()
}

fn palette_row_keys(h: &PlexiUiHarness) -> Vec<String> {
    h.with_app(|app| {
        app.host
            .picker(crate::host::picker::PickerId::Palette)
            .map(|picker| {
                picker
                    .rows()
                    .iter()
                    .map(|row| row.key.as_str().to_owned())
                    .collect()
            })
            .unwrap_or_default()
    })
}

fn palette_row_title(h: &PlexiUiHarness, index: usize) -> String {
    h.with_app(|app| {
        app.host
            .picker(crate::host::picker::PickerId::Palette)
            .expect("palette picker")
            .rows()[index]
            .title
            .clone()
    })
}

/// Close whatever the last activation opened, then open the palette with
/// Cmd+P and let its first answer land. Returns the presented row keys.
fn reopen_palette(h: &mut PlexiUiHarness) -> Vec<String> {
    if h.with_app(|app| app.command_palette_open()) {
        h.harness().key_press(egui::Key::Escape);
        h.run_steps(2);
    }
    if h.with_app(|app| app.show_shortcuts) {
        h.harness().key_press(egui::Key::Escape);
        h.run_steps(2);
    }
    h.harness()
        .key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::P);
    h.run_steps(3);
    assert!(h.with_app(|app| app.command_palette_open()));
    palette_activations();
    let keys = palette_row_keys(h);
    assert!(
        keys.len() >= 3,
        "the palette lists rows to choose: {keys:?}"
    );
    assert_ne!(keys[0], KEYBOARD_ROW);
    assert_ne!(keys[1], KEYBOARD_ROW);
    keys
}

fn palette_harness() -> PlexiUiHarness {
    let mut h = PlexiUiHarness::new_sized(1168.0, 800.0);
    add_focused_pane(&mut h);
    h.step();
    h
}

/// Stint 1391 review round 1: Enter acts at its position in the input
/// stream. Text before it is the query it accepts; text after it belongs
/// to the closed session and never reaches the pane behind the palette.
#[test]
fn palette_enter_acts_at_its_position_in_the_input_stream() {
    let mut h = palette_harness();
    let ime_commit = |text: &str| {
        vec![
            egui::Event::Ime(egui::ImeEvent::Enabled),
            egui::Event::Ime(egui::ImeEvent::Preedit(text.to_owned())),
            egui::Event::Ime(egui::ImeEvent::Commit(text.to_owned())),
            egui::Event::Ime(egui::ImeEvent::Disabled),
        ]
    };
    for (label, edit) in [
        ("text", vec![egui::Event::Text("keyboard".into())]),
        ("paste", vec![egui::Event::Paste("keyboard".into())]),
        ("ime", ime_commit("keyboard")),
    ] {
        reopen_palette(&mut h);
        let mut events = edit.clone();
        events.push(enter());
        deliver(&mut h, events);
        h.run_steps(3);
        assert_eq!(
            palette_activations(),
            vec![KEYBOARD_ROW.to_owned()],
            "{label} then Enter runs the edited query's best match"
        );

        let keys = reopen_palette(&mut h);
        let mut events = vec![enter()];
        events.extend(edit);
        deliver(&mut h, events);
        h.run_steps(3);
        assert_eq!(
            palette_activations(),
            vec![keys[0].clone()],
            "Enter then {label} runs the row selected before the edit"
        );
        assert!(
            h.harness().query_by_label("Empty directory").is_some(),
            "{label}: nothing reached the explorer behind the palette"
        );
    }
}

/// Stint 1391 review round 2: keys translate in arrival order, so Enter
/// before an arrow accepts the row it saw, and clamped arrows count one
/// by one.
#[test]
fn palette_keys_act_in_arrival_order() {
    let mut h = palette_harness();
    let cmd = egui::Modifiers::COMMAND;
    let cases: Vec<(&str, Vec<egui::Event>, usize)> = vec![
        ("Enter, Down", vec![enter(), down()], 0),
        ("Down, Enter", vec![down(), enter()], 1),
        (
            "Down, Down, Up, Enter",
            vec![down(), down(), up(), enter()],
            1,
        ),
        ("Up, Down, Enter", vec![up(), down(), enter()], 1),
        (
            "Cmd+J, Cmd+J, Cmd+K, Enter",
            vec![
                press(egui::Key::J, cmd),
                press(egui::Key::J, cmd),
                press(egui::Key::K, cmd),
                enter(),
            ],
            1,
        ),
    ];
    for (label, events, expected) in cases {
        let keys = reopen_palette(&mut h);
        deliver(&mut h, events);
        h.run_steps(3);
        assert_eq!(
            palette_activations(),
            vec![keys[expected].clone()],
            "{label}"
        );
    }
}

/// Stint 1391 review round 2: a pointer over rows painted for an older
/// query never chooses what a pending Enter runs, before or after the
/// Enter. A stale click alone runs nothing; a hover on current rows does.
#[test]
fn palette_stale_pointer_never_redirects_enter() {
    let mut h = palette_harness();
    let away = egui::pos2(4.0, 4.0);
    let row_center = |h: &mut PlexiUiHarness| {
        let title = palette_row_title(h, 1);
        h.harness().get_by_label(&title).rect().center()
    };

    reopen_palette(&mut h);
    let b = row_center(&mut h);
    deliver(&mut h, vec![egui::Event::PointerMoved(away)]);
    deliver(
        &mut h,
        vec![
            egui::Event::Paste("keyboard".into()),
            egui::Event::PointerMoved(b),
            enter(),
        ],
    );
    h.run_steps(3);
    assert_eq!(
        palette_activations(),
        vec![KEYBOARD_ROW.to_owned()],
        "move before Enter"
    );

    reopen_palette(&mut h);
    let b = row_center(&mut h);
    deliver(&mut h, vec![egui::Event::PointerMoved(away)]);
    deliver(
        &mut h,
        vec![
            egui::Event::Paste("keyboard".into()),
            enter(),
            egui::Event::PointerMoved(b),
        ],
    );
    h.run_steps(3);
    assert_eq!(
        palette_activations(),
        vec![KEYBOARD_ROW.to_owned()],
        "move after Enter"
    );

    reopen_palette(&mut h);
    let b = row_center(&mut h);
    deliver(&mut h, vec![egui::Event::PointerMoved(away)]);
    deliver(
        &mut h,
        vec![
            egui::Event::Paste("keyboard".into()),
            egui::Event::PointerMoved(b),
            pointer_button(b, true),
            pointer_button(b, false),
        ],
    );
    h.run_steps(3);
    assert!(
        palette_activations().is_empty(),
        "a stale click runs nothing"
    );

    let keys = reopen_palette(&mut h);
    let b = row_center(&mut h);
    deliver(&mut h, vec![egui::Event::PointerMoved(away)]);
    deliver(&mut h, vec![egui::Event::PointerMoved(b)]);
    deliver(&mut h, vec![enter()]);
    h.run_steps(3);
    assert_eq!(
        palette_activations(),
        vec![keys[1].clone()],
        "current hover, then Enter"
    );
}

/// Stint 1391: an Enter that waits for the edited query's rows freezes
/// the arrows before it and ignores the ones after; it runs once.
#[test]
fn palette_pending_enter_keeps_its_ordered_moves() {
    let mut h = palette_harness();
    reopen_palette(&mut h);
    deliver(&mut h, vec![egui::Event::Paste("pane".into())]);
    h.run_steps(2);
    let expected = palette_row_keys(&h);
    assert!(expected.len() >= 3, "query 'pane' lists rows: {expected:?}");

    reopen_palette(&mut h);
    deliver(
        &mut h,
        vec![egui::Event::Paste("pane".into()), down(), enter(), down()],
    );
    h.run_steps(4);
    assert_eq!(palette_activations(), vec![expected[1].clone()]);
}

/// Two file-browser panes side by side, the left one focused.
fn palette_over_two_panes() -> (PlexiUiHarness, PaneId, PaneId) {
    let mut h = PlexiUiHarness::new_sized(1168.0, 800.0);
    let left = add_focused_pane(&mut h);
    let right = add_focused_pane(&mut h);
    h.with_app_mut(|app| {
        let w = app.active_window;
        let tree = &mut app.windows[w].tree;
        let tiles = [left, right].map(|p| tree.tiles.find_pane(&p).expect("pane tile"));
        tree.root = Some(tree.tiles.insert_horizontal_tile(tiles.to_vec()));
        app.windows[w].focused_pane = Some(tiles[0]);
    });
    h.run_steps(3);
    (h, left, right)
}

fn focused_pane_id(h: &PlexiUiHarness) -> Option<PaneId> {
    h.with_app(|app| {
        let win = &app.windows[app.active_window];
        match win.tree.tiles.get(win.focused_pane?)? {
            egui_tiles::Tile::Pane(id) => Some(*id),
            egui_tiles::Tile::Container(_) => None,
        }
    })
}

fn pane_center(h: &PlexiUiHarness, pane: PaneId) -> egui::Pos2 {
    h.with_app(|app| {
        let tiles = &app.windows[app.active_window].tree.tiles;
        tiles
            .rect(tiles.find_pane(&pane).expect("pane tile"))
            .expect("pane laid out")
            .center()
    })
}

fn click_events(pos: egui::Pos2) -> Vec<egui::Event> {
    vec![
        egui::Event::PointerMoved(pos),
        pointer_button(pos, true),
        pointer_button(pos, false),
    ]
}

fn focus_left(h: &mut PlexiUiHarness, left: PaneId) {
    h.with_app_mut(|app| {
        let w = app.active_window;
        let tile = app.windows[w].tree.tiles.find_pane(&left);
        app.windows[w].focused_pane = tile;
    });
    h.run_steps(2);
    assert_eq!(focused_pane_id(h), Some(left));
}

/// Stint 1391, dismissal barrier (ledger row 152): a click in the same
/// delivery as the Escape that closes the palette, or in the delivery
/// whose logic completes a pending Enter, never reaches the pane the
/// palette covered. A fresh click afterwards focuses it as usual.
#[test]
fn palette_dismissal_never_clicks_through() {
    let (mut h, left, right) = palette_over_two_panes();
    let target = pane_center(&h, right);
    deliver(&mut h, click_events(target));
    h.run_steps(2);
    assert_eq!(
        focused_pane_id(&h),
        Some(right),
        "control: the click focuses"
    );

    focus_left(&mut h, left);
    reopen_palette(&mut h);
    h.save_screenshot(&evidence_png!()).expect("render failed");
    let mut events = vec![press(egui::Key::Escape, egui::Modifiers::NONE)];
    events.extend(click_events(target));
    deliver(&mut h, events);
    h.run_steps(2);
    assert!(!h.with_app(|app| app.command_palette_open()));
    assert_eq!(focused_pane_id(&h), Some(left), "Escape, click");

    // Opened and dismissed in one delivery: the palette never drew, so
    // no palette layer stands between the click and the pane.
    let mut events = vec![
        press(egui::Key::P, egui::Modifiers::COMMAND),
        press(egui::Key::Escape, egui::Modifiers::NONE),
    ];
    events.extend(click_events(target));
    deliver(&mut h, events);
    h.run_steps(2);
    assert!(!h.with_app(|app| app.command_palette_open()));
    assert_eq!(focused_pane_id(&h), Some(left), "Cmd+P, Escape, click");

    reopen_palette(&mut h);
    deliver(
        &mut h,
        vec![egui::Event::Paste("reset text size".into()), enter()],
    );
    assert!(h.with_app(|app| app.command_palette_open()), "Enter waits");
    deliver(&mut h, click_events(target));
    h.run_steps(2);
    assert_eq!(palette_activations().len(), 1);
    assert_eq!(focused_pane_id(&h), Some(left), "logic-time close, click");

    deliver(&mut h, click_events(target));
    h.run_steps(2);
    assert_eq!(focused_pane_id(&h), Some(right), "a fresh click focuses");
}

/// Stint 1391 review round 4: Cmd+P pressed and released in one delivery
/// closes the palette and leaves no host claim on P's release, so the
/// next P typed into a pane keeps its key-up.
#[test]
fn palette_closing_chord_retires_its_release_claim() {
    let mut h = palette_harness();
    reopen_palette(&mut h);
    let cmd = egui::Modifiers::COMMAND;
    deliver(
        &mut h,
        vec![
            press(egui::Key::P, cmd),
            egui::Event::Key {
                key: egui::Key::P,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: cmd,
            },
        ],
    );
    assert!(!h.with_app(|app| app.command_palette_open()));
    assert!(
        !h.with_app(|app| app.claimed_host_key_releases.contains(&egui::Key::P)),
        "P's release was delivered with the chord"
    );
}

/// Stint 1391 review round 4: Focus Approval lifts an actionable approval
/// above the open palette. An Enter in the same delivery is not the
/// palette's: no row runs and the request stays pending.
#[test]
fn palette_enter_after_focus_approval_runs_no_row() {
    let mut h = palette_harness();
    let monitor =
        crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir()).unwrap();
    let id = monitor
        .file_needs_you(crate::broker::gate::NeedsYouFile {
            kind: crate::broker::gate::NeedsYouKind::Question,
            actor: crate::broker::principal::RecordOwner::Host("codex".into()),
            resource: "pane:1".into(),
            summary: "Continue?".into(),
            expires_at: None,
            run_tag: None,
        })
        .unwrap();
    reopen_palette(&mut h);
    let mut focus_approval = egui::Modifiers::COMMAND;
    focus_approval.alt = true;
    deliver(&mut h, vec![press(egui::Key::P, focus_approval), enter()]);
    h.run_steps(3);
    assert_eq!(
        h.with_app(|app| app.focus_stack.top().as_ref().copied()),
        Some(FocusKind::ApprovalBanner),
        "the approval took focus"
    );
    assert!(palette_activations().is_empty(), "no palette row ran");
    assert!(
        monitor
            .host_records()
            .question(&id)
            .is_some_and(|record| record.resolution.is_none()),
        "the request is still pending"
    );
}

/// Stint 1391: Cmd+P opens on the Commands stop with today's rows and
/// the first row selected; Tab and Shift+Tab keep the query. Enter runs the selected row.
#[test]
fn palette_commands_stop() {
    let mut h = PlexiUiHarness::new_sized(1168.0, 800.0);
    root_context_in_workspace(&mut h);
    add_focused_pane(&mut h);
    h.step();
    h.harness()
        .key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::P);
    h.run_steps(3);
    let (query, stop, selected) = palette_picker(&h);
    assert_eq!((query.as_str(), stop), ("", "commands"));
    assert!(
        selected
            .as_deref()
            .is_some_and(|k| k.starts_with("commands:")),
        "a fresh palette selects its first row: {selected:?}"
    );
    h.harness().get_by_label("CONTEXTS");
    h.harness().get_by_label("COMMANDS");
    h.save_screenshot(&evidence_png!("_empty"))
        .expect("render failed");

    type_palette_text(&mut h, "keyboard");
    h.run_steps(2);
    let (_, _, selected) = palette_picker(&h);
    assert_eq!(
        selected.as_deref(),
        Some("commands:cmd/Keyboard shortcuts"),
        "typed query selects its best match"
    );
    h.save_screenshot(&evidence_png!("_query"))
        .expect("render failed");
    for (modifiers, expected) in [
        (egui::Modifiers::NONE, "files"),
        (egui::Modifiers::SHIFT, "commands"),
    ] {
        h.harness().key_press_modifiers(modifiers, egui::Key::Tab);
        h.step();
        let (query, stop, _) = palette_picker(&h);
        assert_eq!((query.as_str(), stop), ("keyboard", expected));
    }
    h.run_steps(2);
    h.harness().key_press(egui::Key::Enter);
    h.run_steps(2);
    assert!(!h.with_app(|app| app.command_palette_open()));
    assert!(
        h.with_app(|app| app.show_shortcuts),
        "Enter ran the selected Keyboard shortcuts row"
    );
}

// ── Stint 1392: the Files stop ──

/// Root the active context in the harness's own workspace, so the Files
/// stop walks a tempdir rather than the system temp dir.
fn root_context_in_workspace(h: &mut PlexiUiHarness) -> std::path::PathBuf {
    let root = h.workspace_root().to_path_buf();
    h.with_app_mut(|app| {
        let idx = app.router.active_idx();
        app.router.get_mut(idx).root = root.clone();
    });
    root
}

/// Step until `done` holds; the Files source and previews answer from
/// worker threads, so this waits on the wall clock.
fn wait_for_palette(h: &mut PlexiUiHarness, what: &str, done: impl Fn(&PlexiApp) -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        h.step();
        if h.with_app(&done) {
            h.run_steps(2);
            return;
        }
        assert!(std::time::Instant::now() < deadline, "timed out: {what}");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

fn palette_preview(app: &PlexiApp) -> Option<crate::host::picker::PreviewStatus> {
    app.host
        .picker(crate::host::picker::PickerId::Palette)
        .and_then(|picker| picker.preview().cloned())
}

fn open_files_stop(h: &mut PlexiUiHarness) {
    h.step();
    h.harness()
        .key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::P);
    h.run_steps(2);
    h.harness().key_press(egui::Key::Tab);
    h.step();
    assert_eq!(palette_picker(h).1, "files");
}

fn seed_files_workspace(root: &std::path::Path) {
    std::fs::create_dir_all(root.join("docs")).expect("mkdir docs");
    std::fs::create_dir_all(root.join("assets")).expect("mkdir assets");
    std::fs::create_dir_all(root.join("src")).expect("mkdir src");
    std::fs::write(
        root.join("docs/notes.md"),
        "# Release notes\n\nPalette files preview body line.\n\n- Cmd+P, Tab, type, Enter\n- Text and images preview\n",
    )
    .expect("write notes");
    std::fs::write(
        root.join("src/main.rs"),
        "fn main() {\n    println!(\"hi\");\n}\n",
    )
    .expect("write main");
    let photo = image::RgbaImage::from_fn(160, 96, |x, y| {
        image::Rgba([(x * 255 / 160) as u8, (y * 255 / 96) as u8, 180, 255])
    });
    photo
        .save(root.join("assets/photo.png"))
        .expect("write photo");
}

/// Stint 1392: Tab from Commands lists the active context's files with an
/// always-on preview: text, an image, the empty state, and no preview half
/// below the compact modal width.
#[test]
fn palette_files_stop_with_preview() {
    use crate::host::picker::{PreviewBody, PreviewStatus};
    let mut h = PlexiUiHarness::new_sized(1168.0, 800.0);
    let root = root_context_in_workspace(&mut h);
    seed_files_workspace(&root);
    open_files_stop(&mut h);

    type_palette_text(&mut h, "notes");
    wait_for_palette(&mut h, "text preview", |app| {
        matches!(
            palette_preview(app),
            Some(PreviewStatus::Ready(PreviewBody::Text { .. }))
        )
    });
    let selected = palette_picker(&h).2.expect("a selected file");
    assert!(selected.ends_with("docs/notes.md"), "{selected}");
    h.harness().get_by_label("notes.md");
    assert!(
        h.harness()
            .query_all_by_label_contains("Palette files preview body line")
            .next()
            .is_some(),
        "the text preview shows the file's body"
    );
    h.save_screenshot(&evidence_png!("_text"))
        .expect("render failed");

    h.with_app_mut(|app| app.set_palette_query("photo"));
    wait_for_palette(&mut h, "image preview", |app| {
        matches!(
            palette_preview(app),
            Some(PreviewStatus::Ready(PreviewBody::Image {
                width: 160,
                height: 96
            }))
        )
    });
    h.save_screenshot(&evidence_png!("_image"))
        .expect("render failed");

    h.with_app_mut(|app| app.set_palette_query("zzzqqq"));
    wait_for_palette(&mut h, "empty answer", |app| {
        app.host
            .picker(crate::host::picker::PickerId::Palette)
            .is_some_and(|picker| !picker.is_loading() && picker.rows().is_empty())
    });
    h.harness().get_by_label("No matching files");
    h.save_screenshot(&evidence_png!("_empty"))
        .expect("render failed");

    // Narrow: below the compact modal width the preview half is dropped.
    let mut narrow = PlexiUiHarness::new_sized(600.0, 800.0);
    let root = root_context_in_workspace(&mut narrow);
    seed_files_workspace(&root);
    open_files_stop(&mut narrow);
    type_palette_text(&mut narrow, "notes");
    wait_for_palette(&mut narrow, "narrow rows", |app| {
        matches!(
            palette_preview(app),
            Some(PreviewStatus::Ready(PreviewBody::Text { .. }))
        )
    });
    narrow.harness().get_by_label("notes.md");
    assert!(
        narrow
            .harness()
            .query_all_by_label_contains("Palette files preview body line")
            .next()
            .is_none(),
        "no preview half below the compact modal width"
    );
    narrow
        .save_screenshot(&evidence_png!("_narrow"))
        .expect("render failed");
}

#[test]
fn palette_agents_stop_with_preview() {
    let mut h = PlexiUiHarness::new_sized(1168.0, 800.0);
    add_focused_pane(&mut h);
    h.with_app_mut(|app| {
        let id = app.host.alloc_pane_id();
        let mut term = TerminalPane::fixture(
            id,
            app.ctx.clone(),
            app.pty_event_tx.clone(),
            egui_term::BackendSettings {
                shell: "/bin/sh".into(),
                ..Default::default()
            },
            14.0,
        )
        .unwrap();
        term.agent_role = Some("release-reviewer".into());
        term.agent_harness = Some("codex".into());
        let win = &mut app.windows[app.active_window];
        win.panes
            .insert(id, Pane::Terminal(Box::new(term)))
            .unwrap();
        let agent_tile = win.tree.tiles.insert_pane(id);
        let root = win.tree.root.expect("fixture pane is laid out");
        win.tree.root = Some(
            win.tree
                .tiles
                .insert_horizontal_tile(vec![root, agent_tile]),
        );
        let monitor =
            crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir())
                .unwrap();
        monitor
            .file_needs_you(crate::broker::gate::NeedsYouFile {
                kind: crate::broker::gate::NeedsYouKind::Question,
                actor: crate::broker::principal::RecordOwner::Host("palette-fixture".into()),
                resource: format!("pane:{id}"),
                summary: "Ship the release today?".into(),
                expires_at: None,
                run_tag: None,
            })
            .unwrap();
    });
    reopen_palette(&mut h);
    for _ in 0..2 {
        h.harness().key_press(egui::Key::Tab);
        h.run_steps(2);
    }
    assert_eq!(palette_picker(&h).1, "agents");
    wait_for_palette(&mut h, "Agents snapshot", |app| {
        app.palette_agents_load_count() == 1
    });
    assert!(h
        .harness()
        .query_all_by_label("release-reviewer")
        .next()
        .is_some());
    h.harness()
        .get_by_label("Waiting for you: Ship the release today?");

    deliver(
        &mut h,
        vec![
            egui::Event::Paste("reviewer".into()),
            press(egui::Key::Enter, egui::Modifiers::COMMAND),
        ],
    );
    h.run_steps(2);
    h.harness().get_by_label("Close pane");
    h.harness()
        .get_by_label("Waiting for you: Ship the release today?");
    type_palette_text(&mut h, "close");
    h.run_steps(2);
    assert!(h.harness().query_by_label("Focus").is_none());
    h.harness().key_press(egui::Key::Escape);
    h.run_steps(2);
    assert_eq!(palette_picker(&h).0, "reviewer");
    h.harness()
        .get_by_label("Waiting for you: Ship the release today?");
    assert!(h
        .harness()
        .query_all_by_label("release-reviewer")
        .next()
        .is_some());
    h.save_screenshot(&evidence_png!()).expect("render failed");
}
