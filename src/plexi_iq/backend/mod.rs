/// Plexi IQ backend — claude -p --resume subprocess driver.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

/// Invoke `claude -p --resume <session_id>` with the given prompt.
/// Writes prompt to stdin, reads response from stdout.
/// This is a blocking call — run in a spawned thread if needed.
pub async fn run_claude_proxy(
    session_id: &str,
    prompt: &str,
    workspace_dir: &Path,
) -> Result<String, String> {
    let mut child = Command::new("claude")
        .arg("-p")
        .arg("--resume")
        .arg(session_id)
        .current_dir(workspace_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to spawn claude: {e}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(prompt.as_bytes())
            .map_err(|e| format!("stdin write: {e}"))?;
        stdin
            .write_all(b"\n")
            .map_err(|e| format!("stdin newline: {e}"))?;
    }

    let output = child
        .wait_with_output()
        .map_err(|e| format!("wait: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        log::warn!("plexi_iq: claude exited non-zero: {stderr}");
    }

    let text = String::from_utf8_lossy(&output.stdout).to_string();
    Ok(text.trim().to_string())
}
