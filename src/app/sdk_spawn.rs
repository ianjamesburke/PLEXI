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

/// How a view-requested terminal starts: its command and placement.
struct ViewTerminalShape {
    vertical: bool,
    new_pane_first: bool,
    command: Option<crate::pane_ops::construct::LaunchCommand>,
    keep_focus: bool,
}

impl ViewTerminalShape {
    /// SDK `spawn_pane`: a single argument is a command line, the terminal
    /// closes when its command exits, and the requester keeps focus.
    fn spawn_pane(layout: &str, args: &[String]) -> Self {
        let command = super::cmd_from_args(args);
        Self {
            vertical: matches!(layout, "split_h" | "split_right" | "split_left"),
            new_pane_first: matches!(layout, "split_above" | "split_left"),
            command: command.map(|command| crate::pane_ops::construct::LaunchCommand {
                command,
                close_on_exit: true,
            }),
            keep_focus: true,
        }
    }

    /// Legacy `SpawnView("terminal")` and a component's `Spawn`: `args` is
    /// argv, the terminal stays open after its command and takes focus.
    fn legacy(layout: Option<&str>, args: &[String]) -> Self {
        let layout = layout.unwrap_or("split_h");
        Self {
            vertical: !matches!(layout, "split_v" | "split_below" | "split_above"),
            new_pane_first: matches!(layout, "split_above" | "split_left"),
            command: (!args.is_empty()).then(|| crate::pane_ops::construct::LaunchCommand {
                command: crate::host::shell::shell_join(args),
                close_on_exit: false,
            }),
            keep_focus: false,
        }
    }
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

    /// The live view instance `emitter` names, in its slot or covered by an
    /// overlay there; `None` once that instance has ended.
    pub(crate) fn view_instance_mut(&mut self, emitter: ViewEmitter) -> Option<&mut ViewPane> {
        self.windows
            .iter_mut()
            .find_map(|w| w.panes.get_mut(&emitter.pane_id))
            .and_then(|pane| emitter_in(pane, emitter))
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
        match self.view_instance_mut(emitter) {
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

    /// The validated destination window of a view's launch: the named
    /// target context's window, which must be the requester's own context or
    /// a descendant of it (#1518), or else the requester's own window. Never
    /// whichever window is active.
    fn view_destination(
        &mut self,
        emitter: ViewEmitter,
        target_context: Option<u64>,
    ) -> Result<usize, String> {
        let requester_win = self
            .emitter_window(emitter)
            .ok_or_else(|| "the requesting view is no longer open".to_string())?;
        let requester_context_id = self.windows[requester_win].context_id;
        let Some(target_ctx_id) = target_context else {
            return Ok(requester_win);
        };
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
        Ok(win_idx)
    }

    /// Install a terminal a view asked for, in its validated `destination`
    /// window, admitted as that view's (stint 1666). The anchor resolves only
    /// inside the destination, so the credential scope is the destination's.
    fn spawn_view_terminal(
        &mut self,
        emitter: ViewEmitter,
        destination: usize,
        from_pane_id: Option<u64>,
        shape: ViewTerminalShape,
    ) -> Result<u64, String> {
        let destination_ctx = self.windows[destination].context_id;
        let target_tile = match from_pane_id {
            Some(from_id) => self.windows[destination]
                .tree
                .tiles
                .find_pane(&from_id)
                .ok_or_else(|| {
                    format!("origin pane {from_id} is not in destination context {destination_ctx}")
                })?,
            None => self.windows[destination]
                .focused_pane
                .or(self.windows[destination].tree.root)
                .ok_or("the target window is empty")?,
        };
        log::info!(
            "SpawnPane: terminal for emitter={emitter:?} destination context={destination_ctx} anchor={from_pane_id:?} keep_focus={}",
            shape.keep_focus
        );
        // Directory as the anchor shows it now: this spawn is admitted and
        // applied in this turn.
        let cwd = self.windows[destination].get_focused_pane_cwd(target_tile);
        let context = self.pane_context_env_for_window(destination);
        let launch = self.plugin_terminal_launch(emitter.pane_id, context, cwd, shape.command);
        self.spawn_terminal_pane_at(
            crate::pane_ops::create::SplitPlacement {
                win_idx: destination,
                target_tile,
                vertical: shape.vertical,
                new_pane_first: shape.new_pane_first,
                keep_focus: shape.keep_focus,
            },
            launch,
        )
    }

    /// Serve a legacy view terminal request (`SpawnView("terminal")` or a
    /// component's `Spawn` effect) in the requester's own context, admitted
    /// as that view's, with the legacy argv and launch behavior.
    pub(crate) fn serve_legacy_view_terminal(
        &mut self,
        emitter: Option<ViewEmitter>,
        layout: Option<&str>,
        args: &[String],
    ) -> Result<u64, String> {
        let emitter = emitter.ok_or_else(|| "the request carried no emitting view".to_string())?;
        let destination = self.view_destination(emitter, None)?;
        let outcome = self.spawn_view_terminal(
            emitter,
            destination,
            None,
            ViewTerminalShape::legacy(layout, args),
        );
        match &outcome {
            Ok(pane_id) => {
                log::info!("SpawnView: terminal pane_id={pane_id} for emitter={emitter:?}")
            }
            Err(error) => {
                log::warn!("SpawnView: terminal for emitter={emitter:?} not created: {error}")
            }
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
        let destination = self.view_destination(emitter, target_context)?;
        if type_id == "terminal" {
            // "terminal" is a builtin pane type, not in the plugin registry.
            return self.spawn_view_terminal(
                emitter,
                destination,
                from_pane_id,
                ViewTerminalShape::spawn_pane(layout, &args),
            );
        }
        if target_context.is_some() {
            self.active_window = destination;
        }
        let active = self.active_window;
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
