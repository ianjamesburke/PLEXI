/// Synchronous turn runner for the Claude CLI backend.
///
/// Uses `claude -p --output-format json` to get structured output including
/// the session_id needed for `--resume` on subsequent turns.
use std::path::Path;
use std::process::Command;

pub struct TurnResult {
    pub response: String,
    pub session_id: String,
}

/// Build an augmented PATH that includes common Claude CLI install locations.
/// macOS GUI bundles inherit a minimal PATH (/usr/bin:/bin) that misses
/// user-local installs like ~/.local/bin and /usr/local/bin.
fn augmented_path() -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    let extras = format!(
        "{home}/.local/bin:{home}/.claude/local/bin:/usr/local/bin:/opt/homebrew/bin"
    );
    match std::env::var("PATH") {
        Ok(p) => format!("{extras}:{p}"),
        Err(_) => format!("{extras}:/usr/bin:/bin"),
    }
}

pub fn run_turn(
    session_id: &str,
    message: &str,
    cwd: &Path,
    soul_context: Option<String>,
) -> Result<TurnResult, String> {
    let path = augmented_path();

    let found = Command::new("which")
        .arg("claude")
        .env("PATH", &path)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !found {
        return Err(format!(
            "claude not found on PATH ({path}) — install Claude Code CLI"
        ));
    }

    let full_message = match soul_context {
        Some(ctx) if !ctx.is_empty() => format!("{ctx}{message}"),
        _ => message.to_string(),
    };

    let mut cmd = Command::new("claude");
    cmd.env("PATH", &path);
    cmd.arg("-p");
    cmd.arg("--output-format");
    cmd.arg("json");
    if !session_id.is_empty() {
        cmd.arg("--resume");
        cmd.arg(session_id);
    }
    cmd.arg(&full_message);
    cmd.current_dir(cwd);

    let output = cmd.output().map_err(|e| format!("Failed to spawn claude: {e}"))?;

    let raw = String::from_utf8_lossy(&output.stdout).into_owned();
    parse_json_output(&raw, session_id)
}

/// Parse the JSON output from `claude -p --output-format json`.
/// Extracts `result` (response text) and `session_id`.
fn parse_json_output(raw: &str, existing_session_id: &str) -> Result<TurnResult, String> {
    let v: serde_json::Value = serde_json::from_str(raw.trim())
        .map_err(|e| format!("Failed to parse claude JSON output: {e}\nRaw: {raw}"))?;

    let session_id = v["session_id"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or(existing_session_id)
        .to_string();

    // `is_error` means the API returned an error (e.g. credit balance) but
    // the CLI itself exited cleanly. Surface the result text as an error.
    if v["is_error"].as_bool().unwrap_or(false) {
        let msg = v["result"].as_str().unwrap_or("unknown error").to_string();
        return Err(msg);
    }

    let response = v["result"].as_str().unwrap_or("").trim().to_string();
    Ok(TurnResult { response, session_id })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_success_response() {
        let raw = r#"{"type":"result","subtype":"success","is_error":false,"result":"Hello!","session_id":"abc-123"}"#;
        let r = parse_json_output(raw, "").unwrap();
        assert_eq!(r.session_id, "abc-123");
        assert_eq!(r.response, "Hello!");
    }

    #[test]
    fn parse_error_response() {
        let raw = r#"{"type":"result","is_error":true,"result":"Credit balance is too low","session_id":"fe56a3d8"}"#;
        let err = parse_json_output(raw, "").unwrap_err();
        assert!(err.contains("Credit balance"));
    }

    #[test]
    fn fallback_to_existing_session_id() {
        let raw = r#"{"type":"result","is_error":false,"result":"Hi","session_id":""}"#;
        let r = parse_json_output(raw, "old-id").unwrap();
        assert_eq!(r.session_id, "old-id");
    }
}
