use super::*;

impl PlexiApp {
    pub(crate) fn draw_welcome_delete_overlay(&self, ctx: &egui::Context) {
        self.draw_triple_tap_overlay(
            ctx,
            "welcome_delete_overlay",
            self.welcome_delete_press_count,
            "⌫ pressed",
        );
    }

    pub(crate) fn draw_quit_confirm_overlay(&self, ctx: &egui::Context) {
        let count = self.quit_press_count;
        // Bottom-hung progress toast, not a blocking dialog: no scrim, and
        // width follows content rather than the modal default.
        crate::ui::overlay::ModalShell::centered("quit_confirm_overlay")
            .anchor(Align2::CENTER_BOTTOM, Vec2::new(0.0, -40.0))
            .scrim(false)
            .width(0.0)
            .show(ctx, &self.colors, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "\u{2318}Q pressed {} of 3 — press again to quit",
                            count
                        ))
                        .size(style::TEXT_CAPTION)
                        .color(self.colors.text_dim),
                    );
                    ui.add_space(style::SPACE_SM);
                    for i in 1u8..=3 {
                        let color = if i <= count {
                            self.colors.accent
                        } else {
                            self.colors.bg_active
                        };
                        let (rect, _) =
                            ui.allocate_exact_size(Vec2::new(8.0, 8.0), egui::Sense::hover());
                        ui.painter().circle_filled(rect.center(), 4.0, color);
                    }
                });
            });
    }

    pub(crate) fn draw_confirm_close(&mut self, ctx: &egui::Context) {
        let mut confirmed = false;
        let mut cancelled = false;

        // Consume Enter/Escape at the context level so they cannot bleed
        // through to the focused pane this frame. `ui.input(|i| key_pressed)`
        // is a *read-only* check — it does not remove the event — which is
        // how this modal used to leak a confirming Enter into the pane behind
        // it (e.g. the backlog app opened the selected note on Cmd+W → Enter).
        // Combined with the `FocusLayer::ConfirmClose` capture, this is the
        // systemic fix: overlay owns focus, overlay drains its own keys.
        ctx.input_mut(|i| {
            if i.consume_key(egui::Modifiers::NONE, egui::Key::Enter) {
                confirmed = true;
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                cancelled = true;
            }
        });

        // Centered kit modal. Click-away sets `dismissed` (the old scrim
        // cancel); button results come out via the captured locals.
        let mut btn_confirmed = false;
        let mut btn_cancelled = false;
        let response = crate::ui::overlay::ModalShell::centered("confirm_close_overlay")
            .title("Close pane?")
            .width(super::MODAL_WIDTH)
            .show(ctx, &self.colors, |ui| {
                let c = &mut btn_confirmed;
                let k = &mut btn_cancelled;
                ui.label(
                    RichText::new("The running process will be terminated.")
                        .size(style::TEXT_CAPTION)
                        .color(self.colors.text_dim),
                );
                ui.add_space(style::SPACE_MD);
                ui.horizontal(|ui| {
                    if crate::ui::button::chrome_button(
                        ui,
                        "Close",
                        crate::ui::button::ButtonKind::Danger,
                        &self.colors,
                        0.0,
                    )
                    .clicked()
                    {
                        *c = true;
                    }
                    ui.add_space(8.0);
                    if crate::ui::button::chrome_button(
                        ui,
                        "Cancel",
                        crate::ui::button::ButtonKind::Secondary,
                        &self.colors,
                        0.0,
                    )
                    .clicked()
                    {
                        *k = true;
                    }
                });
                let hints = [
                    crate::ui::hints::HintGroup::new(&["Enter"], "confirm"),
                    crate::ui::hints::HintGroup::new(&["Esc"], "cancel"),
                ];
                crate::ui::hints::HintBar::new(&hints).show(ui, &self.colors);
            });
        confirmed |= btn_confirmed;
        cancelled |= response.dismissed | btn_cancelled;

        if confirmed {
            self.pending_close = false;
            if self.execute_close_pane() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            } else {
                self.save_workspace();
            }
        } else if cancelled {
            self.pending_close = false;
        }
    }

    /// Capability / secret consent modal for the focused ProcessApp pane.
    /// Called from step 2 of `update()` so it holds exclusive keyboard
    /// ownership before `dispatch_app_key_events` runs.
    pub(crate) fn draw_capability_modal(&mut self, ctx: &egui::Context) {
        let active = self.active_window;
        let colors = self.colors;

        // Resolve focused pane id — bail if it's not a ProcessApp.
        // Use find_pane_in_tile so we traverse through any Container wrapper that
        // egui_tiles may have inserted around a bare-pane root after first render.
        let pane_id = {
            let win = &self.windows[active];
            let focused_tile = match win.focused_pane {
                Some(t) => t,
                None => return,
            };
            match crate::app::PlexiApp::find_pane_in_tile(&win.tree, focused_tile) {
                Some(id) => id,
                None => return,
            }
        };

        // Take fields out of the ProcessApp, call the modal, put them back.
        // Two separate borrows so Rust doesn't see a conflict.
        let (
            mut pending_prompts,
            mut outbound_events,
            mut permissions,
            mut secret_input_buf,
            mut permission_store,
            mut grant_store,
            mut deferred_ai_queries,
            mut deferred_gated_requests,
            mut pending_commands,
            type_id,
            workspace_root,
            ai_broker,
            http_tx,
            proc_pane_id,
            mut pending_async_completions,
        ) = {
            let pane = match self.windows[active].panes.get_mut(&pane_id) {
                Some(crate::host::pane::Pane::App(a)) => a,
                _ => return,
            };
            let crate::host::pane::AppRuntime::Process(ref mut proc) = pane.runtime else {
                return;
            };
            if proc.pending_prompts.is_empty() {
                return;
            }
            (
                std::mem::take(&mut proc.pending_prompts),
                std::mem::take(&mut proc.outbound_events),
                std::mem::take(&mut proc.permissions),
                std::mem::take(&mut proc.secret_input_buf),
                std::mem::take(&mut proc.permission_store),
                std::mem::take(&mut proc.grant_store),
                std::mem::take(&mut proc.deferred_ai_queries),
                std::mem::take(&mut proc.deferred_gated_requests),
                std::mem::take(&mut proc.pending_commands),
                proc.type_id.clone(),
                proc.workspace_root.clone(),
                std::sync::Arc::clone(&proc.ai_broker),
                proc.waking_http_tx("ai_query_deferred"),
                proc.pane_id,
                proc.pending_async_completions,
            )
        };

        let config_dir = crate::config::config_dir();
        crate::process_app::prompts::show_prompt_modal(
            ctx,
            &mut pending_prompts,
            &mut outbound_events,
            &mut permissions,
            &type_id,
            &workspace_root,
            &mut secret_input_buf,
            &config_dir,
            &mut permission_store,
            &mut grant_store,
            &colors,
            &mut deferred_ai_queries,
            &mut deferred_gated_requests,
            &mut pending_commands,
            ai_broker,
            http_tx,
            proc_pane_id,
            &mut pending_async_completions,
        );

        // Put the data back.
        let pane = match self.windows[active].panes.get_mut(&pane_id) {
            Some(crate::host::pane::Pane::App(a)) => a,
            _ => return,
        };
        let crate::host::pane::AppRuntime::Process(ref mut proc) = pane.runtime else {
            return;
        };
        proc.pending_prompts = pending_prompts;
        proc.outbound_events = outbound_events;
        proc.permissions = permissions;
        proc.secret_input_buf = secret_input_buf;
        proc.permission_store = permission_store;
        proc.grant_store = grant_store;
        proc.deferred_ai_queries = deferred_ai_queries;
        proc.deferred_gated_requests = deferred_gated_requests;
        // Restore the async-wake count, including arms added for deferred
        // ai.query dispatches. Nothing else mutates the counter while the
        // modal renders (single-threaded UI pass), so copy-back is safe.
        proc.pending_async_completions = pending_async_completions;
        // The modal may have appended ForwardPaneRequest commands for
        // deferred gated requests; anything routed onto proc.pending_commands
        // between take and restore would be lost, so append rather than
        // overwrite.
        pending_commands.append(&mut proc.pending_commands);
        proc.pending_commands = pending_commands;
    }

    /// Context-close confirmation dialog. Shows pane inventory with three choices:
    /// Close All (Enter), Dissolve (D), Cancel (Escape).
    pub(crate) fn draw_context_close_confirm(&mut self, ctx: &egui::Context) {
        let state = match self.pending_context_close.take() {
            Some(s) => s,
            None => return,
        };

        let mut close_all = false;
        let mut dissolve = false;
        let mut cancelled = false;

        ctx.input_mut(|i| {
            if i.consume_key(egui::Modifiers::NONE, egui::Key::Enter) {
                close_all = true;
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::D) {
                dissolve = true;
            }
            if i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                cancelled = true;
            }
        });

        let colors = self.colors;
        let title = if state.context_name.is_empty() {
            "Close context?".to_string()
        } else {
            format!("Close \"{}\"?", state.context_name)
        };
        let mut btn_close_all = false;
        let mut btn_dissolve = false;
        let mut btn_cancelled = false;
        let response = crate::ui::overlay::ModalShell::centered("ctx_close_confirm_modal")
            .title(&title)
            .width(super::MODAL_WIDTH)
            .show(ctx, &colors, |ui| {
                let ca = &mut btn_close_all;
                let dv = &mut btn_dissolve;
                let ck = &mut btn_cancelled;

                for item in &state.items {
                    let label = format!("{} — {}", item.kind, item.name);
                    ui.label(
                        RichText::new(&label)
                            .size(style::TEXT_HINT)
                            .color(colors.text_dim),
                    );
                }

                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if crate::ui::button::chrome_button(
                        ui,
                        "Close all",
                        crate::ui::button::ButtonKind::Danger,
                        &colors,
                        0.0,
                    )
                    .clicked()
                    {
                        *ca = true;
                    }
                    ui.add_space(6.0);
                    if crate::ui::button::chrome_button(
                        ui,
                        "Dissolve",
                        crate::ui::button::ButtonKind::Secondary,
                        &colors,
                        0.0,
                    )
                    .clicked()
                    {
                        *dv = true;
                    }
                    ui.add_space(6.0);
                    if crate::ui::button::chrome_button(
                        ui,
                        "Cancel",
                        crate::ui::button::ButtonKind::Secondary,
                        &colors,
                        0.0,
                    )
                    .clicked()
                    {
                        *ck = true;
                    }
                });

                let hints = [
                    crate::ui::hints::HintGroup::new(&["Enter"], "close all"),
                    crate::ui::hints::HintGroup::new(&["D"], "dissolve"),
                    crate::ui::hints::HintGroup::new(&["Esc"], "cancel"),
                ];
                crate::ui::hints::HintBar::new(&hints).show(ui, &colors);
            });
        close_all |= btn_close_all;
        dissolve |= btn_dissolve;
        cancelled |= response.dismissed | btn_cancelled;

        if close_all {
            let idx = self
                .router
                .iter()
                .position(|c| c.context_id == state.context_id);
            if let Some(i) = idx {
                log::info!(
                    "context_close: close_all ctx={} name={:?}",
                    state.context_id,
                    state.context_name
                );
                self.delete_context(i);
                self.save_workspace();
            } else {
                log::warn!(
                    "context_close: close_all ctx={} not found in router",
                    state.context_id
                );
            }
        } else if dissolve {
            log::info!(
                "context_close: dissolve ctx={} name={:?}",
                state.context_id,
                state.context_name
            );
            self.dissolve_portal(state.context_id);
            self.save_workspace();
        } else if cancelled {
            log::info!("context_close: cancelled ctx={}", state.context_id);
        } else {
            // No input yet — put state back for the next frame.
            self.pending_context_close = Some(state);
        }
    }

    /// Primary notification surface: a keyboard-first centered modal over the
    /// work area. Renders the front of the queue.
    ///
    /// Enter / option-select / input-submit dispatches exactly one
    /// `DeliverNotifyAction` and removes the notification from the queue.
    /// Esc defers: the modal closes but the notification stays in the
    /// queue (reopen with Cmd+Shift+A to come back to it). No
    /// NotifyAction is dispatched on Esc — the app hasn't been answered.
    /// Required notifications reject Esc.
    ///
    /// Input map (all kinds):
    ///   Esc        — defer (only when `required == false`); keeps in queue
    ///
    /// `kind = message`:
    ///   Enter | Space — Acknowledge
    ///
    /// `kind = choice`:
    ///   ↑/↓ or j/k   — move focus
    ///   Enter | Space — confirm focused option
    ///   1-9          — direct-select the Nth option
    ///   per-option `shortcut` — direct-select that option
    ///
    /// `kind = input`:
    ///   text typing  — edits buffer
    ///   Enter        — submit (only if non-empty OR `required == false`)

    pub(crate) fn confirm_close_handle_key(
        &mut self,
        _ctx: &egui::Context,
    ) -> crate::app::app_trait::KeyDisposition {
        crate::app::app_trait::KeyDisposition::Consumed
    }

    pub(crate) fn context_close_confirm_handle_key(
        &mut self,
        _ctx: &egui::Context,
    ) -> crate::app::app_trait::KeyDisposition {
        crate::app::app_trait::KeyDisposition::Consumed
    }

    pub(crate) fn capability_modal_handle_key(
        &mut self,
        _ctx: &egui::Context,
    ) -> crate::app::app_trait::KeyDisposition {
        crate::app::app_trait::KeyDisposition::Consumed
    }

    fn draw_triple_tap_overlay(&self, ctx: &egui::Context, id: &str, count: u8, label: &str) {
        crate::ui::overlay::ModalShell::centered(id)
            .anchor(Align2::CENTER_BOTTOM, Vec2::new(0.0, -40.0))
            .scrim(false)
            .width(0.0)
            .show(ctx, &self.colors, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "{label} {} of 3 -- press again to delete context",
                            count
                        ))
                        .size(style::TEXT_CAPTION)
                        .color(self.colors.text_dim),
                    );
                    ui.add_space(style::SPACE_SM);
                    for i in 1u8..=3 {
                        let color = if i <= count {
                            self.colors.accent
                        } else {
                            self.colors.bg_active
                        };
                        let (rect, _) =
                            ui.allocate_exact_size(Vec2::new(8.0, 8.0), egui::Sense::hover());
                        ui.painter().circle_filled(rect.center(), 4.0, color);
                    }
                });
            });
    }
}
