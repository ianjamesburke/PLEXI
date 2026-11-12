//! `plexi permissions` — the permission monitor, from the terminal.
//!
//! A pane id or a call credential marks the caller as an agent. Reset and
//! allow from an agent file Needs you and do not change the stored decision.
//! Revoke narrows and runs for either caller.

/// `plexi permissions list [--json]`, and `reset` / `revoke` / `allow <id>`.
pub fn permissions_cli(op: &str, id: Option<&str>, json: bool) -> i32 {
    let response_file = crate::rpc::response_file("permissions", "json");
    let (pane_id, credential) = caller_fields();
    let payload = serde_json::json!({
        "type": "permissions",
        "op": op,
        "id": id,
        "pane_id": pane_id,
        "credential": credential,
        "response_file": response_file,
    });
    log::info!("permissions:cli: op={op} id={id:?} pane={pane_id:?}");
    let content = match super::request_with(
        payload,
        "permissions",
        "permissions",
        std::time::Duration::from_secs(15),
    ) {
        Ok(content) => content,
        Err(code) => return code,
    };
    let value = match serde_json::from_str::<serde_json::Value>(&content) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("error: host reply is not JSON: {error}");
            return 1;
        }
    };
    if op == "list" && !json {
        print_table(&value);
    } else {
        println!("{value}");
    }
    if value.get("ok").and_then(|item| item.as_bool()) == Some(true) {
        0
    } else {
        1
    }
}

fn caller_fields() -> (Option<u64>, Option<String>) {
    let pane_id = std::env::var("PLEXI_PANE_ID")
        .ok()
        .filter(|value| !value.is_empty())
        .and_then(|value| value.parse().ok());
    let mut credential = std::env::var("PLEXI_CALL_CREDENTIAL")
        .ok()
        .filter(|value| !value.is_empty());
    // Clearing the env vars is not a human. A child of a pane on this host's
    // socket is still an agent (`caller_is_pane_agent`).
    if pane_id.is_none() && credential.is_none() && super::pane_caller::caller_is_pane_agent() {
        credential = Some("pane-ancestor".to_string());
    }
    (pane_id, credential)
}

fn print_table(value: &serde_json::Value) {
    let Some(entries) = value.get("entries").and_then(|item| item.as_array()) else {
        println!("{value}");
        return;
    };
    if entries.is_empty() {
        println!("no permission decisions");
        return;
    }
    println!("id\tkind\tduration\tactor\ttool\twhen");
    for entry in entries {
        let cell = |key: &str| entry.get(key).and_then(|item| item.as_str()).unwrap_or("");
        println!(
            "{}\t{}\t{}\t{}\t{}\t{}",
            cell("id"),
            cell("kind"),
            cell("duration"),
            cell("actor_id"),
            cell("tool"),
            cell("when")
        );
    }
}
