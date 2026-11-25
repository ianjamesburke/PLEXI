//! The palette's Agents source. Logic refreshes the snapshot independently of
//! query edits; querying it is entirely in memory.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;

use super::attention_queue::QueueCategory;
use super::PlexiApp;
use crate::agent::fleet::{self, AgentInput, BlockedOn, DisplayState};
use crate::broker::gate::{NeedsYouRecord, PermissionMonitor};
use crate::host::picker::{
    AttentionRank, PickerRow, PreviewBody, RowAction, RowKey, AGENTS_SOURCE,
};
use crate::protocol::AgentState;
use crate::spatial::tiling::PaneId;

const FOCUS: RowAction = RowAction {
    id: "focus",
    label: "Focus",
    chord: None,
};
const ANSWER: RowAction = RowAction {
    id: "answer",
    label: "Answer",
    chord: None,
};
const CLOSE: RowAction = RowAction {
    id: "close",
    label: "Close pane",
    chord: None,
};

#[derive(Default)]
pub(crate) struct PaletteAgents {
    origin: Option<AgentOrigin>,
    snapshot: Option<AgentSnapshot>,
    loading: Option<Receiver<Result<LoadedAgentSource, String>>>,
    #[cfg(test)]
    load_gate: Option<Receiver<Result<(), String>>>,
    #[cfg(test)]
    loads: usize,
}

#[derive(Clone)]
struct AgentOrigin {
    cwd: PathBuf,
    caller: Option<PaneId>,
}

struct LoadedAgentSource {
    monitor: Arc<PermissionMonitor>,
    definitions: Vec<AgentEntry>,
    #[cfg(test)]
    read_on: std::thread::ThreadId,
}

#[derive(Default)]
struct AgentSnapshot {
    source: Option<LoadedAgentSource>,
    needs_you: Vec<NeedsYouRecord>,
    // Keep the shared roster inputs as the change detector. Terminal tails
    // are captured only when rebuilding the snapshot.
    inputs: Vec<(AgentInput, DisplayState)>,
    live: Vec<AgentEntry>,
}

struct AgentEntry {
    row: PickerRow,
    search_text: String,
    target: AgentTarget,
}

impl LoadedAgentSource {
    fn load(profile: &Path, cwd: &Path) -> Result<Self, String> {
        // Workspace discovery can resolve legacy, channel-scoped agent dirs.
        #[cfg(test)]
        let _profile_guard = crate::config::set_test_profile_dir(profile.to_path_buf());
        let workspace = super::registry::resolve_agent_lookup_workspace_root(cwd);
        let registry = crate::agent::AgentRegistry::load_with_optional_workspace(
            profile,
            workspace.as_deref(),
        );
        let monitor = PermissionMonitor::for_profile(profile)?;
        let definitions = registry
            .agents()
            .map(|definition| {
                let mut entry = AgentEntry::new(
                    AgentTarget::Definition(definition.id.clone()),
                    fleet::resolve_agent_label(
                        Some(&definition.display_name),
                        None,
                        Some(&definition.id),
                    ),
                    "not running",
                );
                let harness = definition.harness.as_ref().map_or("", |h| h.cli.as_str());
                entry.search_text = format!("{} {harness}", definition.id);
                entry.row.actions.push(RowAction {
                    chord: Some("Enter"),
                    ..FOCUS
                });
                entry
            })
            .collect();
        Ok(Self {
            monitor,
            definitions,
            #[cfg(test)]
            read_on: std::thread::current().id(),
        })
    }
}

#[derive(Clone)]
pub(crate) enum AgentTarget {
    Pane { id: PaneId, ask: Option<String> },
    Definition(String),
}

pub(crate) struct AgentActivation {
    origin: AgentOrigin,
    target: AgentTarget,
    action: &'static str,
}

impl AgentEntry {
    fn new(target: AgentTarget, title: String, state: &'static str) -> Self {
        let id = match &target {
            AgentTarget::Pane { id, .. } => format!("pane/{id}"),
            AgentTarget::Definition(id) => format!("definition/{id}"),
        };
        Self {
            row: PickerRow {
                key: RowKey::new(AGENTS_SOURCE, &id),
                title,
                secondary: String::new(),
                chips: vec![state],
                group: "",
                group_rank: 0,
                score: None,
                file_match: None,
                pinned: false,
                unavailable: None,
                actions: Vec::new(),
                attention: None,
                preview: None,
                inline_preview: Some(PreviewBody::Note("No preview".to_owned())),
                usage: None,
                modified: None,
            },
            search_text: String::new(),
            target,
        }
    }

    fn live(
        agent: &AgentInput,
        display: DisplayState,
        tail: Option<String>,
        task_title: &str,
    ) -> Self {
        let waiting_on_you =
            display == DisplayState::Blocked && agent.state == AgentState::NeedsYou;
        let state = match display {
            DisplayState::Blocked if waiting_on_you => "waiting on you",
            DisplayState::DoneUnseen => "done",
            state if state.is_failure_like() => "failed",
            state => state.label(),
        };
        let mut entry = Self::new(
            AgentTarget::Pane {
                id: agent.pane_id,
                ask: agent.ask_id.clone(),
            },
            agent.role.clone(),
            state,
        );
        entry.search_text = format!(
            "{} {} {task_title}",
            agent.pane_name.as_deref().unwrap_or_default(),
            agent.harness,
        );
        entry.row.secondary.clone_from(&agent.context_name);
        entry.row.attention = Some(AttentionRank {
            waiting_on_you,
            last_interaction_at: agent.last_interaction_at,
        });
        let preview = if waiting_on_you {
            Some(
                agent
                    .blocked_on
                    .as_ref()
                    .unwrap_or(&BlockedOn::Unknown)
                    .label(),
            )
        } else if let Some(event) = agent.events.last() {
            Some(if event.body.is_empty() {
                event.title.clone()
            } else {
                format!("{}\n{}", event.title, event.body)
            })
        } else {
            tail
        };
        if let Some(text) = preview {
            entry.row.inline_preview = Some(PreviewBody::Text {
                text,
                truncated: false,
            });
        }
        if waiting_on_you && agent.ask_id.is_some() {
            entry.row.actions.push(ANSWER);
        }
        entry.row.actions.extend([FOCUS, CLOSE]);
        entry.row.actions[0].chord = Some("Enter");
        entry
    }

    fn matches(&self, query: &str) -> Option<PickerRow> {
        let mut row = self.row.clone();
        if !query.trim().is_empty() {
            let candidate =
                crate::search::CandidateInput::new(&row.title).with_secondary(&self.search_text);
            let score = crate::search::score_candidate(query, &candidate, std::time::UNIX_EPOCH)?;
            row.score = Some(score.total);
        }
        Some(row)
    }
}

impl PlexiApp {
    pub(crate) fn begin_palette_agents_session(&mut self) {
        let window = &self.windows[self.active_window];
        let cwd = window
            .focused_pane
            .and_then(|tile| window.get_focused_pane_cwd(tile))
            .unwrap_or_else(|| self.router.active().root.clone());
        self.palette_agents = PaletteAgents {
            origin: Some(AgentOrigin {
                cwd,
                caller: self.active_focused_pane_id(),
            }),
            ..Default::default()
        };
    }

    fn start_palette_agents_load(&mut self, cwd: PathBuf) {
        let profile = crate::config::config_dir();
        let wake = Arc::clone(&self.ui_wake);
        let (done, loading) = mpsc::channel();
        #[cfg(test)]
        let gate = self.palette_agents.load_gate.take();
        let spawned = std::thread::Builder::new()
            .name("plexi-palette-agents".to_owned())
            .spawn(move || {
                let result = std::panic::catch_unwind(|| {
                    #[cfg(test)]
                    if let Some(gate) = gate {
                        gate.recv().map_err(|error| error.to_string())??;
                    }
                    LoadedAgentSource::load(&profile, &cwd)
                })
                .unwrap_or_else(|_| Err("The Agents loader stopped unexpectedly".into()));
                // A closed palette drops the receiver for this session.
                if done.send(result).is_ok() {
                    wake.wake("palette_agents");
                }
            });
        match spawned {
            Ok(_) => {
                log::info!("palette agents: publishing partial answers while loading definitions and needs-you");
                self.palette_agents.loading = Some(loading);
            }
            Err(error) => self.report_palette_agents_load_failure(&error.to_string()),
        }
    }

    pub(crate) fn palette_agents_load_pending(&self) -> bool {
        self.palette_agents.loading.is_some()
    }

    pub(crate) fn drain_palette_agents_load(&mut self) {
        let Some(loading) = &self.palette_agents.loading else {
            return;
        };
        let result = match loading.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("The Agents loader returned no snapshot".into()),
        };
        self.palette_agents.loading = None;
        match result {
            Ok(source) => {
                log::info!(
                    "palette agents: loaded snapshot ({} definitions)",
                    source.definitions.len()
                );
                if let Some(snapshot) = &mut self.palette_agents.snapshot {
                    snapshot.source = Some(source);
                    #[cfg(test)]
                    {
                        self.palette_agents.loads += 1;
                    }
                }
            }
            Err(error) => self.report_palette_agents_load_failure(&error),
        }
    }

    fn report_palette_agents_load_failure(&mut self, error: &str) {
        use super::notifications::{new_notify_id, NotifySource, PendingNotification};

        log::error!("palette agents: could not load definitions and needs-you: {error}");
        self.enqueue_notification(
            NotifySource::HostInternal,
            PendingNotification {
                notify_id: new_notify_id("palette-agents-load"),
                title: "Could not load agent definitions or requests".into(),
                body: error.into(),
                scope: crate::protocol::NotifyScope::Global,
                ..Default::default()
            },
        );
    }

    pub(crate) fn refresh_palette_agents(&mut self) {
        let Some(origin) = self.palette_agents.origin.as_ref() else {
            return;
        };
        if self.palette_agents.snapshot.is_none() {
            self.start_palette_agents_load(origin.cwd.clone());
        }
        let mut snapshot = self.palette_agents.snapshot.take().unwrap_or_default();
        let needs_you = snapshot
            .source
            .as_ref()
            .map_or_else(Vec::new, |source| source.monitor.host_records().open());
        let now = crate::platform::clock::now_secs();
        let stalled_after = self
            .config
            .agents
            .as_ref()
            .and_then(|a| a.stalled_after_secs)
            .unwrap_or(120);
        let focused = self.fleet_focused_pane();
        let mut inputs: Vec<_> = self
            .agent_inputs_with_needs_you(&needs_you, false)
            .into_iter()
            .map(|mut agent| {
                if agent.state == AgentState::NeedsYou && agent.ask_id.is_none() {
                    agent.ask_id = self
                        .attention_queue
                        .items
                        .iter()
                        .find(|bar| {
                            bar.pane_id == agent.pane_id && bar.category == QueueCategory::Ask
                        })
                        .map(|bar| bar.id.clone());
                }
                let display = agent.display_state(
                    now,
                    stalled_after,
                    agent.seen || focused == Some(agent.pane_id),
                );
                (agent, display)
            })
            .collect();
        inputs.sort_by_key(|(agent, _)| agent.pane_id);
        if snapshot.needs_you != needs_you || snapshot.inputs != inputs {
            let by_id: HashMap<_, _> = inputs
                .iter()
                .map(|(agent, _)| (agent.pane_id, agent))
                .collect();
            snapshot.live = inputs
                .iter()
                .map(|(agent, display)| {
                    let mut root = agent;
                    for _ in 0..inputs.len() {
                        let Some(parent) = root.parent_pane_id.and_then(|id| by_id.get(&id)) else {
                            break;
                        };
                        root = parent;
                    }
                    let task = fleet::resolve_task_title(
                        Some(&root.role),
                        agent.parent_title.as_deref(),
                        &agent.role,
                        &agent.context_name,
                    );
                    let tail = if agent.state != AgentState::NeedsYou && agent.events.is_empty() {
                        self.windows
                            .iter()
                            .find_map(|window| window.panes.get(&agent.pane_id))
                            .and_then(|pane| pane.as_terminal())
                            .and_then(|terminal| terminal.capture_tail_preview())
                    } else {
                        None
                    };
                    AgentEntry::live(agent, *display, tail, &task)
                })
                .collect();
            snapshot.inputs = inputs;
            snapshot.needs_you = needs_you;
            log::info!(
                "palette agents: refreshed snapshot ({} live)",
                snapshot.live.len()
            );
        }
        self.palette_agents.snapshot = Some(snapshot);
    }

    pub(crate) fn palette_agent_rows(
        &self,
        query: &str,
    ) -> (Vec<PickerRow>, HashMap<RowKey, AgentTarget>) {
        let mut rows = Vec::new();
        let mut targets = HashMap::new();
        if let Some(snapshot) = &self.palette_agents.snapshot {
            let definitions = snapshot
                .source
                .iter()
                .flat_map(|source| &source.definitions);
            for entry in snapshot.live.iter().chain(definitions) {
                if let AgentTarget::Definition(id) = &entry.target {
                    if snapshot
                        .inputs
                        .iter()
                        .any(|(agent, _)| agent.definition_id.as_ref() == Some(id))
                    {
                        continue;
                    }
                }
                if let Some(row) = entry.matches(query) {
                    targets.insert(row.key.clone(), entry.target.clone());
                    rows.push(row);
                }
            }
        }
        (rows, targets)
    }

    pub(crate) fn queue_palette_agent_activation(
        &mut self,
        target: AgentTarget,
        action: &'static str,
    ) {
        if let Some(origin) = self.palette_agents.origin.clone() {
            self.palette_agent_activation = Some(AgentActivation {
                origin,
                target,
                action,
            });
            self.ctx.request_repaint();
        }
    }

    pub(crate) fn service_palette_agent_activation(&mut self) {
        let Some(activation) = self.palette_agent_activation.take() else {
            return;
        };
        if let Err(error) = self.run_palette_agent_action(activation) {
            log::warn!("palette agents: action failed: {error}");
            self.notify_command_refused("Agent palette action", &error);
        }
    }

    fn run_palette_agent_action(&mut self, activation: AgentActivation) -> Result<(), String> {
        let AgentActivation {
            origin,
            target,
            action,
        } = activation;
        match (target, action) {
            (AgentTarget::Pane { id, .. }, "focus") => {
                if !self.pane_navigate(id) {
                    return Err(format!("Could not focus pane {id}"));
                }
            }
            (AgentTarget::Definition(name), "focus") => {
                self.open_named_agent(
                    &name,
                    &origin.cwd,
                    origin.caller,
                    &crate::broker::caller::RequestOrigin::HostUi,
                )?;
            }
            (AgentTarget::Pane { ask: Some(id), .. }, "answer") => {
                if !self.open_decision_overlay_for_ask(&id) {
                    return Err("This request is no longer available".into());
                }
            }
            (AgentTarget::Pane { id, .. }, "close") => {
                if self.find_pane_in_any_window(id).is_none() {
                    return Err(format!("Pane {id} is already closed"));
                }
                if self.confirm_close() {
                    self.ask_to_close_pane(self.close_confirm_target_for(id));
                } else {
                    log::info!("palette agents: closing pane={id}");
                    self.close_pane_by_id(id);
                }
            }
            _ => return Err(format!("The selected agent does not support '{action}'")),
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn hold_palette_agents_load(&mut self) -> mpsc::Sender<Result<(), String>> {
        let (release, gate) = mpsc::channel();
        self.palette_agents.load_gate = Some(gate);
        release
    }

    #[cfg(test)]
    pub(crate) fn palette_agents_load_count(&self) -> usize {
        self.palette_agents.loads
    }

    #[cfg(test)]
    pub(crate) fn palette_agents_load_thread(&self) -> Option<std::thread::ThreadId> {
        Some(
            self.palette_agents
                .snapshot
                .as_ref()?
                .source
                .as_ref()?
                .read_on,
        )
    }
}
