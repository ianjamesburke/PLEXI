use serde::Serialize;
use std::path::{Path, PathBuf};

fn user_home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(dirs::home_dir)
}

#[cfg(unix)]
fn quote_shell(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

#[cfg(test)]
pub fn completions_sentinel_path() -> PathBuf {
    crate::config::config_dir().join("completions_setup_done")
}

#[cfg(test)]
pub fn completions_was_prompted() -> bool {
    completions_sentinel_path().exists()
}

#[cfg(test)]
pub fn completions_mark_prompted() {
    let _ = std::fs::write(completions_sentinel_path(), "");
}

/// Check whether shell completions for this build are installed.
///
/// Detects the current shell from `$SHELL` and checks the expected location.
/// Returns `true` for unknown shells to avoid a spurious banner.
pub fn completions_installed() -> bool {
    let cli = cli_name();
    let shell = std::env::var("SHELL").unwrap_or_default();

    if let Ok(Some(receipt)) = crate::distribution::installed() {
        let directory = receipt.root.join("completions");
        let path = if shell.contains("zsh") {
            directory.join("zsh").join(format!("_{cli}"))
        } else if shell.contains("bash") {
            directory.join("bash").join(&cli)
        } else {
            directory.join("fish").join(format!("{cli}.fish"))
        };
        if path.is_file() {
            log::info!("cli_setup: managed completions found for {cli}");
            return true;
        }
    }

    if shell.contains("zsh") {
        let managed_zsh = crate::config::config_dir()
            .join("shell-integration/zsh/completions")
            .join(format!("_{cli}"));
        if managed_zsh.is_file() {
            log::info!("cli_setup: zsh completions found in shell-integration for {cli}");
            return true;
        }
        // Prefer Homebrew site-functions; fall back to ~/.zfunc
        let brew_ok = std::process::Command::new("brew")
            .arg("--prefix")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|prefix| {
                PathBuf::from(prefix.trim())
                    .join("share/zsh/site-functions")
                    .join(format!("_{cli}"))
                    .exists()
            })
            .unwrap_or(false);
        if brew_ok {
            log::info!("cli_setup: completions found in brew site-functions for {cli}");
            return true;
        }
        let zfunc_ok = user_home_dir()
            .map(|h| h.join(".zfunc").join(format!("_{cli}")).exists())
            .unwrap_or(false);
        if zfunc_ok {
            log::info!("cli_setup: completions found in ~/.zfunc for {cli}");
        } else {
            log::info!("cli_setup: zsh completions not found for {cli}");
        }
        zfunc_ok
    } else if shell.contains("bash") {
        let xdg_ok = {
            #[cfg(unix)]
            {
                plexi_distribution::transaction::dropin_completion_paths(&cli, "bash")
                    .iter()
                    .any(|p| p.is_file())
            }
            #[cfg(not(unix))]
            {
                false
            }
        };
        let found = xdg_ok
            || user_home_dir()
                .map(|h| h.join(".bash_completion.d").join(&cli).exists())
                .unwrap_or(false);
        log::info!("cli_setup: bash completions found={found} for {cli}");
        found
    } else if shell.contains("fish") {
        let xdg_ok = {
            #[cfg(unix)]
            {
                plexi_distribution::transaction::dropin_completion_paths(&cli, "fish")
                    .iter()
                    .any(|p| p.is_file())
            }
            #[cfg(not(unix))]
            {
                false
            }
        };
        let found = xdg_ok
            || user_home_dir()
                .map(|h| {
                    h.join(".config/fish/completions")
                        .join(format!("{cli}.fish"))
                        .exists()
                })
                .unwrap_or(false);
        log::info!("cli_setup: fish completions found={found} for {cli}");
        found
    } else {
        log::info!("cli_setup: unknown shell {shell:?} — skipping completions check");
        true
    }
}

/// Show the completions banner once when the CLI is installed but completions are not.
/// Dismissed permanently via sentinel; session-only via the banner's "Not now" button.
#[cfg(test)]
pub fn should_prompt_completions() -> bool {
    if crate::config::build_channel().is_some_and(|channel| channel.starts_with("pr-")) {
        log::info!("cli_setup: PR build — skipping completions banner");
        return false;
    }
    if completions_was_prompted() {
        log::info!("cli_setup: completions sentinel present — skipping banner");
        return false;
    }
    if !is_installed() {
        // Wait until the CLI is installed before nudging about completions.
        return false;
    }
    if completions_installed() {
        log::info!("cli_setup: completions already installed — writing sentinel");
        completions_mark_prompted();
        return false;
    }
    log::info!("cli_setup: completions not installed — showing banner");
    true
}

/// CLI name for the running build variant (e.g. `plexi`, `plexi-alpha`).
pub fn cli_name() -> String {
    if let Some(channel) = crate::config::build_channel() {
        return plexi_distribution::release::command_name(&channel);
    }
    crate::config::current_exe_basename()
}

fn install_path() -> PathBuf {
    PathBuf::from("/usr/local/bin").join(cli_name())
}

#[cfg(test)]
fn install_command_for(channel: &str, windows: bool) -> String {
    if windows {
        format!(
            "& ([scriptblock]::Create((irm https://plexiapp.com/install.ps1))) -Channel {channel} -InstallOnly"
        )
    } else {
        format!(
            "curl -fsSL https://plexiapp.com/install | bash -s -- --channel {channel} --install-only"
        )
    }
}

/// Managed installs use the receipt's owned launcher, including custom paths.
/// Unmanaged legacy installs retain PATH discovery.
pub fn is_installed() -> bool {
    match crate::distribution::installed() {
        Ok(Some(receipt)) => {
            let name = if cfg!(windows) {
                format!(
                    "{}.exe",
                    plexi_distribution::release::command_name(&receipt.channel)
                )
            } else {
                plexi_distribution::release::command_name(&receipt.channel)
            };
            let launcher = receipt.bin_dir.join(name);
            return receipt.integrations.iter().any(|owned| {
                owned.path == launcher
                    && match owned.matches() {
                        Ok(matches) => matches,
                        Err(error) => {
                            log::error!("cli_setup: launcher verification failed: {error}");
                            false
                        }
                    }
            });
        }
        Ok(None) => {}
        Err(error) => {
            log::error!("cli_setup: installation receipt failed: {error}");
            return false;
        }
    }
    if install_path().exists() {
        return true;
    }
    let name = cli_name();
    if user_home_dir().is_some_and(|h| h.join(".local/bin").join(&name).is_file()) {
        return true;
    }
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(&name).exists()))
}

#[derive(Debug, Clone, Serialize)]
pub struct CliSetupReport {
    pub cli_name: String,
    pub shim_path: PathBuf,
    pub dropin_completions: Vec<PathBuf>,
    pub path_modified: bool,
    pub modified_rc_files: Vec<PathBuf>,
}

/// Silently self-heal the local CLI shim (`~/.local/bin/<cli_name>`) and
/// user-local drop-in completions (`XDG_DATA_HOME/fish/vendor_completions.d`,
/// `XDG_DATA_HOME/bash-completion/completions`, and managed zsh shell
/// integration) without touching any user dotfiles (`.zshrc`/`.bashrc`/`.profile`).
pub fn ensure_local_shim() -> Result<PathBuf, String> {
    let cli = cli_name();
    let _ = crate::host::shell::ensure_shell_integration();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let (launcher, script, channel) = match crate::distribution::installed() {
            Ok(Some(receipt)) => {
                let name = plexi_distribution::release::command_name(&receipt.channel);
                let installer = receipt.active.path.join("plexi-installer");
                let script = format!(
                    "#!/bin/sh\nexec {} launch --receipt {} -- \"$@\"\n",
                    quote_shell(&installer),
                    quote_shell(&receipt.root)
                );
                (receipt.bin_dir.join(&name), script, receipt.channel.clone())
            }
            _ => {
                let home = user_home_dir()
                    .ok_or_else(|| "cannot resolve user home directory".to_string())?;
                let exe = std::env::current_exe()
                    .map_err(|e| format!("resolve current executable: {e}"))?;
                let script = format!("#!/bin/sh\nexec {} \"$@\"\n", quote_shell(&exe));
                let channel = crate::config::build_channel().unwrap_or_else(|| "stable".into());
                (home.join(".local/bin").join(&cli), script, channel)
            }
        };
        if !launcher.exists() && !launcher.is_symlink() {
            if let Some(parent) = launcher.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("create {}: {e}", parent.display()))?;
            }
            std::fs::write(&launcher, script.as_bytes())
                .map_err(|e| format!("write shim {}: {e}", launcher.display()))?;
            let _ = std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(0o755));
            log::info!("cli_setup: self-healed local shim at {}", launcher.display());
        }
        if !channel.starts_with("pr-") {
            let name = plexi_distribution::release::command_name(&channel);
            for shell in ["fish", "bash", "zsh"] {
                for dropin in
                    plexi_distribution::transaction::dropin_completion_paths(&name, shell)
                {
                    if !dropin.exists() && !dropin.is_symlink() {
                        if let Ok(bytes) = crate::cli::completions::render_completions(shell, &name)
                        {
                            if let Some(parent) = dropin.parent() {
                                let _ = std::fs::create_dir_all(parent);
                            }
                            let _ = std::fs::write(&dropin, bytes);
                        }
                    }
                }
            }
        }
        Ok(launcher)
    }
    #[cfg(not(unix))]
    {
        if let Ok(Some(receipt)) = crate::distribution::installed() {
            let name = format!(
                "{}.exe",
                plexi_distribution::release::command_name(&receipt.channel)
            );
            return Ok(receipt.bin_dir.join(name));
        }
        let home =
            user_home_dir().ok_or_else(|| "cannot resolve user home directory".to_string())?;
        Ok(home.join(".local/bin").join(&cli))
    }
}

/// Explicitly set up the CLI shim, drop-in completions, and (when `modify_path`
/// is true) shell rc files (`.zshrc`, `.bashrc`, `.profile`, `config.fish`).
pub fn setup_external_shell(modify_path: bool) -> Result<CliSetupReport, String> {
    let cli = cli_name();
    let shim_path = ensure_local_shim()?;
    let mut dropin_completions = Vec::new();
    #[cfg(unix)]
    {
        let channel = crate::config::build_channel().unwrap_or_else(|| "stable".into());
        if !channel.starts_with("pr-") {
            let name = plexi_distribution::release::command_name(&channel);
            for shell in ["fish", "bash", "zsh"] {
                for dropin in
                    plexi_distribution::transaction::dropin_completion_paths(&name, shell)
                {
                    if let Ok(bytes) = crate::cli::completions::render_completions(shell, &name) {
                        if let Some(parent) = dropin.parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        if !dropin.is_symlink() && std::fs::write(&dropin, bytes).is_ok() {
                            dropin_completions.push(dropin);
                        }
                    }
                }
            }
        }
    }
    let mut modified_rc_files = Vec::new();
    #[cfg(unix)]
    if modify_path {
        if let Ok(Some(receipt)) = crate::distribution::installed() {
            let updated = plexi_distribution::transaction::register_path(&receipt.root)
                .map_err(|e| e.to_string())?;
            for edit in updated.path_edits {
                if !modified_rc_files.contains(&edit.path) {
                    modified_rc_files.push(edit.path);
                }
            }
        } else {
            let home =
                user_home_dir().ok_or_else(|| "cannot resolve user home directory".to_string())?;
            let bin_dir = shim_path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| home.join(".local/bin"));
            let marker = format!("# Plexi command path: {}", bin_dir.display());
            let block = format!(
                "\n{marker}\nexport PATH={}:\"$PATH\"\n# End Plexi command path\n",
                quote_shell(&bin_dir)
            );
            for rc in [
                home.join(".profile"),
                home.join(".zprofile"),
                home.join(".zshrc"),
                home.join(".bashrc"),
            ] {
                let mut existing = std::fs::read_to_string(&rc).unwrap_or_default();
                if !existing.contains(&marker) {
                    existing.push_str(&block);
                    if std::fs::write(&rc, existing).is_ok() {
                        modified_rc_files.push(rc);
                    }
                }
            }
        }
    }
    Ok(CliSetupReport {
        cli_name: cli,
        shim_path,
        dropin_completions,
        path_modified: modify_path,
        modified_rc_files,
    })
}

pub fn run_cli_setup(modify_path: bool, json: bool) -> i32 {
    match setup_external_shell(modify_path) {
        Ok(report) => {
            if json {
                if let Ok(s) = serde_json::to_string_pretty(&report) {
                    println!("{s}");
                }
            } else {
                println!(
                    "Installed `{}` launcher at {}",
                    report.cli_name,
                    report.shim_path.display()
                );
                for comp in &report.dropin_completions {
                    println!("Installed completion at {}", comp.display());
                }
                if report.path_modified {
                    if report.modified_rc_files.is_empty() {
                        println!("Shell PATH configuration is already up to date.");
                    } else {
                        for rc in &report.modified_rc_files {
                            println!("Updated shell profile {}", rc.display());
                        }
                        println!("Open a new terminal window to refresh PATH.");
                    }
                } else {
                    println!(
                        "Shell dotfiles were not modified (pass `--modify-path` to add {} to ~/.zshrc / ~/.bashrc).",
                        report
                            .shim_path
                            .parent()
                            .unwrap_or_else(|| Path::new("~/.local/bin"))
                            .display()
                    );
                }
            }
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

pub fn external_shell_status() -> serde_json::Value {
    let cli = cli_name();
    let home = user_home_dir();
    let shim_path = match crate::distribution::installed() {
        Ok(Some(ref r)) => r
            .bin_dir
            .join(plexi_distribution::release::command_name(&r.channel)),
        _ => home
            .as_ref()
            .map(|h| h.join(".local/bin").join(&cli))
            .unwrap_or_else(|| PathBuf::from(&cli)),
    };
    let shim_exists = shim_path.is_file() || is_installed();
    let bin_dir = shim_path.parent().map(Path::to_path_buf);
    let in_path = bin_dir.as_ref().is_some_and(|dir| {
        std::env::var_os("PATH")
            .is_some_and(|path| std::env::split_paths(&path).any(|p| p == *dir))
    });
    let rc_configured = crate::distribution::installed()
        .ok()
        .flatten()
        .is_some_and(|r| !r.path_edits.is_empty())
        || home.as_ref().is_some_and(|h| {
            [".zshrc", ".bashrc", ".profile", ".zprofile"]
                .iter()
                .any(|f| {
                    std::fs::read_to_string(h.join(f))
                        .is_ok_and(|s| s.contains("# Plexi command path"))
                })
        });
    serde_json::json!({
        "cli_name": cli,
        "shim_path": shim_path,
        "shim_installed": shim_exists,
        "in_path": in_path || rc_configured,
        "rc_path_configured": rc_configured,
        "completions_installed": completions_installed(),
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn install_instructions_preserve_channel_and_do_not_launch_again() {
        for channel in ["stable", "beta", "alpha"] {
            let unix = super::install_command_for(channel, false);
            assert!(unix.contains(&format!("--channel {channel} --install-only")));
            assert!(unix.contains("| bash -s --"));
            let windows = super::install_command_for(channel, true);
            assert!(windows.contains("install.ps1"));
            assert!(windows.ends_with(&format!("-Channel {channel} -InstallOnly")));
        }
    }

    #[test]
    fn pr_builds_skip_completions_prompt() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let _profile_guard = crate::config::set_test_profile_dir(tmp.path().to_path_buf());
        let _channel_guard = crate::config::set_test_channel("pr-2265");

        assert!(!super::should_prompt_completions());
        assert!(
            !super::completions_sentinel_path().exists(),
            "skipping a PR prompt should not create profile state"
        );
    }

    #[cfg(unix)]
    #[test]
    fn ensure_local_shim_and_explicit_setup_respect_zero_dotfile_default() {
        let _env_lock = crate::cli::test_env::socket_env_guard();
        let tmp = tempfile::tempdir().expect("tempdir");
        let home = tmp.path().join("home");
        let xdg_data = tmp.path().join("xdg_data");
        let dist_home = tmp.path().join("dist_home");
        let profile = tmp.path().join("profile");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&xdg_data).unwrap();
        std::fs::create_dir_all(&dist_home).unwrap();
        std::fs::create_dir_all(&profile).unwrap();

        let _profile_guard = crate::config::set_test_profile_dir(profile.clone());
        let _channel_guard = crate::config::set_test_channel("alpha");

        let prev_home = std::env::var_os("HOME");
        let prev_xdg = std::env::var_os("XDG_DATA_HOME");
        let prev_dist = std::env::var_os("PLEXI_DISTRIBUTION_HOME");
        // SAFETY: serialized via crate::cli::test_env::lock()
        unsafe {
            std::env::set_var("HOME", &home);
            std::env::set_var("XDG_DATA_HOME", &xdg_data);
            std::env::set_var("PLEXI_DISTRIBUTION_HOME", &dist_home);
        }

        // 1. Self-heal shim + drop-in completions must NOT touch .zshrc/.bashrc/.profile.
        let shim = super::ensure_local_shim().expect("ensure_local_shim");
        assert!(shim.is_file(), "local shim must be created at {shim:?}");
        assert!(
            xdg_data
                .join("fish/vendor_completions.d/plexi-alpha.fish")
                .is_file()
        );
        assert!(
            xdg_data
                .join("bash-completion/completions/plexi-alpha")
                .is_file()
        );
        assert!(
            profile
                .join("shell-integration/zsh/completions/_plexi-alpha")
                .is_file()
        );
        assert!(!home.join(".zshrc").exists());
        assert!(!home.join(".bashrc").exists());
        assert!(!home.join(".profile").exists());

        // 2. Explicit setup with modify_path=false keeps dotfiles untouched.
        let report_no_rc = super::setup_external_shell(false).expect("setup without rc edits");
        assert!(!report_no_rc.path_modified);
        assert!(report_no_rc.modified_rc_files.is_empty());
        assert!(!home.join(".zshrc").exists());

        // 3. Explicit setup with modify_path=true writes the command path block to dotfiles.
        let report_rc = super::setup_external_shell(true).expect("setup with rc edits");
        assert!(report_rc.path_modified);
        assert!(!report_rc.modified_rc_files.is_empty());
        let zshrc = std::fs::read_to_string(home.join(".zshrc")).expect("zshrc written");
        assert!(zshrc.contains("# Plexi command path:"));

        // Restore env
        unsafe {
            match prev_home {
                Some(v) => std::env::set_var("HOME", v),
                None => std::env::remove_var("HOME"),
            }
            match prev_xdg {
                Some(v) => std::env::set_var("XDG_DATA_HOME", v),
                None => std::env::remove_var("XDG_DATA_HOME"),
            }
            match prev_dist {
                Some(v) => std::env::set_var("PLEXI_DISTRIBUTION_HOME", v),
                None => std::env::remove_var("PLEXI_DISTRIBUTION_HOME"),
            }
        }
    }
}
