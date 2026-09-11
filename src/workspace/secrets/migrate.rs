//! One-shot startup migration of pre-#322 keychain entries.
//!
//! Non-destructive by construction: the signature takes
//! [`NonDestructiveStore`], so this layer cannot express an overwrite.

use super::store::NonDestructiveStore;

#[cfg(any(target_os = "macos", test))]
use super::store::SecretError;
#[cfg(all(target_os = "macos", not(test)))]
use super::{keychain_user_name, KEYCHAIN_SERVICE};

/// One-shot migration on startup: any legacy `plexi-run/.../<key>` Keychain
/// entry referenced in the old index gets re-stored under
/// `plexi:user:<key>` (the friendly name == the canonical name). Logs every
/// migration. Idempotent — re-runs are no-ops once `secrets-index.json` is
/// in the new flat-string form.
///
/// `not(test)`: this body holds the only direct Security.framework value
/// read (`get_generic_password`) outside `MacKeychain` in OUR code, so
/// compiling it out removes every keychain route our code offers a test
/// binary. That is a routing guarantee, not an access impossibility: any
/// test can still call the `security_framework` dependency directly (see
/// `src/workspace/AGENTS.md` — contract-banned; stint 0603 owns the real
/// close). (No test calls this; only `main()` does.)
#[cfg(all(target_os = "macos", not(test)))]
pub fn migrate_legacy_global_secrets(store: &dyn NonDestructiveStore) -> usize {
    use security_framework::passwords::get_generic_password;
    let path = super::index::index_path();
    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return 0,
    };
    // Already migrated? Flat-string schema parses; bail.
    if serde_json::from_str::<Vec<String>>(&raw).is_ok() {
        return 0;
    }
    let legacy: Vec<crate::secrets::SecretEntry> = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => return 0,
    };
    let mut migrated = 0;
    for entry in &legacy {
        // Read the legacy Keychain account: `{app_id}/{directory}/{key}`.
        let legacy_account = format!("{}/{}/{}", entry.app_id, entry.directory, entry.key);
        let value = match get_generic_password(KEYCHAIN_SERVICE, &legacy_account) {
            Ok(data) => String::from_utf8_lossy(&data).trim().to_string(),
            Err(e) if e.code() == -25300 => {
                continue; // legacy entry already gone; index is stale
            }
            Err(e) => {
                log::warn!("workspace_secrets::migrate: keychain error for {legacy_account}: {e}");
                continue;
            }
        };
        let new_account = keychain_user_name(&entry.key);
        if migrate_legacy_value(store, &legacy_account, &new_account, &value) {
            migrated += 1;
        }
    }
    migrated
}

/// Copy one legacy secret value to its new account, create-only. An existing
/// value under `new_account` is authoritative and is never overwritten — the
/// legacy Keychain item is left in place either way (this migration never
/// deletes). Returns whether the copy happened.
#[cfg(any(target_os = "macos", test))]
fn migrate_legacy_value(
    store: &dyn NonDestructiveStore,
    legacy_account: &str,
    new_account: &str,
    value: &str,
) -> bool {
    match store.add_new(new_account, value) {
        Ok(()) => {
            log::info!("workspace_secrets::migrate: {legacy_account} → {new_account}");
            true
        }
        Err(SecretError::AlreadyExists(_)) => {
            log::warn!(
                "workspace_secrets::migrate: '{new_account}' already exists — keeping its current \
                 value; legacy item '{legacy_account}' left in place"
            );
            false
        }
        Err(e) => {
            log::warn!("workspace_secrets::migrate: failed to write {new_account}: {e}");
            false
        }
    }
}

#[cfg(any(not(target_os = "macos"), test))]
pub fn migrate_legacy_global_secrets(_store: &dyn NonDestructiveStore) -> usize {
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::secrets::{InMemoryKeychain, SecretStore};

    #[test]
    fn legacy_global_migration_never_overwrites_an_existing_new_account() {
        // The startup migration (main.rs → migrate_legacy_global_secrets) used
        // to upsert: a legacy index entry whose new account already held a
        // different value silently overwrote it. Create-only refuses instead.
        let store = InMemoryKeychain::new();
        store.set("plexi:user:MY_KEY", "current-value").unwrap();

        let copied = migrate_legacy_value(
            &store,
            "old-app/dir/MY_KEY",
            "plexi:user:MY_KEY",
            "stale-legacy-value",
        );

        assert!(!copied, "a refused copy must not count as migrated");
        assert_eq!(
            store.get("plexi:user:MY_KEY").map(|v| v.to_string()),
            Some("current-value".to_string()),
            "an existing value must never be overwritten by the startup migration"
        );
    }
}
