//! Consent dialogs take a decision only from a fresh, real key press or click
//! made after the arm delay (stint 1498): injected input, input in the frame
//! the prompt appears, and OS key repeat decide nothing. One shared guard,
//! `consent_input_admitted`, serves the permission banner and the dialogs:
//! event consent, the capability prompt, raw WASM review and keychain
//! recovery (stint 1560). The
//! capability prompt's checks live with its pane in `host::wasm_pane::tests`,
//! which has a loadable test plugin.

use super::pane_lifecycle_consumers::{pump_until, subscriptions, wait_request, Connection};
use crate::broker::consent::ARM_DELAY;
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

pub(crate) fn pointer(pos: Pos2, pressed: bool) -> RawInput {
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
/// row (the approving one, disabled until armed), read from the last pass's
/// widget rects. Rows sense clicks only and are row-sized; selectable body
/// text also senses drags, and the modal frame itself is far taller.
pub(crate) fn first_modal_action(h: &HostHarness) -> Pos2 {
    let full = screen().unwrap();
    h.app.ctx.viewport(|viewport| {
        let widgets = &viewport.prev_pass.widgets;
        widgets
            .layer_ids()
            .filter(|layer| layer.order == egui::Order::Foreground)
            .flat_map(|layer| widgets.get_layer(layer))
            .filter(|w| {
                w.sense.senses_click()
                    && !w.sense.senses_drag()
                    && w.rect.height() <= 2.0 * crate::ui::style::BUTTON_H_MD
                    && w.rect.width() < full.width() / 2.0
            })
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
    let pane = h.add_focused_terminal();
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
    tap(&mut h, Key::Escape);
    assert!(
        h.app.pending_event_consents.is_empty(),
        "a real Esc denies even before the prompt arms"
    );
    assert_eq!(subscriptions(term), 0);
    drop(connection);
}

#[test]
fn event_consent_ignores_a_click_on_a_synthetic_frame() {
    let mut h = HostHarness::new();
    let pane = h.add_focused_terminal();
    h.focus_pane(pane);
    h.run_frames(1);
    let connection = open_event_consent(&mut h, pane);
    h.run_frames(2);
    h.app.event_consent_guard.backdate(ARM_DELAY);
    h.run_frames(1);
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
    let root = std::path::PathBuf::from("/nonexistent/raw-guard-test");
    h.app
        .pending_raw_wasm_launches
        .push_back(crate::app::focus::PendingRawWasmLaunch {
            request_id: crate::broker::consent::next_consent_request_id(),
            plugin_id: "raw-guard-test".to_string(),
            wasm_path: root.join("missing.wasm"),
            workspace_root: root.clone(),
            missing_capabilities: vec!["fs:read:/tmp".to_string()],
            launch_args: Vec::new(),
        });
}

#[test]
fn raw_wasm_review_takes_only_a_fresh_real_enter_after_the_arm_delay() {
    let mut h = HostHarness::new();
    let pane = h.add_focused_terminal();
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

#[test]
fn event_consent_refusal_works_at_once_from_real_input_but_not_injected() {
    let mut h = HostHarness::new();
    let pane = h.add_focused_terminal();
    h.focus_pane(pane);
    h.run_frames(1);
    let connection = open_event_consent(&mut h, pane);
    h.run_frames(1);

    inject_key(&mut h, pane, Key::Escape);
    assert_eq!(
        h.app.pending_event_consents.len(),
        1,
        "an injected Esc denied"
    );
    tap(&mut h, Key::Escape);
    assert!(
        h.app.pending_event_consents.is_empty(),
        "a real Esc denies inside the arm delay"
    );
    assert_eq!(subscriptions(pane), 0);
    drop(connection);
}

#[test]
fn identical_event_consents_queued_together_each_arm_on_their_own() {
    let mut h = HostHarness::new();
    let pane = h.add_focused_terminal();
    h.focus_pane(pane);
    h.run_frames(1);
    // Two identical publish requests from the same pane: same actor id, verb
    // and target, so a guard keyed on content could not tell them apart.
    // Pumped with visible frames only, so the dialog is never off screen.
    let declare = serde_json::json!({
        "type": "events_declare",
        "plugin_id": "guard-check",
        "streams": [{"name": "probe.tick", "schema": {}}],
        "from_pane_id": pane,
    });
    let pump = |h: &mut HostHarness, n: usize| {
        let deadline = std::time::Instant::now()
            + crate::testing::load_aware_timeout(std::time::Duration::from_secs(5));
        while h.app.pending_event_consents.len() < n {
            h.run_frames(1);
            assert!(
                std::time::Instant::now() < deadline,
                "consent {n} never queued"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    };
    let first = Connection::open(&mut h, declare.clone());
    pump(&mut h, 1);
    let second = Connection::open(&mut h, declare);
    pump(&mut h, 2);
    h.app.event_consent_guard.backdate(ARM_DELAY);

    tap(&mut h, Key::Enter);
    assert_eq!(
        h.app.pending_event_consents.len(),
        1,
        "the armed first request is granted"
    );
    tap(&mut h, Key::Enter);
    assert_eq!(
        h.app.pending_event_consents.len(),
        1,
        "the identical second request inherited the first one's arm state"
    );
    tap(&mut h, Key::Escape);
    assert!(
        h.app.pending_event_consents.is_empty(),
        "a real Esc denies the second"
    );
    drop(first);
    drop(second);
}

#[test]
fn identical_raw_wasm_reviews_queued_together_each_arm_on_their_own() {
    let mut h = HostHarness::new();
    let pane = h.add_focused_terminal();
    h.focus_pane(pane);
    h.run_frames(1);
    queue_raw_wasm_review(&mut h);
    queue_raw_wasm_review(&mut h);
    h.run_frames(1);
    h.app.raw_wasm_review_guard.backdate(ARM_DELAY);

    // The first is denied; the identical second shows on the very next frame.
    tap(&mut h, Key::Escape);
    assert_eq!(
        h.app.pending_raw_wasm_launches.len(),
        1,
        "Esc denies the first"
    );
    tap(&mut h, Key::Enter);
    assert_eq!(
        h.app.pending_raw_wasm_launches.len(),
        1,
        "the identical second review inherited the first one's arm state"
    );
}

#[test]
fn raw_wasm_review_re_arms_after_the_window_was_hidden() {
    let mut h = HostHarness::new();
    let pane = h.add_focused_terminal();
    h.focus_pane(pane);
    h.run_frames(1);
    queue_raw_wasm_review(&mut h);
    h.run_frames(1);
    h.app.raw_wasm_review_guard.backdate(ARM_DELAY);
    h.run_hidden_frames(3);

    tap(&mut h, Key::Enter);
    assert_eq!(
        h.app.pending_raw_wasm_launches.len(),
        1,
        "the review was armed the instant the window came back"
    );
}

#[test]
fn raw_wasm_review_ignores_a_click_on_a_synthetic_frame() {
    let mut h = HostHarness::new();
    let pane = h.add_focused_terminal();
    h.focus_pane(pane);
    h.run_frames(1);
    queue_raw_wasm_review(&mut h);
    h.run_frames(2);
    h.app.raw_wasm_review_guard.backdate(ARM_DELAY);
    h.run_frames(1);
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
        h.app.pending_raw_wasm_launches.len(),
        1,
        "an injected click allowed"
    );

    h.frame(pointer(allow, true));
    h.frame(pointer(allow, false));
    assert!(
        h.app.pending_raw_wasm_launches.is_empty(),
        "a real click on Allow and remember must still allow"
    );
}

#[test]
fn raw_wasm_review_ignores_a_real_click_inside_the_arm_delay() {
    let mut h = HostHarness::new();
    let pane = h.add_focused_terminal();
    h.focus_pane(pane);
    h.run_frames(1);
    queue_raw_wasm_review(&mut h);
    h.run_frames(1);
    let allow = first_modal_action(&h);

    h.frame(pointer(allow, true));
    h.frame(pointer(allow, false));
    assert_eq!(
        h.app.pending_raw_wasm_launches.len(),
        1,
        "a real click inside the arm delay allowed"
    );
}

/// ActionModal stacks the full-width event consent actions when they overflow
/// one row. The armed and unarmed screenshots show Allow once, Allow until
/// this pane closes, and Deny from top to bottom.
#[test]
fn screenshot_event_consent_button_row() {
    let mut h = HostHarness::new();
    let pane = h.add_focused_terminal();
    h.focus_pane(pane);
    h.run_frames(1);
    let connection = open_event_consent(&mut h, pane);
    h.run_frames(2);
    h.render_png(crate::ui_tests::evidence_png!("_arming"))
        .unwrap();
    h.app.event_consent_guard.backdate(ARM_DELAY);
    h.run_frames(2);
    h.render_png(crate::ui_tests::evidence_png!("_armed"))
        .unwrap();
    let rows: Vec<(String, egui::Rect)> = crate::ui::layout_report::snapshot(&h.app.ctx)
        .into_iter()
        .filter(|r| r.surface == "modal_action")
        .flat_map(|r| r.actions)
        .map(|a| (a.id, a.hit))
        .collect();
    let ids: Vec<&str> = rows.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids, ["allow_once", "allow_pane", "deny"], "paint order");
    assert!(rows.iter().all(|(_, hit)| hit.left() == rows[0].1.left()));
    assert!(rows[0].1.top() < rows[1].1.top() && rows[1].1.top() < rows[2].1.top());
    drop(connection);
}

/// File the keychain recovery record startup files when it could not read
/// the seal key. Tests run against the in-memory host-key store.
fn file_keychain_recovery() -> (
    std::sync::Arc<crate::broker::gate::PermissionMonitor>,
    String,
) {
    let monitor =
        crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir()).unwrap();
    let id = monitor
        .file_needs_you(crate::broker::gate::NeedsYouFile {
            kind: crate::broker::gate::NeedsYouKind::Keychain,
            actor: crate::broker::principal::RecordOwner::Host("host".to_string()),
            resource: "permission-mac".to_string(),
            summary: crate::broker::gate::KEYCHAIN_ACCESS_SUMMARY.to_string(),
            expires_at: None,
            run_tag: Some("keychain:permission-mac".to_string()),
        })
        .expect("file keychain recovery");
    (monitor, id)
}

fn keychain_resolution(
    monitor: &crate::broker::gate::PermissionMonitor,
    id: &str,
) -> Option<crate::broker::gate::NeedsYouResolution> {
    monitor
        .host_records()
        .question(id)
        .and_then(|record| record.resolution)
}

/// Stint 1560: the banner advertises Enter and Esc only while the keys can
/// reach it, that is while nothing owns input.
#[test]
fn keychain_recovery_shows_key_chips_only_while_keys_are_live() {
    let live = crate::app::plugin_call::keychain_recovery_actions(true);
    assert!(live.iter().all(|action| action.shortcut.is_some()));
    let owned = crate::app::plugin_call::keychain_recovery_actions(false);
    assert!(owned.iter().all(|action| action.shortcut.is_none()));
}

/// Stint 1560: with a terminal focused the banner stays non-blocking. Esc and
/// Enter go to the terminal and decide nothing.
#[test]
fn keychain_recovery_leaves_keys_to_a_focused_terminal() {
    use crate::app::input_owner::InputOwner;
    let mut h = HostHarness::new();
    let term = h.add_focused_terminal();
    h.terminal_backend(term).enable_input_tap();
    let (monitor, id) = file_keychain_recovery();
    h.run_frames(2);
    h.app.keychain_recovery_guard.backdate(ARM_DELAY);
    h.run_frames(1);
    h.render_png(crate::ui_tests::evidence_png!())
        .expect("render keychain banner over a focused terminal");

    tap(&mut h, Key::Escape);
    tap(&mut h, Key::Enter);
    h.run_frames(1);
    assert_eq!(keychain_resolution(&monitor, &id), None);
    assert_eq!(h.app.host_input_owner(), InputOwner::Pane(term));
    assert!(
        !h.terminal_backend(term).take_input_tap().is_empty(),
        "the terminal kept its keys"
    );
}

/// Unfocus every pane so nothing owns input and the banner's keys go live.
fn release_input(h: &mut HostHarness) {
    h.app.windows[0].focused_pane = None;
    h.run_frames(1);
    assert_eq!(
        h.app.host_input_owner(),
        crate::app::input_owner::InputOwner::Nobody
    );
}

/// Stint 1560: with nothing owning input, Esc defers at once.
#[test]
fn keychain_recovery_esc_defers_when_nothing_owns_input() {
    let mut h = HostHarness::new();
    h.add_focused_terminal();
    let (monitor, id) = file_keychain_recovery();
    release_input(&mut h);

    tap(&mut h, Key::Escape);
    h.run_frames(1);
    assert_eq!(
        keychain_resolution(&monitor, &id),
        Some(crate::broker::gate::NeedsYouResolution::Denied)
    );
}

/// Stint 1560: Enter continues only from a fresh, real press made after the
/// arm delay, and the delay restarts when the keys go live, so `ls⏎` typed
/// into a terminal as focus leaves it never asks macOS.
#[test]
fn keychain_recovery_enter_continues_only_once_armed() {
    let mut h = HostHarness::new();
    h.add_focused_terminal();
    let (monitor, id) = file_keychain_recovery();
    h.run_frames(2);
    // The banner has been up long enough while the terminal owned the keys.
    h.app.keychain_recovery_guard.backdate(ARM_DELAY);
    release_input(&mut h);

    hold(&mut h, Key::Enter);
    assert_eq!(
        keychain_resolution(&monitor, &id),
        None,
        "Enter right after the keys went live"
    );
    h.app.keychain_recovery_guard.backdate(ARM_DELAY);
    repeat_then_release(&mut h, Key::Enter);
    assert_eq!(keychain_resolution(&monitor, &id), None, "a key repeat");

    tap(&mut h, Key::Enter);
    h.run_frames(1);
    assert_eq!(
        keychain_resolution(&monitor, &id),
        Some(crate::broker::gate::NeedsYouResolution::Approved)
    );
}
