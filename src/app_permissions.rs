//! App permission enforcement — PGAP v3 capability-based model.
//!
//! Permissions are keyed by `(app_id, workspace_root, capability)` triple and
//! persisted to `permissions.jsonl` (append-only, one decision per line).
//!
//! The v2 boolean-field model (`terminal_write`, `filesystem`, etc.) is replaced
//! by a `HashSet<Capability>`.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt;
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
    /// Make LLM API calls via host broker (uses ANTHROPIC_API_KEY from secrets store).
    Llm,
    /// Set and cancel one-shot timers that fire PlexiEvent::Timer.
    Timer,
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl Capability {
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
            Self::Llm => "llm",
            Self::Timer => "timer",
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
            "llm" => Ok(Self::Llm),
            "timer" => Ok(Self::Timer),
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
    /// Allowed HTTP hosts. Empty = unrestricted.
    #[serde(default)]
    pub allowed_hosts: Vec<String>,
}

impl AppPermissions {
    /// Full permissions for built-in first-party apps — bypasses all checks.
    pub fn builtin() -> Self {
        Self {
            capabilities: HashSet::new(), // not needed — is_builtin bypasses checks
            is_builtin: true,
            allowed_hosts: vec![],
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
            allowed_hosts: vec![],
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



