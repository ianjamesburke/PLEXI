//! Native modifier loss cannot be synthesized here: these guards exercise the
//! host boundary with the key/modifier edges supplied by the native adapter.

use crate::app::FocusKind;
use crate::testing::HostHarness;
use egui::{Event, Key, Modifiers, RawInput};

fn key(key: Key, pressed: bool, modifiers: Modifiers) -> Event {
    Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers,
    }
}

fn edge(h: &mut HostHarness, key_code: Key, pressed: bool, modifiers: Modifiers) {
    h.frame(RawInput {
        modifiers,
        events: vec![key(key_code, pressed, modifiers)],
        ..Default::default()
    });
}

#[test]
fn notes_picker_modifier_sequence_preserves_terminal_input() {
    // 1191 removed the confirmation modal. Enter/Y below deliberately replay
    // the old user's next key against today's empty picker after deletion.
    for confirm in [Key::Enter, Key::Y] {
        for close in [Key::Escape, Key::O] {
            for held_at_close in [false, true] {
                // Cmd+Escape is not a picker binding. The still-held close
                // variant is the configured Cmd+O chord, not a new shortcut.
                if close == Key::Escape && held_at_close {
                    continue;
                }
                let mut h = HostHarness::new();
                let term = h.add_focused_terminal();
                let cmd = Modifiers {
                    command: true,
                    mac_cmd: cfg!(target_os = "macos"),
                    ..Modifiers::NONE
                };
                edge(&mut h, Key::O, true, cmd);
                edge(&mut h, Key::O, false, Modifiers::NONE);
                assert!(h.app.focus_stack.contains(&FocusKind::NotesPicker));

                let dir = tempfile::tempdir().unwrap();
                let path = dir.path().join("probe.md");
                std::fs::write(&path, "probe").unwrap();
                h.app.notes_picker_entries = vec![crate::notes::NotePickerEntry {
                    title: "probe".into(),
                    preview: "probe".into(),
                    search_text: "probe".into(),
                    path: path.clone(),
                    tier_label: None,
                }];
                h.app.notes_picker_selected = 0;
                h.run_frames(1);
                edge(&mut h, Key::Backspace, true, cmd);
                assert!(!path.exists(), "probe must be deleted");
                edge(&mut h, Key::Backspace, false, Modifiers::NONE);
                edge(&mut h, confirm, true, Modifiers::NONE);
                edge(&mut h, confirm, false, Modifiers::NONE);
                let close_mods = if close == Key::O || held_at_close {
                    cmd
                } else {
                    Modifiers::NONE
                };
                edge(&mut h, close, true, close_mods);
                assert!(!h.app.focus_stack.contains(&FocusKind::NotesPicker));
                edge(
                    &mut h,
                    close,
                    false,
                    if held_at_close { cmd } else { Modifiers::NONE },
                );
                // A standalone native modifier release has no egui Key event.
                h.frame(RawInput {
                    modifiers: Modifiers::NONE,
                    ..Default::default()
                });
                h.run_frames(2);

                // Inspect the exact buffer handed to TerminalView, before the
                // widget consumes it; checking only PTY bytes misses stale flags.
                let ctx = h.app.ctx.clone();
                let _ = ctx.run_ui(
                    RawInput {
                        modifiers: Modifiers::NONE,
                        events: vec![key(Key::V, true, Modifiers::NONE), Event::Text("v".into())],
                        ..Default::default()
                    },
                    |ui| {
                        let input = h
                            .app
                            .take_focused_terminal_input(ui.ctx())
                            .expect("terminal must own input after close");
                        assert_eq!(input.modifiers, Modifiers::NONE);
                        assert_eq!(
                            input.keyboard_events,
                            vec![key(Key::V, true, Modifiers::NONE), Event::Text("v".into())]
                        );
                    },
                );

                // The inspection pass above deliberately didn't paint widgets.
                // Restore the terminal widget's egui focus before testing PTY
                // delivery through complete production frames.
                edge(&mut h, Key::V, false, Modifiers::NONE);
                h.run_frames(2);
                h.terminal_backend(term).enable_input_tap();
                for _ in 0..10 {
                    h.frame(RawInput {
                        events: vec![key(Key::V, true, Modifiers::NONE), Event::Text("v".into())],
                        ..Default::default()
                    });
                    edge(&mut h, Key::V, false, Modifiers::NONE);
                }
                assert_eq!(
                    h.terminal_backend(term).take_input_tap(),
                    b"vvvvvvvvvv",
                    "confirm={confirm:?} close={close:?} held_at_close={held_at_close}"
                );
            }
        }
    }
}
