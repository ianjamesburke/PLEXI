//! kittest passes reach `eframe::App::raw_input_hook`, as eframe's do.
//!
//! eframe hands every delivery to the hook before the egui pass
//! (`EpiIntegration::update`). Upstream egui_kittest skips it, so the vendored
//! copy (`deps/egui_kittest-0.34.3/PLEXI-PATCH.md`) calls it on the bootstrap
//! pass and on every step. Each step is still exactly one pass per delivery:
//! a test that needs to cut between deliveries steps, never drains.

use egui::{Event, Pos2, RawInput};

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

/// Production witness: a `plexi pane drag` batch reaches egui only through
/// `PlexiApp::raw_input_hook`, one batch per pass. A pass run the way upstream
/// kittest ran it (no hook) delivers nothing.
#[test]
fn kittest_step_delivers_queued_pane_pointer_frames_through_the_hook() {
    let mut h = super::PlexiUiHarness::new_sized(1280.0, 800.0);
    h.open_file_browser(h.workspace_root().to_path_buf());
    h.run_steps(2);
    let pane = h
        .with_app(|app| app.active_focused_pane_id())
        .expect("file browser is focused");
    let at = |x: f32| Pos2::new(x, 300.0);
    h.with_app_mut(|app| {
        let frames = app.pending_pane_pointer_frames.entry(pane).or_default();
        for x in [100.0, 200.0, 300.0] {
            frames.push_back(RawInput {
                events: vec![Event::PointerMoved(at(x))],
                ..Default::default()
            });
        }
    });
    let queued = |h: &super::PlexiUiHarness| {
        h.with_app(|app| {
            app.pending_pane_pointer_frames
                .get(&pane)
                .map_or(0, |f| f.len())
        })
    };
    let ctx = h.harness().ctx.clone();
    let pointer = || ctx.input(|i| i.pointer.latest_pos());

    // Negative control: logic and ui without the hook.
    h.with_app_mut(|app| {
        use eframe::App;
        let mut frame = eframe::Frame::_new_kittest();
        let _ = ctx.run_ui(RawInput::default(), |ui| {
            app.logic(ui.ctx(), &mut frame);
            app.ui(ui, &mut frame);
        });
    });
    assert_eq!(queued(&h), 3, "a pass without the hook delivered a batch");
    assert_ne!(pointer(), Some(at(100.0)));

    h.step();
    assert_eq!(queued(&h), 2, "one step delivers exactly one batch");
    assert_eq!(pointer(), Some(at(100.0)));
    h.step();
    assert_eq!(queued(&h), 1);
    assert_eq!(pointer(), Some(at(200.0)));
}
