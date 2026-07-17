//! Deterministic Assistant turn-loop harness.
//!
//! The harness drives `AssistantApp` without egui. A scripted broker observes
//! the real broker request, dispatches through the real gated tool snapshot,
//! and records every orchestration boundary as typed data.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::model::PermissionChoice;
use super::{AssistantApp, SkillRegistry};
use crate::app::app_trait::{App, AppCommand};
use crate::plexi_ai::broker::{AiBroker, AiBrokerRequest, AiBrokerResponse};
use crate::plexi_ai::tool_dispatch::ToolCallResult;
use crate::plexi_ai::turn_loop::TurnDelta;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum OrchestrationEvent {
    Selection {
        agent: String,
        tier: String,
        provider: Option<String>,
        model: Option<String>,
    },
    SkillsOffered {
        names: Vec<String>,
    },
    SkillActivated {
        name: String,
    },
    ToolCall {
        call_id: String,
        name: String,
        arguments: serde_json::Value,
    },
    PermissionDecision {
        tool: String,
        decision: HarnessPermission,
    },
    HostEffect {
        name: String,
        arguments: serde_json::Value,
    },
    ToolResult {
        call_id: String,
        output: Option<serde_json::Value>,
        error: Option<String>,
    },
    ModelObservedToolResult {
        call_id: String,
        output: Option<serde_json::Value>,
        error: Option<String>,
    },
    FinalOutcome {
        text: Option<String>,
        error: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HarnessPermission {
    AllowOnce,
    AllowSession,
    AllowAlways,
    Deny,
}

impl From<HarnessPermission> for PermissionChoice {
    fn from(value: HarnessPermission) -> Self {
        match value {
            HarnessPermission::AllowOnce => Self::AllowOnce,
            HarnessPermission::AllowSession => Self::AllowSession,
            HarnessPermission::AllowAlways => Self::AllowAlways,
            HarnessPermission::Deny => Self::Deny,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ScriptedCall {
    /// Native `host.*` tool. Connector calls need a real registered app and
    /// belong in HostHarness integration tests.
    pub name: String,
    pub arguments: serde_json::Value,
    pub permission: Option<HarnessPermission>,
    pub result: ToolCallResult,
}

impl ScriptedCall {
    pub fn success(
        name: impl Into<String>,
        arguments: serde_json::Value,
        output: serde_json::Value,
    ) -> Self {
        Self {
            name: name.into(),
            arguments,
            permission: None,
            result: ToolCallResult {
                output_json: Some(output.to_string()),
                error: None,
            },
        }
    }

    pub fn failure(
        name: impl Into<String>,
        arguments: serde_json::Value,
        error: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            arguments,
            permission: None,
            result: ToolCallResult {
                output_json: None,
                error: Some(error.into()),
            },
        }
    }

    pub fn with_permission(mut self, permission: HarnessPermission) -> Self {
        self.permission = Some(permission);
        self
    }
}

#[derive(Debug, Clone)]
pub enum ScriptedOutcome {
    Answer(String),
    Failure(String),
}

#[derive(Debug, Clone)]
pub struct ScriptedTurn {
    pub calls: Vec<ScriptedCall>,
    pub outcome: ScriptedOutcome,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExpectedCall {
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedSelection {
    pub agent: String,
    pub tier: String,
    pub provider: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedPermission {
    pub tool: String,
    pub decision: HarnessPermission,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExpectedToolResult {
    pub output: Option<serde_json::Value>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExpectedEvent {
    Selection(ExpectedSelection),
    SkillsOffered(Vec<String>),
    SkillActivated(String),
    ToolCall(ExpectedCall),
    Permission(ExpectedPermission),
    HostEffect(ExpectedCall),
    ToolResult(ExpectedToolResult),
    ModelObservedToolResult(ExpectedToolResult),
    FinalAnswer(String),
    FinalError(String),
}

impl ExpectedEvent {
    fn matches(&self, actual: &OrchestrationEvent) -> bool {
        match (self, actual) {
            (
                Self::Selection(expected),
                OrchestrationEvent::Selection {
                    agent,
                    tier,
                    provider,
                    model,
                },
            ) => {
                expected.agent == *agent
                    && expected.tier == *tier
                    && expected.provider == *provider
                    && expected.model == *model
            }
            (Self::SkillActivated(expected), OrchestrationEvent::SkillActivated { name }) => {
                expected == name
            }
            (Self::SkillsOffered(expected), OrchestrationEvent::SkillsOffered { names }) => {
                expected == names
            }
            (
                Self::ToolCall(expected),
                OrchestrationEvent::ToolCall {
                    name, arguments, ..
                },
            )
            | (Self::HostEffect(expected), OrchestrationEvent::HostEffect { name, arguments }) => {
                expected.name == *name && expected.arguments == *arguments
            }
            (
                Self::Permission(expected),
                OrchestrationEvent::PermissionDecision { tool, decision },
            ) => expected.tool == *tool && expected.decision == *decision,
            (Self::ToolResult(expected), OrchestrationEvent::ToolResult { output, error, .. })
            | (
                Self::ModelObservedToolResult(expected),
                OrchestrationEvent::ModelObservedToolResult { output, error, .. },
            ) => expected.output == *output && expected.error == *error,
            (
                Self::FinalAnswer(expected),
                OrchestrationEvent::FinalOutcome {
                    text: Some(text),
                    error: None,
                },
            ) => expected == text,
            (
                Self::FinalError(expected),
                OrchestrationEvent::FinalOutcome {
                    text: None,
                    error: Some(error),
                },
            ) => expected == error,
            _ => false,
        }
    }
}

#[derive(Debug, Default)]
pub struct TraceAssertions {
    pub selection: Option<ExpectedSelection>,
    pub offered_skills: Option<Vec<String>>,
    pub activated_skill: Option<String>,
    pub calls_in_order: Vec<ExpectedCall>,
    pub permission_decisions: Vec<ExpectedPermission>,
    pub host_effects_in_order: Vec<ExpectedCall>,
    pub tool_results_in_order: Vec<ExpectedToolResult>,
    /// Required typed events in temporal order. Unrelated events may appear
    /// between them.
    pub ordered_events: Vec<ExpectedEvent>,
    pub reject_unexpected_calls: bool,
    pub final_answer: Option<String>,
    pub final_error: Option<String>,
}

impl TraceAssertions {
    pub fn check(&self, trace: &[OrchestrationEvent]) -> Result<(), String> {
        if let Some(expected) = &self.selection {
            let found = trace
                .iter()
                .any(|event| ExpectedEvent::Selection(expected.clone()).matches(event));
            if !found {
                return Err(format!("required selection was not observed: {expected:?}"));
            }
        }
        if let Some(expected) = &self.activated_skill {
            let found = trace.iter().any(|event| {
                matches!(event, OrchestrationEvent::SkillActivated { name } if name == expected)
            });
            if !found {
                return Err(format!("required skill '{expected}' was not activated"));
            }
        }
        if let Some(expected) = &self.offered_skills {
            let found = trace
                .iter()
                .any(|event| ExpectedEvent::SkillsOffered(expected.clone()).matches(event));
            if !found {
                return Err(format!("offered skills mismatch: expected {expected:?}"));
            }
        }
        let actual_calls = trace
            .iter()
            .filter_map(|event| match event {
                OrchestrationEvent::ToolCall {
                    name, arguments, ..
                } => Some(ExpectedCall {
                    name: name.clone(),
                    arguments: arguments.clone(),
                }),
                _ => None,
            })
            .collect::<Vec<_>>();
        if self.reject_unexpected_calls && actual_calls.len() != self.calls_in_order.len() {
            return Err(format!(
                "expected {} tool call(s), observed {}",
                self.calls_in_order.len(),
                actual_calls.len()
            ));
        }
        for (index, expected) in self.calls_in_order.iter().enumerate() {
            let Some(actual) = actual_calls.get(index) else {
                return Err(format!("missing required tool call {}", expected.name));
            };
            if actual != expected {
                return Err(format!(
                    "tool call {index} mismatch: expected {expected:?}, observed {actual:?}"
                ));
            }
        }
        let permissions = trace
            .iter()
            .filter_map(|event| match event {
                OrchestrationEvent::PermissionDecision { tool, decision } => {
                    Some(ExpectedPermission {
                        tool: tool.clone(),
                        decision: *decision,
                    })
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        if permissions != self.permission_decisions {
            return Err(format!(
                "permission decisions mismatch: expected {:?}, observed {:?}",
                self.permission_decisions, permissions
            ));
        }
        let host_effects = trace
            .iter()
            .filter_map(|event| match event {
                OrchestrationEvent::HostEffect { name, arguments } => Some(ExpectedCall {
                    name: name.clone(),
                    arguments: arguments.clone(),
                }),
                _ => None,
            })
            .collect::<Vec<_>>();
        if host_effects != self.host_effects_in_order {
            return Err(format!(
                "host effects mismatch: expected {:?}, observed {:?}",
                self.host_effects_in_order, host_effects
            ));
        }
        let tool_results = trace
            .iter()
            .filter_map(|event| match event {
                OrchestrationEvent::ToolResult { output, error, .. } => Some(ExpectedToolResult {
                    output: output.clone(),
                    error: error.clone(),
                }),
                _ => None,
            })
            .collect::<Vec<_>>();
        if tool_results != self.tool_results_in_order {
            return Err(format!(
                "tool results mismatch: expected {:?}, observed {:?}",
                self.tool_results_in_order, tool_results
            ));
        }
        let mut next_index = 0usize;
        for expected in &self.ordered_events {
            let Some(relative) = trace[next_index..]
                .iter()
                .position(|actual| expected.matches(actual))
            else {
                return Err(format!(
                    "ordered event was missing after trace index {next_index}: {expected:?}"
                ));
            };
            next_index += relative + 1;
        }
        let final_event = trace.iter().rev().find_map(|event| match event {
            OrchestrationEvent::FinalOutcome { text, error } => Some((text, error)),
            _ => None,
        });
        let Some((text, error)) = final_event else {
            return Err("trace has no final outcome".to_string());
        };
        if text.as_ref() != self.final_answer.as_ref()
            || error.as_ref() != self.final_error.as_ref()
        {
            return Err(format!(
                "final outcome mismatch: observed text={text:?} error={error:?}"
            ));
        }
        Ok(())
    }
}

struct ScriptState {
    calls: VecDeque<ScriptedCall>,
    outcome: ScriptedOutcome,
}

struct ScriptedBroker {
    state: Mutex<Option<ScriptState>>,
    trace: Arc<Mutex<Vec<OrchestrationEvent>>>,
    skills: Mutex<Vec<(String, String)>>,
    agent_id: Mutex<String>,
}

impl AiBroker for ScriptedBroker {
    fn dispatch(
        &self,
        request: AiBrokerRequest,
        on_delta: &mut dyn FnMut(TurnDelta<'_>),
    ) -> AiBrokerResponse {
        let Some(mut state) = self.state.lock().unwrap().take() else {
            return AiBrokerResponse::err("script_exhausted: no turn was queued");
        };
        let route = request.concrete_model.as_ref();
        let agent = self.agent_id.lock().unwrap().clone();
        let mut trace = self.trace.lock().unwrap();
        trace.push(OrchestrationEvent::Selection {
            agent,
            tier: super::settings::model_tier_name(request.model_tier).to_string(),
            provider: route.map(|route| route.provider.clone()),
            model: route.map(|route| route.model.clone()),
        });
        let skills = self.skills.lock().unwrap();
        trace.push(OrchestrationEvent::SkillsOffered {
            names: skills.iter().map(|(name, _)| name.clone()).collect(),
        });
        for (name, instructions) in skills.iter() {
            if request.system.contains(instructions) {
                trace.push(OrchestrationEvent::SkillActivated { name: name.clone() });
            }
        }
        drop(trace);

        let Some(dispatcher) = request.tool_dispatcher else {
            return AiBrokerResponse::err("script_invalid: assistant supplied no tool dispatcher");
        };
        while let Some(call) = state.calls.pop_front() {
            let call_id = format!("script-call-{}", self.trace.lock().unwrap().len());
            self.trace
                .lock()
                .unwrap()
                .push(OrchestrationEvent::ToolCall {
                    call_id: call_id.clone(),
                    name: call.name.clone(),
                    arguments: call.arguments.clone(),
                });
            let result =
                dispatcher.dispatch_call(call_id.clone(), &call.name, call.arguments.to_string());
            let output = result
                .output_json
                .as_deref()
                .and_then(|json| serde_json::from_str(json).ok());
            self.trace
                .lock()
                .unwrap()
                .push(OrchestrationEvent::ToolResult {
                    call_id: call_id.clone(),
                    output: output.clone(),
                    error: result.error.clone(),
                });
            self.trace
                .lock()
                .unwrap()
                .push(OrchestrationEvent::ModelObservedToolResult {
                    call_id,
                    output,
                    error: result.error,
                });
        }
        match state.outcome {
            ScriptedOutcome::Answer(text) => {
                on_delta(TurnDelta::Text(&text));
                self.trace
                    .lock()
                    .unwrap()
                    .push(OrchestrationEvent::FinalOutcome {
                        text: Some(text.clone()),
                        error: None,
                    });
                AiBrokerResponse::ok(text, 0, 0)
            }
            ScriptedOutcome::Failure(error) => {
                self.trace
                    .lock()
                    .unwrap()
                    .push(OrchestrationEvent::FinalOutcome {
                        text: None,
                        error: Some(error.clone()),
                    });
                AiBrokerResponse::err(error)
            }
        }
    }
}

pub struct AssistantHarness {
    _root: tempfile::TempDir,
    workspace: PathBuf,
    app: AssistantApp,
    broker: Arc<ScriptedBroker>,
    trace: Arc<Mutex<Vec<OrchestrationEvent>>>,
}

impl AssistantHarness {
    pub fn new(skills: &[(&str, &str, &str)]) -> Self {
        Self::new_with_route(skills, None)
    }

    pub fn with_agent_route(
        skills: &[(&str, &str, &str)],
        agent_id: &str,
        provider: &str,
        model: &str,
    ) -> Self {
        Self::new_with_route(skills, Some((agent_id, provider, model)))
    }

    fn new_with_route(skills: &[(&str, &str, &str)], route: Option<(&str, &str, &str)>) -> Self {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        let profile = root.path().join("profile");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::create_dir_all(&profile).unwrap();
        if let Some((agent_id, provider, model)) = route {
            let agent_dir = profile.join("agents").join(agent_id);
            std::fs::create_dir_all(&agent_dir).unwrap();
            std::fs::write(agent_dir.join("AGENT.md"), "Test orchestration exactly.").unwrap();
            std::fs::write(
                agent_dir.join("settings.toml"),
                format!(
                    "[agent]\nid = \"{agent_id}\"\ndisplay_name = \"Harness agent\"\ndefault_tier = \"low\"\n\n[permissions]\ndefault_posture = \"ask\"\n\n[models.low]\nprovider = \"{provider}\"\nmodel = \"{model}\"\n"
                ),
            )
            .unwrap();
        }
        for (name, description, instructions) in skills {
            let dir = workspace.join(".plexi/skills").join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("SKILL.md"),
                format!("---\nname: {name}\ndescription: {description}\n---\n{instructions}\n"),
            )
            .unwrap();
        }
        let probes = SkillRegistry::load(&profile, &workspace)
            .all()
            .iter()
            .map(|skill| (skill.name.clone(), skill.instructions.clone()))
            .collect();
        let trace = Arc::new(Mutex::new(Vec::new()));
        let broker = Arc::new(ScriptedBroker {
            state: Mutex::new(None),
            trace: Arc::clone(&trace),
            skills: Mutex::new(probes),
            agent_id: Mutex::new("default".to_string()),
        });
        let mut app = AssistantApp::new(workspace.clone(), broker.clone(), &profile);
        if let Some((agent_id, _, _)) = route {
            app.model.active_agent_id = agent_id.to_string();
        }
        Self {
            _root: root,
            workspace,
            app,
            broker,
            trace,
        }
    }

    pub fn run_turn(
        &mut self,
        user_input: &str,
        script: ScriptedTurn,
    ) -> Result<Vec<OrchestrationEvent>, String> {
        if let Some(call) = script
            .calls
            .iter()
            .find(|call| !call.name.starts_with("host."))
        {
            return Err(format!(
                "scripted result injection supports native host tools only; '{}' needs a registered app connector",
                call.name
            ));
        }
        self.trace.lock().unwrap().clear();
        *self.broker.agent_id.lock().unwrap() = self.app.model.active_agent_id.clone();
        let enabled = self
            .app
            .active_agent()
            .map(|agent| agent.skills.clone())
            .unwrap_or_default();
        *self.broker.skills.lock().unwrap() = self
            .app
            .skill_registry
            .all()
            .iter()
            .filter(|skill| enabled.is_empty() || enabled.contains(&skill.name))
            .map(|skill| (skill.name.clone(), skill.instructions.clone()))
            .collect();
        let calls_for_host = script.calls.clone();
        *self.broker.state.lock().unwrap() = Some(ScriptState {
            calls: script.calls.into(),
            outcome: script.outcome,
        });
        let mut host_results = calls_for_host.into_iter().collect::<VecDeque<_>>();
        self.app.model.composer = user_input.to_string();
        let effects = self.app.model.submit();
        self.app.execute_effects(effects);

        let started = Instant::now();
        while self.app.model.streaming.in_flight {
            self.app.pump_turn_io();
            if let Some(pending) = self.app.model.pending_permission.clone() {
                let decision = host_results
                    .iter()
                    .find(|call| call.name == pending.tool)
                    .and_then(|call| call.permission)
                    .ok_or_else(|| {
                        format!("script omitted permission decision for '{}'", pending.tool)
                    })?;
                self.trace
                    .lock()
                    .unwrap()
                    .push(OrchestrationEvent::PermissionDecision {
                        tool: pending.tool,
                        decision,
                    });
                self.app.resolve_permission(decision.into());
            }
            for command in App::take_pending_commands(&mut self.app) {
                let AppCommand::AssistantHostTool {
                    name,
                    input_json,
                    reply,
                    ..
                } = command
                else {
                    return Err("assistant emitted an unexpected non-host command".to_string());
                };
                let arguments = serde_json::from_str(&input_json)
                    .map_err(|error| format!("host effect arguments were invalid JSON: {error}"))?;
                self.trace
                    .lock()
                    .unwrap()
                    .push(OrchestrationEvent::HostEffect {
                        name: name.clone(),
                        arguments,
                    });
                let index = host_results
                    .iter()
                    .position(|call| call.name == name)
                    .ok_or_else(|| format!("unexpected host effect '{name}'"))?;
                let call = host_results.remove(index).unwrap();
                reply
                    .send(call.result)
                    .map_err(|_| format!("model dropped host effect reply for '{name}'"))?;
            }
            if started.elapsed() > Duration::from_secs(5) {
                return Err("assistant harness turn timed out".to_string());
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        if self.broker.state.lock().unwrap().is_some() {
            return Err("assistant never dispatched the scripted model turn".to_string());
        }
        Ok(self.trace.lock().unwrap().clone())
    }

    pub fn transcript_path(&self) -> PathBuf {
        self.workspace
            .join(crate::config::workspace_channel_dir())
            .join("assistant/conversations")
            .join(format!("{}.jsonl", self.app.model.conversation_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripted_turn_records_skill_permission_tool_result_and_final_answer() {
        let mut harness = AssistantHarness::with_agent_route(
            &[(
                "pane-audit",
                "audit workspace panes",
                "Use host pane tools and report their typed results.",
            )],
            "verifier",
            "openrouter",
            "test/cheap",
        );
        let args = serde_json::json!({"type_id": "file_browser", "layout": "split_h"});
        let trace = harness
            .run_turn(
                "/pane-audit open the file browser",
                ScriptedTurn {
                    calls: vec![ScriptedCall::success(
                        "host.panes.open",
                        args.clone(),
                        serde_json::json!({"ok": true, "pane_id": 44}),
                    )
                    .with_permission(HarnessPermission::AllowOnce)],
                    outcome: ScriptedOutcome::Answer("Opened pane 44.".to_string()),
                },
            )
            .unwrap();

        TraceAssertions {
            selection: Some(ExpectedSelection {
                agent: "verifier".to_string(),
                tier: "low".to_string(),
                provider: Some("openrouter".to_string()),
                model: Some("test/cheap".to_string()),
            }),
            offered_skills: Some(vec![
                "build-plexi-app".to_string(),
                "pane-audit".to_string(),
            ]),
            activated_skill: Some("pane-audit".to_string()),
            calls_in_order: vec![ExpectedCall {
                name: "host.panes.open".to_string(),
                arguments: args,
            }],
            reject_unexpected_calls: true,
            permission_decisions: vec![ExpectedPermission {
                tool: "host.panes.open".to_string(),
                decision: HarnessPermission::AllowOnce,
            }],
            host_effects_in_order: vec![ExpectedCall {
                name: "host.panes.open".to_string(),
                arguments: serde_json::json!({"type_id": "file_browser", "layout": "split_h"}),
            }],
            tool_results_in_order: vec![ExpectedToolResult {
                output: Some(serde_json::json!({"ok": true, "pane_id": 44})),
                error: None,
            }],
            ordered_events: vec![
                ExpectedEvent::SkillsOffered(vec![
                    "build-plexi-app".to_string(),
                    "pane-audit".to_string(),
                ]),
                ExpectedEvent::SkillActivated("pane-audit".to_string()),
                ExpectedEvent::ToolCall(ExpectedCall {
                    name: "host.panes.open".to_string(),
                    arguments: serde_json::json!({"type_id": "file_browser", "layout": "split_h"}),
                }),
                ExpectedEvent::Permission(ExpectedPermission {
                    tool: "host.panes.open".to_string(),
                    decision: HarnessPermission::AllowOnce,
                }),
                ExpectedEvent::HostEffect(ExpectedCall {
                    name: "host.panes.open".to_string(),
                    arguments: serde_json::json!({"type_id": "file_browser", "layout": "split_h"}),
                }),
                ExpectedEvent::ToolResult(ExpectedToolResult {
                    output: Some(serde_json::json!({"ok": true, "pane_id": 44})),
                    error: None,
                }),
                ExpectedEvent::ModelObservedToolResult(ExpectedToolResult {
                    output: Some(serde_json::json!({"ok": true, "pane_id": 44})),
                    error: None,
                }),
                ExpectedEvent::FinalAnswer("Opened pane 44.".to_string()),
            ],
            final_answer: Some("Opened pane 44.".to_string()),
            final_error: None,
        }
        .check(&trace)
        .unwrap();
        assert!(trace.iter().any(|event| matches!(
            event,
            OrchestrationEvent::ModelObservedToolResult {
                output: Some(output), ..
            } if output["pane_id"] == 44
        )));
        assert!(harness.transcript_path().is_file());
    }

    #[test]
    fn scripted_failure_is_returned_to_model_and_unexpected_calls_are_rejected() {
        let mut harness = AssistantHarness::new(&[]);
        let args = serde_json::json!({"pane_id": 999});
        let trace = harness
            .run_turn(
                "close pane 999",
                ScriptedTurn {
                    calls: vec![ScriptedCall::failure(
                        "host.panes.close",
                        args.clone(),
                        "pane_not_found: 999",
                    )
                    .with_permission(HarnessPermission::AllowOnce)],
                    outcome: ScriptedOutcome::Failure("model_failed_after_tool".to_string()),
                },
            )
            .unwrap();
        TraceAssertions {
            calls_in_order: vec![ExpectedCall {
                name: "host.panes.close".to_string(),
                arguments: args,
            }],
            reject_unexpected_calls: true,
            permission_decisions: vec![ExpectedPermission {
                tool: "host.panes.close".to_string(),
                decision: HarnessPermission::AllowOnce,
            }],
            host_effects_in_order: vec![ExpectedCall {
                name: "host.panes.close".to_string(),
                arguments: serde_json::json!({"pane_id": 999}),
            }],
            tool_results_in_order: vec![ExpectedToolResult {
                output: None,
                error: Some("pane_not_found: 999".to_string()),
            }],
            ordered_events: vec![
                ExpectedEvent::ToolResult(ExpectedToolResult {
                    output: None,
                    error: Some("pane_not_found: 999".to_string()),
                }),
                ExpectedEvent::ModelObservedToolResult(ExpectedToolResult {
                    output: None,
                    error: Some("pane_not_found: 999".to_string()),
                }),
                ExpectedEvent::FinalError("model_failed_after_tool".to_string()),
            ],
            final_error: Some("model_failed_after_tool".to_string()),
            ..Default::default()
        }
        .check(&trace)
        .unwrap();
        assert!(trace.iter().any(|event| matches!(
            event,
            OrchestrationEvent::ModelObservedToolResult {
                error: Some(error), ..
            } if error == "pane_not_found: 999"
        )));
    }

    #[test]
    fn assertions_reject_wrong_arguments_and_repeated_unexpected_calls() {
        let mut harness = AssistantHarness::new(&[]);
        let trace = harness
            .run_turn(
                "list panes",
                ScriptedTurn {
                    calls: vec![
                        ScriptedCall::success(
                            "host.panes.list",
                            serde_json::json!({}),
                            serde_json::json!([]),
                        ),
                        ScriptedCall::success(
                            "host.panes.list",
                            serde_json::json!({}),
                            serde_json::json!([]),
                        ),
                    ],
                    outcome: ScriptedOutcome::Answer("No panes.".to_string()),
                },
            )
            .unwrap();
        let assertions = TraceAssertions {
            calls_in_order: vec![ExpectedCall {
                name: "host.panes.list".to_string(),
                arguments: serde_json::json!({"wrong": true}),
            }],
            reject_unexpected_calls: true,
            host_effects_in_order: vec![
                ExpectedCall {
                    name: "host.panes.list".to_string(),
                    arguments: serde_json::json!({}),
                },
                ExpectedCall {
                    name: "host.panes.list".to_string(),
                    arguments: serde_json::json!({}),
                },
            ],
            tool_results_in_order: vec![
                ExpectedToolResult {
                    output: Some(serde_json::json!([])),
                    error: None,
                },
                ExpectedToolResult {
                    output: Some(serde_json::json!([])),
                    error: None,
                },
            ],
            final_answer: Some("No panes.".to_string()),
            ..Default::default()
        };
        assert!(assertions
            .check(&trace)
            .unwrap_err()
            .contains("expected 1 tool call"));

        let wrong_args_only = TraceAssertions {
            calls_in_order: assertions.calls_in_order,
            reject_unexpected_calls: false,
            host_effects_in_order: assertions.host_effects_in_order,
            tool_results_in_order: assertions.tool_results_in_order,
            final_answer: assertions.final_answer,
            ..Default::default()
        };
        assert!(wrong_args_only
            .check(&trace)
            .unwrap_err()
            .contains("tool call 0 mismatch"));
    }

    /// Stint 0421: `host.build.run` executes on the embedded build surface —
    /// ask-gated, no terminal pane, real subprocess output returned to the
    /// model in one call.
    #[test]
    fn build_run_executes_embedded_command_behind_ask_gate() {
        let mut harness = AssistantHarness::new(&[]);
        let stub = harness.workspace.join("stub-plexi.sh");
        std::fs::write(&stub, "#!/bin/sh\necho \"stub-build $@\"\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        harness.app.build_exe = stub;

        let args = serde_json::json!({"args": ["app", "init", "demo-app"]});
        let trace = harness
            .run_turn(
                "build me a demo app",
                ScriptedTurn {
                    calls: vec![ScriptedCall::success(
                        "host.build.run",
                        args.clone(),
                        serde_json::json!({"unused": "real exec provides the output"}),
                    )
                    .with_permission(HarnessPermission::AllowOnce)],
                    outcome: ScriptedOutcome::Answer("Scaffolded.".to_string()),
                },
            )
            .unwrap();

        assert!(
            trace.iter().any(|event| matches!(
                event,
                OrchestrationEvent::PermissionDecision { tool, decision }
                    if tool == "host.build.run" && *decision == HarnessPermission::AllowOnce
            )),
            "build.run must pass through the ask gate: {trace:?}"
        );
        assert!(
            !trace
                .iter()
                .any(|event| matches!(event, OrchestrationEvent::HostEffect { .. })),
            "the embedded surface must not route through a pane host effect: {trace:?}"
        );
        let output = trace
            .iter()
            .find_map(|event| match event {
                OrchestrationEvent::ModelObservedToolResult {
                    output: Some(output),
                    ..
                } => Some(output.clone()),
                _ => None,
            })
            .expect("model must observe the real exec output");
        assert_eq!(output["exit_code"], 0, "{output}");
        assert_eq!(output["timed_out"], false, "{output}");
        assert!(
            output["stdout"]
                .as_str()
                .unwrap()
                .contains("stub-build app init demo-app"),
            "{output}"
        );
    }

    /// A disallowed subcommand never spawns: the validation error comes back
    /// as the tool error after the gate approves the call.
    #[test]
    fn build_run_rejects_non_allowlisted_commands() {
        let mut harness = AssistantHarness::new(&[]);
        let args = serde_json::json!({"args": ["host", "stop"]});
        let trace = harness
            .run_turn(
                "stop the host",
                ScriptedTurn {
                    calls: vec![ScriptedCall::success(
                        "host.build.run",
                        args,
                        serde_json::json!({}),
                    )
                    .with_permission(HarnessPermission::AllowOnce)],
                    outcome: ScriptedOutcome::Answer("Cannot.".to_string()),
                },
            )
            .unwrap();
        let error = trace
            .iter()
            .find_map(|event| match event {
                OrchestrationEvent::ToolResult {
                    error: Some(error), ..
                } => Some(error.clone()),
                _ => None,
            })
            .expect("disallowed command must fail as a tool error");
        assert!(error.starts_with("command_not_allowed"), "{error}");
    }

    #[test]
    fn connector_result_injection_fails_before_dispatch() {
        let mut harness = AssistantHarness::new(&[]);
        let error = harness
            .run_turn(
                "query app",
                ScriptedTurn {
                    calls: vec![ScriptedCall::success(
                        "csv.read",
                        serde_json::json!({}),
                        serde_json::json!({"rows": []}),
                    )],
                    outcome: ScriptedOutcome::Answer("done".to_string()),
                },
            )
            .unwrap_err();
        assert!(error.contains("supports native host tools only"));
    }
}
