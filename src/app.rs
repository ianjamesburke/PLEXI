use crate::app_registry::AppRegistry;
use crate::config;
use crate::context::Context;
use crate::keys::{self, Action};
use crate::pane::TerminalPane;
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
    pub(crate) pane_visit_history: Vec<(usize, egui_tiles::TileId)>,
    pub(crate) renaming_pane: Option<PaneId>,
}

impl PlexiApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        #[cfg(target_os = "macos")]
        crate::macos_menu::customize_app_menu();

        theme::setup_fonts(&cc.egui_ctx);
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);

        let config = config::PlexiConfig::load();
        let default_font_size = config.font_size.unwrap_or(theme::FONT_SIZE);
        let theme_cfg = config.theme.unwrap_or_default();
        let colors = Colors::from_config(&theme_cfg);
        theme::setup_style(&cc.egui_ctx, &colors);

        let (tx, rx) = mpsc::channel();

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
                            let app: Option<Box<dyn crate::app_trait::App>> = match app_type.as_str() {
                                "file_browser" => {
                                    let cwd = saved_pane.cwd.clone();
                                    let mut fb = crate::file_browser_app::FileBrowserApp::new(cwd.clone());
                                    if let Some(state) = &saved_pane.active_app_state {
                                        use crate::app_trait::App;
                                        fb.restore_state(state);
                                    }
                                    Some(Box::new(fb))
                                }
                                _ => None,
                            };
                            if let Some(app) = app {
                                let perms = crate::app_permissions::AppPermissions::builtin();
                                let scope = saved_pane.cwd.clone();
                                pane.open_app(app, perms, scope);
                                pane.linked_terminal_pane = saved_pane.linked_terminal_pane;
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
                    pane_visit_history: Vec::new(),
                    renaming_pane: None,
                    registry: AppRegistry::load(),
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
            pane_visit_history: Vec::new(),
            renaming_pane: None,
            registry: AppRegistry::load(),
        }
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
        while let Ok((id, event)) = self.pty_event_rx.try_recv() {
            if matches!(event, PtyEvent::Exit) {
                for context in &mut self.contexts {
                    if let Some(pane) = context.panes.get_mut(&id) {
                        pane.exited = true;
                        break;
                    }
                }
            }
        }
    }

    /// Feed keyboard input to the focused pane's active app and dispatch any
    /// resulting AppCommands to the linked terminal pane.
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

        // Gather commands from the app.
        let (commands, scope, perms, linked_id) = {
            let Some(pane) = self.contexts[active].panes.get_mut(&pane_id) else {
                return;
            };
            if pane.active_app.is_none() {
                return;
            }
            let app = pane.active_app.as_mut().unwrap();
            ctx.input(|i| {
                app.handle_key(i);
            });
            let cmds = app.take_pending_commands();
            let scope = pane.app_scope.clone().unwrap_or_else(|| PathBuf::from("/"));
            let perms = pane.app_permissions.clone();
            let linked = pane.linked_terminal_pane;
            (cmds, scope, perms, linked)
        };

        // Route commands to the linked terminal pane (the one below).
        if let Some(linked_id) = linked_id {
            if let Some(target_pane) = self.contexts[active].panes.get_mut(&linked_id) {
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
    fn sync_app_cwd(&mut self) {
        let ctx = &mut self.contexts[self.active_context];
        // Collect pane IDs that have an active app with a linked terminal.
        let app_panes: Vec<(PaneId, PaneId)> = ctx
            .panes
            .iter()
            .filter_map(|(&pane_id, pane)| {
                let linked = pane.linked_terminal_pane?;
                if pane.active_app.is_some() {
                    Some((pane_id, linked))
                } else {
                    None
                }
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

fn shell_escape(s: &str) -> String {
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
                    // Tab navigates between the app pane and the linked
                    // terminal pane below (they're separate tiles now).
                    self.navigate(crate::keys::Direction::Down);
                }
                Action::OpenFileBrowser => {
                    self.open_file_browser();
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
                        ui.ctx().request_repaint(); // continuous repaints while dragging
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
                                            };
                                            app.ui(ui, &app_ctx);
                                        }
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

        // Rename pane overlay
        if self.renaming_pane.is_some() {
            self.draw_rename_pane_overlay(ctx);
        }
    }
}

impl PlexiApp {
    pub(crate) fn record_pane_visit(&mut self, ctx_idx: usize, tile_id: egui_tiles::TileId) {
        self.pane_visit_history.retain(|&(c, t)| !(c == ctx_idx && t == tile_id));
        self.pane_visit_history.insert(0, (ctx_idx, tile_id));
        self.pane_visit_history.truncate(100);
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
