//! Native execution seam for Assistant-owned host tools.

use crate::app::permissions::Capability;
use crate::plexi_ai::tool_dispatch::ToolCallResult;

use super::PlexiApp;

/// Assistant host tool that fetches one URL through the host's HTTP broker.
pub(crate) const HOST_TOOL_NET_FETCH: &str = "host.net.fetch";

/// Cap on the response body handed back to the model. A fetched page is
/// untrusted text entering a capable planner's context; an unbounded body
/// would also blow the turn's token budget on one call.
const MAX_FETCH_BODY_CHARS: usize = 100_000;

/// Byte cap passed to the streaming body reader — four bytes per char covers
/// the widest UTF-8 encoding of `MAX_FETCH_BODY_CHARS`, so the char cap below
/// stays the user-visible limit while the wire read is still bounded.
const MAX_FETCH_BODY_BYTES: u64 = MAX_FETCH_BODY_CHARS as u64 * 4;

/// Methods the Assistant may issue. Anything outside this set is refused
/// rather than passed through to `ureq`.
const ALLOWED_FETCH_METHODS: [&str; 5] = ["GET", "HEAD", "POST", "PUT", "DELETE"];

impl PlexiApp {
    /// Execute one `host.net.fetch` call. Authorization is resolved here on
    /// the UI thread against the *origin pane's* real permissions — the same
    /// `net.http` capability and `allowed_hosts` allowlist that gate
    /// `DrawCommand::HttpRequest` — and only then is the request handed to a
    /// worker thread, so a hung peer cannot wedge the host. The user-facing
    /// prompt already happened: the tool is declared non-read-only, so the
    /// Assistant's gate asks before this is ever reached.
    pub(crate) fn handle_assistant_net_fetch(
        &mut self,
        input_json: &str,
        origin_pane_id: u64,
        reply: std::sync::mpsc::SyncSender<ToolCallResult>,
    ) {
        let refuse = |reply: &std::sync::mpsc::SyncSender<ToolCallResult>, error: String| {
            log::warn!("assistant_host_tool: {HOST_TOOL_NET_FETCH} rejected: {error}");
            let _ = reply.send(failed(error));
        };
        let parsed: serde_json::Value = match serde_json::from_str(input_json) {
            Ok(value) => value,
            Err(error) => return refuse(&reply, format!("invalid_input: {error}")),
        };
        let Some(url) = parsed
            .get("url")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
        else {
            return refuse(&reply, "invalid_input: url is required".to_string());
        };
        let method = parsed
            .get("method")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("GET")
            .to_ascii_uppercase();
        if !ALLOWED_FETCH_METHODS.contains(&method.as_str()) {
            return refuse(
                &reply,
                format!(
                    "invalid_input: method {method} is not allowed (expected one of {})",
                    ALLOWED_FETCH_METHODS.join(", ")
                ),
            );
        }
        let allowed_hosts = match self.assistant_net_policy(origin_pane_id) {
            Ok(hosts) => hosts,
            Err(error) => return refuse(&reply, error),
        };
        if !crate::host::services::http_host_allowed(&url, &allowed_hosts) {
            return refuse(
                &reply,
                format!("net_host_not_allowed: {url} is not an allowed http(s) host"),
            );
        }
        if let Some(reason) = fetch_destination_rejected(&url) {
            return refuse(&reply, format!("net_destination_rejected: {reason}"));
        }
        let headers: std::collections::HashMap<String, String> = parsed
            .get("headers")
            .and_then(serde_json::Value::as_object)
            .map(|values| {
                values
                    .iter()
                    .filter_map(|(key, value)| {
                        value.as_str().map(|value| (key.clone(), value.to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let body = parsed
            .get("body")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string);
        log::info!(
            "assistant_host_tool: {HOST_TOOL_NET_FETCH} {method} {url} origin_pane={origin_pane_id} allowed_hosts={}",
            if allowed_hosts.is_empty() {
                "unrestricted".to_string()
            } else {
                allowed_hosts.join(",")
            }
        );
        let worker_reply = reply.clone();
        let spawned = std::thread::Builder::new()
            .name("assistant-net-fetch".to_string())
            .spawn(move || {
                use crate::host::services::NetService;
                let response = crate::host::services::UreqNetService::new().http(
                    &method,
                    &url,
                    &headers,
                    body.as_deref(),
                    MAX_FETCH_BODY_BYTES,
                );
                let outcome = if let Some(error) = response.error {
                    log::warn!("assistant_host_tool: {HOST_TOOL_NET_FETCH} {url} failed: {error}");
                    failed(format!("fetch_failed: {error}"))
                } else {
                    let (text, char_truncated) = truncate_fetch_body(&response.body);
                    let truncated = response.truncated || char_truncated;
                    log::info!(
                        "assistant_host_tool: {HOST_TOOL_NET_FETCH} {url} status={} bytes={} truncated={truncated}",
                        response.status,
                        response.body.len()
                    );
                    succeeded(serde_json::json!({
                        "url": url,
                        "status": response.status,
                        "headers": response.response_headers,
                        "body": text,
                        "truncated": truncated,
                    }))
                };
                if worker_reply.send(outcome).is_err() {
                    log::warn!(
                        "assistant_host_tool: {HOST_TOOL_NET_FETCH} worker dropped its reply channel"
                    );
                }
            });
        // Every exit from this function answers the model — a spawn failure
        // must not leave the tool call hanging until its outer timeout.
        if let Err(error) = spawned {
            refuse(&reply, format!("fetch_failed: worker spawn: {error}"));
        }
    }

    /// Network policy for the Assistant's origin pane: the `net.http`
    /// capability and the `allowed_hosts` allowlist, read from the pane's own
    /// permissions rather than from anything the caller declared. Built-in
    /// panes carry an empty allowlist, which the shared check reads as
    /// unrestricted http(s) — the same posture a manifest app gets when it
    /// declares `net.http` without naming hosts.
    fn assistant_net_policy(&self, origin_pane_id: u64) -> Result<Vec<String>, String> {
        let Some((window_index, _)) = self.find_pane_in_any_window(origin_pane_id) else {
            return Err(format!("pane_not_found: {origin_pane_id}"));
        };
        let Some(app) = self.windows[window_index]
            .panes
            .get(&origin_pane_id)
            .and_then(crate::host::pane::Pane::as_app)
        else {
            return Err(format!("origin pane {origin_pane_id} is not an app pane"));
        };
        if !app.permissions.is_builtin
            && !app.permissions.capabilities.contains(&Capability::NetHttp)
        {
            return Err("net_capability_missing: origin app lacks net.http".to_string());
        }
        Ok(app.permissions.allowed_hosts.clone())
    }
}

/// SSRF guard for `host.net.fetch` destinations, applied after the
/// allowlist: an agent-issued fetch must never reach the local machine or
/// the local network. All IP-literal hosts are rejected outright (which
/// covers loopback, RFC 1918, and link-local), as are `localhost` names.
/// DNS-rebinding defence is explicitly out of scope (parked ruling).
fn fetch_destination_rejected(raw_url: &str) -> Option<String> {
    let url = url::Url::parse(raw_url).ok()?;
    match url.host()? {
        url::Host::Ipv4(ip) => Some(format!("IP-literal destination {ip} is not allowed")),
        url::Host::Ipv6(ip) => Some(format!("IP-literal destination {ip} is not allowed")),
        url::Host::Domain(domain) => {
            let domain = domain.trim_end_matches('.').to_ascii_lowercase();
            if domain == "localhost" || domain.ends_with(".localhost") {
                Some(format!("loopback destination {domain} is not allowed"))
            } else {
                None
            }
        }
    }
}

/// Trim a fetched body to `MAX_FETCH_BODY_CHARS`, always on a char boundary.
fn truncate_fetch_body(body: &str) -> (String, bool) {
    if body.chars().count() <= MAX_FETCH_BODY_CHARS {
        return (body.to_string(), false);
    }
    let cut: String = body.chars().take(MAX_FETCH_BODY_CHARS).collect();
    (format!("{cut}… [body truncated]"), true)
}

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
                let offset = parsed
                    .get("offset")
                    .and_then(serde_json::Value::as_u64)
                    .map(|n| n as usize)
                    .unwrap_or(1);
                let limit = parsed
                    .get("limit")
                    .and_then(serde_json::Value::as_u64)
                    .map(|n| n as usize)
                    .unwrap_or(MAX_READ_LINES);
                let roots = self.assistant_file_roots(origin_context_id);
                match read_scoped_file_slice(&roots, path, offset, limit) {
                    Ok(slice) => succeeded(serde_json::json!({
                        "path": path,
                        "content": slice.content,
                        "total_lines": slice.total_lines,
                        "offset": offset,
                        "lines_returned": slice.lines_returned,
                    })),
                    Err(error) => failed(error),
                }
            }
            "host.files.grep" => {
                let Some(pattern) = parsed.get("pattern").and_then(serde_json::Value::as_str)
                else {
                    return failed("invalid_input: pattern is required".to_string());
                };
                let path = parsed.get("path").and_then(serde_json::Value::as_str);
                let max_matches = parsed
                    .get("max_matches")
                    .and_then(serde_json::Value::as_u64)
                    .map(|n| (n as usize).clamp(1, MAX_GREP_MATCHES))
                    .unwrap_or(DEFAULT_GREP_MATCHES);
                let roots = self.assistant_file_roots(origin_context_id);
                match grep_scoped(&roots, path, pattern, max_matches) {
                    Ok(result) => succeeded(result),
                    Err(error) => failed(error),
                }
            }
            "host.files.list" => {
                let path = parsed.get("path").and_then(serde_json::Value::as_str);
                let roots = self.assistant_file_roots(origin_context_id);
                match list_scoped(&roots, path) {
                    Ok(result) => succeeded(result),
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
                    Ok(outcome) => succeeded(serde_json::json!({
                        "ok": true, "path": path, "bytes": content.len(),
                        "created": outcome.created, "diff": outcome.diff,
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
                    Ok(diff) => succeeded(serde_json::json!({
                        "ok": true, "path": path, "diff": diff,
                    })),
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
    // Arg-struct refactor is a design change tracked in stint 0661.
    #[allow(clippy::too_many_arguments)]
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
            context_name: None,
            name: None,
            agent_cmd: None,
            boot_timeout_secs: None,
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

    /// Directories the Assistant's file tools may touch. The caller context
    /// root is primary so relative paths and pathless walks behave like a
    /// coding agent opened in that workspace. The global apps directory stays
    /// available as an explicit auxiliary root for app authoring.
    fn assistant_file_roots(&self, origin_context_id: u64) -> Vec<std::path::PathBuf> {
        let global_apps = crate::app::registry::apps_dir();
        let mut roots = Vec::new();
        if let Some(root) = self.context_root_for(origin_context_id) {
            roots.push(root);
        }
        if !roots.iter().any(|root| global_apps.starts_with(root)) {
            roots.push(global_apps);
        }
        log::info!(
            "assistant_host_tool: file scope context={origin_context_id} roots={}",
            roots
                .iter()
                .map(|root| root.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
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
/// Default/maximum lines one `host.files.read` call returns.
const MAX_READ_LINES: usize = 2000;
/// Individual lines longer than this are truncated in read/grep output.
const MAX_LINE_CHARS: usize = 2000;
/// Hard cap on grep matches per call; the default is lower.
const MAX_GREP_MATCHES: usize = 200;
const DEFAULT_GREP_MATCHES: usize = 50;
/// Directory names never descended into by grep/list walks: the shared
/// scaffold/check artifact list (`package::is_generated_dev_dir_name`) plus
/// VCS and JS dependency trees that packaging never encounters.
fn is_skipped_walk_dir(name: &str) -> bool {
    crate::app::package::is_generated_dev_dir_name(name) || matches!(name, ".git" | "node_modules")
}
/// Caps on the grep/list directory walk so a runaway tree fails visibly
/// instead of hanging the tool call.
const MAX_WALK_DEPTH: usize = 8;
const MAX_WALK_FILES: usize = 5000;
const MAX_LIST_ENTRIES: usize = 500;
/// Unified-diff shaping for edit/write results rendered in the transcript.
const DIFF_CONTEXT_LINES: usize = 3;
const MAX_DIFF_CHARS: usize = 4000;

/// Resolve `raw` against the allowed roots. Relative paths use the primary
/// root, while absolute and `~/` paths may address any explicit root. The
/// deepest existing prefix is canonicalized before containment is checked so
/// a workspace symlink cannot redirect direct reads or writes outside scope.
fn resolve_scoped_file_path(
    roots: &[std::path::PathBuf],
    raw: &str,
) -> Result<std::path::PathBuf, String> {
    let Some(primary_root) = roots.first() else {
        return Err("path_scope_unavailable: this context has no file root".to_string());
    };
    let requested = if let Some(rest) = raw.strip_prefix("~/") {
        dirs::home_dir()
            .ok_or_else(|| "path_error: could not resolve home directory".to_string())?
            .join(rest)
    } else {
        std::path::PathBuf::from(raw)
    };
    if requested
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(format!("path_traversal_rejected: {raw}"));
    }
    let absolute = if requested.is_absolute() {
        requested
    } else {
        primary_root.join(requested)
    };
    let resolved = super::canvas_bindings::canonicalize_existing_prefix(&absolute);
    let allowed = roots
        .iter()
        .map(|root| super::canvas_bindings::canonicalize_existing_prefix(root))
        .any(|root| resolved.starts_with(root));
    if !allowed {
        return Err(format!(
            "path_out_of_scope: {raw} is not under a context file root ({})",
            roots
                .iter()
                .map(|r| r.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    Ok(resolved)
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

/// A line-ranged read: numbered content plus range metadata so the model can
/// page through files instead of re-reading them whole.
#[derive(Debug)]
struct ReadSlice {
    content: String,
    total_lines: usize,
    lines_returned: usize,
}

/// Read lines `offset..offset+limit` (1-based) of a scoped file, each
/// prefixed with its line number — the same contract coding agents use, so
/// follow-up edits can reference exact locations.
fn read_scoped_file_slice(
    roots: &[std::path::PathBuf],
    raw: &str,
    offset: usize,
    limit: usize,
) -> Result<ReadSlice, String> {
    if offset == 0 {
        return Err("invalid_input: offset is 1-based and must be >= 1".to_string());
    }
    if limit == 0 {
        return Err("invalid_input: limit must be >= 1".to_string());
    }
    let content = read_scoped_file(roots, raw)?;
    let lines: Vec<&str> = content.lines().collect();
    let total_lines = lines.len();
    if offset > total_lines && total_lines > 0 {
        return Err(format!(
            "invalid_input: offset {offset} is past the end of the file ({total_lines} lines)"
        ));
    }
    let limit = limit.min(MAX_READ_LINES);
    let mut out = String::new();
    let mut returned = 0usize;
    for (index, line) in lines.iter().enumerate().skip(offset - 1).take(limit) {
        out.push_str(&format!("{:>6}\t{}\n", index + 1, truncate_line(line)));
        returned += 1;
    }
    Ok(ReadSlice {
        content: out,
        total_lines,
        lines_returned: returned,
    })
}

fn truncate_line(line: &str) -> std::borrow::Cow<'_, str> {
    if line.chars().count() <= MAX_LINE_CHARS {
        return std::borrow::Cow::Borrowed(line);
    }
    let cut: String = line.chars().take(MAX_LINE_CHARS).collect();
    std::borrow::Cow::Owned(format!("{cut}… [line truncated]"))
}

/// Walk `start` depth-first in sorted order, yielding files. Skips the
/// well-known junk directories, and stops at the walk caps. Symlinks are
/// never followed (same rule as packaging): grep/list are read-only
/// auto-allowed tools, so a symlink inside an apps directory must not widen
/// their reach beyond the scoped roots. Returns `(files, hit_file_cap)`.
fn walk_scoped_files(start: &std::path::Path) -> (Vec<std::path::PathBuf>, bool) {
    let mut files = Vec::new();
    if start.is_file() {
        files.push(start.to_path_buf());
        return (files, false);
    }
    let mut stack = vec![(start.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        if depth > MAX_WALK_DEPTH {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut children: Vec<(std::path::PathBuf, std::fs::FileType)> = entries
            .flatten()
            .filter_map(|entry| entry.file_type().ok().map(|kind| (entry.path(), kind)))
            .collect();
        children.sort_by(|(a, _), (b, _)| a.cmp(b));
        let mut subdirs = Vec::new();
        for (child, kind) in children {
            // DirEntry::file_type does not follow symlinks — a link to a
            // directory or file outside the roots is skipped entirely.
            if kind.is_symlink() {
                continue;
            }
            let name = child
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if kind.is_dir() {
                if !is_skipped_walk_dir(&name) {
                    subdirs.push(child);
                }
            } else if kind.is_file() {
                if files.len() >= MAX_WALK_FILES {
                    return (files, true);
                }
                files.push(child);
            }
        }
        // Reverse so the stack pops subdirectories in sorted order.
        for subdir in subdirs.into_iter().rev() {
            stack.push((subdir, depth + 1));
        }
    }
    (files, false)
}

/// Resolve the search start points for grep/list: an explicit scoped path, or
/// the primary existing root when no path is given.
fn scoped_walk_starts(
    roots: &[std::path::PathBuf],
    raw_path: Option<&str>,
) -> Result<Vec<std::path::PathBuf>, String> {
    match raw_path {
        Some(raw) => {
            let path = resolve_scoped_file_path(roots, raw)?;
            if !path.exists() {
                return Err(format!("path_not_found: {raw}"));
            }
            Ok(vec![path])
        }
        None => Ok(roots
            .iter()
            .find(|root| root.exists())
            .cloned()
            .into_iter()
            .collect()),
    }
}

/// Regex search across scoped workspace files — the in-process replacement
/// for the `cat`/`grep` PTY round-trips that burned the tool budget.
fn grep_scoped(
    roots: &[std::path::PathBuf],
    raw_path: Option<&str>,
    pattern: &str,
    max_matches: usize,
) -> Result<serde_json::Value, String> {
    let regex = regex::Regex::new(pattern).map_err(|error| format!("invalid_pattern: {error}"))?;
    let starts = scoped_walk_starts(roots, raw_path)?;
    let mut matches = Vec::new();
    let mut files_scanned = 0usize;
    let mut truncated = false;
    'outer: for start in &starts {
        let (files, hit_cap) = walk_scoped_files(start);
        truncated |= hit_cap;
        for file in files {
            if std::fs::metadata(&file)
                .map(|meta| meta.len() > MAX_ASSISTANT_FILE_BYTES)
                .unwrap_or(true)
            {
                continue;
            }
            // Binary or non-UTF-8 files are silently skipped, like any grep.
            let Ok(content) = std::fs::read_to_string(&file) else {
                continue;
            };
            files_scanned += 1;
            for (index, line) in content.lines().enumerate() {
                if !regex.is_match(line) {
                    continue;
                }
                if matches.len() >= max_matches {
                    truncated = true;
                    break 'outer;
                }
                matches.push(serde_json::json!({
                    "file": file.display().to_string(),
                    "line": index + 1,
                    "text": truncate_line(line),
                }));
            }
        }
    }
    Ok(serde_json::json!({
        "matches": matches,
        "truncated": truncated,
        "files_scanned": files_scanned,
    }))
}

/// List scoped workspace files with sizes.
fn list_scoped(
    roots: &[std::path::PathBuf],
    raw_path: Option<&str>,
) -> Result<serde_json::Value, String> {
    let starts = scoped_walk_starts(roots, raw_path)?;
    let mut entries = Vec::new();
    let mut truncated = false;
    'outer: for start in &starts {
        let (files, hit_cap) = walk_scoped_files(start);
        truncated |= hit_cap;
        for file in files {
            if entries.len() >= MAX_LIST_ENTRIES {
                truncated = true;
                break 'outer;
            }
            let bytes = std::fs::metadata(&file).map(|meta| meta.len()).unwrap_or(0);
            entries.push(serde_json::json!({
                "path": file.display().to_string(),
                "bytes": bytes,
            }));
        }
    }
    Ok(serde_json::json!({"entries": entries, "truncated": truncated}))
}

/// Minimal single-hunk unified diff: trims the common prefix/suffix and
/// wraps the changed span in up to `DIFF_CONTEXT_LINES` of context. Exact
/// for single-span changes (every `host.files.edit`); for multi-span writes
/// the one hunk covers the full changed region.
fn unified_diff(old: &str, new: &str, path: &str) -> String {
    if old == new {
        return String::new();
    }
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    let max_common = old_lines.len().min(new_lines.len());
    let mut prefix = 0usize;
    while prefix < max_common && old_lines[prefix] == new_lines[prefix] {
        prefix += 1;
    }
    let mut suffix = 0usize;
    while suffix < max_common - prefix
        && old_lines[old_lines.len() - 1 - suffix] == new_lines[new_lines.len() - 1 - suffix]
    {
        suffix += 1;
    }
    let context_start = prefix.saturating_sub(DIFF_CONTEXT_LINES);
    let context_end_old = (old_lines.len() - suffix + DIFF_CONTEXT_LINES).min(old_lines.len());
    let context_end_new = (new_lines.len() - suffix + DIFF_CONTEXT_LINES).min(new_lines.len());
    let old_count = context_end_old - context_start;
    let new_count = context_end_new - context_start;
    let mut out = format!(
        "--- a/{path}\n+++ b/{path}\n@@ -{},{old_count} +{},{new_count} @@\n",
        context_start + 1,
        context_start + 1,
    );
    // Enforce the cap while building: a full-file rewrite would otherwise
    // materialize the whole double-sided diff only to throw most of it away.
    // The early return is also what keeps truncation on a char boundary —
    // never byte-truncate a String built from arbitrary file content.
    let push_line = |out: &mut String, marker: char, line: &str| -> bool {
        if out.len() >= MAX_DIFF_CHARS {
            out.push_str("… [diff truncated]\n");
            return false;
        }
        out.push(marker);
        out.push_str(&truncate_line(line));
        out.push('\n');
        true
    };
    let sections: [(char, &[&str]); 4] = [
        (' ', &old_lines[context_start..prefix]),
        ('-', &old_lines[prefix..old_lines.len() - suffix]),
        ('+', &new_lines[prefix..new_lines.len() - suffix]),
        (' ', &old_lines[old_lines.len() - suffix..context_end_old]),
    ];
    'sections: for (marker, lines) in sections {
        for line in lines {
            if !push_line(&mut out, marker, line) {
                break 'sections;
            }
        }
    }
    out
}

/// Outcome of a scoped write: whether the file was created, and the diff
/// against the previous content when it was overwritten.
#[derive(Debug)]
struct WriteOutcome {
    created: bool,
    diff: Option<String>,
}

fn write_scoped_file(
    roots: &[std::path::PathBuf],
    raw: &str,
    content: &str,
) -> Result<WriteOutcome, String> {
    let path = resolve_scoped_file_path(roots, raw)?;
    let previous = path
        .is_file()
        .then(|| std::fs::read_to_string(&path).ok())
        .flatten();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("write_failed: {}: {error}", parent.display()))?;
    }
    std::fs::write(&path, content)
        .map_err(|error| format!("write_failed: {}: {error}", path.display()))?;
    Ok(WriteOutcome {
        created: previous.is_none(),
        diff: previous.map(|old| unified_diff(&old, content, raw)),
    })
}

/// Replace exactly one occurrence of `old_string` and return the unified
/// diff of the change. Zero or multiple matches fail loudly — the same
/// edit-verification contract proven by Claude Code's Edit tool.
fn edit_scoped_file(
    roots: &[std::path::PathBuf],
    raw: &str,
    old_string: &str,
    new_string: &str,
) -> Result<String, String> {
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
        1 => {
            let updated = content.replacen(old_string, new_string, 1);
            std::fs::write(&path, &updated)
                .map_err(|error| format!("write_failed: {}: {error}", path.display()))?;
            Ok(unified_diff(&content, &updated, raw))
        }
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

    /// Every rejection `host.net.fetch` makes before a socket is opened —
    /// each one must answer on the reply channel rather than hang the worker.
    #[test]
    fn net_fetch_refuses_bad_input_schemes_and_methods_without_touching_the_network() {
        let mut harness = HostHarness::new();
        let origin = harness.add_test_pane();

        let refusal = |harness: &mut HostHarness, input: &str| -> String {
            let (tx, rx) = std::sync::mpsc::sync_channel(1);
            harness.app.handle_assistant_net_fetch(input, origin, tx);
            let result = rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("net fetch must always answer its reply channel");
            result.error.expect("expected a refusal")
        };

        assert!(refusal(&mut harness, "not json").starts_with("invalid_input"));
        assert!(refusal(&mut harness, "{}").starts_with("invalid_input"));
        // A non-http(s) scheme is refused even though a built-in pane carries
        // an empty (unrestricted) allowlist — `file://` is never a read path.
        assert!(
            refusal(&mut harness, r#"{"url":"file:///etc/passwd"}"#)
                .starts_with("net_host_not_allowed"),
            "file:// must be refused under an empty allowlist"
        );
        assert!(refusal(&mut harness, r#"{"url":"https://example.com","method":"TRACE"}"#)
            .starts_with("invalid_input"));

        let missing_pane = {
            let (tx, rx) = std::sync::mpsc::sync_channel(1);
            harness
                .app
                .handle_assistant_net_fetch(r#"{"url":"https://example.com"}"#, 999_999, tx);
            rx.recv_timeout(std::time::Duration::from_secs(5))
                .unwrap()
                .error
                .unwrap()
        };
        assert!(missing_pane.starts_with("pane_not_found"), "{missing_pane}");

        // SSRF guard: loopback / private / link-local reach the host machine
        // or LAN; every IP literal plus localhost names are refused even
        // under an unrestricted allowlist.
        for url in [
            "http://127.0.0.1:8080/admin",
            "http://10.0.0.5/metadata",
            "http://192.168.1.1/router",
            "http://169.254.169.254/latest/meta-data/",
            "http://[::1]/",
            "http://[fe80::1]/",
            "http://8.8.8.8/",
            "http://localhost:3000/",
            "http://dev.localhost/",
        ] {
            let error = refusal(&mut harness, &format!(r#"{{"url":"{url}"}}"#));
            assert!(
                error.starts_with("net_destination_rejected"),
                "{url} must be rejected, got: {error}"
            );
        }
        assert_eq!(super::fetch_destination_rejected("https://example.com/x"), None);
    }

    /// The allowlist gating `host.net.fetch` is the pane's own
    /// `allowed_hosts` — the same source `DrawCommand::HttpRequest` uses —
    /// not anything the tool call declares.
    #[test]
    fn net_fetch_allowlist_comes_from_pane_permissions_not_the_call() {
        let mut harness = HostHarness::new();
        let origin = harness.add_test_pane();
        let app = harness.app.windows[0]
            .panes
            .get_mut(&origin)
            .and_then(crate::host::pane::Pane::as_app_mut)
            .expect("test pane is an app pane");
        app.permissions.allowed_hosts = vec!["api.example.com".to_string()];

        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        harness.app.handle_assistant_net_fetch(
            r#"{"url":"https://evil.test/steal","allowed_hosts":["evil.test"]}"#,
            origin,
            tx,
        );
        let error = rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
            .error
            .expect("an off-allowlist host must be refused");
        assert!(error.starts_with("net_host_not_allowed"), "{error}");
    }

    #[test]
    fn fetch_body_truncation_lands_on_a_char_boundary() {
        let short = "hello";
        assert_eq!(
            super::truncate_fetch_body(short),
            ("hello".to_string(), false)
        );
        let long: String = std::iter::repeat_n('★', super::MAX_FETCH_BODY_CHARS + 500).collect();
        let (text, truncated) = super::truncate_fetch_body(&long);
        assert!(truncated);
        assert!(text.ends_with("… [body truncated]"), "{}", text.len());
        assert_eq!(
            text.chars().count(),
            super::MAX_FETCH_BODY_CHARS + "… [body truncated]".chars().count()
        );
    }

    #[test]
    fn scoped_file_helpers_write_read_edit_within_roots_and_reject_escapes() {
        let root = tempfile::tempdir().unwrap();
        let apps = root.path().join("apps");
        std::fs::create_dir_all(&apps).unwrap();
        let roots = vec![apps.clone()];
        let file = apps.join("demo/main.py").display().to_string();

        let created = super::write_scoped_file(&roots, &file, "count = 0\nprint(count)\n").unwrap();
        assert!(created.created);
        assert!(created.diff.is_none(), "fresh writes have no diff");
        assert_eq!(
            super::read_scoped_file(&roots, &file).unwrap(),
            "count = 0\nprint(count)\n"
        );

        let diff = super::edit_scoped_file(&roots, &file, "count = 0", "count = 5").unwrap();
        assert!(diff.contains("-count = 0"), "{diff}");
        assert!(diff.contains("+count = 5"), "{diff}");
        assert_eq!(
            super::read_scoped_file(&roots, &file).unwrap(),
            "count = 5\nprint(count)\n"
        );

        let no_match = super::edit_scoped_file(&roots, &file, "absent", "x").unwrap_err();
        assert!(no_match.starts_with("edit_no_match"), "{no_match}");

        let overwrite = super::write_scoped_file(&roots, &file, "a\na\n").unwrap();
        assert!(!overwrite.created);
        assert!(
            overwrite.diff.as_deref().is_some_and(|d| d.contains("+a")),
            "{:?}",
            overwrite.diff
        );
        let ambiguous = super::edit_scoped_file(&roots, &file, "a", "b").unwrap_err();
        assert!(ambiguous.starts_with("edit_ambiguous"), "{ambiguous}");

        let outside = super::write_scoped_file(&roots, "/tmp/plexi-escape.py", "x").unwrap_err();
        assert!(outside.starts_with("path_out_of_scope"), "{outside}");

        let traversal = apps.join("demo/../../etc/passwd").display().to_string();
        let escaped = super::read_scoped_file(&roots, &traversal).unwrap_err();
        assert!(escaped.starts_with("path_traversal_rejected"), "{escaped}");

        assert_eq!(
            super::read_scoped_file(&roots, "demo/main.py").unwrap(),
            "a\na\n"
        );
    }

    #[test]
    fn read_slice_numbers_lines_and_pages_through_the_file() {
        let root = tempfile::tempdir().unwrap();
        let apps = root.path().join("apps");
        std::fs::create_dir_all(apps.join("demo")).unwrap();
        let roots = vec![apps.clone()];
        let file = apps.join("demo/main.py").display().to_string();
        let body = (1..=10).map(|n| format!("line{n}\n")).collect::<String>();
        super::write_scoped_file(&roots, &file, &body).unwrap();

        let all = super::read_scoped_file_slice(&roots, &file, 1, 2000).unwrap();
        assert_eq!(all.total_lines, 10);
        assert_eq!(all.lines_returned, 10);
        assert!(
            all.content.starts_with("     1\tline1\n"),
            "{}",
            all.content
        );

        let page = super::read_scoped_file_slice(&roots, &file, 4, 2).unwrap();
        assert_eq!(page.lines_returned, 2);
        assert_eq!(page.content, "     4\tline4\n     5\tline5\n");

        let past_end = super::read_scoped_file_slice(&roots, &file, 99, 5).unwrap_err();
        assert!(past_end.contains("past the end"), "{past_end}");
        let zero = super::read_scoped_file_slice(&roots, &file, 0, 5).unwrap_err();
        assert!(zero.starts_with("invalid_input"), "{zero}");
    }

    #[test]
    fn grep_scoped_matches_lines_respects_caps_and_rejects_bad_patterns() {
        let root = tempfile::tempdir().unwrap();
        let apps = root.path().join("apps");
        std::fs::create_dir_all(apps.join("demo")).unwrap();
        std::fs::create_dir_all(apps.join("demo/__pycache__")).unwrap();
        let roots = vec![apps.clone()];
        std::fs::write(
            apps.join("demo/main.py"),
            "def view():\n    return Button('go')\n\ndef update(event):\n    pass\n",
        )
        .unwrap();
        std::fs::write(apps.join("demo/__pycache__/skip.py"), "def view(): pass\n").unwrap();

        let result = super::grep_scoped(&roots, None, r"def \w+\(", 50).unwrap();
        let matches = result["matches"].as_array().unwrap();
        assert_eq!(matches.len(), 2, "{result}");
        assert_eq!(matches[0]["line"], 1);
        assert!(matches[0]["file"]
            .as_str()
            .unwrap()
            .ends_with("demo/main.py"));

        let capped = super::grep_scoped(&roots, None, r"def \w+\(", 1).unwrap();
        assert_eq!(capped["matches"].as_array().unwrap().len(), 1);
        assert_eq!(capped["truncated"], true);

        let scoped = super::grep_scoped(
            &roots,
            Some(&apps.join("demo/main.py").display().to_string()),
            "update",
            50,
        )
        .unwrap();
        assert_eq!(scoped["matches"].as_array().unwrap().len(), 1);

        let bad = super::grep_scoped(&roots, None, "def (", 50).unwrap_err();
        assert!(bad.starts_with("invalid_pattern"), "{bad}");
        let missing = super::grep_scoped(&roots, Some("/nope"), "x", 50).unwrap_err();
        assert!(missing.starts_with("path_out_of_scope"), "{missing}");
    }

    #[test]
    fn list_scoped_walks_sorted_and_skips_junk_dirs() {
        let root = tempfile::tempdir().unwrap();
        let apps = root.path().join("apps");
        std::fs::create_dir_all(apps.join("demo/.venv")).unwrap();
        let roots = vec![apps.clone()];
        std::fs::write(apps.join("demo/manifest.toml"), "x").unwrap();
        std::fs::write(apps.join("demo/main.py"), "y").unwrap();
        std::fs::write(apps.join("demo/.venv/junk.py"), "z").unwrap();

        let result = super::list_scoped(&roots, None).unwrap();
        let entries = result["entries"].as_array().unwrap();
        let paths: Vec<&str> = entries
            .iter()
            .map(|entry| entry["path"].as_str().unwrap())
            .collect();
        assert_eq!(paths.len(), 2, "{paths:?}");
        assert!(paths[0].ends_with("demo/main.py"), "{paths:?}");
        assert!(paths[1].ends_with("demo/manifest.toml"), "{paths:?}");
        assert_eq!(result["truncated"], false);
    }

    /// Review fix (stint 0421): the diff cap must land on a char boundary and
    /// stop building once reached — a full-file rewrite of multibyte content
    /// used to byte-truncate and could panic mid-char.
    #[test]
    fn unified_diff_truncates_char_safely_on_multibyte_rewrites() {
        let old: String = (0..400).map(|n| format!("línea →{n}★\n")).collect();
        let new: String = (0..400).map(|n| format!("нова →{n}✦\n")).collect();
        let diff = super::unified_diff(&old, &new, "demo/main.py");
        assert!(diff.contains("… [diff truncated]"), "{}", diff.len());
        assert!(diff.len() < super::MAX_DIFF_CHARS + 200, "{}", diff.len());
    }

    /// Review fix (stint 0421): symlinks inside an apps dir must not widen
    /// the auto-allowed grep/list walks beyond the scoped roots.
    #[cfg(unix)]
    #[test]
    fn grep_and_list_walks_never_follow_symlinks() {
        let root = tempfile::tempdir().unwrap();
        let apps = root.path().join("apps");
        let outside = root.path().join("outside");
        std::fs::create_dir_all(apps.join("demo")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(apps.join("demo/main.py"), "inside = 1\n").unwrap();
        std::fs::write(outside.join("secret.py"), "outside = 1\n").unwrap();
        std::os::unix::fs::symlink(&outside, apps.join("demo/escape")).unwrap();
        std::os::unix::fs::symlink(outside.join("secret.py"), apps.join("demo/leak.py")).unwrap();
        let roots = vec![apps.clone()];

        let listed = super::list_scoped(&roots, None).unwrap();
        let paths: Vec<&str> = listed["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["path"].as_str().unwrap())
            .collect();
        assert_eq!(paths.len(), 1, "{paths:?}");
        assert!(paths[0].ends_with("demo/main.py"), "{paths:?}");

        let matches = super::grep_scoped(&roots, None, "= 1", 50).unwrap();
        assert_eq!(matches["matches"].as_array().unwrap().len(), 1, "{matches}");
    }

    #[test]
    fn unified_diff_wraps_changed_span_in_context() {
        let old = "a\nb\nc\nd\ne\nf\ng\nh\ni\n";
        let new = "a\nb\nc\nd\nE\nf\ng\nh\ni\n";
        let diff = super::unified_diff(old, new, "demo/main.py");
        assert!(diff.starts_with("--- a/demo/main.py\n+++ b/demo/main.py\n"));
        assert!(diff.contains("@@ -2,7 +2,7 @@"), "{diff}");
        assert!(diff.contains("-e\n+E\n"), "{diff}");
        assert!(
            !diff.contains("\na\n"),
            "context must stay within 3 lines: {diff}"
        );
        assert_eq!(super::unified_diff("same\n", "same\n", "x"), "");

        let grow = super::unified_diff("a\n", "a\nb\nc\n", "x");
        assert!(grow.contains("+b\n+c\n"), "{grow}");
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
    fn files_tools_use_context_root_and_accept_relative_paths() {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(workspace.path().join("src")).unwrap();
        std::fs::write(workspace.path().join("README.md"), "workspace marker\n").unwrap();
        std::fs::write(workspace.path().join("src/lib.rs"), "pub fn marker() {}\n").unwrap();

        let mut harness = HostHarness::new();
        let origin = harness.add_test_pane();
        let context = harness.app.windows[0].context_id;
        harness
            .app
            .set_context_root(workspace.path().to_path_buf(), None);

        let listed =
            harness
                .app
                .handle_assistant_host_tool("host.files.list", "{}", origin, context);
        assert!(
            listed
                .output_json
                .as_deref()
                .is_some_and(|output| output.contains("README.md")
                    && output.contains("src/lib.rs")),
            "{listed:?}"
        );

        let read = harness.app.handle_assistant_host_tool(
            "host.files.read",
            r#"{"path":"src/lib.rs"}"#,
            origin,
            context,
        );
        assert!(
            read.output_json
                .as_deref()
                .is_some_and(|output| output.contains("pub fn marker()")),
            "{read:?}"
        );

        let write = harness.app.handle_assistant_host_tool(
            "host.files.write",
            r#"{"path":"notes/new.txt","content":"before\n"}"#,
            origin,
            context,
        );
        assert!(write.error.is_none(), "{write:?}");

        let edit = harness.app.handle_assistant_host_tool(
            "host.files.edit",
            r#"{"path":"notes/new.txt","old_string":"before","new_string":"after"}"#,
            origin,
            context,
        );
        assert!(edit.error.is_none(), "{edit:?}");
        assert_eq!(
            std::fs::read_to_string(workspace.path().join("notes/new.txt")).unwrap(),
            "after\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn direct_file_tools_reject_symlink_escapes() {
        let fixture = tempfile::tempdir().unwrap();
        let workspace = fixture.path().join("workspace");
        let outside = fixture.path().join("outside");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret.txt"), "outside\n").unwrap();
        std::os::unix::fs::symlink(&outside, workspace.join("escape")).unwrap();
        let roots = vec![workspace.clone()];

        let read = super::read_scoped_file(
            &roots,
            &workspace.join("escape/secret.txt").display().to_string(),
        )
        .unwrap_err();
        assert!(read.starts_with("path_out_of_scope"), "{read}");

        let write = super::write_scoped_file(
            &roots,
            &workspace.join("escape/new.txt").display().to_string(),
            "outside\n",
        )
        .unwrap_err();
        assert!(write.starts_with("path_out_of_scope"), "{write}");
        assert!(!outside.join("new.txt").exists());
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
        let value = serde_json::from_str::<serde_json::Value>(read.output_json.as_deref().unwrap())
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
        assert!(no_arg
            .error
            .as_deref()
            .unwrap()
            .starts_with("invalid_input"));
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
