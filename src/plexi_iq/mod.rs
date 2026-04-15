/// Plexi IQ — in-host agent orchestrator (Stage 1).
///
/// Stage 1 uses `claude -p --resume <session_id>` as the backend.
/// The native API backend is a config option for the future.

pub mod backend;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PlexiIqConfig {
    pub backend: IqBackend,
    #[serde(default)]
    pub api_key: Option<String>,
    pub default_directory_scope: Option<PathBuf>,
}

impl Default for PlexiIqConfig {
    fn default() -> Self {
        Self {
            backend: IqBackend::ClaudeProxy,
            api_key: None,
            default_directory_scope: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IqBackend {
    ClaudeProxy,
    NativeApi,
}

pub struct PlexiIq {
    pub config: PlexiIqConfig,
}

impl PlexiIq {
    pub fn new(config: PlexiIqConfig) -> Self {
        Self { config }
    }

    pub fn default() -> Self {
        Self::new(PlexiIqConfig::default())
    }
}

/// Per-pane IQ session. Tracks session state for `claude -p --resume`.
pub struct IqSession {
    pub session_id: String,
    pub run_id: Option<String>,
    pub workspace_dir: PathBuf,
}

impl IqSession {
    pub fn new(workspace_dir: PathBuf) -> Self {
        // Use a timestamp-based ID to avoid uuid dependency.
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        Self {
            session_id: format!("plexi-iq-{ts:x}"),
            run_id: None,
            workspace_dir,
        }
    }

    pub async fn send(&self, prompt: &str) -> Result<String, String> {
        backend::run_claude_proxy(&self.session_id, prompt, &self.workspace_dir).await
    }
}
