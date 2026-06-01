use super::{send_to_socket};
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
    send_to_socket(serde_json::json!({
        "type": "set_context_root",
        "root": root,
    }))
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
