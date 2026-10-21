//! Host snapshot and actions for the shared Agents view.
use super::PlexiApp;
use crate::agent::fleet::{AgentInput, AgentsView, BlockedOn, FleetFilter};
use crate::protocol::{AgentBlockedReason, AgentState};

impl PlexiApp {
    pub(crate) fn fleet_focused_pane(&self) -> Option<u64> {
        let win = self.windows.get(self.active_window)?;
        Self::find_pane_in_tile(&win.tree, win.focused_pane?)
    }

    pub(crate) fn mark_agent_seen(&mut self, pane_id: u64) {
        let mut changed = false;
        for win in &mut self.windows {
            if let Some(t) = win
                .panes
                .get_mut(&pane_id)
                .and_then(|p| p.as_terminal_mut())
            {
                if !t.fleet_seen && (t.fleet_done_unseen || t.agent_completed) {
                    t.fleet_seen = true;
                    changed = true;
                }
            }
        }
        for record in self
            .fleet_history
            .borrow_mut()
            .iter_mut()
            .filter(|r| r.pane_id == pane_id && !r.fleet_seen)
        {
            record.fleet_seen = true;
            if let Err(e) = crate::workspace::append_agent_history(record) {
                log::error!(
                    "agents: persist archived seen run_id={}: {e}",
                    record.run_id
                );
                record.fleet_seen = false;
            }
        }
        if changed {
            log::info!("agents: seen pane_id={pane_id}");
            self.mark_workspace_dirty();
        }
    }

    pub(crate) fn agents_view(&self, filter: FleetFilter) -> AgentsView {
        let mut input = Vec::new();
        let needs_you =
            crate::broker::gate::PermissionMonitor::loaded(&crate::config::config_dir())
                .map(|m| m.open_needs_you())
                .unwrap_or_default();
        for win in &self.windows {
            let context_name = self
                .router
                .iter()
                .find(|c| c.context_id == win.context_id)
                .map(|c| c.name.displayed().to_string())
                .unwrap_or_default();
            for (&pane_id, pane) in &win.panes {
                let Some(t) = pane.as_terminal() else {
                    continue;
                };
                if !t.is_agent_pane() {
                    continue;
                }
                let report = pane.agent();
                let state = if t.agent_completed {
                    AgentState::Completed
                } else {
                    report.map(|a| a.state.clone()).unwrap_or(AgentState::Idle)
                };
                let question = report
                    .and_then(|a| a.detail.clone())
                    .filter(|s| !s.is_empty());
                let human_record = needs_you.iter().find(|n| {
                    n.resource == format!("pane:{pane_id}")
                        || n.actor == format!("pane:{pane_id}")
                        || n.run_tag
                            .as_ref()
                            .zip(t.agent_session_id.as_ref())
                            .is_some_and(|(a, b)| a == b)
                });
                let state = if human_record.is_some() {
                    AgentState::NeedsYou
                } else {
                    state
                };
                let blocked_on = if let Some(n) = human_record {
                    if matches!(
                        n.kind,
                        crate::broker::gate::NeedsYouKind::ApprovalClick
                            | crate::broker::gate::NeedsYouKind::PermissionChange
                    ) {
                        Some(BlockedOn::Permission {
                            summary: n.summary.clone(),
                        })
                    } else {
                        Some(BlockedOn::Human {
                            question: n.summary.clone(),
                        })
                    }
                } else if t.last_blocked_reason == Some(AgentBlockedReason::PermissionPrompt) {
                    Some(BlockedOn::Permission {
                        summary: question
                            .clone()
                            .unwrap_or_else(|| "Permission requested".into()),
                    })
                } else if matches!(state, AgentState::Blocked | AgentState::NeedsYou) {
                    question
                        .map(|question| match t.parent_pane_id {
                            Some(parent) if state != AgentState::NeedsYou => BlockedOn::Agent {
                                pane_id: parent,
                                role: self
                                    .windows
                                    .iter()
                                    .find_map(|w| w.panes.get(&parent))
                                    .and_then(|p| p.as_terminal())
                                    .and_then(|t| t.agent_role.clone())
                                    .unwrap_or_else(|| "parent".into()),
                                question,
                            },
                            _ => BlockedOn::Human { question },
                        })
                        .or_else(|| {
                            self.pane_mailboxes
                                .get(&pane_id)
                                .and_then(|q| q.front())
                                .and_then(|m| m.from_pane_id)
                                .map(|from_pane| BlockedOn::Message { from_pane })
                        })
                } else {
                    None
                };
                let events = self
                    .pending_notifications
                    .iter()
                    .filter(|n| n.agent_source_pane() == pane_id)
                    .map(|n| n.agent_event())
                    .collect();
                input.push(AgentInput {
                    live: true,
                    pane_id,
                    parent_pane_id: t.parent_pane_id,
                    role: t.agent_role.clone().unwrap_or_else(|| "unknown".into()),
                    harness: t
                        .agent_harness
                        .clone()
                        .or_else(|| report.map(|a| a.agent.clone()))
                        .unwrap_or_else(|| "unknown".into()),
                    context_name: context_name.clone(),
                    state,
                    since: t.fleet_since,
                    ephemeral: t.ephemeral && !t.slots.contains_key("question"),
                    seen: t.fleet_seen,
                    done_unseen: t.fleet_done_unseen,
                    blocked_on,
                    tail: t.capture_tail_preview(),
                    result: self
                        .pane_results
                        .get(&pane_id)
                        .and_then(|r| r.committed.clone()),
                    events,
                });
            }
        }
        let live_ids: std::collections::HashSet<_> = input.iter().map(|r| r.pane_id).collect();
        for record in self.fleet_history.borrow().iter().rev().take(50) {
            if live_ids.contains(&record.pane_id) {
                continue;
            }
            input.push(AgentInput {
                live: false,
                pane_id: record.pane_id,
                parent_pane_id: record.parent_pane_id,
                role: record
                    .config_name
                    .clone()
                    .unwrap_or_else(|| "unknown".into()),
                harness: record.harness.clone(),
                context_name: record.context_name.clone(),
                state: record.last_state.clone(),
                since: record.closed_at,
                ephemeral: false,
                seen: record.fleet_seen,
                done_unseen: record.last_state == AgentState::Completed,
                blocked_on: None,
                tail: record.tail_preview.clone(),
                result: record.result.clone(),
                events: vec![],
            });
        }
        AgentsView::build(
            input,
            crate::platform::clock::now_secs(),
            self.config
                .agents
                .as_ref()
                .and_then(|a| a.stalled_after_secs)
                .unwrap_or(120),
            self.fleet_focused_pane(),
            filter,
        )
    }

    /// Runs from logic, including hidden windows and inactive contexts.
    pub(crate) fn sync_agents_panes(&mut self, ctx: &egui::Context) {
        if let Some(id) = self.fleet_focused_pane() {
            self.mark_agent_seen(id);
        }
        let has_agents_pane = self.windows.iter().flat_map(|w| w.panes.values()).any(
            |p| matches!(p, crate::host::pane::Pane::App(a) if a.runtime.type_id() == "agents"),
        );
        if !has_agents_pane {
            return;
        }
        let view = self.agents_view(FleetFilter::All);
        for win in &mut self.windows {
            for pane in win.panes.values_mut() {
                if let crate::host::pane::Pane::App(a) = pane {
                    if let crate::host::pane::AppRuntime::Builtin(app) = &mut a.runtime {
                        app.sync_agents_view(&view);
                    }
                }
            }
        }
        ctx.request_repaint_after(std::time::Duration::from_secs(1));
    }

    pub(crate) fn reply_to_fleet_agent(&mut self, pane_id: u64, from_pane_id: u64, text: String) {
        let Some((index, _)) = self.find_pane_in_any_window(pane_id) else {
            log::warn!("agents: reply target {pane_id} no longer exists");
            return;
        };
        self.enqueue_pane_message(
            pane_id,
            Some(from_pane_id),
            serde_json::json!({"answer":text}).to_string(),
        );
        if let Some(pane) = self.windows[index].panes.get_mut(&pane_id) {
            if let Some(mut state) = pane.agent().cloned() {
                state.state = AgentState::Working;
                state.detail = None;
                pane.set_agent(Some(state));
            }
            if let Some(t) = pane.as_terminal_mut() {
                t.last_blocked_reason = None;
            }
        }
        self.mark_workspace_dirty();
        log::info!("agents: reply delivered pane_id={pane_id} from_pane_id={from_pane_id}");
    }

    pub(crate) fn open_agents_pane(&mut self) {
        log::info!("agents: open pane");
        if let Err(e) =
            self.launch_app_by_id_with_layout("agents", Some("split_h".into()), &[], None)
        {
            log::error!("agents: open failed: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::testing::HostHarness;

    #[test]
    fn fleet_notifications_share_queue_and_suppress_focused_agents() {
        let mut h = HostHarness::new();
        h.app.notifications_enabled = true;
        let agent = h.add_focused_terminal();
        h.app.windows[0]
            .panes
            .get_mut(&agent)
            .unwrap()
            .set_agent(Some(crate::protocol::PaneAgentState {
                pane_id: agent,
                state: crate::protocol::AgentState::Blocked,
                agent: "pi".into(),
                detail: Some("Need a branch".into()),
                session_id: None,
            }));
        let notification = crate::app::notifications::PendingNotification {
            sender_pane_id: agent,
            title: "Blocked".into(),
            body: "Need a branch".into(),
            ..Default::default()
        };
        assert!(!h.app.enqueue_notification(
            crate::app::notifications::NotifySource::HostInternal,
            notification.clone()
        ));
        let other = h.add_test_pane();
        h.focus_pane(other);
        assert_ne!(agent, other);
        assert!(h.app.enqueue_notification(
            crate::app::notifications::NotifySource::HostInternal,
            notification
        ));
        assert_eq!(
            h.app.pending_notifications.len(),
            1,
            "agent event stays in the original notification panel"
        );
        let view = h.app.agents_view(crate::protocol::FleetFilter::All);
        let row = view
            .tasks
            .iter()
            .flat_map(|t| &t.rows)
            .find(|r| r.pane_id == agent)
            .unwrap();
        assert_eq!(row.events[0].body, "Need a branch");
    }
}
