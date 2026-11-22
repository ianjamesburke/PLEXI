//! The `--profile` name contract.
//!
//! A profile name selects one directory, `~/.plexi-<name>`, so it must be a
//! single path component that cannot traverse, alias or nest. The grammar is
//! `[A-Za-z0-9][A-Za-z0-9-]{0,63}` (length in bytes). Exactly `default` is a
//! selector keyword for the binary's own profile, not a name. Anything else is
//! a usage error: no trimming, no replacement, no fallback.
//!
//! Parsing is pure. It never reads the filesystem, so the CLI front end can
//! reject a selector before it touches any profile.

use std::fmt;

/// Longest accepted profile name, in bytes.
pub const PROFILE_NAME_MAX: usize = 64;

/// A validated explicit profile name. Its spelling is preserved: on a
/// case-sensitive volume `Team` and `team` are distinct profiles.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileName(String);

impl ProfileName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProfileName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What the invocation's one `--profile` occurrence selects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfileSelection {
    /// No `--profile`, or exactly `default`: the binary's own profile.
    Default,
    Named(ProfileName),
}

impl ProfileSelection {
    /// Parse one `--profile` value. The error is the user-facing reason.
    pub fn parse(raw: &str) -> Result<Self, String> {
        if raw == "default" {
            return Ok(Self::Default);
        }
        let reason = if raw.eq_ignore_ascii_case("default") {
            format!("profile name '{raw}' is reserved and cannot name a profile")
        } else if is_profile_name(raw) {
            return Ok(Self::Named(ProfileName(raw.to_string())));
        } else {
            format!(
                "invalid profile name '{}': use 1-{PROFILE_NAME_MAX} ASCII letters, digits or hyphens, starting with a letter or digit",
                raw.escape_debug()
            )
        };
        Err(format!(
            "{reason}\n  note: if ~/.plexi-{} exists, rename that directory to a valid, unreserved profile name (for example ~/.plexi-{}-legacy) and pass that name to --profile; Plexi never renames a profile itself",
            raw.escape_debug(),
            migration_example(raw)
        ))
    }
}

/// A grammar-valid stand-in name derived from `raw`, for the migration hint.
fn migration_example(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('-');
    let base = if trimmed.is_empty() {
        "profile"
    } else {
        trimmed
    };
    base.chars()
        .take(PROFILE_NAME_MAX - "-legacy".len())
        .collect()
}

pub(crate) fn is_profile_name(raw: &str) -> bool {
    let bytes = raw.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= PROFILE_NAME_MAX
        && bytes[0].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(raw: &str) -> ProfileSelection {
        ProfileSelection::Named(ProfileName(raw.to_string()))
    }

    #[test]
    fn grammar_accepts_channel_shaped_mixed_case_and_leading_digit_names() {
        for raw in [
            "main",
            "stable",
            "alpha",
            "pr-3137",
            "Team",
            "050rename",
            "a",
        ] {
            assert_eq!(ProfileSelection::parse(raw), Ok(named(raw)), "{raw}");
        }
        let longest = "a".repeat(PROFILE_NAME_MAX);
        assert_eq!(ProfileSelection::parse(&longest), Ok(named(&longest)));
    }

    /// A reserved spelling names the directory it would have selected and a
    /// valid migration target, and never advises lowercase `default` (which
    /// selects a different profile).
    #[test]
    fn default_is_a_selector_and_its_case_variants_are_reserved() {
        assert_eq!(
            ProfileSelection::parse("default"),
            Ok(ProfileSelection::Default)
        );
        for raw in ["Default", "DEFAULT"] {
            let error = ProfileSelection::parse(raw).unwrap_err();
            assert!(error.contains("reserved"), "{error}");
            assert!(error.contains(&format!("~/.plexi-{raw} exists")), "{error}");
            assert!(error.contains("~/.plexi-default-legacy"), "{error}");
            assert!(!error.contains("'default'"), "{error}");
        }
    }

    #[test]
    fn migration_examples_are_valid_unreserved_names() {
        for raw in [
            "Default",
            "DEFAULT",
            "beta.backup-20261104-1637",
            "../x",
            "...",
        ] {
            let example = format!("{}-legacy", migration_example(raw));
            assert_eq!(
                ProfileSelection::parse(&example),
                Ok(ProfileSelection::Named(ProfileName(example.clone()))),
                "{raw}"
            );
        }
    }

    #[test]
    fn grammar_rejects_paths_and_anything_that_is_not_one_plain_component() {
        let overlong = "a".repeat(PROFILE_NAME_MAX + 1);
        for raw in [
            "",
            "nested/name",
            "../x",
            "..",
            ".",
            "a b",
            "back\\slash",
            "beta.backup-20261104-1637",
            "-leading",
            "naïve",
            "tab\tname",
            overlong.as_str(),
        ] {
            assert!(ProfileSelection::parse(raw).is_err(), "{raw:?} accepted");
        }
    }
}
