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

// ── Workspace-secret index file (accounts only, no values) ───────────────────
//
// Replaces the legacy SecretEntry-based `secrets-index.json`. We store full
// account strings (`plexi:<scope>:<friendly>`) because that's the single
// natural primary key — workspace_id is embedded in the account name.

#[cfg(target_os = "macos")]
fn index_path() -> PathBuf {
    crate::config::config_dir().join("secrets-index.json")
}

#[cfg(target_os = "macos")]
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

#[cfg(target_os = "macos")]
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

#[cfg(target_os = "macos")]
fn index_add(account: &str) {
    let mut entries = index_read();
    if !entries.iter().any(|a| a == account) {
        entries.push(account.to_string());
        index_write(&entries);
    }
}

#[cfg(target_os = "macos")]
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
            log::warn!("workspace_secrets::migrate: failed to write {new_account}: {e}");
            continue;
        }
        log::info!("workspace_secrets::migrate: {legacy_account} → {new_account}");
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
        g.keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect()
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
    store: &dyn SecretStore,
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
    store: &dyn SecretStore,
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
    store: &dyn SecretStore,
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
