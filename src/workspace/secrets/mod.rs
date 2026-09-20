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
//! - [`migrate`] — the one-shot startup migration of pre-#322 entries.
//! - [`reconcile`] — index ↔ keychain reconciliation and canonical spellings.
//! - [`resolver`] — the workspace TOML model and the canonical-name resolver.
//! - [`file`] — `workspace init` scaffolding and `secrets.toml` upserts.
//!
//! This file is the module's public surface: the keychain account naming
//! helpers, [`system_store`], and the re-exports every caller reaches through.

mod file;
mod migrate;
mod reconcile;
mod resolver;
mod store;

#[cfg(all(target_os = "macos", not(test)))]
mod index;

// Consumed by the macOS startup migration in `main.rs` and by tests. No other
// platform ever had a legacy Keychain to migrate from.
#[cfg(any(target_os = "macos", test))]
pub use migrate::migrate_legacy_global_secrets;
pub use store::{NonDestructiveStore, SecretStore};

// Only test code outside this module names the error type or the rename record;
// in-module callers reach both through their defining submodule.
#[cfg(test)]
pub use store::{InMemoryKeychain, SecretError};

#[cfg(any(target_os = "macos", windows))]
pub use reconcile::reconcile_index_with_keychain;
#[cfg(test)]
pub use reconcile::AccountRename;
pub use reconcile::ReconcileReport;

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
#[cfg(any(target_os = "macos", windows))]
pub fn system_store() -> &'static dyn SecretStore {
    #[cfg(all(target_os = "macos", not(test)))]
    {
        static STORE: store::MacKeychain = store::MacKeychain;
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

/// Shared test helper: build an owned account list from string literals.
#[cfg(test)]
fn accounts(names: &[&str]) -> Vec<String> {
    names.iter().map(|s| (*s).to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

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
