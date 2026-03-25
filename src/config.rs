use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize, Default)]
pub struct PlexiConfig {
    pub font_size: Option<f32>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config() {
        let cfg = PlexiConfig::default();
        assert!(cfg.theme.is_none());
        assert!(cfg.font_size.is_none());
    }

    #[test]
    fn parse_empty_toml() {
        let cfg: PlexiConfig = toml::from_str("").unwrap();
        assert!(cfg.theme.is_none());
        assert!(cfg.font_size.is_none());
    }

    #[test]
    fn parse_font_size() {
        let cfg: PlexiConfig = toml::from_str("font_size = 16.0").unwrap();
        assert_eq!(cfg.font_size, Some(16.0));
    }

    #[test]
    fn parse_theme_colors() {
        let toml_str = r##"
[theme]
accent = "#ff0000"
bg_darkest = "#000000"
"##;
        let cfg: PlexiConfig = toml::from_str(toml_str).unwrap();
        let theme = cfg.theme.unwrap();
        assert_eq!(theme.accent, Some("#ff0000".into()));
        assert_eq!(theme.bg_darkest, Some("#000000".into()));
        assert!(theme.bg_sidebar.is_none());
    }

    #[test]
    fn parse_terminal_palette() {
        let toml_str = r##"
[theme]
foreground = "#e8e6ed"
black = "#12131e"
"##;
        let cfg: PlexiConfig = toml::from_str(toml_str).unwrap();
        let theme = cfg.theme.unwrap();
        assert_eq!(theme.foreground, Some("#e8e6ed".into()));
        assert_eq!(theme.black, Some("#12131e".into()));
    }
}
