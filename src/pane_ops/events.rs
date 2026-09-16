//! Publish pane facts through the same timeline used by app event streams.
use crate::app::PlexiApp;
use crate::host::pane_lifecycle::{PaneLifecycleEvent, PaneLifecycleState, Provenance};

impl PlexiApp {
    /// Register panes from all creation/restoration paths before servicing events.
    /// Called in the logic pass, including while the window is hidden.
    pub(crate) fn observe_pane_spawns(&mut self) {
        match crate::host::app_timeline::global().lock() {
            Ok(mut timeline) => {
                for pane_id in self.host.pane_lifecycle.keys() {
                    if !self
                        .windows
                        .iter()
                        .any(|win| win.panes.contains_key(pane_id))
                    {
                        timeline.close_lifecycle_pane(*pane_id);
                    }
                }
                for win in &self.windows {
                    for pane_id in win.panes.keys() {
                        timeline.locate_lifecycle_pane(*pane_id, win.context_id);
                    }
                }
            }
            Err(error) => log::error!("pane_lifecycle: updating pane ownership failed: {error}"),
        }
        let fresh: Vec<_> = self
            .windows
            .iter()
            .flat_map(|win| win.panes.keys())
            .filter(|id| !self.host.pane_lifecycle.contains_key(id))
            .copied()
            .collect();
        for pane_id in fresh {
            self.ensure_pane_lifecycle(pane_id);
        }
    }

    fn ensure_pane_lifecycle(&mut self, pane_id: u64) -> Option<u64> {
        let context_id = self
            .windows
            .iter()
            .find(|win| win.panes.contains_key(&pane_id))
            .map(|win| win.context_id)?;
        if let Some(state) = self.host.pane_lifecycle.get_mut(&pane_id) {
            state.context_id = context_id;
        } else if self.publish_pane_lifecycle(context_id, pane_id, PaneLifecycleEvent::Spawned) {
            self.host.pane_lifecycle.insert(
                pane_id,
                PaneLifecycleState {
                    context_id,
                    ..Default::default()
                },
            );
        } else {
            return None;
        }
        Some(context_id)
    }

    pub(crate) fn emit_pane_lifecycle(&mut self, pane_id: u64, event: PaneLifecycleEvent) {
        // The saved context is needed for a boot failure after its pane closed.
        let context_id = self.ensure_pane_lifecycle(pane_id).or_else(|| {
            self.host
                .pane_lifecycle
                .get(&pane_id)
                .map(|state| state.context_id)
        });
        if let Some(context_id) = context_id {
            self.publish_pane_lifecycle(context_id, pane_id, event);
        } else {
            log::warn!("pane_lifecycle: cannot attribute event for missing pane {pane_id}");
        }
    }

    fn publish_pane_lifecycle(
        &self,
        context_id: u64,
        pane_id: u64,
        event: PaneLifecycleEvent,
    ) -> bool {
        let timeline = crate::host::app_timeline::global();
        let result = match timeline.lock() {
            Ok(mut timeline) => timeline.record_pane_lifecycle(context_id, pane_id, &event),
            Err(error) => {
                log::error!("pane_lifecycle: timeline lock failed for pane {pane_id}: {error}");
                return false;
            }
        };
        match result {
            Ok(_) => true,
            Err(error) => {
                log::error!("pane_lifecycle: publish failed for pane {pane_id}: {error}");
                false
            }
        }
    }

    pub(crate) fn emit_agent_booted(&mut self, pane_id: u64, provenance: Provenance) {
        if self.ensure_pane_lifecycle(pane_id).is_none() {
            return;
        }
        if let Some(state) = self.host.pane_lifecycle.get_mut(&pane_id) {
            if state.booted {
                return;
            }
            state.booted = true;
        }
        self.emit_pane_lifecycle(pane_id, PaneLifecycleEvent::AgentBooted { provenance });
    }
}

impl PlexiApp {
    pub(crate) fn pane_observation_provenance(&self, pane_id: u64) -> Provenance {
        let agent = self
            .windows
            .iter()
            .find_map(|win| win.panes.get(&pane_id))
            .and_then(|pane| pane.agent());
        Provenance {
            source: crate::host::pane_lifecycle::Source::HostObservation,
            agent_label: agent
                .map(|agent| agent.agent.clone())
                .unwrap_or_else(|| "unknown".into()),
            session_id: agent.and_then(|agent| agent.session_id.clone()),
            raw_event: None,
        }
    }

    pub(crate) fn emit_agent_boot_failure(&mut self, pane_id: u64) {
        self.emit_pane_lifecycle(
            pane_id,
            PaneLifecycleEvent::AgentBlocked {
                reason: crate::protocol::AgentBlockedReason::BootFailure,
                provenance: self.pane_observation_provenance(pane_id),
            },
        );
    }

    pub(crate) fn emit_observed_agent_transition(
        &mut self,
        pane_id: u64,
        agent: &crate::protocol::PaneAgentState,
    ) {
        let provenance = Provenance {
            source: crate::host::pane_lifecycle::Source::HostObservation,
            agent_label: agent.agent.clone(),
            session_id: agent.session_id.clone(),
            raw_event: None,
        };
        if agent.state == crate::protocol::AgentState::Idle {
            self.emit_agent_booted(pane_id, provenance.clone());
        }
        self.emit_pane_lifecycle(
            pane_id,
            PaneLifecycleEvent::from_report(&agent.state, provenance, None),
        );
    }
}
