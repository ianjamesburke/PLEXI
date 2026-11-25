//! Stint 1391: the picker's key map. Ordering against the field and the
//! rows lives in `app::tests::command_palette_oracle_tests` and the
//! `ui_tests::tests::palette*` cases.

use super::*;

fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

/// Review round 3: a native macOS Cmd chord sets `mac_cmd` too; Cmd+J and
/// Cmd+K still move, and an extra modifier is no picker key.
#[test]
fn command_palette_native_command_chords_move() {
    let native_cmd = egui::Modifiers {
        command: true,
        mac_cmd: cfg!(target_os = "macos"),
        ctrl: !cfg!(target_os = "macos"),
        ..Default::default()
    };
    assert_eq!(
        picker_key(&key(egui::Key::J, native_cmd), false),
        Some(PickerCommand::Move(Step::Down))
    );
    assert_eq!(
        picker_key(&key(egui::Key::K, native_cmd), false),
        Some(PickerCommand::Move(Step::Up))
    );
    let mut with_alt = native_cmd;
    with_alt.alt = true;
    assert_eq!(picker_key(&key(egui::Key::J, with_alt), false), None);
    assert_eq!(
        picker_key(&key(egui::Key::Tab, egui::Modifiers::SHIFT), false),
        Some(PickerCommand::PrevStop)
    );
}

/// An Enter during IME composition confirms the composition: it is field
/// input, never the picker's Accept.
#[test]
fn command_palette_enter_while_composing_is_the_fields() {
    let enter = key(egui::Key::Enter, egui::Modifiers::NONE);
    assert_eq!(
        picker_key(&enter, false),
        Some(PickerCommand::Accept(None, Default::default()))
    );
    assert_eq!(picker_key(&enter, true), None);
    assert!(is_field_input(&enter));
    assert!(is_field_input(&key(egui::Key::A, egui::Modifiers::CTRL)));
    assert!(!is_field_input(&egui::Event::PointerGone));
}
