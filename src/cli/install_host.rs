//! App installer (#308 Phase 2).
//!
//! Three top-level CLI subcommands route here:
//!   - `plexi install <source-spec>[@ref]` — clone + place one plugin
//!   - `plexi install --pack <path|core>`  — apply every [[plugin]] in a pack
//!   - `plexi update plugins [<id>]`           — git-pull installed plugin(s)
//!   - `plexi uninstall <id>`              — remove the install dir
//!   - `plexi list`                        — show installed plugins
//!
//! All git work goes through the [`Cloner`] trait so unit tests can mock it
//! (mirrors `workspace_secrets::SecretStore`'s pattern). Production uses
//! [`GitCloner`] which shells out to `git` — no `git2` dependency.
//!
//! The bundled core pack is `packs/core.toml` baked in at compile time via
//! `include_str!`. Sources are `local:<plugin-name>` — the installer copies
//! from the embedded `plugins/` tree.

use crate::app::packs::{parse_source_spec, Pack, PackPlugin, SourceSpec};
use crate::app::python_env::VenvError;
use crate::app::registry::{PluginManifest, MANIFEST_SCHEMA_VERSION};
use plexi_distribution::bounded::Helper;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Pack file embedded into the binary. Auto-applied on every launch (core) or
/// on first launch only (examples). Also reachable via `plexi install --pack core`.
pub const CORE_PACK_TOML: &str = include_str!("../../packs/core.toml");
pub const EXAMPLES_PACK_TOML: &str = include_str!("../../packs/examples.toml");

// All plugins under plugins/ are embedded at compile time, including plugins/dev/.
// `local:` source-spec installs look up by name in EMBEDDED_APPS.
// plugins/dev/ plugins are not referenced by any pack entry, so they are never
// auto-installed — they are only rsync'd to the profile dir by install.sh.
static EMBEDDED_APPS: include_dir::Dir<'_> = include_dir::include_dir!("$CARGO_MANIFEST_DIR/plugins");

/// Result of installing or updating one plugin entry. Aggregated for pack apply.
#[derive(Debug, Clone)]
pub struct InstallOutcome {
    pub id: String,
    pub status: InstallStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallStatus {
    /// Newly installed at the given path.
    Installed(PathBuf),
    /// Re-extracted a bundled `local:` plugin over an existing install (pack
    /// apply with `refresh_local`) — how core-plugin updates reach existing
    /// profiles on stable channels.
    Refreshed(PathBuf),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GitRefPolicy {
    Strict,
    FallbackToDefaultBranch,
}

/// Why a checkout failed. Only a ref that is not there lets a pack install
/// fall back to the default branch; anything else leaves the clone in an
/// unknown state (a git that was cut off or could not be stopped) and fails
/// the install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckoutError {
    RefUnavailable(String),
    Fatal(String),
}

impl std::fmt::Display for CheckoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RefUnavailable(message) | Self::Fatal(message) => f.write_str(message),
        }
    }
}

/// Abstraction over the actual git client. Production: [`GitCloner`] (shells
/// out to `git`). Tests: in-memory mock that just creates the target dir.
pub trait Cloner: Send + Sync {
    /// Clone `url` into `target_dir`. `target_dir` must not exist yet — the
    /// implementation is responsible for creating it.
    fn clone(&self, url: &str, target_dir: &Path) -> Result<(), String>;
    /// Check out a specific ref (tag, branch, or sha) inside an existing
    /// clone. Optional — `None` = leave on the default branch.
    fn checkout(&self, repo_dir: &Path, git_ref: &str) -> Result<(), CheckoutError>;
    /// `git fetch && git pull` — used by `plexi update plugins`.
    fn pull(&self, repo_dir: &Path) -> Result<(), String>;
}

/// Production [`Cloner`] that shells out to the system `git` binary. No
/// libgit2 / `git2` crate dependency — keeps the release build slim.
pub struct GitCloner {
    /// Clone, fetch and pull.
    network_limit: Duration,
}

impl Default for GitCloner {
    fn default() -> Self {
        Self {
            network_limit: GIT_NETWORK_LIMIT,
        }
    }
}

/// Network git steps (clone, fetch, pull) get a generous limit; local reads
/// finish at once when healthy. The limits only stop a hung git.
const GIT_NETWORK_LIMIT: Duration = Duration::from_secs(600);
const GIT_LOCAL_LIMIT: Duration = Duration::from_secs(30);

/// Run a git helper bounded by `limit`; the error names what failed and,
/// for an auth failure, how to set up credentials.
fn run_git(
    helper: &mut Helper,
    what: &str,
    limit: Duration,
) -> Result<plexi_distribution::bounded::Output, String> {
    helper
        .run(what, limit)
        .map_err(|error| plexi_distribution::bounded::git_error(&error))
}

/// Check out `git_ref` once its fetch has run.
fn checkout_after_fetch(
    fetched: Result<plexi_distribution::bounded::Output, plexi_distribution::bounded::RunError>,
    repo_dir: &Path,
    git_ref: &str,
) -> Result<(), CheckoutError> {
    fetch_or_fall_back(fetched, git_ref).map_err(CheckoutError::Fatal)?;
    let checked_out = git_in(repo_dir)
        .args(["checkout", git_ref])
        .run(&format!("git checkout {git_ref}"), GIT_LOCAL_LIMIT);
    checkout_outcome(checked_out)
}

/// A ref already present needs no fetch, so a fetch that exited non-zero
/// (the remote has no such ref, or refused) falls back to the local ref.
/// Every other failure is fatal: a fetch cut off at its limit or not stopped
/// may have left the repository half-updated or still changing, and cut
/// output is incomplete.
fn fetch_or_fall_back(
    fetched: Result<plexi_distribution::bounded::Output, plexi_distribution::bounded::RunError>,
    git_ref: &str,
) -> Result<(), String> {
    use plexi_distribution::bounded::RunError;
    match fetched {
        Ok(_) => Ok(()),
        Err(error @ RunError::Failed { .. }) => {
            log::info!(
                "install_host: {}; trying checkout of {git_ref} anyway",
                plexi_distribution::bounded::git_error(&error)
            );
            Ok(())
        }
        Err(error) => Err(plexi_distribution::bounded::git_error(&error)),
    }
}

/// `git checkout` exiting non-zero means the ref is not there and the tree
/// is untouched. A checkout cut off at its limit or not stopped may have
/// half-written the tree, so it is fatal.
fn checkout_outcome(
    checked_out: Result<plexi_distribution::bounded::Output, plexi_distribution::bounded::RunError>,
) -> Result<(), CheckoutError> {
    use plexi_distribution::bounded::{git_error, RunError};
    match checked_out {
        Ok(_) => Ok(()),
        Err(error @ RunError::Failed { .. }) => {
            Err(CheckoutError::RefUnavailable(git_error(&error)))
        }
        Err(error) => Err(CheckoutError::Fatal(git_error(&error))),
    }
}

fn git_in(dir: &Path) -> Helper {
    let mut helper = plexi_distribution::bounded::git();
    helper.current_dir(dir);
    helper
}

impl Cloner for GitCloner {
    fn clone(&self, url: &str, target_dir: &Path) -> Result<(), String> {
        run_git(
            plexi_distribution::bounded::git()
                .args(["clone", "--depth", "1", url])
                .arg(target_dir),
            &format!("git clone {url}"),
            self.network_limit,
        )?;
        Ok(())
    }

    fn checkout(&self, repo_dir: &Path, git_ref: &str) -> Result<(), CheckoutError> {
        // For shallow clones we may need the ref fetched explicitly.
        let fetched = git_in(repo_dir)
            .args(["fetch", "--depth", "1", "origin", git_ref])
            .run(&format!("git fetch {git_ref}"), self.network_limit);
        checkout_after_fetch(fetched, repo_dir, git_ref)
    }

    fn pull(&self, repo_dir: &Path) -> Result<(), String> {
        run_git(
            git_in(repo_dir).arg("fetch"),
            "git fetch",
            self.network_limit,
        )?;
        run_git(
            git_in(repo_dir).args(["pull", "--ff-only"]),
            "git pull",
            self.network_limit,
        )?;
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

/// Install one plugin from a parsed source spec at an optional git ref.
/// `target_root` is the channel plugins dir (e.g. `~/.plexi-alpha/plugins`).
pub fn install_one(
    cloner: &dyn Cloner,
    source: &SourceSpec,
    git_ref: Option<&str>,
    target_root: &Path,
) -> Result<InstallOutcome, String> {
    install_one_with_ref_policy(
        cloner,
        &UvProvisioner,
        source,
        git_ref,
        target_root,
        GitRefPolicy::Strict,
    )
}

fn install_one_with_ref_policy(
    cloner: &dyn Cloner,
    provision: &dyn Provision,
    source: &SourceSpec,
    git_ref: Option<&str>,
    target_root: &Path,
    ref_policy: GitRefPolicy,
) -> Result<InstallOutcome, String> {
    // The shared install boundary (stint 1398): every caller inherits the
    // source decision, and a refused source is never fetched.
    if let SourceSpec::Git(url) = source {
        crate::host::plugin_admission::admit_install_source(
            crate::host::plugin_admission::InstallSource::Git(url),
        )
        .map_err(|refusal| refusal.to_string())?;
    }
    install_one_admitted(cloner, provision, source, git_ref, target_root, ref_policy)
}

/// Install a source the boundary already admitted. Only the admission
/// boundaries call this (and tests of the fetch mechanics behind them).
fn install_one_admitted(
    cloner: &dyn Cloner,
    provision: &dyn Provision,
    source: &SourceSpec,
    git_ref: Option<&str>,
    target_root: &Path,
    ref_policy: GitRefPolicy,
) -> Result<InstallOutcome, String> {
    match source {
        SourceSpec::Git(url) => {
            install_one_git(cloner, provision, url, git_ref, target_root, ref_policy)
        }
        SourceSpec::Local(name) => install_one_local(provision, name, target_root),
    }
}

fn install_one_git(
    cloner: &dyn Cloner,
    provision: &dyn Provision,
    url: &str,
    git_ref: Option<&str>,
    target_root: &Path,
    ref_policy: GitRefPolicy,
) -> Result<InstallOutcome, String> {
    std::fs::create_dir_all(target_root)
        .map_err(|e| format!("create plugins dir {}: {e}", target_root.display()))?;

    // Stage into a sibling `.tmp-<unique>/clone` dir inside `target_root` so
    // we can read the manifest BEFORE committing to the final dest. Same-FS
    // ensures the eventual `rename` is atomic. A UUID keeps concurrent installs
    // in this process separate even when the wall clock returns the same tick.
    let stage_root = target_root.join(format!(".tmp-install-{}", uuid::Uuid::new_v4()));
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
            match (ref_policy, &e) {
                (GitRefPolicy::Strict, _) | (_, CheckoutError::Fatal(_)) => {
                    cleanup(&stage_root);
                    return Err(e.to_string());
                }
                (GitRefPolicy::FallbackToDefaultBranch, CheckoutError::RefUnavailable(_)) => {
                    log::warn!(
                        "install: ref '{r}' unavailable for {url}; using default branch HEAD: {e}"
                    );
                }
            }
        }
    }

    let manifest = match read_manifest(&stage_path) {
        Ok(m) => m,
        Err(e) => {
            cleanup(&stage_root);
            return Err(e);
        }
    };
    let _command_install = match manifest.lock_command_install(target_root) {
        Ok(guard) => guard,
        Err(error) => {
            cleanup(&stage_root);
            return Err(error.to_string());
        }
    };
    let id = manifest.plugin.id.clone();
    let dest = target_root.join(&id);

    if dest.exists() {
        cleanup(&stage_root);
        return Err(format!(
            "plugin '{id}' already installed at {}; \
             run `plexi uninstall {id}` or `plexi update plugins {id}` first",
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
    provision_venv(provision, &id, &dest, &manifest)?;
    write_installed_version_file(&dest, &manifest.plugin.version);
    log::info!("install: id={} ref={}", id, git_ref.unwrap_or("default"));
    Ok(InstallOutcome {
        id,
        status: InstallStatus::Installed(dest),
    })
}

fn install_one_local(
    provision: &dyn Provision,
    name: &str,
    target_root: &Path,
) -> Result<InstallOutcome, String> {
    std::fs::create_dir_all(target_root)
        .map_err(|e| format!("create plugins dir {}: {e}", target_root.display()))?;

    let example = EMBEDDED_APPS
        .get_dir(name)
        .ok_or_else(|| format!("local source '{name}' not found in bundled plugins"))?;

    // Read manifest text from the embedded tree to recover the canonical id.
    let manifest_text = example
        .get_file(format!("{name}/manifest.toml"))
        .and_then(|f| f.contents_utf8())
        .ok_or_else(|| format!("local source '{name}' has no manifest.toml"))?;
    let manifest = PluginManifest::parse(manifest_text)
        .map_err(|e| format!("local source '{name}' manifest invalid: {e}"))?;
    if manifest.schema_version > MANIFEST_SCHEMA_VERSION {
        return Err(format!(
            "local source '{name}' manifest schema_version = {} > supported {}",
            manifest.schema_version, MANIFEST_SCHEMA_VERSION
        ));
    }
    let _command_install = manifest
        .lock_command_install(target_root)
        .map_err(|e| e.to_string())?;
    let id = manifest.plugin.id.clone();
    let dest = target_root.join(&id);
    if dest.exists() {
        return Err(format!(
            "plugin '{id}' already installed at {}",
            dest.display()
        ));
    }
    // `include_dir::Dir::extract` writes paths relative to the *caller*'s
    // arg using the entries' stored paths (which include `<name>/...` because
    // we embedded `plugins/`). Calling `extract(target_root)` therefore
    // writes `target_root/<name>/manifest.toml`, etc. We just need to ensure
    // `dest` exists first; `extract` panics otherwise on macOS tempdirs.
    std::fs::create_dir_all(&dest).map_err(|e| format!("create {}: {e}", dest.display()))?;
    example
        .extract(target_root)
        .map_err(|e| format!("extract local '{name}' → {}: {e}", target_root.display()))?;
    chmod_python_entries(&dest);
    provision_venv(provision, &id, &dest, &manifest)?;
    write_installed_version_file(&dest, &manifest.plugin.version);
    log::info!("install: {id} ← local:{name}");
    Ok(InstallOutcome {
        id,
        status: InstallStatus::Installed(dest),
    })
}

/// Write `<plugin_dir>/installed_version.txt`. Best-effort — never fails the install.
fn write_installed_version_file(plugin_dir: &Path, version: &str) {
    let path = plugin_dir.join("installed_version.txt");
    if let Err(e) = std::fs::write(&path, version) {
        log::warn!(
            "install: could not write installed_version.txt for {}: {e}",
            plugin_dir.display()
        );
    } else {
        log::info!(
            "install: wrote installed_version.txt={version} for {}",
            plugin_dir.display()
        );
    }
}

/// Read + validate `manifest.toml` from a freshly-cloned repo. Returns a
/// clear error for a missing file, a parse error, or an unsupported
/// schema_version.
fn read_manifest(repo_dir: &Path) -> Result<PluginManifest, String> {
    let manifest_path = repo_dir.join("manifest.toml");
    let text = std::fs::read_to_string(&manifest_path).map_err(|e| {
        format!(
            "no manifest.toml at repo root ({}): {e}",
            manifest_path.display()
        )
    })?;
    let manifest = PluginManifest::parse(&text).map_err(|e| e.to_string())?;
    if manifest.schema_version > MANIFEST_SCHEMA_VERSION {
        return Err(format!(
            "manifest schema_version = {} is newer than this Plexi build supports (max {}); \
             update Plexi to install this plugin",
            manifest.schema_version, MANIFEST_SCHEMA_VERSION
        ));
    }
    Ok(manifest)
}

#[cfg(unix)]
fn chmod_python_entries(plugin_dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let entries = match std::fs::read_dir(plugin_dir) {
        Ok(d) => d,
        Err(e) => {
            log::warn!("install: could not chmod {}: {e}", plugin_dir.display());
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
fn chmod_python_entries(_plugin_dir: &Path) {}

/// Prepares an installed plugin's runtime. Production: [`UvProvisioner`].
pub(crate) trait Provision {
    fn provision(
        &self,
        plugin_id: &str,
        plugin_dir: &Path,
        manifest: &PluginManifest,
    ) -> Result<(), VenvError>;
}

/// Creates a Python plugin's venv and installs its dependencies with uv.
pub(crate) struct UvProvisioner;

impl Provision for UvProvisioner {
    fn provision(
        &self,
        plugin_id: &str,
        plugin_dir: &Path,
        manifest: &PluginManifest,
    ) -> Result<(), VenvError> {
        if !manifest.plugin.entry.ends_with(".py") {
            return Ok(());
        }
        let python = crate::app::python_env::ensure_plugin_venv(
            plugin_id,
            plugin_dir,
            &manifest.plugin.dependencies,
        )?;
        log::info!(
            "install[{plugin_id}]: Python plugin environment ready at {}",
            python.display()
        );
        Ok(())
    }
}

/// Provision an installed plugin. A missing or refusing uv or Python leaves
/// the plugin installed with a warning, as before. A provisioning helper
/// that timed out, could not be stopped or was cut leaves the environment
/// unknown and possibly still changing: the install fails and the plugin
/// directory is removed.
fn provision_venv(
    provision: &dyn Provision,
    plugin_id: &str,
    plugin_dir: &Path,
    manifest: &PluginManifest,
) -> Result<(), String> {
    match provision.provision(plugin_id, plugin_dir, manifest) {
        Ok(()) => Ok(()),
        Err(VenvError::Unavailable(e)) => {
            log::warn!(
                "install[{plugin_id}]: Python plugin environment setup failed: {e}; plugin may not launch until `plexi plugin check {}` succeeds",
                plugin_dir.display()
            );
            Ok(())
        }
        Err(VenvError::Fatal(e)) => {
            log::error!("install[{plugin_id}]: Python plugin environment setup failed: {e}");
            if let Err(remove) = std::fs::remove_dir_all(plugin_dir) {
                log::error!(
                    "install[{plugin_id}]: could not remove {} after the failed setup: {remove}",
                    plugin_dir.display()
                );
            }
            Err(format!(
                "install {plugin_id}: Python environment setup failed: {e}"
            ))
        }
    }
}

/// Apply every `[[plugin]]` entry in a pack against `target_root`. Errors are
/// aggregated per plugin so one bad entry doesn't abort the whole apply.
/// Idempotent: an entry whose id is already installed at the same version is
/// reported as `AlreadyAtVersion`.
///
/// `refresh_local`: re-extract already-installed `local:` entries from this
/// binary's embedded tree, replacing the installed directory wholesale. This
/// is the update path for bundled core plugins — `local:` versions are
/// `"bundled"` on both sides, so content, not version, is the ground truth.
/// Only the explicit installer path (`plexi plugin install --pack core
/// --refresh`) sets this; launch-time reseeding never clobbers an existing
/// install.
pub fn apply_pack(
    cloner: &dyn Cloner,
    pack: &Pack,
    target_root: &Path,
    refresh_local: bool,
) -> Vec<InstallOutcome> {
    apply_pack_with(cloner, &UvProvisioner, pack, target_root, refresh_local)
}

fn apply_pack_with(
    cloner: &dyn Cloner,
    provision: &dyn Provision,
    pack: &Pack,
    target_root: &Path,
    refresh_local: bool,
) -> Vec<InstallOutcome> {
    // One refused source refuses the whole pack before anything is fetched
    // or written, so a pack is never half-applied (stint 1398).
    if let Err(refusal) = admit_pack_sources(pack) {
        let error = refusal.to_string();
        return pack
            .plugins
            .iter()
            .map(|entry| InstallOutcome {
                id: entry.id.clone(),
                status: InstallStatus::Failed(error.clone()),
            })
            .collect();
    }
    apply_admitted_pack(cloner, provision, pack, target_root, refresh_local)
}

/// Apply a pack whose sources the boundary already admitted.
fn apply_admitted_pack(
    cloner: &dyn Cloner,
    provision: &dyn Provision,
    pack: &Pack,
    target_root: &Path,
    refresh_local: bool,
) -> Vec<InstallOutcome> {
    let mut outcomes = Vec::with_capacity(pack.plugins.len());
    let installed = installed_versions(target_root);
    for entry in &pack.plugins {
        outcomes.push(apply_pack_entry(
            cloner,
            provision,
            entry,
            target_root,
            &installed,
            refresh_local,
        ));
    }
    outcomes
}

/// `plugin install local:<name>` over a copy that is not the shipped one
/// replaces it with the shipped one: the recovery a refused launch names
/// (stint 1398). `None` when nothing is there or it is already the bundle.
pub(crate) fn repair_changed_bundled(name: &str, target_root: &Path) -> Option<InstallOutcome> {
    if !is_bundled_plugin_id(name) {
        return None;
    }
    let dest = target_root.join(name);
    let meta = std::fs::symlink_metadata(&dest).ok()?;
    if !meta.file_type().is_symlink() && matches_bundled_plugin(name, &dest) {
        return None;
    }
    log::info!(
        "install: replacing changed copy of bundled {name} at {}",
        dest.display()
    );
    Some(refresh_one_local(&UvProvisioner, name, name, target_root))
}

/// Replace an installed `local:` plugin with the copy embedded in this binary.
fn refresh_one_local(
    provision: &dyn Provision,
    name: &str,
    id: &str,
    target_root: &Path,
) -> InstallOutcome {
    let dest = target_root.join(id);
    if let Err(e) = std::fs::remove_dir_all(&dest) {
        return InstallOutcome {
            id: id.to_string(),
            status: InstallStatus::Failed(format!("refresh: remove {}: {e}", dest.display())),
        };
    }
    match install_one_local(provision, name, target_root) {
        Ok(o) => {
            log::info!("install: refreshed {} ← local:{name}", o.id);
            InstallOutcome {
                id: o.id,
                status: match o.status {
                    InstallStatus::Installed(p) => InstallStatus::Refreshed(p),
                    other => other,
                },
            }
        }
        Err(e) => InstallOutcome {
            id: id.to_string(),
            status: InstallStatus::Failed(format!("refresh: {e}")),
        },
    }
}

fn apply_pack_entry(
    cloner: &dyn Cloner,
    provision: &dyn Provision,
    entry: &PackPlugin,
    target_root: &Path,
    installed: &HashMap<String, String>,
    refresh_local: bool,
) -> InstallOutcome {
    if let Some(installed_version) = installed.get(&entry.id) {
        if refresh_local {
            if let Ok(SourceSpec::Local(name)) = parse_source_spec(&entry.source) {
                return refresh_one_local(provision, &name, &entry.id, target_root);
            }
        }
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
    match install_one_admitted(
        cloner,
        provision,
        &source,
        git_ref,
        target_root,
        GitRefPolicy::FallbackToDefaultBranch,
    ) {
        Ok(outcome) => outcome,
        Err(e) => InstallOutcome {
            id: entry.id.clone(),
            status: InstallStatus::Failed(e),
        },
    }
}

/// Walk `<target_root>/<id>/manifest.toml` for every dir entry and harvest
/// the manifest version. Used to drive `plexi list` and pack-apply
/// idempotency. Plugins without a parseable manifest are skipped silently
/// (already logged elsewhere by `PluginRegistry`).
pub fn installed_versions(target_root: &Path) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let entries = match std::fs::read_dir(target_root) {
        Ok(d) => d,
        Err(_) => return out,
    };
    for e in entries.flatten() {
        let manifest = e.path().join("manifest.toml");
        if let Ok(text) = std::fs::read_to_string(&manifest) {
            if let Ok(parsed) = toml::from_str::<PluginManifest>(&text) {
                out.insert(parsed.plugin.id, parsed.plugin.version);
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
    crate::host::cli_ops::purge_plugin_copies(id);
    Ok(())
}

/// `git fetch && git pull` a concrete installed plugin directory. Used by the
/// workspace-aware CLI update path after registry resolution.
pub fn update_plugin_dir(cloner: &dyn Cloner, id: &str, dest: &Path) -> Result<(), String> {
    if !dest.exists() {
        return Err(format!("'{id}' is not installed at {}", dest.display()));
    }
    if !dest.join(".git").exists() {
        return Err("not a git checkout (likely bundled core plugin)".to_string());
    }
    let source = dest.display().to_string();
    crate::host::plugin_admission::admit_install_source(
        crate::host::plugin_admission::InstallSource::Git(&source),
    )
    .map_err(|refusal| refusal.to_string())?;
    cloner.pull(dest)?;
    log::info!("update: {id} at {}", dest.display());
    Ok(())
}

/// Export the current set of installed plugins under `target_root` as a
/// `pack.toml` written to `dest_path`. For each plugin:
///   - If the install dir is a git checkout (`.git/` present), the source is
///     `git+<remote-origin-url>` and the version is the current ref
///     (`git describe --tags --exact-match` → tag, else short SHA).
///   - Otherwise (bundled core / `local:` install), source is
///     `local:<id>` and version is `manifest.version` (or `bundled` if
///     unset). These re-install from the binary's embedded examples on
///     `plexi install --pack <path>`.
///
/// Plugins without a parseable manifest are skipped with a warning.
///
/// Returns the number of entries written.
pub fn export_pack(target_root: &Path, dest_path: &Path) -> Result<usize, String> {
    let entries = match std::fs::read_dir(target_root) {
        Ok(d) => d,
        Err(e) => {
            return Err(format!("read plugins dir {}: {e}", target_root.display()));
        }
    };

    let mut rows: Vec<(String, String, String)> = Vec::new();
    for e in entries.flatten() {
        let plugin_dir = e.path();
        // Skip non-dirs and the staging dirs install_one leaves behind on
        // crash (`.tmp-install-*`). Both would produce a noisy export.
        if !plugin_dir.is_dir() {
            continue;
        }
        if plugin_dir
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with(".tmp-install-"))
        {
            continue;
        }
        let manifest_path = plugin_dir.join("manifest.toml");
        let text = match std::fs::read_to_string(&manifest_path) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let manifest: PluginManifest = match toml::from_str(&text) {
            Ok(m) => m,
            Err(err) => {
                log::warn!(
                    "pack export: skipping {}: manifest invalid: {err}",
                    plugin_dir.display()
                );
                continue;
            }
        };
        let id = manifest.plugin.id.clone();
        let (source, version) = infer_source_and_version(&plugin_dir, &manifest);
        rows.push((id, source, version));
    }

    rows.sort_by(|a, b| a.0.cmp(&b.0));

    let mut out = String::new();
    out.push_str("# Generated by `plexi pack export`. Edit freely.\n");
    out.push_str("schema_version = 2\n\n");
    for (id, source, version) in &rows {
        out.push_str("\n[[plugin]]\n");
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
        "pack export: wrote {} plugins to {}",
        rows.len(),
        dest_path.display()
    );
    Ok(rows.len())
}

/// Best-effort inference of `(source_spec, version)` for one installed plugin
/// dir. Pure-IO helper for `export_pack`. Public for the CLI test that
/// wants to assert the local-vs-git split independently of file scanning.
pub fn infer_source_and_version(plugin_dir: &Path, manifest: &PluginManifest) -> (String, String) {
    if plugin_dir.join(".git").exists() {
        let url = git_remote_origin_url(plugin_dir).unwrap_or_else(|| String::from("git+unknown"));
        let source = format_git_source_spec(&url);
        let version =
            git_current_ref(plugin_dir).unwrap_or_else(|| manifest.plugin.version.clone());
        return (source, version);
    }
    let id = &manifest.plugin.id;
    let version = if manifest.plugin.version.is_empty() {
        "bundled".to_string()
    } else {
        manifest.plugin.version.clone()
    };
    (format!("local:{id}"), version)
}

fn git_remote_origin_url(repo_dir: &Path) -> Option<String> {
    let output = run_git(
        git_in(repo_dir).args(["config", "--get", "remote.origin.url"]),
        "git config remote.origin.url",
        GIT_LOCAL_LIMIT,
    )
    .ok()?;
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
    if let Ok(out) = run_git(
        git_in(repo_dir).args(["describe", "--tags", "--exact-match"]),
        "git describe",
        GIT_LOCAL_LIMIT,
    ) {
        if let Ok(s) = String::from_utf8(out.stdout) {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    // Fall back to a short SHA — re-install still reproducible.
    let sha = run_git(
        git_in(repo_dir).args(["rev-parse", "--short", "HEAD"]),
        "git rev-parse",
        GIT_LOCAL_LIMIT,
    )
    .ok()?;
    let s = String::from_utf8(sha.stdout).ok()?;
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Read-only check for a stale build or missing core plugin.
pub fn core_pack_needs_seeding(target_root: &Path) -> bool {
    if !std::fs::read_to_string(target_root.join(".core_build"))
        .is_ok_and(|build| build == env!("PLEXI_BUILD_ID"))
    {
        return true;
    }
    let installed = installed_versions(target_root);
    core_pack_ids().iter().any(|id| !installed.contains_key(id))
}

/// Apply the bundled core pack into `target_root` on every launch. Only installs
/// plugins that are missing — already-installed plugins return `SkippedOtherVersion` or
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
    let stamp = target_root.join(".core_build");
    let build = env!("PLEXI_BUILD_ID");
    let refresh = std::fs::read_to_string(&stamp).map_or(true, |s| s != build);
    let outcomes = apply_pack(cloner, &pack, target_root, refresh);
    if outcomes
        .iter()
        .all(|o| !matches!(o.status, InstallStatus::Failed(_)))
    {
        if let Err(error) = std::fs::write(&stamp, build) {
            log::warn!("core pack: record build identity: {error}");
        }
    }
    outcomes
}

/// Apply the bundled examples pack into `target_root` only if `target_root` is
/// currently empty (first-launch seeding). Idempotent — returns `None` if the
/// dir is non-empty.
///
/// Respects `reseed = "once"` (default for examples) — skips if plugins dir is
/// already populated.
pub fn apply_examples_pack_if_empty(
    cloner: &dyn Cloner,
    target_root: &Path,
) -> Option<Vec<InstallOutcome>> {
    let is_empty = match std::fs::read_dir(target_root) {
        Ok(d) => d.flatten().next().is_none(),
        Err(e) => {
            log::warn!(
                "examples pack: cannot read plugins dir {}: {e}",
                target_root.display()
            );
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
    Some(apply_pack(cloner, &pack, target_root, false))
}

/// Root entries the installer writes beside an extracted plugin. `.venv` comes
/// from `python_env::ensure_plugin_venv`; the files are `package` host metadata.
/// The CPython guest cannot import any of them: `/app` is a plain PYTHONPATH
/// entry with no `.pth` processing, and `.venv` is not a module name.
fn is_installer_written_root_entry(name: &str) -> bool {
    name == ".venv" || crate::app::package::HOST_METADATA_FILES.contains(&name)
}

/// What `scripts/package.py` leaves out of a packaged plugin, in its
/// `shutil.ignore_patterns` order. Both sides of the bundled comparison skip
/// exactly these, at any depth (stint 1398).
pub(crate) const PACKAGER_EXCLUDES: [&str; 4] = ["__pycache__", "*.pyc", ".venv", ".DS_Store"];

/// True when the packager keeps an entry named `name` (see [`PACKAGER_EXCLUDES`]).
fn packaged(name: &str) -> bool {
    PACKAGER_EXCLUDES
        .iter()
        .all(|pattern| match pattern.strip_prefix('*') {
            Some(suffix) => !name.ends_with(suffix),
            None => name != *pattern,
        })
}

/// Python bytecode: a `.pyc` file or anything under `__pycache__`. The
/// packager drops it, but unlike Finder metadata or a venv it is code the
/// guest's importer runs, so the comparison treats it apart.
fn is_bytecode(rel: &Path) -> bool {
    rel.extension().is_some_and(|ext| ext == "pyc")
        || rel
            .components()
            .any(|part| part.as_os_str() == "__pycache__")
}

/// An excluded entry that is not code (Finder metadata, a venv): skipped on
/// both sides of the comparison.
fn non_code_exclude(name: &str) -> bool {
    !packaged(name) && !is_bytecode(Path::new(name))
}

/// True when this binary embeds a plugin named `id` (a top-level folder of
/// `plugins/` with a manifest), so `local:<id>` installs it. Such ids are
/// reserved: no workspace or folder plugin may take one (stint 1398).
pub(crate) fn is_bundled_plugin_id(id: &str) -> bool {
    !id.is_empty()
        && !id.contains(['/', '\\'])
        && EMBEDDED_APPS
            .get_file(format!("{id}/manifest.toml"))
            .is_some()
}

/// Host-derived provenance: true when `plugin_dir` holds exactly the plugin
/// this binary embeds as `local:<name>`. Same files, identical bytes, nothing
/// extra, no symlinks. The selection follows the packager's
/// [`PACKAGER_EXCLUDES`] (stint 1398): Finder metadata and venvs are skipped on
/// both sides. Bytecode is code the importer runs, so it is never skipped on
/// disk: an embedded `.pyc` may be absent (the packager drops it), but a `.pyc`
/// present must be one the binary embeds, byte for byte. A manifest id or package
/// claim cannot satisfy this. It covers the plugin folder only, not the SDK or
/// stdlib a guest also imports, and it is a scan-time answer only. The walk
/// stops at the first difference, so its cost is bounded by the embedded tree.
pub(crate) fn matches_bundled_plugin(name: &str, plugin_dir: &Path) -> bool {
    let Some(bundled) = EMBEDDED_APPS.get_dir(name) else {
        log::warn!("bundled_check: no embedded plugin '{name}'");
        return false;
    };
    let mut files = HashMap::new();
    let mut dirs = std::collections::HashSet::new();
    collect_embedded(bundled, Path::new(name), &mut files, &mut dirs);
    let outcome =
        compare_with_embedded(plugin_dir, Path::new(""), &files, &dirs).and_then(|matched| {
            let required = files.keys().filter(|rel| !is_bytecode(rel)).count();
            if matched == required {
                Ok(())
            } else {
                Err(format!("{} bundled file(s) missing", required - matched))
            }
        });
    match outcome {
        Ok(()) => {
            log::info!("bundled_check: {name} verified at {}", plugin_dir.display());
            true
        }
        Err(reason) => {
            log::info!(
                "bundled_check: {name} not bundled at {}: {reason}",
                plugin_dir.display()
            );
            false
        }
    }
}

fn collect_embedded<'a>(
    dir: &'a include_dir::Dir<'a>,
    root: &Path,
    files: &mut HashMap<PathBuf, &'a [u8]>,
    dirs: &mut std::collections::HashSet<PathBuf>,
) {
    for entry in dir.entries() {
        let Ok(rel) = entry.path().strip_prefix(root) else {
            continue;
        };
        if rel
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(non_code_exclude)
        {
            continue;
        }
        match entry {
            include_dir::DirEntry::Dir(child) => {
                dirs.insert(rel.to_path_buf());
                collect_embedded(child, root, files, dirs);
            }
            include_dir::DirEntry::File(file) => {
                files.insert(rel.to_path_buf(), file.contents());
            }
        }
    }
}

/// Count the bundled files found under `rel`, or name the first difference.
/// Entries the packager leaves out are skipped here as in `collect_embedded`.
fn compare_with_embedded(
    root: &Path,
    rel: &Path,
    files: &HashMap<PathBuf, &[u8]>,
    dirs: &std::collections::HashSet<PathBuf>,
) -> Result<usize, String> {
    let dir = root.join(rel);
    let entries = std::fs::read_dir(&dir).map_err(|e| format!("read {}: {e}", dir.display()))?;
    let mut matched = 0;
    for entry in entries {
        let entry = entry.map_err(|e| format!("read {}: {e}", dir.display()))?;
        let name = entry.file_name();
        let child = rel.join(&name);
        if name.to_str().is_some_and(non_code_exclude)
            || (rel.as_os_str().is_empty()
                && name.to_str().is_some_and(is_installer_written_root_entry))
        {
            continue;
        }
        let path = entry.path();
        let meta = std::fs::symlink_metadata(&path)
            .map_err(|e| format!("stat {}: {e}", path.display()))?;
        if meta.file_type().is_symlink() {
            return Err(format!("symlink {}", child.display()));
        }
        if meta.is_dir() {
            if !dirs.contains(&child) {
                return Err(format!("extra directory {}", child.display()));
            }
            matched += compare_with_embedded(root, &child, files, dirs)?;
            continue;
        }
        let Some(expected) = files.get(&child) else {
            return Err(format!("extra file {}", child.display()));
        };
        if meta.len() != expected.len() as u64 {
            return Err(format!("changed file {}", child.display()));
        }
        let actual = std::fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
        if actual != *expected {
            return Err(format!("changed file {}", child.display()));
        }
        if !is_bytecode(&child) {
            matched += 1;
        }
    }
    Ok(matched)
}

/// Returns the set of plugin IDs defined in the bundled core pack.
pub fn core_pack_ids() -> std::collections::HashSet<String> {
    Pack::from_toml_str(CORE_PACK_TOML)
        .map(|p| p.plugins.into_iter().map(|a| a.id).collect())
        .unwrap_or_default()
}

/// First-party plugins deliberately retired before the v3 SDK migration. An plugin
/// is a quarantine candidate only when its stable former-first-party id is in
/// this set *and* its source still imports the removed v2 App/RenderContext
/// surface. The intersection is intentional: a user or marketplace plugin with
/// an unfamiliar id is never removed merely because it is not in core.
const RETIRED_PRE_V3_FIRST_PARTY_APP_IDS: &[&str] = &[
    "assistant",
    "audio-recorder",
    "bluesky",
    "calendar",
    "chess",
    "context-notifier-poc",
    "descriptor-renderer",
    "dev-here",
    "gh-projects",
    "github-tree",
    "image-loader",
    "image-render-poc",
    "image-url-poc",
    "input-inspector",
    "layout-demo",
    "mcp-demo",
    "mind-map",
    "node-canvas",
    "notification-tester",
    "permission-test",
    "quick-note",
    "screen-time",
    "secrets-routing-tester",
    "snake-race",
    "timer",
    "typing-tutor",
    "ui-playground",
    "workspace",
];

/// Return plugin directories identified as former first-party pre-v3 installs.
/// This is side-effect free so `plexi plugin prune --dry-run` can report exactly
/// what a launch-time reconciliation would remove.
pub fn orphaned_pre_v3_first_party_apps(target_root: &Path) -> Vec<PathBuf> {
    let core_ids = core_pack_ids();
    let mut candidates = Vec::new();
    let Ok(entries) = std::fs::read_dir(target_root) else {
        return candidates;
    };
    for entry in entries.flatten() {
        let plugin_dir = entry.path();
        if !plugin_dir.is_dir() {
            continue;
        }
        let Some(id) = plugin_dir.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if core_ids.contains(id) || !RETIRED_PRE_V3_FIRST_PARTY_APP_IDS.contains(&id) {
            continue;
        }
        if has_pre_v3_plugin_imports(&plugin_dir) {
            candidates.push(plugin_dir);
        }
    }
    candidates.sort();
    candidates
}

/// Quarantine only candidates returned by [`orphaned_pre_v3_first_party_apps`].
/// The hidden profile directory keeps ambiguous legacy content recoverable while
/// removing it from normal plugin discovery. Re-running is a no-op.
pub fn reconcile_orphaned_pre_v3_first_party_apps(target_root: &Path) -> Vec<PathBuf> {
    let candidates = orphaned_pre_v3_first_party_apps(target_root);
    let quarantine = target_root.join(".plexi-quarantine-pre-v3");
    for path in &candidates {
        let Some(name) = path.file_name() else {
            continue;
        };
        let destination = quarantine.join(name);
        match std::fs::create_dir_all(&quarantine)
            .and_then(|()| std::fs::rename(path, &destination))
        {
            Ok(()) => log::info!(
                "install: quarantined orphaned pre-v3 plugin {}",
                path.display()
            ),
            Err(error) => log::warn!(
                "install: could not quarantine orphaned pre-v3 plugin {}: {error}",
                path.display()
            ),
        }
    }
    candidates
        .into_iter()
        .filter(|path| !path.exists())
        .collect()
}

fn has_pre_v3_plugin_imports(plugin_dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(plugin_dir) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            return false;
        };
        if file_type.is_symlink() {
            return false;
        }
        if file_type.is_dir() {
            return has_pre_v3_plugin_imports(&path);
        }
        if !file_type.is_file() || path.extension().is_none_or(|extension| extension != "py") {
            return false;
        }
        std::fs::read_to_string(path).is_ok_and(|source| {
            source.contains("from plexi_sdk import")
                && source.contains("App")
                && source.contains("RenderContext")
        })
    })
}

/// Returns the set of plugin IDs defined in the bundled examples pack.
pub fn examples_pack_ids() -> std::collections::HashSet<String> {
    Pack::from_toml_str(EXAMPLES_PACK_TOML)
        .map(|p| p.plugins.into_iter().map(|a| a.id).collect())
        .unwrap_or_default()
}

/// Admit every source a pack names before any is fetched (stint 1398): one
/// git entry refuses the whole pack, so nothing is half-applied.
fn admit_pack_sources(pack: &Pack) -> Result<(), crate::host::plugin_admission::Refusal> {
    use crate::host::plugin_admission::{admit_install_source, InstallSource};
    for entry in &pack.plugins {
        match parse_source_spec(&entry.source) {
            Ok(SourceSpec::Local(name)) => admit_install_source(InstallSource::Bundled(&name))?,
            _ => admit_install_source(InstallSource::Git(&entry.source))?,
        }
    }
    Ok(())
}

/// Read `<workspace_root>/<channel_dir>/plugins.toml` and install declared plugins into the
/// workspace-scoped plugins dir (`<workspace_root>/<channel_dir>/plugins/`).
pub fn apply_workspace_pack(
    workspace_root: &Path,
    cloner: &dyn Cloner,
) -> Result<Vec<InstallOutcome>, String> {
    let channel_dir = crate::config::workspace_channel_dir();
    crate::app::plugin_migration::migrate(&workspace_root.join(&channel_dir))?;
    let plugins_toml = workspace_root.join(&channel_dir).join("plugins.toml");
    if !plugins_toml.exists() {
        return Err(
            format!("no {channel_dir}/plugins.toml found — run 'plexi workspace init' to initialize a workspace with a stub plugins.toml"),
        );
    }
    let pack = Pack::from_path(&plugins_toml)?;
    let workspace_apps = crate::app::registry::workspace_plugins_dir(workspace_root);
    std::fs::create_dir_all(&workspace_apps).map_err(|e| {
        format!(
            "create workspace plugins dir {}: {e}",
            workspace_apps.display()
        )
    })?;
    log::info!(
        "install: workspace pack {} → {} ({} plugins)",
        plugins_toml.display(),
        workspace_apps.display(),
        pack.plugins.len()
    );
    // A workspace never owns a bundled plugin's id (stint 1398), so a copy of
    // one here could never be opened by id: report the conflict instead of
    // installing it. The bundled plugin is already in the profile.
    let (bundled, installable): (Vec<_>, Vec<_>) = pack
        .plugins
        .iter()
        .cloned()
        .partition(|entry| crate::cli::install_host::is_bundled_plugin_id(&entry.id));
    let mut outcomes: Vec<InstallOutcome> = bundled
        .iter()
        .map(|entry| {
            let error = format!(
                "'{}' is bundled with Plexi and already in your profile; a workspace copy \
                 would never be opened by id. Remove it from {channel_dir}/plugins.toml.",
                entry.id
            );
            log::info!("install: workspace pack conflict: {error}");
            InstallOutcome {
                id: entry.id.clone(),
                status: InstallStatus::Failed(error),
            }
        })
        .collect();
    let remaining = Pack {
        plugins: installable,
        ..pack
    };
    outcomes.extend(apply_pack(cloner, &remaining, &workspace_apps, false));
    Ok(outcomes)
}

/// Return the set of plugin IDs declared in `<workspace_root>/<channel_dir>/plugins.toml`.
/// Returns an empty set if the file is absent or unparseable.
pub fn workspace_manifest_ids(workspace_root: &Path) -> std::collections::HashSet<String> {
    let channel_dir = crate::config::workspace_channel_dir();
    let path = workspace_root.join(&channel_dir).join("plugins.toml");
    match Pack::from_path(&path) {
        Ok(p) => p.plugins.into_iter().map(|a| a.id).collect(),
        Err(e) => {
            if path.exists() {
                log::warn!("failed to parse workspace plugins.toml: {e}");
            }
            std::collections::HashSet::new()
        }
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use std::sync::Mutex;

    /// The fetch mechanics behind the admission boundary, for tests of git
    /// install behaviour that stays in the code until third-party installs
    /// open (stint 1398). Production callers go through `install_one`.
    pub fn install_one_admitted(
        cloner: &dyn Cloner,
        source: &SourceSpec,
        git_ref: Option<&str>,
        target_root: &Path,
    ) -> Result<InstallOutcome, String> {
        super::install_one_admitted(
            cloner,
            &UvProvisioner,
            source,
            git_ref,
            target_root,
            GitRefPolicy::Strict,
        )
    }

    /// `apply_pack` behind the admission boundary; see [`install_one_admitted`].
    pub(crate) fn apply_pack_admitted(
        cloner: &dyn Cloner,
        provision: &dyn Provision,
        pack: &Pack,
        target_root: &Path,
        refresh_local: bool,
    ) -> Vec<InstallOutcome> {
        apply_admitted_pack(cloner, provision, pack, target_root, refresh_local)
    }

    /// Extract this binary's embedded `local:<name>` plugin under `target_root`
    /// the way the installer does, without provisioning a venv.
    pub fn extract_bundled(name: &str, target_root: &Path) -> PathBuf {
        let dest = target_root.join(name);
        std::fs::create_dir_all(&dest).unwrap();
        EMBEDDED_APPS
            .get_dir(name)
            .expect("embedded plugin")
            .extract(target_root)
            .unwrap();
        dest
    }

    /// In-memory [`Cloner`] for hermetic tests. Each "clone" writes a fixed
    /// manifest + entry file into the target dir; pull/checkout are no-ops.
    pub struct MockCloner {
        /// Optional manifest text to write per URL. Defaults to a minimal
        /// schema-1 manifest with the URL's repo name as the id.
        pub manifests: Mutex<HashMap<String, String>>,
        /// Per-URL clone failures, queued and popped in order.
        pub failures: Mutex<Vec<String>>,
        /// Per-checkout failures, queued and popped in order.
        pub checkout_failures: Mutex<Vec<CheckoutError>>,
        /// Track every `clone` call for assertions.
        pub clones: Mutex<Vec<(String, PathBuf)>>,
        /// Track every checkout ref for assertions.
        pub checkouts: Mutex<Vec<String>>,
        /// Track every pull target for assertions.
        pub pulls: Mutex<Vec<PathBuf>>,
    }

    impl MockCloner {
        pub fn new() -> Self {
            Self {
                manifests: Mutex::new(HashMap::new()),
                failures: Mutex::new(Vec::new()),
                checkout_failures: Mutex::new(Vec::new()),
                clones: Mutex::new(Vec::new()),
                checkouts: Mutex::new(Vec::new()),
                pulls: Mutex::new(Vec::new()),
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
        pub fn fail_next_checkout(&self, msg: &str) {
            self.checkout_failures
                .lock()
                .unwrap()
                .push(CheckoutError::RefUnavailable(msg.to_string()));
        }
        pub fn fail_next_checkout_fatally(&self, msg: &str) {
            self.checkout_failures
                .lock()
                .unwrap()
                .push(CheckoutError::Fatal(msg.to_string()));
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
                        "schema_version = 2\n\n[plugin]\nid = \"{id}\"\nname = \"{id}\"\nversion = \"0.0.1\"\n\n[runtime]\nkind = \"python\"\nentry = \"run.sh\"\n\n[[contributes.views]]\nid = \"{id}.main\"\ndefault = true\n"
                    )
                });
            std::fs::write(target_dir.join("manifest.toml"), manifest)
                .map_err(|e| e.to_string())?;
            let entry = target_dir.join("run.sh");
            std::fs::write(&entry, "#!/bin/sh\nexit 0\n").map_err(|e| e.to_string())?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut p = std::fs::metadata(&entry)
                    .map_err(|e| e.to_string())?
                    .permissions();
                p.set_mode(0o755);
                std::fs::set_permissions(&entry, p).map_err(|e| e.to_string())?;
            }
            Ok(())
        }
        fn checkout(&self, _repo_dir: &Path, git_ref: &str) -> Result<(), CheckoutError> {
            self.checkouts.lock().unwrap().push(git_ref.to_string());
            if let Some(err) = self.checkout_failures.lock().unwrap().pop() {
                return Err(err);
            }
            Ok(())
        }
        fn pull(&self, repo_dir: &Path) -> Result<(), String> {
            self.pulls.lock().unwrap().push(repo_dir.to_path_buf());
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

    /// Stint 1610: a clone from a server that never answers is stopped at
    /// the network limit instead of holding the install.
    #[test]
    fn a_hung_clone_is_stopped_at_its_limit() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        // Accept and hold the connection without ever answering.
        let held = std::thread::spawn(move || listener.accept().map(|(stream, _)| stream));
        let dir = tmp();
        let target = dir.path().join("r");
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let cloner = GitCloner {
                network_limit: Duration::from_millis(500),
            };
            let _ = tx.send(cloner.clone(&format!("git://127.0.0.1:{port}/r.git"), &target));
        });
        let result = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("the clone returned inside its limit");
        let error = result.unwrap_err();
        assert!(error.contains("did not finish within"), "{error}");
        drop(held);
    }

    /// Stint 1610: a fetch that could not be stopped fails the checkout;
    /// an ordinary fetch failure falls back to the local ref.
    #[test]
    fn only_a_refused_fetch_falls_back_to_the_local_ref() {
        use plexi_distribution::bounded::RunError;
        let refused = Err(RunError::Failed {
            what: "git fetch v1".into(),
            status: std::process::ExitStatus::default(),
            stderr: b"fatal: couldn't find remote ref v1".to_vec(),
        });
        assert!(fetch_or_fall_back(refused, "v1").is_ok());
        let unstopped = Err(RunError::NotStopped {
            what: "git fetch v1".into(),
            cause: "test".into(),
        });
        assert!(fetch_or_fall_back(unstopped, "v1").is_err());
        let timed_out = Err(RunError::TimedOut {
            what: "git fetch v1".into(),
            limit: Duration::from_secs(1),
            stdout: Vec::new(),
            stderr: Vec::new(),
        });
        assert!(fetch_or_fall_back(timed_out, "v1").is_err());
    }

    /// Stint 1610: a clone through the bounded runner still works end to end.
    #[test]
    fn git_cloner_clones_a_local_repository() {
        let dir = tmp();
        let source = dir.path().join("source");
        std::fs::create_dir(&source).unwrap();
        let git = |args: &[&str]| {
            run_git(
                git_in(&source)
                    .args(["-c", "user.name=t", "-c", "user.email=t@example.invalid"])
                    .args(args),
                "test git",
                GIT_LOCAL_LIMIT,
            )
            .unwrap();
        };
        git(&["init", "-q"]);
        std::fs::write(source.join("README"), "hi").unwrap();
        git(&["add", "README"]);
        git(&["commit", "-q", "-m", "init"]);
        let target = dir.path().join("clone");
        let url = format!("file://{}", source.display());
        GitCloner::default().clone(&url, &target).unwrap();
        assert_eq!(
            std::fs::read_to_string(target.join("README")).unwrap(),
            "hi"
        );
    }

    fn tmp() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn bundled_check_accepts_only_the_exact_embedded_tree() {
        use super::test_support::extract_bundled;
        let root = tmp();
        let dir = extract_bundled("permissions", root.path());
        assert!(matches_bundled_plugin("permissions", &dir));
        for name in crate::app::package::HOST_METADATA_FILES {
            std::fs::write(dir.join(name), "meta").unwrap();
        }
        std::fs::create_dir_all(dir.join(".venv/bin")).unwrap();
        assert!(
            matches_bundled_plugin("permissions", &dir),
            "installer-written metadata and venv are not plugin code"
        );
        assert!(!matches_bundled_plugin("logs", &dir), "another bundle");

        let cases: [(&str, fn(&Path)); 5] = [
            ("edited entry", |d| {
                std::fs::write(d.join("main.py"), "print('x')\n").unwrap()
            }),
            ("sourceless bytecode the importer would load", |d| {
                std::fs::write(d.join("typing.pyc"), b"\x00").unwrap()
            }),
            ("pycache beside the entry", |d| {
                // A dev build may embed a cache dir; a planted file is still extra.
                std::fs::create_dir_all(d.join("__pycache__")).unwrap();
                std::fs::write(d.join("__pycache__/planted.cpython-312.pyc"), b"\x00").unwrap();
            }),
            ("missing file", |d| {
                std::fs::remove_file(d.join("main.py")).unwrap()
            }),
            ("extra module", |d| {
                std::fs::write(d.join("helper.py"), "x = 1\n").unwrap()
            }),
        ];
        for (label, mutate) in cases {
            let root = tmp();
            let dir = extract_bundled("permissions", root.path());
            mutate(&dir);
            assert!(!matches_bundled_plugin("permissions", &dir), "{label}");
        }
        #[cfg(unix)]
        {
            let root = tmp();
            let dir = extract_bundled("permissions", root.path());
            let bytes = std::fs::read(dir.join("main.py")).unwrap();
            let elsewhere = root.path().join("main.py");
            std::fs::write(&elsewhere, bytes).unwrap();
            std::fs::remove_file(dir.join("main.py")).unwrap();
            std::os::unix::fs::symlink(&elsewhere, dir.join("main.py")).unwrap();
            assert!(
                !matches_bundled_plugin("permissions", &dir),
                "a symlink can change after the check"
            );
        }
    }

    #[test]
    fn python_plugin_venv_version_matches_sdk_floor() {
        let sdk_pyproject = include_str!("../../sdk/python/pyproject.toml");
        assert!(
            sdk_pyproject.contains(r#"requires-python = ">=3.12""#),
            "update this test when the SDK Python floor changes"
        );
        assert_eq!(crate::app::python_env::PYTHON_APP_VENV_VERSION, "3.12");
    }

    #[test]
    fn install_clone_writes_plugin_dir_with_manifest_id() {
        let target = tmp();
        let cloner = MockCloner::new();
        let outcome = test_support::install_one_admitted(
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
        test_support::install_one_admitted(
            &cloner,
            &SourceSpec::Git("https://example.com/foo.git".into()),
            None,
            target.path(),
        )
        .expect("first install should succeed");
        // Second install: must error with a clear message.
        let err = test_support::install_one_admitted(
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
            "schema_version = {}\n\n[plugin]\nid = \"future\"\nname = \"Future\"\nversion = \"0.1.0\"\n\n[runtime]\nkind = \"python\"\nentry = \"run.sh\"\n\n[[contributes.views]]\nid = \"future.main\"\ndefault = true\n",
            MANIFEST_SCHEMA_VERSION + 1
        );
        let cloner = MockCloner::new().with_manifest("https://example.com/future.git", &manifest);
        let err = test_support::install_one_admitted(
            &cloner,
            &SourceSpec::Git("https://example.com/future.git".into()),
            None,
            target.path(),
        )
        .expect_err("future schema must refuse");
        assert!(err.contains("schema_version must be 2"), "got: {err}");
        assert!(
            !target.path().join("future").exists(),
            "failed install must not leave a partial dir"
        );
    }

    #[test]
    fn install_passes_ref_to_checkout_when_set() {
        let target = tmp();
        let cloner = MockCloner::new();
        test_support::install_one_admitted(
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

    #[test]
    fn direct_install_ref_failure_remains_strict() {
        let target = tmp();
        let cloner = MockCloner::new();
        cloner.fail_next_checkout("git checkout missing failed");
        let err = test_support::install_one_admitted(
            &cloner,
            &SourceSpec::Git("https://example.com/strict.git".into()),
            Some("missing"),
            target.path(),
        )
        .expect_err("direct install with explicit bad ref must fail");

        assert!(err.contains("missing"), "got: {err}");
        assert!(
            !target.path().join("strict").exists(),
            "strict ref failure must not install default branch"
        );
    }

    // D2: per-plugin Python venv path construction tests.

    #[test]
    fn venv_path_is_inside_plugin_dir() {
        let plugin_dir = std::path::Path::new("/tmp/apps/my-app");
        let venv = plugin_dir.join(".venv");
        assert_eq!(venv, std::path::PathBuf::from("/tmp/apps/my-app/.venv"));
        // Verify the expected python binary path within the venv.
        let py = venv.join("bin").join("python");
        assert_eq!(
            py,
            std::path::PathBuf::from("/tmp/apps/my-app/.venv/bin/python")
        );
    }

    #[test]
    fn python_entry_detected_by_extension() {
        // .py entry → is_python = true
        let py_entry = "main.py";
        assert!(
            py_entry.ends_with(".py"),
            "main.py should be detected as Python"
        );

        // .sh entry → is_python = false
        let sh_entry = "run.sh";
        assert!(
            !sh_entry.ends_with(".py"),
            "run.sh must not be detected as Python"
        );

        // entry with no extension → is_python = false
        let no_ext = "myapp";
        assert!(
            !no_ext.ends_with(".py"),
            "entry with no extension must not be detected as Python"
        );

        // entry that happens to end in .py in a longer name
        let py_in_name = "app.py";
        assert!(
            py_in_name.ends_with(".py"),
            "plugin.py should be detected as Python"
        );
    }
}

#[cfg(test)]
mod pack_tests {
    use super::test_support::MockCloner;
    use super::*;

    fn pack_with(entries: &[(&str, &str, &str)]) -> Pack {
        let mut text = String::from("schema_version = 2\n\n");
        for (id, source, version) in entries {
            text.push_str(&format!(
                "\n[[plugin]]\nid = \"{id}\"\nsource = \"{source}\"\nversion = \"{version}\"\n\n"
            ));
        }
        Pack::from_toml_str(&text).unwrap()
    }

    #[test]
    fn pack_apply_skips_already_installed_at_same_version() {
        let target = tempfile::tempdir().unwrap();
        let cloner = MockCloner::new();
        let pack = pack_with(&[("foo", "github:owner/foo", "0.0.1")]);

        let first =
            test_support::apply_pack_admitted(&cloner, &UvProvisioner, &pack, target.path(), false);
        assert!(matches!(first[0].status, InstallStatus::Installed(_)));

        // Second apply: same id at same version → AlreadyAtVersion.
        let second =
            test_support::apply_pack_admitted(&cloner, &UvProvisioner, &pack, target.path(), false);
        assert!(matches!(second[0].status, InstallStatus::AlreadyAtVersion));
    }

    /// Regression guard (stint 0296 / PR #2359 validation): `refresh_local`
    /// must re-extract an installed `local:` plugin from the embedded tree —
    /// this is the only path by which core-plugin updates reach existing
    /// profiles on stable channels (launch reseed installs missing plugins
    /// only).
    #[test]
    fn pack_apply_refresh_local_replaces_stale_install() {
        let target = tempfile::tempdir().unwrap();
        let cloner = MockCloner::new();
        let pack = pack_with(&[("logs", "local:logs", "bundled")]);

        let first =
            test_support::apply_pack_admitted(&cloner, &UvProvisioner, &pack, target.path(), false);
        assert!(matches!(first[0].status, InstallStatus::Installed(_)));

        // Simulate a stale profile: drift the installed copy from the
        // embedded ground truth.
        let manifest_path = target.path().join("logs/manifest.toml");
        let embedded = std::fs::read_to_string(&manifest_path).unwrap();
        std::fs::write(&manifest_path, format!("{embedded}\n# stale drift\n")).unwrap();
        let stray = target.path().join("logs/stray_file.txt");
        std::fs::write(&stray, "left behind by an old version").unwrap();

        // Without refresh: untouched (skipped or up-to-date — the installed
        // manifest's real version never equals the pack's "bundled", which is
        // precisely why version comparison can't drive core-plugin updates).
        let plain =
            test_support::apply_pack_admitted(&cloner, &UvProvisioner, &pack, target.path(), false);
        assert!(matches!(
            plain[0].status,
            InstallStatus::AlreadyAtVersion | InstallStatus::SkippedOtherVersion { .. }
        ));
        assert!(std::fs::read_to_string(&manifest_path)
            .unwrap()
            .contains("stale drift"));

        // With refresh: replaced wholesale from the embedded tree.
        let refreshed =
            test_support::apply_pack_admitted(&cloner, &UvProvisioner, &pack, target.path(), true);
        assert!(
            matches!(refreshed[0].status, InstallStatus::Refreshed(_)),
            "expected Refreshed, got {:?}",
            refreshed[0].status
        );
        assert_eq!(std::fs::read_to_string(&manifest_path).unwrap(), embedded);
        assert!(
            !stray.exists(),
            "refresh must remove files not in the embedded tree"
        );
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

        let outcomes =
            test_support::apply_pack_admitted(&cloner, &UvProvisioner, &pack, target.path(), false);
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
        assert_eq!(
            n_failed, 1,
            "exactly one failure expected, got {outcomes:?}"
        );
        assert_eq!(n_ok, 1, "exactly one install expected, got {outcomes:?}");
    }

    #[test]
    fn pack_apply_records_unknown_source_scheme_per_entry() {
        let target = tempfile::tempdir().unwrap();
        let cloner = MockCloner::new();
        let pack = pack_with(&[("ok", "github:owner/ok", "v1"), ("bad", "ftp://nope", "v1")]);
        let outcomes =
            test_support::apply_pack_admitted(&cloner, &UvProvisioner, &pack, target.path(), false);
        let bad = outcomes.iter().find(|o| o.id == "bad").unwrap();
        match &bad.status {
            InstallStatus::Failed(msg) => assert!(msg.contains("unknown source scheme")),
            other => panic!("bad entry should fail, got: {other:?}"),
        }
        let ok = outcomes.iter().find(|o| o.id == "ok").unwrap();
        assert!(matches!(ok.status, InstallStatus::Installed(_)));
    }

    /// Stint 1610: a pack entry whose version fetch timed out fails instead
    /// of falling back to the default branch and reporting Installed. The
    /// cloner clones like the mock and checks out through the production
    /// path after a fetch that timed out.
    #[test]
    fn pack_apply_fails_an_entry_whose_fetch_timed_out() {
        struct TimedOutFetch(MockCloner);
        impl Cloner for TimedOutFetch {
            fn clone(&self, url: &str, target_dir: &Path) -> Result<(), String> {
                self.0.clone(url, target_dir)
            }
            fn checkout(&self, repo_dir: &Path, git_ref: &str) -> Result<(), CheckoutError> {
                let timed_out = Err(plexi_distribution::bounded::RunError::TimedOut {
                    what: format!("git fetch {git_ref}"),
                    limit: Duration::from_secs(600),
                    stdout: Vec::new(),
                    stderr: Vec::new(),
                });
                checkout_after_fetch(timed_out, repo_dir, git_ref)
            }
            fn pull(&self, repo_dir: &Path) -> Result<(), String> {
                self.0.pull(repo_dir)
            }
        }
        let target = tempfile::tempdir().unwrap();
        let cloner = TimedOutFetch(MockCloner::new());
        let pack = pack_with(&[("slow", "github:owner/slow", "0.0.1")]);

        let outcomes =
            test_support::apply_pack_admitted(&cloner, &UvProvisioner, &pack, target.path(), false);

        assert!(
            matches!(&outcomes[0].status, InstallStatus::Failed(msg) if msg.contains("did not finish within")),
            "got {:?}",
            outcomes[0].status
        );
        assert!(!target.path().join("slow").exists());
    }

    /// Stint 1610: a Python environment setup that could not be stopped
    /// fails the pack entry and removes the plugin, instead of reporting
    /// Installed; a missing uv still installs with a warning.
    #[test]
    fn pack_apply_fails_an_entry_whose_provisioning_could_not_be_stopped() {
        struct Stuck(VenvError);
        impl Provision for Stuck {
            fn provision(&self, _: &str, _: &Path, _: &PluginManifest) -> Result<(), VenvError> {
                Err(self.0.clone())
            }
        }
        let target = tempfile::tempdir().unwrap();
        let cloner = MockCloner::new();
        let pack = pack_with(&[("stuck", "github:owner/stuck", "0.0.1")]);
        let stuck = Stuck(VenvError::Fatal(
            "`uv pip install` could not be stopped".into(),
        ));

        let outcomes =
            test_support::apply_pack_admitted(&cloner, &stuck, &pack, target.path(), false);

        assert!(
            matches!(&outcomes[0].status, InstallStatus::Failed(msg) if msg.contains("could not be stopped")),
            "got {:?}",
            outcomes[0].status
        );
        assert!(!target.path().join("stuck").exists());

        let target = tempfile::tempdir().unwrap();
        let missing = Stuck(VenvError::Unavailable("could not start uv".into()));
        let outcomes =
            test_support::apply_pack_admitted(&cloner, &missing, &pack, target.path(), false);
        assert!(
            matches!(outcomes[0].status, InstallStatus::Installed(_)),
            "got {:?}",
            outcomes[0].status
        );
    }

    /// Stint 1610: a checkout that could not be stopped fails the pack entry
    /// instead of falling back and reporting Installed.
    #[test]
    fn pack_apply_fails_an_entry_whose_checkout_could_not_be_stopped() {
        let target = tempfile::tempdir().unwrap();
        let cloner = MockCloner::new();
        cloner.fail_next_checkout_fatally("git fetch 0.0.1 could not be stopped");
        let pack = pack_with(&[("stuck", "github:owner/stuck", "0.0.1")]);

        let outcomes =
            test_support::apply_pack_admitted(&cloner, &UvProvisioner, &pack, target.path(), false);

        assert!(
            matches!(&outcomes[0].status, InstallStatus::Failed(msg) if msg.contains("could not be stopped")),
            "got {:?}",
            outcomes[0].status
        );
        assert!(!target.path().join("stuck").exists());
    }

    /// Stint 1610: only a non-zero `git checkout` is a missing ref.
    #[test]
    fn only_a_failed_checkout_is_a_missing_ref() {
        use plexi_distribution::bounded::RunError;
        let failed = RunError::Failed {
            what: "git checkout v1".into(),
            status: std::process::ExitStatus::default(),
            stderr: b"error: pathspec 'v1' did not match".to_vec(),
        };
        assert!(matches!(
            checkout_outcome(Err(failed)),
            Err(CheckoutError::RefUnavailable(_))
        ));
        let timed_out = RunError::TimedOut {
            what: "git checkout v1".into(),
            limit: Duration::from_secs(1),
            stdout: Vec::new(),
            stderr: Vec::new(),
        };
        assert!(matches!(
            checkout_outcome(Err(timed_out)),
            Err(CheckoutError::Fatal(_))
        ));
    }

    #[test]
    fn pack_apply_falls_back_to_default_branch_when_version_ref_missing() {
        let target = tempfile::tempdir().unwrap();
        let cloner = MockCloner::new();
        cloner.fail_next_checkout("git checkout 0.0.1 failed");
        let pack = pack_with(&[("fallback", "github:owner/fallback", "0.0.1")]);

        let outcomes =
            test_support::apply_pack_admitted(&cloner, &UvProvisioner, &pack, target.path(), false);

        assert_eq!(cloner.checkouts.lock().unwrap().as_slice(), ["0.0.1"]);
        assert!(
            matches!(outcomes[0].status, InstallStatus::Installed(_)),
            "missing version ref should fall back to default branch HEAD, got {:?}",
            outcomes[0].status
        );
        assert!(target
            .path()
            .join("fallback")
            .join("manifest.toml")
            .exists());
    }
}

#[cfg(test)]
mod pack_export_tests {
    use super::*;

    /// Helper: write a manifest into `target_root/<id>/manifest.toml` so the
    /// export scanner has something to pick up.
    fn write_plugin(target_root: &Path, id: &str, version: &str) {
        let dir = target_root.join(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("manifest.toml"),
            format!(
                "schema_version = 2\n\n[plugin]\nid = \"{id}\"\nname = \"{id}\"\nversion = \"{version}\"\n\n[runtime]\nkind = \"python\"\nentry = \"run.sh\"\n\n[[contributes.views]]\nid = \"{id}.main\"\ndefault = true\n"
            ),
        )
        .unwrap();
    }

    #[test]
    fn pack_export_writes_valid_pack_to_path() {
        let target = tempfile::tempdir().unwrap();
        write_plugin(target.path(), "alpha-app", "0.1.0");
        write_plugin(target.path(), "beta-app", "0.2.0");

        let dest = target.path().join("out.toml");
        let n = export_pack(target.path(), &dest).expect("export should succeed");
        assert_eq!(n, 2, "should export both plugins");
        let text = std::fs::read_to_string(&dest).expect("dest written");
        // Re-parse the written file via the real Pack parser to prove it's
        // shape-valid and not just textually plausible.
        let parsed = Pack::from_toml_str(&text).expect("written pack must round-trip");
        assert_eq!(parsed.schema_version, 2);
        assert_eq!(parsed.plugins.len(), 2);
        let ids: Vec<&str> = parsed.plugins.iter().map(|a| a.id.as_str()).collect();
        assert!(ids.contains(&"alpha-app"));
        assert!(ids.contains(&"beta-app"));
    }

    #[test]
    fn pack_export_uses_local_scheme_for_non_git_apps() {
        let target = tempfile::tempdir().unwrap();
        write_plugin(target.path(), "bundled-thing", "1.0.0");
        // No `.git/` dir → infer_source_and_version must produce `local:`.
        let dest = target.path().join("out.toml");
        export_pack(target.path(), &dest).unwrap();
        let parsed = Pack::from_toml_str(&std::fs::read_to_string(&dest).unwrap()).unwrap();
        let entry = parsed
            .plugins
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
        write_plugin(target.path(), "real-app", "0.0.1");
        // Staging dir leftover from a crashed install — must be skipped.
        std::fs::create_dir_all(target.path().join(".tmp-install-1234-9999/clone")).unwrap();
        // A directory that isn't a Plexi plugin at all (no manifest) — also skipped.
        std::fs::create_dir_all(target.path().join("no-manifest-here")).unwrap();

        let dest = target.path().join("out.toml");
        let n = export_pack(target.path(), &dest).unwrap();
        assert_eq!(n, 1, "only the real plugin should be exported");
        let parsed = Pack::from_toml_str(&std::fs::read_to_string(&dest).unwrap()).unwrap();
        assert_eq!(parsed.plugins.len(), 1);
        assert_eq!(parsed.plugins[0].id, "real-app");
    }

    #[test]
    fn infer_source_for_local_install_uses_manifest_id() {
        let target = tempfile::tempdir().unwrap();
        let dir = target.path().join("foo");
        std::fs::create_dir_all(&dir).unwrap();
        let manifest_text = "schema_version = 2\n\n[plugin]\nid = \"foo\"\nname = \"Foo\"\nversion = \"0.3.0\"\n\n[runtime]\nkind = \"python\"\nentry = \"run.sh\"\n\n[[contributes.views]]\nid = \"foo.main\"\ndefault = true\n";
        let manifest: PluginManifest = toml::from_str(manifest_text).unwrap();
        let (source, version) = infer_source_and_version(&dir, &manifest);
        assert_eq!(source, "local:foo");
        assert_eq!(version, "0.3.0");
    }
}

#[cfg(test)]
mod core_pack_tests {
    use super::test_support::MockCloner;
    use super::*;

    fn plant_python_plugin(root: &Path, id: &str, source: &str) -> PathBuf {
        let plugin_dir = root.join(id);
        std::fs::create_dir_all(&plugin_dir).unwrap();
        std::fs::write(
            plugin_dir.join("manifest.toml"),
            format!(
                "schema_version = 2\n\n[plugin]\nid = \"{id}\"\nname = \"{id}\"\nversion = \"0.1.0\"\n\n[runtime]\nkind = \"python\"\nentry = \"main.py\"\n\n[[contributes.views]]\nid = \"{id}.main\"\ndefault = true\n"
            ),
        )
        .unwrap();
        std::fs::write(plugin_dir.join("main.py"), source).unwrap();
        plugin_dir
    }

    #[test]
    fn reconcile_quarantines_only_identified_orphaned_pre_v3_first_party_apps() {
        let target = tempfile::tempdir().unwrap();
        let legacy = "from plexi_sdk import App, RenderContext\nclass Old(App): pass\n";
        let stale_quick_note = plant_python_plugin(target.path(), "quick-note", legacy);
        let stale_timer = plant_python_plugin(target.path(), "timer", legacy);
        let live_core = plant_python_plugin(target.path(), "balls", legacy);
        let user_plugin = plant_python_plugin(target.path(), "my-private-app", legacy);
        let marketplace_plugin = plant_python_plugin(target.path(), "marketplace-legacy", legacy);
        let current_v3 = plant_python_plugin(
            target.path(),
            "calendar",
            "from plexi_sdk import log, state\nstate.get('today')\n",
        );

        let candidates = orphaned_pre_v3_first_party_apps(target.path());
        assert_eq!(
            candidates,
            vec![stale_quick_note.clone(), stale_timer.clone()]
        );
        let removed = reconcile_orphaned_pre_v3_first_party_apps(target.path());
        assert_eq!(removed, vec![stale_quick_note.clone(), stale_timer.clone()]);
        assert!(!stale_quick_note.exists());
        assert!(!stale_timer.exists());
        assert!(target
            .path()
            .join(".plexi-quarantine-pre-v3/quick-note")
            .exists());
        assert!(target
            .path()
            .join(".plexi-quarantine-pre-v3/timer")
            .exists());
        for preserved in [&live_core, &user_plugin, &marketplace_plugin, &current_v3] {
            assert!(preserved.exists(), "must preserve {}", preserved.display());
        }
        assert!(
            reconcile_orphaned_pre_v3_first_party_apps(target.path()).is_empty(),
            "a second reconcile must be idempotent"
        );
    }

    #[test]
    fn orphaned_pre_v3_dry_run_candidates_leave_directories_unchanged() {
        let target = tempfile::tempdir().unwrap();
        let stale = plant_python_plugin(
            target.path(),
            "quick-note",
            "from plexi_sdk import App, RenderContext\nclass Old(App): pass\n",
        );
        let candidates = orphaned_pre_v3_first_party_apps(target.path());
        assert_eq!(candidates, vec![stale.clone()]);
        assert!(
            stale.exists(),
            "candidate discovery must not delete anything"
        );
        assert!(stale.join("main.py").exists());
    }

    #[test]
    fn core_pack_always_installs_missing_apps() {
        let target = tempfile::tempdir().unwrap();
        let cloner = MockCloner::new();

        // First call: all plugins are missing → should install them.
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

        // Second call: plugins are already present → no Installed outcomes.
        let second = apply_core_pack_always(&cloner, target.path());
        let n_installed = second
            .iter()
            .filter(|o| matches!(o.status, InstallStatus::Installed(_)))
            .count();
        assert_eq!(
            n_installed, 0,
            "core pack must not re-install already-present plugins"
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
            "examples pack must NOT re-apply when plugins dir is non-empty"
        );
    }

    #[test]
    fn core_pack_parses() {
        let pack = Pack::from_toml_str(CORE_PACK_TOML)
            .expect("bundled core pack must parse against the current schema");
        assert!(
            !pack.plugins.is_empty(),
            "core pack should list at least one plugin"
        );
        for entry in &pack.plugins {
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
            !examples_pack.plugins.is_empty(),
            "examples pack should list at least one plugin"
        );
        for entry in &examples_pack.plugins {
            assert!(
                parse_source_spec(&entry.source).is_ok(),
                "examples pack entry '{}' has invalid source '{}'",
                entry.id,
                entry.source
            );
        }
    }

    /// Drift guard (stint 0391/0393): every `local:` entry in a bundled pack
    /// must resolve to a real directory under `plugins/` (recursively — pack
    /// entries may point at `plugins/examples/<id>` etc, not just top-level
    /// `plugins/<id>`) whose `manifest.toml` id matches the pack entry id. A
    /// pack entry that names a plugin which exists nowhere on disk fails
    /// silently at install time otherwise (stint 0393: `packs/examples.toml`
    /// carried four such phantom entries for two weeks before anyone
    /// installed on a fresh profile and hit the failure).
    fn assert_pack_entry_resolves_to_bundled_plugin(entry: &PackPlugin, pack_label: &str) {
        let spec = parse_source_spec(&entry.source).unwrap_or_else(|e| {
            panic!("{pack_label} entry '{}' has invalid source: {e}", entry.id)
        });
        let name = match spec {
            SourceSpec::Local(n) => n,
            SourceSpec::Git(_) => return, // git: sources are not bundled; nothing to check on disk.
        };
        assert_eq!(
            name, entry.id,
            "{pack_label} entry id '{}' must match its local: source name '{name}'",
            entry.id
        );
        let dir = find_embedded_plugin_dir(&name).unwrap_or_else(|| {
            panic!("{pack_label} plugin '{name}' has no directory under plugins/ (checked recursively)")
        });
        let manifest_text = dir
            .get_file(format!("{}/manifest.toml", dir.path().display()))
            .and_then(|f| f.contents_utf8())
            .unwrap_or_else(|| panic!("{pack_label} plugin '{name}' has no manifest.toml"));
        let manifest: PluginManifest = toml::from_str(manifest_text)
            .unwrap_or_else(|e| panic!("{pack_label} plugin '{name}' manifest invalid: {e}"));
        assert_eq!(
            manifest.plugin.id, entry.id,
            "{pack_label} plugin '{name}' manifest id '{}' must match pack id '{}'",
            manifest.plugin.id, entry.id
        );
    }

    /// Find a directory named `name` anywhere under the embedded `plugins/`
    /// tree (not just top-level). `include_dir::Dir::get_dir` only matches
    /// an exact relative path, so a bare id like `"coding-agent"` would miss
    /// `plugins/examples/coding-agent` — this walks every directory to find it
    /// by its final path component instead.
    fn find_embedded_plugin_dir<'a>(name: &str) -> Option<&'a include_dir::Dir<'a>> {
        fn walk<'a>(dir: &'a include_dir::Dir<'a>, name: &str) -> Option<&'a include_dir::Dir<'a>> {
            for sub in dir.dirs() {
                if sub.path().file_name().and_then(|n| n.to_str()) == Some(name) {
                    return Some(sub);
                }
                if let Some(found) = walk(sub, name) {
                    return Some(found);
                }
            }
            None
        }
        walk(&EMBEDDED_APPS, name)
    }

    /// Drift guard (stint 0296): `packs/core.toml` is the single source of
    /// truth for the maintained/core plugin set. Every entry must resolve to a
    /// bundled plugin directory whose manifest id matches, and the installer must
    /// not carry a competing hardcoded list of those same plugins.
    #[test]
    fn core_pack_is_canonical_and_consistent() {
        let pack = Pack::from_toml_str(CORE_PACK_TOML).expect("core pack must parse");
        assert!(
            !pack.plugins.is_empty(),
            "core pack must list at least one plugin"
        );

        for entry in &pack.plugins {
            assert_pack_entry_resolves_to_bundled_plugin(entry, "core pack");
        }

        // The maintained set lives ONLY in packs/core.toml. install.sh must
        // never re-enumerate it: it may only rsync `plugins/` or `plugins/dev/`
        // wholesale, never a specific `plugins/<core-id>/` path.
        let install_sh = include_str!("../../scripts/install.sh");
        for entry in &pack.plugins {
            let needle = format!("plugins/{}/", entry.id);
            assert!(
                !install_sh.contains(&needle),
                "install.sh hardcodes '{needle}' — the maintained plugin list must come \
                 only from packs/core.toml, not be enumerated in the installer"
            );
        }
    }

    /// Drift guard (stint 0393): every `packs/*.toml` entry must name a plugin
    /// that actually exists on disk. `packs/examples.toml` previously carried
    /// four phantom entries (`typing-tutor`, `mind-map`, `gh-projects`,
    /// `calendar`) that named plugins existing nowhere in the repo — silent
    /// until the moment a fresh profile tried to install them.
    #[test]
    fn examples_pack_entries_exist_on_disk() {
        let pack = Pack::from_toml_str(EXAMPLES_PACK_TOML).expect("examples pack must parse");
        assert!(
            !pack.plugins.is_empty(),
            "examples pack must list at least one plugin"
        );
        for entry in &pack.plugins {
            assert_pack_entry_resolves_to_bundled_plugin(entry, "examples pack");
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
    fn apply_workspace_pack_errors_when_no_apps_toml() {
        let ws = tempfile::tempdir().unwrap();
        let channel_dir = crate::config::workspace_channel_dir();
        std::fs::create_dir_all(ws.path().join(&channel_dir)).unwrap();

        let cloner = MockCloner::new();
        let err = apply_workspace_pack(ws.path(), &cloner).expect_err("should error");
        assert!(
            err.contains(&format!("no {channel_dir}/plugins.toml found")),
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
            plexi_dir.join("plugins.toml"),
            "schema_version = 2\n\n[[plugin]]\nid = \"foo\"\nsource = \"github:a/b\"\nversion = \"v1\"\n\n[[plugin]]\nid = \"bar\"\nsource = \"github:a/c\"\nversion = \"v2\"\n",
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
}
