//! Channel-aware release resolution via the GitHub releases API.

use crate::app::host_version::{PatchPolicy, Version};
use std::cmp::Ordering;
use std::time::Duration;

use serde::Deserialize;

const RELEASES_URL: &str = "https://api.github.com/repos/ianjamesburke/PLEXI/releases";

/// A parsed release tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseTag {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub pre: Option<Prerelease>,
    pub raw: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prerelease {
    pub kind: PreKind,
    pub num: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PreKind {
    /// Disposable Windows dogfood tags (`vX.Y.Z-windows.N`). Sorted below
    /// alpha so a normal alpha install never "upgrades" into a windows-only cut.
    Windows,
    Alpha,
    Beta,
}

/// Which releases a channel accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateChannel {
    /// only plain vX.Y.Z
    Stable,
    /// beta + stable, never alpha
    Beta,
    /// alpha + beta + stable
    Alpha,
}

impl ReleaseTag {
    /// Parse `vX.Y.Z`, `vX.Y.Z-alpha.N`, `vX.Y.Z-beta.N`, `vX.Y.Z-windows.N`.
    /// Returns `None` for malformed tags or unrecognized prerelease schemes
    /// (e.g. old `-rc1` tags).
    pub fn parse(tag: &str) -> Option<Self> {
        let raw = tag.to_string();
        let body = tag.strip_prefix('v').unwrap_or(tag);
        let (version, pre_str) = match body.split_once('-') {
            Some((v, p)) => (v, Some(p)),
            None => (body, None),
        };

        let Version(major, minor, patch) = Version::parse(version, PatchPolicy::Required)?;

        let pre = match pre_str {
            None => None,
            Some(p) => {
                let (kind_str, num_str) = p.split_once('.')?;
                let kind = match kind_str {
                    "alpha" => PreKind::Alpha,
                    "beta" => PreKind::Beta,
                    "windows" => PreKind::Windows,
                    _ => return None,
                };
                let num = num_str.parse().ok()?;
                Some(Prerelease { kind, num })
            }
        };

        Some(ReleaseTag {
            major,
            minor,
            patch,
            pre,
            raw,
        })
    }
}

impl Ord for ReleaseTag {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch)
            .cmp(&(other.major, other.minor, other.patch))
            .then_with(|| match (&self.pre, &other.pre) {
                // A release with no prerelease is greater than any prerelease
                // of the same base version (SemVer §11).
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(a), Some(b)) => a.kind.cmp(&b.kind).then(a.num.cmp(&b.num)),
            })
    }
}

impl PartialOrd for ReleaseTag {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl UpdateChannel {
    /// `plexi` → Stable, `plexi-beta` → Beta, `plexi-alpha`/`plexi-pr-*` → Alpha.
    /// Unknown channels default to Stable (the most conservative policy).
    pub fn from_binary_name(name: &str) -> Self {
        let suffix = match name.strip_prefix("plexi") {
            Some(s) => s.strip_prefix('-').unwrap_or(""),
            None => "",
        };
        if suffix.is_empty() {
            UpdateChannel::Stable
        } else if suffix == "beta" {
            UpdateChannel::Beta
        } else if suffix == "alpha" || suffix.starts_with("pr-") {
            UpdateChannel::Alpha
        } else {
            UpdateChannel::Stable
        }
    }

    /// Filtering logic per channel policy.
    pub fn accepts(&self, tag: &ReleaseTag) -> bool {
        match (self, &tag.pre) {
            (_, None) => true, // every channel accepts stable releases
            (UpdateChannel::Stable, Some(_)) => false,
            (UpdateChannel::Beta, Some(p)) => p.kind == PreKind::Beta,
            (UpdateChannel::Alpha, Some(_)) => true,
        }
    }
}

/// A GitHub release as returned by the releases list endpoint.
#[derive(Debug, Clone, Deserialize)]
pub struct GithubRelease {
    pub tag_name: String,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub draft: bool,
}

/// Pick the best update candidate for `channel` that is strictly newer than `current`.
pub fn resolve_best(
    releases: &[GithubRelease],
    channel: UpdateChannel,
    current: &ReleaseTag,
) -> Option<ReleaseTag> {
    let current_is_windows = matches!(
        current.pre.as_ref().map(|p| p.kind),
        Some(PreKind::Windows)
    );
    releases
        .iter()
        .filter(|r| !r.draft)
        .filter_map(|r| ReleaseTag::parse(&r.tag_name).map(|t| (r.prerelease, t)))
        // GitHub's prerelease flag is authoritative: a release GitHub marks as a
        // prerelease must never reach the Stable channel, even if the tag string
        // parsed as a plain vX.Y.Z.
        .filter(|(is_prerelease, t)| {
            if *is_prerelease && t.pre.is_none() {
                channel != UpdateChannel::Stable
            } else {
                channel.accepts(t)
            }
        })
        // Windows dogfood tags stay on their own track: a `*-windows.N` install
        // only updates to a newer windows tag, and alpha/stable never jump onto
        // a windows-only cut that may lack other OS assets.
        .filter(|(_, t)| {
            let candidate_is_windows = matches!(
                t.pre.as_ref().map(|p| p.kind),
                Some(PreKind::Windows)
            );
            current_is_windows == candidate_is_windows
        })
        .filter(|(_, t)| t > current)
        .map(|(_, t)| t)
        .max()
}

/// Result of comparing the on-disk bundle version against a running host's
/// version (stint 0596). `Unknown` means either string failed to parse as a
/// `ReleaseTag` — never silently treated as in-sync.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkewStatus {
    /// Running host version matches or is newer than the on-disk bundle.
    InSync,
    /// Running host is older than the on-disk bundle — a restart would pick
    /// up a newer version.
    Skewed { bundle: String, running: String },
    /// One or both versions could not be parsed as a `ReleaseTag`.
    Unknown { reason: String },
}

/// Real-SemVer comparison (via `ReleaseTag::Ord`, which orders prereleases
/// correctly per SemVer §11) between the version installed on disk
/// (`bundle_raw`) and the version a running host process was launched with
/// (`running_raw`). A running host that is *newer* than the on-disk bundle
/// (e.g. mid-update) is not an error state — only `running < bundle` is
/// `Skewed`.
pub fn detect_version_skew(bundle_raw: &str, running_raw: &str) -> SkewStatus {
    let bundle = match ReleaseTag::parse(bundle_raw) {
        Some(t) => t,
        None => {
            return SkewStatus::Unknown {
                reason: format!("could not parse bundle version {bundle_raw:?}"),
            }
        }
    };
    let running = match ReleaseTag::parse(running_raw) {
        Some(t) => t,
        None => {
            return SkewStatus::Unknown {
                reason: format!("could not parse running host version {running_raw:?}"),
            }
        }
    };
    if running < bundle {
        SkewStatus::Skewed {
            bundle: bundle_raw.to_string(),
            running: running_raw.to_string(),
        }
    } else {
        SkewStatus::InSync
    }
}

/// Fetch all releases from the GitHub releases list endpoint.
pub fn fetch_releases(agent: &ureq::Agent) -> Result<Vec<GithubRelease>, String> {
    let body = agent
        .get(RELEASES_URL)
        .set("User-Agent", "plexi-updater")
        .set("Accept", "application/vnd.github+json")
        .timeout(Duration::from_secs(30))
        .call()
        .map_err(|e| format!("failed to fetch releases: {e}"))?
        .into_string()
        .map_err(|e| format!("failed to read releases response: {e}"))?;
    serde_json::from_str(&body).map_err(|e| format!("failed to parse releases response: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(s: &str) -> ReleaseTag {
        ReleaseTag::parse(s).expect("valid tag")
    }

    #[test]
    fn parse_stable() {
        let t = tag("v0.1.12");
        assert_eq!((t.major, t.minor, t.patch), (0, 1, 12));
        assert!(t.pre.is_none());
        assert_eq!(t.raw, "v0.1.12");
    }

    #[test]
    fn parse_without_v_prefix() {
        let t = tag("1.2.3");
        assert_eq!((t.major, t.minor, t.patch), (1, 2, 3));
    }

    #[test]
    fn parse_alpha() {
        let t = tag("v0.1.12-alpha.3");
        assert_eq!(
            t.pre,
            Some(Prerelease {
                kind: PreKind::Alpha,
                num: 3
            })
        );
    }

    #[test]
    fn parse_beta() {
        let t = tag("v0.1.12-beta.1");
        assert_eq!(
            t.pre,
            Some(Prerelease {
                kind: PreKind::Beta,
                num: 1
            })
        );
    }

    #[test]
    fn parse_malformed() {
        assert!(ReleaseTag::parse("v1.2").is_none());
        assert!(ReleaseTag::parse("v1.2.3.4").is_none());
        assert!(ReleaseTag::parse("vfoo").is_none());
        assert!(ReleaseTag::parse("v1.2.x").is_none());
    }

    #[test]
    fn parse_old_rc_tag_rejected() {
        assert!(ReleaseTag::parse("v3.5.0-rc1").is_none());
        assert!(ReleaseTag::parse("v3.5.0-rc.1").is_none());
    }

    #[test]
    fn ordering_cross_version() {
        assert!(tag("v0.2.0") > tag("v0.1.99"));
        assert!(tag("v1.0.0") > tag("v0.9.9"));
        assert!(tag("v0.1.13") > tag("v0.1.12"));
    }

    #[test]
    fn ordering_prerelease_below_release() {
        assert!(tag("v0.1.12") > tag("v0.1.12-beta.1"));
        assert!(tag("v0.1.12") > tag("v0.1.12-alpha.9"));
    }

    #[test]
    fn ordering_same_base_prereleases() {
        assert!(tag("v0.1.12-alpha.1") < tag("v0.1.12-alpha.2"));
        assert!(tag("v0.1.12-alpha.2") < tag("v0.1.12-beta.1"));
        assert!(tag("v0.1.12-beta.1") < tag("v0.1.12-beta.2"));
        assert!(tag("v0.1.12-beta.2") < tag("v0.1.12"));
    }

    #[test]
    fn channel_acceptance() {
        let stable = tag("v0.1.12");
        let beta = tag("v0.1.12-beta.1");
        let alpha = tag("v0.1.12-alpha.1");

        assert!(UpdateChannel::Stable.accepts(&stable));
        assert!(!UpdateChannel::Stable.accepts(&beta));
        assert!(!UpdateChannel::Stable.accepts(&alpha));

        assert!(UpdateChannel::Beta.accepts(&stable));
        assert!(UpdateChannel::Beta.accepts(&beta));
        assert!(!UpdateChannel::Beta.accepts(&alpha));

        assert!(UpdateChannel::Alpha.accepts(&stable));
        assert!(UpdateChannel::Alpha.accepts(&beta));
        assert!(UpdateChannel::Alpha.accepts(&alpha));
    }

    #[test]
    fn from_binary_name() {
        assert_eq!(
            UpdateChannel::from_binary_name("plexi"),
            UpdateChannel::Stable
        );
        assert_eq!(
            UpdateChannel::from_binary_name("plexi-beta"),
            UpdateChannel::Beta
        );
        assert_eq!(
            UpdateChannel::from_binary_name("plexi-alpha"),
            UpdateChannel::Alpha
        );
        assert_eq!(
            UpdateChannel::from_binary_name("plexi-pr-123"),
            UpdateChannel::Alpha
        );
        assert_eq!(
            UpdateChannel::from_binary_name("plexi-rc-010"),
            UpdateChannel::Stable
        );
    }

    fn rel(t: &str) -> GithubRelease {
        GithubRelease {
            tag_name: t.to_string(),
            prerelease: false,
            draft: false,
        }
    }

    #[test]
    fn resolve_best_alpha_picks_newest_prerelease() {
        let releases = vec![
            rel("v0.1.12"),
            rel("v0.1.13-alpha.1"),
            rel("v0.1.13-alpha.2"),
            rel("v0.1.13-beta.1"),
        ];
        let current = tag("v0.1.12");
        let best = resolve_best(&releases, UpdateChannel::Alpha, &current).unwrap();
        assert_eq!(best.raw, "v0.1.13-beta.1");
    }

    #[test]
    fn resolve_best_stable_ignores_prereleases() {
        let releases = vec![rel("v0.1.13-alpha.5"), rel("v0.1.13-beta.2")];
        let current = tag("v0.1.12");
        assert!(resolve_best(&releases, UpdateChannel::Stable, &current).is_none());
    }

    #[test]
    fn resolve_best_skips_drafts() {
        let mut draft = rel("v0.2.0");
        draft.draft = true;
        let releases = vec![draft, rel("v0.1.13")];
        let current = tag("v0.1.12");
        let best = resolve_best(&releases, UpdateChannel::Stable, &current).unwrap();
        assert_eq!(best.raw, "v0.1.13");
    }

    #[test]
    fn resolve_best_none_when_current_is_newest() {
        let releases = vec![rel("v0.1.10"), rel("v0.1.11")];
        let current = tag("v0.1.12");
        assert!(resolve_best(&releases, UpdateChannel::Alpha, &current).is_none());
    }

    #[test]
    fn resolve_best_github_prerelease_flag_excluded_from_stable() {
        // A stable-looking tag GitHub flagged as prerelease must not reach stable.
        let mut flagged = rel("v0.1.13");
        flagged.prerelease = true;
        let releases = vec![flagged];
        let current = tag("v0.1.12");
        assert!(resolve_best(&releases, UpdateChannel::Stable, &current).is_none());
        // But alpha still accepts it.
        assert!(resolve_best(&releases, UpdateChannel::Alpha, &current).is_some());
    }

    #[test]
    fn parse_windows() {
        let t = tag("v0.3.1-windows.1");
        assert_eq!(
            t.pre,
            Some(Prerelease {
                kind: PreKind::Windows,
                num: 1
            })
        );
    }

    #[test]
    fn resolve_best_windows_stays_on_windows_track() {
        let releases = vec![
            rel("v0.3.1-alpha.6"),
            rel("v0.3.1-windows.1"),
            rel("v0.3.1-windows.2"),
        ];
        let current = tag("v0.3.1-windows.1");
        let best = resolve_best(&releases, UpdateChannel::Alpha, &current).unwrap();
        assert_eq!(best.raw, "v0.3.1-windows.2");
    }

    #[test]
    fn resolve_best_alpha_ignores_windows_tags() {
        let releases = vec![rel("v0.3.1-windows.9"), rel("v0.3.1-alpha.6")];
        let current = tag("v0.3.1-alpha.5");
        let best = resolve_best(&releases, UpdateChannel::Alpha, &current).unwrap();
        assert_eq!(best.raw, "v0.3.1-alpha.6");
    }

    #[test]
    fn resolve_best_beta_rejects_alpha_accepts_beta() {

        let releases = vec![rel("v0.1.13-alpha.9"), rel("v0.1.13-beta.1")];
        let current = tag("v0.1.12");
        let best = resolve_best(&releases, UpdateChannel::Beta, &current).unwrap();
        assert_eq!(best.raw, "v0.1.13-beta.1");
    }

    #[test]
    fn skew_in_sync_when_equal() {
        assert_eq!(detect_version_skew("v0.2.0", "v0.2.0"), SkewStatus::InSync);
    }

    #[test]
    fn skew_in_sync_when_running_is_newer() {
        // Transient state (mid-update) is not an error.
        assert_eq!(detect_version_skew("v0.2.0", "v0.2.1"), SkewStatus::InSync);
    }

    #[test]
    fn skew_detected_on_patch_behind() {
        assert_eq!(
            detect_version_skew("v0.2.1", "v0.2.0"),
            SkewStatus::Skewed {
                bundle: "v0.2.1".to_string(),
                running: "v0.2.0".to_string(),
            }
        );
    }

    #[test]
    fn skew_detected_on_major_minor_behind() {
        assert_eq!(
            detect_version_skew("v0.2.0", "v0.1.16"),
            SkewStatus::Skewed {
                bundle: "v0.2.0".to_string(),
                running: "v0.1.16".to_string(),
            }
        );
    }

    #[test]
    fn skew_detected_running_old_prerelease_bundle_final() {
        assert_eq!(
            detect_version_skew("v0.2.0", "v0.2.0-alpha.1"),
            SkewStatus::Skewed {
                bundle: "v0.2.0".to_string(),
                running: "v0.2.0-alpha.1".to_string(),
            }
        );
    }

    #[test]
    fn skew_detected_running_old_prerelease_bundle_newer_prerelease() {
        assert_eq!(
            detect_version_skew("v0.2.0-alpha.2", "v0.2.0-alpha.1"),
            SkewStatus::Skewed {
                bundle: "v0.2.0-alpha.2".to_string(),
                running: "v0.2.0-alpha.1".to_string(),
            }
        );
    }

    #[test]
    fn skew_unknown_on_unparseable_bundle() {
        match detect_version_skew("not-a-version", "v0.2.0") {
            SkewStatus::Unknown { reason } => {
                assert!(reason.contains("bundle"), "reason was: {reason}");
            }
            other => panic!("expected Unknown, got {other:?}"),
        }
    }

    #[test]
    fn skew_unknown_on_unparseable_running() {
        match detect_version_skew("v0.2.0", "not-a-version") {
            SkewStatus::Unknown { reason } => {
                assert!(reason.contains("running"), "reason was: {reason}");
            }
            other => panic!("expected Unknown, got {other:?}"),
        }
    }
}
