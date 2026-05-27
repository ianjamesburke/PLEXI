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
//!    route is defined). Out-of-band of this module — see `process_app/routing.rs`.

use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

// ── Keychain naming ──────────────────────────────────────────────────────────

/// Build the workspace-namespaced Keychain account: `plexi:<workspace-id>:<friendly>`.
pub fn keychain_workspace_name(workspace_id: &str, friendly: &str) -> String {
    format!("plexi:{workspace_id}:{friendly}")
}

/// Build the user-scope (cross-workspace) Keychain account: `plexi:user:<friendly>`.
pub fn keychain_user_name(friendly: &str) -> String {
    format!("plexi:user:{friendly}")
}

// ── SecretStore trait + impls ────────────────────────────────────────────────

/// Abstraction over the actual storage backend so tests can swap in an
/// in-memory implementation. `account` is the full namespaced key
/// (e.g. `plexi:abc-123:openai_prod`).
pub trait SecretStore: Send + Sync {
    fn get(&self, account: &str) -> Option<Zeroizing<String>>;
    fn set(&self, account: &str, value: &str) -> Result<(), SecretError>;
    fn delete(&self, account: &str) -> Result<(), SecretError>;
    /// Best-effort listing of accounts with a given prefix. Used by the host
    /// to enumerate `plexi:<workspace-id>:*` entries for the missing-secret
    /// modal. macOS Keychain has no clean prefix-list — production impl
    /// reads from `secrets-index.json`.
    fn list_with_prefix(&self, prefix: &str) -> Vec<String>;
}

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("keychain backend error: {0}")]
    Backend(String),
}

/// macOS Keychain backend via the `security` CLI.
///
/// Maintains `~/.plexi-<channel>/secrets-index.json` so list operations work
/// without invoking `security dump-keychain` (which triggers an invisible
/// permission prompt). See DEV_LOG 2026-04-11.
#[cfg(target_os = "macos")]
pub struct MacKeychain;

#[cfg(target_os = "macos")]
impl MacKeychain {
    pub fn new() -> Self {
        Self
    }
}

#[cfg(target_os = "macos")]
impl SecretStore for MacKeychain {
    fn get(&self, account: &str) -> Option<Zeroizing<String>> {
        use security_framework::passwords::get_generic_password;
        match get_generic_password("plexi", account) {
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

    fn set(&self, account: &str, value: &str) -> Result<(), SecretError> {
        use security_framework::passwords::set_generic_password;
        set_generic_password("plexi", account, value.as_bytes())
            .map_err(|e| SecretError::Backend(format!("{e}")))?;
        index_add(account);
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<(), SecretError> {
        use security_framework::passwords::delete_generic_password;
        match delete_generic_password("plexi", account) {
            Ok(()) => {}
            // Already gone — treat as success.
            Err(e) if e.code() == -25300 => {}
            Err(e) => return Err(SecretError::Backend(format!("{e}"))),
        }
        index_remove(account);
        Ok(())
    }

    fn list_with_prefix(&self, prefix: &str) -> Vec<String> {
        index_read()
            .into_iter()
            .filter(|a| a.starts_with(prefix))
            .collect()
    }
}

/// Windows Credential Manager backend. Mirrors `MacKeychain`: values
/// live in the OS credential store and `secrets-index.json` tracks
/// accounts for prefix listing (so we don't have to enumerate the whole
/// credential set every time).
#[cfg(target_os = "windows")]
pub struct WinCredentialStore;

#[cfg(target_os = "windows")]
impl WinCredentialStore {
    pub fn new() -> Self {
        Self
    }
}

#[cfg(target_os = "windows")]
impl Default for WinCredentialStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(target_os = "windows")]
impl SecretStore for WinCredentialStore {
    fn get(&self, account: &str) -> Option<Zeroizing<String>> {
        crate::secrets_win::cred_read(account)
    }

    fn set(&self, account: &str, value: &str) -> Result<(), SecretError> {
        crate::secrets_win::cred_write(account, value).map_err(SecretError::Backend)?;
        index_add(account);
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<(), SecretError> {
        crate::secrets_win::cred_delete(account).map_err(SecretError::Backend)?;
        index_remove(account);
        Ok(())
    }

    fn list_with_prefix(&self, prefix: &str) -> Vec<String> {
        index_read()
            .into_iter()
            .filter(|a| a.starts_with(prefix))
            .collect()
    }
}

// ── Workspace-secret index file (accounts only, no values) ───────────────────
//
// Replaces the legacy SecretEntry-based `secrets-index.json`. We store full
// account strings (`plexi:<scope>:<friendly>`) because that's the single
// natural primary key — workspace_id is embedded in the account name.

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn index_path() -> PathBuf {
    crate::config::config_dir().join("secrets-index.json")
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
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
            index_write(&migrated);
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

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn index_write(entries: &[String]) {
    let path = index_path();
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            log::error!("workspace_secrets: failed to create config dir: {e}");
            return;
        }
    }
    match serde_json::to_string_pretty(entries) {
        Ok(s) => {
            if let Err(e) = std::fs::write(&path, s) {
                log::error!("workspace_secrets: failed to write index: {e}");
            }
        }
        Err(e) => log::error!("workspace_secrets: failed to serialize index: {e}"),
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn index_add(account: &str) {
    let mut entries = index_read();
    if !entries.iter().any(|a| a == account) {
        entries.push(account.to_string());
        index_write(&entries);
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn index_remove(account: &str) {
    let mut entries = index_read();
    entries.retain(|a| a != account);
    index_write(&entries);
}

/// One-shot migration on startup: any legacy `plexi-run/.../<key>` Keychain
/// entry referenced in the old index gets re-stored under
/// `plexi:user:<key>` (the friendly name == the canonical name). Logs every
/// migration. Idempotent — re-runs are no-ops once `secrets-index.json` is
/// in the new flat-string form.
#[cfg(target_os = "macos")]
pub fn migrate_legacy_global_secrets(store: &dyn SecretStore) -> usize {
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
        let value = match get_generic_password("plexi", &legacy_account) {
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
        if let Err(e) = store.set(&new_account, &value) {
            log::warn!(
                "workspace_secrets::migrate: failed to write {new_account}: {e}"
            );
            continue;
        }
        log::info!(
            "workspace_secrets::migrate: {legacy_account} → {new_account}"
        );
        migrated += 1;
    }
    migrated
}

#[cfg(not(target_os = "macos"))]
pub fn migrate_legacy_global_secrets(_store: &dyn SecretStore) -> usize {
    0
}

/// Pure in-memory `SecretStore` for tests. Wraps a `Mutex<HashMap>` for
/// interior mutability so tests can share a single instance behind `&dyn`.
#[cfg(test)]
pub struct InMemoryKeychain {
    store: std::sync::Mutex<HashMap<String, String>>,
}

#[cfg(test)]
impl InMemoryKeychain {
    pub fn new() -> Self {
        Self {
            store: std::sync::Mutex::new(HashMap::new()),
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
impl SecretStore for InMemoryKeychain {
    fn get(&self, account: &str) -> Option<Zeroizing<String>> {
        self.store
            .lock()
            .ok()?
            .get(account)
            .cloned()
            .map(Zeroizing::new)
    }

    fn set(&self, account: &str, value: &str) -> Result<(), SecretError> {
        let mut g = self
            .store
            .lock()
            .map_err(|e| SecretError::Backend(format!("mutex poisoned: {e}")))?;
        g.insert(account.to_string(), value.to_string());
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<(), SecretError> {
        let mut g = self
            .store
            .lock()
            .map_err(|e| SecretError::Backend(format!("mutex poisoned: {e}")))?;
        g.remove(account);
        Ok(())
    }

    fn list_with_prefix(&self, prefix: &str) -> Vec<String> {
        let g = match self.store.lock() {
            Ok(g) => g,
            Err(_) => return Vec::new(),
        };
        g.keys().filter(|k| k.starts_with(prefix)).cloned().collect()
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
    /// Read `<root>/.plexi/workspace.toml`. Returns `None` if the file does
    /// not exist; returns `Err` only on a present-but-invalid file.
    pub fn load(workspace_root: &Path) -> Result<Option<Self>, String> {
        let path = workspace_root.join(".plexi").join("workspace.toml");
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

    /// Read or create — if the file does not exist, generate a fresh UUID and
    /// write a minimal `id = "<uuid>"` file. The rest of `workspace.toml` is
    /// owned by #308 Phase 1; we only touch the `id` line.
    pub fn load_or_init(workspace_root: &Path) -> Result<Self, String> {
        if let Some(cfg) = Self::load(workspace_root)? {
            return Ok(cfg);
        }
        let id = uuid::Uuid::new_v4().to_string();
        let dir = workspace_root.join(".plexi");
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("create {}: {e}", dir.display()))?;
        let path = dir.join("workspace.toml");
        std::fs::write(&path, format!("id = \"{id}\"\n"))
            .map_err(|e| format!("write {}: {e}", path.display()))?;
        Ok(Self { id, context: None })
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
}

impl WorkspaceSecrets {
    pub fn parse(raw: &str) -> Result<Self, String> {
        toml::from_str::<Self>(raw).map_err(|e| format!("parse secrets.toml: {e}"))
    }

    /// Read `<root>/.plexi/secrets.toml`. Returns `None` if the file does
    /// not exist; returns `Err` if it exists but is malformed (incl. missing
    /// `fallback`).
    pub fn load(workspace_root: &Path) -> Result<Option<Self>, String> {
        let path = workspace_root.join(".plexi").join("secrets.toml");
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

/// 4-step runtime resolution. Pure function — no I/O beyond the `SecretStore`
/// trait calls. Tests use `InMemoryKeychain`; production uses `MacKeychain`.
pub fn resolve(
    workspace_id: &str,
    app_id: &str,
    canonical_name: &str,
    router: &WorkspaceSecrets,
    store: &dyn SecretStore,
) -> ResolveOutcome {
    // Step 1+2: workspace route (apps.<id> first, then [default]).
    if let Some(friendly) = router.route_for(app_id, canonical_name) {
        let account = keychain_workspace_name(workspace_id, friendly);
        if let Some(value) = store.get(&account) {
            return ResolveOutcome::Found(value);
        }
        // Route declared but Keychain is empty — prompt the user (don't
        // silently fall through to user-scope; the route was explicit).
        return ResolveOutcome::PromptUser;
    }

    // Step 3: user-scope fallback when allowed.
    if router.fallback {
        let user_account = keychain_user_name(canonical_name);
        if let Some(value) = store.get(&user_account) {
            return ResolveOutcome::Found(value);
        }
        return ResolveOutcome::PromptUser;
    }

    // Step 4: no route + fallback disabled → hard error.
    ResolveOutcome::HardMissing {
        reason: format!(
            "no route in .plexi/secrets.toml for app '{app_id}' / secret '{canonical_name}', \
             and fallback = false"
        ),
    }
}

// ── Workspace init helpers ───────────────────────────────────────────────────

/// `plexi workspace init` scaffolds:
///   - `<root>/.plexi/workspace.toml` with a fresh UUID (idempotent — keeps
///     existing id if present)
///   - `<root>/.plexi/secrets.toml` with `fallback = true` (chosen as the
///     ergonomic default; users opt into stricter `false` later)
///   - `<root>/.plexi/.gitignore` so the secrets file and host caches never
///     end up in git. **Existing `.gitignore` files are preserved verbatim**
///     so user edits survive subsequent `init` runs.
///
/// Returns the resolved `WorkspaceConfig` so the caller can echo the UUID.
pub fn init_workspace(workspace_root: &Path) -> Result<WorkspaceConfig, String> {
    let cfg = WorkspaceConfig::load_or_init(workspace_root)?;
    let secrets_path = workspace_root.join(".plexi").join("secrets.toml");
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
                        # GITHUB_TOKEN = \"github_personal\"\n";
        std::fs::write(&secrets_path, template)
            .map_err(|e| format!("write {}: {e}", secrets_path.display()))?;
    }
    write_gitignore_if_absent(workspace_root)?;
    Ok(cfg)
}

/// Default contents for `<root>/.plexi/.gitignore`. Anything that holds a
/// secret value or is generated host state lives here.
const GITIGNORE_TEMPLATE: &str = "# Auto-generated by plexi workspace init.\n\
                                  # Edit this file freely — re-running init never overwrites it.\n\
                                  secrets.toml\n\
                                  cache/\n";

/// Write `<root>/.plexi/.gitignore` only when the file does not already exist.
/// User edits to an existing file are preserved verbatim.
fn write_gitignore_if_absent(workspace_root: &Path) -> Result<(), String> {
    let dir = workspace_root.join(".plexi");
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let path = dir.join(".gitignore");
    if path.exists() {
        return Ok(());
    }
    std::fs::write(&path, GITIGNORE_TEMPLATE)
        .map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn router(toml_src: &str) -> WorkspaceSecrets {
        WorkspaceSecrets::parse(toml_src).expect("router parses")
    }

    #[test]
    fn keychain_naming_uses_workspace_and_user_namespaces() {
        assert_eq!(
            keychain_workspace_name("abc-123", "openai_prod"),
            "plexi:abc-123:openai_prod"
        );
        assert_eq!(keychain_user_name("github_token"), "plexi:user:github_token");
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
        store
            .set("plexi:ws-1:openai_prod", "sk-abc")
            .unwrap();
        let r = router(
            "fallback = false\n[apps.claude-code]\nOPENAI_API_KEY = \"openai_prod\"\n",
        );
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
    fn same_canonical_name_two_workspaces_returns_different_values() {
        // The whole point of workspace-scoping: same OPENAI_API_KEY in the
        // app, different bills downstream.
        let store = InMemoryKeychain::new();
        store.set("plexi:work:openai_prod", "sk-work").unwrap();
        store
            .set("plexi:personal:openai_personal", "sk-personal")
            .unwrap();
        let work_router = router(
            "fallback = false\n[apps.claude-code]\nOPENAI_API_KEY = \"openai_prod\"\n",
        );
        let personal_router = router(
            "fallback = false\n[apps.claude-code]\nOPENAI_API_KEY = \"openai_personal\"\n",
        );
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
        let r = router(
            "fallback = true\n[apps.claude-code]\nOPENAI_API_KEY = \"openai_prod\"\n",
        );
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
        let cfg = WorkspaceConfig::load_or_init(tmp.path()).expect("load_or_init");
        assert!(uuid::Uuid::parse_str(&cfg.id).is_ok());
        // Idempotent — second call returns the same id.
        let cfg2 = WorkspaceConfig::load_or_init(tmp.path()).expect("second load");
        assert_eq!(cfg.id, cfg2.id);
    }

    #[test]
    fn init_workspace_creates_workspace_and_secrets_files() {
        let tmp = tempfile::tempdir().unwrap();
        init_workspace(tmp.path()).expect("init_workspace");
        assert!(tmp.path().join(".plexi").join("workspace.toml").is_file());
        let secrets_raw =
            std::fs::read_to_string(tmp.path().join(".plexi").join("secrets.toml")).unwrap();
        // Generated secrets.toml must parse cleanly with the required field.
        let parsed = WorkspaceSecrets::parse(&secrets_raw).expect("template parses");
        assert!(parsed.fallback);
    }

    #[test]
    fn init_writes_gitignore_when_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let gitignore = tmp.path().join(".plexi").join(".gitignore");
        assert!(!gitignore.exists());

        init_workspace(tmp.path()).expect("init_workspace");

        assert!(
            gitignore.is_file(),
            "init must create .plexi/.gitignore"
        );
        let raw = std::fs::read_to_string(&gitignore).unwrap();
        assert!(raw.contains("secrets.toml"), "got: {raw}");
        assert!(raw.contains("cache/"), "got: {raw}");
    }

    #[test]
    fn init_preserves_existing_gitignore() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(".plexi");
        std::fs::create_dir_all(&dir).unwrap();
        let gitignore = dir.join(".gitignore");
        let custom = "# my own rules\nfoo\nbar\n";
        std::fs::write(&gitignore, custom).unwrap();

        init_workspace(tmp.path()).expect("init_workspace");

        let raw = std::fs::read_to_string(&gitignore).unwrap();
        assert_eq!(
            raw, custom,
            "init must NOT overwrite an existing .gitignore"
        );
    }
}
