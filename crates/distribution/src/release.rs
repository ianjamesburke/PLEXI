use crate::{Error, IoContext, Result};
use chrono::{Local, TimeZone};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::Path,
    time::{Duration, SystemTime},
};

pub const RELEASES_URL: &str = "https://api.github.com/repos/ianjamesburke/PLEXI/releases";
pub const DOWNLOAD_URL: &str = "https://github.com/ianjamesburke/PLEXI/releases/download";

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Asset {
    pub name: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

pub fn validate_channel(channel: &str) -> Result<()> {
    if channel.is_empty()
        || channel.len() > 64
        || !channel
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        || channel.starts_with('-')
    {
        return Err(Error::Invalid(format!(
            "invalid installation channel: {channel:?}"
        )));
    }
    Ok(())
}

pub fn normalized_channel(channel: &str) -> &str {
    if channel == "main" { "stable" } else { channel }
}
pub fn command_name(channel: &str) -> String {
    let channel = normalized_channel(channel);
    if channel == "stable" {
        "plexi".into()
    } else {
        format!("plexi-{channel}")
    }
}
pub fn platform() -> Result<String> {
    let os = match std::env::consts::OS {
        "macos" => "macos",
        "linux" => "linux",
        "windows" => "windows",
        os => return Err(Error::Invalid(format!("unsupported OS: {os}"))),
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "x64",
        arch => return Err(Error::Invalid(format!("unsupported architecture: {arch}"))),
    };
    Ok(format!("{os}-{arch}"))
}
pub fn archive_name(platform: &str, channel: &str) -> String {
    let suffix = if normalized_channel(channel) == "stable" {
        String::new()
    } else {
        format!("-{}", normalized_channel(channel))
    };
    let ext = if platform.starts_with("windows-") {
        "zip"
    } else {
        "tar.gz"
    };
    format!("plexi-{platform}{suffix}.{ext}")
}
pub fn version(tag: &str) -> Option<semver::Version> {
    semver::Version::parse(tag.strip_prefix('v')?).ok()
}
pub fn accepts(channel: &str, tag: &str, prerelease: bool) -> bool {
    let Some(version) = version(tag) else {
        return false;
    };
    let lane = version
        .pre
        .as_str()
        .split_once('.')
        .map(|(lane, n)| (lane, n.parse::<u64>().is_ok()));
    match (normalized_channel(channel), lane) {
        ("stable", _) => version.pre.is_empty() && !prerelease,
        (_, None) => version.pre.is_empty(),
        ("beta", Some(("beta", true))) => true,
        ("alpha", Some(("alpha" | "beta", true))) => true,
        (c, Some(("alpha" | "beta", true))) if c.starts_with("pr-") => true,
        _ => false,
    }
}

pub fn validate_tag(channel: &str, tag: &str) -> Result<()> {
    if version(tag).is_none() || !accepts(channel, tag, tag.contains('-')) {
        return Err(Error::Invalid(format!(
            "release tag {tag:?} is not valid for channel {channel}"
        )));
    }
    Ok(())
}

/// Returns a human-oriented GitHub API rate-limit diagnostic when the response
/// headers identify the failure as rate limiting.
pub fn rate_limit_message(
    status: u16,
    remaining: Option<&str>,
    reset: Option<&str>,
    retry_after: Option<&str>,
    now: SystemTime,
) -> Option<String> {
    let limited = status == 429
        || ((status == 403)
            && (remaining.is_some_and(|value| value.trim() == "0") || retry_after.is_some()));
    if !limited {
        return None;
    }
    let reset_at = reset
        .and_then(|value| value.trim().parse::<i64>().ok())
        .and_then(|seconds| Local.timestamp_opt(seconds, 0).single())
        .or_else(|| {
            retry_after
                .and_then(|value| value.trim().parse::<u64>().ok())
                .and_then(|seconds| now.checked_add(Duration::from_secs(seconds)))
                .map(chrono::DateTime::<Local>::from)
        })
        .map(|time| time.format("%Y-%m-%d %H:%M %Z").to_string())
        .unwrap_or_else(|| "the time shown by GitHub".into());
    Some(format!(
        "GitHub API rate limit reached (unauthenticated limit is 60 requests/hour per IP). Resets at {reset_at}. Set GITHUB_TOKEN (or GH_TOKEN) and retry, or retry after the reset."
    ))
}

fn github_token() -> Option<String> {
    ["GITHUB_TOKEN", "GH_TOKEN"]
        .into_iter()
        .find_map(|name| std::env::var(name).ok().filter(|token| !token.is_empty()))
}

/// The alpha installer consumes alpha builds first. A stable package can be
/// published with alpha assets so it remains an eligible fallback, but it must
/// not outrank an alpha (or beta) prerelease merely because SemVer orders a
/// final release above its prerelease.
fn alpha_preference(tag: &str) -> u8 {
    let Some(version) = version(tag) else {
        return 0;
    };
    match version.pre.as_str().split_once('.') {
        Some(("alpha", number)) if number.parse::<u64>().is_ok() => 2,
        Some(("beta", number)) if number.parse::<u64>().is_ok() => 1,
        _ => 0,
    }
}

fn same_release_series(left: &semver::Version, right: &semver::Version) -> bool {
    (left.major, left.minor, left.patch) == (right.major, right.minor, right.patch)
}

fn is_same_series_alpha_demotion(
    channel: &str,
    current: &semver::Version,
    next: &semver::Version,
) -> bool {
    normalized_channel(channel) == "alpha"
        && !current.pre.is_empty()
        && next.pre.is_empty()
        && same_release_series(current, next)
}

pub fn select<'a>(
    releases: &'a [Release],
    channel: &str,
    platform: &str,
    current: Option<&str>,
) -> Option<&'a Release> {
    let current = current.and_then(version);
    let asset = archive_name(platform, channel);
    let checksum = format!("{asset}.sha256");
    releases
        .iter()
        .filter(|r| !r.draft && accepts(channel, &r.tag_name, r.prerelease))
        .filter(|r| {
            r.assets.iter().any(|a| a.name == asset) && r.assets.iter().any(|a| a.name == checksum)
        })
        .filter(|r| {
            current.as_ref().is_none_or(|v| {
                version(&r.tag_name).is_some_and(|next| {
                    &next > v && !is_same_series_alpha_demotion(channel, v, &next)
                })
            })
        })
        .max_by_key(|r| {
            let preference = if normalized_channel(channel) == "alpha" {
                alpha_preference(&r.tag_name)
            } else {
                0
            };
            (preference, version(&r.tag_name))
        })
}

pub fn fetch(agent: &ureq::Agent, endpoint: &str) -> Result<Vec<Release>> {
    let mut releases = Vec::new();
    for page in 1..=1000 {
        let url = format!("{endpoint}?per_page=100&page={page}");
        let request = agent
            .get(&url)
            .set("User-Agent", "plexi-installer")
            .set("Accept", "application/vnd.github+json")
            .timeout(Duration::from_secs(30));
        let request = if let Some(token) = github_token() {
            request.set("Authorization", &format!("Bearer {token}"))
        } else {
            request
        };
        let response = match request.call() {
            Ok(response) => response,
            Err(ureq::Error::Status(status, response)) => {
                if let Some(message) = rate_limit_message(
                    status,
                    response.header("x-ratelimit-remaining"),
                    response.header("x-ratelimit-reset"),
                    response.header("retry-after"),
                    SystemTime::now(),
                ) {
                    return Err(Error::Invalid(message));
                }
                return Err(Error::Invalid(format!(
                    "fetch releases page {page}: status code {status}"
                )));
            }
            Err(error) => {
                return Err(Error::Invalid(format!(
                    "fetch releases page {page}: {error}"
                )));
            }
        };
        let batch: Vec<Release> = response.into_json().context("decode releases page")?;
        let done = batch.len() < 100;
        releases.extend(batch);
        if done {
            return Ok(releases);
        }
    }
    Err(Error::Invalid(
        "release pagination exceeded its bound".into(),
    ))
}

pub fn download(agent: &ureq::Agent, url: &str, destination: &Path) -> Result<()> {
    let response = agent
        .get(url)
        .set("User-Agent", "plexi-installer")
        .timeout(Duration::from_secs(300))
        .call()
        .map_err(|e| Error::Invalid(format!("download {url}: {e}")))?;
    let mut file =
        fs::File::create(destination).context(format!("create {}", destination.display()))?;
    std::io::copy(&mut response.into_reader(), &mut file).context(format!("download {url}"))?;
    file.sync_all().context("sync download")
}

fn package_urls(base: &str, tag: &str, name: &str) -> (String, String) {
    (
        format!("{base}/{tag}/{name}.sha256"),
        format!("{base}/{tag}/{name}"),
    )
}

pub fn download_package(
    agent: &ureq::Agent,
    base: &str,
    tag: &str,
    platform: &str,
    channel: &str,
    destination: &Path,
) -> Result<()> {
    validate_channel(channel)?;
    validate_tag(channel, tag)?;
    let name = archive_name(platform, channel);
    let archive = destination.join(&name);
    let sidecar = destination.join(format!("{name}.sha256"));
    let (checksum_url, archive_url) = package_urls(base, tag, &name);
    download(agent, &checksum_url, &sidecar)?;
    download(agent, &archive_url, &archive)?;
    verify_checksum(&archive, &sidecar, &name)?;
    let root = destination.join("package");
    fs::create_dir(&root).context("create unpack directory")?;
    let file = fs::File::open(&archive).context("open archive")?;
    if name.ends_with(".zip") {
        let mut zip =
            zip::ZipArchive::new(file).map_err(|e| Error::Invalid(format!("read zip: {e}")))?;
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).map_err(|e| Error::Invalid(e.to_string()))?;
            let path = root.join(crate::package::relative(
                entry.name().trim_end_matches('/'),
            )?);
            if entry.is_symlink() {
                return Err(Error::Invalid("archive symlink refused".into()));
            }
            if entry.is_dir() {
                fs::create_dir_all(&path).context("unpack directory")?;
            } else {
                fs::create_dir_all(
                    path.parent()
                        .ok_or_else(|| Error::Invalid("invalid zip path".into()))?,
                )
                .context("create archive parent")?;
                let mut out = fs::File::create(path).context("unpack file")?;
                std::io::copy(&mut entry, &mut out).context("unpack zip")?;
            }
        }
    } else {
        let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
        for entry in tar.entries().context("read tar")? {
            let mut entry = entry.context("read tar entry")?;
            if !entry.header().entry_type().is_file() && !entry.header().entry_type().is_dir() {
                return Err(Error::Invalid(
                    "archive link or special file refused".into(),
                ));
            }
            let path = entry.path().context("read archive path")?;
            let name = path
                .to_str()
                .ok_or_else(|| Error::Invalid("archive path is not UTF8".into()))?;
            crate::package::relative(name.trim_end_matches('/'))?;
            if !entry.unpack_in(&root).context("unpack tar")? {
                return Err(Error::Invalid("archive path escaped package".into()));
            }
        }
    }
    Ok(())
}

pub fn verify_checksum(archive: &Path, sidecar: &Path, name: &str) -> Result<()> {
    let checksum = fs::read_to_string(sidecar).context("read checksum")?;
    let parts: Vec<_> = checksum.split_whitespace().collect();
    if parts.len() != 2
        || parts[1].trim_start_matches('*') != name
        || parts[0].len() != 64
        || crate::package::hash_file(archive)? != parts[0].to_ascii_lowercase()
    {
        return Err(Error::Invalid(format!(
            "SHA-256 mismatch or malformed checksum for {name}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, complete: bool) -> Release {
        let asset = archive_name("macos-arm64", "alpha");
        Release {
            tag_name: tag.into(),
            draft: false,
            prerelease: tag.contains('-'),
            assets: if complete {
                vec![
                    Asset {
                        name: asset.clone(),
                    },
                    Asset {
                        name: format!("{asset}.sha256"),
                    },
                ]
            } else {
                vec![]
            },
        }
    }

    #[test]
    fn selection_orders_semantically_and_requires_archive_and_checksum() {
        let releases = vec![
            release("v1.0.0-alpha.2", true),
            release("v1.0.0-alpha.10", true),
            release("v2.0.0", false),
            release("v0.9.0", true),
        ];
        assert_eq!(
            select(&releases, "alpha", "macos-arm64", None)
                .unwrap()
                .tag_name,
            "v1.0.0-alpha.10"
        );
    }

    #[test]
    fn channel_acceptance_preserves_explicit_stable_alpha_installs() {
        assert!(accepts("alpha", "v1.0.0", false));
        assert!(accepts("beta", "v1.0.0", false));
        assert!(!accepts("stable", "v1.0.0", true));
        assert!(!accepts("beta", "v1.0.0-alpha.1", true));
        assert!(!accepts("alpha", "v1.0.0-windows.1", true));
    }

    #[test]
    fn alpha_selection_prefers_newest_alpha_prerelease_over_stable() {
        let releases = vec![
            release("v0.3.5", true),
            release("v0.3.5-alpha.1", true),
            release("v0.3.5-alpha.2", true),
        ];
        assert_eq!(
            select(&releases, "alpha", "macos-arm64", None)
                .unwrap()
                .tag_name,
            "v0.3.5-alpha.2"
        );
    }

    #[test]
    fn alpha_update_never_demotes_same_release_series_to_stable() {
        let releases = vec![release("v0.3.5", true), release("v0.3.5-alpha.2", true)];
        assert_eq!(
            select(&releases, "alpha", "macos-arm64", Some("v0.3.5-alpha.1"),)
                .unwrap()
                .tag_name,
            "v0.3.5-alpha.2"
        );

        let stable_only = vec![release("v0.3.5", true)];
        assert!(select(&stable_only, "alpha", "macos-arm64", Some("v0.3.5-alpha.1"),).is_none());
    }

    #[test]
    fn alpha_selection_falls_back_to_stable_when_no_prerelease_exists() {
        let releases = vec![release("v0.3.5", true)];
        assert_eq!(
            select(&releases, "alpha", "macos-arm64", None)
                .unwrap()
                .tag_name,
            "v0.3.5"
        );
    }

    #[test]
    fn stable_selection_rejects_prereleases() {
        let mut prerelease = release("v0.3.5-alpha.2", true);
        let mut stable = release("v0.3.4", true);
        let asset = archive_name("macos-arm64", "stable");
        for release in [&mut prerelease, &mut stable] {
            release.assets = vec![
                Asset {
                    name: asset.clone(),
                },
                Asset {
                    name: format!("{asset}.sha256"),
                },
            ];
        }
        let releases = vec![prerelease, stable];
        assert_eq!(
            select(&releases, "stable", "macos-arm64", None)
                .unwrap()
                .tag_name,
            "v0.3.4"
        );
    }

    #[test]
    fn explicit_tag_uses_direct_package_urls_without_a_release_lookup() {
        let name = archive_name("linux-x64", "alpha");
        assert_eq!(
            package_urls("http://example.test/download", "v1.2.3-alpha.4", &name),
            (
                "http://example.test/download/v1.2.3-alpha.4/plexi-linux-x64-alpha.tar.gz.sha256"
                    .into(),
                "http://example.test/download/v1.2.3-alpha.4/plexi-linux-x64-alpha.tar.gz".into(),
            )
        );
        assert!(validate_tag("alpha", "v1.2.3-alpha.4").is_ok());
        assert!(validate_tag("stable", "v1.2.3-alpha.4").is_err());
    }

    #[test]
    fn rate_limit_message_identifies_only_rate_limited_responses() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let reset = rate_limit_message(403, Some("0"), Some("1700000060"), None, now).unwrap();
        assert!(reset.contains("GitHub API rate limit reached"));
        assert!(reset.contains("GITHUB_TOKEN"));
        let retry = rate_limit_message(429, None, None, Some("60"), now).unwrap();
        assert!(retry.contains("Resets at "));
        assert!(rate_limit_message(403, None, None, None, now).is_none());
    }

    #[test]
    fn fetch_sends_github_token_when_available() {
        use std::{
            io::{Read, Write},
            net::TcpListener,
            thread,
        };

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut bytes = [0; 4096];
            let count = stream.read(&mut bytes).unwrap();
            let request = String::from_utf8_lossy(&bytes[..count]).into_owned();
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n[]")
                .unwrap();
            request
        });
        let previous = std::env::var_os("GITHUB_TOKEN");
        unsafe { std::env::set_var("GITHUB_TOKEN", "distribution-test-token") };
        let result = fetch(&ureq::Agent::new(), &endpoint);
        if let Some(value) = previous {
            unsafe { std::env::set_var("GITHUB_TOKEN", value) };
        } else {
            unsafe { std::env::remove_var("GITHUB_TOKEN") };
        }
        assert!(result.unwrap().is_empty());
        assert!(
            server
                .join()
                .unwrap()
                .contains("Authorization: Bearer distribution-test-token")
        );
    }
}
