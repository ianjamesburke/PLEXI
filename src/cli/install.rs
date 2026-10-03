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

pub fn plexi_uninstall_cli(_keep_data: bool, assume_yes: bool) -> i32 {
    let result = (|| -> Result<(), String> {
        let receipt = crate::distribution::installed()?.ok_or_else(|| "No managed installation receipt found. No files were removed. Reinstall through the current installer to migrate this legacy installation.".to_string())?;
        println!("Remove Plexi channel {} from {}. User data will be retained.", receipt.channel, receipt.root.display());
        if !assume_yes {
            eprint!("Continue? [y/N]: ");
            io::stderr().flush().map_err(|e| format!("write confirmation: {e}"))?;
            let mut answer = String::new();
            io::stdin().read_line(&mut answer).map_err(|e| format!("read confirmation: {e}"))?;
            if !matches!(answer.trim(), "y" | "Y" | "yes") { return Err("Uninstall cancelled.".into()); }
        }
        if super::host::running_build_info().is_some() && super::host::host_stop_cli() != 0 {
            return Err("could not stop the channel host before uninstall".into());
        }
        #[cfg(not(windows))]
        plexi_distribution::transaction::uninstall(&receipt.root).map_err(|e| e.to_string())?;
        #[cfg(windows)]
        {
            let helper = std::env::temp_dir().join(format!("plexi-uninstall-{}.exe", uuid::Uuid::new_v4()));
            std::fs::copy(receipt.active.path.join("plexi-installer.exe"), &helper).map_err(|e| format!("stage removal helper: {e}"))?;
            let log = std::fs::File::create(receipt.root.join("uninstall.log")).map_err(|e| format!("create uninstall log: {e}"))?;
            let stderr = log.try_clone().map_err(|e| format!("clone uninstall log: {e}"))?;
            std::process::Command::new(helper).arg("remove").arg("--receipt").arg(&receipt.root).arg("--wait-pid").arg(std::process::id().to_string())
                .stdin(std::process::Stdio::null()).stdout(log).stderr(stderr).spawn().map_err(|e| format!("schedule uninstall: {e}"))?;
            println!("Removal will finish after this command exits; details: {}", receipt.root.join("uninstall.log").display());
        }
        log::info!("uninstall: managed removal requested for {}", receipt.channel);
        Ok(())
    })();
    match result { Ok(()) => { println!("User data retained."); 0 }, Err(error) => { eprintln!("error: {error}"); 1 } }
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

    let update_channel = release_resolver::UpdateChannel::from_binary_name(binary_name);
    log::info!(
        "cli: self-update input channel={channel} update_channel={update_channel:?} install_target={channel}"
    );

    let current_version_raw = crate::distribution::installed()?
        .map(|receipt| receipt.active.tag).unwrap_or_else(crate::distribution::build_tag);
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
    run_binary_asset_install(&channel, &tag_name)?;

    Ok(format!(
        "Installed v{latest_version}. Restart Plexi to apply."
    ))
}

/// Prefer the platform installer that downloads the release zip/tarball.
/// Never falls back to a cargo source build from this path (BIN-07).
pub(crate) fn run_binary_asset_install(channel: &str, tag_name: &str) -> Result<(), String> {
    use plexi_distribution::{package::Package, release, transaction};
    let channel = release::normalized_channel(channel);
    if !release::accepts(channel, tag_name, tag_name.contains('-')) {
        return Err(format!("release {tag_name} is not accepted by {channel}"));
    }
    let current = crate::distribution::installed()?;
    let expected_build = current.as_ref().map(|r| r.active.build_id.clone());
    let temp = tempfile::tempdir().map_err(|e| format!("stage update: {e}"))?;
    let agent = ureq::AgentBuilder::new().timeout_connect(std::time::Duration::from_secs(15)).build();
    let platform = release::platform().map_err(|e| e.to_string())?;
    release::download_package(&agent, release::DOWNLOAD_URL, tag_name, &platform, channel, temp.path()).map_err(|e| e.to_string())?;
    let package = Package::load(&temp.path().join("package")).map_err(|e| e.to_string())?;
    if package.manifest.tag != tag_name { return Err("downloaded package tag differs from requested update".into()); }
    let options = match current {
        Some(r) => transaction::InstallOptions { channel: r.channel, root: r.root, bin_dir: r.bin_dir, applications_dir: r.applications_dir },
        None => transaction::InstallOptions::for_channel(channel).map_err(|e| e.to_string())?,
    };
    let receipt = match expected_build {
        Some(expected) => transaction::update(&package, options, &expected),
        None => transaction::install(&package, options),
    }.map_err(|e| e.to_string())?;
    log::info!("update: verified installed build={} tag={}", receipt.active.build_id, receipt.active.tag);
    Ok(())
}

/// `plexi update` — thin CLI wrapper around `run_self_update`.
pub fn self_update_cli(rollback: bool) -> i32 {
    let result = if rollback {
        crate::distribution::installed().and_then(|r| r.ok_or_else(|| "no managed installation".to_string()))
            .and_then(|r| plexi_distribution::transaction::rollback(&r.root).map_err(|e| e.to_string()))
            .map(|r| format!("Restored {}. Restart Plexi to apply.", r.active.tag))
    } else { run_self_update() };
    match result {
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
