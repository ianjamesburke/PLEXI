//! A key held when a terminal stops owning the keyboard (window focus loss,
//! focus moving to another pane, the pane closing) is released to the pane
//! that held it, and its late physical release reaches no other pane
//! (stint 1564). The fixture requests kitty release events (`CSI > 3 u`).

use super::terminal_page_scroll_tests::{release_key, terminal_with_fixture};
use crate::spatial::tiling::PaneId;
use crate::testing::HostHarness;

const PRESS: &str = "\x1b[5;1:1~";
const RELEASE: &str = "\x1b[5;1:3~";

fn tap(h: &mut HostHarness, id: PaneId) -> String {
    String::from_utf8_lossy(&h.terminal_backend(id).take_input_tap()).into_owned()
}

fn kitty_terminal(h: &mut HostHarness) -> PaneId {
    let id = terminal_with_fixture(h, "kitty-types");
    assert!(
        h.terminal_backend(id)
            .sync()
            .terminal_mode
            .contains(egui_term::TerminalMode::REPORT_EVENT_TYPES),
        "fixture must have requested kitty release events"
    );
    h.terminal_backend(id).enable_input_tap();
    id
}

/// Two kitty terminals, `a` focused and settled.
fn two_terminals(h: &mut HostHarness) -> (PaneId, PaneId) {
    let a = kitty_terminal(h);
    let b = kitty_terminal(h);
    h.focus_pane(a);
    h.run_frames(3);
    tap(h, a);
    tap(h, b);
    (a, b)
}

/// The harness's first (non-terminal) pane.
fn other_pane(h: &HostHarness, id: PaneId) -> PaneId {
    *h.app.windows[0]
        .panes
        .keys()
        .find(|pane| **pane != id)
        .expect("the harness's first pane")
}

fn press_page_up(h: &mut HostHarness, id: PaneId) {
    // Held: the release comes later, from the test.
    h.key_down(egui::Key::PageUp, egui::Modifiers::NONE);
    h.run_frames(1);
    assert_eq!(
        tap(h, id),
        PRESS,
        "the held key's press reaches the program"
    );
}

#[test]
#[ignore = "requires-pty"]
fn window_focus_loss_releases_a_held_key_once() {
    let mut h = HostHarness::new();
    let id = kitty_terminal(&mut h);
    press_page_up(&mut h, id);
    for _ in 0..2 {
        h.frame(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            focused: false,
            viewports: std::iter::once((
                egui::ViewportId::ROOT,
                egui::ViewportInfo {
                    focused: Some(false),
                    ..Default::default()
                },
            ))
            .collect(),
            events: vec![egui::Event::WindowFocused(false)],
            ..Default::default()
        });
    }
    assert_eq!(
        tap(&mut h, id),
        RELEASE,
        "exactly one release on focus loss"
    );
    // Refocus, then the physical release arrives: the key is no longer held.
    h.run_frames(2);
    release_key(&mut h, egui::Key::PageUp, egui::Modifiers::NONE);
    h.run_frames(1);
    assert_eq!(tap(&mut h, id), "", "no stray release after refocus");
}

#[test]
#[ignore = "requires-pty"]
fn focus_switch_releases_a_held_key_to_the_pane_that_held_it() {
    let mut h = HostHarness::new();
    let (a, b) = two_terminals(&mut h);
    press_page_up(&mut h, a);
    h.focus_pane(b);
    h.run_frames(2);
    release_key(&mut h, egui::Key::PageUp, egui::Modifiers::NONE);
    h.run_frames(1);
    assert_eq!(tap(&mut h, b), "", "the new pane never saw the press");
    assert_eq!(tap(&mut h, a), RELEASE, "the holding pane gets one release");
}

#[test]
#[ignore = "requires-pty"]
fn closing_the_holding_pane_sends_no_stray_release_to_its_successor() {
    let mut h = HostHarness::new();
    let (a, b) = two_terminals(&mut h);
    press_page_up(&mut h, a);
    h.app.close_pane_by_id(a);
    h.focus_pane(b);
    h.run_frames(2);
    release_key(&mut h, egui::Key::PageUp, egui::Modifiers::NONE);
    h.run_frames(1);
    assert_eq!(tap(&mut h, b), "", "the successor never saw the press");
}

/// The holding pane stops rendering (another pane zoomed over it) at the
/// same moment focus leaves it: the release must not wait for a render.
#[test]
#[ignore = "requires-pty"]
fn focus_switch_to_a_zoomed_pane_releases_the_hidden_holder() {
    let mut h = HostHarness::new();
    let (a, b) = two_terminals(&mut h);
    press_page_up(&mut h, a);
    h.focus_pane(b);
    h.app.toggle_pane_zoom();
    h.run_frames(2);
    release_key(&mut h, egui::Key::PageUp, egui::Modifiers::NONE);
    h.run_frames(1);
    assert_eq!(tap(&mut h, b), "", "the zoomed pane never saw the press");
    assert_eq!(
        tap(&mut h, a),
        RELEASE,
        "the hidden holder gets one release"
    );
}

/// A kitty terminal that is hovered but not focused must not record a key
/// meant for the focused (non-terminal) pane, so it is never sent a release
/// for a press it was not given.
#[test]
#[ignore = "requires-pty"]
fn hovered_unfocused_terminal_records_no_hold() {
    let mut h = HostHarness::new();
    let id = kitty_terminal(&mut h);
    let other = other_pane(&h, id);
    h.focus_pane(other);
    h.run_frames(3);
    let rect = h
        .app
        .ctx
        .read_response(egui_term::terminal_widget_id(id))
        .expect("the terminal widget rendered")
        .rect;
    let screen = Some(egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(1280.0, 800.0),
    ));
    h.frame(egui::RawInput {
        screen_rect: screen,
        events: vec![egui::Event::PointerMoved(rect.center())],
        ..Default::default()
    });
    h.frame(egui::RawInput {
        screen_rect: screen,
        events: vec![
            egui::Event::PointerMoved(rect.center()),
            egui::Event::Key {
                key: egui::Key::PageUp,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            },
        ],
        ..Default::default()
    });
    h.run_frames(2);
    assert_eq!(tap(&mut h, id), "", "an unfocused terminal sends nothing");
}

/// Keys the terminal consumes itself (Cmd+F entering search, then a key
/// typed into search) never reached the program, so losing focus sends no
/// release for them.
#[test]
#[ignore = "requires-pty"]
fn locally_consumed_keys_get_no_release() {
    let mut h = HostHarness::new();
    let id = kitty_terminal(&mut h);
    let cmd = egui::Modifiers {
        mac_cmd: true,
        command: true,
        ..egui::Modifiers::NONE
    };
    h.press_key(egui::Key::F, cmd);
    release_key(&mut h, egui::Key::F, cmd);
    h.press_key(egui::Key::PageUp, egui::Modifiers::NONE);
    h.run_frames(1);
    assert_eq!(tap(&mut h, id), "", "search consumed both presses");
    let other = other_pane(&h, id);
    h.focus_pane(other);
    h.run_frames(2);
    assert_eq!(tap(&mut h, id), "", "no release for a press never sent");
}

fn key_event(key: egui::Key, modifiers: egui::Modifiers, pressed: bool) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers,
    }
}

/// Several key events buffered into one frame (a delayed frame).
fn one_frame(h: &mut HostHarness, events: Vec<egui::Event>) {
    h.frame(egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1280.0, 800.0),
        )),
        events,
        ..Default::default()
    });
}

const CMD: egui::Modifiers = egui::Modifiers {
    mac_cmd: true,
    command: true,
    ..egui::Modifiers::NONE
};

/// Cmd+A is consumed by the terminal (select-all copy); its release in the
/// same frame must not reach the program either.
#[test]
#[ignore = "requires-pty"]
fn locally_consumed_press_and_release_in_one_frame_send_nothing() {
    let mut h = HostHarness::new();
    let id = kitty_terminal(&mut h);
    one_frame(
        &mut h,
        vec![
            key_event(egui::Key::A, CMD, true),
            key_event(egui::Key::A, CMD, false),
        ],
    );
    h.run_frames(1);
    assert_eq!(
        tap(&mut h, id),
        "",
        "neither Cmd+A press nor release is sent"
    );
}

/// A consumed Cmd+A followed in the same frame by a delivered Ctrl+A: the
/// Ctrl+A hold survives, so losing focus releases it.
#[test]
#[ignore = "requires-pty"]
fn consumed_press_does_not_drop_a_later_delivered_hold_in_one_frame() {
    let mut h = HostHarness::new();
    let id = kitty_terminal(&mut h);
    one_frame(
        &mut h,
        vec![
            key_event(egui::Key::A, CMD, true),
            key_event(egui::Key::A, CMD, false),
            key_event(egui::Key::A, egui::Modifiers::CTRL, true),
        ],
    );
    h.run_frames(1);
    assert_eq!(
        tap(&mut h, id),
        "\x1b[97;5:1u",
        "only Ctrl+A's press is sent"
    );
    let other = other_pane(&h, id);
    h.focus_pane(other);
    h.run_frames(2);
    assert_eq!(
        tap(&mut h, id),
        "\x1b[97;5:3u",
        "Ctrl+A is released on focus loss"
    );
}
