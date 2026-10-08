//! Pi-style assistant loop.
//!
//! The default Assistant path does not use this module. `[ai] harness = "pi"`
//! selects it. Design and the MIT credit for Pi live in
//! `docs/assistant-harness-pi.md`.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[cfg(test)]
use crate::broker::{ActorScope, ActorType, GrantRecord, GrantSource, ResourceScope, TargetType};
use crate::broker::{Decision, GrantDuration, GrantStore};
use crate::plexi_ai::backend::{BillingModel, ConcreteModelRoute};
use crate::plexi_ai::broker::{AiBroker, AiBrokerRequest};
use crate::plexi_ai::ledger::{self, LedgerRow};
use crate::plexi_ai::turn_loop::TurnDelta;
use crate::plexi_ai::CancelToken;
use crate::protocol::{AiMessage, AiTool, ModelTier};

pub const SESSION_SUFFIX: &str = ".pi.jsonl";
pub const STALE_READ_PREFIX: &str = "Stale file read:";
const SUMMARY_PREFIX: &str = "Conversation summary:\n";
const DEFAULT_SUMMARY_MARK: &str = "Compacted history:";

/// Which loop `AssistantApp::start_turn` runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HarnessKind {
    Current,
    Pi,
}

/// `None`, `""`, and `"current"` keep the shipped loop. `"pi"` selects this one.
/// Any other string logs a warning and stays on the current loop.
pub fn harness_kind_from_config(value: Option<&str>) -> HarnessKind {
    match value.map(str::trim) {
        Some("pi") => {
            log::info!("assistant harness: selected pi");
            HarnessKind::Pi
        }
        Some(other) if !other.is_empty() && other != "current" => {
            log::warn!("assistant harness: unknown value {other:?}; using current");
            HarnessKind::Current
        }
        _ => HarnessKind::Current,
    }
}

pub fn tier_title(tier: ModelTier) -> &'static str {
    match tier {
        ModelTier::Low => "Low",
        ModelTier::Medium => "Medium",
        ModelTier::High => "High",
    }
}

/// Label a later `/model` row can show. The picker UI does not read this yet.
pub fn picker_label(tier: ModelTier, served_model: &str) -> String {
    format!("{} — {served_model}", tier_title(tier))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelVisibility {
    pub tier: ModelTier,
    pub configured_model: String,
    pub served_model: String,
    pub picker_label: String,
}

impl ModelVisibility {
    fn new(tier: ModelTier, configured: &str, served: &str) -> Self {
        Self {
            tier,
            configured_model: configured.to_string(),
            served_model: served.to_string(),
            picker_label: picker_label(tier, served),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PiConfig {
    pub provider: String,
    pub tier: ModelTier,
    pub configured_model: String,
    pub system: String,
    pub token_budget: u32,
    pub keep_recent_turns: usize,
    pub max_steps: usize,
    pub session_id: String,
}

impl PiConfig {
    pub fn from_ai(ai: &crate::config::AiConfig, tier: ModelTier, session_id: &str) -> Self {
        let provider = ai
            .backend
            .clone()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "openrouter".to_string());
        let configured = match provider.as_str() {
            "ollama" => ai.ollama.as_ref().and_then(|cfg| cfg.tiers.resolve(tier)),
            "local" => ai.local.as_ref().and_then(|cfg| cfg.tiers.resolve(tier)),
            _ => ai
                .openrouter
                .as_ref()
                .and_then(|cfg| cfg.tiers.resolve(tier)),
        }
        .unwrap_or_default();
        Self {
            provider,
            tier,
            configured_model: configured,
            system: String::new(),
            token_budget: 24_000,
            keep_recent_turns: 8,
            max_steps: 30,
            session_id: session_id.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

impl ToolCall {
    #[cfg(test)]
    pub fn new(id: impl Into<String>, name: impl Into<String>, arguments: Value) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            arguments,
        }
    }
}

/// One persisted transcript row. User, assistant, and tool lines carry the
/// same `role` / `text` / `created_at` fields as `Turn`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum SessionEntry {
    User {
        text: String,
        created_at: String,
    },
    Assistant {
        text: String,
        created_at: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tool_calls: Vec<ToolCall>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model_id: Option<String>,
    },
    Tool {
        text: String,
        created_at: String,
        call_id: String,
        name: String,
        #[serde(default)]
        is_error: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        read_path: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        read_mtime_ns: Option<u64>,
        #[serde(default)]
        stale: bool,
    },
    Compaction {
        text: String,
        created_at: String,
        kept_from: usize,
    },
    Note {
        text: String,
        created_at: String,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PiSession {
    pub id: String,
    pub entries: Vec<SessionEntry>,
}

impl PiSession {
    pub fn path(assistant_dir: &Path, id: &str) -> PathBuf {
        assistant_dir
            .join("conversations")
            .join(format!("{id}{SESSION_SUFFIX}"))
    }

    pub fn load(assistant_dir: &Path, id: &str) -> Result<Option<Self>, String> {
        let path = Self::path(assistant_dir, id);
        let raw = match std::fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(format!("pi session: read {}: {error}", path.display()));
            }
        };
        let mut entries = Vec::new();
        for (index, line) in raw.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<SessionEntry>(line) {
                Ok(entry) => entries.push(entry),
                Err(error) => {
                    log::error!(
                        "pi session: skipping line {} in {}: {error}",
                        index + 1,
                        path.display()
                    );
                }
            }
        }
        Ok(Some(Self {
            id: id.to_string(),
            entries,
        }))
    }

    pub fn save(&self, assistant_dir: &Path) -> Result<(), String> {
        let path = Self::path(assistant_dir, &self.id);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("pi session: create {}: {error}", parent.display()))?;
        }
        let mut raw = String::new();
        for entry in &self.entries {
            let line = serde_json::to_string(entry)
                .map_err(|error| format!("pi session: serialize: {error}"))?;
            raw.push_str(&line);
            raw.push('\n');
        }
        crate::platform::fs::atomic_write(&path, raw.as_bytes())
            .map_err(|error| format!("pi session: write {}: {error}", path.display()))?;
        log::info!(
            "pi_harness[{}]: saved session entries={}",
            self.id,
            self.entries.len()
        );
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LlmMessage {
    pub role: String,
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelStepRequest {
    pub provider: String,
    pub tier: ModelTier,
    pub configured_model: String,
    pub system: String,
    pub messages: Vec<LlmMessage>,
    pub tools: Vec<String>,
    pub estimated_tokens: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelStep {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
    pub model_id: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
}

impl ModelStep {
    #[cfg(test)]
    pub fn text(model_id: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            tool_calls: Vec::new(),
            model_id: model_id.into(),
            input_tokens: 1,
            output_tokens: 1,
        }
    }

    #[cfg(test)]
    pub fn calls(model_id: impl Into<String>, tool_calls: Vec<ToolCall>) -> Self {
        Self {
            text: String::new(),
            tool_calls,
            model_id: model_id.into(),
            input_tokens: 1,
            output_tokens: 1,
        }
    }
}

pub trait ModelStepper: Send {
    fn step(
        &mut self,
        request: &ModelStepRequest,
        on_delta: &mut dyn FnMut(TurnDelta<'_>),
    ) -> Result<ModelStep, String>;
}

/// One provider call through the existing broker. Tools are not dispatched.
pub struct BrokerStepper {
    broker: Arc<dyn AiBroker>,
    cancel: CancelToken,
}

impl BrokerStepper {
    #[cfg(test)]
    pub fn new(broker: Arc<dyn AiBroker>) -> Self {
        Self {
            broker,
            cancel: CancelToken::new(),
        }
    }

    pub fn with_cancel(broker: Arc<dyn AiBroker>, cancel: CancelToken) -> Self {
        Self { broker, cancel }
    }
}

impl ModelStepper for BrokerStepper {
    fn step(
        &mut self,
        request: &ModelStepRequest,
        on_delta: &mut dyn FnMut(TurnDelta<'_>),
    ) -> Result<ModelStep, String> {
        let broker_request = AiBrokerRequest {
            app_id: "assistant".to_string(),
            model_tier: request.tier,
            concrete_model: Some(ConcreteModelRoute {
                provider: request.provider.clone(),
                model: request.configured_model.clone(),
            }),
            reasoning_effort: None,
            system: request.system.clone(),
            messages: request
                .messages
                .iter()
                .filter(|message| message.role == "user" || message.role == "assistant")
                .map(|message| AiMessage {
                    role: message.role.clone(),
                    content: message.content.clone(),
                })
                .collect(),
            tools: request
                .tools
                .iter()
                .map(|name| AiTool {
                    name: name.clone(),
                    description: name.clone(),
                    input_schema: serde_json::json!({"type": "object"}),
                    output_schema: serde_json::json!({"type": "object"}),
                    timeout_ms: None,
                    read_only: false,
                })
                .collect(),
            workspace_root: None,
            open_panes: Arc::new(Vec::new()),
            tool_dispatcher: None,
            cancel: self.cancel.clone(),
            max_tool_iterations: Some(1),
            client: None,
            kind: None,
            single_completion: true,
            structured_messages: llm_messages_to_json(&request.messages),
        };
        let response = self.broker.complete_once(broker_request, on_delta);
        if let Some(error) = response.error {
            return Err(error);
        }
        let model_id = response
            .model_id
            .filter(|id| !id.is_empty())
            .unwrap_or_else(|| request.configured_model.clone());
        Ok(ModelStep {
            text: response.content.unwrap_or_default(),
            tool_calls: response
                .tool_calls
                .into_iter()
                .map(|call| ToolCall {
                    id: call.id,
                    name: call.name,
                    arguments: parse_tool_arguments(&call.arguments),
                })
                .collect(),
            model_id,
            input_tokens: response.tokens_in,
            output_tokens: response.tokens_out,
        })
    }
}

fn parse_tool_arguments(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or_else(|_| serde_json::json!({ "raw": raw }))
}

fn llm_messages_to_json(messages: &[LlmMessage]) -> Vec<Value> {
    messages
        .iter()
        .map(|message| {
            if message.role == "tool" {
                serde_json::json!({
                    "role": "tool",
                    "tool_call_id": message.tool_call_id,
                    "content": message.content,
                })
            } else if !message.tool_calls.is_empty() {
                let calls: Vec<Value> = message
                    .tool_calls
                    .iter()
                    .map(|call| {
                        serde_json::json!({
                            "id": call.id,
                            "type": "function",
                            "function": {
                                "name": call.name,
                                "arguments": call.arguments.to_string(),
                            }
                        })
                    })
                    .collect();
                serde_json::json!({
                    "role": "assistant",
                    "content": message.content,
                    "tool_calls": calls,
                })
            } else {
                serde_json::json!({
                    "role": message.role,
                    "content": message.content,
                })
            }
        })
        .collect()
}

#[derive(Debug, Clone)]
pub struct ToolOutput {
    pub text: String,
    pub read_path: Option<String>,
}

impl ToolOutput {
    #[cfg(test)]
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            read_path: None,
        }
    }

    #[cfg(test)]
    pub fn read(path: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            read_path: Some(path.into()),
        }
    }
}

pub trait ToolHandler: Send {
    fn name(&self) -> &str;
    fn execute(&mut self, args: &Value) -> Result<ToolOutput, String>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionChoice {
    #[cfg(test)]
    AllowOnce,
    #[cfg(test)]
    AllowAlways,
    Deny,
}

pub trait PermissionDecider: Send {
    fn decide(&mut self, tool: &str, args: &Value) -> PermissionChoice;
}

struct DenyAll;

impl PermissionDecider for DenyAll {
    fn decide(&mut self, _tool: &str, _args: &Value) -> PermissionChoice {
        PermissionChoice::Deny
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolGate {
    Allow,
    #[cfg(test)]
    Veto {
        reason: String,
    },
}

pub struct BeforeTurn<'a> {
    pub user_text: &'a str,
    pub extra_system: &'a mut String,
}

pub struct ToolResultDraft<'a> {
    pub text: &'a mut String,
    pub is_error: &'a mut bool,
}

pub trait PiPlugin: Send {
    fn name(&self) -> &str;
    fn before_turn(&mut self, _turn: &mut BeforeTurn<'_>) {}
    fn before_tool(&mut self, _call: &ToolCall) -> ToolGate {
        ToolGate::Allow
    }
    fn after_tool(&mut self, _call: &ToolCall, _result: &mut ToolResultDraft<'_>) {}
    fn on_compact(&mut self, _older: &[SessionEntry]) -> Option<String> {
        None
    }
    fn on_turn_end(&mut self, _text: Option<&str>, _error: Option<&str>) {}
    fn context_block(&self) -> Option<String> {
        None
    }
}

#[derive(Debug, Clone)]
pub struct OpenPane {
    pub app_type: String,
    pub title: String,
    pub path: Option<String>,
    pub focused: bool,
    pub excerpt: String,
}

pub struct OpenPanePlugin {
    pub panes: Vec<OpenPane>,
}

impl OpenPanePlugin {
    pub fn new(panes: Vec<OpenPane>) -> Self {
        Self { panes }
    }
}

impl PiPlugin for OpenPanePlugin {
    fn name(&self) -> &str {
        "open-panes"
    }

    fn context_block(&self) -> Option<String> {
        if self.panes.is_empty() {
            return None;
        }
        let mut block = String::from("Open panes:\n");
        for pane in &self.panes {
            let path = pane.path.as_deref().unwrap_or("");
            let focused = if pane.focused { "yes" } else { "no" };
            let excerpt: String = pane.excerpt.chars().take(400).collect();
            block.push_str(&format!(
                "- app={app} title={title} path={path} focused={focused} excerpt={excerpt}\n",
                app = pane.app_type,
                title = pane.title,
            ));
        }
        Some(block)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum HarnessEvent {
    BeforeTurn {
        plugin: String,
    },
    BeforeTool {
        plugin: String,
        tool: String,
    },
    AfterTool {
        plugin: String,
        tool: String,
    },
    OnCompact {
        plugin: String,
    },
    OnTurnEnd {
        plugin: String,
    },
    PermissionAsk {
        tool: String,
    },
    PermissionDecision {
        tool: String,
        decision: String,
    },
    Compaction {
        tokens_before: u32,
        tokens_after: u32,
        summary: String,
    },
    Aborted,
    MaxSteps,
    ToolError {
        tool: String,
        message: String,
    },
    #[cfg(test)]
    Veto {
        tool: String,
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnReport {
    pub text: Option<String>,
    pub error: Option<String>,
    pub aborted: bool,
    pub hit_max_steps: bool,
    pub visibility: Option<ModelVisibility>,
}

#[derive(Clone)]
pub struct AbortHandle(Arc<AtomicBool>);

impl AbortHandle {
    pub fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }

    #[cfg(test)]
    pub fn abort(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    fn is_aborted(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

impl Default for AbortHandle {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
pub struct SteeringHandle(Arc<Mutex<VecDeque<String>>>);

impl SteeringHandle {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(VecDeque::new())))
    }

    #[cfg(test)]
    pub fn steer(&self, text: impl Into<String>) {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push_back(text.into());
    }

    fn drain(&self) -> Vec<String> {
        let mut queue = self.0.lock().unwrap_or_else(|error| error.into_inner());
        queue.drain(..).collect()
    }
}

impl Default for SteeringHandle {
    fn default() -> Self {
        Self::new()
    }
}

struct RegisteredTool {
    needs_grant: bool,
    handler: Box<dyn ToolHandler>,
}

pub struct PiHarness {
    config: PiConfig,
    session: PiSession,
    tools: Vec<RegisteredTool>,
    plugins: Vec<Box<dyn PiPlugin>>,
    stepper: Box<dyn ModelStepper>,
    permissions: Box<dyn PermissionDecider>,
    grants: GrantStore,
    events: Vec<HarnessEvent>,
    requests: Vec<ModelStepRequest>,
    abort: AbortHandle,
    steering: SteeringHandle,
    visibility: Option<ModelVisibility>,
    next_call: u64,
    assistant_dir: Option<PathBuf>,
    cancel: CancelToken,
}

impl PiHarness {
    #[cfg(test)]
    pub fn new(config: PiConfig, stepper: Box<dyn ModelStepper>, profile_dir: &Path) -> Self {
        let session = PiSession {
            id: config.session_id.clone(),
            entries: Vec::new(),
        };
        Self::from_session(config, session, stepper, profile_dir)
    }

    pub fn from_session(
        config: PiConfig,
        mut session: PiSession,
        stepper: Box<dyn ModelStepper>,
        profile_dir: &Path,
    ) -> Self {
        if session.id.is_empty() {
            session.id = config.session_id.clone();
        }
        Self {
            config,
            session,
            tools: Vec::new(),
            plugins: Vec::new(),
            stepper,
            permissions: Box::new(DenyAll),
            grants: GrantStore::load_or_default(profile_dir),
            events: Vec::new(),
            requests: Vec::new(),
            abort: AbortHandle::new(),
            steering: SteeringHandle::new(),
            visibility: None,
            next_call: 1,
            assistant_dir: None,
            cancel: CancelToken::new(),
        }
    }

    pub fn set_cancel(&mut self, cancel: CancelToken) {
        self.cancel = cancel;
    }

    #[cfg(test)]
    pub fn set_abort(&mut self, abort: AbortHandle) {
        self.abort = abort;
    }

    #[cfg(test)]
    pub fn set_steering(&mut self, steering: SteeringHandle) {
        self.steering = steering;
    }

    #[cfg(test)]
    pub fn set_permissions(&mut self, permissions: Box<dyn PermissionDecider>) {
        self.permissions = permissions;
    }

    pub fn set_assistant_dir(&mut self, dir: PathBuf) {
        self.assistant_dir = Some(dir);
    }

    #[cfg(test)]
    pub fn register_tool(&mut self, needs_grant: bool, handler: Box<dyn ToolHandler>) {
        self.tools.push(RegisteredTool {
            needs_grant,
            handler,
        });
    }

    pub fn add_plugin(&mut self, plugin: Box<dyn PiPlugin>) {
        log::info!(
            "pi_harness[{}]: plugin registered {}",
            self.session.id,
            plugin.name()
        );
        self.plugins.push(plugin);
    }

    #[cfg(test)]
    pub fn events(&self) -> &[HarnessEvent] {
        &self.events
    }

    #[cfg(test)]
    pub fn requests(&self) -> &[ModelStepRequest] {
        &self.requests
    }

    pub fn session(&self) -> &PiSession {
        &self.session
    }

    pub fn visibility(&self) -> Option<&ModelVisibility> {
        self.visibility.as_ref()
    }

    #[cfg(test)]
    pub fn run_user_turn(&mut self, text: &str) -> TurnReport {
        self.run_user_turn_with(text, &mut |_| {})
    }

    /// Same loop as [`Self::run_user_turn`], forwarding provider deltas.
    pub fn run_user_turn_with(
        &mut self,
        text: &str,
        on_delta: &mut dyn FnMut(TurnDelta<'_>),
    ) -> TurnReport {
        log::info!(
            "pi_harness[{}]: turn start provider={} tier={} configured={}",
            self.session.id,
            self.config.provider,
            self.config.tier.as_str(),
            self.config.configured_model
        );
        let mut extra_system = String::new();
        for plugin in &mut self.plugins {
            self.events.push(HarnessEvent::BeforeTurn {
                plugin: plugin.name().to_string(),
            });
            let mut turn = BeforeTurn {
                user_text: text,
                extra_system: &mut extra_system,
            };
            plugin.before_turn(&mut turn);
            log::info!(
                "pi_harness[{}]: before_turn plugin={} user_chars={} extra_chars={}",
                self.session.id,
                plugin.name(),
                turn.user_text.chars().count(),
                turn.extra_system.chars().count()
            );
        }
        self.session.entries.push(SessionEntry::User {
            text: text.to_string(),
            created_at: now_stamp(),
        });

        let mut steps = 0usize;
        let mut final_text = None;
        let mut final_error = None;
        let mut aborted = false;
        let mut hit_max_steps = false;

        loop {
            if self.abort.is_aborted() || self.cancel_requested() {
                aborted = true;
                self.events.push(HarnessEvent::Aborted);
                log::info!("pi_harness[{}]: abort", self.session.id);
                break;
            }
            self.refresh_staleness(text);
            self.maybe_compact();
            let request = self.build_request(&extra_system);
            self.requests.push(request.clone());
            let step = match self.stepper.step(&request, on_delta) {
                Ok(step) => step,
                Err(error) => {
                    final_error = Some(error);
                    break;
                }
            };
            steps += 1;
            self.record_visibility(&step);
            let calls = assign_call_ids(&step.tool_calls, &mut self.next_call);
            self.session.entries.push(SessionEntry::Assistant {
                text: step.text.clone(),
                created_at: now_stamp(),
                tool_calls: calls.clone(),
                model_id: Some(step.model_id.clone()),
            });
            if calls.is_empty() {
                final_text = Some(step.text);
                break;
            }
            if steps >= self.config.max_steps {
                hit_max_steps = true;
                self.events.push(HarnessEvent::MaxSteps);
                log::info!(
                    "pi_harness[{}]: max steps {}",
                    self.session.id,
                    self.config.max_steps
                );
                for call in &calls {
                    self.push_tool_result(call, "max_steps_exceeded", true, None, None);
                }
                final_text = Some(format!(
                    "Paused after {} tool steps without a final answer.",
                    self.config.max_steps
                ));
                break;
            }
            if self.abort.is_aborted() || self.cancel_requested() {
                aborted = true;
                self.events.push(HarnessEvent::Aborted);
                log::info!("pi_harness[{}]: abort before tools", self.session.id);
                for call in &calls {
                    self.push_tool_result(call, "aborted", true, None, None);
                }
                break;
            }
            for call in calls {
                self.execute_call(&call);
            }
            for steered in self.steering.drain() {
                self.session.entries.push(SessionEntry::User {
                    text: steered,
                    created_at: now_stamp(),
                });
            }
        }

        for plugin in &mut self.plugins {
            self.events.push(HarnessEvent::OnTurnEnd {
                plugin: plugin.name().to_string(),
            });
            plugin.on_turn_end(final_text.as_deref(), final_error.as_deref());
        }
        if let Some(dir) = self.assistant_dir.clone() {
            if let Err(error) = self.session.save(&dir) {
                log::error!("pi_harness[{}]: {error}", self.session.id);
            }
        }
        TurnReport {
            text: final_text,
            error: final_error,
            aborted,
            hit_max_steps,
            visibility: self.visibility.clone(),
        }
    }

    fn cancel_requested(&self) -> bool {
        self.cancel.is_cancelled()
    }

    fn record_visibility(&mut self, step: &ModelStep) {
        let visibility = ModelVisibility::new(
            self.config.tier,
            &self.config.configured_model,
            &step.model_id,
        );
        log::info!(
            "pi_harness[{}]: model tier={} configured={} served={}",
            self.session.id,
            tier_title(self.config.tier),
            visibility.configured_model,
            visibility.served_model
        );
        let row = LedgerRow::with_attribution(
            &self.config.provider,
            BillingModel::Subscription,
            Some("assistant".to_string()),
            Some(visibility.served_model.clone()),
            nonzero(step.input_tokens),
            nonzero(step.output_tokens),
            None,
        );
        ledger::append(&row);
        self.visibility = Some(visibility);
    }

    fn execute_call(&mut self, call: &ToolCall) {
        for plugin in &mut self.plugins {
            self.events.push(HarnessEvent::BeforeTool {
                plugin: plugin.name().to_string(),
                tool: call.name.clone(),
            });
            match plugin.before_tool(call) {
                ToolGate::Allow => {}
                #[cfg(test)]
                ToolGate::Veto { reason } => {
                    self.events.push(HarnessEvent::Veto {
                        tool: call.name.clone(),
                        reason: reason.clone(),
                    });
                    log::info!(
                        "pi_harness[{}]: before_tool veto tool={} reason={reason}",
                        self.session.id,
                        call.name
                    );
                    self.push_tool_result(call, &reason, true, None, None);
                    return;
                }
            }
        }
        let Some(index) = self
            .tools
            .iter()
            .position(|tool| tool.handler.name() == call.name)
        else {
            self.push_tool_result(
                call,
                &format!("unknown tool {}", call.name),
                true,
                None,
                None,
            );
            return;
        };
        if self.tools[index].needs_grant && !self.allow_tool(&call.name, &call.arguments) {
            let payload = serde_json::json!({
                "error": "permission_denied",
                "tool": call.name,
                "reason": "denied",
            })
            .to_string();
            self.push_tool_result(call, &payload, true, None, None);
            return;
        }
        let executed = self.tools[index].handler.execute(&call.arguments);
        let (mut text, mut is_error, read_path) = match executed {
            Ok(output) => (output.text, false, output.read_path),
            Err(error) => {
                self.events.push(HarnessEvent::ToolError {
                    tool: call.name.clone(),
                    message: error.clone(),
                });
                log::info!(
                    "pi_harness[{}]: tool error tool={} error={error}",
                    self.session.id,
                    call.name
                );
                (format!("error: {error}"), true, None)
            }
        };
        let read_mtime_ns = read_path.as_deref().and_then(mtime_ns);
        for plugin in &mut self.plugins {
            self.events.push(HarnessEvent::AfterTool {
                plugin: plugin.name().to_string(),
                tool: call.name.clone(),
            });
            let mut draft = ToolResultDraft {
                text: &mut text,
                is_error: &mut is_error,
            };
            plugin.after_tool(call, &mut draft);
            log::info!(
                "pi_harness[{}]: after_tool plugin={} tool={} error={} chars={}",
                self.session.id,
                plugin.name(),
                call.name,
                draft.is_error,
                draft.text.chars().count()
            );
        }
        self.push_tool_result(call, &text, is_error, read_path, read_mtime_ns);
    }

    fn allow_tool(&mut self, tool: &str, args: &Value) -> bool {
        if self.always_covers(tool) {
            log::info!("pi_harness[{}]: grant covers {tool}", self.session.id);
            return true;
        }
        self.events.push(HarnessEvent::PermissionAsk {
            tool: tool.to_string(),
        });
        log::info!(
            "pi_harness[{}]: permission ask tool={tool}",
            self.session.id
        );
        let choice = self.permissions.decide(tool, args);
        let decision = match choice {
            #[cfg(test)]
            PermissionChoice::AllowOnce => "allow_once",
            #[cfg(test)]
            PermissionChoice::AllowAlways => "allow_always",
            PermissionChoice::Deny => "deny",
        };
        self.events.push(HarnessEvent::PermissionDecision {
            tool: tool.to_string(),
            decision: decision.to_string(),
        });
        match choice {
            #[cfg(test)]
            PermissionChoice::AllowOnce => true,
            #[cfg(test)]
            PermissionChoice::AllowAlways => {
                self.persist_always(tool);
                true
            }
            PermissionChoice::Deny => false,
        }
    }

    fn always_covers(&self, tool: &str) -> bool {
        self.grants.records().iter().any(|record| {
            record.decision == Decision::Allow
                && record.duration == GrantDuration::Always
                && record.target_id == tool
                && record.actor_id == "assistant"
                && !record.consumed
        })
    }

    #[cfg(test)]
    fn persist_always(&mut self, tool: &str) {
        let record = GrantRecord {
            actor_type: ActorType::Agent,
            actor_id: "assistant".to_string(),
            actor_scope: ActorScope::BuiltIn,
            target_type: TargetType::HostTool,
            target_id: tool.to_string(),
            resource_scope: ResourceScope::Global,
            decision: Decision::Allow,
            duration: GrantDuration::Always,
            source: GrantSource::User,
            created_at: crate::platform::clock::now_secs() as i64,
            binding_schema: 1,
            grant_id: format!("pi:assistant:{tool}"),
            tool_scoped: true,
            trust_origin: "host".to_string(),
            ..GrantRecord::unbound()
        };
        self.grants.record(record);
        if let Err(error) = self.grants.try_save() {
            log::error!(
                "pi_harness[{}]: grant save failed for {tool}: {error}",
                self.session.id
            );
        } else {
            log::info!(
                "pi_harness[{}]: persisted always grant for {tool}",
                self.session.id
            );
        }
    }

    fn push_tool_result(
        &mut self,
        call: &ToolCall,
        text: &str,
        is_error: bool,
        read_path: Option<String>,
        read_mtime_ns: Option<u64>,
    ) {
        self.session.entries.push(SessionEntry::Tool {
            text: text.to_string(),
            created_at: now_stamp(),
            call_id: call.id.clone(),
            name: call.name.clone(),
            is_error,
            read_path,
            read_mtime_ns,
            stale: false,
        });
    }

    fn refresh_staleness(&mut self, user_text: &str) {
        let mut paths = Vec::new();
        for entry in &self.session.entries {
            if let SessionEntry::Tool {
                read_path: Some(path),
                ..
            } = entry
            {
                if !paths.iter().any(|existing: &String| existing == path) {
                    paths.push(path.clone());
                }
            }
        }
        let lower = user_text.to_lowercase();
        let asks_reread = lower.contains("read")
            && (lower.contains("again")
                || lower.contains("reread")
                || lower.contains("re-read")
                || paths.iter().any(|path| user_text.contains(path)));
        for entry in &mut self.session.entries {
            let SessionEntry::Tool {
                read_path,
                read_mtime_ns,
                stale,
                ..
            } = entry
            else {
                continue;
            };
            let Some(path) = read_path.clone() else {
                continue;
            };
            let changed = read_mtime_ns
                .zip(mtime_ns(&path))
                .is_some_and(|(stored, now)| stored != now);
            let asked = asks_reread
                && (user_text.contains(&path)
                    || lower.contains("again")
                    || lower.contains("reread")
                    || lower.contains("re-read"));
            if changed || asked {
                *stale = true;
            }
        }
    }

    fn stale_notes(&self) -> Vec<String> {
        let mut notes = Vec::new();
        for entry in &self.session.entries {
            let SessionEntry::Tool {
                stale: true,
                read_path: Some(path),
                ..
            } = entry
            else {
                continue;
            };
            let note = format!(
                "{STALE_READ_PREFIX} {path} changed or was asked for again. Call the read tool before answering."
            );
            if !notes.contains(&note) {
                notes.push(note);
            }
        }
        notes
    }

    fn maybe_compact(&mut self) {
        let projected = self.projected();
        let tokens_before = estimate_projected(&projected);
        if tokens_before <= self.config.token_budget {
            return;
        }
        let groups = user_group_starts(&projected);
        let keep = self.config.keep_recent_turns.max(1);
        if groups.len() <= keep {
            return;
        }
        let cut = groups[groups.len() - keep];
        if cut == 0 {
            return;
        }
        let older: Vec<SessionEntry> = projected[..cut]
            .iter()
            .map(|(_, entry)| entry.clone())
            .collect();
        if older.is_empty() {
            return;
        }
        let kept_from = projected[cut].0;
        let summary = self.summarize(&older);
        self.session.entries.push(SessionEntry::Compaction {
            text: summary.clone(),
            created_at: now_stamp(),
            kept_from,
        });
        let tokens_after = estimate_projected(&self.projected());
        log::info!(
            "pi_harness[{}]: compaction tokens_before={tokens_before} tokens_after={tokens_after} kept_from={kept_from}",
            self.session.id
        );
        self.events.push(HarnessEvent::Compaction {
            tokens_before,
            tokens_after,
            summary,
        });
    }

    fn summarize(&mut self, older: &[SessionEntry]) -> String {
        for plugin in &mut self.plugins {
            self.events.push(HarnessEvent::OnCompact {
                plugin: plugin.name().to_string(),
            });
            if let Some(summary) = plugin.on_compact(older) {
                return summary;
            }
        }
        default_summary(older)
    }

    fn build_request(&self, extra_system: &str) -> ModelStepRequest {
        let mut system = self.config.system.clone();
        if !extra_system.is_empty() {
            system.push('\n');
            system.push_str(extra_system);
        }
        for plugin in &self.plugins {
            if let Some(block) = plugin.context_block() {
                system.push('\n');
                system.push_str(&block);
            }
        }
        let mut messages = llm_from_projected(&self.projected());
        for note in self.stale_notes() {
            messages.push(LlmMessage {
                role: "user".to_string(),
                content: note,
                tool_calls: Vec::new(),
                tool_call_id: None,
            });
        }
        let estimated_tokens = estimate_messages(&messages) + estimate_text(&system);
        ModelStepRequest {
            provider: self.config.provider.clone(),
            tier: self.config.tier,
            configured_model: self.config.configured_model.clone(),
            system,
            messages,
            tools: self
                .tools
                .iter()
                .map(|tool| tool.handler.name().to_string())
                .collect(),
            estimated_tokens,
        }
    }

    fn projected(&self) -> Vec<(usize, SessionEntry)> {
        let Some(comp_idx) = self
            .session
            .entries
            .iter()
            .rposition(|entry| matches!(entry, SessionEntry::Compaction { .. }))
        else {
            return self.session.entries.iter().cloned().enumerate().collect();
        };
        let (summary, kept_from) = match &self.session.entries[comp_idx] {
            SessionEntry::Compaction {
                text, kept_from, ..
            } => (text.clone(), *kept_from),
            _ => unreachable!("compaction index"),
        };
        let mut out = vec![(
            comp_idx,
            SessionEntry::User {
                text: format!("{SUMMARY_PREFIX}{summary}"),
                created_at: now_stamp(),
            },
        )];
        for (index, entry) in self.session.entries.iter().enumerate() {
            if index >= kept_from
                && index != comp_idx
                && !matches!(entry, SessionEntry::Compaction { .. })
            {
                out.push((index, entry.clone()));
            }
        }
        out
    }
}

fn assign_call_ids(calls: &[ToolCall], next: &mut u64) -> Vec<ToolCall> {
    calls
        .iter()
        .map(|call| {
            let mut call = call.clone();
            if call.id.is_empty() {
                call.id = format!("call-{next}");
                *next += 1;
            }
            call
        })
        .collect()
}

fn llm_from_projected(projected: &[(usize, SessionEntry)]) -> Vec<LlmMessage> {
    projected
        .iter()
        .filter_map(|(_, entry)| match entry {
            SessionEntry::User { text, .. } | SessionEntry::Note { text, .. } => Some(LlmMessage {
                role: "user".to_string(),
                content: text.clone(),
                tool_calls: Vec::new(),
                tool_call_id: None,
            }),
            SessionEntry::Assistant {
                text, tool_calls, ..
            } => Some(LlmMessage {
                role: "assistant".to_string(),
                content: text.clone(),
                tool_calls: tool_calls.clone(),
                tool_call_id: None,
            }),
            SessionEntry::Tool {
                text,
                call_id,
                stale,
                ..
            } => {
                let content = if *stale {
                    format!("[stale] {text}")
                } else {
                    text.clone()
                };
                Some(LlmMessage {
                    role: "tool".to_string(),
                    content,
                    tool_calls: Vec::new(),
                    tool_call_id: Some(call_id.clone()),
                })
            }
            SessionEntry::Compaction { .. } => None,
        })
        .collect()
}

fn user_group_starts(projected: &[(usize, SessionEntry)]) -> Vec<usize> {
    projected
        .iter()
        .enumerate()
        .filter_map(|(index, (_, entry))| match entry {
            SessionEntry::User { text, .. } if !text.starts_with(SUMMARY_PREFIX) => Some(index),
            _ => None,
        })
        .collect()
}

fn default_summary(older: &[SessionEntry]) -> String {
    let mut summary = String::from(DEFAULT_SUMMARY_MARK);
    summary.push('\n');
    for entry in older {
        let line = match entry {
            SessionEntry::User { text, .. } => format!("User: {}", clip(text, 120)),
            SessionEntry::Assistant {
                text, tool_calls, ..
            } => {
                let tools: Vec<&str> = tool_calls.iter().map(|call| call.name.as_str()).collect();
                format!("Assistant: {} tools={tools:?}", clip(text, 120))
            }
            SessionEntry::Tool { name, text, .. } => {
                format!("Tool {name}: {}", clip(text, 120))
            }
            SessionEntry::Note { text, .. } => format!("Note: {}", clip(text, 120)),
            SessionEntry::Compaction { text, .. } => format!("Summary: {}", clip(text, 120)),
        };
        summary.push_str(&line);
        summary.push('\n');
    }
    summary
}

fn clip(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

fn estimate_projected(projected: &[(usize, SessionEntry)]) -> u32 {
    estimate_messages(&llm_from_projected(projected))
}

fn estimate_messages(messages: &[LlmMessage]) -> u32 {
    messages
        .iter()
        .map(|message| {
            estimate_text(&message.content)
                + message
                    .tool_calls
                    .iter()
                    .map(|call| {
                        estimate_text(&call.name) + estimate_text(&call.arguments.to_string())
                    })
                    .sum::<u32>()
        })
        .sum()
}

fn estimate_text(text: &str) -> u32 {
    let chars = text.chars().count() as u32;
    if chars == 0 {
        0
    } else {
        chars.div_ceil(4)
    }
}

fn mtime_ns(path: &str) -> Option<u64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    modified
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|duration| u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX))
}

fn now_stamp() -> String {
    crate::host::event_log::now_timestamp()
}

fn nonzero(value: u32) -> Option<u32> {
    (value > 0).then_some(value)
}

#[cfg(test)]
mod evals;
