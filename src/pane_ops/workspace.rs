//! Multi-context and workspace persistence: new context, reset, delete,
//! and on-disk workspace save.

use crate::app::PlexiApp;
use crate::context::Window;
use crate::shell;
use crate::workspace::WorkspaceFile;
use std::path::PathBuf;

impl PlexiApp {
    pub(crate) fn new_context(&mut self) {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let Some((tree, panes, root_tile)) = self.create_single_pane_tree(Some(home.clone()))
        else {
            log::error!("Failed to create terminal for new context");
            return;
        };

        let ctx_id = self.next_window_id;
        self.next_window_id += 1;
        let win_id = self.next_window_id;
        self.next_window_id += 1;

        let ctx_name = format!("Context {}", self.contexts.len() + 1);
        self.contexts.push(crate::context::Context {
            name: ctx_name,
            path: home.clone(),
            context_id: ctx_id,
        });
        self.windows.push(Window {
            name: String::new(),
            path: home,
            tree,
            panes,
            focused_pane: Some(root_tile),
            zoomed_pane: None,
            grid_x: 0,
            grid_y: 0,
            window_id: win_id,
            context_id: ctx_id,
        });
        self.active_context = self.contexts.len() - 1;
        self.active_window = self.windows.len() - 1;
        self.context_active_window.insert(ctx_id, win_id);
        self.minimap.visible = false;

        let new_ctx_idx = self.contexts.len() - 1;
        let _ = new_ctx_idx;
    }

    /// Create a new page immediately to the right of the active page on the
    /// same grid row, then switch to it.
    pub(crate) fn new_page_right(&mut self) {
        let ws_id = self.contexts[self.active_context].context_id;
        let active_y = self.windows[self.active_window].grid_y;
        let max_x = self.windows.iter()
            .filter(|c| c.context_id == ws_id && c.grid_y == active_y)
            .map(|c| c.grid_x)
            .max();
        let new_x = match max_x {
            Some(x) => x + 1,
            None => 1,
        };
        self.create_page_at(new_x, active_y);
    }

    /// Shared creation helper: create a single-pane context at `(grid_x, grid_y)`
    /// and make it the active context.
    pub(crate) fn create_page_at(&mut self, grid_x: u32, grid_y: u32) {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let Some((tree, panes, root_tile)) = self.create_single_pane_tree(Some(home.clone()))
        else {
            log::error!("Failed to create terminal for new page at ({grid_x}, {grid_y})");
            return;
        };
        let name = String::new();
        let ctx_id = self.contexts[self.active_context].context_id;
        let win_id = self.next_window_id;
        self.next_window_id += 1;
        self.windows.push(Window {
            name,
            path: home,
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
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let Some((tree, panes, root_tile)) = self.create_single_pane_tree(Some(home.clone()))
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
        if self.contexts.len() <= 1 {
            return;
        }
        let ws_id = self.contexts[ws_index].context_id;

        // Remove all contexts belonging to this workspace.
        self.windows.retain(|c| c.context_id != ws_id);
        self.contexts.remove(ws_index);

        // Fix active indices.
        if self.active_context >= self.contexts.len() {
            self.active_context = self.contexts.len().saturating_sub(1);
        } else if self.active_context > ws_index {
            self.active_context -= 1;
        }
        // Pick a valid active_context in the new active workspace.
        self.pick_active_context_from_workspace();

        // Notifications scoped to this workspace are dropped.
        self.pending_notifications.retain(|n| {
            !(matches!(n.scope, crate::app_protocol::NotifyScope::Context)
                && n.source_context == ws_index)
        });
        if let Some(ref id) = self.current_notify_id.clone() {
            let still_present = self.pending_notifications.iter().any(|n| &n.notify_id == id);
            if !still_present {
                self.current_notify_id = None;
            }
        }

        // Restore minimap state for the workspace we landed on.
        let new_ws_id = self.contexts[self.active_context].context_id;
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

        // If workspace now has no windows, remove it too.
        let ws_has_windows = self.windows.iter().any(|c| c.context_id == removed_ws_id);
        if !ws_has_windows {
            if let Some(ws_idx) = self.contexts.iter().position(|w| w.context_id == removed_ws_id) {
                self.contexts.remove(ws_idx);
                if self.active_context >= self.contexts.len() {
                    self.active_context = self.contexts.len().saturating_sub(1);
                } else if self.active_context > ws_idx {
                    self.active_context -= 1;
                }
            }
        }

        if was_active {
            self.active_window = self.nearest_context_after_delete(removed_x, removed_y);
            // Sync context to the new active window.
            let new_ctx_id = self.windows[self.active_window].context_id;
            if let Some(ctx_idx) = self.contexts.iter().position(|w| w.context_id == new_ctx_id) {
                self.active_context = ctx_idx;
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

        // Restore minimap state for the workspace we landed on.
        let ws_id = self.contexts[self.active_context].context_id;
        let page_count = self.windows.iter().filter(|c| c.context_id == ws_id).count();
        self.minimap.visible = self
            .minimap_visible_per_context
            .get(&ws_id)
            .copied()
            .unwrap_or(page_count > 1);
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

    /// Switch the active workspace to `new_ws_idx`, saving the current
    /// workspace's minimap state and restoring the target workspace's saved
    /// state. Falls back to `visible = (page count > 1)` on first visit.
    ///
    /// This is the **only** place workspace switching should be performed —
    /// do not inline `active_workspace = i` + `pick_active_context_from_workspace`
    /// elsewhere, as that bypasses the save/restore.
    pub(crate) fn switch_workspace(&mut self, new_ctx_idx: usize) {
        // Save current context's active window + minimap state.
        let old_ctx_id = self.contexts[self.active_context].context_id;
        self.context_active_window
            .insert(old_ctx_id, self.windows[self.active_window].window_id);
        self.minimap_visible_per_context
            .insert(old_ctx_id, self.minimap.visible);

        self.active_context = new_ctx_idx;
        self.pick_active_context_from_workspace();

        // Restore minimap state for the new context.
        let new_ctx_id = self.contexts[self.active_context].context_id;
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
        let ctx_id = self.contexts[self.active_context].context_id;
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

    pub(crate) fn save_workspace(&self) {
        let mut saved_contexts = Vec::new();
        let mut saved_windows = Vec::new();

        for ctx in &self.contexts {
            saved_contexts.push(crate::workspace::SavedContext {
                name: ctx.name.clone(),
                path: ctx.path.clone(),
                context_id: ctx.context_id,
            });
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
                        agent_workspace: None,
                    });
                } else if let Some(a) = pane.as_app() {
                    saved_panes.push(crate::workspace::SavedPane {
                        id,
                        kind: crate::workspace::SavedPaneKind::App,
                        cwd: a.workspace_root.clone(),
                        name: Some(a.name.clone()),
                        app_id: Some(a.runtime.type_id().to_string()),
                        app_state: a.runtime.serialize_state(),
                        agent_workspace: None,
                    });
                } else if let Some(ag) = pane.as_agent() {
                    saved_panes.push(crate::workspace::SavedPane {
                        id,
                        kind: crate::workspace::SavedPaneKind::Agent,
                        cwd: ag.cwd(),
                        name: None,
                        app_id: None,
                        app_state: None,
                        agent_workspace: None,
                    });
                } else if let Some(w) = pane.as_agent_workspace() {
                    saved_panes.push(crate::workspace::SavedPane {
                        id,
                        kind: crate::workspace::SavedPaneKind::AgentWorkspace,
                        cwd: w.worktree_path.clone(),
                        name: None,
                        app_id: None,
                        app_state: None,
                        agent_workspace: Some(crate::workspace::SavedAgentWorkspace {
                            cli: w.cli,
                            repo_path: w.repo_path.clone(),
                            branch_name: w.branch_name.clone(),
                            worktree_path: w.worktree_path.clone(),
                            task_label: w.task_label.clone(),
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
            active_context: self.active_context,
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
