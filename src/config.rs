use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize, Default)]
pub struct PlexiConfig {
    pub font_size: Option<f32>,
    pub theme_preset: Option<String>,
    pub theme: Option<ThemeConfig>,
    pub beta: Option<BetaConfig>,
    pub log: Option<LogConfig>,
    pub notifications: Option<NotificationsConfig>,
    /// Set to false to quit immediately on Cmd+Q without triple-press confirmation (default: true).
    pub confirm_quit: Option<bool>,
    /// Set to false to close panes immediately on Cmd+W without a confirmation dialog (default: true).
    pub confirm_close: Option<bool>,
}

#[derive(Deserialize, Default, Clone)]
pub struct NotificationsConfig {
    /// Master switch. If false, incoming notifications are silently dropped —
    /// apps still send them, but the modal never appears and the queue stays
    /// empty. Defaults to true.
    pub enabled: Option<bool>,
    /// Focus mode. When true, notifications silently queue instead of auto-
    /// surfacing the modal. The user enters review mode with Cmd+Shift+D,
    /// which opens the modal on the front of the queue; Cmd+] and Cmd+[
    /// cycle forward and back through the queue without acknowledging.
    /// Defaults to false (auto-surface, the original behavior).
    pub focus_mode: Option<bool>,
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
            Some("warn") => Some(log::LevelFilter::Warn),
            Some("info") => Some(log::LevelFilter::Info),
            Some("debug") => Some(log::LevelFilter::Debug),
            _ => None,
        }
    }
}

#[derive(Deserialize, Default)]
pub struct BetaConfig {
    pub crt: Option<bool>,
    pub pulse: Option<bool>,
    pub ghost: Option<bool>,
    /// Set to false to disable triple-Cmd+Q confirmation (default: true).
    pub quit_confirm: Option<bool>,
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
    eprintln!(
        "profile init: seeded {} with {} apps",
        dir.display(),
        std::fs::read_dir(&apps_dir).map(|r| r.count()).unwrap_or(0)
    );
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

const CONFIG_TEMPLATE: &str = r##"# ╔══════════════════════════════════════════════════════════════╗
# ║  Plexi Configuration                                        ║
# ║  Changes take effect on next launch.                        ║
# ╚══════════════════════════════════════════════════════════════╝

font_size = 14.0

# ── Confirmation Dialogs ───────────────────────────────────────
# Set to false to disable the corresponding confirmation flow.
# confirm_quit  = false   # Triple Cmd+Q to quit (default: true)
# confirm_close = false   # Dialog before Cmd+W closes a pane (default: true)

# ── Notifications ──────────────────────────────────────────────
# The work-area modal is the one and only notification surface.
# Apps emit `ctx.notify(...)`, `ctx.notify_choice(...)`, or
# `ctx.notify_input(...)` and the modal renders each kind with
# keyboard-first navigation (Enter confirms, j/k or ↑↓ cycle
# options, 1-9 direct-select, Esc cancels when allowed).
[notifications]
# Master switch. If false, notifications are silently dropped at
# arrival — apps still send them, the modal never appears, and
# the queue stays empty.
enabled = true

# Focus mode. When true, arriving notifications queue silently
# instead of auto-opening the modal. The user opts into review
# with Cmd+Shift+A, which opens the modal on the front of the
# queue; Cmd+] / Cmd+[ cycle forward/back without acknowledging.
# When false, notifications auto-surface as they arrive.
focus_mode = false

# ── Theme ──────────────────────────────────────────────────────
# Pick a preset OR customize individual colors below.
# Presets: catppuccin-mocha, dracula, tokyo-night, gruvbox-dark, nord, solarized-dark
# theme_preset = "catppuccin-mocha"

[theme]
# UI chrome colors (hex format)
accent = "#89b4fa"
# bg_darkest = "#11111b"      # Deepest background (window edges)
# bg_sidebar = "#181825"      # Sidebar background
# bg_toolbar = "#181825"      # Toolbar/status bar background
# terminal_bg = "#292a44"     # Terminal pane background
# bg_hover = "#2a2a3c"        # Hover highlight
# bg_active = "#313144"       # Active/selected item
# text_primary = "#cdd6f4"    # Main text color
# text_dim = "#6c7086"        # Dimmed/secondary text
# text_section = "#585b70"    # Section headers
# border = "#2a2a3c"          # Pane borders

# Terminal ANSI colors (override the palette)
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

# ── Experimental Features ──────────────────────────────────────
# Flip any flag to true and restart to enable.
[beta]
# crt   = false    # Retro CRT scanlines + green phosphor tint
# pulse = false    # Focused pane border gently breathes
# ghost = false    # Unfocused panes render at reduced opacity
# quit_confirm = false   # Deprecated; prefer top-level `confirm_quit`

# ── Logging ────────────────────────────────────────────────────
# [log]
# level = "info"   # error | warn | info | debug  (default: info)
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

