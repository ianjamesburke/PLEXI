//! App command dispatch — keyboard routing + command drain from app panes.

use crate::app::app_trait::AppCommand;

use super::PlexiApp;

impl PlexiApp {
    /// Feed keyboard input to the focused app pane. Keys only go to the
    /// focused pane (that's the whole point of focus). Command drain
    /// happens separately via [`drain_all_app_commands`] so background
    /// apps can surface notifications and other commands while the
    /// focused pane is something else (or while a modal holds focus).
    pub(super) fn dispatch_app_key_events(&mut self, ctx: &egui::Context) {
        // Block key delivery when the OS has focus elsewhere. `active_window` is not
        // cleared on blur, so without this guard keys route to the last-active pane
        // even while a different app (or Plexi window) owns the keyboard.
        if !ctx.input(|i| i.viewport().focused.unwrap_or(true)) {
            return;
        }
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
        // Capture whether the app consumed the key. If it did, also consume
        // Escape from the InputState so poll_actions can't re-fire CloseApp
        // for keys the app already handled (e.g. Escape exiting search mode
        // in the file browser rather than closing the pane).
        let disposition = ctx.input(|i| app_pane.runtime.handle_key(i));
        if disposition == crate::app::app_trait::KeyDisposition::Consumed {
            ctx.input_mut(|i| {
                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp);
            });
        }
    }

    /// True when any non-active-context pane or parked background app has
    /// pending background work. Drives the bounded 100ms wake loop in
    /// `update()` — this MUST use the same predicate as the tick gate in
    /// [`Self::drain_all_app_commands`], otherwise work could be marked
    /// pending without a frame ever being scheduled to drain it (#2021).
    pub(crate) fn background_processes_need_wake(&self) -> bool {
        let active = self.active_window;
        let inactive_context_needs_wake =
            self.windows.iter().enumerate().any(|(ctx_idx, context)| {
                ctx_idx != active
                    && context.panes.values().any(|pane| {
                        pane.as_app()
                            .is_some_and(|app_pane| app_pane.runtime.needs_background_tick())
                    })
            });

        inactive_context_needs_wake
            || self
                .background_apps
                .values()
                .any(|(_, app)| app.needs_background_tick())
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
        // Collect (context_id, pane_id, type_id, commands) per pane. We capture
        // context_id (stable u64) and type_id here because the manifest-declared
        // notification scope is looked up from the registry by type_id, and we
        // can't hold a mutable borrow on contexts + a shared borrow on the
        // registry at the same time during the match below.
        let active = self.active_window;
        let mut per_pane: Vec<(u64, u64, String, Vec<AppCommand>)> = Vec::new();
        for (ctx_idx, context) in self.windows.iter_mut().enumerate() {
            let context_id = context.context_id;
            for (pane_id, pane) in context.panes.iter_mut() {
                if let Some(app_pane) = pane.as_app_mut() {
                    // Active-context panes are already fully updated by ui()
                    // this frame. Non-active panes need a headless tick so
                    // timer/async events reach the subprocess and control
                    // commands flow out — but only when they actually have
                    // pending background work; idle apps are skipped so a
                    // busy foreground doesn't tick every background app on
                    // every frame (#2021).
                    if ctx_idx != active && app_pane.runtime.needs_background_tick() {
                        app_pane.runtime.background_tick();
                    }
                    let type_id = app_pane.runtime.type_id().to_string();
                    let cmds = app_pane.runtime.take_pending_commands();
                    if !cmds.is_empty() {
                        per_pane.push((context_id, *pane_id, type_id, cmds));
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
        let parked: Vec<(String, u64, Vec<AppCommand>)> = self
            .background_apps
            .iter_mut()
            .map(|(type_id, (park_context_id, app))| {
                // Same gate as visible non-active panes: tick only when the
                // app has pending background work (#2021). Already-queued
                // pending_commands are still taken unconditionally.
                if app.needs_background_tick() {
                    log::debug!("parked background app '{type_id}': pending work — ticking");
                    app.background_tick();
                }
                let cmds = app.take_pending_commands();
                (type_id.to_owned(), *park_context_id, cmds)
            })
            .collect();

        let mut deferred = Vec::new();

        for (type_id, park_context_id, cmds) in &parked {
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
                            "parked background app '{}' notification: {} (routing to context_id {})",
                            type_id, title, park_context_id
                        );
                        deferred.push(AppCommand::ShowNotification {
                            notify_id: notify_id.clone(),
                            sender_pane_id: 0, // no live pane — tombstone won't fire
                            source_context_id: *park_context_id,
                            level: level.clone(),
                            title: title.clone(),
                            body: body.clone(),
                            kind: kind.clone(),
                            options: options.clone(),
                            input_prompt: input_prompt.clone(),
                            required: *required,
                            priority: *priority,
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

        for (context_id, pane_id, type_id, cmds) in per_pane {
            // Scope is a per-app user-facing policy declared in the app's
            // manifest.toml. Apps never set it — the host resolves it once
            // per notification here. Defaults to `Context` when the manifest
            // omits the field (safe default: don't interrupt across contexts).
            let resolved_scope = self.registry.default_notification_scope_for(&type_id);
            for cmd in cmds {
                match cmd {
                    AppCommand::AssistantHostTool {
                        name,
                        input_json,
                        reply,
                        ..
                    } => {
                        deferred.push(AppCommand::AssistantHostTool {
                            name,
                            input_json,
                            origin_pane_id: pane_id,
                            origin_context_id: context_id,
                            reply,
                        });
                    }
                    AppCommand::SpawnApp { .. }
                    | AppCommand::SpawnPane { .. }
                    | AppCommand::ForwardPaneRequest { .. }
                    | AppCommand::DeliverPipeMessage { .. }
                    | AppCommand::OpenDirectedPipe { .. }
                    | AppCommand::DeliverRunUpdate { .. }
                    | AppCommand::InsertPathToken { .. }
                    | AppCommand::RequestCommandPreview { .. }
                    | AppCommand::OpenArtifact { .. }
                    | AppCommand::QueryContextState { .. } => deferred.push(cmd),
                    // Builtin apps emit sender_pane_id=0; rewrite to
                    // the real pane_id so the host can route the response.
                    AppCommand::RequestLinkedTerminal {
                        request_id,
                        cwd,
                        label,
                        place_below,
                        ..
                    } => {
                        deferred.push(AppCommand::RequestLinkedTerminal {
                            sender_pane_id: pane_id,
                            request_id,
                            cwd,
                            label,
                            place_below,
                        });
                    }
                    AppCommand::RunInLinkedTerminal {
                        terminal_pane_id,
                        command,
                        echo,
                        ..
                    } => {
                        deferred.push(AppCommand::RunInLinkedTerminal {
                            sender_pane_id: pane_id,
                            terminal_pane_id,
                            command,
                            echo,
                        });
                    }
                    AppCommand::CdRequest { cwd, .. } => {
                        deferred.push(AppCommand::CdRequest {
                            cwd,
                            sender_pane_id: pane_id,
                        });
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
                        deferred.push(AppCommand::ShowNotification {
                            notify_id,
                            sender_pane_id: pane_id,
                            source_context_id: context_id,
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
