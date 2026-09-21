use super::app::{is_bare_id, is_github_shorthand};
use super::print_tip;
use crate::cli::release_resolver;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub fn install_cli(spec: &str, assume_yes: bool) -> i32 {
    use crate::cli::marketplace::InstallPlan;
    let (source_str, git_ref) = crate::cli::install_host::split_source_and_ref(spec);
    let resolved = if is_bare_id(&source_str) {
        // A bare id resolves only through the hosted marketplace catalog. A
        // free/licensed app installs from its CDN artifact (or a declared github
        // source); a paid app with no license is blocked; an unknown id or an
        // unreachable registry is a hard error. There is no legacy fallback.
        match crate::cli::marketplace::plan_install(&source_str) {
            InstallPlan::Package {
                path,
                reviewed_native,
                source_metadata,
            } => {
                if reviewed_native {
                    return crate::cli::app::app_install_marketplace_package(
                        &path.to_string_lossy(),
                        None,
                        crate::cli::InstallConfirm::Interactive,
                        assume_yes,
                        Some(source_metadata),
                    );
                }
                return crate::cli::app::app_install_package(
                    &path.to_string_lossy(),
                    None,
                    crate::cli::InstallConfirm::Interactive,
                    assume_yes,
                );
            }
            InstallPlan::Source(spec) => spec,
            InstallPlan::Blocked => return 1,
            InstallPlan::NotFound => {
                eprintln!(
                    "error: no app '{source_str}' in the marketplace — run `plexi app search {source_str}` or `plexi app browse`"
                );
                return 1;
            }
            InstallPlan::Unreachable => {
                eprintln!(
                    "error: could not reach the marketplace to resolve '{source_str}'. \
                     Check your connection, or install from an explicit source (github:owner/repo)."
                );
                return 1;
            }
        }
    } else if is_github_shorthand(&source_str) {
        let prefixed = format!("github:{source_str}");
        log::info!("install: bare shorthand '{source_str}' → {prefixed}");
        prefixed
    } else {
        source_str
    };
    let source = match crate::app::packs::parse_source_spec(&resolved) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let target_root = crate::app::registry::apps_dir();
    let cloner = crate::cli::install_host::GitCloner;
    match crate::cli::install_host::install_one(&cloner, &source, git_ref.as_deref(), &target_root)
    {
        Ok(outcome) => match outcome.status {
            crate::cli::install_host::InstallStatus::Installed(path) => {
                println!("installed '{}' at {}", outcome.id, path.display());
                print_tip(&format!(
                    "open your app with `plexi app open {}`.",
                    outcome.id
                ));
                0
            }
            crate::cli::install_host::InstallStatus::Refreshed(path) => {
                println!("refreshed '{}' at {}", outcome.id, path.display());
                0
            }
            crate::cli::install_host::InstallStatus::AlreadyAtVersion => {
                println!("already at requested version");
                0
            }
            crate::cli::install_host::InstallStatus::SkippedOtherVersion {
                installed,
                requested,
            } => {
                eprintln!(
                    "'{}' already installed at {installed} (requested {requested}); \
                     uninstall first or use `plexi update apps`",
                    outcome.id
                );
                1
            }
            crate::cli::install_host::InstallStatus::Failed(msg) => {
                eprintln!("error: {msg}");
                1
            }
        },
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

/// `plexi install --pack <path|core>` — apply a whole pack file.
/// `refresh_local`: re-extract already-installed `local:` apps from the
/// embedded tree (see `install_host::apply_pack`).
pub fn install_pack_cli(spec: &str, refresh_local: bool) -> i32 {
    let pack = if spec == "core" {
        match crate::app::packs::Pack::from_toml_str(crate::cli::install_host::CORE_PACK_TOML) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("error: bundled core pack invalid: {e}");
                return 1;
            }
        }
    } else {
        match crate::app::packs::Pack::from_path(std::path::Path::new(spec)) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("error: {e}");
                return 1;
            }
        }
    };
    let target_root = crate::app::registry::apps_dir();
    if let Err(e) = std::fs::create_dir_all(&target_root) {
        eprintln!("error: create apps dir {}: {e}", target_root.display());
        return 1;
    }
    let cloner = crate::cli::install_host::GitCloner;
    let outcomes =
        crate::cli::install_host::apply_pack(&cloner, &pack, &target_root, refresh_local);
    let mut any_failed = false;
    for o in &outcomes {
        match &o.status {
            crate::cli::install_host::InstallStatus::Installed(p) => {
                println!("  installed  {:30} → {}", o.id, p.display());
            }
            crate::cli::install_host::InstallStatus::Refreshed(p) => {
                println!("  refreshed  {:30} → {}", o.id, p.display());
            }
            crate::cli::install_host::InstallStatus::AlreadyAtVersion => {
                println!("  up-to-date {:30}", o.id);
            }
            crate::cli::install_host::InstallStatus::SkippedOtherVersion {
                installed,
                requested,
            } => {
                println!(
                    "  skipped    {:30} (installed {installed}, requested {requested})",
                    o.id
                );
            }
            crate::cli::install_host::InstallStatus::Failed(msg) => {
                eprintln!("  FAILED     {:30} {msg}", o.id);
                any_failed = true;
            }
        }
    }
    if any_failed {
        1
    } else {
        0
    }
}

/// `plexi install` with no args — detect `<channel_dir>/apps.toml` and apply it.
///
/// Walks up from CWD looking for the channel dir (workspace marker), reads
/// `apps.toml` from it, and installs declared apps into the workspace-scoped
/// channel apps dir (`<workspace_root>/<channel_dir>/apps/`).
pub fn install_workspace_pack_cli() -> i32 {
    log::info!("cli: install_workspace_pack (no-args flow)");
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };

    let channel_dir = crate::config::workspace_channel_dir();

    // Walk up from CWD looking for the channel dir (workspace marker).
    let workspace_root = {
        let home = dirs::home_dir();
        let mut current = cwd;
        let mut found: Option<std::path::PathBuf> = None;
        loop {
            if let Some(ref h) = home {
                if current == *h {
                    break;
                }
            }
            if current == std::path::Path::new("/") {
                break;
            }
            if current.join(&channel_dir).is_dir() {
                found = Some(current);
                break;
            }
            if !current.pop() {
                break;
            }
        }
        found
    };

    let Some(root) = workspace_root else {
        eprintln!(
            "Usage: plexi app install <source-spec>[@ref] | plexi app install --pack <path|core>"
        );
        eprintln!("  In a workspace (directory with {channel_dir}/), `plexi app install` applies the manifest.");
        eprintln!("  Run `plexi workspace init` to initialize a workspace here.");
        return 1;
    };

    let apps_toml = root.join(&channel_dir).join("apps.toml");
    if !apps_toml.exists() {
        eprintln!(
            "no {channel_dir}/apps.toml found in workspace at {}",
            root.display()
        );
        eprintln!("  Declare app dependencies there, then re-run `plexi app install`.");
        eprintln!(
            "  Usage: plexi app install <source-spec>[@ref] | plexi app install --pack <path|core>"
        );
        return 1;
    }

    log::info!(
        "install_workspace_pack:cli: applying {}",
        apps_toml.display()
    );
    println!("Applying workspace manifest {}...", apps_toml.display());

    let cloner = crate::cli::install_host::GitCloner;
    let outcomes = match crate::cli::install_host::apply_workspace_pack(&root, &cloner) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };

    if outcomes.is_empty() {
        println!("No apps declared in .plexi/apps.toml.");
        return 0;
    }

    let mut any_failed = false;
    for o in &outcomes {
        match &o.status {
            crate::cli::install_host::InstallStatus::Installed(p) => {
                println!("  installed  {:30} → {}", o.id, p.display());
            }
            crate::cli::install_host::InstallStatus::Refreshed(p) => {
                println!("  refreshed  {:30} → {}", o.id, p.display());
            }
            crate::cli::install_host::InstallStatus::AlreadyAtVersion => {
                println!("  up-to-date {:30}", o.id);
            }
            crate::cli::install_host::InstallStatus::SkippedOtherVersion {
                installed,
                requested,
            } => {
                println!(
                    "  skipped    {:30} (installed {installed}, requested {requested})",
                    o.id
                );
            }
            crate::cli::install_host::InstallStatus::Failed(msg) => {
                eprintln!("  FAILED     {:30} {msg}", o.id);
                any_failed = true;
            }
        }
    }
    if any_failed {
        1
    } else {
        0
    }
}

/// Maps a release channel (as returned by `config::build_channel()`, e.g.
/// `None` for the main channel, `Some("alpha")`, `Some("pr-2357")`, or an
/// arbitrary named channel) to the capitalized bundle-name suffix used for
/// `/Applications/Plexi<cap>.app`.
///
/// `None` → `""`, `Some("alpha")` → `" Alpha"`, `Some("beta")` → `" Beta"`,
/// `Some("pr-2357")` → `" PR2357"`, `Some("foo")` → `" Foo"` (title-cased).
///
/// The Rust-side source of truth: the uninstaller and
/// `plexi host start/stop/status` (`src/cli/host.rs`) both call this, so bundle
/// path resolution never drifts between them.
///
/// The install scripts run before any Plexi binary exists, so they carry their
/// own bash implementations of the same mapping: `scripts/install.sh`,
/// `scripts/channel-clean.sh`, and `_channel_cap` in `scripts/uninstall.sh`.
/// A change here has to be mirrored in all three.
pub(crate) fn channel_bundle_cap(channel: Option<&str>) -> String {
    match channel {
        None => String::new(),
        Some(c) => {
            if let Some(n) = c.strip_prefix("pr-") {
                format!(" PR{n}")
            } else {
                match c {
                    "alpha" => " Alpha".to_string(),
                    "beta" => " Beta".to_string(),
                    other => {
                        let mut chars = other.chars();
                        match chars.next() {
                            Some(first) => {
                                format!(" {}{}", first.to_uppercase(), chars.as_str())
                            }
                            None => String::new(),
                        }
                    }
                }
            }
        }
    }
}

/// `plexi uninstall [--keep-data] [--yes]` — remove Plexi itself from the Mac.
pub fn plexi_uninstall_cli(keep_data: bool, assume_yes: bool) -> i32 {
    // Channel of the running binary (`plexi-alpha` → `-alpha`, `plexi` → ``).
    let channel = crate::config::build_channel();
    let suffix = channel
        .as_deref()
        .map(|c| format!("-{c}"))
        .unwrap_or_default();
    let cap_owned = channel_bundle_cap(channel.as_deref());
    let cap = cap_owned.as_str();

    let profile_dir = dirs::home_dir().unwrap().join(format!(".plexi{suffix}"));
    let app_bundle = std::path::PathBuf::from(format!("/Applications/Plexi{cap}.app"));
    let cli_binary = std::path::PathBuf::from(format!("/usr/local/bin/plexi{suffix}"));

    // Single confirmation prompt: keep data or remove everything?
    // Resolved before the banner so the preview accurately reflects the outcome.
    let keep_data = if keep_data || !profile_dir.exists() {
        log::info!(
            "uninstall: keep_data=flag({keep_data}) profile_exists={}",
            profile_dir.exists()
        );
        keep_data
    } else if assume_yes {
        log::info!("uninstall: keep_data=false (assume_yes, no --keep-data)");
        false
    } else {
        eprint!("Keep your ~/.plexi{suffix} data for future installs? [y/n, Enter=abort]: ");
        let _ = io::stderr().flush();
        let mut answer = String::new();
        if let Err(e) = io::stdin().read_line(&mut answer) {
            log::warn!("uninstall: failed to read keep-data confirmation: {e}");
            eprintln!("error: failed to read: {e}");
            return 1;
        }
        match answer.trim().to_lowercase().as_str() {
            "y" | "yes" => {
                log::info!("uninstall: keep_data=true (user chose y)");
                true
            }
            "n" | "no" => {
                log::info!("uninstall: keep_data=false (user chose n)");
                eprintln!("Removing everything.");
                false
            }
            other => {
                log::info!("uninstall: aborted (user input {:?})", other);
                eprintln!("Aborted.");
                return 0;
            }
        }
    };

    // Print what will be removed (after keep_data is resolved so the preview is accurate)
    println!("This will remove:");
    if app_bundle.exists() {
        println!("  \u{2022} {}", app_bundle.display());
    }
    if cli_binary.exists() {
        println!("  \u{2022} {}", cli_binary.display());
    }
    if !keep_data && profile_dir.exists() {
        println!(
            "  \u{2022} {}  (settings, secrets, app configs)",
            profile_dir.display()
        );
    } else if profile_dir.exists() {
        println!("  \u{2022} {} will be kept", profile_dir.display());
    }

    let mut removed = false;

    // Archive backlog before potentially deleting profile dir
    if !keep_data {
        let backlog = profile_dir.join("backlog");
        if backlog.exists() {
            let ts = crate::platform::clock::now_secs();
            let archive = dirs::home_dir()
                .unwrap()
                .join(format!("plexi-backlog-archive/plexi{suffix}-backlog-{ts}"));
            if let Some(parent) = archive.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if std::fs::rename(&backlog, &archive).is_ok() {
                println!("Archived backlog \u{2192} {}", archive.display());
            }
        }
    }

    // Remove app bundle
    if app_bundle.exists() {
        match std::fs::remove_dir_all(&app_bundle) {
            Ok(()) => {
                println!("Removed {}", app_bundle.display());
                removed = true;
            }
            Err(e) => eprintln!("warning: could not remove {}: {e}", app_bundle.display()),
        }
    }

    // Remove CLI binary
    if cli_binary.exists() || cli_binary.is_symlink() {
        match std::fs::remove_file(&cli_binary) {
            Ok(()) => {
                println!("Removed {}", cli_binary.display());
                removed = true;
            }
            Err(e) => eprintln!("warning: could not remove {}: {e}", cli_binary.display()),
        }
    }

    // Remove completions (only for main uninstall)
    if suffix.is_empty() {
        let brew_prefix = std::process::Command::new("brew")
            .arg("--prefix")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string());
        if let Some(prefix) = brew_prefix {
            let zsh_comp = std::path::PathBuf::from(prefix).join("share/zsh/site-functions/_plexi");
            if zsh_comp.exists() {
                let _ = std::fs::remove_file(&zsh_comp);
                println!("Removed {}", zsh_comp.display());
            }
        }
    }

    // Remove profile dir
    if !keep_data && profile_dir.exists() {
        match std::fs::remove_dir_all(&profile_dir) {
            Ok(()) => {
                println!("Removed {}", profile_dir.display());
                removed = true;
            }
            Err(e) => eprintln!("warning: could not remove {}: {e}", profile_dir.display()),
        }
    }

    if removed {
        println!(
            "\nDone. Plexi{} has been removed.",
            if cap.is_empty() { "" } else { cap }
        );
    } else {
        println!("\nNothing found to remove.");
    }
    0
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UpdateTarget {
    pub id: String,
    pub app_dir: PathBuf,
}

fn update_target_from_installed(app: &crate::app::registry::InstalledApp) -> Option<UpdateTarget> {
    match app.source {
        crate::app::registry::RegistrySource::Global
        | crate::app::registry::RegistrySource::LocalApp => Some(UpdateTarget {
            id: app.manifest.id.clone(),
            app_dir: app.app_dir.clone(),
        }),
        crate::app::registry::RegistrySource::LocalAgent => None,
    }
}

pub(crate) fn resolve_update_targets(
    maybe_id: Option<&str>,
    cwd: &Path,
    global_apps_dir: &Path,
) -> Result<Vec<UpdateTarget>, String> {
    let registry = crate::app::registry::AppRegistry::load_with_global(cwd, global_apps_dir);
    if let Some(id) = maybe_id {
        let app = registry
            .get(id)
            .ok_or_else(|| format!("app '{id}' not installed — run `plexi app list`"))?;
        return update_target_from_installed(app)
            .map(|target| vec![target])
            .ok_or_else(|| format!("'{id}' is not an installed app"));
    }
    Ok(registry
        .list()
        .into_iter()
        .filter_map(update_target_from_installed)
        .collect())
}

/// `plexi app update [<id>]` — git-pull one installed app, or all visible apps.
/// `plexi update apps [<id>]` is a compatibility alias for the same path.
/// Apps that aren't git checkouts (e.g. bundled core entries) are skipped.
pub fn update_cli(maybe_id: Option<&str>) -> i32 {
    let cwd = match std::env::current_dir() {
        Ok(path) => path,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let global_apps_dir = crate::app::registry::apps_dir();
    let cloner = crate::cli::install_host::GitCloner;
    let targets = match resolve_update_targets(maybe_id, &cwd, &global_apps_dir) {
        Ok(targets) => targets,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    if targets.is_empty() {
        println!("no apps installed");
        return 0;
    }
    log::info!(
        "app_update: resolved {} target(s) from cwd={}",
        targets.len(),
        cwd.display()
    );
    let mut any_failed = false;
    for target in targets {
        match crate::cli::install_host::update_app_dir(&cloner, &target.id, &target.app_dir) {
            Ok(()) => println!("  updated  {}", target.id),
            Err(e) if e.contains("not a git checkout") => {
                println!("  skipped  {} (not a git checkout)", target.id);
            }
            Err(e) => {
                eprintln!("  FAILED   {}: {e}", target.id);
                any_failed = true;
            }
        }
    }
    if any_failed {
        1
    } else {
        0
    }
}

#[cfg(test)]
mod update_tests {
    use super::*;

    fn write_app(apps_root: &Path, id: &str, name: &str) -> PathBuf {
        let app_dir = apps_root.join(id);
        std::fs::create_dir_all(&app_dir).unwrap();
        std::fs::write(
            app_dir.join("manifest.toml"),
            format!(
                "schema_version = 1\n\n[app]\nid = \"{id}\"\ntype = \"app\"\nname = \"{name}\"\nversion = \"0.1.0\"\nentry = \"run.sh\"\n"
            ),
        )
        .unwrap();
        std::fs::write(app_dir.join("run.sh"), "#!/bin/sh\nexit 0\n").unwrap();
        app_dir
    }

    #[test]
    fn update_target_for_id_resolves_workspace_app_before_global() {
        let global = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let channel_dir = crate::config::workspace_channel_dir();
        let workspace_apps = workspace.path().join(&channel_dir).join("apps");
        std::fs::create_dir_all(&workspace_apps).unwrap();

        write_app(global.path(), "tool", "Global Tool");
        let local_dir = write_app(&workspace_apps, "tool", "Workspace Tool");

        let targets = resolve_update_targets(Some("tool"), workspace.path(), global.path())
            .expect("workspace app should resolve");

        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].id, "tool");
        assert_eq!(targets[0].app_dir, local_dir);
    }

    #[test]
    fn update_targets_include_global_and_current_workspace_apps() {
        let global = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let channel_dir = crate::config::workspace_channel_dir();
        let workspace_apps = workspace.path().join(&channel_dir).join("apps");
        std::fs::create_dir_all(&workspace_apps).unwrap();

        let global_dir = write_app(global.path(), "global-tool", "Global Tool");
        let local_dir = write_app(&workspace_apps, "local-tool", "Local Tool");

        let targets = resolve_update_targets(None, workspace.path(), global.path())
            .expect("all update targets should resolve");

        assert_eq!(targets.len(), 2);
        assert!(targets.iter().any(|target| target.app_dir == global_dir));
        assert!(targets.iter().any(|target| target.app_dir == local_dir));
    }
}

/// Core update logic, callable from both the CLI and the GUI one-click button.
///
/// `from_gui` — when `true`, the caller is the changelog modal running inside the
/// live Plexi process.  The function skips the `PLEXI_RUNNING` env-var check and
/// uses the relaunch-script path unconditionally (the app *is* running).
/// When `false` (CLI), the env-var check is used instead.
///
/// Returns `Ok(human_readable_message)` on success and `Err(error_message)` on
/// failure so callers can surface the outcome appropriately.
/// CLI-only self-update: download and install a signed-channel release asset.
/// The GUI uses the same asset installer in the background, then restarts.
fn run_self_update() -> Result<String, String> {
    let binary_name = crate::config::current_exe_basename();
    let binary_name = binary_name.as_str();

    let build_channel = crate::config::build_channel();
    let suffix = build_channel
        .as_deref()
        .map(|c| format!("-{c}"))
        .unwrap_or_default();
    let channel = build_channel.unwrap_or_else(|| "main".to_string());

    log::info!("cli: self-update channel={channel} suffix={suffix}");

    if let Ok(pty_channel) = std::env::var("PLEXI_CHANNEL") {
        if !pty_channel.is_empty() && pty_channel != channel {
            return Err(format!(
                "error: PLEXI_CHANNEL={pty_channel} but this binary is '{binary_name}' (channel {channel}).\nRun the matching channel binary to self-update."
            ));
        }
    }

    let update_channel = release_resolver::UpdateChannel::from_binary_name(binary_name);
    log::info!(
        "cli: self-update input channel={channel} update_channel={update_channel:?} install_target={channel}"
    );

    let profile_dir = dirs::home_dir()
        .unwrap_or_default()
        .join(format!(".plexi{suffix}"));
    let current_version_raw = std::fs::read_to_string(profile_dir.join("installed_tag"))
        .ok()
        .and_then(|s| {
            let t = s.trim().to_string();
            release_resolver::ReleaseTag::parse(&t).map(|_| t)
        })
        .unwrap_or_else(|| format!("v{}", env!("CARGO_PKG_VERSION")));
    println!("Checking for updates...");
    println!("Current: {current_version_raw}");

    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .build();

    let releases = release_resolver::fetch_releases(&agent).map_err(|e| format!("error: {e}"))?;
    let current_tag = release_resolver::ReleaseTag::parse(&current_version_raw)
        .ok_or_else(|| format!("error: could not parse current version {current_version_raw}"))?;
    let selected = match release_resolver::resolve_best(&releases, update_channel, &current_tag) {
        Some(t) => t,
        None => return Ok(format!("Already up to date ({current_version_raw}).")),
    };
    let tag_name = selected.raw.clone();
    let latest_version = tag_name.trim_start_matches('v').to_string();
    log::info!("cli: self-update selected tag={tag_name} install_target_channel={channel}");
    println!("Latest:  {tag_name}");

    println!("Downloading v{latest_version} release asset...");
    let script_url = format!(
        "https://raw.githubusercontent.com/ianjamesburke/PLEXI/{tag_name}/scripts/install.sh"
    );
    let install = std::process::Command::new("bash")
        .args([
            "-c",
            "curl -fsSL \"$1\" | bash -s -- --channel \"$2\" --tag \"$3\"",
            "plexi-update",
            &script_url,
            &channel,
            &tag_name,
        ])
        .env("PLEXI_INSTALL_TAG", &tag_name)
        .status()
        .map_err(|e| format!("error: failed to run install script: {e}"))?;
    if !install.success() {
        return Err("error: binary install failed — this release may predate binary assets; use a current v1 release or build a checkout with scripts/install.sh --from-source".to_string());
    }

    Ok(format!(
        "Installed v{latest_version}. Restart Plexi to apply."
    ))
}

/// `plexi update` — thin CLI wrapper around `run_self_update`.
pub fn self_update_cli() -> i32 {
    match run_self_update() {
        Ok(msg) => {
            println!("{msg}");
            0
        }
        Err(msg) => {
            eprintln!("{msg}");
            1
        }
    }
}

#[cfg(test)]
mod channel_bundle_cap_tests {
    use super::channel_bundle_cap;

    #[test]
    fn main_channel_has_no_cap() {
        assert_eq!(channel_bundle_cap(None), "");
    }

    #[test]
    fn alpha_channel() {
        assert_eq!(channel_bundle_cap(Some("alpha")), " Alpha");
    }

    #[test]
    fn beta_channel() {
        assert_eq!(channel_bundle_cap(Some("beta")), " Beta");
    }

    #[test]
    fn pr_channel() {
        assert_eq!(channel_bundle_cap(Some("pr-2357")), " PR2357");
    }

    #[test]
    fn arbitrary_named_channel_is_title_cased() {
        assert_eq!(channel_bundle_cap(Some("gpui")), " Gpui");
        assert_eq!(channel_bundle_cap(Some("foo")), " Foo");
    }
}
