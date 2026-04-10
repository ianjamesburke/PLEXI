use std::collections::HashSet;

use crate::config::PlexiConfig;

/// Runtime feature flags for visual effects.
/// Configured via `[beta]` section in ~/.plexi/config.toml.
#[derive(Clone, Debug, Default)]
pub struct FeatureFlags {
    enabled: HashSet<String>,
}

impl FeatureFlags {
    pub fn from_config(config: &PlexiConfig) -> Self {
        let mut enabled = HashSet::new();
        if let Some(beta) = &config.beta {
            if beta.crt.unwrap_or(false) {
                enabled.insert("crt".to_string());
            }
            if beta.pulse.unwrap_or(false) {
                enabled.insert("pulse".to_string());
            }
            if beta.ghost.unwrap_or(false) {
                enabled.insert("ghost".to_string());
            }
        }
        if !enabled.is_empty() {
            log::info!("Feature flags enabled: {:?}", enabled);
        }
        Self { enabled }
    }

    pub fn is_enabled(&self, flag: &str) -> bool {
        self.enabled.contains(flag)
    }
}
