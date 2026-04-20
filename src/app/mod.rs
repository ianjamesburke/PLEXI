mod dispatch;
mod sync;

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
    pub(crate) theme: TerminalTheme,
    pub(crate) colors: Colors,
    pub(crate) default_font_size: f32,
    pub(crate) ctx: egui::Context,
    pub(crate) contexts: Vec<Context>,
    pub(crate) active_context: usize,
    pub(crate) sidebar_visible: bool,
    pub(crate) show_shortcuts: bool,
    pub(crate) quitting: bool,
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
    pub(crate) host: crate::host::model::HostModel,
    pub(crate) host_services: crate::host::services::HostServices,
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
                                        overlay_replaced: None,
                                    })));
                                }
                            }
                        }
                    } else if matches!(saved_pane.kind, crate::workspace::SavedPaneKind::Agent) {
                        pane_entry = Some(Pane::Agent(Box::new(crate::pane::AgentPane {
                            id: saved_pane.id,
                            instance: Some(crate::plexi_iq::PlexiIqInstance::default()),
                            label: saved_pane
                                .name
                                .clone()
                                .unwrap_or_else(|| format!("Agent {}", saved_pane.id)),
                            transcript: Vec::new(),
                            input_buf: String::new(),
                            turn_rx: None,
                            session_id: None,
                        })));
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
                    host,
                    host_services: crate::host::services::HostServices::new(),
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
            host: crate::host::model::HostModel::new(),
            host_services: crate::host::services::HostServices::new(),
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
        self.drain_pty_events();
        let deferred_app_cmds = self.dispatch_app_key_events(ctx);
        self.sync_app_cwd();

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

                    // Find the tile for the sender, walk up to its immediate parent
                    // container, then collect sibling pane IDs (terminals only).
                    let siblings: Vec<PaneId> = {
                        use egui_tiles::Tile;
                        let ctx = &self.contexts[active];
                        let sender_tile = ctx
                            .tree
                            .tiles
                            .iter()
                            .find(|(_, t)| matches!(t, Tile::Pane(id) if *id == sender_pane_id))
                            .map(|(tid, _)| tid);
                        let sibling_tiles: Vec<PaneId> = sender_tile
                            .and_then(|st| ctx.tree.tiles.parent_of(*st))
                            .and_then(|parent_id| ctx.tree.tiles.get(parent_id))
                            .map(|tile| match tile {
                                Tile::Container(c) => c
                                    .children()
                                    .filter_map(|child_tid| {
                                        if let Some(Tile::Pane(pid)) =
                                            ctx.tree.tiles.get(*child_tid)
                                        {
                                            Some(*pid)
                                        } else {
                                            None
                                        }
                                    })
                                    .filter(|pid| *pid != sender_pane_id)
                                    .collect(),
                                _ => vec![],
                            })
                            .unwrap_or_default();
                        sibling_tiles
                    };
                    for pid in siblings {
                        if let Some(t) = self.contexts[active]
                            .panes
                            .get_mut(&pid)
                            .and_then(|p| p.as_terminal_mut())
                        {
                            t.backend.process_command(egui_term::BackendCommand::Write(
                                cd_cmd.as_bytes().to_vec(),
                            ));
                        }
                    }
                }
                AppCommand::Notify(_) => {}
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
                    } else if let Some(a) = pane.as_app() {
                        Some(a.name.clone())
                    } else {
                        pane.as_agent().map(|a| a.label.clone())
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
        for action in keys::poll_actions(ctx, app_active, keyboard_capture_active) {
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
                    self.contexts[self.active_context].zoomed_pane = None;
                    let active_panes = self.contexts[self.active_context].panes.len();
                    if active_panes > 1 {
                        self.close_focused();
                    } else if self.contexts.len() > 1 {
                        self.delete_context(self.active_context);
                    } else {
                        self.reset_active_context();
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
                    self.quitting = true;
                    self.save_workspace();
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
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
                Action::SpawnAgentPane => {
                    self.spawn_agent_pane();
                }
                Action::ToggleRunPalette => {
                    self.show_run_palette = !self.show_run_palette;
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
                        egui::Frame::new()
                            .fill(self.colors.terminal_bg)
                            .inner_margin(egui::Margin::same(8))
                            .show(&mut child_ui, |ui| {
                                if let Some(pane) = ctx.panes.get_mut(&pane_id) {
                                    if let Some(t) = pane.as_terminal_mut() {
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

        // Command palette overlay
        if self.show_command_palette {
            self.draw_command_palette(ctx);
        }

        // Run palette overlay (Cmd+R)
        if self.show_run_palette {
            self.draw_run_palette(ctx);
        }

        // Rename pane overlay
        if self.renaming_pane.is_some() {
            self.draw_rename_pane_overlay(ctx);
        }

        self.draw_feature_effects(ctx);
    }
}

impl PlexiApp {
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
