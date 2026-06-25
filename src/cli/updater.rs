use std::{
    path::Path,
    sync::mpsc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::cli::release_resolver::{self, ReleaseTag, UpdateChannel};

const CHECK_INTERVAL: Duration = Duration::from_secs(86_400);

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
                        let _ = tx.send(latest.trim_start_matches('v').to_string());
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
