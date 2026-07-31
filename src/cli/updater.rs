use std::{
    fs::OpenOptions,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use std::os::unix::process::CommandExt;

use crate::app::ui_mailbox::UiMailbox;
use crate::cli::release_resolver::{self, ReleaseTag, UpdateChannel};

pub(crate) const CHECK_INTERVAL: Duration = Duration::from_secs(86_400);

/// Spawns a background thread that checks for updates, and if a newer version
/// is found, builds and installs it silently. Only sends on `mailbox` when the
/// new binary is ready - the UI badge means "restart to apply", not
/// "downloading". The mailbox wakes the UI thread so the badge appears even on
/// an idle host.
pub fn spawn_update_check(cache_dir: std::path::PathBuf, mailbox: UiMailbox<String>) {
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
                                let _ = mailbox.send(latest.trim_start_matches('v').to_string());
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

pub(crate) fn update_cache_fresh(cache_dir: &Path) -> bool {
    update_cache_fresh_for_channel(
        &cache_dir.join("update_cache.json"),
        detect_channel(),
        unix_now_secs(),
    )
}

/// Read the installed release tag from `<profile>/installed_tag`, falling back
/// to `CARGO_PKG_VERSION` for source builds that don't write the tag file.
fn installed_tag_or_cargo_version(cache_dir: &Path) -> String {
    let tag_path = cache_dir.join("installed_tag");
    if let Ok(tag) = std::fs::read_to_string(&tag_path) {
        let trimmed = tag.trim().to_string();
        if ReleaseTag::parse(&trimmed).is_some() {
            log::info!(
                "update check: using installed tag from {}",
                tag_path.display()
            );
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
            if cached_json_fresh_for_channel(&json, channel, unix_now_secs()) {
                return json["latest"].as_str().map(|s| s.to_string());
            }
        }
    }
    fetch_and_cache(cache_path, channel, current_raw)
}

fn update_cache_fresh_for_channel(cache_path: &Path, channel: UpdateChannel, now: u64) -> bool {
    std::fs::read(cache_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .is_some_and(|json| {
            cached_json_fresh_for_channel(&json, channel, now)
        })
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

fn unix_now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
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

    #[test]
    #[cfg(unix)]
    fn background_child_stdio_is_captured_and_session_is_detached() {
        let dir =
            std::env::temp_dir().join(format!("plexi-updater-stdio-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create updater test dir");
        let log_path = dir.join("update.log");
        let mut command = Command::new("/bin/sh");
        command.args([
            "-c",
            "read ignored || true; printf 'child-out\\n'; printf 'child-err\\n' >&2; if test -t 0 || test -t 1 || test -t 2; then exit 9; fi; if printf tty >/dev/tty 2>/dev/null; then exit 10; fi",
        ]);

        let status = run_logged_command(&mut command, &log_path, "stdio-test").expect("run child");
        assert!(status.success(), "child inherited a terminal: {status}");
        let log = std::fs::read_to_string(&log_path).expect("read update log");
        assert!(log.contains("child-out"), "stdout missing from log: {log}");
        assert!(log.contains("child-err"), "stderr missing from log: {log}");
        let _ = std::fs::remove_dir_all(dir);
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
    std::fs::File::create(&log_path).map_err(|e| format!("create update log: {e}"))?;

    log::info!("background_build: fetching source for {tag}");
    if src_dir.join(".git").is_dir() {
        let mut command = Command::new("git");
        command.args([
            "-C",
            &src_dir.to_string_lossy(),
            "fetch",
            "origin",
            "--tags",
            "--force",
        ]);
        let status = run_logged_command(&mut command, &log_path, "git fetch")?;
        if !status.success() {
            return Err("git fetch failed".to_string());
        }
    } else {
        let mut command = Command::new("git");
        command.args(["clone", repo, &src_dir.to_string_lossy()]);
        let status = run_logged_command(&mut command, &log_path, "git clone")?;
        if !status.success() {
            return Err("git clone failed".to_string());
        }
    }

    let mut checkout = Command::new("git");
    checkout.args(["-C", &src_dir.to_string_lossy(), "checkout", "--force", tag]);
    let status = run_logged_command(&mut checkout, &log_path, "git checkout")?;
    if !status.success() {
        return Err(format!("git checkout {tag} failed"));
    }

    log::info!("background_build: running install.sh for {tag} channel={channel}; sudo/bin install skipped because PLEXI_SKIP_BIN_INSTALL=1 and the updater has no TTY");

    let install_cmd = format!(
        "PLEXI_INSTALL_TAG='{}' PLEXI_SKIP_BIN_INSTALL=1 bash '{}' '{}'",
        tag,
        src_dir.join("scripts/install.sh").display(),
        channel,
    );
    let mut install = Command::new("bash");
    install
        .args(["-l", "-c", &install_cmd])
        .current_dir(&src_dir);
    let status = run_logged_command(&mut install, &log_path, "install.sh")?;

    if !status.success() {
        return Err(format!(
            "install.sh exited {status} — see {}",
            log_path.display()
        ));
    }

    log::info!("background_build: install complete for {tag}");
    Ok(())
}

fn run_logged_command(
    command: &mut Command,
    log_path: &Path,
    stage: &str,
) -> Result<std::process::ExitStatus, String> {
    let stdout = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .map_err(|error| format!("open update log for {stage}: {error}"))?;
    let stderr = stdout
        .try_clone()
        .map_err(|error| format!("clone update log for {stage}: {error}"))?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    #[cfg(unix)]
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    log::info!(
        "background_build: starting {stage}; stdin=/dev/null stdout/stderr={} session=isolated",
        log_path.display()
    );
    command
        .status()
        .map_err(|error| format!("{stage}: {error}"))
}
