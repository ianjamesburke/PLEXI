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
//!
//! ## Module layout
//!
//! - [`store`] — the `NonDestructiveStore` / `SecretStore` trait split and the
//!   keychain backends.
//! - [`index`] — the `secrets-index.json` cache (compiled out under test).
//! - [`reconcile`] — index ↔ keychain reconciliation and canonical spellings.
//! - [`resolver`] — the workspace TOML model and the canonical-name resolver.
//! - [`file`] — `workspace init` scaffolding and `secrets.toml` upserts.
//! - [`pane_read`] — a pane's `plexi secret get`, admitted by the host gate.
//!
//! This file is the module's public surface: the keychain account naming
//! helpers, [`system_store`], and the re-exports every caller reaches through.

mod file;
pub(crate) mod manager;
mod folder;
mod folder_store;
mod pane_read;
mod reconcile;
mod resolver;
mod store;

#[cfg(all(target_os = "macos", not(test)))]
mod index;

pub use store::{NonDestructiveStore, SecretStore};
#[cfg(all(target_os = "macos", not(test)))]
pub(crate) use store::keychain_call_lock;

// Only test code outside this module names these. `SecretError` is named from
// the macOS keychain-counting tests; every platform's tests name the in-memory
// store. In-module callers reach both through their defining submodule.
#[cfg(test)]
pub use store::InMemoryKeychain;
#[cfg(all(test, target_os = "macos"))]
pub use store::SecretError;

#[cfg(any(target_os = "macos", target_os = "linux", windows))]
pub use reconcile::reconcile_index_with_keychain;

pub use folder::{
    canonical_folder, deliver_folder_secret_choice, env_for_cwd, folder_secret_sheet_pending,
    grant_folder_secret, list_folder_secrets, read_folder_secret, read_through_host_gate,
    remove_folder_secret, set_folder_secret, take_folder_secret_sheet, ReadResult, SecretActor,
    SECRET_READ_TOOL,
};
pub use folder_store::backend_label;
pub use pane_read::{read_pane_secret, SecretScope};
#[cfg(test)]
pub use pane_read::SECRET_GET_TOOL;
pub use resolver::{
    resolve, resolve_terminal_env, resolve_with_source, ResolveOutcome, ResolveWithSourceOutcome,
    WorkspaceConfig, WorkspaceSecrets,
};

pub(crate) use file::ensure_app_state_gitignore;
pub use file::{init_workspace, write_default_route, write_terminal_env_inject};

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

/// `host` and `user` are not workspace ids. `plexi:host:*` is the seal-key
/// namespace and is not readable through workspace secret resolution.
pub fn workspace_id_is_reserved(id: &str) -> bool {
    let id = id.trim();
    id.is_empty()
        || id == "host"
        || id == "user"
        || id.contains(':')
        || id.chars().any(char::is_whitespace)
}

/// True when an account names the host seal store.
pub fn secret_account_is_reserved(account: &str) -> bool {
    account.starts_with("plexi:host:")
}

pub fn reserved_secret_error(workspace_id: &str) -> String {
    format!(
        "workspace id '{workspace_id}' is reserved. The host seal key is not a workspace secret and cannot be read with plexi secret get."
    )
}

// ── Backend selector ─────────────────────────────────────────────────────────

/// The process-wide secret store handle — the ONLY way to reach a store
/// backend. Production builds return the platform keychain — macOS Keychain
/// or Windows Credential Manager; test builds
/// ALWAYS return a process-local in-memory store, and the real backend type
/// is not even compiled under `cfg(test)`, so a test that tries to name it
/// does not build. Default-safe, opt-in-dangerous — except the opt-in does
/// not exist inside a test binary.
///
/// Why (2026-07-28): macOS keychain ACLs are per-binary, so every freshly
/// compiled test binary is a new unsigned app and each login-keychain value
/// read from a test fires its own credential dialog — an unattended agent
/// gate cannot click one, so a prompting test silently stalls automation.
/// Windows Credential Manager does not prompt, but a test binary writing into
/// the developer's real credential store is its own problem, so the same rule
/// applies there.
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
pub fn system_store() -> &'static dyn SecretStore {
    #[cfg(all(target_os = "macos", not(test)))]
    {
        static STORE: store::MacKeychain = store::MacKeychain;
        &STORE
    }
    #[cfg(all(target_os = "linux", not(test)))]
    {
        static STORE: LinuxUserStore = LinuxUserStore;
        &STORE
    }
    #[cfg(all(windows, not(test)))]
    {
        static STORE: store::CredentialManager = store::CredentialManager;
        &STORE
    }
    #[cfg(test)]
    {
        static STORE: std::sync::OnceLock<InMemoryKeychain> = std::sync::OnceLock::new();
        STORE.get_or_init(InMemoryKeychain::new)
    }
}

/// Linux user secrets use the same Secret Service/encrypted-file backend as
/// folder secrets.  The legacy JSON store is read once and removed only after
/// every value has reached the protected backend.
#[cfg(all(target_os = "linux", not(test)))]
struct LinuxUserStore;

#[cfg(all(target_os = "linux", not(test)))]
impl NonDestructiveStore for LinuxUserStore {
    fn get(&self, account: &str) -> Option<zeroize::Zeroizing<String>> {
        store::migrate_linux_file_store(folder_store::platform_folder_store());
        folder_store::platform_folder_store().get(account)
    }
    fn add_new(&self, account: &str, value: &str) -> Result<(), store::SecretError> {
        store::migrate_linux_file_store(folder_store::platform_folder_store());
        folder_store::platform_folder_store().add_new(account, value)
    }
    fn delete_if_value(&self, account: &str, expected: &str) -> Result<(), store::SecretError> {
        store::migrate_linux_file_store(folder_store::platform_folder_store());
        folder_store::platform_folder_store().delete_if_value(account, expected)
    }
    fn list_with_prefix(&self, prefix: &str) -> Vec<String> {
        store::migrate_linux_file_store(folder_store::platform_folder_store());
        folder_store::platform_folder_store().list_with_prefix(prefix)
    }
    fn scan_accounts(&self) -> Result<Vec<String>, store::SecretError> {
        store::migrate_linux_file_store(folder_store::platform_folder_store());
        folder_store::platform_folder_store().scan_accounts()
    }
}

#[cfg(all(target_os = "linux", not(test)))]
impl SecretStore for LinuxUserStore {
    fn set(&self, account: &str, value: &str) -> Result<(), store::SecretError> {
        store::migrate_linux_file_store(folder_store::platform_folder_store());
        folder_store::platform_folder_store().set(account, value)
    }
    fn delete(&self, account: &str) -> Result<(), store::SecretError> {
        store::migrate_linux_file_store(folder_store::platform_folder_store());
        folder_store::platform_folder_store().delete(account)
    }
}

/// Store for folder-scoped secrets. Tests get a process-local memory store.
/// Production uses the OS keychain, or on Linux the Secret Service with a
/// labeled encrypted-file fallback. This is not [`system_store`]: the Linux
/// workspace [`system_store`] is an unencrypted file, and folder secrets must
/// not use it.
pub fn folder_store() -> &'static dyn SecretStore {
    #[cfg(test)]
    {
        static STORE: std::sync::OnceLock<InMemoryKeychain> = std::sync::OnceLock::new();
        STORE.get_or_init(InMemoryKeychain::new)
    }
    #[cfg(all(not(test), target_os = "linux"))]
    {
        folder_store::platform_folder_store()
    }
    #[cfg(all(not(test), any(target_os = "macos", windows)))]
    {
        system_store()
    }
}

/// Shared test helper: build an owned account list from string literals.
#[cfg(test)]
fn accounts(names: &[&str]) -> Vec<String> {
    names.iter().map(|s| (*s).to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserved_ids_cover_the_host_namespace() {
        assert!(workspace_id_is_reserved("host"));
        assert!(workspace_id_is_reserved("user"));
        assert!(workspace_id_is_reserved(""));
        assert!(workspace_id_is_reserved("a:b"));
        assert!(workspace_id_is_reserved("has space"));
        assert!(!workspace_id_is_reserved("ws-1"));
        assert!(secret_account_is_reserved("plexi:host:permission-mac"));
        assert!(!secret_account_is_reserved("plexi:ws-1:permission-mac"));
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
}
