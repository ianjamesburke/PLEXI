//! Host agent runtime — Phase C of `docs/prm/agent-platform.md`
//! (`docs/prm/chess-agent-poc.md`).
//!
//! Agents are host-level permission actors (`ActorType::Agent`), defined by
//! files in the workspace:
//!
//! ```text
//! <workspace>/<workspace_channel_dir>/agents/<id>/
//!   AGENT.md       — role prose, injected as the system prompt
//!   settings.toml  — identity, requested permissions, requested subscriptions
//! ```
//!
//! `settings.toml` *requests* behavior; only broker grant records (and the
//! agent's own posture for actor-tier allow/ask/deny) decide what the agent
//! may actually do:
//!
//! - **Event subscriptions** always require a persisted user `Allow` grant on
//!   `app_event_stream:<app_id>::<event>`. A denied/ungranted subscription
//!   means the agent never sees the events.
//! - **App tools** are gated per turn on `app_connector:app.<tool_name>`.
//!   The agent's `[permissions]` posture participates at the actor tiers, so
//!   read-only tools in `allow = [...]` work without a user grant, while
//!   `ask = [...]` tools need an explicit user grant record. Non-allowed
//!   tools are removed from the turn's tool snapshot — the model can comment
//!   but cannot call them.
//!
//! Turns run on worker threads (the LLM broker blocks on network); outcomes
//! land in the agent's transcript, which is the host-visible record of what
//! the agent said — the Phase D Assistant UI consumes this seam.

use crate::broker::{
    ActorType, Decision, GrantDuration, GrantStore, PermissionPosture, PermissionRequest,
    TargetType,
};
use crate::host::app_timeline::{AppTimeline, EventDelivery};
use crate::plexi_ai::broker::{AiBroker, AiBrokerRequest};
use crate::plexi_ai::tool_dispatch::ToolDispatcher;
use crate::app_protocol::{AiMessage, ModelTier, PayloadMode, TriggerMode};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

// ── Agent definition ─────────────────────────────────────────────────────────

/// One requested event subscription from `[[subscriptions]]` in
/// `settings.toml`. A request, not a grant — `AgentHost::attach` only
/// subscribes streams the broker allows.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentSubscriptionRequest {
    pub app: String,
    pub events: Vec<String>,
    pub payload: PayloadMode,
    pub trigger: TriggerMode,
    /// What the future grant sheet should preselect (`allow` | `ask`).
    /// Informational — subscriptions always need a persisted user grant.
    pub default: Decision,
}

/// A loaded agent: `AGENT.md` prose + parsed `settings.toml`.
#[derive(Debug, Clone)]
pub struct AgentDefinition {
    pub id: String,
    pub display_name: String,
    pub default_tier: ModelTier,
    /// `AGENT.md` contents — the system prompt.
    pub prompt: String,
    /// `[permissions]` posture — actor-tier allow/ask/deny lists.
    pub posture: PermissionPosture,
    pub subscriptions: Vec<AgentSubscriptionRequest>,
}

#[derive(Debug, serde::Deserialize)]
struct SettingsToml {
    agent: AgentTable,
    #[serde(default)]
    subscriptions: Vec<SubscriptionTable>,
}

#[derive(Debug, serde::Deserialize)]
struct AgentTable {
    id: String,
    display_name: String,
    default_tier: String,
}

#[derive(Debug, serde::Deserialize)]
struct SubscriptionTable {
    app: String,
    events: Vec<String>,
    payload: String,
    trigger: String,
    default: String,
}

fn parse_tier(raw: &str) -> Result<ModelTier, String> {
    match raw {
        "low" => Ok(ModelTier::Low),
        "medium" => Ok(ModelTier::Medium),
        "high" => Ok(ModelTier::High),
        other => Err(format!(
            "invalid default_tier '{other}' (expected low | medium | high)"
        )),
    }
}

fn parse_payload(raw: &str) -> Result<PayloadMode, String> {
    match raw {
        "off" => Ok(PayloadMode::Off),
        "summary" => Ok(PayloadMode::Summary),
        "full" => Ok(PayloadMode::Full),
        "state_ref" => Ok(PayloadMode::StateRef),
        other => Err(format!(
            "invalid subscription payload '{other}' (expected off | summary | full | state_ref)"
        )),
    }
}

fn parse_trigger(raw: &str) -> Result<TriggerMode, String> {
    match raw {
        "never" => Ok(TriggerMode::Never),
        "conversation" => Ok(TriggerMode::Conversation),
        "ambient" => Ok(TriggerMode::Ambient),
        "ask" => Ok(TriggerMode::Ask),
        other => Err(format!(
            "invalid subscription trigger '{other}' (expected never | conversation | ambient | ask)"
        )),
    }
}

fn parse_default(raw: &str) -> Result<Decision, String> {
    match raw {
        "allow" => Ok(Decision::Allow),
        "ask" => Ok(Decision::Ask),
        other => Err(format!(
            "invalid subscription default '{other}' (expected allow | ask)"
        )),
    }
}

impl AgentDefinition {
    /// Parse an agent from raw `AGENT.md` + `settings.toml` contents.
    /// Required fields fail fast with errors naming the field; nothing is
    /// silently defaulted.
    pub fn parse(prompt: &str, settings_toml: &str) -> Result<Self, String> {
        if prompt.trim().is_empty() {
            return Err("AGENT.md must be non-empty — it is the agent's system prompt".to_string());
        }
        let settings: SettingsToml = toml::from_str(settings_toml)
            .map_err(|e| format!("invalid settings.toml: {e}"))?;
        if settings.agent.id.trim().is_empty() {
            return Err("settings.toml: [agent] id must be non-empty".to_string());
        }
        if settings.agent.display_name.trim().is_empty() {
            return Err("settings.toml: [agent] display_name must be non-empty".to_string());
        }
        let default_tier = parse_tier(&settings.agent.default_tier)
            .map_err(|e| format!("settings.toml: [agent] {e}"))?;
        let posture = PermissionPosture::from_toml_str(settings_toml)
            .map_err(|e| format!("settings.toml: {e}"))?;
        let mut subscriptions = Vec::with_capacity(settings.subscriptions.len());
        for (i, sub) in settings.subscriptions.iter().enumerate() {
            let ctx = |e: String| format!("settings.toml: [[subscriptions]] #{}: {e}", i + 1);
            if sub.app.trim().is_empty() {
                return Err(ctx("'app' must be non-empty".to_string()));
            }
            if sub.events.is_empty() {
                return Err(ctx("'events' must be non-empty".to_string()));
            }
            subscriptions.push(AgentSubscriptionRequest {
                app: sub.app.clone(),
                events: sub.events.clone(),
                payload: parse_payload(&sub.payload).map_err(&ctx)?,
                trigger: parse_trigger(&sub.trigger).map_err(&ctx)?,
                default: parse_default(&sub.default).map_err(&ctx)?,
            });
        }
        Ok(Self {
            id: settings.agent.id,
            display_name: settings.agent.display_name,
            default_tier,
            prompt: prompt.to_string(),
            posture,
            subscriptions,
        })
    }

    /// Load one agent directory (`AGENT.md` + `settings.toml`).
    pub fn load_dir(dir: &Path) -> Result<Self, String> {
        let prompt_path = dir.join("AGENT.md");
        let settings_path = dir.join("settings.toml");
        let prompt = std::fs::read_to_string(&prompt_path)
            .map_err(|e| format!("failed to read {}: {e}", prompt_path.display()))?;
        let settings = std::fs::read_to_string(&settings_path)
            .map_err(|e| format!("failed to read {}: {e}", settings_path.display()))?;
        Self::parse(&prompt, &settings)
    }
}

/// Load every agent under `<workspace>/<workspace_channel_dir>/agents/`.
/// Missing dir = no agents (not an error). A broken agent dir is logged
/// loudly and skipped so one bad agent cannot take down the rest.
pub fn load_workspace_agents(workspace_root: &Path) -> Vec<AgentDefinition> {
    let agents_dir = workspace_root
        .join(crate::config::workspace_channel_dir())
        .join("agents");
    let Ok(entries) = std::fs::read_dir(&agents_dir) else {
        return Vec::new();
    };
    let mut agents = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        // Agent dirs without settings.toml are pane agents (`plexi agent
        // add` installs AGENT.md + memory/ + logs/), not runtime agents —
        // skip silently rather than erroring on a legitimate layout.
        if !path.join("settings.toml").is_file() {
            log::info!(
                "agent: {} has no settings.toml — not a runtime agent, skipping",
                path.display()
            );
            continue;
        }
        match AgentDefinition::load_dir(&path) {
            Ok(def) => {
                log::info!(
                    "agent: loaded '{}' ({}) from {}",
                    def.id,
                    def.display_name,
                    path.display()
                );
                agents.push(def);
            }
            Err(e) => {
                log::error!("agent: skipping {}: {e}", path.display());
            }
        }
    }
    agents
}

// ── Transcript ───────────────────────────────────────────────────────────────

/// One user-visible line in an agent's conversation record. The Phase D
/// Assistant UI renders this; until then it is surfaced via the host log.
#[derive(Debug, Clone, PartialEq)]
pub struct TranscriptEntry {
    /// `"event"` (injected app event), `"agent"` (model text), or `"error"`.
    pub role: String,
    pub text: String,
    /// RFC 3339.
    pub created_at: String,
}

// ── AgentRuntime ─────────────────────────────────────────────────────────────

/// Outcome of one completed agent turn, sent back from the worker thread.
struct TurnOutcome {
    agent_id: String,
    text: Option<String>,
    error: Option<String>,
}

/// One attached agent: definition + broker-granted subscriptions + transcript.
pub struct AgentRuntime {
    pub def: AgentDefinition,
    /// Subscription ids created at attach time (broker-allowed only).
    pub subscription_ids: Vec<String>,
    /// Host-visible conversation record (Phase D Assistant UI seam).
    pub transcript: Vec<TranscriptEntry>,
    /// Turns currently running on worker threads.
    in_flight: usize,
}

impl AgentRuntime {
    fn push_transcript(&mut self, role: &str, text: String) {
        log::info!("agent[{}] {role}: {text}", self.def.id);
        self.transcript.push(TranscriptEntry {
            role: role.to_string(),
            text,
            created_at: crate::host::event_log::now_timestamp(),
        });
    }
}

// ── AgentHost ────────────────────────────────────────────────────────────────

/// Host-level registry of attached agents. Owned by `PlexiApp`, ticked once
/// per frame. Holds its own broker grant store (same `grants.toml` as the
/// panes) and the shared app timeline.
pub struct AgentHost {
    pub agents: Vec<AgentRuntime>,
    pub grant_store: GrantStore,
    timeline: Arc<Mutex<AppTimeline>>,
    ai_broker: Arc<dyn AiBroker>,
    workspace_root: PathBuf,
    /// Where to re-read `grants.toml` from on workspace reload (`Some` in
    /// production). Until the Phase D grant sheet lands, grants are edited on
    /// disk, so a reload must pick them up. `None` (tests) keeps the
    /// in-memory store.
    grants_dir: Option<PathBuf>,
    outcome_tx: Sender<TurnOutcome>,
    outcome_rx: Receiver<TurnOutcome>,
}

impl AgentHost {
    /// Production constructor: grants from the channel config dir, the global
    /// shared timeline, and the live AI broker.
    pub fn new(
        grant_store: GrantStore,
        timeline: Arc<Mutex<AppTimeline>>,
        ai_broker: Arc<dyn AiBroker>,
        workspace_root: PathBuf,
    ) -> Self {
        let (outcome_tx, outcome_rx) = std::sync::mpsc::channel();
        Self {
            agents: Vec::new(),
            grant_store,
            timeline,
            ai_broker,
            workspace_root,
            grants_dir: None,
            outcome_tx,
            outcome_rx,
        }
    }

    /// Re-point the host at a (possibly different or newly active) workspace.
    /// Called from `apply_context_transition_effects` — the same choke point
    /// that rescans the app registry — so agents defined in a workspace that
    /// becomes active after boot are picked up. Detaches every current agent
    /// (removing its timeline subscriptions), re-reads grants from disk when
    /// `grants_dir` is set, and attaches the new root's agents. `None` =
    /// no active workspace = no agents.
    pub fn reload_workspace(&mut self, workspace_root: Option<PathBuf>) {
        if !self.agents.is_empty() {
            let mut timeline = self.timeline.lock().unwrap();
            for agent in &self.agents {
                for sub_id in &agent.subscription_ids {
                    timeline.remove_subscription(sub_id);
                }
            }
        }
        self.agents.clear();
        if let Some(dir) = &self.grants_dir {
            self.grant_store = GrantStore::load_or_default(dir);
        }
        let Some(root) = workspace_root else {
            log::info!("agent: no active workspace — no agents attached");
            return;
        };
        self.workspace_root = root.clone();
        for def in load_workspace_agents(&root) {
            self.attach(def);
        }
        log::info!(
            "agent: workspace reload — {} agent(s) attached for {}",
            self.agents.len(),
            root.display()
        );
    }

    /// Attach an agent: register it as an `ActorType::Agent` actor and create
    /// broker-gated subscriptions for its requested event streams. Every
    /// event target (`app_event_stream:<app>::<event>`) must evaluate to a
    /// persisted user `Allow`; anything else (including posture-only allows)
    /// is refused — subscriptions always need an explicit user grant. The
    /// agent is attached either way; without subscriptions it sees nothing.
    pub fn attach(&mut self, def: AgentDefinition) {
        let mut subscription_ids = Vec::new();
        for sub in &def.subscriptions {
            let mut granted = true;
            for event in &sub.events {
                let target = format!("{}::{}", sub.app, event);
                let req = PermissionRequest::new(
                    ActorType::Agent,
                    &def.id,
                    TargetType::AppEventStream,
                    &target,
                    Some(&self.workspace_root),
                );
                // Subscriptions deliberately evaluate WITHOUT the agent's own
                // posture: an agent must not self-grant event access from its
                // settings file. Only user/workspace/managed grants count.
                let decision = self.grant_store.evaluate(&req, None);
                if decision != Decision::Allow {
                    log::info!(
                        "agent[{}]: subscription to '{target}' not granted ({}) — skipping",
                        def.id,
                        decision.as_str()
                    );
                    granted = false;
                }
            }
            if !granted {
                continue;
            }
            let subscription_id = format!("agent-sub-{}", uuid::Uuid::new_v4());
            self.timeline.lock().unwrap().add_subscription(
                crate::host::app_timeline::SubscriptionRecord {
                    subscription_id: subscription_id.clone(),
                    subscriber_type: ActorType::Agent,
                    subscriber_id: def.id.clone(),
                    app_id: sub.app.clone(),
                    event_names: sub.events.clone(),
                    payload_mode: sub.payload,
                    trigger_mode: sub.trigger,
                    resource_id: None,
                    duration: GrantDuration::Session,
                    created_at: crate::host::event_log::now_timestamp(),
                },
            );
            subscription_ids.push(subscription_id);
        }
        log::info!(
            "agent: attached '{}' ({}) with {} subscription(s)",
            def.id,
            def.display_name,
            subscription_ids.len()
        );
        self.agents.push(AgentRuntime {
            def,
            subscription_ids,
            transcript: Vec::new(),
            in_flight: 0,
        });
    }

    /// Per-frame tick: collect finished turn outcomes, then consume queued
    /// event deliveries and trigger turns for conversation-mode events.
    pub fn tick(&mut self) {
        // 1. Drain completed turns into transcripts.
        while let Ok(outcome) = self.outcome_rx.try_recv() {
            let Some(agent) = self.agents.iter_mut().find(|a| a.def.id == outcome.agent_id)
            else {
                log::warn!("agent: turn outcome for unknown agent '{}'", outcome.agent_id);
                continue;
            };
            agent.in_flight = agent.in_flight.saturating_sub(1);
            if let Some(text) = outcome.text {
                agent.push_transcript("agent", text);
            }
            if let Some(error) = outcome.error {
                agent.push_transcript("error", error);
            }
        }

        // 2. Consume deliveries and trigger turns.
        if self.agents.is_empty() {
            return;
        }
        if self.timeline.lock().unwrap().pending_delivery_count() == 0 {
            return;
        }
        for i in 0..self.agents.len() {
            // An agent without granted subscriptions can have no deliveries.
            if self.agents[i].subscription_ids.is_empty() {
                continue;
            }
            let agent_id = self.agents[i].def.id.clone();
            let deliveries = self
                .timeline
                .lock()
                .unwrap()
                .take_deliveries_for(ActorType::Agent, &agent_id);
            if deliveries.is_empty() {
                continue;
            }
            self.handle_deliveries(i, deliveries);
        }
    }

    fn handle_deliveries(&mut self, agent_idx: usize, deliveries: Vec<EventDelivery>) {
        let mut conversation_events = Vec::new();
        // Broker identity this agent's tool calls carry (`ToolDispatcher`
        // caller id) — events it caused come back stamped with it.
        let self_id = format!("agent:{}", self.agents[agent_idx].def.id);
        for d in deliveries {
            let agent = &mut self.agents[agent_idx];
            // Never trigger an agent from its own actions: events it emitted
            // as the actor, or events an app emitted while servicing one of
            // its tool calls (`caused_by`). Without this, a move → move.played
            // → turn → move feedback loop plays the game against itself.
            let self_caused = d.actor_id == self_id
                || d.caused_by.as_deref() == Some(self_id.as_str());
            if self_caused {
                log::info!(
                    "agent[{}]: delivery {} of '{}' is self-caused — recorded, no turn",
                    agent.def.id,
                    d.delivery_id,
                    d.event
                );
            }
            let line = format!(
                "[{}] {} {}{}",
                d.app_id,
                d.event,
                d.summary.as_deref().unwrap_or("(no summary)"),
                d.payload
                    .as_ref()
                    .map(|p| format!(" payload={p}"))
                    .unwrap_or_default(),
            );
            agent.push_transcript("event", line.clone());
            match d.trigger_mode {
                TriggerMode::Conversation if !self_caused => conversation_events.push(line),
                TriggerMode::Conversation => {}
                // Ambient workflows and ask-prompts are Phase D surface; the
                // event is recorded in the transcript either way.
                TriggerMode::Ambient | TriggerMode::Ask | TriggerMode::Never => {
                    log::info!(
                        "agent[{}]: trigger {:?} for '{}' recorded without a turn (Phase D)",
                        agent.def.id,
                        d.trigger_mode,
                        d.event
                    );
                }
            }
        }
        if conversation_events.is_empty() {
            return;
        }
        self.start_turn(agent_idx, conversation_events);
    }

    /// Build the broker-gated tool snapshot for an agent: tools visible in
    /// this workspace whose `app_connector:app.<tool_name>` target evaluates
    /// to `Allow` for the agent (user grants + the agent's posture tiers).
    fn gated_dispatcher(&self, agent: &AgentRuntime) -> ToolDispatcher {
        let mut dispatcher = ToolDispatcher::from_registry(
            0,
            format!("agent:{}", agent.def.id),
            self.workspace_root.clone(),
        );
        let allowed: HashSet<String> = dispatcher
            .all_tools()
            .into_iter()
            .filter(|tool| {
                let req = PermissionRequest::new(
                    ActorType::Agent,
                    &agent.def.id,
                    TargetType::AppConnector,
                    &format!("app.{}", tool.name),
                    Some(&self.workspace_root),
                );
                let with_posture = self.grant_store.evaluate(&req, Some(&agent.def.posture));
                // `Ask` from the posture tier is satisfied by an explicit
                // user grant: re-evaluate without the posture — `Allow` then
                // means a persisted user/workspace allow exists (denies still
                // dominate both evaluations).
                let allowed = with_posture == Decision::Allow
                    || (with_posture == Decision::Ask
                        && self.grant_store.evaluate(&req, None) == Decision::Allow);
                if !allowed {
                    log::info!(
                        "agent[{}]: tool '{}' withheld from turn ({})",
                        agent.def.id,
                        tool.name,
                        with_posture.as_str()
                    );
                }
                allowed
            })
            .map(|tool| tool.name)
            .collect();
        dispatcher.retain_allowed(&allowed);
        dispatcher
    }

    fn start_turn(&mut self, agent_idx: usize, event_lines: Vec<String>) {
        let agent = &self.agents[agent_idx];
        let dispatcher = Arc::new(self.gated_dispatcher(agent));

        // Conversation history: prior transcript (events + agent replies)
        // followed by the new injected events as the user turn.
        let mut messages: Vec<AiMessage> = agent
            .transcript
            .iter()
            .filter(|e| e.role == "agent")
            .map(|e| AiMessage {
                role: "assistant".to_string(),
                content: e.text.clone(),
            })
            .collect();
        messages.push(AiMessage {
            role: "user".to_string(),
            content: format!(
                "App events delivered to you:\n{}",
                event_lines.join("\n")
            ),
        });

        let request = AiBrokerRequest {
            app_id: format!("agent:{}", agent.def.id),
            model_tier: agent.def.default_tier,
            system: agent.def.prompt.clone(),
            messages,
            tools: Vec::new(),
            workspace_root: Some(self.workspace_root.clone()),
            open_panes: crate::plexi_ai::broker::get_pane_snapshot(),
            tool_dispatcher: Some(dispatcher),
        };
        let agent_id = agent.def.id.clone();
        log::info!(
            "agent[{agent_id}]: starting conversation turn ({} event line(s))",
            event_lines.len()
        );
        let broker = Arc::clone(&self.ai_broker);
        let outcome_tx = self.outcome_tx.clone();
        let spawn = std::thread::Builder::new()
            .name(format!("agent-turn-{agent_id}"))
            .spawn(move || {
                let resp = broker.dispatch(request);
                let _ = outcome_tx.send(TurnOutcome {
                    agent_id,
                    text: resp.content,
                    error: resp.error,
                });
            });
        match spawn {
            Ok(_) => self.agents[agent_idx].in_flight += 1,
            Err(e) => {
                let agent = &mut self.agents[agent_idx];
                let msg = format!("failed to spawn agent turn thread: {e}");
                log::error!("agent[{}]: {msg}", agent.def.id);
                agent.push_transcript("error", msg);
            }
        }
    }

    /// True while any agent has a turn running on a worker thread.
    pub fn turns_in_flight(&self) -> bool {
        self.agents.iter().any(|a| a.in_flight > 0)
    }

    /// Production constructor: grants from the channel config dir, the
    /// global shared timeline, and the live AI broker. Agents are NOT loaded
    /// here — `apply_context_transition_effects` runs at the end of every
    /// `PlexiApp` constructor and on every context change, and its
    /// `reload_workspace` call is the single owner of agent loading.
    pub fn production(ai_config: Option<crate::config::AiConfig>) -> Self {
        let config_dir = crate::config::config_dir();
        let grant_store = GrantStore::load_or_default(&config_dir);
        // The config dir is an inert root that matches no pane's workspace —
        // replaced by the first `reload_workspace` with an active workspace.
        let mut host = Self::new(
            grant_store,
            crate::host::app_timeline::global(),
            Arc::new(crate::plexi_ai::broker::LiveAiBroker::new(ai_config)),
            config_dir.clone(),
        );
        host.grants_dir = Some(config_dir);
        host
    }

    /// Inert constructor for `PlexiApp::new_for_test`: isolated timeline,
    /// empty grants, and a broker that fails fast if a turn is ever started.
    #[cfg(test)]
    pub fn inert() -> Self {
        Self::new(
            GrantStore::default(),
            Arc::new(Mutex::new(AppTimeline::default())),
            Arc::new(InertAiBroker),
            std::env::temp_dir(),
        )
    }

    /// Test constructor: isolated timeline + injected broker, no disk I/O.
    #[cfg(test)]
    pub fn new_for_test(
        timeline: Arc<Mutex<AppTimeline>>,
        ai_broker: Arc<dyn AiBroker>,
        workspace_root: PathBuf,
    ) -> Self {
        Self::new(GrantStore::default(), timeline, ai_broker, workspace_root)
    }
}

/// Broker for inert hosts: never queries a model, always errors.
#[cfg(test)]
struct InertAiBroker;

#[cfg(test)]
impl AiBroker for InertAiBroker {
    fn dispatch(&self, request: AiBrokerRequest) -> crate::plexi_ai::broker::AiBrokerResponse {
        log::warn!(
            "agent: InertAiBroker dispatch for '{}' — no AI broker in this context",
            request.app_id
        );
        crate::plexi_ai::broker::AiBrokerResponse::err("no AI broker in this context")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SETTINGS: &str = r#"
[agent]
id = "chess-opponent"
display_name = "Chess Opponent"
default_tier = "medium"

[permissions]
default_posture = "review"

allow = [
  "app.chess.current_state",
  "app.chess.legal_moves",
]

ask = [
  "app.chess.make_move",
  "app.chess.undo_move",
]

[[subscriptions]]
app = "chess"
events = ["game.started", "turn.ready", "move.played", "move.undone", "game.ended"]
payload = "full"
trigger = "conversation"
default = "ask"
"#;

    #[test]
    fn spec_settings_example_parses() {
        let def = AgentDefinition::parse("You are a chess opponent.", SETTINGS)
            .expect("spec settings.toml must parse");
        assert_eq!(def.id, "chess-opponent");
        assert_eq!(def.display_name, "Chess Opponent");
        assert_eq!(def.default_tier, ModelTier::Medium);
        assert_eq!(def.posture.default_posture, Decision::Ask);
        assert_eq!(def.posture.allow.len(), 2);
        assert_eq!(def.posture.ask.len(), 2);
        assert_eq!(def.subscriptions.len(), 1);
        let sub = &def.subscriptions[0];
        assert_eq!(sub.app, "chess");
        assert_eq!(sub.events.len(), 5);
        assert_eq!(sub.payload, PayloadMode::Full);
        assert_eq!(sub.trigger, TriggerMode::Conversation);
        assert_eq!(sub.default, Decision::Ask);
    }

    #[test]
    fn required_fields_fail_fast() {
        // Empty prompt.
        let err = AgentDefinition::parse("  ", SETTINGS).unwrap_err();
        assert!(err.contains("AGENT.md"), "{err}");

        // Missing [agent] table.
        let err = AgentDefinition::parse("p", "[permissions]\ndefault_posture = \"ask\"\n")
            .unwrap_err();
        assert!(err.contains("settings.toml"), "{err}");

        // Bad tier.
        let bad_tier = SETTINGS.replace("default_tier = \"medium\"", "default_tier = \"huge\"");
        let err = AgentDefinition::parse("p", &bad_tier).unwrap_err();
        assert!(err.contains("huge"), "error must name the bad value: {err}");

        // Missing [permissions] table — no silent default posture.
        let no_perms = r#"
[agent]
id = "a"
display_name = "A"
default_tier = "low"
"#;
        assert!(AgentDefinition::parse("p", no_perms).is_err());

        // Bad subscription trigger.
        let bad_trigger = SETTINGS.replace("trigger = \"conversation\"", "trigger = \"sometimes\"");
        let err = AgentDefinition::parse("p", &bad_trigger).unwrap_err();
        assert!(err.contains("sometimes"), "{err}");

        // Empty subscription events.
        let bad_events = SETTINGS.replace(
            "events = [\"game.started\", \"turn.ready\", \"move.played\", \"move.undone\", \"game.ended\"]",
            "events = []",
        );
        let err = AgentDefinition::parse("p", &bad_events).unwrap_err();
        assert!(err.contains("events"), "{err}");
    }

    #[test]
    fn load_dir_reads_agent_files_and_load_workspace_skips_broken() {
        let ws = tempfile::tempdir().unwrap();
        let agents_dir = ws
            .path()
            .join(crate::config::workspace_channel_dir())
            .join("agents");
        let good = agents_dir.join("chess-opponent");
        std::fs::create_dir_all(&good).unwrap();
        std::fs::write(good.join("AGENT.md"), "You are a chess opponent.").unwrap();
        std::fs::write(good.join("settings.toml"), SETTINGS).unwrap();
        let broken = agents_dir.join("broken");
        std::fs::create_dir_all(&broken).unwrap();
        std::fs::write(broken.join("AGENT.md"), "prose").unwrap();
        std::fs::write(broken.join("settings.toml"), "not toml ][[").unwrap();

        let def = AgentDefinition::load_dir(&good).expect("good agent dir must load");
        assert_eq!(def.id, "chess-opponent");
        assert!(AgentDefinition::load_dir(&broken).is_err());

        let agents = load_workspace_agents(ws.path());
        assert_eq!(agents.len(), 1, "broken agent must be skipped, not fatal");
        assert_eq!(agents[0].id, "chess-opponent");

        // No agents dir at all → empty, not an error.
        let empty_ws = tempfile::tempdir().unwrap();
        assert!(load_workspace_agents(empty_ws.path()).is_empty());
    }

    /// Context transitions must reload agents: a host built before the
    /// workspace became active picks its agents up on the next transition,
    /// and switching away detaches them and removes their subscriptions.
    #[test]
    fn reload_workspace_attaches_and_detaches_agents() {
        use crate::broker::{
            ActorScope, GrantRecord, GrantSource, ResourceScope,
        };

        let ws = tempfile::tempdir().unwrap();
        let agent_dir = ws
            .path()
            .join(crate::config::workspace_channel_dir())
            .join("agents")
            .join("chess-opponent");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(agent_dir.join("AGENT.md"), "You are a chess opponent.").unwrap();
        std::fs::write(agent_dir.join("settings.toml"), SETTINGS).unwrap();

        let timeline = Arc::new(Mutex::new(AppTimeline::default()));
        // Host built with NO workspace — the boot-before-workspace case.
        let mut host = AgentHost::new_for_test(
            timeline.clone(),
            Arc::new(InertAiBroker),
            std::env::temp_dir(),
        );
        // Persisted user grants for every requested stream so the
        // subscription attaches.
        for event in [
            "game.started",
            "turn.ready",
            "move.played",
            "move.undone",
            "game.ended",
        ] {
            host.grant_store.record(GrantRecord {
                actor_type: ActorType::Agent,
                actor_id: "chess-opponent".to_string(),
                actor_scope: ActorScope::User,
                workspace_root: None,
                target_type: TargetType::AppEventStream,
                target_id: format!("chess::{event}"),
                resource_scope: ResourceScope::Game,
                resource_id: None,
                decision: Decision::Allow,
                duration: GrantDuration::Session,
                source: GrantSource::User,
                created_at: 0,
                expires_at: None,
            });
        }
        assert!(host.agents.is_empty());

        // Workspace becomes active → agent attaches with its subscription.
        host.reload_workspace(Some(ws.path().to_path_buf()));
        assert_eq!(host.agents.len(), 1);
        assert_eq!(host.agents[0].subscription_ids.len(), 1);
        assert_eq!(timeline.lock().unwrap().subscriptions().len(), 1);

        // Reload onto the same workspace must not duplicate.
        host.reload_workspace(Some(ws.path().to_path_buf()));
        assert_eq!(host.agents.len(), 1);
        assert_eq!(timeline.lock().unwrap().subscriptions().len(), 1);

        // Switching to no workspace detaches and removes subscriptions.
        host.reload_workspace(None);
        assert!(host.agents.is_empty());
        assert!(timeline.lock().unwrap().subscriptions().is_empty());
    }

    /// Self-caused deliveries (the agent as actor, or events an app emitted
    /// while servicing the agent's tool call) must never trigger a turn —
    /// otherwise an agent that moves causes `move.played`/`turn.ready` which
    /// trigger it again, and it plays the game against itself.
    #[test]
    fn self_caused_deliveries_do_not_trigger_turns() {
        use crate::app_protocol::AppEventActor;
        use crate::host::app_timeline::EventDelivery;

        let timeline = Arc::new(Mutex::new(AppTimeline::default()));
        let mut host = AgentHost::new_for_test(
            timeline,
            Arc::new(InertAiBroker),
            std::env::temp_dir(),
        );
        host.attach(AgentDefinition::parse("prose", SETTINGS).unwrap());
        assert_eq!(host.agents[0].in_flight, 0);

        let delivery = |id: u64, actor: AppEventActor, actor_id: &str, caused_by: Option<&str>| {
            EventDelivery {
                delivery_id: id,
                subscription_id: "sub-1".to_string(),
                subscriber_type: ActorType::Agent,
                subscriber_id: "chess-opponent".to_string(),
                trigger_mode: TriggerMode::Conversation,
                app_id: "chess".to_string(),
                event: "turn.ready".to_string(),
                event_id: id,
                resource_id: "game-1".to_string(),
                actor,
                actor_id: actor_id.to_string(),
                caused_by: caused_by.map(str::to_string),
                summary: Some("black to move".to_string()),
                payload: None,
                state_ref: None,
                created_at: "2026-01-01T00:00:00Z".to_string(),
            }
        };

        // Agent's own move (actor_id matches its caller identity): no turn.
        host.handle_deliveries(
            0,
            vec![delivery(1, AppEventActor::Agent, "agent:chess-opponent", None)],
        );
        assert_eq!(host.agents[0].in_flight, 0, "own action must not trigger");

        // App-emitted event caused by the agent's tool call: no turn.
        host.handle_deliveries(
            0,
            vec![delivery(2, AppEventActor::App, "chess", Some("agent:chess-opponent"))],
        );
        assert_eq!(host.agents[0].in_flight, 0, "caused-by-self must not trigger");

        // Both still land in the transcript as event lines.
        assert_eq!(host.agents[0].transcript.len(), 2);

        // A user-caused event triggers a turn.
        host.handle_deliveries(0, vec![delivery(3, AppEventActor::User, "chess", None)]);
        assert_eq!(host.agents[0].in_flight, 1, "user event must trigger");
    }
}
