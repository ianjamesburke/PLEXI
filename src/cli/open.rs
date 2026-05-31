use super::{send_to_socket};
use super::pane::open_github_ephemeral;
use std::io;

pub fn open_cli(type_id: &str, args: &[String], layout: Option<&str>, from_pane_id: Option<u64>, cwd: Option<&str>) -> i32 {
    // Intercept github: prefix for ephemeral open-without-install.
    if type_id.starts_with("github:") {
        return open_github_ephemeral(type_id, layout, from_pane_id, cwd);
    }

    if type_id == "terminal" {
        log::warn!("open:cli: 'plexi app open terminal' is deprecated, use 'plexi terminal' instead");
        eprintln!("warning: 'plexi app open terminal' is deprecated, use 'plexi terminal' instead");
    }

    let from_pane_id = from_pane_id.or_else(|| std::env::var("PLEXI_PANE_ID").ok()?.parse().ok());

    // Socket path is set when running inside a Plexi pane — use it directly so
    // the command reaches the correct running instance regardless of channel.
    if std::env::var("PLEXI_SOCKET").is_ok() {
        let id = uuid::Uuid::new_v4();
        let response_file = crate::config::config_dir()
            .join(format!("spawn-pane-response-{id}.json"))
            .to_string_lossy()
            .into_owned();
        let mut payload = serde_json::json!({
            "type": "spawn_pane",
            "type_id": type_id,
            "args": args,
            "layout": layout,
            "response_file": response_file,
        });
        if let Some(pid) = from_pane_id {
            payload["from_pane_id"] = serde_json::Value::Number(pid.into());
        }
        if let Some(cwd) = cwd {
            payload["cwd"] = serde_json::Value::String(cwd.to_string());
        }
        log::info!("open:cli: sending via socket from_pane_id={from_pane_id:?} cwd={cwd:?} response_file={response_file:?}");
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
                        // Fallback: print raw content
                        print!("{content}");
                        return 0;
                    }
                    Err(e) => {
                        log::warn!("open:cli: could not read response file: {e}");
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

    // Fallback: write to the spawn-queue for the channel this binary belongs to.
    if from_pane_id.is_some() {
        log::warn!("open:cli: --from-pane-id requires PLEXI_SOCKET (run inside a Plexi pane); ignoring");
        eprintln!("warning: --from-pane-id is ignored outside a Plexi pane");
    }
    let queue_dir = crate::config::config_dir().join("spawn-queue");
    if let Err(e) = std::fs::create_dir_all(&queue_dir) {
        eprintln!("error: could not create spawn queue: {e}");
        return 1;
    }
    let id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut queue_payload = serde_json::json!({
        "type_id": type_id,
        "args": args,
        "layout": layout,
    });
    if let Some(cwd) = cwd {
        queue_payload["cwd"] = serde_json::Value::String(cwd.to_string());
    }
    let file = queue_dir.join(format!("{id}.json"));
    if let Err(e) = std::fs::write(&file, queue_payload.to_string()) {
        eprintln!("error: could not write spawn request: {e}");
        return 1;
    }
    log::info!("cli: open queued: type_id={type_id} cwd={cwd:?}");
    println!("queued: open {type_id}");
    println!("(running outside a Plexi pane — Plexi will pick this up within a second)");
    0
}

/// `plexi terminal [cmd] [--ephemeral] [--layout=X] [--no-focus]`
///
/// Opens a terminal pane. Supports the --ephemeral flag which closes the pane when the
/// process exits, and --no-focus to keep focus on the originating pane.
pub fn terminal_cli(cmd: Option<&str>, ephemeral: bool, layout: Option<&str>, from_pane_id: Option<u64>, cwd: Option<&str>, no_focus: bool) -> i32 {
    let layout_str = layout.unwrap_or("split_v");
    let args: Vec<String> = cmd.map(|c| vec![c.to_string()]).unwrap_or_default();
    let from_pane_id = from_pane_id.or_else(|| std::env::var("PLEXI_PANE_ID").ok()?.parse().ok());

    if std::env::var("PLEXI_SOCKET").is_ok() {
        let id = uuid::Uuid::new_v4();
        let response_file = crate::config::config_dir()
            .join(format!("spawn-pane-response-{id}.json"))
            .to_string_lossy()
            .into_owned();
        let mut payload = serde_json::json!({
            "type": "spawn_pane",
            "type_id": "terminal",
            "args": args,
            "layout": layout_str,
            "response_file": response_file,
        });
        if ephemeral {
            payload["ephemeral"] = serde_json::Value::Bool(true);
        }
        if let Some(pid) = from_pane_id {
            payload["from_pane_id"] = serde_json::Value::Number(pid.into());
        }
        if let Some(cwd) = cwd {
            payload["cwd"] = serde_json::Value::String(cwd.to_string());
        }
        if no_focus {
            payload["no_focus"] = serde_json::Value::Bool(true);
        }
        log::info!("terminal:cli: sending via socket ephemeral={ephemeral} no_focus={no_focus} from_pane_id={from_pane_id:?} cwd={cwd:?} response_file={response_file:?}");
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
                            if let Some(pid) = v.get("pane_id").and_then(|v| v.as_u64()) {
                                println!("{pid}");
                                return 0;
                            }
                        }
                        print!("{content}");
                        return 0;
                    }
                    Err(e) => {
                        log::warn!("terminal:cli: could not read response file: {e}");
                        eprintln!("error: could not read response file: {e}");
                        return 1;
                    }
                }
            }
            if std::time::Instant::now() >= deadline {
                eprintln!("error: timed out waiting for terminal response");
                return 1;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }

    // Fallback: spawn-queue (outside a Plexi pane)
    if from_pane_id.is_some() {
        log::warn!("terminal:cli: --from-pane-id requires PLEXI_SOCKET (run inside a Plexi pane); ignoring");
        eprintln!("warning: --from-pane-id is ignored outside a Plexi pane");
    }
    let queue_dir = crate::config::config_dir().join("spawn-queue");
    if let Err(e) = std::fs::create_dir_all(&queue_dir) {
        eprintln!("error: could not create spawn queue: {e}");
        return 1;
    }
    let id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut queue_payload = serde_json::json!({
        "type_id": "terminal",
        "args": args,
        "layout": layout_str,
    });
    if ephemeral {
        queue_payload["ephemeral"] = serde_json::Value::Bool(true);
    }
    if let Some(cwd) = cwd {
        queue_payload["cwd"] = serde_json::Value::String(cwd.to_string());
    }
    if no_focus {
        queue_payload["no_focus"] = serde_json::Value::Bool(true);
    }
    let file = queue_dir.join(format!("{id}.json"));
    if let Err(e) = std::fs::write(&file, queue_payload.to_string()) {
        eprintln!("error: could not write spawn request: {e}");
        return 1;
    }
    log::info!("terminal:cli: queued ephemeral={ephemeral} no_focus={no_focus} cwd={cwd:?}");
    println!("queued: open terminal");
    println!("(running outside a Plexi pane — Plexi will pick this up within a second)");
    0
}

/// Read a line from stdin with echo disabled (for password-style input).
pub(super) fn read_secret_from_stdin() -> io::Result<String> {
    // Disable echo via stty (avoids libc dependency).
    let _ = std::process::Command::new("stty").arg("-echo").status();

    let result = read_line_plain();

    // Restore echo.
    let _ = std::process::Command::new("stty").arg("echo").status();
    // Print newline since echo was off during input.
    eprintln!();

    result
}

fn read_line_plain() -> io::Result<String> {
    let mut buf = String::new();
    io::stdin().read_line(&mut buf)?;
    Ok(buf
        .trim_end_matches('\n')
        .trim_end_matches('\r')
        .to_string())
}
