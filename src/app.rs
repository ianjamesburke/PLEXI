use crate::app_registry::AppRegistry;
use crate::config;
use crate::context::Context;
use crate::keys::{self, Action};
use crate::media_cache::MediaCache;
use crate::pane::TerminalPane;
use crate::shell;
use crate::theme::{self, Colors};
use crate::tiling::{PaneId, PlexiBehavior};
use crate::workspace::WorkspaceFile;
use egui::{Color32, CornerRadius, Stroke, StrokeKind, Vec2};
use egui_term::{BackendSettings, PtyEvent, TerminalTheme, TerminalView};
use egui_tiles::{Tile, Tree};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

pub struct PlexiApp {
    pub(crate) pty_event_rx: mpsc::Receiver<(u64, PtyEvent)>,
    pub(crate) pty_event_tx: mpsc::Sender<(u64, PtyEvent)>,
    pub(crate) theme: TerminalTheme,
    pub(crate) colors: Colors,
    pub(crate) default_font_size: f32,
    pub(crate) next_pane_id: u64,
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
    pub(crate) show_notification_palette: bool,
    pub(crate) notification_palette_selected: usize,
    pub(crate) pane_visit_history: Vec<(usize, egui_tiles::TileId)>,
    pub(crate) app_visit_history: Vec<String>,
    pub(crate) renaming_pane: Option<PaneId>,
    pub(crate) features: crate::features::FeatureFlags,
    /// Shared image + video thumbnail cache. Owned here so all apps on all
    /// panes share the same decoded textures and ffmpeg-generated thumbnails.
    pub(crate) media_cache: RefCell<MediaCache>,
    /// In-memory registry of every active parent → child pane spawn relationship.
    /// Drives cascade/orphan semantics when a pane closes.
    pub(crate) spawn_relationships: crate::app_protocol::SpawnRelationships,
}

impl PlexiApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        #[cfg(target_os = "macos")]
        crate::macos_menu::customize_app_menu();
        #[cfg(target_os = "macos")]
        crate::finder_service::register();

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
                    let settings = Self::make_backend_settings(cwd, &colors);
                    if let Some(mut pane) =
                        TerminalPane::new(saved_pane.id, cc.egui_ctx.clone(), tx.clone(), settings, default_font_size)
                    {
                        pane.name = saved_pane.name.clone();
                        // Restore active app if one was saved.
                        if let Some(app_type) = &saved_pane.active_app_type {
                            let cwd = saved_pane.cwd.clone();
                            let builtin_perms = crate::app_permissions::AppPermissions::builtin();

                            let (app, perms): (Option<Box<dyn crate::app_trait::App>>, _) = match app_type.as_str() {
                                "file_browser" => {
                                    let mut fb = crate::file_browser::FileBrowserApp::new(cwd.clone());
                                    if let Some(state) = &saved_pane.active_app_state {
                                        use crate::app_trait::App;
                                        fb.restore_state(state);
                                    }
                                    (Some(Box::new(fb)), builtin_perms)
                                }
                                "quick_note" => {
                                    let mut note = crate::quick_note_app::QuickNoteApp::new(cwd.clone());
                                    if let Some(state) = &saved_pane.active_app_state {
                                        use crate::app_trait::App;
                                        note.restore_state(state);
                                    }
                                    (Some(Box::new(note)), builtin_perms)
                                }
                                "audio_player" => {
                                    let mut player = crate::audio_app::AudioApp::new(cwd.clone());
                                    if let Some(state) = &saved_pane.active_app_state {
                                        use crate::app_trait::App;
                                        player.restore_state(state);
                                    }
                                    (Some(Box::new(player)), builtin_perms)
                                }
                                "secrets_manager" => {
                                    let mut secrets = crate::secrets_app::SecretsApp::new(cwd.clone());
                                    if let Some(state) = &saved_pane.active_app_state {
                                        use crate::app_trait::App;
                                        secrets.restore_state(state);
                                    }
                                    (Some(Box::new(secrets)), builtin_perms)
                                }
                                // Third-party apps: launch via registry using type_id.
                                other => {
                                    let app = registry.launch(other, &cwd, &[]);
                                    let perms = registry.permissions_for(other)
                                        .unwrap_or(builtin_perms);
                                    (app, perms)
                                }
                            };
                            if let Some(app) = app {
                                // Restore in the same mode the workspace was
                                // saved in. If the saved pane had a linked
                                // terminal sibling tile, restore as legacy
                                // `AppActive`. Otherwise restore as the
                                // in-pane `AppWithCompanion` mode so the same
                                // terminal hosts the embedded companion view.
                                if saved_pane.linked_terminal_pane.is_some() {
                                    pane.open_app(app, perms, cwd);
                                    pane.linked_terminal_pane =
                                        saved_pane.linked_terminal_pane;
                                } else {
                                    pane.open_app_with_companion(app, perms, cwd);
                                }
                            }
                        }
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
                return Self {
                    pty_event_rx: rx,
                    pty_event_tx: tx,
                    theme: theme::terminal_theme(&theme_cfg),
                    colors,
                    default_font_size,
                    next_pane_id: ws.next_pane_id,
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
                    show_notification_palette: false,
                    notification_palette_selected: 0,
                    pane_visit_history: Vec::new(),
                    app_visit_history: config::load_app_mru(),
                    renaming_pane: None,
                    registry,
                    features: features.clone(),
                    media_cache: RefCell::new(MediaCache::new()),
                    spawn_relationships: crate::app_protocol::SpawnRelationships::new(),
                };
            }
        }

        // Default: single context with single pane
        let settings = Self::make_backend_settings(None, &colors);
        let pane = TerminalPane::new(0, cc.egui_ctx.clone(), tx.clone(), settings, default_font_size)
            .expect("failed to create initial terminal");
        let mut panes = HashMap::new();
        panes.insert(0u64, pane);

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
            next_pane_id: 1,
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
            show_notification_palette: false,
            notification_palette_selected: 0,
            pane_visit_history: Vec::new(),
            app_visit_history: config::load_app_mru(),
            renaming_pane: None,
            registry: AppRegistry::load(&std::env::current_dir().unwrap_or_default()),
            features,
            media_cache: RefCell::new(MediaCache::new()),
            spawn_relationships: crate::app_protocol::SpawnRelationships::new(),
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

    pub(crate) fn make_backend_settings(working_directory: Option<PathBuf>, colors: &Colors) -> BackendSettings {
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
                            pane.exited = true;
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

    /// Feed keyboard input to the focused pane's active app and dispatch any
    /// resulting AppCommands to the appropriate terminal pane.
    ///
    /// Routing rules for AppCommands:
    /// * `AppActive` (legacy split): commands go to the linked sibling pane.
    /// * `AppWithCompanion`: commands go to the same pane (the embedded
    ///   terminal IS the original terminal that was here before the app).
    fn dispatch_app_key_events(&mut self, ctx: &egui::Context) {
        let active = self.active_context;
        let Some(focused_tile) = self.contexts[active].focused_pane else {
            return;
        };
        let Some(egui_tiles::Tile::Pane(pane_id)) =
            self.contexts[active].tree.tiles.get(focused_tile)
        else {
            return;
        };
        let pane_id = *pane_id;

        // Gather commands from the app. Only feed key events when the app
        // surface is the focused layer — otherwise the user is interacting
        // with the embedded terminal and the app should not steal keys.
        let (commands, scope, perms, target_id) = {
            let Some(pane) = self.contexts[active].panes.get_mut(&pane_id) else {
                return;
            };
            if pane.active_app.is_none() {
                return;
            }
            let surface_mode = pane.surface_mode;
            let focused_surface = pane.focused_surface;
            let app_has_focus =
                focused_surface == crate::app_trait::SurfaceLayer::App;

            let app = pane.active_app.as_mut().unwrap();
            if app_has_focus {
                ctx.input(|i| {
                    app.handle_key(i);
                });
            }
            let cmds = app.take_pending_commands();
            let scope = pane.app_scope.clone().unwrap_or_else(|| PathBuf::from("/"));
            let perms = pane.app_permissions.clone();
            let target = match surface_mode {
                crate::app_trait::SurfaceMode::AppWithCompanion { .. } => Some(pane_id),
                _ => pane.linked_terminal_pane,
            };
            (cmds, scope, perms, target)
        };

        if let Some(target_id) = target_id {
            if let Some(target_pane) = self.contexts[active].panes.get_mut(&target_id) {
                for cmd in commands {
                    match crate::app_permissions::check_command(&cmd, &perms, &scope) {
                        crate::app_permissions::PermissionCheck::Allowed => {
                            Self::execute_app_command(cmd, target_pane);
                        }
                        crate::app_permissions::PermissionCheck::Denied(reason) => {
                            log::warn!("App command denied: {reason}");
                        }
                    }
                }
            }
        }
    }

    fn execute_app_command(cmd: crate::app_trait::AppCommand, pane: &mut crate::pane::TerminalPane) {
        use crate::app_trait::AppCommand;
        use egui_term::BackendCommand;

        match cmd {
            AppCommand::RunInTerminal(command) => {
                let mut bytes = command.into_bytes();
                bytes.push(b'\n');
                pane.backend.process_command(BackendCommand::Write(bytes));
            }
            AppCommand::Cd(path) => {
                // Clear the terminal then cd, so the user doesn't see the raw
                // `cd` command printing and reflowing. The next prompt appears
                // clean in the new directory.
                let cmd = format!("clear && cd {}\n", shell_escape(&path.display().to_string()));
                pane.backend
                    .process_command(BackendCommand::Write(cmd.into_bytes()));
            }
            AppCommand::Notify(_msg) => {
                // Notification system not yet wired — no-op for now.
            }
        }
    }

    /// Poll the linked terminal's CWD and sync it to the active app.
    /// This enables two-way directory sync: terminal cd → file browser updates.
    ///
    /// For `AppWithCompanion` panes the "linked" terminal is the same pane,
    /// so we sync the app to its own backend's CWD.
    fn sync_app_cwd(&mut self) {
        let ctx = &mut self.contexts[self.active_context];
        // Collect (app_pane_id, terminal_pane_id) pairs for every pane that
        // has an active app, regardless of routing mode.
        let app_panes: Vec<(PaneId, PaneId)> = ctx
            .panes
            .iter()
            .filter_map(|(&pane_id, pane)| {
                if pane.active_app.is_none() {
                    return None;
                }
                let target = match pane.surface_mode {
                    crate::app_trait::SurfaceMode::AppWithCompanion { .. } => pane_id,
                    _ => pane.linked_terminal_pane?,
                };
                Some((pane_id, target))
            })
            .collect();

        for (app_pane_id, linked_id) in app_panes {
            // Get the linked terminal's CWD via lsof.
            let cwd = ctx.panes.get(&linked_id).and_then(|linked_pane| {
                crate::shell::get_pid_cwd(linked_pane.backend.child_pid())
            });
            if let Some(cwd) = cwd {
                if let Some(app_pane) = ctx.panes.get_mut(&app_pane_id) {
                    if let Some(app) = app_pane.active_app.as_mut() {
                        app.sync_cwd(&cwd);
                    }
                }
            }
        }
    }
}

pub(crate) fn shell_escape(s: &str) -> String {
    if s.contains(|c: char| c.is_whitespace() || "\"'\\()&|;$`!#".contains(c)) {
        format!("'{}'", s.replace('\'', "'\\''"))
    } else {
        s.to_string()
    }
}

impl eframe::App for PlexiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_pty_events();
        self.dispatch_app_key_events(ctx);
        self.sync_app_cwd();
        self.dispatch_pending_spawns();
        self.dispatch_pipe_writes();

        // Drain completed ffmpeg thumbnail extractions from worker threads. The
        // worker itself also calls `request_repaint`, but polling here keeps
        // the cache authoritative once per frame before any app reads it.
        self.media_cache.borrow_mut().poll_completions();

        // Consume any paths queued by the macOS Finder service
        // ("Open in Plexi") or by `plexi <path>` on startup, and open each
        // one as a new context.
        #[cfg(target_os = "macos")]
        {
            for path in crate::finder_service::drain_pending_paths() {
                self.open_path_as_context(path);
                ctx.request_repaint();
            }
        }

        // Check if the focused app wants to close itself (e.g. after saving).
        {
            let ctx_ref = &self.contexts[self.active_context];
            let should_close = ctx_ref.focused_pane
                .and_then(|tile| ctx_ref.tree.tiles.get(tile))
                .and_then(|tile| {
                    if let egui_tiles::Tile::Pane(pid) = tile { Some(*pid) } else { None }
                })
                .and_then(|pid| ctx_ref.panes.get(&pid))
                .and_then(|pane| pane.active_app.as_ref())
                .map(|app| app.wants_close())
                .unwrap_or(false);
            if should_close {
                self.close_focused_app();
            }
        }

        // Update window title to reflect active pane — readable by AppleScript / OS scripts
        {
            let context = &self.contexts[self.active_context];
            let pane_name = context.focused_pane
                .and_then(|tile_id| context.tree.tiles.get(tile_id))
                .and_then(|tile| if let egui_tiles::Tile::Pane(pane_id) = tile { context.panes.get(pane_id) } else { None })
                .and_then(|pane| pane.name.clone());
            let title = match pane_name {
                Some(name) => format!("{} — {}", context.name, name),
                None => context.name.clone(),
            };
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
        }

        // Determine if the focused pane has an active app surface.
        let app_active = {
            let context = &self.contexts[self.active_context];
            context.focused_pane
                .and_then(|tile_id| {
                    if let Some(egui_tiles::Tile::Pane(pane_id)) = context.tree.tiles.get(tile_id) {
                        context.panes.get(pane_id)
                    } else {
                        None
                    }
                })
                .map(|pane| pane.active_app.is_some())
                .unwrap_or(false)
        };

        // Handle keyboard shortcuts
        for action in keys::poll_actions(ctx, app_active) {
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
                    // In AppWithCompanion mode the app is layered onto the
                    // same pane as the terminal. If the App surface has
                    // focus, Cmd+W just dismisses the app (terminal remains
                    // intact). If the Terminal surface has focus, Cmd+W
                    // closes the whole pane (legacy behavior).
                    let in_pane_app_focused = {
                        let context = &self.contexts[self.active_context];
                        context.focused_pane
                            .and_then(|tile| {
                                if let Some(Tile::Pane(pid)) = context.tree.tiles.get(tile) {
                                    context.panes.get(pid)
                                } else {
                                    None
                                }
                            })
                            .map(|pane| {
                                matches!(
                                    pane.surface_mode,
                                    crate::app_trait::SurfaceMode::AppWithCompanion { .. }
                                ) && pane.focused_surface
                                    == crate::app_trait::SurfaceLayer::App
                            })
                            .unwrap_or(false)
                    };
                    if in_pane_app_focused {
                        self.close_focused_app();
                    } else {
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
                                .and_then(|p| p.name.clone())
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
                    // For AppWithCompanion (in-pane embed), toggle the
                    // focused surface within the same pane. For legacy
                    // AppActive (separate tiles), navigate down to the
                    // sibling terminal pane.
                    let in_pane_companion = {
                        let context = &self.contexts[self.active_context];
                        context.focused_pane
                            .and_then(|tile| {
                                if let Some(Tile::Pane(pane_id)) =
                                    context.tree.tiles.get(tile)
                                {
                                    context.panes.get(pane_id)
                                } else {
                                    None
                                }
                            })
                            .map(|pane| matches!(
                                pane.surface_mode,
                                crate::app_trait::SurfaceMode::AppWithCompanion { .. }
                            ))
                            .unwrap_or(false)
                    };
                    if in_pane_companion {
                        self.toggle_focused_surface();
                    } else {
                        self.navigate(crate::keys::Direction::Down);
                    }
                }
                Action::OpenFileBrowser => {
                    self.open_file_browser();
                }
                Action::OpenQuickNote => {
                    self.open_quick_note();
                }
                Action::OpenAudioPlayer => {
                    self.open_audio_player();
                }
                Action::OpenConfig => {
                    self.open_config_editor();
                }
                Action::OpenSecretsManager => {
                    self.open_secrets_manager();
                }
                Action::ToggleAgentMode => {
                    self.toggle_agent_mode();
                }
                Action::AppUndo => {
                    self.app_undo();
                }
                Action::AppRedo => {
                    self.app_redo();
                }
                Action::ToggleNotificationPalette => {
                    self.show_notification_palette = !self.show_notification_palette;
                    if self.show_notification_palette {
                        self.notification_palette_selected = 0;
                    }
                }
            }
        }

        // Handle window close request (X button / system shutdown) — always quit
        if ctx.input(|i| i.viewport().close_requested()) && !self.quitting {
            self.save_workspace();
            let size = ctx.input(|i| i.screen_rect().size());
            config::save_window_size(size.x, size.y);
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
                    .filter_map(|(&id, p)| p.name.as_ref().map(|n| (id, n.clone())))
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
                        ui.ctx().request_repaint_after(std::time::Duration::from_millis(16)); // continuous repaints while dragging
                        use objc2_app_kit::NSApplication;
                        use objc2_foundation::MainThreadMarker;
                        MainThreadMarker::new()
                            .and_then(|mtm| {
                                let app = NSApplication::sharedApplication(mtm);
                                app.keyWindow()
                                    .or_else(|| unsafe { app.mainWindow() })
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
                    focused_tile: if suppress_focus { None } else { ctx.focused_pane },
                    theme: self.theme.clone(),
                    new_focused: None,
                    close_exited: None,
                    tab_info,
                    zoomed_pane,
                    colors: self.colors,
                    pane_names,
                    drag_cursor_pos,
                    media_cache: &self.media_cache,
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
                        ui.painter().rect_filled(
                            panel_rect,
                            0.0,
                            Color32::from_black_alpha(75),
                        );

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
                        let mut child_ui = ui.new_child(
                            egui::UiBuilder::new().max_rect(inner_rect),
                        );
                        egui::Frame::new()
                            .fill(self.colors.terminal_bg)
                            .inner_margin(egui::Margin::same(8))
                            .show(&mut child_ui, |ui| {
                                if let Some(pane) = ctx.panes.get_mut(&pane_id) {
                                    if pane.exited {
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
                                    } else if pane.surface_mode == crate::app_trait::SurfaceMode::AppActive {
                                        // Zoomed app: render the app surface full-size.
                                        if let Some(app) = pane.active_app.as_mut() {
                                            let app_ctx = crate::app_trait::AppRenderContext {
                                                colors: &self.colors,
                                                is_focused: true,
                                                linked_terminal: pane_id,
                                                media_cache: &self.media_cache,
                                            };
                                            app.ui(ui, &app_ctx);
                                        }
                                    } else if let crate::app_trait::SurfaceMode::AppWithCompanion {
                                        companion_fraction,
                                    } = pane.surface_mode
                                    {
                                        // Zoomed AppWithCompanion: same vertical
                                        // split as in the tile renderer (app on
                                        // top, embedded terminal on bottom).
                                        let pane_rect = ui.max_rect();
                                        let frac = companion_fraction.clamp(0.10, 0.90);
                                        let separator_h = 1.0;
                                        let term_h = (pane_rect.height() * frac).max(40.0);
                                        let app_h =
                                            (pane_rect.height() - term_h - separator_h).max(40.0);
                                        let app_rect = egui::Rect::from_min_size(
                                            pane_rect.min,
                                            egui::vec2(pane_rect.width(), app_h),
                                        );
                                        let sep_rect = egui::Rect::from_min_size(
                                            egui::pos2(pane_rect.left(), pane_rect.top() + app_h),
                                            egui::vec2(pane_rect.width(), separator_h),
                                        );
                                        let term_rect = egui::Rect::from_min_size(
                                            egui::pos2(
                                                pane_rect.left(),
                                                pane_rect.top() + app_h + separator_h,
                                            ),
                                            egui::vec2(pane_rect.width(), term_h),
                                        );
                                        ui.painter().rect_filled(
                                            sep_rect,
                                            0.0,
                                            self.colors.border,
                                        );
                                        let app_focused = pane.focused_surface
                                            == crate::app_trait::SurfaceLayer::App;
                                        if let Some(app) = pane.active_app.as_mut() {
                                            let mut app_ui = ui.new_child(
                                                egui::UiBuilder::new().max_rect(app_rect),
                                            );
                                            let app_ctx = crate::app_trait::AppRenderContext {
                                                colors: &self.colors,
                                                is_focused: app_focused,
                                                linked_terminal: pane_id,
                                                media_cache: &self.media_cache,
                                            };
                                            app.ui(&mut app_ui, &app_ctx);
                                        }
                                        let mut term_ui = ui.new_child(
                                            egui::UiBuilder::new().max_rect(term_rect),
                                        );
                                        let font_size = pane.font_size;
                                        let terminal =
                                            TerminalView::new(&mut term_ui, &mut pane.backend)
                                                .set_focus(!app_focused)
                                                .set_theme(self.theme.clone())
                                                .set_font(theme::terminal_font(font_size))
                                                .set_size(Vec2::new(
                                                    term_rect.width(),
                                                    term_rect.height(),
                                                ));
                                        term_ui.add(terminal);
                                    } else {
                                        // Reserve space for tab dots if in a tab group
                                        if zoomed_tab_info.is_some() {
                                            ui.add_space(crate::tiling::TAB_DOT_RESERVED_HEIGHT);
                                        }
                                        let font_size = pane.font_size;
                                        let terminal =
                                            TerminalView::new(ui, &mut pane.backend)
                                                .set_focus(true)
                                                .set_theme(self.theme.clone())
                                                .set_font(theme::terminal_font(font_size))
                                                .set_size(Vec2::new(
                                                    ui.available_width(),
                                                    ui.available_height(),
                                                ));
                                        ui.add(terminal);
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

        // Notification palette overlay
        if self.show_notification_palette {
            self.draw_notification_palette(ctx);
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
        self.pane_visit_history.retain(|&(c, t)| !(c == ctx_idx && t == tile_id));
        self.pane_visit_history.insert(0, (ctx_idx, tile_id));
        self.pane_visit_history.truncate(100);
    }

    pub(crate) fn record_app_visit(&mut self, id: &str) {
        self.app_visit_history.retain(|s| s != id);
        self.app_visit_history.insert(0, id.to_string());
        self.app_visit_history.truncate(100);
        config::save_app_mru(&self.app_visit_history);
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
                    painter.rect_filled(
                        screen,
                        0.0,
                        Color32::from_rgba_unmultiplied(0, 40, 0, 18),
                    );

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
