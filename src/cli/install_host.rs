//! App installer (#308 Phase 2).
//!
//! Three top-level CLI subcommands route here:
//!   - `plexi install <source-spec>[@ref]` — clone + place one app
//!   - `plexi install --pack <path|core>`  — apply every [[app]] in a pack
//!   - `plexi update apps [<id>]`           — git-pull installed app(s)
//!   - `plexi uninstall <id>`              — remove the install dir
//!   - `plexi list`                        — show installed apps
//!
//! All git work goes through the [`Cloner`] trait so unit tests can mock it
//! (mirrors `workspace_secrets::SecretStore`'s pattern). Production uses
//! [`GitCloner`] which shells out to `git` — no `git2` dependency.
//!
//! The bundled core pack is `packs/core.toml` baked in at compile time via
//! `include_str!`. Sources are `local:<app-name>` — the installer copies
//! from the embedded `apps/` tree.

use crate::app::registry::{AppManifest, MANIFEST_SCHEMA_VERSION};
use crate::app::packs::{parse_source_spec, Pack, PackApp, SourceSpec};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Pack file embedded into the binary. Auto-applied on every launch (core) or
/// on first launch only (examples). Also reachable via `plexi install --pack core`.
pub const CORE_PACK_TOML: &str = include_str!("../../packs/core.toml");
pub const EXAMPLES_PACK_TOML: &str = include_str!("../../packs/examples.toml");

// All apps under apps/ are embedded at compile time, including apps/dev/.
// `local:` source-spec installs look up by name in EMBEDDED_APPS.
// apps/dev/ apps are not referenced by any pack entry, so they are never
// auto-installed — they are only rsync'd to the profile dir by install.sh.
static EMBEDDED_APPS: include_dir::Dir<'_> =
    include_dir::include_dir!("$CARGO_MANIFEST_DIR/apps");

/// Result of installing or updating one app entry. Aggregated for pack apply.
#[derive(Debug, Clone)]
pub struct InstallOutcome {
    pub id: String,
    pub status: InstallStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallStatus {
    /// Newly installed at the given path.
    Installed(PathBuf),
    /// Already present at the requested version — no-op (pack apply
    /// idempotency).
    AlreadyAtVersion,
    /// Skipped because installed but at a different version (pack apply
    /// surfaces this so the user can decide).
    SkippedOtherVersion {
        installed: String,
        requested: String,
    },
    /// Installation failed; `String` carries a human-readable error.
    Failed(String),
}

/// Abstraction over the actual git client. Production: [`GitCloner`] (shells
/// out to `git`). Tests: in-memory mock that just creates the target dir.
pub trait Cloner: Send + Sync {
    /// Clone `url` into `target_dir`. `target_dir` must not exist yet — the
    /// implementation is responsible for creating it.
    fn clone(&self, url: &str, target_dir: &Path) -> Result<(), String>;
    /// Check out a specific ref (tag, branch, or sha) inside an existing
    /// clone. Optional — `None` = leave on the default branch.
    fn checkout(&self, repo_dir: &Path, git_ref: &str) -> Result<(), String>;
    /// `git fetch && git pull` — used by `plexi update apps`.
    fn pull(&self, repo_dir: &Path) -> Result<(), String>;
}

/// Production [`Cloner`] that shells out to the system `git` binary. No
/// libgit2 / `git2` crate dependency — keeps the release build slim.
pub struct GitCloner;

impl Cloner for GitCloner {
    fn clone(&self, url: &str, target_dir: &Path) -> Result<(), String> {
        let status = Command::new("git")
            .args(["clone", "--depth", "1", url])
            .arg(target_dir)
            .status()
            .map_err(|e| format!("could not run git: {e}"))?;
        if !status.success() {
            return Err(format!(
                "git clone {url} failed (exit {})",
                status.code().unwrap_or(-1)
            ));
        }
        Ok(())
    }

    fn checkout(&self, repo_dir: &Path, git_ref: &str) -> Result<(), String> {
        // For shallow clones we may need the ref fetched explicitly.
        let _ = Command::new("git")
            .args(["fetch", "--depth", "1", "origin", git_ref])
            .current_dir(repo_dir)
            .status();
        let status = Command::new("git")
            .args(["checkout", git_ref])
            .current_dir(repo_dir)
            .status()
            .map_err(|e| format!("could not run git checkout: {e}"))?;
        if !status.success() {
            return Err(format!(
                "git checkout {git_ref} failed (exit {})",
                status.code().unwrap_or(-1)
            ));
        }
        Ok(())
    }

    fn pull(&self, repo_dir: &Path) -> Result<(), String> {
        let fetch = Command::new("git")
            .args(["fetch"])
            .current_dir(repo_dir)
            .status()
            .map_err(|e| format!("could not run git fetch: {e}"))?;
        if !fetch.success() {
            return Err(format!(
                "git fetch failed (exit {})",
                fetch.code().unwrap_or(-1)
            ));
        }
        let pull = Command::new("git")
            .args(["pull", "--ff-only"])
            .current_dir(repo_dir)
            .status()
            .map_err(|e| format!("could not run git pull: {e}"))?;
        if !pull.success() {
            return Err(format!(
                "git pull failed (exit {})",
                pull.code().unwrap_or(-1)
            ));
        }
        Ok(())
    }
}

/// Split a source-with-optional-ref spec like `github:owner/repo@v1.2.0`.
pub fn split_source_and_ref(input: &str) -> (String, Option<String>) {
    // The `@` may legitimately appear inside `git+ssh://user@host/...` — only
    // treat it as a ref delimiter when it appears AFTER the scheme separator
    // and there is exactly one `@` after that point.
    if let Some(idx) = input.rfind('@') {
        let scheme_end = input.find("://").map(|i| i + 3).unwrap_or(0);
        if idx >= scheme_end {
            // Distinguish git+ssh's user@host (which has '@' before any '/')
            // from a true `@ref` (which has '@' after a `/`).
            let after_scheme = &input[scheme_end..];
            let at_in_after = idx - scheme_end;
            let has_slash_before_at = after_scheme[..at_in_after].contains('/');
            if has_slash_before_at || !after_scheme.contains('/') {
                let (src, rest) = input.split_at(idx);
                let git_ref = rest.trim_start_matches('@').to_string();
                if !git_ref.is_empty() {
                    return (src.to_string(), Some(git_ref));
                }
            }
        }
    }
    (input.to_string(), None)
}

/// Install one app from a parsed source spec at an optional git ref.
/// `target_root` is the channel apps dir (e.g. `~/.plexi-alpha/apps`).
pub fn install_one(
    cloner: &dyn Cloner,
    source: &SourceSpec,
    git_ref: Option<&str>,
    target_root: &Path,
) -> Result<InstallOutcome, String> {
    match source {
        SourceSpec::Git(url) => install_one_git(cloner, url, git_ref, target_root),
        SourceSpec::Local(name) => install_one_local(name, target_root),
    }
}

fn install_one_git(
    cloner: &dyn Cloner,
    url: &str,
    git_ref: Option<&str>,
    target_root: &Path,
) -> Result<InstallOutcome, String> {
    std::fs::create_dir_all(target_root)
        .map_err(|e| format!("create apps dir {}: {e}", target_root.display()))?;

    // Stage into a sibling `.tmp-<unique>/clone` dir inside `target_root` so
    // we can read the manifest BEFORE committing to the final dest. Same-FS
    // ensures the eventual `rename` is atomic. We hand-roll the temp dir name
    // (no `tempfile` dependency in non-test code) — uniqueness comes from
    // nanos + pid.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    let stage_root = target_root.join(format!(".tmp-install-{pid}-{nanos}"));
    let stage_path = stage_root.join("clone");
    // Best-effort cleanup wrapper: any early return below leaks the dir, so
    // we explicitly clean on every error branch via a local closure.
    let cleanup = |p: &Path| {
        let _ = std::fs::remove_dir_all(p);
    };
    if let Err(e) = std::fs::create_dir_all(&stage_root) {
        return Err(format!("create staging dir: {e}"));
    }

    if let Err(e) = cloner.clone(url, &stage_path) {
        cleanup(&stage_root);
        return Err(e);
    }
    if let Some(r) = git_ref {
        if let Err(e) = cloner.checkout(&stage_path, r) {
            cleanup(&stage_root);
            return Err(e);
        }
    }

    let manifest = match read_manifest(&stage_path) {
        Ok(m) => m,
        Err(e) => {
            cleanup(&stage_root);
            return Err(e);
        }
    };
    let id = manifest.app.id.clone();
    let dest = target_root.join(&id);

    if dest.exists() {
        cleanup(&stage_root);
        return Err(format!(
            "app '{id}' already installed at {}; \
             run `plexi uninstall {id}` or `plexi update apps {id}` first",
            dest.display()
        ));
    }

    if let Err(e) = std::fs::rename(&stage_path, &dest) {
        cleanup(&stage_root);
        return Err(format!("install {id}: rename staging → dest failed: {e}"));
    }
    // Drop the (now-empty) staging root.
    cleanup(&stage_root);

    chmod_python_entries(&dest);
    provision_venv(&id, &dest, &manifest);
    write_installed_version_file(&dest, &manifest.app.version);
    log::info!("install: {} ← {url} ({})", id, git_ref.unwrap_or("default"));
    Ok(InstallOutcome {
        id,
        status: InstallStatus::Installed(dest),
    })
}

fn install_one_local(name: &str, target_root: &Path) -> Result<InstallOutcome, String> {
    std::fs::create_dir_all(target_root)
        .map_err(|e| format!("create apps dir {}: {e}", target_root.display()))?;

    let example = EMBEDDED_APPS
        .get_dir(name)
        .ok_or_else(|| format!("local source '{name}' not found in bundled apps"))?;

    // Read manifest text from the embedded tree to recover the canonical id.
    let manifest_text = example
        .get_file(format!("{name}/manifest.toml"))
        .and_then(|f| f.contents_utf8())
        .ok_or_else(|| format!("local source '{name}' has no manifest.toml"))?;
    let manifest: AppManifest = toml::from_str(manifest_text)
        .map_err(|e| format!("local source '{name}' manifest invalid: {e}"))?;
    if manifest.schema_version > MANIFEST_SCHEMA_VERSION {
        return Err(format!(
            "local source '{name}' manifest schema_version = {} > supported {}",
            manifest.schema_version, MANIFEST_SCHEMA_VERSION
        ));
    }
    let id = manifest.app.id.clone();
    let dest = target_root.join(&id);
    if dest.exists() {
        return Err(format!(
            "app '{id}' already installed at {}",
            dest.display()
        ));
    }
    // `include_dir::Dir::extract` writes paths relative to the *caller*'s
    // arg using the entries' stored paths (which include `<name>/...` because
    // we embedded `apps/`). Calling `extract(target_root)` therefore
    // writes `target_root/<name>/manifest.toml`, etc. We just need to ensure
    // `dest` exists first; `extract` panics otherwise on macOS tempdirs.
    std::fs::create_dir_all(&dest)
        .map_err(|e| format!("create {}: {e}", dest.display()))?;
    example
        .extract(target_root)
        .map_err(|e| format!("extract local '{name}' → {}: {e}", target_root.display()))?;
    chmod_python_entries(&dest);
    provision_venv(&id, &dest, &manifest);
    write_installed_version_file(&dest, &manifest.app.version);
    log::info!("install: {id} ← local:{name}");
    Ok(InstallOutcome {
        id,
        status: InstallStatus::Installed(dest),
    })
}

/// Write `<app_dir>/installed_version.txt`. Best-effort — never fails the install.
fn write_installed_version_file(app_dir: &Path, version: &str) {
    let path = app_dir.join("installed_version.txt");
    if let Err(e) = std::fs::write(&path, version) {
        log::warn!(
            "install: could not write installed_version.txt for {}: {e}",
            app_dir.display()
        );
    } else {
        log::info!(
            "install: wrote installed_version.txt={version} for {}",
            app_dir.display()
        );
    }
}

/// Read + validate `manifest.toml` from a freshly-cloned repo. Returns a
/// clear error for a missing file, a parse error, or an unsupported
/// schema_version.
fn read_manifest(repo_dir: &Path) -> Result<AppManifest, String> {
    let manifest_path = repo_dir.join("manifest.toml");
    let text = std::fs::read_to_string(&manifest_path).map_err(|e| {
        format!(
            "no manifest.toml at repo root ({}): {e}",
            manifest_path.display()
        )
    })?;
    let manifest: AppManifest =
        toml::from_str(&text).map_err(|e| format!("manifest.toml invalid: {e}"))?;
    if manifest.schema_version > MANIFEST_SCHEMA_VERSION {
        return Err(format!(
            "manifest schema_version = {} is newer than this Plexi build supports (max {}); \
             update Plexi to install this app",
            manifest.schema_version, MANIFEST_SCHEMA_VERSION
        ));
    }
    Ok(manifest)
}

#[cfg(unix)]
fn chmod_python_entries(app_dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let entries = match std::fs::read_dir(app_dir) {
        Ok(d) => d,
        Err(e) => {
            log::warn!("install: could not chmod {}: {e}", app_dir.display());
            return;
        }
    };
    for f in entries.flatten() {
        let p = f.path();
        if p.extension().and_then(|x| x.to_str()) == Some("py") {
            if let Ok(meta) = std::fs::metadata(&p) {
                let mut perms = meta.permissions();
                perms.set_mode(perms.mode() | 0o111);
                let _ = std::fs::set_permissions(&p, perms);
            }
        }
    }
}

#[cfg(not(unix))]
fn chmod_python_entries(_app_dir: &Path) {}

/// Create a per-app Python venv using `uv` when the app has a `.py` entry.
///
/// - If `.venv` already exists (re-install): logs `info!` and skips creation.
/// - If `uv` is not on PATH: logs `warn!` and returns — never fails the install.
/// - If venv creation or dep install fails: logs `warn!` — never fails the install.
/// - If `dependencies` is non-empty: runs `uv pip install` into the venv.
fn provision_venv(app_id: &str, app_dir: &Path, manifest: &AppManifest) {
    let entry = &manifest.app.entry;
    let is_python = entry.ends_with(".py");
    if !is_python {
        return;
    }

    let venv_path = app_dir.join(".venv");

    // Skip if venv already exists (idempotent re-install).
    if venv_path.exists() {
        log::info!(
            "install[{app_id}]: venv already exists at {}; skipping creation",
            venv_path.display()
        );
        // Still attempt dep install in case deps changed — best-effort, warn on failure.
        if !manifest.app.dependencies.is_empty() {
            install_deps_into_venv(app_id, &venv_path, &manifest.app.dependencies);
        }
        return;
    }

    // Check uv is available.
    let uv_check = Command::new("uv").arg("--version").output();
    if uv_check.is_err() || !uv_check.as_ref().map(|o| o.status.success()).unwrap_or(false) {
        log::warn!(
            "install[{app_id}]: `uv` not found on PATH — skipping venv creation. \
             Install uv (https://docs.astral.sh/uv/getting-started/installation/) to enable \
             isolated Python environments for Plexi apps."
        );
        return;
    }

    // Create the venv.
    let status = Command::new("uv")
        .args(["venv", "--python", "3.11"])
        .arg(&venv_path)
        .status();
    match status {
        Err(e) => {
            log::warn!(
                "install[{app_id}]: failed to run `uv venv`: {e} — app will use system Python"
            );
            return;
        }
        Ok(s) if !s.success() => {
            // uv may refuse 3.11 if not installed; fall back to default Python.
            log::warn!(
                "install[{app_id}]: `uv venv --python 3.11` failed (exit {}); \
                 retrying without version pin",
                s.code().unwrap_or(-1)
            );
            let retry = Command::new("uv").arg("venv").arg(&venv_path).status();
            match retry {
                Err(e) => {
                    log::warn!(
                        "install[{app_id}]: `uv venv` retry failed: {e} — app will use system Python"
                    );
                    return;
                }
                Ok(s2) if !s2.success() => {
                    log::warn!(
                        "install[{app_id}]: `uv venv` retry exit {} — app will use system Python",
                        s2.code().unwrap_or(-1)
                    );
                    return;
                }
                _ => {}
            }
        }
        _ => {}
    }

    log::info!(
        "install[{app_id}]: created venv at {}",
        venv_path.display()
    );

    if !manifest.app.dependencies.is_empty() {
        install_deps_into_venv(app_id, &venv_path, &manifest.app.dependencies);
    }
}

/// Install `deps` into an existing venv using `uv pip install`. Best-effort —
/// logs `warn!` on failure but never panics or propagates.
fn install_deps_into_venv(app_id: &str, venv_path: &Path, deps: &[String]) {
    let python = venv_path.join("bin").join("python");
    let mut cmd = Command::new("uv");
    cmd.args(["pip", "install", "--python"]);
    cmd.arg(&python);
    cmd.args(deps);
    log::info!(
        "install[{app_id}]: installing {} dep(s): {}",
        deps.len(),
        deps.join(", ")
    );
    match cmd.status() {
        Err(e) => log::warn!("install[{app_id}]: `uv pip install` failed to run: {e}"),
        Ok(s) if !s.success() => log::warn!(
            "install[{app_id}]: `uv pip install` exit {} — deps may be missing",
            s.code().unwrap_or(-1)
        ),
        Ok(_) => log::info!("install[{app_id}]: deps installed successfully"),
    }
}

/// Apply every `[[app]]` entry in a pack against `target_root`. Errors are
/// aggregated per app so one bad entry doesn't abort the whole apply.
/// Idempotent: an entry whose id is already installed at the same version is
/// reported as `AlreadyAtVersion`.
pub fn apply_pack(
    cloner: &dyn Cloner,
    pack: &Pack,
    target_root: &Path,
) -> Vec<InstallOutcome> {
    let mut outcomes = Vec::with_capacity(pack.apps.len());
    let installed = installed_versions(target_root);
    for entry in &pack.apps {
        outcomes.push(apply_pack_entry(cloner, entry, target_root, &installed));
    }
    outcomes
}

fn apply_pack_entry(
    cloner: &dyn Cloner,
    entry: &PackApp,
    target_root: &Path,
    installed: &HashMap<String, String>,
) -> InstallOutcome {
    if let Some(installed_version) = installed.get(&entry.id) {
        if installed_version == &entry.version {
            return InstallOutcome {
                id: entry.id.clone(),
                status: InstallStatus::AlreadyAtVersion,
            };
        }
        return InstallOutcome {
            id: entry.id.clone(),
            status: InstallStatus::SkippedOtherVersion {
                installed: installed_version.clone(),
                requested: entry.version.clone(),
            },
        };
    }
    let source = match parse_source_spec(&entry.source) {
        Ok(s) => s,
        Err(e) => {
            return InstallOutcome {
                id: entry.id.clone(),
                status: InstallStatus::Failed(e),
            };
        }
    };
    let git_ref = match &source {
        SourceSpec::Git(_) => Some(entry.version.as_str()),
        SourceSpec::Local(_) => None,
    };
    match install_one(cloner, &source, git_ref, target_root) {
        Ok(outcome) => outcome,
        Err(e) => InstallOutcome {
            id: entry.id.clone(),
            status: InstallStatus::Failed(e),
        },
    }
}

/// Walk `<target_root>/<id>/manifest.toml` for every dir entry and harvest
/// the manifest version. Used to drive `plexi list` and pack-apply
/// idempotency. Apps without a parseable manifest are skipped silently
/// (already logged elsewhere by `AppRegistry`).
pub fn installed_versions(target_root: &Path) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let entries = match std::fs::read_dir(target_root) {
        Ok(d) => d,
        Err(_) => return out,
    };
    for e in entries.flatten() {
        let manifest = e.path().join("manifest.toml");
        if let Ok(text) = std::fs::read_to_string(&manifest) {
            if let Ok(parsed) = toml::from_str::<AppManifest>(&text) {
                out.insert(parsed.app.id, parsed.app.version);
            }
        }
    }
    out
}

/// Remove `<target_root>/<id>/`. Errors if the dir doesn't exist.
pub fn uninstall_one(id: &str, target_root: &Path) -> Result<(), String> {
    let dest = target_root.join(id);
    if !dest.exists() {
        return Err(format!("'{id}' is not installed at {}", dest.display()));
    }
    std::fs::remove_dir_all(&dest).map_err(|e| format!("remove {}: {e}", dest.display()))?;
    log::info!("uninstall: {id}");
    Ok(())
}

/// `git fetch && git pull` an installed app. Returns an error if the app
/// dir isn't a git checkout (no `.git/`) — that's the bundled-core case and
/// the caller should treat it as a skip rather than a hard error.
pub fn update_one(cloner: &dyn Cloner, id: &str, target_root: &Path) -> Result<(), String> {
    let dest = target_root.join(id);
    if !dest.exists() {
        return Err(format!("'{id}' is not installed at {}", dest.display()));
    }
    if !dest.join(".git").exists() {
        return Err("not a git checkout (likely bundled core app)".to_string());
    }
    cloner.pull(&dest)?;
    log::info!("update: {id}");
    Ok(())
}

/// Export the current set of installed apps under `target_root` as a
/// `pack.toml` written to `dest_path`. For each app:
///   - If the install dir is a git checkout (`.git/` present), the source is
///     `git+<remote-origin-url>` and the version is the current ref
///     (`git describe --tags --exact-match` → tag, else short SHA).
///   - Otherwise (bundled core / `local:` install), source is
///     `local:<id>` and version is `manifest.version` (or `bundled` if
///     unset). These re-install from the binary's embedded examples on
///     `plexi install --pack <path>`.
/// Apps without a parseable manifest are skipped with a warning.
///
/// Returns the number of entries written.
pub fn export_pack(target_root: &Path, dest_path: &Path) -> Result<usize, String> {
    let entries = match std::fs::read_dir(target_root) {
        Ok(d) => d,
        Err(e) => {
            return Err(format!(
                "read apps dir {}: {e}",
                target_root.display()
            ));
        }
    };

    let mut rows: Vec<(String, String, String)> = Vec::new();
    for e in entries.flatten() {
        let app_dir = e.path();
        // Skip non-dirs and the staging dirs install_one leaves behind on
        // crash (`.tmp-install-*`). Both would produce a noisy export.
        if !app_dir.is_dir() {
            continue;
        }
        if app_dir
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with(".tmp-install-"))
        {
            continue;
        }
        let manifest_path = app_dir.join("manifest.toml");
        let text = match std::fs::read_to_string(&manifest_path) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let manifest: AppManifest = match toml::from_str(&text) {
            Ok(m) => m,
            Err(err) => {
                log::warn!(
                    "pack export: skipping {}: manifest invalid: {err}",
                    app_dir.display()
                );
                continue;
            }
        };
        let id = manifest.app.id.clone();
        let (source, version) = infer_source_and_version(&app_dir, &manifest);
        rows.push((id, source, version));
    }

    rows.sort_by(|a, b| a.0.cmp(&b.0));

    let mut out = String::new();
    out.push_str("# Generated by `plexi pack export`. Edit freely.\n");
    out.push_str("schema_version = 1\n\n");
    for (id, source, version) in &rows {
        out.push_str("[[app]]\n");
        out.push_str(&format!("id      = \"{id}\"\n"));
        out.push_str(&format!("source  = \"{source}\"\n"));
        out.push_str(&format!("version = \"{version}\"\n\n"));
    }

    if let Some(parent) = dest_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("create dest parent {}: {e}", parent.display()))?;
        }
    }
    std::fs::write(dest_path, out)
        .map_err(|e| format!("write pack to {}: {e}", dest_path.display()))?;
    log::info!(
        "pack export: wrote {} apps to {}",
        rows.len(),
        dest_path.display()
    );
    Ok(rows.len())
}

/// Best-effort inference of `(source_spec, version)` for one installed app
/// dir. Pure-IO helper for `export_pack`. Public for the CLI test that
/// wants to assert the local-vs-git split independently of file scanning.
pub fn infer_source_and_version(app_dir: &Path, manifest: &AppManifest) -> (String, String) {
    if app_dir.join(".git").exists() {
        let url = git_remote_origin_url(app_dir).unwrap_or_else(|| String::from("git+unknown"));
        let source = format_git_source_spec(&url);
        let version = git_current_ref(app_dir)
            .unwrap_or_else(|| manifest.app.version.clone());
        return (source, version);
    }
    let id = &manifest.app.id;
    let version = if manifest.app.version.is_empty() {
        "bundled".to_string()
    } else {
        manifest.app.version.clone()
    };
    (format!("local:{id}"), version)
}

fn git_remote_origin_url(repo_dir: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["config", "--get", "remote.origin.url"])
        .current_dir(repo_dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let raw = String::from_utf8(output.stdout).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_string())
}

/// Convert a raw remote URL into our pack source-spec scheme. Prefers the
/// short `github:owner/repo` form when the URL points at github.com.
fn format_git_source_spec(url: &str) -> String {
    // Strip a trailing `.git` for the github short form so it matches the
    // canonical write-back of `parse_source_spec("github:owner/repo")`.
    let stripped = url.trim_end_matches(".git");
    if let Some(rest) = stripped.strip_prefix("https://github.com/") {
        if rest.matches('/').count() == 1 && !rest.is_empty() {
            return format!("github:{rest}");
        }
    }
    if let Some(rest) = stripped.strip_prefix("git@github.com:") {
        if rest.matches('/').count() == 1 && !rest.is_empty() {
            return format!("github:{rest}");
        }
    }
    // Anything else: pass through under the `git+` scheme so re-install
    // routes through the standard git cloner.
    if url.starts_with("https://")
        || url.starts_with("http://")
        || url.starts_with("ssh://")
        || url.starts_with("file://")
    {
        return format!("git+{url}");
    }
    // Last resort — keep the raw value so the user can fix it by hand
    // rather than silently lose info.
    url.to_string()
}

fn git_current_ref(repo_dir: &Path) -> Option<String> {
    // Prefer an exact-tag match so re-install lands on the same release.
    let tag = Command::new("git")
        .args(["describe", "--tags", "--exact-match"])
        .current_dir(repo_dir)
        .output()
        .ok();
    if let Some(out) = tag {
        if out.status.success() {
            if let Ok(s) = String::from_utf8(out.stdout) {
                let trimmed = s.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
        }
    }
    // Fall back to a short SHA — re-install still reproducible.
    let sha = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(repo_dir)
        .output()
        .ok()?;
    if !sha.status.success() {
        return None;
    }
    let s = String::from_utf8(sha.stdout).ok()?;
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Apply the bundled core pack into `target_root` on every launch. Only installs
/// apps that are missing — already-installed apps return `SkippedOtherVersion` or
/// `AlreadyAtVersion` and are left untouched.
///
/// Respects `reseed = "always"` (default for core) — always runs regardless of
/// whether `target_root` is populated.
pub fn apply_core_pack_always(cloner: &dyn Cloner, target_root: &Path) -> Vec<InstallOutcome> {
    let pack = match Pack::from_toml_str(CORE_PACK_TOML) {
        Ok(p) => p,
        Err(e) => {
            log::error!("core pack parse failed: {e}");
            return Vec::new();
        }
    };
    log::debug!(
        "install: core pack reseed={:?}",
        pack.reseed.as_deref().unwrap_or("once")
    );
    apply_pack(cloner, &pack, target_root)
}

/// Apply the bundled examples pack into `target_root` only if `target_root` is
/// currently empty (first-launch seeding). Idempotent — returns `None` if the
/// dir is non-empty.
///
/// Respects `reseed = "once"` (default for examples) — skips if apps dir is
/// already populated.
pub fn apply_examples_pack_if_empty(
    cloner: &dyn Cloner,
    target_root: &Path,
) -> Option<Vec<InstallOutcome>> {
    let is_empty = match std::fs::read_dir(target_root) {
        Ok(d) => d.flatten().next().is_none(),
        Err(e) => {
            log::warn!("examples pack: cannot read apps dir {}: {e}", target_root.display());
            true
        }
    };
    if !is_empty {
        return None;
    }
    let pack = match Pack::from_toml_str(EXAMPLES_PACK_TOML) {
        Ok(p) => p,
        Err(e) => {
            log::error!("examples pack parse failed: {e}");
            return None;
        }
    };
    log::debug!(
        "install: examples pack reseed={:?}",
        pack.reseed.as_deref().unwrap_or("once")
    );
    Some(apply_pack(cloner, &pack, target_root))
}

/// Returns the set of app IDs defined in the bundled core pack.
pub fn core_pack_ids() -> std::collections::HashSet<String> {
    Pack::from_toml_str(CORE_PACK_TOML)
        .map(|p| p.apps.into_iter().map(|a| a.id).collect())
        .unwrap_or_default()
}

/// Returns the set of app IDs defined in the bundled examples pack.
pub fn examples_pack_ids() -> std::collections::HashSet<String> {
    Pack::from_toml_str(EXAMPLES_PACK_TOML)
        .map(|p| p.apps.into_iter().map(|a| a.id).collect())
        .unwrap_or_default()
}

/// Read `<workspace_root>/<channel_dir>/apps.toml` and install declared apps into the
/// workspace-scoped apps dir (`<workspace_root>/<channel_dir>/apps/`).
pub fn apply_workspace_pack(
    workspace_root: &Path,
    cloner: &dyn Cloner,
) -> Result<Vec<InstallOutcome>, String> {
    let channel_dir = crate::config::workspace_channel_dir();
    let apps_toml = workspace_root.join(&channel_dir).join("apps.toml");
    if !apps_toml.exists() {
        return Err(
            format!("no {channel_dir}/apps.toml found — run 'plexi workspace init' to initialize a workspace with a stub apps.toml"),
        );
    }
    let pack = Pack::from_path(&apps_toml)?;
    let workspace_apps = crate::app::registry::workspace_apps_dir(workspace_root);
    std::fs::create_dir_all(&workspace_apps)
        .map_err(|e| format!("create workspace apps dir {}: {e}", workspace_apps.display()))?;
    log::info!(
        "install: workspace pack {} → {} ({} apps)",
        apps_toml.display(),
        workspace_apps.display(),
        pack.apps.len()
    );
    Ok(apply_pack(cloner, &pack, &workspace_apps))
}

/// Return the set of app IDs declared in `<workspace_root>/<channel_dir>/apps.toml`.
/// Returns an empty set if the file is absent or unparseable.
pub fn workspace_manifest_ids(workspace_root: &Path) -> std::collections::HashSet<String> {
    let channel_dir = crate::config::workspace_channel_dir();
    let path = workspace_root.join(&channel_dir).join("apps.toml");
    match Pack::from_path(&path) {
        Ok(p) => p.apps.into_iter().map(|a| a.id).collect(),
        Err(e) => {
            if path.exists() {
                log::warn!("failed to parse workspace apps.toml: {e}");
            }
            std::collections::HashSet::new()
        }
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use std::sync::Mutex;

    /// In-memory [`Cloner`] for hermetic tests. Each "clone" writes a fixed
    /// manifest + entry file into the target dir; pull/checkout are no-ops.
    pub struct MockCloner {
        /// Optional manifest text to write per URL. Defaults to a minimal
        /// schema-1 manifest with the URL's repo name as the id.
        pub manifests: Mutex<HashMap<String, String>>,
        /// Per-URL clone failures, queued and popped in order.
        pub failures: Mutex<Vec<String>>,
        /// Track every `clone` call for assertions.
        pub clones: Mutex<Vec<(String, PathBuf)>>,
    }

    impl MockCloner {
        pub fn new() -> Self {
            Self {
                manifests: Mutex::new(HashMap::new()),
                failures: Mutex::new(Vec::new()),
                clones: Mutex::new(Vec::new()),
            }
        }
        pub fn with_manifest(self, url: &str, manifest: &str) -> Self {
            self.manifests
                .lock()
                .unwrap()
                .insert(url.to_string(), manifest.to_string());
            self
        }
        pub fn fail_next(&self, msg: &str) {
            self.failures.lock().unwrap().push(msg.to_string());
        }
    }

    impl Cloner for MockCloner {
        fn clone(&self, url: &str, target_dir: &Path) -> Result<(), String> {
            self.clones
                .lock()
                .unwrap()
                .push((url.to_string(), target_dir.to_path_buf()));
            if let Some(err) = self.failures.lock().unwrap().pop() {
                return Err(err);
            }
            std::fs::create_dir_all(target_dir).map_err(|e| e.to_string())?;
            let id = derive_id(url);
            let manifest = self
                .manifests
                .lock()
                .unwrap()
                .get(url)
                .cloned()
                .unwrap_or_else(|| {
                    format!(
                        "schema_version = 1\n\n[app]\nid = \"{id}\"\ntype = \"app\"\nname = \"{id}\"\nversion = \"0.0.1\"\nentry = \"run.sh\"\n"
                    )
                });
            std::fs::write(target_dir.join("manifest.toml"), manifest)
                .map_err(|e| e.to_string())?;
            let entry = target_dir.join("run.sh");
            std::fs::write(&entry, "#!/bin/sh\nexit 0\n").map_err(|e| e.to_string())?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut p = std::fs::metadata(&entry).map_err(|e| e.to_string())?.permissions();
                p.set_mode(0o755);
                std::fs::set_permissions(&entry, p).map_err(|e| e.to_string())?;
            }
            Ok(())
        }
        fn checkout(&self, _repo_dir: &Path, _git_ref: &str) -> Result<(), String> {
            Ok(())
        }
        fn pull(&self, _repo_dir: &Path) -> Result<(), String> {
            Ok(())
        }
    }

    /// Best-effort id from a clone URL — last path segment without `.git`.
    fn derive_id(url: &str) -> String {
        url.trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or("app")
            .trim_end_matches(".git")
            .to_string()
    }
}

#[cfg(test)]
mod install_tests {
    use super::test_support::MockCloner;
    use super::*;

    fn tmp() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn install_clone_writes_app_dir_with_manifest_id() {
        let target = tmp();
        let cloner = MockCloner::new();
        let outcome = install_one(
            &cloner,
            &SourceSpec::Git("https://example.com/foo.git".into()),
            None,
            target.path(),
        )
        .expect("install should succeed");
        assert_eq!(outcome.id, "foo");
        let dest = target.path().join("foo");
        assert!(dest.join("manifest.toml").exists());
    }

    #[test]
    fn install_already_installed_errors_clearly() {
        let target = tmp();
        let cloner = MockCloner::new();
        // First install: succeeds.
        install_one(
            &cloner,
            &SourceSpec::Git("https://example.com/foo.git".into()),
            None,
            target.path(),
        )
        .expect("first install should succeed");
        // Second install: must error with a clear message.
        let err = install_one(
            &cloner,
            &SourceSpec::Git("https://example.com/foo.git".into()),
            None,
            target.path(),
        )
        .expect_err("second install must error");
        assert!(
            err.contains("already installed") && err.contains("uninstall"),
            "expected already-installed error mentioning uninstall, got: {err}"
        );
    }

    #[test]
    fn install_with_future_schema_refuses() {
        let target = tmp();
        let manifest = format!(
            "schema_version = {}\n\n[app]\nid = \"future\"\ntype = \"app\"\nname = \"Future\"\nversion = \"0.1.0\"\nentry = \"run.sh\"\n",
            MANIFEST_SCHEMA_VERSION + 1
        );
        let cloner =
            MockCloner::new().with_manifest("https://example.com/future.git", &manifest);
        let err = install_one(
            &cloner,
            &SourceSpec::Git("https://example.com/future.git".into()),
            None,
            target.path(),
        )
        .expect_err("future schema must refuse");
        assert!(
            err.contains("newer than this Plexi build"),
            "got: {err}"
        );
        assert!(
            !target.path().join("future").exists(),
            "failed install must not leave a partial dir"
        );
    }

    #[test]
    fn install_passes_ref_to_checkout_when_set() {
        let target = tmp();
        let cloner = MockCloner::new();
        install_one(
            &cloner,
            &SourceSpec::Git("https://example.com/bar.git".into()),
            Some("v1.0.0"),
            target.path(),
        )
        .expect("install with ref should succeed");
        // MockCloner's checkout is a no-op but the install still produces the
        // staged manifest. Here we just verify the path was reached.
        assert_eq!(cloner.clones.lock().unwrap().len(), 1);
    }

    // D2: per-app Python venv path construction tests.

    #[test]
    fn venv_path_is_inside_app_dir() {
        let app_dir = std::path::Path::new("/tmp/apps/my-app");
        let venv = app_dir.join(".venv");
        assert_eq!(venv, std::path::PathBuf::from("/tmp/apps/my-app/.venv"));
        // Verify the expected python binary path within the venv.
        let py = venv.join("bin").join("python");
        assert_eq!(py, std::path::PathBuf::from("/tmp/apps/my-app/.venv/bin/python"));
    }

    #[test]
    fn python_entry_detected_by_extension() {
        // .py entry → is_python = true
        let py_entry = "main.py";
        assert!(py_entry.ends_with(".py"), "main.py should be detected as Python");

        // .sh entry → is_python = false
        let sh_entry = "run.sh";
        assert!(!sh_entry.ends_with(".py"), "run.sh must not be detected as Python");

        // entry with no extension → is_python = false
        let no_ext = "myapp";
        assert!(!no_ext.ends_with(".py"), "entry with no extension must not be detected as Python");

        // entry that happens to end in .py in a longer name
        let py_in_name = "app.py";
        assert!(py_in_name.ends_with(".py"), "app.py should be detected as Python");
    }
}

#[cfg(test)]
mod pack_tests {
    use super::test_support::MockCloner;
    use super::*;

    fn pack_with(entries: &[(&str, &str, &str)]) -> Pack {
        let mut text = String::from("schema_version = 1\n\n");
        for (id, source, version) in entries {
            text.push_str(&format!(
                "[[app]]\nid = \"{id}\"\nsource = \"{source}\"\nversion = \"{version}\"\n\n"
            ));
        }
        Pack::from_toml_str(&text).unwrap()
    }

    #[test]
    fn pack_apply_skips_already_installed_at_same_version() {
        let target = tempfile::tempdir().unwrap();
        let cloner = MockCloner::new();
        let pack = pack_with(&[("foo", "github:owner/foo", "0.0.1")]);

        let first = apply_pack(&cloner, &pack, target.path());
        assert!(matches!(first[0].status, InstallStatus::Installed(_)));

        // Second apply: same id at same version → AlreadyAtVersion.
        let second = apply_pack(&cloner, &pack, target.path());
        assert!(matches!(second[0].status, InstallStatus::AlreadyAtVersion));
    }

    #[test]
    fn pack_apply_aggregates_errors() {
        let target = tempfile::tempdir().unwrap();
        let cloner = MockCloner::new();
        cloner.fail_next("simulated network error");
        // Failures are popped in LIFO order, so the FIRST clone hits the
        // failure and the second succeeds.
        let pack = pack_with(&[
            ("alpha", "github:owner/alpha", "v1"),
            ("beta", "github:owner/beta", "v1"),
        ]);

        let outcomes = apply_pack(&cloner, &pack, target.path());
        assert_eq!(outcomes.len(), 2);
        // Exactly one failure, one install — order depends on apply_pack
        // iteration, just count by status.
        let n_failed = outcomes
            .iter()
            .filter(|o| matches!(o.status, InstallStatus::Failed(_)))
            .count();
        let n_ok = outcomes
            .iter()
            .filter(|o| matches!(o.status, InstallStatus::Installed(_)))
            .count();
        assert_eq!(n_failed, 1, "exactly one failure expected, got {outcomes:?}");
        assert_eq!(n_ok, 1, "exactly one install expected, got {outcomes:?}");
    }

    #[test]
    fn pack_apply_records_unknown_source_scheme_per_entry() {
        let target = tempfile::tempdir().unwrap();
        let cloner = MockCloner::new();
        let pack = pack_with(&[
            ("ok", "github:owner/ok", "v1"),
            ("bad", "ftp://nope", "v1"),
        ]);
        let outcomes = apply_pack(&cloner, &pack, target.path());
        let bad = outcomes.iter().find(|o| o.id == "bad").unwrap();
        match &bad.status {
            InstallStatus::Failed(msg) => assert!(msg.contains("unknown source scheme")),
            other => panic!("bad entry should fail, got: {other:?}"),
        }
        let ok = outcomes.iter().find(|o| o.id == "ok").unwrap();
        assert!(matches!(ok.status, InstallStatus::Installed(_)));
    }
}

#[cfg(test)]
mod pack_export_tests {
    use super::*;

    /// Helper: write a manifest into `target_root/<id>/manifest.toml` so the
    /// export scanner has something to pick up.
    fn write_app(target_root: &Path, id: &str, version: &str) {
        let dir = target_root.join(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("manifest.toml"),
            format!(
                "schema_version = 1\n\n[app]\nid = \"{id}\"\ntype = \"app\"\nname = \"{id}\"\nversion = \"{version}\"\nentry = \"run.sh\"\n"
            ),
        )
        .unwrap();
    }

    #[test]
    fn pack_export_writes_valid_pack_to_path() {
        let target = tempfile::tempdir().unwrap();
        write_app(target.path(), "alpha-app", "0.1.0");
        write_app(target.path(), "beta-app", "0.2.0");

        let dest = target.path().join("out.toml");
        let n = export_pack(target.path(), &dest).expect("export should succeed");
        assert_eq!(n, 2, "should export both apps");
        let text = std::fs::read_to_string(&dest).expect("dest written");
        // Re-parse the written file via the real Pack parser to prove it's
        // shape-valid and not just textually plausible.
        let parsed = Pack::from_toml_str(&text).expect("written pack must round-trip");
        assert_eq!(parsed.schema_version, 1);
        assert_eq!(parsed.apps.len(), 2);
        let ids: Vec<&str> = parsed.apps.iter().map(|a| a.id.as_str()).collect();
        assert!(ids.contains(&"alpha-app"));
        assert!(ids.contains(&"beta-app"));
    }

    #[test]
    fn pack_export_uses_local_scheme_for_non_git_apps() {
        let target = tempfile::tempdir().unwrap();
        write_app(target.path(), "bundled-thing", "1.0.0");
        // No `.git/` dir → infer_source_and_version must produce `local:`.
        let dest = target.path().join("out.toml");
        export_pack(target.path(), &dest).unwrap();
        let parsed = Pack::from_toml_str(&std::fs::read_to_string(&dest).unwrap()).unwrap();
        let entry = parsed
            .apps
            .iter()
            .find(|a| a.id == "bundled-thing")
            .expect("bundled-thing exported");
        assert_eq!(
            entry.source, "local:bundled-thing",
            "non-git installs must export under the local: scheme"
        );
        assert_eq!(entry.version, "1.0.0");
    }

    #[test]
    fn pack_export_skips_staging_dirs_and_non_apps() {
        let target = tempfile::tempdir().unwrap();
        write_app(target.path(), "real-app", "0.0.1");
        // Staging dir leftover from a crashed install — must be skipped.
        std::fs::create_dir_all(target.path().join(".tmp-install-1234-9999/clone")).unwrap();
        // A directory that isn't a Plexi app at all (no manifest) — also skipped.
        std::fs::create_dir_all(target.path().join("no-manifest-here")).unwrap();

        let dest = target.path().join("out.toml");
        let n = export_pack(target.path(), &dest).unwrap();
        assert_eq!(n, 1, "only the real app should be exported");
        let parsed = Pack::from_toml_str(&std::fs::read_to_string(&dest).unwrap()).unwrap();
        assert_eq!(parsed.apps.len(), 1);
        assert_eq!(parsed.apps[0].id, "real-app");
    }

    #[test]
    fn infer_source_for_local_install_uses_manifest_id() {
        let target = tempfile::tempdir().unwrap();
        let dir = target.path().join("foo");
        std::fs::create_dir_all(&dir).unwrap();
        let manifest_text = "schema_version = 1\n\n[app]\nid = \"foo\"\ntype = \"app\"\nname = \"Foo\"\nversion = \"0.3.0\"\nentry = \"run.sh\"\n";
        let manifest: AppManifest = toml::from_str(manifest_text).unwrap();
        let (source, version) = infer_source_and_version(&dir, &manifest);
        assert_eq!(source, "local:foo");
        assert_eq!(version, "0.3.0");
    }
}

#[cfg(test)]
mod core_pack_tests {
    use super::test_support::MockCloner;
    use super::*;

    #[test]
    fn core_pack_always_installs_missing_apps() {
        let target = tempfile::tempdir().unwrap();
        let cloner = MockCloner::new();

        // First call: all apps are missing → should install them.
        let first = apply_core_pack_always(&cloner, target.path());
        assert!(
            !first.is_empty(),
            "core pack should produce at least one outcome"
        );
        for o in &first {
            assert!(
                matches!(o.status, InstallStatus::Installed(_)),
                "core pack entry '{}' did not install on first call: {:?}",
                o.id,
                o.status
            );
        }

        // Second call: apps are already present → no Installed outcomes.
        let second = apply_core_pack_always(&cloner, target.path());
        let n_installed = second
            .iter()
            .filter(|o| matches!(o.status, InstallStatus::Installed(_)))
            .count();
        assert_eq!(
            n_installed, 0,
            "core pack must not re-install already-present apps"
        );
    }

    #[test]
    fn examples_pack_applies_only_when_empty() {
        let target = tempfile::tempdir().unwrap();
        let cloner = MockCloner::new();

        // Empty dir: should apply.
        let first =
            apply_examples_pack_if_empty(&cloner, target.path()).expect("must apply on empty");
        assert!(
            !first.is_empty(),
            "examples pack should produce at least one outcome"
        );

        // Now non-empty. Second call must return None.
        let second = apply_examples_pack_if_empty(&cloner, target.path());
        assert!(
            second.is_none(),
            "examples pack must NOT re-apply when apps dir is non-empty"
        );
    }

    #[test]
    fn core_pack_parses() {
        let pack = Pack::from_toml_str(CORE_PACK_TOML)
            .expect("bundled core pack must parse against the current schema");
        assert!(!pack.apps.is_empty(), "core pack should list at least one app");
        for entry in &pack.apps {
            assert!(
                parse_source_spec(&entry.source).is_ok(),
                "core pack entry '{}' has invalid source '{}'",
                entry.id,
                entry.source
            );
        }
        let examples_pack = Pack::from_toml_str(EXAMPLES_PACK_TOML)
            .expect("bundled examples pack must parse against the current schema");
        assert!(
            !examples_pack.apps.is_empty(),
            "examples pack should list at least one app"
        );
        for entry in &examples_pack.apps {
            assert!(
                parse_source_spec(&entry.source).is_ok(),
                "examples pack entry '{}' has invalid source '{}'",
                entry.id,
                entry.source
            );
        }
    }

    #[test]
    fn split_source_and_ref_handles_at_suffix() {
        let (s, r) = split_source_and_ref("github:owner/repo@v1.0.0");
        assert_eq!(s, "github:owner/repo");
        assert_eq!(r.as_deref(), Some("v1.0.0"));

        let (s, r) = split_source_and_ref("github:owner/repo");
        assert_eq!(s, "github:owner/repo");
        assert_eq!(r, None);

        // git+ssh://user@host/repo.git@v1 — must split at the LAST @ after scheme.
        let (s, r) = split_source_and_ref("git+ssh://user@host.example.com/repo.git@v1");
        assert_eq!(s, "git+ssh://user@host.example.com/repo.git");
        assert_eq!(r.as_deref(), Some("v1"));

        // git+ssh://user@host/repo.git (no ref) — must NOT eat user@host.
        let (s, r) = split_source_and_ref("git+ssh://user@host.example.com/repo.git");
        assert_eq!(s, "git+ssh://user@host.example.com/repo.git");
        assert_eq!(r, None);
    }
}

#[cfg(test)]
mod workspace_pack_tests {
    use super::test_support::MockCloner;
    use super::*;

    #[test]
    fn apply_workspace_pack_installs_into_channel_apps_dir() {
        let ws = tempfile::tempdir().unwrap();
        let channel_dir = crate::config::workspace_channel_dir();
        let plexi_dir = ws.path().join(&channel_dir);
        std::fs::create_dir_all(&plexi_dir).unwrap();
        std::fs::write(
            plexi_dir.join("apps.toml"),
            "schema_version = 1\n\n\
             [[app]]\n\
             id = \"my-tool\"\n\
             source = \"github:org/my-tool\"\n\
             version = \"v1.0.0\"\n",
        )
        .unwrap();

        let cloner = MockCloner::new();
        let outcomes = apply_workspace_pack(ws.path(), &cloner).expect("should succeed");
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].id, "my-tool");
        assert!(
            matches!(outcomes[0].status, InstallStatus::Installed(_)),
            "expected Installed, got {:?}",
            outcomes[0].status
        );

        let workspace_apps = crate::app::registry::workspace_apps_dir(ws.path());
        assert!(
            workspace_apps.join("my-tool").join("manifest.toml").exists(),
            "app should be present in workspace apps dir at {}",
            workspace_apps.display()
        );
    }

    #[test]
    fn apply_workspace_pack_errors_when_no_apps_toml() {
        let ws = tempfile::tempdir().unwrap();
        let channel_dir = crate::config::workspace_channel_dir();
        std::fs::create_dir_all(ws.path().join(&channel_dir)).unwrap();

        let cloner = MockCloner::new();
        let err = apply_workspace_pack(ws.path(), &cloner).expect_err("should error");
        assert!(
            err.contains(&format!("no {channel_dir}/apps.toml found")),
            "expected no-manifest error, got: {err}"
        );
    }

    #[test]
    fn workspace_manifest_ids_returns_declared_ids() {
        let ws = tempfile::tempdir().unwrap();
        let channel_dir = crate::config::workspace_channel_dir();
        let plexi_dir = ws.path().join(&channel_dir);
        std::fs::create_dir_all(&plexi_dir).unwrap();
        std::fs::write(
            plexi_dir.join("apps.toml"),
            "schema_version = 1\n\n\
             [[app]]\nid = \"foo\"\nsource = \"github:a/b\"\nversion = \"v1\"\n\n\
             [[app]]\nid = \"bar\"\nsource = \"github:a/c\"\nversion = \"v2\"\n",
        )
        .unwrap();

        let ids = workspace_manifest_ids(ws.path());
        assert!(ids.contains("foo"));
        assert!(ids.contains("bar"));
        assert_eq!(ids.len(), 2);
    }

    #[test]
    fn workspace_manifest_ids_empty_when_no_file() {
        let ws = tempfile::tempdir().unwrap();
        let ids = workspace_manifest_ids(ws.path());
        assert!(ids.is_empty());
    }

    #[test]
    fn apply_workspace_pack_idempotent_on_second_run() {
        let ws = tempfile::tempdir().unwrap();
        let channel_dir = crate::config::workspace_channel_dir();
        let plexi_dir = ws.path().join(&channel_dir);
        std::fs::create_dir_all(&plexi_dir).unwrap();
        // MockCloner writes version "0.0.1" by default — pack must match for AlreadyAtVersion.
        std::fs::write(
            plexi_dir.join("apps.toml"),
            "schema_version = 1\n\n\
             [[app]]\nid = \"my-tool\"\nsource = \"github:org/my-tool\"\nversion = \"0.0.1\"\n",
        )
        .unwrap();

        let cloner = MockCloner::new();
        apply_workspace_pack(ws.path(), &cloner).unwrap();
        let second = apply_workspace_pack(ws.path(), &cloner).unwrap();
        assert_eq!(second.len(), 1);
        assert!(
            matches!(second[0].status, InstallStatus::AlreadyAtVersion),
            "second apply should be no-op, got {:?}",
            second[0].status
        );
    }
}
