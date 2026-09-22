//! Release-channel gates for features that are not part of the stable v1 scope.
//!
//! Stable v1 is intentionally narrow. Release-gated features stay available in
//! alpha, beta, and PR builds according to the minimum tier declared here, so
//! they can keep moving without leaking into the public stable product. Local
//! `rc-*` channels are explicit stable-tier release candidates.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReleaseFeature {
    Assistant,
    AppWrappers,
    Marketplace,
    Daw,
    MediaIo,
    Accessibility,
    McpClient,
    Routines,
}

impl ReleaseFeature {
    pub fn name(self) -> &'static str {
        match self {
            Self::Assistant => "assistant",
            Self::AppWrappers => "app wrappers",
            Self::Marketplace => "marketplace",
            Self::Daw => "DAW",
            Self::MediaIo => "media I/O",
            Self::Accessibility => "experimental accessibility",
            Self::McpClient => "MCP client",
            Self::Routines => "routines",
        }
    }

    pub fn minimum_tier(self) -> ReleaseTier {
        match self {
            Self::Assistant
            | Self::AppWrappers
            | Self::Marketplace
            | Self::Daw
            | Self::MediaIo
            | Self::Accessibility
            | Self::McpClient
            | Self::Routines => ReleaseTier::Beta,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum ReleaseTier {
    Stable,
    Beta,
    Alpha,
}

impl ReleaseTier {
    pub fn for_channel(channel: Option<&str>) -> Option<Self> {
        match channel {
            None => Some(Self::Stable),
            Some("main") => Some(Self::Stable),
            Some(name) if name.starts_with("rc-") => Some(Self::Stable),
            Some("beta") => Some(Self::Beta),
            Some("alpha") => Some(Self::Alpha),
            Some(name) if name.starts_with("pr-") => Some(Self::Alpha),
            Some(_) => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Beta => "beta",
            Self::Alpha => "alpha",
        }
    }

    fn binary_hint(self) -> &'static str {
        match self {
            Self::Stable => "plexi",
            Self::Beta => "plexi-beta or plexi-alpha",
            Self::Alpha => "plexi-alpha",
        }
    }
}

pub fn feature_enabled(feature: ReleaseFeature) -> bool {
    feature_enabled_for_channel(feature, crate::config::build_channel().as_deref())
}

pub fn log_feature_blocked(feature: ReleaseFeature) {
    log::info!(
        "release_gate: blocked access to {} feature; requires {:?}",
        feature.name(),
        feature.minimum_tier()
    );
}

pub fn feature_unavailable_message(feature: ReleaseFeature) -> String {
    log_feature_blocked(feature);
    feature_unavailable_text(feature)
}

/// The user-facing unavailable text without the block-log side effect, for
/// static surfaces such as generated help.
pub fn feature_unavailable_text(feature: ReleaseFeature) -> String {
    format!(
        "{} requires the {} channel and is not part of the stable v1 surface. Use {} to try it.",
        feature.name(),
        feature.minimum_tier().label(),
        feature.minimum_tier().binary_hint()
    )
}

pub(crate) fn feature_enabled_for_channel(feature: ReleaseFeature, channel: Option<&str>) -> bool {
    ReleaseTier::for_channel(channel).is_some_and(|tier| tier >= feature.minimum_tier())
}

#[cfg(test)]
mod tests {
    use super::{feature_enabled_for_channel, ReleaseFeature, ReleaseTier};

    #[test]
    fn v1_channels_disable_all_excluded_runtime_surfaces() {
        // These surfaces may remain compiled for alpha/beta development, but
        // a normal v1 host boot must not construct or contact any of them.
        // Keep this list synchronized with every excluded runtime entry point.
        for feature in [
            ReleaseFeature::Assistant,
            ReleaseFeature::Marketplace,
            ReleaseFeature::McpClient,
            ReleaseFeature::Routines,
        ] {
            assert!(!feature_enabled_for_channel(feature, None));
            assert!(!feature_enabled_for_channel(feature, Some("main")));
            assert!(!feature_enabled_for_channel(feature, Some("rc-010")));
        }
    }

    #[test]
    fn alpha_beta_and_pr_channels_enable_excluded_development_surfaces() {
        for feature in [
            ReleaseFeature::Assistant,
            ReleaseFeature::Marketplace,
            ReleaseFeature::McpClient,
            ReleaseFeature::Routines,
        ] {
            assert!(feature_enabled_for_channel(feature, Some("alpha")));
            assert!(feature_enabled_for_channel(feature, Some("beta")));
            assert!(feature_enabled_for_channel(feature, Some("pr-2259")));
        }
    }

    #[test]
    fn stable_and_rc_channels_disable_v1_stubbed_surfaces() {
        for feature in [
            ReleaseFeature::Daw,
            ReleaseFeature::MediaIo,
            ReleaseFeature::Accessibility,
        ] {
            assert!(!feature_enabled_for_channel(feature, None));
            assert!(!feature_enabled_for_channel(feature, Some("main")));
            assert!(!feature_enabled_for_channel(feature, Some("rc-010")));
            assert!(feature_enabled_for_channel(feature, Some("alpha")));
            assert!(feature_enabled_for_channel(feature, Some("beta")));
        }
    }

    #[test]
    fn unknown_named_channels_disable_release_gated_features() {
        assert!(!feature_enabled_for_channel(
            ReleaseFeature::Marketplace,
            Some("client")
        ));
    }

    #[test]
    fn alpha_tier_is_higher_than_beta_for_future_alpha_only_features() {
        assert!(ReleaseTier::Alpha > ReleaseTier::Beta);
        assert!(ReleaseTier::Beta > ReleaseTier::Stable);
    }
}
