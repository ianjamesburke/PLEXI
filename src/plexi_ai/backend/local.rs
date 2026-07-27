//! Local OpenAI-compatible backend for the Plexi AI broker (stint 0579).
//!
//! Speaks the OpenAI chat-completions SSE protocol against a configurable
//! `base_url` (e.g. a Meridian proxy at `http://127.0.0.1:3456`), reusing the
//! shared streaming worker in `super::openrouter`. Auth is optional: an
//! `Authorization: Bearer <key>` header is sent only when an API key is
//! configured — local proxies commonly accept unauthenticated requests.
//! OpenRouter's nonstandard `reasoning` body field is never sent — strict
//! OpenAI-compatible servers reject unknown request properties.

use std::sync::mpsc;
use std::thread;

use super::openrouter::stream_openai_compatible;
use super::{AiBackend, AiBackendError, AiBackendRequest, StreamEvent};

/// Local OpenAI-compatible streaming backend.
pub struct LocalOpenAiBackend {
    /// Server base URL, e.g. `"http://127.0.0.1:3456"`. The chat-completions
    /// path is appended by [`endpoint_url`].
    pub base_url: String,
    /// Optional API key. `None` = no auth header.
    pub api_key: Option<String>,
    /// Model ID as understood by the server, e.g. `"claude-fable-5"`.
    pub model: String,
}

impl std::fmt::Debug for LocalOpenAiBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalOpenAiBackend")
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("api_key", &self.api_key.as_ref().map(|_| "[redacted]"))
            .finish()
    }
}

/// Chat-completions endpoint for a base URL — a trailing `/` on the base is
/// trimmed so both `http://host:port` and `http://host:port/` work.
fn endpoint_url(base_url: &str) -> String {
    format!("{}/v1/chat/completions", base_url.trim_end_matches('/'))
}

impl AiBackend for LocalOpenAiBackend {
    fn name(&self) -> &str {
        "local"
    }

    fn stream_to_channel(
        &self,
        request: AiBackendRequest,
        tx: mpsc::Sender<StreamEvent>,
    ) -> Result<(), AiBackendError> {
        let endpoint = endpoint_url(&self.base_url);
        let api_key = self.api_key.clone();
        let model = self.model.clone();

        thread::Builder::new()
            .name("plexi-ai-local".to_string())
            .spawn(move || {
                stream_openai_compatible(&endpoint, api_key, model, false, request, tx);
            })
            .map_err(|e| {
                AiBackendError::Io(format!("failed to spawn local AI stream thread: {e}"))
            })?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_url_appends_chat_completions_path() {
        assert_eq!(
            endpoint_url("http://127.0.0.1:3456"),
            "http://127.0.0.1:3456/v1/chat/completions"
        );
    }

    #[test]
    fn endpoint_url_trims_trailing_slash() {
        assert_eq!(
            endpoint_url("http://127.0.0.1:3456/"),
            "http://127.0.0.1:3456/v1/chat/completions"
        );
    }

    #[test]
    fn debug_redacts_api_key() {
        let backend = LocalOpenAiBackend {
            base_url: "http://127.0.0.1:3456".to_string(),
            api_key: Some("sk-secret".to_string()),
            model: "claude-fable-5".to_string(),
        };
        let dbg = format!("{backend:?}");
        assert!(!dbg.contains("sk-secret"), "api key must be redacted: {dbg}");
        assert!(dbg.contains("[redacted]"));
    }
}
