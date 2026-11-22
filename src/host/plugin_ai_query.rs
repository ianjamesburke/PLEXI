//! Plugin `ai.query` dispatch shared by the WASM and CPython plugin panes.
//!
//! Each pane runtime decides admission (`ai.query` grant) and parses its own
//! wire shape; this module owns the part that must be identical for both:
//! building the broker request, running it off the UI thread, and the one
//! `info` trace per call. Config, budget and ledger are applied inside
//! [`AiBroker::dispatch`], so every runtime gets them by construction.
//!
//! Nothing a plugin supplies reaches a thread name or a trace: the worker is
//! named and the trace keyed by a host-issued [`CallId`], and a failure is
//! traced as a bounded category, never as its message (which can carry the
//! plugin's prompt or the provider's raw error body).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::plexi_ai::broker::{AiBroker, AiBrokerRequest, AiBrokerResponse, ModelMode};
use crate::plexi_ai::turn_loop::TurnDelta;
use crate::protocol::AiMessage;

/// Host-issued id of one plugin AI call: names its worker thread and keys
/// every trace line about it. Unique per host process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CallId(u64);

impl CallId {
    pub(crate) fn next() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

impl std::fmt::Display for CallId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Why the host answered a call itself, without a broker call.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Refusal {
    /// Malformed request: unknown mode, bad messages.
    InvalidRequest,
    /// The `ai.query` consent gate refused it.
    Consent,
    /// The worker thread could not be started.
    WorkerSpawn,
}

impl Refusal {
    fn category(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::Consent => "consent_refused",
            Self::WorkerSpawn => "worker_spawn_failed",
        }
    }
}

/// One admitted plugin query, already parsed from the runtime's wire shape.
pub(crate) struct PluginAiQuery {
    pub plugin_id: String,
    pub request_id: String,
    pub mode: ModelMode,
    pub system: String,
    pub messages: Vec<AiMessage>,
}

/// Streamed text/reasoning increments, forwarded on the worker thread.
pub(crate) type OnDelta = Box<dyn for<'a> FnMut(TurnDelta<'a>) + Send>;
/// Receives the final broker response on the worker thread.
pub(crate) type Deliver = Box<dyn FnOnce(AiBrokerResponse) + Send>;

/// Run `query` through `broker` on a dedicated worker thread named for
/// `call`. `deliver` is called exactly once with the response. A spawn
/// failure is returned to the caller, which owns reporting it to the plugin;
/// `deliver` is not called.
pub(crate) fn spawn(
    broker: Arc<dyn AiBroker>,
    call: CallId,
    query: PluginAiQuery,
    mut on_delta: OnDelta,
    deliver: Deliver,
) -> std::io::Result<()> {
    let PluginAiQuery {
        plugin_id,
        // The plugin's id for its own reply; the caller's `deliver` carries it.
        request_id: _,
        mode,
        system,
        messages,
    } = query;
    // Test profiles are per-thread; the broker's budget and ledger must see
    // the caller's isolated profile, not the shared fallback.
    #[cfg(test)]
    let profile = crate::config::test_profile_dir_override();
    std::thread::Builder::new()
        .name(format!("plugin-ai-query-{call}"))
        .spawn(move || {
            #[cfg(test)]
            let _profile = profile.map(crate::config::set_test_profile_dir);
            let response = broker.dispatch(
                AiBrokerRequest {
                    plugin_id: plugin_id.clone(),
                    model_mode: mode,
                    concrete_model: None,
                    reasoning_effort: None,
                    system,
                    messages,
                    tools: Vec::new(),
                    workspace_root: None,
                    open_panes: crate::plexi_ai::broker::get_pane_snapshot(),
                    tool_dispatcher: None,
                    cancel: crate::plexi_ai::CancelToken::new(),
                    max_tool_iterations: None,
                    client: None,
                    kind: None,
                },
                &mut *on_delta,
            );
            trace_outcome(&plugin_id, call, Some(mode), &response);
            deliver(response);
        })
        .map(drop)
}

/// The one `info` line for a brokered call. `mode` is `None` when the
/// plugin's value did not parse.
pub(crate) fn trace_outcome(
    plugin_id: &str,
    call: CallId,
    mode: Option<ModelMode>,
    response: &AiBrokerResponse,
) {
    let mode = mode.map_or("invalid", ModelMode::as_str);
    match &response.error {
        None => log::info!(
            "ai.query: plugin={plugin_id} call={call} mode={mode} outcome=ok tokens_in={} tokens_out={}",
            response.tokens_in,
            response.tokens_out
        ),
        Some(error) => log::info!(
            "ai.query: plugin={plugin_id} call={call} mode={mode} outcome=error category={}",
            broker_error_category(error)
        ),
    }
}

/// The one `info` line for a call the host answered itself.
pub(crate) fn trace_refusal(
    plugin_id: &str,
    call: CallId,
    mode: Option<ModelMode>,
    refusal: Refusal,
) {
    let mode = mode.map_or("invalid", ModelMode::as_str);
    log::info!(
        "ai.query: plugin={plugin_id} call={call} mode={mode} outcome=refused category={}",
        refusal.category()
    );
}

/// The bounded category of a broker error. Broker errors lead with a
/// machine code (`code: detail`) or a fixed phrase; the detail after it is
/// never traced.
fn broker_error_category(error: &str) -> &'static str {
    const CODES: &[(&str, &str)] = &[
        ("ai_config_missing", "ai_config_missing"),
        ("budget_exceeded", "budget_exceeded"),
        ("ledger_unavailable", "ledger_unavailable"),
        ("api_key_missing", "api_key_missing"),
        ("unsupported_model_provider", "unsupported_model_provider"),
        ("backend error", "backend_error"),
        ("stream error", "stream_error"),
        ("stream closed", "stream_closed"),
    ];
    CODES
        .iter()
        .find(|(prefix, _)| error.starts_with(prefix))
        .map_or("other", |(_, category)| category)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    struct Echo;

    impl AiBroker for Echo {
        fn dispatch(
            &self,
            _request: AiBrokerRequest,
            _on_delta: &mut dyn FnMut(TurnDelta<'_>),
        ) -> AiBrokerResponse {
            AiBrokerResponse::ok("echo".to_string(), 1, 1)
        }
    }

    /// A plugin's request id is reply data only: an interior NUL, which
    /// `std::thread::Builder` rejects in a thread name by panicking, still
    /// gets a worker and a delivered reply.
    #[test]
    fn a_request_id_with_a_nul_byte_neither_names_the_worker_nor_panics() {
        let (tx, rx) = mpsc::channel();
        let query = PluginAiQuery {
            plugin_id: "probe".to_string(),
            request_id: "q\0evil".to_string(),
            mode: ModelMode::Fast,
            system: String::new(),
            messages: Vec::new(),
        };
        spawn(
            Arc::new(Echo),
            CallId::next(),
            query,
            Box::new(|_| {}),
            Box::new(move |response| tx.send(response.content).expect("receiver")),
        )
        .expect("worker spawned");
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(10)).expect("delivered"),
            Some("echo".to_string())
        );
    }

    /// The trace of a failed call names a bounded category and the host's
    /// call id; the provider's error body and the plugin's text never reach
    /// the log.
    #[test]
    fn a_failed_call_traces_its_category_and_call_id_not_its_message() {
        let logs = crate::testing::test_log::capture();
        let call = CallId::next();
        trace_outcome(
            "probe",
            call,
            Some(ModelMode::Slow),
            &AiBrokerResponse::err("backend error: 400 {\"echo\":\"SECRET-PROMPT\"}"),
        );
        trace_outcome(
            "probe",
            call,
            None,
            &AiBrokerResponse::err("something new: SECRET-DETAIL"),
        );
        let lines = logs.lines();
        assert_eq!(
            lines,
            vec![
                format!("ai.query: plugin=probe call={call} mode=slow outcome=error category=backend_error"),
                format!("ai.query: plugin=probe call={call} mode=invalid outcome=error category=other"),
            ]
        );
    }
}
