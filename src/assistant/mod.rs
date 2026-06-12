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
use crate::app_protocol::{AiMessage, ModelTier};
use crate::broker::{
    ActorScope, ActorType, Decision, GrantDuration, GrantRecord, GrantSource, GrantStore,
    PermissionRequest, ResourceScope, TargetType,
};
use crate::plexi_ai::broker::{AiBroker, AiBrokerRequest, StreamDelta, StreamSink};
use crate::plexi_ai::tool_dispatch::{ToolCallHooks, ToolDispatcher};

use audit::{AuditEvent, AuditLog};
use model::{AssistantEffect, AssistantModel, PermissionChoice, TurnRole};
use render::{AssistantRenderer, ComposerEvent};
use store::AssistantStore;

const ASSISTANT_SYSTEM_PROMPT: &str = "You are the Plexi Assistant, the workspace \
operator inside the Plexi terminal environment. Answer concisely. Tools exposed \
by running apps are available to you when listed; calls may pause for the user's \
permission. Pane and terminal control arrives in a later phase — when asked to \
act on panes, explain that those tools are not wired up yet.";

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
}

impl AssistantApp {
    /// Open the Assistant for a workspace: resume the persisted active
    /// conversation, or create a fresh one. `profile_dir` is the channel
    /// config dir — grants load from `<profile_dir>/grants.toml` and audit
    /// events append to `<profile_dir>/audit.jsonl`.
    pub fn new(workspace_root: PathBuf, broker: Arc<dyn AiBroker>, profile_dir: &Path) -> Self {
        let store = AssistantStore::new(&workspace_root);
        let model = match store.active_conversation() {
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
        };
        // Persist the active id immediately so close-then-reopen resumes
        // this conversation even before the first turn.
        app.session_write();
        app
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
        dispatcher
    }

    /// Persist the active conversation id and any turns not yet on disk.
    fn session_write(&mut self) {
        if self.persisted_conversation != self.model.conversation_id {
            self.persisted_conversation = self.model.conversation_id.clone();
            self.persisted_turns = 0;
        }
        if let Err(e) = self
            .store
            .set_active_conversation(&self.model.conversation_id)
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

    /// Conversation history for the broker: user/assistant turns only.
    fn history_messages(&self) -> Vec<AiMessage> {
        self.model
            .turns
            .iter()
            .filter_map(|turn| match turn.role {
                TurnRole::User => Some(AiMessage {
                    role: "user".to_string(),
                    content: turn.text.clone(),
                }),
                TurnRole::Assistant => Some(AiMessage {
                    role: "assistant".to_string(),
                    content: turn.text.clone(),
                }),
                TurnRole::Tool | TurnRole::Error => None,
            })
            .collect()
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
            stream_sink: Some(StreamSink(delta_tx)),
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
                let resp = broker.dispatch(request);
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

    /// Per-frame pump: tool-flow events, live stream deltas, finished turns.
    fn pump_turn_io(&mut self) {
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
    }

    /// Apply the user's permission-sheet decision: record the grant per its
    /// duration, audit it, and unblock the worker thread.
    fn resolve_permission(&mut self, choice: PermissionChoice) {
        let Some(pending) = self.model.pending_permission.clone() else {
            return;
        };
        let target = Self::connector_target(&pending.tool);
        let (decision_str, reply) = match choice {
            PermissionChoice::Deny => ("deny", PermissionReply::Deny),
            PermissionChoice::AllowOnce => ("allow_once", PermissionReply::Allow { remember: false }),
            PermissionChoice::AllowSession => {
                self.record_assistant_grant(&target, GrantDuration::Session, GrantSource::Session);
                ("allow_session", PermissionReply::Allow { remember: true })
            }
            PermissionChoice::AllowAlways => {
                self.record_assistant_grant(&target, GrantDuration::Always, GrantSource::User);
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

    /// Record an Allow grant for the assistant actor on `target`.
    fn record_assistant_grant(
        &mut self,
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
            target_type: TargetType::AppConnector,
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
        log::info!("assistant: /tools — {} connector tool(s) discovered", tools.len());
        let text = if tools.is_empty() {
            "No app connector tools are exposed in this workspace.".to_string()
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

    /// `/revoke <target_id>`: remove persisted grants for one target.
    fn cmd_revoke(&mut self, target_id: &str) {
        let removed = self
            .grant_store
            .revoke(ActorType::Agent, ASSISTANT_ACTOR_ID, target_id);
        let text = if removed == 0 {
            format!("No grants found for '{target_id}'. See /permissions for target ids.")
        } else {
            self.grant_store.save();
            self.audit.append(&AuditEvent::now(
                "revoke",
                target_id,
                "revoked",
                &format!("{removed} grant(s) removed"),
            ));
            format!("Revoked {removed} grant(s) for '{target_id}'.")
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

    /// Test broker: echoes a canned reply and streams deltas into the sink.
    struct EchoBroker;

    impl AiBroker for EchoBroker {
        fn dispatch(&self, request: AiBrokerRequest) -> AiBrokerResponse {
            if let Some(sink) = &request.stream_sink {
                let _ = sink.0.send(StreamDelta::Reasoning("pondering".to_string()));
                let _ = sink.0.send(StreamDelta::Answer("echo: ".to_string()));
                let _ = sink.0.send(StreamDelta::Answer("ok".to_string()));
            }
            AiBrokerResponse::ok_with_deltas(
                "echo: ok".to_string(),
                1,
                1,
                vec!["echo: ".to_string(), "ok".to_string()],
            )
        }
    }

    /// Assistant whose grants + audit live in the (temp) workspace dir.
    fn test_app(ws: &Path) -> AssistantApp {
        AssistantApp::new(ws.to_path_buf(), Arc::new(EchoBroker), ws)
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
        let (tx, rx) = std::sync::mpsc::channel();
        let tool = tool.to_string();
        let call_id = format!("call-{}", uuid::Uuid::new_v4());
        std::thread::spawn(move || {
            let result = dispatcher.dispatch_call(call_id, &tool, "{\"x\": 1}".to_string());
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
        assert_eq!(visible, vec!["t_allow", "t_ask"], "denied tool must be invisible");

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
}
