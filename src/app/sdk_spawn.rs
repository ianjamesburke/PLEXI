//! An SDK view's `spawn_pane` request and its one reply (stint 1632).
//!
//! The reply goes to the view that emitted the request, stamped when the
//! host drained that view's commands. Target context, target focus and
//! `from_pane_id` choose where a pane lands; none of them chooses who hears
//! about it. Every outcome, including an invalid or empty destination,
//! produces exactly one `PaneSpawned` or `PaneSpawnError`.

use super::view_trait::ViewEmitter;
use super::PlexiApp;
use crate::host::pane::{Pane, ViewPane};
use crate::protocol::PlexiEvent;

/// The emitting view instance in `pane`, in the slot or under an overlay
/// that covers it.
fn emitter_in(pane: &mut Pane, emitter: ViewEmitter) -> Option<&mut ViewPane> {
    let view = pane.as_view_mut()?;
    if view.incarnation == emitter.incarnation {
        return Some(view);
    }
    view.overlay_replaced
        .as_deref_mut()
        .and_then(|covered| emitter_in(covered, emitter))
}

impl PlexiApp {
    /// Drop the requests a view instance left held (behind an overlay) when
    /// that instance ends: they have no requester to act for or answer.
    pub(crate) fn retire_commands_from(&mut self, emitter: ViewEmitter) {
        use super::view_trait::ViewCommand;
        let before = self.overlay_held_cmds.len();
        self.overlay_held_cmds.retain(|cmd| {
            !matches!(cmd,
                ViewCommand::SpawnPane { emitter: Some(e), .. }
                | ViewCommand::SpawnView { emitter: Some(e), .. }
                | ViewCommand::WasmHostEffect { emitter: Some(e), .. } if *e == emitter)
        });
        let retired = before - self.overlay_held_cmds.len();
        if retired > 0 {
            log::info!("sdk_reply: retired {retired} held request(s) of ended emitter {emitter:?}");
        }
    }

    /// The window holding the emitting view instance, if it is still open.
    fn emitter_window(&mut self, emitter: ViewEmitter) -> Option<usize> {
        self.windows.iter_mut().position(|w| {
            w.panes
                .get_mut(&emitter.pane_id)
                .is_some_and(|pane| emitter_in(pane, emitter).is_some())
        })
    }

    /// Deliver `event` to the view instance that emitted a request. A
    /// requester that has gone, even if another view now occupies its slot,
    /// records that disposition and nothing else receives it.
    pub(crate) fn deliver_to_emitter(&mut self, emitter: Option<ViewEmitter>, event: PlexiEvent) {
        let Some(emitter) = emitter else {
            log::warn!("sdk_reply: request carried no emitting view; {event:?} undelivered");
            return;
        };
        let view = self
            .windows
            .iter_mut()
            .find_map(|w| w.panes.get_mut(&emitter.pane_id))
            .and_then(|pane| emitter_in(pane, emitter));
        match view {
            Some(view) => {
                log::info!("sdk_reply: delivered to emitter {emitter:?}");
                view.runtime.queue_outbound_event(event);
            }
            None => log::info!(
                "sdk_reply: emitter {emitter:?} is gone; reply {event:?} has no recipient"
            ),
        }
    }

    pub(crate) fn reply_sdk_spawn_pane(
        &mut self,
        emitter: Option<ViewEmitter>,
        request_id: Option<String>,
        outcome: Result<u64, String>,
    ) {
        let event = match outcome {
            Ok(pane_id) => PlexiEvent::PaneSpawned {
                pane_id,
                request_id,
            },
            Err(reason) => PlexiEvent::PaneSpawnError { reason, request_id },
        };
        self.deliver_to_emitter(emitter, event);
    }

    /// Serve one SDK `spawn_pane`: the installed pane's id, or why none was.
    pub(crate) fn serve_sdk_spawn_pane(
        &mut self,
        emitter: Option<ViewEmitter>,
        type_id: &str,
        layout: &str,
        args: Vec<String>,
        from_pane_id: Option<u64>,
        target_context: Option<u64>,
    ) -> Result<u64, String> {
        // "background" layout is not yet implemented (blocked on #291).
        if layout == "background" {
            log::warn!("SpawnPane: layout='background' not implemented, rejected");
            return Err("layout 'background' not yet implemented".to_string());
        }
        let original_active_window = self.active_window;
        let Some(emitter) = emitter else {
            return Err("the request carried no emitting view".to_string());
        };
        let outcome =
            self.spawn_from_sdk(emitter, type_id, layout, args, from_pane_id, target_context);
        // Restore the active window if the spawn switched it for placement.
        if original_active_window < self.windows.len() {
            self.active_window = original_active_window;
        }
        match &outcome {
            Ok(pane_id) => log::info!(
                "SpawnPane: launched '{}' pane_id={pane_id} for emitter={emitter:?}",
                crate::host::guest_text::shown(type_id)
            ),
            Err(error) => log::warn!(
                "SpawnPane: '{}' for emitter={emitter:?} not created: {error}",
                crate::host::guest_text::shown(type_id)
            ),
        }
        outcome
    }

    fn spawn_from_sdk(
        &mut self,
        emitter: ViewEmitter,
        type_id: &str,
        layout: &str,
        args: Vec<String>,
        from_pane_id: Option<u64>,
        target_context: Option<u64>,
    ) -> Result<u64, String> {
        // The requester's own context bounds `target_context` (#1518): the
        // emitting view's window, never whichever window is active.
        let requester_win = self
            .emitter_window(emitter)
            .ok_or_else(|| "the requesting view is no longer open".to_string())?;
        let requester_context_id = self.windows[requester_win].context_id;
        let mut destination = requester_win;
        if let Some(target_ctx_id) = target_context {
            let target_exists = self.router.iter().any(|c| c.context_id == target_ctx_id);
            let is_descendant = self
                .host
                .ancestors_of(target_ctx_id)
                .contains(&requester_context_id);
            if !target_exists || (!is_descendant && requester_context_id != target_ctx_id) {
                return Err(format!(
                    "target_context {target_ctx_id} is not a descendant of context {requester_context_id}"
                ));
            }
            let win_idx = self
                .windows
                .iter()
                .position(|w| w.context_id == target_ctx_id)
                .ok_or_else(|| format!("target_context {target_ctx_id} has no window"))?;
            log::info!("SpawnPane: target_context={target_ctx_id} -> window index {win_idx}");
            self.active_window = win_idx;
            destination = win_idx;
        }
        let active = self.active_window;
        if type_id == "terminal" {
            // "terminal" is a builtin pane type, not in the plugin registry.
            let vertical = matches!(layout, "split_h" | "split_right" | "split_left");
            let new_pane_first = matches!(layout, "split_above" | "split_left");
            let initial_cmd = super::cmd_from_args(&args);
            // An SDK-spawned terminal with a command closes on exit.
            let close_on_exit = initial_cmd.is_some();
            // The validated destination is the named target context's
            // window, or else the requester's own; never the active window.
            // The anchor resolves only inside it, so the credential scope is
            // the destination's (stint 1666).
            let target_win = destination;
            let destination_ctx = self.windows[target_win].context_id;
            let target_tile = match from_pane_id {
                Some(from_id) => self.windows[target_win]
                    .tree
                    .tiles
                    .find_pane(&from_id)
                    .ok_or_else(|| {
                        format!(
                            "origin pane {from_id} is not in destination context {destination_ctx}"
                        )
                    })?,
                None => self.windows[target_win]
                    .focused_pane
                    .or(self.windows[target_win].tree.root)
                    .ok_or("the target window is empty")?,
            };
            log::info!(
                "SpawnPane: terminal for emitter={emitter:?} destination context={destination_ctx} anchor={from_pane_id:?}"
            );
            // Directory as the anchor shows it now: this spawn is admitted
            // and applied in this turn. The coordinator keeps focus.
            let cwd = self.windows[target_win].get_focused_pane_cwd(target_tile);
            let context = self.pane_context_env_for_window(target_win);
            let launch = self.plugin_terminal_launch(
                emitter.pane_id,
                context,
                cwd,
                initial_cmd.map(|command| crate::pane_ops::construct::LaunchCommand {
                    command,
                    close_on_exit,
                }),
            );
            return self.spawn_terminal_pane_at(
                crate::pane_ops::create::SplitPlacement {
                    win_idx: target_win,
                    target_tile,
                    vertical,
                    new_pane_first,
                    keep_focus: true,
                },
                launch,
            );
        }
        if let Some(from_id) = from_pane_id {
            // With a target context, `from_pane_id` resolves only inside its
            // window so it cannot override the target.
            let tile = if target_context.is_some() {
                self.windows[active]
                    .tree
                    .tiles
                    .find_pane(&from_id)
                    .map(|tile| (active, tile))
            } else {
                self.find_pane_in_any_window(from_id)
            };
            let (win, tile) = tile.ok_or_else(|| format!("origin pane {from_id} not found"))?;
            self.active_window = win;
            self.set_window_focused_pane(win, tile);
        }
        let predicted = self.host.next_pane_id();
        match self.launch_view_by_id_with_layout(type_id, Some(layout.to_string()), &args, None)? {
            // Dedup focused a live instance; report its real id.
            Some(existing) => Ok(existing),
            None => Ok(predicted),
        }
    }
}
