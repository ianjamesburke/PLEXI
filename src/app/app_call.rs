//! `plexi app call` — invoke an app-exposed tool from the command socket.
//!
//! The call runs through the same `ToolDispatcher` the Assistant and the host
//! MCP server use, so the app receives an ordinary `ToolCall` and answers with
//! an ordinary `ToolResult`. The host decides two things the caller cannot:
//! the viewer context (the calling pane's live context, or the credential's
//! context for a caller outside any pane) and the caller identity the app sees
//! (`pane:<id>`, `agent:<id>`, `mcp:pane:<id>`, or `session:<id>`). A missing
//! pane is never the human `user`.
//!
//! `dispatch_call` blocks until the app answers, and the app's answer is
//! drained by `App::logic`, so the call runs on a worker thread and writes the
//! reply file itself — the frame loop never waits on a guest.

use super::{ApprovalButton, PlexiApp};

/// The identity an app sees for a verified socket caller. No pane is never
/// the human `user`.
pub(crate) fn caller_identity(caller_pane_id: Option<u64>, session_actor: Option<&str>) -> String {
    match (caller_pane_id, session_actor) {
        (_, Some(actor)) if !actor.is_empty() => actor.to_string(),
        (Some(pane_id), _) => format!("pane:{pane_id}"),
        (None, _) => "session:unscoped".to_string(),
    }
}

impl PlexiApp {
    pub(crate) fn observe_permissions(
        &mut self,
        op: &str,
        pending_id: Option<&String>,
        choice: Option<&String>,
        response_file: &str,
    ) {
        let monitor =
            crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir());
        let body = match op {
            "list" => {
                let pending = monitor.list_pending();
                log::info!(
                    "permission_monitor: list pending count={}",
                    pending.len()
                );
                let buttons: Vec<serde_json::Value> = self
                    .approval_buttons
                    .iter()
                    .map(|button| {
                        serde_json::json!({
                            "label": button.label,
                            "bounds": button.bounds,
                            "pending_request_id": button.pending_request_id,
                        })
                    })
                    .collect();
                let mut panes = Vec::new();
                for window in &self.windows {
                    for (pane_id, pane) in &window.panes {
                        let Some(tile) = window.tree.tiles.find_pane(pane_id) else {
                            continue;
                        };
                        let Some(rect) = window.tree.tiles.rect(tile) else {
                            continue;
                        };
                        panes.push(serde_json::json!({
                            "id": pane_id,
                            "manifest_id": pane.as_app().map(|app| app.manifest_id.as_str()).unwrap_or(""),
                            "bounds": [rect.min.x, rect.min.y, rect.max.x, rect.max.y],
                        }));
                    }
                }
                serde_json::json!({
                    "ok": true,
                    "pending": pending,
                    "audit": monitor.audit_records(),
                    "buttons": buttons,
                    "panes": panes,
                })
            }
            "show" => {
                let id = pending_id.map(String::as_str).unwrap_or("");
                match monitor.show_pending(id) {
                    Some(row) => {
                        log::info!("permission_monitor: show pending {id}");
                        serde_json::json!({"ok": true, "pending": row})
                    }
                    None => serde_json::json!({
                        "ok": false,
                        "error_code": "permission_denied",
                        "error": format!("unknown pending request {id}"),
                    }),
                }
            }
            "resolve" => {
                // No CLI or socket path resolves. The desktop banner and a
                // paired device are the only principals that call approve_pending.
                let id = pending_id.map(String::as_str).unwrap_or("");
                let _choice = choice.map(String::as_str).unwrap_or("");
                monitor.refuse_client_resolve(id);
                serde_json::json!({
                    "ok": false,
                    "error_code": "permission_denied",
                    "error": "only the person at the desktop can resolve a permission",
                    "pending_request_id": id,
                })
            }
            _ => serde_json::json!({"ok": false, "error": "unknown permission operation"}),
        };
        crate::rpc::write_response(response_file, body.to_string().as_bytes());
    }

    pub(crate) fn observe_needs_you(
        &mut self,
        op: &str,
        id: Option<&String>,
        approve: Option<bool>,
        response_file: &str,
    ) {
        let monitor =
            crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir());
        let body = match op {
            "list" => {
                let items = monitor.list_needs_you();
                log::info!("needs_you: host list count={}", items.len());
                serde_json::json!({"ok": true, "items": items})
            }
            "resolve" => {
                // The socket cannot resolve. A desktop input event calls
                // `approve_pending` / `resolve_needs_you` directly.
                let id = id.map(String::as_str).unwrap_or("");
                let _approve = approve.unwrap_or(false);
                monitor.refuse_client_resolve(id);
                serde_json::json!({
                    "ok": false,
                    "error_code": "permission_denied",
                    "error": "only the person at the desktop can resolve a permission",
                    "pending_request_id": id,
                })
            }
            _ => serde_json::json!({"ok": false, "error": "unknown needs-you operation"}),
        };
        crate::rpc::write_response(response_file, body.to_string().as_bytes());
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn call_app_tool(
        &mut self,
        app_id: String,
        tool: String,
        input_json: String,
        caller_pane_id: Option<u64>,
        call_credential: Option<String>,
        peer_ancestry: Vec<u32>,
        target_pane_id: Option<u64>,
        response_file: Option<String>,
    ) {
        let reply = move |body: serde_json::Value| {
            if let Some(rf) = &response_file {
                crate::rpc::write_response(rf, body.to_string().as_bytes());
            }
        };
        let monitor =
            crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir());
        let peer_pane = self
            .resolve_socket_peer_pane(&peer_ancestry)
            .map(|(pane_id, _, _)| pane_id);
        let verified = match monitor.authenticate_call(
            caller_pane_id,
            call_credential.as_deref(),
            peer_pane,
        ) {
            Ok(caller) => caller,
            Err(error) => {
                let actor = caller_pane_id
                    .map(|pane| format!("pane:{pane}"))
                    .unwrap_or_else(|| "session:unscoped".to_string());
                monitor.note_denial(&actor, "app-call", &app_id, "", "identity");
                log::info!(
                    "app_call: denied app={app_id} tool={tool} identity={error:?} claim={caller_pane_id:?} actor={actor}"
                );
                let code = match error {
                    crate::broker::gate::IdentityError::StaleCredential => "permission_denied",
                    crate::broker::gate::IdentityError::Forged
                    | crate::broker::gate::IdentityError::Mismatch
                    | crate::broker::gate::IdentityError::Missing => "permission_denied",
                };
                reply(serde_json::json!({
                    "ok": false,
                    "error_code": code,
                    "error": crate::broker::gate::structured_error(code, "app-call", None),
                }));
                return;
            }
        };
        if verified.is_human {
            reply(serde_json::json!({
                "ok": false,
                "error_code": "permission_denied",
                "error": "socket caller cannot be the human user",
            }));
            return;
        }
        let context_id = if verified.context_id != 0 {
            verified.context_id
        } else if let Some(pane_id) = verified.pane_id {
            match self.find_pane_in_any_window(pane_id) {
                Some((win_idx, _)) => self.windows[win_idx].context_id,
                None => {
                    log::warn!("app_call: refused — verified pane {pane_id} is not live");
                    reply(serde_json::json!({
                        "ok": false,
                        "error_code": "permission_denied",
                        "error": format!("caller pane {pane_id} not found"),
                    }));
                    return;
                }
            }
        } else {
            // A no-pane session may ask in an explicit context. It is not the human.
            self.windows[self.active_window].context_id
        };
        let workspace = if verified.workspace_root.as_os_str().is_empty() {
            self.context_root_for(context_id)
                .unwrap_or_else(|| std::path::PathBuf::from("."))
        } else {
            verified.workspace_root.clone()
        };
        let caller = caller_identity(verified.pane_id, Some(&verified.actor_id));
        let mut scope = crate::plexi_ai::tool_dispatch::DispatchScope::new(
            verified.pane_id.unwrap_or(0),
            caller.clone(),
            workspace,
            context_id,
        );
        scope.actor_type = verified.actor_type;
        scope.actor_scope = verified.actor_scope;
        let dispatcher = crate::plexi_ai::tool_dispatch::ToolDispatcher::namespaced_for(
            scope,
            std::sync::Arc::clone(&monitor),
        );
        let namespaced = match dispatcher.select_app_tool(&app_id, &tool, target_pane_id) {
            Ok(name) => name,
            Err(error) => {
                monitor.note_denial(&caller, "app-call", &app_id, "", "ambiguous_instance");
                log::info!("app_call: {error}");
                reply(serde_json::json!({
                    "ok": false,
                    "error_code": "permission_denied",
                    "error": error,
                }));
                return;
            }
        };
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
                        log::info!(
                            "app_call: call_id={call_id} rejected code={:?}: {error}",
                            result.error_code
                        );
                        serde_json::json!({
                            "ok": false,
                            "error": error,
                            "error_code": result.error_code,
                            "pending_request_id": result.pending_request_id,
                        })
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

    /// Floating Allow once control for a pending app call. Real pointer
    /// clicks approve. Socket-injected input is ignored for the frame.
    pub(crate) fn draw_approval_banner(&mut self, ctx: &egui::Context) {
        let monitor =
            crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir());
        let pending = monitor.list_pending();
        self.approval_buttons.clear();
        if pending.is_empty() {
            return;
        }
        let pending_id = pending[0].pending_request_id.clone();
        let caption = format!("{} wants {}", pending[0].actor_id, pending[0].tool);
        let synthetic = self.synthetic_input_frame;
        let mut clicked = false;
        let mut rect = None;
        egui::Area::new(egui::Id::new("approval_banner"))
            .fixed_pos(egui::pos2(12.0, 8.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(caption);
                    let response = ui.add(egui::Button::new("Allow once"));
                    rect = Some(response.rect);
                    clicked = response.clicked();
                });
            });
        if let Some(rect) = rect {
            self.approval_buttons.push(ApprovalButton {
                label: "Allow once".to_string(),
                bounds: [rect.min.x, rect.min.y, rect.max.x, rect.max.y],
                pending_request_id: pending_id.clone(),
            });
        }
        if clicked && !synthetic {
            match monitor.approve_pending(
                &pending_id,
                crate::broker::gate::ApprovalChoice::Once,
            ) {
                Ok(()) => {
                    log::info!("permission_monitor: banner approved {pending_id}")
                }
                Err(error) => {
                    log::warn!("permission_monitor: banner approve {pending_id} failed: {error}")
                }
            }
        } else if clicked {
            log::info!("permission_monitor: ignored synthetic banner click {pending_id}");
        }
    }

    fn oldest_pending_id(&self) -> Option<String> {
        crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir())
            .list_pending()
            .into_iter()
            .next()
            .map(|row| row.pending_request_id)
    }

    /// Pending id a synthetic key, click, or approval verb must not resolve.
    pub(crate) fn synthetic_approval_target(
        &self,
        pane_id: u64,
        key: Option<&str>,
        click_abs: Option<egui::Pos2>,
    ) -> Option<String> {
        if let Some(pos) = click_abs {
            for button in &self.approval_buttons {
                let [x0, y0, x1, y1] = button.bounds;
                if pos.x >= x0 && pos.x < x1 && pos.y >= y0 && pos.y < y1 {
                    return Some(button.pending_request_id.clone());
                }
            }
        }
        let app = self
            .windows
            .iter()
            .find_map(|window| window.panes.get(&pane_id).and_then(|pane| pane.as_app()));
        let app = app?;
        if app.manifest_id == "permissions" {
            return Some(
                self.oldest_pending_id()
                    .unwrap_or_else(|| "permissions".to_string()),
            );
        }
        if let Some(id) = app.runtime.approval_request_id() {
            if !id.is_empty() {
                return Some(id);
            }
        }
        let sheet_key = key.is_some_and(|raw| {
            matches!(
                raw.to_ascii_lowercase().as_str(),
                "right" | "left" | "up" | "down" | "enter" | "return" | "space" | "tab" | "escape"
            )
        });
        if app.runtime.type_id() == "assistant" && sheet_key {
            return self.oldest_pending_id();
        }
        None
    }

    /// Write `permission_denied` and an audit row. Returns true when the
    /// event must not be injected.
    pub(crate) fn refuse_synthetic_approval(
        &mut self,
        pane_id: u64,
        key: Option<&str>,
        click_abs: Option<egui::Pos2>,
        response_file: Option<&str>,
    ) -> bool {
        let Some(pending_id) = self.synthetic_approval_target(pane_id, key, click_abs) else {
            return false;
        };
        crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir())
            .refuse_client_resolve(&pending_id);
        log::info!(
            "permission_monitor: refused synthetic input pane={pane_id} pending={pending_id}"
        );
        if let Some(path) = response_file {
            crate::rpc::write_json_response(
                path,
                serde_json::json!({
                    "ok": false,
                    "error_code": "permission_denied",
                    "error": "synthetic input cannot resolve a permission",
                    "pending_request_id": pending_id,
                }),
            );
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::caller_identity;

    #[test]
    fn socket_caller_identity_is_never_the_human() {
        assert_eq!(caller_identity(Some(42), None), "pane:42");
        assert_eq!(caller_identity(None, Some("session:abc")), "session:abc");
        assert_ne!(caller_identity(None, None), "user");
    }
}
