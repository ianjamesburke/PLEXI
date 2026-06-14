//! Release-channel gates for features that are not part of the stable v1 scope.
//!
//! Stable v1 is intentionally narrow: terminal multiplexing plus bundled core
//! apps. Release-gated features stay available in alpha, beta, and PR builds
//! according to the minimum channel declared here, so they can keep moving
//! without leaking into the stable product.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReleaseFeature {
    Assistant,
    AppWrappers,
    Marketplace,
}

impl ReleaseFeature {
    pub fn name(self) -> &'static str {
        match self {
            Self::Assistant => "assistant",
            Self::AppWrappers => "app wrappers",
            Self::Marketplace => "marketplace",
        }
    }

    pub fn minimum_channel(self) -> ReleaseTier {
        match self {
            Self::Assistant | Self::AppWrappers | Self::Marketplace => ReleaseTier::Beta,
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
    fn current_for_channel(channel: Option<&str>) -> Option<Self> {
        match channel {
            None => Some(Self::Stable),
            Some("beta") => Some(Self::Beta),
            Some("alpha") => Some(Self::Alpha),
            Some(name) if name.starts_with("pr-") => Some(Self::Alpha),
            Some(_) => None,
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
        feature.minimum_channel()
    );
}

pub fn feature_unavailable_message(feature: ReleaseFeature) -> String {
    log_feature_blocked(feature);
    format!(
        "{} requires the {} channel and is not part of the stable v1 surface. Use {} to try it.",
        feature.name(),
        feature.minimum_channel().label(),
        feature.minimum_channel().binary_hint()
    )
}

impl ReleaseTier {
    fn label(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Beta => "beta",
            Self::Alpha => "alpha",
        }
    }
}

fn feature_enabled_for_channel(feature: ReleaseFeature, channel: Option<&str>) -> bool {
    ReleaseTier::current_for_channel(channel).is_some_and(|tier| tier >= feature.minimum_channel())
}

#[cfg(test)]
mod tests {
    use super::{feature_enabled_for_channel, ReleaseFeature, ReleaseTier};

    #[test]
    fn stable_main_channel_disables_beta_features() {
        assert!(!feature_enabled_for_channel(
            ReleaseFeature::Marketplace,
            None
        ));
    }

    #[test]
    fn alpha_beta_and_pr_channels_enable_beta_features() {
        assert!(feature_enabled_for_channel(
            ReleaseFeature::Marketplace,
            Some("alpha")
        ));
        assert!(feature_enabled_for_channel(
            ReleaseFeature::Marketplace,
            Some("beta")
        ));
        assert!(feature_enabled_for_channel(
            ReleaseFeature::Marketplace,
            Some("pr-2259")
        ));
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
