//! Stint 1391: the boundary between the command palette and the panes
//! behind it. Every case drives `raw_input_hook`, logic and ui through
//! `PlexiUiHarness::step`, one native delivery per step, beside a control
//! that shows the same input acts when nothing is in the way.
//!
//! Carried from Codex round 5 on PR #3265 (`evidence/3265-round5`) and the
//! closed router PR #3297 (its pane survival, focus, wheel and drag cases).

use super::*;
use egui::{Event, Key, Modifiers, PointerButton, Pos2};
use egui_kittest::kittest::{by, Queryable};
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

fn wheel_event(dy: f32, phase: egui::TouchPhase) -> Event {
    Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: egui::vec2(0.0, dy),
        modifiers: Modifiers::NONE,
        phase,
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
    h.with_app(|app| app.command_palette_open())
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

fn two_views() -> (PlexiUiHarness, PaneId, PaneId) {
    let mut h = PlexiUiHarness::new_sized(1168.0, 800.0);
    let left = add_focused_pane(&mut h);
    let right = add_focused_pane(&mut h);
    side_by_side(&mut h, left, right);
    (h, left, right)
}

// ── Keyboard handoffs (Codex round 5) ──────────────────────────────────────

/// Round 5 finding 1 (P1): holding Cmd+W over the palette dismisses the
/// palette once; the repeats never close the pane behind it.
#[test]
fn held_close_chord_spares_the_pane_behind_the_palette() {
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

fn queue_open(h: &PlexiUiHarness) -> bool {
    h.with_app(|app| app.attention_queue.open)
}

/// Round 5 finding 2 (P1): the Attention Queue opened from the palette
/// takes the palette's place and answers its own Escape.
#[test]
fn attention_queue_opened_from_the_palette_answers_escape() {
    let (mut h, _, _) = two_views();
    chord(&mut h, Modifiers::COMMAND, Key::I);
    h.run_steps(3);
    assert!(queue_open(&h), "control: Cmd+I opens the queue");
    chord(&mut h, Modifiers::NONE, Key::Escape);
    h.run_steps(3);
    assert!(!queue_open(&h), "control: Escape closes the queue");

    let (mut h, _, _) = two_views();
    open_palette(&mut h);
    chord(&mut h, Modifiers::COMMAND, Key::I);
    h.run_steps(3);
    assert!(queue_open(&h), "Cmd+I from the palette opens the queue");
    assert!(!palette_open(&h), "the palette handed off to the queue");
    chord(&mut h, Modifiers::NONE, Key::Escape);
    h.run_steps(3);
    assert!(
        !queue_open(&h),
        "Escape closes the queue opened from the palette"
    );
}

fn surfaces(h: &PlexiUiHarness) -> Vec<FocusKind> {
    h.with_app(|app| app.focus_stack.kinds().collect())
}

/// Round 6 finding 1 (P1), V3 D row 1: the queue, then the palette over it.
/// The palette owns the field; the first Escape gives a keyboard-operable
/// queue back, the second returns to the pane that invoked it.
#[test]
fn queue_then_palette_escapes_back_through_the_queue() {
    let (mut h, _, right) = two_views();
    chord(&mut h, Modifiers::COMMAND, Key::I);
    h.run_steps(3);
    assert_eq!(surfaces(&h), [FocusKind::AttentionQueue]);
    chord(&mut h, Modifiers::COMMAND, Key::P);
    h.run_steps(3);
    assert_eq!(
        surfaces(&h),
        [FocusKind::AttentionQueue, FocusKind::CommandPalette],
        "the palette opened over the queue"
    );
    frame(&mut h, Modifiers::NONE, vec![Event::Text("abc".into())]);
    h.run_steps(3);
    assert_eq!(
        h.with_app(|app| app
            .host
            .picker(plexi_host_model::picker::PickerId::Palette)
            .map(|picker| picker.query().to_owned())),
        Some("abc".to_owned()),
        "the palette owns the field"
    );

    chord(&mut h, Modifiers::NONE, Key::Escape);
    h.run_steps(3);
    assert!(!palette_open(&h), "the first Escape closed the palette");
    assert_eq!(surfaces(&h), [FocusKind::AttentionQueue]);
    assert!(queue_open(&h), "the queue it covered is back");
    assert_eq!(
        h.with_app(|app| app.overlay_surface()),
        Some(crate::app::input_owner::OverlaySurface::AttentionQueue),
        "the queue owns input"
    );

    chord(&mut h, Modifiers::NONE, Key::Escape);
    h.run_steps(3);
    assert!(!queue_open(&h), "the second Escape closed the queue");
    assert!(surfaces(&h).is_empty());
    assert_eq!(focused(&h), Some(right), "back at the invoking pane");
}

/// Round 7: Cmd+Esc closes the queue only when the queue is the top surface.
/// Covered by the palette, the queue and the palette both stay.
#[test]
fn cmd_escape_never_dismisses_a_covered_queue() {
    let (mut h, _, _) = two_views();
    chord(&mut h, Modifiers::COMMAND, Key::I);
    h.run_steps(3);
    chord(&mut h, Modifiers::COMMAND, Key::P);
    h.run_steps(3);
    assert_eq!(
        surfaces(&h),
        [FocusKind::AttentionQueue, FocusKind::CommandPalette]
    );
    chord(&mut h, Modifiers::COMMAND, Key::Escape);
    h.run_steps(3);
    assert_eq!(
        surfaces(&h),
        [FocusKind::AttentionQueue, FocusKind::CommandPalette],
        "Cmd+Esc left the covered queue and the palette open"
    );
    assert!(queue_open(&h));

    // Control: uncovered, the same chord closes the queue.
    chord(&mut h, Modifiers::NONE, Key::Escape);
    h.run_steps(3);
    assert_eq!(surfaces(&h), [FocusKind::AttentionQueue]);
    chord(&mut h, Modifiers::COMMAND, Key::Escape);
    h.run_steps(3);
    assert!(!queue_open(&h), "Cmd+Esc closes the queue on top");
    assert!(surfaces(&h).is_empty());
}

/// V3 D row 1: with no actionable request, Focus Approval leaves the
/// palette owning input.
#[test]
fn empty_focus_approval_leaves_the_palette_in_charge() {
    let (mut h, _, _) = two_views();
    open_palette(&mut h);
    chord(&mut h, FOCUS_APPROVAL, Key::P);
    h.run_steps(3);
    assert!(palette_open(&h));
    assert_eq!(surfaces(&h), [FocusKind::CommandPalette]);
    frame(&mut h, Modifiers::NONE, vec![Event::Text("x".into())]);
    h.run_steps(3);
    assert_eq!(
        h.with_app(|app| app
            .host
            .picker(plexi_host_model::picker::PickerId::Palette)
            .map(|picker| picker.query().to_owned())),
        Some("x".to_owned())
    );
}

/// V3 D row 1: an actionable Focus Approval from the palette ends the
/// palette's opening; the banner is the only surface, for its request.
#[test]
fn actionable_focus_approval_replaces_the_palette() {
    let (mut h, _, _) = two_views();
    let id = file_question();
    h.run_steps(3);
    open_palette(&mut h);
    chord(&mut h, FOCUS_APPROVAL, Key::P);
    h.run_steps(3);
    assert!(!palette_open(&h), "the handoff ended the palette's opening");
    assert_eq!(surfaces(&h), [FocusKind::ApprovalBanner]);
    assert_eq!(
        h.with_app(|app| app
            .focus_stack
            .top_opening()
            .and_then(|opening| opening.request.clone())),
        Some(id),
        "the banner's opening names its request"
    );
}

fn file_question() -> String {
    let monitor =
        crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir()).unwrap();
    monitor
        .file_needs_you(crate::broker::gate::NeedsYouFile {
            kind: crate::broker::gate::NeedsYouKind::Question,
            actor: crate::broker::principal::RecordOwner::Host("codex".into()),
            resource: "pane:1".into(),
            summary: "Continue?".into(),
            expires_at: None,
            run_tag: None,
        })
        .unwrap()
}

fn approval_on_top(h: &PlexiUiHarness) -> bool {
    h.with_app(|app| app.focus_stack.top().as_ref().copied()) == Some(FocusKind::ApprovalBanner)
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
fn focus_approval_works_after_palette_dismissal() {
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

/// Round 5 finding 6 (P2): an Enter frozen while its query loads never
/// completes beneath an approval that took input before its answer came.
#[test]
fn pending_enter_never_completes_beneath_an_approval() {
    let (mut h, _, _) = two_views();
    let id = file_question();
    h.run_steps(3);
    open_palette(&mut h);
    crate::overlays::command_palette::palette_test_log::take_activations();
    frame(
        &mut h,
        FOCUS_APPROVAL,
        vec![
            Event::Paste("keyboard".into()),
            key(Key::Enter, Modifiers::NONE, true, false),
            key(Key::P, FOCUS_APPROVAL, true, false),
        ],
    );
    h.run_steps(3);
    assert!(approval_on_top(&h), "the approval took focus");
    assert!(
        crate::overlays::command_palette::palette_test_log::take_activations().is_empty(),
        "the pending Enter ran no row"
    );
    let monitor =
        crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir()).unwrap();
    assert!(
        monitor
            .host_records()
            .question(&id)
            .is_some_and(|record| record.resolution.is_none()),
        "the request is still pending"
    );
}

// ── The results list (Codex round 5) ───────────────────────────────────────

fn rect_of(h: &mut PlexiUiHarness, label: &str) -> egui::Rect {
    h.harness().get_by_label(label).rect()
}

fn selected_rect(h: &mut PlexiUiHarness) -> Option<egui::Rect> {
    h.harness()
        .query_all(by().predicate(|node| node.toggled() == Some(egui::accesskit::Toggled::True)))
        .next()
        .map(|node| node.rect())
}

fn scroll(h: &mut PlexiUiHarness, at: Pos2, dy: f32) {
    frame(
        h,
        Modifiers::NONE,
        vec![
            Event::PointerMoved(at),
            Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, dy),
                modifiers: Modifiers::NONE,
                phase: egui::TouchPhase::Move,
            },
        ],
    );
    h.run_steps(2);
}

/// Round 5 finding 4 (P2): the results scrollbar thumb drags the list.
#[test]
fn results_scrollbar_thumb_drags_the_list() {
    let (mut h, _, _) = two_views();
    open_palette(&mut h);
    let top0 = rect_of(&mut h, "CONTEXTS").top();
    let row = selected_rect(&mut h).expect("a selected row");
    let list_center = egui::pos2(row.center().x, top0 + 200.0);

    scroll(&mut h, list_center, -300.0);
    let scrolled = rect_of(&mut h, "CONTEXTS").top();
    assert!(
        scrolled < top0 - 100.0,
        "control: the wheel scrolls ({top0} -> {scrolled})"
    );
    scroll(&mut h, list_center, 5000.0);
    assert_eq!(
        rect_of(&mut h, "CONTEXTS").top(),
        top0,
        "control: wheel back to top"
    );

    // The thumb sits in the scrollbar gutter just right of the rows, at the
    // top of the list while the offset is zero.
    let thumb = egui::pos2(row.right() + 6.0, top0 + 8.0);
    frame(&mut h, Modifiers::NONE, vec![Event::PointerMoved(thumb)]);
    frame(&mut h, Modifiers::NONE, vec![button(thumb, true)]);
    let mut at = thumb;
    for _ in 0..6 {
        at.y += 40.0;
        frame(&mut h, Modifiers::NONE, vec![Event::PointerMoved(at)]);
    }
    frame(&mut h, Modifiers::NONE, vec![button(at, false)]);
    h.run_steps(2);
    h.save_screenshot(&evidence_png!()).expect("render failed");
    let dragged = rect_of(&mut h, "CONTEXTS").top();
    assert!(
        dragged < top0 - 100.0,
        "dragging the thumb from {thumb:?} to {at:?} scrolls the list ({top0} -> {dragged})"
    );
    assert!(palette_open(&h), "the drag left the palette open");
}

/// Round 5 finding 5 (P2): after scrolling, a typed query whose results
/// overflow shows the row Enter would run.
#[test]
fn typed_query_after_scrolling_shows_the_selected_row() {
    // The list's visible band: from its first caption at offset zero to the
    // footer hints.
    let visible = |h: &mut PlexiUiHarness, list_top: f32| {
        let footer = rect_of(h, "dismiss").top();
        let row = selected_rect(h).expect("a selected row");
        (
            row.top() >= list_top && row.bottom() <= footer,
            row,
            list_top,
            footer,
        )
    };

    let (mut h, _, _) = two_views();
    open_palette(&mut h);
    let top0 = rect_of(&mut h, "CONTEXTS").top();
    frame(&mut h, Modifiers::NONE, vec![Event::Text("e".into())]);
    h.run_steps(4);
    let (shown, row, top, footer) = visible(&mut h, top0 - 4.0);
    assert!(
        shown,
        "control: typed without scrolling, {row:?} between {top} and {footer}"
    );

    let (mut h, _, _) = two_views();
    open_palette(&mut h);
    let top0 = rect_of(&mut h, "CONTEXTS").top();
    let row = selected_rect(&mut h).expect("a selected row");
    scroll(&mut h, egui::pos2(row.center().x, top0 + 200.0), -1500.0);
    assert!(
        h.harness().query_by_label("CONTEXTS").is_none()
            || rect_of(&mut h, "CONTEXTS").top() < top0 - 1000.0,
        "the list scrolled several screens"
    );
    frame(&mut h, Modifiers::NONE, vec![Event::Text("e".into())]);
    h.run_steps(4);
    h.save_screenshot(&evidence_png!()).expect("render failed");
    let (shown, row, top, footer) = visible(&mut h, top0 - 4.0);
    assert!(
        shown,
        "the selected row {row:?} is between {top} and {footer}"
    );
}

// ── Real terminals (carried from #3297) ────────────────────────────────────

/// Prints READY, then reads and discards input.
const PLAIN: &str = "printf 'READY\\n'; exec cat >/dev/null";
/// Asks for X10/normal mouse reports first.
const MOUSE: &str = "printf '\\033[?1000hREADY\\n'; exec cat >/dev/null";
/// Leaves several screens of scrollback.
const SCROLLBACK: &str = "seq 1 400; printf 'READY\\n'; exec cat >/dev/null";

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

/// Bytes the host wrote to this terminal since the last take, mouse
/// reports included.
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

/// Holding Cmd+W over the palette dismisses the palette once. Neither
/// terminal sees the chord, and a fresh Cmd+W after the key-up acts on the
/// focused pane.
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

/// `Escape, press, release` over the other terminal in one delivery:
/// Escape dismisses the palette and the click is the session's. The next
/// click works.
#[test]
#[ignore = "requires-pty"]
fn escape_then_click_over_another_terminal_does_not_click_through() {
    let (mut h, left, right) = two_terminals(MOUSE);
    let at = center(&mut h, left);
    click(&mut h, at);
    assert_eq!(
        focused(&h),
        Some(left),
        "control: a click focuses the left terminal"
    );
    assert_eq!(mouse_reports(&tap(&mut h, left)), [true, false], "control");
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
    assert_eq!(mouse_reports(&tap(&mut h, left)), [true, false]);
}

/// `Cmd+P, press, release` over the other terminal in one delivery: egui hit
/// tested the click against a layout without the palette, so the panes are
/// gated for it. Nothing is focused, nothing is reported, the palette stays.
#[test]
#[ignore = "requires-pty"]
fn a_click_in_the_opening_delivery_never_reaches_a_pane() {
    let (mut h, left, right) = two_terminals(MOUSE);
    let at = center(&mut h, left);
    let cmd = Modifiers::COMMAND;
    frame(
        &mut h,
        cmd,
        vec![
            key(Key::P, cmd, true, false),
            Event::PointerMoved(at),
            button(at, true),
            button(at, false),
        ],
    );
    frame(
        &mut h,
        Modifiers::NONE,
        vec![key(Key::P, cmd, false, false)],
    );
    h.run_steps(3);
    assert!(palette_open(&h), "the click did not dismiss the palette");
    chord(&mut h, Modifiers::NONE, Key::Escape);
    h.run_steps(3);
    assert!(!palette_open(&h));
    assert_eq!(
        focused(&h),
        Some(right),
        "the opening click focused nothing"
    );
    assert!(
        tap(&mut h, left).is_empty(),
        "no mouse report reached the left terminal"
    );

    click(&mut h, at);
    assert_eq!(focused(&h), Some(left), "control: a fresh click focuses");
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
            wheel_event(5.0, egui::TouchPhase::Move),
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
        vec![
            Event::PointerMoved(at),
            wheel_event(5.0, egui::TouchPhase::Move),
        ],
    );
    h.run_steps(3);
    assert!(
        offset(&mut h) > 0,
        "control: the next delivery's wheel scrolls"
    );
}

/// A two-finger scroll that began over the palette stays the palette's
/// through its end, even after Escape reveals a terminal under the pointer.
/// The next gesture scrolls the terminal.
#[test]
#[ignore = "requires-pty"]
fn a_scroll_begun_on_the_palette_never_scrolls_the_revealed_terminal() {
    let (mut h, _left, right) = two_terminals(SCROLLBACK);
    let at = center(&mut h, right);
    let offset = |h: &mut PlexiUiHarness| {
        with_terminal(h, right, |t| t.backend.last_content().grid.display_offset())
    };
    open_palette(&mut h);
    frame(
        &mut h,
        Modifiers::NONE,
        vec![
            Event::PointerMoved(at),
            wheel_event(0.0, egui::TouchPhase::Start),
            wheel_event(2.0, egui::TouchPhase::Move),
        ],
    );
    chord(&mut h, Modifiers::NONE, Key::Escape);
    assert!(!palette_open(&h));
    frame(
        &mut h,
        Modifiers::NONE,
        vec![wheel_event(5.0, egui::TouchPhase::Move)],
    );
    frame(
        &mut h,
        Modifiers::NONE,
        vec![wheel_event(0.0, egui::TouchPhase::End)],
    );
    h.run_steps(3);
    assert_eq!(offset(&mut h), 0, "the palette's scroll ended with it");

    frame(
        &mut h,
        Modifiers::NONE,
        vec![
            wheel_event(0.0, egui::TouchPhase::Start),
            wheel_event(5.0, egui::TouchPhase::Move),
            wheel_event(0.0, egui::TouchPhase::End),
        ],
    );
    h.run_steps(3);
    assert!(
        offset(&mut h) > 0,
        "control: a fresh scroll gesture scrolls"
    );
}

/// A press a terminal took before the palette opened, as on alpha: released
/// over the terminal after the palette closed, the program hears it; released
/// under the open palette, it does not. Either way a fresh click reports both
/// edges, and the palette never acts on the gesture.
#[test]
#[ignore = "requires-pty"]
fn a_terminal_press_across_the_palette_matches_alpha() {
    let (mut h, _left, right) = two_terminals(MOUSE);
    let at = center(&mut h, right);
    let cmd = Modifiers::COMMAND;

    // Control: press and release report both edges.
    click(&mut h, at);
    assert_eq!(mouse_reports(&tap(&mut h, right)), [true, false], "control");

    // Released over the pane after the palette closed.
    frame(
        &mut h,
        Modifiers::NONE,
        vec![Event::PointerMoved(at), button(at, true)],
    );
    h.run_steps(1);
    assert_eq!(mouse_reports(&tap(&mut h, right)), [true]);
    frame(&mut h, cmd, vec![key(Key::P, cmd, true, false)]);
    frame(&mut h, cmd, vec![key(Key::P, cmd, false, false)]);
    h.run_steps(2);
    assert!(palette_open(&h));
    chord(&mut h, Modifiers::NONE, Key::Escape);
    h.run_steps(3);
    assert!(!palette_open(&h));
    assert!(tap(&mut h, right).is_empty(), "nothing while it was open");
    frame(&mut h, Modifiers::NONE, vec![button(at, false)]);
    h.run_steps(2);
    assert_eq!(
        mouse_reports(&tap(&mut h, right)),
        [false],
        "the release over the pane reached its program"
    );
    assert!(!pointer_down(&mut h));

    // Released under the open palette: off the pane, so unheard.
    frame(
        &mut h,
        Modifiers::NONE,
        vec![Event::PointerMoved(at), button(at, true)],
    );
    h.run_steps(1);
    assert_eq!(mouse_reports(&tap(&mut h, right)), [true]);
    frame(&mut h, cmd, vec![key(Key::P, cmd, true, false)]);
    frame(&mut h, cmd, vec![key(Key::P, cmd, false, false)]);
    h.run_steps(2);
    frame(&mut h, Modifiers::NONE, vec![button(at, false)]);
    h.run_steps(2);
    assert!(palette_open(&h), "the release did not touch the palette");
    assert!(tap(&mut h, right).is_empty(), "unheard under the palette");
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

/// A press on the palette held through Escape: its move and release are the
/// palette's, so the revealed mouse-reporting terminal hears nothing.
#[test]
#[ignore = "requires-pty"]
fn a_press_held_through_dismissal_never_reaches_the_revealed_terminal() {
    let (mut h, _left, right) = two_terminals(MOUSE);
    let at = center(&mut h, right);
    open_palette(&mut h);
    frame(
        &mut h,
        Modifiers::NONE,
        vec![Event::PointerMoved(at), button(at, true)],
    );
    chord(&mut h, Modifiers::NONE, Key::Escape);
    assert!(!palette_open(&h));
    frame(
        &mut h,
        Modifiers::NONE,
        vec![Event::PointerMoved(at + egui::vec2(0.0, 30.0))],
    );
    frame(&mut h, Modifiers::NONE, vec![button(at, false)]);
    h.run_steps(3);
    assert!(
        tap(&mut h, right).is_empty(),
        "no report for the palette's gesture"
    );
    assert!(!pointer_down(&mut h));

    click(&mut h, at);
    assert_eq!(
        mouse_reports(&tap(&mut h, right)),
        [true, false],
        "control: a fresh click reaches the terminal"
    );
}

/// Asks for mouse reports, then on each line from its input turns them off
/// (printing OFF), then back on (printing ON).
const MOUSE_TOGGLE: &str = "printf '\\033[?1000hREADY\\n'; read a; \
     printf '\\033[?1000lOFF\\n'; read b; printf '\\033[?1000hON\\n'; exec cat >/dev/null";

/// Send a line to the program in `pane`, wait until it prints `marker`, and
/// drop the bytes the line put on the tap.
fn answer_and_wait(h: &mut PlexiUiHarness, pane: PaneId, marker: &str) {
    with_terminal(h, pane, |t| {
        t.backend
            .process_command(egui_term::BackendCommand::Write(b"\n".to_vec()))
    });
    let deadline = Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(10));
    loop {
        h.run_steps(2);
        let seen = with_terminal(h, pane, |t| {
            t.backend
                .capture_lines(60)
                .iter()
                .any(|line| line.contains(marker))
        });
        if seen {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "terminal {pane} never printed {marker}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    h.run_steps(2);
    tap(h, pane);
}

/// Round 6 finding 6: a press reported to a mouse-reporting program ends at
/// its release even when the program turned reporting off in between, so a
/// later click in another pane never sends this pane a release.
#[test]
#[ignore = "requires-pty"]
fn a_reported_press_ends_at_its_release_when_mouse_mode_changes_mid_press() {
    let (mut h, left, right) = two_terminals(MOUSE_TOGGLE);
    let a = center(&mut h, left);
    let b = center(&mut h, right);
    frame(
        &mut h,
        Modifiers::NONE,
        vec![Event::PointerMoved(a), button(a, true)],
    );
    h.run_steps(1);
    assert_eq!(
        mouse_reports(&tap(&mut h, left)),
        [true],
        "A reported the press"
    );
    answer_and_wait(&mut h, left, "OFF");
    frame(&mut h, Modifiers::NONE, vec![button(a, false)]);
    h.run_steps(2);
    assert!(tap(&mut h, left).is_empty(), "reporting was off at release");
    answer_and_wait(&mut h, left, "ON");

    click(&mut h, b);
    assert!(
        tap(&mut h, left).is_empty(),
        "B's click sent A no mouse report"
    );
    assert_eq!(
        mouse_reports(&tap(&mut h, right)),
        [true, false],
        "control: B reported its own click"
    );
}

// ── The opener's position and the gate under the scrim (V3 D row 4) ────────

/// Cmd+B before Cmd+P in one delivery was the pane's: the sidebar toggles
/// once and the palette opens. Cmd+B after Cmd+P was the palette's.
#[test]
fn a_chord_before_the_opener_acts_and_one_after_it_does_not() {
    let cmd = Modifiers::COMMAND;
    let (mut h, _, _) = two_views();
    let sidebar = h.with_app(|app| app.sidebar_visible);
    frame(
        &mut h,
        cmd,
        vec![
            key(Key::B, cmd, true, false),
            key(Key::B, cmd, false, false),
            key(Key::P, cmd, true, false),
            key(Key::P, cmd, false, false),
        ],
    );
    h.run_steps(3);
    assert!(palette_open(&h), "Cmd+P opened the palette");
    assert_eq!(
        h.with_app(|app| app.sidebar_visible),
        !sidebar,
        "Cmd+B before the opener toggled the sidebar once"
    );

    let (mut h, _, _) = two_views();
    let sidebar = h.with_app(|app| app.sidebar_visible);
    frame(
        &mut h,
        cmd,
        vec![
            key(Key::P, cmd, true, false),
            key(Key::P, cmd, false, false),
            key(Key::B, cmd, true, false),
            key(Key::B, cmd, false, false),
        ],
    );
    h.run_steps(3);
    assert!(palette_open(&h), "Cmd+P opened the palette");
    assert_eq!(
        h.with_app(|app| app.sidebar_visible),
        sidebar,
        "Cmd+B after the opener belonged to the palette"
    );
}

/// Two terminals in one tab group, and the point on the inactive tab, which
/// lies on the scrim above the palette's frame.
fn tabbed_terminals() -> (PlexiUiHarness, PaneId, PaneId, Pos2) {
    let mut h = PlexiUiHarness::new_sized(1168.0, 800.0);
    h.with_app_mut(|app| app.sidebar_visible = false);
    h.run_steps(2);
    let first = add_terminal(&mut h, MOUSE);
    let second = add_terminal(&mut h, MOUSE);
    h.with_app_mut(|app| {
        let w = app.active_window;
        let tree = &mut app.windows[w].tree;
        let tiles = [first, second].map(|p| tree.tiles.find_pane(&p).expect("pane tile"));
        tree.root = Some(tree.tiles.insert_tab_tile(tiles.to_vec()));
        app.windows[w].focused_pane = Some(tiles[0]);
    });
    h.run_steps(3);
    wait_ready_active(&mut h, &[first, second]);
    let body = center(&mut h, first);
    let rect = h
        .harness()
        .ctx
        .read_response(egui_term::terminal_widget_id(first))
        .expect("rendered terminal widget")
        .rect;
    let tab = egui::pos2(
        rect.left() + rect.width() * 0.75,
        rect.top() - crate::spatial::tiling::TAB_BAR_HEIGHT / 2.0,
    );
    assert!(body.y > tab.y);
    (h, first, second, tab)
}

/// Both tabs' programs are up, without needing them on screen.
fn wait_ready_active(h: &mut PlexiUiHarness, panes: &[PaneId]) {
    let deadline = Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(10));
    for &pane in panes {
        while !with_terminal(h, pane, |t| {
            t.backend.sync();
            t.backend
                .capture_lines(60)
                .iter()
                .any(|line| line.contains("READY"))
        }) {
            assert!(
                Instant::now() < deadline,
                "terminal {pane} never printed READY"
            );
            h.run_steps(2);
            std::thread::sleep(Duration::from_millis(10));
        }
        tap(h, pane);
    }
    h.run_steps(3);
}

/// The panes of the tab group in order, and the active one.
fn tab_state(h: &PlexiUiHarness) -> (Vec<PaneId>, Option<PaneId>) {
    h.with_app(|app| {
        let win = &app.windows[app.active_window];
        let root = win.tree.root.expect("a root tile");
        let Some(egui_tiles::Tile::Container(egui_tiles::Container::Tabs(tabs))) =
            win.tree.tiles.get(root)
        else {
            panic!("the root is the tab group");
        };
        let pane_of = |tile: egui_tiles::TileId| match win.tree.tiles.get(tile) {
            Some(egui_tiles::Tile::Pane(pane)) => Some(*pane),
            _ => None,
        };
        (
            tabs.children.iter().filter_map(|t| pane_of(*t)).collect(),
            tabs.active.and_then(pane_of),
        )
    })
}

/// Round 6 follow-up, V3 D row 4: a click through the palette's scrim on an
/// inactive tab switches nothing, on first open, on ordinary dismissal and
/// on a deferred close. A fresh click on the tab then switches it.
#[test]
#[ignore = "requires-pty"]
fn a_scrim_click_over_an_inactive_tab_never_switches_it() {
    let (mut h, first, second, tab) = tabbed_terminals();
    let before = tab_state(&h);
    assert_eq!(before, (vec![first, second], Some(first)));
    let cmd = Modifiers::COMMAND;
    let unchanged = |h: &mut PlexiUiHarness, label: &str| {
        assert_eq!(tab_state(h), before, "{label}: tabs");
        assert_eq!(focused(h), Some(first), "{label}: pane focus");
        assert!(tap(h, first).is_empty(), "{label}: first terminal's bytes");
        assert!(
            tap(h, second).is_empty(),
            "{label}: second terminal's bytes"
        );
    };

    // First open: the click follows Cmd+P in one delivery.
    frame(
        &mut h,
        cmd,
        vec![
            key(Key::P, cmd, true, false),
            key(Key::P, cmd, false, false),
            Event::PointerMoved(tab),
            button(tab, true),
            button(tab, false),
        ],
    );
    h.run_steps(3);
    assert!(palette_open(&h), "the opening click did not dismiss it");
    unchanged(&mut h, "first open");
    chord(&mut h, Modifiers::NONE, Key::Escape);
    h.run_steps(3);
    assert!(!palette_open(&h));
    unchanged(&mut h, "after Escape");

    // Ordinary dismissal: the scrim click closes the palette.
    open_palette(&mut h);
    frame(
        &mut h,
        Modifiers::NONE,
        vec![Event::PointerMoved(tab), button(tab, true)],
    );
    frame(&mut h, Modifiers::NONE, vec![button(tab, false)]);
    h.run_steps(3);
    assert!(!palette_open(&h), "the scrim click dismissed the palette");
    unchanged(&mut h, "dismissal");

    // Deferred close: the click lands in the pass whose logic runs Enter.
    open_palette(&mut h);
    frame(
        &mut h,
        Modifiers::NONE,
        vec![
            Event::Paste("reset text size".into()),
            key(Key::Enter, Modifiers::NONE, true, false),
            key(Key::Enter, Modifiers::NONE, false, false),
        ],
    );
    frame(
        &mut h,
        Modifiers::NONE,
        vec![
            Event::PointerMoved(tab),
            button(tab, true),
            button(tab, false),
        ],
    );
    h.run_steps(3);
    assert!(!palette_open(&h), "Enter closed the palette");
    unchanged(&mut h, "deferred close");

    // Control: the same click, with nothing above the panes, switches.
    click(&mut h, tab);
    assert_eq!(
        tab_state(&h),
        (vec![first, second], Some(second)),
        "a fresh click on the tab switches it"
    );
}
