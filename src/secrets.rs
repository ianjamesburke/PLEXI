use log::{error, warn};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

const SERVICE_NAME: &str = "plexi";

/// A parsed Keychain entry stored under service="plexi".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretEntry {
    pub app_id: String,
    pub directory: String,
    pub key: String,
}

/// Build the Keychain account string: "{app_id}/{directory}/{key}"
fn account_key(key: &str, app_id: &str, directory: &str) -> String {
    format!("{app_id}/{directory}/{key}")
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
    });
    write_index(&entries);
}

fn index_remove(key: &str, app_id: &str, directory: &str) {
    let mut entries = read_index();
    entries.retain(|e| !(e.key == key && e.app_id == app_id && e.directory == directory));
    write_index(&entries);
}

// ── macOS Keychain implementation ──────────────────────────────────────

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
