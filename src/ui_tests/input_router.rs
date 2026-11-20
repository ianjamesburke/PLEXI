//! Host input router witnesses (stint 1659).
//!
//! Every test drives `HostHarness::frame`, so input enters through
//! `PlexiApp::raw_input_hook` exactly as eframe delivers it. A delivery is one
//! `RawInput`; the router must give each actionable event one owner at its
//! ordinal, commit owner changes before the next ordinal, and consume the rest
//! of a delivery after an overlay is dismissed (the dismissal barrier).
//!
//! The panes under test are `EdgeProbe` views: they record every key edge and
//! text event their structured `handle_key` receives, so a lost or duplicated
//! key-up is observable (a PTY that only echoes characters cannot show one).

use crate::app::permissions::PluginPermissions;
use crate::app::FocusKind;
use crate::spatial::tiling::PaneId;
use crate::testing::HostHarness;
use egui::{Event, Key, Modifiers, PointerButton, Pos2, RawInput};

/// Records the structured key stream a pane receives, in order.
#[derive(Default)]
struct EdgeProbe {
    log: Vec<String>,
}

impl crate::app::view_trait::View for EdgeProbe {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn type_id(&self) -> &'static str {
        "edge-probe"
    }

    fn display_name(&self) -> String {
        "Edge Probe".to_string()
    }

    /// A pointer click egui delivers to the pane's own widgets is recorded
    /// too: a click that leaks behind a modal is observable here even when
    /// host focus does not move.
    fn ui(&mut self, ui: &mut egui::Ui, ctx: &crate::app::view_trait::ViewRenderContext<'_>) {
        // Keyboard input egui hands this pane's widgets while it owns input:
        // after the plugin consumed an event, any copy here is a second
        // delivery.
        if crate::app::input_owner::published_frame_owner_is_pane(ui.ctx(), ctx.pane_id)
            == Some(true)
        {
            let leaked: Vec<String> = ui.input(|i| {
                i.events
                    .iter()
                    .filter_map(|event| match event {
                        // Releases are folded for key-state balance when egui
                        // folded the press, whoever owns input now.
                        Event::Key {
                            key, pressed: true, ..
                        } => Some(format!("egui:down:{key:?}")),
                        Event::Text(text) => Some(format!("egui:text:{text}")),
                        Event::Paste(_) => Some("egui:paste".to_string()),
                        _ => None,
                    })
                    .collect()
            });
            self.log.extend(leaked);
        }
        let r = ui.allocate_rect(ui.max_rect(), egui::Sense::click());
        if r.clicked() {
            self.log.push("click".to_string());
        }
    }

    fn handle_key(
        &mut self,
        input: &crate::app::input_router::PlexiInput,
    ) -> crate::app::view_trait::KeyDisposition {
        for event in input.events() {
            match event {
                Event::Key { key, pressed, .. } => self
                    .log
                    .push(format!("{}:{key:?}", if *pressed { "down" } else { "up" })),
                Event::Text(text) => self.log.push(format!("text:{text}")),
                Event::Paste(text) => self.log.push(format!("paste:{text}")),
                Event::Ime(egui::ImeEvent::Commit(text)) => self.log.push(format!("ime:{text}")),
                _ => {}
            }
        }
        crate::app::view_trait::KeyDisposition::Consumed
    }
}

/// The native macOS Command chord shape: winit sets both `command` and
/// `mac_cmd`, which raw equality with `Modifiers::COMMAND` rejects.
fn native_cmd() -> Modifiers {
    Modifiers {
        command: true,
        mac_cmd: cfg!(target_os = "macos"),
        ctrl: !cfg!(target_os = "macos"),
        ..Modifiers::NONE
    }
}

fn native_cmd_alt() -> Modifiers {
    Modifiers {
        alt: true,
        ..native_cmd()
    }
}

fn down(key: Key, modifiers: Modifiers) -> Event {
    Event::Key {
        key,
        physical_key: Some(key),
        pressed: true,
        repeat: false,
        modifiers,
    }
}

fn up(key: Key, modifiers: Modifiers) -> Event {
    Event::Key {
        key,
        physical_key: Some(key),
        pressed: false,
        repeat: false,
        modifiers,
    }
}

fn text(t: &str) -> Event {
    Event::Text(t.to_string())
}

fn moved(pos: Pos2) -> Event {
    Event::PointerMoved(pos)
}

fn button(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    }
}

fn click(pos: Pos2) -> Vec<Event> {
    vec![moved(pos), button(pos, true), button(pos, false)]
}

fn screen() -> Option<egui::Rect> {
    Some(egui::Rect::from_min_size(
        Pos2::ZERO,
        egui::vec2(1280.0, 800.0),
    ))
}

/// One delivery: every event in one `RawInput`, the way one native
/// `raw_input_hook` call receives a batch.
fn deliver(h: &mut HostHarness, events: Vec<Event>) {
    let modifiers = events
        .iter()
        .rev()
        .find_map(|event| match event {
            Event::Key { modifiers, .. } => Some(*modifiers),
            _ => None,
        })
        .unwrap_or_default();
    h.frame(RawInput {
        screen_rect: screen(),
        modifiers,
        events,
        ..Default::default()
    });
    // A split delivery finishes on the following passes (the router requests
    // them at once in production).
    for _ in 0..16 {
        if !h.app.input_router.has_held_input() {
            return;
        }
        h.run_frames(1);
    }
    panic!("held input never drained");
}

fn probe_pane(h: &mut HostHarness) -> PaneId {
    h.add_builtin_view_pane(Box::new(EdgeProbe::default()), PluginPermissions::builtin())
}

fn tile_of(h: &HostHarness, pane: PaneId) -> egui_tiles::TileId {
    h.app.windows[0]
        .tree
        .tiles
        .iter()
        .find_map(|(tile, t)| matches!(t, egui_tiles::Tile::Pane(p) if *p == pane).then_some(*tile))
        .expect("pane tile")
}

/// Two side-by-side probe panes, the first focused and both presented.
fn two_probes(h: &mut HostHarness) -> (PaneId, PaneId) {
    let a = probe_pane(h);
    let b = probe_pane(h);
    let (ta, tb) = (tile_of(h, a), tile_of(h, b));
    let win = &mut h.app.windows[0];
    let root = win.tree.tiles.insert_horizontal_tile(vec![ta, tb]);
    win.tree.root = Some(root);
    h.focus_pane(a);
    h.run_frames(3);
    probe_log(h, a);
    probe_log(h, b);
    (a, b)
}

fn probe_log(h: &mut HostHarness, pane: PaneId) -> Vec<String> {
    let view = h.app.windows[0]
        .panes
        .get_mut(&pane)
        .and_then(|p| p.as_view_mut())
        .expect("probe pane");
    let crate::host::pane::ViewRuntime::Builtin(app) = &mut view.runtime else {
        panic!("probe is builtin");
    };
    let probe = app
        .as_any_mut()
        .downcast_mut::<EdgeProbe>()
        .expect("pane is an EdgeProbe");
    std::mem::take(&mut probe.log)
}

fn focused(h: &HostHarness) -> Option<PaneId> {
    let win = &h.app.windows[0];
    match win.focused_pane.and_then(|t| win.tree.tiles.get(t)) {
        Some(egui_tiles::Tile::Pane(p)) => Some(*p),
        _ => None,
    }
}

fn center(h: &HostHarness, pane: PaneId) -> Pos2 {
    h.app.windows[0]
        .tree
        .tiles
        .rect(tile_of(h, pane))
        .expect("presented pane rect")
        .center()
}

fn open_palette(h: &mut HostHarness) {
    deliver(h, vec![down(Key::P, native_cmd())]);
    deliver(h, vec![up(Key::P, Modifiers::NONE)]);
    assert!(h.app.show_command_palette, "Cmd+P opens the palette");
}

fn pending_request(
    root: &std::path::Path,
) -> (
    std::sync::Arc<crate::broker::gate::PermissionMonitor>,
    String,
) {
    let monitor =
        crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir()).unwrap();
    let result = crate::workspace::secrets::read_pane_secret(
        &monitor,
        &crate::workspace::secrets::InMemoryKeychain::new(),
        &crate::workspace::secrets::SecretActor {
            kind: crate::broker::ActorType::Agent,
            id: "pane:4".into(),
        },
        "ROUTER",
        &crate::workspace::secrets::SecretScope::Workspace {
            id: "ws".into(),
            root: root.to_owned(),
        },
        root,
    )
    .unwrap();
    let crate::workspace::secrets::ReadResult::PermissionRequired { pending_id, .. } = result
    else {
        panic!("expected a permission request");
    };
    (monitor, pending_id)
}

// ── R3: shortcut eligibility is settled at the chord's ordinal ──────────────

/// A Normal pane chord pressed while the palette owns input is settled
/// (suppressed) when pressed; the Escape that follows in the same delivery
/// must not let it run against the pane focus it restores.
#[test]
fn palette_delivery_settles_pane_chords_before_escape_restores_focus() {
    let mut h = HostHarness::new();
    let (a, _b) = two_probes(&mut h);
    let panes = h.pane_count();
    let sidebar = h.app.sidebar_visible;

    open_palette(&mut h);
    deliver(
        &mut h,
        vec![
            down(Key::T, native_cmd()),
            down(Key::Escape, Modifiers::NONE),
        ],
    );
    assert!(!h.app.show_command_palette, "Escape closed the palette");
    assert_eq!(
        h.pane_count(),
        panes,
        "Cmd+T was the palette's, not the pane's"
    );
    deliver(
        &mut h,
        vec![
            up(Key::T, Modifiers::NONE),
            up(Key::Escape, Modifiers::NONE),
        ],
    );

    open_palette(&mut h);
    deliver(
        &mut h,
        vec![
            down(Key::B, native_cmd()),
            down(Key::Escape, Modifiers::NONE),
        ],
    );
    assert!(!h.app.show_command_palette);
    assert_eq!(h.app.sidebar_visible, sidebar, "Cmd+B was the palette's");
    assert_eq!(focused(&h), Some(a));
    deliver(
        &mut h,
        vec![
            up(Key::B, Modifiers::NONE),
            up(Key::Escape, Modifiers::NONE),
        ],
    );
    assert!(probe_log(&mut h, a)
        .iter()
        .all(|e| !e.ends_with(":T") && !e.ends_with(":B")));

    // Positive control: the same chord, owned by the pane, runs.
    deliver(
        &mut h,
        vec![down(Key::B, native_cmd()), up(Key::B, native_cmd())],
    );
    assert_eq!(
        h.app.sidebar_visible, !sidebar,
        "a pane-owned Cmd+B toggles the sidebar"
    );
}

// ── R3/R4: exact matching and explicit transfer ─────────────────────────────

/// Native-shaped Cmd+J moves the palette selection before Enter acts.
#[test]
fn native_cmd_j_moves_the_palette_row_before_enter() {
    let mut h = HostHarness::new();
    let (_a, _b) = two_probes(&mut h);
    open_palette(&mut h);
    h.run_frames(1);
    let before = h.app.palette_selected;
    deliver(
        &mut h,
        vec![down(Key::J, native_cmd()), up(Key::J, native_cmd())],
    );
    assert_eq!(
        h.app.palette_selected,
        before + 1,
        "native Cmd+J (command + mac_cmd) must match the palette's next-row chord"
    );
    deliver(
        &mut h,
        vec![down(Key::K, native_cmd()), up(Key::K, native_cmd())],
    );
    assert_eq!(h.app.palette_selected, before);
}

/// `Cmd+Option+P, Enter` with an actionable approval: Focus Approval is an
/// explicit transfer, so Enter reaches the (unarmed) approval, runs no
/// palette row, and leaves the request pending.
#[test]
fn focus_approval_transfer_takes_the_next_event_from_the_palette() {
    let mut h = HostHarness::new();
    let root = tempfile::tempdir().unwrap();
    let (a, _b) = two_probes(&mut h);
    let (monitor, id) = pending_request(root.path());
    open_palette(&mut h);
    h.run_frames(1);

    deliver(
        &mut h,
        vec![
            down(Key::P, native_cmd_alt()),
            up(Key::P, native_cmd_alt()),
            down(Key::Enter, Modifiers::NONE),
        ],
    );
    assert_eq!(
        h.app.focus_stack.last(),
        Some(&FocusKind::ApprovalBanner),
        "Cmd+Option+P transferred input to the approval"
    );
    assert!(h.app.show_command_palette, "no palette row ran");
    assert_eq!(focused(&h), Some(a));
    assert!(
        monitor.show_pending(&id).is_some(),
        "the unarmed request stays pending"
    );
}

/// The same chord with no actionable approval leaves the palette eligible:
/// Enter then runs the selected row.
#[test]
fn focus_approval_without_a_request_leaves_the_palette_owner() {
    let mut h = HostHarness::new();
    let (_a, _b) = two_probes(&mut h);
    open_palette(&mut h);
    h.run_frames(1);
    deliver(
        &mut h,
        vec![
            down(Key::P, native_cmd_alt()),
            up(Key::P, native_cmd_alt()),
            down(Key::Enter, Modifiers::NONE),
        ],
    );
    assert_ne!(h.app.focus_stack.last(), Some(&FocusKind::ApprovalBanner));
    assert!(
        !h.app.show_command_palette,
        "the palette kept Enter and ran its row"
    );
}

// ── R3/R4: immediate and deferred click-through ─────────────────────────────

/// Before the palette's first draw, `Escape, move, press, release` over the
/// other pane: the barrier consumes the pointer, so focus and the pane stay
/// put. A fresh click on the exposed pane then focuses it.
#[test]
fn escape_then_click_before_first_palette_draw_never_reaches_the_pane() {
    let mut h = HostHarness::new();
    let (a, b) = two_probes(&mut h);
    let target = center(&h, b);
    let mut events = vec![
        down(Key::P, native_cmd()),
        down(Key::Escape, Modifiers::NONE),
    ];
    events.extend(click(target));
    deliver(&mut h, events);
    assert!(!h.app.show_command_palette);
    assert_eq!(
        focused(&h),
        Some(a),
        "the click belonged to the dismissed palette"
    );
    assert!(
        !h.egui_ctx()
            .input(|i| i.pointer.any_pressed() || i.pointer.any_released()),
        "egui never folded the barrier's pointer edges"
    );
    assert!(
        probe_log(&mut h, b).is_empty(),
        "the pane's widgets saw no click"
    );

    deliver(
        &mut h,
        vec![
            up(Key::P, Modifiers::NONE),
            up(Key::Escape, Modifiers::NONE),
        ],
    );
    deliver(&mut h, click(target));
    assert_eq!(
        focused(&h),
        Some(b),
        "a fresh click on the exposed pane focuses it"
    );
    assert_eq!(probe_log(&mut h, b), ["click"], "and reaches its widgets");
}

/// The same history against a palette that has been presented.
#[test]
fn escape_then_click_after_palette_draw_never_reaches_the_pane() {
    let mut h = HostHarness::new();
    let (a, b) = two_probes(&mut h);
    let target = center(&h, b);
    open_palette(&mut h);
    h.run_frames(2);
    let mut events = vec![down(Key::Escape, Modifiers::NONE)];
    events.extend(click(target));
    deliver(&mut h, events);
    assert!(!h.app.show_command_palette);
    assert_eq!(focused(&h), Some(a));
    deliver(&mut h, vec![up(Key::Escape, Modifiers::NONE)]);
    deliver(&mut h, click(target));
    assert_eq!(focused(&h), Some(b));
}

/// A press held on a pane when the palette opens: the gesture is cancelled
/// at the transfer, so its continuation cannot act behind the modal, and its
/// physical release is retired without reaching anyone.
#[test]
fn a_pointer_gesture_started_before_opening_cannot_continue_behind_the_palette() {
    let mut h = HostHarness::new();
    let (a, b) = two_probes(&mut h);
    let start = center(&h, a);
    deliver(&mut h, vec![moved(start), button(start, true)]);
    assert!(h.egui_ctx().input(|i| i.pointer.primary_down()));

    deliver(
        &mut h,
        vec![down(Key::P, native_cmd()), up(Key::P, native_cmd())],
    );
    assert!(h.app.show_command_palette);
    assert!(
        !h.egui_ctx().input(|i| i.pointer.primary_down()),
        "opening the palette cancelled the pane's held press"
    );

    let over_b = center(&h, b);
    deliver(&mut h, vec![moved(over_b), button(over_b, false)]);
    assert_eq!(focused(&h), Some(a));
    assert!(
        h.app.show_command_palette,
        "the retired release did not click the palette away"
    );

    deliver(
        &mut h,
        vec![Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -40.0),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        }],
    );
    assert!(h.app.show_command_palette);
}

/// Before its first draw the palette cannot hit-test, so it declines the
/// pointer instead of letting it fall through to the pane underneath.
#[test]
fn an_unpresented_palette_declines_the_pointer() {
    let mut h = HostHarness::new();
    let (a, b) = two_probes(&mut h);
    let target = center(&h, b);
    let mut events = vec![down(Key::P, native_cmd())];
    events.extend(click(target));
    deliver(&mut h, events);
    assert!(h.app.show_command_palette);
    assert_eq!(focused(&h), Some(a));
    assert!(
        probe_log(&mut h, b).is_empty(),
        "the pane's own widgets never saw the click"
    );
}

// ── R4: release edges follow their press, not current focus ─────────────────

/// `Cmd+P down, P up` closes the palette in one delivery. The P release is
/// retired with its press; the next ordinary `P down, Text, P up` reaches the
/// pane exactly once.
#[test]
fn a_closing_chord_retires_its_release_and_the_next_key_reaches_the_pane_whole() {
    let mut h = HostHarness::new();
    let (a, _b) = two_probes(&mut h);
    open_palette(&mut h);
    deliver(
        &mut h,
        vec![down(Key::P, native_cmd()), up(Key::P, Modifiers::NONE)],
    );
    assert!(!h.app.show_command_palette);
    deliver(
        &mut h,
        vec![
            down(Key::P, Modifiers::NONE),
            text("p"),
            up(Key::P, Modifiers::NONE),
        ],
    );
    assert_eq!(probe_log(&mut h, a), ["down:P", "text:p", "up:P"]);
}

/// The same across deliveries, Command released before P.
#[test]
fn a_closing_chord_release_in_a_later_delivery_is_still_retired() {
    let mut h = HostHarness::new();
    let (a, _b) = two_probes(&mut h);
    open_palette(&mut h);
    deliver(&mut h, vec![down(Key::P, native_cmd())]);
    deliver(&mut h, vec![up(Key::P, Modifiers::NONE)]);
    assert!(!h.app.show_command_palette);
    assert!(
        probe_log(&mut h, a).is_empty(),
        "the chord's release is not the pane's"
    );
    deliver(
        &mut h,
        vec![
            down(Key::P, Modifiers::NONE),
            text("p"),
            up(Key::P, Modifiers::NONE),
        ],
    );
    assert_eq!(probe_log(&mut h, a), ["down:P", "text:p", "up:P"]);
}

/// The same with the window blurred before the release arrives.
#[test]
fn blur_before_release_leaves_no_stale_claim() {
    let mut h = HostHarness::new();
    let (a, _b) = two_probes(&mut h);
    open_palette(&mut h);
    deliver(&mut h, vec![down(Key::P, native_cmd())]);
    deliver(&mut h, vec![Event::WindowFocused(false)]);
    deliver(&mut h, vec![Event::WindowFocused(true)]);
    deliver(
        &mut h,
        vec![
            down(Key::P, Modifiers::NONE),
            text("p"),
            up(Key::P, Modifiers::NONE),
        ],
    );
    assert_eq!(probe_log(&mut h, a), ["down:P", "text:p", "up:P"]);
}

// ── R3/R4: opening carry and event order ────────────────────────────────────

/// A pane-owned prefix goes to the pane; everything after Cmd+P belongs to
/// the palette, with zero suffix bytes in the pane.
#[test]
fn opening_mid_delivery_splits_the_delivery_at_the_chord() {
    let mut h = HostHarness::new();
    let (a, _b) = two_probes(&mut h);
    deliver(
        &mut h,
        vec![
            down(Key::X, Modifiers::NONE),
            text("x"),
            up(Key::X, Modifiers::NONE),
            down(Key::P, native_cmd()),
            down(Key::Y, Modifiers::NONE),
            text("y"),
            up(Key::Y, Modifiers::NONE),
        ],
    );
    assert!(h.app.show_command_palette);
    assert_eq!(probe_log(&mut h, a), ["down:X", "text:x", "up:X"]);

    // Cmd+P first: nothing reaches the pane.
    deliver(
        &mut h,
        vec![
            up(Key::P, Modifiers::NONE),
            down(Key::Escape, Modifiers::NONE),
        ],
    );
    deliver(&mut h, vec![up(Key::Escape, Modifiers::NONE)]);
    probe_log(&mut h, a);
    deliver(
        &mut h,
        vec![
            down(Key::P, native_cmd()),
            down(Key::Y, Modifiers::NONE),
            text("y"),
        ],
    );
    assert!(h.app.show_command_palette);
    assert!(probe_log(&mut h, a).is_empty());
}

/// Enter acts at its ordinal: `Down, Enter` runs the second row, while
/// `Enter, Down` runs the first and the barrier consumes the Down.
#[test]
fn enter_acts_on_the_row_selected_at_its_ordinal() {
    let run = |events: Vec<Event>| {
        let mut h = HostHarness::new();
        let (a, b) = two_probes(&mut h);
        open_palette(&mut h);
        h.run_frames(1);
        deliver(&mut h, events);
        assert!(
            !h.app.show_command_palette,
            "Enter ran a row and closed the palette"
        );
        let f = focused(&h);
        (f == Some(a), f == Some(b), h.app.palette_selected)
    };
    let reference_first = run(vec![down(Key::Enter, Modifiers::NONE)]);
    let reference_second = run(vec![
        down(Key::ArrowDown, Modifiers::NONE),
        down(Key::Enter, Modifiers::NONE),
    ]);
    let enter_then_down = run(vec![
        down(Key::Enter, Modifiers::NONE),
        down(Key::ArrowDown, Modifiers::NONE),
    ]);
    assert_eq!(
        (enter_then_down.0, enter_then_down.1),
        (reference_first.0, reference_first.1),
        "Enter, Down must act on the first row: the Down came after the dismissal"
    );
    assert_ne!(
        (reference_first.0, reference_first.1),
        (reference_second.0, reference_second.1),
        "the two rows target different panes, so the histories are distinguishable"
    );
}

// ── Cross-overlay and lifecycle ─────────────────────────────────────────────

/// Quick Note preempting the palette is an explicit transfer: the palette
/// closes, Quick Note owns the rest of the delivery, and the pane sees none.
#[test]
fn quick_note_preempt_transfers_the_rest_of_the_delivery() {
    let mut h = HostHarness::new();
    let (a, _b) = two_probes(&mut h);
    open_palette(&mut h);
    let (m, k) = h.app.key_bindings.open_quick_note;
    let m = if m.command {
        Modifiers {
            mac_cmd: cfg!(target_os = "macos"),
            ..m
        }
    } else {
        m
    };
    deliver(
        &mut h,
        vec![down(k, m), down(Key::Z, Modifiers::NONE), text("z")],
    );
    assert!(!h.app.show_command_palette);
    assert_eq!(h.app.focus_stack.last(), Some(&FocusKind::QuickNote));
    assert!(probe_log(&mut h, a).is_empty());
}

/// An approval decision key held after the decision: winit reports its
/// repeats as `repeat: false`, so the router's own down set must keep them
/// (and their text) from the pane until the physical release.
#[test]
fn a_held_approval_decision_key_never_reaches_the_pane() {
    let mut h = HostHarness::new();
    let root = tempfile::tempdir().unwrap();
    let (a, _b) = two_probes(&mut h);
    let (monitor, id) = pending_request(root.path());
    deliver(
        &mut h,
        vec![down(Key::P, native_cmd_alt()), up(Key::P, native_cmd_alt())],
    );
    assert_eq!(h.app.focus_stack.last(), Some(&FocusKind::ApprovalBanner));
    h.app
        .approval_keyboard
        .backdate_focus(crate::app::approval_keyboard::ARM_DELAY);

    deliver(&mut h, vec![down(Key::A, Modifiers::NONE), text("a")]);
    assert!(
        monitor.show_pending(&id).is_none(),
        "the armed approval decided"
    );
    assert_ne!(h.app.focus_stack.last(), Some(&FocusKind::ApprovalBanner));
    deliver(&mut h, vec![down(Key::A, Modifiers::NONE), text("a")]);
    deliver(&mut h, vec![down(Key::A, Modifiers::NONE), text("a")]);
    deliver(&mut h, vec![up(Key::A, Modifiers::NONE)]);
    assert!(
        probe_log(&mut h, a).is_empty(),
        "the held decision key stayed the approval's"
    );

    deliver(
        &mut h,
        vec![
            down(Key::A, Modifiers::NONE),
            text("a"),
            up(Key::A, Modifiers::NONE),
        ],
    );
    assert_eq!(probe_log(&mut h, a), ["down:A", "text:a", "up:A"]);
}

/// Input that arrives while the window is hidden is not replayed when it is
/// drawn again, and the hidden pass still services external work in logic.
#[test]
fn hidden_delivery_is_not_replayed_and_logic_still_services_ipc() {
    let mut h = HostHarness::new();
    let (a, b) = two_probes(&mut h);
    h.inject_ipc(crate::protocol::HostRequest::FocusPane { pane_id: b });
    h.hidden_frame_with(RawInput {
        screen_rect: screen(),
        events: vec![
            down(Key::H, Modifiers::NONE),
            text("h"),
            up(Key::H, Modifiers::NONE),
        ],
        ..Default::default()
    });
    assert_eq!(
        focused(&h),
        Some(b),
        "the hidden pass serviced pane IPC in logic"
    );
    h.run_frames(2);
    assert!(probe_log(&mut h, a).is_empty());
    assert!(
        probe_log(&mut h, b).is_empty(),
        "hidden input is not replayed on the next draw"
    );
}

// ── Production oracle over generated deliveries ─────────────────────────────
//
// `Model` is an independent reading of the ownership contract: its own owner
// and instance counter, its own held-edge table, its own presentation flag.
// It never calls the production matcher, router, palette or hit tester.
// `run_production` drives the same history through `HostHarness::frame`.
// Both are compared after every delivery, so a later focus change cannot
// hide an earlier misroute.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    A,
    B,
}

/// One generated input step. Key steps are single edges so a delivery cut
/// can fall between a press and its release; `Click` is a whole click, and
/// `Press`, `MoveFar`, `Release` are a gesture's parts (drag-return).
#[derive(Clone, Debug, PartialEq, Eq)]
enum Step {
    Down(Key),
    Text(char),
    Up(Key),
    CmdPDown,
    Click(Side),
    Paste(String),
    ImeCommit(String),
    Press(Side),
    /// Beyond click slop, within the pressed pane.
    MoveFar(Side),
    Release(Side),
}

impl Step {
    /// Decisive in the router's sense: it can decide something, so it waits
    /// for the next pass when an egui run already holds this one.
    fn decisive(&self) -> bool {
        !matches!(self, Step::Text(_) | Step::Up(_) | Step::MoveFar(_))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Observed {
    palette_open: bool,
    focus: Side,
    a: Vec<String>,
    b: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ModelOwner {
    Palette,
    Pane(Side),
}

#[derive(Clone, Copy)]
struct ModelHeld {
    instance: u64,
    /// The release goes back to the press's owner while it still holds input.
    returns: bool,
}

/// Where a pointer press landed, in the model.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PressTarget {
    Pane(Side),
    Scrim,
    Retired,
}

#[derive(Clone, Copy)]
struct ModelPress {
    target: PressTarget,
    /// Where it was pressed: a release elsewhere is beyond click slop.
    at: Side,
    instance: u64,
    moved_far: bool,
    /// egui hit-tested the press against a scrim painted the pass before.
    swallowed: bool,
}

struct Model {
    palette_open: bool,
    presented: bool,
    /// The palette painted in the last pass of the previous delivery: egui
    /// hit-tests one more pass against that pass's layers.
    drawn_last_pass: bool,
    focus: Side,
    instance: u64,
    last_owner: ModelOwner,
    held: std::collections::HashMap<Key, ModelHeld>,
    press: Option<ModelPress>,
    a: Vec<String>,
    b: Vec<String>,
}

/// Per-delivery model state: the pass structure the run-splitting rule
/// imposes.
#[derive(Default)]
struct Pass {
    /// An egui run holds this pass: the next decisive step waits for a new
    /// pass, after this one paints.
    egui_run: bool,
    /// Still the delivery's first pass (egui may see a stale scrim).
    first: bool,
    /// A widget click egui reports when this pass paints.
    click: Option<Side>,
    /// The palette closes when this pass paints (scrim click-away).
    closing: bool,
    /// The dismissal barrier, or the retirement of held input whose owner
    /// left while painting: the rest of the delivery acts on nothing.
    barrier: bool,
    text_owned: bool,
}

impl Model {
    fn new() -> Self {
        Self {
            palette_open: false,
            presented: false,
            drawn_last_pass: false,
            focus: Side::A,
            instance: 0,
            last_owner: ModelOwner::Pane(Side::A),
            held: Default::default(),
            press: None,
            a: Vec::new(),
            b: Vec::new(),
        }
    }

    fn owner(&mut self) -> (ModelOwner, u64) {
        let owner = if self.palette_open {
            ModelOwner::Palette
        } else {
            ModelOwner::Pane(self.focus)
        };
        if owner != self.last_owner {
            self.instance += 1;
            self.last_owner = owner;
        }
        (owner, self.instance)
    }

    fn log(&mut self, side: Side, entry: String) {
        match side {
            Side::A => self.a.push(entry),
            Side::B => self.b.push(entry),
        }
    }

    /// The current pass paints; the next step starts a new pass.
    fn paint(&mut self, pass: &mut Pass) {
        if let Some(side) = pass.click.take() {
            self.log(side, "click".to_string());
        }
        if pass.closing {
            pass.closing = false;
            self.palette_open = false;
            // Held input was held for the palette, which is gone: retired.
            pass.barrier = true;
        }
        if self.palette_open {
            self.presented = true;
        }
        pass.egui_run = false;
        pass.first = false;
    }

    /// A press of a key still held is a repeat. Its press's owner gone (or
    /// the key retired): the repeat, and its text, go with it.
    fn repeat_retired(&mut self, key: Key, instance: u64, pass: &mut Pass) -> bool {
        let Some(held) = self.held.get(&key).copied() else {
            return false;
        };
        if held.instance != instance || !held.returns || pass.barrier {
            self.held.insert(
                key,
                ModelHeld {
                    instance: u64::MAX,
                    returns: false,
                },
            );
            pass.text_owned = true;
            return true;
        }
        false
    }

    fn press(&mut self, pass: &mut Pass, side: Side, owner: ModelOwner, instance: u64) {
        let target = if pass.barrier {
            PressTarget::Retired
        } else {
            match owner {
                ModelOwner::Palette if self.presented => PressTarget::Scrim,
                // Before its first draw the modal declines the press.
                ModelOwner::Palette => PressTarget::Retired,
                ModelOwner::Pane(_) => {
                    self.focus = side;
                    PressTarget::Pane(side)
                }
            }
        };
        if target != PressTarget::Retired {
            pass.egui_run = true;
        }
        let (_, instance_after) = self.owner();
        self.press = Some(ModelPress {
            target,
            at: side,
            instance: if matches!(target, PressTarget::Pane(_)) {
                instance_after
            } else {
                instance
            },
            moved_far: false,
            swallowed: pass.first && self.drawn_last_pass,
        });
    }

    fn release(&mut self, pass: &mut Pass, side: Side, instance: u64) {
        let Some(press) = self.press.take() else {
            return;
        };
        if pass.barrier || press.instance != instance {
            return;
        }
        match press.target {
            PressTarget::Pane(pressed)
                if pressed == side && !press.moved_far && !press.swallowed =>
            {
                pass.egui_run = true;
                pass.click = Some(side);
            }
            PressTarget::Pane(_) => pass.egui_run = true,
            PressTarget::Scrim if !press.moved_far && press.at == side => {
                pass.egui_run = true;
                pass.closing = true;
            }
            PressTarget::Scrim => pass.egui_run = true,
            PressTarget::Retired => {}
        }
    }

    fn delivery(&mut self, steps: &[Step]) -> Observed {
        let mut pass = Pass {
            first: true,
            ..Pass::default()
        };
        for step in steps {
            if step.decisive() && pass.egui_run {
                self.paint(&mut pass);
            }
            let (owner, instance) = self.owner();
            let field_input = matches!(
                step,
                Step::Down(_)
                    | Step::Up(_)
                    | Step::Text(_)
                    | Step::CmdPDown
                    | Step::Paste(_)
                    | Step::ImeCommit(_)
            );
            if owner == ModelOwner::Palette && !self.presented && field_input && !pass.barrier {
                // Held until the palette has painted and its field has focus.
                self.paint(&mut pass);
            }
            let (owner, instance) = if self.owner() != (owner, instance) {
                self.owner()
            } else {
                (owner, instance)
            };
            match step.clone() {
                Step::Down(key) => {
                    if self.repeat_retired(key, instance, &mut pass) {
                        continue;
                    }
                    if pass.barrier {
                        self.held.insert(
                            key,
                            ModelHeld {
                                instance,
                                returns: false,
                            },
                        );
                        pass.text_owned = true;
                        continue;
                    }
                    match owner {
                        ModelOwner::Palette if key == Key::Escape => {
                            self.palette_open = false;
                            pass.barrier = true;
                            self.held.insert(
                                key,
                                ModelHeld {
                                    instance,
                                    returns: true,
                                },
                            );
                            pass.text_owned = true;
                        }
                        ModelOwner::Palette => {
                            // The query field's: an egui edit.
                            self.held.insert(
                                key,
                                ModelHeld {
                                    instance,
                                    returns: true,
                                },
                            );
                            pass.text_owned = false;
                            pass.egui_run = true;
                        }
                        ModelOwner::Pane(side) => {
                            self.log(side, format!("down:{key:?}"));
                            self.held.insert(
                                key,
                                ModelHeld {
                                    instance,
                                    returns: true,
                                },
                            );
                            pass.text_owned = false;
                        }
                    }
                }
                Step::CmdPDown => {
                    if self.repeat_retired(Key::P, instance, &mut pass) {
                        continue;
                    }
                    self.held.insert(
                        Key::P,
                        ModelHeld {
                            instance,
                            returns: false,
                        },
                    );
                    pass.text_owned = true;
                    if pass.barrier {
                        continue;
                    }
                    if self.palette_open {
                        self.palette_open = false;
                        pass.barrier = true;
                    } else {
                        self.palette_open = true;
                        self.presented = false;
                    }
                }
                Step::Up(key) => {
                    let held = self.held.remove(&key);
                    let delivered = match held {
                        Some(held) => !pass.barrier && held.returns && held.instance == instance,
                        None => !pass.barrier,
                    };
                    if delivered {
                        if let ModelOwner::Pane(side) = owner {
                            self.log(side, format!("up:{key:?}"));
                        }
                    }
                }
                Step::Text(c) => {
                    if pass.barrier || pass.text_owned {
                        continue;
                    }
                    match owner {
                        ModelOwner::Pane(side) => self.log(side, format!("text:{c}")),
                        ModelOwner::Palette => pass.egui_run = true,
                    }
                }
                Step::Paste(text) => {
                    if pass.barrier {
                        continue;
                    }
                    match owner {
                        ModelOwner::Pane(side) => self.log(side, format!("paste:{text}")),
                        ModelOwner::Palette => pass.egui_run = true,
                    }
                }
                Step::ImeCommit(text) => {
                    if pass.barrier {
                        continue;
                    }
                    match owner {
                        ModelOwner::Pane(side) => self.log(side, format!("ime:{text}")),
                        ModelOwner::Palette => pass.egui_run = true,
                    }
                }
                Step::Click(side) => {
                    self.press(&mut pass, side, owner, instance);
                    // The release is decisive: it waits for the next pass.
                    if pass.egui_run {
                        self.paint(&mut pass);
                    }
                    let (_, instance) = self.owner();
                    self.release(&mut pass, side, instance);
                }
                Step::Press(side) => self.press(&mut pass, side, owner, instance),
                Step::MoveFar(_) => {
                    if let Some(press) = self.press.as_mut() {
                        press.moved_far = true;
                    }
                }
                Step::Release(side) => self.release(&mut pass, side, instance),
            }
        }
        self.drawn_last_pass = self.palette_open;
        self.paint(&mut pass);
        Observed {
            palette_open: self.palette_open,
            focus: self.focus,
            a: std::mem::take(&mut self.a),
            b: std::mem::take(&mut self.b),
        }
    }
}

fn step_events(h: &HostHarness, step: Step, panes: (PaneId, PaneId)) -> Vec<Event> {
    let at = |side: Side| {
        center(
            h,
            match side {
                Side::A => panes.0,
                Side::B => panes.1,
            },
        )
    };
    match step {
        Step::Down(key) => vec![down(key, Modifiers::NONE)],
        Step::Up(key) => vec![up(key, Modifiers::NONE)],
        Step::Text(c) => vec![text(&c.to_string())],
        Step::CmdPDown => vec![down(Key::P, native_cmd())],
        Step::Click(side) => click(at(side)),
        Step::Paste(text) => vec![Event::Paste(text)],
        Step::ImeCommit(text) => vec![Event::Ime(egui::ImeEvent::Commit(text))],
        Step::Press(side) => vec![moved(at(side)), button(at(side), true)],
        // Beyond click slop, within the same large pane.
        Step::MoveFar(side) => vec![moved(at(side) + egui::vec2(0.0, 120.0))],
        Step::Release(side) => vec![moved(at(side)), button(at(side), false)],
    }
}

fn run_production(deliveries: &[Vec<Step>]) -> Vec<Observed> {
    let mut h = HostHarness::new();
    let (a, b) = two_probes(&mut h);
    deliveries
        .iter()
        .map(|steps| {
            let events = steps
                .iter()
                .flat_map(|step| step_events(&h, step.clone(), (a, b)))
                .collect();
            deliver(&mut h, events);
            Observed {
                palette_open: h.app.show_command_palette,
                focus: if focused(&h) == Some(b) {
                    Side::B
                } else {
                    Side::A
                },
                a: probe_log(&mut h, a),
                b: probe_log(&mut h, b),
            }
        })
        .collect()
}

fn run_model(deliveries: &[Vec<Step>]) -> Vec<Observed> {
    let mut model = Model::new();
    deliveries
        .iter()
        .map(|steps| model.delivery(steps))
        .collect()
}

/// First delivery where production and model disagree.
fn divergence(deliveries: &[Vec<Step>]) -> Option<(usize, Observed, Observed)> {
    let production = run_production(deliveries);
    let model = run_model(deliveries);
    production
        .into_iter()
        .zip(model)
        .enumerate()
        .find(|(_, (p, m))| p != m)
        .map(|(i, (p, m))| (i, p, m))
}

/// Deterministic xorshift, so every generated history is reproducible.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// A history: steps in order, then cut into deliveries at random points that
/// are independent of where the palette opens, closes or is presented. A key
/// is never pressed again before its release.
fn generate(seed: u64) -> Vec<Vec<Step>> {
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let side = |rng: &mut Rng| if rng.below(2) == 0 { Side::A } else { Side::B };
    let mut steps = Vec::new();
    for _ in 0..(6 + rng.below(8)) {
        match rng.below(9) {
            0 | 1 => {
                let (key, c) = [(Key::X, 'x'), (Key::Y, 'y'), (Key::Z, 'z')][rng.below(3) as usize];
                steps.extend([Step::Down(key), Step::Text(c), Step::Up(key)]);
            }
            2 => steps.extend([Step::CmdPDown, Step::Up(Key::P)]),
            3 => steps.extend([Step::Down(Key::Escape), Step::Up(Key::Escape)]),
            4 => steps.push(Step::Paste("pasted".to_string())),
            5 => steps.push(Step::ImeCommit("ime".to_string())),
            6 => {
                // A gesture: press, maybe cross the slop, release on either pane.
                let pressed = side(&mut rng);
                steps.push(Step::Press(pressed));
                if rng.below(2) == 0 {
                    steps.push(Step::MoveFar(pressed));
                }
                let released = if rng.below(3) == 0 {
                    side(&mut rng)
                } else {
                    pressed
                };
                steps.push(Step::Release(released));
            }
            _ => steps.push(Step::Click(side(&mut rng))),
        }
    }
    let mut deliveries = vec![Vec::new()];
    for step in steps {
        if !deliveries.last().is_some_and(Vec::is_empty) && rng.below(3) == 0 {
            deliveries.push(Vec::new());
        }
        deliveries.last_mut().expect("a delivery").push(step);
    }
    deliveries
}

/// Remove one step at a time while the divergence persists.
fn shrink(mut deliveries: Vec<Vec<Step>>) -> Vec<Vec<Step>> {
    'outer: loop {
        for d in 0..deliveries.len() {
            for s in 0..deliveries[d].len() {
                let mut candidate = deliveries.clone();
                candidate[d].remove(s);
                candidate.retain(|delivery| !delivery.is_empty());
                if divergence(&candidate).is_some() {
                    deliveries = candidate;
                    continue 'outer;
                }
            }
        }
        return deliveries;
    }
}

fn assert_production_matches_model(deliveries: Vec<Vec<Step>>, label: &str) {
    if divergence(&deliveries).is_some() {
        let shrunk = shrink(deliveries);
        let (at, production, model) = divergence(&shrunk).expect("shrinking keeps a divergence");
        panic!(
            "{label}: production diverged from the ownership model at delivery {at}\n\
             history: {shrunk:?}\nproduction: {production:?}\nmodel: {model:?}"
        );
    }
}

/// The mandatory histories from the stint's findings table, seeded directly.
#[test]
fn production_oracle_mandatory_histories() {
    use Step::*;
    let histories: Vec<(&str, Vec<Vec<Step>>)> = vec![
        (
            "escape then click before first draw",
            vec![vec![CmdPDown, Down(Key::Escape), Click(Side::B)]],
        ),
        (
            "escape then click after draw",
            vec![
                vec![CmdPDown, Up(Key::P)],
                vec![Down(Key::Escape), Click(Side::B), Up(Key::Escape)],
            ],
        ),
        (
            "closing chord release retired",
            vec![
                vec![CmdPDown, Up(Key::P)],
                vec![CmdPDown, Up(Key::P)],
                vec![Down(Key::P), Text('p'), Up(Key::P)],
            ],
        ),
        (
            "release in a later delivery",
            vec![
                vec![CmdPDown],
                vec![Up(Key::P)],
                vec![CmdPDown],
                vec![Up(Key::P), Down(Key::P), Text('p'), Up(Key::P)],
            ],
        ),
        (
            "opening mid delivery",
            vec![vec![
                Down(Key::X),
                Text('x'),
                CmdPDown,
                Down(Key::Y),
                Text('y'),
                Up(Key::Y),
                Up(Key::X),
            ]],
        ),
        (
            "escape then typing",
            vec![
                vec![CmdPDown, Up(Key::P)],
                vec![
                    Down(Key::Escape),
                    Down(Key::Z),
                    Text('z'),
                    Up(Key::Z),
                    Up(Key::Escape),
                ],
                vec![Down(Key::Z), Text('z'), Up(Key::Z)],
            ],
        ),
        (
            "click away then type",
            vec![
                vec![CmdPDown, Up(Key::P)],
                vec![Click(Side::B), Down(Key::X), Text('x'), Up(Key::X)],
                vec![Down(Key::X), Text('x'), Up(Key::X)],
            ],
        ),
        (
            "pane click then keys",
            vec![vec![Click(Side::B), Down(Key::Y), Text('y'), Up(Key::Y)]],
        ),
    ];
    for (label, history) in histories {
        assert_production_matches_model(history, label);
    }
}

/// Generated histories: openings, closes, releases and clicks at any
/// position, cut into deliveries independently of those transitions.
#[test]
fn production_oracle_generated_histories() {
    for seed in 1..=120 {
        assert_production_matches_model(generate(seed), &format!("seed {seed}"));
    }
}

/// Visual evidence: the palette over two panes, then the same panes after
/// `Escape, click` in one delivery (focus unchanged, no click-through), then
/// after a fresh click (focus moved).
#[test]
fn screenshot_palette_dismissal_and_fresh_click() {
    let mut h = HostHarness::new();
    let (a, b) = two_probes(&mut h);
    let target = center(&h, b);
    open_palette(&mut h);
    h.run_frames(2);
    h.render_png(crate::ui_tests::evidence_png!("_open"))
        .expect("render palette");
    let mut events = vec![down(Key::Escape, Modifiers::NONE)];
    events.extend(click(target));
    deliver(&mut h, events);
    deliver(&mut h, vec![up(Key::Escape, Modifiers::NONE)]);
    h.run_frames(2);
    assert_eq!(focused(&h), Some(a));
    h.render_png(crate::ui_tests::evidence_png!("_dismissed"))
        .expect("render dismissed");
    deliver(&mut h, click(target));
    h.run_frames(2);
    assert_eq!(focused(&h), Some(b));
    h.render_png(crate::ui_tests::evidence_png!("_clicked"))
        .expect("render clicked");
}

/// The kittest-backed `PlexiUiHarness` must hand every step's delivery to
/// `raw_input_hook`, as eframe does (vendored `egui_kittest`, see
/// `deps/egui_kittest-0.34.3/PLEXI-PATCH.md`). Without it, ui_tests input
/// bypasses the router: a key press here would never reach the palette.
#[test]
fn kittest_harness_steps_go_through_raw_input_hook() {
    let mut h = crate::ui_tests::PlexiUiHarness::new();
    h.step();
    let before = h.with_app(|app| app.input_router.deliveries());
    h.step();
    assert_eq!(
        h.with_app(|app| app.input_router.deliveries()),
        before + 1,
        "one harness step is one routed delivery"
    );
    h.harness().key_press_modifiers(native_cmd(), Key::P);
    h.run_steps(2);
    assert!(
        h.with_app(|app| app.show_command_palette),
        "a kittest key press reaches the router and opens the palette"
    );
}

// ── Round 2: ordered runs, held input, edges, drops, paint-time decisions ───

fn tap(h: &mut HostHarness, pane: PaneId) -> String {
    String::from_utf8_lossy(&h.terminal_backend(pane).take_input_tap()).into_owned()
}

fn two_terminals(h: &mut HostHarness) -> (PaneId, PaneId) {
    let t1 = h.add_focused_terminal();
    let t2 = h.add_focused_terminal();
    h.focus_pane(t1);
    h.run_frames(3);
    h.terminal_backend(t1).enable_input_tap();
    h.terminal_backend(t2).enable_input_tap();
    (t1, t2)
}

fn typed(c: char) -> Vec<Event> {
    let key = Key::from_name(&c.to_ascii_uppercase().to_string()).expect("letter key");
    vec![
        down(key, Modifiers::NONE),
        text(&c.to_string()),
        up(key, Modifiers::NONE),
    ]
}

/// `x`, click terminal B, `y` in one delivery: each terminal gets exactly
/// its own keys, in order, from its own stream.
#[test]
fn typing_around_a_click_reaches_each_terminal_in_order() {
    let mut h = HostHarness::new();
    let (t1, t2) = two_terminals(&mut h);
    let mut events = typed('x');
    events.extend(click(center(&h, t2)));
    events.extend(typed('y'));
    deliver(&mut h, events);
    assert_eq!(focused(&h), Some(t2));
    assert_eq!(tap(&mut h, t1), "x");
    assert_eq!(tap(&mut h, t2), "y");
}

/// A fast burst of terminal keys is one pass: terminals take their ordered
/// stream structurally, so the run-splitting rule never delays them.
#[test]
fn a_terminal_key_burst_stays_in_one_pass() {
    let mut h = HostHarness::new();
    let (t1, _t2) = two_terminals(&mut h);
    let burst: Vec<Event> = "abcdefghij".chars().flat_map(typed).collect();
    let before = h.app.input_router.deliveries();
    h.frame(RawInput {
        screen_rect: screen(),
        events: burst,
        ..Default::default()
    });
    assert!(
        !h.app.input_router.has_held_input(),
        "nothing waited for a later pass"
    );
    assert_eq!(h.app.input_router.deliveries(), before + 1);
    assert_eq!(tap(&mut h, t1), "abcdefghij");
}

fn name_pane(h: &mut HostHarness, pane: PaneId, name: &str) {
    h.app.windows[0]
        .panes
        .get_mut(&pane)
        .and_then(|p| p.as_view_mut())
        .expect("view pane")
        .name = name.to_string();
}

/// `Cmd+P, Paste, Enter` in one delivery: the paste edits the new palette's
/// query once its field exists, and Enter runs the row matching it, with
/// nothing reaching the pane.
#[test]
fn opening_carry_pastes_into_the_query_and_enter_runs_the_matching_row() {
    for commit in [
        Event::Paste("zebra".into()),
        Event::Ime(egui::ImeEvent::Commit("zebra".into())),
    ] {
        let mut h = HostHarness::new();
        let (a, b) = two_probes(&mut h);
        name_pane(&mut h, a, "alpha probe");
        name_pane(&mut h, b, "zebra probe");
        h.run_frames(1);
        deliver(
            &mut h,
            vec![
                down(Key::P, native_cmd()),
                commit.clone(),
                down(Key::Enter, Modifiers::NONE),
            ],
        );
        assert!(!h.app.show_command_palette, "Enter ran a row");
        assert_eq!(
            focused(&h),
            Some(b),
            "the row matching the pasted query ran"
        );
        assert!(
            probe_log(&mut h, a).is_empty(),
            "no suffix reached the pane"
        );
    }
}

/// An already-open palette: a paste then Enter in one delivery acts on the
/// query as of Enter's ordinal (after the paste).
#[test]
fn enter_acts_on_the_query_as_of_its_ordinal() {
    let mut h = HostHarness::new();
    let (a, b) = two_probes(&mut h);
    name_pane(&mut h, a, "alpha probe");
    name_pane(&mut h, b, "zebra probe");
    open_palette(&mut h);
    h.run_frames(2);
    deliver(
        &mut h,
        vec![
            Event::Paste("zebra".into()),
            down(Key::Enter, Modifiers::NONE),
        ],
    );
    assert_eq!(focused(&h), Some(b));
}

/// After a blur the outstanding release of a chord pressed before it reaches
/// nobody, and the next ordinary key reaches the pane whole.
#[test]
fn blur_retires_an_outstanding_release() {
    let mut h = HostHarness::new();
    let (a, _b) = two_probes(&mut h);
    open_palette(&mut h);
    deliver(&mut h, vec![down(Key::P, native_cmd())]);
    deliver(&mut h, vec![Event::WindowFocused(false)]);
    deliver(&mut h, vec![Event::WindowFocused(true)]);
    deliver(&mut h, vec![up(Key::P, Modifiers::NONE)]);
    assert!(
        probe_log(&mut h, a).is_empty(),
        "the late release reached nobody"
    );
    deliver(&mut h, typed('p'));
    assert_eq!(probe_log(&mut h, a), ["down:P", "text:p", "up:P"]);
}

/// A key pressed behind the barrier stays consumed through its repeats and
/// its release, even though the revealed pane now owns input.
#[test]
fn a_key_pressed_behind_the_barrier_stays_consumed_until_released() {
    let mut h = HostHarness::new();
    let (a, _b) = two_probes(&mut h);
    open_palette(&mut h);
    h.run_frames(1);
    deliver(
        &mut h,
        vec![
            down(Key::Escape, Modifiers::NONE),
            down(Key::B, Modifiers::NONE),
            text("b"),
        ],
    );
    assert!(!h.app.show_command_palette);
    deliver(&mut h, vec![down(Key::B, Modifiers::NONE), text("b")]);
    deliver(
        &mut h,
        vec![
            up(Key::B, Modifiers::NONE),
            up(Key::Escape, Modifiers::NONE),
        ],
    );
    assert!(
        probe_log(&mut h, a).is_empty(),
        "the barrier's key never reached the pane"
    );
    deliver(&mut h, typed('b'));
    assert_eq!(probe_log(&mut h, a), ["down:B", "text:b", "up:B"]);
}

/// Click B, then navigate back to A, in one delivery: paint never replays
/// the earlier press, so the later focus change stands.
#[test]
fn paint_never_replays_a_press_over_a_later_focus_change() {
    let mut h = HostHarness::new();
    let (a, b) = two_probes(&mut h);
    let (m, k) = h.app.key_bindings.navigate_left;
    let m = Modifiers {
        mac_cmd: m.command && cfg!(target_os = "macos"),
        ..m
    };
    let mut events = click(center(&h, b));
    events.extend([down(k, m), up(k, m)]);
    deliver(&mut h, events);
    assert_eq!(focused(&h), Some(a));
}

/// A modal owns file drops: a zoomed terminal under the palette never gets
/// the dropped path; without the palette it does.
#[test]
fn file_drops_follow_modal_ownership() {
    let mut h = HostHarness::new();
    let (t1, _t2) = two_terminals(&mut h);
    h.app.toggle_pane_zoom();
    h.run_frames(2);
    let drop = |h: &mut HostHarness| {
        h.frame(RawInput {
            screen_rect: screen(),
            dropped_files: vec![egui::DroppedFile {
                path: Some(std::path::PathBuf::from("/tmp/plexi-router-drop.txt")),
                ..Default::default()
            }],
            ..Default::default()
        });
        h.run_frames(1);
    };
    open_palette(&mut h);
    h.run_frames(1);
    tap(&mut h, t1);
    drop(&mut h);
    assert!(
        !tap(&mut h, t1).contains("plexi-router-drop"),
        "the palette owned the drop"
    );
    deliver(
        &mut h,
        vec![
            down(Key::Escape, Modifiers::NONE),
            up(Key::Escape, Modifiers::NONE),
        ],
    );
    h.run_frames(1);
    tap(&mut h, t1);
    drop(&mut h);
    assert!(
        tap(&mut h, t1).contains("plexi-router-drop"),
        "positive control"
    );
}

/// A pointer release that arrives while the window is hidden still reaches
/// egui, so no button stays down when drawing resumes.
#[test]
fn a_hidden_release_still_releases_the_button() {
    let mut h = HostHarness::new();
    let (a, _b) = two_probes(&mut h);
    let at = center(&h, a);
    deliver(&mut h, vec![moved(at), button(at, true)]);
    assert!(h.egui_ctx().input(|i| i.pointer.primary_down()));
    h.hidden_frame_with(RawInput {
        screen_rect: screen(),
        events: vec![button(at, false)],
        ..Default::default()
    });
    h.run_frames(1);
    assert!(!h.egui_ctx().input(|i| i.pointer.primary_down()));
}

fn egui_folded_wheel(h: &HostHarness) -> bool {
    h.egui_ctx().input(|i| {
        i.raw
            .events
            .iter()
            .any(|event| matches!(event, Event::MouseWheel { .. }))
    })
}

fn wheel(phase: egui::TouchPhase) -> Event {
    Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -40.0),
        phase,
        modifiers: Modifiers::NONE,
    }
}

/// A wheel gesture begun over the palette keeps its owner: once Escape has
/// closed the palette, its continuation never scrolls the pane revealed; a
/// fresh gesture does.
#[test]
fn a_wheel_gesture_keeps_its_owner() {
    let mut h = HostHarness::new();
    let (a, _b) = two_probes(&mut h);
    open_palette(&mut h);
    h.run_frames(2);
    let over_a = center(&h, a);
    deliver(&mut h, vec![moved(over_a), wheel(egui::TouchPhase::Start)]);
    deliver(
        &mut h,
        vec![
            down(Key::Escape, Modifiers::NONE),
            up(Key::Escape, Modifiers::NONE),
        ],
    );
    assert!(!h.app.show_command_palette);
    deliver(&mut h, vec![wheel(egui::TouchPhase::Move)]);
    assert!(
        !egui_folded_wheel(&h),
        "the palette's gesture never reached the pane"
    );
    deliver(&mut h, vec![wheel(egui::TouchPhase::End)]);
    deliver(&mut h, vec![wheel(egui::TouchPhase::Start)]);
    assert!(egui_folded_wheel(&h), "a fresh gesture scrolls");
}

/// Cmd+W under Confirm Close is that dialog's Escape: the dialog cancels,
/// and the original W never reaches egui.
#[test]
fn close_chord_under_confirm_close_cancels_it() {
    let mut h = HostHarness::new();
    let (_a, _b) = two_probes(&mut h);
    let panes = h.pane_count();
    h.app.pending_close = true;
    h.app.sync_confirm_close_focus();
    h.run_frames(3);
    let (m, k) = h.app.key_bindings.close_pane;
    let m = Modifiers {
        mac_cmd: m.command && cfg!(target_os = "macos"),
        ..m
    };
    deliver(&mut h, vec![down(k, m), up(k, m)]);
    assert!(!h.app.pending_close, "the close chord cancelled the dialog");
    assert_eq!(h.pane_count(), panes);
}

/// A dialog that decides while painting (Description's Cmd+Enter commit):
/// the click that followed it in the same delivery was held for the dialog
/// and is retired, not replayed against the pane it revealed.
#[test]
fn a_paint_time_decision_retires_the_rest_of_its_delivery() {
    let mut h = HostHarness::new();
    let (a, b) = two_probes(&mut h);
    h.app.open_description_edit(0);
    h.app.description_buffer = "described".into();
    h.run_frames(3);
    assert!(h.app.editing_description.is_some());
    let mut events = vec![down(Key::Enter, native_cmd()), up(Key::Enter, native_cmd())];
    events.extend(click(center(&h, b)));
    deliver(&mut h, events);
    assert!(h.app.editing_description.is_none(), "Cmd+Enter committed");
    assert_eq!(focused(&h), Some(a), "the click went with the dialog");
    assert!(probe_log(&mut h, b).is_empty());
    let at_b = center(&h, b);
    deliver(&mut h, click(at_b));
    assert_eq!(focused(&h), Some(b), "a fresh click works");
}
