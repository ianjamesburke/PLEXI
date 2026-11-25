//! `plexi plugin call` — invoke a plugin-exposed tool from the command socket.
//!
//! The call runs through the same `ToolDispatcher` the Assistant and the host
//! MCP server use, so the plugin receives an ordinary `ToolCall` and answers with
//! an ordinary `ToolResult`. The host decides two things the caller cannot:
//! the viewer context (the calling pane's live context, or the credential's
//! context for a caller outside any pane) and the caller identity the plugin sees
//! through its verified principal. Pane credentials identify a single terminal lifetime.
//!
//! `dispatch_call` blocks until the plugin answers, and the plugin's answer is
//! drained by `App::logic`, so the call runs on a worker thread and writes the
//! reply file itself — the frame loop never waits on a guest.

use super::PlexiApp;

/// Observes a human decision; it never creates an allow grant. Dispatch then
/// re-admits the captured request, so revocation before execution still wins.
struct WaitingApproval {
    principal: crate::broker::principal::Principal,
    monitor: std::sync::Arc<crate::broker::gate::PermissionMonitor>,
    /// The connecting process at its kernel incarnation: a reused pid is
    /// not the caller that asked (stint 1285).
    peer: crate::broker::caller::PeerHop,
    /// A pane caller's grant as it was admitted: closing, replacing or
    /// re-scoping the pane while it waits cancels the call (Codex 3290 r1).
    authority: Option<crate::host::plugin_timeline::AdmittedScope>,
    timeout: std::time::Duration,
}

impl crate::plexi_ai::tool_dispatch::PermissionPresenter for WaitingApproval {
    fn present(
        &self,
        pending_id: &str,
        _tool: &str,
        _summary: &str,
        _actor: &str,
        _resource: &str,
    ) -> Result<crate::plexi_ai::tool_dispatch::PresentedChoice, String> {
        let records = self.monitor.records_for(&self.principal);
        let started = std::time::Instant::now();
        log::info!("plugin_call: waiting for desktop approval pending={pending_id}");
        loop {
            let expired = started.elapsed() >= self.timeout;
            let revoked = self
                .authority
                .as_ref()
                .is_some_and(|authority| authority.current().is_err());
            if expired || revoked || !self.peer.is_live() {
                // Only cancel this request. A late click cannot restart this worker.
                if let Err(error) = records.cancel(pending_id) {
                    log::info!("plugin_call: cancellation pending={pending_id}: {error}");
                }
                let reason = if expired {
                    "approval_timeout"
                } else if revoked {
                    "caller_revoked"
                } else {
                    "caller_exited"
                };
                log::info!("plugin_call: {reason} pending={pending_id}");
                return Err(format!("{reason}: request was not executed"));
            }
            if let Some(choice) = records.decision(pending_id) {
                log::info!("plugin_call: desktop resolved pending={pending_id} choice={choice:?}");
                return Ok(crate::plexi_ai::tool_dispatch::PresentedChoice::observed(choice));
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
}

/// The identity a plugin sees for a verified socket caller. No pane is never
/// the human `user`.
/// `--recommend` must name one of the offered `--choice` values.
pub(crate) fn validate_recommend(
    recommend: Option<&str>,
    choices: &[String],
) -> Result<(), String> {
    match recommend {
        None => Ok(()),
        Some(r) if choices.iter().any(|c| c == r) => Ok(()),
        Some(r) => Err(format!(
            "--recommend {r:?} is not one of the choices {choices:?}"
        )),
    }
}

fn ask_recommend_error(payload: &serde_json::Value) -> Option<String> {
    let recommend = payload
        .get("recommend")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())?;
    let choices: Vec<String> = payload
        .get("choices")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.trim().to_string()))
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();
    validate_recommend(Some(recommend), &choices).err()
}

fn records_for_origin<'a>(
    monitor: &'a crate::broker::gate::PermissionMonitor,
    origin: &crate::broker::caller::RequestOrigin,
) -> Option<crate::broker::gate::Records<'a>> {
    use crate::broker::caller::RequestOrigin::*;
    match origin {
        Socket(caller) => caller.principal().map(|owner| monitor.records_for(&owner)),
        PluginGuest(plugin) => {
            Some(monitor.records_for(&crate::broker::principal::Principal::Plugin(plugin.clone())))
        }
        HostUi | HostSystem => Some(monitor.host_records()),
        SpawnQueue => None,
    }
}

pub(super) fn file_needs_you_ask(
    monitor: &crate::broker::gate::PermissionMonitor,
    records: &crate::broker::gate::Records<'_>,
    question: crate::broker::gate::NeedsYouFile,
    details: crate::broker::gate::NeedsYouDetails,
) -> Result<serde_json::Value, String> {
    let id = monitor.file_needs_you_with_details(question, Some(details))?;
    Ok(serde_json::json!({
        "ok": true,
        "id": id,
        "item": records.question(&id),
        "continuation": records.run(&id),
    }))
}

impl PlexiApp {
    pub(crate) fn observe_permissions(
        &mut self,
        origin: &crate::broker::caller::RequestOrigin,
        op: &str,
        pending_id: Option<&String>,
        choice: Option<&String>,
        response_file: &str,
    ) {
        let monitor =
            match crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir())
            {
                Ok(monitor) => monitor,
                Err(error) => {
                    crate::rpc::write_response(
                        response_file,
                        serde_json::json!({"error": error.to_string(), "error_code": "not_host"})
                            .to_string()
                            .as_bytes(),
                    );
                    return;
                }
            };
        // A socket caller observes only its own principal's pending asks and
        // audit rows (stint 1285); host surfaces see them all.
        let records = records_for_origin(&monitor, origin);
        let body = match op {
            "list" => {
                let pending = records
                    .as_ref()
                    .map(|records| records.pending())
                    .unwrap_or_default();
                log::info!("permission_monitor: list pending count={}", pending.len());
                let mut buttons: Vec<serde_json::Value> = monitor.sheet_buttons_json();
                buttons.extend(self.approval_buttons.iter().map(|button| {
                    serde_json::json!({
                        "label": button.label,
                        "bounds": button.bounds,
                        "pending_request_id": button.pending_request_id,
                    })
                }));
                if !matches!(
                    origin,
                    crate::broker::caller::RequestOrigin::HostUi
                        | crate::broker::caller::RequestOrigin::HostSystem
                ) {
                    buttons.retain(|button| {
                        pending.iter().any(|row| {
                            button["pending_request_id"].as_str() == Some(&row.pending_request_id)
                        })
                    });
                }
                let mut panes = Vec::new();
                for window in &self.windows {
                    for (pane_id, pane) in window.panes.iter() {
                        let Some(tile) = window.tree.tiles.find_pane(pane_id) else {
                            continue;
                        };
                        let Some(rect) = window.tree.tiles.rect(tile) else {
                            continue;
                        };
                        panes.push(serde_json::json!({
                            "id": pane_id,
                            "manifest_id": pane.as_view().map(|app| app.manifest_id.as_str()).unwrap_or(""),
                            "bounds": [rect.min.x, rect.min.y, rect.max.x, rect.max.y],
                        }));
                    }
                }
                serde_json::json!({
                    "ok": true,
                    "pending": pending,
                    "audit": records.as_ref().map(|records| records.audit()).unwrap_or_default(),
                    "buttons": buttons,
                    "panes": panes,
                })
            }
            "show" => {
                let id = pending_id.map(String::as_str).unwrap_or("");
                match records.as_ref().and_then(|records| {
                    records
                        .pending()
                        .into_iter()
                        .find(|row| row.pending_request_id == id)
                }) {
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
                // No CLI or socket path resolves. A real pointer click
                // on the desktop banner or the pairing sheet is what grants.
                let id = pending_id.map(String::as_str).unwrap_or("");
                let _choice = choice.map(String::as_str).unwrap_or("");
                if let Some(records) = records {
                    records.record_refusal(id);
                }
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

    /// `plexi permissions` over the command socket. No socket caller is
    /// human: a pane, a credential, or their absence changes nothing about
    /// that (`docs/prm/permissions-broker.md`, admission step 5). `allow` and
    /// `reset` therefore file Needs you; `revoke` narrows and applies.
    /// The caller is the host-classified socket caller; it sees and
    /// revokes only its own principal's rows (stint 1285). The full
    /// inventory lives in the desktop Permissions view.
    pub(crate) fn permissions_cli_request(
        &mut self,
        origin: &crate::broker::caller::RequestOrigin,
        op: &str,
        id: Option<&str>,
        duration: Option<&str>,
        response_file: &str,
    ) {
        let requested = match duration.map(crate::broker::gate::ApprovalChoice::parse_duration) {
            Some(None) => {
                crate::rpc::write_json_response(
                    response_file,
                    serde_json::json!({"ok": false, "error_code": "invalid_argument",
                        "error": "duration must be once, session, or always"}),
                );
                return;
            }
            Some(parsed) => parsed,
            None => None,
        };
        let monitor =
            match crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir())
            {
                Ok(monitor) => monitor,
                Err(error) => {
                    crate::rpc::write_response(
                        response_file,
                        serde_json::json!({"error": error.to_string(), "error_code": "not_host"})
                            .to_string()
                            .as_bytes(),
                    );
                    return;
                }
            };
        let Some(principal) = origin
            .socket_caller()
            .and_then(crate::broker::caller::SocketCaller::principal)
        else {
            crate::rpc::write_json_response(
                response_file,
                serde_json::json!({"ok": false, "error_code": "permission_denied",
                    "error": format!("permissions are not served to a {} caller", origin.class())}),
            );
            return;
        };
        let actor_id = principal.label();
        let body = if op == "list" {
            let entries = monitor.records_for(&principal).entries();
            log::info!(
                "permissions:cli list count={} surface=socket actor={actor_id}",
                entries.len()
            );
            serde_json::json!({"ok": true, "entries": entries})
        } else {
            let id = id.unwrap_or("");
            let outcome = monitor.mutate_entry_for(
                id,
                op,
                &crate::broker::gate::PermissionCaller {
                    owner: crate::broker::principal::RecordOwner::requester(&principal),
                    revoke_scope: crate::broker::gate::RevokeScope::Own,
                },
                requested,
            );
            let mut body = serde_json::to_value(&outcome).unwrap_or(serde_json::Value::Null);
            log::info!(
                "permissions:cli op={op:?} id={id:?} surface=socket actor={actor_id} outcome={}",
                body["status"].as_str().unwrap_or("unknown")
            );
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
        origin: &crate::broker::caller::RequestOrigin,
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
                // A socket caller sees the items it filed; host surfaces see
                // them all (stint 1285).
                let items = records_for_origin(&monitor, origin)
                    .map(|records| records.questions(true))
                    .unwrap_or_default();
                log::info!("needs_you: host list count={}", items.len());
                serde_json::json!({"ok": true, "items": items})
            }
            "resolve" => {
                // The socket never approves and never settles an integrity or
                // keychain item; the broker owns that rule. A real click in
                // the host window is the only path that approves.
                let id = id.map(String::as_str).unwrap_or("");
                let approve = approve.unwrap_or(false);
                match records_for_origin(&monitor, origin)
                    .ok_or_else(|| {
                        crate::broker::gate::NeedsYouResolveError::Failed(format!(
                            "unknown needs-you item {id}"
                        ))
                    })
                    .and_then(|records| records.resolve_from_socket(id, approve))
                {
                    Ok(receipt) => serde_json::json!({
                        "ok": !receipt.already,
                        "id": receipt.id,
                        "resolution": receipt.resolution.as_str(),
                        "already": receipt.already,
                        "run_outcome": receipt.run_outcome.as_str(),
                    }),
                    Err(crate::broker::gate::NeedsYouResolveError::Refused) => serde_json::json!({
                        "ok": false,
                        "error_code": "permission_denied",
                        "error": "only the person at the desktop can do this; decide in the Plexi window",
                        "id": id,
                    }),
                    Err(crate::broker::gate::NeedsYouResolveError::Failed(error)) => {
                        serde_json::json!({
                            "ok": false,
                            "error": error,
                        })
                    }
                }
            }
            _ => serde_json::json!({"ok": false, "error": "unknown needs-you operation"}),
        };
        crate::rpc::write_response(response_file, body.to_string().as_bytes());
    }

    pub(crate) fn handle_needs_you_api(
        &mut self,
        op: &str,
        payload: &serde_json::Value,
        origin: &crate::broker::caller::RequestOrigin,
        response_file: &str,
    ) {
        use crate::broker::caller::SocketCaller;
        let profile = crate::config::config_dir();
        let Some(monitor) = crate::broker::gate::PermissionMonitor::loaded(&profile) else {
            log::warn!("needs_you: host api query ({op}) arrived before the monitor was ready");
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

        let caller_pane = origin.socket_caller().and_then(SocketCaller::pane);
        // The principal a socket caller asks and completes as; never a name
        // from the payload (stint 1285).
        let Some(records) = records_for_origin(&monitor, origin) else {
            log::info!("needs_you: unidentified caller refused");
            crate::rpc::write_json_response(
                response_file,
                serde_json::json!({"ok":false, "error_code":"permission_denied"}),
            );
            return;
        };
        if op == "claude_hook" {
            if let Some(reply) =
                self.claude_question_hook(&monitor, payload, caller_pane, response_file)
            {
                crate::rpc::write_json_response(response_file, reply);
            }
            return;
        }
        let body = match op {
            "ask" if ask_recommend_error(payload).is_some() => {
                let error = ask_recommend_error(payload).unwrap_or_default();
                log::warn!("needs_you: api ask refused: {error}");
                serde_json::json!({ "ok": false, "error": error })
            }
            "ask" => {
                let summary = payload
                    .get("summary")
                    .or_else(|| payload.get("question"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let resource = payload
                    .get("resource")
                    .and_then(|v| v.as_str())
                    .unwrap_or("workspace")
                    .trim()
                    .to_string();
                let resolved_pane = caller_pane.map(|pane| pane.visible_pane);
                let resolved_context = caller_pane
                    .map(|pane| pane.context_id)
                    .unwrap_or_else(|| self.windows[self.active_window].context_id);
                let actor = records.filing_owner();
                let choices: Vec<String> = payload
                    .get("choices")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(|s| s.trim().to_string()))
                            .filter(|s| !s.is_empty())
                            .collect()
                    })
                    .unwrap_or_default();
                let allow_free_text = payload
                    .get("allow_free_text")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(choices.is_empty());
                let recommend = payload
                    .get("recommend")
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty());
                let preview_refs: Vec<String> = payload
                    .get("preview_refs")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(|s| s.trim().to_string()))
                            .filter(|s| !s.is_empty())
                            .collect()
                    })
                    .unwrap_or_default();
                let correlation_id = payload
                    .get("correlation_id")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .map(ToString::to_string);
                let session_id = payload
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .map(ToString::to_string);
                let expected_revision = payload
                    .get("expected_revision")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .map(ToString::to_string);
                let run_tag = payload
                    .get("operation_id")
                    .or_else(|| payload.get("run_tag"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .map(ToString::to_string)
                    .or_else(|| correlation_id.clone());
                let details = crate::broker::gate::NeedsYouDetails {
                    choices,
                    allow_free_text,
                    recommend,
                    preview_refs,
                    correlation_id,
                    session_id,
                    context_id: Some(resolved_context),
                    pane_id: resolved_pane,
                    expected_revision,
                    ..Default::default()
                };
                file_needs_you_ask(
                    &monitor,
                    &records,
                    crate::broker::gate::NeedsYouFile {
                        kind: crate::broker::gate::NeedsYouKind::Question,
                        actor,
                        resource,
                        summary,
                        expires_at: None,
                        run_tag,
                    },
                    details,
                )
                .unwrap_or_else(|error| serde_json::json!({ "ok": false, "error": error }))
            }
            "show" => {
                let id = payload.get("id").and_then(|v| v.as_str()).unwrap_or("");
                let item = records.question(id);
                let continuation = records.run(id);
                if item.is_none() && continuation.is_none() {
                    serde_json::json!({
                        "ok": false,
                        "error": format!("unknown needs-you item or continuation {id}"),
                    })
                } else {
                    serde_json::json!({
                        "ok": true,
                        "id": id,
                        "item": item,
                        "continuation": continuation,
                    })
                }
            }
            "answer" => {
                // An answer is the person's decision: only the desktop
                // surface that shows the question applies one (stint 1285).
                let id = payload.get("id").and_then(|v| v.as_str()).unwrap_or("");
                log::info!(
                    "needs_you: refused api answer id={id} caller={} outcome=approval_required",
                    origin.class()
                );
                serde_json::json!({
                    "ok": false,
                    "error_code": "approval_required",
                    "error": "Answer it in the Plexi window.",
                    "id": id,
                })
            }
            "reconcile" => {
                let session_id = payload
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty());
                let pane_id = payload.get("pane_id").and_then(|v| v.as_u64());
                let context_id = payload.get("context_id").and_then(|v| v.as_u64());
                let id_filter = payload
                    .get("id")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty());
                let mut continuations = records.runs(session_id, pane_id, context_id);
                if let Some(id) = id_filter {
                    continuations.retain(|c| {
                        c.run_id == id
                            || c.needs_you_id.as_deref() == Some(id)
                            || c.operation_id.as_deref() == Some(id)
                            || c.correlation_id.as_deref() == Some(id)
                    });
                }
                let mut items = records.questions(false);
                items.retain(|item| {
                    let details = item.details.as_ref();
                    let match_id = id_filter.is_none_or(|id| {
                        item.id == id
                            || item.run_tag.as_deref() == Some(id)
                            || details.and_then(|d| d.correlation_id.as_deref()) == Some(id)
                    });
                    let match_session = session_id.is_none_or(|sid| {
                        details.and_then(|d| d.session_id.as_deref()) == Some(sid)
                    });
                    let match_pane = pane_id.is_none_or(|pid| {
                        details.and_then(|d| d.pane_id) == Some(pid)
                            || item.actor.pane().is_some_and(|owner| owner.number() == pid)
                    });
                    let match_ctx = context_id
                        .is_none_or(|cid| details.and_then(|d| d.context_id) == Some(cid));
                    match_id && match_session && match_pane && match_ctx
                });
                log::info!(
                    "needs_you: api reconcile session={session_id:?} pane={pane_id:?} context={context_id:?} items={} continuations={}",
                    items.len(),
                    continuations.len()
                );
                serde_json::json!({
                    "ok": true,
                    "items": items,
                    "continuations": continuations,
                })
            }
            "complete" => {
                let id = payload
                    .get("id")
                    .or_else(|| payload.get("run_id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let state_str = payload
                    .get("state")
                    .and_then(|v| v.as_str())
                    .unwrap_or("completed");
                let state = match state_str {
                    "completed" => crate::broker::gate::ContinuationState::Completed,
                    "failed" => crate::broker::gate::ContinuationState::Failed,
                    "canceled" => crate::broker::gate::ContinuationState::Canceled,
                    "outcome_unknown" => crate::broker::gate::ContinuationState::OutcomeUnknown,
                    "running" => crate::broker::gate::ContinuationState::Running,
                    _ => crate::broker::gate::ContinuationState::Completed,
                };
                let receipt = payload
                    .get("receipt")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string);
                let record = records.run(id);
                if record
                    .as_ref()
                    .and_then(|run| run.needs_you_id.as_deref())
                    .and_then(|question| records.question(question))
                    .is_some_and(|row| row.is_claude_question())
                {
                    crate::rpc::write_json_response(
                        response_file,
                        serde_json::json!({"ok":false,"error_code":"permission_denied"}),
                    );
                    return;
                }
                if record
                    .as_ref()
                    .is_some_and(|r| r.request_fingerprint.is_some())
                {
                    log::info!(
                        "needs_you: refused external completion of host-owned operation {id}"
                    );
                    serde_json::json!({
                        "ok": false,
                        "error_code": "host_owned_operation",
                        "error": "pane send outcomes are recorded by the host",
                    })
                } else {
                    match record
                        .ok_or_else(|| format!("unknown continuation {id}"))
                        .and_then(|row| {
                            monitor.complete_continuation(&row.handle(), state, receipt)
                        }) {
                        Ok(continuation) => serde_json::json!({
                            "ok": true,
                            "continuation": continuation,
                        }),
                        Err(error) => serde_json::json!({
                            "ok": false,
                            "error": error,
                        }),
                    }
                }
            }
            _ => serde_json::json!({
                "ok": false,
                "error": format!("unknown needs-you api operation {op}"),
            }),
        };
        crate::rpc::write_response(response_file, body.to_string().as_bytes());
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn call_plugin_tool(
        &mut self,
        plugin_id: String,
        tool: String,
        input_json: String,
        origin: &crate::broker::caller::RequestOrigin,
        call_credential: Option<String>,
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
            match crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir())
            {
                Ok(monitor) => monitor,
                Err(error) => {
                    reply(
                        serde_json::json!({"error": error.to_string(), "error_code": "not_host"}),
                    );
                    return;
                }
            };
        let unidentified = crate::broker::caller::SocketCaller::Unidentified(
            crate::broker::caller::UnidentifiedReason::NoPeerCredential,
        );
        let socket_caller = origin.socket_caller().unwrap_or(&unidentified);
        let verified = match monitor.authenticate_call(socket_caller, call_credential.as_deref()) {
            Ok(caller) => caller,
            Err(error) => {
                let actor = socket_caller
                    .principal()
                    .as_ref()
                    .map(crate::broker::principal::RecordOwner::requester)
                    .unwrap_or_else(|| {
                        crate::broker::principal::RecordOwner::Host("unidentified".into())
                    });
                monitor.note_denial(&actor, "app-call", &plugin_id, "", "identity");
                log::info!(
                    "plugin_call: denied plugin={plugin_id} tool={tool} identity={error:?} caller={} actor={actor}",
                    origin.class()
                );
                let code = "permission_denied";
                reply(serde_json::json!({
                    "ok": false,
                    "error_code": code,
                    "error": crate::broker::gate::structured_error(code, "app-call", None),
                }));
                return;
            }
        };
        // A pane caller must be live, however it authenticated: a credential
        // carries the context it was issued in, never proof the pane still
        // exists (stint 1632).
        let live_context = match verified.pane_id {
            Some(pane_id) => match self.find_pane_in_any_window(pane_id) {
                Some((win_idx, _)) => Some(self.windows[win_idx].context_id),
                None => {
                    log::warn!("plugin_call: refused — verified pane {pane_id} is not live");
                    reply(serde_json::json!({
                        "ok": false,
                        "error_code": "permission_denied",
                        "error": format!("caller pane {pane_id} not found"),
                    }));
                    return;
                }
            },
            None => None,
        };
        let context_id = if verified.context_id != 0 {
            verified.context_id
        } else if let Some(context_id) = live_context {
            context_id
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
        let caller = verified.principal.actor_id().to_owned();
        let mut scope = crate::plexi_ai::tool_dispatch::DispatchScope::new(
            verified.pane_id.unwrap_or(0),
            verified.principal.clone(),
            workspace,
            context_id,
        );
        scope.actor_scope = verified.actor_scope;
        // A pane credential carries its pane's ceiling; a peer-verified
        // caller's comes from the pane it resolved to (stint 1632).
        let ceiling = match &verified.authority {
            Some(authority) => authority
                .grant()
                .posture()
                .map(|p| p.cloned())
                .map_err(|sealed| sealed.to_string()),
            None => verified
                .pane_id
                .map_or(Ok(None), |pid| self.pane_ceiling(pid)),
        };
        scope.posture = match ceiling {
            Ok(posture) => posture,
            Err(error) => {
                log::info!(
                    "plugin_call: caller pane {:?} refused: {error}",
                    verified.pane_id
                );
                reply(
                    serde_json::json!({"ok":false, "error_code":"permission_denied", "error": error}),
                );
                return;
            }
        };
        let owner = scope.principal.clone();
        let mut dispatcher = crate::plexi_ai::tool_dispatch::ToolDispatcher::namespaced_for(
            scope,
            std::sync::Arc::clone(&monitor),
        );
        if wait_approved {
            let Some(peer) = socket_caller.peer().and_then(|peer| peer.hops.first().copied()) else {
                reply(
                    serde_json::json!({"ok":false, "error_code":"permission_denied",
                    "error":"approval waiting requires a verified live socket peer"}),
                );
                return;
            };
            dispatcher.set_presenter(std::sync::Arc::new(WaitingApproval {
                principal: owner,
                monitor: std::sync::Arc::clone(&monitor),
                peer,
                authority: socket_caller
                    .pane()
                    .map(|pane| crate::host::plugin_timeline::AdmittedScope::of(&pane.authority)),
                timeout: crate::cli::plugin::APP_CALL_APPROVAL_TIMEOUT,
            }));
        }
        let namespaced = match dispatcher.select_plugin_tool(&plugin_id, &tool, target_pane_id) {
            Ok(name) => name,
            Err(error) => {
                monitor.note_denial(
                    &crate::broker::principal::RecordOwner::requester(&verified.principal),
                    "app-call",
                    &plugin_id,
                    "",
                    "ambiguous_instance",
                );
                log::info!("plugin_call: {error}");
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
            "plugin_call: caller={caller} context={context_id} plugin={plugin_id} tool={tool} call_id={call_id}"
        );
        let spawned = std::thread::Builder::new()
            .name("app-call".to_string())
            .spawn(move || {
                let result = dispatcher.dispatch_call(call_id.clone(), &namespaced, input_json);
                let body = match (result.error, result.output_json) {
                    (Some(error), _) => {
                        log::info!(
                            "plugin_call: call_id={call_id} rejected code={:?}: {error}",
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
                        log::info!("plugin_call: call_id={call_id} ok");
                        serde_json::json!({"ok": true, "output": output})
                    }
                    (None, None) => serde_json::json!({"ok": true, "output": null}),
                };
                reply(body);
            });
        if let Err(error) = spawned {
            log::error!("plugin_call: failed to spawn worker thread: {error}");
        }
    }

    pub(crate) fn list_plugin_tools(
        &self,
        plugin_id: Option<&str>,
        explicit_context_id: Option<u64>,
        origin: &crate::broker::caller::RequestOrigin,
        response_file: &str,
    ) {
        let context_known = |id: u64| {
            self.router.iter().any(|c| c.context_id == id)
                || self.windows.iter().any(|w| w.context_id == id)
        };
        let resolved_context_id = if let Some(ctx_id) = explicit_context_id {
            if !context_known(ctx_id) {
                log::warn!("plugin_tools: explicit context_id={ctx_id} not found");
                crate::rpc::write_json_response(
                    response_file,
                    serde_json::json!({
                        "ok": false,
                        "error": format!("context {ctx_id} not found"),
                    }),
                );
                return;
            }
            ctx_id
        } else if let Some(pane) = origin
            .socket_caller()
            .and_then(crate::broker::caller::SocketCaller::pane)
        {
            // The caller's context, never its window id (stint 1673).
            pane.context_id
        } else {
            self.windows[self.active_window].context_id
        };

        let mut tools =
            crate::plexi_ai::tool_dispatch::snapshot_tools_for_context(resolved_context_id);
        if let Some(filter_plugin) = plugin_id.map(str::trim).filter(|s| !s.is_empty()) {
            tools.retain(|entry| entry.plugin_id == filter_plugin);
        }
        log::info!(
            "pane_ipc: kind=list_plugin_tools context_id={resolved_context_id} plugin_id={plugin_id:?} count={}",
            tools.len()
        );
        crate::rpc::write_json_response(
            response_file,
            serde_json::json!({
                "ok": true,
                "context_id": resolved_context_id,
                "tools": tools,
            }),
        );
    }

    pub(crate) fn approval_actor_label(
        &self,
        owner: &crate::broker::principal::RecordOwner,
    ) -> String {
        let Some(pane) = owner.pane() else {
            return owner
                .label()
                .strip_prefix("agent:")
                .map(|name| format!("Agent {name}"))
                .unwrap_or_else(|| owner.label().to_owned());
        };
        let id = pane.number();
        if self.pane_principal(id).as_ref() != Some(pane) {
            return format!("Terminal pane {id}");
        }
        let agent_name = self
            .windows
            .iter()
            .find_map(|w| w.panes.get(&id))
            .and_then(|p| p.as_terminal())
            .and_then(|t| {
                t.agent_role
                    .clone()
                    .or_else(|| t.agent.as_ref().map(|a| a.agent.clone()))
                    .or_else(|| {
                        t.agent_harness.as_ref().and_then(|_| {
                            t.name.clone().filter(|n| !n.is_empty() && n != "terminal")
                        })
                    })
            });
        match agent_name {
            Some(name) => format!("Agent {name} (pane {id})"),
            None => format!("Terminal pane {id}"),
        }
    }

    /// The open Keychain Needs you record, if any. Reads the process-cached
    /// monitor, so a frame never opens a profile store to find nothing.
    pub(crate) fn open_keychain_recovery(&self) -> Option<crate::broker::gate::NeedsYouRecord> {
        crate::broker::gate::PermissionMonitor::loaded(&crate::config::config_dir())?
            .host_records()
            .open()
            .into_iter()
            .find(|item| item.kind == crate::broker::gate::NeedsYouKind::Keychain)
    }

    /// Surface the one recovery action for a host seal key that startup could
    /// not read without prompting. This is deliberately separate from a tool
    /// approval: it reloads or rotates the host seal and never grants a plugin
    /// capability. It stays a non-blocking banner (stint 1560): Enter and Esc
    /// reach it only while nothing owns input, so it shows its key chips only
    /// then. It is consent-guarded (stint 1498): Continue, which lets macOS
    /// prompt, counts only once armed, and the guard re-arms whenever the keys
    /// go live, so Enter typed into a pane never carries over; Not now counts
    /// at once.
    pub(crate) fn draw_keychain_recovery_dialog(&mut self, ctx: &egui::Context) {
        use crate::ui::dialog::ActionModal;
        let Some(item) = self.open_keychain_recovery() else {
            return;
        };
        let keys_live = self.host_input_owner() == crate::app::input_owner::InputOwner::Nobody;
        let now = std::time::Instant::now();
        let request = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            (&item.id, keys_live).hash(&mut hasher);
            hasher.finish()
        };
        self.keychain_recovery_guard
            .present(request, ctx.cumulative_pass_nr(), now);
        let input = self
            .keychain_recovery_guard
            .input(now, self.synthetic_input_frame);
        if !input.armed {
            ctx.request_repaint_after(self.keychain_recovery_guard.remaining(now));
        }
        let colors = self.colors;
        let actions = keychain_recovery_actions(keys_live);
        let response = ActionModal::new("keychain_recovery", "Keychain access needed", &actions)
            .banner()
            .consent_input(input)
            .show(ctx, &colors, |ui| {
                crate::ui::typography::body_strong(
                    ui,
                    "Saved permissions are unavailable.",
                    &colors,
                );
                crate::ui::typography::body(
                    ui,
                    "Continue lets macOS ask for access to Plexi’s saved permission key. If it remains unavailable, Plexi safely replaces that key and resets only permissions it can no longer verify.",
                    &colors,
                );
            });

        let approve = match response.selected {
            Some("continue") => true,
            Some("not_now") => false,
            _ => return,
        };
        let outcome = if approve {
            self.keychain_recovery_guard
                .witness(crate::broker::consent::ConsentDecision {
                    displayed: request,
                    target: &item.id,
                    choice: "approve",
                    refuses: false,
                    now,
                    synthetic: self.synthetic_input_frame,
                })
                .map_err(|refusal| refusal.to_string())
                .and_then(|evidence| {
                    self.approve_from_desktop(evidence, &item.id, "keychain recovery")
                })
        } else {
            self.refuse_from_desktop(&item.id, "keychain recovery")
        };
        if let Err(error) = outcome {
            log::info!("keychain_recovery: decision on {} not applied: {error}", item.id);
        }
    }

    /// Approve a Needs-you record with the evidence the surface showing it
    /// issued (stint 1285). Resolving a record never mints a plugin grant.
    pub(crate) fn approve_from_desktop(
        &mut self,
        evidence: crate::broker::consent::DecisionEvidence,
        id: &str,
        label: &str,
    ) -> Result<(), String> {
        let monitor =
            crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir())
                .map_err(|error| error.to_string())?;
        self.settle_desktop_receipt(monitor.resolve_needs_you_decided(evidence, id), id, true, label)
    }

    /// Refuse or dismiss a Needs-you record at the desktop. It narrows, so
    /// it needs real input but no arm delay and no evidence.
    pub(crate) fn refuse_from_desktop(&mut self, id: &str, label: &str) -> Result<(), String> {
        if self.synthetic_input_frame {
            log::info!("permission_seal: ignored synthetic {label} refusal id={id}");
            return Err("Ignored: only the person at the desktop can do this.".to_string());
        }
        let monitor =
            crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir())
                .map_err(|error| error.to_string())?;
        self.settle_desktop_receipt(monitor.dismiss_needs_you_at_desktop(id), id, false, label)
    }

    fn settle_desktop_receipt(
        &mut self,
        receipt: Result<crate::broker::gate::NeedsYouReceipt, crate::broker::gate::NeedsYouResolveError>,
        id: &str,
        approve: bool,
        label: &str,
    ) -> Result<(), String> {
        match receipt {
            Ok(receipt) => {
                if approve && receipt.run_outcome == crate::broker::gate::RunOutcome::OutcomeUnknown
                {
                    log::warn!(
                        "permission_seal: desktop {label} resolved id={} resolution={} run_outcome={} (request could not be applied)",
                        receipt.id,
                        receipt.resolution.as_str(),
                        receipt.run_outcome.as_str()
                    );
                    return Err("Approval was recorded, but the operation outcome is unknown. Check the requester before retrying.".to_string());
                } else {
                    log::info!(
                        "permission_seal: desktop {label} resolved id={} resolution={} run_outcome={}",
                        receipt.id,
                        receipt.resolution.as_str(),
                        receipt.run_outcome.as_str()
                    );
                }
                Ok(())
            }
            Err(crate::broker::gate::NeedsYouResolveError::Refused) => {
                log::error!("permission_seal: desktop {label} refused id={id}");
                Err(format!("Could not complete {label}: refused"))
            }
            Err(crate::broker::gate::NeedsYouResolveError::Failed(error)) => {
                log::error!("permission_seal: desktop {label} failed id={id}: {error}");
                Err(format!("Could not complete {label}: {error}"))
            }
        }
    }

    /// Answer a creative `Question` / `BlockedRun` Needs-you record from a desktop click.
    /// Never mints a plugin grant or widens permission scopes.
    pub(crate) fn answer_from_desktop(
        &mut self,
        evidence: crate::broker::consent::DecisionEvidence,
        id: &str,
        answer: &str,
        label: &str,
    ) -> Result<(), String> {
        let monitor =
            crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir())
                .map_err(|error| error.to_string())?;
        match monitor.answer_needs_you_decided(evidence, id, answer) {
            Ok(receipt) => {
                log::info!(
                    "permission_seal: desktop {label} answered id={} answer_bytes={} resolution={}",
                    receipt.id,
                    answer.len(),
                    receipt.resolution.as_str()
                );
                Ok(())
            }
            Err(crate::broker::gate::NeedsYouResolveError::Refused) => {
                log::error!("permission_seal: desktop {label} answer refused id={id}");
                Err(format!("Could not complete {label}: refused"))
            }
            Err(crate::broker::gate::NeedsYouResolveError::Failed(error)) => {
                log::error!("permission_seal: desktop {label} answer failed id={id}: {error}");
                Err(format!("Could not complete {label}: {error}"))
            }
        }
    }

    fn oldest_pending_id(&self) -> Option<String> {
        crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir())
            .ok()?
            .host_records()
            .pending()
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
            .find_map(|window| window.panes.get(&pane_id).and_then(|pane| pane.as_view()));
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
        origin: &crate::broker::caller::RequestOrigin,
        pane_id: u64,
        key: Option<&str>,
        click_abs: Option<egui::Pos2>,
        response_file: Option<&str>,
    ) -> bool {
        let Some(pending_id) = self.synthetic_approval_target(pane_id, key, click_abs) else {
            return false;
        };
        self.deny_synthetic_decision(origin, pane_id, Some(&pending_id), response_file);
        true
    }

    pub(super) fn deny_synthetic_decision(
        &self,
        origin: &crate::broker::caller::RequestOrigin,
        pane_id: u64,
        target: Option<&str>,
        response_file: Option<&str>,
    ) {
        let monitor = crate::broker::gate::PermissionMonitor::loaded(&crate::config::config_dir());
        let records = monitor
            .as_deref()
            .and_then(|monitor| records_for_origin(monitor, origin));
        let pending_id = records
            .as_ref()
            .and_then(|records| match target {
                Some(id) => records.pending_id(id).map(|row| row.pending_request_id),
                None => records
                    .pending()
                    .first()
                    .map(|row| row.pending_request_id.clone()),
            })
            .unwrap_or_else(|| "none".into());
        if let Some(records) = records {
            records.record_refusal(&pending_id);
        }
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
    }
}

/// The keychain recovery banner's buttons (stint 1560). The Enter and Esc
/// shortcuts, and their chips, exist only while `keys_live`: when a pane or
/// overlay owns input the keys never reach the banner, so it advertises none.
pub(crate) fn keychain_recovery_actions(
    keys_live: bool,
) -> [crate::ui::dialog::DialogAction<'static>; 2] {
    use crate::ui::button::ButtonKind;
    use crate::ui::dialog::{DialogAction, DialogShortcut};
    let continue_ = DialogAction::new("continue", "Continue", ButtonKind::Primary);
    let not_now = DialogAction::new("not_now", "Not now", ButtonKind::Secondary).refuses();
    if !keys_live {
        return [continue_, not_now];
    }
    [
        continue_.shortcut(DialogShortcut::new(
            &["Enter"],
            egui::Modifiers::NONE,
            egui::Key::Enter,
        )),
        not_now.shortcut(DialogShortcut::new(
            &["Esc"],
            egui::Modifiers::NONE,
            egui::Key::Escape,
        )),
    ]
}

#[cfg(test)]
mod tests {

    #[test]
    fn waiting_approval_timeout_and_dead_caller_cancel_only_the_pending_call() {
        use crate::broker::gate::{Admission, AdmitRequest, ApprovalChoice, PermissionMonitor};
        use crate::broker::{ActorScope, TargetType};
        use crate::plexi_ai::tool_dispatch::PermissionPresenter;
        use crate::broker::caller::PeerHop;
        use crate::platform::process::ProcStart;
        let me = PeerHop {
            pid: std::process::id(),
            start: crate::platform::process::process_info(std::process::id())
                .unwrap()
                .start,
        };
        for (peer, timeout, reason) in [
            (me, std::time::Duration::ZERO, "approval_timeout"),
            (
                PeerHop { pid: 0, start: me.start },
                std::time::Duration::from_secs(1),
                "caller_exited",
            ),
            // A live pid at another start time is a reused pid: the asker
            // is gone (stint 1285).
            (
                PeerHop { pid: me.pid, start: ProcStart::for_test(0) },
                std::time::Duration::from_secs(1),
                "caller_exited",
            ),
        ] {
            let monitor = PermissionMonitor::ephemeral();
            let workspace = tempfile::tempdir().unwrap();
            let ask = |call_id, input_json| match monitor.admit(AdmitRequest {
                call_id,
                tool: "probe.add",
                input_json,
                principal: &crate::broker::principal::Principal::agent("agent:waiter")
                    .expect("named agent"),
                actor_scope: ActorScope::User,
                trust_origin: "host",
                workspace_root: workspace.path(),
                context_id: 1,
                package_id: "probe",
                instance_id: 1,
                target_type: TargetType::PluginConnector,
            }) {
                Admission::Required { pending_request_id } => pending_request_id,
                _ => panic!("expected pending request"),
            };
            let pending = ask("waited", "{}");
            let unrelated = ask("other", r#"{"other":true}"#);
            let waiter = super::WaitingApproval {
                principal: crate::broker::principal::Principal::agent("agent:waiter").unwrap(),
                monitor: monitor.clone(),
                peer,
                authority: None,
                timeout,
            };
            let error = waiter
                .present(&pending, "probe.add", "{}", "agent:waiter", "")
                .unwrap_err();
            assert!(error.contains(reason), "{error}");
            assert_eq!(
                monitor.approval_resolution(&pending),
                Some(ApprovalChoice::Deny)
            );
            assert!(
                monitor
                    .approve_pending(&pending, ApprovalChoice::Once)
                    .is_err(),
                "late approval must not resume cancelled work"
            );
            assert!(monitor.host_records().pending_id(&unrelated).is_some());
            assert!(!monitor.approval_granted(&pending));
        }
    }

    /// Codex 3290 r1 P2: a plugin call waiting for approval is bound to the
    /// pane grant it was admitted under; closing the pane cancels it.
    #[test]
    fn waiting_approval_ends_when_the_callers_pane_is_closed() {
        use crate::broker::gate::{Admission, AdmitRequest, PermissionMonitor};
        use crate::plexi_ai::tool_dispatch::PermissionPresenter;
        let mut h = crate::testing::HostHarness::new();
        let pane = h.add_focused_terminal();
        let admitted = crate::host::plugin_timeline::AdmittedScope::of(
            h.app.terminal_incarnation(pane).unwrap().authority().unwrap(),
        );
        let principal =
            crate::broker::principal::Principal::Pane(h.app.pane_principal(pane).unwrap());
        h.app.close_pane_by_id(pane);
        let monitor = PermissionMonitor::ephemeral();
        let workspace = tempfile::tempdir().unwrap();
        let Admission::Required { pending_request_id } = monitor.admit(AdmitRequest {
            call_id: "closed",
            tool: "probe.add",
            input_json: "{}",
            principal: &principal,
            actor_scope: crate::broker::ActorScope::User,
            trust_origin: "host",
            workspace_root: workspace.path(),
            context_id: 1,
            package_id: "probe",
            instance_id: 1,
            target_type: crate::broker::TargetType::PluginConnector,
        }) else {
            panic!("expected pending request");
        };
        let me = crate::broker::caller::PeerHop {
            pid: std::process::id(),
            start: crate::platform::process::process_info(std::process::id())
                .unwrap()
                .start,
        };
        let waiter = super::WaitingApproval {
            principal,
            monitor: monitor.clone(),
            peer: me,
            authority: Some(admitted),
            timeout: std::time::Duration::from_secs(5),
        };
        let error = waiter
            .present(&pending_request_id, "probe.add", "{}", "pane", "")
            .unwrap_err();
        assert!(error.contains("caller_revoked"), "{error}");
    }
}

#[cfg(test)]
mod recommend_tests {
    use super::*;

    #[test]
    fn needs_you_ask_recommend_must_be_one_of_the_choices() {
        let choices = vec!["Ship".to_string(), "Hold".to_string()];
        assert!(validate_recommend(None, &choices).is_ok());
        assert!(validate_recommend(Some("Hold"), &choices).is_ok());
        assert!(validate_recommend(Some("Nope"), &choices).is_err());
        assert!(validate_recommend(Some("Ship"), &[]).is_err());
        let bad = serde_json::json!({"choices": ["Ship"], "recommend": "Hold"});
        assert!(ask_recommend_error(&bad).is_some());
        let good = serde_json::json!({"choices": ["Ship", "Hold"], "recommend": "Hold"});
        assert!(ask_recommend_error(&good).is_none());
    }
}
