//! `plexi command-view` — list the projection and steer a lead through the gate.

use serde_json::{Value, json};

use super::args::CommandViewCmd;

pub fn command_view_follow_cli() -> i32 {
    log::info!("command_view:cli: follow");
    super::events::stream_control_line(json!({"type":"command_view_follow"}))
}

pub fn command_view_cli(cmd: Option<CommandViewCmd>) -> i32 {
    let workspace = match std::env::current_dir() {
        Ok(dir) => crate::platform::path::canonical_or_self(&dir)
            .display()
            .to_string(),
        Err(error) => {
            eprintln!("error: current directory: {error}");
            return 1;
        }
    };
    let (op, lead, run, text, summary, tool, id, approve) = match cmd {
        None => (
            "list",
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            false,
        ),
        Some(CommandViewCmd::Send { lead, text }) => (
            "send",
            lead,
            String::new(),
            text,
            String::new(),
            String::new(),
            String::new(),
            false,
        ),
        Some(CommandViewCmd::Enqueue { lead, text }) => (
            "enqueue",
            lead,
            String::new(),
            text,
            String::new(),
            String::new(),
            String::new(),
            false,
        ),
        Some(CommandViewCmd::Pause { run }) => (
            "pause",
            String::new(),
            run,
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            false,
        ),
        Some(CommandViewCmd::Cancel { run }) => (
            "cancel",
            String::new(),
            run,
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            false,
        ),
        Some(CommandViewCmd::Block { lead, run, summary }) => (
            "block",
            lead,
            run,
            String::new(),
            summary,
            String::new(),
            String::new(),
            false,
        ),
        Some(CommandViewCmd::Resolve { id, approve, deny }) => (
            "resolve",
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            id,
            approve && !deny,
        ),
        Some(CommandViewCmd::Allow {
            tool,
            lead,
            run,
            text,
        }) => (
            "allow",
            lead.unwrap_or_default(),
            run.unwrap_or_default(),
            text.unwrap_or_default(),
            String::new(),
            tool,
            String::new(),
            false,
        ),
    };
    let response_file = crate::rpc::response_file("command-view", "json");
    let payload = json!({
        "type": "command_view",
        "op": op,
        "lead": lead,
        "run": run,
        "text": text,
        "summary": summary,
        "tool": tool,
        "id": id,
        "approve": approve,
        "workspace": workspace,
        "response_file": response_file,
    });
    log::info!("command_view:cli: op={op} lead={lead} run={run} tool={tool}");
    let content = match super::request_with(
        payload,
        "command-view",
        "command-view",
        std::time::Duration::from_secs(15),
    ) {
        Ok(content) => content,
        Err(code) => return code,
    };
    println!("{content}");
    match serde_json::from_str::<Value>(&content) {
        Ok(value)
            if value.get("error_code").and_then(|v| v.as_str()) == Some("permission_required") =>
        {
            2
        }
        Ok(value) if value.get("ok").and_then(|v| v.as_bool()) == Some(true) => 0,
        Ok(_) => 1,
        Err(_) => 1,
    }
}
