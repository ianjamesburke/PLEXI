//! App command dispatch — keyboard routing + command drain from app panes.

use crate::app_trait::{App, AppCommand};

use super::PlexiApp;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_permissions::AppPermissions;
    use crate::app_protocol::{NotifyKind, NotifyScope};
    use crate::process_app::ProcessApp;
    use crate::testing::HostHarness;
    /// Regression guard for issue #879: parked background app notifications
    /// hardcoded `source_context: 0`. Verify that the context index recorded
    /// at park time propagates to the `ShowNotification` command.
    #[test]
    fn parked_app_notification_uses_park_context() {
        let mut h = HostHarness::new();

        // Seed a ShowNotification on a fresh ProcessApp's pending_commands,
        // then insert it into background_apps with park_ctx = 2.
        let (mut process_app, _draw_tx) =
            ProcessApp::new_for_test(999, AppPermissions::builtin());

        process_app.pending_commands.push(AppCommand::ShowNotification {
            notify_id: "test-notify".to_string(),
            sender_pane_id: 0,
            source_context: 0, // pre-filled stub value; drain should overwrite
            level: "info".to_string(),
            title: "hello from context 2".to_string(),
            body: String::new(),
            kind: NotifyKind::Message,
            options: vec![],
            input_prompt: None,
            required: false,
            priority: 0,
            scope: NotifyScope::Context,
            image_inline: None,
            image_pipe_id: None,
            timeout_secs: None,
            on_dismiss: None,
        });

        let park_ctx: usize = 2;
        h.app
            .background_apps
            .insert("test-app".to_string(), (park_ctx, Box::new(process_app)));

        let cmds = h.app.drain_all_app_commands();

        let notification = cmds.iter().find(|c| {
            matches!(c, AppCommand::ShowNotification { notify_id, .. } if notify_id == "test-notify")
        });

        let Some(AppCommand::ShowNotification { source_context, .. }) = notification else {
            panic!("expected ShowNotification in drained commands — not found");
        };

        assert_eq!(
            *source_context, park_ctx,
            "source_context should be the park-time context ({park_ctx}), not hardcoded 0"
        );
    }
}

impl PlexiApp {
    /// Feed keyboard input to the focused app pane. Keys only go to the
    /// focused pane (that's the whole point of focus). Command drain
    /// happens separately via [`drain_all_app_commands`] so background
    /// apps can surface notifications and other commands while the
    /// focused pane is something else (or while a modal holds focus).
    pub(super) fn dispatch_app_key_events(&mut self, ctx: &egui::Context) {
        let active = self.active_window;
        let Some(focused_tile) = self.windows[active].focused_pane else {
            return;
        };
        let Some(egui_tiles::Tile::Pane(pane_id)) =
            self.windows[active].tree.tiles.get(focused_tile)
        else {
            return;
        };
        let pane_id = *pane_id;
        let Some(pane) = self.windows[active].panes.get_mut(&pane_id) else {
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
    /// Parked background apps (pane closed, process alive) are also ticked so
    /// their timers and notifications keep firing while detached.
    pub(super) fn drain_all_app_commands(&mut self) -> Vec<AppCommand> {
        // Collect (ctx_idx, pane_id, type_id, commands) per pane. We capture
        // type_id here because the manifest-declared notification scope is
        // looked up from the registry by type_id, and we can't hold a
        // mutable borrow on contexts + a shared borrow on the registry at
        // the same time during the match below.
        let active = self.active_window;
        let mut per_pane: Vec<(usize, u64, String, Vec<AppCommand>)> = Vec::new();
        for (ctx_idx, context) in self.windows.iter_mut().enumerate() {
            for (pane_id, pane) in context.panes.iter_mut() {
                if let Some(app_pane) = pane.as_app_mut() {
                    // Active-context panes are already fully updated by ui()
                    // this frame. Non-active panes need a headless tick so
                    // timer/async events reach the subprocess and control
                    // commands flow out.
                    if ctx_idx != active {
                        app_pane.runtime.background_tick();
                    }
                    let type_id = app_pane.runtime.type_id().to_string();
                    let cmds = app_pane.runtime.take_pending_commands();
                    if !cmds.is_empty() {
                        per_pane.push((ctx_idx, *pane_id, type_id, cmds));
                    }
                    continue;
                }
            }
        }

        // Tick parked background apps (process alive, no visible pane).
        // Mirrors the headless tick above but operates on self.background_apps
        // instead of context panes. Without this, timers stop and notifications
        // never fire the moment the pane is closed.
        // Collect (type_id, park_context_idx, commands) for each parked app.
        let parked: Vec<(String, usize, Vec<AppCommand>)> = self
            .background_apps
            .iter_mut()
            .map(|(type_id, (park_ctx, app))| {
                app.background_tick();
                let cmds = app.take_pending_commands();
                (type_id.to_owned(), *park_ctx, cmds)
            })
            .collect();

        let mut deferred = Vec::new();

        for (type_id, park_ctx, cmds) in &parked {
            let resolved_scope = self.registry.default_notification_scope_for(type_id);
            for cmd in cmds.iter() {
                match cmd {
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
                        image_inline,
                        image_pipe_id,
                        timeout_secs,
                        on_dismiss,
                        ..
                    } => {
                        log::info!(
                            "parked background app '{}' notification: {} (routing to context {})",
                            type_id, title, park_ctx
                        );
                        deferred.push(AppCommand::ShowNotification {
                            notify_id: notify_id.clone(),
                            sender_pane_id: 0, // no live pane — tombstone won't fire
                            source_context: *park_ctx,
                            level: level.clone(),
                            title: title.clone(),
                            body: body.clone(),
                            kind: kind.clone(),
                            options: options.clone(),
                            input_prompt: input_prompt.clone(),
                            required: *required,
                            priority: priority.clone(),
                            scope: resolved_scope,
                            image_inline: image_inline.clone(),
                            image_pipe_id: image_pipe_id.clone(),
                            timeout_secs: *timeout_secs,
                            on_dismiss: on_dismiss.clone(),
                        });
                    }
                    AppCommand::Notify(msg) => {
                        log::info!("parked bg app '{}': {}", type_id, msg);
                    }
                    // Commands that require a live pane context are silently
                    // dropped when the app is parked — the app has no tile to
                    // target and no context to route through.
                    _ => {}
                }
            }
        }

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
                    | AppCommand::SpawnPane { .. }
                    | AppCommand::DeliverPipeMessage { .. }
                    | AppCommand::OpenDirectedPipe { .. }
                    | AppCommand::DeliverRunUpdate { .. }
                    // Canvas Terminal Binding Primitives (#78) — sender_pane_id
                    // is already populated by `route_command` (the originating
                    // app knows its own pane_id). The dispatch site below
                    // mutates the pane tree, so they defer like SpawnApp.
                    | AppCommand::RequestLinkedTerminal { .. }
                    | AppCommand::RunInLinkedTerminal { .. }
                    | AppCommand::InsertPathToken { .. }
                    | AppCommand::RequestCommandPreview { .. }
                    | AppCommand::OpenArtifact { .. } => deferred.push(cmd),
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
                        image_inline,
                        image_pipe_id,
                        timeout_secs,
                        on_dismiss,
                        ..
                    } => {
                        let ws_idx = self.windows.get(ctx_idx)
                            .and_then(|w| self.router.position(|c| c.context_id == w.context_id))
                            .unwrap_or(0);
                        deferred.push(AppCommand::ShowNotification {
                            notify_id,
                            sender_pane_id: pane_id,
                            source_context: ws_idx,
                            level,
                            title,
                            body,
                            kind,
                            options,
                            input_prompt,
                            required,
                            priority,
                            scope: resolved_scope,
                            image_inline,
                            image_pipe_id,
                            timeout_secs,
                            on_dismiss,
                        });
                    }
                    AppCommand::DeliverNotifyAction { .. } => deferred.push(cmd),
                }
            }
        }

        deferred
    }
}
