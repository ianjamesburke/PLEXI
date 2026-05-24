//! Anchor — the `.plexi/` directory as a canonical context entry point.
//!
//! When a project directory contains `.plexi/`, that directory is an *anchor*:
//! opening Plexi from it attaches to a root context with pre-configured
//! metadata, secrets, and app layout. The anchor is purely local — no cloud
//! state.
//!
//! # Profile dir guard
//!
//! The home directory's `~/.plexi/` is the main channel profile dir, NOT an anchor.
//! [`Anchor::detect`] explicitly excludes it (and all `~/.plexi-*` profile
//! dirs) to prevent the profile from masquerading as a workspace anchor.

use std::path::{Path, PathBuf};

/// Optional context defaults read from `[context]` in `workspace.toml`.
/// These are suggestions, not mandates — user overrides always win.
#[derive(Debug, Clone, Default)]
pub struct ContextDefaults {
    pub name: Option<String>,
    pub description: Option<String>,
}

/// A `.plexi/` directory anchor. Wraps the project root that contains the
/// `.plexi/` dir, plus any defaults parsed from `workspace.toml`.
#[derive(Debug, Clone)]
pub struct Anchor {
    /// The project root directory (parent of `.plexi/`).
    pub root: PathBuf,
    /// Context defaults from `[context]` in `workspace.toml`.
    pub context_defaults: Option<ContextDefaults>,
}

impl Anchor {
    /// Detect an anchor at `path`. Returns `Some` if `path/.plexi/` exists
    /// and `path` is not a profile directory (`~/.plexi*`).
    ///
    /// Reads `workspace.toml` for workspace_id and context defaults if present.
    pub fn detect(path: &Path) -> Option<Self> {
        let dot_plexi = path.join(".plexi");
        if !dot_plexi.is_dir() {
            return None;
        }

        // Guard: never treat home dir as an anchor. ~/.plexi/ is the stable
        // profile dir, and ~/  would cause the AppRegistry to load stable
        // apps instead of the active channel's apps (issue #1064).
        if is_profile_dir(path) {
            log::info!(
                "anchor::detect: skipping profile dir at {}",
                path.display()
            );
            return None;
        }

        log::info!("anchor::detect: found .plexi/ anchor at {}", path.display());

        // Try to read workspace.toml for ID and context defaults.
        // Reuse WorkspaceConfig from workspace_secrets to avoid duplicate
        // parsing structs and dead-code warnings.
        let context_defaults =
            match crate::workspace_secrets::WorkspaceConfig::load(path) {
                Ok(Some(cfg)) => {
                    cfg.context.map(|c| ContextDefaults {
                        name: c.name,
                        description: c.description,
                    })
                }
                Ok(None) => None,
                Err(e) => {
                    log::warn!("anchor: workspace.toml parse error: {e}");
                    None
                }
            };

        if let Some(ref defaults) = context_defaults {
            log::info!(
                "anchor::detect: context defaults name={:?} description={:?}",
                defaults.name,
                defaults.description
            );
        }

        Some(Anchor {
            root: path.to_path_buf(),
            context_defaults,
        })
    }
}

/// Returns true if `path` is the home directory or looks like a profile dir
/// (`~/.plexi`, `~/.plexi-alpha`, `~/.plexi-pr-123`, etc.).
fn is_profile_dir(path: &Path) -> bool {
    let Some(home) = dirs::home_dir() else {
        return false;
    };
    if path == home {
        return true;
    }
    // Check if path is directly inside home and starts with `.plexi`
    if path.parent() == Some(&home) {
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name == ".plexi" || name.starts_with(".plexi-") {
                return true;
            }
        }
    }
    false
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_returns_none_without_dot_plexi() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Anchor::detect(dir.path()).is_none());
    }

    #[test]
    fn detect_returns_anchor_with_dot_plexi() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".plexi")).unwrap();
        let anchor = Anchor::detect(dir.path()).unwrap();
        assert_eq!(anchor.root, dir.path());
        assert!(anchor.context_defaults.is_none());
    }

    #[test]
    fn detect_reads_context_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let dot_plexi = dir.path().join(".plexi");
        std::fs::create_dir(&dot_plexi).unwrap();
        std::fs::write(
            dot_plexi.join("workspace.toml"),
            "id = \"abc\"\n\n[context]\nname = \"My Project\"\ndescription = \"The main dev workspace\"\n",
        ).unwrap();
        let anchor = Anchor::detect(dir.path()).unwrap();
        let defaults = anchor.context_defaults.unwrap();
        assert_eq!(defaults.name.as_deref(), Some("My Project"));
        assert_eq!(defaults.description.as_deref(), Some("The main dev workspace"));
    }

    #[test]
    fn detect_partial_context_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let dot_plexi = dir.path().join(".plexi");
        std::fs::create_dir(&dot_plexi).unwrap();
        std::fs::write(
            dot_plexi.join("workspace.toml"),
            "id = \"abc\"\n\n[context]\nname = \"Just Name\"\n",
        ).unwrap();
        let anchor = Anchor::detect(dir.path()).unwrap();
        let defaults = anchor.context_defaults.unwrap();
        assert_eq!(defaults.name.as_deref(), Some("Just Name"));
        assert!(defaults.description.is_none());
    }

    #[test]
    fn is_profile_dir_rejects_home() {
        if let Some(home) = dirs::home_dir() {
            assert!(is_profile_dir(&home));
        }
    }

    #[test]
    fn is_profile_dir_rejects_plexi_profiles() {
        if let Some(home) = dirs::home_dir() {
            assert!(is_profile_dir(&home.join(".plexi")));
            assert!(is_profile_dir(&home.join(".plexi-alpha")));
            assert!(is_profile_dir(&home.join(".plexi-pr-123")));
        }
    }

    #[test]
    fn is_profile_dir_allows_normal_dirs() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!is_profile_dir(dir.path()));
    }
}
