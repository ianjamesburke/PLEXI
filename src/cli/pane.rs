use super::{print_json_output, print_tip, send_to_socket};

pub fn pane_set_title_cli(pane_id: Option<u64>, name: &str) -> i32 {
    let resolved_pane_id = match pane_id {
        Some(id) => id,
        None => {
            let pane_id_str = match std::env::var("PLEXI_PANE_ID") {
                Ok(v) => v,
                Err(_) => {
                    eprintln!(
                        "error: PLEXI_PANE_ID is not set — run this inside a Plexi terminal pane or provide a pane ID"
                    );
                    return 1;
                }
            };
            match pane_id_str.parse::<u64>() {
                Ok(n) => n,
                Err(_) => {
                    eprintln!("error: PLEXI_PANE_ID is not a valid number: {pane_id_str}");
                    return 1;
                }
            }
        }
    };
    log::info!("pane_set_title:cli: pane_id={resolved_pane_id} name_chars={}", name.chars().count());
    send_to_socket(serde_json::json!({
        "type": "set_pane_title",
        "pane_id": resolved_pane_id,
        "name": name,
    }))
}

fn resolve_pane_id(pane_id: Option<u64>) -> Result<u64, i32> {
    match pane_id {
        Some(id) => Ok(id),
        None => {
            let raw = match std::env::var("PLEXI_PANE_ID") {
                Ok(v) => v,
                Err(_) => {
                    eprintln!(
                        "error: pane_id not provided and PLEXI_PANE_ID is not set — run inside a Plexi pane or pass a pane ID"
                    );
                    return Err(1);
                }
            };
            match raw.parse::<u64>() {
                Ok(id) => Ok(id),
                Err(_) => {
                    eprintln!("error: PLEXI_PANE_ID is not a valid number: {raw}");
                    Err(1)
                }
            }
        }
    }
}

fn wait_for_response_bytes(response_file: &str, label: &str) -> Result<Vec<u8>, i32> {
    match crate::rpc::poll_bytes(response_file, Some(crate::rpc::DEFAULT_TIMEOUT)) {
        Ok(content) => Ok(content),
        Err(crate::rpc::PollError::TimedOut) => {
            eprintln!("error: timed out waiting for {label} response");
            Err(1)
        }
        Err(e) => {
            log::warn!("{label}:cli: {e}");
            eprintln!("error: {e}");
            Err(1)
        }
    }
}

fn wait_for_slot_read_response(response_file: &str) -> Result<Vec<u8>, i32> {
    match crate::rpc::poll_slot_reply(response_file, Some(crate::rpc::DEFAULT_TIMEOUT)) {
        Ok(crate::rpc::SlotReply::Data(content)) => Ok(content),
        Ok(crate::rpc::SlotReply::Err(content)) => {
            if let Some(err) = response_error(&content) {
                eprintln!("error: {err}");
            } else {
                eprintln!("error: invalid slot read error response");
            }
            Err(1)
        }
        Err(crate::rpc::PollError::TimedOut) => {
            eprintln!("error: timed out waiting for pane slot read response");
            Err(1)
        }
        Err(e) => {
            log::warn!("pane_slot_read:cli: {e}");
            eprintln!("error: {e}");
            Err(1)
        }
    }
}

fn response_error(content: &[u8]) -> Option<String> {
    let value = serde_json::from_slice::<serde_json::Value>(content).ok()?;
    if value.get("ok").and_then(|v| v.as_bool()) != Some(false) {
        return None;
    }
    value
        .get("error")
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

/// Terse success ack for `pane slot write`, printed to stdout with other CLI payloads.
fn slot_write_ack(name: &str, bytes: usize, append: bool) -> String {
    let verb = if append { "+<-" } else { "<-" };
    format!("slot {name:?} {verb} {bytes} bytes")
}

/// `plexi pane slot write <name> [content] [pane_id]`
pub fn pane_slot_write_cli(
    name: &str,
    content: Option<&str>,
    append: bool,
    replace: bool,
    pane_id: Option<u64>,
) -> i32 {
    if name.trim().is_empty() {
        eprintln!("error: slot name empty");
        return 1;
    }
    let resolved_pane_id = match resolve_pane_id(pane_id) {
        Ok(id) => id,
        Err(code) => return code,
    };
    let mut bytes = Vec::new();
    if let Some(content) = content {
        bytes.extend_from_slice(content.as_bytes());
    } else {
        use std::io::Read as _;
        if let Err(e) = std::io::stdin().read_to_end(&mut bytes) {
            eprintln!("error: could not read stdin: {e}");
            return 1;
        }
    }
    log::info!(
        "pane_slot_write:cli: pane_id={resolved_pane_id} slot={name:?} bytes={} append={append} replace={replace}",
        bytes.len()
    );
    let response_file = match super::send_request(
        serde_json::json!({
            "type": "slot_write",
            "pane_id": resolved_pane_id,
            "slot_name": name,
            "content": bytes,
            "append": append,
            "replace": replace,
        }),
        "pane-slot-write-response",
        "pane slot write",
    ) {
        Ok(response_file) => response_file,
        Err(code) => return code,
    };
    let content = match wait_for_response_bytes(&response_file, "pane slot write") {
        Ok(content) => content,
        Err(code) => return code,
    };
    if let Some(err) = response_error(&content) {
        eprintln!("error: {err}");
        return 1;
    }
    println!("{}", slot_write_ack(name, bytes.len(), append));
    0
}

/// `plexi pane slot read <name> [pane_id]`
pub fn pane_slot_read_cli(name: &str, pane_id: Option<u64>) -> i32 {
    let resolved_pane_id = match resolve_pane_id(pane_id) {
        Ok(id) => id,
        Err(code) => return code,
    };
    log::info!("pane_slot_read:cli: pane_id={resolved_pane_id} slot={name:?}");
    let response_file = match super::send_request(
        serde_json::json!({
            "type": "slot_read",
            "pane_id": resolved_pane_id,
            "slot_name": name,
        }),
        "pane-slot-read-response",
        "pane slot read",
    ) {
        Ok(response_file) => response_file,
        Err(code) => return code,
    };
    let content = match wait_for_slot_read_response(&response_file) {
        Ok(content) => content,
        Err(code) => return code,
    };
    use std::io::Write as _;
    if let Err(e) = std::io::stdout().write_all(&content) {
        eprintln!("error: could not write stdout: {e}");
        return 1;
    }
    0
}

/// Margin the CLI adds to the caller's `--timeout` before giving up on the
/// response file. The host expires the wait at exactly `--timeout` and writes
/// a typed reply, so this window only has to outlast that write — the caller
/// then branches on the host's reason, never on a client-side stall.
const SLOT_WAIT_REPLY_MARGIN: std::time::Duration = std::time::Duration::from_secs(5);

/// What a `pane slot wait` reply means to the caller. The three variants are
/// the command's three exit codes; nothing else may reach the shell.
#[derive(Debug, PartialEq)]
enum SlotWaitOutcome {
    /// Slot matched: these raw bytes go to stdout, exit 0.
    Matched(Vec<u8>),
    /// The host expired the wait: exit 2 with an empty stdout, so a caller
    /// can branch on the exit code alone.
    TimedOut(String),
    /// Usage or plumbing failure: exit 1.
    Failed(String),
}

impl SlotWaitOutcome {
    fn exit_code(&self) -> i32 {
        match self {
            SlotWaitOutcome::Matched(_) => 0,
            SlotWaitOutcome::Failed(_) => 1,
            SlotWaitOutcome::TimedOut(_) => 2,
        }
    }
}

/// Map a host reply to the caller's outcome. Pure so the exit-code contract is
/// testable without a host or a socket.
fn slot_wait_outcome(
    reply: Result<crate::rpc::SlotReply, crate::rpc::PollError>,
) -> SlotWaitOutcome {
    match reply {
        Ok(crate::rpc::SlotReply::Data(content)) => SlotWaitOutcome::Matched(content),
        Ok(crate::rpc::SlotReply::Err(content)) => {
            let Ok(value) = serde_json::from_slice::<serde_json::Value>(&content) else {
                return SlotWaitOutcome::Failed("invalid slot wait error response".to_string());
            };
            let message = value
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("slot wait failed")
                .to_string();
            if value.get("timeout").and_then(|v| v.as_bool()) == Some(true) {
                SlotWaitOutcome::TimedOut(message)
            } else {
                SlotWaitOutcome::Failed(message)
            }
        }
        // The host answers every wait within its own deadline, so reaching
        // the client deadline means the request never landed — a plumbing
        // failure, not the caller's condition failing to hold.
        Err(crate::rpc::PollError::TimedOut) => SlotWaitOutcome::Failed(
            "no response from host within the slot wait window — the host may not be running"
                .to_string(),
        ),
        Err(e) => SlotWaitOutcome::Failed(e.to_string()),
    }
}

/// `plexi pane slot wait <name> [pane_id] --until <PATTERN> [--timeout <SECS>]`
pub fn pane_slot_wait_cli(name: &str, pane_id: Option<u64>, until: &str, timeout: f64) -> i32 {
    if !timeout.is_finite()
        || !(0.0..=crate::app::pane_wait::MAX_WAIT_TIMEOUT_SECS).contains(&timeout)
    {
        eprintln!(
            "error: --timeout must be a non-negative number of seconds at most {}, got {timeout}",
            crate::app::pane_wait::MAX_WAIT_TIMEOUT_SECS
        );
        return 1;
    }
    let resolved_pane_id = match resolve_pane_id(pane_id) {
        Ok(id) => id,
        Err(code) => return code,
    };
    log::info!(
        "pane_slot_wait:cli: pane_id={resolved_pane_id} slot={name:?} until={until:?} timeout={timeout}"
    );
    let response_file = match super::send_request(
        serde_json::json!({
            "type": "slot_wait",
            "pane_id": resolved_pane_id,
            "slot_name": name,
            "pattern": until,
            "timeout_secs": timeout,
        }),
        "pane-slot-wait-response",
        "pane slot wait",
    ) {
        Ok(response_file) => response_file,
        Err(code) => return code,
    };
    let deadline = std::time::Duration::from_secs_f64(timeout) + SLOT_WAIT_REPLY_MARGIN;
    let outcome = slot_wait_outcome(crate::rpc::poll_slot_reply(&response_file, Some(deadline)));
    match &outcome {
        SlotWaitOutcome::Matched(content) => {
            use std::io::Write as _;
            if let Err(e) = std::io::stdout().write_all(content) {
                eprintln!("error: could not write stdout: {e}");
                return 1;
            }
        }
        SlotWaitOutcome::TimedOut(message) | SlotWaitOutcome::Failed(message) => {
            eprintln!("error: {message}");
        }
    }
    outcome.exit_code()
}

/// `plexi pane slot list [pane_id]`
pub fn pane_slot_list_cli(pane_id: Option<u64>) -> i32 {
    let resolved_pane_id = match resolve_pane_id(pane_id) {
        Ok(id) => id,
        Err(code) => return code,
    };
    log::info!("pane_slot_list:cli: pane_id={resolved_pane_id}");
    let response_file = match super::send_request(
        serde_json::json!({
            "type": "slot_list",
            "pane_id": resolved_pane_id,
        }),
        "pane-slot-list-response",
        "pane slot list",
    ) {
        Ok(response_file) => response_file,
        Err(code) => return code,
    };
    let content = match wait_for_response_bytes(&response_file, "pane slot list") {
        Ok(content) => content,
        Err(code) => return code,
    };
    if let Some(err) = response_error(&content) {
        eprintln!("error: {err}");
        return 1;
    }
    print_json_output(&String::from_utf8_lossy(&content))
}

/// `plexi pane slot delete <name> [pane_id]`
pub fn pane_slot_delete_cli(name: &str, pane_id: Option<u64>) -> i32 {
    let resolved_pane_id = match resolve_pane_id(pane_id) {
        Ok(id) => id,
        Err(code) => return code,
    };
    log::info!("pane_slot_delete:cli: pane_id={resolved_pane_id} slot={name:?}");
    let response_file = match super::send_request(
        serde_json::json!({
            "type": "slot_delete",
            "pane_id": resolved_pane_id,
            "slot_name": name,
        }),
        "pane-slot-delete-response",
        "pane slot delete",
    ) {
        Ok(response_file) => response_file,
        Err(code) => return code,
    };
    let content = match wait_for_response_bytes(&response_file, "pane slot delete") {
        Ok(content) => content,
        Err(code) => return code,
    };
    if let Some(err) = response_error(&content) {
        eprintln!("error: {err}");
        return 1;
    }
    0
}

/// `plexi pane message send <pane_id> <payload> [--submit]`
pub fn pane_message_send_cli(pane_id: u64, payload: &str, submit: bool) -> i32 {
    log::info!(
        "pane_message_send:cli: pane_id={pane_id} bytes={} submit={submit}",
        payload.len()
    );
    match super::request(
        serde_json::json!({
            "type": "pane_message_send",
            "pane_id": pane_id,
            "payload": payload,
            "submit": submit,
        }),
        "pane-message-send-response",
        "pane message send",
    ) {
        Ok(content) => match super::check_reply_error(&content) {
            Ok(()) => print_json_output(&content),
            Err(code) => code,
        },
        Err(code) => code,
    }
}

/// `plexi pane message recv [pane_id] [--wait] [--timeout <SECS>]`
pub fn pane_message_recv_cli(pane_id: Option<u64>, wait: bool, timeout: f64) -> i32 {
    if !timeout.is_finite()
        || !(0.0..=crate::app::pane_wait::MAX_WAIT_TIMEOUT_SECS).contains(&timeout)
    {
        eprintln!(
            "error: --timeout must be a non-negative number of seconds at most {}, got {timeout}",
            crate::app::pane_wait::MAX_WAIT_TIMEOUT_SECS
        );
        return 1;
    }
    let resolved_pane_id = match resolve_pane_id(pane_id) {
        Ok(id) => id,
        Err(code) => return code,
    };
    log::info!(
        "pane_message_recv:cli: pane_id={resolved_pane_id} wait={wait} timeout={timeout}"
    );
    let response_file = match super::send_request(
        serde_json::json!({
            "type": "pane_message_recv",
            "pane_id": resolved_pane_id,
            "wait": wait,
            "timeout_secs": timeout,
        }),
        "pane-message-recv-response",
        "pane message recv",
    ) {
        Ok(response_file) => response_file,
        Err(code) => return code,
    };
    let deadline = if wait {
        std::time::Duration::from_secs_f64(timeout) + SLOT_WAIT_REPLY_MARGIN
    } else {
        crate::rpc::DEFAULT_TIMEOUT
    };
    let outcome = slot_wait_outcome(crate::rpc::poll_slot_reply(&response_file, Some(deadline)));
    match &outcome {
        SlotWaitOutcome::Matched(content) => {
            return print_json_output(&String::from_utf8_lossy(content));
        }
        SlotWaitOutcome::TimedOut(message) | SlotWaitOutcome::Failed(message) => {
            eprintln!("error: {message}");
        }
    }
    outcome.exit_code()
}

/// Format an error message with its typed `error_code` prefix (if not already present).
fn format_coded_error(error_code: Option<&str>, message: &str) -> String {
    match error_code {
        Some(code) if !code.is_empty() && !message.starts_with(code) => {
            format!("{code}: {message}")
        }
        _ => message.to_string(),
    }
}

/// Outcome of `plexi pane result <id>` / `plexi pane result read <id>`.
///
/// Distinguishes:
/// - `Validated`: committed JSON object (exit 0, printed to stdout)
/// - `NoResultYet`: no result written yet or `--wait` timed out (exit 2, empty stdout)
/// - `InvalidResult`: last write was rejected as malformed/schema-violating and not yet corrected (exit 1, empty stdout)
/// - `Failed`: plumbing or usage error (exit 1, empty stdout)
#[derive(Debug, PartialEq)]
pub enum PaneResultReadOutcome {
    Validated(Vec<u8>),
    NoResultYet {
        error_code: String,
        message: String,
    },
    InvalidResult {
        error_code: String,
        message: String,
    },
    Failed(String),
}

impl PaneResultReadOutcome {
    pub fn exit_code(&self) -> i32 {
        match self {
            PaneResultReadOutcome::Validated(_) => 0,
            PaneResultReadOutcome::NoResultYet { .. } => 2,
            PaneResultReadOutcome::InvalidResult { .. } | PaneResultReadOutcome::Failed(_) => 1,
        }
    }
}

/// Pure mapper for `pane_result_read` replies.
pub fn pane_result_read_outcome(
    reply: Result<crate::rpc::SlotReply, crate::rpc::PollError>,
) -> PaneResultReadOutcome {
    match reply {
        Ok(crate::rpc::SlotReply::Data(content)) => PaneResultReadOutcome::Validated(content),
        Ok(crate::rpc::SlotReply::Err(content)) => {
            let Ok(value) = serde_json::from_slice::<serde_json::Value>(&content) else {
                return PaneResultReadOutcome::Failed(
                    "invalid pane result error response".to_string(),
                );
            };
            let error_code = value
                .get("error_code")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            let raw_msg = value
                .get("error")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("pane result read failed");
            let timed_out = value
                .get("timeout")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let formatted = format_coded_error(
                (!error_code.is_empty()).then_some(error_code),
                raw_msg,
            );
            if error_code == "no_result_yet" || error_code == "timeout" || timed_out {
                let code = if !error_code.is_empty() {
                    error_code.to_string()
                } else {
                    "timeout".to_string()
                };
                PaneResultReadOutcome::NoResultYet {
                    error_code: code,
                    message: formatted,
                }
            } else if error_code == "invalid_result" {
                PaneResultReadOutcome::InvalidResult {
                    error_code: "invalid_result".to_string(),
                    message: formatted,
                }
            } else {
                PaneResultReadOutcome::Failed(formatted)
            }
        }
        Err(crate::rpc::PollError::TimedOut) => PaneResultReadOutcome::Failed(
            "no response from host within the pane result wait window — the host may not be running"
                .to_string(),
        ),
        Err(e) => PaneResultReadOutcome::Failed(e.to_string()),
    }
}

/// `plexi pane result [pane_id] [--wait] [--timeout <SECS>]`
pub fn pane_result_read_cli(pane_id: Option<u64>, wait: bool, timeout: f64) -> i32 {
    if !timeout.is_finite()
        || !(0.0..=crate::app::pane_wait::MAX_WAIT_TIMEOUT_SECS).contains(&timeout)
    {
        eprintln!(
            "error: --timeout must be a non-negative number of seconds at most {}, got {timeout}",
            crate::app::pane_wait::MAX_WAIT_TIMEOUT_SECS
        );
        return 1;
    }
    let resolved_pane_id = match resolve_pane_id(pane_id) {
        Ok(id) => id,
        Err(code) => return code,
    };
    log::info!(
        "pane_result_read:cli: pane_id={resolved_pane_id} wait={wait} timeout={timeout}"
    );
    let response_file = match super::send_request(
        serde_json::json!({
            "type": "pane_result_read",
            "pane_id": resolved_pane_id,
            "wait": wait,
            "timeout_secs": timeout,
        }),
        "pane-result-read-response",
        "pane result",
    ) {
        Ok(response_file) => response_file,
        Err(code) => return code,
    };
    let deadline = if wait {
        std::time::Duration::from_secs_f64(timeout) + SLOT_WAIT_REPLY_MARGIN
    } else {
        crate::rpc::DEFAULT_TIMEOUT
    };
    let outcome =
        pane_result_read_outcome(crate::rpc::poll_slot_reply(&response_file, Some(deadline)));
    match &outcome {
        PaneResultReadOutcome::Validated(content) => {
            return print_json_output(&String::from_utf8_lossy(content));
        }
        PaneResultReadOutcome::NoResultYet { message, .. }
        | PaneResultReadOutcome::InvalidResult { message, .. }
        | PaneResultReadOutcome::Failed(message) => {
            eprintln!("error: {message}");
        }
    }
    outcome.exit_code()
}

/// `plexi pane list`
///
/// Sends a `list_panes` command to PLEXI_SOCKET. The host writes a JSON array
/// to a response file; this function polls for it and prints it to stdout.
/// Returns 0 on success, 1 on error.
pub fn pane_layout_cli() -> i32 {
    log::info!("pane_layout:cli: requesting workspace snapshot");
    match super::request(serde_json::json!({"type":"workspace_snapshot"}), "pane-layout-response", "pane layout") {
        Ok(content) => match super::check_reply_error(&content) {
            Ok(()) => print_json_output(&content),
            Err(code) => code,
        },
        Err(code) => code,
    }
}

pub fn pane_list_cli(context: Option<u64>, current: bool) -> i32 {
    let context_id: Option<u64> = if current {
        let raw = match std::env::var("PLEXI_CONTEXT_ID") {
            Ok(v) => v,
            Err(_) => {
                eprintln!(
                    "error: PLEXI_CONTEXT_ID is not set — run this inside a Plexi terminal pane"
                );
                return 1;
            }
        };
        match raw.parse::<u64>() {
            Ok(n) => Some(n),
            Err(_) => {
                eprintln!("error: PLEXI_CONTEXT_ID is not a valid number: {raw}");
                return 1;
            }
        }
    } else {
        context
    };

    let mut payload = serde_json::json!({
        "type": "list_panes",
    });
    if let Some(cid) = context_id {
        payload["context_id"] = serde_json::json!(cid);
    }

    log::info!("pane_list:cli: sending via socket context_id={context_id:?}");

    match super::request(payload, "pane-list-response", "pane list") {
        Ok(content) => print_json_output(&content),
        Err(code) => code,
    }
}

/// Format a flat `list_panes` JSON array as a `pstree` / `ps --forest` parent-child session
/// provenance tree (or nested JSON forest when `json == true`).
pub fn format_pane_tree(panes: &[serde_json::Value], json: bool) -> String {
    let known_ids: std::collections::HashSet<u64> = panes
        .iter()
        .filter_map(|p| p.get("id").and_then(|v| v.as_u64()))
        .collect();
    let mut children_by_parent: std::collections::BTreeMap<u64, Vec<&serde_json::Value>> =
        std::collections::BTreeMap::new();
    let mut roots: Vec<&serde_json::Value> = Vec::new();
    for p in panes {
        let parent_id = p
            .get("parent_pane_id")
            .and_then(|v| v.as_u64())
            .filter(|pid| known_ids.contains(pid));
        if let Some(pid) = parent_id {
            children_by_parent.entry(pid).or_default().push(p);
        } else {
            roots.push(p);
        }
    }

    if json {
        fn build_node(
            pane: &serde_json::Value,
            children_by_parent: &std::collections::BTreeMap<u64, Vec<&serde_json::Value>>,
        ) -> serde_json::Value {
            let id = pane.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
            let kids = children_by_parent
                .get(&id)
                .map(|list| {
                    list.iter()
                        .map(|c| build_node(c, children_by_parent))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let mut obj = pane.clone();
            if let Some(map) = obj.as_object_mut() {
                map.insert("children".to_string(), serde_json::Value::Array(kids));
            }
            obj
        }
        let forest: Vec<serde_json::Value> = roots
            .iter()
            .map(|r| build_node(r, &children_by_parent))
            .collect();
        return serde_json::to_string_pretty(&forest).unwrap_or_else(|_| "[]".to_string());
    }

    fn format_node_line(pane: &serde_json::Value) -> String {
        let id = pane.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
        let name = pane
            .get("name")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .or_else(|| pane.get("title").and_then(|v| v.as_str()).filter(|s| !s.is_empty()))
            .unwrap_or("pane");
        let role = pane
            .get("agent_role")
            .and_then(|v| v.as_str())
            .or_else(|| pane.get("agent").and_then(|v| v.as_str()));
        let state = pane.get("agent_state").and_then(|v| v.as_str());
        let spawned_by = pane
            .get("spawned_by")
            .and_then(|v| v.as_str())
            .unwrap_or("human");
        let waiting = pane
            .get("pending_child_results_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let role_part = match (role, state) {
            (Some(r), Some(s)) => format!("agent:{r} [{s}] "),
            (Some(r), None) => format!("agent:{r} "),
            (None, Some(s)) => format!("[{s}] "),
            (None, None) => String::new(),
        };
        let waiting_part = if waiting == 1 {
            " · 1 result waiting".to_string()
        } else if waiting > 1 {
            format!(" · {waiting} results waiting")
        } else {
            String::new()
        };
        format!("#{id} {role_part}\"{name}\" (spawned by {spawned_by}){waiting_part}")
    }

    fn walk(
        pane: &serde_json::Value,
        prefix: &str,
        branch: &str,
        children_by_parent: &std::collections::BTreeMap<u64, Vec<&serde_json::Value>>,
        lines: &mut Vec<String>,
    ) {
        let id = pane.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
        lines.push(format!("{prefix}{branch}{}", format_node_line(pane)));
        if let Some(kids) = children_by_parent.get(&id) {
            let child_prefix = if branch.is_empty() {
                String::new()
            } else if branch == "├── " {
                format!("{prefix}│   ")
            } else {
                format!("{prefix}    ")
            };
            for (idx, child) in kids.iter().enumerate() {
                let is_last = idx + 1 == kids.len();
                let child_branch = if is_last { "└── " } else { "├── " };
                walk(child, &child_prefix, child_branch, children_by_parent, lines);
            }
        }
    }

    let mut lines = Vec::new();
    for root in roots {
        walk(root, "", "", &children_by_parent, &mut lines);
    }
    lines.join("\n")
}

/// `plexi pane tree [--context [ID]] [--json]`
pub fn pane_tree_cli(context: Option<u64>, current: bool, json: bool) -> i32 {
    let context_id: Option<u64> = if current {
        let raw = match std::env::var("PLEXI_CONTEXT_ID") {
            Ok(v) => v,
            Err(_) => {
                eprintln!(
                    "error: PLEXI_CONTEXT_ID is not set — run this inside a Plexi terminal pane"
                );
                return 1;
            }
        };
        match raw.parse::<u64>() {
            Ok(n) => Some(n),
            Err(_) => {
                eprintln!("error: PLEXI_CONTEXT_ID is not a valid number: {raw}");
                return 1;
            }
        }
    } else {
        context
    };

    let mut payload = serde_json::json!({
        "type": "list_panes",
    });
    if let Some(cid) = context_id {
        payload["context_id"] = serde_json::json!(cid);
    }

    let content = match super::request(payload, "pane-tree-response", "pane tree") {
        Ok(c) => c,
        Err(code) => return code,
    };
    let panes: Vec<serde_json::Value> = match serde_json::from_str(&content) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: invalid pane list response: {e}");
            return 1;
        }
    };
    println!("{}", format_pane_tree(&panes, json));
    0
}

/// `plexi pane self`
///
/// Prints the current pane ID (just the number) to stdout. Reads PLEXI_PANE_ID.
/// No JSON, no socket round-trip — pure env-var lookup for agent callers.
pub fn pane_self_cli() -> i32 {
    let pane_id_str = match std::env::var("PLEXI_PANE_ID") {
        Ok(v) => v,
        Err(_) => {
            eprintln!("error: PLEXI_PANE_ID is not set — run this inside a Plexi terminal pane");
            return 1;
        }
    };
    match pane_id_str.parse::<u64>() {
        Ok(id) => {
            log::info!("pane_self:cli: pane_id={id}");
            println!("{id}");
            0
        }
        Err(_) => {
            eprintln!("error: PLEXI_PANE_ID is not a valid number: {pane_id_str}");
            1
        }
    }
}

/// `plexi pane info [--previous [N]]`
///
/// Sends a `get_pane_info` or `get_previous_pane_info` command to PLEXI_SOCKET.
/// When `steps` is `Some(n)`, walks back N steps in focus history.
/// Merges in client-side fields (socket, channel) and pretty-prints the result
/// as JSON. Returns 0 on success, 1 on error.
pub fn pane_info_cli(pane_id: Option<u64>, previous: Option<u64>) -> i32 {
    let socket_path = match super::resolve_command_socket() {
        Some(path) => path,
        None => {
            eprintln!("error: PLEXI_SOCKET is not set — run this inside a Plexi terminal pane");
            return 1;
        }
    };

    let payload = if let Some(steps) = previous {
        log::info!("pane_info:cli: previous steps={steps}");
        serde_json::json!({
            "type": "get_previous_pane_info",
            "steps": steps,
        })
    } else {
        let pane_id: u64 = match pane_id {
            Some(id) => id,
            None => {
                let pane_id_str = match std::env::var("PLEXI_PANE_ID") {
                    Ok(v) => v,
                    Err(_) => {
                        eprintln!(
                            "error: PLEXI_PANE_ID is not set — run this inside a Plexi terminal pane, or pass a pane id"
                        );
                        return 1;
                    }
                };
                match pane_id_str.parse() {
                    Ok(n) => n,
                    Err(_) => {
                        eprintln!("error: PLEXI_PANE_ID is not a valid number: {pane_id_str}");
                        return 1;
                    }
                }
            }
        };
        log::info!("pane_info:cli: pane_id={pane_id}");
        serde_json::json!({
            "type": "get_pane_info",
            "pane_id": pane_id,
        })
    };

    let content = match super::request(payload, "pane-info-response", "pane info") {
        Ok(content) => content,
        Err(code) => return code,
    };
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
        if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
            eprintln!("error: {err}");
            return 1;
        }
        let mut obj = v;
        obj["socket"] = serde_json::Value::String(socket_path.to_string_lossy().into_owned());
        let channel = crate::config::build_channel().unwrap_or_else(|| "main".to_string());
        obj["channel"] = serde_json::Value::String(channel);
        match serde_json::to_string(&obj) {
            Ok(json_str) => print_json_output(&json_str),
            Err(e) => {
                eprintln!("error: could not serialize response: {e}");
                1
            }
        }
    } else {
        eprintln!("error: invalid JSON from host: {content}");
        1
    }
}

/// `plexi pane focus <pane_id>`
///
/// Sends a `focus_pane` command to PLEXI_SOCKET. Fire-and-forget.
/// Returns 0 on success, 1 on error.
pub fn pane_focus_cli(pane_id: u64) -> i32 {
    send_to_socket(serde_json::json!({
        "type": "focus_pane",
        "pane_id": pane_id,
    }))
}

/// Map a host `close_pane` JSON reply onto `(stdout_line, stderr_line, exit_code)`.
///
/// When the closed pane owned a `--worktree` checkout that was dirty or ahead,
/// the host retains the worktree and includes `"worktree": {"action": "retained", "path": "...", "reason": "..."}`
/// in the reply; the CLI surfaces the retained path on stdout.
pub fn close_pane_outcome(response: &str) -> (Option<String>, Option<String>, i32) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(response) else {
        return (
            None,
            Some(format!("error: invalid JSON from host: {response}")),
            1,
        );
    };
    if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
        return (None, Some(format!("error: {err}")), 1);
    }
    if let Some(wt) = v.get("worktree") {
        if wt.get("action").and_then(|a| a.as_str()) == Some("retained") {
            let path = wt.get("path").and_then(|p| p.as_str()).unwrap_or("");
            let reason = wt
                .get("reason")
                .and_then(|r| r.as_str())
                .unwrap_or("dirty_or_ahead");
            return (
                Some(format!("retained worktree: {path} ({reason})")),
                None,
                0,
            );
        }
    }
    (None, None, 0)
}

/// `plexi pane close <pane_id>`
///
/// Sends a `close_pane` command to PLEXI_SOCKET. When closing another pane,
/// waits for the host's response so any retained `--worktree` path is reported
/// on stdout. When a pane closes itself (`PLEXI_PANE_ID == pane_id`), sends
/// fire-and-forget so the CLI exits cleanly before the PTY receives SIGHUP
/// (the host still reclaims or retains the worktree on close).
pub fn pane_close_cli(pane_id: u64) -> i32 {
    let self_pane = std::env::var("PLEXI_PANE_ID")
        .ok()
        .and_then(|s| s.parse::<u64>().ok());
    if self_pane == Some(pane_id) {
        return send_to_socket(serde_json::json!({
            "type": "close_pane",
            "pane_id": pane_id,
        }));
    }
    let payload = serde_json::json!({
        "type": "close_pane",
        "pane_id": pane_id,
    });
    let content = match super::request(payload, "close-pane-response", "pane close") {
        Ok(content) => content,
        Err(code) => return code,
    };
    let (stdout, stderr, code) = close_pane_outcome(&content);
    if let Some(err) = stderr {
        eprintln!("{err}");
    }
    if let Some(out) = stdout {
        println!("{out}");
    }
    code
}

/// `plexi pane send <pane_id> <text>`
///
/// Writes text to a terminal pane's PTY stdin, or focuses an app pane and
/// injects one egui text-input event through the production render path.
/// Polls a response file to surface errors (e.g. pane not found) to the caller.
/// Returns 0 on success, 1 on error.
pub fn pane_send_cli(pane_id: u64, text: &str, submit: bool, paste: bool) -> i32 {
    log::info!(
        "pane_send:cli: pane_id={pane_id} len={} submit={submit} paste={paste}",
        text.len()
    );
    let payload = serde_json::json!({
        "type": "send_to_pane",
        "pane_id": pane_id,
        "text": text,
        "submit": submit,
        "paste": paste,
        "peer_ancestry": [],
    });
    if submit {
        // The host settles, presses Enter and confirms before answering, all
        // within its own ceiling. This window sits above that so the caller
        // sees the host's typed outcome rather than a client-side timeout.
        let content = match super::request_with(
            payload,
            "send-to-pane-response",
            "pane send",
            SUBMIT_REPLY_WINDOW,
        ) {
            Ok(content) => content,
            Err(code) => return code,
        };
        let outcome = submit_outcome(&content);
        outcome.report();
        return outcome.exit_code();
    }
    let content = match super::request_with(
        payload,
        "send-to-pane-response",
        "pane send",
        SUBMIT_REPLY_WINDOW,
    ) {
        Ok(content) => content,
        Err(code) => return code,
    };
    if let Err(code) = super::check_reply_error(&content) {
        return code;
    }
    0
}

pub fn pane_heartbeat_cli(
    pane_id: u64,
    every: Option<&str>,
    text: Option<&str>,
    while_idle_only: Option<bool>,
    off: bool,
) -> i32 {
    let every_ms = match every.map(parse_heartbeat_duration) {
        Some(Ok(duration)) => Some(duration.as_millis() as u64),
        Some(Err(error)) => {
            eprintln!("error: invalid --every: {error}");
            return 1;
        }
        None => None,
    };
    match super::request(
        serde_json::json!({"type":"pane_heartbeat", "pane_id":pane_id, "every_ms":every_ms, "text":text, "while_idle_only":while_idle_only, "off":off}),
        "pane-heartbeat-response",
        "pane heartbeat",
    ) {
        Ok(content) => match super::check_reply_error(&content) {
            Ok(()) => print_json_output(&content),
            Err(code) => code,
        },
        Err(code) => code,
    }
}

fn parse_heartbeat_duration(value: &str) -> Result<std::time::Duration, String> {
    let (number, suffix) = value
        .trim()
        .chars()
        .position(|c| !c.is_ascii_digit())
        .map_or((value.trim(), "s"), |index| value.trim().split_at(index));
    let number: u64 = number
        .parse()
        .map_err(|_| "expected a positive integer followed by ms, s, m, or h".to_string())?;
    let duration = match suffix {
        "ms" => std::time::Duration::from_millis(number),
        "s" => std::time::Duration::from_secs(number),
        "m" => std::time::Duration::from_secs(number.saturating_mul(60)),
        "h" => std::time::Duration::from_secs(number.saturating_mul(3600)),
        _ => return Err("expected ms, s, m, or h".to_string()),
    };
    (!duration.is_zero())
        .then_some(duration)
        .ok_or_else(|| "must be greater than zero".to_string())
}

/// Client-side poll window for `pane send --submit`. Strictly above the host's
/// own settle + confirm + retry-confirm ceiling.
const SUBMIT_REPLY_WINDOW: std::time::Duration = std::time::Duration::from_secs(25);

/// What the host said about a `--submit`, in the two shapes a caller branches on.
#[derive(Debug, PartialEq, Eq)]
enum SubmitOutcome {
    /// Typed and confirmed submitted. `retried` tells a clean submit from one
    /// the host had to heal.
    Confirmed { retried: bool },
    /// Typed, but the host could not confirm the prompt left the input line.
    /// `input_line` is what the pane actually showed.
    Unconfirmed {
        message: String,
        input_line: Vec<String>,
    },
}

impl SubmitOutcome {
    /// Zero only when the text was both typed *and* confirmed submitted.
    /// Everything else — an unconfirmed submit, a refused pane, a malformed
    /// reply — is one failure from the caller's side: the prompt may not have
    /// been delivered.
    fn exit_code(&self) -> i32 {
        match self {
            SubmitOutcome::Confirmed { .. } => 0,
            SubmitOutcome::Unconfirmed { .. } => 1,
        }
    }

    fn report(&self) {
        match self {
            SubmitOutcome::Confirmed { retried } => {
                if *retried {
                    print_tip(
                        "the prompt was still parked as a collapsed paste; a second Enter submitted it",
                    );
                }
            }
            SubmitOutcome::Unconfirmed {
                message,
                input_line,
            } => {
                eprintln!("error: {message}");
                for line in input_line {
                    eprintln!("  {line}");
                }
            }
        }
    }
}

/// Map the host's submit reply to the caller's outcome. Pure so the exit-code
/// contract is testable without a host or a socket.
fn submit_outcome(reply: &str) -> SubmitOutcome {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(reply) else {
        return SubmitOutcome::Unconfirmed {
            message: format!("invalid submit response from host: {reply}"),
            input_line: Vec::new(),
        };
    };
    let retried = value
        .get("retried")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if value.get("ok").and_then(serde_json::Value::as_bool) == Some(true) {
        return SubmitOutcome::Confirmed { retried };
    }
    SubmitOutcome::Unconfirmed {
        message: value
            .get("error")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("submission not confirmed")
            .to_string(),
        input_line: value
            .get("input_line")
            .and_then(serde_json::Value::as_array)
            .map(|lines| {
                lines
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// `plexi pane key <pane_id> <key>`
///
/// Sends `key_pane` command to PLEXI_SOCKET. Waits for response.
/// Returns 0 on success, 1 on error.
pub fn pane_key_cli(pane_id: u64, key: &str) -> i32 {
    log::info!(
        "pane_key:cli: pane_id={pane_id} key={key:?} key_chars={}",
        key.chars().count()
    );
    let content = match super::request(
        serde_json::json!({
            "type": "key_pane",
            "pane_id": pane_id,
            "key": key,
        }),
        "pane-key-response",
        "pane key",
    ) {
        Ok(content) => content,
        Err(code) => return code,
    };
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
        if v.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
            // Native app panes report whether the app's key handler consumed
            // the key — surface a miss so driving agents don't assume the
            // key acted.
            if let Some(d) = v.get("disposition").and_then(|v| v.as_str()) {
                match d {
                    "consumed" => {}
                    // Routed into the focused text surface's real input
                    // queue; egui applies it next frame, so consumption is
                    // not knowable here — but this is the success path, not
                    // a miss.
                    "text_input" | "text_input_escape" => eprintln!(
                        "note: key routed to the pane's focused text \
                         surface (applied next frame)"
                    ),
                    _ => eprintln!(
                        "note: key delivered but not consumed by the app \
                         (disposition: {d})"
                    ),
                }
            }
            return 0;
        }
        if let Some(msg) = host_error_text(&content) {
            eprintln!("error: {msg}");
            return 1;
        }
    }
    0
}

pub fn pane_drop_cli(pane_id: u64, path_or_url: &str) -> i32 {
    let content = match super::request(
        serde_json::json!({
            "type": "drop_file",
            "pane_id": pane_id,
            "path_or_url": path_or_url,
        }),
        "pane-drop-response",
        "pane drop",
    ) {
        Ok(content) => content,
        Err(code) => return code,
    };
    if let Err(code) = super::check_reply_error(&content) {
        return code;
    }
    print_json_output(&content)
}

/// Send a `click_pane`/`click_pane_node` request and report the host's answer,
/// shared by the pixel, node-targeted, and drag `plexi pane click` variants.
/// Returns 0 on success, 1 on error.
fn click_request(payload: serde_json::Value, prefix: &str, log_prefix: &str) -> i32 {
    let content = match super::request(payload, prefix, "pane click") {
        Ok(content) => content,
        Err(code) => return code,
    };
    if let Some(msg) = host_error_text(&content) {
        log::warn!("{log_prefix}: host reported error: {msg}");
        eprintln!("error: {msg}");
        return 1;
    }
    0
}

/// Host `error` text, with `error_code` prefixed when the reply carries one.
fn host_error_text(content: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(content).ok()?;
    let msg = value.get("error")?.as_str()?;
    match value.get("error_code").and_then(|code| code.as_str()) {
        Some(code) if !code.is_empty() && !msg.contains(code) => Some(format!("{code}: {msg}")),
        _ => Some(msg.to_string()),
    }
}

/// `plexi pane click <pane_id> <x> <y> [--button left]`
///
/// Sends `click_pane` command to PLEXI_SOCKET. Waits for response.
/// Returns 0 on success, 1 on error.
pub fn pane_click_cli(pane_id: u64, x: f32, y: f32, button: &str) -> i32 {
    log::info!("pane_click:cli: pane_id={pane_id} x={x} y={y} button={button}");
    click_request(
        serde_json::json!({
            "type": "click_pane",
            "pane_id": pane_id,
            "x": x,
            "y": y,
            "button": button,
        }),
        "pane-click-response",
        "pane_click:cli",
    )
}

/// `plexi pane click <pane_id> --node <node_id> [--button left]`
///
/// Sends `click_pane_node` command to PLEXI_SOCKET. Waits for response.
/// Returns 0 on success, 1 on error.
pub fn pane_click_node_cli(pane_id: u64, node_id: &str, button: &str) -> i32 {
    log::info!("pane_click_node:cli: pane_id={pane_id} node_id={node_id} button={button}");
    click_request(
        serde_json::json!({
            "type": "click_pane_node",
            "pane_id": pane_id,
            "node_id": node_id,
            "button": button,
        }),
        "pane-click-node-response",
        "pane_click_node:cli",
    )
}

/// Parse a `"x,y"` pane-pixel coordinate pair from a CLI flag value.
fn parse_pane_coords(flag: &str, value: &str) -> Result<[f32; 2], String> {
    let parts: Vec<&str> = value.split(',').map(str::trim).collect();
    let [x, y] = parts.as_slice() else {
        return Err(format!("--{flag} must be \"x,y\" (got {value:?})"));
    };
    match (x.parse::<f32>(), y.parse::<f32>()) {
        (Ok(x), Ok(y)) => Ok([x, y]),
        _ => Err(format!("--{flag} must be numeric \"x,y\" (got {value:?})")),
    }
}

/// `plexi pane drag <pane_id> --from x,y --to x,y [--steps N] [--button left]`
/// (or `--from-node`/`--to-node` with semantic node ids).
///
/// Sends `drag_pane` to PLEXI_SOCKET; the host queues a press → moves →
/// release schedule through the production input path and acks once queued.
/// Returns 0 on success, 1 on error.
pub fn pane_drag_cli(
    pane_id: u64,
    from: Option<&str>,
    from_node: Option<&str>,
    to: Option<&str>,
    to_node: Option<&str>,
    steps: u32,
    button: &str,
) -> i32 {
    let parse = |flag: &str, value: Option<&str>| -> Result<Option<[f32; 2]>, String> {
        value.map(|v| parse_pane_coords(flag, v)).transpose()
    };
    let (from, to) = match (parse("from", from), parse("to", to)) {
        (Ok(f), Ok(t)) => (f, t),
        (Err(e), _) | (_, Err(e)) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    if from.is_none() && from_node.is_none() {
        eprintln!("error: pane drag requires --from x,y or --from-node <node_id>");
        return 2;
    }
    if to.is_none() && to_node.is_none() {
        eprintln!("error: pane drag requires --to x,y or --to-node <node_id>");
        return 2;
    }
    log::info!(
        "pane_drag:cli: pane_id={pane_id} from={from:?} from_node={from_node:?} to={to:?} \
         to_node={to_node:?} steps={steps} button={button}"
    );
    click_request(
        serde_json::json!({
            "type": "drag_pane",
            "pane_id": pane_id,
            "from": from,
            "from_node": from_node,
            "to": to,
            "to_node": to_node,
            "steps": steps,
            "button": button,
        }),
        "pane-drag-response",
        "pane_drag:cli",
    )
}

/// `plexi pane capture [--lines N] [pane_id]`
///
/// Reads the last N lines from a pane's PTY scrollback buffer and prints a JSON array
/// of strings to stdout. If `pane_id` is omitted, defaults to PLEXI_PANE_ID.
/// Returns 0 on success, 1 on error.
pub fn pane_capture_cli(
    pane_id: Option<u64>,
    lines: usize,
    full_output: bool,
    from_cursor: Option<u64>,
    plain: bool,
) -> i32 {
    let resolved_pane_id = match pane_id {
        Some(id) => id,
        None => match std::env::var("PLEXI_PANE_ID") {
            Ok(v) => match v.parse::<u64>() {
                Ok(id) => id,
                Err(_) => {
                    eprintln!("error: PLEXI_PANE_ID is not a valid number: {v}");
                    return 1;
                }
            },
            Err(_) => {
                eprintln!(
                    "error: pane_id not provided and PLEXI_PANE_ID is not set — run inside a Plexi pane or pass a pane ID"
                );
                return 1;
            }
        },
    };

    log::info!(
        "pane_capture:cli: pane_id={resolved_pane_id} lines={lines} full_output={full_output} from_cursor={from_cursor:?} plain={plain}"
    );

    let mut req = serde_json::json!({
        "type": "capture_pane",
        "pane_id": resolved_pane_id,
        "lines": lines,
        "full_output": full_output,
    });
    if let Some(cursor) = from_cursor {
        req["from_cursor"] = serde_json::Value::Number(serde_json::Number::from(cursor));
    }

    let content = match super::request(req, "pane-capture-response", "pane capture") {
        Ok(content) => content,
        Err(code) => return code,
    };
    let parsed = match parse_capture_reply(&content) {
        Ok(parsed) => parsed,
        Err(error) => {
            eprintln!("error: {error}");
            return 1;
        }
    };
    // The cursor is metadata, so it always stays off stdout. In `--plain`
    // mode stdout is exactly the raw line stream; default JSON is unchanged.
    eprintln!("cursor={}", parsed.cursor);
    if plain {
        print!("{}", plain_capture_body(&parsed.lines));
        return 0;
    }
    print_json_output(&content)
}

#[derive(serde::Deserialize)]
struct CaptureReply {
    lines: Vec<String>,
    cursor: u64,
}

fn parse_capture_reply(content: &str) -> Result<CaptureReply, String> {
    let value: serde_json::Value = serde_json::from_str(content)
        .map_err(|error| format!("invalid pane capture response: {error}"))?;
    if let Some(error) = value.get("error").and_then(serde_json::Value::as_str) {
        return Err(error.to_string());
    }
    serde_json::from_value(value).map_err(|error| format!("invalid pane capture response: {error}"))
}

fn plain_capture_body(lines: &[String]) -> String {
    let mut body = String::new();
    for line in lines {
        body.push_str(line);
        body.push('\n');
    }
    body
}

/// `plexi pane status <id>`
pub fn pane_status_cli(pane_id: u64) -> i32 {
    log::info!("pane_status:cli: pane_id={pane_id}");
    let content = match super::request(
        serde_json::json!({
            "type": "pane_status",
            "pane_id": pane_id,
        }),
        "pane-status-response",
        "pane status",
    ) {
        Ok(content) => content,
        Err(code) => return code,
    };
    if let Err(error) = validate_status_reply(&content) {
        eprintln!("error: {error}");
        return 1;
    }
    print_json_output(&content)
}

fn validate_status_reply(content: &str) -> Result<(), String> {
    let value: serde_json::Value = serde_json::from_str(content)
        .map_err(|error| format!("invalid pane status response: {error}"))?;
    if let Some(error) = value.get("error").and_then(serde_json::Value::as_str) {
        return Err(error.to_string());
    }
    for field in [
        "verdict",
        "confidence",
        "agent_state",
        "status_bar",
        "status_bar_truncated",
        "last_buffer_line",
    ] {
        if value.get(field).is_none() {
            return Err(format!("invalid pane status response: missing {field}"));
        }
    }
    Ok(())
}

/// `plexi pane state <id> [--stale-after <secs>]`
///
/// Sends a `get_pane_state` command to PLEXI_SOCKET. For app panes, the host
/// writes a JSON object containing a versioned normalized `semantic` tree.
/// Process apps also retain the compatible `frame` RenderCommand array.
/// For terminal panes, returns a simple status object. Every response also
/// carries `claimed_state` (the pane's agent-authored `status` slot) and
/// `observed_state` (host-derived liveness), always together — see
/// `AppRequest::GetPaneState`. Returns 0 on success, 1 on error.
pub fn pane_state_cli(pane_id: u64, stale_after_secs: Option<u64>) -> i32 {
    log::info!("pane_state:cli: pane_id={pane_id} stale_after_secs={stale_after_secs:?}");

    let content = match super::request(
        serde_json::json!({
            "type": "get_pane_state",
            "pane_id": pane_id,
            "stale_after_secs": stale_after_secs,
        }),
        "pane-state-response",
        "pane state",
    ) {
        Ok(content) => content,
        Err(code) => return code,
    };
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
        if let Some(err) = pane_state_envelope_error(&v) {
            eprintln!("error: {err}");
            return 1;
        }
    }
    print_json_output(&content)
}

/// The pane-state transport envelope signals failure with a top-level `error`
/// string. Successful responses never carry a top-level `error` key — guest
/// failure detail is nested under `failure` (see the `get_pane_state` handler)
/// — so a success can never be misread as a transport failure here.
pub(crate) fn pane_state_envelope_error(value: &serde_json::Value) -> Option<&str> {
    value.get("error").and_then(|e| e.as_str())
}

/// `plexi open <type_id> [args...] [--layout=X]`
///
/// When called from inside a Plexi pane (PLEXI_SOCKET is set), sends a
/// spawn_pane command directly via the socket — channel-agnostic, works on
/// alpha, beta, main, and PR builds without caring which binary is on PATH.
///
/// `plexi open github:owner/repo` — clone and run ephemerally, without installing.
///
/// Clones to a channel-scoped cache dir and sends a path-based spawn_pane,
/// passing the user's workspace root so app state is scoped correctly.
pub(super) fn open_github_ephemeral(
    source: &str,
    layout: Option<&str>,
    from_pane_id: Option<u64>,
    cwd: Option<&str>,
) -> i32 {
    let rest = source.strip_prefix("github:").unwrap_or(source);
    let parts: Vec<&str> = rest.splitn(2, '/').collect();
    if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
        eprintln!("error: invalid github source '{source}'; expected 'github:owner/repo'");
        return 1;
    }
    let owner = parts[0];
    let repo = parts[1].trim_end_matches(".git");

    let cache_dir = crate::config::config_dir()
        .join("github-cache")
        .join(owner)
        .join(repo);

    // Ensure the parent directory exists before cloning.
    if let Some(parent) = cache_dir.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!(
                "error: could not create cache directory {}: {e}",
                parent.display()
            );
            return 1;
        }
    }

    if !cache_dir.exists() {
        let url = format!("https://github.com/{owner}/{repo}.git");
        log::info!(
            "open_github_ephemeral: cloning {url} → {}",
            cache_dir.display()
        );
        eprintln!("Cloning github:{owner}/{repo}...");
        match std::process::Command::new("git")
            .arg("clone")
            .arg("--depth=1")
            .arg(&url)
            .arg(&cache_dir)
            .status()
        {
            Ok(s) if s.success() => {}
            Ok(s) => {
                eprintln!("error: git clone failed (exit {})", s.code().unwrap_or(-1));
                return 1;
            }
            Err(e) => {
                eprintln!("error: could not run git: {e}");
                return 1;
            }
        }
    } else {
        log::info!(
            "open_github_ephemeral: reusing cache at {}",
            cache_dir.display()
        );
    }

    // Resolve workspace root from the provided cwd, falling back to current_dir.
    let start_dir = cwd
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::current_dir().ok());
    let workspace_root: Option<String> = start_dir
        .as_deref()
        .and_then(crate::app::registry::resolve_workspace_root)
        .map(|p| p.to_string_lossy().into_owned());

    let abs_path = cache_dir.to_string_lossy().into_owned();
    log::info!(
        "open_github_ephemeral: launching from {abs_path} workspace_root={workspace_root:?}"
    );

    if super::command_socket_available() {
        let mut payload = serde_json::json!({
            "type": "spawn_pane",
            "type_id": "",
            "path": abs_path,
            "layout": layout,
        });
        if let Some(pid) = from_pane_id {
            payload["from_pane_id"] = serde_json::Value::Number(pid.into());
        }
        if let Some(ref ws) = workspace_root {
            payload["workspace_root"] = serde_json::Value::String(ws.clone());
        }
        if let Some(cwd_str) = cwd {
            payload["cwd"] = serde_json::Value::String(cwd_str.to_string());
        }
        log::info!("open_github_ephemeral: sending via socket");
        let response_file = match super::send_request(payload, "spawn-pane-response", "pane") {
            Ok(response_file) => response_file,
            Err(code) => return code,
        };
        return super::open::wait_for_response(&response_file);
    }

    // Fallback: spawn-queue (outside a Plexi pane). Only queue when a host is
    // actually servicing this channel — never park a spawn into the void
    // (stint 0532).
    if let Err(code) = crate::cli::require_spawn_servicing_host("open github") {
        return code;
    }
    let mut queue_payload = serde_json::json!({
        "type_id": "",
        "path": abs_path,
        "layout": layout,
    });
    if let Some(ref ws) = workspace_root {
        queue_payload["workspace_root"] = serde_json::Value::String(ws.clone());
    }
    if let Some(cwd_str) = cwd {
        queue_payload["cwd"] = serde_json::Value::String(cwd_str.to_string());
    }
    log::info!("open_github_ephemeral: queued path={abs_path}");
    match crate::cli::queue_spawn(
        queue_payload,
        "open github",
        &format!("open github:{owner}/{repo}"),
    ) {
        Ok(_) => 0,
        Err(code) => code,
    }
}

#[cfg(test)]
mod slot_write_tests {
    use super::*;

    #[test]
    fn ack_replace_reports_byte_count() {
        assert_eq!(
            slot_write_ack("status", 4, false),
            "slot \"status\" <- 4 bytes"
        );
    }

    #[test]
    fn ack_append_uses_append_arrow() {
        assert_eq!(
            slot_write_ack("status", 12, true),
            "slot \"status\" +<- 12 bytes"
        );
    }

    #[test]
    fn empty_slot_name_exits_nonzero() {
        // Must fail before any socket contact — no PLEXI_PANE_ID needed.
        assert_eq!(pane_slot_write_cli("", Some("x"), false, true, Some(7)), 1);
        assert_eq!(
            pane_slot_write_cli("   ", Some("x"), false, true, Some(7)),
            1
        );
    }
}

#[cfg(test)]
mod capture_and_status_tests {
    use super::*;
    use crate::cli::test_env::socket_env_guard;
    use std::io::{BufRead as _, BufReader};
    use std::os::unix::net::UnixListener;

    fn capture_payload_and_answer(
        response: serde_json::Value,
        run_cli: impl FnOnce() -> i32,
    ) -> (i32, serde_json::Value) {
        let env = socket_env_guard();
        let dir = tempfile::tempdir().expect("tempdir");
        let socket_path = dir.path().join("notify.sock");
        let listener = UnixListener::bind(&socket_path).expect("bind socket");
        env.set(&socket_path);
        let (tx, rx) = std::sync::mpsc::channel();
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            let mut line = String::new();
            BufReader::new(stream)
                .read_line(&mut line)
                .expect("read payload");
            let payload: serde_json::Value = serde_json::from_str(&line).expect("payload json");
            crate::rpc::write_json_response(
                payload["response_file"].as_str().expect("response_file"),
                response,
            );
            tx.send(payload).ok();
        });
        let code = run_cli();
        let payload = rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("listener never received payload");
        handle.join().expect("listener thread");
        (code, payload)
    }

    #[test]
    fn plain_body_is_only_raw_lines() {
        let lines = vec!["alpha".to_string(), String::new(), "{not json}".to_string()];
        assert_eq!(plain_capture_body(&lines), "alpha\n\n{not json}\n");
        assert!(!plain_capture_body(&lines).contains(r#""lines""#));
    }

    #[test]
    fn capture_plain_composes_with_from_cursor_in_one_request() {
        let (code, payload) = capture_payload_and_answer(
            serde_json::json!({
                "lines": ["delta-one", "delta-two"],
                "cursor": 44,
                "missed": false,
            }),
            || pane_capture_cli(Some(42), 50, false, Some(39), true),
        );
        assert_eq!(code, 0);
        assert_eq!(payload["type"], "capture_pane");
        assert_eq!(payload["pane_id"], 42);
        assert_eq!(payload["from_cursor"], 39);
        assert_eq!(payload["full_output"], false);
    }

    #[test]
    fn capture_host_error_exits_one() {
        let (code, _) =
            capture_payload_and_answer(serde_json::json!({"error": "pane 42 not found"}), || {
                pane_capture_cli(Some(42), 50, false, None, true)
            });
        assert_eq!(code, 1);
    }

    #[test]
    fn status_payload_and_success_shape_match_contract() {
        let (code, payload) = capture_payload_and_answer(
            serde_json::json!({
                "verdict": "working",
                "confidence": "high",
                "agent_state": "working",
                "detail": null,
                "status_bar": "esc to interrupt",
                "status_bar_truncated": false,
                "last_buffer_line": "⏺ Bash(cargo test)",
            }),
            || pane_status_cli(42),
        );
        assert_eq!(code, 0);
        assert_eq!(payload["type"], "pane_status");
        assert_eq!(payload["pane_id"], 42);
    }

    #[test]
    fn status_host_and_malformed_errors_exit_one() {
        let (host_error, _) =
            capture_payload_and_answer(serde_json::json!({"error": "pane 42 not found"}), || {
                pane_status_cli(42)
            });
        assert_eq!(host_error, 1);
        assert!(
            validate_status_reply("{}")
                .expect_err("missing fields")
                .contains("missing verdict")
        );
    }
}

/// `pane slot wait`'s exit codes are its contract: a caller branches on 0 /
/// 2 / 1 alone, so every reply shape is pinned to one of them here.
#[cfg(test)]
mod slot_wait_tests {
    use super::*;
    use crate::rpc::{PollError, SlotReply};

    #[test]
    fn oversized_timeout_exits_one_before_socket_contact() {
        // Finite values beyond Duration's range would panic in
        // Duration::from_secs_f64 — the bound check must fire first.
        assert_eq!(pane_slot_wait_cli("status", Some(7), "ready", 1e308), 1);
    }

    #[test]
    fn matching_value_exits_zero_and_carries_raw_bytes() {
        let outcome = slot_wait_outcome(Ok(SlotReply::Data(b"done: ok".to_vec())));
        assert_eq!(outcome.exit_code(), 0);
        assert_eq!(outcome, SlotWaitOutcome::Matched(b"done: ok".to_vec()));
    }

    #[test]
    fn host_timeout_reply_exits_two() {
        let body = br#"{"ok":false,"timeout":true,"error":"timed out waiting for slot 'status'"}"#;
        let outcome = slot_wait_outcome(Ok(SlotReply::Err(body.to_vec())));
        assert_eq!(outcome.exit_code(), 2);
        assert!(matches!(outcome, SlotWaitOutcome::TimedOut(_)));
    }

    #[test]
    fn host_error_reply_exits_one() {
        let body = br#"{"ok":false,"error":"pane 42 not found"}"#;
        let outcome = slot_wait_outcome(Ok(SlotReply::Err(body.to_vec())));
        assert_eq!(outcome.exit_code(), 1);
        assert_eq!(
            outcome,
            SlotWaitOutcome::Failed("pane 42 not found".to_string())
        );
    }

    #[test]
    fn client_side_poll_timeout_is_a_plumbing_failure_not_a_wait_timeout() {
        // The host always answers inside its own deadline, so an unanswered
        // response file means the request never landed. Reporting that as 2
        // would tell the caller its condition failed when nothing watched it.
        let outcome = slot_wait_outcome(Err(PollError::TimedOut));
        assert_eq!(outcome.exit_code(), 1);
    }

    #[test]
    fn unparseable_error_reply_exits_one() {
        let outcome = slot_wait_outcome(Ok(SlotReply::Err(b"not json".to_vec())));
        assert_eq!(outcome.exit_code(), 1);
    }

    #[test]
    fn negative_timeout_exits_one_before_any_socket_contact() {
        assert_eq!(pane_slot_wait_cli("status", Some(7), "ready", -1.0), 1);
        assert_eq!(pane_slot_wait_cli("status", Some(7), "ready", f64::NAN), 1);
    }
}

/// `pane send --submit`'s exit code is its contract: a caller branches on
/// 0 (typed and confirmed) versus non-zero (typed, unconfirmed) alone, so every
/// reply shape the host can produce is pinned to one of them here.
#[cfg(test)]
mod submit_tests {
    use super::*;
    use crate::cli::test_env::socket_env_guard;
    use std::io::{BufRead as _, BufReader};
    use std::os::unix::net::UnixListener;

    #[test]
    fn confirmed_reply_exits_zero() {
        let outcome = submit_outcome(r#"{"ok":true,"retried":false}"#);
        assert_eq!(outcome, SubmitOutcome::Confirmed { retried: false });
        assert_eq!(outcome.exit_code(), 0);
    }

    /// A healed submit still succeeded. The retry is reported, not penalised —
    /// the prompt reached the agent either way.
    #[test]
    fn confirmed_after_retry_still_exits_zero() {
        let outcome = submit_outcome(r#"{"ok":true,"retried":true}"#);
        assert_eq!(outcome, SubmitOutcome::Confirmed { retried: true });
        assert_eq!(outcome.exit_code(), 0);
    }

    #[test]
    fn unconfirmed_reply_exits_one_and_carries_the_observed_input_line() {
        let outcome = submit_outcome(
            r#"{"error":"submission not confirmed on pane 7","input_line":["> echo hi","[Pasted text #1] paste again to expand"],"retried":true}"#,
        );
        assert_eq!(
            outcome,
            SubmitOutcome::Unconfirmed {
                message: "submission not confirmed on pane 7".to_string(),
                input_line: vec![
                    "> echo hi".to_string(),
                    "[Pasted text #1] paste again to expand".to_string(),
                ],
            }
        );
        assert_eq!(outcome.exit_code(), 1);
    }

    /// A refusal (app pane, submit already in flight, pane gone) carries no
    /// input line and is still a non-zero exit: nothing was submitted.
    #[test]
    fn refusal_reply_exits_one() {
        let outcome = submit_outcome(
            r#"{"error":"pane 7 is an app pane; --submit only applies to terminal panes"}"#,
        );
        assert_eq!(outcome.exit_code(), 1);
        assert!(matches!(outcome, SubmitOutcome::Unconfirmed { .. }));
    }

    /// Never report success on a reply we could not read. An unparseable answer
    /// means the submission state is unknown, which the caller must treat as
    /// unconfirmed.
    #[test]
    fn unparseable_reply_exits_one() {
        assert_eq!(submit_outcome("not json").exit_code(), 1);
        assert_eq!(submit_outcome(r#"{"ok":false}"#).exit_code(), 1);
    }

    /// The flag has to reach the host, and the plain verb has to keep sending
    /// the absence of it — the host branches on this field alone.
    #[test]
    fn submit_flag_lands_in_the_host_payload() {
        for (submit, paste) in [(true, false), (false, false), (false, true)] {
            let env = socket_env_guard();
            let dir = tempfile::tempdir().expect("tempdir");
            let socket_path = dir.path().join("notify.sock");
            let listener = UnixListener::bind(&socket_path).expect("bind socket");
            env.set(&socket_path);

            let (tx, rx) = std::sync::mpsc::channel();
            let handle = std::thread::spawn(move || {
                let (stream, _) = listener.accept().expect("accept");
                let mut line = String::new();
                BufReader::new(stream)
                    .read_line(&mut line)
                    .expect("read payload");
                let payload: serde_json::Value = serde_json::from_str(&line).expect("payload json");
                // Answer as the host would, so the CLI does not sit on its poll
                // window: this test is about the request, not the wait.
                crate::rpc::write_json_response(
                    payload["response_file"].as_str().expect("response_file"),
                    serde_json::json!({ "ok": true, "retried": false }),
                );
                tx.send(payload).ok();
            });

            let code = pane_send_cli(42, "hello", submit, paste);
            let payload = rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("host listener never received the send payload");
            handle.join().expect("listener thread");

            assert_eq!(code, 0, "submit={submit}");
            assert_eq!(payload["type"], "send_to_pane");
            assert_eq!(payload["submit"], submit, "payload was {payload}");
            assert_eq!(payload["paste"], paste, "payload was {payload}");
        }
    }
}

#[cfg(test)]
mod pane_message_tests {
    use super::*;
    use crate::cli::test_env::socket_env_guard;
    use std::io::{BufRead as _, BufReader};
    use std::os::unix::net::UnixListener;

    #[test]
    fn send_payload_omits_self_reported_sender_and_forwards_submit() {
        for submit in [false, true] {
            let env = socket_env_guard();
            let dir = tempfile::tempdir().expect("tempdir");
            let socket_path = dir.path().join("notify.sock");
            let listener = UnixListener::bind(&socket_path).expect("bind socket");
            env.set(&socket_path);

            let (tx, rx) = std::sync::mpsc::channel();
            let handle = std::thread::spawn(move || {
                let (stream, _) = listener.accept().expect("accept");
                let mut line = String::new();
                BufReader::new(stream)
                    .read_line(&mut line)
                    .expect("read payload");
                let payload: serde_json::Value = serde_json::from_str(&line).expect("payload json");
                crate::rpc::write_json_response(
                    payload["response_file"].as_str().expect("response_file"),
                    serde_json::json!({
                        "ok": true,
                        "status": if submit { "delivered" } else { "queued" },
                        "state": if submit { "delivered" } else { "queued" },
                        "pane_id": 42,
                        "from_pane_id": 7,
                        "sender_pane_id": 7,
                        "message_id": 1,
                        "queue_len": 1,
                        "woke": submit,
                    }),
                );
                tx.send(payload).ok();
            });

            let code = pane_message_send_cli(42, "hello mailbox", submit);
            let payload = rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("host listener never received the message send payload");
            handle.join().expect("listener thread");

            assert_eq!(code, 0, "submit={submit}");
            assert_eq!(payload["type"], "pane_message_send");
            assert_eq!(payload["pane_id"], 42);
            assert_eq!(payload["payload"], "hello mailbox");
            assert_eq!(payload["submit"], submit);
            assert!(
                payload.get("from_pane_id").is_none()
                    && payload.get("sender_pane_id").is_none()
                    && payload.get("peer_ancestry").is_none(),
                "sender identity must never be self-reported by the CLI: {payload}"
            );
        }
    }

    #[test]
    fn recv_invalid_timeout_exits_one_before_socket_contact() {
        assert_eq!(pane_message_recv_cli(Some(7), true, -1.0), 1);
        assert_eq!(pane_message_recv_cli(Some(7), true, f64::NAN), 1);
        assert_eq!(pane_message_recv_cli(Some(7), true, 1e308), 1);
    }

    #[test]
    fn recv_timeout_sidecar_exits_two() {
        let env = socket_env_guard();
        let dir = tempfile::tempdir().expect("tempdir");
        let socket_path = dir.path().join("notify.sock");
        let listener = UnixListener::bind(&socket_path).expect("bind socket");
        env.set(&socket_path);

        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            let mut line = String::new();
            BufReader::new(stream)
                .read_line(&mut line)
                .expect("read payload");
            let payload: serde_json::Value = serde_json::from_str(&line).expect("payload json");
            let rf = payload["response_file"].as_str().expect("response_file");
            crate::rpc::write_json_response(
                &format!("{rf}.err"),
                serde_json::json!({
                    "ok": false,
                    "timeout": true,
                    "error": "timed out waiting for message on pane 7",
                }),
            );
        });

        let code = pane_message_recv_cli(Some(7), true, 0.1);
        handle.join().expect("listener thread");
        assert_eq!(code, 2);
    }

    #[test]
    fn close_pane_outcome_formats_retained_and_reclaimed_worktrees() {
        let (out, err, code) = close_pane_outcome(
            r#"{"ok":true,"pane_id":9,"worktree":{"branch":"feature/x","path":"/tmp/wt/feature/x","action":"retained","reason":"dirty"}}"#,
        );
        assert_eq!(code, 0);
        assert!(err.is_none());
        assert_eq!(
            out.as_deref(),
            Some("retained worktree: /tmp/wt/feature/x (dirty)")
        );

        let (out2, err2, code2) = close_pane_outcome(
            r#"{"ok":true,"pane_id":9,"worktree":{"branch":"feature/x","path":"/tmp/wt/feature/x","action":"reclaimed","reason":"clean_and_unmoved"}}"#,
        );
        assert_eq!(code2, 0);
        assert!(err2.is_none());
        assert!(out2.is_none());

        let (out3, err3, code3) = close_pane_outcome(r#"{"error":"pane 9 not found"}"#);
        assert_eq!(code3, 1);
        assert!(out3.is_none());
        assert_eq!(err3.as_deref(), Some("error: pane 9 not found"));
    }
}

#[cfg(test)]
mod pane_result_tests {
    use super::*;
    use crate::cli::test_env::socket_env_guard;
    use crate::rpc::{PollError, SlotReply};
    use std::io::{BufRead as _, BufReader};
    use std::os::unix::net::UnixListener;

    #[test]
    fn read_outcome_distinguishes_validated_no_result_yet_and_invalid_result() {
        let valid = pane_result_read_outcome(Ok(SlotReply::Data(
            br#"{"status":"pass","pr":42}"#.to_vec(),
        )));
        assert_eq!(valid.exit_code(), 0);
        assert_eq!(
            valid,
            PaneResultReadOutcome::Validated(br#"{"status":"pass","pr":42}"#.to_vec())
        );

        let no_result = pane_result_read_outcome(Ok(SlotReply::Err(
            br#"{"ok":false,"pane_id":7,"error_code":"no_result_yet","error":"pane 7 has not written a result yet"}"#
                .to_vec(),
        )));
        assert_eq!(
            no_result.exit_code(),
            2,
            "no_result_yet must exit 2 so callers can distinguish it from invalid_result"
        );
        assert_eq!(
            no_result,
            PaneResultReadOutcome::NoResultYet {
                error_code: "no_result_yet".to_string(),
                message: "no_result_yet: pane 7 has not written a result yet".to_string(),
            }
        );

        let invalid = pane_result_read_outcome(Ok(SlotReply::Err(
            br#"{"ok":false,"pane_id":7,"error_code":"invalid_result","error":"$: missing required property \"status\""}"#
                .to_vec(),
        )));
        assert_eq!(
            invalid.exit_code(),
            1,
            "invalid_result must exit 1 so callers can distinguish it from no_result_yet"
        );
        assert_eq!(
            invalid,
            PaneResultReadOutcome::InvalidResult {
                error_code: "invalid_result".to_string(),
                message: "invalid_result: $: missing required property \"status\"".to_string(),
            }
        );

        let timeout = pane_result_read_outcome(Ok(SlotReply::Err(
            br#"{"ok":false,"timeout":true,"pane_id":7,"error_code":"timeout","error":"timed out waiting for valid result on pane 7"}"#
                .to_vec(),
        )));
        assert_eq!(timeout.exit_code(), 2);

        let plumbing = pane_result_read_outcome(Err(PollError::TimedOut));
        assert_eq!(plumbing.exit_code(), 1);
    }

    #[test]
    fn read_cli_roundtrip_over_socket() {
        let env = socket_env_guard();
        let dir = tempfile::tempdir().expect("tempdir");
        let socket_path = dir.path().join("notify.sock");
        let listener = UnixListener::bind(&socket_path).expect("bind socket");
        env.set(&socket_path);

        let handle = std::thread::spawn(move || {
            // 2. Read while last write is invalid -> sidecar .err with invalid_result (exit 1)
            let (stream, _) = listener.accept().expect("accept 2");
            let mut line = String::new();
            BufReader::new(stream).read_line(&mut line).expect("read 2");
            let p2: serde_json::Value = serde_json::from_str(&line).expect("json 2");
            assert_eq!(p2["type"], "pane_result_read");
            let rf2 = p2["response_file"].as_str().unwrap();
            crate::rpc::write_json_response(
                &format!("{rf2}.err"),
                serde_json::json!({
                    "ok": false,
                    "pane_id": 7,
                    "error_code": "invalid_result",
                    "error": "$: missing required property \"status\"",
                }),
            );

            // 3. Read when no result yet -> sidecar .err with no_result_yet (exit 2)
            let (stream, _) = listener.accept().expect("accept 3");
            let mut line = String::new();
            BufReader::new(stream).read_line(&mut line).expect("read 3");
            let p3: serde_json::Value = serde_json::from_str(&line).expect("json 3");
            let rf3 = p3["response_file"].as_str().unwrap();
            crate::rpc::write_json_response(
                &format!("{rf3}.err"),
                serde_json::json!({
                    "ok": false,
                    "pane_id": 8,
                    "error_code": "no_result_yet",
                    "error": "pane 8 has not written a result yet",
                }),
            );

            // 5. Read committed result (exit 0)
            let (stream, _) = listener.accept().expect("accept 5");
            let mut line = String::new();
            BufReader::new(stream).read_line(&mut line).expect("read 5");
            let p5: serde_json::Value = serde_json::from_str(&line).expect("json 5");
            let rf5 = p5["response_file"].as_str().unwrap();
            crate::rpc::write_json_response(rf5, serde_json::json!({ "status": "pass" }));
        });

        assert_eq!(pane_result_read_cli(Some(7), false, 300.0), 1);
        assert_eq!(pane_result_read_cli(Some(8), false, 300.0), 2);
        assert_eq!(pane_result_read_cli(Some(7), false, 300.0), 0);

        handle.join().expect("listener thread");
    }

    #[test]
    fn context_and_pane_target_resolve_and_commands_over_socket() {
        use crate::cli::{
            host_action_cli, pane_focus_cli, resolve_opt_context_target, ContextTarget, PaneTarget,
        };

        let env = socket_env_guard();
        let dir = tempfile::tempdir().expect("tempdir");
        let socket_path = dir.path().join("notify.sock");
        let listener = UnixListener::bind(&socket_path).expect("bind socket");
        env.set(&socket_path);

        // Numeric targets resolve without any socket connection.
        assert_eq!(ContextTarget::from("42").resolve(), Ok(42));
        assert_eq!(PaneTarget::from("99").resolve(), Ok(99));

        let handle = std::thread::spawn(move || {
            for step in 0..10 {
                let (stream, _) = listener.accept().expect("accept");
                let mut line = String::new();
                BufReader::new(stream).read_line(&mut line).expect("read");
                let payload: serde_json::Value = serde_json::from_str(&line).expect("json");
                let ty = payload["type"].as_str().unwrap_or("");
                match ty {
                    "list_contexts" => {
                        let rf = payload["response_file"].as_str().unwrap();
                        crate::rpc::write_json_response(
                            rf,
                            serde_json::json!([
                                { "context_id": 1, "name": { "auto": "main" }, "parked": false },
                                { "context_id": 2, "name": { "custom": "sprint-alpha" }, "parked": true },
                                { "context_id": 3, "name": "sprint-beta", "parked": false },
                            ]),
                        );
                    }
                    "list_panes" => {
                        let rf = payload["response_file"].as_str().unwrap();
                        crate::rpc::write_json_response(
                            rf,
                            serde_json::json!([
                                { "id": 10, "title": "worker-0890" },
                                { "id": 11, "title": "worker-0891" },
                                { "id": 12, "title": "tester-lane" },
                            ]),
                        );
                    }
                    "context_park" => {
                        assert_eq!(payload["context_id"], 2, "step={step}");
                        assert_eq!(payload["parked"], false, "step={step}");
                        let rf = payload["response_file"].as_str().unwrap();
                        crate::rpc::write_json_response(rf, serde_json::json!({ "ok": true }));
                    }
                    "focus_pane" => {
                        assert_eq!(payload["pane_id"], 12, "step={step}");
                        if let Some(rf) = payload["response_file"].as_str() {
                            crate::rpc::write_json_response(rf, serde_json::json!({ "ok": true }));
                        }
                    }
                    other => panic!("unexpected request type at step {step}: {other}"),
                }
            }
        });

        // ContextTarget over socket: exact, unique prefix, ambiguous prefix, unknown name
        assert_eq!(ContextTarget::from("sprint-alpha").resolve(), Ok(2));
        assert_eq!(ContextTarget::from("ma").resolve(), Ok(1));
        assert_eq!(ContextTarget::from("sprint-").resolve(), Err(1));
        assert_eq!(ContextTarget::from("no-such-ctx").resolve(), Err(1));

        // PaneTarget over socket: exact, unique prefix, ambiguous prefix
        assert_eq!(PaneTarget::from("worker-0890").resolve(), Ok(10));
        assert_eq!(PaneTarget::from("worker-").resolve(), Err(1));

        // End-to-end `plexi context unpark <name>` (via unique prefix "sprint-a" -> context 2)
        let unpark_ctx = resolve_opt_context_target(Some(&ContextTarget::from("sprint-a")))
            .expect("resolve sprint-a");
        assert_eq!(unpark_ctx, Some(2));
        assert_eq!(
            host_action_cli(
                serde_json::json!({ "type": "context_park", "parked": false }),
                Some(("context_id", unpark_ctx, "PLEXI_CONTEXT_ID")),
            ),
            0
        );

        // End-to-end `plexi pane focus <name>` (via unique prefix "tester" -> pane 12)
        let focus_pane = PaneTarget::from("tester").resolve().expect("resolve tester");
        assert_eq!(focus_pane, 12);
        assert_eq!(pane_focus_cli(focus_pane), 0);

        handle.join().expect("listener thread");
    }
}
