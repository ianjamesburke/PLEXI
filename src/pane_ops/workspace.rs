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

        // Place sidebar-created contexts at (0, max_y + 1) so they don't
        // collide with any existing page. The spatial grid can grow in both
        // dimensions; sidebar creation always adds a new row.
        let max_y = self.contexts.iter().map(|c| c.grid_y).max().unwrap_or(0);
        let grid_y = if self.contexts.is_empty() { 0 } else { max_y + 1 };

        let name = format!("Context {}", self.contexts.len() + 1);
        self.contexts.push(Context {
            name,
            path: home,
            tree,
            panes,
            focused_pane: Some(root_tile),
            zoomed_pane: None,
            grid_x: 0,
            grid_y,
            spatial: false,
            context_id: self.next_context_id,
            parent_context_id: 0,
        });
        self.next_context_id += 1;
        self.active_context = self.contexts.len() - 1;
    }

    /// Create a new page immediately to the right of the active page on the
    /// same grid row, then switch to it.
    pub(crate) fn new_page_right(&mut self) {
        let active_y = self.contexts[self.active_context].grid_y;
        let max_x_on_row = self
            .contexts
            .iter()
            .filter(|c| c.grid_y == active_y)
            .map(|c| c.grid_x)
            .max()
            .unwrap_or(0);
        self.create_page_at(max_x_on_row + 1, active_y);
    }

    /// Create a new page at `(0, max_y + 1)` — starts a new row below all
    /// existing rows — then switch to it.
    pub(crate) fn new_page_next_row(&mut self) {
        let max_y = self.contexts.iter().map(|c| c.grid_y).max().unwrap_or(0);
        self.create_page_at(0, max_y + 1);
    }

    /// Shared creation helper: create a single-pane context at `(grid_x, grid_y)`
    /// and make it the active context.
    fn create_page_at(&mut self, grid_x: u32, grid_y: u32) {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let Some((tree, panes, root_tile)) = self.create_single_pane_tree(Some(home.clone()))
        else {
            log::error!("Failed to create terminal for new page at ({grid_x}, {grid_y})");
            return;
        };
        let name = format!("Page {},{}", grid_x, grid_y);
        let parent_context_id = self.contexts[self.active_sidebar_context].context_id;
        self.contexts.push(Context {
            name,
            path: home,
            tree,
            panes,
            focused_pane: Some(root_tile),
            zoomed_pane: None,
            grid_x,
            grid_y,
            spatial: true,
            context_id: self.next_context_id,
            parent_context_id,
        });
        self.next_context_id += 1;
        self.active_context = self.contexts.len() - 1;
        // Auto-show minimap once there are 2+ spatial pages for this sidebar context.
        let sidebar_ctx_id = self.contexts[self.active_sidebar_context].context_id;
        let spatial_count = self.contexts.iter()
            .filter(|c| c.spatial && c.parent_context_id == sidebar_ctx_id)
            .count();
        if spatial_count >= 2 {
            self.minimap.visible = true;
        }
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

    pub(crate) fn delete_context(&mut self, index: usize) {
        if self.contexts.len() <= 1 {
            return;
        }

        // If deleting a sidebar context, first remove all its spatial pages.
        if !self.contexts[index].spatial {
            let parent_id = self.contexts[index].context_id;
            let spatial_indices: Vec<usize> = self.contexts
                .iter()
                .enumerate()
                .filter(|(_, c)| c.spatial && c.parent_context_id == parent_id)
                .map(|(i, _)| i)
                .collect();
            // Remove in reverse order to keep indices valid.
            for &i in spatial_indices.iter().rev() {
                self.contexts.remove(i);
                // Adjust active_context if needed.
                if self.active_context > i && self.active_context > 0 {
                    self.active_context -= 1;
                } else if self.active_context == i {
                    self.active_context = 0;
                }
            }
        }

        // Guard again: cascaded removals may have left only 1 context.
        if self.contexts.len() <= 1 {
            return;
        }

        // Capture the removed page's grid coords before removal so we can find
        // the spatially nearest remaining page afterwards.
        let removed_x = self.contexts[index].grid_x;
        let removed_y = self.contexts[index].grid_y;
        let was_active = self.active_context == index;

        self.contexts.remove(index);

        // Adjust active_context: if the active page was deleted, jump to the
        // spatially nearest remaining page. Otherwise, just fix the index if
        // the removal shifted it.
        if was_active {
            self.active_context = self.nearest_context_after_delete(removed_x, removed_y);
        } else if self.active_context >= self.contexts.len() {
            self.active_context = self.contexts.len() - 1;
        } else if self.active_context > index {
            self.active_context -= 1;
        }
        // Clear rename state if it referenced the deleted context
        if self.renaming_context == Some(index) {
            self.renaming_context = None;
        } else if let Some(r) = self.renaming_context {
            if r > index {
                self.renaming_context = Some(r - 1);
            }
        }
        // Remove context-scoped notifications from the deleted context.
        // Global notifications survive (they aren't tied to any context).
        // Re-index source_context for entries that referenced contexts after
        // the deleted one (positional indices shifted down by one).
        self.pending_notifications.retain(|n| {
            !(matches!(n.scope, crate::app_protocol::NotifyScope::Context)
                && n.source_context == index)
        });
        for n in &mut self.pending_notifications {
            if n.source_context > index {
                n.source_context -= 1;
            }
        }
        // If the current notification was removed, clear the pin so the
        // next highest-priority visible one is picked on the next render.
        if let Some(ref id) = self.current_notify_id.clone() {
            let still_present = self.pending_notifications.iter().any(|n| &n.notify_id == id);
            if !still_present {
                self.current_notify_id = None;
            }
        }
        // Auto-hide minimap when fewer than 2 spatial pages remain for the current sidebar context.
        let sidebar_ctx_id = self.contexts
            .get(self.active_sidebar_context)
            .map(|c| c.context_id)
            .unwrap_or(0);
        if self.contexts.iter().filter(|c| c.spatial && c.parent_context_id == sidebar_ctx_id).count() < 2 {
            self.minimap.visible = false;
        }
    }

    pub(crate) fn save_workspace(&self) {
        let mut saved_contexts = Vec::new();
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
                spatial: context.spatial,
                context_id: context.context_id,
                parent_context_id: context.parent_context_id,
            });
        }

        let ws = WorkspaceFile {
            version: 1,
            active_context: self.active_context,
            sidebar_visible: self.sidebar_visible,
            next_pane_id: self.host.next_pane_id(),
            contexts: saved_contexts,
        };

        if let Err(e) = ws.save() {
            log::error!("Failed to save workspace: {e}");
        }
    }
}
