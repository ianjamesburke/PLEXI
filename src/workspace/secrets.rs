//! Workspace-scoped secret routing (issue #322).
//!
//! Three layers:
//! 1. **Keychain** stores raw values under namespaced names:
//!    `plexi:<workspace-id>:<friendly-name>` for workspace-scoped, and
//!    `plexi:user:<friendly-name>` for cross-workspace fallback.
//! 2. **App manifest** declares canonical secret names — the app calls
//!    `ctx.secret("OPENAI_API_KEY")` but never knows the friendly Keychain name.
//! 3. **Workspace router** at `<workspace_root>/.plexi/secrets.toml` maps
//!    canonical names per-app (and a shared `[default]` route) to friendly
//!    Keychain names. Plus a required `fallback` flag controlling whether
//!    `plexi:user:*` is consulted on a miss.
//!
//! ## Runtime resolution (4-step order)
//!
//! When app `<app-id>` in workspace `<root>` calls `ctx.secret("X")`:
//! 1. `[apps.<app-id>] X = "fname"` → return `plexi:<workspace-id>:fname`.
//! 2. `[default] X = "fname"` → return `plexi:<workspace-id>:fname`.
//! 3. `fallback = true` AND `plexi:user:X` exists → return user-scope value.
//! 4. Else: missing-secret prompt (or hard error if `fallback = false` and no
//!    route is defined). Out-of-band of this module — resolved by the host at
//!    app launch, not here.

use serde::Deserialize;
use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use zeroize::Zeroizing;

// ── Keychain naming ──────────────────────────────────────────────────────────

/// Keychain generic-password service every Plexi secret is stored under.
pub const KEYCHAIN_SERVICE: &str = "plexi";

/// Build the workspace-namespaced Keychain account: `plexi:<workspace-id>:<friendly>`.
pub fn keychain_workspace_name(workspace_id: &str, friendly: &str) -> String {
    format!("plexi:{workspace_id}:{friendly}")
}

/// Build the user-scope (cross-workspace) Keychain account: `plexi:user:<friendly>`.
pub fn keychain_user_name(friendly: &str) -> String {
    format!("plexi:user:{friendly}")
}

// ── SecretStore trait + impls ────────────────────────────────────────────────

/// The non-destructive storage surface — the **only** trait migration and
/// reconciliation code takes. Invariant: no method on this trait can overwrite
/// or unconditionally destroy a value the caller did not write, so a
/// destructive op in a migration is a compile error, not a review catch.
/// `account` is the full namespaced key (e.g. `plexi:abc-123:openai_prod`).
pub trait NonDestructiveStore: Send + Sync {
    fn get(&self, account: &str) -> Option<Zeroizing<String>>;
    /// Create-only write: stores `value` **only** if `account` does not
    /// already exist, and returns [`SecretError::AlreadyExists`] if it does.
    /// Never updates — the backend itself refuses the duplicate, so the check
    /// and the write cannot race.
    fn add_new(&self, account: &str, value: &str) -> Result<(), SecretError>;
    /// Value-guarded delete: removes `account` only while it still holds
    /// exactly `expected`. Any other stored value returns
    /// [`SecretError::ValueChanged`] and leaves the item untouched; an
    /// already-missing item is success. See each impl for its atomicity.
    fn delete_if_value(&self, account: &str, expected: &str) -> Result<(), SecretError>;
    /// Best-effort listing of accounts with a given prefix. Used by the host
    /// to enumerate `plexi:<workspace-id>:*` entries for the missing-secret
    /// modal. macOS Keychain has no clean prefix-list — production impl
    /// reads from `secrets-index.json`.
    fn list_with_prefix(&self, prefix: &str) -> Vec<String>;
    /// Enumerate every account in the backend itself, bypassing the index
    /// cache. Attributes-only on macOS — never reads values, so it never
    /// crosses the keychain ACL prompt boundary (value reads of items another
    /// binary wrote are what prompt; attribute enumeration does not).
    fn scan_accounts(&self) -> Result<Vec<String>, SecretError>;
}

/// The full store surface. Adds the destructive ops — upsert and
/// unconditional delete — that only user-initiated flows (the Secrets app
/// editor, `plexi secret set`/`delete`) may express. Migration and
/// reconciliation signatures take [`NonDestructiveStore`] and cannot name
/// these methods.
pub trait SecretStore: NonDestructiveStore {
    fn set(&self, account: &str, value: &str) -> Result<(), SecretError>;
    fn delete(&self, account: &str) -> Result<(), SecretError>;
}

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("keychain backend error: {0}")]
    Backend(String),
    /// A create-only write lost the race: the account already exists. Callers
    /// must treat the existing value as authoritative and never overwrite it.
    #[error("keychain account already exists: {0}")]
    AlreadyExists(String),
    /// A value-guarded delete refused: the account no longer holds the value
    /// the caller copied. Callers must keep the item and report a conflict.
    #[error("keychain account value changed since it was read: {0}")]
    ValueChanged(String),
}

/// The process-wide secret store handle — the ONLY way to reach a store
/// backend. Production builds return the real macOS Keychain; test builds
/// ALWAYS return a process-local in-memory store, and the real backend type
/// is not even compiled under `cfg(test)`, so a test that tries to name it
/// does not build. Default-safe, opt-in-dangerous — except the opt-in does
/// not exist inside a test binary.
///
/// Why (2026-07-28): macOS keychain ACLs are per-binary, so every freshly
/// compiled test binary is a new unsigned app and each login-keychain value
/// read from a test fires its own credential dialog — an unattended agent
/// gate cannot click one, so a prompting test silently stalls automation.
#[cfg(target_os = "macos")]
pub fn system_store() -> &'static dyn SecretStore {
    #[cfg(not(test))]
    {
        static STORE: MacKeychain = MacKeychain;
        &STORE
    }
    #[cfg(test)]
    {
        static STORE: std::sync::OnceLock<InMemoryKeychain> = std::sync::OnceLock::new();
        STORE.get_or_init(InMemoryKeychain::new)
    }
}

/// macOS Keychain backend via `security-framework`.
///
/// Maintains `~/.plexi-<channel>/secrets-index.json` so list operations work
/// without invoking `security dump-keychain` (which triggers an invisible
/// permission prompt). See DEV_LOG 2026-04-11.
///
/// Private, non-constructible outside this module, and absent from test
/// builds entirely — [`system_store`] is the only handle.
#[cfg(all(target_os = "macos", not(test)))]
struct MacKeychain;

#[cfg(all(target_os = "macos", not(test)))]
impl NonDestructiveStore for MacKeychain {
    fn get(&self, account: &str) -> Option<Zeroizing<String>> {
        use security_framework::passwords::get_generic_password;
        match get_generic_password(KEYCHAIN_SERVICE, account) {
            Ok(data) => Some(Zeroizing::new(
                String::from_utf8_lossy(&data).trim().to_string(),
            )),
            Err(e) if e.code() == -25300 => None,
            Err(e) => {
                log::warn!(
                    "workspace_secrets::MacKeychain::get: keychain error for account={account}: {e}"
                );
                None
            }
        }
    }

    fn add_new(&self, account: &str, value: &str) -> Result<(), SecretError> {
        use core_foundation::data::CFData;
        use security_framework::item::{ItemAddOptions, ItemAddValue, ItemClass, Location};

        // `SecItemAdd` (via `ItemAddOptions::add`) is create-only and reports
        // `errSecDuplicateItem` for an existing account. `set_generic_password`
        // cannot be used here: it upserts, silently rewriting the duplicate.
        let result = ItemAddOptions::new(ItemAddValue::Data {
            class: ItemClass::generic_password(),
            data: CFData::from_buffer(value.as_bytes()),
        })
        .set_service(KEYCHAIN_SERVICE)
        .set_account_name(account)
        .set_location(Location::DefaultFileKeychain)
        .add();

        match result {
            Ok(()) => {
                index_add(account);
                Ok(())
            }
            // errSecDuplicateItem
            Err(e) if e.code() == -25299 => Err(SecretError::AlreadyExists(account.to_string())),
            Err(e) => Err(SecretError::Backend(format!(
                "create-only add of '{account}' failed: {e}"
            ))),
        }
    }

    fn delete_if_value(&self, account: &str, expected: &str) -> Result<(), SecretError> {
        // Read-compare-delete. macOS Security.framework has no atomic
        // compare-and-delete (and no multi-item transaction), so the guard is
        // best-effort: a cross-process write landing in the one-syscall gap
        // between this read and the delete below can still be lost. The
        // in-memory test impl IS atomic; this one is honestly not, and the
        // residual window is irreducible — do not document it as closed.
        match self.get(account) {
            None => Ok(()), // already gone — nothing to lose
            Some(current) if current.as_str() == expected => {
                use security_framework::passwords::delete_generic_password;
                match delete_generic_password(KEYCHAIN_SERVICE, account) {
                    Ok(()) => {}
                    // Already gone — treat as success.
                    Err(e) if e.code() == -25300 => {}
                    Err(e) => return Err(SecretError::Backend(format!("{e}"))),
                }
                index_remove(account);
                Ok(())
            }
            Some(_) => Err(SecretError::ValueChanged(account.to_string())),
        }
    }

    fn list_with_prefix(&self, prefix: &str) -> Vec<String> {
        index_read()
            .into_iter()
            .filter(|a| a.starts_with(prefix))
            .collect()
    }

    /// Attributes-only: the query asks for `kSecReturnAttributes` and never
    /// `kSecReturnData`, so it reads item metadata without unlocking any
    /// value and never raises a keychain-access prompt.
    fn scan_accounts(&self) -> Result<Vec<String>, SecretError> {
        use security_framework::item::{ItemClass, ItemSearchOptions, Limit};

        let results = match ItemSearchOptions::new()
            .class(ItemClass::generic_password())
            .service(KEYCHAIN_SERVICE)
            .load_attributes(true)
            .limit(Limit::All)
            .search()
        {
            Ok(results) => results,
            // errSecItemNotFound — no Plexi secrets stored yet.
            Err(e) if e.code() == -25300 => return Ok(Vec::new()),
            Err(e) => {
                return Err(SecretError::Backend(format!(
                    "keychain scan for service '{KEYCHAIN_SERVICE}' failed: {e}"
                )))
            }
        };

        let mut accounts = Vec::with_capacity(results.len());
        for result in &results {
            match result.simplify_dict().and_then(|d| d.get("acct").cloned()) {
                Some(account) => accounts.push(account),
                None => log::warn!(
                    "workspace_secrets::scan: keychain item under service '{KEYCHAIN_SERVICE}' \
                     has no account attribute; skipping"
                ),
            }
        }
        log::info!(
            "workspace_secrets::scan: found {} keychain item(s) under service '{KEYCHAIN_SERVICE}'",
            accounts.len()
        );
        Ok(accounts)
    }
}

#[cfg(all(target_os = "macos", not(test)))]
impl SecretStore for MacKeychain {
    fn set(&self, account: &str, value: &str) -> Result<(), SecretError> {
        use security_framework::passwords::set_generic_password;
        set_generic_password(KEYCHAIN_SERVICE, account, value.as_bytes())
            .map_err(|e| SecretError::Backend(format!("{e}")))?;
        index_add(account);
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<(), SecretError> {
        use security_framework::passwords::delete_generic_password;
        match delete_generic_password(KEYCHAIN_SERVICE, account) {
            Ok(()) => {}
            // Already gone — treat as success.
            Err(e) if e.code() == -25300 => {}
            Err(e) => return Err(SecretError::Backend(format!("{e}"))),
        }
        index_remove(account);
        Ok(())
    }
}

// ── Workspace-secret index file (accounts only, no values) ───────────────────
//
// Replaces the legacy SecretEntry-based `secrets-index.json`. We store full
// account strings (`plexi:<scope>:<friendly>`) because that's the single
// natural primary key — workspace_id is embedded in the account name.

// The whole index-file layer is compiled out under test: with `MacKeychain`
// absent and the legacy migration stubbed, nothing in a test binary can reach
// it (the compiler proves this — these go dead-code without the cfg), so a
// test can never read or rewrite the user's real secrets-index.json.
#[cfg(all(target_os = "macos", not(test)))]
fn index_path() -> std::path::PathBuf {
    crate::config::config_dir().join("secrets-index.json")
}

#[cfg(all(target_os = "macos", not(test)))]
fn index_read() -> Vec<String> {
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

#[cfg(all(target_os = "macos", not(test)))]
fn index_write(entries: &[String]) -> Result<(), SecretError> {
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
#[cfg(all(target_os = "macos", not(test)))]
fn index_add(account: &str) {
    let mut entries = index_read();
    if !entries.iter().any(|a| a == account) {
        entries.push(account.to_string());
        if let Err(e) = index_write(&entries) {
            log::error!("workspace_secrets: failed to index '{account}': {e}");
        }
    }
}

#[cfg(all(target_os = "macos", not(test)))]
fn index_remove(account: &str) {
    let mut entries = index_read();
    entries.retain(|a| a != account);
    if let Err(e) = index_write(&entries) {
        log::error!("workspace_secrets: failed to unindex '{account}': {e}");
    }
}

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
    let path = index_path();
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

// ── Index ↔ keychain reconciliation ──────────────────────────────────────────
//
// `secrets-index.json` is a cache of the keychain, written only when a secret
// goes through `SecretStore::set`. A key that reached the keychain any other
// way (`security add-generic-password`, a pre-index build) is readable by the
// resolver but invisible to every listing. Reconciliation makes the keychain
// the source of truth: the index is rebuilt from a real scan, and legacy
// account spellings are collapsed onto one canonical name.

/// Legacy → canonical friendly-name spellings. One canonical form per secret;
/// anything on the left is migrated to the right on the next reconcile.
const FRIENDLY_NAME_ALIASES: &[(&str, &str)] = &[("openrouter-api-key", "OPENROUTER_API_KEY")];

/// Split `plexi:<scope>:<friendly>` into its scope and friendly parts. `None`
/// for any account that is not in the namespaced form (e.g. the pre-#322
/// `plexi-run/<dir>/<key>` accounts, which `migrate_legacy_global_secrets`
/// owns).
fn split_account(account: &str) -> Option<(&str, &str)> {
    account.strip_prefix("plexi:")?.split_once(':')
}

/// The canonical spelling for a friendly name, or `None` when `friendly` is
/// already canonical. The single authority for legacy spellings — every code
/// path that interprets a persisted friendly name (keychain accounts, route
/// values in `secrets.toml`) resolves it through this table.
fn canonical_friendly(friendly: &str) -> Option<&'static str> {
    FRIENDLY_NAME_ALIASES
        .iter()
        .find(|(legacy, _)| *legacy == friendly)
        .map(|(_, canonical)| *canonical)
}

/// The canonical account for `account`, or `None` when it is already canonical
/// (or not a namespaced Plexi account).
pub fn canonical_account(account: &str) -> Option<String> {
    let (scope, friendly) = split_account(account)?;
    let canonical = canonical_friendly(friendly)?;
    Some(format!("plexi:{scope}:{canonical}"))
}

/// One legacy→canonical account migration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountRename {
    pub from: String,
    pub to: String,
}

/// Outcome of one reconcile pass. Every field is reported to the caller so the
/// UI can surface what changed; `index` is the contents the caller must persist.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ReconcileReport {
    /// Keychain accounts that were missing from the index and are now listed.
    pub adopted: Vec<String>,
    /// Index entries with no backing keychain item — dropped as stale.
    pub stale: Vec<String>,
    /// Legacy spellings migrated to their canonical account.
    pub renamed: Vec<AccountRename>,
    /// Legacy spellings whose canonical account already holds a value.
    /// Migrating would destroy one of them, so both are left alone.
    pub conflicts: Vec<AccountRename>,
    /// Keychain accounts under the Plexi service that are not in namespaced
    /// form — never adopted into the index.
    pub ignored: Vec<String>,
    /// Reconciled index: every namespaced account that really exists, canonical.
    pub index: Vec<String>,
}

impl ReconcileReport {
    /// True when the pass left the index exactly as it found it.
    pub fn is_noop(&self) -> bool {
        self.adopted.is_empty()
            && self.stale.is_empty()
            && self.renamed.is_empty()
            && self.conflicts.is_empty()
    }
}

/// Reconcile `index` against a real keychain scan.
///
/// `scanned` is every account under [`KEYCHAIN_SERVICE`]. Alias migration is
/// the only step that touches secret values, and it goes through `store` so
/// tests can drive it without the real keychain. The returned `index` is what
/// the caller persists — this function never writes the index file itself.
pub fn reconcile(
    scanned: &[String],
    index: &[String],
    store: &dyn NonDestructiveStore,
) -> ReconcileReport {
    let mut report = ReconcileReport::default();

    let mut live: BTreeSet<String> = BTreeSet::new();
    for account in scanned {
        if split_account(account).is_some() {
            live.insert(account.clone());
        } else if !report.ignored.iter().any(|a| a == account) {
            report.ignored.push(account.clone());
        }
    }
    if !report.ignored.is_empty() {
        log::info!(
            "workspace_secrets::reconcile: ignoring {} non-namespaced keychain account(s) under service '{KEYCHAIN_SERVICE}'",
            report.ignored.len()
        );
    }

    // Alias normalization. Runs before the adopt/stale diff so renamed
    // accounts are reported as renames, not as an adopt + a stale.
    for account in live.clone() {
        let Some(canonical) = canonical_account(&account) else {
            continue;
        };
        let rename = AccountRename {
            from: account.clone(),
            to: canonical.clone(),
        };
        if live.contains(&canonical) {
            log::warn!(
                "workspace_secrets::reconcile: both '{}' and canonical '{}' exist in the keychain — \
                 leaving both in place; delete the legacy one manually",
                rename.from,
                rename.to
            );
            report.conflicts.push(rename);
            continue;
        }
        let Some(value) = store.get(&account) else {
            log::warn!(
                "workspace_secrets::reconcile: could not read '{account}' to migrate it to '{canonical}' \
                 (keychain access denied or item removed mid-scan) — leaving it under the legacy name"
            );
            continue;
        };
        // Create-only. The `live` check above is a snapshot; this is the guard
        // that actually holds, because the Keychain itself refuses a duplicate.
        match store.add_new(&canonical, &value) {
            Ok(()) => {}
            Err(SecretError::AlreadyExists(_)) => {
                log::warn!(
                    "workspace_secrets::reconcile: canonical account '{canonical}' already exists \
                     (created after the scan) — leaving both it and '{account}' untouched; delete \
                     the legacy one manually"
                );
                live.insert(canonical);
                report.conflicts.push(rename);
                continue;
            }
            Err(e) => {
                log::warn!(
                    "workspace_secrets::reconcile: failed to write canonical account '{canonical}': {e} — \
                     leaving '{account}' in place"
                );
                continue;
            }
        }
        // Verified read-back before anything destructive. The legacy account is
        // the only copy of the secret until the canonical one provably holds
        // the same value.
        match store.get(&canonical) {
            Some(written) if written.as_str() == value.as_str() => {}
            _ => {
                log::warn!(
                    "workspace_secrets::reconcile: '{canonical}' did not read back the value written \
                     from '{account}' — keeping the legacy account; resolve the duplicate manually"
                );
                live.insert(canonical);
                report.conflicts.push(rename);
                continue;
            }
        }
        // Value-guarded delete: the legacy item is removed only while it still
        // holds exactly the value that was copied. A write that landed since
        // the read above refuses the delete instead of being lost. The
        // symmetric race — the canonical item being deleted between the
        // read-back above and this call — is NOT closed: the Keychain has no
        // multi-item transaction, and that syscall-wide window is irreducible.
        match store.delete_if_value(&account, &value) {
            Ok(()) => {}
            Err(SecretError::ValueChanged(_)) => {
                log::warn!(
                    "workspace_secrets::reconcile: legacy account '{account}' changed value after \
                     it was copied to '{canonical}' — keeping both; resolve the duplicate manually"
                );
                live.insert(canonical);
                report.conflicts.push(rename);
                continue;
            }
            Err(e) => {
                // The copy landed, so the canonical account now really exists —
                // but so does the legacy one. Report a conflict rather than a
                // completed rename; hiding a live keychain item is the exact bug
                // this reconciliation exists to fix.
                log::warn!(
                    "workspace_secrets::reconcile: value copied to '{canonical}' but legacy account \
                     '{account}' could not be deleted: {e} — both spellings now exist; delete the \
                     legacy one manually"
                );
                live.insert(canonical);
                report.conflicts.push(rename);
                continue;
            }
        }
        log::info!(
            "workspace_secrets::reconcile: normalized keychain account '{}' → '{}'",
            rename.from,
            rename.to
        );
        live.remove(&rename.from);
        live.insert(rename.to.clone());
        report.renamed.push(rename);
    }

    let indexed: BTreeSet<&str> = index.iter().map(String::as_str).collect();
    for account in &live {
        if indexed.contains(account.as_str()) || report.renamed.iter().any(|r| &r.to == account) {
            continue;
        }
        log::info!(
            "workspace_secrets::reconcile: adopted keychain account '{account}' that was missing from the index"
        );
        report.adopted.push(account.clone());
    }
    for account in &indexed {
        if live.contains(*account) || report.renamed.iter().any(|r| r.from == **account) {
            continue;
        }
        log::warn!(
            "workspace_secrets::reconcile: index entry '{account}' has no keychain item — dropping as stale"
        );
        report.stale.push((*account).to_string());
    }

    report.index = live.into_iter().collect();
    log::info!(
        "workspace_secrets::reconcile: {} account(s) live — {} adopted, {} stale, {} renamed, {} conflict(s)",
        report.index.len(),
        report.adopted.len(),
        report.stale.len(),
        report.renamed.len(),
        report.conflicts.len()
    );
    report
}

/// Scan the store backend, reconcile `secrets-index.json` against it, and
/// persist the result. Called when the Secrets app loads or is refreshed.
#[cfg(all(target_os = "macos", not(test)))]
pub fn reconcile_index_with_keychain() -> Result<ReconcileReport, SecretError> {
    let store = system_store();
    let scanned = store.scan_accounts()?;
    let index = index_read();
    let report = reconcile(&scanned, &index, store);
    // Persist only when the index contents actually changed — a standing
    // conflict makes `is_noop` permanently false and must not rewrite an
    // unchanged file on every load.
    if report.index != index {
        index_write(&report.index)?;
        log::info!(
            "workspace_secrets::reconcile: rewrote secrets index with {} account(s)",
            report.index.len()
        );
    }
    Ok(report)
}

/// Test variant. The index-file layer is compiled out of test binaries
/// entirely — its only possible target is the user's real
/// `secrets-index.json` — so there is no file to read back or persist and the
/// scan is the whole truth. Keeps the Secrets app's load path callable under
/// test without giving a test binary a route to the real index.
#[cfg(all(target_os = "macos", test))]
pub fn reconcile_index_with_keychain() -> Result<ReconcileReport, SecretError> {
    let store = system_store();
    let scanned = store.scan_accounts()?;
    Ok(reconcile(&scanned, &[], store))
}

/// Pure in-memory `SecretStore` for tests. Wraps a `Mutex<HashMap>` for
/// interior mutability so tests can share a single instance behind `&dyn`.
#[cfg(test)]
pub struct InMemoryKeychain {
    store: std::sync::Mutex<HashMap<String, String>>,
    /// Models a Keychain that serves reads and writes but refuses removal.
    delete_fails: bool,
    /// Account whose reads always miss, however it was written.
    unreadable: Option<String>,
    /// `(account, stale_value)` — the FIRST read of `account` returns
    /// `stale_value` instead of the stored value. Models a cross-process
    /// write landing between a caller's read and its later guarded delete.
    stale_read: std::sync::Mutex<Option<(String, String)>>,
}

#[cfg(test)]
impl InMemoryKeychain {
    pub fn new() -> Self {
        Self {
            store: std::sync::Mutex::new(HashMap::new()),
            delete_fails: false,
            unreadable: None,
            stale_read: std::sync::Mutex::new(None),
        }
    }

    /// Models a concurrent writer: the first read of `account` returns
    /// `stale_value`; the store's real contents are what later reads (and the
    /// guarded delete) see.
    pub fn with_stale_read(account: &str, stale_value: &str) -> Self {
        Self {
            stale_read: std::sync::Mutex::new(Some((account.to_string(), stale_value.to_string()))),
            ..Self::new()
        }
    }

    pub fn with_failing_delete() -> Self {
        Self {
            delete_fails: true,
            ..Self::new()
        }
    }

    /// Models a Keychain whose write appears to succeed but whose read-back of
    /// `account` does not return the value that was written.
    pub fn with_unreadable_account(account: &str) -> Self {
        Self {
            unreadable: Some(account.to_string()),
            ..Self::new()
        }
    }
}

#[cfg(test)]
impl Default for InMemoryKeychain {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
impl NonDestructiveStore for InMemoryKeychain {
    fn get(&self, account: &str) -> Option<Zeroizing<String>> {
        if self.unreadable.as_deref() == Some(account) {
            return None;
        }
        if let Ok(mut hook) = self.stale_read.lock() {
            if hook.as_ref().is_some_and(|(a, _)| a == account) {
                let (_, stale) = hook.take().expect("checked above");
                return Some(Zeroizing::new(stale));
            }
        }
        self.store
            .lock()
            .ok()?
            .get(account)
            .cloned()
            .map(Zeroizing::new)
    }

    fn add_new(&self, account: &str, value: &str) -> Result<(), SecretError> {
        let mut g = self
            .store
            .lock()
            .map_err(|e| SecretError::Backend(format!("mutex poisoned: {e}")))?;
        if g.contains_key(account) {
            return Err(SecretError::AlreadyExists(account.to_string()));
        }
        g.insert(account.to_string(), value.to_string());
        Ok(())
    }

    fn delete_if_value(&self, account: &str, expected: &str) -> Result<(), SecretError> {
        if self.delete_fails {
            return Err(SecretError::Backend(
                "delete refused by test store".to_string(),
            ));
        }
        // Genuinely atomic under the store lock — unlike MacKeychain's
        // read-compare-delete, no window exists here.
        let mut g = self
            .store
            .lock()
            .map_err(|e| SecretError::Backend(format!("mutex poisoned: {e}")))?;
        match g.get(account) {
            None => Ok(()),
            Some(current) if current == expected => {
                g.remove(account);
                Ok(())
            }
            Some(_) => Err(SecretError::ValueChanged(account.to_string())),
        }
    }

    fn list_with_prefix(&self, prefix: &str) -> Vec<String> {
        let g = match self.store.lock() {
            Ok(g) => g,
            Err(_) => return Vec::new(),
        };
        g.keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect()
    }

    fn scan_accounts(&self) -> Result<Vec<String>, SecretError> {
        let g = self
            .store
            .lock()
            .map_err(|e| SecretError::Backend(format!("mutex poisoned: {e}")))?;
        Ok(g.keys().cloned().collect())
    }
}

#[cfg(test)]
impl SecretStore for InMemoryKeychain {
    fn set(&self, account: &str, value: &str) -> Result<(), SecretError> {
        let mut g = self
            .store
            .lock()
            .map_err(|e| SecretError::Backend(format!("mutex poisoned: {e}")))?;
        g.insert(account.to_string(), value.to_string());
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<(), SecretError> {
        if self.delete_fails {
            return Err(SecretError::Backend(
                "delete refused by test store".to_string(),
            ));
        }
        let mut g = self
            .store
            .lock()
            .map_err(|e| SecretError::Backend(format!("mutex poisoned: {e}")))?;
        g.remove(account);
        Ok(())
    }
}

// ── workspace.toml (minimal — just `id`) ─────────────────────────────────────

#[derive(Deserialize, Debug, Clone)]
pub struct WorkspaceConfig {
    pub id: String,
    /// Optional `[context]` section — default name/description for the root
    /// context when this workspace is first opened. User overrides always win.
    #[serde(default)]
    pub context: Option<WorkspaceContextConfig>,
}

/// `[context]` section in workspace.toml. Provides default name and
/// description for the anchor's root context. Both fields are optional.
#[derive(Deserialize, Debug, Clone)]
pub struct WorkspaceContextConfig {
    pub name: Option<String>,
    pub description: Option<String>,
}

impl WorkspaceConfig {
    /// Read `<root>/<channel_dir>/workspace.toml`. Returns `None` if the file does
    /// not exist; returns `Err` only on a present-but-invalid file.
    pub fn load(workspace_root: &Path) -> Result<Option<Self>, String> {
        let path = workspace_root
            .join(crate::config::workspace_channel_dir())
            .join("workspace.toml");
        let raw = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                return Err(format!("read {}: {e}", path.display()));
            }
        };
        toml::from_str::<WorkspaceConfig>(&raw)
            .map(Some)
            .map_err(|e| format!("parse {}: {e}", path.display()))
    }
}

// ── secrets.toml (router) ────────────────────────────────────────────────────

/// Parsed `<workspace_root>/.plexi/secrets.toml`. The `fallback` field has
/// **no** serde default — a missing-fallback file is rejected loudly so
/// users have to declare their stance explicitly.
#[derive(Deserialize, Debug, Clone)]
pub struct WorkspaceSecrets {
    pub fallback: bool,
    #[serde(default)]
    pub apps: HashMap<String, HashMap<String, String>>,
    #[serde(default)]
    pub default: HashMap<String, String>,
    #[serde(default)]
    pub terminal: TerminalSecrets,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct TerminalSecrets {
    #[serde(default)]
    pub env: TerminalEnvSecrets,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct TerminalEnvSecrets {
    #[serde(default)]
    pub inject: Vec<String>,
}

impl WorkspaceSecrets {
    pub fn parse(raw: &str) -> Result<Self, String> {
        toml::from_str::<Self>(raw).map_err(|e| format!("parse secrets.toml: {e}"))
    }

    /// Read `<root>/<channel_dir>/secrets.toml`. Returns `None` if the file does
    /// not exist; returns `Err` if it exists but is malformed (incl. missing
    /// `fallback`).
    pub fn load(workspace_root: &Path) -> Result<Option<Self>, String> {
        let path = workspace_root
            .join(crate::config::workspace_channel_dir())
            .join("secrets.toml");
        let raw = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("read {}: {e}", path.display())),
        };
        Self::parse(&raw)
            .map(Some)
            .map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Look up a route for `(app_id, canonical_name)`. Returns the friendly
    /// Keychain name when an explicit `[apps.<app_id>]` route exists, or a
    /// `[default]` route otherwise. `None` means no route is defined.
    pub fn route_for(&self, app_id: &str, canonical_name: &str) -> Option<&str> {
        if let Some(app_routes) = self.apps.get(app_id) {
            if let Some(friendly) = app_routes.get(canonical_name) {
                return Some(friendly.as_str());
            }
        }
        self.default.get(canonical_name).map(|s| s.as_str())
    }
}

// ── Resolution result ────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum ResolveOutcome {
    /// Found a value via app/default route or user-scope fallback.
    Found(Zeroizing<String>),
    /// `fallback = false` and no route defined — surface as a hard in-pane
    /// error. The host should NOT show a "create new" modal.
    HardMissing { reason: String },
    /// Route defined but no Keychain entry, OR no route + `fallback = true`
    /// + no user-scope entry. Show the missing-secret prompt modal.
    PromptUser,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedSecretSource {
    WorkspaceRoute,
    WorkspaceCanonical,
    GlobalCanonical,
}

#[derive(Debug)]
pub struct ResolvedSecret {
    pub value: Zeroizing<String>,
    pub source: ResolvedSecretSource,
}

/// 4-step runtime resolution. Pure function — no I/O beyond the `SecretStore`
/// trait calls. Tests use `InMemoryKeychain`; production uses `MacKeychain`.
pub fn resolve(
    workspace_id: &str,
    app_id: &str,
    canonical_name: &str,
    router: &WorkspaceSecrets,
    store: &dyn NonDestructiveStore,
) -> ResolveOutcome {
    match resolve_with_source(workspace_id, app_id, canonical_name, router, store) {
        ResolveWithSourceOutcome::Found(found) => ResolveOutcome::Found(found.value),
        ResolveWithSourceOutcome::HardMissing { reason } => ResolveOutcome::HardMissing { reason },
        ResolveWithSourceOutcome::PromptUser => ResolveOutcome::PromptUser,
    }
}

#[derive(Debug)]
pub enum ResolveWithSourceOutcome {
    Found(ResolvedSecret),
    HardMissing { reason: String },
    PromptUser,
}

/// Canonical-name resolver used by PGAP, `plexi run`, PTY env injection, and
/// host integrations. An explicit route remains an alias override; without a
/// route, the canonical env var name is the workspace Keychain suffix.
pub fn resolve_with_source(
    workspace_id: &str,
    app_id: &str,
    canonical_name: &str,
    router: &WorkspaceSecrets,
    store: &dyn NonDestructiveStore,
) -> ResolveWithSourceOutcome {
    // Step 1+2: workspace route (apps.<id> first, then [default]).
    if let Some(friendly) = router.route_for(app_id, canonical_name) {
        let account = keychain_workspace_name(workspace_id, friendly);
        if let Some(value) = store.get(&account) {
            return ResolveWithSourceOutcome::Found(ResolvedSecret {
                value,
                source: ResolvedSecretSource::WorkspaceRoute,
            });
        }
        // The route value is a persisted friendly name that reconcile cannot
        // rewrite (secrets.toml lives per-workspace; reconcile is
        // workspace-blind). If it is a legacy spelling whose keychain account
        // was renamed to canonical, honor the route through the same alias
        // table that renamed it — loudly, until the file is fixed.
        if let Some(canonical) = canonical_friendly(friendly) {
            let renamed = keychain_workspace_name(workspace_id, canonical);
            if let Some(value) = store.get(&renamed) {
                log::warn!(
                    "workspace_secrets: route for '{canonical_name}' (app '{app_id}') points at \
                     legacy spelling '{friendly}' but the keychain account is now '{renamed}' — \
                     resolving anyway; update the route value in .plexi/secrets.toml to '{canonical}'"
                );
                return ResolveWithSourceOutcome::Found(ResolvedSecret {
                    value,
                    source: ResolvedSecretSource::WorkspaceRoute,
                });
            }
        }
        // Route declared but Keychain is empty — prompt the user (don't
        // silently fall through to user-scope; the route was explicit).
        return ResolveWithSourceOutcome::PromptUser;
    }

    // Step 2.5: no alias route means the canonical name is the workspace key.
    let workspace_account = keychain_workspace_name(workspace_id, canonical_name);
    if let Some(value) = store.get(&workspace_account) {
        return ResolveWithSourceOutcome::Found(ResolvedSecret {
            value,
            source: ResolvedSecretSource::WorkspaceCanonical,
        });
    }

    // Step 3: user-scope fallback when allowed.
    if router.fallback {
        let user_account = keychain_user_name(canonical_name);
        if let Some(value) = store.get(&user_account) {
            return ResolveWithSourceOutcome::Found(ResolvedSecret {
                value,
                source: ResolvedSecretSource::GlobalCanonical,
            });
        }
        return ResolveWithSourceOutcome::PromptUser;
    }

    // Step 4: no route + fallback disabled → hard error.
    ResolveWithSourceOutcome::HardMissing {
        reason: format!(
            "no workspace or route value in .plexi/secrets.toml for app '{app_id}' / secret \
             '{canonical_name}', and fallback = false"
        ),
    }
}

pub fn resolve_terminal_env(
    workspace_root: &Path,
    store: &dyn NonDestructiveStore,
) -> Result<HashMap<String, Zeroizing<String>>, String> {
    let cfg = WorkspaceConfig::load(workspace_root)?
        .ok_or_else(|| format!("workspace.toml missing at {}", workspace_root.display()))?;
    let router = WorkspaceSecrets::load(workspace_root)?
        .ok_or_else(|| format!("secrets.toml missing at {}", workspace_root.display()))?;

    let mut env = HashMap::new();
    for canonical_name in &router.terminal.env.inject {
        match resolve_with_source(&cfg.id, "terminal", canonical_name, &router, store) {
            ResolveWithSourceOutcome::Found(found) => {
                log::info!(
                    "workspace_secrets: terminal env injecting {canonical_name} source={:?}",
                    found.source
                );
                env.insert(canonical_name.clone(), found.value);
            }
            ResolveWithSourceOutcome::PromptUser => {
                log::info!(
                    "workspace_secrets: terminal env skipped missing allowlisted secret {canonical_name}"
                );
            }
            ResolveWithSourceOutcome::HardMissing { reason } => {
                log::warn!("workspace_secrets: terminal env skipped {canonical_name}: {reason}");
            }
        }
    }
    Ok(env)
}

// ── Workspace init helpers ───────────────────────────────────────────────────

/// `plexi workspace init` scaffolds all workspace files under `channel_dir`
/// (e.g. `.plexi-alpha/` or `.plexi/` for main):
///   - `<root>/<channel_dir>/workspace.toml` with a fresh UUID (idempotent)
///   - `<root>/<channel_dir>/secrets.toml` with `fallback = true`
///   - `<root>/<channel_dir>/.gitignore` so secrets never end up in git
///
/// `channel_dir` is the dot-prefixed workspace channel directory name
/// (e.g. `.plexi-alpha`, `.plexi`, `.plexi-pr-N`).
///
/// Returns the resolved `WorkspaceConfig` so the caller can echo the UUID.
pub fn init_workspace(workspace_root: &Path, channel_dir: &str) -> Result<WorkspaceConfig, String> {
    // Write workspace.toml under the channel dir
    let ws_path = workspace_root.join(channel_dir).join("workspace.toml");
    let cfg = if ws_path.exists() {
        let raw = std::fs::read_to_string(&ws_path)
            .map_err(|e| format!("read {}: {e}", ws_path.display()))?;
        toml::from_str::<WorkspaceConfig>(&raw)
            .map_err(|e| format!("parse {}: {e}", ws_path.display()))?
    } else {
        // Create the channel dir and write a fresh workspace.toml
        let dir = workspace_root.join(channel_dir);
        std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
        let id = uuid::Uuid::new_v4().to_string();
        std::fs::write(&ws_path, format!("id = \"{id}\"\n"))
            .map_err(|e| format!("write {}: {e}", ws_path.display()))?;
        WorkspaceConfig { id, context: None }
    };

    // secrets.toml under the channel dir
    let secrets_path = workspace_root.join(channel_dir).join("secrets.toml");
    if !secrets_path.exists() {
        let template = "# Workspace secret routing — see issue #322.\n\
                        # fallback: when no [apps.<id>] / [default] route matches a canonical\n\
                        # secret, true allows reading plexi:user:<name>; false errors loudly.\n\
                        fallback = true\n\
                        \n\
                        # [apps.<app-id>]\n\
                        # OPENAI_API_KEY = \"openai_personal\"\n\
                        \n\
                        # [default]\n\
                        # GITHUB_TOKEN = \"github_personal\"\n\
                        \n\
                        # [terminal.env]\n\
                        # inject = [\"OPENAI_API_KEY\"]\n";
        std::fs::write(&secrets_path, template)
            .map_err(|e| format!("write {}: {e}", secrets_path.display()))?;
    }

    // stub apps.toml under the channel dir
    let apps_toml = workspace_root.join(channel_dir).join("apps.toml");
    if !apps_toml.exists() {
        let stub = concat!(
            "schema_version = 1\n\n",
            "# Declare workspace app dependencies here.\n",
            "# Run `plexi app install` in this directory to install them.\n",
            "#\n",
            "# Example:\n",
            "#\n",
            "# [[app]]\n",
            "# id      = \"gh-issues\"\n",
            "# source  = \"local:gh-issues\"\n",
            "# version = \"bundled\"\n",
            "#\n",
            "# [[app]]\n",
            "# id      = \"my-tool\"\n",
            "# source  = \"github:org/my-tool\"\n",
            "# version = \"v1.0.0\"\n",
        );
        std::fs::write(&apps_toml, stub)
            .map_err(|e| format!("write {}: {e}", apps_toml.display()))?;
    }

    // stub commands.toml under the channel dir
    let commands_toml = workspace_root.join(channel_dir).join("commands.toml");
    if !commands_toml.exists() {
        let stub = concat!(
            "# Workspace commands — run with: plexi run <name>\n",
            "#\n",
            "# Simple form:   build = \"cargo build\"\n",
            "# With metadata: dev = { run = \"npm run dev\", description = \"Start dev server\" }\n",
            "# With secrets:  deploy = { run = \"./deploy.sh\", secrets = [\"API_KEY\"] }\n",
            "\n",
            "[commands]\n",
            "guess = \"$PLEXI_CONFIG_DIR/scripts/guess\"\n",
        );
        std::fs::write(&commands_toml, stub)
            .map_err(|e| format!("write {}: {e}", commands_toml.display()))?;
    }

    write_gitignore_if_absent(workspace_root)?;
    Ok(cfg)
}

/// Ensure channel-neutral app state cannot be committed with a context root.
///
/// App state is personal, single-user, local data — never committed, never
/// shared. Existing user rules are preserved byte-for-byte and the required
/// entry is appended only when absent.
pub(crate) fn ensure_app_state_gitignore(workspace_root: &Path) -> Result<(), String> {
    use std::io::Write;

    let dir = workspace_root.join(".plexi");
    std::fs::create_dir_all(&dir).map_err(|error| format!("create {}: {error}", dir.display()))?;
    let path = dir.join(".gitignore");
    let contents = match std::fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(format!("read {}: {error}", path.display())),
    };
    if contents.lines().any(|line| line.trim() == "app_states/") {
        return Ok(());
    }
    let prefix = if contents.is_empty() || contents.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|error| format!("open {} for append: {error}", path.display()))?;
    file.write_all(format!("{prefix}app_states/\n").as_bytes())
        .map_err(|error| format!("append {}: {error}", path.display()))?;
    file.sync_all()
        .map_err(|error| format!("sync {}: {error}", path.display()))
}


/// Default contents for `<root>/.plexi/.gitignore`. Anything that holds a
/// secret value or is generated host state lives here.
const GITIGNORE_TEMPLATE: &str = "# Auto-generated by plexi workspace init.\n\
                                  # Edit this file freely — re-running init never overwrites it.\n\
                                  secrets.toml\n\
                                  cache/\n\
                                  agents/*/memory/\n\
                                  agents/*/logs/\n";

/// Write `<root>/.plexi/.gitignore` only when the file does not already exist.
/// User edits to an existing file are preserved verbatim.
fn write_gitignore_if_absent(workspace_root: &Path) -> Result<(), String> {
    let dir = workspace_root.join(crate::config::workspace_channel_dir());
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let path = dir.join(".gitignore");
    if path.exists() {
        return Ok(());
    }
    std::fs::write(&path, GITIGNORE_TEMPLATE)
        .map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(())
}

// ── Route auto-write ─────────────────────────────────────────────────────────

/// After a `plexi secret set` Keychain write, record the canonical→friendly
/// route in `<workspace_root>/.plexi/secrets.toml` under `[default]`.
///
/// - File absent: creates it with `fallback = true` + the route.
/// - Canonical already maps to the same friendly: no-op (idempotent).
/// - Canonical maps to a different friendly: updates the existing entry in-place.
/// - Canonical not present: injects a new entry, preserving existing content.
pub fn write_default_route(
    workspace_root: &Path,
    canonical: &str,
    friendly: &str,
) -> Result<(), String> {
    let secrets_path = workspace_root
        .join(crate::config::workspace_channel_dir())
        .join("secrets.toml");

    if !secrets_path.exists() {
        let dir = workspace_root.join(crate::config::workspace_channel_dir());
        std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
        let content = format!("fallback = true\n\n[default]\n{canonical} = \"{friendly}\"\n");
        return std::fs::write(&secrets_path, content)
            .map_err(|e| format!("write {}: {e}", secrets_path.display()));
    }

    let raw = std::fs::read_to_string(&secrets_path)
        .map_err(|e| format!("read {}: {e}", secrets_path.display()))?;

    // Idempotency: bail out only when the mapping already points at the same friendly name.
    if let Ok(Some(router)) = WorkspaceSecrets::load(workspace_root) {
        if router.default.get(canonical).map(|s| s.as_str()) == Some(friendly) {
            return Ok(());
        }
    }

    // Insert or update the canonical→friendly mapping.
    let updated = upsert_default_route_line(&raw, canonical, friendly);
    std::fs::write(&secrets_path, updated)
        .map_err(|e| format!("write {}: {e}", secrets_path.display()))
}

/// Update `[terminal.env] inject = [...]` in workspace `secrets.toml`.
///
/// Workspace-scoped secrets default to terminal injection on when created by
/// the native Secrets app or CLI. This helper is also used by the native app
/// toggle so the TOML file remains the durable source of policy.
pub fn write_terminal_env_inject(
    workspace_root: &Path,
    canonical: &str,
    enabled: bool,
) -> Result<(), String> {
    let secrets_path = workspace_root
        .join(crate::config::workspace_channel_dir())
        .join("secrets.toml");

    if !secrets_path.exists() {
        let dir = workspace_root.join(crate::config::workspace_channel_dir());
        std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
        let inject = if enabled {
            format!("  \"{canonical}\",\n")
        } else {
            String::new()
        };
        let content = format!("fallback = true\n\n[terminal.env]\ninject = [\n{inject}]\n");
        return std::fs::write(&secrets_path, content)
            .map_err(|e| format!("write {}: {e}", secrets_path.display()));
    }

    let raw = std::fs::read_to_string(&secrets_path)
        .map_err(|e| format!("read {}: {e}", secrets_path.display()))?;

    let mut names = WorkspaceSecrets::parse(&raw)
        .map(|router| router.terminal.env.inject)
        .unwrap_or_default();
    let already_present = names.iter().any(|name| name == canonical);
    if enabled && !already_present {
        names.push(canonical.to_string());
    } else if !enabled && already_present {
        names.retain(|name| name != canonical);
    } else {
        return Ok(());
    }

    let updated = upsert_terminal_env_inject_section(&raw, &names);
    std::fs::write(&secrets_path, updated)
        .map_err(|e| format!("write {}: {e}", secrets_path.display()))
}

/// Insert or update `canonical = "friendly"` in the `[default]` section of a
/// raw `secrets.toml` string, creating the section if absent.
/// If an existing `canonical = "..."` line is found, it is replaced in-place.
/// All other content and comments are preserved.
fn upsert_default_route_line(raw: &str, canonical: &str, friendly: &str) -> String {
    let entry_line = format!("{canonical} = \"{friendly}\"");
    let lines: Vec<&str> = raw.lines().collect();
    let trailing_newline = raw.ends_with('\n');

    if let Some(start) = lines.iter().position(|l| {
        let t = l.trim();
        t == "[default]" || (t.starts_with("[default]") && t[9..].trim_start().starts_with('#'))
    }) {
        // End of section: next uncommented table header, or EOF.
        let end = lines[start + 1..]
            .iter()
            .position(|l| {
                let t = l.trim();
                t.starts_with('[') && !t.starts_with('#')
            })
            .map(|p| start + 1 + p)
            .unwrap_or(lines.len());

        // Check for an existing entry for this canonical key and replace it if found.
        let canonical_prefix = format!("{canonical} = ");
        let existing = lines[start + 1..end]
            .iter()
            .position(|l| l.trim().starts_with(&canonical_prefix))
            .map(|p| start + 1 + p);

        let result = if let Some(idx) = existing {
            let mut parts: Vec<&str> = Vec::with_capacity(lines.len());
            parts.extend_from_slice(&lines[..idx]);
            parts.push(&entry_line);
            parts.extend_from_slice(&lines[idx + 1..]);
            parts
        } else {
            // No existing entry — append inside section before next section header.
            let mut parts: Vec<&str> = Vec::with_capacity(lines.len() + 1);
            parts.extend_from_slice(&lines[..end]);
            parts.push(&entry_line);
            parts.extend_from_slice(&lines[end..]);
            parts
        };

        let joined = result.join("\n");
        if trailing_newline {
            format!("{joined}\n")
        } else {
            joined
        }
    } else {
        // No [default] section — append one.
        let base = raw.trim_end_matches('\n');
        format!("{base}\n\n[default]\n{entry_line}\n")
    }
}

fn upsert_terminal_env_inject_section(raw: &str, names: &[String]) -> String {
    let mut names = names.to_vec();
    names.sort();
    names.dedup();

    let section = render_terminal_env_inject_section(&names);
    let lines: Vec<&str> = raw.lines().collect();
    let trailing_newline = raw.ends_with('\n');

    if let Some(start) = lines.iter().position(|line| {
        let trimmed = line.trim();
        trimmed == "[terminal.env]"
            || (trimmed.starts_with("[terminal.env]")
                && trimmed[14..].trim_start().starts_with('#'))
    }) {
        let end = lines[start + 1..]
            .iter()
            .position(|line| {
                let trimmed = line.trim();
                trimmed.starts_with('[') && !trimmed.starts_with('#')
            })
            .map(|pos| start + 1 + pos)
            .unwrap_or(lines.len());

        let mut parts: Vec<&str> = Vec::with_capacity(lines.len() + section.lines().count());
        parts.extend_from_slice(&lines[..start]);
        parts.extend(section.trim_end_matches('\n').lines());
        parts.extend_from_slice(&lines[end..]);
        let joined = parts.join("\n");
        if trailing_newline {
            format!("{joined}\n")
        } else {
            joined
        }
    } else {
        let base = raw.trim_end_matches('\n');
        if base.is_empty() {
            section
        } else {
            format!("{base}\n\n{section}")
        }
    }
}

fn render_terminal_env_inject_section(names: &[String]) -> String {
    let mut out = String::from("[terminal.env]\ninject = [\n");
    for name in names {
        out.push_str("  \"");
        out.push_str(name);
        out.push_str("\",\n");
    }
    out.push_str("]\n");
    out
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn router(toml_src: &str) -> WorkspaceSecrets {
        WorkspaceSecrets::parse(toml_src).expect("router parses")
    }

    fn write_terminal_env_workspace(root: &Path, workspace_id: &str) {
        let channel_dir = crate::config::workspace_channel_dir();
        std::fs::create_dir_all(root.join(&channel_dir)).unwrap();
        std::fs::write(
            root.join(&channel_dir).join("workspace.toml"),
            format!("id = \"{workspace_id}\"\n"),
        )
        .unwrap();
        std::fs::write(
            root.join(&channel_dir).join("secrets.toml"),
            "fallback = true\n\n[terminal.env]\ninject = [\"OPENROUTER_API_KEY\"]\n",
        )
        .unwrap();
    }

    // ── reconcile ─────────────────────────────────────────────────────────────

    fn accounts(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn reconcile_adopts_keychain_accounts_missing_from_index() {
        let store = InMemoryKeychain::new();
        let scanned = accounts(&["plexi:user:OPENAI_API_KEY", "plexi:ws-1:GITHUB_TOKEN"]);
        let index = accounts(&["plexi:user:OPENAI_API_KEY"]);

        let report = reconcile(&scanned, &index, &store);

        assert_eq!(report.adopted, accounts(&["plexi:ws-1:GITHUB_TOKEN"]));
        assert!(report.stale.is_empty(), "{:?}", report.stale);
        assert_eq!(
            report.index,
            accounts(&["plexi:user:OPENAI_API_KEY", "plexi:ws-1:GITHUB_TOKEN"])
        );
    }

    #[test]
    fn reconcile_drops_index_entries_with_no_keychain_item() {
        let store = InMemoryKeychain::new();
        let scanned = accounts(&["plexi:user:OPENAI_API_KEY"]);
        let index = accounts(&["plexi:user:OPENAI_API_KEY", "plexi:user:DELETED_BY_HAND"]);

        let report = reconcile(&scanned, &index, &store);

        assert_eq!(report.stale, accounts(&["plexi:user:DELETED_BY_HAND"]));
        assert!(report.adopted.is_empty(), "{:?}", report.adopted);
        assert_eq!(report.index, accounts(&["plexi:user:OPENAI_API_KEY"]));
    }

    #[test]
    fn reconcile_is_noop_when_index_matches_keychain() {
        let store = InMemoryKeychain::new();
        let scanned = accounts(&["plexi:user:OPENAI_API_KEY"]);

        let report = reconcile(&scanned, &scanned, &store);

        assert!(report.is_noop(), "{report:?}");
        assert_eq!(report.index, scanned);
    }

    #[test]
    fn reconcile_never_adopts_non_namespaced_legacy_accounts() {
        let store = InMemoryKeychain::new();
        // Pre-#322 accounts still live under the same keychain service.
        let scanned = accounts(&["plexi-run//Users/me/project/TEST_KEY", "plexi:user:AGE"]);

        let report = reconcile(&scanned, &[], &store);

        assert_eq!(
            report.ignored,
            accounts(&["plexi-run//Users/me/project/TEST_KEY"])
        );
        assert_eq!(report.index, accounts(&["plexi:user:AGE"]));
    }

    #[test]
    fn reconcile_normalizes_legacy_openrouter_spelling() {
        let store = InMemoryKeychain::new();
        store
            .set("plexi:user:openrouter-api-key", "sk-legacy")
            .unwrap();
        let scanned = accounts(&["plexi:user:openrouter-api-key"]);

        let report = reconcile(&scanned, &[], &store);

        assert_eq!(
            report.renamed,
            vec![AccountRename {
                from: "plexi:user:openrouter-api-key".to_string(),
                to: "plexi:user:OPENROUTER_API_KEY".to_string(),
            }]
        );
        assert_eq!(report.index, accounts(&["plexi:user:OPENROUTER_API_KEY"]));
        // Value moved, legacy account gone.
        assert_eq!(
            store
                .get("plexi:user:OPENROUTER_API_KEY")
                .map(|v| v.to_string()),
            Some("sk-legacy".to_string())
        );
        assert!(store.get("plexi:user:openrouter-api-key").is_none());
        // A rename is neither an adoption nor a stale entry.
        assert!(report.adopted.is_empty(), "{:?}", report.adopted);
        assert!(report.stale.is_empty(), "{:?}", report.stale);
    }

    #[test]
    fn reconcile_leaves_both_spellings_alone_when_canonical_already_exists() {
        let store = InMemoryKeychain::new();
        store
            .set("plexi:user:openrouter-api-key", "sk-legacy")
            .unwrap();
        store
            .set("plexi:user:OPENROUTER_API_KEY", "sk-canonical")
            .unwrap();
        let scanned = accounts(&[
            "plexi:user:openrouter-api-key",
            "plexi:user:OPENROUTER_API_KEY",
        ]);

        let report = reconcile(&scanned, &[], &store);

        assert_eq!(
            report.conflicts,
            vec![AccountRename {
                from: "plexi:user:openrouter-api-key".to_string(),
                to: "plexi:user:OPENROUTER_API_KEY".to_string(),
            }]
        );
        assert!(report.renamed.is_empty(), "{:?}", report.renamed);
        // Neither value was overwritten, and both stay listed.
        assert_eq!(
            store
                .get("plexi:user:OPENROUTER_API_KEY")
                .map(|v| v.to_string()),
            Some("sk-canonical".to_string())
        );
        assert_eq!(
            store
                .get("plexi:user:openrouter-api-key")
                .map(|v| v.to_string()),
            Some("sk-legacy".to_string())
        );
        assert_eq!(
            report.index,
            accounts(&[
                "plexi:user:OPENROUTER_API_KEY",
                "plexi:user:openrouter-api-key",
            ])
        );
    }

    #[test]
    fn add_new_refuses_an_existing_account_and_leaves_its_value_alone() {
        let store = InMemoryKeychain::new();
        store.add_new("plexi:user:AGE", "first").expect("first add");

        let err = store
            .add_new("plexi:user:AGE", "second")
            .expect_err("a second add of the same account must fail");

        assert!(
            matches!(err, SecretError::AlreadyExists(ref a) if a == "plexi:user:AGE"),
            "expected AlreadyExists, got {err:?}"
        );
        assert_eq!(
            store.get("plexi:user:AGE").map(|v| v.to_string()),
            Some("first".to_string()),
            "a refused add must not change the stored value"
        );
    }

    #[test]
    fn reconcile_never_overwrites_a_canonical_account_created_after_the_scan() {
        // The scan is a snapshot. If the canonical account is created between
        // the scan and the migration, an upsert would silently destroy it and
        // then delete the legacy item too — losing two values at once.
        let store = InMemoryKeychain::new();
        store
            .set("plexi:user:openrouter-api-key", "sk-legacy")
            .unwrap();
        store
            .set("plexi:user:OPENROUTER_API_KEY", "NEW-VALUE")
            .unwrap();
        // Snapshot taken before the canonical account existed.
        let scanned = accounts(&["plexi:user:openrouter-api-key"]);

        let report = reconcile(&scanned, &[], &store);

        assert_eq!(
            store
                .get("plexi:user:OPENROUTER_API_KEY")
                .map(|v| v.to_string()),
            Some("NEW-VALUE".to_string()),
            "an existing canonical value must never be overwritten"
        );
        assert_eq!(
            store
                .get("plexi:user:openrouter-api-key")
                .map(|v| v.to_string()),
            Some("sk-legacy".to_string()),
            "the legacy secret must survive a migration that could not complete"
        );
        assert!(report.renamed.is_empty(), "{:?}", report.renamed);
        assert_eq!(
            report.conflicts,
            vec![AccountRename {
                from: "plexi:user:openrouter-api-key".to_string(),
                to: "plexi:user:OPENROUTER_API_KEY".to_string(),
            }]
        );
    }

    #[test]
    fn reconcile_does_not_delete_the_legacy_account_when_read_back_fails() {
        // The write reported success but the canonical account does not read
        // back the value. Deleting the legacy item here would destroy the only
        // surviving copy of the secret.
        let store = InMemoryKeychain::with_unreadable_account("plexi:user:OPENROUTER_API_KEY");
        store
            .set("plexi:user:openrouter-api-key", "sk-legacy")
            .unwrap();
        let scanned = accounts(&["plexi:user:openrouter-api-key"]);

        let report = reconcile(&scanned, &[], &store);

        assert_eq!(
            store
                .get("plexi:user:openrouter-api-key")
                .map(|v| v.to_string()),
            Some("sk-legacy".to_string()),
            "legacy must be retained until the canonical copy is verified"
        );
        assert!(report.renamed.is_empty(), "{:?}", report.renamed);
        assert_eq!(
            report.conflicts,
            vec![AccountRename {
                from: "plexi:user:openrouter-api-key".to_string(),
                to: "plexi:user:OPENROUTER_API_KEY".to_string(),
            }]
        );
    }

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

    #[test]
    fn delete_if_value_refuses_when_the_stored_value_changed() {
        let store = InMemoryKeychain::new();
        store.set("plexi:user:X", "current").unwrap();

        let refused = store.delete_if_value("plexi:user:X", "what-i-read-earlier");
        assert!(
            matches!(refused, Err(SecretError::ValueChanged(_))),
            "{refused:?}"
        );
        assert_eq!(
            store.get("plexi:user:X").map(|v| v.to_string()),
            Some("current".to_string()),
            "a refused guarded delete must leave the value untouched"
        );

        store.delete_if_value("plexi:user:X", "current").unwrap();
        assert!(
            store.get("plexi:user:X").is_none(),
            "matching value deletes"
        );
        // Already gone — success, nothing to lose.
        store.delete_if_value("plexi:user:X", "anything").unwrap();
    }

    #[test]
    fn reconcile_keeps_a_legacy_value_written_after_it_was_copied() {
        // A concurrent writer updates the legacy account between reconcile's
        // read and its delete. An unconditional delete would destroy that
        // update; the value-guarded delete refuses and keeps both accounts.
        let store =
            InMemoryKeychain::with_stale_read("plexi:user:openrouter-api-key", "sk-as-read");
        store
            .set("plexi:user:openrouter-api-key", "sk-updated-concurrently")
            .unwrap();
        let scanned = accounts(&["plexi:user:openrouter-api-key"]);

        let report = reconcile(&scanned, &[], &store);

        assert_eq!(
            store
                .get("plexi:user:openrouter-api-key")
                .map(|v| v.to_string()),
            Some("sk-updated-concurrently".to_string()),
            "a legacy value written mid-pass must never be deleted"
        );
        assert_eq!(
            store
                .get("plexi:user:OPENROUTER_API_KEY")
                .map(|v| v.to_string()),
            Some("sk-as-read".to_string()),
            "the copied value stays under the canonical account"
        );
        assert!(report.renamed.is_empty(), "{:?}", report.renamed);
        assert_eq!(
            report.conflicts,
            vec![AccountRename {
                from: "plexi:user:openrouter-api-key".to_string(),
                to: "plexi:user:OPENROUTER_API_KEY".to_string(),
            }]
        );
    }

    #[test]
    fn routed_workspace_still_resolves_after_reconcile_renames_the_account() {
        // `secret set --alias openrouter-api-key` writes a route whose value
        // is the legacy friendly name. Reconcile renames the keychain account
        // to canonical but cannot rewrite per-workspace secrets.toml — the
        // resolver must honor the routed legacy spelling through the alias
        // table, or the secret silently vanishes from PTY env injection,
        // PGAP, and `plexi run` (found live by tester-6 on PR 2503).
        let store = InMemoryKeychain::new();
        store
            .set("plexi:ws-1:openrouter-api-key", "sk-routed")
            .unwrap();
        let scanned = accounts(&["plexi:ws-1:openrouter-api-key"]);
        let report = reconcile(&scanned, &[], &store);
        assert_eq!(report.renamed.len(), 1, "{report:?}");
        assert!(
            store.get("plexi:ws-1:openrouter-api-key").is_none(),
            "precondition: the legacy account was renamed"
        );

        let router =
            router("fallback = false\n\n[default]\nOPENROUTER_API_KEY = \"openrouter-api-key\"\n");
        let outcome =
            resolve_with_source("ws-1", "terminal", "OPENROUTER_API_KEY", &router, &store);
        match outcome {
            ResolveWithSourceOutcome::Found(found) => {
                assert_eq!(found.value.to_string(), "sk-routed");
                assert!(matches!(found.source, ResolvedSecretSource::WorkspaceRoute));
            }
            other => panic!("routed secret must survive the rename, got {other:?}"),
        }
    }

    #[test]
    fn reconcile_reports_a_conflict_when_the_legacy_account_cannot_be_deleted() {
        // Copy succeeds, delete is refused: both spellings are live, so the
        // legacy one must stay listed instead of being hidden by a rename that
        // only half happened.
        let store = InMemoryKeychain::with_failing_delete();
        store
            .set("plexi:user:openrouter-api-key", "sk-legacy")
            .unwrap();
        let scanned = accounts(&["plexi:user:openrouter-api-key"]);

        let report = reconcile(&scanned, &[], &store);

        assert!(report.renamed.is_empty(), "{:?}", report.renamed);
        assert_eq!(
            report.conflicts,
            vec![AccountRename {
                from: "plexi:user:openrouter-api-key".to_string(),
                to: "plexi:user:OPENROUTER_API_KEY".to_string(),
            }]
        );
        assert_eq!(
            report.index,
            accounts(&[
                "plexi:user:OPENROUTER_API_KEY",
                "plexi:user:openrouter-api-key",
            ])
        );
    }

    #[test]
    fn reconcile_keeps_legacy_account_visible_when_its_value_is_unreadable() {
        // Keychain ACL denial: the value cannot be moved, but the secret must
        // still show up in listings rather than staying invisible.
        let store = InMemoryKeychain::new();
        let scanned = accounts(&["plexi:user:openrouter-api-key"]);

        let report = reconcile(&scanned, &[], &store);

        assert!(report.renamed.is_empty(), "{:?}", report.renamed);
        assert_eq!(report.index, scanned);
        assert_eq!(report.adopted, scanned);
    }

    #[test]
    fn canonical_account_maps_only_known_aliases() {
        assert_eq!(
            canonical_account("plexi:user:openrouter-api-key").as_deref(),
            Some("plexi:user:OPENROUTER_API_KEY")
        );
        assert_eq!(
            canonical_account("plexi:ws-1:openrouter-api-key").as_deref(),
            Some("plexi:ws-1:OPENROUTER_API_KEY")
        );
        assert!(canonical_account("plexi:user:OPENROUTER_API_KEY").is_none());
        assert!(canonical_account("plexi:user:GITHUB_TOKEN").is_none());
        assert!(canonical_account("plexi-run//Users/me/TEST_KEY").is_none());
    }

    #[test]
    fn keychain_naming_uses_workspace_and_user_namespaces() {
        assert_eq!(
            keychain_workspace_name("abc-123", "openai_prod"),
            "plexi:abc-123:openai_prod"
        );
        assert_eq!(
            keychain_user_name("github_token"),
            "plexi:user:github_token"
        );
    }

    #[test]
    fn parse_rejects_missing_fallback() {
        let err = WorkspaceSecrets::parse("[apps.foo]\nX = \"y\"\n")
            .expect_err("missing fallback should error");
        assert!(
            err.contains("fallback") || err.contains("missing field"),
            "expected fallback-related error, got: {err}"
        );
    }

    #[test]
    fn layer_1_app_route_returns_workspace_namespaced_value() {
        let store = InMemoryKeychain::new();
        store.set("plexi:ws-1:openai_prod", "sk-abc").unwrap();
        let r = router("fallback = false\n[apps.claude-code]\nOPENAI_API_KEY = \"openai_prod\"\n");
        match resolve("ws-1", "claude-code", "OPENAI_API_KEY", &r, &store) {
            ResolveOutcome::Found(v) => assert_eq!(v.as_str(), "sk-abc"),
            other => panic!("expected Found, got {other:?}"),
        }
    }

    #[test]
    fn layer_2_default_route_used_when_no_app_route() {
        let store = InMemoryKeychain::new();
        store.set("plexi:ws-1:gh_team", "ghp-team").unwrap();
        let r = router("fallback = false\n[default]\nGITHUB_TOKEN = \"gh_team\"\n");
        match resolve("ws-1", "any-app", "GITHUB_TOKEN", &r, &store) {
            ResolveOutcome::Found(v) => assert_eq!(v.as_str(), "ghp-team"),
            other => panic!("expected Found, got {other:?}"),
        }
    }

    #[test]
    fn fallback_false_with_no_route_is_hard_missing() {
        let store = InMemoryKeychain::new();
        // Even with a user-scope value present, fallback=false must NOT use it.
        store.set("plexi:user:OPENAI_API_KEY", "sk-user").unwrap();
        let r = router("fallback = false\n");
        match resolve("ws-1", "claude-code", "OPENAI_API_KEY", &r, &store) {
            ResolveOutcome::HardMissing { reason } => {
                assert!(reason.contains("fallback = false"), "reason: {reason}");
            }
            other => panic!("expected HardMissing, got {other:?}"),
        }
    }

    #[test]
    fn fallback_true_reads_user_scope_when_no_route() {
        let store = InMemoryKeychain::new();
        store.set("plexi:user:GITHUB_TOKEN", "ghp-user").unwrap();
        let r = router("fallback = true\n");
        match resolve("ws-1", "claude-code", "GITHUB_TOKEN", &r, &store) {
            ResolveOutcome::Found(v) => assert_eq!(v.as_str(), "ghp-user"),
            other => panic!("expected Found, got {other:?}"),
        }
    }

    #[test]
    fn canonical_workspace_name_resolves_without_alias_route() {
        let store = InMemoryKeychain::new();
        store
            .set("plexi:ws-1:OPENROUTER_API_KEY", "sk-workspace")
            .unwrap();
        let r = router("fallback = true\n");
        match resolve("ws-1", "terminal", "OPENROUTER_API_KEY", &r, &store) {
            ResolveOutcome::Found(v) => assert_eq!(v.as_str(), "sk-workspace"),
            other => panic!("expected Found, got {other:?}"),
        }
    }

    #[test]
    fn workspace_canonical_value_overrides_global_fallback() {
        let store = InMemoryKeychain::new();
        store
            .set("plexi:ws-1:OPENAI_API_KEY", "sk-workspace")
            .unwrap();
        store.set("plexi:user:OPENAI_API_KEY", "sk-global").unwrap();
        let r = router("fallback = true\n");
        match resolve("ws-1", "terminal", "OPENAI_API_KEY", &r, &store) {
            ResolveOutcome::Found(v) => assert_eq!(v.as_str(), "sk-workspace"),
            other => panic!("expected Found, got {other:?}"),
        }
    }

    #[test]
    fn explicit_alias_route_takes_precedence_over_canonical_workspace_name() {
        let store = InMemoryKeychain::new();
        store
            .set("plexi:ws-1:OPENAI_API_KEY", "sk-canonical")
            .unwrap();
        store.set("plexi:ws-1:openai_personal", "sk-alias").unwrap();
        let r = router("fallback = true\n[default]\nOPENAI_API_KEY = \"openai_personal\"\n");
        match resolve_with_source("ws-1", "terminal", "OPENAI_API_KEY", &r, &store) {
            ResolveWithSourceOutcome::Found(found) => {
                assert_eq!(found.value.as_str(), "sk-alias");
                assert_eq!(found.source, ResolvedSecretSource::WorkspaceRoute);
            }
            other => panic!("expected Found, got {other:?}"),
        }
    }

    #[test]
    fn terminal_env_injects_only_allowlisted_names() {
        let tmp = tempfile::tempdir().unwrap();
        let channel_dir = crate::config::workspace_channel_dir();
        std::fs::create_dir_all(tmp.path().join(&channel_dir)).unwrap();
        std::fs::write(
            tmp.path().join(&channel_dir).join("workspace.toml"),
            "id = \"ws-1\"\n",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join(&channel_dir).join("secrets.toml"),
            "fallback = true\n\n[terminal.env]\ninject = [\"OPENROUTER_API_KEY\"]\n",
        )
        .unwrap();
        let store = InMemoryKeychain::new();
        store
            .set("plexi:ws-1:OPENROUTER_API_KEY", "sk-openrouter")
            .unwrap();
        store.set("plexi:ws-1:OPENAI_API_KEY", "sk-openai").unwrap();

        let env = resolve_terminal_env(tmp.path(), &store).expect("terminal env resolves");

        assert_eq!(
            env.get("OPENROUTER_API_KEY").map(|v| v.as_str()),
            Some("sk-openrouter")
        );
        assert!(
            !env.contains_key("OPENAI_API_KEY"),
            "non-allowlisted secret must not be injected"
        );
    }

    #[test]
    fn terminal_env_uses_global_fallback_when_allowed() {
        let tmp = tempfile::tempdir().unwrap();
        let channel_dir = crate::config::workspace_channel_dir();
        std::fs::create_dir_all(tmp.path().join(&channel_dir)).unwrap();
        std::fs::write(
            tmp.path().join(&channel_dir).join("workspace.toml"),
            "id = \"ws-1\"\n",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join(&channel_dir).join("secrets.toml"),
            "fallback = true\n\n[terminal.env]\ninject = [\"OPENAI_API_KEY\"]\n",
        )
        .unwrap();
        let store = InMemoryKeychain::new();
        store.set("plexi:user:OPENAI_API_KEY", "sk-global").unwrap();

        let env = resolve_terminal_env(tmp.path(), &store).expect("terminal env resolves");

        assert_eq!(
            env.get("OPENAI_API_KEY").map(|v| v.as_str()),
            Some("sk-global")
        );
    }

    #[test]
    fn terminal_env_resolves_same_openrouter_name_per_workspace() {
        let ws_a = tempfile::tempdir().unwrap();
        let ws_b = tempfile::tempdir().unwrap();
        write_terminal_env_workspace(ws_a.path(), "ws-a");
        write_terminal_env_workspace(ws_b.path(), "ws-b");

        let store = InMemoryKeychain::new();
        store
            .set("plexi:ws-a:OPENROUTER_API_KEY", "sk-openrouter-a")
            .unwrap();
        store
            .set("plexi:ws-b:OPENROUTER_API_KEY", "sk-openrouter-b")
            .unwrap();

        let env_a = resolve_terminal_env(ws_a.path(), &store).expect("workspace A env");
        let env_b = resolve_terminal_env(ws_b.path(), &store).expect("workspace B env");

        assert_eq!(
            env_a.get("OPENROUTER_API_KEY").map(|v| v.as_str()),
            Some("sk-openrouter-a")
        );
        assert_eq!(
            env_b.get("OPENROUTER_API_KEY").map(|v| v.as_str()),
            Some("sk-openrouter-b")
        );
    }

    #[test]
    fn terminal_env_injects_nothing_by_default() {
        let tmp = tempfile::tempdir().unwrap();
        let channel_dir = crate::config::workspace_channel_dir();
        std::fs::create_dir_all(tmp.path().join(&channel_dir)).unwrap();
        std::fs::write(
            tmp.path().join(&channel_dir).join("workspace.toml"),
            "id = \"ws-1\"\n",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join(&channel_dir).join("secrets.toml"),
            "fallback = true\n",
        )
        .unwrap();
        let store = InMemoryKeychain::new();
        store
            .set("plexi:ws-1:OPENROUTER_API_KEY", "sk-workspace")
            .unwrap();
        store.set("plexi:user:OPENAI_API_KEY", "sk-global").unwrap();

        let env = resolve_terminal_env(tmp.path(), &store).expect("terminal env resolves");

        assert!(env.is_empty(), "terminal injection must be opt-in");
    }

    #[test]
    fn same_canonical_name_two_workspaces_returns_different_values() {
        // The whole point of workspace-scoping: same OPENAI_API_KEY in the
        // app, different bills downstream.
        let store = InMemoryKeychain::new();
        store.set("plexi:work:openai_prod", "sk-work").unwrap();
        store
            .set("plexi:personal:openai_personal", "sk-personal")
            .unwrap();
        let work_router =
            router("fallback = false\n[apps.claude-code]\nOPENAI_API_KEY = \"openai_prod\"\n");
        let personal_router =
            router("fallback = false\n[apps.claude-code]\nOPENAI_API_KEY = \"openai_personal\"\n");
        let work = match resolve(
            "work",
            "claude-code",
            "OPENAI_API_KEY",
            &work_router,
            &store,
        ) {
            ResolveOutcome::Found(v) => v.to_string(),
            other => panic!("work: {other:?}"),
        };
        let personal = match resolve(
            "personal",
            "claude-code",
            "OPENAI_API_KEY",
            &personal_router,
            &store,
        ) {
            ResolveOutcome::Found(v) => v.to_string(),
            other => panic!("personal: {other:?}"),
        };
        assert_eq!(work, "sk-work");
        assert_eq!(personal, "sk-personal");
        assert_ne!(work, personal);
    }

    #[test]
    fn route_declared_but_keychain_empty_prompts_user() {
        let store = InMemoryKeychain::new();
        // Router points at a friendly name but no Keychain entry was set.
        let r = router("fallback = true\n[apps.claude-code]\nOPENAI_API_KEY = \"openai_prod\"\n");
        match resolve("ws-1", "claude-code", "OPENAI_API_KEY", &r, &store) {
            ResolveOutcome::PromptUser => {}
            other => panic!("expected PromptUser, got {other:?}"),
        }
    }

    #[test]
    fn app_route_takes_precedence_over_default() {
        let store = InMemoryKeychain::new();
        store.set("plexi:ws-1:per_app", "per-app-val").unwrap();
        store.set("plexi:ws-1:default_val", "default-val").unwrap();
        let r = router(
            "fallback = false\n\
             [apps.claude-code]\n\
             OPENAI_API_KEY = \"per_app\"\n\
             [default]\n\
             OPENAI_API_KEY = \"default_val\"\n",
        );
        match resolve("ws-1", "claude-code", "OPENAI_API_KEY", &r, &store) {
            ResolveOutcome::Found(v) => assert_eq!(v.as_str(), "per-app-val"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn workspace_config_load_or_init_writes_uuid_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let channel_dir = crate::config::workspace_channel_dir();
        let cfg = init_workspace(tmp.path(), &channel_dir).expect("load_or_init");
        assert!(uuid::Uuid::parse_str(&cfg.id).is_ok());
        // Idempotent — second call returns the same id.
        let cfg2 = init_workspace(tmp.path(), &channel_dir).expect("second load");
        assert_eq!(cfg.id, cfg2.id);
    }

    #[test]
    fn init_workspace_creates_workspace_and_secrets_files() {
        let tmp = tempfile::tempdir().unwrap();
        let channel_dir = crate::config::workspace_channel_dir();
        init_workspace(tmp.path(), &channel_dir).expect("init_workspace");
        assert!(tmp
            .path()
            .join(&channel_dir)
            .join("workspace.toml")
            .is_file());
        let secrets_raw =
            std::fs::read_to_string(tmp.path().join(&channel_dir).join("secrets.toml")).unwrap();
        // Generated secrets.toml must parse cleanly with the required field.
        let parsed = WorkspaceSecrets::parse(&secrets_raw).expect("template parses");
        assert!(parsed.fallback);
    }

    #[test]
    fn init_writes_gitignore_when_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let channel_dir = crate::config::workspace_channel_dir();
        let gitignore = tmp.path().join(&channel_dir).join(".gitignore");
        assert!(!gitignore.exists());

        init_workspace(tmp.path(), &channel_dir).expect("init_workspace");

        assert!(
            gitignore.is_file(),
            "init must create {channel_dir}/.gitignore"
        );
        let raw = std::fs::read_to_string(&gitignore).unwrap();
        assert!(raw.contains("secrets.toml"), "got: {raw}");
        assert!(raw.contains("cache/"), "got: {raw}");
    }

    #[test]
    fn init_preserves_existing_gitignore() {
        let tmp = tempfile::tempdir().unwrap();
        let channel_dir = crate::config::workspace_channel_dir();
        let dir = tmp.path().join(&channel_dir);
        std::fs::create_dir_all(&dir).unwrap();
        let gitignore = dir.join(".gitignore");
        let custom = "# my own rules\nfoo\nbar\n";
        std::fs::write(&gitignore, custom).unwrap();

        init_workspace(tmp.path(), &channel_dir).expect("init_workspace");

        let raw = std::fs::read_to_string(&gitignore).unwrap();
        assert_eq!(
            raw, custom,
            "init must NOT overwrite an existing .gitignore"
        );
    }

    #[test]
    fn app_state_gitignore_preserves_rules_and_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let neutral_dir = tmp.path().join(".plexi");
        std::fs::create_dir_all(&neutral_dir).unwrap();
        let gitignore = neutral_dir.join(".gitignore");
        std::fs::write(&gitignore, "# personal\ncustom/\n").unwrap();

        ensure_app_state_gitignore(tmp.path()).expect("first ensure");
        ensure_app_state_gitignore(tmp.path()).expect("second ensure");

        assert_eq!(
            std::fs::read_to_string(gitignore).unwrap(),
            "# personal\ncustom/\napp_states/\n"
        );
    }

    /// The standing ruling made effective: in a real git repository, a state
    /// file under `<root>/.plexi/app_states/` must be invisible to git after
    /// the ensure runs — a user cannot accidentally commit their app state.
    #[test]
    fn app_state_gitignore_is_effective_in_a_real_repo() {
        let repo = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(repo.path())
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_SYSTEM", "/dev/null")
                .output()
                .expect("run git")
        };
        assert!(git(&["init", "-q"]).status.success(), "git init");

        ensure_app_state_gitignore(repo.path()).expect("ensure gitignore");
        let state_dir = repo.path().join(".plexi").join("app_states");
        std::fs::create_dir_all(&state_dir).unwrap();
        std::fs::write(state_dir.join("todo.json"), b"{\"k\":1}").unwrap();

        let check = git(&["check-ignore", "-q", ".plexi/app_states/todo.json"]);
        assert!(
            check.status.success(),
            "git must ignore the state file (check-ignore exit {:?})",
            check.status.code()
        );
        let status = git(&["status", "--porcelain"]);
        let listing = String::from_utf8_lossy(&status.stdout).to_string();
        assert!(
            !listing.contains("app_states"),
            "git status must not surface app state: {listing:?}"
        );
    }

    // ── write_default_route tests ──────────────────────────────────────────────

    #[test]
    fn write_default_route_creates_file_when_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let channel_dir = crate::config::workspace_channel_dir();
        std::fs::create_dir_all(tmp.path().join(&channel_dir)).unwrap();
        write_default_route(tmp.path(), "AGE", "AGE").expect("should succeed");
        let raw =
            std::fs::read_to_string(tmp.path().join(&channel_dir).join("secrets.toml")).unwrap();
        assert!(raw.contains("fallback = true"), "missing fallback: {raw}");
        assert!(raw.contains("[default]"), "missing section: {raw}");
        assert!(raw.contains("AGE = \"AGE\""), "missing route: {raw}");
        WorkspaceSecrets::parse(&raw).expect("created file must parse");
    }

    #[test]
    fn write_default_route_idempotent_when_route_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let channel_dir = crate::config::workspace_channel_dir();
        std::fs::create_dir_all(tmp.path().join(&channel_dir)).unwrap();
        let initial = "fallback = true\n\n[default]\nAGE = \"AGE\"\n";
        std::fs::write(tmp.path().join(&channel_dir).join("secrets.toml"), initial).unwrap();
        write_default_route(tmp.path(), "AGE", "AGE").expect("should succeed");
        let raw =
            std::fs::read_to_string(tmp.path().join(&channel_dir).join("secrets.toml")).unwrap();
        // Content must not grow — no duplicate entries.
        assert_eq!(
            raw.matches("AGE = \"AGE\"").count(),
            1,
            "duplicate entry: {raw}"
        );
    }

    #[test]
    fn write_default_route_appends_to_existing_default_section() {
        let tmp = tempfile::tempdir().unwrap();
        let channel_dir = crate::config::workspace_channel_dir();
        std::fs::create_dir_all(tmp.path().join(&channel_dir)).unwrap();
        let initial = "fallback = true\n\n[default]\nGITHUB_TOKEN = \"gh_personal\"\n";
        std::fs::write(tmp.path().join(&channel_dir).join("secrets.toml"), initial).unwrap();
        write_default_route(tmp.path(), "AGE", "AGE").expect("should succeed");
        let raw =
            std::fs::read_to_string(tmp.path().join(&channel_dir).join("secrets.toml")).unwrap();
        assert!(
            raw.contains("GITHUB_TOKEN = \"gh_personal\""),
            "existing entry lost: {raw}"
        );
        assert!(raw.contains("AGE = \"AGE\""), "new entry missing: {raw}");
        WorkspaceSecrets::parse(&raw).expect("file must still parse");
    }

    #[test]
    fn write_default_route_appends_new_section_when_none_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let channel_dir = crate::config::workspace_channel_dir();
        std::fs::create_dir_all(tmp.path().join(&channel_dir)).unwrap();
        let initial = "fallback = true\n\n# [default]\n# GITHUB_TOKEN = \"gh\"\n";
        std::fs::write(tmp.path().join(&channel_dir).join("secrets.toml"), initial).unwrap();
        write_default_route(tmp.path(), "AGE", "AGE").expect("should succeed");
        let raw =
            std::fs::read_to_string(tmp.path().join(&channel_dir).join("secrets.toml")).unwrap();
        assert!(raw.contains("[default]"), "section missing: {raw}");
        assert!(raw.contains("AGE = \"AGE\""), "entry missing: {raw}");
        WorkspaceSecrets::parse(&raw).expect("file must parse");
    }

    #[test]
    fn write_default_route_with_alias_writes_friendly_name() {
        let tmp = tempfile::tempdir().unwrap();
        let channel_dir = crate::config::workspace_channel_dir();
        std::fs::create_dir_all(tmp.path().join(&channel_dir)).unwrap();
        write_default_route(tmp.path(), "OPENAI_API_KEY", "openai_personal")
            .expect("should succeed");
        let raw =
            std::fs::read_to_string(tmp.path().join(&channel_dir).join("secrets.toml")).unwrap();
        assert!(
            raw.contains("OPENAI_API_KEY = \"openai_personal\""),
            "route wrong: {raw}"
        );
    }

    #[test]
    fn write_default_route_updates_existing_entry_when_alias_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let channel_dir = crate::config::workspace_channel_dir();
        std::fs::create_dir_all(tmp.path().join(&channel_dir)).unwrap();
        let initial = "fallback = true\n\n[default]\nOPENAI_API_KEY = \"old_alias\"\n";
        std::fs::write(tmp.path().join(&channel_dir).join("secrets.toml"), initial).unwrap();
        write_default_route(tmp.path(), "OPENAI_API_KEY", "new_alias").expect("should succeed");
        let raw =
            std::fs::read_to_string(tmp.path().join(&channel_dir).join("secrets.toml")).unwrap();
        assert!(
            raw.contains("OPENAI_API_KEY = \"new_alias\""),
            "updated entry missing: {raw}"
        );
        assert!(!raw.contains("old_alias"), "stale entry not removed: {raw}");
        WorkspaceSecrets::parse(&raw).expect("must parse");
    }

    #[test]
    fn write_terminal_env_inject_creates_file_when_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let channel_dir = crate::config::workspace_channel_dir();
        std::fs::create_dir_all(tmp.path().join(&channel_dir)).unwrap();

        write_terminal_env_inject(tmp.path(), "OPENROUTER_API_KEY", true).expect("should succeed");

        let raw =
            std::fs::read_to_string(tmp.path().join(&channel_dir).join("secrets.toml")).unwrap();
        let parsed = WorkspaceSecrets::parse(&raw).expect("must parse");
        assert!(parsed.fallback);
        assert_eq!(
            parsed.terminal.env.inject,
            vec!["OPENROUTER_API_KEY".to_string()]
        );
    }

    #[test]
    fn write_terminal_env_inject_adds_and_removes_name() {
        let tmp = tempfile::tempdir().unwrap();
        let channel_dir = crate::config::workspace_channel_dir();
        std::fs::create_dir_all(tmp.path().join(&channel_dir)).unwrap();
        let initial = "fallback = true\n\n[default]\nOPENAI_API_KEY = \"openai\"\n";
        std::fs::write(tmp.path().join(&channel_dir).join("secrets.toml"), initial).unwrap();

        write_terminal_env_inject(tmp.path(), "OPENAI_API_KEY", true).expect("enable");
        write_terminal_env_inject(tmp.path(), "OPENROUTER_API_KEY", true).expect("enable");
        write_terminal_env_inject(tmp.path(), "OPENAI_API_KEY", false).expect("disable");

        let raw =
            std::fs::read_to_string(tmp.path().join(&channel_dir).join("secrets.toml")).unwrap();
        let parsed = WorkspaceSecrets::parse(&raw).expect("must parse");
        assert_eq!(
            parsed.default.get("OPENAI_API_KEY").map(String::as_str),
            Some("openai")
        );
        assert_eq!(
            parsed.terminal.env.inject,
            vec!["OPENROUTER_API_KEY".to_string()]
        );
    }

    #[test]
    fn write_terminal_env_inject_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let channel_dir = crate::config::workspace_channel_dir();
        std::fs::create_dir_all(tmp.path().join(&channel_dir)).unwrap();
        let initial = "fallback = true\n\n[terminal.env]\ninject = [\n  \"OPENAI_API_KEY\",\n]\n";
        std::fs::write(tmp.path().join(&channel_dir).join("secrets.toml"), initial).unwrap();

        write_terminal_env_inject(tmp.path(), "OPENAI_API_KEY", true).expect("enable");

        let raw =
            std::fs::read_to_string(tmp.path().join(&channel_dir).join("secrets.toml")).unwrap();
        assert_eq!(
            raw.matches("OPENAI_API_KEY").count(),
            1,
            "duplicate entry: {raw}"
        );
    }

    #[test]
    fn upsert_default_route_line_appends_when_no_section() {
        let raw = "fallback = true\n";
        let out = upsert_default_route_line(raw, "X", "x_alias");
        assert!(out.contains("[default]"), "{out}");
        assert!(out.contains("X = \"x_alias\""), "{out}");
        WorkspaceSecrets::parse(&out).expect("must parse: {out}");
    }

    #[test]
    fn upsert_default_route_line_inserts_before_next_section() {
        let raw = "fallback = true\n\n[default]\nA = \"a\"\n\n[apps.foo]\nB = \"b\"\n";
        let out = upsert_default_route_line(raw, "C", "c");
        // C must appear inside [default], before [apps.foo]
        let default_pos = out.find("[default]").unwrap();
        let apps_pos = out.find("[apps.foo]").unwrap();
        let c_pos = out.find("C = \"c\"").unwrap();
        assert!(
            c_pos > default_pos && c_pos < apps_pos,
            "C not in [default]: {out}"
        );
        WorkspaceSecrets::parse(&out).expect("must parse");
    }

    #[test]
    fn upsert_default_route_line_handles_inline_comment_on_section_header() {
        let raw = "fallback = true\n\n[default] # route table\nA = \"a\"\n";
        let out = upsert_default_route_line(raw, "B", "b_alias");
        assert!(out.contains("B = \"b_alias\""), "entry missing: {out}");
        assert!(
            out.contains("[default] # route table"),
            "header modified: {out}"
        );
        WorkspaceSecrets::parse(&out).expect("must parse");
    }

    #[test]
    fn upsert_default_route_line_replaces_existing_entry() {
        let raw = "fallback = true\n\n[default]\nFOO = \"old\"\nBAR = \"bar\"\n";
        let out = upsert_default_route_line(raw, "FOO", "new");
        assert!(out.contains("FOO = \"new\""), "replacement missing: {out}");
        assert!(
            !out.contains("FOO = \"old\""),
            "old entry not removed: {out}"
        );
        assert!(out.contains("BAR = \"bar\""), "sibling entry lost: {out}");
        WorkspaceSecrets::parse(&out).expect("must parse");
    }
}
