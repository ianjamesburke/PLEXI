//! Ollama backend for the Plexi AI broker (`ai.query` path).
//!
//! Streams responses from a local Ollama instance via `POST /api/chat`.
//! Response format is NDJSON (one JSON object per line, no `data:` prefix).
//!
//! Uses a 5s connect timeout (fail fast if Ollama isn't running) and a 60s
//! overall timeout (local models can be slow on first load).

use std::collections::HashMap;
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

fn messages_for_ollama(messages: &[serde_json::Value]) -> Result<Vec<serde_json::Value>, String> {
    let mut translated = Vec::with_capacity(messages.len());
    let mut pending_tool_names = HashMap::new();

    for source in messages {
        let mut message = source.clone();
        let role = message.get("role").and_then(serde_json::Value::as_str);

        if role == Some("assistant") {
            if let Some(tool_calls) = message
                .get_mut("tool_calls")
                .and_then(serde_json::Value::as_array_mut)
            {
                pending_tool_names.clear();
                for tool_call in tool_calls {
                    let function = tool_call
                        .get_mut("function")
                        .and_then(serde_json::Value::as_object_mut)
                        .ok_or_else(|| {
                            "assistant tool call is missing a function object".to_string()
                        })?;
                    let name = function
                        .get("name")
                        .and_then(serde_json::Value::as_str)
                        .filter(|name| !name.is_empty())
                        .ok_or_else(|| {
                            "assistant tool call is missing a function name".to_string()
                        })?
                        .to_string();
                    let arguments = function.get_mut("arguments").ok_or_else(|| {
                        format!("assistant tool call '{name}' is missing arguments")
                    })?;
                    if let Some(encoded) = arguments.as_str() {
                        let parsed: serde_json::Value =
                            serde_json::from_str(encoded).map_err(|e| {
                                format!(
                                    "assistant tool call '{name}' has invalid JSON arguments: {e}"
                                )
                            })?;
                        if !parsed.is_object() {
                            return Err(format!(
                                "assistant tool call '{name}' arguments must be a JSON object"
                            ));
                        }
                        *arguments = parsed;
                    } else if !arguments.is_object() {
                        return Err(format!(
                            "assistant tool call '{name}' arguments must be a JSON object"
                        ));
                    }
                    let id = tool_call
                        .get("id")
                        .and_then(serde_json::Value::as_str)
                        .filter(|id| !id.is_empty())
                        .ok_or_else(|| format!("assistant tool call '{name}' is missing an id"))?;
                    pending_tool_names.insert(id.to_string(), name);
                }
            }
        } else if role == Some("tool") {
            let object = message
                .as_object_mut()
                .ok_or_else(|| "tool result message must be a JSON object".to_string())?;
            if object.get("tool_name").is_none() {
                let call_id = object
                    .get("tool_call_id")
                    .and_then(serde_json::Value::as_str)
                    .filter(|id| !id.is_empty())
                    .ok_or_else(|| "tool result is missing tool_call_id".to_string())?;
                let name = pending_tool_names.remove(call_id).ok_or_else(|| {
                    format!("tool result references unknown tool call '{call_id}'")
                })?;
                object.insert("tool_name".to_string(), serde_json::Value::String(name));
            }
            object.remove("tool_call_id");
        }

        translated.push(message);
    }

    Ok(translated)
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

    // Build the Ollama conversation, including its native tool-history fields.
    let mut messages: Vec<serde_json::Value> = Vec::new();
    if !request.system.is_empty() {
        messages.push(serde_json::json!({
            "role": "system",
            "content": request.system
        }));
    }
    let translated = match messages_for_ollama(&request.messages) {
        Ok(messages) => messages,
        Err(error) => {
            let _ = tx.send(StreamEvent::Error(format!(
                "failed to translate Ollama messages: {error}"
            )));
            return;
        }
    };
    messages.extend(translated);

    let mut body = serde_json::json!({
        "model": model,
        "messages": messages,
        "stream": true
    });

    // Ollama tool definitions use the OpenAI function-definition schema.
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
        // Cooperative cancel: stop reading and drop `tx` so the turn loop
        // returns the partial text rather than draining the full stream.
        if request.cancel.is_cancelled() {
            log::info!("ollama: stream cancelled by caller — aborting read mid-stream");
            return;
        }
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
            if !text.is_empty()
                && tx.send(StreamEvent::Text(text.to_string())).is_err() {
                    return;
                }
        }

        // Ollama tool calls arrive in a `done: true` chunk with
        // `message.tool_calls[].function.{name, arguments}`.
        // `arguments` is already a JSON object (not a string — unlike OpenRouter).
        if let Some(tool_calls_arr) = chunk["message"]["tool_calls"].as_array() {
            if !tool_calls_arr.is_empty() {
                let mut calls: Vec<RawToolCall> = Vec::new();
                for (index, tc) in tool_calls_arr.iter().enumerate() {
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
                        // Ollama does not assign call IDs. The broker needs a
                        // stable identifier to pair each result with its call
                        // when translating the next history request.
                        id: format!("ollama-{}", index + 1),
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
    use super::{messages_for_ollama, RawToolCall};

    #[test]
    fn tool_call_arguments_are_objects_in_ollama_history() {
        let messages = vec![serde_json::json!({
            "role": "assistant",
            "content": null,
            "tool_calls": [{
                "id": "call-1",
                "type": "function",
                "function": {
                    "name": "host.apps.open",
                    "arguments": "{\"app\":\"file_browser\",\"layout\":\"split_h\"}"
                }
            }]
        })];

        let translated = messages_for_ollama(&messages).unwrap();

        assert_eq!(
            translated[0]["tool_calls"][0]["function"]["arguments"],
            serde_json::json!({"app": "file_browser", "layout": "split_h"})
        );
    }

    #[test]
    fn tool_results_use_ollama_tool_name_instead_of_tool_call_id() {
        let messages = vec![
            serde_json::json!({
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": "call-1",
                    "type": "function",
                    "function": {
                        "name": "host.apps.open",
                        "arguments": "{\"app\":\"file_browser\"}"
                    }
                }]
            }),
            serde_json::json!({
                "role": "tool",
                "tool_call_id": "call-1",
                "content": "{\"ok\":true,\"pane_id\":5}"
            }),
        ];

        let translated = messages_for_ollama(&messages).unwrap();

        assert_eq!(
            translated[1],
            serde_json::json!({
                "role": "tool",
                "tool_name": "host.apps.open",
                "content": "{\"ok\":true,\"pane_id\":5}"
            })
        );
    }

    #[test]
    fn reordered_tool_results_keep_their_matching_ollama_tool_names() {
        let messages = vec![
            serde_json::json!({
                "role": "assistant",
                "content": null,
                "tool_calls": [
                    {
                        "id": "call-1",
                        "type": "function",
                        "function": { "name": "host.apps.open", "arguments": "{}" }
                    },
                    {
                        "id": "call-2",
                        "type": "function",
                        "function": { "name": "host.panes.list", "arguments": "{}" }
                    }
                ]
            }),
            serde_json::json!({
                "role": "tool",
                "tool_call_id": "call-2",
                "content": "[]"
            }),
            serde_json::json!({
                "role": "tool",
                "tool_call_id": "call-1",
                "content": "{\"ok\":true}"
            }),
        ];

        let translated = messages_for_ollama(&messages).unwrap();

        assert_eq!(translated[1]["tool_name"], "host.panes.list");
        assert_eq!(translated[2]["tool_name"], "host.apps.open");
    }

    #[test]
    fn synthesized_ollama_call_id_round_trips_into_tool_result_history() {
        let call_id = format!("ollama-{}", 1);
        let messages = vec![
            serde_json::json!({
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": call_id,
                    "type": "function",
                    "function": { "name": "host.panes.list", "arguments": "{}" }
                }]
            }),
            serde_json::json!({
                "role": "tool",
                "tool_call_id": call_id,
                "content": "[]"
            }),
        ];

        let translated = messages_for_ollama(&messages).unwrap();

        assert_eq!(translated[1]["tool_name"], "host.panes.list");
    }

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
            arguments,
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
