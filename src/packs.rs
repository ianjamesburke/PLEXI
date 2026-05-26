//! Pack file format — `pack.toml` (#308 Phase 2).
//!
//! A pack is a requirements.txt-style list of apps to install. Three sources
//! eat the same shape:
//!   - `<workspace>/.plexi/pack.toml`           (project-local, Phase 3+)
//!   - `~/.plexi-<channel>/packs/core.toml`     (channel-level overrides)
//!   - the binary's compile-time-bundled core pack (`packs/core.toml` baked in)
//!
//! ```toml
//! schema_version = 1
//!
//! [[app]]
//! id      = "stand-up-reminder"
//! source  = "github:plexi-apps/stand-up-reminder"
//! version = "v1.2.0"
//! # Optional: checksum = "sha256:..."
//! ```
//!
//! Source spec parser supports:
//! - `github:owner/repo`        → `https://github.com/owner/repo.git`
//! - `git+https://...`          → literal URL (`git+` stripped)
//! - `git+ssh://...` / `git+http://...`
//! - `local:<app-name>`         → bundled-app seed (no clone, host copies
//!                                from `apps/<name>/` baked into the binary)
//! - anything else → error (no silent fallthrough).
//!
//! The current pack schema version constant is [`PACK_SCHEMA_VERSION`].

use serde::Deserialize;
use std::path::Path;

/// The current pack schema version. See [`Pack::schema_version`].
pub const PACK_SCHEMA_VERSION: u32 = 1;

/// Parsed `pack.toml`.
#[derive(Deserialize, Debug, Clone)]
pub struct Pack {
    /// Required; rejected loud if greater than [`PACK_SCHEMA_VERSION`].
    pub schema_version: u32,
    /// Seeding policy. `"always"` = re-seed on every install; `"once"` = seed
    /// only on first launch (empty apps dir). Defaults to `"once"` if absent.
    #[serde(default)]
    pub reseed: Option<String>,
    /// One entry per app to install. `[[app]]` table-array.
    #[serde(default, rename = "app")]
    pub apps: Vec<PackApp>,
}

/// A single `[[app]]` entry within a pack.
#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct PackApp {
    /// Manifest id this entry should install. Required so the resolver can
    /// reject collisions and key the install dir without cloning first.
    pub id: String,
    /// Source spec — see module docs for the supported schemes.
    pub source: String,
    /// Git ref (tag / branch / commit) for clone-based sources, or any
    /// version label for `local:` sources (informational).
    pub version: String,
    /// Optional. Format: `sha256:<hex>`. Verified post-clone if set.
    #[serde(default)]
    pub checksum: Option<String>,
}

impl Pack {
    /// Parse a pack from its TOML text. Refuses pack files whose
    /// `schema_version` is newer than this build supports — same loud-fail
    /// pattern as `AppManifest`.
    pub fn from_toml_str(text: &str) -> Result<Self, String> {
        let pack: Pack = toml::from_str(text).map_err(|e| format!("invalid pack.toml: {e}"))?;
        if pack.schema_version > PACK_SCHEMA_VERSION {
            return Err(format!(
                "pack schema_version = {} is newer than this Plexi build supports (max {}); \
                 update Plexi to apply this pack",
                pack.schema_version, PACK_SCHEMA_VERSION
            ));
        }
        Ok(pack)
    }

    /// Read + parse a pack from disk.
    pub fn from_path(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read pack {}: {e}", path.display()))?;
        Self::from_toml_str(&text)
    }
}

/// What a `source = "..."` string in a pack entry resolves to. Distinguishes
/// "go fetch this from the network with git" from "copy this bundled
/// example out of the binary".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceSpec {
    /// Standard git clone target — pass directly to `git clone <url>`.
    Git(String),
    /// Compile-time-bundled app, identified by its directory name in
    /// `apps/`. The installer copies the directory tree to the apps dir.
    Local(String),
}

/// Parse a pack `source` field into a [`SourceSpec`]. Returns an error for
/// unknown schemes — there is no silent fallthrough.
pub fn parse_source_spec(source: &str) -> Result<SourceSpec, String> {
    if let Some(rest) = source.strip_prefix("github:") {
        // github:owner/repo
        let trimmed = rest.trim_end_matches(".git");
        if !trimmed.contains('/') || trimmed.starts_with('/') || trimmed.ends_with('/') {
            return Err(format!(
                "github: source must be 'github:owner/repo'; got '{source}'"
            ));
        }
        return Ok(SourceSpec::Git(format!("https://github.com/{trimmed}.git")));
    }
    if let Some(rest) = source.strip_prefix("git+") {
        // git+https://..., git+http://..., git+ssh://..., git+file://...
        // (file:// is intentionally allowed — useful for local-repo testing
        // and for offline environments. Trust model is the same as for
        // remote URLs: the user explicitly typed it.)
        if rest.starts_with("https://")
            || rest.starts_with("http://")
            || rest.starts_with("ssh://")
            || rest.starts_with("file://")
        {
            return Ok(SourceSpec::Git(rest.to_string()));
        }
        return Err(format!(
            "git+ source must be git+https://..., git+http://..., git+ssh://..., or git+file://...; got '{source}'"
        ));
    }
    if let Some(rest) = source.strip_prefix("local:") {
        if rest.is_empty() || rest.contains(['/', '\\', '.', '\0']) {
            return Err(format!(
                "local: source must be a bare example name (no '/', '\\', '.'); got '{source}'"
            ));
        }
        return Ok(SourceSpec::Local(rest.to_string()));
    }
    // No silent fallthrough.
    Err(format!(
        "unknown source scheme '{source}'; supported: github:owner/repo, \
         git+https://..., git+ssh://..., local:<example-name>"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_pack() {
        let raw = r#"
schema_version = 1

[[app]]
id = "stand-up-reminder"
source = "github:plexi-apps/stand-up-reminder"
version = "v1.2.0"
"#;
        let pack = Pack::from_toml_str(raw).expect("pack should parse");
        assert_eq!(pack.schema_version, 1);
        assert_eq!(pack.apps.len(), 1);
        assert_eq!(pack.apps[0].id, "stand-up-reminder");
        assert_eq!(pack.apps[0].source, "github:plexi-apps/stand-up-reminder");
        assert_eq!(pack.apps[0].version, "v1.2.0");
        assert!(pack.apps[0].checksum.is_none());
    }

    #[test]
    fn parse_pack_with_optional_checksum() {
        let raw = r#"
schema_version = 1

[[app]]
id = "secrets-app"
source = "git+https://example.com/secrets.git"
version = "main"
checksum = "sha256:deadbeef"
"#;
        let pack = Pack::from_toml_str(raw).expect("pack should parse");
        assert_eq!(
            pack.apps[0].checksum.as_deref(),
            Some("sha256:deadbeef"),
            "optional checksum field must round-trip"
        );
    }

    #[test]
    fn parse_pack_refuses_future_schema_version() {
        let raw = format!(
            "schema_version = {}\n\n[[app]]\nid=\"x\"\nsource=\"github:a/b\"\nversion=\"v1\"\n",
            PACK_SCHEMA_VERSION + 1
        );
        let err = Pack::from_toml_str(&raw).expect_err("future schema must refuse");
        assert!(
            err.contains("newer than this Plexi build"),
            "expected schema-too-new error, got: {err}"
        );
    }

    #[test]
    fn source_spec_github_resolves_to_url() {
        let spec = parse_source_spec("github:plexi-apps/stand-up-reminder")
            .expect("github source should parse");
        assert_eq!(
            spec,
            SourceSpec::Git("https://github.com/plexi-apps/stand-up-reminder.git".to_string())
        );
    }

    #[test]
    fn source_spec_github_with_dot_git_suffix_normalizes() {
        let spec = parse_source_spec("github:owner/repo.git").expect("trailing .git ok");
        assert_eq!(
            spec,
            SourceSpec::Git("https://github.com/owner/repo.git".to_string())
        );
    }

    #[test]
    fn source_spec_git_https_passes_through() {
        let spec = parse_source_spec("git+https://example.com/some/repo.git")
            .expect("git+https should parse");
        assert_eq!(
            spec,
            SourceSpec::Git("https://example.com/some/repo.git".to_string())
        );
    }

    #[test]
    fn source_spec_local_resolves() {
        let spec = parse_source_spec("local:secrets-app").expect("local source should parse");
        assert_eq!(spec, SourceSpec::Local("secrets-app".to_string()));
    }

    #[test]
    fn source_spec_unknown_scheme_errors() {
        let err = parse_source_spec("ftp://example.com/repo")
            .expect_err("unknown scheme must error");
        assert!(
            err.contains("unknown source scheme"),
            "expected unknown-scheme error, got: {err}"
        );
    }

    #[test]
    fn source_spec_malformed_github_errors() {
        let err = parse_source_spec("github:no-slash").expect_err("malformed github must error");
        assert!(
            err.contains("must be 'github:owner/repo'"),
            "got: {err}"
        );
    }

    #[test]
    fn source_spec_local_with_slash_errors() {
        let err = parse_source_spec("local:foo/bar").expect_err("path-traversal must error");
        assert!(err.contains("bare example name"), "got: {err}");
    }
}
