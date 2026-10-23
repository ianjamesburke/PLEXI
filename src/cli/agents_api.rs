//! CLI front end for team orchestration.

use serde_json::{json, Value};

pub fn agent_team_dispatch(cmd: crate::cli::args::AgentTeamCmd, json: bool) -> i32 {
    use crate::cli::args::AgentTeamCmd;
    let cwd = match std::env::current_dir() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("error: current directory: {error}");
            return 1;
        }
    };
    let workspace = if matches!(&cmd, AgentTeamCmd::Show { .. }) {
        crate::app::registry::resolve_workspace_root(&cwd).unwrap_or_else(|| cwd.clone())
    } else {
        match crate::cli::agent::resolve_workspace_cwd() {
            Ok(root) => root,
            Err(code) => return code,
        }
    };
    let (payload, json_out, timeout) = match cmd {
        AgentTeamCmd::Show { name } => match crate::agent::team::load(&workspace, &name) {
            Ok(mut team) => {
                team.tie_break = Some(team.tie_break().to_string());
                team.quorum = Some(team.quorum());
                log::info!(
                    "agent_team: show name={name} workspace={}",
                    workspace.display()
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
        },
        AgentTeamCmd::Run {
            name,
            question,
            out,
            options,
            timeout,
        } => {
            let out = out.map(|p| if p.is_absolute() { p } else { cwd.join(p) });
            (
                json!({"type":"agents_api", "op":"team_run", "payload": {
                    "workspace": workspace, "name":name, "question":question,
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
        Err(error) => { eprintln!("error: invalid team reply: {error}"); return 1; }
    };
    if json_out { println!("{body}"); }
    else { println!("{}", team_run_text(&value)); }
    if let Err(code) = super::check_reply_error(&body) { return code; }
    0
}

pub(crate) fn team_run_text(value: &Value) -> String {
    format!("{}\nRESULT.md: {}", value["report"].as_str().unwrap_or_else(|| value["error"].as_str().unwrap_or("No team report returned")), value["result_path"].as_str().unwrap_or("unavailable"))
}
