use std::{
    path::Path,
    sync::mpsc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const CHECK_INTERVAL: Duration = Duration::from_secs(86_400);
const API_URL: &str =
    "https://api.github.com/repos/ianjamesburke/PLEXI/releases/latest";

pub fn spawn_update_check(cache_dir: std::path::PathBuf, tx: mpsc::Sender<String>) {
    std::thread::Builder::new()
        .name("update-check".into())
        .spawn(move || {
            let cache_path = cache_dir.join("update_cache.json");
            match cached_or_fetch(&cache_path) {
                Some(latest) => {
                    let current = env!("CARGO_PKG_VERSION");
                    if semver_gt(&latest, current) {
                        log::info!(
                            "update check: newer release available: v{latest} (current v{current})"
                        );
                        let _ = tx.send(latest);
                    } else {
                        log::info!("update check: already on latest or ahead (v{current})");
                    }
                }
                None => log::warn!("update check: could not determine latest version"),
            }
        })
        .ok();
}

/// Returns true only when `a` is strictly newer than `b` by semver (M.m.p).
/// Pre-release suffixes (e.g. `-alpha`, `-rc1`) are stripped before comparison.
fn semver_gt(a: &str, b: &str) -> bool {
    fn parse_component(s: &str) -> u64 {
        let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
        s[..end].parse().unwrap_or(0)
    }
    let parse = |s: &str| -> (u64, u64, u64) {
        let mut parts = s.splitn(4, '.');
        let major = parts.next().map(parse_component).unwrap_or(0);
        let minor = parts.next().map(parse_component).unwrap_or(0);
        let patch = parts.next().map(parse_component).unwrap_or(0);
        (major, minor, patch)
    };
    parse(a) > parse(b)
}

#[cfg(test)]
mod tests {
    use super::semver_gt;

    #[test]
    fn test_semver_gt() {
        assert!(semver_gt("3.4.114", "3.4.113"));
        assert!(semver_gt("3.5.0", "3.4.113"));
        assert!(semver_gt("4.0.0", "3.9.9"));
        assert!(!semver_gt("3.4.113", "3.4.113")); // equal
        assert!(!semver_gt("3.4.111", "3.4.113")); // behind
        assert!(!semver_gt("3.4.112", "3.4.113")); // one behind
        // pre-release suffix stripping (#876)
        assert!(!semver_gt("3.4.84", "3.4.84-alpha")); // equal after strip
        assert!(semver_gt("3.4.85", "3.4.84-alpha"));  // newer than pre-release
        assert!(!semver_gt("3.4.84-alpha", "3.4.84")); // pre-release not newer than release
        assert!(!semver_gt("3.5.0-rc1", "3.5.0"));     // rc not newer than final
    }
}

fn cached_or_fetch(cache_path: &Path) -> Option<String> {
    if let Ok(bytes) = std::fs::read(cache_path) {
        if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&bytes) {
            let checked_at = json["checked_at"].as_u64().unwrap_or(0);
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            if Duration::from_secs(now.saturating_sub(checked_at)) < CHECK_INTERVAL {
                return json["latest"].as_str().map(|s| s.to_string());
            }
        }
    }
    fetch_and_cache(cache_path)
}

fn fetch_and_cache(cache_path: &Path) -> Option<String> {
    let response = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(5))
        .build()
        .get(API_URL)
        .set("User-Agent", "plexi-updater")
        .call()
        .map_err(|e| log::warn!("update check: request failed: {e}"))
        .ok()?;
    let body = response
        .into_string()
        .map_err(|e| log::warn!("update check: response read failed: {e}"))
        .ok()?;
    let json: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| log::warn!("update check: response parse failed: {e}"))
        .ok()?;
    let tag = json["tag_name"].as_str()?;
    let version = tag.trim_start_matches('v').to_string();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let cache = serde_json::json!({"checked_at": now, "latest": version});
    if let Err(e) = std::fs::write(cache_path, cache.to_string()) {
        log::warn!("update check: failed to write cache to {}: {e}", cache_path.display());
    }
    Some(version)
}
