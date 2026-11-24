use super::reply::OutputMode;
use super::validate::resolve_path;
use super::{print_json_output, print_tip, send_to_socket};

#[derive(Default)]
pub(crate) struct ContextSpawnPlacement {
    pub focus: bool,
    pub from: Option<u64>,
}

pub fn context_new_cli(
    name: Option<&str>,
    path: Option<&str>,
    parent: Option<&str>,
    windows: &[String],
    direction: &str,
    placement: ContextSpawnPlacement,
    mode: OutputMode,
) -> i32 {
    let ContextSpawnPlacement { focus, from } = placement;
    let explicit_root = match resolve_path(path) {
        Ok(p) => Some(p),
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
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
    // Every creation waits for the host's authoritative name. Sub-context
    // callers get the structured response; top-level creation stays quiet on
    // success and prints only the collision note.
    let mode = if explicit_parent.is_some() {
        OutputMode::Json
    } else {
        mode
    };
    match super::request(payload, "context-new-response", "context-new", mode) {
        Ok(content) => {
            let response: serde_json::Value = serde_json::from_str(&content).unwrap_or_default();
            if explicit_parent.is_some() {
                println!("{content}");
            } else if let (Some(name), Some(existing)) = (
                response["name"].as_str(),
                response["auto_name_collision_with"].as_str(),
            ) {
                println!(
                    "note: context name '{name}' disambiguated from existing context '{existing}'"
                );
            }
        }
        Err(code) => return code,
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
    layout: Option<crate::protocol::SubContextLayout>,
    placement: ContextSpawnPlacement,
    mode: OutputMode,
) -> i32 {
    let ContextSpawnPlacement { focus, from } = placement;
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
    let mut payload = serde_json::json!({
        "type": "create_sub_context",
        "name": name,
        "root": root.to_string_lossy(),
        "panes": panes,
        "focus": focus,
    });
    if let Some(layout) = layout {
        payload["layout"] = serde_json::to_value(layout).unwrap_or_default();
    }
    if let Some(id) = parent_context_id {
        payload["parent_context_id"] = serde_json::Value::from(id);
    }
    if let Some(n) = parent_name {
        payload["parent_name"] = serde_json::Value::String(n);
    }
    if let Some(ap) = anchor_pane {
        payload["anchor_pane"] = serde_json::Value::from(ap);
    }
    match super::request(payload, "context-sub-response", "context-sub", mode) {
        Ok(content) => {
            println!("{content}");
            0
        }
        Err(code) => code,
    }
}

/// `plexi context zoom <context_id>`
///
/// Zoom into a sub-context by its numeric context_id. The host answers, so an
/// id naming no live context is an `error:` on stderr and exit 1 rather than a
/// silent success.
pub fn context_zoom_cli(context_id: u64, mode: OutputMode) -> i32 {
    log::info!("context_zoom_cli: context_id={context_id}");
    super::request(
        serde_json::json!({
            "type": "zoom_into_context",
            "context_id": context_id,
        }),
        "context-zoom-response",
        "context zoom",
        mode,
    )
    .map_or_else(|code| code, |_| 0)
}

/// `plexi context zoom-out`
///
/// Zoom out of the current sub-context to the parent.
pub fn context_zoom_out_cli() -> i32 {
    send_to_socket(serde_json::json!({
        "type": "zoom_out_of_context",
    }))
}

/// Mutations report host rejection, never transport-only success.
fn context_mutation(payload: serde_json::Value, label: &str, mode: OutputMode) -> i32 {
    match super::request(payload, "context-response", label, mode) {
        Ok(content) => {
            println!("{content}");
            0
        }
        Err(code) => code,
    }
}

/// `plexi context open [path]`
///
/// Focuses the context with matching root, or creates one. Uses CWD if path omitted.
pub fn context_open_cli(path: Option<&str>, mode: OutputMode) -> i32 {
    let root = match resolve_path(path) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    context_mutation(
        serde_json::json!({"type":"focus_context", "root":root}),
        "context open",
        mode,
    )
}

/// `plexi context set-root [path]`
///
/// Sets the root of the caller's own context. The frame names it from the
/// pane's environment; the host checks it against the calling pane (stint
/// 1285). Uses CWD if path omitted.
pub fn context_set_root_cli(path: Option<&str>, mode: OutputMode) -> i32 {
    let root = match resolve_path(path) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let payload = set_root_frame(&root, caller_context_id());
    log::info!(
        "context_set_root_cli: root={} frame={payload}",
        root.display()
    );
    let rc = context_mutation(payload, "context set-root", mode);
    if rc == 0 {
        print_tip(
            "you can also press \u{21E7}\u{2318}I to set the context root from the focused pane",
        );
    }
    rc
}

/// The `set_context_root` request for `root`. It names the caller's own
/// context: a host of this build checks it is the caller's, and an older
/// host, which reads a missing id as its active context, changes the right
/// one (Codex 3290 r4).
fn set_root_frame(root: &std::path::Path, context_id: Option<u64>) -> serde_json::Value {
    let mut payload = serde_json::json!({"type": "set_context_root", "root": root});
    if let Some(id) = context_id {
        payload["context_id"] = serde_json::Value::from(id);
    }
    payload
}

/// `plexi context describe "text"`
///
/// Sets the description of the caller's context (PLEXI_CONTEXT_ID), falling
/// back to the active context.
pub fn context_describe_cli(text: &str, mode: OutputMode) -> i32 {
    let context_id = caller_context_id();
    log::info!("context_describe_cli: context_id={context_id:?}");
    let mut payload = serde_json::json!({
        "type": "set_context_description",
        "description": text,
    });
    if let Some(cid) = context_id {
        payload["context_id"] = serde_json::Value::from(cid);
    }
    context_mutation(payload, "context describe", mode)
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
/// Prints the context ID, name and description for the calling pane as JSON.
/// The host answers with the context that currently owns the pane (the same
/// truth `pane info` reports). `PLEXI_CONTEXT_ID` is spawn-time only: a pane
/// moved by `context push` keeps its stale stamp, so it is never the answer.
pub fn context_current_cli(mode: OutputMode) -> i32 {
    let pane_id = match std::env::var("PLEXI_PANE_ID") {
        Ok(v) => match v.parse::<u64>() {
            Ok(n) => n,
            Err(_) => {
                eprintln!("error: PLEXI_PANE_ID is not a valid number: {v}");
                return 1;
            }
        },
        Err(_) => {
            eprintln!("error: PLEXI_PANE_ID is not set — run this inside a Plexi terminal pane");
            return 1;
        }
    };
    log::info!("context_current_cli: pane_id={pane_id}");
    let content = match super::request(
        serde_json::json!({ "type": "get_pane_info", "pane_id": pane_id }),
        "pane-info-response",
        "context current",
        mode,
    ) {
        Ok(content) => content,
        Err(code) => return code,
    };
    match render_current_context(&content) {
        Ok(out) => {
            println!("{out}");
            0
        }
        Err(msg) => {
            eprintln!("error: {msg}");
            1
        }
    }
}

/// Project a host `get_pane_info` reply onto the `context current` JSON shape.
fn render_current_context(reply: &str) -> Result<String, String> {
    let v: serde_json::Value =
        serde_json::from_str(reply).map_err(|e| format!("invalid JSON from host: {e}"))?;
    let context_id = v
        .get("context_id")
        .and_then(|c| c.as_u64())
        .ok_or_else(|| "host reply carried no context_id".to_string())?;
    let text = |key: &str| v.get(key).and_then(|x| x.as_str()).unwrap_or_default();
    serde_json::to_string_pretty(&serde_json::json!({
        "context_id": context_id,
        "context_name": text("context_name"),
        "context_description": text("context_description"),
    }))
    .map_err(|e| format!("failed to serialize context JSON: {e}"))
}

/// `plexi context list`
///
/// Sends a `list_contexts` command to PLEXI_SOCKET. The host writes a JSON array
/// to a response file; this function polls for it and prints it to stdout.
/// Returns 0 on success, 1 on error.
pub fn context_list_cli() -> i32 {
    log::info!("context_list:cli: sending via socket");

    match super::request(
        serde_json::json!({ "type": "list_contexts" }),
        "context-list-response",
        "context list",
        OutputMode::Json,
    ) {
        Ok(content) => print_json_output(&content),
        Err(code) => code,
    }
}

#[cfg(test)]
mod context_sub_tests {
    use super::expand_pane_commands;

    /// Codex 3290 r4 L3: the frame names the caller's context, so an older
    /// host (which reads a missing id as its active context) changes the
    /// caller's own root.
    #[test]
    fn set_root_names_the_callers_context() {
        let frame = super::set_root_frame(std::path::Path::new("/w"), Some(7));
        assert_eq!(frame["context_id"], 7, "{frame}");
        let request: crate::protocol::HostRequest = serde_json::from_value(frame).unwrap();
        assert!(matches!(
            request,
            crate::protocol::HostRequest::SetContextRoot { context_id: Some(7), .. }
        ));
    }

    /// Codex 3290 r4 L3: a fire-and-forget verb whose caller does not matter
    /// does not wait for an identity ack an older host never sends.
    #[test]
    fn pane_focus_does_not_wait_for_an_identity_ack() {
        let (mut client, _host) = std::os::unix::net::UnixStream::pair().unwrap();
        let started = std::time::Instant::now();
        crate::cli::await_peer_identity_ack(&mut client, r#"{"type":"focus_pane","pane_id":3}"#);
        assert!(started.elapsed() < std::time::Duration::from_millis(100));
        let started = std::time::Instant::now();
        crate::cli::await_peer_identity_ack(&mut client, r#"{"type":"set_agent_state","pane_id":3}"#);
        assert!(
            started.elapsed() >= std::time::Duration::from_millis(400),
            "an identity-consuming verb waits for the ack"
        );
    }

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
mod context_current_tests {
    use super::render_current_context;

    #[test]
    fn projects_the_host_context_not_the_env_stamp() {
        let reply = r#"{"id":5,"type":"terminal","context_id":6,"context_name":"child","context_description":"d","window_id":9}"#;
        let out: serde_json::Value =
            serde_json::from_str(&render_current_context(reply).expect("ok")).expect("json");
        assert_eq!(out["context_id"], 6);
        assert_eq!(out["context_name"], "child");
        assert_eq!(out["context_description"], "d");
    }

    #[test]
    fn reply_without_context_id_is_an_error() {
        assert!(render_current_context(r#"{"id":5}"#).is_err());
    }
}
