use super::validate::resolve_path;
use super::{print_tip, send_to_socket};

pub fn context_new_cli(
    name: Option<&str>,
    path: Option<&str>,
    parent: Option<&str>,
    windows: &[String],
    focus: bool,
    direction: &str,
    from: Option<u64>,
) -> i32 {
    let explicit_root = match path {
        Some(_) => match resolve_path(path) {
            Ok(p) => Some(p),
            Err(e) => {
                eprintln!("{e}");
                return 1;
            }
        },
        None => None,
    };
    // "--parent" with no value resolves to the current context via env.
    // "--parent <name>" passes that name directly.
    // No "--parent" → None (top-level context).
    //
    // Bare "--parent" also sends PLEXI_CONTEXT_ID, which disambiguates when two
    // contexts share a name. An explicit "--parent=<name>" does not: the caller
    // named a context, so name resolution is what they asked for.
    let mut parent_context_id = None;
    let explicit_parent = match parent {
        None => None,
        Some("__current__") => match std::env::var("PLEXI_CONTEXT_NAME") {
            Ok(n) if !n.is_empty() => {
                parent_context_id = std::env::var("PLEXI_CONTEXT_ID")
                    .ok()
                    .and_then(|v| v.parse::<u64>().ok());
                Some(n)
            }
            _ => {
                eprintln!(
                    "error: --parent (no value) requires PLEXI_CONTEXT_NAME — run inside a Plexi pane"
                );
                return 1;
            }
        },
        Some(n) => {
            let trimmed = n.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
    };
    // Only request a response file when creating a sub-context (--parent set).
    // Top-level context new stays fire-and-forget to preserve existing behaviour.
    let response_file = explicit_parent
        .is_some()
        .then(|| crate::rpc::response_file("context-new-response", "json"));
    // Portal split anchor: explicit --from wins; otherwise the caller's own pane
    // (PLEXI_PANE_ID, set in every Plexi terminal). Only meaningful with --parent.
    let anchor_pane = from.or_else(|| {
        std::env::var("PLEXI_PANE_ID")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
    });
    log::info!(
        "context_new_cli: name={:?} root={:?} parent={:?} windows={} focus={focus} direction={direction} anchor_pane={anchor_pane:?}",
        name,
        explicit_root
            .as_ref()
            .map(|p: &std::path::PathBuf| p.display().to_string()),
        explicit_parent.as_deref(),
        windows.len()
    );
    let mut payload = serde_json::json!({ "type": "create_context" });
    if let Some(r) = explicit_root {
        payload["root"] = serde_json::Value::String(r.to_string_lossy().into_owned());
    }
    if let Some(n) = name {
        payload["name"] = serde_json::Value::String(n.to_string());
    }
    if let Some(p) = &explicit_parent {
        payload["parent_name"] = serde_json::Value::String(p.clone());
    }
    if let Some(id) = parent_context_id {
        payload["parent_context_id"] = serde_json::Value::from(id);
    }
    if !windows.is_empty() {
        payload["windows"] = serde_json::to_value(windows).expect("Vec<String> is serializable");
    }
    if focus {
        payload["focus"] = serde_json::Value::Bool(true);
    }
    if direction != "right" {
        payload["portal_direction"] = serde_json::Value::String(direction.to_string());
    }
    if explicit_parent.is_some() {
        if let Some(ap) = anchor_pane {
            payload["anchor_pane"] = serde_json::Value::from(ap);
        }
    }
    if let Some(ref rf) = response_file {
        payload["response_file"] = serde_json::Value::String(rf.clone());
    }
    let rc = send_to_socket(payload);
    if rc != 0 {
        return rc;
    }
    // Poll for response JSON (sub-context only).
    if let Some(rf) = response_file {
        match super::poll_rpc(&rf, "context-new") {
            Ok(content) => println!("{content}"),
            Err(code) => return code,
        }
    }
    0
}

/// Expand `--agents N` + repeated `--command` into exactly one entry per pane.
///
/// Zero commands means N plain shells; one command applies to every pane; N
/// commands assign one per pane in order. Any other count is an error — never a
/// silent truncation or a cycle, because a squad that quietly drops an agent's
/// job is worse than one that refuses to start.
pub(super) fn expand_pane_commands(
    agents: u32,
    commands: &[String],
) -> Result<Vec<Option<String>>, String> {
    let agents = agents as usize;
    match commands.len() {
        0 => Ok(vec![None; agents]),
        1 => Ok(vec![Some(commands[0].clone()); agents]),
        n if n == agents => Ok(commands.iter().cloned().map(Some).collect()),
        n => Err(format!(
            "error: --command given {n} times but --agents is {agents} — pass it once (same command for every pane) or exactly {agents} times (one per pane)"
        )),
    }
}

/// `plexi context sub <name> --agents N --command CMD`
///
/// Creates a sub-context under the caller's context and seeds it with exactly
/// `agents` panes in one tiled window, each running its assigned command.
pub fn context_sub_cli(
    name: &str,
    path: Option<&str>,
    agents: u32,
    commands: &[String],
    layout: crate::app_protocol::SubContextLayout,
    focus: bool,
    from: Option<u64>,
) -> i32 {
    let panes = match expand_pane_commands(agents, commands) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    // Deliberate divergence from `context new`: the sub-context roots at the
    // caller's cwd, not the parent context's path, so agents launched here
    // start in the directory the operator was standing in.
    let root = match resolve_path(path) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    // Parent by id — PLEXI_CONTEXT_ID is set in every Plexi pane and is unique;
    // the name is only a fallback for a pane that somehow has one without the
    // other.
    let parent_context_id = std::env::var("PLEXI_CONTEXT_ID")
        .ok()
        .and_then(|v| v.parse::<u64>().ok());
    let parent_name = std::env::var("PLEXI_CONTEXT_NAME")
        .ok()
        .filter(|n| !n.is_empty());
    if parent_context_id.is_none() && parent_name.is_none() {
        eprintln!("error: context sub requires PLEXI_CONTEXT_ID — run it inside a Plexi pane");
        return 1;
    }
    let anchor_pane = from.or_else(|| {
        std::env::var("PLEXI_PANE_ID")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
    });
    log::info!(
        "context_sub_cli: name={name:?} root={} parent_context_id={parent_context_id:?} \
         parent_name={parent_name:?} agents={agents} layout={layout:?} focus={focus} \
         anchor_pane={anchor_pane:?} panes={panes:?}",
        root.display()
    );
    let response_file = crate::rpc::response_file("context-sub-response", "json");
    let mut payload = serde_json::json!({
        "type": "create_sub_context",
        "name": name,
        "root": root.to_string_lossy(),
        "panes": panes,
        "layout": layout,
        "focus": focus,
        "response_file": response_file,
    });
    if let Some(id) = parent_context_id {
        payload["parent_context_id"] = serde_json::Value::from(id);
    }
    if let Some(n) = parent_name {
        payload["parent_name"] = serde_json::Value::String(n);
    }
    if let Some(ap) = anchor_pane {
        payload["anchor_pane"] = serde_json::Value::from(ap);
    }
    let rc = send_to_socket(payload);
    if rc != 0 {
        return rc;
    }
    match super::poll_rpc(&response_file, "context-sub") {
        Ok(content) => {
            // `poll_rpc` only distinguishes transport failures. The host reports
            // a refused parent or a failed terminal spawn *in* the payload, so
            // an unattended caller needs that surfaced as a nonzero exit rather
            // than a successful-looking JSON blob.
            if let Some(err) = sub_response_error(&content) {
                eprintln!("error: context sub failed: {err}");
                return 1;
            }
            println!("{content}");
            0
        }
        Err(code) => code,
    }
}

/// The `error` field of a `context sub` response, if the host reported one.
///
/// Unparseable output is not treated as an error here — `poll_rpc` already
/// owns transport failures, and swallowing a valid-but-unexpected shape would
/// hide the response from the caller.
fn sub_response_error(content: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(content)
        .ok()?
        .get("error")?
        .as_str()
        .map(|s| s.to_string())
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
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    send_to_socket(serde_json::json!({
        "type": "focus_context",
        "root": root,
    }))
}

/// `plexi context set-root [path]`
///
/// Sets the root of the caller's context (PLEXI_CONTEXT_ID), falling back to
/// the active context. Uses CWD if path omitted.
pub fn context_set_root_cli(path: Option<&str>) -> i32 {
    let root = match resolve_path(path) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let context_id = caller_context_id();
    log::info!(
        "context_set_root_cli: root={} context_id={context_id:?}",
        root.display()
    );
    let mut payload = serde_json::json!({
        "type": "set_context_root",
        "root": root,
    });
    if let Some(cid) = context_id {
        payload["context_id"] = serde_json::Value::from(cid);
    }
    let rc = send_to_socket(payload);
    if rc == 0 {
        print_tip(
            "you can also press \u{21E7}\u{2318}I to set the context root from the focused pane",
        );
        print_tip(
            "PLEXI_CONTEXT_ROOT is set when a pane opens — open a new pane to pick up the change",
        );
    }
    rc
}

/// `plexi context describe "text"`
///
/// Sets the description of the caller's context (PLEXI_CONTEXT_ID), falling
/// back to the active context.
pub fn context_describe_cli(text: &str) -> i32 {
    let context_id = caller_context_id();
    log::info!("context_describe_cli: context_id={context_id:?}");
    let mut payload = serde_json::json!({
        "type": "set_context_description",
        "description": text,
    });
    if let Some(cid) = context_id {
        payload["context_id"] = serde_json::Value::from(cid);
    }
    send_to_socket(payload)
}

/// `plexi context push [name] [--pane-id <id>]`
///
/// Push a pane into a new sub-context. The pane becomes a portal and its
/// content moves into the child context. Defaults to the calling pane
/// (PLEXI_PANE_ID), falling back to the focused pane.
pub fn context_push_cli(name: Option<&str>, pane_id: Option<u64>) -> i32 {
    let pane_id = pane_id.or_else(|| {
        std::env::var("PLEXI_PANE_ID")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
    });
    log::info!("context_push_cli: name={name:?} pane_id={pane_id:?}");
    let mut payload = serde_json::json!({ "type": "push_pane_to_subcontext" });
    if let Some(n) = name {
        payload["name"] = serde_json::Value::String(n.to_string());
    }
    if let Some(pid) = pane_id {
        payload["pane_id"] = serde_json::Value::from(pid);
    }
    send_to_socket(payload)
}

/// The caller's context id from PLEXI_CONTEXT_ID (set in every Plexi pane).
fn caller_context_id() -> Option<u64> {
    std::env::var("PLEXI_CONTEXT_ID")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
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
    let response_file = crate::rpc::response_file("context-list-response", "json");

    let payload = serde_json::json!({
        "type": "list_contexts",
        "response_file": response_file,
    });

    log::info!(
        "context_list:cli: sending via socket response_file={:?}",
        response_file
    );

    let code = send_to_socket(payload);
    if code != 0 {
        return code;
    }

    match super::poll_rpc(&response_file, "context list") {
        Ok(content) => print_json_output(&content),
        Err(code) => code,
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
                    if let Err(e) = stdin.write_all(json_str.as_bytes()) {
                        log::warn!(
                            "context_list:print_json_output: failed writing to jq stdin ({e})"
                        );
                    }
                }
                match child.wait() {
                    Ok(status) if status.success() => {
                        log::info!("context_list:print_json_output: rendered via jq");
                        return 0;
                    }
                    Ok(status) => {
                        log::warn!("context_list:print_json_output: jq exited non-zero ({status}), falling back to serde");
                    }
                    Err(e) => {
                        log::warn!("context_list:print_json_output: jq wait failed ({e}), falling back to serde");
                    }
                }
            }
            Err(e) => {
                log::warn!(
                    "context_list:print_json_output: jq spawn failed ({e}), falling back to serde"
                );
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
        Err(e) => {
            eprintln!("error: invalid JSON output: {e}");
            1
        }
    }
}

#[cfg(test)]
mod context_sub_tests {
    use super::expand_pane_commands;

    #[test]
    fn no_command_gives_every_pane_a_plain_shell() {
        assert_eq!(expand_pane_commands(3, &[]), Ok(vec![None, None, None]));
    }

    #[test]
    fn one_command_applies_to_every_pane() {
        assert_eq!(
            expand_pane_commands(3, &["cm".to_string()]),
            Ok(vec![
                Some("cm".to_string()),
                Some("cm".to_string()),
                Some("cm".to_string())
            ])
        );
    }

    #[test]
    fn n_commands_map_one_per_pane_in_order() {
        let cmds = ["a".to_string(), "b".to_string()];
        assert_eq!(
            expand_pane_commands(2, &cmds),
            Ok(vec![Some("a".to_string()), Some("b".to_string())])
        );
    }

    /// A count that is neither 1 nor N is an error, never a silent truncation
    /// or a cycle — a squad that quietly drops an agent's job is worse than one
    /// that refuses to start.
    #[test]
    fn mismatched_command_count_is_an_error() {
        let cmds = ["a".to_string(), "b".to_string()];
        let err = expand_pane_commands(3, &cmds).expect_err("2 commands for 3 agents must fail");
        assert!(err.contains("--command given 2 times"), "got: {err}");
        assert!(err.contains("--agents is 3"), "got: {err}");
    }
}

#[cfg(test)]
mod sub_response_tests {
    use super::sub_response_error;

    /// A host-reported failure must be detectable so the CLI can exit nonzero —
    /// `poll_rpc` succeeds here, because the transport worked.
    #[test]
    fn error_field_is_surfaced() {
        assert_eq!(
            sub_response_error(r#"{"error":"no parent context for id=Some(9)"}"#).as_deref(),
            Some("no parent context for id=Some(9)")
        );
    }

    #[test]
    fn successful_response_has_no_error() {
        assert_eq!(
            sub_response_error(r#"{"context_id":76,"windows":[],"panes":[317,318]}"#),
            None
        );
    }

    /// Unparseable output is not an error here: `poll_rpc` owns transport
    /// failures, and swallowing an unexpected-but-valid shape would hide the
    /// response from the caller.
    #[test]
    fn unparseable_output_is_not_treated_as_an_error() {
        assert_eq!(sub_response_error("not json at all"), None);
    }
}
