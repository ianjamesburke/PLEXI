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
