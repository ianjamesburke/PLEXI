//! Host Assistant — Phase 1 of `docs/assistant-host-app.md`.
//!
//! The Assistant is a first-party host app: a `Pane::App(AppRuntime::Builtin)`
//! pane, not a PGAP process. It is split into pure state (`model`), slash
//! command parsing (`commands`), disk persistence (`store`), and egui
//! rendering (`render`); this module is the pane shell that wires those to
//! the `App` trait and runs model turns on worker threads — the same
//! dispatch-on-worker / outcome-channel pattern as `crate::agent::AgentHost`.

pub mod audit;
pub mod commands;
#[cfg(test)]
pub mod harness;
pub mod model;
pub mod render;
pub mod settings;
pub mod skills;
pub mod store;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, SyncSender};
use std::sync::{Arc, Mutex};

use crate::agent::{AgentDefinition, AgentRegistry, AgentSource};
use crate::app::app_trait::AppCommand;
use crate::app::app_trait::{App, AppRenderContext, KeyDisposition};
use crate::app_protocol::{AiMessage, AiTool, ModelTier, PayloadMode, TriggerMode};
use crate::broker::{
    ActorScope, ActorType, Decision, GrantDuration, GrantRecord, GrantSource, GrantStore,
    PermissionRequest, ResourceScope, TargetType,
};
use crate::host::app_timeline::AppTimeline;
use crate::plexi_ai::broker::{AiBroker, AiBrokerRequest, ReasoningEffort};
use crate::plexi_ai::turn_loop::TurnDelta;
use crate::plexi_ai::CancelToken;

/// Owned streaming delta forwarded from the broker worker thread to the UI
/// thread (`TurnDelta` borrows from the stream buffer and cannot cross the
/// channel).
enum StreamDelta {
    Answer(String),
    Reasoning(String),
}
use crate::plexi_ai::tool_dispatch::{
    HostToolHandler, ToolCallHooks, ToolCallResult, ToolDispatcher,
};

use audit::{AuditEvent, AuditLog};
use model::{
    AgentChoice, AssistantEffect, AssistantModel, AssistantOverlay, GrantRow, PermissionChoice,
    TurnRole,
};
use render::{AssistantRenderer, ComposerEvent, MarkdownTextCache};
use settings::{AssistantSettings, SessionOverrides, SettingsLoadError, SettingsLoader};
use skills::{SkillDefinition, SkillRegistry};
use store::AssistantStore;

pub(crate) const DEFAULT_AGENT_PROMPT: &str = "You are the Plexi Assistant, the workspace \
operator inside the Plexi terminal environment. Answer concisely. Tools exposed \
by running apps are available to you when listed; calls may pause for the user's \
permission. You can subscribe to app event streams with the host tool \
host.events.subscribe (input: {\"app\": \"<app_id>\", \"event\": \"<event name or *>\"}) \
and stop with host.events.unsubscribe — subscribing may pause for the user's \
permission. Once subscribed, delivered events appear in this conversation and you \
should respond to them. IMPORTANT: when the user starts any interactive or ongoing \
activity in an app (playing a game, watching a process, editing a document) or asks \
you to react when something happens, your FIRST action must be to call \
host.events.subscribe for that app's relevant events — without a subscription you \
will never see the user's actions, so do not assume you will be notified. After \
subscribing, tell the user you are now watching those events. \
When a delivered event hands you the turn, ACT: if the situation calls for a \
tool call, make it immediately rather than only describing what you would do. \
Do not narrate intentions you can carry out — take the action, then report the \
result. \
Use the host.panes.*, host.apps.open, host.terminals.open, and host.terminals.run \
tools for native pane, app, and terminal operations; never shell out to the plexi \
CLI. When a terminal pane has already been opened or focused, reuse its pane id \
with host.terminals.run; do not open a redundant terminal. host.terminals.run \
does not return the command's output — after every run call, read the result \
with host.terminals.read before deciding your next step; never assume a command \
succeeded or guess at paths it printed. \
When the user asks you to build an app, game, or tool, build it as a Plexi app, \
never as a loose script: scaffold with `plexi app init --global <kebab-name>` in a \
terminal, read the scaffolded AGENTS.md to learn the SDK, write main.py, validate \
with `plexi app check .` from the app directory, then open it with host.apps.open. \
The app-authoring commands `plexi app init` and `plexi app check` run via \
host.terminals.run and are the one sanctioned use of the plexi CLI. ";

/// Host tool names the Assistant injects into its dispatcher snapshot.
const HOST_TOOL_SUBSCRIBE: &str = "host.events.subscribe";
const HOST_TOOL_UNSUBSCRIBE: &str = "host.events.unsubscribe";
const HOST_TOOL_PANES_LIST: &str = "host.panes.list";
const HOST_TOOL_PANES_STATE: &str = "host.panes.state";
const HOST_TOOL_PANES_OPEN: &str = "host.panes.open";
const HOST_TOOL_PANES_FOCUS: &str = "host.panes.focus";
const HOST_TOOL_PANES_CLOSE: &str = "host.panes.close";
const HOST_TOOL_APPS_OPEN: &str = "host.apps.open";
const HOST_TOOL_TERMINALS_OPEN: &str = "host.terminals.open";
const HOST_TOOL_TERMINALS_RUN: &str = "host.terminals.run";
const HOST_TOOL_TERMINALS_READ: &str = "host.terminals.read";

/// Broker identity for the Assistant: actor id at the permission tiers,
/// `agent:assistant` as the `ToolDispatcher` caller id (Phase C convention).
const ASSISTANT_ACTOR_ID: &str = "assistant";

/// Outcome of one completed Assistant turn, sent back from the worker thread.
struct TurnOutcome {
    conversation_id: String,
    text: Option<String>,
    error: Option<String>,
}

/// The worker's answer channel for one ask-gated tool call.
enum PermissionReply {
    /// Run the call. `remember` lets the in-turn gate skip re-asking for the
    /// same tool (session/always grants).
    Allow {
        remember: bool,
    },
    Deny,
}

/// Tool-flow notifications from the broker worker thread to the pane.
enum ToolFlowEvent {
    /// A tool call is about to run (passed the gate).
    Started { tool: String },
    /// A tool call finished (`error: None` = success).
    Finished { tool: String, error: Option<String> },
    /// An ask-gated tool needs a user decision. The worker is blocked on
    /// `reply` until the sheet resolves (or the pane drops the sender).
    Ask {
        tool: String,
        input_json: String,
        actor_id: String,
        actor_scope: ActorScope,
        reply: SyncSender<PermissionReply>,
    },
    /// A host tool call (`host.events.*`) for the pane to execute on the UI
    /// thread, where the grant store and timeline live. The worker blocks on
    /// `reply`; an ask-gated subscribe holds the reply until the sheet
    /// resolves.
    HostCall {
        tool: String,
        input_json: String,
        reply: SyncSender<ToolCallResult>,
    },
}

/// The Assistant's per-turn ask-gate, installed on the turn's
/// `ToolDispatcher` snapshot. Runs on the broker worker thread; `Ask` blocks
/// until the UI thread answers. PGAP and `AgentHost` dispatchers install no
/// hooks and are unaffected.
struct AssistantToolHooks {
    /// Tools whose broker decision was `Ask` at snapshot time.
    ask_tools: HashSet<String>,
    /// Tools the user allowed with "remember" during this turn.
    session_allowed: Mutex<HashSet<String>>,
    flow_tx: Sender<ToolFlowEvent>,
    actor_id: String,
    actor_scope: ActorScope,
}

impl ToolCallHooks for AssistantToolHooks {
    fn before_call(&self, name: &str, input_json: &str) -> Result<(), String> {
        let needs_ask =
            self.ask_tools.contains(name) && !self.session_allowed.lock().unwrap().contains(name);
        if needs_ask {
            let (reply_tx, reply_rx) = std::sync::mpsc::sync_channel(1);
            self.flow_tx
                .send(ToolFlowEvent::Ask {
                    tool: name.to_string(),
                    input_json: input_json.to_string(),
                    actor_id: self.actor_id.clone(),
                    actor_scope: self.actor_scope,
                    reply: reply_tx,
                })
                .map_err(|_| "permission_denied: assistant pane closed".to_string())?;
            match reply_rx.recv() {
                Ok(PermissionReply::Allow { remember }) => {
                    if remember {
                        self.session_allowed
                            .lock()
                            .unwrap()
                            .insert(name.to_string());
                    }
                }
                Ok(PermissionReply::Deny) | Err(_) => {
                    return Err("permission_denied: the user denied this tool call".to_string());
                }
            }
        }
        let _ = self.flow_tx.send(ToolFlowEvent::Started {
            tool: name.to_string(),
        });
        Ok(())
    }

    fn after_call(&self, name: &str, error: Option<&str>) {
        let _ = self.flow_tx.send(ToolFlowEvent::Finished {
            tool: name.to_string(),
            error: error.map(str::to_string),
        });
    }
}

/// Compact single-line summary of a tool input for sheets and audit lines.
fn summarize_input(input_json: &str) -> String {
    let flat: String = input_json.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > 120 {
        let truncated: String = flat.chars().take(117).collect();
        format!("{truncated}…")
    } else {
        flat
    }
}

fn format_setting_ids(ids: &[String]) -> String {
    if ids.is_empty() {
        "none".to_string()
    } else {
        ids.iter()
            .map(|id| format!("`{id}`"))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn bounded_head_tail(text: &str, budget: usize) -> String {
    let flat = text.replace('\n', " ");
    let chars = flat.chars().collect::<Vec<_>>();
    if chars.len() <= budget {
        return flat;
    }
    if budget < 12 {
        return chars.into_iter().take(budget).collect();
    }

    let mut keep = budget;
    for _ in 0..2 {
        let omitted = chars.len().saturating_sub(keep);
        let marker_len = format!(" [{omitted} chars omitted] ").chars().count();
        keep = budget.saturating_sub(marker_len);
    }
    let head = keep.div_ceil(2);
    let tail = keep.saturating_sub(head);
    let omitted = chars.len().saturating_sub(head + tail);
    let marker = format!(" [{omitted} chars omitted] ");
    let mut out = chars.iter().take(head).collect::<String>();
    out.push_str(&marker);
    out.extend(chars.iter().skip(chars.len() - tail));
    out.chars().take(budget).collect()
}

fn deterministic_context_summary(turns: &[model::Turn], budget: usize) -> String {
    let mut out = String::new();
    for (index, turn) in turns.iter().enumerate() {
        let remaining = budget.saturating_sub(out.chars().count());
        let turns_left = turns.len() - index;
        let label = format!("- {:?}: ", turn.role);
        let fair_share = (remaining / turns_left).min(640);
        if fair_share <= label.chars().count() + 1 {
            let omitted = turns.len() - index;
            let marker = format!("- [{omitted} older turn(s) omitted; see raw checkpoint]");
            let available = budget.saturating_sub(out.chars().count());
            out.push_str(&bounded_head_tail(&marker, available));
            break;
        }
        let content_budget = fair_share - label.chars().count() - 1;
        out.push_str(&label);
        out.push_str(&bounded_head_tail(&turn.text, content_budget));
        out.push('\n');
    }
    out.truncate(
        out.char_indices()
            .nth(budget)
            .map(|(index, _)| index)
            .unwrap_or(out.len()),
    );
    out
}

/// The host Assistant pane: model + store + broker + grant wiring.
pub struct AssistantApp {
    pub(crate) model: AssistantModel,
    store: AssistantStore,
    broker: Arc<dyn AiBroker>,
    workspace_root: PathBuf,
    profile_dir: PathBuf,
    agent_registry: AgentRegistry,
    skill_registry: SkillRegistry,
    pending_skill: Option<SkillDefinition>,
    pending_commands: Vec<AppCommand>,
    settings_loader: SettingsLoader,
    session_overrides: SessionOverrides,
    settings: AssistantSettings,
    settings_errors: Vec<SettingsLoadError>,
    /// Unified broker grants (same `grants.toml` shape as `AgentHost`).
    /// Session grants are recorded in memory only; "always" grants are saved.
    grant_store: GrantStore,
    audit: AuditLog,
    outcome_tx: Sender<TurnOutcome>,
    outcome_rx: Receiver<TurnOutcome>,
    /// Live deltas from the in-flight turn's worker thread.
    delta_rx: Option<Receiver<StreamDelta>>,
    flow_tx: Sender<ToolFlowEvent>,
    flow_rx: Receiver<ToolFlowEvent>,
    /// Reply channel for the worker blocked on the pending permission sheet.
    pending_reply: Option<SyncSender<PermissionReply>>,
    pending_connector_actor: Option<(String, ActorScope)>,
    /// Shared app event timeline (production: the global instance).
    timeline: Arc<Mutex<AppTimeline>>,
    /// Live event-stream subscriptions: `(target_id, subscription_id)` where
    /// `target_id` is `"<app>::<event>"`.
    live_subs: Vec<(String, String)>,
    /// Pending ask-gated `host.events.subscribe`: the stream target plus the
    /// blocked worker's reply channel. Resolved by the permission sheet.
    pending_subscribe: Option<PendingSubscribe>,
    /// Trigger lines for non-self-caused deliveries that arrived while a
    /// turn was in flight — folded into the next dispatched turn.
    queued_event_lines: Vec<String>,
    /// Cancellation handle for the in-flight turn. A fresh token is installed
    /// at each `start_turn`; ESC and event-preempt trip it to abort the
    /// streaming turn and fold queued context into an immediate follow-up.
    turn_cancel: CancelToken,
    /// Cached layout state for `egui_commonmark` markdown rendering of
    /// assistant replies. Persists across frames for performance.
    commonmark_cache: egui_commonmark::CommonMarkCache,
    /// Cached soft-break conversion for committed markdown turns.
    markdown_text_cache: MarkdownTextCache,
    /// `/compact` yields once so the renderer can show its progress row before
    /// the existing synchronous storage operation starts.
    compact_pending: bool,
}

/// An ask-gated subscribe waiting on the permission sheet.
struct PendingSubscribe {
    app: String,
    event: String,
    target: String,
    reply: SyncSender<ToolCallResult>,
}

impl AssistantApp {
    /// Open the Assistant for a workspace: resume the persisted active
    /// conversation, or create a fresh one. `profile_dir` is the channel
    /// config dir — grants load from `<profile_dir>/grants.toml` and audit
    /// events append to `<profile_dir>/audit.jsonl`.
    pub fn new(workspace_root: PathBuf, broker: Arc<dyn AiBroker>, profile_dir: &Path) -> Self {
        Self::new_with_timeline(
            workspace_root,
            broker,
            profile_dir,
            crate::host::app_timeline::global(),
        )
    }

    /// `new` with an explicit timeline — tests inject an isolated instance.
    pub fn new_with_timeline(
        workspace_root: PathBuf,
        broker: Arc<dyn AiBroker>,
        profile_dir: &Path,
        timeline: Arc<Mutex<AppTimeline>>,
    ) -> Self {
        let store = AssistantStore::new(&workspace_root);
        let mut model = match store.active_conversation() {
            Some(id) => {
                let turns = store.load_turns(&id);
                log::info!(
                    "assistant: resuming conversation {id} ({} turn(s)) for {}",
                    turns.len(),
                    workspace_root.display()
                );
                AssistantModel::resume(id, turns)
            }
            None => {
                let model = AssistantModel::fresh();
                log::info!(
                    "assistant: created conversation {} for {}",
                    model.conversation_id,
                    workspace_root.display()
                );
                model
            }
        };
        // Load persisted session name (if any) so it survives restarts.
        model.session_name = store.active_session_name();
        model.show_thoughts = store.show_thoughts();
        model.active_agent_id = store
            .active_agent_id()
            .unwrap_or_else(|| "default".to_string());
        model.effort_override = store.effort_override();
        match store.recover_interrupted_turn(&model.conversation_id) {
            Ok(true) => {
                model.turns.push(model::Turn::now(
                    TurnRole::Error,
                    "The previous model/tool turn was interrupted when Plexi stopped.",
                ));
                log::info!(
                    "assistant[{}]: recovered interrupted turn",
                    model.conversation_id
                );
            }
            Ok(false) => {}
            Err(e) => log::error!(
                "assistant[{}]: failed to recover interrupted turn: {e}",
                model.conversation_id
            ),
        }
        let settings_loader = SettingsLoader::new(profile_dir, &workspace_root);
        let agent_registry = AgentRegistry::load(profile_dir, &workspace_root);
        let skill_registry = SkillRegistry::load(profile_dir, &workspace_root);
        if agent_registry.active(&model.active_agent_id).is_none() {
            log::warn!(
                "assistant: persisted agent '{}' is unavailable; using default",
                model.active_agent_id
            );
            model.active_agent_id = "default".to_string();
        }
        let session_overrides = SessionOverrides::default();
        let settings_report = settings_loader.load(&session_overrides);
        let (outcome_tx, outcome_rx) = std::sync::mpsc::channel();
        let (flow_tx, flow_rx) = std::sync::mpsc::channel();
        let mut app = Self {
            model,
            store,
            broker,
            workspace_root,
            profile_dir: profile_dir.to_path_buf(),
            agent_registry,
            skill_registry,
            pending_skill: None,
            pending_commands: Vec::new(),
            settings_loader,
            session_overrides,
            settings: settings_report.settings,
            settings_errors: settings_report.errors,
            grant_store: GrantStore::load_or_default(profile_dir),
            audit: AuditLog::new(profile_dir.join("audit.jsonl")),
            outcome_tx,
            outcome_rx,
            delta_rx: None,
            flow_tx,
            flow_rx,
            pending_reply: None,
            pending_connector_actor: None,
            timeline,
            live_subs: Vec::new(),
            pending_subscribe: None,
            queued_event_lines: Vec::new(),
            turn_cancel: CancelToken::new(),
            commonmark_cache: egui_commonmark::CommonMarkCache::default(),
            markdown_text_cache: MarkdownTextCache::default(),
            compact_pending: false,
        };
        // Persist the active id immediately so close-then-reopen resumes
        // this conversation even before the first turn.
        app.session_write();
        // A previous pane instance may have leaked its subscriptions into the
        // shared timeline (close drops the pane without unsubscribing) —
        // clear them before re-registering, or every reopen duplicates each
        // delivery. Stale queued deliveries are dropped with them; the model
        // can read current app state instead of replaying history.
        app.timeline
            .lock()
            .unwrap()
            .clear_subscriber(ActorType::Agent, ASSISTANT_ACTOR_ID);
        // Persisted event-stream grants survive restarts: resubscribe them.
        app.resubscribe_granted_streams();
        app
    }

    /// Re-create timeline subscriptions for every persisted `Allow` grant on
    /// an event stream — same semantics as `AgentHost::attach`'s seeded
    /// grants. Runs on pane open so subscriptions survive restarts.
    fn resubscribe_granted_streams(&mut self) {
        let targets: Vec<String> = self
            .grant_store
            .records()
            .iter()
            .filter(|r| {
                r.actor_type == ActorType::Agent
                    && r.actor_id == ASSISTANT_ACTOR_ID
                    && r.target_type == TargetType::AppEventStream
                    && r.decision == Decision::Allow
            })
            .map(|r| r.target_id.clone())
            .collect();
        for target in targets {
            let Some((app, event)) = target.split_once("::") else {
                log::warn!("assistant: malformed event-stream grant target '{target}' — skipping");
                continue;
            };
            let (app, event) = (app.to_string(), event.to_string());
            if self.event_stream_decision(&app, &event) != Decision::Allow {
                log::info!(
                    "assistant: persisted subscription '{target}' withheld by permission posture"
                );
                continue;
            }
            self.subscribe_stream(&app, &event);
        }
        log::info!(
            "assistant: event-stream discovery — {} persisted subscription(s) restored",
            self.live_subs.len()
        );
    }

    /// Add a live timeline subscription for `<app>::<event>` (`*` = all
    /// streams the app declares). Caller must have established the grant.
    fn subscribe_stream(&mut self, app: &str, event: &str) {
        let target = format!("{app}::{event}");
        if self.live_subs.iter().any(|(t, _)| *t == target) {
            log::info!("assistant: already subscribed to '{target}'");
            return;
        }
        let event_names = if event == "*" {
            Vec::new()
        } else {
            vec![event.to_string()]
        };
        let subscription_id = crate::host::event_subscriptions::record_subscription(
            &self.timeline,
            app,
            ActorType::Agent,
            ASSISTANT_ACTOR_ID,
            event_names,
            PayloadMode::Full,
            TriggerMode::Conversation,
            None,
            GrantDuration::Session,
        );
        log::info!("assistant: subscribed to '{target}' ({subscription_id})");
        self.live_subs.push((target, subscription_id));
    }

    /// Remove live subscription(s) for one target. Returns how many.
    fn unsubscribe_stream(&mut self, target: &str) -> usize {
        let mut removed = 0;
        let mut timeline = self.timeline.lock().unwrap();
        self.live_subs.retain(|(t, sub_id)| {
            if t == target {
                timeline.remove_subscription(sub_id);
                removed += 1;
                false
            } else {
                true
            }
        });
        drop(timeline);
        if removed > 0 {
            log::info!("assistant: unsubscribed {removed} subscription(s) for '{target}'");
        }
        removed
    }

    /// Execute the effects a model transition returned.
    fn execute_effects(&mut self, effects: Vec<AssistantEffect>) {
        for effect in effects {
            match effect {
                AssistantEffect::AiQuery {
                    conversation_id,
                    prompt,
                } => self.start_turn(conversation_id, prompt),
                AssistantEffect::SessionWrite { .. } => self.session_write(),
                AssistantEffect::ListTools => self.cmd_list_tools(),
                AssistantEffect::ListApps => self.cmd_list_apps(),
                AssistantEffect::ListSkills => self.cmd_list_skills(),
                AssistantEffect::ShowContext => self.cmd_show_context(),
                AssistantEffect::ShowHooks => self.cmd_show_hooks(),
                AssistantEffect::InvokeSkill { name, args } => self.cmd_invoke_skill(&name, &args),
                AssistantEffect::OpenPermissionsManager => self.open_permissions_manager(),
                AssistantEffect::RevokeGrant { target_id } => self.cmd_revoke(&target_id),
                AssistantEffect::ShowAudit => self.cmd_show_audit(),
                AssistantEffect::ShowSettings => self.cmd_show_settings(),
                AssistantEffect::OpenModelPicker => self.open_model_picker(),
                AssistantEffect::SetSessionModel(tier) => self.set_session_model(tier),
                AssistantEffect::ListAgents => self.cmd_list_agents(),
                AssistantEffect::SwitchAgent(id) => self.cmd_switch_agent(&id),
                AssistantEffect::InspectAgent(id) => self.cmd_inspect_agent(&id),
                AssistantEffect::CreateAgent(id) => self.cmd_create_agent(&id),
                AssistantEffect::EditAgent(id) => self.cmd_edit_agent(&id),
                AssistantEffect::ShowEffort => self.cmd_show_effort(),
                AssistantEffect::SetSessionEffort(effort) => self.set_session_effort(effort),
                AssistantEffect::ListConversations => self.cmd_list_conversations(),
                AssistantEffect::ResumeConversation(selector) => {
                    self.cmd_resume_conversation(&selector)
                }
                AssistantEffect::ShowHistory => self.cmd_show_history(),
                AssistantEffect::RewindConversation(selector) => {
                    self.cmd_rewind_conversation(&selector)
                }
                AssistantEffect::CompactConversation => {
                    self.compact_pending = true;
                    log::info!(
                        "assistant[{}]: compaction queued; showing progress indicator",
                        self.model.conversation_id
                    );
                }
                AssistantEffect::ExportConversation => self.cmd_export_conversation(),
                AssistantEffect::PersistShowThoughts(show) => {
                    if let Err(e) = self.store.set_show_thoughts(show) {
                        log::error!("assistant: failed to persist show_thoughts={show}: {e}");
                    }
                }
                // The model already reset its streaming state; here we only
                // unblock worker threads parked on a permission reply so they
                // can finish and have their stale outcome dropped.
                AssistantEffect::CancelTurn => {
                    self.pending_skill = None;
                    self.unblock_pending_workers(
                        "cancelled: the conversation was cleared mid-turn",
                    );
                    log::info!("assistant: in-flight turn cancelled by conversation switch");
                }
            }
        }
    }

    /// Broker target id for an app-exposed tool.
    fn connector_target(tool: &str) -> String {
        format!("app.{tool}")
    }

    fn active_agent(&self) -> Option<&AgentDefinition> {
        self.agent_registry
            .active(&self.model.active_agent_id)
            .or_else(|| self.agent_registry.active("default"))
    }

    fn connector_actor(&self) -> (String, ActorScope) {
        self.active_agent()
            .map(|agent| {
                let scope = match agent.source {
                    AgentSource::BuiltIn => ActorScope::BuiltIn,
                    AgentSource::User => ActorScope::User,
                    AgentSource::Workspace => ActorScope::Workspace,
                };
                (agent.id.clone(), scope)
            })
            .unwrap_or_else(|| (ASSISTANT_ACTOR_ID.to_string(), ActorScope::BuiltIn))
    }

    fn active_posture(&self) -> crate::broker::PermissionPosture {
        let settings = self.settings.permissions.broker_posture();
        let Some(agent) = self.active_agent().map(|agent| &agent.posture) else {
            return settings;
        };
        let default_posture = if settings.default_posture == Decision::Deny
            || agent.default_posture == Decision::Deny
        {
            Decision::Deny
        } else if settings.default_posture == Decision::Ask
            || agent.default_posture == Decision::Ask
        {
            Decision::Ask
        } else {
            Decision::Allow
        };
        let mut allow = settings.allow;
        allow.extend(agent.allow.iter().cloned());
        let mut ask = settings.ask;
        ask.extend(agent.ask.iter().cloned());
        let mut deny = settings.deny;
        deny.extend(agent.deny.iter().cloned());
        crate::broker::PermissionPosture {
            default_posture,
            allow,
            ask,
            deny,
        }
    }

    /// Evaluate one app connector tool for the assistant actor.
    fn tool_decision(&self, tool: &AiTool) -> Decision {
        if self.settings.permissions.posture.value == settings::AssistantPermissionPosture::Plan
            && !tool.read_only
        {
            return Decision::Deny;
        }
        let (actor_id, _) = self.connector_actor();
        let req = PermissionRequest::new(
            ActorType::Agent,
            &actor_id,
            TargetType::AppConnector,
            &Self::connector_target(&tool.name),
            Some(&self.workspace_root),
        );
        let posture = self.active_posture();
        self.grant_store.evaluate(&req, Some(&posture))
    }

    fn host_tool_decision(&self, tool: &AiTool) -> Decision {
        if self.settings.permissions.posture.value == settings::AssistantPermissionPosture::Plan
            && !tool.read_only
        {
            return Decision::Deny;
        }
        let (actor_id, _) = self.connector_actor();
        let req = PermissionRequest::new(
            ActorType::Agent,
            &actor_id,
            TargetType::HostTool,
            &tool.name,
            Some(&self.workspace_root),
        );
        self.grant_store
            .evaluate(&req, Some(&self.active_posture()))
    }

    fn event_stream_decision(&self, app: &str, event: &str) -> Decision {
        let event_names = if event == "*" {
            Vec::new()
        } else {
            vec![event.to_string()]
        };
        let posture = self.active_posture();
        crate::host::event_subscriptions::evaluate_subscription(
            &self.grant_store,
            Some(&posture),
            &self.workspace_root,
            app,
            ActorType::Agent,
            ASSISTANT_ACTOR_ID,
            &event_names,
        )
    }

    /// Build the broker-gated dispatcher for one turn: denied tools are
    /// stripped (invisible and uninvocable), ask tools stay visible behind
    /// the permission-sheet hook, allowed tools pass through.
    fn gated_dispatcher(&self) -> ToolDispatcher {
        let mut dispatcher = ToolDispatcher::from_registry(
            0,
            format!("agent:{}", self.connector_actor().0),
            self.workspace_root.clone(),
        );
        let mut allowed = HashSet::new();
        let mut ask_tools = HashSet::new();
        let mut denied = 0usize;
        let mut ro_auto = 0usize;
        let declared_tools = self
            .active_agent()
            .map(|agent| agent.tools.as_slice())
            .unwrap_or(&[]);
        let settings_tools = &self.settings.tools.enabled.value;
        for tool in dispatcher.all_tools() {
            if (!declared_tools.is_empty() && !declared_tools.contains(&tool.name))
                || (!settings_tools.is_empty() && !settings_tools.contains(&tool.name))
            {
                denied += 1;
                continue;
            }
            match self.tool_decision(&tool) {
                // Read-only tools skip the ask prompt (no permission sheet) but
                // still respect explicit Deny grants — an admin deny wins.
                Decision::Allow | Decision::Ask if tool.read_only => {
                    log::info!(
                        "assistant: tool '{}' auto-allowed (read-only, no prompt needed)",
                        tool.name
                    );
                    allowed.insert(tool.name);
                    ro_auto += 1;
                }
                Decision::Allow => {
                    allowed.insert(tool.name);
                }
                Decision::Ask => {
                    ask_tools.insert(tool.name.clone());
                    allowed.insert(tool.name);
                }
                Decision::Deny => {
                    log::info!("assistant: tool '{}' withheld from turn (deny)", tool.name);
                    denied += 1;
                }
            }
        }
        log::info!(
            "assistant: connector discovery — {} tool(s) visible ({ro_auto} read-only auto, {} ask-gated, {denied} denied)",
            allowed.len(),
            ask_tools.len(),
        );
        dispatcher.retain_allowed(&allowed);
        let (actor_id, actor_scope) = self.connector_actor();
        let native_tools = Self::host_tools();
        let mut visible_host_tools = Vec::new();
        for tool in native_tools {
            if (!declared_tools.is_empty() && !declared_tools.contains(&tool.name))
                || (!settings_tools.is_empty() && !settings_tools.contains(&tool.name))
            {
                log::info!(
                    "assistant: host tool '{}' withheld by enabled-tool filter",
                    tool.name
                );
                continue;
            }
            match self.host_tool_decision(&tool) {
                Decision::Deny => {
                    log::info!("assistant: host tool '{}' withheld (deny)", tool.name)
                }
                Decision::Ask if !tool.read_only => {
                    ask_tools.insert(tool.name.clone());
                    visible_host_tools.push(tool);
                }
                Decision::Allow | Decision::Ask => visible_host_tools.push(tool),
            }
        }
        dispatcher.set_hooks(Arc::new(AssistantToolHooks {
            ask_tools,
            session_allowed: Mutex::new(HashSet::new()),
            flow_tx: self.flow_tx.clone(),
            actor_id,
            actor_scope,
        }));
        // Host event tools: always visible; subscribing is ask-gated
        // per-stream inside the pane's host-call handler, not here.
        let flow_tx = self.flow_tx.clone();
        let handler: HostToolHandler = Arc::new(move |name, input_json| {
            let (reply_tx, reply_rx) = std::sync::mpsc::sync_channel(1);
            let sent = flow_tx.send(ToolFlowEvent::HostCall {
                tool: name.to_string(),
                input_json: input_json.to_string(),
                reply: reply_tx,
            });
            if sent.is_err() {
                return ToolCallResult {
                    output_json: None,
                    error: Some("host_tool_failed: assistant pane closed".to_string()),
                };
            }
            reply_rx.recv().unwrap_or(ToolCallResult {
                output_json: None,
                error: Some("host_tool_failed: assistant pane closed".to_string()),
            })
        });
        visible_host_tools.extend(Self::host_event_tools());
        dispatcher.add_host_tools(visible_host_tools, handler);
        dispatcher
    }

    /// Declarations for the Assistant's host event tools.
    fn host_event_tools() -> Vec<AiTool> {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "app": {"type": "string", "description": "App id, e.g. 'calc'"},
                "event": {
                    "type": "string",
                    "description": "Declared event stream name, or '*' for all"
                },
            },
            "required": ["app", "event"],
        });
        vec![
            AiTool {
                name: HOST_TOOL_SUBSCRIBE.to_string(),
                description: "Subscribe to an app's event stream so its events \
                    appear in this conversation. May pause for the user's permission."
                    .to_string(),
                input_schema: schema.clone(),
                output_schema: serde_json::json!({"type": "object"}),
                timeout_ms: Some(120_000),
                read_only: false,
            },
            AiTool {
                name: HOST_TOOL_UNSUBSCRIBE.to_string(),
                description: "Stop receiving an app's event stream.".to_string(),
                input_schema: schema,
                output_schema: serde_json::json!({"type": "object"}),
                timeout_ms: Some(30_000),
                read_only: false,
            },
        ]
    }

    fn host_tools() -> Vec<AiTool> {
        let pane_id = serde_json::json!({
            "type": "object", "properties": {"pane_id": {"type": "integer"}},
            "required": ["pane_id"]
        });
        vec![
            AiTool {
                name: HOST_TOOL_PANES_LIST.into(),
                description: "List live Plexi panes and their context.".into(),
                input_schema: serde_json::json!({"type":"object"}),
                output_schema: serde_json::json!({"type":"object"}),
                timeout_ms: Some(30_000),
                read_only: true,
            },
            AiTool {
                name: HOST_TOOL_PANES_STATE.into(),
                description: "Read the semantic state of a live pane.".into(),
                input_schema: pane_id.clone(),
                output_schema: serde_json::json!({"type":"object"}),
                timeout_ms: Some(30_000),
                read_only: true,
            },
            AiTool {
                name: HOST_TOOL_PANES_OPEN.into(),
                description: "Open an app or terminal pane by native type id.".into(),
                input_schema: serde_json::json!({"type":"object","properties":{"type_id":{"type":"string"},"layout":{"type":"string"},"cwd":{"type":"string"},"args":{"type":"array","items":{"type":"string"}},"pane_id":{"type":"integer","description":"Open into this existing empty terminal pane instead of spawning a new one. The pane must be an idle terminal; occupied panes return an error."}},"required":["type_id"]}),
                output_schema: serde_json::json!({"type":"object"}),
                timeout_ms: Some(30_000),
                read_only: false,
            },
            AiTool {
                name: HOST_TOOL_PANES_FOCUS.into(),
                description: "Focus a live pane by id.".into(),
                input_schema: pane_id.clone(),
                output_schema: serde_json::json!({"type":"object"}),
                timeout_ms: Some(30_000),
                read_only: false,
            },
            AiTool {
                name: HOST_TOOL_PANES_CLOSE.into(),
                description: "Close a live pane by id.".into(),
                input_schema: pane_id,
                output_schema: serde_json::json!({"type":"object"}),
                timeout_ms: Some(30_000),
                read_only: false,
            },
            AiTool {
                name: HOST_TOOL_APPS_OPEN.into(),
                description: "Open an installed Plexi app in a pane.".into(),
                input_schema: serde_json::json!({"type":"object","properties":{"app":{"type":"string"},"layout":{"type":"string"},"args":{"type":"array","items":{"type":"string"}},"pane_id":{"type":"integer","description":"Open into this existing empty terminal pane instead of spawning a new one. The pane must be an idle terminal; occupied panes return an error."}},"required":["app"]}),
                output_schema: serde_json::json!({"type":"object"}),
                timeout_ms: Some(30_000),
                read_only: false,
            },
            AiTool {
                name: HOST_TOOL_TERMINALS_OPEN.into(),
                description: "Open a terminal pane, optionally at a cwd.".into(),
                input_schema: serde_json::json!({"type":"object","properties":{"layout":{"type":"string"},"cwd":{"type":"string"}}}),
                output_schema: serde_json::json!({"type":"object"}),
                timeout_ms: Some(30_000),
                read_only: false,
            },
            AiTool {
                name: HOST_TOOL_TERMINALS_RUN.into(),
                description: "Run a command in a terminal pane already opened or focused by the Assistant. The terminal remains human-observed; echo=true submits the command with Enter.".into(),
                input_schema: serde_json::json!({"type":"object","properties":{"terminal_pane_id":{"type":"integer"},"command":{"type":"string"},"echo":{"type":"boolean","description":"Must be true; the command is visibly submitted to the terminal."}},"required":["terminal_pane_id","command","echo"]}),
                output_schema: serde_json::json!({"type":"object"}),
                timeout_ms: Some(30_000),
                read_only: false,
            },
            AiTool {
                name: HOST_TOOL_TERMINALS_READ.into(),
                description: "Read the last lines of a terminal pane's screen. \
                    Call this after host.terminals.run to see the command's \
                    output before deciding your next step."
                    .into(),
                input_schema: serde_json::json!({"type":"object","properties":{"terminal_pane_id":{"type":"integer"},"lines":{"type":"integer","description":"How many trailing lines to read (default 40)."}},"required":["terminal_pane_id"]}),
                output_schema: serde_json::json!({"type":"object"}),
                timeout_ms: Some(30_000),
                read_only: true,
            },
        ]
    }

    /// Persist the active conversation id and the full transcript.
    fn session_write(&mut self) {
        if let Err(e) = self.store.set_active_conversation(
            &self.model.conversation_id,
            self.model.session_name.as_deref(),
            &self.model.active_agent_id,
            self.model.effort_override,
        ) {
            log::error!("assistant: failed to persist active conversation: {e}");
        }
        if let Err(e) = self
            .store
            .write_turns(&self.model.conversation_id, &self.model.turns)
        {
            log::error!(
                "assistant[{}]: failed to persist transcript: {e}",
                self.model.conversation_id
            );
        }
        if let Err(e) = self
            .store
            .set_turn_in_flight(self.model.streaming.in_flight)
        {
            log::error!(
                "assistant[{}]: failed to persist in-flight state: {e}",
                self.model.conversation_id
            );
        }
    }

    /// Conversation history for the broker: user/assistant turns plus
    /// delivered app events (as user-role context lines).
    fn history_messages(&self) -> Vec<AiMessage> {
        self.model
            .turns
            .iter()
            .filter_map(|turn| match turn.role {
                TurnRole::User => Some(AiMessage {
                    role: "user".to_string(),
                    content: turn.text.clone(),
                }),
                TurnRole::Event => Some(AiMessage {
                    role: "user".to_string(),
                    content: format!("App event delivered to you: {}", turn.text),
                }),
                TurnRole::Assistant => Some(AiMessage {
                    role: "assistant".to_string(),
                    content: turn.text.clone(),
                }),
                TurnRole::Tool | TurnRole::Error => None,
            })
            .collect()
    }

    /// Execute one `host.events.*` call on the UI thread. Replies on the
    /// worker's channel — except an ask-gated subscribe, which parks the
    /// reply in `pending_subscribe` until the permission sheet resolves.
    fn handle_host_call(
        &mut self,
        tool: &str,
        input_json: &str,
        reply: SyncSender<ToolCallResult>,
    ) {
        if !matches!(tool, HOST_TOOL_SUBSCRIBE | HOST_TOOL_UNSUBSCRIBE) {
            log::info!("assistant: queueing native host tool '{tool}'");
            if tool == HOST_TOOL_TERMINALS_RUN {
                self.audit.append(&AuditEvent::now(
                    "terminal_command",
                    HOST_TOOL_TERMINALS_RUN,
                    "requested",
                    &summarize_input(input_json),
                ));
            }
            self.pending_commands.push(AppCommand::AssistantHostTool {
                name: tool.to_string(),
                input_json: input_json.to_string(),
                origin_pane_id: 0,
                origin_context_id: 0,
                reply,
            });
            return;
        }
        let err = |msg: String| ToolCallResult {
            output_json: None,
            error: Some(msg),
        };
        let ok = |msg: String| ToolCallResult {
            output_json: Some(serde_json::json!({"ok": true, "detail": msg}).to_string()),
            error: None,
        };
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(input_json);
        let (app, event) = match &parsed {
            Ok(v) => (
                v.get("app")
                    .and_then(|a| a.as_str())
                    .unwrap_or("")
                    .to_string(),
                v.get("event")
                    .and_then(|e| e.as_str())
                    .unwrap_or("")
                    .to_string(),
            ),
            Err(e) => {
                let _ = reply.send(err(format!("invalid_input: {e}")));
                return;
            }
        };
        if app.trim().is_empty() || event.trim().is_empty() {
            let _ = reply.send(err(
                "invalid_input: 'app' and 'event' must be non-empty".to_string()
            ));
            return;
        }
        let target = format!("{app}::{event}");
        match tool {
            HOST_TOOL_SUBSCRIBE => {
                if self.live_subs.iter().any(|(t, _)| *t == target) {
                    let _ = reply.send(ok(format!("already subscribed to {target}")));
                    return;
                }
                match self.event_stream_decision(&app, &event) {
                    Decision::Allow => {
                        self.subscribe_stream(&app, &event);
                        self.audit
                            .append(&AuditEvent::now("subscribe", &target, "ok", "granted"));
                        let _ = reply.send(ok(format!("subscribed to {target}")));
                    }
                    Decision::Deny => {
                        log::info!("assistant: subscribe to '{target}' denied by grant");
                        self.audit
                            .append(&AuditEvent::now("subscribe", &target, "deny", "grant"));
                        let _ = reply.send(err(format!(
                            "permission_denied: subscription to {target} is denied"
                        )));
                    }
                    Decision::Ask => {
                        log::info!("assistant: permission sheet shown for stream '{target}'");
                        self.model
                            .permission_requested(HOST_TOOL_SUBSCRIBE, &target);
                        self.pending_subscribe = Some(PendingSubscribe {
                            app,
                            event,
                            target,
                            reply,
                        });
                    }
                }
            }
            HOST_TOOL_UNSUBSCRIBE => {
                let removed = self.unsubscribe_stream(&target);
                self.audit.append(&AuditEvent::now(
                    "unsubscribe",
                    &target,
                    if removed > 0 { "ok" } else { "noop" },
                    &format!("{removed} subscription(s) removed"),
                ));
                if removed > 0 {
                    let _ = reply.send(ok(format!("unsubscribed from {target}")));
                } else {
                    let _ = reply.send(err(format!("not_subscribed: no subscription to {target}")));
                }
            }
            other => {
                let _ = reply.send(err(format!("host_tool_unknown: {other:?}")));
            }
        }
    }

    /// Run one model turn on a worker thread (the broker blocks on network).
    fn start_turn(&mut self, conversation_id: String, prompt: String) {
        let (delta_tx, delta_rx) = std::sync::mpsc::channel();
        self.delta_rx = Some(delta_rx);
        // Fresh cancel token per turn — a clone goes to the worker/broker, the
        // original stays here so ESC and event-preempt can trip this turn only.
        let cancel = CancelToken::new();
        self.turn_cancel = cancel.clone();
        let dispatcher = self.gated_dispatcher();
        let Some(agent) = self.active_agent().cloned() else {
            log::error!("assistant: no active or default agent available for dispatch");
            let effects = self.model.finish_turn(
                &conversation_id,
                Err("Assistant agent registry has no default agent.".to_string()),
            );
            self.execute_effects(effects);
            return;
        };
        let tier = self.session_overrides.model_tier.unwrap_or_else(|| {
            if self.settings.model.tier.source.scope == settings::SettingsScope::Default {
                agent.default_tier
            } else {
                self.settings.model.tier.value
            }
        });
        let concrete_model = agent.model_routes.for_tier(tier).cloned();
        let effort = self.model.effort_override.or(agent.effort);
        let selected_skill = self.pending_skill.take().or_else(|| {
            self.skill_registry
                .matching_enabled(&prompt, &agent.skills)
                .cloned()
        });
        let mut system = agent.prompt;
        system.push_str("\n\nCompaction status: ");
        system.push_str(&self.model.compaction_status());
        if let Some(skill) = selected_skill {
            log::info!(
                "assistant[{conversation_id}]: loading {} skill '{}' from {}",
                skill.source.label(),
                skill.name,
                skill.path.display()
            );
            system.push_str("\n\nFollow this loaded skill for the current turn:\n");
            system.push_str(&skill.instructions);
        }
        let request = AiBrokerRequest {
            app_id: "assistant".to_string(),
            model_tier: tier,
            concrete_model,
            reasoning_effort: effort,
            system,
            messages: self.history_messages(),
            tools: Vec::new(),
            workspace_root: Some(self.workspace_root.clone()),
            open_panes: crate::plexi_ai::broker::get_pane_snapshot(),
            tool_dispatcher: Some(Arc::new(dispatcher)),
            cancel,
        };
        log::info!(
            "assistant[{conversation_id}]: dispatching agent={} tier={} route={} effort={} messages={} tools={}",
            agent.id,
            settings::model_tier_name(request.model_tier),
            request
                .concrete_model
                .as_ref()
                .map(|route| format!("{}/{}", route.provider, route.model))
                .unwrap_or_else(|| "tier-default".to_string()),
            request
                .reasoning_effort
                .map(ReasoningEffort::label)
                .unwrap_or("auto"),
            request.messages.len(),
            request
                .tool_dispatcher
                .as_ref()
                .map(|dispatcher| dispatcher.all_tools().len())
                .unwrap_or(0)
        );
        let broker = Arc::clone(&self.broker);
        let outcome_tx = self.outcome_tx.clone();
        let spawn = std::thread::Builder::new()
            .name("assistant-turn".to_string())
            .spawn(move || {
                let resp = broker.dispatch(request, &mut |delta| {
                    let owned = match delta {
                        TurnDelta::Text(chunk) => StreamDelta::Answer(chunk.to_string()),
                        TurnDelta::Reasoning(chunk) => StreamDelta::Reasoning(chunk.to_string()),
                    };
                    let _ = delta_tx.send(owned);
                });
                let _ = outcome_tx.send(TurnOutcome {
                    conversation_id,
                    text: resp.content,
                    error: resp.error,
                });
            });
        if let Err(e) = spawn {
            log::error!("assistant: failed to spawn turn thread: {e}");
            let effects = self.model.finish_turn(
                &self.model.conversation_id.clone(),
                Err(format!("failed to spawn turn thread: {e}")),
            );
            self.execute_effects(effects);
        }
    }

    /// Per-frame pump: tool-flow events, live stream deltas, finished turns,
    /// and queued app-event deliveries.
    fn pump_turn_io(&mut self) {
        self.pump_deliveries();
        let flow_events: Vec<ToolFlowEvent> = self.flow_rx.try_iter().collect();
        for event in flow_events {
            match event {
                ToolFlowEvent::Started { tool } => {
                    log::info!("assistant: tool '{tool}' running");
                    self.model.tool_call_started(&tool);
                }
                ToolFlowEvent::Finished { tool, error } => {
                    log::info!(
                        "assistant: tool '{tool}' finished ({})",
                        if error.is_none() { "ok" } else { "error" }
                    );
                    let audit_target = if tool.starts_with("host.") {
                        tool.clone()
                    } else {
                        Self::connector_target(&tool)
                    };
                    self.audit.append(&AuditEvent::now(
                        "tool_call",
                        &audit_target,
                        if error.is_none() { "ok" } else { "error" },
                        error.as_deref().unwrap_or(""),
                    ));
                    let effects = self.model.tool_call_finished(&tool, error);
                    self.execute_effects(effects);
                }
                ToolFlowEvent::Ask {
                    tool,
                    input_json,
                    actor_id,
                    actor_scope,
                    reply,
                } => {
                    log::info!("assistant: permission sheet shown for '{tool}'");
                    self.model
                        .permission_requested(&tool, &summarize_input(&input_json));
                    self.pending_reply = Some(reply);
                    self.pending_connector_actor = Some((actor_id, actor_scope));
                }
                ToolFlowEvent::HostCall {
                    tool,
                    input_json,
                    reply,
                } => self.handle_host_call(&tool, &input_json, reply),
            }
        }
        if let Some(rx) = &self.delta_rx {
            let deltas: Vec<StreamDelta> = rx.try_iter().collect();
            for delta in deltas {
                match delta {
                    StreamDelta::Answer(chunk) => self.model.apply_answer_delta(&chunk),
                    StreamDelta::Reasoning(chunk) => self.model.apply_reasoning_delta(&chunk),
                }
            }
        }
        while let Ok(outcome) = self.outcome_rx.try_recv() {
            self.delta_rx = None;
            self.pending_reply = None;
            self.pending_connector_actor = None;
            let result = match (outcome.text, outcome.error) {
                (Some(text), _) => Ok(text),
                (None, Some(error)) => Err(error),
                (None, None) => Err("broker returned neither content nor error".to_string()),
            };
            let effects = self.model.finish_turn(&outcome.conversation_id, result);
            self.execute_effects(effects);
        }
        // Events and user messages that arrived mid-turn trigger one
        // follow-up turn that folds them in (they are already in the
        // transcript history).
        if !self.model.streaming.in_flight
            && self.pending_subscribe.is_none()
            && (!self.queued_event_lines.is_empty() || self.model.queued_user_turns > 0)
        {
            let lines = std::mem::take(&mut self.queued_event_lines);
            let users = std::mem::take(&mut self.model.queued_user_turns);
            log::info!(
                "assistant: dispatching follow-up turn ({} queued event line(s), {} queued user message(s))",
                lines.len(),
                users
            );
            self.start_event_turn(lines.len() + users);
        }
    }

    /// Drain queued event deliveries: append visible event rows, then
    /// auto-start a turn for non-self-caused deliveries (queue them when a
    /// turn is already in flight).
    fn pump_deliveries(&mut self) {
        if self.live_subs.is_empty() {
            return;
        }
        if self.timeline.lock().unwrap().pending_delivery_count() == 0 {
            return;
        }
        let deliveries = self
            .timeline
            .lock()
            .unwrap()
            .take_deliveries_for(ActorType::Agent, ASSISTANT_ACTOR_ID);
        if deliveries.is_empty() {
            return;
        }
        // Same self-caused rule as `AgentHost::handle_deliveries`: never
        // trigger on events the assistant emitted as the actor or caused via
        // one of its tool calls — only record them.
        let self_id = format!("agent:{ASSISTANT_ACTOR_ID}");
        let mut trigger_lines = Vec::new();
        for d in deliveries {
            let self_caused =
                d.actor_id == self_id || d.caused_by.as_deref() == Some(self_id.as_str());
            let line = format!(
                "⚡ {}: {} — {}{}",
                d.app_id,
                d.event,
                d.summary.as_deref().unwrap_or("(no summary)"),
                d.payload
                    .as_ref()
                    .map(|p| format!(" payload={p}"))
                    .unwrap_or_default(),
            );
            log::info!(
                "assistant: delivery {} of '{}' from '{}'{}",
                d.delivery_id,
                d.event,
                d.app_id,
                if self_caused {
                    " is self-caused — recorded, no turn"
                } else {
                    ""
                }
            );
            self.model
                .turns
                .push(model::Turn::now(TurnRole::Event, line.clone()));
            if !self_caused {
                trigger_lines.push(line);
            }
        }
        self.session_write();
        if trigger_lines.is_empty() {
            return;
        }
        if self.model.streaming.in_flight {
            // Preempt: the world changed under the in-flight turn (e.g. the
            // game board moved). Queue the new lines and trip the cancel token
            // so the current turn ends fast and the pump folds these into an
            // immediate follow-up that reacts to the latest state — instead of
            // finishing a now-stale comment (the 6-26s queue-behind lag).
            log::info!(
                "assistant: {} event line(s) queued, preempting in-flight turn",
                trigger_lines.len()
            );
            self.queued_event_lines.extend(trigger_lines);
            self.turn_cancel.cancel();
        } else {
            self.start_event_turn(trigger_lines.len());
        }
    }

    /// Auto-dispatch a turn in response to delivered events. The event rows
    /// are already in the transcript, so the turn's history ends with them.
    fn start_event_turn(&mut self, line_count: usize) {
        log::info!("assistant: auto-starting turn for {line_count} delivered event(s)");
        self.audit.append(&AuditEvent::now(
            "auto_turn",
            "app_events",
            "ok",
            &format!("{line_count} event line(s)"),
        ));
        self.model.streaming = model::StreamingState {
            in_flight: true,
            ..Default::default()
        };
        // Anchor this turn's rows after the queued messages/events that
        // triggered it, same as a `submit()`-dispatched turn.
        self.model.turn_anchor = Some(self.model.turns.len());
        let conversation_id = self.model.conversation_id.clone();
        self.start_turn(conversation_id, String::new());
    }

    /// User pressed ESC during an in-flight turn. Stop generating, and if the
    /// composer holds a draft, queue it so the pump folds it into an immediate
    /// follow-up turn (stop-and-send). With an empty composer this is a plain
    /// stop. Default typing-while-streaming still queues silently (see
    /// `AssistantModel::submit`); ESC is the explicit interrupt.
    fn interrupt_in_flight_turn(&mut self) {
        if !self.model.streaming.in_flight {
            return;
        }
        // A pending draft becomes a queued user turn first, so the folded
        // follow-up dispatched after cancel includes it.
        if !self.model.composer.trim().is_empty() {
            let effects = self.model.submit();
            self.execute_effects(effects);
        }
        self.turn_cancel.cancel();
        // Unblock a worker parked on a permission sheet so it observes the
        // cancel at the next tool-loop boundary instead of hanging.
        self.unblock_pending_workers("cancelled: interrupted by user (ESC)");
        log::info!(
            "assistant: ESC interrupt — cancelling in-flight turn ({} queued user message(s), {} queued event line(s))",
            self.model.queued_user_turns,
            self.queued_event_lines.len()
        );
    }

    /// Unblock any worker thread parked on a permission sheet or pending
    /// subscribe so it observes the cancel token at the next tool-loop
    /// boundary instead of hanging. Shared by conversation-switch cancel and
    /// ESC interrupt. `reason` is surfaced to the blocked subscribe caller.
    fn unblock_pending_workers(&mut self, reason: &str) {
        if let Some(tx) = self.pending_reply.take() {
            let _ = tx.send(PermissionReply::Deny);
        }
        self.pending_connector_actor = None;
        if let Some(pending) = self.pending_subscribe.take() {
            let _ = pending.reply.send(ToolCallResult {
                output_json: None,
                error: Some(reason.to_string()),
            });
        }
    }

    /// Apply the user's permission-sheet decision: record the grant per its
    /// duration, audit it, and unblock the worker thread.
    fn resolve_permission(&mut self, choice: PermissionChoice) {
        if self.pending_subscribe.is_some() {
            self.resolve_subscribe_permission(choice);
            return;
        }
        let Some(pending) = self.model.pending_permission.clone() else {
            return;
        };
        let is_host_tool = pending.tool.starts_with("host.");
        let target_type = if is_host_tool {
            TargetType::HostTool
        } else {
            TargetType::AppConnector
        };
        let target = if is_host_tool {
            pending.tool.clone()
        } else {
            Self::connector_target(&pending.tool)
        };
        let (decision_str, reply) = match choice {
            PermissionChoice::Deny => ("deny", PermissionReply::Deny),
            PermissionChoice::AllowOnce => {
                ("allow_once", PermissionReply::Allow { remember: false })
            }
            PermissionChoice::AllowSession => {
                self.record_connector_grant(
                    target_type,
                    &target,
                    GrantDuration::Session,
                    GrantSource::Session,
                );
                ("allow_session", PermissionReply::Allow { remember: true })
            }
            PermissionChoice::AllowAlways => {
                self.record_connector_grant(
                    target_type,
                    &target,
                    GrantDuration::Always,
                    GrantSource::User,
                );
                self.grant_store.save();
                ("allow_always", PermissionReply::Allow { remember: true })
            }
        };
        log::info!(
            "assistant: permission sheet decision for '{}' = {decision_str}",
            pending.tool
        );
        self.audit.append(&AuditEvent::now(
            "permission_decision",
            &target,
            decision_str,
            &pending.input_summary,
        ));
        match self.pending_reply.take() {
            Some(tx) => {
                let _ = tx.send(reply);
            }
            None => log::warn!(
                "assistant: permission decision for '{}' had no waiting worker",
                pending.tool
            ),
        }
        self.pending_connector_actor = None;
        let effects = self.model.permission_resolved(choice);
        self.execute_effects(effects);
    }

    /// Apply the user's permission-sheet decision for an ask-gated
    /// `host.events.subscribe`: record the grant per its duration, create
    /// the live subscription, audit it, and unblock the worker thread.
    fn resolve_subscribe_permission(&mut self, choice: PermissionChoice) {
        let Some(pending) = self.pending_subscribe.take() else {
            return;
        };
        let PendingSubscribe {
            app,
            event,
            target,
            reply,
        } = pending;
        let decision_str = match choice {
            PermissionChoice::Deny => "deny",
            PermissionChoice::AllowOnce => "allow_once",
            PermissionChoice::AllowSession => {
                self.record_assistant_grant(
                    TargetType::AppEventStream,
                    &target,
                    GrantDuration::Session,
                    GrantSource::Session,
                );
                "allow_session"
            }
            PermissionChoice::AllowAlways => {
                self.record_assistant_grant(
                    TargetType::AppEventStream,
                    &target,
                    GrantDuration::Always,
                    GrantSource::User,
                );
                self.grant_store.save();
                "allow_always"
            }
        };
        log::info!("assistant: permission sheet decision for stream '{target}' = {decision_str}");
        self.audit.append(&AuditEvent::now(
            "permission_decision",
            &target,
            decision_str,
            "event stream subscription",
        ));
        let result = if choice == PermissionChoice::Deny {
            ToolCallResult {
                output_json: None,
                error: Some(format!(
                    "permission_denied: the user denied the subscription to {target}"
                )),
            }
        } else {
            self.subscribe_stream(&app, &event);
            self.audit
                .append(&AuditEvent::now("subscribe", &target, "ok", decision_str));
            ToolCallResult {
                output_json: Some(
                    serde_json::json!({"ok": true, "detail": format!("subscribed to {target}")})
                        .to_string(),
                ),
                error: None,
            }
        };
        let _ = reply.send(result);
        let effects = self.model.permission_resolved(choice);
        self.execute_effects(effects);
    }

    fn record_connector_grant(
        &mut self,
        target_type: TargetType,
        target: &str,
        duration: GrantDuration,
        source: GrantSource,
    ) {
        let (actor_id, actor_scope) = self
            .pending_connector_actor
            .clone()
            .unwrap_or_else(|| self.connector_actor());
        self.record_grant(
            &actor_id,
            actor_scope,
            target_type,
            target,
            duration,
            source,
        );
    }

    /// Record an Allow grant for the shared Assistant event actor on `target`.
    fn record_assistant_grant(
        &mut self,
        target_type: TargetType,
        target: &str,
        duration: GrantDuration,
        source: GrantSource,
    ) {
        self.record_grant(
            ASSISTANT_ACTOR_ID,
            ActorScope::BuiltIn,
            target_type,
            target,
            duration,
            source,
        );
    }

    fn record_grant(
        &mut self,
        actor_id: &str,
        actor_scope: ActorScope,
        target_type: TargetType,
        target: &str,
        duration: GrantDuration,
        source: GrantSource,
    ) {
        self.record_grant_with_decision(
            actor_id,
            actor_scope,
            target_type,
            target,
            duration,
            source,
            Decision::Allow,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn record_grant_with_decision(
        &mut self,
        actor_id: &str,
        actor_scope: ActorScope,
        target_type: TargetType,
        target: &str,
        duration: GrantDuration,
        source: GrantSource,
        decision: Decision,
    ) {
        // Canonicalize to match `PermissionRequest`'s workspace normalization
        // (macOS tempdirs resolve `/var` → `/private/var`).
        let workspace_root = self
            .workspace_root
            .canonicalize()
            .unwrap_or_else(|_| self.workspace_root.clone());
        self.grant_store.record(GrantRecord {
            actor_type: ActorType::Agent,
            actor_id: actor_id.to_string(),
            actor_scope,
            workspace_root: Some(workspace_root),
            target_type,
            target_id: target.to_string(),
            resource_scope: ResourceScope::Workspace,
            resource_id: None,
            decision,
            duration,
            source,
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0),
            expires_at: None,
        });
    }

    fn reload_settings(&mut self) {
        let report = self.settings_loader.load(&self.session_overrides);
        self.settings = report.settings;
        self.settings_errors = report.errors;
    }

    fn reload_agents(&mut self) {
        self.agent_registry = AgentRegistry::load(&self.profile_dir, &self.workspace_root);
        if self
            .agent_registry
            .active(&self.model.active_agent_id)
            .is_none()
        {
            self.model.active_agent_id = "default".to_string();
        }
    }

    fn cmd_list_agents(&mut self) {
        self.reload_agents();
        let lines = self
            .agent_registry
            .agents()
            .map(|agent| {
                let marker = if agent.id == self.model.active_agent_id {
                    " (active)"
                } else {
                    ""
                };
                let shadowed = self.agent_registry.shadowed(&agent.id);
                let shadow_note = if shadowed.is_empty() {
                    String::new()
                } else {
                    format!(
                        "; {} shadowed: {}",
                        shadowed.len(),
                        shadowed
                            .iter()
                            .map(|entry| entry.source.label())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                };
                format!(
                    "- `{}`: {} [{}]{}{}",
                    agent.id,
                    agent.display_name,
                    agent.source.label(),
                    marker,
                    shadow_note
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let effects = self
            .model
            .push_info(format!("**Assistant agents**\n\n{lines}"));
        self.execute_effects(effects);
    }

    fn cmd_switch_agent(&mut self, id: &str) {
        self.reload_agents();
        let Some(agent) = self.agent_registry.active(id) else {
            let effects = self
                .model
                .push_error(format!("Unknown agent `{id}`. Run /agent to list agents."));
            self.execute_effects(effects);
            return;
        };
        self.model.active_agent_id = agent.id.clone();
        self.model.effort_override = None;
        log::info!(
            "assistant: switched active agent to '{}' ({})",
            agent.id,
            agent.source.label()
        );
        let effects = self.model.push_info(format!(
            "Active agent: `{}` ({}).",
            agent.id, agent.display_name
        ));
        self.execute_effects(effects);
    }

    fn cmd_inspect_agent(&mut self, id: &str) {
        self.reload_agents();
        let Some(agent) = self.agent_registry.active(id) else {
            let effects = self.model.push_error(format!("Unknown agent `{id}`."));
            self.execute_effects(effects);
            return;
        };
        let shadowed = self
            .agent_registry
            .shadowed(id)
            .iter()
            .map(|entry| entry.source.label())
            .collect::<Vec<_>>()
            .join(", ");
        let routes = [ModelTier::Low, ModelTier::Medium, ModelTier::High]
            .into_iter()
            .filter_map(|tier| {
                agent.model_routes.for_tier(tier).map(|route| {
                    format!(
                        "{}={}/{}",
                        settings::model_tier_name(tier),
                        route.provider,
                        route.model
                    )
                })
            })
            .collect::<Vec<_>>()
            .join(", ");
        let shadow_details = self
            .agent_registry
            .shadowed(id)
            .iter()
            .map(|entry| {
                format!(
                    "- {} [{}], tier={}, description={}, path={}, tools={}, skills={}, hooks={}",
                    entry.display_name,
                    entry.source.label(),
                    settings::model_tier_name(entry.default_tier),
                    if entry.description.is_empty() {
                        "none"
                    } else {
                        &entry.description
                    },
                    entry
                        .path
                        .as_ref()
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| "compiled".to_string()),
                    format_setting_ids(&entry.tools),
                    format_setting_ids(&entry.skills),
                    format_setting_ids(&entry.hooks)
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let text = format!(
            "**Agent `{}`**\n\n- Name: {}\n- Source: {}\n- Description: {}\n- Default tier: {}\n- Routes: {}\n- Effort: {}\n- Tools: {}\n- Skills: {}\n- Hooks: {}\n- Shadowed sources: {}",
            agent.id,
            agent.display_name,
            agent.source.label(),
            if agent.description.is_empty() { "none" } else { &agent.description },
            settings::model_tier_name(agent.default_tier),
            if routes.is_empty() { "tier defaults" } else { &routes },
            agent.effort.map(ReasoningEffort::label).unwrap_or("auto"),
            format_setting_ids(&agent.tools),
            format_setting_ids(&agent.skills),
            format_setting_ids(&agent.hooks),
            if shadowed.is_empty() { "none" } else { &shadowed }
        );
        let text = if shadow_details.is_empty() {
            text
        } else {
            format!("{text}\n\n**Shadowed definitions**\n\n{shadow_details}")
        };
        let effects = self.model.push_info(text);
        self.execute_effects(effects);
    }

    fn valid_agent_id(id: &str) -> bool {
        !id.is_empty()
            && id
                .chars()
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-' || ch == '_')
    }

    fn cmd_create_agent(&mut self, id: &str) {
        self.reload_agents();
        if !Self::valid_agent_id(id) {
            let effects = self.model.push_error(
                "Agent ids use lowercase letters, digits, hyphens, and underscores only."
                    .to_string(),
            );
            self.execute_effects(effects);
            return;
        }
        if self.agent_registry.active(id).is_some() {
            let effects = self
                .model
                .push_error(format!("Agent `{id}` already exists."));
            self.execute_effects(effects);
            return;
        }
        let parent = self.profile_dir.join("agents");
        let dir = parent.join(id);
        let mut created_dir = false;
        let result = (|| {
            std::fs::create_dir_all(&parent)?;
            std::fs::create_dir(&dir)?;
            created_dir = true;
            std::fs::write(
                dir.join("AGENT.md"),
                format!("You are {id}, a Plexi Assistant agent.\n"),
            )?;
            std::fs::write(
                dir.join("settings.toml"),
                format!(
                    "[agent]\nid = \"{id}\"\ndisplay_name = \"{id}\"\ndefault_tier = \"medium\"\n\n[permissions]\ndefault_posture = \"ask\"\n"
                ),
            )
        })();
        if let Err(error) = result {
            if created_dir {
                if let Err(cleanup_error) = std::fs::remove_dir_all(&dir) {
                    log::error!(
                        "assistant: failed to clean partial agent '{}' at {}: {cleanup_error}",
                        id,
                        dir.display()
                    );
                }
            }
            log::error!(
                "assistant: failed to create agent '{}' at {}: {error}",
                id,
                dir.display()
            );
            let effects = self
                .model
                .push_error(format!("Failed to create agent `{id}`: {error}"));
            self.execute_effects(effects);
            return;
        }
        self.reload_agents();
        log::info!(
            "assistant: created user agent '{}' at {}",
            id,
            dir.display()
        );
        let effects = self
            .model
            .push_info(format!("Created agent `{id}` at `{}`.", dir.display()));
        self.execute_effects(effects);
    }

    fn cmd_edit_agent(&mut self, id: &str) {
        self.reload_agents();
        let Some(agent) = self.agent_registry.active(id) else {
            let effects = self.model.push_error(format!("Unknown agent `{id}`."));
            self.execute_effects(effects);
            return;
        };
        if agent.source == AgentSource::BuiltIn {
            let effects = self.model.push_error(
                "Built-in agents cannot be edited. Create a user agent instead.".to_string(),
            );
            self.execute_effects(effects);
            return;
        }
        let path = agent
            .path
            .clone()
            .unwrap_or_else(|| self.profile_dir.join("agents").join(id));
        log::info!("assistant: edit agent '{}' at {}", id, path.display());
        let effects = self
            .model
            .push_info(format!("Edit agent `{id}` at `{}`.", path.display()));
        self.execute_effects(effects);
    }

    fn cmd_show_effort(&mut self) {
        let agent_effort = self.active_agent().and_then(|agent| agent.effort);
        let effective = self.model.effort_override.or(agent_effort);
        let source = if self.model.effort_override.is_some() {
            "session"
        } else if agent_effort.is_some() {
            "agent"
        } else {
            "provider default"
        };
        let effects = self.model.push_info(format!(
            "Reasoning effort: `{}` ({source}).",
            effective.map(ReasoningEffort::label).unwrap_or("auto")
        ));
        self.execute_effects(effects);
    }

    fn set_session_effort(&mut self, effort: Option<ReasoningEffort>) {
        self.model.effort_override = effort;
        log::info!(
            "assistant: session reasoning effort set to {}",
            effort.map(ReasoningEffort::label).unwrap_or("auto")
        );
        let effects = self.model.push_info(format!(
            "Reasoning effort set to `{}` for this session.",
            effort.map(ReasoningEffort::label).unwrap_or("auto")
        ));
        self.execute_effects(effects);
    }

    fn cmd_list_conversations(&mut self) {
        match self.store.list_conversations() {
            Ok(items) => {
                log::info!("assistant: /resume listed {} conversation(s)", items.len());
                let body = if items.is_empty() {
                    "No saved conversations in this workspace.".to_string()
                } else {
                    let rows = items
                        .iter()
                        .enumerate()
                        .map(|(index, item)| {
                            format!(
                                "{}. {} `{}`: {} turn(s){}",
                                index + 1,
                                item.title,
                                item.id,
                                item.turn_count,
                                if item.active { " (active)" } else { "" }
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    format!("**Workspace conversations**\n\n{rows}\n\nUse `/resume <index-or-id>`.")
                };
                let effects = self.model.push_info(body);
                self.execute_effects(effects);
            }
            Err(e) => {
                log::error!("assistant: /resume failed to list conversations: {e}");
                let effects = self
                    .model
                    .push_error(format!("Could not list conversations: {e}"));
                self.execute_effects(effects);
            }
        }
    }

    fn resolve_conversation(&self, selector: &str) -> Result<String, String> {
        let items = self.store.list_conversations()?;
        if let Ok(index) = selector.parse::<usize>() {
            return items
                .get(index.saturating_sub(1))
                .map(|item| item.id.clone())
                .ok_or_else(|| format!("No conversation at index {index}."));
        }
        let matches = items
            .iter()
            .filter(|item| item.id == selector || item.id.starts_with(selector))
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [item] => Ok(item.id.clone()),
            [] => Err(format!("No conversation matches `{selector}`.")),
            _ => Err(format!("Conversation selector `{selector}` is ambiguous.")),
        }
    }

    fn cmd_resume_conversation(&mut self, selector: &str) {
        let id = match self.resolve_conversation(selector) {
            Ok(id) => id,
            Err(e) => {
                let effects = self.model.push_error(e);
                self.execute_effects(effects);
                return;
            }
        };
        let turns = self.store.load_turns(&id);
        let session_name = match self.store.load_history(&id) {
            Ok(history) => history.name,
            Err(e) => {
                log::error!("assistant[{id}]: /resume failed to read metadata: {e}");
                let effects = self
                    .model
                    .push_error(format!("Could not resume conversation metadata: {e}"));
                self.execute_effects(effects);
                return;
            }
        };
        let turn_count = turns.len();
        self.pending_skill = None;
        let cancel = self.model.switch_conversation(id.clone(), turns);
        self.model.session_name = session_name;
        self.execute_effects(cancel);
        log::info!("assistant[{id}]: resumed conversation ({turn_count} turn(s))");
        let effects = self.model.push_info(format!(
            "Resumed `{id}` with {turn_count} turn(s). Agent, effort, and thought settings were preserved."
        ));
        self.execute_effects(effects);
    }

    fn cmd_show_history(&mut self) {
        let id = self.model.conversation_id.clone();
        match self.store.load_history(&id) {
            Ok(history) => {
                let mut rows = self
                    .model
                    .turns
                    .iter()
                    .enumerate()
                    .map(|(index, turn)| {
                        let preview: String =
                            turn.text.replace('\n', " ").chars().take(80).collect();
                        format!(
                            "- turn:{} | {:?} | {} | {}",
                            index + 1,
                            turn.role,
                            turn.created_at,
                            preview
                        )
                    })
                    .collect::<Vec<_>>();
                rows.extend(history.checkpoints.iter().map(|checkpoint| {
                    format!(
                        "- checkpoint:{} | {} | {} turn(s) | {}",
                        checkpoint.id,
                        checkpoint.label,
                        checkpoint.turn_count,
                        checkpoint.created_at
                    )
                }));
                rows.extend(history.compactions.iter().map(|boundary| {
                    format!(
                        "- compaction | {} older turn(s) | raw checkpoint:{} | {}",
                        boundary.compacted_turns, boundary.checkpoint_id, boundary.created_at
                    )
                }));
                rows.extend(
                    history
                        .interruptions
                        .iter()
                        .map(|at| format!("- interrupted | {at}")),
                );
                log::info!(
                    "assistant[{id}]: /history turns={} checkpoints={} compactions={} interruptions={}",
                    self.model.turns.len(),
                    history.checkpoints.len(),
                    history.compactions.len(),
                    history.interruptions.len()
                );
                let effects = self.model.push_info(format!(
                    "**Conversation history**\n\n{}",
                    if rows.is_empty() {
                        "No history yet.".to_string()
                    } else {
                        rows.join("\n")
                    }
                ));
                self.execute_effects(effects);
            }
            Err(e) => {
                log::error!("assistant[{id}]: /history failed: {e}");
                let effects = self
                    .model
                    .push_error(format!("Could not read history: {e}"));
                self.execute_effects(effects);
            }
        }
    }

    fn cmd_rewind_conversation(&mut self, selector: &str) {
        let target = if let Some(raw) = selector.strip_prefix("turn:") {
            match raw.parse::<usize>() {
                Ok(count) if count <= self.model.turns.len() => {
                    Ok(self.model.turns[..count].to_vec())
                }
                _ => Err(format!("Invalid turn selector `{selector}`.")),
            }
        } else if let Some(checkpoint) = selector.strip_prefix("checkpoint:") {
            self.store
                .load_checkpoint(&self.model.conversation_id, checkpoint)
        } else {
            Err("Use `/rewind turn:N` or `/rewind checkpoint:ID`.".to_string())
        };
        let target = match target {
            Ok(target) => target,
            Err(e) => {
                let effects = self.model.push_error(e);
                self.execute_effects(effects);
                return;
            }
        };
        let id = self.model.conversation_id.clone();
        let checkpoint = match self
            .store
            .write_checkpoint(&id, "rewind-safety", &self.model.turns)
        {
            Ok(checkpoint) => checkpoint,
            Err(e) => {
                log::error!("assistant[{id}]: /rewind safety checkpoint failed: {e}");
                let effects = self.model.push_error(format!(
                    "Rewind stopped: could not write safety checkpoint: {e}"
                ));
                self.execute_effects(effects);
                return;
            }
        };
        self.pending_skill = None;
        let cancel = self.model.switch_conversation(id.clone(), target);
        self.execute_effects(cancel);
        log::info!(
            "assistant[{id}]: rewound conversation context safety_checkpoint={}",
            checkpoint.id
        );
        let effects = self.model.push_info(format!(
            "Conversation context rewound to `{selector}`. Safety checkpoint: `{}`. Files and apps were untouched.",
            checkpoint.id
        ));
        self.execute_effects(effects);
    }

    fn cmd_compact_conversation(&mut self) {
        const RETAIN_RECENT: usize = 6;
        const SUMMARY_BUDGET_CHARS: usize = 4_096;
        if self.model.turns.len() <= RETAIN_RECENT {
            self.model.clear_compaction();
            let effects = self
                .model
                .push_info("Nothing to compact yet; fewer than seven turns are active.");
            self.execute_effects(effects);
            return;
        }
        let id = self.model.conversation_id.clone();
        let compacted = self.model.turns.len() - RETAIN_RECENT;
        let checkpoint =
            match self
                .store
                .write_checkpoint(&id, "pre-compaction-raw-history", &self.model.turns)
            {
                Ok(checkpoint) => checkpoint,
                Err(e) => {
                    self.model.clear_compaction();
                    log::error!("assistant[{id}]: /compact checkpoint failed: {e}");
                    let effects = self.model.push_error(format!(
                        "Compaction stopped: could not preserve raw history: {e}"
                    ));
                    self.execute_effects(effects);
                    return;
                }
            };
        let summary =
            deterministic_context_summary(&self.model.turns[..compacted], SUMMARY_BUDGET_CHARS);
        let recent = self.model.turns[compacted..].to_vec();
        let mut compacted_turns = vec![model::Turn::now(
            TurnRole::Assistant,
            format!(
                "Compacted context ({compacted} turn(s)); raw history checkpoint `{}`:\n{excerpts}",
                checkpoint.id,
                excerpts = summary
            ),
        )];
        compacted_turns.extend(recent);
        if let Err(e) = self.store.record_compaction(&id, &checkpoint.id, compacted) {
            self.model.clear_compaction();
            log::error!("assistant[{id}]: failed to record compaction boundary: {e}");
            let effects = self.model.push_error(format!(
                "Raw checkpoint written, but compaction stopped because boundary metadata failed: {e}"
            ));
            self.execute_effects(effects);
            return;
        }
        self.pending_skill = None;
        self.model
            .complete_compaction(compacted, checkpoint.id.clone());
        let cancel = self.model.switch_conversation(id.clone(), compacted_turns);
        self.execute_effects(cancel);
        log::info!(
            "assistant[{id}]: compacted {compacted} turn(s) checkpoint={}",
            checkpoint.id
        );
        let effects = self.model.push_info(format!(
            "Compacted {compacted} turns into checkpoint `{}`.",
            checkpoint.id
        ));
        self.execute_effects(effects);
    }

    /// Complete a queued `/compact` after the visible status frame. Active
    /// panes call this from [`App::ui`]; inactive panes use `background_tick`.
    fn run_pending_compaction(&mut self) -> bool {
        if !self.compact_pending {
            return false;
        }
        self.compact_pending = false;
        self.cmd_compact_conversation();
        true
    }

    fn cmd_export_conversation(&mut self) {
        let audit_path = self.profile_dir.join("audit.jsonl");
        let audit = match std::fs::read_to_string(&audit_path) {
            Ok(raw) => raw,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => {
                log::error!("assistant: /export read {}: {e}", audit_path.display());
                let effects = self.model.push_error(format!(
                    "Export failed reading {}: {e}",
                    audit_path.display()
                ));
                self.execute_effects(effects);
                return;
            }
        };
        let id = self.model.conversation_id.clone();
        match self
            .store
            .export_conversation(&id, &self.model.turns, &audit)
        {
            Ok(path) => {
                log::info!(
                    "assistant[{id}]: exported transcript and audit to {}",
                    path.display()
                );
                let effects = self.model.push_info(format!(
                    "Exported transcript and tool/audit log to `{}`.",
                    path.display()
                ));
                self.execute_effects(effects);
            }
            Err(e) => {
                log::error!("assistant[{id}]: /export failed: {e}");
                let effects = self.model.push_error(format!("Export failed: {e}"));
                self.execute_effects(effects);
            }
        }
    }

    fn set_session_model(&mut self, tier: ModelTier) {
        self.session_overrides.model_tier = Some(tier);
        self.reload_settings();
        log::info!(
            "assistant[{}]: session model tier changed to {}",
            self.model.conversation_id,
            settings::model_tier_name(tier)
        );
        let effects = self.model.push_info(format!(
            "Model tier set to `{}` for this session.",
            settings::model_tier_name(tier)
        ));
        self.execute_effects(effects);
    }

    /// The model tier that would drive the next turn: a session/user override
    /// wins, otherwise the active agent's default. `reload_settings` must have
    /// run so `self.settings` is current.
    fn resolved_model_tier(&self) -> ModelTier {
        let configured = &self.settings.model.tier;
        if self.session_overrides.model_tier.is_some()
            || configured.source.scope != settings::SettingsScope::Default
        {
            configured.value
        } else if let Some(agent) = self.active_agent() {
            agent.default_tier
        } else {
            configured.value
        }
    }

    /// `/model` (no args): open the interactive model/agent picker.
    fn open_model_picker(&mut self) {
        self.reload_settings();
        self.reload_agents();
        let current_tier = self.resolved_model_tier();
        let tiers = vec![ModelTier::Low, ModelTier::Medium, ModelTier::High];
        let agents = self
            .agent_registry
            .agents()
            .map(|agent| AgentChoice {
                id: agent.id.clone(),
                display_name: agent.display_name.clone(),
            })
            .collect();
        self.model.open_model_picker(current_tier, tiers, agents);
    }

    fn cmd_show_settings(&mut self) {
        self.reload_settings();
        let tier = &self.settings.model.tier;
        let mut text = format!(
            "**Assistant settings**\n\n- Model tier: `{}` ({})\n\
             - Permission posture: `{}` ({})\n\
             - Enabled tools: {} ({})\n\
             - Memory: {} ({})\n\
             - Enabled hooks: {} ({})\n",
            settings::model_tier_name(tier.value),
            tier.source.description(),
            self.settings.permissions.posture.value.as_str(),
            self.settings.permissions.posture.source.description(),
            format_setting_ids(&self.settings.tools.enabled.value),
            self.settings.tools.enabled.source.description(),
            if self.settings.memory.enabled.value {
                "enabled"
            } else {
                "disabled"
            },
            self.settings.memory.enabled.source.description(),
            format_setting_ids(&self.settings.hooks.enabled.value),
            self.settings.hooks.enabled.source.description()
        );
        if self.settings.permissions.rules.is_empty() {
            text.push_str("- Permission rules: none\n");
        } else {
            text.push_str("- Permission rules:\n");
            for rule in &self.settings.permissions.rules {
                text.push_str(&format!(
                    "  - `{}`: {} ({})\n",
                    rule.rule,
                    rule.decision.as_str(),
                    rule.source.description()
                ));
            }
        }
        if !self.settings_errors.is_empty() {
            text.push_str("\n**Load errors**\n\n");
            for error in &self.settings_errors {
                text.push_str(&format!("- {error}\n"));
            }
        }
        log::info!(
            "assistant: /settings showing {} permission rule(s), {} load error(s)",
            self.settings.permissions.rules.len(),
            self.settings_errors.len()
        );
        let effects = self.model.push_info(text);
        self.execute_effects(effects);
    }

    /// `/tools`: discovered app connector tools with their broker decisions.
    fn cmd_list_tools(&mut self) {
        let dispatcher = ToolDispatcher::from_registry(
            0,
            format!("agent:{ASSISTANT_ACTOR_ID}"),
            self.workspace_root.clone(),
        );
        let mut tools = dispatcher.all_tools();
        tools.sort_by(|a, b| a.name.cmp(&b.name));
        let streams = self.timeline.lock().unwrap().all_declared_streams();
        log::info!(
            "assistant: /tools — {} connector tool(s), {} declared event stream(s) discovered",
            tools.len(),
            streams.len()
        );
        let mut text = if tools.is_empty() {
            "No app connector tools are exposed in this workspace.\n".to_string()
        } else {
            let mut out = String::from("App connector tools:\n");
            for tool in tools {
                out.push_str(&format!(
                    "{} — {}\n",
                    Self::connector_target(&tool.name),
                    self.tool_decision(&tool).as_str()
                ));
            }
            out
        };
        text.push_str("\nNative host tools:\n");
        for tool in Self::host_tools() {
            text.push_str(&format!(
                "{} — {}\n",
                tool.name,
                self.host_tool_decision(&tool).as_str()
            ));
        }
        if streams.is_empty() {
            text.push_str("\nNo app event streams declared in this workspace.");
        } else {
            text.push_str("\nApp event streams (host.events.subscribe / unsubscribe):\n");
            for (app, event) in streams {
                let target = format!("{app}::{event}");
                let decision = self.event_stream_decision(&app, &event);
                let subscribed = self.live_subs.iter().any(|(t, _)| *t == target);
                text.push_str(&format!(
                    "{target} — {}{}\n",
                    decision.as_str(),
                    if subscribed { ", subscribed" } else { "" }
                ));
            }
        }
        let effects = self.model.push_info(text);
        self.execute_effects(effects);
    }

    fn cmd_list_skills(&mut self) {
        let enabled = self
            .active_agent()
            .map(|agent| agent.skills.as_slice())
            .unwrap_or(&[]);
        let visible = self
            .skill_registry
            .all()
            .iter()
            .filter(|skill| enabled.is_empty() || enabled.contains(&skill.name))
            .collect::<Vec<_>>();
        log::info!("assistant: /skills — {} installed skill(s)", visible.len());
        let text = if visible.is_empty() {
            "No skills are installed for this channel or workspace.".to_string()
        } else {
            let mut text = String::from("Installed skills:\n");
            for skill in visible {
                text.push_str(&format!(
                    "- `/{}` — {} _({}: {})_\n",
                    skill.name,
                    skill.description,
                    skill.source.label(),
                    skill.path.display()
                ));
            }
            text
        };
        let effects = self.model.push_info(text);
        self.execute_effects(effects);
    }

    fn cmd_invoke_skill(&mut self, name: &str, args: &str) {
        let enabled = self
            .active_agent()
            .map(|agent| agent.skills.as_slice())
            .unwrap_or(&[]);
        if !enabled.is_empty() && !enabled.iter().any(|skill| skill == name) {
            let effects = self.model.push_error(format!(
                "Skill `/{name}` is not enabled for agent `{}`.",
                self.model.active_agent_id
            ));
            self.execute_effects(effects);
            return;
        }
        let Some(skill) = self.skill_registry.get(name).cloned() else {
            let effects = self.model.push_error(format!(
                "Unknown command or installed skill `/{name}`. Type /help or /skills."
            ));
            self.execute_effects(effects);
            return;
        };
        log::info!(
            "assistant: manually invoking {} skill '{}'",
            skill.source.label(),
            name
        );
        self.pending_skill = Some(skill);
        let prompt = if args.is_empty() {
            format!("Run the /{name} skill.")
        } else {
            args.to_string()
        };
        let effects = self.model.submit_prompt(prompt);
        self.execute_effects(effects);
    }

    fn cmd_show_context(&mut self) {
        let chars: usize = self
            .model
            .turns
            .iter()
            .map(|turn| turn.text.chars().count())
            .sum();
        let tool_names = self
            .gated_dispatcher()
            .all_tools()
            .into_iter()
            .map(|tool| tool.name)
            .collect::<Vec<_>>();
        let agent = self.active_agent();
        let enabled_skill_count = agent
            .map(|agent| {
                self.skill_registry
                    .all()
                    .iter()
                    .filter(|skill| agent.skills.is_empty() || agent.skills.contains(&skill.name))
                    .count()
            })
            .unwrap_or(0);
        let panes = crate::plexi_ai::broker::get_pane_snapshot();
        let pane_context = if panes.is_empty() {
            "none reported yet".to_string()
        } else {
            panes
                .iter()
                .map(|pane| format!("{} (pane {})", pane.type_id, pane.pane_id))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let text = format!(
            "Assistant context:\n- Estimated transcript tokens: ~{} ({} turns)\n- Loaded instructions: agent `{}` plus {} installed skill(s) available on demand\n- Active workspace context: `{}`\n- Open pane/app context: {}\n- Enabled tools: {}",
            chars.div_ceil(4),
            self.model.turns.len(),
            agent.map(|agent| agent.id.as_str()).unwrap_or("default"),
            enabled_skill_count,
            self.workspace_root.display(),
            pane_context,
            if tool_names.is_empty() { "none".to_string() } else { tool_names.join(", ") },
        );
        log::info!(
            "assistant: /context — turns={} estimated_tokens={}",
            self.model.turns.len(),
            chars.div_ceil(4)
        );
        let effects = self.model.push_info(text);
        self.execute_effects(effects);
    }

    fn cmd_show_hooks(&mut self) {
        let mut hooks = self.settings.hooks.enabled.value.clone();
        if let Some(agent) = self.active_agent() {
            hooks.extend(agent.hooks.iter().cloned());
        }
        hooks.sort();
        hooks.dedup();
        let source = &self.settings.hooks.enabled.source;
        let text = if hooks.is_empty() {
            "No Assistant lifecycle hooks are enabled.".to_string()
        } else {
            format!(
                "Assistant lifecycle hooks (settings source: `{:?}` at `{}`):\n{}",
                source.scope,
                source
                    .path
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "defaults".to_string()),
                hooks
                    .into_iter()
                    .map(|hook| format!("- `{hook}`"))
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        };
        log::info!("assistant: /hooks");
        let effects = self.model.push_info(text);
        self.execute_effects(effects);
    }

    /// `/apps`: running apps and the connectors they expose, with broker decisions.
    fn cmd_list_apps(&mut self) {
        let apps = ToolDispatcher::apps_for_workspace(self.workspace_root.clone());
        log::info!(
            "assistant: /apps — {} app(s) with connector tools in workspace",
            apps.len()
        );
        let text = if apps.is_empty() {
            "No apps are exposing connector tools in this workspace.\n\
             Start an app that calls emit_tools() to see its connectors here."
                .to_string()
        } else {
            let mut out = String::new();
            for (app_id, mut tools) in apps {
                tools.sort_by(|a, b| a.name.cmp(&b.name));
                let ro_count = tools.iter().filter(|t| t.read_only).count();
                let rw_count = tools.len() - ro_count;
                out.push_str(&format!(
                    "{app_id} — {} tool(s) ({ro_count} read-only, {rw_count} mutating)\n",
                    tools.len()
                ));
                for tool in &tools {
                    let decision = self.tool_decision(tool);
                    let kind = if tool.read_only { "read" } else { "mutate" };
                    out.push_str(&format!(
                        "  {} [{}] — {}\n",
                        Self::connector_target(&tool.name),
                        kind,
                        decision.as_str()
                    ));
                }
            }
            out
        };
        let effects = self.model.push_info(text);
        self.execute_effects(effects);
    }

    /// Persisted grants attributable to the assistant actor (connector tools,
    /// host tools, and shared event streams), as editable overlay rows.
    fn assistant_grant_rows(&self) -> Vec<GrantRow> {
        let connector_actor_id = self.connector_actor().0;
        self.grant_store
            .records()
            .iter()
            .filter(|r| {
                r.actor_type == ActorType::Agent
                    && ((r.target_type == TargetType::AppConnector
                        && r.actor_id == connector_actor_id)
                        || (r.target_type == TargetType::HostTool
                            && r.actor_id == connector_actor_id)
                        || (r.target_type == TargetType::AppEventStream
                            && r.actor_id == ASSISTANT_ACTOR_ID))
            })
            .map(|r| GrantRow {
                target_id: r.target_id.clone(),
                decision: r.decision,
            })
            .collect()
    }

    /// `/permissions` and `/revoke` (no args): open the permissions manager.
    fn open_permissions_manager(&mut self) {
        let grants = self.assistant_grant_rows();
        self.model.open_permissions_manager(grants);
    }

    /// Apply the permissions manager's edited rows (Enter). Only rows whose
    /// decision changed touch the grant store: Ask reverts a target to the
    /// posture default (delete), Allow/Deny re-record the existing grant's
    /// metadata with the new decision. Every write is audited.
    fn apply_permission_grants(&mut self, rows: Vec<GrantRow>) {
        let current: std::collections::HashMap<String, Decision> = self
            .assistant_grant_rows()
            .into_iter()
            .map(|r| (r.target_id, r.decision))
            .collect();
        let mut changed = 0usize;
        for row in rows {
            if current.get(&row.target_id) == Some(&row.decision) {
                continue;
            }
            let actor_id = if row.target_id.starts_with("app.") || row.target_id.starts_with("host.")
            {
                self.connector_actor().0
            } else {
                ASSISTANT_ACTOR_ID.to_string()
            };
            match row.decision {
                Decision::Ask => {
                    let removed =
                        self.grant_store
                            .revoke(ActorType::Agent, &actor_id, &row.target_id);
                    let unsubscribed = self.unsubscribe_stream(&row.target_id);
                    if removed > 0 {
                        self.grant_store.save();
                    }
                    self.audit.append(&AuditEvent::now(
                        "revoke",
                        &row.target_id,
                        "revoked",
                        &format!(
                            "{removed} grant(s), {unsubscribed} subscription(s) removed via permissions manager"
                        ),
                    ));
                }
                Decision::Allow | Decision::Deny => {
                    // Rows are built only from existing persisted grants, so
                    // each has a record whose metadata we mirror with the new
                    // decision.
                    let meta = self
                        .grant_store
                        .records()
                        .iter()
                        .find(|r| {
                            r.actor_type == ActorType::Agent
                                && r.actor_id == actor_id
                                && r.target_id == row.target_id
                        })
                        .map(|r| (r.target_type, r.actor_scope, r.duration, r.source));
                    let Some((target_type, actor_scope, duration, source)) = meta else {
                        log::warn!(
                            "assistant: permissions manager skipped '{}' — no existing grant to re-record",
                            row.target_id
                        );
                        continue;
                    };
                    self.record_grant_with_decision(
                        &actor_id,
                        actor_scope,
                        target_type,
                        &row.target_id,
                        duration,
                        source,
                        row.decision,
                    );
                    self.grant_store.save();
                    self.audit.append(&AuditEvent::now(
                        "permission_set",
                        &row.target_id,
                        row.decision.as_str(),
                        "via permissions manager",
                    ));
                }
            }
            changed += 1;
            log::info!(
                "assistant: permission '{}' set to {} via picker",
                row.target_id,
                row.decision.as_str()
            );
        }
        let text = if changed == 0 {
            "No permission changes.".to_string()
        } else {
            format!("Updated {changed} permission(s).")
        };
        let effects = self.model.push_info(text);
        self.execute_effects(effects);
    }

    /// Confirm the open overlay (Enter): dispatch the selection through the
    /// same handlers the text commands use, then close.
    fn confirm_overlay(&mut self) {
        match std::mem::take(&mut self.model.overlay) {
            AssistantOverlay::ModelPicker {
                selected,
                tiers,
                agents,
                ..
            } => {
                if selected < tiers.len() {
                    self.set_session_model(tiers[selected]);
                } else if let Some(choice) = agents.get(selected - tiers.len()) {
                    let id = choice.id.clone();
                    self.cmd_switch_agent(&id);
                }
            }
            AssistantOverlay::PermissionsManager { grants, .. } => {
                self.apply_permission_grants(grants);
            }
            AssistantOverlay::None => {}
        }
    }

    /// `/revoke <target_id>`: remove persisted grants for one target. Event
    /// stream targets also lose their live timeline subscription.
    fn cmd_revoke(&mut self, target_id: &str) {
        let actor_id = if target_id.starts_with("app.") || target_id.starts_with("host.") {
            self.connector_actor().0
        } else {
            ASSISTANT_ACTOR_ID.to_string()
        };
        let removed = self
            .grant_store
            .revoke(ActorType::Agent, &actor_id, target_id);
        let unsubscribed = self.unsubscribe_stream(target_id);
        let text = if removed == 0 && unsubscribed == 0 {
            format!("No grants found for '{target_id}'. See /permissions for target ids.")
        } else {
            if removed > 0 {
                self.grant_store.save();
            }
            self.audit.append(&AuditEvent::now(
                "revoke",
                target_id,
                "revoked",
                &format!("{removed} grant(s), {unsubscribed} subscription(s) removed"),
            ));
            format!(
                "Revoked {removed} grant(s) for '{target_id}'.{}",
                if unsubscribed > 0 {
                    format!(" Removed {unsubscribed} live subscription(s).")
                } else {
                    String::new()
                }
            )
        };
        let effects = self.model.push_info(text);
        self.execute_effects(effects);
    }

    /// `/audit`: recent audit events as an info row.
    fn cmd_show_audit(&mut self) {
        const AUDIT_TAIL: usize = 10;
        let events = self.audit.tail(AUDIT_TAIL);
        log::info!("assistant: /audit — showing {} event(s)", events.len());
        let text = if events.is_empty() {
            "No audit events yet.".to_string()
        } else {
            let mut out = format!("Last {} audit event(s):\n", events.len());
            for ev in events {
                out.push_str(&format!(
                    "{} {} {} = {}{}\n",
                    ev.ts,
                    ev.kind,
                    ev.target,
                    ev.decision,
                    if ev.summary.is_empty() {
                        String::new()
                    } else {
                        format!(" ({})", ev.summary)
                    }
                ));
            }
            out
        };
        let effects = self.model.push_info(text);
        self.execute_effects(effects);
    }
}

impl App for AssistantApp {
    fn type_id(&self) -> &'static str {
        "assistant"
    }

    fn display_name(&self) -> String {
        "Assistant".to_string()
    }

    fn take_pending_commands(&mut self) -> Vec<AppCommand> {
        std::mem::take(&mut self.pending_commands)
    }

    fn background_tick(&mut self) {
        self.pump_turn_io();
        self.run_pending_compaction();
    }

    fn needs_background_tick(&self) -> bool {
        self.model.streaming.in_flight || self.compact_pending || !self.pending_commands.is_empty()
    }

    fn handle_key(&mut self, input: &crate::app::input_router::PlexiInput) -> KeyDisposition {
        // The model/agent picker and permissions manager own Escape while
        // open: this runs before `poll_actions`, which binds plain Escape to
        // `CloseApp` when the assistant's app surface is focused
        // (`src/host/keys.rs` `AppActive` context). Without claiming it here
        // first, Escape falls through and destroys the pane instead of just
        // closing the overlay — the same precedent `dispatch_app_key_events`
        // already documents for file-browser search-mode Escape. Returning
        // `Passthrough` for every other key (including ArrowUp) leaves them
        // in the frame's input buffer for `render.rs`'s `handle_overlay_keys`
        // to consume during the render pass — `Consumed` here would make
        // `dispatch_app_key_events` strip ArrowUp out of the buffer too
        // (it claims Escape *and* ArrowUp together on any `Consumed`
        // disposition), starving the overlay's own up-navigation.
        if self.model.overlay_active() {
            if input.key_pressed(egui::Key::Escape) {
                self.model.cancel_overlay();
                return KeyDisposition::Consumed;
            }
            return KeyDisposition::Passthrough;
        }
        // The permission sheet also owns Escape while open — same rationale
        // as the overlay case above: resolve it as a deny here so `Escape`
        // never falls through to `CloseApp`, and leave Tab/arrows/Enter in
        // the buffer for `render.rs`'s `handle_permission_keys` to consume.
        if self.model.pending_permission.is_some() {
            if input.key_pressed(egui::Key::Escape) {
                self.resolve_permission(PermissionChoice::Deny);
                return KeyDisposition::Consumed;
            }
            return KeyDisposition::Passthrough;
        }
        if input.key_pressed(egui::Key::Escape) && self.model.streaming.in_flight {
            self.interrupt_in_flight_turn();
            return KeyDisposition::Consumed;
        }
        if !input.modifiers().any()
            && input.key_pressed(egui::Key::ArrowUp)
            && self.model.recall_previous_user_message()
        {
            return KeyDisposition::Consumed;
        }
        KeyDisposition::Passthrough
    }

    fn rename_seed(&self) -> Option<String> {
        self.model.session_name.clone()
    }

    fn on_pane_renamed(&mut self, name: &str) {
        log::info!(
            "assistant[{}]: pane renamed to '{name}'",
            self.model.conversation_id
        );
        self.model.set_session_name(name);
        self.session_write();
    }

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &AppRenderContext<'_>) {
        self.pump_turn_io();
        // Active panes do not receive `background_tick`; defer until after
        // this frame draws the status row, then compact before the next frame.
        let compact_visible_this_frame = self.compact_pending;
        let event = AssistantRenderer::draw(
            ui,
            &mut self.model,
            &mut self.commonmark_cache,
            &mut self.markdown_text_cache,
            ctx.colors,
            ctx.is_focused,
        );
        match event {
            Some(ComposerEvent::Submit) => {
                let effects = self.model.submit();
                self.execute_effects(effects);
            }
            Some(ComposerEvent::Permission(choice)) => self.resolve_permission(choice),
            Some(ComposerEvent::OverlayConfirm) => self.confirm_overlay(),
            None => {}
        }
        if compact_visible_this_frame && self.run_pending_compaction() {
            ui.ctx().request_repaint();
        }
        if self.model.streaming.in_flight || self.compact_pending {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(50));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plexi_ai::broker::AiBrokerResponse;

    /// Scripted test broker.
    struct MockBroker {
        /// Canned reply text. If None, simulates an error turn.
        reply: Option<String>,
    }

    #[derive(Default)]
    struct CapturingBroker {
        seen: Mutex<Vec<AiBrokerRequest>>,
    }

    impl AiBroker for CapturingBroker {
        fn dispatch(
            &self,
            request: AiBrokerRequest,
            _on_delta: &mut dyn FnMut(TurnDelta<'_>),
        ) -> AiBrokerResponse {
            self.seen.lock().unwrap().push(request);
            AiBrokerResponse::ok("ok".to_string(), 0, 0)
        }
    }

    impl MockBroker {
        fn ok(reply: impl Into<String>) -> Arc<Self> {
            Arc::new(Self {
                reply: Some(reply.into()),
            })
        }
        fn error() -> Arc<Self> {
            Arc::new(Self { reply: None })
        }
    }

    impl AiBroker for MockBroker {
        fn dispatch(
            &self,
            _request: AiBrokerRequest,
            on_delta: &mut dyn FnMut(TurnDelta<'_>),
        ) -> AiBrokerResponse {
            match &self.reply {
                Some(text) => {
                    on_delta(TurnDelta::Reasoning("pondering"));
                    for chunk in text.split_inclusive(' ') {
                        on_delta(TurnDelta::Text(chunk));
                    }
                    AiBrokerResponse::ok(text.clone(), 1, 1)
                }
                None => AiBrokerResponse {
                    content: None,
                    error: Some("mock_error: simulated broker failure".to_string()),
                    tokens_in: 0,
                    tokens_out: 0,
                },
            }
        }
    }

    /// Assistant whose grants + audit live in the (temp) workspace dir.
    fn test_app(ws: &Path) -> AssistantApp {
        AssistantApp::new(ws.to_path_buf(), MockBroker::ok("echo: ok"), ws)
    }

    fn wait_for_turn(app: &mut AssistantApp) {
        let start = std::time::Instant::now();
        while app.model.streaming.in_flight {
            app.pump_turn_io();
            assert!(
                start.elapsed() < std::time::Duration::from_secs(5),
                "turn never completed"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn full_turn_streams_deltas_and_persists() {
        let ws = tempfile::tempdir().unwrap();
        let mut app = test_app(ws.path());
        let conversation_id = app.model.conversation_id.clone();

        app.model.composer = "hello".to_string();
        let effects = app.model.submit();
        app.execute_effects(effects);
        wait_for_turn(&mut app);

        assert_eq!(app.model.turns.len(), 2);
        assert_eq!(app.model.turns[1].role, TurnRole::Assistant);
        assert_eq!(app.model.turns[1].text, "echo: ok");

        // Close-and-reopen resumes the same conversation from disk.
        drop(app);
        let reopened = test_app(ws.path());
        assert_eq!(reopened.model.conversation_id, conversation_id);
        assert_eq!(reopened.model.turns.len(), 2);
        assert_eq!(reopened.model.turns[1].text, "echo: ok");
    }

    #[test]
    fn scoped_and_session_model_tiers_route_active_agent_concretely() {
        let ws = tempfile::tempdir().unwrap();
        let assistant_settings_dir = ws
            .path()
            .join(crate::config::workspace_channel_dir())
            .join("agents");
        std::fs::create_dir_all(&assistant_settings_dir).unwrap();
        std::fs::write(
            assistant_settings_dir.join("settings.toml"),
            "[model]\ntier = \"medium\"\n",
        )
        .unwrap();
        let agent_dir = ws.path().join("agents").join("writer");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(agent_dir.join("AGENT.md"), "Writer system prompt").unwrap();
        std::fs::write(
            agent_dir.join("settings.toml"),
            r#"
[agent]
id = "writer"
display_name = "Writer"
default_tier = "high"
effort = "medium"
[permissions]
default_posture = "ask"
[models.high]
provider = "openrouter"
model = "anthropic/test"
[models.medium]
provider = "openrouter"
model = "anthropic/medium"
[models.low]
provider = "openrouter"
model = "anthropic/low"
[tools]
enabled = ["allowed.tool"]
"#,
        )
        .unwrap();
        let broker = Arc::new(CapturingBroker::default());
        let mut app = AssistantApp::new(ws.path().to_path_buf(), broker.clone(), ws.path());
        register_echo_provider(
            9199,
            &["allowed.tool", "denied.tool"],
            ws.path().to_path_buf(),
        );

        app.model.composer = "/agent writer".to_string();
        let effects = app.model.submit();
        app.execute_effects(effects);
        app.model.composer = "hello".to_string();
        let effects = app.model.submit();
        app.execute_effects(effects);
        wait_for_turn(&mut app);

        app.model.composer = "/model low".to_string();
        let effects = app.model.submit();
        app.execute_effects(effects);
        app.model.composer = "hello again".to_string();
        let effects = app.model.submit();
        app.execute_effects(effects);
        wait_for_turn(&mut app);

        let seen = broker.seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(
            seen[0].system,
            "Writer system prompt\n\nCompaction status: No compaction is running."
        );
        assert_eq!(seen[0].model_tier, ModelTier::Medium);
        assert_eq!(
            seen[0].concrete_model.as_ref().unwrap().model,
            "anthropic/medium"
        );
        assert_eq!(seen[1].model_tier, ModelTier::Low);
        assert_eq!(
            seen[1].concrete_model.as_ref().unwrap().model,
            "anthropic/low"
        );
        assert_eq!(seen[1].reasoning_effort, Some(ReasoningEffort::Medium));
        let tool_names = seen[1]
            .tool_dispatcher
            .as_ref()
            .unwrap()
            .all_tools()
            .into_iter()
            .map(|tool| tool.name)
            .collect::<Vec<_>>();
        assert!(tool_names.contains(&"allowed.tool".to_string()));
        assert!(!tool_names.contains(&"denied.tool".to_string()));
        tool_dispatch::unregister(9199);
    }

    #[test]
    fn create_and_edit_agent_are_file_backed_and_builtin_is_read_only() {
        let ws = tempfile::tempdir().unwrap();
        let mut app = test_app(ws.path());
        app.cmd_create_agent("writer");
        let dir = ws.path().join("agents").join("writer");
        assert!(dir.join("AGENT.md").is_file());
        assert!(dir.join("settings.toml").is_file());
        app.cmd_edit_agent("writer");
        assert!(app
            .model
            .turns
            .last()
            .unwrap()
            .text
            .contains(&dir.display().to_string()));
        app.cmd_edit_agent("default");
        assert_eq!(app.model.turns.last().unwrap().role, TurnRole::Error);
        assert!(app
            .model
            .turns
            .last()
            .unwrap()
            .text
            .contains("cannot be edited"));
    }

    #[test]
    fn create_agent_refuses_to_overwrite_partial_definition() {
        let ws = tempfile::tempdir().unwrap();
        let dir = ws.path().join("agents").join("writer");
        std::fs::create_dir_all(&dir).unwrap();
        let prompt_path = dir.join("AGENT.md");
        std::fs::write(&prompt_path, "Keep this prompt.\n").unwrap();
        let mut app = test_app(ws.path());

        app.cmd_create_agent("writer");

        assert_eq!(
            std::fs::read_to_string(prompt_path).unwrap(),
            "Keep this prompt.\n"
        );
        assert!(!dir.join("settings.toml").exists());
        assert_eq!(app.model.turns.last().unwrap().role, TurnRole::Error);
    }

    #[test]
    fn named_agent_connector_permissions_use_agent_identity_and_scope() {
        let ws = tempfile::tempdir().unwrap();
        let mut app = test_app(ws.path());
        app.cmd_create_agent("writer");
        app.cmd_switch_agent("writer");
        app.record_connector_grant(
            TargetType::AppConnector,
            "app.docs.write",
            GrantDuration::Session,
            GrantSource::Session,
        );

        let record = app.grant_store.records().last().unwrap();
        assert_eq!(record.actor_id, "writer");
        assert_eq!(record.actor_scope, ActorScope::User);
        let tool = AiTool {
            name: "docs.write".to_string(),
            description: "write docs".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
            output_schema: serde_json::json!({"type": "object"}),
            timeout_ms: None,
            read_only: false,
        };
        assert_eq!(app.tool_decision(&tool), Decision::Allow);
    }

    #[test]
    fn agent_commands_expose_shadowed_definition_summaries() {
        let ws = tempfile::tempdir().unwrap();
        let write_agent = |dir: PathBuf, name: &str, description: &str| {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("AGENT.md"), format!("{name} prompt")).unwrap();
            std::fs::write(
                dir.join("settings.toml"),
                format!(
                    "[agent]\nid = \"writer\"\ndisplay_name = \"{name}\"\ndescription = \"{description}\"\ndefault_tier = \"medium\"\n\n[permissions]\ndefault_posture = \"ask\"\n"
                ),
            )
            .unwrap();
        };
        write_agent(ws.path().join("agents/writer"), "User Writer", "user copy");
        write_agent(
            ws.path()
                .join(crate::config::workspace_channel_dir())
                .join("agents/writer"),
            "Workspace Writer",
            "workspace copy",
        );
        let mut app = test_app(ws.path());

        app.cmd_list_agents();
        let list = &app.model.turns.last().unwrap().text;
        assert!(list.contains("1 shadowed: user"), "{list}");
        app.cmd_inspect_agent("writer");
        let inspect = &app.model.turns.last().unwrap().text;
        assert!(inspect.contains("Workspace Writer"), "{inspect}");
        assert!(inspect.contains("User Writer [user]"), "{inspect}");
        assert!(inspect.contains("description=user copy"), "{inspect}");
    }

    #[test]
    fn read_only_connector_auto_allowed_without_broker_grant() {
        let ws = tempfile::tempdir().unwrap();
        let app = test_app(ws.path());
        let (tx, _rx) = std::sync::mpsc::channel();
        let ro_tool = crate::app_protocol::AiTool {
            name: "csv.read_range".to_string(),
            description: "read cells".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
            output_schema: serde_json::json!({"type": "object"}),
            timeout_ms: Some(2_000),
            read_only: true,
        };
        let rw_tool = crate::app_protocol::AiTool {
            name: "csv.write_cell".to_string(),
            description: "write a cell".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
            output_schema: serde_json::json!({"type": "object"}),
            timeout_ms: Some(2_000),
            read_only: false,
        };
        tool_dispatch::register(
            9200,
            "csv".to_string(),
            vec![ro_tool, rw_tool],
            AppEventSender::Channel(tx),
            ws.path().to_path_buf(),
        );
        // No grants seeded — read-only tool must be auto-allowed; mutating
        // tool must remain ask-gated and visible.
        let dispatcher = app.gated_dispatcher();
        let mut visible: Vec<String> = dispatcher.all_tools().into_iter().map(|t| t.name).collect();
        visible.sort();
        assert!(
            visible.contains(&"csv.read_range".to_string()),
            "read-only tool must be visible without an explicit grant: {visible:?}"
        );
        assert!(
            visible.contains(&"csv.write_cell".to_string()),
            "mutating tool must be visible (ask-gated by default): {visible:?}"
        );
        tool_dispatch::unregister(9200);
    }

    #[test]
    fn deny_grant_withholds_read_only_tool() {
        let ws = tempfile::tempdir().unwrap();
        let mut app = test_app(ws.path());
        let (tx, _rx) = std::sync::mpsc::channel();
        tool_dispatch::register(
            9205,
            "csv".to_string(),
            vec![crate::app_protocol::AiTool {
                name: "csv.read_range".to_string(),
                description: "read cells".to_string(),
                input_schema: serde_json::json!({"type": "object"}),
                output_schema: serde_json::json!({"type": "object"}),
                timeout_ms: None,
                read_only: true,
            }],
            AppEventSender::Channel(tx),
            ws.path().to_path_buf(),
        );
        // Explicit deny must still win even though the tool is read-only.
        seed_grant(&mut app, "app.csv.read_range", Decision::Deny);

        let dispatcher = app.gated_dispatcher();
        let visible: Vec<String> = dispatcher.all_tools().into_iter().map(|t| t.name).collect();
        assert!(
            !visible.contains(&"csv.read_range".to_string()),
            "explicit deny must withhold a read-only tool: {visible:?}"
        );
        tool_dispatch::unregister(9205);
    }

    #[test]
    fn resolved_settings_deny_is_enforced_by_the_permission_broker() {
        let ws = tempfile::tempdir().unwrap();
        let settings_path = ws.path().join("agents/settings.toml");
        std::fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
        std::fs::write(
            settings_path,
            "[permissions]\ndeny = [\"app.csv.read_range\"]\n",
        )
        .unwrap();
        let app = test_app(ws.path());
        let (tx, _rx) = std::sync::mpsc::channel();
        tool_dispatch::register(
            9206,
            "csv".to_string(),
            vec![crate::app_protocol::AiTool {
                name: "csv.read_range".to_string(),
                description: "read cells".to_string(),
                input_schema: serde_json::json!({"type": "object"}),
                output_schema: serde_json::json!({"type": "object"}),
                timeout_ms: None,
                read_only: true,
            }],
            AppEventSender::Channel(tx),
            ws.path().to_path_buf(),
        );

        let visible: Vec<String> = app
            .gated_dispatcher()
            .all_tools()
            .into_iter()
            .map(|tool| tool.name)
            .collect();

        assert!(!visible.contains(&"csv.read_range".to_string()));
        tool_dispatch::unregister(9206);
    }

    #[test]
    fn plan_posture_hides_mutating_connector_despite_allow_grant_but_keeps_reads() {
        let ws = tempfile::tempdir().unwrap();
        let settings_path = ws.path().join("agents/settings.toml");
        std::fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
        std::fs::write(settings_path, "[permissions]\ndefault_posture = \"plan\"\n").unwrap();
        let mut app = test_app(ws.path());
        let (tx, _rx) = std::sync::mpsc::channel();
        tool_dispatch::register(
            9207,
            "csv".to_string(),
            vec![
                crate::app_protocol::AiTool {
                    name: "csv.read_range".to_string(),
                    description: "read cells".to_string(),
                    input_schema: serde_json::json!({"type": "object"}),
                    output_schema: serde_json::json!({"type": "object"}),
                    timeout_ms: None,
                    read_only: true,
                },
                crate::app_protocol::AiTool {
                    name: "csv.write_cell".to_string(),
                    description: "write a cell".to_string(),
                    input_schema: serde_json::json!({"type": "object"}),
                    output_schema: serde_json::json!({"type": "object"}),
                    timeout_ms: None,
                    read_only: false,
                },
            ],
            AppEventSender::Channel(tx),
            ws.path().to_path_buf(),
        );
        seed_grant(&mut app, "app.csv.write_cell", Decision::Allow);

        let visible: HashSet<String> = app
            .gated_dispatcher()
            .all_tools()
            .into_iter()
            .map(|tool| tool.name)
            .collect();

        assert!(visible.contains("csv.read_range"));
        assert!(!visible.contains("csv.write_cell"));
        assert!(visible.contains(HOST_TOOL_PANES_LIST));
        assert!(!visible.contains(HOST_TOOL_PANES_CLOSE));
        assert!(!visible.contains(HOST_TOOL_APPS_OPEN));
        tool_dispatch::unregister(9207);
    }

    #[test]
    fn enabled_tool_filter_applies_to_native_host_tools() {
        let ws = tempfile::tempdir().unwrap();
        let settings_path = ws.path().join("agents/settings.toml");
        std::fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
        std::fs::write(settings_path, "[tools]\nenabled = [\"host.panes.list\"]\n").unwrap();
        let app = test_app(ws.path());

        let visible = app
            .gated_dispatcher()
            .all_tools()
            .into_iter()
            .map(|tool| tool.name)
            .collect::<HashSet<_>>();

        assert!(visible.contains(HOST_TOOL_PANES_LIST));
        assert!(!visible.contains(HOST_TOOL_PANES_CLOSE));
        assert!(!visible.contains(HOST_TOOL_APPS_OPEN));
    }

    #[test]
    fn apps_command_lists_running_app_connectors() {
        let ws = tempfile::tempdir().unwrap();
        let mut app = test_app(ws.path());
        let (tx, _rx) = std::sync::mpsc::channel();
        tool_dispatch::register(
            9201,
            "csv".to_string(),
            vec![
                crate::app_protocol::AiTool {
                    name: "csv.read_range".to_string(),
                    description: "read cells".to_string(),
                    input_schema: serde_json::json!({"type": "object"}),
                    output_schema: serde_json::json!({"type": "object"}),
                    timeout_ms: None,
                    read_only: true,
                },
                crate::app_protocol::AiTool {
                    name: "csv.write_cell".to_string(),
                    description: "write a cell".to_string(),
                    input_schema: serde_json::json!({"type": "object"}),
                    output_schema: serde_json::json!({"type": "object"}),
                    timeout_ms: None,
                    read_only: false,
                },
            ],
            AppEventSender::Channel(tx),
            ws.path().to_path_buf(),
        );
        app.model.composer = "/apps".to_string();
        let effects = app.model.submit();
        app.execute_effects(effects);

        let last = app.model.turns.last().expect("a turn must be added");
        assert!(
            last.text.contains("csv"),
            "/apps output must name the 'csv' app: {}",
            last.text
        );
        assert!(
            last.text.contains("csv.read_range"),
            "/apps output must list csv.read_range: {}",
            last.text
        );
        assert!(
            last.text.contains("read"),
            "/apps output must label read-only tools: {}",
            last.text
        );
        tool_dispatch::unregister(9201);
    }

    #[test]
    fn clear_keeps_prior_transcript_resumable_on_disk() {
        let ws = tempfile::tempdir().unwrap();
        let mut app = test_app(ws.path());
        let first_id = app.model.conversation_id.clone();

        app.model.composer = "first question".to_string();
        let effects = app.model.submit();
        app.execute_effects(effects);
        wait_for_turn(&mut app);

        app.model.composer = "/clear".to_string();
        let effects = app.model.submit();
        app.execute_effects(effects);
        let second_id = app.model.conversation_id.clone();
        assert_ne!(second_id, first_id);

        // The prior conversation is still fully on disk.
        let prior = app.store.load_turns(&first_id);
        assert_eq!(prior.len(), 2);
        // The new conversation is the persisted active one.
        assert_eq!(
            app.store.active_conversation().as_deref(),
            Some(second_id.as_str())
        );
    }

    #[test]
    fn resume_history_rewind_compact_and_export_are_observable() {
        let ws = tempfile::tempdir().unwrap();
        let mut app = test_app(ws.path());
        let first_id = app.model.conversation_id.clone();
        for index in 0..4 {
            let question = if index == 0 {
                format!("beginning-intent {} ending-intent", "q".repeat(2_000))
            } else {
                format!("question {index}")
            };
            app.model
                .turns
                .push(model::Turn::now(TurnRole::User, question));
            app.model.turns.push(model::Turn::now(
                TurnRole::Assistant,
                format!("answer {index}"),
            ));
        }
        app.session_write();

        app.model.composer = "/new second".to_string();
        let effects = app.model.submit();
        app.execute_effects(effects);
        let second_id = app.model.conversation_id.clone();

        app.model.composer = "/resume".to_string();
        let effects = app.model.submit();
        app.execute_effects(effects);
        assert!(app.model.turns.last().unwrap().text.contains(&first_id));
        assert!(app.model.turns.last().unwrap().text.contains(&second_id));

        app.model.composer = format!("/resume {first_id}");
        let effects = app.model.submit();
        app.execute_effects(effects);
        assert_eq!(app.model.conversation_id, first_id);
        assert!(app.model.turns.last().unwrap().text.contains("Resumed"));

        let pre_compaction = app.model.turns.clone();
        let pre_compaction_chars = pre_compaction
            .iter()
            .map(|turn| turn.text.chars().count())
            .sum::<usize>();
        app.model.composer = "/compact".to_string();
        let effects = app.model.submit();
        app.execute_effects(effects);
        App::background_tick(&mut app);
        assert!(app.model.turns[0].text.contains("Compacted context"));
        assert!(app.model.turns[0].text.contains("beginning-intent"));
        assert!(app.model.turns[0].text.contains("ending-intent"));
        assert!(app.model.turns[0].text.contains("chars omitted"));
        let history = app.store.load_history(&first_id).unwrap();
        assert_eq!(history.compactions.len(), 1);
        let raw = app
            .store
            .load_checkpoint(&first_id, &history.compactions[0].checkpoint_id)
            .unwrap();
        assert_eq!(raw, pre_compaction, "raw checkpoint is exact");
        let compacted_chars = app
            .model
            .turns
            .iter()
            .map(|turn| turn.text.chars().count())
            .sum::<usize>();
        assert!(
            compacted_chars < pre_compaction_chars / 2,
            "active context must be materially smaller"
        );

        app.model.composer = "/rewind turn:2".to_string();
        let effects = app.model.submit();
        app.execute_effects(effects);
        assert_eq!(app.model.turns.len(), 3, "two turns plus rewind notice");
        assert!(app
            .model
            .turns
            .last()
            .unwrap()
            .text
            .contains("Files and apps were untouched"));

        app.model.composer = "/history".to_string();
        let effects = app.model.submit();
        app.execute_effects(effects);
        let history_row = app.model.turns.last().unwrap();
        assert!(history_row.text.contains("checkpoint:"));
        assert!(history_row.text.contains("compaction"));

        app.model.composer = "/export".to_string();
        let effects = app.model.submit();
        app.execute_effects(effects);
        let export_row = app.model.turns.last().unwrap();
        assert!(export_row.text.contains("/exports/"));
    }

    #[test]
    fn restart_marks_persisted_in_flight_turn_interrupted() {
        let ws = tempfile::tempdir().unwrap();
        let id = {
            let mut app = test_app(ws.path());
            app.model.streaming.in_flight = true;
            app.session_write();
            app.model.conversation_id.clone()
        };

        let reopened = test_app(ws.path());
        assert_eq!(reopened.model.conversation_id, id);
        assert!(reopened
            .model
            .turns
            .iter()
            .any(|turn| turn.text.contains("was interrupted")));
        assert_eq!(
            reopened
                .store
                .load_history(&id)
                .unwrap()
                .interruptions
                .len(),
            1
        );
    }

    /// The Tool row role renders through the same Turn shape (Phase 2 seam).
    #[test]
    fn store_api_uses_model_turn_type() {
        let turn = crate::assistant::model::Turn::now(TurnRole::Tool, "tool output");
        assert_eq!(turn.role, TurnRole::Tool);
    }

    // ── Phase 2: connector gating + permission round trips ────────────────────

    use super::model::ToolStatus;
    use crate::plexi_ai::tool_dispatch::{self, AppEventSender, ToolCallResult};

    /// Register tools in the global registry with a responder thread that
    /// answers every `ToolCall` event with `{"ok": true}`.
    fn register_echo_provider(pane_id: u64, tool_names: &[&str], ws: PathBuf) {
        let (tx, rx) = std::sync::mpsc::channel();
        let tools = tool_names
            .iter()
            .map(|n| crate::app_protocol::AiTool {
                name: n.to_string(),
                description: format!("test tool {n}"),
                input_schema: serde_json::json!({"type": "object", "properties": {}}),
                output_schema: serde_json::json!({"type": "object"}),
                timeout_ms: Some(2_000),
                read_only: false,
            })
            .collect();
        tool_dispatch::register(
            pane_id,
            "test-app".to_string(),
            tools,
            AppEventSender::Channel(tx),
            ws,
        );
        std::thread::spawn(move || {
            while let Ok(item) = rx.recv() {
                let json = item;
                let Ok(value) = serde_json::from_str::<serde_json::Value>(&json) else {
                    continue;
                };
                if value.get("type").and_then(|t| t.as_str()) != Some("tool_call") {
                    continue;
                }
                if let Some(call_id) = value.get("call_id").and_then(|c| c.as_str()) {
                    tool_dispatch::resolve_pending(
                        call_id,
                        ToolCallResult {
                            output_json: Some("{\"ok\": true}".to_string()),
                            error: None,
                        },
                    );
                }
            }
        });
    }

    /// Persist a grant for the active connector agent directly into the store.
    fn seed_grant(app: &mut AssistantApp, target: &str, decision: Decision) {
        let (actor_id, actor_scope) = app.connector_actor();
        let workspace_root = app
            .workspace_root
            .canonicalize()
            .unwrap_or_else(|_| app.workspace_root.clone());
        app.grant_store.record(GrantRecord {
            actor_type: ActorType::Agent,
            actor_id,
            actor_scope,
            workspace_root: Some(workspace_root),
            target_type: TargetType::AppConnector,
            target_id: target.to_string(),
            resource_scope: ResourceScope::Workspace,
            resource_id: None,
            decision,
            duration: GrantDuration::Always,
            source: GrantSource::User,
            created_at: 0,
            expires_at: None,
        });
    }

    fn pump_until(app: &mut AssistantApp, what: &str, cond: impl Fn(&AssistantApp) -> bool) {
        let start = std::time::Instant::now();
        while !cond(app) {
            app.pump_turn_io();
            assert!(
                start.elapsed() < std::time::Duration::from_secs(5),
                "timeout waiting for {what}"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// Dispatch `tool` on a worker thread (the gate blocks there, never on
    /// the test thread). Returns the result receiver.
    fn dispatch_on_worker(dispatcher: Arc<ToolDispatcher>, tool: &str) -> Receiver<ToolCallResult> {
        dispatch_on_worker_with_input(dispatcher, tool, "{\"x\": 1}")
    }

    fn dispatch_on_worker_with_input(
        dispatcher: Arc<ToolDispatcher>,
        tool: &str,
        input_json: &str,
    ) -> Receiver<ToolCallResult> {
        let (tx, rx) = std::sync::mpsc::channel();
        let tool = tool.to_string();
        let input_json = input_json.to_string();
        let call_id = format!("call-{}", uuid::Uuid::new_v4());
        std::thread::spawn(move || {
            let result = dispatcher.dispatch_call(call_id, &tool, input_json);
            let _ = tx.send(result);
        });
        rx
    }

    #[test]
    fn connector_filtering_denied_invisible_ask_and_allow_visible() {
        let ws = tempfile::tempdir().unwrap();
        let mut app = test_app(ws.path());
        register_echo_provider(
            9100,
            &["t_allow", "t_ask", "t_deny"],
            ws.path().to_path_buf(),
        );
        seed_grant(&mut app, "app.t_allow", Decision::Allow);
        seed_grant(&mut app, "app.t_deny", Decision::Deny);
        // t_ask has no grant → default Ask.

        let dispatcher = app.gated_dispatcher();
        let mut visible: Vec<String> = dispatcher.all_tools().into_iter().map(|t| t.name).collect();
        visible.sort();
        let connectors = visible
            .iter()
            .filter(|name| name.starts_with("t_"))
            .map(String::as_str)
            .collect::<Vec<_>>();
        assert_eq!(
            connectors,
            vec!["t_allow", "t_ask"],
            "denied connector must be invisible"
        );
        assert!(visible.iter().any(|name| name == HOST_TOOL_PANES_LIST));
        assert!(visible.iter().any(|name| name == HOST_TOOL_SUBSCRIBE));

        // Denied tool is also uninvocable.
        let result = dispatcher.dispatch_call("c-deny".to_string(), "t_deny", "{}".to_string());
        assert!(
            result
                .error
                .as_deref()
                .unwrap_or("")
                .contains("tool_not_found"),
            "denied tool must be uninvocable: {:?}",
            result.error
        );

        tool_dispatch::unregister(9100);
    }

    #[test]
    fn ask_allow_once_runs_tool_without_persisting_a_grant() {
        let ws = tempfile::tempdir().unwrap();
        let mut app = test_app(ws.path());
        register_echo_provider(9101, &["gated.tool"], ws.path().to_path_buf());

        let dispatcher = Arc::new(app.gated_dispatcher());
        let result_rx = dispatch_on_worker(dispatcher, "gated.tool");

        pump_until(&mut app, "permission sheet", |a| {
            a.model.pending_permission.is_some()
        });
        assert_eq!(
            app.model.pending_permission.as_ref().unwrap().tool,
            "gated.tool"
        );
        app.resolve_permission(PermissionChoice::AllowOnce);

        let result = result_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("tool call must complete");
        assert!(
            result.error.is_none(),
            "allowed call must succeed: {:?}",
            result.error
        );

        // The completed tool row lands in the transcript.
        pump_until(&mut app, "tool row", |a| {
            a.model
                .turns
                .iter()
                .any(|t| t.status == Some(ToolStatus::Succeeded))
        });
        // Allow-once persists nothing.
        assert!(
            app.grant_store.records().is_empty(),
            "allow-once must not persist a grant"
        );
        // Audit: one permission decision + one tool call.
        let events = app.audit.tail(10);
        assert_eq!(
            events.len(),
            2,
            "audit must record decision + call: {events:?}"
        );
        assert_eq!(events[0].kind, "permission_decision");
        assert_eq!(events[0].decision, "allow_once");
        assert_eq!(events[1].kind, "tool_call");
        assert_eq!(events[1].decision, "ok");

        tool_dispatch::unregister(9101);
    }

    #[test]
    fn ask_allow_always_persists_grant_and_next_call_skips_sheet() {
        let ws = tempfile::tempdir().unwrap();
        let mut app = test_app(ws.path());
        register_echo_provider(9102, &["gated.tool"], ws.path().to_path_buf());

        let dispatcher = Arc::new(app.gated_dispatcher());
        let result_rx = dispatch_on_worker(dispatcher, "gated.tool");
        pump_until(&mut app, "permission sheet", |a| {
            a.model.pending_permission.is_some()
        });
        app.resolve_permission(PermissionChoice::AllowAlways);
        let result = result_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("tool call must complete");
        assert!(result.error.is_none());

        // Grant persisted in the store and on disk.
        let record = app
            .grant_store
            .records()
            .iter()
            .find(|r| r.target_id == "app.gated.tool")
            .expect("allow-always must persist a grant");
        assert_eq!(record.decision, Decision::Allow);
        assert_eq!(record.duration, GrantDuration::Always);
        assert!(
            ws.path().join("grants.toml").is_file(),
            "grant must be saved to disk"
        );

        // Next turn's dispatcher: the tool now evaluates Allow — no sheet.
        let dispatcher2 = Arc::new(app.gated_dispatcher());
        let result_rx2 = dispatch_on_worker(dispatcher2, "gated.tool");
        let result2 = result_rx2
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("granted call must complete without a sheet");
        assert!(result2.error.is_none());
        app.pump_turn_io();
        assert!(
            app.model.pending_permission.is_none(),
            "granted tool must not show a sheet"
        );

        tool_dispatch::unregister(9102);
    }

    #[test]
    fn ask_denied_by_user_returns_tool_error_not_a_crash() {
        let ws = tempfile::tempdir().unwrap();
        let mut app = test_app(ws.path());
        register_echo_provider(9103, &["gated.tool"], ws.path().to_path_buf());

        let dispatcher = Arc::new(app.gated_dispatcher());
        let result_rx = dispatch_on_worker(dispatcher, "gated.tool");
        pump_until(&mut app, "permission sheet", |a| {
            a.model.pending_permission.is_some()
        });
        app.resolve_permission(PermissionChoice::Deny);

        let result = result_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("denied call must still return");
        assert!(
            result
                .error
                .as_deref()
                .unwrap_or("")
                .contains("permission_denied"),
            "denial must be a tool error for the model: {:?}",
            result.error
        );
        // The denial lands as a failed tool row and an audit line.
        let row = app.model.turns.last().expect("denied row must exist");
        assert_eq!(row.status, Some(ToolStatus::Failed));
        assert!(row.text.contains("denied by user"));
        let events = app.audit.tail(10);
        assert_eq!(events[0].kind, "permission_decision");
        assert_eq!(events[0].decision, "deny");
        assert!(
            app.grant_store.records().is_empty(),
            "deny persists nothing"
        );

        tool_dispatch::unregister(9103);
    }

    #[test]
    fn slash_views_answer_with_info_rows() {
        let ws = tempfile::tempdir().unwrap();
        let mut app = test_app(ws.path());
        register_echo_provider(9104, &["view.tool"], ws.path().to_path_buf());
        seed_grant(&mut app, "app.view.tool", Decision::Allow);

        app.model.composer = "/tools".to_string();
        let effects = app.model.submit();
        app.execute_effects(effects);
        let tools_row = &app.model.turns.last().unwrap().text;
        assert!(tools_row.contains("app.view.tool — allow"), "{tools_row}");

        // `/permissions` now opens the interactive manager, populated from the
        // grant store, instead of dumping text.
        app.model.composer = "/permissions".to_string();
        let effects = app.model.submit();
        app.execute_effects(effects);
        let AssistantOverlay::PermissionsManager { grants, .. } = &app.model.overlay else {
            panic!("expected the permissions manager overlay to be open");
        };
        assert!(
            grants.iter().any(|g| g.target_id == "app.view.tool"),
            "seeded grant is listed: {grants:?}"
        );
        app.model.cancel_overlay();

        // `/revoke <target_id>` keeps its fast-path text behavior.
        app.model.composer = "/revoke app.view.tool".to_string();
        let effects = app.model.submit();
        app.execute_effects(effects);
        assert!(app
            .model
            .turns
            .last()
            .unwrap()
            .text
            .contains("Revoked 1 grant(s)"));
        assert!(app.grant_store.records().is_empty());

        app.model.composer = "/audit".to_string();
        let effects = app.model.submit();
        app.execute_effects(effects);
        let audit_row = &app.model.turns.last().unwrap().text;
        assert!(
            audit_row.contains("revoke"),
            "revoke must be audited: {audit_row}"
        );

        tool_dispatch::unregister(9104);
    }

    #[test]
    fn permissions_manager_set_to_block_persists_and_audits() {
        let ws = tempfile::tempdir().unwrap();
        let mut app = test_app(ws.path());
        seed_grant(&mut app, "app.view.tool", Decision::Allow);

        app.model.composer = "/permissions".to_string();
        let effects = app.model.submit();
        app.execute_effects(effects);

        // Cycle the row Allow -> Ask -> Deny, then apply.
        app.model.overlay_cycle_decision();
        app.model.overlay_cycle_decision();
        app.confirm_overlay();

        assert!(!app.model.overlay_active());
        let record = app
            .grant_store
            .records()
            .iter()
            .find(|r| r.target_id == "app.view.tool")
            .expect("grant still persisted after set-to-block");
        assert_eq!(record.decision, Decision::Deny);

        let audited = app
            .audit
            .tail(5)
            .into_iter()
            .any(|e| e.kind == "permission_set" && e.target == "app.view.tool" && e.decision == "deny");
        assert!(audited, "block change must be audited");
    }

    #[test]
    fn permissions_manager_set_to_ask_revokes_and_audits() {
        let ws = tempfile::tempdir().unwrap();
        let mut app = test_app(ws.path());
        seed_grant(&mut app, "app.view.tool", Decision::Allow);

        // `/revoke` (no args) opens the same manager as `/permissions`.
        app.model.composer = "/revoke".to_string();
        let effects = app.model.submit();
        app.execute_effects(effects);
        assert!(matches!(
            app.model.overlay,
            AssistantOverlay::PermissionsManager { .. }
        ));

        // Allow -> Ask reverts to the posture default: the record is deleted.
        app.model.overlay_cycle_decision();
        app.confirm_overlay();

        assert!(
            app.grant_store.records().is_empty(),
            "set-to-ask removes the persisted grant"
        );
        let audited = app
            .audit
            .tail(5)
            .into_iter()
            .any(|e| e.kind == "revoke" && e.target == "app.view.tool");
        assert!(audited, "revert-to-ask must be audited");
    }

    #[test]
    fn permissions_manager_cancel_leaves_grants_untouched() {
        let ws = tempfile::tempdir().unwrap();
        let mut app = test_app(ws.path());
        seed_grant(&mut app, "app.view.tool", Decision::Allow);

        app.model.composer = "/permissions".to_string();
        let effects = app.model.submit();
        app.execute_effects(effects);

        app.model.overlay_cycle_decision(); // edit in-flight, then cancel
        app.model.cancel_overlay();

        assert!(!app.model.overlay_active());
        let record = app
            .grant_store
            .records()
            .iter()
            .find(|r| r.target_id == "app.view.tool")
            .expect("grant untouched after cancel");
        assert_eq!(record.decision, Decision::Allow);
    }

    // ── Phase D3: event subscriptions + delivery bridge ───────────────────────

    use crate::app_protocol::{AppEventActor, EventStreamDecl};
    use crate::host::app_timeline::EmittedEvent;

    fn chess_timeline() -> Arc<Mutex<AppTimeline>> {
        let timeline = Arc::new(Mutex::new(AppTimeline::default()));
        timeline
            .lock()
            .unwrap()
            .declare_streams(
                "chess",
                vec![EventStreamDecl {
                    name: "move.played".to_string(),
                    schema: serde_json::json!({"type": "object"}),
                    description: None,
                }],
            )
            .unwrap();
        timeline
    }

    fn test_app_with_timeline(ws: &Path, timeline: Arc<Mutex<AppTimeline>>) -> AssistantApp {
        AssistantApp::new_with_timeline(ws.to_path_buf(), MockBroker::ok("echo: ok"), ws, timeline)
    }

    fn emit_move(
        timeline: &Arc<Mutex<AppTimeline>>,
        actor: AppEventActor,
        actor_id: Option<&str>,
        caused_by: Option<&str>,
    ) -> usize {
        timeline
            .lock()
            .unwrap()
            .record_event(
                "chess",
                1,
                EmittedEvent {
                    event: "move.played".to_string(),
                    actor,
                    actor_id: actor_id.map(str::to_string),
                    caused_by: caused_by.map(str::to_string),
                    summary: "White played e4".to_string(),
                    resource_id: "game-1".to_string(),
                    resource_scope: Some("game".to_string()),
                    revision_after: "rev-2".to_string(),
                    payload: Some(serde_json::json!({"san": "e4"})),
                    state_ref: None,
                    revision_before: None,
                    rollback_token: None,
                    changed_resources: vec![],
                    suggested_trigger: None,
                },
            )
            .expect("event must record")
            .deliveries_queued
    }

    fn event_rows(app: &AssistantApp) -> usize {
        app.model
            .turns
            .iter()
            .filter(|t| t.role == TurnRole::Event)
            .count()
    }

    #[test]
    fn persisted_stream_grant_resubscribes_and_user_event_auto_triggers_turn() {
        let ws = tempfile::tempdir().unwrap();
        let timeline = chess_timeline();
        // Persist an event-stream grant, as allow-always would.
        {
            let mut first = test_app_with_timeline(ws.path(), timeline.clone());
            assert!(first.live_subs.is_empty(), "no grant yet = no subscription");
            first.record_assistant_grant(
                TargetType::AppEventStream,
                "chess::move.played",
                GrantDuration::Always,
                GrantSource::User,
            );
            first.grant_store.save();
        }
        assert!(ws.path().join("grants.toml").is_file());

        // Restart: a fresh AssistantApp over the same store resubscribes.
        let mut app = test_app_with_timeline(ws.path(), timeline.clone());
        assert_eq!(app.live_subs.len(), 1, "persisted grant must resubscribe");
        assert_eq!(timeline.lock().unwrap().subscriptions().len(), 1);

        // A user-actor event lands as a visible row and auto-starts a turn.
        assert_eq!(emit_move(&timeline, AppEventActor::User, None, None), 1);
        app.pump_turn_io();
        assert_eq!(event_rows(&app), 1);
        let row = app
            .model
            .turns
            .iter()
            .find(|t| t.role == TurnRole::Event)
            .unwrap();
        assert!(row.text.contains("chess: move.played"), "{}", row.text);
        assert!(row.text.contains("White played e4"), "{}", row.text);
        assert!(
            app.model.streaming.in_flight,
            "user event must auto-start a turn"
        );
        wait_for_turn(&mut app);
        assert_eq!(app.model.turns.last().unwrap().role, TurnRole::Assistant);
        assert_eq!(app.model.turns.last().unwrap().text, "echo: ok");

        // The event row is persisted: reopen and find it.
        drop(app);
        let reopened = test_app_with_timeline(ws.path(), timeline);
        assert_eq!(event_rows(&reopened), 1, "event row must persist");
    }

    #[test]
    fn reopen_never_duplicates_subscriptions_or_deliveries() {
        let ws = tempfile::tempdir().unwrap();
        let timeline = chess_timeline();

        // First instance subscribes live and records the grant, then the
        // pane closes without unsubscribing (the leak this guards against).
        {
            let mut first = test_app_with_timeline(ws.path(), timeline.clone());
            first.subscribe_stream("chess", "*");
            first.grant_store.record(GrantRecord {
                actor_type: ActorType::Agent,
                actor_id: ASSISTANT_ACTOR_ID.to_string(),
                actor_scope: ActorScope::BuiltIn,
                workspace_root: Some(ws.path().canonicalize().unwrap()),
                target_type: TargetType::AppEventStream,
                target_id: "chess::*".to_string(),
                resource_scope: ResourceScope::Workspace,
                resource_id: None,
                decision: Decision::Allow,
                duration: GrantDuration::Always,
                source: GrantSource::User,
                created_at: 0,
                expires_at: None,
            });
            first.grant_store.save();
        }
        assert_eq!(timeline.lock().unwrap().subscriptions().len(), 1);
        // An event lands while no pane is open: queued against the leaked sub.
        emit_move(&timeline, AppEventActor::User, None, None);
        assert_eq!(timeline.lock().unwrap().pending_delivery_count(), 1);

        // Reopen: leaked subscription + stale delivery are cleared, exactly
        // one live subscription remains.
        let mut app = test_app_with_timeline(ws.path(), timeline.clone());
        assert_eq!(timeline.lock().unwrap().subscriptions().len(), 1);
        assert_eq!(app.live_subs.len(), 1);
        assert_eq!(
            timeline.lock().unwrap().pending_delivery_count(),
            0,
            "stale deliveries from the leaked subscription must be dropped"
        );

        // A fresh event is delivered exactly once.
        assert_eq!(emit_move(&timeline, AppEventActor::User, None, None), 1);
        app.pump_turn_io();
        assert_eq!(event_rows(&app), 1, "one event row, not duplicates");
    }

    /// A granted assistant subscribe records a timeline `SubscriptionRecord`
    /// stamped with the assistant's identity — the same shape the CLI/MCP
    /// transports produce, now that all three share `record_subscription`.
    #[test]
    fn subscribe_stream_records_assistant_subscription_shape() {
        let ws = tempfile::tempdir().unwrap();
        let timeline = chess_timeline();
        let mut app = test_app_with_timeline(ws.path(), timeline.clone());
        app.subscribe_stream("chess", "move.played");

        let guard = timeline.lock().unwrap();
        let subs = guard.subscriptions();
        assert_eq!(subs.len(), 1);
        let record = &subs[0];
        assert_eq!(record.subscriber_type, ActorType::Agent);
        assert_eq!(record.subscriber_id, ASSISTANT_ACTOR_ID);
        assert_eq!(record.app_id, "chess");
        assert_eq!(record.event_names, vec!["move.played".to_string()]);

        // `*` subscribes to all declared streams → empty event_names.
        drop(guard);
        app.subscribe_stream("chess", "*");
        let guard = timeline.lock().unwrap();
        let star = guard
            .subscriptions()
            .iter()
            .find(|r| r.event_names.is_empty())
            .expect("`*` subscribe must record an all-streams subscription");
        assert_eq!(star.app_id, "chess");
        assert_eq!(star.subscriber_id, ASSISTANT_ACTOR_ID);
    }

    #[test]
    fn self_caused_deliveries_record_rows_without_turns() {
        let ws = tempfile::tempdir().unwrap();
        let timeline = chess_timeline();
        let mut app = test_app_with_timeline(ws.path(), timeline.clone());
        app.subscribe_stream("chess", "move.played");

        // The assistant as the event actor: row, no turn.
        emit_move(
            &timeline,
            AppEventActor::Agent,
            Some("agent:assistant"),
            None,
        );
        app.pump_turn_io();
        assert_eq!(event_rows(&app), 1);
        assert!(
            !app.model.streaming.in_flight,
            "own action must not trigger"
        );

        // App-emitted event caused by the assistant's tool call: row, no turn.
        emit_move(&timeline, AppEventActor::App, None, Some("agent:assistant"));
        app.pump_turn_io();
        assert_eq!(event_rows(&app), 2);
        assert!(
            !app.model.streaming.in_flight,
            "caused-by-self must not trigger"
        );
        assert!(app.queued_event_lines.is_empty());
    }

    #[test]
    fn event_during_in_flight_turn_is_queued_into_next_turn() {
        let ws = tempfile::tempdir().unwrap();
        let timeline = chess_timeline();
        let mut app = test_app_with_timeline(ws.path(), timeline.clone());
        app.subscribe_stream("chess", "move.played");

        // Simulate an in-flight turn; the event must queue, not dispatch.
        app.model.streaming.in_flight = true;
        emit_move(&timeline, AppEventActor::User, None, None);
        app.pump_turn_io();
        assert_eq!(event_rows(&app), 1, "row appended even while in flight");
        assert_eq!(app.queued_event_lines.len(), 1);
        assert!(
            app.turn_cancel.is_cancelled(),
            "a mid-turn event must preempt the in-flight turn, not just queue behind it"
        );

        // The turn ends: the queued event triggers the follow-up turn.
        app.model.streaming = model::StreamingState::default();
        app.pump_turn_io();
        assert!(
            app.model.streaming.in_flight,
            "queued event must start the next turn"
        );
        assert!(app.queued_event_lines.is_empty());
        // The event line is folded into the dispatched history.
        let history = app.history_messages();
        assert!(
            history
                .iter()
                .any(|m| m.role == "user" && m.content.contains("App event delivered")),
            "event line must be in the turn history"
        );
        wait_for_turn(&mut app);
        assert_eq!(app.model.turns.last().unwrap().text, "echo: ok");
    }

    /// ESC mid-turn with a pending draft: the in-flight turn is cancelled and
    /// the draft is folded into an immediate follow-up turn. The cancelled
    /// turn commits no empty bubble.
    #[test]
    fn esc_interrupt_folds_queued_draft_into_followup() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        /// Turn 1 blocks until the cancel token trips (simulating a long
        /// stream the user ESCs); the folded follow-up replies immediately.
        struct InterruptibleBroker {
            dispatched: AtomicUsize,
        }
        impl AiBroker for InterruptibleBroker {
            fn dispatch(
                &self,
                request: AiBrokerRequest,
                on_delta: &mut dyn FnMut(TurnDelta<'_>),
            ) -> AiBrokerResponse {
                let n = self.dispatched.fetch_add(1, Ordering::SeqCst) + 1;
                if n == 1 {
                    let start = std::time::Instant::now();
                    while !request.cancel.is_cancelled() {
                        if start.elapsed() > std::time::Duration::from_secs(5) {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(1));
                    }
                    // Cancelled before any text streamed: empty partial.
                    return AiBrokerResponse::ok(String::new(), 0, 0);
                }
                on_delta(TurnDelta::Text("folded reply"));
                AiBrokerResponse::ok("folded reply".to_string(), 1, 1)
            }
        }

        let ws = tempfile::tempdir().unwrap();
        let mut app = AssistantApp::new(
            ws.path().to_path_buf(),
            Arc::new(InterruptibleBroker {
                dispatched: AtomicUsize::new(0),
            }),
            ws.path(),
        );

        // Turn 1 starts and parks in the broker.
        app.model.composer = "start".to_string();
        let effects = app.model.submit();
        app.execute_effects(effects);
        assert!(app.model.streaming.in_flight, "turn 1 must be in flight");

        // User types a follow-up and hits ESC.
        app.model.composer = "actually do X".to_string();
        app.interrupt_in_flight_turn();
        assert!(
            app.turn_cancel.is_cancelled(),
            "ESC must trip the cancel token"
        );
        assert_eq!(
            app.model.queued_user_turns, 1,
            "the draft must be queued for the fold"
        );

        // Drive to idle: turn 1 unblocks (cancelled), the fold dispatches turn 2.
        let start = std::time::Instant::now();
        loop {
            app.pump_turn_io();
            let idle = !app.model.streaming.in_flight
                && app.queued_event_lines.is_empty()
                && app.model.queued_user_turns == 0;
            if idle {
                break;
            }
            assert!(
                start.elapsed() < std::time::Duration::from_secs(8),
                "assistant never reached idle after interrupt"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }

        // The folded follow-up replied; the draft is in the transcript.
        assert_eq!(app.model.turns.last().unwrap().text, "folded reply");
        assert!(
            app.model
                .turns
                .iter()
                .any(|t| t.role == TurnRole::User && t.text == "actually do X"),
            "the interrupted-and-folded draft must be in the transcript"
        );
        // The cancelled turn left no empty assistant bubble.
        let empty_assistant = app
            .model
            .turns
            .iter()
            .filter(|t| t.role == TurnRole::Assistant && t.text.is_empty())
            .count();
        assert_eq!(
            empty_assistant, 0,
            "a cancelled turn must not commit an empty assistant bubble"
        );
    }

    #[test]
    fn subscribe_host_tool_ask_allow_always_persists_grant_and_revoke_unsubscribes() {
        let ws = tempfile::tempdir().unwrap();
        let timeline = chess_timeline();
        let mut app = test_app_with_timeline(ws.path(), timeline.clone());

        // Host event tools are visible in the turn snapshot.
        let dispatcher = Arc::new(app.gated_dispatcher());
        let names: HashSet<String> = dispatcher.all_tools().into_iter().map(|t| t.name).collect();
        assert!(names.contains(HOST_TOOL_SUBSCRIBE));
        assert!(names.contains(HOST_TOOL_UNSUBSCRIBE));

        // Ungranted subscribe asks via the permission sheet.
        let result_rx = dispatch_on_worker_with_input(
            dispatcher,
            HOST_TOOL_SUBSCRIBE,
            r#"{"app": "chess", "event": "move.played"}"#,
        );
        pump_until(&mut app, "subscribe permission sheet", |a| {
            a.model.pending_permission.is_some()
        });
        let pending = app.model.pending_permission.as_ref().unwrap();
        assert_eq!(pending.tool, HOST_TOOL_SUBSCRIBE);
        assert_eq!(pending.input_summary, "chess::move.played");

        app.resolve_permission(PermissionChoice::AllowAlways);
        let result = result_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("subscribe must complete");
        assert!(result.error.is_none(), "{:?}", result.error);

        // Grant persisted on disk; live subscription created.
        let record = app
            .grant_store
            .records()
            .iter()
            .find(|r| r.target_id == "chess::move.played")
            .expect("allow-always must persist the stream grant");
        assert_eq!(record.target_type, TargetType::AppEventStream);
        assert_eq!(record.duration, GrantDuration::Always);
        assert!(ws.path().join("grants.toml").is_file());
        assert_eq!(app.live_subs.len(), 1);
        assert_eq!(timeline.lock().unwrap().subscriptions().len(), 1);

        // Deliveries now flow and trigger turns.
        assert_eq!(emit_move(&timeline, AppEventActor::User, None, None), 1);
        app.pump_turn_io();
        assert_eq!(event_rows(&app), 1);
        wait_for_turn(&mut app);

        // /revoke removes the grant AND the live subscription.
        app.model.composer = "/revoke chess::move.played".to_string();
        let effects = app.model.submit();
        app.execute_effects(effects);
        assert!(
            app.model
                .turns
                .last()
                .unwrap()
                .text
                .contains("live subscription"),
            "{}",
            app.model.turns.last().unwrap().text
        );
        assert!(app
            .grant_store
            .records()
            .iter()
            .all(|r| r.target_type != TargetType::AppEventStream));
        assert!(app.live_subs.is_empty());
        assert!(timeline.lock().unwrap().subscriptions().is_empty());

        // No further deliveries reach the assistant.
        assert_eq!(emit_move(&timeline, AppEventActor::User, None, None), 0);
        app.pump_turn_io();
        assert_eq!(event_rows(&app), 1, "revoked stream must deliver nothing");
    }

    #[test]
    fn subscribe_denied_by_user_returns_tool_error_without_subscription() {
        let ws = tempfile::tempdir().unwrap();
        let timeline = chess_timeline();
        let mut app = test_app_with_timeline(ws.path(), timeline.clone());

        let dispatcher = Arc::new(app.gated_dispatcher());
        let result_rx = dispatch_on_worker_with_input(
            dispatcher,
            HOST_TOOL_SUBSCRIBE,
            r#"{"app": "chess", "event": "move.played"}"#,
        );
        pump_until(&mut app, "subscribe permission sheet", |a| {
            a.model.pending_permission.is_some()
        });
        app.resolve_permission(PermissionChoice::Deny);
        let result = result_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("denied subscribe must still return");
        assert!(
            result
                .error
                .as_deref()
                .unwrap_or("")
                .contains("permission_denied"),
            "{:?}",
            result.error
        );
        assert!(app.live_subs.is_empty());
        assert!(timeline.lock().unwrap().subscriptions().is_empty());
        assert!(
            app.grant_store.records().is_empty(),
            "deny persists nothing"
        );
    }

    #[test]
    fn resolved_settings_deny_blocks_event_subscription() {
        let ws = tempfile::tempdir().unwrap();
        let settings_path = ws.path().join("agents/settings.toml");
        std::fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
        std::fs::write(
            settings_path,
            "[permissions]\ndeny = [\"chess::move.played\"]\n",
        )
        .unwrap();
        let timeline = chess_timeline();
        let mut app = test_app_with_timeline(ws.path(), timeline.clone());
        let (tx, rx) = std::sync::mpsc::sync_channel(1);

        app.handle_host_call(
            HOST_TOOL_SUBSCRIBE,
            r#"{"app": "chess", "event": "move.played"}"#,
            tx,
        );

        let result = rx.try_recv().expect("settings deny replies synchronously");
        assert!(result
            .error
            .as_deref()
            .unwrap_or("")
            .contains("permission_denied"));
        assert!(app.live_subs.is_empty());
        assert!(timeline.lock().unwrap().subscriptions().is_empty());
    }

    #[test]
    fn plan_posture_allows_read_only_event_subscription_when_rule_allows() {
        let ws = tempfile::tempdir().unwrap();
        let settings_path = ws.path().join("agents/settings.toml");
        std::fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
        std::fs::write(
            settings_path,
            "[permissions]\ndefault_posture = \"plan\"\nallow = [\"chess::move.played\"]\n",
        )
        .unwrap();
        let timeline = chess_timeline();
        let mut app = test_app_with_timeline(ws.path(), timeline.clone());
        let (tx, rx) = std::sync::mpsc::sync_channel(1);

        app.handle_host_call(
            HOST_TOOL_SUBSCRIBE,
            r#"{"app": "chess", "event": "move.played"}"#,
            tx,
        );

        let result = rx
            .try_recv()
            .expect("an allowed subscription replies synchronously");
        assert!(result.error.is_none(), "{:?}", result.error);
        assert_eq!(app.live_subs.len(), 1);
        assert_eq!(timeline.lock().unwrap().subscriptions().len(), 1);
    }

    #[test]
    fn tools_view_reports_resolved_event_stream_decision() {
        let ws = tempfile::tempdir().unwrap();
        let settings_path = ws.path().join("agents/settings.toml");
        std::fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
        std::fs::write(
            settings_path,
            "[permissions]\ndeny = [\"chess::move.played\"]\n",
        )
        .unwrap();
        let mut app = test_app_with_timeline(ws.path(), chess_timeline());

        app.model.composer = "/tools".to_string();
        let effects = app.model.submit();
        app.execute_effects(effects);

        assert!(app
            .model
            .turns
            .last()
            .unwrap()
            .text
            .contains("chess::move.played — deny"));
    }

    #[test]
    fn host_unsubscribe_removes_live_subscription() {
        let ws = tempfile::tempdir().unwrap();
        let timeline = chess_timeline();
        let mut app = test_app_with_timeline(ws.path(), timeline.clone());
        app.subscribe_stream("chess", "move.played");

        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        app.handle_host_call(
            HOST_TOOL_UNSUBSCRIBE,
            r#"{"app": "chess", "event": "move.played"}"#,
            tx,
        );
        let result = rx.try_recv().expect("unsubscribe replies synchronously");
        assert!(result.error.is_none(), "{:?}", result.error);
        assert!(app.live_subs.is_empty());
        assert!(timeline.lock().unwrap().subscriptions().is_empty());

        // Unsubscribing again is a named error, not a crash.
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        app.handle_host_call(
            HOST_TOOL_UNSUBSCRIBE,
            r#"{"app": "chess", "event": "move.played"}"#,
            tx,
        );
        let result = rx.try_recv().unwrap();
        assert!(result
            .error
            .as_deref()
            .unwrap_or("")
            .contains("not_subscribed"));
    }

    #[test]
    fn rename_persists_session_name() {
        let ws = tempfile::tempdir().unwrap();
        let mut app = AssistantApp::new(ws.path().to_path_buf(), MockBroker::ok("ok"), ws.path());
        assert_eq!(app.model.session_name, None);
        app.on_pane_renamed("My Session");
        assert_eq!(app.model.session_name.as_deref(), Some("My Session"));
        // Reopen: name persists.
        drop(app);
        let reopened = AssistantApp::new(ws.path().to_path_buf(), MockBroker::ok("ok"), ws.path());
        assert_eq!(reopened.model.session_name.as_deref(), Some("My Session"));
    }

    #[test]
    fn selected_agent_and_effort_persist_when_conversation_reopens() {
        let ws = tempfile::tempdir().unwrap();
        let mut app = test_app(ws.path());
        app.cmd_create_agent("writer");
        app.cmd_switch_agent("writer");
        app.set_session_effort(Some(ReasoningEffort::High));

        drop(app);
        let reopened = test_app(ws.path());

        assert_eq!(reopened.model.active_agent_id, "writer");
        assert_eq!(reopened.model.effort_override, Some(ReasoningEffort::High));
    }

    #[test]
    fn error_turn_appends_error_row() {
        let ws = tempfile::tempdir().unwrap();
        let mut app = AssistantApp::new(ws.path().to_path_buf(), MockBroker::error(), ws.path());
        app.model.composer = "trigger error".to_string();
        let effects = app.model.submit();
        app.execute_effects(effects);
        wait_for_turn(&mut app);
        assert_eq!(app.model.turns.last().unwrap().role, TurnRole::Error);
    }

    #[test]
    fn model_command_changes_session_scope_without_writing_settings_files() {
        let ws = tempfile::tempdir().unwrap();
        let user_path = ws.path().join("agents/settings.toml");
        let workspace_agents = ws
            .path()
            .join(crate::config::workspace_channel_dir())
            .join("agents");
        let workspace_path = workspace_agents.join("settings.toml");
        let local_path = workspace_agents.join("settings.local.toml");
        let mut app = test_app(ws.path());

        app.model.composer = "/model high".to_string();
        let effects = app.model.submit();
        app.execute_effects(effects);

        assert_eq!(app.session_overrides.model_tier, Some(ModelTier::High));
        assert_eq!(app.settings.model.tier.value, ModelTier::High);
        assert_eq!(
            app.settings.model.tier.source.scope,
            settings::SettingsScope::Session
        );
        assert!(app
            .model
            .turns
            .last()
            .unwrap()
            .text
            .contains("this session"));
        assert!(!user_path.is_file());
        assert!(!workspace_path.is_file());
        assert!(!local_path.is_file());
    }

    #[test]
    fn settings_load_errors_do_not_persist_as_conversation_turns() {
        let ws = tempfile::tempdir().unwrap();
        let settings_path = ws.path().join("agents/settings.toml");
        std::fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
        std::fs::write(&settings_path, "unknown = true\n").unwrap();

        let app = test_app(ws.path());
        assert_eq!(app.settings_errors.len(), 1);
        assert!(app.model.turns.is_empty());
        drop(app);

        std::fs::write(&settings_path, "[model]\ntier = \"low\"\n").unwrap();
        let reopened = test_app(ws.path());
        assert!(reopened.settings_errors.is_empty());
        assert!(reopened.model.turns.is_empty());
    }

    #[test]
    fn model_without_args_opens_picker_on_resolved_tier() {
        let ws = tempfile::tempdir().unwrap();
        let settings_path = ws.path().join("agents/settings.toml");
        std::fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
        std::fs::write(&settings_path, "[model]\ntier = \"low\"\n").unwrap();
        let mut app = test_app(ws.path());

        app.model.composer = "/model".to_string();
        let effects = app.model.submit();
        app.execute_effects(effects);

        // The picker opens with the cursor on the resolved tier (low, index 0)
        // and the current agent listed. No text row is pushed.
        assert!(app.model.turns.is_empty(), "picker opens instead of a text row");
        let AssistantOverlay::ModelPicker {
            selected,
            current_tier,
            tiers,
            agents,
            ..
        } = &app.model.overlay
        else {
            panic!("expected the model picker overlay to be open");
        };
        assert_eq!(*current_tier, ModelTier::Low);
        assert_eq!(*selected, 0);
        assert_eq!(tiers.len(), 3);
        assert!(
            agents.iter().any(|a| a.id == "default"),
            "the default agent is listed"
        );
    }

    #[test]
    fn model_picker_confirm_sets_the_selected_tier() {
        let ws = tempfile::tempdir().unwrap();
        let mut app = test_app(ws.path());

        app.model.composer = "/model".to_string();
        let effects = app.model.submit();
        app.execute_effects(effects);

        // Default resolves to `medium` (index 1); one step down lands on
        // `high` (index 2). Confirm follows the same path as `/model high`.
        assert_eq!(app.model.overlay_selected(), 1);
        app.model.overlay_move_down();
        assert_eq!(app.model.overlay_selected(), 2);
        app.confirm_overlay();

        assert!(!app.model.overlay_active(), "overlay closes on confirm");
        assert_eq!(app.session_overrides.model_tier, Some(ModelTier::High));
        assert!(app
            .model
            .turns
            .last()
            .unwrap()
            .text
            .contains("Model tier set to"));
    }

    #[test]
    fn settings_and_config_commands_show_the_same_resolved_view() {
        let ws = tempfile::tempdir().unwrap();
        let mut app = test_app(ws.path());

        app.model.composer = "/settings".to_string();
        let effects = app.model.submit();
        app.execute_effects(effects);
        let settings_output = app.model.turns.last().unwrap().text.clone();

        app.model.composer = "/config".to_string();
        let effects = app.model.submit();
        app.execute_effects(effects);
        let config_output = &app.model.turns.last().unwrap().text;

        assert_eq!(config_output, &settings_output);
    }

    #[test]
    fn settings_command_shows_tools_memory_and_hooks_with_sources() {
        let ws = tempfile::tempdir().unwrap();
        let settings_path = ws.path().join("agents/settings.toml");
        std::fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
        std::fs::write(
            &settings_path,
            "[tools]\nenabled = [\"host.panes.read\"]\n\
             [memory]\nenabled = true\n\
             [hooks]\nenabled = [\"turn.finished\"]\n",
        )
        .unwrap();
        let mut app = test_app(ws.path());

        app.model.composer = "/settings".to_string();
        let effects = app.model.submit();
        app.execute_effects(effects);

        let output = &app.model.turns.last().unwrap().text;
        assert!(output.contains("Enabled tools: `host.panes.read` (user"));
        assert!(output.contains("Memory: enabled (user"));
        assert!(output.contains("Enabled hooks: `turn.finished` (user"));
    }

    #[test]
    fn invalid_settings_toml_is_visible_without_preventing_startup() {
        let ws = tempfile::tempdir().unwrap();
        let settings_path = ws.path().join("agents/settings.toml");
        std::fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
        std::fs::write(&settings_path, "[model\ntier = nope").unwrap();

        let mut app = test_app(ws.path());
        assert_eq!(app.settings_errors.len(), 1);

        app.model.composer = "/settings".to_string();
        let effects = app.model.submit();
        app.execute_effects(effects);
        assert!(app
            .model
            .turns
            .last()
            .unwrap()
            .text
            .contains(settings_path.to_string_lossy().as_ref()));
    }

    #[test]
    fn reopening_with_the_same_settings_error_keeps_one_active_error() {
        let ws = tempfile::tempdir().unwrap();
        let settings_path = ws.path().join("agents/settings.toml");
        std::fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
        std::fs::write(&settings_path, "[model\ntier = nope").unwrap();

        drop(test_app(ws.path()));
        let reopened = test_app(ws.path());
        assert_eq!(reopened.settings_errors.len(), 1);
        assert!(reopened.model.turns.is_empty());
    }

    #[test]
    fn settings_read_error_is_visible_without_preventing_startup() {
        let ws = tempfile::tempdir().unwrap();
        let settings_path = ws.path().join("agents/settings.toml");
        std::fs::create_dir_all(&settings_path).unwrap();

        let mut app = test_app(ws.path());
        assert_eq!(app.settings_errors.len(), 1);

        app.model.composer = "/settings".to_string();
        let effects = app.model.submit();
        app.execute_effects(effects);
        assert!(app
            .model
            .turns
            .last()
            .unwrap()
            .text
            .contains(settings_path.to_string_lossy().as_ref()));
    }

    #[test]
    fn context_command_reports_open_pane_context() {
        let ws = tempfile::tempdir().unwrap();
        crate::plexi_ai::broker::update_pane_snapshot(vec![crate::plexi_ai::broker::PaneContext {
            type_id: "text-editor".to_string(),
            pane_id: 42,
        }]);
        let mut app = test_app(ws.path());
        app.model.composer = "/context".to_string();
        let effects = app.model.submit();
        app.execute_effects(effects);
        let output = &app.model.turns.last().unwrap().text;
        assert!(output.contains("Active workspace context"));
        assert!(output.contains("text-editor (pane 42)"));
    }

    #[test]
    fn background_tick_queues_native_host_calls_for_offscreen_assistant() {
        let ws = tempfile::tempdir().unwrap();
        let mut app = test_app(ws.path());
        let (reply, _reply_rx) = std::sync::mpsc::sync_channel(1);
        app.model.streaming.in_flight = true;
        app.flow_tx
            .send(ToolFlowEvent::HostCall {
                tool: HOST_TOOL_PANES_LIST.to_string(),
                input_json: "{}".to_string(),
                reply,
            })
            .unwrap();

        assert!(App::needs_background_tick(&app));
        App::background_tick(&mut app);
        let commands = App::take_pending_commands(&mut app);
        assert!(matches!(
            commands.as_slice(),
            [AppCommand::AssistantHostTool { name, .. }] if name == HOST_TOOL_PANES_LIST
        ));
    }

    #[test]
    fn active_ui_frame_drains_compaction_and_preserves_observable_checkpoint_state() {
        let ws = tempfile::tempdir().unwrap();
        let mut app = test_app(ws.path());
        for index in 0..7 {
            app.model
                .turns
                .push(model::Turn::now(TurnRole::User, format!("turn {index}")));
        }
        app.model.begin_compaction();
        app.compact_pending = true;

        let egui_ctx = egui::Context::default();
        let colors = crate::ui::theme::colors_from_config(&crate::config::PlexiConfig::default());
        let _ = egui_ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let render_ctx = AppRenderContext {
                    colors: &colors,
                    is_focused: true,
                };
                App::ui(&mut app, ui, &render_ctx);
            });
        });

        let model::CompactionState::Completed {
            compacted_turns,
            checkpoint_id,
        } = &app.model.compaction
        else {
            panic!("compaction state must remain observable after completion");
        };
        assert_eq!(*compacted_turns, 1);
        assert!(checkpoint_id.starts_with("checkpoint-"));
        assert_eq!(
            app.model.turns.last().unwrap().text,
            format!("Compacted 1 turns into checkpoint `{checkpoint_id}`.")
        );
    }

    #[test]
    fn unknown_slash_command_invokes_matching_installed_skill_with_args() {
        let ws = tempfile::tempdir().unwrap();
        let skill_path = ws.path().join(".plexi/skills/release/SKILL.md");
        std::fs::create_dir_all(skill_path.parent().unwrap()).unwrap();
        std::fs::write(
            &skill_path,
            "---\nname: release\ndescription: prepare a release\n---\nCHECK THE RELEASE CONTRACT",
        )
        .unwrap();
        let broker = Arc::new(CapturingBroker::default());
        let mut app = AssistantApp::new(ws.path().to_path_buf(), broker.clone(), ws.path());
        app.model.composer = "/release /Users/ian/project".to_string();
        let effects = app.model.submit();
        assert!(matches!(
            &effects[0],
            AssistantEffect::InvokeSkill { name, args }
                if name == "release" && args == "/Users/ian/project"
        ));
        app.execute_effects(effects);
        wait_for_turn(&mut app);
        let seen = broker.seen.lock().unwrap();
        assert!(seen[0].system.contains("CHECK THE RELEASE CONTRACT"));
        assert!(seen[0]
            .messages
            .iter()
            .any(|message| message.content == "/Users/ian/project"));
    }
}
