//! `plexi permissions` — the permission monitor, from the terminal.
//!
//! No command-socket caller is human, whether it runs in a pane, in your own
//! terminal, or anywhere else. `allow` and `reset` widen, so they file a Needs
//! you item and change nothing until a click in the Plexi window approves it.
//! `revoke` narrows and applies directly.

/// Exit code when the request was filed and waits for a click in the Plexi
/// window (same meaning as `secret get`'s `permission_required`).
const EXIT_WAITING_FOR_APPROVAL: i32 = 2;

/// `plexi permissions list [--json]`, and `reset` / `revoke` / `allow <id>`.
pub fn permissions_cli(op: &str, id: Option<&str>, json: bool) -> i32 {
    let response_file = crate::rpc::response_file("permissions", "json");
    let payload = serde_json::json!({
        "type": "permissions",
        "op": op,
        "id": id,
        "response_file": response_file,
    });
    log::info!("permissions:cli: op={op} id={id:?}");
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
    if value.get("status").and_then(|item| item.as_str()) == Some("needs_you") {
        let item = value
            .get("needs_you_id")
            .and_then(|item| item.as_str())
            .unwrap_or("");
        eprintln!(
            "Filed Needs you item {item}. Nothing changed yet: approve it in the Plexi window \
             (Needs you). A command cannot {op} a permission by itself."
        );
        return EXIT_WAITING_FOR_APPROVAL;
    }
    if value.get("ok").and_then(|item| item.as_bool()) == Some(true) {
        0
    } else {
        1
    }
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
