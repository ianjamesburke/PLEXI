//! The host input router's two host-side halves (stint 1659): ingress in
//! `raw_input_hook` (stamp, snapshot, route) and execution in `App::logic`
//! (terminal input, routed focus, host actions, dialog decisions). Routing
//! itself is pure and lives in [`crate::app::host_router`].

use super::host_router::{
    dialog::{ActionSpec, DialogPresentation},
    DialogSnapshot, KeyboardOwner, Outcome, PaneInstance, Received, RoutedCommand, Snapshot,
    Source,
};
use super::{FocusKind, PlexiApp};

/// The Confirm Close actions for a single pane, or for quitting with working
/// agents. Paint and the router both read this one list.
pub(crate) fn confirm_close_actions(
    pending_quit: bool,
) -> [crate::ui::dialog::DialogAction<'static>; 2] {
    [
        crate::ui::dialog::DialogAction::new(
            "close",
            if pending_quit {
                "Quit Plexi and stop them"
            } else {
                "Close"
            },
            crate::ui::button::ButtonKind::Danger,
        )
        .destructive()
        .shortcut(crate::ui::dialog::DialogShortcut::new(
            &["Enter"],
            egui::Modifiers::NONE,
            egui::Key::Enter,
        )),
        crate::ui::dialog::DialogAction::new(
            "cancel",
            "Cancel",
            crate::ui::button::ButtonKind::Secondary,
        )
        .refuses()
        .shortcut(crate::ui::dialog::DialogShortcut::new(
            &["Esc"],
            egui::Modifiers::NONE,
            egui::Key::Escape,
        )),
    ]
}

impl PlexiApp {
    /// The pane-close confirmation the router owns, when it owns input: a
    /// single or batched pane close (not the quit dialogs), on top of the
    /// focus stack or about to be pushed there, with no host overlay above.
    pub(crate) fn router_dialog(&self) -> Option<DialogSnapshot> {
        if !self.pending_close
            || self.pending_quit
            || self.quit_coordinator.save_failure().is_some()
        {
            return None;
        }
        if self.attention_queue.decision_overlay.is_some() || self.attention_queue.open {
            return None;
        }
        let buried = self
            .focus_stack
            .iter()
            .position(|kind| *kind == FocusKind::ConfirmClose)
            .is_some_and(|at| at + 1 != self.focus_stack.len());
        if buried {
            return None;
        }
        let target = self.close_confirm_target.as_ref()?;
        Some(DialogSnapshot {
            opening: target.opening,
            actions: confirm_close_actions(false)
                .iter()
                .map(crate::ui::dialog::DialogAction::spec)
                .collect::<Vec<ActionSpec>>(),
            default_action: None,
        })
    }

    /// The focused pane has conjoined tool children: Cmd+Down zooms them.
    pub(crate) fn focused_has_conjoined_children(&self) -> bool {
        self.visual_owner_pane().is_some_and(|parent| {
            self.conjoined_contexts.iter().any(|(context, id)| {
                *id == parent
                    && self
                        .windows
                        .iter()
                        .any(|window| window.context_id == *context && !window.panes.is_empty())
            })
        })
    }

    fn pane_instance(&self, pane: u64) -> Option<PaneInstance> {
        self.windows.iter().find_map(|window| {
            let terminal = window.panes.get(&pane)?.as_terminal()?;
            Some(PaneInstance {
                window_id: window.window_id,
                pane,
                endpoint: terminal.backend.instance(),
            })
        })
    }

    fn router_snapshot(
        &self,
        ctx: &egui::Context,
        visible: bool,
        os_focused: bool,
    ) -> Snapshot<'_> {
        let keyboard = match self.host_input_owner() {
            crate::app::input_owner::InputOwner::Pane(pane) => self
                .pane_instance(pane)
                .map_or(KeyboardOwner::Legacy, KeyboardOwner::Terminal),
            _ => KeyboardOwner::Legacy,
        };
        let dialog = self.router_dialog();
        Snapshot {
            visible,
            os_focused,
            legacy_overlay: dialog.is_none()
                && (self.overlay_surface().is_some() || egui::Popup::is_any_open(ctx)),
            dialog,
            keyboard,
            table: &self.binding_table,
            gates: super::host_router::binding::BindingGates {
                overlay_open: false,
                pane_selection: self
                    .host
                    .pane_selection(self.windows[self.active_window].window_id)
                    .is_some(),
                conjoined_zoom: self.focused_has_conjoined_children(),
            },
            dialog_presentation: self.router_dialog_presentation.clone(),
        }
    }

    /// Ingress (stint 1659): stamp the native delivery and each queued pane
    /// IPC batch as their own deliveries, route them, and leave egui only
    /// the legacy run. Runs before egui folds anything.
    pub(super) fn route_raw_input(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        let visible = raw_input.viewport().visible() != Some(false);
        let os_focused = raw_input.viewport().focused.unwrap_or(true);
        let now = std::time::Instant::now();
        let stamp = |router: &mut super::host_router::HostRouter,
                     events: Vec<egui::Event>,
                     source: Source| {
            let delivery = router.next_delivery();
            events
                .into_iter()
                .enumerate()
                .map(|(ordinal, event)| Received {
                    id: super::host_router::EventId {
                        delivery,
                        ordinal: ordinal as u32,
                    },
                    source,
                    received_at: now,
                    event,
                })
                .collect::<Vec<_>>()
        };
        let mut deliveries = vec![stamp(
            &mut self.host_router,
            std::mem::take(&mut raw_input.events),
            Source::Native,
        )];
        let mut injected_modifiers = None;
        // A hidden window has no visible pass to act in: queued pane input
        // stays queued for the next visible frame.
        if visible {
            for batch in self.take_injected_batches(ctx) {
                injected_modifiers = Some(batch.modifiers);
                deliveries.push(stamp(&mut self.host_router, batch.events, Source::Injected));
            }
        }
        let mut router = std::mem::take(&mut self.host_router);
        let snapshot = self.router_snapshot(ctx, visible, os_focused);
        let routed = router.route(&snapshot, deliveries);
        drop(snapshot);
        self.host_router = router;
        if let Some(modifiers) = injected_modifiers {
            // This frame took socket-injected input; legacy consumers read
            // the frame-wide flag, the router reads each event's source.
            self.synthetic_input_frame = true;
            if routed
                .egui
                .iter()
                .any(|event| event.source == Source::Injected)
            {
                raw_input.modifiers = modifiers;
            }
        }
        raw_input.events = routed.egui.into_iter().map(|event| event.event).collect();
        if routed.held {
            ctx.request_repaint();
        }
    }

    /// Queued pane IPC input for the focused pane, under the existing
    /// visibility and text-focus policy for that exact pane.
    fn take_injected_batches(&mut self, ctx: &egui::Context) -> Vec<egui::RawInput> {
        let focused_pane = self.windows.get(self.active_window).and_then(|window| {
            window
                .focused_pane
                .and_then(|tile_id| window.tree.tiles.get(tile_id))
                .and_then(|tile| match tile {
                    egui_tiles::Tile::Pane(pane_id) => Some(*pane_id),
                    egui_tiles::Tile::Container(_) => None,
                })
        });
        let Some(pane_id) = focused_pane else {
            return Vec::new();
        };
        let mut batches = Vec::new();
        let defer_text_until_focused =
            self.pending_pane_inputs
                .get(&pane_id)
                .is_some_and(|batches| {
                    batches.iter().any(|batch| {
                        batch
                            .events
                            .iter()
                            .any(|event| matches!(event, egui::Event::Text(_)))
                    })
                })
                && !crate::app::input_owner::pane_receives_ipc_text(ctx, pane_id);
        if defer_text_until_focused {
            // A pane can be created while eframe runs hidden logic-only
            // passes. Its first visible pass has not rendered a text surface
            // yet, so feeding queued text into that pass loses it before the
            // post-frame reconciler grants default focus. Preserve the
            // ordered batch until that focus exists.
            log::info!(
                "pane_ipc: deferring text input for pane_id={pane_id} until its text surface receives focus"
            );
        } else if let Some(queued) = self.pending_pane_inputs.remove(&pane_id) {
            batches.extend(queued);
        }
        // Drag pacing (stint 0510): exactly ONE pointer batch per visible
        // frame, so a drag arrives as a genuine multi-frame trajectory
        // instead of a same-frame click.
        if let Some(frames) = self.pending_pane_pointer_frames.get_mut(&pane_id) {
            if let Some(batch) = frames.pop_front() {
                batches.push(batch);
            }
            if frames.is_empty() {
                self.pending_pane_pointer_frames.remove(&pane_id);
            } else {
                // Keep frames coming until the schedule drains.
                ctx.request_repaint();
            }
        }
        batches
    }

    /// Execute the router's commands in order (stint 1659). Runs early in
    /// `App::logic`, so hidden passes drain too and nothing waits for paint.
    pub(super) fn drain_routed_input(&mut self, ctx: &egui::Context) {
        for command in self.host_router.take_commands() {
            match command {
                RoutedCommand::Terminal {
                    target,
                    first,
                    events,
                } => self.apply_terminal_input(ctx, target, first, events),
                RoutedCommand::HostAction {
                    id,
                    action,
                    target,
                    event,
                } => {
                    let eligible = target.is_none_or(|target| {
                        self.routed_terminal_mut(target).is_some()
                            && self.host_input_owner()
                                == crate::app::input_owner::InputOwner::Pane(target.pane)
                    });
                    if eligible {
                        log::info!(
                            "host_router: executing {action:?} for delivery={} ordinal={}",
                            event.delivery.0,
                            event.ordinal
                        );
                        self.dispatch_host_action(action, ctx);
                        // An action that closed or replaced its own pane
                        // retired that recipient: the suffix admitted for it
                        // must not reach whichever pane is revealed.
                        let retired =
                            target.is_some_and(|target| self.routed_terminal_mut(target).is_none());
                        if retired {
                            log::info!(
                                "host_router: action retired its target pane; its suffix retires"
                            );
                        }
                        self.host_router.resolve(
                            id,
                            if retired {
                                Outcome::Failed
                            } else {
                                Outcome::Done
                            },
                        );
                    } else {
                        log::info!(
                            "host_router: {action:?} target {target:?} no longer owns input; not executed"
                        );
                        self.host_router.resolve(id, Outcome::Failed);
                    }
                }
                RoutedCommand::DialogDecision {
                    id,
                    opening,
                    action,
                    event,
                } => {
                    self.decide_close_confirm(opening, &action, event);
                    self.host_router.resolve(id, Outcome::Done);
                }
            }
        }
    }

    fn routed_terminal_mut(
        &mut self,
        target: PaneInstance,
    ) -> Option<&mut crate::host::pane::TerminalPane> {
        let window = self
            .windows
            .iter_mut()
            .find(|window| window.window_id == target.window_id)?;
        let terminal = window.panes.get_mut(&target.pane)?.as_terminal_mut()?;
        (terminal.backend.instance() == target.endpoint).then_some(terminal)
    }

    /// Queue routed keyboard input on its pane, in delivery order. The
    /// pane's `egui_term` widget applies it when it next paints, where the
    /// encoder, search and copy mode live; an exited or standby terminal
    /// handles it there too.
    fn apply_terminal_input(
        &mut self,
        ctx: &egui::Context,
        target: PaneInstance,
        first: super::host_router::EventId,
        events: Vec<egui::Event>,
    ) {
        let Some(terminal) = self.routed_terminal_mut(target) else {
            log::info!(
                "host_router: pane {} endpoint {} gone; {} routed events retired (delivery={} ordinal={})",
                target.pane,
                target.endpoint,
                events.len(),
                first.delivery.0,
                first.ordinal
            );
            return;
        };
        terminal.routed_input.extend(events);
        ctx.request_repaint();
    }

    /// Answer one opening of the pane-close confirmation. A decision for an
    /// opening that is no longer current (replaced, closed, reopened) makes
    /// no decision.
    fn decide_close_confirm(
        &mut self,
        opening: super::host_router::OpeningId,
        action: &str,
        event: super::host_router::EventId,
    ) {
        let current = self.router_dialog().map(|dialog| dialog.opening);
        if current != Some(opening) {
            log::info!(
                "close_confirm: {action} for opening {} ignored; current opening {:?} (delivery={} ordinal={})",
                opening.0,
                current.map(|o| o.0),
                event.delivery.0,
                event.ordinal
            );
            return;
        }
        log::info!(
            "close_confirm: opening {} decided {action} (delivery={} ordinal={})",
            opening.0,
            event.delivery.0,
            event.ordinal
        );
        match action {
            "close" => {
                self.pending_close = false;
                if !self.confirm_close_batch_pane() {
                    self.confirm_single_close();
                }
                self.close_confirm_target = None;
            }
            _ => {
                self.pending_close = false;
                self.close_confirm_target = None;
            }
        }
        self.sync_confirm_close_focus();
    }

    /// Publish what the router-owned dialog presented this pass.
    pub(crate) fn publish_dialog_presentation(
        &mut self,
        opening: super::host_router::OpeningId,
        presented: Option<crate::ui::dialog::PresentedDialog>,
    ) {
        if let Some(presented) = presented {
            self.router_dialog_presentation = Some(DialogPresentation {
                opening,
                panel: presented.panel,
                buttons: presented.buttons,
            });
        }
    }
}
