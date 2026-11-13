use std::{path::Path, time::Duration};

use crate::app::ui_mailbox::UiMailbox;
use crate::cli::release_resolver::{self, ReleaseTag, UpdateChannel};
use crate::host::update_status::{CheckMode, UpdateEvent};

pub(crate) const CHECK_INTERVAL: Duration = Duration::from_secs(86_400);

/// Spawns a background thread that checks for updates and, if a newer version
/// is found, downloads and installs it silently. Reports each phase on
/// `mailbox` and always ends with exactly one outcome event (`Ready`,
/// `UpToDate`, `Failed` or `Unavailable`); "ready" means "restart to apply".
/// A manual check skips the update cache. The mailbox wakes the UI thread so
/// the status reaches an idle host.
pub(crate) fn spawn_update_check(
    cache_dir: std::path::PathBuf,
    mailbox: UiMailbox<UpdateEvent>,
    mode: CheckMode,
) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("update-check".into())
        .spawn(move || {
            log::info!("update check: {} check started", mode.label());
            let outcome = run_check(&cache_dir, mode, &mailbox);
            log::info!("update check: {} check finished: {outcome:?}", mode.label());
            send_event(&mailbox, outcome);
        })
        .map(|_| ())
}

fn send_event(mailbox: &UiMailbox<UpdateEvent>, event: UpdateEvent) {
    if let Err(error) = mailbox.send(event) {
        log::warn!(
            "update check: host stopped listening before {:?} arrived",
            error.0
        );
    }
}

fn run_check(cache_dir: &Path, mode: CheckMode, mailbox: &UiMailbox<UpdateEvent>) -> UpdateEvent {
    if let Ok(Some(receipt)) = crate::distribution::installed() {
        if receipt.active.build_id != env!("PLEXI_BUILD_ID") {
            log::info!("update check: running and installed builds differ; restart available");
            return UpdateEvent::Ready {
                version: receipt.active.tag.trim_start_matches('v').to_string(),
            };
        }
    }
    if let Some(build_channel) =
        crate::config::build_channel().filter(|c| c != "alpha" && c != "beta")
    {
        return UpdateEvent::Unavailable {
            reason: format!("{build_channel} builds do not check for updates"),
        };
    }
    let channel = detect_channel();
    let cache_path = cache_dir.join("update_cache.json");
    let current_raw = installed_version();
    let (latest, checked_at) = match latest_release(&cache_path, channel, &current_raw, mode) {
        Ok(found) => found,
        Err(reason) => {
            log::warn!("update check: {reason}");
            return UpdateEvent::Failed { reason };
        }
    };
    let Some(latest) = latest else {
        log::info!("update check: no newer release for channel {channel:?}");
        return UpdateEvent::UpToDate { checked_at };
    };
    let newer = match (ReleaseTag::parse(&latest), ReleaseTag::parse(&current_raw)) {
        (Some(l), Some(c)) => l > c,
        _ => false,
    };
    if !newer {
        log::info!("update check: already on latest or ahead ({current_raw}, channel {channel:?})");
        return UpdateEvent::UpToDate { checked_at };
    }
    log::info!(
        "update check: newer release available: {latest} (current {current_raw}, channel {channel:?})"
    );
    let version = latest.trim_start_matches('v').to_string();
    send_event(
        mailbox,
        UpdateEvent::Downloading {
            version: version.clone(),
        },
    );
    match background_build(&latest, cache_dir) {
        Ok(()) => {
            log::info!("update check: background build complete for {latest}");
            UpdateEvent::Ready { version }
        }
        Err(reason) => {
            log::warn!("update check: background build failed: {reason}");
            UpdateEvent::Failed { reason }
        }
    }
}

pub(crate) fn update_cache_fresh(cache_dir: &Path) -> bool {
    update_cache_fresh_for_channel(
        &cache_dir.join("update_cache.json"),
        detect_channel(),
        crate::platform::clock::now_secs(),
    )
}

fn installed_version() -> String {
    match crate::distribution::installed() {
        Ok(Some(receipt)) => receipt.active.tag,
        Ok(None) => crate::distribution::build_tag(),
        Err(error) => { log::warn!("update check: read installation identity: {error}"); crate::distribution::build_tag() }
    }
}

fn detect_channel() -> UpdateChannel {
    UpdateChannel::from_binary_name(&crate::config::current_exe_basename())
}

/// Returns the best candidate tag (e.g. `v0.1.13-beta.1`) for `channel`, or
/// `None` when there is none, with the time of the check that found it. A
/// scheduled check answers from a fresh cache; a manual one always fetches.
fn latest_release(
    cache_path: &Path,
    channel: UpdateChannel,
    current_raw: &str,
    mode: CheckMode,
) -> Result<(Option<String>, u64), String> {
    if mode == CheckMode::Scheduled {
        if let Ok(bytes) = std::fs::read(cache_path) {
            if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                if cached_json_fresh_for_channel(&json, channel, crate::platform::clock::now_secs())
                {
                    return Ok((
                        json["latest"].as_str().map(|s| s.to_string()),
                        json["checked_at"].as_u64().unwrap_or(0),
                    ));
                }
            }
        }
    }
    fetch_and_cache(cache_path, channel, current_raw)
}

fn update_cache_fresh_for_channel(cache_path: &Path, channel: UpdateChannel, now: u64) -> bool {
    std::fs::read(cache_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .is_some_and(|json| cached_json_fresh_for_channel(&json, channel, now))
}

fn cached_json_fresh_for_channel(
    json: &serde_json::Value,
    channel: UpdateChannel,
    now: u64,
) -> bool {
    let checked_at = json["checked_at"].as_u64().unwrap_or(0);
    let cached_channel = json["channel"].as_str().unwrap_or("");
    let fresh = Duration::from_secs(now.saturating_sub(checked_at)) < CHECK_INTERVAL;
    fresh && cached_channel == channel_key(channel)
}

fn channel_key(channel: UpdateChannel) -> &'static str {
    match channel {
        UpdateChannel::Stable => "stable",
        UpdateChannel::Beta => "beta",
        UpdateChannel::Alpha => "alpha",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_cache(dir: &Path, checked_at: u64, channel: &str) {
        std::fs::create_dir_all(dir).expect("create temp cache dir");
        let cache = serde_json::json!({
            "checked_at": checked_at,
            "channel": channel,
            "latest": serde_json::Value::Null,
        });
        std::fs::write(dir.join("update_cache.json"), cache.to_string())
            .expect("write update cache");
    }

    #[test]
    fn update_check_cache_fresh_requires_matching_channel_and_interval() {
        let dir =
            std::env::temp_dir().join(format!("plexi-update-cache-test-{}", uuid::Uuid::new_v4()));
        let now = CHECK_INTERVAL.as_secs() * 2;

        write_cache(&dir, now - CHECK_INTERVAL.as_secs() + 1, "alpha");
        assert!(update_cache_fresh_for_channel(
            &dir.join("update_cache.json"),
            UpdateChannel::Alpha,
            now,
        ));
        assert!(!update_cache_fresh_for_channel(
            &dir.join("update_cache.json"),
            UpdateChannel::Beta,
            now,
        ));

        write_cache(&dir, now - CHECK_INTERVAL.as_secs(), "alpha");
        assert!(!update_cache_fresh_for_channel(
            &dir.join("update_cache.json"),
            UpdateChannel::Alpha,
            now,
        ));

        let _ = std::fs::remove_dir_all(dir);
    }


}

fn fetch_and_cache(
    cache_path: &Path,
    channel: UpdateChannel,
    current_raw: &str,
) -> Result<(Option<String>, u64), String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(5))
        .build();
    let releases = release_resolver::fetch_releases(&agent)
        .map_err(|e| format!("could not fetch releases: {e}"))?;

    let current = ReleaseTag::parse(current_raw)
        .ok_or_else(|| format!("could not read the running version {current_raw}"))?;
    let best = release_resolver::resolve_best(&releases, channel, &current);

    let now = crate::platform::clock::now_secs();
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
    Ok((latest_raw, now))
}

/// Foreground and background updates run the same native transaction.
fn background_build(tag: &str, profile_dir: &Path) -> Result<(), String> {
    let channel = crate::config::build_channel().unwrap_or_else(|| "stable".into());
    let result = super::install::run_binary_asset_install(&channel, tag);
    let text = match &result { Ok(()) => format!("Installed and verified {tag}\n"), Err(error) => format!("Update {tag} failed: {error}\n") };
    std::fs::write(profile_dir.join("update.log"), text).map_err(|e| format!("write update log: {e}"))?;
    result
}
