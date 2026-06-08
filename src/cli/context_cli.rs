use super::{print_tip, send_to_socket};
use super::validate::resolve_path;

pub fn context_new_cli(name: Option<&str>, path: Option<&str>, parent: Option<&str>) -> i32 {
    // Only resolve root when the user explicitly passed a path argument.
    // Without an explicit path, we let the host use the focused pane's cwd.
    let explicit_root = match path {
        Some(_) => match resolve_path(path) {
            Ok(p) => Some(p),
            Err(e) => { eprintln!("{e}"); return 1; }
        },
        None => None,
    };
    // Only pass parent_name when the user explicitly provided --parent.
    // Auto-resolving from PLEXI_CONTEXT_NAME would route to new_child_context
    // (which always creates a fresh terminal), bypassing the wrap behavior.
    let explicit_parent = parent
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    log::info!(
        "context_new_cli: name={:?} root={:?} parent={:?}",
        name,
        explicit_root.as_ref().map(|p: &std::path::PathBuf| p.display().to_string()),
        explicit_parent.as_deref()
    );
    let mut payload = serde_json::json!({ "type": "create_context" });
    if let Some(r) = explicit_root {
        payload["root"] = serde_json::Value::String(r.to_string_lossy().into_owned());
    }
    if let Some(n) = name {
        payload["name"] = serde_json::Value::String(n.to_string());
    }
    if let Some(p) = explicit_parent {
        payload["parent_name"] = serde_json::Value::String(p);
    }
    send_to_socket(payload)
}

/// `plexi context zoom <context_id>`
///
/// Zoom into a sub-context by its numeric context_id.
pub fn context_zoom_cli(context_id: u64) -> i32 {
    send_to_socket(serde_json::json!({
        "type": "zoom_into_context",
        "context_id": context_id,
    }))
}

/// `plexi context zoom-out`
///
/// Zoom out of the current sub-context to the parent.
pub fn context_zoom_out_cli() -> i32 {
    send_to_socket(serde_json::json!({
        "type": "zoom_out_of_context",
    }))
}

/// `plexi context open [path]`
///
/// Focuses the context with matching root, or creates one. Uses CWD if path omitted.
pub fn context_open_cli(path: Option<&str>) -> i32 {
    let root = match resolve_path(path) {
        Ok(p) => p,
        Err(e) => { eprintln!("{e}"); return 1; }
    };
    send_to_socket(serde_json::json!({
        "type": "focus_context",
        "root": root,
    }))
}

/// `plexi context set-root [path]`
///
/// Sets the root of the active context. Uses CWD if path omitted.
pub fn context_set_root_cli(path: Option<&str>) -> i32 {
    let root = match resolve_path(path) {
        Ok(p) => p,
        Err(e) => { eprintln!("{e}"); return 1; }
    };
    let rc = send_to_socket(serde_json::json!({
        "type": "set_context_root",
        "root": root,
    }));
    if rc == 0 {
        print_tip("you can also press \u{21E7}\u{2318}I to set the context root from the focused pane");
    }
    rc
}

/// `plexi context describe "text"`
///
/// Sets the description of the active context.
pub fn context_describe_cli(text: &str) -> i32 {
    send_to_socket(serde_json::json!({
        "type": "set_context_description",
        "description": text,
    }))
}

/// `plexi context push [name]`
///
/// Push the focused pane into a new sub-context. The pane becomes a portal
/// and its content moves into the child context.
pub fn context_push_cli(name: Option<&str>) -> i32 {
    let mut payload = serde_json::json!({ "type": "push_pane_to_subcontext" });
    if let Some(n) = name {
        payload["name"] = serde_json::Value::String(n.to_string());
    }
    send_to_socket(payload)
}

/// `plexi context current`
///
/// Prints the context ID and name for the current pane as JSON.
/// Reads PLEXI_CONTEXT_ID and PLEXI_CONTEXT_NAME set at pane spawn time.
pub fn context_current_cli() -> i32 {
    let context_id = match std::env::var("PLEXI_CONTEXT_ID") {
        Ok(v) => v,
        Err(_) => {
            eprintln!("error: PLEXI_CONTEXT_ID is not set — run this inside a Plexi terminal pane");
            return 1;
        }
    };
    let context_name = std::env::var("PLEXI_CONTEXT_NAME").unwrap_or_default();
    let context_description = std::env::var("PLEXI_CONTEXT_DESCRIPTION").unwrap_or_default();
    let id_num: u64 = match context_id.parse() {
        Ok(n) => n,
        Err(_) => {
            eprintln!("error: PLEXI_CONTEXT_ID is not a valid number: {context_id}");
            return 1;
        }
    };
    let json = serde_json::json!({
        "context_id": id_num,
        "context_name": context_name,
        "context_description": context_description,
    });
    match serde_json::to_string_pretty(&json) {
        Ok(s) => println!("{s}"),
        Err(e) => {
            eprintln!("error: failed to serialize context JSON: {e}");
            return 1;
        }
    }
    0
}

/// `plexi context list`
///
/// Sends a `list_contexts` command to PLEXI_SOCKET. The host writes a JSON array
/// to a response file; this function polls for it and prints it to stdout.
/// Returns 0 on success, 1 on error.
pub fn context_list_cli() -> i32 {
    let id = uuid::Uuid::new_v4();
    let response_file = crate::config::config_dir()
        .join(format!("context-list-response-{id}.json"))
        .to_string_lossy()
        .into_owned();

    let payload = serde_json::json!({
        "type": "list_contexts",
        "response_file": response_file,
    });

    log::info!("context_list:cli: sending via socket response_file={:?}", response_file);

    let code = send_to_socket(payload);
    if code != 0 {
        return code;
    }

    let response_path = std::path::PathBuf::from(&response_file);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if response_path.exists() {
            match std::fs::read_to_string(&response_path) {
                Ok(content) => {
                    let _ = std::fs::remove_file(&response_path);
                    return print_json_output(&content);
                }
                Err(e) => {
                    log::warn!("context_list:cli: could not read response file: {e}");
                    eprintln!("error: could not read response file: {e}");
                    return 1;
                }
            }
        }
        if std::time::Instant::now() >= deadline {
            eprintln!("error: timed out waiting for context list response");
            return 1;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

fn print_json_output(json_str: &str) -> i32 {
    use std::io::Write as _;
    use std::process::{Command, Stdio};

    let jq_available = Command::new("jq")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if jq_available {
        match Command::new("jq").arg(".").stdin(Stdio::piped()).spawn() {
            Ok(mut child) => {
                if let Some(mut stdin) = child.stdin.take() {
                    let _ = stdin.write_all(json_str.as_bytes());
                }
                let _ = child.wait();
                log::info!("context_list:print_json_output: rendered via jq");
                return 0;
            }
            Err(e) => {
                log::warn!("context_list:print_json_output: jq spawn failed ({e}), falling back to serde");
            }
        }
    }

    match serde_json::from_str::<serde_json::Value>(json_str) {
        Ok(v) => match serde_json::to_string_pretty(&v) {
            Ok(pretty) => {
                println!("{pretty}");
                0
            }
            Err(e) => {
                eprintln!("error: could not serialize: {e}");
                1
            }
        },
        Err(_) => {
            print!("{json_str}");
            0
        }
    }
}
