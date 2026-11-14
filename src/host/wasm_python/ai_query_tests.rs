//! Stint 1450: a Python plugin's `AiQuery` reaches the shared broker path
//! under the `ai.query` grant, with the broker's config, budget and ledger
//! gates. Every test runs a real CPython guest; none calls a real provider.
//!
//! The grant is the launch-time capability set (`PythonLaunchConfig::
//! capabilities`), set here by the harness. Python has no runtime consent
//! path yet; that is stint 1238 step 2.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use super::{LivePythonPane, PythonLaunchConfig};
use crate::config::{AiConfig, ResolvedModelMode, ResolvedModelModes};
use crate::plexi_ai::backend::{AiBackend, AiBackendError, AiBackendRequest, StreamEvent};
use crate::plexi_ai::broker::{
    AiBroker, AiBrokerRequest, AiBrokerResponse, LiveAiBroker, ModelMode,
};

const PLUGIN: &str = "ai-probe";
const FAST_MODEL: &str = "test/fast-model";
const SLOW_MODEL: &str = "test/slow-model";

/// A plugin that sends one `AiQuery` in the mode named by its first launch
/// argument and shows the `AiResponse` in its title.
fn write_ai_plugin(dir: &std::path::Path) {
    std::fs::write(
        dir.join("manifest.toml"),
        format!(
            "schema_version = 2\n\n[plugin]\nid = \"{PLUGIN}\"\nname = \"AI probe\"\nversion = \"0.1.0\"\n\n\
             [plugin.capabilities]\ncapabilities = [\"ai.query\"]\n\n\
             [runtime]\nkind = \"python\"\nentry = \"main.py\"\n\n\
             [[contributes.views]]\nid = \"{PLUGIN}.main\"\ndefault = true\n"
        ),
    )
    .expect("write manifest");
    std::fs::write(
        dir.join("main.py"),
        "from plexi_sdk.effects import AiMessage, AiQuery, SetTitle\n\
         from plexi_sdk.events import AiResponse\n\
         from plexi_sdk.ui import Column\n\n\
         def init(size, args):\n\
         \x20   return [SetTitle(\"asked\"), AiQuery(\"q1\", args[0], \"be brief\", [AiMessage(\"user\", \"hi\")])]\n\n\
         def update(event):\n\
         \x20   if isinstance(event, AiResponse):\n\
         \x20       return [SetTitle(f\"ai|{event.request_id}|{event.content}|{event.error}\")]\n\
         \x20   return []\n\n\
         def view():\n\
         \x20   return Column([])\n",
    )
    .expect("write main.py");
}

/// Launch the probe with `granted` as its launch-time capability set and
/// `broker` as its AI broker, then pump it until the `AiResponse` arrives.
/// Returns the `(content, error)` the guest saw.
fn ask(mode: &str, granted: bool, broker: Arc<dyn AiBroker>) -> (String, String) {
    let dir = tempfile::tempdir().expect("plugin dir");
    write_ai_plugin(dir.path());
    let mut config: PythonLaunchConfig = PythonLaunchConfig::from_manifest_file(dir.path())
        .expect("manifest")
        .expect("python plugin");
    config.launch_args = vec![mode.to_string()];
    config.capabilities = if granted {
        vec!["ai.query".to_string()]
    } else {
        Vec::new()
    };
    let mut pane = LivePythonPane::launch(config).expect("launch probe");
    pane.set_ai_broker(broker);
    let start = std::time::Instant::now();
    loop {
        pane.background_tick();
        if let Some(title) = pane.title.as_deref() {
            if let Some(rest) = title.strip_prefix("ai|") {
                let parts: Vec<&str> = rest.splitn(3, '|').collect();
                assert_eq!(parts[0], "q1", "the response names the request");
                return (parts[1].to_string(), parts[2].to_string());
            }
        }
        assert!(
            start.elapsed()
                < crate::testing::load_aware_timeout(std::time::Duration::from_secs(30)),
            "no AiResponse reached the guest (title {:?}, error {:?})",
            pane.title,
            pane.error
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
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

/// Provider stand-in for the live broker: counts calls, replies `summary`
/// with a reported cost so the ledger row is complete without a lookup.
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

/// Red without the grant check: the query would reach the broker.
#[test]
fn python_ai_query_without_the_grant_is_refused_before_the_broker() {
    let broker = Arc::new(RecordingBroker::default());
    let (content, error) = ask("fast", false, broker.clone());
    assert_eq!(content, "None");
    assert!(
        error.contains("ai.query") && error.contains("[plugin.capabilities]"),
        "the refusal names the capability and how it is granted: {error}"
    );
    assert!(broker.seen.lock().unwrap().is_empty(), "no broker call");
}

#[test]
fn python_ai_query_unknown_mode_is_refused_naming_accepted_values() {
    let broker = Arc::new(RecordingBroker::default());
    let (_, error) = ask("medium", true, broker.clone());
    assert!(
        error.contains("\"medium\"") && error.contains("accepted values: fast, slow"),
        "{error}"
    );
    assert!(broker.seen.lock().unwrap().is_empty(), "no broker call");
}

#[test]
fn python_ai_query_without_ai_config_returns_ai_config_missing() {
    let _profile = crate::config::set_test_profile_dir(tempfile::tempdir().unwrap().keep());
    let (backend, broker) = live_broker(None);
    let (content, error) = ask("fast", true, broker);
    assert_eq!(content, "None");
    assert!(error.starts_with("ai_config_missing"), "{error}");
    assert_eq!(backend.calls.load(Ordering::SeqCst), 0, "no provider call");
}

#[test]
fn python_ai_query_over_the_per_plugin_cap_is_refused_without_a_provider_call() {
    let _profile = crate::config::set_test_profile_dir(tempfile::tempdir().unwrap().keep());
    let (backend, broker) = live_broker(Some(AiConfig {
        per_plugin_daily_usd: Some(0.0),
        ..AiConfig::default()
    }));
    let (content, error) = ask("fast", true, broker);
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

/// Red without the handler: the guest never gets an `AiResponse`.
#[test]
fn python_ai_query_reply_reaches_the_plugin_and_the_ledger_names_it_per_mode() {
    let _profile = crate::config::set_test_profile_dir(tempfile::tempdir().unwrap().keep());
    let (backend, broker) = live_broker(Some(AiConfig::default()));
    for (mode, model) in [("fast", FAST_MODEL), ("slow", SLOW_MODEL)] {
        let (content, error) = ask(mode, true, broker.clone());
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
}

#[test]
fn python_ai_query_mode_reaches_the_broker_as_fast_or_slow() {
    let broker = Arc::new(RecordingBroker::default());
    assert_eq!(ask("fast", true, broker.clone()).0, "recorded");
    assert_eq!(ask("slow", true, broker.clone()).0, "recorded");
    assert_eq!(
        *broker.seen.lock().unwrap(),
        vec![
            (PLUGIN.to_string(), ModelMode::Fast),
            (PLUGIN.to_string(), ModelMode::Slow)
        ]
    );
}
