use std::{path::Path, time::Duration};

use crate::app::ui_mailbox::UiMailbox;
use crate::cli::release_resolver::{self, GithubRelease, ReleaseTag, UpdateChannel};
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
        if discards_active_generation(&receipt, env!("PLEXI_BUILD_ID")) {
            log::info!(
                "update check: discarding installed {} on channel {}: it leaves the alpha lane of the running {}; restoring the running build",
                receipt.active.tag,
                receipt.channel,
                receipt.previous.as_ref().map_or("", |p| p.tag.as_str()),
            );
            if let Err(error) = plexi_distribution::transaction::rollback(&receipt.root) {
                let reason = format!(
                    "could not discard installed {}: {error}",
                    receipt.active.tag
                );
                log::warn!("update check: {reason}");
                return UpdateEvent::Failed { reason };
            }
        } else if receipt.active.build_id != env!("PLEXI_BUILD_ID") {
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
    let cache_path = cache_dir.join("update_cache.json");
    let current_raw = installed_version();
    let check = Check {
        cache_path: &cache_path,
        channel: detect_channel(),
        current_raw: &current_raw,
        build_id: env!("PLEXI_BUILD_ID"),
        now: crate::platform::clock::now_secs(),
    };
    let fetch = || {
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(5))
            .build();
        release_resolver::fetch_releases(&agent)
    };
    check_and_install(
        &check,
        mode,
        fetch,
        |tag| background_build(tag, cache_dir),
        mailbox,
    )
}

/// One check's inputs from the running build and installation, passed in so the
/// whole decision (cache, policy, install) runs under test without a network.
struct Check<'a> {
    cache_path: &'a Path,
    channel: UpdateChannel,
    current_raw: &'a str,
    build_id: &'a str,
    now: u64,
}

/// Find the release `release::select` allows from the current version and, if
/// there is one, install and activate it. The new generation is what every
/// later launch runs; "ready" means this process is still the old build.
fn check_and_install(
    check: &Check,
    mode: CheckMode,
    fetch: impl FnOnce() -> Result<Vec<GithubRelease>, String>,
    install: impl FnOnce(&str) -> Result<(), String>,
    mailbox: &UiMailbox<UpdateEvent>,
) -> UpdateEvent {
    let Check {
        channel,
        current_raw,
        ..
    } = *check;
    let (latest, checked_at) = match latest_release(check, mode, fetch) {
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
    // `release::select` is the update policy; a channel-correcting move (a beta
    // back to its series' alpha) is an update even though SemVer ranks it lower.
    if latest == current_raw {
        log::info!("update check: already on latest ({current_raw}, channel {channel:?})");
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
    match install(&latest) {
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

/// The active generation is waiting for a restart, but the channel policy no
/// longer allows it as an update from the running build (an earlier updater
/// moved an alpha to a same-series beta or stable). The running build is the
/// retained previous generation, so a rollback returns the installation to it.
/// A rollback the user made (an older alpha active) is not touched.
fn discards_active_generation(
    receipt: &plexi_distribution::transaction::Receipt,
    running_build: &str,
) -> bool {
    receipt.active.build_id != running_build
        && receipt.previous.as_ref().is_some_and(|previous| {
            previous.build_id == running_build
                && plexi_distribution::release::leaves_alpha_lane(
                    &receipt.channel,
                    &previous.tag,
                    &receipt.active.tag,
                )
        })
}

pub(crate) fn update_cache_fresh(cache_dir: &Path) -> bool {
    update_cache_fresh_for(
        &cache_dir.join("update_cache.json"),
        detect_channel(),
        env!("PLEXI_BUILD_ID"),
        crate::platform::clock::now_secs(),
    )
}

fn installed_version() -> String {
    match crate::distribution::installed() {
        Ok(Some(receipt)) => receipt.active.tag,
        Ok(None) => crate::distribution::build_tag(),
        Err(error) => {
            log::warn!("update check: read installation identity: {error}");
            crate::distribution::build_tag()
        }
    }
}

fn detect_channel() -> UpdateChannel {
    UpdateChannel::from_binary_name(&crate::config::current_exe_basename())
}

/// Returns the best candidate tag (e.g. `v0.1.13-beta.1`) for `channel`, or
/// `None` when there is none, with the time of the check that found it. A
/// scheduled check answers from a fresh cache; a manual one always fetches.
fn latest_release(
    check: &Check,
    mode: CheckMode,
    fetch: impl FnOnce() -> Result<Vec<GithubRelease>, String>,
) -> Result<(Option<String>, u64), String> {
    if mode == CheckMode::Scheduled {
        if let Ok(bytes) = std::fs::read(check.cache_path) {
            if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                if cached_json_fresh_for(&json, check.channel, check.build_id, check.now) {
                    return Ok((
                        json["latest"].as_str().map(|s| s.to_string()),
                        json["checked_at"].as_u64().unwrap_or(0),
                    ));
                }
            }
        }
    }
    fetch_and_cache(check, fetch)
}

fn update_cache_fresh_for(
    cache_path: &Path,
    channel: UpdateChannel,
    build_id: &str,
    now: u64,
) -> bool {
    std::fs::read(cache_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .is_some_and(|json| cached_json_fresh_for(&json, channel, build_id, now))
}

/// A cached answer is only reused by the build that computed it: another build
/// (an older updater, or the build before an install) chose under its own
/// version and policy, and its answer may be a release this build refuses.
fn cached_json_fresh_for(
    json: &serde_json::Value,
    channel: UpdateChannel,
    build_id: &str,
    now: u64,
) -> bool {
    let checked_at = json["checked_at"].as_u64().unwrap_or(0);
    let cached_channel = json["channel"].as_str().unwrap_or("");
    let fresh = Duration::from_secs(now.saturating_sub(checked_at)) < CHECK_INTERVAL;
    fresh && cached_channel == channel_key(channel) && json["build_id"].as_str() == Some(build_id)
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

    fn write_cache(dir: &Path, checked_at: u64, channel: &str, build_id: &str, latest: &str) {
        std::fs::create_dir_all(dir).expect("create temp cache dir");
        let cache = serde_json::json!({
            "checked_at": checked_at,
            "channel": channel,
            "build_id": build_id,
            "latest": latest,
        });
        std::fs::write(dir.join("update_cache.json"), cache.to_string())
            .expect("write update cache");
    }

    #[test]
    fn update_check_cache_fresh_requires_matching_channel_build_and_interval() {
        let dir =
            std::env::temp_dir().join(format!("plexi-update-cache-test-{}", uuid::Uuid::new_v4()));
        let now = CHECK_INTERVAL.as_secs() * 2;
        let path = dir.join("update_cache.json");

        write_cache(
            &dir,
            now - CHECK_INTERVAL.as_secs() + 1,
            "alpha",
            "a6",
            "v1",
        );
        assert!(update_cache_fresh_for(
            &path,
            UpdateChannel::Alpha,
            "a6",
            now
        ));
        assert!(!update_cache_fresh_for(
            &path,
            UpdateChannel::Beta,
            "a6",
            now
        ));
        assert!(!update_cache_fresh_for(
            &path,
            UpdateChannel::Alpha,
            "a7",
            now
        ));

        write_cache(&dir, now - CHECK_INTERVAL.as_secs(), "alpha", "a6", "v1");
        assert!(!update_cache_fresh_for(
            &path,
            UpdateChannel::Alpha,
            "a6",
            now
        ));

        let _ = std::fs::remove_dir_all(dir);
    }

    fn published(tag: &str) -> GithubRelease {
        let platform = plexi_distribution::release::platform().expect("supported test platform");
        let asset = plexi_distribution::release::archive_name(&platform, "alpha");
        serde_json::from_value(serde_json::json!({
            "tag_name": tag,
            "prerelease": tag.contains('-'),
            "assets": [{ "name": asset }, { "name": format!("{asset}.sha256") }],
        }))
        .expect("release fixture")
    }

    /// Runs the check thread's whole decision as a scheduled launch check on
    /// the alpha channel, with a cache an older build left behind that names
    /// v0.5.0-beta.1. Returns the outcome, every event sent before it, and the
    /// tags handed to the installer.
    fn launch_check(current: &str, tags: &[&str]) -> (UpdateEvent, Vec<UpdateEvent>, Vec<String>) {
        let dir =
            std::env::temp_dir().join(format!("plexi-update-check-test-{}", uuid::Uuid::new_v4()));
        let now = 1_000_000;
        write_cache(&dir, now - 60, "alpha", "older-build", "v0.5.0-beta.1");
        let cache_path = dir.join("update_cache.json");
        let check = Check {
            cache_path: &cache_path,
            channel: UpdateChannel::Alpha,
            current_raw: current,
            build_id: "this-build",
            now,
        };
        let releases: Vec<_> = tags.iter().map(|tag| published(tag)).collect();
        let (mailbox, rx) = UiMailbox::channel(
            std::sync::Arc::new(crate::app::ui_mailbox::RecordingWake::new()),
            "update_check",
        );
        let mut installed = Vec::new();
        let outcome = check_and_install(
            &check,
            CheckMode::Scheduled,
            || Ok(releases),
            |tag| {
                installed.push(tag.to_string());
                Ok(())
            },
            &mailbox,
        );
        let events = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        let _ = std::fs::remove_dir_all(dir);
        (outcome, events, installed)
    }

    #[test]
    fn update_check_launch_on_alpha_ignores_a_same_series_beta() {
        let (outcome, events, installed) =
            launch_check("v0.5.0-alpha.6", &["v0.5.0-alpha.6", "v0.5.0-beta.1"]);
        assert_eq!(
            outcome,
            UpdateEvent::UpToDate {
                checked_at: 1_000_000
            }
        );
        assert!(events.is_empty(), "no Downloading event: {events:?}");
        assert!(installed.is_empty());
    }

    #[test]
    fn update_check_launch_on_alpha_installs_the_next_alpha() {
        let (outcome, events, installed) = launch_check(
            "v0.5.0-alpha.6",
            &["v0.5.0-alpha.6", "v0.5.0-alpha.7", "v0.5.0-beta.1"],
        );
        assert_eq!(
            events,
            vec![UpdateEvent::Downloading {
                version: "0.5.0-alpha.7".into()
            }]
        );
        assert_eq!(installed, vec!["v0.5.0-alpha.7".to_string()]);
        assert_eq!(
            outcome,
            UpdateEvent::Ready {
                version: "0.5.0-alpha.7".into()
            }
        );
    }

    #[test]
    fn update_check_launch_on_a_stuck_beta_returns_to_the_newest_alpha() {
        let (outcome, _, installed) = launch_check(
            "v0.5.0-beta.1",
            &["v0.5.0-alpha.6", "v0.5.0-alpha.7", "v0.5.0-beta.1"],
        );
        assert_eq!(installed, vec!["v0.5.0-alpha.7".to_string()]);
        assert!(matches!(outcome, UpdateEvent::Ready { .. }));
    }

    fn receipt(
        channel: &str,
        active: (&str, &str),
        previous: (&str, &str),
    ) -> plexi_distribution::transaction::Receipt {
        let generation = |(build, tag): (&str, &str)| plexi_distribution::transaction::Generation {
            path: Path::new("/generations").join(build),
            build_id: build.into(),
            tag: tag.into(),
            executable: "plexi".into(),
        };
        plexi_distribution::transaction::Receipt {
            schema: 1,
            channel: channel.into(),
            root: "/root".into(),
            bin_dir: "/bin".into(),
            applications_dir: "/Applications".into(),
            active: generation(active),
            previous: Some(generation(previous)),
            integrations: vec![],
            path_edits: vec![],
            windows_path_added: false,
        }
    }

    #[test]
    fn update_check_discards_an_installed_same_series_beta_for_a_running_alpha() {
        let staged = receipt("alpha", ("b1", "v0.5.0-beta.1"), ("a6", "v0.5.0-alpha.6"));
        assert!(discards_active_generation(&staged, "a6"));
        // After a restart into it there is nothing waiting to discard.
        assert!(!discards_active_generation(&staged, "b1"));
        let stable = receipt("alpha", ("s", "v0.5.0"), ("a6", "v0.5.0-alpha.6"));
        assert!(discards_active_generation(&stable, "a6"));
    }

    #[test]
    fn update_check_keeps_allowed_and_user_chosen_active_builds() {
        let newer_alpha = receipt("alpha", ("a7", "v0.5.0-alpha.7"), ("a6", "v0.5.0-alpha.6"));
        assert!(!discards_active_generation(&newer_alpha, "a6"));
        let rolled_back = receipt("alpha", ("a6", "v0.5.0-alpha.6"), ("a7", "v0.5.0-alpha.7"));
        assert!(!discards_active_generation(&rolled_back, "a7"));
        let next_series = receipt("alpha", ("b", "v0.6.0-beta.1"), ("a6", "v0.5.0-alpha.6"));
        assert!(!discards_active_generation(&next_series, "a6"));
        let beta_promotion = receipt("beta", ("s", "v0.5.0"), ("b1", "v0.5.0-beta.1"));
        assert!(!discards_active_generation(&beta_promotion, "b1"));
    }
}

fn fetch_and_cache(
    check: &Check,
    fetch: impl FnOnce() -> Result<Vec<GithubRelease>, String>,
) -> Result<(Option<String>, u64), String> {
    let releases = fetch().map_err(|e| format!("could not fetch releases: {e}"))?;
    let current = ReleaseTag::parse(check.current_raw)
        .ok_or_else(|| format!("could not read the running version {}", check.current_raw))?;
    let best = release_resolver::resolve_best(&releases, check.channel, &current);

    let latest_raw = best.as_ref().map(|t| t.raw.clone());
    let cache = serde_json::json!({
        "checked_at": check.now,
        "channel": channel_key(check.channel),
        "build_id": check.build_id,
        "latest": latest_raw,
    });
    if let Err(e) = std::fs::write(check.cache_path, cache.to_string()) {
        log::warn!(
            "update check: failed to write cache to {}: {e}",
            check.cache_path.display()
        );
    }
    Ok((latest_raw, check.now))
}

/// Foreground and background updates run the same native transaction.
fn background_build(tag: &str, profile_dir: &Path) -> Result<(), String> {
    let channel = crate::config::build_channel().unwrap_or_else(|| "stable".into());
    let result = super::install::run_binary_asset_install(&channel, tag);
    let text = match &result {
        Ok(()) => format!("Installed and verified {tag}\n"),
        Err(error) => format!("Update {tag} failed: {error}\n"),
    };
    std::fs::write(profile_dir.join("update.log"), text)
        .map_err(|e| format!("write update log: {e}"))?;
    result
}
