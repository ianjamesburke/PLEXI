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
        // Collect (ctx_idx, pane_id, type_id, commands) per pane. We capture
        // type_id here because the manifest-declared notification scope is
        // looked up from the registry by type_id, and we can't hold a
        // mutable borrow on contexts + a shared borrow on the registry at
        // the same time during the match below.
        let active = self.active_context;
        let mut per_pane: Vec<(usize, u64, String, Vec<AppCommand>)> = Vec::new();
        for (ctx_idx, context) in self.contexts.iter_mut().enumerate() {
            for (pane_id, pane) in context.panes.iter_mut() {
                let Some(app_pane) = pane.as_app_mut() else { continue };
                // Active-context panes are already fully updated by ui() this frame.
                // Non-active panes need a headless tick so their timer/async events
                // reach the subprocess and control commands (notifications, etc.) flow out.
                if ctx_idx != active {
                    app_pane.runtime.background_tick();
                }
                let type_id = app_pane.runtime.type_id().to_string();
                let cmds = app_pane.runtime.take_pending_commands();
                if !cmds.is_empty() {
                    per_pane.push((ctx_idx, *pane_id, type_id, cmds));
                }
            }
        }

        let mut deferred = Vec::new();
        for (ctx_idx, pane_id, type_id, cmds) in per_pane {
            // Scope is a per-app user-facing policy declared in the app's
            // manifest.toml. Apps never set it — the host resolves it once
            // per notification here. Defaults to `Context` when the manifest
            // omits the field (safe default: don't interrupt across contexts).
            let resolved_scope = self
                .registry
                .default_notification_scope_for(&type_id);
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
                        ..
                    } => {
                        deferred.push(AppCommand::ShowNotification {
                            notify_id,
                            sender_pane_id: pane_id,
                            source_context: ctx_idx,
                            level,
                            title,
                            body,
                            kind,
                            options,
                            input_prompt,
                            required,
                            priority,
                            scope: resolved_scope,
                        });
                    }
                    AppCommand::DeliverNotifyAction { .. } => deferred.push(cmd),
                }
            }
        }

        deferred
    }
}
