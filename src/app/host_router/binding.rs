//! Pure one-event host binding reduction (stint 1659).
//!
//! One fresh key press, with the modifiers it was received with, matches at
//! most one host binding. The table's order is its priority
//! (`keys::build_binding_table` sorts exact and more specific chords first).
//! Repeats are not decided here: the router's edge record for the physical
//! key says whether a press is fresh, and the matched entry's `no_repeat`
//! says what its repeats do.

use crate::host::keys::{Action, BindingContext, BindingEntry};

/// Which binding contexts are live for the press's recipient.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct BindingGates {
    /// An overlay owns input: only `Global` bindings match.
    pub overlay_open: bool,
    /// The focused pane has a multi-pane selection: bare Escape cancels it.
    pub pane_selection: bool,
    /// The focused pane has conjoined tool children: Cmd+Down zooms them.
    pub conjoined_zoom: bool,
}

/// The host binding a fresh press triggers, if any.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Matched {
    pub action: Action,
    /// Repeats of this press are consumed without acting.
    pub no_repeat: bool,
}

/// Match one press against `table` and the router's conditional chords.
pub(crate) fn match_press(
    table: &[BindingEntry],
    key: egui::Key,
    modifiers: egui::Modifiers,
    gates: BindingGates,
) -> Option<Matched> {
    let exact = |pattern: &egui::Modifiers| {
        crate::host::keys::modifiers_match_exact_for_os(
            &modifiers,
            pattern,
            cfg!(target_os = "macos"),
        )
    };
    for entry in table {
        let live = match entry.context {
            BindingContext::Global => true,
            BindingContext::Normal => !gates.overlay_open,
            // View-active bindings belong to a focused view pane, which the
            // router does not own.
            BindingContext::ViewActive => false,
        };
        if !live || entry.key != key {
            continue;
        }
        let matches = if entry.exact {
            exact(&entry.modifiers)
        } else {
            modifiers.matches_logically(entry.modifiers)
        };
        if matches {
            return Some(Matched {
                action: entry.action.clone(),
                no_repeat: entry.no_repeat,
            });
        }
    }
    if gates.overlay_open {
        return None;
    }
    const CONTEXT_KEYS: [egui::Key; 9] = [
        egui::Key::Num1,
        egui::Key::Num2,
        egui::Key::Num3,
        egui::Key::Num4,
        egui::Key::Num5,
        egui::Key::Num6,
        egui::Key::Num7,
        egui::Key::Num8,
        egui::Key::Num9,
    ];
    if let Some(index) = CONTEXT_KEYS.iter().position(|k| *k == key) {
        if exact(&egui::Modifiers::COMMAND) {
            return Some(Matched {
                action: Action::SwitchContext(index),
                no_repeat: false,
            });
        }
    }
    if gates.conjoined_zoom && key == egui::Key::ArrowDown && exact(&egui::Modifiers::COMMAND) {
        return Some(Matched {
            action: Action::ToggleZoom,
            no_repeat: true,
        });
    }
    if gates.pane_selection && key == egui::Key::Escape && exact(&egui::Modifiers::NONE) {
        return Some(Matched {
            action: Action::CancelPaneSelection,
            no_repeat: true,
        });
    }
    None
}
