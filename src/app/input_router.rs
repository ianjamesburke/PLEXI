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
    /// A structured handler or host binding claimed it.
    Claimed,
    /// The owner's egui widgets (left in `RawInput`).
    Egui,
    /// The focused terminal's ordered stream.
    Terminal,
}

#[derive(Clone, Copy, Debug)]
struct HeldEdge {
    owner: OwnerInstance,
    path: EdgePath,
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

/// A press the router assigned to a pane at its ordinal. Rendering applies
/// the click's selection and history effects from this, never from egui's
/// global pointer state.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PanePress {
    pub(crate) window_id: u64,
    pub(crate) tile: egui_tiles::TileId,
    /// Focus before the press moved it, for focus history and the click flash.
    pub(crate) prior_focus: Option<egui_tiles::TileId>,
    /// The press event's own modifiers (Cmd+Shift extends the selection).
    pub(crate) modifiers: egui::Modifiers,
}

/// What the router produced for one routed event.
enum Disposition {
    Consumed,
    Egui,
    Terminal(PaneId),
}

/// Router state that outlives a delivery.
#[derive(Default)]
pub(crate) struct InputRouter {
    delivery: u64,
    epoch: u64,
    current: Option<InputOwner>,
    held_keys: HashMap<egui::Key, HeldEdge>,
    held_buttons: [Option<HeldEdge>; egui::NUM_POINTER_BUTTONS],
    /// Epoch of the overlay instance most recently drawn in full.
    presented_epoch: Option<u64>,
    /// Pane hit areas from the last visible pass.
    presented_panes: Vec<PresentedPane>,
    /// The conjoined parent pane's presented rect, when one is shown.
    conjoined_parent_hit: Option<(PaneId, egui::Rect, egui::LayerId)>,
    /// Font and UI-scale actions already run in this delivery: one physical
    /// press can arrive as both `Equals` and `Plus`.
    scale_actions: Vec<crate::host::keys::Action>,
    /// This delivery's outputs, read by the pass that folds it.
    pub(crate) egui_owner: Option<InputOwner>,
    pub(crate) terminal_stream: Option<crate::render::terminal_pane::TerminalInput>,
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

    /// True while `key`'s press was claimed by a structured owner and its
    /// release has not arrived: the release will be retired, not delivered.
    #[cfg(test)]
    pub(crate) fn holds_claimed_key(&self, key: egui::Key) -> bool {
        self.held_keys
            .get(&key)
            .is_some_and(|held| held.path == EdgePath::Claimed)
    }

    fn presented(&self, instance: OwnerInstance) -> bool {
        self.presented_epoch == Some(instance.epoch)
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

impl PlexiApp {
    /// Route one native delivery (see the module docs). Called from
    /// `raw_input_hook` after queued pane IPC input has been appended, so
    /// injected and native events share one order.
    pub(crate) fn route_delivery(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.input_router.delivery += 1;
        let delivery = self.input_router.delivery;
        self.input_router.egui_owner = None;
        self.input_router.terminal_stream = None;
        self.input_router.trap_tab = None;
        self.input_router.pane_press = None;
        self.input_router.scale_actions.clear();
        let hidden = raw.viewport().visible() == Some(false);
        let os_focused = raw
            .viewport()
            .focused
            .unwrap_or_else(|| ctx.input(|i| i.viewport().focused.unwrap_or(true)));
        let modifiers = raw.modifiers;
        let mut out: Vec<(egui::Event, Option<OwnerInstance>)> = Vec::new();
        let mut terminal: Vec<egui::Event> = Vec::new();
        let mut terminal_pane = None;
        let mut barrier = false;
        let mut text_claimed = false;

        for (ordinal, mut event) in std::mem::take(&mut raw.events).into_iter().enumerate() {
            if matches!(event, egui::Event::WindowFocused(false)) {
                // egui clears its keys_down on blur; no release will follow.
                self.input_router.held_keys.clear();
            }
            if !is_actionable(&event) {
                out.push((event, None));
                continue;
            }
            let owner = self.input_router.observe(self.host_input_owner());
            if hidden {
                // Nothing is drawn, so nobody can act on it; retire edges only.
                if let egui::Event::Key {
                    key,
                    physical_key,
                    pressed: false,
                    ..
                } = &event
                {
                    self.input_router
                        .held_keys
                        .remove(&physical_key.unwrap_or(*key));
                }
                if let egui::Event::PointerButton {
                    button,
                    pressed: false,
                    ..
                } = &event
                {
                    self.input_router.held_buttons[*button as usize] = None;
                }
                continue;
            }
            let stack_before = self.focus_stack.len();
            let mut explicit = false;
            let disposition = match &mut event {
                egui::Event::Key {
                    key,
                    physical_key,
                    pressed,
                    repeat,
                    modifiers: key_modifiers,
                } => {
                    let id = physical_key.unwrap_or(*key);
                    if !*pressed {
                        match self.input_router.held_keys.remove(&id) {
                            // The press's owner still holds input: the release
                            // follows the press's path.
                            Some(held)
                                if held.path != EdgePath::Claimed
                                    && !barrier
                                    && held.owner == owner =>
                            {
                                self.dispatch_owner_input(ctx, owner, event.clone(), os_focused)
                            }
                            Some(held) if held.path == EdgePath::Egui => {
                                // egui folded the down; it must fold the up to
                                // keep its key state balanced. No owner acts.
                                out.push((event, Some(held.owner)));
                                continue;
                            }
                            Some(_) => Disposition::Consumed,
                            None if barrier => Disposition::Consumed,
                            None => self.dispatch_owner_input(ctx, owner, event.clone(), os_focused),
                        }
                    } else {
                        let held = self.input_router.held_keys.get(&id).copied();
                        *repeat = held.is_some();
                        let key_modifiers = *key_modifiers;
                        match held {
                            Some(held) if held.owner != owner || barrier => {
                                // The key's owner is gone: its repeats and
                                // their text are retired with it.
                                text_claimed = true;
                                Disposition::Consumed
                            }
                            Some(_) => {
                                // A repeat goes where its press went, with
                                // the same binding and handler rules.
                                let (d, _) = self.dispatch_key_press(
                                    ctx,
                                    owner,
                                    event.clone(),
                                    key_modifiers,
                                    os_focused,
                                );
                                text_claimed = matches!(d, Disposition::Consumed);
                                d
                            }
                            None if barrier => {
                                self.input_router.held_keys.insert(
                                    id,
                                    HeldEdge {
                                        owner,
                                        path: EdgePath::Claimed,
                                    },
                                );
                                text_claimed = true;
                                Disposition::Consumed
                            }
                            None => {
                                let (d, by_host) = self.dispatch_key_press(
                                    ctx,
                                    owner,
                                    event.clone(),
                                    key_modifiers,
                                    os_focused,
                                );
                                explicit = by_host;
                                let path = match d {
                                    Disposition::Consumed => EdgePath::Claimed,
                                    Disposition::Egui => EdgePath::Egui,
                                    Disposition::Terminal(_) => EdgePath::Terminal,
                                };
                                self.input_router
                                    .held_keys
                                    .insert(id, HeldEdge { owner, path });
                                text_claimed = path == EdgePath::Claimed;
                                d
                            }
                        }
                    }
                }
                egui::Event::Text(_) | egui::Event::Paste(_) | egui::Event::Ime(_)
                    if barrier || text_claimed =>
                {
                    Disposition::Consumed
                }
                egui::Event::Text(_)
                | egui::Event::Paste(_)
                | egui::Event::Copy
                | egui::Event::Cut
                | egui::Event::Ime(_) => {
                    if barrier {
                        Disposition::Consumed
                    } else {
                        self.dispatch_owner_input(ctx, owner, event.clone(), os_focused)
                    }
                }
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    modifiers: press_modifiers,
                } => {
                    if *pressed {
                        let (d, owner) = if barrier {
                            (Disposition::Consumed, owner)
                        } else if matches!(owner.owner, InputOwner::Overlay(_))
                            && !self.input_router.presented(owner)
                        {
                            // Before its first draw a modal cannot hit-test;
                            // it declines the press rather than leak it.
                            (Disposition::Consumed, owner)
                        } else if matches!(owner.owner, InputOwner::Overlay(_)) {
                            (Disposition::Egui, owner)
                        } else {
                            self.route_pane_press(ctx, *pos, *press_modifiers);
                            let owner = self.input_router.observe(self.host_input_owner());
                            (Disposition::Egui, owner)
                        };
                        let path = if matches!(d, Disposition::Egui) {
                            EdgePath::Egui
                        } else {
                            EdgePath::Claimed
                        };
                        self.input_router.held_buttons[*button as usize] =
                            Some(HeldEdge { owner, path });
                        match d {
                            Disposition::Egui => {
                                out.push((event, Some(owner)));
                                self.note_owner_change(delivery, ordinal, "pointer_button", owner, false, &mut barrier);
                                continue;
                            }
                            other => other,
                        }
                    } else {
                        match self.input_router.held_buttons[*button as usize].take() {
                            Some(held)
                                if held.path == EdgePath::Egui && held.owner == owner && !barrier =>
                            {
                                Disposition::Egui
                            }
                            Some(held) if held.path == EdgePath::Egui => {
                                // Its owner lost egui input: cancel, never click.
                                let button = *button;
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
                                continue;
                            }
                            Some(_) => Disposition::Consumed,
                            None if barrier => Disposition::Consumed,
                            None => Disposition::Egui,
                        }
                    }
                }
                egui::Event::PointerGone => Disposition::Egui,
                _ => {
                    if barrier
                        || (matches!(owner.owner, InputOwner::Overlay(_))
                            && !self.input_router.presented(owner))
                    {
                        Disposition::Consumed
                    } else {
                        Disposition::Egui
                    }
                }
            };
            match disposition {
                Disposition::Consumed => {}
                Disposition::Egui => out.push((event.clone(), Some(owner))),
                Disposition::Terminal(pane) => {
                    terminal_pane = Some(pane);
                    terminal.push(event.clone());
                }
            }
            let _ = stack_before;
            self.note_owner_change(delivery, ordinal, event_kind(&event), owner, explicit, &mut barrier);
        }

        let final_owner = self.input_router.observe(self.host_input_owner());
        self.input_router.egui_owner = Some(final_owner.owner);
        // A gesture whose owner no longer holds egui input ends here,
        // off-screen, so it cannot drag or click behind the new owner.
        let mut cancels = Vec::new();
        let mut cancelled = Vec::new();
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
                held.path = EdgePath::Claimed;
                cancelled.push(button);
            }
        }
        for button in cancelled {
            log::info!(
                "input_router: cancelled held pointer {button:?} delivery={delivery} — its owner lost input to {:?}",
                final_owner.owner
            );
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
        if matches!(final_owner.owner, InputOwner::Overlay(_)) || barrier {
            raw.hovered_files.clear();
            raw.dropped_files.clear();
        }
        if let Some(pane_id) = terminal_pane {
            self.input_router.terminal_stream = Some(crate::render::terminal_pane::TerminalInput {
                pane_id,
                keyboard_events: terminal,
                modifiers,
            });
        }
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
    fn route_pane_press(&mut self, ctx: &egui::Context, pos: egui::Pos2, modifiers: egui::Modifiers) {
        let active = self.active_window;
        let window_id = self.windows[active].window_id;
        let layer_at = ctx.layer_id_at(pos);
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
        win.navigate_to(hit.tile);
        self.conjoined_parent_focus = None;
        self.input_router.pane_press = Some(PanePress {
            window_id,
            tile: hit.tile,
            prior_focus,
            modifiers,
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
    ) -> (Disposition, bool) {
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
                let is_tab = matches!(event, egui::Event::Key { key: egui::Key::Tab, .. });
                if is_tab {
                    self.input_router.trap_tab = Some(modifiers.shift);
                }
                let qn = self.key_bindings.open_quick_note;
                if keys::event_matches_exact(&event, qn.0, qn.1) && self.is_quick_note_preemptable() {
                    self.dismiss_preemptable_modal();
                    self.open_quick_note_modal();
                    self.sync_overlay_layers();
                    log::info!("quick_note: opened by preempting non-critical modal");
                    return (Disposition::Consumed, true);
                }
                let mut one = PlexiInput::synthetic(vec![event.clone()], modifiers);
                kind.owner().handle_key(self, ctx, &mut one);
                self.sync_overlay_layers();
                if one.is_empty() {
                    return (Disposition::Consumed, false);
                }
                if let Some(action) = self.match_host_binding(&event, BindingPolicy::overlay()) {
                    self.run_host_action(action, ctx);
                    return (Disposition::Consumed, true);
                }
                if is_tab && crate::app::input_owner::focus_trap_active(ctx) {
                    // The trap's order is the one Tab order: egui's own
                    // traversal never sees this Tab (reconciled at frame end).
                    return (Disposition::Consumed, false);
                }
                (Disposition::Egui, false)
            }
            InputOwner::Overlay(_) => {
                if let Some(action) = self.match_host_binding(&event, BindingPolicy::overlay()) {
                    self.run_host_action(action, ctx);
                    return (Disposition::Consumed, true);
                }
                (self.dispatch_owner_input(ctx, owner, event, os_focused), false)
            }
            InputOwner::Pane(_) | InputOwner::Nobody | InputOwner::OsUnfocused => {
                if egui::Popup::is_any_open(ctx)
                    && keys::event_matches_exact(&event, egui::Modifiers::NONE, egui::Key::Escape)
                {
                    egui::Popup::close_all(ctx);
                    log::info!("input_router: Escape dismissed native popup before pane key dispatch");
                    return (Disposition::Consumed, false);
                }
                let (_, keyboard_capture_active) = self.focused_view_capture_state();
                if let Some(action) =
                    self.match_host_binding(&event, BindingPolicy::pane(false, keyboard_capture_active))
                {
                    self.run_host_action(action, ctx);
                    return (Disposition::Consumed, true);
                }
                let disposition = self.dispatch_owner_input(ctx, owner, event.clone(), os_focused);
                if !matches!(disposition, Disposition::Consumed) {
                    // ViewActive bindings (CloseView on Escape) come after the
                    // pane: a plugin or focused field owns bare Escape first.
                    let (view_active, keyboard_capture_active) = self.focused_view_capture_state();
                    if view_active {
                        if let Some(action) = self.match_host_binding(
                            &event,
                            BindingPolicy::pane(true, keyboard_capture_active),
                        ) {
                            self.run_host_action(action, ctx);
                            return (Disposition::Consumed, true);
                        }
                    }
                }
                (disposition, false)
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
