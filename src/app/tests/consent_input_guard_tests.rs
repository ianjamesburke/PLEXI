//! Consent dialogs take a decision only from a fresh, real key press or click
//! made after the arm delay (stint 1498): injected input, input in the frame
//! the prompt appears, and OS key repeat decide nothing. One shared guard,
//! `consent_input_admitted`, serves the permission banner and all three
//! dialogs: event consent, the capability prompt and raw WASM review. The
//! capability prompt's checks live with its pane in `host::wasm_pane::tests`,
//! which has a loadable test plugin.

use super::pane_lifecycle_consumers::{pump_until, subscriptions, wait_request, Connection};
use crate::app::approval_keyboard::ARM_DELAY;
use crate::spatial::tiling::PaneId;
use crate::testing::HostHarness;
use egui::{Event, Key, Modifiers, PointerButton, Pos2, RawInput};

fn screen() -> Option<egui::Rect> {
    Some(egui::Rect::from_min_size(
        Pos2::ZERO,
        egui::vec2(1280.0, 800.0),
    ))
}

fn key_event(key: Key, pressed: bool, repeat: bool) -> Event {
    Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat,
        modifiers: Modifiers::NONE,
    }
}

fn keys(events: Vec<Event>) -> RawInput {
    RawInput {
        screen_rect: screen(),
        events,
        ..Default::default()
    }
}

/// A key pressed and held (egui marks later presses of a held key as repeats).
fn hold(h: &mut HostHarness, k: Key) {
    h.frame(keys(vec![key_event(k, true, false)]));
}

/// An OS auto-repeat of a held key, then its release.
fn repeat_then_release(h: &mut HostHarness, k: Key) {
    h.frame(keys(vec![key_event(k, true, true)]));
    h.frame(keys(vec![key_event(k, false, false)]));
}

/// A real, fresh press and release.
fn tap(h: &mut HostHarness, k: Key) {
    h.frame(keys(vec![key_event(k, true, false)]));
    h.frame(keys(vec![key_event(k, false, false)]));
}

fn pointer(pos: Pos2, pressed: bool) -> RawInput {
    RawInput {
        screen_rect: screen(),
        events: vec![
            Event::PointerMoved(pos),
            Event::PointerButton {
                pos,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            },
        ],
        ..Default::default()
    }
}

/// Queue a key through the pane-IPC injection path: `raw_input_hook` merges it
/// into the next frame and flags that frame synthetic.
fn inject_key(h: &mut HostHarness, pane: PaneId, k: Key) {
    h.app
        .pending_pane_inputs
        .entry(pane)
        .or_default()
        .push(keys(vec![
            key_event(k, true, false),
            key_event(k, false, false),
        ]));
    h.run_frames(1);
    assert!(
        h.app
            .pending_pane_inputs
            .get(&pane)
            .is_none_or(Vec::is_empty),
        "the injected key must have been delivered into a frame"
    );
}

/// The topmost clickable action of the open consent modal: its first action
/// row (the approving one), read from the last pass's widget rects.
fn first_modal_action(h: &HostHarness) -> Pos2 {
    let full = screen().unwrap();
    h.app.ctx.viewport(|viewport| {
        let widgets = &viewport.prev_pass.widgets;
        widgets
            .layer_ids()
            .filter(|layer| layer.order == egui::Order::Foreground)
            .flat_map(|layer| widgets.get_layer(layer))
            .filter(|w| w.enabled && w.sense.senses_click() && w.rect.width() < full.width() / 2.0)
            .min_by(|a, b| a.rect.top().total_cmp(&b.rect.top()))
            .map(|w| w.interact_rect.center())
            .expect("an open consent modal exposes clickable actions")
    })
}

fn open_event_consent(h: &mut HostHarness, pane: PaneId) -> Connection {
    let connection = Connection::open(h, wait_request(pane, pane, "blocked", 5.0));
    pump_until(h, |h| !h.app.pending_event_consents.is_empty());
    connection
}

#[test]
fn event_consent_takes_only_a_fresh_real_enter_after_the_arm_delay() {
    let mut h = HostHarness::new();
    let pane = h.add_test_pane();
    h.focus_pane(pane);
    h.run_frames(1);
    let connection = open_event_consent(&mut h, pane);

    hold(&mut h, Key::Enter);
    assert_eq!(
        h.app.pending_event_consents.len(),
        1,
        "Enter inside the arm delay granted"
    );
    h.app.event_consent_guard.backdate(ARM_DELAY);
    repeat_then_release(&mut h, Key::Enter);
    assert_eq!(
        h.app.pending_event_consents.len(),
        1,
        "a key repeat granted"
    );

    inject_key(&mut h, pane, Key::Enter);
    assert_eq!(
        h.app.pending_event_consents.len(),
        1,
        "an injected Enter granted"
    );
    inject_key(&mut h, pane, Key::A);
    assert_eq!(
        h.app.pending_event_consents.len(),
        1,
        "an injected A granted always"
    );

    tap(&mut h, Key::Enter);
    assert!(
        h.app.pending_event_consents.is_empty(),
        "a real fresh Enter must still grant"
    );
    assert_eq!(
        subscriptions(pane),
        1,
        "Allow once records the subscription"
    );
    drop(connection);
}

#[test]
fn event_consent_appearing_under_a_focused_terminal_ignores_the_keystroke_in_flight() {
    let mut h = HostHarness::new();
    let term = h.add_focused_terminal();
    let connection = open_event_consent(&mut h, term);

    tap(&mut h, Key::Enter);
    assert_eq!(
        h.app.pending_event_consents.len(),
        1,
        "the Enter typed for the shell answered the prompt that just appeared"
    );
    h.app.event_consent_guard.backdate(ARM_DELAY);
    tap(&mut h, Key::Escape);
    assert!(
        h.app.pending_event_consents.is_empty(),
        "a deliberate Esc still denies"
    );
    assert_eq!(subscriptions(term), 0);
    drop(connection);
}

#[test]
fn event_consent_ignores_a_click_on_a_synthetic_frame() {
    let mut h = HostHarness::new();
    let pane = h.add_test_pane();
    h.focus_pane(pane);
    h.run_frames(1);
    let connection = open_event_consent(&mut h, pane);
    h.run_frames(2);
    h.app.event_consent_guard.backdate(ARM_DELAY);
    let allow = first_modal_action(&h);

    let frames = h.app.pending_pane_pointer_frames.entry(pane).or_default();
    frames.push_back(pointer(allow, true));
    frames.push_back(pointer(allow, false));
    h.run_frames(3);
    assert!(h
        .app
        .pending_pane_pointer_frames
        .get(&pane)
        .is_none_or(|f| f.is_empty()));
    assert_eq!(
        h.app.pending_event_consents.len(),
        1,
        "an injected click granted"
    );

    h.frame(pointer(allow, true));
    h.frame(pointer(allow, false));
    assert!(
        h.app.pending_event_consents.is_empty(),
        "a real click must still grant"
    );
    assert_eq!(subscriptions(pane), 1, "the click landed on Allow once");
    drop(connection);
}

fn queue_raw_wasm_review(h: &mut HostHarness) {
    let root = tempfile::tempdir().unwrap();
    h.app
        .pending_raw_wasm_launches
        .push_back(crate::app::focus::PendingRawWasmLaunch {
            plugin_id: "raw-guard-test".to_string(),
            wasm_path: root.path().join("missing.wasm"),
            workspace_root: root.path().to_path_buf(),
            missing_capabilities: vec!["fs:read:/tmp".to_string()],
            launch_args: Vec::new(),
        });
}

#[test]
fn raw_wasm_review_takes_only_a_fresh_real_enter_after_the_arm_delay() {
    let mut h = HostHarness::new();
    let pane = h.add_test_pane();
    h.focus_pane(pane);
    h.run_frames(1);
    queue_raw_wasm_review(&mut h);

    hold(&mut h, Key::Enter);
    assert_eq!(
        h.app.pending_raw_wasm_launches.len(),
        1,
        "Enter inside the arm delay allowed"
    );
    h.app.raw_wasm_review_guard.backdate(ARM_DELAY);
    repeat_then_release(&mut h, Key::Enter);
    assert_eq!(
        h.app.pending_raw_wasm_launches.len(),
        1,
        "a key repeat allowed"
    );

    inject_key(&mut h, pane, Key::Enter);
    assert_eq!(
        h.app.pending_raw_wasm_launches.len(),
        1,
        "an injected Enter allowed"
    );

    tap(&mut h, Key::Enter);
    assert!(
        h.app.pending_raw_wasm_launches.is_empty(),
        "a real fresh Enter must still allow"
    );
}
