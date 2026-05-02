//! OpenRouter backend for the Plexi AI broker (`ai.query` path, #383).
//!
//! Routes to any OpenRouter model via SSE streaming using `ureq` (blocking,
//! no tokio). The host reads `OPENROUTER_API_KEY` from the environment at
//! dispatch time; apps never see the key.
//!
//! **System prompt format:** OpenRouter uses the OpenAI message format.
//! The system prompt is injected as a leading `{"role":"system","content":"…"}`
//! entry in the messages array — NOT as a top-level `"system"` field (which
//! is Anthropic format and silently ignored by OpenRouter).
//!
//! **Generation ID:** captured from `X-Generation-Id` response header before
//! reading the SSE body. Threaded back via `StreamEvent::Done` so the broker
//! can query the real cost after the turn completes.

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::sync::mpsc;
use std::thread;

use super::{AiBackend, AiBackendError, AiBackendRequest, RawToolCall, StreamEvent};

fn parse_usage_tokens(usage: &serde_json::Value) -> (Option<u32>, Option<u32>) {
    let input = usage["prompt_tokens"]
        .as_u64()
        .or_else(|| usage["input_tokens"].as_u64())
        .or_else(|| usage["tokens_in"].as_u64())
        .map(|n| n as u32);
    let output = usage["completion_tokens"]
        .as_u64()
        .or_else(|| usage["output_tokens"].as_u64())
        .or_else(|| usage["tokens_out"].as_u64())
        .map(|n| n as u32);
    (input, output)
}

/// OpenRouter streaming backend.
pub struct OpenRouterBackend {
    pub api_key: String,
    /// Full OpenRouter model ID. e.g. `"anthropic/claude-sonnet-4-6"`.
    pub model: String,
}

impl std::fmt::Debug for OpenRouterBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenRouterBackend")
            .field("model", &self.model)
            .field("api_key", &"[redacted]")
            .finish()
    }
}

impl AiBackend for OpenRouterBackend {
    fn name(&self) -> &str {
        "openrouter"
    }

    fn stream_to_channel(
        &self,
        request: AiBackendRequest,
        tx: mpsc::Sender<StreamEvent>,
    ) -> Result<(), AiBackendError> {
        let api_key = self.api_key.clone();
        let model = self.model.clone();

        thread::Builder::new()
            .name("plexi-ai-openrouter".to_string())
            .spawn(move || {
                stream_openrouter(api_key, model, request, tx);
            })
            .map_err(|e| AiBackendError::Io(format!("failed to spawn OpenRouter stream thread: {e}")))?;

        Ok(())
    }
}

/// Partial accumulator for a streaming tool-call delta.
#[derive(Default)]
struct PartialToolCall {
    id: String,
    name: String,
    arguments: String,
}

/// Worker: calls OpenRouter SSE endpoint and delivers events to `tx`.
fn stream_openrouter(
    api_key: String,
    model: String,
    request: AiBackendRequest,
    tx: mpsc::Sender<StreamEvent>,
) {
    if request.messages.is_empty() {
        let _ = tx.send(StreamEvent::Error(
            "AiBackendRequest.messages is empty — backend requires at least one message".to_string(),
        ));
        return;
    }

    // Build the messages array. OpenRouter uses OpenAI format:
    // system prompt goes as a leading {"role":"system"} entry, NOT top-level.
    // Messages are already serde_json::Value — pass them through directly.
    let mut messages: Vec<serde_json::Value> = Vec::new();
    if !request.system.is_empty() {
        messages.push(serde_json::json!({
            "role": "system",
            "content": request.system
        }));
    }
    messages.extend(request.messages.iter().cloned());

    let mut body = serde_json::json!({
        "model": model,
        "messages": messages,
        "stream": true,
        // Request usage in streamed responses so broker token accounting can
        // populate AiResponse.tokens_in / tokens_out reliably.
        "stream_options": { "include_usage": true }
    });

    // Inject tools when present.
    if !request.tools.is_empty() {
        let tools_json: Vec<serde_json::Value> = request
            .tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.input_schema
                    }
                })
            })
            .collect();
        body["tools"] = serde_json::Value::Array(tools_json);
    }

    let body_str = match serde_json::to_string(&body) {
        Ok(s) => s,
        Err(e) => {
            let _ = tx.send(StreamEvent::Error(format!(
                "failed to serialize OpenRouter request: {e}"
            )));
            return;
        }
    };

    // 10s connect timeout, 90s overall timeout — tool calls add latency.
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(90))
        .build();

    let resp = agent
        .post("https://openrouter.ai/api/v1/chat/completions")
        .set("Authorization", &format!("Bearer {api_key}"))
        .set("Content-Type", "application/json")
        .set("HTTP-Referer", "https://www.plexiapp.com/")
        .set("X-Title", "Plexi")
        .send_string(&body_str);

    let resp = match resp {
        Ok(r) => r,
        Err(ureq::Error::Status(status, error_resp)) => {
            let body = error_resp.into_string().unwrap_or_default();
            // Try to parse OpenRouter error shape: {"error":{"code":N,"message":"…"}}
            let msg = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| {
                    v["error"]["message"]
                        .as_str()
                        .map(|s| s.to_string())
                })
                .unwrap_or_else(|| body.clone());
            let _ = tx.send(StreamEvent::Error(format!(
                "openrouter http error {status}: {msg}"
            )));
            return;
        }
        Err(e) => {
            let _ = tx.send(StreamEvent::Error(format!("openrouter io error: {e}")));
            return;
        }
    };

    // Capture generation ID from the response header BEFORE reading the body.
    // This header is stable for the entire call and used to fetch real cost.
    let gen_id = resp.header("X-Generation-Id").map(|s| s.to_string());

    let reader = BufReader::new(resp.into_reader());

    let mut input_tokens: Option<u32> = None;
    let mut output_tokens: Option<u32> = None;
    // Accumulate tool-call deltas across SSE chunks, keyed by index.
    let mut partial_tool_calls: HashMap<usize, PartialToolCall> = HashMap::new();

    for line_result in reader.lines() {
        let line = match line_result {
            Ok(l) => l,
            Err(e) => {
                let _ = tx.send(StreamEvent::Error(format!("sse read error: {e}")));
                return;
            }
        };

        // Skip SSE comment lines (heartbeats like ": OPENROUTER PROCESSING")
        if line.starts_with(':') || line.is_empty() {
            continue;
        }

        let data = if let Some(suffix) = line.strip_prefix("data: ") {
            suffix
        } else {
            continue;
        };

        if data == "[DONE]" {
            break;
        }

        let chunk: serde_json::Value = match serde_json::from_str(data) {
            Ok(v) => v,
            Err(e) => {
                log::debug!("openrouter: failed to parse SSE chunk: {e} — data={data}");
                continue;
            }
        };

        // Usage-only chunk: choices is empty/absent, usage is present.
        if chunk["choices"].as_array().map_or(true, |c| c.is_empty()) {
            if let Some(usage) = chunk.get("usage").filter(|v| !v.is_null()) {
                let (inp, out) = parse_usage_tokens(usage);
                input_tokens = inp;
                output_tokens = out;
            }
            continue;
        }

        let choice = &chunk["choices"][0];
        let finish_reason = choice["finish_reason"].as_str();

        // Check for mid-stream error
        if finish_reason == Some("error") {
            let msg = choice["error"]["message"]
                .as_str()
                .unwrap_or("unknown error")
                .to_string();
            let _ = tx.send(StreamEvent::Error(format!("openrouter stream error: {msg}")));
            return;
        }

        // Accumulate tool-call deltas.
        if let Some(tc_deltas) = choice["delta"]["tool_calls"].as_array() {
            for delta in tc_deltas {
                let idx = delta["index"].as_u64().unwrap_or(0) as usize;
                let entry = partial_tool_calls.entry(idx).or_default();
                if let Some(id) = delta["id"].as_str() {
                    entry.id.push_str(id);
                }
                if let Some(name) = delta["function"]["name"].as_str() {
                    entry.name.push_str(name);
                }
                if let Some(args) = delta["function"]["arguments"].as_str() {
                    entry.arguments.push_str(args);
                }
            }
        }

        // When finish_reason == "tool_calls", finalize and emit.
        if finish_reason == Some("tool_calls") {
            let calls: Vec<RawToolCall> = partial_tool_calls
                .into_iter()
                .collect::<std::collections::BTreeMap<_, _>>()
                .into_values()
                .map(|p| RawToolCall {
                    id: p.id,
                    name: p.name,
                    arguments: p.arguments,
                })
                .collect();
            // Sort is stable from BTreeMap — calls are in index order.
            let _ = tx.send(StreamEvent::ToolCalls(calls));
            // Token counts may arrive in subsequent chunks or in this one.
            if let Some(usage) = chunk.get("usage").filter(|v| !v.is_null()) {
                let (inp, out) = parse_usage_tokens(usage);
                input_tokens = inp;
                output_tokens = out;
            }
            let _ = tx.send(StreamEvent::Done {
                input_tokens,
                output_tokens,
                generation_id: gen_id,
            });
            return;
        }

        // Text delta
        if let Some(text) = choice["delta"]["content"].as_str() {
            if !text.is_empty() {
                if tx.send(StreamEvent::Text(text.to_string())).is_err() {
                    // Receiver dropped — caller cancelled.
                    return;
                }
            }
        }
    }

    let _ = tx.send(StreamEvent::Done {
        input_tokens,
        output_tokens,
        generation_id: gen_id,
    });
    if input_tokens.is_none() && output_tokens.is_none() {
        log::warn!(
            "openrouter: stream completed without usage metadata (model may omit stream usage)"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that the system prompt is injected as a leading messages-array
    /// entry (OpenAI format), not as a top-level "system" field (Anthropic).
    #[test]
    fn system_prompt_goes_in_messages_array_not_top_level() {
        let system = "You are a helpful assistant.".to_string();
        let msgs = vec![serde_json::json!({"role": "user", "content": "hello"})];

        let mut messages: Vec<serde_json::Value> = Vec::new();
        if !system.is_empty() {
            messages.push(serde_json::json!({
                "role": "system",
                "content": system
            }));
        }
        messages.extend(msgs.iter().cloned());

        let body = serde_json::json!({
            "model": "anthropic/claude-sonnet-4-6",
            "messages": messages,
            "stream": true
        });

        // Top-level "system" field must not be present.
        assert!(
            body.get("system").is_none(),
            "top-level 'system' field must not be set (Anthropic format — silently ignored by OpenRouter)"
        );

        // First message must be the system entry.
        let first = &body["messages"][0];
        assert_eq!(first["role"].as_str(), Some("system"));
        assert_eq!(
            first["content"].as_str(),
            Some("You are a helpful assistant.")
        );

        // Second message must be the user turn.
        let second = &body["messages"][1];
        assert_eq!(second["role"].as_str(), Some("user"));
        assert_eq!(second["content"].as_str(), Some("hello"));
    }

    /// Verify that an empty system string produces no system entry.
    #[test]
    fn empty_system_omits_system_message() {
        let system = String::new();
        let msgs = vec![serde_json::json!({"role": "user", "content": "hi"})];

        let mut messages: Vec<serde_json::Value> = Vec::new();
        if !system.is_empty() {
            messages.push(serde_json::json!({"role": "system", "content": system}));
        }
        messages.extend(msgs.iter().cloned());

        assert_eq!(messages.len(), 1, "only user message — no system entry");
        assert_eq!(messages[0]["role"].as_str(), Some("user"));
    }

    /// Verify Done carries the generation_id field.
    #[test]
    fn done_variant_carries_generation_id() {
        let event = StreamEvent::Done {
            input_tokens: Some(10),
            output_tokens: Some(20),
            generation_id: Some("gen-abc123".to_string()),
        };
        if let StreamEvent::Done { generation_id, .. } = event {
            assert_eq!(generation_id.as_deref(), Some("gen-abc123"));
        } else {
            panic!("expected Done variant");
        }
    }
}
