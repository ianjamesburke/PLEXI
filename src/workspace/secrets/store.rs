//! Keychain backends and the two-trait storage split.
//!
//! The trait split is the enforcement: [`NonDestructiveStore`] is the only
//! surface migration and reconciliation signatures accept, so a destructive
//! op there is a compile error. [`SecretStore`] adds the upsert and
//! unconditional delete that belong exclusively to user-initiated flows.
//!
//! [`super::system_store`] is the only selector for a real backend.

use zeroize::Zeroizing;

#[cfg(all(target_os = "macos", not(test)))]
use super::KEYCHAIN_SERVICE;
#[cfg(test)]
use std::collections::HashMap;

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

/// macOS Keychain backend via `security-framework`.
///
/// Maintains `~/.plexi-<channel>/secrets-index.json` so list operations work
/// without invoking `security dump-keychain` (which triggers an invisible
/// permission prompt). See DEV_LOG 2026-04-11.
///
/// Private, non-constructible outside this module, and absent from test
/// builds entirely — [`system_store`] is the only handle.
#[cfg(all(target_os = "macos", not(test)))]
pub(super) struct MacKeychain;

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
                super::index::index_add(account);
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
                super::index::index_remove(account);
                Ok(())
            }
            Some(_) => Err(SecretError::ValueChanged(account.to_string())),
        }
    }

    fn list_with_prefix(&self, prefix: &str) -> Vec<String> {
        super::index::index_read()
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
        super::index::index_add(account);
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
        super::index::index_remove(account);
        Ok(())
    }
}

/// Windows Credential Manager backend.
///
/// The Win32 analogue of [`MacKeychain`]: generic credentials, one per
/// account, persisted per-user. Unlike macOS there is no index sidecar —
/// `CredEnumerateW` takes a wildcard filter, so prefix listing and full scan
/// are both native and always agree with the backend.
///
/// Every account becomes the TargetName `plexi/<account>`. The prefix groups
/// Plexi's entries in the Credential Manager UI and gives `CredEnumerateW`
/// something to filter on; workspace accounts already start with `plexi:`, so
/// the result reads `plexi/plexi:<workspace-id>:<key>`. That redundancy is
/// deliberate — callers pass their account strings through verbatim and the
/// prefix rule stays a one-liner.
///
/// Private, non-constructible outside this module, and absent from test
/// builds entirely — [`super::system_store`] is the only handle.
#[cfg(all(windows, not(test)))]
pub(super) struct CredentialManager;

#[cfg(all(windows, not(test)))]
impl CredentialManager {
    /// Prefix applied to every account before it becomes a TargetName.
    const TARGET_PREFIX: &'static str = "plexi/";

    fn target_name(account: &str) -> String {
        format!("{}{account}", Self::TARGET_PREFIX)
    }

    /// NUL-terminated UTF-16, for Win32 PCWSTR / PWSTR arguments.
    fn to_wide_nul(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Decode a NUL-terminated UTF-16 buffer (e.g. `CREDENTIALW.TargetName`).
    /// The length is capped so a missing terminator cannot run off the end;
    /// real TargetNames are orders of magnitude under the cap.
    ///
    /// # Safety
    /// `ptr` must be null, or a valid NUL-terminated UTF-16 string the OS owns
    /// for the duration of the call.
    unsafe fn read_wide_nul(ptr: *const u16) -> String {
        const MAX_CHARS: usize = 4096;
        if ptr.is_null() {
            return String::new();
        }
        let mut len = 0usize;
        while len < MAX_CHARS && unsafe { *ptr.add(len) } != 0 {
            len += 1;
        }
        String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(ptr, len) })
    }

    /// Accounts whose TargetName matches `filter`, a wildcard pattern such as
    /// `plexi/*`. The `plexi/` prefix is stripped back off before returning,
    /// so callers only ever see the account strings they passed in.
    fn enumerate(filter: &str) -> Result<Vec<String>, SecretError> {
        use windows_sys::Win32::Foundation::ERROR_NOT_FOUND;
        use windows_sys::Win32::Security::Credentials::{
            CredEnumerateW, CredFree, CREDENTIALW,
        };

        let wide = Self::to_wide_nul(filter);
        let mut count: u32 = 0;
        let mut credentials: *mut *mut CREDENTIALW = std::ptr::null_mut();
        // SAFETY: NUL-terminated wide string; both out-params point at locals.
        let ok = unsafe { CredEnumerateW(wide.as_ptr(), 0, &mut count, &mut credentials) };
        if ok == 0 {
            let error = std::io::Error::last_os_error();
            // No Plexi credentials stored yet — an empty set, not a failure.
            if error.raw_os_error() == Some(ERROR_NOT_FOUND as i32) {
                return Ok(Vec::new());
            }
            return Err(SecretError::Backend(format!(
                "CredEnumerateW('{filter}') failed: {error}"
            )));
        }

        let mut accounts = Vec::with_capacity(count as usize);
        // SAFETY: on success `credentials` points at a single OS-allocated
        // block of `count` CREDENTIALW pointers, freed once below.
        for i in 0..count as usize {
            unsafe {
                let entry = *credentials.add(i);
                if entry.is_null() {
                    continue;
                }
                let target = Self::read_wide_nul((*entry).TargetName);
                accounts.push(
                    target
                        .strip_prefix(Self::TARGET_PREFIX)
                        .map(str::to_string)
                        .unwrap_or(target),
                );
            }
        }
        // SAFETY: frees the block CredEnumerateW allocated; nothing above
        // retained a pointer into it (every string was copied).
        unsafe { CredFree(credentials as *const core::ffi::c_void) };
        Ok(accounts)
    }

    fn write(account: &str, value: &str) -> Result<(), SecretError> {
        use windows_sys::Win32::Security::Credentials::{
            CredWriteW, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW,
        };

        let target = Self::target_name(account);
        // Both buffers are borrowed by `credential` and must outlive the call.
        let mut wide_target = Self::to_wide_nul(&target);
        let blob = Zeroizing::new(value.as_bytes().to_vec());

        // SAFETY: CREDENTIALW is a plain C struct; an all-zero value is the
        // documented starting point before filling in the fields below.
        let mut credential: CREDENTIALW = unsafe { std::mem::zeroed() };
        credential.Type = CRED_TYPE_GENERIC;
        credential.TargetName = wide_target.as_mut_ptr();
        credential.CredentialBlobSize = blob.len() as u32;
        credential.CredentialBlob = blob.as_ptr() as *mut u8;
        credential.Persist = CRED_PERSIST_LOCAL_MACHINE;

        // SAFETY: every pointer in `credential` is live until the explicit
        // drops below.
        let ok = unsafe { CredWriteW(&credential, 0) };
        let error = std::io::Error::last_os_error();
        // Explicit drops tie the buffer lifetimes past the FFI call; NLL would
        // otherwise be free to release them at their last named use above.
        drop(blob);
        drop(wide_target);

        if ok == 0 {
            return Err(SecretError::Backend(format!(
                "CredWriteW('{target}') failed: {error}"
            )));
        }
        Ok(())
    }

    fn remove(account: &str) -> Result<(), SecretError> {
        use windows_sys::Win32::Foundation::ERROR_NOT_FOUND;
        use windows_sys::Win32::Security::Credentials::{CredDeleteW, CRED_TYPE_GENERIC};

        let target = Self::target_name(account);
        let wide = Self::to_wide_nul(&target);
        // SAFETY: NUL-terminated wide string; the rest are plain flags.
        let ok = unsafe { CredDeleteW(wide.as_ptr(), CRED_TYPE_GENERIC, 0) };
        if ok == 0 {
            let error = std::io::Error::last_os_error();
            // Already gone — success, nothing to lose.
            if error.raw_os_error() == Some(ERROR_NOT_FOUND as i32) {
                return Ok(());
            }
            return Err(SecretError::Backend(format!(
                "CredDeleteW('{target}') failed: {error}"
            )));
        }
        Ok(())
    }
}

#[cfg(all(windows, not(test)))]
impl NonDestructiveStore for CredentialManager {
    fn get(&self, account: &str) -> Option<Zeroizing<String>> {
        use windows_sys::Win32::Foundation::ERROR_NOT_FOUND;
        use windows_sys::Win32::Security::Credentials::{
            CredFree, CredReadW, CRED_TYPE_GENERIC, CREDENTIALW,
        };

        let target = Self::target_name(account);
        let wide = Self::to_wide_nul(&target);
        let mut credential: *mut CREDENTIALW = std::ptr::null_mut();
        // SAFETY: NUL-terminated wide string; `credential` is a local out-param.
        let ok = unsafe { CredReadW(wide.as_ptr(), CRED_TYPE_GENERIC, 0, &mut credential) };
        if ok == 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(ERROR_NOT_FOUND as i32) {
                log::warn!(
                    "workspace_secrets::CredentialManager::get: CredReadW('{target}') failed: {error}"
                );
            }
            return None;
        }
        // SAFETY: CredReadW returned TRUE, so `credential` points at one
        // OS-allocated CREDENTIALW that stays valid until the CredFree below.
        let value = unsafe {
            let cred = &*credential;
            let size = cred.CredentialBlobSize as usize;
            if cred.CredentialBlob.is_null() || size == 0 {
                String::new()
            } else {
                String::from_utf8_lossy(std::slice::from_raw_parts(cred.CredentialBlob, size))
                    .trim()
                    .to_string()
            }
        };
        // SAFETY: frees the block CredReadW allocated; `value` owns its bytes.
        unsafe { CredFree(credential as *const core::ffi::c_void) };
        Some(Zeroizing::new(value))
    }

    fn add_new(&self, account: &str, value: &str) -> Result<(), SecretError> {
        // Read-then-write. `CredWriteW` has no create-only flag — the closest,
        // `CRED_PRESERVE_CREDENTIAL_BLOB`, still updates the entry and only
        // keeps the old blob — so unlike macOS's `SecItemAdd` the duplicate
        // check cannot be pushed into the backend. A concurrent writer landing
        // between this read and the write below still wins. The window is
        // irreducible here; do not document it as closed.
        if self.get(account).is_some() {
            return Err(SecretError::AlreadyExists(account.to_string()));
        }
        Self::write(account, value)
    }

    fn delete_if_value(&self, account: &str, expected: &str) -> Result<(), SecretError> {
        // Read-compare-delete, with the same irreducible gap as `add_new` and
        // as the macOS path above.
        match self.get(account) {
            None => Ok(()), // already gone — nothing to lose
            Some(current) if current.as_str() == expected => Self::remove(account),
            Some(_) => Err(SecretError::ValueChanged(account.to_string())),
        }
    }

    fn list_with_prefix(&self, prefix: &str) -> Vec<String> {
        // Filtered in Rust rather than by passing `plexi/<prefix>*` to
        // `CredEnumerateW`: a caller's prefix may contain `*` or `?`, which
        // the Win32 filter would interpret as wildcards.
        match Self::enumerate("plexi/*") {
            Ok(accounts) => accounts
                .into_iter()
                .filter(|account| account.starts_with(prefix))
                .collect(),
            Err(error) => {
                log::warn!(
                    "workspace_secrets::CredentialManager::list_with_prefix('{prefix}'): {error}"
                );
                Vec::new()
            }
        }
    }

    /// Names only. `CredEnumerateW` does return the blob alongside each entry,
    /// but nothing here reads it, so no secret value is materialised by a scan.
    fn scan_accounts(&self) -> Result<Vec<String>, SecretError> {
        let accounts = Self::enumerate("plexi/*")?;
        log::info!(
            "workspace_secrets::CredentialManager::scan_accounts: {} account(s)",
            accounts.len()
        );
        Ok(accounts)
    }
}

#[cfg(all(windows, not(test)))]
impl SecretStore for CredentialManager {
    fn set(&self, account: &str, value: &str) -> Result<(), SecretError> {
        // `CredWriteW` upserts, which is exactly the contract here.
        Self::write(account, value)
    }

    fn delete(&self, account: &str) -> Result<(), SecretError> {
        Self::remove(account)
    }
}

/// Linux file-backed secret store.
///
/// **This is not an OS keyring.** Linux has no Plexi-blessed Keychain
/// equivalent in v0 (libsecret/gnome-keyring integration is an explicit
/// non-goal — see `docs/linux-support-plan.md`), so values live in a
/// `0600` JSON file inside the channel profile directory. That is strictly
/// weaker than the macOS Keychain: anything running as the same user can read
/// it, and it is not encrypted at rest.
///
/// Why this exists rather than "no store on Linux": with no backend at all
/// every secret-consuming path — `plexi ai onboard`, the OpenRouter key
/// lookup, terminal env injection, the Secrets app — is dead code on Linux,
/// which both breaks the `-D warnings` gate across the whole subsystem and
/// makes the host unusable for its primary AI flow. A weak-but-honest store
/// keeps one code path on both platforms; the weakness is logged on first use
/// so it can never be mistaken for keyring-backed storage.
///
/// Private and non-constructible outside this module — [`super::system_store`]
/// is the only handle. Absent from test builds, like `MacKeychain`.
#[cfg(all(target_os = "linux", not(test)))]
pub(super) struct FileStore;

#[cfg(all(target_os = "linux", not(test)))]
impl FileStore {
    fn path() -> std::path::PathBuf {
        crate::config::config_dir().join("secrets.json")
    }

    /// Serializes every read-modify-write in this process. Cross-process
    /// atomicity is NOT provided — the rename below is atomic, but a
    /// concurrent writer in another process can still clobber an interleaved
    /// update. Do not document this as closed.
    fn lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn warn_once() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            log::info!(
                "workspace_secrets::FileStore: Linux has no keyring backend; secrets are stored \
                 in {} with mode 0600 — readable by any process running as this user",
                Self::path().display()
            );
        });
    }

    /// Fallible on purpose. An unreadable or corrupt store must NOT read as
    /// empty: every writer below does read-modify-write, so an empty read
    /// followed by a save would rewrite the file and destroy every secret it
    /// still holds. A missing file is the one case that really is empty.
    fn load() -> Result<std::collections::BTreeMap<String, String>, SecretError> {
        Self::warn_once();
        let path = Self::path();
        let raw = match std::fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Default::default()),
            Err(e) => {
                log::error!(
                    "workspace_secrets::FileStore: read {} failed: {e}",
                    path.display()
                );
                return Err(SecretError::Backend(format!(
                    "read {} failed: {e}",
                    path.display()
                )));
            }
        };
        serde_json::from_str(&raw).map_err(|e| {
            log::error!(
                "workspace_secrets::FileStore: {} is not valid JSON ({e}) — refusing to read or \
                 write until it is repaired, so no secret is lost to a rewrite",
                path.display()
            );
            SecretError::Backend(format!("{} is not valid JSON: {e}", path.display()))
        })
    }

    fn save(map: &std::collections::BTreeMap<String, String>) -> Result<(), SecretError> {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                SecretError::Backend(format!("create {} failed: {e}", parent.display()))
            })?;
        }
        let tmp = path.with_extension("json.tmp");
        let body = serde_json::to_vec_pretty(map)
            .map_err(|e| SecretError::Backend(format!("serialize secrets failed: {e}")))?;
        {
            // Create at 0600 rather than chmod after the fact — the widened
            // mode must never exist on disk, not even briefly.
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&tmp)
                .map_err(|e| {
                    SecretError::Backend(format!("open {} failed: {e}", tmp.display()))
                })?;
            f.write_all(&body)
                .and_then(|()| f.sync_all())
                .map_err(|e| {
                    SecretError::Backend(format!("write {} failed: {e}", tmp.display()))
                })?;
            // An existing tmp file from a crashed run keeps its old mode.
            let _ = f.set_permissions(std::fs::Permissions::from_mode(0o600));
        }
        std::fs::rename(&tmp, &path).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            SecretError::Backend(format!("rename into {} failed: {e}", path.display()))
        })
    }
}

#[cfg(all(target_os = "linux", not(test)))]
impl NonDestructiveStore for FileStore {
    fn get(&self, account: &str) -> Option<Zeroizing<String>> {
        let _guard = Self::lock();
        // `Option` has no room for "the store is broken" — `load` already
        // logged which file and why, so a miss here is not silent.
        Self::load()
            .ok()?
            .get(account)
            .map(|v| Zeroizing::new(v.clone()))
    }

    fn add_new(&self, account: &str, value: &str) -> Result<(), SecretError> {
        let _guard = Self::lock();
        let mut map = Self::load()?;
        if map.contains_key(account) {
            return Err(SecretError::AlreadyExists(account.to_string()));
        }
        map.insert(account.to_string(), value.to_string());
        Self::save(&map)
    }

    fn delete_if_value(&self, account: &str, expected: &str) -> Result<(), SecretError> {
        let _guard = Self::lock();
        let mut map = Self::load()?;
        match map.get(account) {
            None => Ok(()), // already gone — nothing to lose
            Some(current) if current == expected => {
                map.remove(account);
                Self::save(&map)
            }
            Some(_) => Err(SecretError::ValueChanged(account.to_string())),
        }
    }

    fn list_with_prefix(&self, prefix: &str) -> Vec<String> {
        let _guard = Self::lock();
        Self::load()
            .unwrap_or_default()
            .into_keys()
            .filter(|a| a.starts_with(prefix))
            .collect()
    }

    fn scan_accounts(&self) -> Result<Vec<String>, SecretError> {
        // The file IS the backend, so there is no index/backend skew to
        // reconcile here — the scan and the listing read the same bytes.
        let _guard = Self::lock();
        let accounts: Vec<String> = Self::load()?.into_keys().collect();
        log::info!(
            "workspace_secrets::scan: found {} secret(s) in the file store",
            accounts.len()
        );
        Ok(accounts)
    }
}

#[cfg(all(target_os = "linux", not(test)))]
impl SecretStore for FileStore {
    fn set(&self, account: &str, value: &str) -> Result<(), SecretError> {
        let _guard = Self::lock();
        let mut map = Self::load()?;
        map.insert(account.to_string(), value.to_string());
        Self::save(&map)
    }

    fn delete(&self, account: &str) -> Result<(), SecretError> {
        let _guard = Self::lock();
        let mut map = Self::load()?;
        map.remove(account);
        Self::save(&map)
    }
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
