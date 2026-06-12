//! App command dispatch — keyboard routing + command drain from app panes.

use crate::app::app_trait::{App, AppCommand};

use super::PlexiApp;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::permissions::AppPermissions;
    use crate::app_protocol::{NotifyKind, NotifyScope, PlexiEvent};
    use crate::process_app::ProcessApp;
    use crate::testing::HostHarness;
    /// Regression guard for issue #879: parked background app notifications
    /// hardcoded `source_context_id: 0`. Verify that the context_id recorded
    /// at park time propagates to the `ShowNotification` command.
    #[test]
    fn parked_app_notification_uses_park_context() {
        let mut h = HostHarness::new();

        // Seed a ShowNotification on a fresh ProcessApp's pending_commands,
        // then insert it into background_apps with park_context_id = 42.
        let (mut process_app, _draw_tx) = ProcessApp::new_for_test(999, AppPermissions::builtin());

        process_app
            .pending_commands
            .push(AppCommand::ShowNotification {
                notify_id: "test-notify".to_string(),
                sender_pane_id: 0,
                source_context_id: 0, // pre-filled stub value; drain should overwrite
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

        let park_context_id: u64 = 42;
        h.app.background_apps.insert(
            "test-app".to_string(),
            (park_context_id, Box::new(process_app)),
        );

        let cmds = h.app.drain_all_app_commands();

        let notification = cmds.iter().find(|c| {
            matches!(c, AppCommand::ShowNotification { notify_id, .. } if notify_id == "test-notify")
        });

        let Some(AppCommand::ShowNotification {
            source_context_id, ..
        }) = notification
        else {
            panic!("expected ShowNotification in drained commands — not found");
        };

        assert_eq!(
            *source_context_id, park_context_id,
            "source_context_id should be the park-time context_id ({park_context_id}), not hardcoded 0"
        );
    }

    #[test]
    fn parked_process_with_pending_headless_work_requests_host_wake() {
        let mut h = HostHarness::new();
        let (mut process_app, _draw_tx) = ProcessApp::new_for_test(999, AppPermissions::builtin());
        process_app.pending_timers.insert(
            "timer-1".to_string(),
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        );

        h.app
            .background_apps
            .insert("test-app".to_string(), (42, Box::new(process_app)));

        assert!(
            h.app.background_processes_need_wake(),
            "parked process apps with pending timers/async work must keep the host wake loop alive"
        );
    }

    /// Issue #2021: an idle parked app (no pending timers, no async work, no
    /// queued draw commands) must NOT be background-ticked on every frame.
    #[test]
    fn idle_parked_app_is_not_background_ticked() {
        let mut h = HostHarness::new();
        let (process_app, _draw_tx) = ProcessApp::new_for_test(999, AppPermissions::builtin());
        assert!(
            !process_app.needs_background_tick(),
            "a fresh idle test app must report no pending background work"
        );
        h.app
            .background_apps
            .insert("idle-app".to_string(), (1, Box::new(process_app)));

        let _ = h.app.drain_all_app_commands();

        let (_, app) = &h.app.background_apps["idle-app"];
        assert_eq!(
            app.background_tick_count, 0,
            "idle parked app must not be ticked by the foreground frame loop (#2021)"
        );
        assert!(
            !h.app.background_processes_need_wake(),
            "an idle parked app must not keep the host wake loop alive"
        );
    }

    /// Issue #2021 done-condition: a parked app's timer can surface its event
    /// in a single wake frame, without relying on continuous foreground
    /// repaint — and after draining, the app quiesces again.
    #[test]
    fn parked_app_timer_event_is_consumed_by_a_single_gated_tick() {
        let mut h = HostHarness::new();
        let (mut process_app, _draw_tx) = ProcessApp::new_for_test(999, AppPermissions::builtin());
        // Simulate SetTimer: routing inserts the pending flag, the timer
        // thread later delivers the Timer event on http_tx.
        process_app.pending_timers.insert(
            "t1".to_string(),
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        );
        process_app
            .http_tx
            .clone()
            .send(PlexiEvent::Timer {
                timer_id: "t1".to_string(),
            })
            .expect("test http channel send");
        assert!(
            process_app.needs_background_tick(),
            "a pending timer must mark the parked app as needing a tick"
        );
        h.app
            .background_apps
            .insert("timer-app".to_string(), (1, Box::new(process_app)));
        assert!(
            h.app.background_processes_need_wake(),
            "pending timer must keep the host wake loop alive"
        );

        let _ = h.app.drain_all_app_commands();

        let (_, app) = &h.app.background_apps["timer-app"];
        assert_eq!(
            app.background_tick_count, 1,
            "parked app with a fired timer must be ticked exactly once"
        );
        assert!(
            app.pending_timers.is_empty(),
            "the gated tick must consume the timer event"
        );
        assert!(
            !h.app.background_processes_need_wake(),
            "after the timer drains, the host wake loop must quiesce"
        );
    }

    /// Issue #2021: a parked app emitting a notification from its own process
    /// (draw command arriving on the stdout channel) surfaces in one wake
    /// frame. The stdout reader thread marks `draw_pending` and wakes the
    /// host; the next drain must tick the app and return the notification.
    #[test]
    fn parked_app_draw_command_notification_surfaces_via_draw_pending() {
        let mut h = HostHarness::new();
        let (process_app, draw_tx) = ProcessApp::new_for_test(999, AppPermissions::builtin());
        let draw_pending = std::sync::Arc::clone(&process_app.draw_pending);
        h.app
            .background_apps
            .insert("notify-app".to_string(), (7, Box::new(process_app)));

        // Idle frame first: no tick.
        let _ = h.app.drain_all_app_commands();
        assert_eq!(h.app.background_apps["notify-app"].1.background_tick_count, 0);

        // App emits a notification: the stdout reader sends the command, then
        // sets draw_pending (and wakes the host via repaint_ctx in prod).
        let cmd: crate::app_protocol::DrawCommand = serde_json::from_str(
            r#"{"type":"notify","level":"info","title":"ping","body":"","priority":50}"#,
        )
        .expect("notify draw command parses");
        draw_tx.send(cmd).expect("draw channel send");
        draw_pending.store(true, std::sync::atomic::Ordering::Release);

        let cmds = h.app.drain_all_app_commands();
        assert!(
            cmds.iter().any(|c| matches!(
                c,
                AppCommand::ShowNotification { title, .. } if title == "ping"
            )),
            "parked app's notification must surface on the wake frame"
        );
        assert!(
            !h.app.background_processes_need_wake(),
            "after draining the draw command, the host wake loop must quiesce"
        );
    }

    /// Issue #2021: an app exposing an MCP server must not keep the host in
    /// a permanent 100ms wake loop — only a queued (undelivered) tool call
    /// counts as pending background work.
    #[test]
    fn mcp_server_only_needs_tick_when_calls_are_queued() {
        use std::sync::{Arc, Mutex};

        let (mut process_app, _draw_tx) = ProcessApp::new_for_test(999, AppPermissions::builtin());
        let handle =
            crate::process_app::mcp_server::start_mcp_server(vec![], Arc::new(Mutex::new(None)))
                .expect("mcp server starts");
        let queue = Arc::clone(&handle.call_queue);
        process_app.mcp_server = Some(handle);

        assert!(
            !process_app.needs_background_tick(),
            "an MCP server with an empty call queue must not keep the host awake"
        );

        let (response_tx, _response_rx) = std::sync::mpsc::sync_channel(1);
        queue
            .lock()
            .unwrap()
            .push_back(crate::process_app::mcp_server::McpCallRequest {
                call_id: "c1".to_string(),
                tool_name: "noop".to_string(),
                arguments: serde_json::Value::Null,
                response_tx,
            });
        assert!(
            process_app.needs_background_tick(),
            "a queued MCP tool call must mark the app as needing a background tick"
        );
    }

    /// Regression guard for issue #1795: key events must not reach an app pane
    /// when the egui viewport reports `focused = Some(false)` (window out of OS focus).
    ///
    /// Strategy: call `dispatch_app_key_events` directly with a hand-crafted context
    /// that holds a `Paste` event (Paste queues to `outbound_events` directly, so it
    /// is readable via `effects_drain` before any `flush_outbound_events` call drains
    /// the queue). With the guard in place, `dispatch_app_key_events` returns before
    /// `handle_key` is reached, so `outbound_events` stays empty.
    #[test]
    fn no_key_dispatch_when_viewport_unfocused() {
        let mut h = HostHarness::new();
        let pane = h.add_test_pane();

        // Stabilise egui tile structure (bare-pane root rewrites itself on first render).
        h.run_frames(1);

        // Point window focus at the app pane tile.
        let tile_id = {
            let win = &h.app.windows[0];
            win.tree
                .tiles
                .iter()
                .find_map(|(id, tile)| match tile {
                    egui_tiles::Tile::Pane(p) if *p == pane => Some(*id),
                    _ => None,
                })
                .expect("test pane must have a tile")
        };
        h.app.windows[0].focused_pane = Some(tile_id);

        // Build a fresh egui context with viewport focused = Some(false) and a Paste
        // event. We call dispatch_app_key_events directly rather than through a full
        // frame so flush_outbound_events (called inside ui()) cannot drain the queue
        // before we inspect it.
        let test_ctx = egui::Context::default();
        let viewports: egui::ViewportIdMap<egui::ViewportInfo> =
            std::iter::once((egui::ViewportId::ROOT, {
                let mut info = egui::ViewportInfo::default();
                info.focused = Some(false);
                info
            }))
            .collect();

        let _ = test_ctx.run(
            egui::RawInput {
                viewports,
                events: vec![egui::Event::Paste("should-not-arrive".into())],
                ..Default::default()
            },
            |ctx| {
                h.app.dispatch_app_key_events(ctx);
            },
        );

        // outbound_events must be empty: the viewport-focus guard must have returned
        // before handle_key was reached. Paste queues to outbound_events directly
        // (unlike Key/Text which go through send_event), so it would be visible here
        // if the guard were absent.
        let events = h.effects_drain(pane);
        let paste_events: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, PlexiEvent::Paste { .. }))
            .collect();
        assert!(
            paste_events.is_empty(),
            "Paste must not reach the app when the viewport is unfocused; got {paste_events:?}"
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
                        ..
                    } => {
                        deferred.push(AppCommand::RequestLinkedTerminal {
                            sender_pane_id: pane_id,
                            request_id,
                            cwd,
                            label,
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

#[cfg(test)]
mod ownership_tests {
    use crate::app::app_trait::AppCommand;
    use crate::app::FocusLayer;
    use crate::app_protocol::{ArtifactOpenMode, NotifyKind, NotifyScope};
    use crate::host::pane::{AppRuntime, Pane};
    use crate::testing::HostHarness;

    /// Helper: push an `AppCommand` directly onto the `ProcessApp`'s
    /// `pending_commands` queue so the drain loop sees it next frame.
    fn push_command(h: &mut HostHarness, pane_id: u64, cmd: AppCommand) {
        let win = &mut h.app.windows[0];
        let Some(Pane::App(app_pane)) = win.panes.get_mut(&pane_id) else {
            panic!("pane {pane_id} not found");
        };
        let AppRuntime::Process(ref mut pa) = app_pane.runtime else {
            panic!("pane {pane_id} is not a ProcessApp");
        };
        pa.pending_commands.push(cmd);
    }

    #[test]
    fn run_in_linked_terminal_cross_pane_rejected() {
        let mut h = HostHarness::new();
        let app_pane = h.add_test_pane();

        // app_pane has linked_pane_id = None — any terminal_pane_id is rejected.
        push_command(
            &mut h,
            app_pane,
            AppCommand::RunInLinkedTerminal {
                sender_pane_id: app_pane,
                terminal_pane_id: 999, // not linked
                command: "rm -rf /".to_string(),
                echo: true,
            },
        );

        h.run_frames(1);
        // No crash, no events back to the app.
        let events = h.effects_drain(app_pane);
        assert!(
            events.is_empty(),
            "RunInLinkedTerminal with unlinked terminal must produce no events; got {events:?}"
        );
    }

    #[test]
    fn request_command_preview_unlinked_terminal_rejected() {
        let mut h = HostHarness::new();
        let app_pane = h.add_test_pane();

        // app_pane has linked_pane_id = None — any preview on an unlinked terminal is rejected.
        // Call dispatch_command_preview directly so we can read outbound_events before
        // flush_outbound_events drains them (which happens inside the egui frame loop).
        h.app.dispatch_command_preview(
            app_pane,
            "req-1".to_string(),
            999, // terminal_pane_id not linked
            "ls".to_string(),
        );

        // Rejection emits CommandPreview with empty would_run_in_cwd directly into
        // process_app.outbound_events — readable here before any frame flush.
        let events = h.effects_drain(app_pane);
        let preview = events
            .iter()
            .find(|e| matches!(e, crate::app_protocol::PlexiEvent::CommandPreview { .. }));
        let Some(crate::app_protocol::PlexiEvent::CommandPreview {
            would_run_in_cwd, ..
        }) = preview
        else {
            panic!("expected CommandPreview rejection event; got: {events:?}");
        };
        assert!(
            would_run_in_cwd.is_empty(),
            "expected empty cwd for rejected preview, got {would_run_in_cwd:?}"
        );
    }

    #[test]
    fn open_artifact_outside_workspace_rejected() {
        let mut h = HostHarness::new();
        let app_pane = h.add_test_pane();
        // workspace_root is std::env::temp_dir() — /etc/passwd is outside.
        push_command(
            &mut h,
            app_pane,
            AppCommand::OpenArtifact {
                sender_pane_id: app_pane,
                path: "/etc/passwd".to_string(),
                mode: ArtifactOpenMode::OpenWithDefault,
            },
        );
        // Should not panic; shell_open is a no-op in cfg(test) environments
        // because no real shell is available — we just verify clean exit.
        h.run_frames(1);
    }

    /// QueryContextState for own context: drain_all_app_commands collects
    /// the command with the correct sender_pane_id and context_id.
    #[test]
    fn query_context_state_own_context_collected() {
        let mut h = HostHarness::new();
        let pane_id = h.add_test_pane();

        push_command(
            &mut h,
            pane_id,
            AppCommand::QueryContextState {
                sender_pane_id: pane_id,
                context_id: 1,
            },
        );

        let cmds = h.app.drain_all_app_commands();
        let found = cmds.iter().any(|c| {
            matches!(c, AppCommand::QueryContextState {
                sender_pane_id: sid, context_id: cid
            } if *sid == pane_id && *cid == 1)
        });
        assert!(found, "QueryContextState must appear in drained commands");
    }

    /// ShowNotification is overlay-safe: must reach `pending_notifications` even
    /// when a modal holds keyboard input (otherwise notifications from background
    /// apps would be silently lost while the user is interacting with a modal).
    #[test]
    fn show_notification_safe_while_overlay_active() {
        let mut h = HostHarness::new();
        let pane_id = h.add_test_pane();
        h.run_frames(1); // stabilize tile layout

        // Simulate a modal owning keyboard input.
        h.app.push_focus_layer(FocusLayer::NotificationModal);
        h.app.show_notification_modal = true;
        // Test constructor initialises notifications_enabled = false; enable it so
        // ShowNotification is not silently dropped by the master-switch guard.
        h.app.notifications_enabled = true;
        assert!(h.app.input_captured_by_overlay(), "overlay must be active");

        push_command(
            &mut h,
            pane_id,
            AppCommand::ShowNotification {
                notify_id: "overlay-safe-notify".to_string(),
                sender_pane_id: pane_id,
                source_context_id: 0,
                level: "info".to_string(),
                title: "notification while modal open".to_string(),
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
            },
        );

        h.run_frames(1);

        let found = h
            .app
            .pending_notifications
            .iter()
            .any(|n| n.notify_id == "overlay-safe-notify");
        assert!(
            found,
            "ShowNotification must reach pending_notifications immediately even while overlay is active"
        );
        assert!(
            h.app.overlay_held_cmds.is_empty(),
            "ShowNotification must not be placed in the overlay hold queue"
        );
    }

    /// SpawnPane is overlay-unsafe: must be held while a modal owns input and
    /// released only after the overlay is dismissed.
    #[test]
    fn spawn_pane_deferred_while_overlay_active_released_after() {
        let mut h = HostHarness::new();
        let pane_id = h.add_test_pane();
        h.run_frames(1); // stabilize

        // Simulate a modal owning keyboard input.
        h.app.push_focus_layer(FocusLayer::NotificationModal);
        h.app.show_notification_modal = true;
        assert!(h.app.input_captured_by_overlay(), "overlay must be active");

        push_command(
            &mut h,
            pane_id,
            AppCommand::SpawnPane {
                type_id: "__test_nonexistent__".to_string(),
                layout: "split".to_string(),
                args: vec![],
                pipe_id: None,
                from_pane_id: Some(pane_id),
                request_id: Some("defer-test".to_string()),
                target_context: None,
            },
        );

        h.run_frames(1);

        // SpawnPane must be held, not dispatched.
        assert_eq!(
            h.app.overlay_held_cmds.len(),
            1,
            "SpawnPane must be held in overlay_held_cmds while overlay is active"
        );

        // Dismiss the modal.
        h.app.show_notification_modal = false;
        h.app.pop_focus_layer(&FocusLayer::NotificationModal);
        assert!(
            !h.app.input_captured_by_overlay(),
            "overlay must be inactive after pop"
        );

        h.run_frames(1);

        // Hold queue must be empty — the command was released this frame.
        assert!(
            h.app.overlay_held_cmds.is_empty(),
            "overlay_held_cmds must be empty after overlay releases"
        );
    }

    /// QueryContextState for non-descendant context: the dispatch handler
    /// must NOT produce a ContextStateResponse (visibility denied).
    /// We run a full frame so the deferred dispatch executes, then verify
    /// outbound_events is empty (the frame flush sends events to a dead
    /// stdin channel which is harmless in tests, but even if something
    /// slipped through, the denial path skips queue_outbound_event entirely).
    #[test]
    fn query_context_state_non_descendant_produces_no_response() {
        let mut h = HostHarness::new();
        let pane_id = h.add_test_pane();

        // Context 99 is a sibling (parent=None), not a descendant of context 1.
        h.app.host.add_context(99, None);

        push_command(
            &mut h,
            pane_id,
            AppCommand::QueryContextState {
                sender_pane_id: pane_id,
                context_id: 99,
            },
        );

        h.run_frames(1);

        // After the frame, outbound_events should be empty. Even though
        // flush_outbound_events() runs during the frame, the denial path
        // never queues anything, so nothing gets sent.
        let effects = h.effects_drain(pane_id);
        let has_response = effects.iter().any(|e| {
            matches!(
                e,
                crate::app_protocol::PlexiEvent::ContextStateResponse { .. }
            )
        });
        assert!(
            !has_response,
            "non-descendant QueryContextState must not produce ContextStateResponse"
        );
    }
}
