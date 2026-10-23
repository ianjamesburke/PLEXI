//! Host snapshot and actions for the shared Agents view.
use super::PlexiApp;
use crate::agent::fleet::{AgentInput, AgentsView, BlockedOn, FleetFilter};
use crate::protocol::{AgentBlockedReason, AgentState};

impl PlexiApp {
    pub(crate) fn fleet_focused_pane(&self) -> Option<u64> {
        if let Some(pane) = self.flap_keyboard_pane() {
            return Some(pane);
        }
        if let Some(parent) = self.conjoined_parent() {
            if self.conjoined_parent_focus == Some(parent) { return Some(parent); }
        }
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

    fn agent_inputs(&self) -> Vec<AgentInput> {
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
                            | crate::broker::gate::NeedsYouKind::PermissionRequest
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
                    state: state.clone(),
                    since: t.fleet_since,
                    ephemeral: t.ephemeral,
                    seen: t.fleet_seen,
                    done_unseen: t.fleet_done_unseen,
                    blocked_on,
                    tail: if state == AgentState::Failed { report.and_then(|a| a.detail.clone()).map(|reason| format!("failed: {reason}")) } else { t.capture_tail_preview() },
                    result: self
                        .pane_results
                        .get(&pane_id)
                        .and_then(|r| r.committed.clone())
                        .or_else(|| t.fleet_result.clone()),
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
        input
    }

    pub(crate) fn agents_view(&self, filter: FleetFilter) -> AgentsView {
        AgentsView::build(
            self.agent_inputs().into_iter().filter(|agent| agent.live).collect(),
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
        self.service_agent_attention(ctx);
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
        let mut session_id = None;
        if let Some(pane) = self.windows[index].panes.get_mut(&pane_id) {
            if let Some(mut state) = pane.agent().cloned() {
                session_id = state.session_id.clone();
                state.state = AgentState::Working;
                state.detail = None;
                pane.set_agent(Some(state));
            }
            if let Some(t) = pane.as_terminal_mut() {
                if session_id.is_none() {
                    session_id = t.agent_session_id.clone();
                }
                t.last_blocked_reason = None;
            }
        }
        self.withdraw_agent_attention_items(pane_id, session_id.as_deref());
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
    fn agents_view_excludes_closed_runs_but_retains_history() {
        let mut h = HostHarness::new();
        let live = h.add_focused_terminal();
        h.app.windows[0].panes.get_mut(&live).unwrap().set_agent(Some(
            crate::protocol::PaneAgentState {
                pane_id: live, state: crate::protocol::AgentState::Working,
                agent: "pi".into(), detail: None, session_id: None,
            },
        ));
        for (pane_id, state) in [(900001, "working"), (900002, "blocked"), (900003, "completed")] {
            let record = serde_json::from_value(serde_json::json!({
                "run_id": format!("old-{pane_id}"), "pane_id": pane_id,
                "context_id": 1, "context_name": "previous session", "harness": "codex",
                "cwd": "/tmp", "last_state": state, "closed_at": 1
            })).unwrap();
            h.app.fleet_history.borrow_mut().push(record);
        }
        let view = h.app.agents_view(crate::protocol::FleetFilter::All);
        let ids: Vec<_> = view.tasks.iter().flat_map(|t| &t.rows).map(|r| r.pane_id).collect();
        assert_eq!(ids, vec![live]);
        assert_eq!(h.app.fleet_history.borrow().len(), 3);
    }

    #[test]
    fn attention_reload_projects_config_without_restart_on_hidden_frame() {
        let mut h = HostHarness::new();
        let path = crate::config::config_path();
        std::fs::write(&path, "[attention]\nstyle='loud'\nintensity=0.5\ndock='none'\n[theme]\nstate_preset='mono'\n").unwrap();
        h.app.reload_config();
        h.hidden_frame();
        let visual = crate::ui::activity::attention(&h.app.ctx);
        assert!(visual.loud);
        assert!(visual.alpha <= 0.5);
        assert_eq!(h.app.colors.state_working, h.app.colors.state_waiting);
        std::fs::write(&path, "[attention]\nstyle='off'\n").unwrap();
        h.app.reload_config();
        h.hidden_frame();
        assert_eq!(crate::ui::activity::attention(&h.app.ctx).alpha, 0.0);
        assert!(!crate::ui::activity::attention(&h.app.ctx).animate);
    }

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

#[cfg(test)]
mod attention_tests {
    use crate::{
        protocol::{AgentState, AppRequest},
        testing::HostHarness,
    };

    #[test]
    fn attention_cycles_needs_you_blocked_done_and_wraps() {
        let mut h = HostHarness::new();
        let first = h.add_focused_terminal();
        let mut ids = Vec::new();
        for state in [
            AgentState::NeedsYou,
            AgentState::Blocked,
            AgentState::Completed,
        ] {
            let id = if ids.is_empty() {
                first
            } else {
                h.app.split_focused(true, None, false, false, None);
                h.app.fleet_focused_pane().unwrap()
            };
            h.app.windows[0].panes.get_mut(&id).unwrap().set_agent(Some(
                crate::protocol::PaneAgentState {
                    pane_id: id,
                    state,
                    agent: "pi".into(),
                    detail: None,
                    session_id: None,
                },
            ));
            ids.push(id);
        }
        let other = h.add_test_pane();
        h.focus_pane(other);
        for expected in [ids[0], ids[1], ids[2], ids[0]] {
            assert_eq!(h.app.next_blocked_agent(), Some(expected));
            assert_eq!(h.app.fleet_focused_pane(), Some(expected));
        }
    }

    #[test]
    fn attention_empty_does_not_move_focus_and_shows_feedback() {
        let mut h = HostHarness::new();
        let pane = h.add_focused_terminal();
        assert_eq!(h.app.next_blocked_agent(), None);
        assert_eq!(h.app.fleet_focused_pane(), Some(pane));
        assert!(h.app.agent_attention.toast_until.is_some());
    }

    #[test]
    fn attention_cli_dispatch_uses_same_selector() {
        let mut h = HostHarness::new();
        let id = h.add_focused_terminal();
        h.app.windows[0].panes.get_mut(&id).unwrap().set_agent(Some(
            crate::protocol::PaneAgentState {
                pane_id: id,
                state: AgentState::NeedsYou,
                agent: "pi".into(),
                detail: None,
                session_id: None,
            },
        ));
        let other = h.add_test_pane();
        h.focus_pane(other);
        let dir = tempfile::tempdir().unwrap();
        let response = dir.path().join("next.json");
        h.app.handle_pane_ipc_request(AppRequest::AgentNext {
            response_file: response.to_string_lossy().into_owned(),
        });
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(response).unwrap()).unwrap();
        assert_eq!(value["pane_id"], id);
        assert_eq!(h.app.fleet_focused_pane(), Some(id));
    }

    #[test]
    fn attention_report_feeds_both_surfaces_and_debounces() {
        let mut h = HostHarness::new();
        h.app.notifications_enabled = true;
        let agent = h.add_focused_terminal();
        let other = h.add_test_pane();
        h.focus_pane(other);
        let report = AppRequest::SetAgentState {
            pane_id: agent,
            state: AgentState::NeedsYou,
            agent: "pi".into(),
            detail: Some("Which branch?".into()),
            session_id: None,
            event: None,
            blocked_reason: None,
        };
        h.inject_ipc(report.clone());
        h.inject_ipc(report);
        h.hidden_frame();
        let notifications: Vec<_> = h
            .app
            .pending_notifications
            .iter()
            .filter(|n| n.agent_source_pane() == agent)
            .collect();
        assert_eq!(notifications.len(), 1);
        assert!(h.app.notification_is_visible(notifications[0]));
        assert_eq!(
            notifications[0].source_context_id,
            h.app.windows[0].context_id
        );
        let view = h.app.agents_view(crate::protocol::FleetFilter::All);
        let row = view
            .tasks
            .iter()
            .flat_map(|t| &t.rows)
            .find(|r| r.pane_id == agent)
            .unwrap();
        assert_eq!(row.events[0], notifications[0].agent_event());
        assert!(row.events[0].body.contains("Which branch?"));
    }

    #[test]
    fn non_attention_state_report_withdraws_agent_attention_and_preserves_broker_permission_request() {
        let mut h = HostHarness::new();
        h.app.notifications_enabled = true;
        let agent = h.add_focused_terminal();
        let other = h.add_test_pane();
        h.focus_pane(other);

        // 1. Agent reports needs_you -> files NeedsYouKind::Question, appears in open_items and agent next.
        h.inject_ipc(AppRequest::SetAgentState {
            pane_id: agent,
            state: AgentState::NeedsYou,
            agent: "test".into(),
            detail: Some("Which branch?".into()),
            session_id: None,
            event: None,
            blocked_reason: None,
        });
        h.hidden_frame();

        let items = crate::overlays::attention::open_items();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind, crate::broker::gate::NeedsYouKind::Question);
        let question_id = items[0].id.clone();

        // 2. Agent reports idle -> withdraws its attention item without recording an approval,
        // clears the attention notification, and removes the agent from `agent next`.
        h.inject_ipc(AppRequest::SetAgentState {
            pane_id: agent,
            state: AgentState::Idle,
            agent: "test".into(),
            detail: None,
            session_id: None,
            event: None,
            blocked_reason: None,
        });
        h.hidden_frame();

        assert!(
            crate::overlays::attention::open_items().is_empty(),
            "attention card must be cleared when agent transitions to idle"
        );
        let monitor =
            crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir());
        let settled = monitor.get_needs_you(&question_id).expect("settled item");
        assert_eq!(
            settled.resolution,
            Some(crate::broker::gate::NeedsYouResolution::Denied),
            "withdrawn item must not be recorded as an approval"
        );
        assert!(
            h.app
                .pending_notifications
                .iter()
                .all(|n| n.agent_source_pane() != agent),
            "agent-attention notification must be removed"
        );

        let dir = tempfile::tempdir().unwrap();
        let response = dir.path().join("next.json");
        h.app.handle_pane_ipc_request(AppRequest::AgentNext {
            response_file: response.to_string_lossy().into_owned(),
        });
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&response).unwrap()).unwrap();
        assert!(
            value["pane_id"].is_null(),
            "plexi agent next --json must return null after agent returns to idle"
        );
        assert_eq!(
            h.app.fleet_focused_pane(),
            Some(other),
            "focus must not move when no agents need attention"
        );

        // 3. Also verify working and completed withdraw blocked/needs_you items,
        // while a broker-filed PermissionRequest is NOT withdrawn by an agent state report.
        h.inject_ipc(AppRequest::SetAgentState {
            pane_id: agent,
            state: AgentState::Blocked,
            agent: "test".into(),
            detail: Some("Waiting on input".into()),
            session_id: Some("sess-1".into()),
            event: None,
            blocked_reason: None,
        });
        let perm_req_id = monitor
            .file_needs_you_with_details(
                crate::broker::gate::NeedsYouFile {
                    kind: crate::broker::gate::NeedsYouKind::PermissionRequest,
                    actor: format!("pane:{agent}"),
                    resource: "git.push".into(),
                    summary: "tool 'git.push' was denied".into(),
                    expires_at: None,
                    run_tag: Some("sess-1".into()),
                },
                Some(crate::broker::gate::NeedsYouDetails {
                    pane_id: Some(agent),
                    tool: Some("git.push".into()),
                    ..Default::default()
                }),
            )
            .unwrap()
            .0;
        h.hidden_frame();
        assert_eq!(crate::overlays::attention::open_items().len(), 2);

        h.inject_ipc(AppRequest::SetAgentState {
            pane_id: agent,
            state: AgentState::Working,
            agent: "test".into(),
            detail: None,
            session_id: Some("sess-1".into()),
            event: None,
            blocked_reason: None,
        });
        h.hidden_frame();

        let remaining = crate::overlays::attention::open_items();
        assert_eq!(
            remaining.len(),
            1,
            "broker PermissionRequest must survive agent state report"
        );
        assert_eq!(remaining[0].id, perm_req_id);
        assert_eq!(
            remaining[0].kind,
            crate::broker::gate::NeedsYouKind::PermissionRequest
        );

        // 4. Verify completed also withdraws an open agent needs_you item.
        monitor
            .resolve_needs_you_from(
                &perm_req_id,
                false,
                crate::broker::gate::ResolveSurface::Desktop,
            )
            .unwrap();
        h.inject_ipc(AppRequest::SetAgentState {
            pane_id: agent,
            state: AgentState::NeedsYou,
            agent: "test".into(),
            detail: Some("Final check?".into()),
            session_id: Some("sess-1".into()),
            event: None,
            blocked_reason: None,
        });
        h.hidden_frame();
        assert_eq!(crate::overlays::attention::open_items().len(), 1);

        h.inject_ipc(AppRequest::SetAgentState {
            pane_id: agent,
            state: AgentState::Completed,
            agent: "test".into(),
            detail: None,
            session_id: Some("sess-1".into()),
            event: None,
            blocked_reason: None,
        });
        h.hidden_frame();
        assert!(
            crate::overlays::attention::open_items().is_empty(),
            "completed state report must withdraw the agent's open needs-you item"
        );
    }
}

#[derive(Default)]
pub(crate) struct AttentionState {
    dock: crate::platform::attention::DockState,
    reports: std::collections::HashMap<u64, AgentState>,
    pending_os: Vec<(u64, AgentState, crate::agent::fleet::AgentEvent)>,
    toast_until: Option<std::time::Instant>,
}

impl PlexiApp {
    pub(crate) fn report_agent_attention(
        &mut self,
        pane_id: u64,
        state: &AgentState,
        detail: Option<&str>,
    ) {
        if self
            .agent_attention
            .reports
            .insert(pane_id, state.clone())
            .as_ref()
            == Some(state)
        {
            return;
        }
        self.agent_attention
            .pending_os
            .retain(|(id, _, _)| *id != pane_id);
        if !matches!(
            state,
            AgentState::NeedsYou | AgentState::Blocked | AgentState::Completed
        ) {
            self.clear_agent_attention_notification(pane_id);
            return;
        }
        let role = self
            .windows
            .iter()
            .find_map(|w| w.panes.get(&pane_id))
            .and_then(|p| p.as_terminal())
            .and_then(|t| t.agent_role.as_deref())
            .unwrap_or("Agent");
        let event = crate::agent::fleet::AgentEvent {
            title: format!("{role} · pane #{pane_id}"),
            body: detail
                .filter(|s| !s.is_empty())
                .unwrap_or(match state {
                    AgentState::Completed => "Completed",
                    _ => "blocked (no reason reported)",
                })
                .to_owned(),
        };
        // Both inboxes read this same event through the notification queue.
        let mut notification =
            super::notifications::PendingNotification::from_agent_event(pane_id, &event);
        if let Some(win) = self.windows.iter().find(|w| w.panes.contains_key(&pane_id)) {
            notification.source_context_id = win.context_id;
            notification.source_window_id = win.window_id;
        }
        self.pending_notifications
            .retain(|n| n.notify_id != notification.notify_id);
        self.enqueue_notification(
            super::notifications::NotifySource::HostInternal,
            notification,
        );
        if matches!(state, AgentState::NeedsYou | AgentState::Blocked) {
            self.agent_attention
                .pending_os
                .push((pane_id, state.clone(), event));
        }
    }

    pub(crate) fn clear_agent_attention_notification(&mut self, pane_id: u64) {
        let notify_id = format!("agent-attention-{pane_id}");
        let before = self.pending_notifications.len();
        self.pending_notifications
            .retain(|n| n.notify_id != notify_id);
        if self.pending_notifications.len() != before {
            if self.current_notify_id.as_deref() == Some(notify_id.as_str()) {
                self.current_notify_id = None;
            }
            if self.pending_notifications.is_empty() {
                self.show_notification_modal = false;
            }
            self.save_notifications();
        }
    }

    pub(crate) fn withdraw_agent_attention_items(
        &mut self,
        pane_id: u64,
        session_id: Option<&str>,
    ) {
        let withdrawn = crate::broker::gate::PermissionMonitor::for_profile(
            &crate::config::config_dir(),
        )
        .withdraw_agent_needs_you(pane_id, session_id);
        if !withdrawn.is_empty() {
            log::info!(
                "attention: withdrew {} agent needs-you item(s) for pane_id={pane_id}: {:?}",
                withdrawn.len(),
                withdrawn
            );
        }
        self.clear_agent_attention_notification(pane_id);
    }

    pub(crate) fn service_agent_attention(&mut self, ctx: &egui::Context) {
        for agent in self.agent_inputs().into_iter().filter(|a| a.live) {
            let detail = agent.blocked_on.as_ref().map(|reason| reason.label());
            self.report_agent_attention(agent.pane_id, &agent.state, detail.as_deref());
        }

        let enabled = !cfg!(test)
            && self
                .config
                .agents
                .as_ref()
                .and_then(|a| a.os_notifications)
                .unwrap_or(true);
        let focused = crate::platform::attention::app_focused(ctx);
        let attention = self.config.attention.unwrap_or_default();
        crate::ui::activity::set_attention(ctx, attention, crate::platform::attention::reduce_motion());
        self.agent_attention.dock.sync(
            crate::overlays::attention::open_items().into_iter().map(|item| item.id).collect(),
            attention.dock.unwrap_or_default(), focused,
        );
        let live: std::collections::HashSet<_> = self
            .windows
            .iter()
            .flat_map(|w| w.panes.keys().copied())
            .collect();
        self.agent_attention
            .reports
            .retain(|id, _| live.contains(id));
        for (pane_id, state, event) in self.agent_attention.pending_os.drain(..) {
            if should_raise_attention(enabled, focused, live.contains(&pane_id)) {
                crate::platform::attention::raise(pane_id, &state, &event, ctx);
            }
        }
        for pane_id in crate::platform::attention::take_clicked() {
            if self.pane_navigate(pane_id) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                crate::platform::attention::activate();
                log::info!("attention: notification click pane={pane_id}");
            } else {
                log::warn!("attention: notification target pane={pane_id} no longer exists");
            }
        }
    }

    pub(crate) fn next_blocked_agent(&mut self) -> Option<u64> {
        // Use the same effective states as the Agents pane, including broker requests.
        let mut candidates = Vec::new();
        for agent in self.agent_inputs().into_iter().filter(|a| a.live) {
            let rank = match agent.state {
                AgentState::NeedsYou => 0,
                AgentState::Blocked => 1,
                _ if (agent.done_unseen || agent.state == AgentState::Completed) && !agent.seen => {
                    2
                }
                _ => continue,
            };
            candidates.push((rank, std::cmp::Reverse(agent.since), agent.pane_id));
        }
        candidates.sort_unstable();
        let ids: Vec<_> = candidates.into_iter().map(|(_, _, id)| id).collect();
        let target = next_attention_target(&ids, self.fleet_focused_pane());
        if let Some(id) = target {
            self.pane_navigate(id);
            self.mark_agent_seen(id);
            self.agent_attention.toast_until = None;
            log::info!("attention: next agent pane={id}");
        } else {
            self.agent_attention.toast_until =
                Some(std::time::Instant::now() + std::time::Duration::from_secs(3));
            log::info!("attention: no agents need attention");
        }
        target
    }

    pub(crate) fn draw_agent_attention_toast(&mut self, ctx: &egui::Context) {
        let Some(until) = self.agent_attention.toast_until else {
            return;
        };
        if std::time::Instant::now() >= until {
            self.agent_attention.toast_until = None;
            return;
        }
        crate::ui::toast::ToastShell::bottom("agent_attention_empty").show(
            ctx,
            &self.colors,
            |ui| {
                crate::ui::toast::toast_caption(ui, "No agents need attention.", &self.colors);
            },
        );
        ctx.request_repaint_after(until.saturating_duration_since(std::time::Instant::now()));
    }
}

fn next_attention_target(ids: &[u64], current: Option<u64>) -> Option<u64> {
    if ids.is_empty() {
        return None;
    }
    let next = current
        .and_then(|id| ids.iter().position(|candidate| *candidate == id))
        .map_or(0, |index| (index + 1) % ids.len());
    Some(ids[next])
}

fn should_raise_attention(enabled: bool, focused: bool, live: bool) -> bool {
    enabled && !focused && live
}

#[cfg(test)]
mod delivery_tests {
    #[test]
    fn attention_delivery_requires_enabled_unfocused_live_pane() {
        for enabled in [false, true] {
            for focused in [false, true] {
                for live in [false, true] {
                    assert_eq!(
                        super::should_raise_attention(enabled, focused, live),
                        (enabled, focused, live) == (true, false, true)
                    );
                }
            }
        }
    }
}
