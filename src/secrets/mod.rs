use serde::{Deserialize, Serialize};

/// A parsed Keychain entry stored under service="plexi".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretEntry {
    pub app_id: String,
    pub directory: String,
    pub key: String,
    /// Workspace root this secret is scoped to (v3). None for legacy v1/v2 secrets.
    #[serde(default)]
    pub workspace_root: Option<String>,
    /// When true, this secret is injected as an env var into every new shell session.
    #[serde(default)]
    pub inject: bool,
}
