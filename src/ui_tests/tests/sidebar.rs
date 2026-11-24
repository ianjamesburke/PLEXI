use super::*;

#[test]
fn parked_sidebar_dropdown_uses_shared_list_header() {
    let mut h = PlexiUiHarness::new_sized(900.0, 620.0);
    h.with_app_mut(|app| {
        app.sidebar_visible = true;
        app.parked_section_expanded = false;
        let mk = |id: u64, name: &str| crate::host::context::Context {
            name: name.to_string().into(),
            root: std::env::temp_dir().join(name),
            description: None,
            context_id: id,
            parent_id: None,
            depth: 0,
            parked: true,
        };
        app.router.push(mk(70_001, "parked-alpha"));
        app.router.push(mk(70_002, "parked-beta"));
    });
    h.run_steps(2);
    h.with_app(|app| {
        assert_eq!(
            app.router.iter().filter(|ctx| ctx.parked).count(),
            2,
            "test setup should render a parked dropdown"
        );
        assert!(!app.parked_section_expanded);
    });
    h.save_screenshot(&evidence_png!()).expect("render failed");

    // Needs-you banners and context rows can move the header. Click its
    // rendered hit target so those layout changes cannot miss the control.
    let click = h
        .harness()
        .ctx
        .read_response(egui::Id::new("parked_divider"))
        .expect("parked dropdown header should be rendered")
        .rect
        .center();
    h.harness()
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(click));
    h.harness()
        .input_mut()
        .events
        .push(egui::Event::PointerButton {
            pos: click,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        });
    h.harness()
        .input_mut()
        .events
        .push(egui::Event::PointerButton {
            pos: click,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
    h.run_steps(2);

    h.with_app(|app| {
        assert!(
            app.parked_section_expanded,
            "shared dropdown header should toggle the parked list"
        );
    });
    h.save_screenshot(&evidence_png!()).expect("render failed");
}

/// While a context is being dragged, the Parked header renders even with
/// zero parked contexts so the drag has a drop target. Smoke test: set up
/// an in-progress drag and confirm the sidebar renders without panicking.
#[test]
fn dragging_context_shows_parked_drop_target() {
    let mut h = PlexiUiHarness::new_sized(900.0, 620.0);
    h.with_app_mut(|app| {
        app.sidebar_visible = true;
        app.parked_section_expanded = false;
        // Add a second active context so there is a drag source + a list.
        app.router.push(crate::host::context::Context {
            name: "second".to_string().into(),
            root: std::env::temp_dir().join("second"),
            description: None,
            context_id: 71_001,
            parent_id: None,
            depth: 0,
            parked: false,
        });
        // Simulate an in-progress drag of the first context.
        app.drag_context = Some(0);
    });
    // Hover low in the sidebar where the (empty) Parked header renders.
    let pos = egui::pos2(28.0, 150.0);
    h.harness()
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(pos));
    h.run_steps(2);
    h.with_app(|app| {
        assert_eq!(app.router.iter().filter(|c| c.parked).count(), 0);
        assert_eq!(app.drag_context, Some(0));
    });
    h.save_screenshot(&evidence_png!()).expect("render failed");
}

#[test]
fn screenshot_sidebar_ancestry_overflow_scale_matrix() {
    for (width, scale) in [(520.0, 1.0), (1100.0, 1.0), (1100.0, 1.5)] {
        let mut h = PlexiUiHarness::new_sized_scaled(width, 800.0, scale);
        let root_path = h.workspace_root().to_string_lossy().to_string();
        let root = h.with_app(|app| app.router.active().context_id);
        fill_window_with_panes(&mut h, 0, 8);
        let second = push_window_for_context(&mut h, root, 1, &root_path);
        fill_window_with_panes(&mut h, second, 6);
        let (child_idx, child, child_window) = push_sidebar_context(
            &mut h,
            "Research child with a long descriptive name",
            &root_path,
        );
        fill_window_with_panes(&mut h, child_window, 12);
        let (leaf_idx, leaf, leaf_window) = push_sidebar_context(
            &mut h,
            "Current grandchild with a long descriptive name",
            &root_path,
        );
        fill_window_with_panes(&mut h, leaf_window, 12);
        let (_, _, other_window) =
            push_sidebar_context(&mut h, "Unrelated root stays compact", &root_path);
        fill_window_with_panes(&mut h, other_window, 2);
        h.with_app_mut(|app| {
            app.sidebar_visible = true;
            app.router.get_mut(0).name = "Root workspace".to_string().into();
            app.router.get_mut(child_idx).parent_id = Some(root);
            app.router.get_mut(leaf_idx).parent_id = Some(child);
            for (window, target) in [(second, child), (child_window, leaf)] {
                let win = &mut app.windows[window];
                let panes = crate::spatial::tiling::collect_pane_ids_spatial(
                    &win.tree.tiles,
                    win.tree.root().unwrap(),
                );
                let pane_id = *panes.last().unwrap();
                win.take_pane(pane_id);
                win.panes
                    .insert(
                        pane_id,
                        Pane::Portal(Box::new(crate::host::pane::PortalPane {
                            pane_id,
                            target_context_id: target,
                            context_state: None,
                        })),
                    )
                    .unwrap();
            }
            for win in &mut app.windows {
                let panes = crate::spatial::tiling::collect_pane_ids_spatial(
                    &win.tree.tiles,
                    win.tree.root().unwrap(),
                );
                for (index, pane_id) in panes.iter().enumerate() {
                    if let Some(Pane::View(pane)) = win.panes.get_mut(pane_id) {
                        let state = match index % 5 {
                            0 => Some(crate::protocol::AgentState::Working),
                            1 => Some(crate::protocol::AgentState::NeedsYou),
                            2 => Some(crate::protocol::AgentState::Blocked),
                            _ => None,
                        };
                        pane.agent = state.map(|state| crate::protocol::PaneAgentState {
                            pane_id: *pane_id,
                            state,
                            agent: "fixture".into(),
                            detail: None,
                            session_id: None,
                        });
                    }
                }
                win.focused_pane = panes.last().and_then(|id| win.tree.tiles.find_pane(id));
            }
            app.switch_workspace(leaf_idx);
            app.set_ui_scale(scale);
        });
        h.run_steps(3);
        h.with_app(|app| assert!((app.ctx.zoom_factor() - scale).abs() < 0.01));
        h.save_screenshot(&evidence_png!(format!("_{width}_{scale}")))
            .unwrap();
        assert!(h.host_has_label("Current grandchild with a long descriptive name"));
    }
}

#[test]
fn sidebar_ancestor_pointer_restores_context_and_folds_path() {
    let mut h = PlexiUiHarness::new_sized(900.0, 620.0);
    let cwd = h.workspace_root().to_path_buf();
    let open = |h: &mut PlexiUiHarness| {
        h.open_target(HarnessOpenTarget::Builtin {
            id: "file_browser",
            cwd: &cwd,
            args: &[],
        })
        .unwrap()
    };
    let pane = open(&mut h);
    open(&mut h);
    let (root, child) = h.with_app_mut(|app| {
        app.sidebar_visible = true;
        app.router.get_mut(0).name = "Root path".to_string().into();
        let root = app.router.active().context_id;
        app.push_pane_to_subcontext(Some("Child path".into()), Some(pane));
        let index = app.router.position(|c| c.parent_id == Some(root)).unwrap();
        let child = app.router.get(index).context_id;
        app.switch_workspace(index);
        (root, child)
    });
    open(&mut h);
    let grandchild = h.with_app_mut(|app| {
        app.push_pane_to_subcontext(Some("Grandchild path".into()), Some(pane));
        let index = app.router.position(|c| c.parent_id == Some(child)).unwrap();
        let id = app.router.get(index).context_id;
        app.switch_workspace(index);
        id
    });
    h.run_steps(3);
    h.with_app_mut(|app| {
        app.router.get_mut(0).parked = true;
        app.parked_section_expanded = false;
    });
    h.run_steps(3);
    assert!(
        h.host_has_label("Child path"),
        "active parked ancestry remains reachable"
    );
    h.with_app_mut(|app| app.router.get_mut(0).parked = false);
    h.run_steps(3);
    h.save_screenshot(&evidence_png!("_nested")).unwrap();
    h.harness().get_by_label("Child path").click();
    h.run_steps(3);
    h.with_app(|app| assert_eq!(app.router.active().context_id, child));
    assert!(!h.host_has_label("Grandchild path"));
    h.with_app_mut(|app| {
        let index = app.router.position(|c| c.context_id == grandchild).unwrap();
        app.switch_workspace(index);
    });
    h.run_steps(3);
    h.harness()
        .get_all_by_label("Root path")
        .min_by(|a, b| a.rect().height().total_cmp(&b.rect().height()))
        .unwrap()
        .click();
    h.run_steps(3);
    h.with_app(|app| assert_eq!(app.router.active().context_id, root));
    assert!(!h.host_has_label("Child path"));
    h.save_screenshot(&evidence_png!("_root")).unwrap();
}

/// Stint 0241: the sidebar is master-context navigation only. Every slot is
/// a numbered, keyboard-addressable top-level destination, so a subcontext
/// is never rendered and never consumes a number — it is reached from
/// inside its parent through the Portal tile instead. Removing the row must
/// not remove the *signal*: the child's state still rolls up to the parent.
#[test]
fn sidebar_renders_and_numbers_master_contexts_only() {
    let mut h = PlexiUiHarness::new_sized(900.0, 620.0);
    let child_ctx_id = h.with_app_mut(|app| {
        app.sidebar_visible = true;
        app.router.get_mut(0).name = "master-one".to_string().into();
        let parent_ctx_id = app.router.get(0).context_id;
        let child_ctx_id = parent_ctx_id + 9_000;

        // A subcontext of the first master, sitting *between* the two
        // masters in router order — the position that used to shift the
        // second master's sidebar number from 2 to 3.
        app.router.push(crate::host::context::Context {
            name: "child-of-one".to_string().into(),
            root: std::env::temp_dir().join("child-of-one"),
            description: None,
            context_id: child_ctx_id,
            parent_id: Some(parent_ctx_id),
            depth: 1,
            parked: false,
        });
        app.router.push(crate::host::context::Context {
            name: "master-two".to_string().into(),
            root: std::env::temp_dir().join("master-two"),
            description: None,
            context_id: 72_002,
            parent_id: None,
            depth: 0,
            parked: false,
        });

        // Give the child a window so it has real state to roll up.
        let child_window_id = app.next_window_id;
        app.next_window_id += 1;
        app.windows.push(Window {
            name: "child window".to_string(),
            path: crate::testing::scratch_context_root("ui"),
            tree: egui_tiles::Tree::empty("child window"),
            panes: Default::default(),
            focused_pane: None,
            zoomed_pane: None,
            grid_x: 0,
            grid_y: 1,
            window_id: child_window_id,
            context_id: child_ctx_id,
            tree_history: Default::default(),
        });
        child_ctx_id
    });
    let child_window_idx = h.with_app(|app| app.windows.len() - 1);
    add_focused_pane_to_window(&mut h, child_window_idx);
    h.run_steps(2);

    // Rendered sidebar: both masters present, the subcontext absent.
    assert!(
        h.host_has_label("master-one"),
        "top-level contexts must render in the sidebar"
    );
    assert!(
        h.host_has_label("master-two"),
        "top-level contexts must render in the sidebar"
    );
    assert!(
        !h.host_has_label("child-of-one"),
        "a subcontext must never render a sidebar row"
    );

    h.with_app(|app| {
        // Numbering: the sidebar enumerates only masters, so master-two
        // holds slot 2 (Cmd+2) despite the subcontext preceding it.
        let order = app.router.top_level_order();
        let names: Vec<&str> = order
            .iter()
            .map(|&i| app.router.get(i).name.displayed())
            .collect();
        assert_eq!(
            names,
            vec!["master-one", "master-two"],
            "subcontexts must not be enumerated or shift top-level numbering"
        );

        // Rollup intact: the child is still observable from its parent,
        // which is what the Portal tile paints.
        let parent_state = crate::host::context_state::ContextState::compute(
            app.router.get(0).context_id,
            app.router.as_slice(),
            &app.windows,
        );
        let child_state = parent_state
            .children
            .iter()
            .find(|c| c.context_id == child_ctx_id)
            .expect("parent must still roll up its subcontext");
        assert_eq!(child_state.label, "child-of-one");
        assert_eq!(
            child_state.pane_count, 1,
            "child pane activity must still surface through the parent"
        );
    });

    h.save_screenshot(&evidence_png!()).expect("render failed");
}

/// Stint 0749: the sidebar context list has no vertical scroll, so once
/// enough contexts stack up to exceed the panel's height the overflow
/// rows were unreachable — no scrollbar, no way to bring them into view.
/// Push enough contexts to overflow a short sidebar, then drive the
/// accessibility `ScrollIntoView` action (the same path a real scroll
/// gesture or keyboard nav ends up using) on the last row and confirm it
/// actually moves into the visible sidebar area.
#[test]
fn sidebar_scrolls_to_reach_contexts_past_the_panel_height() {
    let mut h = PlexiUiHarness::new_sized(900.0, 320.0);
    h.with_app_mut(|app| app.sidebar_visible = true);
    h.step();

    for n in 0..20 {
        push_sidebar_context(&mut h, &format!("ctx-{n}"), &format!("/tmp/ctx-{n}"));
    }
    h.run_steps(3);

    // The harness window is 320pt tall, so anything laid out well past
    // that is outside the visible sidebar area — proof this test is
    // actually exercising overflow, not a coincidentally-fitting list.
    let last_rect_before = h.harness().get_by_label("ctx-19").rect();
    assert!(
        last_rect_before.min.y > 320.0,
        "test setup: the last context should start out past the visible sidebar \
             area (rect={:?}), otherwise this test isn't exercising overflow",
        last_rect_before
    );

    h.harness().get_by_label("ctx-19").scroll_to_me();
    h.run_steps(3);

    let last_rect_after = h.harness().get_by_label("ctx-19").rect();
    assert!(
        last_rect_after.min.y < last_rect_before.min.y,
        "scrolling the sidebar must move an overflowed context row toward the \
             visible area (before={:?}, after={:?}) — a bare Panel with no ScrollArea \
             never moves row rects in response to a scroll action",
        last_rect_before,
        last_rect_after
    );
}

/// Stint 0715: visual review surface for the first-run sidebar — exactly
/// one context, so the close zone is suppressed and the row is at its
/// widest text budget. This is the state that used to ramp the panel to
/// its `size_range` maximum, clip the row card, and leave a dead strip
/// between the sidebar and the central pane.
#[test]
fn screenshot_single_context_sidebar() {
    let mut h = PlexiUiHarness::new_sized_ppp(1280.0, 800.0, 2.0);
    h.with_app_mut(|app| app.sidebar_visible = true);
    h.run_steps(6);
    h.with_app(|app| {
        assert_eq!(app.router.len(), 1, "first-run is a single context");
    });
    h.save_screenshot(&evidence_png!()).expect("render failed");
}

/// Stint 0388: visual proof that creating a context lands directly in the
/// workspace with its auto-derived sidebar label and no rename overlay.
#[test]
fn screenshot_new_context_without_rename_modal() {
    let mut h = PlexiUiHarness::new_sized_ppp(1280.0, 800.0, 2.0);
    h.with_app_mut(|app| {
        app.sidebar_visible = true;
        let before = app.router.len();
        app.new_context(crate::broker::pane_authority::GrantSource::DesktopInput);
        assert_eq!(app.router.len(), before + 1, "new context created");
        assert_eq!(app.context_rename, None, "rename overlay stays closed");
    });
    h.run_steps(6);
    h.save_screenshot(&evidence_png!()).expect("render failed");
}

/// Stint 0726: visual proof that the sidebar shows the new project's
/// basename immediately after an explicit root change, not the stale
/// automatic name from before the move.
#[test]
fn screenshot_sidebar_auto_name_follows_root_change() {
    let mut h = PlexiUiHarness::new_sized_ppp(1280.0, 800.0, 2.0);
    let parent = tempfile::tempdir().expect("tempdir");
    let new_root = parent.path().join("plexi-target-project");
    std::fs::create_dir(&new_root).expect("target project dir");

    h.with_app_mut(|app| {
        app.sidebar_visible = true;
        let idx = app.router.active_idx();
        app.router.get_mut(idx).name = crate::host::context::ContextName::auto("stale-name");
        app.set_context_root(new_root.clone(), None);
        assert_eq!(
            app.router.get(idx).name.displayed(),
            "plexi-target-project",
            "sidebar label must resolve to the new root's basename"
        );
    });
    h.run_steps(6);
    h.save_screenshot(&evidence_png!()).expect("render failed");
}

/// Push a second spatial window onto `context_id` and return its index, so
/// a sidebar row can be seeded with more than one pip capsule.
fn push_window_for_context(
    h: &mut PlexiUiHarness,
    context_id: u64,
    grid_x: u32,
    path: &str,
) -> usize {
    h.with_app_mut(|app| {
        let window_id = app.next_window_id;
        app.next_window_id += 1;
        app.windows.push(crate::host::context::Window {
            name: String::new(),
            path: std::path::PathBuf::from(path),
            tree: egui_tiles::Tree::empty("sidebar-row-states"),
            panes: Default::default(),
            focused_pane: None,
            zoomed_pane: None,
            grid_x,
            grid_y: 0,
            window_id,
            context_id,
            tree_history: Default::default(),
        });
        app.windows.len() - 1
    })
}

/// Fill `window_index` with `count` view panes wired into one horizontal
/// container, so the window's tree really reports every pane — inserting
/// panes without attaching them to the root leaves them invisible to the
/// spatial walk the sidebar pips are built from.
fn fill_window_with_panes(h: &mut PlexiUiHarness, window_index: usize, count: usize) {
    let browser_root = h.workspace.path().to_path_buf();
    h.with_app_mut(|app| {
        let mut tiles = Vec::with_capacity(count);
        for _ in 0..count {
            let pane_id = app.host.alloc_pane_id();
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
                name: "Test App".to_string(),
                pane_group: None,
                linked_pane_id: None,
                overlay_replaced: None,
                agent: None,
                slots: std::collections::HashMap::new(),
                semantic_state: Default::default(),
            };
            let win = &mut app.windows[window_index];
            win.panes
                .insert(pane_id, Pane::View(Box::new(view_pane)))
                .unwrap();
            tiles.push(win.tree.tiles.insert_pane(pane_id));
        }
        let win = &mut app.windows[window_index];
        let container = win.tree.tiles.insert_horizontal_tile(tiles.clone());
        win.tree.root = Some(container);
        win.focused_pane = tiles.first().copied();
    });
}

/// Seed a context with a name, a root path, and its own window, returning
/// `(router index, context id, window index)`.
fn push_sidebar_context(h: &mut PlexiUiHarness, name: &str, root: &str) -> (usize, u64, usize) {
    let (idx, context_id) = h.with_app_mut(|app| {
        let context_id = app.next_window_id;
        app.next_window_id += 1;
        app.router.push(crate::host::context::Context {
            name: name.to_string().into(),
            root: std::path::PathBuf::from(root),
            description: None,
            context_id,
            parent_id: None,
            depth: 0,
            parked: false,
        });
        (app.router.len() - 1, context_id)
    });
    let window_idx = push_window_for_context(h, context_id, 0, root);
    (idx, context_id, window_idx)
}

fn seeded_notification(
    notify_id: &str,
    source_context_id: u64,
    source_window_id: u64,
) -> crate::app::PendingNotification {
    crate::app::PendingNotification {
        notify_id: notify_id.to_string(),
        sender_pane_id: 0,
        dismiss_owner_pane_id: 0,
        source_context_id,
        source_window_id,
        title: "Preview badge".to_string(),
        body: "Seeded sidebar notification".to_string(),
        kind: crate::protocol::NotifyKind::Message,
        options: vec![],
        input_prompt: None,
        required: false,
        scope: crate::protocol::NotifyScope::Context,
        image_inline: None,
        image_pipe_id: None,
        response_file: None,
        timeout_secs: None,
        on_dismiss: None,
        enqueued_at: std::time::Instant::now(),
        tombstoned: false,
        delivery_unknown: false,
        host_integrity: false,
        deliver_after: None,
        origin_in_view: false,
    }
}

/// Stint 0717: the whole vocabulary of the rebuilt two-tier context row in
/// one frame, at HiDPI. Identity tier — index number, context name, and
/// the close action pinned top-right. Pane-state tier — per-window pip
/// capsules with the return-target pin and agent activity colors, the
/// overflow count, the notification badge, and the root path trailing.
/// Read the PNG: the number must sit on the name's optical center; the
/// badge must read as a different kind of thing from the collapsed pips
/// beside it; the capsule's top edge must be unbroken except where the pin
/// meets it; and nothing on either tier may touch its neighbour.
#[test]
fn screenshot_sidebar_row_states() {
    const SHOT_PPP: f32 = 2.0;
    let mut h = PlexiUiHarness::new_sized_ppp(1100.0, 700.0, SHOT_PPP);
    h.step();

    let (active_context_id, active_window_id) = h.with_app_mut(|app| {
        app.sidebar_visible = true;
        let idx = app.router.active_idx();
        app.router.get_mut(idx).name = "PLEXI Workspace".to_string().into();
        app.router.get_mut(idx).root = std::path::PathBuf::from("/projects/PLEXI");
        (
            app.router.get(idx).context_id,
            app.windows[app.active_window].window_id,
        )
    });

    // Active context: two spatial windows, so the row carries two capsules
    // and the pin marks the one activation will return to.
    let active_window = h.with_app(|app| app.active_window);
    fill_window_with_panes(&mut h, active_window, 3);
    let second_window =
        push_window_for_context(&mut h, active_context_id, 1, "/projects/PLEXI");
    fill_window_with_panes(&mut h, second_window, 2);

    // A long name that must elide, and enough panes to overflow the cap.
    let (_, narrator_id, narrator_window) = push_sidebar_context(
        &mut h,
        "Narrator Video Production Workspace",
        "/projects/narrator-site",
    );
    fill_window_with_panes(&mut h, narrator_window, 11);
    let narrator_window_id = h.with_app(|app| app.windows[narrator_window].window_id);

    let (_, _, scratch_window) = push_sidebar_context(&mut h, "scratch", "/tmp/scratch");
    fill_window_with_panes(&mut h, scratch_window, 1);

    // Agent activity across the pips, and badges on two rows.
    h.with_app_mut(|app| {
        let statuses = [
            crate::protocol::PipStatus::Yellow,
            crate::protocol::PipStatus::Green,
            crate::protocol::PipStatus::Red,
        ];
        for (i, pane) in app.windows[0].panes.values_mut().enumerate() {
            if let Pane::View(view_pane) = pane {
                view_pane.pip_status = Some(statuses[i % statuses.len()]);
            }
        }
        app.pending_notifications.extend([
            seeded_notification("active-one", active_context_id, active_window_id),
            seeded_notification("active-two", active_context_id, active_window_id),
            seeded_notification("active-three", active_context_id, active_window_id),
            seeded_notification("narrator-one", narrator_id, narrator_window_id),
            seeded_notification("narrator-two", narrator_id, narrator_window_id),
        ]);
    });
    h.run_steps(3);

    // Hover the inactive "scratch" row so one row shows its close glyph
    // while the active row shows its badge — the two states that used to
    // share a single trailing zone.
    // kittest reports node rects in physical pixels; pointer events are in
    // points, so the HiDPI scale has to come back out.
    let scratch_row = h.harness().get_by_label("scratch").rect().center() / SHOT_PPP;
    h.harness()
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(scratch_row));
    h.step();

    assert_long_context_name_elides(&mut h, "Narrator Video Production Workspace");
    h.save_screenshot(&evidence_png!()).expect("render failed");
}

/// A sidebar row paints a name too long for its identity tier with a
/// trailing ellipsis and never paints it whole.
fn assert_long_context_name_elides(h: &mut PlexiUiHarness, full: &str) {
    let head = full.split(' ').next().expect("non-empty name");
    let texts = painted_texts(h);
    assert!(
        texts.iter().any(
            |(_, text)| text.starts_with(head) && text.ends_with(crate::ui::text::ELLIPSIS)
        ),
        "{full:?} must paint elided: {texts:?}"
    );
    assert!(
        !texts.iter().any(|(_, text)| text == full),
        "{full:?} must not paint whole: {texts:?}"
    );
}

/// Stint 0605: `NextContext`/`PrevContext` cycle the same enumeration the
/// sidebar draws. Against a live router carrying a real subcontext wedged
/// between two masters, cycling steps master → master and wraps, so it can
/// never activate a context with no row, no number, and no recorded depth
/// entry for zoom-out to pop.
#[test]
fn context_cycling_visits_master_contexts_only() {
    let mut h = PlexiUiHarness::new_sized(900.0, 620.0);
    let child_ctx_id = h.with_app_mut(|app| {
        app.router.get_mut(0).name = "master-one".to_string().into();
        let parent_ctx_id = app.router.get(0).context_id;
        let child_ctx_id = parent_ctx_id + 9_000;

        // Subcontext at router index 1, between the two masters — the
        // index the old raw-index walk would have landed on.
        app.router.push(crate::host::context::Context {
            name: "child-of-one".to_string().into(),
            root: std::env::temp_dir().join("child-of-one"),
            description: None,
            context_id: child_ctx_id,
            parent_id: Some(parent_ctx_id),
            depth: 1,
            parked: false,
        });
        app.router.push(crate::host::context::Context {
            name: "master-two".to_string().into(),
            root: std::env::temp_dir().join("master-two"),
            description: None,
            context_id: 72_002,
            parent_id: None,
            depth: 0,
            parked: false,
        });
        child_ctx_id
    });
    h.run_steps(2);

    h.with_app(|app| {
        // From master-one (idx 0): forward skips the subcontext at idx 1.
        let next = app.router.cycle_top_level(true).expect("next exists");
        assert_eq!(
            app.router.get(next).name,
            "master-two",
            "cycling must skip the subcontext sitting between the masters"
        );
        assert_ne!(
            app.router.get(next).context_id,
            child_ctx_id,
            "cycling must never activate a subcontext"
        );

        // Backward from the first master wraps to the last master, not to
        // the subcontext that precedes it in raw router order.
        let prev = app.router.cycle_top_level(false).expect("prev exists");
        assert_eq!(
            app.router.get(prev).name,
            "master-two",
            "cycling backward from the first master wraps to the last"
        );
    });
}

/// Stint 0713: at the minimum window width (400pt) and across standard
/// widths (520pt, 800pt, 1280pt), the sidebar must clamp so that at least
/// `style::SIDEBAR_CONTENT_MIN_W` (260pt) of usable content area remains.
#[test]
fn sidebar_preserves_content_area_floor_at_minimum_and_standard_window_widths() {
    for (window_w, expected_sidebar_w) in [
        (400.0, crate::ui::style::SIDEBAR_MIN_W),
        (520.0, crate::ui::style::SIDEBAR_DEFAULT_W),
        (800.0, crate::ui::style::SIDEBAR_DEFAULT_W),
        (1280.0, crate::ui::style::SIDEBAR_DEFAULT_W),
    ] {
        let mut h = PlexiUiHarness::new_sized(window_w, 320.0);
        h.step();
        add_focused_pane(&mut h);
        h.with_app_mut(|app| {
            app.sidebar_visible = true;
        });
        h.run_steps(2);
        if (window_w - 400.0).abs() < f32::EPSILON {
            let shot = evidence_png!("_400pt");
            h.save_screenshot(&shot)
                .expect("save 400pt sidebar clamp screenshot");
        }
        let sidebar_w = h.with_app(|app| {
            egui::containers::panel::PanelState::load(&app.ctx, egui::Id::new("sidebar"))
                .expect("sidebar panel state must exist")
                .rect
                .width()
        });
        assert!(
            (sidebar_w - expected_sidebar_w).abs() <= 1.0,
            "at window_w={window_w}, expected sidebar width {expected_sidebar_w}, got {sidebar_w}"
        );
        let content_w = window_w - sidebar_w;
        assert!(
            content_w + 0.5 >= crate::ui::style::SIDEBAR_CONTENT_MIN_W,
            "at window_w={window_w}, content width {content_w} must be >= SIDEBAR_CONTENT_MIN_W ({})",
            crate::ui::style::SIDEBAR_CONTENT_MIN_W
        );
    }
}

// ── s8 pixel-grid + typography evidence (stints 0527–0530) ──────────────

/// Stint 0700: the 220-point sidebar shows an ordinary context name in full
/// while a genuinely long name ellipsizes. The hover makes the sole trailing
/// close affordance visible in the evidence image.
#[test]
fn screenshot_sidebar_x_end_at_default_width() {
    let mut h = PlexiUiHarness::new_sized(1100.0, 700.0);
    h.step();
    let inactive_window_idx = h.with_app_mut(|app| {
        app.sidebar_visible = true;
        let active_context_idx = app.router.active_idx();
        let active_context_id = app.router.get(active_context_idx).context_id;
        let active_window_id = app.windows[app.active_window].window_id;
        app.router.get_mut(active_context_idx).name = "PLEXI Workspace".to_string().into();
        app.router.get_mut(active_context_idx).root =
            std::path::PathBuf::from("/projects/PLEXI");

        let inactive_context_id = app.next_window_id;
        app.next_window_id += 1;
        let inactive_root = std::path::PathBuf::from("/projects/narrator-site");
        app.router.push(crate::host::context::Context {
            // Deliberately longer than a normal workspace title: the
            // screenshot must exercise truncation without making the
            // ordinary active title a false-positive fixture.
            name: "Narrator Video Production Workspace".to_string().into(),
            root: inactive_root.clone(),
            description: None,
            context_id: inactive_context_id,
            parent_id: None,
            depth: 0,
            parked: false,
        });
        let inactive_window_id = app.next_window_id;
        app.next_window_id += 1;
        app.windows.push(crate::host::context::Window {
            name: String::new(),
            path: inactive_root,
            tree: egui_tiles::Tree::empty("sidebar-x-end-preview"),
            panes: Default::default(),
            focused_pane: None,
            zoomed_pane: None,
            grid_x: 0,
            grid_y: 0,
            window_id: inactive_window_id,
            context_id: inactive_context_id,
            tree_history: Default::default(),
        });

        let notification = |notify_id: &str, source_context_id, source_window_id| {
            crate::app::PendingNotification {
                notify_id: notify_id.to_string(),
                sender_pane_id: 0,
                dismiss_owner_pane_id: 0,
                source_context_id,
                source_window_id,
                title: "Preview badge".to_string(),
                body: "Seeded sidebar notification".to_string(),
                kind: crate::protocol::NotifyKind::Message,
                options: vec![],
                input_prompt: None,
                required: false,
                scope: crate::protocol::NotifyScope::Context,
                image_inline: None,
                image_pipe_id: None,
                response_file: None,
                timeout_secs: None,
                on_dismiss: None,
                enqueued_at: std::time::Instant::now(),
                tombstoned: false,
                delivery_unknown: false,
                host_integrity: false,
                deliver_after: None,
                origin_in_view: false,
            }
        };
        app.pending_notifications.extend([
            notification("active-one", active_context_id, active_window_id),
            notification("active-two", active_context_id, active_window_id),
            notification("active-three", active_context_id, active_window_id),
            notification("inactive-one", inactive_context_id, inactive_window_id),
            notification("inactive-two", inactive_context_id, inactive_window_id),
        ]);
        app.windows.len() - 1
    });
    add_focused_pane(&mut h);
    add_focused_pane_to_window(&mut h, inactive_window_idx);
    h.run_steps(3);

    // Hover a sidebar row so the trailing close glyph is visible in the PNG.
    // The zoom path bar repeats the active context name; the sidebar row is
    // the leftmost match.
    let active_title = h
        .harness()
        .get_all_by_label("PLEXI Workspace")
        .map(|n| n.rect())
        .min_by(|a, b| a.min.x.total_cmp(&b.min.x))
        .expect("sidebar row")
        .center();
    h.harness()
        .get_by_label("Narrator Video Production Workspace");
    h.harness()
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(active_title));
    h.step();
    assert_long_context_name_elides(&mut h, "Narrator Video Production Workspace");
    let texts = painted_texts(&mut h);
    assert!(
        texts.iter().any(|(_, text)| text == "PLEXI Workspace"),
        "an ordinary context name paints in full: {texts:?}"
    );
    h.save_screenshot(&evidence_png!())
        .expect("220-point sidebar render");
}
