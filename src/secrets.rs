use log::{error, warn};
use zeroize::Zeroizing;

const SERVICE_NAME: &str = "plexi";

/// Build the Keychain account string: "{app_id}/{directory}/{key}"
fn account_key(key: &str, app_id: &str, directory: &str) -> String {
    format!("{app_id}/{directory}/{key}")
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
        Ok(output) if output.status.success() => true,
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
        Ok(output) if output.status.success() => true,
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

#[cfg(target_os = "macos")]
pub fn list_secrets(app_id: &str) -> Vec<String> {
    use std::process::Command;

    // `security dump-keychain` dumps all items. We parse it immediately into
    // key names and drop the raw dump string before returning, so the full
    // keychain contents don't linger in memory longer than necessary.
    let result = {
        match Command::new("security")
            .args(["dump-keychain"])
            .output()
        {
            Ok(output) if output.status.success() => {
                let text = String::from_utf8_lossy(&output.stdout);
                let keys = parse_keychain_dump(&text, app_id);
                // `text` (the raw dump) is dropped here as the block ends
                keys
            }
            Ok(output) => {
                error!(
                    "secrets::list_secrets failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                );
                Vec::new()
            }
            Err(e) => {
                error!("secrets::list_secrets failed to run security CLI: {e}");
                Vec::new()
            }
        }
    };
    result
}

#[cfg(target_os = "macos")]
fn parse_keychain_dump(dump: &str, app_id: &str) -> Vec<String> {
    let mut results = Vec::new();
    let mut in_plexi_entry = false;
    let prefix = format!("{app_id}/");

    for line in dump.lines() {
        let trimmed = line.trim();

        // Detect service match
        if trimmed.contains("svce")
            && trimmed.contains(&format!("\"{}\"", SERVICE_NAME))
        {
            in_plexi_entry = true;
            continue;
        }

        // If we're inside a matching entry, look for the account field
        if in_plexi_entry && trimmed.contains("acct") {
            // Format: "acct"<blob>="app_id/dir/key"
            if let Some(start) = trimmed.rfind('=') {
                let account = trimmed[start + 1..].trim().trim_matches('"');
                if account.starts_with(&prefix) {
                    results.push(account.to_string());
                }
            }
            in_plexi_entry = false;
        }

        // Reset on new keychain entry boundary
        if trimmed.starts_with("keychain:") || trimmed.starts_with("class:") {
            in_plexi_entry = false;
        }
    }

    results
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

        // Stop if we've reached home (or gone past it)
        if current == home {
            break;
        }

        // Move to parent
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
    warn!(
        "secrets::store_secret({key}, {app_id}, {directory}): Keychain not available on this platform"
    );
    false
}

#[cfg(not(target_os = "macos"))]
pub fn retrieve_secret(key: &str, app_id: &str, directory: &str) -> Option<Zeroizing<String>> {
    warn!(
        "secrets::retrieve_secret({key}, {app_id}, {directory}): Keychain not available on this platform"
    );
    None
}

#[cfg(not(target_os = "macos"))]
pub fn delete_secret(key: &str, app_id: &str, directory: &str) -> bool {
    warn!(
        "secrets::delete_secret({key}, {app_id}, {directory}): Keychain not available on this platform"
    );
    false
}

#[cfg(not(target_os = "macos"))]
pub fn list_secrets(app_id: &str) -> Vec<String> {
    warn!(
        "secrets::list_secrets({app_id}): Keychain not available on this platform"
    );
    Vec::new()
}

#[cfg(not(target_os = "macos"))]
pub fn resolve_secret(key: &str, app_id: &str, launch_dir: &str) -> Option<Zeroizing<String>> {
    warn!(
        "secrets::resolve_secret({key}, {app_id}, {launch_dir}): Keychain not available on this platform"
    );
    None
}
