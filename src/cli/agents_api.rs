//! CLI front end for the Agents API and Command View.

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
    let content = match super::request_with(
        json!({"type": "command_view", "op": op, "payload": {"workspace": workspace}}),
        "command-view",
        "command-view",
        std::time::Duration::from_secs(20),
    ) {
        Ok(content) => content,
        Err(code) => return code,
    };
    println!("{content}");
    match serde_json::from_str::<Value>(&content) {
        Ok(value) if value.get("ok").and_then(|v| v.as_bool()) != Some(false) => {
            let _ = json_out;
            0
        }
        Ok(_) => 1,
        Err(_) => 1,
    }
}

pub fn command_view_cancel_cli(run: &str) -> i32 {
    let workspace = match crate::cli::agent::resolve_workspace_cwd() {
        Ok(root) => root,
        Err(code) => return code,
    };
    log::info!("command_view:cli: cancel run={run}");
    let content = match super::request_with(
        json!({
            "type": "command_view",
            "op": "cancel",
            "payload": {"workspace": workspace, "run": run}
        }),
        "command-view-cancel",
        "command-view cancel",
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
            "command-view resolve and allow do not grant",
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
                    "caller_pane":std::env::var("PLEXI_PANE_ID").ok().and_then(|p| p.parse::<u64>().ok())
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
