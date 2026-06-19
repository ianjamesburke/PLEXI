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
    WasmBenchmarks,
}

impl ReleaseFeature {
    pub fn name(self) -> &'static str {
        match self {
            Self::Assistant => "assistant",
            Self::AppWrappers => "app wrappers",
            Self::Marketplace => "marketplace",
            Self::WasmBenchmarks => "WASM benchmarks",
        }
    }

    pub fn minimum_tier(self) -> ReleaseTier {
        match self {
            Self::Assistant | Self::AppWrappers | Self::Marketplace => ReleaseTier::Beta,
            Self::WasmBenchmarks => ReleaseTier::Alpha,
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
    format!(
        "{} requires the {} channel and is not part of the stable v1 surface. Use {} to try it.",
        feature.name(),
        feature.minimum_tier().label(),
        feature.minimum_tier().binary_hint()
    )
}

fn feature_enabled_for_channel(feature: ReleaseFeature, channel: Option<&str>) -> bool {
    ReleaseTier::for_channel(channel).is_some_and(|tier| tier >= feature.minimum_tier())
}

#[cfg(test)]
mod tests {
    use super::{feature_enabled_for_channel, ReleaseFeature, ReleaseTier};

    #[test]
    fn stable_and_rc_channels_disable_beta_features() {
        assert!(!feature_enabled_for_channel(
            ReleaseFeature::Assistant,
            None
        ));
        assert!(!feature_enabled_for_channel(
            ReleaseFeature::Assistant,
            Some("main")
        ));
        assert!(!feature_enabled_for_channel(
            ReleaseFeature::Assistant,
            Some("rc-010")
        ));
    }

    #[test]
    fn alpha_beta_and_pr_channels_enable_beta_features() {
        assert!(feature_enabled_for_channel(
            ReleaseFeature::Assistant,
            Some("alpha")
        ));
        assert!(feature_enabled_for_channel(
            ReleaseFeature::Assistant,
            Some("beta")
        ));
        assert!(feature_enabled_for_channel(
            ReleaseFeature::Assistant,
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

    #[test]
    fn wasm_benchmarks_are_alpha_only() {
        assert!(!feature_enabled_for_channel(
            ReleaseFeature::WasmBenchmarks,
            Some("beta")
        ));
        assert!(feature_enabled_for_channel(
            ReleaseFeature::WasmBenchmarks,
            Some("alpha")
        ));
        assert!(feature_enabled_for_channel(
            ReleaseFeature::WasmBenchmarks,
            Some("pr-2299")
        ));
    }
}
