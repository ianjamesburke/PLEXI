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

/// Observes a human decision; it never creates an allow grant. Dispatch then
/// re-admits the captured request, so revocation before execution still wins.
struct WaitingApproval {
    monitor: std::sync::Arc<crate::broker::gate::PermissionMonitor>,
    peer_pid: u32,
    timeout: std::time::Duration,
}

impl crate::plexi_ai::tool_dispatch::PermissionPresenter for WaitingApproval {
    fn present(&self, pending_id: &str, _tool: &str, _summary: &str, _actor: &str, _resource: &str)
        -> Result<crate::broker::gate::ApprovalChoice, String>
    {
        let started = std::time::Instant::now();
        log::info!("app_call: waiting for desktop approval pending={pending_id}");
        loop {
            let expired = started.elapsed() >= self.timeout;
            if expired || !crate::host::pane_liveness::pid_is_alive(self.peer_pid) {
                // Only cancel this request. A late click cannot restart this worker.
                if let Err(error) = self.monitor.approve_pending(pending_id, crate::broker::gate::ApprovalChoice::Deny) {
                    log::info!("app_call: cancellation pending={pending_id}: {error}");
                }
                let reason = if expired { "approval_timeout" } else { "caller_exited" };
                log::info!("app_call: {reason} pending={pending_id}");
                return Err(format!("{reason}: request was not executed"));
            }
            if let Some(choice) = self.monitor.approval_resolution(pending_id) {
                log::info!("app_call: desktop resolved pending={pending_id} choice={choice:?}");
                return Ok(choice);
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}

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
                log::info!("permission_monitor: list pending count={}", pending.len());
                let mut buttons: Vec<serde_json::Value> = monitor.sheet_buttons_json();
                buttons.extend(self.approval_buttons.iter().map(|button| {
                    serde_json::json!({
                        "label": button.label,
                        "bounds": button.bounds,
                        "pending_request_id": button.pending_request_id,
                    })
                }));
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
                // No CLI, socket, or phone path resolves. A real pointer click
                // on the desktop banner or the pairing sheet is what grants.
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

    pub(crate) fn permissions_cli_request(
        &mut self,
        op: &str,
        id: Option<&str>,
        pane_id: Option<u64>,
        credential: Option<&str>,
        response_file: &str,
    ) {
        let monitor =
            crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir());
        let credential_set = credential.is_some_and(|value| !value.is_empty());
        let human = pane_id.is_none() && !credential_set;
        let actor_id = match pane_id {
            Some(id) => format!("pane:{id}"),
            None if credential_set => "agent:credential".to_string(),
            None => "user".to_string(),
        };
        let body = if op == "list" {
            let entries = monitor.list_entries();
            log::info!(
                "permissions:cli list count={} human={human} actor={actor_id}",
                entries.len()
            );
            serde_json::json!({"ok": true, "entries": entries})
        } else {
            let id = id.unwrap_or("");
            let outcome = monitor.mutate_entry(
                id,
                op,
                &crate::broker::gate::PermissionCaller {
                    human,
                    actor_id: actor_id.clone(),
                },
            );
            log::info!("permissions:cli {op} id={id} human={human} actor={actor_id}");
            let mut body = serde_json::to_value(&outcome).unwrap_or(serde_json::Value::Null);
            let ok = matches!(
                outcome,
                crate::broker::gate::PermissionMutation::Applied { .. }
            );
            if let Some(map) = body.as_object_mut() {
                map.insert("ok".to_string(), serde_json::json!(ok));
            }
            body
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
        // The host opens its monitor before accepting its command socket. Do
        // not make an observation request open a profile or touch Keychain:
        // callers need a bounded, typed answer while Keychain recovery is
        // pending, not a second startup attempt on the command path.
        let profile = crate::config::config_dir();
        let Some(monitor) = crate::broker::gate::PermissionMonitor::loaded(&profile) else {
            log::warn!("needs_you: host query arrived before the monitor was ready");
            crate::rpc::write_response(
                response_file,
                serde_json::json!({
                    "ok": false,
                    "error_code": "host_not_ready",
                    "error": "needs-you is not ready; retry after the host finishes starting",
                })
                .to_string()
                .as_bytes(),
            );
            return;
        };
        let body = match op {
            "list" => {
                let items = crate::broker::gate::needs_you_phone_items(&monitor.list_needs_you());
                log::info!("needs_you: host list count={}", items.len());
                serde_json::json!({"ok": true, "items": items})
            }
            "resolve" => {
                let id = id.map(String::as_str).unwrap_or("");
                let approve = approve.unwrap_or(false);
                let grants = monitor.list_needs_you().iter().any(|row| {
                    row.id == id
                        && matches!(
                            row.kind,
                            crate::broker::gate::NeedsYouKind::ApprovalClick
                                | crate::broker::gate::NeedsYouKind::PermissionChange
                        )
                }) || monitor.show_pending(id).is_some();
                if grants && approve {
                    // Socket and protocol resolves never mint a grant. The
                    // desktop banner calls approve_pending directly, and only
                    // when the click was not synthetic. Deny still settles.
                    // A phone or terminal approve is permission_denied.
                    log::info!("needs_you: refused socket resolve {id} approve={approve}");
                    monitor.refuse_client_resolve(id);
                    serde_json::json!({
                        "ok": false,
                        "error_code": "permission_denied",
                        "error": "only the person at the desktop can resolve a permission",
                        "id": id,
                    })
                } else {
                    log::info!("needs_you: host resolve {id} approve={approve}");
                    match monitor.resolve_needs_you(id, approve) {
                        Ok(receipt) => serde_json::json!({
                            "ok": !receipt.already,
                            "id": receipt.id,
                            "resolution": receipt.resolution.as_str(),
                            "already": receipt.already,
                            "run_outcome": receipt.run_outcome.as_str(),
                        }),
                        Err(error) => serde_json::json!({
                            "ok": false,
                            "error": error,
                        }),
                    }
                }
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
        wait_approved: bool,
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
        let mut dispatcher = crate::plexi_ai::tool_dispatch::ToolDispatcher::namespaced_for(
            scope,
            std::sync::Arc::clone(&monitor),
        );
        if wait_approved {
            let Some(peer_pid) = peer_ancestry.first().copied() else {
                reply(serde_json::json!({"ok":false, "error_code":"permission_denied",
                    "error":"approval waiting requires a verified live socket peer"}));
                return;
            };
            dispatcher.set_presenter(std::sync::Arc::new(WaitingApproval {
                monitor: std::sync::Arc::clone(&monitor), peer_pid,
                timeout: crate::cli::app::APP_CALL_APPROVAL_TIMEOUT,
            }));
        }
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

    /// A framed, nonmodal permission surface. Only real pointer decisions
    /// grant authority; ordinary typing continues in the active pane.
    pub(crate) fn draw_approval_banner(&mut self, ctx: &egui::Context) {
        let monitor =
            crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir());
        let pending = monitor.list_pending();
        self.approval_buttons.clear();
        if pending.is_empty() {
            self.approval_banner_logged = None;
            return;
        }
        let pending_id = pending[0].pending_request_id.clone();
        if self.approval_banner_logged.as_deref() != Some(pending_id.as_str()) {
            log::info!(
                "permission_monitor: approval banner shown pending={pending_id} actor={} tool={}",
                pending[0].actor_id,
                pending[0].tool
            );
            self.approval_banner_logged = Some(pending_id.clone());
        }
        let request = &pending[0];
        let actor = request.actor_id.strip_prefix("pane:")
            .map(|id| format!("Terminal pane {id}"))
            .or_else(|| request.actor_id.strip_prefix("agent:").map(|id| format!("Agent {id}")))
            .unwrap_or_else(|| request.actor_id.clone());
        let tool = request.tool.replace("__", " / ");
        let caption = format!("{actor} wants to use {tool}");
        let synthetic = self.synthetic_input_frame;
        let mut choices = vec![
            ("Allow once", crate::broker::gate::ApprovalChoice::Once),
        ];
        if request.allow_tool {
            choices.push(("Allow for this session", crate::broker::gate::ApprovalChoice::AllowTool));
        }
        choices.push(("Deny", crate::broker::gate::ApprovalChoice::Deny));
        choices.push(("Always deny", crate::broker::gate::ApprovalChoice::DenyAlways));
        let mut chosen = None;
        crate::ui::overlay::ModalShell::centered("approval_banner")
            .title("Permission requested")
            .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -crate::ui::style::SPACE_MD))
            .scrim(false)
            .click_away(false)
            .show(ctx, &self.colors, |ui| {
                crate::ui::typography::body_strong(ui, caption, &self.colors);
                if let Some(resource) = &request.resource_id {
                    crate::ui::typography::body(ui, format!("Resource: {resource}"), &self.colors);
                }
                if request.input_summary != "{}" && !request.input_summary.is_empty() {
                    ui.collapsing("Request details", |ui| {
                        crate::ui::typography::body(ui, &request.input_summary, &self.colors);
                    });
                }
                if request.allow_tool {
                    crate::ui::typography::caption(ui,
                        "Session approval covers different arguments for this tool and caller, in this app instance and workspace only. The resource stays the same. Ends when Plexi quits; revoke in Permissions.", &self.colors);
                }
                ui.add_space(crate::ui::style::SPACE_MD);
                ui.horizontal_wrapped(|ui| {
                    for &(label, choice) in &choices {
                        let kind = if choice == crate::broker::gate::ApprovalChoice::Once {
                            crate::ui::button::ButtonKind::Accent
                        } else {
                            crate::ui::button::ButtonKind::Primary
                        };
                        let response = crate::ui::button::chrome_button(ui, label, kind, &self.colors, 0.0);
                        let rect = response.rect;
                        self.approval_buttons.push(ApprovalButton {
                            label: label.to_string(),
                            bounds: [rect.min.x, rect.min.y, rect.max.x, rect.max.y],
                            pending_request_id: pending_id.clone(),
                        });
                        if response.clicked_by(egui::PointerButton::Primary) {
                            chosen = Some((label, choice));
                        }
                    }
                });
                if pending.len() > 1 {
                    crate::ui::typography::caption(ui, format!("{} more requests waiting", pending.len() - 1), &self.colors);
                }
            });
        if let Some((label, choice)) = chosen {
            if synthetic {
                log::info!(
                    "permission_monitor: ignored synthetic banner click {pending_id} choice={label}"
                );
            } else {
                match monitor.approve_pending(&pending_id, choice) {
                    Ok(()) => {
                        log::info!("permission_monitor: banner decided {pending_id} choice={label}")
                    }
                    Err(error) => {
                        log::warn!(
                            "permission_monitor: banner decide {pending_id} choice={label} failed: {error}"
                        )
                    }
                }
            }
        }
    }

    /// Surface the one recovery action for a host seal key that startup could
    /// not read without prompting. This is deliberately separate from a tool
    /// approval: it reloads or rotates the host seal and never grants an app
    /// capability.
    pub(crate) fn draw_keychain_recovery_banner(&mut self, ctx: &egui::Context) {
        // The attention review offers the same recovery action; never stack
        // two competing recovery overlays.
        if self.attention.open {
            return;
        }
        let monitor =
            crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir());
        let Some(item) = monitor
            .open_needs_you()
            .into_iter()
            .find(|item| item.kind == crate::broker::gate::NeedsYouKind::Keychain)
        else {
            return;
        };

        let mut choice = None;
        crate::ui::overlay::ModalShell::centered("keychain_recovery")
            .title("Keychain access needed")
            .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -crate::ui::style::SPACE_MD))
            .scrim(false)
            .click_away(false)
            .show(ctx, &self.colors, |ui| {
                crate::ui::typography::body_strong(
                    ui,
                    "Saved permissions are unavailable.",
                    &self.colors,
                );
                crate::ui::typography::body(
                    ui,
                    "Continue lets macOS ask for access to Plexi’s saved permission key. If it remains unavailable, Plexi safely replaces that key and resets only permissions it can no longer verify.",
                    &self.colors,
                );
                ui.add_space(crate::ui::style::SPACE_MD);
                ui.horizontal_wrapped(|ui| {
                    if crate::ui::button::chrome_button(
                        ui,
                        "Continue",
                        crate::ui::button::ButtonKind::Accent,
                        &self.colors,
                        0.0,
                    )
                    .clicked_by(egui::PointerButton::Primary)
                    {
                        choice = Some(true);
                    }
                    if crate::ui::button::chrome_button(
                        ui,
                        "Not now",
                        crate::ui::button::ButtonKind::Secondary,
                        &self.colors,
                        0.0,
                    )
                    .clicked_by(egui::PointerButton::Primary)
                    {
                        choice = Some(false);
                    }
                });
            });

        let Some(approve) = choice else {
            return;
        };
        self.resolve_from_desktop(&item.id, approve, "keychain recovery");
    }

    /// Resolve a Needs-you record from a desktop click or key. Synthetic
    /// input, sockets and phones never reach this: it is the single guarded
    /// desktop path shared by the recovery banner and the attention review.
    /// Resolving a record never mints an app grant.
    pub(crate) fn resolve_from_desktop(&mut self, id: &str, approve: bool, label: &str) {
        if self.synthetic_input_frame {
            log::info!(
                "permission_seal: ignored synthetic {label} click id={id} approve={approve}"
            );
            return;
        }
        let monitor =
            crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir());
        match monitor.resolve_needs_you(id, approve) {
            Ok(receipt) => log::info!(
                "permission_seal: desktop {label} resolved id={} resolution={}",
                receipt.id,
                receipt.resolution.as_str()
            ),
            Err(error) => log::error!("permission_seal: desktop {label} failed id={id}: {error}"),
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
    use crate::broker::gate::NeedsYouKind;

    #[test]
    fn waiting_approval_timeout_and_dead_caller_cancel_only_the_pending_call() {
        use crate::broker::gate::{AdmitRequest, Admission, ApprovalChoice, PermissionMonitor};
        use crate::broker::{ActorScope, ActorType, TargetType};
        use crate::plexi_ai::tool_dispatch::PermissionPresenter;
        for (peer_pid, timeout, reason) in [
            (std::process::id(), std::time::Duration::ZERO, "approval_timeout"),
            (0, std::time::Duration::from_secs(1), "caller_exited"),
        ] {
            let monitor = PermissionMonitor::ephemeral();
            let workspace = tempfile::tempdir().unwrap();
            let ask = |call_id, input_json| match monitor.admit(AdmitRequest {
                call_id, tool: "probe.add", input_json, actor_type: ActorType::Agent,
                actor_id: "agent:waiter", actor_scope: ActorScope::User, trust_origin: "host",
                workspace_root: workspace.path(), context_id: 1, package_id: "probe",
                instance_id: 1, target_type: TargetType::AppConnector,
            }) {
                Admission::Required { pending_request_id } => pending_request_id,
                _ => panic!("expected pending request"),
            };
            let pending = ask("waited", "{}");
            let unrelated = ask("other", r#"{"other":true}"#);
            let waiter = super::WaitingApproval { monitor: monitor.clone(), peer_pid, timeout };
            let error = waiter.present(&pending, "probe.add", "{}", "agent:waiter", "").unwrap_err();
            assert!(error.contains(reason), "{error}");
            assert_eq!(monitor.approval_resolution(&pending), Some(ApprovalChoice::Deny));
            assert!(monitor.approve_pending(&pending, ApprovalChoice::Once).is_err(), "late approval must not resume cancelled work");
            assert!(monitor.show_pending(&unrelated).is_some());
            assert!(!monitor.approval_granted(&pending));
        }
    }

    fn phone_may_answer(kind: Option<NeedsYouKind>) -> bool {
        kind.is_some_and(|kind| kind.phone_may_approve())
    }

    #[test]
    fn phone_approve_answers_only_a_visible_non_click() {
        assert!(phone_may_answer(Some(NeedsYouKind::Question)));
        assert!(phone_may_answer(Some(NeedsYouKind::BlockedRun)));
        assert!(!phone_may_answer(Some(NeedsYouKind::ApprovalClick)));
        assert!(!phone_may_answer(Some(NeedsYouKind::PermissionChange)));
        assert!(!phone_may_answer(Some(NeedsYouKind::Integrity)));
        assert!(!phone_may_answer(Some(NeedsYouKind::Keychain)));
        assert!(!phone_may_answer(None));
    }

    #[test]
    fn socket_caller_identity_is_never_the_human() {
        assert_eq!(caller_identity(Some(42), None), "pane:42");
        assert_eq!(caller_identity(None, Some("session:abc")), "session:abc");
        assert_ne!(caller_identity(None, None), "user");
    }
}
