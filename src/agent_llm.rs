//! Worker-thread Anthropic API client for agent mode.
//!
//! MVP: single-turn, non-streaming messages API call. The UI thread spawns an
//! `LlmWorker`, sends `LlmRequest`s via its channel, and polls `try_recv()`
//! each frame for `LlmResponse`s. HTTP happens entirely on the worker thread
//! so egui is never blocked.
//!
//! Streaming, multi-turn context, and tool use are intentionally out of scope
//! here — they'll land in follow-up PRs.

use std::sync::mpsc;
use std::thread;

const ANTHROPIC_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const MODEL: &str = "claude-sonnet-4-20250514";
const MAX_TOKENS: u32 = 4096;

/// Request sent from the UI thread to the LLM worker.
#[derive(Debug)]
pub enum LlmRequest {
    /// Single-turn completion: a system prompt and a user message.
    Complete { prompt: String, system: String },
}

/// Response sent from the LLM worker back to the UI thread.
#[derive(Debug)]
pub enum LlmResponse {
    /// Reserved for future streaming. Not emitted in the MVP.
    #[allow(dead_code)]
    Token(String),
    /// Full assistant response (non-streaming MVP path).
    Complete(String),
    /// Terminal error — surface this in the conversation.
    Error(String),
}

/// Handle held by the UI thread. Drop to shut the worker down.
pub struct LlmWorker {
    tx: mpsc::Sender<LlmRequest>,
    rx: mpsc::Receiver<LlmResponse>,
    _handle: thread::JoinHandle<()>,
}

impl LlmWorker {
    /// Spawn a worker thread that owns an HTTP client and the given API key.
    pub fn spawn(api_key: String) -> Self {
        let (req_tx, req_rx) = mpsc::channel::<LlmRequest>();
        let (resp_tx, resp_rx) = mpsc::channel::<LlmResponse>();

        let handle = thread::Builder::new()
            .name("plexi-agent-llm".to_string())
            .spawn(move || run_worker(api_key, req_rx, resp_tx))
            .expect("failed to spawn agent LLM worker thread");

        Self {
            tx: req_tx,
            rx: resp_rx,
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
}

fn run_worker(
    api_key: String,
    rx: mpsc::Receiver<LlmRequest>,
    tx: mpsc::Sender<LlmResponse>,
) {
    // One agent per worker lifetime. ureq::Agent is cheap to clone and
    // safe to reuse across calls.
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(15))
        .timeout_read(std::time::Duration::from_secs(120))
        .build();

    while let Ok(req) = rx.recv() {
        match req {
            LlmRequest::Complete { prompt, system } => {
                let result = call_anthropic(&agent, &api_key, &system, &prompt);
                let resp = match result {
                    Ok(text) => LlmResponse::Complete(text),
                    Err(err) => {
                        log::error!("agent_llm: Anthropic API call failed: {err}");
                        LlmResponse::Error(err)
                    }
                };
                if tx.send(resp).is_err() {
                    log::warn!("agent_llm: response channel closed, shutting worker down");
                    return;
                }
            }
        }
    }
}

/// Non-streaming call to the Anthropic messages API. Returns the assistant
/// text on success or a human-readable error string on any failure.
fn call_anthropic(
    agent: &ureq::Agent,
    api_key: &str,
    system: &str,
    user_prompt: &str,
) -> Result<String, String> {
    let body = serde_json::json!({
        "model": MODEL,
        "max_tokens": MAX_TOKENS,
        "system": system,
        "messages": [
            { "role": "user", "content": user_prompt }
        ],
    });

    let result = agent
        .post(ANTHROPIC_URL)
        .set("x-api-key", api_key)
        .set("anthropic-version", ANTHROPIC_VERSION)
        .set("content-type", "application/json")
        .send_json(body);

    match result {
        Ok(response) => {
            let json: serde_json::Value = response
                .into_json()
                .map_err(|e| format!("failed to parse Anthropic response JSON: {e}"))?;
            extract_text(&json)
        }
        Err(ureq::Error::Status(code, response)) => {
            let body = response
                .into_string()
                .unwrap_or_else(|_| "<no body>".to_string());
            Err(format!("Anthropic API returned HTTP {code}: {body}"))
        }
        Err(ureq::Error::Transport(t)) => {
            Err(format!("Anthropic API transport error: {t}"))
        }
    }
}

/// Pull the assistant text out of an Anthropic `messages` response.
/// The response looks like `{ "content": [ { "type": "text", "text": "..." } ], ... }`.
fn extract_text(json: &serde_json::Value) -> Result<String, String> {
    let content = json
        .get("content")
        .and_then(|c| c.as_array())
        .ok_or_else(|| format!("Anthropic response missing 'content' array: {json}"))?;

    let mut out = String::new();
    for block in content {
        if block.get("type").and_then(|t| t.as_str()) == Some("text") {
            if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(text);
            }
        }
    }

    if out.is_empty() {
        Err(format!("Anthropic response had no text blocks: {json}"))
    } else {
        Ok(out)
    }
}
