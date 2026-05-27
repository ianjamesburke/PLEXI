//! Multi-context and workspace persistence: new context, reset, delete,
//! and on-disk workspace save.

use crate::app::PlexiApp;
use crate::context::Window;
use crate::shell;
use crate::workspace::WorkspaceFile;
use std::path::PathBuf;

impl PlexiApp {
    /// Create a child context nested inside `parent_name`. Always creates a fresh
    /// terminal in the child (portal model — no pane adoption). Inserts a Portal
    /// tile into the parent window as a sibling of the focused tile. No depth cap.
    pub(crate) fn new_child_context(&mut self, parent_name: &str, path: PathBuf) -> Result<(), String> {
        let parent_idx = self.router.position(|c| c.name.eq_ignore_ascii_case(parent_name))
            .ok_or_else(|| format!("no context named '{parent_name}'"))?;
        let parent_id = self.router.get(parent_idx).context_id;
        let parent_depth = self.router.get(parent_idx).depth;
        let child_depth = parent_depth + 1;

        let ctx_id = self.next_window_id;
        self.next_window_id += 1;
        let win_id = self.next_window_id;
        self.next_window_id += 1;

        // Check for anchor defaults from .plexi/workspace.toml [context] section.
        let anchor = crate::anchor::Anchor::detect(&path);
        let (ctx_name, ctx_description) = match anchor.as_ref().and_then(|a| a.context_defaults.as_ref()) {
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
        let parent_win_idx = {
            let preferred = self.context_active_window.get(&parent_id).copied();
            preferred
                .and_then(|wid| self.windows.iter().position(|w| w.window_id == wid && w.context_id == parent_id))
                .or_else(|| self.windows.iter().position(|w| w.context_id == parent_id))
        };
        let sub_ctx_pane_id = self.host.alloc_pane_id();
        if let Some(parent_win_idx) = parent_win_idx {
            let split_target = self.windows[parent_win_idx].focused_pane;
            crate::pane_ops::layout::insert_split_tile(
                &mut self.windows[parent_win_idx].tree,
                split_target,
                sub_ctx_pane_id,
                true, // vertical = side-by-side
                crate::host::command::ShareRatio { numerator: 1.0, denominator: 1.0 },
                false,
            );
            self.windows[parent_win_idx].panes.insert(
                sub_ctx_pane_id,
                crate::pane::Pane::Portal(Box::new(crate::pane::PortalPane {
                    pane_id: sub_ctx_pane_id,
                    target_context_id: ctx_id,
                    context_state: None,
                })),
            );
        } else {
            log::warn!("new_child_context: parent ctx_id={parent_id} has no window — child context has no Portal tile");
        }

        // 3. Register the child context + window.
        self.router.push(crate::context::Context {
            name: ctx_name,
            path: path.clone(),
            root: Some(path.clone()),
            description: ctx_description,
            context_id: ctx_id,
            parent_id: Some(parent_id),
            depth: child_depth,
        });
        self.windows.push(crate::context::Window {
            name: String::new(),
            path: path.clone(),
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

    pub(crate) fn new_context(&mut self) {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let cwd = self.windows[self.active_window]
            .focused_pane
            .and_then(|t| self.windows[self.active_window].get_focused_pane_cwd(t))
            .filter(|p| p != &PathBuf::from("/"))
            .unwrap_or_else(|| home.clone());
        log::info!("new_context: cwd={}", cwd.display());
        let Some((tree, panes, root_tile)) = self.create_single_pane_tree(Some(cwd.clone()), None, false)
        else {
            log::error!("Failed to create terminal for new context");
            return;
        };

        let ctx_id = self.next_window_id;
        self.next_window_id += 1;
        let win_id = self.next_window_id;
        self.next_window_id += 1;

        let ctx_name = format!("Context {}", self.router.len() + 1);
        self.router.push(crate::context::Context {
            name: ctx_name,
            path: cwd.clone(),
            root: Some(cwd.clone()),
            description: None,
            context_id: ctx_id,
            parent_id: None,
            depth: 0,
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

        // Auto-open inline rename so the user can name the context immediately.
        let new_ctx_idx = self.router.len() - 1;
        self.renaming_window = Some(new_ctx_idx);
        self.rename_buffer = self.router.get(new_ctx_idx).name.clone();
    }

    /// Create a new context at a specific directory path. The terminal pane
    /// opens at `path` and the context root is set to it. Named after the
    /// directory basename, unless the path has a `.plexi/workspace.toml` with
    /// `[context]` defaults. Callers must call `save_workspace()` afterward.
    pub(crate) fn new_context_at_path(&mut self, path: PathBuf) {
        log::info!("new_context_at_path: path={}", path.display());
        let Some((tree, panes, root_tile)) = self.create_single_pane_tree(Some(path.clone()), None, false)
        else {
            log::error!("new_context_at_path: failed to create terminal for {}", path.display());
            return;
        };

        let ctx_id = self.next_window_id;
        self.next_window_id += 1;
        let win_id = self.next_window_id;
        self.next_window_id += 1;

        // Check for anchor defaults from .plexi/workspace.toml [context] section.
        let anchor = crate::anchor::Anchor::detect(&path);
        let (ctx_name, ctx_description) = match anchor.as_ref().and_then(|a| a.context_defaults.as_ref()) {
            Some(defaults) => {
                let name = defaults.name.clone().unwrap_or_else(|| {
                    path.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| format!("Context {}", self.router.len() + 1))
                });
                log::info!(
                    "new_context_at_path: applying anchor defaults name={:?} description={:?}",
                    name, defaults.description
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

        self.router.push(crate::context::Context {
            name: ctx_name,
            path: path.clone(),
            root: Some(path.clone()),
            description: ctx_description,
            context_id: ctx_id,
            parent_id: None,
            depth: 0,
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
    }

    /// Create a new page immediately to the right of the active page on the
    /// same grid row, then switch to it.
    pub(crate) fn new_page_right(&mut self) {
        let ws_id = self.router.active().context_id;
        let active_y = self.windows[self.active_window].grid_y;
        let max_x = self.windows.iter()
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
    pub(crate) fn create_page_at(&mut self, grid_x: u32, grid_y: u32, initial_cmd: Option<&str>, close_on_exit: bool) {
        let old_window_id = self.windows[self.active_window].window_id;
        let old_focus = self.windows[self.active_window].focused_pane;
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let cwd = self.resolve_new_pane_cwd(None, self.windows[self.active_window].focused_pane)
            .filter(|p| p != &PathBuf::from("/"))
            .unwrap_or(home);
        log::info!("create_page_at({grid_x},{grid_y}): cwd={} context_root={:?} initial_cmd={initial_cmd:?} close_on_exit={close_on_exit}", cwd.display(), self.router.active().root);
        let Some((tree, panes, root_tile)) = self.create_single_pane_tree(Some(cwd.clone()), initial_cmd, close_on_exit)
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
        log::info!("reset_active_context: cwd={} context_root={:?}", cwd.display(), self.router.active().root);
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
            deleted.len() - 1, &deleted[1..]
        );

        // 3. Remove all windows belonging to any deleted context.
        self.windows.retain(|c| !deleted.contains(&c.context_id));

        // 4. Remove Portal tiles in surviving windows that point to any deleted ctx.
        for win in &mut self.windows {
            let portal_pane_ids: Vec<crate::tiling::PaneId> = win.panes.iter()
                .filter(|(_, p)| p.portal_target().map(|cid| deleted.contains(&cid)).unwrap_or(false))
                .map(|(id, _)| *id)
                .collect();
            for pane_id in portal_pane_ids {
                win.panes.remove(&pane_id);
                if let Some(tile_id) = win.tree.tiles.find_pane(&pane_id) {
                    win.tree.remove_recursively(tile_id);
                }
            }
        }

        // 5. Remove from router. Iterate until none remain (positions shift after each removal).
        loop {
            let next = self.router.position(|c| deleted.contains(&c.context_id));
            match next {
                Some(idx) => { self.router.remove_at(idx); }
                None => break,
            }
        }

        // 6. Clean depth_stack: drop entries pointing to any deleted context.
        let before = self.router.depth_stack.len();
        self.router.retain_depth_stack(|cid| !deleted.contains(&cid));
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
            let still_present = self.pending_notifications.iter().any(|n| &n.notify_id == id);
            if !still_present {
                self.current_notify_id = None;
            }
        }

        // 9. Restore minimap state for the context we landed on.
        let new_ws_id = self.router.active().context_id;
        let page_count = self.windows.iter().filter(|c| c.context_id == new_ws_id).count();
        self.minimap.visible = self
            .minimap_visible_per_context
            .get(&new_ws_id)
            .copied()
            .unwrap_or(page_count > 1);
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
                self.context_active_window.insert(removed_ws_id, replacement.window_id);
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
                self.context_active_window.insert(new_ctx_id, self.windows[self.active_window].window_id);
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
            let page_count = self.windows.iter().filter(|c| c.context_id == ws_id).count();
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
        let entries: Vec<(u64, u32, u32)> = self.windows.iter()
            .filter(|w| w.context_id == ctx_id)
            .map(|w| (w.window_id, w.grid_y, w.grid_x))
            .collect();

        // Group by row (grid_y)
        let mut by_row: std::collections::HashMap<u32, Vec<(u64, u32)>> = std::collections::HashMap::new();
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
    /// This is the **only** place context navigation should be performed —
    /// calling `router.set_active` directly bypasses the minimap save/restore.
    pub(crate) fn switch_workspace(&mut self, new_ctx_idx: usize) {
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
    }

    pub(crate) fn pick_active_context_from_workspace(&mut self) {
        let ctx_id = self.router.active().context_id;
        let preferred = self.context_active_window.get(&ctx_id).copied();
        if let Some(win_id) = preferred {
            if let Some(idx) = self.windows.iter().position(|w| w.window_id == win_id && w.context_id == ctx_id) {
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

    /// Set the `root` of the active context.
    pub(crate) fn set_active_context_root(&mut self, root: PathBuf) {
        let idx = self.router.active_idx();
        log::info!(
            "set_active_context_root: ctx_id={} root={}",
            self.router.active().context_id,
            root.display()
        );
        self.router.get_mut(idx).root = Some(root);
    }

    /// Extract the focused pane from the active window into a new child context.
    /// Creates a Portal tile at the old pane position, zooms into the new child context,
    /// and opens the rename overlay.
    pub(crate) fn extract_pane_to_subcontext(&mut self) {
        use egui_tiles::{Container, Tile};

        let active_win_idx = self.active_window;
        let focused_tile_id = match self.windows[active_win_idx].focused_pane {
            Some(t) => t,
            None => {
                log::warn!("extract_pane_to_subcontext: no focused pane");
                return;
            }
        };

        let focused_pane_id = match self.windows[active_win_idx].tree.tiles.get(focused_tile_id) {
            Some(Tile::Pane(p)) => *p,
            _ => {
                log::warn!("extract_pane_to_subcontext: focused tile is not a pane");
                return;
            }
        };

        // Bail if portal
        if self.windows[active_win_idx].panes.get(&focused_pane_id)
            .map(|p| p.portal_target().is_some())
            .unwrap_or(false)
        {
            log::warn!("extract_pane_to_subcontext: focused pane is a portal, cannot extract");
            return;
        }

        // Remove pane from parent window
        let pane = match self.windows[active_win_idx].panes.remove(&focused_pane_id) {
            Some(p) => p,
            None => {
                log::warn!("extract_pane_to_subcontext: pane {focused_pane_id} not found");
                return;
            }
        };

        // Determine path for new context from pane CWD
        let path = pane.as_terminal()
            .and_then(|t| shell::get_pid_cwd(t.backend.child_pid()))
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")));

        let child_ctx_id = self.next_window_id;
        self.next_window_id += 1;
        let child_win_id = self.next_window_id;
        self.next_window_id += 1;

        let parent_id = self.router.active().context_id;
        let parent_depth = self.router.active().depth;
        let child_depth = parent_depth + 1;
        let child_name = format!("Sub-context {}", self.router.len() + 1);

        log::info!(
            "extract_pane_to_subcontext: pane={focused_pane_id} → new ctx_id={child_ctx_id} depth={child_depth}"
        );

        // Build child window with the extracted pane
        let mut child_tree = egui_tiles::Tree::empty(format!("tree_{child_win_id}"));
        let child_tile_id = child_tree.tiles.insert_pane(focused_pane_id);
        child_tree.root = Some(child_tile_id);
        let mut child_panes = std::collections::HashMap::new();
        child_panes.insert(focused_pane_id, pane);

        // Insert Portal pane at the old tile position in parent window
        let portal_pane_id = self.host.alloc_pane_id();
        let portal_tile_id = self.windows[active_win_idx].tree.tiles.insert_pane(portal_pane_id);

        // Replace old tile with portal in the parent tree
        let parent_of_focused = self.windows[active_win_idx].tree.tiles.parent_of(focused_tile_id);
        if let Some(parent_tile) = parent_of_focused {
            if let Some(Tile::Container(container)) = self.windows[active_win_idx].tree.tiles.get_mut(parent_tile) {
                match container {
                    Container::Linear(lin) => {
                        if let Some(pos) = lin.children.iter().position(|&c| c == focused_tile_id) {
                            lin.children[pos] = portal_tile_id;
                        }
                    }
                    Container::Tabs(tabs) => {
                        if let Some(pos) = tabs.children.iter().position(|&c| c == focused_tile_id) {
                            tabs.children[pos] = portal_tile_id;
                        }
                    }
                    Container::Grid(_) => {
                        container.remove_child(focused_tile_id);
                        container.add_child(portal_tile_id);
                    }
                }
            }
        } else {
            // Was root tile
            self.windows[active_win_idx].tree.root = Some(portal_tile_id);
        }
        // Remove the focused tile from tiles
        self.windows[active_win_idx].tree.tiles.remove(focused_tile_id);

        // Insert portal pane into parent window
        self.windows[active_win_idx].panes.insert(
            portal_pane_id,
            crate::pane::Pane::Portal(Box::new(crate::pane::PortalPane {
                pane_id: portal_pane_id,
                target_context_id: child_ctx_id,
                context_state: None,
            })),
        );

        // Register child context
        self.router.push(crate::context::Context {
            name: child_name.clone(),
            path: path.clone(),
            root: Some(path.clone()),
            description: None,
            context_id: child_ctx_id,
            parent_id: Some(parent_id),
            depth: child_depth,
        });

        // Register child window
        let child_win = crate::context::Window {
            name: String::new(),
            path: path,
            tree: child_tree,
            panes: child_panes,
            focused_pane: Some(child_tile_id),
            zoomed_pane: None,
            grid_x: 0,
            grid_y: 0,
            window_id: child_win_id,
            context_id: child_ctx_id,
        };
        self.windows.push(child_win);
        self.context_active_window.insert(child_ctx_id, child_win_id);

        // Zoom into child context
        let current_win_id = self.windows[active_win_idx].window_id;
        self.router.push_depth(parent_id, current_win_id, Some(portal_tile_id));
        if let Some(ctx_idx) = self.router.position(|c| c.context_id == child_ctx_id) {
            self.switch_workspace(ctx_idx);
            // Open rename for the new context
            let new_idx = self.router.active_idx();
            self.renaming_window = Some(new_idx);
            self.rename_buffer = child_name;
            log::info!("extract_pane_to_subcontext: zoomed to ctx_id={child_ctx_id}, opening rename");
        }

        self.save_workspace();
    }

    /// Dissolve a portal: reparent all its panes into the parent window (the active
    /// window that contains the Portal tile), replace the Portal tile with the
    /// adopted panes, then delete the now-empty child context.
    ///
    /// Only promotes one level. Nested portals inside the dissolved context remain
    /// intact — their tiles are moved to the parent window alongside the regular panes.
    pub(crate) fn dissolve_portal(&mut self, child_ctx_id: u64) {
        use egui_tiles::{Container, Tile};

        // Find the Portal tile in the active (parent) window.
        let parent_idx = self.active_window;
        let portal_pane_id = {
            let win = &self.windows[parent_idx];
            win.panes.iter()
                .find(|(_, p)| p.portal_target() == Some(child_ctx_id))
                .map(|(id, _)| *id)
        };
        let portal_pane_id = match portal_pane_id {
            Some(id) => id,
            None => {
                log::warn!("dissolve_portal: no Portal tile for ctx={child_ctx_id} in active window");
                return;
            }
        };

        let portal_tile_id = self.windows[parent_idx].tree.tiles.find_pane(&portal_pane_id);
        let portal_tile_id = match portal_tile_id {
            Some(id) => id,
            None => {
                log::warn!("dissolve_portal: Portal pane {portal_pane_id} has no tile");
                return;
            }
        };

        // Collect panes from all child windows — extract them so we can move ownership.
        let child_win_indices: Vec<usize> = self.windows.iter().enumerate()
            .filter(|(_, w)| w.context_id == child_ctx_id)
            .map(|(i, _)| i)
            .collect();

        // Drain panes from child windows into a staging list (sorted by ID for deterministic order).
        let mut adopted: Vec<(crate::tiling::PaneId, crate::pane::Pane)> = Vec::new();
        for &win_idx in &child_win_indices {
            let mut pane_ids: Vec<crate::tiling::PaneId> = self.windows[win_idx].panes.keys().copied().collect();
            pane_ids.sort();
            for pane_id in pane_ids {
                if let Some(pane) = self.windows[win_idx].panes.remove(&pane_id) {
                    adopted.push((pane_id, pane));
                }
            }
        }

        log::info!("dissolve_portal: ctx={child_ctx_id} adopting {} panes into parent win={parent_idx}", adopted.len());

        // Insert adopted panes into the parent window.
        // Strategy: add each new tile alongside the Portal tile.
        // After all siblings are added, remove the Portal tile.
        for (pane_id, pane) in adopted {
            let new_tile = self.windows[parent_idx].tree.tiles.insert_pane(pane_id);
            self.windows[parent_idx].panes.insert(pane_id, pane);

            // Add the new tile as a sibling of the Portal tile.
            let parent_of_portal = self.windows[parent_idx].tree.tiles.parent_of(portal_tile_id);
            if let Some(parent_tile) = parent_of_portal {
                if let Some(Tile::Container(Container::Linear(lin))) =
                    self.windows[parent_idx].tree.tiles.get_mut(parent_tile)
                {
                    if let Some(pos) = lin.children.iter().position(|&c| c == portal_tile_id) {
                        lin.children.insert(pos, new_tile);
                    } else {
                        lin.children.push(new_tile);
                    }
                } else {
                    // Parent is Tabs or another container — append via a new horizontal split at root.
                    let existing_root = self.windows[parent_idx].tree.root;
                    if let Some(root) = existing_root {
                        let new_root = self.windows[parent_idx].tree.tiles.insert_horizontal_tile(vec![root, new_tile]);
                        self.windows[parent_idx].tree.root = Some(new_root);
                    } else {
                        self.windows[parent_idx].tree.root = Some(new_tile);
                    }
                }
            } else {
                // Portal tile is the root — wrap everything in a horizontal container.
                let new_root = self.windows[parent_idx].tree.tiles.insert_horizontal_tile(vec![new_tile, portal_tile_id]);
                self.windows[parent_idx].tree.root = Some(new_root);
            }
        }

        // Remove the Portal tile from the parent window.
        {
            let win = &mut self.windows[parent_idx];
            win.panes.remove(&portal_pane_id);
            if let Some(parent_tile) = win.tree.tiles.parent_of(portal_tile_id) {
                if let Some(Tile::Container(parent_container)) = win.tree.tiles.get_mut(parent_tile) {
                    parent_container.remove_child(portal_tile_id);
                }
            }
            win.tree.tiles.remove(portal_tile_id);
            win.tree.simplify(&egui_tiles::SimplificationOptions {
                all_panes_must_have_tabs: true,
                ..egui_tiles::SimplificationOptions::default()
            });
        }

        // Fix up active_window index BEFORE the retain shifts indices.
        // Count how many child windows sit before parent_idx — each removal shifts the index down by 1.
        let removed_before = child_win_indices.iter().filter(|&&i| i < parent_idx).count();

        // Remove child context windows and router entry.
        self.windows.retain(|w| w.context_id != child_ctx_id);
        if let Some(idx) = self.router.position(|c| c.context_id == child_ctx_id) {
            self.router.remove_at(idx);
        }

        self.active_window = parent_idx - removed_before;

        // Focus the first pane in the parent window.
        let new_focus = self.windows[self.active_window].tree.root
            .and_then(|root| self.windows[self.active_window].find_first_pane_in(root));
        self.windows[self.active_window].focused_pane = new_focus;
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
                    });
                } else if let Some(a) = pane.as_app() {
                    saved_panes.push(crate::workspace::SavedPane {
                        id,
                        kind: crate::workspace::SavedPaneKind::App,
                        cwd: a.workspace_root.clone(),
                        name: Some(a.name.clone()),
                        app_id: Some(a.runtime.type_id().to_string()),
                        app_state: a.runtime.serialize_state(),
                    });
                } else if let Some(child_ctx_id) = pane.portal_target() {
                    saved_panes.push(crate::workspace::SavedPane {
                        id,
                        kind: crate::workspace::SavedPaneKind::Portal { context_id: child_ctx_id },
                        cwd: std::path::PathBuf::new(),
                        name: None,
                        app_id: None,
                        app_state: None,
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
            next_pane_id: self.host.next_pane_id(),
            contexts: saved_contexts,
            windows: saved_windows,
            context_active_window: self.context_active_window.clone(),
        };

        if let Err(e) = ws.save() {
            log::error!("Failed to save workspace: {e}");
        }
    }
}
