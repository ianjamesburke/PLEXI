//! Host Assistant — Phase 1 of `docs/prm/assistant-host-app.md`.
//!
//! The Assistant is a first-party host app: a `Pane::App(AppRuntime::Builtin)`
//! pane, not a PGAP process. It is split into pure state (`model`), slash
//! command parsing (`commands`), disk persistence (`store`), and egui
//! rendering (`render`); this module is the pane shell that wires those to
//! the `App` trait and runs model turns on worker threads — the same
//! dispatch-on-worker / outcome-channel pattern as `crate::agent::AgentHost`.

pub mod audit;
pub mod commands;
pub mod model;
pub mod render;
pub mod store;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, SyncSender};
use std::sync::{Arc, Mutex};

use crate::app::app_trait::{App, AppRenderContext};
use crate::app_protocol::{AiMessage, AiTool, ModelTier, PayloadMode, TriggerMode};
use crate::broker::{
    ActorScope, ActorType, Decision, GrantDuration, GrantRecord, GrantSource, GrantStore,
    PermissionRequest, ResourceScope, TargetType,
};
use crate::host::app_timeline::{AppTimeline, SubscriptionRecord};
use crate::plexi_ai::broker::{AiBroker, AiBrokerRequest};
use crate::plexi_ai::turn_loop::TurnDelta;

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
use model::{AssistantEffect, AssistantModel, PermissionChoice, TurnRole};
use render::{AssistantRenderer, ComposerEvent};
use store::AssistantStore;

const ASSISTANT_SYSTEM_PROMPT: &str = "You are the Plexi Assistant, the workspace \
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
Pane and terminal control arrives in a later phase — when \
asked to act on panes, explain that those tools are not wired up yet.";

/// Host tool names the Assistant injects into its dispatcher snapshot.
const HOST_TOOL_SUBSCRIBE: &str = "host.events.subscribe";
const HOST_TOOL_UNSUBSCRIBE: &str = "host.events.unsubscribe";

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
    Allow { remember: bool },
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
}

impl ToolCallHooks for AssistantToolHooks {
    fn before_call(&self, name: &str, input_json: &str) -> Result<(), String> {
        let needs_ask = self.ask_tools.contains(name)
            && !self.session_allowed.lock().unwrap().contains(name);
        if needs_ask {
            let (reply_tx, reply_rx) = std::sync::mpsc::sync_channel(1);
            self.flow_tx
                .send(ToolFlowEvent::Ask {
                    tool: name.to_string(),
                    input_json: input_json.to_string(),
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

/// The host Assistant pane: model + store + broker + grant wiring.
pub struct AssistantApp {
    pub(crate) model: AssistantModel,
    store: AssistantStore,
    broker: Arc<dyn AiBroker>,
    workspace_root: PathBuf,
    /// Unified broker grants (same `grants.toml` shape as `AgentHost`).
    /// Session grants are recorded in memory only; "always" grants are saved.
    grant_store: GrantStore,
    audit: AuditLog,
    /// How many of `model.turns` are already on disk for the active
    /// conversation. Reset when the conversation id changes.
    persisted_turns: usize,
    persisted_conversation: String,
    outcome_tx: Sender<TurnOutcome>,
    outcome_rx: Receiver<TurnOutcome>,
    /// Live deltas from the in-flight turn's worker thread.
    delta_rx: Option<Receiver<StreamDelta>>,
    flow_tx: Sender<ToolFlowEvent>,
    flow_rx: Receiver<ToolFlowEvent>,
    /// Reply channel for the worker blocked on the pending permission sheet.
    pending_reply: Option<SyncSender<PermissionReply>>,
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
        let persisted_turns = model.turns.len();
        let persisted_conversation = model.conversation_id.clone();
        let (outcome_tx, outcome_rx) = std::sync::mpsc::channel();
        let (flow_tx, flow_rx) = std::sync::mpsc::channel();
        let mut app = Self {
            model,
            store,
            broker,
            workspace_root,
            grant_store: GrantStore::load_or_default(profile_dir),
            audit: AuditLog::new(profile_dir.join("audit.jsonl")),
            persisted_turns,
            persisted_conversation,
            outcome_tx,
            outcome_rx,
            delta_rx: None,
            flow_tx,
            flow_rx,
            pending_reply: None,
            timeline,
            live_subs: Vec::new(),
            pending_subscribe: None,
            queued_event_lines: Vec::new(),
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
        let subscription_id = format!("assistant-sub-{}", uuid::Uuid::new_v4());
        self.timeline.lock().unwrap().add_subscription(SubscriptionRecord {
            subscription_id: subscription_id.clone(),
            subscriber_type: ActorType::Agent,
            subscriber_id: ASSISTANT_ACTOR_ID.to_string(),
            app_id: app.to_string(),
            event_names: if event == "*" {
                Vec::new()
            } else {
                vec![event.to_string()]
            },
            payload_mode: PayloadMode::Full,
            trigger_mode: TriggerMode::Conversation,
            resource_id: None,
            duration: GrantDuration::Session,
            created_at: crate::host::event_log::now_timestamp(),
        });
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
                AssistantEffect::ListPermissions => self.cmd_list_permissions(),
                AssistantEffect::RevokeGrant { target_id } => self.cmd_revoke(&target_id),
                AssistantEffect::ShowAudit => self.cmd_show_audit(),
                // Phase 3 stub: correctly shaped, logged, never panics.
                AssistantEffect::PaneAction { action } => {
                    log::info!("assistant: PaneAction '{action}' not yet implemented (Phase 3)");
                }
            }
        }
    }

    /// Broker target id for an app-exposed tool.
    fn connector_target(tool: &str) -> String {
        format!("app.{tool}")
    }

    /// Evaluate one app connector tool for the assistant actor.
    fn tool_decision(&self, tool: &str) -> Decision {
        let req = PermissionRequest::new(
            ActorType::Agent,
            ASSISTANT_ACTOR_ID,
            TargetType::AppConnector,
            &Self::connector_target(tool),
            Some(&self.workspace_root),
        );
        self.grant_store.evaluate(&req, None)
    }

    /// Build the broker-gated dispatcher for one turn: denied tools are
    /// stripped (invisible and uninvocable), ask tools stay visible behind
    /// the permission-sheet hook, allowed tools pass through.
    fn gated_dispatcher(&self) -> ToolDispatcher {
        let mut dispatcher = ToolDispatcher::from_registry(
            0,
            format!("agent:{ASSISTANT_ACTOR_ID}"),
            self.workspace_root.clone(),
        );
        let mut allowed = HashSet::new();
        let mut ask_tools = HashSet::new();
        let mut denied = 0usize;
        for tool in dispatcher.all_tools() {
            match self.tool_decision(&tool.name) {
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
            "assistant: connector discovery — {} tool(s) visible ({} ask-gated, {denied} denied)",
            allowed.len(),
            ask_tools.len(),
        );
        dispatcher.retain_allowed(&allowed);
        dispatcher.set_hooks(Arc::new(AssistantToolHooks {
            ask_tools,
            session_allowed: Mutex::new(HashSet::new()),
            flow_tx: self.flow_tx.clone(),
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
        dispatcher.add_host_tools(Self::host_event_tools(), handler);
        dispatcher
    }

    /// Declarations for the Assistant's host event tools.
    fn host_event_tools() -> Vec<AiTool> {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "app": {"type": "string", "description": "App id, e.g. 'chess'"},
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
                timeout_ms: Some(120_000),
            },
            AiTool {
                name: HOST_TOOL_UNSUBSCRIBE.to_string(),
                description: "Stop receiving an app's event stream.".to_string(),
                input_schema: schema,
                timeout_ms: Some(30_000),
            },
        ]
    }

    /// Persist the active conversation id and any turns not yet on disk.
    fn session_write(&mut self) {
        if self.persisted_conversation != self.model.conversation_id {
            self.persisted_conversation = self.model.conversation_id.clone();
            self.persisted_turns = 0;
        }
        if let Err(e) = self
            .store
            .set_active_conversation(&self.model.conversation_id, self.model.session_name.as_deref())
        {
            log::error!("assistant: failed to persist active conversation: {e}");
        }
        for turn in &self.model.turns[self.persisted_turns.min(self.model.turns.len())..] {
            if let Err(e) = self.store.append_turn(&self.model.conversation_id, turn) {
                log::error!(
                    "assistant[{}]: failed to persist turn: {e}",
                    self.model.conversation_id
                );
            }
        }
        self.persisted_turns = self.model.turns.len();
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
    fn handle_host_call(&mut self, tool: &str, input_json: &str, reply: SyncSender<ToolCallResult>) {
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
                v.get("app").and_then(|a| a.as_str()).unwrap_or("").to_string(),
                v.get("event").and_then(|e| e.as_str()).unwrap_or("").to_string(),
            ),
            Err(e) => {
                let _ = reply.send(err(format!("invalid_input: {e}")));
                return;
            }
        };
        if app.trim().is_empty() || event.trim().is_empty() {
            let _ = reply.send(err(
                "invalid_input: 'app' and 'event' must be non-empty".to_string(),
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
                let req = PermissionRequest::new(
                    ActorType::Agent,
                    ASSISTANT_ACTOR_ID,
                    TargetType::AppEventStream,
                    &target,
                    Some(&self.workspace_root),
                );
                match self.grant_store.evaluate(&req, None) {
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
                        self.model.permission_requested(HOST_TOOL_SUBSCRIBE, &target);
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
    fn start_turn(&mut self, conversation_id: String, _prompt: String) {
        let (delta_tx, delta_rx) = std::sync::mpsc::channel();
        self.delta_rx = Some(delta_rx);
        let dispatcher = self.gated_dispatcher();
        let request = AiBrokerRequest {
            app_id: "assistant".to_string(),
            model_tier: ModelTier::Medium,
            system: ASSISTANT_SYSTEM_PROMPT.to_string(),
            messages: self.history_messages(),
            tools: Vec::new(),
            workspace_root: Some(self.workspace_root.clone()),
            open_panes: crate::plexi_ai::broker::get_pane_snapshot(),
            tool_dispatcher: Some(Arc::new(dispatcher)),
        };
        log::info!(
            "assistant[{conversation_id}]: dispatching turn ({} message(s))",
            request.messages.len()
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
                    self.audit.append(&AuditEvent::now(
                        "tool_call",
                        &Self::connector_target(&tool),
                        if error.is_none() { "ok" } else { "error" },
                        error.as_deref().unwrap_or(""),
                    ));
                    let effects = self.model.tool_call_finished(&tool, error);
                    self.execute_effects(effects);
                }
                ToolFlowEvent::Ask {
                    tool,
                    input_json,
                    reply,
                } => {
                    log::info!("assistant: permission sheet shown for '{tool}'");
                    self.model
                        .permission_requested(&tool, &summarize_input(&input_json));
                    self.pending_reply = Some(reply);
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
            self.model.turns.push(model::Turn::now(TurnRole::Event, line.clone()));
            if !self_caused {
                trigger_lines.push(line);
            }
        }
        self.session_write();
        if trigger_lines.is_empty() {
            return;
        }
        if self.model.streaming.in_flight {
            log::info!(
                "assistant: {} event line(s) queued — turn in flight",
                trigger_lines.len()
            );
            self.queued_event_lines.extend(trigger_lines);
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
        let conversation_id = self.model.conversation_id.clone();
        self.start_turn(conversation_id, String::new());
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
        let target = Self::connector_target(&pending.tool);
        let (decision_str, reply) = match choice {
            PermissionChoice::Deny => ("deny", PermissionReply::Deny),
            PermissionChoice::AllowOnce => ("allow_once", PermissionReply::Allow { remember: false }),
            PermissionChoice::AllowSession => {
                self.record_assistant_grant(
                    TargetType::AppConnector,
                    &target,
                    GrantDuration::Session,
                    GrantSource::Session,
                );
                ("allow_session", PermissionReply::Allow { remember: true })
            }
            PermissionChoice::AllowAlways => {
                self.record_assistant_grant(
                    TargetType::AppConnector,
                    &target,
                    GrantDuration::Always,
                    GrantSource::User,
                );
                self.grant_store.save();
                ("allow_always", PermissionReply::Allow { remember: true })
            }
        };
        log::info!("assistant: permission sheet decision for '{}' = {decision_str}", pending.tool);
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

    /// Record an Allow grant for the assistant actor on `target`.
    fn record_assistant_grant(
        &mut self,
        target_type: TargetType,
        target: &str,
        duration: GrantDuration,
        source: GrantSource,
    ) {
        // Canonicalize to match `PermissionRequest`'s workspace normalization
        // (macOS tempdirs resolve `/var` → `/private/var`).
        let workspace_root = self
            .workspace_root
            .canonicalize()
            .unwrap_or_else(|_| self.workspace_root.clone());
        self.grant_store.record(GrantRecord {
            actor_type: ActorType::Agent,
            actor_id: ASSISTANT_ACTOR_ID.to_string(),
            actor_scope: ActorScope::BuiltIn,
            workspace_root: Some(workspace_root),
            target_type,
            target_id: target.to_string(),
            resource_scope: ResourceScope::Workspace,
            resource_id: None,
            decision: Decision::Allow,
            duration,
            source,
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0),
            expires_at: None,
        });
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
                    self.tool_decision(&tool.name).as_str()
                ));
            }
            out
        };
        if streams.is_empty() {
            text.push_str("\nNo app event streams declared in this workspace.");
        } else {
            text.push_str("\nApp event streams (host.events.subscribe / unsubscribe):\n");
            for (app, event) in streams {
                let target = format!("{app}::{event}");
                let req = PermissionRequest::new(
                    ActorType::Agent,
                    ASSISTANT_ACTOR_ID,
                    TargetType::AppEventStream,
                    &target,
                    Some(&self.workspace_root),
                );
                let decision = self.grant_store.evaluate(&req, None);
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

    /// `/permissions`: persisted grants for the assistant actor.
    fn cmd_list_permissions(&mut self) {
        let lines: Vec<String> = self
            .grant_store
            .records()
            .iter()
            .filter(|r| r.actor_type == ActorType::Agent && r.actor_id == ASSISTANT_ACTOR_ID)
            .map(|r| {
                format!(
                    "{} = {} ({:?}, {:?})",
                    r.target_id,
                    r.decision.as_str(),
                    r.duration,
                    r.source
                )
            })
            .collect();
        log::info!("assistant: /permissions — {} grant(s) for assistant", lines.len());
        let text = if lines.is_empty() {
            "No persisted grants for the assistant. Tool calls will ask.".to_string()
        } else {
            format!(
                "Assistant grants:\n{}\nUse /revoke <target_id> to remove one.",
                lines.join("\n")
            )
        };
        let effects = self.model.push_info(text);
        self.execute_effects(effects);
    }

    /// `/revoke <target_id>`: remove persisted grants for one target. Event
    /// stream targets also lose their live timeline subscription.
    fn cmd_revoke(&mut self, target_id: &str) {
        let removed = self
            .grant_store
            .revoke(ActorType::Agent, ASSISTANT_ACTOR_ID, target_id);
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

    fn rename_seed(&self) -> Option<String> {
        self.model.session_name.clone()
    }

    fn on_pane_renamed(&mut self, name: &str) {
        log::info!("assistant[{}]: pane renamed to '{name}'", self.model.conversation_id);
        self.model.set_session_name(name);
        self.session_write();
    }

    fn ui(&mut self, ui: &mut egui::Ui, ctx: &AppRenderContext<'_>) {
        self.pump_turn_io();
        if self.model.streaming.in_flight {
            // Keep frames coming while a worker thread streams a turn.
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
        }
        let event = AssistantRenderer::draw(ui, &mut self.model, ctx.colors, ctx.is_focused);
        match event {
            Some(ComposerEvent::Submit) => {
                let effects = self.model.submit();
                self.execute_effects(effects);
            }
            Some(ComposerEvent::Permission(choice)) => self.resolve_permission(choice),
            None => {}
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

    impl MockBroker {
        fn ok(reply: impl Into<String>) -> Arc<Self> {
            Arc::new(Self { reply: Some(reply.into()) })
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
    fn pane_action_stub_executes_without_panicking() {
        let ws = tempfile::tempdir().unwrap();
        let mut app = test_app(ws.path());
        app.execute_effects(vec![AssistantEffect::PaneAction {
            action: "focus:1".to_string(),
        }]);
        assert!(!app.model.streaming.in_flight);
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
        assert_eq!(app.store.active_conversation().as_deref(), Some(second_id.as_str()));
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
                timeout_ms: Some(2_000),
            })
            .collect();
        tool_dispatch::register(pane_id, tools, AppEventSender { tx }, ws);
        std::thread::spawn(move || {
            while let Ok(item) = rx.recv() {
                let crate::process_app::StdinItem::Event(json) = item else {
                    continue;
                };
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

    /// Persist a grant for the assistant actor directly into the app's store.
    fn seed_grant(app: &mut AssistantApp, target: &str, decision: Decision) {
        let workspace_root = app
            .workspace_root
            .canonicalize()
            .unwrap_or_else(|_| app.workspace_root.clone());
        app.grant_store.record(GrantRecord {
            actor_type: ActorType::Agent,
            actor_id: ASSISTANT_ACTOR_ID.to_string(),
            actor_scope: ActorScope::BuiltIn,
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
    fn dispatch_on_worker(
        dispatcher: Arc<ToolDispatcher>,
        tool: &str,
    ) -> Receiver<ToolCallResult> {
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
        let mut visible: Vec<String> =
            dispatcher.all_tools().into_iter().map(|t| t.name).collect();
        visible.sort();
        assert_eq!(
            visible,
            vec![
                HOST_TOOL_SUBSCRIBE,
                HOST_TOOL_UNSUBSCRIBE,
                "t_allow",
                "t_ask"
            ],
            "denied tool must be invisible; host tools always visible"
        );

        // Denied tool is also uninvocable.
        let result = dispatcher.dispatch_call("c-deny".to_string(), "t_deny", "{}".to_string());
        assert!(
            result.error.as_deref().unwrap_or("").contains("tool_not_found"),
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
        assert!(result.error.is_none(), "allowed call must succeed: {:?}", result.error);

        // The completed tool row lands in the transcript.
        pump_until(&mut app, "tool row", |a| {
            a.model.turns.iter().any(|t| t.status == Some(ToolStatus::Succeeded))
        });
        // Allow-once persists nothing.
        assert!(
            app.grant_store.records().is_empty(),
            "allow-once must not persist a grant"
        );
        // Audit: one permission decision + one tool call.
        let events = app.audit.tail(10);
        assert_eq!(events.len(), 2, "audit must record decision + call: {events:?}");
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
        assert!(ws.path().join("grants.toml").is_file(), "grant must be saved to disk");

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
            result.error.as_deref().unwrap_or("").contains("permission_denied"),
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
        assert!(app.grant_store.records().is_empty(), "deny persists nothing");

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

        app.model.composer = "/permissions".to_string();
        let effects = app.model.submit();
        app.execute_effects(effects);
        let perms_row = &app.model.turns.last().unwrap().text;
        assert!(perms_row.contains("app.view.tool = allow"), "{perms_row}");

        app.model.composer = "/revoke app.view.tool".to_string();
        let effects = app.model.submit();
        app.execute_effects(effects);
        assert!(app.model.turns.last().unwrap().text.contains("Revoked 1 grant(s)"));
        assert!(app.grant_store.records().is_empty());

        app.model.composer = "/audit".to_string();
        let effects = app.model.submit();
        app.execute_effects(effects);
        let audit_row = &app.model.turns.last().unwrap().text;
        assert!(audit_row.contains("revoke"), "revoke must be audited: {audit_row}");

        tool_dispatch::unregister(9104);
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
        assert!(app.model.streaming.in_flight, "user event must auto-start a turn");
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

    #[test]
    fn self_caused_deliveries_record_rows_without_turns() {
        let ws = tempfile::tempdir().unwrap();
        let timeline = chess_timeline();
        let mut app = test_app_with_timeline(ws.path(), timeline.clone());
        app.subscribe_stream("chess", "move.played");

        // The assistant as the event actor: row, no turn.
        emit_move(&timeline, AppEventActor::Agent, Some("agent:assistant"), None);
        app.pump_turn_io();
        assert_eq!(event_rows(&app), 1);
        assert!(!app.model.streaming.in_flight, "own action must not trigger");

        // App-emitted event caused by the assistant's tool call: row, no turn.
        emit_move(&timeline, AppEventActor::App, None, Some("agent:assistant"));
        app.pump_turn_io();
        assert_eq!(event_rows(&app), 2);
        assert!(!app.model.streaming.in_flight, "caused-by-self must not trigger");
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

        // The turn ends: the queued event triggers the follow-up turn.
        app.model.streaming = model::StreamingState::default();
        app.pump_turn_io();
        assert!(app.model.streaming.in_flight, "queued event must start the next turn");
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

    #[test]
    fn subscribe_host_tool_ask_allow_always_persists_grant_and_revoke_unsubscribes() {
        let ws = tempfile::tempdir().unwrap();
        let timeline = chess_timeline();
        let mut app = test_app_with_timeline(ws.path(), timeline.clone());

        // Host event tools are visible in the turn snapshot.
        let dispatcher = Arc::new(app.gated_dispatcher());
        let names: HashSet<String> = dispatcher
            .all_tools()
            .into_iter()
            .map(|t| t.name)
            .collect();
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
            app.model.turns.last().unwrap().text.contains("live subscription"),
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
            result.error.as_deref().unwrap_or("").contains("permission_denied"),
            "{:?}",
            result.error
        );
        assert!(app.live_subs.is_empty());
        assert!(timeline.lock().unwrap().subscriptions().is_empty());
        assert!(app.grant_store.records().is_empty(), "deny persists nothing");
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
        assert!(result.error.as_deref().unwrap_or("").contains("not_subscribed"));
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
    fn error_turn_appends_error_row() {
        let ws = tempfile::tempdir().unwrap();
        let mut app = AssistantApp::new(ws.path().to_path_buf(), MockBroker::error(), ws.path());
        app.model.composer = "trigger error".to_string();
        let effects = app.model.submit();
        app.execute_effects(effects);
        wait_for_turn(&mut app);
        assert_eq!(app.model.turns.last().unwrap().role, TurnRole::Error);
    }
}
