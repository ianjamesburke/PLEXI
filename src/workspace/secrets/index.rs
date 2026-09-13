//! `secrets-index.json`: an accounts-only cache of the keychain.
//!
//! Full account strings (`plexi:<scope>:<friendly>`) are the single natural
//! primary key — the workspace id is embedded in the account name. Replaces
//! the legacy `SecretEntry`-based index.
//!
//! The whole layer is compiled out under test — `mod index` is gated in
//! [`super`] — so a test binary has no route to the user's real index file.

use super::keychain_user_name;
use super::store::SecretError;

pub(super) fn index_path() -> std::path::PathBuf {
    crate::config::config_dir().join("secrets-index.json")
}

pub(super) fn index_read() -> Vec<String> {
    let path = index_path();
    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => {
            log::error!("workspace_secrets: failed to read index {path:?}: {e}");
            return Vec::new();
        }
    };
    // Try the new flat-string schema first.
    if let Ok(v) = serde_json::from_str::<Vec<String>>(&raw) {
        return v;
    }
    // Migration: old SecretEntry array. Convert legacy entries to
    // `plexi:user:<key>` (friendly name == canonical name; no workspace scope).
    // NB: the on-disk friendly name needs to match what `set_user_secret_cli`
    // writes, which is just the key itself. Keychain entries are migrated
    // separately by `migrate_legacy_global_secrets`.
    match serde_json::from_str::<Vec<crate::secrets::SecretEntry>>(&raw) {
        Ok(legacy) => {
            let migrated: Vec<String> = legacy
                .into_iter()
                .map(|e| keychain_user_name(&e.key))
                .collect();
            // Persist the migrated form so we don't re-do this every run.
            if let Err(e) = index_write(&migrated) {
                log::error!("workspace_secrets: failed to persist migrated index: {e}");
            }
            log::info!(
                "workspace_secrets: migrated {} legacy index entries to plexi:user:* form",
                migrated.len()
            );
            migrated
        }
        Err(e) => {
            log::error!("workspace_secrets: failed to parse index {path:?}: {e}");
            Vec::new()
        }
    }
}

pub(super) fn index_write(entries: &[String]) -> Result<(), SecretError> {
    let path = index_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            SecretError::Backend(format!("create config dir {}: {e}", parent.display()))
        })?;
    }
    let serialized = serde_json::to_string_pretty(entries)
        .map_err(|e| SecretError::Backend(format!("serialize secrets index: {e}")))?;
    std::fs::write(&path, serialized)
        .map_err(|e| SecretError::Backend(format!("write {}: {e}", path.display())))
}

/// Index maintenance that rides along with a Keychain write. The value is
/// already stored at this point, so an index failure is logged rather than
/// propagated — the index is a cache, and failing the write would tell the
/// caller their secret was not saved.
pub(super) fn index_add(account: &str) {
    let mut entries = index_read();
    if !entries.iter().any(|a| a == account) {
        entries.push(account.to_string());
        if let Err(e) = index_write(&entries) {
            log::error!("workspace_secrets: failed to index '{account}': {e}");
        }
    }
}

pub(super) fn index_remove(account: &str) {
    let mut entries = index_read();
    entries.retain(|a| a != account);
    if let Err(e) = index_write(&entries) {
        log::error!("workspace_secrets: failed to unindex '{account}': {e}");
    }
}
