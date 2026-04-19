//! App permission enforcement — PGAP v3 capability-based model.
//!
//! Permissions are keyed by `(app_id, workspace_root, capability)` triple and
//! persisted to `permissions.jsonl` (append-only, one decision per line).
//!
//! The v2 boolean-field model (`terminal_write`, `filesystem`, etc.) is replaced
//! by a `HashSet<Capability>`.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt;
use std::path::{Path, PathBuf};
#[allow(unused_imports)]
use std::convert::TryFrom;

// ── Capability enum ───────────────────────────────────────────────────────────

/// v3 capability set. Matches the strings in manifest.toml / Init handshake.
/// Nine spec capabilities (`docs/specs/releases/plexi-v3.0.md §4`).
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
    /// Open typed pipes (JSON or binary mode).
    PipeOpen,
    /// Launch another app in a new pane.
    SpawnApp,
    /// Capture microphone audio via host broker.
    AudioRecord,
    /// Play audio via host `rodio` broker.
    AudioPlayback,
    /// Decode and display video via host broker.
    VideoPlayback,
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl Capability {
    pub const ALL: &'static [Capability] = &[
        Self::FsRead,
        Self::FsWrite,
        Self::NetHttp,
        Self::SecretsGet,
        Self::PipeOpen,
        Self::SpawnApp,
        Self::AudioRecord,
        Self::AudioPlayback,
        Self::VideoPlayback,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::FsRead => "fs.read",
            Self::FsWrite => "fs.write",
            Self::NetHttp => "net.http",
            Self::SecretsGet => "secrets.get",
            Self::PipeOpen => "pipe.open",
            Self::SpawnApp => "spawn.app",
            Self::AudioRecord => "audio.record",
            Self::AudioPlayback => "audio.playback",
            Self::VideoPlayback => "video.playback",
        }
    }
}

/// Error produced when a manifest or decision log names a capability that is not
/// in the `Capability` enum. This is the replacement for the old `From<&str>`
/// silent fallback to `FsRead`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownCapability(pub String);

impl fmt::Display for UnknownCapability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown capability: '{}'", self.0)
    }
}

impl std::error::Error for UnknownCapability {}

impl<'a> TryFrom<&'a str> for Capability {
    type Error = UnknownCapability;

    fn try_from(s: &'a str) -> Result<Self, Self::Error> {
        match s {
            "fs.read" => Ok(Self::FsRead),
            "fs.write" => Ok(Self::FsWrite),
            "net.http" => Ok(Self::NetHttp),
            "secrets.get" => Ok(Self::SecretsGet),
            "pipe.open" => Ok(Self::PipeOpen),
            "spawn.app" => Ok(Self::SpawnApp),
            "audio.record" => Ok(Self::AudioRecord),
            "audio.playback" => Ok(Self::AudioPlayback),
            "video.playback" => Ok(Self::VideoPlayback),
            other => Err(UnknownCapability(other.to_string())),
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
    /// Unknown capability strings are logged and dropped — manifest loaders
    /// should validate with `parse_capability_strings` first and refuse to
    /// install an app that names an unknown capability.
    pub fn from_capability_strings(strings: &[String]) -> Self {
        let mut capabilities = HashSet::new();
        for s in strings {
            match Capability::try_from(s.as_str()) {
                Ok(cap) => {
                    capabilities.insert(cap);
                }
                Err(e) => {
                    log::warn!("app_permissions: {e}; dropped");
                }
            }
        }
        Self {
            capabilities,
            is_builtin: false,
        }
    }
}

/// Parse a list of capability strings, returning an error on the first unknown
/// entry. Manifest loaders should call this before installing an app.
pub fn parse_capability_strings(strings: &[String]) -> Result<HashSet<Capability>, UnknownCapability> {
    strings
        .iter()
        .map(|s| Capability::try_from(s.as_str()))
        .collect()
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
        let decisions =
            match std::fs::read_to_string(&path) {
                Ok(content) => {
                    content
                        .lines()
                        .filter(|l| !l.trim().is_empty())
                        .filter_map(|line| {
                            serde_json::from_str::<PermissionDecision>(line).map_err(|e| {
                        log::warn!("app_permissions: failed to parse permissions.jsonl line: {e}");
                        e
                    }).ok()
                        })
                        .collect()
                }
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
                match std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)
                {
                    Ok(mut f) => {
                        if let Err(e) = f.write_all(line.as_bytes()) {
                            log::error!(
                                "app_permissions: failed to append to permissions.jsonl: {e}"
                            );
                        }
                    }
                    Err(e) => log::error!(
                        "app_permissions: failed to open permissions.jsonl for append: {e}"
                    ),
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
            .filter_map(|d| match Capability::try_from(d.capability.as_str()) {
                Ok(cap) => Some(cap),
                Err(e) => {
                    log::warn!("app_permissions: decision log {e}; skipped");
                    None
                }
            })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn try_from_parses_all_nine_spec_capabilities() {
        for cap in Capability::ALL {
            let s = cap.as_str();
            let parsed = Capability::try_from(s).expect("canonical string roundtrips");
            assert_eq!(&parsed, cap, "roundtrip mismatch for '{s}'");
        }
    }

    #[test]
    fn try_from_rejects_unknown_capability() {
        let err = Capability::try_from("net.http_").expect_err("typo should not parse");
        assert_eq!(err.0, "net.http_");
    }

    #[test]
    fn parse_capability_strings_fails_on_first_unknown() {
        let strings = vec!["fs.read".to_string(), "bogus".to_string()];
        let err = parse_capability_strings(&strings).expect_err("bogus rejects the set");
        assert_eq!(err.0, "bogus");
    }

    #[test]
    fn parse_capability_strings_accepts_valid_set() {
        let strings = vec!["fs.read".to_string(), "net.http".to_string()];
        let caps = parse_capability_strings(&strings).expect("valid set parses");
        assert!(caps.contains(&Capability::FsRead));
        assert!(caps.contains(&Capability::NetHttp));
    }
}
