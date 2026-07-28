use super::{print_tip, send_to_socket};

pub fn pane_set_title_cli(pane_id: Option<u64>, name: &str) -> i32 {
    let resolved_pane_id = match pane_id {
        Some(id) => id,
        None => {
            let pane_id_str = match std::env::var("PLEXI_PANE_ID") {
                Ok(v) => v,
                Err(_) => {
                    eprintln!("error: PLEXI_PANE_ID is not set — run this inside a Plexi terminal pane or provide a pane ID");
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
    log::info!("pane_set_title:cli: pane_id={resolved_pane_id} name={name:?}");
    send_to_socket(serde_json::json!({
        "type": "set_pane_title",
        "pane_id": resolved_pane_id,
        "name": name,
    }))
}

/// Prints JSON to stdout. If `jq` is in PATH, pipes through `jq .` for
/// colour and pretty-printing; otherwise falls back to serde pretty-print.
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
                log::info!("print_json_output: rendered via jq");
                return 0;
            }
            Err(e) => {
                log::warn!("print_json_output: jq spawn failed ({e}), falling back to serde");
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

fn resolve_pane_id(pane_id: Option<u64>) -> Result<u64, i32> {
    match pane_id {
        Some(id) => Ok(id),
        None => {
            let raw = match std::env::var("PLEXI_PANE_ID") {
                Ok(v) => v,
                Err(_) => {
                    eprintln!("error: pane_id not provided and PLEXI_PANE_ID is not set — run inside a Plexi pane or pass a pane ID");
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

/// Terse success ack for `pane slot write`, printed to stderr so stdout stays byte-clean.
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
    let response_file = crate::rpc::response_file("pane-slot-write-response", "json");
    log::info!(
        "pane_slot_write:cli: pane_id={resolved_pane_id} slot={name:?} bytes={} append={append} replace={replace} response_file={response_file:?}",
        bytes.len()
    );
    let code = send_to_socket(serde_json::json!({
        "type": "slot_write",
        "pane_id": resolved_pane_id,
        "slot_name": name,
        "content": bytes,
        "append": append,
        "replace": replace,
        "response_file": response_file,
    }));
    if code != 0 {
        return code;
    }
    let content = match wait_for_response_bytes(&response_file, "pane slot write") {
        Ok(content) => content,
        Err(code) => return code,
    };
    if let Some(err) = response_error(&content) {
        eprintln!("error: {err}");
        return 1;
    }
    eprintln!("{}", slot_write_ack(name, bytes.len(), append));
    0
}

/// `plexi pane slot read <name> [pane_id]`
pub fn pane_slot_read_cli(name: &str, pane_id: Option<u64>) -> i32 {
    let resolved_pane_id = match resolve_pane_id(pane_id) {
        Ok(id) => id,
        Err(code) => return code,
    };
    let response_file = crate::rpc::response_file("pane-slot-read-response", "json");
    log::info!(
        "pane_slot_read:cli: pane_id={resolved_pane_id} slot={name:?} response_file={response_file:?}"
    );
    let code = send_to_socket(serde_json::json!({
        "type": "slot_read",
        "pane_id": resolved_pane_id,
        "slot_name": name,
        "response_file": response_file,
    }));
    if code != 0 {
        return code;
    }
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
        || timeout < 0.0
        || timeout > crate::app::pane_wait::MAX_WAIT_TIMEOUT_SECS
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
    let response_file = crate::rpc::response_file("pane-slot-wait-response", "json");
    log::info!(
        "pane_slot_wait:cli: pane_id={resolved_pane_id} slot={name:?} until={until:?} timeout={timeout} response_file={response_file:?}"
    );
    let code = send_to_socket(serde_json::json!({
        "type": "slot_wait",
        "pane_id": resolved_pane_id,
        "slot_name": name,
        "pattern": until,
        "timeout_secs": timeout,
        "response_file": response_file,
    }));
    if code != 0 {
        return code;
    }
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
    let response_file = crate::rpc::response_file("pane-slot-list-response", "json");
    log::info!("pane_slot_list:cli: pane_id={resolved_pane_id} response_file={response_file:?}");
    let code = send_to_socket(serde_json::json!({
        "type": "slot_list",
        "pane_id": resolved_pane_id,
        "response_file": response_file,
    }));
    if code != 0 {
        return code;
    }
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
    let response_file = crate::rpc::response_file("pane-slot-delete-response", "json");
    log::info!(
        "pane_slot_delete:cli: pane_id={resolved_pane_id} slot={name:?} response_file={response_file:?}"
    );
    let code = send_to_socket(serde_json::json!({
        "type": "slot_delete",
        "pane_id": resolved_pane_id,
        "slot_name": name,
        "response_file": response_file,
    }));
    if code != 0 {
        return code;
    }
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

/// `plexi pane list`
///
/// Sends a `list_panes` command to PLEXI_SOCKET. The host writes a JSON array
/// to a response file; this function polls for it and prints it to stdout.
/// Returns 0 on success, 1 on error.
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

    let response_file = crate::rpc::response_file("pane-list-response", "json");

    let mut payload = serde_json::json!({
        "type": "list_panes",
        "response_file": response_file,
    });
    if let Some(cid) = context_id {
        payload["context_id"] = serde_json::json!(cid);
    }

    log::info!(
        "pane_list:cli: sending via socket context_id={:?} response_file={:?}",
        context_id,
        response_file
    );

    let code = send_to_socket(payload);
    if code != 0 {
        return code;
    }

    match super::poll_rpc(&response_file, "pane list") {
        Ok(content) => print_json_output(&content),
        Err(code) => code,
    }
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
pub fn pane_info_cli(previous: Option<u64>) -> i32 {
    let socket_path = match super::resolve_command_socket() {
        Some(path) => path,
        None => {
            eprintln!("error: PLEXI_SOCKET is not set — run this inside a Plexi terminal pane");
            return 1;
        }
    };

    let response_file = crate::rpc::response_file("pane-info-response", "json");

    let payload = if let Some(steps) = previous {
        log::info!(
            "pane_info:cli: previous steps={steps} response_file={:?}",
            response_file
        );
        serde_json::json!({
            "type": "get_previous_pane_info",
            "response_file": response_file,
            "steps": steps,
        })
    } else {
        let pane_id_str = match std::env::var("PLEXI_PANE_ID") {
            Ok(v) => v,
            Err(_) => {
                eprintln!(
                    "error: PLEXI_PANE_ID is not set — run this inside a Plexi terminal pane"
                );
                return 1;
            }
        };
        let pane_id: u64 = match pane_id_str.parse() {
            Ok(n) => n,
            Err(_) => {
                eprintln!("error: PLEXI_PANE_ID is not a valid number: {pane_id_str}");
                return 1;
            }
        };
        log::info!(
            "pane_info:cli: pane_id={pane_id} response_file={:?}",
            response_file
        );
        serde_json::json!({
            "type": "get_pane_info",
            "pane_id": pane_id,
            "response_file": response_file,
        })
    };

    let code = send_to_socket(payload);
    if code != 0 {
        return code;
    }

    let content = match super::poll_rpc(&response_file, "pane info") {
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

/// `plexi pane close <pane_id>`
///
/// Sends a `close_pane` command to PLEXI_SOCKET. Fire-and-forget.
/// Returns 0 on success, 1 on error.
pub fn pane_close_cli(pane_id: u64) -> i32 {
    send_to_socket(serde_json::json!({
        "type": "close_pane",
        "pane_id": pane_id,
    }))
}

/// `plexi pane send <pane_id> <text>`
///
/// Writes text to a terminal pane's PTY stdin, or focuses an app pane and
/// injects one egui text-input event through the production render path.
/// Polls a response file to surface errors (e.g. pane not found) to the caller.
/// Returns 0 on success, 1 on error.
pub fn pane_send_cli(pane_id: u64, text: &str, submit: bool) -> i32 {
    let response_file = crate::rpc::response_file("send-to-pane-response", "json");
    log::info!(
        "pane_send:cli: pane_id={pane_id} len={} submit={submit} response_file={response_file:?}",
        text.len()
    );
    let code = send_to_socket(serde_json::json!({
        "type": "send_to_pane",
        "pane_id": pane_id,
        "text": text,
        "submit": submit,
        "response_file": response_file,
    }));
    if code != 0 {
        return code;
    }
    if submit {
        // The host settles, presses Enter and confirms before answering, all
        // within its own ceiling. This window sits above that so the caller
        // sees the host's typed outcome rather than a client-side timeout.
        let content = match super::poll_rpc_with(&response_file, "pane send", SUBMIT_REPLY_WINDOW) {
            Ok(content) => content,
            Err(code) => return code,
        };
        let outcome = submit_outcome(&content);
        outcome.report();
        return outcome.exit_code();
    }
    let content = match super::poll_rpc(&response_file, "pane send") {
        Ok(content) => content,
        Err(code) => return code,
    };
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
        if let Some(msg) = v.get("error").and_then(|v| v.as_str()) {
            eprintln!("error: {msg}");
            return 1;
        }
    }
    0
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
                    print_tip("the prompt was still parked as a collapsed paste; a second Enter submitted it");
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
    let response_file = crate::rpc::response_file("pane-key-response", "json");
    log::info!(
        "pane_key:cli: pane_id={pane_id} key={key:?} key_chars={} response_file={response_file:?}",
        key.chars().count()
    );
    let code = send_to_socket(serde_json::json!({
        "type": "key_pane",
        "pane_id": pane_id,
        "key": key,
        "response_file": response_file,
    }));
    if code != 0 {
        return code;
    }
    let content = match super::poll_rpc(&response_file, "pane key") {
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
        if let Some(msg) = v.get("error").and_then(|v| v.as_str()) {
            eprintln!("error: {msg}");
            return 1;
        }
    }
    0
}

pub fn pane_drop_cli(pane_id: u64, path_or_url: &str) -> i32 {
    let response_file = crate::rpc::response_file("pane-drop-response", "json");
    let code = send_to_socket(serde_json::json!({
        "type": "drop_file",
        "pane_id": pane_id,
        "path_or_url": path_or_url,
        "response_file": response_file,
    }));
    if code != 0 {
        return code;
    }
    let content = match super::poll_rpc(&response_file, "pane drop") {
        Ok(content) => content,
        Err(code) => return code,
    };
    if serde_json::from_str::<serde_json::Value>(&content)
        .ok()
        .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_owned))
        .is_some_and(|error| {
            eprintln!("error: {error}");
            true
        })
    {
        return 1;
    }
    print_json_output(&content)
}

/// Poll `response_file` for a `click_pane`/`click_pane_node` response,
/// shared by the pixel and node-targeted `plexi pane click` variants.
/// Returns 0 on success, 1 on error.
fn poll_click_response(response_file: &str, log_prefix: &str) -> i32 {
    let content = match super::poll_rpc(response_file, "pane click") {
        Ok(content) => content,
        Err(code) => return code,
    };
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
        if let Some(msg) = v.get("error").and_then(|v| v.as_str()) {
            log::warn!("{log_prefix}: host reported error: {msg}");
            eprintln!("error: {msg}");
            return 1;
        }
    }
    0
}

/// `plexi pane click <pane_id> <x> <y> [--button left]`
///
/// Sends `click_pane` command to PLEXI_SOCKET. Waits for response.
/// Returns 0 on success, 1 on error.
pub fn pane_click_cli(pane_id: u64, x: f32, y: f32, button: &str) -> i32 {
    let response_file = crate::rpc::response_file("pane-click-response", "json");
    log::info!(
        "pane_click:cli: pane_id={pane_id} x={x} y={y} button={button} response_file={response_file:?}"
    );
    let code = send_to_socket(serde_json::json!({
        "type": "click_pane",
        "pane_id": pane_id,
        "x": x,
        "y": y,
        "button": button,
        "response_file": response_file,
    }));
    if code != 0 {
        return code;
    }
    poll_click_response(&response_file, "pane_click:cli")
}

/// `plexi pane click <pane_id> --node <node_id> [--button left]`
///
/// Sends `click_pane_node` command to PLEXI_SOCKET. Waits for response.
/// Returns 0 on success, 1 on error.
pub fn pane_click_node_cli(pane_id: u64, node_id: &str, button: &str) -> i32 {
    let response_file = crate::rpc::response_file("pane-click-node-response", "json");
    log::info!(
        "pane_click_node:cli: pane_id={pane_id} node_id={node_id} button={button} response_file={response_file:?}"
    );
    let code = send_to_socket(serde_json::json!({
        "type": "click_pane_node",
        "pane_id": pane_id,
        "node_id": node_id,
        "button": button,
        "response_file": response_file,
    }));
    if code != 0 {
        return code;
    }
    poll_click_response(&response_file, "pane_click_node:cli")
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
    let id = uuid::Uuid::new_v4();
    let response_file = crate::config::config_dir()
        .join(format!("pane-drag-response-{id}.json"))
        .to_string_lossy()
        .into_owned();
    log::info!(
        "pane_drag:cli: pane_id={pane_id} from={from:?} from_node={from_node:?} to={to:?} \
         to_node={to_node:?} steps={steps} button={button} response_file={response_file:?}"
    );
    let code = send_to_socket(serde_json::json!({
        "type": "drag_pane",
        "pane_id": pane_id,
        "from": from,
        "from_node": from_node,
        "to": to,
        "to_node": to_node,
        "steps": steps,
        "button": button,
        "response_file": response_file,
    }));
    if code != 0 {
        return code;
    }
    poll_click_response(&response_file, "pane_drag:cli")
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
                eprintln!("error: pane_id not provided and PLEXI_PANE_ID is not set — run inside a Plexi pane or pass a pane ID");
                return 1;
            }
        },
    };

    let response_file = crate::rpc::response_file("pane-capture-response", "json");

    log::info!("pane_capture:cli: pane_id={resolved_pane_id} lines={lines} full_output={full_output} from_cursor={from_cursor:?} response_file={response_file:?}");

    let mut req = serde_json::json!({
        "type": "capture_pane",
        "pane_id": resolved_pane_id,
        "lines": lines,
        "full_output": full_output,
        "response_file": response_file,
    });
    if let Some(cursor) = from_cursor {
        req["from_cursor"] = serde_json::Value::Number(serde_json::Number::from(cursor));
    }
    let code = send_to_socket(req);
    if code != 0 {
        return code;
    }

    let content = match super::poll_rpc(&response_file, "pane capture") {
        Ok(content) => content,
        Err(code) => return code,
    };
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
        if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
            eprintln!("error: {err}");
            return 1;
        }
        // Print cursor to stderr so callers can capture it without
        // polluting the line stream.
        if let Some(cursor) = v.get("cursor").and_then(|c| c.as_u64()) {
            eprintln!("cursor={cursor}");
        }
    }
    print_json_output(&content)
}

/// `plexi pane state <id>`
///
/// Sends a `get_pane_state` command to PLEXI_SOCKET. For app panes, the host
/// writes a JSON object containing a versioned normalized `semantic` tree.
/// Process apps also retain the compatible `frame` RenderCommand array.
/// For terminal panes, returns a simple status object. Returns 0 on success, 1 on error.
pub fn pane_state_cli(pane_id: u64) -> i32 {
    let response_file = crate::rpc::response_file("pane-state-response", "json");

    log::info!("pane_state:cli: pane_id={pane_id} response_file={response_file:?}");

    let code = send_to_socket(serde_json::json!({
        "type": "get_pane_state",
        "pane_id": pane_id,
        "response_file": response_file,
    }));
    if code != 0 {
        return code;
    }

    let content = match super::poll_rpc(&response_file, "pane state") {
        Ok(content) => content,
        Err(code) => return code,
    };
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
        if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
            eprintln!("error: {err}");
            return 1;
        }
    }
    print_json_output(&content)
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
        .and_then(|d| crate::app::registry::resolve_workspace_root(d))
        .map(|p| p.to_string_lossy().into_owned());

    let abs_path = cache_dir.to_string_lossy().into_owned();
    log::info!(
        "open_github_ephemeral: launching from {abs_path} workspace_root={workspace_root:?}"
    );

    if super::command_socket_available() {
        let response_file = crate::rpc::response_file("spawn-pane-response", "json");
        let mut payload = serde_json::json!({
            "type": "spawn_pane",
            "type_id": "",
            "path": abs_path,
            "layout": layout,
            "response_file": response_file,
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
        log::info!("open_github_ephemeral: sending via socket response_file={response_file:?}");
        let code = send_to_socket(payload);
        if code != 0 {
            return code;
        }
        return super::open::wait_for_response(&response_file);
    }

    // Fallback: spawn-queue (outside a Plexi pane). Only queue when a host is
    // actually servicing this channel — never park a spawn into the void
    // (stint 0532).
    if let Err(code) = crate::cli::require_spawn_servicing_host("open github") {
        return code;
    }
    let queue_dir = crate::config::config_dir().join("spawn-queue");
    if let Err(e) = std::fs::create_dir_all(&queue_dir) {
        eprintln!("error: could not create spawn queue: {e}");
        return 1;
    }
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let queue_id = uuid::Uuid::new_v4();
    let mut queue_payload = serde_json::json!({
        "type_id": "",
        "path": abs_path,
        "layout": layout,
        "origin": "open github",
        "queued_at_ms": crate::cli::spawn_queued_at_ms(),
    });
    if let Some(ref ws) = workspace_root {
        queue_payload["workspace_root"] = serde_json::Value::String(ws.clone());
    }
    if let Some(cwd_str) = cwd {
        queue_payload["cwd"] = serde_json::Value::String(cwd_str.to_string());
    }
    let file = queue_dir.join(format!("{ts}-{queue_id}.json"));
    if let Err(e) = std::fs::write(&file, queue_payload.to_string()) {
        eprintln!("error: could not write spawn request: {e}");
        return 1;
    }
    crate::cli::nudge_running_instance();
    log::info!("open_github_ephemeral: queued path={abs_path}");
    println!("queued: open github:{owner}/{repo}");
    println!("(running outside a Plexi pane — Plexi will pick this up within a second)");
    0
}

#[cfg(test)]
mod slot_write_tests {
    use super::*;

    #[test]
    fn ack_replace_reports_byte_count() {
        assert_eq!(slot_write_ack("status", 4, false), "slot \"status\" <- 4 bytes");
    }

    #[test]
    fn ack_append_uses_append_arrow() {
        assert_eq!(slot_write_ack("status", 12, true), "slot \"status\" +<- 12 bytes");
    }

    #[test]
    fn empty_slot_name_exits_nonzero() {
        // Must fail before any socket contact — no PLEXI_PANE_ID needed.
        assert_eq!(pane_slot_write_cli("", Some("x"), false, true, Some(7)), 1);
        assert_eq!(pane_slot_write_cli("   ", Some("x"), false, true, Some(7)), 1);
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
        for submit in [true, false] {
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

            let code = pane_send_cli(42, "hello", submit);
            let payload = rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("host listener never received the send payload");
            handle.join().expect("listener thread");

            assert_eq!(code, 0, "submit={submit}");
            assert_eq!(payload["type"], "send_to_pane");
            assert_eq!(payload["submit"], submit, "payload was {payload}");
        }
    }
}
