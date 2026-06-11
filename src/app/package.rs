//! Single-app package artifact — `.plexipkg` (stint 0015).
//!
//! A package is a gzipped tar with the app's files at the archive root plus a
//! generated `PACKAGE.toml` describing the contents:
//!
//! ```toml
//! schema_version = 1
//!
//! [package]
//! id      = "my-app"
//! version = "0.1.0"
//! runtime = "python"   # "python" if entry ends in .py, else "native"
//!
//! capabilities = ["ai.query"]
//!
//! [[files]]
//! path   = "manifest.toml"
//! sha256 = "…"
//! size   = 123
//! ```
//!
//! NOT a "pack" (`src/app/packs.rs`) — a pack is a *collection* manifest that
//! lists multiple apps to install; a package is one app's distributable
//! artifact.
//!
//! The validator is fail-closed: every check is a hard [`PackageError`], never
//! a warning. Archive extraction is streaming and path-checked — absolute
//! paths, `..` components, and symlinks (in a source dir or as archive
//! entries) are all rejected before any byte is written.

use crate::app::permissions::Capability;
use crate::app::registry::{AppManifest, MANIFEST_SCHEMA_VERSION};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::convert::TryFrom;
use std::fs;
use std::path::{Component, Path, PathBuf};

/// Current `PACKAGE.toml` schema version.
pub const PACKAGE_SCHEMA_VERSION: u32 = 1;

/// File extension for single-app package artifacts.
pub const PACKAGE_EXTENSION: &str = "plexipkg";

/// Name of the generated package descriptor inside the archive.
pub const PACKAGE_TOML: &str = "PACKAGE.toml";

// ── Errors ────────────────────────────────────────────────────────────────────

/// Fail-closed package validation/build errors. Every variant names what
/// failed, what was expected, and the offending path or value.
#[derive(Debug, thiserror::Error)]
pub enum PackageError {
    #[error("io error during {action} at {path}: {source}")]
    Io {
        action: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("not a directory: {0} — expected an app directory containing manifest.toml")]
    NotADirectory(PathBuf),
    #[error("manifest.toml not found in {0} — every package requires one at the app root")]
    ManifestMissing(PathBuf),
    #[error("manifest.toml at {path} failed to parse: {source}")]
    ManifestParse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("manifest schema_version {found} is newer than supported (max {max}) in {path}")]
    UnsupportedSchemaVersion {
        found: u32,
        max: u32,
        path: PathBuf,
    },
    #[error("manifest field [app].{field} is missing or empty in {path}")]
    MissingField { field: &'static str, path: PathBuf },
    #[error("entry file '{entry}' declared in manifest.toml not found at {path}")]
    EntryMissing { entry: String, path: PathBuf },
    #[error("symlink not allowed in a package: {0} — packages must contain only regular files")]
    SymlinkInDir(PathBuf),
    #[error("archive entry '{0}' is a {1} — only regular files and directories are allowed")]
    ForbiddenEntryType(String, &'static str),
    #[error("unsafe archive path '{0}' — absolute paths and '..' components are rejected")]
    UnsafePath(String),
    #[error(
        "unknown capability '{value}' in {path} — valid capabilities: {valid}",
        valid = Capability::all_str_values().join(", ")
    )]
    UnknownCapability { value: String, path: PathBuf },
    #[error("sha256 mismatch for '{path}': PACKAGE.toml says {expected}, actual content is {actual}")]
    ChecksumMismatch {
        path: String,
        expected: String,
        actual: String,
    },
    #[error("size mismatch for '{path}': PACKAGE.toml says {expected} bytes, actual is {actual}")]
    SizeMismatch {
        path: String,
        expected: u64,
        actual: u64,
    },
    #[error("PACKAGE.toml missing from archive {0} — not a valid .plexipkg")]
    PackageTomlMissing(PathBuf),
    #[error("PACKAGE.toml in {path} failed to parse: {source}")]
    PackageTomlParse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("PACKAGE.toml schema_version {found} is newer than supported (max {max})")]
    UnsupportedPackageSchema { found: u32, max: u32 },
    #[error("PACKAGE.toml id '{package}' disagrees with manifest.toml id '{manifest}'")]
    IdMismatch { package: String, manifest: String },
    #[error("PACKAGE.toml version '{package}' disagrees with manifest.toml version '{manifest}'")]
    VersionMismatch { package: String, manifest: String },
    #[error("unsupported runtime '{0}' in PACKAGE.toml — expected \"python\" or \"native\"")]
    UnsupportedRuntime(String),
    #[error("file '{0}' listed in PACKAGE.toml is missing from the archive")]
    ListedFileMissing(String),
    #[error("file '{0}' present in the archive but not listed in PACKAGE.toml")]
    UnlistedFile(String),
    #[error("package file {0} does not have the .{PACKAGE_EXTENSION} extension")]
    WrongExtension(PathBuf),
}

fn io_err(action: &'static str, path: &Path, source: std::io::Error) -> PackageError {
    PackageError::Io {
        action,
        path: path.to_path_buf(),
        source,
    }
}

// ── Runtime ───────────────────────────────────────────────────────────────────

/// Package runtime — derived from the manifest entry point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageRuntime {
    /// Entry ends in `.py` — launched via python3.
    Python,
    /// Anything else — a native executable.
    Native,
}

impl PackageRuntime {
    pub fn from_entry(entry: &str) -> Self {
        if entry.ends_with(".py") {
            Self::Python
        } else {
            Self::Native
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Python => "python",
            Self::Native => "native",
        }
    }

    fn parse(s: &str) -> Result<Self, PackageError> {
        match s {
            "python" => Ok(Self::Python),
            "native" => Ok(Self::Native),
            other => Err(PackageError::UnsupportedRuntime(other.to_string())),
        }
    }
}

// ── PACKAGE.toml schema ───────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
struct PackageToml {
    schema_version: u32,
    package: PackageSection,
    capabilities: Vec<String>,
    files: Vec<PackageFile>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PackageSection {
    id: String,
    version: String,
    runtime: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct PackageFile {
    path: String,
    sha256: String,
    size: u64,
}

// ── Report ────────────────────────────────────────────────────────────────────

/// Result of a successful validation — the parsed identity of the package.
/// Reused by `plexi app install <file.plexipkg>` and the future inspect path
/// (stint 0016).
#[derive(Debug, Clone)]
pub struct PackageReport {
    pub id: String,
    pub name: String,
    pub version: String,
    pub runtime: PackageRuntime,
    pub entry: String,
    pub capabilities: Vec<Capability>,
    pub file_count: usize,
    pub total_size: u64,
}

// ── Directory validation ──────────────────────────────────────────────────────

/// Validate an app directory as packageable. Fail-closed: any structural
/// problem is a hard error. A root-level `PACKAGE.toml` (left over from a
/// previous extraction) is ignored by the file walk — it is regenerated at
/// build time and verified at package-validate time.
pub fn validate_dir(app_dir: &Path) -> Result<PackageReport, PackageError> {
    let report = validate_dir_inner(app_dir)?;
    log::info!(
        "package: validate_dir ok id={} version={} runtime={} files={} bytes={} dir={}",
        report.id,
        report.version,
        report.runtime.as_str(),
        report.file_count,
        report.total_size,
        app_dir.display()
    );
    Ok(report)
}

fn validate_dir_inner(app_dir: &Path) -> Result<PackageReport, PackageError> {
    if !app_dir.is_dir() {
        return Err(PackageError::NotADirectory(app_dir.to_path_buf()));
    }

    let manifest = read_manifest(app_dir)?;
    let entry = manifest.app.entry.clone();

    // Entry must exist and must not be a symlink.
    let entry_path = app_dir.join(&entry);
    let entry_meta = fs::symlink_metadata(&entry_path).map_err(|_| PackageError::EntryMissing {
        entry: entry.clone(),
        path: entry_path.clone(),
    })?;
    if entry_meta.file_type().is_symlink() {
        return Err(PackageError::SymlinkInDir(entry_path));
    }

    let capabilities = parse_capabilities(
        &manifest.app.capabilities.capabilities,
        &app_dir.join("manifest.toml"),
    )?;

    let files = collect_files(app_dir)?;
    let total_size = files.iter().map(|(_, size)| size).sum();

    Ok(PackageReport {
        id: manifest.app.id,
        name: manifest.app.name,
        version: manifest.app.version,
        runtime: PackageRuntime::from_entry(&entry),
        entry,
        capabilities,
        file_count: files.len(),
        total_size,
    })
}

/// Read + structurally validate `manifest.toml` from an app dir.
fn read_manifest(app_dir: &Path) -> Result<AppManifest, PackageError> {
    let manifest_path = app_dir.join("manifest.toml");
    if !manifest_path.exists() {
        return Err(PackageError::ManifestMissing(app_dir.to_path_buf()));
    }
    let raw =
        fs::read_to_string(&manifest_path).map_err(|e| io_err("read", &manifest_path, e))?;
    let manifest: AppManifest =
        toml::from_str(&raw).map_err(|e| PackageError::ManifestParse {
            path: manifest_path.clone(),
            source: e,
        })?;

    if manifest.schema_version > MANIFEST_SCHEMA_VERSION {
        return Err(PackageError::UnsupportedSchemaVersion {
            found: manifest.schema_version,
            max: MANIFEST_SCHEMA_VERSION,
            path: manifest_path,
        });
    }
    // serde enforces id/name/entry presence; version is serde(default) so an
    // empty version must be rejected here — the artifact name needs it.
    for (field, value) in [
        ("id", &manifest.app.id),
        ("name", &manifest.app.name),
        ("entry", &manifest.app.entry),
        ("version", &manifest.app.version),
    ] {
        if value.is_empty() {
            return Err(PackageError::MissingField {
                field,
                path: manifest_path,
            });
        }
    }
    Ok(manifest)
}

/// Parse capability strings against the real enum. Unknown → hard error.
fn parse_capabilities(
    strings: &[String],
    source_path: &Path,
) -> Result<Vec<Capability>, PackageError> {
    strings
        .iter()
        .map(|s| {
            Capability::try_from(s.as_str()).map_err(|_| PackageError::UnknownCapability {
                value: s.clone(),
                path: source_path.to_path_buf(),
            })
        })
        .collect()
}

/// Recursively collect every regular file under `root` as
/// `(relative path, size)`, sorted by path for deterministic archives.
/// Rejects any symlink anywhere in the tree. Skips a root-level PACKAGE.toml.
fn collect_files(root: &Path) -> Result<Vec<(PathBuf, u64)>, PackageError> {
    let mut files = Vec::new();
    collect_files_rec(root, Path::new(""), &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_files_rec(
    root: &Path,
    rel: &Path,
    out: &mut Vec<(PathBuf, u64)>,
) -> Result<(), PackageError> {
    let dir = root.join(rel);
    let entries = fs::read_dir(&dir).map_err(|e| io_err("read_dir", &dir, e))?;
    for entry in entries {
        let entry = entry.map_err(|e| io_err("read_dir", &dir, e))?;
        let path = entry.path();
        let rel_path = rel.join(entry.file_name());
        let meta = fs::symlink_metadata(&path).map_err(|e| io_err("stat", &path, e))?;
        if meta.file_type().is_symlink() {
            return Err(PackageError::SymlinkInDir(path));
        }
        if meta.is_dir() {
            collect_files_rec(root, &rel_path, out)?;
        } else {
            // Root-level PACKAGE.toml is the descriptor, never package content:
            // regenerated at build, verified separately at validate.
            if rel_path == Path::new(PACKAGE_TOML) {
                continue;
            }
            out.push((rel_path, meta.len()));
        }
    }
    Ok(())
}

// ── Build ─────────────────────────────────────────────────────────────────────

/// Build a `.plexipkg` artifact from an app directory.
///
/// Validates the directory first (fail-closed), then writes
/// `<id>-<version>.plexipkg` into the current directory, or to `out` when
/// given (used verbatim as the output file path).
pub fn build_package(app_dir: &Path, out: Option<&Path>) -> Result<PathBuf, PackageError> {
    let report = validate_dir(app_dir)?;
    if app_dir.join(PACKAGE_TOML).exists() {
        log::warn!(
            "package: ignoring stale root-level {PACKAGE_TOML} in {} — it is regenerated at build",
            app_dir.display()
        );
    }
    let files = collect_files(app_dir)?;

    // Hash every file.
    let mut package_files = Vec::with_capacity(files.len());
    for (rel, size) in &files {
        let abs = app_dir.join(rel);
        let sha256 = sha256_file(&abs)?;
        package_files.push(PackageFile {
            path: rel.to_string_lossy().into_owned(),
            sha256,
            size: *size,
        });
    }

    let descriptor = PackageToml {
        schema_version: PACKAGE_SCHEMA_VERSION,
        package: PackageSection {
            id: report.id.clone(),
            version: report.version.clone(),
            runtime: report.runtime.as_str().to_string(),
        },
        capabilities: report.capabilities.iter().map(|c| c.as_str().to_string()).collect(),
        files: package_files,
    };
    let descriptor_toml = toml::to_string_pretty(&descriptor).map_err(|e| {
        // Serialization of an in-memory struct failing is unexpected; surface
        // it as an io-class error on the output path.
        io_err(
            "serialize PACKAGE.toml",
            app_dir,
            std::io::Error::other(e),
        )
    })?;

    let out_path = match out {
        Some(p) => p.to_path_buf(),
        None => PathBuf::from(format!(
            "{}-{}.{PACKAGE_EXTENSION}",
            report.id, report.version
        )),
    };

    let out_file = fs::File::create(&out_path).map_err(|e| io_err("create", &out_path, e))?;
    let gz = flate2::write::GzEncoder::new(out_file, flate2::Compression::default());
    let mut builder = tar::Builder::new(gz);

    // PACKAGE.toml first, then app files at the archive root.
    let descriptor_bytes = descriptor_toml.as_bytes();
    let mut header = tar::Header::new_gnu();
    header.set_size(descriptor_bytes.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    builder
        .append_data(&mut header, PACKAGE_TOML, descriptor_bytes)
        .map_err(|e| io_err("append PACKAGE.toml", &out_path, e))?;

    for (rel, _) in &files {
        let abs = app_dir.join(rel);
        builder
            .append_path_with_name(&abs, rel)
            .map_err(|e| io_err("append file", &abs, e))?;
    }

    let gz = builder
        .into_inner()
        .map_err(|e| io_err("finish tar", &out_path, e))?;
    gz.finish().map_err(|e| io_err("finish gzip", &out_path, e))?;

    log::info!(
        "package: built id={} version={} files={} → {}",
        report.id,
        report.version,
        files.len(),
        out_path.display()
    );
    Ok(out_path)
}

fn sha256_file(path: &Path) -> Result<String, PackageError> {
    let mut file = fs::File::open(path).map_err(|e| io_err("open", path, e))?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher).map_err(|e| io_err("hash", path, e))?;
    Ok(hex_string(&hasher.finalize()))
}

fn hex_string(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

// ── Archive extraction (safe, streaming) ──────────────────────────────────────

/// Returns the path-safe relative form of an archive entry path, or an error
/// if the path is absolute or contains `..` / non-normal components.
fn safe_entry_path(raw: &Path) -> Result<PathBuf, PackageError> {
    let display = raw.display().to_string();
    if raw.is_absolute() {
        return Err(PackageError::UnsafePath(display));
    }
    let mut safe = PathBuf::new();
    for component in raw.components() {
        match component {
            Component::Normal(c) => safe.push(c),
            Component::CurDir => {}
            _ => return Err(PackageError::UnsafePath(display)),
        }
    }
    if safe.as_os_str().is_empty() {
        return Err(PackageError::UnsafePath(display));
    }
    Ok(safe)
}

/// Safely extract a `.plexipkg` archive into `dest` (created if absent).
/// Streams entries; each path is checked before any write; only regular files
/// and directories are accepted — symlinks, hardlinks, and devices are
/// rejected. Does NOT validate content hashes — callers wanting the full
/// fail-closed gate use [`validate_package`].
pub fn extract_package(file: &Path, dest: &Path) -> Result<(), PackageError> {
    let f = fs::File::open(file).map_err(|e| io_err("open", file, e))?;
    let gz = flate2::read::GzDecoder::new(f);
    let mut archive = tar::Archive::new(gz);

    fs::create_dir_all(dest).map_err(|e| io_err("create_dir_all", dest, e))?;

    let entries = archive.entries().map_err(|e| io_err("read archive", file, e))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| io_err("read archive entry", file, e))?;
        let raw_path = entry
            .path()
            .map_err(|e| io_err("read entry path", file, e))?
            .into_owned();
        let rel = safe_entry_path(&raw_path)?;
        let entry_type = entry.header().entry_type();
        match entry_type {
            tar::EntryType::Directory => {
                let dir = dest.join(&rel);
                fs::create_dir_all(&dir).map_err(|e| io_err("create_dir_all", &dir, e))?;
            }
            tar::EntryType::Regular => {
                let target = dest.join(&rel);
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent).map_err(|e| io_err("create_dir_all", parent, e))?;
                }
                let mut out =
                    fs::File::create(&target).map_err(|e| io_err("create", &target, e))?;
                std::io::copy(&mut entry, &mut out).map_err(|e| io_err("write", &target, e))?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if let Ok(mode) = entry.header().mode() {
                        // Preserve the executable bit; never honor setuid/sticky bits.
                        let mode = mode & 0o777;
                        let _ = fs::set_permissions(&target, fs::Permissions::from_mode(mode));
                    }
                }
            }
            other => {
                let kind: &'static str = match other {
                    tar::EntryType::Symlink => "symlink",
                    tar::EntryType::Link => "hardlink",
                    _ => "non-regular entry",
                };
                return Err(PackageError::ForbiddenEntryType(
                    rel.to_string_lossy().into_owned(),
                    kind,
                ));
            }
        }
    }
    Ok(())
}

/// Removes a temp extraction dir on drop — cleanup happens on both the success
/// and every error path.
struct TempDirGuard(PathBuf);

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        if let Err(e) = fs::remove_dir_all(&self.0) {
            if self.0.exists() {
                log::warn!(
                    "package: could not clean up temp dir {}: {e}",
                    self.0.display()
                );
            }
        }
    }
}

/// Create a unique temp dir for package extraction.
fn unique_temp_dir() -> Result<TempDirGuard, PackageError> {
    let dir = std::env::temp_dir().join(format!("plexipkg-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&dir).map_err(|e| io_err("create_dir_all", &dir, e))?;
    Ok(TempDirGuard(dir))
}

// ── Package validation ────────────────────────────────────────────────────────

/// Validate a `.plexipkg` file end-to-end (fail-closed):
///
/// 1. safe streaming extraction into a unique temp dir (path + entry-type checks)
/// 2. `PACKAGE.toml` present, parseable, supported schema, supported runtime
/// 3. every listed file present with matching sha256 + size; no unlisted files
/// 4. the extracted tree passes [`validate_dir`] (manifest, entry, symlinks,
///    capability strings)
/// 5. `PACKAGE.toml` id/version/runtime agree with `manifest.toml`
pub fn validate_package(file: &Path) -> Result<PackageReport, PackageError> {
    let result = validate_package_inner(file);
    match &result {
        Ok(report) => log::info!(
            "package: validate_package ok id={} version={} files={} file={}",
            report.id,
            report.version,
            report.file_count,
            file.display()
        ),
        Err(e) => log::warn!("package: validate_package failed for {}: {e}", file.display()),
    }
    result
}

fn validate_package_inner(file: &Path) -> Result<PackageReport, PackageError> {
    if file.extension().and_then(|e| e.to_str()) != Some(PACKAGE_EXTENSION) {
        return Err(PackageError::WrongExtension(file.to_path_buf()));
    }

    let tmp = unique_temp_dir()?;
    let root = tmp.0.clone();
    extract_package(file, &root)?;

    // PACKAGE.toml
    let descriptor_path = root.join(PACKAGE_TOML);
    if !descriptor_path.exists() {
        return Err(PackageError::PackageTomlMissing(file.to_path_buf()));
    }
    let raw = fs::read_to_string(&descriptor_path)
        .map_err(|e| io_err("read", &descriptor_path, e))?;
    let descriptor: PackageToml =
        toml::from_str(&raw).map_err(|e| PackageError::PackageTomlParse {
            path: file.to_path_buf(),
            source: e,
        })?;
    if descriptor.schema_version > PACKAGE_SCHEMA_VERSION {
        return Err(PackageError::UnsupportedPackageSchema {
            found: descriptor.schema_version,
            max: PACKAGE_SCHEMA_VERSION,
        });
    }
    let declared_runtime = PackageRuntime::parse(&descriptor.package.runtime)?;
    // Capability strings in PACKAGE.toml must also resolve against the enum.
    parse_capabilities(&descriptor.capabilities, file)?;

    // Verify every listed file's checksum + size, and that nothing extra is present.
    let on_disk = collect_files(&root)?; // also rejects symlinks defensively
    let mut listed: std::collections::BTreeMap<&str, &PackageFile> = std::collections::BTreeMap::new();
    for pf in &descriptor.files {
        // Listed paths must themselves be safe before we join them anywhere.
        safe_entry_path(Path::new(&pf.path))?;
        listed.insert(pf.path.as_str(), pf);
    }
    for (rel, size) in &on_disk {
        let rel_str = rel.to_string_lossy();
        let Some(pf) = listed.remove(rel_str.as_ref()) else {
            return Err(PackageError::UnlistedFile(rel_str.into_owned()));
        };
        if *size != pf.size {
            return Err(PackageError::SizeMismatch {
                path: pf.path.clone(),
                expected: pf.size,
                actual: *size,
            });
        }
        let actual = sha256_file(&root.join(rel))?;
        if actual != pf.sha256 {
            return Err(PackageError::ChecksumMismatch {
                path: pf.path.clone(),
                expected: pf.sha256.clone(),
                actual,
            });
        }
    }
    if let Some((path, _)) = listed.into_iter().next() {
        return Err(PackageError::ListedFileMissing(path.to_string()));
    }

    // The extracted tree must itself be a valid app dir.
    let report = validate_dir_inner(&root)?;

    // PACKAGE.toml identity must agree with manifest.toml.
    if descriptor.package.id != report.id {
        return Err(PackageError::IdMismatch {
            package: descriptor.package.id,
            manifest: report.id,
        });
    }
    if descriptor.package.version != report.version {
        return Err(PackageError::VersionMismatch {
            package: descriptor.package.version,
            manifest: report.version,
        });
    }
    if declared_runtime != report.runtime {
        return Err(PackageError::UnsupportedRuntime(format!(
            "{} (manifest entry '{}' implies {})",
            descriptor.package.runtime,
            report.entry,
            report.runtime.as_str()
        )));
    }

    Ok(report)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::io::{Read, Write};
    use tempfile::TempDir;

    /// Minimal valid python app dir: manifest + entry.
    fn write_app(dir: &Path, id: &str, capabilities: &str) {
        fs::write(
            dir.join("manifest.toml"),
            format!(
                "schema_version = 1\n\n\
                 [app]\n\
                 id = \"{id}\"\n\
                 type = \"app\"\n\
                 name = \"Test App\"\n\
                 entry = \"main.py\"\n\
                 version = \"0.1.0\"\n\
                 description = \"A test app\"\n\n\
                 [app.capabilities]\n\
                 capabilities = [{capabilities}]\n\n\
                 [launch]\n"
            ),
        )
        .unwrap();
        fs::write(dir.join("main.py"), "# entry\nprint('hi')\n").unwrap();
    }

    /// Read every entry of a built package into (path → bytes).
    fn read_entries(pkg: &Path) -> BTreeMap<String, Vec<u8>> {
        let f = fs::File::open(pkg).unwrap();
        let gz = flate2::read::GzDecoder::new(f);
        let mut archive = tar::Archive::new(gz);
        let mut map = BTreeMap::new();
        for entry in archive.entries().unwrap() {
            let mut entry = entry.unwrap();
            let path = entry.path().unwrap().display().to_string();
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).unwrap();
            map.insert(path, bytes);
        }
        map
    }

    /// Write a tar.gz package from (path → bytes) entries. Writes the entry
    /// name into the raw GNU header bytes so tests can forge paths (e.g.
    /// `../evil`) that `tar::Header::set_path` would refuse to create.
    fn write_entries(out: &Path, entries: &BTreeMap<String, Vec<u8>>) {
        let f = fs::File::create(out).unwrap();
        let gz = flate2::write::GzEncoder::new(f, flate2::Compression::default());
        let mut builder = tar::Builder::new(gz);
        for (path, bytes) in entries {
            let mut header = tar::Header::new_gnu();
            {
                let gnu = header.as_gnu_mut().unwrap();
                let name = path.as_bytes();
                assert!(name.len() < gnu.name.len(), "test path too long: {path}");
                gnu.name[..name.len()].copy_from_slice(name);
            }
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append(&header, bytes.as_slice()).unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap();
    }

    /// Build a valid package for a fresh app dir; returns (workdir, pkg path).
    fn build_valid_package(id: &str) -> (TempDir, PathBuf) {
        let work = TempDir::new().unwrap();
        let app_dir = work.path().join("app");
        fs::create_dir(&app_dir).unwrap();
        write_app(&app_dir, id, "\"ai.query\"");
        let out = work.path().join(format!("{id}-0.1.0.plexipkg"));
        let pkg = build_package(&app_dir, Some(&out)).unwrap();
        (work, pkg)
    }

    #[test]
    fn round_trip_build_and_validate() {
        let (_work, pkg) = build_valid_package("pkg-roundtrip");
        let report = validate_package(&pkg).expect("freshly built package must validate");
        assert_eq!(report.id, "pkg-roundtrip");
        assert_eq!(report.name, "Test App");
        assert_eq!(report.version, "0.1.0");
        assert_eq!(report.runtime, PackageRuntime::Python);
        assert_eq!(report.entry, "main.py");
        assert_eq!(report.capabilities, vec![Capability::AiQuery]);
        assert_eq!(report.file_count, 2, "manifest.toml + main.py");
        assert!(report.total_size > 0);
    }

    #[test]
    fn tampered_content_fails_checksum() {
        let (work, pkg) = build_valid_package("pkg-tamper");
        let mut entries = read_entries(&pkg);
        // Flip a byte in main.py without changing its length.
        let body = entries.get_mut("main.py").unwrap();
        body[0] ^= 0xFF;
        let tampered = work.path().join("tampered.plexipkg");
        write_entries(&tampered, &entries);

        let err = validate_package(&tampered).unwrap_err();
        assert!(
            matches!(err, PackageError::ChecksumMismatch { ref path, .. } if path == "main.py"),
            "expected ChecksumMismatch for main.py, got: {err}"
        );
    }

    #[test]
    fn path_traversal_entry_rejected() {
        let (work, pkg) = build_valid_package("pkg-traversal");
        let mut entries = read_entries(&pkg);
        entries.insert("../evil".to_string(), b"owned".to_vec());
        let evil = work.path().join("evil.plexipkg");
        write_entries(&evil, &entries);

        let err = validate_package(&evil).unwrap_err();
        assert!(
            matches!(err, PackageError::UnsafePath(ref p) if p.contains("..")),
            "expected UnsafePath, got: {err}"
        );
        // The traversal target must not have been written.
        assert!(!work.path().join("evil").exists());
    }

    #[test]
    fn symlink_in_dir_rejected_at_build() {
        let work = TempDir::new().unwrap();
        let app_dir = work.path().join("app");
        fs::create_dir(&app_dir).unwrap();
        write_app(&app_dir, "pkg-symlink", "");
        std::os::unix::fs::symlink("/etc/hosts", app_dir.join("link")).unwrap();

        let err = build_package(&app_dir, Some(&work.path().join("x.plexipkg"))).unwrap_err();
        assert!(
            matches!(err, PackageError::SymlinkInDir(_)),
            "expected SymlinkInDir, got: {err}"
        );
    }

    #[test]
    fn symlink_entry_in_archive_rejected() {
        let (work, pkg) = build_valid_package("pkg-symlink-tar");
        let entries = read_entries(&pkg);

        // Re-write the archive and append a symlink entry by hand.
        let evil = work.path().join("symlink.plexipkg");
        let f = fs::File::create(&evil).unwrap();
        let gz = flate2::write::GzEncoder::new(f, flate2::Compression::default());
        let mut builder = tar::Builder::new(gz);
        for (path, bytes) in &entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(&mut header, path.as_str(), bytes.as_slice())
                .unwrap();
        }
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        header.set_mode(0o777);
        builder
            .append_link(&mut header, "sneaky-link", "/etc/hosts")
            .unwrap();
        builder.into_inner().unwrap().finish().unwrap();

        let err = validate_package(&evil).unwrap_err();
        assert!(
            matches!(err, PackageError::ForbiddenEntryType(ref p, "symlink") if p == "sneaky-link"),
            "expected ForbiddenEntryType symlink, got: {err}"
        );
    }

    #[test]
    fn unknown_capability_is_hard_error() {
        let work = TempDir::new().unwrap();
        let app_dir = work.path().join("app");
        fs::create_dir(&app_dir).unwrap();
        // "net.dns" is NOT a Capability variant.
        write_app(&app_dir, "pkg-badcap", "\"net.dns\"");

        let err = validate_dir(&app_dir).unwrap_err();
        match err {
            PackageError::UnknownCapability { ref value, .. } => assert_eq!(value, "net.dns"),
            other => panic!("expected UnknownCapability, got: {other}"),
        }
        // Build must refuse too.
        assert!(build_package(&app_dir, Some(&work.path().join("x.plexipkg"))).is_err());
    }

    #[test]
    fn missing_manifest_is_hard_error() {
        let work = TempDir::new().unwrap();
        let app_dir = work.path().join("app");
        fs::create_dir(&app_dir).unwrap();
        fs::write(app_dir.join("main.py"), "# no manifest\n").unwrap();

        let err = validate_dir(&app_dir).unwrap_err();
        assert!(
            matches!(err, PackageError::ManifestMissing(_)),
            "expected ManifestMissing, got: {err}"
        );
    }

    #[test]
    fn package_toml_id_mismatch_rejected() {
        let (work, pkg) = build_valid_package("pkg-idmismatch");
        let mut entries = read_entries(&pkg);
        let descriptor = String::from_utf8(entries.get(PACKAGE_TOML).unwrap().clone()).unwrap();
        let edited = descriptor.replace("pkg-idmismatch", "some-other-id");
        assert_ne!(descriptor, edited, "fixture must actually change the id");
        entries.insert(PACKAGE_TOML.to_string(), edited.into_bytes());
        let forged = work.path().join("forged.plexipkg");
        write_entries(&forged, &entries);

        let err = validate_package(&forged).unwrap_err();
        assert!(
            matches!(err, PackageError::IdMismatch { .. }),
            "expected IdMismatch, got: {err}"
        );
    }

    #[test]
    fn missing_package_toml_rejected() {
        let (work, pkg) = build_valid_package("pkg-nodescriptor");
        let mut entries = read_entries(&pkg);
        entries.remove(PACKAGE_TOML).unwrap();
        let stripped = work.path().join("stripped.plexipkg");
        write_entries(&stripped, &entries);

        let err = validate_package(&stripped).unwrap_err();
        assert!(
            matches!(err, PackageError::PackageTomlMissing(_)),
            "expected PackageTomlMissing, got: {err}"
        );
    }

    #[test]
    fn unlisted_extra_file_rejected() {
        let (work, pkg) = build_valid_package("pkg-extrafile");
        let mut entries = read_entries(&pkg);
        entries.insert("smuggled.py".to_string(), b"import os\n".to_vec());
        let smuggled = work.path().join("smuggled.plexipkg");
        write_entries(&smuggled, &entries);

        let err = validate_package(&smuggled).unwrap_err();
        assert!(
            matches!(err, PackageError::UnlistedFile(ref p) if p == "smuggled.py"),
            "expected UnlistedFile, got: {err}"
        );
    }

    #[test]
    fn wrong_extension_rejected() {
        let work = TempDir::new().unwrap();
        let bogus = work.path().join("app.tar.gz");
        fs::File::create(&bogus).unwrap().write_all(b"x").unwrap();
        let err = validate_package(&bogus).unwrap_err();
        assert!(
            matches!(err, PackageError::WrongExtension(_)),
            "expected WrongExtension, got: {err}"
        );
    }

    #[test]
    fn default_output_name_is_id_version() {
        let work = TempDir::new().unwrap();
        let app_dir = work.path().join("app");
        fs::create_dir(&app_dir).unwrap();
        write_app(&app_dir, "pkg-name", "");
        // Build with out inside the temp dir to avoid touching CWD.
        let out = work.path().join("pkg-name-0.1.0.plexipkg");
        let path = build_package(&app_dir, Some(&out)).unwrap();
        assert_eq!(path, out);
        assert!(path.exists());
        let report = validate_package(&path).unwrap();
        assert_eq!(report.runtime, PackageRuntime::Python);
    }
}
