//! `PlexiInput`: frame-scoped ownership-transfer router for keyboard/text input.
//!
//! Historically every consumer (overlays, app panes, `keys::poll_actions`, the
//! vendored terminal widget) independently peeked or mutated the single shared
//! `ctx.input()` queue. Ownership was enforced retroactively — whichever
//! consumer's render call happened to run first got the event, and a stray
//! reorder could silently change behavior (see the paste-leak class of bug,
//! #1236). `PlexiInput` makes ownership explicit instead: at the start of each
//! frame every input-*intent* event (physical keys, typed text, paste, and the
//! clipboard `Copy`/`Cut` shortcut events) is removed from `ctx` into an owned
//! buffer, handed to consumers in a fixed order, each consuming only what it
//! claims.
//!
//! Every keyboard consumer now routes through this buffer (stint 0387):
//!   1. `keys::poll_actions` — the global hotkey allowlist. Always the outer,
//!      first consumer of the frame; claims Cmd+Q/W/P etc.
//!   2. The top [`crate::app::FocusKind`] overlay's `*_handle_key`, when an
//!      overlay owns input.
//!   3. `dispatch_app_key_events` — the focused app pane's `App::handle_key`,
//!      which reads this buffer directly (see [`PlexiInput::events`] /
//!      [`PlexiInput::key_pressed`]) instead of `ctx.input()`.
//!   4. The focused terminal widget. Because the host takes this buffer *before*
//!      the render pass and hands the events straight to `egui_term` via
//!      `TerminalView::with_input`, egui's own render-time widget machinery can
//!      no longer swallow a key (e.g. Cmd+A select-all) before the terminal
//!      sees it — the structural fix for the terminal Cmd+A bug.
//!
//! Overlay and app/global consumers `give_back` whatever they don't claim so
//! the render pass (focused `TextEdit` widgets) still sees it. The focused
//! terminal is the sole intended keyboard consumer for its frame, so its host
//! handoff does *not* give events back: anything the terminal ignores is
//! correctly dropped rather than leaking to an unfocused widget.
//!
//! Mouse events, scroll, hover, and paint requests are untouched: this router
//! is keyboard/text-input scoped only. `Event::Ime` composition is also
//! deliberately excluded — see [`is_input_intent`].

/// Returns true for events this router takes ownership of: physical keys,
/// typed text, paste, and the clipboard `Copy`/`Cut` shortcut events (on macOS
/// Cmd+C / Cmd+X arrive as `Event::Copy` / `Event::Cut`, and both the terminal
/// widget and focused `TextEdit`s consume them). Widening the taken set is safe
/// for the overlay/app/global consumers because they `give_back` everything
/// unclaimed before their render pass, so a `TextEdit` still receives an
/// unclaimed Copy/Cut.
///
/// IME composition events (`Event::Ime`) are deliberately excluded — pre-edit
/// composition must reach the focused egui widget directly during the render
/// pass for correct pre-edit rendering, and no router consumer processes
/// composition. Carving it out here (rather than letting it fall through a
/// catch-all) avoids a silent foot-gun.
fn is_input_intent(event: &egui::Event) -> bool {
    matches!(
        event,
        egui::Event::Key { .. }
            | egui::Event::Text(_)
            | egui::Event::Paste(_)
            | egui::Event::Copy
            | egui::Event::Cut
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

    /// The frame's owned input-intent events, in arrival order. Consumers that
    /// need the raw event stream (e.g. `App::handle_key` for WASM/Python panes
    /// that forward every key/text edge to their guest) read this instead of
    /// `ctx.input(|i| &i.events)`.
    pub(crate) fn events(&self) -> &[egui::Event] {
        &self.events
    }

    /// Was `key` pressed this frame? Mirrors `egui::InputState::key_pressed`
    /// exactly — any `Event::Key { pressed: true }` for this logical key,
    /// modifiers ignored, key-repeat included — but over this frame's owned
    /// buffer so `App::handle_key` classifies from the router rather than a
    /// second read of the shared `ctx` queue.
    pub(crate) fn key_pressed(&self, key: egui::Key) -> bool {
        self.events.iter().any(|e| {
            matches!(
                e,
                egui::Event::Key { key: k, pressed: true, .. } if *k == key
            )
        })
    }

    /// True when the buffer holds no more input-intent events (everything was
    /// claimed by a consumer this frame, or none arrived).
    pub(crate) fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Consume the whole buffer into its owned events + modifiers. Used by the
    /// focused-terminal handoff, which hands every remaining event straight to
    /// `egui_term` (the sole intended keyboard consumer for its frame) and
    /// never gives events back to `ctx`.
    pub(crate) fn into_events_and_modifiers(self) -> (Vec<egui::Event>, egui::Modifiers) {
        (self.events, self.modifiers)
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

    #[test]
    fn take_from_claims_copy_and_cut_but_leaves_ime() {
        let ctx = egui::Context::default();
        let mut raw = egui::RawInput::default();
        raw.events.push(egui::Event::Copy);
        raw.events.push(egui::Event::Cut);
        raw.events
            .push(egui::Event::Ime(egui::ImeEvent::Enabled));
        let _ = ctx.run(raw, |ctx| {
            let input = PlexiInput::take_from(ctx);
            // Copy + Cut are input-intent and taken.
            assert_eq!(input.events().len(), 2);
            // IME composition is left behind for the render pass.
            ctx.input(|i| {
                assert_eq!(i.events.len(), 1);
                assert!(matches!(i.events[0], egui::Event::Ime(_)));
            });
        });
    }

    #[test]
    fn key_pressed_matches_buffer_ignoring_modifiers() {
        let ctx = egui::Context::default();
        let mut raw = egui::RawInput::default();
        raw.events.push(key_event(egui::Key::A, egui::Modifiers::COMMAND));
        let _ = ctx.run(raw, |ctx| {
            let input = PlexiInput::take_from(ctx);
            assert!(input.key_pressed(egui::Key::A));
            assert!(!input.key_pressed(egui::Key::B));
        });
    }

    #[test]
    fn into_events_and_modifiers_yields_the_owned_buffer() {
        let ctx = egui::Context::default();
        let mut raw = egui::RawInput::default();
        raw.modifiers = egui::Modifiers::COMMAND;
        raw.events.push(key_event(egui::Key::A, egui::Modifiers::COMMAND));
        let _ = ctx.run(raw, |ctx| {
            let input = PlexiInput::take_from(ctx);
            let (events, modifiers) = input.into_events_and_modifiers();
            assert_eq!(events.len(), 1);
            assert!(modifiers.command);
        });
    }
}
