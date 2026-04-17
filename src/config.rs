use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize, Default)]
pub struct PlexiConfig {
    pub font_size: Option<f32>,
    pub theme_preset: Option<String>,
    pub theme: Option<ThemeConfig>,
    pub beta: Option<BetaConfig>,
    pub log: Option<LogConfig>,
}

#[derive(Deserialize, Default)]
pub struct LogConfig {
    pub level: Option<String>,
}

impl LogConfig {
    /// Convert the `level` string to a `log::LevelFilter`.
    /// Returns `None` if unset; invalid values are ignored (returns `None`).
    pub fn level_filter(&self) -> Option<log::LevelFilter> {
        match self.level.as_deref() {
            Some("error") => Some(log::LevelFilter::Error),
            Some("warn")  => Some(log::LevelFilter::Warn),
            Some("info")  => Some(log::LevelFilter::Info),
            Some("debug") => Some(log::LevelFilter::Debug),
            _             => None,
        }
    }
}

#[derive(Deserialize, Default)]
pub struct BetaConfig {
    pub crt: Option<bool>,
    pub pulse: Option<bool>,
    pub ghost: Option<bool>,
}

#[derive(Deserialize, Default, Clone)]
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

use std::sync::OnceLock;

static PROFILE_OVERRIDE: OnceLock<Option<String>> = OnceLock::new();

/// Set the active profile. Called once from main() after CLI parsing.
/// `None` or `Some("default")` → fall through to binary-name detection.
/// `Some(name)` → use `.plexi-<name>` as the config dir.
pub fn set_profile(name: Option<String>) {
    let normalized = match name.as_deref() {
        None | Some("") | Some("default") => None,
        Some(_) => name,
    };
    let _ = PROFILE_OVERRIDE.set(normalized);
}

/// If a profile is set and its directory doesn't exist yet, create it and
/// seed `apps/` from the example apps embedded at compile time.
pub fn ensure_profile_initialized() {
    let dir = config_dir();
    if dir.exists() {
        return;
    }
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("profile init: failed to create {}: {e}", dir.display());
        return;
    }
    let apps_dir = dir.join("apps");
    if let Err(e) = std::fs::create_dir_all(&apps_dir) {
        eprintln!("profile init: failed to create apps dir: {e}");
        return;
    }
    let embedded = include_dir::include_dir!("$CARGO_MANIFEST_DIR/examples");
    if let Err(e) = embedded.extract(&apps_dir) {
        eprintln!("profile init: failed to seed apps from bundle: {e}");
        return;
    }
    // chmod +x on all .py entries.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(entries) = std::fs::read_dir(&apps_dir) {
            for app_dir in entries.flatten().filter(|e| e.path().is_dir()) {
                if let Ok(files) = std::fs::read_dir(app_dir.path()) {
                    for f in files.flatten() {
                        let p = f.path();
                        if p.extension().and_then(|x| x.to_str()) == Some("py") {
                            if let Ok(meta) = std::fs::metadata(&p) {
                                let mut perms = meta.permissions();
                                perms.set_mode(perms.mode() | 0o111);
                                let _ = std::fs::set_permissions(&p, perms);
                            }
                        }
                    }
                }
            }
        }
    }
    eprintln!("profile init: seeded {} with {} apps", dir.display(),
              std::fs::read_dir(&apps_dir).map(|r| r.count()).unwrap_or(0));
}

/// Returns the config directory name.
/// Priority: `--profile <name>` CLI flag → binary-name detection → `.plexi`.
fn config_dir_name() -> String {
    if let Some(Some(profile)) = PROFILE_OVERRIDE.get() {
        return format!(".plexi-{profile}");
    }
    let binary = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
    match binary.as_deref() {
        Some(name) if name.contains("alpha") => ".plexi-alpha".to_string(),
        Some(name) if name.contains("beta") => ".plexi-beta".to_string(),
        Some(name) if name.contains("v3") => ".plexi-v3".to_string(),
        _ => ".plexi".to_string(),
    }
}

pub fn config_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(config_dir_name())
        .join("config.toml")
}

pub fn config_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(config_dir_name())
}

const CONFIG_TEMPLATE: &str = r##"# Plexi Configuration
# Changes take effect on next launch.

font_size = 14.0

# Theme preset — uncomment one to use it as a base.
# Individual [theme] values below will override the preset.
# Options: catppuccin-mocha, dracula, tokyo-night, gruvbox-dark, nord, solarized-dark
# theme_preset = "catppuccin-mocha"

[theme]
# UI chrome
# bg_darkest = "#11111b"
# bg_sidebar = "#181825"
# bg_toolbar = "#181825"
# terminal_bg = "#292a44"
# bg_hover = "#2a2a3c"
# bg_active = "#313144"
# text_primary = "#cdd6f4"
# text_dim = "#6c7086"
# text_section = "#585b70"
accent = "#89b4fa"
# border = "#2a2a3c"

# Terminal ANSI palette
# foreground = "#e8e6ed"
# background = "#292a44"
# black = "#12131e"
# red = "#dd7755"
# green = "#04dbb5"
# yellow = "#f2e7b7"
# blue = "#7aa5ff"
# magenta = "#bf9cf9"
# cyan = "#56d3c2"
# white = "#e4e3e9"
# bright_black = "#666699"
# bright_red = "#ff92cd"
# bright_green = "#01eac0"
# bright_yellow = "#fffca8"
# bright_blue = "#69c0fa"
# bright_magenta = "#c17ff8"
# bright_cyan = "#8bfde1"
# bright_white = "#f4f2f9"
# bright_foreground = "#f4f2f9"

[beta]
# Experimental visual effects. Set to true to enable.
# crt = false     # Retro CRT scanlines + green phosphor tint
# pulse = false   # Focused pane border gently breathes
# ghost = false   # Unfocused panes render at reduced opacity

# [log]
# level = "info"  # error | warn | info | debug  (default: info)
"##;

pub fn open_config_file() {
    let path = config_path();

    if !path.exists() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&path, CONFIG_TEMPLATE);
    }

    let _ = std::process::Command::new("open").arg(&path).spawn();
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
