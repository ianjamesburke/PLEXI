use std::path::PathBuf;

pub fn completions_sentinel_path() -> PathBuf {
    crate::config::config_dir().join("completions_setup_done")
}

pub fn completions_was_prompted() -> bool {
    completions_sentinel_path().exists()
}

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
        let zfunc_ok = dirs::home_dir()
            .map(|h| h.join(".zfunc").join(format!("_{cli}")).exists())
            .unwrap_or(false);
        if zfunc_ok {
            log::info!("cli_setup: completions found in ~/.zfunc for {cli}");
        } else {
            log::info!("cli_setup: zsh completions not found for {cli}");
        }
        zfunc_ok
    } else if shell.contains("bash") {
        let found = dirs::home_dir()
            .map(|h| h.join(".bash_completion.d").join(&cli).exists())
            .unwrap_or(false);
        log::info!("cli_setup: bash completions found={found} for {cli}");
        found
    } else if shell.contains("fish") {
        let found = dirs::home_dir()
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
    crate::config::current_exe_basename()
}

fn install_path() -> PathBuf {
    PathBuf::from("/usr/local/bin").join(cli_name())
}

pub fn sentinel_path() -> PathBuf {
    crate::config::config_dir().join("cli_setup_done")
}

pub fn was_prompted() -> bool {
    sentinel_path().exists()
}

pub fn mark_prompted() {
    let _ = std::fs::write(sentinel_path(), "");
}

/// Install this channel without trying to launch a second host.
pub fn install_command() -> String {
    install_command_for(
        crate::config::build_channel()
            .as_deref()
            .unwrap_or("stable"),
        cfg!(windows),
    )
}

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

pub fn completions_command() -> String {
    let shell = std::env::var("SHELL").unwrap_or_default();
    let shell = if shell.contains("fish") {
        "fish"
    } else if shell.contains("bash") {
        "bash"
    } else {
        "zsh"
    };
    format!("{} completions {shell}", cli_name())
}

/// Shows every launch until the CLI is verified installed.
/// "Not now" and Escape dismiss for the session only.
///
/// If the sentinel exists but the binary isn't installed (e.g. profile was
/// migrated from another machine), the stale sentinel is cleared and the
/// prompt is shown again.
pub fn should_prompt() -> bool {
    if is_installed() {
        // CLI is present. Write the sentinel if it's missing so we don't
        // prompt again, then skip.
        if !was_prompted() {
            log::info!("cli_setup: {} installed — writing sentinel", cli_name());
            mark_prompted();
        }
        log::info!(
            "cli_setup: {} already installed — skipping prompt",
            cli_name()
        );
        return false;
    }
    if was_prompted() {
        // Sentinel was set on a different machine or the symlink was deleted.
        // Clear it so the prompt shows.
        log::info!(
            "cli_setup: stale sentinel found but {} not installed — clearing and re-prompting",
            cli_name()
        );
        let _ = std::fs::remove_file(sentinel_path());
    }
    log::info!("cli_setup: {} not installed — showing prompt", cli_name());
    true
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
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(&name).exists()))
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
}
