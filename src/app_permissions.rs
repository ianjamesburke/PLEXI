/// App permission enforcement — PGAP v3 capability-based model.
///
/// Permissions are keyed by `(app_id, workspace_root, capability)` triple and
/// persisted to `permissions.jsonl` (append-only, one decision per line).
///
/// The v2 boolean-field model (`terminal_write`, `filesystem`, etc.) is replaced
/// by a `HashSet<Capability>`. `check_command` is kept for back-compat with
/// existing call sites that will be properly migrated in Layer 3.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt;
use std::path::{Path, PathBuf};

// ── Capability enum ───────────────────────────────────────────────────────────

/// v3 capability set. Matches the strings in manifest.toml / Init handshake.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    /// Read files within workspace_root.
    FsRead,
    /// Write files within workspace_root.
    FsWrite,
    /// Outbound HTTP(S) requests.
    NetHttp,
    /// Call SecretGet (scoped to workspace_root).
    SecretsGet,
    /// Record from host audio device.
    AudioRecord,
    /// Play audio via host audio device.
    AudioPlayback,
    /// Decode and display video via host video subsystem.
    VideoPlayback,
    /// Open typed pipes (JSON or binary mode).
    PipeOpen,
    /// Launch another app in a new pane.
    SpawnApp,
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::FsRead => "fs.read",
            Self::FsWrite => "fs.write",
            Self::NetHttp => "net.http",
            Self::SecretsGet => "secrets.get",
            Self::AudioRecord => "audio.record",
            Self::AudioPlayback => "audio.playback",
            Self::VideoPlayback => "video.playback",
            Self::PipeOpen => "pipe.open",
            Self::SpawnApp => "spawn.app",
        };
        write!(f, "{s}")
    }
}

impl<'a> From<&'a str> for Capability {
    /// Parses a manifest capability string. Unknown strings silently map to `FsRead`
    /// as a safe no-op default — callers should validate manifest strings at load time.
    fn from(s: &'a str) -> Self {
        match s {
            "fs.read" => Self::FsRead,
            "fs.write" => Self::FsWrite,
            "net.http" => Self::NetHttp,
            "secrets.get" => Self::SecretsGet,
            "audio.record" => Self::AudioRecord,
            "audio.playback" => Self::AudioPlayback,
            "video.playback" => Self::VideoPlayback,
            "pipe.open" => Self::PipeOpen,
            "spawn.app" => Self::SpawnApp,
            _ => {
                log::warn!("app_permissions: unknown capability string '{s}', defaulting to FsRead");
                Self::FsRead
            }
        }
    }
}

// ── AppPermissions ────────────────────────────────────────────────────────────

/// Per-app, per-workspace permission set (v3).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppPermissions {
    /// Granted capabilities for this app in this workspace.
    pub capabilities: HashSet<Capability>,
    /// True when this is a built-in app; bypasses all capability checks.
    #[serde(default)]
    pub is_builtin: bool,
}

impl AppPermissions {
    /// Full permissions for built-in first-party apps — bypasses all checks.
    pub fn builtin() -> Self {
        Self {
            capabilities: HashSet::new(), // not needed — is_builtin bypasses checks
            is_builtin: true,
        }
    }

    /// Create a permissions set from a list of v3 capability strings.
    pub fn from_capability_strings(strings: &[String]) -> Self {
        Self {
            capabilities: strings.iter().map(|s| Capability::from(s.as_str())).collect(),
            is_builtin: false,
        }
    }
}

// ── PermissionCheck result ────────────────────────────────────────────────────

/// Result of a permission check.
#[derive(Debug)]
pub enum PermissionCheck {
    Allowed,
    Denied(String),
}

// ── v3 check API ──────────────────────────────────────────────────────────────

/// Check whether a specific capability is granted for an app+workspace pair.
pub fn check(perms: &AppPermissions, cap: Capability) -> PermissionCheck {
    if perms.is_builtin {
        return PermissionCheck::Allowed;
    }
    if perms.capabilities.contains(&cap) {
        PermissionCheck::Allowed
    } else {
        PermissionCheck::Denied(format!(
            "App does not have the '{}' capability. Use CapabilityRequest to prompt the user.",
            cap
        ))
    }
}

// ── v3 command-level permission helpers ───────────────────────────────────────

/// Check whether a `Cd` app command is allowed (requires FsWrite + path in scope).
/// Used by app.rs to gate directory changes from apps.
pub fn check_cd(path: &Path, perms: &AppPermissions, scope_root: &Path) -> PermissionCheck {
    if perms.is_builtin {
        return PermissionCheck::Allowed;
    }
    if !perms.capabilities.contains(&Capability::FsWrite) {
        return PermissionCheck::Denied("Cd requires fs.write capability".to_string());
    }
    if !path_within_scope(path, scope_root) {
        return PermissionCheck::Denied(format!(
            "Path {} is outside app workspace scope {}",
            path.display(),
            scope_root.display()
        ));
    }
    PermissionCheck::Allowed
}

/// Returns true if `path` is equal to or a descendant of `scope_root`.
fn path_within_scope(path: &Path, scope_root: &Path) -> bool {
    let Ok(canonical_path) = std::fs::canonicalize(path) else {
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

// ── permissions.jsonl persistence ────────────────────────────────────────────

/// One persisted decision (one line in permissions.jsonl).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionDecision {
    pub app_id: String,
    pub workspace_root: String,
    pub capability: String,
    pub granted: bool,
    pub at: String, // RFC3339
}

/// Append-only permissions log at `~/.plexi/permissions.jsonl`.
pub struct PermissionsLog {
    decisions: Vec<PermissionDecision>,
}

impl PermissionsLog {
    /// Load all decisions from permissions.jsonl.
    pub fn load() -> Self {
        let path = permissions_jsonl_path();
        let decisions = match std::fs::read_to_string(&path) {
            Ok(content) => content
                .lines()
                .filter(|l| !l.trim().is_empty())
                .filter_map(|line| {
                    serde_json::from_str::<PermissionDecision>(line).map_err(|e| {
                        log::warn!("app_permissions: failed to parse permissions.jsonl line: {e}");
                        e
                    }).ok()
                })
                .collect(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => {
                log::warn!("app_permissions: failed to read permissions.jsonl: {e}");
                Vec::new()
            }
        };
        Self { decisions }
    }

    /// Check whether a specific (app_id, workspace_root, capability) triple has a recorded decision.
    pub fn check(&self, app_id: &str, workspace_root: &Path, cap: Capability) -> Option<bool> {
        let root_str = workspace_root.to_string_lossy();
        let cap_str = cap.to_string();
        // Last recorded decision for this triple wins (append-only log; latest = authoritative).
        self.decisions
            .iter()
            .rev()
            .find(|d| d.app_id == app_id && d.workspace_root == root_str && d.capability == cap_str)
            .map(|d| d.granted)
    }

    /// Record a new decision and append it to the log file.
    pub fn record(&mut self, app_id: &str, workspace_root: &Path, cap: Capability, granted: bool) {
        let decision = PermissionDecision {
            app_id: app_id.to_string(),
            workspace_root: workspace_root.to_string_lossy().to_string(),
            capability: cap.to_string(),
            granted,
            at: Utc::now().to_rfc3339(),
        };
        self.decisions.push(decision.clone());
        self.append_to_file(&decision);
    }

    fn append_to_file(&self, decision: &PermissionDecision) {
        let path = permissions_jsonl_path();
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                log::error!("app_permissions: failed to create config dir: {e}");
                return;
            }
        }
        match serde_json::to_string(decision) {
            Ok(mut line) => {
                line.push('\n');
                use std::io::Write;
                match std::fs::OpenOptions::new().create(true).append(true).open(&path) {
                    Ok(mut f) => {
                        if let Err(e) = f.write_all(line.as_bytes()) {
                            log::error!("app_permissions: failed to append to permissions.jsonl: {e}");
                        }
                    }
                    Err(e) => log::error!("app_permissions: failed to open permissions.jsonl for append: {e}"),
                }
            }
            Err(e) => log::error!("app_permissions: failed to serialize decision: {e}"),
        }
    }

    /// Build an `AppPermissions` set from persisted decisions for a given (app_id, workspace_root).
    pub fn resolve(&self, app_id: &str, workspace_root: &Path) -> AppPermissions {
        let root_str = workspace_root.to_string_lossy();
        let capabilities = self
            .decisions
            .iter()
            .filter(|d| d.app_id == app_id && d.workspace_root == root_str && d.granted)
            .map(|d| Capability::from(d.capability.as_str()))
            .collect();
        AppPermissions {
            capabilities,
            is_builtin: false,
        }
    }
}

fn permissions_jsonl_path() -> PathBuf {
    crate::config::config_dir().join("permissions.jsonl")
}

