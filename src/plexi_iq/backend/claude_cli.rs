//! Proxied mode — `claude -p --resume --output-format stream-json` subprocess.
//!
//! Claude Code owns the tool loop. Plexi IQ receives only the assistant's
//! text output. `supports_tool_dispatch()` returns `false`; the turn loop
//! skips tool-schema injection and tool_use block collection.
//!
//! The streaming logic is adapted from `src/agent_llm.rs` but routed
//! through the `LlmBackend` trait so `PlexiIqInstance` can swap backends
//! without knowing which one is active.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;

use super::{BillingModel, LlmBackend, LlmError, LlmRequest, StreamEvent};

/// `claude -p --resume` subprocess backend.
#[derive(Debug, Default)]
pub struct ClaudeCliBackend {}

impl ClaudeCliBackend {
    pub fn new() -> Self {
        Self::default()
    }
}

impl LlmBackend for ClaudeCliBackend {
    fn name(&self) -> &str {
        "claude-cli (proxied)"
    }

    fn supports_tool_dispatch(&self) -> bool {
        // Claude Code owns the tool loop in proxied mode.
        false
    }

    fn billing_model(&self) -> BillingModel {
        BillingModel::Subscription
    }

    fn stream_to_channel(
        &self,
        request: LlmRequest,
        tx: mpsc::Sender<StreamEvent>,
    ) -> Result<(), LlmError> {
        let claude_bin = find_claude_binary().ok_or_else(|| {
            LlmError::NotAvailable(
                "claude CLI not found — install Claude Code (https://claude.ai/code)".to_string(),
            )
        })?;

        thread::Builder::new()
            .name("plexi-iq-cli".to_string())
            .spawn(move || stream_worker(claude_bin, request, tx))
            .map_err(|e| LlmError::Io(format!("failed to spawn stream worker thread: {e}")))?;

        Ok(())
    }
}

/// Worker thread: spawns `claude -p`, streams JSON events, sends `StreamEvent`s.
fn stream_worker(claude_bin: String, request: LlmRequest, tx: mpsc::Sender<StreamEvent>) {
    let mut cmd = Command::new(&claude_bin);
    cmd.arg("-p");
    cmd.args(["--output-format", "stream-json"]);
    cmd.arg("--verbose");
    // Disable all tools — GUI subprocess context has no TTY for permission prompts.
    // Tool calls produce empty assistant events → silent response.
    cmd.args(["--allowedTools", ""]);

    if request.session_id.is_none() && !request.system.is_empty() {
        cmd.args(["--system-prompt", &request.system]);
    }

    if let Some(ref sid) = request.session_id {
        cmd.args(["--resume", sid]);
    }

    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(StreamEvent::Error(format!(
                "failed to spawn claude CLI ({claude_bin}): {e}"
            )));
            return;
        }
    };

    // Write prompt and close stdin.
    if let Some(mut stdin) = child.stdin.take() {
        if let Err(e) = stdin.write_all(request.prompt.as_bytes()) {
            log::error!("plexi_iq: failed to write prompt to claude stdin: {e}");
        }
        // Drop closes the pipe.
    }

    // Forward stderr to the log on a background thread.
    if let Some(stderr) = child.stderr.take() {
        thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines().flatten() {
                if !line.trim().is_empty() {
                    log::warn!("plexi_iq claude-cli stderr: {line}");
                }
            }
        });
    }

    let stdout = match child.stdout.take() {
        Some(s) => s,
        None => {
            let _ = tx.send(StreamEvent::Error(
                "failed to capture claude CLI stdout".to_string(),
            ));
            return;
        }
    };

    let reader = BufReader::new(stdout);
    let mut captured_session_id: Option<String> = None;

    for line_result in reader.lines() {
        let line = match line_result {
            Ok(l) => l,
            Err(e) => {
                log::warn!("plexi_iq: error reading claude stdout: {e}");
                break;
            }
        };
        if line.is_empty() {
            continue;
        }

        match parse_cli_event(&line) {
            CliEvent::Text(chunk) => {
                if tx.send(StreamEvent::Text(chunk)).is_err() {
                    // Receiver dropped — caller cancelled.
                    return;
                }
            }
            CliEvent::SessionId(sid) => {
                captured_session_id = Some(sid);
            }
            CliEvent::Done => break,
            CliEvent::Unknown => {}
        }
    }

    // Reap the child.
    if let Err(e) = child.wait() {
        log::warn!("plexi_iq: error waiting for claude process: {e}");
    }

    let _ = tx.send(StreamEvent::Done {
        input_tokens: None,
        output_tokens: None,
        session_id: captured_session_id,
    });
}

enum CliEvent {
    Text(String),
    SessionId(String),
    Done,
    Unknown,
}

/// Parse a single stream-json line from the `claude -p` subprocess.
fn parse_cli_event(line: &str) -> CliEvent {
    let Ok(val) = serde_json::from_str::<serde_json::Value>(line) else {
        return CliEvent::Unknown;
    };

    match val.get("type").and_then(|t| t.as_str()) {
        Some("assistant") => {
            // assistant event: { "type": "assistant", "message": { "content": [...] } }
            let text = val
                .pointer("/message/content")
                .and_then(|c| c.as_array())
                .and_then(|arr| {
                    arr.iter()
                        .filter_map(|block| {
                            if block.get("type").and_then(|t| t.as_str()) == Some("text") {
                                block.get("text").and_then(|t| t.as_str()).map(|s| s.to_string())
                            } else {
                                None
                            }
                        })
                        .reduce(|mut a, b| {
                            a.push_str(&b);
                            a
                        })
                })
                .unwrap_or_default();

            if text.is_empty() {
                CliEvent::Unknown
            } else {
                CliEvent::Text(text)
            }
        }
        Some("result") => {
            // result event: { "type": "result", "session_id": "...", "is_error": false }
            let session_id = val
                .get("session_id")
                .and_then(|s| s.as_str())
                .map(|s| s.to_string());
            if let Some(sid) = session_id {
                CliEvent::SessionId(sid)
            } else {
                CliEvent::Done
            }
        }
        _ => CliEvent::Unknown,
    }
}

/// Locate the `claude` binary. macOS GUI app bundles don't inherit shell PATH,
/// so we probe known install paths before falling back to PATH resolution.
fn find_claude_binary() -> Option<String> {
    let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("/"));
    let local_bin = home.join(".local/bin/claude").to_string_lossy().into_owned();

    let candidates = [
        local_bin.as_str(),
        "/usr/local/bin/claude",
        "/opt/homebrew/bin/claude",
        "/usr/local/share/npm/bin/claude",
    ];

    for candidate in &candidates {
        if std::path::Path::new(candidate).exists() {
            return Some(candidate.to_string());
        }
    }

    // Try `which claude` to handle shell-managed paths (nvm, volta, etc.).
    if let Ok(output) = Command::new("which").arg("claude").output() {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return Some(path);
            }
        }
    }

    // Last resort: let the OS resolve it. If not found, spawn returns NotFound.
    Some("claude".to_string())
}
