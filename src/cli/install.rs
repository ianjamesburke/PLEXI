use super::app::{is_bare_id, is_github_shorthand};
use super::print_tip;
use crate::cli::release_resolver;
use std::io::{self, Write};

pub fn install_cli(spec: &str) -> i32 {
    use crate::cli::marketplace::InstallPlan;
    let (source_str, git_ref) = crate::cli::install_host::split_source_and_ref(spec);
    let resolved = if is_bare_id(&source_str) {
        // A bare id resolves only through the hosted marketplace catalog. A
        // free/licensed app installs from its CDN artifact (or a declared github
        // source); a paid app with no license is blocked; an unknown id or an
        // unreachable registry is a hard error. There is no legacy fallback.
        match crate::cli::marketplace::plan_install(&source_str) {
            InstallPlan::Package(pkg) => {
                return crate::cli::app::app_install_package(
                    &pkg.to_string_lossy(),
                    None,
                    crate::cli::InstallConfirm::Interactive,
                    false,
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
pub fn install_pack_cli(spec: &str) -> i32 {
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
    let outcomes = crate::cli::install_host::apply_pack(&cloner, &pack, &target_root);
    let mut any_failed = false;
    for o in &outcomes {
        match &o.status {
            crate::cli::install_host::InstallStatus::Installed(p) => {
                println!("  installed  {:30} → {}", o.id, p.display());
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

/// `plexi uninstall [--keep-data] [--yes]` — remove Plexi itself from the Mac.
pub fn plexi_uninstall_cli(keep_data: bool, assume_yes: bool) -> i32 {
    // Detect channel suffix from binary name (e.g. "plexi-alpha" → "-alpha", "plexi" → "")
    let exe = std::env::current_exe().unwrap_or_default();
    let binary_name = exe
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let suffix = if binary_name == "plexi" {
        String::new()
    } else {
        binary_name.strip_prefix("plexi").unwrap_or("").to_string()
    };
    let cap_owned = if let Some(n) = suffix.strip_prefix("-pr-") {
        format!(" PR{n}")
    } else {
        match suffix.as_str() {
            "-alpha" => " Alpha".to_string(),
            "-beta" => " Beta".to_string(),
            _ => String::new(),
        }
    };
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
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
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

/// `plexi update apps [<id>]` — git-pull one installed app, or all of them.
/// Apps that aren't git checkouts (e.g. bundled core entries) are skipped
/// with a debug-level log line and reported but not failed.
pub fn update_cli(maybe_id: Option<&str>) -> i32 {
    let target_root = crate::app::registry::apps_dir();
    let cloner = crate::cli::install_host::GitCloner;
    let ids: Vec<String> = match maybe_id {
        Some(id) => vec![id.to_string()],
        None => crate::cli::install_host::installed_versions(&target_root)
            .into_keys()
            .collect(),
    };
    if ids.is_empty() {
        println!("no apps installed");
        return 0;
    }
    let mut any_failed = false;
    for id in ids {
        match crate::cli::install_host::update_one(&cloner, &id, &target_root) {
            Ok(()) => println!("  updated  {id}"),
            Err(e) if e.contains("not a git checkout") => {
                println!("  skipped  {id} (not a git checkout)");
            }
            Err(e) => {
                eprintln!("  FAILED   {id}: {e}");
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

/// Core update logic, callable from both the CLI and the GUI one-click button.
///
/// `from_gui` — when `true`, the caller is the changelog modal running inside the
/// live Plexi process.  The function skips the `PLEXI_RUNNING` env-var check and
/// uses the relaunch-script path unconditionally (the app *is* running).
/// When `false` (CLI), the env-var check is used instead.
///
/// Returns `Ok(human_readable_message)` on success and `Err(error_message)` on
/// failure so callers can surface the outcome appropriately.
/// CLI-only self-update: check for a newer release, build, and install inline.
/// The GUI uses `spawn_update_check` (background build) + restart instead.
fn run_self_update() -> Result<String, String> {
    let binary = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
    let binary_name = binary.as_deref().unwrap_or("plexi");

    let suffix = if binary_name == "plexi" {
        String::new()
    } else {
        binary_name.strip_prefix("plexi").unwrap_or("").to_string()
    };
    let channel = if suffix.is_empty() {
        "main".to_string()
    } else {
        suffix.strip_prefix('-').unwrap_or("unknown").to_string()
    };

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
        .join(format!(".plexi{}", if suffix.is_empty() { String::new() } else { format!("-{channel}") }));
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

    let releases = release_resolver::fetch_releases(&agent)
        .map_err(|e| format!("error: {e}"))?;
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

    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    let src_dir = std::path::PathBuf::from(&home).join(".plexi-src");
    let repo = "https://github.com/ianjamesburke/PLEXI.git";
    let install_script = src_dir.join("scripts/install.sh");

    println!("Updating source...");
    if src_dir.join(".git").is_dir() {
        let fetch = std::process::Command::new("git")
            .args(["-C", &src_dir.to_string_lossy(), "fetch", "origin", "--tags", "--force"])
            .status()
            .map_err(|e| format!("error: git fetch failed: {e}"))?;
        if !fetch.success() {
            return Err("error: git fetch failed".to_string());
        }
    } else {
        println!("Cloning source to ~/.plexi-src...");
        let clone = std::process::Command::new("git")
            .args(["clone", repo, &src_dir.to_string_lossy()])
            .status()
            .map_err(|e| format!("error: git clone failed: {e}"))?;
        if !clone.success() {
            return Err("error: git clone failed".to_string());
        }
    }
    let checkout = std::process::Command::new("git")
        .args(["-C", &src_dir.to_string_lossy(), "checkout", "--force", &tag_name])
        .status()
        .map_err(|e| format!("error: git checkout failed: {e}"))?;
    if !checkout.success() {
        return Err(format!("error: git checkout of tag {tag_name} failed"));
    }

    println!("Building v{latest_version} (this takes a few minutes)...");
    let install = std::process::Command::new("bash")
        .arg(&install_script)
        .arg(&channel)
        .env("PLEXI_INSTALL_TAG", &tag_name)
        .current_dir(&src_dir)
        .status()
        .map_err(|e| format!("error: failed to run install script: {e}"))?;
    if !install.success() {
        return Err("error: install script failed — check output above".to_string());
    }

    Ok(format!("Installed v{latest_version}. Restart Plexi to apply."))
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
