//! Stint 1450: a Python plugin's `AiQuery` takes the `ai.query` consent gate
//! (stint 1489: asked in the pane, or answered from the broker's grant store)
//! and, once admitted, reaches the shared broker path with its config, budget
//! and ledger gates. Every test runs a real CPython guest in a real host; none
//! calls a provider. The only stand-ins are Plexi's own `AiBroker` and
//! `AiBackend` traits.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};

use serde_json::json;

use super::consent_tests::{
    launch, monitor, press_on_prompt, prompt, python, title, wait_for_prompt, wait_for_title,
};
use crate::config::{AiConfig, ResolvedModelMode, ResolvedModelModes};
use crate::plexi_ai::backend::{AiBackend, AiBackendError, AiBackendRequest, StreamEvent};
use crate::plexi_ai::broker::{
    AiBroker, AiBrokerRequest, AiBrokerResponse, LiveAiBroker, ModelMode,
};
use crate::spatial::tiling::PaneId;
use crate::testing::HostHarness;

const PLUGIN: &str = "ai-probe";
const FAST_MODEL: &str = "test/fast-model";
const SLOW_MODEL: &str = "test/slow-model";

/// A plugin that sends `AiQuery("q1", <mode>, ...)` when the host delivers
/// the UI action `ask:<mode>`, and shows the `AiResponse` in its title.
fn write_ai_plugin(dir: &std::path::Path, declares_ai_query: bool) {
    let capabilities = if declares_ai_query {
        "\"ai.query\""
    } else {
        ""
    };
    std::fs::write(
        dir.join("manifest.toml"),
        format!(
            "schema_version = 2\n\n[plugin]\nid = \"{PLUGIN}\"\nname = \"AI probe\"\nversion = \"0.1.0\"\n\n\
             [plugin.capabilities]\ncapabilities = [{capabilities}]\n\n\
             [runtime]\nkind = \"python\"\nentry = \"main.py\"\n\n\
             [[contributes.views]]\nid = \"{PLUGIN}.main\"\ndefault = true\n"
        ),
    )
    .expect("write manifest");
    std::fs::write(
        dir.join("main.py"),
        "from plexi_sdk.effects import AiMessage, AiQuery, SetTitle\n\
         from plexi_sdk.events import AiResponse, UiAction\n\
         from plexi_sdk.ui import Column\n\n\
         def init(size, args):\n\
         \x20   return [SetTitle(\"ready\")]\n\n\
         def update(event):\n\
         \x20   if isinstance(event, UiAction) and event.handler_id.startswith(\"ask:\"):\n\
         \x20       mode = event.handler_id[4:]\n\
         \x20       return [AiQuery(\"q1\", mode, \"be brief\", [AiMessage(\"user\", \"hi\")])]\n\
         \x20   if isinstance(event, AiResponse):\n\
         \x20       return [SetTitle(f\"ai|{event.request_id}|{event.content}|{event.error}\")]\n\
         \x20   return []\n\n\
         def view():\n\
         \x20   return Column([])\n",
    )
    .expect("write main.py");
}

/// A host with the probe running and `broker` as its AI broker.
struct Probe {
    h: HostHarness,
    pane: PaneId,
    _dir: tempfile::TempDir,
}

impl Probe {
    fn launch(declares_ai_query: bool, broker: Arc<dyn AiBroker>) -> Self {
        let dir = tempfile::tempdir().expect("plugin dir");
        write_ai_plugin(dir.path(), declares_ai_query);
        let mut h = HostHarness::new();
        let pane = launch(&mut h, dir.path());
        wait_for_title(&mut h, pane, "ready");
        python(&mut h, pane).set_ai_broker(broker);
        Self { h, pane, _dir: dir }
    }

    /// "Always allow" for this plugin's `ai.query`, as the prompt stores it.
    fn grant_always(&mut self) {
        let workspace = python(&mut self.h, self.pane).config.workspace_root.clone();
        monitor()
            .remember_plugin_capability(PLUGIN, &workspace, "ai.query", None)
            .expect("store the ai.query grant");
    }

    /// The plugin sends its `AiQuery` in `mode`.
    fn ask(&mut self, mode: &str) {
        python(&mut self.h, self.pane)
            .send_to_runtime(&json!({"type": "ui_action", "handler_id": format!("ask:{mode}")}));
    }

    /// Pump until the `AiResponse` reaches the guest; `(content, error)`.
    fn reply(&mut self) -> (String, String) {
        let shown = wait_for_title(&mut self.h, self.pane, "ai|");
        let parts: Vec<&str> = shown
            .strip_prefix("ai|")
            .expect("ai| title")
            .splitn(3, '|')
            .collect();
        assert_eq!(parts[0], "q1", "the response names the request");
        (parts[1].to_string(), parts[2].to_string())
    }

    fn ask_and_reply(&mut self, mode: &str) -> (String, String) {
        self.ask(mode);
        let reply = self.reply();
        python(&mut self.h, self.pane).title = Some("ready".to_string());
        reply
    }
}

/// Records every request; answers with a fixed reply.
#[derive(Default)]
struct RecordingBroker {
    seen: Mutex<Vec<(String, ModelMode)>>,
}

impl AiBroker for RecordingBroker {
    fn dispatch(
        &self,
        request: AiBrokerRequest,
        _on_delta: &mut dyn FnMut(crate::plexi_ai::turn_loop::TurnDelta<'_>),
    ) -> AiBrokerResponse {
        self.seen
            .lock()
            .unwrap()
            .push((request.plugin_id, request.model_mode));
        AiBrokerResponse::ok("recorded".to_string(), 1, 1)
    }
}

/// Stand-in for the provider behind the live broker: counts calls and
/// streams Plexi's own `StreamEvent`s with a reported cost, so the ledger
/// row is complete without a price lookup.
#[derive(Default)]
struct CountingBackend {
    calls: AtomicUsize,
}

impl AiBackend for CountingBackend {
    fn name(&self) -> &str {
        "openrouter"
    }

    fn stream_to_channel(
        &self,
        _request: AiBackendRequest,
        tx: mpsc::Sender<StreamEvent>,
    ) -> Result<(), AiBackendError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let _ = tx.send(StreamEvent::Text("summary".to_string()));
        let _ = tx.send(StreamEvent::Done {
            input_tokens: Some(11),
            output_tokens: Some(4),
            cost_usd: Some(0.001),
            generation_id: None,
        });
        Ok(())
    }
}

fn modes() -> ResolvedModelModes {
    let mode = |model: &str| ResolvedModelMode {
        provider: "openrouter".to_string(),
        model: model.to_string(),
        thinking: "off".to_string(),
    };
    ResolvedModelModes {
        fast: mode(FAST_MODEL),
        slow: mode(SLOW_MODEL),
    }
}

fn live_broker(ai: Option<AiConfig>) -> (Arc<CountingBackend>, Arc<dyn AiBroker>) {
    let backend = Arc::new(CountingBackend::default());
    let broker = LiveAiBroker::with_test_backend(ai, modes(), backend.clone());
    (backend, Arc::new(broker))
}

fn ledger_rows() -> Vec<serde_json::Value> {
    std::fs::read_to_string(crate::plexi_ai::ledger::ledger_path())
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).expect("ledger row is JSON"))
        .collect()
}

/// No grant: the query asks in the pane and reaches nothing. "Deny once"
/// answers it with an `AiResponse.error` naming `ai.query`; the broker is
/// never called.
#[test]
fn python_ai_query_without_a_grant_asks_and_deny_refuses_before_the_broker() {
    let broker = Arc::new(RecordingBroker::default());
    let mut probe = Probe::launch(true, broker.clone());
    probe.ask("fast");
    wait_for_prompt(&mut probe.h, probe.pane, "ai.query");
    probe.h.run_frames(5);
    assert_eq!(
        title(&mut probe.h, probe.pane),
        "ready",
        "nothing answered yet"
    );
    assert!(
        broker.seen.lock().unwrap().is_empty(),
        "no broker call while asking"
    );

    press_on_prompt(&mut probe.h, probe.pane, egui::Key::Escape, false);
    let (content, error) = probe.reply();
    assert_eq!(content, "None");
    assert!(
        error.contains("ai.query") && error.contains("denied"),
        "the refusal names the capability: {error}"
    );
    assert!(broker.seen.lock().unwrap().is_empty(), "no broker call");
}

/// No grant: "Grant once" sends exactly the request it was shown for to the
/// broker, in the mode the plugin chose.
#[test]
fn python_ai_query_grant_once_reaches_the_broker() {
    let broker = Arc::new(RecordingBroker::default());
    let mut probe = Probe::launch(true, broker.clone());
    probe.ask("slow");
    wait_for_prompt(&mut probe.h, probe.pane, "ai.query");
    press_on_prompt(&mut probe.h, probe.pane, egui::Key::Enter, false);
    assert_eq!(probe.reply(), ("recorded".to_string(), "None".to_string()));
    assert_eq!(
        *broker.seen.lock().unwrap(),
        vec![(PLUGIN.to_string(), ModelMode::Slow)]
    );
}

/// A manifest without `ai.query` is refused without asking: only the author
/// can fix it.
#[test]
fn python_ai_query_undeclared_is_refused_without_asking() {
    let broker = Arc::new(RecordingBroker::default());
    let mut probe = Probe::launch(false, broker.clone());
    probe.ask("fast");
    let (_, error) = probe.reply();
    assert_eq!(error, "missing capability ai.query");
    assert!(
        prompt(&mut probe.h, probe.pane).is_none(),
        "nobody is asked"
    );
    assert!(broker.seen.lock().unwrap().is_empty(), "no broker call");
}

/// An unknown mode is refused before anyone is asked, naming the accepted
/// values.
#[test]
fn python_ai_query_unknown_mode_is_refused_naming_accepted_values() {
    let broker = Arc::new(RecordingBroker::default());
    let mut probe = Probe::launch(true, broker.clone());
    probe.ask("medium");
    let (_, error) = probe.reply();
    assert!(
        error.contains("\"medium\"") && error.contains("accepted values: fast, slow"),
        "{error}"
    );
    assert!(
        prompt(&mut probe.h, probe.pane).is_none(),
        "nobody is asked"
    );
    assert!(broker.seen.lock().unwrap().is_empty(), "no broker call");
}

/// Granted, with no `[ai]` config: the broker's `ai_config_missing` error.
#[test]
fn python_ai_query_without_ai_config_returns_ai_config_missing() {
    let (backend, broker) = live_broker(None);
    let mut probe = Probe::launch(true, broker);
    probe.grant_always();
    let (content, error) = probe.ask_and_reply("fast");
    assert_eq!(content, "None");
    assert!(error.starts_with("ai_config_missing"), "{error}");
    assert_eq!(backend.calls.load(Ordering::SeqCst), 0, "no provider call");
}

/// Granted, over the per-plugin daily cap: refused naming the cap, with no
/// provider call and no ledger row.
#[test]
fn python_ai_query_over_the_per_plugin_cap_is_refused_without_a_provider_call() {
    let (backend, broker) = live_broker(Some(AiConfig {
        per_plugin_daily_usd: Some(0.0),
        ..AiConfig::default()
    }));
    let mut probe = Probe::launch(true, broker);
    probe.grant_always();
    let (content, error) = probe.ask_and_reply("fast");
    assert_eq!(content, "None");
    assert!(
        error.contains("budget_exceeded")
            && error.contains("per-plugin daily AI budget exceeded")
            && error.contains(PLUGIN),
        "the refusal names the cap: {error}"
    );
    assert_eq!(backend.calls.load(Ordering::SeqCst), 0, "no provider call");
    assert!(
        ledger_rows().is_empty(),
        "a refused call writes no ledger row"
    );
}

/// Granted: the reply reaches the plugin, `fast` and `slow` resolve to their
/// configured models, and each call's ledger row names the plugin.
#[test]
fn python_ai_query_reply_reaches_the_plugin_and_the_ledger_names_it_per_mode() {
    let (backend, broker) = live_broker(Some(AiConfig::default()));
    let mut probe = Probe::launch(true, broker);
    probe.grant_always();
    for (mode, model) in [("fast", FAST_MODEL), ("slow", SLOW_MODEL)] {
        let (content, error) = probe.ask_and_reply(mode);
        assert_eq!(content, "summary", "{mode}: reply delivered");
        assert_eq!(error, "None", "{mode}");
        let rows = ledger_rows();
        let row = rows.last().expect("a ledger row per call");
        assert_eq!(row["plugin_id"], PLUGIN, "{mode}: attributed to the plugin");
        assert_eq!(
            row["model"], model,
            "{mode} resolves to its configured model"
        );
    }
    assert_eq!(backend.calls.load(Ordering::SeqCst), 2);
    assert_eq!(ledger_rows().len(), 2);
    assert!(
        prompt(&mut probe.h, probe.pane).is_none(),
        "a stored grant answers without asking"
    );
}
