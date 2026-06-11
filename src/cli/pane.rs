use super::send_to_socket;

pub fn pane_set_title_cli(pane_id: Option<u64>, name: &str) -> i32 {
    let socket_path = match std::env::var("PLEXI_SOCKET") {
        Ok(v) => v,
        Err(_) => {
            eprintln!("error: PLEXI_SOCKET is not set — run this inside a Plexi terminal pane");
            return 1;
        }
    };
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
    let payload = serde_json::json!({
        "type": "set_pane_title",
        "pane_id": resolved_pane_id,
        "name": name,
    });
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    let mut stream = match UnixStream::connect(&socket_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: could not connect to PLEXI_SOCKET {socket_path:?}: {e}");
            return 1;
        }
    };
    let line = format!("{}\n", payload);
    if let Err(e) = stream.write_all(line.as_bytes()) {
        eprintln!("error: could not write to socket: {e}");
        return 1;
    }
    0
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

fn response_file(prefix: &str) -> String {
    let id = uuid::Uuid::new_v4();
    crate::config::config_dir()
        .join(format!("{prefix}-{id}.json"))
        .to_string_lossy()
        .into_owned()
}

fn wait_for_response_bytes(response_file: &str, label: &str) -> Result<Vec<u8>, i32> {
    let response_path = std::path::PathBuf::from(response_file);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if response_path.exists() {
            match std::fs::read(&response_path) {
                Ok(content) => {
                    let _ = std::fs::remove_file(&response_path);
                    return Ok(content);
                }
                Err(e) => {
                    log::warn!("{label}:cli: could not read response file: {e}");
                    eprintln!("error: could not read response file: {e}");
                    return Err(1);
                }
            }
        }
        if std::time::Instant::now() >= deadline {
            eprintln!("error: timed out waiting for {label} response");
            return Err(1);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

fn wait_for_slot_read_response(response_file: &str) -> Result<Vec<u8>, i32> {
    let response_path = std::path::PathBuf::from(response_file);
    let error_path = std::path::PathBuf::from(format!("{response_file}.err"));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if error_path.exists() {
            let content = match std::fs::read(&error_path) {
                Ok(content) => {
                    let _ = std::fs::remove_file(&error_path);
                    content
                }
                Err(e) => {
                    log::warn!("pane_slot_read:cli: could not read error response file: {e}");
                    eprintln!("error: could not read response file: {e}");
                    return Err(1);
                }
            };
            if let Some(err) = response_error(&content) {
                eprintln!("error: {err}");
                return Err(1);
            }
            eprintln!("error: invalid slot read error response");
            return Err(1);
        }
        if response_path.exists() {
            match std::fs::read(&response_path) {
                Ok(content) => {
                    let _ = std::fs::remove_file(&response_path);
                    return Ok(content);
                }
                Err(e) => {
                    log::warn!("pane_slot_read:cli: could not read response file: {e}");
                    eprintln!("error: could not read response file: {e}");
                    return Err(1);
                }
            }
        }
        if std::time::Instant::now() >= deadline {
            eprintln!("error: timed out waiting for pane slot read response");
            return Err(1);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

fn response_error(content: &[u8]) -> Option<String> {
    let value = serde_json::from_slice::<serde_json::Value>(content).ok()?;
    if value.get("ok").and_then(|v| v.as_bool()) != Some(false) {
        return None;
    }
    value.get("error")
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

/// `plexi pane slot write <name> [content] [pane_id]`
pub fn pane_slot_write_cli(
    name: &str,
    content: Option<&str>,
    append: bool,
    replace: bool,
    pane_id: Option<u64>,
) -> i32 {
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
    let response_file = response_file("pane-slot-write-response");
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
    0
}

/// `plexi pane slot read <name> [pane_id]`
pub fn pane_slot_read_cli(name: &str, pane_id: Option<u64>) -> i32 {
    let resolved_pane_id = match resolve_pane_id(pane_id) {
        Ok(id) => id,
        Err(code) => return code,
    };
    let response_file = response_file("pane-slot-read-response");
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

/// `plexi pane slot list [pane_id]`
pub fn pane_slot_list_cli(pane_id: Option<u64>) -> i32 {
    let resolved_pane_id = match resolve_pane_id(pane_id) {
        Ok(id) => id,
        Err(code) => return code,
    };
    let response_file = response_file("pane-slot-list-response");
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
    let response_file = response_file("pane-slot-delete-response");
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

    let id = uuid::Uuid::new_v4();
    let response_file = crate::config::config_dir()
        .join(format!("pane-list-response-{id}.json"))
        .to_string_lossy()
        .into_owned();

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
                    log::warn!("pane_list:cli: could not read response file: {e}");
                    eprintln!("error: could not read response file: {e}");
                    return 1;
                }
            }
        }
        if std::time::Instant::now() >= deadline {
            eprintln!("error: timed out waiting for pane list response");
            return 1;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
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

/// `plexi pane info [--previous]`
///
/// Sends a `get_pane_info` or `get_previous_pane_info` command to PLEXI_SOCKET.
/// Merges in client-side fields (socket, channel) and pretty-prints the result
/// as JSON. Returns 0 on success, 1 on error.
pub fn pane_info_cli(previous: bool) -> i32 {
    let socket_path = match std::env::var("PLEXI_SOCKET") {
        Ok(v) => v,
        Err(_) => {
            eprintln!("error: PLEXI_SOCKET is not set — run this inside a Plexi terminal pane");
            return 1;
        }
    };

    let id = uuid::Uuid::new_v4();
    let response_file = crate::config::config_dir()
        .join(format!("pane-info-response-{id}.json"))
        .to_string_lossy()
        .into_owned();

    let payload = if previous {
        log::info!(
            "pane_info:cli: previous=true response_file={:?}",
            response_file
        );
        serde_json::json!({
            "type": "get_previous_pane_info",
            "response_file": response_file,
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

    let response_path = std::path::PathBuf::from(&response_file);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if response_path.exists() {
            match std::fs::read_to_string(&response_path) {
                Ok(content) => {
                    let _ = std::fs::remove_file(&response_path);
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
                        if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
                            eprintln!("error: {err}");
                            return 1;
                        }
                        let mut obj = v;
                        obj["socket"] = serde_json::Value::String(socket_path.clone());
                        let channel =
                            crate::config::build_channel().unwrap_or_else(|| "main".to_string());
                        obj["channel"] = serde_json::Value::String(channel);
                        match serde_json::to_string(&obj) {
                            Ok(json_str) => {
                                return print_json_output(&json_str);
                            }
                            Err(e) => {
                                eprintln!("error: could not serialize response: {e}");
                                return 1;
                            }
                        }
                    } else {
                        eprintln!("error: invalid JSON from host: {content}");
                        return 1;
                    }
                }
                Err(e) => {
                    log::warn!("pane_info:cli: could not read response file: {e}");
                    eprintln!("error: could not read response file: {e}");
                    return 1;
                }
            }
        }
        if std::time::Instant::now() >= deadline {
            eprintln!("error: timed out waiting for pane info response");
            return 1;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
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
/// Writes text to the target pane's PTY stdin. Polls a response file to
/// surface errors (e.g. pane not found) back to the caller.
/// Returns 0 on success, 1 on error.
pub fn pane_send_cli(pane_id: u64, text: &str) -> i32 {
    let id = uuid::Uuid::new_v4();
    let response_file = crate::config::config_dir()
        .join(format!("send-to-pane-response-{id}.json"))
        .to_string_lossy()
        .into_owned();
    log::info!(
        "pane_send:cli: pane_id={pane_id} len={} response_file={response_file:?}",
        text.len()
    );
    let code = send_to_socket(serde_json::json!({
        "type": "send_to_pane",
        "pane_id": pane_id,
        "text": text,
        "response_file": response_file,
    }));
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
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
                        if v.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
                            return 0;
                        }
                        if let Some(msg) = v.get("error").and_then(|v| v.as_str()) {
                            eprintln!("error: {msg}");
                            return 1;
                        }
                    }
                    return 0;
                }
                Err(e) => {
                    log::warn!("pane_send:cli: could not read response file: {e}");
                    eprintln!("error: could not read response file: {e}");
                    return 1;
                }
            }
        }
        if std::time::Instant::now() >= deadline {
            eprintln!("error: timed out waiting for pane send response");
            return 1;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// `plexi pane key <pane_id> <key>`
///
/// Sends `key_pane` command to PLEXI_SOCKET. Waits for response.
/// Returns 0 on success, 1 on error.
pub fn pane_key_cli(pane_id: u64, key: &str) -> i32 {
    let id = uuid::Uuid::new_v4();
    let response_file = crate::config::config_dir()
        .join(format!("pane-key-response-{id}.json"))
        .to_string_lossy()
        .into_owned();
    log::info!("pane_key:cli: pane_id={pane_id} key={key:?} response_file={response_file:?}");
    let code = send_to_socket(serde_json::json!({
        "type": "key_pane",
        "pane_id": pane_id,
        "key": key,
        "response_file": response_file,
    }));
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
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
                        if v.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
                            return 0;
                        }
                        if let Some(msg) = v.get("error").and_then(|v| v.as_str()) {
                            eprintln!("error: {msg}");
                            return 1;
                        }
                    }
                    return 0;
                }
                Err(e) => {
                    log::warn!("pane_key:cli: could not read response file: {e}");
                    eprintln!("error: could not read response file: {e}");
                    return 1;
                }
            }
        }
        if std::time::Instant::now() >= deadline {
            eprintln!("error: timed out waiting for pane key response");
            return 1;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
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

    let id = uuid::Uuid::new_v4();
    let response_file = crate::config::config_dir()
        .join(format!("pane-capture-response-{id}.json"))
        .to_string_lossy()
        .into_owned();

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

    let response_path = std::path::PathBuf::from(&response_file);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if response_path.exists() {
            match std::fs::read_to_string(&response_path) {
                Ok(content) => {
                    let _ = std::fs::remove_file(&response_path);
                    match serde_json::from_str::<serde_json::Value>(&content) {
                        Ok(v) => {
                            if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
                                eprintln!("error: {err}");
                                return 1;
                            }
                            // Print cursor to stderr so callers can capture it without
                            // polluting the line stream.
                            if let Some(cursor) = v.get("cursor").and_then(|c| c.as_u64()) {
                                eprintln!("cursor={cursor}");
                            }
                            return print_json_output(&content);
                        }
                        Err(_) => return print_json_output(&content),
                    }
                }
                Err(e) => {
                    log::warn!("pane_capture:cli: could not read response file: {e}");
                    eprintln!("error: could not read response file: {e}");
                    return 1;
                }
            }
        }
        if std::time::Instant::now() >= deadline {
            eprintln!("error: timed out waiting for pane capture response");
            return 1;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// `plexi pane state <id>`
///
/// Sends a `get_pane_state` command to PLEXI_SOCKET. For app panes, the host
/// writes a JSON object containing the last-rendered frame (RenderCommand array).
/// For terminal panes, returns a simple status object. Returns 0 on success, 1 on error.
pub fn pane_state_cli(pane_id: u64) -> i32 {
    let id = uuid::Uuid::new_v4();
    let response_file = crate::config::config_dir()
        .join(format!("pane-state-response-{id}.json"))
        .to_string_lossy()
        .into_owned();

    log::info!("pane_state:cli: pane_id={pane_id} response_file={response_file:?}");

    let code = send_to_socket(serde_json::json!({
        "type": "get_pane_state",
        "pane_id": pane_id,
        "response_file": response_file,
    }));
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
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
                        if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
                            eprintln!("error: {err}");
                            return 1;
                        }
                    }
                    return print_json_output(&content);
                }
                Err(e) => {
                    log::warn!("pane_state:cli: could not read response file: {e}");
                    eprintln!("error: could not read response file: {e}");
                    return 1;
                }
            }
        }
        if std::time::Instant::now() >= deadline {
            eprintln!("error: timed out waiting for pane state response");
            return 1;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
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

    if std::env::var("PLEXI_SOCKET").is_ok() {
        let id = uuid::Uuid::new_v4();
        let response_file = crate::config::config_dir()
            .join(format!("spawn-pane-response-{id}.json"))
            .to_string_lossy()
            .into_owned();
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
        let response_path = std::path::PathBuf::from(&response_file);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if response_path.exists() {
                match std::fs::read_to_string(&response_path) {
                    Ok(content) => {
                        let _ = std::fs::remove_file(&response_path);
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
                            if let Some(msg) = v.get("error").and_then(|v| v.as_str()) {
                                eprintln!("error: {msg}");
                                return 1;
                            }
                            if let Some(pid) = v.get("pane_id").and_then(|v| v.as_u64()) {
                                println!("{pid}");
                                return 0;
                            }
                        }
                        print!("{content}");
                        return 0;
                    }
                    Err(e) => {
                        eprintln!("error: could not read response file: {e}");
                        return 1;
                    }
                }
            }
            if std::time::Instant::now() >= deadline {
                eprintln!("error: timed out waiting for open response");
                return 1;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }

    // Fallback: spawn-queue (outside a Plexi pane).
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
    log::info!("open_github_ephemeral: queued path={abs_path}");
    println!("queued: open github:{owner}/{repo}");
    println!("(running outside a Plexi pane — Plexi will pick this up within a second)");
    0
}
