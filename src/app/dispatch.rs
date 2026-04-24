//! App command dispatch — keyboard routing + command drain from app panes.

use crate::app_trait::AppCommand;

use super::PlexiApp;

impl PlexiApp {
    /// Feed keyboard input to the focused app pane. Keys only go to the
    /// focused pane (that's the whole point of focus). Command drain
    /// happens separately via [`drain_all_app_commands`] so background
    /// apps can surface notifications and other commands while the
    /// focused pane is something else (or while a modal holds focus).
    pub(super) fn dispatch_app_key_events(&mut self, ctx: &egui::Context) {
        let active = self.active_context;
        let Some(focused_tile) = self.contexts[active].focused_pane else {
            return;
        };
        let Some(egui_tiles::Tile::Pane(pane_id)) =
            self.contexts[active].tree.tiles.get(focused_tile)
        else {
            return;
        };
        let pane_id = *pane_id;
        let Some(pane) = self.contexts[active].panes.get_mut(&pane_id) else {
            return;
        };
        let Some(app_pane) = pane.as_app_mut() else {
            return;
        };
        ctx.input(|i| {
            app_pane.runtime.handle_key(i);
        });
    }

    /// Drain `pending_commands` from **every** app pane in **every** context,
    /// stamp sender pane ids and source_context indices, and return the
    /// consolidated list.
    ///
    /// Must run every frame regardless of focus, modal state, or any
    /// overlay. Draining all contexts (not just the active one) ensures
    /// that background apps — e.g. stand-up-reminder in a non-active context
    /// — can surface Global-scope notifications while the user is elsewhere.
    pub(super) fn drain_all_app_commands(&mut self) -> Vec<AppCommand> {
        // Collect (ctx_idx, pane_id, commands) first so we don't hold
        // aliasing mutable borrows across pushes in the loop body.
        let mut per_pane: Vec<(usize, u64, Vec<AppCommand>)> = Vec::new();
        for (ctx_idx, context) in self.contexts.iter_mut().enumerate() {
            for (pane_id, pane) in context.panes.iter_mut() {
                let Some(app_pane) = pane.as_app_mut() else { continue };
                let cmds = app_pane.runtime.take_pending_commands();
                if !cmds.is_empty() {
                    per_pane.push((ctx_idx, *pane_id, cmds));
                }
            }
        }

        let mut deferred = Vec::new();
        for (ctx_idx, pane_id, cmds) in per_pane {
            for cmd in cmds {
                match cmd {
                    AppCommand::SpawnApp { .. }
                    | AppCommand::DeliverPipeMessage { .. }
                    | AppCommand::DeliverRunUpdate { .. } => deferred.push(cmd),
                    AppCommand::CdRequest { cwd, .. } => {
                        deferred.push(AppCommand::CdRequest { cwd, sender_pane_id: pane_id });
                    }
                    AppCommand::Notify(msg) => {
                        log::info!("app notify: {msg}");
                    }
                    AppCommand::ShowNotification {
                        notify_id,
                        level,
                        title,
                        body,
                        kind,
                        options,
                        input_prompt,
                        required,
                        priority,
                        scope,
                        ..
                    } => {
                        deferred.push(AppCommand::ShowNotification {
                            notify_id,
                            sender_pane_id: pane_id,
                            // Stamp the originating context so the host can
                            // filter by scope (Context-scoped notifs are only
                            // visible in their source context; Global ones always).
                            source_context: ctx_idx,
                            level,
                            title,
                            body,
                            kind,
                            options,
                            input_prompt,
                            required,
                            priority,
                            scope,
                        });
                    }
                    AppCommand::DeliverNotifyAction { .. } => deferred.push(cmd),
                }
            }
        }

        deferred
    }
}
