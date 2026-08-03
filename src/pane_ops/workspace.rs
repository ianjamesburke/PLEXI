//! Multi-context and workspace persistence: new context, reset, delete,
//! and on-disk workspace save.

use crate::app::PlexiApp;
use crate::host::context::{ContextName, Window};
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
/// Ensure `<root>/.plexi/.gitignore` covers `app_states/` so a user can never
/// accidentally commit their app state with a project (standing ruling: app
/// state is personal, single-user, local data). Idempotent; preserves any
/// existing user rules. Every path that creates or rewrites a context root
/// calls this.
fn ensure_context_state_ignore(root: &std::path::Path) {
    if let Err(error) = crate::workspace::secrets::ensure_app_state_gitignore(root) {
        log::warn!(
            "could not ensure {}/.plexi/.gitignore covers app_states/: {error}",
            root.display()
        );
    }
}

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

    ensure_context_state_ignore(root);


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

fn revoke_window_pane_credentials(window: &Window) {
    for pane_id in window.panes.keys() {
        crate::app::host_mcp::revoke_pane_credentials(*pane_id);
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

/// Where a new child context attaches, and what its window is seeded with.
pub(crate) struct ChildContextSpec {
    /// Parent context id — the caller's `PLEXI_CONTEXT_ID`. Authoritative when
    /// present: two contexts can share a name, so resolving by name alone
    /// silently nests under whichever was created first.
    pub parent_id: Option<u64>,
    /// Parent context name. Consulted only when `parent_id` is absent — a
    /// present-but-unknown id is an error, never a reason to guess by name.
    pub parent_name: String,
    /// Explicit name for the child context. When `None` the name is derived
    /// from the path (or the anchor's `[context]` defaults). Set it here rather
    /// than renaming afterwards: the panes are spawned with this name in
    /// `PLEXI_CONTEXT_NAME`, so a later rename would leave running agents
    /// advertising a name the router no longer uses.
    pub name: Option<String>,
    /// Root path and working directory for the child context.
    pub path: PathBuf,
    /// Portal tile orientation in the parent window: `true` splits side-by-side.
    pub portal_vertical: bool,
    /// `true` places the portal before the anchor tile (left / up).
    pub portal_first: bool,
    /// Pane in the parent to anchor the portal split at; falls back to the
    /// parent's focused pane when absent or unknown.
    pub anchor_pane: Option<u64>,
    /// One entry per pane to seed in the child window, in order; `None` launches
    /// a plain shell. Never empty.
    pub panes: Vec<Option<String>>,
    /// How the seeded panes are arranged inside the child's single window.
    pub layout: crate::app_protocol::SubContextLayout,
}

impl ChildContextSpec {
    /// The historical single-terminal child: one plain shell, tiled.
    pub fn single_terminal(
        parent_id: Option<u64>,
        parent_name: String,
        path: PathBuf,
        portal_vertical: bool,
        portal_first: bool,
        anchor_pane: Option<u64>,
    ) -> Self {
        Self {
            parent_id,
            parent_name,
            name: None,
            path,
            portal_vertical,
            portal_first,
            anchor_pane,
            panes: vec![None],
            layout: crate::app_protocol::SubContextLayout::default(),
        }
    }
}

/// What a successful [`PlexiApp::new_child_context`] created.
pub(crate) struct ChildContext {
    pub context_id: u64,
    /// Seeded pane ids, in the order their commands were given.
    pub pane_ids: Vec<PaneId>,
    /// The parent window the portal was inserted into, and its focused tile,
    /// both captured *before* the insert. A caller that pushes a depth entry
    /// must use these rather than the globally active window: a background pane
    /// can create a sub-context while the user is looking at another context
    /// entirely, and a depth entry pairing this parent with an unrelated window
    /// cannot restore the caller's location on zoom-out.
    pub parent_window_id: Option<u64>,
    pub parent_focused_pane: Option<egui_tiles::TileId>,
}

impl PlexiApp {
    /// Resolve a parent context to a router index: by `id` when one is given,
    /// otherwise by case-insensitive `name`.
    ///
    /// An `id` that names no live context resolves to `None` — it does **not**
    /// fall back to the name. Names are not unique, so a stale
    /// `PLEXI_CONTEXT_ID` (its context deleted) plus a name another context
    /// happens to share would silently attach the child to a stranger. Failing
    /// loudly is the only safe answer.
    pub(crate) fn resolve_parent_context(&self, id: Option<u64>, name: &str) -> Option<usize> {
        match id {
            Some(want) => self.router.position(|c| c.context_id == want),
            None => self
                .router
                .position(|c| c.name.displayed().eq_ignore_ascii_case(name)),
        }
    }

    /// Create a child context nested under `spec`'s parent, seeded with exactly
    /// `spec.panes.len()` terminals in one window (portal model — no pane
    /// adoption). Inserts a Portal tile into the parent window as a sibling of
    /// the anchor tile. No depth cap.
    pub(crate) fn new_child_context(
        &mut self,
        spec: ChildContextSpec,
    ) -> Result<ChildContext, String> {
        let ChildContextSpec {
            parent_id: want_parent_id,
            parent_name,
            name: explicit_name,
            path,
            portal_vertical: vertical,
            portal_first: new_pane_first,
            anchor_pane,
            panes: pane_commands,
            layout,
        } = spec;
        if pane_commands.is_empty() {
            return Err("child context requires at least one pane".to_string());
        }
        let parent_idx = self
            .resolve_parent_context(want_parent_id, &parent_name)
            .ok_or_else(|| match want_parent_id {
                Some(id) => format!(
                    "no context with id {id} — PLEXI_CONTEXT_ID is stale (its context was closed); \
                     open a new pane or pass an explicit parent"
                ),
                None => format!("no context named '{parent_name}'"),
            })?;
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
        // An explicit name wins over the derived one, and must be settled here:
        // the panes below are spawned with it in PLEXI_CONTEXT_NAME.
        let ctx_name = explicit_name.unwrap_or(ctx_name);

        log::info!(
            "new_child_context: parent_id={parent_id} parent_depth={parent_depth} \
             child_depth={child_depth} name={ctx_name} panes={} layout={layout:?} path={}",
            pane_commands.len(),
            path.display()
        );

        // 1. Build the child window's panes. Their PLEXI_CONTEXT_* env is
        // stamped from the child's own identity, which is not in the router
        // yet — hence the explicit PaneContextEnv rather than the active
        // window's context.
        let child_env = super::create::PaneContextEnv {
            context_id: ctx_id,
            name: ctx_name.clone(),
            description: ctx_description.clone().unwrap_or_default(),
            root: Some(path.clone()),
            depth: child_depth,
        };
        let Some((child_tree, child_panes, child_root_tile, child_pane_ids)) =
            self.create_context_pane_set(&child_env, path.clone(), &pane_commands, layout)
        else {
            log::error!("new_child_context: failed to create terminals for child context");
            return Err("failed to create terminals for child context".to_string());
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
        // Snapshot the caller's location before the portal insert. A depth entry
        // built from `active_window` would be wrong whenever the request came
        // from a pane the user is not currently looking at.
        let parent_window_id = parent_win_idx.map(|idx| self.windows[idx].window_id);
        let parent_focused_pane = parent_win_idx.and_then(|idx| self.windows[idx].focused_pane);
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
            log::warn!(
                "new_child_context: parent ctx_id={parent_id} has no window — child context has no Portal tile"
            );
        }

        // 3. Register the child context + window.
        ensure_context_state_ignore(&path);
        self.router.push(crate::host::context::Context {
            name: ContextName::custom(ctx_name),
            root: path.clone(),
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

        self.mark_workspace_dirty();
        Ok(ChildContext {
            context_id: ctx_id,
            pane_ids: child_pane_ids,
            parent_window_id,
            parent_focused_pane,
        })
    }

    /// Create a new empty child context under the current context and auto-zoom
    /// into it. Bound to Cmd+Shift+Option+N. Uses the current context's path as
    /// the child's working directory and names the child "Sub-context N".
    pub(crate) fn new_child_context_from_keyboard(&mut self) {
        let parent_ctx_id = self.router.active().context_id;
        let parent_name = self.router.active().name.to_string();
        let parent_path = self.router.active().root.clone();
        let current_win_id = self.windows[self.active_window].window_id;
        let current_focused = self.windows[self.active_window].focused_pane;

        log::info!(
            "new_child_context_from_keyboard: parent_ctx_id={parent_ctx_id} parent_name={parent_name} path={}",
            parent_path.display()
        );

        match self.new_child_context(ChildContextSpec::single_terminal(
            Some(parent_ctx_id),
            parent_name.clone(),
            parent_path,
            true,
            false,
            None,
        )) {
            Ok(_) => {
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

    pub(crate) fn push_pane_to_subcontext(
        &mut self,
        name: Option<String>,
        target_pane: Option<u64>,
    ) {
        // An explicit pane (the caller's PLEXI_PANE_ID over IPC) selects both
        // the window and the tile; otherwise the focused pane is pushed.
        let target = target_pane.and_then(|pid| {
            self.windows
                .iter()
                .enumerate()
                .find_map(|(idx, w)| w.tree.tiles.find_pane(&pid).map(|tile| (idx, tile)))
        });
        if target_pane.is_some() && target.is_none() {
            log::warn!(
                "push_pane_to_subcontext: pane {target_pane:?} not found — falling back to focused pane"
            );
        }
        let (parent_win_idx, target_tile) = match target {
            Some(t) => t,
            None => {
                let win_idx = self.active_window;
                let Some(focused_tile) = self.windows[win_idx].focused_pane else {
                    log::warn!("push_pane_to_subcontext: no focused pane");
                    return;
                };
                (win_idx, focused_tile)
            }
        };
        let parent_ctx_id = self.windows[parent_win_idx].context_id;
        let parent_depth = self
            .router
            .iter()
            .find(|c| c.context_id == parent_ctx_id)
            .map(|c| c.depth)
            .unwrap_or(0);

        let pane_id = match self.windows[parent_win_idx].tree.tiles.get(target_tile) {
            Some(egui_tiles::Tile::Pane(pid)) => *pid,
            _ => {
                log::warn!("push_pane_to_subcontext: target tile is not a pane");
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

        let parent_root = self
            .router
            .iter()
            .find(|c| c.context_id == parent_ctx_id)
            .map(|c| c.root.clone())
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")));

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
        if let Some(egui_tiles::Tile::Pane(slot)) =
            self.windows[parent_win_idx].tree.tiles.get_mut(target_tile)
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

        ensure_context_state_ignore(&parent_root);
        self.router.insert_after_subtree(
            parent_ctx_id,
            crate::host::context::Context {
                name: ContextName::custom(ctx_name),
                root: parent_root.clone(),
                description: None,
                context_id: ctx_id,
                parent_id: Some(parent_ctx_id),
                depth: child_depth,
                parked: false,
            },
        );
        self.windows.push(Window {
            name: String::new(),
            path: parent_root,
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
            .push_depth(parent_ctx_id, current_win_id, Some(target_tile));
        let new_ctx_idx = self
            .router
            .position(|c| c.context_id == ctx_id)
            .expect("just-inserted subcontext must be in router");
        self.switch_workspace(new_ctx_idx);

        self.mark_workspace_dirty();
    }

    pub(crate) fn new_context(&mut self) {
        log::info!("new_context: creating new top-level context");
        self.new_context_empty();
    }

    pub(crate) fn auto_context_name_for_path(
        &self,
        context_id: u64,
        path: &std::path::Path,
    ) -> String {
        let base = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| format!("Context {}", self.router.len() + 1));
        let existing = self
            .router
            .iter()
            .find(|context| context.context_id != context_id && context.name.displayed() == base);
        let Some(existing) = existing else {
            return base;
        };

        let mut suffix = 2;
        loop {
            let candidate = format!("{base} ({suffix})");
            if !self.router.iter().any(|context| {
                context.context_id != context_id && context.name.displayed() == candidate
            }) {
                log::info!(
                    "context_auto_name: '{}' collides with context {} — using '{}'",
                    base,
                    existing.context_id,
                    candidate
                );
                return candidate;
            }
            suffix += 1;
        }
    }

    /// Resolve an empty auto name from a focused terminal exactly once. A
    /// terminal may not have reported its cwd on the creation frame, so this
    /// runs from the host logic loop until that first report arrives.
    pub(crate) fn resolve_auto_context_names(&mut self) {
        let unresolved: Vec<(usize, u64)> = self
            .router
            .iter()
            .enumerate()
            .filter(|(_, context)| context.name.is_unresolved_auto())
            .map(|(idx, context)| (idx, context.context_id))
            .collect();
        for (idx, context_id) in unresolved {
            let cwd = self
                .windows
                .iter()
                .find(|window| window.context_id == context_id)
                .and_then(|window| {
                    window
                        .focused_pane
                        .and_then(|tile_id| window.get_focused_pane_cwd(tile_id))
                });
            let Some(cwd) = cwd else {
                continue;
            };
            let name = self.auto_context_name_for_path(context_id, &cwd);
            self.router.get_mut(idx).name = ContextName::auto(name.clone());
            log::info!(
                "context_auto_name: context_id={context_id} cwd={} name={name}",
                cwd.display()
            );
            self.save_workspace();
        }
    }

    /// Apply a rename submission. Blank submissions intentionally return the
    /// label to automatic mode, using the focused cwd (or the context root if
    /// that terminal is between cwd reports).
    pub(crate) fn rename_context(&mut self, ctx_idx: usize, submitted: &str) -> String {
        let context_id = self.router.get(ctx_idx).context_id;
        let trimmed = submitted.trim();
        let name = if trimmed.is_empty() {
            let cwd = self
                .windows
                .iter()
                .find(|window| window.context_id == context_id)
                .and_then(|window| {
                    window
                        .focused_pane
                        .and_then(|tile_id| window.get_focused_pane_cwd(tile_id))
                        .or_else(|| Some(window.path.clone()))
                });
            match cwd {
                Some(cwd) => ContextName::auto(self.auto_context_name_for_path(context_id, &cwd)),
                None => ContextName::auto(String::new()),
            }
        } else {
            ContextName::custom(trimmed)
        };
        let displayed = name.displayed().to_owned();
        self.router.get_mut(ctx_idx).name = name;
        log::info!("context_rename: context_id={context_id} name={displayed}");
        crate::host::event_log::emit(crate::host::event_log::HostEvent::ContextRenamed {
            context_id,
            name: displayed.clone(),
            timestamp: crate::host::event_log::now_timestamp(),
        });
        self.save_workspace();
        displayed
    }

    /// Create a new standalone empty context at the home directory.
    fn new_context_empty(&mut self) {
        let cwd = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        log::info!("new_context_empty: cwd={}", cwd.display());

        let ctx_id = self.next_window_id;
        self.next_window_id += 1;
        let win_id = self.next_window_id;
        self.next_window_id += 1;

        let ctx_name = self.auto_context_name_for_path(ctx_id, &cwd);
        let context_env = super::create::PaneContextEnv {
            context_id: ctx_id,
            name: ctx_name.clone(),
            description: String::new(),
            root: Some(cwd.clone()),
            depth: 0,
        };

        // Push an empty window, then seed its base root pane with the new
        // context's identity before it is registered in the router.
        self.windows.push(Window {
            name: String::new(),
            path: cwd.clone(),
            tree: egui_tiles::Tree::empty("plexi"),
            panes: HashMap::new(),
            focused_pane: None,
            zoomed_pane: None,
            grid_x: 0,
            grid_y: 0,
            window_id: win_id,
            context_id: ctx_id,
        });
        let new_idx = self.windows.len() - 1;
        if self
            .seed_window_root_pane(new_idx, &context_env, cwd.clone(), None, false)
            .is_none()
        {
            log::error!("new_context_empty: failed to seed root pane — aborting new context");
            self.windows.pop();
            return;
        }

        ensure_context_state_ignore(&cwd);
        self.router.push(crate::host::context::Context {
            name: ContextName::auto(String::new()),
            root: cwd,
            description: None,
            context_id: ctx_id,
            parent_id: None,
            depth: 0,
            parked: false,
        });
        self.router.activate_last();
        self.active_window = new_idx;
        self.context_active_window.insert(ctx_id, win_id);
        self.minimap.visible = false;
        self.apply_context_transition_effects();

        self.mark_workspace_dirty();
        log::info!(
            "new_context_empty: emitting ContextCreated context_id={ctx_id} name={ctx_name}"
        );
        crate::host::event_log::emit(crate::host::event_log::HostEvent::ContextCreated {
            context_id: ctx_id,
            name: ctx_name,
            timestamp: crate::host::event_log::now_timestamp(),
        });
    }

    /// Create a new context at a specific directory path. The terminal pane
    /// opens at `path` and the context root is set to it. Named after the
    /// directory basename, unless the path has a `.plexi/workspace.toml` with
    /// `[context]` defaults. Callers must call `mark_workspace_dirty()` afterward.
    pub(crate) fn new_context_at_path(&mut self, path: PathBuf) {
        log::info!("new_context_at_path: path={}", path.display());

        let ctx_id = self.next_window_id;
        self.next_window_id += 1;
        let win_id = self.next_window_id;
        self.next_window_id += 1;

        // Resolve the identity before spawning: the root pane starts before this
        // context is registered in the router.
        let anchor = crate::host::anchor::Anchor::detect(&path);
        let (ctx_name, ctx_description, name_is_custom) =
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
                    (name, defaults.description.clone(), defaults.name.is_some())
                }
                None => {
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| format!("Context {}", self.router.len() + 1));
                    (name, None, false)
                }
            };
        let context_name = if name_is_custom {
            ContextName::custom(ctx_name)
        } else {
            ContextName::auto(self.auto_context_name_for_path(ctx_id, &path))
        };
        let context_env = super::create::PaneContextEnv {
            context_id: ctx_id,
            name: context_name.displayed().to_owned(),
            description: ctx_description.clone().unwrap_or_default(),
            root: Some(path.clone()),
            depth: 0,
        };

        // Push an empty window, then seed its base root pane in place with the
        // context identity resolved above.
        self.windows.push(Window {
            name: String::new(),
            path: path.clone(),
            tree: egui_tiles::Tree::empty("plexi"),
            panes: HashMap::new(),
            focused_pane: None,
            zoomed_pane: None,
            grid_x: 0,
            grid_y: 0,
            window_id: win_id,
            context_id: ctx_id,
        });
        let new_idx = self.windows.len() - 1;
        if self
            .seed_window_root_pane(new_idx, &context_env, path.clone(), None, false)
            .is_none()
        {
            log::error!(
                "new_context_at_path: failed to seed root pane for {} — aborting new context",
                path.display()
            );
            self.windows.pop();
            return;
        }

        ensure_context_state_ignore(&path);
        self.router.push(crate::host::context::Context {
            name: context_name,
            root: path,
            description: ctx_description,
            context_id: ctx_id,
            parent_id: None,
            depth: 0,
            parked: false,
        });
        self.router.activate_last();
        self.active_window = new_idx;
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
        self.create_page_at(new_x, active_y, ws_id, None, false, None);
    }

    /// Shared creation helper: create a single-pane window at `(grid_x, grid_y)` in
    /// `context_id` and make it the active window.
    pub(crate) fn create_page_at(
        &mut self,
        grid_x: u32,
        grid_y: u32,
        context_id: u64,
        initial_cmd: Option<&str>,
        close_on_exit: bool,
        cwd_override: Option<PathBuf>,
    ) {
        let old_window_id = self.windows[self.active_window].window_id;
        let old_focus = self.windows[self.active_window].focused_pane;
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let cwd = cwd_override
            .or_else(|| {
                self.resolve_new_pane_cwd(None)
            })
            .filter(|p| p != &PathBuf::from("/"))
            .unwrap_or(home);
        log::info!(
            "create_page_at({grid_x},{grid_y}): cwd={} context_id={context_id} initial_cmd={initial_cmd:?} close_on_exit={close_on_exit}",
            cwd.display()
        );
        let context_env = self.pane_context_env_for_context(context_id);
        let Some((tree, panes, root_tile)) = self.create_single_pane_tree(
            &context_env,
            Some(cwd.clone()),
            initial_cmd,
            close_on_exit,
        ) else {
            log::error!("Failed to create terminal for new page at ({grid_x}, {grid_y})");
            return;
        };
        let name = String::new();
        let ctx_id = context_id;
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

    /// The single install point for a context's base root terminal pane. Creates
    /// a fresh single-pane terminal tree via
    /// [`create_single_pane_tree`](Self::create_single_pane_tree) and installs it
    /// into window `win_idx` in place — replacing its tree/panes, focusing the
    /// root tile, and clearing any zoom. Every path that must leave a context
    /// with a live root terminal funnels here: fresh-profile first boot,
    /// [`new_context_empty`](Self::new_context_empty),
    /// [`new_context_at_path`](Self::new_context_at_path), and the spawn-queue
    /// fallback ([`seed_root_pane`](Self::seed_root_pane)).
    ///
    /// Context metadata for the terminal's backend settings is read from the
    /// *active* window (see `create_single_pane_tree`). New-context callers seed
    /// before switching `active_window`, so that historical env-var context is
    /// preserved unchanged.
    ///
    /// Returns `(pane_id, root_tile)`, or `None` if the PTY-backed terminal
    /// failed to spawn — the caller decides how to degrade.
    pub(crate) fn seed_window_root_pane(
        &mut self,
        win_idx: usize,
        context: &super::create::PaneContextEnv,
        cwd: PathBuf,
        initial_cmd: Option<&str>,
        close_on_exit: bool,
    ) -> Option<(PaneId, egui_tiles::TileId)> {
        let (tree, panes, root_tile) =
            self.create_single_pane_tree(context, Some(cwd), initial_cmd, close_on_exit)?;
        let pane_id = *panes
            .keys()
            .next()
            .expect("create_single_pane_tree always yields exactly one pane");
        let win = &mut self.windows[win_idx];
        win.tree = tree;
        win.panes = panes;
        win.focused_pane = Some(root_tile);
        win.zoomed_pane = None;
        Some((pane_id, root_tile))
    }

    /// Seed a root terminal pane into the active window when it has no tree root
    /// and no focused pane yet (the windowless-boot state). Returns the id of the
    /// pane actually created, or `None` if terminal creation failed.
    ///
    /// The spawn_pane IPC fallback needs this because `split_focused` returns
    /// early when there is no focused pane to split, silently dropping the spawn.
    /// Unlike `create_page_at`, this populates the *existing* active window in
    /// place rather than pushing a new one, so a seeded/queued `pane new` lands
    /// in the window the host booted with.
    pub(crate) fn seed_root_pane(
        &mut self,
        initial_cmd: Option<&str>,
        close_on_exit: bool,
        cwd_override: Option<PathBuf>,
    ) -> Option<crate::spatial::tiling::PaneId> {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let cwd = cwd_override
            .or_else(|| {
                self.resolve_new_pane_cwd(None)
            })
            .filter(|p| p != &PathBuf::from("/"))
            .unwrap_or(home);
        let active = self.active_window;
        let context = self.pane_context_env_for_window(active);
        let Some((pane_id, _root_tile)) =
            self.seed_window_root_pane(active, &context, cwd, initial_cmd, close_on_exit)
        else {
            log::error!("seed_root_pane: failed to create terminal for empty active window");
            return None;
        };
        log::info!(
            "seed_root_pane: seeded root pane_id={pane_id} into empty window_id={} (windowless-boot spawn fallback) initial_cmd={initial_cmd:?} close_on_exit={close_on_exit}",
            self.windows[active].window_id
        );
        Some(pane_id)
    }

    pub(crate) fn reset_active_context(&mut self) {
        let cwd = self.cwd_for_welcome_tab();
        log::info!(
            "reset_active_context: cwd={} context_root={:?}",
            cwd.display(),
            self.router.active().root
        );
        let context = self.pane_context_env_for_window(self.active_window);
        let Some((tree, panes, root_tile)) =
            self.create_single_pane_tree(&context, Some(cwd), None, false)
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
        // Refuse a cascade that would empty the router entirely — `remove_at`
        // clamps `active` to 0 on an empty vec, and the next `router.active()`
        // call panics with an out-of-bounds index. Same semantics as the
        // single-context guard above, just discovered after the BFS instead
        // of before it.
        if deleted.len() >= self.router.len() {
            log::warn!(
                "delete_context: refusing cascade of ctx_id={target_ctx_id} + {} descendants — \
                 would empty the router (no surviving contexts)",
                deleted.len() - 1
            );
            return;
        }

        log::info!(
            "delete_context: cascading delete of ctx_id={target_ctx_id} + {} descendants ({:?})",
            deleted.len() - 1,
            &deleted[1..]
        );

        // 3. Remove all windows belonging to any deleted context. Release
        // their host MCP credentials before dropping the panes: a context
        // delete bypasses close_tile and therefore has no terminal-exit
        // cleanup.
        for window in self
            .windows
            .iter()
            .filter(|window| deleted.contains(&window.context_id))
        {
            revoke_window_pane_credentials(window);
        }
        let doomed: Vec<Window> = {
            let mut kept = Vec::new();
            let mut doomed = Vec::new();
            for w in std::mem::take(&mut self.windows) {
                if deleted.contains(&w.context_id) {
                    doomed.push(w);
                } else {
                    kept.push(w);
                }
            }
            self.windows = kept;
            doomed
        };
        log::info!(
            "delete_context: disposing {} window(s) off UI thread (ctx_id={target_ctx_id})",
            doomed.len()
        );
        std::thread::spawn(move || {
            let start = std::time::Instant::now();
            drop(doomed);
            log::debug!(
                "delete_context: window disposal thread finished in {:?}",
                start.elapsed()
            );
        });

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
                if let Some(parent_idx) = self.router.position(|c| c.context_id == parent_ctx_id) {
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

        revoke_window_pane_credentials(&self.windows[index]);
        let removed_window = self.windows.remove(index);
        log::info!(
            "delete_window: disposing window off UI thread (window_id={removed_win_id}, ctx_id={removed_ws_id})"
        );
        std::thread::spawn(move || {
            let start = std::time::Instant::now();
            drop(removed_window);
            log::debug!(
                "delete_window: window disposal thread finished in {:?}",
                start.elapsed()
            );
        });

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
        for last_x in self.last_page_x_per_row.values_mut() {
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
            self.mark_workspace_dirty();
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

        self.mark_workspace_dirty();
    }

    /// Park a specific context by index, moving focus to the nearest unparked neighbor.
    pub(crate) fn park_context(&mut self, idx: usize) {
        if self.router.get(idx).parked {
            return;
        }
        let name = self.router.get(idx).name.clone();
        self.router.get_mut(idx).parked = true;
        log::info!("context: parked '{name}' (idx={idx})");

        if self.router.active_idx() == idx {
            let len = self.router.len();
            let next_unparked = (1..len)
                .map(|offset| (idx + offset) % len)
                .find(|&i| !self.router.get(i).parked);
            if let Some(new_idx) = next_unparked {
                self.switch_workspace(new_idx);
            }
        }
        self.mark_workspace_dirty();
    }

    /// Unpark a specific context by index and switch focus to it.
    pub(crate) fn unpark_context(&mut self, idx: usize) {
        self.router.get_mut(idx).parked = false;
        log::info!(
            "context: unparked '{}' (idx={idx})",
            self.router.get(idx).name
        );
        self.switch_workspace(idx);
        self.mark_workspace_dirty();
    }

    /// Resolve an optional context id to a router index. Falls back to the
    /// active context (with a warning) when the id is absent or unknown.
    pub(crate) fn resolve_context_idx(&self, context_id: Option<u64>, op: &str) -> usize {
        match context_id {
            Some(cid) => match self.router.position(|c| c.context_id == cid) {
                Some(idx) => idx,
                None => {
                    log::warn!("{op}: context_id={cid} not found — falling back to active context");
                    self.router.active_idx()
                }
            },
            None => self.router.active_idx(),
        }
    }

    /// Set the `root` of a context. `context_id` targets a specific context
    /// (the caller's PLEXI_CONTEXT_ID over IPC); `None` means the active one.
    pub(crate) fn set_context_root(&mut self, root: PathBuf, context_id: Option<u64>) {
        let idx = self.resolve_context_idx(context_id, "set_context_root");
        log::info!(
            "set_context_root: ctx_id={} root={}",
            self.router.get(idx).context_id,
            root.display()
        );
        auto_init_workspace(&root);
        let context_id = self.router.get(idx).context_id;
        for pane_id in self
            .windows
            .iter()
            .filter(|window| window.context_id == context_id)
            .flat_map(|window| window.panes.keys())
        {
            crate::app::host_mcp::rebind_pane_credential(*pane_id, root.clone());
        }
        self.router.get_mut(idx).root = root;
        // Transition effects (registry rescan, watcher restart, agent reload)
        // only apply when the *active* context's root changed.
        if idx == self.router.active_idx() {
            self.apply_context_transition_effects();
        }
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
        let root = self.router.active().root.clone();
        let context_id = self.router.active().context_id;
        log::info!(
            "transition_context: ctx_id={context_id} root={} — rescanning registry off-thread + reloading config + restarting watcher",
            root.display()
        );
        // The full registry rescan (apps/agents dir walk + every manifest.toml
        // parse) is a filesystem-bound operation that can stall the UI thread
        // on a workspace with many apps. Spawn it off-thread and apply the
        // result later via `registry_load_rx`, guarded by `context_id` so a
        // stale result (user already navigated elsewhere) is dropped instead
        // of applied (stint 0548).
        let (tx, rx) = crate::app::ui_mailbox::UiMailbox::channel(
            std::sync::Arc::clone(&self.ui_wake),
            "registry_load",
        );
        self.registry_load_rx = Some(rx);
        let load_root = root.clone();
        std::thread::spawn(move || {
            let start = std::time::Instant::now();
            let registry = crate::app::registry::AppRegistry::load(&load_root);
            log::debug!(
                "transition_context: registry load thread finished in {:?} (ctx_id={context_id})",
                start.elapsed()
            );
            let _ = tx.send((context_id, registry));
        });
        // Same workspace resolution `AppRegistry::load` performs internally
        // (cwd-walk to the channel dir) — cheap enough to run synchronously,
        // and agents live in that workspace's `agents/` dir, so the agent host
        // reload doesn't need to wait on the background registry scan.
        self.agent_host
            .reload_workspace(crate::app::registry::resolve_workspace_root(&root));
        let watch_dirs = crate::app::registry::registry_watch_dirs(&root);
        match crate::app::registry_watcher::start(watch_dirs, std::sync::Arc::clone(&self.ui_wake))
        {
            Some((watcher, rx)) => {
                self._registry_watcher = Some(watcher);
                self.registry_reload_rx = Some(rx);
            }
            None => {
                self._registry_watcher = None;
                self.registry_reload_rx = None;
            }
        }
        // Reload config so workspace-scoped config.toml applies immediately —
        // both on initial launch and on context switches.
        self.reload_config_for_active_context();
    }

    /// Drain the background `AppRegistry::load` result queued by
    /// [`Self::apply_context_transition_effects`]. Only the newest queued
    /// result is applied — anything older is superseded. Returns `true` when
    /// a result was applied or discarded (i.e. a message was drained at
    /// all), so tests can spin-wait on this instead of guessing a sleep.
    pub(crate) fn drain_registry_load(&mut self) -> bool {
        let Some(rx) = &self.registry_load_rx else {
            return false;
        };
        let mut latest = None;
        while let Ok((loaded_ctx_id, registry)) = rx.try_recv() {
            latest = Some((loaded_ctx_id, registry));
        }
        let Some((loaded_ctx_id, registry)) = latest else {
            return false;
        };
        if self.router.active().context_id == loaded_ctx_id {
            let app_count = registry.list().len();
            self.registry = registry;
            log::info!(
                "transition_context: registry reload applied for ctx_id={loaded_ctx_id} ({app_count} apps)"
            );
        } else {
            log::debug!(
                "transition_context: discarding stale registry load for ctx_id={loaded_ctx_id} — active context is now {}",
                self.router.active().context_id
            );
        }
        true
    }

    /// True when the active window holds a Portal tile targeting `child_ctx_id`,
    /// which is precisely what [`Self::dissolve_portal`] needs to do anything.
    /// Offer Dissolve only when this holds — a top-level context has no parent
    /// portal, so the action would early-return and visibly do nothing.
    pub(crate) fn context_has_portal(&self, child_ctx_id: u64) -> bool {
        self.windows[self.active_window]
            .panes
            .values()
            .any(|pane| pane.portal_target() == Some(child_ctx_id))
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

        for window in self.windows.iter().filter(|window| {
            window.window_id == primary_child_window_id || window.context_id == child_ctx_id
        }) {
            // The primary window's panes were grafted into the parent above,
            // and promoted windows were rebound to the parent context. Revoke
            // only credentials for panes still present in windows that this
            // retain is about to destroy.
            revoke_window_pane_credentials(window);
        }
        self.windows.retain(|window| {
            window.window_id != primary_child_window_id && window.context_id != child_ctx_id
        });
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
            self.reload_config_for_active_context();
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

    /// Mark the workspace dirty so the debounce flush in `update_preamble`
    /// picks it up on its next tick, instead of writing to disk synchronously
    /// on the UI thread for every call site that used to call
    /// `save_workspace_now` directly. No I/O here — just a flag.
    ///
    /// The debounce flush only runs inside a frame, and Plexi legitimately
    /// produces zero frames while fully idle (App Nap trap). Without a
    /// scheduled wake, a mutation that happens to be the host's last activity
    /// before it goes idle would never reach a frame where the debounce
    /// deadline is checked, and the workspace would stay unsaved
    /// indefinitely. On the 0→1 transition, arm a one-shot background wake
    /// through the sanctioned `ui_wake` seam (never a raw `request_repaint()`)
    /// timed to land after the debounce window, guaranteeing at least one
    /// later frame observes `workspace_dirty` and flushes it.
    pub(crate) fn mark_workspace_dirty(&mut self) {
        let was_clean = !self.workspace_dirty;
        self.workspace_dirty = true;
        if was_clean {
            let wake = std::sync::Arc::clone(&self.ui_wake);
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(
                    crate::app::render::WORKSPACE_SAVE_DEBOUNCE_MS,
                ));
                wake.wake("workspace_dirty_flush");
            });
        }
    }

    pub(crate) fn save_workspace_now(&self) {
        crate::platform::logging::mark_ui_phase(
            &self.ui_phase,
            crate::platform::logging::UiPhase::WorkspaceSave,
        );
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
                        heartbeat: self.pane_heartbeats.get(&id).map(|heartbeat| {
                            crate::workspace::SavedPaneHeartbeat {
                                every_ms: heartbeat
                                    .every
                                    .as_millis()
                                    .try_into()
                                    .unwrap_or(u64::MAX),
                                text: heartbeat.text.clone(),
                                while_idle_only: heartbeat.while_idle_only,
                            }
                        }),
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
                        heartbeat: self.pane_heartbeats.get(&id).map(|heartbeat| {
                            crate::workspace::SavedPaneHeartbeat {
                                every_ms: heartbeat
                                    .every
                                    .as_millis()
                                    .try_into()
                                    .unwrap_or(u64::MAX),
                                text: heartbeat.text.clone(),
                                while_idle_only: heartbeat.while_idle_only,
                            }
                        }),
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
                        heartbeat: self.pane_heartbeats.get(&id).map(|heartbeat| {
                            crate::workspace::SavedPaneHeartbeat {
                                every_ms: heartbeat
                                    .every
                                    .as_millis()
                                    .try_into()
                                    .unwrap_or(u64::MAX),
                                text: heartbeat.text.clone(),
                                while_idle_only: heartbeat.while_idle_only,
                            }
                        }),
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
                    heartbeat: None,
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

    #[test]
    fn auto_init_existing_channel_workspace_ensures_neutral_app_state_ignore() {
        let root = tempfile::tempdir().expect("root");
        let channel_dir = crate::config::workspace_channel_dir();
        let channel_path = root.path().join(channel_dir);
        std::fs::create_dir_all(&channel_path).expect("channel workspace");
        std::fs::write(channel_path.join("workspace.toml"), "id = \"existing\"\n")
            .expect("workspace");
        let neutral_dir = root.path().join(".plexi");
        std::fs::create_dir_all(&neutral_dir).expect("neutral dir");
        std::fs::write(neutral_dir.join(".gitignore"), "# user rule\nbuild/\n").expect("gitignore");

        auto_init_workspace(root.path());

        assert_eq!(
            std::fs::read_to_string(neutral_dir.join(".gitignore")).expect("read gitignore"),
            "# user rule\nbuild/\napp_states/\n"
        );
    }
}
