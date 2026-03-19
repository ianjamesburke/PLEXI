use crate::keys::{self, Action, Direction};
use crate::pane::TerminalPane;
use crate::shell;
use crate::theme::{self, Colors};
use crate::tiling::{PaneId, PlexiBehavior};
use crate::workspace::WorkspaceFile;
use egui::{
    Align, Align2, Color32, CornerRadius, Layout, Rect, RichText, Stroke, StrokeKind, Vec2,
};
use egui_term::{BackendSettings, PtyEvent, TerminalFont, TerminalTheme, TerminalView};
use egui_tiles::{Container, SimplificationOptions, Tile, TileId, Tree};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc;

enum ContextMenuAction {
    Rename,
    MoveToTop,
    MoveUp,
    MoveDown,
    Delete,
}

pub struct Context {
    pub name: String,
    pub path: PathBuf,
    pub tree: Tree<PaneId>,
    pub panes: HashMap<PaneId, TerminalPane>,
    pub focused_pane: Option<TileId>,
    pub zoomed_pane: Option<TileId>,
}

impl Context {
    fn find_ancestor_tabs(&self, tile_id: TileId) -> Option<(TileId, TileId)> {
        let mut current = tile_id;
        loop {
            let parent_id = self.tree.tiles.parent_of(current)?;
            if matches!(
                self.tree.tiles.get(parent_id),
                Some(Tile::Container(Container::Tabs(_)))
            ) {
                return Some((parent_id, current));
            }
            current = parent_id;
        }
    }

    fn find_logical_parent(&self, tile_id: TileId) -> Option<(TileId, TileId)> {
        let mut current = tile_id;
        loop {
            let parent_id = self.tree.tiles.parent_of(current)?;
            if let Some(Tile::Container(container)) = self.tree.tiles.get(parent_id) {
                if container.children().count() > 1 {
                    return Some((parent_id, current));
                }
            }
            current = parent_id;
        }
    }

    fn find_first_pane_in(&self, tile_id: TileId) -> Option<TileId> {
        match self.tree.tiles.get(tile_id)? {
            Tile::Pane(_) => Some(tile_id),
            Tile::Container(container) => {
                if let Container::Tabs(tabs) = container {
                    // Only follow the active tab — others are invisible
                    return self.find_first_pane_in(tabs.active?);
                }
                for &child in container.children() {
                    if let Some(pane) = self.find_first_pane_in(child) {
                        return Some(pane);
                    }
                }
                None
            }
        }
    }

    fn find_next_focus(&self, excluding: TileId) -> Option<TileId> {
        for dir in [
            Direction::Left,
            Direction::Up,
            Direction::Right,
            Direction::Down,
        ] {
            if let Some(target) = self.find_pane_in_direction_from(excluding, dir) {
                return Some(target);
            }
        }
        self.tree
            .active_tiles()
            .into_iter()
            .find(|&id| id != excluding && matches!(self.tree.tiles.get(id), Some(Tile::Pane(_))))
    }

    fn find_pane_in_direction_from(&self, from: TileId, dir: Direction) -> Option<TileId> {
        let from_rect = self.tree.tiles.rect(from)?;
        let center = from_rect.center();

        // Score: (overlap_tier, primary_axis_distance)
        // tier 0 = candidate overlaps on perpendicular axis, tier 1 = no overlap (fallback)
        let mut best: Option<(TileId, (u8, f32))> = None;

        let is_horizontal = matches!(dir, Direction::Left | Direction::Right);

        for tile_id in self.tree.active_tiles() {
            if tile_id == from {
                continue;
            }
            if !matches!(self.tree.tiles.get(tile_id), Some(Tile::Pane(_))) {
                continue;
            }
            let Some(rect) = self.tree.tiles.rect(tile_id) else {
                continue;
            };
            let other = rect.center();

            let valid = match dir {
                Direction::Left => other.x < center.x,
                Direction::Right => other.x > center.x,
                Direction::Up => other.y < center.y,
                Direction::Down => other.y > center.y,
            };

            if valid {
                let (has_overlap, primary_dist) = if is_horizontal {
                    // For Left/Right: check y-range overlap, distance along x
                    let overlap = from_rect.top() < rect.bottom() && rect.top() < from_rect.bottom();
                    (overlap, (other.x - center.x).abs())
                } else {
                    // For Up/Down: check x-range overlap, distance along y
                    let overlap = from_rect.left() < rect.right() && rect.left() < from_rect.right();
                    (overlap, (other.y - center.y).abs())
                };

                let tier = if has_overlap { 0 } else { 1 };
                let score = (tier, primary_dist);
                if best.map_or(true, |(_, s)| score < s) {
                    best = Some((tile_id, score));
                }
            }
        }

        best.map(|(id, _)| id)
    }

    fn compute_tab_info(&self) -> HashMap<TileId, (usize, usize)> {
        let mut info = HashMap::new();
        for (_tile_id, tile) in self.tree.tiles.iter() {
            if let Tile::Container(Container::Tabs(tabs)) = tile {
                let children = &tabs.children;
                if children.len() < 2 {
                    continue;
                }
                let count = children.len();
                let active_idx = tabs
                    .active
                    .and_then(|a| children.iter().position(|&c| c == a))
                    .unwrap_or(0);
                for child in children {
                    self.collect_panes(*child, &mut |pane_tile| {
                        info.insert(pane_tile, (active_idx, count));
                    });
                }
            }
        }
        info
    }

    fn collect_panes(&self, tile_id: TileId, f: &mut dyn FnMut(TileId)) {
        match self.tree.tiles.get(tile_id) {
            Some(Tile::Pane(_)) => f(tile_id),
            Some(Tile::Container(container)) => {
                for &child in container.children() {
                    self.collect_panes(child, f);
                }
            }
            None => {}
        }
    }

    fn get_focused_pane_cwd(&self, tile_id: TileId) -> Option<PathBuf> {
        let pane_id = match self.tree.tiles.get(tile_id)? {
            Tile::Pane(id) => *id,
            _ => return None,
        };
        let pane = self.panes.get(&pane_id)?;
        shell::get_pid_cwd(pane.backend.child_pid())
    }
}

pub struct PlexiApp {
    pty_event_rx: mpsc::Receiver<(u64, PtyEvent)>,
    pty_event_tx: mpsc::Sender<(u64, PtyEvent)>,
    theme: TerminalTheme,
    font: TerminalFont,
    next_pane_id: u64,
    ctx: egui::Context,
    contexts: Vec<Context>,
    active_context: usize,
    sidebar_visible: bool,
    show_shortcuts: bool,
    quitting: bool,
    renaming_context: Option<usize>,
    rename_buffer: String,
}

impl PlexiApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        #[cfg(target_os = "macos")]
        crate::macos_menu::remove_intercepted_menu_items();

        theme::setup_fonts(&cc.egui_ctx);
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        theme::setup_style(&cc.egui_ctx);

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
                    let settings = Self::make_backend_settings(cwd);
                    if let Some(pane) =
                        TerminalPane::new(saved_pane.id, cc.egui_ctx.clone(), tx.clone(), settings)
                    {
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
                    theme: theme::catppuccin_mocha(),
                    font: theme::terminal_font(),
                    next_pane_id: ws.next_pane_id,
                    ctx: cc.egui_ctx.clone(),
                    contexts,
                    active_context: active,
                    sidebar_visible: ws.sidebar_visible,
                    show_shortcuts: false,
                    quitting: false,
                    renaming_context: None,
                    rename_buffer: String::new(),
                };
            }
        }

        // Default: single context with single pane
        let settings = Self::make_backend_settings(None);
        let pane = TerminalPane::new(0, cc.egui_ctx.clone(), tx.clone(), settings)
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
            theme: theme::catppuccin_mocha(),
            font: theme::terminal_font(),
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
        }
    }

    fn make_backend_settings(working_directory: Option<PathBuf>) -> BackendSettings {
        BackendSettings {
            shell: shell::detect_shell(),
            args: vec!["-l".to_string()],
            env: shell::build_env(),
            working_directory,
            ..Default::default()
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

    fn split_focused(&mut self, vertical: bool) {
        let Some(focused) = self.contexts[self.active_context].focused_pane else {
            return;
        };

        let new_id = self.next_pane_id;
        self.next_pane_id += 1;

        let cwd = self.contexts[self.active_context].get_focused_pane_cwd(focused);
        let settings = Self::make_backend_settings(cwd);
        let Some(pane) =
            TerminalPane::new(new_id, self.ctx.clone(), self.pty_event_tx.clone(), settings)
        else {
            log::error!("Failed to create new terminal pane");
            return;
        };
        self.contexts[self.active_context]
            .panes
            .insert(new_id, pane);

        let split_target =
            match self.contexts[self.active_context].find_ancestor_tabs(focused) {
                Some((tabs_id, _)) => tabs_id,
                None => focused,
            };

        let ctx = &mut self.contexts[self.active_context];
        let parent = ctx.tree.tiles.parent_of(split_target);
        let new_tile = ctx.tree.tiles.insert_pane(new_id);

        let split_dir = if vertical {
            egui_tiles::LinearDir::Vertical
        } else {
            egui_tiles::LinearDir::Horizontal
        };

        let inserted_as_sibling = if let Some(parent_id) = parent {
            if let Some(Tile::Container(Container::Linear(linear))) =
                ctx.tree.tiles.get_mut(parent_id)
            {
                if linear.dir == split_dir {
                    if let Some(pos) = linear.children.iter().position(|&c| c == split_target) {
                        linear.children.insert(pos + 1, new_tile);
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            } else {
                false
            }
        } else {
            false
        };

        if !inserted_as_sibling {
            let container_tile = if vertical {
                ctx.tree
                    .tiles
                    .insert_vertical_tile(vec![split_target, new_tile])
            } else {
                ctx.tree
                    .tiles
                    .insert_horizontal_tile(vec![split_target, new_tile])
            };

            if let Some(parent_id) = parent {
                if let Some(Tile::Container(parent)) = ctx.tree.tiles.get_mut(parent_id) {
                    replace_child(parent, split_target, container_tile);
                }
            } else {
                ctx.tree.root = Some(container_tile);
            }
        }

        ctx.focused_pane = Some(new_tile);
    }

    fn new_tab(&mut self) {
        let Some(focused) = self.contexts[self.active_context].focused_pane else {
            return;
        };

        let new_id = self.next_pane_id;
        self.next_pane_id += 1;

        let cwd = self.contexts[self.active_context].get_focused_pane_cwd(focused);
        let settings = Self::make_backend_settings(cwd);
        let Some(pane) =
            TerminalPane::new(new_id, self.ctx.clone(), self.pty_event_tx.clone(), settings)
        else {
            log::error!("Failed to create new terminal pane");
            return;
        };
        self.contexts[self.active_context]
            .panes
            .insert(new_id, pane);

        let ctx = &mut self.contexts[self.active_context];
        let new_tile = ctx.tree.tiles.insert_pane(new_id);

        if let Some((tabs_id, _)) = ctx.find_ancestor_tabs(focused) {
            if let Some(Tile::Container(Container::Tabs(tabs))) =
                ctx.tree.tiles.get_mut(tabs_id)
            {
                tabs.add_child(new_tile);
                tabs.set_active(new_tile);
            }
            ctx.focused_pane = Some(new_tile);
            return;
        }

        let parent = ctx.tree.tiles.parent_of(focused);
        let tab_tile = ctx.tree.tiles.insert_tab_tile(vec![focused, new_tile]);

        if let Some(Tile::Container(Container::Tabs(tabs))) = ctx.tree.tiles.get_mut(tab_tile) {
            tabs.set_active(new_tile);
        }

        if let Some(parent_id) = parent {
            if let Some(Tile::Container(parent_container)) = ctx.tree.tiles.get_mut(parent_id) {
                replace_child(parent_container, focused, tab_tile);
            }
        } else {
            ctx.tree.root = Some(tab_tile);
        }

        ctx.focused_pane = Some(new_tile);
    }

    fn cycle_tab(&mut self, forward: bool) {
        let ctx = &self.contexts[self.active_context];
        let Some(focused) = ctx.focused_pane else {
            return;
        };

        let Some((tabs_id, current_tab_child)) = ctx.find_ancestor_tabs(focused) else {
            return;
        };

        let Some(Tile::Container(Container::Tabs(tabs))) = ctx.tree.tiles.get(tabs_id) else {
            return;
        };

        let children = &tabs.children;
        if children.len() < 2 {
            return;
        }

        let Some(pos) = children.iter().position(|&c| c == current_tab_child) else {
            return;
        };

        let next_pos = if forward {
            (pos + 1) % children.len()
        } else {
            (pos + children.len() - 1) % children.len()
        };
        let next_tile = children[next_pos];

        let ctx = &mut self.contexts[self.active_context];
        if let Some(Tile::Container(Container::Tabs(tabs))) = ctx.tree.tiles.get_mut(tabs_id) {
            tabs.set_active(next_tile);
        }

        if let Some(pane_tile) = ctx.find_first_pane_in(next_tile) {
            ctx.focused_pane = Some(pane_tile);
            if ctx.zoomed_pane.is_some() {
                ctx.zoomed_pane = Some(pane_tile);
            }
        }
    }

    fn close_focused(&mut self) {
        let focused = match self.contexts[self.active_context].focused_pane {
            Some(f) => f,
            None => return,
        };

        // Phase 1: Read-only — determine sibling and container type
        let parent_info = self.contexts[self.active_context].find_logical_parent(focused);

        let next = if let Some((parent_id, child_in_parent)) = parent_info {
            let sibling_info = {
                let ctx = &self.contexts[self.active_context];
                if let Some(Tile::Container(container)) = ctx.tree.tiles.get(parent_id) {
                    let children: Vec<TileId> = container.children().copied().collect();
                    children
                        .iter()
                        .position(|&c| c == child_in_parent)
                        .map(|pos| {
                            let sibling = if pos > 0 {
                                children[pos - 1]
                            } else {
                                children[pos + 1]
                            };
                            let is_tabs = matches!(container, Container::Tabs(_));
                            let is_linear = matches!(container, Container::Linear(_));
                            (sibling, is_tabs, is_linear, children)
                        })
                } else {
                    None
                }
            };

            if let Some((sibling, is_tabs, is_linear, all_children)) = sibling_info {
                // Phase 2: Mutable — update container state
                let ctx = &mut self.contexts[self.active_context];
                if is_tabs {
                    if let Some(Tile::Container(Container::Tabs(tabs))) =
                        ctx.tree.tiles.get_mut(parent_id)
                    {
                        tabs.set_active(sibling);
                    }
                }
                if is_linear {
                    if let Some(Tile::Container(Container::Linear(linear))) =
                        ctx.tree.tiles.get_mut(parent_id)
                    {
                        for &child in &all_children {
                            linear.shares.set_share(child, 1.0);
                        }
                    }
                }

                self.contexts[self.active_context].find_first_pane_in(sibling)
            } else {
                self.contexts[self.active_context].find_next_focus(focused)
            }
        } else {
            self.contexts[self.active_context].find_next_focus(focused)
        };

        // Phase 3: Remove tile and pane
        let ctx = &mut self.contexts[self.active_context];
        if let Some(parent_id) = ctx.tree.tiles.parent_of(focused) {
            if let Some(Tile::Container(parent)) = ctx.tree.tiles.get_mut(parent_id) {
                parent.remove_child(focused);
            }
        }

        if let Some(Tile::Pane(pane_id)) = ctx.tree.tiles.remove(focused) {
            ctx.panes.remove(&pane_id);
        }

        ctx.tree.simplify(&SimplificationOptions {
            all_panes_must_have_tabs: true,
            ..SimplificationOptions::default()
        });
        ctx.focused_pane = next;
    }

    fn navigate(&mut self, dir: Direction) {
        let ctx = &self.contexts[self.active_context];
        if let Some(focused) = ctx.focused_pane {
            if let Some(target) = ctx.find_pane_in_direction_from(focused, dir) {
                self.contexts[self.active_context].focused_pane = Some(target);
            }
        }
    }

    fn new_context(&mut self) {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let new_id = self.next_pane_id;
        self.next_pane_id += 1;

        let settings = Self::make_backend_settings(Some(home.clone()));
        let Some(pane) =
            TerminalPane::new(new_id, self.ctx.clone(), self.pty_event_tx.clone(), settings)
        else {
            log::error!("Failed to create terminal for new context");
            return;
        };

        let mut panes = HashMap::new();
        panes.insert(new_id, pane);

        let mut tiles = egui_tiles::Tiles::default();
        let root_tile = tiles.insert_pane(new_id);
        let tree = Tree::new("plexi", root_tile, tiles);

        let name = format!("Context {}", self.contexts.len() + 1);
        self.contexts.push(Context {
            name,
            path: home,
            tree,
            panes,
            focused_pane: Some(root_tile),
            zoomed_pane: None,
        });
        self.active_context = self.contexts.len() - 1;
    }

    fn delete_context(&mut self, index: usize) {
        if self.contexts.len() <= 1 {
            return;
        }
        self.contexts.remove(index);
        if self.active_context >= self.contexts.len() {
            self.active_context = self.contexts.len() - 1;
        }
        // Clear rename state if it referenced the deleted context
        if self.renaming_context == Some(index) {
            self.renaming_context = None;
        } else if let Some(r) = self.renaming_context {
            if r > index {
                self.renaming_context = Some(r - 1);
            }
        }
    }

    fn save_workspace(&self) {
        let mut saved_contexts = Vec::new();
        for context in &self.contexts {
            let mut saved_panes = Vec::new();
            for (&id, pane) in &context.panes {
                let cwd = shell::get_pid_cwd(pane.backend.child_pid())
                    .unwrap_or_else(|| context.path.clone());
                saved_panes.push(crate::workspace::SavedPane { id, cwd });
            }
            saved_contexts.push(crate::workspace::SavedContext {
                name: context.name.clone(),
                path: context.path.clone(),
                tree: context.tree.clone(),
                panes: saved_panes,
                focused_pane: context.focused_pane,
            });
        }

        let ws = WorkspaceFile {
            version: 1,
            active_context: self.active_context,
            sidebar_visible: self.sidebar_visible,
            next_pane_id: self.next_pane_id,
            contexts: saved_contexts,
        };

        if let Err(e) = ws.save() {
            log::error!("Failed to save workspace: {e}");
        }
    }
}

impl eframe::App for PlexiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_pty_events();

        // Handle keyboard shortcuts
        for action in keys::poll_actions(ctx) {
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
                        self.save_workspace();
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
                Action::NewTab => self.new_tab(),
                Action::NextTab => self.cycle_tab(true),
                Action::PrevTab => self.cycle_tab(false),
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
                Action::SwitchContext(i) => {
                    if i < self.contexts.len() {
                        self.active_context = i;
                    }
                }
            }
        }

        // Handle window close request (X button, Cmd+W on macOS)
        if ctx.input(|i| i.viewport().close_requested()) && !self.quitting {
            let total_panes: usize = self.contexts.iter().map(|c| c.panes.len()).sum();
            if total_panes > 1 {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                let active_panes = self.contexts[self.active_context].panes.len();
                if active_panes > 1 {
                    self.close_focused();
                } else if self.contexts.len() > 1 {
                    self.delete_context(self.active_context);
                }
            }
            // Always save on close (X button, system shutdown, last pane)
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
                    .fill(Colors::BG_TOOLBAR)
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
            .frame(egui::Frame::new().fill(Colors::BORDER))
            .show(ctx, |_ui| {});

        // Sidebar
        if self.sidebar_visible {
            egui::SidePanel::left("sidebar")
                .exact_width(220.0)
                .frame(
                    egui::Frame::new()
                        .fill(Colors::BG_SIDEBAR)
                        .inner_margin(egui::Margin::same(0)),
                )
                .show(ctx, |ui| {
                    self.draw_sidebar(ui);
                });
        }

        // Central panel — terminal tiles
        egui::CentralPanel::default()
            .frame(egui::Frame {
                fill: Colors::BG_DARKEST,
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
                let mut behavior = PlexiBehavior {
                    panes: &mut ctx.panes,
                    focused_tile: if self.renaming_context.is_some() { None } else { ctx.focused_pane },
                    theme: self.theme.clone(),
                    font: self.font.clone(),
                    new_focused: None,
                    close_exited: None,
                    tab_info,
                    zoomed_pane,
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
                            Color32::from_black_alpha(80),
                        );

                        // Inset rect for the zoomed pane
                        let inset = 10.0;
                        let zoom_rect = panel_rect.shrink(inset);

                        // Thicker blue border (2px)
                        let accent = Color32::from_rgb(137, 180, 250);
                        ui.painter().rect_stroke(
                            zoom_rect,
                            CornerRadius::same(4),
                            Stroke::new(2.0, accent),
                            StrokeKind::Inside,
                        );

                        // Render zoomed terminal in the inset rect
                        let inner_rect = zoom_rect.shrink(2.0); // inside the border
                        let mut child_ui = ui.new_child(
                            egui::UiBuilder::new().max_rect(inner_rect),
                        );
                        egui::Frame::new()
                            .fill(Color32::from_rgb(0x1e, 0x1e, 0x2e))
                            .inner_margin(egui::Margin::same(8))
                            .show(&mut child_ui, |ui| {
                                if let Some(pane) = ctx.panes.get_mut(&pane_id) {
                                    if pane.exited {
                                        let rect = ui.max_rect();
                                        ui.painter().rect_filled(
                                            rect,
                                            0.0,
                                            Color32::from_rgb(0x1e, 0x1e, 0x2e),
                                        );
                                        ui.allocate_new_ui(
                                            egui::UiBuilder::new().max_rect(rect),
                                            |ui| {
                                                ui.centered_and_justified(|ui| {
                                                    ui.colored_label(
                                                        Color32::from_rgb(0x6c, 0x70, 0x86),
                                                        "[process exited]",
                                                    );
                                                });
                                            },
                                        );
                                    } else {
                                        // Reserve space for tab dots if in a tab group
                                        if zoomed_tab_info.is_some() {
                                            ui.add_space(14.0);
                                        }
                                        let terminal =
                                            TerminalView::new(ui, &mut pane.backend)
                                                .set_focus(true)
                                                .set_theme(self.theme.clone())
                                                .set_font(self.font.clone())
                                                .set_size(Vec2::new(
                                                    ui.available_width(),
                                                    ui.available_height(),
                                                ));
                                        ui.add(terminal);
                                    }
                                }

                                // Draw tab indicator dots (same style as tiling.rs)
                                if let Some((active_idx, count)) = zoomed_tab_info {
                                    let dot_radius = 4.0;
                                    let dot_spacing = 12.0;
                                    let rect = ui.max_rect();
                                    let start_x = rect.left() + 2.0;
                                    let y = rect.top() + 2.0 + dot_radius;

                                    let accent = Color32::from_rgb(137, 180, 250);
                                    let dim = Color32::from_rgb(0x45, 0x47, 0x5a);

                                    for i in 0..count {
                                        let cx = start_x + (i as f32) * dot_spacing + dot_radius;
                                        let color = if i == active_idx { accent } else { dim };
                                        ui.painter().circle_filled(egui::pos2(cx, y), dot_radius, color);
                                    }
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
    }
}

const R6: CornerRadius = CornerRadius::same(6);

impl PlexiApp {
    fn draw_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let active_ctx = &self.contexts[self.active_context];

            // Sidebar toggle
            let toggle_text = if self.sidebar_visible {
                "\u{25C0}"
            } else {
                "\u{25B6}"
            };
            if ui
                .add(
                    egui::Button::new(
                        RichText::new(toggle_text).size(11.0).color(Colors::TEXT_DIM),
                    )
                    .frame(false),
                )
                .on_hover_text("Toggle sidebar (\u{2318}B)")
                .clicked()
            {
                self.sidebar_visible = !self.sidebar_visible;
            }

            ui.add_space(8.0);

            // Context info
            ui.label(
                RichText::new(&active_ctx.name)
                    .size(12.0)
                    .color(Colors::TEXT_PRIMARY)
                    .strong(),
            );
            ui.label(
                RichText::new(active_ctx.path.display().to_string())
                    .size(11.0)
                    .color(Colors::TEXT_DIM)
                    .family(egui::FontFamily::Monospace),
            );
            let pane_count = active_ctx.panes.len();
            ui.label(
                RichText::new(format!(
                    "{} pane{}",
                    pane_count,
                    if pane_count == 1 { "" } else { "s" }
                ))
                .size(11.0)
                .color(Colors::TEXT_SECTION),
            );

            // Right side — help button
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new("?").size(12.0).color(Colors::TEXT_DIM),
                        )
                        .frame(false),
                    )
                    .on_hover_text("Keyboard shortcuts (\u{2318}/)")
                    .clicked()
                {
                    self.show_shortcuts = !self.show_shortcuts;
                }
            });
        });
    }

    fn draw_sidebar(&mut self, ui: &mut egui::Ui) {
        let sidebar_width = ui.available_width();

        // Branding
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            ui.label(
                RichText::new("PLEXI")
                    .size(16.0)
                    .color(Colors::TEXT_PRIMARY)
                    .strong(),
            );
        });
        ui.add_space(12.0);

        // Divider
        let rect = ui.cursor();
        ui.painter().line_segment(
            [
                egui::pos2(rect.min.x, rect.min.y),
                egui::pos2(rect.min.x + sidebar_width, rect.min.y),
            ],
            Stroke::new(1.0, Colors::BORDER),
        );
        ui.add_space(4.0);

        // Contexts section header with "+" button
        ui.add_space(8.0);
        let mut add_clicked = false;
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            ui.label(
                RichText::new("Contexts")
                    .size(10.0)
                    .color(Colors::TEXT_SECTION),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(12.0);
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new("+").size(12.0).color(Colors::TEXT_DIM),
                        )
                        .frame(false),
                    )
                    .on_hover_text("New context")
                    .clicked()
                {
                    add_clicked = true;
                }
            });
        });
        ui.add_space(4.0);

        // Context list — iterate by index to avoid borrow issues with rename_buffer
        let num_contexts = self.contexts.len();
        let mut clicked_context: Option<usize> = None;
        let mut double_clicked_context: Option<usize> = None;
        let mut delete_context: Option<usize> = None;
        let mut menu_action: Option<(usize, ContextMenuAction)> = None;

        for i in 0..num_contexts {
            let is_active = i == self.active_context;
            let is_renaming = self.renaming_context == Some(i);

            let response = ui.allocate_ui_with_layout(
                Vec2::new(sidebar_width, 26.0),
                Layout::left_to_right(Align::Center),
                |ui| {
                    let rect = ui.max_rect();
                    let hover = ui.rect_contains_pointer(rect);

                    let fill = if is_active {
                        Colors::BG_ACTIVE
                    } else if hover {
                        Colors::BG_HOVER
                    } else {
                        Color32::TRANSPARENT
                    };
                    ui.painter()
                        .rect_filled(rect, CornerRadius::ZERO, fill);

                    if is_active {
                        ui.painter().rect_filled(
                            Rect::from_min_size(rect.min, Vec2::new(3.0, rect.height())),
                            CornerRadius::ZERO,
                            Colors::ACCENT,
                        );
                    }

                    ui.add_space(20.0);

                    if is_renaming {
                        let te_id = egui::Id::new(("rename_ctx", i));
                        let te = ui.add(
                            egui::TextEdit::singleline(&mut self.rename_buffer)
                                .id(te_id)
                                .desired_width(sidebar_width - 56.0)
                                .font(egui::TextStyle::Body),
                        );
                        if te.lost_focus() {
                            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                                self.renaming_context = None;
                            } else {
                                // Apply rename
                                let new_name = self.rename_buffer.trim().to_string();
                                if !new_name.is_empty() {
                                    self.contexts[i].name = new_name;
                                }
                                self.renaming_context = None;
                            }
                            // Consume Enter/Escape so it doesn't leak to the terminal
                            ui.input_mut(|i| {
                                i.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
                                i.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
                            });
                        }
                        // Auto-focus and select all on first frame
                        if te.gained_focus() || !te.has_focus() {
                            te.request_focus();
                            if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), te_id) {
                                state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                                    egui::text::CCursor::new(0),
                                    egui::text::CCursor::new(self.rename_buffer.len()),
                                )));
                                state.store(ui.ctx(), te_id);
                            }
                        }
                    } else {
                        let text_color = if is_active {
                            Colors::TEXT_PRIMARY
                        } else {
                            Colors::TEXT_DIM
                        };
                        ui.label(
                            RichText::new(&self.contexts[i].name)
                                .size(12.0)
                                .color(text_color),
                        );

                        // Delete button on hover when 2+ contexts
                        if hover && num_contexts > 1 {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.add_space(8.0);
                                if ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new("\u{2715}")
                                                .size(10.0)
                                                .color(Colors::TEXT_DIM),
                                        )
                                        .frame(false),
                                    )
                                    .on_hover_text("Delete context")
                                    .clicked()
                                {
                                    delete_context = Some(i);
                                }
                            });
                        }
                    }
                },
            );

            // Interact on the full row rect for click/right-click sensing
            let row_rect = response.response.rect;
            let row_response = ui.interact(row_rect, egui::Id::new(("ctx_row", i)), egui::Sense::click());

            if !is_renaming {
                row_response.context_menu(|ui| {
                    if ui.button("Rename").clicked() {
                        menu_action = Some((i, ContextMenuAction::Rename));
                        ui.close_menu();
                    }
                    ui.separator();
                    if i > 0 {
                        if ui.button("Move to Top").clicked() {
                            menu_action = Some((i, ContextMenuAction::MoveToTop));
                            ui.close_menu();
                        }
                        if ui.button("Move Up").clicked() {
                            menu_action = Some((i, ContextMenuAction::MoveUp));
                            ui.close_menu();
                        }
                    }
                    if i < num_contexts - 1 {
                        if ui.button("Move Down").clicked() {
                            menu_action = Some((i, ContextMenuAction::MoveDown));
                            ui.close_menu();
                        }
                    }
                    if num_contexts > 1 {
                        ui.separator();
                        if ui.button("Delete").clicked() {
                            menu_action = Some((i, ContextMenuAction::Delete));
                            ui.close_menu();
                        }
                    }
                });

                if row_response.double_clicked() {
                    double_clicked_context = Some(i);
                } else if row_response.clicked() {
                    clicked_context = Some(i);
                }
            }
        }

        // Handle collected actions after the loop
        // Handle context menu actions
        if let Some((i, action)) = menu_action {
            match action {
                ContextMenuAction::Rename => {
                    self.renaming_context = Some(i);
                    self.rename_buffer = self.contexts[i].name.clone();
                }
                ContextMenuAction::MoveToTop => {
                    let ctx = self.contexts.remove(i);
                    self.contexts.insert(0, ctx);
                    if self.active_context == i {
                        self.active_context = 0;
                    } else if self.active_context < i {
                        self.active_context += 1;
                    }
                }
                ContextMenuAction::MoveUp => {
                    self.contexts.swap(i, i - 1);
                    if self.active_context == i {
                        self.active_context = i - 1;
                    } else if self.active_context == i - 1 {
                        self.active_context = i;
                    }
                }
                ContextMenuAction::MoveDown => {
                    self.contexts.swap(i, i + 1);
                    if self.active_context == i {
                        self.active_context = i + 1;
                    } else if self.active_context == i + 1 {
                        self.active_context = i;
                    }
                }
                ContextMenuAction::Delete => {
                    self.delete_context(i);
                }
            }
        } else if let Some(i) = delete_context {
            self.delete_context(i);
        } else if let Some(i) = double_clicked_context {
            self.renaming_context = Some(i);
            self.rename_buffer = self.contexts[i].name.clone();
        } else if let Some(i) = clicked_context {
            self.active_context = i;
        }

        if add_clicked {
            self.new_context();
        }
    }

    fn draw_shortcuts_overlay(&self, ctx: &egui::Context) {
        egui::Area::new(egui::Id::new("shortcuts_overlay"))
            .anchor(Align2::RIGHT_TOP, Vec2::new(-16.0, 44.0))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(Colors::BG_SIDEBAR)
                    .stroke(Stroke::new(1.0, Colors::BORDER))
                    .corner_radius(R6)
                    .inner_margin(egui::Margin::symmetric(16, 12))
                    .show(ui, |ui| {
                        ui.set_width(240.0);
                        ui.label(
                            RichText::new("Keyboard Shortcuts")
                                .size(13.0)
                                .color(Colors::TEXT_PRIMARY)
                                .strong(),
                        );
                        ui.add_space(8.0);

                        let shortcuts = [
                            ("\u{2318}T", "New tab"),
                            ("\u{2318}]/[", "Next/prev tab"),
                            ("\u{2318}D", "Split right"),
                            ("\u{2318}\u{21E7}D", "Split down"),
                            ("\u{2318}W", "Close pane"),
                            ("\u{2318}B", "Toggle sidebar"),
                            ("\u{2318}H/J/K/L", "Focus pane"),
                            ("\u{2318}\u{21A9}", "Zoom pane"),
                            ("\u{2318}1-9", "Switch context"),
                            ("\u{2318}/", "This help"),
                            ("\u{2318}Q", "Quit"),
                        ];

                        for (key, desc) in shortcuts {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(key)
                                        .size(11.0)
                                        .color(Colors::ACCENT)
                                        .family(egui::FontFamily::Monospace),
                                );
                                ui.add_space(8.0);
                                ui.label(
                                    RichText::new(desc)
                                        .size(11.0)
                                        .color(Colors::TEXT_DIM),
                                );
                            });
                        }
                    });
            });
    }
}

fn replace_child(container: &mut Container, old: TileId, new: TileId) {
    match container {
        Container::Linear(linear) => {
            if let Some(pos) = linear.children.iter().position(|&c| c == old) {
                linear.children[pos] = new;
            }
        }
        Container::Tabs(tabs) => {
            if let Some(pos) = tabs.children.iter().position(|&c| c == old) {
                tabs.children[pos] = new;
            }
        }
        Container::Grid(_) => {
            container.remove_child(old);
            container.add_child(new);
        }
    }
}
