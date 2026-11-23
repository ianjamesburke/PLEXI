//! Stint 1659, first router slice: the boundary between a real pane and the
//! command palette. Every case drives `raw_input_hook`, logic and ui through
//! `PlexiUiHarness::step`, one native delivery per step, beside a control
//! that shows the same input acts when nothing is in the way.
//!
//! The palette is opened with the Cmd+P down alone where a case needs input
//! that lands before the palette's first draw: egui still has the pane's
//! layers from the last pass, so only the router keeps the pane out of it.

use super::PlexiUiHarness;
use crate::app::FocusKind;
use crate::host::pane::{Pane, TerminalPane, ViewPane, ViewRuntime};
use crate::spatial::tiling::PaneId;
use egui::{Event, Key, Modifiers, PointerButton, Pos2};
use std::time::{Duration, Instant};

fn key(key: Key, modifiers: Modifiers, pressed: bool, repeat: bool) -> Event {
    Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat,
        modifiers,
    }
}

fn button(at: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    }
}

fn wheel(dy: f32) -> Event {
    Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: egui::vec2(0.0, dy),
        modifiers: Modifiers::NONE,
        phase: egui::TouchPhase::Move,
    }
}

/// One native delivery: the modifiers held for the pass, then its events.
fn frame(h: &mut PlexiUiHarness, modifiers: Modifiers, events: Vec<Event>) {
    h.harness().input_mut().modifiers = modifiers;
    h.harness().input_mut().events.extend(events);
    h.step();
}

/// A chord pressed and released across two deliveries, as a person types it.
fn chord(h: &mut PlexiUiHarness, modifiers: Modifiers, k: Key) {
    frame(h, modifiers, vec![key(k, modifiers, true, false)]);
    frame(h, modifiers, vec![key(k, modifiers, false, false)]);
    frame(h, Modifiers::NONE, vec![]);
}

fn palette_open(h: &PlexiUiHarness) -> bool {
    h.with_app(|app| app.show_command_palette)
}

fn open_palette(h: &mut PlexiUiHarness) {
    chord(h, Modifiers::COMMAND, Key::P);
    h.run_steps(3);
    assert!(palette_open(h), "Cmd+P opened the palette");
}

/// Cmd+W held through native key repeat: one down, two repeats, one up,
/// each in its own delivery.
fn hold_close_chord(h: &mut PlexiUiHarness) {
    let cmd = Modifiers::COMMAND;
    frame(h, cmd, vec![key(Key::W, cmd, true, false)]);
    frame(h, cmd, vec![key(Key::W, cmd, true, true)]);
    frame(h, cmd, vec![key(Key::W, cmd, true, true)]);
    frame(h, cmd, vec![key(Key::W, cmd, false, false)]);
    frame(h, Modifiers::NONE, vec![]);
    h.run_steps(3);
}

fn pane_alive(h: &PlexiUiHarness, pane: PaneId) -> bool {
    h.with_app(|app| app.windows.iter().any(|w| w.panes.contains_key(&pane)))
}

fn focused(h: &PlexiUiHarness) -> Option<PaneId> {
    h.with_app(|app| app.active_focused_pane_id())
}

fn confirm_close_open(h: &PlexiUiHarness) -> bool {
    h.with_app(|app| app.focus_stack.contains(&FocusKind::ConfirmClose))
}

fn pointer_down(h: &mut PlexiUiHarness) -> bool {
    h.harness().ctx.input(|i| i.pointer.any_down())
}

/// Put `left` and `right` side by side and focus `right`.
fn side_by_side(h: &mut PlexiUiHarness, left: PaneId, right: PaneId) {
    h.with_app_mut(|app| {
        let w = app.active_window;
        let tree = &mut app.windows[w].tree;
        let tiles = [left, right].map(|p| tree.tiles.find_pane(&p).expect("pane tile"));
        tree.root = Some(tree.tiles.insert_horizontal_tile(tiles.to_vec()));
        app.windows[w].focused_pane = Some(tiles[1]);
    });
    h.run_steps(3);
}

fn install(h: &mut PlexiUiHarness, pane: PaneId, content: Pane) {
    h.with_app_mut(|app| {
        let w = app.active_window;
        let win = &mut app.windows[w];
        win.panes.insert(pane, content).expect("vacant pane slot");
        let tile = win.tree.tiles.insert_pane(pane);
        if win.tree.root.is_none() {
            win.tree.root = Some(tile);
        }
        win.focused_pane = Some(tile);
    });
}

// ── Real terminals ──────────────────────────────────────────────────────────

/// Prints READY, then reads and discards input.
const PLAIN: &str = "printf 'READY\\n'; exec cat >/dev/null";
/// Asks for X10/normal mouse reports first.
const MOUSE: &str = "printf '\\033[?1000hREADY\\n'; exec cat >/dev/null";
/// Leaves several screens of scrollback.
const SCROLLBACK: &str = "seq 1 400; printf 'READY\\n'; exec cat >/dev/null";

fn add_terminal(h: &mut PlexiUiHarness, script: &str) -> PaneId {
    let (pane, terminal) = h.with_app_mut(|app| {
        let pane = app.host.alloc_pane_id();
        let terminal = TerminalPane::fixture(
            pane,
            app.ctx.clone(),
            app.pty_event_tx.clone(),
            egui_term::BackendSettings {
                shell: "/bin/sh".into(),
                args: vec!["-c".into(), script.into()],
                working_directory: Some(app.router.active().root.clone()),
                ..Default::default()
            },
            16.0,
        )
        .expect("start terminal fixture");
        terminal.backend.enable_input_tap();
        (pane, terminal)
    });
    install(h, pane, Pane::Terminal(Box::new(terminal)));
    pane
}

fn with_terminal<R>(
    h: &mut PlexiUiHarness,
    pane: PaneId,
    f: impl FnOnce(&mut TerminalPane) -> R,
) -> R {
    h.with_app_mut(|app| {
        let terminal = app
            .windows
            .iter_mut()
            .find_map(|w| w.panes.get_mut(&pane))
            .and_then(Pane::as_terminal_mut)
            .expect("terminal pane");
        f(terminal)
    })
}

/// Bytes the host wrote to this terminal since the last take.
fn tap(h: &mut PlexiUiHarness, pane: PaneId) -> Vec<u8> {
    with_terminal(h, pane, |t| t.backend.take_input_tap())
}

fn wait_ready(h: &mut PlexiUiHarness, panes: &[PaneId]) {
    let deadline = Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(10));
    for &pane in panes {
        loop {
            h.run_steps(2);
            let ready = with_terminal(h, pane, |t| {
                t.backend
                    .capture_lines(60)
                    .iter()
                    .any(|line| line.contains("READY"))
            });
            if ready {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "terminal {pane} never printed READY"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    h.run_steps(3);
    for &pane in panes {
        tap(h, pane);
    }
}

fn two_terminals(script: &str) -> (PlexiUiHarness, PaneId, PaneId) {
    let mut h = PlexiUiHarness::new_sized(1168.0, 800.0);
    h.with_app_mut(|app| app.sidebar_visible = false);
    h.run_steps(2);
    let left = add_terminal(&mut h, script);
    let right = add_terminal(&mut h, script);
    side_by_side(&mut h, left, right);
    wait_ready(&mut h, &[left, right]);
    (h, left, right)
}

fn center(h: &mut PlexiUiHarness, pane: PaneId) -> Pos2 {
    h.harness()
        .ctx
        .read_response(egui_term::terminal_widget_id(pane))
        .expect("rendered terminal widget")
        .rect
        .center()
}

/// Normal-encoding mouse reports in `bytes`: `true` for a left press,
/// `false` for a release.
fn mouse_reports(bytes: &[u8]) -> Vec<bool> {
    bytes
        .windows(4)
        .filter(|w| w[..3] == *b"\x1b[M")
        .map(|w| match w[3] {
            32 => true,
            35 => false,
            other => panic!("unexpected mouse report button byte {other}"),
        })
        .collect()
}

/// A press and release over `at`, in one delivery.
fn click(h: &mut PlexiUiHarness, at: Pos2) {
    frame(
        h,
        Modifiers::NONE,
        vec![Event::PointerMoved(at), button(at, true), button(at, false)],
    );
    h.run_steps(3);
}

/// Holding Cmd+W over the palette dismisses the palette once. The repeats
/// and the up belong to the palette's opening, so neither terminal sees the
/// chord, and a fresh Cmd+W after the key-up acts on the focused pane.
#[test]
#[ignore = "requires-pty"]
fn held_close_chord_dismisses_the_palette_once_and_the_terminal_survives() {
    let (mut h, left, right) = two_terminals(PLAIN);
    open_palette(&mut h);
    hold_close_chord(&mut h);
    assert!(!palette_open(&h), "Cmd+W dismissed the palette");
    assert!(
        pane_alive(&h, left) && pane_alive(&h, right) && !confirm_close_open(&h),
        "the held chord that dismissed the palette asked to close no pane"
    );
    assert!(tap(&mut h, left).is_empty() && tap(&mut h, right).is_empty());

    // Control: the next, fresh Cmd+W reaches the focused terminal.
    chord(&mut h, Modifiers::COMMAND, Key::W);
    h.run_steps(3);
    assert!(
        !pane_alive(&h, right) || confirm_close_open(&h),
        "a fresh Cmd+W after the key-up closes the focused pane or asks to"
    );
    assert!(pane_alive(&h, left));
}

/// `Escape, press, release` over the other terminal in one delivery, before
/// the palette's first draw: Escape dismisses the palette and the click is
/// consumed with the rest of its delivery. The next click works.
#[test]
#[ignore = "requires-pty"]
fn escape_then_click_over_another_terminal_does_not_click_through() {
    let (mut h, left, right) = two_terminals(PLAIN);
    let at = center(&mut h, left);
    click(&mut h, at);
    assert_eq!(
        focused(&h),
        Some(left),
        "control: a click focuses the left terminal"
    );
    side_by_side(&mut h, left, right);
    assert_eq!(focused(&h), Some(right));

    let cmd = Modifiers::COMMAND;
    frame(&mut h, cmd, vec![key(Key::P, cmd, true, false)]);
    assert!(palette_open(&h), "Cmd+P opened the palette");
    frame(
        &mut h,
        Modifiers::NONE,
        vec![
            key(Key::Escape, Modifiers::NONE, true, false),
            Event::PointerMoved(at),
            button(at, true),
            button(at, false),
        ],
    );
    frame(
        &mut h,
        Modifiers::NONE,
        vec![
            key(Key::P, Modifiers::NONE, false, false),
            key(Key::Escape, Modifiers::NONE, false, false),
        ],
    );
    h.run_steps(3);
    assert!(!palette_open(&h), "Escape dismissed the palette");
    assert_eq!(
        focused(&h),
        Some(right),
        "the click behind Escape did not land"
    );
    assert!(!pointer_down(&mut h));
    assert!(tap(&mut h, left).is_empty() && tap(&mut h, right).is_empty());

    click(&mut h, at);
    assert_eq!(
        focused(&h),
        Some(left),
        "a fresh click on the revealed pane works"
    );
}

/// A wheel after Escape in the same delivery does not scroll the terminal the
/// dismissal revealed. A wheel in the next delivery does.
#[test]
#[ignore = "requires-pty"]
fn wheel_after_escape_in_one_delivery_does_not_scroll_the_revealed_terminal() {
    let (mut h, _left, right) = two_terminals(SCROLLBACK);
    let at = center(&mut h, right);
    let offset = |h: &mut PlexiUiHarness| {
        with_terminal(h, right, |t| t.backend.last_content().grid.display_offset())
    };
    assert_eq!(offset(&mut h), 0);

    let cmd = Modifiers::COMMAND;
    frame(&mut h, cmd, vec![key(Key::P, cmd, true, false)]);
    assert!(palette_open(&h));
    frame(
        &mut h,
        Modifiers::NONE,
        vec![
            key(Key::Escape, Modifiers::NONE, true, false),
            Event::PointerMoved(at),
            wheel(5.0),
        ],
    );
    frame(
        &mut h,
        Modifiers::NONE,
        vec![
            key(Key::P, Modifiers::NONE, false, false),
            key(Key::Escape, Modifiers::NONE, false, false),
        ],
    );
    h.run_steps(3);
    assert!(!palette_open(&h), "Escape dismissed the palette");
    assert_eq!(offset(&mut h), 0, "the wheel behind Escape did not scroll");

    frame(
        &mut h,
        Modifiers::NONE,
        vec![Event::PointerMoved(at), wheel(5.0)],
    );
    h.run_steps(3);
    assert!(
        offset(&mut h) > 0,
        "control: the next delivery's wheel scrolls"
    );
}

/// A terminal mouse-down, then the palette opens, then mouse-up: the
/// terminal's program hears the button go up once, when the palette took the
/// pointer, and the physical release is retired. Nothing is left pressed,
/// and a fresh click reaches the terminal after the palette closes.
#[test]
#[ignore = "requires-pty"]
fn a_terminal_drag_is_cancelled_when_the_palette_opens() {
    let (mut h, _left, right) = two_terminals(MOUSE);
    let at = center(&mut h, right);
    let below = at + egui::vec2(0.0, 40.0);

    // Control: press, move and release report press then release.
    frame(
        &mut h,
        Modifiers::NONE,
        vec![Event::PointerMoved(at), button(at, true)],
    );
    frame(&mut h, Modifiers::NONE, vec![Event::PointerMoved(below)]);
    frame(&mut h, Modifiers::NONE, vec![button(below, false)]);
    h.run_steps(2);
    assert_eq!(mouse_reports(&tap(&mut h, right)), [true, false], "control");

    frame(
        &mut h,
        Modifiers::NONE,
        vec![Event::PointerMoved(at), button(at, true)],
    );
    h.run_steps(1);
    assert_eq!(mouse_reports(&tap(&mut h, right)), [true]);
    let cmd = Modifiers::COMMAND;
    frame(&mut h, cmd, vec![key(Key::P, cmd, true, false)]);
    frame(&mut h, cmd, vec![key(Key::P, cmd, false, false)]);
    frame(&mut h, Modifiers::NONE, vec![Event::PointerMoved(below)]);
    h.run_steps(2);
    assert!(palette_open(&h));
    assert_eq!(
        mouse_reports(&tap(&mut h, right)),
        [false],
        "the drag was cancelled at the terminal once"
    );
    assert!(!pointer_down(&mut h), "egui holds no button");

    frame(&mut h, Modifiers::NONE, vec![button(below, false)]);
    h.run_steps(2);
    assert!(
        tap(&mut h, right).is_empty(),
        "the physical release was retired"
    );
    assert!(
        palette_open(&h),
        "the retired release did not touch the palette"
    );
    assert!(!pointer_down(&mut h));

    chord(&mut h, Modifiers::NONE, Key::Escape);
    h.run_steps(3);
    assert!(!palette_open(&h));
    click(&mut h, at);
    assert_eq!(
        mouse_reports(&tap(&mut h, right)),
        [true, false],
        "a fresh click reaches the terminal"
    );
}

/// A blur in the middle of a terminal drag cancels it the same way.
#[test]
#[ignore = "requires-pty"]
fn a_window_blur_cancels_a_held_terminal_drag() {
    let (mut h, _left, right) = two_terminals(MOUSE);
    let at = center(&mut h, right);
    frame(
        &mut h,
        Modifiers::NONE,
        vec![Event::PointerMoved(at), button(at, true)],
    );
    h.run_steps(1);
    assert_eq!(mouse_reports(&tap(&mut h, right)), [true]);

    frame(&mut h, Modifiers::NONE, vec![Event::WindowFocused(false)]);
    h.run_steps(2);
    assert_eq!(
        mouse_reports(&tap(&mut h, right)),
        [false],
        "blur cancelled the drag"
    );
    assert!(!pointer_down(&mut h));

    frame(
        &mut h,
        Modifiers::NONE,
        vec![Event::WindowFocused(true), button(at, false)],
    );
    h.run_steps(2);
    assert!(
        tap(&mut h, right).is_empty(),
        "the late release was retired"
    );
    click(&mut h, at);
    assert_eq!(mouse_reports(&tap(&mut h, right)), [true, false]);
}

// ── Codex round 5 reproductions (alpha side) ───────────────────────────────
// Carried from `evidence/3265-round5-alpha`; file browser panes, no PTY.

fn add_view_pane(h: &mut PlexiUiHarness) -> PaneId {
    let root = h.workspace_root().to_path_buf();
    let pane = h.with_app_mut(|app| app.host.alloc_pane_id());
    let view = ViewPane {
        incarnation: Default::default(),
        pip_status: None,
        id: pane,
        runtime: ViewRuntime::Builtin(Box::new(crate::file_browser::FileBrowserView::new(
            root.clone(),
        ))),
        workspace_root: root,
        permissions: crate::app::permissions::PluginPermissions::builtin(),
        manifest_id: "test".to_string(),
        name: "Test App".to_string(),
        pane_group: None,
        linked_pane_id: None,
        overlay_replaced: None,
        agent: None,
        slots: std::collections::HashMap::new(),
        semantic_state: Default::default(),
    };
    install(h, pane, Pane::View(Box::new(view)));
    pane
}

fn two_views() -> (PlexiUiHarness, PaneId, PaneId) {
    let mut h = PlexiUiHarness::new_sized(1168.0, 800.0);
    let left = add_view_pane(&mut h);
    let right = add_view_pane(&mut h);
    side_by_side(&mut h, left, right);
    (h, left, right)
}

/// Round 5 finding 1 (P1): holding Cmd+W over the palette closes the palette,
/// then on key repeat the pane behind it.
#[test]
fn round5_held_close_chord_spares_the_pane_behind_the_palette() {
    let (mut h, left, right) = two_views();
    hold_close_chord(&mut h);
    assert!(
        !pane_alive(&h, right) && pane_alive(&h, left),
        "control: held Cmd+W closes exactly the focused pane"
    );

    let (mut h, left, right) = two_views();
    open_palette(&mut h);
    hold_close_chord(&mut h);
    assert!(!palette_open(&h), "Cmd+W dismissed the palette");
    assert!(
        pane_alive(&h, right) && pane_alive(&h, left),
        "the held chord that dismissed the palette closed no pane"
    );
}

fn file_question() {
    let monitor =
        crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir()).unwrap();
    monitor
        .file_needs_you(crate::broker::gate::NeedsYouFile {
            kind: crate::broker::gate::NeedsYouKind::Question,
            actor: "codex".into(),
            resource: "pane:1".into(),
            summary: "Continue?".into(),
            expires_at: None,
            run_tag: None,
        })
        .unwrap();
}

fn approval_on_top(h: &PlexiUiHarness) -> bool {
    h.with_app(|app| app.focus_stack.last().copied()) == Some(FocusKind::ApprovalBanner)
}

const FOCUS_APPROVAL: Modifiers = Modifiers {
    alt: true,
    ctrl: false,
    shift: false,
    mac_cmd: false,
    command: true,
};

/// Round 5 finding 3 (P2): Focus Approval still works after a palette whose
/// opening P was released while it was open, then dismissed with Escape.
#[test]
fn round5_focus_approval_after_palette_dismissal() {
    let (mut h, _, _) = two_views();
    file_question();
    h.run_steps(3);
    chord(&mut h, FOCUS_APPROVAL, Key::P);
    h.run_steps(3);
    assert!(
        approval_on_top(&h),
        "control: Cmd+Option+P focuses the approval"
    );

    let (mut h, _, _) = two_views();
    file_question();
    h.run_steps(3);
    open_palette(&mut h);
    chord(&mut h, Modifiers::NONE, Key::Escape);
    h.run_steps(3);
    assert!(!palette_open(&h), "Escape dismissed the palette");
    chord(&mut h, FOCUS_APPROVAL, Key::P);
    h.run_steps(3);
    assert!(
        approval_on_top(&h),
        "Cmd+Option+P after the dismissal focuses the approval"
    );
}
