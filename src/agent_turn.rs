/// Synchronous turn runner for the Claude CLI backend.
///
/// Calls `claude -p --resume <session_id> "<message>"` as a subprocess.
/// On the first turn (no session_id), runs without `--resume` and parses
/// the new session ID from the output header that the Claude CLI prints.
use std::path::Path;
use std::process::Command;

/// Result of a completed turn.
pub struct TurnResult {
    /// The assistant's response text.
    pub response: String,
    /// The session ID to persist (may be the same as the one passed in, or
    /// a freshly-allocated one for the first turn).
    pub session_id: String,
}

/// Run one conversational turn against the Claude CLI.
///
/// - `session_id`: empty string on the first turn; non-empty to resume.
/// - `message`: the user's message text.
/// - `cwd`: working directory for the subprocess.
///
/// Returns `Ok(TurnResult)` on success, `Err(String)` on failure.
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

pub fn run_turn(session_id: &str, message: &str, cwd: &Path) -> Result<TurnResult, String> {
    let path = augmented_path();

    // Locate claude before spawning so we give a clear error if absent.
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

    let mut cmd = Command::new("claude");
    cmd.env("PATH", &path);
    cmd.arg("-p");
    if !session_id.is_empty() {
        cmd.arg("--resume");
        cmd.arg(session_id);
    }
    cmd.arg(message);
    cmd.current_dir(cwd);

    let output = cmd.output().map_err(|e| format!("Failed to spawn claude: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("claude exited with error: {stderr}"));
    }

    let raw = String::from_utf8_lossy(&output.stdout).into_owned();

    // The Claude CLI `--print` mode emits the response directly. When a new
    // session is created the CLI writes a header line:
    //   > Session ID: <id>
    // before the response body. We parse that out if present.
    let (new_session_id, response) = parse_output(&raw, session_id);

    Ok(TurnResult {
        response,
        session_id: new_session_id,
    })
}

/// Extract the session ID header (if present) and return (session_id, body).
///
/// Claude CLI `-p` mode may print a line like:
///   `> Session ID: abc123`
/// as the first line of stdout. We strip it and use it as the session ID.
/// If no such line is found, we keep the caller's existing `session_id`.
fn parse_output(raw: &str, existing_session_id: &str) -> (String, String) {
    let session_prefix = "> Session ID: ";
    let mut lines = raw.lines();
    if let Some(first) = lines.next() {
        if let Some(id) = first.strip_prefix(session_prefix) {
            let body: Vec<&str> = lines.collect();
            return (id.trim().to_string(), body.join("\n").trim().to_string());
        }
    }
    (existing_session_id.to_string(), raw.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_output_with_session_header() {
        let raw = "> Session ID: abc123\nHello, world!";
        let (id, body) = parse_output(raw, "old");
        assert_eq!(id, "abc123");
        assert_eq!(body, "Hello, world!");
    }

    #[test]
    fn parse_output_without_session_header() {
        let raw = "Hello, world!";
        let (id, body) = parse_output(raw, "existing-id");
        assert_eq!(id, "existing-id");
        assert_eq!(body, "Hello, world!");
    }

    #[test]
    fn parse_output_multiline_body() {
        let raw = "> Session ID: new-id\nLine one\nLine two\n";
        let (id, body) = parse_output(raw, "old");
        assert_eq!(id, "new-id");
        assert_eq!(body, "Line one\nLine two");
    }
}
