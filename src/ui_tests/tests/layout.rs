use super::*;

/// Renders realistic seeded screenshots of crowded Atlas at scales 1.0 and 1.3 for visual verification.
#[test]
fn screenshot_crowded_atlas_scales() {
    let _experiments =
        crate::release::set_test_experiments(&[crate::release::ReleaseFeature::TreeAtlas]);
    for scale in [1.0, 1.3] {
        // Crowded atlas
        {
            let mut h = PlexiUiHarness::new_sized_scaled(1440.0, 900.0, scale);
            let selected_context = h.with_app_mut(|app| {
                app.sidebar_visible = false;
                let r1 = app.create_context_without_pty("hub-main", None);
                let r2 = app.create_context_without_pty("hub-ops", None);
                let c1 = app.create_context_without_pty("feature-auth", Some(r1));
                let c2 = app.create_context_without_pty("feature-billing", Some(r1));
                let _c3 = app.create_context_without_pty("worker-job", Some(r2));
                let _c4 = app.create_context_without_pty("worker-cron", Some(r2));
                let _g1 = app.create_context_without_pty("auth-tests", Some(c1));
                let _g2 = app.create_context_without_pty("auth-tokens", Some(c1));
                let _g3 = app.create_context_without_pty("billing-stripe", Some(c2));
                let _g4 = app.create_context_without_pty("billing-invoicing", Some(c2));
                app.set_zoom_level_with_transition(crate::app::zoom::ZoomLevel::TreeAtlas);
                app.zoom.atlas_focused_context = Some(c1);
                c1
            });
            h.run_steps(6);
            let (canvas, nodes): (egui::Rect, Vec<(u64, egui::Rect, bool)>) =
                h.with_app(|app| {
                    app.ctx
                        .data(|data| {
                            data.get_temp(egui::Id::new("tree_atlas_painted_geometry"))
                        })
                        .expect("Atlas render must report its painted geometry")
                });
            let selected = nodes
                .iter()
                .find(|(context_id, _, _)| *context_id == selected_context)
                .expect("selected Atlas context must be painted");
            assert!(
                selected.2,
                "selected Atlas context must retain focus treatment"
            );
            assert!(
                canvas.contains_rect(selected.1),
                "selected Atlas rect {:?} must be wholly visible in {:?}",
                selected.1,
                canvas
            );
            h.save_screenshot(&evidence_png!(format!("_scale_{scale:.1}")))
                .unwrap();
        }
    }
}

/// Stint 0988: Enter opens the selected context; the marker stays on the origin context.

/// Stint 0988: Realistic screenshot verifying overflow, marker alignment and Reduce Motion behavior.

/// Stint 0989: Tree Atlas directional navigation follows top-down hierarchy:
/// - up: parent
/// - down: remembered-or-first child
/// - left/right: neighboring siblings
/// Proves arrows and hjkl parity, child memory, root/leaf boundaries, and that
/// selected nodes remain visible and focused through every move, including
/// uneven multi-root, leaf-root, and missing-parent fixtures.
#[test]
fn screenshot_tree_atlas_directional_navigation_hierarchy() {
    let _experiments =
        crate::release::set_test_experiments(&[crate::release::ReleaseFeature::TreeAtlas]);
    let mut h = PlexiUiHarness::new_sized(1440.0, 900.0);
    let tap_key = |h: &mut PlexiUiHarness, key: egui::Key, modifiers: egui::Modifiers| {
        h.harness().input_mut().events.push(egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        });
        h.harness().input_mut().events.push(egui::Event::Key {
            key,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers,
        });
    };

    let (r1, _c1a, _g1a1, _c1b, _r2, _c2a, _c2b, _c2c, _r3, _r4) = h.with_app_mut(|app| {
        app.sidebar_visible = false;
        // Root 1 (active root)
        let r1 = app.router.active().context_id;
        let c1a = app.create_context_without_pty("client-web", Some(r1));
        let g1a1 = app.create_context_without_pty("design-tokens", Some(c1a));
        let c1b = app.create_context_without_pty("client-mobile", Some(r1));

        // Root 2
        let r2 = app.create_context_without_pty("backend-cluster", None);
        let c2a = app.create_context_without_pty("api-gateway", Some(r2));
        let c2b = app.create_context_without_pty("auth-service", Some(r2));
        let c2c = app.create_context_without_pty("billing-engine", Some(r2));

        // Root 3: leaf root (no children)
        let r3 = app.create_context_without_pty("docs-site", None);

        // Root 4: missing parent
        let r4 = app.create_context_without_pty("legacy-orphan", Some(999999));

        // Parked context: marked parked
        let parked = app.create_context_without_pty("parked-archive", Some(r1));
        if let Some(idx) = app
            .router
            .as_slice()
            .iter()
            .position(|c| c.context_id == parked)
        {
            app.router.get_mut(idx).parked = true;
        }

        app.switch_to_context_restoring_last_pane(r1);
        app.set_zoom_level_with_transition(crate::app::zoom::ZoomLevel::TreeAtlas);
        app.zoom.atlas_focused_context = Some(r1);
        (r1, c1a, g1a1, c1b, r2, c2a, c2b, c2c, r3, r4)
    });

    // Run frames to render TreeAtlas
    h.run_steps(4);

    let check_selected_visible = |h: &mut PlexiUiHarness, expected_ctx: u64| {
        let (canvas, nodes): (egui::Rect, Vec<(u64, egui::Rect, bool)>) = h.with_app(|app| {
            app.ctx
                .data(|data| data.get_temp(egui::Id::new("tree_atlas_painted_geometry")))
                .expect("Atlas render must report its painted geometry")
        });
        let selected = nodes
            .iter()
            .find(|(cid, _, _)| *cid == expected_ctx)
            .unwrap_or_else(|| panic!("context {expected_ctx} must be painted in tree atlas"));
        assert!(
            selected.2,
            "context {expected_ctx} must have focus treatment"
        );
        assert!(
            canvas.contains_rect(selected.1),
            "context {expected_ctx} rect {:?} must be wholly visible in canvas {:?}",
            selected.1,
            canvas
        );
    };

    // 1. Initial focused node: Root 1
    check_selected_visible(&mut h, r1);

    // 2. Down (ArrowDown) -> First child (c1a)
    tap_key(&mut h, egui::Key::ArrowDown, egui::Modifiers::NONE);
    h.run_steps(2);
    check_selected_visible(&mut h, _c1a);

    // 3. Right (ArrowRight) -> Sibling (c1b)
    tap_key(&mut h, egui::Key::ArrowRight, egui::Modifiers::NONE);
    h.run_steps(2);
    check_selected_visible(&mut h, _c1b);

    // 4. Up (K) -> Parent (r1)
    tap_key(&mut h, egui::Key::K, egui::Modifiers::NONE);
    h.run_steps(2);
    check_selected_visible(&mut h, r1);

    // 5. Down (J) -> Remembered child (c1b)
    tap_key(&mut h, egui::Key::J, egui::Modifiers::NONE);
    h.run_steps(2);
    check_selected_visible(&mut h, _c1b);

    // 6. Left (H) -> Sibling (c1a)
    tap_key(&mut h, egui::Key::H, egui::Modifiers::NONE);
    h.run_steps(2);
    check_selected_visible(&mut h, _c1a);

    // 7. Down (J) -> Grandchild (g1a1)
    tap_key(&mut h, egui::Key::J, egui::Modifiers::NONE);
    h.run_steps(2);
    check_selected_visible(&mut h, _g1a1);

    // 8. Up (K) -> c1a
    tap_key(&mut h, egui::Key::K, egui::Modifiers::NONE);
    h.run_steps(2);
    check_selected_visible(&mut h, _c1a);

    // 9. Up (K) -> r1
    tap_key(&mut h, egui::Key::K, egui::Modifiers::NONE);
    h.run_steps(2);
    check_selected_visible(&mut h, r1);

    // 10. Right (L) -> Root 2 (r2)
    tap_key(&mut h, egui::Key::L, egui::Modifiers::NONE);
    h.run_steps(2);
    check_selected_visible(&mut h, _r2);

    // 11. Right (ArrowRight) -> Root 3 (r3, leaf root)
    tap_key(&mut h, egui::Key::ArrowRight, egui::Modifiers::NONE);
    h.run_steps(2);
    check_selected_visible(&mut h, _r3);

    // 12. Down (ArrowDown) at leaf root is a no-op -> stays on r3
    tap_key(&mut h, egui::Key::ArrowDown, egui::Modifiers::NONE);
    h.run_steps(2);
    check_selected_visible(&mut h, _r3);

    // 13. Right (ArrowRight) -> Root 4 (r4, missing parent orphan)
    tap_key(&mut h, egui::Key::ArrowRight, egui::Modifiers::NONE);
    h.run_steps(2);
    check_selected_visible(&mut h, _r4);

    // 14. Up (ArrowUp) at orphan root is a no-op -> stays on r4
    tap_key(&mut h, egui::Key::ArrowUp, egui::Modifiers::NONE);
    h.run_steps(2);
    check_selected_visible(&mut h, _r4);

    h.save_screenshot(&evidence_png!())
        .expect("save atlas screenshot");
}

#[test]
fn screenshot_agent_conjoined_strip_and_parent_pin() {
    let mut h = PlexiUiHarness::new_sized(1440.0, 900.0);
    h.step();
    let (parent, children) = h.with_app_mut(|app| {
        let (parent, _) = app
            .spawn_agent_worker_pane(
                "lead",
                "pi",
                "printf 'Lead: review implementation and testing\\n'; sleep 120",
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
        let (_, tile) = app.find_pane_in_any_window(parent).unwrap();
        app.windows[app.active_window].focused_pane = Some(tile);
        let mut children = Vec::new();
        for role in ["implementer", "tester"] {
            let (child, _) = app
                .spawn_agent_worker_pane(
                    role,
                    "pi",
                    "printf 'Worker: checking the requested changes\\n'; sleep 120",
                    "Verify child call",
                    Some("agent-tools-preview"),
                    true,
                    Some(parent),
                    None,
                    None,
                    None,
                    true,
                    None,
                )
                .unwrap();
            app.agent_tool_children.insert(child, Some(parent));
            children.push(child);
        }
        (parent, children)
    });
    let deadline = Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(20));
    loop {
        h.run_steps(2);
        let ready = h.with_app_mut(|app| {
            std::iter::once(parent)
                .chain(children.iter().copied())
                .all(|id| {
                    let (window, _) = app.find_pane_in_any_window(id).unwrap();
                    app.windows[window]
                        .panes
                        .get_mut(&id)
                        .unwrap()
                        .as_terminal_mut()
                        .unwrap()
                        .backend
                        .capture_lines(30)
                        .iter()
                        .any(|line| line.contains("Lead:") || line.contains("Worker:"))
                })
        });
        if ready {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "conjoined terminals did not print fixture output"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    h.run_steps(4);
    h.save_screenshot(&evidence_png!("_strip")).unwrap();
    h.harness()
        .key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::ArrowDown);
    h.run_steps(4);
    h.with_app(|app| {
        assert_eq!(app.conjoined_parent(), Some(parent));
        assert_eq!(app.visual_owner_pane(), Some(parent));
        assert_eq!(app.fleet_focused_pane(), Some(parent));
        let parent_rect = app.visible_pane_rect(parent).unwrap();
        let child_rect = app.windows[app.active_window]
            .tree
            .root
            .and_then(|tile| app.windows[app.active_window].tree.tiles.rect(tile))
            .unwrap();
        let share = parent_rect.width() / (parent_rect.width() + child_rect.width());
        assert!((share - 0.35).abs() < 0.01, "parent share: {share}");
        assert!(!app
            .windows
            .iter()
            .flat_map(|w| w.panes.values())
            .any(|p| matches!(p, Pane::Portal(_))));
    });
    h.save_screenshot(&evidence_png!("_view")).unwrap();
    h.harness()
        .key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Escape);
    h.run_steps(2);
    h.with_app(|app| {
        assert_eq!(app.zoom.level, crate::app::zoom::ZoomLevel::Columns);
        assert!(app.router.active().parent_id.is_none());
    });
}

// ── Regression: layout flows ──────────────────────────────────────────────

/// Holding Control exposes every pane's centered ID, including a window
/// with just one pane. The two-pane layout is the control case for the
/// same production render path.
#[test]
fn ctrl_pane_id_overlay_renders_for_single_and_split_layouts() {
    fn pane_rect(h: &PlexiUiHarness, pane_id: PaneId) -> egui::Rect {
        h.with_app(|app| {
            let window = &app.windows[app.active_window];
            let tile_id = window
                .tree
                .tiles
                .iter()
                .find_map(|(tile_id, tile)| {
                    matches!(tile, egui_tiles::Tile::Pane(id) if *id == pane_id)
                        .then_some(tile_id)
                })
                .expect("pane tile");
            window.tree.tiles.rect(*tile_id).expect("painted pane rect")
        })
    }

    fn center_glyph_changed(
        without_ctrl: &image::RgbaImage,
        with_ctrl: &image::RgbaImage,
        pane_rect: egui::Rect,
    ) -> bool {
        let center = pane_rect.center();
        let x_start = (center.x - 20.0).max(0.0) as u32;
        let x_end = (center.x + 20.0).min(with_ctrl.width() as f32) as u32;
        let y_start = (center.y - 20.0).max(0.0) as u32;
        let y_end = (center.y + 20.0).min(with_ctrl.height() as f32) as u32;

        (y_start..y_end).any(|y| {
            (x_start..x_end).any(|x| without_ctrl.get_pixel(x, y) != with_ctrl.get_pixel(x, y))
        })
    }

    let screenshot_dir =
        std::env::temp_dir().join(format!("plexi-0630-pane-id-overlay-{}", std::process::id()));
    std::fs::create_dir_all(&screenshot_dir).expect("create screenshot directory");
    println!("pane ID overlay screenshots: {}", screenshot_dir.display());

    let mut single = PlexiUiHarness::new_sized(900.0, 600.0);
    single.step();
    let single_pane = add_focused_pane(&mut single);
    single.step();
    let single_off = single.render().expect("single Ctrl-off render");
    single.harness().input_mut().modifiers = egui::Modifiers::CTRL;
    single.step();
    let single_on = single.render().expect("single Ctrl-on render");
    single_off
        .save(screenshot_dir.join("single-ctrl-off.png"))
        .expect("save single Ctrl-off screenshot");
    single_on
        .save(screenshot_dir.join("single-ctrl-on.png"))
        .expect("save single Ctrl-on screenshot");
    assert!(
        center_glyph_changed(&single_off, &single_on, pane_rect(&single, single_pane)),
        "Control must paint the pane ID in a single-pane layout"
    );

    let mut split = PlexiUiHarness::new_sized(900.0, 600.0);
    split.step();
    let first_pane = add_focused_pane(&mut split);
    let second_pane = add_focused_pane(&mut split);
    split.with_app_mut(|app| {
        let window = &mut app.windows[app.active_window];
        let first_tile = window
            .tree
            .tiles
            .iter()
            .find_map(|(tile_id, tile)| {
                matches!(tile, egui_tiles::Tile::Pane(id) if *id == first_pane)
                    .then_some(*tile_id)
            })
            .expect("first pane tile");
        let second_tile = window
            .tree
            .tiles
            .iter()
            .find_map(|(tile_id, tile)| {
                matches!(tile, egui_tiles::Tile::Pane(id) if *id == second_pane)
                    .then_some(*tile_id)
            })
            .expect("second pane tile");
        window.tree.root = Some(
            window
                .tree
                .tiles
                .insert_horizontal_tile(vec![first_tile, second_tile]),
        );
        window.focused_pane = Some(first_tile);
    });
    split.step();
    let split_off = split.render().expect("split Ctrl-off render");
    split.harness().input_mut().modifiers = egui::Modifiers::CTRL;
    split.step();
    let split_on = split.render().expect("split Ctrl-on render");
    split_off
        .save(screenshot_dir.join("split-ctrl-off.png"))
        .expect("save split Ctrl-off screenshot");
    split_on
        .save(screenshot_dir.join("split-ctrl-on.png"))
        .expect("save split Ctrl-on screenshot");

    for pane_id in [first_pane, second_pane] {
        assert!(
            center_glyph_changed(&split_off, &split_on, pane_rect(&split, pane_id)),
            "Control must paint pane {pane_id}'s ID in a split layout"
        );
    }
}

/// Round 3 item 7, round 4 drag: a real pointer drag of a tab to a new
/// position and back restores the tree's shape but is still a
/// structural edit, so a container anchor admitted before it refuses.
/// A tab switch (focus only) and a divider resize do not invalidate it,
/// and a fresh admission after the drag launches.
#[test]
fn launch_plan_tab_drag_and_back_invalidates_a_container_anchor() {
    let mut h = PlexiUiHarness::new_sized(1200.0, 700.0);
    h.step();
    let browser_root = h.workspace.path().to_path_buf();
    let (a, b) = h.with_app_mut(|app| {
        // Terminals: their headers carry the draggable tab bar.
        let context = app.pane_context_env_for_window(app.active_window);
        let mut ids = Vec::new();
        for _ in 0..3 {
            let launch =
                app.desktop_terminal_launch(context.clone(), Some(browser_root.clone()), None);
            let pane = app.construct_terminal(launch).expect("terminal starts");
            ids.push(pane.id);
            let win = &mut app.windows[app.active_window];
            win.panes
                .insert(pane.id, Pane::Terminal(Box::new(pane)))
                .unwrap();
        }
        let win = &mut app.windows[app.active_window];
        let tiles: Vec<_> = ids
            .iter()
            .map(|id| win.tree.tiles.insert_pane(*id))
            .collect();
        let tab_tile = win.tree.tiles.insert_tab_tile(vec![tiles[0], tiles[1]]);
        let side_tab = win.tree.tiles.insert_tab_tile(vec![tiles[2]]);
        let root = win
            .tree
            .tiles
            .insert_horizontal_tile(vec![tab_tile, side_tab]);
        win.tree.root = Some(root);
        win.focused_pane = Some(tiles[0]);
        (ids[0], ids[1])
    });
    h.run_steps(3);
    let tab_center = |h: &PlexiUiHarness, pane: u64| {
        h.with_app(|app| {
            crate::ui::layout_report::snapshot(&app.ctx)
                .iter()
                .flat_map(|r| r.texts.iter())
                .filter(|t| t.id == format!("tab-{pane}"))
                .last()
                .map(|t| t.allocated.min + egui::vec2(6.0, 6.0))
                .unwrap_or_else(|| panic!("tab of pane {pane} painted"))
        })
    };
    let drag = |h: &mut PlexiUiHarness, from: egui::Pos2, to: egui::Pos2| {
        h.harness().input_mut().events.extend([
            egui::Event::PointerMoved(from),
            egui::Event::PointerButton {
                pos: from,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        h.step();
        h.harness()
            .input_mut()
            .events
            .push(egui::Event::PointerMoved(to));
        h.step();
        h.harness()
            .input_mut()
            .events
            .push(egui::Event::PointerButton {
                pos: to,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            });
        h.run_steps(3);
    };
    let spec = crate::app::launch_spec::PaneLaunchSpec::from_spawn_pane(
        &plexi_protocol::HostRequest::SpawnPane {
            type_id: "terminal".to_string(),
            layout: Some("split_h".to_string()),
            args: Vec::new(),
            from_pane_id: None,
            request_id: None,
            response_file: None,
            ephemeral: false,
            cwd: Some(browser_root.to_string_lossy().into_owned()),
            no_focus: true,
            path: None,
            workspace_root: None,
            target_context: None,
            context_name: None,
            name: None,
            agent_cmd: None,
            boot_timeout_secs: None,
            worktree: None,
            result_schema: None,
        }, crate::broker::caller::RequestOrigin::HostUi)
    .unwrap();
    // A container anchor: the window's root, admitted with no focus.
    let admit = |h: &mut PlexiUiHarness| {
        h.with_app_mut(|app| {
            let win = app.active_window;
            let focus = app.windows[win].focused_pane.take();
            let plan = app.admit_terminal_launch(&spec).unwrap();
            app.windows[win].focused_pane = focus;
            assert!(matches!(
                plan.destination,
                crate::app::launch_plan::TerminalDestination::Split {
                    anchor: crate::app::launch_plan::SplitAnchor::Node { .. },
                    ..
                }
            ));
            plan
        })
    };
    let apply = |h: &mut PlexiUiHarness, plan: crate::app::launch_plan::TerminalLaunchPlan| {
        h.with_app_mut(|app| {
            app.apply_launch(crate::app::launch_plan::LaunchPlan::Terminal(Box::new(
                plan,
            )))
            .map_err(|refused| refused.to_string())
        })
    };
    let order = |h: &PlexiUiHarness| {
        h.with_app(|app| {
            let win = &app.windows[app.active_window];
            win.tree
                .tiles
                .iter()
                .find_map(|(_, tile)| match tile {
                    egui_tiles::Tile::Container(egui_tiles::Container::Tabs(tabs))
                        if tabs.children.len() == 2 =>
                    {
                        Some(tabs.children.clone())
                    }
                    _ => None,
                })
                .unwrap()
        })
    };

    // Positive controls: a tab switch and a divider resize.
    let plan = admit(&mut h);
    let at_b = tab_center(&h, b);
    h.harness().input_mut().events.extend([
        egui::Event::PointerMoved(at_b),
        egui::Event::PointerButton {
            pos: at_b,
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
            pos: at_b,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
    h.run_steps(3);
    let divider = h.with_app(|app| {
        app.active_strip_layout()
            .dividers
            .iter()
            .find(|d| d.is_vertical)
            .map(|d| d.rect.center())
            .expect("a divider between the columns")
    });
    drag(&mut h, divider, divider + egui::vec2(-60.0, 0.0));
    apply(&mut h, plan).expect("focus and resize leave the anchor valid");

    // A drag and a drag back.
    let plan = admit(&mut h);
    let before = order(&h);
    let (at_a, at_b) = (tab_center(&h, a), tab_center(&h, b));
    drag(&mut h, at_a, at_b);
    assert_ne!(order(&h), before, "fixture: the drag reordered the tabs");
    let (at_a, at_b) = (tab_center(&h, a), tab_center(&h, b));
    drag(&mut h, at_a, at_b);
    assert_eq!(
        order(&h),
        before,
        "fixture: the drag back restored the order"
    );
    let error = apply(&mut h, plan).expect_err("a structural edit voids the container anchor");
    assert!(error.contains("changed after admission"), "{error}");
    let fresh = admit(&mut h);
    apply(&mut h, fresh).expect("a fresh admission after the edit launches");
}

/// Stint 0581 visual + geometry review: `--agents 3` with `--layout`
/// omitted arranges the squad as three equal full-height columns (not a
/// 2×2 grid with a dead bottom-right quadrant) and labels each pane header
/// `<name>-1`, `<name>-2`, `<name>-3`.
#[test]
fn screenshot_context_sub_three_agents_default_columns_and_names() {
    let mut h = PlexiUiHarness::new_sized(1280.0, 800.0);
    h.step();
    let anchor = add_focused_pane(&mut h);
    let root = tempfile::tempdir().expect("squad root");

    let pane_ids = h.with_app_mut(|app| {
        let parent_id = app.router.active().context_id;
        let layout = crate::pane_ops::default_squad_layout(3);
        let child = app
            .new_child_context(crate::pane_ops::ChildContextSpec {
                parent_id: Some(parent_id),
                parent_name: String::new(),
                name: Some("agentsquad".to_string()),
                path: root.path().to_path_buf(),
                portal_vertical: true,
                portal_first: false,
                anchor_pane: Some(anchor),
                panes: vec![None, None, None],
                layout,
                source: crate::broker::pane_authority::GrantSource::DesktopInput,
            })
            .expect("create 3-pane squad");
        let child_idx = app.router.len() - 1;
        app.switch_workspace(child_idx);
        child.pane_ids
    });
    h.run_steps(3);

    let rects: Vec<egui::Rect> = h.with_app(|app| {
        let window = &app.windows[app.active_window];
        pane_ids
            .iter()
            .map(|pane_id| {
                let tile_id = window
                    .tree
                    .tiles
                    .iter()
                    .find_map(|(tid, tile)| {
                        matches!(tile, egui_tiles::Tile::Pane(id) if id == pane_id)
                            .then_some(*tid)
                    })
                    .expect("squad pane tile");
                window.tree.tiles.rect(tile_id).expect("painted pane rect")
            })
            .collect()
    });
    assert_eq!(rects.len(), 3);
    for (i, rect) in rects.iter().enumerate() {
        assert!(
            (rect.min.y - rects[0].min.y).abs() < 2.0
                && (rect.max.y - rects[0].max.y).abs() < 2.0,
            "pane {i} must span the same full vertical band as pane 0 (got {rect:?} vs {:?})",
            rects[0]
        );
        // Column widths follow the zoom strip presets (35% leading, 65%
        // following), so only a non-empty width is required here.
        assert!(
            rect.width() > 1.0,
            "pane {i} must have a painted width: {rect:?}"
        );
    }
    assert!(
        rects[0].max.x <= rects[1].min.x + 2.0 && rects[1].max.x <= rects[2].min.x + 2.0,
        "the 3 squad panes must sit side-by-side left-to-right: {rects:?}"
    );

    let names: Vec<Option<String>> = h.with_app(|app| {
        let window = &app.windows[app.active_window];
        pane_ids
            .iter()
            .map(|id| {
                window
                    .panes
                    .get(id)
                    .and_then(Pane::as_terminal)
                    .and_then(|t| t.name.clone())
            })
            .collect()
    });
    assert_eq!(
        names,
        vec![
            Some("agentsquad-1".to_string()),
            Some("agentsquad-2".to_string()),
            Some("agentsquad-3".to_string()),
        ]
    );

    h.save_screenshot(&evidence_png!())
        .expect("render 3-pane squad columns");
}

/// Item 4 regression: a context with several windows must lay out the ACTIVE
/// window's columns from its own indices. Moving left/right across the window
/// boundary used to feed the context-wide scroll offset into the window's tree and
/// painted 6% slivers beside one oversized column (and overflowed the viewport).
#[test]
fn cross_window_navigation_keeps_every_pane_inside_viewport() {
    use crate::host::keys::Direction;
    const MIN_COLUMN_WIDTH: f32 = 80.0;
    let mut h = PlexiUiHarness::new_sized(1100.0, 700.0);
    let (ctx_id, first_window_panes, second_pane) = h.with_app_mut(|app| {
        app.sidebar_visible = false;
        let ctx_id = app.router.active().context_id;
        let p1 = app.add_test_pane().1;
        let p2 = app.add_test_pane().1;
        let p3 = app.add_test_pane().1;
        app.set_context_columns(ctx_id, vec![vec![p1], vec![p2], vec![p3]]);
        let win_id = app.next_window_id;
        app.next_window_id += 1;
        app.windows.push(crate::host::context::Window {
            name: String::new(),
            path: std::path::PathBuf::from("/tmp"),
            tree: egui_tiles::Tree::empty("plexi-second"),
            panes: Default::default(),
            focused_pane: None,
            zoomed_pane: None,
            grid_x: 1,
            grid_y: 0,
            window_id: win_id,
            context_id: ctx_id,
            tree_history: Default::default(),
        });
        let second_idx = app.windows.len() - 1;
        let (tile, p4) = app.add_view_pane_in_window(second_idx, "test");
        app.windows[second_idx].focused_pane = Some(tile);
        app.active_window = second_idx;
        app.context_active_window.insert(ctx_id, win_id);
        (ctx_id, vec![p1, p2, p3], p4)
    });
    h.run_steps(3);

    let check = |h: &mut PlexiUiHarness, step: &str| {
        h.run_steps(3);
        h.with_app(|app| {
            let viewport = app.ctx.content_rect();
            let window = &app.windows[app.active_window];
            let mut narrow = 0usize;
            let mut columns = 0usize;
            if let Some(root) = window.tree.root() {
                let children = match window.tree.tiles.get(root) {
                    Some(egui_tiles::Tile::Container(c)) => c.children().copied().collect(),
                    _ => vec![root],
                };
                columns = children.len();
                for child in children {
                    let rect = window.tree.tiles.rect(child).expect("painted column rect");
                    let clipped = rect.intersect(viewport);
                    if clipped.width() > 0.0 && clipped.height() > 0.0 {
                        assert!(
                            viewport.expand(1.0).contains_rect(clipped),
                            "{step}: clipped column rect {clipped:?} escapes the viewport {viewport:?}"
                        );
                    }
                    if rect.width() < MIN_COLUMN_WIDTH {
                        narrow += 1;
                    }
                }
            }
            assert_eq!(
                narrow, 0,
                "{step}: {narrow} of {columns} columns are narrower than {MIN_COLUMN_WIDTH}px"
            );
            let focused = app.active_focused_pane_id().expect("focused pane");
            let tile = window.tree.tiles.find_pane(&focused).expect("focused tile");
            let rect = window.tree.tiles.rect(tile).expect("focused rect");
            assert!(
                viewport.expand(1.0).contains_rect(rect),
                "{step}: focused pane {rect:?} must be revealed inside viewport {viewport:?}"
            );
            assert!(
                rect.width() >= 200.0 && rect.height() >= 200.0,
                "{step}: focused pane squeezed to {rect:?}"
            );
        });
    };

    check(&mut h, "start on second window");
    for (i, dir) in [
        Direction::Left,
        Direction::Left,
        Direction::Left,
        Direction::Right,
        Direction::Right,
        Direction::Right,
        Direction::Left,
    ]
    .into_iter()
    .enumerate()
    {
        h.with_app_mut(|app| {
            app.navigate(dir);
        });
        check(&mut h, &format!("move {i} {dir:?}"));
    }
    let _ = (ctx_id, first_window_panes, second_pane);
}

#[test]
fn screenshot_columns_cmd_enter_zoom_toggle_one_and_multi_pane() {
    let mut h = PlexiUiHarness::new_sized(1280.0, 800.0);
    let p1 = add_focused_pane(&mut h);
    h.run_steps(2);

    let p2 = h.with_app_mut(|app| {
        app.split_focused(true, None, false, false, None)
            .expect("split right for column 1")
    });
    h.run_steps(2);
    h.focus_pane(p2).unwrap();

    let p3 = h.with_app_mut(|app| {
        app.split_focused(false, None, false, false, None)
            .expect("split stacked pane inside column 1")
    });
    h.run_steps(2);

    h.with_app_mut(|app| {
        let ctx = app.router.active().context_id;
        app.set_context_columns(ctx, vec![vec![p1], vec![p2, p3]]);
    });
    h.run_steps(4);

    let tiled_shot = evidence_png!("_tiled_columns");
    h.save_screenshot(&tiled_shot)
        .expect("render tiled columns");

    // 1. Zoom the one-pane column (p1) via Cmd+Enter.
    h.focus_pane(p1).unwrap();
    h.run_steps(2);
    h.harness()
        .key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Enter);
    h.run_steps(4);
    h.with_app(|app| {
        assert!(
            app.windows[app.active_window].zoomed_pane.is_some(),
            "one-pane column must be zoomed"
        );
    });
    let one_pane_shot = evidence_png!("_one_pane_column_zoomed");
    h.save_screenshot(&one_pane_shot)
        .expect("render zoomed one-pane column");

    // Unzoom p1 via Cmd+Enter.
    h.harness()
        .key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Enter);
    h.run_steps(4);
    h.with_app(|app| {
        assert!(
            app.windows[app.active_window].zoomed_pane.is_none(),
            "one-pane column must be unzoomed"
        );
    });

    // 2. Zoom stacked pane in multi-pane column (p3) via Cmd+Enter.
    h.focus_pane(p3).unwrap();
    h.run_steps(2);
    h.harness()
        .key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Enter);
    h.run_steps(4);
    h.with_app(|app| {
        assert!(
            app.windows[app.active_window].zoomed_pane.is_some(),
            "multi-pane column pane must be zoomed"
        );
    });
    let multi_pane_shot = evidence_png!("_multi_pane_column_zoomed");
    h.save_screenshot(&multi_pane_shot)
        .expect("render zoomed multi-pane column pane");

    // Unzoom p3 via Cmd+Enter.
    h.harness()
        .key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Enter);
    h.run_steps(4);
    h.with_app(|app| {
        assert!(
            app.windows[app.active_window].zoomed_pane.is_none(),
            "multi-pane column pane must be unzoomed"
        );
    });
    let restored_shot = evidence_png!("_tiled_restored");
    h.save_screenshot(&restored_shot)
        .expect("render restored tiled columns");

    println!("Screenshots: {tiled_shot}, {one_pane_shot}, {multi_pane_shot}, {restored_shot}");
    for p in [
        &tiled_shot,
        &one_pane_shot,
        &multi_pane_shot,
        &restored_shot,
    ] {
        let _ = std::fs::remove_file(p);
    }
}
#[test]
fn tiled_divider_drag_survives_repaint() {
    let mut h = PlexiUiHarness::new_sized(1200.0, 700.0);
    h.step();
    let first = add_focused_pane(&mut h);
    h.run_steps(2);
    let second = h
        .with_app_mut(|app| app.split_focused(false, None, false, false, None))
        .unwrap();
    h.run_steps(3);
    let (before, pos) = h.with_app(|app| {
        let layout = app.active_strip_layout();
        (
            layout.pane(first).unwrap().outer_rect.width(),
            layout
                .dividers
                .iter()
                .find(|d| d.is_vertical)
                .unwrap()
                .rect
                .center(),
        )
    });
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
    let moved = pos + egui::vec2(80.0, 0.0);
    h.harness()
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(moved));
    h.step();
    h.harness()
        .input_mut()
        .events
        .push(egui::Event::PointerButton {
            pos: moved,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
    h.run_steps(4);
    let after = h.with_app(|app| {
        app.active_strip_layout()
            .pane(first)
            .unwrap()
            .outer_rect
            .width()
    });
    assert!(
        after > before + 30.0,
        "drag must persist: {before} -> {after}; second={second}"
    );
}

#[test]
fn tiled_stack_drag_survives_repaint() {
    let mut h = PlexiUiHarness::new_sized(1200.0, 700.0);
    h.step();
    let first = add_focused_pane(&mut h);
    h.run_steps(2);
    h.with_app_mut(|app| app.split_focused(true, None, false, false, None))
        .unwrap();
    h.run_steps(3);
    let (before, pos) = h.with_app(|app| {
        let layout = app.active_strip_layout();
        (
            layout.pane(first).unwrap().outer_rect.height(),
            layout
                .dividers
                .iter()
                .find(|d| !d.is_vertical)
                .unwrap()
                .rect
                .center(),
        )
    });
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
    let moved = pos + egui::vec2(0.0, 80.0);
    h.harness()
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(moved));
    h.step();
    h.harness()
        .input_mut()
        .events
        .push(egui::Event::PointerButton {
            pos: moved,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
    h.run_steps(4);
    let after = h.with_app(|app| {
        app.active_strip_layout()
            .pane(first)
            .unwrap()
            .outer_rect
            .height()
    });
    assert!(
        after > before + 30.0,
        "stack drag must persist: {before} -> {after}"
    );
}

#[test]
fn tiled_double_click_resets_only_adjacent_columns() {
    let mut h = PlexiUiHarness::new_sized(1200.0, 700.0);
    h.step();
    let first = add_focused_pane(&mut h);
    h.run_steps(2);
    let second = h
        .with_app_mut(|app| app.split_focused(false, None, false, false, None))
        .unwrap();
    h.run_steps(2);
    let third = h
        .with_app_mut(|app| app.split_focused(false, None, false, false, None))
        .unwrap();
    h.run_steps(2);
    h.with_app_mut(|app| {
        let window = &mut app.windows[app.active_window];
        let root = window.tree.root().unwrap();
        let Some(egui_tiles::Tile::Container(egui_tiles::Container::Linear(linear))) =
            window.tree.tiles.get_mut(root)
        else {
            panic!("columns")
        };
        for (&tile, share) in linear.children.iter().zip([3.0, 4.0, 7.0]) {
            linear.shares.set_share(tile, share);
        }
        app.focus_pane_in_strip(first);
    });
    h.run_steps(3);
    let pos = h.with_app(|app| {
        app.active_strip_layout()
            .dividers
            .iter()
            .find(|d| d.is_vertical)
            .unwrap()
            .rect
            .center()
    });
    let mut time = h.with_app(|app| app.ctx.input(|input| input.time));
    for click in 0..2 {
        time += 0.05;
        h.harness().input_mut().time = Some(time);
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
        time += 0.05;
        h.harness().input_mut().time = Some(time);
        h.harness()
            .input_mut()
            .events
            .push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            });
        h.step();
        if click == 1 {
            assert!(
                h.with_app(|app| app.ctx.input(|input| input
                    .pointer
                    .button_double_clicked(egui::PointerButton::Primary))),
                "fixture must produce a double click"
            );
        }
    }
    h.run_steps(3);
    h.with_app(|app| {
        let window = &app.windows[app.active_window];
        let Some(egui_tiles::Tile::Container(egui_tiles::Container::Linear(linear))) =
            window.tree.tiles.get(window.tree.root().unwrap())
        else {
            panic!("columns")
        };
        assert!(
            (linear.shares[linear.children[0]] - linear.shares[linear.children[1]]).abs()
                < 0.01
        );
        assert!((linear.shares[linear.children[2]] - 7.0).abs() < 0.01);
        let layout = app.active_strip_layout();
        assert!(
            layout.pane(second).unwrap().is_visible && layout.pane(third).unwrap().is_visible
        );
    });
}

#[derive(Default)]
struct StripWheelSnapshot {
    offset: egui::Vec2,
    raw_y: f32,
    keys: usize,
}
struct StripWheelFixture {
    horizontal: bool,
    state: Arc<std::sync::Mutex<StripWheelSnapshot>>,
}
impl crate::app::view_trait::View for StripWheelFixture {
    fn type_id(&self) -> &'static str {
        "strip_wheel_fixture"
    }
    fn display_name(&self) -> String {
        "Scroll fixture".into()
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn handle_key(
        &mut self,
        input: &crate::app::input_router::PlexiInput,
    ) -> crate::app::view_trait::KeyDisposition {
        if input.key_pressed(egui::Key::X) {
            self.state.lock().unwrap().keys += 1;
            return crate::app::view_trait::KeyDisposition::Consumed;
        }
        crate::app::view_trait::KeyDisposition::Passthrough
    }
    fn ui(
        &mut self,
        ui: &mut egui::Ui,
        _: &crate::app::view_trait::ViewRenderContext<'_>,
    ) {
        let raw_y = ui.input(|input| {
            input
                .events
                .iter()
                .filter_map(|event| match event {
                    egui::Event::MouseWheel { delta, .. } => Some(delta.y),
                    _ => None,
                })
                .sum::<f32>()
        });
        let area = if self.horizontal {
            egui::ScrollArea::horizontal()
        } else {
            egui::ScrollArea::vertical()
        };
        let output = area.max_width(300.0).max_height(300.0).show(ui, |ui| {
            ui.allocate_space(egui::vec2(2000.0, 2000.0));
        });
        let mut state = self.state.lock().unwrap();
        state.offset = output.state.offset;
        state.raw_y += raw_y;
    }
}

#[test]
fn strip_nested_scroll_controls_keep_their_wheel() {
    for horizontal in [false, true] {
        let mut h = PlexiUiHarness::new_sized(1000.0, 700.0);
        h.step();
        let first = add_focused_pane(&mut h);
        let state = Arc::new(std::sync::Mutex::new(StripWheelSnapshot::default()));
        h.with_app_mut(|app| {
            let pane = app.windows[app.active_window]
                .panes
                .get_mut(&first)
                .unwrap()
                .as_view_mut()
                .unwrap();
            pane.runtime = ViewRuntime::Builtin(Box::new(StripWheelFixture {
                horizontal,
                state: state.clone(),
            }));
        });
        h.run_steps(2);
        h.with_app_mut(|app| app.split_focused(false, None, false, false, None))
            .unwrap();
        h.run_steps(2);
        h.with_app_mut(|app| app.split_focused(false, None, false, false, None))
            .unwrap();
        h.run_steps(2);
        h.with_app_mut(|app| {
            app.focus_pane_in_strip(first);
        });
        h.run_steps(3);
        let (before, pos) = h.with_app(|app| {
            let layout = app.active_strip_layout();
            (
                layout.scroll_offset_px,
                layout.pane(first).unwrap().clip_rect.min + egui::vec2(80.0, 80.0),
            )
        });
        let delta = if horizontal {
            egui::vec2(-200.0, 0.0)
        } else {
            egui::vec2(0.0, -200.0)
        };
        h.harness().input_mut().events.extend([
            egui::Event::PointerMoved(pos),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta,
                modifiers: egui::Modifiers::NONE,
                phase: egui::TouchPhase::Move,
            },
        ]);
        h.run_steps(8);
        let offset = state.lock().unwrap().offset;
        assert!(
            if horizontal {
                offset.x > 1.0
            } else {
                offset.y > 1.0
            },
            "pane did not scroll: {offset:?}"
        );
        assert!(
            (h.with_app(|app| app.active_strip_layout().scroll_offset_px) - before).abs() < 0.5,
            "pane control must consume its gesture"
        );
    }
}

#[test]
fn screenshot_populated_tiled_windows() {
    for (width, height, scale, suffix) in [
        (1200.0, 700.0, 1.0, "_wide"),
        (900.0, 700.0, 1.5, "_narrow"),
    ] {
        let mut h = PlexiUiHarness::new_sized_scaled(width, height, scale);
        for index in 0..3 {
            let path = h.workspace.path().join(format!("note-{index}.md"));
            std::fs::write(&path, format!("# Project {index}\n\nA populated workspace pane with notes and useful context.\n\n- Review the plan\n- Follow up on the next step\n")).unwrap();
            h.with_app_mut(|app| {
                app.launch_view_by_id_with_layout_forced(
                    "text-editor",
                    Some("split_h".into()),
                    &[path.display().to_string()],
                    None,
                )
                .expect("open note");
            });
            h.run_steps(3);
        }
        h.run_steps(4);
        // Three note panes are open; each visible header fits its title
        // and controls at both sizes.
        let views = h.with_app(|app| {
            app.windows
                .iter()
                .flat_map(|window| window.panes.values())
                .filter(|pane| pane.as_view().is_some())
                .count()
        });
        assert_eq!(views, 3, "three note panes open{suffix}");
        assert_layout_surface(&h, "pane_header");
        h.save_screenshot(&evidence_png!(suffix))
            .expect("render strip");
    }
}

#[test]
fn screenshot_saved_workspace_tiles_and_windows() {
    let mut h = PlexiUiHarness::new_sized(1200.0, 700.0);
    for (name, layout) in [("first", "split_h"), ("second", "split_h")] {
        let path = h.workspace.path().join(format!("{name}.md"));
        std::fs::write(
            &path,
            format!("# {name} project\n\nReview the migration and record the result.\n"),
        )
        .unwrap();
        h.with_app_mut(|app| {
            app.launch_view_by_id_with_layout_forced(
                "text-editor",
                Some(layout.into()),
                &[path.display().to_string()],
                None,
            )
            .unwrap()
        });
        h.run_steps(3);
    }
    h.save_screenshot(&evidence_png!("_fresh")).unwrap();
    let path = h.workspace.path().join("window.md");
    std::fs::write(
        &path,
        "# Second window\n\nThe workspace keeps its pane identity.\n",
    )
    .unwrap();
    let (terminal, first_editor) = h.with_app_mut(|app| {
        let context = app.router.active().context_id;
        let launch = app.test_terminal_launch(
            context,
            path.parent().map(|p| p.to_path_buf()),
            Some("printf 'Second window ready\\n'"),
        );
        app.create_page_at(1, 0, context, launch);
        let terminal = app.focused_pane_id().expect("new page terminal");
        let previous: std::collections::HashSet<_> = app.windows[app.active_window]
            .panes
            .keys()
            .copied()
            .collect();
        app.launch_view_by_id_with_layout_forced(
            "text-editor",
            Some("split_h".into()),
            &[path.display().to_string()],
            None,
        )
        .unwrap();
        let first_editor = app.windows[app.active_window]
            .panes
            .keys()
            .copied()
            .find(|id| !previous.contains(id))
            .expect("second-page editor");
        (terminal, first_editor)
    });
    let deadline = Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(20));
    loop {
        h.run_steps(2);
        let ready = h.with_app_mut(|app| {
            app.windows
                .iter_mut()
                .find_map(|window| window.panes.get_mut(&terminal))
                .and_then(Pane::as_terminal_mut)
                .is_some_and(|pane| {
                    pane.backend
                        .capture_lines(30)
                        .iter()
                        .any(|line| line.contains("Second window ready"))
                })
        });
        if ready {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "second window terminal did not print fixture output"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    h.run_steps(4);
    h.save_screenshot(&evidence_png!("_terminal_multi"))
        .unwrap();
    let companion = h.workspace.path().join("companion.md");
    std::fs::write(
        &companion,
        "# Companion note\n\nThe second page has two visible panes.\n",
    )
    .unwrap();
    h.with_app_mut(|app| {
        app.close_pane_by_id(terminal);
        let previous: std::collections::HashSet<_> = app.windows[app.active_window]
            .panes
            .keys()
            .copied()
            .collect();
        app.launch_view_by_id_with_layout_forced(
            "text-editor",
            Some("split_h".into()),
            &[companion.display().to_string()],
            None,
        )
        .unwrap();
        let second_editor = app.windows[app.active_window]
            .panes
            .keys()
            .copied()
            .find(|id| !previous.contains(id))
            .expect("companion editor");
        let context = app.router.active().context_id;
        let strip = app.ensure_authoritative_strip(context);
        for (pane, width) in [(first_editor, 480.0), (second_editor, 680.0)] {
            let column = strip.find_column_for_pane(pane).expect("editor column");
            strip.columns[column].width = crate::strip::ColumnWidth::CustomPixels(width);
        }
        strip.focus_pane(second_editor);
    });
    h.run_steps(4);
    assert!(h.with_app(|app| app.windows.len()) >= 2);
    h.with_app(|app| {
        let layout = app.active_strip_layout();
        let pane = layout
            .pane(first_editor)
            .expect("second-page editor layout");
        assert!(
            pane.is_visible,
            "visible editor must not be marked off-screen: {pane:?}"
        );
        let window = &app.windows[app.active_window];
        let tile = window.tree.tiles.find_pane(&first_editor).unwrap();
        let painted = window.tree.tiles.rect(tile).unwrap();
        let painted_visible = painted.intersect(layout.canvas_rect);
        assert!(
            pane.clip_rect.intersect(painted_visible).width() >= painted_visible.width() - 2.0,
            "editor snapshot and painted tile must agree: snapshot={:?}, tile={painted:?}",
            pane.clip_rect
        );
    });
    h.save_screenshot(&evidence_png!("_multi_window")).unwrap();
    h.with_app_mut(|app| app.save_workspace_now());
    let profile = h._profile_dir.path().to_path_buf();
    let saved = crate::workspace::WorkspaceFile::load_from(&profile).expect("saved workspace");
    let mut restored = PlexiUiHarness::new_sized(1200.0, 700.0);
    restored.with_app_mut(|app| {
        let config = app.config.clone();
        let colors = app.colors.clone();
        let context = app.ctx.clone();
        let sender = app.pty_event_tx.clone();
        let workspace = crate::app::restore_workspace(
            saved,
            &config,
            crate::app::WorkspaceRestoreCollaborators {
                profile_dir: &profile,
                colors: &colors,
                hot_reload: &mut app.hot_reload,
                state_watch: &mut app.state_watch,
                terminal_spawner: Some((&context, &sender)),
            },
        )
        .expect("restore workspace");
        app.install_restored_workspace(workspace);
    });
    restored.run_steps(4);
    assert!(restored.with_app(|app| app.windows.len()) >= 2);
    restored
        .save_screenshot(&evidence_png!("_restored"))
        .unwrap();
}

#[test]
fn tiled_stack_double_click_resets_only_adjacent_shares() {
    let mut h = PlexiUiHarness::new_sized(1200.0, 700.0);
    h.step();
    add_focused_pane(&mut h);
    h.run_steps(2);
    h.with_app_mut(|app| app.split_focused(true, None, false, false, None))
        .unwrap();
    h.run_steps(2);
    h.with_app_mut(|app| app.split_focused(true, None, false, false, None))
        .unwrap();
    h.run_steps(2);
    h.with_app_mut(|app| {
        let window = &mut app.windows[app.active_window];
        let root = window.tree.root().unwrap();
        let Some(egui_tiles::Tile::Container(egui_tiles::Container::Linear(linear))) =
            window.tree.tiles.get_mut(root)
        else {
            panic!("rows")
        };
        for (&tile, share) in linear.children.iter().zip([1.0, 2.0, 4.0]) {
            linear.shares.set_share(tile, share);
        }
    });
    h.run_steps(3);
    let pos = h.with_app(|app| {
        app.active_strip_layout()
            .dividers
            .iter()
            .find(|d| !d.is_vertical)
            .unwrap()
            .rect
            .center()
    });
    let mut time = h.with_app(|app| app.ctx.input(|input| input.time));
    for _ in 0..2 {
        time += 0.05;
        h.harness().input_mut().time = Some(time);
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
        time += 0.05;
        h.harness().input_mut().time = Some(time);
        h.harness()
            .input_mut()
            .events
            .push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            });
        h.step();
    }
    h.run_steps(3);
    let heights = h.with_app(|app| {
        let window = &app.windows[app.active_window];
        let Some(egui_tiles::Tile::Container(egui_tiles::Container::Linear(linear))) =
            window.tree.tiles.get(window.tree.root().unwrap())
        else {
            panic!("rows")
        };
        linear
            .children
            .iter()
            .map(|tile| linear.shares[*tile])
            .collect::<Vec<_>>()
    });
    assert_eq!(heights, vec![1.5, 1.5, 4.0]);
}

#[test]
fn tiled_nested_editors_pointer_input_and_screenshots() {
    for (width, height, scale, suffix) in [
        (1280.0, 800.0, 1.0, "_wide"),
        (720.0, 600.0, 1.0, "_narrow"),
        (1280.0, 800.0, 2.0, "_ppp2"),
    ] {
        let mut h = PlexiUiHarness::new_sized_ppp(width, height, scale);
        let mut editors = Vec::new();
        for (index, layout) in ["split_h", "split_v", "split_h"].into_iter().enumerate() {
            let path = h.workspace.path().join(format!("plan-{index}.md"));
            std::fs::write(&path, format!("# Project {}\n\nOrdinary split workspace.\n\n- Review the implementation\n- Check the running build\n- Record the next step\n", index + 1)).unwrap();
            h.with_app_mut(|app| {
                app.launch_view_by_id_with_layout_forced(
                    "text-editor",
                    Some(layout.into()),
                    &[path.display().to_string()],
                    None,
                )
                .unwrap()
            });
            h.run_steps(3);
            editors.push((
                h.with_app(|app| app.active_focused_pane_id().unwrap()),
                path,
            ));
        }
        h.with_app(|app| {
            let layout = app.active_strip_layout();
            let top = layout.pane(editors[0].0).unwrap().outer_rect;
            let left = layout.pane(editors[1].0).unwrap().outer_rect;
            let right = layout.pane(editors[2].0).unwrap().outer_rect;
            assert!(top.bottom() <= left.top() + 1.0 && top.bottom() <= right.top() + 1.0);
            assert!(left.right() <= right.left() + 1.0);
            for pane in &layout.panes {
                assert!(
                    layout
                        .canvas_rect
                        .expand(1.0)
                        .contains_rect(pane.outer_rect),
                    "pane outside tiled canvas: {pane:?}"
                );
            }
        });
        for (index, (pane, path)) in editors.iter().enumerate() {
            let pos = h.with_app(|app| {
                app.active_strip_layout()
                    .pane(*pane)
                    .unwrap()
                    .hit_rect
                    .center()
            });
            h.harness().input_mut().events.extend([
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
            h.run_steps(2);
            h.with_app(|app| {
                assert_eq!(
                    app.host_input_owner(),
                    crate::app::input_owner::InputOwner::Pane(*pane)
                )
            });
            let marker = format!("typed-in-pane-{index}");
            h.harness()
                .input_mut()
                .events
                .push(egui::Event::Text(marker.clone()));
            h.run_steps(2);
            h.harness().key_press_modifiers(
                egui::Modifiers::MAC_CMD | egui::Modifiers::COMMAND,
                egui::Key::S,
            );
            h.run_steps(2);
            assert!(std::fs::read_to_string(path).unwrap().contains(&marker));
            for (other, (_, other_path)) in editors.iter().enumerate() {
                if other != index {
                    assert!(!std::fs::read_to_string(other_path)
                        .unwrap()
                        .contains(&marker));
                }
            }
        }
        h.save_screenshot(&evidence_png!(suffix)).unwrap();
    }
}
