/// App permission enforcement.
///
/// Every `AppCommand` passes through `check_command` before execution.
/// Built-in apps are pre-approved for everything. Third-party apps are
/// scoped to their launch directory and must declare capabilities in
/// their manifest.

use crate::app_trait::AppCommand;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Trust level determines the baseline permission set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustLevel {
    /// Ships with Plexi. All permissions granted.
    Builtin,
    /// User explicitly elevated this app.
    Trusted,
    /// Default for third-party. Strict scoping.
    Sandboxed,
}

impl Default for TrustLevel {
    fn default() -> Self {
        Self::Sandboxed
    }
}

/// Per-app permission grants.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppPermissions {
    #[serde(default)]
    pub trust_level: TrustLevel,
    /// Can send commands to the linked terminal PTY.
    #[serde(default)]
    pub terminal_write: bool,
    /// Filesystem access level.
    #[serde(default)]
    pub filesystem: FsPermission,
    /// Can read .env / credentials files.
    #[serde(default)]
    pub env_file_access: bool,
    /// Can make network requests (future — not enforced at OS level yet).
    #[serde(default)]
    pub network: bool,
    /// Can write secrets to Keychain via the API.
    #[serde(default)]
    pub secrets_write: bool,
}

impl Default for AppPermissions {
    fn default() -> Self {
        Self {
            trust_level: TrustLevel::Sandboxed,
            terminal_write: false,
            filesystem: FsPermission::ReadOnly,
            env_file_access: false,
            network: false,
            secrets_write: false,
        }
    }
}

impl AppPermissions {
    /// Full permissions for built-in apps.
    pub fn builtin() -> Self {
        Self {
            trust_level: TrustLevel::Builtin,
            terminal_write: true,
            filesystem: FsPermission::ReadWrite,
            env_file_access: false, // even builtins don't need .env by default
            network: true,
            secrets_write: false, // builtins use CLI directly; no API secret writes
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FsPermission {
    None,
    ReadOnly,
    ReadWrite,
}

impl Default for FsPermission {
    fn default() -> Self {
        Self::ReadOnly
    }
}

/// Result of a permission check.
#[derive(Debug)]
pub enum PermissionCheck {
    Allowed,
    Denied(String),
}

/// Check whether an `AppCommand` is allowed given the app's permissions and scope.
///
/// `scope_root` is the directory the app was launched from. Paths in commands
/// are resolved relative to this root; escapes above it are rejected for
/// sandboxed apps.
pub fn check_command(
    cmd: &AppCommand,
    perms: &AppPermissions,
    scope_root: &Path,
) -> PermissionCheck {
    // Built-in apps bypass all checks.
    if perms.trust_level == TrustLevel::Builtin {
        return PermissionCheck::Allowed;
    }

    match cmd {
        AppCommand::RunInTerminal(_) => {
            if perms.terminal_write {
                PermissionCheck::Allowed
            } else {
                PermissionCheck::Denied(
                    "App does not have terminal_write permission".to_string(),
                )
            }
        }
        AppCommand::Cd(path) => {
            if !perms.terminal_write {
                return PermissionCheck::Denied(
                    "App does not have terminal_write permission (needed for cd)".to_string(),
                );
            }
            // Scope check: path must be within or below scope_root.
            if !path_within_scope(path, scope_root) {
                return PermissionCheck::Denied(format!(
                    "Path {} is outside app scope {}",
                    path.display(),
                    scope_root.display()
                ));
            }
            PermissionCheck::Allowed
        }
        AppCommand::Notify(_) | AppCommand::DescendDepth(_) => {
            // Notifications and depth navigation are always allowed — non-destructive.
            PermissionCheck::Allowed
        }
    }
}

/// Returns true if `path` is equal to or a descendant of `scope_root`.
/// Resolves symlinks and `..` components via canonicalize.
fn path_within_scope(path: &Path, scope_root: &Path) -> bool {
    let Ok(canonical_path) = std::fs::canonicalize(path) else {
        // If the path doesn't exist yet, resolve what we can.
        // Try the parent — if the parent is in scope, the child would be too.
        return path
            .parent()
            .map(|p| path_within_scope(p, scope_root))
            .unwrap_or(false);
    };
    let Ok(canonical_root) = std::fs::canonicalize(scope_root) else {
        return false;
    };
    canonical_path.starts_with(&canonical_root)
}

// ─── Permissions config file ──────────────────────────────────────────────────

/// Top-level permissions config stored at `~/.plexi/permissions.toml`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PermissionsConfig {
    /// Global overrides.
    #[serde(default)]
    pub global: GlobalPermissions,
    /// Per-app permission overrides keyed by app id.
    #[serde(default)]
    pub apps: HashMap<String, AppPermissions>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalPermissions {
    /// Master kill switch for all terminal write access.
    #[serde(default = "default_true")]
    pub allow_terminal_write: bool,
    /// Master kill switch for all network access.
    #[serde(default = "default_true")]
    pub allow_network: bool,
    /// Never allow any app to read .env files.
    #[serde(default)]
    pub env_file_access: bool,
}

impl Default for GlobalPermissions {
    fn default() -> Self {
        Self {
            allow_terminal_write: true,
            allow_network: true,
            env_file_access: false,
        }
    }
}

fn default_true() -> bool {
    true
}

impl PermissionsConfig {
    /// Load from `~/.plexi/permissions.toml`, creating defaults if missing.
    pub fn load() -> Self {
        let path = permissions_path();
        match std::fs::read_to_string(&path) {
            Ok(content) => match toml::from_str(&content) {
                Ok(config) => config,
                Err(e) => {
                    log::warn!("Invalid permissions.toml, using defaults: {e}");
                    Self::default()
                }
            },
            Err(_) => Self::default(),
        }
    }

    /// Resolve the effective permissions for an app.
    /// Priority: per-app config > manifest defaults > sandboxed defaults.
    /// Global kill switches override everything.
    pub fn resolve(&self, app_id: &str, manifest_perms: Option<&AppPermissions>) -> AppPermissions {
        let base = self
            .apps
            .get(app_id)
            .cloned()
            .or_else(|| manifest_perms.cloned())
            .unwrap_or_default();

        // Apply global kill switches.
        AppPermissions {
            terminal_write: base.terminal_write && self.global.allow_terminal_write,
            network: base.network && self.global.allow_network,
            env_file_access: base.env_file_access && self.global.env_file_access,
            ..base
        }
    }
}

fn permissions_path() -> PathBuf {
    crate::config::config_dir().join("permissions.toml")
}

// ─── Runtime permission store ─────────────────────────────────────────────────

/// Persistent store for runtime capability decisions (yes_once, yes_always, no).
/// Stored as JSON at `{config_dir}/permissions.json`.
pub struct PermissionStore {
    path: PathBuf,
    decisions: HashMap<String, String>,
}

impl PermissionStore {
    pub fn new(path: PathBuf) -> Self {
        let decisions = if path.exists() {
            std::fs::read_to_string(&path)
                .ok()
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default()
        } else {
            HashMap::new()
        };
        Self { path, decisions }
    }

    /// Check whether a decision exists for app_id:capability.
    pub fn check(&self, app_id: &str, capability: &str) -> Option<&str> {
        self.decisions
            .get(&format!("{app_id}:{capability}"))
            .map(|s| s.as_str())
    }

    /// Record a decision for app_id:capability.
    pub fn record(&mut self, app_id: &str, capability: &str, decision: &str) {
        self.decisions
            .insert(format!("{app_id}:{capability}"), decision.to_string());
        match serde_json::to_string_pretty(&self.decisions) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&self.path, json) {
                    log::warn!("permission_store: failed to persist: {e}");
                }
            }
            Err(e) => log::warn!("permission_store: serialize error: {e}"),
        }
    }
}

/// Returns true if the app's `observes` list allows receiving events of `kind_tag`.
pub fn check_observes_capability(observes: &[String], kind_tag: &str) -> bool {
    observes.iter().any(|o| o == "*" || o == kind_tag)
}

/// Validate that a file path in an OpenIntent doesn't escape the workspace root.
pub fn validate_open_intent_path(path: &str, workspace_root: Option<&str>) -> Result<(), String> {
    if let Some(root) = workspace_root {
        let abs_path = std::path::Path::new(path);
        let abs_root = std::path::Path::new(root);
        if abs_path.is_absolute() {
            let canonical_path = abs_path
                .canonicalize()
                .unwrap_or_else(|_| abs_path.to_path_buf());
            let canonical_root = abs_root
                .canonicalize()
                .unwrap_or_else(|_| abs_root.to_path_buf());
            if !canonical_path.starts_with(&canonical_root) {
                return Err(format!(
                    "path {path:?} escapes workspace root {root:?}"
                ));
            }
        }
    }
    Ok(())
}
