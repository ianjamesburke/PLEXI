/// Synchronous turn runner for the Claude CLI backend.
///
/// Uses `claude -p --output-format stream-json --verbose` to stream responses
/// line by line. Partial text chunks and tool-use events are sent back via
/// a `Sender<WorkerEvent>` so the UI can update in real time.
///
/// `child_slot` is shared with the owning AgentPane so it can kill the subprocess
/// mid-turn for interruption.
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::sync::mpsc;

pub struct TurnResult {
    pub response: String,
    pub session_id: String,
}

/// Events streamed back to the UI during a turn.
pub enum WorkerEvent {
    /// Accumulated assistant text so far — replace the current in-progress line.
    Chunk(String),
    /// A tool the agent is using, shown as a status line.
    ToolUse { name: String, input_preview: String },
    /// Turn complete (success or error).
    Done(Result<TurnResult, String>),
}

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
    event_tx: mpsc::SyncSender<WorkerEvent>,
    child_slot: Arc<Mutex<Option<std::process::Child>>>,
) -> Result<(), String> {
    let path = augmented_path();

    let found = Command::new("which")
        .arg("claude")
        .env("PATH", &path)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !found {
        let _ = event_tx.send(WorkerEvent::Done(Err(
            "claude not found on PATH — install Claude Code CLI".into(),
        )));
        return Ok(());
    }

    let full_message = match soul_context {
        Some(ctx) if !ctx.is_empty() => format!("{ctx}{message}"),
        _ => message.to_string(),
    };

    let mut cmd = Command::new("claude");
    cmd.env("PATH", &path)
        .arg("-p")
        .arg("--output-format")
        .arg("stream-json")
        .arg("--verbose");
    if !session_id.is_empty() {
        cmd.arg("--resume");
        cmd.arg(session_id);
    }
    cmd.arg(&full_message)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    let mut child = cmd.spawn().map_err(|e| format!("Failed to spawn claude: {e}"))?;
    let stdout = child.stdout.take().expect("piped stdout");

    // Store child in the shared slot so the pane can kill it to interrupt.
    {
        let mut slot = child_slot.lock().unwrap();
        *slot = Some(child);
    }

    let reader = BufReader::new(stdout);
    let mut accumulated_text = String::new();
    let mut final_session_id = session_id.to_string();
    let mut is_error = false;
    let mut final_result: Option<String> = None;

    for line in reader.lines() {
        let line = match line {
            Ok(l) if l.trim().is_empty() => continue,
            // stdout closed — either process killed (interrupt) or normal EOF
            Ok(l) => l,
            Err(_) => break,
        };

        let v: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        match v["type"].as_str() {
            Some("assistant") => {
                if let Some(content) = v["message"]["content"].as_array() {
                    for block in content {
                        match block["type"].as_str() {
                            Some("text") => {
                                if let Some(text) = block["text"].as_str() {
                                    accumulated_text = text.to_string();
                                    let _ = event_tx.send(WorkerEvent::Chunk(accumulated_text.clone()));
                                }
                            }
                            Some("tool_use") => {
                                let name = block["name"].as_str().unwrap_or("tool").to_string();
                                let input_preview = block["input"]
                                    .as_object()
                                    .and_then(|o| o.values().next())
                                    .and_then(|v| v.as_str())
                                    .map(|s| s.chars().take(60).collect::<String>())
                                    .unwrap_or_default();
                                let _ = event_tx.send(WorkerEvent::ToolUse { name, input_preview });
                            }
                            _ => {}
                        }
                    }
                }
            }
            Some("result") => {
                if let Some(sid) = v["session_id"].as_str().filter(|s| !s.is_empty()) {
                    final_session_id = sid.to_string();
                }
                is_error = v["is_error"].as_bool().unwrap_or(false);
                final_result = v["result"].as_str().map(|s| s.trim().to_string());
            }
            _ => {}
        }
    }

    // Reclaim the child and wait to avoid zombies.
    let child_opt = child_slot.lock().unwrap().take();
    if let Some(mut c) = child_opt {
        let _ = c.wait();
    }

    let done = if is_error {
        WorkerEvent::Done(Err(final_result.unwrap_or_else(|| "unknown error".to_string())))
    } else {
        WorkerEvent::Done(Ok(TurnResult {
            response: if accumulated_text.is_empty() {
                final_result.unwrap_or_default()
            } else {
                accumulated_text
            },
            session_id: final_session_id,
        }))
    };
    let _ = event_tx.send(done);
    Ok(())
}
