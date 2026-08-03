//! Lifecycle methods — per-frame drain and tick operations on PlexiApp.

use egui_term::PtyEvent;

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
        for req in subscribe_reqs {
            // `Allow`/`Deny`/undeclared answer the transport inline; `Ask`
            // returns a parked consent we surface as a host modal next frame.
            if let Some(consent) = self.host_subscriptions.classify_subscribe_request(req) {
                self.pending_event_consents.push_back(consent);
            }
        }
        for req in publish_reqs {
            if let Some(consent) = self.host_subscriptions.classify_publish_request(req) {
                self.pending_event_consents.push_back(consent);
            }
        }
    }

    /// Handle a single pane-IPC `AppRequest`. Shared by the socket drain above
    /// and the PGAP forwarding path (`AppCommand::ForwardPaneRequest`, stint
    /// 0013/0014) so capability-gated app requests take the identical host
    /// code path as CLI requests arriving over PLEXI_SOCKET.
    pub(crate) fn handle_pane_ipc_request(&mut self, cmd: crate::app_protocol::AppRequest) {
        match &cmd {
            crate::app_protocol::AppRequest::SetPaneTitle { pane_id, name } => {
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
                    crate::host::event_log::emit(crate::host::event_log::HostEvent::PaneRenamed {
                        pane_id: *pane_id,
                        name: name.clone(),
                        timestamp: crate::host::event_log::now_timestamp(),
                    });
                }
                if !found {
                    log::warn!("pane_ipc: set_pane_title: pane_id={pane_id} not found");
                }
            }
            crate::app_protocol::AppRequest::LogMarker {
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
            crate::app_protocol::AppRequest::ListPanes {
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
                        .map(|ctx| ctx.name.clone())
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
                                let cwd = crate::host::shell::get_pid_cwd(t.backend.child_pid())
                                    .map(|p| p.to_string_lossy().into_owned());
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
                        entries.push(serde_json::json!({
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
                        }));
                    }
                }
                let json_str = serde_json::to_string(&entries).unwrap_or_else(|_| "[]".to_string());
                write_response(response_file, json_str.as_bytes());
            }
            crate::app_protocol::AppRequest::ListContexts { response_file } => {
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
            crate::app_protocol::AppRequest::GetPaneInfo {
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
                                let cwd = crate::host::shell::get_pid_cwd(t.backend.child_pid())
                                    .map(|p| p.to_string_lossy().into_owned());
                                serde_json::json!({
                                    "id": pane_id,
                                    "type": "terminal",
                                    "title": t.name.clone().unwrap_or_else(|| "terminal".to_string()),
                                    "focused": focused,
                                    "context_id": win.context_id,
                                    "window_id": win.window_id,
                                    "cwd": cwd,
                                    "agent": agent,
                                    "slots": pane_slots_json(pane),
                                    "heartbeat": self.pane_heartbeat_json(*pane_id),
                                })
                            }
                            crate::host::pane::Pane::App(a) => {
                                serde_json::json!({
                                    "id": pane_id,
                                    "type": "app",
                                    "title": a.name.clone(),
                                    "focused": focused,
                                    "context_id": win.context_id,
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
            crate::app_protocol::AppRequest::GetPreviousPaneInfo {
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
            crate::app_protocol::AppRequest::SlotWrite {
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
                if let Err(msg) = validate_slot_name(slot_name) {
                    slot_error(response_file, msg);
                    return;
                }
                if *append && *replace {
                    slot_error(response_file, "use only one of append or replace");
                    return;
                }
                if content.len() > MAX_SLOT_CONTENT_SIZE {
                    slot_error(
                        response_file,
                        format!(
                            "slot '{slot_name}' content size {} exceeds 10485760 bytes",
                            content.len()
                        ),
                    );
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
                let slot_dir = slot_base_dir(context_root.as_deref()).join(pane_id.to_string());
                let slot_path = slot_dir.join(slot_name);
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
                let tracked_path = slots.get(slot_name).cloned();
                let existing_path = tracked_path.clone().unwrap_or(slot_path.clone());
                let write_path = if *append {
                    tracked_path.clone().unwrap_or(slot_path)
                } else {
                    slot_path
                };
                let exists = tracked_path.as_ref().is_some_and(|path| path.exists());
                if exists && !*append && !*replace {
                    slot_error(
                        response_file,
                        format!(
                            "slot '{slot_name}' already exists — use --append to add to it or --replace to overwrite it"
                        ),
                    );
                    return;
                }
                if *append {
                    let existing_size = std::fs::metadata(&existing_path)
                        .map(|m| m.len())
                        .unwrap_or(0);
                    let final_size = existing_size.saturating_add(content.len() as u64);
                    if final_size > MAX_SLOT_CONTENT_SIZE as u64 {
                        slot_error(
                            response_file,
                            format!(
                                "slot '{slot_name}' content size {final_size} exceeds 10485760 bytes"
                            ),
                        );
                        return;
                    }
                }
                let write_dir = write_path.parent().unwrap_or(slot_dir.as_path());
                if let Err(e) = std::fs::create_dir_all(write_dir) {
                    slot_error(
                        response_file,
                        format!(
                            "could not create slot directory {}: {e}",
                            write_dir.display()
                        ),
                    );
                    return;
                }
                let write_result = if *append {
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
                    slot_error(
                        response_file,
                        format!("could not write slot '{slot_name}': {e}"),
                    );
                    return;
                }
                let absolute = write_path
                    .canonicalize()
                    .unwrap_or_else(|_| write_path.clone());
                slots.insert(slot_name.clone(), absolute.clone());
                let size = std::fs::metadata(&absolute).map(|m| m.len()).unwrap_or(0);
                write_json_response(
                    response_file,
                    serde_json::json!({
                        "ok": true,
                        "name": slot_name,
                        "path": absolute.to_string_lossy(),
                        "size": size,
                    }),
                );
                // The write path is the only thing that changes a slot's
                // value, so it is where parked waiters are answered.
                self.complete_slot_waits(*pane_id, slot_name);
            }
            crate::app_protocol::AppRequest::SlotWait {
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
                // Level-triggered: a value that already matches answers now.
                // A caller cannot guarantee it armed the wait before the
                // writer ran, so an edge-only wait would hang on a race it
                // has no way to avoid.
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
            crate::app_protocol::AppRequest::SlotRead {
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
                    .and_then(|slots| slots.get(slot_name).cloned());
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
            crate::app_protocol::AppRequest::SlotList {
                pane_id,
                response_file,
            } => {
                log::info!("pane_ipc: kind=slot_list pane_id={pane_id}");
                let slots = self
                    .windows
                    .iter()
                    .find_map(|win| win.panes.get(pane_id))
                    .and_then(|pane| pane.slots());
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
            crate::app_protocol::AppRequest::SlotDelete {
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
                write_json_response(
                    response_file,
                    serde_json::json!({
                        "ok": true,
                        "name": slot_name,
                        "removed": removed,
                    }),
                );
            }
            crate::app_protocol::AppRequest::WorkspaceCleanSlots {
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
            crate::app_protocol::AppRequest::FocusPane { pane_id } => {
                log::info!("pane_ipc: kind=focus_pane pane_id={pane_id}");
                if !self.pane_navigate(*pane_id) {
                    log::warn!("pane_ipc: focus_pane: pane_id={pane_id} not found");
                }
            }
            crate::app_protocol::AppRequest::ClosePane { pane_id } => {
                log::info!("pane_ipc: kind=close_pane pane_id={pane_id}");
                let before: usize = self.windows.iter().map(|w| w.panes.len()).sum();
                self.close_pane_by_id(*pane_id);
                let after: usize = self.windows.iter().map(|w| w.panes.len()).sum();
                if before == after {
                    log::warn!("pane_ipc: close_pane: pane_id={pane_id} not found");
                }
            }
            crate::app_protocol::AppRequest::SpawnPane { response_file, .. } => {
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
                    "pane_ipc: kind=spawn_pane target={} layout={:?} args={:?} ephemeral={} no_focus={} from_pane_id={:?} cwd={:?} workspace_root={:?} response_file={:?}",
                    spec.target_for_log(),
                    spec.layout,
                    spec.args,
                    spec.ephemeral,
                    spec.no_focus,
                    spec.from_pane_id,
                    spec.cwd,
                    spec.workspace_root,
                    spec.response_file
                );
                let mut response_pane_id = self.host.next_pane_id();

                let active = self.active_window;
                let cwd_override = spec.cwd.clone();
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
                    if let Some(ctx_name) = spec.context_name.as_deref().filter(|c| !c.is_empty()) {
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
                        let vertical =
                            matches!(layout_str, "split_h" | "split_right" | "split_left");
                        let new_pane_first = matches!(layout_str, "split_above" | "split_left");
                        log::info!(
                            "pane_ipc: spawn_pane terminal context_name='{ctx_name}' context_id={target_ctx_id} target_win={target_win} layout={layout_str} initial_cmd={initial_cmd:?} ephemeral={}",
                            spec.ephemeral
                        );
                        response_pane_id = self.spawn_terminal_pane_at(
                            target_win,
                            target_tile,
                            vertical,
                            new_pane_first,
                            initial_cmd.as_deref(),
                            spec.ephemeral,
                            cwd_override,
                            spec.no_focus,
                        );
                        if let Some(ref pane_name) = spec.name {
                            if !pane_name.is_empty() {
                                self.apply_inline_pane_name(response_pane_id, pane_name);
                            }
                        }
                        self.reply_spawn_pane(&spec, response_pane_id, &Ok(()));
                        return;
                    }
                    if layout_str == "new_window" {
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
                            initial_cmd.as_deref(),
                            spec.ephemeral,
                            cwd_override,
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
                        let (target_win, target_tile) = if let Some(from_id) = spec.from_pane_id {
                            match self.find_pane_in_any_window(from_id) {
                                Some(loc) => {
                                    log::info!(
                                        "pane_ipc: spawn_pane: targeting from_pane_id={from_id} in win_idx={}",
                                        loc.0
                                    );
                                    loc
                                }
                                None => {
                                    log::warn!(
                                        "pane_ipc: spawn_pane: from_pane_id={from_id} not found in any window, using focused pane"
                                    );
                                    if let Some(tile) = self.windows[active]
                                        .focused_pane
                                        .or(self.windows[active].tree.root)
                                    {
                                        (active, tile)
                                    } else {
                                        // Active window is empty (windowless boot):
                                        // split_focused would no-op, so seed a root
                                        // pane in place instead of dropping the spawn.
                                        log::info!(
                                            "pane_ipc: spawn_pane terminal layout={layout_str} (empty active window — seeding root)"
                                        );
                                        match self.seed_root_pane(
                                            initial_cmd.as_deref(),
                                            spec.ephemeral,
                                            cwd_override,
                                        ) {
                                            Some(seeded_id) => response_pane_id = seeded_id,
                                            None => {
                                                launch_result =
                                                    Err("failed to seed root pane".into())
                                            }
                                        }
                                        if spec.no_focus {
                                            self.active_window = active;
                                        }
                                        if launch_result.is_ok() {
                                            if let Some(ref pane_name) = spec.name {
                                                if !pane_name.is_empty() {
                                                    self.apply_inline_pane_name(
                                                        response_pane_id,
                                                        pane_name,
                                                    );
                                                }
                                            }
                                        }
                                        self.reply_spawn_pane(
                                            &spec,
                                            response_pane_id,
                                            &launch_result,
                                        );
                                        return;
                                    }
                                }
                            }
                        } else if let Some(tile) = self.windows[active]
                            .focused_pane
                            .or(self.windows[active].tree.root)
                        {
                            (active, tile)
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
                            ) {
                                Some(seeded_id) => response_pane_id = seeded_id,
                                None => launch_result = Err("failed to seed root pane".into()),
                            }
                            if spec.no_focus {
                                self.active_window = active;
                            }
                            if launch_result.is_ok() {
                                if let Some(ref pane_name) = spec.name {
                                    if !pane_name.is_empty() {
                                        self.apply_inline_pane_name(response_pane_id, pane_name);
                                    }
                                }
                            }
                            // Skip rest of split path
                            self.reply_spawn_pane(&spec, response_pane_id, &launch_result);
                            return;
                        };
                        let keep_focus = spec.no_focus || spec.from_pane_id.is_some();
                        log::info!(
                            "pane_ipc: spawn_pane terminal layout={layout_str} vertical={vertical} new_pane_first={new_pane_first} initial_cmd={initial_cmd:?} ephemeral={} target_win={target_win} keep_focus={keep_focus}",
                            spec.ephemeral
                        );
                        response_pane_id = self.spawn_terminal_pane_at(
                            target_win,
                            target_tile,
                            vertical,
                            new_pane_first,
                            initial_cmd.as_deref(),
                            spec.ephemeral,
                            cwd_override,
                            keep_focus,
                        );
                        if spec.no_focus {
                            self.active_window = active;
                        }
                    }
                } else if let crate::app::launch_spec::PaneLaunchTarget::Path(app_path) =
                    &spec.target
                {
                    let (target_win, orig_focused_in_target) = if let Some(from_id) =
                        spec.from_pane_id
                    {
                        match self.find_pane_in_any_window(from_id) {
                            Some((fw, ft)) => {
                                log::info!(
                                    "pane_ipc: spawn_pane path: targeting from_pane_id={from_id} win_idx={fw}"
                                );
                                let saved = self.windows[fw].focused_pane;
                                self.active_window = fw;
                                self.set_window_focused_pane(fw, ft);
                                (fw, saved)
                            }
                            None => {
                                log::warn!(
                                    "pane_ipc: spawn_pane path: from_pane_id={from_id} not found, using focused pane"
                                );
                                (active, self.windows[active].focused_pane)
                            }
                        }
                    } else {
                        (active, self.windows[active].focused_pane)
                    };
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
                    let (target_win, orig_focused_in_target) = if let Some(from_id) =
                        spec.from_pane_id
                    {
                        match self.find_pane_in_any_window(from_id) {
                            Some((fw, ft)) => {
                                log::info!(
                                    "pane_ipc: spawn_pane app: targeting from_pane_id={from_id} win_idx={fw}"
                                );
                                let saved = self.windows[fw].focused_pane;
                                self.active_window = fw;
                                self.set_window_focused_pane(fw, ft);
                                (fw, saved)
                            }
                            None => {
                                log::warn!(
                                    "pane_ipc: spawn_pane app: from_pane_id={from_id} not found, using focused pane"
                                );
                                (active, self.windows[active].focused_pane)
                            }
                        }
                    } else {
                        (active, self.windows[active].focused_pane)
                    };
                    // CLI/spawn-request app opens default to a sibling split, never
                    // an overlay takeover of the caller's pane. Manifest `[launch]
                    // placement` still overrides the default (stint 0330).
                    let placement = crate::pane_ops::cli_open_placement(
                        spec.layout.clone(),
                        self.registry.placement_for(type_id),
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
            crate::app_protocol::AppRequest::SendToPane {
                pane_id,
                text,
                submit,
                response_file,
            } => {
                log::info!(
                    "pane_ipc: kind=send_to_pane pane_id={pane_id} len={} submit={submit} windows={} response_file={response_file:?}",
                    text.len(),
                    self.windows.len()
                );
                let text_with_newlines = text.replace("\\n", "\n");
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
                                    events: vec![egui::Event::Text(text_with_newlines)],
                                    ..Default::default()
                                },
                            );
                            self.ctx.request_repaint();
                            log::info!(
                                "pane_ipc: send_to_pane: app pane_id={pane_id} text_chars={}",
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
            crate::app_protocol::AppRequest::PaneHeartbeat {
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
            crate::app_protocol::AppRequest::KeyPane {
                pane_id,
                key,
                response_file,
            } => {
                log::info!(
                    "pane_ipc: kind=key_pane pane_id={pane_id} key_chars={}",
                    key.chars().count()
                );
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
                            let bytes = super::key_str_to_pty_bytes(key);
                            term.backend
                                .process_command(egui_term::BackendCommand::Write(bytes));
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
                                                passthrough_raw =
                                                    super::key_str_to_egui_raw_input(key);
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
            crate::app_protocol::AppRequest::DropFile {
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
            crate::app_protocol::AppRequest::ClickPane {
                pane_id,
                x,
                y,
                button,
                response_file,
            } => {
                log::info!(
                    "pane_ipc: kind=click_pane pane_id={pane_id} x={x} y={y} button={button:?}"
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
            crate::app_protocol::AppRequest::ClickPaneNode {
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
            crate::app_protocol::AppRequest::DragPane {
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
            crate::app_protocol::AppRequest::CapturePane {
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
                            if let Some(cursor) = from_cursor {
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
                                log::info!(
                                    "pane_ipc: capture_pane: lines={} cursor={lw}",
                                    captured.len()
                                );
                                Ok(serde_json::json!({
                                    "lines": captured,
                                    "cursor": lw,
                                    "missed": false,
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
            crate::app_protocol::AppRequest::PaneStatus {
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
            crate::app_protocol::AppRequest::Screenshot {
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
            crate::app_protocol::AppRequest::GetPaneState {
                pane_id,
                response_file,
            } => {
                log::info!(
                    "pane_ipc: kind=get_pane_state pane_id={pane_id} response_file={response_file:?}"
                );
                let json_str = match self.windows.iter().find_map(|win| win.panes.get(pane_id)) {
                    None => {
                        log::warn!("pane_ipc: get_pane_state: pane_id={pane_id} not found");
                        serde_json::json!({"error": format!("pane {pane_id} not found")})
                            .to_string()
                    }
                    Some(pane) => {
                        if let Some(app_pane) = pane.as_app() {
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
                            .to_string()
                        } else if let Some(term) = pane.as_terminal() {
                            let title = term.name.clone().unwrap_or_else(|| "terminal".to_string());
                            serde_json::json!({
                                "pane_id": pane_id,
                                "type": "terminal",
                                "title": title,
                            })
                            .to_string()
                        } else {
                            serde_json::json!({
                                "pane_id": pane_id,
                                "type": "unknown",
                            })
                            .to_string()
                        }
                    }
                };
                write_response(response_file, json_str.as_bytes());
            }
            crate::app_protocol::AppRequest::SendAppAction {
                pane_id,
                action,
                args,
                response_file,
            } => {
                log::info!(
                    "pane_ipc: kind=send_app_action pane_id={pane_id} action={action:?} args={args:?}"
                );
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
            crate::app_protocol::AppRequest::SetAgentState {
                pane_id,
                state,
                agent,
                detail,
                session_id,
            } => {
                log::info!(
                    "pane_ipc: kind=set_agent_state pane_id={pane_id} agent={agent} state={state:?} detail_present={}",
                    detail.is_some()
                );
                let mut found = false;
                for win in &mut self.windows {
                    if let Some(pane) = win.panes.get_mut(pane_id) {
                        found = pane.set_agent(Some(crate::app_protocol::PaneAgentState {
                            pane_id: *pane_id,
                            state: state.clone(),
                            agent: agent.clone(),
                            detail: detail.clone(),
                            session_id: session_id.clone(),
                        }));
                        break;
                    }
                }
                if found {
                    log::info!("pane_ipc: set_agent_state: pane_id={pane_id} stored on pane");
                    // Fast path: answer a parked `pane new --agent` spawn the
                    // frame the hook report lands. The host-observed detector
                    // path is picked up by the level-triggered re-check in
                    // `service_pending_agent_boots` instead.
                    self.complete_agent_boots(*pane_id);
                } else {
                    log::warn!(
                        "pane_ipc: set_agent_state: pane_id={pane_id} not found or not agent-addressable"
                    );
                }
            }
            crate::app_protocol::AppRequest::SetPipStatus { pane_id, status } => {
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
            crate::app_protocol::AppRequest::GetAgentStates { response_file } => {
                log::info!("pane_ipc: kind=get_agent_states response_file={response_file:?}");
                let states: Vec<&crate::app_protocol::PaneAgentState> = self
                    .windows
                    .iter()
                    .flat_map(|win| win.panes.values())
                    .filter_map(|pane| pane.agent())
                    .collect();
                let json_str = serde_json::to_string(&states).unwrap_or_else(|_| "[]".to_string());
                write_response(response_file, json_str.as_bytes());
            }
            crate::app_protocol::AppRequest::Notify {
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
                let internal_id = notify_id.clone().unwrap_or_else(|| {
                    format!(
                        "__host__:{}",
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_nanos())
                            .unwrap_or(0)
                    )
                });
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
                if let Some((resolved_pane_id, resolved_context_id, resolved_window_id)) =
                    resolved
                {
                    log::info!(
                        "pane_ipc: peer identity: notify sender resolved to pane={resolved_pane_id} context={resolved_context_id} window={resolved_window_id} peer_pid={peer_pid:?}"
                    );
                }
                if source_pane_id.is_some() || source_context_id.is_some() {
                    let claimed_matches = resolved
                        .is_some_and(|(pane_id, context_id, _)| {
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
                    crate::app_protocol::NotifyScope::Global => {
                        crate::app_protocol::NotifyScope::Global
                    }
                    crate::app_protocol::NotifyScope::Context if caller_context_id.is_some() => {
                        crate::app_protocol::NotifyScope::Context
                    }
                    crate::app_protocol::NotifyScope::Window if caller_window_id.is_some() => {
                        crate::app_protocol::NotifyScope::Window
                    }
                    unresolvable => {
                        log::warn!(
                            "pane_ipc: notify: scope {unresolvable:?} needs a caller identity but none resolved \
                             (source_context_id={source_context_id:?} source_pane_id={source_pane_id:?}) — \
                             escalating to global so the notification stays reachable"
                        );
                        crate::app_protocol::NotifyScope::Global
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
                self.enqueue_notification(
                    crate::app::notifications::NotifySource::Cli,
                    PendingNotification {
                        notify_id: internal_id,
                        sender_pane_id: 0,
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
                        enqueued_at: std::time::Instant::now(),
                        tombstoned: false,
                        deliver_after: None,
                    },
                );
            }
            crate::app_protocol::AppRequest::DismissNotification {
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
                if let Some((resolved_pane_id, resolved_context_id, resolved_window_id)) =
                    resolved
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
            crate::app_protocol::AppRequest::CreateContext {
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
                        if let Some(n) = name {
                            let idx = self.router.len() - 1;
                            self.router.get_mut(idx).name = n.clone();
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
                        self.router.get_mut(idx).name = n.clone();
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
                            Some(cmd.as_str()),
                            false,
                            None,
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
                        let response = serde_json::json!({
                            "context_id": new_ctx_id,
                            "windows": windows_info,
                        });
                        write_json_response(rf, response);
                    }
                }
                self.mark_workspace_dirty();
            }
            crate::app_protocol::AppRequest::CreateSubContext {
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
                log::info!(
                    "pane_ipc: kind=create_sub_context name={name:?} parent_ctx_id={parent_ctx_id} \
                     panes={} layout={layout:?} focus={focus} root={} anchor_pane={anchor_pane:?}",
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
                    layout: *layout,
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
                     pane_count={} layout={layout:?} root={} launched={launched:?}",
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
            crate::app_protocol::AppRequest::FocusContext { root } => {
                log::warn!(
                    "pane_ipc: FocusContext ignored — CWD-based auto-switch removed (root={})",
                    root.display()
                );
            }
            crate::app_protocol::AppRequest::SetContextRoot { root, context_id } => {
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
            crate::app_protocol::AppRequest::SetContextDescription {
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
            crate::app_protocol::AppRequest::ZoomIntoContext { context_id } => {
                log::info!("pane_ipc: kind=zoom_into_context context_id={context_id}");
                if let Some(ctx_idx) = self.router.position(|c| c.context_id == *context_id) {
                    let current_ctx_id = self.router.active().context_id;
                    let current_win_id = self.windows[self.active_window].window_id;
                    let current_focused = self.windows[self.active_window].focused_pane;
                    self.router
                        .push_depth(current_ctx_id, current_win_id, current_focused);
                    self.switch_workspace(ctx_idx);
                }
            }
            crate::app_protocol::AppRequest::ZoomOutOfContext => {
                log::info!(
                    "pane_ipc: kind=zoom_out_of_context depth={}",
                    self.router.current_depth()
                );
                self.zoom_out_of_context();
            }
            crate::app_protocol::AppRequest::PushPaneToSubcontext { name, pane_id } => {
                log::info!(
                    "pane_ipc: kind=push_pane_to_subcontext name={name:?} pane_id={pane_id:?}"
                );
                self.push_pane_to_subcontext(name.clone(), *pane_id);
            }
            crate::app_protocol::AppRequest::ListPermissions { response_file } => {
                log::info!("pane_ipc: kind=list_permissions response_file={response_file:?}");
                self.handle_list_permissions(response_file);
            }
            crate::app_protocol::AppRequest::SetPermission {
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
            crate::app_protocol::AppRequest::Wake => {
                // No-op: the wake effect is the socket listener's repaint
                // request — by the time this arm runs, a frame is already
                // in flight and queued work (spawn-queue, channels) drains.
                log::debug!("pane_ipc: kind=wake (no-op)");
            }
            crate::app_protocol::AppRequest::Shutdown => {
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

            // Dual-write the unified broker store so grants.toml stays in
            // lockstep with the legacy permissions.toml until all call sites
            // read through the broker (permissions-broker spec, Phase A).
            let mut grants = crate::broker::GrantStore::load_or_default(&self.permission_store_dir);
            grants.record_app_capability(
                app_id,
                &ws,
                cap,
                crate::broker::Decision::from_permission_state(new_state),
            );
            grants.save();

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
        use crate::app_protocol::AgentState;
        for window in self.windows.iter_mut() {
            for (pane_id, pane) in window.panes.iter_mut() {
                let Some(t) = pane.as_terminal_mut() else {
                    continue;
                };
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
                            crate::app_protocol::PaneAgentState {
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
                    t.observed_agent = observed;
                }
            }
        }
    }

    pub(crate) fn tick_scheduler(&mut self) {
        // Load routines from every context root
        let roots: Vec<std::path::PathBuf> = self
            .router
            .iter()
            .map(|ctx| ctx.root.clone())
            .collect();
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
            Some(id) => (crate::app_protocol::NotifyScope::Context, id),
            None => (crate::app_protocol::NotifyScope::Global, 0),
        };
        // Millis alone can collide when two routine issues surface in the same
        // tick; notify_id is an identity key, so disambiguate with a counter.
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let queued = self.enqueue_notification(
            crate::app::notifications::NotifySource::HostInternal,
            crate::app::notifications::PendingNotification {
                notify_id: format!("routine-{millis}-{seq}"),
                sender_pane_id: 0,
                dismiss_owner_pane_id: 0,
                source_context_id,
                source_window_id: 0,
                title: title.to_string(),
                body: body.to_string(),
                kind: crate::app_protocol::NotifyKind::Message,
                options: vec![],
                input_prompt: None,
                required: false,
                scope,
                image_inline: None,
                image_pipe_id: None,
                response_file: None,
                timeout_secs: None,
                on_dismiss: None,
                enqueued_at: std::time::Instant::now(),
                tombstoned: false,
                deliver_after: None,
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
            self.spawn_terminal_pane_at(
                win,
                tile,
                true,
                false,
                Some(command),
                ephemeral,
                cwd,
                false,
            )
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
            let request = crate::app_protocol::AppRequest::SpawnPane {
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
            };
            let Ok(spec) = crate::app::launch_spec::PaneLaunchSpec::from_spawn_pane(&request)
            else {
                log::warn!("spawn-queue: invalid spawn request, skipping");
                continue;
            };
            let origin = val["origin"].as_str().unwrap_or("unknown");
            let age_secs = spawn_file_age_secs(
                val["queued_at_ms"].as_u64(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0),
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
    pub(super) fn service_python_pane_runtimes(&mut self) {
        for win in &mut self.windows {
            for pane in win.panes.values_mut() {
                if let Some(app) = pane.as_app_mut() {
                    if let crate::host::pane::AppRuntime::Python(python) = &mut app.runtime {
                        python.service_external_io();
                    }
                }
            }
        }
    }

    pub(super) fn drain_pty_events(&mut self) {
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
                PtyEvent::Exit => {
                    for win in &mut self.windows {
                        if let Some(pane) = win.panes.get_mut(&id) {
                            if let Some(t) = pane.as_terminal_mut() {
                                t.exited = true;
                                log::info!(
                                    "pty: pane {id} process exited ephemeral={}",
                                    t.ephemeral
                                );
                                if t.ephemeral {
                                    panes_to_close.push(id);
                                }
                            }
                            break;
                        }
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
