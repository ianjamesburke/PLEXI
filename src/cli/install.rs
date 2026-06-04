use super::{print_tip};
use super::app::{is_bare_id, resolve_registry_id, is_github_shorthand, app_is_python, ensure_plexi_sdk};
use std::io::{self, Write};

pub fn install_cli(spec: &str) -> i32 {
    let (source_str, git_ref) = crate::cli::install_host::split_source_and_ref(spec);
    let resolved = if is_bare_id(&source_str) {
        match resolve_registry_id(&source_str) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("error: {e}");
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
    match crate::cli::install_host::install_one(&cloner, &source, git_ref.as_deref(), &target_root) {
        Ok(outcome) => match outcome.status {
            crate::cli::install_host::InstallStatus::Installed(path) => {
                println!("installed '{}' at {}", outcome.id, path.display());
                if app_is_python(&path) {
                    ensure_plexi_sdk();
                }
                print_tip(&format!("open your app with `plexi app open {}`.", outcome.id));
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

/// `plexi install` with no args — detect `.plexi/apps.toml` and apply it.
///
/// Walks up from CWD looking for `.plexi/` (the workspace marker), reads
/// `apps.toml` from it, and installs declared apps into the workspace-scoped
/// channel apps dir (`<workspace_root>/<channel_dir>/apps/`).
pub fn install_workspace_pack_cli() -> i32 {
    log::info!("cli: install_workspace_pack (no-args flow)");
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => { eprintln!("error: {e}"); return 1; }
    };

    // Walk up from CWD looking for `.plexi/` (workspace marker).
    let workspace_root = {
        let home = dirs::home_dir();
        let mut current = cwd.clone();
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
            if current.join(".plexi").is_dir() {
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
        eprintln!("Usage: plexi app install <source-spec>[@ref] | plexi app install --pack <path|core>");
        eprintln!("  In a workspace (directory with .plexi/apps.toml), `plexi app install` applies the manifest.");
        eprintln!("  Run `plexi workspace init` to initialize a workspace here.");
        return 1;
    };

    let apps_toml = root.join(".plexi").join("apps.toml");
    if !apps_toml.exists() {
        eprintln!("no .plexi/apps.toml found in workspace at {}", root.display());
        eprintln!("  Declare app dependencies there, then re-run `plexi app install`.");
        eprintln!("  Usage: plexi app install <source-spec>[@ref] | plexi app install --pack <path|core>");
        return 1;
    }

    log::info!("install_workspace_pack:cli: applying {}", apps_toml.display());
    println!("Applying workspace manifest {}...", apps_toml.display());

    let cloner = crate::cli::install_host::GitCloner;
    let outcomes = match crate::cli::install_host::apply_workspace_pack(&root, &cloner) {
        Ok(o) => o,
        Err(e) => { eprintln!("error: {e}"); return 1; }
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
            crate::cli::install_host::InstallStatus::SkippedOtherVersion { installed, requested } => {
                println!("  skipped    {:30} (installed {installed}, requested {requested})", o.id);
            }
            crate::cli::install_host::InstallStatus::Failed(msg) => {
                eprintln!("  FAILED     {:30} {msg}", o.id);
                any_failed = true;
            }
        }
    }
    if any_failed { 1 } else { 0 }
}

/// `plexi uninstall [--keep-data] [--yes]` — remove Plexi itself from the Mac.
pub fn plexi_uninstall_cli(keep_data: bool, assume_yes: bool) -> i32 {
    // Detect channel suffix from binary name (e.g. "plexi-alpha" → "-alpha", "plexi" → "")
    let exe = std::env::current_exe().unwrap_or_default();
    let binary_name = exe.file_stem().unwrap_or_default().to_string_lossy().to_string();
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
            "-beta"  => " Beta".to_string(),
            _        => String::new(),
        }
    };
    let cap = cap_owned.as_str();

    let profile_dir = dirs::home_dir().unwrap().join(format!(".plexi{suffix}"));
    let app_bundle  = std::path::PathBuf::from(format!("/Applications/Plexi{cap}.app"));
    let cli_binary  = std::path::PathBuf::from(format!("/usr/local/bin/plexi{suffix}"));

    // Single confirmation prompt: keep data or remove everything?
    // Resolved before the banner so the preview accurately reflects the outcome.
    let keep_data = if keep_data || !profile_dir.exists() {
        log::info!("uninstall: keep_data=flag({keep_data}) profile_exists={}", profile_dir.exists());
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
    if app_bundle.exists()  { println!("  \u{2022} {}", app_bundle.display()); }
    if cli_binary.exists()  { println!("  \u{2022} {}", cli_binary.display()); }
    if !keep_data && profile_dir.exists() {
        println!("  \u{2022} {}  (settings, secrets, app configs)", profile_dir.display());
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
            let archive = dirs::home_dir().unwrap().join(format!(
                "plexi-backlog-archive/plexi{suffix}-backlog-{ts}"
            ));
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
            Ok(()) => { println!("Removed {}", app_bundle.display()); removed = true; }
            Err(e) => eprintln!("warning: could not remove {}: {e}", app_bundle.display()),
        }
    }

    // Remove CLI binary
    if cli_binary.exists() || cli_binary.is_symlink() {
        match std::fs::remove_file(&cli_binary) {
            Ok(()) => { println!("Removed {}", cli_binary.display()); removed = true; }
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
            Ok(()) => { println!("Removed {}", profile_dir.display()); removed = true; }
            Err(e) => eprintln!("warning: could not remove {}: {e}", profile_dir.display()),
        }
    }

    if removed {
        println!("\nDone. Plexi{} has been removed.", if cap.is_empty() { "" } else { cap });
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

/// `plexi update` — download and install the latest Plexi release from GitHub.
/// Only supports main channel. Alpha (dev) and PR builds must use `just install`.
/// Beta builds require a channel-renamed bundle that can't yet be produced without
/// the install script, so they are also unsupported here.
pub fn self_update_cli() -> i32 {
    use std::io::Read;

    // Detect channel from binary name (mirrors config_dir_name in config.rs).
    let binary = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
    let binary_name = binary.as_deref().unwrap_or("plexi");

    // Derive channel-specific values from the binary name.
    // Mirrors the channel detection in plexi_uninstall_cli() and install.sh.
    let suffix = if binary_name == "plexi" {
        String::new() // main channel
    } else {
        binary_name.strip_prefix("plexi").unwrap_or("-").to_string()
    };
    let channel = if suffix.is_empty() {
        "main".to_string()
    } else {
        suffix.strip_prefix('-').unwrap_or("unknown").to_string()
    };
    let cap = if let Some(n) = suffix.strip_prefix("-pr-") {
        format!(" PR{n}")
    } else {
        match suffix.as_str() {
            "-alpha" => " Alpha".to_string(),
            "-beta"  => " Beta".to_string(),
            _         => String::new(),
        }
    };
    let display = format!("Plexi{cap}");
    let bundle_id = format!("com.ianjamesburke.plexi{suffix}");
    log::info!("cli: self-update channel={channel} suffix={suffix} display={display}");

    if binary_name.contains("alpha") || binary_name.contains("pr-") {
        eprintln!("Self-update is not available for dev builds.");
        eprintln!("Update from source: git pull && just install");
        return 1;
    }

    let current_version = env!("CARGO_PKG_VERSION");
    println!("Checking for updates...");
    println!("Current: v{current_version}");

    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .build();

    let release_body = match agent
        .get("https://api.github.com/repos/ianjamesburke/PLEXI/releases/latest")
        .set("User-Agent", "plexi-self-update")
        .set("Accept", "application/vnd.github+json")
        .call()
    {
        Ok(r) => match r.into_string() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("error: failed to read release response: {e}");
                return 1;
            }
        },
        Err(e) => {
            eprintln!("error: failed to fetch release info: {e}");
            return 1;
        }
    };

    let release: serde_json::Value = match serde_json::from_str(&release_body) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: failed to parse release response: {e}");
            return 1;
        }
    };

    let tag_name = match release["tag_name"].as_str() {
        Some(t) => t.to_string(),
        None => {
            eprintln!("error: release has no tag_name");
            return 1;
        }
    };

    let latest_version = tag_name.trim_start_matches('v');
    if latest_version == current_version {
        println!("Already up to date (v{current_version}).");
        return 0;
    }
    println!("Latest:  {tag_name}");

    // Find the zip asset in the release.
    let asset_name = format!("Plexi-{tag_name}.zip");
    let download_url = match release["assets"]
        .as_array()
        .and_then(|arr| {
            arr.iter()
                .find(|a| a["name"].as_str() == Some(asset_name.as_str()))
        })
        .and_then(|a| a["browser_download_url"].as_str())
    {
        Some(url) => url.to_string(),
        None => {
            eprintln!("error: no asset named {asset_name} in release {tag_name}");
            eprintln!(
                "Check: https://github.com/ianjamesburke/PLEXI/releases/tag/{tag_name}"
            );
            return 1;
        }
    };

    // Determine the installed app bundle path from current_exe():
    // .../Plexi.app/Contents/MacOS/plexi  →  walk up 3 levels
    let current_exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error: could not determine current binary path: {e}");
            return 1;
        }
    };
    let app_bundle = current_exe
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .filter(|p| p.extension().map_or(false, |e| e == "app"))
        .map(|p| p.to_path_buf());
    let app_bundle = match app_bundle {
        Some(p) => p,
        None => {
            log::info!("cli: self-update skipped — not a bundle install");
            println!("Self-update requires a bundled .app installation.");
            println!("For a dev install, update from source: git pull && just install");
            return 0;
        }
    };

    println!("Downloading {asset_name}...");

    let download_resp = match agent
        .get(&download_url)
        .set("User-Agent", "plexi-self-update")
        .call()
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: failed to download {asset_name}: {e}");
            return 1;
        }
    };

    // Write zip to a temp directory.
    let tmp_dir = std::env::temp_dir().join("plexi-update");
    let _ = std::fs::remove_dir_all(&tmp_dir);
    if let Err(e) = std::fs::create_dir_all(&tmp_dir) {
        eprintln!("error: failed to create temp dir: {e}");
        return 1;
    }
    let zip_path = tmp_dir.join(&asset_name);
    let mut zip_file = match std::fs::File::create(&zip_path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("error: failed to create temp file: {e}");
            return 1;
        }
    };
    let mut buf = Vec::new();
    if let Err(e) = download_resp.into_reader().read_to_end(&mut buf) {
        eprintln!("error: failed to download file: {e}");
        return 1;
    }
    if let Err(e) = std::io::Write::write_all(&mut zip_file, &buf) {
        eprintln!("error: failed to write download to disk: {e}");
        return 1;
    }
    drop(zip_file);

    // Extract using system unzip.
    println!("Installing...");
    let extract_dir = tmp_dir.join("extracted");
    let _ = std::fs::create_dir_all(&extract_dir);
    let unzip_out = std::process::Command::new("unzip")
        .arg("-o")
        .arg(&zip_path)
        .arg("-d")
        .arg(&extract_dir)
        .output();
    match unzip_out {
        Ok(out) if out.status.success() => {}
        Ok(out) => {
            eprintln!("error: unzip failed: {}", String::from_utf8_lossy(&out.stderr));
            return 1;
        }
        Err(e) => {
            eprintln!("error: failed to run unzip: {e}");
            return 1;
        }
    }

    let extracted_app = extract_dir.join("Plexi.app");
    if !extracted_app.is_dir() {
        eprintln!("error: Plexi.app not found in downloaded archive");
        return 1;
    }

    // Replace the installed app bundle. Write to a temp path first so that
    // if cp fails we still have the old bundle to fall back to.
    let app_parent = app_bundle.parent().unwrap_or_else(|| std::path::Path::new("/Applications"));
    let staging = app_parent.join("Plexi.app.update-staging");
    let _ = std::fs::remove_dir_all(&staging);
    let cp_stage = std::process::Command::new("cp")
        .arg("-R")
        .arg(&extracted_app)
        .arg(&staging)
        .output();
    match cp_stage {
        Ok(out) if out.status.success() => {}
        Ok(out) => {
            eprintln!(
                "error: failed to stage new app (permission denied?): {}",
                String::from_utf8_lossy(&out.stderr)
            );
            eprintln!("Run with sudo if /Applications is not user-writable.");
            let _ = std::fs::remove_dir_all(&tmp_dir);
            return 1;
        }
        Err(e) => {
            eprintln!("error: failed to run cp: {e}");
            return 1;
        }
    }

    // For non-main channels, patch the bundle's Info.plist and rename the binary
    // inside it. This mirrors the per-channel patching in scripts/install.sh.
    // If any patch fails, abort — a misconfigured bundle would break the channel.
    if !suffix.is_empty() {
        log::info!("cli: self-update patching bundle for channel={channel}");
        let plist = staging.join("Contents/Info.plist");
        if !plist.exists() {
            eprintln!("error: Info.plist not found in staged bundle at {}", plist.display());
            let _ = std::fs::remove_dir_all(&staging);
            let _ = std::fs::remove_dir_all(&tmp_dir);
            return 1;
        }
        let plist_str = plist.to_string_lossy();
        for (key, val) in [
            ("CFBundleName", display.as_str()),
            ("CFBundleDisplayName", display.as_str()),
            ("CFBundleIdentifier", bundle_id.as_str()),
            ("CFBundleExecutable", binary_name),
        ] {
            let plutil_out = std::process::Command::new("/usr/bin/plutil")
                .args(["-replace", key, "-string", val, &plist_str])
                .output();
            match plutil_out {
                Ok(out) if out.status.success() => {}
                Ok(out) => {
                    eprintln!("error: plutil -replace {key} failed: {}",
                        String::from_utf8_lossy(&out.stderr));
                    let _ = std::fs::remove_dir_all(&staging);
                    let _ = std::fs::remove_dir_all(&tmp_dir);
                    return 1;
                }
                Err(e) => {
                    eprintln!("error: failed to run plutil: {e}");
                    let _ = std::fs::remove_dir_all(&staging);
                    let _ = std::fs::remove_dir_all(&tmp_dir);
                    return 1;
                }
            }
        }
        // Rename the binary inside the bundle from plexi → plexi-<channel>.
        let macos_dir = staging.join("Contents/MacOS");
        let old_bin = macos_dir.join("plexi");
        let new_bin = macos_dir.join(binary_name);
        if old_bin.exists() && old_bin != new_bin {
            if let Err(e) = std::fs::rename(&old_bin, &new_bin) {
                eprintln!("error: failed to rename binary in bundle: {e}");
                let _ = std::fs::remove_dir_all(&staging);
                let _ = std::fs::remove_dir_all(&tmp_dir);
                return 1;
            }
        }
    }

    // When running inside Plexi the bundle can't be replaced while the app is live.
    // Write a relaunch script, launch it detached, trigger app quit, and exit.
    if std::env::var("PLEXI_RUNNING").as_deref() == Ok("1") {
        let app_display_name = app_bundle
            .file_stem()
            .and_then(|n| n.to_str())
            .unwrap_or("Plexi");
        let script = format!(
            "#!/bin/bash\n\
             while pgrep -x '{binary_name}' > /dev/null 2>&1; do sleep 0.3; done\n\
             rm -rf '{bundle}'\n\
             mv '{staging_path}' '{bundle}'\n\
             ln -sf '{bundle}/Contents/MacOS/{binary_name}' /usr/local/bin/{binary_name} 2>/dev/null || true\n\
             open '{bundle}'\n",
            binary_name = binary_name,
            staging_path = staging.display(),
            bundle = app_bundle.display(),
        );
        let script_path = tmp_dir.join("plexi-relaunch.sh");
        if let Err(e) = std::fs::write(&script_path, &script) {
            eprintln!("error: failed to write relaunch script: {e}");
            let _ = std::fs::remove_dir_all(&staging);
            return 1;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(
                &script_path,
                std::fs::Permissions::from_mode(0o755),
            );
        }
        match std::process::Command::new("nohup")
            .arg("bash")
            .arg(&script_path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(_) => {}
            Err(e) => {
                eprintln!("error: failed to launch relaunch script: {e}");
                let _ = std::fs::remove_dir_all(&staging);
                return 1;
            }
        }
        println!("Plexi will restart to apply the update.");
        let _ = std::process::Command::new("osascript")
            .args([
                "-e",
                &format!("tell application \"{app_display_name}\" to quit"),
            ])
            .status();
        return 0;
    }

    if let Err(e) = std::fs::remove_dir_all(&app_bundle) {
        eprintln!("error: failed to remove old app bundle: {e}");
        eprintln!("Run with sudo if /Applications is not user-writable.");
        let _ = std::fs::remove_dir_all(&staging);
        let _ = std::fs::remove_dir_all(&tmp_dir);
        return 1;
    }
    if let Err(e) = std::fs::rename(&staging, &app_bundle) {
        eprintln!("error: failed to move new app into place: {e}");
        eprintln!(
            "Staged bundle is at {}. Move it manually to {}.",
            staging.display(),
            app_bundle.display()
        );
        return 1;
    }

    // Re-symlink the CLI binary at /usr/local/bin/plexi{suffix} (non-fatal if missing).
    let new_binary = app_bundle.join("Contents/MacOS").join(binary_name);
    let bin_link = std::path::Path::new("/usr/local/bin").join(binary_name);
    if bin_link.is_symlink() || bin_link.exists() {
        let _ = std::fs::remove_file(&bin_link);
        if let Err(e) = std::os::unix::fs::symlink(&new_binary, &bin_link) {
            eprintln!("warning: could not update CLI symlink: {e}");
        }
    }
    // Main channel also owns the bare `plexi` symlink.
    if suffix.is_empty() {
        let bare_link = std::path::Path::new("/usr/local/bin/plexi");
        if bare_link.is_symlink() || bare_link.exists() {
            let _ = std::fs::remove_file(bare_link);
            if let Err(e) = std::os::unix::fs::symlink(&new_binary, bare_link) {
                eprintln!("warning: could not update bare plexi symlink: {e}");
            }
        }
    }

    let _ = std::fs::remove_dir_all(&tmp_dir);
    println!("Installed v{latest_version}. Restart Plexi to apply.");
    0
}
