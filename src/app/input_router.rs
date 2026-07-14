//! `PlexiInput`: frame-scoped ownership-transfer router for keyboard/text input.
//!
//! Historically every consumer (overlays, app panes, `keys::poll_actions`, the
//! vendored terminal widget) independently peeked or mutated the single shared
//! `ctx.input()` queue. Ownership was enforced retroactively — whichever
//! consumer's render call happened to run first got the event, and a stray
//! reorder could silently change behavior (see the paste-leak class of bug,
//! #1236). `PlexiInput` makes ownership explicit instead: at the start of each
//! frame every input-*intent* event (physical keys, typed text, paste) is
//! removed from `ctx` into an owned buffer. That buffer is handed, in order,
//! to `keys::poll_actions` (the global hotkey allowlist — always runs first,
//! see module doc on why) and then to the top [`crate::app::FocusLayer`] on
//! `PlexiApp.focus_stack`, each consuming only what it claims. Whatever
//! neither claims is handed back to `ctx` so the render pass (focused
//! `TextEdit` widgets, app pane `handle_key`, the terminal widget) still sees
//! it — this refactor is scoped to the overlay/global-hotkey ownership path;
//! it does not (yet) migrate app-pane or terminal PTY input to full
//! ownership-transfer (see `stint 0240` design-decision notes).
//!
//! Mouse events, scroll, hover, and paint requests are untouched: this router
//! is keyboard/text-input scoped only.

/// Returns true for events this router takes ownership of: physical keys,
/// typed text, and paste. IME composition events
/// (`Event::Ime`) are deliberately excluded — composition must reach the
/// focused egui widget directly during the render pass for correct pre-edit
/// rendering, so carving them out here (rather than letting them fall through
/// a catch-all) avoids a silent foot-gun.
fn is_input_intent(event: &egui::Event) -> bool {
    matches!(
        event,
        egui::Event::Key { .. } | egui::Event::Text(_) | egui::Event::Paste(_)
    )
}

/// Owned, frame-scoped buffer of input-intent events removed from `ctx`.
pub(crate) struct PlexiInput {
    events: Vec<egui::Event>,
    modifiers: egui::Modifiers,
}

impl PlexiInput {
    /// Remove all input-intent events from `ctx`'s raw event queue into an
    /// owned buffer, leaving IME and non-keyboard events (paint, motion,
    /// scroll, hover) untouched for egui's normal hit-testing/paint path.
    pub(crate) fn take_from(ctx: &egui::Context) -> Self {
        let (events, modifiers) = ctx.input_mut(|i| {
            let mut taken = Vec::new();
            i.events.retain(|e| {
                if is_input_intent(e) {
                    taken.push(e.clone());
                    false
                } else {
                    true
                }
            });
            (taken, i.modifiers)
        });
        Self { events, modifiers }
    }

    pub(crate) fn modifiers(&self) -> egui::Modifiers {
        self.modifiers
    }

    /// True when the buffer holds no more input-intent events (everything was
    /// claimed by a consumer this frame, or none arrived).
    pub(crate) fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Consume the first matching key-press event, removing it from the
    /// buffer. Mirrors `egui::InputState::consume_key`, but operates on this
    /// frame's owned buffer instead of the shared `ctx` queue, so a claim here
    /// is final — no other consumer this frame (or the render pass that
    /// follows, once events are given back) can see it again.
    pub(crate) fn consume_key(&mut self, modifiers: egui::Modifiers, key: egui::Key) -> bool {
        if let Some(pos) = self.events.iter().position(|e| {
            matches!(
                e,
                egui::Event::Key { key: k, pressed: true, modifiers: m, .. }
                    if *k == key && m.matches_logically(modifiers)
            )
        }) {
            self.events.remove(pos);
            true
        } else {
            false
        }
    }

    /// Consume a key press only if it isn't a synthetic OS auto-repeat.
    /// Mirrors `keys::consume_key_no_repeat`'s intent for callers holding a
    /// `PlexiInput` instead of a raw `InputState`.
    pub(crate) fn consume_key_no_repeat(&mut self, modifiers: egui::Modifiers, key: egui::Key) -> bool {
        if let Some(pos) = self.events.iter().position(|e| {
            matches!(
                e,
                egui::Event::Key { key: k, pressed: true, repeat: false, modifiers: m, .. }
                    if *k == key && m.matches_logically(modifiers)
            )
        }) {
            self.events.remove(pos);
            true
        } else {
            false
        }
    }

    /// Return every event still owned by this buffer to `ctx` so consumers
    /// later in the frame (a focused `TextEdit`, `dispatch_app_key_events`,
    /// the terminal widget's own read of `ctx.input()`) see whatever this
    /// frame's router-level consumers didn't claim. No-op if the buffer is
    /// already empty.
    pub(crate) fn give_back(self, ctx: &egui::Context) {
        if self.is_empty() {
            return;
        }
        ctx.input_mut(|i| {
            let mut merged = self.events;
            merged.append(&mut i.events);
            i.events = merged;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key_event(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    #[test]
    fn take_from_removes_key_events_but_leaves_pointer_events() {
        let ctx = egui::Context::default();
        let mut raw = egui::RawInput::default();
        raw.events.push(key_event(egui::Key::A, egui::Modifiers::COMMAND));
        raw.events.push(egui::Event::PointerMoved(egui::pos2(1.0, 1.0)));
        let _ = ctx.run(raw, |ctx| {
            let input = PlexiInput::take_from(ctx);
            assert_eq!(input.events.len(), 1);
            // Pointer event stays behind in ctx.
            ctx.input(|i| {
                assert_eq!(i.events.len(), 1);
                assert!(matches!(i.events[0], egui::Event::PointerMoved(_)));
            });
        });
    }

    #[test]
    fn consume_key_claims_exactly_one_matching_event() {
        let ctx = egui::Context::default();
        let mut raw = egui::RawInput::default();
        raw.events.push(key_event(egui::Key::A, egui::Modifiers::COMMAND));
        let _ = ctx.run(raw, |ctx| {
            let mut input = PlexiInput::take_from(ctx);
            assert!(input.consume_key(egui::Modifiers::COMMAND, egui::Key::A));
            assert!(!input.consume_key(egui::Modifiers::COMMAND, egui::Key::A));
            assert!(input.is_empty());
        });
    }

    #[test]
    fn give_back_restores_unclaimed_events_to_ctx() {
        let ctx = egui::Context::default();
        let mut raw = egui::RawInput::default();
        raw.events.push(key_event(egui::Key::B, egui::Modifiers::NONE));
        let _ = ctx.run(raw, |ctx| {
            let input = PlexiInput::take_from(ctx);
            input.give_back(ctx);
            ctx.input(|i| {
                assert_eq!(i.events.len(), 1);
                assert!(matches!(i.events[0], egui::Event::Key { key: egui::Key::B, .. }));
            });
        });
    }
}
