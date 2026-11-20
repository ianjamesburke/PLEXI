//! Unit tests for the pure router (stint 1659). Each test states the
//! expected sink effects literally; none calls the production matcher,
//! dialog reducer or hit test to compute an expectation.

use super::dialog::{ActionSpec, DialogPresentation};
use super::*;
use crate::host::keys::{build_binding_table, Action, KeyBindings};
use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect};

const A: PaneInstance = PaneInstance {
    window_id: 1,
    pane: 10,
    endpoint: 100,
};
const B: PaneInstance = PaneInstance {
    window_id: 1,
    pane: 11,
    endpoint: 101,
};

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

fn confirm_actions() -> Vec<ActionSpec> {
    vec![
        ActionSpec {
            id: "close".into(),
            shortcut: Some((Modifiers::NONE, Key::Enter)),
            destructive: true,
            refuses: false,
        },
        ActionSpec {
            id: "cancel".into(),
            shortcut: Some((Modifiers::NONE, Key::Escape)),
            destructive: false,
            refuses: true,
        },
    ]
}

const CLOSE_RECT: Rect = Rect {
    min: Pos2::new(500.0, 400.0),
    max: Pos2::new(560.0, 430.0),
};
const CANCEL_RECT: Rect = Rect {
    min: Pos2::new(420.0, 400.0),
    max: Pos2::new(490.0, 430.0),
};
const PANEL: Rect = Rect {
    min: Pos2::new(400.0, 300.0),
    max: Pos2::new(600.0, 440.0),
};

/// A terminal presented over the left half (A) and right half (B).
struct World {
    table: Vec<crate::host::keys::BindingEntry>,
    dialog: Option<OpeningId>,
    presented: bool,
    keyboard: KeyboardOwner,
    os_focused: bool,
    legacy_overlay: bool,
}

impl World {
    fn new() -> Self {
        Self {
            table: build_binding_table(&KeyBindings::default()),
            dialog: None,
            presented: true,
            keyboard: KeyboardOwner::Terminal(A),
            os_focused: true,
            legacy_overlay: false,
        }
    }

    fn snapshot(&self) -> Snapshot<'_> {
        let layer = egui::LayerId::background();
        Snapshot {
            visible: true,
            os_focused: self.os_focused,
            dialog: self.dialog.map(|opening| DialogSnapshot {
                opening,
                actions: confirm_actions(),
                default_action: None,
            }),
            legacy_overlay: self.legacy_overlay,
            keyboard: self.keyboard,
            table: &self.table,
            gates: binding::BindingGates::default(),
            terminals: vec![
                PresentedTerminal {
                    target: A,
                    hit_rect: Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(640.0, 800.0)),
                    layer,
                },
                PresentedTerminal {
                    target: B,
                    hit_rect: Rect::from_min_max(Pos2::new(640.0, 0.0), Pos2::new(1280.0, 800.0)),
                    layer,
                },
            ],
            dialog_presentation: self.dialog.filter(|_| self.presented).map(|opening| {
                DialogPresentation {
                    opening,
                    panel: PANEL,
                    buttons: vec![
                        ("close".into(), CLOSE_RECT, true),
                        ("cancel".into(), CANCEL_RECT, true),
                    ],
                }
            }),
        }
    }
}

/// Stamp `events` as one delivery from `source`.
fn delivery(router: &mut HostRouter, source: Source, events: Vec<Event>) -> Vec<Received> {
    let id = router.next_delivery();
    let mut pointer = None;
    events
        .into_iter()
        .enumerate()
        .map(|(ordinal, event)| {
            if let Event::PointerMoved(pos) | Event::PointerButton { pos, .. } = &event {
                pointer = Some(*pos);
            }
            Received {
                id: EventId {
                    delivery: id,
                    ordinal: ordinal as u32,
                },
                source,
                received_at: std::time::Instant::now(),
                modifiers: Modifiers::NONE,
                pointer: matches!(event, Event::PointerMoved(_) | Event::PointerButton { .. })
                    .then_some(pointer)
                    .flatten(),
                top_layer: Some(egui::LayerId::background()),
                event,
            }
        })
        .collect()
}

/// A readable record of each command.
#[derive(Debug, PartialEq)]
enum Effect {
    Terminal(u64, Vec<String>),
    Retire(u64),
    Focus(u64),
    Host(Action),
    Decide(u64, String),
}

fn label(event: &Event) -> String {
    match event {
        Event::Text(text) => format!("text:{text}"),
        Event::Key { key, pressed, .. } => {
            format!("{}:{key:?}", if *pressed { "down" } else { "up" })
        }
        Event::PointerButton { pressed, .. } => {
            (if *pressed { "press" } else { "release" }).to_string()
        }
        Event::PointerMoved(_) => "move".to_string(),
        other => format!("{other:?}"),
    }
}

/// Drain commands, resolving suspending ones as logic would.
fn drain(router: &mut HostRouter) -> Vec<Effect> {
    router
        .take_commands()
        .into_iter()
        .map(|command| match command {
            RoutedCommand::Terminal { target, events, .. } => Effect::Terminal(
                target.pane,
                events.iter().map(|e| label(&e.event)).collect(),
            ),
            RoutedCommand::RetireTerminalPointer { target } => Effect::Retire(target.pane),
            RoutedCommand::FocusPane { target, .. } => Effect::Focus(target.pane),
            RoutedCommand::HostAction { id, action, .. } => {
                router.resolve(id, Outcome::Done);
                Effect::Host(action)
            }
            RoutedCommand::DialogDecision {
                id,
                opening,
                action,
                ..
            } => {
                router.resolve(id, Outcome::Done);
                Effect::Decide(opening.0, action)
            }
        })
        .collect()
}

fn texts(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

/// `Text a, Cmd+W, Text b, Escape, …, Text c` over a terminal: `a` reaches
/// the terminal, the close chord suspends the delivery, the dialog that
/// opened owns `b` and cancels on Escape, and the rest of that delivery is
/// barred. A fresh delivery afterwards reaches the terminal.
#[test]
fn close_chord_mid_delivery_hands_the_suffix_to_the_dialog() {
    let mut world = World::new();
    let mut router = HostRouter::default();
    let events = vec![
        Event::Text("a".into()),
        key(Key::W, cmd(), true),
        Event::Text("b".into()),
        key(Key::Escape, Modifiers::NONE, true),
        key(Key::Escape, Modifiers::NONE, false),
        key(Key::W, Modifiers::NONE, false),
        Event::Text("c".into()),
    ];
    let first = delivery(&mut router, Source::Native, events);
    let routed = router.route(&world.snapshot(), vec![first]);
    assert!(routed.held, "the suffix waits for the close to execute");
    assert_eq!(
        drain(&mut router),
        vec![
            Effect::Terminal(10, texts(&["text:a"])),
            Effect::Host(Action::ClosePane)
        ]
    );

    // Logic opened opening 7. The suffix resumes against it.
    world.dialog = Some(OpeningId(7));
    let fresh = delivery(&mut router, Source::Native, vec![Event::Text("d".into())]);
    let routed = router.route(&world.snapshot(), vec![fresh]);
    assert!(routed.egui.is_empty());
    assert!(routed.held, "the fresh delivery waits for the decision");
    assert_eq!(drain(&mut router), vec![Effect::Decide(7, "cancel".into())]);

    // Logic answered the dialog; the fresh delivery keeps its own boundary.
    world.dialog = None;
    let routed = router.route(&world.snapshot(), vec![]);
    assert!(!routed.held);
    assert_eq!(
        drain(&mut router),
        vec![Effect::Terminal(10, texts(&["text:d"]))]
    );
}

/// A critical dialog vetoes Quick Note and stays the owner: the Enter later
/// in the same delivery closes it.
#[test]
fn quick_note_is_vetoed_and_the_dialog_keeps_the_delivery() {
    let mut world = World::new();
    world.dialog = Some(OpeningId(3));
    let mut router = HostRouter::default();
    let quick_note = KeyBindings::default().open_quick_note;
    let events = delivery(
        &mut router,
        Source::Native,
        vec![
            key(quick_note.1, quick_note.0, true),
            key(quick_note.1, quick_note.0, false),
            key(Key::Enter, Modifiers::NONE, true),
        ],
    );
    router.route(&world.snapshot(), vec![events]);
    assert_eq!(drain(&mut router), vec![Effect::Decide(3, "close".into())]);
    let transitions: Vec<_> = router.traces().map(|t| t.transition.clone()).collect();
    assert_eq!(transitions[0], Transition::Veto);
    assert_eq!(transitions[2], Transition::Dismiss);
}

/// Injected Enter never runs Close; injected Escape still cancels.
#[test]
fn injected_input_cannot_close_but_can_cancel() {
    let mut world = World::new();
    world.dialog = Some(OpeningId(4));
    let mut router = HostRouter::default();
    let enter = delivery(
        &mut router,
        Source::Injected,
        vec![key(Key::Enter, Modifiers::NONE, true)],
    );
    router.route(&world.snapshot(), vec![enter]);
    assert_eq!(drain(&mut router), vec![]);
    assert!(matches!(
        router.traces().last().map(|t| &t.disposition),
        Some(Disposition::Dialog(DialogOutcome::Refused {
            reason: "synthetic input",
            ..
        }))
    ));
    let escape = delivery(
        &mut router,
        Source::Injected,
        vec![key(Key::Escape, Modifiers::NONE, true)],
    );
    router.route(&world.snapshot(), vec![escape]);
    assert_eq!(drain(&mut router), vec![Effect::Decide(4, "cancel".into())]);
}

/// Tab enters at Cancel; Enter and Space then cancel. Tab, Tab reaches
/// Close; Enter there closes. Escape and Enter are decided in received
/// order, before any repaint.
#[test]
fn tab_order_and_activation_follow_the_model() {
    let mut world = World::new();
    world.dialog = Some(OpeningId(5));
    let cases: [(Vec<Event>, &str); 5] = [
        (
            vec![
                key(Key::Tab, Modifiers::NONE, true),
                key(Key::Enter, Modifiers::NONE, true),
            ],
            "cancel",
        ),
        (
            vec![
                key(Key::Tab, Modifiers::NONE, true),
                key(Key::Space, Modifiers::NONE, true),
            ],
            "cancel",
        ),
        (
            vec![
                key(Key::Tab, Modifiers::NONE, true),
                key(Key::Tab, Modifiers::NONE, true),
                key(Key::Enter, Modifiers::NONE, true),
            ],
            "close",
        ),
        (
            vec![
                key(Key::Escape, Modifiers::NONE, true),
                key(Key::Enter, Modifiers::NONE, true),
            ],
            "cancel",
        ),
        (
            vec![
                key(Key::Enter, Modifiers::NONE, true),
                key(Key::Escape, Modifiers::NONE, true),
            ],
            "close",
        ),
    ];
    for (events, want) in cases {
        let mut router = HostRouter::default();
        let names: Vec<String> = events.iter().map(label).collect();
        let events = delivery(&mut router, Source::Native, events);
        router.route(&world.snapshot(), vec![events]);
        assert_eq!(
            drain(&mut router),
            vec![Effect::Decide(5, want.into())],
            "{names:?}"
        );
    }
    world.dialog = None;
}

/// The close chord over the dialog is its Cancel, and its W release stays
/// with the dialog rather than reaching the revealed terminal.
#[test]
fn close_chord_over_the_dialog_cancels_and_keeps_its_release() {
    let mut world = World::new();
    world.dialog = Some(OpeningId(6));
    let mut router = HostRouter::default();
    let down = delivery(&mut router, Source::Native, vec![key(Key::W, cmd(), true)]);
    router.route(&world.snapshot(), vec![down]);
    assert_eq!(drain(&mut router), vec![Effect::Decide(6, "cancel".into())]);
    world.dialog = None;
    let up = delivery(
        &mut router,
        Source::Native,
        vec![key(Key::W, Modifiers::NONE, false), Event::Text("w".into())],
    );
    router.route(&world.snapshot(), vec![up]);
    assert_eq!(
        drain(&mut router),
        vec![Effect::Terminal(10, texts(&["text:w"]))],
        "the paired release is retired; only fresh text reaches the terminal"
    );
}

/// Both buttons by pointer; a press that leaves its button activates
/// nothing; the scrim cancels; an unpresented dialog captures the pointer.
#[test]
fn pointer_activation_needs_press_and_release_on_one_target() {
    let mut world = World::new();
    world.dialog = Some(OpeningId(8));
    let click = |at: Pos2, to: Pos2| vec![button(at, true), button(to, false)];
    let cases: [(Vec<Event>, Vec<Effect>); 5] = [
        (
            click(CLOSE_RECT.center(), CLOSE_RECT.center()),
            vec![Effect::Decide(8, "close".into())],
        ),
        (
            click(CANCEL_RECT.center(), CANCEL_RECT.center()),
            vec![Effect::Decide(8, "cancel".into())],
        ),
        (click(CLOSE_RECT.center(), CANCEL_RECT.center()), vec![]),
        (
            click(Pos2::new(5.0, 5.0), Pos2::new(5.0, 5.0)),
            vec![Effect::Decide(8, "cancel".into())],
        ),
        (click(PANEL.center(), PANEL.center()), vec![]),
    ];
    for (events, want) in cases {
        let mut router = HostRouter::default();
        let events = delivery(&mut router, Source::Native, events);
        router.route(&world.snapshot(), vec![events]);
        assert_eq!(drain(&mut router), want);
    }
    world.presented = false;
    let mut router = HostRouter::default();
    let events = delivery(
        &mut router,
        Source::Native,
        click(CLOSE_RECT.center(), CLOSE_RECT.center()),
    );
    let routed = router.route(&world.snapshot(), vec![events]);
    assert_eq!(drain(&mut router), vec![]);
    assert!(
        routed.egui.is_empty(),
        "an unpresented modal still captures the pointer"
    );
}

/// An injected click on Close is refused even though press and release hit it.
#[test]
fn injected_click_cannot_close() {
    let mut world = World::new();
    world.dialog = Some(OpeningId(9));
    let mut router = HostRouter::default();
    let events = delivery(
        &mut router,
        Source::Injected,
        vec![
            button(CLOSE_RECT.center(), true),
            button(CLOSE_RECT.center(), false),
        ],
    );
    router.route(&world.snapshot(), vec![events]);
    assert_eq!(drain(&mut router), vec![]);
}

/// A drag on terminal A is retired once when the dialog takes input; its
/// late release reaches nobody; after the dialog, a fresh press works.
#[test]
fn a_terminal_gesture_retires_when_the_dialog_opens() {
    let mut world = World::new();
    let mut router = HostRouter::default();
    let at = Pos2::new(100.0, 100.0);
    let press = delivery(&mut router, Source::Native, vec![button(at, true)]);
    router.route(&world.snapshot(), vec![press]);
    assert_eq!(
        drain(&mut router),
        vec![Effect::Focus(10), Effect::Terminal(10, texts(&["press"]))]
    );
    world.dialog = Some(OpeningId(11));
    let rest = delivery(
        &mut router,
        Source::Native,
        vec![
            Event::PointerMoved(Pos2::new(120.0, 100.0)),
            button(at, false),
        ],
    );
    let routed = router.route(&world.snapshot(), vec![rest]);
    assert_eq!(drain(&mut router), vec![Effect::Retire(10)]);
    assert_eq!(routed.egui.len(), 1, "egui keeps hover, nothing else");
    world.dialog = None;
    let fresh = delivery(
        &mut router,
        Source::Native,
        vec![button(at, true), button(at, false)],
    );
    router.route(&world.snapshot(), vec![fresh]);
    assert_eq!(
        drain(&mut router),
        vec![
            Effect::Focus(10),
            Effect::Terminal(10, texts(&["press", "release"]))
        ]
    );
}

/// A press on terminal B transfers keyboard ownership at its ordinal: text
/// before it reaches A, text after it reaches B.
#[test]
fn a_press_transfers_the_rest_of_the_delivery() {
    let world = World::new();
    let mut router = HostRouter::default();
    let b = Pos2::new(900.0, 100.0);
    let events = delivery(
        &mut router,
        Source::Native,
        vec![
            Event::Text("x".into()),
            button(b, true),
            button(b, false),
            Event::Text("y".into()),
        ],
    );
    router.route(&world.snapshot(), vec![events]);
    assert_eq!(
        drain(&mut router),
        vec![
            Effect::Terminal(10, texts(&["text:x"])),
            Effect::Focus(11),
            Effect::Terminal(11, texts(&["press", "release", "text:y"])),
        ]
    );
}

/// A held no-repeat chord does not fire again, whatever the repeat flag.
#[test]
fn a_held_close_chord_fires_once() {
    let world = World::new();
    let mut router = HostRouter::default();
    let first = delivery(&mut router, Source::Native, vec![key(Key::W, cmd(), true)]);
    router.route(&world.snapshot(), vec![first]);
    assert_eq!(drain(&mut router), vec![Effect::Host(Action::ClosePane)]);
    let again = delivery(&mut router, Source::Native, vec![key(Key::W, cmd(), true)]);
    router.route(&world.snapshot(), vec![again]);
    assert_eq!(drain(&mut router), vec![]);
}

/// A blur inside a delivery retires the terminal gesture and bars the rest.
#[test]
fn blur_inside_a_delivery_retires_and_bars() {
    let world = World::new();
    let mut router = HostRouter::default();
    let at = Pos2::new(100.0, 100.0);
    let events = delivery(
        &mut router,
        Source::Native,
        vec![
            button(at, true),
            Event::WindowFocused(false),
            button(at, false),
            Event::Text("z".into()),
        ],
    );
    let routed = router.route(&world.snapshot(), vec![events]);
    assert_eq!(
        drain(&mut router),
        vec![
            Effect::Focus(10),
            Effect::Terminal(10, texts(&["press"])),
            Effect::Retire(10)
        ]
    );
    assert_eq!(routed.egui.len(), 1, "only the blur reaches egui");
}

/// After an event egui must run first (a key on a view pane), input the
/// router would claim waits for the next pass.
#[test]
fn router_input_after_a_legacy_key_waits_a_pass() {
    let mut world = World::new();
    world.keyboard = KeyboardOwner::Legacy;
    let mut router = HostRouter::default();
    let at = Pos2::new(100.0, 100.0);
    let events = delivery(
        &mut router,
        Source::Native,
        vec![key(Key::X, Modifiers::NONE, true), button(at, true)],
    );
    let routed = router.route(&world.snapshot(), vec![events]);
    assert_eq!(routed.egui.len(), 1);
    assert!(routed.held);
    assert_eq!(drain(&mut router), vec![]);
    router.route(&world.snapshot(), vec![]);
    assert_eq!(
        drain(&mut router),
        vec![Effect::Focus(10), Effect::Terminal(10, texts(&["press"]))]
    );
}

/// A failed host action retires the suffix bound to it.
#[test]
fn a_failed_action_retires_its_suffix() {
    let world = World::new();
    let mut router = HostRouter::default();
    let events = delivery(
        &mut router,
        Source::Native,
        vec![key(Key::W, cmd(), true), Event::Text("q".into())],
    );
    router.route(&world.snapshot(), vec![events]);
    let commands = router.take_commands();
    let RoutedCommand::HostAction { id, .. } = &commands[0] else {
        panic!("expected the close action: {commands:?}");
    };
    router.resolve(*id, Outcome::Failed);
    router.route(&world.snapshot(), vec![]);
    assert_eq!(drain(&mut router), vec![]);
}

/// A dismissal bars the rest of its delivery: `Escape, click` cancels once,
/// and the click in the same delivery decides nothing more.
#[test]
fn escape_then_click_in_one_delivery_decides_once() {
    let mut world = World::new();
    world.dialog = Some(OpeningId(12));
    let mut router = HostRouter::default();
    let away = Pos2::new(5.0, 5.0);
    let events = delivery(
        &mut router,
        Source::Native,
        vec![
            key(Key::Escape, Modifiers::NONE, true),
            button(away, true),
            button(away, false),
        ],
    );
    router.route(&world.snapshot(), vec![events]);
    assert_eq!(drain(&mut router), vec![Effect::Decide(12, "cancel".into())]);
}
