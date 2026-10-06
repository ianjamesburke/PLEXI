//! CLI front end for the Agents API. The host owns heads, runs, and the gate.

use crate::cli::args::{AgentHeadCmd, AgentRunCmd};
use serde_json::{json, Value};

pub fn agent_head_dispatch(cmd: AgentHeadCmd) -> i32 {
    match cmd {
        AgentHeadCmd::Create {
            name,
            display_name,
            description,
            grant,
            json,
        } => {
            let mut payload = json!({"name": name, "grants": grant});
            if let Ok(pane) = std::env::var("PLEXI_PANE_ID") {
                if let Ok(pane_id) = pane.parse::<u64>() {
                    payload["caller_pane"] = json!(pane_id);
                    log::info!("agents_api:cli: create_head caller_pane={pane_id}");
                }
            }
            if let Some(display_name) = display_name {
                payload["display_name"] = json!(display_name);
            }
            if let Some(description) = description {
                payload["description"] = json!(description);
            }
            call("create_head", payload, json)
        }
        AgentHeadCmd::List { all, json } => call("list_heads", json!({"all": all}), json),
    }
}

pub fn agent_run_dispatch(cmd: AgentRunCmd) -> i32 {
    match cmd {
        AgentRunCmd::Spawn {
            head,
            admission,
            client_ref,
            kind,
            input_tokens,
            output_tokens,
            text,
            json,
        } => {
            let mut payload = json!({"head": head});
            if let Some(admission) = admission {
                payload["admission"] = json!(admission);
            }
            if let Some(client_ref) = client_ref {
                payload["client_ref"] = json!(client_ref);
            }
            if let Some(kind) = kind {
                payload["kind"] = json!(kind);
            }
            if let Some(input_tokens) = input_tokens {
                payload["input_tokens"] = json!(input_tokens);
            }
            if let Some(output_tokens) = output_tokens {
                payload["output_tokens"] = json!(output_tokens);
            }
            if let Some(text) = text {
                payload["text"] = json!(text);
            }
            call_in(
                "spawn_run",
                payload,
                json,
                std::time::Duration::from_secs(120),
            )
        }
        AgentRunCmd::List { json } => call("list_runs", json!({}), json),
        AgentRunCmd::Show { id, json } => call("show_run", json!({"id": id}), json),
        AgentRunCmd::Finish { id, json } => call("finish_run", json!({"id": id}), json),
    }
}

pub fn agent_conversation_cli(head: &str, as_head: Option<&str>, json: bool) -> i32 {
    call(
        "read_conversation",
        json!({"head": head, "as_head": as_head}),
        json,
    )
}

pub fn command_view_cli(op: &str, json_out: bool) -> i32 {
    let workspace = match crate::cli::agent::resolve_workspace_cwd() {
        Ok(root) => root,
        Err(code) => return code,
    };
    log::info!("command_view:cli: op={op} workspace={}", workspace.display());
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

pub fn assistant_open_head_cli(head: &str) -> i32 {
    log::info!("assistant_open:cli: head={head}");
    let content = match super::request_with(
        json!({"type": "open_assistant_head", "head": head}),
        "assistant-open",
        "assistant open",
        std::time::Duration::from_secs(20),
    ) {
        Ok(content) => content,
        Err(code) => return code,
    };
    println!("{content}");
    match serde_json::from_str::<Value>(&content) {
        Ok(value) if value.get("ok").and_then(|v| v.as_bool()) == Some(true) => 0,
        Ok(_) => 1,
        Err(_) => 1,
    }
}

pub fn agent_delegate_cli(parent_run: &str, name: &str, grant: &[String], json: bool) -> i32 {
    call(
        "delegate",
        json!({"parent_run": parent_run, "name": name, "grants": grant}),
        json,
    )
}

fn call(op: &str, payload: Value, json_out: bool) -> i32 {
    call_in(op, payload, json_out, std::time::Duration::from_secs(15))
}

fn call_in(op: &str, mut payload: Value, json_out: bool, timeout: std::time::Duration) -> i32 {
    let workspace = match crate::cli::agent::resolve_workspace_cwd() {
        Ok(root) => root,
        Err(code) => return code,
    };
    let Some(obj) = payload.as_object_mut() else {
        eprintln!("error: internal: agents payload is not an object");
        return 1;
    };
    obj.insert(
        "workspace".to_string(),
        Value::String(workspace.to_string_lossy().into_owned()),
    );
    log::info!("agents_api:cli: op={op} workspace={}", workspace.display());
    let content = match super::request_with(
        json!({"type": "agents_api", "op": op, "payload": payload}),
        "agents-api",
        "agent",
        timeout,
    ) {
        Ok(content) => content,
        Err(code) => return code,
    };
    let value = match serde_json::from_str::<Value>(&content) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("error: host reply is not JSON: {error}");
            println!("{content}");
            return 1;
        }
    };
    if json_out {
        println!("{content}");
    } else {
        print_human(&value);
    }
    if value.get("ok").and_then(|v| v.as_bool()) == Some(true) {
        0
    } else {
        1
    }
}

fn print_human(value: &Value) {
    if value.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        let code = value
            .get("error_code")
            .and_then(|v| v.as_str())
            .unwrap_or("error");
        let error = value
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("request failed");
        eprintln!("error: {code}: {error}");
        return;
    }
    if let Some(heads) = value.get("heads").and_then(|v| v.as_array()) {
        if heads.is_empty() {
            println!("No agent heads.");
            return;
        }
        for head in heads {
            println!(
                "{}  {}",
                head.get("id").and_then(|v| v.as_str()).unwrap_or("?"),
                grant_list(head.get("grants"))
            );
        }
        return;
    }
    if let Some(runs) = value.get("runs").and_then(|v| v.as_array()) {
        if runs.is_empty() {
            println!("No agent runs.");
            return;
        }
        for run in runs {
            println!("{}", run_line(run));
        }
        return;
    }
    if let Some(run) = value.get("run") {
        println!("{}", run_line(run));
        return;
    }
    if let Some(head) = value.get("head") {
        println!(
            "created {}  {}",
            head.get("id").and_then(|v| v.as_str()).unwrap_or("?"),
            grant_list(head.get("grants"))
        );
        return;
    }
    println!("{value}");
}

fn run_line(run: &Value) -> String {
    format!(
        "{}  head={}  state={}",
        run.get("id").and_then(|v| v.as_str()).unwrap_or("?"),
        run.get("head_id").and_then(|v| v.as_str()).unwrap_or("?"),
        run.get("state").and_then(|v| v.as_str()).unwrap_or("?"),
    )
}

fn grant_list(grants: Option<&Value>) -> String {
    grants
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|grant| {
                    let tool = grant.get("tool")?.as_str()?;
                    let decision = grant.get("decision")?.as_str()?;
                    Some(format!("{tool}={decision}"))
                })
                .collect::<Vec<_>>()
                .join(",")
        })
        .unwrap_or_default()
}
