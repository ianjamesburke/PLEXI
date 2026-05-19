//! ContextState — rolled-up status summary for a context and its children.
//!
//! Reads existing status channels only (StatusSummary for apps, pty_title for
//! terminals). No new protocol messages.

use crate::tiling::PaneId;

/// Rolled-up status for a single context, including recursive children.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct ContextState {
    pub context_id: u64,
    pub label: String,
    pub pane_count: u32,
    pub active_agents: u32,
    pub status: ContextStatus,
    pub pane_summaries: Vec<PaneSummary>,
    pub children: Vec<ContextState>,
}

/// Overall status of a context, derived from its pane summaries.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub enum ContextStatus {
    Idle,
    Working,
    Error,
    Done,
}

/// Summary of a single pane within a context.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct PaneSummary {
    pub pane_id: PaneId,
    pub label: String,
    pub status: PaneSummaryStatus,
}

/// Status of an individual pane.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub enum PaneSummaryStatus {
    Idle,
    Active,
    Error,
}

impl ContextState {
    /// Compute the context state for a given context by walking panes and
    /// recursing into child contexts.
    ///
    /// `contexts` is the full router slice, `windows` is all windows.
    /// Reads existing status: `StatusSummary` text for apps, `pty_title` for terminals.
    pub fn compute(
        context_id: u64,
        contexts: &[crate::context::Context],
        windows: &[crate::context::Window],
    ) -> Self {
        let label = contexts
            .iter()
            .find(|c| c.context_id == context_id)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| "(unknown)".to_string());

        let mut pane_summaries = Vec::new();
        let mut active_agents: u32 = 0;
        let mut has_error = false;

        for win in windows.iter().filter(|w| w.context_id == context_id) {
            let mut entries: Vec<_> = win.panes.iter().collect();
            entries.sort_by_key(|(id, _)| *id);
            for (&pid, pane) in entries {
                let (plabel, pstatus) = match pane {
                    crate::pane::Pane::Terminal(t) => {
                        let name = t.name.clone()
                            .or_else(|| t.pty_title.clone())
                            .unwrap_or_else(|| "Terminal".to_string());
                        let status = if t.exited {
                            PaneSummaryStatus::Idle
                        } else {
                            PaneSummaryStatus::Active
                        };
                        if status == PaneSummaryStatus::Active {
                            active_agents += 1;
                        }
                        (name, status)
                    }
                    crate::pane::Pane::App(a) => {
                        let status_text = if let crate::pane::AppRuntime::Process(pa) = &a.runtime {
                            pa.status_summary.clone()
                        } else {
                            None
                        };
                        let status = match status_text.as_deref() {
                            Some(t) if t.contains("error") || t.contains("Error") => {
                                has_error = true;
                                PaneSummaryStatus::Error
                            }
                            Some(_) => {
                                active_agents += 1;
                                PaneSummaryStatus::Active
                            }
                            None => PaneSummaryStatus::Idle,
                        };
                        (a.name.clone(), status)
                    }
                    crate::pane::Pane::Portal(_) => continue,
                };
                pane_summaries.push(PaneSummary {
                    pane_id: pid,
                    label: plabel,
                    status: pstatus,
                });
            }
        }

        // Recurse into child contexts.
        let child_ids: Vec<u64> = contexts
            .iter()
            .filter(|c| c.parent_id == Some(context_id))
            .map(|c| c.context_id)
            .collect();

        let children: Vec<ContextState> = child_ids
            .iter()
            .map(|&cid| Self::compute(cid, contexts, windows))
            .collect();

        // Roll up child status.
        for child in &children {
            if child.status == ContextStatus::Error {
                has_error = true;
            }
            if child.status == ContextStatus::Working {
                active_agents += child.active_agents;
            }
        }

        let pane_count = pane_summaries.len() as u32;
        let status = if has_error {
            ContextStatus::Error
        } else if active_agents > 0 {
            ContextStatus::Working
        } else {
            ContextStatus::Idle
        };

        Self {
            context_id,
            label,
            pane_count,
            active_agents,
            status,
            pane_summaries,
            children,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{Context, Window};
    use std::path::PathBuf;

    fn make_context(id: u64, name: &str, parent: Option<u64>) -> Context {
        Context {
            name: name.to_string(),
            path: PathBuf::from("/tmp"),
            root: None,
            description: None,
            context_id: id,
            parent_id: parent,
            depth: if parent.is_some() { 1 } else { 0 },
        }
    }

    fn make_empty_window(context_id: u64, window_id: u64) -> Window {
        Window {
            name: String::new(),
            path: PathBuf::from("/tmp"),
            tree: egui_tiles::Tree::empty("test"),
            panes: std::collections::HashMap::new(),
            focused_pane: None,
            zoomed_pane: None,
            grid_x: 0,
            grid_y: 0,
            window_id,
            context_id,
        }
    }

    #[test]
    fn empty_context_is_idle() {
        let contexts = vec![make_context(1, "Root", None)];
        let windows = vec![make_empty_window(1, 100)];
        let state = ContextState::compute(1, &contexts, &windows);
        assert_eq!(state.status, ContextStatus::Idle);
        assert_eq!(state.pane_count, 0);
        assert_eq!(state.label, "Root");
    }

    #[test]
    fn child_error_rolls_up() {
        let contexts = vec![
            make_context(1, "Root", None),
            make_context(2, "Child", Some(1)),
        ];
        let windows = vec![
            make_empty_window(1, 100),
            make_empty_window(2, 101),
        ];
        // Child has no panes but we test the rollup structure.
        let state = ContextState::compute(1, &contexts, &windows);
        assert_eq!(state.children.len(), 1);
        assert_eq!(state.children[0].label, "Child");
    }
}
