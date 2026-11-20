//! Host input router (stint 1659, slice 2): one ordered delivery in, one
//! disposition and one transition per event out.
//!
//! `PlexiApp::raw_input_hook` stamps every event at ingress (delivery,
//! ordinal, source, receipt time, modifiers) and hands the deliveries here
//! with a [`Snapshot`] of committed host state and the last presentation. The
//! router owns, in this slice, a focused terminal's keyboard and text input,
//! host bindings pressed there, and the Confirm Close dialog (keys and
//! pointer). Terminal pointer, selection, mouse reporting and search stay in
//! the `egui_term` widget on egui's path. The router has no executor:
//! it returns the events that stay on egui's legacy path, in order, and queues
//! ordered, target-bound [`RoutedCommand`]s that `App::logic` executes.
//!
//! Every other recipient (view panes, other overlays, chrome) is the legacy
//! path: the router hands those events to egui unchanged, as one explicit,
//! exclusive handoff. An event the router claims never reaches egui, and an
//! event it hands to egui never reaches a router sink.
//!
//! Ordering rules:
//! - A host action whose result can change authority suspends the delivery at
//!   its ordinal. Logic executes it and resolves it; the suffix resumes at the
//!   next hook against the new committed state.
//! - A dialog decision is a dismissal: the rest of that delivery is consumed
//!   (`barrier`), apart from lifecycle events and the releases of edges it
//!   already owns. A later delivery keeps its own boundary.
//! - After an event handed to egui that can change authority, events the
//!   router would claim wait for the next pass, so egui's effect is committed
//!   before they are resolved.
//!   A click that moves pane focus is such an event: keys after it in the
//!   same delivery reach the newly focused pane.
//! - Hide or blur retires dialog gestures and invalidates held suffixes.

pub(crate) mod binding;
pub(crate) mod dialog;
#[cfg(test)]
mod tests;

use crate::host::keys::{Action, BindingEntry};
use binding::BindingGates;
use dialog::{ActionSpec, DialogHit, DialogModel, DialogOutcome, DialogPresentation};
use std::collections::{HashMap, VecDeque};
use std::time::Instant;

/// One opening of a router-owned dialog. Allocated by the operation that
/// opens it, so a replacement or a reopening is a different instance even
/// when it looks the same.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct OpeningId(pub u64);

impl OpeningId {
    pub(crate) fn fresh() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Self(NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct DeliveryId(pub u64);

/// An event's immutable identity: its delivery and its place in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct EventId {
    pub delivery: DeliveryId,
    pub ordinal: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct CommandId(pub u64);

/// Where an event entered the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Source {
    /// From the OS window.
    Native,
    /// Queued by pane IPC (`plexi pane key/click`, the harness). Synthetic.
    Injected,
}

/// One stamped event. Stamps never change while the event is held.
#[derive(Clone, Debug)]
pub(crate) struct Received {
    pub id: EventId,
    pub source: Source,
    /// Monotonic receipt time; a held event keeps it.
    pub received_at: Instant,
    pub event: egui::Event,
}

impl Received {
    fn synthetic(&self) -> bool {
        self.source == Source::Injected
    }
}

/// A live terminal pane and its input endpoint incarnation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct PaneInstance {
    pub window_id: u64,
    pub pane: u64,
    /// `TerminalBackend::instance`: a replaced endpoint is a new instance.
    pub endpoint: u64,
}

/// Who owns keyboard input when no overlay does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KeyboardOwner {
    Terminal(PaneInstance),
    /// A view pane, or nobody: the legacy path.
    Legacy,
}

/// A router-owned dialog opening and its actions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DialogSnapshot {
    pub opening: OpeningId,
    pub actions: Vec<ActionSpec>,
    pub default_action: Option<String>,
}

/// Committed host state and the last presentation, read once per hook.
pub(crate) struct Snapshot<'a> {
    pub visible: bool,
    pub os_focused: bool,
    pub dialog: Option<DialogSnapshot>,
    /// A legacy overlay or popup owns input.
    pub legacy_overlay: bool,
    pub keyboard: KeyboardOwner,
    pub table: &'a [BindingEntry],
    pub gates: BindingGates,
    pub dialog_presentation: Option<DialogPresentation>,
}

/// The recipient of an event at its ordinal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Owner {
    Terminal(PaneInstance),
    Dialog(OpeningId),
    Legacy,
}

/// Where an event went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Disposition {
    Terminal(PaneInstance),
    HostAction(Action),
    Dialog(DialogOutcome),
    /// Captured by a dialog press that has not completed.
    DialogPointer,
    Egui,
    /// Consumed with no effect: a late or barred edge, associated text.
    Retired(&'static str),
}

/// What the event did to ownership.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Transition {
    Stay,
    /// The owner refused a change of authority and stays eligible.
    Veto,
    /// The owner was answered or dismissed: the rest of the delivery is barred.
    Dismiss,
    /// Suspended until logic resolves this command.
    Wait(CommandId),
}

/// One routing decision, kept for traces and tests.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RouteTrace {
    pub event: EventId,
    pub source: Source,
    pub owner: Owner,
    pub disposition: Disposition,
    pub transition: Transition,
}

/// An ordered, target-bound effect for `App::logic`.
#[derive(Clone, Debug)]
pub(crate) enum RoutedCommand {
    Terminal {
        target: PaneInstance,
        first: EventId,
        events: Vec<egui::Event>,
    },
    HostAction {
        id: CommandId,
        action: Action,
        /// The pane the action was admitted for; `None` under a dialog.
        target: Option<PaneInstance>,
        event: EventId,
    },
    DialogDecision {
        id: CommandId,
        opening: OpeningId,
        action: String,
        event: EventId,
    },
}

/// What logic reports for a suspending command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    Done,
    /// The target was gone or no longer eligible, or the action retired it
    /// (closed its pane): input bound to it retires.
    Failed,
}

/// Where a held key's later edges go.
#[derive(Clone, Debug, PartialEq, Eq)]
enum KeySink {
    Terminal(PaneInstance),
    Host { action: Action, no_repeat: bool },
    Dialog(OpeningId),
    Legacy,
    Retired,
}

/// Where a held pointer button's later edges go.
#[derive(Clone, Debug, PartialEq, Eq)]
enum ButtonSink {
    Dialog {
        opening: OpeningId,
        hit: DialogHit,
        synthetic: bool,
    },
    Legacy,
    Retired,
}

/// A delivery, or the suffix of one, waiting to be routed.
#[derive(Clone, Debug)]
struct Held {
    events: VecDeque<Received>,
    /// A dismissal earlier in this delivery bars its remaining input.
    barrier: bool,
}

/// What routing one held delivery left behind.
enum Step {
    Done,
    /// Stop: this suffix waits (for logic, or for the next pass).
    Hold(Held),
}

#[derive(Default)]
pub(crate) struct HostRouter {
    next_delivery: u64,
    next_command: u64,
    keys: HashMap<egui::Key, KeySink>,
    /// Keyed by `PointerButton as u8` (egui's enum is not `Hash`).
    buttons: HashMap<u8, ButtonSink>,
    dialog: Option<DialogModel>,
    held: VecDeque<Held>,
    waiting: Option<CommandId>,
    resolved: HashMap<CommandId, Outcome>,
    commands: Vec<RoutedCommand>,
    /// The previous event was a press claimed by the host or a dialog: a
    /// `Text` directly after it, in the same delivery, is that key's
    /// associated text.
    claimed_text: Option<DeliveryId>,
    traces: VecDeque<RouteTrace>,
}

const TRACE_CAPACITY: usize = 512;

/// What one hook produced for egui.
pub(crate) struct Routed {
    /// The legacy run, in received order.
    pub egui: Vec<Received>,
    /// Input is held for a later pass: schedule one.
    pub held: bool,
}

impl HostRouter {
    pub(crate) fn next_delivery(&mut self) -> DeliveryId {
        self.next_delivery += 1;
        DeliveryId(self.next_delivery)
    }

    fn next_command(&mut self) -> CommandId {
        self.next_command += 1;
        CommandId(self.next_command)
    }

    /// The commands queued since the last drain, in order.
    pub(crate) fn take_commands(&mut self) -> Vec<RoutedCommand> {
        std::mem::take(&mut self.commands)
    }

    /// Logic's answer for a suspending command.
    pub(crate) fn resolve(&mut self, id: CommandId, outcome: Outcome) {
        self.resolved.insert(id, outcome);
    }

    /// The Tab-focused action of `opening`, for paint to project.
    pub(crate) fn dialog_focus(&self, opening: OpeningId) -> Option<&str> {
        self.dialog
            .as_ref()
            .filter(|model| model.opening == opening)
            .and_then(DialogModel::focused_id)
    }

    #[cfg(test)]
    pub(crate) fn traces(&self) -> impl Iterator<Item = &RouteTrace> {
        self.traces.iter()
    }

    /// Route this hook's deliveries, oldest first after any held suffix.
    pub(crate) fn route(&mut self, snap: &Snapshot<'_>, deliveries: Vec<Vec<Received>>) -> Routed {
        self.sync_dialog(snap);
        if self.dialog.is_some() || snap.legacy_overlay {
            // A key held into a terminal before an overlay took input: its
            // late release belongs to nobody. The terminal's own hold is
            // ended by `release_unowned_terminal_holds`.
            for sink in self.keys.values_mut() {
                if matches!(sink, KeySink::Terminal(_)) {
                    *sink = KeySink::Retired;
                }
            }
        }
        let mut egui = Vec::new();
        if !snap.visible {
            // A hidden window draws nothing: nothing held may act later
            // against a surface the user cannot see.
            self.invalidate("hidden");
            for event in deliveries.into_iter().flatten() {
                if lifecycle(&event.event) {
                    egui.push(event);
                }
            }
            return Routed { egui, held: false };
        }
        for events in deliveries {
            if !events.is_empty() {
                self.held.push_back(Held {
                    events: events.into(),
                    barrier: false,
                });
            }
        }
        if let Some(waiting) = self.waiting {
            match self.resolved.remove(&waiting) {
                Some(outcome) => {
                    self.waiting = None;
                    if outcome == Outcome::Failed {
                        if let Some(front) = self.held.front_mut() {
                            log::info!(
                                "host_router: command {} failed; its suffix retires",
                                waiting.0
                            );
                            front.barrier = true;
                        }
                    }
                }
                None => {
                    log::warn!(
                        "host_router: command {} unresolved at hook; {} deliveries wait",
                        waiting.0,
                        self.held.len()
                    );
                    return Routed { egui, held: true };
                }
            }
        }
        let mut decisive_egui = false;
        while let Some(held) = self.held.pop_front() {
            match self.run(snap, held, &mut egui, &mut decisive_egui) {
                Step::Done => {}
                Step::Hold(rest) => {
                    self.held.push_front(rest);
                    break;
                }
            }
            if self.waiting.is_some() {
                break;
            }
        }
        Routed {
            egui,
            held: !self.held.is_empty(),
        }
    }

    /// Drop held input and retire terminal gestures (hide, blur).
    fn invalidate(&mut self, reason: &'static str) {
        if !self.held.is_empty() {
            log::info!(
                "host_router: {reason} invalidates {} held deliveries",
                self.held.len()
            );
            self.held.clear();
        }
        self.retire_gestures(reason);
    }

    fn sync_dialog(&mut self, snap: &Snapshot<'_>) {
        match &snap.dialog {
            Some(dialog) if self.dialog.as_ref().map(|m| m.opening) != Some(dialog.opening) => {
                log::info!(
                    "host_router: dialog opening {} owns input",
                    dialog.opening.0
                );
                self.dialog = Some(DialogModel::new(
                    dialog.opening,
                    dialog.actions.clone(),
                    dialog.default_action.clone(),
                ));
            }
            Some(_) => {}
            None => self.dialog = None,
        }
    }

    fn owner(&self, snap: &Snapshot<'_>, event: &Received) -> Owner {
        if let Some(model) = &self.dialog {
            return Owner::Dialog(model.opening);
        }
        if snap.legacy_overlay {
            return Owner::Legacy;
        }
        match snap.keyboard {
            // Native keys follow OS window focus; injected input targets its
            // pane under the existing pane IPC policy.
            KeyboardOwner::Terminal(target)
                if snap.os_focused || event.source == Source::Injected =>
            {
                Owner::Terminal(target)
            }
            _ => Owner::Legacy,
        }
    }

    /// Hide or blur: dialog gestures retire, and the OS drops key-ups
    /// across a blur (egui clears its own down set there too), so no held key
    /// survives it.
    fn retire_gestures(&mut self, reason: &'static str) {
        for sink in self.buttons.values_mut() {
            if matches!(sink, ButtonSink::Dialog { .. }) {
                log::info!("host_router: {reason} retires a dialog press");
                *sink = ButtonSink::Retired;
            }
        }
        self.keys.clear();
    }

    fn trace(
        &mut self,
        event: &Received,
        owner: Owner,
        disposition: Disposition,
        transition: Transition,
    ) {
        let loud = transition != Transition::Stay
            || matches!(
                disposition,
                Disposition::HostAction(_) | Disposition::Dialog(_) | Disposition::Retired(_)
            );
        // Never the payload: no text, paste contents or terminal bytes.
        let line = format!(
            "host_router: delivery={} ordinal={} source={:?} held_us={} owner={owner:?} disposition={} transition={transition:?}",
            event.id.delivery.0,
            event.id.ordinal,
            event.source,
            event.received_at.elapsed().as_micros(),
            disposition_label(&disposition)
        );
        if loud {
            log::info!("{line}");
        } else {
            log::debug!("{line}");
        }
        if self.traces.len() == TRACE_CAPACITY {
            self.traces.pop_front();
        }
        self.traces.push_back(RouteTrace {
            event: event.id,
            source: event.source,
            owner,
            disposition,
            transition,
        });
    }

    fn push_terminal(&mut self, target: PaneInstance, event: &Received) {
        let input = event.event.clone();
        if let Some(RoutedCommand::Terminal {
            target: last,
            events,
            ..
        }) = self.commands.last_mut()
        {
            if *last == target {
                events.push(input);
                return;
            }
        }
        self.commands.push(RoutedCommand::Terminal {
            target,
            first: event.id,
            events: vec![input],
        });
    }

    /// Whether the router, not egui, would take `event` at this ordinal.
    fn claims(&self, owner: Owner, event: &Received) -> bool {
        if lifecycle(&event.event) || matches!(event.event, egui::Event::PointerMoved(_)) {
            return false;
        }
        match owner {
            Owner::Dialog(_) => true,
            // Pointer input over a terminal stays on egui's path.
            Owner::Terminal(_) => !pointer(&event.event),
            Owner::Legacy => false,
        }
    }

    fn run(
        &mut self,
        snap: &Snapshot<'_>,
        mut held: Held,
        egui: &mut Vec<Received>,
        decisive_egui: &mut bool,
    ) -> Step {
        while let Some(event) = held.events.pop_front() {
            if matches!(event.event, egui::Event::WindowFocused(false)) {
                // A blur inside the delivery bars the rest of it too.
                self.retire_gestures("blur");
                held.barrier = true;
            }
            let claimed_text = self.claimed_text.take() == Some(event.id.delivery);
            if lifecycle(&event.event) {
                egui.push(event);
                continue;
            }
            let owner = self.owner(snap, &event);
            if held.barrier {
                self.barred(owner, event, egui);
                continue;
            }
            if *decisive_egui && self.claims(owner, &event) {
                // egui has not yet run an earlier event that may change who
                // owns this one.
                held.events.push_front(event);
                return Step::Hold(held);
            }
            let (disposition, transition) = self.dispatch(snap, owner, &event, claimed_text);
            match &disposition {
                Disposition::Egui => {
                    if actionable(&event.event) {
                        *decisive_egui = true;
                    }
                    egui.push(event.clone());
                }
                Disposition::Terminal(target) => self.push_terminal(*target, &event),
                _ => {}
            }
            self.trace(&event, owner, disposition, transition.clone());
            match transition {
                Transition::Dismiss => held.barrier = true,
                Transition::Wait(id) => {
                    self.waiting = Some(id);
                    if held.events.is_empty() {
                        return Step::Done;
                    }
                    return Step::Hold(held);
                }
                Transition::Stay | Transition::Veto => {}
            }
        }
        Step::Done
    }

    /// An event after a dismissal in the same delivery: releases of edges
    /// that egui folded go back to egui so its state balances; everything
    /// else actionable is consumed.
    fn barred(&mut self, owner: Owner, event: Received, egui: &mut Vec<Received>) {
        let disposition = match &event.event {
            egui::Event::PointerMoved(_) => {
                egui.push(event.clone());
                Disposition::Egui
            }
            egui::Event::Key {
                key,
                physical_key,
                pressed: false,
                ..
            } => match self.keys.remove(&physical_key.unwrap_or(*key)) {
                Some(KeySink::Legacy) => {
                    egui.push(event.clone());
                    Disposition::Egui
                }
                _ => Disposition::Retired("barrier"),
            },
            egui::Event::PointerButton {
                button,
                pressed: false,
                ..
            } => match self.buttons.remove(&(*button as u8)) {
                Some(ButtonSink::Legacy) => {
                    egui.push(event.clone());
                    Disposition::Egui
                }
                _ => Disposition::Retired("barrier"),
            },
            egui::Event::Key {
                key,
                physical_key,
                pressed: true,
                ..
            } => {
                self.keys
                    .insert(physical_key.unwrap_or(*key), KeySink::Retired);
                Disposition::Retired("barrier")
            }
            egui::Event::PointerButton {
                button,
                pressed: true,
                ..
            } => {
                self.buttons.insert(*button as u8, ButtonSink::Retired);
                Disposition::Retired("barrier")
            }
            _ => Disposition::Retired("barrier"),
        };
        self.trace(&event, owner, disposition, Transition::Stay);
    }

    fn dispatch(
        &mut self,
        snap: &Snapshot<'_>,
        owner: Owner,
        event: &Received,
        claimed_text: bool,
    ) -> (Disposition, Transition) {
        match &event.event {
            egui::Event::Key {
                key,
                physical_key,
                pressed,
                modifiers,
                ..
            } => {
                let identity = physical_key.unwrap_or(*key);
                if *pressed {
                    self.key_press(snap, owner, event, identity, *modifiers)
                } else {
                    (self.key_release(owner, identity), Transition::Stay)
                }
            }
            egui::Event::Text(_) if claimed_text => {
                (Disposition::Retired("associated text"), Transition::Stay)
            }
            egui::Event::Text(_) | egui::Event::Paste(_) | egui::Event::Copy | egui::Event::Cut => {
                match owner {
                    Owner::Terminal(target) => (Disposition::Terminal(target), Transition::Stay),
                    Owner::Dialog(_) => (
                        Disposition::Dialog(DialogOutcome::Ignored),
                        Transition::Stay,
                    ),
                    Owner::Legacy => (Disposition::Egui, Transition::Stay),
                }
            }
            // Hover is egui's, even under a dialog: it moves no authority.
            egui::Event::PointerMoved(_) => (Disposition::Egui, Transition::Stay),
            egui::Event::PointerButton {
                button,
                pressed,
                pos,
                ..
            } => {
                if *pressed {
                    self.button_press(snap, owner, event, *button, *pos)
                } else {
                    self.button_release(snap, owner, event, *button, *pos)
                }
            }
            egui::Event::MouseWheel { .. } => match owner {
                Owner::Dialog(_) => (
                    Disposition::Retired("dialog owns the pointer"),
                    Transition::Stay,
                ),
                _ => (Disposition::Egui, Transition::Stay),
            },
            _ => match owner {
                // IME, touch, zoom and accessibility requests have no
                // recipient inside a router dialog yet.
                Owner::Dialog(_) => (
                    Disposition::Retired("no dialog recipient"),
                    Transition::Stay,
                ),
                _ => (Disposition::Egui, Transition::Stay),
            },
        }
    }

    fn key_press(
        &mut self,
        snap: &Snapshot<'_>,
        owner: Owner,
        event: &Received,
        identity: egui::Key,
        modifiers: egui::Modifiers,
    ) -> (Disposition, Transition) {
        let key = match &event.event {
            egui::Event::Key { key, .. } => *key,
            _ => identity,
        };
        if let Some(sink) = self.keys.get(&identity).cloned() {
            // A press of a key already down is a repeat, whatever the
            // platform's repeat flag says.
            return match sink {
                KeySink::Host {
                    no_repeat: true, ..
                } => {
                    self.claimed_text = Some(event.id.delivery);
                    (Disposition::Retired("no-repeat binding"), Transition::Stay)
                }
                KeySink::Host { action, .. } => {
                    self.claimed_text = Some(event.id.delivery);
                    self.host_action(owner, event, action)
                }
                KeySink::Terminal(target) if owner == Owner::Terminal(target) => {
                    (Disposition::Terminal(target), Transition::Stay)
                }
                KeySink::Dialog(opening) if owner == Owner::Dialog(opening) => {
                    self.claimed_text = Some(event.id.delivery);
                    self.dialog_key(snap, owner, event, identity, modifiers, false)
                }
                KeySink::Legacy if owner == Owner::Legacy => (Disposition::Egui, Transition::Stay),
                _ => {
                    self.claimed_text = Some(event.id.delivery);
                    (
                        Disposition::Retired("repeat after authority change"),
                        Transition::Stay,
                    )
                }
            };
        }
        match owner {
            Owner::Terminal(target) => {
                let gates = BindingGates {
                    overlay_open: false,
                    ..snap.gates
                };
                match binding::match_press(snap.table, key, modifiers, gates) {
                    Some(matched) => {
                        self.keys.insert(
                            identity,
                            KeySink::Host {
                                action: matched.action.clone(),
                                no_repeat: matched.no_repeat,
                            },
                        );
                        self.claimed_text = Some(event.id.delivery);
                        self.host_action(owner, event, matched.action)
                    }
                    None => {
                        self.keys.insert(identity, KeySink::Terminal(target));
                        (Disposition::Terminal(target), Transition::Stay)
                    }
                }
            }
            Owner::Dialog(opening) => {
                self.keys.insert(identity, KeySink::Dialog(opening));
                self.claimed_text = Some(event.id.delivery);
                self.dialog_key(snap, owner, event, identity, modifiers, true)
            }
            Owner::Legacy => {
                self.keys.insert(identity, KeySink::Legacy);
                (Disposition::Egui, Transition::Stay)
            }
        }
    }

    fn key_release(&mut self, owner: Owner, identity: egui::Key) -> Disposition {
        match self.keys.remove(&identity) {
            Some(KeySink::Terminal(target)) if owner == Owner::Terminal(target) => {
                Disposition::Terminal(target)
            }
            Some(KeySink::Legacy) => Disposition::Egui,
            Some(_) => Disposition::Retired("paired release"),
            None => match owner {
                Owner::Terminal(target) => Disposition::Terminal(target),
                Owner::Legacy => Disposition::Egui,
                Owner::Dialog(_) => Disposition::Retired("unpaired release"),
            },
        }
    }

    fn host_action(
        &mut self,
        owner: Owner,
        event: &Received,
        action: Action,
    ) -> (Disposition, Transition) {
        let id = self.next_command();
        let target = match owner {
            Owner::Terminal(target) => Some(target),
            _ => None,
        };
        self.commands.push(RoutedCommand::HostAction {
            id,
            action: action.clone(),
            target,
            event: event.id,
        });
        (Disposition::HostAction(action), Transition::Wait(id))
    }

    fn dialog_decision(&mut self, event: &Received, action: String) -> (Disposition, Transition) {
        let Some(opening) = self.dialog.as_ref().map(|m| m.opening) else {
            return (Disposition::Retired("no dialog"), Transition::Stay);
        };
        let id = self.next_command();
        self.commands.push(RoutedCommand::DialogDecision {
            id,
            opening,
            action: action.clone(),
            event: event.id,
        });
        // Later deliveries wait for logic to answer the dialog; the rest of
        // this one is barred.
        self.waiting = Some(id);
        (
            Disposition::Dialog(DialogOutcome::Activate(action)),
            Transition::Dismiss,
        )
    }

    fn dialog_outcome(
        &mut self,
        event: &Received,
        outcome: DialogOutcome,
    ) -> (Disposition, Transition) {
        match outcome {
            DialogOutcome::Activate(action) => self.dialog_decision(event, action),
            DialogOutcome::Refused { .. } => (Disposition::Dialog(outcome), Transition::Veto),
            other => (Disposition::Dialog(other), Transition::Stay),
        }
    }

    fn dialog_key(
        &mut self,
        snap: &Snapshot<'_>,
        owner: Owner,
        event: &Received,
        identity: egui::Key,
        modifiers: egui::Modifiers,
        fresh: bool,
    ) -> (Disposition, Transition) {
        let key = match &event.event {
            egui::Event::Key { key, .. } => *key,
            _ => identity,
        };
        let global = BindingGates {
            overlay_open: true,
            ..snap.gates
        };
        let matched = binding::match_press(snap.table, key, modifiers, global);
        // The close chord cancels the dialog that owns it, with the same
        // policy as its Cancel button; its W edge stays paired here.
        if matched
            .as_ref()
            .is_some_and(|m| m.action == Action::ClosePane)
        {
            let cancel = self
                .dialog
                .as_ref()
                .and_then(|m| m.cancel_id().map(str::to_owned));
            return match cancel {
                Some(cancel) if fresh => self.dialog_decision(event, cancel),
                _ => (Disposition::Retired("close chord repeat"), Transition::Stay),
            };
        }
        let synthetic = event.synthetic();
        let outcome = match self.dialog.as_mut() {
            Some(model) => model.key(key, modifiers, fresh, synthetic),
            None => DialogOutcome::Ignored,
        };
        if outcome != DialogOutcome::Ignored {
            return self.dialog_outcome(event, outcome);
        }
        match matched {
            // Presentation-only host chords stay live over a dialog.
            Some(m) if fresh && dialog_admits(&m.action) => {
                self.keys.insert(
                    identity,
                    KeySink::Host {
                        action: m.action.clone(),
                        no_repeat: m.no_repeat,
                    },
                );
                self.host_action(owner, event, m.action)
            }
            // A critical dialog is not preempted (Quick Note, palette, ...).
            Some(m) => {
                log::info!("host_router: dialog vetoes {:?} (critical modal)", m.action);
                (
                    Disposition::Dialog(DialogOutcome::Ignored),
                    Transition::Veto,
                )
            }
            None => (
                Disposition::Dialog(DialogOutcome::Ignored),
                Transition::Stay,
            ),
        }
    }

    fn button_press(
        &mut self,
        snap: &Snapshot<'_>,
        owner: Owner,
        event: &Received,
        button: egui::PointerButton,
        pos: egui::Pos2,
    ) -> (Disposition, Transition) {
        if let Owner::Dialog(opening) = owner {
            let hit = dialog::hit(opening, snap.dialog_presentation.as_ref(), pos);
            self.buttons.insert(
                button as u8,
                ButtonSink::Dialog {
                    opening,
                    hit,
                    synthetic: event.synthetic(),
                },
            );
            return (Disposition::DialogPointer, Transition::Stay);
        }
        // Terminal panes, chrome and legacy overlays take their pointer
        // input from egui; the release follows the press there.
        self.buttons.insert(button as u8, ButtonSink::Legacy);
        (Disposition::Egui, Transition::Stay)
    }

    fn button_release(
        &mut self,
        snap: &Snapshot<'_>,
        owner: Owner,
        event: &Received,
        button: egui::PointerButton,
        pos: egui::Pos2,
    ) -> (Disposition, Transition) {
        match self.buttons.remove(&(button as u8)) {
            Some(ButtonSink::Dialog {
                opening,
                hit,
                synthetic,
            }) if owner == Owner::Dialog(opening) => {
                let release = dialog::hit(opening, snap.dialog_presentation.as_ref(), pos);
                if release != hit {
                    return (
                        Disposition::Dialog(DialogOutcome::Ignored),
                        Transition::Stay,
                    );
                }
                let outcome = match self.dialog.as_ref() {
                    Some(model) => model.click(
                        &hit,
                        snap.dialog_presentation.as_ref(),
                        synthetic || event.synthetic(),
                    ),
                    None => DialogOutcome::Ignored,
                };
                self.dialog_outcome(event, outcome)
            }
            Some(ButtonSink::Legacy) => (Disposition::Egui, Transition::Stay),
            Some(_) => (Disposition::Retired("paired release"), Transition::Stay),
            None => match owner {
                Owner::Dialog(_) => (Disposition::Retired("unpaired release"), Transition::Stay),
                _ => (Disposition::Egui, Transition::Stay),
            },
        }
    }
}

/// Host chords that stay live over a router dialog: they change presentation
/// or quit, never who owns input.
fn dialog_admits(action: &Action) -> bool {
    matches!(
        action,
        Action::Quit
            | Action::IncreasePaneFontSize
            | Action::DecreasePaneFontSize
            | Action::ResetPaneFontSize
            | Action::UiScaleUp
            | Action::UiScaleDown
            | Action::UiScaleReset
    )
}

/// Pointer input: under a terminal it stays on egui's path.
fn pointer(event: &egui::Event) -> bool {
    matches!(
        event,
        egui::Event::PointerMoved(_)
            | egui::Event::PointerButton { .. }
            | egui::Event::MouseWheel { .. }
            | egui::Event::PointerGone
    )
}

/// Events every pass needs whoever owns input.
fn lifecycle(event: &egui::Event) -> bool {
    matches!(
        event,
        egui::Event::WindowFocused(_) | egui::Event::Screenshot { .. } | egui::Event::PointerGone
    )
}

/// An event whose effect on egui's recipients can change authority.
fn actionable(event: &egui::Event) -> bool {
    matches!(
        event,
        egui::Event::Key { pressed: true, .. }
            | egui::Event::Text(_)
            | egui::Event::Paste(_)
            | egui::Event::Copy
            | egui::Event::Cut
            | egui::Event::PointerButton { pressed: true, .. }
            | egui::Event::AccessKitActionRequest(_)
            | egui::Event::Ime(_)
    )
}

fn disposition_label(disposition: &Disposition) -> String {
    match disposition {
        Disposition::Terminal(t) => format!("terminal(pane={})", t.pane),
        Disposition::HostAction(action) => format!("host({action:?})"),
        Disposition::Dialog(DialogOutcome::Activate(id)) => format!("dialog(activate {id})"),
        Disposition::Dialog(DialogOutcome::Refused { action, reason }) => {
            format!("dialog(refused {action}: {reason})")
        }
        Disposition::Dialog(outcome) => format!("dialog({outcome:?})"),
        Disposition::DialogPointer => "dialog(pointer)".to_string(),
        Disposition::Egui => "egui".to_string(),
        Disposition::Retired(why) => format!("retired({why})"),
    }
}
