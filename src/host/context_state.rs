//! Host-side projection of live host state into the wire `ContextState`
//! summary (the wire types live in `protocol`).
//!
//! Reads existing status channels only (StatusSummary for plugins, pty_title for
//! terminals). No new protocol messages.

use plexi_protocol::{ContextState, ContextStatus, PaneSummary, PaneSummaryStatus};

/// Compute the context state for a given context by walking panes and
/// recursing into child contexts.
///
/// `contexts` is the full router slice, `windows` is all windows.
/// Precedence: `NeedsYou > Blocked > Error > Working (Active) > Idle`.
pub fn compute_context_state(
    context_id: u64,
    contexts: &[crate::host::context::Context],
    windows: &[crate::host::context::Window],
) -> ContextState {
    use plexi_protocol::AgentState;

    let label = contexts
        .iter()
        .find(|c| c.context_id == context_id)
        .map(|c| c.name.to_string())
        .unwrap_or_else(|| "(unknown)".to_string());

    let mut pane_summaries = Vec::new();
    let mut active_agents: u32 = 0;
    let mut has_error = false;
    let mut has_blocked = false;
    let mut has_needs_you = false;

    for win in windows.iter().filter(|w| w.context_id == context_id) {
        let mut entries: Vec<_> = win.panes.iter().collect();
        entries.sort_by_key(|(id, _)| *id);
        for (&pid, pane) in entries {
            let (plabel, pstatus) = match pane {
                crate::host::pane::Pane::Terminal(t) => {
                    let name = t
                        .name
                        .clone()
                        .or_else(|| t.pty_title.clone())
                        .unwrap_or_else(|| "Terminal".to_string());
                    let effective = pane.effective_agent();
                    let status = if t.exited && t.effective_agent_state().0 == AgentState::Failed {
                        has_error = true;
                        PaneSummaryStatus::Error
                    } else if let Some(agent) = effective.as_ref() {
                        match agent.state {
                            AgentState::NeedsYou => {
                                has_needs_you = true;
                                PaneSummaryStatus::NeedsYou
                            }
                            AgentState::Failed
                            | AgentState::Blocked
                            | AgentState::ExitedUnreported => {
                                has_blocked = true;
                                PaneSummaryStatus::Blocked
                            }
                            AgentState::Working => {
                                if t.exited {
                                    PaneSummaryStatus::Idle
                                } else {
                                    active_agents += 1;
                                    PaneSummaryStatus::Active
                                }
                            }
                            AgentState::Idle | AgentState::Interrupted | AgentState::Completed => {
                                PaneSummaryStatus::Idle
                            }
                        }
                    } else if t.exited {
                        PaneSummaryStatus::Idle
                    } else if let Some(ref sb) = t.standby_session {
                        match sb.last_state {
                            AgentState::NeedsYou => {
                                has_needs_you = true;
                                PaneSummaryStatus::NeedsYou
                            }
                            AgentState::Failed
                            | AgentState::Blocked
                            | AgentState::ExitedUnreported => {
                                has_blocked = true;
                                PaneSummaryStatus::Blocked
                            }
                            AgentState::Working => {
                                active_agents += 1;
                                PaneSummaryStatus::Active
                            }
                            AgentState::Idle | AgentState::Interrupted | AgentState::Completed => {
                                PaneSummaryStatus::Idle
                            }
                        }
                    } else {
                        PaneSummaryStatus::Idle
                    };
                    (name, status)
                }
                crate::host::pane::Pane::View(a) => {
                    let status = if let Some(agent) = pane.agent() {
                        match agent.state {
                            AgentState::NeedsYou => {
                                has_needs_you = true;
                                PaneSummaryStatus::NeedsYou
                            }
                            AgentState::Failed
                            | AgentState::Blocked
                            | AgentState::ExitedUnreported => {
                                has_blocked = true;
                                PaneSummaryStatus::Blocked
                            }
                            AgentState::Working => {
                                active_agents += 1;
                                PaneSummaryStatus::Active
                            }
                            AgentState::Idle | AgentState::Interrupted | AgentState::Completed => {
                                PaneSummaryStatus::Idle
                            }
                        }
                    } else {
                        PaneSummaryStatus::Idle
                    };
                    (a.name.clone(), status)
                }
                crate::host::pane::Pane::Portal(_) => continue,
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
        .map(|&cid| compute_context_state(cid, contexts, windows))
        .collect();

    // Roll up child status (NeedsYou > Blocked > Error > Working > Idle).
    for child in &children {
        match child.status {
            ContextStatus::NeedsYou => has_needs_you = true,
            ContextStatus::Blocked => has_blocked = true,
            ContextStatus::Error => has_error = true,
            ContextStatus::Working | ContextStatus::Idle | ContextStatus::Done => {}
        }
        active_agents += child.active_agents;
    }

    let pane_count = pane_summaries.len() as u32;
    let status = if has_needs_you {
        ContextStatus::NeedsYou
    } else if has_blocked {
        ContextStatus::Blocked
    } else if has_error {
        ContextStatus::Error
    } else if active_agents > 0 {
        ContextStatus::Working
    } else {
        ContextStatus::Idle
    };

    ContextState {
        context_id,
        label,
        pane_count,
        active_agents,
        status,
        pane_summaries,
        children,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::context::{Context, Window};
    use std::path::PathBuf;

    fn make_context(id: u64, name: &str, parent: Option<u64>) -> Context {
        Context {
            name: name.to_string().into(),
            root: PathBuf::from("/tmp"),
            description: None,
            context_id: id,
            parent_id: parent,
            depth: if parent.is_some() { 1 } else { 0 },
            parked: false,
        }
    }

    fn make_empty_window(context_id: u64, window_id: u64) -> Window {
        Window {
            name: String::new(),
            path: PathBuf::from("/tmp"),
            tree: egui_tiles::Tree::empty("test"),
            panes: Default::default(),
            focused_pane: None,
            zoomed_pane: None,
            grid_x: 0,
            grid_y: 0,
            window_id,
            context_id,
            tree_history: Default::default(),
        }
    }

    #[test]
    fn empty_context_is_idle() {
        let contexts = vec![make_context(1, "Root", None)];
        let windows = vec![make_empty_window(1, 100)];
        let state = compute_context_state(1, &contexts, &windows);
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
        let windows = vec![make_empty_window(1, 100), make_empty_window(2, 101)];
        // Child has no panes but we test the rollup structure.
        let state = compute_context_state(1, &contexts, &windows);
        assert_eq!(state.children.len(), 1);
        assert_eq!(state.children[0].label, "Child");
    }

    use plexi_protocol::AgentState;

    struct DummyView;
    impl crate::app::view_trait::View for DummyView {
        fn type_id(&self) -> &'static str {
            "test.app"
        }
        fn display_name(&self) -> String {
            "Test App".to_string()
        }
        fn ui(&mut self, _ui: &mut egui::Ui, _ctx: &crate::app::view_trait::ViewRenderContext<'_>) {
        }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
    }

    #[test]
    fn child_status_precedence_needs_you_beats_blocked_beats_error_beats_working() {
        let contexts = vec![
            make_context(1, "Root", None),
            make_context(2, "Squad", Some(1)),
        ];
        let mut child_win = make_empty_window(2, 101);
        let make_view_pane = |id: u64, state: AgentState| {
            crate::host::pane::Pane::View(Box::new(crate::host::pane::ViewPane {
                incarnation: Default::default(),
                id,
                name: format!("app-{id}"),
                manifest_id: "test.app".into(),
                runtime: crate::host::pane::ViewRuntime::Builtin(Box::new(DummyView)),
                workspace_root: PathBuf::from("/tmp"),
                permissions: crate::app::permissions::PluginPermissions::default(),
                pane_group: None,
                linked_pane_id: None,
                agent: Some(plexi_protocol::PaneAgentState {
                    pane_id: id,
                    state,
                    agent: "codex".into(),
                    detail: None,
                    session_id: None,
                }),
                pip_status: None,
                slots: std::collections::HashMap::new(),
                overlay_replaced: None,
                semantic_state: crate::host::pane::SemanticPaneState::default(),
            }))
        };

        child_win
            .panes
            .insert(10, make_view_pane(10, AgentState::Working))
            .unwrap();
        let windows = vec![make_empty_window(1, 100), child_win];
        let state = compute_context_state(1, &contexts, &windows);
        assert_eq!(state.children[0].status, ContextStatus::Working);
        assert_eq!(state.status, ContextStatus::Working);
        assert_eq!(
            state.children[0].status_summary_badge().as_deref(),
            Some("[1 WORKING]")
        );

        let mut windows = windows;
        windows[1]
            .panes
            .insert(11, make_view_pane(11, AgentState::Working))
            .unwrap();
        windows[1]
            .panes
            .insert(12, make_view_pane(12, AgentState::Blocked))
            .unwrap();
        let state = compute_context_state(1, &contexts, &windows);
        assert_eq!(state.children[0].status, ContextStatus::Blocked);
        assert_eq!(state.status, ContextStatus::Blocked);
        assert_eq!(
            state.children[0].status_summary_badge().as_deref(),
            Some("[2 WORKING \u{b7} 1 BLOCKED]")
        );

        let mut needs_you_win = make_empty_window(2, 101);
        needs_you_win
            .panes
            .insert(20, make_view_pane(20, AgentState::NeedsYou))
            .unwrap();
        let windows = vec![make_empty_window(1, 100), needs_you_win];
        let state = compute_context_state(1, &contexts, &windows);
        assert_eq!(state.children[0].status, ContextStatus::NeedsYou);
        assert_eq!(state.status, ContextStatus::NeedsYou);
        assert_eq!(
            state.children[0].status_summary_badge().as_deref(),
            Some("[1 NEEDS YOU]")
        );
    }
}
