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
        crate::ui::toast::ToastShell::bottom("quit_confirm_overlay").show(
            ctx,
            &self.colors,
            |ui| {
                ui.horizontal(|ui| {
                    crate::ui::toast::toast_caption(
                        ui,
                        format!("\u{2318}Q pressed {} of 3 — press again to quit", count),
                        &self.colors,
                    );
                    crate::ui::toast::ProgressDots::new(count, 3).show(ui, &self.colors);
                });
            },
        );
    }

    pub(crate) fn draw_confirm_close(&mut self, ctx: &egui::Context) {
        let actions = [
            crate::ui::dialog::DialogAction::new(
                "close",
                "Close",
                crate::ui::button::ButtonKind::Danger,
            )
            .shortcut(crate::ui::dialog::DialogShortcut::new(
                &["Enter"],
                egui::Modifiers::NONE,
                egui::Key::Enter,
            )),
            crate::ui::dialog::DialogAction::new(
                "cancel",
                "Cancel",
                crate::ui::button::ButtonKind::Secondary,
            )
            .shortcut(crate::ui::dialog::DialogShortcut::new(
                &["Esc"],
                egui::Modifiers::NONE,
                egui::Key::Escape,
            )),
        ];
        let response =
            crate::ui::dialog::ActionModal::new("confirm_close_overlay", "Close pane?", &actions)
                .width(super::MODAL_WIDTH)
                .show(ctx, &self.colors, |ui| {
                    crate::ui::typography::caption(
                        ui,
                        "The running process will be terminated.",
                        &self.colors,
                    );
                });

        let confirmed = response.selected == Some("close");
        let cancelled = response.dismissed || response.selected == Some("cancel");

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

    /// Capability / secret consent modal for the focused app pane.
    /// Called from step 2 of `update()` so it holds exclusive keyboard
    /// ownership before `dispatch_app_key_events` runs.
    pub(crate) fn draw_capability_modal(&mut self, ctx: &egui::Context) {
        let active = self.active_window;
        let pane_id = self.windows[active].focused_pane.and_then(|tile| {
            crate::app::PlexiApp::find_pane_in_tile(&self.windows[active].tree, tile)
        });
        if let Some(pane_id) = pane_id {
            if let Some(app) = self.windows[active]
                .panes
                .get_mut(&pane_id)
                .and_then(|pane| pane.as_app_mut())
            {
                if let crate::host::pane::AppRuntime::Wasm(runtime) = &mut app.runtime {
                    runtime.draw_capability_modal(ctx, &self.colors);
                }
            }
        }
    }

    /// Context-close confirmation dialog. Shows pane inventory with three choices:
    /// Close All (Enter), Dissolve (D), Cancel (Escape).
    pub(crate) fn draw_context_close_confirm(&mut self, ctx: &egui::Context) {
        let state = match self.pending_context_close.take() {
            Some(s) => s,
            None => return,
        };

        let colors = self.colors;
        let title = if state.context_name.is_empty() {
            "Close context?".to_string()
        } else {
            format!("Close \"{}\"?", state.context_name)
        };
        let mut actions = vec![crate::ui::dialog::DialogAction::new(
            "close_all",
            "Close all",
            crate::ui::button::ButtonKind::Danger,
        )
        .shortcut(crate::ui::dialog::DialogShortcut::new(
            &["Enter"],
            egui::Modifiers::NONE,
            egui::Key::Enter,
        ))];
        // Dissolve only exists for a context reached through a Portal tile.
        // Offering it on a top-level context reads as a broken command: the
        // key is consumed and `dissolve_portal` early-returns.
        if state.can_dissolve {
            actions.push(
                crate::ui::dialog::DialogAction::new(
                    "dissolve",
                    "Dissolve",
                    crate::ui::button::ButtonKind::Secondary,
                )
                .shortcut(crate::ui::dialog::DialogShortcut::new(
                    &["D"],
                    egui::Modifiers::NONE,
                    egui::Key::D,
                )),
            );
        }
        actions.push(
            crate::ui::dialog::DialogAction::new(
                "cancel",
                "Cancel",
                crate::ui::button::ButtonKind::Secondary,
            )
            .shortcut(crate::ui::dialog::DialogShortcut::new(
                &["Esc"],
                egui::Modifiers::NONE,
                egui::Key::Escape,
            )),
        );
        let response =
            crate::ui::dialog::ActionModal::new("ctx_close_confirm_modal", &title, &actions)
                .width(super::MODAL_WIDTH)
                .show(ctx, &colors, |ui| {
                    for item in &state.items {
                        let label = format!("{} — {}", item.kind, item.name);
                        crate::ui::typography::caption(ui, label, &colors);
                    }
                });

        let close_all = response.selected == Some("close_all");
        let dissolve = response.selected == Some("dissolve");
        let cancelled = response.dismissed || response.selected == Some("cancel");

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
        _input: &mut crate::app::input_router::PlexiInput,
    ) -> crate::app::app_trait::KeyDisposition {
        crate::app::app_trait::KeyDisposition::Consumed
    }

    pub(crate) fn context_close_confirm_handle_key(
        &mut self,
        _input: &mut crate::app::input_router::PlexiInput,
    ) -> crate::app::app_trait::KeyDisposition {
        crate::app::app_trait::KeyDisposition::Consumed
    }

    pub(crate) fn capability_modal_handle_key(
        &mut self,
        _input: &mut crate::app::input_router::PlexiInput,
    ) -> crate::app::app_trait::KeyDisposition {
        crate::app::app_trait::KeyDisposition::Consumed
    }

    /// Host event-bus consent modal. Surfaces the front parked
    /// [`PendingEventConsent`](crate::host::event_subscriptions::PendingEventConsent):
    /// a CLI/MCP agent's subscribe *or* publish request the broker answered with
    /// `Ask`. The subscriber identity shown is host-stamped — it is never taken
    /// from the user's choice. Enter allows once, `A` allows always (persists a grant),
    /// Esc/Deny refuses. Resolving the consent fires the transport's reply.
    pub(crate) fn draw_event_consent_modal(&mut self, ctx: &egui::Context) {
        use crate::host::event_subscriptions::ConsentChoice;
        let (subscriber, verb, target) = match self.pending_event_consents.front() {
            Some(c) => (c.subscriber_id.clone(), c.action_verb(), c.target_label()),
            None => return,
        };
        let colors = self.colors;
        let actions = [
            crate::ui::dialog::DialogAction::new(
                "allow_once",
                "Allow once",
                crate::ui::button::ButtonKind::Primary,
            )
            .shortcut(crate::ui::dialog::DialogShortcut::new(
                &["Enter"],
                egui::Modifiers::NONE,
                egui::Key::Enter,
            )),
            crate::ui::dialog::DialogAction::new(
                "allow_always",
                "Always",
                crate::ui::button::ButtonKind::Secondary,
            )
            .shortcut(crate::ui::dialog::DialogShortcut::new(
                &["A"],
                egui::Modifiers::NONE,
                egui::Key::A,
            )),
            crate::ui::dialog::DialogAction::new(
                "deny",
                "Deny",
                crate::ui::button::ButtonKind::Danger,
            )
            .shortcut(crate::ui::dialog::DialogShortcut::new(
                &["Esc"],
                egui::Modifiers::NONE,
                egui::Key::Escape,
            )),
        ];
        let response = crate::ui::dialog::ActionModal::new(
            "event_consent_overlay",
            "Event subscription request",
            &actions,
        )
        .width(super::MODAL_WIDTH)
        .show(ctx, &colors, |ui| {
            crate::ui::typography::caption(ui, format!("{subscriber} wants to {verb}:"), &colors);
            crate::ui::typography::caption(ui, target, &colors);
        });

        let choice = if response.selected == Some("allow_once") {
            Some(ConsentChoice::AllowOnce)
        } else if response.selected == Some("allow_always") {
            Some(ConsentChoice::AllowAlways)
        } else if response.dismissed || response.selected == Some("deny") {
            Some(ConsentChoice::Deny)
        } else {
            None
        };

        if let Some(choice) = choice {
            if let Some(consent) = self.pending_event_consents.pop_front() {
                let config_dir = crate::config::config_dir();
                self.host_subscriptions
                    .resolve_consent(consent, choice, &config_dir);
            }
        }
    }

    pub(crate) fn event_consent_handle_key(
        &mut self,
        _input: &mut crate::app::input_router::PlexiInput,
    ) -> crate::app::app_trait::KeyDisposition {
        crate::app::app_trait::KeyDisposition::Consumed
    }

    pub(crate) fn draw_raw_wasm_review_modal(&mut self, ctx: &egui::Context) {
        let Some(review) = self.pending_raw_wasm_launches.front().cloned() else {
            return;
        };
        let colors = self.colors;
        let actions = [
            crate::ui::dialog::DialogAction::new(
                "allow",
                "Allow and remember",
                crate::ui::button::ButtonKind::Primary,
            )
            .shortcut(crate::ui::dialog::DialogShortcut::new(
                &["Enter"],
                egui::Modifiers::NONE,
                egui::Key::Enter,
            )),
            crate::ui::dialog::DialogAction::new(
                "deny",
                "Deny",
                crate::ui::button::ButtonKind::Danger,
            )
            .shortcut(crate::ui::dialog::DialogShortcut::new(
                &["Esc"],
                egui::Modifiers::NONE,
                egui::Key::Escape,
            )),
        ];
        let response = crate::ui::dialog::ActionModal::new(
            "raw_wasm_review_overlay",
            "Raw WASM import review",
            &actions,
        )
        .width(super::MODAL_WIDTH)
        .show(ctx, &colors, |ui| {
            crate::ui::typography::caption(
                ui,
                format!("{} requests host imports:", review.app_id),
                &colors,
            );
            for capability in &review.missing_capabilities {
                crate::ui::typography::caption(ui, format!("- {capability}"), &colors);
            }
            crate::ui::typography::caption(
                ui,
                format!("Path: {}", review.wasm_path.display()),
                &colors,
            );
        });

        if response.selected == Some("allow") {
            let Some(review) = self.pending_raw_wasm_launches.pop_front() else {
                return;
            };
            let mut store = crate::app::permissions::PermissionStore::load_or_default(
                &crate::config::config_dir(),
            );
            for capability in &review.missing_capabilities {
                store.set_wasm(
                    &review.app_id,
                    &review.workspace_root,
                    capability,
                    crate::app::permissions::PermissionState::Green,
                );
            }
            store.save();
            log::info!(
                "raw_wasm_review: approved {} imports for app_id={} path={}",
                review.missing_capabilities.len(),
                review.app_id,
                review.wasm_path.display()
            );
            if let Err(err) = self.open_wasm_app_pane(
                &review.app_id,
                &review.wasm_path,
                review.workspace_root,
                review.launch_args,
            ) {
                log::warn!(
                    "raw_wasm_review: approved launch failed for app_id={} path={}: {err}",
                    review.app_id,
                    review.wasm_path.display()
                );
            }
        } else if response.dismissed || response.selected == Some("deny") {
            if let Some(review) = self.pending_raw_wasm_launches.pop_front() {
                log::info!(
                    "raw_wasm_review: denied app_id={} path={}",
                    review.app_id,
                    review.wasm_path.display()
                );
            }
        }
    }

    pub(crate) fn raw_wasm_review_handle_key(
        &mut self,
        _input: &mut crate::app::input_router::PlexiInput,
    ) -> crate::app::app_trait::KeyDisposition {
        crate::app::app_trait::KeyDisposition::Consumed
    }

    fn draw_triple_tap_overlay(&self, ctx: &egui::Context, id: &str, count: u8, label: &str) {
        crate::ui::toast::ToastShell::bottom(id).show(ctx, &self.colors, |ui| {
            ui.horizontal(|ui| {
                crate::ui::toast::toast_caption(
                    ui,
                    format!("{label} {} of 3 -- press again to delete context", count),
                    &self.colors,
                );
                crate::ui::toast::ProgressDots::new(count, 3).show(ui, &self.colors);
            });
        });
    }
}
