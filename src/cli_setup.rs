use std::os::unix::fs::symlink;
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

/// Shows every launch until the user clicks Install (writes sentinel).
/// "Not now" and Escape dismiss for the session only.
///
/// If the sentinel exists but the binary isn't installed (e.g. profile was
/// migrated from another machine), the stale sentinel is cleared and the
/// prompt is shown again.
pub fn should_prompt() -> bool {
    if is_installed() {
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

/// Note: `just pr-install` always creates `/usr/local/bin/plexi-pr-<N>` as part of its
/// setup, so this returns `true` in every PR build. To test the CLI setup modal in a PR
/// build, first remove that symlink manually before opening the app.
pub fn is_installed() -> bool {
    install_path().exists()
}

/// Symlink `/usr/local/bin/<cli_name>` → current binary.
pub fn install_symlink() -> Result<String, String> {
    let name = cli_name();
    let link_path = install_path();
    let current_binary =
        std::env::current_exe().map_err(|e| format!("could not locate binary: {e}"))?;

    if link_path.exists() || link_path.symlink_metadata().is_ok() {
        std::fs::remove_file(&link_path)
            .map_err(|e| format!("could not remove existing link: {e}"))?;
    }
    symlink(&current_binary, &link_path)
        .map_err(|e| format!("could not create /usr/local/bin/{name}: {e}"))?;

    Ok(format!("/usr/local/bin/{name} → {}", current_binary.display()))
}
