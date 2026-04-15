//! Worker-thread Claude CLI client for agent mode.
//!
//! Replaces the previous ureq-based Anthropic API backend with a `claude -p`
//! subprocess approach. The UI thread spawns an `LlmWorker`, sends `LlmRequest`s
//! via its channel, and polls `try_recv()` each frame for `LlmResponse`s.
//!
//! Session continuity is maintained by passing `--resume <session_id>` on all
//! turns after the first. The first turn receives a new session ID from the
//! `result` JSON line and stores it for subsequent requests.
//!
//! Output is streamed via `--output-format stream-json --verbose`. Each
//! `assistant` JSON line is parsed for text deltas and forwarded as
//! `LlmResponse::Token`. A `result` line ends the turn with
//! `LlmResponse::Complete` and captures the session ID.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;

/// Request sent from the UI thread to the LLM worker.
#[derive(Debug)]
pub enum LlmRequest {
    /// Conversational turn: a user message, optional system prompt, and the
    /// current session ID (None on the first turn).
    Complete {
        prompt: String,
        system: String,
        session_id: Option<String>,
    },
}

/// Response sent from the LLM worker back to the UI thread.
#[derive(Debug)]
pub enum LlmResponse {
    /// Streaming text chunk — emitted as each `assistant` JSON event arrives.
    Token(String),
    /// Turn complete. Contains the full response text and the session ID to
    /// use for the next request.
    Complete(String, String),
    /// Terminal error — surface this in the conversation.
    Error(String),
}

/// Handle held by the UI thread. Drop to shut the worker down.
pub struct LlmWorker {
    tx: mpsc::Sender<LlmRequest>,
    rx: mpsc::Receiver<LlmResponse>,
    /// Shared handle to the currently-running `claude` subprocess, if any.
    /// `cancel()` locks this and calls `kill()` on it. `call_claude` sets it
    /// when spawning and clears it when the process finishes.
    current_child: Arc<Mutex<Option<Child>>>,
    /// Signal set by `cancel()` so the worker thread can suppress the final
    /// `Complete` (or `Error`) response for a killed turn. The worker clears
    /// it at the top of each new turn.
    cancelled: Arc<AtomicBool>,
    _handle: thread::JoinHandle<()>,
}

impl LlmWorker {
    /// Spawn a worker thread that owns the subprocess and response channel.
    pub fn spawn() -> Self {
        let (req_tx, req_rx) = mpsc::channel::<LlmRequest>();
        let (resp_tx, resp_rx) = mpsc::channel::<LlmResponse>();
        let current_child: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(None));
        let cancelled: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));

        let worker_child = Arc::clone(&current_child);
        let worker_cancelled = Arc::clone(&cancelled);
        let handle = thread::Builder::new()
            .name("plexi-agent-llm".to_string())
            .spawn(move || run_worker(req_rx, resp_tx, worker_child, worker_cancelled))
            .expect("failed to spawn agent LLM worker thread");

        Self {
            tx: req_tx,
            rx: resp_rx,
            current_child,
            cancelled,
            _handle: handle,
        }
    }

    /// Enqueue a request. Fails if the worker thread has shut down.
    pub fn send(&self, req: LlmRequest) -> Result<(), String> {
        self.tx
            .send(req)
            .map_err(|e| format!("agent LLM worker disconnected: {e}"))
    }

    /// Non-blocking: drain a single pending response, if any.
    pub fn try_recv(&self) -> Option<LlmResponse> {
        match self.rx.try_recv() {
            Ok(resp) => Some(resp),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                log::warn!("agent_llm: worker disconnected");
                None
            }
        }
    }

    /// Cancel the in-flight LLM request, if any. Sets the `cancelled` flag so
    /// the worker thread suppresses the final `Complete`/`Error` response for
    /// this turn, then kills the spawned `claude` child process. Safe to call
    /// even when no request is running — it's a no-op.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        let mut guard = match self.current_child.lock() {
            Ok(g) => g,
            Err(poisoned) => {
                log::warn!("agent_llm: current_child mutex poisoned on cancel");
                poisoned.into_inner()
            }
        };
        if let Some(mut child) = guard.take() {
            match child.kill() {
                Ok(()) => log::info!("agent_llm: killed in-flight claude subprocess"),
                Err(e) => log::warn!("agent_llm: failed to kill claude subprocess: {e}"),
            }
            // Reap the zombie so the kernel releases the PID.
            let _ = child.wait();
        }
    }

    /// Drain any pending responses in the channel. Used after `cancel()` so
    /// late streamed tokens from the killed turn don't surface in the next turn.
    pub fn drain_pending(&self) {
        while self.rx.try_recv().is_ok() {}
    }
}

fn run_worker(
    rx: mpsc::Receiver<LlmRequest>,
    tx: mpsc::Sender<LlmResponse>,
    current_child: Arc<Mutex<Option<Child>>>,
    cancelled: Arc<AtomicBool>,
) {
    while let Ok(req) = rx.recv() {
        match req {
            LlmRequest::Complete { prompt, system, session_id } => {
                // Clear any stale cancel flag from a previous turn before
                // starting a new one.
                cancelled.store(false, Ordering::SeqCst);
                if let Err(send_err) = call_claude(
                    &prompt,
                    &system,
                    session_id.as_deref(),
                    &tx,
                    &current_child,
                    &cancelled,
                ) {
                    log::error!("agent_llm: failed to send response: {send_err}");
                    return;
                }
            }
        }
    }
}

/// Spawn `claude -p --output-format stream-json --verbose` with the prompt on
/// stdin, stream JSON events to the response channel, and send `Complete` when
/// the process exits cleanly.
fn call_claude(
    prompt: &str,
    system: &str,
    session_id: Option<&str>,
    tx: &mpsc::Sender<LlmResponse>,
    current_child: &Arc<Mutex<Option<Child>>>,
    cancelled: &Arc<AtomicBool>,
) -> Result<(), mpsc::SendError<LlmResponse>> {
    // Locate the `claude` binary. Look in common user-local paths first so
    // macOS GUI apps (which don't inherit the user's shell PATH) can find it.
    let claude_bin = find_claude_binary();
    let Some(claude_bin) = claude_bin else {
        return tx.send(LlmResponse::Error(
            "claude CLI not found — install Claude Code (https://claude.ai/code)".to_string(),
        ));
    };

    let mut cmd = Command::new(&claude_bin);
    cmd.arg("-p");
    cmd.args(["--output-format", "stream-json"]);
    cmd.arg("--verbose");

    // System prompt only on the first turn — resumed sessions carry the original
    // system prompt in their history; re-injecting it causes context confusion.
    if session_id.is_none() && !system.is_empty() {
        cmd.args(["--system-prompt", system]);
    }

    if let Some(sid) = session_id {
        cmd.args(["--resume", sid]);
    }

    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::null());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return tx.send(LlmResponse::Error(format!(
                "failed to spawn claude CLI ({claude_bin}): {e}"
            )));
        }
    };

    // Write the prompt to stdin and close it so the process knows input is done.
    if let Some(mut stdin) = child.stdin.take() {
        if let Err(e) = stdin.write_all(prompt.as_bytes()) {
            log::error!("agent_llm: failed to write prompt to claude stdin: {e}");
        }
        // stdin is dropped here, closing the pipe
    }

    let stdout = match child.stdout.take() {
        Some(s) => s,
        None => {
            return tx.send(LlmResponse::Error(
                "failed to capture claude CLI stdout".to_string(),
            ));
        }
    };

    // Publish the child handle so `LlmWorker::cancel()` can kill it.
    {
        let mut guard = match current_child.lock() {
            Ok(g) => g,
            Err(poisoned) => {
                log::warn!("agent_llm: current_child mutex poisoned on spawn");
                poisoned.into_inner()
            }
        };
        *guard = Some(child);
    }

    let reader = BufReader::new(stdout);
    let mut full_response = String::new();
    let mut captured_session_id: Option<String> = None;
    let mut stream_error: Option<String> = None;

    for line_result in reader.lines() {
        // If we were cancelled mid-stream, stop forwarding tokens immediately.
        if cancelled.load(Ordering::SeqCst) {
            break;
        }

        let line = match line_result {
            Ok(l) => l,
            Err(e) => {
                // A broken pipe here typically means cancel() killed the child.
                log::warn!("agent_llm: error reading claude stdout: {e}");
                break;
            }
        };

        if line.is_empty() {
            continue;
        }

        match parse_stream_event(&line) {
            StreamEvent::AssistantText(chunk) => {
                full_response.push_str(&chunk);
                if !cancelled.load(Ordering::SeqCst) {
                    tx.send(LlmResponse::Token(chunk))?;
                }
            }
            StreamEvent::Result { session_id, is_error, error_text } => {
                if let Some(sid) = session_id {
                    captured_session_id = Some(sid);
                }
                if is_error {
                    stream_error = Some(
                        error_text.unwrap_or_else(|| "claude CLI returned an error".to_string()),
                    );
                }
            }
            StreamEvent::Unknown => {
                // Ignore system init, tool_use, etc.
            }
        }
    }

    // Reclaim the child handle (if still present) so we can wait on it here.
    // If cancel() already took and killed it, this will be None.
    let reclaimed = match current_child.lock() {
        Ok(mut g) => g.take(),
        Err(poisoned) => {
            log::warn!("agent_llm: current_child mutex poisoned on reclaim");
            poisoned.into_inner().take()
        }
    };
    if let Some(mut child) = reclaimed {
        let _ = child.wait();
    }

    // If the turn was cancelled, swallow any final response — the UI already
    // rendered the "^C — interrupted" line and reset its state.
    if cancelled.swap(false, Ordering::SeqCst) {
        log::info!("agent_llm: turn cancelled; suppressing Complete/Error response");
        return Ok(());
    }

    if let Some(msg) = stream_error {
        return tx.send(LlmResponse::Error(msg));
    }

    let sid = captured_session_id.unwrap_or_else(|| {
        // Fallback: generate a placeholder so the caller doesn't lose the turn.
        log::warn!("agent_llm: no session_id captured from claude output");
        format!("unknown-{}", std::time::UNIX_EPOCH
            .elapsed()
            .map(|d| d.as_secs())
            .unwrap_or(0))
    });

    tx.send(LlmResponse::Complete(full_response, sid))
}

/// Parsed representation of a single stream-json event line.
enum StreamEvent {
    AssistantText(String),
    Result {
        session_id: Option<String>,
        is_error: bool,
        error_text: Option<String>,
    },
    Unknown,
}

fn parse_stream_event(line: &str) -> StreamEvent {
    let Ok(val) = serde_json::from_str::<serde_json::Value>(line) else {
        return StreamEvent::Unknown;
    };

    let event_type = val.get("type").and_then(|t| t.as_str()).unwrap_or("");

    match event_type {
        "assistant" => {
            // {"type":"assistant","message":{"content":[{"type":"text","text":"..."}],...}}
            let text = val
                .pointer("/message/content")
                .and_then(|c| c.as_array())
                .map(|blocks| {
                    blocks
                        .iter()
                        .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
                        .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                        .collect::<Vec<_>>()
                        .join("")
                })
                .unwrap_or_default();

            if text.is_empty() {
                StreamEvent::Unknown
            } else {
                StreamEvent::AssistantText(text)
            }
        }
        "result" => {
            let session_id = val
                .get("session_id")
                .and_then(|s| s.as_str())
                .map(|s| s.to_string());

            let is_error = val
                .get("is_error")
                .and_then(|e| e.as_bool())
                .unwrap_or(false);

            // Prefer the top-level `result` string for the error message, but
            // also check common error-wrapping structures.
            let error_text = if is_error {
                val.get("result")
                    .and_then(|r| r.as_str())
                    .map(|s| s.to_string())
            } else {
                None
            };

            StreamEvent::Result { session_id, is_error, error_text }
        }
        _ => StreamEvent::Unknown,
    }
}

/// Search for the `claude` binary in common locations. macOS GUI app bundles
/// do not inherit the user's shell PATH, so we probe known install paths
/// before falling back to a raw `claude` (which works in dev/terminal runs).
fn find_claude_binary() -> Option<String> {
    let candidates = [
        "/usr/local/bin/claude",
        "/opt/homebrew/bin/claude",
        // npm global installs
        "/usr/local/share/npm/bin/claude",
    ];

    for candidate in &candidates {
        if std::path::Path::new(candidate).exists() {
            return Some(candidate.to_string());
        }
    }

    // Fall back to bare name — works in terminal contexts where PATH is set.
    // Also try `which claude` so that shell-managed paths (nvm, volta, etc.)
    // are discovered at runtime.
    if let Ok(output) = Command::new("which").arg("claude").output() {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return Some(path);
            }
        }
    }

    // Last resort: trust that it's on PATH and let the OS resolve it.
    // If it truly isn't found, Command::spawn will return a NotFound error
    // which we surface as an LlmResponse::Error.
    Some("claude".to_string())
}
