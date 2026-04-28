//! Multi-context and workspace persistence: new context, reset, delete,
//! and on-disk workspace save.

use crate::app::PlexiApp;
use crate::context::Context;
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

        let ws_id = self.next_context_id;
        self.next_context_id += 1;
        let ctx_id = self.next_context_id;
        self.next_context_id += 1;

        let name = format!("Context {}", self.workspaces.len() + 1);
        self.workspaces.push(crate::context::WorkspaceMeta {
            name: name.clone(),
            path: home.clone(),
            context_id: ws_id,
        });
        self.contexts.push(Context {
            name,
            path: home,
            tree,
            panes,
            focused_pane: Some(root_tile),
            zoomed_pane: None,
            grid_x: 0,
            grid_y: 0,
            context_id: ctx_id,
            workspace_id: ws_id,
        });
        self.active_workspace = self.workspaces.len() - 1;
        self.active_context = self.contexts.len() - 1;
        self.workspace_active_window.insert(ws_id, ctx_id);
        self.minimap.visible = false;

        // Auto-open inline rename so the user can name the context immediately.
        let new_ws_idx = self.workspaces.len() - 1;
        self.renaming_context = Some(new_ws_idx);
        self.rename_buffer = self.workspaces[new_ws_idx].name.clone();
    }

    /// Create a new page immediately to the right of the active page on the
    /// same grid row, then switch to it.
    pub(crate) fn new_page_right(&mut self) {
        let ws_id = self.workspaces[self.active_workspace].context_id;
        let active_y = self.contexts[self.active_context].grid_y;
        let max_x = self.contexts.iter()
            .filter(|c| c.workspace_id == ws_id && c.grid_y == active_y)
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
        let name = format!("Page {},{}", grid_x, grid_y);
        let ws_id = self.workspaces[self.active_workspace].context_id;
        let ctx_id = self.next_context_id;
        self.next_context_id += 1;
        self.contexts.push(Context {
            name,
            path: home,
            tree,
            panes,
            focused_pane: Some(root_tile),
            zoomed_pane: None,
            grid_x,
            grid_y,
            context_id: ctx_id,
            workspace_id: ws_id,
        });
        self.active_context = self.contexts.len() - 1;
        self.workspace_active_window.insert(ws_id, ctx_id);
        self.minimap.visible = true;
    }

    pub(crate) fn reset_active_context(&mut self) {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let Some((tree, panes, root_tile)) = self.create_single_pane_tree(Some(home.clone()))
        else {
            log::error!("Failed to create terminal for reset context");
            return;
        };

        let ctx = &mut self.contexts[self.active_context];
        ctx.tree = tree;
        ctx.panes = panes;
        ctx.focused_pane = Some(root_tile);
        ctx.zoomed_pane = None;
    }

    pub(crate) fn delete_workspace(&mut self, ws_index: usize) {
        if self.workspaces.len() <= 1 {
            return;
        }
        let ws_id = self.workspaces[ws_index].context_id;

        // Remove all contexts belonging to this workspace.
        self.contexts.retain(|c| c.workspace_id != ws_id);
        self.workspaces.remove(ws_index);

        // Fix active indices.
        if self.active_workspace >= self.workspaces.len() {
            self.active_workspace = self.workspaces.len().saturating_sub(1);
        } else if self.active_workspace > ws_index {
            self.active_workspace -= 1;
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
        let new_ws_id = self.workspaces[self.active_workspace].context_id;
        let page_count = self.contexts.iter().filter(|c| c.workspace_id == new_ws_id).count();
        self.minimap.visible = self
            .minimap_visible_per_workspace
            .get(&new_ws_id)
            .copied()
            .unwrap_or(page_count > 1);
    }

    pub(crate) fn delete_context(&mut self, index: usize) {
        if self.contexts.len() <= 1 {
            return;
        }
        let removed_ws_id = self.contexts[index].workspace_id;
        let was_active = self.active_context == index;
        let removed_x = self.contexts[index].grid_x;
        let removed_y = self.contexts[index].grid_y;

        self.contexts.remove(index);

        // If workspace now has no windows, remove it too.
        let ws_has_windows = self.contexts.iter().any(|c| c.workspace_id == removed_ws_id);
        if !ws_has_windows {
            if let Some(ws_idx) = self.workspaces.iter().position(|w| w.context_id == removed_ws_id) {
                self.workspaces.remove(ws_idx);
                if self.active_workspace >= self.workspaces.len() {
                    self.active_workspace = self.workspaces.len().saturating_sub(1);
                } else if self.active_workspace > ws_idx {
                    self.active_workspace -= 1;
                }
            }
        }

        if was_active {
            self.active_context = self.nearest_context_after_delete(removed_x, removed_y);
            // Sync workspace to the new active context.
            let new_ws_id = self.contexts[self.active_context].workspace_id;
            if let Some(ws_idx) = self.workspaces.iter().position(|w| w.context_id == new_ws_id) {
                self.active_workspace = ws_idx;
                self.workspace_active_window.insert(new_ws_id, self.contexts[self.active_context].context_id);
            }
        } else if self.active_context >= self.contexts.len() {
            self.active_context = self.contexts.len() - 1;
        } else if self.active_context > index {
            self.active_context -= 1;
        }

        if self.renaming_context == Some(index) {
            self.renaming_context = None;
        } else if let Some(r) = self.renaming_context {
            if r > index {
                self.renaming_context = Some(r - 1);
            }
        }

        // ── Compact the grid: shift columns left if a column becomes empty ──
        self.compact_workspace_grid(removed_ws_id);

        // Restore minimap state for the workspace we landed on.
        let ws_id = self.workspaces[self.active_workspace].context_id;
        let page_count = self.contexts.iter().filter(|c| c.workspace_id == ws_id).count();
        self.minimap.visible = self
            .minimap_visible_per_workspace
            .get(&ws_id)
            .copied()
            .unwrap_or(page_count > 1);
    }

    /// After a deletion, compact each row independently: within each row,
    /// shift windows left to close any gaps in grid_x. This ensures that
    /// deleting the left-most window in a row causes the rest to slide over.
    fn compact_workspace_grid(&mut self, ws_id: u64) {
        // Collect positions: (context_id, grid_y, grid_x)
        let entries: Vec<(u64, u32, u32)> = self.contexts.iter()
            .filter(|c| c.workspace_id == ws_id)
            .map(|c| (c.context_id, c.grid_y, c.grid_x))
            .collect();

        // Group by row (grid_y)
        let mut by_row: std::collections::HashMap<u32, Vec<(u64, u32)>> = std::collections::HashMap::new();
        for (ctx_id, y, x) in entries {
            by_row.entry(y).or_default().push((ctx_id, x));
        }

        // For each row, sort by x and assign sequential new_x = 0,1,2,...
        let mut updates: Vec<(u64, u32)> = Vec::new();
        for (_, mut items) in by_row {
            items.sort_by_key(|(_, x)| *x);
            for (new_x, (ctx_id, _)) in items.iter().enumerate() {
                updates.push((*ctx_id, new_x as u32));
            }
        }

        // Apply updates
        let mut old_to_new: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
        for ctx in self.contexts.iter_mut() {
            if let Some(&(_, new_x)) = updates.iter().find(|(id, _)| *id == ctx.context_id) {
                if ctx.grid_x != new_x {
                    old_to_new.insert(ctx.grid_x, new_x);
                    ctx.grid_x = new_x;
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
    pub(crate) fn switch_workspace(&mut self, new_ws_idx: usize) {
        // Save current workspace's active window + minimap state.
        let old_ws_id = self.workspaces[self.active_workspace].context_id;
        self.workspace_active_window
            .insert(old_ws_id, self.contexts[self.active_context].context_id);
        self.minimap_visible_per_workspace
            .insert(old_ws_id, self.minimap.visible);

        self.active_workspace = new_ws_idx;
        self.pick_active_context_from_workspace();

        // Restore minimap state for the new workspace.
        let new_ws_id = self.workspaces[self.active_workspace].context_id;
        let page_count = self
            .contexts
            .iter()
            .filter(|c| c.workspace_id == new_ws_id)
            .count();
        self.minimap.visible = self
            .minimap_visible_per_workspace
            .get(&new_ws_id)
            .copied()
            .unwrap_or(page_count > 1);
    }

    pub(crate) fn pick_active_context_from_workspace(&mut self) {
        let ws_id = self.workspaces[self.active_workspace].context_id;
        let preferred = self.workspace_active_window.get(&ws_id).copied();
        if let Some(ctx_id) = preferred {
            if let Some(idx) = self.contexts.iter().position(|c| c.context_id == ctx_id && c.workspace_id == ws_id) {
                self.active_context = idx;
                return;
            }
        }
        if let Some(idx) = self.contexts.iter().position(|c| c.workspace_id == ws_id) {
            self.active_context = idx;
            self.workspace_active_window.insert(ws_id, self.contexts[idx].context_id);
        }
    }

    pub(crate) fn save_workspace(&self) {
        let mut saved_workspaces = Vec::new();
        let mut saved_contexts = Vec::new();

        for ws in &self.workspaces {
            saved_workspaces.push(crate::workspace::SavedWorkspaceMeta {
                name: ws.name.clone(),
                path: ws.path.clone(),
                context_id: ws.context_id,
            });
        }

        for context in &self.contexts {
            let mut saved_panes = Vec::new();
            for (&id, pane) in &context.panes {
                debug_assert_eq!(pane.id(), id);
                if let Some(t) = pane.as_terminal() {
                    let cwd = shell::get_pid_cwd(t.backend.child_pid())
                        .unwrap_or_else(|| context.path.clone());
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
            saved_contexts.push(crate::workspace::SavedContext {
                name: context.name.clone(),
                path: context.path.clone(),
                tree: context.tree.clone(),
                panes: saved_panes,
                focused_pane: context.focused_pane,
                grid_x: context.grid_x,
                grid_y: context.grid_y,
                context_id: context.context_id,
                workspace_id: context.workspace_id,
            });
        }

        let ws = WorkspaceFile {
            version: 2,
            active_workspace: self.active_workspace,
            sidebar_visible: self.sidebar_visible,
            next_pane_id: self.host.next_pane_id(),
            workspaces: saved_workspaces,
            contexts: saved_contexts,
            workspace_active_window: self.workspace_active_window.clone(),
        };

        if let Err(e) = ws.save() {
            log::error!("Failed to save workspace: {e}");
        }
    }
}
