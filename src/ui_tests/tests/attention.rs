use super::*;

/// The attention chip in the path bar keeps its own slot: at every UI
/// scale and window width it stays inside the viewport and never
/// intersects the toolbar controls, version label, or the pane header's
/// provenance icon, however long the pending ask is.
#[test]
fn attention_chip_never_overlaps_chrome_at_any_scale_or_width() {
    for scale in [0.6_f32, 1.0, 2.0] {
        for width in [700.0_f32, 1400.0] {
            let mut h = PlexiUiHarness::new_sized_scaled(width, 900.0, scale);
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
                        working_directory: Some(workspace_root),
                        ..Default::default()
                    },
                    14.0,
                )
                .expect("spawn terminal pane");
                terminal.spawned_by = Some("agent:researcher".to_string());
                let win = &mut app.windows[app.active_window];
                win.panes.insert(id, Pane::Terminal(Box::new(terminal))).unwrap();
                let tile = win.tree.tiles.insert_pane(id);
                win.tree.root = Some(tile);
                win.focused_pane = Some(tile);
                app.enqueue_ask(
                    "ask-chip",
                    id,
                    "permission: Allow flash: cargo test --workspace --all-features -- --nocapture --test-threads=1",
                    1_000,
                    Vec::new(),
                );
                id
            });
            h.run_steps(4);
            let (screen, chip, siblings) = h.with_app(|app| {
                let get = |id: egui::Id| app.ctx.data(|d| d.get_temp::<egui::Rect>(id));
                let chip = get(egui::Id::new("attention_chip_rect")).expect("chip rect");
                let mut siblings = Vec::new();
                for name in ["help", "version"] {
                    siblings.push((
                        name,
                        get(egui::Id::new(("toolbar_ctl_rect", name))).expect("toolbar rect"),
                    ));
                }
                siblings.push((
                    "provenance",
                    get(egui::Id::new(("provenance_icon_rect", pane_id)))
                        .expect("provenance icon rect"),
                ));
                (app.ctx.content_rect(), chip, siblings)
            });
            let what = format!("scale {scale} width {width}");
            assert_inside("attention chip", scale, chip, screen);
            for (name, rect) in &siblings {
                assert!(
                    !chip.intersects(*rect),
                    "{what}: chip {chip:?} overlaps {name} {rect:?}"
                );
            }
            for (i, (an, a)) in siblings.iter().enumerate() {
                for (bn, b) in &siblings[i + 1..] {
                    assert!(!a.intersects(*b), "{what}: {an} {a:?} overlaps {bn} {b:?}");
                }
            }
        }
    }
}

#[test]
fn decision_overlay_shows_no_scrollbar_or_scroll_state_when_content_fits() {
    for scale in [1.0_f32, 2.0] {
        let mut h = PlexiUiHarness::new_sized_scaled(1440.0, 900.0, scale);
        let pane_id = add_focused_pane(&mut h);
        h.with_app_mut(|app| {
            app.enqueue_ask(
                "ask-no-scroll",
                pane_id,
                "cargo test --bin plexi -- --test-threads=4",
                1_000,
                Vec::new(),
            );
            assert!(app.open_decision_overlay_for_ask("ask-no-scroll"));
        });
        h.run_steps(4);

        let (
            content_size,
            avail_size,
            card_inner_size,
            scroll_id,
            has_scroll_state,
            button_widths,
            modal_w,
        ) = h.with_app_mut(|app| {
            let (content_size, avail_size, card_inner_size, scroll_id): (
                egui::Vec2,
                egui::Vec2,
                egui::Vec2,
                egui::Id,
            ) = app
                .ctx
                .data(|d| d.get_temp(egui::Id::new("decision_overlay_scroll_metrics")))
                .expect("decision overlay scroll metrics");
            let has_scroll_state =
                egui::scroll_area::State::load(&app.ctx, scroll_id).is_some();
            let button_widths: Vec<f32> = app
                .approval_buttons
                .iter()
                .map(|b| b.bounds[2] - b.bounds[0])
                .collect();
            let screen = app.ctx.content_rect();
            let modal_w = crate::ui::style::DECISION_MODAL_W
                .min(screen.width() - 32.0)
                .max(300.0);
            (
                content_size,
                avail_size,
                card_inner_size,
                scroll_id,
                has_scroll_state,
                button_widths,
                modal_w,
            )
        });

        assert_eq!(button_widths.len(), 3, "scale {scale}: expected 3 options");
        assert!(
            content_size.y <= avail_size.y,
            "scale {scale}: content height {} must fit within available {}",
            content_size.y,
            avail_size.y
        );
        assert!(
            (content_size.y - card_inner_size.y).abs() <= 1.0,
            "scale {scale}: card inner height {} must shrink to content height {}",
            card_inner_size.y,
            content_size.y
        );
        assert!(
            !has_scroll_state,
            "scale {scale}: no ScrollArea offset state ({scroll_id:?}) should persist when content fits"
        );
        for (idx, w) in button_widths.iter().enumerate() {
            assert!(
                (w - modal_w).abs() <= 1.0,
                "scale {scale}: option {idx} width {w} must equal modal_w {modal_w} (no scrollbar gutter)"
            );
        }
    }
}

#[test]
fn needs_you_failed_audit_error_is_visible_in_decision_overlay() {
    let mut h = PlexiUiHarness::new_sized(1100.0, 760.0);
    let pane_id = add_focused_pane(&mut h);
    let monitor =
        crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir())
            .unwrap();
    let id = monitor
        .file_needs_you_with_details(
            crate::broker::gate::NeedsYouFile {
                kind: crate::broker::gate::NeedsYouKind::Question,
                actor: "codex".into(),
                resource: format!("pane:{pane_id}"),
                summary: "Ready to continue the attention-cue repair on Alpha".into(),
                expires_at: None,
                run_tag: Some(format!("pane:{pane_id}")),
            },
            Some(crate::broker::gate::NeedsYouDetails {
                choices: vec!["Continue".to_string()],
                ..Default::default()
            }),
        )
        .unwrap();
    h.with_app_mut(|app| {
        assert!(app.open_decision_overlay_for_ask(&id));
        app.arm_decision_overlay_for_test();
    });
    monitor.fail_audit(true);
    h.with_app_mut(|app| {
        assert!(!app.pick_decision_overlay_option(0));
    });
    h.run_steps(3);
    let message = format!(
        "Decision failed: Could not complete decision overlay: needs-you item {id} could not be audited"
    );
    assert!(
        h.host_has_label(&message),
        "audit error must be painted in the retained overlay"
    );
    h.save_screenshot(&crate::ui_tests::evidence_png!())
        .unwrap();
    monitor.fail_audit(false);
}

#[test]
fn screenshot_attention_queue_all_states_themes() {
    use crate::app::attention_queue::{QueueBar, QueueCategory, QueueSource};
    let mut h = PlexiUiHarness::new_sized(1280.0, 800.0);
    h.with_app_mut(|app| {
        app.attention_queue.synthetic_items = [
            QueueCategory::Ask,
            QueueCategory::Failed,
            QueueCategory::Done,
            QueueCategory::Notice,
        ]
        .into_iter()
        .enumerate()
        .map(|(i, category)| QueueBar {
            id: format!("fixture-{i}"),
            category,
            pane_id: i as u64 + 1,
            pane_name: format!("Agent {category:?}"),
            context_id: 0,
            context_path: "workspace / agent".into(),
            summary: format!(
                "{category:?}: Review the latest agent result and decide what to do next"
            ),
            command_text: String::new(),
            options: vec![],
            created_at_ms: i as u64,
            locked: false,
            source: QueueSource::Synthetic {
                id: format!("fixture-{i}"),
            },
        })
        .collect();
        app.sync_attention_queue();
        app.attention_queue.open = true;
    });
    for theme in ["plexi-night", "plexi-dusk", "plexi-dawn", "plexi-day"] {
        h.with_app_mut(|app| {
            let cfg = crate::ui::theme::preset_colors(theme).unwrap();
            app.colors = crate::ui::theme::Colors::from_config(&cfg);
            app.theme = crate::ui::theme::terminal_theme(
                &cfg,
                crate::config::DEFAULT_TERMINAL_MINIMUM_CONTRAST,
            );
            crate::ui::theme::setup_style(&app.ctx, &app.colors);
        });
        for selected in 0..4 {
            h.with_app_mut(|app| app.attention_queue.selected_idx = selected);
            h.run_steps(4);
            h.with_app(|app| assert_eq!(app.attention_queue.items.len(), 4));
            h.save_screenshot(&evidence_png!(&format!("_{theme}_{selected}")))
                .unwrap();
        }
    }
}

#[test]
fn screenshot_attention_queue_realistic_asks_and_results() {
    let mut h = PlexiUiHarness::new_sized(1280.0, 800.0);
    let monitor =
        crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir())
            .unwrap();
    monitor
        .file_needs_you(crate::broker::gate::NeedsYouFile {
            kind: crate::broker::gate::NeedsYouKind::Question,
            actor: "agent:coder".to_string(),
            resource: "src/app/attention_queue.rs".to_string(),
            summary: "Should we return to parent conversation on Enter?".to_string(),
            expires_at: None,
            run_tag: None,
        })
        .unwrap();

    h.with_app_mut(|app| {
        app.record_child_agent_result(
            None,
            crate::workspace::QueuedChildResult {
                child_pane_id: 1042,
                child_agent: "researcher".to_string(),
                status: "completed".to_string(),
                summary: Some(
                    "Analyzed parent-first navigation patterns across codebase".to_string(),
                ),
                result_json: "{}".to_string(),
                finished_at_epoch_secs: 1000,
            },
        );
        app.sync_attention_queue();
        app.attention_queue.open = true;
    });

    h.run_steps(4);
    // The open queue lists both the agent's question and the finished
    // child's result, each with its own summary.
    let texts = painted_texts(&mut h);
    painted_rect(&texts, "Should we return to parent");
    painted_rect(&texts, "Analyzed parent-first");
    h.save_screenshot(&evidence_png!())
        .expect("render attention queue screenshot");
}

fn add_attention_result_terminal(
    app: &mut PlexiApp,
    agent: Option<(&str, &str)>,
    parent_pane_id: Option<PaneId>,
    spawned_by: Option<&str>,
) -> PaneId {
    let pane_id = app.host.alloc_pane_id();
    let mut terminal = TerminalPane::fixture(
        pane_id,
        app.ctx.clone(),
        app.pty_event_tx.clone(),
        egui_term::BackendSettings {
            shell: "/bin/sh".into(),
            ..Default::default()
        },
        14.0,
    )
    .expect("create attention-result terminal");
    if let Some((harness, role)) = agent {
        terminal.agent_harness = Some(harness.into());
        terminal.agent_role = Some(role.into());
        terminal.agent = Some(crate::protocol::PaneAgentState {
            pane_id,
            state: crate::protocol::AgentState::Idle,
            agent: harness.into(),
            detail: None,
            session_id: None,
        });
    }
    terminal.parent_pane_id = parent_pane_id;
    terminal.spawned_by = spawned_by.map(str::to_owned);
    let active_window = app.active_window;
    let window = &mut app.windows[active_window];
    window
        .panes
        .insert(pane_id, Pane::Terminal(Box::new(terminal)))
        .unwrap();
    let tile = window.tree.tiles.insert_pane(pane_id);
    if window.tree.root().is_none() {
        window.tree.root = Some(tile);
        window.focused_pane = Some(tile);
    }
    pane_id
}

#[test]
fn attention_result_enter_and_click_use_live_parent_activation_paths() {
    let mut h = PlexiUiHarness::new_sized(1280.0, 800.0);
    h.step();
    let (parent, child) = h.with_app_mut(|app| {
        let parent = add_attention_result_terminal(app, Some(("claude", "lead")), None, None);
        let child = add_attention_result_terminal(
            app,
            Some(("claude", "result-worker")),
            Some(parent),
            Some("agent:lead"),
        );
        app.record_child_agent_result(
            Some(parent),
            crate::workspace::QueuedChildResult {
                child_pane_id: child,
                child_agent: "result-worker".to_string(),
                status: "completed".to_string(),
                summary: Some("Finished the parent-first navigation review".to_string()),
                result_json: "{}".to_string(),
                finished_at_epoch_secs: 1_000,
            },
        );
        app.close_pane_by_id(child);
        app.sync_attention_queue();
        app.windows[app.active_window].focused_pane = None;
        app.toggle_attention_queue_list();
        (parent, child)
    });

    // Keyboard Enter uses the host input path and resolves to the live parent.
    h.run_steps(2);
    h.harness().key_press(egui::Key::Enter);
    h.run_steps(2);
    h.with_app(|app| {
        assert_eq!(app.active_focused_pane_id(), Some(parent));
        assert!(
            app.attention_queue
                .items
                .iter()
                .any(|bar| bar.pane_id == child),
            "Enter must leave the child result reachable"
        );
    });

    // Clear focus, then click the rendered queue row through kittest's pointer path.
    h.with_app_mut(|app| {
        app.windows[app.active_window].focused_pane = None;
        app.toggle_attention_queue_list();
    });
    h.run_steps(2);
    let row_click = h.with_app(|app| {
        let panel = app
            .ctx
            .memory(|memory| memory.area_rect(egui::Id::new("attention_queue_panel")))
            .expect("rendered attention queue panel");
        egui::pos2(panel.center().x, panel.max.y - 28.0)
    });
    h.harness().input_mut().events.extend([
        egui::Event::PointerMoved(row_click),
        egui::Event::PointerButton {
            pos: row_click,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        },
        egui::Event::PointerButton {
            pos: row_click,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        },
    ]);
    h.run_steps(2);
    h.with_app(|app| {
        assert_eq!(app.active_focused_pane_id(), Some(parent));
        assert!(
            app.attention_queue
                .items
                .iter()
                .any(|bar| bar.pane_id == child),
            "click must leave the child result reachable"
        );
    });
}

#[test]
fn attention_mailbox_states_and_absence_of_second_queue_bar() {
    let mut h = PlexiUiHarness::new_sized(1280.0, 800.0);
    h.step();

    let path_bar_shape_count = |h: &mut PlexiUiHarness| {
        let path_bar = egui::Rect::from_min_size(
            egui::pos2(0.0, crate::ui::style::TOOLBAR_H + 1.0),
            egui::vec2(
                h.with_app(|app| app.ctx.content_rect().width()),
                crate::ui::style::PATH_BAR_H,
            ),
        );
        h.harness()
            .output()
            .shapes
            .iter()
            .filter(|shape| {
                let painted = shape.shape.visual_bounding_rect();
                painted.is_positive() && path_bar.contains_rect(painted)
            })
            .count()
    };

    // 1. Empty state
    h.run_steps(4);
    let empty_path_bar_shape_count = path_bar_shape_count(&mut h);
    h.with_app(|app| {
        let mailbox_rect = app
            .ctx
            .data(|d| d.get_temp::<egui::Rect>(egui::Id::new("attention_mailbox_rect")))
            .expect("mailbox rect rendered in empty state");
        assert!(mailbox_rect.width() > 0.0 && mailbox_rect.height() > 0.0);
    });

    // 2. Asks-present state
    let pane_id = h.with_app_mut(|app| {
        let pid = app.host.alloc_pane_id();
        app.enqueue_ask(
            "ask-mb-1",
            pid,
            "permission: allow cargo test",
            1_000,
            Vec::new(),
        );
        pid
    });
    h.run_steps(4);
    assert_eq!(
        path_bar_shape_count(&mut h),
        empty_path_bar_shape_count,
        "the path bar paint must not gain a second attention queue affordance when asks arrive"
    );
    h.with_app(|app| {
        let mailbox_rect = app
            .ctx
            .data(|d| d.get_temp::<egui::Rect>(egui::Id::new("attention_mailbox_rect")))
            .expect("mailbox rect rendered in asks state");
        assert!(mailbox_rect.width() > 0.0);
        assert_eq!(app.attention_queue.ask_count(), 1);
    });

    // 3. Failure/result state
    h.with_app_mut(|app| {
        app.attention_queue.synthetic_items.clear();
        app.attention_queue
            .answered_ask_ids
            .insert("ask-mb-1".to_string());
        app.record_child_agent_result(
            None,
            crate::workspace::QueuedChildResult {
                child_pane_id: pane_id,
                child_agent: "worker".to_string(),
                status: "failed".to_string(),
                summary: Some("Build failed with compilation error".to_string()),
                result_json: "{}".to_string(),
                finished_at_epoch_secs: 1000,
            },
        );
        app.sync_attention_queue();
    });
    h.run_steps(4);
    assert_eq!(
        path_bar_shape_count(&mut h),
        empty_path_bar_shape_count,
        "the path bar paint must not gain a second attention queue affordance for failures"
    );
    h.with_app(|app| {
        let mailbox_rect = app
            .ctx
            .data(|d| d.get_temp::<egui::Rect>(egui::Id::new("attention_mailbox_rect")))
            .expect("mailbox rect rendered in failure state");
        assert!(mailbox_rect.width() > 0.0);
        assert_eq!(app.attention_queue.ask_count(), 0);
        assert!(app.attention_queue.failed_count() > 0);
        assert_eq!(
            app.ctx.data(|d| {
                d.get_temp::<egui::Color32>(egui::Id::new("attention_mailbox_color"))
            }),
            Some(app.colors.zoom_failed),
            "failed attention mailbox must use the zoom_failed color"
        );
    });
}

#[test]
fn attention_mailbox_pointer_and_keyboard_activation_preserve_decision_and_parent_first_semantics(
) {
    let mut h = PlexiUiHarness::new_sized(1280.0, 800.0);
    h.step();

    // Initially closed
    h.with_app(|app| assert!(!app.attention_queue.open));

    let mb_rect = h.with_app(|app| {
        app.ctx
            .data(|d| d.get_temp::<egui::Rect>(egui::Id::new("attention_mailbox_rect")))
            .expect("mailbox rect")
    });

    // Guard/mutation test: click somewhere else (not the mailbox)
    let miss_pos = egui::pos2(10.0, 10.0);
    h.harness().input_mut().events.extend([
        egui::Event::PointerMoved(miss_pos),
        egui::Event::PointerButton {
            pos: miss_pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        },
        egui::Event::PointerButton {
            pos: miss_pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        },
    ]);
    h.step();
    h.with_app(|app| {
        assert!(
            !app.attention_queue.open,
            "clicking outside mailbox must not open queue list"
        );
    });

    // Pointer click on mailbox opens the queue list
    let click_pos = mb_rect.center();
    h.harness().input_mut().events.extend([
        egui::Event::PointerMoved(click_pos),
        egui::Event::PointerButton {
            pos: click_pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        },
        egui::Event::PointerButton {
            pos: click_pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        },
    ]);
    h.step();
    h.with_app(|app| {
        assert!(
            app.attention_queue.open,
            "pointer click on mailbox must open attention queue list"
        );
    });

    // Click again toggles it closed
    h.harness().input_mut().events.extend([
        egui::Event::PointerMoved(click_pos),
        egui::Event::PointerButton {
            pos: click_pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        },
        egui::Event::PointerButton {
            pos: click_pos,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        },
    ]);
    h.step();
    h.with_app(|app| {
        assert!(
            !app.attention_queue.open,
            "second click on mailbox must close attention queue list"
        );
    });

    // Keyboard Cmd+I toggles it open
    h.harness()
        .key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::I);
    h.step();
    h.with_app(|app| {
        assert!(
            app.attention_queue.open,
            "Cmd+I must open attention queue list"
        );
    });

    // Close list with Esc
    h.harness().key_press(egui::Key::Escape);
    h.step();
    h.with_app(|app| {
        assert!(
            !app.attention_queue.open,
            "Esc must close attention queue list"
        );
    });

    // Verify decision actions: an open ask can be opened in Decision Overlay via Enter
    let ask_pane = add_focused_pane(&mut h);
    h.with_app_mut(|app| {
        app.enqueue_ask(
            "ask-decision-test",
            ask_pane,
            "permission: allow rm -rf",
            1_000,
            Vec::new(),
        );
        app.toggle_attention_queue_list();
    });
    h.run_steps(2);
    h.harness().key_press(egui::Key::Enter);
    h.run_steps(2);
    h.with_app(|app| {
        assert!(
            app.attention_queue.decision_overlay.is_some(),
            "Enter on an ask bar must open Decision Overlay"
        );
    });

    // Close overlay with Esc
    h.harness().key_press(egui::Key::Escape);
    h.run_steps(2);
    h.with_app_mut(|app| {
        app.attention_queue
            .answered_ask_ids
            .insert("ask-decision-test".to_string());
        app.attention_queue
            .synthetic_items
            .retain(|b| b.id != "ask-decision-test");
        app.close_pane_by_id(ask_pane);
        app.sync_attention_queue();
    });

    // Verify parent-first result navigation is preserved (stint 0962)
    let (parent, child) = h.with_app_mut(|app| {
        let parent = add_attention_result_terminal(app, Some(("claude", "lead")), None, None);
        let child = add_attention_result_terminal(
            app,
            Some(("claude", "worker")),
            Some(parent),
            Some("agent:lead"),
        );
        app.record_child_agent_result(
            Some(parent),
            crate::workspace::QueuedChildResult {
                child_pane_id: child,
                child_agent: "worker".to_string(),
                status: "completed".to_string(),
                summary: Some("Task finished".to_string()),
                result_json: "{}".to_string(),
                finished_at_epoch_secs: 1_000,
            },
        );
        app.close_pane_by_id(child);
        app.sync_attention_queue();
        app.windows[app.active_window].focused_pane = None;
        app.toggle_attention_queue_list();
        (parent, child)
    });
    h.run_steps(2);
    h.harness().key_press(egui::Key::Enter);
    h.run_steps(2);
    h.with_app(|app| {
        assert_eq!(
            app.active_focused_pane_id(),
            Some(parent),
            "parent-first navigation must activate parent pane"
        );
        assert!(
            app.attention_queue
                .items
                .iter()
                .any(|bar| bar.pane_id == child),
            "child result remains reachable"
        );
    });
}

#[test]
fn screenshot_attention_mailbox_empty_asks_results_narrow_and_scaled() {
    // 1. Empty state
    {
        let mut h = PlexiUiHarness::new_sized(1280.0, 800.0);
        h.run_steps(4);
        let shot = evidence_png!("_empty");
        h.save_screenshot(&shot)
            .expect("render empty mailbox screenshot");
        assert!(std::path::Path::new(&shot).exists());
        let png_bytes = std::fs::read(&shot).expect("read screenshot");
        assert!(!png_bytes.is_empty());
        let _ = std::fs::remove_file(&shot);
    }

    // 2. Asks-present state
    {
        let mut h = PlexiUiHarness::new_sized(1280.0, 800.0);
        h.with_app_mut(|app| {
            let pid = app.host.alloc_pane_id();
            app.enqueue_ask(
                "ask-shot",
                pid,
                "permission: Allow flash: cargo test --workspace",
                1_000,
                Vec::new(),
            );
        });
        h.run_steps(4);
        let shot = evidence_png!("_asks");
        h.save_screenshot(&shot)
            .expect("render asks mailbox screenshot");
        assert!(std::path::Path::new(&shot).exists());
        let png_bytes = std::fs::read(&shot).expect("read screenshot");
        assert!(!png_bytes.is_empty());
        let _ = std::fs::remove_file(&shot);
    }

    // 3. Failed state
    {
        let mut h = PlexiUiHarness::new_sized(1280.0, 800.0);
        h.with_app_mut(|app| {
            app.record_child_agent_result(
                None,
                crate::workspace::QueuedChildResult {
                    child_pane_id: 1041,
                    child_agent: "builder".to_string(),
                    status: "failed".to_string(),
                    summary: Some("Build failed with a compiler error".to_string()),
                    result_json: "{}".to_string(),
                    finished_at_epoch_secs: 1000,
                },
            );
            app.sync_attention_queue();
        });
        h.run_steps(4);
        let shot = evidence_png!("_failed");
        h.save_screenshot(&shot)
            .expect("render failed mailbox screenshot");
        assert!(std::path::Path::new(&shot).exists());
        let png_bytes = std::fs::read(&shot).expect("read screenshot");
        assert!(!png_bytes.is_empty());
        let _ = std::fs::remove_file(&shot);
    }

    // 4. Completed-result state
    {
        let mut h = PlexiUiHarness::new_sized(1280.0, 800.0);
        h.with_app_mut(|app| {
            app.record_child_agent_result(
                None,
                crate::workspace::QueuedChildResult {
                    child_pane_id: 1042,
                    child_agent: "researcher".to_string(),
                    status: "completed".to_string(),
                    summary: Some("Analyzed navigation patterns".to_string()),
                    result_json: "{}".to_string(),
                    finished_at_epoch_secs: 1000,
                },
            );
            app.sync_attention_queue();
        });
        h.run_steps(4);
        let shot = evidence_png!("_results");
        h.save_screenshot(&shot)
            .expect("render results mailbox screenshot");
        assert!(std::path::Path::new(&shot).exists());
        let png_bytes = std::fs::read(&shot).expect("read screenshot");
        assert!(!png_bytes.is_empty());
        let _ = std::fs::remove_file(&shot);
    }

    // 5. Narrow width (700.0)
    {
        let mut h = PlexiUiHarness::new_sized(700.0, 800.0);
        h.with_app_mut(|app| {
            let pid = app.host.alloc_pane_id();
            app.enqueue_ask(
                "ask-narrow",
                pid,
                "permission: Allow narrow layout",
                1_000,
                Vec::new(),
            );
        });
        h.run_steps(4);
        let shot = evidence_png!("_narrow");
        h.save_screenshot(&shot)
            .expect("render narrow mailbox screenshot");
        assert!(std::path::Path::new(&shot).exists());
        let png_bytes = std::fs::read(&shot).expect("read screenshot");
        assert!(!png_bytes.is_empty());
        let _ = std::fs::remove_file(&shot);
    }

    // 6. High UI scale (1.5x)
    {
        let mut h = PlexiUiHarness::new_sized_scaled(1280.0, 800.0, 1.5);
        h.with_app_mut(|app| {
            let pid = app.host.alloc_pane_id();
            app.enqueue_ask(
                "ask-scaled",
                pid,
                "permission: Allow high scale layout",
                1_000,
                Vec::new(),
            );
        });
        h.run_steps(4);
        let shot = evidence_png!("_scaled");
        h.save_screenshot(&shot)
            .expect("render scaled mailbox screenshot");
        assert!(std::path::Path::new(&shot).exists());
        let png_bytes = std::fs::read(&shot).expect("read screenshot");
        assert!(!png_bytes.is_empty());
        let _ = std::fs::remove_file(&shot);
    }
}
