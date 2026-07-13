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
                match self.assistant_spawn_pane(
                    origin_pane_id,
                    origin_context_id,
                    type_id,
                    layout,
                    args,
                    cwd,
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
                match self.assistant_spawn_pane(
                    origin_pane_id,
                    origin_context_id,
                    app,
                    layout,
                    args,
                    None,
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

    fn assistant_spawn_pane(
        &mut self,
        origin_pane_id: u64,
        origin_context_id: u64,
        type_id: &str,
        layout: Option<String>,
        args: Vec<String>,
        cwd: Option<std::path::PathBuf>,
    ) -> Result<u64, String> {
        if !self.windows.iter().any(|window| {
            window.context_id == origin_context_id && window.panes.contains_key(&origin_pane_id)
        }) {
            return Err(format!(
                "origin pane {origin_pane_id} is no longer in context {origin_context_id}"
            ));
        }
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
            from_pane_id: Some(origin_pane_id),
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
        if let Some(created) = self
            .windows
            .iter()
            .flat_map(|window| window.panes.keys().copied())
            .find(|pane_id| !before.contains(pane_id))
        {
            return Ok(created);
        }
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
            .ok_or_else(|| format!("open request for '{type_id}' did not create or focus a pane"))
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
}
