use std::path::PathBuf;

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
        log::info!("cli_setup: {} already installed — skipping prompt", cli_name());
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
