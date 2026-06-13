//! Multi-context and workspace persistence: new context, reset, delete,
//! and on-disk workspace save.

use crate::app::PlexiApp;
use crate::host::context::Window;
use crate::host::shell;
use crate::spatial::tiling::PaneId;
use crate::workspace::WorkspaceFile;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// Auto-initialize a project workspace at `root` if one does not already exist.
///
/// Calls the shared `init_workspace` function which creates all workspace files
/// (workspace.toml, secrets.toml, apps.toml, commands.toml) under the channel dir.
///
/// Idempotent — skips if the channel dir already has workspace.toml.
/// Non-fatal — logs warn on failure but never prevents the root from being set.
fn auto_init_workspace(root: &std::path::Path) {
    // Guard: never init at home dir, filesystem root, or inside a Plexi profile dir.
    let home = dirs::home_dir();
    let root_str = root.to_string_lossy();
    let is_home_or_root =
        root == std::path::Path::new("/") || home.as_ref().map(|h| root == *h).unwrap_or(false);
    let is_inside_profile = home
        .as_ref()
        .map(|h| {
            let prefix = format!("{}/.plexi", h.to_string_lossy());
            root_str.starts_with(&prefix)
        })
        .unwrap_or(false);
    if is_home_or_root || is_inside_profile {
        log::info!(
            "auto_init_workspace: skipping home/root/profile path {}",
            root.display()
        );
        return;
    }

    let channel_dir = crate::config::workspace_channel_dir();
    let channel_path = root.join(&channel_dir);
    if channel_path.exists() {
        log::info!(
            "auto_init_workspace: already initialized at {}",
            root.display()
        );
        return;
    }
    match crate::workspace::secrets::init_workspace(root, &channel_dir) {
        Ok(cfg) => log::info!(
            "auto_init_workspace: created {}/{channel_dir} workspace_id={}",
            root.display(),
            cfg.id
        ),
        Err(e) => log::warn!(
            "auto_init_workspace: could not create {}/{channel_dir}: {e}",
            root.display()
        ),
    }
}

fn persisted_next_pane_id(host_next: u64, windows: &[crate::workspace::SavedWindow]) -> u64 {
    let next_from_saved = windows
        .iter()
        .flat_map(|win| win.panes.iter().map(|pane| pane.id))
        .max()
        .map(|id| id.saturating_add(1))
        .unwrap_or(host_next);
    host_next.max(next_from_saved)
}

fn two_windows_mut(
    windows: &mut [Window],
    first: usize,
    second: usize,
) -> (&mut Window, &mut Window) {
    debug_assert_ne!(first, second, "cannot borrow the same window twice");
    if first < second {
        let (left, right) = windows.split_at_mut(second);
        (&mut left[first], &mut right[0])
    } else {
        let (left, right) = windows.split_at_mut(first);
        (&mut right[0], &mut left[second])
    }
}

fn clone_tile_subtree(
    source_tiles: &egui_tiles::Tiles<PaneId>,
    source_id: egui_tiles::TileId,
    dest_tiles: &mut egui_tiles::Tiles<PaneId>,
    tile_map: &mut HashMap<egui_tiles::TileId, egui_tiles::TileId>,
) -> Option<egui_tiles::TileId> {
    if let Some(mapped) = tile_map.get(&source_id) {
        return Some(*mapped);
    }

    let tile = source_tiles.get(source_id)?;
    let new_id = match tile {
        egui_tiles::Tile::Pane(pane_id) => dest_tiles.insert_pane(*pane_id),
        egui_tiles::Tile::Container(egui_tiles::Container::Linear(linear)) => {
            let child_pairs: Option<Vec<_>> = linear
                .children
                .iter()
                .map(|&child| {
                    clone_tile_subtree(source_tiles, child, dest_tiles, tile_map)
                        .map(|new_child| (child, new_child))
                })
                .collect();
            let child_pairs = child_pairs?;
            let new_children: Vec<_> = child_pairs
                .iter()
                .map(|(_, new_child)| *new_child)
                .collect();
            let mut new_linear = egui_tiles::Linear::new(linear.dir, new_children);
            for (old_child, new_child) in child_pairs {
                new_linear
                    .shares
                    .set_share(new_child, linear.shares[old_child]);
            }
            dest_tiles.insert_container(new_linear)
        }
        egui_tiles::Tile::Container(egui_tiles::Container::Tabs(tabs)) => {
            let child_pairs: Option<Vec<_>> = tabs
                .children
                .iter()
                .map(|&child| {
                    clone_tile_subtree(source_tiles, child, dest_tiles, tile_map)
                        .map(|new_child| (child, new_child))
                })
                .collect();
            let child_pairs = child_pairs?;
            let new_children: Vec<_> = child_pairs
                .iter()
                .map(|(_, new_child)| *new_child)
                .collect();
            let mut new_tabs = egui_tiles::Tabs::new(new_children);
            new_tabs.active = tabs.active.and_then(|active| {
                child_pairs
                    .iter()
                    .find_map(|(old_child, new_child)| (*old_child == active).then_some(*new_child))
            });
            dest_tiles.insert_container(new_tabs)
        }
        egui_tiles::Tile::Container(egui_tiles::Container::Grid(grid)) => {
            let child_pairs: Option<Vec<_>> = grid
                .children()
                .copied()
                .map(|child| {
                    clone_tile_subtree(source_tiles, child, dest_tiles, tile_map)
                        .map(|new_child| (child, new_child))
                })
                .collect();
            let child_pairs = child_pairs?;
            let new_children: Vec<_> = child_pairs
                .iter()
                .map(|(_, new_child)| *new_child)
                .collect();
            let mut new_grid = egui_tiles::Grid::new(new_children);
            new_grid.layout = grid.layout;
            new_grid.col_shares = grid.col_shares.clone();
            new_grid.row_shares = grid.row_shares.clone();
            dest_tiles.insert_container(new_grid)
        }
    };

    tile_map.insert(source_id, new_id);
    Some(new_id)
}

fn map_focus_tile(
    tile_map: &HashMap<egui_tiles::TileId, egui_tiles::TileId>,
    source_tile: Option<egui_tiles::TileId>,
) -> Option<egui_tiles::TileId> {
    source_tile.and_then(|tile| tile_map.get(&tile).copied())
}

fn reserve_grid_slot(
    occupied: &mut HashSet<(u32, u32)>,
    preferred_x: u32,
    preferred_y: u32,
) -> (u32, u32) {
    if occupied.insert((preferred_x, preferred_y)) {
        return (preferred_x, preferred_y);
    }

    let mut x = 0;
    loop {
        if occupied.insert((x, preferred_y)) {
            return (x, preferred_y);
        }
        x += 1;
    }
}

impl PlexiApp {
    /// Create a child context nested inside `parent_name`. Always creates a fresh
    /// terminal in the child (portal model — no pane adoption). Inserts a Portal
    /// tile into the parent window as a sibling of the focused tile. No depth cap.
    pub(crate) fn new_child_context(
        &mut self,
        parent_name: &str,
        path: PathBuf,
        vertical: bool,
        new_pane_first: bool,
        anchor_pane: Option<u64>,
    ) -> Result<(), String> {
        let parent_idx = self
            .router
            .position(|c| c.name.eq_ignore_ascii_case(parent_name))
            .ok_or_else(|| format!("no context named '{parent_name}'"))?;
        let parent_id = self.router.get(parent_idx).context_id;
        let parent_depth = self.router.get(parent_idx).depth;
        let child_depth = parent_depth + 1;

        let ctx_id = self.next_window_id;
        self.next_window_id += 1;
        let win_id = self.next_window_id;
        self.next_window_id += 1;

        // Check for anchor defaults from .plexi/workspace.toml [context] section.
        let anchor = crate::host::anchor::Anchor::detect(&path);
        let (ctx_name, ctx_description) =
            match anchor.as_ref().and_then(|a| a.context_defaults.as_ref()) {
                Some(defaults) => {
                    let name = defaults.name.clone().unwrap_or_else(|| {
                        path.file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| format!("Sub-context {}", self.router.len() + 1))
                    });
                    (name, defaults.description.clone())
                }
                None => {
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| format!("Sub-context {}", self.router.len() + 1));
                    (name, None)
                }
            };

        log::info!(
            "new_child_context: parent_id={parent_id} parent_depth={parent_depth} \
             child_depth={child_depth} name={ctx_name} path={}",
            path.display()
        );

        // 1. Build the child window with a fresh terminal.
        let Some((child_tree, child_panes, child_root_tile)) =
            self.create_single_pane_tree(Some(path.clone()), None, false)
        else {
            log::error!("new_child_context: failed to create terminal for child context");
            return Err("failed to create terminal for child context".to_string());
        };

        // 2. Insert Portal tile into the parent window via the standard split path.
        // An explicit anchor pane (the CLI caller's pane, from --pane or
        // PLEXI_PANE_ID) selects both the parent window and the split target;
        // otherwise fall back to the parent's active window and its focused pane.
        let portal_anchor = anchor_pane.and_then(|pid| {
            self.windows
                .iter()
                .position(|w| w.context_id == parent_id && w.tree.tiles.find_pane(&pid).is_some())
                .map(|idx| (idx, pid))
        });
        if anchor_pane.is_some() && portal_anchor.is_none() {
            log::warn!(
                "new_child_context: anchor pane {anchor_pane:?} not found in parent \
                 ctx_id={parent_id} — falling back to focused pane"
            );
        }
        let parent_win_idx = portal_anchor.map(|(idx, _)| idx).or_else(|| {
            let preferred = self.context_active_window.get(&parent_id).copied();
            preferred
                .and_then(|wid| {
                    self.windows
                        .iter()
                        .position(|w| w.window_id == wid && w.context_id == parent_id)
                })
                .or_else(|| self.windows.iter().position(|w| w.context_id == parent_id))
        });
        let sub_ctx_pane_id = self.host.alloc_pane_id();
        if let Some(parent_win_idx) = parent_win_idx {
            let split_target = portal_anchor
                .and_then(|(_, pid)| self.windows[parent_win_idx].tree.tiles.find_pane(&pid))
                .or(self.windows[parent_win_idx].focused_pane);
            crate::pane_ops::layout::insert_split_tile(
                &mut self.windows[parent_win_idx].tree,
                split_target,
                sub_ctx_pane_id,
                vertical,
                crate::host::command::ShareRatio {
                    numerator: 1.0,
                    denominator: 1.0,
                },
                new_pane_first,
            );
            self.windows[parent_win_idx].panes.insert(
                sub_ctx_pane_id,
                crate::host::pane::Pane::Portal(Box::new(crate::host::pane::PortalPane {
                    pane_id: sub_ctx_pane_id,
                    target_context_id: ctx_id,
                    context_state: None,
                    hidden: false,
                })),
            );
        } else {
            log::warn!("new_child_context: parent ctx_id={parent_id} has no window — child context has no Portal tile");
        }

        // 3. Register the child context + window.
        self.router.push(crate::host::context::Context {
            name: ctx_name,
            path: path.clone(),
            root: Some(path.clone()),
            description: ctx_description,
            context_id: ctx_id,
            parent_id: Some(parent_id),
            depth: child_depth,
            parked: false,
        });
        self.windows.push(crate::host::context::Window {
            name: String::new(),
            path,
            tree: child_tree,
            panes: child_panes,
            focused_pane: Some(child_root_tile),
            zoomed_pane: None,
            grid_x: 0,
            grid_y: 0,
            window_id: win_id,
            context_id: ctx_id,
        });
        self.context_active_window.insert(ctx_id, win_id);

        self.save_workspace();
        Ok(())
    }

    /// Create a new empty child context under the current context and auto-zoom
    /// into it. Bound to Cmd+Shift+Option+N. Uses the current context's path as
    /// the child's working directory and names the child "Sub-context N".
    pub(crate) fn new_child_context_from_keyboard(&mut self) {
        let parent_ctx_id = self.router.active().context_id;
        let parent_name = self.router.active().name.clone();
        let parent_path = self.router.active().path.clone();
        let current_win_id = self.windows[self.active_window].window_id;
        let current_focused = self.windows[self.active_window].focused_pane;

        log::info!(
            "new_child_context_from_keyboard: parent_ctx_id={parent_ctx_id} parent_name={parent_name} path={}",
            parent_path.display()
        );

        match self.new_child_context(&parent_name, parent_path, true, false, None) {
            Ok(()) => {
                let new_ctx_idx = self.router.len() - 1;
                self.router
                    .push_depth(parent_ctx_id, current_win_id, current_focused);
                self.switch_workspace(new_ctx_idx);
                log::info!(
                    "new_child_context_from_keyboard: zoomed into child ctx_id={}",
                    self.router.active().context_id
                );
            }
            Err(e) => {
                log::warn!("new_child_context_from_keyboard: failed to create child context: {e}");
            }
        }
    }

    pub(crate) fn push_pane_to_subcontext(&mut self, name: Option<String>) {
        let parent_win_idx = self.active_window;
        let parent_ctx_id = self.windows[parent_win_idx].context_id;
        let parent_depth = self
            .router
            .iter()
            .find(|c| c.context_id == parent_ctx_id)
            .map(|c| c.depth)
            .unwrap_or(0);

        let Some(focused_tile) = self.windows[parent_win_idx].focused_pane else {
            log::warn!("push_pane_to_subcontext: no focused pane");
            return;
        };

        let pane_id = match self.windows[parent_win_idx].tree.tiles.get(focused_tile) {
            Some(egui_tiles::Tile::Pane(pid)) => *pid,
            _ => {
                log::warn!("push_pane_to_subcontext: focused tile is not a pane");
                return;
            }
        };

        if self.windows[parent_win_idx]
            .panes
            .get(&pane_id)
            .map(|p| p.as_portal().is_some())
            .unwrap_or(false)
        {
            log::warn!("push_pane_to_subcontext: cannot push a portal pane");
            return;
        }

        let pane_name = self.windows[parent_win_idx]
            .panes
            .get(&pane_id)
            .and_then(|p| {
                p.as_terminal()
                    .and_then(|t| t.name.clone())
                    .or_else(|| p.as_app().map(|a| a.name.clone()))
            });
        let ctx_name = name
            .or(pane_name)
            .unwrap_or_else(|| format!("Sub-context {}", self.router.len() + 1));

        let (parent_path, parent_root) = self
            .router
            .iter()
            .find(|c| c.context_id == parent_ctx_id)
            .map(|c| (c.path.clone(), c.root.clone()))
            .unwrap_or_else(|| {
                let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
                (home.clone(), Some(home))
            });

        let ctx_id = self.next_window_id;
        self.next_window_id += 1;
        let win_id = self.next_window_id;
        self.next_window_id += 1;
        let child_depth = parent_depth + 1;

        log::info!(
            "push_pane_to_subcontext: pane_id={pane_id} ctx_name={ctx_name} \
             parent_ctx_id={parent_ctx_id} child_depth={child_depth}"
        );

        let pane = match self.windows[parent_win_idx].panes.remove(&pane_id) {
            Some(p) => p,
            None => {
                log::error!("push_pane_to_subcontext: pane not found in parent window");
                return;
            }
        };

        let portal_pane_id = self.host.alloc_pane_id();
        if let Some(egui_tiles::Tile::Pane(slot)) = self.windows[parent_win_idx]
            .tree
            .tiles
            .get_mut(focused_tile)
        {
            *slot = portal_pane_id;
        }
        self.windows[parent_win_idx].panes.insert(
            portal_pane_id,
            crate::host::pane::Pane::Portal(Box::new(crate::host::pane::PortalPane {
                pane_id: portal_pane_id,
                target_context_id: ctx_id,
                context_state: None,
                hidden: false,
            })),
        );

        let mut child_tiles = egui_tiles::Tiles::default();
        let child_tile_id = child_tiles.insert_pane(pane_id);
        let child_tree = egui_tiles::Tree::new("child_tree", child_tile_id, child_tiles);
        let child_root_tile = child_tree.root.unwrap();
        let mut child_panes = std::collections::HashMap::new();
        child_panes.insert(pane_id, pane);

        self.router.push(crate::host::context::Context {
            name: ctx_name,
            path: parent_path.clone(),
            root: parent_root,
            description: None,
            context_id: ctx_id,
            parent_id: Some(parent_ctx_id),
            depth: child_depth,
            parked: false,
        });
        self.windows.push(Window {
            name: String::new(),
            path: parent_path,
            tree: child_tree,
            panes: child_panes,
            focused_pane: Some(child_root_tile),
            zoomed_pane: None,
            grid_x: 0,
            grid_y: 0,
            window_id: win_id,
            context_id: ctx_id,
        });
        self.context_active_window.insert(ctx_id, win_id);

        let current_win_id = self.windows[parent_win_idx].window_id;
        self.router
            .push_depth(parent_ctx_id, current_win_id, Some(focused_tile));
        let new_ctx_idx = self.router.len() - 1;
        self.switch_workspace(new_ctx_idx);

        self.save_workspace();
    }

    pub(crate) fn new_context(&mut self) {
        log::info!("new_context: creating new top-level context");
        self.new_context_empty();
    }

    /// Create a new standalone empty context at the home directory.
    fn new_context_empty(&mut self) {
        let cwd = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        log::info!("new_context_empty: cwd={}", cwd.display());
        let Some((tree, panes, root_tile)) =
            self.create_single_pane_tree(Some(cwd.clone()), None, false)
        else {
            log::error!("new_context_empty: failed to create terminal");
            return;
        };

        let ctx_id = self.next_window_id;
        self.next_window_id += 1;
        let win_id = self.next_window_id;
        self.next_window_id += 1;

        let ctx_name = format!("Context {}", self.router.len() + 1);
        self.router.push(crate::host::context::Context {
            name: ctx_name,
            path: cwd.clone(),
            root: Some(cwd.clone()),
            description: None,
            context_id: ctx_id,
            parent_id: None,
            depth: 0,
            parked: false,
        });
        self.windows.push(Window {
            name: String::new(),
            path: cwd,
            tree,
            panes,
            focused_pane: Some(root_tile),
            zoomed_pane: None,
            grid_x: 0,
            grid_y: 0,
            window_id: win_id,
            context_id: ctx_id,
        });
        self.router.activate_last();
        self.active_window = self.windows.len() - 1;
        self.context_active_window.insert(ctx_id, win_id);
        self.minimap.visible = false;
        self.apply_context_transition_effects();

        let new_ctx_idx = self.router.len() - 1;
        self.open_context_rename(new_ctx_idx);
        self.save_workspace();
        log::info!(
            "new_context_empty: emitting ContextCreated context_id={ctx_id} name={}",
            self.rename_buffer
        );
        crate::host::event_log::emit(crate::host::event_log::HostEvent::ContextCreated {
            context_id: ctx_id,
            name: self.rename_buffer.clone(),
            timestamp: crate::host::event_log::now_timestamp(),
        });
    }

    /// Create a new context at a specific directory path. The terminal pane
    /// opens at `path` and the context root is set to it. Named after the
    /// directory basename, unless the path has a `.plexi/workspace.toml` with
    /// `[context]` defaults. Callers must call `save_workspace()` afterward.
    pub(crate) fn new_context_at_path(&mut self, path: PathBuf) {
        log::info!("new_context_at_path: path={}", path.display());
        let Some((tree, panes, root_tile)) =
            self.create_single_pane_tree(Some(path.clone()), None, false)
        else {
            log::error!(
                "new_context_at_path: failed to create terminal for {}",
                path.display()
            );
            return;
        };

        let ctx_id = self.next_window_id;
        self.next_window_id += 1;
        let win_id = self.next_window_id;
        self.next_window_id += 1;

        // Check for anchor defaults from .plexi/workspace.toml [context] section.
        let anchor = crate::host::anchor::Anchor::detect(&path);
        let (ctx_name, ctx_description) =
            match anchor.as_ref().and_then(|a| a.context_defaults.as_ref()) {
                Some(defaults) => {
                    let name = defaults.name.clone().unwrap_or_else(|| {
                        path.file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| format!("Context {}", self.router.len() + 1))
                    });
                    log::info!(
                        "new_context_at_path: applying anchor defaults name={:?} description={:?}",
                        name,
                        defaults.description
                    );
                    (name, defaults.description.clone())
                }
                None => {
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| format!("Context {}", self.router.len() + 1));
                    (name, None)
                }
            };

        self.router.push(crate::host::context::Context {
            name: ctx_name,
            path: path.clone(),
            root: Some(path.clone()),
            description: ctx_description,
            context_id: ctx_id,
            parent_id: None,
            depth: 0,
            parked: false,
        });
        self.windows.push(Window {
            name: String::new(),
            path,
            tree,
            panes,
            focused_pane: Some(root_tile),
            zoomed_pane: None,
            grid_x: 0,
            grid_y: 0,
            window_id: win_id,
            context_id: ctx_id,
        });
        self.router.activate_last();
        self.active_window = self.windows.len() - 1;
        self.context_active_window.insert(ctx_id, win_id);
        self.minimap.visible = false;
        self.apply_context_transition_effects();
    }

    /// Create a new page immediately to the right of the active page on the
    /// same grid row, then switch to it.
    pub(crate) fn new_page_right(&mut self) {
        let ws_id = self.router.active().context_id;
        let active_y = self.windows[self.active_window].grid_y;
        let max_x = self
            .windows
            .iter()
            .filter(|c| c.context_id == ws_id && c.grid_y == active_y)
            .map(|c| c.grid_x)
            .max();
        let new_x = match max_x {
            Some(x) => x + 1,
            None => 1,
        };
        self.create_page_at(new_x, active_y, None, false);
    }

    /// Shared creation helper: create a single-pane context at `(grid_x, grid_y)`
    /// and make it the active context.
    pub(crate) fn create_page_at(
        &mut self,
        grid_x: u32,
        grid_y: u32,
        initial_cmd: Option<&str>,
        close_on_exit: bool,
    ) {
        let old_window_id = self.windows[self.active_window].window_id;
        let old_focus = self.windows[self.active_window].focused_pane;
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let cwd = self
            .resolve_new_pane_cwd(None, self.windows[self.active_window].focused_pane)
            .filter(|p| p != &PathBuf::from("/"))
            .unwrap_or(home);
        log::info!("create_page_at({grid_x},{grid_y}): cwd={} context_root={:?} initial_cmd={initial_cmd:?} close_on_exit={close_on_exit}", cwd.display(), self.router.active().root);
        let Some((tree, panes, root_tile)) =
            self.create_single_pane_tree(Some(cwd.clone()), initial_cmd, close_on_exit)
        else {
            log::error!("Failed to create terminal for new page at ({grid_x}, {grid_y})");
            return;
        };
        let name = String::new();
        let ctx_id = self.router.active().context_id;
        let win_id = self.next_window_id;
        self.next_window_id += 1;
        self.push_focus_history(old_window_id, old_focus);
        self.windows.push(Window {
            name,
            path: cwd,
            tree,
            panes,
            focused_pane: Some(root_tile),
            zoomed_pane: None,
            grid_x,
            grid_y,
            window_id: win_id,
            context_id: ctx_id,
        });
        self.active_window = self.windows.len() - 1;
        self.context_active_window.insert(ctx_id, win_id);
        self.minimap.visible = true;
    }

    pub(crate) fn reset_active_context(&mut self) {
        let cwd = self.cwd_for_welcome_tab();
        log::info!(
            "reset_active_context: cwd={} context_root={:?}",
            cwd.display(),
            self.router.active().root
        );
        let Some((tree, panes, root_tile)) = self.create_single_pane_tree(Some(cwd), None, false)
        else {
            log::error!("Failed to create terminal for reset context");
            return;
        };

        let ctx = &mut self.windows[self.active_window];
        ctx.tree = tree;
        ctx.panes = panes;
        ctx.focused_pane = Some(root_tile);
        ctx.zoomed_pane = None;
    }

    pub(crate) fn delete_context(&mut self, ws_index: usize) {
        if self.router.len() <= 1 {
            return;
        }
        let target_ctx_id = self.router.get(ws_index).context_id;

        // 1+2. Collect target + all descendants (BFS via parent_id).
        let mut deleted: Vec<u64> = vec![target_ctx_id];
        let mut frontier: Vec<u64> = vec![target_ctx_id];
        while let Some(cur) = frontier.pop() {
            for ctx in self.router.iter() {
                if ctx.parent_id == Some(cur) && !deleted.contains(&ctx.context_id) {
                    deleted.push(ctx.context_id);
                    frontier.push(ctx.context_id);
                }
            }
        }
        log::info!(
            "delete_context: cascading delete of ctx_id={target_ctx_id} + {} descendants ({:?})",
            deleted.len() - 1,
            &deleted[1..]
        );

        // 3. Remove all windows belonging to any deleted context.
        self.windows.retain(|c| !deleted.contains(&c.context_id));

        // 4. Remove Portal tiles in surviving windows that point to any deleted ctx.
        for win in &mut self.windows {
            let portal_pane_ids: Vec<crate::spatial::tiling::PaneId> = win
                .panes
                .iter()
                .filter(|(_, p)| {
                    p.portal_target()
                        .map(|cid| deleted.contains(&cid))
                        .unwrap_or(false)
                })
                .map(|(id, _)| *id)
                .collect();
            for pane_id in portal_pane_ids {
                win.panes.remove(&pane_id);
                if let Some(tile_id) = win.tree.tiles.find_pane(&pane_id) {
                    win.tree.remove_recursively(tile_id);
                }
            }
        }

        // 4b. Delete surviving windows that are now empty (portal was their only pane),
        //     but only if a non-empty sibling window exists for the same context. This
        //     preserves the invariant that every context retains at least one window.
        {
            let mut empty_indices: Vec<usize> = self
                .windows
                .iter()
                .enumerate()
                .filter(|(_, w)| w.panes.is_empty())
                .filter(|(_, w)| {
                    self.windows
                        .iter()
                        .any(|o| o.context_id == w.context_id && !o.panes.is_empty())
                })
                .map(|(i, _)| i)
                .collect();
            empty_indices.sort_unstable_by(|a, b| b.cmp(a)); // reverse order
            for idx in empty_indices {
                log::info!(
                    "delete_context: window idx={idx} is empty after portal removal — deleting it",
                );
                self.delete_window(idx);
            }
        }

        // 4c. Reset any focused_pane that now points to a removed Portal tile.
        for win in &mut self.windows {
            if let Some(fp) = win.focused_pane {
                if win.tree.tiles.get(fp).is_none() {
                    win.focused_pane = win.tree.root.and_then(|root| win.find_first_pane_in(root));
                    log::info!(
                        "delete_context: stale focused_pane reset for window ctx_id={}",
                        win.context_id
                    );
                }
            }
        }

        // 5. Remove from router. Iterate until none remain (positions shift after each removal).
        loop {
            let next = self.router.position(|c| deleted.contains(&c.context_id));
            match next {
                Some(idx) => {
                    self.router.remove_at(idx);
                }
                None => break,
            }
        }

        // 6. Clean depth_stack: drop entries pointing to any deleted context.
        let before = self.router.depth_stack.len();
        self.router
            .retain_depth_stack(|cid| !deleted.contains(&cid));
        let cleaned = before - self.router.depth_stack.len();
        if cleaned > 0 {
            log::info!("delete_context: removed {cleaned} stale depth_stack entries");
        }

        // 7. Pick a valid active window in the new active context.
        self.pick_active_context_from_workspace();

        // 8. Notifications scoped to any deleted context are dropped.
        self.pending_notifications.retain(|n| {
            !(matches!(n.scope, crate::app_protocol::NotifyScope::Context)
                && deleted.contains(&n.source_context_id))
        });
        self.save_notifications();
        if let Some(ref id) = self.current_notify_id.clone() {
            let still_present = self
                .pending_notifications
                .iter()
                .any(|n| &n.notify_id == id);
            if !still_present {
                self.current_notify_id = None;
            }
        }

        // 9. Restore minimap state for the context we landed on.
        let new_ws_id = self.router.active().context_id;
        let page_count = self
            .windows
            .iter()
            .filter(|c| c.context_id == new_ws_id)
            .count();
        self.minimap.visible = self
            .minimap_visible_per_context
            .get(&new_ws_id)
            .copied()
            .unwrap_or(page_count > 1);
    }

    /// Pop the depth stack and return to the parent context/window/focus.
    /// Shared body of Cmd+Escape (`Action::ContextZoomOut`) and the
    /// `ZoomOutOfContext` IPC request. Returns true when a switch happened.
    pub(crate) fn zoom_out_of_context(&mut self) -> bool {
        if let Some((parent_ctx_id, parent_win_id, focused_tile)) = self.router.pop_depth() {
            if let Some(ctx_idx) = self.router.position(|c| c.context_id == parent_ctx_id) {
                self.switch_workspace(ctx_idx);
                if let Some(win_idx) = self
                    .windows
                    .iter()
                    .position(|w| w.window_id == parent_win_id)
                {
                    self.active_window = win_idx;
                    self.windows[win_idx].focused_pane = focused_tile;
                }
                return true;
            }
        }
        false
    }

    /// If `ctx_id` names a subcontext whose windows are now all empty, delete
    /// it. When it was the active context, first zoom back out to the parent
    /// exactly as if Cmd+Escape had been pressed. Root contexts are never
    /// collapsed — their sole window stays alive as the welcome screen.
    pub(crate) fn collapse_subcontext_if_empty(&mut self, ctx_id: u64) {
        if self.router.len() <= 1 {
            return;
        }
        let Some(ctx_idx) = self.router.position(|c| c.context_id == ctx_id) else {
            return;
        };
        let Some(parent_ctx_id) = self.router.get(ctx_idx).parent_id else {
            return;
        };
        if self
            .windows
            .iter()
            .any(|w| w.context_id == ctx_id && !w.panes.is_empty())
        {
            return;
        }
        let was_active = self.router.active().context_id == ctx_id;
        log::info!(
            "collapse_subcontext_if_empty: subcontext {ctx_id} emptied — deleting \
             (was_active={was_active}, parent={parent_ctx_id})"
        );
        if was_active {
            let zoomed_out = self.zoom_out_of_context();
            if !zoomed_out || self.router.active().context_id == ctx_id {
                // No usable depth-stack entry — land on the parent directly.
                if let Some(parent_idx) =
                    self.router.position(|c| c.context_id == parent_ctx_id)
                {
                    self.switch_workspace(parent_idx);
                }
            }
        }
        if let Some(idx) = self.router.position(|c| c.context_id == ctx_id) {
            self.delete_context(idx);
        }
    }

    pub(crate) fn delete_window(&mut self, index: usize) {
        if self.windows.len() <= 1 {
            return;
        }
        let removed_ws_id = self.windows[index].context_id;
        let removed_win_id = self.windows[index].window_id;
        let was_active = self.active_window == index;
        let removed_x = self.windows[index].grid_x;
        let removed_y = self.windows[index].grid_y;

        // Save current minimap state before deletion so it's always fresh.
        self.minimap_visible_per_context
            .insert(removed_ws_id, self.minimap.visible);

        self.windows.remove(index);

        // If the deleted window was the stored last-visited for its context,
        // point to another window in the same context so the palette doesn't
        // navigate to a ghost window_id.
        if self.context_active_window.get(&removed_ws_id) == Some(&removed_win_id) {
            if let Some(replacement) = self.windows.iter().find(|w| w.context_id == removed_ws_id) {
                self.context_active_window
                    .insert(removed_ws_id, replacement.window_id);
            } else {
                self.context_active_window.remove(&removed_ws_id);
            }
        }

        // If context now has no windows, remove it too.
        let ws_has_windows = self.windows.iter().any(|c| c.context_id == removed_ws_id);
        if !ws_has_windows {
            if let Some(ws_idx) = self.router.position(|w| w.context_id == removed_ws_id) {
                self.router.remove_at(ws_idx);
            }
        }

        if was_active {
            self.active_window = self.nearest_context_after_delete(removed_x, removed_y);
            // Sync router active to the new active window's context.
            let new_ctx_id = self.windows[self.active_window].context_id;
            if let Some(ctx_idx) = self.router.position(|w| w.context_id == new_ctx_id) {
                self.router.set_active(ctx_idx);
                self.context_active_window
                    .insert(new_ctx_id, self.windows[self.active_window].window_id);
            }
        } else if self.active_window >= self.windows.len() {
            self.active_window = self.windows.len() - 1;
        } else if self.active_window > index {
            self.active_window -= 1;
        }

        if self.renaming_window == Some(index) {
            self.renaming_window = None;
        } else if let Some(r) = self.renaming_window {
            if r > index {
                self.renaming_window = Some(r - 1);
            }
        }

        // ── Compact the grid: shift columns left if a column becomes empty ──
        self.compact_workspace_grid(removed_ws_id);

        // Minimap: if we stayed in the same context, preserve the current
        // visibility state (the render loop hides it when page_count < 2). If
        // the deletion caused a context switch (context was fully deleted), restore
        // the saved state for the new context.
        let ws_id = self.router.active().context_id;
        if ws_id != removed_ws_id {
            let page_count = self
                .windows
                .iter()
                .filter(|c| c.context_id == ws_id)
                .count();
            self.minimap.visible = self
                .minimap_visible_per_context
                .get(&ws_id)
                .copied()
                .unwrap_or(page_count > 1);
        }
    }

    /// After a deletion, compact each row independently: within each row,
    /// shift windows left to close any gaps in grid_x. This ensures that
    /// deleting the left-most window in a row causes the rest to slide over.
    fn compact_workspace_grid(&mut self, ctx_id: u64) {
        // Collect positions: (window_id, grid_y, grid_x) for windows in the given context.
        let entries: Vec<(u64, u32, u32)> = self
            .windows
            .iter()
            .filter(|w| w.context_id == ctx_id)
            .map(|w| (w.window_id, w.grid_y, w.grid_x))
            .collect();

        // Group by row (grid_y)
        let mut by_row: std::collections::HashMap<u32, Vec<(u64, u32)>> =
            std::collections::HashMap::new();
        for (win_id, y, x) in entries {
            by_row.entry(y).or_default().push((win_id, x));
        }

        // For each row, sort by x and assign sequential new_x = 0,1,2,...
        let mut updates: Vec<(u64, u32)> = Vec::new();
        for (_, mut items) in by_row {
            items.sort_by_key(|(_, x)| *x);
            for (new_x, (win_id, _)) in items.iter().enumerate() {
                updates.push((*win_id, new_x as u32));
            }
        }

        // Apply updates
        let mut old_to_new: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
        for win in self.windows.iter_mut() {
            if let Some(&(_, new_x)) = updates.iter().find(|(id, _)| *id == win.window_id) {
                if win.grid_x != new_x {
                    old_to_new.insert(win.grid_x, new_x);
                    win.grid_x = new_x;
                }
            }
        }

        // Update last_page_x_per_row bookkeeping for any shifted columns
        for (_, last_x) in self.last_page_x_per_row.iter_mut() {
            if let Some(&new) = old_to_new.get(last_x) {
                *last_x = new;
            }
        }
    }

    /// Switch the active context to `new_ctx_idx`, saving the current
    /// context's minimap state and restoring the target context's saved state.
    /// Falls back to `visible = (page count > 1)` on first visit.
    ///
    /// This is the standard path for context navigation. Focus-history traversal
    /// has its own save/restore path because it targets a specific pane tile.
    pub(crate) fn switch_workspace(&mut self, new_ctx_idx: usize) {
        // Record the outgoing focus so Cmd+[ can return here from any context.
        if let Some(w) = self.windows.get(self.active_window) {
            let old_window_id = w.window_id;
            let old_focus = w.focused_pane;
            self.push_focus_history(old_window_id, old_focus);
        }

        // Save current context's active window + minimap state.
        let old_ctx_id = self.router.active().context_id;
        self.context_active_window
            .insert(old_ctx_id, self.windows[self.active_window].window_id);
        self.minimap_visible_per_context
            .insert(old_ctx_id, self.minimap.visible);

        self.router.set_active(new_ctx_idx);
        self.pick_active_context_from_workspace();

        // Restore minimap state for the new context.
        let new_ctx_id = self.router.active().context_id;
        let page_count = self
            .windows
            .iter()
            .filter(|w| w.context_id == new_ctx_id)
            .count();
        self.minimap.visible = self
            .minimap_visible_per_context
            .get(&new_ctx_id)
            .copied()
            .unwrap_or(page_count > 1);

        self.apply_context_transition_effects();
    }

    pub(crate) fn pick_active_context_from_workspace(&mut self) {
        let ctx_id = self.router.active().context_id;
        let preferred = self.context_active_window.get(&ctx_id).copied();
        if let Some(win_id) = preferred {
            if let Some(idx) = self
                .windows
                .iter()
                .position(|w| w.window_id == win_id && w.context_id == ctx_id)
            {
                self.active_window = idx;
                self.record_context_visit(win_id);
                return;
            }
        }
        if let Some(idx) = self.windows.iter().position(|w| w.context_id == ctx_id) {
            self.active_window = idx;
            let wid = self.windows[idx].window_id;
            self.context_active_window.insert(ctx_id, wid);
            self.record_context_visit(wid);
        }
    }

    /// Toggle parked state on the active context. When parking, focus moves
    /// to the nearest unparked neighbor. When unparking (called from sidebar
    /// click), the context is restored and focused.
    pub(crate) fn toggle_park_active_context(&mut self) {
        let idx = self.router.active_idx();
        let is_parked = self.router.get(idx).parked;

        if is_parked {
            // Unpark
            self.router.get_mut(idx).parked = false;
            log::info!(
                "context: unparked '{}' (idx={idx})",
                self.router.get(idx).name
            );
            self.save_workspace();
            return;
        }

        // Park the active context
        let name = self.router.get(idx).name.clone();
        self.router.get_mut(idx).parked = true;
        log::info!("context: parked '{name}' (idx={idx})");

        // Find the nearest unparked context to switch focus to.
        // Search forward first, then backward.
        let len = self.router.len();
        let next_unparked = (1..len)
            .map(|offset| (idx + offset) % len)
            .find(|&i| !self.router.get(i).parked);

        if let Some(new_idx) = next_unparked {
            self.switch_workspace(new_idx);
        }
        // If all contexts are parked, stay on the current one (degenerate case).

        self.save_workspace();
    }

    /// Unpark a specific context by index and switch focus to it.
    pub(crate) fn unpark_context(&mut self, idx: usize) {
        self.router.get_mut(idx).parked = false;
        log::info!(
            "context: unparked '{}' (idx={idx})",
            self.router.get(idx).name
        );
        self.switch_workspace(idx);
        self.save_workspace();
    }

    /// Set the `root` of the active context.
    pub(crate) fn set_active_context_root(&mut self, root: PathBuf) {
        let idx = self.router.active_idx();
        log::info!(
            "set_active_context_root: ctx_id={} root={}",
            self.router.active().context_id,
            root.display()
        );
        auto_init_workspace(&root);
        self.router.get_mut(idx).root = Some(root);
        self.apply_context_transition_effects();
    }

    /// Single choke point for all context-transition side effects.
    ///
    /// Must be called after every operation that changes the active context (either
    /// which context is active or what root it points at). Owns, in order:
    ///   1. Rescan the AppRegistry for the new context root.
    ///   2. Restart the app_registry_watcher on the new root's watch dirs.
    ///   3. Palette scope follows implicitly — the palette reads `self.registry` directly.
    ///   4. Reload workspace agents into the AgentHost — agents are
    ///      workspace-scoped, so a workspace that becomes active after boot
    ///      must still get its agents attached.
    pub(crate) fn apply_context_transition_effects(&mut self) {
        let root = self.router.active().root.clone().unwrap_or_else(|| {
            std::env::current_dir()
                .unwrap_or_else(|_| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")))
        });
        log::info!(
            "transition_context: ctx_id={} root={} — rescanning registry + restarting watcher",
            self.router.active().context_id,
            root.display()
        );
        self.registry = crate::app::registry::AppRegistry::load(&root);
        // Same workspace resolution the registry just performed (cwd-walk to
        // the channel dir) — agents live in that workspace's `agents/` dir.
        self.agent_host
            .reload_workspace(self.registry.loaded_workspace.clone());
        let watch_dirs = crate::app::registry::registry_watch_dirs(&root);
        match crate::app::registry_watcher::start(watch_dirs) {
            Some((watcher, rx)) => {
                self._registry_watcher = Some(watcher);
                self.registry_reload_rx = Some(rx);
            }
            None => {
                self._registry_watcher = None;
                self.registry_reload_rx = None;
            }
        }
    }

    /// Dissolve a portal: remove the context boundary while preserving the child
    /// layout. The active child window is grafted into the Portal tile's exact
    /// position; any remaining child windows are promoted as parent-context windows.
    pub(crate) fn dissolve_portal(&mut self, child_ctx_id: u64) {
        use egui_tiles::Tile;

        // Find the Portal tile in the active (parent) window.
        let parent_idx = self.active_window;
        let parent_ctx_id = self.windows[parent_idx].context_id;
        let parent_window_id = self.windows[parent_idx].window_id;
        let portal_pane_id = {
            let win = &self.windows[parent_idx];
            win.panes
                .iter()
                .find(|(_, p)| p.portal_target() == Some(child_ctx_id))
                .map(|(id, _)| *id)
        };
        let portal_pane_id = match portal_pane_id {
            Some(id) => id,
            None => {
                log::warn!(
                    "dissolve_portal: no Portal tile for ctx={child_ctx_id} in active window"
                );
                return;
            }
        };

        let portal_tile_id = self.windows[parent_idx]
            .tree
            .tiles
            .find_pane(&portal_pane_id);
        let portal_tile_id = match portal_tile_id {
            Some(id) => id,
            None => {
                log::warn!("dissolve_portal: Portal pane {portal_pane_id} has no tile");
                return;
            }
        };

        let mut child_windows: Vec<(u64, u32, u32)> = self
            .windows
            .iter()
            .filter(|w| w.context_id == child_ctx_id)
            .map(|w| (w.window_id, w.grid_y, w.grid_x))
            .collect();
        child_windows.sort_by_key(|(window_id, grid_y, grid_x)| (*grid_y, *grid_x, *window_id));

        let Some(primary_child_window_id) = self
            .context_active_window
            .get(&child_ctx_id)
            .copied()
            .filter(|active_id| {
                child_windows
                    .iter()
                    .any(|(window_id, _, _)| window_id == active_id)
            })
            .or_else(|| child_windows.first().map(|(window_id, _, _)| *window_id))
        else {
            log::warn!(
                "dissolve_portal: ctx={child_ctx_id} has no child windows; removing portal only"
            );
            self.windows[parent_idx].panes.remove(&portal_pane_id);
            self.windows[parent_idx]
                .tree
                .remove_recursively(portal_tile_id);
            if let Some(idx) = self.router.position(|c| c.context_id == child_ctx_id) {
                self.router.remove_at(idx);
            }
            self.context_active_window.remove(&child_ctx_id);
            self.router.retain_depth_stack(|cid| cid != child_ctx_id);
            return;
        };
        let promoted_child_window_count = child_windows.len().saturating_sub(1);

        let Some(primary_idx) = self
            .windows
            .iter()
            .position(|w| w.window_id == primary_child_window_id && w.context_id == child_ctx_id)
        else {
            log::warn!(
                "dissolve_portal: primary child window {primary_child_window_id} missing for ctx={child_ctx_id}"
            );
            return;
        };
        if primary_idx == parent_idx {
            log::warn!(
                "dissolve_portal: primary child window matches parent window idx={parent_idx}"
            );
            return;
        }

        log::info!(
            "dissolve_portal: ctx={child_ctx_id} parent_ctx={parent_ctx_id} graft_primary_window={primary_child_window_id} promote_windows={}",
            promoted_child_window_count
        );

        {
            let (parent_win, primary_child_win) =
                two_windows_mut(&mut self.windows, parent_idx, primary_idx);
            let Some(child_root) = primary_child_win.tree.root else {
                log::warn!(
                    "dissolve_portal: primary child window {primary_child_window_id} has no root"
                );
                return;
            };

            let child_focus = primary_child_win.focused_pane;
            let child_zoom = primary_child_win.zoomed_pane;
            let mut tile_map = HashMap::new();
            let Some(grafted_root) = clone_tile_subtree(
                &primary_child_win.tree.tiles,
                child_root,
                &mut parent_win.tree.tiles,
                &mut tile_map,
            ) else {
                log::warn!("dissolve_portal: failed to clone child tree for ctx={child_ctx_id}");
                return;
            };

            parent_win.panes.remove(&portal_pane_id);
            parent_win
                .panes
                .extend(std::mem::take(&mut primary_child_win.panes));
            if let Some(parent_tile) = parent_win.tree.tiles.parent_of(portal_tile_id) {
                if let Some(Tile::Container(parent_container)) =
                    parent_win.tree.tiles.get_mut(parent_tile)
                {
                    crate::host::context::replace_child(
                        parent_container,
                        portal_tile_id,
                        grafted_root,
                    );
                }
            } else {
                parent_win.tree.root = Some(grafted_root);
            }
            parent_win.tree.tiles.remove(portal_tile_id);

            parent_win.focused_pane = map_focus_tile(&tile_map, child_focus)
                .or_else(|| parent_win.find_first_pane_in(grafted_root));
            parent_win.zoomed_pane = map_focus_tile(&tile_map, child_zoom);
            parent_win.reconcile_stale_tiles();
        }

        let mut occupied: HashSet<(u32, u32)> = self
            .windows
            .iter()
            .filter(|w| w.context_id == parent_ctx_id)
            .map(|w| (w.grid_x, w.grid_y))
            .collect();
        for (window_id, _grid_y, _grid_x) in child_windows
            .iter()
            .filter(|(window_id, _, _)| *window_id != primary_child_window_id)
        {
            let Some(win) = self
                .windows
                .iter_mut()
                .find(|w| w.window_id == *window_id && w.context_id == child_ctx_id)
            else {
                continue;
            };
            let (grid_x, grid_y) = reserve_grid_slot(&mut occupied, win.grid_x, win.grid_y);
            win.context_id = parent_ctx_id;
            win.grid_x = grid_x;
            win.grid_y = grid_y;
            log::info!(
                "dissolve_portal: promoted child window {} to parent ctx={} grid=({}, {})",
                win.window_id,
                parent_ctx_id,
                grid_x,
                grid_y
            );
        }

        self.windows
            .retain(|w| w.window_id != primary_child_window_id && w.context_id != child_ctx_id);
        if let Some(idx) = self.router.position(|c| c.context_id == child_ctx_id) {
            self.router.remove_at(idx);
        }

        self.context_active_window.remove(&child_ctx_id);
        self.context_active_window
            .insert(parent_ctx_id, parent_window_id);

        let before_depth = self.router.depth_stack.len();
        self.router.retain_depth_stack(|cid| cid != child_ctx_id);
        let cleaned_depth = before_depth - self.router.depth_stack.len();
        if cleaned_depth > 0 {
            log::info!(
                "dissolve_portal: removed {cleaned_depth} stale depth_stack entries for ctx={child_ctx_id}"
            );
        }

        for win in &mut self.windows {
            let stale_portal_ids: Vec<_> = win
                .panes
                .iter()
                .filter(|(_, pane)| pane.portal_target() == Some(child_ctx_id))
                .map(|(pane_id, _)| *pane_id)
                .collect();
            for pane_id in stale_portal_ids {
                win.panes.remove(&pane_id);
                if let Some(tile_id) = win.tree.tiles.find_pane(&pane_id) {
                    win.tree.remove_recursively(tile_id);
                }
                log::warn!(
                    "dissolve_portal: removed stale PortalPane {pane_id} targeting dissolved ctx={child_ctx_id}"
                );
            }
            win.reconcile_stale_tiles();
        }

        self.active_window = self
            .windows
            .iter()
            .position(|w| w.window_id == parent_window_id && w.context_id == parent_ctx_id)
            .unwrap_or(0);
        if let Some(parent_ctx_idx) = self.router.position(|c| c.context_id == parent_ctx_id) {
            self.router.set_active(parent_ctx_idx);
        }

        self.pending_notifications.retain(|n| {
            !(matches!(n.scope, crate::app_protocol::NotifyScope::Context)
                && n.source_context_id == child_ctx_id)
        });
        self.save_notifications();
        if let Some(ref id) = self.current_notify_id.clone() {
            let still_present = self
                .pending_notifications
                .iter()
                .any(|n| &n.notify_id == id);
            if !still_present {
                self.current_notify_id = None;
            }
        }

        let page_count = self
            .windows
            .iter()
            .filter(|w| w.context_id == parent_ctx_id)
            .count();
        let restored_minimap_visible = self
            .minimap_visible_per_context
            .get(&parent_ctx_id)
            .copied()
            .unwrap_or(page_count > 1);
        let show_promoted_windows = promoted_child_window_count > 0 && page_count > 1;
        self.minimap.visible = restored_minimap_visible || show_promoted_windows;
        if show_promoted_windows {
            self.minimap_visible_per_context.insert(parent_ctx_id, true);
            log::info!(
                "dissolve_portal: parent ctx={parent_ctx_id} now has {page_count} windows; showing minimap"
            );
        }
    }

    pub(crate) fn save_workspace(&self) {
        let mut saved_contexts = Vec::new();
        let mut saved_windows = Vec::new();

        for ctx in self.router.iter() {
            saved_contexts.push(ctx.clone());
        }

        for win in &self.windows {
            let mut saved_panes = Vec::new();
            for (&id, pane) in &win.panes {
                debug_assert_eq!(pane.id(), id);
                let pane_hidden = pane.is_hidden();
                if let Some(t) = pane.as_terminal() {
                    let cwd = shell::get_pid_cwd(t.backend.child_pid())
                        .unwrap_or_else(|| win.path.clone());
                    saved_panes.push(crate::workspace::SavedPane {
                        id,
                        kind: crate::workspace::SavedPaneKind::Terminal,
                        cwd,
                        name: t.name.clone(),
                        app_id: None,
                        app_state: None,
                        hidden: pane_hidden,
                    });
                } else if let Some(a) = pane.as_app() {
                    saved_panes.push(crate::workspace::SavedPane {
                        id,
                        kind: crate::workspace::SavedPaneKind::App,
                        cwd: a.workspace_root.clone(),
                        name: Some(a.name.clone()),
                        app_id: Some(a.runtime.type_id().to_string()),
                        app_state: a.runtime.serialize_state(),
                        hidden: pane_hidden,
                    });
                } else if let Some(child_ctx_id) = pane.portal_target() {
                    saved_panes.push(crate::workspace::SavedPane {
                        id,
                        kind: crate::workspace::SavedPaneKind::Portal {
                            context_id: child_ctx_id,
                        },
                        cwd: std::path::PathBuf::new(),
                        name: None,
                        app_id: None,
                        app_state: None,
                        hidden: pane_hidden,
                    });
                }
            }
            saved_windows.push(crate::workspace::SavedWindow {
                name: win.name.clone(),
                path: win.path.clone(),
                tree: win.tree.clone(),
                panes: saved_panes,
                focused_pane: win.focused_pane,
                grid_x: win.grid_x,
                grid_y: win.grid_y,
                window_id: win.window_id,
                context_id: win.context_id,
            });
        }

        let ws = WorkspaceFile {
            version: 2,
            active_context: self.router.active_idx(),
            sidebar_visible: self.sidebar_visible,
            next_pane_id: persisted_next_pane_id(self.host.next_pane_id(), &saved_windows),
            contexts: saved_contexts,
            windows: saved_windows,
            context_active_window: self.context_active_window.clone(),
        };

        if let Err(e) = ws.save() {
            log::error!("Failed to save workspace: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved_window_with_panes(ids: &[u64]) -> crate::workspace::SavedWindow {
        crate::workspace::SavedWindow {
            name: "test".to_string(),
            path: std::env::temp_dir(),
            tree: egui_tiles::Tree::empty("test_tree"),
            panes: ids
                .iter()
                .map(|id| crate::workspace::SavedPane {
                    id: *id,
                    kind: crate::workspace::SavedPaneKind::App,
                    cwd: std::env::temp_dir(),
                    name: None,
                    app_id: Some("test".to_string()),
                    app_state: None,
                    hidden: false,
                })
                .collect(),
            focused_pane: None,
            grid_x: 0,
            grid_y: 0,
            window_id: 1,
            context_id: 1,
        }
    }

    #[test]
    fn persisted_next_pane_id_cannot_collide_with_saved_panes() {
        let windows = vec![
            saved_window_with_panes(&[1, 4]),
            saved_window_with_panes(&[9, 12]),
        ];

        assert_eq!(persisted_next_pane_id(3, &windows), 13);
        assert_eq!(persisted_next_pane_id(20, &windows), 20);
        assert_eq!(persisted_next_pane_id(7, &[]), 7);
    }
}
