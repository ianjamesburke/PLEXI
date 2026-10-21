//! Lifecycle methods — per-frame drain and tick operations on PlexiApp.

use egui_term::PtyEvent;
use crate::host::pane_lifecycle::{ExitStatus, PaneLifecycleEvent, Provenance, Source};

use super::PendingNotification;
use super::PlexiApp;

const MAX_SLOT_CONTENT_SIZE: usize = 10 * 1024 * 1024;

/// Quiet window after which a foreground agent TUI with no hook report yet is
/// considered idle by the host-observed detector. Measured against a real
/// Codex boot: the banner draws in bursts with gaps under ~300ms and goes
/// quiet within ~2.2s, so one full second of PTY silence cleanly separates
/// "still drawing" from "sitting at the prompt" while staying far inside the
/// 60s default boot window.
const OBSERVED_AGENT_SETTLE: std::time::Duration = std::time::Duration::from_secs(1);

use crate::rpc::{write_json_response, write_response};

fn slot_error(response_file: &str, message: impl Into<String>) {
    write_json_response(
        response_file,
        serde_json::json!({
            "ok": false,
            "error": message.into(),
        }),
    );
}

fn slot_read_error(response_file: &str, message: impl Into<String>) {
    write_json_response(
        &format!("{response_file}.err"),
        serde_json::json!({
            "ok": false,
            "error": message.into(),
        }),
    );
}

fn validate_slot_name(slot_name: &str) -> Result<(), String> {
    if slot_name.is_empty() {
        return Err("slot name cannot be empty".to_string());
    }
    if slot_name == "." || slot_name == ".." || slot_name.contains('/') || slot_name.contains('\\')
    {
        return Err(format!("invalid slot name '{slot_name}'"));
    }
    Ok(())
}

fn slot_base_dir(context_root: Option<&std::path::Path>) -> std::path::PathBuf {
    match context_root {
        Some(root) => root
            .join(crate::config::workspace_channel_dir())
            .join("slots"),
        None => crate::config::config_dir().join("slots"),
    }
}

fn pane_slots_json(pane: &crate::host::pane::Pane) -> serde_json::Value {
    match pane.slots() {
        Some(slots) => {
            let obj = slots
                .iter()
                .map(|(name, path)| {
                    (
                        name.clone(),
                        serde_json::Value::String(path.to_string_lossy().into_owned()),
                    )
                })
                .collect();
            serde_json::Value::Object(obj)
        }
        None => serde_json::Value::Object(serde_json::Map::new()),
    }
}

fn slot_list_entries(
    slots: &std::collections::HashMap<String, std::path::PathBuf>,
) -> Vec<serde_json::Value> {
    let mut entries: Vec<serde_json::Value> = slots
        .iter()
        .map(|(name, path)| {
            let metadata = std::fs::metadata(path).ok();
            let size = metadata.as_ref().map(|m| m.len()).unwrap_or(0);
            let modified = metadata
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs());
            serde_json::json!({
                "name": name,
                "path": path.to_string_lossy(),
                "size": size,
                "modified": modified,
            })
        })
        .collect();
    entries.sort_by(|a, b| {
        a.get("name")
            .and_then(|v| v.as_str())
            .cmp(&b.get("name").and_then(|v| v.as_str()))
    });
    entries
}

impl PlexiApp {
    pub(super) fn drain_pane_cmd_channel(&mut self) {
        while let Ok(cmd) = self.pane_ipc_rx.try_recv() {
            self.handle_pane_ipc_request(cmd);
        }
        self.drain_event_subscribe_channel();
    }

    /// Service CLI/MCP subscribe *and* publish requests routed from socket
    /// connection threads. The UI thread owns the grant store, so identity
    /// resolution + the broker check happen here; the connection thread streams
    /// deliveries (subscribe) or awaits one reply (publish) afterward. Grants
    /// are reloaded from disk first so a just-granted permission applies without
    /// a host restart.
    fn drain_event_subscribe_channel(&mut self) {
        self.pending_event_consents.retain(|consent| !consent.is_cancelled());
        self.observe_pane_spawns();
        // Pull all pending requests before touching the grant store so the
        // reload happens at most once per frame regardless of request count.
        let mut subscribe_reqs = Vec::new();
        while let Ok(req) = self.event_subscribe_rx.try_recv() {
            subscribe_reqs.push(req);
        }
        let mut publish_reqs = Vec::new();
        while let Ok(req) = self.event_publish_rx.try_recv() {
            publish_reqs.push(req);
        }
        if subscribe_reqs.is_empty() && publish_reqs.is_empty() {
            return;
        }
        self.host_subscriptions.reload(&crate::config::config_dir());
        // Resolve the caller's host-established origin here, at the point of
        // dispatch — never cached, never trusted verbatim from the wire
        // (stint 0724 Phase D). A request whose sender already resolved its
        // own origin (host MCP, a WASM/Python app's own subscribe path) is
        // left untouched; only the raw CLI-socket shape (`context_id_override
        // == None`) is resolved here, preferring kernel-verified
        // `peer_ancestry` over the client-forwarded `from_pane_id` claim.
        for mut req in subscribe_reqs {
            if req.context_id_override.is_none() {
                let (verified_pane_id, context_id, workspace_root) =
                    self.resolve_event_bus_caller(req.from_pane_id, req.peer_ancestry.as_deref());
                req.from_pane_id = verified_pane_id;
                req.context_id_override = context_id;
                req.workspace_root_override = workspace_root;
            }
            if req.cancelled.is_some() {
                req.broker_actor_override = Some(
                    crate::host::event_subscriptions::HostSubscriptionService::resolve_cli_subscriber(req.from_pane_id).1,
                );
            }
            // When a parent master pane subscribes to its spawned child worker's
            // `plexi.pane::lifecycle` stream across a sub-context boundary, bind the
            // subscription to the child pane's context and grant a session-scoped
            // allow so Tier 1 escalation (`Worker -> Master`) does not prompt the human.
            if req.publisher_app_id == crate::host::pane_lifecycle::PUBLISHER {
                if let (Some(caller_id), Some(target_pane_id)) = (
                    req.from_pane_id,
                    req.resource_id.as_deref().and_then(|s| s.parse::<u64>().ok()),
                ) {
                    let target_parent = self
                        .windows
                        .iter()
                        .find_map(|win| win.panes.get(&target_pane_id))
                        .and_then(|p| p.as_terminal())
                        .and_then(|t| t.parent_pane_id)
                        .or_else(|| {
                            self.host
                                .pane_lifecycle
                                .get(&target_pane_id)
                                .and_then(|s| s.parent_pane_id)
                        });
                    if target_parent == Some(caller_id) {
                        let target_ctx_id = self
                            .windows
                            .iter()
                            .find_map(|win| {
                                win.panes
                                    .contains_key(&target_pane_id)
                                    .then_some(win.context_id)
                            })
                            .or_else(|| {
                                self.host
                                    .pane_lifecycle
                                    .get(&target_pane_id)
                                    .map(|s| s.context_id)
                            });
                        if let Some(t_ctx_id) = target_ctx_id {
                            req.context_id_override = Some(t_ctx_id);
                        }
                        if let Some(ref ws) = req.workspace_root_override {
                            let actor = req.broker_actor_override.clone().unwrap_or_else(|| {
                                crate::host::event_subscriptions::HostSubscriptionService::resolve_cli_subscriber(req.from_pane_id).1
                            });
                            self.host_subscriptions
                                .allow_parent_child_lifecycle(&actor, ws);
                        }
                    }
                }
            }
            // `Allow`/`Deny`/undeclared answer the transport inline; `Ask`
            // returns a parked consent we surface as a host modal next frame.
            if let Some(consent) = self.host_subscriptions.classify_subscribe_request(req) {
                self.pending_event_consents.push_back(consent);
            }
        }
        for mut req in publish_reqs {
            if req.context_id_override.is_none() {
                let (verified_pane_id, context_id, workspace_root) =
                    self.resolve_event_bus_caller(req.from_pane_id, req.peer_ancestry.as_deref());
                req.from_pane_id = verified_pane_id;
                req.context_id_override = context_id;
                req.workspace_root_override = workspace_root;
            }
            if let Some(consent) = self.host_subscriptions.classify_publish_request(req) {
                self.pending_event_consents.push_back(consent);
            }
        }
    }

    /// Point focus at the window and tile that own `from_pane_id` so a
    /// spawn lands beside its caller, returning `(target_window,
    /// focused_pane_before_redirect)`. The caller restores that saved pane
    /// through `restore_window_focused_pane` once the launch settles.
    /// `kind` only names the caller in the log line. No `from_pane_id`, or
    /// one that names no live pane, leaves focus alone and reports
    /// `fallback_win`.
    fn redirect_focus_to_spawn_origin(
        &mut self,
        from_pane_id: Option<crate::spatial::tiling::PaneId>,
        kind: &str,
        fallback_win: usize,
    ) -> (usize, Option<egui_tiles::TileId>) {
        let Some(from_id) = from_pane_id else {
            return (fallback_win, self.windows[fallback_win].focused_pane);
        };
        match self.find_pane_in_any_window(from_id) {
            Some((fw, ft)) => {
                log::info!(
                    "pane_ipc: spawn_pane {kind}: targeting from_pane_id={from_id} win_idx={fw}"
                );
                let saved = self.windows[fw].focused_pane;
                self.active_window = fw;
                self.set_window_focused_pane(fw, ft);
                (fw, saved)
            }
            None => {
                log::warn!(
                    "pane_ipc: spawn_pane {kind}: from_pane_id={from_id} not found, using focused pane"
                );
                (fallback_win, self.windows[fallback_win].focused_pane)
            }
        }
    }

    /// Handle a single pane-IPC `AppRequest`. Shared by the socket drain above
    /// and the PGAP forwarding path (`AppCommand::ForwardPaneRequest`, stint
    /// 0013/0014) so capability-gated app requests take the identical host
    /// code path as CLI requests arriving over PLEXI_SOCKET.
    pub(crate) fn handle_pane_ipc_request(&mut self, cmd: crate::protocol::AppRequest) {
        match &cmd {
            crate::protocol::AppRequest::SubmitAssistantTurn { text, request_id, response_file, pane_id, context_id, conversation_id, join_desktop, status_for, client, kind, head } => {
                if let Some(head) = head {
                    let workspace = context_id
                        .and_then(|id| self.context_root_for(id))
                        .or_else(crate::config::active_workspace_root)
                        .unwrap_or_else(|| self.router.active().root.clone());
                    log::info!("pane_ipc: kind=submit_assistant_turn head={head}");
                    let accepted = crate::agent::leads::submit_turn(&workspace, head, text, request_id, response_file, None);
                    if accepted.get("ok").and_then(|v| v.as_bool()) != Some(true) {
                        write_json_response(response_file, serde_json::json!({
                            "request_id": request_id,
                            "state": "failed",
                            "error": accepted.get("error").cloned().unwrap_or(serde_json::json!("lead turn was not accepted")),
                            "error_code": accepted.get("error_code").cloned().unwrap_or(serde_json::json!("failed")),
                        }));
                    }
                } else {
                if pane_id.is_none() {
                    let _ = self.ensure_headless_assistant(*context_id);
                }
                let mut submitted = false;
                let mut failure = None;
                for window in &mut self.windows {
                    if context_id.is_some_and(|id| window.context_id != id) { continue; }
                    for (id, pane) in &mut window.panes {
                        if pane_id.is_some_and(|wanted| wanted != *id) { continue; }
                        let Some(app) = pane.as_app_mut() else { continue; };
                        if app.runtime.type_id() != "assistant" { continue; }
                        submitted = true;
                        if let Err(error) = app.runtime.submit_external_turn(text.clone(), request_id.clone(), response_file.clone(), crate::app::app_trait::ExternalTurnOpts { conversation_id: conversation_id.clone(), join_desktop: *join_desktop, status_for: status_for.clone(), client: client.clone(), kind: kind.clone() }) { failure = Some(error); }
                        break;
                    }
                    if submitted { break; }
                }
                if let Some(error) = failure { write_json_response(response_file, serde_json::json!({"request_id": request_id, "state":"failed", "error":error})); }
                else if !submitted { write_json_response(response_file, serde_json::json!({"request_id": request_id, "state":"failed", "error":"assistant_pane_not_found"})); }
                }
            }
            crate::protocol::AppRequest::OpenAssistantHead { head, context_id, response_file } => {
                log::info!("pane_ipc: kind=open_assistant_head head={head}");
                match self.open_assistant_for_head(head, *context_id) {
                    Ok(pane_id) => write_json_response(response_file, serde_json::json!({"ok": true, "pane_id": pane_id, "head": head})),
                    Err(error) => write_json_response(response_file, serde_json::json!({"ok": false, "error": error})),
                };
            }
            crate::protocol::AppRequest::CommandView { op, payload, response_file } => {
                log::info!("pane_ipc: kind=command_view op={op}");
                let workspace = payload.get("workspace").and_then(|v| v.as_str()).map(std::path::PathBuf::from)
                    .or_else(crate::config::active_workspace_root)
                    .unwrap_or_else(|| self.router.active().root.clone());
                let mut defer_reply = false;
                let body = match op.as_str() {
                    "open" => match self.open_command_view(&workspace) {
                        Ok(pane_id) => serde_json::json!({"ok": true, "pane_id": pane_id}),
                        Err(error) => serde_json::json!({"ok": false, "error": error}),
                    },
                    "send" => {
                        let lead = payload.get("lead").and_then(|value| value.as_str()).unwrap_or("");
                        let text = payload.get("text").and_then(|value| value.as_str()).unwrap_or("");
                        let request_id = payload.get("request_id").and_then(|value| value.as_str()).unwrap_or("command-view");
                        log::info!("command_view: send lead={lead} request_id={request_id}");
                        let accepted = crate::agent::leads::submit_turn(&workspace, lead, text, request_id, response_file, None);
                        if accepted.get("ok").and_then(|value| value.as_bool()) == Some(true) {
                            defer_reply = true;
                        }
                        accepted
                    }
                    "cancel" => {
                        let run = payload.get("run").and_then(|value| value.as_str()).unwrap_or("");
                        log::info!("command_view: cancel run={run}");
                        crate::agent::queue::request_cancel_run(&workspace, run)
                    }
                    _ => crate::agent::leads::projection(&workspace),
                };
                if !defer_reply {
                    write_json_response(response_file, body);
                }
            }
            crate::protocol::AppRequest::AssistantHostTool { name, input_json, response_file } => {
                log::info!("assistant_host_tool: ipc name={name}");
                let result = self.dispatch_assistant_host_tool(name, input_json);
                let body = if let Some(error) = result.error.as_deref() {
                    serde_json::json!({
                        "ok": false,
                        "error": error,
                        "error_code": result.error_code,
                        "pending_request_id": result.pending_request_id,
                    })
                } else {
                    let output = result
                        .output_json
                        .as_deref()
                        .and_then(|text| serde_json::from_str::<serde_json::Value>(text).ok())
                        .unwrap_or(serde_json::Value::Null);
                    serde_json::json!({"ok": true, "output": output})
                };
                write_json_response(response_file, body);
            }
            crate::protocol::AppRequest::ResourceLease { op, name, max, holder_pid, peer_ancestry, response_file } => {
                if !peer_ancestry.contains(holder_pid) || *holder_pid <= 1 {
                    write_json_response(response_file, serde_json::json!({"error": "holder must be a verified caller ancestor", "error_code": "permission_denied"}));
                } else {
                    if self.resource_leases.is_none() {
                        match crate::host::resource_leases::LeaseService::start() {
                            Ok(service) => self.resource_leases = Some(service),
                            Err(error) => {
                                log::error!("lease: service start failed: {error}");
                                write_json_response(response_file, serde_json::json!({"error": error.to_string(), "error_code": "lease_unavailable"}));
                            }
                        }
                    }
                    if let Some(service) = &self.resource_leases {
                        service.submit(crate::host::resource_leases::LeaseRequest {
                            op: op.clone(), name: name.clone(), max: *max,
                            holder_pid: *holder_pid, response_file: response_file.clone(),
                        });
                    }
                }
            }
            crate::protocol::AppRequest::AgentQueue { op, payload, response_file } => {
                log::info!("pane_ipc: kind=agent_queue op={op}");
                let workspace = payload
                    .get("workspace")
                    .and_then(|value| value.as_str())
                    .map(std::path::PathBuf::from)
                    .or_else(crate::config::active_workspace_root)
                    .unwrap_or_else(|| self.router.active().root.clone());
                let body = crate::agent::queue::handle(&workspace, op, payload);
                write_json_response(response_file, body);
            }
            crate::protocol::AppRequest::ListPermissionRequests { response_file } => {
                self.observe_permissions("list", None, None, response_file);
            }
            crate::protocol::AppRequest::ShowPermissionRequest { pending_request_id, response_file } => {
                self.observe_permissions("show", Some(pending_request_id), None, response_file);
            }
            crate::protocol::AppRequest::ResolvePermissionRequest { pending_request_id, choice, response_file } => {
                self.observe_permissions("resolve", Some(pending_request_id), Some(choice), response_file);
            }
            crate::protocol::AppRequest::Permissions {
                op,
                id,
                pane_id,
                credential,
                response_file,
            } => {
                self.permissions_cli_request(
                    op,
                    id.as_deref(),
                    *pane_id,
                    credential.as_deref(),
                    response_file,
                );
            }
            crate::protocol::AppRequest::ListNeedsYou { response_file } => {
                self.observe_needs_you("list", None, None, response_file);
            }
            crate::protocol::AppRequest::ResolveNeedsYou { id, approve, response_file } => {
                self.observe_needs_you("resolve", Some(id), Some(*approve), response_file);
            }
            crate::protocol::AppRequest::NeedsYouApi {
                op,
                payload,
                peer_ancestry,
                response_file,
            } => {
                self.handle_needs_you_api(op, payload, peer_ancestry, response_file);
            }
            crate::protocol::AppRequest::PaneSecretGet {
                name,
                global,
                workspace_root,
                peer_ancestry,
                response_file,
            } => {
                self.serve_secret_get(
                    name,
                    *global,
                    workspace_root.as_deref(),
                    peer_ancestry,
                    response_file,
                );
            }
            crate::protocol::AppRequest::AgentsApi { op, payload, response_file } => {
                log::info!("pane_ipc: kind=agents_api op={op}");
                if op == "team_run" {
                    if let Err(error) = self.start_team(payload, response_file) {
                        log::error!("agent_team: start failed: {error}");
                        crate::rpc::write_json_response(response_file, serde_json::json!({"ok":false,"error":error}));
                    }
                    return;
                }
                let mut payload = payload.clone();
                if op == "create_head" {
                    if let Some(pane_id) = payload.get("caller_pane").and_then(|value| value.as_u64()) {
                        let head = self.bound_head_for_pane(pane_id).unwrap_or_default();
                        log::info!(
                            "agents_api: create_head caller_pane={pane_id} caller_head={head}"
                        );
                        if let Some(obj) = payload.as_object_mut() {
                            obj.insert(
                                "caller_head".to_string(),
                                serde_json::json!(head),
                            );
                        }
                    }
                }
                let mut value = crate::agent::heads::handle_request(op, &payload);
                if let Some(port) = crate::app::host_mcp::bound_port() {
                    if let Some(obj) = value.as_object_mut() {
                        obj.insert("mcp_port".to_string(), serde_json::json!(port));
                    }
                }
                crate::rpc::write_json_response(response_file, value);
            }
            crate::protocol::AppRequest::SetPaneTitle { pane_id, name } => {
                log::info!("pane_ipc: kind=set_pane_title pane_id={pane_id}");
                let mut found = false;
                for win in &mut self.windows {
                    if let Some(pane) = win.panes.get_mut(pane_id) {
                        if let Some(t) = pane.as_terminal_mut() {
                            t.name_locked = !name.is_empty();
                            t.name = if name.is_empty() {
                                None
                            } else {
                                Some(name.clone())
                            };
                            found = true;
                            break;
                        } else if let Some(a) = pane.as_app_mut() {
                            a.runtime.on_pane_renamed(name);
                            a.name = if name.is_empty() {
                                a.runtime.display_name()
                            } else {
                                name.clone()
                            };
                            log::info!(
                                "pane_ipc: set_pane_title: app pane {pane_id} named {:?}",
                                a.name
                            );
                            found = true;
                            break;
                        }
                    }
                }
                if found {
                    let context_root = self
                        .origin_for_pane(*pane_id)
                        .map(|origin| origin.context_root);
                    crate::host::event_log::emit_scoped(
                        crate::host::event_log::HostEvent::PaneRenamed {
                            pane_id: *pane_id,
                            name: name.clone(),
                            timestamp: crate::host::event_log::now_timestamp(),
                        },
                        context_root.as_deref(),
                    );
                }
                if !found {
                    log::warn!("pane_ipc: set_pane_title: pane_id={pane_id} not found");
                }
            }
            crate::protocol::AppRequest::LogMarker {
                source,
                message,
                response_file,
            } => {
                // The host process owns the channel logger, so this is the
                // one place an external driver's marker can reach
                // `~/.plexi-<channel>/plexi.log`.
                let flattened = message.replace(['\n', '\r'], " ");
                log::info!("marker[{source}]: {flattened}");
                if let Some(response_file) = response_file {
                    write_json_response(response_file, serde_json::json!({"ok": true}));
                }
            }
            crate::protocol::AppRequest::GetBuildInfo { response_file } => {
                log::info!("host: reporting running build identity {}", env!("PLEXI_BUILD_ID"));
                let mut value = crate::distribution::build_info();
                value["pid"] = serde_json::json!(std::process::id());
                write_json_response(response_file, value);
            }
            crate::protocol::AppRequest::ListPanes {
                response_file,
                context_id: filter_context_id,
            } => {
                log::info!(
                    "pane_ipc: kind=list_panes context_id={:?} response_file={:?}",
                    filter_context_id,
                    response_file
                );
                let active_win = self.active_window;
                let mut entries: Vec<serde_json::Value> = Vec::new();
                for (win_idx, win) in self.windows.iter().enumerate() {
                    if let Some(cid) = filter_context_id {
                        if win.context_id != *cid {
                            continue;
                        }
                    }
                    let focused_pane_id = win
                        .focused_pane
                        .and_then(|t| win.tree.tiles.get(t))
                        .and_then(|tile| {
                            if let egui_tiles::Tile::Pane(id) = tile {
                                Some(*id)
                            } else {
                                None
                            }
                        });
                    let context_name = self
                        .router
                        .iter()
                        .find(|ctx| ctx.context_id == win.context_id)
                        .map(|ctx| ctx.name.to_string())
                        .unwrap_or_default();
                    for (pane_id, pane) in &win.panes {
                        // Only emit panes that have a corresponding tile in the tree.
                        // win.panes and the tile tree can desync (e.g. from corrupted
                        // restore state); omitting orphaned entries ensures every id
                        // returned here is navigable via pane_focus. (#996)
                        if win.tree.tiles.find_pane(pane_id).is_none() {
                            log::warn!(
                                "pane_list: pane_id={pane_id} in win.panes but absent \
                                 from tile tree — skipping (desync)"
                            );
                            continue;
                        }
                        let (pane_type, title, cwd) = match pane {
                            crate::host::pane::Pane::Terminal(t) => {
                                let name = t.name.clone().unwrap_or_else(|| "terminal".to_string());
                                let cwd = t
                                    .standby_cwd
                                    .as_ref()
                                    .map(|p| p.to_string_lossy().into_owned())
                                    .or_else(|| {
                                        crate::host::shell::get_pid_cwd(t.backend.child_pid())
                                            .map(|p| p.to_string_lossy().into_owned())
                                    })
                                    .or_else(|| {
                                        self.pane_worktrees
                                            .get(pane_id)
                                            .map(|wt| wt.path.to_string_lossy().into_owned())
                                    });
                                ("terminal", name, cwd)
                            }
                            crate::host::pane::Pane::App(a) => {
                                let cwd = Some(a.workspace_root.to_string_lossy().into_owned());
                                ("app", a.name.clone(), cwd)
                            }
                            crate::host::pane::Pane::Portal(p) => {
                                ("portal", format!("portal:{}", p.target_context_id), None)
                            }
                        };
                        let focused = win_idx == active_win && focused_pane_id == Some(*pane_id);
                        let mut entry = serde_json::json!({
                            "id": pane_id,
                            "type": pane_type,
                            "title": title,
                            "focused": focused,
                            "context_id": win.context_id,
                            "context_name": context_name,
                            "window_id": win.window_id,
                            "cwd": cwd,
                            "agent": pane.agent(),
                            "slots": pane_slots_json(pane),
                            "heartbeat": self.pane_heartbeat_json(*pane_id),
                        });
                        if let crate::host::pane::Pane::Terminal(t) = pane {
                            entry["parent_pane_id"] = serde_json::json!(t.parent_pane_id);
                            entry["agent_harness"] = serde_json::json!(t.agent_harness);
                            entry["agent_session_id"] = serde_json::json!(t.agent_session_id);
                            entry["agent_role"] = serde_json::json!(t.agent_role);
                            entry["resume_command"] = serde_json::json!(t.resume_command());
                            entry["standby"] = serde_json::json!(t.is_standby());
                            entry["standby_banner"] = serde_json::json!(t.standby_banner());
                            entry["last_resumed_command"] =
                                serde_json::json!(t.last_resumed_command);
                            entry["agent_completed"] = serde_json::json!(t.agent_completed);
                            entry["agent_session"] =
                                serde_json::json!(t.build_saved_agent_session());
                            entry["exited"] = serde_json::json!(t.exited);
                            entry["exit_status"] = serde_json::json!(t.exit_status);
                        }
                        if let Some(wt) = self.pane_worktrees.get(pane_id) {
                            entry["worktree"] = serde_json::json!({
                                "branch": wt.branch,
                                "path": wt.path,
                                "repo_root": wt.repo_root,
                            });
                        }
                        entries.push(entry);
                    }
                }
                let json_str = serde_json::to_string(&entries).unwrap_or_else(|_| "[]".to_string());
                write_response(response_file, json_str.as_bytes());
            }
            crate::protocol::AppRequest::ListContexts { response_file } => {
                log::info!(
                    "pane_ipc: kind=list_contexts response_file={:?}",
                    response_file
                );
                let active_ctx_id = self.router.active().context_id;
                let entries: Vec<serde_json::Value> = self
                    .router
                    .iter()
                    .map(|ctx| {
                        serde_json::json!({
                            "context_id": ctx.context_id,
                            "name": ctx.name,
                            "root": ctx.root.to_string_lossy(),
                            "description": ctx.description,
                            "parent_id": ctx.parent_id,
                            "depth": ctx.depth,
                            "is_active": ctx.context_id == active_ctx_id,
                        })
                    })
                    .collect();
                let json_str = serde_json::to_string(&entries).unwrap_or_else(|_| "[]".to_string());
                write_response(response_file, json_str.as_bytes());
            }
            crate::protocol::AppRequest::GetPaneInfo {
                pane_id,
                response_file,
            } => {
                log::info!(
                    "pane_ipc: kind=get_pane_info pane_id={pane_id} response_file={:?}",
                    response_file
                );
                let active_win = self.active_window;
                let mut found = false;
                'outer: for (win_idx, win) in self.windows.iter().enumerate() {
                    let focused_pane_id = win
                        .focused_pane
                        .and_then(|t| win.tree.tiles.get(t))
                        .and_then(|tile| {
                            if let egui_tiles::Tile::Pane(id) = tile {
                                Some(*id)
                            } else {
                                None
                            }
                        });
                    if let Some(pane) = win.panes.get(pane_id) {
                        let focused = win_idx == active_win && focused_pane_id == Some(*pane_id);
                        let agent = pane.agent();
                        let info = match pane {
                            crate::host::pane::Pane::Terminal(t) => {
                                let cwd = t
                                    .standby_cwd
                                    .as_ref()
                                    .map(|p| p.to_string_lossy().into_owned())
                                    .or_else(|| {
                                        crate::host::shell::get_pid_cwd(t.backend.child_pid())
                                            .map(|p| p.to_string_lossy().into_owned())
                                    })
                                    .or_else(|| {
                                        self.pane_worktrees
                                            .get(pane_id)
                                            .map(|wt| wt.path.to_string_lossy().into_owned())
                                    });
                                let mut obj = serde_json::json!({
                                    "id": pane_id,
                                    "type": "terminal",
                                    "title": t.name.clone().unwrap_or_else(|| "terminal".to_string()),
                                    "focused": focused,
                                    "context_id": win.context_id,
                                    "context_name": self.context_name_for(win.context_id),
                                    "context_description": self.context_description_for(win.context_id),
                                    "window_id": win.window_id,
                                    "cwd": cwd,
                                    "agent": agent,
                                    "parent_pane_id": t.parent_pane_id,
                                    "agent_harness": t.agent_harness,
                                    "agent_session_id": t.agent_session_id,
                                    "agent_role": t.agent_role,
                                    "resume_command": t.resume_command(),
                                    "standby": t.is_standby(),
                                    "standby_banner": t.standby_banner(),
                                    "last_resumed_command": t.last_resumed_command,
                                    "agent_completed": t.agent_completed,
                                    "agent_session": t.build_saved_agent_session(),
                                    "exited": t.exited,
                                    "exit_status": t.exit_status,
                                    "slots": pane_slots_json(pane),
                                    "heartbeat": self.pane_heartbeat_json(*pane_id),
                                });
                                if let Some(wt) = self.pane_worktrees.get(pane_id) {
                                    obj["worktree"] = serde_json::json!({
                                        "branch": wt.branch,
                                        "path": wt.path,
                                        "repo_root": wt.repo_root,
                                    });
                                }
                                if let Some(schema) = self
                                    .pane_results
                                    .get(pane_id)
                                    .and_then(|s| s.schema.as_ref())
                                {
                                    obj["result_schema"] = schema.clone();
                                }
                                obj
                            }
                            crate::host::pane::Pane::App(a) => {
                                serde_json::json!({
                                    "id": pane_id,
                                    "type": "app",
                                    "title": a.name.clone(),
                                    "focused": focused,
                                    "context_id": win.context_id,
                                    "context_name": self.context_name_for(win.context_id),
                                    "context_description": self.context_description_for(win.context_id),
                                    "window_id": win.window_id,
                                    "cwd": a.workspace_root.to_string_lossy().as_ref(),
                                    "manifest_id": a.manifest_id.clone(),
                                    "agent": agent,
                                    "slots": pane_slots_json(pane),
                                    "heartbeat": self.pane_heartbeat_json(*pane_id),
                                })
                            }
                            crate::host::pane::Pane::Portal(p) => {
                                serde_json::json!({
                                    "id": pane_id,
                                    "type": "portal",
                                    "context_id": p.target_context_id,
                                    "focused": focused,
                                })
                            }
                        };
                        let json_str =
                            serde_json::to_string(&info).unwrap_or_else(|_| "{}".to_string());
                        write_response(response_file, json_str.as_bytes());
                        found = true;
                        break 'outer;
                    }
                }
                if !found {
                    log::warn!("pane_ipc: get_pane_info: pane_id={pane_id} not found");
                    let json_str = format!("{{\"error\":\"pane {pane_id} not found\"}}");
                    write_response(response_file, json_str.as_bytes());
                }
            }
            crate::protocol::AppRequest::GetPreviousPaneInfo {
                response_file,
                steps,
            } => {
                let steps = (*steps).max(1);
                log::info!(
                    "pane_ipc: kind=get_previous_pane_info steps={steps} response_file={:?}",
                    response_file
                );
                let mut hits: u64 = 0;
                let mut result_json: Option<String> = None;
                'prev_outer: for (window_id, tile_id) in self.pane_focus_history.iter().rev() {
                    if let Some(win) = self.windows.iter().find(|w| w.window_id == *window_id) {
                        if let Some(egui_tiles::Tile::Pane(pane_id)) = win.tree.tiles.get(*tile_id)
                        {
                            if let Some(pane) = win.panes.get(pane_id) {
                                hits += 1;
                                if hits < steps {
                                    continue 'prev_outer;
                                }
                                let info = match pane {
                                    crate::host::pane::Pane::Terminal(t) => {
                                        let cwd =
                                            crate::host::shell::get_pid_cwd(t.backend.child_pid())
                                                .map(|p| p.to_string_lossy().into_owned());
                                        serde_json::json!({
                                            "id": pane_id,
                                            "type": "terminal",
                                            "title": t.name.clone().unwrap_or_else(|| "terminal".to_string()),
                                            "focused": false,
                                            "context_id": win.context_id,
                                            "window_id": win.window_id,
                                            "cwd": cwd,
                                            "slots": pane_slots_json(pane),
                                        })
                                    }
                                    crate::host::pane::Pane::App(a) => {
                                        serde_json::json!({
                                            "id": pane_id,
                                            "type": "app",
                                            "title": a.name.clone(),
                                            "focused": false,
                                            "context_id": win.context_id,
                                            "window_id": win.window_id,
                                            "cwd": a.workspace_root.to_string_lossy().as_ref(),
                                            "manifest_id": a.manifest_id.clone(),
                                            "slots": pane_slots_json(pane),
                                        })
                                    }
                                    crate::host::pane::Pane::Portal(p) => {
                                        serde_json::json!({
                                            "id": pane_id,
                                            "type": "portal",
                                            "context_id": p.target_context_id,
                                            "focused": false,
                                        })
                                    }
                                };
                                result_json = Some(
                                    serde_json::to_string(&info)
                                        .unwrap_or_else(|_| "{}".to_string()),
                                );
                                break 'prev_outer;
                            }
                        }
                    }
                }
                let json_str = match result_json {
                    Some(j) => j,
                    None => {
                        log::warn!(
                            "pane_ipc: get_previous_pane_info: fewer than {steps} valid panes in history (found {hits})"
                        );
                        format!(
                            "{{\"error\":\"not enough pane history (requested step {steps}, found {hits} valid entries)\"}}"
                        )
                    }
                };
                write_response(response_file, json_str.as_bytes());
            }
            crate::protocol::AppRequest::SlotWrite {
                pane_id,
                slot_name,
                content,
                append,
                replace,
                response_file,
            } => {
                log::info!(
                    "pane_ipc: kind=slot_write pane_id={pane_id} slot={slot_name:?} bytes={} append={append} replace={replace}",
                    content.len()
                );
                match self.write_pane_slot_internal(*pane_id, slot_name, content, *append, *replace) {
                    Ok((absolute, size)) => {
                        write_json_response(
                            response_file,
                            serde_json::json!({
                                "ok": true,
                                "name": slot_name,
                                "path": absolute.to_string_lossy(),
                                "size": size,
                            }),
                        );
                    }
                    Err(msg) => slot_error(response_file, msg),
                }
            }
            crate::protocol::AppRequest::SlotWait {
                pane_id,
                slot_name,
                pattern,
                timeout_secs,
                response_file,
            } => {
                log::info!(
                    "pane_ipc: kind=slot_wait pane_id={pane_id} slot={slot_name:?} pattern={pattern:?} timeout_secs={timeout_secs}"
                );
                if let Err(msg) = validate_slot_name(slot_name) {
                    slot_read_error(response_file, msg);
                    return;
                }
                // Bounded above MAX_WAIT_TIMEOUT_SECS: Duration::from_secs_f64
                // panics on finite values beyond Duration's range, and the
                // host must never trust the client to have pre-validated.
                if !timeout_secs.is_finite()
                    || *timeout_secs < 0.0
                    || *timeout_secs > crate::app::pane_wait::MAX_WAIT_TIMEOUT_SECS
                {
                    slot_read_error(
                        response_file,
                        format!(
                            "--timeout must be a non-negative number of seconds at most {}, got {timeout_secs}",
                            crate::app::pane_wait::MAX_WAIT_TIMEOUT_SECS
                        ),
                    );
                    return;
                }
                let regex = match regex::Regex::new(pattern) {
                    Ok(regex) => regex,
                    Err(e) => {
                        slot_read_error(
                            response_file,
                            format!("invalid --until pattern {pattern:?}: {e}"),
                        );
                        return;
                    }
                };
                // Level-triggered: a value that already matches answers now,
                // even if an ephemeral worker pane already exited 0 and closed.
                if let Some(path) = self.tracked_slot_path(*pane_id, slot_name) {
                    match std::fs::read(&path) {
                        Ok(bytes) => {
                            if regex.is_match(&String::from_utf8_lossy(&bytes)) {
                                log::info!(
                                    "pane_slot_wait: pane_id={pane_id} slot={slot_name:?} pattern={pattern:?} already matched {} bytes",
                                    bytes.len()
                                );
                                write_response(response_file, &bytes);
                                return;
                            }
                        }
                        Err(e) => {
                            slot_read_error(
                                response_file,
                                format!("could not read slot '{slot_name}': {e}"),
                            );
                            return;
                        }
                    }
                }
                let Some(pane) = self.windows.iter().find_map(|win| win.panes.get(pane_id)) else {
                    slot_read_error(response_file, format!("pane {pane_id} not found"));
                    return;
                };
                if pane.slots().is_none() {
                    slot_read_error(
                        response_file,
                        format!("pane {pane_id} does not support slots"),
                    );
                    return;
                }
                let expires_at =
                    std::time::Instant::now() + std::time::Duration::from_secs_f64(*timeout_secs);
                log::info!(
                    "pane_slot_wait: pane_id={pane_id} slot={slot_name:?} pattern={pattern:?} parked for {timeout_secs}s"
                );
                self.pending_slot_waits
                    .push(crate::app::pane_wait::PendingSlotWait {
                        pane_id: *pane_id,
                        slot_name: slot_name.clone(),
                        pattern: regex,
                        response_file: response_file.clone(),
                        expires_at,
                    });
            }
            crate::protocol::AppRequest::PaneMessageSend {
                pane_id,
                payload,
                submit,
                peer_ancestry,
                response_file,
            } => {
                let from_pane_id = peer_ancestry
                    .as_deref()
                    .and_then(|ancestry| self.resolve_socket_peer_pane(ancestry))
                    .map(|(sender_id, _, _)| sender_id);
                log::info!(
                    "pane_ipc: kind=pane_message_send pane_id={pane_id} from_pane_id={from_pane_id:?} submit={submit}"
                );
                let Some((win_idx, _)) = self.find_pane_in_any_window(*pane_id) else {
                    crate::rpc::write_json_response(
                        response_file,
                        serde_json::json!({
                            "ok": false,
                            "error": format!("pane {pane_id} not found"),
                        }),
                    );
                    return;
                };
                let (message_id, delivered_to_waiter) = self.enqueue_pane_message(*pane_id, from_pane_id, payload.clone());
                let mut woke = false;
                if *submit && !delivered_to_waiter {
                    let can_wake =
                        self.agent_reports_idle(*pane_id) && !self.submit_in_flight(*pane_id);
                    if can_wake {
                        if let Some(term) = self.windows[win_idx]
                            .panes
                            .get_mut(pane_id)
                            .and_then(crate::host::pane::Pane::as_terminal_mut)
                        {
                            let sender_label = from_pane_id
                                .map(|id| id.to_string())
                                .unwrap_or_else(|| "external".to_string());
                            let notice = format!(
                                "[plexi message from pane {sender_label}] run `plexi pane message recv`"
                            );
                            term.backend.process_command(
                                egui_term::BackendCommand::Write(notice.clone().into_bytes()),
                            );
                            self.register_pending_submit(*pane_id, None, notice.chars().count());
                            woke = true;
                            log::info!(
                                "pane_message_send: pane_id={pane_id} from_pane_id={from_pane_id:?} id={message_id} woke idle agent"
                            );
                        }
                    } else {
                        log::info!(
                            "pane_message_send: pane_id={pane_id} from_pane_id={from_pane_id:?} id={message_id} queued without wake (busy or non-terminal)"
                        );
                    }
                }

                let queue_len = self.pane_mailboxes.get(pane_id).map_or(0, |q| q.len());
                let status = if delivered_to_waiter || woke {
                    "delivered"
                } else {
                    "queued"
                };
                crate::rpc::write_json_response(
                    response_file,
                    serde_json::json!({
                        "ok": true,
                        "status": status,
                        "state": status,
                        "pane_id": pane_id,
                        "from_pane_id": from_pane_id,
                        "sender_pane_id": from_pane_id,
                        "message_id": message_id,
                        "queue_len": queue_len,
                        "woke": woke,
                    }),
                );
            }
            crate::protocol::AppRequest::PaneMessageRecv {
                pane_id,
                wait,
                timeout_secs,
                response_file,
            } => {
                log::info!(
                    "pane_ipc: kind=pane_message_recv pane_id={pane_id} wait={wait} timeout={timeout_secs}"
                );
                if *wait
                    && (!timeout_secs.is_finite()
                        || *timeout_secs < 0.0
                        || *timeout_secs > crate::app::pane_wait::MAX_WAIT_TIMEOUT_SECS)
                {
                    slot_read_error(
                        response_file,
                        format!(
                            "--timeout must be a non-negative number of seconds at most {}, got {timeout_secs}",
                            crate::app::pane_wait::MAX_WAIT_TIMEOUT_SECS
                        ),
                    );
                    return;
                }
                if self.find_pane_in_any_window(*pane_id).is_none() {
                    slot_read_error(response_file, format!("pane {pane_id} not found"));
                    return;
                }
                let has_messages = self
                    .pane_mailboxes
                    .get(pane_id)
                    .is_some_and(|q| !q.is_empty());
                if !*wait || has_messages {
                    let drained = self.drain_pane_messages(*pane_id);
                    let mut bytes = serde_json::to_vec(&drained).unwrap_or_else(|_| b"[]".to_vec());
                    bytes.push(b'\n');
                    log::info!(
                        "pane_message_recv: pane_id={pane_id} satisfied immediately count={}",
                        drained.len()
                    );
                    write_response(response_file, &bytes);
                    return;
                }
                let expires_at =
                    std::time::Instant::now() + std::time::Duration::from_secs_f64(*timeout_secs);
                log::info!("pane_message_recv: pane_id={pane_id} parked for {timeout_secs}s");
                self.pending_message_recvs
                    .push(crate::app::pane_wait::PendingMessageRecv {
                        pane_id: *pane_id,
                        response_file: response_file.clone(),
                        expires_at,
                    });
            }
            crate::protocol::AppRequest::PaneResultSchemaSet {
                pane_id,
                schema,
                response_file,
            } => {
                log::info!("pane_ipc: kind=pane_result_schema_set pane_id={pane_id}");
                let win_info = self.find_pane_in_any_window(*pane_id);
                if win_info.is_none() && !self.pane_results.contains_key(pane_id) {
                    log::warn!("pane_result: schema set rejected — pane {pane_id} not found");
                    write_json_response(
                        response_file,
                        serde_json::json!({
                            "ok": false,
                            "error_code": "pane_not_found",
                            "error": format!("pane {pane_id} not found"),
                        }),
                    );
                    return;
                }
                let base_dir = win_info
                    .and_then(|(win_idx, _)| self.windows.get(win_idx))
                    .map(|w| w.path.clone());
                match self
                    .pane_results
                    .entry(*pane_id)
                    .or_default()
                    .set_schema(schema.clone(), base_dir.as_deref())
                {
                    Ok(normalized) => {
                        log::info!("pane_result: schema set for pane_id={pane_id}");
                        write_json_response(
                            response_file,
                            serde_json::json!({
                                "ok": true,
                                "pane_id": pane_id,
                                "schema": normalized,
                            }),
                        );
                    }
                    Err(msg) => {
                        log::warn!("pane_result: invalid schema for pane_id={pane_id}: {msg}");
                        write_json_response(
                            response_file,
                            serde_json::json!({
                                "ok": false,
                                "pane_id": pane_id,
                                "error_code": "invalid_schema",
                                "error": msg,
                            }),
                        );
                    }
                }
            }
            crate::protocol::AppRequest::PaneResultWrite {
                pane_id,
                payload,
                response_file,
            } => {
                log::info!(
                    "pane_ipc: kind=pane_result_write pane_id={pane_id} bytes={}",
                    payload.len()
                );
                let pane_alive = self.find_pane_in_any_window(*pane_id).is_some();
                if !pane_alive && !self.pane_results.contains_key(pane_id) {
                    log::warn!("pane_result: write rejected — pane {pane_id} not found");
                    write_json_response(
                        response_file,
                        serde_json::json!({
                            "ok": false,
                            "pane_id": pane_id,
                            "error_code": "pane_not_found",
                            "error": format!("pane {pane_id} not found"),
                        }),
                    );
                    return;
                }
                match self
                    .pane_results
                    .entry(*pane_id)
                    .or_default()
                    .write_payload(payload)
                {
                    Ok(committed) => {
                        log::info!("pane_result: valid result committed for pane_id={pane_id}");
                        write_json_response(
                            response_file,
                            serde_json::json!({
                                "ok": true,
                                "pane_id": pane_id,
                                "result": committed,
                            }),
                        );
                        self.complete_result_waits(*pane_id);
                    }
                    Err(err) => {
                        log::warn!(
                            "pane_result: rejected write for pane_id={pane_id} code={} error={}",
                            err.error_code(),
                            err.message()
                        );
                        write_json_response(
                            response_file,
                            serde_json::json!({
                                "ok": false,
                                "pane_id": pane_id,
                                "error_code": err.error_code(),
                                "error": err.message(),
                                "validation_errors": err.validation_errors(),
                            }),
                        );
                    }
                }
            }
            crate::protocol::AppRequest::PaneResultRead {
                pane_id,
                wait,
                timeout_secs,
                response_file,
            } => {
                log::info!(
                    "pane_ipc: kind=pane_result_read pane_id={pane_id} wait={wait} timeout={timeout_secs}"
                );
                if *wait
                    && (!timeout_secs.is_finite()
                        || *timeout_secs < 0.0
                        || *timeout_secs > crate::app::pane_wait::MAX_WAIT_TIMEOUT_SECS)
                {
                    write_json_response(
                        &format!("{response_file}.err"),
                        serde_json::json!({
                            "ok": false,
                            "error_code": "invalid_timeout",
                            "error": format!(
                                "--timeout must be a non-negative number of seconds at most {}, got {timeout_secs}",
                                crate::app::pane_wait::MAX_WAIT_TIMEOUT_SECS
                            ),
                        }),
                    );
                    return;
                }
                if let Some(committed) = self
                    .pane_results
                    .get(pane_id)
                    .and_then(|state| state.committed.as_ref())
                {
                    let mut bytes =
                        serde_json::to_vec(committed).unwrap_or_else(|_| b"{}".to_vec());
                    bytes.push(b'\n');
                    log::info!(
                        "pane_result_read: pane_id={pane_id} satisfied immediately ({} bytes)",
                        bytes.len()
                    );
                    write_response(response_file, &bytes);
                    return;
                }
                let pane_alive = self.find_pane_in_any_window(*pane_id).is_some();
                let known_state = self.pane_results.get(pane_id);
                if !pane_alive && known_state.is_none() {
                    log::warn!("pane_result_read: pane_id={pane_id} not found");
                    write_json_response(
                        &format!("{response_file}.err"),
                        serde_json::json!({
                            "ok": false,
                            "error_code": "pane_not_found",
                            "error": format!("pane {pane_id} not found"),
                        }),
                    );
                    return;
                }
                if *wait && pane_alive {
                    let expires_at = std::time::Instant::now()
                        + std::time::Duration::from_secs_f64(*timeout_secs);
                    log::info!("pane_result_read: pane_id={pane_id} parked for {timeout_secs}s");
                    self.pending_result_waits
                        .push(crate::app::pane_wait::PendingResultWait {
                            pane_id: *pane_id,
                            response_file: response_file.clone(),
                            expires_at,
                        });
                    return;
                }
                if let Some(reason) = known_state.and_then(|s| s.last_invalid_error.as_deref()) {
                    log::info!(
                        "pane_result_read: pane_id={pane_id} no valid result (last write invalid: {reason})"
                    );
                    write_json_response(
                        &format!("{response_file}.err"),
                        serde_json::json!({
                            "ok": false,
                            "error_code": "invalid_result",
                            "error": format!(
                                "no valid result for pane {pane_id} (last write rejected: {reason})"
                            ),
                        }),
                    );
                } else {
                    log::info!("pane_result_read: pane_id={pane_id} no result yet");
                    write_json_response(
                        &format!("{response_file}.err"),
                        serde_json::json!({
                            "ok": false,
                            "error_code": "no_result_yet",
                            "error": format!("no result yet for pane {pane_id}"),
                        }),
                    );
                }
            }
            crate::protocol::AppRequest::SlotRead {
                pane_id,
                slot_name,
                response_file,
            } => {
                log::info!("pane_ipc: kind=slot_read pane_id={pane_id} slot={slot_name:?}");
                if let Err(msg) = validate_slot_name(slot_name) {
                    slot_read_error(response_file, msg);
                    return;
                }
                let path = self
                    .windows
                    .iter()
                    .find_map(|win| win.panes.get(pane_id))
                    .and_then(|pane| pane.slots())
                    .and_then(|slots| slots.get(slot_name).cloned())
                    .or_else(|| {
                        self.closed_pane_slots
                            .get(pane_id)
                            .and_then(|slots| slots.get(slot_name).cloned())
                    });
                let Some(path) = path else {
                    slot_read_error(response_file, format!("slot '{slot_name}' not found"));
                    return;
                };
                match std::fs::read(&path) {
                    Ok(bytes) => {
                        write_response(response_file, &bytes);
                    }
                    Err(e) => {
                        slot_read_error(
                            response_file,
                            format!("could not read slot '{slot_name}': {e}"),
                        );
                    }
                }
            }
            crate::protocol::AppRequest::SlotList {
                pane_id,
                response_file,
            } => {
                log::info!("pane_ipc: kind=slot_list pane_id={pane_id}");
                let slots = self
                    .windows
                    .iter()
                    .find_map(|win| win.panes.get(pane_id))
                    .and_then(|pane| pane.slots())
                    .or_else(|| self.closed_pane_slots.get(pane_id));
                match slots {
                    Some(slots) => {
                        write_json_response(
                            response_file,
                            serde_json::Value::Array(slot_list_entries(slots)),
                        );
                    }
                    None => slot_error(response_file, format!("pane {pane_id} not found")),
                }
            }
            crate::protocol::AppRequest::SlotDelete {
                pane_id,
                slot_name,
                response_file,
            } => {
                log::info!("pane_ipc: kind=slot_delete pane_id={pane_id} slot={slot_name:?}");
                if let Err(msg) = validate_slot_name(slot_name) {
                    slot_error(response_file, msg);
                    return;
                }
                let target = self.windows.iter().enumerate().find_map(|(idx, win)| {
                    if win.panes.contains_key(pane_id) {
                        Some((idx, win.context_id))
                    } else {
                        None
                    }
                });
                let Some((win_idx, context_id)) = target else {
                    slot_error(response_file, format!("pane {pane_id} not found"));
                    return;
                };
                let context_root = self
                    .router
                    .iter()
                    .find(|ctx| ctx.context_id == context_id)
                    .map(|ctx| ctx.root.clone());
                let fallback_path = slot_base_dir(context_root.as_deref())
                    .join(pane_id.to_string())
                    .join(slot_name);
                let Some(pane) = self.windows[win_idx].panes.get_mut(pane_id) else {
                    slot_error(response_file, format!("pane {pane_id} not found"));
                    return;
                };
                let Some(slots) = pane.slots_mut() else {
                    slot_error(
                        response_file,
                        format!("pane {pane_id} does not support slots"),
                    );
                    return;
                };
                let path = slots.remove(slot_name).unwrap_or(fallback_path);
                let removed = if path.exists() {
                    match std::fs::remove_file(&path) {
                        Ok(()) => true,
                        Err(e) => {
                            slot_error(
                                response_file,
                                format!("could not delete slot '{slot_name}': {e}"),
                            );
                            return;
                        }
                    }
                } else {
                    false
                };
                self.mark_workspace_dirty();
                write_json_response(
                    response_file,
                    serde_json::json!({
                        "ok": true,
                        "name": slot_name,
                        "removed": removed,
                    }),
                );
            }
            crate::protocol::AppRequest::WorkspaceCleanSlots {
                dry_run,
                response_file,
            } => {
                log::info!("pane_ipc: kind=workspace_clean_slots dry_run={dry_run}");
                let live_panes: std::collections::HashSet<u64> = self
                    .windows
                    .iter()
                    .flat_map(|win| win.panes.keys().copied())
                    .collect();
                let live_slot_paths: std::collections::HashSet<std::path::PathBuf> = self
                    .windows
                    .iter()
                    .flat_map(|win| win.panes.values())
                    .filter_map(|pane| pane.slots())
                    .flat_map(|slots| slots.values().cloned())
                    .collect();
                let mut roots = vec![crate::config::config_dir().join("slots")];
                for root in self.router.iter().map(|ctx| &ctx.root) {
                    roots.push(slot_base_dir(Some(root)));
                }
                roots.sort();
                roots.dedup();

                let mut paths = Vec::new();
                for root in roots {
                    let Ok(entries) = std::fs::read_dir(&root) else {
                        continue;
                    };
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if !path.is_dir() {
                            continue;
                        }
                        let Some(pane_id) = entry.file_name().to_string_lossy().parse::<u64>().ok()
                        else {
                            continue;
                        };
                        if !live_panes.contains(&pane_id) {
                            paths.push(path);
                            continue;
                        }
                        let has_live_slots = live_slot_paths
                            .iter()
                            .any(|slot_path| slot_path.parent() == Some(path.as_path()));
                        if !has_live_slots {
                            paths.push(path);
                            continue;
                        }
                        let Ok(slot_entries) = std::fs::read_dir(&path) else {
                            continue;
                        };
                        for slot_entry in slot_entries.flatten() {
                            let slot_path = slot_entry.path();
                            if !live_slot_paths.contains(&slot_path) {
                                paths.push(slot_path);
                            }
                        }
                    }
                }
                paths.sort();
                let mut cleaned = Vec::new();
                let mut clean_error = None;
                for path in paths {
                    if !*dry_run {
                        let remove_result = if path.is_dir() {
                            std::fs::remove_dir_all(&path)
                        } else {
                            std::fs::remove_file(&path)
                        };
                        if let Err(e) = remove_result {
                            clean_error = Some(format!(
                                "could not remove slot path {}: {e}",
                                path.display()
                            ));
                            break;
                        }
                    }
                    cleaned.push(path.to_string_lossy().into_owned());
                }
                if let Some(error) = clean_error {
                    slot_error(response_file, error);
                    return;
                }
                write_json_response(
                    response_file,
                    serde_json::json!({
                        "ok": true,
                        "dry_run": dry_run,
                        "paths": cleaned,
                    }),
                );
            }
            crate::protocol::AppRequest::FocusPane { pane_id } => {
                log::info!("pane_ipc: kind=focus_pane pane_id={pane_id}");
                if !self.pane_navigate(*pane_id) {
                    log::warn!("pane_ipc: focus_pane: pane_id={pane_id} not found");
                }
            }
            crate::protocol::AppRequest::ClosePane {
                pane_id,
                response_file,
            } => {
                log::info!(
                    "pane_ipc: kind=close_pane pane_id={pane_id} response_file={response_file:?}"
                );
                let exists = self.windows.iter().any(|w| w.panes.contains_key(pane_id));
                if !exists {
                    log::warn!("pane_ipc: close_pane: pane_id={pane_id} not found");
                    if let Some(rf) = response_file {
                        write_json_response(
                            rf,
                            serde_json::json!({ "error": format!("pane {pane_id} not found") }),
                        );
                    }
                } else {
                    let wt_report = self.reclaim_pane_worktree(*pane_id);
                    if let Some(rf) = response_file {
                        let mut reply = serde_json::json!({
                            "ok": true,
                            "pane_id": pane_id,
                        });
                        if let Some(report) = &wt_report {
                            reply["worktree"] =
                                serde_json::to_value(report).unwrap_or(serde_json::Value::Null);
                        }
                        write_json_response(rf, reply);
                    }
                    self.close_pane_by_id(*pane_id);
                }
            }
            crate::protocol::AppRequest::SpawnPane { response_file, .. } => {
                let spec = match crate::app::launch_spec::PaneLaunchSpec::from_spawn_pane(&cmd) {
                    Ok(spec) => spec,
                    Err(msg) => {
                        log::warn!("pane_ipc: spawn_pane rejected: {msg}");
                        if let Some(rf) = response_file {
                            write_json_response(rf, serde_json::json!({ "error": msg }));
                        }
                        return;
                    }
                };
                log::info!(
                    "pane_ipc: kind=spawn_pane target={} layout={:?} args={:?} ephemeral={} no_focus={} from_pane_id={:?} cwd={:?} worktree={:?} workspace_root={:?} response_file={:?}",
                    spec.target_for_log(),
                    spec.layout,
                    spec.args,
                    spec.ephemeral,
                    spec.no_focus,
                    spec.from_pane_id,
                    spec.cwd,
                    spec.worktree,
                    spec.workspace_root,
                    spec.response_file
                );
                let requester_is_pane = spec
                    .peer_ancestry
                    .as_deref()
                    .filter(|pids| !pids.is_empty())
                    .is_some_and(|pids| self.resolve_socket_peer_pane(pids).is_some());
                let inject_folder_secrets = !requester_is_pane
                    && crate::release::feature_enabled(crate::release::ReleaseFeature::FolderSecrets);
                if requester_is_pane {
                    log::info!(
                        "pane_ipc: spawn_pane requester is a pane; withholding folder secrets"
                    );
                }
                let mut response_pane_id = self.host.next_pane_id();

                let active = self.active_window;
                let mut cwd_override = spec.cwd.clone();
                let mut created_worktree: Option<crate::host::worktree::ManagedWorktree> = None;
                let mut launch_result: Result<(), String> = Ok(());
                if matches!(
                    spec.target,
                    crate::app::launch_spec::PaneLaunchTarget::Terminal
                ) {
                    let layout_str = spec.layout.as_deref().unwrap_or("split_h");
                    let initial_cmd = super::cmd_from_args(&spec.args);
                    // Named-context targeting (stint 0574): `plexi routine run`
                    // fires a routine into its configured context by name,
                    // matching the scheduler's fire path. The context is a hard
                    // target — a missing name errors back to the caller, and
                    // the spawn never touches active_window or focus state.
                    let resolved_context_target = if let Some(ctx_name) =
                        spec.context_name.as_deref().filter(|c| !c.is_empty())
                    {
                        if layout_str == "new_window" || layout_str == "tab" {
                            let msg = format!(
                                "context_name does not support layout '{layout_str}' — use a split layout"
                            );
                            log::warn!("pane_ipc: spawn_pane rejected: {msg}");
                            if let Some(rf) = &spec.response_file {
                                write_json_response(rf, serde_json::json!({ "error": msg }));
                            }
                            return;
                        }
                        let Some(target_ctx_id) = self
                            .router
                            .iter()
                            .find(|c| c.name == ctx_name)
                            .map(|c| c.context_id)
                        else {
                            let msg = format!("context '{ctx_name}' does not exist");
                            log::warn!("pane_ipc: spawn_pane: {msg}");
                            if let Some(rf) = &spec.response_file {
                                write_json_response(rf, serde_json::json!({ "error": msg }));
                            }
                            return;
                        };
                        let Some((target_win, target_tile)) =
                            self.context_spawn_target(target_ctx_id)
                        else {
                            let msg = format!("context '{ctx_name}' has no pane to spawn beside");
                            log::warn!("pane_ipc: spawn_pane: {msg}");
                            if let Some(rf) = &spec.response_file {
                                write_json_response(rf, serde_json::json!({ "error": msg }));
                            }
                            return;
                        };
                        Some((ctx_name, target_ctx_id, target_win, target_tile))
                    } else {
                        None
                    };

                    if let Some(ref branch) = spec.worktree {
                        let anchor_dir = self.resolve_worktree_anchor(&spec);
                        match crate::host::worktree::create_worktree(&anchor_dir, branch) {
                            Ok(wt) => {
                                cwd_override = Some(wt.path.clone());
                                created_worktree = Some(wt);
                            }
                            Err(msg) => {
                                log::warn!("pane_ipc: spawn_pane worktree rejected: {msg}");
                                if let Some(rf) = &spec.response_file {
                                    write_json_response(rf, serde_json::json!({ "error": msg }));
                                }
                                return;
                            }
                        }
                    }

                    if let Some((ctx_name, target_ctx_id, target_win, target_tile)) =
                        resolved_context_target
                    {
                        let vertical =
                            matches!(layout_str, "split_h" | "split_right" | "split_left");
                        let new_pane_first = matches!(layout_str, "split_above" | "split_left");
                        log::info!(
                            "pane_ipc: spawn_pane terminal context_name='{ctx_name}' context_id={target_ctx_id} target_win={target_win} layout={layout_str} initial_cmd={initial_cmd:?} ephemeral={}",
                            spec.ephemeral
                        );
                        response_pane_id = self.spawn_terminal_pane_at(
                            crate::pane_ops::create::SpawnTerminalAtSpec {
                                win_idx: target_win,
                                target_tile,
                                vertical,
                                new_pane_first,
                                keep_focus: spec.no_focus,
                                options: crate::pane_ops::create::TerminalSpawnOptions {
                                    initial_cmd: initial_cmd.as_deref(),
                                    close_on_exit: spec.ephemeral,
                                    cwd_override,
                                    agent_pane: spec.agent.is_some(),
                                    inject_folder_secrets,
                                },
                            },
                        );
                    } else if layout_str == "new_window" {
                        // Derive context from the calling pane's window; fall back to active.
                        let target_win_idx = spec
                            .from_pane_id
                            .and_then(|fid| self.find_pane_in_any_window(fid).map(|(idx, _)| idx))
                            .unwrap_or(self.active_window);
                        let ws_id = self.windows[target_win_idx].context_id;
                        let active_y = self.windows[target_win_idx].grid_y;
                        let max_x = self
                            .windows
                            .iter()
                            .filter(|w| w.context_id == ws_id && w.grid_y == active_y)
                            .map(|w| w.grid_x)
                            .max();
                        let new_x = max_x.map(|x| x + 1).unwrap_or(1);
                        log::info!(
                            "pane_ipc: spawn_pane terminal layout=new_window from_pane_id={:?} target_win_idx={target_win_idx} context={ws_id} grid=({new_x},{active_y}) response_pane_id={response_pane_id} cwd={cwd_override:?} initial_cmd={initial_cmd:?} ephemeral={}",
                            spec.from_pane_id,
                            spec.ephemeral
                        );
                        self.create_page_at(
                            new_x,
                            active_y,
                            ws_id,
                            crate::pane_ops::create::TerminalSpawnOptions {
                                initial_cmd: initial_cmd.as_deref(),
                                close_on_exit: spec.ephemeral,
                                cwd_override,
                                agent_pane: spec.agent.is_some(),
                                inject_folder_secrets,
                            },
                        );
                        if spec.no_focus {
                            self.active_window = active;
                        }
                    } else if layout_str == "tab" {
                        // Derive target window from the calling pane's window; fall back to active.
                        let target_win_idx = spec
                            .from_pane_id
                            .and_then(|fid| self.find_pane_in_any_window(fid).map(|(idx, _)| idx))
                            .unwrap_or(active);
                        let original_focused = self.windows[target_win_idx].focused_pane;
                        log::info!(
                            "pane_ipc: spawn_pane terminal layout=tab from_pane_id={:?} target_win_idx={target_win_idx} window_id={} response_pane_id={response_pane_id} cwd={cwd_override:?} initial_cmd={initial_cmd:?} ephemeral={}",
                            spec.from_pane_id,
                            self.windows[target_win_idx].window_id,
                            spec.ephemeral
                        );
                        self.new_tab(
                            target_win_idx,
                            initial_cmd.as_deref(),
                            spec.ephemeral,
                            cwd_override,
                            spec.agent.is_some(),
                            inject_folder_secrets,
                        );
                        if spec.no_focus {
                            self.restore_window_focused_pane(target_win_idx, original_focused);
                        }
                        // self.active_window is intentionally left untouched here, matching
                        // the split path below: a from_pane_id spawn must never steal the
                        // user's currently active Plexi window, only set the new tab as the
                        // target window's own focused pane (done inside new_tab above).
                    } else {
                        let vertical =
                            matches!(layout_str, "split_h" | "split_right" | "split_left");
                        let new_pane_first = matches!(layout_str, "split_above" | "split_left");
                        // Resolve target window and tile: from_pane_id wins (cross-window),
                        // then fall back to the active window's focused pane.
                        let split_target = if let Some(from_id) = spec.from_pane_id {
                            match self.find_pane_in_any_window(from_id) {
                                Some(loc) => {
                                    log::info!(
                                        "pane_ipc: spawn_pane: targeting from_pane_id={from_id} in win_idx={}",
                                        loc.0
                                    );
                                    Some(loc)
                                }
                                None => {
                                    log::warn!(
                                        "pane_ipc: spawn_pane: from_pane_id={from_id} not found in any window, using focused pane"
                                    );
                                    self.windows[active]
                                        .focused_pane
                                        .or(self.windows[active].tree.root)
                                        .map(|tile| (active, tile))
                                }
                            }
                        } else {
                            self.windows[active]
                                .focused_pane
                                .or(self.windows[active].tree.root)
                                .map(|tile| (active, tile))
                        };

                        if let Some((target_win, target_tile)) = split_target {
                            let keep_focus = spec.no_focus || spec.from_pane_id.is_some();
                            log::info!(
                                "pane_ipc: spawn_pane terminal layout={layout_str} vertical={vertical} new_pane_first={new_pane_first} initial_cmd={initial_cmd:?} ephemeral={} target_win={target_win} keep_focus={keep_focus}",
                                spec.ephemeral
                            );
                            response_pane_id = self.spawn_terminal_pane_at(
                                crate::pane_ops::create::SpawnTerminalAtSpec {
                                    win_idx: target_win,
                                    target_tile,
                                    vertical,
                                    new_pane_first,
                                    keep_focus,
                                    options: crate::pane_ops::create::TerminalSpawnOptions {
                                        initial_cmd: initial_cmd.as_deref(),
                                        close_on_exit: spec.ephemeral,
                                        cwd_override,
                                        agent_pane: spec.agent.is_some(),
                                        inject_folder_secrets,
                                    },
                                },
                            );
                            if spec.no_focus {
                                self.active_window = active;
                            }
                        } else {
                            // Truly empty window (windowless boot): no root pane to
                            // split, so `split_focused` would no-op and drop the
                            // spawn. Seed a root pane into the existing active window
                            // instead. The seeded pane's id is the id we report.
                            log::info!(
                                "pane_ipc: spawn_pane terminal layout={layout_str} vertical={vertical} new_pane_first={new_pane_first} initial_cmd={initial_cmd:?} ephemeral={} (empty active window — seeding root)",
                                spec.ephemeral
                            );
                            match self.seed_root_pane(
                                initial_cmd.as_deref(),
                                spec.ephemeral,
                                cwd_override,
                                spec.agent.is_some(),
                                inject_folder_secrets,
                            ) {
                                Some(seeded_id) => response_pane_id = seeded_id,
                                None => launch_result = Err("failed to seed root pane".into()),
                            }
                            if spec.no_focus {
                                self.active_window = active;
                            }
                        }
                    }
                } else if let crate::app::launch_spec::PaneLaunchTarget::Path(app_path) =
                    &spec.target
                {
                    let (target_win, orig_focused_in_target) =
                        self.redirect_focus_to_spawn_origin(spec.from_pane_id, "path", active);
                    launch_result = self
                        .launch_app_by_path_with_layout_no_review_modal(
                            &app_path.to_string_lossy(),
                            spec.layout.clone(),
                            spec.workspace_root.clone(),
                            &spec.args,
                        )
                        .map(|pane_id| {
                            if let Some(pane_id) = pane_id {
                                response_pane_id = pane_id;
                            }
                        });
                    if spec.from_pane_id.is_some() {
                        self.active_window = active;
                        // Undo the temporary focus redirect when launch failed.
                        if launch_result.is_err() {
                            self.restore_window_focused_pane(target_win, orig_focused_in_target);
                        }
                    }
                    if spec.no_focus {
                        self.active_window = active;
                        self.restore_window_focused_pane(target_win, orig_focused_in_target);
                    }
                } else if let crate::app::launch_spec::PaneLaunchTarget::AppId(type_id) =
                    &spec.target
                {
                    let (target_win, orig_focused_in_target) =
                        self.redirect_focus_to_spawn_origin(spec.from_pane_id, "app", active);
                    // CLI/spawn-request app opens default to a sibling split, never
                    // an overlay takeover of the caller's pane. Manifest `[launch]
                    // placement` still overrides the default (stint 0330).
                    //
                    // `self.active_window` has already been redirected to the
                    // target window above (when `from_pane_id` named one), so
                    // its context_id is the launch's actual target context —
                    // matching what `launch_app_by_id_with_layout` itself
                    // resolves against (stint 0724 Phase B).
                    let target_context_id = self.windows[self.active_window].context_id;
                    let placement = crate::pane_ops::cli_open_placement(
                        spec.layout.clone(),
                        self.registries
                            .view_for_context(target_context_id, &self.router)
                            .placement_for(type_id),
                    );
                    launch_result = self
                        .launch_app_by_id_with_layout(
                            type_id,
                            Some(placement),
                            &spec.args,
                            cwd_override,
                        )
                        .map(|existing_id| {
                            // A dedup focused a live instance; the response must
                            // report that pane, not the predicted id (#0336).
                            if let Some(existing_id) = existing_id {
                                response_pane_id = existing_id;
                            }
                        });
                    if spec.from_pane_id.is_some() {
                        self.active_window = active;
                        // Undo the temporary focus redirect when launch failed.
                        if launch_result.is_err() {
                            self.restore_window_focused_pane(target_win, orig_focused_in_target);
                        }
                    }
                    if spec.no_focus {
                        self.active_window = active;
                        self.restore_window_focused_pane(target_win, orig_focused_in_target);
                    }
                }
                if let Some(wt) = created_worktree {
                    let pane_live = launch_result.is_ok()
                        && self
                            .windows
                            .iter()
                            .any(|w| w.panes.contains_key(&response_pane_id));
                    if pane_live {
                        log::info!(
                            "pane_worktree: registered pane_id={response_pane_id} branch='{}' path='{}'",
                            wt.branch,
                            wt.path.display()
                        );
                        self.pane_worktrees.insert(response_pane_id, wt);
                    } else {
                        let _ = crate::host::worktree::reclaim_or_retain_worktree(&wt);
                    }
                }
                if launch_result.is_ok() {
                    let entry = self.pane_results.entry(response_pane_id).or_default();
                    if let Some(ref schema) = spec.result_schema {
                        entry.schema = Some(schema.clone());
                        log::info!(
                            "pane_result: registered schema at spawn for pane_id={response_pane_id}"
                        );
                    }
                }
                if matches!(
                    spec.target,
                    crate::app::launch_spec::PaneLaunchTarget::Terminal
                ) && launch_result.is_ok()
                {
                    if let Some(ref pane_name) = spec.name {
                        if !pane_name.is_empty() {
                            self.apply_inline_pane_name(response_pane_id, pane_name);
                        }
                    }
                }
                self.reply_spawn_pane(&spec, response_pane_id, &launch_result);
            }
            crate::protocol::AppRequest::SendToPane {
                pane_id,
                text,
                submit,
                paste,
                response_file,
            } => {
                log::info!(
                    "pane_ipc: kind=send_to_pane pane_id={pane_id} len={} submit={submit} windows={} response_file={response_file:?}",
                    text.len(),
                    self.windows.len()
                );
                let text_with_newlines = if *paste {
                    text.clone()
                } else {
                    text.replace("\\n", "\n")
                };
                // Set when the reply is owed by `service_pending_submits`
                // instead of by the write below.
                let mut deferred = false;
                let pane_kind = self.windows.iter().find_map(|win| {
                    win.panes.get(pane_id).map(|pane| {
                        if pane.as_terminal().is_some() {
                            "terminal"
                        } else if pane.as_app().is_some() {
                            "app"
                        } else {
                            "unsupported"
                        }
                    })
                });
                let result = match pane_kind {
                    None => {
                        log::warn!(
                            "pane_ipc: send_to_pane: pane_id={pane_id} not found in any window"
                        );
                        Err(format!("pane {pane_id} not found"))
                    }
                    Some(_) if *paste && *submit => {
                        Err("--paste and --submit cannot be combined".to_string())
                    }
                    Some("terminal") if *paste => {
                        Err("--paste only applies to app panes".to_string())
                    }
                    Some("terminal") if *submit && self.submit_in_flight(*pane_id) => {
                        // Refuse before typing anything. Two submits racing on
                        // one input line interleave their enters, which is the
                        // exact corruption this verb exists to prevent.
                        log::warn!(
                            "pane_submit: pane_id={pane_id} refused — a submit is already in flight"
                        );
                        Err(format!(
                            "pane {pane_id} already has a `pane send --submit` in flight"
                        ))
                    }
                    Some("terminal") => {
                        self.resume_standby_pane(*pane_id);
                        match self
                            .windows
                            .iter_mut()
                            .find_map(|win| win.panes.get_mut(pane_id))
                            .and_then(crate::host::pane::Pane::as_terminal_mut)
                        {
                            Some(term) => {
                                term.backend
                                    .process_command(egui_term::BackendCommand::Write(
                                        text_with_newlines.into_bytes(),
                                    ));
                                if *submit {
                                    self.register_pending_submit(
                                        *pane_id,
                                        response_file.clone(),
                                        text.chars().count(),
                                    );
                                    deferred = response_file.is_some();
                                }
                                Ok(())
                            }
                            None => Err(format!("pane {pane_id} changed while routing text")),
                        }
                    }
                    Some("app") if *submit => {
                        log::warn!("pane_submit: pane_id={pane_id} refused — not a terminal pane");
                        Err(format!(
                            "pane {pane_id} is an app pane; --submit only applies to terminal panes"
                        ))
                    }
                    Some("app") => {
                        if !self.pane_navigate(*pane_id) {
                            Err(format!("pane {pane_id} could not be focused"))
                        } else {
                            self.pending_pane_inputs.entry(*pane_id).or_default().push(
                                egui::RawInput {
                                    events: vec![if *paste {
                                        egui::Event::Paste(text_with_newlines)
                                    } else {
                                        egui::Event::Text(text_with_newlines)
                                    }],
                                    ..Default::default()
                                },
                            );
                            self.ctx.request_repaint();
                            log::info!(
                                "pane_ipc: send_to_pane: app pane_id={pane_id} paste={paste} text_chars={}",
                                text.chars().count()
                            );
                            Ok(())
                        }
                    }
                    Some(_) => Err(format!("pane {pane_id} does not accept text input")),
                };
                if let Some(rf) = response_file.as_ref().filter(|_| !deferred) {
                    let json = match result {
                        Ok(()) => r#"{"ok":true}"#.to_string(),
                        Err(ref msg) => format!(
                            "{{\"error\":{}}}",
                            serde_json::to_string(msg).unwrap_or_else(|_| format!("\"{msg}\""))
                        ),
                    };
                    write_response(rf, json.as_bytes());
                }
            }
            crate::protocol::AppRequest::PaneHeartbeat {
                pane_id,
                every_ms,
                text,
                while_idle_only,
                off,
                response_file,
            } => {
                let result: Result<serde_json::Value, String> = (|| {
                    if *off {
                        self.pane_heartbeats.remove(pane_id);
                        self.mark_workspace_dirty();
                        log::info!("pane_heartbeat: pane_id={pane_id} disabled");
                        Ok(serde_json::json!({"ok": true, "heartbeat": null}))
                    } else {
                        let every_ms = every_ms
                            .ok_or_else(|| "--every is required unless --off is set".to_string())?;
                        let text = text
                            .clone()
                            .ok_or_else(|| "--text is required unless --off is set".to_string())?;
                        if every_ms == 0 {
                            Err("--every must be greater than zero".to_string())
                        } else if !self.windows.iter().any(|window| {
                            window
                                .panes
                                .get(pane_id)
                                .and_then(crate::host::pane::Pane::as_terminal)
                                .is_some()
                        }) {
                            Err(format!("pane {pane_id} is not a terminal pane"))
                        } else {
                            let while_idle_only = while_idle_only.unwrap_or(true);
                            self.pane_heartbeats.insert(
                                *pane_id,
                                crate::app::PaneHeartbeat {
                                    every: std::time::Duration::from_millis(every_ms),
                                    text,
                                    while_idle_only,
                                    next_fire: std::time::Instant::now()
                                        + std::time::Duration::from_millis(every_ms),
                                },
                            );
                            self.mark_workspace_dirty();
                            log::info!(
                                "pane_heartbeat: pane_id={pane_id} configured every_ms={every_ms} while_idle_only={while_idle_only}"
                            );
                            Ok(self.pane_heartbeat_json(*pane_id))
                        }
                    }
                })();
                if let Some(response_file) = response_file {
                    write_json_response(
                        response_file,
                        result.unwrap_or_else(|error| serde_json::json!({"error": error})),
                    );
                }
            }
            crate::protocol::AppRequest::KeyPane {
                pane_id,
                key,
                response_file,
            } => {
                log::info!(
                    "pane_ipc: kind=key_pane pane_id={pane_id} key_chars={}",
                    key.chars().count()
                );
                if self.refuse_synthetic_approval(
                    *pane_id,
                    Some(key),
                    None,
                    response_file.as_deref(),
                ) {
                    // Sheet keys from the socket never reach the approval widget.
                } else {
                let mut passthrough_raw = None;
                // Mirror the live dispatch gate (stint 0456): while this
                // pane's TextInput holds egui focus, keys belong to the
                // host TextEdit — replay into the real ctx (the passthrough
                // path below) instead of the app's raw KeyEvent path, the
                // same routing a physical keypress gets.
                let text_input_focused =
                    crate::app::input_owner::focused_pane_text_surface(&self.ctx, *pane_id);
                let mut result: Result<serde_json::Value, String> = match self
                    .windows
                    .iter_mut()
                    .find_map(|win| win.panes.get_mut(pane_id))
                {
                    None => {
                        log::warn!("pane_ipc: key_pane: pane_id={pane_id} not found");
                        Err(format!("pane {pane_id} not found"))
                    }
                    Some(pane) => {
                        if let Some(term) = pane.as_terminal_mut() {
                            let is_enter = key.eq_ignore_ascii_case("enter")
                                || key.eq_ignore_ascii_case("return");
                            if is_enter && term.is_standby() {
                                self.resume_standby_pane(*pane_id);
                            } else {
                                let bytes = super::key_str_to_pty_bytes(key);
                                term.backend
                                    .process_command(egui_term::BackendCommand::Write(bytes));
                            }
                            Ok(serde_json::json!({"ok": true}))
                        } else if pane.as_app_mut().is_some()
                            && text_input_focused
                            && key.eq_ignore_ascii_case("escape")
                        {
                            // Escape parity with a live keypress (stint 0460):
                            // a replayed raw Escape lands mid-frame, after the
                            // dispatch gate already ran, so it would fall
                            // through to the AppActive CloseApp binding and
                            // destroy the pane. Deliver it to the app's
                            // handle_key instead — the assistant interrupts
                            // its in-flight turn — and never replay it raw.
                            let app_pane = pane.as_app_mut().expect("checked above");
                            match super::drive_native_pane_key(&mut app_pane.runtime, key) {
                                Ok(disposition) => {
                                    log::info!(
                                        "pane_ipc: key_pane: pane {pane_id} Escape delivered to app (text surface focused, CloseApp suppressed, result={disposition:?})"
                                    );
                                    Ok(serde_json::json!({
                                        "ok": true,
                                        "disposition": "text_input_escape",
                                    }))
                                }
                                Err(e) => Err(e),
                            }
                        } else if pane.as_app_mut().is_some() && text_input_focused {
                            match super::key_str_to_egui_raw_input(key) {
                                Some(raw) => {
                                    log::info!(
                                        "pane_ipc: key_pane: pane {pane_id} routed to its focused TextInput"
                                    );
                                    passthrough_raw = Some(raw);
                                    Ok(serde_json::json!({
                                        "ok": true,
                                        "disposition": "text_input",
                                    }))
                                }
                                None => Err(format!(
                                    "key {key:?} does not map to a native app key event"
                                )),
                            }
                        } else if let Some(app_pane) = pane.as_app_mut() {
                            let runtime = &mut app_pane.runtime;
                            match super::drive_native_pane_key(runtime, key) {
                                Ok(disposition) => {
                                    let disposition_label = match disposition {
                                        crate::app::app_trait::KeyDisposition::Consumed => {
                                            "consumed"
                                        }
                                        crate::app::app_trait::KeyDisposition::Passthrough => {
                                            passthrough_raw = super::key_str_to_egui_raw_input(key);
                                            "passthrough"
                                        }
                                    };
                                    log::info!(
                                        "pane_ipc: key_pane: native app pane_id={pane_id} \
                                             key_chars={} disposition={disposition_label}",
                                        key.chars().count()
                                    );
                                    Ok(serde_json::json!({
                                        "ok": true,
                                        "disposition": disposition_label,
                                    }))
                                }
                                Err(e) => {
                                    log::warn!("pane_ipc: key_pane: {e}");
                                    Err(e)
                                }
                            }
                        } else {
                            Err(format!("pane {pane_id}: unknown pane type"))
                        }
                    }
                };
                if let Some(raw) = passthrough_raw {
                    if self.pane_navigate(*pane_id) {
                        self.pending_pane_inputs
                            .entry(*pane_id)
                            .or_default()
                            .push(raw);
                        self.ctx.request_repaint();
                    } else {
                        result = Err(format!("pane {pane_id} could not be focused"));
                    }
                }
                if let Some(rf) = response_file {
                    let json = match &result {
                        Ok(v) => v.to_string(),
                        Err(msg) => serde_json::json!({"error": msg}).to_string(),
                    };
                    write_response(rf, json.as_bytes());
                }
                }
            }
            crate::protocol::AppRequest::DropFile {
                pane_id,
                path_or_url,
                response_file,
            } => {
                let result = (|| {
                    let Some((win_idx, _tile_id)) = self.find_pane_in_any_window(*pane_id) else {
                        return Err(format!("pane {pane_id} not found"));
                    };
                    if !self.pane_navigate(*pane_id) {
                        return Err(format!("pane {pane_id} could not be focused for drop"));
                    }
                    let app = self.windows[win_idx]
                        .panes
                        .get_mut(pane_id)
                        .and_then(crate::host::pane::Pane::as_app_mut)
                        .ok_or_else(|| format!("pane {pane_id} cannot accept this drop"))?;
                    crate::spatial::tiling::dispatch_drop_to_app(*pane_id, app, path_or_url, true)
                })();
                match &result {
                    Ok(_) => log::info!(
                        "drop: delivery accepted pane_id={pane_id} source_kind={}",
                        if path_or_url.contains("://") {
                            "url"
                        } else {
                            "file"
                        }
                    ),
                    Err(error) => {
                        log::info!("drop: delivery rejected pane_id={pane_id} reason={error}")
                    }
                }
                let json = match result {
                    Ok(value) => serde_json::json!({"ok": true, "delivery": value}),
                    Err(error) => serde_json::json!({"error": error}),
                };
                write_json_response(response_file, json);
                self.ctx.request_repaint();
            }
            crate::protocol::AppRequest::ClickPane {
                pane_id,
                x,
                y,
                button,
                response_file,
            } => {
                log::info!(
                    "pane_ipc: kind=click_pane pane_id={pane_id} x={x} y={y} button={button:?}"
                );
                let click_abs = self.find_pane_in_any_window(*pane_id).and_then(|(win, tile)| {
                    self.windows[win]
                        .tree
                        .tiles
                        .rect(tile)
                        .map(|rect| rect.min + egui::vec2(*x, *y))
                });
                if self.refuse_synthetic_approval(
                    *pane_id,
                    None,
                    click_abs,
                    response_file.as_deref(),
                ) {
                    // A socket click on an approval button or surface is not a grant.
                } else {
                let result: Result<serde_json::Value, String> = (|| {
                    let Some((win_idx, tile_id)) = self.find_pane_in_any_window(*pane_id) else {
                        return Err(format!("pane {pane_id} not found"));
                    };
                    let Some(pane_rect) = self.windows[win_idx].tree.tiles.rect(tile_id) else {
                        return Err(format!(
                            "pane {pane_id}: no known screen rect (pane has not rendered yet)"
                        ));
                    };
                    let Some(is_builtin) = self.windows[win_idx]
                        .panes
                        .get(pane_id)
                        .and_then(crate::host::pane::Pane::as_app)
                        .map(|app_pane| {
                            matches!(app_pane.runtime, crate::host::pane::AppRuntime::Builtin(_))
                        })
                    else {
                        return Err(format!(
                            "pane {pane_id}: click injection is only supported for app panes"
                        ));
                    };
                    if !self.pane_navigate(*pane_id) {
                        return Err(format!("pane {pane_id} could not be focused"));
                    }
                    let abs = pane_rect.min + egui::vec2(*x, *y);
                    let button_str = match button.as_deref() {
                        Some("right") => "right",
                        Some("middle") => "middle",
                        _ => "left",
                    };
                    // NOT a mid-pass `ctx.input_mut()` replay (that pattern only
                    // works for `KeyPane`): egui resolves `Response::clicked()`
                    // once, inside `Context::begin_pass`, strictly before this
                    // dispatch code runs, and discards unconsumed `events` at
                    // the next pass — so a mutated pointer event can never be
                    // recognized as a click. Queue instead, on the plane the
                    // pane's runtime actually consumes:
                    if is_builtin {
                        // Builtin apps are ordinary egui widgets — nothing
                        // consumes `PendingPaneClick` on their render path, so
                        // queue genuine pointer events through the same
                        // pre-pass raw-input merge `KeyPane` uses
                        // (`raw_input_hook`). egui then resolves an authentic
                        // press/release click on whatever widget owns that
                        // position, exactly as a physical click would.
                        let pointer_button = match button_str {
                            "right" => egui::PointerButton::Secondary,
                            "middle" => egui::PointerButton::Middle,
                            _ => egui::PointerButton::Primary,
                        };
                        let events = vec![
                            egui::Event::PointerMoved(abs),
                            egui::Event::PointerButton {
                                pos: abs,
                                button: pointer_button,
                                pressed: true,
                                modifiers: egui::Modifiers::default(),
                            },
                            egui::Event::PointerButton {
                                pos: abs,
                                button: pointer_button,
                                pressed: false,
                                modifiers: egui::Modifiers::default(),
                            },
                        ];
                        self.pending_pane_inputs.entry(*pane_id).or_default().push(
                            egui::RawInput {
                                events,
                                ..Default::default()
                            },
                        );
                    } else {
                        // Canvas/Python panes: the render branch
                        // (`wasm_render.rs`) picks the queued click up against
                        // the frame's live widget rect, still through the
                        // pane's real `canvas_transform` inversion.
                        self.pending_pane_clicks.insert(
                            *pane_id,
                            crate::host::pane::PendingPaneClick {
                                target: crate::host::pane::PaneClickTarget::Pos(abs),
                                button: button_str,
                                phase: crate::host::pane::PointerPhase::Click,
                            },
                        );
                    }
                    self.ctx.request_repaint();
                    log::info!(
                        "pane_ipc: click_pane: pane_id={pane_id} queued at abs=({:.1},{:.1})",
                        abs.x,
                        abs.y
                    );
                    Ok(serde_json::json!({"ok": true}))
                })();
                if let Some(rf) = response_file {
                    let json = match &result {
                        Ok(v) => v.to_string(),
                        Err(msg) => serde_json::json!({"error": msg}).to_string(),
                    };
                    write_response(rf, json.as_bytes());
                }
                }
            }
            crate::protocol::AppRequest::ClickPaneNode {
                pane_id,
                node_id,
                button,
                response_file,
            } => {
                log::info!(
                    "pane_ipc: kind=click_pane_node pane_id={pane_id} node_id={node_id} button={button:?}"
                );
                let result: Result<serde_json::Value, String> = (|| {
                    let Some((win_idx, _tile_id)) = self.find_pane_in_any_window(*pane_id) else {
                        return Err(format!("pane {pane_id} not found"));
                    };
                    let Some(app_pane) = self.windows[win_idx]
                        .panes
                        .get(pane_id)
                        .and_then(|pane| pane.as_app())
                    else {
                        return Err(format!(
                            "pane {pane_id}: click injection is only supported for app panes"
                        ));
                    };
                    let arena_id = app_pane
                        .semantic_state()
                        .resolve_interactive_node(node_id)?;
                    if !self.pane_navigate(*pane_id) {
                        return Err(format!("pane {pane_id} could not be focused"));
                    }
                    let button_str = match button.as_deref() {
                        Some("right") => "right",
                        Some("middle") => "middle",
                        _ => "left",
                    };
                    // Same non-mid-pass reasoning as `ClickPane` above: queue
                    // instead of replaying pointer input. The interactive
                    // node's own render branch (`wasm_render.rs`) matches this
                    // arena id against the frame's live tree and activates it
                    // exactly as a real click would.
                    self.pending_pane_clicks.insert(
                        *pane_id,
                        crate::host::pane::PendingPaneClick {
                            target: crate::host::pane::PaneClickTarget::Node(arena_id),
                            button: button_str,
                            phase: crate::host::pane::PointerPhase::Click,
                        },
                    );
                    self.ctx.request_repaint();
                    log::info!(
                        "pane_ipc: click_pane_node: pane_id={pane_id} queued node_id={node_id} arena_id={arena_id}"
                    );
                    Ok(serde_json::json!({"ok": true}))
                })();
                if let Some(rf) = response_file {
                    let json = match &result {
                        Ok(v) => v.to_string(),
                        Err(msg) => serde_json::json!({"error": msg}).to_string(),
                    };
                    write_response(rf, json.as_bytes());
                }
            }
            crate::protocol::AppRequest::DragPane {
                pane_id,
                from,
                from_node,
                to,
                to_node,
                steps,
                button,
                response_file,
            } => {
                log::info!(
                    "pane_ipc: kind=drag_pane pane_id={pane_id} from={from:?} from_node={from_node:?} \
                     to={to:?} to_node={to_node:?} steps={steps:?} button={button:?}"
                );
                let result: Result<serde_json::Value, String> = (|| {
                    let Some((win_idx, tile_id)) = self.find_pane_in_any_window(*pane_id) else {
                        return Err(format!("pane {pane_id} not found"));
                    };
                    let Some(pane_rect) = self.windows[win_idx].tree.tiles.rect(tile_id) else {
                        return Err(format!(
                            "pane {pane_id}: no known screen rect (pane has not rendered yet)"
                        ));
                    };
                    let Some(app_pane) = self.windows[win_idx]
                        .panes
                        .get(pane_id)
                        .and_then(crate::host::pane::Pane::as_app)
                    else {
                        return Err(format!(
                            "pane {pane_id}: drag injection is only supported for app panes"
                        ));
                    };
                    let is_builtin =
                        matches!(app_pane.runtime, crate::host::pane::AppRuntime::Builtin(_));
                    // Resolve each endpoint to an absolute screen position:
                    // pane-pixel coordinates directly, or a semantic node's
                    // cached bounds center. Nodes without recorded bounds
                    // fail loudly — a drag must never silently guess.
                    let resolve = |px: &Option<[f32; 2]>,
                                   node: &Option<String>,
                                   which: &str|
                     -> Result<egui::Pos2, String> {
                        match (px, node) {
                            (Some(_), Some(_)) => Err(format!(
                                "drag {which}: give pixel coordinates or a node id, not both"
                            )),
                            (Some([x, y]), None) => Ok(pane_rect.min + egui::vec2(*x, *y)),
                            (None, Some(node_id)) => {
                                let state = app_pane.semantic_state();
                                let Some(node) = state.nodes.iter().find(|n| &n.id == node_id)
                                else {
                                    return Err(format!(
                                        "drag {which}: node {node_id} not found in this pane's current tree"
                                    ));
                                };
                                let Some([x0, y0, x1, y1]) = node.bounds else {
                                    return Err(format!(
                                        "drag {which}: node {node_id} has no rendered bounds; \
                                         target it by pixel coordinates instead"
                                    ));
                                };
                                Ok(egui::pos2(
                                    ((x0 + x1) / 2.0) as f32,
                                    ((y0 + y1) / 2.0) as f32,
                                ))
                            }
                            (None, None) => Err(format!(
                                "drag {which}: missing endpoint — set `{which}` = [x, y] or `{which}_node`"
                            )),
                        }
                    };
                    let from_abs = resolve(from, from_node, "from")?;
                    let to_abs = resolve(to, to_node, "to")?;
                    let steps = steps.unwrap_or(8).clamp(1, 256);
                    let button_str = match button.as_deref() {
                        Some("right") => "right",
                        Some("middle") => "middle",
                        _ => "left",
                    };
                    if !self.pane_navigate(*pane_id) {
                        return Err(format!("pane {pane_id} could not be focused"));
                    }
                    let lerp = |t: f32| from_abs + (to_abs - from_abs) * t;
                    // Press, `steps` intermediate moves, release — one entry
                    // per frame on the plane the pane's runtime consumes.
                    if is_builtin {
                        let pointer_button = match button_str {
                            "right" => egui::PointerButton::Secondary,
                            "middle" => egui::PointerButton::Middle,
                            _ => egui::PointerButton::Primary,
                        };
                        let press = |pos: egui::Pos2, pressed: bool| egui::RawInput {
                            events: vec![
                                egui::Event::PointerMoved(pos),
                                egui::Event::PointerButton {
                                    pos,
                                    button: pointer_button,
                                    pressed,
                                    modifiers: egui::Modifiers::default(),
                                },
                            ],
                            ..Default::default()
                        };
                        let mut frames = std::collections::VecDeque::new();
                        frames.push_back(press(from_abs, true));
                        for i in 1..=steps {
                            frames.push_back(egui::RawInput {
                                events: vec![egui::Event::PointerMoved(lerp(
                                    i as f32 / (steps + 1) as f32,
                                ))],
                                ..Default::default()
                            });
                        }
                        frames.push_back(press(to_abs, false));
                        self.pending_pane_pointer_frames.insert(*pane_id, frames);
                    } else {
                        use crate::host::pane::{PaneClickTarget, PendingPaneClick, PointerPhase};
                        let sample = |pos: egui::Pos2, phase: PointerPhase| PendingPaneClick {
                            target: PaneClickTarget::Pos(pos),
                            button: button_str,
                            phase,
                        };
                        let mut samples = std::collections::VecDeque::new();
                        samples.push_back(sample(from_abs, PointerPhase::Press));
                        for i in 1..=steps {
                            samples.push_back(sample(
                                lerp(i as f32 / (steps + 1) as f32),
                                PointerPhase::Move,
                            ));
                        }
                        samples.push_back(sample(to_abs, PointerPhase::Release));
                        self.pending_pane_drags.insert(*pane_id, samples);
                    }
                    self.ctx.request_repaint();
                    log::info!(
                        "pane_ipc: drag_pane: pane_id={pane_id} queued {} frames \
                         from=({:.1},{:.1}) to=({:.1},{:.1}) button={button_str}",
                        steps + 2,
                        from_abs.x,
                        from_abs.y,
                        to_abs.x,
                        to_abs.y
                    );
                    Ok(serde_json::json!({"ok": true, "frames": steps + 2}))
                })();
                if let Some(rf) = response_file {
                    let json = match &result {
                        Ok(v) => v.to_string(),
                        Err(msg) => serde_json::json!({"error": msg}).to_string(),
                    };
                    if let Err(e) = std::fs::write(rf, &json) {
                        log::error!("pane_ipc: drag_pane: could not write response file: {e}");
                    }
                }
            }
            crate::protocol::AppRequest::CapturePane {
                pane_id,
                lines,
                response_file,
                full_output,
                from_cursor,
            } => {
                log::info!(
                    "pane_ipc: kind=capture_pane pane_id={pane_id} lines={lines} full_output={full_output} from_cursor={from_cursor:?} response_file={:?}",
                    response_file
                );
                let result: Result<serde_json::Value, String> = match self
                    .windows
                    .iter()
                    .find_map(|win| win.panes.get(pane_id))
                {
                    None => {
                        log::warn!("pane_ipc: capture_pane: pane_id={pane_id} not found");
                        Err(format!("pane {pane_id} not found"))
                    }
                    Some(pane) => match pane.as_terminal() {
                        None => {
                            log::warn!(
                                "pane_ipc: capture_pane: pane_id={pane_id} is not a terminal pane"
                            );
                            Err(format!("pane {pane_id} is not a terminal pane"))
                        }
                        Some(term) => {
                            if let Some(banner) = term.standby_banner() {
                                let mut captured_lines = vec![banner];
                                if let Some(tail) = term
                                    .tail_preview
                                    .as_deref()
                                    .filter(|s| !s.trim().is_empty())
                                {
                                    captured_lines.extend(tail.lines().map(ToString::to_string));
                                }
                                Ok(serde_json::json!({
                                    "lines": captured_lines,
                                    "cursor": 0,
                                    "missed": false,
                                    "standby": true,
                                    "tail_preview": term.tail_preview,
                                }))
                            } else if let Some(cursor) = from_cursor {
                                let (mut captured_lines, new_cursor, missed) =
                                    term.backend.capture_lines_since(*cursor);
                                if !full_output {
                                    let trimmed = captured_lines
                                        .iter()
                                        .rposition(|l| !l.trim().is_empty())
                                        .map(|pos| pos + 1)
                                        .unwrap_or(0);
                                    captured_lines.truncate(trimmed);
                                }
                                log::info!(
                                    "pane_ipc: capture_pane: cursor={cursor} new_cursor={new_cursor} missed={missed} lines={}",
                                    captured_lines.len()
                                );
                                Ok(serde_json::json!({
                                    "lines": captured_lines,
                                    "cursor": new_cursor,
                                    "missed": missed,
                                    "standby": false,
                                    "tail_preview": term.tail_preview,
                                }))
                            } else {
                                let (mut captured, lw) =
                                    term.backend.capture_lines_with_cursor(*lines);
                                if !full_output {
                                    let trimmed = captured
                                        .iter()
                                        .rposition(|l| !l.trim().is_empty())
                                        .map(|pos| pos + 1)
                                        .unwrap_or(0);
                                    captured.truncate(trimmed);
                                    log::info!(
                                        "pane_ipc: capture_pane: stripped trailing empty lines, result len={}",
                                        captured.len()
                                    );
                                }
                                if term
                                    .agent
                                    .as_ref()
                                    .is_some_and(|a| a.state == crate::protocol::AgentState::Interrupted)
                                {
                                    if let Some(tail) = term
                                        .tail_preview
                                        .as_deref()
                                        .filter(|s| !s.trim().is_empty())
                                    {
                                        let first_tail_line = tail.lines().next().unwrap_or("");
                                        if !first_tail_line.is_empty()
                                            && !captured.iter().any(|l| l.contains(first_tail_line))
                                        {
                                            let mut with_tail: Vec<String> =
                                                tail.lines().map(ToString::to_string).collect();
                                            with_tail.extend(captured);
                                            captured = with_tail;
                                        }
                                    }
                                }
                                log::info!(
                                    "pane_ipc: capture_pane: lines={} cursor={lw}",
                                    captured.len()
                                );
                                Ok(serde_json::json!({
                                    "lines": captured,
                                    "cursor": lw,
                                    "missed": false,
                                    "standby": false,
                                    "tail_preview": term.tail_preview,
                                }))
                            }
                        }
                    },
                };
                let json_str = match result {
                    Ok(v) => serde_json::to_string(&v).unwrap_or_else(|_| {
                        r#"{"lines":[],"cursor":0,"missed":false}"#.to_string()
                    }),
                    Err(msg) => serde_json::json!({"error": msg}).to_string(),
                };
                write_response(response_file, json_str.as_bytes());
            }
            crate::protocol::AppRequest::PaneStatus {
                pane_id,
                response_file,
            } => {
                log::info!(
                    "pane_ipc: kind=pane_status pane_id={pane_id} response_file={response_file:?}"
                );
                let result = self
                    .windows
                    .iter()
                    .find_map(|window| window.panes.get(pane_id))
                    .ok_or_else(|| format!("pane {pane_id} not found"))
                    .and_then(|pane| {
                        let terminal = pane
                            .as_terminal()
                            .ok_or_else(|| format!("pane {pane_id} is not a terminal pane"))?;
                        let captured = terminal
                            .backend
                            .capture_lines(crate::app::pane_status::capture_depth());
                        let mut status =
                            crate::app::pane_status::composite_status(pane.agent(), &captured);
                        status["heartbeat"] = self.pane_heartbeat_json(*pane_id);
                        Ok(status)
                    });
                let json = match result {
                    Ok(status) => {
                        log::info!(
                            "pane_ipc: pane_status: pane_id={pane_id} verdict={} confidence={}",
                            status["verdict"],
                            status["confidence"]
                        );
                        status
                    }
                    Err(error) => {
                        log::warn!("pane_ipc: pane_status: {error}");
                        serde_json::json!({"error": error})
                    }
                };
                write_json_response(response_file, json);
            }
            crate::protocol::AppRequest::Screenshot {
                pane_id,
                output_path,
                response_file,
            } => {
                log::info!(
                    "pane_ipc: kind=screenshot pane_id={pane_id:?} output_path={output_path}"
                );
                // Validate a pane target up front so the CLI fails fast on a
                // bad id; the rect itself is resolved at capture time.
                let unknown_pane =
                    pane_id.is_some_and(|id| self.find_pane_in_any_window(id).is_none());
                if unknown_pane {
                    let response = serde_json::json!({
                        "error": format!("pane {} not found", pane_id.unwrap_or_default())
                    });
                    write_json_response(response_file, response);
                } else {
                    self.pending_screenshots
                        .push(crate::app::screenshot::PendingScreenshot {
                            pane_id: *pane_id,
                            output_path: output_path.clone(),
                            response_file: response_file.clone(),
                            capture_requested_at: None,
                            expires_at: std::time::Instant::now()
                                + crate::app::screenshot::SCREENSHOT_DEADLINE,
                        });
                    self.ctx.request_repaint_of(egui::ViewportId::ROOT);
                }
            }
            crate::protocol::AppRequest::GetPaneState {
                pane_id,
                response_file,
                stale_after_secs,
            } => {
                log::info!(
                    "pane_ipc: kind=get_pane_state pane_id={pane_id} response_file={response_file:?} stale_after_secs={stale_after_secs:?}"
                );
                let json_str = match self.windows.iter().find_map(|win| win.panes.get(pane_id)) {
                    None => {
                        log::warn!("pane_ipc: get_pane_state: pane_id={pane_id} not found");
                        serde_json::json!({"error": format!("pane {pane_id} not found")})
                            .to_string()
                    }
                    Some(pane) => {
                        let liveness = crate::host::pane_liveness::compute(pane, *stale_after_secs);
                        let mut response = if let Some(app_pane) = pane.as_app() {
                            if matches!(app_pane.runtime, crate::host::pane::AppRuntime::Builtin(_)) {
                                // Core pane inspection needs the rendered tree even when
                                // experimental eager accessibility capture is off. This
                                // enables internal capture, not the native OS adapter.
                                self.ctx.enable_accesskit();
                                self.ctx.request_repaint();
                            }
                            let frame = app_pane
                                .runtime
                                .frame_json()
                                .unwrap_or(serde_json::Value::Array(vec![]));
                            let semantic = app_pane.semantic_state();
                            let app_state = app_pane.runtime.semantic_details();
                            let (lifecycle, guest_error) = app_pane.runtime.lifecycle();
                            log::info!(
                                "pane_ipc: get_pane_state: pane_id={pane_id} lifecycle={lifecycle} runtime={} schema_version={} node_count={}",
                                app_pane.runtime.runtime_kind(),
                                semantic.schema_version,
                                semantic.nodes.len(),
                            );
                            serde_json::json!({
                                "pane_id": pane_id,
                                "type": "app",
                                "title": app_pane.name,
                                "manifest_id": app_pane.manifest_id,
                                "lifecycle": lifecycle,
                                // The CLI transport envelope signals failure
                                // with a top-level `error` string. Guest-domain
                                // failure detail therefore lives one level down
                                // in a nested object, where no envelope check
                                // on top-level scalars can ever read it.
                                "failure": guest_error.map(|error| serde_json::json!({ "error": error })),
                                "frame": frame,
                                "semantic": semantic,
                                "app_state": app_state,
                            })
                        } else if let Some(term) = pane.as_terminal() {
                            let title = term.name.clone().unwrap_or_else(|| "terminal".to_string());
                            serde_json::json!({
                                "pane_id": pane_id,
                                "type": "terminal",
                                "title": title,
                            })
                        } else {
                            serde_json::json!({
                                "pane_id": pane_id,
                                "type": "unknown",
                            })
                        };
                        // Every branch above carries the same two liveness
                        // fields, merged in exactly once here — the only way
                        // to reach either field is through this one call, so
                        // reading the claim without the observation (or vice
                        // versa) is not expressible.
                        response["claimed_state"] = liveness["claimed_state"].clone();
                        response["observed_state"] = liveness["observed_state"].clone();
                        response["stale_claim"] = liveness["stale_claim"].clone();
                        response.to_string()
                    }
                };
                write_response(response_file, json_str.as_bytes());
            }
            crate::protocol::AppRequest::SendAppAction {
                pane_id,
                action,
                args,
                response_file,
            } => {
                log::info!(
                    "pane_ipc: kind=send_app_action pane_id={pane_id} action={action:?} args={args:?}"
                );
                let approval_verb = {
                    let name = action.to_ascii_lowercase();
                    ["allow", "approve", "resolve", "grant", "deny", "revoke"]
                        .iter()
                        .any(|word| name == *word || name.contains(word))
                };
                if approval_verb
                    && self.refuse_synthetic_approval(
                        *pane_id,
                        None,
                        None,
                        response_file.as_deref(),
                    )
                {
                    // The surface itself refused.
                } else if approval_verb {
                    let pending_id = self
                        .synthetic_approval_target(*pane_id, None, None)
                        .or_else(|| {
                            crate::broker::gate::PermissionMonitor::for_profile(
                                &crate::config::config_dir(),
                            )
                            .list_pending()
                            .into_iter()
                            .next()
                            .map(|row| row.pending_request_id)
                        })
                        .unwrap_or_else(|| "none".to_string());
                    crate::broker::gate::PermissionMonitor::for_profile(
                        &crate::config::config_dir(),
                    )
                    .refuse_client_resolve(&pending_id);
                    log::info!(
                        "permission_monitor: refused synthetic action pane={pane_id} action={action} pending={pending_id}"
                    );
                    if let Some(path) = response_file.as_deref() {
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
                } else {
                let result = match self
                    .windows
                    .iter_mut()
                    .find_map(|win| win.panes.get_mut(pane_id))
                {
                    None => {
                        log::warn!("pane_ipc: send_app_action: pane_id={pane_id} not found");
                        Err(format!("pane {pane_id} not found"))
                    }
                    Some(pane) => {
                        if let Some(app_pane) = pane.as_app_mut() {
                            app_pane
                                .runtime
                                .send_app_action(action.clone(), args.clone())
                        } else {
                            log::warn!(
                                "pane_ipc: send_app_action: pane_id={pane_id} is not an app pane"
                            );
                            Err(format!("pane {pane_id} is not an app pane"))
                        }
                    }
                };
                if let Some(rf) = response_file {
                    let json = match &result {
                        Ok(()) => serde_json::json!({"ok": true}).to_string(),
                        Err(msg) => serde_json::json!({"error": msg}).to_string(),
                    };
                    write_response(rf, json.as_bytes());
                }
                }
            }
            crate::protocol::AppRequest::CallAppTool {
                app_id,
                tool,
                input_json,
                caller_pane_id,
                call_credential,
                peer_ancestry,
                target_pane_id,
                response_file,
                wait_approved,
            } => {
                self.call_app_tool(
                    app_id.clone(),
                    tool.clone(),
                    input_json.clone(),
                    *caller_pane_id,
                    call_credential.clone(),
                    peer_ancestry.clone(),
                    *target_pane_id,
                    response_file.clone(),
                    *wait_approved,
                );
            }
            crate::protocol::AppRequest::ListAppTools {
                app_id,
                context_id,
                caller_pane_id,
                caller_context_id,
                peer_ancestry,
                response_file,
            } => {
                self.list_app_tools(
                    app_id.as_deref(),
                    *context_id,
                    *caller_pane_id,
                    *caller_context_id,
                    peer_ancestry,
                    response_file,
                );
            }
            crate::protocol::AppRequest::SetAgentState {
                pane_id,
                state,
                agent,
                detail,
                session_id,
                event,
                blocked_reason,
            } => {
                log::info!(
                    "pane_ipc: kind=set_agent_state pane_id={pane_id} agent={agent} state={state:?} detail_present={}",
                    detail.is_some()
                );
                let mut found = false;
                let mut dirty = false;
                let mut has_parent_pane = false;
                for win in &mut self.windows {
                    if let Some(pane) = win.panes.get_mut(pane_id) {
                        if let Some(t) = pane.as_terminal_mut() {
                            has_parent_pane = t.parent_pane_id.is_some();
                            if let Some(sid) = session_id.as_ref().filter(|s| !s.is_empty()) {
                                if t.agent_session_id.as_ref() != Some(sid) {
                                    t.agent_session_id = Some(sid.clone());
                                    dirty = true;
                                }
                            }
                            if t.agent_harness.is_none() {
                                if let Some(h) = crate::cli::AgentHarness::parse(agent) {
                                    t.agent_harness = Some(h.as_str().to_string());
                                    dirty = true;
                                }
                            }
                            let new_reason = if matches!(
                                state,
                                crate::protocol::AgentState::Blocked
                                    | crate::protocol::AgentState::NeedsYou
                            ) {
                                *blocked_reason
                            } else {
                                None
                            };
                            if t.last_blocked_reason != new_reason {
                                t.last_blocked_reason = new_reason;
                                dirty = true;
                            }
                            if *state == crate::protocol::AgentState::Completed && !t.agent_completed
                            {
                                t.agent_completed = true;
                                dirty = true;
                            }
                        }
                        let (set_ok, state_dirty) =
                            pane.set_agent_with_dirty(Some(crate::protocol::PaneAgentState {
                                pane_id: *pane_id,
                                state: state.clone(),
                                agent: agent.clone(),
                                detail: detail.clone(),
                                session_id: session_id.clone(),
                            }));
                        found = set_ok;
                        dirty |= state_dirty;
                        break;
                    }
                }
                if self.fleet_focused_pane() == Some(*pane_id) { self.mark_agent_seen(*pane_id); }
                if dirty {
                    self.mark_workspace_dirty();
                }
                if found {
                    let provenance = Provenance {
                        source: if event.is_some() { Source::Hook } else { Source::LegacyReport },
                        agent_label: agent.clone(), session_id: session_id.clone(), raw_event: event.clone(),
                    };
                    let fact = PaneLifecycleEvent::from_report(state, provenance.clone(), *blocked_reason);
                    let session_started = matches!(&fact, PaneLifecycleEvent::SessionStarted { .. });
                    let session_ended = matches!(&fact, PaneLifecycleEvent::SessionEnded { .. });
                    let boot_failed = session_ended || matches!(&fact, PaneLifecycleEvent::TurnFailed { .. });
                    let idle = *state == crate::protocol::AgentState::Idle
                        && matches!(&fact, PaneLifecycleEvent::TurnFinished { .. } | PaneLifecycleEvent::SessionStarted { .. });
                    self.emit_pane_lifecycle(*pane_id, fact);
                    if idle {
                        self.emit_pane_lifecycle(*pane_id, PaneLifecycleEvent::AgentIdle { provenance: provenance.clone() });
                    }
                    if session_ended {
                        if let Some(tracked) = self.host.pane_lifecycle.get_mut(pane_id) {
                            tracked.booted = false;
                        }
                    } else if *state == crate::protocol::AgentState::Idle && (session_started || event.is_none()) {
                        self.emit_agent_booted(*pane_id, provenance);
                    }
                    self.refresh_portal_context_states();
                    log::info!("pane_ipc: set_agent_state: pane_id={pane_id} stored on pane");
                    // 2-tier escalation (stint 0802):
                    // Tier 1 (`AgentState::Blocked`) on a child worker pane with a `parent_pane_id`
                    // wakes the master agent via `PaneLifecycleEvent::AgentBlocked` without
                    // interrupting the human via `PermissionMonitor::file_needs_you`.
                    // Tier 2 (`AgentState::NeedsYou`, or `Blocked` on an unparented root agent)
                    // files a human-owned NeedsYou record.
                    let should_file_human_needs_you = match state {
                        crate::protocol::AgentState::NeedsYou => true,
                        crate::protocol::AgentState::Blocked => {
                            !has_parent_pane
                                && *blocked_reason
                                    != Some(crate::protocol::AgentBlockedReason::PermissionPrompt)
                        }
                        _ => false,
                    };
                    if should_file_human_needs_you {
                        let summary = detail.clone().filter(|text| !text.is_empty()).unwrap_or_else(|| {
                            format!("{agent} is blocked")
                        });
                        let kind = if *state == crate::protocol::AgentState::NeedsYou
                            || event.as_deref() == Some("AskQuestion")
                            || event.as_deref() == Some("AgentAsk")
                        {
                            crate::broker::gate::NeedsYouKind::Question
                        } else {
                            crate::broker::gate::NeedsYouKind::BlockedRun
                        };
                        let filed = crate::broker::gate::PermissionMonitor::for_profile(
                            &crate::config::config_dir(),
                        )
                        .file_needs_you(crate::broker::gate::NeedsYouFile {
                            kind,
                            actor: agent.clone(),
                            resource: format!("pane:{pane_id}"),
                            summary,
                            expires_at: None,
                            run_tag: session_id.clone().or_else(|| Some(format!("pane:{pane_id}"))),
                        });
                        if let Err(error) = filed {
                            log::info!("needs_you: could not file blocked pane {pane_id}: {error}");
                        }
                    }
                    // Fast path: answer a parked `pane new --agent` spawn the
                    // frame the hook report lands. The host-observed detector
                    // path is picked up by the level-triggered re-check in
                    // `service_pending_agent_boots` instead.
                    if boot_failed {
                        self.fail_agent_boots_from_terminal_report(*pane_id);
                    } else {
                        self.complete_agent_boots(*pane_id);
                    }
                } else {
                    log::warn!(
                        "pane_ipc: set_agent_state: pane_id={pane_id} not found or not agent-addressable"
                    );
                }
            }
            crate::protocol::AppRequest::SetPipStatus { pane_id, status } => {
                log::info!("pane_ipc: kind=set_pip_status pane_id={pane_id} status={status:?}");
                let mut found = false;
                for win in &mut self.windows {
                    if let Some(pane) = win.panes.get_mut(pane_id) {
                        found = pane.set_pip_status(Some(*status));
                        break;
                    }
                }
                if found {
                    log::info!("pane_ipc: set_pip_status: pane_id={pane_id} stored on pane");
                } else {
                    log::warn!(
                        "pane_ipc: set_pip_status: pane_id={pane_id} not found or not an app pane"
                    );
                }
            }
            crate::protocol::AppRequest::GetAgentStates { response_file } => {
                log::info!("pane_ipc: kind=get_agent_states response_file={response_file:?}");
                let states: Vec<&crate::protocol::PaneAgentState> = self
                    .windows
                    .iter()
                    .flat_map(|win| win.panes.values())
                    .filter_map(|pane| pane.agent())
                    .collect();
                let json_str = serde_json::to_string(&states).unwrap_or_else(|_| "[]".to_string());
                write_response(response_file, json_str.as_bytes());
            }
            crate::protocol::AppRequest::Notify {
                notify_id,
                title,
                body,
                kind,
                options,
                input_prompt,
                required,
                image_inline,
                image_pipe_id,
                timeout_secs,
                on_dismiss,
                response_file,
                scope,
                source_context_id,
                source_pane_id,
                peer_pid,
                ..
            } => {
                let internal_id = notify_id
                    .clone()
                    .unwrap_or_else(|| format!("__host__:{}", crate::platform::clock::now_nanos()));
                log::info!(
                    "pane_ipc: kind=notify title={:?} choices={} scope={:?} peer_pid={:?} response_file={:?}",
                    title,
                    options.len(),
                    scope,
                    peer_pid,
                    response_file
                );
                // Caller identity is resolved from the socket peer's OS
                // credential (`peer_pid`, host-established in
                // `handle_socket_connection`), never from dispatch-time
                // active state and never from the client-claimed
                // `source_context_id`/`source_pane_id` fields — those are
                // attacker-controlled and, before this fix, let any process
                // with socket access widen or misattribute a notification's
                // scope by lying about which pane sent it.
                let resolved = peer_pid
                    .as_deref()
                    .and_then(|ancestry| self.resolve_socket_peer_pane(ancestry));
                if let Some((resolved_pane_id, resolved_context_id, resolved_window_id)) = resolved
                {
                    log::info!(
                        "pane_ipc: peer identity: notify sender resolved to pane={resolved_pane_id} context={resolved_context_id} window={resolved_window_id} peer_pid={peer_pid:?}"
                    );
                }
                if source_pane_id.is_some() || source_context_id.is_some() {
                    let claimed_matches = resolved.is_some_and(|(pane_id, context_id, _)| {
                        source_pane_id.is_none_or(|claimed| claimed == pane_id)
                            && source_context_id.is_none_or(|claimed| claimed == context_id)
                    });
                    if !claimed_matches {
                        log::warn!(
                            "pane_ipc: notify: claimed identity (source_context_id={source_context_id:?} source_pane_id={source_pane_id:?}) \
                             disagrees with host-resolved sender {resolved:?} — using the host-resolved identity only"
                        );
                    }
                }
                let (caller_context_id, caller_window_id) = match resolved {
                    Some((_, context_id, window_id)) => (Some(context_id), Some(window_id)),
                    None => (None, None),
                };
                // Unscoped `plexi notify` belongs to the context that produced
                // it; `--scope global` is the opt-in. A context or window
                // scope with no resolvable sender (outside-pane caller, a
                // dead peer pid, or a peer that matches no live terminal
                // pane) escalates to global: showing the notification
                // everywhere is the only resolution that cannot hide it, and
                // attaching it to some other context would fabricate
                // provenance. `caller_context_id` and `caller_window_id` are
                // always both `Some` or both `None` — they come from the
                // same resolved `(pane_id, context_id, window_id)` tuple, so
                // there is no "context resolved but window didn't" case to
                // narrow separately; the pre-fix `source_context_id`-only
                // fallback that produced that case is exactly the forgeable
                // input this stint removes.
                let effective_scope = match scope.unwrap_or_default() {
                    crate::protocol::NotifyScope::Global => {
                        crate::protocol::NotifyScope::Global
                    }
                    crate::protocol::NotifyScope::Context if caller_context_id.is_some() => {
                        crate::protocol::NotifyScope::Context
                    }
                    crate::protocol::NotifyScope::Window if caller_window_id.is_some() => {
                        crate::protocol::NotifyScope::Window
                    }
                    unresolvable => {
                        log::warn!(
                            "pane_ipc: notify: scope {unresolvable:?} needs a caller identity but none resolved \
                             (source_context_id={source_context_id:?} source_pane_id={source_pane_id:?}) — \
                             escalating to global so the notification stays reachable"
                        );
                        crate::protocol::NotifyScope::Global
                    }
                };
                // The host-resolved sender pane, so `dismiss_notification_from_sender`
                // can check ownership against a value the requester never
                // controlled instead of re-parsing it out of `notify_id` (the
                // pre-fix behavior — `notify_id` is itself client-generated,
                // so that check was comparing two attacker-controlled values
                // against each other). 0 = no resolvable sender — the
                // notification can never be dismissed via `plexi notify
                // dismiss`, same as before this fix for any non-CLI-format
                // notify_id. Deliberately NOT `sender_pane_id`: that field
                // drives auto-dismiss-on-focus (`notifications.rs`), which
                // CLI notifications opt out of on purpose.
                let dismiss_owner_pane_id = resolved.map(|(pane_id, ..)| pane_id).unwrap_or(0);
                let is_invalid_choice = matches!(kind, crate::protocol::NotifyKind::Choice)
                    && options.is_empty();
                if is_invalid_choice {
                    log::warn!("pane_ipc: notify id={internal_id:?} has no choices; resolving explicitly");
                    self.deliver_notify_action(
                        0,
                        internal_id,
                        "invalid_choice".to_string(),
                        Some(crate::app::notifications::NOTIFY_OUTCOME_INVALID.to_string()),
                        response_file.clone(),
                        None,
                    );
                } else {
                self.enqueue_notification(
                    crate::app::notifications::NotifySource::Cli,
                    PendingNotification {
                        notify_id: internal_id,
                        dismiss_owner_pane_id,
                        // 0 = no context / no window, the same sentinel
                        // host-internal notifications use. Real ids start at 1.
                        source_context_id: caller_context_id.unwrap_or(0),
                        source_window_id: caller_window_id.unwrap_or(0),
                        scope: effective_scope,
                        title: title.clone(),
                        body: body.clone(),
                        kind: kind.clone(),
                        options: options.clone(),
                        input_prompt: input_prompt.clone(),
                        required: *required,
                        image_inline: image_inline.clone(),
                        image_pipe_id: image_pipe_id.clone(),
                        response_file: response_file.clone(),
                        timeout_secs: *timeout_secs,
                        on_dismiss: on_dismiss.clone(),
                        ..Default::default()
                    },
                );
                }
            }
            crate::protocol::AppRequest::DismissNotification {
                notify_id,
                source_context_id,
                source_pane_id,
                response_file,
                peer_pid,
            } => {
                // Same trust boundary as `Notify`: ownership is decided from
                // the socket peer's host-resolved pane, never from the
                // client-claimed `source_context_id`/`source_pane_id` — those
                // are attacker-controlled and, before this fix, were the
                // entire ownership check (compared against a pane id
                // re-parsed out of the equally client-generated `notify_id`).
                let resolved = peer_pid
                    .as_deref()
                    .and_then(|ancestry| self.resolve_socket_peer_pane(ancestry));
                if let Some((resolved_pane_id, resolved_context_id, resolved_window_id)) = resolved
                {
                    log::info!(
                        "pane_ipc: peer identity: dismiss sender resolved to pane={resolved_pane_id} context={resolved_context_id} window={resolved_window_id} peer_pid={peer_pid:?}"
                    );
                }
                if source_pane_id.is_some() || source_context_id.is_some() {
                    let claimed_matches = resolved.is_some_and(|(pane_id, context_id, _)| {
                        source_pane_id.is_none_or(|claimed| claimed == pane_id)
                            && source_context_id.is_none_or(|claimed| claimed == context_id)
                    });
                    if !claimed_matches {
                        log::warn!(
                            "pane_ipc: notify dismiss: claimed identity (source_context_id={source_context_id:?} source_pane_id={source_pane_id:?}) \
                             disagrees with host-resolved sender {resolved:?} — using the host-resolved identity only"
                        );
                    }
                }
                let result = match resolved {
                    Some((pane_id, _, _)) => self
                        .dismiss_notification_from_sender(notify_id, pane_id)
                        .map(|()| "dismissed"),
                    None => Err("notify dismiss requires a resolvable caller pane"),
                };
                match result {
                    Ok(message) => {
                        write_response(response_file, message.as_bytes());
                    }
                    Err(message) => {
                        log::warn!("pane_ipc: notify dismiss id={notify_id:?}: {message}");
                        write_response(response_file, message.as_bytes());
                    }
                }
            }
            crate::protocol::AppRequest::CreateContext {
                root,
                name,
                parent_name,
                parent_context_id,
                windows,
                focus,
                portal_direction,
                anchor_pane,
                response_file,
            } => {
                // Map direction string to insert_split_tile params.
                // vertical=true  → side-by-side (left/right)
                // vertical=false → stacked (up/down)
                // new_pane_first=true → portal appears before the existing pane (left/up)
                let (portal_vertical, portal_first) =
                    match portal_direction.as_deref().unwrap_or("right") {
                        "down" => (false, false),
                        "up" => (false, true),
                        "left" => (true, true),
                        _ => (true, false), // "right" is default
                    };
                log::info!(
                    "pane_ipc: kind=create_context root={:?} name={:?} parent_name={:?} \
                     parent_context_id={parent_context_id:?} windows={} focus={focus} \
                     direction={:?} anchor_pane={:?}",
                    root,
                    name,
                    parent_name,
                    windows.len(),
                    portal_direction,
                    anchor_pane
                );
                let mut ctx_ok = true;
                let requested_auto_name = name
                    .is_none()
                    .then(|| {
                        root.as_ref().and_then(|path| {
                            path.file_name()
                                .map(|component| component.to_string_lossy().into_owned())
                        })
                    })
                    .flatten();
                // Track which context was active before and which context was just created.
                // Windows must be placed in the new context regardless of focus state.
                let orig_ctx_id = self.router.active().context_id;
                let mut new_ctx_id = orig_ctx_id;
                if let Some(pname) = parent_name {
                    let path = root.as_ref().cloned().unwrap_or_else(|| {
                        let p = self.router.active().root.clone();
                        if p.is_absolute() {
                            p
                        } else {
                            dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/"))
                        }
                    });
                    let current_ctx_id = self.router.active().context_id;
                    let current_win_id = self.windows[self.active_window].window_id;
                    let current_focused = self.windows[self.active_window].focused_pane;
                    if let Err(e) =
                        self.new_child_context(crate::pane_ops::ChildContextSpec::single_terminal(
                            *parent_context_id,
                            pname.clone(),
                            path,
                            portal_vertical,
                            portal_first,
                            *anchor_pane,
                        ))
                    {
                        log::warn!("pane_ipc: create_context with parent failed: {e}");
                        ctx_ok = false;
                    } else {
                        let idx = self.router.len() - 1;
                        if let Some(n) = name {
                            self.router.get_mut(idx).name =
                                crate::host::context::ContextName::custom(n);
                        } else if let Some(path) = root {
                            let context_id = self.router.get(idx).context_id;
                            let auto_name = self.auto_context_name_for_path(context_id, path);
                            self.router.get_mut(idx).name =
                                crate::host::context::ContextName::auto(auto_name);
                        }
                        let new_ctx_idx = self.router.len() - 1;
                        new_ctx_id = self.router.get(new_ctx_idx).context_id;
                        if *focus {
                            self.router
                                .push_depth(current_ctx_id, current_win_id, current_focused);
                            self.switch_workspace(new_ctx_idx);
                            log::info!(
                                "pane_ipc: zoomed into new child context ctx_id={new_ctx_id}"
                            );
                        } else {
                            log::info!(
                                "pane_ipc: created child context ctx_id={new_ctx_id} (no-focus)"
                            );
                        }
                    }
                } else {
                    if let Some(r) = root {
                        self.new_context_at_path(r.clone());
                    } else {
                        self.new_context();
                    }
                    if let Some(n) = name {
                        let idx = self.router.len() - 1;
                        self.router.get_mut(idx).name =
                            crate::host::context::ContextName::custom(n);
                    }
                    new_ctx_id = self.router.get(self.router.len() - 1).context_id;
                }
                if ctx_ok && !windows.is_empty() {
                    let active_y = self
                        .windows
                        .iter()
                        .find(|w| w.context_id == new_ctx_id)
                        .map(|w| w.grid_y)
                        .unwrap_or_else(|| self.windows[self.active_window].grid_y);
                    let first_x = self
                        .windows
                        .iter()
                        .filter(|w| w.context_id == new_ctx_id && w.grid_y == active_y)
                        .map(|w| w.grid_x)
                        .max()
                        .map(|x| x + 1)
                        .unwrap_or(1);
                    for (new_x, cmd) in (first_x..).zip(windows) {
                        log::info!(
                            "pane_ipc: create_context window ctx_id={new_ctx_id} grid_x={new_x} cmd={cmd:?}"
                        );
                        self.create_page_at(
                            new_x,
                            active_y,
                            new_ctx_id,
                            crate::pane_ops::create::TerminalSpawnOptions {
                                initial_cmd: Some(cmd.as_str()),
                                ..Default::default()
                            },
                        );
                    }
                }
                // Write JSON response for callers that passed a response_file path.
                if ctx_ok {
                    if let Some(rf) = response_file {
                        let windows_info: Vec<serde_json::Value> = self
                            .windows
                            .iter()
                            .filter(|w| w.context_id == new_ctx_id)
                            .map(|w| {
                                serde_json::json!({
                                    "window_id": w.window_id,
                                    "grid_x": w.grid_x,
                                    "grid_y": w.grid_y,
                                })
                            })
                            .collect();
                        let created_context = self
                            .router
                            .iter()
                            .find(|context| context.context_id == new_ctx_id);
                        let created_name = created_context
                            .map(|context| context.name.displayed())
                            .unwrap_or_default();
                        let auto_name_collision_with = created_context
                            .filter(|context| {
                                matches!(context.name, crate::host::context::ContextName::Auto(_))
                            })
                            .and(requested_auto_name.as_deref())
                            .filter(|base| created_name != *base);
                        let response = serde_json::json!({
                            "context_id": new_ctx_id,
                            "windows": windows_info,
                            "name": created_name,
                            "auto_name_collision_with": auto_name_collision_with,
                        });
                        write_json_response(rf, response);
                    }
                }
                self.mark_workspace_dirty();
            }
            crate::protocol::AppRequest::CreateSubContext {
                name,
                root,
                parent_context_id,
                parent_name,
                panes,
                layout,
                focus,
                anchor_pane,
                response_file,
            } => {
                // Resolve the parent up front: everything below (depth push,
                // window bookkeeping) is relative to it, and a bad parent must
                // fail before any pane is spawned.
                let parent_idx = self.resolve_parent_context(
                    *parent_context_id,
                    parent_name.as_deref().unwrap_or_default(),
                );
                let Some(parent_idx) = parent_idx else {
                    log::warn!(
                        "pane_ipc: create_sub_context — no parent context for id={parent_context_id:?} name={parent_name:?}"
                    );
                    if let Some(rf) = response_file {
                        write_json_response(
                            rf,
                            serde_json::json!({
                                "error": format!(
                                    "no parent context for id={parent_context_id:?} name={parent_name:?}"
                                ),
                            }),
                        );
                    }
                    return;
                };
                let parent_ctx_id = self.router.get(parent_idx).context_id;
                let resolved_layout = layout.unwrap_or_else(|| {
                    crate::pane_ops::default_squad_layout(panes.len())
                });
                log::info!(
                    "pane_ipc: kind=create_sub_context name={name:?} parent_ctx_id={parent_ctx_id} \
                     panes={} requested_layout={layout:?} layout={resolved_layout:?} focus={focus} \
                     root={} anchor_pane={anchor_pane:?}",
                    panes.len(),
                    root.display()
                );
                let spec = crate::pane_ops::ChildContextSpec {
                    parent_id: Some(parent_ctx_id),
                    parent_name: parent_name.clone().unwrap_or_default(),
                    // Named up front, not renamed after: the squad's panes are
                    // spawned with this in PLEXI_CONTEXT_NAME.
                    name: Some(name.clone()),
                    path: root.clone(),
                    // A squad reads best beside the caller, matching the
                    // `context new --parent` default.
                    portal_vertical: true,
                    portal_first: false,
                    anchor_pane: *anchor_pane,
                    panes: panes.clone(),
                    layout: resolved_layout,
                };
                let child = match self.new_child_context(spec) {
                    Ok(child) => child,
                    Err(e) => {
                        log::warn!("pane_ipc: create_sub_context failed: {e}");
                        if let Some(rf) = response_file {
                            write_json_response(rf, serde_json::json!({ "error": e }));
                        }
                        return;
                    }
                };
                let child_idx = self.router.len() - 1;
                // The stint-stipulated trace: parent id, child id, pane count,
                // and the command each pane launched. Pairs pane_id → cmd so a
                // "wrong agent got the wrong job" report is answerable from the
                // log alone, without re-running anything.
                let launched = launched_pairs(&child.pane_ids, panes);
                log::info!(
                    "context_sub: parent_ctx_id={parent_ctx_id} child_ctx_id={} name={name:?} \
                     pane_count={} layout={resolved_layout:?} root={} launched={launched:?}",
                    child.context_id,
                    child.pane_ids.len(),
                    root.display()
                );
                if *focus {
                    // Zoom-out must land back on the *caller's* window, which is
                    // not necessarily the globally active one — a background
                    // pane can create a squad while the user is elsewhere.
                    match child.parent_window_id {
                        Some(win_id) => {
                            self.router.push_depth(
                                parent_ctx_id,
                                win_id,
                                child.parent_focused_pane,
                            );
                            self.switch_workspace(child_idx);
                            log::info!(
                                "pane_ipc: zoomed into new sub-context ctx_id={} (return win_id={win_id})",
                                child.context_id
                            );
                        }
                        None => {
                            log::warn!(
                                "pane_ipc: create_sub_context --focus ignored — parent ctx_id={parent_ctx_id} has no window to return to"
                            );
                        }
                    }
                }
                if let Some(rf) = response_file {
                    let windows_info: Vec<serde_json::Value> = self
                        .windows
                        .iter()
                        .filter(|w| w.context_id == child.context_id)
                        .map(|w| {
                            serde_json::json!({
                                "window_id": w.window_id,
                                "grid_x": w.grid_x,
                                "grid_y": w.grid_y,
                            })
                        })
                        .collect();
                    write_json_response(
                        rf,
                        serde_json::json!({
                            "context_id": child.context_id,
                            "windows": windows_info,
                            "panes": child.pane_ids,
                        }),
                    );
                }
                self.mark_workspace_dirty();
            }
            crate::protocol::AppRequest::FocusContext { root } => {
                log::warn!(
                    "pane_ipc: FocusContext ignored — CWD-based auto-switch removed (root={})",
                    root.display()
                );
            }
            crate::protocol::AppRequest::SetContextRoot { root, context_id } => {
                log::info!(
                    "pane_ipc: kind=set_context_root root={} context_id={context_id:?}",
                    root.display()
                );
                // Standing ruling: every path that establishes a context root
                // ensures app_states/ is gitignored there (personal local
                // data, never committed). Non-fatal; guarded so a bad root
                // never gains directories as a side effect.
                if root.is_dir() {
                    if let Err(error) = crate::workspace::secrets::ensure_app_state_gitignore(root)
                    {
                        log::warn!(
                            "could not ensure {}/.plexi/.gitignore covers app_states/: {error}",
                            root.display()
                        );
                    }
                }
                self.set_context_root(root.clone(), *context_id);
                self.mark_workspace_dirty();
            }
            crate::protocol::AppRequest::SetContextDescription {
                description,
                context_id,
            } => {
                log::info!("pane_ipc: kind=set_context_description context_id={context_id:?}");
                let idx = self.resolve_context_idx(*context_id, "set_context_description");
                let trimmed = description.trim().to_string();
                self.router.get_mut(idx).description = if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed)
                };
                self.mark_workspace_dirty();
            }
            crate::protocol::AppRequest::ZoomIntoContext {
                context_id,
                response_file,
            } => {
                log::info!(
                    "pane_ipc: kind=zoom_into_context context_id={context_id} response_file={response_file:?}"
                );
                if let Some(ctx_idx) = self.router.position(|c| c.context_id == *context_id) {
                    let current_ctx_id = self.router.active().context_id;
                    let current_win_id = self.windows[self.active_window].window_id;
                    let current_focused = self.windows[self.active_window].focused_pane;
                    self.router
                        .push_depth(current_ctx_id, current_win_id, current_focused);
                    self.switch_workspace(ctx_idx);
                    if let Some(rf) = response_file {
                        write_json_response(rf, serde_json::json!({ "context_id": context_id }));
                    }
                } else {
                    log::warn!("pane_ipc: zoom_into_context: no context with id {context_id}");
                    if let Some(rf) = response_file {
                        write_json_response(
                            rf,
                            serde_json::json!({
                                "error": format!(
                                    "no context with id {context_id} — run `plexi context list` to see live ids"
                                )
                            }),
                        );
                    }
                }
            }
            crate::protocol::AppRequest::ZoomOutOfContext => {
                log::info!(
                    "pane_ipc: kind=zoom_out_of_context depth={}",
                    self.router.current_depth()
                );
                self.zoom_out_of_context();
            }
            crate::protocol::AppRequest::PushPaneToSubcontext { name, pane_id } => {
                log::info!(
                    "pane_ipc: kind=push_pane_to_subcontext name={name:?} pane_id={pane_id:?}"
                );
                self.push_pane_to_subcontext(name.clone(), *pane_id);
            }
            crate::protocol::AppRequest::ListPermissions { response_file } => {
                log::info!("pane_ipc: kind=list_permissions response_file={response_file:?}");
                self.handle_list_permissions(response_file);
            }
            crate::protocol::AppRequest::SetPermission {
                app_id,
                workspace,
                capability,
                state,
                response_file,
            } => {
                log::info!(
                    "pane_ipc: kind=set_permission app_id={app_id} workspace={workspace:?} \
                     capability={capability} state={state} response_file={response_file:?}"
                );
                self.handle_set_permission(
                    app_id,
                    workspace.as_deref(),
                    capability,
                    state,
                    response_file,
                );
            }
            crate::protocol::AppRequest::AgentSpawn {
                role,
                harness,
                command,
                task,
                subcontext,
                ephemeral,
                parent_pane_id,
                name,
                cwd,
                worktree: _,
                result_schema,
                initial_slots,
                no_focus: _,
                response_file,
            } => {
                log::info!(
                    "pane_ipc: kind=agent_spawn role={role} harness={harness} subcontext={subcontext:?} ephemeral={ephemeral} parent_pane_id={parent_pane_id:?}"
                );
                let result = self.spawn_agent_worker_pane(
                    role,
                    harness,
                    command,
                    task.as_deref().unwrap_or_default(),
                    subcontext.as_deref(),
                    *ephemeral,
                    *parent_pane_id,
                    cwd.as_deref().map(std::path::PathBuf::from),
                    result_schema.clone(),
                    None,
                );
                let payload = match result {
                    Ok((new_pane_id, target_ctx_id)) => {
                        if let Some(ref custom_name) = name {
                            if !custom_name.is_empty() {
                                self.apply_inline_pane_name(new_pane_id, custom_name);
                            }
                        }
                        for (slot_key, slot_val) in initial_slots {
                            let _ = self.write_pane_slot_internal(
                                new_pane_id,
                                slot_key,
                                slot_val.as_bytes(),
                                false,
                                true,
                            );
                        }
                        let portal_pane_id = self
                            .windows
                            .iter()
                            .flat_map(|w| w.panes.values())
                            .find_map(|p| match p {
                                crate::host::pane::Pane::Portal(portal)
                                    if portal.target_context_id == target_ctx_id =>
                                {
                                    Some(portal.pane_id)
                                }
                                _ => None,
                            });
                        serde_json::json!({
                            "ok": true,
                            "pane_id": new_pane_id,
                            "context_id": target_ctx_id,
                            "context_name": self.context_name_for(target_ctx_id),
                            "portal_pane_id": portal_pane_id,
                            "role": role,
                            "harness": harness,
                            "command": command,
                        })
                    }
                    Err(e) => serde_json::json!({ "ok": false, "error": e }),
                };
                write_json_response(response_file, payload);
            }
            crate::protocol::AppRequest::AgentDone {
                pane_id,
                result,
                summary,
                response_file,
            } => {
                let raw_result = result.as_deref().unwrap_or("{}");
                let summary_str = summary.as_deref().unwrap_or_default();
                log::info!(
                    "pane_ipc: kind=agent_done pane_id={pane_id} result_len={} summary_len={}",
                    raw_result.len(),
                    summary_str.len()
                );
                let pane_alive = self.find_pane_in_any_window(*pane_id).is_some();
                if !pane_alive && !self.pane_results.contains_key(pane_id) {
                    write_json_response(
                        response_file,
                        serde_json::json!({
                            "ok": false,
                            "pane_id": pane_id,
                            "error_code": "pane_not_found",
                            "error": format!("pane {pane_id} not found"),
                        }),
                    );
                    return;
                }
                match self
                    .pane_results
                    .entry(*pane_id)
                    .or_default()
                    .write_payload(raw_result)
                {
                    Ok(committed) => {
                        self.complete_result_waits(*pane_id);
                        let completed_payload = serde_json::json!({
                            "pane_id": pane_id,
                            "status": "completed",
                            "summary": summary_str,
                            "result": committed,
                        })
                        .to_string();
                        if !summary_str.is_empty() {
                            let _ = self.write_pane_slot_internal(
                                *pane_id,
                                "summary",
                                summary_str.as_bytes(),
                                false,
                                true,
                            );
                        }
                        let slot_res = self.write_pane_slot_internal(
                            *pane_id,
                            "completed",
                            completed_payload.as_bytes(),
                            false,
                            true,
                        );
                        let mut agent_label = "agent".to_string();
                        let mut session_id = None;
                        for win in &mut self.windows {
                            if let Some(pane) = win.panes.get_mut(pane_id) {
                                if let Some(t) = pane.as_terminal_mut() {
                                    t.agent_completed = true;
                                    t.fleet_result = Some(committed.clone());
                                    t.fleet_done_unseen = true;
                                    t.fleet_seen = false;
                                    t.fleet_since = crate::platform::clock::now_secs();
                                    t.last_blocked_reason = None;
                                    if let Some(ref h) = t.agent_harness {
                                        agent_label = h.clone();
                                    } else if let Some(ref a) = t.agent {
                                        agent_label = a.agent.clone();
                                    }
                                    session_id = t.agent_session_id.clone();
                                }
                                let _ = pane.set_agent(Some(crate::protocol::PaneAgentState {
                                    pane_id: *pane_id,
                                    state: crate::protocol::AgentState::Idle,
                                    agent: agent_label.clone(),
                                    detail: (!summary_str.is_empty())
                                        .then(|| summary_str.to_string()),
                                    session_id: session_id.clone(),
                                }));
                                break;
                            }
                        }
                        self.report_agent_attention(*pane_id, &crate::protocol::AgentState::Completed, None);
                        if self.fleet_focused_pane() == Some(*pane_id) { self.mark_agent_seen(*pane_id); }
                        self.mark_workspace_dirty();
                        let provenance = Provenance {
                            source: Source::LegacyReport,
                            agent_label,
                            session_id,
                            raw_event: Some("AgentDone".to_string()),
                        };
                        self.emit_pane_lifecycle(
                            *pane_id,
                            PaneLifecycleEvent::TurnFinished {
                                provenance: provenance.clone(),
                            },
                        );
                        self.emit_pane_lifecycle(
                            *pane_id,
                            PaneLifecycleEvent::AgentIdle { provenance },
                        );
                        self.refresh_portal_context_states();
                        let payload = match slot_res {
                            Ok((path, bytes)) => serde_json::json!({
                                "ok": true,
                                "pane_id": pane_id,
                                "slot": "completed",
                                "path": path.to_string_lossy(),
                                "bytes": bytes,
                                "result": committed,
                                "summary": summary,
                            }),
                            Err(e) => serde_json::json!({
                                "ok": true,
                                "pane_id": pane_id,
                                "result": committed,
                                "summary": summary,
                                "slot_warning": e,
                            }),
                        };
                        write_json_response(response_file, payload);
                    }
                    Err(err) => {
                        log::warn!(
                            "pane_ipc: agent_done validation failed for pane {pane_id}: {}",
                            err.message()
                        );
                        write_json_response(
                            response_file,
                            serde_json::json!({
                                "ok": false,
                                "pane_id": pane_id,
                                "error_code": err.error_code(),
                                "error": err.message(),
                                "validation_errors": err.validation_errors(),
                            }),
                        );
                    }
                }
            }
            crate::protocol::AppRequest::AgentNext { response_file } => {
                let pane_id = self.next_blocked_agent();
                write_json_response(response_file, serde_json::json!({"pane_id": pane_id}));
            }
            crate::protocol::AppRequest::AgentFleet { filter, response_file } => {
                log::info!("agents: fleet query filter={filter:?}");
                write_json_response(response_file, serde_json::json!(self.agents_view(*filter)));
            }
            crate::protocol::AppRequest::AgentAsk {
                pane_id,
                question,
                response_file,
            } => {
                log::info!(
                    "pane_ipc: kind=agent_ask pane_id={pane_id} question_len={}",
                    question.len()
                );
                let mut found = false;
                let mut parent_pane_id = None;
                let mut agent_label = "agent".to_string();
                let mut session_id = None;
                for win in &mut self.windows {
                    if let Some(pane) = win.panes.get_mut(pane_id) {
                        if let Some(t) = pane.as_terminal_mut() {
                            parent_pane_id = t.parent_pane_id;
                            t.last_blocked_reason =
                                Some(crate::protocol::AgentBlockedReason::Unknown);
                            if let Some(ref h) = t.agent_harness {
                                agent_label = h.clone();
                            } else if let Some(ref a) = t.agent {
                                agent_label = a.agent.clone();
                            }
                            session_id = t.agent_session_id.clone();
                        }
                        found = pane.set_agent(Some(crate::protocol::PaneAgentState {
                            pane_id: *pane_id,
                            state: crate::protocol::AgentState::Blocked,
                            agent: agent_label.clone(),
                            detail: Some(question.clone()),
                            session_id: session_id.clone(),
                        }));
                        break;
                    }
                }
                if found {
                    self.mark_workspace_dirty();
                    let question_payload = serde_json::json!({
                        "pane_id": pane_id,
                        "parent_pane_id": parent_pane_id,
                        "question": question,
                    })
                    .to_string();
                    let _ = self.write_pane_slot_internal(
                        *pane_id,
                        "question",
                        question_payload.as_bytes(),
                        false,
                        true,
                    );
                    let provenance = Provenance {
                        source: Source::Hook,
                        agent_label: agent_label.clone(),
                        session_id: session_id.clone(),
                        raw_event: Some("AgentAsk".to_string()),
                    };
                    self.emit_pane_lifecycle(
                        *pane_id,
                        PaneLifecycleEvent::AgentBlocked {
                            provenance,
                            reason: crate::protocol::AgentBlockedReason::Unknown,
                        },
                    );
                    self.refresh_portal_context_states();
                    // Tier 1 escalation: if this pane has a `parent_pane_id`, wake the master
                    // agent via `PaneLifecycleEvent::AgentBlocked` (and its `question` slot)
                    // without interrupting the human. Only file human `NeedsYou` when unparented.
                    if parent_pane_id.is_none() {
                        let _ = crate::broker::gate::PermissionMonitor::for_profile(
                            &crate::config::config_dir(),
                        )
                        .file_needs_you(crate::broker::gate::NeedsYouFile {
                            kind: crate::broker::gate::NeedsYouKind::Question,
                            actor: agent_label,
                            resource: format!("pane:{pane_id}"),
                            summary: question.clone(),
                            expires_at: None,
                            run_tag: session_id.or_else(|| Some(format!("pane:{pane_id}"))),
                        });
                    }
                    let payload = serde_json::json!({
                        "ok": true,
                        "pane_id": pane_id,
                        "parent_pane_id": parent_pane_id,
                        "state": "blocked",
                        "question": question,
                    });
                    write_json_response(response_file, payload);
                } else {
                    let payload = serde_json::json!({
                        "ok": false,
                        "error": format!("pane {pane_id} not found or not a terminal"),
                    });
                    write_json_response(response_file, payload);
                }
            }
            crate::protocol::AppRequest::AgentReopen {
                query,
                subcontext,
                response_file,
            } => {
                log::info!(
                    "pane_ipc: kind=agent_reopen query={query} subcontext={subcontext:?}"
                );
                let history = crate::workspace::load_agent_history();
                let matched = history.into_iter().rev().find(|r| {
                    r.run_id == *query || r.session_id.as_deref() == Some(query.as_str())
                });
                let Some(record) = matched else {
                    write_json_response(
                        response_file,
                        serde_json::json!({
                            "ok": false,
                            "error_code": "run_not_found",
                            "error": format!("no closed agent run or session matching '{query}' in agent_history.jsonl"),
                        }),
                    );
                    return;
                };
                let resume_cmd = match record.session_id.as_deref() {
                    Some(sid) if !sid.trim().is_empty() => {
                        if let Some(adapter) =
                            crate::cli::agent::harness_adapter(&record.harness)
                        {
                            adapter.resume_command(sid)
                        } else {
                            format!("{} --resume {}", record.harness, sid.trim())
                        }
                    }
                    _ => record.harness.clone(),
                };
                let role = record
                    .config_name
                    .clone()
                    .or_else(|| record.pane_name.clone())
                    .unwrap_or_else(|| record.harness.clone());
                let result = self.spawn_agent_worker_pane(
                    &role,
                    &record.harness,
                    &resume_cmd,
                    record.summary.as_deref().unwrap_or_default(),
                    subcontext.as_deref(),
                    false,
                    record.parent_pane_id,
                    Some(record.cwd.clone()),
                    None,
                    None,
                );
                let payload = match result {
                    Ok((new_pane_id, target_ctx_id)) => {
                        for win in &mut self.windows {
                            if let Some(pane) = win.panes.get_mut(&new_pane_id) {
                                if let Some(t) = pane.as_terminal_mut() {
                                    t.agent_session_id = record.session_id.clone();
                                    t.last_resumed_command = Some(resume_cmd.clone());
                                    t.slots = record.slots.clone();
                                    if let Some(ref mut a) = t.agent {
                                        a.session_id = record.session_id.clone();
                                        a.state = crate::protocol::AgentState::Idle;
                                    }
                                }
                                break;
                            }
                        }
                        self.mark_workspace_dirty();
                        log::info!(
                            "agent_reopen: reopened run_id={} session_id={:?} harness={} in pane {new_pane_id} cmd={resume_cmd}",
                            record.run_id,
                            record.session_id,
                            record.harness
                        );
                        serde_json::json!({
                            "ok": true,
                            "pane_id": new_pane_id,
                            "context_id": target_ctx_id,
                            "run_id": record.run_id,
                            "session_id": record.session_id,
                            "harness": record.harness,
                            "role": role,
                            "command": resume_cmd,
                            "cwd": record.cwd,
                        })
                    }
                    Err(e) => serde_json::json!({
                        "ok": false,
                        "error": e,
                    }),
                };
                write_json_response(response_file, payload);
            }
            crate::protocol::AppRequest::Wake => {
                // No-op: the wake effect is the socket listener's repaint
                // request — by the time this arm runs, a frame is already
                // in flight and queued work (spawn-queue, channels) drains.
                log::debug!("pane_ipc: kind=wake (no-op)");
            }
            crate::protocol::AppRequest::Shutdown => {
                // Sent by `plexi host stop` over a direct notify.sock
                // connection. This handler has no `egui::Context`, so it
                // cannot send ViewportCommand::Close directly — it sets a
                // flag consumed at the top of the next frame
                // (`update_preamble`, mirrors `update_quit_pending`).
                log::info!("pane_ipc: kind=shutdown — closing on next frame");
                self.shutdown_requested = true;
            }
            _ => {
                log::warn!("pane_ipc: unsupported command kind, dropping");
            }
        }
    }

    pub(crate) fn write_pane_slot_internal(
        &mut self,
        pane_id: u64,
        slot_name: &str,
        content: &[u8],
        append: bool,
        replace: bool,
    ) -> Result<(std::path::PathBuf, u64), String> {
        validate_slot_name(slot_name)?;
        if append && replace {
            return Err("use only one of append or replace".to_string());
        }
        if content.len() > MAX_SLOT_CONTENT_SIZE {
            return Err(format!(
                "slot '{slot_name}' content size {} exceeds 10485760 bytes",
                content.len()
            ));
        }
        let target = self.windows.iter().enumerate().find_map(|(idx, win)| {
            if win.panes.contains_key(&pane_id) {
                Some((idx, win.context_id))
            } else {
                None
            }
        });
        let Some((win_idx, context_id)) = target else {
            return Err(format!("pane {pane_id} not found"));
        };
        let context_root = self
            .router
            .iter()
            .find(|ctx| ctx.context_id == context_id)
            .map(|ctx| ctx.root.clone());
        let slot_dir = slot_base_dir(context_root.as_deref()).join(pane_id.to_string());
        let slot_path = slot_dir.join(slot_name);
        let Some(pane) = self.windows[win_idx].panes.get_mut(&pane_id) else {
            return Err(format!("pane {pane_id} not found"));
        };
        if slot_name == "completed" {
            if let Some(t) = pane.as_terminal_mut() {
                t.agent_completed = true;
            }
        }
        let Some(slots) = pane.slots_mut() else {
            return Err(format!("pane {pane_id} does not support slots"));
        };
        let tracked_path = slots.get(slot_name).cloned();
        let existing_path = tracked_path.clone().unwrap_or_else(|| slot_path.clone());
        let write_path = if append {
            tracked_path.clone().unwrap_or(slot_path)
        } else {
            slot_path
        };
        let exists = tracked_path.as_ref().is_some_and(|path| path.exists());
        if exists && !append && !replace {
            return Err(format!(
                "slot '{slot_name}' already exists — use --append to add to it or --replace to overwrite it"
            ));
        }
        if append {
            let existing_size = std::fs::metadata(&existing_path)
                .map(|m| m.len())
                .unwrap_or(0);
            let final_size = existing_size.saturating_add(content.len() as u64);
            if final_size > MAX_SLOT_CONTENT_SIZE as u64 {
                return Err(format!(
                    "slot '{slot_name}' content size {final_size} exceeds 10485760 bytes"
                ));
            }
        }
        let write_dir = write_path.parent().unwrap_or(slot_dir.as_path());
        if let Err(e) = std::fs::create_dir_all(write_dir) {
            return Err(format!(
                "could not create slot directory {}: {e}",
                write_dir.display()
            ));
        }
        let write_result = if append {
            use std::io::Write as _;
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&write_path)
                .and_then(|mut f| f.write_all(content).map(|_| ()))
        } else {
            std::fs::write(&write_path, content)
        };
        if let Err(e) = write_result {
            return Err(format!("could not write slot '{slot_name}': {e}"));
        }
        let absolute = write_path
            .canonicalize()
            .unwrap_or_else(|_| write_path.clone());
        slots.insert(slot_name.to_string(), absolute.clone());
        self.mark_workspace_dirty();
        let size = std::fs::metadata(&absolute).map(|m| m.len()).unwrap_or(0);
        match std::fs::read(&absolute) {
            Ok(value) => self.emit_pane_lifecycle(
                pane_id,
                PaneLifecycleEvent::SlotChanged {
                    name: slot_name.to_string(),
                    value,
                },
            ),
            Err(error) => log::error!(
                "pane_lifecycle: cannot read committed slot {}: {error}",
                absolute.display()
            ),
        }
        self.complete_slot_waits(pane_id, slot_name);
        Ok((absolute, size))
    }

    /// `ListPermissions` host handler (stint 0017). Writes the permission
    /// inventory JSON documented on the `AppRequest::ListPermissions` variant:
    /// one row per stored `permissions.toml` entry (`stored: true`) plus one
    /// row per live granted/blocked capability of a running app that has no
    /// stored entry (`stored: false`), and the list of running app ids.
    fn handle_list_permissions(&mut self, response_file: &str) {
        use crate::app::permissions::PermissionStore;
        use crate::host::pane::Pane;

        let store = PermissionStore::load_or_default(&self.permission_store_dir);
        let mut rows: Vec<serde_json::Value> = Vec::new();
        let mut stored_keys: std::collections::HashSet<String> = std::collections::HashSet::new();
        for (app_id, workspace, cap, state) in store.iter_entries() {
            stored_keys.insert(format!("{app_id}::{workspace}::{}", cap.as_str()));
            rows.push(serde_json::json!({
                "app_id": app_id,
                "workspace": workspace,
                "capability": cap.as_str(),
                "state": state.as_str(),
                "stored": true,
                "sensitive": cap.is_sensitive(),
                "description": cap.description(),
            }));
        }

        let mut running: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for win in &self.windows {
            for pane in win.panes.values() {
                let Pane::App(app_pane) = pane else { continue };
                running.insert(app_pane.manifest_id.clone());
                let workspace = app_pane
                    .workspace_root
                    .canonicalize()
                    .unwrap_or_else(|_| app_pane.workspace_root.clone())
                    .display()
                    .to_string();
                let live = app_pane
                    .permissions
                    .capabilities
                    .iter()
                    .map(|&cap| (cap, "green"))
                    .chain(app_pane.permissions.blocked.iter().map(|&cap| (cap, "red")));
                for (cap, state) in live {
                    let key = format!("{}::{}::{}", app_pane.manifest_id, workspace, cap.as_str());
                    if !stored_keys.insert(key) {
                        continue; // already covered by a stored row
                    }
                    rows.push(serde_json::json!({
                        "app_id": app_pane.manifest_id,
                        "workspace": workspace,
                        "capability": cap.as_str(),
                        "state": state,
                        "stored": false,
                        "sensitive": cap.is_sensitive(),
                        "description": cap.description(),
                    }));
                }
            }
        }

        let body = serde_json::json!({
            "permissions": rows,
            "running": running.into_iter().collect::<Vec<_>>(),
        })
        .to_string();
        write_response(response_file, body.as_bytes());
    }

    /// `SetPermission` host handler (stint 0017). Persists the new state to
    /// `permissions.toml` and live-updates every running app instance with a
    /// matching (app_id, workspace) so a revocation takes effect on the app's
    /// next request. Unknown capability/state strings fail closed with an
    /// error reply; a missing workspace fails unless the app is running.
    fn handle_set_permission(
        &mut self,
        app_id: &str,
        workspace: Option<&str>,
        capability: &str,
        state: &str,
        response_file: &str,
    ) {
        use crate::app::permissions::{Capability, PermissionState, PermissionStore};
        use crate::host::pane::Pane;

        let outcome: Result<(), String> = (|| {
            let cap = Capability::try_from(capability).map_err(|e| e.to_string())?;
            let new_state = PermissionState::try_from(state)?;

            let ws: std::path::PathBuf = match workspace {
                Some(w) => std::path::PathBuf::from(w),
                None => self
                    .windows
                    .iter()
                    .find_map(|win| {
                        win.panes.values().find_map(|pane| {
                            let Pane::App(app_pane) = pane else {
                                return None;
                            };
                            (app_pane.manifest_id == app_id)
                                .then(|| app_pane.workspace_root.clone())
                        })
                    })
                    .ok_or_else(|| {
                        format!(
                            "workspace required: app '{app_id}' is not running; \
                             pass workspace explicitly"
                        )
                    })?,
            };

            let mut store = PermissionStore::load_or_default(&self.permission_store_dir);
            store.set(app_id, &ws, cap, new_state);
            store.save();

            crate::broker::gate::PermissionMonitor::for_profile(&self.permission_store_dir)
                .grant_app_capability(
                    app_id,
                    &ws,
                    cap,
                    crate::broker::Decision::from_permission_state(new_state),
                );

            // Live-update every running instance of this app in this workspace.
            let ws_canonical = ws.canonicalize().unwrap_or_else(|_| ws.clone());
            for win in &mut self.windows {
                for pane in win.panes.values_mut() {
                    let Pane::App(app_pane) = pane else { continue };
                    if app_pane.manifest_id != app_id {
                        continue;
                    }
                    let proc_ws = app_pane
                        .workspace_root
                        .canonicalize()
                        .unwrap_or_else(|_| app_pane.workspace_root.clone());
                    if proc_ws != ws_canonical {
                        continue;
                    }
                    match new_state {
                        PermissionState::Green => {
                            app_pane.permissions.capabilities.insert(cap);
                            app_pane.permissions.blocked.remove(&cap);
                        }
                        PermissionState::Yellow => {
                            app_pane.permissions.capabilities.remove(&cap);
                            app_pane.permissions.blocked.remove(&cap);
                        }
                        PermissionState::Red => {
                            app_pane.permissions.blocked.insert(cap);
                            app_pane.permissions.capabilities.remove(&cap);
                        }
                    }
                    log::info!(
                        "pane_ipc: set_permission: live-updated '{app_id}' pane {} — {} → {}",
                        app_pane.id,
                        cap.as_str(),
                        new_state.as_str()
                    );
                }
            }
            Ok(())
        })();

        let body = match &outcome {
            Ok(()) => "{\"ok\":true}".to_string(),
            Err(e) => serde_json::json!({ "error": e }).to_string(),
        };
        write_response(response_file, body.as_bytes());
        if let Err(e) = outcome {
            log::warn!("pane_ipc: set_permission failed: {e}");
        }
    }

    /// Poll host-observed terminal activity for every terminal pane (1 Hz,
    /// called from the throttled block in the render loop). Working = a
    /// foreground process other than the shell holds the PTY (`tcgetpgrp`),
    /// Blocked = the shell exited, None = shell sitting at its prompt.
    ///
    /// The same pass synthesizes the host-observed half of the agent detector
    /// (`TerminalPane::observed_agent`): when a known agent binary is the PTY
    /// foreground-group leader, the pane's agent identity comes from the
    /// process name (or an interpreter's script argv) and its state from the
    /// PTY output settle. This covers both a hook-silent boot and a stale hook
    /// Idle that contradicts sustained fresh PTY activity.
    pub(super) fn tick_terminal_activity(&mut self) {
        use crate::protocol::AgentState;
        let mut transitions = Vec::new();
        let mut observed_dirty = false;
        for window in self.windows.iter_mut() {
            for (pane_id, pane) in window.panes.iter_mut() {
                let Some(t) = pane.as_terminal_mut() else {
                    continue;
                };
                if t.is_standby() {
                    continue;
                }
                let shell_pid = t.backend.child_pid();
                let fg = t.backend.foreground_pid();
                let new = if t.exited {
                    Some(AgentState::Blocked)
                } else {
                    // Two independent signals; either one means a command is
                    // running. fg pgid covers foreground jobs; the children
                    // check covers shells whose job-control behavior doesn't
                    // move the fg pgid, plus backgrounded servers.
                    let running = fg.is_some_and(|fg| fg != shell_pid as i32)
                        || crate::host::shell::pid_has_children(shell_pid);
                    running.then_some(AgentState::Working)
                };
                if new != t.activity {
                    log::info!(
                        "terminal activity: pane {} {:?} -> {:?} (shell_pid={}, fg_pgid={:?})",
                        pane_id,
                        t.activity,
                        new,
                        shell_pid,
                        fg
                    );
                    t.activity = new;
                }

                let observed = if t.exited {
                    None
                } else {
                    fg.filter(|fg| *fg != shell_pid as i32)
                        .and_then(
                            |fg| match crate::host::shell::get_pid_agent_probe(fg as u32) {
                                crate::host::shell::AgentProcessProbe::Known(agent) => Some(agent),
                                crate::host::shell::AgentProcessProbe::UnsupportedInterpreter {
                                    ..
                                }
                                | crate::host::shell::AgentProcessProbe::Unknown => None,
                            },
                        )
                        .map(|agent| {
                            // A TUI that has stopped redrawing is sitting at its
                            // prompt; one that is still streaming (banner draw,
                            // spinner animation) is not ready yet. Same physical
                            // signal `pane send --submit` settles on.
                            let settled = t
                                .last_pty_output_at
                                .is_some_and(|at| at.elapsed() >= OBSERVED_AGENT_SETTLE);
                            crate::protocol::PaneAgentState {
                                pane_id: *pane_id,
                                state: if settled {
                                    AgentState::Idle
                                } else {
                                    AgentState::Working
                                },
                                agent: agent.to_string(),
                                detail: None,
                                session_id: None,
                            }
                        })
                };
                let was_stale_idle_fallback = t.agent.as_ref().is_some_and(|hook| {
                    hook.state == AgentState::Idle
                        && t.agent_reported_at.is_some_and(|reported| {
                            reported.elapsed() >= crate::host::pane::HOOK_AGENT_FRESHNESS
                        })
                        && t.observed_agent.as_ref().is_some_and(|observed| {
                            observed.agent == hook.agent && observed.state == AgentState::Working
                        })
                });
                let is_stale_idle_fallback = t.agent.as_ref().is_some_and(|hook| {
                    hook.state == AgentState::Idle
                        && t.agent_reported_at.is_some_and(|reported| {
                            reported.elapsed() >= crate::host::pane::HOOK_AGENT_FRESHNESS
                        })
                        && observed.as_ref().is_some_and(|observed| {
                            observed.agent == hook.agent && observed.state == AgentState::Working
                        })
                });
                let changed = match (&t.observed_agent, &observed) {
                    (None, None) => false,
                    (Some(a), Some(b)) => a.agent != b.agent || a.state != b.state,
                    _ => true,
                };
                if changed {
                    log::info!(
                        "observed agent: pane {} {:?} -> {:?} (fg_pgid={:?})",
                        pane_id,
                        t.observed_agent.as_ref().map(|a| (&a.agent, &a.state)),
                        observed.as_ref().map(|a| (&a.agent, &a.state)),
                        fg
                    );
                    if is_stale_idle_fallback && !was_stale_idle_fallback {
                        log::info!(
                            "observed agent: pane {pane_id} stale hook idle yielded to corroborated working"
                        );
                    } else if was_stale_idle_fallback && !is_stale_idle_fallback {
                        log::info!(
                            "observed agent: pane {pane_id} corroborated working settled; returning to hook idle"
                        );
                    }
                    if let Some(agent) = &observed {
                        transitions.push((*pane_id, agent.clone()));
                    }
                    t.set_observed_agent(observed);
                    observed_dirty = true;
                }
            }
        }
        if observed_dirty {
            self.mark_workspace_dirty();
        }
        for (pane_id, agent) in transitions {
            self.emit_observed_agent_transition(pane_id, &agent);
        }
    }

    pub(crate) fn tick_scheduler(&mut self) {
        // Load routines from every context root
        let roots: Vec<std::path::PathBuf> =
            self.router.iter().map(|ctx| ctx.root.clone()).collect();
        for root in &roots {
            let failures = self.scheduler.load_from_root(root);
            for f in failures {
                self.notify_routine_issue(root, &f.title, &f.body);
            }
        }

        let now = chrono::Local::now();
        let due = self.scheduler.due_routines(now);
        for idx in due {
            let (name, command, context_name, ephemeral, source_root) = {
                let entry = &self.scheduler.entries[idx];
                (
                    entry.routine.name.clone(),
                    entry.routine.command.clone(),
                    entry.routine.context.clone(),
                    entry.routine.ephemeral,
                    entry.source_root.clone(),
                )
            };

            // Overlap guard: while the previous run's pane is alive (exists
            // and its PTY child has not exited), skip — never stack panes.
            // Ephemeral panes free the routine when the command exits (the
            // pane auto-closes); a non-ephemeral pane frees it when its shell
            // session ends or the pane is closed — an exited "[process
            // exited]" placeholder never holds a routine hostage. The skip
            // does NOT stamp last_fire, so the routine fires on the next
            // tick after the run ends rather than waiting a full interval.
            if let Some(run) = self.scheduler.live_run(&source_root, &name) {
                let pane_id = run.pane_id;
                let skip_notified = run.skip_notified;
                if self.routine_run_is_alive(pane_id) {
                    log::info!(
                        "scheduler: routine '{name}' skipped — previous run's pane {pane_id} is still open"
                    );
                    if !skip_notified {
                        self.notify_routine_issue(
                            &source_root,
                            &format!("Routine '{name}' skipped"),
                            &format!(
                                "The previous run's pane ({pane_id}) is still running; the routine fires again once it exits or the pane is closed."
                            ),
                        );
                        self.scheduler.mark_skip_notified(&source_root, &name);
                    }
                    continue;
                }
                // Run ended (PTY child exited, or the pane vanished through a
                // path that skipped the close hook) — reap and fall through
                // to fire.
                self.scheduler.reap_run(&source_root, &name);
            }

            self.scheduler.mark_fired(idx, now);
            if let Some(pane_id) =
                self.fire_routine(&name, &command, &context_name, ephemeral, &source_root)
            {
                self.scheduler.register_run(&source_root, &name, pane_id);
                let root_key = source_root.display();
                self.scheduler
                    .clear_failure(&format!("{root_key}|{name}|context-missing"));
                self.scheduler
                    .clear_failure(&format!("{root_key}|{name}|spawn-failed"));
            }
        }
    }

    /// True while a routine run's pane exists and its PTY child has not
    /// exited. Liveness is this kernel fact (`PtyEvent::Exit` → `t.exited`)
    /// and never the `activity` sampler: activity is a UI affordance whose
    /// heuristics (fg pgid, child scan) go blind when the shell
    /// exec-optimises a bare command into itself — an ephemeral run's
    /// `zsh -c "sleep 30"` IS the sleep, has no children, and never moves
    /// the fg pgid, so sampling it read live runs as finished and stacked
    /// panes without bound.
    fn routine_run_is_alive(&self, pane_id: u64) -> bool {
        self.windows.iter().any(|w| {
            w.panes
                .get(&pane_id)
                .is_some_and(|p| p.as_terminal().is_some_and(|t| !t.exited))
        })
    }

    /// Surface a routine problem as a user-visible notification, scoped to the
    /// context that owns the routine's workspace root (global when no context
    /// matches). Routed through the single notification choke point.
    fn notify_routine_issue(&mut self, source_root: &std::path::Path, title: &str, body: &str) {
        let ctx_id = self
            .router
            .iter()
            .find(|c| c.root.as_path() == source_root)
            .map(|c| c.context_id);
        let (scope, source_context_id) = match ctx_id {
            Some(id) => (crate::protocol::NotifyScope::Context, id),
            None => (crate::protocol::NotifyScope::Global, 0),
        };
        let queued = self.enqueue_notification(
            crate::app::notifications::NotifySource::HostInternal,
            crate::app::notifications::PendingNotification {
                notify_id: crate::app::notifications::new_notify_id("routine"),
                source_context_id,
                title: title.to_string(),
                body: body.to_string(),
                scope,
                ..Default::default()
            },
        );
        log::info!("scheduler: routine notification queued={queued} title='{title}' body='{body}'");
    }

    /// The pane a context-targeted terminal spawn splits beside: the first
    /// window of `context_id`, and that window's focused pane (or tree root).
    /// `None` when the context has no window or the window has no pane —
    /// nothing to split, so the spawn fails rather than landing elsewhere.
    /// Shared by the scheduler's `fire_routine` and the `context_name` spawn
    /// path (`plexi routine run`), so both doors target identically.
    pub(crate) fn context_spawn_target(
        &self,
        context_id: u64,
    ) -> Option<(usize, egui_tiles::TileId)> {
        let win_idx = self
            .windows
            .iter()
            .position(|w| w.context_id == context_id)?;
        let win = &self.windows[win_idx];
        let tile = win.focused_pane.or(win.tree.root)?;
        Some((win_idx, tile))
    }

    /// Fire a routine into its target context. Returns the spawned pane id,
    /// or `None` when the routine could not run (missing context, spawn
    /// failure) — both surfaced as notifications on transition into the
    /// failure state.
    pub(crate) fn fire_routine(
        &mut self,
        name: &str,
        command: &str,
        context_name: &str,
        ephemeral: bool,
        source_root: &std::path::Path,
    ) -> Option<u64> {
        log::info!(
            "scheduler: routine '{}' fired context='{}' ephemeral={}",
            name,
            context_name,
            ephemeral
        );

        // Find target context index. A named context is a *target*, not a
        // gate: the routine fires into it regardless of which context is
        // active, and is skipped only when no context by that name exists.
        let target_ctx_idx = if context_name.is_empty() {
            self.router.active_idx()
        } else {
            match self.router.position(|c| c.name == context_name) {
                Some(idx) => idx,
                None => {
                    log::warn!(
                        "scheduler: routine '{}' — context '{}' not found, skipping",
                        name,
                        context_name
                    );
                    let key = format!("{}|{name}|context-missing", source_root.display());
                    if self.scheduler.note_failure(key) {
                        self.notify_routine_issue(
                            source_root,
                            &format!("Routine '{name}' skipped"),
                            &format!(
                                "Context '{context_name}' does not exist — the routine cannot fire until it does."
                            ),
                        );
                    }
                    return None;
                }
            }
        };

        // Explicit-target spawn: never steer through router.set_active /
        // active_window — the target is a parameter, not ambient focus state
        // (stint 0574; "Don't switch global state to thread data" trap).
        let target_ctx_id = self.router.get(target_ctx_idx).context_id;
        let cwd = Some(self.router.get(target_ctx_idx).root.clone());
        let spawned = self.context_spawn_target(target_ctx_id).map(|(win, tile)| {
            // vertical=true → side-by-side, matching the previous
            // split_focused(false, ..) behavior (its LinearDir is inverted).
            self.spawn_terminal_pane_at(crate::pane_ops::create::SpawnTerminalAtSpec {
                win_idx: win,
                target_tile: tile,
                vertical: true,
                new_pane_first: false,
                keep_focus: false,
                options: crate::pane_ops::create::TerminalSpawnOptions {
                    initial_cmd: Some(command),
                    close_on_exit: ephemeral,
                    cwd_override: cwd,
                    ..Default::default()
                },
            })
        });

        if spawned.is_none() {
            log::warn!("scheduler: routine '{name}' — failed to spawn a pane");
            let key = format!("{}|{name}|spawn-failed", source_root.display());
            if self.scheduler.note_failure(key) {
                self.notify_routine_issue(
                    source_root,
                    &format!("Routine '{name}' failed"),
                    "Could not spawn a pane for the routine's command.",
                );
            }
        }
        spawned
    }

    pub(super) fn drain_spawn_queue(&mut self) {
        const STALE_SPAWN_WARN_SECS: u64 = 60;
        let queue_dir = crate::config::config_dir().join("spawn-queue");
        let Ok(entries) = std::fs::read_dir(&queue_dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(&path) else {
                continue;
            };
            if let Err(e) = std::fs::remove_file(&path) {
                log::error!("spawn-queue: failed to remove spawn file {path:?}: {e}");
                continue;
            }
            let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) else {
                continue;
            };
            let type_id = val["type_id"].as_str().unwrap_or("").to_string();
            let path_target = val["path"].as_str().map(str::to_string);
            let args: Vec<String> = val["args"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            let request = crate::protocol::AppRequest::SpawnPane {
                type_id,
                layout: val["layout"].as_str().map(str::to_string),
                args,
                from_pane_id: None,
                request_id: None,
                response_file: val["response_file"].as_str().map(str::to_string),
                ephemeral: val["ephemeral"].as_bool().unwrap_or(false),
                cwd: val["cwd"].as_str().map(str::to_string),
                no_focus: val["no_focus"].as_bool().unwrap_or(false),
                path: path_target,
                workspace_root: val["workspace_root"].as_str().map(str::to_string),
                target_context: val["target_context"].as_u64(),
                context_name: val["context_name"].as_str().map(str::to_string),
                name: val["name"].as_str().map(str::to_string),
                // Never from the queue: an agent boot owes its caller a reply
                // on a response file, and a queued spawn has no live caller.
                // `pane new --agent` requires PLEXI_SOCKET for this reason.
                agent_cmd: None,
                boot_timeout_secs: None,
                worktree: val["worktree"].as_str().map(str::to_string),
                result_schema: val.get("result_schema").cloned().filter(|v| !v.is_null()),
                peer_ancestry: None,
            };
            let Ok(spec) = crate::app::launch_spec::PaneLaunchSpec::from_spawn_pane(&request)
            else {
                log::warn!("spawn-queue: invalid spawn request, skipping");
                continue;
            };
            let origin = val["origin"].as_str().unwrap_or("unknown");
            let age_secs = spawn_file_age_secs(
                val["queued_at_ms"].as_u64(),
                crate::platform::clock::now_millis() as u64,
            );
            // A spawn file older than the queue's promised ~1 s pickup was
            // written while this host was not servicing — surface who queued
            // it and how long it sat, so a late-materializing pane is
            // attributable instead of a silent surprise (stint 0532).
            if let Some(age) = age_secs.filter(|a| *a > STALE_SPAWN_WARN_SECS) {
                log::warn!(
                    "spawn-queue: draining stale spawn file queued {age}s ago (origin={origin}) — request predates this host servicing the queue"
                );
            }
            log::info!(
                "spawn-queue: launching target={} origin={origin} age_secs={age_secs:?} layout={:?} args={:?} ephemeral={} no_focus={} cwd={:?} workspace_root={:?}",
                spec.target_for_log(),
                spec.layout,
                spec.args,
                spec.ephemeral,
                spec.no_focus,
                spec.cwd,
                spec.workspace_root
            );
            self.handle_pane_ipc_request(request);
        }
    }

    /// Service every WASM Python pane's external I/O from the logic pass.
    /// `LivePythonPane::ui` also drains, but eframe skips `App::ui` entirely
    /// while the window is hidden — a drain that lives only in the paint path
    /// wedges whenever the pane is idle or the window is occluded (CLAUDE.md
    /// trap; the MCP handshake stall this fixes is documented on
    /// `LivePythonPane::service_external_io`).
    pub(super) fn service_python_pane_runtimes(&mut self, ctx: &egui::Context) {
        for win in &mut self.windows {
            for pane in win.panes.values_mut() {
                if let Some(app) = pane.as_app_mut() {
                    if let crate::host::pane::AppRuntime::Python(python) = &mut app.runtime {
                        python.service_external_io(ctx);
                    }
                }
            }
        }
    }

    pub(super) fn drain_pty_events(&mut self) {
        self.observe_pane_spawns();
        let mut panes_to_close: Vec<u64> = Vec::new();

        while let Ok((id, event)) = self.pty_event_rx.try_recv() {
            // Every event on this channel — `Wakeup` above all, which fires
            // whenever the terminal processes new PTY output — is evidence the
            // pane is still producing. `pane send --submit` waits for a quiet
            // window in this stamp before it presses Enter, so it must be taken
            // for the whole event stream, not the two variants matched below.
            if let Some(term) = self
                .windows
                .iter_mut()
                .find_map(|win| win.panes.get_mut(&id))
                .and_then(crate::host::pane::Pane::as_terminal_mut)
            {
                term.last_pty_output_at = Some(std::time::Instant::now());
            }
            match &event {
                PtyEvent::Exit(pty_status) => {
                    let status = ExitStatus::from(*pty_status);
                    let mut newly_exited = false;
                    let subcontext_ids: std::collections::HashSet<u64> = self
                        .router
                        .iter()
                        .filter(|c| c.parent_id.is_some())
                        .map(|c| c.context_id)
                        .collect();
                    for win in &mut self.windows {
                        let is_subcontext = subcontext_ids.contains(&win.context_id);
                        if let Some(pane) = win.panes.get_mut(&id) {
                            if let Some(t) = pane.as_terminal_mut() {
                                newly_exited = !t.exited;
                                t.exited = true;
                                t.exit_status = Some(status);
                                let is_agent_or_subcontext = is_subcontext
                                    || t.parent_pane_id.is_some()
                                    || t.agent_harness.is_some()
                                    || t.agent_role.is_some();
                                log::info!(
                                    "pty: pane {id} process exited status={status:?} ephemeral={} is_agent_or_subcontext={is_agent_or_subcontext}",
                                    t.ephemeral
                                );
                                // Stint 0765 / 0802: ephemeral agent/sub-context panes only
                                // self-close on clean exit (0); non-zero or signal-terminated
                                // crashes stay open in Error state for debugging and prevent
                                // `collapse_subcontext_if_empty` from hiding the failed run.
                                if t.ephemeral {
                                    if status.is_failure() && is_agent_or_subcontext {
                                        t.agent = None;
                                        t.observed_agent = None;
                                        log::info!(
                                            "pty: retaining ephemeral pane {id} open after non-zero exit ({status:?})"
                                        );
                                    } else {
                                        panes_to_close.push(id);
                                    }
                                }
                            }
                            break;
                        }
                    }
                    if newly_exited {
                        self.emit_pane_lifecycle(id, PaneLifecycleEvent::Exited { status });
                    }
                }
                PtyEvent::Title(title) => {
                    if let Some(cmd) = title.strip_prefix("plexi:") {
                        match cmd {
                            "close" => panes_to_close.push(id),
                            _ => log::debug!("unknown plexi command: {}", cmd),
                        }
                    } else {
                        let title_trimmed = title.trim();
                        let osc_enabled = self.config.osc_pane_title_enabled();
                        for win in &mut self.windows {
                            if let Some(pane) = win.panes.get_mut(&id) {
                                if let Some(t) = pane.as_terminal_mut() {
                                    // Always track the raw OSC 2 title for event logging,
                                    // independent of osc_enabled and name_locked.
                                    t.pty_title = if title_trimmed.is_empty() {
                                        None
                                    } else {
                                        Some(title_trimmed.to_string())
                                    };
                                    if osc_enabled {
                                        if t.name_locked {
                                            log::debug!(
                                                "osc_title: pane {id} name locked, skipping"
                                            );
                                        } else {
                                            let is_empty = title_trimmed.is_empty();
                                            let already_matches = match &t.name {
                                                None => is_empty,
                                                Some(curr) => !is_empty && curr == title_trimmed,
                                            };
                                            if !already_matches {
                                                t.name = if is_empty {
                                                    None
                                                } else {
                                                    Some(title_trimmed.to_string())
                                                };
                                                log::debug!(
                                                    "osc_title: pane {id} name set to {:?}",
                                                    t.name
                                                );
                                            }
                                        }
                                    }
                                }
                                break;
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        for pane_id in panes_to_close {
            self.close_pane_by_id(pane_id);
        }
        self.refresh_portal_context_states();
    }

    /// Apply an inline pane name immediately after spawn. Sets `name_locked` so
    /// OSC title sequences don't overwrite the explicit label.
    fn apply_inline_pane_name(&mut self, pane_id: u64, pane_name: &str) {
        log::info!("pane_ipc: spawn_pane: applying inline name={pane_name:?} to pane_id={pane_id}");
        for win in &mut self.windows {
            if let Some(pane) = win.panes.get_mut(&pane_id) {
                if let Some(t) = pane.as_terminal_mut() {
                    t.name_locked = true;
                    t.name = Some(pane_name.to_string());
                    return;
                }
            }
        }
        log::warn!(
            "pane_ipc: spawn_pane: could not apply name to pane_id={pane_id} (not found or not a terminal)"
        );
    }

    /// Resolve the repository anchor directory for a `plexi pane new --worktree`
    /// request: explicit `--cwd` wins, then named-context root, then the calling
    /// pane's repo root / cwd / context root, then the active window's focused
    /// pane or active context root.
    pub(crate) fn resolve_worktree_anchor(
        &self,
        spec: &crate::app::launch_spec::PaneLaunchSpec,
    ) -> std::path::PathBuf {
        if let Some(ref cwd) = spec.cwd {
            return cwd.clone();
        }
        if let Some(ctx_name) = spec.context_name.as_deref().filter(|c| !c.is_empty()) {
            if let Some(ctx) = self.router.iter().find(|c| c.name == ctx_name) {
                return ctx.root.clone();
            }
        }
        if let Some(from_id) = spec.from_pane_id {
            if let Some(wt) = self.pane_worktrees.get(&from_id) {
                return wt.repo_root.clone();
            }
            if let Some((win_idx, tile)) = self.find_pane_in_any_window(from_id) {
                if let Some(cwd) = self.windows[win_idx].get_focused_pane_cwd(tile) {
                    return cwd;
                }
                let ctx_id = self.windows[win_idx].context_id;
                if let Some(ctx) = self.router.iter().find(|c| c.context_id == ctx_id) {
                    return ctx.root.clone();
                }
            }
        }
        let active = self.active_window;
        if let Some(win) = self.windows.get(active) {
            if let Some(tile) = win.focused_pane.or(win.tree.root) {
                if let Some(egui_tiles::Tile::Pane(pid)) = win.tree.tiles.get(tile) {
                    if let Some(wt) = self.pane_worktrees.get(pid) {
                        return wt.repo_root.clone();
                    }
                }
                if let Some(cwd) = win.get_focused_pane_cwd(tile) {
                    return cwd;
                }
            }
            let ctx_id = win.context_id;
            if let Some(ctx) = self.router.iter().find(|c| c.context_id == ctx_id) {
                return ctx.root.clone();
            }
        }
        self.router.active().root.clone()
    }
}

/// Pair each created pane with the command it launched, for the `context_sub:`
/// trace. A plain shell renders as `<shell>` rather than `None`, so the log
/// reads the same way whether or not `--command` was given.
///
/// Zips defensively: if the host ever creates a different number of panes than
/// commands requested, the trace shows the panes it actually made rather than
/// panicking inside a log statement.
fn launched_pairs<'a>(pane_ids: &[u64], commands: &'a [Option<String>]) -> Vec<(u64, &'a str)> {
    pane_ids
        .iter()
        .zip(commands.iter())
        .map(|(id, cmd)| (*id, cmd.as_deref().unwrap_or("<shell>")))
        .collect()
}

/// Age of a spawn-queue file in whole seconds, from its `queued_at_ms` stamp.
/// `None` when the file predates the stamp (written by an older CLI) or the
/// clock went backwards.
fn spawn_file_age_secs(queued_at_ms: Option<u64>, now_ms: u64) -> Option<u64> {
    queued_at_ms
        .and_then(|q| now_ms.checked_sub(q))
        .map(|ms| ms / 1000)
}

#[cfg(test)]
mod spawn_queue_age_tests {
    use super::spawn_file_age_secs;

    #[test]
    fn age_from_stamp() {
        assert_eq!(spawn_file_age_secs(Some(1_000), 91_000), Some(90));
        assert_eq!(spawn_file_age_secs(Some(5_000), 5_400), Some(0));
    }

    #[test]
    fn missing_or_future_stamp_is_none() {
        assert_eq!(spawn_file_age_secs(None, 91_000), None);
        assert_eq!(spawn_file_age_secs(Some(91_000), 1_000), None);
    }
}

#[cfg(test)]
mod context_sub_trace_tests {
    use super::launched_pairs;

    /// The stint-stipulated trace must name the command each pane launched, so
    /// "agent 2 got the wrong job" is answerable from `plexi.log` alone.
    #[test]
    fn pairs_each_pane_with_its_command() {
        let cmds = vec![
            Some("cm review".to_string()),
            Some("cm test".to_string()),
            Some("cm".to_string()),
        ];
        assert_eq!(
            launched_pairs(&[317, 318, 319], &cmds),
            vec![(317, "cm review"), (318, "cm test"), (319, "cm")]
        );
    }

    /// A plain shell reads as `<shell>`, not `None` — the log line has the same
    /// shape whether or not `--command` was given.
    #[test]
    fn a_plain_shell_is_named_not_null() {
        assert_eq!(
            launched_pairs(&[42, 43], &[None, None]),
            vec![(42, "<shell>"), (43, "<shell>")]
        );
    }

    /// Never panic inside a log statement: a length mismatch reports the panes
    /// actually created rather than taking the host down.
    #[test]
    fn length_mismatch_reports_what_exists() {
        assert_eq!(
            launched_pairs(&[1], &[Some("a".into()), Some("b".into())]),
            vec![(1, "a")]
        );
        assert_eq!(launched_pairs(&[1, 2], &[Some("a".into())]), vec![(1, "a")]);
    }
}
