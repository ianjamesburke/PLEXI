use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

/// CLI name for the running build variant (e.g. `plexi`, `plexi-alpha`).
pub fn cli_name() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "plexi".to_string())
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

/// Compute whether to show the first-launch prompt, logging the decision.
pub fn should_prompt() -> bool {
    if was_prompted() {
        return false;
    }
    if is_installed() {
        log::info!("cli_setup: {} already installed — skipping prompt", cli_name());
        return false;
    }
    log::info!("cli_setup: {} not installed — showing first-launch prompt", cli_name());
    true
}

/// Returns true if the CLI command for this build already exists in a
/// well-known install location (avoids spawning a shell on every startup).
pub fn is_installed() -> bool {
    let name = cli_name();
    let home = match dirs::home_dir() {
        Some(h) => h,
        None => return false,
    };
    home.join(".local").join("bin").join(&name).exists()
        || Path::new("/usr/local/bin").join(&name).exists()
}

/// Symlink `~/.local/bin/<cli_name>` → current binary.
/// Creates `~/.local/bin/` and patches `~/.zshrc` if needed.
/// Returns a short status string.
pub fn install_symlink() -> Result<String, String> {
    let name = cli_name();
    let home = dirs::home_dir().ok_or_else(|| "could not determine home directory".to_string())?;
    let bin_dir = home.join(".local").join("bin");
    std::fs::create_dir_all(&bin_dir)
        .map_err(|e| format!("could not create {}: {e}", bin_dir.display()))?;

    let link_path = bin_dir.join(&name);
    let current_binary =
        std::env::current_exe().map_err(|e| format!("could not locate binary: {e}"))?;

    if link_path.exists() || link_path.symlink_metadata().is_ok() {
        std::fs::remove_file(&link_path)
            .map_err(|e| format!("could not remove existing link: {e}"))?;
    }
    symlink(&current_binary, &link_path)
        .map_err(|e| format!("could not create symlink at {}: {e}", link_path.display()))?;

    maybe_patch_zshrc(&home);

    Ok(format!(
        "~/.local/bin/{name} → {}",
        current_binary.display()
    ))
}

fn maybe_patch_zshrc(home: &Path) {
    let local_bin = home.join(".local").join("bin");
    let local_bin_str = local_bin.to_string_lossy();
    // If the current process PATH already includes ~/.local/bin, no patch needed.
    let path_env = std::env::var("PATH").unwrap_or_default();
    if path_env.split(':').any(|d| d == local_bin_str.as_ref()) {
        return;
    }
    let zshrc = home.join(".zshrc");
    // Don't double-patch if already present.
    if let Ok(existing) = std::fs::read_to_string(&zshrc) {
        if existing.contains(".local/bin") {
            return;
        }
    }
    let line = "\n# Added by Plexi\nexport PATH=\"$HOME/.local/bin:$PATH\"\n";
    if let Err(e) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&zshrc)
        .and_then(|mut f| {
            use std::io::Write;
            f.write_all(line.as_bytes())
        })
    {
        log::warn!("cli_setup: could not patch ~/.zshrc: {e}");
    }
}
