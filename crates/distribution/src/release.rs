use crate::{Error, IoContext, Result};
use chrono::{Local, TimeZone};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::Path,
    time::{Duration, SystemTime},
};

/// The repository's REST API root. Resolution reads `releases/latest`,
/// `git/matching-refs/tags/v` and `releases/tags/<tag>` under it, never the
/// paginated release list (it returned 504 for hours on 2026-11-23 while
/// these single-object endpoints answered).
pub const REPO_API_URL: &str = "https://api.github.com/repos/ianjamesburke/PLEXI";
/// How many ranked tags a channel resolution looks up before it gives up on
/// finding a complete published release among them.
pub const MAX_TAG_LOOKUPS: usize = 5;
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

fn is_alpha_build(version: &semver::Version) -> bool {
    matches!(
        version.pre.as_str().split_once('.'),
        Some(("alpha", number)) if number.parse::<u64>().is_ok()
    )
}

/// Within one release series the alpha channel only moves to alpha builds
/// once it runs a prerelease: a beta or stable of that series carries
/// older code (it is cut from an earlier alpha) and, once installed, would
/// hide every later alpha of that series because SemVer orders it above them.
pub fn leaves_alpha_lane(channel: &str, from: &str, to: &str) -> bool {
    match (version(from), version(to)) {
        (Some(from), Some(to)) => {
            normalized_channel(channel) == "alpha"
                && !from.pre.is_empty()
                && !is_alpha_build(&to)
                && same_release_series(&from, &to)
        }
        _ => false,
    }
}

/// An alpha installation that an earlier updater moved onto a beta returns to
/// its series' alpha builds. SemVer ranks those alphas below the beta, but the
/// beta was cut from one of them, so the newest alpha is never older code. A
/// same-series stable is excluded: it is cut after the series' alphas.
fn returns_to_alpha_lane(channel: &str, from: &str, to: &str) -> bool {
    match (version(from), version(to)) {
        (Some(from), Some(to)) => {
            normalized_channel(channel) == "alpha"
                && !from.pre.is_empty()
                && !is_alpha_build(&from)
                && is_alpha_build(&to)
                && same_release_series(&from, &to)
        }
        _ => false,
    }
}

/// Whether the channel would move from `current` to `tag`. No current version
/// (a fresh install) offers every tag.
fn offered(channel: &str, current: Option<&str>, tag: &str) -> bool {
    let Some((from, from_version)) = current.and_then(|from| Some((from, version(from)?))) else {
        return true;
    };
    version(tag).is_some_and(|next| {
        (next > from_version && !leaves_alpha_lane(channel, from, tag))
            || returns_to_alpha_lane(channel, from, tag)
    })
}

/// The channel's ordering: the alpha channel prefers alpha, then beta, then
/// stable builds; every other channel is plain SemVer.
fn rank(channel: &str, tag: &str) -> (u8, Option<semver::Version>) {
    let preference = if normalized_channel(channel) == "alpha" {
        alpha_preference(tag)
    } else {
        0
    };
    (preference, version(tag))
}

pub fn select<'a>(
    releases: &'a [Release],
    channel: &str,
    platform: &str,
    current: Option<&str>,
) -> Option<&'a Release> {
    let asset = archive_name(platform, channel);
    let checksum = format!("{asset}.sha256");
    releases
        .iter()
        .filter(|r| !r.draft && accepts(channel, &r.tag_name, r.prerelease))
        .filter(|r| {
            r.assets.iter().any(|a| a.name == asset) && r.assets.iter().any(|a| a.name == checksum)
        })
        .filter(|r| offered(channel, current, &r.tag_name))
        .max_by_key(|r| rank(channel, &r.tag_name))
}

/// Tags `select` could choose, best first, judged from the tag alone. Draft,
/// prerelease-flag and asset checks need the release record.
fn ranked_tags<'a>(tags: &'a [String], channel: &str, current: Option<&str>) -> Vec<&'a str> {
    let mut ranked: Vec<&str> = tags
        .iter()
        .map(String::as_str)
        .filter(|tag| accepts(channel, tag, tag.contains('-')))
        .filter(|tag| offered(channel, current, tag))
        .collect();
    ranked.sort_by_cached_key(|tag| std::cmp::Reverse(rank(channel, tag)));
    ranked
}

/// Where channel resolution reads releases from: GitHub in production, a
/// fixture in tests.
pub trait ReleaseSource {
    /// The release GitHub marks latest (newest non-prerelease, non-draft).
    fn latest(&self) -> Result<Option<Release>>;
    /// Every `v*` tag name in the repository.
    fn tags(&self) -> Result<Vec<String>>;
    /// The published release for `tag`, if there is one.
    fn release(&self, tag: &str) -> Result<Option<Release>>;
}

/// The release `select` would choose from every published release, read
/// without the release list. Stable is GitHub's latest release. Other
/// channels rank the tag list by `select`'s policy and look up the best
/// tags one at a time; the first complete release wins. Looking past
/// `MAX_TAG_LOOKUPS` incomplete candidates is an error, not an answer.
pub fn resolve(
    source: &impl ReleaseSource,
    channel: &str,
    platform: &str,
    current: Option<&str>,
) -> Result<Option<Release>> {
    let complete = |release: &Release| {
        select(std::slice::from_ref(release), channel, platform, current).is_some()
    };
    if normalized_channel(channel) == "stable" {
        let latest = source.latest()?;
        log::info!(
            "release: stable latest is {:?}",
            latest.as_ref().map(|r| &r.tag_name)
        );
        return Ok(latest.filter(complete));
    }
    let tags = source.tags()?;
    let ranked = ranked_tags(&tags, channel, current);
    log::info!(
        "release: {} of {} tags are candidates for {channel} from {current:?}",
        ranked.len(),
        tags.len()
    );
    for tag in ranked.iter().take(MAX_TAG_LOOKUPS) {
        match source.release(tag)? {
            Some(release) if complete(&release) => return Ok(Some(release)),
            Some(_) => log::info!(
                "release: {tag} has no complete {channel} package for {platform}; trying the next tag"
            ),
            None => log::info!("release: {tag} has no published release; trying the next tag"),
        }
    }
    if ranked.len() > MAX_TAG_LOOKUPS {
        return Err(Error::Invalid(format!(
            "none of the newest {MAX_TAG_LOOKUPS} {channel} releases has a complete package for {platform}"
        )));
    }
    Ok(None)
}

/// Retry budget for one GitHub API request. Server errors, timeouts and
/// connection failures are retried with doubling delays; 4xx and rate limits
/// are answered at once.
#[derive(Clone, Copy, Debug)]
pub struct Retry {
    pub attempts: u32,
    pub first_delay: Duration,
    pub timeout: Duration,
}

impl Retry {
    /// Worst case per request: 3 × 15 s plus 1 s and 2 s of backoff.
    pub const GITHUB: Self = Self {
        attempts: 3,
        first_delay: Duration::from_secs(1),
        timeout: Duration::from_secs(15),
    };
}

/// The GitHub REST API for one repository.
pub struct GithubApi<'a> {
    agent: &'a ureq::Agent,
    repo: String,
    retry: Retry,
}

#[derive(Deserialize)]
struct GitRef {
    #[serde(rename = "ref")]
    name: String,
}

enum Failure {
    /// Worth another attempt: a 5xx, timeout, reset or refused connection.
    Transient(String),
    Final(Error),
}

impl<'a> GithubApi<'a> {
    pub fn new(agent: &'a ureq::Agent, repo: impl Into<String>, retry: Retry) -> Self {
        Self {
            agent,
            repo: repo.into(),
            retry,
        }
    }

    /// GET `path` under the repository. `Ok(None)` is a 404.
    fn get(&self, path: &str) -> Result<Option<String>> {
        let url = format!("{}/{path}", self.repo);
        let attempts = self.retry.attempts.max(1);
        let mut delay = self.retry.first_delay;
        let mut attempt = 1;
        loop {
            let detail = match self.attempt(&url) {
                Ok(body) => return Ok(body),
                Err(Failure::Final(error)) => return Err(error),
                Err(Failure::Transient(detail)) => detail,
            };
            if attempt == attempts {
                let detail = format!("GET {url}: {detail} after {attempts} attempts");
                log::warn!("github api: {detail}");
                return Err(Error::GithubSlow { detail });
            }
            log::info!(
                "github api: GET {url} attempt {attempt}/{attempts} failed ({detail}); retrying in {delay:?}"
            );
            std::thread::sleep(delay);
            delay = delay.saturating_mul(2);
            attempt += 1;
        }
    }

    fn attempt(&self, url: &str) -> std::result::Result<Option<String>, Failure> {
        let request = self
            .agent
            .get(url)
            .set("User-Agent", "plexi-installer")
            .set("Accept", "application/vnd.github+json")
            .timeout(self.retry.timeout);
        let request = if let Some(token) = github_token() {
            request.set("Authorization", &format!("Bearer {token}"))
        } else {
            request
        };
        match request.call() {
            Ok(response) => response
                .into_string()
                .map(Some)
                .map_err(|error| Failure::Transient(format!("read body: {error}"))),
            Err(ureq::Error::Status(404, _)) => Ok(None),
            Err(ureq::Error::Status(status, response)) => {
                if let Some(message) = rate_limit_message(
                    status,
                    response.header("x-ratelimit-remaining"),
                    response.header("x-ratelimit-reset"),
                    response.header("retry-after"),
                    SystemTime::now(),
                ) {
                    return Err(Failure::Final(Error::Invalid(message)));
                }
                if status >= 500 {
                    Err(Failure::Transient(format!("status code {status}")))
                } else {
                    Err(Failure::Final(Error::Invalid(format!(
                        "GET {url}: status code {status}"
                    ))))
                }
            }
            Err(ureq::Error::Transport(transport)) => match transport.kind() {
                ureq::ErrorKind::Io | ureq::ErrorKind::ConnectionFailed => {
                    let cause = std::error::Error::source(&transport)
                        .map(|source| format!(" ({source})"))
                        .unwrap_or_default();
                    Err(Failure::Transient(format!(
                        "{}: {}{cause}",
                        transport.kind(),
                        transport.message().unwrap_or("transport failure")
                    )))
                }
                _ => Err(Failure::Final(Error::Invalid(format!(
                    "GET {url}: {transport}"
                )))),
            },
        }
    }
}

impl ReleaseSource for GithubApi<'_> {
    fn latest(&self) -> Result<Option<Release>> {
        self.get("releases/latest")?
            .map(|body| serde_json::from_str(&body).map_err(Error::from))
            .transpose()
    }

    fn tags(&self) -> Result<Vec<String>> {
        let body = self
            .get("git/matching-refs/tags/v")?
            .ok_or_else(|| Error::Invalid(format!("{}: tag list not found", self.repo)))?;
        let refs: Vec<GitRef> = serde_json::from_str(&body)?;
        Ok(refs
            .into_iter()
            .filter_map(|r| r.name.strip_prefix("refs/tags/").map(str::to_owned))
            .collect())
    }

    fn release(&self, tag: &str) -> Result<Option<Release>> {
        self.get(&format!("releases/tags/{tag}"))?
            .map(|body| serde_json::from_str(&body).map_err(Error::from))
            .transpose()
    }
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
    fn alpha_update_never_moves_to_a_same_series_beta() {
        let mut releases = vec![
            release("v0.5.0-alpha.6", true),
            release("v0.5.0-beta.1", true),
        ];
        assert!(select(&releases, "alpha", "macos-arm64", Some("v0.5.0-alpha.6")).is_none());
        releases.push(release("v0.5.0-alpha.7", true));
        assert_eq!(
            select(&releases, "alpha", "macos-arm64", Some("v0.5.0-alpha.6"))
                .unwrap()
                .tag_name,
            "v0.5.0-alpha.7"
        );
    }

    #[test]
    fn alpha_install_on_a_same_series_beta_returns_to_the_newest_alpha() {
        let releases = vec![
            release("v0.5.0-alpha.6", true),
            release("v0.5.0-alpha.7", true),
            release("v0.5.0-beta.1", true),
        ];
        assert_eq!(
            select(&releases, "alpha", "macos-arm64", Some("v0.5.0-beta.1"))
                .unwrap()
                .tag_name,
            "v0.5.0-alpha.7"
        );
        // Only the alpha channel corrects; a beta install stays on beta.
        assert!(
            select(
                &[release_for("v0.5.0-alpha.7", "beta")],
                "beta",
                "macos-arm64",
                Some("v0.5.0-beta.1")
            )
            .is_none()
        );
        // A same-series stable is never moved back to that series' alphas.
        assert!(select(&releases, "alpha", "macos-arm64", Some("v0.5.0")).is_none());
    }

    #[test]
    fn alpha_update_acceptance_by_series_and_lane() {
        let from = Some("v0.5.0-alpha.6");
        let offered = |tag: &str| {
            select(&[release(tag, true)], "alpha", "macos-arm64", from).map(|r| r.tag_name.clone())
        };
        assert_eq!(offered("v0.5.0-alpha.7").as_deref(), Some("v0.5.0-alpha.7"));
        assert_eq!(offered("v0.5.0-beta.1"), None);
        assert_eq!(offered("v0.5.0"), None);
        assert_eq!(offered("v0.5.0-alpha.5"), None);
        assert_eq!(offered("v0.6.0-alpha.1").as_deref(), Some("v0.6.0-alpha.1"));
        assert_eq!(offered("v0.6.0-beta.1").as_deref(), Some("v0.6.0-beta.1"));
        assert_eq!(offered("v0.6.0").as_deref(), Some("v0.6.0"));
        assert_eq!(offered("v0.5.1").as_deref(), Some("v0.5.1"));
        // A newer alpha still outranks a higher-series beta or stable.
        let releases = vec![
            release("v0.6.0", true),
            release("v0.6.0-beta.1", true),
            release("v0.5.0-alpha.7", true),
        ];
        assert_eq!(
            select(&releases, "alpha", "macos-arm64", from)
                .unwrap()
                .tag_name,
            "v0.5.0-alpha.7"
        );
    }

    fn release_for(tag: &str, channel: &str) -> Release {
        let asset = archive_name("macos-arm64", channel);
        Release {
            tag_name: tag.into(),
            draft: false,
            prerelease: tag.contains('-'),
            assets: vec![
                Asset {
                    name: asset.clone(),
                },
                Asset {
                    name: format!("{asset}.sha256"),
                },
            ],
        }
    }

    #[test]
    fn beta_update_moves_between_betas_and_promotes_to_stable_never_to_alpha() {
        let offered = |tags: &[&str], from: &str| {
            let releases: Vec<_> = tags.iter().map(|t| release_for(t, "beta")).collect();
            select(&releases, "beta", "macos-arm64", Some(from)).map(|r| r.tag_name.clone())
        };
        assert_eq!(offered(&["v0.5.0-alpha.7"], "v0.5.0-beta.1"), None);
        assert_eq!(offered(&["v0.6.0-alpha.1"], "v0.5.0-beta.1"), None);
        assert_eq!(
            offered(&["v0.5.0-beta.2"], "v0.5.0-beta.1").as_deref(),
            Some("v0.5.0-beta.2")
        );
        assert_eq!(
            offered(&["v0.5.0"], "v0.5.0-beta.1").as_deref(),
            Some("v0.5.0")
        );
        assert_eq!(
            offered(&["v0.5.0", "v0.6.0-beta.1"], "v0.5.0").as_deref(),
            Some("v0.6.0-beta.1")
        );
    }

    #[test]
    fn stable_update_only_moves_between_stable_releases() {
        let offered = |tags: &[&str], from: &str| {
            let releases: Vec<_> = tags.iter().map(|t| release_for(t, "stable")).collect();
            select(&releases, "stable", "macos-arm64", Some(from)).map(|r| r.tag_name.clone())
        };
        assert_eq!(
            offered(&["v0.6.0-beta.1", "v0.6.0-alpha.1"], "v0.5.0"),
            None
        );
        assert_eq!(
            offered(&["v0.5.1", "v0.6.0-beta.1"], "v0.5.0").as_deref(),
            Some("v0.5.1")
        );
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

    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    /// Answers one connection per entry, in order, then stops listening, so a
    /// retry past the script is refused. `None` reads the request and hangs up
    /// after the client's timeout. A connection that never comes (fewer
    /// requests than scripted) ends the script after a few seconds instead of
    /// hanging the test. Returns the base URL and the requests received.
    fn stub(script: Vec<Option<String>>) -> (String, thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            listener.set_nonblocking(true).unwrap();
            let mut requests = Vec::new();
            for reply in script {
                let deadline = std::time::Instant::now() + Duration::from_secs(3);
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            if std::time::Instant::now() > deadline {
                                return requests;
                            }
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(e) => panic!("stub accept: {e}"),
                    }
                };
                stream.set_nonblocking(false).unwrap();
                let mut bytes = [0; 8192];
                let count = stream.read(&mut bytes).unwrap();
                let request = String::from_utf8_lossy(&bytes[..count]).into_owned();
                requests.push(request);
                match reply {
                    Some(reply) => stream.write_all(reply.as_bytes()).unwrap(),
                    None => thread::sleep(Duration::from_millis(350)),
                }
            }
            requests
        });
        (base, server)
    }

    fn reply(status: &str, headers: &str, body: &str) -> Option<String> {
        Some(format!(
            "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        ))
    }

    fn ok(body: &str) -> Option<String> {
        reply("200 OK", "", body)
    }

    fn gateway_timeout() -> Option<String> {
        reply("504 Gateway Timeout", "", "")
    }

    const FAST: Retry = Retry {
        attempts: 3,
        first_delay: Duration::from_millis(5),
        timeout: Duration::from_millis(250),
    };

    fn first_lines(requests: &[String]) -> Vec<&str> {
        requests
            .iter()
            .map(|r| r.lines().next().unwrap_or_default())
            .collect()
    }

    fn release_json(tag: &str, channel: &str) -> String {
        serde_json::to_string(&release_for(tag, channel)).unwrap()
    }

    #[test]
    fn github_request_retries_a_504_then_returns_the_release() {
        let (base, server) = stub(vec![
            gateway_timeout(),
            ok(&release_json("v0.5.2", "stable")),
        ]);
        let agent = ureq::Agent::new();
        let latest = GithubApi::new(&agent, base, FAST).latest().unwrap();
        assert_eq!(latest.unwrap().tag_name, "v0.5.2");
        let requests = server.join().unwrap();
        assert_eq!(first_lines(&requests), ["GET /releases/latest HTTP/1.1"; 2]);
    }

    #[test]
    fn github_request_retries_a_timeout() {
        let (base, server) = stub(vec![None, ok(&release_json("v0.5.2", "stable"))]);
        let agent = ureq::Agent::new();
        let latest = GithubApi::new(&agent, base, FAST).latest().unwrap();
        assert_eq!(latest.unwrap().tag_name, "v0.5.2");
        assert_eq!(server.join().unwrap().len(), 2);
    }

    #[test]
    fn github_request_retries_a_200_whose_body_stops_short() {
        let truncated =
            Some("HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\nshort".into());
        let (base, server) = stub(vec![truncated, ok(&release_json("v0.5.2", "stable"))]);
        let agent = ureq::Agent::new();
        let latest = GithubApi::new(&agent, base, FAST).latest().unwrap();
        assert_eq!(latest.unwrap().tag_name, "v0.5.2");
        assert_eq!(server.join().unwrap().len(), 2);
    }

    #[test]
    fn persistent_504_stops_after_the_bounded_attempts_with_a_friendly_error() {
        let (base, server) = stub(vec![gateway_timeout(); 3]);
        let agent = ureq::Agent::new();
        let started = std::time::Instant::now();
        let error = GithubApi::new(&agent, base, FAST).latest().unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(error.to_string(), "GitHub is slow, try again");
        let Error::GithubSlow { detail } = &error else {
            panic!("expected GithubSlow, got {error:?}");
        };
        assert!(detail.contains("status code 504"), "{detail}");
        assert!(detail.contains("after 3 attempts"), "{detail}");
        assert_eq!(server.join().unwrap().len(), 3);
    }

    #[test]
    fn client_errors_and_rate_limits_are_not_retried() {
        let cases = [
            (reply("400 Bad Request", "", ""), "status code 400"),
            (
                reply(
                    "403 Forbidden",
                    "x-ratelimit-remaining: 0\r\nx-ratelimit-reset: 1700000060\r\n",
                    "",
                ),
                "GitHub API rate limit reached",
            ),
            (
                reply("429 Too Many Requests", "retry-after: 60\r\n", ""),
                "GitHub API rate limit reached",
            ),
        ];
        for (response, expected) in cases {
            let (base, server) = stub(vec![response]);
            let agent = ureq::Agent::new();
            let error = GithubApi::new(&agent, base, FAST).latest().unwrap_err();
            assert!(error.to_string().contains(expected), "{error}");
            assert!(!matches!(error, Error::GithubSlow { .. }));
            assert_eq!(server.join().unwrap().len(), 1);
        }
        let (base, server) = stub(vec![reply("404 Not Found", "", "")]);
        let agent = ureq::Agent::new();
        assert!(
            GithubApi::new(&agent, base, FAST)
                .latest()
                .unwrap()
                .is_none()
        );
        assert_eq!(server.join().unwrap().len(), 1);
    }

    #[test]
    fn alpha_resolution_reads_tags_and_single_releases_never_the_list() {
        let refs = serde_json::json!([
            {"ref": "refs/tags/v0.4.0"},
            {"ref": "refs/tags/v0.5.0-alpha.6"},
            {"ref": "refs/tags/v0.5.0-alpha.7"},
            {"ref": "refs/tags/v0.5.0-beta.1"},
        ])
        .to_string();
        let (base, server) = stub(vec![
            ok(&refs),
            reply("404 Not Found", "", ""),
            ok(&release_json("v0.5.0-alpha.6", "alpha")),
        ]);
        let agent = ureq::Agent::new();
        let api = GithubApi::new(&agent, base, FAST);
        let chosen = resolve(&api, "alpha", "macos-arm64", None).unwrap();
        assert_eq!(chosen.unwrap().tag_name, "v0.5.0-alpha.6");
        assert_eq!(
            first_lines(&server.join().unwrap()),
            [
                "GET /git/matching-refs/tags/v HTTP/1.1",
                "GET /releases/tags/v0.5.0-alpha.7 HTTP/1.1",
                "GET /releases/tags/v0.5.0-alpha.6 HTTP/1.1",
            ]
        );
    }

    #[test]
    fn stable_resolution_reads_only_the_latest_release() {
        let (base, server) = stub(vec![ok(&release_json("v0.5.2", "stable"))]);
        let agent = ureq::Agent::new();
        let api = GithubApi::new(&agent, base, FAST);
        let chosen = resolve(&api, "main", "macos-arm64", Some("v0.5.1")).unwrap();
        assert_eq!(chosen.unwrap().tag_name, "v0.5.2");
        assert_eq!(
            first_lines(&server.join().unwrap()),
            ["GET /releases/latest HTTP/1.1"]
        );
    }

    /// The whole release list as a source, with GitHub's meaning of latest.
    struct Listed(Vec<Release>);

    impl ReleaseSource for Listed {
        fn latest(&self) -> Result<Option<Release>> {
            Ok(self
                .0
                .iter()
                .filter(|r| !r.draft && !r.prerelease)
                .max_by_key(|r| version(&r.tag_name))
                .cloned())
        }
        fn tags(&self) -> Result<Vec<String>> {
            Ok(self.0.iter().map(|r| r.tag_name.clone()).collect())
        }
        fn release(&self, tag: &str) -> Result<Option<Release>> {
            Ok(self
                .0
                .iter()
                .find(|r| r.tag_name == tag && !r.draft)
                .cloned())
        }
    }

    fn every_channel(tag: &str) -> Release {
        let mut release = release_for(tag, "alpha");
        for channel in ["beta", "stable"] {
            release.assets.extend(release_for(tag, channel).assets);
        }
        release
    }

    #[test]
    fn resolution_chooses_what_select_chooses_from_the_full_list() {
        let bare = |tag: &str| Release {
            assets: vec![],
            ..every_channel(tag)
        };
        let draft = |tag: &str| Release {
            draft: true,
            ..every_channel(tag)
        };
        let fixtures = vec![
            vec![
                every_channel("v0.5.0-alpha.6"),
                every_channel("v0.5.0-alpha.7"),
                every_channel("v0.5.0-beta.1"),
                every_channel("v0.4.0"),
                bare("v0.5.0-alpha.8"),
                draft("v0.5.0-alpha.9"),
            ],
            vec![
                every_channel("v0.6.0"),
                every_channel("v0.6.0-beta.1"),
                every_channel("v0.5.0-alpha.7"),
                every_channel("v0.3.5"),
                bare("v0.6.0-beta.2"),
            ],
            vec![every_channel("v0.3.5")],
            vec![
                every_channel("v1.0.0-alpha.2"),
                every_channel("v1.0.0-alpha.10"),
                every_channel("v0.9.0"),
                every_channel("v0.9.0-beta.4"),
                every_channel("v0.9.1-beta.1"),
            ],
        ];
        for releases in fixtures {
            let source = Listed(releases.clone());
            let currents =
                std::iter::once(None).chain(releases.iter().map(|r| Some(r.tag_name.as_str())));
            for current in currents {
                for channel in ["alpha", "beta", "stable"] {
                    let expected = select(&releases, channel, "macos-arm64", current)
                        .map(|r| r.tag_name.clone());
                    let resolved = resolve(&source, channel, "macos-arm64", current)
                        .unwrap()
                        .map(|r| r.tag_name);
                    assert_eq!(resolved, expected, "{channel} from {current:?}");
                }
            }
        }
    }

    #[test]
    fn stable_resolution_does_not_fall_back_past_an_incomplete_latest_release() {
        let releases = vec![
            every_channel("v0.3.4"),
            Release {
                assets: vec![],
                ..every_channel("v0.3.5")
            },
        ];
        assert!(
            resolve(&Listed(releases), "stable", "macos-arm64", None)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn resolution_looks_up_a_bounded_number_of_incomplete_tags() {
        let bare = |n: usize| Release {
            assets: vec![],
            ..every_channel(&format!("v0.5.0-alpha.{n}"))
        };
        let within: Vec<_> = (1..=MAX_TAG_LOOKUPS).map(bare).collect();
        assert!(
            resolve(&Listed(within), "alpha", "macos-arm64", None)
                .unwrap()
                .is_none()
        );
        let mut beyond: Vec<_> = (1..=MAX_TAG_LOOKUPS + 1).map(bare).collect();
        beyond.push(every_channel("v0.4.0"));
        let error = resolve(&Listed(beyond), "alpha", "macos-arm64", None).unwrap_err();
        assert!(error.to_string().contains("none of the newest"), "{error}");
    }

    #[test]
    fn github_request_sends_token_when_available() {
        let (base, server) = stub(vec![ok("[]")]);
        let previous = std::env::var_os("GITHUB_TOKEN");
        unsafe { std::env::set_var("GITHUB_TOKEN", "distribution-test-token") };
        let agent = ureq::Agent::new();
        let result = GithubApi::new(&agent, base, FAST).tags();
        if let Some(value) = previous {
            unsafe { std::env::set_var("GITHUB_TOKEN", value) };
        } else {
            unsafe { std::env::remove_var("GITHUB_TOKEN") };
        }
        assert!(result.unwrap().is_empty());
        assert!(
            server.join().unwrap()[0].contains("Authorization: Bearer distribution-test-token")
        );
    }
}
