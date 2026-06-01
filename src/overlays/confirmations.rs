use super::*;

impl PlexiApp {
    pub(crate) fn draw_inspector_delete_overlay(&self, ctx: &egui::Context) {
        self.draw_triple_tap_overlay(
            ctx,
            "inspector_delete_overlay",
            self.inspector_delete_press_count,
            "⌫ pressed",
        );
    }

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
        egui::Area::new(egui::Id::new("quit_confirm_overlay"))
            .anchor(Align2::CENTER_BOTTOM, Vec2::new(0.0, -40.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(self.colors.bg_sidebar)
                    .stroke(Stroke::new(1.0, self.colors.border))
                    .corner_radius(crate::style::RADIUS_LG)
                    .inner_margin(egui::Margin::symmetric(16, 10))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!(
                                    "\u{2318}Q pressed {} of 3 — press again to quit",
                                    count
                                ))
                                .size(12.0)
                                .color(self.colors.text_dim),
                            );
                            ui.add_space(8.0);
                            for i in 1u8..=3 {
                                let color = if i <= count {
                                    self.colors.accent
                                } else {
                                    self.colors.bg_active
                                };
                                let (rect, _) = ui.allocate_exact_size(
                                    Vec2::new(8.0, 8.0),
                                    egui::Sense::hover(),
                                );
                                ui.painter()
                                    .circle_filled(rect.center(), 4.0, color);
                            }
                        });
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

        // Scrim — visually reinforces modal capture and mirrors the command
        // palette / notification modal chrome.
        let screen_rect = ctx.screen_rect();
        egui::Area::new(egui::Id::new("confirm_close_scrim"))
            .fixed_pos(screen_rect.min)
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                ui.painter().rect_filled(
                    screen_rect,
                    0.0,
                    egui::Color32::from_black_alpha(120),
                );
                let scrim_response = ui.allocate_rect(screen_rect, egui::Sense::click());
                if scrim_response.clicked() {
                    cancelled = true;
                }
            });

        egui::Area::new(egui::Id::new("confirm_close_overlay"))
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(self.colors.bg_sidebar)
                    .stroke(egui::Stroke::new(1.0, self.colors.border))
                    .corner_radius(crate::style::RADIUS_LG)
                    .inner_margin(egui::Margin::symmetric(20, 16))
                    .show(ui, |ui| {
                        ui.set_width(MODAL_WIDTH);
                        ui.label(
                            RichText::new("Close pane?")
                                .size(13.0)
                                .color(self.colors.text_primary)
                                .strong(),
                        );
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new("The running process will be terminated.")
                                .size(12.0)
                                .color(self.colors.text_dim),
                        );
                        ui.add_space(12.0);
                        ui.horizontal(|ui| {
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("Close")
                                            .size(12.0)
                                            .color(self.colors.bg_darkest),
                                    )
                                    .fill(self.colors.danger),
                                )
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                            {
                                confirmed = true;
                            }
                            ui.add_space(8.0);
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("Cancel")
                                            .size(12.0)
                                            .color(self.colors.text_dim),
                                    )
                                    .fill(self.colors.bg_active),
                                )
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                            {
                                cancelled = true;
                            }
                            ui.add_space(12.0);
                            crate::widgets::key_chip(ui, "Enter", &self.colors);
                            ui.label(
                                RichText::new("confirm")
                                    .size(style::TEXT_HINT)
                                    .color(self.colors.text_dim),
                            );
                            ui.add_space(style::SPACE_SM);
                            crate::widgets::key_chip(ui, "Esc", &self.colors);
                            ui.label(
                                RichText::new("cancel")
                                    .size(style::TEXT_HINT)
                                    .color(self.colors.text_dim),
                            );
                        });
                    });
            });

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
        let (mut pending_prompts, mut outbound_events, mut permissions,
             mut secret_input_buf, mut permission_store, mut deferred_ai_queries,
             type_id, workspace_root, ai_broker, http_tx, proc_pane_id) = {
            let pane = match self.windows[active].panes.get_mut(&pane_id) {
                Some(crate::pane::Pane::App(a)) => a,
                _ => return,
            };
            let crate::pane::AppRuntime::Process(ref mut proc) = pane.runtime else { return };
            if proc.pending_prompts.is_empty() {
                return;
            }
            (
                std::mem::take(&mut proc.pending_prompts),
                std::mem::take(&mut proc.outbound_events),
                std::mem::take(&mut proc.permissions),
                std::mem::take(&mut proc.secret_input_buf),
                std::mem::take(&mut proc.permission_store),
                std::mem::take(&mut proc.deferred_ai_queries),
                proc.type_id.clone(),
                proc.workspace_root.clone(),
                std::sync::Arc::clone(&proc.ai_broker),
                proc.http_tx.clone(),
                proc.pane_id,
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
            &colors,
            &mut deferred_ai_queries,
            ai_broker,
            http_tx,
            proc_pane_id,
        );

        // Put the data back.
        let pane = match self.windows[active].panes.get_mut(&pane_id) {
            Some(crate::pane::Pane::App(a)) => a,
            _ => return,
        };
        let crate::pane::AppRuntime::Process(ref mut proc) = pane.runtime else { return };
        proc.pending_prompts = pending_prompts;
        proc.outbound_events = outbound_events;
        proc.permissions = permissions;
        proc.secret_input_buf = secret_input_buf;
        proc.permission_store = permission_store;
        proc.deferred_ai_queries = deferred_ai_queries;
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

        let screen_rect = ctx.screen_rect();
        egui::Area::new(egui::Id::new("ctx_close_confirm_scrim"))
            .fixed_pos(screen_rect.min)
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                ui.painter().rect_filled(screen_rect, 0.0, egui::Color32::from_black_alpha(120));
                let scrim_resp = ui.allocate_rect(screen_rect, egui::Sense::click());
                if scrim_resp.clicked() {
                    cancelled = true;
                }
            });

        let colors = self.colors;
        egui::Area::new(egui::Id::new("ctx_close_confirm_modal"))
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(colors.bg_sidebar)
                    .stroke(egui::Stroke::new(1.0, colors.border))
                    .corner_radius(crate::style::RADIUS_LG)
                    .inner_margin(egui::Margin::symmetric(20, 16))
                    .show(ui, |ui| {
                        ui.set_width(MODAL_WIDTH);

                        let title = if state.context_name.is_empty() {
                            "Close context?".to_string()
                        } else {
                            format!("Close \"{}\"?", state.context_name)
                        };
                        ui.label(
                            RichText::new(&title)
                                .size(13.0)
                                .color(colors.text_primary)
                                .strong(),
                        );
                        ui.add_space(8.0);

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
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("Close all")
                                            .size(12.0)
                                            .color(colors.bg_darkest),
                                    )
                                    .fill(colors.danger),
                                )
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                            {
                                close_all = true;
                            }
                            ui.add_space(6.0);
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("Dissolve")
                                            .size(12.0)
                                            .color(colors.text_dim),
                                    )
                                    .fill(colors.bg_active),
                                )
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                            {
                                dissolve = true;
                            }
                            ui.add_space(6.0);
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("Cancel")
                                            .size(12.0)
                                            .color(colors.text_dim),
                                    )
                                    .fill(colors.bg_active),
                                )
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                            {
                                cancelled = true;
                            }
                        });

                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            crate::widgets::key_chip(ui, "Enter", &colors);
                            ui.label(
                                RichText::new("close all")
                                    .size(style::TEXT_HINT)
                                    .color(colors.text_dim),
                            );
                            ui.add_space(style::SPACE_SM);
                            crate::widgets::key_chip(ui, "D", &colors);
                            ui.label(
                                RichText::new("dissolve")
                                    .size(style::TEXT_HINT)
                                    .color(colors.text_dim),
                            );
                            ui.add_space(style::SPACE_SM);
                            crate::widgets::key_chip(ui, "Esc", &colors);
                            ui.label(
                                RichText::new("cancel")
                                    .size(style::TEXT_HINT)
                                    .color(colors.text_dim),
                            );
                        });
                    });
            });

        if close_all {
            let idx = self.router.iter().position(|c| c.context_id == state.context_id);
            if let Some(i) = idx {
                log::info!("context_close: close_all ctx={} name={:?}", state.context_id, state.context_name);
                self.delete_context(i);
                self.save_workspace();
            } else {
                log::warn!("context_close: close_all ctx={} not found in router", state.context_id);
            }
        } else if dissolve {
            log::info!("context_close: dissolve ctx={} name={:?}", state.context_id, state.context_name);
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
    ) -> crate::app_trait::KeyDisposition {
        crate::app_trait::KeyDisposition::Consumed
    }

    pub(crate) fn context_close_confirm_handle_key(
        &mut self,
        _ctx: &egui::Context,
    ) -> crate::app_trait::KeyDisposition {
        crate::app_trait::KeyDisposition::Consumed
    }

    pub(crate) fn capability_modal_handle_key(
        &mut self,
        _ctx: &egui::Context,
    ) -> crate::app_trait::KeyDisposition {
        crate::app_trait::KeyDisposition::Consumed
    }
}
