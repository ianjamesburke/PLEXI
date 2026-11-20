//! The host input router (stint 1659): one ordered delivery, one owner per
//! event, owner changes committed before the next event.
//!
//! eframe calls [`eframe::App::raw_input_hook`] with each native delivery
//! *before* egui folds it into pointer, key and wheel state; `App::logic` and
//! `App::ui` run inside the pass that folds it. So ownership is decided here,
//! pre-fold, by [`PlexiApp::route_delivery`]: every actionable event gets
//! exactly one disposition at its ordinal:
//!
//! - consumed by a structured owner (an overlay's `handle_key`, a host
//!   binding, a pane plugin's `handle_key`, the dismissal barrier), or
//! - appended to the focused terminal's ordered stream, or
//! - left in the `RawInput` for egui, but only when its owner is the single
//!   owner that holds egui input at the end of the delivery.
//!
//! An owner is an *instance*: an [`InputOwner`] plus an epoch that advances on
//! every observed owner change, so two openings of one overlay differ.
//!
//! Down edges are paired with their up edges independently of focus: the
//! router keeps its own down set (egui-winit reports `repeat: false` and
//! expects egui to derive repeats), and a release, repeat or the text a held
//! key produces follows the owner that took the down. A departed owner's
//! release is retired, never handed to whoever owns input now. A pointer
//! gesture whose owner loses egui input is cancelled off-screen at once.
//!
//! [`DISMISSAL_BARRIER`] is the one policy point for what follows a dismissal.

use crate::app::input_owner::InputOwner;
use crate::app::PlexiApp;
use crate::spatial::tiling::PaneId;
use std::collections::HashMap;

/// Dismissal barrier (decisions ledger row 152, pending Ian's confirmation).
///
/// `true`: when an event dismisses, accepts in or otherwise retires the overlay
/// that owned it (without an explicit transfer to a newly opened owner), the
/// rest of that delivery is consumed. `Escape, click` therefore never clicks
/// the pane the Escape revealed; that pane is eligible from the next delivery.
/// Releases are still retired and lifecycle events still reach egui.
///
/// `false`: later events in the same delivery go to the revealed owner.
pub(crate) const DISMISSAL_BARRIER: bool = true;

/// Off-screen position used to cancel a pointer gesture whose owner lost egui
/// input: moving there first makes the release a non-click for every widget.
const CANCEL_POS: egui::Pos2 = egui::pos2(-100_000.0, -100_000.0);

/// Every pointer button, indexed as `button as usize`.
const POINTER_BUTTONS: [egui::PointerButton; egui::NUM_POINTER_BUTTONS] = [
    egui::PointerButton::Primary,
    egui::PointerButton::Secondary,
    egui::PointerButton::Middle,
    egui::PointerButton::Extra1,
    egui::PointerButton::Extra2,
];

/// One owner's input, handed to a structured handler (`FocusOwner::handle_key`,
/// `View::handle_key`, a host surface handler). The router builds one per
/// routed event; a handler claims the event by removing it.
pub(crate) struct PlexiInput {
    events: Vec<egui::Event>,
    modifiers: egui::Modifiers,
}

impl PlexiInput {
    /// A buffer over `events` with `modifiers` as the modifier state.
    pub(crate) fn synthetic(events: Vec<egui::Event>, modifiers: egui::Modifiers) -> Self {
        Self { events, modifiers }
    }

    pub(crate) fn modifiers(&self) -> egui::Modifiers {
        self.modifiers
    }

    /// The buffer's events, in arrival order.
    pub(crate) fn events(&self) -> &[egui::Event] {
        &self.events
    }

    /// Was `key` pressed? Any `Event::Key { pressed: true }` for this logical
    /// key, modifiers ignored, repeats included.
    pub(crate) fn key_pressed(&self, key: egui::Key) -> bool {
        self.events.iter().any(|e| {
            matches!(
                e,
                egui::Event::Key { key: k, pressed: true, .. } if *k == key
            )
        })
    }

    /// True when every event was claimed (or none arrived).
    pub(crate) fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Claim every remaining event.
    pub(crate) fn clear(&mut self) {
        self.events.clear();
    }

    /// Claim the first key press matching `key` with logically matching
    /// modifiers.
    pub(crate) fn consume_key(&mut self, modifiers: egui::Modifiers, key: egui::Key) -> bool {
        self.consume_first(|e| {
            matches!(
                e,
                egui::Event::Key { key: k, pressed: true, modifiers: m, .. }
                    if *k == key && m.matches_logically(modifiers)
            )
        })
    }

    /// Like [`Self::consume_key`], but with the host's platform-aware exact
    /// modifier match, so a more specific chord (Cmd+Shift+0) is not claimed
    /// by a less specific one (Cmd+0) and native Cmd (`mac_cmd`) matches.
    pub(crate) fn consume_key_exact(&mut self, modifiers: egui::Modifiers, key: egui::Key) -> bool {
        self.consume_first(|e| {
            matches!(
                e,
                egui::Event::Key { key: k, pressed: true, modifiers: m, .. }
                    if *k == key
                        && crate::host::keys::modifiers_match_exact_for_os(
                            m,
                            &modifiers,
                            cfg!(target_os = "macos"),
                        )
            )
        })
    }

    /// Claim a key press only if it is not a repeat.
    pub(crate) fn consume_key_no_repeat(
        &mut self,
        modifiers: egui::Modifiers,
        key: egui::Key,
    ) -> bool {
        self.consume_first(|e| {
            matches!(
                e,
                egui::Event::Key { key: k, pressed: true, repeat: false, modifiers: m, .. }
                    if *k == key && m.matches_logically(modifiers)
            )
        })
    }

    /// Claim a Copy action (Cmd+C key press or `egui::Event::Copy`).
    pub(crate) fn consume_copy(&mut self) -> bool {
        self.consume_key(egui::Modifiers::COMMAND, egui::Key::C)
            || self.consume_first(|e| matches!(e, egui::Event::Copy))
    }

    /// Claim a Cut action (Cmd+X key press or `egui::Event::Cut`).
    pub(crate) fn consume_cut(&mut self) -> bool {
        self.consume_key(egui::Modifiers::COMMAND, egui::Key::X)
            || self.consume_first(|e| matches!(e, egui::Event::Cut))
    }

    fn consume_first(&mut self, pred: impl Fn(&egui::Event) -> bool) -> bool {
        match self.events.iter().position(pred) {
            Some(pos) => {
                self.events.remove(pos);
                true
            }
            None => false,
        }
    }
}

/// An owner instance: who, and which opening of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OwnerInstance {
    pub(crate) owner: InputOwner,
    pub(crate) epoch: u64,
}

/// Where a down edge went, so its repeats and release follow it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EdgePath {
    /// A host binding claimed it: repeats re-match while its owner holds
    /// input, and the release is retired.
    Claimed,
    /// The owner's structured handler consumed it: repeats and the release
    /// go back to that owner while it still holds input.
    Owner,
    /// The owner's egui widgets (left in `RawInput`).
    Egui,
    /// A terminal pane's ordered stream.
    Terminal,
    /// Behind the dismissal barrier, for a departed owner, or across a blur:
    /// repeats, text and the release are consumed.
    Retired,
}

#[derive(Clone, Copy, Debug)]
struct HeldEdge {
    owner: OwnerInstance,
    path: EdgePath,
    /// The terminal pane whose stream carried the press, if any.
    terminal: Option<PaneId>,
    /// Retired by a window blur: no release may follow, so a later press of
    /// the same key starts fresh instead of reading as a repeat.
    blurred: bool,
}

impl HeldEdge {
    fn new(owner: OwnerInstance, path: EdgePath) -> Self {
        Self {
            owner,
            path,
            terminal: None,
            blurred: false,
        }
    }
}

/// A pane as last presented: where a press can land on it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PresentedPane {
    pub(crate) window_id: u64,
    pub(crate) tile: egui_tiles::TileId,
    pub(crate) pane: PaneId,
    pub(crate) hit_rect: egui::Rect,
    pub(crate) layer: egui::LayerId,
}

/// A press the router assigned to a pane at its ordinal (focus and selection
/// already applied). Rendering applies only the click's history and flash
/// from this; it never moves focus again.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PanePress {
    pub(crate) window_id: u64,
    pub(crate) tile: egui_tiles::TileId,
    /// Focus before the press moved it, for focus history and the click flash.
    pub(crate) prior_focus: Option<egui_tiles::TileId>,
}

/// What the router produced for one routed event.
enum Disposition {
    Consumed,
    Egui,
    Terminal(PaneId),
}

/// A wheel gesture (`TouchPhase::Start` .. `End`/`Cancel`) and its owner.
#[derive(Clone, Copy, Debug)]
struct WheelGesture {
    owner: OwnerInstance,
    terminal: Option<PaneId>,
}

/// Router state that outlives a delivery.
#[derive(Default)]
pub(crate) struct InputRouter {
    delivery: u64,
    epoch: u64,
    current: Option<InputOwner>,
    held_keys: HashMap<egui::Key, HeldEdge>,
    held_buttons: [Option<HeldEdge>; egui::NUM_POINTER_BUTTONS],
    wheel: Option<WheelGesture>,
    last_pointer: Option<egui::Pos2>,
    /// The rest of a split delivery, routed by the next pass, and the owner
    /// instance it was held for. Retired, never replayed, if that owner is
    /// gone by then.
    held_input: Vec<egui::Event>,
    held_for: Option<OwnerInstance>,
    /// Epoch of the overlay instance most recently drawn in full.
    presented_epoch: Option<u64>,
    /// Pane hit areas from the last visible pass.
    presented_panes: Vec<PresentedPane>,
    /// The conjoined parent pane's presented rect, when one is shown.
    conjoined_parent_hit: Option<(PaneId, egui::Rect, egui::LayerId)>,
    /// Font and UI-scale actions already run in this delivery: one physical
    /// press can arrive as both `Equals` and `Plus`.
    scale_actions: Vec<crate::host::keys::Action>,
    /// This pass's outputs, read by the pass that folds it.
    pub(crate) egui_owner: Option<InputOwner>,
    pub(crate) terminal_streams: Vec<crate::render::terminal_pane::TerminalInput>,
    pub(crate) trap_tab: Option<bool>,
    pub(crate) pane_press: Option<PanePress>,
}

impl InputRouter {
    /// Note `owner` and return its instance, advancing the epoch on change.
    pub(crate) fn observe(&mut self, owner: InputOwner) -> OwnerInstance {
        if self.current != Some(owner) {
            self.epoch += 1;
            self.current = Some(owner);
        }
        OwnerInstance {
            owner,
            epoch: self.epoch,
        }
    }

    /// Record that `instance` was drawn in full this pass, so its painted
    /// geometry can be hit-tested by the next delivery.
    pub(crate) fn note_presented(&mut self, instance: OwnerInstance) {
        if matches!(instance.owner, InputOwner::Overlay(_)) {
            self.presented_epoch = Some(instance.epoch);
        }
    }

    pub(crate) fn set_presented_panes(&mut self, panes: Vec<PresentedPane>) {
        self.presented_panes = panes;
    }

    pub(crate) fn set_conjoined_parent_hit(
        &mut self,
        hit: Option<(PaneId, egui::Rect, egui::LayerId)>,
    ) {
        self.conjoined_parent_hit = hit;
    }

    /// True while part of a split delivery waits for the next pass.
    pub(crate) fn has_held_input(&self) -> bool {
        !self.held_input.is_empty()
    }

    /// Deliveries routed so far: a test proves a harness step reached
    /// `raw_input_hook`.
    #[cfg(test)]
    pub(crate) fn deliveries(&self) -> u64 {
        self.delivery
    }

    /// True while `key`'s press was consumed by a host binding, a structured
    /// owner or the barrier, and its release has not arrived yet.
    #[cfg(test)]
    pub(crate) fn holds_claimed_key(&self, key: egui::Key) -> bool {
        self.held_keys.get(&key).is_some_and(|held| {
            matches!(
                held.path,
                EdgePath::Claimed | EdgePath::Owner | EdgePath::Retired
            )
        })
    }

    fn presented(&self, instance: OwnerInstance) -> bool {
        self.presented_epoch == Some(instance.epoch)
    }

    /// Append `event` to `pane`'s terminal stream, keeping the order of every
    /// stream and of events within it.
    fn push_terminal(&mut self, pane: PaneId, event: egui::Event, modifiers: egui::Modifiers) {
        match self
            .terminal_streams
            .iter_mut()
            .find(|stream| stream.pane_id == pane)
        {
            Some(stream) => {
                stream.keyboard_events.push(event);
                stream.modifiers = modifiers;
            }
            None => self
                .terminal_streams
                .push(crate::render::terminal_pane::TerminalInput {
                    pane_id: pane,
                    keyboard_events: vec![event],
                    modifiers,
                }),
        }
    }

    /// A window blur: no release will follow for keys held now. Their
    /// records stay, retired, so a late release reaches nobody, and a fresh
    /// press of the same key starts over.
    fn retire_for_blur(&mut self) {
        for held in self.held_keys.values_mut() {
            held.path = EdgePath::Retired;
            held.blurred = true;
        }
    }
}

fn event_kind(event: &egui::Event) -> &'static str {
    match event {
        egui::Event::Key { .. } => "key",
        egui::Event::Text(_) => "text",
        egui::Event::Paste(_) => "paste",
        egui::Event::Copy | egui::Event::Cut => "clipboard",
        egui::Event::Ime(_) => "ime",
        egui::Event::PointerButton { .. } => "pointer_button",
        egui::Event::PointerMoved(_) | egui::Event::MouseMoved(_) => "pointer_move",
        egui::Event::MouseWheel { .. } => "wheel",
        _ => "other",
    }
}

/// Events egui folds into shared interaction state. Everything else
/// (screenshots, window focus, lifecycle) passes through untouched.
fn is_actionable(event: &egui::Event) -> bool {
    matches!(
        event,
        egui::Event::Key { .. }
            | egui::Event::Text(_)
            | egui::Event::Paste(_)
            | egui::Event::Copy
            | egui::Event::Cut
            | egui::Event::Ime(_)
            | egui::Event::PointerMoved(_)
            | egui::Event::MouseMoved(_)
            | egui::Event::PointerButton { .. }
            | egui::Event::PointerGone
            | egui::Event::MouseWheel { .. }
            | egui::Event::Zoom(_)
            | egui::Event::Rotate(_)
            | egui::Event::Touch { .. }
            | egui::Event::AccessKitActionRequest(_)
    )
}

/// An event that can decide something: a key press, a pointer edge, a
/// paste, clipboard command, IME commit or accessibility action. Once an
/// egui run holds input in a pass, the next decisive event waits for the
/// next pass, after egui has applied the run (the run-splitting rule).
fn is_decisive(event: &egui::Event) -> bool {
    matches!(
        event,
        egui::Event::Key { pressed: true, .. }
            | egui::Event::PointerButton { .. }
            | egui::Event::Paste(_)
            | egui::Event::Copy
            | egui::Event::Cut
            | egui::Event::Ime(egui::ImeEvent::Commit(_))
            | egui::Event::AccessKitActionRequest(_)
    )
}

/// An event that changes what an egui widget holds (an edit, a press, a
/// release). Folding one opens the pass's egui run.
fn changes_egui_state(event: &egui::Event) -> bool {
    is_decisive(event) || matches!(event, egui::Event::Text(_))
}

/// Keyboard-side input an overlay's egui fields take: held, not dropped,
/// while that overlay has not been drawn yet (its field has no focus).
fn is_field_input(event: &egui::Event) -> bool {
    matches!(
        event,
        egui::Event::Key { .. }
            | egui::Event::Text(_)
            | egui::Event::Paste(_)
            | egui::Event::Copy
            | egui::Event::Cut
            | egui::Event::Ime(_)
    )
}

fn event_pos(event: &egui::Event) -> Option<egui::Pos2> {
    match event {
        egui::Event::PointerMoved(pos) | egui::Event::PointerButton { pos, .. } => Some(*pos),
        _ => None,
    }
}

impl PlexiApp {
    /// Route one native delivery (see the module docs). Called from
    /// `raw_input_hook` after queued pane IPC input has been appended, so
    /// injected and native events share one order. Input held from a split
    /// delivery is routed first, ahead of anything new.
    pub(crate) fn route_delivery(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.input_router.delivery += 1;
        let delivery = self.input_router.delivery;
        self.input_router.egui_owner = None;
        self.input_router.terminal_streams.clear();
        self.input_router.trap_tab = None;
        self.input_router.pane_press = None;
        self.input_router.scale_actions.clear();
        // Owner state before ordinal 0: overlay layers reflect what changed
        // since the last pass (IPC, a decision applied in logic or ui).
        self.sync_overlay_layers();
        let hidden = raw.viewport().visible() == Some(false);
        let os_focused = raw
            .viewport()
            .focused
            .unwrap_or_else(|| ctx.input(|i| i.viewport().focused.unwrap_or(true)));
        let modifiers = raw.modifiers;
        let incoming = std::mem::take(&mut raw.events);
        let mut out: Vec<(egui::Event, Option<OwnerInstance>)> = Vec::new();

        // Held input first. A hidden pass leaves it waiting; a blur, a hide
        // or a different owner retires it under the barrier's rules.
        let mut queue = Vec::new();
        if self.input_router.has_held_input() && !hidden {
            let current = self.input_router.observe(self.host_input_owner());
            let blur = incoming
                .iter()
                .any(|event| matches!(event, egui::Event::WindowFocused(false)));
            let held = std::mem::take(&mut self.input_router.held_input);
            if blur || self.input_router.held_for != Some(current) {
                log::info!(
                    "input_router: delivery={delivery} retired {} held event(s): their owner is gone",
                    held.len()
                );
                for event in held {
                    self.retire_event(event, &mut out);
                }
            } else {
                queue.extend(held);
            }
            self.input_router.held_for = None;
        }
        queue.extend(incoming);

        let mut barrier = false;
        let mut text_claimed = false;
        let mut egui_run = false;
        let mut events = queue.into_iter().enumerate();
        while let Some((ordinal, event)) = events.next() {
            if matches!(event, egui::Event::WindowFocused(false)) {
                self.input_router.retire_for_blur();
                self.input_router.wheel = None;
            }
            if !is_actionable(&event) {
                out.push((event, None));
                continue;
            }
            if hidden {
                // Nothing is drawn, so nobody can act on it. Edges still
                // retire, and egui still folds releases of what it holds.
                self.retire_event(event, &mut out);
                continue;
            }
            let owner = self.input_router.observe(self.host_input_owner());
            let unpresented_overlay = matches!(owner.owner, InputOwner::Overlay(_))
                && !self.input_router.presented(owner);
            let hold_for_field = unpresented_overlay && is_field_input(&event) && !barrier;
            if (egui_run && is_decisive(&event)) || hold_for_field {
                // The run-splitting rule: this event, and the rest, wait for
                // the next pass, held for the owner that holds input now.
                let mut held = vec![event];
                for (_, rest) in events.by_ref() {
                    if is_actionable(&rest) {
                        held.push(rest);
                    } else {
                        out.push((rest, None));
                    }
                }
                log::info!(
                    "input_router: delivery={delivery} split at ordinal={ordinal} held={} reason={}",
                    held.len(),
                    if hold_for_field { "owner_not_drawn" } else { "egui_run" }
                );
                self.input_router.held_input = held;
                self.input_router.held_for = Some(owner);
                ctx.request_repaint();
                break;
            }
            if let egui::Event::PointerMoved(pos) | egui::Event::PointerButton { pos, .. } = &event
            {
                self.input_router.last_pointer = Some(*pos);
            }
            let mut explicit = false;
            let routed = match event {
                egui::Event::Key { .. } => self.route_key(
                    ctx,
                    owner,
                    event,
                    os_focused,
                    barrier,
                    &mut text_claimed,
                    &mut explicit,
                ),
                egui::Event::Text(_) if barrier || text_claimed => None,
                egui::Event::Paste(_)
                | egui::Event::Copy
                | egui::Event::Cut
                | egui::Event::Ime(_)
                | egui::Event::Text(_)
                    if barrier =>
                {
                    None
                }
                egui::Event::Text(_)
                | egui::Event::Paste(_)
                | egui::Event::Copy
                | egui::Event::Cut
                | egui::Event::Ime(_) => {
                    let d = self.dispatch_owner_input(ctx, owner, event.clone(), os_focused);
                    Some((d, event))
                }
                egui::Event::PointerButton { .. } => {
                    self.route_pointer_button(ctx, owner, event, modifiers, barrier, &mut out)
                }
                egui::Event::MouseWheel { phase, .. } => {
                    self.route_wheel(owner, event, phase, barrier, unpresented_overlay)
                }
                egui::Event::PointerGone => Some((Disposition::Egui, event)),
                other => {
                    // Pointer motion, zoom, rotate, touch, accessibility.
                    if barrier || unpresented_overlay {
                        None
                    } else {
                        Some((Disposition::Egui, other))
                    }
                }
            };
            let Some((disposition, routed_event)) = routed else {
                self.note_owner_change(delivery, ordinal, "consumed", owner, false, &mut barrier);
                continue;
            };
            // A pane's terminal under the pointer sees pointer input in its
            // own ordered stream too (selection, mouse reporting, wheel).
            // (Button edges were placed by `route_pointer_button`, which
            // keeps a gesture in the terminal that took its press.)
            if matches!(disposition, Disposition::Egui)
                && !matches!(owner.owner, InputOwner::Overlay(_))
                && !matches!(routed_event, egui::Event::PointerButton { .. })
            {
                if let Some(pane) = self.pointer_terminal(&routed_event) {
                    self.input_router
                        .push_terminal(pane, routed_event.clone(), modifiers);
                }
            }
            match disposition {
                Disposition::Consumed => {}
                Disposition::Egui => {
                    if changes_egui_state(&routed_event) {
                        egui_run = true;
                    }
                    // A press on a pane belongs to the pane it focused.
                    let folded_for = if matches!(routed_event, egui::Event::PointerButton { .. }) {
                        self.input_router.observe(self.host_input_owner())
                    } else {
                        owner
                    };
                    out.push((routed_event.clone(), Some(folded_for)));
                }
                Disposition::Terminal(pane) => {
                    self.input_router
                        .push_terminal(pane, routed_event.clone(), modifiers);
                }
            }
            self.note_owner_change(
                delivery,
                ordinal,
                event_kind(&routed_event),
                owner,
                explicit,
                &mut barrier,
            );
        }

        let final_owner = self.input_router.observe(self.host_input_owner());
        self.input_router.egui_owner = Some(final_owner.owner);
        // A gesture whose owner no longer holds egui input ends here,
        // off-screen, so it cannot drag or click behind the new owner.
        let mut cancels = Vec::new();
        for (index, slot) in self.input_router.held_buttons.iter_mut().enumerate() {
            let Some(held) = slot else { continue };
            if held.path == EdgePath::Egui && held.owner != final_owner {
                let button = POINTER_BUTTONS[index];
                cancels.push(egui::Event::PointerButton {
                    pos: CANCEL_POS,
                    button,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                });
                // Its physical release is retired when it arrives.
                held.path = EdgePath::Retired;
                log::info!(
                    "input_router: cancelled held pointer {button:?} delivery={delivery} — its owner lost input to {:?}",
                    final_owner.owner
                );
            }
        }
        if !cancels.is_empty() {
            cancels.insert(0, egui::Event::PointerMoved(CANCEL_POS));
        }
        raw.events = cancels
            .into_iter()
            .chain(out.into_iter().filter_map(|(event, owner)| match owner {
                None => Some(event),
                Some(owner) if owner == final_owner => Some(event),
                // Releases of downs egui folded keep its key state balanced.
                Some(_) if matches!(event, egui::Event::Key { pressed: false, .. }) => Some(event),
                Some(_) => None,
            }))
            .collect();
        if barrier || hidden || matches!(final_owner.owner, InputOwner::Overlay(_)) {
            // A modal owns file drops: no pane under it may take the paths.
            // The modal itself reads them through `ctx` when it accepts drops.
            if barrier || hidden {
                raw.hovered_files.clear();
                raw.dropped_files.clear();
            }
        }
    }

    /// An event retired without an owner: a key release finishes its edge
    /// record (egui folds it when it folded the press); a pointer release
    /// that egui holds becomes an off-screen cancel; everything else drops.
    fn retire_event(
        &mut self,
        event: egui::Event,
        out: &mut Vec<(egui::Event, Option<OwnerInstance>)>,
    ) {
        match event {
            egui::Event::Key {
                key,
                physical_key,
                pressed: false,
                ..
            } => {
                let held = self
                    .input_router
                    .held_keys
                    .remove(&physical_key.unwrap_or(key));
                if held.is_some_and(|held| held.path == EdgePath::Egui) {
                    out.push((event, None));
                }
            }
            egui::Event::PointerButton {
                button,
                pressed: false,
                modifiers,
                ..
            } => {
                let held = self.input_router.held_buttons[button as usize].take();
                if held.is_some_and(|held| held.path == EdgePath::Egui) {
                    out.push((egui::Event::PointerMoved(CANCEL_POS), None));
                    out.push((
                        egui::Event::PointerButton {
                            pos: CANCEL_POS,
                            button,
                            pressed: false,
                            modifiers,
                        },
                        None,
                    ));
                }
            }
            egui::Event::Key { .. } => {
                // A press with nobody to act on it: its release is retired.
                if let egui::Event::Key {
                    key, physical_key, ..
                } = &event
                {
                    let id = physical_key.unwrap_or(*key);
                    let owner = self.input_router.observe(self.host_input_owner());
                    self.input_router
                        .held_keys
                        .insert(id, HeldEdge::new(owner, EdgePath::Retired));
                }
            }
            egui::Event::PointerButton { button, .. } => {
                let owner = self.input_router.observe(self.host_input_owner());
                self.input_router.held_buttons[button as usize] =
                    Some(HeldEdge::new(owner, EdgePath::Retired));
            }
            _ => {}
        }
    }

    /// Route one key edge at its ordinal. Returns the disposition and the
    /// event as routed (a close chord under a modal becomes its Escape).
    #[allow(clippy::too_many_arguments)]
    fn route_key(
        &mut self,
        ctx: &egui::Context,
        owner: OwnerInstance,
        event: egui::Event,
        os_focused: bool,
        barrier: bool,
        text_claimed: &mut bool,
        explicit: &mut bool,
    ) -> Option<(Disposition, egui::Event)> {
        let egui::Event::Key {
            key,
            physical_key,
            pressed,
            modifiers,
            ..
        } = event
        else {
            return None;
        };
        let id = physical_key.unwrap_or(key);
        if !pressed {
            *text_claimed = false;
            return match self.input_router.held_keys.remove(&id) {
                // The press's owner still holds input: the release follows
                // the press's path.
                Some(held)
                    if matches!(
                        held.path,
                        EdgePath::Owner | EdgePath::Egui | EdgePath::Terminal
                    ) && !barrier
                        && held.owner == owner =>
                {
                    match held.path {
                        EdgePath::Terminal => Some((Disposition::Terminal(held.terminal?), event)),
                        _ => Some((
                            self.dispatch_owner_input(ctx, owner, event.clone(), os_focused),
                            event,
                        )),
                    }
                }
                // egui folded the down; it must fold the up to keep its key
                // state balanced. No owner acts on it.
                Some(held) if held.path == EdgePath::Egui => Some((Disposition::Egui, event)),
                Some(_) => None,
                None if barrier => None,
                None => Some((
                    self.dispatch_owner_input(ctx, owner, event.clone(), os_focused),
                    event,
                )),
            };
        }
        let held = self
            .input_router
            .held_keys
            .get(&id)
            .copied()
            .filter(|held| !held.blurred);
        let mut event = event;
        if let egui::Event::Key { repeat, .. } = &mut event {
            // egui-winit reports every press as `repeat: false`; the router's
            // own down set decides.
            *repeat = held.is_some();
        }
        match held {
            Some(held) if held.path == EdgePath::Retired || held.owner != owner || barrier => {
                // Retired, or its owner is gone: repeats and their text go
                // with the press.
                if let Some(held) = self.input_router.held_keys.get_mut(&id) {
                    held.path = EdgePath::Retired;
                }
                *text_claimed = true;
                None
            }
            Some(_) => {
                // A repeat goes where its press went, under the same binding
                // and handler rules.
                let (d, by_host, routed) =
                    self.dispatch_key_press(ctx, owner, event, modifiers, os_focused);
                *text_claimed = matches!(d, Disposition::Consumed)
                    && (by_host || matches!(owner.owner, InputOwner::Overlay(_)));
                Some((d, routed))
            }
            None if barrier => {
                self.input_router
                    .held_keys
                    .insert(id, HeldEdge::new(owner, EdgePath::Retired));
                *text_claimed = true;
                None
            }
            None => {
                let (mut d, by_host, routed) =
                    self.dispatch_key_press(ctx, owner, event, modifiers, os_focused);
                if id == egui::Key::Tab && matches!(d, Disposition::Egui) {
                    // A focus trap's order is the one Tab order: the reconciler
                    // steps it at frame end, and egui's own traversal never
                    // sees this Tab once a trap owns input.
                    self.input_router.trap_tab = Some(modifiers.shift);
                    if crate::app::input_owner::focus_trap_active(ctx) {
                        d = Disposition::Consumed;
                    }
                }
                *explicit = by_host;
                let path = match d {
                    Disposition::Consumed if by_host => EdgePath::Claimed,
                    Disposition::Consumed => EdgePath::Owner,
                    Disposition::Egui => EdgePath::Egui,
                    Disposition::Terminal(_) => EdgePath::Terminal,
                };
                let mut record = HeldEdge::new(owner, path);
                if let Disposition::Terminal(pane) = d {
                    record.terminal = Some(pane);
                }
                self.input_router.held_keys.insert(id, record);
                // The text a consumed key types is consumed with it, except
                // in a pane: a pane's plugin owns its key and its text alike.
                *text_claimed = path == EdgePath::Claimed
                    || (path == EdgePath::Owner && matches!(owner.owner, InputOwner::Overlay(_)));
                match d {
                    Disposition::Consumed => None,
                    d => Some((d, routed)),
                }
            }
        }
    }

    /// Route one pointer button edge at its ordinal.
    fn route_pointer_button(
        &mut self,
        ctx: &egui::Context,
        owner: OwnerInstance,
        event: egui::Event,
        modifiers: egui::Modifiers,
        barrier: bool,
        out: &mut Vec<(egui::Event, Option<OwnerInstance>)>,
    ) -> Option<(Disposition, egui::Event)> {
        let egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: press_modifiers,
        } = event
        else {
            return None;
        };
        let slot = button as usize;
        if pressed {
            if barrier
                || (matches!(owner.owner, InputOwner::Overlay(_))
                    && !self.input_router.presented(owner))
            {
                // Behind the barrier, or a modal not yet drawn (it cannot
                // hit-test): declined, and its release is retired with it.
                self.input_router.held_buttons[slot] =
                    Some(HeldEdge::new(owner, EdgePath::Retired));
                return None;
            }
            let owner = if matches!(owner.owner, InputOwner::Overlay(_)) {
                owner
            } else {
                // The press's own modifiers, else the delivery's (synthetic
                // clicks carry none).
                let press_modifiers = if press_modifiers.is_none() {
                    modifiers
                } else {
                    press_modifiers
                };
                self.route_pane_press(ctx, pos, press_modifiers);
                self.input_router.observe(self.host_input_owner())
            };
            let mut record = HeldEdge::new(owner, EdgePath::Egui);
            if !matches!(owner.owner, InputOwner::Overlay(_)) {
                record.terminal = self.pointer_terminal(&event);
            }
            self.input_router.held_buttons[slot] = Some(record);
            if let Some(pane) = record.terminal {
                self.input_router
                    .push_terminal(pane, event.clone(), modifiers);
            }
            return Some((Disposition::Egui, event));
        }
        match self.input_router.held_buttons[slot].take() {
            Some(held) if held.path == EdgePath::Egui && held.owner == owner && !barrier => {
                if let Some(pane) = held.terminal {
                    // The release ends the gesture in the terminal that took
                    // the press, wherever the pointer is now.
                    self.input_router
                        .push_terminal(pane, event.clone(), modifiers);
                }
                Some((Disposition::Egui, event))
            }
            Some(held) if held.path == EdgePath::Egui => {
                // Its owner lost egui input: cancel, never click. egui still
                // gets a release, so no button stays down.
                self.retire_event_release(button, out);
                None
            }
            Some(_) => None,
            None if barrier => None,
            None => Some((Disposition::Egui, event)),
        }
    }

    fn retire_event_release(
        &mut self,
        button: egui::PointerButton,
        out: &mut Vec<(egui::Event, Option<OwnerInstance>)>,
    ) {
        out.push((egui::Event::PointerMoved(CANCEL_POS), None));
        out.push((
            egui::Event::PointerButton {
                pos: CANCEL_POS,
                button,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            },
            None,
        ));
    }

    /// Route a wheel event: a gesture (`Start` .. `End`) keeps the owner it
    /// began under until it ends; a continuation whose owner is gone is
    /// retired.
    fn route_wheel(
        &mut self,
        owner: OwnerInstance,
        event: egui::Event,
        phase: egui::TouchPhase,
        barrier: bool,
        unpresented_overlay: bool,
    ) -> Option<(Disposition, egui::Event)> {
        let gesture = match phase {
            egui::TouchPhase::Start => {
                self.input_router.wheel = None;
                let terminal = if matches!(owner.owner, InputOwner::Overlay(_)) {
                    None
                } else {
                    self.pointer_terminal(&event)
                };
                self.input_router.wheel = Some(WheelGesture { owner, terminal });
                self.input_router.wheel
            }
            egui::TouchPhase::Move => self.input_router.wheel,
            egui::TouchPhase::End | egui::TouchPhase::Cancel => self.input_router.wheel.take(),
        };
        if barrier || unpresented_overlay {
            if let Some(wheel) = self.input_router.wheel.as_mut() {
                wheel.owner.epoch = u64::MAX; // retired with its owner
            }
            return None;
        }
        match gesture {
            Some(wheel) if wheel.owner != owner => {
                if let Some(active) = self.input_router.wheel.as_mut() {
                    active.owner.epoch = u64::MAX;
                }
                None
            }
            _ => Some((Disposition::Egui, event)),
        }
    }

    /// The terminal pane a pointer event is over, by the last presented
    /// layout (or the terminal holding the primary button, during a drag).
    fn pointer_terminal(&self, event: &egui::Event) -> Option<PaneId> {
        if matches!(event, egui::Event::MouseWheel { .. }) {
            if let Some(wheel) = self.input_router.wheel {
                // A wheel gesture stays with the terminal it began over.
                return wheel.terminal;
            }
        }
        if matches!(event, egui::Event::PointerMoved(_)) {
            if let Some(held) =
                self.input_router.held_buttons[egui::PointerButton::Primary as usize]
            {
                if held.path == EdgePath::Egui {
                    return held.terminal;
                }
            }
        }
        let pos = event_pos(event).or(self.input_router.last_pointer)?;
        let window_id = self.windows[self.active_window].window_id;
        self.input_router
            .presented_panes
            .iter()
            .find(|p| p.window_id == window_id && p.hit_rect.contains(pos))
            .map(|p| p.pane)
            .filter(|pane| self.pane_is_terminal(*pane))
    }

    /// Trace and classify an owner change committed by the event at `ordinal`.
    /// A retired overlay installs the dismissal barrier unless the change was
    /// an explicit transfer to a newly opened owner.
    fn note_owner_change(
        &mut self,
        delivery: u64,
        ordinal: usize,
        source: &str,
        before: OwnerInstance,
        explicit: bool,
        barrier: &mut bool,
    ) {
        let after = self.input_router.observe(self.host_input_owner());
        if after == before {
            return;
        }
        let retired = matches!(before.owner, InputOwner::Overlay(_))
            && !(explicit && matches!(after.owner, InputOwner::Overlay(_)));
        let transition = if retired && DISMISSAL_BARRIER && !*barrier {
            *barrier = true;
            "dismissal_barrier"
        } else if retired {
            "dismissal"
        } else {
            "transfer"
        };
        log::info!(
            "input_router: owner change delivery={delivery} ordinal={ordinal} source={source} from={:?} to={:?} transition={transition}",
            before.owner,
            after.owner
        );
    }

    /// A press with a pane-level owner: hit-test the last presented layout and
    /// move focus to the pressed pane at this ordinal.
    fn route_pane_press(
        &mut self,
        ctx: &egui::Context,
        pos: egui::Pos2,
        modifiers: egui::Modifiers,
    ) {
        let active = self.active_window;
        let window_id = self.windows[active].window_id;
        let layer_at = ctx
            .layer_id_at(pos)
            .filter(|layer| !crate::ui::overlay::stale_modal_layer(ctx, *layer));
        if let Some((parent, rect, layer)) = self.input_router.conjoined_parent_hit {
            if rect.contains(pos) && layer_at.is_none_or(|l| l == layer) {
                self.conjoined_parent_focus = Some(parent);
                return;
            }
        }
        let Some(hit) = self.input_router.presented_panes.iter().copied().find(|p| {
            p.window_id == window_id
                && p.hit_rect.contains(pos)
                && layer_at.is_none_or(|layer| layer == p.layer)
        }) else {
            return;
        };
        let win = &mut self.windows[active];
        if !matches!(win.tree.tiles.get(hit.tile), Some(egui_tiles::Tile::Pane(p)) if *p == hit.pane)
        {
            return;
        }
        let prior_focus = win.focused_pane;
        let prior_pane = match prior_focus.and_then(|tile| win.tree.tiles.get(tile)) {
            Some(egui_tiles::Tile::Pane(pane)) => Some(*pane),
            _ => None,
        };
        win.navigate_to(hit.tile);
        self.conjoined_parent_focus = None;
        // A click replaces any multi-pane selection with the clicked pane
        // alone (stint 1340); Cmd+Shift+click adds it to the selection
        // instead, like Cmd+Shift+H/J/K/L (stint 1458).
        match prior_pane {
            Some(start) if modifiers.command && modifiers.shift => {
                log::info!(
                    "pane_selection: toggled by cmd+shift click window={window_id} pane={}",
                    hit.pane
                );
                self.host.toggle_pane_selection(window_id, start, hit.pane);
            }
            _ => {
                if self.host.clear_pane_selection(window_id).is_some() {
                    log::info!("pane_selection: cleared by pane click window={window_id}");
                }
            }
        }
        self.input_router.pane_press = Some(PanePress {
            window_id,
            tile: hit.tile,
            prior_focus,
        });
    }

    /// Dispatch a fresh key press to `owner`. Returns its disposition and
    /// whether a host binding claimed it (an explicit host action).
    fn dispatch_key_press(
        &mut self,
        ctx: &egui::Context,
        owner: OwnerInstance,
        mut event: egui::Event,
        modifiers: egui::Modifiers,
        os_focused: bool,
    ) -> (Disposition, bool, egui::Event) {
        use crate::host::keys::{self, BindingPolicy};
        match owner.owner {
            InputOwner::Overlay(crate::app::input_owner::OverlaySurface::Layer(kind)) => {
                // A close chord belongs to the top modal, never the pane under
                // it: it is that modal's Escape, with its cancel vetoes.
                let close = self.key_bindings.close_pane;
                if let egui::Event::Key {
                    key,
                    physical_key,
                    modifiers: m,
                    ..
                } = &mut event
                {
                    if *key == close.1 && m.matches_logically(close.0) {
                        *key = egui::Key::Escape;
                        *physical_key = None;
                        *m = egui::Modifiers::NONE;
                    }
                }
                // Noted before the owner runs: a modal that claims every key
                // (unsaved changes) still steps its trap on Tab.
                if matches!(
                    event,
                    egui::Event::Key {
                        key: egui::Key::Tab,
                        ..
                    }
                ) {
                    self.input_router.trap_tab = Some(modifiers.shift);
                }
                let qn = self.key_bindings.open_quick_note;
                if keys::event_matches_exact(&event, qn.0, qn.1) && self.is_quick_note_preemptable()
                {
                    self.dismiss_preemptable_modal();
                    self.open_quick_note_modal();
                    self.sync_overlay_layers();
                    log::info!("quick_note: opened by preempting non-critical modal");
                    return (Disposition::Consumed, true, event);
                }
                let mut one = PlexiInput::synthetic(vec![event.clone()], modifiers);
                kind.owner().handle_key(self, ctx, &mut one);
                self.sync_overlay_layers();
                if one.is_empty() {
                    return (Disposition::Consumed, false, event);
                }
                if crate::ui::dialog::presented_dialog_claims(ctx, &event) {
                    return (Disposition::Egui, false, event);
                }
                if let Some(action) = self.match_host_binding(&event, BindingPolicy::overlay()) {
                    self.run_host_action(action, ctx);
                    return (Disposition::Consumed, true, event);
                }
                (Disposition::Egui, false, event)
            }
            InputOwner::Overlay(_) => {
                if let Some(action) = self.match_host_binding(&event, BindingPolicy::overlay()) {
                    self.run_host_action(action, ctx);
                    return (Disposition::Consumed, true, event);
                }
                (
                    self.dispatch_owner_input(ctx, owner, event.clone(), os_focused),
                    false,
                    event,
                )
            }
            InputOwner::Pane(_) | InputOwner::Nobody | InputOwner::OsUnfocused => {
                if egui::Popup::is_any_open(ctx)
                    && keys::event_matches_exact(&event, egui::Modifiers::NONE, egui::Key::Escape)
                {
                    egui::Popup::close_all(ctx);
                    log::info!(
                        "input_router: Escape dismissed native popup before pane key dispatch"
                    );
                    return (Disposition::Consumed, false, event);
                }
                let (_, keyboard_capture_active) = self.focused_view_capture_state();
                if let Some(action) = self
                    .match_host_binding(&event, BindingPolicy::pane(false, keyboard_capture_active))
                {
                    self.run_host_action(action, ctx);
                    return (Disposition::Consumed, true, event);
                }
                // A focused text field owns every key it is given, Escape
                // included (it leaves the field); read before dispatch, which
                // may release the field.
                let field_owns = matches!(owner.owner, InputOwner::Pane(pane_id)
                    if crate::app::input_owner::focused_pane_text_surface(ctx, pane_id));
                let disposition = self.dispatch_owner_input(ctx, owner, event.clone(), os_focused);
                if !matches!(disposition, Disposition::Consumed) && !field_owns {
                    // ViewActive bindings (CloseView on Escape) come after the
                    // pane: a plugin or focused field owns bare Escape first.
                    let (view_active, keyboard_capture_active) = self.focused_view_capture_state();
                    if view_active {
                        if let Some(action) = self.match_host_binding(
                            &event,
                            BindingPolicy::pane(true, keyboard_capture_active),
                        ) {
                            self.run_host_action(action, ctx);
                            return (Disposition::Consumed, true, event);
                        }
                    }
                }
                (disposition, false, event)
            }
        }
    }

    /// Hand one event (no host bindings) to `owner`'s structured handler.
    /// Whatever it leaves goes to its egui widgets, or to the terminal stream.
    fn dispatch_owner_input(
        &mut self,
        ctx: &egui::Context,
        owner: OwnerInstance,
        event: egui::Event,
        os_focused: bool,
    ) -> Disposition {
        use crate::app::input_owner::OverlaySurface;
        let modifiers = match &event {
            egui::Event::Key { modifiers, .. } => *modifiers,
            _ => egui::Modifiers::NONE,
        };
        let mut one = PlexiInput::synthetic(vec![event], modifiers);
        match owner.owner {
            InputOwner::Overlay(OverlaySurface::Layer(kind)) => {
                kind.owner().handle_key(self, ctx, &mut one);
                self.sync_overlay_layers();
            }
            InputOwner::Overlay(OverlaySurface::DecisionOverlay) => {
                self.handle_decision_overlay_input(&mut one);
            }
            InputOwner::Overlay(OverlaySurface::AttentionQueue) => {
                self.handle_attention_queue_list_input(&mut one);
            }
            InputOwner::Overlay(OverlaySurface::TreeAtlas) => {
                self.handle_zoom_surface_input(&mut one);
            }
            // Input injected while the window is blurred still reaches the
            // owner's terminal stream, as native input does when focused.
            InputOwner::Pane(pane_id) if self.pane_is_terminal(pane_id) => {
                return Disposition::Terminal(pane_id);
            }
            InputOwner::Pane(_) if os_focused => {
                self.dispatch_plugin_key_events(ctx, &mut one);
            }
            InputOwner::Pane(_) | InputOwner::Nobody | InputOwner::OsUnfocused => {}
        }
        if one.is_empty() {
            Disposition::Consumed
        } else {
            Disposition::Egui
        }
    }

    /// Run a host binding's action at its ordinal, then commit any overlay it
    /// opened or closed so the next event sees the new owner.
    fn run_host_action(&mut self, action: crate::host::keys::Action, ctx: &egui::Context) {
        use crate::host::keys::Action;
        let scale = matches!(
            action,
            Action::IncreasePaneFontSize
                | Action::DecreasePaneFontSize
                | Action::ResetPaneFontSize
                | Action::UiScaleUp
                | Action::UiScaleDown
                | Action::UiScaleReset
        );
        if scale {
            if self.input_router.scale_actions.contains(&action) {
                return;
            }
            self.input_router.scale_actions.push(action.clone());
        }
        self.dispatch_host_action(action, ctx);
        self.sync_overlay_layers();
    }

    fn pane_is_terminal(&self, pane_id: PaneId) -> bool {
        self.windows
            .iter()
            .find_map(|w| w.panes.get(&pane_id))
            .is_some_and(|pane| pane.as_terminal().is_some())
    }

    /// Match one key press against the host binding table under `policy`.
    fn match_host_binding(
        &mut self,
        event: &egui::Event,
        policy: crate::host::keys::BindingPolicy,
    ) -> Option<crate::host::keys::Action> {
        use crate::host::keys::{self, Action, BindingContext, BindingEntry};
        let skip_toggle_shortcuts = !policy.overlay_open
            && !self.show_shortcuts
            && self.focused_builtin_editor_captures_shortcuts();
        let policy = crate::host::keys::BindingPolicy {
            shortcuts_overlay_open: self.show_shortcuts,
            ..policy
        };
        let table = &self.binding_table;
        let mut action = keys::match_binding(event, table, policy)
            .filter(|action| !(skip_toggle_shortcuts && *action == Action::ToggleShortcuts));
        if action.is_none() && !policy.overlay_open {
            // Context-dependent pane chords, matched with the same policy.
            let conjoined_tools = self.visual_owner_pane().is_some_and(|parent| {
                self.conjoined_contexts.iter().any(|(context, id)| {
                    *id == parent
                        && self
                            .windows
                            .iter()
                            .any(|window| window.context_id == *context && !window.panes.is_empty())
                })
            });
            let selection = self
                .host
                .pane_selection(self.windows[self.active_window].window_id)
                .is_some();
            let mut extra = Vec::new();
            if conjoined_tools {
                extra.push(BindingEntry {
                    modifiers: egui::Modifiers::COMMAND,
                    key: egui::Key::ArrowDown,
                    exact: true,
                    no_repeat: true,
                    context: BindingContext::Normal,
                    action: Action::ToggleZoom,
                });
            }
            if selection {
                extra.push(BindingEntry {
                    modifiers: egui::Modifiers::NONE,
                    key: egui::Key::Escape,
                    exact: true,
                    no_repeat: true,
                    context: BindingContext::Normal,
                    action: Action::CancelPaneSelection,
                });
            }
            action = keys::match_binding(event, &extra, policy);
        }
        action
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
    fn consume_key_claims_exactly_one_matching_event() {
        let mut input = PlexiInput::synthetic(
            vec![key_event(egui::Key::A, egui::Modifiers::COMMAND)],
            egui::Modifiers::COMMAND,
        );
        assert!(input.consume_key(egui::Modifiers::COMMAND, egui::Key::A));
        assert!(!input.consume_key(egui::Modifiers::COMMAND, egui::Key::A));
        assert!(input.is_empty());
    }

    #[test]
    fn consume_key_exact_matches_native_command() {
        let native = egui::Modifiers {
            command: true,
            mac_cmd: cfg!(target_os = "macos"),
            ctrl: !cfg!(target_os = "macos"),
            ..egui::Modifiers::NONE
        };
        let mut input = PlexiInput::synthetic(vec![key_event(egui::Key::J, native)], native);
        assert!(input.consume_key_exact(egui::Modifiers::COMMAND, egui::Key::J));
        let with_alt = egui::Modifiers {
            alt: true,
            ..native
        };
        let mut input = PlexiInput::synthetic(vec![key_event(egui::Key::P, with_alt)], with_alt);
        assert!(
            !input.consume_key_exact(egui::Modifiers::COMMAND, egui::Key::P),
            "Cmd+Option+P is not Cmd+P"
        );
    }

    #[test]
    fn observe_advances_the_epoch_only_on_owner_change() {
        let mut router = InputRouter::default();
        let a = router.observe(InputOwner::Pane(1));
        assert_eq!(router.observe(InputOwner::Pane(1)), a);
        let b = router.observe(InputOwner::Nobody);
        assert_ne!(a, b);
        let a2 = router.observe(InputOwner::Pane(1));
        assert_ne!(a, a2, "a reopened owner is a new instance");
    }

    #[test]
    fn key_pressed_matches_buffer_ignoring_modifiers() {
        let input = PlexiInput::synthetic(
            vec![key_event(egui::Key::A, egui::Modifiers::COMMAND)],
            egui::Modifiers::COMMAND,
        );
        assert!(input.key_pressed(egui::Key::A));
        assert!(!input.key_pressed(egui::Key::B));
    }
}
