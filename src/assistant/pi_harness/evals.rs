//! Deterministic Pi-harness evals. No network. The live OpenRouter replay
//! is `#[ignore]` and returns immediately unless `PLEXI_LIVE_EVALS=1` and
//! `OPENROUTER_API_KEY` are both set.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use serde_json::{json, Value};

use super::*;
use crate::plexi_ai::broker::{AiBroker, AiBrokerRequest, AiBrokerResponse};
use crate::plexi_ai::turn_loop::TurnDelta;

/// Queued provider replies. No network.
struct ScriptedModel {
    steps: VecDeque<ModelStep>,
}

impl ScriptedModel {
    fn new(steps: Vec<ModelStep>) -> Self {
        Self {
            steps: steps.into(),
        }
    }
}

impl ModelStepper for ScriptedModel {
    fn step(
        &mut self,
        _request: &ModelStepRequest,
        _on_delta: &mut dyn FnMut(TurnDelta<'_>),
    ) -> Result<ModelStep, String> {
        self.steps
            .pop_front()
            .ok_or_else(|| "script exhausted".to_string())
    }
}

struct ScriptedPermissions {
    choices: VecDeque<PermissionChoice>,
}

impl ScriptedPermissions {
    fn new(choices: Vec<PermissionChoice>) -> Self {
        Self {
            choices: choices.into(),
        }
    }
}

impl PermissionDecider for ScriptedPermissions {
    fn decide(&mut self, _tool: &str, _args: &Value) -> PermissionChoice {
        self.choices.pop_front().unwrap_or(PermissionChoice::Deny)
    }
}

const SERVED: &str = "xiaomi/mimo-v2.5";
const CONFIGURED_ALIAS: &str = "xiaomi/mimo-v2.5-pro";

fn isolated() -> (tempfile::TempDir, crate::config::TestProfileDirGuard) {
    let dir = tempfile::tempdir().expect("tempdir");
    let guard = crate::config::set_test_profile_dir(dir.path().to_path_buf());
    (dir, guard)
}

fn config(session_id: &str) -> PiConfig {
    PiConfig {
        provider: "openrouter".to_string(),
        tier: crate::protocol::ModelTier::Medium,
        configured_model: SERVED.to_string(),
        system: "You are the assistant.".to_string(),
        token_budget: 100_000,
        keep_recent_turns: 8,
        max_steps: 8,
        session_id: session_id.to_string(),
    }
}

fn scripted(session_id: &str, steps: Vec<ModelStep>, profile: &std::path::Path) -> PiHarness {
    PiHarness::new(
        config(session_id),
        Box::new(ScriptedModel::new(steps)),
        profile,
    )
}

fn assert_pairs(messages: &[LlmMessage]) {
    let mut pending: Vec<String> = Vec::new();
    for message in messages {
        if message.role == "assistant" {
            for call in &message.tool_calls {
                pending.push(call.id.clone());
            }
        } else if message.role == "tool" {
            let id = message
                .tool_call_id
                .clone()
                .expect("tool message has a call id");
            let index = pending
                .iter()
                .position(|pending_id| pending_id == &id)
                .unwrap_or_else(|| panic!("orphaned tool result {id}"));
            pending.remove(index);
        }
    }
    assert!(
        pending.is_empty(),
        "tool calls missing results: {pending:?}"
    );
}

fn blob(messages: &[LlmMessage]) -> String {
    messages
        .iter()
        .map(|message| message.content.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

struct ReadTool {
    body: String,
}

impl ToolHandler for ReadTool {
    fn name(&self) -> &str {
        "read"
    }

    fn execute(&mut self, args: &Value) -> Result<ToolOutput, String> {
        let path = args
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or("missing")
            .to_string();
        Ok(ToolOutput::read(path, self.body.clone()))
    }
}

struct TextTool {
    name: &'static str,
    text: String,
}

impl ToolHandler for TextTool {
    fn name(&self) -> &str {
        self.name
    }

    fn execute(&mut self, _args: &Value) -> Result<ToolOutput, String> {
        Ok(ToolOutput::text(self.text.clone()))
    }
}

struct FailingTool;

impl ToolHandler for FailingTool {
    fn name(&self) -> &str {
        "read"
    }

    fn execute(&mut self, _args: &Value) -> Result<ToolOutput, String> {
        Err("disk failed".to_string())
    }
}

struct CountingTool {
    hits: Arc<AtomicUsize>,
}

impl ToolHandler for CountingTool {
    fn name(&self) -> &str {
        "read"
    }

    fn execute(&mut self, _args: &Value) -> Result<ToolOutput, String> {
        self.hits.fetch_add(1, Ordering::SeqCst);
        Ok(ToolOutput::text("should-not-run"))
    }
}

struct SteeringTool {
    steering: SteeringHandle,
}

impl ToolHandler for SteeringTool {
    fn name(&self) -> &str {
        "read"
    }

    fn execute(&mut self, _args: &Value) -> Result<ToolOutput, String> {
        self.steering.steer("use the short answer");
        Ok(ToolOutput::text("tool-ok"))
    }
}

struct PanicTool;

impl ToolHandler for PanicTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn execute(&mut self, _args: &Value) -> Result<ToolOutput, String> {
        panic!("vetoed tool must not run");
    }
}

struct OrderPlugin {
    label: &'static str,
    log: Arc<Mutex<Vec<String>>>,
    veto_tool: Option<&'static str>,
}

impl PiPlugin for OrderPlugin {
    fn name(&self) -> &str {
        self.label
    }

    fn before_turn(&mut self, _turn: &mut BeforeTurn<'_>) {
        self.log
            .lock()
            .expect("log")
            .push(format!("{}:before_turn", self.label));
    }

    fn before_tool(&mut self, call: &ToolCall) -> ToolGate {
        self.log
            .lock()
            .expect("log")
            .push(format!("{}:before_tool", self.label));
        if self.veto_tool == Some(call.name.as_str()) {
            ToolGate::Veto {
                reason: "bash is disabled".to_string(),
            }
        } else {
            ToolGate::Allow
        }
    }

    fn after_tool(&mut self, _call: &ToolCall, _result: &mut ToolResultDraft<'_>) {
        self.log
            .lock()
            .expect("log")
            .push(format!("{}:after_tool", self.label));
    }

    fn on_compact(&mut self, _older: &[SessionEntry]) -> Option<String> {
        self.log
            .lock()
            .expect("log")
            .push(format!("{}:on_compact", self.label));
        None
    }

    fn on_turn_end(&mut self, _text: Option<&str>, _error: Option<&str>) {
        self.log
            .lock()
            .expect("log")
            .push(format!("{}:on_turn_end", self.label));
    }
}

struct SummaryPlugin;

impl PiPlugin for SummaryPlugin {
    fn name(&self) -> &str {
        "summary"
    }

    fn on_compact(&mut self, _older: &[SessionEntry]) -> Option<String> {
        Some("PLUGIN SUMMARY".to_string())
    }
}

struct AbortOnCall {
    abort: AbortHandle,
}

impl ModelStepper for AbortOnCall {
    fn step(
        &mut self,
        _request: &ModelStepRequest,
        _on_delta: &mut dyn FnMut(TurnDelta<'_>),
    ) -> Result<ModelStep, String> {
        self.abort.abort();
        Ok(ModelStep::calls(
            SERVED,
            vec![ToolCall::new("c1", "read", json!({}))],
        ))
    }
}

struct PaneModel {
    path: String,
    answered: bool,
}

impl ModelStepper for PaneModel {
    fn step(
        &mut self,
        request: &ModelStepRequest,
        _on_delta: &mut dyn FnMut(TurnDelta<'_>),
    ) -> Result<ModelStep, String> {
        assert!(
            request.system.contains("pane-excerpt-hello"),
            "pane excerpt missing from system"
        );
        assert!(request.system.contains(&self.path));
        assert!(
            !request
                .tools
                .iter()
                .any(|name| name.contains("grep") || name.contains("list")),
            "grep/list must not be required: {:?}",
            request.tools
        );
        if self.answered {
            return Ok(ModelStep::text(SERVED, "from pane-excerpt-hello"));
        }
        self.answered = true;
        Ok(ModelStep::calls(
            SERVED,
            vec![ToolCall::new("c1", "read", json!({ "path": self.path }))],
        ))
    }
}

struct RecordingBroker {
    dispatches: AtomicUsize,
    providers: Mutex<Vec<String>>,
}

impl AiBroker for RecordingBroker {
    fn dispatch(
        &self,
        _request: AiBrokerRequest,
        _on_delta: &mut dyn FnMut(TurnDelta<'_>),
    ) -> AiBrokerResponse {
        self.dispatches.fetch_add(1, Ordering::SeqCst);
        AiBrokerResponse::err("dispatch must not run for a pi step")
    }

    fn complete_once(
        &self,
        request: AiBrokerRequest,
        _on_delta: &mut dyn FnMut(TurnDelta<'_>),
    ) -> AiBrokerResponse {
        let route = request.concrete_model.clone().expect("concrete route");
        self.providers
            .lock()
            .expect("providers")
            .push(route.provider);
        assert!(request.single_completion);
        assert!(!request.structured_messages.is_empty());
        let mut response = AiBrokerResponse::ok("provider-ok".to_string(), 2, 3);
        response.model_id = Some(route.model);
        response
    }
}

fn read_call(path: &str) -> ModelStep {
    ModelStep::calls(
        SERVED,
        vec![ToolCall::new("c1", "read", json!({ "path": path }))],
    )
}

#[test]
fn history_integrity_keeps_tool_pairs_and_file_read() {
    let (dir, _guard) = isolated();
    let file = dir.path().join("temp.md");
    std::fs::write(&file, "alpha-bravo-temp-contents").expect("write");
    let path = file.display().to_string();
    let mut harness = scripted(
        "history",
        vec![
            read_call(&path),
            ModelStep::text(SERVED, "saw alpha-bravo-temp-contents"),
            ModelStep::text(SERVED, "still alpha-bravo-temp-contents"),
        ],
        dir.path(),
    );
    harness.register_tool(
        false,
        Box::new(ReadTool {
            body: "alpha-bravo-temp-contents".to_string(),
        }),
    );
    let first = harness.run_user_turn("read temp.md");
    assert!(first.error.is_none(), "{first:?}");
    assert_eq!(harness.requests().len(), 2);
    for request in harness.requests() {
        assert_pairs(&request.messages);
    }
    let paired = &harness.requests()[1].messages;
    assert!(blob(paired).contains("alpha-bravo-temp-contents"));
    assert!(paired.iter().any(|message| message.role == "tool"));

    let second = harness.run_user_turn("what was in the file?");
    assert!(second.error.is_none(), "{second:?}");
    let next = harness.requests().last().expect("next request");
    assert_pairs(&next.messages);
    assert!(
        blob(&next.messages).contains("alpha-bravo-temp-contents"),
        "previous file read dropped from the next turn"
    );
}

#[test]
fn compaction_summarizes_older_turns_and_plugin_can_replace_strategy() {
    let (dir, _guard) = isolated();
    let ancient = "ANCIENT-TURN".repeat(200);
    let mut plain = config("compact");
    plain.token_budget = 80;
    plain.keep_recent_turns = 1;
    let mut harness = PiHarness::new(
        plain.clone(),
        Box::new(ScriptedModel::new(vec![
            ModelStep::text(SERVED, "ack-ancient"),
            ModelStep::text(SERVED, "ack-recent"),
        ])),
        dir.path(),
    );
    assert!(harness.run_user_turn(&ancient).error.is_none());
    assert!(
        harness
            .events()
            .iter()
            .all(|event| !matches!(event, HarnessEvent::Compaction { .. })),
        "a single turn must stay verbatim"
    );
    assert!(harness.run_user_turn("RECENT-TURN").error.is_none());
    let compaction = harness
        .events()
        .iter()
        .find_map(|event| match event {
            HarnessEvent::Compaction {
                tokens_before,
                tokens_after,
                summary,
            } => Some((*tokens_before, *tokens_after, summary.clone())),
            _ => None,
        })
        .expect("compaction event");
    assert!(compaction.0 > compaction.1, "prompt did not shrink");
    assert!(compaction.2.contains("Compacted history:"));
    let request = harness.requests().last().expect("compacted request");
    let text = blob(&request.messages);
    assert!(text.contains("RECENT-TURN"));
    assert!(text.contains("Conversation summary:"));
    assert!(!text.contains(&ancient), "ancient turn stayed verbatim");
    assert!(text.contains("ANCIENT-TURN"));

    let mut replaced = PiHarness::new(
        plain,
        Box::new(ScriptedModel::new(vec![
            ModelStep::text(SERVED, "ack-ancient"),
            ModelStep::text(SERVED, "ack-recent"),
        ])),
        dir.path(),
    );
    replaced.add_plugin(Box::new(SummaryPlugin));
    assert!(replaced.run_user_turn(&ancient).error.is_none());
    assert!(replaced.run_user_turn("RECENT-TURN").error.is_none());
    let summary = replaced
        .events()
        .iter()
        .find_map(|event| match event {
            HarnessEvent::Compaction { summary, .. } => Some(summary.clone()),
            _ => None,
        })
        .expect("plugin compaction");
    assert_eq!(summary, "PLUGIN SUMMARY");
    let text = blob(&replaced.requests().last().expect("request").messages);
    assert!(text.contains("PLUGIN SUMMARY"));
    assert!(text.contains("RECENT-TURN"));
    assert!(!text.contains("Compacted history:"));
}

#[test]
fn stale_read_guard_surfaces_mtime_change_and_reread() {
    let (dir, _guard) = isolated();
    let file = dir.path().join("notes.txt");
    std::fs::write(&file, "version-one").expect("write");
    let path = file.display().to_string();

    let mut changed = scripted(
        "stale-mtime",
        vec![
            read_call(&path),
            ModelStep::text(SERVED, "version-one"),
            ModelStep::text(SERVED, "will re-read"),
        ],
        dir.path(),
    );
    changed.register_tool(
        false,
        Box::new(ReadTool {
            body: "version-one".to_string(),
        }),
    );
    assert!(changed.run_user_turn("look at the file").error.is_none());
    std::fs::write(&file, "version-two").expect("rewrite");
    let handle = std::fs::File::options()
        .write(true)
        .open(&file)
        .expect("open");
    handle
        .set_modified(SystemTime::now() + Duration::from_secs(30))
        .expect("mtime");
    drop(handle);
    assert!(changed.run_user_turn("continue").error.is_none());
    let request = changed.requests().last().expect("request after change");
    let text = blob(&request.messages);
    assert!(text.contains("[stale]"), "stale tool result not marked");
    assert!(
        text.contains(STALE_READ_PREFIX),
        "staleness note missing: {text}"
    );
    assert!(text.contains(&path));

    let mut again = scripted(
        "stale-reread",
        vec![
            read_call(&path),
            ModelStep::text(SERVED, "version-two"),
            ModelStep::text(SERVED, "reading again"),
        ],
        dir.path(),
    );
    again.register_tool(
        false,
        Box::new(ReadTool {
            body: "version-two".to_string(),
        }),
    );
    assert!(again.run_user_turn("look once").error.is_none());
    assert!(again
        .run_user_turn(&format!("read {path} again"))
        .error
        .is_none());
    let text = blob(&again.requests().last().expect("reread").messages);
    assert!(text.contains(STALE_READ_PREFIX));
    assert!(text.contains("[stale]"));
}

#[test]
fn permission_elevation_allow_once_always_and_deny() {
    let (dir, _guard) = isolated();

    let mut once = scripted(
        "allow-once",
        vec![
            ModelStep::calls(SERVED, vec![ToolCall::new("c1", "bash", json!({}))]),
            ModelStep::calls(SERVED, vec![ToolCall::new("c2", "bash", json!({}))]),
            ModelStep::text(SERVED, "done"),
        ],
        dir.path(),
    );
    once.set_permissions(Box::new(ScriptedPermissions::new(vec![
        PermissionChoice::AllowOnce,
        PermissionChoice::AllowOnce,
    ])));
    once.register_tool(
        true,
        Box::new(TextTool {
            name: "bash",
            text: "ran".to_string(),
        }),
    );
    assert!(once.run_user_turn("run twice").error.is_none());
    let asks = once
        .events()
        .iter()
        .filter(|event| matches!(event, HarnessEvent::PermissionAsk { .. }))
        .count();
    assert_eq!(asks, 2);
    let store = crate::broker::GrantStore::load_or_default(dir.path());
    assert!(
        store
            .records()
            .iter()
            .all(|record| record.target_id != "bash"
                || record.duration != crate::broker::GrantDuration::Always),
        "allow-once must not persist an always grant"
    );

    let mut always = scripted(
        "allow-always",
        vec![
            ModelStep::calls(SERVED, vec![ToolCall::new("c1", "bash", json!({}))]),
            ModelStep::text(SERVED, "allowed"),
        ],
        dir.path(),
    );
    always.set_permissions(Box::new(ScriptedPermissions::new(vec![
        PermissionChoice::AllowAlways,
    ])));
    always.register_tool(
        true,
        Box::new(TextTool {
            name: "bash",
            text: "ran".to_string(),
        }),
    );
    let first = always.run_user_turn("run it");
    assert!(first.error.is_none(), "{first:?}");
    assert!(always
        .events()
        .iter()
        .any(|event| matches!(event, HarnessEvent::PermissionAsk { tool } if tool == "bash")));
    let saved = crate::broker::GrantStore::load_or_default(dir.path());
    assert!(
        saved.records().iter().any(|record| {
            record.decision == crate::broker::Decision::Allow
                && record.duration == crate::broker::GrantDuration::Always
                && record.target_id == "bash"
                && record.actor_id == "assistant"
        }),
        "always grant missing after save: {:?}",
        saved.records()
    );

    let mut covered = scripted(
        "allow-covered",
        vec![
            ModelStep::calls(SERVED, vec![ToolCall::new("c1", "bash", json!({}))]),
            ModelStep::text(SERVED, "covered"),
        ],
        dir.path(),
    );
    covered.set_permissions(Box::new(ScriptedPermissions::new(vec![])));
    covered.register_tool(
        true,
        Box::new(TextTool {
            name: "bash",
            text: "ran-without-ask".to_string(),
        }),
    );
    assert!(covered.run_user_turn("run it covered").error.is_none());
    assert!(
        covered
            .events()
            .iter()
            .all(|event| !matches!(event, HarnessEvent::PermissionAsk { .. })),
        "always grant still prompted: {:?}",
        covered.events()
    );
    assert!(blob(&covered.requests().last().expect("covered").messages).contains("ran-without-ask"));

    let mut denied = scripted(
        "deny",
        vec![
            ModelStep::calls(SERVED, vec![ToolCall::new("c1", "write", json!({}))]),
            ModelStep::text(SERVED, "still here"),
        ],
        dir.path(),
    );
    denied.set_permissions(Box::new(ScriptedPermissions::new(vec![
        PermissionChoice::Deny,
    ])));
    denied.register_tool(
        true,
        Box::new(TextTool {
            name: "write",
            text: "should-not-run".to_string(),
        }),
    );
    let report = denied.run_user_turn("do not run");
    assert_eq!(report.text.as_deref(), Some("still here"));
    assert!(report.error.is_none());
    let text = blob(&denied.requests().last().expect("after deny").messages);
    assert!(text.contains("permission_denied"));
    assert!(!text.contains("should-not-run"));
}

#[test]
fn plugin_hooks_run_in_order_and_before_tool_can_veto() {
    let (dir, _guard) = isolated();
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut harness = scripted(
        "hooks",
        vec![
            ModelStep::calls(SERVED, vec![ToolCall::new("c1", "read", json!({}))]),
            ModelStep::text(SERVED, "ok"),
        ],
        dir.path(),
    );
    harness.register_tool(
        false,
        Box::new(TextTool {
            name: "read",
            text: "body".to_string(),
        }),
    );
    harness.add_plugin(Box::new(OrderPlugin {
        label: "alpha",
        log: Arc::clone(&log),
        veto_tool: None,
    }));
    harness.add_plugin(Box::new(OrderPlugin {
        label: "beta",
        log: Arc::clone(&log),
        veto_tool: None,
    }));
    assert!(harness.run_user_turn("go").error.is_none());
    assert_eq!(
        log.lock().expect("log").clone(),
        [
            "alpha:before_turn",
            "beta:before_turn",
            "alpha:before_tool",
            "beta:before_tool",
            "alpha:after_tool",
            "beta:after_tool",
            "alpha:on_turn_end",
            "beta:on_turn_end",
        ]
        .map(str::to_string)
        .to_vec()
    );

    let mut vetoed = scripted(
        "veto",
        vec![
            ModelStep::calls(SERVED, vec![ToolCall::new("c1", "bash", json!({}))]),
            ModelStep::text(SERVED, "saw the veto"),
        ],
        dir.path(),
    );
    vetoed.register_tool(false, Box::new(PanicTool));
    vetoed.add_plugin(Box::new(OrderPlugin {
        label: "guard",
        log: Arc::new(Mutex::new(Vec::new())),
        veto_tool: Some("bash"),
    }));
    let report = vetoed.run_user_turn("run bash");
    assert_eq!(report.text.as_deref(), Some("saw the veto"));
    assert!(vetoed.events().iter().any(
        |event| matches!(event, HarnessEvent::Veto { reason, .. } if reason == "bash is disabled")
    ));
    let text = blob(&vetoed.requests()[1].messages);
    assert!(text.contains("bash is disabled"));
}

#[test]
fn open_pane_context_answers_without_grep_or_list() {
    let (dir, _guard) = isolated();
    let path = dir.path().join("temp.md");
    std::fs::write(&path, "pane-excerpt-hello").expect("write");
    let path_text = path.display().to_string();
    let mut harness = PiHarness::new(
        config("panes"),
        Box::new(PaneModel {
            path: path_text.clone(),
            answered: false,
        }),
        dir.path(),
    );
    harness.register_tool(
        false,
        Box::new(ReadTool {
            body: "pane-excerpt-hello".to_string(),
        }),
    );
    harness.add_plugin(Box::new(OpenPanePlugin::new(vec![OpenPane {
        app_type: "editor".to_string(),
        title: "temp.md".to_string(),
        path: Some(path_text.clone()),
        focused: true,
        excerpt: "pane-excerpt-hello".to_string(),
    }])));
    let report = harness.run_user_turn("what does the open file say?");
    assert_eq!(report.text.as_deref(), Some("from pane-excerpt-hello"));
    let call = harness
        .requests()
        .iter()
        .flat_map(|request| request.messages.iter())
        .find_map(|message| message.tool_calls.first())
        .expect("model chose a tool");
    assert_eq!(call.name, "read");
    assert_eq!(call.arguments["path"], path_text);
    assert!(harness.requests()[0].system.contains("focused=yes"));
}

#[test]
fn loop_safety_max_steps_tool_error_and_abort() {
    let (dir, _guard) = isolated();

    let hits = Arc::new(AtomicUsize::new(0));
    let mut capped = config("max-steps");
    capped.max_steps = 1;
    let mut harness = PiHarness::new(
        capped,
        Box::new(ScriptedModel::new(vec![read_call("x")])),
        dir.path(),
    );
    harness.register_tool(
        false,
        Box::new(CountingTool {
            hits: Arc::clone(&hits),
        }),
    );
    let report = harness.run_user_turn("loop");
    assert!(report.hit_max_steps);
    assert_eq!(harness.requests().len(), 1);
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    assert!(harness.session().entries.iter().any(|entry| matches!(
        entry,
        SessionEntry::Tool { text, .. } if text == "max_steps_exceeded"
    )));
    assert_pairs(&llm_pairs(harness.session()));

    let mut failing = scripted(
        "tool-error",
        vec![read_call("x"), ModelStep::text(SERVED, "recovered")],
        dir.path(),
    );
    failing.register_tool(false, Box::new(FailingTool));
    let report = failing.run_user_turn("read it");
    assert_eq!(report.text.as_deref(), Some("recovered"));
    assert!(!report.hit_max_steps);
    assert!(failing.events().iter().any(
        |event| matches!(event, HarnessEvent::ToolError { message, .. } if message == "disk failed")
    ));
    assert!(blob(&failing.requests()[1].messages).contains("error: disk failed"));

    let abort = AbortHandle::new();
    let mut aborted = PiHarness::new(
        config("abort"),
        Box::new(AbortOnCall {
            abort: abort.clone(),
        }),
        dir.path(),
    );
    aborted.set_abort(abort);
    aborted.register_tool(
        false,
        Box::new(CountingTool {
            hits: Arc::new(AtomicUsize::new(0)),
        }),
    );
    let report = aborted.run_user_turn("stop");
    assert!(report.aborted);
    assert_eq!(aborted.requests().len(), 1);
    assert!(aborted.session().entries.iter().any(|entry| matches!(
        entry,
        SessionEntry::Tool { text, .. } if text == "aborted"
    )));
    assert_pairs(&llm_pairs(aborted.session()));

    let steering = SteeringHandle::new();
    let mut steered = scripted(
        "steer",
        vec![read_call("x"), ModelStep::text(SERVED, "short")],
        dir.path(),
    );
    steered.set_steering(steering.clone());
    steered.register_tool(false, Box::new(SteeringTool { steering }));
    assert!(steered.run_user_turn("answer").error.is_none());
    assert!(blob(&steered.requests()[1].messages).contains("use the short answer"));
}

fn llm_pairs(session: &PiSession) -> Vec<LlmMessage> {
    session
        .entries
        .iter()
        .filter_map(|entry| match entry {
            SessionEntry::Assistant {
                text, tool_calls, ..
            } => Some(LlmMessage {
                role: "assistant".to_string(),
                content: text.clone(),
                tool_calls: tool_calls.clone(),
                tool_call_id: None,
            }),
            SessionEntry::Tool { text, call_id, .. } => Some(LlmMessage {
                role: "tool".to_string(),
                content: text.clone(),
                tool_calls: Vec::new(),
                tool_call_id: Some(call_id.clone()),
            }),
            SessionEntry::User { text, .. } => Some(LlmMessage {
                role: "user".to_string(),
                content: text.clone(),
                tool_calls: Vec::new(),
                tool_call_id: None,
            }),
            _ => None,
        })
        .collect()
}

#[test]
fn session_resume_matches_uninterrupted_request() {
    let (dir, _guard) = isolated();
    let assistant_dir = dir.path().join("assistant");
    let steps = || {
        vec![
            read_call("temp.md"),
            ModelStep::text(SERVED, "saw alpha-bravo-temp-contents"),
            ModelStep::text(SERVED, "continued"),
        ]
    };
    let mut live = scripted("resume", steps(), dir.path());
    live.set_assistant_dir(assistant_dir.clone());
    live.register_tool(
        false,
        Box::new(ReadTool {
            body: "alpha-bravo-temp-contents".to_string(),
        }),
    );
    assert!(live.run_user_turn("read temp.md").error.is_none());
    let loaded = PiSession::load(&assistant_dir, "resume")
        .expect("load")
        .expect("session file");
    assert!(live.run_user_turn("continue").error.is_none());
    let expected = live.requests().last().expect("live next").clone();
    let user_line = std::fs::read_to_string(PiSession::path(&assistant_dir, "resume"))
        .expect("read session")
        .lines()
        .find(|line| line.contains("\"role\":\"user\""))
        .expect("user line")
        .to_string();
    let turn: crate::assistant::model::Turn =
        serde_json::from_str(&user_line).expect("user line is a Turn");
    assert_eq!(turn.role, crate::assistant::model::TurnRole::User);
    assert!(turn.text.contains("read temp.md"));

    let mut resumed = PiHarness::from_session(
        config("resume"),
        loaded,
        Box::new(ScriptedModel::new(vec![ModelStep::text(
            SERVED,
            "continued",
        )])),
        dir.path(),
    );
    resumed.register_tool(
        false,
        Box::new(ReadTool {
            body: "alpha-bravo-temp-contents".to_string(),
        }),
    );
    assert!(resumed.run_user_turn("continue").error.is_none());
    let actual = resumed.requests().last().expect("resumed");
    assert_eq!(actual.messages, expected.messages);
    assert_eq!(actual.system, expected.system);
}

#[test]
fn model_visibility_records_served_model_in_the_ledger() {
    let (dir, _guard) = isolated();
    let mut cfg = config("visibility");
    cfg.configured_model = CONFIGURED_ALIAS.to_string();
    let mut harness = PiHarness::new(
        cfg,
        Box::new(ScriptedModel::new(vec![ModelStep::text(SERVED, "hello")])),
        dir.path(),
    );
    let report = harness.run_user_turn("hi");
    let visibility = report.visibility.expect("visibility");
    assert_eq!(visibility.configured_model, CONFIGURED_ALIAS);
    assert_eq!(visibility.served_model, SERVED);
    assert_ne!(visibility.configured_model, visibility.served_model);
    assert_eq!(visibility.picker_label, "Medium — xiaomi/mimo-v2.5");
    assert_eq!(
        picker_label(crate::protocol::ModelTier::Medium, SERVED),
        visibility.picker_label
    );
    let ledger = std::fs::read_to_string(crate::plexi_ai::ledger::ledger_path()).expect("ledger");
    assert!(
        ledger.contains(SERVED),
        "served model missing from ledger: {ledger}"
    );
    assert!(ledger.contains("\"app_id\":\"assistant\""));
    let row: serde_json::Value =
        serde_json::from_str(ledger.lines().last().expect("row")).expect("row");
    assert_eq!(row["model"], SERVED);
}

#[test]
fn provider_agnostic_complete_once_for_openrouter_ollama_and_local() {
    let (dir, _guard) = isolated();
    let broker = Arc::new(RecordingBroker {
        dispatches: AtomicUsize::new(0),
        providers: Mutex::new(Vec::new()),
    });
    for provider in ["openrouter", "ollama", "local"] {
        let mut cfg = config(provider);
        cfg.provider = provider.to_string();
        cfg.configured_model = format!("{provider}-model");
        let mut harness = PiHarness::new(
            cfg,
            Box::new(BrokerStepper::new(Arc::clone(&broker) as Arc<dyn AiBroker>)),
            dir.path(),
        );
        let report = harness.run_user_turn("ping");
        assert_eq!(report.text.as_deref(), Some("provider-ok"));
        assert_eq!(
            report.visibility.expect("visibility").served_model,
            format!("{provider}-model")
        );
    }
    assert_eq!(broker.dispatches.load(Ordering::SeqCst), 0);
    assert_eq!(
        broker.providers.lock().expect("providers").as_slice(),
        ["openrouter", "ollama", "local"]
    );
}

#[test]
fn harness_flag_selects_pi_and_unknown_stays_current() {
    assert_eq!(harness_kind_from_config(None), HarnessKind::Current);
    assert_eq!(harness_kind_from_config(Some("")), HarnessKind::Current);
    assert_eq!(
        harness_kind_from_config(Some("current")),
        HarnessKind::Current
    );
    assert_eq!(harness_kind_from_config(Some("pi")), HarnessKind::Pi);
    assert_eq!(
        harness_kind_from_config(Some("other")),
        HarnessKind::Current
    );

    let mut base = crate::config::AiConfig::default();
    let mut overlay = crate::config::AiConfig::default();
    overlay.harness = Some("pi".to_string());
    overlay.backend = Some("ollama".to_string());
    overlay.ollama = Some(crate::config::OllamaBackendConfig {
        tiers: crate::config::ModelTiers {
            model_medium: Some("llama3.2".to_string()),
            ..Default::default()
        },
        ..Default::default()
    });
    base.overlay(overlay);
    assert_eq!(base.harness.as_deref(), Some("pi"));
    let resolved = PiConfig::from_ai(&base, crate::protocol::ModelTier::Medium, "cfg");
    assert_eq!(resolved.provider, "ollama");
    assert_eq!(resolved.configured_model, "llama3.2");
}

#[test]
#[ignore = "live eval: PLEXI_LIVE_EVALS=1 and OPENROUTER_API_KEY"]
fn live_temp_md_replay_is_opt_in() {
    if std::env::var("PLEXI_LIVE_EVALS").ok().as_deref() != Some("1")
        || std::env::var("OPENROUTER_API_KEY").is_err()
    {
        return;
    }
    let (dir, _guard) = isolated();
    let file = dir.path().join("temp.md");
    std::fs::write(&file, "alpha-bravo-temp-contents").expect("write");
    let mut ai = crate::config::AiConfig::default();
    ai.backend = Some("openrouter".to_string());
    ai.openrouter = Some(crate::config::OpenRouterBackendConfig {
        tiers: crate::config::ModelTiers {
            model_medium: Some(SERVED.to_string()),
            ..Default::default()
        },
        ..Default::default()
    });
    let broker = Arc::new(crate::plexi_ai::broker::LiveAiBroker::new(Some(ai.clone())));
    let mut cfg = PiConfig::from_ai(&ai, crate::protocol::ModelTier::Medium, "live");
    cfg.system =
        "When asked about a file, call the read tool and answer only from its result.".to_string();
    cfg.max_steps = 4;
    let mut harness = PiHarness::new(cfg, Box::new(BrokerStepper::new(broker)), dir.path());
    harness.register_tool(
        false,
        Box::new(ReadTool {
            body: "alpha-bravo-temp-contents".to_string(),
        }),
    );
    let report = harness.run_user_turn(&format!("Read {} and quote it.", file.display()));
    assert!(report.error.is_none(), "{report:?}");
    if harness.requests().len() > 1 {
        let next = harness.requests().last().expect("follow-up");
        assert_pairs(&next.messages);
        assert!(blob(&next.messages).contains("alpha-bravo-temp-contents"));
    }
}
