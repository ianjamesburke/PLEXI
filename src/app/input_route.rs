//! Host input router, first slice (stint 1659): one ordered route across the
//! boundary between a pane and a modal.
//!
//! `PlexiApp::raw_input_hook` hands every delivery to [`InputRoute::route`]
//! before egui folds it into key, click and drag state. The router walks the
//! delivery in order against the owners committed by the previous pass, and
//! only what it routes reaches egui this pass:
//!
//! - **Keys.** A key's first down records its owner. Its repeats, its text and
//!   its up follow that owner while it keeps authority. Once the owner loses
//!   authority they are retired, and egui's folded key state is balanced so the
//!   next fresh press is not mistaken for a repeat.
//! - **Pointer.** A press captures its owner (and the terminal it landed on)
//!   until release. If the owner loses authority the gesture is cancelled at
//!   its sinks: egui gets a non-clicking release, the terminal ends its drag
//!   and reports the button up, and the physical release is retired.
//! - **Runs.** A pass's run ends at its first decisive event, one that can
//!   change who owns input. Everything after it waits for the next pass, so the
//!   change commits first. When that event dismissed or revealed its owner the
//!   rest of its delivery is consumed (the dismissal barrier; releases of live
//!   presses still finish). When it opened something above its owner, the rest
//!   goes to the new owner. A later delivery keeps its own boundary.
//! - **Blur** cancels every held interaction; a hidden window drops waiting
//!   suffixes the same way.
//!
//! The hook only decides. Sink effects and the routing trace run once per pass
//! in `App::logic` ([`InputRoute::take_commands`], [`InputRoute::take_notes`]).
//! The recipients behind a run (the palette, other overlays, panes) still read
//! it through their existing egui paths; moving those onto owner-local streams
//! is the next slice.

use crate::app::input_owner::OverlaySurface;
use crate::host::keys::BindingEntry;
use crate::spatial::tiling::PaneId;
use std::collections::{HashMap, HashSet, VecDeque};

/// Where a cancelled press is released for egui: off every window.
const RELEASE_NOWHERE: egui::Pos2 = egui::pos2(-1.0e4, -1.0e4);

/// One owner instance. A palette owner is one opening of the palette.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Owner {
    /// The focused pane, for keyboard input.
    Pane(PaneId),
    /// Panes and chrome with no overlay above them: pointer input, and keys
    /// when no pane is focused.
    Workspace,
    /// The command palette, by opening.
    Palette(u64),
    /// Any other overlay surface (its own input stays on its legacy path).
    Overlay(OverlaySurface),
}

/// Which owner an event addresses: keyboard and text, or the pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Key,
    Pointer,
}

/// Who holds authority, from committed host state, when a delivery is routed.
pub(crate) struct Authority {
    pub(crate) key: Owner,
    pub(crate) pointer: Owner,
    /// The open palette's opening, if it is open.
    pub(crate) palette: Option<u64>,
    /// Every overlay surface currently present, covered or not.
    pub(crate) overlays: Vec<OverlaySurface>,
    pub(crate) panes: HashSet<PaneId>,
}

impl Authority {
    fn owner(&self, class: Class) -> Owner {
        match class {
            Class::Key => self.key,
            Class::Pointer => self.pointer,
        }
    }

    /// Whether `owner` still exists, even if something now covers it.
    fn alive(&self, owner: Owner) -> bool {
        match owner {
            Owner::Pane(pane) => self.panes.contains(&pane),
            Owner::Workspace => true,
            Owner::Palette(opening) => self.palette == Some(opening),
            Owner::Overlay(surface) => self.overlays.contains(&surface),
        }
    }
}

/// An effect the router decided on, executed once in `App::logic`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RouteCommand {
    /// End this terminal's pointer gesture: its owner lost the pointer.
    CancelTerminalGesture { pane: PaneId },
}

/// What one pass hands egui.
#[derive(Default)]
pub(crate) struct RoutedPass {
    /// The run, in order, including cancellation releases.
    pub(crate) events: Vec<egui::Event>,
    /// The modifiers of the delivery the run's last event came from.
    pub(crate) modifiers: Option<egui::Modifiers>,
    /// Whether any event in the run was injected rather than native.
    pub(crate) synthetic: bool,
    /// Keys egui folded a down for whose owner lost authority: remove them
    /// from egui's held keys before this pass folds.
    pub(crate) unfold_keys: Vec<egui::Key>,
    /// Ups retired here, so no legacy release claim outlives its key.
    pub(crate) retired_ups: Vec<egui::Key>,
    /// Input is still waiting: the host must run another pass.
    pub(crate) waiting: bool,
}

struct KeyEdge {
    owner: Owner,
    /// False once retired: its owner lost authority, or a barrier took it.
    live: bool,
}

struct ButtonEdge {
    button: egui::PointerButton,
    owner: Owner,
    /// The terminal the press landed on, in the last presented layout.
    terminal: Option<PaneId>,
    live: bool,
    /// Whether egui folded the press (it must then fold a release).
    in_egui: bool,
}

/// A delivery, or the part of one still waiting.
struct Segment {
    delivery: u64,
    modifiers: egui::Modifiers,
    synthetic: bool,
    events: VecDeque<egui::Event>,
    /// Set when an earlier run of this delivery ended at a decisive event:
    /// the owner it went to, and which kind of owner that was.
    bound: Option<(Owner, Class)>,
}

#[derive(Default)]
pub(crate) struct InputRoute {
    next_delivery: u64,
    waiting: VecDeque<Segment>,
    keys: HashMap<egui::Key, KeyEdge>,
    /// Held presses, by `PointerButton as usize`.
    buttons: [Option<ButtonEdge>; egui::NUM_POINTER_BUTTONS],
    /// Text that belongs to a retired key down is retired with it.
    retire_text: bool,
    commands: Vec<RouteCommand>,
    notes: Vec<String>,
}

/// Events a recipient acts on. Everything else (lifecycle, screenshots, focus
/// changes) reaches egui whatever the owner.
fn actionable(event: &egui::Event) -> bool {
    use egui::Event as E;
    matches!(
        event,
        E::Key { .. }
            | E::Text(_)
            | E::Paste(_)
            | E::Copy
            | E::Cut
            | E::Ime(_)
            | E::PointerButton { .. }
            | E::PointerMoved(_)
            | E::MouseMoved(_)
            | E::MouseWheel { .. }
            | E::Zoom(_)
            | E::Rotate(_)
            | E::Touch { .. }
            | E::AccessKitActionRequest(_)
    )
}

/// Keys that edit or move rather than type: a modal may act on them.
fn command_key(key: egui::Key) -> bool {
    use egui::Key as K;
    matches!(
        key,
        K::Escape
            | K::Enter
            | K::Tab
            | K::Backspace
            | K::Delete
            | K::Home
            | K::End
            | K::PageUp
            | K::PageDown
            | K::ArrowUp
            | K::ArrowDown
            | K::ArrowLeft
            | K::ArrowRight
    )
}

/// Whether a key down sent to `owner` can change who owns input.
fn key_decisive(
    key: egui::Key,
    modifiers: egui::Modifiers,
    owner: Owner,
    bindings: &[BindingEntry],
) -> bool {
    let bound = bindings.iter().any(|entry| {
        let same_key = entry.key == key
            || matches!(
                (entry.key, key),
                (egui::Key::Equals, egui::Key::Plus) | (egui::Key::Plus, egui::Key::Equals)
            );
        same_key && modifiers.matches_logically(entry.modifiers)
    });
    // Cmd+1..9 switch context from a hardcoded loop, not the table.
    let context_switch = modifiers.command
        && matches!(
            key,
            egui::Key::Num1
                | egui::Key::Num2
                | egui::Key::Num3
                | egui::Key::Num4
                | egui::Key::Num5
                | egui::Key::Num6
                | egui::Key::Num7
                | egui::Key::Num8
                | egui::Key::Num9
        );
    let modal = matches!(owner, Owner::Palette(_) | Owner::Overlay(_));
    bound
        || context_switch
        || key == egui::Key::Escape
        || (modal && (modifiers.command || modifiers.ctrl || command_key(key)))
}

impl InputRoute {
    /// Route one delivery. `raw.events` becomes this pass's run; anything
    /// after its first decisive event waits for the next pass.
    pub(crate) fn route(
        &mut self,
        raw: &mut egui::RawInput,
        synthetic: bool,
        authority: &Authority,
        bindings: &[BindingEntry],
        terminal_at: &dyn Fn(egui::Pos2) -> Option<PaneId>,
    ) -> RoutedPass {
        if !raw.events.is_empty() {
            self.next_delivery += 1;
            self.waiting.push_back(Segment {
                delivery: self.next_delivery,
                modifiers: raw.modifiers,
                synthetic,
                events: raw.events.drain(..).collect(),
                bound: None,
            });
        }
        let mut out = RoutedPass::default();
        self.reconcile(authority, &mut out);
        while let Some(mut segment) = self.waiting.pop_front() {
            if let Some((bound, class)) = segment.bound.take() {
                let now = authority.owner(class);
                if now != bound {
                    if authority.alive(bound) {
                        self.notes.push(format!(
                            "delivery={} transfer {bound:?} -> {now:?}",
                            segment.delivery
                        ));
                    } else {
                        self.bar(segment, authority, &mut out);
                        continue;
                    }
                }
            }
            while let Some(event) = segment.events.pop_front() {
                let Some(decisive) =
                    self.route_event(event, authority, bindings, terminal_at, &mut out)
                else {
                    continue;
                };
                out.modifiers = Some(segment.modifiers);
                out.synthetic |= segment.synthetic;
                if let Some(class) = decisive {
                    if !segment.events.is_empty() {
                        segment.bound = Some((authority.owner(class), class));
                        self.waiting.push_front(segment);
                        out.waiting = true;
                        return out;
                    }
                    if !self.waiting.is_empty() {
                        out.waiting = true;
                        return out;
                    }
                }
            }
        }
        out
    }

    /// A hidden window has no recipient to commit anything: retire the input
    /// still waiting and cancel interactions whose owner went away.
    pub(crate) fn route_hidden(&mut self, authority: &Authority) -> RoutedPass {
        let mut out = RoutedPass::default();
        self.reconcile(authority, &mut out);
        while let Some(segment) = self.waiting.pop_front() {
            self.bar(segment, authority, &mut out);
        }
        out
    }

    pub(crate) fn take_commands(&mut self) -> Vec<RouteCommand> {
        std::mem::take(&mut self.commands)
    }

    pub(crate) fn take_notes(&mut self) -> Vec<String> {
        std::mem::take(&mut self.notes)
    }

    /// Cancel every held interaction whose owner no longer has authority.
    fn reconcile(&mut self, authority: &Authority, out: &mut RoutedPass) {
        for (key, edge) in &mut self.keys {
            if edge.live && edge.owner != authority.key {
                edge.live = false;
                out.unfold_keys.push(*key);
                self.notes.push(format!(
                    "retire key {key:?}: owner {:?} lost authority",
                    edge.owner
                ));
            }
        }
        let lost: Vec<egui::PointerButton> = self
            .buttons
            .iter()
            .flatten()
            .filter(|edge| edge.live && edge.owner != authority.pointer)
            .map(|edge| edge.button)
            .collect();
        for button in lost {
            self.cancel_button(button, "owner lost authority", out);
        }
    }

    fn cancel_button(&mut self, button: egui::PointerButton, why: &str, out: &mut RoutedPass) {
        let Some(edge) = self.buttons[button as usize].as_mut() else {
            return;
        };
        edge.live = false;
        self.notes.push(format!(
            "cancel {button:?} press of {:?}: {why}",
            edge.owner
        ));
        if let Some(pane) = edge.terminal {
            self.commands
                .push(RouteCommand::CancelTerminalGesture { pane });
        }
        if edge.in_egui {
            edge.in_egui = false;
            // egui folded the press, so it must fold a release. It lands
            // outside every window, so no widget hovers it or can take it as
            // a click; the sinks were cancelled above, and the next move
            // restores hover.
            out.events.push(egui::Event::PointerButton {
                pos: RELEASE_NOWHERE,
                button,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            });
            out.events.push(egui::Event::PointerGone);
        }
    }

    fn cancel_all(&mut self, why: &str, out: &mut RoutedPass) {
        for (key, edge) in &mut self.keys {
            if edge.live {
                edge.live = false;
                out.unfold_keys.push(*key);
            }
        }
        let held: Vec<egui::PointerButton> = self
            .buttons
            .iter()
            .flatten()
            .filter(|edge| edge.live)
            .map(|edge| edge.button)
            .collect();
        for button in held {
            self.cancel_button(button, why, out);
        }
        self.notes.push(format!("cancel held input: {why}"));
    }

    /// Route one event. `None`: retired. `Some(None)`: routed. `Some(Some)`:
    /// routed, and decisive for that class of owner.
    fn route_event(
        &mut self,
        event: egui::Event,
        authority: &Authority,
        bindings: &[BindingEntry],
        terminal_at: &dyn Fn(egui::Pos2) -> Option<PaneId>,
        out: &mut RoutedPass,
    ) -> Option<Option<Class>> {
        use egui::Event as E;
        let mut decisive = None;
        match &event {
            E::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } => {
                let owner = match self.keys.get(key) {
                    Some(edge) if !edge.live => {
                        self.retire_text = true;
                        return None;
                    }
                    Some(edge) => edge.owner,
                    None => {
                        self.keys.insert(
                            *key,
                            KeyEdge {
                                owner: authority.key,
                                live: true,
                            },
                        );
                        authority.key
                    }
                };
                self.retire_text = false;
                if key_decisive(*key, *modifiers, owner, bindings) {
                    decisive = Some(Class::Key);
                }
            }
            E::Key {
                key,
                pressed: false,
                ..
            } => {
                self.retire_text = false;
                if self.keys.remove(key).is_some_and(|edge| !edge.live) {
                    out.retired_ups.push(*key);
                    return None;
                }
            }
            E::Text(_) if self.retire_text => return None,
            E::PointerButton {
                pos,
                button,
                pressed: true,
                ..
            } => {
                let terminal = (authority.pointer == Owner::Workspace)
                    .then(|| terminal_at(*pos))
                    .flatten();
                self.buttons[*button as usize] = Some(ButtonEdge {
                    button: *button,
                    owner: authority.pointer,
                    terminal,
                    live: true,
                    in_egui: true,
                });
                decisive = Some(Class::Pointer);
            }
            E::PointerButton {
                button,
                pressed: false,
                ..
            } => {
                if self.buttons[*button as usize]
                    .take()
                    .is_some_and(|edge| !edge.live)
                {
                    return None;
                }
                decisive = Some(Class::Pointer);
            }
            E::WindowFocused(false) => self.cancel_all("window blurred", out),
            E::AccessKitActionRequest(_) => decisive = Some(Class::Pointer),
            _ => {}
        }
        out.events.push(event);
        Some(decisive)
    }

    /// The dismissal barrier: the rest of this delivery reaches nobody.
    /// Releases of presses that still have their owner finish there; every
    /// new down is retired with its repeats and up.
    fn bar(&mut self, segment: Segment, authority: &Authority, out: &mut RoutedPass) {
        let mut retired = 0usize;
        for event in segment.events {
            use egui::Event as E;
            match &event {
                E::Key {
                    key, pressed: true, ..
                } => {
                    self.keys.entry(*key).or_insert(KeyEdge {
                        owner: authority.key,
                        live: false,
                    });
                }
                E::Key {
                    key,
                    pressed: false,
                    ..
                } => match self.keys.remove(key) {
                    Some(edge) if edge.live => {
                        out.events.push(event);
                        continue;
                    }
                    Some(_) => out.retired_ups.push(*key),
                    None => {}
                },
                E::PointerButton {
                    button,
                    pressed: true,
                    ..
                } => {
                    self.buttons[*button as usize] = Some(ButtonEdge {
                        button: *button,
                        owner: authority.pointer,
                        terminal: None,
                        live: false,
                        in_egui: false,
                    });
                }
                E::PointerButton {
                    button,
                    pressed: false,
                    ..
                } => {
                    if self.buttons[*button as usize]
                        .take()
                        .is_some_and(|edge| edge.live)
                    {
                        out.events.push(event);
                        continue;
                    }
                }
                E::WindowFocused(false) => {
                    self.cancel_all("window blurred", out);
                    out.events.push(event);
                    continue;
                }
                _ if !actionable(&event) => {
                    out.events.push(event);
                    continue;
                }
                _ => {}
            }
            retired += 1;
        }
        self.retire_text = false;
        self.notes.push(format!(
            "delivery={} dismissal barrier: retired {retired} event(s)",
            segment.delivery
        ));
    }
}

/// Host glue: what the router reads from committed state, and the effects it
/// hands `App::logic`.
impl crate::app::PlexiApp {
    /// Authority as the last pass committed it, by the one precedence rule
    /// (`host_input_owner`).
    pub(crate) fn input_authority(&self) -> Authority {
        use crate::app::input_owner::InputOwner;
        use crate::app::FocusKind;
        let opening = self.palette_opening;
        let owner_of = |surface| match surface {
            OverlaySurface::Layer(FocusKind::CommandPalette) => Owner::Palette(opening),
            surface => Owner::Overlay(surface),
        };
        let (key, pointer) = match self.host_input_owner() {
            InputOwner::Overlay(surface) => (owner_of(surface), owner_of(surface)),
            InputOwner::Pane(pane) => (Owner::Pane(pane), Owner::Workspace),
            InputOwner::OsUnfocused | InputOwner::Nobody => (Owner::Workspace, Owner::Workspace),
        };
        let mut overlays: Vec<OverlaySurface> = self
            .focus_stack
            .iter()
            .map(|kind| OverlaySurface::Layer(*kind))
            .collect();
        if self.attention_queue.decision_overlay.is_some() {
            overlays.push(OverlaySurface::DecisionOverlay);
        }
        if self.attention_queue.open {
            overlays.push(OverlaySurface::AttentionQueue);
        }
        if self.zoom.level == crate::app::zoom::ZoomLevel::TreeAtlas {
            overlays.push(OverlaySurface::TreeAtlas);
        }
        Authority {
            key,
            pointer,
            palette: self
                .focus_stack
                .contains(&FocusKind::CommandPalette)
                .then_some(opening),
            overlays,
            panes: self
                .windows
                .iter()
                .flat_map(|window| window.panes.keys().copied())
                .collect(),
        }
    }

    /// Route this delivery (see the module docs). Runs in `raw_input_hook`,
    /// before egui folds `raw_input`.
    pub(crate) fn route_delivery(
        &mut self,
        ctx: &egui::Context,
        raw_input: &mut egui::RawInput,
        synthetic: bool,
    ) {
        let authority = self.input_authority();
        let hidden = raw_input.viewport().visible() == Some(false);
        let pass = if hidden {
            self.input_route.route_hidden(&authority)
        } else {
            let windows = &self.windows;
            let terminal_at = |pos| terminal_under(ctx, windows, pos);
            self.input_route.route(
                raw_input,
                synthetic,
                &authority,
                &self.binding_table,
                &terminal_at,
            )
        };
        if !pass.unfold_keys.is_empty() {
            ctx.input_mut(|input| {
                for key in &pass.unfold_keys {
                    input.keys_down.remove(key);
                }
            });
        }
        for key in &pass.retired_ups {
            self.claimed_host_key_releases.remove(key);
        }
        if hidden {
            // A hidden pass folds its own native input as before; only what
            // the router released from waiting suffixes goes ahead of it.
            raw_input.events.splice(0..0, pass.events);
        } else {
            raw_input.events = pass.events;
        }
        if let Some(modifiers) = pass.modifiers {
            raw_input.modifiers = modifiers;
        }
        self.synthetic_input_frame |= pass.synthetic;
        if pass.waiting {
            ctx.request_repaint();
        }
    }

    /// Execute what the router decided, once per pass, and trace it.
    pub(crate) fn service_input_route(&mut self, ctx: &egui::Context) {
        for note in self.input_route.take_notes() {
            log::info!("input_route: {note}");
        }
        for command in self.input_route.take_commands() {
            match command {
                RouteCommand::CancelTerminalGesture { pane } => {
                    let terminal = self
                        .windows
                        .iter_mut()
                        .find_map(|window| window.panes.get_mut(&pane))
                        .and_then(|pane| pane.as_terminal_mut());
                    match terminal {
                        Some(terminal) => {
                            egui_term::cancel_pointer_gesture(ctx, &mut terminal.backend);
                            log::info!("input_route: cancelled pointer gesture in pane={pane}");
                        }
                        None => log::info!(
                            "input_route: pane={pane} closed before its pointer gesture was cancelled"
                        ),
                    }
                }
            }
        }
    }
}

/// The terminal a press at `pos` lands on, in the last presented layout.
fn terminal_under(
    ctx: &egui::Context,
    windows: &[crate::host::context::Window],
    pos: egui::Pos2,
) -> Option<PaneId> {
    let layer = ctx.layer_id_at(pos)?;
    windows
        .iter()
        .flat_map(|window| window.panes.iter())
        .filter(|(_, pane)| pane.as_terminal().is_some())
        .map(|(id, _)| *id)
        .find(|id| {
            ctx.read_response(egui_term::terminal_widget_id(*id))
                .is_some_and(|response| {
                    response.layer_id == layer && response.interact_rect.contains(pos)
                })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(key: egui::Key, pressed: bool) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    fn authority(key: Owner, pointer: Owner, palette: Option<u64>) -> Authority {
        Authority {
            key,
            pointer,
            palette,
            overlays: Vec::new(),
            panes: [1, 2].into_iter().collect(),
        }
    }

    fn route(r: &mut InputRoute, events: Vec<egui::Event>, a: &Authority) -> RoutedPass {
        let mut raw = egui::RawInput {
            events,
            ..Default::default()
        };
        let pass = r.route(&mut raw, false, a, &[], &|_| None);
        assert_eq!(raw.events.len(), 0, "the hook replaces raw.events itself");
        pass
    }

    /// Escape to the palette ends the run; once the palette has closed, the
    /// rest of that delivery is barred, and the next delivery is routed.
    #[test]
    fn dismissal_bars_the_rest_of_its_delivery_only() {
        let mut r = InputRoute::default();
        let palette = authority(Owner::Palette(1), Owner::Palette(1), Some(1));
        let at = egui::pos2(5.0, 5.0);
        let press = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let pass = route(
            &mut r,
            vec![key(egui::Key::Escape, true), press(true), press(false)],
            &palette,
        );
        assert_eq!(pass.events.len(), 1);
        assert!(pass.waiting);
        let pane = authority(Owner::Pane(1), Owner::Workspace, None);
        let pass = route(&mut r, vec![], &pane);
        assert!(pass.events.is_empty(), "barrier: {:?}", pass.events);
        assert_eq!(pass.unfold_keys, vec![egui::Key::Escape]);
        let pass = route(&mut r, vec![press(true), press(false)], &pane);
        assert_eq!(pass.events.len(), 1, "a fresh press is routed");
    }

    /// A key retired with its owner takes its repeats, text and up with it.
    #[test]
    fn a_retired_key_takes_its_repeats_text_and_up() {
        let mut r = InputRoute::default();
        let pane = authority(Owner::Pane(1), Owner::Workspace, None);
        route(&mut r, vec![key(egui::Key::A, true)], &pane);
        let palette = authority(Owner::Palette(1), Owner::Palette(1), Some(1));
        let pass = route(
            &mut r,
            vec![
                key(egui::Key::A, true),
                egui::Event::Text("a".into()),
                key(egui::Key::A, false),
            ],
            &palette,
        );
        assert!(pass.events.is_empty(), "{:?}", pass.events);
        assert_eq!(pass.unfold_keys, vec![egui::Key::A]);
        assert_eq!(pass.retired_ups, vec![egui::Key::A]);
    }
}
