//! App command dispatch — keyboard routing + command drain from view panes.

use crate::app::notifications::PendingNotification;
use crate::app::view_trait::ViewCommand;

use super::{is_overlay_unsafe_cmd, overlay_unsafe_cmd_name, register_directed_pipe_on, PlexiApp};

pub(crate) fn native_wasm_notification_contract_error(icon: Option<&str>) -> Option<&'static str> {
    icon.map(|_| "native WASM notifications support title and body only; icon is unsupported")
}

impl PlexiApp {
    /// Feed keyboard input to the focused view pane. Keys only go to the
    /// focused pane (that's the whole point of focus). Command drain
    /// happens separately via [`drain_all_view_commands`] so background
    /// plugins can surface notifications and other commands while the
    /// focused pane is something else (or while a modal holds focus).
    pub(super) fn dispatch_plugin_key_events(
        &mut self,
        ctx: &egui::Context,
        input: &mut crate::app::input_router::PlexiInput,
    ) {
        // Block key delivery when the OS has focus elsewhere. `active_window` is not
        // cleared on blur, so without this guard keys route to the last-active pane
        // even while a different plugin (or Plexi window) owns the keyboard.
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
        let Some(view_pane) = pane.as_view_mut() else {
            return;
        };
        // A focused declarative TextInput owns the keyboard (stint 0456):
        // leave the frame's events in the router so the render pass's
        // TextEdit consumes them (typing, arrows, Enter-submit). Escape
        // surrenders egui focus natively, flipping this gate back off so
        // keys reach the plugin's KeyEvent path again next frame.
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
                    "plugin key events"
                }
            );
            ctx.memory_mut(|m| m.data.insert_temp(gate_id, text_surface_focused));
        }
        if text_surface_focused {
            // Most focused declarative TextInputs own every key until their
            // render pass. CLI renderer forms are the narrow exception: their
            // single-line fields advertise bare Enter as the Run action, so
            // route and claim it before TextEdit can turn it into a newline.
            // Cmd+Enter remains available to the host pane-zoom binding.
            let bare_enter: Vec<egui::Event> = input
                .events()
                .iter()
                .filter(|event| {
                    matches!(
                        event,
                        egui::Event::Key {
                            key: egui::Key::Enter,
                            pressed: true,
                            modifiers,
                            ..
                        } if modifiers.is_none()
                    )
                })
                .cloned()
                .collect();
            if !bare_enter.is_empty() && view_pane.runtime.submit_on_focused_text_input_enter() {
                let synthetic =
                    crate::app::input_router::PlexiInput::synthetic(bare_enter, input.modifiers());
                if view_pane.runtime.handle_key(&synthetic)
                    == crate::app::view_trait::KeyDisposition::Consumed
                {
                    input.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
                    log::info!(
                        "app_keys: pane {pane_id} bare Enter submitted from focused TextInput"
                    );
                }
            }
            if input.events().iter().any(|event| {
                matches!(
                    event,
                    egui::Event::Key {
                        key: egui::Key::Enter,
                        pressed: true,
                        ..
                    }
                )
            }) {
                log::info!(
                    "app_keys: pane {pane_id} Enter owned by focused TextInput — raw plugin KeyEvent suppressed"
                );
            }
            // Bare vertical-arrow presses still reach the plugin so a search-results
            // list can navigate while the query box keeps focus for typing
            // (stint 0729 follow-up). Every other key — printable characters,
            // Backspace/Delete, Left/Right/Home/End, Enter, modified chords —
            // is left in the buffer for the render pass's TextEdit, unchanged.
            // A single-line TextEdit has no vertical layout to move within, so
            // egui's cursor_range handling for ArrowUp/ArrowDown is a no-op on
            // position (it only collapses an existing selection); claiming the
            // events here avoids even that redundant pass once the plugin has
            // consumed them.
            let arrow_presses: Vec<egui::Event> = input
                .events()
                .iter()
                .filter(|event| {
                    matches!(
                        event,
                        egui::Event::Key {
                            key: egui::Key::ArrowUp | egui::Key::ArrowDown,
                            pressed: true,
                            modifiers,
                            ..
                        } if modifiers.is_none()
                    )
                })
                .cloned()
                .collect();
            if !arrow_presses.is_empty() {
                let synthetic = crate::app::input_router::PlexiInput::synthetic(
                    arrow_presses,
                    input.modifiers(),
                );
                let disposition = view_pane.runtime.handle_key(&synthetic);
                if disposition == crate::app::view_trait::KeyDisposition::Consumed {
                    input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp);
                    input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown);
                    log::info!(
                        "app_keys: pane {pane_id} vertical arrow routed to plugin past the focused TextInput"
                    );
                }
            }
            return;
        }
        // Egui releases a TextInput's focus before this dispatch. On the
        // physical Escape that did so, hand only that bare key to the plugin via
        // its dedicated bridge path, then consume it before CloseView sees it.
        // Normal `handle_key` remains Escape-reserving for every other case.
        let text_input_escape: Vec<egui::Event> = input
            .events()
            .iter()
            .filter(|event| {
                matches!(
                    event,
                    egui::Event::Key {
                        key: egui::Key::Escape,
                        pressed: true,
                        modifiers,
                        ..
                    } if modifiers.is_none()
                )
            })
            .cloned()
            .collect();
        if previously_focused && !text_input_escape.is_empty() {
            let synthetic = crate::app::input_router::PlexiInput::synthetic(
                text_input_escape,
                input.modifiers(),
            );
            let disposition = view_pane.runtime.handle_text_input_escape(&synthetic);
            input.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
            log::info!(
                "app_keys: pane {pane_id} Escape left focused TextInput — dedicated delivery disposition={disposition:?}, CloseView suppressed"
            );
            return;
        }
        // The plugin classifies keys from the frame's ownership-transfer buffer
        // (stint 0387) rather than a second read of `ctx`. Bare Escape while
        // zoomed reaches `handle_key` without un-zooming (stint 0898). If the
        // plugin consumed the key, also claim Escape/ArrowUp out of the buffer so
        // `poll_actions` (which takes the same buffer later this frame) can't
        // re-fire CloseView for keys the plugin already handled — e.g. Escape
        // exiting search mode in the file browser rather than closing the
        // pane, or ArrowUp recalling a prior message in the assistant.
        let disposition = view_pane.runtime.handle_key(input);
        if disposition == crate::app::view_trait::KeyDisposition::Consumed {
            input.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
            input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp);
        }
    }

    /// End the key holds of every terminal that does not own the keyboard
    /// this frame (stint 1564). Such a terminal never receives the physical
    /// releases (window blurred, focus moved, an overlay took input), and a
    /// hidden pane (behind a zoom, in another context) never renders, so this
    /// runs from `App::logic` over all panes rather than from the view.
    pub(super) fn release_unowned_terminal_holds(&mut self, ctx: &egui::Context) {
        let owner = match self.input_owner(ctx) {
            crate::app::input_owner::InputOwner::Pane(pane_id) => Some(pane_id),
            _ => None,
        };
        for (pane_id, pane) in self.windows.iter_mut().flat_map(|w| w.panes.iter_mut()) {
            if Some(*pane_id) != owner {
                if let Some(terminal) = pane.as_terminal_mut() {
                    terminal.backend.release_held_keys();
                }
            }
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
    /// the focus stack, including context rename, an unfocused OS
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
        self.windows
            .iter()
            .find_map(|w| w.panes.get(&pane_id))?
            .as_terminal()?;

        let router = crate::app::input_router::PlexiInput::take_from(ctx);
        let (keyboard_events, modifiers) = router.into_events_and_modifiers();
        if !self.synthetic_input_frame
            && keyboard_events.iter().any(|event| match event {
                egui::Event::Text(text)
                | egui::Event::Paste(text)
                | egui::Event::Ime(egui::ImeEvent::Commit(text)) => !text.is_empty(),
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => {
                    matches!(
                        key,
                        egui::Key::Enter
                            | egui::Key::Backspace
                            | egui::Key::Delete
                            | egui::Key::Tab
                    ) && !modifiers.command
                }
                _ => false,
            })
        {
            self.record_agent_interaction(pane_id, "input");
        }

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

    /// True when any pane or parked background plugin needing a headless tick
    /// (a non-active context always; the active context too when
    /// `window_hidden`, per the same reasoning as
    /// [`Self::drain_all_view_commands`]) has pending background work. Drives
    /// the bounded 100ms wake loop in `update()` — this MUST use the same
    /// predicate as the tick gate in [`Self::drain_all_view_commands`],
    /// otherwise work could be marked pending without a frame ever being
    /// scheduled to drain it (#2021).
    pub(crate) fn background_processes_need_wake(&self, window_hidden: bool) -> bool {
        let active = self.active_window;
        let inactive_context_needs_wake =
            self.windows.iter().enumerate().any(|(ctx_idx, context)| {
                (ctx_idx != active || window_hidden)
                    && context.panes.values().any(|pane| {
                        pane.as_view()
                            .is_some_and(|view_pane| view_pane.runtime.needs_background_tick())
                    })
            });

        inactive_context_needs_wake
            || self
                .background_apps
                .values()
                .any(|(_, app)| app.needs_background_tick())
    }

    /// Drain every view pane's outbound commands and execute them (stint 0688).
    ///
    /// This is external-client servicing, not rendering: a view pane answers
    /// assistant tool calls, emits notifications, and requests host actions on
    /// its own schedule, with no dependence on being painted. It therefore runs
    /// from [`eframe::App::logic`], per the rule in the root `AGENTS.md` —
    /// eframe skips `App::ui` entirely while the window is minimized or fully
    /// occluded, and this drain living in `ui` meant a covered host executed no
    /// plugin command of any runtime. Every `todo.list` / `calc.add` call to such a
    /// host sat here until the broker's 30s timeout fired.
    ///
    /// Overlay state is consulted (`input_captured_by_overlay`) but never
    /// rendered here: unsafe commands are held while a modal owns input and
    /// released on the next modal-free pass, exactly as before.
    pub(super) fn service_plugin_commands(&mut self, ctx: &egui::Context) {
        self.sync_agents_panes(ctx);
        let mut config_saved = false;
        for window in &mut self.windows {
            for pane in window.panes.values_mut() {
                if let Some(view_pane) = pane.as_view_mut() {
                    if view_pane.runtime.take_config_saved() {
                        config_saved = true;
                    }
                }
            }
        }
        if config_saved {
            log::info!("config: reloading after builtin editor saved config.toml");
            self.reload_config();
        }
        // eframe skips `App::ui` entirely while the window is hidden
        // (minimized or fully occluded) — the active context's own panes
        // then get no `ui()` pass either, so they need the same headless
        // tick as a non-active context (stint 0759; see
        // `drain_all_view_commands`).
        let window_hidden = ctx.input(|i| i.viewport().visible()) == Some(false);
        // Drain every view pane's pending_commands every frame — including
        // while a modal holds focus. Background plugins emitting notifications
        // must reach the queue *now*, not be buffered until the modal
        // closes (which caused the "ghost queue appears on reopen" bug).
        let fresh_cmds = self.drain_all_plugin_commands(window_hidden);
        // An end whose guest finished while its pane stays mounted is retired
        // before anything this pass can open to it or send on it (stint 1681).
        self.retire_directed_pipes_of_exited_ends();
        if self.background_processes_need_wake(window_hidden) {
            crate::platform::frame_diag::note(
                crate::platform::frame_diag::RepaintCause::ViewIdlePoll,
            );
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        // When the overlay releases, prepend any held unsafe commands so they
        // execute before new commands this frame (FIFO order preserved).
        let deferred_plugin_cmds: Vec<_> =
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

        // Handle deferred plugin commands returned from dispatch_plugin_key_events.
        for cmd in deferred_plugin_cmds {
            use crate::app::view_trait::ViewCommand;
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
                ViewCommand::WasmHostEffect {
                    pane_id,
                    effect,
                    emitter,
                } => {
                    use crate::host::wasm_pane::WasmHostEffect;
                    use crate::host::wasm_view::InputEvent;

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
                                if let Some(error) =
                                    native_wasm_notification_contract_error(icon.as_deref())
                                {
                                    let error = error.to_string();
                                    log::warn!(
                                        "wasm effect: notify rejected pane_id={pane_id}: {error}"
                                    );
                                    Some(InputEvent::NotifyResult(Err(error)))
                                } else {
                                    let source_context_id = self.windows[window_index].context_id;
                                    let source_window_id = self.windows[window_index].window_id;
                                    let notify_id = crate::app::notifications::new_notify_id(
                                        &format!("wasm:{pane_id}"),
                                    );
                                    log::info!(
                                    "wasm effect: notify pane_id={pane_id} context_id={source_context_id}"
                                );
                                    // `WasmHostEffect::Notify` carries no scope, so
                                    // a guest gets the shared default and cannot
                                    // request global; that needs a WIT change.
                                    let queued = self.enqueue_notification(
                                        crate::app::notifications::NotifySource::Wasm,
                                        PendingNotification {
                                            notify_id,
                                            sender_pane_id: pane_id,
                                            source_context_id,
                                            source_window_id,
                                            title,
                                            body,
                                            ..Default::default()
                                        },
                                    );
                                    Some(InputEvent::NotifyResult(queued.then_some(()).ok_or_else(|| {
                                    "notification was rejected because delivery is disabled or the queue is full".to_string()
                                })))
                                }
                            } else {
                                let error = format!("source pane {pane_id} is closed");
                                log::warn!("wasm effect: notify failed: {error}");
                                Some(InputEvent::NotifyResult(Err(error)))
                            }
                        }
                        WasmHostEffect::Spawn {
                            plugin_id,
                            layout,
                            args,
                        } => {
                            let predicted_pane_id = self.host.next_pane_id();
                            // A component's terminal is admitted as that view's,
                            // like `SpawnPane` (stint 1666).
                            let result = if plugin_id == "terminal" {
                                match emitter {
                                    Some(emitter) => self.serve_legacy_view_terminal(
                                        Some(emitter),
                                        layout.as_deref(),
                                        &args,
                                    ),
                                    None => {
                                        log::warn!(
                                            "wasm effect: spawn pane_id={pane_id} terminal refused: no emitting view"
                                        );
                                        Err("terminal spawn carried no emitting view".to_string())
                                    }
                                }
                            } else {
                                self.launch_view_by_id_with_layout(&plugin_id, layout, &args, None)
                                    .map(|existing| existing.unwrap_or(predicted_pane_id))
                            };
                            log::info!(
                                "wasm effect: spawn pane_id={pane_id} plugin_id={plugin_id} ok={}",
                                result.is_ok(),
                                plugin_id = crate::host::guest_text::shown(&plugin_id)
                            );
                            Some(InputEvent::SpawnResult(result))
                        }
                        WasmHostEffect::SubscribeEvents {
                            request_id,
                            plugin_id,
                            event_names,
                            payload_mode,
                            trigger_mode,
                            resource_id,
                        } => {
                            self.begin_plugin_event_subscription(
                                pane_id,
                                request_id,
                                plugin_id,
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
                            let result = crate::host::plugin_timeline::global()
                                .lock()
                                .unwrap()
                                .remove_subscription_for(
                                    crate::broker::ActorType::Plugin,
                                    &crate::host::event_subscriptions::plugin_subscriber_id(
                                        pane_id,
                                    ),
                                    &subscription_id,
                                );
                            let (removed, error) = match result {
                                Ok(removed) => (removed, None),
                                Err(error) => (false, Some(error)),
                            };
                            Some(InputEvent::EventUnsubscriptionResult(
                                crate::host::wasm_view::bindings::plexi::platform::types::EventUnsubscriptionResultEvent {
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
                                    pane.as_view().and_then(|app| {
                                        app.runtime.tool_event_sender(ctx.clone()).map(|sender| {
                                            (
                                                app.tool_identity(),
                                                sender,
                                                crate::plexi_ai::tool_dispatch::ViewInstanceKey::of(
                                                    app,
                                                ),
                                            )
                                        })
                                    })
                                })
                            });
                            if let (Some((plugin_id, sender, owner)), Some(origin)) =
                                (provider, self.origin_for_pane(pane_id))
                            {
                                crate::plexi_ai::tool_dispatch::register(
                                    pane_id, owner, plugin_id, tools, sender, origin,
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
                            error_code,
                        } => {
                            crate::plexi_ai::tool_dispatch::resolve_pending(
                                &call_id,
                                crate::plexi_ai::tool_dispatch::ToolCallResult {
                                    output_json,
                                    error,
                                    error_code,
                                    pending_request_id: None,
                                },
                            );
                            None
                        }
                    };
                    if let Some(event) = event {
                        self.complete_wasm_host_effect(pane_id, event);
                    }
                }
                ViewCommand::ExposeTools {
                    tools,
                    pane_id: Some(pane_id),
                } => {
                    let provider = self.windows.iter().find_map(|window| {
                        window.panes.get(&pane_id).and_then(|pane| {
                            pane.as_view().and_then(|app| {
                                app.runtime.tool_event_sender(ctx.clone()).map(|sender| {
                                    (
                                        app.tool_identity(),
                                        sender,
                                        crate::plexi_ai::tool_dispatch::ViewInstanceKey::of(app),
                                    )
                                })
                            })
                        })
                    });
                    if let (Some((plugin_id, sender, owner)), Some(origin)) =
                        (provider, self.origin_for_pane(pane_id))
                    {
                        crate::plexi_ai::tool_dispatch::register(
                            pane_id, owner, plugin_id, tools, sender, origin,
                        );
                    } else {
                        log::warn!(
                            "tool_dispatch: ignore ExposeTools from unsupported pane {pane_id}"
                        );
                    }
                }
                ViewCommand::ExposeTools { pane_id: None, .. } => {
                    log::warn!("tool_dispatch: ExposeTools missing source pane")
                }
                ViewCommand::ToolResult {
                    call_id,
                    output_json,
                    error,
                    error_code,
                } => crate::plexi_ai::tool_dispatch::resolve_pending(
                    &call_id,
                    crate::plexi_ai::tool_dispatch::ToolCallResult {
                        output_json,
                        error,
                        error_code,
                        pending_request_id: None,
                    },
                ),
                ViewCommand::PluginEventRequest {
                    request,
                    pane_id: Some(pane_id),
                } => {
                    let plugin_id = self.windows.iter().find_map(|window| {
                        window
                            .panes
                            .get(&pane_id)
                            .and_then(|pane| pane.as_view())
                            .map(|app| app.manifest_id.clone())
                    });
                    let Some(plugin_id) = plugin_id else {
                        log::warn!("plugin events: source pane {pane_id} is closed");
                        continue;
                    };
                    // Host-established context this plugin instance lives in
                    // (stint 0724 Phase D) — the owning scope every stream
                    // this pane declares/emits on is stamped with. Resolved
                    // fresh here, never cached: the pane just resolved above
                    // is live, so its owning window's `context_id` is too.
                    let Some(context_id) = self
                        .origin_for_pane(pane_id)
                        .map(|origin| origin.context_id)
                    else {
                        log::warn!("plugin events: source pane {pane_id} has no resolvable origin");
                        continue;
                    };
                    let response = match request {
                        crate::protocol::HostRequest::DeclareEventStreams { streams } => {
                            match crate::host::plugin_timeline::global()
                                .lock()
                                .unwrap()
                                .declare_streams(context_id, &plugin_id, streams)
                            {
                                Ok(streams) => {
                                    crate::protocol::PlexiEvent::DeclareEventStreamsResult {
                                        streams: Some(streams),
                                        error: None,
                                    }
                                }
                                Err(error) => {
                                    crate::protocol::PlexiEvent::DeclareEventStreamsResult {
                                        streams: None,
                                        error: Some(error),
                                    }
                                }
                            }
                        }
                        crate::protocol::HostRequest::EmitEvent {
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
                            let emitted = crate::host::plugin_timeline::EmittedEvent {
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
                            match crate::host::plugin_timeline::global()
                                .lock()
                                .unwrap()
                                .record_event(context_id, &plugin_id, pane_id, emitted)
                            {
                                Ok(outcome) => crate::protocol::PlexiEvent::EmitEventResult {
                                    sequence: Some(outcome.event_id),
                                    error: None,
                                },
                                Err(error) => crate::protocol::PlexiEvent::EmitEventResult {
                                    sequence: None,
                                    error: Some(error),
                                },
                            }
                        }
                        crate::protocol::HostRequest::SubscribePluginEvents {
                            request_id,
                            plugin_id: publisher_plugin_id,
                            event_names,
                            payload_mode,
                            trigger_mode,
                            resource_id,
                        } => {
                            self.begin_plugin_event_subscription(
                                pane_id,
                                request_id,
                                publisher_plugin_id,
                                event_names,
                                payload_mode,
                                trigger_mode,
                                resource_id,
                            );
                            continue;
                        }
                        crate::protocol::HostRequest::UnsubscribePluginEvents {
                            request_id,
                            subscription_id,
                        } => {
                            let result = crate::host::plugin_timeline::global()
                                .lock()
                                .unwrap()
                                .remove_subscription_for(
                                    crate::broker::ActorType::Plugin,
                                    &crate::host::event_subscriptions::plugin_subscriber_id(
                                        pane_id,
                                    ),
                                    &subscription_id,
                                );
                            match result {
                                Ok(removed) => {
                                    crate::protocol::PlexiEvent::PluginEventsUnsubscribed {
                                        request_id,
                                        removed,
                                        error: None,
                                    }
                                }
                                Err(error) => {
                                    crate::protocol::PlexiEvent::PluginEventsUnsubscribed {
                                        request_id,
                                        removed: false,
                                        error: Some(error),
                                    }
                                }
                            }
                        }
                        other => {
                            log::warn!("plugin events: unsupported plugin request {other:?}");
                            continue;
                        }
                    };
                    self.queue_event_to_view_pane_anywhere(pane_id, response);
                }
                ViewCommand::PluginEventRequest { pane_id: None, .. } => {
                    log::warn!("plugin events: request missing source pane")
                }
                ViewCommand::AssistantHostTool {
                    cancel,
                    name,
                    input_json,
                    origin_pane_id,
                    origin_context_id,
                    reply,
                } => {
                    if cancel.is_cancelled() {
                        let _ = reply.send(crate::plexi_ai::tool_dispatch::ToolCallResult::err(
                            "cancelled: turn ended",
                        ));
                        continue;
                    }
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
                // Capability-gated pane read/control request from a PGAP plugin
                // (stint 0013/0014). routing.rs already checked panes.read /
                // panes.control; execute it through the same handler CLI
                // requests take over PLEXI_SOCKET.
                ViewCommand::Fleet(action) => match action {
                    crate::app::agents_view::FleetAction::Seen(id) => self.mark_agent_seen(id),
                    crate::app::agents_view::FleetAction::Dismiss(id) => {
                        self.dismiss_fleet_agent(Some(id), false);
                    }
                    crate::app::agents_view::FleetAction::ReviewAttention { pane_id } => {
                        self.sync_attention_queue();
                        if let Some(ask_id) = self
                            .attention_queue
                            .first_ask_for_pane(pane_id)
                            .map(|b| b.id.clone())
                        {
                            let _ = self.open_decision_overlay_for_ask(&ask_id);
                        } else if !self.attention_queue.open {
                            self.toggle_attention_queue_list();
                        }
                    }
                    crate::app::agents_view::FleetAction::Reply {
                        pane_id,
                        from_pane_id,
                        text,
                    } => {
                        self.reply_to_fleet_agent(pane_id, from_pane_id, text);
                    }
                },
                ViewCommand::ForwardPaneRequest { request, origin } => {
                    self.handle_pane_ipc_request(origin, request);
                }
                ViewCommand::SpawnView {
                    type_id,
                    layout,
                    args,
                    emitter,
                } => {
                    // Predict the pane id, but let a dedup override it with the
                    // real existing instance's id (#0336): `Ok(Some(id))` means
                    // the launch focused a live instance instead of spawning.
                    let predicted_pane_id = self.host.next_pane_id();
                    // A view's terminal is that view's, in its own context:
                    // the same emitter-bound admission as `SpawnPane`, never
                    // a desktop terminal in the active window (stint 1666).
                    let launched = if type_id == "terminal" {
                        self.serve_legacy_view_terminal(emitter, layout.as_deref(), &args)
                            .map(Some)
                    } else {
                        self.launch_view_by_id_with_layout(&type_id, layout, &args, None)
                    };
                    let new_pane_id = match launched {
                        Ok(Some(existing_id)) => existing_id,
                        Ok(None) => predicted_pane_id,
                        Err(e) => {
                            // Fail loud: never swallow a launch failure or confirm
                            // a spawn that did not happen (#2283).
                            log::warn!(
                                "SpawnView: launch of '{type_id}' failed — {e}; not confirming ViewOpened", type_id = crate::host::guest_text::shown(&type_id)
                            );
                            continue;
                        }
                    };
                    // Confirm to the view that emitted the request, wherever
                    // focus is now (stint 1632).
                    self.deliver_to_emitter(
                        emitter,
                        crate::protocol::PlexiEvent::ViewOpened {
                            pane_id: new_pane_id,
                            type_id: type_id.clone(),
                        },
                    );
                }
                ViewCommand::SpawnPane {
                    type_id,
                    layout,
                    args,
                    from_pane_id,
                    request_id,
                    target_context,
                    emitter,
                } => {
                    let outcome = self.serve_sdk_spawn_pane(
                        emitter,
                        &type_id,
                        &layout,
                        args,
                        from_pane_id,
                        target_context,
                    );
                    self.reply_sdk_spawn_pane(emitter, request_id, outcome);
                }
                ViewCommand::CdRequest {
                    cwd,
                    sender_pane_id,
                } => {
                    // Explicit, plugin-requested cd only (#2145). Two delivery
                    // targets: a linked terminal pane, or — for overlay plugins —
                    // the hidden terminal stored in `overlay_replaced`, whose
                    // PTY stays alive while the overlay covers it.
                    let active = self.active_window;
                    let escaped = cwd.replace('\'', "'\\''");
                    let cd_cmd = format!("\x15cd '{}'\n", escaped);
                    let linked_id = self.windows[active]
                        .panes
                        .get(&sender_pane_id)
                        .and_then(|p| p.as_view())
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
                            .and_then(|p| p.as_view_mut())
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
                ViewCommand::ShowNotification {
                    notify_id,
                    sender_pane_id,
                    source_context_id,
                    title,
                    body,
                    kind,
                    options,
                    input_prompt,
                    required,
                    scope,
                    image_inline,
                    image_pipe_id,
                    timeout_secs,
                    on_dismiss,
                } => {
                    let (scope, notif_source_win_id) =
                        self.resolve_plugin_notification_provenance(sender_pane_id, scope);
                    // Strip any per-option shortcut that conflicts with navigation keys.
                    let options: Vec<crate::protocol::NotifyOption> = options.into_iter().map(|mut opt| {
                        if let Some(ref sc) = opt.shortcut.clone() {
                            if crate::protocol::is_reserved_shortcut(sc) {
                                log::warn!(
                                    "notify:shortcut: view pane {} sent reserved shortcut {:?} on option {:?} — stripped",
                                    sender_pane_id, sc, opt.label
                                );
                                opt.shortcut = None;
                            }
                        }
                        opt
                    }).collect();
                    if matches!(kind, crate::protocol::NotifyKind::Choice) && options.is_empty() {
                        log::warn!("notify: invalid empty choice from pane {sender_pane_id}; resolving explicitly");
                        self.deliver_notify_action(
                            sender_pane_id,
                            notify_id,
                            "invalid_choice".to_string(),
                            Some(crate::app::notifications::NOTIFY_OUTCOME_INVALID.to_string()),
                            None,
                            None,
                        );
                    } else {
                        self.enqueue_notification(
                            crate::app::notifications::NotifySource::View,
                            PendingNotification {
                                notify_id,
                                sender_pane_id,
                                source_context_id,
                                source_window_id: notif_source_win_id,
                                title,
                                body,
                                kind,
                                options,
                                input_prompt,
                                required,
                                scope,
                                image_inline,
                                image_pipe_id,
                                timeout_secs,
                                on_dismiss,
                                ..Default::default()
                            },
                        );
                    }
                }
                ViewCommand::DeliverNotifyAction {
                    pane_id,
                    notify_id,
                    action_label,
                    value,
                    response_file,
                    host_action,
                } => {
                    self.deliver_notify_action(
                        pane_id,
                        notify_id,
                        action_label,
                        value,
                        response_file,
                        host_action,
                    );
                }
                ViewCommand::DeliverPipeMessage {
                    sender_pane_id,
                    generation,
                    payload,
                    emitter,
                } => {
                    // JSON pipe messages route only through directed pipes
                    // (#286); event streams replace the non-directed fan-out
                    // removed in 0327. The guest named the connection when it
                    // emitted the send, so the send reaches the sender's peer
                    // on that connection while it is live and nothing else:
                    // not a reopened pipe of the same name, not a view that
                    // later took the peer's slot (stint 1681).
                    let Some(sender) = emitter else {
                        log::warn!(
                            "DeliverPipeMessage: send on connection {generation} from pane {sender_pane_id} was never drained; dropping"
                        );
                        continue;
                    };
                    let Some(pipe) = self.directed_pipes.get(generation) else {
                        log::warn!(
                            "DeliverPipeMessage: connection {generation} from pane {sender_pane_id} is not live; dropping"
                        );
                        continue;
                    };
                    let Some(target) = pipe.peer_of(sender) else {
                        log::warn!(
                            "DeliverPipeMessage: {sender:?} is not an end of connection {generation}; dropping"
                        );
                        continue;
                    };
                    let event = crate::protocol::PlexiEvent::PipeMessage {
                        pipe_id: pipe.pipe_id.clone(),
                        generation,
                        payload,
                    };
                    match self.view_instance_mut(target) {
                        Some(view) => view.runtime.queue_outbound_event(event),
                        None => log::warn!(
                            "DeliverPipeMessage: connection {generation} peer {target:?} has ended; dropping"
                        ),
                    }
                }
                ViewCommand::CloseDirectedPipe {
                    sender_pane_id,
                    generation,
                    emitter,
                } => {
                    let Some(closer) = emitter else {
                        log::warn!(
                            "CloseDirectedPipe: close of connection {generation} from pane {sender_pane_id} was never drained; dropping"
                        );
                        continue;
                    };
                    self.close_directed_pipe(closer, generation);
                }
                ViewCommand::OpenDirectedPipe {
                    sender_pane_id,
                    request_id,
                    pipe_id,
                    target_pane_id,
                    emitter,
                    admission,
                } => {
                    let Some(opener) = emitter else {
                        log::warn!(
                            "OpenDirectedPipe: request '{request_id}' from pane {sender_pane_id} was never drained; dropped",
                            request_id = crate::host::guest_text::shown(&request_id)
                        );
                        continue;
                    };
                    let Some(admission) = admission else {
                        log::warn!(
                            "OpenDirectedPipe: request '{request_id}' from {opener:?} dropped: the opener is not a directed pipe endpoint, so no answer can reach it",
                            request_id = crate::host::guest_text::shown(&request_id)
                        );
                        continue;
                    };
                    self.handle_directed_pipe_open(
                        target_pane_id,
                        crate::app::directed_pipes::OpenRequest {
                            opener,
                            request_id,
                            pipe_id,
                            admission,
                        },
                    );
                }
                ViewCommand::RequestLinkedTerminal {
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
                ViewCommand::RunInLinkedTerminal {
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
                ViewCommand::InsertPathToken {
                    sender_pane_id,
                    terminal_pane_id,
                    path,
                    mode,
                } => {
                    self.dispatch_insert_path_token(sender_pane_id, terminal_pane_id, path, mode);
                }
                ViewCommand::RequestCommandPreview {
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
                ViewCommand::OpenArtifact {
                    sender_pane_id,
                    path,
                    mode,
                } => {
                    self.dispatch_open_artifact(sender_pane_id, path, mode);
                }
                ViewCommand::QueryContextState {
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

                    if !self
                        .router
                        .is_self_or_ancestor(requester_context_id, context_id)
                    {
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
                            if let Some(app) = pane.as_view_mut() {
                                app.runtime.queue_outbound_event(
                                    crate::protocol::PlexiEvent::ContextStateResponse { state },
                                );
                            }
                        }
                    }
                }
                ViewCommand::DeliverRunUpdate {
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
                            .and_then(|p| p.as_view())
                            .map(|a| a.manifest_id == originator_type_id)
                            .unwrap_or(false);
                        if matches {
                            if let Some(pane) = self.windows[active].panes.get_mut(&pid) {
                                if let Some(app) = pane.as_view_mut() {
                                    app.runtime.queue_outbound_event(event.clone());
                                    delivered = true;
                                    break;
                                }
                            }
                        }
                    }
                    if !delivered {
                        log::warn!(
                            "DeliverRunUpdate: no pane found for type_id='{originator_type_id}'",
                            originator_type_id =
                                crate::host::guest_text::shown(&originator_type_id)
                        );
                    }
                }
            }
        }
    }

    /// Drain `pending_commands` from **every** view pane in **every** context,
    /// stamp sender pane ids and source_context indices, and return the
    /// consolidated list.
    ///
    /// Must run every frame regardless of focus, modal state, or any
    /// overlay. Draining all contexts (not just the active one) ensures
    /// that background plugins — e.g. stand-up-reminder in a non-active context
    /// — can surface Global-scope notifications while the user is elsewhere.
    /// Parked background plugins (pane closed, process alive) are also ticked so
    /// their timers and notifications keep firing while detached.
    ///
    /// `window_hidden` is true when eframe is running a logic-only pass (the
    /// window is minimized or fully occluded), in which case `App::ui` never
    /// runs for *any* pane this frame — including the active context's own.
    /// The active context is then treated as non-active for tick purposes
    /// too, so its panes still get the headless tick (stint 0759).
    pub(crate) fn drain_all_plugin_commands(&mut self, window_hidden: bool) -> Vec<ViewCommand> {
        // Collect (context_id, pane_id, type_id, commands) per pane. We capture
        // context_id (stable u64) and type_id here because the manifest-declared
        // notification scope is looked up from the registry by type_id, and we
        // can't hold a mutable borrow on contexts + a shared borrow on the
        // registry at the same time during the match below.
        let active = self.active_window;
        let mut per_pane: Vec<(
            u64,
            u64,
            crate::host::pane::ViewIncarnation,
            String,
            Vec<ViewCommand>,
        )> = Vec::new();
        for (ctx_idx, context) in self.windows.iter_mut().enumerate() {
            let context_id = context.context_id;
            // Panes that are not in the tile tree never receive `ui()`, even
            // when this window is visible. A headless Assistant is one of
            // those: it lives in `panes` so a send can complete, and it has
            // to be ticked from logic.
            let tiled_ids: Vec<u64> = context
                .tree
                .tiles
                .iter()
                .filter_map(|(_, tile)| match tile {
                    egui_tiles::Tile::Pane(id) => Some(*id),
                    _ => None,
                })
                .collect();
            for (pane_id, pane) in context.panes.iter_mut() {
                if let Some(view_pane) = pane.as_view_mut() {
                    let off_tree = !tiled_ids.contains(pane_id);
                    // Active-context panes are already fully updated by
                    // ui() this frame — unless the window is hidden, in
                    // which case ui() never runs at all and the active
                    // context needs the same headless tick as everyone
                    // else (stint 0759). Non-active panes always need it:
                    // timer/async events reach the subprocess and control
                    // commands flow out — but only when they actually have
                    // pending background work; idle plugins are skipped so a
                    // busy foreground doesn't tick every background plugin on
                    // every frame (#2021). Off-tree panes are not painted,
                    // so they get the tick even in the active visible window.
                    if (ctx_idx != active || window_hidden || off_tree)
                        && view_pane.runtime.needs_background_tick()
                    {
                        view_pane.runtime.background_tick();
                    }
                    let type_id = view_pane.runtime.type_id().to_string();
                    let cmds = view_pane.runtime.take_pending_commands();
                    if !cmds.is_empty() {
                        per_pane.push((context_id, *pane_id, view_pane.incarnation, type_id, cmds));
                    }
                    continue;
                }
            }
        }

        // Tick parked background plugins (process alive, no visible pane).
        // Mirrors the headless tick above but operates on self.background_apps
        // instead of context panes. Without this, timers stop and notifications
        // never fire the moment the pane is closed.
        // Collect (type_id, park_context_idx, commands) for each parked plugin.
        let parked: Vec<(String, u64, Vec<ViewCommand>)> = self
            .background_apps
            .iter_mut()
            .map(|(type_id, (park_context_id, app))| {
                // Same gate as visible non-active panes: tick only when the
                // plugin has pending background work (#2021). Already-queued
                // pending_commands are still taken unconditionally.
                if app.needs_background_tick() {
                    log::debug!(
                        "parked background plugin '{type_id}': pending work — ticking",
                        type_id = crate::host::guest_text::shown(type_id)
                    );
                    app.background_tick();
                }
                let cmds = app.take_pending_commands();
                (type_id.to_owned(), *park_context_id, cmds)
            })
            .collect();

        let mut deferred = Vec::new();

        for (type_id, park_context_id, cmds) in parked {
            let resolved_scope = self
                .registries
                .view_for_context(park_context_id, &self.router)
                .default_notification_scope_for(&type_id);
            for cmd in cmds {
                // Commands that require a live pane context are silently
                // dropped when the plugin is parked — the plugin has no tile to
                // target and no context to route through.
                let ViewCommand::ShowNotification { title, .. } = &cmd else {
                    continue;
                };
                log::info!(
                    "parked background plugin '{}' notification: {} (routing to context_id {})",
                    type_id,
                    title,
                    park_context_id
                );
                // 0 = no live pane — tombstone won't fire.
                deferred.push(stamp_notification_origin(
                    cmd,
                    0,
                    park_context_id,
                    resolved_scope,
                ));
            }
        }

        for (context_id, pane_id, incarnation, type_id, cmds) in per_pane {
            let emitter = crate::app::view_trait::ViewEmitter {
                pane_id,
                incarnation,
            };
            // Scope is a per-plugin user-facing policy declared in the plugin's
            // manifest.toml. Plugins never set it — the host resolves it once
            // per notification here. A plugin that omits the field gets
            // `DefaultNotifyScope`'s own default (`Window`, the most
            // restrictive), which is a declared per-plugin policy and so does not
            // go through `NotifyScope::default()`.
            let resolved_scope = self
                .registries
                .view_for_context(context_id, &self.router)
                .default_notification_scope_for(&type_id);
            for cmd in cmds {
                match cmd {
                    ViewCommand::WasmHostEffect { effect, .. } => {
                        deferred.push(ViewCommand::WasmHostEffect {
                            pane_id,
                            effect,
                            emitter: Some(emitter),
                        });
                    }
                    ViewCommand::ExposeTools { tools, .. } => {
                        deferred.push(ViewCommand::ExposeTools {
                            tools,
                            pane_id: Some(pane_id),
                        });
                    }
                    ViewCommand::Fleet(_) | ViewCommand::ToolResult { .. } => deferred.push(cmd),
                    ViewCommand::PluginEventRequest { request, .. } => {
                        deferred.push(ViewCommand::PluginEventRequest {
                            request,
                            pane_id: Some(pane_id),
                        });
                    }
                    ViewCommand::AssistantHostTool {
                        cancel,
                        name,
                        input_json,
                        reply,
                        ..
                    } => {
                        deferred.push(ViewCommand::AssistantHostTool {
                            cancel,
                            name,
                            input_json,
                            origin_pane_id: pane_id,
                            origin_context_id: context_id,
                            reply,
                        });
                    }
                    // The emitting view is the one recipient of the reply,
                    // stamped here where its pane id is known (stint 1632).
                    ViewCommand::SpawnView {
                        type_id,
                        layout,
                        args,
                        ..
                    } => deferred.push(ViewCommand::SpawnView {
                        type_id,
                        layout,
                        args,
                        emitter: Some(emitter),
                    }),
                    ViewCommand::SpawnPane {
                        type_id,
                        layout,
                        args,
                        from_pane_id,
                        request_id,
                        target_context,
                        ..
                    } => deferred.push(ViewCommand::SpawnPane {
                        type_id,
                        layout,
                        args,
                        from_pane_id,
                        request_id,
                        target_context,
                        emitter: Some(emitter),
                    }),
                    ViewCommand::ForwardPaneRequest { .. }
                    | ViewCommand::DeliverRunUpdate { .. }
                    | ViewCommand::InsertPathToken { .. }
                    | ViewCommand::RequestCommandPreview { .. }
                    | ViewCommand::QueryContextState { .. } => deferred.push(cmd),
                    // The file browser (and every other builtin) emits
                    // `sender_pane_id: 0` because it does not know its pane id.
                    // Stamp the real id here so `open_text_file_in_split` anchors
                    // the editor on the requesting pane instead of logging
                    // "anchor pane 0 not found".
                    ViewCommand::OpenArtifact { path, mode, .. } => {
                        deferred.push(ViewCommand::OpenArtifact {
                            sender_pane_id: pane_id,
                            path,
                            mode,
                        });
                    }
                    // Every SDK bridge that constructs these emits a
                    // placeholder sender_pane_id=0 (it doesn't know its own
                    // pane id); rewrite to the real pane_id here, where it's
                    // known, so `origin_for_pane(sender_pane_id)` downstream
                    // resolves the ACTUAL caller instead of failing to find
                    // pane 0 (stint 0724 Phase D 2/2 — required for the
                    // directed-pipe reachability check to see a real caller
                    // at all, not just cosmetic).
                    //
                    // The emitting instance is stamped too: a pipe joins
                    // instances, not slots (stint 1681).
                    ViewCommand::DeliverPipeMessage {
                        generation,
                        payload,
                        ..
                    } => {
                        deferred.push(ViewCommand::DeliverPipeMessage {
                            sender_pane_id: pane_id,
                            generation,
                            payload,
                            emitter: Some(emitter),
                        });
                    }
                    ViewCommand::CloseDirectedPipe { generation, .. } => {
                        deferred.push(ViewCommand::CloseDirectedPipe {
                            sender_pane_id: pane_id,
                            generation,
                            emitter: Some(emitter),
                        });
                    }
                    ViewCommand::OpenDirectedPipe {
                        request_id,
                        pipe_id,
                        target_pane_id,
                        admission,
                        ..
                    } => {
                        deferred.push(ViewCommand::OpenDirectedPipe {
                            sender_pane_id: pane_id,
                            request_id,
                            pipe_id,
                            target_pane_id,
                            emitter: Some(emitter),
                            admission,
                        });
                    }
                    // Builtin plugins emit sender_pane_id=0; rewrite to
                    // the real pane_id so the host can route the response.
                    ViewCommand::RequestLinkedTerminal {
                        request_id,
                        cwd,
                        label,
                        place_below,
                        ..
                    } => {
                        deferred.push(ViewCommand::RequestLinkedTerminal {
                            sender_pane_id: pane_id,
                            request_id,
                            cwd,
                            label,
                            place_below,
                        });
                    }
                    ViewCommand::RunInLinkedTerminal {
                        terminal_pane_id,
                        command,
                        echo,
                        ..
                    } => {
                        deferred.push(ViewCommand::RunInLinkedTerminal {
                            sender_pane_id: pane_id,
                            terminal_pane_id,
                            command,
                            echo,
                        });
                    }
                    ViewCommand::CdRequest { cwd, .. } => {
                        deferred.push(ViewCommand::CdRequest {
                            cwd,
                            sender_pane_id: pane_id,
                        });
                    }
                    ViewCommand::ShowNotification { .. } => {
                        deferred.push(stamp_notification_origin(
                            cmd,
                            pane_id,
                            context_id,
                            resolved_scope,
                        ));
                    }
                    ViewCommand::DeliverNotifyAction { .. } => deferred.push(cmd),
                }
            }
        }

        deferred
    }

    /// Decide one admitted open request. Every outcome reaches the opener
    /// in the room its admission holds, unless the opener has ended.
    fn handle_directed_pipe_open(
        &mut self,
        target_pane_id: u64,
        request: crate::app::directed_pipes::OpenRequest,
    ) {
        use crate::protocol::PipeOpenRefusal;
        let opener = request.opener;
        if request.pipe_id.len() > crate::protocol::MAX_PIPE_ID_BYTES {
            return self.refuse_directed_pipe_open(request, PipeOpenRefusal::NameTooLong);
        }
        // Resolve host-established origins for BOTH ends — never
        // `router.active()`/`active_window` — so reachability is decided from
        // facts the host itself observed, not a claim or ambient focus state
        // (stint 0724 Phase D 2/2).
        let Some(sender_origin) = self.origin_for_pane(opener.pane_id) else {
            return self.refuse_directed_pipe_open(request, PipeOpenRefusal::OpenerUnplaced);
        };
        let Some(target_origin) = self.origin_for_pane(target_pane_id) else {
            return self.refuse_directed_pipe_open(request, PipeOpenRefusal::NoTarget);
        };
        // A directed pipe is an exclusive channel to ONE named pane — the
        // tightest scope available — so a caller in another context is
        // rejected outright. No `CrossContextGrant` consumer exists for
        // directed pipes yet; `evaluate_reach` already logs the rejection.
        let owner = crate::host::scope::Scope::Pane {
            pane_id: target_pane_id,
            context_id: target_origin.context_id,
        };
        if crate::host::scope::evaluate_reach(&owner, sender_origin.context_id, None)
            == crate::host::scope::Reach::Rejected
        {
            return self.refuse_directed_pipe_open(request, PipeOpenRefusal::OutOfReach);
        }
        // The pipe joins the view instance now in the target slot, never the
        // slot itself (stint 1681).
        let target = match self
            .windows
            .iter()
            .find_map(|w| w.panes.get(&target_pane_id))
        {
            Some(crate::host::pane::Pane::View(view)) => crate::app::view_trait::ViewEmitter {
                pane_id: target_pane_id,
                incarnation: view.incarnation,
            },
            Some(_) => {
                return self.refuse_directed_pipe_open(request, PipeOpenRefusal::NotAPlugin);
            }
            None => return self.refuse_directed_pipe_open(request, PipeOpenRefusal::NoTarget),
        };
        // An opener that has ended, or is no longer an endpoint (its guest
        // finished or failed), cannot be told of a connection; nothing is
        // prepared for it.
        if !self
            .view_instance_mut(opener)
            .is_some_and(|view| view.runtime.is_directed_pipe_endpoint())
        {
            log::warn!(
                "OpenDirectedPipe: opener {opener:?} ended before its request '{request_id}' was handled; dropped",
                request_id = crate::host::guest_text::shown(&request.request_id)
            );
            return;
        }
        // Both exact instances must be endpoints now, for a fresh open and an
        // idempotent one alike: an instance that has finished, even with its
        // pane still mounted, or a builtin that takes no pipe events, is told
        // of no connection.
        let target_is_endpoint = self
            .view_instance_mut(target)
            .is_some_and(|view| view.runtime.is_directed_pipe_endpoint());
        let context_id = sender_origin.context_id;
        match self
            .directed_pipes
            .check_open(context_id, &request.pipe_id, opener, target)
        {
            crate::app::directed_pipes::OpenCheck::InUse => {
                self.refuse_directed_pipe_open(request, PipeOpenRefusal::InUse);
            }
            crate::app::directed_pipes::OpenCheck::AlreadyOpen(_) if !target_is_endpoint => {
                // The live connection's target has finished: retire it now,
                // and refuse.
                self.retire_directed_pipes_of_exited_ends();
                self.refuse_directed_pipe_open(request, PipeOpenRefusal::TargetUnavailable);
            }
            crate::app::directed_pipes::OpenCheck::AlreadyOpen(generation) => {
                log::info!(
                    "OpenDirectedPipe: '{pipe_id}' is already connection {generation} between pane {} and pane {target_pane_id}",
                    opener.pane_id,
                    pipe_id = crate::host::guest_text::shown(&request.pipe_id)
                );
                // An idempotent open needs only its answer; the connection
                // already holds its close room.
                let crate::app::directed_pipes::OpenRequest {
                    request_id,
                    pipe_id,
                    admission,
                    ..
                } = request;
                if let Err(error) = self.answer_directed_pipe_open(
                    opener,
                    admission.answer,
                    crate::protocol::PlexiEvent::DirectedPipeOpened {
                        request_id,
                        pipe_id,
                        generation,
                    },
                ) {
                    log::warn!(
                        "OpenDirectedPipe: {opener:?} could not be told it holds connection {generation}: {error}"
                    );
                }
            }
            crate::app::directed_pipes::OpenCheck::Free if !target_is_endpoint => {
                log::info!("OpenDirectedPipe: target {target:?} is not a directed pipe endpoint");
                self.refuse_directed_pipe_open(request, PipeOpenRefusal::TargetUnavailable);
            }
            crate::app::directed_pipes::OpenCheck::Free => {
                self.open_directed_pipe(context_id, target, request);
            }
        }
    }

    /// Open a directed pipe whose name `check_open` found free. Everything is
    /// prepared before anything is published: an unpublished generation,
    /// close room at each end, both ends' registrations, and the target's
    /// `DirectedPipeIncoming`. Any failure undoes what was made and refuses
    /// the open; success publishes the connection and answers the opener in
    /// the same dispatch turn. An opener that ends between the two is
    /// retired at once, and the target is told it closed.
    fn open_directed_pipe(
        &mut self,
        context_id: u64,
        target: crate::app::view_trait::ViewEmitter,
        request: crate::app::directed_pipes::OpenRequest,
    ) {
        use crate::protocol::PipeOpenRefusal;
        let opener = request.opener;
        let pipe_id = request.pipe_id.clone();
        // An open admitted while the opener's close pool was held by its live
        // connections takes its close room now, or is refused in its answer
        // room.
        let mut request = request;
        if request.admission.close.is_none() {
            let reserved = self
                .view_instance_mut(opener)
                .ok_or(crate::host::pane::NotAdmitted::NotAnEndpoint)
                .and_then(|view| {
                    view.runtime.reserve_directed_control(
                        crate::host::pane::ControlRoom::OpenerClose { pipe_id: &pipe_id },
                    )
                });
            match reserved {
                Ok(slot) => request.admission.close = Some(slot),
                Err(why) => {
                    log::info!(
                        "OpenDirectedPipe: '{pipe_id}' refused, {opener:?} has no room for its close: {why:?}",
                        pipe_id = crate::host::guest_text::shown(&pipe_id)
                    );
                    return self.refuse_directed_pipe_open(request, PipeOpenRefusal::OpenerFull);
                }
            }
        }
        let generation = self.directed_pipes.allocate();
        let target_close = if target == opener {
            None
        } else {
            let reserved = self
                .view_instance_mut(target)
                .ok_or(crate::host::pane::NotAdmitted::NotAnEndpoint)
                .and_then(|view| {
                    view.runtime.reserve_directed_control(
                        crate::host::pane::ControlRoom::TargetClose { pipe_id: &pipe_id },
                    )
                });
            match reserved {
                Ok(slot) => Some(slot),
                Err(why) => {
                    log::warn!(
                        "OpenDirectedPipe: '{pipe_id}' refused, {target:?} has no room for its close: {why:?}",
                        pipe_id = crate::host::guest_text::shown(&pipe_id)
                    );
                    return self
                        .refuse_directed_pipe_open(request, PipeOpenRefusal::TargetUnavailable);
                }
            }
        };
        let ends: Vec<_> = if opener == target {
            vec![opener]
        } else {
            vec![opener, target]
        };
        let mut registered = Vec::new();
        for &end in &ends {
            let result = self.view_instance_mut(end).map_or(Ok(false), |view| {
                register_directed_pipe_on(view, &pipe_id, generation)
            });
            match result {
                Ok(true) => registered.push(end),
                Ok(false) => {}
                Err(reason) => {
                    log::warn!(
                        "OpenDirectedPipe: '{pipe_id}' refused, {end:?} cannot register it: {reason}",
                        pipe_id = crate::host::guest_text::shown(&pipe_id)
                    );
                    self.undo_directed_pipe_open(generation, &pipe_id, &registered);
                    return self.refuse_directed_pipe_open(request, PipeOpenRefusal::NameConflict);
                }
            }
        }
        if target != opener {
            let incoming = crate::protocol::PlexiEvent::DirectedPipeIncoming {
                pipe_id: pipe_id.clone(),
                generation,
                peer_pane_id: opener.pane_id,
            };
            let told = self
                .view_instance_mut(target)
                .ok_or_else(|| "the target has ended".to_string())
                .and_then(|view| view.runtime.deliver_directed_incoming(incoming));
            if let Err(reason) = told {
                log::warn!(
                    "OpenDirectedPipe: '{pipe_id}' refused, {target:?} could not be told of connection {generation}: {reason}",
                    pipe_id = crate::host::guest_text::shown(&pipe_id)
                );
                self.undo_directed_pipe_open(generation, &pipe_id, &registered);
                return self.refuse_directed_pipe_open(request, PipeOpenRefusal::TargetUnavailable);
            }
        }
        let crate::app::directed_pipes::OpenRequest {
            request_id,
            admission,
            ..
        } = request;
        let mut closes: Vec<_> = admission
            .close
            .map(|slot| (opener, slot))
            .into_iter()
            .collect();
        closes.extend(target_close.map(|slot| (target, slot)));
        self.directed_pipes
            .publish(generation, context_id, &pipe_id, (opener, target), closes);
        log::info!(
            "OpenDirectedPipe: '{pipe_id}' is connection {generation}: pane {} ↔ pane {} (context {context_id})",
            opener.pane_id,
            target.pane_id,
            pipe_id = crate::host::guest_text::shown(&pipe_id)
        );
        let answered = self.answer_directed_pipe_open(
            opener,
            admission.answer,
            crate::protocol::PlexiEvent::DirectedPipeOpened {
                request_id,
                pipe_id: pipe_id.clone(),
                generation,
            },
        );
        if let Err(error) = answered {
            log::warn!(
                "OpenDirectedPipe: '{pipe_id}' connection {generation} retired: its opener {opener:?} ended after the target was told ({error})",
                pipe_id = crate::host::guest_text::shown(&pipe_id)
            );
            let plugin_id = self
                .view_instance_mut(opener)
                .map(|view| {
                    view.runtime.close_directed_pipe(&pipe_id, generation);
                    view.manifest_id.clone()
                })
                .unwrap_or_default();
            if let Some(pipe) = self.directed_pipes.revoke(generation) {
                let tell: Vec<_> = pipe
                    .peer_of(opener)
                    .filter(|peer| *peer != opener)
                    .into_iter()
                    .collect();
                self.end_directed_pipe(
                    generation,
                    pipe,
                    opener,
                    &plugin_id,
                    &tell,
                    crate::protocol::PipeCloseReason::PeerClosed,
                );
            }
        }
    }

    /// Undo an open that failed part way: drop `generation` from every end
    /// that registered it. It was never published.
    fn undo_directed_pipe_open(
        &mut self,
        generation: u64,
        pipe_id: &str,
        registered: &[crate::app::view_trait::ViewEmitter],
    ) {
        for &end in registered {
            if let Some(view) = self.view_instance_mut(end) {
                view.runtime.close_directed_pipe(pipe_id, generation);
            }
        }
    }

    /// Deliver an open's answer to `opener` in the room its admission holds.
    fn answer_directed_pipe_open(
        &mut self,
        opener: crate::app::view_trait::ViewEmitter,
        answer: crate::host::pane::ControlSlot,
        event: crate::protocol::PlexiEvent,
    ) -> Result<(), String> {
        self.view_instance_mut(opener)
            .ok_or_else(|| "the opener has ended".to_string())?
            .runtime
            .deliver_directed_control(answer, event)
    }

    /// Answer `request` with `DirectedPipeOpenRefused`.
    fn refuse_directed_pipe_open(
        &mut self,
        request: crate::app::directed_pipes::OpenRequest,
        reason: crate::protocol::PipeOpenRefusal,
    ) {
        let crate::app::directed_pipes::OpenRequest {
            opener,
            request_id,
            pipe_id,
            admission,
        } = request;
        log::info!(
            "OpenDirectedPipe: request '{request_id}' for '{pipe_id}' from {opener:?} refused ({})",
            reason.as_str(),
            request_id = crate::host::guest_text::shown(&request_id),
            pipe_id = crate::host::guest_text::shown(&pipe_id)
        );
        let refused = crate::protocol::PlexiEvent::DirectedPipeOpenRefused {
            request_id,
            pipe_id,
            reason,
        };
        if let Err(error) = self.answer_directed_pipe_open(opener, admission.answer, refused) {
            log::warn!("OpenDirectedPipe: refusal to {opener:?} undelivered: {error}");
        }
    }

    /// Retire every connection an end of which is no longer a directed pipe
    /// endpoint: its guest finished or failed while its pane stays mounted.
    /// Each surviving end is told once, in the room it holds. Runs every
    /// logic pass, and when an open finds a connection's target finished.
    pub(crate) fn retire_directed_pipes_of_exited_ends(&mut self) {
        let mut exited: Vec<(crate::app::view_trait::ViewEmitter, String)> = Vec::new();
        for end in self.directed_pipes.ends() {
            if exited.iter().any(|(seen, _)| *seen == end) {
                continue;
            }
            if let Some(view) = self.view_instance_mut(end) {
                if !view.runtime.is_directed_pipe_endpoint() {
                    log::info!(
                        "directed pipes: {end:?} (plugin={}) is no longer an endpoint; retiring its connections",
                        view.manifest_id
                    );
                    exited.push((end, view.manifest_id.clone()));
                }
            }
        }
        if !exited.is_empty() {
            self.retire_directed_pipes_for(&exited, crate::protocol::PipeCloseReason::PeerClosed);
        }
    }

    /// Retire every directed pipe connection an `ended` view instance is an
    /// end of, because it closed, was uncovered as an overlay, was deleted
    /// with its page or context, or its guest was replaced. Each `ended`
    /// entry is the instance and its plugin id. Every other end is told with
    /// `PipeClosed`; an end that is itself ending (a self-pipe, or both ends
    /// deleted together) has no one to tell. `ended`'s own registry goes
    /// with it (or, on relaunch, is cleared by the runtime).
    pub(crate) fn retire_directed_pipes_for(
        &mut self,
        ended: &[(crate::app::view_trait::ViewEmitter, String)],
        reason: crate::protocol::PipeCloseReason,
    ) {
        let retired = self
            .directed_pipes
            .retire_where(|pipe| ended.iter().any(|(end, _)| pipe.has_end(*end)));
        for (generation, pipe) in retired {
            let (from, from_plugin) = ended
                .iter()
                .find(|(end, _)| pipe.has_end(*end))
                .expect("retire_where returns only connections with an ending end");
            let survivors: Vec<_> = [pipe.ends.0, pipe.ends.1]
                .into_iter()
                .filter(|end| !ended.iter().any(|(ending, _)| ending == end))
                .collect();
            let (from, from_plugin) = (*from, from_plugin.clone());
            self.end_directed_pipe(generation, pipe, from, &from_plugin, &survivors, reason);
        }
    }

    /// Retire the directed pipes of every view instance in the `pane_ids`
    /// slots, the visible ones and any they cover, because the slots are
    /// being deleted together.
    pub(crate) fn retire_directed_pipes_in_slots(
        &mut self,
        pane_ids: &[u64],
        reason: crate::protocol::PipeCloseReason,
    ) {
        let ended: Vec<_> = self
            .view_instances()
            .into_iter()
            .filter(|(end, _, _)| pane_ids.contains(&end.pane_id))
            .map(|(end, _, plugin_id)| (end, plugin_id))
            .collect();
        self.retire_directed_pipes_for(&ended, reason);
    }

    /// Retire every directed pipe connection an end of which no longer sits
    /// in the context that admitted it, because a wrap or dissolve moved it.
    /// Both ends live on and both are told (`PipeClosed { moved }`); a reopen
    /// in the new context is a new connection. Called by every operation
    /// that moves panes between contexts, before it returns, so no command
    /// dispatches while a connection spans two contexts.
    pub(crate) fn retire_directed_pipes_moved(&mut self) {
        let instances = self.view_instances();
        let moved = |end: crate::app::view_trait::ViewEmitter, context_id: u64| {
            instances
                .iter()
                .find(|(instance, _, _)| *instance == end)
                .filter(|(_, now, _)| *now != context_id)
                .map(|(_, _, plugin_id)| plugin_id.clone())
        };
        let retired = self.directed_pipes.retire_where(|pipe| {
            moved(pipe.ends.0, pipe.context_id).is_some()
                || moved(pipe.ends.1, pipe.context_id).is_some()
        });
        for (generation, pipe) in retired {
            let (from, from_plugin) = [pipe.ends.0, pipe.ends.1]
                .into_iter()
                .find_map(|end| Some((end, moved(end, pipe.context_id)?)))
                .expect("retire_where returns only connections with a moved end");
            let ends: Vec<_> = if pipe.ends.0 == pipe.ends.1 {
                vec![pipe.ends.0]
            } else {
                vec![pipe.ends.0, pipe.ends.1]
            };
            self.end_directed_pipe(
                generation,
                pipe,
                from,
                &from_plugin,
                &ends,
                crate::protocol::PipeCloseReason::Moved,
            );
        }
    }

    /// End connection `generation` because its end `closer` closed it. A
    /// view that is not an end, or a connection already retired, changes
    /// nothing. The other end is told; the closer already dropped its own
    /// handle.
    fn close_directed_pipe(
        &mut self,
        closer: crate::app::view_trait::ViewEmitter,
        generation: u64,
    ) {
        let Some(plugin_id) = self
            .view_instance_mut(closer)
            .map(|view| view.manifest_id.clone())
        else {
            log::warn!("CloseDirectedPipe: {closer:?} has ended; connection {generation} left to its retirement");
            return;
        };
        let is_end = self
            .directed_pipes
            .get(generation)
            .is_some_and(|pipe| pipe.has_end(closer));
        let Some(pipe) = is_end
            .then(|| self.directed_pipes.revoke(generation))
            .flatten()
        else {
            log::warn!(
                "CloseDirectedPipe: {closer:?} is not an end of live connection {generation}; nothing to close"
            );
            return;
        };
        let tell: Vec<_> = pipe
            .peer_of(closer)
            .filter(|peer| *peer != closer)
            .into_iter()
            .collect();
        self.end_directed_pipe(
            generation,
            pipe,
            closer,
            &plugin_id,
            &tell,
            crate::protocol::PipeCloseReason::PeerClosed,
        );
    }

    /// Every view instance in a pane slot, visible or covered by an overlay,
    /// with its context and plugin id.
    fn view_instances(&self) -> Vec<(crate::app::view_trait::ViewEmitter, u64, String)> {
        let mut instances = Vec::new();
        for window in &self.windows {
            for (&pane_id, pane) in window.panes.iter() {
                let mut next = Some(pane);
                while let Some(crate::host::pane::Pane::View(view)) = next {
                    instances.push((
                        crate::app::view_trait::ViewEmitter {
                            pane_id,
                            incarnation: view.incarnation,
                        },
                        window.context_id,
                        view.manifest_id.clone(),
                    ));
                    next = view.overlay_replaced.as_deref();
                }
            }
        }
        instances
    }

    /// Tell each of `tell` that connection `generation` ended, then drop it
    /// from that end's registry, and record the retirement. `from` is the
    /// end whose change ended it, `from_plugin` its plugin id.
    fn end_directed_pipe(
        &mut self,
        generation: u64,
        pipe: crate::app::directed_pipes::DirectedPipe,
        from: crate::app::view_trait::ViewEmitter,
        from_plugin: &str,
        tell: &[crate::app::view_trait::ViewEmitter],
        reason: crate::protocol::PipeCloseReason,
    ) {
        let mut pipe = pipe;
        for &end in tell {
            let room = pipe.take_close_room(end);
            if let Some(view) = self.view_instance_mut(end) {
                // Told first, in the room this end holds for it: a WASM
                // guest hears of it on the handle the registry is about to
                // drop.
                let closed = crate::protocol::PlexiEvent::PipeClosed {
                    pipe_id: pipe.pipe_id.clone(),
                    generation,
                    reason,
                };
                let told = match room {
                    Some(room) => view.runtime.deliver_directed_control(room, closed),
                    None => Err("it holds no room for the close".to_string()),
                };
                if let Err(error) = told {
                    log::warn!(
                        "directed pipe connection {generation}: {end:?} not told it closed: {error}"
                    );
                }
                view.runtime.close_directed_pipe(&pipe.pipe_id, generation);
            }
        }
        log::info!(
            "directed pipe '{pipe_id}' connection {generation} retired ({}) plugin={from_plugin} pane={} told={:?} context={}",
            reason.as_str(),
            from.pane_id,
            tell.iter().map(|end| end.pane_id).collect::<Vec<_>>(),
            pipe.context_id,
            pipe_id = crate::host::guest_text::shown(&pipe.pipe_id)
        );
        crate::host::event_log::emit_scoped(
            crate::host::event_log::HostEvent::PipeClosed {
                from_plugin: from_plugin.to_string(),
                channel: pipe.pipe_id,
                timestamp: crate::host::event_log::now_timestamp(),
            },
            self.context_root_for(pipe.context_id).as_deref(),
        );
    }
}

/// Stamp a drained `ShowNotification` with the origin the host resolved for
/// it. The two drain paths in `drain_all_view_commands` differ only in which
/// sender pane and context they stamp — a parked background plugin has no live
/// pane and routes to its park context, a live pane stamps itself — so the
/// field-by-field rebuild lives here once. Any other command passes through
/// untouched.
fn stamp_notification_origin(
    cmd: ViewCommand,
    sender_pane_id: u64,
    source_context_id: u64,
    scope: crate::protocol::NotifyScope,
) -> ViewCommand {
    match cmd {
        ViewCommand::ShowNotification {
            notify_id,
            title,
            body,
            kind,
            options,
            input_prompt,
            required,
            image_inline,
            image_pipe_id,
            timeout_secs,
            on_dismiss,
            ..
        } => ViewCommand::ShowNotification {
            notify_id,
            sender_pane_id,
            source_context_id,
            title,
            body,
            kind,
            options,
            input_prompt,
            required,
            scope,
            image_inline,
            image_pipe_id,
            timeout_secs,
            on_dismiss,
        },
        other => other,
    }
}
