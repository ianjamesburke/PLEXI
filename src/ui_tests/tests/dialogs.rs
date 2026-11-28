use super::*;

#[test]
fn screenshot_quit_save_failure_dialog() {
    let mut h = PlexiUiHarness::new_sized(900.0, 600.0);
    apply_fixture_theme(&mut h, "plexi-dusk");
    h.with_app_mut(|app| {
        let path = crate::workspace::workspace_path_in(&crate::config::config_dir());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"invalid workspace").unwrap();
        // A desktop quit: `host stop` has nobody to ask and waives the
        // save instead of opening this dialog (stint 1540).
        app.request_quit(crate::app::quit::ExitReason::WindowClosed);
        for _ in 0..crate::app::quit::MAX_QUIT_SAVE_ATTEMPTS {
            assert!(!app.persist_quit_state());
        }
        app.pending_close = true;
    });
    h.run_steps(4);
    h.with_app(|app| {
        assert!(app.quit_coordinator.save_failure().is_some());
        let reports = crate::ui::layout_report::snapshot(&app.ctx);
        let errors: Vec<_> = reports.iter().flat_map(|r| r.violations()).collect();
        assert!(errors.is_empty(), "{errors:?}");
    });
    h.save_screenshot(&evidence_png!()).unwrap();
}

fn open_quit_save_failure() -> PlexiUiHarness {
    let mut h = PlexiUiHarness::new_sized(900.0, 600.0);
    seed_quit_save_failure(&mut h);
    h
}

fn seed_quit_save_failure(h: &mut PlexiUiHarness) {
    h.with_app_mut(|app| {
        let path = crate::workspace::workspace_path_in(&crate::config::config_dir());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"invalid workspace").unwrap();
        // A desktop quit: `host stop` has nobody to ask and waives the
        // save instead of opening this dialog (stint 1540).
        app.request_quit(crate::app::quit::ExitReason::WindowClosed);
        for _ in 0..crate::app::quit::MAX_QUIT_SAVE_ATTEMPTS {
            assert!(!app.persist_quit_state());
        }
        app.pending_close = true;
    });
    h.run_steps(4);
    assert!(h.with_app(|app| app.quit_coordinator.save_failure().is_some()));
}

/// Stint 1513: "Quit without saving" takes only a fresh, real Q: not a Q
/// held from before the dialog, an injected Q or click, or Shift+Q.
#[test]
fn quit_without_saving_takes_only_a_fresh_real_q() {
    let mut h = PlexiUiHarness::new_sized(900.0, 600.0);
    add_focused_pane(&mut h);
    h.harness().key_down(egui::Key::Q);
    h.run_steps(1);
    seed_quit_save_failure(&mut h);
    let open = |h: &mut PlexiUiHarness| {
        h.with_app(|app| app.quit_coordinator.save_failure().is_some())
    };
    h.harness().key_down(egui::Key::Q);
    h.run_steps(3);
    assert!(open(&mut h), "a held Q quit without saving");
    h.harness().key_up(egui::Key::Q);
    h.run_steps(1);
    injected(&mut h, key_tap(egui::Key::Q));
    assert!(open(&mut h), "an injected Q quit without saving");
    let quit = h.harness().get_by_label("Quit without saving").rect();
    let click = injected_click(&h, quit);
    injected(&mut h, click);
    assert!(open(&mut h), "an injected click quit without saving");
    h.harness()
        .key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::Q);
    h.run_steps(3);
    assert!(open(&mut h), "Shift+Q quit without saving");
}

/// Stints 1498 and 1505: the quit save-failure dialog's rows take no Tab
/// focus, so Tab then Space does nothing and Tab then Enter cancels (the
/// safe default); neither quits without saving.
#[test]
fn quit_save_failure_tab_then_space_or_enter_does_not_quit() {
    let mut h = open_quit_save_failure();
    h.harness().key_press(egui::Key::Tab);
    h.run_steps(2);
    h.harness().key_press(egui::Key::Space);
    h.run_steps(3);
    assert!(
        h.with_app(|app| app.quit_coordinator.save_failure().is_some()),
        "Tab then Space decided the dialog"
    );
    h.harness().key_press(egui::Key::Tab);
    h.run_steps(2);
    h.harness().key_press(egui::Key::Enter);
    h.run_steps(3);
    // Quit without saving clears the failure and keeps quitting.
    assert!(
        h.with_app(|app| app.quit_coordinator.save_failure().is_some() || !app.quitting),
        "Tab then Enter quit without saving"
    );
}

/// Stint 1505: Cancel is the first action and the Enter default; "Quit
/// without saving" comes last and needs its own key.
#[test]
fn quit_save_failure_lists_cancel_first_and_enter_cancels() {
    let mut h = open_quit_save_failure();
    let cancel = h.harness().get_by_label("Cancel").rect();
    let quit = h.harness().get_by_label("Quit without saving").rect();
    assert!(
        cancel.left() > quit.left(),
        "Cancel is listed first (rightmost)"
    );
    h.harness().key_press(egui::Key::Enter);
    h.run_steps(3);
    h.with_app(|app| {
        assert!(app.quit_coordinator.save_failure().is_none(), "Enter decided the dialog");
        assert!(!app.quitting, "Enter must cancel, not quit");
    });
}

/// A text-input widget whose accessible value is exactly `value`. Selectable
/// labels expose text runs with values too, so the role is part of the match.
fn has_text_field_holding(h: &mut PlexiUiHarness, value: &str) -> bool {
    h.harness()
        .query_all_by(|node| {
            matches!(
                node.role(),
                egui::accesskit::Role::TextInput | egui::accesskit::Role::MultilineTextInput
            ) && node.value().as_deref() == Some(value)
        })
        .next()
        .is_some()
}

#[test]
fn screenshot_working_agent_quit_confirmation() {
    let mut h = PlexiUiHarness::new_sized(1168.0, 720.0);
    for name in [
        "release-checks.md",
        "implementation-plan.md",
        "test-results.json",
    ] {
        std::fs::write(
            h.workspace.path().join(name),
            "Working agent release validation",
        )
        .unwrap();
    }
    let pane = add_focused_pane(&mut h);
    h.with_app_mut(|app| {
        app.windows[0].panes.get_mut(&pane).unwrap().set_agent(Some(
            plexi_protocol::PaneAgentState {
                pane_id: pane,
                agent: "codex".into(),
                state: plexi_protocol::AgentState::Working,
                detail: Some("Validating the release build".into()),
                session_id: None,
            },
        ));
        assert!(!app.prepare_user_quit());
    });
    h.step();
    h.save_screenshot(&evidence_png!())
        .expect("render quit confirmation");
    assert!(h.with_app(|app| app.pending_quit && !app.quitting));
    println!("Screenshot saved to {}", evidence_png!());
}

/// ActionModal's one layout (stint 1558): right-aligned kit buttons with
/// their keys inside, kind-coloured, Cancel on the left.
#[test]
fn screenshot_action_modal_context_close_confirm() {
    let mut h = PlexiUiHarness::new_sized(1168.0, 720.0);
    add_focused_pane(&mut h);
    h.step();
    seed_context_close_confirm(&mut h, true);
    h.save_screenshot(&evidence_png!()).expect("render failed");
    assert!(
        h.with_app(|app| app.pending_context_close.is_some()),
        "confirm must still be pending — nothing consumed it"
    );
    println!("Screenshot saved to {}", evidence_png!());
}

/// A top-level context has no parent portal, so the modal must render
/// exactly two action rows — Dissolve would early-return (stint 0542).
#[test]
fn screenshot_action_modal_context_close_top_level_has_no_dissolve() {
    let mut h = PlexiUiHarness::new_sized(1168.0, 720.0);
    add_focused_pane(&mut h);
    h.step();
    seed_context_close_confirm(&mut h, false);
    h.save_screenshot(&evidence_png!()).expect("render failed");
    assert!(
        h.with_app(|app| app
            .pending_context_close
            .as_ref()
            .is_some_and(|state| !state.can_dissolve)),
        "a top-level context must not advertise Dissolve"
    );
    println!("Screenshot saved to {}", evidence_png!());
}

/// Stint 1491: the single-pane "Close pane?" dialog names the pane it
/// will close. The PNG is the dialog for review.
#[test]
fn close_confirm_dialog_names_the_pane() {
    let mut h = PlexiUiHarness::new_sized(1100.0, 640.0);
    let a = add_focused_pane(&mut h);
    let b = add_focused_pane(&mut h);
    h.with_app_mut(|app| {
        app.sidebar_visible = false;
        let context_id = app.windows[app.active_window].context_id;
        app.set_context_columns(context_id, vec![vec![a], vec![b]]);
        if let Some(view) = app.windows[app.active_window]
            .panes
            .get_mut(&a)
            .and_then(Pane::as_view_mut)
        {
            view.name = "Build log".to_string();
        }
    });
    h.run_steps(3);
    h.focus_pane(a).unwrap();
    h.run_steps(1);
    h.with_app_mut(|app| {
        // Set here: the frames above reload config from the harness profile.
        app.config.confirm_close = Some(true);
        let ctx = app.ctx.clone();
        app.dispatch_host_action(crate::host::keys::Action::ClosePane, &ctx);
    });
    h.run_steps(3);
    h.save_screenshot(&evidence_png!("")).unwrap();
    assert_eq!(
        h.with_app(|app| app.close_confirm_target.clone().map(|t| (t.pane, t.label))),
        Some((a, "Build log".to_string())),
        "the dialog records the pane it asked about"
    );
    assert!(
        h.host_has_label("Close \u{201c}Build log\u{201d}?"),
        "the dialog names the pane it will close"
    );
}

/// Stint 1493: open "Close pane?" over two panes, pane `a` focused.
fn open_close_dialog() -> (PlexiUiHarness, u64, u64) {
    open_close_dialog_after(|_| {})
}

/// As [`open_close_dialog`], running `before` just ahead of the dialog.
fn open_close_dialog_after(
    before: impl FnOnce(&mut PlexiUiHarness),
) -> (PlexiUiHarness, u64, u64) {
    let mut h = PlexiUiHarness::new_sized(1100.0, 640.0);
    let a = add_focused_pane(&mut h);
    let b = add_focused_pane(&mut h);
    h.with_app_mut(|app| {
        app.sidebar_visible = false;
        let context_id = app.windows[app.active_window].context_id;
        app.set_context_columns(context_id, vec![vec![a], vec![b]]);
    });
    h.run_steps(3);
    h.focus_pane(a).unwrap();
    h.run_steps(1);
    before(&mut h);
    h.with_app_mut(|app| {
        // Set here: the frames above reload config from the harness profile.
        app.config.confirm_close = Some(true);
        let ctx = app.ctx.clone();
        app.dispatch_host_action(crate::host::keys::Action::ClosePane, &ctx);
    });
    h.run_steps(3);
    assert!(h.with_app(|app| app.pending_close), "the dialog is open");
    (h, a, b)
}

/// Stint 1513: Close pane closes only on a fresh, real Enter: not an
/// Enter held from before the dialog, an injected Enter or click, or
/// Shift+Enter.
#[test]
fn close_pane_takes_only_a_fresh_real_enter() {
    let mut h = PlexiUiHarness::new_sized(1100.0, 640.0);
    let a = add_focused_pane(&mut h);
    let b = add_focused_pane(&mut h);
    h.with_app_mut(|app| {
        app.sidebar_visible = false;
        let context_id = app.windows[app.active_window].context_id;
        app.set_context_columns(context_id, vec![vec![a], vec![b]]);
    });
    h.run_steps(3);
    h.focus_pane(a).unwrap();
    h.run_steps(1);
    h.harness().key_down(egui::Key::Enter);
    h.run_steps(1);
    h.with_app_mut(|app| {
        app.config.confirm_close = Some(true);
        let ctx = app.ctx.clone();
        app.dispatch_host_action(crate::host::keys::Action::ClosePane, &ctx);
    });
    h.run_steps(3);
    let open = |h: &mut PlexiUiHarness| {
        h.with_app(|app| app.pending_close && app.windows[app.active_window].panes.len() == 2)
    };
    assert!(open(&mut h), "the dialog is open");
    arm_destructive(&mut h);
    h.harness().key_down(egui::Key::Enter);
    h.run_steps(3);
    assert!(open(&mut h), "a held Enter closed the pane");
    h.harness().key_up(egui::Key::Enter);
    h.run_steps(1);
    injected(&mut h, key_tap(egui::Key::Enter));
    assert!(open(&mut h), "an injected Enter closed the pane");
    let close = h.harness().get_by_label("Close").rect();
    let click = injected_click(&h, close);
    injected(&mut h, click);
    assert!(open(&mut h), "an injected click closed the pane");
    h.harness()
        .key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::Enter);
    h.run_steps(3);
    assert!(open(&mut h), "Shift+Enter closed the pane");
    h.harness().key_press(egui::Key::Enter);
    h.run_steps(3);
    assert!(
        h.with_app(
            |app| !app.pending_close && !app.windows[app.active_window].panes.contains_key(&a)
        ),
        "a fresh real Enter closes the pane"
    );
}

/// Stint 1513: context close runs Close all and Dissolve only on a fresh,
/// real press: not a held Enter, an injected key or click, or a Shift
/// variant.
#[test]
fn context_close_destructive_actions_take_only_a_fresh_real_press() {
    let mut h = PlexiUiHarness::new_sized(1100.0, 640.0);
    add_focused_pane(&mut h);
    h.harness().key_down(egui::Key::Enter);
    h.run_steps(1);
    seed_context_close_confirm(&mut h, true);
    let open = |h: &mut PlexiUiHarness| h.with_app(|app| app.pending_context_close.is_some());
    assert!(open(&mut h));
    arm_destructive(&mut h);
    h.harness().key_down(egui::Key::Enter);
    h.run_steps(3);
    assert!(open(&mut h), "a held Enter ran Close all");
    h.harness().key_up(egui::Key::Enter);
    h.run_steps(1);
    for k in [egui::Key::Enter, egui::Key::D] {
        injected(&mut h, key_tap(k));
        assert!(open(&mut h), "an injected {k:?} decided the dialog");
        h.harness().key_press_modifiers(egui::Modifiers::SHIFT, k);
        h.run_steps(3);
        assert!(open(&mut h), "Shift+{k:?} decided the dialog");
    }
    for label in ["Close all", "Dissolve"] {
        let button = h.harness().get_by_label(label).rect();
        let click = injected_click(&h, button);
        injected(&mut h, click);
        assert!(
            open(&mut h),
            "an injected click on {label} decided the dialog"
        );
    }
    h.harness().key_press(egui::Key::D);
    h.run_steps(3);
    assert!(!open(&mut h), "a fresh real D dissolves");
}

/// Stint 1513: Discard in "Unsaved changes" takes only a fresh, real D:
/// not an injected D or click, or Shift+D. (Held keys and the reload and
/// conflicted variants: `ui::dialog` adoption tests.)
#[test]
fn unsaved_changes_discard_takes_only_a_fresh_real_d() {
    let (mut h, _dir) = open_unsaved_prompt();
    let open = |h: &mut PlexiUiHarness| h.harness().query_by_label("Unsaved changes").is_some();
    arm_destructive(&mut h);
    injected(&mut h, key_tap(egui::Key::D));
    assert!(open(&mut h), "an injected D discarded");
    let discard = h.harness().get_by_label("Discard").rect();
    let click = injected_click(&h, discard);
    injected(&mut h, click);
    assert!(open(&mut h), "an injected click discarded");
    h.harness()
        .key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::D);
    h.run_steps(3);
    assert!(open(&mut h), "Shift+D discarded");
    h.harness().key_press(egui::Key::D);
    h.run_steps(3);
    assert!(!open(&mut h), "a fresh real D discards");
}

/// Stint 1513 critic: a Tab-focused destructive row ignores Enter or Space
/// with any modifier; a fresh real Enter still fires it.
fn focused_row_checks(
    h: &mut PlexiUiHarness,
    open: impl Fn(&mut PlexiUiHarness) -> bool,
    what: &str,
) {
    use egui::{Key, Modifiers};
    for (mods, k) in [
        (Modifiers::SHIFT, Key::Enter),
        (Modifiers::ALT, Key::Enter),
        (Modifiers::COMMAND, Key::Enter),
        (Modifiers::SHIFT, Key::Space),
    ] {
        h.harness().key_press_modifiers(mods, k);
        h.run_steps(3);
        assert!(open(h), "{what}: {mods:?}+{k:?} fired the focused row");
    }
    h.harness().key_press(Key::Enter);
    h.run_steps(3);
    assert!(!open(h), "{what}: a fresh real Enter fires the focused row");
}

#[test]
fn focused_discard_row_takes_only_a_fresh_real_enter() {
    let (mut h, _dir) = open_unsaved_prompt_after(|h| {
        // Held from before the prompt, so the dialog only sees repeats.
        h.harness().key_down(egui::Key::Enter);
        h.run_steps(1);
    });
    arm_destructive(&mut h);
    for _ in 0..3 {
        tab(&mut h, false);
    }
    assert_eq!(
        focused_label(&mut h, &["Save", "Cancel", "Discard"]).as_deref(),
        Some("Discard")
    );
    h.harness().key_down(egui::Key::Enter);
    h.run_steps(3);
    let open = |h: &mut PlexiUiHarness| h.harness().query_by_label("Unsaved changes").is_some();
    assert!(open(&mut h), "a held Enter discarded");
    h.harness().key_up(egui::Key::Enter);
    h.run_steps(1);
    focused_row_checks(&mut h, open, "Discard");
}

#[test]
fn focused_close_row_takes_only_a_fresh_real_enter() {
    let (mut h, a, _b) = open_close_dialog_after(|h| {
        // Held from before the dialog, so the dialog only sees repeats.
        h.harness().key_down(egui::Key::Enter);
        h.run_steps(1);
    });
    arm_destructive(&mut h);
    tab(&mut h, false);
    tab(&mut h, false);
    assert_eq!(
        focused_label(&mut h, &["Close", "Cancel", "UI"]).as_deref(),
        Some("Close")
    );
    let open = move |h: &mut PlexiUiHarness| {
        h.with_app(|app| {
            app.pending_close && app.windows[app.active_window].panes.contains_key(&a)
        })
    };
    h.harness().key_down(egui::Key::Enter);
    h.run_steps(3);
    assert!(open(&mut h), "a held Enter closed the focused row's pane");
    h.harness().key_up(egui::Key::Enter);
    h.run_steps(1);
    focused_row_checks(&mut h, open, "Close");
}

/// Stint 1513 critic: a fresh real Enter runs Close all.
#[test]
fn context_close_fresh_real_enter_closes_all() {
    let mut h = PlexiUiHarness::new_sized(1100.0, 640.0);
    seed_context_close_confirm(&mut h, true);
    assert!(h.with_app(|app| app.pending_context_close.is_some()));
    arm_destructive(&mut h);
    h.harness().key_press(egui::Key::Enter);
    h.run_steps(3);
    assert!(
        h.with_app(|app| app.pending_context_close.is_none()),
        "a fresh real Enter runs Close all"
    );
}

/// Stint 1513 critic: a fresh real Q selects Quit without saving. Checked on
/// the dialog itself: in the harness the quit would end the test process.
#[test]
fn quit_save_failure_dialog_takes_a_fresh_real_q() {
    let ctx = egui::Context::default();
    crate::ui::theme::setup_fonts(&ctx);
    let colors = crate::ui::theme::Colors::from_config(
        &crate::ui::theme::preset_colors("plexi-dusk").unwrap(),
    );
    let actions = crate::overlays::confirmations::quit_save_failure_actions();
    let mut selected = None;
    for stage in 0..3 {
        let mut input = egui::RawInput::default();
        if stage > 0 {
            input.events.push(egui::Event::Key {
                key: egui::Key::Q,
                physical_key: None,
                pressed: stage == 1,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            });
        }
        let _ = ctx.run_ui(input, |_| {
            let response = crate::overlays::confirmations::quit_save_failure_modal(&actions)
                .destructive_input(false, "quit_save_failure")
                .show(&ctx, &colors, |_| {});
            crate::ui::dialog::arm_destructive_dialogs_for_test(&ctx);
            selected = selected.or(response.selected);
        });
    }
    assert_eq!(selected, Some("quit_without_saving"));
}

/// Whether the action named `label` is drawn disabled on the last frame.
fn action_inactive(h: &PlexiUiHarness, label: &str) -> bool {
    h.with_app(|app| {
        crate::ui::layout_report::snapshot(&app.ctx)
            .iter()
            .flat_map(|report| report.actions.iter())
            .find(|action| action.name == label)
            .unwrap_or_else(|| panic!("no action named {label}"))
            .inactive
    })
}

/// Keep input time fixed across the kittest frames that deliver a key tap.
fn freeze_arm_delay_clock(h: &mut PlexiUiHarness) {
    let time = h.with_app(|app| {
        crate::ui::dialog::use_destructive_frame_clock_for_test(&app.ctx);
        app.ctx.input(|i| i.time)
    });
    h.harness().input_mut().time = Some(time);
}

/// Stint 1513, ruling 109: Close is greyed when the dialog first draws, and
/// an Enter typed ahead into it does nothing. Cancel is available at once.
#[test]
fn close_pane_is_disabled_during_the_arm_delay() {
    let (mut h, a, _b) = open_close_dialog_after(freeze_arm_delay_clock);
    assert!(action_inactive(&h, "Close"), "Close is greyed at first");
    assert!(!action_inactive(&h, "Cancel"), "Cancel is never delayed");
    h.save_screenshot(&evidence_png!()).unwrap();
    h.harness().key_press(egui::Key::Enter);
    h.run_steps(3);
    assert!(
        h.with_app(|app| app.pending_close
            && app.windows[app.active_window].panes.contains_key(&a)),
        "an Enter during the delay closed the pane"
    );
}

/// Stint 1513, ruling 109: Close all and Dissolve wait out the delay.
#[test]
fn context_close_is_disabled_during_the_arm_delay() {
    let mut h = PlexiUiHarness::new_sized(1100.0, 640.0);
    add_focused_pane(&mut h);
    h.step();
    freeze_arm_delay_clock(&mut h);
    seed_context_close_confirm(&mut h, true);
    h.run_steps(2);
    assert!(action_inactive(&h, "Close all"));
    assert!(action_inactive(&h, "Dissolve"));
    assert!(!action_inactive(&h, "Cancel"), "Cancel is never delayed");
    h.save_screenshot(&evidence_png!()).unwrap();
    for key in [egui::Key::Enter, egui::Key::D] {
        h.harness().key_press(key);
        h.run_steps(3);
        assert!(
            h.with_app(|app| app.pending_context_close.is_some()),
            "{key:?} during the delay decided the dialog"
        );
    }

    let before = h.with_app(|app| app.ctx.input(|i| i.time));
    h.harness().input_mut().time = Some(before + crate::broker::consent::ARM_DELAY.as_secs_f64());
    h.harness().key_press(egui::Key::D);
    h.run_steps(3);
    assert!(
        h.with_app(|app| app.pending_context_close.is_none()),
        "D after the delay did not decide the dialog"
    );
}

/// Stint 1513, ruling 109: Discard is greyed at first; Save and Cancel are not.
#[test]
fn unsaved_changes_discard_is_disabled_during_the_arm_delay() {
    let (mut h, _dir) = open_unsaved_prompt_after(freeze_arm_delay_clock);
    assert!(action_inactive(&h, "Discard"));
    assert!(!action_inactive(&h, "Save"), "Save is never delayed");
    assert!(!action_inactive(&h, "Cancel"), "Cancel is never delayed");
    h.save_screenshot(&evidence_png!()).unwrap();
    h.harness().key_press(egui::Key::D);
    h.run_steps(3);
    assert!(
        h.harness().query_by_label("Unsaved changes").is_some(),
        "a D during the delay discarded"
    );
}

/// One frame of a dialog at fixed input time, with one key tapped when `tap`.
fn dialog_frame<T>(
    ctx: &egui::Context,
    tap: Option<(egui::Key, egui::Modifiers)>,
    show: impl FnOnce(&egui::Context, &crate::ui::theme::Colors) -> T,
) -> T {
    let colors = crate::ui::theme::Colors::from_config(
        &crate::ui::theme::preset_colors("plexi-dusk").unwrap(),
    );
    crate::ui::dialog::use_destructive_frame_clock_for_test(ctx);
    let mut input = egui::RawInput {
        time: Some(0.0),
        ..Default::default()
    };
    if let Some((key, modifiers)) = tap {
        input.modifiers = modifiers;
        input.events = [true, false]
            .map(|pressed| egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers,
            })
            .into();
    }
    let mut show = Some(show);
    let mut out = None;
    let _ = ctx.run_ui(input, |_| {
        if let Some(show) = show.take() {
            out = Some(show(ctx, &colors));
        }
    });
    out.unwrap()
}

/// Stint 1513, ruling 109: Discard, Reload and Overwrite each ignore a real
/// press during the delay. Save stays instant on the plain dialog.
#[test]
fn unsaved_changes_variants_ignore_a_press_during_the_arm_delay() {
    use egui::{Key, Modifiers};
    for (conflicted, reload, key, modifiers) in [
        (false, false, Key::D, Modifiers::NONE),
        (false, false, Key::D, Modifiers::COMMAND),
        (false, true, Key::R, Modifiers::NONE),
        (true, false, Key::Enter, Modifiers::NONE),
        (true, false, Key::S, Modifiers::COMMAND),
    ] {
        let ctx = egui::Context::default();
        crate::ui::theme::setup_fonts(&ctx);
        let dialog = crate::ui::dialog::UnsavedChangesDialog {
            path: "/tmp/draft.txt".to_string(),
            conflicted,
            reload,
            error: None,
        };
        dialog_frame(&ctx, None, |ctx, colors| dialog.show(ctx, colors, false));
        assert_eq!(
            dialog_frame(&ctx, Some((key, modifiers)), |ctx, colors| dialog
                .show(ctx, colors, false)),
            None,
            "{key:?} {modifiers:?} conflicted={conflicted} reload={reload}"
        );
    }
    let ctx = egui::Context::default();
    crate::ui::theme::setup_fonts(&ctx);
    let dialog = crate::ui::dialog::UnsavedChangesDialog {
        path: "/tmp/draft.txt".to_string(),
        conflicted: false,
        reload: false,
        error: None,
    };
    dialog_frame(&ctx, None, |ctx, colors| dialog.show(ctx, colors, false));
    assert_eq!(
        dialog_frame(&ctx, Some((Key::S, Modifiers::COMMAND)), |ctx, colors| {
            dialog.show(ctx, colors, false)
        }),
        Some(crate::ui::dialog::UnsavedChangesChoice::Save),
        "Save is never delayed"
    );
}

/// Stint 1513, ruling 109: Quit without saving ignores a Q during the delay.
/// Checked on the dialog itself: in the harness the quit would end the test.
#[test]
fn quit_without_saving_ignores_a_press_during_the_arm_delay() {
    let ctx = egui::Context::default();
    crate::ui::theme::setup_fonts(&ctx);
    let actions = crate::overlays::confirmations::quit_save_failure_actions();
    let show = |ctx: &egui::Context, colors: &crate::ui::theme::Colors| {
        crate::overlays::confirmations::quit_save_failure_modal(&actions)
            .destructive_input(false, "quit_save_failure")
            .show(ctx, colors, |_| {})
            .selected
    };
    dialog_frame(&ctx, None, show);
    assert_eq!(
        dialog_frame(&ctx, Some((egui::Key::Q, egui::Modifiers::NONE)), show),
        None,
        "a Q during the delay quit without saving"
    );
}

/// Stint 1514: the trap order is the one Tab order. In "Unsaved changes",
/// Tab past the last action (Discard) wraps to the first (Save), and
/// Shift+Tab from the first goes to the last.
#[test]
fn unsaved_changes_tab_wraps_both_ways_in_trap_order() {
    let labels = ["Save", "Cancel", "Discard"];
    let (mut h, _dir) = open_unsaved_prompt();
    arm_destructive(&mut h);
    let mut seen = Vec::new();
    for _ in 0..4 {
        tab(&mut h, false);
        seen.push(focused_label(&mut h, &labels));
    }
    assert_eq!(
        seen,
        ["Save", "Cancel", "Discard", "Save"].map(|l| Some(l.to_string())),
        "Tab past Discard wraps to Save"
    );
    tab(&mut h, true);
    assert_eq!(
        focused_label(&mut h, &labels).as_deref(),
        Some("Discard"),
        "Shift+Tab from Save goes to Discard"
    );
    tab(&mut h, true);
    assert_eq!(focused_label(&mut h, &labels).as_deref(), Some("Cancel"));
}

/// Stint 1514: context close with Dissolve showing enters at Cancel, steps
/// Close all then Dissolve, and wraps both ways.
#[test]
fn context_close_with_dissolve_tabs_in_trap_order() {
    let labels = ["Cancel", "Close all", "Dissolve"];
    let mut h = PlexiUiHarness::new_sized(1100.0, 640.0);
    seed_context_close_confirm(&mut h, true);
    h.harness().get_by_label("Dissolve");
    arm_destructive(&mut h);
    let mut seen = Vec::new();
    for _ in 0..4 {
        tab(&mut h, false);
        seen.push(focused_label(&mut h, &labels));
    }
    assert_eq!(
        seen,
        ["Cancel", "Close all", "Dissolve", "Cancel"].map(|l| Some(l.to_string())),
        "Tab order and wrap"
    );
    tab(&mut h, true);
    assert_eq!(
        focused_label(&mut h, &labels).as_deref(),
        Some("Dissolve"),
        "Shift+Tab from Cancel goes to Dissolve"
    );
    assert!(h.with_app(|app| app.pending_context_close.is_some()));
}

/// A dirty text editor with its "Unsaved changes" prompt open.
fn open_unsaved_prompt() -> (PlexiUiHarness, tempfile::TempDir) {
    open_unsaved_prompt_after(|_| {})
}

/// As [`open_unsaved_prompt`], running `before` ahead of the prompt.
fn open_unsaved_prompt_after(
    before: impl FnOnce(&mut PlexiUiHarness),
) -> (PlexiUiHarness, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("draft.txt");
    std::fs::write(&path, "fn main() {}\n").unwrap();
    let mut h = PlexiUiHarness::new_sized(900.0, 320.0);
    h.step();
    before(&mut h);
    let pane_id = h.with_app_mut(|app| {
        let pane_id = app.host.alloc_pane_id();
        let view_pane = ViewPane {
            incarnation: Default::default(),
            pip_status: None,
            id: pane_id,
            runtime: ViewRuntime::Builtin(Box::new(
                crate::app::text_editor_view::TextEditorView::new(path.clone()),
            )),
            workspace_root: crate::testing::scratch_context_root("ui"),
            permissions: PluginPermissions::builtin(),
            manifest_id: "text-editor".to_string(),
            name: "draft.txt".to_string(),
            pane_group: None,
            linked_pane_id: None,
            overlay_replaced: None,
            agent: None,
            slots: std::collections::HashMap::new(),
            semantic_state: Default::default(),
        };
        let win = &mut app.windows[app.active_window];
        win.panes
            .insert(pane_id, Pane::View(Box::new(view_pane)))
            .unwrap();
        let tile_id = win.tree.tiles.insert_pane(pane_id);
        if win.tree.root.is_none() {
            win.tree.root = Some(tile_id);
        }
        win.focused_pane = Some(tile_id);
        pane_id
    });
    h.run_steps(2);
    h.with_app_mut(|app| {
        let win = &mut app.windows[app.active_window];
        if let Some(Pane::View(a)) = win.panes.get_mut(&pane_id) {
            if let ViewRuntime::Builtin(b) = &mut a.runtime {
                b.as_any_mut()
                    .downcast_mut::<crate::app::text_editor_view::TextEditorView>()
                    .expect("text editor")
                    .test_set_unsaved("edited\n");
            }
        }
    });
    h.run_steps(2);
    h.with_app_mut(|app| assert!(app.focused_view_vetoes_close()));
    h.run_steps(2);
    h.harness().get_by_label("Unsaved changes");
    (h, dir)
}

/// Stint 1505: keyboard focus enters "Unsaved changes" at Save (its safe
/// default) from either direction; Discard is last, three Tabs away.
#[test]
fn unsaved_changes_tab_order_is_save_cancel_then_discard() {
    let labels = ["Save", "Cancel", "Discard"];
    let (mut h, _dir) = open_unsaved_prompt();
    arm_destructive(&mut h);
    h.save_screenshot(&evidence_png!()).unwrap();
    let mut seen = Vec::new();
    for _ in 0..3 {
        tab(&mut h, false);
        seen.push(focused_label(&mut h, &labels));
    }
    assert_eq!(
        seen,
        vec![
            Some("Save".to_string()),
            Some("Cancel".to_string()),
            Some("Discard".to_string()),
        ],
        "Save first, Discard last"
    );

    let (mut h, _dir) = open_unsaved_prompt();
    arm_destructive(&mut h);
    tab(&mut h, true);
    assert_eq!(
        focused_label(&mut h, &labels).as_deref(),
        Some("Save"),
        "Shift+Tab also enters at Save"
    );
}

/// Stint 1493: Tab and Shift+Tab cycle the dialog's own actions and never
/// reach the toolbar behind it; Enter on a Tab-focused Cancel cancels.
/// Stint 1505: focus enters at Cancel, the safe action.
#[test]
fn tab_in_close_dialog_cycles_its_actions_and_enter_activates_cancel() {
    let (mut h, a, b) = open_close_dialog();
    arm_destructive(&mut h);
    let labels = ["Close", "Cancel", "UI"];
    let mut seen = Vec::new();
    for _ in 0..4 {
        tab(&mut h, false);
        seen.push(focused_label(&mut h, &labels));
    }
    assert_eq!(
        seen,
        vec![
            Some("Cancel".to_string()),
            Some("Close".to_string()),
            Some("Cancel".to_string()),
            Some("Close".to_string()),
        ],
        "Tab enters at Cancel, cycles Close and Cancel and wraps"
    );
    tab(&mut h, true);
    assert_eq!(focused_label(&mut h, &labels).as_deref(), Some("Cancel"));
    tab(&mut h, true);
    assert_eq!(
        focused_label(&mut h, &labels).as_deref(),
        Some("Close"),
        "Shift+Tab wraps backwards"
    );
    tab(&mut h, true);
    assert_eq!(focused_label(&mut h, &labels).as_deref(), Some("Cancel"));
    h.save_screenshot(&evidence_png!("_cancel_focused"))
        .unwrap();

    h.harness().key_press(egui::Key::Enter);
    h.run_steps(3);
    assert!(
        !h.with_app(|app| app.pending_close),
        "Cancel closed the dialog"
    );
    let open: Vec<u64> = h.with_app(|app| {
        let mut v: Vec<u64> = app.windows[app.active_window]
            .panes
            .keys()
            .copied()
            .collect();
        v.sort_unstable();
        v
    });
    let mut both = vec![a, b];
    both.sort_unstable();
    assert_eq!(open, both, "Enter on Cancel closes nothing");
}

/// Stint 1558 review: a Tab-focused Cancel keeps keyboard focus when the
/// close dialog reflows from a row to a stack and back (a narrow window or
/// a raised UI scale), so Enter still cancels instead of firing Close's
/// Enter shortcut. Both directions: focused in the row, then in the stack.
#[test]
fn close_dialog_cancel_keeps_focus_across_row_and_stack_reflow() {
    const WIDE: f32 = 1100.0;
    const NARROW: f32 = 170.0;
    for focus_while_stacked in [false, true] {
        let (mut h, a, b) = open_close_dialog();
        if focus_while_stacked {
            assert!(resize_settle(&mut h, NARROW), "narrow window stacks");
        }
        tab(&mut h, false);
        let labels = ["Close", "Cancel", "UI"];
        assert_eq!(focused_label(&mut h, &labels).as_deref(), Some("Cancel"));
        for (width, stacks) in [(NARROW, true), (WIDE, false), (NARROW, true)] {
            assert_eq!(resize_settle(&mut h, width), stacks, "width {width}");
            assert_eq!(
                focused_label(&mut h, &labels).as_deref(),
                Some("Cancel"),
                "focus after reflow to width {width} (focused while stacked: {focus_while_stacked})"
            );
        }
        resize_settle(&mut h, if focus_while_stacked { WIDE } else { NARROW });
        h.harness().key_press(egui::Key::Enter);
        h.run_steps(3);
        assert!(
            !h.with_app(|app| app.pending_close),
            "Enter left the dialog open"
        );
        let mut open: Vec<u64> = h.with_app(|app| {
            app.windows[app.active_window]
                .panes
                .keys()
                .copied()
                .collect()
        });
        open.sort_unstable();
        let mut both = vec![a, b];
        both.sort_unstable();
        assert_eq!(open, both, "Enter after reflow closed a pane");
    }
}

/// Stint 1493: Enter on a Tab-focused Close closes the pane, and Esc still
/// dismisses an untouched dialog. Stint 1505: Close is the second Tab.
#[test]
fn enter_on_tab_focused_close_closes_and_esc_still_cancels() {
    let (mut h, a, b) = open_close_dialog();
    arm_destructive(&mut h);
    tab(&mut h, false);
    tab(&mut h, false);
    assert_eq!(
        focused_label(&mut h, &["Close", "Cancel", "UI"]).as_deref(),
        Some("Close")
    );
    h.harness().key_press(egui::Key::Enter);
    h.run_steps(3);
    let open: Vec<u64> = h.with_app(|app| {
        app.windows[app.active_window]
            .panes
            .keys()
            .copied()
            .collect()
    });
    assert_eq!(open, vec![b], "Close closed pane {a}");

    let (mut h, _, _) = open_close_dialog();
    h.harness().key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(!h.with_app(|app| app.pending_close), "Esc cancels");
}

// ── Text-entry dialogs on the kit (stint 1559) ─────────────────────────────

/// The four text-entry dialogs, each opened with `buffer` in its field.
#[derive(Clone, Copy, Debug)]
enum TextDialog {
    RenamePane,
    RenameContext,
    Description,
    ContextRoot,
}

impl TextDialog {
    const ALL: [Self; 4] = [
        Self::RenamePane,
        Self::RenameContext,
        Self::Description,
        Self::ContextRoot,
    ];

    fn commit_label(self) -> &'static str {
        match self {
            Self::RenamePane | Self::RenameContext => "Rename",
            Self::Description => "Save",
            Self::ContextRoot => "Set root",
        }
    }

    fn commit_key(self) -> (egui::Modifiers, egui::Key) {
        match self {
            Self::Description => (egui::Modifiers::COMMAND, egui::Key::Enter),
            _ => (egui::Modifiers::NONE, egui::Key::Enter),
        }
    }

    fn open(self, h: &mut PlexiUiHarness, buffer: &str) {
        h.with_app_mut(|app| match self {
            Self::RenamePane => {
                app.open_rename_for_focused();
                app.rename_buffer = buffer.to_string();
            }
            Self::RenameContext => {
                app.open_context_rename(0);
                app.context_rename.as_mut().unwrap().buffer = buffer.to_string();
            }
            Self::Description => {
                app.open_description_edit(0);
                app.description_buffer = buffer.to_string();
            }
            Self::ContextRoot => {
                app.text_overlay = Some((
                    crate::app::TextInputOverlay {
                        label: "Set context root".to_string(),
                        hint: "/path/to/project or ~/...".to_string(),
                        buffer: buffer.to_string(),
                        session: crate::ui::text_field::OpenSession::new(),
                    },
                    crate::app::OverlayTarget::ContextRoot(app.router.active().context_id),
                ));
            }
        });
        h.run_steps(2);
    }

    fn is_open(self, h: &mut PlexiUiHarness) -> bool {
        h.with_app(|app| match self {
            Self::RenamePane => app.renaming_pane.is_some(),
            Self::RenameContext => app.context_rename.is_some(),
            Self::Description => app.editing_description.is_some(),
            Self::ContextRoot => app.text_overlay.is_some(),
        })
    }

    /// The text in the open dialog's field.
    fn buffer(self, h: &mut PlexiUiHarness) -> String {
        h.with_app(|app| match self {
            Self::RenamePane => app.rename_buffer.clone(),
            Self::RenameContext => app.context_rename.as_ref().unwrap().buffer.clone(),
            Self::Description => app.description_buffer.clone(),
            Self::ContextRoot => app.text_overlay.as_ref().unwrap().0.buffer.clone(),
        })
    }

    /// The value the dialog writes on commit.
    fn committed(self, h: &mut PlexiUiHarness, pane: crate::host::model::PaneId) -> String {
        h.with_app(|app| match self {
            Self::RenamePane => app.windows[app.active_window]
                .panes
                .get(&pane)
                .and_then(|p| p.as_view())
                .map(|v| v.name.clone())
                .unwrap_or_default(),
            Self::RenameContext => app.router.get(0).name.to_string(),
            Self::Description => app.router.get(0).description.clone().unwrap_or_default(),
            Self::ContextRoot => app.router.get(0).root.display().to_string(),
        })
    }
}

/// Stint 1559: every text-entry dialog shows "Cancel" and its commit
/// button, and the buttons do what the keys do: Enter (Cmd+Enter for the
/// description) and the commit button commit; Esc and Cancel cancel.
#[test]
fn text_entry_dialogs_commit_and_cancel_by_key_and_button() {
    for dialog in TextDialog::ALL {
        let mut h = PlexiUiHarness::new();
        h.step();
        let pane = add_focused_pane(&mut h);
        h.step();
        let root = h.workspace.path().to_path_buf();
        let value = |n: usize| match dialog {
            TextDialog::ContextRoot => {
                let dir = root.join(format!("root{n}"));
                std::fs::create_dir_all(&dir).unwrap();
                dir.display().to_string()
            }
            _ => format!("value {n}"),
        };
        for (n, by_button) in [(1, false), (2, true)] {
            let wanted = value(n);
            dialog.open(&mut h, &wanted);
            if by_button {
                h.harness().get_by_label(dialog.commit_label()).click();
            } else {
                let (mods, key) = dialog.commit_key();
                h.harness().key_press_modifiers(mods, key);
            }
            h.run_steps(2);
            assert!(
                !dialog.is_open(&mut h),
                "{dialog:?} commit by_button={by_button}"
            );
            assert_eq!(
                dialog.committed(&mut h, pane),
                wanted,
                "{dialog:?} by_button={by_button}"
            );

            dialog.open(&mut h, &value(n + 10));
            if by_button {
                h.harness().get_by_label("Cancel").click();
            } else {
                h.harness().key_press(egui::Key::Escape);
            }
            h.run_steps(2);
            assert!(
                !dialog.is_open(&mut h),
                "{dialog:?} cancel by_button={by_button}"
            );
            assert_eq!(
                dialog.committed(&mut h, pane),
                wanted,
                "{dialog:?} cancel by_button={by_button} must not commit"
            );
        }
    }
}

/// Stint 1559: Tab walks field, commit button, Cancel, and wraps to the
/// field; Enter on a Tab-focused Cancel cancels instead of committing.
#[test]
fn rename_pane_tab_walks_field_then_buttons_and_enter_on_cancel_cancels() {
    let mut h = PlexiUiHarness::new();
    h.step();
    let pane = add_focused_pane(&mut h);
    h.step();
    let before = TextDialog::RenamePane.committed(&mut h, pane);
    TextDialog::RenamePane.open(&mut h, "never committed");
    let field = egui::Id::new("rename_pane_input");
    let focused = |h: &mut PlexiUiHarness| h.with_app(|app| app.ctx.memory(|m| m.focused()));
    assert_eq!(
        focused(&mut h),
        Some(field),
        "the field takes focus on open"
    );
    let labels = ["Rename", "Cancel"];
    tab(&mut h, false);
    assert_eq!(focused_label(&mut h, &labels).as_deref(), Some("Rename"));
    tab(&mut h, false);
    assert_eq!(focused_label(&mut h, &labels).as_deref(), Some("Cancel"));
    tab(&mut h, false);
    assert_eq!(focused(&mut h), Some(field), "Tab wraps back to the field");
    tab(&mut h, true);
    assert_eq!(focused_label(&mut h, &labels).as_deref(), Some("Cancel"));
    h.harness().key_press(egui::Key::Enter);
    h.run_steps(2);
    assert!(!TextDialog::RenamePane.is_open(&mut h));
    assert_eq!(TextDialog::RenamePane.committed(&mut h, pane), before);
}

/// Stint 1559 review: the field selects all only when the dialog opens.
/// Edit the draft, Tab to a button, click back into the text and type:
/// the draft keeps its edit and gains the new text. Reopening selects
/// the text again.
#[test]
fn text_entry_field_keeps_the_draft_when_refocused_from_a_button() {
    for dialog in [TextDialog::RenamePane, TextDialog::Description] {
        let mut h = PlexiUiHarness::new();
        h.step();
        add_focused_pane(&mut h);
        h.step();
        dialog.open(&mut h, "draft");
        let typed = |h: &mut PlexiUiHarness, text: &str| {
            h.harness()
                .input_mut()
                .events
                .push(egui::Event::Text(text.to_string()));
            h.step();
        };
        let click_field = |h: &mut PlexiUiHarness, value: &str| {
            h.harness()
                .query_all_by(|node| {
                    matches!(
                        node.role(),
                        egui::accesskit::Role::TextInput
                            | egui::accesskit::Role::MultilineTextInput
                    ) && node.value().as_deref() == Some(value)
                })
                .next()
                .expect("the dialog's field")
                .click();
            h.run_steps(2);
        };
        // Opening selects the draft; a click places the caret.
        click_field(&mut h, "draft");
        typed(&mut h, " one");
        assert_eq!(dialog.buffer(&mut h), "draft one", "{dialog:?}");
        tab(&mut h, false);
        h.run_steps(2);
        click_field(&mut h, "draft one");
        typed(&mut h, " two");
        assert_eq!(
            dialog.buffer(&mut h),
            "draft one two",
            "{dialog:?}: refocusing the field replaced the draft"
        );
        // A reopened dialog selects its text again.
        h.harness().key_press(egui::Key::Escape);
        h.run_steps(2);
        dialog.open(&mut h, "again");
        typed(&mut h, "fresh");
        assert_eq!(dialog.buffer(&mut h), "fresh", "{dialog:?} reopen");
    }
}

/// Stint 1559 review: a text dialog covered by another overlay is hidden,
/// not closed. Edit the draft, cover it, uncover it and type: the draft
/// keeps its edit. Reopening still selects the text.
#[test]
fn text_entry_field_keeps_the_draft_when_uncovered() {
    for dialog in TextDialog::ALL {
        let mut h = PlexiUiHarness::new();
        h.step();
        add_focused_pane(&mut h);
        h.step();
        dialog.open(&mut h, "draft");
        let typed = |h: &mut PlexiUiHarness, text: &str| {
            h.harness()
                .input_mut()
                .events
                .push(egui::Event::Text(text.to_string()));
            h.step();
        };
        h.harness().key_press(egui::Key::End);
        h.step();
        typed(&mut h, " one");
        assert_eq!(dialog.buffer(&mut h), "draft one", "{dialog:?}");
        h.with_app_mut(|app| {
            app.show_shortcuts = true;
            app.sync_shortcuts_focus();
        });
        h.run_steps(2);
        h.with_app_mut(|app| {
            app.show_shortcuts = false;
            app.sync_shortcuts_focus();
        });
        h.run_steps(2);
        typed(&mut h, " two");
        assert_eq!(
            dialog.buffer(&mut h),
            "draft one two",
            "{dialog:?}: uncovering the dialog replaced the draft"
        );
        h.harness().key_press(egui::Key::Escape);
        h.run_steps(2);
        dialog.open(&mut h, "again");
        typed(&mut h, "fresh");
        assert_eq!(dialog.buffer(&mut h), "fresh", "{dialog:?} reopen");
    }
}

/// Visual review (stint 1559): each text-entry dialog is an `ActionModal`
/// with a field body and a "Cancel Esc" / commit button row, at a roomy
/// and a narrow window.
#[test]
fn screenshot_text_entry_dialogs() {
    for width in [900.0, 360.0] {
        for dialog in TextDialog::ALL {
            let mut h = PlexiUiHarness::new_sized(width, 420.0);
            h.step();
            add_focused_pane(&mut h);
            h.step();
            dialog.open(&mut h, "notes-scratch");
            h.run_steps(2);
            h.harness().get_by_label(dialog.commit_label());
            h.harness().get_by_label("Cancel");
            assert!(
                has_text_field_holding(&mut h, "notes-scratch"),
                "{dialog:?}"
            );
            assert_layout_surface(&h, "modal_action");
            h.save_screenshot(&evidence_png!(format!("_{dialog:?}_{width}")))
                .expect("render failed");
        }
    }
}

// ── Rename flow tests (require a real egui frame to process key events) ────

/// Full rename flow: set up rename state → press Enter → verify commit.
///
/// Visual review for the shared single-line rename modal. The pane and
/// context rename overlays are one helper, so the pixels are what prove
/// the extraction kept the top-hung scrim-less shell, its anchor, and the
/// select-all-on-focus text field.
#[test]
fn screenshot_rename_pane_overlay() {
    let mut h = PlexiUiHarness::new();
    h.step();
    let pane_id = add_focused_pane(&mut h);
    h.step();
    h.with_app_mut(|app| {
        app.rename_buffer = "notes-scratch".to_string();
        app.renaming_pane = Some(pane_id);
        app.focus_stack.open(FocusKind::RenamePane, None);
    });
    h.step();
    // The shared single-line rename field is up and holds the buffer.
    assert!(
        has_text_field_holding(&mut h, "notes-scratch"),
        "pane rename overlay must show its text field"
    );
    h.save_screenshot(&evidence_png!()).expect("render failed");
}

/// The rename commit happens inside `draw_rename_context_overlay`, which reads
/// Enter from `ctx.input_mut()` during the egui draw pass. This cannot be
/// tested with HostHarness — it requires PlexiUiHarness + a real frame.
#[test]
fn context_rename_commits_on_enter() {
    let mut h = PlexiUiHarness::new();
    h.step();

    // Simulate what Action::RenameContext does: populate rename state and push focus layer.
    h.with_app_mut(|app| {
        let ctx_idx = app.router.active_idx();
        app.open_context_rename(ctx_idx);
        app.context_rename.as_mut().unwrap().buffer = "My Project".to_string();
    });

    // Queue Enter for the next frame — processed by draw_rename_context_overlay.
    h.harness().key_press(egui::Key::Enter);
    h.step();

    let (name, still_renaming) = h.with_app(|app| {
        (
            app.router.active().name.clone(),
            app.context_rename.is_some(),
        )
    });
    assert_eq!(name, "My Project");
    assert!(
        !still_renaming,
        "context rename must be cleared after commit"
    );
}

#[test]
fn screenshot_context_1041_rename_with_sidebar() {
    let mut h = PlexiUiHarness::new_sized(1000.0, 700.0);
    h.with_app_mut(|app| {
        app.sidebar_visible = true;
        app.rename_context(0, "Project workspace");
        app.open_context_rename(0);
    });
    h.run_steps(3);
    assert!(
        has_text_field_holding(&mut h, "Project workspace"),
        "context rename overlay must show its text field beside the sidebar"
    );
    h.save_screenshot(&evidence_png!())
        .expect("render context rename");
}

/// Escape discards the rename buffer; the context name must be unchanged.
#[test]
fn context_rename_cancels_on_escape() {
    let mut h = PlexiUiHarness::new();
    h.step();

    let original_name = h.with_app(|app| app.router.active().name.clone());

    h.with_app_mut(|app| {
        let ctx_idx = app.router.active_idx();
        app.open_context_rename(ctx_idx);
        app.context_rename.as_mut().unwrap().buffer = "Discarded Name".to_string();
    });

    h.harness().key_press(egui::Key::Escape);
    h.step();

    let (name, still_renaming) = h.with_app(|app| {
        (
            app.router.active().name.clone(),
            app.context_rename.is_some(),
        )
    });
    assert_eq!(name, original_name, "name must be unchanged after Escape");
    assert!(
        !still_renaming,
        "context rename must be cleared after Escape"
    );
}

/// Empty rename buffer returns the context to an auto-derived name.
#[test]
fn context_rename_empty_buffer_reverts_to_auto() {
    let mut h = PlexiUiHarness::new();
    h.step();

    h.with_app_mut(|app| {
        let ctx_idx = app.router.active_idx();
        app.open_context_rename(ctx_idx);
        app.context_rename.as_mut().unwrap().buffer = "   ".to_string();
    });

    h.harness().key_press(egui::Key::Enter);
    h.step();

    let name = h.with_app(|app| app.router.active().name.clone());
    assert!(
        matches!(name, crate::host::context::ContextName::Auto(ref value) if !value.is_empty()),
        "whitespace-only rename must restore a non-empty automatic name"
    );
}

/// Context rename receives text while a terminal is focused.
#[test]
fn context_rename_receives_text_with_focused_terminal() {
    let mut h = PlexiUiHarness::new_sized(1000.0, 700.0);
    h.step();

    // Need a real focused terminal pane. Seed a view pane, then split — the
    // split spawns a real TerminalPane and focuses it.
    add_focused_pane(&mut h);
    h.step();
    h.with_app_mut(|app| app.split_focused(true, None, false, false, None));
    h.run_steps(2);

    let focused_is_terminal = h.with_app(|app| {
        let win = &app.windows[app.active_window];
        win.focused_pane
            .and_then(|t| match win.tree.tiles.get(t) {
                Some(egui_tiles::Tile::Pane(pid)) => win.panes.get(pid),
                _ => None,
            })
            .map(|p| matches!(p, Pane::Terminal(_)))
            .unwrap_or(false)
    });
    assert!(
        focused_is_terminal,
        "focused pane must be a real terminal for this regression to be meaningful"
    );

    // Open the context rename overlay with the sidebar visible.
    h.with_app_mut(|app| {
        app.sidebar_visible = true;
        let ctx_idx = app.router.active_idx();
        app.open_context_rename(ctx_idx);
        app.context_rename.as_mut().unwrap().buffer.clear();
    });
    // Let the field mount and gain focus.
    h.run_steps(3);

    // Type into the rename box.
    h.harness()
        .input_mut()
        .events
        .push(egui::Event::Text("x".to_string()));
    h.step();
    h.harness()
        .input_mut()
        .events
        .push(egui::Event::Text("y".to_string()));
    h.step();

    let buffer = h.with_app(|app| app.context_rename.as_ref().unwrap().buffer.clone());
    assert_eq!(
        buffer, "xy",
        "context rename field must receive typed characters even while a terminal is the focused tile"
    );
}
