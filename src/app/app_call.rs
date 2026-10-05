//! `plexi app call` — invoke an app-exposed tool from the command socket.
//!
//! The call runs through the same `ToolDispatcher` the Assistant and the host
//! MCP server use, so the app receives an ordinary `ToolCall` and answers with
//! an ordinary `ToolResult`. The host decides two things the caller cannot:
//! the viewer context (the calling pane's live context, or the active window's
//! context for a caller outside any pane) and the caller identity the app sees
//! (`pane:<id>` or `user`).
//!
//! `dispatch_call` blocks until the app answers, and the app's answer is
//! drained by `App::logic`, so the call runs on a worker thread and writes the
//! reply file itself — the frame loop never waits on a guest.

use super::PlexiApp;

/// The identity an app sees for a socket caller. A pane id the host cannot
/// find is refused rather than downgraded to `user`, so a stale or forged id
/// never gains the outside-a-pane identity.
pub(crate) fn caller_identity(caller_pane_id: Option<u64>) -> String {
    match caller_pane_id {
        Some(pane_id) => format!("pane:{pane_id}"),
        None => "user".to_string(),
    }
}

impl PlexiApp {
    pub(crate) fn call_app_tool(
        &mut self,
        app_id: String,
        tool: String,
        input_json: String,
        caller_pane_id: Option<u64>,
        response_file: Option<String>,
    ) {
        let reply = move |body: serde_json::Value| {
            if let Some(rf) = &response_file {
                crate::rpc::write_response(rf, body.to_string().as_bytes());
            }
        };
        let context_id = match caller_pane_id {
            Some(pane_id) => match self.find_pane_in_any_window(pane_id) {
                Some((win_idx, _)) => self.windows[win_idx].context_id,
                None => {
                    log::warn!(
                        "app_call: refused app={app_id} tool={tool} — caller pane {pane_id} not found"
                    );
                    reply(serde_json::json!({
                        "error": format!("caller pane {pane_id} not found")
                    }));
                    return;
                }
            },
            None => self.windows[self.active_window].context_id,
        };
        let caller = caller_identity(caller_pane_id);
        let dispatcher = crate::plexi_ai::tool_dispatch::ToolDispatcher::namespaced_for(
            caller_pane_id.unwrap_or(0),
            caller.clone(),
            context_id,
        );
        let namespaced = format!("{app_id}__{tool}");
        let call_id = format!("cli-{}", uuid::Uuid::new_v4());
        log::info!(
            "app_call: caller={caller} context={context_id} app={app_id} tool={tool} call_id={call_id}"
        );
        let spawned = std::thread::Builder::new()
            .name("app-call".to_string())
            .spawn(move || {
                let result = dispatcher.dispatch_call(call_id.clone(), &namespaced, input_json);
                let body = match (result.error, result.output_json) {
                    (Some(error), _) => {
                        log::info!("app_call: call_id={call_id} rejected: {error}");
                        serde_json::json!({"error": error})
                    }
                    (None, Some(output)) => {
                        let output = serde_json::from_str::<serde_json::Value>(&output)
                            .unwrap_or(serde_json::Value::String(output));
                        log::info!("app_call: call_id={call_id} ok");
                        serde_json::json!({"ok": true, "output": output})
                    }
                    (None, None) => serde_json::json!({"ok": true, "output": null}),
                };
                reply(body);
            });
        if let Err(error) = spawned {
            log::error!("app_call: failed to spawn worker thread: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::caller_identity;

    #[test]
    fn socket_caller_identity_is_pane_or_user() {
        assert_eq!(caller_identity(Some(42)), "pane:42");
        assert_eq!(caller_identity(None), "user");
    }
}
