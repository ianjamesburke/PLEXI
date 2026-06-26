use std::{
    path::Path,
    sync::mpsc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::cli::release_resolver::{self, ReleaseTag, UpdateChannel};

const CHECK_INTERVAL: Duration = Duration::from_secs(86_400);

/// Spawns a background thread that checks for updates, and if a newer version
/// is found, builds and installs it silently. Only sends on `tx` when the new
/// binary is ready - the UI badge means "restart to apply", not "downloading".
pub fn spawn_update_check(cache_dir: std::path::PathBuf, tx: mpsc::Sender<String>) {
    std::thread::Builder::new()
        .name("update-check".into())
        .spawn(move || {
            let channel = detect_channel();
            let cache_path = cache_dir.join("update_cache.json");
            let current_raw = installed_tag_or_cargo_version(&cache_dir);
            match cached_or_fetch(&cache_path, channel, &current_raw) {
                Some(latest) => {
                    let current = ReleaseTag::parse(&current_raw);
                    let latest_tag = ReleaseTag::parse(&latest);
                    let newer = match (&latest_tag, &current) {
                        (Some(l), Some(c)) => l > c,
                        _ => false,
                    };
                    if newer {
                        log::info!(
                            "update check: newer release available: {latest} (current {current_raw}, channel {channel:?})"
                        );
                        match background_build(&latest, &cache_dir) {
                            Ok(()) => {
                                log::info!("update check: background build complete for {latest}");
                                let _ = tx.send(latest.trim_start_matches('v').to_string());
                            }
                            Err(e) => {
                                log::warn!("update check: background build failed: {e}");
                            }
                        }
                    } else {
                        log::info!(
                            "update check: already on latest or ahead ({current_raw}, channel {channel:?})"
                        );
                    }
                }
                None => log::info!("update check: no newer release for channel {channel:?}"),
            }
        })
        .ok();
}

/// Read the installed release tag from `<profile>/installed_tag`, falling back
/// to `CARGO_PKG_VERSION` for source builds that don't write the tag file.
fn installed_tag_or_cargo_version(cache_dir: &Path) -> String {
    let tag_path = cache_dir.join("installed_tag");
    if let Ok(tag) = std::fs::read_to_string(&tag_path) {
        let trimmed = tag.trim().to_string();
        if ReleaseTag::parse(&trimmed).is_some() {
            log::info!("update check: using installed tag from {}", tag_path.display());
            return trimmed;
        }
    }
    format!("v{}", env!("CARGO_PKG_VERSION"))
}

fn detect_channel() -> UpdateChannel {
    let name = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "plexi".to_string());
    UpdateChannel::from_binary_name(&name)
}

/// Returns the best candidate tag (e.g. `v0.1.13-beta.1`) for `channel`, or
/// `None` when the cache is fresh and held no candidate or the fetch found none.
fn cached_or_fetch(cache_path: &Path, channel: UpdateChannel, current_raw: &str) -> Option<String> {
    if let Ok(bytes) = std::fs::read(cache_path) {
        if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&bytes) {
            let checked_at = json["checked_at"].as_u64().unwrap_or(0);
            let cached_channel = json["channel"].as_str().unwrap_or("");
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let fresh = Duration::from_secs(now.saturating_sub(checked_at)) < CHECK_INTERVAL;
            if fresh && cached_channel == channel_key(channel) {
                return json["latest"].as_str().map(|s| s.to_string());
            }
        }
    }
    fetch_and_cache(cache_path, channel, current_raw)
}

fn channel_key(channel: UpdateChannel) -> &'static str {
    match channel {
        UpdateChannel::Stable => "stable",
        UpdateChannel::Beta => "beta",
        UpdateChannel::Alpha => "alpha",
    }
}

fn fetch_and_cache(cache_path: &Path, channel: UpdateChannel, current_raw: &str) -> Option<String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(5))
        .build();
    let releases = release_resolver::fetch_releases(&agent)
        .map_err(|e| log::warn!("update check: {e}"))
        .ok()?;

    let current = ReleaseTag::parse(current_raw)?;
    let best = release_resolver::resolve_best(&releases, channel, &current);

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let latest_raw = best.as_ref().map(|t| t.raw.clone());
    let cache = serde_json::json!({
        "checked_at": now,
        "channel": channel_key(channel),
        "latest": latest_raw,
    });
    if let Err(e) = std::fs::write(cache_path, cache.to_string()) {
        log::warn!(
            "update check: failed to write cache to {}: {e}",
            cache_path.display()
        );
    }
    latest_raw
}

/// Clone/fetch the source repo, check out the target tag, and run the install
/// script. This builds the binary and copies it into /Applications/ while the
/// current instance keeps running. The running binary is not affected because
/// macOS loads it into memory at launch.
fn background_build(tag: &str, profile_dir: &Path) -> Result<(), String> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    let src_dir = std::path::PathBuf::from(&home).join(".plexi-src");
    let repo = "https://github.com/ianjamesburke/PLEXI.git";

    let binary_name = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "plexi".to_string());
    let channel = if binary_name == "plexi" {
        "main".to_string()
    } else {
        binary_name
            .strip_prefix("plexi-")
            .unwrap_or("main")
            .to_string()
    };

    let log_path = profile_dir.join("update.log");

    log::info!("background_build: fetching source for {tag}");
    if src_dir.join(".git").is_dir() {
        let status = std::process::Command::new("git")
            .args(["-C", &src_dir.to_string_lossy(), "fetch", "origin", "--tags", "--force"])
            .status()
            .map_err(|e| format!("git fetch: {e}"))?;
        if !status.success() {
            return Err("git fetch failed".to_string());
        }
    } else {
        let status = std::process::Command::new("git")
            .args(["clone", repo, &src_dir.to_string_lossy()])
            .status()
            .map_err(|e| format!("git clone: {e}"))?;
        if !status.success() {
            return Err("git clone failed".to_string());
        }
    }

    let status = std::process::Command::new("git")
        .args(["-C", &src_dir.to_string_lossy(), "checkout", "--force", tag])
        .status()
        .map_err(|e| format!("git checkout: {e}"))?;
    if !status.success() {
        return Err(format!("git checkout {tag} failed"));
    }

    log::info!("background_build: running install.sh for {tag} channel={channel}");
    let log_file = std::fs::File::create(&log_path)
        .map_err(|e| format!("create update log: {e}"))?;
    let log_err = log_file.try_clone()
        .map_err(|e| format!("clone log handle: {e}"))?;

    let install_cmd = format!(
        "PLEXI_INSTALL_TAG='{}' bash '{}' '{}'",
        tag,
        src_dir.join("scripts/install.sh").display(),
        channel,
    );
    let status = std::process::Command::new("bash")
        .args(["-l", "-c", &install_cmd])
        .current_dir(&src_dir)
        .stdout(log_file)
        .stderr(log_err)
        .status()
        .map_err(|e| format!("install.sh: {e}"))?;

    if !status.success() {
        return Err(format!("install.sh exited {status} — see {}", log_path.display()));
    }

    log::info!("background_build: install complete for {tag}");
    Ok(())
}
