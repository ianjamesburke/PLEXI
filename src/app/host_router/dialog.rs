//! Host dialog model (stint 1659): one opening of an action dialog owns its
//! logical focus, Tab trap, default/cancel actions and refusal policy. Enter,
//! Space, Tab, shortcuts and pointer activation all address this model, in
//! the order they were received; paint draws the model and publishes where
//! its buttons are, and decides nothing.

use super::OpeningId;

/// One action as the router sees it: identity, keyboard shortcut and policy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ActionSpec {
    pub id: String,
    pub shortcut: Option<(egui::Modifiers, egui::Key)>,
    /// Loses work: only a fresh, real press with exact modifiers runs it.
    pub destructive: bool,
    /// A refusal (cancel): keyboard focus enters here when there is no default.
    pub refuses: bool,
}

/// What a dialog decided for one received event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DialogOutcome {
    /// Run this action: the dialog is answered.
    Activate(String),
    /// A destructive action was asked for by input that may not run it.
    Refused {
        action: String,
        reason: &'static str,
    },
    /// Tab moved logical focus.
    Moved,
    /// The event belongs to the dialog and does nothing.
    Ignored,
}

/// Where paint last drew one opening of a dialog. Hit-testing uses only the
/// presentation of the current opening.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DialogPresentation {
    pub opening: OpeningId,
    pub panel: egui::Rect,
    /// Action id, its hit rect, and whether it was drawn enabled.
    pub buttons: Vec<(String, egui::Rect, bool)>,
}

/// What a pointer position addresses in a dialog that owns input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DialogHit {
    Button(String),
    /// Inside the panel, not on a button.
    Panel,
    /// The scrim around the panel: click-away cancels.
    Scrim,
    /// The opening has not been presented yet: the pointer is captured and
    /// addresses nothing.
    Unpresented,
}

/// One opening's logical state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DialogModel {
    pub opening: OpeningId,
    pub actions: Vec<ActionSpec>,
    pub default_action: Option<String>,
    /// Index into `actions` of the Tab-focused action, `None` until Tab.
    pub focused: Option<usize>,
}

/// No Command, Ctrl or Alt: the logical match egui's `consume_key(NONE, _)`
/// applies.
fn logical_none(modifiers: egui::Modifiers) -> bool {
    !(modifiers.command || modifiers.mac_cmd || modifiers.ctrl || modifiers.alt)
}

fn exact(modifiers: egui::Modifiers, pattern: egui::Modifiers) -> bool {
    crate::host::keys::modifiers_match_exact_for_os(&modifiers, &pattern, cfg!(target_os = "macos"))
}

impl DialogModel {
    pub(crate) fn new(
        opening: OpeningId,
        actions: Vec<ActionSpec>,
        default_action: Option<String>,
    ) -> Self {
        Self {
            opening,
            actions,
            default_action,
            focused: None,
        }
    }

    /// The Tab order: the actions as given, rotated to enter at the safe
    /// action (the Enter default, else the first refusal, else the first).
    fn trap(&self) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.actions.len()).collect();
        let entry = self
            .default_action
            .as_ref()
            .and_then(|id| self.actions.iter().position(|a| &a.id == id))
            .or_else(|| self.actions.iter().position(|a| a.refuses));
        if let Some(at) = entry {
            order.rotate_left(at);
        }
        order
    }

    pub(crate) fn focused_id(&self) -> Option<&str> {
        self.focused.map(|index| self.actions[index].id.as_str())
    }

    /// Admit a destructive activation: fresh, exact, real.
    fn admit(&self, action: &ActionSpec, fresh: bool, synthetic: bool) -> DialogOutcome {
        if !action.destructive {
            return DialogOutcome::Activate(action.id.clone());
        }
        let reason = if synthetic {
            "synthetic input"
        } else if !fresh {
            "key repeat"
        } else {
            return DialogOutcome::Activate(action.id.clone());
        };
        DialogOutcome::Refused {
            action: action.id.clone(),
            reason,
        }
    }

    /// Reduce one key press. `fresh` is false for a repeat of a held key.
    pub(crate) fn key(
        &mut self,
        key: egui::Key,
        modifiers: egui::Modifiers,
        fresh: bool,
        synthetic: bool,
    ) -> DialogOutcome {
        if key == egui::Key::Tab && !self.actions.is_empty() {
            let trap = self.trap();
            let at = self
                .focused
                .and_then(|focused| trap.iter().position(|i| *i == focused));
            let next = match (at, modifiers.shift) {
                (None, false) => 0,
                (None, true) => trap.len() - 1,
                (Some(at), false) => (at + 1) % trap.len(),
                (Some(at), true) => (at + trap.len() - 1) % trap.len(),
            };
            self.focused = Some(trap[next]);
            return DialogOutcome::Moved;
        }
        let activation = matches!(key, egui::Key::Enter | egui::Key::Space);
        if activation {
            if let Some(action) = self.focused.map(|i| self.actions[i].clone()) {
                if action.destructive {
                    // Every Enter/Space belongs to a focused destructive row;
                    // only a fresh, exact, real press runs it.
                    if !exact(modifiers, egui::Modifiers::NONE) {
                        return DialogOutcome::Refused {
                            action: action.id,
                            reason: "modifiers",
                        };
                    }
                    return self.admit(&action, fresh, synthetic);
                }
                if logical_none(modifiers) {
                    return DialogOutcome::Activate(action.id);
                }
            }
        }
        let shortcut = self.actions.iter().find(|action| {
            action.shortcut.is_some_and(|(pattern, shortcut_key)| {
                shortcut_key == key
                    && if action.destructive {
                        exact(modifiers, pattern)
                    } else {
                        modifiers.matches_logically(pattern)
                    }
            })
        });
        if let Some(action) = shortcut.cloned() {
            return self.admit(&action, fresh, synthetic);
        }
        if key == egui::Key::Enter && exact(modifiers, egui::Modifiers::NONE) {
            if let Some(default) = self
                .default_action
                .as_ref()
                .and_then(|id| self.actions.iter().find(|a| &a.id == id))
                .cloned()
            {
                return self.admit(&default, fresh, synthetic);
            }
        }
        DialogOutcome::Ignored
    }

    /// The cancel action: the first refusal.
    pub(crate) fn cancel_id(&self) -> Option<&str> {
        self.actions
            .iter()
            .find(|a| a.refuses)
            .map(|a| a.id.as_str())
    }

    /// Reduce a completed pointer activation: the press and the release both
    /// addressed `hit`. `synthetic` is true when either edge was injected.
    pub(crate) fn click(
        &self,
        hit: &DialogHit,
        presentation: Option<&DialogPresentation>,
        synthetic: bool,
    ) -> DialogOutcome {
        match hit {
            DialogHit::Button(id) => {
                let enabled = presentation
                    .and_then(|p| p.buttons.iter().find(|(b, _, _)| b == id))
                    .is_some_and(|(_, _, enabled)| *enabled);
                match self.actions.iter().find(|a| &a.id == id) {
                    Some(action) if enabled => self.admit(action, true, synthetic),
                    _ => DialogOutcome::Ignored,
                }
            }
            DialogHit::Scrim => match self.cancel_id() {
                Some(id) => DialogOutcome::Activate(id.to_string()),
                None => DialogOutcome::Ignored,
            },
            DialogHit::Panel | DialogHit::Unpresented => DialogOutcome::Ignored,
        }
    }
}

/// Hit-test `pos` against the presentation of `opening`.
pub(crate) fn hit(
    opening: OpeningId,
    presentation: Option<&DialogPresentation>,
    pos: egui::Pos2,
) -> DialogHit {
    let Some(presentation) = presentation.filter(|p| p.opening == opening) else {
        return DialogHit::Unpresented;
    };
    if let Some((id, _, _)) = presentation
        .buttons
        .iter()
        .find(|(_, rect, _)| rect.contains(pos))
    {
        return DialogHit::Button(id.clone());
    }
    if presentation.panel.contains(pos) {
        DialogHit::Panel
    } else {
        DialogHit::Scrim
    }
}
