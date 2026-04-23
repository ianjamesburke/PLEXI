mod dispatch;
mod sync;

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
    RunPalette,
    RenamePane,
}

#[derive(Clone)]
pub(crate) struct PendingNotification {
    pub notify_id: String,
    pub sender_pane_id: u64,
    pub level: String,
    pub title: String,
    pub body: String,
    pub kind: crate::app_protocol::NotifyKind,
    pub options: Vec<crate::app_protocol::NotifyOption>,
    pub input_prompt: Option<String>,
    pub required: bool,
}

use crate::app_registry::AppRegistry;
use crate::config;
use crate::context::Context;
use crate::keys::{self, Action};
use crate::pane::{Pane, TerminalPane};
use crate::shell;
use crate::theme::{self, Colors};
use crate::tiling::{PaneId, PlexiBehavior};
use crate::workspace::WorkspaceFile;
use egui::{Color32, CornerRadius, Stroke, StrokeKind, Vec2};
use egui_term::{BackendSettings, PtyEvent, TerminalTheme, TerminalView};
use egui_tiles::{Tile, Tree};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

pub struct PlexiApp {
    pub(crate) pty_event_rx: mpsc::Receiver<(u64, PtyEvent)>,
    pub(crate) pty_event_tx: mpsc::Sender<(u64, PtyEvent)>,
    pub(crate) last_notify_poll: std::time::Instant,
    pub(crate) theme: TerminalTheme,
    pub(crate) colors: Colors,
    pub(crate) default_font_size: f32,
    pub(crate) ctx: egui::Context,
    pub(crate) contexts: Vec<Context>,
    pub(crate) active_context: usize,
    pub(crate) sidebar_visible: bool,
    pub(crate) show_shortcuts: bool,
    pub(crate) quitting: bool,
    pub(crate) quit_press_count: u8,
    pub(crate) quit_last_press: Option<std::time::Instant>,
    pub(crate) quit_confirm_required: bool,
    pub(crate) confirm_close: bool,
    pub(crate) pending_close: bool,
    pub(crate) renaming_context: Option<usize>,
    pub(crate) rename_buffer: String,
    pub(crate) registry: AppRegistry,
    pub(crate) show_command_palette: bool,
    pub(crate) palette_query: String,
    pub(crate) palette_selected: usize,
    pub(crate) pane_visit_history: Vec<(usize, egui_tiles::TileId)>,
    pub(crate) renaming_pane: Option<PaneId>,
    pub(crate) features: crate::features::FeatureFlags,
    /// Whether the Run palette overlay is visible (Cmd+R).
    pub(crate) show_run_palette: bool,
    /// Notifications queued from apps via ShowNotification.
    pub(crate) pending_notifications: Vec<PendingNotification>,
    /// Whether the centered notification modal is visible. Primary (and only)
    /// surface; auto-shown when a new notification arrives unless
    /// `notifications.focus_mode` is on, in which case the modal only opens
    /// on Cmd+Shift+A.
    pub(crate) show_notification_modal: bool,
    /// Index into `pending_notifications` the modal is currently showing.
    /// Cmd+] / Cmd+[ cycle this without acknowledging. Clamped to queue length.
    pub(crate) modal_queue_offset: usize,
    /// Focused option index for `kind = "choice"` notifications (0-based).
    /// Reset to 0 whenever the front of the queue changes.
    pub(crate) modal_focused_option: usize,
    /// Buffer for `kind = "input"` notifications.
    pub(crate) modal_input_buffer: String,
    /// notify_id of the notification the modal currently has state for. Used to
    /// detect a front-of-queue change and reset focus/input buffer.
    pub(crate) modal_state_notify_id: String,
    /// Cached from `[notifications]` config. See NotificationsConfig for semantics.
    pub(crate) notifications_enabled: bool,
    pub(crate) notifications_focus_mode: bool,
    /// Input-focus stack. Top layer receives keyboard input; panes see an
    /// empty event buffer while a non-`Pane` layer is on top. See the
    /// `FocusLayer` docs for the invariant.
    pub(crate) focus_stack: Vec<FocusLayer>,
    pub(crate) host: crate::host::model::HostModel,
    pub(crate) host_services: crate::host::services::HostServices,
    /// Parked background ProcessApps — kept alive when their pane is closed.
    /// Keyed by app type_id. Re-attached by B3 when the app is reopened.
    pub(crate) background_apps: HashMap<String, Box<crate::process_app::ProcessApp>>,
}

impl PlexiApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        #[cfg(target_os = "macos")]
        crate::macos_menu::customize_app_menu();

        theme::setup_fonts(&cc.egui_ctx);
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);

        let config = config::PlexiConfig::load();
        let features = crate::features::FeatureFlags::from_config(&config);
        let quit_confirm_required = config.confirm_quit
            .unwrap_or_else(|| config.beta.as_ref().and_then(|b| b.quit_confirm).unwrap_or(true));
        let confirm_close = config.confirm_close.unwrap_or(true);
        let notifications_enabled = config
            .notifications
            .as_ref()
            .and_then(|n| n.enabled)
            .unwrap_or(true);
        let notifications_focus_mode = config
            .notifications
            .as_ref()
            .and_then(|n| n.focus_mode)
            .unwrap_or(false);
        let default_font_size = config.font_size.unwrap_or(theme::FONT_SIZE);
        let theme_cfg = Self::resolve_theme_config(&config);
        let colors = Colors::from_config(&theme_cfg);
        theme::setup_style(&cc.egui_ctx, &colors);

        let (tx, rx) = mpsc::channel();

        let cwd = std::env::current_dir().unwrap_or_default();
        let registry = AppRegistry::load(&cwd);

        // Initialize the event log. Global log goes to ~/.plexi-*/events.jsonl;
        // workspace log goes to .plexi/events.jsonl if we're inside a workspace.
        {
            let global_path = crate::config::config_dir().join("events.jsonl");
            let workspace_path = crate::event_log::find_workspace_events_path(&cwd);
            crate::event_log::init_global(global_path, workspace_path);
        }

        // Try to load saved workspace
        if let Some(ws) = WorkspaceFile::load() {
            let mut contexts = Vec::new();
            for saved_ctx in ws.contexts {
                let mut panes = HashMap::new();
                for saved_pane in &saved_ctx.panes {
                    let cwd = if saved_pane.cwd.is_dir() {
                        Some(saved_pane.cwd.clone())
                    } else if saved_ctx.path.is_dir() {
                        Some(saved_ctx.path.clone())
                    } else {
                        dirs::home_dir()
                    };
                    let mut pane_entry: Option<Pane> = None;
                    if matches!(saved_pane.kind, crate::workspace::SavedPaneKind::App) {
                        let Some(app_type) = &saved_pane.app_id else {
                            continue;
                        };
                        let app_cwd = saved_pane.cwd.clone();
                        let builtin_perms = crate::app_permissions::AppPermissions::builtin();
                        match app_type.as_str() {
                            "file_browser" => {
                                let mut app =
                                    crate::file_browser::FileBrowserApp::new(app_cwd.clone());
                                if let Some(state) = &saved_pane.app_state {
                                    use crate::app_trait::App;
                                    app.restore_state(state);
                                }
                                pane_entry = Some(Pane::App(Box::new(crate::pane::AppPane {
                                    id: saved_pane.id,
                                    runtime: crate::pane::AppRuntime::Builtin(Box::new(app)),
                                    workspace_root: app_cwd,
                                    permissions: builtin_perms,
                                    manifest_id: "file_browser".to_string(),
                                    name: "File Browser".to_string(),
                                    pane_group: Some("cwd".to_string()),
                                    linked_pane_id: None,
                                    overlay_replaced: None,
                                })));
                            }
                            "quick_note" => {
                                let mut app =
                                    crate::quick_note_app::QuickNoteApp::new(app_cwd.clone());
                                if let Some(state) = &saved_pane.app_state {
                                    use crate::app_trait::App;
                                    app.restore_state(state);
                                }
                                pane_entry = Some(Pane::App(Box::new(crate::pane::AppPane {
                                    id: saved_pane.id,
                                    runtime: crate::pane::AppRuntime::Builtin(Box::new(app)),
                                    workspace_root: app_cwd,
                                    permissions: builtin_perms,
                                    manifest_id: "quick_note".to_string(),
                                    name: "Quick Note".to_string(),
                                    pane_group: None,
                                    linked_pane_id: None,
                                    overlay_replaced: None,
                                })));
                            }
                            "secrets_manager" => {
                                let mut app = crate::secrets_app::SecretsApp::new(app_cwd.clone());
                                if let Some(state) = &saved_pane.app_state {
                                    use crate::app_trait::App;
                                    app.restore_state(state);
                                }
                                pane_entry = Some(Pane::App(Box::new(crate::pane::AppPane {
                                    id: saved_pane.id,
                                    runtime: crate::pane::AppRuntime::Builtin(Box::new(app)),
                                    workspace_root: app_cwd,
                                    permissions: builtin_perms,
                                    manifest_id: "secrets_manager".to_string(),
                                    name: "Secrets Manager".to_string(),
                                    pane_group: None,
                                    linked_pane_id: None,
                                    overlay_replaced: None,
                                })));
                            }
                            other => {
                                if let Some(process) = registry.launch_process(other, &app_cwd, &[])
                                {
                                    pane_entry = Some(Pane::App(Box::new(crate::pane::AppPane {
                                        id: saved_pane.id,
                                        permissions: process.permissions.clone(),
                                        runtime: crate::pane::AppRuntime::Process(Box::new(
                                            process,
                                        )),
                                        workspace_root: app_cwd,
                                        manifest_id: other.to_string(),
                                        name: other.to_string(),
                                        pane_group: registry.group_for(other),
                                        linked_pane_id: None,
                                        overlay_replaced: None,
                                    })));
                                }
                            }
                        }
                    }

                    if pane_entry.is_none()
                        && matches!(
                            saved_pane.kind,
                            crate::workspace::SavedPaneKind::Agent
                        )
                    {
                        let agent_cwd = saved_pane.cwd.clone();
                        let pane =
                            crate::agent_pane::AgentPane::new(saved_pane.id, agent_cwd);
                        pane_entry = Some(crate::pane::Pane::Agent(Box::new(pane)));
                    }

                    if pane_entry.is_none() {
                        let settings = Self::make_backend_settings(cwd, &colors);
                        if let Some(mut pane) = TerminalPane::new(
                            saved_pane.id,
                            cc.egui_ctx.clone(),
                            tx.clone(),
                            settings,
                            default_font_size,
                        ) {
                            pane.name = saved_pane.name.clone();
                            pane_entry = Some(Pane::Terminal(Box::new(pane)));
                        }
                    }

                    if let Some(pane) = pane_entry {
                        panes.insert(saved_pane.id, pane);
                    }
                }
                if panes.is_empty() {
                    continue;
                }
                contexts.push(Context {
                    name: saved_ctx.name,
                    path: saved_ctx.path,
                    tree: saved_ctx.tree,
                    panes,
                    focused_pane: saved_ctx.focused_pane,
                    zoomed_pane: None,
                });
            }
            if !contexts.is_empty() {
                let active = ws.active_context.min(contexts.len() - 1);
                let mut host = crate::host::model::HostModel::new();
                host.seed_next_pane_id(ws.next_pane_id);
                return Self {
                    pty_event_rx: rx,
                    pty_event_tx: tx,
                    theme: theme::terminal_theme(&theme_cfg),
                    colors,
                    default_font_size,
                    ctx: cc.egui_ctx.clone(),
                    contexts,
                    active_context: active,
                    sidebar_visible: ws.sidebar_visible,
                    show_shortcuts: false,
                    quitting: false,
                    quit_press_count: 0,
                    quit_last_press: None,
                    quit_confirm_required,
                    confirm_close,
                    pending_close: false,
                    renaming_context: None,
                    rename_buffer: String::new(),
                    show_command_palette: false,
                    palette_query: String::new(),
                    palette_selected: 0,
                    pane_visit_history: Vec::new(),
                    renaming_pane: None,
                    registry,
                    features: features.clone(),
                    show_run_palette: false,
                    pending_notifications: Vec::new(),
                    show_notification_modal: false,
                    modal_queue_offset: 0,
                    modal_focused_option: 0,
                    modal_input_buffer: String::new(),
                    modal_state_notify_id: String::new(),
                    notifications_enabled,
                    notifications_focus_mode,
                    focus_stack: Vec::new(),
                    last_notify_poll: std::time::Instant::now(),
                    host,
                    host_services: crate::host::services::HostServices::new(),
                    background_apps: HashMap::new(),
                };
            }
        }

        // Default: single context with single pane
        let settings = Self::make_backend_settings(None, &colors);
        let pane = TerminalPane::new(
            0,
            cc.egui_ctx.clone(),
            tx.clone(),
            settings,
            default_font_size,
        )
        .expect("failed to create initial terminal");
        let mut panes = HashMap::new();
        panes.insert(0u64, Pane::Terminal(Box::new(pane)));

        let mut tiles = egui_tiles::Tiles::default();
        let root_tile = tiles.insert_pane(0u64);
        let tree = Tree::new("plexi", root_tile, tiles);

        let path = std::env::current_dir()
            .unwrap_or_else(|_| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")));

        Self {
            pty_event_rx: rx,
            pty_event_tx: tx,
            theme: theme::terminal_theme(&theme_cfg),
            colors,
            default_font_size,
            ctx: cc.egui_ctx.clone(),
            contexts: vec![Context {
                name: "Default".into(),
                path,
                tree,
                panes,
                focused_pane: Some(root_tile),
                zoomed_pane: None,
            }],
            active_context: 0,
            sidebar_visible: true,
            show_shortcuts: false,
            quitting: false,
            quit_press_count: 0,
            quit_last_press: None,
            quit_confirm_required,
            confirm_close,
            pending_close: false,
            renaming_context: None,
            rename_buffer: String::new(),
            show_command_palette: false,
            palette_query: String::new(),
            palette_selected: 0,
            pane_visit_history: Vec::new(),
            renaming_pane: None,
            registry: AppRegistry::load(&std::env::current_dir().unwrap_or_default()),
            features,
            show_run_palette: false,
            pending_notifications: Vec::new(),
            show_notification_modal: false,
            modal_queue_offset: 0,
            modal_focused_option: 0,
            modal_input_buffer: String::new(),
            modal_state_notify_id: String::new(),
            notifications_enabled,
            notifications_focus_mode,
            focus_stack: Vec::new(),
            last_notify_poll: std::time::Instant::now(),
            host: crate::host::model::HostModel::new(),
            host_services: crate::host::services::HostServices::new(),
            background_apps: HashMap::new(),
        }
    }

    fn resolve_theme_config(config: &config::PlexiConfig) -> config::ThemeConfig {
        let user_theme = config.theme.clone().unwrap_or_default();
        if let Some(preset_name) = &config.theme_preset {
            if let Some(preset) = theme::preset_colors(preset_name) {
                log::info!("Applying theme preset: {}", preset_name.trim());
                return theme::apply_preset(&preset, &user_theme);
            }
            log::warn!("Unknown theme preset: {preset_name}");
        }
        user_theme
    }

    pub(crate) fn make_backend_settings(
        working_directory: Option<PathBuf>,
        colors: &Colors,
    ) -> BackendSettings {
        BackendSettings {
            shell: shell::detect_shell(),
            args: vec!["-l".to_string()],
            env: shell::build_env(),
            dynamic_colors: theme::terminal_dynamic_colors(colors),
            working_directory,
        }
    }

    fn drain_notify_queue(&mut self) {
        let queue_dir = crate::config::config_dir().join("notify-queue");
        let Ok(entries) = std::fs::read_dir(&queue_dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&path) else { continue };
            let _ = std::fs::remove_file(&path);
            let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) else { continue };
            if !self.notifications_enabled {
                continue;
            }
            let level = val["level"].as_str().unwrap_or("info").to_string();
            let title = val["title"].as_str().unwrap_or("").to_string();
            let body = val["body"].as_str().unwrap_or("").to_string();
            self.pending_notifications.push(PendingNotification {
                notify_id: String::new(),
                sender_pane_id: 0,
                level,
                title,
                body,
                kind: crate::app_protocol::NotifyKind::Message,
                options: vec![],
                input_prompt: None,
                required: false,
            });
            if !self.notifications_focus_mode {
                self.show_notification_modal = true;
                self.modal_queue_offset = 0;
            }
        }
    }

    fn drain_pty_events(&mut self) {
        let mut panes_to_close: Vec<u64> = Vec::new();

        while let Ok((id, event)) = self.pty_event_rx.try_recv() {
            match &event {
                PtyEvent::Exit => {
                    for context in &mut self.contexts {
                        if let Some(pane) = context.panes.get_mut(&id) {
                            if let Some(t) = pane.as_terminal_mut() {
                                t.exited = true;
                            }
                            break;
                        }
                    }
                }
                PtyEvent::Title(title) => {
                    if let Some(cmd) = title.strip_prefix("plexi:") {
                        match cmd {
                            "close" => panes_to_close.push(id),
                            _ => log::debug!("unknown plexi command: {}", cmd),
                        }
                    }
                }
                _ => {}
            }
        }

        for pane_id in panes_to_close {
            self.close_pane_by_id(pane_id);
        }
    }
}

impl eframe::App for PlexiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.last_notify_poll.elapsed() >= std::time::Duration::from_secs(1) {
            self.last_notify_poll = std::time::Instant::now();
            self.drain_notify_queue();
        }
        self.drain_pty_events();

        // Focus stack: reconcile layer state BEFORE any input routing so
        // `input_captured_by_overlay()` answers correctly this frame.
        self.sync_notification_modal_focus();
        self.sync_confirm_close_focus();
        self.sync_command_palette_focus();
        self.sync_run_palette_focus();
        self.sync_rename_pane_focus();

        // If an overlay owns input, render it FIRST so its widgets (the
        // notification modal's TextEdit for the `input` kind, the palette's
        // search field, the rename input) can read keystrokes before we
        // drain. Then drain the keyboard buffer so downstream readers —
        // focused app (`dispatch_app_key_events`), terminal backends,
        // `keys::poll_actions` — see only the global allowlist (Cmd+Q,
        // Cmd+W, Cmd+Shift+A, Cmd+]/Cmd+[).
        let mut early_modal_cmds: Vec<crate::app_trait::AppCommand> = Vec::new();
        if self.input_captured_by_overlay() {
            match self.focus_stack.last() {
                Some(FocusLayer::NotificationModal) => {
                    early_modal_cmds = self.draw_notification_modal(ctx);
                }
                Some(FocusLayer::ConfirmClose) => {
                    self.draw_confirm_close(ctx);
                }
                Some(FocusLayer::CommandPalette) => {
                    self.draw_command_palette(ctx);
                }
                Some(FocusLayer::RunPalette) => {
                    self.draw_run_palette(ctx);
                }
                Some(FocusLayer::RenamePane) => {
                    self.draw_rename_pane_overlay(ctx);
                }
                None => {}
            }
            self.drain_captured_keyboard_input(ctx);
            // The overlay may have self-closed (notification queue drained,
            // confirm-close confirmed/cancelled, palette picked an entry,
            // rename committed). Re-sync so the layer is accurate for the
            // rest of this frame.
            self.sync_notification_modal_focus();
            self.sync_confirm_close_focus();
            self.sync_command_palette_focus();
            self.sync_run_palette_focus();
            self.sync_rename_pane_focus();
        }

        // Apps only receive key input if nothing is capturing above them.
        let deferred_app_cmds = if self.input_captured_by_overlay() {
            Vec::new()
        } else {
            self.dispatch_app_key_events(ctx)
        };
        self.sync_app_cwd();

        // Dispatch any DeliverNotifyAction commands the early modal render
        // produced. Routes back to the originating pane as NotifyAction events.
        self.dispatch_notify_action_cmds(early_modal_cmds);

        // Handle deferred app commands returned from dispatch_app_key_events.
        for cmd in deferred_app_cmds {
            use crate::app_trait::AppCommand;
            match cmd {
                AppCommand::SpawnApp {
                    type_id,
                    layout,
                    args,
                } => {
                    // Capture requesting pane before launch changes focused_pane.
                    let active = self.active_context;
                    let requesting_pane_id = self.contexts[active]
                        .focused_pane
                        .and_then(|tile| self.contexts[active].tree.tiles.get(tile))
                        .and_then(|tile| {
                            if let egui_tiles::Tile::Pane(pid) = tile {
                                Some(*pid)
                            } else {
                                None
                            }
                        });

                    let new_pane_id = self.host.next_pane_id();
                    self.launch_app_by_id_with_layout(&type_id, layout, &args);

                    // Confirm back to the requesting app.
                    if let Some(req_pane_id) = requesting_pane_id {
                        let active = self.active_context;
                        if let Some(pane) = self.contexts[active].panes.get_mut(&req_pane_id) {
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
                AppCommand::CdRequest { cwd, sender_pane_id } => {
                    let active = self.active_context;
                    let escaped = cwd.replace('\'', "'\\''");
                    let cd_cmd = format!("cd '{}'\n", escaped);
                    let linked_id = self.contexts[active]
                        .panes
                        .get(&sender_pane_id)
                        .and_then(|p| p.as_app())
                        .and_then(|a| a.linked_pane_id);
                    if let Some(tid) = linked_id {
                        if let Some(t) = self.contexts[active]
                            .panes
                            .get_mut(&tid)
                            .and_then(|p| p.as_terminal_mut())
                        {
                            t.backend.process_command(egui_term::BackendCommand::Write(
                                cd_cmd.as_bytes().to_vec(),
                            ));
                        }
                    }
                }
                AppCommand::Notify(_) => {}
                AppCommand::ShowNotification {
                    notify_id,
                    sender_pane_id,
                    level,
                    title,
                    body,
                    kind,
                    options,
                    input_prompt,
                    required,
                } => {
                    if !self.notifications_enabled {
                        // Silently drop — master switch off.
                        continue;
                    }
                    self.pending_notifications.push(PendingNotification {
                        notify_id,
                        sender_pane_id,
                        level,
                        title,
                        body,
                        kind,
                        options,
                        input_prompt,
                        required,
                    });
                    if !self.notifications_focus_mode {
                        self.show_notification_modal = true;
                        self.modal_queue_offset = 0;
                    }
                }
                AppCommand::DeliverNotifyAction { pane_id, notify_id, action_label, value } => {
                    let active = self.active_context;
                    if let Some(pane) = self.contexts[active].panes.get_mut(&pane_id) {
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
                AppCommand::DeliverPipeMessage { sender_pane_id, pipe_id, payload } => {
                    let active = self.active_context;
                    let pane_ids: Vec<_> = self.contexts[active].panes.keys().copied().collect();
                    for pid in pane_ids {
                        if pid == sender_pane_id {
                            continue; // don't echo back to sender
                        }
                        let is_reader = self.contexts[active]
                            .panes
                            .get(&pid)
                            .and_then(|p| p.as_app())
                            .map(|a| match &a.runtime {
                                crate::pane::AppRuntime::Process(pa) => {
                                    pa.pipe_registry.lock().unwrap().has_reader(&pipe_id)
                                }
                                crate::pane::AppRuntime::Builtin(_) => false,
                            })
                            .unwrap_or(false);
                        if is_reader {
                            if let Some(pane) = self.contexts[active].panes.get_mut(&pid) {
                                if let Some(app) = pane.as_app_mut() {
                                    app.runtime.queue_outbound_event(
                                        crate::app_protocol::PlexiEvent::PipeMessage {
                                            pipe_id: pipe_id.clone(),
                                            payload: payload.clone(),
                                        },
                                    );
                                }
                            }
                        }
                    }
                }
                AppCommand::DeliverRunUpdate { originator_type_id, event } => {
                    let active = self.active_context;
                    let pane_ids: Vec<_> = self.contexts[active].panes.keys().copied().collect();
                    let mut delivered = false;
                    for pid in pane_ids {
                        let matches = self.contexts[active]
                            .panes
                            .get(&pid)
                            .and_then(|p| p.as_app())
                            .map(|a| match &a.runtime {
                                crate::pane::AppRuntime::Process(pa) => {
                                    pa.type_id == originator_type_id
                                }
                                crate::pane::AppRuntime::Builtin(_) => false,
                            })
                            .unwrap_or(false);
                        if matches {
                            if let Some(pane) = self.contexts[active].panes.get_mut(&pid) {
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

        // Check if the focused app wants to close itself (e.g. after saving).
        {
            let ctx_ref = &self.contexts[self.active_context];
            let should_close = ctx_ref
                .focused_pane
                .and_then(|tile| ctx_ref.tree.tiles.get(tile))
                .and_then(|tile| {
                    if let egui_tiles::Tile::Pane(pid) = tile {
                        Some(*pid)
                    } else {
                        None
                    }
                })
                .and_then(|pid| ctx_ref.panes.get(&pid))
                .map(|pane| {
                    pane.as_app()
                        .map(|a| a.runtime.wants_close())
                        .unwrap_or(false)
                })
                .unwrap_or(false);
            if should_close {
                self.close_focused_app();
            }
        }

        // Update window title to reflect active pane — readable by AppleScript / OS scripts
        {
            let context = &self.contexts[self.active_context];
            let pane_name = context
                .focused_pane
                .and_then(|tile_id| context.tree.tiles.get(tile_id))
                .and_then(|tile| {
                    if let egui_tiles::Tile::Pane(pane_id) = tile {
                        context.panes.get(pane_id)
                    } else {
                        None
                    }
                })
                .and_then(|pane| {
                    if let Some(t) = pane.as_terminal() {
                        t.name.clone()
                    } else {
                        pane.as_app().map(|a| a.name.clone())
                    }
                });
            let title = match pane_name {
                Some(name) => format!("{} — {}", context.name, name),
                None => context.name.clone(),
            };
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        }

        // Determine if the focused pane has an active app surface, and whether
        // that app has declared keyboard_capture mode.
        let (app_active, keyboard_capture_active) = {
            let context = &self.contexts[self.active_context];
            let focused_pane = context.focused_pane.and_then(|tile_id| {
                if let Some(egui_tiles::Tile::Pane(pane_id)) = context.tree.tiles.get(tile_id) {
                    context.panes.get(pane_id)
                } else {
                    None
                }
            });
            let active = focused_pane
                .map(|pane| pane.as_app().is_some())
                .unwrap_or(false);
            let capture = if active {
                focused_pane
                    .and_then(|p| p.as_app())
                    .map(|a| a.runtime.keyboard_capture())
                    .unwrap_or(false)
            } else {
                false
            };
            (active, capture)
        };

        // Handle keyboard shortcuts
        let modal_open = self.show_notification_modal;
        for action in keys::poll_actions(ctx, app_active, keyboard_capture_active, modal_open) {
            match action {
                Action::SplitHorizontal => {
                    self.contexts[self.active_context].zoomed_pane = None;
                    self.split_focused(false);
                }
                Action::SplitVertical => {
                    self.contexts[self.active_context].zoomed_pane = None;
                    self.split_focused(true);
                }
                Action::Navigate(dir) => {
                    let was_zoomed = self.contexts[self.active_context].zoomed_pane.is_some();
                    self.navigate(dir);
                    if was_zoomed {
                        self.contexts[self.active_context].zoomed_pane =
                            self.contexts[self.active_context].focused_pane;
                    }
                }
                Action::ClosePane => {
                    if self.confirm_close {
                        self.pending_close = true;
                    } else {
                        self.execute_close_pane();
                    }
                }
                Action::NewTab => self.new_tab(),
                Action::ToggleZoom => {
                    let ctx = &mut self.contexts[self.active_context];
                    if let Some(focused) = ctx.focused_pane {
                        if ctx.zoomed_pane == Some(focused) {
                            ctx.zoomed_pane = None;
                        } else {
                            ctx.zoomed_pane = Some(focused);
                        }
                    }
                }
                Action::Quit => {
                    if !self.quit_confirm_required {
                        self.quitting = true;
                        self.save_workspace();
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    } else {
                        let now = std::time::Instant::now();
                        let elapsed = self
                            .quit_last_press
                            .map(|t| now.duration_since(t))
                            .unwrap_or(std::time::Duration::MAX);
                        if elapsed > std::time::Duration::from_millis(1500) {
                            self.quit_press_count = 0;
                        }
                        self.quit_press_count += 1;
                        self.quit_last_press = Some(now);
                        if self.quit_press_count >= 3 {
                            self.quit_press_count = 0;
                            self.quit_last_press = None;
                            self.quitting = true;
                            self.save_workspace();
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    }
                }
                Action::ToggleSidebar => self.sidebar_visible = !self.sidebar_visible,
                Action::ToggleShortcuts => self.show_shortcuts = !self.show_shortcuts,
                Action::ToggleCommandPalette => {
                    self.show_command_palette = !self.show_command_palette;
                    if self.show_command_palette {
                        self.palette_query.clear();
                        self.palette_selected = 0;
                    }
                }
                Action::RenamePane => {
                    let active_ctx = &self.contexts[self.active_context];
                    if let Some(focused_tile) = active_ctx.focused_pane {
                        if let Some(Tile::Pane(pane_id)) = active_ctx.tree.tiles.get(focused_tile) {
                            let pane_id = *pane_id;
                            self.rename_buffer = active_ctx
                                .panes
                                .get(&pane_id)
                                .and_then(|p| p.as_terminal())
                                .and_then(|t| t.name.clone())
                                .unwrap_or_default();
                            self.renaming_pane = Some(pane_id);
                        }
                    }
                }
                Action::NewContext => {
                    self.new_context();
                }
                Action::SwitchContext(n) => {
                    if n < self.contexts.len() {
                        self.active_context = n;
                    }
                }
                Action::NextTab => {
                    self.cycle_tab(true);
                }
                Action::PrevTab => {
                    self.cycle_tab(false);
                }
                Action::IncreasePaneFontSize => {
                    self.adjust_focused_pane_font_size(1.0);
                }
                Action::DecreasePaneFontSize => {
                    self.adjust_focused_pane_font_size(-1.0);
                }
                Action::ScrollUp => {
                    self.scroll_focused_pane(3);
                }
                Action::ScrollDown => {
                    self.scroll_focused_pane(-3);
                }
                Action::CloseApp => {
                    self.close_focused_app();
                }
                Action::ToggleAppFocus => {
                    // Tab navigates between the app pane and the linked
                    // terminal pane below (they're separate tiles now).
                    self.navigate(crate::keys::Direction::Down);
                }
                Action::OpenFileBrowser => {
                    self.open_file_browser();
                }
                Action::OpenQuickNote => {
                    self.open_quick_note();
                }
                Action::OpenConfig => {
                    self.open_config_editor();
                }
                Action::OpenSecretsManager => {
                    self.open_secrets_manager();
                }
                Action::ToggleRunPalette => {
                    self.show_run_palette = !self.show_run_palette;
                }
                Action::ToggleNotificationModal => {
                    if self.show_notification_modal {
                        self.show_notification_modal = false;
                    } else if !self.pending_notifications.is_empty() {
                        self.show_notification_modal = true;
                        self.modal_queue_offset = 0;
                    }
                    // If the queue is empty and the modal is closed, this is a
                    // no-op — there's nothing to review.
                }
                Action::NotificationCycleNext => {
                    if self.show_notification_modal
                        && self.modal_queue_offset + 1 < self.pending_notifications.len()
                    {
                        self.modal_queue_offset += 1;
                    }
                }
                Action::NotificationCyclePrev => {
                    if self.show_notification_modal && self.modal_queue_offset > 0 {
                        self.modal_queue_offset -= 1;
                    }
                }
                Action::OpenAgentPane => {
                    self.open_agent_pane();
                }
            }
        }

        // Handle window close request (X button / system shutdown) — always quit
        if ctx.input(|i| i.viewport().close_requested()) && !self.quitting {
            self.save_workspace();
        }

        // All panes across all contexts exited
        if self.contexts.iter().all(|c| c.panes.is_empty()) {
            self.save_workspace();
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        // Toolbar
        egui::TopBottomPanel::top("toolbar")
            .exact_height(28.0)
            .frame(
                egui::Frame::new()
                    .fill(self.colors.bg_toolbar)
                    .inner_margin(egui::Margin {
                        left: 8,
                        right: 8,
                        top: 4,
                        bottom: 4,
                    }),
            )
            .show(ctx, |ui| {
                self.draw_toolbar(ui);
            });

        // Separator line under toolbar
        egui::TopBottomPanel::top("toolbar_sep")
            .exact_height(1.0)
            .frame(egui::Frame::new().fill(self.colors.border))
            .show(ctx, |_ui| {});

        // Sidebar
        if self.sidebar_visible {
            egui::SidePanel::left("sidebar")
                .exact_width(220.0)
                .frame(
                    egui::Frame::new()
                        .fill(self.colors.bg_sidebar)
                        .inner_margin(egui::Margin::same(0)),
                )
                .show(ctx, |ui| {
                    self.draw_sidebar(ui);
                });
        }

        // Central panel — terminal tiles
        egui::CentralPanel::default()
            .frame(egui::Frame {
                fill: self.colors.bg_darkest,
                inner_margin: egui::Margin::same(4),
                outer_margin: egui::Margin::ZERO,
                ..Default::default()
            })
            .show(ctx, |ui| {
                let ctx = &mut self.contexts[self.active_context];

                // Resolve focused_pane if simplifier moved the tile
                if let Some(fp) = ctx.focused_pane {
                    if !matches!(ctx.tree.tiles.get(fp), Some(Tile::Pane(_))) {
                        ctx.focused_pane = ctx.find_first_pane_in(fp);
                    }
                }

                // Validate zoomed pane still exists
                if let Some(zp) = ctx.zoomed_pane {
                    if !matches!(ctx.tree.tiles.get(zp), Some(Tile::Pane(_))) {
                        ctx.zoomed_pane = None;
                    }
                }

                let zoomed_pane = ctx.zoomed_pane;
                let tab_info = ctx.compute_tab_info();
                let pane_names: HashMap<PaneId, String> = ctx
                    .panes
                    .iter()
                    .filter_map(|(&id, p)| p.as_terminal()?.name.as_ref().map(|n| (id, n.clone())))
                    .collect();
                let suppress_focus = self.renaming_context.is_some()
                    || self.show_command_palette
                    || self.renaming_pane.is_some();

                #[cfg(target_os = "macos")]
                let drag_cursor_pos: Option<egui::Pos2> = {
                    let has_drag = ui.input(|i| {
                        !i.raw.hovered_files.is_empty() || !i.raw.dropped_files.is_empty()
                    });
                    if has_drag {
                        ui.ctx()
                            .request_repaint_after(std::time::Duration::from_millis(16)); // continuous repaints while dragging
                        use objc2_app_kit::NSApplication;
                        use objc2_foundation::MainThreadMarker;
                        MainThreadMarker::new()
                            .and_then(|mtm| {
                                let app = NSApplication::sharedApplication(mtm);
                                app.keyWindow().or_else(|| unsafe { app.mainWindow() })
                            })
                            .map(|w| {
                                let p = unsafe { w.mouseLocationOutsideOfEventStream() };
                                let content_height = ui.ctx().screen_rect().height();
                                egui::pos2(p.x as f32, content_height - p.y as f32)
                            })
                    } else {
                        None
                    }
                };
                #[cfg(not(target_os = "macos"))]
                let drag_cursor_pos: Option<egui::Pos2> = None;

                let mut behavior = PlexiBehavior {
                    panes: &mut ctx.panes,
                    focused_tile: if suppress_focus {
                        None
                    } else {
                        ctx.focused_pane
                    },
                    theme: self.theme.clone(),
                    new_focused: None,
                    close_exited: None,
                    tab_info,
                    zoomed_pane,
                    colors: self.colors,
                    pane_names,
                    drag_cursor_pos,
                };
                ctx.tree.ui(&mut behavior, ui);

                if let Some(new) = behavior.new_focused {
                    ctx.focused_pane = Some(new);
                }

                let should_close_exited = behavior.close_exited.is_some();

                // Draw zoom overlay if a pane is zoomed
                if let Some(zoomed_tile) = zoomed_pane {
                    if let Some(Tile::Pane(pane_id)) = ctx.tree.tiles.get(zoomed_tile) {
                        let pane_id = *pane_id;
                        let panel_rect = ui.max_rect();
                        let zoomed_tab_info = behavior.tab_info.get(&zoomed_tile).copied();

                        // Drop behavior to release the mutable borrow on ctx.panes
                        drop(behavior);

                        // Semi-transparent scrim over the entire central panel
                        ui.painter()
                            .rect_filled(panel_rect, 0.0, Color32::from_black_alpha(75));

                        // Inset rect for the zoomed pane
                        let inset = 5.0;
                        let zoom_rect = panel_rect.shrink(inset);

                        // Thicker accent border (2px)
                        ui.painter().rect_stroke(
                            zoom_rect,
                            CornerRadius::same(4),
                            Stroke::new(2.0, self.colors.accent),
                            StrokeKind::Inside,
                        );

                        // Render zoomed terminal in the inset rect
                        let inner_rect = zoom_rect.shrink(2.0); // inside the border
                        let mut child_ui =
                            ui.new_child(egui::UiBuilder::new().max_rect(inner_rect));
                        // Files dropped onto a zoomed pane must go to the
                        // zoomed terminal, NOT a background tile. The
                        // per-tile drop path in `tiling.rs` is gated off
                        // while zoomed; we handle the drop here instead.
                        let dropped_to_zoom = child_ui.input(|i| {
                            !i.raw.dropped_files.is_empty()
                                && child_ui.rect_contains_pointer(inner_rect)
                        });
                        egui::Frame::new()
                            .fill(self.colors.terminal_bg)
                            .inner_margin(egui::Margin::same(8))
                            .show(&mut child_ui, |ui| {
                                if let Some(pane) = ctx.panes.get_mut(&pane_id) {
                                    if let Some(t) = pane.as_terminal_mut() {
                                        if dropped_to_zoom {
                                            crate::tiling::write_dropped_paths_to_terminal(ui, t);
                                        }
                                        if t.exited {
                                            let rect = ui.max_rect();
                                            ui.painter().rect_filled(
                                                rect,
                                                0.0,
                                                self.colors.terminal_bg,
                                            );
                                            ui.allocate_new_ui(
                                                egui::UiBuilder::new().max_rect(rect),
                                                |ui| {
                                                    ui.centered_and_justified(|ui| {
                                                        ui.colored_label(
                                                            self.colors.text_dim,
                                                            "[process exited]",
                                                        );
                                                    });
                                                },
                                            );
                                        } else {
                                            // Reserve space for tab dots if in a tab group
                                            if zoomed_tab_info.is_some() {
                                                ui.add_space(
                                                    crate::tiling::TAB_DOT_RESERVED_HEIGHT,
                                                );
                                            }
                                            let font_size = t.font_size;
                                            let terminal = TerminalView::new(ui, &mut t.backend)
                                                .set_focus(true)
                                                .set_theme(self.theme.clone())
                                                .set_font(theme::terminal_font(font_size))
                                                .set_size(Vec2::new(
                                                    ui.available_width(),
                                                    ui.available_height(),
                                                ));
                                            ui.add(terminal);
                                        }
                                    } else if let Some(a) = pane.as_app_mut() {
                                        let app_ctx = crate::app_trait::AppRenderContext {
                                            colors: &self.colors,
                                            is_focused: true,
                                        };
                                        a.runtime.ui(ui, &app_ctx);
                                    } else if let Some(agent) = pane.as_agent_mut() {
                                        if crate::agent_pane::render_and_drain(ui, agent, &self.colors) {
                                            ui.ctx().request_repaint();
                                        }
                                    }
                                }

                                // Draw tab indicator dots (same style as tiling.rs)
                                if let Some((active_idx, count)) = zoomed_tab_info {
                                    let rect = ui.max_rect();
                                    crate::tiling::paint_tab_dots(
                                        ui.painter(),
                                        rect.left(),
                                        rect.top() + 2.0 + 4.0, // 4.0 = dot radius
                                        active_idx,
                                        count,
                                        self.colors.accent,
                                        self.colors.bg_active,
                                    );
                                }
                            });
                    } else {
                        drop(behavior);
                    }
                } else {
                    drop(behavior);
                }

                if should_close_exited {
                    self.close_focused();
                }
            });

        // Shortcuts overlay
        if self.show_shortcuts {
            self.draw_shortcuts_overlay(ctx);
        }

        // Command palette, run palette, rename-pane overlay, notification
        // modal, and confirm-close are all drawn by the early input-capture
        // path at the top of `update()` — they own a `FocusLayer` and render
        // their own keystrokes before the drain. Drawing again here would
        // double-dispatch Enter/Escape after keys have been drained.
        if self.pending_notifications.is_empty() {
            self.show_notification_modal = false;
        }

        // Quit confirmation overlay
        if self.quit_confirm_required && self.quit_press_count > 0 {
            // Reset on Escape or timeout
            let timed_out = self
                .quit_last_press
                .map(|t| t.elapsed() > std::time::Duration::from_millis(1500))
                .unwrap_or(false);
            if timed_out || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.quit_press_count = 0;
                self.quit_last_press = None;
            } else {
                self.draw_quit_confirm_overlay(ctx);
                // Keep repainting so the timeout dismissal fires promptly
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
        }

        self.draw_feature_effects(ctx);
    }
}

impl PlexiApp {
    /// True when a modal overlay owns keyboard input. Used by `update()` to
    /// drain remaining key events after the overlay has rendered so panes see
    /// an empty input buffer this frame.
    pub(crate) fn input_captured_by_overlay(&self) -> bool {
        matches!(
            self.focus_stack.last(),
            Some(FocusLayer::NotificationModal)
                | Some(FocusLayer::ConfirmClose)
                | Some(FocusLayer::CommandPalette)
                | Some(FocusLayer::RunPalette)
                | Some(FocusLayer::RenamePane)
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
            self.pop_focus_layer(&FocusLayer::ConfirmClose);
        }
    }

    pub(crate) fn sync_notification_modal_focus(&mut self) {
        let should_own = self.show_notification_modal
            && !self.pending_notifications.is_empty();
        let has_layer = self
            .focus_stack
            .iter()
            .any(|l| *l == FocusLayer::NotificationModal);
        if should_own && !has_layer {
            self.push_focus_layer(FocusLayer::NotificationModal);
        } else if !should_own && has_layer {
            self.pop_focus_layer(&FocusLayer::NotificationModal);
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
            self.pop_focus_layer(&FocusLayer::CommandPalette);
        }
    }

    /// Reconcile the run-palette focus layer with `show_run_palette`.
    pub(crate) fn sync_run_palette_focus(&mut self) {
        let should_own = self.show_run_palette;
        let has_layer = self
            .focus_stack
            .iter()
            .any(|l| *l == FocusLayer::RunPalette);
        if should_own && !has_layer {
            self.push_focus_layer(FocusLayer::RunPalette);
        } else if !should_own && has_layer {
            self.pop_focus_layer(&FocusLayer::RunPalette);
        }
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
            self.pop_focus_layer(&FocusLayer::RenamePane);
        }
    }

    /// Drain keyboard events from `ctx.input` so downstream widgets (panes,
    /// terminal backends, `keys::poll_actions`) see only the global allowlist.
    /// Called after the owning overlay has read what it needs. The allowlist
    /// lets a small set of keybinds (Quit, Close, hide-modal, queue-cycle)
    /// remain live even while an overlay owns focus — users always need a way
    /// to quit or dismiss the overlay.
    pub(crate) fn drain_captured_keyboard_input(&self, ctx: &egui::Context) {
        ctx.input_mut(|i| {
            i.events.retain(|e| match e {
                egui::Event::Key { key, modifiers, .. } => {
                    let cmd = modifiers.command;
                    let shift = modifiers.shift;
                    let alt = modifiers.alt;
                    let ctrl_only = modifiers.ctrl;
                    // Only pass modifier-bearing keys; drop bare key presses.
                    if !cmd || alt || ctrl_only {
                        return false;
                    }
                    // Cmd+Q (quit), Cmd+W (close pane / hide-modal fallback).
                    if !shift && matches!(key, egui::Key::Q | egui::Key::W) {
                        return true;
                    }
                    // Cmd+Shift+A — toggle notification modal (global escape
                    // hatch, survives even required notifs).
                    if shift && matches!(key, egui::Key::A) {
                        return true;
                    }
                    // Cmd+] / Cmd+[ — cycle the notification queue without
                    // acknowledging. Only meaningful while the modal is open.
                    if !shift
                        && matches!(key, egui::Key::CloseBracket | egui::Key::OpenBracket)
                    {
                        return true;
                    }
                    false
                }
                egui::Event::Text(_) => false,
                _ => true,
            });
        });
    }

    /// Route `DeliverNotifyAction` commands back to the originating app pane as
    /// `NotifyAction` events. Shared by the modal and the sidebar panel so both
    /// surfaces dispatch identically.
    pub(crate) fn dispatch_notify_action_cmds(&mut self, cmds: Vec<crate::app_trait::AppCommand>) {
        use crate::app_trait::AppCommand;
        for cmd in cmds {
            if let AppCommand::DeliverNotifyAction { pane_id, notify_id, action_label, value } = cmd {
                let active = self.active_context;
                if let Some(pane) = self.contexts[active].panes.get_mut(&pane_id) {
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

    pub(crate) fn record_pane_visit(&mut self, ctx_idx: usize, tile_id: egui_tiles::TileId) {
        self.pane_visit_history
            .retain(|&(c, t)| !(c == ctx_idx && t == tile_id));
        self.pane_visit_history.insert(0, (ctx_idx, tile_id));
        self.pane_visit_history.truncate(100);
    }

    fn draw_feature_effects(&self, ctx: &egui::Context) {
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

        // Pulse — focused pane border gently breathes
        if self.features.is_enabled("pulse") {
            let time = ctx.input(|i| i.time);
            let pulse_alpha = ((time * 2.0).sin() * 0.5 + 0.5) as f32;
            let pulse_color = Color32::from_rgba_unmultiplied(
                self.colors.accent.r(),
                self.colors.accent.g(),
                self.colors.accent.b(),
                (pulse_alpha * 80.0 + 30.0) as u8,
            );

            egui::Area::new(egui::Id::new("pulse_overlay"))
                .fixed_pos(egui::pos2(0.0, 0.0))
                .order(egui::Order::Foreground)
                .interactable(false)
                .show(ctx, |ui| {
                    let screen = ctx.screen_rect();
                    let thickness = 2.0 + pulse_alpha * 1.5;
                    ui.painter().rect_stroke(
                        screen.shrink(1.0),
                        0.0,
                        Stroke::new(thickness, pulse_color),
                        egui::StrokeKind::Inside,
                    );
                });
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
    }

    pub(crate) fn abbreviate_home_path(path: &Path) -> String {
        let raw = path.display().to_string();
        if let Some(home) = dirs::home_dir() {
            let home_display = home.display().to_string();
            if raw == home_display {
                "~".to_string()
            } else if let Some(rest) = raw.strip_prefix(&(home_display + "/")) {
                format!("~/{rest}")
            } else {
                raw
            }
        } else {
            raw
        }
    }
}
