//! CLI front end for the Agents API and Agents dashboard.

use serde_json::{json, Value};

pub fn command_view_follow_cli() -> i32 {
    let workspace = match crate::cli::agent::resolve_workspace_cwd() {
        Ok(root) => root,
        Err(code) => return code,
    };
    log::info!("command_view:cli: follow workspace={}", workspace.display());
    super::events::stream_control_line(json!({
        "type": "command_view_follow",
        "workspace": workspace,
    }))
}

pub fn command_view_cli(op: &str, json_out: bool) -> i32 {
    let workspace = match crate::cli::agent::resolve_workspace_cwd() {
        Ok(root) => root,
        Err(code) => return code,
    };
    log::info!(
        "command_view:cli: op={op} workspace={}",
        workspace.display()
    );
    let _ = json_out;
    send_dashboard(dashboard_frame(&workspace, op, None), "agents-dashboard")
}

pub fn command_view_cancel_cli(run: &str) -> i32 {
    let workspace = match crate::cli::agent::resolve_workspace_cwd() {
        Ok(root) => root,
        Err(code) => return code,
    };
    log::info!("command_view:cli: cancel run={run}");
    send_dashboard(
        dashboard_frame(&workspace, "cancel", Some(run)),
        "agents-dashboard cancel",
    )
}

/// The host's `agents_dashboard` request for `op` in `workspace`.
fn dashboard_frame(workspace: &std::path::Path, op: &str, run: Option<&str>) -> Value {
    let mut payload = json!({"workspace": workspace});
    if let Some(run) = run {
        payload["run"] = json!(run);
    }
    json!({"type": "agents_dashboard", "op": op, "payload": payload})
}

/// Send one dashboard request and print the host's answer; 0 when it is ok.
fn send_dashboard(frame: Value, label: &str) -> i32 {
    let content = match super::request_with(
        frame,
        &label.replace(' ', "-"),
        label,
        std::time::Duration::from_secs(20),
    ) {
        Ok(content) => content,
        Err(code) => return code,
    };
    println!("{content}");
    match serde_json::from_str::<Value>(&content) {
        Ok(value) if value.get("ok").and_then(|item| item.as_bool()) == Some(true) => 0,
        _ => 1,
    }
}

/// `resolve` and `allow` do not grant. From an agent pane they are refused.
pub fn command_view_refused(kind: &str) -> i32 {
    let from_pane = std::env::var_os("PLEXI_PANE_ID").is_some();
    let (code, error) = if from_pane {
        (
            "agent_cannot_approve",
            "an agent pane cannot resolve or allow",
        )
    } else {
        (
            "not_an_approval",
            "agents-dashboard resolve and allow do not grant",
        )
    };
    log::info!("command_view:cli: refused {kind} code={code}");
    println!(
        "{}",
        json!({"ok": false, "error_code": code, "error": error, "command": kind})
    );
    eprintln!("error: {error}");
    1
}

pub fn agent_team_dispatch(cmd: crate::cli::args::AgentTeamCmd, json: bool) -> i32 {
    use crate::cli::args::AgentTeamCmd;
    let cwd = match std::env::current_dir() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("error: current directory: {error}");
            return 1;
        }
    };
    let workspace = crate::app::registry::resolve_workspace_root(&cwd);
    let (payload, json_out, timeout) = match cmd {
        AgentTeamCmd::Show { name } => {
            match crate::agent::team::load(workspace.as_deref(), &name) {
                Ok(mut team) => {
                    team.tie_break = Some(team.tie_break().to_string());
                    team.quorum = Some(team.quorum());
                    log::info!(
                        "agent_team: show name={name} workspace={}",
                        workspace.as_ref().map_or_else(
                            || "[global]".to_string(),
                            |path| path.display().to_string()
                        )
                    );
                    if json {
                        println!("{}", serde_json::json!({"name":name,"team":team}));
                    } else {
                        match toml::to_string_pretty(&team) {
                            Ok(text) => println!("{text}"),
                            Err(error) => {
                                eprintln!("error: serialize team: {error}");
                                return 1;
                            }
                        }
                    }
                    return 0;
                }
                Err(error) => {
                    eprintln!("error: {error}");
                    return 1;
                }
            }
        }
        AgentTeamCmd::Run {
            name,
            question,
            out,
            options,
            timeout,
        } => {
            let out = out.map(|p| if p.is_absolute() { p } else { cwd.join(p) });
            // Resolve the definition before assigning any execution or state
            // binding.  A missing folder definition is normal: global and
            // built-in teams run from home and unconfigured repositories.
            let (_, config_root) = match crate::agent::team::load_from_cwd(&cwd, &name) {
                Ok(resolved) => resolved,
                Err(error) => {
                    eprintln!("error: {error}");
                    return 1;
                }
            };
            (
                json!({"type":"agents_api", "op":"team_run", "payload": {
                    "execution_cwd": cwd, "config_root": config_root,
                    "state_dir": crate::config::config_dir(), "name":name, "question":question,
                    "out":out, "options":options, "timeout":timeout,
                }}),
                json,
                timeout,
            )
        }
    };
    let body = match super::request_with(
        payload,
        "agent-team",
        "agent team run",
        std::time::Duration::from_secs(timeout + 20),
    ) {
        Ok(body) => body,
        Err(code) => return code,
    };
    let value = match serde_json::from_str::<Value>(&body) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("error: invalid team reply: {error}");
            return 1;
        }
    };
    if json_out {
        println!("{body}");
    } else {
        println!("{}", team_run_text(&value));
    }
    if let Err(code) = super::check_reply_error(&body) {
        return code;
    }
    0
}

pub(crate) fn team_run_text(value: &Value) -> String {
    if value.get("ok").and_then(Value::as_bool) == Some(false) {
        return value["error"]
            .as_str()
            .unwrap_or("team run was refused")
            .to_string();
    }
    format!(
        "{}\nRESULT.md: {}",
        value["report"]
            .as_str()
            .unwrap_or_else(|| value["error"].as_str().unwrap_or("No team report returned")),
        value["result_path"].as_str().unwrap_or("unavailable")
    )
}

#[cfg(test)]
mod tests {
    use crate::cli::test_env::socket_env_guard;
    use std::io::{BufRead as _, BufReader};
    use std::os::unix::net::UnixListener;

    /// Codex 3290 r2: `plexi agents-dashboard cancel` and `list` send a frame
    /// the host reads as its dashboard request. The CLI's send path writes
    /// the line to a socket; it must parse as `HostRequest::AgentsDashboard`
    /// (the cwd-to-workspace step is the only part left out).
    #[test]
    fn dashboard_cli_frames_are_the_hosts_dashboard_request() {
        let workspace = tempfile::tempdir().unwrap();
        for (op, run) in [("cancel", Some("run_x")), ("list", None)] {
            let frame = super::dashboard_frame(workspace.path(), op, run);
            let run_cli = move || super::send_dashboard(frame, "agents-dashboard");
            let env = socket_env_guard();
            let dir = tempfile::tempdir().unwrap();
            let socket_path = dir.path().join("notify.sock");
            let listener = UnixListener::bind(&socket_path).unwrap();
            env.set(&socket_path);
            let host = std::thread::spawn(move || {
                let (stream, _) = listener.accept().unwrap();
                let mut line = String::new();
                BufReader::new(stream).read_line(&mut line).unwrap();
                let request: crate::protocol::HostRequest = serde_json::from_str(&line)
                    .unwrap_or_else(|error| panic!("the host cannot read {line}: {error}"));
                let crate::protocol::HostRequest::AgentsDashboard { op, response_file, .. } = request
                else {
                    panic!("not a dashboard request: {line}");
                };
                crate::rpc::write_json_response(&response_file, serde_json::json!({"ok": true}));
                op
            });
            assert_eq!(run_cli(), 0, "{op}");
            assert_eq!(host.join().unwrap(), op);
            drop(env);
        }
    }
}
