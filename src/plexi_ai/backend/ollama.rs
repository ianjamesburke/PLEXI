//! Ollama backend for the Plexi AI broker (`ai.query` path).
//!
//! Streams responses from a local Ollama instance via `POST /api/chat`.
//! Response format is NDJSON (one JSON object per line, no `data:` prefix).
//!
//! Uses a 5s connect timeout (fail fast if Ollama isn't running) and a 60s
//! overall timeout (local models can be slow on first load).

use std::io::{BufRead, BufReader};
use std::sync::mpsc;
use std::thread;

use super::{AiBackend, AiBackendError, AiBackendRequest, RawToolCall, StreamEvent};

/// Ollama streaming backend.
pub struct OllamaBackend {
    /// Ollama host URL. e.g. `"http://localhost:11434"`.
    pub host: String,
    /// Ollama model name. e.g. `"llama3.2:3b"`.
    pub model: String,
}

impl AiBackend for OllamaBackend {
    fn name(&self) -> &str {
        "ollama"
    }

    fn stream_to_channel(
        &self,
        request: AiBackendRequest,
        tx: mpsc::Sender<StreamEvent>,
    ) -> Result<(), AiBackendError> {
        let host = self.host.clone();
        let model = self.model.clone();

        thread::Builder::new()
            .name("plexi-ai-ollama".to_string())
            .spawn(move || {
                stream_ollama(host, model, request, tx);
            })
            .map_err(|e| {
                AiBackendError::Io(format!("failed to spawn Ollama stream thread: {e}"))
            })?;

        Ok(())
    }
}

/// Worker: calls Ollama `/api/chat` with streaming and delivers events to `tx`.
fn stream_ollama(
    host: String,
    model: String,
    request: AiBackendRequest,
    tx: mpsc::Sender<StreamEvent>,
) {
    if request.messages.is_empty() {
        let _ = tx.send(StreamEvent::Error(
            "AiBackendRequest.messages is empty — backend requires at least one message"
                .to_string(),
        ));
        return;
    }

    // Build messages in Ollama format (same as OpenAI: role + content).
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
        "stream": true
    });

    // Inject tools when present (Ollama uses same OpenAI function-calling schema).
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
                "failed to serialize Ollama request: {e}"
            )));
            return;
        }
    };

    // 5s connect timeout (fail fast if Ollama isn't running), 60s overall
    // timeout (local models can be slow on first load).
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(60))
        .build();

    let url = format!("{host}/api/chat");
    let resp = agent
        .post(&url)
        .set("Content-Type", "application/json")
        .send_string(&body_str);

    let resp = match resp {
        Ok(r) => r,
        Err(ureq::Error::Status(status, error_resp)) => {
            let body = error_resp.into_string().unwrap_or_default();
            let _ = tx.send(StreamEvent::Error(format!(
                "ollama http error {status}: {body}"
            )));
            return;
        }
        Err(e) => {
            let _ = tx.send(StreamEvent::Error(format!("ollama io error: {e}")));
            return;
        }
    };

    let reader = BufReader::new(resp.into_reader());

    let mut input_tokens: Option<u32> = None;
    let mut output_tokens: Option<u32> = None;

    for line_result in reader.lines() {
        let line = match line_result {
            Ok(l) => l,
            Err(e) => {
                let _ = tx.send(StreamEvent::Error(format!("ollama read error: {e}")));
                return;
            }
        };

        if line.is_empty() {
            continue;
        }

        let chunk: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                log::debug!("ollama: failed to parse NDJSON line: {e} — line={line}");
                continue;
            }
        };

        // Extract text delta from message content.
        if let Some(text) = chunk["message"]["content"].as_str() {
            if !text.is_empty() {
                if tx.send(StreamEvent::Text(text.to_string())).is_err() {
                    return;
                }
            }
        }

        // Ollama tool calls arrive in a `done: true` chunk with
        // `message.tool_calls[].function.{name, arguments}`.
        // `arguments` is already a JSON object (not a string — unlike OpenRouter).
        if let Some(tool_calls_arr) = chunk["message"]["tool_calls"].as_array() {
            if !tool_calls_arr.is_empty() {
                let mut calls: Vec<RawToolCall> = Vec::new();
                for tc in tool_calls_arr {
                    let name = match tc["function"]["name"].as_str() {
                        Some(n) if !n.is_empty() => n.to_string(),
                        _ => {
                            log::warn!(
                                "ollama: skipping tool call with missing/empty function name — tc={tc}"
                            );
                            continue;
                        }
                    };
                    // arguments is a JSON object dict — serialize to string for
                    // consistency with RawToolCall.arguments (JSON string contract).
                    let arguments = match serde_json::to_string(&tc["function"]["arguments"]) {
                        Ok(s) => s,
                        Err(e) => {
                            log::warn!(
                                "ollama: failed to serialize tool call arguments for '{name}': {e} — skipping"
                            );
                            continue;
                        }
                    };
                    calls.push(RawToolCall {
                        id: String::new(), // Ollama does not assign call IDs
                        name,
                        arguments,
                    });
                }
                if !calls.is_empty() {
                    input_tokens = chunk["prompt_eval_count"].as_u64().map(|n| n as u32);
                    output_tokens = chunk["eval_count"].as_u64().map(|n| n as u32);
                    let _ = tx.send(StreamEvent::ToolCalls(calls));
                    let _ = tx.send(StreamEvent::Done {
                        input_tokens,
                        output_tokens,
                        generation_id: None,
                    });
                    return;
                }
            }
        }

        // `done: true` marks end of stream; capture token counts.
        if chunk["done"].as_bool().unwrap_or(false) {
            input_tokens = chunk["prompt_eval_count"].as_u64().map(|n| n as u32);
            output_tokens = chunk["eval_count"].as_u64().map(|n| n as u32);
            break;
        }
    }

    let _ = tx.send(StreamEvent::Done {
        input_tokens,
        output_tokens,
        generation_id: None,
    });
}

#[cfg(test)]
mod tests {
    use super::RawToolCall;

    /// Parse a sample Ollama tool call streaming line and verify name + arguments.
    #[test]
    fn ollama_parses_tool_call_from_streaming_line() {
        let line = r#"{"message":{"role":"assistant","content":"","tool_calls":[{"function":{"name":"my_tool","arguments":{"arg1":"val1","count":3}}}]},"done":true,"prompt_eval_count":10,"eval_count":5}"#;
        let chunk: serde_json::Value = serde_json::from_str(line).unwrap();
        let tool_calls = chunk["message"]["tool_calls"].as_array().unwrap();
        assert_eq!(tool_calls.len(), 1);
        let tc = &tool_calls[0];
        assert_eq!(tc["function"]["name"].as_str(), Some("my_tool"));
        let args = &tc["function"]["arguments"];
        assert_eq!(args["arg1"].as_str(), Some("val1"));
        assert_eq!(args["count"].as_u64(), Some(3));
    }

    /// Verify the arguments dict is serialized to a JSON string in RawToolCall.
    #[test]
    fn ollama_tool_call_arguments_serialized_as_string() {
        let line = r#"{"message":{"role":"assistant","tool_calls":[{"function":{"name":"add","arguments":{"a":1,"b":2}}}]},"done":true}"#;
        let chunk: serde_json::Value = serde_json::from_str(line).unwrap();
        let tool_calls = chunk["message"]["tool_calls"].as_array().unwrap();
        let tc = &tool_calls[0];
        let name = tc["function"]["name"].as_str().unwrap().to_string();
        let arguments = serde_json::to_string(&tc["function"]["arguments"]).unwrap();

        let raw = RawToolCall {
            id: String::new(),
            name,
            arguments: arguments.clone(),
        };

        assert_eq!(raw.name, "add");
        // arguments must be a JSON string — parseable back to the original dict.
        let reparsed: serde_json::Value = serde_json::from_str(&raw.arguments).unwrap();
        assert_eq!(reparsed["a"].as_u64(), Some(1));
        assert_eq!(reparsed["b"].as_u64(), Some(2));
    }

    /// A tool call missing a function name must be skipped (no panic).
    #[test]
    fn ollama_tool_call_with_no_function_name_skipped() {
        let line = r#"{"message":{"role":"assistant","tool_calls":[{"function":{"arguments":{"x":1}}}]},"done":true}"#;
        let chunk: serde_json::Value = serde_json::from_str(line).unwrap();
        let tool_calls_arr = chunk["message"]["tool_calls"].as_array().unwrap();
        let mut calls: Vec<RawToolCall> = Vec::new();
        for tc in tool_calls_arr {
            let name = match tc["function"]["name"].as_str() {
                Some(n) if !n.is_empty() => n.to_string(),
                _ => continue, // skip — mirrors production code
            };
            let arguments = serde_json::to_string(&tc["function"]["arguments"]).unwrap_or_default();
            calls.push(RawToolCall {
                id: String::new(),
                name,
                arguments,
            });
        }
        assert!(
            calls.is_empty(),
            "malformed tool call (no name) must be skipped"
        );
    }

    /// Verify that a `done: true` NDJSON line with token counts is parsed correctly.
    #[test]
    fn done_line_extracts_token_counts() {
        let line = r#"{"done":true,"prompt_eval_count":12,"eval_count":5}"#;
        let chunk: serde_json::Value = serde_json::from_str(line).unwrap();
        assert!(chunk["done"].as_bool().unwrap_or(false));
        assert_eq!(chunk["prompt_eval_count"].as_u64(), Some(12));
        assert_eq!(chunk["eval_count"].as_u64(), Some(5));
    }

    /// Verify that a partial text chunk is extracted correctly.
    #[test]
    fn text_chunk_extracted_from_message_content() {
        let line = r#"{"message":{"role":"assistant","content":"Hello"},"done":false}"#;
        let chunk: serde_json::Value = serde_json::from_str(line).unwrap();
        assert_eq!(chunk["message"]["content"].as_str(), Some("Hello"));
        assert!(!chunk["done"].as_bool().unwrap_or(false));
    }
}
