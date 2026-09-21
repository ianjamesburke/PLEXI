//! Index ↔ keychain reconciliation and canonical account spellings.
//!
//! The keychain is authoritative; `secrets-index.json` is a cache written only
//! when a secret goes through `SecretStore::set`. A key that reached the
//! keychain any other way (`security add-generic-password`, a pre-index build)
//! is readable by the resolver but invisible to every listing, so reconcile
//! rebuilds the index from an attributes-only scan and collapses legacy
//! friendly-name spellings onto one canonical name, non-destructively.

use std::collections::BTreeSet;

use super::store::{NonDestructiveStore, SecretError};
use super::KEYCHAIN_SERVICE;

#[cfg(all(target_os = "macos", not(test)))]
use super::index::{index_read, index_write};
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
use super::system_store;

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
pub(super) fn canonical_friendly(friendly: &str) -> Option<&'static str> {
    FRIENDLY_NAME_ALIASES
        .iter()
        .find(|(legacy, _)| *legacy == friendly)
        .map(|(_, canonical)| *canonical)
}

/// The canonical account for `account`, or `None` when it is already canonical
/// (or not a namespaced Plexi account).
fn canonical_account(account: &str) -> Option<String> {
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
pub(super) fn reconcile(
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

/// Index-free variant, for test builds and for Windows.
///
/// Test binaries have the index-file layer compiled out entirely — its only
/// possible target is the user's real `secrets-index.json` — so there is no
/// file to read back or persist. Windows never had one: `CredEnumerateW`
/// enumerates the backend directly, so the scan is already authoritative and
/// a sidecar could only go stale. Either way the scan is the whole truth.
#[cfg(all(target_os = "linux", not(test)))]
pub fn reconcile_index_with_keychain() -> Result<ReconcileReport, SecretError> {
    let store = system_store();
    let scanned = store.scan_accounts()?;
    Ok(reconcile(&scanned, &scanned, store))
}

#[cfg(any(all(any(target_os = "macos", target_os = "linux"), test), all(windows, not(test))))]
pub fn reconcile_index_with_keychain() -> Result<ReconcileReport, SecretError> {
    let store = system_store();
    let scanned = store.scan_accounts()?;
    Ok(reconcile(&scanned, &[], store))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::secrets::{accounts, InMemoryKeychain, SecretStore};

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
}
