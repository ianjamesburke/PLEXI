//! App command dispatch — keyboard routing + command drain from app panes.

use crate::app::app_trait::AppCommand;
use crate::app::notifications::PendingNotification;

use super::{
    cmd_from_args, is_overlay_unsafe_cmd, overlay_unsafe_cmd_name,
    register_directed_pipe_on_target, PlexiApp,
};

impl PlexiApp {
    /// Feed keyboard input to the focused app pane. Keys only go to the
    /// focused pane (that's the whole point of focus). Command drain
    /// happens separately via [`drain_all_app_commands`] so background
    /// apps can surface notifications and other commands while the
    /// focused pane is something else (or while a modal holds focus).
    pub(super) fn dispatch_app_key_events(
        &mut self,
        ctx: &egui::Context,
        input: &mut crate::app::input_router::PlexiInput,
    ) {
        // Block key delivery when the OS has focus elsewhere. `active_window` is not
        // cleared on blur, so without this guard keys route to the last-active pane
        // even while a different app (or Plexi window) owns the keyboard.
        // `viewport().focused` is window-focus state, not an event, so it is
        // untouched by the router's `take_from` — read it from `ctx` directly.
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
        // A focused declarative TextInput owns the keyboard (stint 0456):
        // leave the frame's events in the router so the render pass's
        // TextEdit consumes them (typing, arrows, Enter-submit). Escape
        // surrenders egui focus natively, flipping this gate back off so
        // keys reach the app's KeyEvent path again next frame.
        let text_surface_focused = crate::app::input_owner::focused_pane_text_surface(ctx, pane_id);
        let gate_id = egui::Id::new(("l1_text_input_gate", pane_id));
        let previously_focused =
            ctx.memory_mut(|m| m.data.get_temp::<bool>(gate_id).unwrap_or(false));
        if text_surface_focused != previously_focused {
            log::info!(
                "app_keys: pane {pane_id} text-input gate {} — keys route to {}",
                if text_surface_focused { "on" } else { "off" },
                if text_surface_focused {
                    "focused TextInput"
                } else {
                    "app key events"
                }
            );
            ctx.memory_mut(|m| m.data.insert_temp(gate_id, text_surface_focused));
        }
        if text_surface_focused {
            return;
        }
        // The app classifies keys from the frame's ownership-transfer buffer
        // (stint 0387) rather than a second read of `ctx`. If it consumed the
        // key, also claim Escape/ArrowUp out of the buffer so `poll_actions`
        // (which takes the same buffer later this frame) can't re-fire CloseApp
        // for keys the app already handled — e.g. Escape exiting search mode in
        // the file browser rather than closing the pane, or ArrowUp recalling a
        // prior message in the assistant.
        let disposition = app_pane.runtime.handle_key(input);
        if disposition == crate::app::app_trait::KeyDisposition::Consumed {
            input.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
            input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp);
        }
        // egui surrenders widget focus on Escape during `begin_pass`, before
        // this dispatch runs — so on the keypress that leaves a focused
        // TextInput the gate above is already off. The app has now seen that
        // Escape through `handle_key` (stint 0460 — e.g. the assistant
        // interrupts its in-flight turn on the same press that leaves the
        // composer); claim whatever is left of it out of the buffer so the
        // AppActive CloseApp binding can't close the pane on the same
        // keypress.
        if previously_focused && input.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
            log::info!(
                "app_keys: pane {pane_id} Escape left the focused TextInput — delivered to app, CloseApp suppressed"
            );
        }
    }

    /// If the focused pane in the active window is a terminal and no overlay
    /// owns keyboard input, take this frame's input-intent events out of `ctx`
    /// and return them for the focused terminal (stint 0387). The caller threads
    /// the result into `render_panels`, where whichever pane path is active
    /// (tiled or zoomed) passes it to the focused `TerminalView`, so the events
    /// reach the terminal without egui's render-time widget machinery getting
    /// a chance to consume them first.
    ///
    /// Returns `None` (leaving `ctx` untouched) unless the frame's derived
    /// [`crate::app::input_owner::InputOwner`] is a terminal pane. The single
    /// derivation covers every case the old ad-hoc gates covered: overlays on
    /// the focus stack, the inline sidebar rename editor (the stint 0387
    /// regression — now `InputOwner::Overlay(SidebarRename)`), an unfocused OS
    /// window, and a non-terminal focused pane.
    ///
    /// This is a read of focus state only; it never mutates `active_window` or
    /// `focused_pane` to steer the lookup (repo trap: don't switch global state
    /// to thread data through a function).
    pub(super) fn take_focused_terminal_input(
        &mut self,
        ctx: &egui::Context,
    ) -> Option<crate::render::terminal_pane::TerminalInput> {
        let crate::app::input_owner::InputOwner::Pane(pane_id) = self.input_owner(ctx) else {
            return None;
        };
        // The owner pane must be a terminal (including an exited one — its
        // "[process exited]" view auto-closes on the next keypress, which it now
        // reads from this buffer).
        self.windows[self.active_window]
            .panes
            .get(&pane_id)?
            .as_terminal()?;

        let router = crate::app::input_router::PlexiInput::take_from(ctx);
        let (keyboard_events, modifiers) = router.into_events_and_modifiers();

        // Feature instrumentation (stint 0387): confirm the structural handoff
        // delivered Cmd+A (egui's built-in select-all, the bug's signature) to
        // the terminal rather than letting a widget swallow it. Targeted — only
        // fires on the select-all chord, never per keystroke.
        if keyboard_events.iter().any(|e| {
            matches!(
                e,
                egui::Event::Key { key: egui::Key::A, pressed: true, modifiers: m, .. }
                    if m.command
            )
        }) {
            log::info!(
                "terminal_input: routed Cmd+A (select-all) to focused terminal pane_id={pane_id} via ownership transfer"
            );
        }

        Some(crate::render::terminal_pane::TerminalInput {
            keyboard_events,
            modifiers,
        })
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

    /// Drain every app pane's outbound commands and execute them (stint 0688).
    ///
    /// This is external-client servicing, not rendering: an app pane answers
    /// assistant tool calls, emits notifications, and requests host actions on
    /// its own schedule, with no dependence on being painted. It therefore runs
    /// from [`eframe::App::logic`], per the rule in the root `AGENTS.md` —
    /// eframe skips `App::ui` entirely while the window is minimized or fully
    /// occluded, and this drain living in `ui` meant a covered host executed no
    /// app command of any runtime. Every `todo.list` / `calc.add` call to such a
    /// host sat here until the broker's 30s timeout fired.
    ///
    /// Overlay state is consulted (`input_captured_by_overlay`) but never
    /// rendered here: unsafe commands are held while a modal owns input and
    /// released on the next modal-free pass, exactly as before.
    pub(super) fn service_app_commands(&mut self, ctx: &egui::Context) {
        // Drain every app pane's pending_commands every frame — including
        // while a modal holds focus. Background apps emitting notifications
        // must reach the queue *now*, not be buffered until the modal
        // closes (which caused the "ghost queue appears on reopen" bug).
        let fresh_cmds = self.drain_all_app_commands();
        if self.background_processes_need_wake() {
            crate::platform::frame_diag::note(
                crate::platform::frame_diag::RepaintCause::AppIdlePoll,
            );
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        // When the overlay releases, prepend any held unsafe commands so they
        // execute before new commands this frame (FIFO order preserved).
        let deferred_app_cmds: Vec<_> =
            if !self.input_captured_by_overlay() && !self.overlay_held_cmds.is_empty() {
                let released = std::mem::take(&mut self.overlay_held_cmds);
                log::info!(
                    "app_cmd: releasing {} held command(s) — overlay released",
                    released.len()
                );
                let mut all = released;
                all.extend(fresh_cmds);
                all
            } else {
                fresh_cmds
            };

        // Handle deferred app commands returned from dispatch_app_key_events.
        for cmd in deferred_app_cmds {
            use crate::app::app_trait::AppCommand;
            // Hold overlay-unsafe side effects (layout/terminal mutations, focus changes)
            // while a modal owns input. Safe commands (ShowNotification, pipes, queries)
            // dispatch immediately. Released on the next modal-free frame.
            if self.input_captured_by_overlay() && is_overlay_unsafe_cmd(&cmd) {
                log::info!(
                    "app_cmd: deferring {} — overlay active ({} total held)",
                    overlay_unsafe_cmd_name(&cmd),
                    self.overlay_held_cmds.len() + 1,
                );
                self.overlay_held_cmds.push(cmd);
                continue;
            }
            match cmd {
                AppCommand::WasmHostEffect { pane_id, effect } => {
                    use crate::host::wasm_app::InputEvent;
                    use crate::host::wasm_pane::WasmHostEffect;

                    let source_window_index = self
                        .find_pane_in_any_window(pane_id)
                        .map(|(window_index, _)| window_index);
                    let event = match effect {
                        WasmHostEffect::ClipboardRead => {
                            let result = arboard::Clipboard::new()
                                .and_then(|mut clipboard| clipboard.get_text())
                                .map(Some)
                                .map_err(|error| error.to_string());
                            log::info!(
                                "wasm effect: clipboard-read pane_id={pane_id} ok={}",
                                result.is_ok()
                            );
                            Some(InputEvent::ClipboardReadResult(result))
                        }
                        WasmHostEffect::ClipboardWrite { text } => {
                            ctx.copy_text(text);
                            log::info!("wasm effect: clipboard-write pane_id={pane_id}");
                            Some(InputEvent::ClipboardWriteResult(Ok(())))
                        }
                        WasmHostEffect::Notify { title, body, icon } => {
                            if let Some(window_index) = source_window_index {
                                let source_context_id = self.windows[window_index].context_id;
                                let source_window_id = self.windows[window_index].window_id;
                                let notify_id = format!("wasm:{pane_id}:{}", uuid::Uuid::new_v4());
                                log::info!(
                                    "wasm effect: notify pane_id={pane_id} context_id={source_context_id} icon={icon:?}"
                                );
                                // `WasmHostEffect::Notify` carries no scope, so
                                // a guest gets the shared default and cannot
                                // request global; that needs a WIT change.
                                self.enqueue_notification(
                                    crate::app::notifications::NotifySource::Wasm,
                                    PendingNotification {
                                        notify_id,
                                        sender_pane_id: pane_id,
                                        source_context_id,
                                        source_window_id,
                                        scope: crate::app_protocol::NotifyScope::default(),
                                        level: "info".to_string(),
                                        title,
                                        body,
                                        kind: crate::app_protocol::NotifyKind::Message,
                                        options: vec![],
                                        input_prompt: None,
                                        required: false,
                                        priority: 0,
                                        image_inline: None,
                                        image_pipe_id: None,
                                        response_file: None,
                                        timeout_secs: None,
                                        on_dismiss: None,
                                        enqueued_at: std::time::Instant::now(),
                                        tombstoned: false,
                                        deliver_after: None,
                                    },
                                );
                                Some(InputEvent::NotifyResult(Ok(())))
                            } else {
                                let error = format!("source pane {pane_id} is closed");
                                log::warn!("wasm effect: notify failed: {error}");
                                Some(InputEvent::NotifyResult(Err(error)))
                            }
                        }
                        WasmHostEffect::Spawn {
                            app_id,
                            layout,
                            args,
                        } => {
                            let predicted_pane_id = self.host.next_pane_id();
                            let result = self
                                .launch_app_by_id_with_layout(&app_id, layout, &args, None)
                                .map(|existing| existing.unwrap_or(predicted_pane_id))
                                .map_err(|error| error.to_string());
                            log::info!(
                                "wasm effect: spawn pane_id={pane_id} app_id={app_id} ok={}",
                                result.is_ok()
                            );
                            Some(InputEvent::SpawnResult(result))
                        }
                        WasmHostEffect::SubscribeEvents {
                            request_id,
                            app_id,
                            event_names,
                            payload_mode,
                            trigger_mode,
                            resource_id,
                        } => {
                            self.begin_app_event_subscription(
                                pane_id,
                                request_id,
                                app_id,
                                event_names,
                                payload_mode,
                                trigger_mode,
                                resource_id,
                            );
                            None
                        }
                        WasmHostEffect::UnsubscribeEvents {
                            request_id,
                            subscription_id,
                        } => {
                            let result = crate::host::app_timeline::global()
                                .lock()
                                .unwrap()
                                .remove_subscription_for(
                                    crate::broker::ActorType::App,
                                    &crate::host::event_subscriptions::app_subscriber_id(pane_id),
                                    &subscription_id,
                                );
                            let (removed, error) = match result {
                                Ok(removed) => (removed, None),
                                Err(error) => (false, Some(error)),
                            };
                            Some(InputEvent::EventUnsubscriptionResult(
                                crate::host::wasm_app::bindings::plexi::platform::types::EventUnsubscriptionResultEvent {
                                    request_id,
                                    removed,
                                    error,
                                },
                            ))
                        }
                        WasmHostEffect::DeclareTools { tools } => {
                            let names = tools.iter().map(|tool| tool.name.clone()).collect();
                            let provider = self.windows.iter().find_map(|window| {
                                window.panes.get(&pane_id).and_then(|pane| {
                                    pane.as_app().and_then(|app| {
                                        app.runtime.tool_event_sender(ctx.clone()).map(|sender| {
                                            (
                                                app.manifest_id.clone(),
                                                app.workspace_root.clone(),
                                                sender,
                                            )
                                        })
                                    })
                                })
                            });
                            if let Some((app_id, workspace_root, sender)) = provider {
                                crate::plexi_ai::tool_dispatch::register(
                                    pane_id,
                                    app_id,
                                    tools,
                                    sender,
                                    workspace_root,
                                );
                                Some(InputEvent::DeclareToolsResult(Ok(names)))
                            } else {
                                Some(InputEvent::DeclareToolsResult(Err(format!(
                                    "source pane {pane_id} is closed"
                                ))))
                            }
                        }
                        WasmHostEffect::ToolResult {
                            call_id,
                            output_json,
                            error,
                        } => {
                            crate::plexi_ai::tool_dispatch::resolve_pending(
                                &call_id,
                                crate::plexi_ai::tool_dispatch::ToolCallResult {
                                    output_json,
                                    error,
                                },
                            );
                            None
                        }
                    };
                    if let Some(event) = event {
                        self.complete_wasm_host_effect(pane_id, event);
                    }
                }
                AppCommand::ExposeTools {
                    tools,
                    pane_id: Some(pane_id),
                } => {
                    let provider = self.windows.iter().find_map(|window| {
                        window.panes.get(&pane_id).and_then(|pane| {
                            pane.as_app().and_then(|app| {
                                app.runtime.tool_event_sender(ctx.clone()).map(|sender| {
                                    (app.manifest_id.clone(), app.workspace_root.clone(), sender)
                                })
                            })
                        })
                    });
                    if let Some((app_id, workspace_root, sender)) = provider {
                        crate::plexi_ai::tool_dispatch::register(
                            pane_id,
                            app_id,
                            tools,
                            sender,
                            workspace_root,
                        );
                    } else {
                        log::warn!(
                            "tool_dispatch: ignore ExposeTools from unsupported pane {pane_id}"
                        );
                    }
                }
                AppCommand::ExposeTools { pane_id: None, .. } => {
                    log::warn!("tool_dispatch: ExposeTools missing source pane")
                }
                AppCommand::ToolResult {
                    call_id,
                    output_json,
                    error,
                } => crate::plexi_ai::tool_dispatch::resolve_pending(
                    &call_id,
                    crate::plexi_ai::tool_dispatch::ToolCallResult { output_json, error },
                ),
                AppCommand::AppEventRequest {
                    request,
                    pane_id: Some(pane_id),
                } => {
                    let app_id = self.windows.iter().find_map(|window| {
                        window
                            .panes
                            .get(&pane_id)
                            .and_then(|pane| pane.as_app())
                            .map(|app| app.manifest_id.clone())
                    });
                    let Some(app_id) = app_id else {
                        log::warn!("app events: source pane {pane_id} is closed");
                        continue;
                    };
                    let response = match request {
                        crate::app_protocol::AppRequest::DeclareEventStreams { streams } => {
                            match crate::host::app_timeline::global()
                                .lock()
                                .unwrap()
                                .declare_streams(&app_id, streams)
                            {
                                Ok(streams) => {
                                    crate::app_protocol::PlexiEvent::DeclareEventStreamsResult {
                                        streams: Some(streams),
                                        error: None,
                                    }
                                }
                                Err(error) => {
                                    crate::app_protocol::PlexiEvent::DeclareEventStreamsResult {
                                        streams: None,
                                        error: Some(error),
                                    }
                                }
                            }
                        }
                        crate::app_protocol::AppRequest::EmitEvent {
                            event,
                            actor,
                            actor_id,
                            caused_by,
                            summary,
                            resource_id,
                            resource_scope,
                            revision_after,
                            payload,
                            state_ref,
                            revision_before,
                            rollback_token,
                            changed_resources,
                            suggested_trigger,
                        } => {
                            let emitted = crate::host::app_timeline::EmittedEvent {
                                event,
                                actor,
                                actor_id,
                                caused_by,
                                summary,
                                resource_id,
                                resource_scope,
                                revision_after,
                                payload,
                                state_ref,
                                revision_before,
                                rollback_token,
                                changed_resources,
                                suggested_trigger,
                            };
                            match crate::host::app_timeline::global()
                                .lock()
                                .unwrap()
                                .record_event(&app_id, pane_id, emitted)
                            {
                                Ok(outcome) => crate::app_protocol::PlexiEvent::EmitEventResult {
                                    sequence: Some(outcome.event_id),
                                    error: None,
                                },
                                Err(error) => crate::app_protocol::PlexiEvent::EmitEventResult {
                                    sequence: None,
                                    error: Some(error),
                                },
                            }
                        }
                        crate::app_protocol::AppRequest::SubscribeAppEvents {
                            request_id,
                            app_id: publisher_app_id,
                            event_names,
                            payload_mode,
                            trigger_mode,
                            resource_id,
                        } => {
                            self.begin_app_event_subscription(
                                pane_id,
                                request_id,
                                publisher_app_id,
                                event_names,
                                payload_mode,
                                trigger_mode,
                                resource_id,
                            );
                            continue;
                        }
                        crate::app_protocol::AppRequest::UnsubscribeAppEvents {
                            request_id,
                            subscription_id,
                        } => {
                            let result = crate::host::app_timeline::global()
                                .lock()
                                .unwrap()
                                .remove_subscription_for(
                                    crate::broker::ActorType::App,
                                    &crate::host::event_subscriptions::app_subscriber_id(pane_id),
                                    &subscription_id,
                                );
                            match result {
                                Ok(removed) => {
                                    crate::app_protocol::PlexiEvent::AppEventsUnsubscribed {
                                        request_id,
                                        removed,
                                        error: None,
                                    }
                                }
                                Err(error) => {
                                    crate::app_protocol::PlexiEvent::AppEventsUnsubscribed {
                                        request_id,
                                        removed: false,
                                        error: Some(error),
                                    }
                                }
                            }
                        }
                        other => {
                            log::warn!("app events: unsupported app request {other:?}");
                            continue;
                        }
                    };
                    self.queue_event_to_app_pane_anywhere(pane_id, response);
                }
                AppCommand::AppEventRequest { pane_id: None, .. } => {
                    log::warn!("app events: request missing source pane")
                }
                AppCommand::AssistantHostTool {
                    name,
                    input_json,
                    origin_pane_id,
                    origin_context_id,
                    reply,
                } => {
                    // Network fetch resolves its policy here on the UI thread
                    // but must not block it on a remote peer, so it owns its
                    // own reply channel and answers from a worker thread.
                    if name == crate::app::assistant_host_tools::HOST_TOOL_NET_FETCH {
                        self.handle_assistant_net_fetch(&input_json, origin_pane_id, reply);
                        continue;
                    }
                    let result = self.handle_assistant_host_tool(
                        &name,
                        &input_json,
                        origin_pane_id,
                        origin_context_id,
                    );
                    if reply.send(result).is_err() {
                        log::warn!("assistant_host_tool: worker dropped reply for '{name}'");
                    }
                }
                // Capability-gated pane read/control request from a PGAP app
                // (stint 0013/0014). routing.rs already checked panes.read /
                // panes.control; execute it through the same handler CLI
                // requests take over PLEXI_SOCKET.
                AppCommand::ForwardPaneRequest { request } => {
                    self.handle_pane_ipc_request(request);
                }
                AppCommand::SpawnApp {
                    type_id,
                    layout,
                    args,
                } => {
                    // Capture requesting pane before launch changes focused_pane.
                    let active = self.active_window;
                    let requesting_pane_id = self.windows[active]
                        .focused_pane
                        .and_then(|tile| self.windows[active].tree.tiles.get(tile))
                        .and_then(|tile| {
                            if let egui_tiles::Tile::Pane(pid) = tile {
                                Some(*pid)
                            } else {
                                None
                            }
                        });

                    // Predict the pane id, but let a dedup override it with the
                    // real existing instance's id (#0336): `Ok(Some(id))` means
                    // the launch focused a live instance instead of spawning.
                    let predicted_pane_id = self.host.next_pane_id();
                    let new_pane_id = match self
                        .launch_app_by_id_with_layout(&type_id, layout, &args, None)
                    {
                        Ok(Some(existing_id)) => existing_id,
                        Ok(None) => predicted_pane_id,
                        Err(e) => {
                            // Fail loud: never swallow a launch failure or confirm
                            // a spawn that did not happen (#2283).
                            log::warn!(
                                "SpawnApp: launch of '{type_id}' failed — {e}; not confirming AppSpawned"
                            );
                            continue;
                        }
                    };

                    // Confirm back to the requesting app.
                    if let Some(req_pane_id) = requesting_pane_id {
                        let active = self.active_window;
                        if let Some(pane) = self.windows[active].panes.get_mut(&req_pane_id) {
                            let event = crate::app_protocol::PlexiEvent::AppSpawned {
                                pane_id: new_pane_id,
                                type_id: type_id.clone(),
                            };
                            if let Some(a) = pane.as_app_mut() {
                                a.runtime.queue_outbound_event(event);
                            }
                        }
                    }
                }
                AppCommand::SpawnPane {
                    type_id,
                    layout,
                    args,
                    from_pane_id,
                    request_id,
                    target_context,
                } => {
                    // "background" layout is not yet implemented (blocked on #291).
                    if layout == "background" {
                        let active = self.active_window;
                        let requesting_pane_id = self.windows[active]
                            .focused_pane
                            .and_then(|tile| self.windows[active].tree.tiles.get(tile))
                            .and_then(|tile| {
                                if let egui_tiles::Tile::Pane(pid) = tile {
                                    Some(*pid)
                                } else {
                                    None
                                }
                            });
                        if let Some(req_pane_id) = requesting_pane_id {
                            let active = self.active_window;
                            if let Some(pane) = self.windows[active].panes.get_mut(&req_pane_id) {
                                if let Some(a) = pane.as_app_mut() {
                                    a.runtime.queue_outbound_event(
                                        crate::app_protocol::PlexiEvent::PaneSpawnError {
                                            reason: "layout 'background' not yet implemented"
                                                .to_string(),
                                            request_id: request_id.clone(),
                                        },
                                    );
                                }
                            }
                        }
                        log::warn!("SpawnPane: layout='background' not implemented, rejected");
                        continue;
                    }

                    // A spawned child reports completion over the event bus
                    // (`EmitEvent` with `<app>::completed`), not a JSON-pipe
                    // reply — the `--pipe=<id>` reply coupling was removed in
                    // 0327 along with the legacy peer-broadcast transport.
                    let effective_args = args;

                    // target_context validation (#1518): if set, verify the
                    // target exists and is a descendant of the requester's
                    // context. Switch active_window temporarily so the spawn
                    // lands in the right context.
                    let original_active_window = self.active_window;
                    if let Some(target_ctx_id) = target_context {
                        let requester_context_id = self.windows[self.active_window].context_id;
                        let target_exists =
                            self.router.iter().any(|c| c.context_id == target_ctx_id);
                        let is_descendant = self
                            .host
                            .ancestors_of(target_ctx_id)
                            .contains(&requester_context_id);

                        if !target_exists
                            || (!is_descendant && requester_context_id != target_ctx_id)
                        {
                            log::warn!(
                                "SpawnPane: target_context={target_ctx_id} invalid or not a descendant of requester context {requester_context_id}"
                            );
                            // Send error back to requesting pane.
                            let active = self.active_window;
                            let requesting_pane_id = self.windows[active]
                                .focused_pane
                                .and_then(|tile| self.windows[active].tree.tiles.get(tile))
                                .and_then(|tile| {
                                    if let egui_tiles::Tile::Pane(pid) = tile {
                                        Some(*pid)
                                    } else {
                                        None
                                    }
                                });
                            if let Some(req_pane_id) = requesting_pane_id {
                                if let Some(pane) = self.windows[active].panes.get_mut(&req_pane_id)
                                {
                                    if let Some(a) = pane.as_app_mut() {
                                        a.runtime.queue_outbound_event(
                                            crate::app_protocol::PlexiEvent::PaneSpawnError {
                                                reason: format!(
                                                    "target_context {target_ctx_id} is not a descendant of context {requester_context_id}"
                                                ),
                                                request_id: request_id.clone(),
                                            },
                                        );
                                    }
                                }
                            }
                            continue;
                        }

                        // Switch to a window in the target context for the spawn.
                        if let Some(win_idx) = self
                            .windows
                            .iter()
                            .position(|w| w.context_id == target_ctx_id)
                        {
                            log::info!(
                                "SpawnPane: target_context={target_ctx_id} — switching to window index {win_idx}"
                            );
                            self.active_window = win_idx;
                        } else {
                            log::warn!(
                                "SpawnPane: target_context={target_ctx_id} exists but has no window"
                            );
                            continue;
                        }
                    }

                    // Capture requesting_pane_id from the CURRENT focused pane (the calling app pane).
                    let active = self.active_window;
                    let requesting_pane_id = self.windows[active]
                        .focused_pane
                        .and_then(|tile| self.windows[active].tree.tiles.get(tile))
                        .and_then(|tile| {
                            if let egui_tiles::Tile::Pane(pid) = tile {
                                Some(*pid)
                            } else {
                                None
                            }
                        });

                    // Predict the pane id that will be allocated (next_pane_id peeks without allocating).
                    // A dedup can override this with the real existing pane id (#0336).
                    let mut new_pane_id = self.host.next_pane_id();
                    if type_id == "terminal" {
                        // "terminal" is a builtin pane type, not in the app registry.
                        // Resolve target window+tile from from_pane_id (cross-window search)
                        // or fall back to the active window's focused pane.
                        let vertical =
                            matches!(layout.as_str(), "split_h" | "split_right" | "split_left");
                        let new_pane_first =
                            matches!(layout.as_str(), "split_above" | "split_left");
                        let initial_cmd = cmd_from_args(&effective_args);
                        let close_on_exit = initial_cmd.is_some();
                        let (target_win, target_tile) = if let Some(from_id) = from_pane_id {
                            match self.find_pane_in_any_window(from_id) {
                                Some(loc) => {
                                    log::info!(
                                        "SpawnPane: targeting from_pane_id={from_id} in win_idx={}",
                                        loc.0
                                    );
                                    loc
                                }
                                None => {
                                    log::warn!("SpawnPane: from_pane_id={from_id} not found in any window, using focused pane");
                                    let Some(tile) = self.windows[active]
                                        .focused_pane
                                        .or(self.windows[active].tree.root)
                                    else {
                                        log::warn!(
                                            "SpawnPane: no target tile — window is empty, skipping"
                                        );
                                        self.active_window = original_active_window;
                                        continue;
                                    };
                                    (active, tile)
                                }
                            }
                        } else if let Some(tile) = self.windows[active].focused_pane {
                            (active, tile)
                        } else {
                            let Some(tile) = self.windows[active].tree.root else {
                                log::warn!("SpawnPane: no focused pane and empty tree — skipping");
                                self.active_window = original_active_window;
                                continue;
                            };
                            (active, tile)
                        };
                        log::info!(
                            "SpawnPane: terminal layout='{layout}' vertical={vertical} new_pane_first={new_pane_first} pane_id={new_pane_id} initial_cmd={initial_cmd:?} target_win={target_win}"
                        );
                        // SDK-spawned terminal with a cmd closes on exit (matches historical behavior).
                        // CLI terminal uses the ephemeral flag exclusively — cmd alone does not close.
                        // keep_focus=true: coordinator app always retains focus after spawning a terminal.
                        let _ = self.spawn_terminal_pane_at(
                            target_win,
                            target_tile,
                            vertical,
                            new_pane_first,
                            initial_cmd.as_deref(),
                            close_on_exit,
                            None,
                            true,
                        );
                    } else {
                        if let Some(from_id) = from_pane_id {
                            // When target_context already selected a window, restrict from_pane_id
                            // resolution to that window so it cannot override target_context.
                            let tile_opt = if target_context.is_some() {
                                let ctx_win = self.active_window;
                                self.windows[ctx_win]
                                    .tree
                                    .tiles
                                    .find_pane(&from_id)
                                    .map(|ft| (ctx_win, ft))
                            } else {
                                self.find_pane_in_any_window(from_id)
                            };
                            match tile_opt {
                                Some((fw, ft)) => {
                                    log::info!("SpawnPane: app: targeting from_pane_id={from_id} win_idx={fw}");
                                    self.active_window = fw;
                                    self.set_window_focused_pane(fw, ft);
                                }
                                None => {
                                    log::warn!("SpawnPane: app: from_pane_id={from_id} not found, using focused pane");
                                }
                            }
                        }
                        if let Ok(Some(existing_id)) = self.launch_app_by_id_with_layout(
                            &type_id,
                            Some(layout),
                            &effective_args,
                            None,
                        ) {
                            // Dedup focused a live instance; report its real id.
                            new_pane_id = existing_id;
                        }
                        log::info!("SpawnPane: launched '{type_id}' pane_id={new_pane_id}");
                    }

                    // Restore active_window if we switched for target_context or from_pane_id.
                    self.active_window = original_active_window;

                    // Send PaneSpawned back to the requesting pane (may be
                    // in a different window than where the spawn landed).
                    if let Some(req_pane_id) = requesting_pane_id {
                        let win_idx = self
                            .windows
                            .iter()
                            .position(|w| w.panes.contains_key(&req_pane_id));
                        if let Some(wi) = win_idx {
                            if let Some(pane) = self.windows[wi].panes.get_mut(&req_pane_id) {
                                if let Some(a) = pane.as_app_mut() {
                                    a.runtime.queue_outbound_event(
                                        crate::app_protocol::PlexiEvent::PaneSpawned {
                                            pane_id: new_pane_id,
                                            request_id,
                                        },
                                    );
                                }
                            }
                        }
                    }
                }
                AppCommand::CdRequest {
                    cwd,
                    sender_pane_id,
                } => {
                    // Explicit, app-requested cd only (#2145). Two delivery
                    // targets: a linked terminal pane, or — for overlay apps —
                    // the hidden terminal stored in `overlay_replaced`, whose
                    // PTY stays alive while the overlay covers it.
                    let active = self.active_window;
                    let escaped = cwd.replace('\'', "'\\''");
                    let cd_cmd = format!("\x15cd '{}'\n", escaped);
                    let linked_id = self.windows[active]
                        .panes
                        .get(&sender_pane_id)
                        .and_then(|p| p.as_app())
                        .and_then(|a| a.linked_pane_id);
                    let mut delivered = false;
                    if let Some(tid) = linked_id {
                        if let Some(t) = self.windows[active]
                            .panes
                            .get_mut(&tid)
                            .and_then(|p| p.as_terminal_mut())
                        {
                            t.backend.process_command(egui_term::BackendCommand::Write(
                                cd_cmd.as_bytes().to_vec(),
                            ));
                            log::info!(
                                "file_browser: CdRequest synced cwd '{}' to terminal pane {}",
                                cwd,
                                tid
                            );
                            delivered = true;
                        }
                    }
                    if !delivered {
                        if let Some(t) = self.windows[active]
                            .panes
                            .get_mut(&sender_pane_id)
                            .and_then(|p| p.as_app_mut())
                            .and_then(|a| a.overlay_replaced.as_deref_mut())
                            .and_then(|p| p.as_terminal_mut())
                        {
                            t.backend.process_command(egui_term::BackendCommand::Write(
                                cd_cmd.as_bytes().to_vec(),
                            ));
                            log::info!(
                                "file_browser: CdRequest synced cwd '{}' to overlay-hidden terminal under pane {}",
                                cwd,
                                sender_pane_id
                            );
                        } else {
                            log::warn!(
                                "file_browser: CdRequest from pane {} found no linked or overlay-hidden terminal; cwd '{}' dropped",
                                sender_pane_id,
                                cwd
                            );
                        }
                    }
                }
                AppCommand::Notify(_) => {}
                AppCommand::ShowNotification {
                    notify_id,
                    sender_pane_id,
                    source_context_id,
                    level,
                    title,
                    body,
                    kind,
                    options,
                    input_prompt,
                    required,
                    priority,
                    scope,
                    image_inline,
                    image_pipe_id,
                    timeout_secs,
                    on_dismiss,
                } => {
                    let (scope, notif_source_win_id) =
                        self.resolve_app_notification_provenance(sender_pane_id, scope);
                    // Strip any per-option shortcut that conflicts with navigation keys.
                    let options: Vec<crate::app_protocol::NotifyOption> = options.into_iter().map(|mut opt| {
                        if let Some(ref sc) = opt.shortcut.clone() {
                            if crate::app_protocol::is_reserved_shortcut(sc) {
                                log::warn!(
                                    "notify:shortcut: app pane {} sent reserved shortcut {:?} on option {:?} — stripped",
                                    sender_pane_id, sc, opt.label
                                );
                                opt.shortcut = None;
                            }
                        }
                        opt
                    }).collect();
                    self.enqueue_notification(
                        crate::app::notifications::NotifySource::App,
                        PendingNotification {
                            notify_id,
                            sender_pane_id,
                            source_context_id,
                            source_window_id: notif_source_win_id,
                            level,
                            title,
                            body,
                            kind,
                            options,
                            input_prompt,
                            required,
                            priority,
                            scope,
                            image_inline,
                            image_pipe_id,
                            response_file: None,
                            timeout_secs,
                            on_dismiss,
                            enqueued_at: std::time::Instant::now(),
                            tombstoned: false,
                            deliver_after: None,
                        },
                    );
                }
                AppCommand::DeliverNotifyAction {
                    pane_id,
                    notify_id,
                    action_label,
                    value,
                    response_file,
                    host_action,
                } => {
                    log::info!(
                        "notify:action: pane_id={pane_id} notify_id={notify_id:?} value={value:?} host_action={host_action:?}"
                    );
                    crate::host::event_log::emit(
                        crate::host::event_log::HostEvent::NotificationActionInvoked {
                            id: notify_id.clone(),
                            action: action_label.clone(),
                            timestamp: crate::host::event_log::now_timestamp(),
                        },
                    );
                    // Execute host-side action synchronously before writing the response
                    // file so the navigation is complete before the shell unblocks.
                    if let Some(ref action) = host_action {
                        if let Some(id_str) = action.strip_prefix("pane_focus:") {
                            if let Ok(pane_id_target) = id_str.parse::<u64>() {
                                self.pane_navigate(pane_id_target);
                            } else {
                                log::warn!(
                                    "notify:action: pane_focus: invalid pane_id {:?}",
                                    id_str
                                );
                            }
                        } else {
                            log::warn!("notify:action: unknown host_action {:?}", action);
                        }
                    }
                    if let Some(rf) = &response_file {
                        let content = value.as_deref().unwrap_or("");
                        if crate::rpc::write_response(rf, content.as_bytes()) {
                            log::info!("notify:action: wrote {:?} to {:?}", content, rf);
                        }
                    }
                    // Search all windows for the sender pane — it may not be in the
                    // active context (cross-context notification path).
                    let window_idx = self
                        .windows
                        .iter()
                        .position(|w| w.panes.contains_key(&pane_id));
                    if let Some(win_idx) = window_idx {
                        if let Some(pane) = self.windows[win_idx].panes.get_mut(&pane_id) {
                            if let Some(app) = pane.as_app_mut() {
                                app.runtime.queue_outbound_event(
                                    crate::app_protocol::PlexiEvent::NotifyAction {
                                        notify_id,
                                        action_label,
                                        value,
                                    },
                                );
                            }
                        }
                    }
                }
                AppCommand::DeliverPipeMessage {
                    sender_pane_id,
                    pipe_id,
                    payload,
                } => {
                    let active = self.active_window;
                    // JSON pipe messages route only through directed pipes
                    // (#286): the host scopes delivery to the non-sender member
                    // of the recorded `(sender, target)` pair. The legacy
                    // non-directed peer-broadcast fan-out was removed in 0327 —
                    // event streams (subscription-scoped, cross-window) replace
                    // it. A send on a pipe that was never opened directed is
                    // dropped with a warning.
                    let Some(&(a, b)) = self.directed_pipes.get(&pipe_id) else {
                        log::warn!(
                            "DeliverPipeMessage: '{pipe_id}' from pane {sender_pane_id} is not a directed pipe; dropping (non-directed JSON pipes were removed in 0327)"
                        );
                        continue;
                    };
                    let target_pid = if sender_pane_id == a {
                        Some(b)
                    } else if sender_pane_id == b {
                        Some(a)
                    } else {
                        // Neither side — wire mismatch. Log + drop.
                        log::warn!(
                            "DeliverPipeMessage: directed pipe '{pipe_id}' \
                             sender {sender_pane_id} not in pair ({a}, {b}); dropping"
                        );
                        None
                    };
                    if let Some(tid) = target_pid {
                        if let Some(pane) = self.windows[active].panes.get_mut(&tid) {
                            let event = crate::app_protocol::PlexiEvent::PipeMessage {
                                pipe_id: pipe_id.clone(),
                                payload: payload.clone(),
                            };
                            if let Some(app) = pane.as_app_mut() {
                                app.runtime.queue_outbound_event(event);
                            }
                        }
                    }
                }
                AppCommand::OpenDirectedPipe {
                    sender_pane_id,
                    pipe_id,
                    target_pane_id,
                } => {
                    // Subscribe both sides + record the pair so subsequent
                    // `DeliverPipeMessage` for this pipe routes ONLY between
                    // them (#286). The sender already registered the pipe
                    // locally inside its own app runtime; we need to register
                    // it on the target so its `has_reader` returns true and
                    // its SDK has a Pipe handle if it sends in reverse.
                    let active = self.active_window;
                    let target_kind =
                        self.windows[active]
                            .panes
                            .get(&target_pane_id)
                            .map(|p| match p {
                                crate::host::pane::Pane::App(_) => "app",
                                crate::host::pane::Pane::Terminal(_) => "terminal",
                                crate::host::pane::Pane::Portal(_) => "portal",
                            });
                    match target_kind {
                        Some("app") => {
                            // Register pipe on target's registry so it can
                            // PipeSend back through the same id.
                            let registered = if let Some(pane) =
                                self.windows[active].panes.get_mut(&target_pane_id)
                            {
                                register_directed_pipe_on_target(pane, &pipe_id)
                            } else {
                                false
                            };
                            if !registered {
                                log::warn!(
                                    "OpenDirectedPipe: failed to register '{pipe_id}' on target {target_pane_id}"
                                );
                                continue;
                            }
                            self.directed_pipes
                                .insert(pipe_id.clone(), (sender_pane_id, target_pane_id));
                            log::info!(
                                "OpenDirectedPipe: '{pipe_id}' subscribed pane {sender_pane_id} ↔ pane {target_pane_id}"
                            );
                        }
                        Some(other) => log::warn!(
                            "OpenDirectedPipe: target {target_pane_id} is a {other}; expected app — pipe '{pipe_id}' not subscribed"
                        ),
                        None => log::warn!(
                            "OpenDirectedPipe: target pane {target_pane_id} not found; pipe '{pipe_id}' dropped"
                        ),
                    }
                }
                AppCommand::RequestLinkedTerminal {
                    sender_pane_id,
                    request_id,
                    cwd,
                    label: _label,
                    place_below,
                } => {
                    self.dispatch_request_linked_terminal(
                        sender_pane_id,
                        request_id,
                        cwd,
                        place_below,
                    );
                }
                AppCommand::RunInLinkedTerminal {
                    sender_pane_id,
                    terminal_pane_id,
                    command,
                    echo,
                } => {
                    self.dispatch_run_in_linked_terminal(
                        sender_pane_id,
                        terminal_pane_id,
                        command,
                        echo,
                    );
                }
                AppCommand::InsertPathToken {
                    sender_pane_id,
                    terminal_pane_id,
                    path,
                    mode,
                } => {
                    self.dispatch_insert_path_token(sender_pane_id, terminal_pane_id, path, mode);
                }
                AppCommand::RequestCommandPreview {
                    sender_pane_id,
                    request_id,
                    terminal_pane_id,
                    command,
                } => {
                    self.dispatch_command_preview(
                        sender_pane_id,
                        request_id,
                        terminal_pane_id,
                        command,
                    );
                }
                AppCommand::OpenArtifact {
                    sender_pane_id,
                    path,
                    mode,
                } => {
                    self.dispatch_open_artifact(sender_pane_id, path, mode);
                }
                AppCommand::QueryContextState {
                    sender_pane_id,
                    context_id,
                } => {
                    // Visibility check: the requesting pane must be in the
                    // queried context itself or in an ancestor of it.
                    let requester_context_id = self
                        .windows
                        .iter()
                        .find(|w| w.panes.contains_key(&sender_pane_id))
                        .map(|w| w.context_id)
                        .unwrap_or(0);

                    let is_self = requester_context_id == context_id;
                    let is_ancestor = if !is_self {
                        self.host
                            .ancestors_of(context_id)
                            .contains(&requester_context_id)
                    } else {
                        false
                    };

                    if !is_self && !is_ancestor {
                        log::warn!(
                            "QueryContextState: pane {sender_pane_id} in context {requester_context_id} \
                             cannot query context {context_id} — not an ancestor"
                        );
                        continue;
                    }

                    let state = crate::host::context_state::ContextState::compute(
                        context_id,
                        self.router.as_slice(),
                        &self.windows,
                    );
                    log::info!(
                        "QueryContextState: context_id={context_id} pane_count={} children={} status={:?}",
                        state.pane_count,
                        state.children.len(),
                        state.status,
                    );

                    // Send response back to the requesting pane.
                    let window_idx = self
                        .windows
                        .iter()
                        .position(|w| w.panes.contains_key(&sender_pane_id));
                    if let Some(win_idx) = window_idx {
                        if let Some(pane) = self.windows[win_idx].panes.get_mut(&sender_pane_id) {
                            if let Some(app) = pane.as_app_mut() {
                                app.runtime.queue_outbound_event(
                                    crate::app_protocol::PlexiEvent::ContextStateResponse { state },
                                );
                            }
                        }
                    }
                }
                AppCommand::DeliverRunUpdate {
                    originator_type_id,
                    event,
                } => {
                    let active = self.active_window;
                    let pane_ids: Vec<_> = self.windows[active].panes.keys().copied().collect();
                    let mut delivered = false;
                    for pid in pane_ids {
                        let matches = self.windows[active]
                            .panes
                            .get(&pid)
                            .and_then(|p| p.as_app())
                            .map(|a| a.manifest_id == originator_type_id)
                            .unwrap_or(false);
                        if matches {
                            if let Some(pane) = self.windows[active].panes.get_mut(&pid) {
                                if let Some(app) = pane.as_app_mut() {
                                    app.runtime.queue_outbound_event(event.clone());
                                    delivered = true;
                                    break;
                                }
                            }
                        }
                    }
                    if !delivered {
                        log::warn!(
                            "DeliverRunUpdate: no pane found for type_id='{originator_type_id}'"
                        );
                    }
                }
            }
        }
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
            // per notification here. An app that omits the field gets
            // `DefaultNotifyScope`'s own default (`Window`, the most
            // restrictive), which is a declared per-app policy and so does not
            // go through `NotifyScope::default()`.
            let resolved_scope = self.registry.default_notification_scope_for(&type_id);
            for cmd in cmds {
                match cmd {
                    AppCommand::WasmHostEffect { effect, .. } => {
                        deferred.push(AppCommand::WasmHostEffect { pane_id, effect });
                    }
                    AppCommand::ExposeTools { tools, .. } => {
                        deferred.push(AppCommand::ExposeTools {
                            tools,
                            pane_id: Some(pane_id),
                        });
                    }
                    AppCommand::ToolResult { .. } => deferred.push(cmd),
                    AppCommand::AppEventRequest { request, .. } => {
                        deferred.push(AppCommand::AppEventRequest {
                            request,
                            pane_id: Some(pane_id),
                        });
                    }
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
