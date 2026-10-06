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

pub fn command_view_send_cli(lead: &str, text: &str) -> i32 {
    if text.trim().is_empty() {
        eprintln!("error: text is required");
        return 1;
    }
    let workspace = match crate::cli::agent::resolve_workspace_cwd() {
        Ok(root) => root,
        Err(code) => return code,
    };
    let request_id = uuid::Uuid::new_v4().to_string();
    log::info!("command_view:cli: send lead={lead} request_id={request_id}");
    let content = match super::request_with(
        json!({
            "type": "command_view",
            "op": "send",
            "payload": {
                "workspace": workspace,
                "lead": lead,
                "text": text,
                "request_id": request_id,
            }
        }),
        "command-view-send",
        "command-view send",
        std::time::Duration::from_secs(120),
    ) {
        Ok(content) => content,
        Err(code) => return code,
    };
    println!("{content}");
    match serde_json::from_str::<Value>(&content) {
        Ok(value) if value.get("state").and_then(|item| item.as_str()) == Some("succeeded") => 0,
        Ok(_) => 2,
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

fn task_text(path: &std::path::Path) -> Result<String, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|error| format!("read {}: {error}", path.display()))?;
    if let Ok(value) = serde_json::from_str::<Value>(&raw) {
        if let Some(text) = value.get("text").and_then(|item| item.as_str()) {
            return Ok(text.to_string());
        }
    }
    Ok(raw.trim().to_string())
}

fn print_queue(body: &Value, json_out: bool) -> i32 {
    if json_out {
        println!(
            "{}",
            serde_json::to_string(body).unwrap_or_else(|_| "{}".to_string())
        );
    } else if body.get("ok").and_then(|value| value.as_bool()) == Some(true) {
        if let Some(task) = body.get("task") {
            println!(
                "{}  {}",
                task.get("id")
                    .and_then(|value| value.as_str())
                    .unwrap_or("?"),
                task.get("state")
                    .and_then(|value| value.as_str())
                    .unwrap_or("?")
            );
        } else {
            println!("{body}");
        }
    } else {
        let error = body
            .get("error")
            .and_then(|value| value.as_str())
            .unwrap_or("queue request failed");
        eprintln!("error: {error}");
    }
    if body.get("ok").and_then(|value| value.as_bool()) == Some(true) {
        0
    } else {
        1
    }
}

fn notify_host_pump(workspace: &std::path::Path) {
    let Some(socket) = super::resolve_command_socket() else {
        log::info!("queue: no command socket; task stays on disk");
        return;
    };
    if crate::platform::ipc::IpcStream::connect(&socket).is_err() {
        log::info!("queue: host is not accepting commands; task stays queued");
        return;
    }
    let payload = json!({
        "type": "agent_queue",
        "op": "pump",
        "payload": {"workspace": workspace},
    });
    match super::request_with(
        payload,
        "agent-queue",
        "agent queue",
        std::time::Duration::from_secs(20),
    ) {
        Ok(_) => log::info!("queue: host pumped {}", workspace.display()),
        Err(code) => log::info!("queue: host pump returned {code}; the task remains on disk"),
    }
}

pub fn agent_assign_cli(head: &str, input: &std::path::Path, json_out: bool) -> i32 {
    let workspace = match crate::cli::agent::resolve_workspace_cwd() {
        Ok(root) => root,
        Err(code) => return code,
    };
    let text = match task_text(input) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("error: {error}");
            return 1;
        }
    };
    log::info!(
        "agent_assign:cli: head={head} workspace={}",
        workspace.display()
    );
    let body = crate::agent::queue::enqueue(&workspace, head, &text);
    if body.get("ok").and_then(|value| value.as_bool()) == Some(true) {
        notify_host_pump(&workspace);
    }
    print_queue(&body, json_out)
}

pub fn agent_cancel_cli(id: &str, json_out: bool) -> i32 {
    let workspace = match crate::cli::agent::resolve_workspace_cwd() {
        Ok(root) => root,
        Err(code) => return code,
    };
    log::info!(
        "agent_cancel:cli: id={id} workspace={}",
        workspace.display()
    );
    let body = crate::agent::queue::request_cancel(&workspace, id);
    notify_host_pump(&workspace);
    print_queue(&body, json_out)
}
