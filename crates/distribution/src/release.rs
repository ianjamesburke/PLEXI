use crate::{Error, IoContext, Result};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path, time::Duration};

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
            current
                .as_ref()
                .is_none_or(|v| version(&r.tag_name).is_some_and(|next| &next > v))
        })
        .max_by_key(|r| version(&r.tag_name))
}

pub fn fetch(agent: &ureq::Agent, endpoint: &str) -> Result<Vec<Release>> {
    let mut releases = Vec::new();
    for page in 1..=1000 {
        let url = format!("{endpoint}?per_page=100&page={page}");
        let response = agent
            .get(&url)
            .set("User-Agent", "plexi-installer")
            .set("Accept", "application/vnd.github+json")
            .timeout(Duration::from_secs(30))
            .call()
            .map_err(|e| Error::Invalid(format!("fetch releases page {page}: {e}")))?;
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

pub fn download_package(
    agent: &ureq::Agent,
    base: &str,
    tag: &str,
    platform: &str,
    channel: &str,
    destination: &Path,
) -> Result<()> {
    validate_channel(channel)?;
    if version(tag).is_none() {
        return Err(Error::Invalid("invalid release tag".into()));
    }
    let name = archive_name(platform, channel);
    let archive = destination.join(&name);
    let sidecar = destination.join(format!("{name}.sha256"));
    download(agent, &format!("{base}/{tag}/{name}.sha256"), &sidecar)?;
    download(agent, &format!("{base}/{tag}/{name}"), &archive)?;
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
    fn channels_accept_stable_without_changing_install_identity() {
        assert!(accepts("alpha", "v1.0.0", false));
        assert!(accepts("beta", "v1.0.0", false));
        assert!(!accepts("stable", "v1.0.0", true));
        assert!(!accepts("beta", "v1.0.0-alpha.1", true));
        assert!(!accepts("alpha", "v1.0.0-windows.1", true));
    }
}
