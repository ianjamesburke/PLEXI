use super::{send_to_socket};
use super::pane::open_github_ephemeral;
use std::io;

/// Poll a response file until it appears (or timeout). Shared by all spawn paths.
fn wait_for_response(response_file: &str) -> i32 {
    let response_path = std::path::PathBuf::from(response_file);
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
                    log::warn!("pane_new: could not read response file: {e}");
                    eprintln!("error: could not read response file: {e}");
                    return 1;
                }
            }
        }
        if std::time::Instant::now() >= deadline {
            eprintln!("error: timed out waiting for pane response");
            return 1;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// Unified pane spawning. All CLI spawn paths funnel through here.
pub fn pane_new_cli(
    cmd: Option<&str>,
    name: Option<&str>,
    layout: &str,
    from_pane_id: Option<u64>,
    cwd: Option<&str>,
    ephemeral: bool,
    no_focus: bool,
    app: Option<&str>,
    mcp: &[String],
    cli_tool: Option<&str>,
    extra_args: &[String],
) -> i32 {
    // --cli mode: run help parser, then open as descriptor-renderer app
    if let Some(binary) = cli_tool {
        log::info!("pane_new:cli: running --help parser for `{binary}`");
        match crate::cli::help_parser::parse_help_to_descriptor(binary) {
            Ok(json) => {
                let id = uuid::Uuid::new_v4();
                let tmp = std::env::temp_dir().join(format!("plexi-descriptor-{id}.json"));
                if let Err(e) = std::fs::write(&tmp, &json) {
                    eprintln!("error: could not write descriptor temp file: {e}");
                    return 1;
                }
                let path = tmp.to_string_lossy().to_string();
                // Recurse as an app open with descriptor-renderer
                return pane_new_cli(
                    None, name, layout, from_pane_id, cwd,
                    ephemeral, no_focus, Some("descriptor-renderer"), &[], None, &[path],
                );
            }
            Err(e) => {
                eprintln!("error: could not parse --help output for `{binary}`: {e}");
                return 1;
            }
        }
    }

    // Determine mode: app or terminal
    let is_app = app.is_some() || !mcp.is_empty();
    let type_id = if let Some(a) = app {
        a.to_string()
    } else if !mcp.is_empty() {
        "mcp-renderer".to_string()
    } else {
        "terminal".to_string()
    };

    let args: Vec<String> = if !mcp.is_empty() {
        mcp.to_vec()
    } else if is_app {
        extra_args.to_vec()
    } else if let Some(c) = cmd {
        let command = std::iter::once(c)
            .chain(extra_args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ");
        vec![command]
    } else {
        Vec::new()
    };

    let from_pane_id = from_pane_id.or_else(|| std::env::var("PLEXI_PANE_ID").ok()?.parse().ok());

    // Socket path — inside a Plexi pane
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
        if let Some(n) = name {
            payload["name"] = serde_json::Value::String(n.to_string());
        }
        log::info!("pane_new:cli: sending via socket type_id={type_id} name={name:?} ephemeral={ephemeral} no_focus={no_focus} from_pane_id={from_pane_id:?} cwd={cwd:?} response_file={response_file:?}");
        let code = send_to_socket(payload);
        if code != 0 {
            return code;
        }
        return wait_for_response(&response_file);
    }

    // Fallback: spawn-queue (outside a Plexi pane)
    if from_pane_id.is_some() {
        log::warn!("pane_new:cli: --from-pane-id requires PLEXI_SOCKET (run inside a Plexi pane); ignoring");
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
    if ephemeral {
        queue_payload["ephemeral"] = serde_json::Value::Bool(true);
    }
    if let Some(cwd) = cwd {
        queue_payload["cwd"] = serde_json::Value::String(cwd.to_string());
    }
    if no_focus {
        queue_payload["no_focus"] = serde_json::Value::Bool(true);
    }
    if let Some(n) = name {
        queue_payload["name"] = serde_json::Value::String(n.to_string());
    }
    let file = queue_dir.join(format!("{id}.json"));
    if let Err(e) = std::fs::write(&file, queue_payload.to_string()) {
        eprintln!("error: could not write spawn request: {e}");
        return 1;
    }
    log::info!("pane_new:cli: queued type_id={type_id} name={name:?} ephemeral={ephemeral} no_focus={no_focus} cwd={cwd:?}");
    println!("queued: open {type_id}");
    println!("(running outside a Plexi pane — Plexi will pick this up within a second)");
    0
}

/// Thin wrapper preserving the existing `plexi app open` call site.
pub fn open_cli(type_id: &str, args: &[String], layout: Option<&str>, from_pane_id: Option<u64>, cwd: Option<&str>) -> i32 {
    // Intercept github: prefix for ephemeral open-without-install.
    if type_id.starts_with("github:") {
        return open_github_ephemeral(type_id, layout, from_pane_id, cwd);
    }

    if type_id == "terminal" {
        log::warn!("open:cli: 'plexi app open terminal' is deprecated, use 'plexi terminal' instead");
        eprintln!("warning: 'plexi app open terminal' is deprecated, use 'plexi terminal' instead");
    }

    let layout_str = layout.unwrap_or("split_h");
    pane_new_cli(None, None, layout_str, from_pane_id, cwd, false, false, Some(type_id), &[], None, args)
}

/// Thin wrapper preserving the existing `plexi terminal` call site.
pub fn terminal_cli(cmd: Option<&str>, ephemeral: bool, layout: Option<&str>, from_pane_id: Option<u64>, cwd: Option<&str>, no_focus: bool) -> i32 {
    let layout_str = layout.unwrap_or("split_h");
    pane_new_cli(cmd, None, layout_str, from_pane_id, cwd, ephemeral, no_focus, None, &[], None, &[])
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
