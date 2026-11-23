//! Host adapter for the distribution crate's release policy and asset checks.
use std::cmp::Ordering;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseTag { pub raw: String }
impl ReleaseTag {
    pub fn parse(raw: &str) -> Option<Self> {
        plexi_distribution::release::version(raw)?;
        Some(Self { raw: raw.into() })
    }
}
impl Ord for ReleaseTag {
    fn cmp(&self, other: &Self) -> Ordering {
        plexi_distribution::release::version(&self.raw).cmp(&plexi_distribution::release::version(&other.raw))
    }
}
impl PartialOrd for ReleaseTag { fn partial_cmp(&self, other: &Self) -> Option<Ordering> { Some(self.cmp(other)) } }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateChannel { Stable, Beta, Alpha }
impl UpdateChannel {
    pub fn from_binary_name(name: &str) -> Self {
        match name.strip_suffix(".exe").unwrap_or(name) {
            "plexi-alpha" => Self::Alpha,
            "plexi-beta" => Self::Beta,
            s if s.starts_with("plexi-pr-") => Self::Alpha,
            _ => Self::Stable,
        }
    }
    pub fn key(self) -> &'static str { match self { Self::Stable => "stable", Self::Beta => "beta", Self::Alpha => "alpha" } }
}

/// The release the channel moves to from `current`, read through
/// `release::resolve` (never the paginated release list).
pub fn resolve_latest(
    agent: &ureq::Agent,
    channel: UpdateChannel,
    current: &ReleaseTag,
) -> Result<Option<ReleaseTag>, String> {
    use plexi_distribution::release;
    let platform = release::platform().map_err(|e| e.to_string())?;
    let api = release::GithubApi::new(agent, release::REPO_API_URL, release::Retry::GITHUB);
    let chosen = release::resolve(&api, channel.key(), &platform, Some(&current.raw))
        .map_err(|e| e.to_string())?;
    Ok(chosen.and_then(|r| ReleaseTag::parse(&r.tag_name)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windows_executable_suffix_preserves_channel_policy() {
        assert_eq!(UpdateChannel::from_binary_name("plexi-alpha.exe"), UpdateChannel::Alpha);
        assert_eq!(UpdateChannel::from_binary_name("plexi-beta.exe"), UpdateChannel::Beta);
        assert_eq!(UpdateChannel::from_binary_name("plexi.exe"), UpdateChannel::Stable);
    }
    #[test]
    fn prerelease_numbers_sort_semantically() {
        assert!(ReleaseTag::parse("v1.0.0-alpha.10").unwrap() > ReleaseTag::parse("v1.0.0-alpha.2").unwrap());
        assert!(ReleaseTag::parse("v1.0.0").unwrap() > ReleaseTag::parse("v1.0.0-beta.10").unwrap());
    }
}
