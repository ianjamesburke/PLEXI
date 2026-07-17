//! Native execution seam for Assistant-owned host tools.

use crate::app::permissions::Capability;
use crate::plexi_ai::tool_dispatch::ToolCallResult;

use super::PlexiApp;

impl PlexiApp {
    pub(super) fn handle_assistant_host_tool(
        &mut self,
        name: &str,
        input_json: &str,
        origin_pane_id: u64,
        origin_context_id: u64,
    ) -> ToolCallResult {
        let parsed: serde_json::Value = match serde_json::from_str(input_json) {
            Ok(value) => value,
            Err(error) => return failed(format!("invalid_input: {error}")),
        };
        log::info!("assistant_host_tool: executing '{name}' in process");
        match name {
            "host.panes.list" => succeeded(self.assistant_pane_list()),
            "host.panes.state" => {
                let Some(id) = parsed.get("pane_id").and_then(serde_json::Value::as_u64) else {
                    return failed("invalid_input: pane_id is required".to_string());
                };
                match self.assistant_pane_state(id) {
                    Some(value) => succeeded(value),
                    None => failed(format!("pane_not_found: {id}")),
                }
            }
            "host.panes.open" => {
                let Some(type_id) = parsed.get("type_id").and_then(serde_json::Value::as_str)
                else {
                    return failed("invalid_input: type_id is required".to_string());
                };
                let layout = parsed
                    .get("layout")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string);
                let cwd = parsed
                    .get("cwd")
                    .and_then(serde_json::Value::as_str)
                    .map(std::path::PathBuf::from);
                let args = parsed
                    .get("args")
                    .and_then(serde_json::Value::as_array)
                    .map(|args| {
                        args.iter()
                            .filter_map(serde_json::Value::as_str)
                            .map(str::to_string)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let target_pane_id = parsed.get("pane_id").and_then(serde_json::Value::as_u64);
                match self.assistant_spawn_pane(
                    origin_pane_id,
                    origin_context_id,
                    type_id,
                    layout,
                    args,
                    cwd,
                    target_pane_id,
                ) {
                    Ok(pane_id) => succeeded(
                        serde_json::json!({"ok": true, "pane_id": pane_id, "type_id": type_id}),
                    ),
                    Err(error) => failed(format!("open_pane_failed: {error}")),
                }
            }
            "host.panes.focus" => {
                let Some(id) = parsed.get("pane_id").and_then(serde_json::Value::as_u64) else {
                    return failed("invalid_input: pane_id is required".to_string());
                };
                if self.pane_navigate(id) {
                    succeeded(serde_json::json!({"ok": true, "pane_id": id}))
                } else {
                    failed(format!("pane_not_found: {id}"))
                }
            }
            "host.panes.close" => {
                let Some(id) = parsed.get("pane_id").and_then(serde_json::Value::as_u64) else {
                    return failed("invalid_input: pane_id is required".to_string());
                };
                let existed = self
                    .windows
                    .iter()
                    .any(|window| window.panes.contains_key(&id));
                if !existed {
                    return failed(format!("pane_not_found: {id}"));
                }
                self.close_pane_by_id(id);
                succeeded(serde_json::json!({"ok": true, "pane_id": id}))
            }
            "host.apps.open" => {
                let Some(app) = parsed.get("app").and_then(serde_json::Value::as_str) else {
                    return failed("invalid_input: app is required".to_string());
                };
                let layout = parsed
                    .get("layout")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string);
                let args = parsed
                    .get("args")
                    .and_then(serde_json::Value::as_array)
                    .map(|args| {
                        args.iter()
                            .filter_map(serde_json::Value::as_str)
                            .map(str::to_string)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let target_pane_id = parsed.get("pane_id").and_then(serde_json::Value::as_u64);
                match self.assistant_spawn_pane(
                    origin_pane_id,
                    origin_context_id,
                    app,
                    layout,
                    args,
                    None,
                    target_pane_id,
                ) {
                    Ok(pane_id) => {
                        succeeded(serde_json::json!({"ok": true, "pane_id": pane_id, "app": app}))
                    }
                    Err(error) => failed(format!("open_app_failed: {error}")),
                }
            }
            "host.terminals.open" => {
                let layout = parsed
                    .get("layout")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string);
                let cwd = parsed
                    .get("cwd")
                    .and_then(serde_json::Value::as_str)
                    .map(std::path::PathBuf::from);
                match self.assistant_spawn_pane(
                    origin_pane_id,
                    origin_context_id,
                    "terminal",
                    layout,
                    Vec::new(),
                    cwd,
                    None,
                ) {
                    Ok(pane_id) => {
                        if let Err(error) =
                            self.assistant_bind_terminal(origin_pane_id, origin_context_id, pane_id)
                        {
                            return failed(format!("open_terminal_failed: {error}"));
                        }
                        succeeded(
                            serde_json::json!({"ok": true, "pane_id": pane_id, "type": "terminal"}),
                        )
                    }
                    Err(error) => failed(format!("open_terminal_failed: {error}")),
                }
            }
            "host.terminals.run" => {
                let Some(terminal_pane_id) = parsed
                    .get("terminal_pane_id")
                    .and_then(serde_json::Value::as_u64)
                else {
                    return failed("invalid_input: terminal_pane_id is required".to_string());
                };
                let Some(command) = parsed.get("command").and_then(serde_json::Value::as_str)
                else {
                    return failed("invalid_input: command is required".to_string());
                };
                if command.trim().is_empty() {
                    return failed("invalid_input: command must be non-empty".to_string());
                }
                if parsed.get("echo").and_then(serde_json::Value::as_bool) != Some(true) {
                    return failed(
                        "invalid_input: echo must be true so the human-observed terminal receives Enter"
                            .to_string(),
                    );
                }
                match self.assistant_run_terminal(
                    origin_pane_id,
                    origin_context_id,
                    terminal_pane_id,
                    command,
                ) {
                    Ok(()) => succeeded(serde_json::json!({
                        "ok": true,
                        "terminal_pane_id": terminal_pane_id,
                        "echo": true,
                    })),
                    Err(error) => failed(format!("run_terminal_failed: {error}")),
                }
            }
            "host.files.read" => {
                let Some(path) = parsed.get("path").and_then(serde_json::Value::as_str) else {
                    return failed("invalid_input: path is required".to_string());
                };
                let roots = self.assistant_file_roots(origin_context_id);
                match read_scoped_file(&roots, path) {
                    Ok(content) => succeeded(serde_json::json!({"path": path, "content": content})),
                    Err(error) => failed(error),
                }
            }
            "host.files.write" => {
                let Some(path) = parsed.get("path").and_then(serde_json::Value::as_str) else {
                    return failed("invalid_input: path is required".to_string());
                };
                let Some(content) = parsed.get("content").and_then(serde_json::Value::as_str)
                else {
                    return failed("invalid_input: content is required".to_string());
                };
                let roots = self.assistant_file_roots(origin_context_id);
                match write_scoped_file(&roots, path, content) {
                    Ok(()) => succeeded(serde_json::json!({
                        "ok": true, "path": path, "bytes": content.len(),
                    })),
                    Err(error) => failed(error),
                }
            }
            "host.files.edit" => {
                let Some(path) = parsed.get("path").and_then(serde_json::Value::as_str) else {
                    return failed("invalid_input: path is required".to_string());
                };
                let Some(old_string) = parsed.get("old_string").and_then(serde_json::Value::as_str)
                else {
                    return failed("invalid_input: old_string is required".to_string());
                };
                let Some(new_string) = parsed.get("new_string").and_then(serde_json::Value::as_str)
                else {
                    return failed("invalid_input: new_string is required".to_string());
                };
                let roots = self.assistant_file_roots(origin_context_id);
                match edit_scoped_file(&roots, path, old_string, new_string) {
                    Ok(()) => succeeded(serde_json::json!({"ok": true, "path": path})),
                    Err(error) => failed(error),
                }
            }
            "host.terminals.read" => {
                let Some(terminal_pane_id) = parsed
                    .get("terminal_pane_id")
                    .and_then(serde_json::Value::as_u64)
                else {
                    return failed("invalid_input: terminal_pane_id is required".to_string());
                };
                let lines = parsed
                    .get("lines")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(40) as usize;
                match self.assistant_read_terminal(terminal_pane_id, lines) {
                    Ok(captured) => succeeded(serde_json::json!({
                        "terminal_pane_id": terminal_pane_id,
                        "lines": captured,
                    })),
                    Err(error) => failed(error),
                }
            }
            _ => failed(format!("host_tool_unknown: {name}")),
        }
    }

    /// Bind the Assistant's existing pane to the terminal it just opened.
    /// `RunInLinkedTerminal` is intentionally link-scoped: the Assistant is a
    /// built-in app with `terminal.bindings`, while ordinary apps still need
    /// their manifest capability and their own linked terminal.
    fn assistant_bind_terminal(
        &mut self,
        origin_pane_id: u64,
        origin_context_id: u64,
        terminal_pane_id: u64,
    ) -> Result<(), String> {
        let window = self
            .windows
            .iter_mut()
            .find(|window| window.context_id == origin_context_id)
            .ok_or_else(|| format!("origin context {origin_context_id} closed"))?;
        if !window.panes.contains_key(&terminal_pane_id) {
            return Err(format!("terminal pane {terminal_pane_id} was not created"));
        }
        let Some(app) = window
            .panes
            .get_mut(&origin_pane_id)
            .and_then(crate::host::pane::Pane::as_app_mut)
        else {
            return Err(format!("origin pane {origin_pane_id} is not an app pane"));
        };
        if !app.permissions.is_builtin
            && !app
                .permissions
                .capabilities
                .contains(&Capability::TerminalBindings)
        {
            return Err("origin app lacks terminal.bindings capability".to_string());
        }
        app.linked_pane_id = Some(terminal_pane_id);
        Ok(())
    }

    fn assistant_run_terminal(
        &mut self,
        origin_pane_id: u64,
        origin_context_id: u64,
        terminal_pane_id: u64,
        command: &str,
    ) -> Result<(), String> {
        let active_context = self.windows[self.active_window].context_id;
        if active_context != origin_context_id {
            return Err(format!(
                "terminal context {origin_context_id} is not active (active context {active_context})"
            ));
        }
        let window = self
            .windows
            .iter()
            .find(|window| window.context_id == origin_context_id)
            .ok_or_else(|| format!("origin context {origin_context_id} closed"))?;
        let Some(app) = window
            .panes
            .get(&origin_pane_id)
            .and_then(crate::host::pane::Pane::as_app)
        else {
            return Err(format!("origin pane {origin_pane_id} is not an app pane"));
        };
        if !app.permissions.is_builtin
            && !app
                .permissions
                .capabilities
                .contains(&Capability::TerminalBindings)
        {
            return Err("origin app lacks terminal.bindings capability".to_string());
        }
        if app.linked_pane_id != Some(terminal_pane_id) {
            return Err(format!(
                "terminal pane {terminal_pane_id} is not the Assistant's linked terminal"
            ));
        }
        if !window.panes.contains_key(&terminal_pane_id) {
            return Err(format!("terminal pane {terminal_pane_id} not found"));
        }
        log::info!(
            "assistant_host_tool: terminal.bindings inject origin={origin_pane_id} terminal={terminal_pane_id} command={command:?} echo=true"
        );
        // Reuse the production DrawCommand execution path; its link check is
        // the terminal.bindings capability boundary for app-originated input.
        self.dispatch_run_in_linked_terminal(
            origin_pane_id,
            terminal_pane_id,
            command.to_string(),
            true,
        );
        Ok(())
    }

    /// Spawn `type_id` into a new pane, or — when `target_pane_id` is set —
    /// into the vicinity of that existing pane, which must be an idle
    /// terminal (an "empty" pane). Targeting an occupied pane (already
    /// running an app, or a portal) returns a clear `pane_occupied` error
    /// instead of the opaque `open_..._failed` message a same-pane split
    /// collision used to produce (stint 0374).
    ///
    /// The target's own pane_id is not reused for the new content — the
    /// new app pane keeps its own freshly allocated id like any other
    /// spawn, and the target is torn down (with full hot-reload/notification
    /// cleanup via `close_pane_by_id`) only after the new pane exists. This
    /// avoids reassigning a live pane's id, which would orphan any
    /// subscriptions, watchers, or WASM runtime handles already registered
    /// under the id the new pane launched with.
    fn assistant_spawn_pane(
        &mut self,
        origin_pane_id: u64,
        origin_context_id: u64,
        type_id: &str,
        layout: Option<String>,
        args: Vec<String>,
        cwd: Option<std::path::PathBuf>,
        target_pane_id: Option<u64>,
    ) -> Result<u64, String> {
        if !self.windows.iter().any(|window| {
            window.context_id == origin_context_id && window.panes.contains_key(&origin_pane_id)
        }) {
            return Err(format!(
                "origin pane {origin_pane_id} is no longer in context {origin_context_id}"
            ));
        }
        let from_pane_id = if let Some(target_id) = target_pane_id {
            let Some((target_win, _)) = self.find_pane_in_any_window(target_id) else {
                return Err(format!("pane_not_found: {target_id}"));
            };
            let is_empty_terminal = matches!(
                self.windows[target_win].panes.get(&target_id),
                Some(crate::host::pane::Pane::Terminal(_))
            );
            if !is_empty_terminal {
                return Err(format!(
                    "pane_occupied: pane {target_id} is not an empty terminal pane; only idle terminal panes can be targeted"
                ));
            }
            target_id
        } else {
            origin_pane_id
        };
        let before = self
            .windows
            .iter()
            .flat_map(|window| window.panes.keys().copied())
            .collect::<std::collections::HashSet<_>>();
        let focused_before = self
            .windows
            .iter()
            .find(|window| window.context_id == origin_context_id)
            .and_then(|window| {
                window
                    .focused_pane
                    .and_then(|tile| window.tree.tiles.get(tile))
            })
            .and_then(|tile| match tile {
                egui_tiles::Tile::Pane(id) => Some(*id),
                _ => None,
            });
        self.handle_pane_ipc_request(crate::app_protocol::AppRequest::SpawnPane {
            type_id: type_id.to_string(),
            layout,
            args,
            from_pane_id: Some(from_pane_id),
            request_id: None,
            response_file: None,
            ephemeral: false,
            cwd: cwd.map(|path| path.to_string_lossy().into_owned()),
            no_focus: false,
            path: None,
            workspace_root: None,
            target_context: None,
            name: None,
        });
        let result = if let Some(created) = self
            .windows
            .iter()
            .flat_map(|window| window.panes.keys().copied())
            .find(|pane_id| !before.contains(pane_id))
        {
            Ok(created)
        } else {
            let window = self
                .windows
                .iter()
                .find(|window| window.context_id == origin_context_id)
                .ok_or_else(|| format!("origin context {origin_context_id} closed"))?;
            let focused = window
                .focused_pane
                .and_then(|tile| window.tree.tiles.get(tile))
                .and_then(|tile| match tile {
                    egui_tiles::Tile::Pane(id) => Some(*id),
                    _ => None,
                });
            focused
                .filter(|pane_id| Some(*pane_id) != focused_before)
                .ok_or_else(|| {
                    format!("open request for '{type_id}' did not create or focus a pane")
                })
        };
        // Only tear down the retargeted pane once the new content has
        // actually landed — a failed spawn must leave the target alone
        // rather than silently deleting it.
        if let (Ok(created), Some(target_id)) = (&result, target_pane_id) {
            if *created != target_id {
                log::info!(
                    "assistant_spawn_pane: retargeted pane {target_id} closed after '{type_id}' opened in pane {created}"
                );
                self.close_pane_by_id(target_id);
            }
        }
        result
    }

    fn assistant_pane_list(&self) -> serde_json::Value {
        let active = self.active_window;
        let entries = self
            .windows
            .iter()
            .enumerate()
            .flat_map(|(window_index, window)| {
                let focused = window
                    .focused_pane
                    .and_then(|tile| window.tree.tiles.get(tile))
                    .and_then(|tile| match tile {
                        egui_tiles::Tile::Pane(id) => Some(*id),
                        _ => None,
                    });
                window
                    .panes
                    .iter()
                    .filter(move |(id, _)| window.tree.tiles.find_pane(id).is_some())
                    .map(move |(id, pane)| {
                        let (kind, title) = match pane {
                            crate::host::pane::Pane::Terminal(term) => (
                                "terminal",
                                term.name.clone().unwrap_or_else(|| "terminal".to_string()),
                            ),
                            crate::host::pane::Pane::App(app) => ("app", app.name.clone()),
                            crate::host::pane::Pane::Portal(portal) => {
                                ("portal", format!("portal:{}", portal.target_context_id))
                            }
                        };
                        serde_json::json!({
                            "id": id, "type": kind, "title": title,
                            "focused": window_index == active && focused == Some(*id),
                            "context_id": window.context_id, "window_id": window.window_id,
                        })
                    })
            })
            .collect::<Vec<_>>();
        serde_json::Value::Array(entries)
    }

    /// Directories the Assistant's file tools may touch: the global apps dir
    /// plus the origin context's workspace apps dir. App authoring only —
    /// never the whole filesystem.
    fn assistant_file_roots(&self, origin_context_id: u64) -> Vec<std::path::PathBuf> {
        let mut roots = vec![crate::app::registry::apps_dir()];
        if let Some(root) = self.context_root_for(origin_context_id) {
            roots.push(crate::app::registry::workspace_apps_dir(&root));
        }
        roots
    }

    /// Read the last `lines` non-empty screen lines of a terminal pane, the
    /// same capture path `plexi pane capture --lines` uses.
    fn assistant_read_terminal(
        &self,
        terminal_pane_id: u64,
        lines: usize,
    ) -> Result<Vec<String>, String> {
        let pane = self
            .windows
            .iter()
            .find_map(|window| window.panes.get(&terminal_pane_id))
            .ok_or_else(|| format!("pane_not_found: {terminal_pane_id}"))?;
        let term = pane
            .as_terminal()
            .ok_or_else(|| format!("not_a_terminal: pane {terminal_pane_id}"))?;
        let (mut captured, _cursor) = term.backend.capture_lines_with_cursor(lines);
        let trimmed = captured
            .iter()
            .rposition(|line| !line.trim().is_empty())
            .map(|pos| pos + 1)
            .unwrap_or(0);
        captured.truncate(trimmed);
        Ok(captured)
    }

    fn assistant_pane_state(&self, pane_id: u64) -> Option<serde_json::Value> {
        let pane = self
            .windows
            .iter()
            .find_map(|window| window.panes.get(&pane_id))?;
        Some(if let Some(app) = pane.as_app() {
            serde_json::json!({
                "pane_id": pane_id, "type": "app", "title": app.name,
                "manifest_id": app.manifest_id, "runtime": app.runtime.runtime_kind(),
                "semantic": app.semantic_state(),
            })
        } else if let Some(term) = pane.as_terminal() {
            serde_json::json!({
                "pane_id": pane_id, "type": "terminal",
                "title": term.name.clone().unwrap_or_else(|| "terminal".to_string()),
            })
        } else {
            serde_json::json!({"pane_id": pane_id, "type": "portal"})
        })
    }
}

const MAX_ASSISTANT_FILE_BYTES: u64 = 262_144;

/// Resolve `raw` against the allowed roots: expand a leading `~/`, require an
/// absolute path, reject `..` components, and require the result to sit under
/// one of `roots`. Every rejection names what failed.
fn resolve_scoped_file_path(
    roots: &[std::path::PathBuf],
    raw: &str,
) -> Result<std::path::PathBuf, String> {
    let expanded = if let Some(rest) = raw.strip_prefix("~/") {
        dirs::home_dir()
            .ok_or_else(|| "path_error: could not resolve home directory".to_string())?
            .join(rest)
    } else {
        std::path::PathBuf::from(raw)
    };
    if !expanded.is_absolute() {
        return Err(format!("path_not_absolute: {raw}"));
    }
    if expanded
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(format!("path_traversal_rejected: {raw}"));
    }
    if !roots.iter().any(|root| expanded.starts_with(root)) {
        return Err(format!(
            "path_out_of_scope: {raw} is not under an apps directory ({})",
            roots
                .iter()
                .map(|r| r.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    Ok(expanded)
}

fn read_scoped_file(roots: &[std::path::PathBuf], raw: &str) -> Result<String, String> {
    let path = resolve_scoped_file_path(roots, raw)?;
    let size = std::fs::metadata(&path)
        .map_err(|error| format!("read_failed: {}: {error}", path.display()))?
        .len();
    if size > MAX_ASSISTANT_FILE_BYTES {
        return Err(format!(
            "file_too_large: {} is {size} bytes (max {MAX_ASSISTANT_FILE_BYTES})",
            path.display()
        ));
    }
    std::fs::read_to_string(&path)
        .map_err(|error| format!("read_failed: {}: {error}", path.display()))
}

fn write_scoped_file(
    roots: &[std::path::PathBuf],
    raw: &str,
    content: &str,
) -> Result<(), String> {
    let path = resolve_scoped_file_path(roots, raw)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("write_failed: {}: {error}", parent.display()))?;
    }
    std::fs::write(&path, content)
        .map_err(|error| format!("write_failed: {}: {error}", path.display()))
}

fn edit_scoped_file(
    roots: &[std::path::PathBuf],
    raw: &str,
    old_string: &str,
    new_string: &str,
) -> Result<(), String> {
    if old_string.is_empty() {
        return Err("invalid_input: old_string must be non-empty".to_string());
    }
    let path = resolve_scoped_file_path(roots, raw)?;
    let content = read_scoped_file(roots, raw)?;
    match content.matches(old_string).count() {
        0 => Err(format!(
            "edit_no_match: old_string not found in {}",
            path.display()
        )),
        1 => std::fs::write(&path, content.replacen(old_string, new_string, 1))
            .map_err(|error| format!("write_failed: {}: {error}", path.display())),
        n => Err(format!(
            "edit_ambiguous: old_string matches {n} times in {}; provide a longer unique snippet",
            path.display()
        )),
    }
}

fn succeeded(value: serde_json::Value) -> ToolCallResult {
    ToolCallResult {
        output_json: Some(value.to_string()),
        error: None,
    }
}

fn failed(error: String) -> ToolCallResult {
    ToolCallResult {
        output_json: None,
        error: Some(error),
    }
}

#[cfg(test)]
mod tests {
    use crate::testing::HostHarness;

    #[test]
    fn scoped_file_helpers_write_read_edit_within_roots_and_reject_escapes() {
        let root = tempfile::tempdir().unwrap();
        let apps = root.path().join("apps");
        std::fs::create_dir_all(&apps).unwrap();
        let roots = vec![apps.clone()];
        let file = apps.join("demo/main.py").display().to_string();

        super::write_scoped_file(&roots, &file, "count = 0\nprint(count)\n").unwrap();
        assert_eq!(
            super::read_scoped_file(&roots, &file).unwrap(),
            "count = 0\nprint(count)\n"
        );

        super::edit_scoped_file(&roots, &file, "count = 0", "count = 5").unwrap();
        assert_eq!(
            super::read_scoped_file(&roots, &file).unwrap(),
            "count = 5\nprint(count)\n"
        );

        let no_match = super::edit_scoped_file(&roots, &file, "absent", "x").unwrap_err();
        assert!(no_match.starts_with("edit_no_match"), "{no_match}");

        super::write_scoped_file(&roots, &file, "a\na\n").unwrap();
        let ambiguous = super::edit_scoped_file(&roots, &file, "a", "b").unwrap_err();
        assert!(ambiguous.starts_with("edit_ambiguous"), "{ambiguous}");

        let outside = super::write_scoped_file(&roots, "/tmp/plexi-escape.py", "x").unwrap_err();
        assert!(outside.starts_with("path_out_of_scope"), "{outside}");

        let traversal = apps.join("demo/../../etc/passwd").display().to_string();
        let escaped = super::read_scoped_file(&roots, &traversal).unwrap_err();
        assert!(escaped.starts_with("path_traversal_rejected"), "{escaped}");

        let relative = super::read_scoped_file(&roots, "apps/demo/main.py").unwrap_err();
        assert!(relative.starts_with("path_not_absolute"), "{relative}");
    }

    #[test]
    fn files_tools_dispatch_and_report_scope_errors() {
        let mut harness = HostHarness::new();
        let origin = harness.add_test_pane();
        let context = harness.app.windows[0].context_id;

        let out_of_scope = harness.app.handle_assistant_host_tool(
            "host.files.read",
            r#"{"path": "/etc/passwd"}"#,
            origin,
            context,
        );
        assert!(out_of_scope
            .error
            .as_deref()
            .unwrap()
            .starts_with("path_out_of_scope"));

        let missing_arg =
            harness
                .app
                .handle_assistant_host_tool("host.files.write", "{}", origin, context);
        assert!(missing_arg
            .error
            .as_deref()
            .unwrap()
            .starts_with("invalid_input"));

        let missing_edit_arg = harness.app.handle_assistant_host_tool(
            "host.files.edit",
            r#"{"path": "/tmp/x"}"#,
            origin,
            context,
        );
        assert!(missing_edit_arg
            .error
            .as_deref()
            .unwrap()
            .starts_with("invalid_input"));
    }

    #[test]
    fn terminals_read_returns_screen_lines_and_fails_loudly_on_non_terminals() {
        let mut harness = HostHarness::new();
        let origin = harness.add_test_pane();
        let context = harness.app.windows[0].context_id;

        let terminal = harness.app.handle_assistant_host_tool(
            "host.terminals.open",
            &serde_json::json!({"layout": "split_h", "cwd": std::env::temp_dir()}).to_string(),
            origin,
            context,
        );
        let terminal_id = serde_json::from_str::<serde_json::Value>(
            terminal.output_json.as_deref().expect("terminal output"),
        )
        .unwrap()["pane_id"]
            .as_u64()
            .expect("terminal pane id");

        let read = harness.app.handle_assistant_host_tool(
            "host.terminals.read",
            &serde_json::json!({"terminal_pane_id": terminal_id, "lines": 5}).to_string(),
            origin,
            context,
        );
        assert!(read.error.is_none(), "{:?}", read.error);
        let value =
            serde_json::from_str::<serde_json::Value>(read.output_json.as_deref().unwrap())
                .unwrap();
        assert!(value["lines"].is_array());

        let not_terminal = harness.app.handle_assistant_host_tool(
            "host.terminals.read",
            &serde_json::json!({"terminal_pane_id": origin}).to_string(),
            origin,
            context,
        );
        assert!(not_terminal
            .error
            .as_deref()
            .unwrap()
            .starts_with("not_a_terminal"));

        let missing = harness.app.handle_assistant_host_tool(
            "host.terminals.read",
            r#"{"terminal_pane_id": 999999}"#,
            origin,
            context,
        );
        assert!(missing
            .error
            .as_deref()
            .unwrap()
            .starts_with("pane_not_found"));

        let no_arg =
            harness
                .app
                .handle_assistant_host_tool("host.terminals.read", "{}", origin, context);
        assert!(no_arg.error.as_deref().unwrap().starts_with("invalid_input"));
    }

    #[test]
    fn native_host_tools_list_read_focus_and_close_real_harness_panes() {
        let mut harness = HostHarness::new();
        let pane = harness.add_test_pane();
        let context = harness.app.windows[0].context_id;

        let listed = harness
            .app
            .handle_assistant_host_tool("host.panes.list", "{}", pane, context);
        assert!(listed.error.is_none());
        assert!(listed
            .output_json
            .unwrap()
            .contains(&format!("\"id\":{pane}")));

        let state = harness.app.handle_assistant_host_tool(
            "host.panes.state",
            &serde_json::json!({"pane_id": pane}).to_string(),
            pane,
            context,
        );
        assert!(state
            .output_json
            .unwrap()
            .contains("\"runtime\":\"builtin\""));

        let focused = harness.app.handle_assistant_host_tool(
            "host.panes.focus",
            &serde_json::json!({"pane_id": pane}).to_string(),
            pane,
            context,
        );
        assert!(focused.error.is_none());

        let closed = harness.app.handle_assistant_host_tool(
            "host.panes.close",
            &serde_json::json!({"pane_id": pane}).to_string(),
            pane,
            context,
        );
        assert!(closed.error.is_none());
        assert!(!harness
            .app
            .windows
            .iter()
            .any(|window| window.panes.contains_key(&pane)));
    }

    #[test]
    fn native_open_tools_create_generic_app_and_terminal_panes() {
        let mut harness = HostHarness::new();
        let origin = harness.add_test_pane();
        let context = harness.app.windows[0].context_id;
        let initial = harness.pane_count();

        let generic = harness.app.handle_assistant_host_tool(
            "host.panes.open",
            r#"{"type_id":"text-editor","layout":"split_h"}"#,
            origin,
            context,
        );
        assert!(generic.error.is_none(), "{:?}", generic.error);
        assert!(harness.pane_count() > initial);
        let after_generic = harness.pane_count();

        let app = harness.app.handle_assistant_host_tool(
            "host.apps.open",
            r#"{"app":"text-editor","layout":"split_h","args":[]}"#,
            origin,
            context,
        );
        assert!(app.error.is_none(), "{:?}", app.error);
        assert!(harness.pane_count() > after_generic);
        let after_app = harness.pane_count();

        let terminal = harness.app.handle_assistant_host_tool(
            "host.terminals.open",
            &serde_json::json!({"layout": "split_h", "cwd": std::env::temp_dir()}).to_string(),
            origin,
            context,
        );
        assert!(terminal.error.is_none(), "{:?}", terminal.error);
        assert!(harness.pane_count() > after_app);
        let terminal_id = serde_json::from_str::<serde_json::Value>(
            terminal.output_json.as_deref().expect("terminal output"),
        )
        .unwrap()["pane_id"]
            .as_u64()
            .expect("terminal pane id");

        let ran = harness.app.handle_assistant_host_tool(
            "host.terminals.run",
            &serde_json::json!({
                "terminal_pane_id": terminal_id,
                "command": "echo assistant-terminal-tool",
                "echo": true,
            })
            .to_string(),
            origin,
            context,
        );
        assert!(ran.error.is_none(), "{:?}", ran.error);

        let hidden_echo = harness.app.handle_assistant_host_tool(
            "host.terminals.run",
            &serde_json::json!({
                "terminal_pane_id": terminal_id,
                "command": "echo must-be-observed",
                "echo": false,
            })
            .to_string(),
            origin,
            context,
        );
        assert!(
            hidden_echo
                .error
                .as_deref()
                .is_some_and(|error| error.contains("echo must be true")),
            "{:?}",
            hidden_echo.error
        );
    }

    /// Stint 0374: `host.apps.open`/`host.panes.open` can target an existing
    /// idle terminal pane instead of always spawning a fresh one — the
    /// dogfooding bug where targeting pane 105 for calculator returned an
    /// opaque `open_pane_failed` and the app landed in a brand-new pane.
    #[test]
    fn native_open_tools_target_existing_empty_terminal_pane() {
        let mut harness = HostHarness::new();
        let origin = harness.add_test_pane();
        let context = harness.app.windows[0].context_id;

        let terminal = harness.app.handle_assistant_host_tool(
            "host.terminals.open",
            &serde_json::json!({"layout": "split_h", "cwd": std::env::temp_dir()}).to_string(),
            origin,
            context,
        );
        assert!(terminal.error.is_none(), "{:?}", terminal.error);
        let target_id = serde_json::from_str::<serde_json::Value>(
            terminal.output_json.as_deref().expect("terminal output"),
        )
        .unwrap()["pane_id"]
            .as_u64()
            .expect("terminal pane id");

        let opened = harness.app.handle_assistant_host_tool(
            "host.apps.open",
            &serde_json::json!({"app": "text-editor", "pane_id": target_id}).to_string(),
            origin,
            context,
        );
        assert!(opened.error.is_none(), "{:?}", opened.error);
        let new_id = serde_json::from_str::<serde_json::Value>(
            opened.output_json.as_deref().expect("open output"),
        )
        .unwrap()["pane_id"]
            .as_u64()
            .expect("new pane id");

        assert_ne!(
            new_id, target_id,
            "targeting reuses the target's slot, not its literal pane_id"
        );
        assert!(
            !harness
                .app
                .windows
                .iter()
                .any(|window| window.panes.contains_key(&target_id)),
            "the retargeted terminal pane should be torn down"
        );
        assert!(
            harness
                .app
                .windows
                .iter()
                .any(|window| window.panes.contains_key(&new_id)),
            "the new app pane should exist"
        );
    }

    #[test]
    fn native_open_tools_target_occupied_pane_returns_clear_error() {
        let mut harness = HostHarness::new();
        let origin = harness.add_test_pane();
        let context = harness.app.windows[0].context_id;
        let occupied = harness.add_test_pane();

        let result = harness.app.handle_assistant_host_tool(
            "host.apps.open",
            &serde_json::json!({"app": "text-editor", "pane_id": occupied}).to_string(),
            origin,
            context,
        );
        assert!(
            result
                .error
                .as_deref()
                .is_some_and(|error| error.contains("pane_occupied")),
            "{:?}",
            result.error
        );
        assert!(
            harness
                .app
                .windows
                .iter()
                .any(|window| window.panes.contains_key(&occupied)),
            "an occupied target must not be torn down on rejection"
        );
    }

    #[test]
    fn native_open_tools_target_missing_pane_returns_not_found() {
        let mut harness = HostHarness::new();
        let origin = harness.add_test_pane();
        let context = harness.app.windows[0].context_id;

        let result = harness.app.handle_assistant_host_tool(
            "host.panes.open",
            &serde_json::json!({"type_id": "text-editor", "pane_id": 999_999}).to_string(),
            origin,
            context,
        );
        assert!(
            result
                .error
                .as_deref()
                .is_some_and(|error| error.contains("pane_not_found")),
            "{:?}",
            result.error
        );
    }
}
