//! Focus, navigation, and configuration methods for PlexiApp.

use super::PendingNotification;
use super::PlexiApp;

/// Which layer currently owns keyboard input.
///
/// The top of `PlexiApp.focus_stack` is the active layer. When a non-`Pane`
/// layer is on top, keyboard `Event::Key` and `Event::Text` events are drained
/// from `ctx.input` each frame *after* the owning overlay has rendered, so
/// panes and other passive readers see an empty event buffer. Global
/// keybinds (Cmd+Q, Cmd+W, Cmd+Shift+A) are handled in `keys::poll_actions`
/// which runs before the drain and is always live.
///
/// New overlays should push their layer on open and pop on close to inherit
/// input capture. All keyboard-owning overlays live here: notification modal,
/// confirm-close, command palette, run palette, and rename-pane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FocusLayer {
    NotificationModal,
    ConfirmClose,
    CommandPalette,
    RenamePane,
    /// Context naming modal shown when a new context is created while the
    /// sidebar is hidden. Mirrors the inline sidebar rename but as a centred
    /// overlay so the terminal is immediately usable after dismissal.
    ContextRename,
    /// Context description editor overlay.
    ContextDescription,
    /// Quick note compose modal (text input phase).
    QuickNote,
    /// Quick note destination picker.
    QuickNoteDestination,
    /// Quick note sub-destination picker. Inner Vec<u8> = key path from root to current node.
    /// E.g. vec![3] = inside destination 3's children; vec![3,2] = destination 3 -> child 2.
    QuickNoteSubDestination(Vec<u8>),
    /// First-launch CLI setup prompt. No text input — intercepts keys so they
    /// don't fall through to the active terminal while the modal is visible.
    CliSetupPrompt,
    /// Shared text-input overlay (context root, future: context rename).
    TextInput,
    /// Close-context confirmation dialog with pane inventory and dissolve option.
    ContextCloseConfirm,
    /// Capability / secret consent modal for a focused ProcessApp pane.
    /// Promoted to the focus stack when the focused pane has pending prompts,
    /// so the modal renders in step 2 of `update()` with exclusive keyboard
    /// ownership — before `dispatch_app_key_events` can steal Escape.
    CapabilityModal,
    /// Notes picker overlay: lists workspace notes sorted by mtime, opens selected in focused text-editor.
    NotesPicker,
}

/// A single pane entry shown in the context-close confirmation dialog.
#[derive(Clone, Debug)]
pub(crate) struct ContextCloseItem {
    pub kind: &'static str,
    pub name: String,
}

/// State for the context-close confirmation dialog.
#[derive(Clone, Debug)]
pub(crate) struct ContextCloseState {
    pub context_id: u64,
    pub context_name: String,
    pub items: Vec<ContextCloseItem>,
}

impl PlexiApp {
    /// Collect metadata for the pane at `tile_id` in the window identified by
    /// stable `window_id` and emit a `FocusChanged` event. Called when the
    /// focused pane changes and on shutdown.
    pub(super) fn emit_focus_changed_for_tile(
        &self,
        window_id: u64,
        tile_id: egui_tiles::TileId,
        duration_secs: u64,
    ) {
        use egui_tiles::Tile;
        let Some(win) = self.windows.iter().find(|w| w.window_id == window_id) else {
            return;
        };
        let pane_id = match win.tree.tiles.get(tile_id) {
            Some(Tile::Pane(id)) => *id,
            _ => return,
        };
        let Some(pane) = win.panes.get(&pane_id) else {
            return;
        };
        let context_name = self.context_name_for(win.context_id);
        let context_description = self.context_description_for(win.context_id);

        let (cwd, pty_title, pane_name, app_type_id) = match pane {
            crate::host::pane::Pane::Terminal(t) => {
                let cwd = crate::host::shell::get_pid_cwd(t.backend.child_pid())
                    .map(|p| p.to_string_lossy().into_owned());
                (cwd, t.pty_title.clone(), t.name.clone(), None)
            }
            crate::host::pane::Pane::App(a) => {
                let cwd = Some(a.workspace_root.to_string_lossy().into_owned());
                let type_id = Some(a.manifest_id.clone());
                (cwd, None, None, type_id)
            }
            crate::host::pane::Pane::Portal(_) => (None, None, None, None),
        };

        log::info!(
            "focus_changed: pane_id={pane_id} context={context_name:?} duration_secs={duration_secs} pty_title={pty_title:?} pane_name={pane_name:?} app_type_id={app_type_id:?}"
        );
        crate::host::event_log::emit(crate::host::event_log::HostEvent::FocusChanged {
            pane_id,
            context_name,
            context_description,
            cwd,
            pty_title,
            pane_name,
            app_type_id,
            duration_secs,
            timestamp: crate::host::event_log::now_timestamp(),
        });
    }

    /// Returns `true` when the current top focus layer is a non-critical modal
    /// that QuickNote (Cmd+0) is allowed to dismiss and replace.
    ///
    /// Critical modals (`ConfirmClose`, `CapabilityModal`, `ContextCloseConfirm`)
    /// require explicit user acknowledgement and must NOT be preempted.
    /// Non-critical modals (`NotificationModal`, `CommandPalette`) can be safely
    /// dismissed so QuickNote can open on top.
    pub(crate) fn is_quick_note_preemptable(&self) -> bool {
        matches!(
            self.focus_stack.last(),
            Some(FocusLayer::NotificationModal) | Some(FocusLayer::CommandPalette)
        )
    }

    /// Dismiss the current non-critical modal so QuickNote can open on top.
    /// Only call after `is_quick_note_preemptable()` returns `true`.
    pub(crate) fn dismiss_preemptable_modal(&mut self) {
        match self.focus_stack.last().cloned() {
            Some(FocusLayer::NotificationModal) => {
                log::info!("quick_note: dismissing NotificationModal to open QuickNote");
                self.show_notification_modal = false;
                self.focus_stack
                    .retain(|l| *l != FocusLayer::NotificationModal);
            }
            Some(FocusLayer::CommandPalette) => {
                log::info!("quick_note: dismissing CommandPalette to open QuickNote");
                self.show_command_palette = false;
                self.focus_stack
                    .retain(|l| *l != FocusLayer::CommandPalette);
                self.ctx.memory_mut(|m| {
                    let palette_id = egui::Id::new("palette_search");
                    if m.focused() == Some(palette_id) {
                        m.surrender_focus(palette_id);
                    }
                });
            }
            _ => {}
        }
    }

    /// True when a modal overlay owns keyboard input. Used by `update()` to
    /// drain remaining key events after the overlay has rendered so panes see
    /// an empty input buffer this frame.
    pub(crate) fn input_captured_by_overlay(&self) -> bool {
        matches!(
            self.focus_stack.last(),
            Some(FocusLayer::NotificationModal)
                | Some(FocusLayer::ConfirmClose)
                | Some(FocusLayer::CommandPalette)
                | Some(FocusLayer::RenamePane)
                | Some(FocusLayer::ContextRename)
                | Some(FocusLayer::ContextDescription)
                | Some(FocusLayer::QuickNote)
                | Some(FocusLayer::QuickNoteDestination)
                | Some(FocusLayer::QuickNoteSubDestination(_))
                | Some(FocusLayer::CliSetupPrompt)
                | Some(FocusLayer::TextInput)
                | Some(FocusLayer::ContextCloseConfirm)
                | Some(FocusLayer::CapabilityModal)
                | Some(FocusLayer::NotesPicker)
        )
    }

    /// Push a focus layer. Idempotent — if the same layer is already on top,
    /// it's a no-op. Callers should pair with `pop_focus_layer`.
    pub(crate) fn push_focus_layer(&mut self, layer: FocusLayer) {
        if self.focus_stack.last() != Some(&layer) {
            self.focus_stack.push(layer);
        }
    }

    /// Pop the given layer if it's currently on top. No-op otherwise; this
    /// prevents out-of-order pops from corrupting the stack.
    pub(crate) fn pop_focus_layer(&mut self, layer: &FocusLayer) {
        if self.focus_stack.last() == Some(layer) {
            self.focus_stack.pop();
        }
    }

    /// Reconcile the focus stack with the notification modal visibility. Called
    /// once per frame — the modal can open/close from many paths (arrival,
    /// Cmd+Shift+A, queue drains to empty mid-frame), so the source of truth is
    /// `show_notification_modal && !pending_notifications.is_empty()`.
    /// Read `confirm_quit` from the config tunnel. Defaults to `true` so
    /// users get the safer triple-tap behavior unless they explicitly opt out.
    pub(crate) fn confirm_quit(&self) -> bool {
        self.config.confirm_quit.unwrap_or(true)
    }

    /// Read `confirm_close` from the config tunnel. Defaults to `false` so
    /// pane close is instant unless the user explicitly enables the dialog.
    pub(crate) fn confirm_close(&self) -> bool {
        self.config.confirm_close.unwrap_or(false)
    }

    /// If the currently focused pane is an app with a non-empty nav stack,
    /// emit `PlexiEvent::NavBack` to it and return `true`. Returns `false`
    /// when there is no nav-active focused pane (caller may fall back to
    /// default behaviour such as closing the pane or cycling tabs).
    pub(crate) fn try_nav_back_focused(&mut self) -> bool {
        let active = self.active_window;
        let active_ctx = &self.windows[active];

        // Read nav state under shared borrow first.
        let nav_result = active_ctx.focused_pane.and_then(|tile_id| {
            if let Some(egui_tiles::Tile::Pane(pane_id)) = active_ctx.tree.tiles.get(tile_id) {
                active_ctx.panes.get(pane_id).and_then(|pane| {
                    pane.as_app().and_then(|app| {
                        if app.runtime.nav_stack_depth() > 0 {
                            Some((*pane_id, app.runtime.nav_back_view_id()))
                        } else {
                            None
                        }
                    })
                })
            } else {
                None
            }
        });

        if let Some((pane_id, view_id)) = nav_result {
            if let Some(pane) = self.windows[active].panes.get_mut(&pane_id) {
                if let Some(app) = pane.as_app_mut() {
                    app.runtime
                        .queue_outbound_event(crate::app_protocol::PlexiEvent::NavBack { view_id });
                }
            }
            true
        } else {
            false
        }
    }

    pub(crate) fn push_focus_history(
        &mut self,
        window_id: u64,
        old_focus: Option<egui_tiles::TileId>,
    ) {
        if self.navigating_history {
            return;
        }
        let Some(tile_id) = old_focus else { return };
        self.pane_focus_history.push((window_id, tile_id));
        if self.pane_focus_history.len() > self.focus_history_depth {
            self.pane_focus_history.remove(0);
        }
        self.pane_focus_future.clear();
        log::info!(
            "focus_history: recorded window={window_id} tile={tile_id:?} history_len={}",
            self.pane_focus_history.len()
        );
    }

    /// Step backward through pane focus history (Cmd+[).
    /// Skips stale entries where the window or tile no longer exists.
    pub(crate) fn step_focus_history_back(&mut self) {
        self.navigating_history = true;
        loop {
            let Some((window_id, tile_id)) = self.pane_focus_history.pop() else {
                log::info!("focus_history: back exhausted");
                self.navigating_history = false;
                return;
            };
            let window_idx = self.windows.iter().position(|w| w.window_id == window_id);
            let Some(idx) = window_idx else {
                log::info!("focus_history: skipping stale entry window={window_id} tile={tile_id:?} (window gone)");
                continue;
            };
            if self.windows[idx].tree.tiles.get(tile_id).is_none() {
                log::info!("focus_history: skipping stale entry window={window_id} tile={tile_id:?} (tile gone)");
                continue;
            }
            // Save current focus to future stack before navigating.
            let current_window_id = self.windows[self.active_window].window_id;
            if let Some(current_tile) = self.windows[self.active_window].focused_pane {
                self.pane_focus_future
                    .push((current_window_id, current_tile));
                if self.pane_focus_future.len() > self.focus_history_depth {
                    self.pane_focus_future.remove(0);
                }
            }
            self.windows[idx].navigate_to(tile_id);
            self.active_window = idx;
            // Sync sidebar: router active must match the context of the window we navigated to.
            let ctx_id = self.windows[idx].context_id;
            if let Some(ctx_idx) = self.router.position(|c| c.context_id == ctx_id) {
                self.router.set_active(ctx_idx);
            }
            log::info!("focus_history: back — to window={window_id} tile={tile_id:?} ctx={ctx_id} history_len={}", self.pane_focus_history.len());
            break;
        }
        self.navigating_history = false;
    }

    /// Step forward through pane focus future (Cmd+]).
    /// Skips stale entries where the window or tile no longer exists.
    pub(crate) fn step_focus_history_forward(&mut self) {
        self.navigating_history = true;
        loop {
            let Some((window_id, tile_id)) = self.pane_focus_future.pop() else {
                log::info!("focus_history: forward exhausted");
                self.navigating_history = false;
                return;
            };
            let window_idx = self.windows.iter().position(|w| w.window_id == window_id);
            let Some(idx) = window_idx else {
                log::info!("focus_history: skipping stale entry window={window_id} tile={tile_id:?} (window gone)");
                continue;
            };
            if self.windows[idx].tree.tiles.get(tile_id).is_none() {
                log::info!("focus_history: skipping stale entry window={window_id} tile={tile_id:?} (tile gone)");
                continue;
            }
            // Save current focus to history stack before navigating.
            let current_window_id = self.windows[self.active_window].window_id;
            if let Some(current_tile) = self.windows[self.active_window].focused_pane {
                self.pane_focus_history
                    .push((current_window_id, current_tile));
                if self.pane_focus_history.len() > self.focus_history_depth {
                    self.pane_focus_history.remove(0);
                }
            }
            self.windows[idx].navigate_to(tile_id);
            self.active_window = idx;
            // Sync sidebar: router active must match the context of the window we navigated to.
            let ctx_id = self.windows[idx].context_id;
            if let Some(ctx_idx) = self.router.position(|c| c.context_id == ctx_id) {
                self.router.set_active(ctx_idx);
            }
            log::info!("focus_history: forward — to window={window_id} tile={tile_id:?} ctx={ctx_id} future_len={}", self.pane_focus_future.len());
            break;
        }
        self.navigating_history = false;
    }

    /// Re-read configuration from disk and apply changes that can take
    /// effect without a restart (theme, font size, notification settings,
    /// confirmation toggles). Logs the reload so the user knows it worked.
    pub(crate) fn reload_config(&mut self) {
        let active_workspace = crate::config::active_workspace_root();

        let mut all_diags = crate::config::validate_from_path(&crate::config::config_path());
        if let Some(root) = active_workspace.as_ref() {
            let project_path = root
                .join(crate::config::workspace_channel_dir())
                .join("config.toml");
            all_diags.extend(crate::config::validate_from_path(&project_path));
        }

        let has_errors = all_diags.iter().any(|d| d.is_error());

        let warnings: Vec<_> = all_diags.iter().filter(|d| !d.is_error()).collect();
        if !warnings.is_empty() {
            let body = warnings
                .iter()
                .map(|d| d.to_string())
                .collect::<Vec<_>>()
                .join("\n");
            log::warn!("config: unknown keys found:\n{body}");
        }

        if has_errors {
            let error_msg = all_diags
                .iter()
                .filter(|d| d.is_error())
                .map(|d| d.to_string())
                .collect::<Vec<_>>()
                .join("\n");
            log::warn!("config: parse error, keeping current config:\n{error_msg}");
            let notify_id = format!(
                "config-error-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis())
                    .unwrap_or(0)
            );
            self.pending_notifications.push(PendingNotification {
                notify_id: notify_id.clone(),
                sender_pane_id: 0,
                source_context_id: 0,
                source_window_id: 0,
                level: "error".to_string(),
                title: "Config Error".to_string(),
                body: error_msg,
                kind: crate::app_protocol::NotifyKind::Message,
                options: vec![],
                input_prompt: None,
                required: false,
                priority: 100,
                scope: crate::app_protocol::NotifyScope::Global,
                image_inline: None,
                image_pipe_id: None,
                response_file: None,
                timeout_secs: None,
                on_dismiss: None,
                enqueued_at: std::time::Instant::now(),
                tombstoned: false,
                deliver_after: None,
            });
            self.save_notifications();
            if !self.notifications_focus_mode {
                self.show_notification_modal = true;
                if self.current_notify_id.is_none() {
                    self.current_notify_id = Some(notify_id);
                }
            }
            return;
        }

        let fresh = crate::config::PlexiConfig::load_with_workspace(active_workspace.as_deref());

        // Theme
        let theme_cfg = Self::resolve_theme_config(&fresh);
        let new_colors = crate::ui::theme::Colors::from_config(&theme_cfg);
        if self.colors != new_colors {
            self.colors = new_colors.clone();
            let dark_mode =
                !crate::ui::theme::is_light_preset(fresh.theme_preset.as_deref().unwrap_or(""));
            crate::ui::theme::setup_style(&self.ctx, &new_colors, dark_mode);
            let window_theme = if dark_mode {
                egui::SystemTheme::Dark
            } else {
                egui::SystemTheme::Light
            };
            self.ctx
                .send_viewport_cmd(egui::ViewportCommand::SetTheme(window_theme));
            log::info!("theme: set_window_theme dark_mode={dark_mode} (config reload)");
            self.broadcast_theme_event();
        }

        // Terminal theme
        self.theme = crate::ui::theme::terminal_theme(&theme_cfg);

        // Font size
        if let Some(size) = fresh.font_size {
            if (size - self.default_font_size).abs() > 0.01 {
                self.default_font_size = size;
            }
        }

        // Notifications
        self.notifications_enabled = fresh
            .notifications
            .as_ref()
            .and_then(|n| n.enabled)
            .unwrap_or(true);
        self.notifications_focus_mode = fresh
            .notifications
            .as_ref()
            .and_then(|n| n.focus_mode)
            .unwrap_or(false);
        self.notifications_interrupt_threshold = fresh
            .notifications
            .as_ref()
            .and_then(|n| n.interrupt_threshold)
            .unwrap_or(100);

        self.focus_history_depth = fresh.focus_history_depth.unwrap_or(100);

        // Feature flags
        self.features = crate::features::FeatureFlags::from_config(&fresh);

        // Replace the cached config
        self.config = fresh;
        self.key_bindings = crate::host::keys::build_key_bindings(self.config.keybindings.as_ref());
        self.binding_table = crate::host::keys::build_binding_table(&self.key_bindings);
        log::info!("keybindings: rebuilt after config reload");

        // AI broker config — broadcast fresh snapshot to all living panes and background apps
        let fresh_ai = self.config.ai.clone();
        for win in &mut self.windows {
            for pane in win.panes.values_mut() {
                if let Some(app) = pane.as_app_mut() {
                    if let crate::host::pane::AppRuntime::Process(proc) = &mut app.runtime {
                        proc.update_ai_config(fresh_ai.clone());
                    }
                }
            }
        }
        for app_entry in self.background_apps.values_mut() {
            app_entry.1.update_ai_config(fresh_ai.clone());
        }
        log::info!("ai_broker: config reloaded");

        // Reset so the auto-switch re-evaluates against the current system theme (#1776, #1812).
        // Without this, a config reload that restores a disk preset would leave
        // last_system_theme unchanged, silently suppressing the auto-switch.
        self.last_system_theme = None;
        log::info!("Configuration reloaded from disk.");
    }

    /// Auto-switch to the paired preset for `system_theme`.
    /// No-ops if the configured preset has no paired variant (e.g. nord, dracula).
    pub(super) fn apply_auto_theme(&mut self, system_theme: egui::Theme) {
        let current_preset = self.config.theme_preset.as_deref().unwrap_or("");
        let Some(new_preset) = crate::ui::theme::paired_preset(current_preset, system_theme) else {
            return;
        };
        log::info!("theme: auto-switch to {new_preset} (system_theme={system_theme:?})");
        if let Some(preset) = crate::ui::theme::preset_colors(new_preset) {
            let user_theme = self.config.theme.clone().unwrap_or_default();
            let theme_cfg = crate::ui::theme::apply_preset(&preset, &user_theme);
            let new_colors = crate::ui::theme::Colors::from_config(&theme_cfg);
            if self.colors != new_colors {
                self.colors = new_colors.clone();
                let dark_mode = !crate::ui::theme::is_light_preset(new_preset);
                crate::ui::theme::setup_style(&self.ctx, &new_colors, dark_mode);
                let window_theme = if dark_mode {
                    egui::SystemTheme::Dark
                } else {
                    egui::SystemTheme::Light
                };
                self.ctx
                    .send_viewport_cmd(egui::ViewportCommand::SetTheme(window_theme));
                self.theme = crate::ui::theme::terminal_theme(&theme_cfg);
                self.broadcast_theme_event();
            }
        }
    }

    /// Push the current host `Colors` to every running app as a `Theme` event.
    /// Called after `self.colors` is updated — both on config hot-reload and on
    /// macOS system-appearance change — so apps never need to poll for theme changes.
    fn broadcast_theme_event(&mut self) {
        let event = crate::app_protocol::PlexiEvent::Theme {
            colors: self.colors.to_theme_map(),
        };
        for win in &mut self.windows {
            for pane in win.panes.values_mut() {
                if let Some(app) = pane.as_app_mut() {
                    if let crate::host::pane::AppRuntime::Process(proc) = &mut app.runtime {
                        proc.send_event(&event);
                    }
                }
            }
        }
        for app_entry in self.background_apps.values_mut() {
            app_entry.1.send_event(&event);
        }
        log::info!("theme: broadcast Theme event to all running apps");
    }

    /// Reconcile the confirm-close focus layer with `pending_close`. Mirrors
    /// `sync_notification_modal_focus` — the source of truth is a boolean
    /// toggled from multiple paths, and the focus stack must follow it
    /// deterministically each frame.
    pub(crate) fn sync_confirm_close_focus(&mut self) {
        let should_own = self.pending_close;
        let has_layer = self
            .focus_stack
            .iter()
            .any(|l| *l == FocusLayer::ConfirmClose);
        if should_own && !has_layer {
            self.push_focus_layer(FocusLayer::ConfirmClose);
        } else if !should_own && has_layer {
            log::info!("focus: ConfirmClose layer removed by sync (retain)");
            self.focus_stack.retain(|l| *l != FocusLayer::ConfirmClose);
        }
    }

    pub(crate) fn sync_context_close_focus(&mut self) {
        let should_own = self.pending_context_close.is_some();
        let has_layer = self
            .focus_stack
            .iter()
            .any(|l| *l == FocusLayer::ContextCloseConfirm);
        if should_own && !has_layer {
            self.push_focus_layer(FocusLayer::ContextCloseConfirm);
        } else if !should_own && has_layer {
            log::info!("focus: ContextCloseConfirm layer removed by sync (retain)");
            self.focus_stack
                .retain(|l| *l != FocusLayer::ContextCloseConfirm);
        }
    }

    /// Returns the `context_id` of the child context if the focused pane is a Portal tile.
    pub(crate) fn get_focused_portal_context_id(&self) -> Option<u64> {
        let win = &self.windows[self.active_window];
        let focused_tile = win.focused_pane?;
        let pane_id = match win.tree.tiles.get(focused_tile) {
            Some(egui_tiles::Tile::Pane(id)) => *id,
            _ => return None,
        };
        win.panes.get(&pane_id)?.portal_target()
    }

    /// Collect the pane inventory for a child context close dialog.
    pub(crate) fn build_context_close_state(&self, context_id: u64) -> ContextCloseState {
        let context_name = self
            .router
            .iter()
            .find(|c| c.context_id == context_id)
            .map(|c| c.name.clone())
            .unwrap_or_default();

        let mut items = Vec::new();
        for win in &self.windows {
            if win.context_id != context_id {
                continue;
            }
            let mut pane_entries: Vec<_> = win.panes.iter().collect();
            pane_entries.sort_by_key(|(id, _)| *id);
            for (_, pane) in pane_entries {
                match pane {
                    crate::host::pane::Pane::Terminal(t) => {
                        let name = t
                            .name
                            .clone()
                            .or_else(|| t.pty_title.clone())
                            .unwrap_or_else(|| "Terminal".to_string());
                        items.push(ContextCloseItem {
                            kind: "Terminal",
                            name,
                        });
                    }
                    crate::host::pane::Pane::App(a) => {
                        items.push(ContextCloseItem {
                            kind: "App",
                            name: a.name.clone(),
                        });
                    }
                    crate::host::pane::Pane::Portal(p) => {
                        let name = self
                            .router
                            .iter()
                            .find(|c| c.context_id == p.target_context_id)
                            .map(|c| c.name.clone())
                            .unwrap_or_else(|| "Portal".to_string());
                        items.push(ContextCloseItem {
                            kind: "Context",
                            name,
                        });
                    }
                }
            }
        }

        ContextCloseState {
            context_id,
            context_name,
            items,
        }
    }

    pub(crate) fn sync_notification_modal_focus(&mut self) {
        let should_own = self.show_notification_modal;
        let has_layer = self
            .focus_stack
            .iter()
            .any(|l| *l == FocusLayer::NotificationModal);
        if should_own && !has_layer {
            self.push_focus_layer(FocusLayer::NotificationModal);
        } else if !should_own && has_layer {
            log::info!("focus: NotificationModal layer removed by sync (retain)");
            self.focus_stack
                .retain(|l| *l != FocusLayer::NotificationModal);
        }
    }

    pub(crate) fn sync_cli_setup_prompt_focus(&mut self) {
        let should_own = self.show_cli_setup_prompt;
        let has_layer = self
            .focus_stack
            .iter()
            .any(|l| *l == FocusLayer::CliSetupPrompt);
        if should_own && !has_layer {
            log::info!("cli_setup: focus captured by CliSetupPrompt layer");
            self.push_focus_layer(FocusLayer::CliSetupPrompt);
        } else if !should_own && has_layer {
            log::info!("cli_setup: CliSetupPrompt focus layer released");
            // Use retain rather than pop_focus_layer so stale entries are removed
            // even if another layer was pushed on top (e.g. via rapid state change).
            self.focus_stack
                .retain(|l| *l != FocusLayer::CliSetupPrompt);
        }
    }

    /// Reconcile the command-palette focus layer with `show_command_palette`.
    /// Same pattern as the notification modal: boolean visibility flag is the
    /// source of truth, focus stack follows it deterministically each frame.
    pub(crate) fn sync_command_palette_focus(&mut self) {
        let should_own = self.show_command_palette;
        let has_layer = self
            .focus_stack
            .iter()
            .any(|l| *l == FocusLayer::CommandPalette);
        if should_own && !has_layer {
            self.push_focus_layer(FocusLayer::CommandPalette);
        } else if !should_own && has_layer {
            log::info!("focus: CommandPalette layer removed by sync (retain)");
            self.focus_stack
                .retain(|l| *l != FocusLayer::CommandPalette);
            // Explicitly surrender egui focus from palette_search so AccessKit
            // doesn't hold a stale focused node ID after the widget is gone.
            self.ctx.memory_mut(|m| {
                let palette_id = egui::Id::new("palette_search");
                if m.focused() == Some(palette_id) {
                    log::info!("palette: surrendering palette_search focus on dismiss");
                    m.surrender_focus(palette_id);
                }
            });
        }
    }

    /// Navigate to a pane by id, updating both `focused_pane` on its window and
    /// `active_window`. Returns `true` if the pane was found.
    pub(crate) fn pane_navigate(&mut self, pane_id: u64) -> bool {
        // Read-only pass: find the window index, tile_id, and context_id before mutating.
        // Using iter() instead of iter_mut() so self.windows borrow ends before we call
        // push_focus_history (which needs &mut self).
        let found_read = self.windows.iter().enumerate().find_map(|(idx, win)| {
            win.tree
                .tiles
                .find_pane(&pane_id)
                .map(|tile_id| (idx, tile_id, win.context_id))
        });
        let Some((idx, tile_id, ctx_id)) = found_read else {
            log::warn!("notify:action: pane_navigate pane_id={pane_id} not found");
            return false;
        };
        let old_focus = self.windows[self.active_window].focused_pane;
        let old_window_id = self.windows[self.active_window].window_id;
        // Clear any stale zoom on the destination window — a programmatic focus
        // redirect must not leave zoomed_pane pointing at a pane that is no longer focused.
        if self.windows[idx].zoomed_pane.is_some() {
            self.windows[idx].clear_zoom();
            log::info!("notify:action: pane_navigate cleared stale zoom on window={idx}");
        }
        // navigate_to sets focused_pane and activates the ancestor Tabs container.
        self.windows[idx].navigate_to(tile_id);
        self.push_focus_history(old_window_id, old_focus);
        let prev = self.active_window;
        self.active_window = idx;
        // Sync the router so the sidebar context switcher reflects the
        // new active context immediately (router.active_idx() drives the highlight).
        if let Some(ctx_idx) = self.router.position(|ctx| ctx.context_id == ctx_id) {
            self.router.set_active(ctx_idx);
            log::info!(
                "notify:action: pane_navigate active_window {prev}→{idx} ctx_idx={ctx_idx} pane_id={pane_id}"
            );
        } else {
            log::warn!(
                "notify:action: pane_navigate active_window {prev}→{idx} pane_id={pane_id} ctx_id={ctx_id} not found in router"
            );
        }
        true
    }

    /// Reconcile the rename-pane focus layer with `renaming_pane`.
    pub(crate) fn sync_rename_pane_focus(&mut self) {
        let should_own = self.renaming_pane.is_some();
        let has_layer = self
            .focus_stack
            .iter()
            .any(|l| *l == FocusLayer::RenamePane);
        if should_own && !has_layer {
            self.push_focus_layer(FocusLayer::RenamePane);
        } else if !should_own && has_layer {
            log::info!("focus: RenamePane layer removed by sync (retain)");
            self.focus_stack.retain(|l| *l != FocusLayer::RenamePane);
        }
    }

    /// Reconcile the context-rename focus layer. Active when `renaming_window`
    /// is set AND the sidebar is hidden -- in that case the inline sidebar row
    /// never renders, so we promote the rename to a modal overlay instead.
    pub(crate) fn sync_context_rename_focus(&mut self) {
        let should_own = self.renaming_window.is_some() && !self.sidebar_visible;
        let has_layer = self
            .focus_stack
            .iter()
            .any(|l| *l == FocusLayer::ContextRename);
        if should_own && !has_layer {
            self.push_focus_layer(FocusLayer::ContextRename);
        } else if !should_own && has_layer {
            log::info!("focus: ContextRename layer removed by sync (retain)");
            self.focus_stack.retain(|l| *l != FocusLayer::ContextRename);
        }
    }

    /// Reconcile the text-input overlay focus layer with `text_overlay`.
    pub(crate) fn sync_text_input_focus(&mut self) {
        let should_own = self.text_overlay.is_some();
        let has_layer = self.focus_stack.iter().any(|l| *l == FocusLayer::TextInput);
        if should_own && !has_layer {
            self.push_focus_layer(FocusLayer::TextInput);
        } else if !should_own && has_layer {
            log::info!("focus: TextInput layer removed by sync (retain)");
            self.focus_stack.retain(|l| *l != FocusLayer::TextInput);
        }
    }

    /// Push/pop `FocusLayer::CapabilityModal` based on whether the focused
    /// ProcessApp pane has pending prompts. Called every frame (both before
    /// and after the overlay render block) so the layer tracks prompt state
    /// without polling lag.
    pub(crate) fn sync_capability_modal_focus(&mut self) {
        let should_own = self.focused_pane_has_pending_prompts();
        let has_layer = self
            .focus_stack
            .iter()
            .any(|l| *l == FocusLayer::CapabilityModal);
        let is_top = matches!(self.focus_stack.last(), Some(FocusLayer::CapabilityModal));
        if should_own && !is_top {
            // Push to top (re-promoting from buried position if already in stack).
            self.focus_stack
                .retain(|l| *l != FocusLayer::CapabilityModal);
            log::info!("capability_modal: focus captured — pending prompts on focused pane");
            self.push_focus_layer(FocusLayer::CapabilityModal);
        } else if !should_own && has_layer {
            log::info!("capability_modal: focus released — prompt queue drained");
            self.focus_stack
                .retain(|l| *l != FocusLayer::CapabilityModal);
        }
    }

    /// Returns true when the focused ProcessApp pane has at least one pending prompt.
    ///
    /// `win.focused_pane` holds a `TileId`. After egui_tiles renders a bare-pane
    /// root for the first time it wraps that tile in a Container, so the stored
    /// TileId may now refer to a Container instead of a Pane. `find_pane_in_tile`
    /// descends through any Container layer to reach the actual pane.
    fn focused_pane_has_pending_prompts(&self) -> bool {
        let win = &self.windows[self.active_window];
        let focused_tile = match win.focused_pane {
            Some(t) => t,
            None => return false,
        };
        let pane_id = match Self::find_pane_in_tile(&win.tree, focused_tile) {
            Some(id) => id,
            None => return false,
        };
        match win.panes.get(&pane_id) {
            Some(crate::host::pane::Pane::App(app_pane)) => match &app_pane.runtime {
                crate::host::pane::AppRuntime::Process(proc) => !proc.pending_prompts.is_empty(),
                crate::host::pane::AppRuntime::Builtin(_) => false,
            },
            _ => false,
        }
    }

    /// Walk a tile tree node and return the first `PaneId` found within it.
    /// Handles the case where `tile_id` is a Container wrapping the actual pane
    /// (egui_tiles normalises bare-pane roots into containers on first render).
    pub(crate) fn find_pane_in_tile(
        tree: &egui_tiles::Tree<crate::spatial::tiling::PaneId>,
        tile_id: egui_tiles::TileId,
    ) -> Option<crate::spatial::tiling::PaneId> {
        match tree.tiles.get(tile_id)? {
            egui_tiles::Tile::Pane(id) => Some(*id),
            egui_tiles::Tile::Container(c) => c
                .children()
                .copied()
                .find_map(|child| Self::find_pane_in_tile(tree, child)),
        }
    }

    /// Route `DeliverNotifyAction` commands back to the originating app pane as
    /// `NotifyAction` events. Shared by the modal and the sidebar panel so both
    /// surfaces dispatch identically.
    pub(crate) fn dispatch_notify_action_cmds(
        &mut self,
        cmds: Vec<crate::app::app_trait::AppCommand>,
    ) {
        use crate::app::app_trait::AppCommand;
        for cmd in cmds {
            if let AppCommand::DeliverNotifyAction {
                pane_id,
                notify_id,
                action_label,
                value,
                response_file,
                host_action,
            } = cmd
            {
                log::info!(
                    "notify:action: pane_id={pane_id} notify_id={notify_id:?} value={value:?} host_action={host_action:?}"
                );
                // Execute host-side action synchronously before writing the response
                // file so the navigation is complete before the shell unblocks.
                if let Some(ref action) = host_action {
                    if let Some(id_str) = action.strip_prefix("pane_focus:") {
                        if let Ok(pane_id_target) = id_str.parse::<u64>() {
                            self.pane_navigate(pane_id_target);
                        } else {
                            log::warn!("notify:action: pane_focus: invalid pane_id {:?}", id_str);
                        }
                    } else {
                        log::warn!("notify:action: unknown host_action {:?}", action);
                    }
                }
                if let Some(rf) = &response_file {
                    let content = value.as_deref().unwrap_or("");
                    let tmp = format!("{rf}.tmp");
                    match std::fs::write(&tmp, content).and_then(|_| std::fs::rename(&tmp, rf)) {
                        Ok(_) => log::info!("notify:action: wrote {:?} to {:?}", content, rf),
                        Err(e) => {
                            log::warn!("notify:action: failed to write response file {:?}: {e}", rf)
                        }
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
        }
    }

    pub(crate) fn record_context_visit(&mut self, context_id: u64) {
        self.context_visit_history.retain(|&id| id != context_id);
        self.context_visit_history.insert(0, context_id);
        self.context_visit_history.truncate(50);
    }

    pub(super) fn draw_feature_effects(&self, ctx: &egui::Context) {
        use egui::{Color32, Stroke};

        // CRT effect — scanlines + green phosphor tint
        if self.features.is_enabled("crt") {
            egui::Area::new(egui::Id::new("crt_overlay"))
                .fixed_pos(egui::pos2(0.0, 0.0))
                .order(egui::Order::Foreground)
                .interactable(false)
                .show(ctx, |ui| {
                    let screen = ctx.screen_rect();
                    let painter = ui.painter();

                    // Green phosphor tint
                    painter.rect_filled(screen, 0.0, Color32::from_rgba_unmultiplied(0, 40, 0, 18));

                    // Scanlines every 3 pixels
                    let mut y = screen.top();
                    while y < screen.bottom() {
                        painter.line_segment(
                            [egui::pos2(screen.left(), y), egui::pos2(screen.right(), y)],
                            Stroke::new(0.5, Color32::from_black_alpha(38)),
                        );
                        y += 3.0;
                    }

                    ctx.request_repaint_after(std::time::Duration::from_millis(16));
                });
        }
    }
}
