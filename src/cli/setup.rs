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
    std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "plexi".to_string())
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

/// The install command shown in the CLI setup modal.
pub const INSTALL_COMMAND: &str = "curl -fsSL https://plexiapp.com/install | sh";

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

/// Checks whether the CLI binary is reachable: first at the canonical
/// `/usr/local/bin/<name>` path, then anywhere on `$PATH`. This handles
/// installs into `/opt/homebrew/bin`, `~/.local/bin`, etc.
///
/// Note: `just pr-install` always creates `/usr/local/bin/plexi-pr-<N>` as
/// part of its setup, so this returns `true` in every PR build. To test the
/// CLI setup modal in a PR build, first remove that symlink manually.
pub fn is_installed() -> bool {
    if install_path().exists() {
        return true;
    }
    let name = cli_name();
    std::env::var_os("PATH").map_or(false, |path| {
        std::env::split_paths(&path).any(|dir| dir.join(&name).exists())
    })
}
