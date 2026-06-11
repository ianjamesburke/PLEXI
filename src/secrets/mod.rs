use log::{error, warn};
use serde::{Deserialize, Serialize};
use std::path::Path;
use zeroize::Zeroizing;

const SERVICE_NAME: &str = "plexi";

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

/// Build the v3 Keychain account string: "plexi/{workspace_root}/{key}"
/// Workspace-scoped; app_id is NOT part of the key (secrets are workspace-owned, not app-owned).
fn account_key_scoped(key: &str, workspace_root: &Path) -> String {
    format!("plexi/{}/{}", workspace_root.display(), key)
}

// ── v3 workspace-scoped secret API ───────────────────────────────────────────

/// Retrieve a workspace-scoped secret.
///
/// **Hard invariant:** `workspace_root` must be a non-empty, absolute path.
/// If not, this logs an error and returns `None` — no secret is ever returned
/// from an invalid scope.
///
/// Keychain key format: `plexi/{workspace_root}/{key}`
#[cfg(target_os = "macos")]
pub fn get_secret_scoped(
    key: &str,
    app_id: &str,
    workspace_root: &Path,
) -> Option<Zeroizing<String>> {
    use security_framework::passwords::get_generic_password;

    if !validate_workspace_root(workspace_root, "get_secret_scoped", app_id, key) {
        return None;
    }

    let account = account_key_scoped(key, workspace_root);
    match get_generic_password(SERVICE_NAME, &account) {
        Ok(data) => Some(Zeroizing::new(
            String::from_utf8_lossy(&data).trim().to_string(),
        )),
        Err(e) if e.code() == -25300 => None,
        Err(e) => {
            warn!(
                "secrets::get_secret_scoped: keychain error for app={app_id} workspace={} key={key}: {e}",
                workspace_root.display()
            );
            None
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn get_secret_scoped(
    key: &str,
    app_id: &str,
    workspace_root: &Path,
) -> Option<Zeroizing<String>> {
    warn!(
        "secrets::get_secret_scoped({key}, {app_id}, {}): Keychain not available on this platform",
        workspace_root.display()
    );
    None
}

/// Validate workspace_root for secret operations. Returns false and logs an error on failure.
fn validate_workspace_root(workspace_root: &Path, op: &str, app_id: &str, key: &str) -> bool {
    if workspace_root.as_os_str().is_empty() {
        error!(
            "secrets::{op}: workspace_root is empty for app={app_id} key={key}. \
             Secret denied — workspace_root must be set from Init.workspace_root only."
        );
        return false;
    }
    if !workspace_root.is_absolute() {
        error!(
            "secrets::{op}: workspace_root '{}' is not absolute for app={app_id} key={key}. \
             Secret denied.",
            workspace_root.display()
        );
        return false;
    }
    true
}
