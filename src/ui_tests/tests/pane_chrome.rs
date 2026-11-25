use super::*;

#[test]
fn screenshot_pane_corners_themes_scale_and_override() {
    for theme in ["catppuccin-mocha", "catppuccin-latte"] {
        for (width, scale) in [(1440.0, 1.0), (720.0, 1.5)] {
            let mut h = PlexiUiHarness::new_sized_scaled(width, 900.0, scale);
            std::fs::write(
                h.workspace.path().join("review-notes.md"),
                "Pane corner fixture\n",
            )
            .unwrap();
            let root = h.workspace.path().to_path_buf();
            let config_path = crate::config::config_path();
            std::fs::write(
                &config_path,
                format!("pane_gap = 0.0\nui_scale = {scale}\n[theme]\npreset = \"{theme}\"\n"),
            )
            .unwrap();
            h.with_app_mut(|app| {
                app.sidebar_visible = false;
                app.reload_config();
            });
            h.open_file_browser(root);
            h.run_steps(4);
            let (rect, accent) = h.with_app(|app| {
                let pane = app.visual_owner_pane().unwrap();
                (app.visible_pane_rect(pane).unwrap(), app.colors.accent)
            });
            for radius in [None, Some(18)] {
                let radius_line = radius
                    .map(|r| format!("pane_corner_radius = {r}\n"))
                    .unwrap_or_default();
                std::fs::write(&config_path, format!("pane_gap = 0.0\nui_scale = {scale}\n{radius_line}[theme]\npreset = \"{theme}\"\n")).unwrap();
                h.with_app_mut(|app| app.reload_config());
                h.run_steps(3);
                let expected = crate::ui::style::pane_radius(radius, rect);
                assert!(h.harness().output().shapes.iter().any(|shape| {
                    matches!(&shape.shape, egui::Shape::Rect(r) if r.rect == rect && r.corner_radius == expected
                        && r.stroke.color == accent && r.stroke.width == crate::ui::style::PANE_FOCUS_WIDTH
                        && r.stroke_kind == egui::StrokeKind::Inside)
                }), "focus radius and width must follow the pane config even at zero gap");
                let suffix = format!("_{theme}_{width}_{scale}_{}", radius.unwrap_or(0));
                h.save_screenshot(&evidence_png!(&suffix)).unwrap();
            }
        }
    }
}

fn wait_for_lineage_fixture(h: &mut PlexiUiHarness, panes: &[u64]) {
    let deadline = Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(20));
    loop {
        h.run_steps(2);
        let ready = h.with_app_mut(|app| {
            panes.iter().all(|id| {
                let (win, _) = app.find_pane_in_any_window(*id).unwrap();
                app.windows[win]
                    .panes
                    .get_mut(id)
                    .unwrap()
                    .as_terminal_mut()
                    .unwrap()
                    .backend
                    .capture_lines(30)
                    .iter()
                    .any(|line| line.contains("Lead:") || line.contains("Researcher:"))
            })
        });
        if ready {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "lineage terminal fixture timed out"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn screenshot_pane_corners_lineage_themes() {
    for (theme, width, scale) in [
        ("catppuccin-mocha", 1440.0, 1.0),
        ("catppuccin-latte", 720.0, 1.5),
    ] {
        for conjoined in [true, false] {
            let mut h = PlexiUiHarness::new_sized_scaled(width, 900.0, scale);
            h.step();
            std::fs::write(
                crate::config::config_path(),
                format!("pane_gap = 0.0\nui_scale = {scale}\n[theme]\npreset = \"{theme}\"\n"),
            )
            .unwrap();
            h.with_app_mut(|app| {
                app.reload_config();
            });
            let (parent, child, portal) = h.with_app_mut(|app| {
                let (parent, _) = app
                    .spawn_agent_worker_pane(
                        "lead",
                        "pi",
                        "printf 'Lead: review evidence\\n'; sleep 120",
                        "Coordinate",
                        None,
                        false,
                        None,
                        None,
                        None,
                        None,
                        false,
                        None,
                    )
                    .unwrap();
                let (child, context) = app
                    .spawn_agent_worker_pane(
                        "researcher",
                        "pi",
                        "printf 'Researcher: checking evidence\\n'; sleep 120",
                        "Review",
                        Some("research"),
                        true,
                        Some(parent),
                        None,
                        None,
                        None,
                        conjoined,
                        None,
                    )
                    .unwrap();
                let portal = app
                    .windows
                    .iter()
                    .flat_map(|w| w.panes.iter())
                    .find_map(|(&id, p)| (p.portal_target() == Some(context)).then_some(id));
                if conjoined {
                    assert!(app.dive_into_agent_children(parent));
                }
                (parent, child, portal)
            });
            wait_for_lineage_fixture(&mut h, &[parent, child]);
            h.with_app(|app| {
                let expected = crate::ui::theme::Colors::from_config(
                    &crate::ui::theme::preset_colors(theme).unwrap(),
                );
                assert_eq!(
                    app.colors.bg_darkest, expected.bg_darkest,
                    "fixture theme must survive startup"
                );
            });
            if conjoined {
                h.focus_pane(child).unwrap();
                h.run_steps(3);
                h.save_screenshot(&evidence_png!(&format!("_{theme}_child")))
                    .unwrap();
                h.with_app_mut(|app| app.conjoined_parent_focus = Some(parent));
                h.run_steps(3);
                let (rect, accent) =
                    h.with_app(|app| (app.conjoined_parent_rect.unwrap().1, app.colors.accent));
                assert!(h.harness().output().shapes.iter().any(|shape| {
                    matches!(&shape.shape, egui::Shape::Rect(r) if r.rect == rect
                        && r.corner_radius == crate::ui::style::pane_radius(None, rect)
                        && r.stroke.color == accent && r.stroke.width == crate::ui::style::PANE_FOCUS_WIDTH
                        && r.stroke_kind == egui::StrokeKind::Inside)
                }), "conjoined parent must use the shared radius and independent focus width");
                h.save_screenshot(&evidence_png!(&format!("_{theme}_conjoined")))
                    .unwrap();
            } else {
                h.focus_pane(portal.expect("portal fixture")).unwrap();
                h.run_steps(3);
                h.save_screenshot(&evidence_png!(&format!("_{theme}_portal")))
                    .unwrap();
                h.focus_pane(parent).unwrap();
                h.run_steps(3);
                h.save_screenshot(&evidence_png!(&format!("_{theme}_terminal")))
                    .unwrap();
            }
        }
    }
}

// Agent lineage and pinned-parent input ownership cannot be seeded by scenes.
#[test]
fn screenshot_child_lineage_and_focus_outline() {
    let mut h = PlexiUiHarness::new_sized(1440.0, 900.0);
    h.step();
    let (parent, child) = h.with_app_mut(|app| {
        let (parent, _) = app
            .spawn_agent_worker_pane(
                "lead",
                "pi",
                "printf 'Lead: review research\\n'; sleep 120",
                "Coordinate review",
                None,
                false,
                None,
                None,
                None,
                None,
                false,
                None,
            )
            .unwrap();
        let (child, _) = app
            .spawn_agent_worker_pane(
                "researcher",
                "pi",
                "printf 'Researcher: checking source evidence\\n'; sleep 120",
                "Review evidence",
                Some("lineage-preview"),
                true,
                Some(parent),
                None,
                None,
                None,
                true,
                None,
            )
            .unwrap();
        assert!(app.dive_into_agent_children(parent));
        (parent, child)
    });
    wait_for_lineage_fixture(&mut h, &[parent, child]);
    h.focus_pane(child).unwrap();
    h.run_steps(3);
    let (rect, accent, gap) = h.with_app(|app| {
        assert_eq!(app.visual_owner_pane(), Some(child));
        assert_ne!(app.visual_owner_pane(), Some(parent));
        (
            app.visible_pane_rect(child).unwrap(),
            app.colors.accent,
            app.config.pane_gap.unwrap_or(4.0),
        )
    });
    assert!(
        h.harness().output().shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::Shape::Rect(r) if r.rect == rect
            && r.stroke.color == accent && r.stroke.width > 0.0 && r.stroke.width < gap
            && r.stroke_kind == egui::StrokeKind::Inside)
        }),
        "focused child must have the ordinary accent outline"
    );
    let mut texts = Vec::new();
    for shape in &h.harness().output().shapes {
        gallery_text_shapes(&shape.shape, &mut texts);
    }
    assert!(
        texts
            .iter()
            .any(|(_, text)| text.contains("researcher") && text.contains("child of lead")),
        "{texts:?}"
    );
    assert!(!texts.iter().any(|(_, text)| text.starts_with("↳ lead")));
    h.save_screenshot(&evidence_png!()).unwrap();
}

#[test]
fn screenshot_portal_lineage_and_focus_outline() {
    let mut h = PlexiUiHarness::new_sized(1440.0, 900.0);
    h.step();
    let (parent, child, portal) = h.with_app_mut(|app| {
        let (parent, _) = app
            .spawn_agent_worker_pane(
                "lead",
                "pi",
                "printf 'Lead: coordinate research\\n'; sleep 120",
                "Review",
                None,
                false,
                None,
                None,
                None,
                None,
                false,
                None,
            )
            .unwrap();
        let (child, context) = app
            .spawn_agent_worker_pane(
                "researcher",
                "pi",
                "printf 'Researcher: working\\n'; sleep 120",
                "Research",
                Some("research"),
                true,
                Some(parent),
                None,
                None,
                None,
                false,
                None,
            )
            .unwrap();
        let portal = app
            .windows
            .iter()
            .flat_map(|w| w.panes.iter())
            .find_map(|(&id, p)| (p.portal_target() == Some(context)).then_some(id))
            .unwrap();
        (parent, child, portal)
    });
    wait_for_lineage_fixture(&mut h, &[parent, child]);
    h.focus_pane(portal).unwrap();
    h.run_steps(3);
    let (rect, accent, gap) = h.with_app(|app| {
        assert!(!app.pane_lineage().contains_key(&parent));
        (
            app.visible_pane_rect(portal).unwrap(),
            app.colors.accent,
            app.config.pane_gap.unwrap_or(4.0),
        )
    });
    assert!(h.harness().output().shapes.iter().any(|shape| {
        matches!(&shape.shape, egui::Shape::Rect(r) if r.rect == rect
            && r.stroke.color == accent && r.stroke.width > 0.0 && r.stroke.width < gap
            && r.stroke_kind == egui::StrokeKind::Inside)
    }));
    // Stint 1166: the Portal header title is the context name; its
    // lineage moved to the tile's hover tooltip.
    h.with_app(|app| {
        let header =
            crate::render::terminal_pane::last_rendered_tab_bar_layout(&app.ctx, portal)
                .expect("portal paints the shared pane header");
        assert_eq!(header.elided_label.as_deref(), Some("research"));
        assert_eq!(
            app.pane_lineage().get(&portal).map(String::as_str),
            Some("lead")
        );
    });
    h.save_screenshot(&evidence_png!()).unwrap();
    h.with_app_mut(|app| {
        let (win, _) = app.find_pane_in_any_window(parent).unwrap();
        app.windows[win]
            .panes
            .get_mut(&parent)
            .unwrap()
            .as_terminal_mut()
            .unwrap()
            .name = None;
        assert_eq!(app.pane_lineage().get(&portal), Some(&parent.to_string()));
    });
}

const LONG_PORTAL_NAMES: [&str; 2] = [
    "Investigate Plexi approval routing across nested agent contexts | PLEXI",
    "Refactor workspace persistence so subcontext previews survive restarts",
];

/// Portal tiles whose child contexts carry 60+ character names and long
/// path-like descriptions. Each child context holds one window so the
/// minimap paints. The portals become the active window's only tiles,
/// side by side, unless `tabbed_with` names a pane to share a tab group
/// with the first portal (stints 1166, 1338).
fn seed_long_name_portals(h: &mut PlexiUiHarness, tabbed_with: Option<PaneId>) -> [PaneId; 2] {
    let browser_root = h.workspace_root().to_path_buf();
    h.with_app_mut(|app| {
        app.sidebar_visible = false;
        let active_ctx_id = app.router.active().context_id;
        let mut portals = [0; 2];
        let mut tiles = Vec::new();
        for (i, name) in LONG_PORTAL_NAMES.iter().enumerate() {
            let child_ctx_id = active_ctx_id + 6_000 + i as u64;
            app.router.push(crate::host::context::Context {
                name: name.to_string().into(),
                root: browser_root.clone(),
                description: Some(format!(
                    "~/Documents/GitHub/PLEXI/worktrees/feature/very-long-branch-name-{i}/src/spatial/tiling.rs"
                )),
                context_id: child_ctx_id,
                parent_id: Some(active_ctx_id),
                depth: 1,
                parked: false,
            });
            let child_pane_id = app.host.alloc_pane_id();
            let child = ViewPane {
                incarnation: Default::default(),
                pip_status: None,
                id: child_pane_id,
                runtime: ViewRuntime::Builtin(Box::new(
                    crate::file_browser::FileBrowserView::new(browser_root.clone()),
                )),
                workspace_root: browser_root.clone(),
                permissions: PluginPermissions::builtin(),
                manifest_id: "file-browser".to_string(),
                name: "Files for the approval routing investigation".to_string(),
                pane_group: None,
                linked_pane_id: None,
                overlay_replaced: None,
                agent: None,
                slots: std::collections::HashMap::new(),
                semantic_state: Default::default(),
            };
            let mut child_panes = std::collections::HashMap::new();
            child_panes.insert(child_pane_id, Pane::View(Box::new(child)));
            let mut child_tiles = egui_tiles::Tiles::default();
            let child_tile = child_tiles.insert_pane(child_pane_id);
            let window_id = app.next_window_id;
            app.next_window_id += 1;
            app.windows.push(Window {
                name: name.to_string(),
                path: browser_root.clone(),
                tree: egui_tiles::Tree::new(
                    egui::Id::new(("long_portal_child", i)),
                    child_tile,
                    child_tiles,
                ),
                panes: (child_panes).into(),
                focused_pane: Some(child_tile),
                zoomed_pane: None,
                grid_x: 0,
                grid_y: 0,
                window_id,
                context_id: child_ctx_id,
                tree_history: Default::default(),
            });
            let portal = app.host.alloc_pane_id();
            let win = &mut app.windows[app.active_window];
            win.panes.insert(
                portal,
                Pane::Portal(Box::new(PortalPane {
                    pane_id: portal,
                    target_context_id: child_ctx_id,
                    context_state: None,
                })),
            ).unwrap();
            tiles.push(win.tree.tiles.insert_pane(portal));
            portals[i] = portal;
        }
        let win = &mut app.windows[app.active_window];
        let first = match tabbed_with {
            Some(sibling) => {
                let sibling_tile = win
                    .tree
                    .tiles
                    .find_pane(&sibling)
                    .expect("tab sibling must already be in the tree");
                win.tree.tiles.insert_tab_tile(vec![tiles[0], sibling_tile])
            }
            None => tiles[0],
        };
        let root = win.tree.tiles.insert_horizontal_tile(vec![first, tiles[1]]);
        win.tree.root = Some(root);
        win.focused_pane = Some(tiles[0]);
        portals
    })
}

fn portal_tile_report(
    reports: &[crate::ui::layout_report::LayoutReport],
    tile: egui::Rect,
) -> crate::ui::layout_report::LayoutReport {
    let matches: Vec<_> = reports
        .iter()
        .filter(|r| r.surface == "portal_tile" && tile.expand(1.0).contains(r.parent.center()))
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "expected one portal_tile layout report for tile {tile:?}, got {:?}",
        reports
            .iter()
            .map(|r| (&r.surface, r.parent))
            .collect::<Vec<_>>()
    );
    matches[0].clone()
}

/// Stint 1338: every text a Portal tile paints stays inside its own tile,
/// neighbouring tiles never collide, long names elide with U+2026, and a
/// tile too short for its body hides rows instead of overflowing.
#[test]
fn portal_tiles_keep_long_names_inside_small_tiles() {
    // 520x110 leaves the tiles taller than the pane header but shorter
    // than the header plus `PORTAL_MINIMAP_MIN_H`: the header alone shows.
    // (At 520x90 the strip gives each tile less height than the header.)
    for (width, height) in [(360.0_f32, 200.0_f32), (520.0, 260.0), (520.0, 110.0)] {
        let mut h = PlexiUiHarness::new_sized(width, height);
        h.step();
        let portals = seed_long_name_portals(&mut h, None);
        h.run_steps(3);
        let ppp = h.with_app(|app| app.ctx.pixels_per_point());
        let tiles: Vec<egui::Rect> = h.with_app(|app| {
            portals
                .iter()
                .map(|p| {
                    app.visible_pane_rect(*p).unwrap_or_else(|| {
                        panic!("portal {p} not laid out at {width}x{height}")
                    })
                })
                .collect()
        });
        // Any widget labelled with a context name sits inside its tile.
        for (name, tile) in LONG_PORTAL_NAMES.iter().zip(&tiles) {
            let prefix: String = name.chars().take(24).collect();
            for node in h.harness().query_all_by_label_contains(&prefix) {
                let rect = node.rect() / ppp;
                assert!(
                    tile.expand(0.75).contains_rect(rect),
                    "{width}x{height}: label {prefix:?} {rect:?} leaves its tile {tile:?}"
                );
            }
        }
        let reports = h.with_app(|app| crate::ui::layout_report::snapshot(&app.ctx));
        let tile_reports: Vec<_> = tiles
            .iter()
            .map(|t| portal_tile_report(&reports, *t))
            .collect();
        for (report, tile) in tile_reports.iter().zip(&tiles) {
            for text in &report.texts {
                assert!(
                    tile.expand(0.75).contains_rect(text.paint),
                    "{width}x{height}: portal text {:?} {:?} leaves its tile {tile:?}",
                    text.text,
                    text.paint
                );
            }
            assert!(
                report
                    .texts
                    .iter()
                    .any(|t| t.text.ends_with(crate::ui::text::ELLIPSIS)),
                "{width}x{height}: the long portal title must elide with U+2026: {:?}",
                report.texts.iter().map(|t| &t.text).collect::<Vec<_>>()
            );
        }
        for a in &tile_reports[0].texts {
            for b in &tile_reports[1].texts {
                assert!(
                    !a.paint.intersects(b.paint),
                    "{width}x{height}: {:?} {:?} collides with neighbouring tile text {:?} {:?}",
                    a.text,
                    a.paint,
                    b.text,
                    b.paint
                );
            }
        }
        h.save_screenshot(&evidence_png!(&format!("_{width}x{height}")))
            .unwrap();
    }
}

/// Stint 1166: a Portal tile paints the standard pane header (same
/// height, background and centred elided title as a named terminal
/// header) and a body holding only the minimap.
#[test]
fn portal_tile_uses_standard_pane_header() {
    let mut h = PlexiUiHarness::new_sized(1200.0, 500.0);
    h.step();
    let portals = seed_long_name_portals(&mut h, None);
    let workspace_root = h.workspace_root().to_path_buf();
    let terminal = h.with_app_mut(|app| {
        let id = app.host.alloc_pane_id();
        let mut term = crate::host::pane::TerminalPane::fixture(
            id,
            app.ctx.clone(),
            app.pty_event_tx.clone(),
            egui_term::BackendSettings {
                shell: "/bin/sh".into(),
                working_directory: Some(workspace_root.clone()),
                ..Default::default()
            },
            14.0,
        )
        .expect("spawn terminal pane");
        term.name = Some("build".to_string());
        let win = &mut app.windows[app.active_window];
        win.panes
            .insert(id, Pane::Terminal(Box::new(term)))
            .unwrap();
        let tile = win.tree.tiles.insert_pane(id);
        let root = win.tree.root.expect("portal root");
        win.tree.root = Some(win.tree.tiles.insert_horizontal_tile(vec![tile, root]));
        id
    });
    h.run_steps(3);
    let header_bg = h.with_app(|app| app.colors.pane_header_bg());
    let header = |h: &PlexiUiHarness, pane: PaneId| {
        h.with_app(|app| {
            let tile = app.visible_pane_rect(pane).expect("pane laid out");
            let bar = crate::ui::layout_report::snapshot(&app.ctx)
                .into_iter()
                .filter(|r| r.surface == "pane_header")
                .map(|r| r.parent)
                .find(|bar| tile.expand(1.0).contains_rect(*bar))
                .unwrap_or_else(|| panic!("pane {pane} painted no pane_header"));
            let layout =
                crate::render::terminal_pane::last_rendered_tab_bar_layout(&app.ctx, pane)
                    .unwrap_or_else(|| panic!("pane {pane} recorded no header layout"));
            (tile, bar, layout)
        })
    };
    let (term_tile, term_bar, term_layout) = header(&h, terminal);
    for portal in portals {
        let (tile, bar, layout) = header(&h, portal);
        assert!(
            (bar.height() - term_bar.height()).abs() < 0.5,
            "header height {bar:?} vs {term_bar:?}"
        );
        assert!(
            (bar.top() - tile.top()).abs() < 0.5
                && (term_bar.top() - term_tile.top()).abs() < 0.5
        );
        assert!(
            (bar.width() - tile.width()).abs() < 1.0,
            "header spans the tile: {bar:?} in {tile:?}"
        );
        assert!(
            h.harness().output().shapes.iter().any(|s| {
                matches!(&s.shape, egui::Shape::Rect(r) if r.rect == bar && r.fill == header_bg)
            }),
            "portal header must paint the shared header background"
        );
        let title = layout.label_rect.expect("portal title painted");
        let term_title = term_layout.label_rect.expect("terminal title painted");
        assert!((title.center().y - bar.center().y).abs() < 1.0);
        assert!(
            (title.height() - term_title.height()).abs() < 0.5,
            "same title font"
        );
        assert!(bar.contains_rect(title));
        assert!(
            layout
                .elided_label
                .as_deref()
                .is_some_and(|l| l.ends_with(crate::ui::text::ELLIPSIS)),
            "a long context name elides: {:?}",
            layout.elided_label
        );
        assert!(!layout
            .elided_label
            .as_deref()
            .unwrap_or_default()
            .contains("child of"));
    }
    let mut texts = Vec::new();
    for shape in &h.harness().output().shapes {
        gallery_text_shapes(&shape.shape, &mut texts);
    }
    for portal in portals {
        let tile = h.with_app(|app| app.visible_pane_rect(portal).unwrap());
        for (rect, text) in &texts {
            if tile.contains_rect(*rect) {
                assert!(
                    !text.contains("very-long-branch-name") && !text.contains(" panes"),
                    "portal body paints only the minimap; found {text:?}"
                );
            }
        }
    }
    h.save_screenshot(&evidence_png!()).unwrap();
}

/// Stint 1043: zoom changes only the viewport. A terminal waiting on the
/// user shows the same shared header (title and NEEDS YOU badge) and the
/// same focus ring geometry before zoom, while zoomed and after restore,
/// and zooming never clears the pending state.
#[test]
fn zoomed_pane_keeps_shared_header_attention_and_focus_ring() {
    let mut h = PlexiUiHarness::new_sized(1200.0, 700.0);
    h.step();
    let other = add_focused_pane(&mut h);
    let workspace_root = h.workspace_root().to_path_buf();
    let pane = h.with_app_mut(|app| {
        app.sidebar_visible = false;
        let id = app.host.alloc_pane_id();
        let mut term = crate::host::pane::TerminalPane::fixture(
            id,
            app.ctx.clone(),
            app.pty_event_tx.clone(),
            egui_term::BackendSettings {
                shell: "/bin/sh".into(),
                working_directory: Some(workspace_root.clone()),
                ..Default::default()
            },
            14.0,
        )
        .expect("spawn terminal pane");
        term.name = Some("release checks".to_string());
        let win = &mut app.windows[app.active_window];
        win.panes
            .insert(id, Pane::Terminal(Box::new(term)))
            .unwrap();
        let tile = win.tree.tiles.insert_pane(id);
        let other_tile = win.tree.tiles.find_pane(&other).expect("other pane tile");
        win.tree.root = Some(
            win.tree
                .tiles
                .insert_horizontal_tile(vec![other_tile, tile]),
        );
        win.focused_pane = Some(tile);
        assert!(win.panes.get_mut(&id).unwrap().set_agent(Some(
            crate::protocol::PaneAgentState {
                pane_id: id,
                agent: "codex".into(),
                state: crate::protocol::AgentState::NeedsYou,
                detail: Some("Approve the release tag".into()),
                session_id: None,
            },
        )));
        id
    });
    h.run_steps(3);

    // Header bar and focus ring for the pane, from emitted paint.
    let chrome = |h: &mut PlexiUiHarness, phase: &str| {
        let (bar, accent, configured_radius) = h.with_app(|app| {
            let bar = crate::ui::layout_report::snapshot(&app.ctx)
                .into_iter()
                .filter(|r| r.surface == "pane_header")
                .find(|r| {
                    r.texts.iter().any(|t| t.text == "NEEDS YOU")
                        && r.texts.iter().any(|t| t.text == "release checks")
                })
                .map(|r| r.parent)
                .unwrap_or_else(|| {
                    panic!("{phase}: no shared header with title and NEEDS YOU badge")
                });
            (bar, app.colors.accent, app.config.pane_corner_radius)
        });
        let ring = h
            .harness()
            .output()
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Rect(r)
                    if r.stroke.color == accent
                        && r.stroke_kind == egui::StrokeKind::Inside
                        && (r.rect.min - bar.min).length() < 0.5 =>
                {
                    Some(r.clone())
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("{phase}: no focus ring framing the header {bar:?}"));
        assert_eq!(
            ring.stroke.width,
            crate::ui::style::PANE_FOCUS_WIDTH,
            "{phase}: focus ring width"
        );
        assert_eq!(
            ring.corner_radius,
            crate::ui::style::pane_radius(configured_radius, ring.rect),
            "{phase}: focus ring corners"
        );
        assert!(
            (bar.width() - ring.rect.width()).abs() < 0.5,
            "{phase}: header spans the pane {bar:?} vs ring {:?}",
            ring.rect
        );
        assert_eq!(
            h.with_app(|app| app
                .find_pane_in_any_window(pane)
                .and_then(|(w, _)| app.windows[w].panes.get(&pane))
                .and_then(|p| p.effective_activity().cloned())),
            Some(crate::protocol::AgentState::NeedsYou),
            "{phase}: zoom must not clear the pending state"
        );
        (bar, ring.rect)
    };

    let (before_bar, before_ring) = chrome(&mut h, "tiled");
    h.save_screenshot(&evidence_png!("_tiled")).unwrap();
    h.with_app_mut(|app| app.toggle_pane_zoom());
    h.run_steps(3);
    assert!(h.with_app(|app| app.windows[app.active_window].zoomed_pane.is_some()));
    let (zoom_bar, zoom_ring) = chrome(&mut h, "zoomed");
    h.save_screenshot(&evidence_png!("_zoomed")).unwrap();
    assert!(
        zoom_ring.width() > before_ring.width(),
        "zoom enlarges the viewport: {zoom_ring:?} vs {before_ring:?}"
    );
    assert!(
        (zoom_bar.height() - before_bar.height()).abs() < 0.5,
        "same header height"
    );
    h.with_app_mut(|app| app.toggle_pane_zoom());
    h.run_steps(3);
    assert!(h.with_app(|app| app.windows[app.active_window].zoomed_pane.is_none()));
    let (after_bar, after_ring) = chrome(&mut h, "restored");
    h.save_screenshot(&evidence_png!("_restored")).unwrap();
    assert_eq!(
        after_bar, before_bar,
        "restore returns the header to its tile"
    );
    assert_eq!(
        after_ring, before_ring,
        "restore returns the ring to its tile"
    );
    assert_eq!(h.with_app(|app| app.visual_owner_pane()), Some(pane));
}

/// Stint 1166: a Portal that shares a tab group paints the tab bar, and
/// clicking a sibling's tab switches to it and back.
#[test]
fn portal_tile_in_tab_group_switches_to_sibling() {
    let mut h = PlexiUiHarness::new_sized(1200.0, 500.0);
    h.step();
    let sibling = add_focused_pane(&mut h);
    let portals = seed_long_name_portals(&mut h, Some(sibling));
    h.run_steps(3);
    let active_member = |h: &PlexiUiHarness| {
        h.with_app(|app| {
            let win = &app.windows[app.active_window];
            let info = win.compute_tab_info();
            let group = info
                .values()
                .find(|g| g.members.contains(&sibling))
                .expect("portal and sibling share a tab group")
                .clone();
            group.members[group.active_idx]
        })
    };
    let click = |h: &mut PlexiUiHarness, pos: egui::Pos2| {
        h.harness().input_mut().events.extend([
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        h.step();
        h.harness()
            .input_mut()
            .events
            .push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            });
        h.run_steps(3);
    };
    assert_eq!(active_member(&h), portals[0], "portal starts active");
    let tile = h.with_app(|app| app.visible_pane_rect(portals[0]).expect("portal laid out"));
    let tab_y = tile.top() + crate::spatial::tiling::TAB_BAR_HEIGHT / 2.0;
    assert!(
        h.with_app(|app| {
            crate::render::terminal_pane::last_rendered_tab_bar_layout(&app.ctx, portals[0])
        })
        .is_some(),
        "active portal tab paints the tab bar"
    );
    click(&mut h, egui::pos2(tile.left() + tile.width() * 0.75, tab_y));
    assert_eq!(
        active_member(&h),
        sibling,
        "clicking the sibling tab switches to it"
    );
    let tile = h.with_app(|app| app.visible_pane_rect(sibling).expect("sibling laid out"));
    click(&mut h, egui::pos2(tile.left() + tile.width() * 0.25, tab_y));
    assert_eq!(
        active_member(&h),
        portals[0],
        "clicking the portal tab switches back"
    );
}

/// Subcontext portal whose child context holds a `text-editor` pane: the
/// minimap must render the document glyph (accent text rules, folded corner)
/// instead of the generic plugin grid, with a status pip sized to match the
/// sidebar. Zoom the portal so the icon is legible in the PNG.
#[test]
fn screenshot_portal_text_editor_icon() {
    // Never point the harnessed file browser at the real temp_dir(): a dev
    // machine's temp dir can hold ~50k entries and rendering them balloons
    // this test past 30 GB RSS. An empty scoped tempdir renders instantly.
    let editor_root = tempfile::tempdir().expect("tempdir");
    let editor_path = editor_root.path().to_path_buf();
    let mut h = PlexiUiHarness::new_sized(900.0, 640.0);
    let _base = add_focused_pane(&mut h);
    h.with_app_mut(|app| {
        let active_ctx_id = app.router.active().context_id;
        let child_ctx_id = active_ctx_id + 5000;
        // Register the child context so the portal header shows a real name.
        app.router.push(crate::host::context::Context {
            name: "Notes".to_string().into(),
            root: editor_path.clone(),
            description: Some("Editing notes".to_string()),
            context_id: child_ctx_id,
            parent_id: Some(active_ctx_id),
            depth: 1,
            parked: false,
        });

        // Child window holding the text-editor pane the portal previews.
        let editor_pane_id = app.host.alloc_pane_id();
        let editor = ViewPane {
            incarnation: Default::default(),
            pip_status: Some(crate::protocol::PipStatus::Green),
            id: editor_pane_id,
            runtime: ViewRuntime::Builtin(Box::new(crate::file_browser::FileBrowserView::new(
                editor_path.clone(),
            ))),
            workspace_root: editor_path.clone(),
            permissions: PluginPermissions::builtin(),
            manifest_id: "text-editor".to_string(),
            name: "note.md".to_string(),
            pane_group: None,
            linked_pane_id: None,
            overlay_replaced: None,
            agent: None,
            slots: std::collections::HashMap::new(),
            semantic_state: Default::default(),
        };
        let mut child_panes = std::collections::HashMap::new();
        child_panes.insert(editor_pane_id, Pane::View(Box::new(editor)));
        let mut child_tiles = egui_tiles::Tiles::default();
        let child_tile = child_tiles.insert_pane(editor_pane_id);
        let child_window_id = app.next_window_id;
        app.next_window_id += 1;
        app.windows.push(Window {
            name: "notes window".to_string(),
            path: editor_path.clone(),
            tree: egui_tiles::Tree::new("notes window", child_tile, child_tiles),
            panes: (child_panes).into(),
            focused_pane: Some(child_tile),
            zoomed_pane: None,
            grid_x: 0,
            grid_y: 0,
            window_id: child_window_id,
            context_id: child_ctx_id,
            tree_history: Default::default(),
        });

        // Portal pane in the active window targeting the child context.
        let portal_pane_id = app.host.alloc_pane_id();
        let active_win = app.active_window;
        let win = &mut app.windows[active_win];
        win.panes
            .insert(
                portal_pane_id,
                Pane::Portal(Box::new(PortalPane {
                    pane_id: portal_pane_id,
                    target_context_id: child_ctx_id,
                    context_state: None,
                })),
            )
            .unwrap();
        let portal_tile = win.tree.tiles.insert_pane(portal_pane_id);
        if let Some(root) = win.tree.root {
            let new_root = win
                .tree
                .tiles
                .insert_horizontal_tile(vec![root, portal_tile]);
            win.tree.root = Some(new_root);
        }
        // Zoom the portal so its minimap fills the window.
        win.zoom_to(portal_tile);
    });
    h.run_steps(3);
    h.save_screenshot(&evidence_png!()).expect("render failed");
    assert!(
        h.with_app(|app| app.windows[app.active_window]
            .panes
            .values()
            .any(|p| p.portal_target().is_some())),
        "active window should contain a portal pane previewing the editor"
    );
    println!("Screenshot saved to {}", evidence_png!());
}

/// Direct paint of `paint_portal_minimap` with hand-built `MiniPane`s — no
/// PTY needed — so every glyph arm (terminal `>_`, text-editor document,
/// generic plugin grid) and the status pip render in one screenshot for size
/// verification. The full-plugin test above cannot fabricate a Terminal pane
/// because that needs a real `TerminalBackend`; this bypasses that by
/// setting `PaneKind` directly.
#[test]
fn screenshot_portal_minimap_glyphs() {
    use crate::spatial::tiling::{paint_portal_minimap, MiniPane, MiniWindow, PaneKind};

    let colors = crate::ui::theme::Colors::from_config(
        &crate::ui::theme::preset_colors("catppuccin-mocha").expect("preset"),
    );
    let mk = |kind, focused, activity, x0: f32, x1: f32| MiniPane {
        pane_id: Some((x0 * 1000.0) as u64),
        norm_rect: egui::Rect::from_min_max(egui::pos2(x0, 0.0), egui::pos2(x1, 1.0)),
        kind,
        focused,
        has_content: true,
        title: Some("Investigate Plexi approval routing | PLEXI".into()),
        active: true,
        activity,
        static_health_pip: false,
    };
    let windows = vec![MiniWindow {
        grid_x: 0,
        grid_y: 0,
        panes: vec![
            mk(PaneKind::Terminal, true, None, 0.0, 0.34),
            mk(
                PaneKind::TextEditor,
                false,
                Some(crate::protocol::AgentState::Working),
                0.34,
                0.67,
            ),
            mk(PaneKind::View, false, None, 0.67, 1.0),
        ],
    }];

    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::Vec2::new(540.0, 320.0))
        .build_ui(move |ui| {
            crate::ui::theme::setup_fonts(ui.ctx());
            egui::CentralPanel::default()
                .frame(egui::Frame::default().fill(colors.bg_darkest))
                .show_inside(ui, |ui| {
                    paint_portal_minimap(ui, ui.max_rect(), &windows, &colors, 0.0);
                });
        });
    harness.run();
    let mut texts = Vec::new();
    for shape in &harness.output().shapes {
        gallery_text_shapes(&shape.shape, &mut texts);
    }
    assert!(
        texts.iter().any(|(_, text)| text == ">_"),
        "the terminal arm paints its prompt glyph: {texts:?}"
    );
    let area = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(540.0, 320.0));
    let titles: Vec<_> = texts
        .iter()
        .filter(|(_, text)| text.contains("Investigate"))
        .collect();
    assert!(!titles.is_empty(), "pane titles paint: {texts:?}");
    for (rect, title) in titles {
        assert!(
            area.contains_rect(*rect),
            "title {title:?} at {rect:?} leaves the minimap area"
        );
    }
    harness
        .render()
        .expect("render failed")
        .save(evidence_png!())
        .expect("save screenshot");
}

#[test]
fn screenshot_visual_escalation_panes_and_portals() {
    let _experiments =
        crate::release::set_test_experiments(&[crate::release::ReleaseFeature::AgentMesh]);
    let mut h = PlexiUiHarness::new_sized_ppp(1440.0, 900.0, 2.0);
    let dir = tempfile::tempdir().expect("tempdir");
    h.step();

    let ws_root = h.workspace_root().to_path_buf();
    let master_pane = h
        .open_target(HarnessOpenTarget::Builtin {
            id: "file_browser",
            cwd: &ws_root,
            args: &[],
        })
        .expect("open initial master pane");
    h.step();

    let send_req = |h: &mut PlexiUiHarness, req: crate::protocol::HostRequest| {
        h.with_app_mut(|app| {
            app.handle_pane_ipc_request(crate::broker::caller::RequestOrigin::HostSystem, req);
        });
        h.run_steps(2);
    };

    let spawn_in_subctx = |h: &mut PlexiUiHarness, subctx: &str, name: &str| -> (u64, u64) {
        let reply = dir.path().join(format!("spawn-{subctx}-{name}.json"));
        send_req(
            h,
            crate::protocol::HostRequest::AgentSpawn {
                anchor_pane: None,
                role: name.into(),
                harness: "codex".into(),
                command: "sleep 30".into(),
                task: Some(format!("Task {name}")),
                subcontext: Some(subctx.into()),
                ephemeral: Some(true),
                name: Some(name.into()),
                cwd: None,
                worktree: None,
                result_schema: None,
                initial_slots: vec![],
                focus: false,
                response_file: reply.to_string_lossy().into_owned(),
            },
        );
        let raw = std::fs::read_to_string(&reply).expect("read spawn response");
        let json: serde_json::Value = serde_json::from_str(&raw).expect("parse json");
        (
            json["pane_id"].as_u64().expect("pane_id"),
            json["portal_pane_id"].as_u64().expect("portal_pane_id"),
        )
    };

    // Subcontext 1 ("squad-alpha"): 2 WORKING · 1 BLOCKED -> amber PortalPane card.
    let (sq1_w1, portal_blocked) = spawn_in_subctx(&mut h, "squad-alpha", "worker-api");
    let (sq1_w2, _) = spawn_in_subctx(&mut h, "squad-alpha", "worker-db");
    let (sq1_b1, _) = spawn_in_subctx(&mut h, "squad-alpha", "worker-schema");
    for wid in [sq1_w1, sq1_w2] {
        send_req(
            &mut h,
            serde_json::from_value(serde_json::json!({
                "type": "set_agent_state",
                "pane_id": wid,
                "state": "working",
                "agent": "codex"
            }))
            .unwrap(),
        );
    }
    send_req(
        &mut h,
        serde_json::from_value(serde_json::json!({
            "type": "set_agent_state",
            "pane_id": sq1_b1,
            "state": "blocked",
            "agent": "codex",
            "detail": "Needs master decision on index"
        }))
        .unwrap(),
    );

    // Subcontext 2 ("squad-deploy"): 1 NEEDS YOU -> coral PortalPane card.
    let (sq2_ny, portal_needs_you) = spawn_in_subctx(&mut h, "squad-deploy", "release-gate");
    send_req(
        &mut h,
        serde_json::from_value(serde_json::json!({
            "type": "set_agent_state",
            "pane_id": sq2_ny,
            "state": "needs_you",
            "agent": "codex",
            "detail": "Approve production deploy"
        }))
        .unwrap(),
    );

    // Subcontext 3 ("squad-ci"): 1 ERROR -> danger PortalPane card.
    let (sq3_err, portal_error) = spawn_in_subctx(&mut h, "squad-ci", "ci-runner");
    h.with_app_mut(|app| {
        for win in &mut app.windows {
            if let Some(crate::host::pane::Pane::Terminal(t)) = win.panes.get_mut(&sq3_err) {
                t.exited = true;
                t.exit_status = Some(crate::host::pane_lifecycle::ExitStatus::Code(1));
                t.agent = None;
                t.observed_agent = None;
                t.activity = None;
            }
        }
    });

    // Split visible terminal panes in the active window so Working, Blocked,
    // NeedsYou, and Error terminal headers/borders are all visible alongside
    // the three PortalPane cards and the sidebar.
    let split_pane = |h: &mut PlexiUiHarness, name: &str| -> u64 {
        let reply = dir.path().join(format!("split-{name}.json"));
        send_req(
            h,
            crate::protocol::HostRequest::AgentSpawn {
                anchor_pane: None,
                role: name.into(),
                harness: "codex".into(),
                command: "sleep 30".into(),
                task: Some(format!("Task {name}")),
                subcontext: None,
                ephemeral: Some(true),
                name: Some(name.into()),
                cwd: None,
                worktree: None,
                result_schema: None,
                initial_slots: vec![],
                focus: false,
                response_file: reply.to_string_lossy().into_owned(),
            },
        );
        let raw = std::fs::read_to_string(&reply).expect("read split response");
        let json: serde_json::Value = serde_json::from_str(&raw).expect("parse json");
        json["pane_id"].as_u64().expect("pane_id")
    };

    let pane_working = split_pane(&mut h, "lane-working");
    let pane_idle = split_pane(&mut h, "lane-idle");
    let pane_blocked = split_pane(&mut h, "lane-blocked");
    let pane_needs_you = split_pane(&mut h, "lane-needs-you");
    let pane_error = split_pane(&mut h, "lane-error");
    let pane_program = h.with_app_mut(|app| {
        let id = app.host.alloc_pane_id();
        let mut terminal = crate::host::pane::TerminalPane::fixture(
            id,
            app.ctx.clone(),
            app.pty_event_tx.clone(),
            egui_term::BackendSettings {
                shell: "/bin/sh".into(),
                working_directory: Some(ws_root.clone()),
                ..Default::default()
            },
            14.0,
        )
        .expect("spawn ordinary terminal");
        terminal.name = Some("ordinary program".into());
        terminal
            .backend
            .process_command(egui_term::BackendCommand::Write(b"sleep 30\n".to_vec()));
        app.windows[app.active_window]
            .panes
            .insert(id, Pane::Terminal(Box::new(terminal)))
            .unwrap();
        id
    });

    // Set pane_working = Working, pane_blocked = Blocked, pane_needs_you = NeedsYou, pane_error = Error,
    // and arrange the active window into a clean 2-row layout:
    //   Top row:    [lane-working | lane-blocked | lane-needs-you | lane-error]
    //   Bottom row: [squad-alpha  | squad-deploy | squad-ci]
    send_req(
        &mut h,
        crate::protocol::HostRequest::SetPipStatus {
            pane_id: master_pane,
            status: crate::protocol::PipStatus::Yellow,
        },
    );
    h.with_app_mut(|app| {
        app.sidebar_visible = true;
        let win = &mut app.windows[app.active_window];
        let mut tiles = egui_tiles::Tiles::default();
        let t_work = tiles.insert_pane(pane_working);
        let t_idle = tiles.insert_pane(pane_idle);
        let t_program = tiles.insert_pane(pane_program);
        let t_plugin = tiles.insert_pane(master_pane);
        let t_blk = tiles.insert_pane(pane_blocked);
        let t_ny = tiles.insert_pane(pane_needs_you);
        let t_err = tiles.insert_pane(pane_error);
        let top_row = tiles.insert_horizontal_tile(vec![
            t_program, t_work, t_idle, t_plugin, t_blk, t_ny, t_err,
        ]);

        let p_blk = tiles.insert_pane(portal_blocked);
        let p_ny = tiles.insert_pane(portal_needs_you);
        let p_err = tiles.insert_pane(portal_error);
        let bottom_row = tiles.insert_horizontal_tile(vec![p_blk, p_ny, p_err]);

        let root = tiles.insert_vertical_tile(vec![top_row, bottom_row]);
        win.tree = egui_tiles::Tree::new("escalation_grid", root, tiles);
        win.focused_pane = Some(t_work);
    });
    send_req(
        &mut h,
        serde_json::from_value(serde_json::json!({
            "type": "set_agent_state", "pane_id": pane_idle,
            "state": "idle", "agent": "codex"
        }))
        .unwrap(),
    );
    send_req(
        &mut h,
        serde_json::from_value(serde_json::json!({
            "type": "set_agent_state",
            "pane_id": pane_working,
            "state": "working",
            "agent": "codex"
        }))
        .unwrap(),
    );
    send_req(
        &mut h,
        serde_json::from_value(serde_json::json!({
            "type": "set_agent_state",
            "pane_id": pane_blocked,
            "state": "blocked",
            "agent": "codex",
            "detail": "Waiting on master"
        }))
        .unwrap(),
    );
    send_req(
        &mut h,
        serde_json::from_value(serde_json::json!({
            "type": "set_agent_state",
            "pane_id": pane_needs_you,
            "state": "needs_you",
            "agent": "codex",
            "detail": "Approve credentials"
        }))
        .unwrap(),
    );
    h.with_app_mut(|app| {
        if let Some(crate::host::pane::Pane::Terminal(t)) =
            app.windows[app.active_window].panes.get_mut(&pane_error)
        {
            t.exited = true;
            t.exit_status = Some(crate::host::pane_lifecycle::ExitStatus::Code(1));
            t.agent = None;
            t.observed_agent = None;
            t.activity = None;
        }
    });

    h.run_steps(4);

    h.with_app(|app| {
        let win = &app.windows[app.active_window];
        assert!(win.panes[&pane_program].agent().is_none());
        assert!(win.panes[&pane_program].effective_activity().is_none());
        assert_eq!(
            win.panes[&pane_working].effective_activity(),
            Some(&crate::protocol::AgentState::Working)
        );
        assert_eq!(
            win.panes[&pane_idle].effective_activity(),
            Some(&crate::protocol::AgentState::Idle)
        );
        assert!(win.panes[&master_pane].static_health_pip());
        let p_blocked = win.panes[&portal_blocked]
            .as_portal()
            .expect("portal_blocked")
            .context_state
            .as_ref()
            .expect("context_state");
        assert_eq!(
            p_blocked.status,
            crate::host::context_state::ContextStatus::Blocked
        );
        assert_eq!(
            p_blocked.status_summary_badge().as_deref(),
            Some("[2 WORKING \u{b7} 1 BLOCKED]")
        );

        let p_needs_you = win.panes[&portal_needs_you]
            .as_portal()
            .expect("portal_needs_you")
            .context_state
            .as_ref()
            .expect("context_state");
        assert_eq!(
            p_needs_you.status,
            crate::host::context_state::ContextStatus::NeedsYou
        );
        assert_eq!(
            p_needs_you.status_summary_badge().as_deref(),
            Some("[1 NEEDS YOU]")
        );

        let p_error = win.panes[&portal_error]
            .as_portal()
            .expect("portal_error")
            .context_state
            .as_ref()
            .expect("context_state");
        assert_eq!(
            p_error.status,
            crate::host::context_state::ContextStatus::Error
        );
        assert_eq!(p_error.status_summary_badge().as_deref(), Some("[1 ERROR]"));

        // Named Working pane keeps its normal header bar without an escalation badge.
        let working_layout =
            crate::render::terminal_pane::last_rendered_tab_bar_layout(&app.ctx, pane_working)
                .expect("named Working terminal pane still renders its name header bar");
        assert_eq!(working_layout.elided_label.as_deref(), Some("lane-working"));
    });

    h.save_screenshot(&evidence_png!()).expect("render failed");
}

#[test]
fn unnamed_working_terminal_does_not_spawn_escalation_header_bar() {
    let mut h = PlexiUiHarness::new_sized(600.0, 400.0);
    h.step();
    let pane_id = h.with_app_mut(|app| {
        app.sidebar_visible = false;
        let workspace_root = app.router.active().root.clone();
        let id = app.host.alloc_pane_id();
        let mut terminal = crate::host::pane::TerminalPane::fixture(
            id,
            app.ctx.clone(),
            app.pty_event_tx.clone(),
            egui_term::BackendSettings {
                shell: "/bin/sh".into(),
                working_directory: Some(workspace_root.clone()),
                ..Default::default()
            },
            14.0,
        )
        .expect("spawn unnamed terminal pane");
        terminal.outside_workspace_cached = false;
        terminal.outside_workspace_checked_at = Some(Instant::now());
        terminal.outside_workspace_root = Some(workspace_root);
        terminal.agent = Some(crate::protocol::PaneAgentState {
            pane_id: id,
            state: crate::protocol::AgentState::Working,
            agent: "codex".into(),
            detail: None,
            session_id: None,
        });

        let win = &mut app.windows[app.active_window];
        win.panes
            .insert(id, Pane::Terminal(Box::new(terminal)))
            .unwrap();
        let tile = win.tree.tiles.insert_pane(id);
        win.tree.root = Some(tile);
        win.focused_pane = Some(tile);
        id
    });

    h.run_steps(2);
    h.with_app(|app| {
        assert!(
            crate::render::terminal_pane::last_rendered_tab_bar_layout(&app.ctx, pane_id)
                .is_none(),
            "unnamed single terminal pane in Working state must not render a 20px escalation header bar"
        );
    });

    // Escalating to Blocked on the same unnamed terminal pane MUST spawn the header bar with fallback "Terminal" title.
    h.with_app_mut(|app| {
        let term = app.windows[app.active_window]
            .panes
            .get_mut(&pane_id)
            .and_then(|p| p.as_terminal_mut())
            .expect("terminal pane");
        term.agent.as_mut().unwrap().state = crate::protocol::AgentState::Blocked;
    });
    h.run_steps(2);
    h.with_app(|app| {
        let layout =
            crate::render::terminal_pane::last_rendered_tab_bar_layout(&app.ctx, pane_id)
                .expect(
                    "Blocked state on unnamed terminal pane must spawn escalation header bar",
                );
        assert_eq!(layout.elided_label.as_deref(), Some("Terminal"));
    });

    // Returning to Working removes the escalation header bar again.
    h.with_app_mut(|app| {
        let term = app.windows[app.active_window]
            .panes
            .get_mut(&pane_id)
            .and_then(|p| p.as_terminal_mut())
            .expect("terminal pane");
        term.agent.as_mut().unwrap().state = crate::protocol::AgentState::Working;
    });
    h.run_steps(2);
    h.with_app(|app| {
        assert!(
            crate::render::terminal_pane::last_rendered_tab_bar_layout(&app.ctx, pane_id)
                .is_none(),
            "returning to Working must clear the temporary escalation header bar"
        );
    });
}

/// Regression: zoomed view panes must render through the shared
/// `render::view_pane::render` path (full-rect fill, no collapsing
/// `egui::Frame`) and exercise the overtake-bar chrome. Guards against
/// the black-square / missing-chrome zoom bugs (zoom overlay previously
/// duplicated pane rendering inline and called `runtime.ui()` directly).
#[test]
fn zoomed_view_pane_renders_via_shared_path() {
    let mut h = PlexiUiHarness::new();
    let pane_id = add_focused_pane(&mut h);
    h.step();
    h.with_app_mut(|app| {
        let win = &mut app.windows[app.active_window];
        // Set overlay_replaced so the zoom path renders the overtake bar
        // ("← <replaced> / Esc return") via render::view_pane::render.
        if let Some(Pane::View(view_pane)) = win.panes.get_mut(&pane_id) {
            view_pane.overlay_replaced = Some(Box::new(Pane::Portal(Box::new(PortalPane {
                pane_id: pane_id + 1_000,
                target_context_id: 42,
                context_state: None,
            }))));
        }
        let tile_id = win
            .tree
            .tiles
            .find_pane(&pane_id)
            .expect("test pane tile missing");
        win.zoom_to(tile_id);
    });
    h.run_steps(3);
    h.render()
        .expect("zoomed view pane frame should render without panic");
    assert!(
        h.with_app(|app| app.windows[app.active_window].zoomed_pane.is_some()),
        "pane should remain zoomed after stepping frames"
    );
    h.save_screenshot(&evidence_png!()).expect("render failed");
}

/// Paints its whole `ui.max_rect()`, as the declarative renderer's plugin
/// backdrop does (`render_ui_tree_with_canvas_fits` fills `max_rect` before
/// drawing the tree). A plugin's paint must never reach host chrome.
struct BackdropFillFixture;

const BACKDROP_FIXTURE_FILL: egui::Color32 = egui::Color32::from_rgb(255, 0, 255);

impl crate::app::view_trait::View for BackdropFillFixture {
    fn type_id(&self) -> &'static str {
        "backdrop_fill_fixture"
    }
    fn display_name(&self) -> String {
        "Backdrop fill".into()
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn ui(
        &mut self,
        ui: &mut egui::Ui,
        _: &crate::app::view_trait::ViewRenderContext<'_>,
    ) {
        ui.painter()
            .rect_filled(ui.max_rect(), 0.0, BACKDROP_FIXTURE_FILL);
    }
}

/// Stint 1267: a plugin view that replaced a pane lost its "← <pane> / Esc
/// return" bar when selected. The plugin was handed the pane's whole rect,
/// so its backdrop fill painted over the bar; only the unfocused-pane
/// opacity let the bar show through. The bar must be visible, and plugin
/// paint must start below it, whether or not the view is focused.
#[test]
fn overtake_bar_stays_visible_over_a_full_rect_plugin_focused_or_not() {
    let (mut h, terminal, view) = pane_chrome_fixture(1.0);
    h.with_app_mut(|app| {
        let Some(Pane::View(view_pane)) = app.windows[app.active_window].panes.get_mut(&view)
        else {
            panic!("fixture view pane missing");
        };
        view_pane.runtime = ViewRuntime::Builtin(Box::new(BackdropFillFixture));
        view_pane.overlay_replaced = Some(Box::new(Pane::Portal(Box::new(PortalPane {
            pane_id: view + 1_000,
            target_context_id: 42,
            context_state: None,
        }))));
    });
    let bar_height = crate::render::view_pane::chrome_height(true, false);
    for (focus, label) in [(view, "focused"), (terminal, "unfocused")] {
        h.focus_pane(focus).unwrap();
        h.run_steps(4);
        let rect = h.with_app(|app| app.visible_pane_rect(view).unwrap());
        let img = h.render().unwrap();
        h.save_screenshot(&evidence_png!(&format!("_{label}")))
            .expect("render overtake bar evidence");
        let ppp = img.width() as f32 / h.with_app(|app| app.ctx.content_rect().width());
        let is_fixture = |y: f32| {
            let [r, g, b, _] = img
                .get_pixel((rect.center().x * ppp) as u32, (y * ppp) as u32)
                .0;
            r > 150 && b > 150 && g < 100
        };
        let mut y = rect.top() + 0.5;
        while y < rect.top() + bar_height + 40.0 && !is_fixture(y) {
            y += 0.5;
        }
        assert!(
            y < rect.top() + bar_height + 40.0,
            "{label}: the fixture plugin must paint its content area"
        );
        assert!(
            y >= rect.top() + bar_height - 1.0,
            "{label}: plugin paint starts {:.1}px below the pane top, over the {bar_height}px overtake bar",
            y - rect.top()
        );
    }
}

#[test]
fn pane_chrome_selected_view_keeps_its_state_ring() {
    let (mut h, terminal, view_pane) = pane_chrome_fixture(1.0);
    h.with_app_mut(|app| {
        app.windows[app.active_window]
            .panes
            .get_mut(&view_pane)
            .unwrap()
            .set_agent(Some(crate::protocol::PaneAgentState {
                pane_id: view_pane,
                state: crate::protocol::AgentState::Blocked,
                agent: "fixture".into(),
                detail: None,
                session_id: None,
            }));
        let window = app.windows[app.active_window].window_id;
        // Selected from the view to the terminal, which keeps focus: a
        // selection only lasts while focus is where its last step left it.
        app.host.extend_pane_selection(window, view_pane, terminal);
    });
    h.focus_pane(terminal).unwrap();
    h.run_steps(3);
    let (rect, color) = h.with_app(|app| {
        (
            app.visible_pane_rect(view_pane).unwrap(),
            app.colors.state_blocked,
        )
    });
    let ring = h
        .harness()
        .output()
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Path(p)
                if p.stroke.color == egui::epaint::ColorMode::Solid(color)
                    && p.points.iter().all(|point| rect.contains(*point)) =>
            {
                Some(p)
            }
            _ => None,
        })
        .next()
        .expect("selected plugin retains its state ring");
    let left = ring
        .points
        .iter()
        .map(|p| p.x)
        .fold(f32::INFINITY, f32::min)
        - ring.stroke.width / 2.0;
    assert!((left - rect.left() - crate::ui::style::PANE_FOCUS_WIDTH).abs() < 0.01);
}

#[test]
fn pane_chrome_view_tabs_use_responsive_state_header() {
    let mut h = PlexiUiHarness::new_sized(500.0, 300.0);
    h.step();
    let first = add_focused_pane(&mut h);
    let second = add_focused_pane(&mut h);
    h.with_app_mut(|app| {
        app.sidebar_visible = false;
        let win = &mut app.windows[app.active_window];
        win.panes
            .get_mut(&first)
            .unwrap()
            .set_agent(Some(crate::protocol::PaneAgentState {
                pane_id: first,
                state: crate::protocol::AgentState::Blocked,
                agent: "fixture".into(),
                detail: None,
                session_id: None,
            }));
        let a = win.tree.tiles.find_pane(&first).unwrap();
        let b = win.tree.tiles.find_pane(&second).unwrap();
        win.tree.root = Some(win.tree.tiles.insert_tab_tile(vec![a, b]));
        win.focused_pane = Some(a);
    });
    h.run_steps(3);
    let (layout, fill) = h.with_app(|app| {
        let color = app.colors.status_badge_color(app.colors.state_blocked);
        (
            crate::render::terminal_pane::last_rendered_tab_bar_layout(&app.ctx, first)
                .expect("plugin tab header uses shared layout"),
            egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 36),
        )
    });
    let badge = h
        .harness()
        .output()
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Rect(r) if r.fill == fill => Some(r.rect),
            _ => None,
        })
        .expect("wide plugin tab header shows state chip");
    if let Some(title) = layout.label_rect {
        assert!(
            !title.intersects(badge),
            "plugin tab title must not overlap its state chip"
        );
    }
}

#[test]
fn pane_chrome_rings_are_concentric_at_both_scales() {
    for scale in [1.0, 2.0] {
        let (mut h, pane, _) = pane_chrome_fixture(scale);
        h.with_app_mut(|app| {
            app.windows[app.active_window]
                .panes
                .get_mut(&pane)
                .unwrap()
                .set_agent(Some(crate::protocol::PaneAgentState {
                    pane_id: pane,
                    state: crate::protocol::AgentState::Blocked,
                    agent: "fixture".into(),
                    detail: None,
                    session_id: None,
                }));
        });
        h.focus_pane(pane).unwrap();
        h.run_steps(3);
        let (rect, color, accent) = h.with_app(|app| {
            (
                app.visible_pane_rect(pane).unwrap(),
                app.colors.state_blocked,
                app.colors.accent,
            )
        });
        let radius = crate::ui::style::pane_radius(None, rect).nw as f32;
        let focus = h
            .harness()
            .output()
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Rect(r) if r.stroke.color == accent && r.rect == rect => Some(r),
                _ => None,
            })
            .next()
            .expect("focus ring");
        assert_eq!(focus.corner_radius.nw as f32, radius);
        let ring = h
            .harness()
            .output()
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Path(r)
                    if r.stroke.color == egui::epaint::ColorMode::Solid(color)
                        && r.points.iter().all(|p| rect.contains(*p)) =>
                {
                    Some(r)
                }
                _ => None,
            })
            .next()
            .expect("state ring");
        assert!((ring.points[0].x - (rect.right() - radius)).abs() < 0.01);
        assert!(
            (ring.points[8].y - (rect.top() + radius)).abs() < 0.01,
            "ring curves must share the tile corner center"
        );
    }
}

#[test]
fn screenshot_pane_chrome_state_matrix() {
    use crate::protocol::AgentState;
    let states = [
        ("working", AgentState::Working),
        ("idle", AgentState::Idle),
        ("blocked", AgentState::Blocked),
        ("needs_you", AgentState::NeedsYou),
        ("interrupted", AgentState::Interrupted),
        ("failed", AgentState::Failed),
        ("completed", AgentState::Completed),
    ];
    for theme in ["plexi-dusk", "plexi-dawn"] {
        for scale in [1.0, 2.0] {
            for wide in [false, true] {
                let mut h = PlexiUiHarness::new_sized_scaled(1400.0, 800.0, scale);
                h.step();
                std::fs::write(
                    crate::config::config_path(),
                    format!("ui_scale = {scale}\n[theme]\npreset = \"{theme}\"\n"),
                )
                .unwrap();
                h.with_app_mut(|app| {
                    app.reload_config();
                    app.ui_scale = scale;
                    app.sidebar_visible = false;
                });
                h.run_steps(3);
                h.with_app(|app| {
                    let expected = crate::ui::theme::Colors::from_config(
                        &crate::ui::theme::preset_colors(theme).unwrap(),
                    );
                    assert_eq!(
                        app.colors.bg_darkest, expected.bg_darkest,
                        "fixture must render the requested theme"
                    );
                });
                let root = h.workspace_root().to_path_buf();
                let mut ids = Vec::new();
                for (name, state) in &states {
                    let id = add_focused_pane(&mut h);
                    ids.push(id);
                    h.with_app_mut(|app| {
                    let mut term = crate::host::pane::TerminalPane::fixture(id, app.ctx.clone(), app.pty_event_tx.clone(),
                        egui_term::BackendSettings { shell: "/bin/sh".into(), args: vec!["-c".into(), "printf 'Agent task\\nReviewing changes and tests.\\n'; sleep 300".into()], working_directory: Some(root.clone()), ..Default::default() }, 14.0).expect("spawn test terminal");
                    term.name = Some(format!("{name}: review implementation"));
                    term.outside_workspace_cached = false;
                    term.outside_workspace_checked_at = Some(Instant::now());
                    term.outside_workspace_root = Some(root.clone());
                    term.agent = Some(crate::protocol::PaneAgentState { pane_id: id, state: state.clone(), agent: "codex".into(), detail: None, session_id: None });
                    app.windows[app.active_window].take_pane(id);
                    app.windows[app.active_window].panes.insert(id, Pane::Terminal(Box::new(term))).unwrap();
                });
                }
                h.with_app_mut(|app| {
                    app.sidebar_visible = false;
                    let win = &mut app.windows[app.active_window];
                    let tiles: Vec<_> = ids
                        .iter()
                        .map(|id| win.tree.tiles.find_pane(id).unwrap())
                        .collect();
                    win.tree.root = Some(if wide {
                        win.tree.tiles.insert_grid_tile(tiles)
                    } else {
                        win.tree.tiles.insert_horizontal_tile(tiles)
                    });
                });
                for (index, (name, _)) in states.iter().enumerate() {
                    h.focus_pane(ids[index]).unwrap();
                    h.run_steps(3);
                    h.with_app(|app| {
                        assert!((app.ctx.zoom_factor() - scale).abs() < 0.001);
                        let reports = crate::ui::layout_report::snapshot(&app.ctx);
                        assert!(reports.iter().any(|r| r.surface == "pane_header"));
                        let errors: Vec<_> =
                            reports.iter().flat_map(|r| r.violations()).collect();
                        assert!(
                            errors.is_empty(),
                            "{theme} {scale} {wide} {name}: {errors:?}"
                        );
                    });
                    let path = evidence_png!(&format!(
                        "_{theme}_{scale}_{}_{name}",
                        if wide { "wide" } else { "narrow" }
                    ));
                    h.save_screenshot(&path).unwrap();
                    println!("screenshot: {path}");
                }
            }
        }
    }
}

/// Regression: tab titles with long names must stay on a single line and show
/// a trailing ellipsis rather than wrapping or overflowing the tab bar rect.
#[test]
fn tab_titles_truncate_to_single_line() {
    let mut h = PlexiUiHarness::new();
    h.step();

    let browser_root = h.workspace.path().to_path_buf();
    h.with_app_mut(|app| {
        let pane_id_a = app.host.alloc_pane_id();
        let pane_id_b = app.host.alloc_pane_id();
        for (pane_id, name) in [
            (pane_id_a, "A very long tab title that should not wrap onto a second line under any circumstances"),
            (pane_id_b, "Short"),
        ] {
            let view_pane = ViewPane {
                incarnation: Default::default(),
                pip_status: None,
                id: pane_id,
                runtime: ViewRuntime::Builtin(Box::new(
                    crate::file_browser::FileBrowserView::new(browser_root.clone()),
                )),
                workspace_root: browser_root.clone(),
                permissions: PluginPermissions::builtin(),
                manifest_id: "test".to_string(),
                name: name.to_string(),
                pane_group: None,
                linked_pane_id: None,
                overlay_replaced: None,
                agent: None,
                slots: std::collections::HashMap::new(),
                semantic_state: Default::default(),
            };
            let win = &mut app.windows[app.active_window];
            win.panes.insert(pane_id, Pane::View(Box::new(view_pane))).unwrap();
        }
        let win = &mut app.windows[app.active_window];
        let tile_a = win.tree.tiles.insert_pane(pane_id_a);
        let tile_b = win.tree.tiles.insert_pane(pane_id_b);
        let tab_tile = win.tree.tiles.insert_tab_tile(vec![tile_a, tile_b]);
        win.tree.root = Some(tab_tile);
        win.focused_pane = Some(tile_a);
    });

    h.run_steps(2);
    // The header's title slot owns the long name: it must stay one line
    // inside its allocation, eliding rather than wrapping or overflowing.
    assert_layout_surface(&h, "pane_header");
    h.save_screenshot(&evidence_png!())
        .expect("screenshot failed");
}

/// Ledger row 155: a sealed pane says why in itself, and its notice opens
/// a new pane beside it, owned by the person.
#[test]
fn screenshot_sealed_pane_notice_and_new_pane() {
    let mut h = PlexiUiHarness::new_sized(1100.0, 600.0);
    h.step();
    let root = h.workspace.path().to_path_buf();
    let sealed = h.with_app_mut(|app| {
        let context = app.pane_context_env_for_window(app.active_window);
        let pane_id = app.host.alloc_pane_id();
        let grant = crate::broker::pane_authority::PaneAuthority::sealed(
            crate::broker::pane_authority::SealReason::OlderSave,
            &crate::config::config_dir(),
            pane_id,
            crate::broker::pane_authority::Provenance::LegacyImport,
            None,
            "unknown".into(),
            crate::pane_ops::grant::credential_scope(&context, Some(&root)),
        );
        let pane = app
            .construct_terminal(crate::pane_ops::construct::TerminalLaunch {
                authority: crate::broker::pane_authority::LivePaneAuthority::admitted(grant),
                context,
                cwd: Some(root.clone()),
                command: None,
                ephemeral: false,
                env: Vec::new(),
                mcp: crate::pane_ops::construct::McpBearer::Pane,
                profile_dir: crate::config::config_dir(),
            })
            .expect("terminal starts");
        let win = &mut app.windows[app.active_window];
        win.panes
            .insert(pane_id, Pane::Terminal(Box::new(pane)))
            .unwrap();
        let tile = win.tree.tiles.insert_pane(pane_id);
        win.tree.root = Some(tile);
        win.focused_pane = Some(tile);
        pane_id
    });
    h.run_steps(3);
    let notice = h
        .harness()
        .get_by_label_contains("restored from an older save; open a new pane");
    assert!(notice.rect().width() > 0.0, "the notice is painted");
    h.save_screenshot(&evidence_png!())
        .expect("screenshot failed");
    // An injected click through the pane input queue (`pane click`) is
    // refused: only physical input opens a pane the person owns.
    let button = h.harness().get_by_label("Open a new pane").rect();
    let click = injected_click(&h, button);
    injected(&mut h, click);
    assert_eq!(
        h.with_app(|app| app.windows[app.active_window].panes.len()),
        1,
        "a synthetic click opens nothing"
    );
    h.harness().get_by_label("Open a new pane").click();
    h.run_steps(3);
    let opened = h.with_app(|app| {
        app.windows[app.active_window]
            .panes
            .keys()
            .copied()
            .find(|id| *id != sealed)
    });
    let opened = opened.expect("a new pane opened beside the sealed one");
    assert!(h.with_app(|app| app.terminal_incarnation(opened).is_some_and(|t| t.is_desktop_granted())));
    assert!(h.with_app(|app| app.terminal_incarnation(sealed).is_some()));
}

/// Stint 0755: when `outside_workspace` is active on a terminal pane,
/// `render_name_bar_and_tabs` must reserve the `outside workspace` badge's
/// width plus `SPACE_XS` on both single-tab and multi-tab paths and elide
/// the tab label in the remaining space so the label galley and badge rect
/// never intersect in narrow (~400px) tiles.
#[test]
fn outside_workspace_badge_does_not_overlap_tab_label_in_narrow_tile() {
    let long_cwd_label =
        "/Users/ianburke/Documents/GitHub/external-workspace/packages/core/src/components";

    // 1. Single-tab path in a 400px-wide terminal tile.
    {
        let mut h = PlexiUiHarness::new_sized(400.0, 260.0);
        h.step();
        let workspace_root = h.workspace_root().to_path_buf();
        let pane_id = h.with_app_mut(|app| {
            app.sidebar_visible = false;
            let id = app.host.alloc_pane_id();
            let mut terminal = crate::host::pane::TerminalPane::fixture(
                id,
                app.ctx.clone(),
                app.pty_event_tx.clone(),
                egui_term::BackendSettings {
                    shell: "/bin/sh".into(),
                    working_directory: Some(workspace_root.clone()),
                    ..Default::default()
                },
                14.0,
            )
            .expect("spawn terminal pane");
            terminal.name = Some(long_cwd_label.to_string());
            terminal.outside_workspace_cached = true;
            terminal.outside_workspace_checked_at = Some(Instant::now());
            terminal.outside_workspace_root = Some(workspace_root);

            let win = &mut app.windows[app.active_window];
            win.panes
                .insert(id, Pane::Terminal(Box::new(terminal)))
                .unwrap();
            let tile = win.tree.tiles.insert_pane(id);
            win.tree.root = Some(tile);
            win.focused_pane = Some(tile);
            id
        });

        h.run_steps(2);
        let single_shot = evidence_png!("_single_tab");
        h.save_screenshot(&single_shot)
            .expect("save single-tab outside_workspace screenshot");

        let layout = h
            .with_app(|app| {
                crate::render::terminal_pane::last_rendered_tab_bar_layout(&app.ctx, pane_id)
            })
            .expect("single-tab terminal pane must record tab-bar layout");
        let label_rect = layout.label_rect.expect("single-tab label_rect");
        let badge_rect = layout
            .outside_workspace_badge_rect
            .expect("single-tab outside_workspace_badge_rect");
        assert!(
            !label_rect.intersects(badge_rect),
            "single-tab label_rect ({label_rect:?}) must not intersect outside_workspace badge_rect ({badge_rect:?}); screenshot: {single_shot}"
        );
        assert!(
            label_rect.right() <= badge_rect.left() - crate::ui::style::SPACE_XS + 0.5,
            "single-tab label right ({}) must leave at least SPACE_XS before badge left ({}); screenshot: {single_shot}",
            label_rect.right(),
            badge_rect.left()
        );
        assert!(
            layout
                .elided_label
                .as_deref()
                .unwrap_or_default()
                .ends_with(crate::ui::text::ELLIPSIS),
            "single-tab long cwd label must elide with '…', got {:?}",
            layout.elided_label
        );
    }

    // 2. Multi-tab path in a 400px-wide terminal tile (rightmost tab active
    // directly adjacent to the right-pinned `outside workspace` badge).
    {
        let mut h = PlexiUiHarness::new_sized(400.0, 260.0);
        h.step();
        let workspace_root = h.workspace_root().to_path_buf();
        let pane_id_b = h.with_app_mut(|app| {
            app.sidebar_visible = false;
            let id_a = app.host.alloc_pane_id();
            let id_b = app.host.alloc_pane_id();
            let mut term_a = crate::host::pane::TerminalPane::fixture(
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
            term_a.name = Some("~/Documents/GitHub/PLEXI/crates/daw-engine".to_string());
            term_a.outside_workspace_cached = false;
            term_a.outside_workspace_checked_at = Some(Instant::now());
            term_a.outside_workspace_root = Some(workspace_root.clone());

            let mut term_b = crate::host::pane::TerminalPane::fixture(
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
            term_b.name = Some(long_cwd_label.to_string());
            term_b.outside_workspace_cached = true;
            term_b.outside_workspace_checked_at = Some(Instant::now());
            term_b.outside_workspace_root = Some(workspace_root);

            let win = &mut app.windows[app.active_window];
            win.panes
                .insert(id_a, Pane::Terminal(Box::new(term_a)))
                .unwrap();
            win.panes
                .insert(id_b, Pane::Terminal(Box::new(term_b)))
                .unwrap();
            let tile_a = win.tree.tiles.insert_pane(id_a);
            let tile_b = win.tree.tiles.insert_pane(id_b);
            let tab_tile = win.tree.tiles.insert_tab_tile(vec![tile_a, tile_b]);
            if let Some(egui_tiles::Tile::Container(egui_tiles::Container::Tabs(tabs))) =
                win.tree.tiles.get_mut(tab_tile)
            {
                tabs.set_active(tile_b);
            }
            win.tree.root = Some(tab_tile);
            win.focused_pane = Some(tile_b);
            id_b
        });

        h.run_steps(2);
        let multi_shot = evidence_png!("_multi_tab");
        h.save_screenshot(&multi_shot)
            .expect("save multi-tab outside_workspace screenshot");

        let layout = h
            .with_app(|app| {
                crate::render::terminal_pane::last_rendered_tab_bar_layout(&app.ctx, pane_id_b)
            })
            .expect("multi-tab active terminal pane must record tab-bar layout");
        let label_rect = layout.label_rect.expect("multi-tab label_rect");
        let badge_rect = layout
            .outside_workspace_badge_rect
            .expect("multi-tab outside_workspace_badge_rect");
        assert!(
            !label_rect.intersects(badge_rect),
            "multi-tab label_rect ({label_rect:?}) must not intersect outside_workspace badge_rect ({badge_rect:?}); screenshot: {multi_shot}"
        );
        assert!(
            label_rect.right() <= badge_rect.left() - crate::ui::style::SPACE_XS + 0.5,
            "multi-tab label right ({}) must leave at least SPACE_XS before badge left ({}); screenshot: {multi_shot}",
            label_rect.right(),
            badge_rect.left()
        );
        assert!(
            layout
                .elided_label
                .as_deref()
                .unwrap_or_default()
                .ends_with(crate::ui::text::ELLIPSIS),
            "multi-tab long cwd label must elide with '…', got {:?}",
            layout.elided_label
        );
    }
}
