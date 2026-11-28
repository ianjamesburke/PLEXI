//! CLI front end for the Agents API and Agents dashboard.

use super::reply::OutputMode;
use serde_json::{json, Value};

pub fn command_view_follow_cli() -> i32 {
    let workspace = match crate::cli::agent::resolve_workspace_cwd() {
        Ok(root) => root,
        Err(code) => return code,
    };
    log::info!("command_view:cli: follow workspace={}", workspace.display());
    super::events::stream_control_line(
        json!({
            "type": "command_view_follow",
            "workspace": workspace,
        }),
        OutputMode::Text,
    )
}

pub fn command_view_cli(op: &str, mode: OutputMode) -> i32 {
    let workspace = match crate::cli::agent::resolve_workspace_cwd() {
        Ok(root) => root,
        Err(code) => return code,
    };
    log::info!(
        "command_view:cli: op={op} workspace={}",
        workspace.display()
    );
    send_dashboard(
        dashboard_frame(&workspace, op, None),
        "agents-dashboard",
        mode,
    )
}

pub fn command_view_cancel_cli(run: &str, mode: OutputMode) -> i32 {
    let workspace = match crate::cli::agent::resolve_workspace_cwd() {
        Ok(root) => root,
        Err(code) => return code,
    };
    command_view_cancel_in(&workspace, run, mode)
}

/// `plexi agents-dashboard cancel` for `workspace`: the whole entry past
/// finding the workspace.
fn command_view_cancel_in(workspace: &std::path::Path, run: &str, mode: OutputMode) -> i32 {
    log::info!("command_view:cli: cancel run={run}");
    send_dashboard(
        dashboard_frame(workspace, "cancel", Some(run)),
        "agents-dashboard cancel",
        mode,
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
fn send_dashboard(frame: Value, label: &str, mode: OutputMode) -> i32 {
    let content = match super::request_with(
        frame,
        &label.replace(' ', "-"),
        label,
        std::time::Duration::from_secs(20),
        mode,
    ) {
        Ok(content) => content,
        Err(code) => return code,
    };
    println!("{content}");
    match serde_json::from_str::<Value>(&content) {
        Ok(value) if super::reply::acknowledged(&value) => 0,
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

pub fn agent_team_dispatch(cmd: crate::cli::args::AgentTeamCmd, mode: OutputMode) -> i32 {
    use crate::cli::args::AgentTeamCmd;
    let cwd = match std::env::current_dir() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("error: current directory: {error}");
            return 1;
        }
    };
    let workspace = crate::app::registry::resolve_workspace_root(&cwd);
    let (payload, timeout) = match cmd {
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
                    if mode.is_json() {
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
                timeout,
            )
        }
    };
    let body = match super::request_with(
        payload,
        "agent-team",
        "agent team run",
        std::time::Duration::from_secs(timeout + 20),
        mode,
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
    if mode.is_json() {
        println!("{body}");
    } else {
        println!("{}", team_run_text(&value));
    }
    0
}

pub(crate) fn team_run_text(value: &Value) -> String {
    if let super::reply::HostReply::Refused(refusal) =
        super::reply::HostReply::decode(Ok(value.to_string()))
    {
        return refusal.reason;
    }
    format!(
        "{}\nRESULT.md: {}",
        value["report"]
            .as_str()
            .unwrap_or("No team report returned"),
        value["result_path"].as_str().unwrap_or("unavailable")
    )
}

#[cfg(test)]
mod tests {
    use super::OutputMode;
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
            let path = workspace.path().to_path_buf();
            // Cancel goes through its own entry, so a wrong operation built
            // there fails here (Codex 3290 r4).
            let run_cli = move || match run {
                Some(run) => super::command_view_cancel_in(&path, run, OutputMode::Text),
                None => super::send_dashboard(
                    super::dashboard_frame(&path, "list", None),
                    "agents-dashboard",
                    OutputMode::Text,
                ),
            };
            let env = socket_env_guard();
            let dir = tempfile::tempdir().unwrap();
            let socket_path = dir.path().join("notify.sock");
            let listener = UnixListener::bind(&socket_path).unwrap();
            env.set(&socket_path);
            let expected_workspace = workspace.path().to_path_buf();
            let host = std::thread::spawn(move || {
                use std::io::Write as _;
                let (stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let request: plexi_protocol::HostRequest = serde_json::from_str(&line)
                    .unwrap_or_else(|error| panic!("the host cannot read {line}: {error}"));
                let plexi_protocol::HostRequest::AgentsDashboard {
                    op,
                    payload,
                    response_file,
                } = request
                else {
                    panic!("not a dashboard request: {line}");
                };
                assert_eq!(
                    std::path::PathBuf::from(payload["workspace"].as_str().unwrap()),
                    expected_workspace
                );
                // Answer on the connection, as the host does; never a file.
                let mut stream = stream;
                stream.write_all(br#"{"ok":true}"#).unwrap();
                drop(stream);
                (op, payload, response_file)
            });
            assert_eq!(run_cli(), 0, "{op}");
            let (sent_op, payload, response_file) = host.join().unwrap();
            assert_eq!(sent_op, op);
            assert_eq!(payload["run"].as_str(), run, "{payload}");
            assert!(
                !std::path::Path::new(&response_file).exists(),
                "the reply took the file fallback"
            );
            drop(env);
        }
    }
}
