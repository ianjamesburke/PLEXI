//! Native mode — direct Anthropic Messages API via `async-anthropic`.
//!
//! Plexi IQ owns the tool loop. `supports_tool_dispatch()` returns `true`.
//!
//! Stage 1 scope:
//! - Builds an `async_anthropic::Client` from the provided API key.
//! - Translates `LlmRequest` into a streaming `CreateMessagesRequest`.
//! - Delivers text chunks and token counts as `StreamEvent`s.
//! - Spawns a dedicated tokio runtime on a background thread so the async
//!   streaming call doesn't require the rest of Plexi to be async-aware.
//!
//! Stage 2 will add: tool schemas, tool_use event delivery, cache_control
//! markers, and the 1M context beta header.

use std::sync::mpsc;
use std::thread;

use super::{BillingModel, LlmBackend, LlmError, LlmRequest, StreamEvent};

/// Direct Anthropic Messages API backend.
pub struct AnthropicApiBackend {
    api_key: String,
    /// Model to use. Defaults to `claude-sonnet-4-5`.
    model: String,
}

impl std::fmt::Debug for AnthropicApiBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnthropicApiBackend")
            .field("model", &self.model)
            .field("api_key", &"[redacted]")
            .finish()
    }
}

impl AnthropicApiBackend {
    /// Construct from an explicit API key.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: "claude-sonnet-4-5".to_string(),
        }
    }

    /// Construct from `ANTHROPIC_API_KEY` env var. Returns `None` if unset.
    pub fn from_env() -> Option<Self> {
        let key = std::env::var("ANTHROPIC_API_KEY").ok()?;
        if key.is_empty() {
            return None;
        }
        Some(Self::new(key))
    }
}

impl LlmBackend for AnthropicApiBackend {
    fn name(&self) -> &str {
        "anthropic-api (native)"
    }

    fn supports_tool_dispatch(&self) -> bool {
        // Native mode — Plexi IQ owns the tool loop.
        true
    }

    fn billing_model(&self) -> BillingModel {
        BillingModel::Metered
    }

    fn stream_to_channel(
        &self,
        request: LlmRequest,
        tx: mpsc::Sender<StreamEvent>,
    ) -> Result<(), LlmError> {
        let api_key = self.api_key.clone();
        let model = self.model.clone();

        thread::Builder::new()
            .name("plexi-iq-api".to_string())
            .spawn(move || {
                // Build a minimal single-threaded tokio runtime for the async call.
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ = tx.send(StreamEvent::Error(format!(
                            "failed to build tokio runtime: {e}"
                        )));
                        return;
                    }
                };
                rt.block_on(stream_native(api_key, model, request, tx));
            })
            .map_err(|e| LlmError::Io(format!("failed to spawn API stream thread: {e}")))?;

        Ok(())
    }
}

/// Async worker: calls the Anthropic API and delivers events to `tx`.
async fn stream_native(
    api_key: String,
    model: String,
    request: LlmRequest,
    tx: mpsc::Sender<StreamEvent>,
) {
    use async_anthropic::types::{
        CreateMessagesRequestBuilder, MessageBuilder, MessageRole, MessagesStreamEvent,
    };
    use tokio_stream::StreamExt as _;

    let client = async_anthropic::Client::from_api_key(api_key);

    let user_msg = match MessageBuilder::default()
        .role(MessageRole::User)
        .content(request.prompt.as_str())
        .build()
    {
        Ok(m) => m,
        Err(e) => {
            let _ = tx.send(StreamEvent::Error(format!(
                "failed to build user message: {e}"
            )));
            return;
        }
    };

    // Build the request. The system prompt field takes `&str`, so we keep a
    // local binding for the string value before starting the builder chain.
    let system_str = request.system.clone();
    let api_request = {
        let mut b = CreateMessagesRequestBuilder::default();
        b.model(model);
        b.messages(vec![user_msg]);
        if !system_str.is_empty() {
            b.system(system_str.as_str());
        }
        match b.build() {
            Ok(r) => r,
            Err(e) => {
                let _ = tx.send(StreamEvent::Error(format!(
                    "failed to build API request: {e}"
                )));
                return;
            }
        }
    };

    let mut stream = client.messages().create_stream(api_request).await;

    let mut input_tokens: Option<u32> = None;
    let mut output_tokens: Option<u32> = None;

    while let Some(event_result) = stream.next().await {
        match event_result {
            Ok(event) => match event {
                MessagesStreamEvent::ContentBlockDelta { delta, .. } => {
                    use async_anthropic::types::ContentBlockDelta;
                    if let ContentBlockDelta::TextDelta { text } = delta {
                        if tx.send(StreamEvent::Text(text)).is_err() {
                            // Receiver dropped — caller cancelled.
                            return;
                        }
                    }
                }
                MessagesStreamEvent::MessageStart { usage: Some(u), .. } => {
                    input_tokens = u.input_tokens;
                }
                MessagesStreamEvent::MessageStart { usage: None, .. } => {}
                MessagesStreamEvent::MessageDelta { usage, .. } => {
                    output_tokens = usage.and_then(|u| u.output_tokens);
                }
                MessagesStreamEvent::MessageStop => break,
                _ => {}
            },
            Err(e) => {
                let _ = tx.send(StreamEvent::Error(format!("API stream error: {e}")));
                return;
            }
        }
    }

    let _ = tx.send(StreamEvent::Done {
        input_tokens,
        output_tokens,
        session_id: None,
    });
}
