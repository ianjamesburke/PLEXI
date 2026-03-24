use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize, Default)]
pub struct PlexiConfig {
    pub theme: Option<ThemeConfig>,
}

#[derive(Deserialize, Default)]
pub struct ThemeConfig {
    // UI chrome
    pub bg_darkest: Option<String>,
    pub bg_sidebar: Option<String>,
    pub bg_toolbar: Option<String>,
    pub terminal_bg: Option<String>,
    pub bg_hover: Option<String>,
    pub bg_active: Option<String>,
    pub text_primary: Option<String>,
    pub text_dim: Option<String>,
    pub text_section: Option<String>,
    pub accent: Option<String>,
    pub border: Option<String>,
    // Terminal ANSI palette
    pub foreground: Option<String>,
    pub background: Option<String>,
    pub black: Option<String>,
    pub red: Option<String>,
    pub green: Option<String>,
    pub yellow: Option<String>,
    pub blue: Option<String>,
    pub magenta: Option<String>,
    pub cyan: Option<String>,
    pub white: Option<String>,
    pub bright_black: Option<String>,
    pub bright_red: Option<String>,
    pub bright_green: Option<String>,
    pub bright_yellow: Option<String>,
    pub bright_blue: Option<String>,
    pub bright_magenta: Option<String>,
    pub bright_cyan: Option<String>,
    pub bright_white: Option<String>,
    pub bright_foreground: Option<String>,
}

fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("plexi")
        .join("config.toml")
}

impl PlexiConfig {
    pub fn load() -> Self {
        let path = config_path();
        let data = match std::fs::read_to_string(&path) {
            Ok(d) => d,
            Err(_) => return Self::default(),
        };
        match toml::from_str(&data) {
            Ok(cfg) => cfg,
            Err(e) => {
                log::warn!("Failed to parse config file: {e}");
                Self::default()
            }
        }
    }
}
