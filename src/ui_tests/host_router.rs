//! Confirm Close over a real terminal through the host input router (stint
//! 1659, slice 2). Each test drives the production `raw_input_hook → logic →
//! ui` path with `HostHarness::frame_pass`, one pass per delivery, and
//! observes the terminal's exact PTY bytes, the dialog's decisions by
//! request (opening) identity, and the pane count.

use crate::app::host_router::{Disposition, OpeningId};
use crate::app::FocusKind;
use crate::spatial::tiling::PaneId;
use crate::testing::HostHarness;
use egui::{Event, Key, Modifiers, PointerButton, Pos2, RawInput};
use std::time::{Duration, Instant};

fn cmd() -> Modifiers {
    Modifiers {
        mac_cmd: cfg!(target_os = "macos"),
        command: true,
        ctrl: !cfg!(target_os = "macos"),
        ..Modifiers::NONE
    }
}

fn key(key: Key, modifiers: Modifiers, pressed: bool) -> Event {
    Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers,
    }
}

fn button(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    }
}

/// One native delivery, exactly one pass.
fn deliver(h: &mut HostHarness, events: Vec<Event>) {
    h.frame_pass(RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            Pos2::ZERO,
            egui::vec2(1280.0, 800.0),
        )),
        events,
        ..Default::default()
    });
}

fn tap(h: &mut HostHarness, id: PaneId) -> String {
    String::from_utf8_lossy(&h.terminal_backend(id).take_input_tap()).into_owned()
}

/// The dialog decisions the router admitted, in order.
fn decisions(h: &HostHarness) -> Vec<String> {
    h.app
        .host_router
        .traces()
        .filter_map(|trace| match &trace.disposition {
            Disposition::Dialog(crate::app::host_router::dialog::DialogOutcome::Activate(id)) => {
                Some(id.clone())
            }
            _ => None,
        })
        .collect()
}

fn opening(h: &HostHarness) -> Option<OpeningId> {
    h.app.router_dialog().map(|dialog| dialog.opening)
}

/// A focused terminal with `confirm_close` on and its PTY tap recording.
fn confirming_terminal(h: &mut HostHarness) -> PaneId {
    let id = h.add_focused_terminal();
    h.app.config.confirm_close = Some(true);
    h.terminal_backend(id).enable_input_tap();
    id
}

/// Cmd+W, then its release, then enough passes to present the dialog.
fn open_dialog(h: &mut HostHarness) -> OpeningId {
    deliver(h, vec![key(Key::W, cmd(), true)]);
    deliver(h, vec![key(Key::W, Modifiers::NONE, false)]);
    h.run_frames(2);
    let opening = opening(h).expect("Cmd+W over a terminal asks to close");
    assert!(
        h.app
            .router_dialog_presentation
            .as_ref()
            .is_some_and(|p| p.opening == opening),
        "the dialog was presented"
    );
    opening
}

fn button_center(h: &HostHarness, action: &str) -> Pos2 {
    h.app
        .router_dialog_presentation
        .as_ref()
        .and_then(|p| p.buttons.iter().find(|(id, _, _)| id == action))
        .map(|(_, rect, _)| rect.center())
        .unwrap_or_else(|| panic!("{action} button presented"))
}

/// Entering the dialog mid-delivery: `a` reaches the terminal, Cmd+W opens
/// the dialog, the suffix belongs to it (text ignored, Quick Note vetoed by
/// the critical modal, Escape cancels once), and the rest of that delivery is
/// barred. The revealed terminal gets the next delivery, not this one.
#[test]
#[ignore = "requires-pty"]
fn mid_delivery_dialog_vetoes_dismisses_and_reveals_on_the_next_delivery() {
    let mut h = HostHarness::new();
    let term = confirming_terminal(&mut h);
    let panes = h.pane_count();
    let quick_note = crate::host::keys::KeyBindings::default().open_quick_note;
    deliver(
        &mut h,
        vec![
            Event::Text("a".into()),
            key(Key::W, cmd(), true),
            Event::Text("b".into()),
            key(quick_note.1, quick_note.0, true),
            key(quick_note.1, quick_note.0, false),
            key(Key::Escape, Modifiers::NONE, true),
            key(Key::Escape, Modifiers::NONE, false),
            key(Key::W, Modifiers::NONE, false),
            Event::Text("c".into()),
        ],
    );
    assert_eq!(
        tap(&mut h, term),
        "a",
        "only the prefix reached the terminal"
    );
    assert!(h.app.pending_close, "the close chord opened the dialog");
    // The held suffix resumes on the next pass.
    h.run_frames(3);
    assert!(!h.app.pending_close, "Escape cancelled the dialog");
    assert_eq!(decisions(&h), ["cancel"], "exactly one decision");
    assert!(
        !h.app.focus_stack.contains(&FocusKind::QuickNote),
        "the critical dialog vetoed Quick Note"
    );
    assert_eq!(h.pane_count(), panes, "nothing closed");
    assert_eq!(
        tap(&mut h, term),
        "",
        "nothing after Cmd+W reached the terminal"
    );

    deliver(&mut h, vec![Event::Text("d".into())]);
    h.run_frames(1);
    assert_eq!(
        tap(&mut h, term),
        "d",
        "a fresh delivery reaches the terminal"
    );
}

/// Both real buttons, by pointer: Cancel keeps the pane and the terminal
/// takes fresh input; a reopened dialog is a new opening and Close closes.
#[test]
#[ignore = "requires-pty"]
fn both_buttons_act_by_pointer_on_their_own_opening() {
    let mut h = HostHarness::new();
    let term = confirming_terminal(&mut h);
    let panes = h.pane_count();
    let first = open_dialog(&mut h);
    let cancel = button_center(&h, "cancel");
    deliver(&mut h, vec![button(cancel, true), button(cancel, false)]);
    h.run_frames(2);
    assert!(!h.app.pending_close);
    assert_eq!(h.pane_count(), panes);
    deliver(&mut h, vec![Event::Text("x".into())]);
    h.run_frames(1);
    assert_eq!(tap(&mut h, term), "x");

    let second = open_dialog(&mut h);
    assert_ne!(first, second, "reopening is a new instance");
    // A press that leaves its button activates nothing.
    let close = button_center(&h, "close");
    let cancel = button_center(&h, "cancel");
    deliver(&mut h, vec![button(close, true), button(cancel, false)]);
    h.run_frames(2);
    assert!(
        h.app.pending_close,
        "press on Close, release on Cancel: no decision"
    );
    deliver(&mut h, vec![button(close, true), button(close, false)]);
    h.run_frames(2);
    assert_eq!(decisions(&h), ["cancel", "close"]);
    assert_eq!(h.pane_count(), panes - 1, "Close closed the terminal");
    assert!(!h.app.windows[0].panes.contains_key(&term));
}

/// Keyboard order and refusal: an injected Enter is refused, Tab focuses
/// Cancel and Enter takes it; reopened, a fresh real Enter closes.
#[test]
#[ignore = "requires-pty"]
fn keyboard_refusal_tab_order_and_close() {
    let mut h = HostHarness::new();
    let term = confirming_terminal(&mut h);
    let panes = h.pane_count();
    open_dialog(&mut h);
    h.app
        .pending_pane_inputs
        .entry(term)
        .or_default()
        .push(RawInput {
            events: vec![key(Key::Enter, Modifiers::NONE, true)],
            ..Default::default()
        });
    h.run_frames(2);
    assert!(h.app.pending_close, "an injected Enter is refused");
    assert!(decisions(&h).is_empty());
    deliver(
        &mut h,
        vec![
            key(Key::Enter, Modifiers::NONE, false),
            key(Key::Tab, Modifiers::NONE, true),
            key(Key::Tab, Modifiers::NONE, false),
            key(Key::Enter, Modifiers::NONE, true),
            key(Key::Enter, Modifiers::NONE, false),
        ],
    );
    h.run_frames(2);
    assert!(!h.app.pending_close);
    assert_eq!(
        decisions(&h),
        ["cancel"],
        "Enter took the Tab-focused Cancel"
    );
    assert_eq!(h.pane_count(), panes);

    open_dialog(&mut h);
    deliver(&mut h, vec![key(Key::Enter, Modifiers::NONE, true)]);
    h.run_frames(2);
    assert_eq!(decisions(&h), ["cancel", "close"]);
    assert_eq!(h.pane_count(), panes - 1);
    assert_eq!(
        tap_or_empty(&mut h, term),
        "",
        "the terminal never saw a key"
    );
}

fn tap_or_empty(h: &mut HostHarness, id: PaneId) -> String {
    if h.app.windows[0].panes.contains_key(&id) {
        tap(h, id)
    } else {
        String::new()
    }
}

/// A decision admitted for one opening never reaches its replacement: the
/// dialog is replaced (same look, another pane) before logic executes the
/// Escape admitted for the first. Zero decisions reach the replacement; a
/// fresh Escape then cancels it once.
#[test]
#[ignore = "requires-pty"]
fn a_decision_for_a_replaced_opening_decides_nothing() {
    use eframe::App;
    let mut h = HostHarness::new();
    let a = confirming_terminal(&mut h);
    let first = open_dialog(&mut h);
    let ctx = h.app.ctx.clone();
    let mut input = RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            Pos2::ZERO,
            egui::vec2(1280.0, 800.0),
        )),
        events: vec![key(Key::Escape, Modifiers::NONE, true)],
        ..Default::default()
    };
    // Admit the Escape for the first opening, then replace the dialog before
    // the logic pass that would execute it.
    h.app.raw_input_hook(&ctx, &mut input);
    let replacement = h.app.close_confirm_target_for(a);
    let second = replacement.opening;
    assert_ne!(first, second);
    h.app.close_confirm_target = Some(replacement);
    h.run_frames(2);
    assert!(h.app.pending_close, "the replacement is still asking");
    assert_eq!(opening(&h), Some(second));
    assert_eq!(
        decisions(&h),
        ["cancel"],
        "admitted once, for the first opening"
    );
    deliver(&mut h, vec![key(Key::Escape, Modifiers::NONE, false)]);
    deliver(&mut h, vec![key(Key::Escape, Modifiers::NONE, true)]);
    h.run_frames(2);
    assert!(
        !h.app.pending_close,
        "a fresh Escape cancels the replacement"
    );
    assert_eq!(decisions(&h), ["cancel", "cancel"]);
}

/// Prints `READY` after asking for SGR mouse reports, then idles.
const MOUSE_FIXTURE: &str = r#"
import sys, time
sys.stdout.write("\x1b[?1000h\x1b[?1006h")
print("READY", flush=True)
time.sleep(120)
"#;

fn mouse_terminal(h: &mut HostHarness) -> PaneId {
    let id = h.add_focused_terminal();
    let terminal = crate::host::pane::TerminalPane::new(
        id,
        h.app.ctx.clone(),
        h.app.pty_event_tx.clone(),
        egui_term::BackendSettings {
            shell: "/usr/bin/python3".into(),
            args: vec!["-c".into(), MOUSE_FIXTURE.into()],
            working_directory: Some(std::env::temp_dir()),
            ..Default::default()
        },
        14.0,
    )
    .expect("start mouse fixture");
    h.app.windows[0]
        .panes
        .insert(id, crate::host::pane::Pane::Terminal(Box::new(terminal)));
    let deadline = Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(10));
    loop {
        h.run_frames(2);
        if h.terminal_backend(id)
            .capture_lines(50)
            .iter()
            .any(|line| line.contains("READY"))
        {
            break;
        }
        assert!(Instant::now() < deadline, "mouse fixture timed out");
        std::thread::sleep(Duration::from_millis(20));
    }
    h.run_frames(3);
    h.app.config.confirm_close = Some(true);
    h.terminal_backend(id).enable_input_tap();
    id
}

fn grid_point(h: &mut HostHarness, id: PaneId) -> Pos2 {
    h.terminal_backend(id)
        .presented
        .expect("the terminal was presented")
        .hit_rect
        .center()
}

/// A mouse-reporting drag on the terminal when the dialog takes input: the
/// program gets its press, then exactly one release when the gesture
/// retires; the late physical release and a move send nothing. After the
/// dialog, a fresh click reports press and release.
#[test]
#[ignore = "requires-pty"]
fn a_pointer_gesture_retires_once_when_the_dialog_opens() {
    let mut h = HostHarness::new();
    let term = mouse_terminal(&mut h);
    let at = grid_point(&mut h, term);
    deliver(&mut h, vec![Event::PointerMoved(at), button(at, true)]);
    let pressed = tap(&mut h, term);
    assert!(
        pressed.starts_with("\x1b[<0;") && pressed.ends_with('M'),
        "the program saw the press: {pressed:?}"
    );
    let report = pressed.trim_end_matches('M').to_string();
    deliver(&mut h, vec![key(Key::W, cmd(), true)]);
    h.run_frames(2);
    assert!(h.app.pending_close);
    assert_eq!(
        tap(&mut h, term),
        format!("{report}m"),
        "one retirement release"
    );
    deliver(
        &mut h,
        vec![
            Event::PointerMoved(at + egui::vec2(20.0, 0.0)),
            button(at, false),
            key(Key::W, Modifiers::NONE, false),
        ],
    );
    h.run_frames(2);
    assert_eq!(tap(&mut h, term), "", "the late release reached nobody");
    deliver(&mut h, vec![key(Key::Escape, Modifiers::NONE, true)]);
    deliver(&mut h, vec![key(Key::Escape, Modifiers::NONE, false)]);
    h.run_frames(2);
    assert!(!h.app.pending_close);
    deliver(&mut h, vec![button(at, true), button(at, false)]);
    h.run_frames(1);
    assert_eq!(
        tap(&mut h, term),
        format!("{report}M{report}m"),
        "a fresh click reports"
    );
}

/// A hidden pass invalidates a suffix held behind the close chord.
#[test]
#[ignore = "requires-pty"]
fn hiding_drops_the_held_suffix() {
    let mut h = HostHarness::new();
    let term = confirming_terminal(&mut h);
    deliver(
        &mut h,
        vec![
            key(Key::W, cmd(), true),
            key(Key::Escape, Modifiers::NONE, true),
        ],
    );
    assert!(h.app.pending_close);
    h.hidden_frame();
    h.run_frames(2);
    assert!(
        h.app.pending_close,
        "the held Escape did not act after the hide"
    );
    assert!(decisions(&h).is_empty());
    assert_eq!(tap(&mut h, term), "");
}

/// Harness screenshot: the routed Confirm Close over a terminal, with Tab
/// focus on Cancel projected from the dialog model.
#[test]
#[ignore = "requires-pty"]
fn screenshot_confirm_close_over_terminal() {
    let mut h = HostHarness::new();
    confirming_terminal(&mut h);
    open_dialog(&mut h);
    deliver(&mut h, vec![key(Key::Tab, Modifiers::NONE, true)]);
    deliver(&mut h, vec![key(Key::Tab, Modifiers::NONE, false)]);
    h.run_frames(2);
    assert_eq!(
        h.app
            .router_dialog()
            .and_then(|d| h.app.host_router.dialog_focus(d.opening).map(str::to_owned))
            .as_deref(),
        Some("cancel")
    );
    let path = crate::ui_tests::evidence_png!();
    h.render_png(&path).expect("render the dialog");
    assert!(std::path::Path::new(&path).exists());
}

/// Closing the focused terminal retires the rest of the delivery admitted
/// for it: the pane revealed by the close gets nothing from it, then takes
/// the next delivery.
#[test]
#[ignore = "requires-pty"]
fn a_close_without_confirmation_retires_the_suffix() {
    let mut h = HostHarness::new();
    let revealed = h.add_focused_terminal();
    let closing = h.add_focused_terminal();
    h.app.config.confirm_close = Some(false);
    h.terminal_backend(revealed).enable_input_tap();
    deliver(
        &mut h,
        vec![key(Key::W, cmd(), true), Event::Text("x".into())],
    );
    h.run_frames(2);
    assert!(!h.app.windows[0].panes.contains_key(&closing), "Cmd+W closed it");
    assert_eq!(h.app.active_focused_pane_id(), Some(revealed));
    assert_eq!(tap(&mut h, revealed), "", "the revealed pane got no suffix");
    deliver(&mut h, vec![key(Key::W, Modifiers::NONE, false)]);
    deliver(&mut h, vec![Event::Text("y".into())]);
    h.run_frames(1);
    assert_eq!(tap(&mut h, revealed), "y");
}
