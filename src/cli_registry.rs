//! Tier-2 CLI descriptor registry (issue #321 substrate).
//!
//! Three tiers of UI-descriptor resolution for an arbitrary CLI:
//!
//! 1. **Tier 1** — the CLI itself emits a descriptor when invoked with
//!    `--plexi`. Owned by issue #188 / `plexi_descriptor`.
//! 2. **Tier 2** — *this module*. The Plexi binary ships a registry of
//!    hand-authored descriptors for popular CLIs that haven't (yet) opted in
//!    to the `--plexi` standard. Looked up by `(cli_name, version)`.
//! 3. **Tier 3** — fallback `--help` crawl. Owned by issue #78.
//!
//! The registry baked into the binary at build time lives at `registry/` in
//! the repo (see `RegistrySource::EmbeddedThenUserOverride::EMBEDDED`). A
//! per-channel user-side override directory shadows the embedded copy:
//! `~/.plexi-<channel>/registry/`. Override wins. This lets users hand-author
//! or patch a descriptor locally without rebuilding Plexi.
//!
//! Lookup contract: `lookup("gh", None)` → `gh/latest.json`.
//! `lookup("gh", Some("2.40.0"))` → `gh/2.40.0.json`.

use crate::plexi_descriptor::{self, DescriptorError, PlexiDescriptor};
use std::path::PathBuf;
#[cfg(test)]
use std::path::Path;

/// Compile-time-embedded registry rooted at `registry/` in the repo.
static EMBEDDED_REGISTRY: include_dir::Dir<'_> =
    include_dir::include_dir!("$CARGO_MANIFEST_DIR/registry");

/// Why a registry lookup failed. Distinguishes "no descriptor" (caller falls
/// through to Tier 3) from "descriptor exists but is broken" (loud error —
/// the registry is supposed to be curated).
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("registry has no descriptor for `{cli}` version `{version}`")]
    NotFound { cli: String, version: String },

    #[error("registry descriptor for `{cli}` at `{path}` is malformed: {source}")]
    Malformed {
        cli: String,
        path: String,
        #[source]
        source: DescriptorError,
    },

    #[error("registry descriptor for `{cli}` at `{path}` could not be read: {source}")]
    Io {
        cli: String,
        path: String,
        #[source]
        source: std::io::Error,
    },
}

/// Backing store for the registry. The `Embedded` and `Filesystem` variants
/// exist primarily so unit tests can supply a temp dir without monkey-patching
/// `dirs::home_dir()`.
pub trait RegistryBackend {
    /// Return the raw JSON text for `<cli>/<version>.json`, or `None` if the
    /// path doesn't exist in this backend.
    fn read(&self, cli: &str, version: &str) -> std::io::Result<Option<String>>;
}

/// The compile-time `include_dir!`-baked registry.
pub struct EmbeddedBackend;

impl RegistryBackend for EmbeddedBackend {
    fn read(&self, cli: &str, version: &str) -> std::io::Result<Option<String>> {
        let path = format!("{cli}/{version}.json");
        match EMBEDDED_REGISTRY.get_file(&path) {
            Some(file) => match file.contents_utf8() {
                Some(s) => Ok(Some(s.to_string())),
                None => Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("embedded registry file `{path}` is not valid UTF-8"),
                )),
            },
            None => Ok(None),
        }
    }
}

/// A user-side filesystem registry directory. Used both for the override at
/// `~/.plexi-<channel>/registry/` (which shadows `EmbeddedBackend`) and for
/// tests that need to stage a temp dir.
pub struct FilesystemBackend {
    pub root: PathBuf,
}

impl FilesystemBackend {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

impl RegistryBackend for FilesystemBackend {
    fn read(&self, cli: &str, version: &str) -> std::io::Result<Option<String>> {
        let path = self.root.join(cli).join(format!("{version}.json"));
        match std::fs::read_to_string(&path) {
            Ok(s) => Ok(Some(s)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }
}

/// Resolve the user-side override directory for the active channel.
/// Returns `None` if the home dir is unavailable.
pub fn user_override_dir() -> Option<PathBuf> {
    Some(crate::config::config_dir().join("registry"))
}

/// Look up a descriptor for `(cli_name, version)`. When `version` is `None`,
/// the lookup uses `latest.json`.
///
/// Resolution order: user-override → embedded. The first backend that has a
/// matching file wins, even if it's malformed (we surface the parse error
/// rather than silently falling through — a malformed override is a bug the
/// user wants to know about).
pub fn lookup(cli_name: &str, version: Option<&str>) -> Result<PlexiDescriptor, RegistryError> {
    let version_key = version.unwrap_or("latest");

    // 1. User override.
    if let Some(override_root) = user_override_dir() {
        let backend = FilesystemBackend::new(override_root);
        match read_and_parse(&backend, cli_name, version_key) {
            Ok(Some(d)) => return Ok(d),
            Ok(None) => { /* fall through */ }
            Err(e) => return Err(e),
        }
    }

    // 2. Embedded.
    match read_and_parse(&EmbeddedBackend, cli_name, version_key) {
        Ok(Some(d)) => Ok(d),
        Ok(None) => Err(RegistryError::NotFound {
            cli: cli_name.to_string(),
            version: version_key.to_string(),
        }),
        Err(e) => Err(e),
    }
}

/// Lookup against an explicit list of backends (in priority order). The
/// public `lookup()` is a thin wrapper over this; tests can call this
/// directly to inject a temp filesystem backend.
#[cfg(test)]
pub fn lookup_with_backends(
    backends: &[&dyn RegistryBackend],
    cli_name: &str,
    version: Option<&str>,
) -> Result<PlexiDescriptor, RegistryError> {
    let version_key = version.unwrap_or("latest");
    for backend in backends {
        match read_and_parse(*backend, cli_name, version_key) {
            Ok(Some(d)) => return Ok(d),
            Ok(None) => continue,
            Err(e) => return Err(e),
        }
    }
    Err(RegistryError::NotFound {
        cli: cli_name.to_string(),
        version: version_key.to_string(),
    })
}

fn read_and_parse(
    backend: &dyn RegistryBackend,
    cli: &str,
    version: &str,
) -> Result<Option<PlexiDescriptor>, RegistryError> {
    let path_repr = format!("{cli}/{version}.json");
    let raw = backend.read(cli, version).map_err(|e| RegistryError::Io {
        cli: cli.to_string(),
        path: path_repr.clone(),
        source: e,
    })?;
    match raw {
        None => Ok(None),
        Some(json) => match plexi_descriptor::parse(&json) {
            Ok(d) => Ok(Some(d)),
            Err(e) => Err(RegistryError::Malformed {
                cli: cli.to_string(),
                path: path_repr,
                source: e,
            }),
        },
    }
}

/// Iterate all CLI directories present in either the embedded or override
/// backend. Used by `plexi registry watch` to walk the registered CLI set.
pub fn list_clis() -> Vec<String> {
    let mut names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for entry in EMBEDDED_REGISTRY.entries() {
        if let Some(d) = entry.as_dir() {
            if let Some(name) = d.path().file_name().and_then(|s| s.to_str()) {
                names.insert(name.to_string());
            }
        }
    }
    if let Some(override_root) = user_override_dir() {
        if let Ok(read) = std::fs::read_dir(&override_root) {
            for entry in read.flatten() {
                if entry.path().is_dir() {
                    if let Some(name) = entry.file_name().to_str() {
                        names.insert(name.to_string());
                    }
                }
            }
        }
    }
    names.into_iter().collect()
}

/// Build-time-style sanity check that every embedded descriptor parses. Run
/// from a unit test (cheap — descriptors are small) so a hand-edit that
/// breaks the schema fails CI rather than blowing up at runtime.
#[cfg(test)]
fn parse_all_embedded() -> Result<usize, RegistryError> {
    let mut count = 0;
    for entry in EMBEDDED_REGISTRY.entries() {
        if let Some(dir) = entry.as_dir() {
            let dir_name = dir.path().file_name().and_then(|s| s.to_str()).unwrap_or("?");
            if dir_name == "mcp" {
                continue;
            }
            let cli = dir_name.to_string();
            for f in dir.files() {
                let path_repr = f.path().to_string_lossy().into_owned();
                let json = f.contents_utf8().ok_or_else(|| RegistryError::Io {
                    cli: cli.clone(),
                    path: path_repr.clone(),
                    source: std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "non-utf8 registry file",
                    ),
                })?;
                plexi_descriptor::parse(json).map_err(|e| RegistryError::Malformed {
                    cli: cli.clone(),
                    path: path_repr,
                    source: e,
                })?;
                count += 1;
            }
        }
    }
    Ok(count)
}

#[cfg(test)]
fn write_descriptor(root: &Path, cli: &str, version: &str, json: &str) {
    let dir = root.join(cli);
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(dir.join(format!("{version}.json")), json).expect("write descriptor");
}

#[cfg(test)]
const FAKE_DESCRIPTOR: &str = r#"{
    "plexi_version": "0.1",
    "name": "fake",
    "version": "9.9.9",
    "commands": []
}"#;

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn lookup_returns_descriptor_for_known_cli() {
        // gh ships seeded into the embedded registry. The public `lookup` is
        // OK to call here — no override dir is created in the test sandbox.
        let d = lookup("gh", None).expect("gh latest in embedded registry");
        assert_eq!(d.name, "gh");
        assert!(!d.commands.is_empty());
    }

    #[test]
    fn lookup_returns_none_for_unknown_cli() {
        let err = lookup("nonexistent-cli-zzz", None).expect_err("unknown CLI must NotFound");
        match err {
            RegistryError::NotFound { cli, .. } => assert_eq!(cli, "nonexistent-cli-zzz"),
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn lookup_with_explicit_version_picks_that_file() {
        let tmp = TempDir::new().unwrap();
        write_descriptor(tmp.path(), "fake", "9.9.9", FAKE_DESCRIPTOR);
        let backend = FilesystemBackend::new(tmp.path().to_path_buf());
        let d = lookup_with_backends(&[&backend], "fake", Some("9.9.9"))
            .expect("explicit version found");
        assert_eq!(d.version, "9.9.9");
    }

    #[test]
    fn lookup_without_version_picks_latest() {
        let tmp = TempDir::new().unwrap();
        write_descriptor(tmp.path(), "fake", "latest", FAKE_DESCRIPTOR);
        let backend = FilesystemBackend::new(tmp.path().to_path_buf());
        let d = lookup_with_backends(&[&backend], "fake", None)
            .expect("default version maps to latest.json");
        assert_eq!(d.version, "9.9.9");
    }

    #[test]
    fn user_override_dir_shadows_embedded_registry() {
        // Stage a `gh/latest.json` in a temp override dir whose `name` field
        // differs from the embedded `gh` descriptor. Override must win.
        let tmp = TempDir::new().unwrap();
        let overridden_json = r#"{
            "plexi_version": "0.1",
            "name": "gh-overridden",
            "version": "0.0.0",
            "commands": []
        }"#;
        write_descriptor(tmp.path(), "gh", "latest", overridden_json);
        let override_backend = FilesystemBackend::new(tmp.path().to_path_buf());
        let d = lookup_with_backends(&[&override_backend, &EmbeddedBackend], "gh", None)
            .expect("override should resolve");
        assert_eq!(d.name, "gh-overridden");
    }

    #[test]
    fn malformed_descriptor_in_registry_errors_clearly() {
        let tmp = TempDir::new().unwrap();
        write_descriptor(
            tmp.path(),
            "fake",
            "latest",
            r#"{ "this": "is not a descriptor" }"#,
        );
        let backend = FilesystemBackend::new(tmp.path().to_path_buf());
        let err = lookup_with_backends(&[&backend], "fake", None)
            .expect_err("malformed JSON must error");
        match err {
            RegistryError::Malformed { cli, path, .. } => {
                assert_eq!(cli, "fake");
                assert!(path.contains("fake/latest.json"), "path: {path}");
            }
            other => panic!("expected Malformed, got {other:?}"),
        }
    }

    #[test]
    fn embedded_registry_round_trips_through_parser() {
        // Guard against hand-edit drift: every shipped descriptor must parse.
        let n = parse_all_embedded().expect("all embedded descriptors must parse");
        assert!(n >= 6, "expected ≥6 descriptors (3 CLIs × 2 files), got {n}");
    }

    #[test]
    fn list_clis_returns_seeded_three() {
        let names = list_clis();
        for cli in &["gh", "cargo", "npm"] {
            assert!(names.iter().any(|n| n == cli), "expected {cli} in {names:?}");
        }
    }
}
