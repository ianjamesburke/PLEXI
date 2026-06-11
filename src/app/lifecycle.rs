//! Lifecycle methods — per-frame drain and tick operations on PlexiApp.

use egui_term::PtyEvent;

use super::PendingNotification;
use super::PlexiApp;

const MAX_SLOT_CONTENT_SIZE: usize = 10 * 1024 * 1024;

fn write_json_response(response_file: &str, value: serde_json::Value) {
    match serde_json::to_string(&value) {
        Ok(json) => {
            if let Err(e) = std::fs::write(response_file, json) {
                log::error!("pane_ipc: could not write response file {response_file:?}: {e}");
            }
        }
        Err(e) => {
            log::error!("pane_ipc: could not serialize response for {response_file:?}: {e}");
        }
    }
}

fn slot_error(response_file: &str, message: impl Into<String>) {
    write_json_response(
        response_file,
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
                            }
                        }
                    }
                    if !found {
                        log::warn!("pane_ipc: set_pane_title: pane_id={pane_id} not found");
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
                                    let name =
                                        t.name.clone().unwrap_or_else(|| "terminal".to_string());
                                    let cwd =
                                        crate::host::shell::get_pid_cwd(t.backend.child_pid())
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
                            let focused =
                                win_idx == active_win && focused_pane_id == Some(*pane_id);
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
                            }));
                        }
                    }
                    let json_str =
                        serde_json::to_string(&entries).unwrap_or_else(|_| "[]".to_string());
                    if let Err(e) = std::fs::write(response_file, &json_str) {
                        log::error!("pane_ipc: list_panes: could not write response file {response_file:?}: {e}");
                    }
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
                                "path": ctx.path.to_string_lossy(),
                                "root": ctx.root.as_ref().map(|p| p.to_string_lossy().into_owned()),
                                "description": ctx.description,
                                "parent_id": ctx.parent_id,
                                "depth": ctx.depth,
                                "is_active": ctx.context_id == active_ctx_id,
                            })
                        })
                        .collect();
                    let json_str =
                        serde_json::to_string(&entries).unwrap_or_else(|_| "[]".to_string());
                    let temp_file = format!("{}.tmp", response_file);
                    if let Err(e) = std::fs::write(&temp_file, &json_str) {
                        log::error!("pane_ipc: list_contexts: could not write temp response file {temp_file:?}: {e}");
                    } else if let Err(e) = std::fs::rename(&temp_file, &response_file) {
                        log::error!("pane_ipc: list_contexts: could not rename temp response file to {response_file:?}: {e}");
                        let _ = std::fs::remove_file(&temp_file);
                    }
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
                            let focused =
                                win_idx == active_win && focused_pane_id == Some(*pane_id);
                            let agent = pane.agent();
                            let info = match pane {
                                crate::host::pane::Pane::Terminal(t) => {
                                    let cwd =
                                        crate::host::shell::get_pid_cwd(t.backend.child_pid())
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
                            if let Err(e) = std::fs::write(response_file, &json_str) {
                                log::error!("pane_ipc: get_pane_info: could not write response file {:?}: {e}", response_file);
                            }
                            found = true;
                            break 'outer;
                        }
                    }
                    if !found {
                        log::warn!("pane_ipc: get_pane_info: pane_id={pane_id} not found");
                        let json_str = format!("{{\"error\":\"pane {pane_id} not found\"}}");
                        if let Err(e) = std::fs::write(response_file, &json_str) {
                            log::error!(
                                "pane_ipc: get_pane_info: could not write error response: {e}"
                            );
                        }
                    }
                }
                crate::app_protocol::AppRequest::GetPreviousPaneInfo { response_file } => {
                    log::info!(
                        "pane_ipc: kind=get_previous_pane_info response_file={:?}",
                        response_file
                    );
                    let mut found = false;
                    'prev_outer: for (window_id, tile_id) in self.pane_focus_history.iter().rev() {
                        if let Some(win) = self.windows.iter().find(|w| w.window_id == *window_id) {
                            if let Some(egui_tiles::Tile::Pane(pane_id)) =
                                win.tree.tiles.get(*tile_id)
                            {
                                if let Some(pane) = win.panes.get(pane_id) {
                                    let info = match pane {
                                        crate::host::pane::Pane::Terminal(t) => {
                                            let cwd = crate::host::shell::get_pid_cwd(
                                                t.backend.child_pid(),
                                            )
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
                                    let json_str = serde_json::to_string(&info)
                                        .unwrap_or_else(|_| "{}".to_string());
                                    let temp_file = format!("{}.tmp", response_file);
                                    if let Err(e) = std::fs::write(&temp_file, &json_str) {
                                        log::error!("pane_ipc: get_previous_pane_info: could not write temp response file {temp_file:?}: {e}");
                                    } else if let Err(e) =
                                        std::fs::rename(&temp_file, response_file)
                                    {
                                        log::error!("pane_ipc: get_previous_pane_info: could not rename temp response file to {:?}: {e}", response_file);
                                        let _ = std::fs::remove_file(&temp_file);
                                    }
                                    found = true;
                                    break 'prev_outer;
                                }
                            }
                        }
                    }
                    if !found {
                        log::warn!(
                            "pane_ipc: get_previous_pane_info: no previous pane found in history"
                        );
                        let json_str = "{\"error\":\"no previous pane in history\"}";
                        let temp_file = format!("{}.tmp", response_file);
                        if let Err(e) = std::fs::write(&temp_file, json_str) {
                            log::error!("pane_ipc: get_previous_pane_info: could not write temp error response {temp_file:?}: {e}");
                        } else if let Err(e) = std::fs::rename(&temp_file, response_file) {
                            log::error!("pane_ipc: get_previous_pane_info: could not rename temp error response to {:?}: {e}", response_file);
                            let _ = std::fs::remove_file(&temp_file);
                        }
                    }
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
                        continue;
                    }
                    if *append && *replace {
                        slot_error(response_file, "use only one of append or replace");
                        continue;
                    }
                    if content.len() > MAX_SLOT_CONTENT_SIZE {
                        slot_error(
                            response_file,
                            format!(
                                "slot '{slot_name}' content size {} exceeds 10485760 bytes",
                                content.len()
                            ),
                        );
                        continue;
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
                        continue;
                    };
                    let context_root = self
                        .router
                        .iter()
                        .find(|ctx| ctx.context_id == context_id)
                        .and_then(|ctx| ctx.root.clone());
                    let slot_dir = slot_base_dir(context_root.as_deref()).join(pane_id.to_string());
                    let slot_path = slot_dir.join(slot_name);
                    let Some(pane) = self.windows[win_idx].panes.get_mut(pane_id) else {
                        slot_error(response_file, format!("pane {pane_id} not found"));
                        continue;
                    };
                    let Some(slots) = pane.slots_mut() else {
                        slot_error(
                            response_file,
                            format!("pane {pane_id} does not support slots"),
                        );
                        continue;
                    };
                    let existing_path = slots.get(slot_name).cloned().unwrap_or(slot_path.clone());
                    let exists = existing_path.exists();
                    if exists && !*append && !*replace {
                        slot_error(
                            response_file,
                            format!(
                                "slot '{slot_name}' already exists — use --append to add to it or --replace to overwrite it"
                            ),
                        );
                        continue;
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
                            continue;
                        }
                    }
                    if let Err(e) = std::fs::create_dir_all(&slot_dir) {
                        slot_error(
                            response_file,
                            format!(
                                "could not create slot directory {}: {e}",
                                slot_dir.display()
                            ),
                        );
                        continue;
                    }
                    let write_result = if *append {
                        use std::io::Write as _;
                        std::fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(&slot_path)
                            .and_then(|mut f| f.write_all(content).map(|_| ()))
                    } else {
                        std::fs::write(&slot_path, content)
                    };
                    if let Err(e) = write_result {
                        slot_error(
                            response_file,
                            format!("could not write slot '{slot_name}': {e}"),
                        );
                        continue;
                    }
                    let absolute = slot_path
                        .canonicalize()
                        .unwrap_or_else(|_| slot_path.clone());
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
                }
                crate::app_protocol::AppRequest::SlotRead {
                    pane_id,
                    slot_name,
                    response_file,
                } => {
                    log::info!("pane_ipc: kind=slot_read pane_id={pane_id} slot={slot_name:?}");
                    if let Err(msg) = validate_slot_name(slot_name) {
                        slot_error(response_file, msg);
                        continue;
                    }
                    let path = self
                        .windows
                        .iter()
                        .find_map(|win| win.panes.get(pane_id))
                        .and_then(|pane| pane.slots())
                        .and_then(|slots| slots.get(slot_name).cloned());
                    let Some(path) = path else {
                        slot_error(response_file, format!("slot '{slot_name}' not found"));
                        continue;
                    };
                    match std::fs::read(&path) {
                        Ok(bytes) => {
                            if let Err(e) = std::fs::write(response_file, bytes) {
                                log::error!(
                                    "pane_ipc: slot_read: could not write response file {response_file:?}: {e}"
                                );
                            }
                        }
                        Err(e) => {
                            slot_error(
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
                        continue;
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
                        continue;
                    };
                    let context_root = self
                        .router
                        .iter()
                        .find(|ctx| ctx.context_id == context_id)
                        .and_then(|ctx| ctx.root.clone());
                    let fallback_path = slot_base_dir(context_root.as_deref())
                        .join(pane_id.to_string())
                        .join(slot_name);
                    let Some(pane) = self.windows[win_idx].panes.get_mut(pane_id) else {
                        slot_error(response_file, format!("pane {pane_id} not found"));
                        continue;
                    };
                    let Some(slots) = pane.slots_mut() else {
                        slot_error(
                            response_file,
                            format!("pane {pane_id} does not support slots"),
                        );
                        continue;
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
                                continue;
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
                    let mut roots = vec![crate::config::config_dir().join("slots")];
                    for root in self.router.iter().filter_map(|ctx| ctx.root.as_ref()) {
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
                            let Some(pane_id) =
                                entry.file_name().to_string_lossy().parse::<u64>().ok()
                            else {
                                continue;
                            };
                            if live_panes.contains(&pane_id) {
                                continue;
                            }
                            paths.push(path);
                        }
                    }
                    paths.sort();
                    let mut cleaned = Vec::new();
                    for path in paths {
                        if !*dry_run {
                            if let Err(e) = std::fs::remove_dir_all(&path) {
                                slot_error(
                                    response_file,
                                    format!(
                                        "could not remove slot directory {}: {e}",
                                        path.display()
                                    ),
                                );
                                continue;
                            }
                        }
                        cleaned.push(path.to_string_lossy().into_owned());
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
                crate::app_protocol::AppRequest::SpawnPane {
                    type_id,
                    layout,
                    args,
                    ephemeral,
                    response_file,
                    from_pane_id,
                    cwd,
                    no_focus,
                    path,
                    workspace_root,
                    name,
                    ..
                } => {
                    log::info!("pane_ipc: kind=spawn_pane type_id={type_id} path={path:?} layout={layout:?} ephemeral={ephemeral} no_focus={no_focus} from_pane_id={from_pane_id:?} cwd={cwd:?} workspace_root={workspace_root:?} response_file={response_file:?}");
                    let new_pane_id = self.host.next_pane_id();

                    let active = self.active_window;
                    let cwd_override: Option<std::path::PathBuf> =
                        cwd.as_deref().map(std::path::PathBuf::from);
                    let mut launch_result: Result<(), String> = Ok(());
                    if type_id == "terminal" {
                        let layout_str = layout.as_deref().unwrap_or("split_h");
                        let initial_cmd = super::cmd_from_args(args);
                        if layout_str == "new_window" {
                            // Create a new spatial grid window to the right of the
                            // current context row instead of splitting the active pane.
                            let ws_id = self.router.active().context_id;
                            let active_y = self.windows[self.active_window].grid_y;
                            let max_x = self
                                .windows
                                .iter()
                                .filter(|w| w.context_id == ws_id && w.grid_y == active_y)
                                .map(|w| w.grid_x)
                                .max();
                            let new_x = max_x.map(|x| x + 1).unwrap_or(1);
                            log::info!("pane_ipc: spawn_pane terminal layout=new_window grid=({new_x},{active_y}) initial_cmd={initial_cmd:?} ephemeral={ephemeral}");
                            self.create_page_at(
                                new_x,
                                active_y,
                                initial_cmd.as_deref(),
                                *ephemeral,
                            );
                            if *no_focus {
                                self.active_window = active;
                            }
                        } else if layout_str == "tab" {
                            log::info!("pane_ipc: spawn_pane terminal layout=tab initial_cmd={initial_cmd:?} ephemeral={ephemeral}");
                            let original_focused = self.windows[active].focused_pane;
                            self.new_tab(initial_cmd.as_deref(), *ephemeral);
                            if *no_focus {
                                self.active_window = active;
                                self.restore_window_focused_pane(active, original_focused);
                            }
                        } else {
                            let vertical =
                                matches!(layout_str, "split_h" | "split_right" | "split_left");
                            let new_pane_first = matches!(layout_str, "split_above" | "split_left");
                            // Resolve target window and tile: from_pane_id wins (cross-window),
                            // then fall back to the active window's focused pane.
                            let (target_win, target_tile) = if let Some(from_id) = from_pane_id {
                                match self.find_pane_in_any_window(*from_id) {
                                    Some(loc) => {
                                        log::info!("pane_ipc: spawn_pane: targeting from_pane_id={from_id} in win_idx={}", loc.0);
                                        loc
                                    }
                                    None => {
                                        log::warn!("pane_ipc: spawn_pane: from_pane_id={from_id} not found in any window, using focused pane");
                                        if let Some(tile) = self.windows[active]
                                            .focused_pane
                                            .or(self.windows[active].tree.root)
                                        {
                                            (active, tile)
                                        } else {
                                            // Window is empty — let split_focused handle it
                                            log::info!("pane_ipc: spawn_pane terminal layout={layout_str} (empty context fallback)");
                                            self.split_focused(
                                                vertical,
                                                initial_cmd.as_deref(),
                                                *ephemeral,
                                                new_pane_first,
                                                cwd_override,
                                            );
                                            if *no_focus {
                                                self.active_window = active;
                                            }
                                            if let Some(ref pane_name) = name {
                                                if !pane_name.is_empty() {
                                                    self.apply_inline_pane_name(
                                                        new_pane_id,
                                                        pane_name,
                                                    );
                                                }
                                            }
                                            if let Some(rf) = response_file {
                                                let json = format!("{{\"pane_id\":{new_pane_id}}}");
                                                if let Err(e) = std::fs::write(rf, &json) {
                                                    log::error!("pane_ipc: spawn_pane: could not write response file: {e}");
                                                }
                                            }
                                            continue;
                                        }
                                    }
                                }
                            } else if let Some(tile) = self.windows[active]
                                .focused_pane
                                .or(self.windows[active].tree.root)
                            {
                                (active, tile)
                            } else {
                                // Truly empty window — fall back to split_focused
                                log::info!("pane_ipc: spawn_pane terminal layout={layout_str} vertical={vertical} new_pane_first={new_pane_first} initial_cmd={initial_cmd:?} ephemeral={ephemeral}");
                                self.split_focused(
                                    vertical,
                                    initial_cmd.as_deref(),
                                    *ephemeral,
                                    new_pane_first,
                                    cwd_override,
                                );
                                if *no_focus {
                                    self.active_window = active;
                                }
                                if let Some(ref pane_name) = name {
                                    if !pane_name.is_empty() {
                                        self.apply_inline_pane_name(new_pane_id, pane_name);
                                    }
                                }
                                // Skip rest of split path
                                if let Some(rf) = response_file {
                                    let json = format!("{{\"pane_id\":{new_pane_id}}}");
                                    if let Err(e) = std::fs::write(rf, &json) {
                                        log::error!("pane_ipc: spawn_pane: could not write response file: {e}");
                                    }
                                }
                                continue;
                            };
                            let keep_focus = *no_focus || from_pane_id.is_some();
                            log::info!("pane_ipc: spawn_pane terminal layout={layout_str} vertical={vertical} new_pane_first={new_pane_first} initial_cmd={initial_cmd:?} ephemeral={ephemeral} target_win={target_win} keep_focus={keep_focus}");
                            let _ = self.spawn_terminal_pane_at(
                                target_win,
                                target_tile,
                                vertical,
                                new_pane_first,
                                initial_cmd.as_deref(),
                                *ephemeral,
                                cwd_override,
                                keep_focus,
                            );
                            if *no_focus {
                                self.active_window = active;
                            }
                        }
                    } else if let Some(path_str) = path {
                        let ws_root = workspace_root.as_deref().map(std::path::PathBuf::from);
                        let (target_win, orig_focused_in_target) = if let Some(from_id) =
                            from_pane_id
                        {
                            match self.find_pane_in_any_window(*from_id) {
                                Some((fw, ft)) => {
                                    log::info!("pane_ipc: spawn_pane path: targeting from_pane_id={from_id} win_idx={fw}");
                                    let saved = self.windows[fw].focused_pane;
                                    self.active_window = fw;
                                    self.set_window_focused_pane(fw, ft);
                                    (fw, saved)
                                }
                                None => {
                                    log::warn!("pane_ipc: spawn_pane path: from_pane_id={from_id} not found, using focused pane");
                                    (active, self.windows[active].focused_pane)
                                }
                            }
                        } else {
                            (active, self.windows[active].focused_pane)
                        };
                        launch_result =
                            self.launch_app_by_path_with_layout(path_str, layout.clone(), ws_root);
                        if from_pane_id.is_some() {
                            self.active_window = active;
                            // Undo the temporary focus redirect when launch failed.
                            if launch_result.is_err() {
                                self.restore_window_focused_pane(
                                    target_win,
                                    orig_focused_in_target,
                                );
                            }
                        }
                        if *no_focus {
                            self.active_window = active;
                            self.restore_window_focused_pane(target_win, orig_focused_in_target);
                        }
                    } else {
                        let (target_win, orig_focused_in_target) = if let Some(from_id) =
                            from_pane_id
                        {
                            match self.find_pane_in_any_window(*from_id) {
                                Some((fw, ft)) => {
                                    log::info!("pane_ipc: spawn_pane app: targeting from_pane_id={from_id} win_idx={fw}");
                                    let saved = self.windows[fw].focused_pane;
                                    self.active_window = fw;
                                    self.set_window_focused_pane(fw, ft);
                                    (fw, saved)
                                }
                                None => {
                                    log::warn!("pane_ipc: spawn_pane app: from_pane_id={from_id} not found, using focused pane");
                                    (active, self.windows[active].focused_pane)
                                }
                            }
                        } else {
                            (active, self.windows[active].focused_pane)
                        };
                        launch_result = self.launch_app_by_id_with_layout(
                            type_id,
                            layout.clone(),
                            args,
                            cwd_override,
                        );
                        if from_pane_id.is_some() {
                            self.active_window = active;
                            // Undo the temporary focus redirect when launch failed.
                            if launch_result.is_err() {
                                self.restore_window_focused_pane(
                                    target_win,
                                    orig_focused_in_target,
                                );
                            }
                        }
                        if *no_focus {
                            self.active_window = active;
                            self.restore_window_focused_pane(target_win, orig_focused_in_target);
                        }
                    }
                    if type_id == "terminal" && launch_result.is_ok() {
                        if let Some(ref pane_name) = name {
                            if !pane_name.is_empty() {
                                self.apply_inline_pane_name(new_pane_id, pane_name);
                            }
                        }
                    }
                    if let Some(rf) = response_file {
                        let json = match &launch_result {
                            Ok(()) => format!("{{\"pane_id\":{new_pane_id}}}"),
                            Err(msg) => {
                                log::warn!("pane_ipc: spawn_pane: launch failed, returning error to caller: {msg}");
                                format!(
                                    "{{\"error\":{}}}",
                                    serde_json::to_string(msg)
                                        .unwrap_or_else(|_| format!("\"{msg}\""))
                                )
                            }
                        };
                        if let Err(e) = std::fs::write(rf, &json) {
                            log::error!("pane_ipc: spawn_pane: could not write response file: {e}");
                        }
                    }
                }
                crate::app_protocol::AppRequest::SendToPane {
                    pane_id,
                    text,
                    response_file,
                } => {
                    log::info!("pane_ipc: kind=send_to_pane pane_id={pane_id} len={} windows={} response_file={response_file:?}", text.len(), self.windows.len());
                    let text_with_newlines = text.replace("\\n", "\n");
                    let result = match self
                        .windows
                        .iter_mut()
                        .find_map(|win| win.panes.get_mut(pane_id))
                    {
                        None => {
                            log::warn!(
                                "pane_ipc: send_to_pane: pane_id={pane_id} not found in any window"
                            );
                            Err(format!("pane {pane_id} not found"))
                        }
                        Some(pane) => match pane.as_terminal_mut() {
                            None => {
                                log::warn!("pane_ipc: send_to_pane: pane_id={pane_id} is not a terminal pane");
                                Err(format!("pane {pane_id} is not a terminal pane"))
                            }
                            Some(term) => {
                                term.backend
                                    .process_command(egui_term::BackendCommand::Write(
                                        text_with_newlines.into_bytes(),
                                    ));
                                Ok(())
                            }
                        },
                    };
                    if let Some(rf) = response_file {
                        let json = match result {
                            Ok(()) => r#"{"ok":true}"#.to_string(),
                            Err(ref msg) => format!(
                                "{{\"error\":{}}}",
                                serde_json::to_string(msg).unwrap_or_else(|_| format!("\"{msg}\""))
                            ),
                        };
                        if let Err(e) = std::fs::write(rf, &json) {
                            log::error!(
                                "pane_ipc: send_to_pane: could not write response file: {e}"
                            );
                        }
                    }
                }
                crate::app_protocol::AppRequest::KeyPane {
                    pane_id,
                    key,
                    response_file,
                } => {
                    log::info!("pane_ipc: kind=key_pane pane_id={pane_id} key={key:?}");
                    let result = match self
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
                                Ok(())
                            } else if let Some(app_pane) = pane.as_app_mut() {
                                let (key_str, modifiers) = super::parse_key_str_to_event(key);
                                app_pane.runtime.queue_outbound_event(
                                    crate::app_protocol::PlexiEvent::Key {
                                        key: key_str,
                                        modifiers,
                                    },
                                );
                                Ok(())
                            } else {
                                Err(format!("pane {pane_id}: unknown pane type"))
                            }
                        }
                    };
                    if let Some(rf) = response_file {
                        let json = match &result {
                            Ok(()) => serde_json::json!({"ok": true}).to_string(),
                            Err(msg) => serde_json::json!({"error": msg}).to_string(),
                        };
                        if let Err(e) = std::fs::write(rf, &json) {
                            log::error!("pane_ipc: key_pane: could not write response file: {e}");
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
                    log::info!("pane_ipc: kind=capture_pane pane_id={pane_id} lines={lines} full_output={full_output} from_cursor={from_cursor:?} response_file={:?}", response_file);
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
                                log::warn!("pane_ipc: capture_pane: pane_id={pane_id} is not a terminal pane");
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
                    if let Err(e) = std::fs::write(response_file, &json_str) {
                        log::error!(
                            "pane_ipc: capture_pane: could not write response file {response_file:?}: {e}"
                        );
                    }
                }
                crate::app_protocol::AppRequest::GetPaneState {
                    pane_id,
                    response_file,
                } => {
                    log::info!("pane_ipc: kind=get_pane_state pane_id={pane_id} response_file={response_file:?}");
                    let json_str = match self.windows.iter().find_map(|win| win.panes.get(pane_id))
                    {
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
                                serde_json::json!({
                                    "pane_id": pane_id,
                                    "type": "app",
                                    "title": app_pane.name,
                                    "manifest_id": app_pane.manifest_id,
                                    "frame": frame,
                                })
                                .to_string()
                            } else if let Some(term) = pane.as_terminal() {
                                let title =
                                    term.name.clone().unwrap_or_else(|| "terminal".to_string());
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
                    if let Err(e) = std::fs::write(response_file, &json_str) {
                        log::error!("pane_ipc: get_pane_state: could not write response file {response_file:?}: {e}");
                    }
                }
                crate::app_protocol::AppRequest::SendAppAction {
                    pane_id,
                    action,
                    args,
                    response_file,
                } => {
                    log::info!("pane_ipc: kind=send_app_action pane_id={pane_id} action={action:?} args={args:?}");
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
                                app_pane.runtime.queue_outbound_event(
                                    crate::app_protocol::PlexiEvent::Action {
                                        action: action.clone(),
                                        args: args.clone(),
                                    },
                                );
                                Ok(())
                            } else {
                                log::warn!("pane_ipc: send_app_action: pane_id={pane_id} is not an app pane");
                                Err(format!("pane {pane_id} is not an app pane"))
                            }
                        }
                    };
                    if let Some(rf) = response_file {
                        let json = match &result {
                            Ok(()) => serde_json::json!({"ok": true}).to_string(),
                            Err(msg) => serde_json::json!({"error": msg}).to_string(),
                        };
                        if let Err(e) = std::fs::write(rf, &json) {
                            log::error!(
                                "pane_ipc: send_app_action: could not write response file: {e}"
                            );
                        }
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
                    } else {
                        log::warn!(
                            "pane_ipc: set_agent_state: pane_id={pane_id} not found or not agent-addressable"
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
                    let json_str =
                        serde_json::to_string(&states).unwrap_or_else(|_| "[]".to_string());
                    let temp = format!("{response_file}.tmp");
                    if let Err(e) = std::fs::write(&temp, &json_str) {
                        log::error!("pane_ipc: get_agent_states: could not write temp file: {e}");
                    } else if let Err(e) = std::fs::rename(&temp, response_file) {
                        log::error!("pane_ipc: get_agent_states: rename failed: {e}");
                        let _ = std::fs::remove_file(&temp);
                    }
                }
                crate::app_protocol::AppRequest::Notify {
                    level,
                    title,
                    body,
                    kind,
                    options,
                    input_prompt,
                    required,
                    priority,
                    image_inline,
                    image_pipe_id,
                    timeout_secs,
                    on_dismiss,
                    response_file,
                    scope,
                    ..
                } => {
                    if !self.notifications_enabled {
                        log::info!("pane_ipc: notify dropped — notifications disabled");
                        continue;
                    }
                    let internal_id = format!(
                        "__host__:{}",
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_nanos())
                            .unwrap_or(0)
                    );
                    log::info!(
                        "pane_ipc: kind=notify title={:?} choices={} scope={:?} response_file={:?}",
                        title,
                        options.len(),
                        scope,
                        response_file
                    );
                    self.pending_notifications.push(PendingNotification {
                        notify_id: internal_id.clone(),
                        sender_pane_id: 0,
                        source_context_id: self.router.active().context_id,
                        source_window_id: self.windows[self.active_window].window_id,
                        scope: scope.unwrap_or(crate::app_protocol::NotifyScope::Global),
                        level: level.clone(),
                        title: title.clone(),
                        body: body.clone(),
                        kind: kind.clone(),
                        options: options.clone(),
                        input_prompt: input_prompt.clone(),
                        required: *required,
                        priority: *priority,
                        image_inline: image_inline.clone(),
                        image_pipe_id: image_pipe_id.clone(),
                        response_file: response_file.clone(),
                        timeout_secs: *timeout_secs,
                        on_dismiss: on_dismiss.clone(),
                        enqueued_at: std::time::Instant::now(),
                        tombstoned: false,
                        deliver_after: None,
                    });
                    self.save_notifications();
                    let should_auto_open = !self.notifications_focus_mode
                        && *priority >= self.notifications_interrupt_threshold;
                    if should_auto_open {
                        self.show_notification_modal = true;
                        if self.current_notify_id.is_none() {
                            self.current_notify_id = Some(internal_id);
                        }
                    }
                }
                crate::app_protocol::AppRequest::CreateContext {
                    root,
                    name,
                    parent_name,
                    windows,
                    focus,
                    portal_direction,
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
                         windows={} focus={focus} direction={:?}",
                        root,
                        name,
                        parent_name,
                        windows.len(),
                        portal_direction
                    );
                    let mut ctx_ok = true;
                    // Track which context was active before and which context was just created.
                    // Windows must be placed in the new context regardless of focus state.
                    let orig_ctx_id = self.router.active().context_id;
                    let mut new_ctx_id = orig_ctx_id;
                    if let Some(pname) = parent_name {
                        let path = root.as_ref().cloned().unwrap_or_else(|| {
                            let p = self.router.active().path.clone();
                            if p.is_absolute() {
                                p
                            } else {
                                dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/"))
                            }
                        });
                        let current_ctx_id = self.router.active().context_id;
                        let current_win_id = self.windows[self.active_window].window_id;
                        let current_focused = self.windows[self.active_window].focused_pane;
                        if let Err(e) = self.new_child_context(
                            pname.as_str(),
                            path,
                            portal_vertical,
                            portal_first,
                        ) {
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
                                self.router.push_depth(
                                    current_ctx_id,
                                    current_win_id,
                                    current_focused,
                                );
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
                        // When no-focus, active context is still the parent — temporarily switch
                        // to the new context so create_page_at places windows there, then restore.
                        let need_restore = self.router.active().context_id != new_ctx_id;
                        if need_restore {
                            if let Some(idx) = self.router.position(|c| c.context_id == new_ctx_id)
                            {
                                self.switch_workspace(idx);
                            }
                        }
                        let active_y = self.windows[self.active_window].grid_y;
                        let mut new_x = self
                            .windows
                            .iter()
                            .filter(|w| w.context_id == new_ctx_id && w.grid_y == active_y)
                            .map(|w| w.grid_x)
                            .max()
                            .map(|x| x + 1)
                            .unwrap_or(1);
                        for cmd in windows {
                            log::info!(
                                "pane_ipc: create_context window ctx_id={new_ctx_id} grid_x={new_x} cmd={cmd:?}"
                            );
                            self.create_page_at(new_x, active_y, Some(cmd.as_str()), false);
                            new_x += 1;
                        }
                        if need_restore {
                            if let Some(idx) = self.router.position(|c| c.context_id == orig_ctx_id)
                            {
                                self.switch_workspace(idx);
                            }
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
                            if let Ok(s) = serde_json::to_string(&response) {
                                if let Err(e) = std::fs::write(&rf, s) {
                                    log::warn!(
                                        "pane_ipc: create_context failed to write response file {rf}: {e}"
                                    );
                                }
                            }
                        }
                    }
                    self.save_workspace();
                }
                crate::app_protocol::AppRequest::FocusContext { root } => {
                    log::warn!(
                        "pane_ipc: FocusContext ignored — CWD-based auto-switch removed (root={})",
                        root.display()
                    );
                }
                crate::app_protocol::AppRequest::SetContextRoot { root } => {
                    log::info!("pane_ipc: kind=set_context_root root={}", root.display());
                    self.set_active_context_root(root.clone());
                    self.save_workspace();
                }
                crate::app_protocol::AppRequest::SetContextDescription { description } => {
                    log::info!("pane_ipc: kind=set_context_description");
                    let idx = self.router.active_idx();
                    let trimmed = description.trim().to_string();
                    self.router.get_mut(idx).description = if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed)
                    };
                    self.save_workspace();
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
                    if let Some((parent_ctx_id, parent_win_id, focused_tile)) =
                        self.router.pop_depth()
                    {
                        if let Some(ctx_idx) =
                            self.router.position(|c| c.context_id == parent_ctx_id)
                        {
                            self.switch_workspace(ctx_idx);
                            if let Some(win_idx) = self
                                .windows
                                .iter()
                                .position(|w| w.window_id == parent_win_id)
                            {
                                self.active_window = win_idx;
                                self.windows[win_idx].focused_pane = focused_tile;
                            }
                        }
                    }
                }
                crate::app_protocol::AppRequest::PushPaneToSubcontext { name } => {
                    log::info!("pane_ipc: kind=push_pane_to_subcontext name={:?}", name);
                    self.push_pane_to_subcontext(name.clone());
                }
                _ => {
                    log::warn!("pane_ipc: unsupported command kind, dropping");
                }
            }
        }
    }

    pub(super) fn tick_scheduler(&mut self) {
        // Load routines from every context that has a root set
        let roots: Vec<std::path::PathBuf> = self
            .router
            .iter()
            .filter_map(|ctx| ctx.root.clone())
            .collect();
        for root in &roots {
            self.scheduler.load_from_root(root);
        }

        let now = chrono::Local::now();
        let due = self.scheduler.due_routines(now);
        for idx in due {
            let (name, command, context_name, ephemeral) = {
                let entry = &self.scheduler.entries[idx];
                (
                    entry.routine.name.clone(),
                    entry.routine.command.clone(),
                    entry.routine.context.clone(),
                    entry.routine.ephemeral,
                )
            };
            self.scheduler.mark_fired(idx, now);
            self.fire_routine(&name, &command, &context_name, ephemeral);
        }
    }

    pub(super) fn fire_routine(
        &mut self,
        name: &str,
        command: &str,
        context_name: &str,
        ephemeral: bool,
    ) {
        log::info!(
            "scheduler: routine '{}' fired context='{}' ephemeral={}",
            name,
            context_name,
            ephemeral
        );

        // Find target context index
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
                    return;
                }
            }
        };

        let original_ctx_idx = self.router.active_idx();
        let original_active_win = self.active_window;

        // Temporarily switch to target context's window if different from current
        if target_ctx_idx != original_ctx_idx {
            self.router.set_active(target_ctx_idx);
            let target_ctx_id = self.router.get(target_ctx_idx).context_id;
            if let Some(win_idx) = self
                .windows
                .iter()
                .position(|w| w.context_id == target_ctx_id)
            {
                self.active_window = win_idx;
            }
        }

        let cwd = self.router.get(target_ctx_idx).root.clone();
        self.split_focused(false, Some(command), ephemeral, false, cwd);

        // Restore original context
        if target_ctx_idx != original_ctx_idx {
            self.router.set_active(original_ctx_idx);
            self.active_window = original_active_win;
        }
    }

    pub(super) fn drain_spawn_queue(&mut self) {
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
            let path = val["path"].as_str().map(|s| s.to_string());
            if type_id.is_empty() && path.is_none() {
                log::warn!("spawn-queue: entry missing type_id and path, skipping");
                continue;
            }
            let layout = val["layout"].as_str().map(|s| s.to_string());
            let ephemeral = val["ephemeral"].as_bool().unwrap_or(false);
            let args: Vec<String> = val["args"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            let cwd_override: Option<std::path::PathBuf> =
                val["cwd"].as_str().map(std::path::PathBuf::from);
            let no_focus = val["no_focus"].as_bool().unwrap_or(false);
            let ws_root_override = val["workspace_root"].as_str().map(std::path::PathBuf::from);
            log::info!("spawn-queue: launching '{type_id}' path={path:?} layout={layout:?} ephemeral={ephemeral} no_focus={no_focus} cwd={cwd_override:?} workspace_root={ws_root_override:?}");
            let active = self.active_window;
            if type_id == "terminal" {
                let layout_str = layout.as_deref().unwrap_or("split_h");
                let vertical = matches!(layout_str, "split_h" | "split_right" | "split_left");
                let new_pane_first = matches!(layout_str, "split_above" | "split_left");
                let initial_cmd = super::cmd_from_args(&args);
                if no_focus {
                    // Use explicit targeting so we never need to restore focused_pane.
                    if let Some(tile) = self.windows[active]
                        .focused_pane
                        .or(self.windows[active].tree.root)
                    {
                        log::info!(
                            "spawn-queue: no_focus=true, spawning terminal at current focused pane"
                        );
                        let _ = self.spawn_terminal_pane_at(
                            active,
                            tile,
                            vertical,
                            new_pane_first,
                            initial_cmd.as_deref(),
                            ephemeral,
                            cwd_override,
                            true,
                        );
                    } else {
                        // Truly empty window — split_focused handles initialization
                        self.split_focused(
                            vertical,
                            initial_cmd.as_deref(),
                            ephemeral,
                            new_pane_first,
                            cwd_override,
                        );
                    }
                } else {
                    self.split_focused(
                        vertical,
                        initial_cmd.as_deref(),
                        ephemeral,
                        new_pane_first,
                        cwd_override,
                    );
                }
            } else if let Some(ref path_str) = path {
                let original_focused = self.windows[active].focused_pane;
                if let Err(e) =
                    self.launch_app_by_path_with_layout(path_str, layout, ws_root_override)
                {
                    log::warn!("spawn-queue: launch_app_by_path_with_layout failed for path={path_str:?}: {e}");
                }
                if no_focus {
                    log::info!("spawn-queue: no_focus=true, retaining focus on pane_id={original_focused:?}");
                    self.active_window = active;
                    self.restore_window_focused_pane(active, original_focused);
                }
            } else {
                let original_focused = self.windows[active].focused_pane;
                let _ = self.launch_app_by_id_with_layout(&type_id, layout, &args, cwd_override);
                if no_focus {
                    log::info!("spawn-queue: no_focus=true, retaining focus on pane_id={original_focused:?}");
                    self.active_window = active;
                    self.restore_window_focused_pane(active, original_focused);
                }
            }
        }
    }

    pub(super) fn drain_pty_events(&mut self) {
        let mut panes_to_close: Vec<u64> = Vec::new();

        while let Ok((id, event)) = self.pty_event_rx.try_recv() {
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
                        let osc_enabled = self
                            .config
                            .beta
                            .as_ref()
                            .and_then(|b| b.osc_pane_title)
                            .unwrap_or(false);
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
        log::warn!("pane_ipc: spawn_pane: could not apply name to pane_id={pane_id} (not found or not a terminal)");
    }
}
