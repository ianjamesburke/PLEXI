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
}

/// Build the v1/v2 Keychain account string: "{app_id}/{directory}/{key}"
fn account_key(key: &str, app_id: &str, directory: &str) -> String {
    format!("{app_id}/{directory}/{key}")
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
pub fn get_secret_scoped(key: &str, app_id: &str, workspace_root: &Path) -> Option<Zeroizing<String>> {
    use std::process::Command;

    if !validate_workspace_root(workspace_root, "get_secret_scoped", app_id, key) {
        return None;
    }

    let account = account_key_scoped(key, workspace_root);
    match Command::new("security")
        .args(["find-generic-password", "-s", SERVICE_NAME, "-a", &account, "-w"])
        .output()
    {
        Ok(output) if output.status.success() => {
            Some(Zeroizing::new(String::from_utf8_lossy(&output.stdout).trim().to_string()))
        }
        Ok(_) => None,
        Err(e) => {
            error!("secrets::get_secret_scoped failed for app={app_id} workspace={} key={key}: {e}", workspace_root.display());
            None
        }
    }
}

/// Store a workspace-scoped secret.
///
/// **Hard invariant:** `workspace_root` must be a non-empty, absolute path.
#[cfg(target_os = "macos")]
pub fn set_secret_scoped(key: &str, value: &str, workspace_root: &Path) -> bool {
    use std::process::Command;

    if !validate_workspace_root(workspace_root, "set_secret_scoped", "—", key) {
        return false;
    }

    let account = account_key_scoped(key, workspace_root);

    // Delete existing entry first (ignore errors if not found)
    let _ = Command::new("security")
        .args(["delete-generic-password", "-s", SERVICE_NAME, "-a", &account])
        .output();

    match Command::new("security")
        .args(["add-generic-password", "-s", SERVICE_NAME, "-a", &account, "-w", value])
        .output()
    {
        Ok(output) if output.status.success() => true,
        Ok(output) => {
            error!(
                "secrets::set_secret_scoped failed for workspace={} key={key}: {}",
                workspace_root.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            );
            false
        }
        Err(e) => {
            error!("secrets::set_secret_scoped failed to run security CLI: {e}");
            false
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn get_secret_scoped(key: &str, app_id: &str, workspace_root: &Path) -> Option<Zeroizing<String>> {
    warn!("secrets::get_secret_scoped({key}, {app_id}, {}): Keychain not available on this platform", workspace_root.display());
    None
}

#[cfg(not(target_os = "macos"))]
pub fn set_secret_scoped(key: &str, _value: &str, workspace_root: &Path) -> bool {
    warn!("secrets::set_secret_scoped({key}, {}): Keychain not available on this platform", workspace_root.display());
    false
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

// ── Index file (keys only — values stay in Keychain) ──────────────────

fn index_path() -> std::path::PathBuf {
    crate::config::config_dir().join("secrets-index.json")
}

fn read_index() -> Vec<SecretEntry> {
    let path = index_path();
    match std::fs::read_to_string(&path) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_else(|e| {
            error!("secrets: failed to parse index at {:?}: {e}", path);
            Vec::new()
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => {
            error!("secrets: failed to read index: {e}");
            Vec::new()
        }
    }
}

fn write_index(entries: &[SecretEntry]) {
    let path = index_path();
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            error!("secrets: failed to create config dir: {e}");
            return;
        }
    }
    match serde_json::to_string_pretty(entries) {
        Ok(s) => {
            if let Err(e) = std::fs::write(&path, s) {
                error!("secrets: failed to write index: {e}");
            }
        }
        Err(e) => error!("secrets: failed to serialize index: {e}"),
    }
}

fn index_add(key: &str, app_id: &str, directory: &str) {
    let mut entries = read_index();
    // Remove any existing entry with the same triple to avoid duplicates.
    entries.retain(|e| !(e.key == key && e.app_id == app_id && e.directory == directory));
    entries.push(SecretEntry {
        app_id: app_id.to_string(),
        directory: directory.to_string(),
        key: key.to_string(),
        workspace_root: None, // v1/v2 legacy path — no workspace scoping
    });
    write_index(&entries);
}

fn index_remove(key: &str, app_id: &str, directory: &str) {
    let mut entries = read_index();
    entries.retain(|e| !(e.key == key && e.app_id == app_id && e.directory == directory));
    write_index(&entries);
}

// ── v1/v2 app-scoped secret API ───────────────────────────────────────────────
// v1/v2 app-scoped secret API. v3 uses get_secret_scoped/set_secret_scoped keyed by workspace_root.
// Call sites migrate in layer 3.

#[cfg(target_os = "macos")]
pub fn store_secret(key: &str, value: &str, app_id: &str, directory: &str) -> bool {
    use std::process::Command;

    let account = account_key(key, app_id, directory);

    // Delete existing entry first (ignore errors if it doesn't exist)
    let _ = Command::new("security")
        .args(["delete-generic-password", "-s", SERVICE_NAME, "-a", &account])
        .output();

    match Command::new("security")
        .args([
            "add-generic-password",
            "-s", SERVICE_NAME,
            "-a", &account,
            "-w", value,
        ])
        .output()
    {
        Ok(output) if output.status.success() => {
            index_add(key, app_id, directory);
            true
        }
        Ok(output) => {
            error!(
                "secrets::store_secret failed for account={account}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
            false
        }
        Err(e) => {
            error!("secrets::store_secret failed to run security CLI: {e}");
            false
        }
    }
}

#[cfg(target_os = "macos")]
pub fn retrieve_secret(key: &str, app_id: &str, directory: &str) -> Option<Zeroizing<String>> {
    use std::process::Command;

    let account = account_key(key, app_id, directory);

    match Command::new("security")
        .args([
            "find-generic-password",
            "-s", SERVICE_NAME,
            "-a", &account,
            "-w",
        ])
        .output()
    {
        Ok(output) if output.status.success() => {
            Some(Zeroizing::new(String::from_utf8_lossy(&output.stdout).trim().to_string()))
        }
        Ok(_) => None, // not found is normal, not an error
        Err(e) => {
            error!("secrets::retrieve_secret failed to run security CLI: {e}");
            None
        }
    }
}

#[cfg(target_os = "macos")]
pub fn delete_secret(key: &str, app_id: &str, directory: &str) -> bool {
    use std::process::Command;

    let account = account_key(key, app_id, directory);

    match Command::new("security")
        .args(["delete-generic-password", "-s", SERVICE_NAME, "-a", &account])
        .output()
    {
        Ok(output) if output.status.success() => {
            index_remove(key, app_id, directory);
            true
        }
        Ok(output) => {
            error!(
                "secrets::delete_secret failed for account={account}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
            false
        }
        Err(e) => {
            error!("secrets::delete_secret failed to run security CLI: {e}");
            false
        }
    }
}

/// List all secrets for a given app_id — reads from the index, no Keychain dump needed.
#[cfg(target_os = "macos")]
pub fn list_secrets(app_id: &str) -> Vec<String> {
    read_index()
        .into_iter()
        .filter(|e| e.app_id == app_id)
        .map(|e| account_key(&e.key, &e.app_id, &e.directory))
        .collect()
}

/// List every Plexi secret across all app_ids — reads from the index.
pub fn list_all_secrets() -> Vec<SecretEntry> {
    read_index()
}

/// Walk up from `launch_dir` to the user's home directory, returning the first
/// matching secret. Returns `None` if no match is found at any level.
#[cfg(target_os = "macos")]
pub fn resolve_secret(key: &str, app_id: &str, launch_dir: &str) -> Option<Zeroizing<String>> {
    use std::path::PathBuf;

    let home = match dirs::home_dir() {
        Some(h) => h,
        None => {
            warn!("secrets::resolve_secret: could not determine home directory");
            return None;
        }
    };

    let mut current = PathBuf::from(launch_dir);
    loop {
        let dir_str = current.to_string_lossy();
        if let Some(value) = retrieve_secret(key, app_id, &dir_str) {
            return Some(value);
        }

        if current == home {
            break;
        }

        match current.parent() {
            Some(parent) if parent != current => current = parent.to_path_buf(),
            _ => break,
        }
    }

    None
}

// ── Non-macOS stubs ────────────────────────────────────────────────────

#[cfg(not(target_os = "macos"))]
pub fn store_secret(key: &str, _value: &str, app_id: &str, directory: &str) -> bool {
    warn!("secrets::store_secret({key}, {app_id}, {directory}): Keychain not available on this platform");
    false
}

#[cfg(not(target_os = "macos"))]
pub fn retrieve_secret(key: &str, app_id: &str, directory: &str) -> Option<Zeroizing<String>> {
    warn!("secrets::retrieve_secret({key}, {app_id}, {directory}): Keychain not available on this platform");
    None
}

#[cfg(not(target_os = "macos"))]
pub fn delete_secret(key: &str, app_id: &str, directory: &str) -> bool {
    warn!("secrets::delete_secret({key}, {app_id}, {directory}): Keychain not available on this platform");
    false
}

#[cfg(not(target_os = "macos"))]
pub fn list_secrets(app_id: &str) -> Vec<String> {
    warn!("secrets::list_secrets({app_id}): Keychain not available on this platform");
    Vec::new()
}

#[cfg(not(target_os = "macos"))]
pub fn resolve_secret(key: &str, app_id: &str, launch_dir: &str) -> Option<Zeroizing<String>> {
    warn!("secrets::resolve_secret({key}, {app_id}, {launch_dir}): Keychain not available on this platform");
    None
}
