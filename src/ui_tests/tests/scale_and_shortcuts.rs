use super::*;

/// At every UI scale, a 1440x900 physical window has a logical surface of
/// `1440/scale x 900/scale`; the card gallery, atlas and the modals must
/// all stay inside it.
#[test]
fn ui_scale_gallery_and_modals_fit_window() {
    for scale in [0.6_f32, 1.0, 2.0] {
        let mut h = PlexiUiHarness::new_sized_scaled(1440.0, 900.0, scale);
        add_focused_pane(&mut h);
        h.run_steps(3);
        let screen = h.with_app(|app| {
            assert!(
                (app.ctx.zoom_factor() - scale).abs() < 1e-3,
                "zoom at {scale}"
            );
            app.ctx.content_rect()
        });
        assert!(
            (screen.width() - 1440.0 / scale).abs() < 1.0
                && (screen.height() - 900.0 / scale).abs() < 1.0,
            "logical screen {screen:?} at scale {scale}"
        );

        // Modals: open one at a time, assert its area, then dismiss.
        seed_note_picker(&mut h);
        h.run_steps(3);
        assert_inside(
            "notes picker",
            scale,
            ui_scale_area_rect(&h, "notes_picker"),
            screen,
        );
        h.harness().key_press(egui::Key::Escape);
        h.run_steps(3);

        h.with_app_mut(|app| app.show_command_palette = true);
        h.run_steps(3);
        assert_inside(
            "command palette",
            scale,
            ui_scale_area_rect(&h, "command_palette"),
            screen,
        );
        h.harness().key_press(egui::Key::Escape);
        h.run_steps(3);
        h.with_app_mut(|app| app.show_command_palette = false);

        h.with_app_mut(|app| app.open_quick_note_modal());
        h.run_steps(3);
        assert_inside(
            "quick note",
            scale,
            ui_scale_area_rect(&h, "quick_note_modal"),
            screen,
        );
        h.harness().key_press(egui::Key::Escape);
        h.run_steps(3);

        h.with_app_mut(|app| app.show_shortcuts = true);
        h.run_steps(3);
        assert_inside(
            "shortcuts",
            scale,
            ui_scale_area_rect(&h, "shortcuts"),
            screen,
        );
        h.with_app_mut(|app| app.show_shortcuts = false);

        // Atlas: every node and the hub pill lie inside the logical screen.
        h.with_app_mut(|app| {
            let root = app.create_context_without_pty("alpha", None);
            app.create_context_without_pty("beta", Some(root));
            app.create_context_without_pty("gamma", Some(root));
            app.create_context_without_pty("delta", None);
            let (nodes, _) = app.compute_atlas_graph(screen);
            assert!(!nodes.is_empty());
            for n in &nodes {
                assert_inside(&format!("atlas node {}", n.name), scale, n.rect, screen);
            }
            for (i, a) in nodes.iter().enumerate() {
                for b in &nodes[i + 1..] {
                    assert!(
                        !a.rect.intersects(b.rect),
                        "atlas nodes {} and {} overlap at scale {scale}",
                        a.name,
                        b.name
                    );
                }
            }
            assert_inside(
                "atlas hub",
                scale,
                crate::app::PlexiApp::atlas_hub_rect(screen),
                screen,
            );
        });
    }
}

/// The narrower case reproduces font-driven grid overflow on macOS too.
/// Keep the original gallery/modal scale matrix unchanged.
#[test]
fn ui_scale_shortcuts_fit_narrow_window() {
    for (width, scale) in [(1440.0, 1.0), (1440.0, 2.0), (1200.0, 2.0)] {
        let mut h = PlexiUiHarness::new_sized_scaled(width, 900.0, scale);
        add_focused_pane(&mut h);
        h.with_app_mut(|app| app.show_shortcuts = true);
        h.run_steps(4);
        let screen = h.with_app(|app| app.ctx.content_rect());
        let rect = ui_scale_area_rect(&h, "shortcuts");
        eprintln!("shortcuts {rect:?}, screen {screen:?} at scale {scale}");
        if std::env::var_os("PLEXI_SHORTCUTS_SCREENSHOTS").is_some() {
            let path = evidence_png!(&format!("_{width}_{scale}"));
            h.save_screenshot(&path).unwrap();
            eprintln!("shortcuts screenshot: {path}");
        }
        assert_inside("shortcuts", scale, rect, screen);
    }
}

/// Stint 1481 (b): in a window too short for it, the shortcuts sheet keeps
/// its body floor (`MODAL_BODY_MIN_H`) instead of collapsing. The floor
/// came with PR #3094; this pins it.
#[test]
fn screenshot_shortcuts_overlay_keeps_its_minimum_height_in_a_tiny_window() {
    let mut h = PlexiUiHarness::new_sized(480.0, 200.0);
    add_focused_pane(&mut h);
    h.with_app_mut(|app| app.show_shortcuts = true);
    h.run_steps(4);
    h.save_screenshot(&evidence_png!()).expect("render failed");
    let ppp = h.with_app(|app| app.ctx.pixels_per_point());
    let area = ui_scale_area_rect(&h, "shortcuts");
    let title = h.harness().get_by_label("Keyboard Shortcuts").rect() / ppp;
    let body = area.max.y - title.max.y;
    assert!(
        body >= crate::ui::style::MODAL_BODY_MIN_H,
        "shortcuts body {body:.1} under its floor {} in a 200 pt window: {area:?}",
        crate::ui::style::MODAL_BODY_MIN_H
    );
}

/// Stint 1555: Cmd+/ lists the Explorer's keys, read from the same table
/// as the Explorer's key handling, Move to Trash included.
#[test]
fn screenshot_shortcuts_sheet_lists_the_explorer_keys() {
    let mut h = PlexiUiHarness::new_sized(1440.0, 1100.0);
    add_focused_pane(&mut h);
    h.with_app_mut(|app| app.show_shortcuts = true);
    h.run_steps(4);
    h.save_screenshot(&evidence_png!()).expect("render failed");
    for (label, _) in crate::file_browser::explorer_shortcut_rows() {
        assert!(
            h.harness().query_all_by_label(label).next().is_some(),
            "the sheet is missing the Explorer row {label:?}"
        );
    }
    assert!(h.harness().query_by_label("EXPLORER").is_some());
}

/// Stint 1067: the shortcuts body must not stay capped at egui's seeded
/// default area height. When the window has room, every row is inside the
/// modal's scroll viewport without scrolling; when it does not, the modal
/// fills the screen budget and scrolling reaches the last rows.
#[test]
fn shortcuts_overlay_every_row_reachable_at_every_scale() {
    // The bottom rows: the last Explorer row of each column (the sheet's
    // last section since stint 1555), and the footer.
    let explorer = crate::file_browser::explorer_shortcut_rows();
    let half = explorer.len().div_ceil(2);
    let last_rows = [
        explorer[half - 1].0,
        explorer[explorer.len() - 1].0,
        "ADHDISNTREAL@GMAIL.COM",
    ];
    let pad = f32::from(crate::ui::style::MODAL_PADDING_V);
    let tol = 0.75;
    for scale in [0.6_f32, 1.0, 1.25, 2.0] {
        let mut h = PlexiUiHarness::new_sized_scaled(1440.0, 900.0, scale);
        add_focused_pane(&mut h);
        h.with_app_mut(|app| app.show_shortcuts = true);
        h.run_steps(4);
        let (screen, ppp) =
            h.with_app(|app| (app.ctx.content_rect(), app.ctx.pixels_per_point()));
        let area = ui_scale_area_rect(&h, "shortcuts");
        assert_inside("shortcuts", scale, area, screen);
        // The body starts below the title, never over it.
        let title = h.harness().get_by_label("Keyboard Shortcuts").rect() / ppp;
        let first_section = h.harness().get_by_label("LAYOUT").rect() / ppp;
        assert!(
            first_section.min.y >= title.max.y,
            "shortcuts body {first_section:?} overlaps the title {title:?} at scale {scale}"
        );
        // Rows paint inside the frame's vertical padding.
        let viewport_bottom = area.max.y - pad;
        let hidden: Vec<&str> = last_rows
            .iter()
            .copied()
            .filter(|label| {
                let rect = h.harness().get_by_label(label).rect() / ppp;
                rect.max.y > viewport_bottom + tol
            })
            .collect();
        if std::env::var_os("PLEXI_SHORTCUTS_SCREENSHOTS").is_some() {
            let path = evidence_png!(&format!("_{scale}"));
            h.save_screenshot(&path).unwrap();
            eprintln!("shortcuts screenshot: {path}");
        }
        if hidden.is_empty() {
            continue;
        }
        // Rows below the fold are only acceptable when the modal already
        // uses the whole height the window allows.
        let budget = screen.height() - 2.0 * crate::ui::style::SPACE_MD;
        assert!(
            area.height() >= budget - 2.0,
            "shortcuts rows {hidden:?} are clipped below {viewport_bottom} while the modal \
                 {area:?} is shorter than the {budget} the screen {screen:?} allows at scale {scale}"
        );
        h.harness().input_mut().events.extend([
            egui::Event::PointerMoved(area.center()),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -5000.0),
                modifiers: egui::Modifiers::NONE,
                phase: egui::TouchPhase::Move,
            },
        ]);
        h.run_steps(30);
        let area = ui_scale_area_rect(&h, "shortcuts");
        for label in last_rows {
            let rect = h.harness().get_by_label(label).rect() / ppp;
            assert!(
                rect.max.y <= area.max.y - pad + tol && rect.min.y >= area.min.y - tol,
                "shortcuts row {label:?} {rect:?} is not reachable by scrolling the modal \
                     {area:?} at scale {scale}"
            );
        }
    }
}

/// Cmd+= steps the one global scale: egui's zoom factor follows, and a
/// terminal pane's effective pixel font size grows with it.
#[test]
fn ui_scale_up_key_scales_zoom_factor_and_terminal_pixels() {
    let mut h = PlexiUiHarness::new_sized(900.0, 600.0);
    h.step();
    let pane_id = h.with_app_mut(|app| {
        app.sidebar_visible = false;
        let workspace_root = app.router.active().root.clone();
        let id = app.host.alloc_pane_id();
        let terminal = crate::host::pane::TerminalPane::fixture(
            id,
            app.ctx.clone(),
            app.pty_event_tx.clone(),
            egui_term::BackendSettings {
                shell: "/bin/sh".into(),
                working_directory: Some(workspace_root),
                ..Default::default()
            },
            14.0,
        )
        .expect("spawn terminal pane");
        let win = &mut app.windows[app.active_window];
        win.panes
            .insert(id, Pane::Terminal(Box::new(terminal)))
            .unwrap();
        let tile = win.tree.tiles.insert_pane(id);
        win.tree.root = Some(tile);
        win.focused_pane = Some(tile);
        id
    });
    h.run_steps(3);
    let pixel_font = |h: &PlexiUiHarness| {
        h.with_app(|app| {
            let pane = app.windows[app.active_window].panes.get(&pane_id).unwrap();
            let Pane::Terminal(t) = pane else {
                panic!("terminal pane")
            };
            t.font_size * app.ctx.zoom_factor()
        })
    };
    let before = pixel_font(&h);
    assert!((h.with_app(|a| a.ctx.zoom_factor()) - 1.0).abs() < 1e-4);

    h.harness().key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        egui::Key::Equals,
    );
    h.run_steps(3);
    h.with_app(|app| {
        assert!(
            (app.ui_scale - 1.1).abs() < 1e-4,
            "ui_scale {}",
            app.ui_scale
        );
        assert!(
            (app.ctx.zoom_factor() - 1.1).abs() < 1e-4,
            "zoom {}",
            app.ctx.zoom_factor()
        );
    });
    assert!(pixel_font(&h) > before, "terminal pixel font must grow");

    h.harness().key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        egui::Key::Minus,
    );
    h.harness().key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        egui::Key::Minus,
    );
    h.run_steps(3);
    h.with_app(|app| assert!((app.ui_scale - 0.9).abs() < 1e-4, "{}", app.ui_scale));
    assert!(pixel_font(&h) < before, "terminal pixel font must shrink");

    // Cmd+Shift+0 resets (and is not swallowed by Quick Note on Cmd+0).
    h.harness().key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        egui::Key::Num0,
    );
    h.run_steps(3);
    h.with_app(|app| {
        assert!((app.ui_scale - 1.0).abs() < 1e-4);
        assert!(
            !app.focus_stack.contains(&crate::app::FocusKind::QuickNote),
            "reset chord must not open Quick Note"
        );
    });
}

/// Cmd+= / Cmd+- adjusts *only* the focused pane's font size while leaving
/// sibling panes and global ui_scale untouched. Cmd+Alt+0 resets font size.
#[test]
fn cmd_plus_minus_changes_only_focused_pane_in_seeded_fixture() {
    let mut h = PlexiUiHarness::new_sized(1000.0, 700.0);
    h.step();
    let (term_a_id, term_b_id, plugin_id, _tile_a, tile_plugin) = h.with_app_mut(|app| {
        app.sidebar_visible = false;
        let workspace_root = app.router.active().root.clone();

        let id_a = app.host.alloc_pane_id();
        let term_a = crate::host::pane::TerminalPane::fixture(
            id_a,
            app.ctx.clone(),
            app.pty_event_tx.clone(),
            egui_term::BackendSettings {
                shell: "/bin/sh".into(),
                working_directory: Some(workspace_root.clone()),
                ..Default::default()
            },
            14.0,
        )
        .expect("spawn terminal pane A");

        let id_b = app.host.alloc_pane_id();
        let term_b = crate::host::pane::TerminalPane::fixture(
            id_b,
            app.ctx.clone(),
            app.pty_event_tx.clone(),
            egui_term::BackendSettings {
                shell: "/bin/sh".into(),
                working_directory: Some(workspace_root.clone()),
                ..Default::default()
            },
            14.0,
        )
        .expect("spawn terminal pane B");

        let id_plugin = app.host.alloc_pane_id();
        let view_pane = ViewPane {
            incarnation: Default::default(),
            pip_status: None,
            id: id_plugin,
            runtime: ViewRuntime::Builtin(Box::new(crate::file_browser::FileBrowserView::new(
                workspace_root.clone(),
            ))),
            workspace_root,
            permissions: PluginPermissions::builtin(),
            manifest_id: "file-browser".to_string(),
            name: "Files".to_string(),
            pane_group: None,
            linked_pane_id: None,
            overlay_replaced: None,
            agent: None,
            slots: std::collections::HashMap::new(),
            semantic_state: Default::default(),
        };

        let win = &mut app.windows[app.active_window];
        win.panes
            .insert(id_a, Pane::Terminal(Box::new(term_a)))
            .unwrap();
        win.panes
            .insert(id_b, Pane::Terminal(Box::new(term_b)))
            .unwrap();
        win.panes
            .insert(id_plugin, Pane::View(Box::new(view_pane)))
            .unwrap();

        let tile_a = win.tree.tiles.insert_pane(id_a);
        let tile_b = win.tree.tiles.insert_pane(id_b);
        let tile_c = win.tree.tiles.insert_pane(id_plugin);

        let tabs = win.tree.tiles.insert_tab_tile(vec![tile_a, tile_b, tile_c]);
        win.tree.root = Some(tabs);
        win.focused_pane = Some(tile_a);

        (id_a, id_b, id_plugin, tile_a, tile_c)
    });
    h.run_steps(3);

    let term_font = |h: &PlexiUiHarness, pid: u64| {
        h.with_app(|app| {
            let pane = app.windows[app.active_window].panes.get(&pid).unwrap();
            let Pane::Terminal(t) = pane else {
                panic!("not terminal")
            };
            t.font_size
        })
    };
    let plugin_font = |h: &PlexiUiHarness, pid: u64| {
        h.with_app(|app| {
            let pane = app.windows[app.active_window].panes.get(&pid).unwrap();
            let Pane::View(a) = pane else {
                panic!("not plugin")
            };
            let ViewRuntime::Builtin(b) = &a.runtime else {
                panic!("not builtin")
            };
            b.font_size().unwrap_or(0.0)
        })
    };

    assert_eq!(term_font(&h, term_a_id), 14.0);
    assert_eq!(term_font(&h, term_b_id), 14.0);
    assert_eq!(plugin_font(&h, plugin_id), crate::ui::style::TEXT_HINT);
    assert_eq!(h.with_app(|a| a.ui_scale), 1.0);

    // 1. Cmd+= on focused Terminal A
    h.harness()
        .key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Equals);
    h.run_steps(3);
    assert_eq!(term_font(&h, term_a_id), 15.0);
    assert_eq!(
        term_font(&h, term_b_id),
        14.0,
        "unfocused terminal untouched"
    );
    assert_eq!(
        plugin_font(&h, plugin_id),
        crate::ui::style::TEXT_HINT,
        "unfocused plugin untouched"
    );
    assert_eq!(h.with_app(|a| a.ui_scale), 1.0, "ui_scale untouched");

    // 2. Cmd+- on focused Terminal A
    h.harness()
        .key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Minus);
    h.run_steps(3);
    assert_eq!(term_font(&h, term_a_id), 14.0);

    // 3. Cmd+= twice, then Cmd+Alt+0 resets
    h.harness()
        .key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Equals);
    h.harness()
        .key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Equals);
    h.run_steps(3);
    assert_eq!(term_font(&h, term_a_id), 16.0);
    h.harness().key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::ALT,
        egui::Key::Num0,
    );
    h.run_steps(3);
    assert_eq!(
        term_font(&h, term_a_id),
        14.0,
        "Cmd+Alt+0 resets terminal font"
    );

    // 4. Focus View pane, Cmd+= steps font, Cmd+Alt+0 resets
    h.with_app_mut(|app| {
        app.windows[app.active_window].focused_pane = Some(tile_plugin);
    });
    h.run_steps(3);
    h.harness()
        .key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Equals);
    h.run_steps(3);
    assert_eq!(
        plugin_font(&h, plugin_id),
        crate::ui::style::TEXT_HINT + 1.0,
        "focused plugin font stepped"
    );
    assert_eq!(term_font(&h, term_a_id), 14.0);
    assert_eq!(term_font(&h, term_b_id), 14.0);
    assert_eq!(h.with_app(|a| a.ui_scale), 1.0);

    h.harness().key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::ALT,
        egui::Key::Num0,
    );
    h.run_steps(3);
    assert_eq!(
        plugin_font(&h, plugin_id),
        crate::ui::style::TEXT_HINT,
        "Cmd+Alt+0 resets plugin font"
    );
}

/// Cmd+= / Cmd+- in Tree Atlas adjusts only the remembered
/// active pane for the highlighted context, keeps the overlay open, and
/// returning to Columns shows the pane with its updated font size.
#[test]
fn cmd_plus_minus_in_tree_atlas() {
    let _experiments =
        crate::release::set_test_experiments(&[crate::release::ReleaseFeature::TreeAtlas]);
    let mut h = PlexiUiHarness::new_sized(1000.0, 700.0);
    h.step();
    let (ctx_a, pane_a, ctx_b, pane_b) = h.with_app_mut(|app| {
        app.sidebar_visible = false;
        let root = app.router.active().root.clone();
        let ctx_a = app.router.active().context_id;
        let id_a = app.host.alloc_pane_id();
        let term_a = crate::host::pane::TerminalPane::fixture(
            id_a,
            app.ctx.clone(),
            app.pty_event_tx.clone(),
            egui_term::BackendSettings {
                shell: "/bin/sh".into(),
                working_directory: Some(root.clone()),
                ..Default::default()
            },
            14.0,
        )
        .expect("spawn terminal pane A");
        let win_a = &mut app.windows[app.active_window];
        win_a
            .panes
            .insert(id_a, Pane::Terminal(Box::new(term_a)))
            .unwrap();
        let tile_a = win_a.tree.tiles.insert_pane(id_a);
        win_a.tree.root = Some(tile_a);
        win_a.focused_pane = Some(tile_a);
        app.zoom.context_last_pane.insert(ctx_a, id_a);

        // Create context B
        let ctx_b = app.create_context_without_pty("context-b", None);
        let win_b_idx = app.windows.len() - 1;
        let id_b = app.host.alloc_pane_id();
        let term_b = crate::host::pane::TerminalPane::fixture(
            id_b,
            app.ctx.clone(),
            app.pty_event_tx.clone(),
            egui_term::BackendSettings {
                shell: "/bin/sh".into(),
                working_directory: Some(root),
                ..Default::default()
            },
            14.0,
        )
        .expect("spawn terminal pane B");
        let win_b = &mut app.windows[win_b_idx];
        win_b
            .panes
            .insert(id_b, Pane::Terminal(Box::new(term_b)))
            .unwrap();
        let tile_b = win_b.tree.tiles.insert_pane(id_b);
        win_b.tree.root = Some(tile_b);
        win_b.focused_pane = Some(tile_b);
        app.zoom.context_last_pane.insert(ctx_b, id_b);

        (ctx_a, id_a, ctx_b, id_b)
    });
    h.run_steps(3);

    // 1. Enter TreeAtlas, highlight context B
    h.with_app_mut(|app| {
        app.set_zoom_level_with_transition(crate::app::zoom::ZoomLevel::TreeAtlas);
        app.zoom.atlas_focused_context = Some(ctx_b);
    });
    h.run_steps(3);
    assert_eq!(
        h.with_app(|a| a.zoom.level),
        crate::app::zoom::ZoomLevel::TreeAtlas
    );

    // Cmd+= in TreeAtlas
    h.harness()
        .key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Equals);
    h.run_steps(3);

    // View remains TreeAtlas!
    assert_eq!(
        h.with_app(|a| a.zoom.level),
        crate::app::zoom::ZoomLevel::TreeAtlas
    );
    assert_eq!(h.with_app(|a| a.ui_scale), 1.0);

    // Pane B font size changed to 15.0, Pane A remained 14.0!
    h.with_app(|app| {
        let (win_b_idx, _) = app.find_pane_in_any_window(pane_b).unwrap();
        let p_b = app.windows[win_b_idx].panes.get(&pane_b).unwrap();
        let Pane::Terminal(t_b) = p_b else { panic!() };
        assert_eq!(
            t_b.font_size, 15.0,
            "remembered pane for context B adjusted in Cards"
        );

        let (win_a_idx, _) = app.find_pane_in_any_window(pane_a).unwrap();
        let p_a = app.windows[win_a_idx].panes.get(&pane_a).unwrap();
        let Pane::Terminal(t_a) = p_a else { panic!() };
        assert_eq!(t_a.font_size, 14.0, "context A pane untouched");
    });

    // Return to Columns
    h.with_app_mut(|app| {
        app.zoom_in_level();
    });
    h.run_steps(3);
    h.with_app(|app| {
        assert_eq!(app.zoom.level, crate::app::zoom::ZoomLevel::Columns);
        let p_b = app.windows[app.active_window].panes.get(&pane_b).unwrap();
        let Pane::Terminal(t_b) = p_b else { panic!() };
        assert_eq!(
            t_b.font_size, 15.0,
            "restored pane reflects new font size in Columns"
        );
    });

    // 2. Enter TreeAtlas, highlight context A
    h.with_app_mut(|app| {
        app.set_zoom_level_with_transition(crate::app::zoom::ZoomLevel::TreeAtlas);
        app.zoom.atlas_focused_context = Some(ctx_a);
    });
    h.run_steps(3);
    assert_eq!(
        h.with_app(|a| a.zoom.level),
        crate::app::zoom::ZoomLevel::TreeAtlas
    );

    // Cmd+= in TreeAtlas
    h.harness()
        .key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Equals);
    h.run_steps(3);

    // View remains TreeAtlas!
    assert_eq!(
        h.with_app(|a| a.zoom.level),
        crate::app::zoom::ZoomLevel::TreeAtlas
    );
    assert_eq!(h.with_app(|a| a.ui_scale), 1.0);

    // Pane A font size changed to 15.0!
    h.with_app(|app| {
        let (win_a_idx, _) = app.find_pane_in_any_window(pane_a).unwrap();
        let p_a = app.windows[win_a_idx].panes.get(&pane_a).unwrap();
        let Pane::Terminal(t_a) = p_a else { panic!() };
        assert_eq!(
            t_a.font_size, 15.0,
            "remembered pane for context A adjusted in Atlas"
        );
    });
}

/// Cmd+Shift+/- updates global ui_scale by 0.1 at every zoom level, respects
/// bounds, persists to disk, and Cmd+Shift+0 resets without opening Quick Note.
#[test]
fn cmd_shift_plus_minus_updates_global_ui_scale_from_columns_and_atlas() {
    let _experiments =
        crate::release::set_test_experiments(&[crate::release::ReleaseFeature::TreeAtlas]);
    let mut h = PlexiUiHarness::new_sized(1000.0, 700.0);
    h.step();
    add_focused_pane(&mut h);
    h.run_steps(3);
    assert!((h.with_app(|a| a.ui_scale) - 1.0).abs() < 1e-4);

    // 1. In Columns: Cmd+Shift+= steps from 1.0 to 1.1
    h.harness().key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        egui::Key::Equals,
    );
    h.run_steps(3);
    assert!((h.with_app(|a| a.ui_scale) - 1.1).abs() < 1e-4);

    // 2. In TreeAtlas: Cmd+Shift+= steps from 1.1 to 1.2
    h.with_app_mut(|app| {
        app.set_zoom_level_with_transition(crate::app::zoom::ZoomLevel::TreeAtlas);
    });
    h.run_steps(3);
    assert_eq!(
        h.with_app(|a| a.zoom.level),
        crate::app::zoom::ZoomLevel::TreeAtlas
    );
    h.harness().key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        egui::Key::Equals,
    );
    h.run_steps(3);
    assert_eq!(
        h.with_app(|a| a.zoom.level),
        crate::app::zoom::ZoomLevel::TreeAtlas
    );
    assert!((h.with_app(|a| a.ui_scale) - 1.2).abs() < 1e-4);

    // 3. In TreeAtlas: Cmd+Shift+- steps from 1.2 to 1.1
    h.with_app_mut(|app| {
        app.set_zoom_level_with_transition(crate::app::zoom::ZoomLevel::TreeAtlas);
    });
    h.run_steps(3);
    assert_eq!(
        h.with_app(|a| a.zoom.level),
        crate::app::zoom::ZoomLevel::TreeAtlas
    );
    h.harness().key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        egui::Key::Minus,
    );
    h.run_steps(3);
    assert_eq!(
        h.with_app(|a| a.zoom.level),
        crate::app::zoom::ZoomLevel::TreeAtlas
    );
    assert!((h.with_app(|a| a.ui_scale) - 1.1).abs() < 1e-4);

    // 4. Cmd+Shift+0 resets to 1.0 and does not open QuickNote
    h.harness().key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        egui::Key::Num0,
    );
    h.run_steps(3);
    h.with_app(|app| {
        assert!((app.ui_scale - 1.0).abs() < 1e-4);
        assert!(!app.focus_stack.contains(&crate::app::FocusKind::QuickNote));
    });

    // 5. Test bounds clamping (UI_SCALE_MAX = 2.0, UI_SCALE_MIN = 0.6)
    for _ in 0..15 {
        h.harness().key_press_modifiers(
            egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
            egui::Key::Equals,
        );
    }
    h.run_steps(3);
    assert!(
        (h.with_app(|a| a.ui_scale) - 2.0).abs() < 1e-4,
        "capped at UI_SCALE_MAX 2.0"
    );

    for _ in 0..25 {
        h.harness().key_press_modifiers(
            egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
            egui::Key::Minus,
        );
    }
    h.run_steps(3);
    assert!(
        (h.with_app(|a| a.ui_scale) - 0.6).abs() < 1e-4,
        "floored at UI_SCALE_MIN 0.6"
    );

    // 6. Test persistence to disk
    h.with_app_mut(|app| {
        app.set_ui_scale(1.2);
        app.flush_ui_scale_persist_now();
    });
    let config_path = crate::config::config_path();
    assert!(
        config_path.exists(),
        "ui scale persistence must create config.toml"
    );
    let content = std::fs::read_to_string(&config_path).unwrap();
    assert!(
        content.contains("ui_scale = 1.2"),
        "persisted scale in config: {content}"
    );
    let reloaded = crate::config::PlexiConfig::load();
    let (fresh_host, _) = PlexiApp::new_for_test_with_config(
        egui::Context::default(),
        Arc::new(AtomicU64::new(0)),
        reloaded,
    );
    assert!(
        (fresh_host.ui_scale - 1.2).abs() < 1e-4,
        "fresh host must restore the persisted global scale"
    );
}

/// Harness key routing covers physical Equals/Plus and Minus chords, deduplication,
/// and custom keybinding overrides.
#[test]
fn harness_key_tests_cover_chords_and_custom_bindings() {
    let mut h = PlexiUiHarness::new();
    h.step();
    add_focused_pane(&mut h);
    h.run_steps(3);

    let initial_font = h.with_app(|app| {
        let (win, _) = app
            .find_pane_in_any_window(app.active_focused_pane_id().unwrap())
            .unwrap();
        let pane = app.windows[win]
            .panes
            .get(&app.active_focused_pane_id().unwrap())
            .unwrap();
        let Pane::View(a) = pane else { panic!() };
        let ViewRuntime::Builtin(b) = &a.runtime else {
            panic!()
        };
        b.font_size().unwrap()
    });

    // 1. Key::Equals with Command
    h.harness()
        .key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Equals);
    h.run_steps(3);
    let font_after_equals = h.with_app(|app| {
        let (win, _) = app
            .find_pane_in_any_window(app.active_focused_pane_id().unwrap())
            .unwrap();
        let pane = app.windows[win]
            .panes
            .get(&app.active_focused_pane_id().unwrap())
            .unwrap();
        let Pane::View(a) = pane else { panic!() };
        let ViewRuntime::Builtin(b) = &a.runtime else {
            panic!()
        };
        b.font_size().unwrap()
    });
    assert_eq!(font_after_equals, initial_font + 1.0);

    // 2. Key::Plus with Command
    h.harness()
        .key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Plus);
    h.run_steps(3);
    let font_after_plus = h.with_app(|app| {
        let (win, _) = app
            .find_pane_in_any_window(app.active_focused_pane_id().unwrap())
            .unwrap();
        let pane = app.windows[win]
            .panes
            .get(&app.active_focused_pane_id().unwrap())
            .unwrap();
        let Pane::View(a) = pane else { panic!() };
        let ViewRuntime::Builtin(b) = &a.runtime else {
            panic!()
        };
        b.font_size().unwrap()
    });
    assert_eq!(font_after_plus, initial_font + 2.0);

    // 3. Deduplication in single frame
    h.with_app_mut(|app| {
        let ctx = egui::Context::default();
        let mut raw = egui::RawInput::default();
        raw.events.push(egui::Event::Key {
            key: egui::Key::Equals,
            physical_key: Some(egui::Key::Equals),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        });
        raw.events.push(egui::Event::Key {
            key: egui::Key::Plus,
            physical_key: Some(egui::Key::Plus),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        });
        let mut actions = Vec::new();
        let mut claimed_key_releases = std::collections::HashSet::new();
        let _ = ctx.run_ui(raw, |ui| {
            let mut input = crate::app::input_router::PlexiInput::take_from(ui.ctx());
            actions = crate::host::keys::poll_actions(
                &mut input,
                &app.binding_table,
                &mut claimed_key_releases,
                /* view_active */ true,
                /* keyboard_capture */ false,
                /* overlay_open */ false,
                /* shortcuts_overlay_open */ false,
            );
        });
        let count = actions
            .iter()
            .filter(|a| **a == crate::host::keys::Action::IncreasePaneFontSize)
            .count();
        assert_eq!(
            count, 1,
            "Equals and Plus companions must deduplicate in single frame"
        );
    });

    // 4. Custom configured binding override
    h.with_app_mut(|app| {
        let mut cfg = crate::config::KeybindingsConfig::default();
        cfg.increase_font_size = Some("cmd+alt+=".into());
        app.key_bindings = crate::host::keys::build_key_bindings(Some(&cfg));
        app.binding_table = crate::host::keys::build_binding_table(&app.key_bindings);
    });

    // Old binding Cmd+= no longer triggers
    h.harness()
        .key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Equals);
    h.run_steps(3);
    let font_unchanged = h.with_app(|app| {
        let (win, _) = app
            .find_pane_in_any_window(app.active_focused_pane_id().unwrap())
            .unwrap();
        let pane = app.windows[win]
            .panes
            .get(&app.active_focused_pane_id().unwrap())
            .unwrap();
        let Pane::View(a) = pane else { panic!() };
        let ViewRuntime::Builtin(b) = &a.runtime else {
            panic!()
        };
        b.font_size().unwrap()
    });
    assert_eq!(
        font_unchanged, font_after_plus,
        "Cmd+= overridden so should not step"
    );

    // New binding Cmd+Alt+= triggers
    h.harness().key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::ALT,
        egui::Key::Equals,
    );
    h.run_steps(3);
    let font_stepped = h.with_app(|app| {
        let (win, _) = app
            .find_pane_in_any_window(app.active_focused_pane_id().unwrap())
            .unwrap();
        let pane = app.windows[win]
            .panes
            .get(&app.active_focused_pane_id().unwrap())
            .unwrap();
        let Pane::View(a) = pane else { panic!() };
        let ViewRuntime::Builtin(b) = &a.runtime else {
            panic!()
        };
        b.font_size().unwrap()
    });
    assert_eq!(
        font_stepped,
        font_after_plus + 1.0,
        "Cmd+Alt+= custom override steps font"
    );
}
