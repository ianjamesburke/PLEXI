//! kittest passes reach `eframe::App::raw_input_hook`, as eframe's do.
//!
//! eframe hands every delivery to the hook before the egui pass
//! (`EpiIntegration::update`). Upstream egui_kittest skips it, so the vendored
//! copy (`deps/egui_kittest-0.34.3/PLEXI-PATCH.md`) calls it on the bootstrap
//! pass and on every step. Each step is still exactly one pass per delivery:
//! a test that needs to cut between deliveries steps, never drains.

use egui::{Event, Key, Modifiers, RawInput};

/// Records each hook call and each pass, and whether the pass saw the hook's
/// marker event.
#[derive(Default)]
struct HookRecorder {
    log: Vec<&'static str>,
}

const MARKER: &str = "from-hook";

impl eframe::App for HookRecorder {
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut RawInput) {
        self.log.push("hook");
        raw_input.events.push(Event::Text(MARKER.to_string()));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let hooked = ui.input(|i| {
            i.events
                .iter()
                .any(|e| matches!(e, Event::Text(t) if t == MARKER))
        });
        self.log
            .push(if hooked { "hooked pass" } else { "bare pass" });
    }
}

/// The bootstrap pass and every explicit step call the hook exactly once,
/// before the pass, and the pass sees what the hook added.
#[test]
fn kittest_passes_go_through_raw_input_hook() {
    let mut h = egui_kittest::Harness::new_eframe(|_| HookRecorder::default());
    let log = &h.state().log;
    assert_eq!(
        log.first().copied(),
        Some("hook"),
        "bootstrap pass skipped the hook: {log:?}"
    );
    assert!(
        log.chunks(2).all(|pass| pass == ["hook", "hooked pass"]),
        "construction passes: {log:?}"
    );

    h.state_mut().log.clear();
    h.step();
    assert_eq!(h.state().log, ["hook", "hooked pass"], "one explicit step");

    h.state_mut().log.clear();
    h.key_press(egui::Key::A);
    h.step();
    assert_eq!(
        h.state().log,
        ["hook", "hooked pass", "hook", "hooked pass"],
        "one hooked pass per queued event"
    );
}

/// A focused file browser with `batch` queued the way `plexi pane key`
/// queues it; only `PlexiApp::raw_input_hook` delivers it.
fn harness_with_queued_pane_input(batch: RawInput) -> (super::PlexiUiHarness, u64) {
    let mut h = super::PlexiUiHarness::new_sized(1280.0, 800.0);
    h.open_file_browser(h.workspace_root().to_path_buf());
    h.run_steps(2);
    let pane = h
        .with_app(|app| app.active_focused_pane_id())
        .expect("file browser is focused");
    h.with_app_mut(|app| app.pending_pane_inputs.entry(pane).or_default().push(batch));
    (h, pane)
}

fn native_cmd() -> Modifiers {
    Modifiers {
        mac_cmd: true,
        command: true,
        ..Modifiers::NONE
    }
}

/// Production witness and negative control: a queued Cmd+P opens the command
/// palette on a hooked kittest step, and a pass run the way upstream kittest
/// ran it (logic and ui, no hook) leaves it closed and the batch queued.
#[test]
fn kittest_step_opens_the_palette_from_hook_delivered_input() {
    let cmd_p = |pressed| Event::Key {
        key: Key::P,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: native_cmd(),
    };
    let (mut h, pane) = harness_with_queued_pane_input(RawInput {
        modifiers: native_cmd(),
        events: vec![cmd_p(true), cmd_p(false)],
        ..Default::default()
    });
    let queued =
        |h: &super::PlexiUiHarness| h.with_app(|app| app.pending_pane_inputs.contains_key(&pane));
    let palette = |h: &super::PlexiUiHarness| h.with_app(|app| app.show_command_palette);

    let ctx = h.harness().ctx.clone();
    h.with_app_mut(|app| {
        use eframe::App;
        let mut frame = eframe::Frame::_new_kittest();
        let _ = ctx.run_ui(RawInput::default(), |ui| {
            app.logic(ui.ctx(), &mut frame);
            app.ui(ui, &mut frame);
        });
    });
    assert!(queued(&h), "a pass without the hook delivered the batch");
    assert!(!palette(&h), "the palette opened without the hook");

    h.step();
    assert!(!queued(&h), "the hooked step left the batch queued");
    assert!(palette(&h), "hook-delivered Cmd+P did not open the palette");
}

/// The hook sets a queued batch's modifiers on the input it edits. As in
/// eframe, that is the detached input of one pass, so the next idle step
/// sees no stale modifier.
#[test]
fn hook_modifiers_do_not_carry_into_the_next_step() {
    let (mut h, _pane) = harness_with_queued_pane_input(RawInput {
        modifiers: Modifiers::CTRL,
        ..Default::default()
    });
    let modifiers = |h: &mut super::PlexiUiHarness| h.harness().ctx.input(|i| i.modifiers);
    h.step();
    assert_eq!(modifiers(&mut h), Modifiers::CTRL, "the batch's pass");
    h.step();
    assert_eq!(modifiers(&mut h), Modifiers::NONE, "the next idle step");
}
