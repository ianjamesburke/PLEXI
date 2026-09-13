//! Host version compatibility — does this Plexi build satisfy an app's declared
//! version requirement?
//!
//! Apps declare `[requires]` in their manifest:
//!
//! ```toml
//! [requires]
//! plexi_min = "0.0.760"   # hard floor — host below this refuses to install/run
//! plexi_max = "0.1.0"     # soft ceiling — host above this warns (app may be superseded)
//! ```
//!
//! The split is deliberate. `plexi_min` is an enforced gate: an app that needs a
//! capability added in 0.0.760 must not silently misbehave on 0.0.700. `plexi_max`
//! is advisory: an app built for today's Plexi keeps running on a newer host, but
//! the user is warned that it predates this build and may need an update. Blocking
//! on the ceiling would brick every installed app on each release, so we never do.
//!
//! Versions are simple `major.minor.patch` triples (Plexi's scheme). No range
//! syntax, no pre-release tags — keep the contract obvious.

/// A parsed `major.minor.patch` version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub u64, pub u64, pub u64);

/// Whether the patch component may be omitted when parsing a version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchPolicy {
    /// `major.minor.patch` — a two-component string is rejected.
    Required,
    /// `major.minor[.patch]` — a missing patch reads as `0`.
    Optional,
}

impl Version {
    /// Parse a dotted numeric version into a [`Version`]. Returns `None` on any
    /// malformed input — callers treat an unparseable version as a hard error,
    /// never a silent pass. Surrounding whitespace is ignored; a non-numeric
    /// component, a fourth component, and (under [`PatchPolicy::Required`]) a
    /// missing patch are all rejected.
    ///
    /// The single version parser: manifest `[requires]` bounds, release tags
    /// (`src/cli/release_resolver.rs`), and CLI descriptor `plexi_version`
    /// (`src/app/plexi_descriptor.rs`) all read their numeric triple here.
    pub fn parse(s: &str, patch_policy: PatchPolicy) -> Option<Version> {
        let mut parts = s.trim().split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        let patch = match parts.next() {
            Some(p) => p.parse().ok()?,
            None if patch_policy == PatchPolicy::Optional => 0,
            None => return None,
        };
        if parts.next().is_some() {
            return None; // more than three components — reject
        }
        Some(Version(major, minor, patch))
    }
}

/// This build's version, from `CARGO_PKG_VERSION`.
pub fn current() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION"), PatchPolicy::Required)
        .expect("CARGO_PKG_VERSION must be a major.minor.patch triple")
}

/// The outcome of a compatibility check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionVerdict {
    /// Host satisfies the requirement. Install/run may proceed.
    Ok,
    /// Host is below `plexi_min`. Hard block — the requirement is included.
    TooOld { required_min: String, host: String },
    /// Host is above `plexi_max`. Advisory only — allow, but warn.
    TooNew { declared_max: String, host: String },
    /// A declared version string was malformed. Hard block — never guess.
    Malformed { field: &'static str, value: String },
}

impl VersionVerdict {
    /// Whether this verdict blocks install/launch (vs. merely warns).
    pub fn is_blocking(&self) -> bool {
        matches!(
            self,
            VersionVerdict::TooOld { .. } | VersionVerdict::Malformed { .. }
        )
    }

    /// Human-readable explanation, or `None` for [`VersionVerdict::Ok`].
    pub fn message(&self) -> Option<String> {
        match self {
            VersionVerdict::Ok => None,
            VersionVerdict::TooOld { required_min, host } => Some(format!(
                "this app requires Plexi >= {required_min}, but this build is {host}. \
                 Update Plexi to install it."
            )),
            VersionVerdict::TooNew { declared_max, host } => Some(format!(
                "this app was built for Plexi <= {declared_max}; this build is {host}. \
                 It may be superseded — update the app if it misbehaves."
            )),
            VersionVerdict::Malformed { field, value } => Some(format!(
                "manifest [requires].{field} = \"{value}\" is not a valid major.minor.patch version"
            )),
        }
    }
}

/// Check a declared min/max against the current host version. Pure — the host
/// version is injected so this is fully unit-testable.
pub fn check(min: Option<&str>, max: Option<&str>, host: Version) -> VersionVerdict {
    if let Some(min_s) = min {
        let Some(min_v) = Version::parse(min_s, PatchPolicy::Required) else {
            return VersionVerdict::Malformed {
                field: "plexi_min",
                value: min_s.to_string(),
            };
        };
        if host < min_v {
            return VersionVerdict::TooOld {
                required_min: min_s.to_string(),
                host: format!("{}.{}.{}", host.0, host.1, host.2),
            };
        }
    }
    if let Some(max_s) = max {
        let Some(max_v) = Version::parse(max_s, PatchPolicy::Required) else {
            return VersionVerdict::Malformed {
                field: "plexi_max",
                value: max_s.to_string(),
            };
        };
        if host > max_v {
            return VersionVerdict::TooNew {
                declared_max: max_s.to_string(),
                host: format!("{}.{}.{}", host.0, host.1, host.2),
            };
        }
    }
    VersionVerdict::Ok
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Option<Version> {
        Version::parse(s, PatchPolicy::Required)
    }

    #[test]
    fn parses_triples_and_rejects_junk() {
        assert_eq!(parse("0.0.760"), Some(Version(0, 0, 760)));
        assert_eq!(parse("1.2.3"), Some(Version(1, 2, 3)));
        assert_eq!(parse(" 1.2.3 "), Some(Version(1, 2, 3)));
        assert_eq!(parse("1.2"), None);
        assert_eq!(parse("1.2.3.4"), None);
        assert_eq!(parse("1.2.x"), None);
        assert_eq!(parse("v1.2.3"), None);
    }

    #[test]
    fn optional_patch_defaults_to_zero_and_still_rejects_junk() {
        let optional = |s| Version::parse(s, PatchPolicy::Optional);
        assert_eq!(optional("1.2"), Some(Version(1, 2, 0)));
        assert_eq!(optional("1.2.3"), Some(Version(1, 2, 3)));
        assert_eq!(optional("1"), None);
        assert_eq!(optional("1.2.3.4"), None);
        assert_eq!(optional("1.2.x"), None);
    }

    #[test]
    fn ordering_is_numeric_not_lexical() {
        // 0.0.9 < 0.0.760 numerically (lexical string compare would get this wrong)
        assert!(Version(0, 0, 9) < Version(0, 0, 760));
        assert!(Version(0, 1, 0) > Version(0, 0, 999));
        assert!(Version(1, 0, 0) > Version(0, 99, 99));
    }

    #[test]
    fn no_requirement_is_always_ok() {
        assert_eq!(check(None, None, Version(0, 0, 1)), VersionVerdict::Ok);
    }

    #[test]
    fn min_gate_blocks_when_host_too_old() {
        let v = check(Some("0.0.760"), None, Version(0, 0, 700));
        assert!(v.is_blocking());
        assert!(matches!(v, VersionVerdict::TooOld { .. }));
        // exactly at min is fine
        assert_eq!(
            check(Some("0.0.760"), None, Version(0, 0, 760)),
            VersionVerdict::Ok
        );
        // above min is fine
        assert_eq!(
            check(Some("0.0.760"), None, Version(0, 1, 0)),
            VersionVerdict::Ok
        );
    }

    #[test]
    fn max_ceiling_warns_but_does_not_block() {
        let v = check(None, Some("0.0.760"), Version(0, 1, 0));
        assert!(!v.is_blocking(), "too-new must warn, not block");
        assert!(matches!(v, VersionVerdict::TooNew { .. }));
        // at or below max is fine
        assert_eq!(
            check(None, Some("0.0.760"), Version(0, 0, 760)),
            VersionVerdict::Ok
        );
    }

    #[test]
    fn malformed_requirement_blocks() {
        let v = check(Some("not-a-version"), None, Version(0, 0, 760));
        assert!(v.is_blocking());
        assert!(matches!(
            v,
            VersionVerdict::Malformed {
                field: "plexi_min",
                ..
            }
        ));
    }

    #[test]
    fn current_build_version_parses() {
        // Smoke: the build's own version must be a valid triple.
        let _ = current();
    }
}
