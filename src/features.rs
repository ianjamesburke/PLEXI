use std::collections::HashSet;

use crate::config::PlexiConfig;

/// Runtime feature flags for visual effects.
/// Configured via `[beta]` section in ~/.plexi/config.toml.
#[derive(Clone, Debug)]
pub struct FeatureFlags {
    enabled: HashSet<String>,
    pub zoom_opacity: f32,
}

impl Default for FeatureFlags {
    fn default() -> Self {
        Self {
            enabled: HashSet::new(),
            zoom_opacity: 0.98,
        }
    }
}

impl FeatureFlags {
    pub fn from_config(config: &PlexiConfig) -> Self {
        let mut enabled = HashSet::new();
        let mut zoom_opacity = 0.98_f32;
        if let Some(beta) = &config.beta {
            if beta.crt.unwrap_or(false) {
                enabled.insert("crt".to_string());
            }
            if beta.ghost.unwrap_or(false) {
                enabled.insert("ghost".to_string());
            }
            if let Some(v) = beta.zoom_opacity {
                zoom_opacity = v.clamp(0.0, 1.0);
            }
        }
        if !enabled.is_empty() {
            log::info!("Feature flags enabled: {:?}", enabled);
        }
        Self { enabled, zoom_opacity }
    }

    pub fn is_enabled(&self, flag: &str) -> bool {
        self.enabled.contains(flag)
    }
}
