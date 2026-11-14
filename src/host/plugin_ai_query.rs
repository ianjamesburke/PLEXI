//! Plugin `ai.query` dispatch shared by the WASM and CPython plugin panes.
//!
//! Each pane runtime decides admission (`ai.query` grant) and parses its own
//! wire shape; this module owns the part that must be identical for both:
//! building the broker request, running it off the UI thread, and the one
//! `info` trace per call. Config, budget and ledger are applied inside
//! [`AiBroker::dispatch`], so every runtime gets them by construction.

use std::sync::Arc;

use crate::plexi_ai::broker::{AiBroker, AiBrokerRequest, AiBrokerResponse, ModelMode};
use crate::plexi_ai::turn_loop::TurnDelta;
use crate::protocol::AiMessage;

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

/// Run `query` through `broker` on a dedicated worker thread. `deliver` is
/// called exactly once with the response. A spawn failure is returned to the
/// caller, which owns reporting it to the plugin; `deliver` is not called.
pub(crate) fn spawn(
    broker: Arc<dyn AiBroker>,
    query: PluginAiQuery,
    mut on_delta: OnDelta,
    deliver: Deliver,
) -> std::io::Result<()> {
    let PluginAiQuery {
        plugin_id,
        request_id,
        mode,
        system,
        messages,
    } = query;
    // Test profiles are per-thread; the broker's budget and ledger must see
    // the caller's isolated profile, not the shared fallback.
    #[cfg(test)]
    let profile = crate::config::test_profile_dir_override();
    std::thread::Builder::new()
        .name(format!("plugin-ai-query-{plugin_id}-{request_id}"))
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
            trace_outcome(&plugin_id, &request_id, mode.as_str(), &response);
            deliver(response);
        })
        .map(drop)
}

/// The one `info` line per plugin AI call, for brokered and refused calls
/// alike. `mode` is the raw requested value so a refused unknown mode is
/// still traced.
pub(crate) fn trace_outcome(
    plugin_id: &str,
    request_id: &str,
    mode: &str,
    response: &AiBrokerResponse,
) {
    match &response.error {
        None => log::info!(
            "ai.query: plugin={plugin_id} request={request_id} mode={mode} outcome=ok tokens_in={} tokens_out={}",
            response.tokens_in,
            response.tokens_out
        ),
        Some(error) => log::info!(
            "ai.query: plugin={plugin_id} request={request_id} mode={mode} outcome=error error={error:?}"
        ),
    }
}
