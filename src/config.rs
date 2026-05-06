use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Deserialize, Default, Clone)]
pub struct PlexiConfig {
    pub font_size: Option<f32>,
    pub theme_preset: Option<String>,
    pub theme: Option<ThemeConfig>,
    pub beta: Option<BetaConfig>,
    pub log: Option<LogConfig>,
    pub notifications: Option<NotificationsConfig>,
    pub ai: Option<AiConfig>,
    /// Set to false to quit immediately on Cmd+Q without triple-press confirmation (default: true).
    pub confirm_quit: Option<bool>,
    /// Set to false to close panes immediately on Cmd+W without a confirmation dialog (default: true).
    pub confirm_close: Option<bool>,
}

/// Plexi AI broker configuration (`ai.query` capability).
///
/// `backend` selects the provider: `"openrouter"` (default) or `"ollama"`.
/// API keys are NOT stored here — export `OPENROUTER_API_KEY` in your shell
/// profile (`~/.zshrc`, `~/.zprofile`, etc.). Never store API keys in
/// plaintext config files.
#[derive(Deserialize, Default, Clone)]
pub struct AiConfig {
    /// Backend selection: `"openrouter"` (default) or `"ollama"`.
    pub backend: Option<String>,
    pub openrouter: Option<OpenRouterBackendConfig>,
    pub ollama: Option<OllamaBackendConfig>,
}

/// OpenRouter backend configuration.
#[derive(Deserialize, Default, Clone)]
pub struct OpenRouterBackendConfig {
    /// Environment variable name for the API key. Default: `OPENROUTER_API_KEY`.
    pub api_key_env: Option<String>,
    /// Low-tier model. e.g. "google/gemini-2.0-flash-001"
    pub model_low: Option<String>,
    /// Medium-tier model. e.g. "anthropic/claude-sonnet-4-6"
    pub model_medium: Option<String>,
    /// High-tier model. e.g. "anthropic/claude-opus-4-7"
    pub model_high: Option<String>,
}

/// Ollama backend configuration.
#[derive(Deserialize, Default, Clone)]
pub struct OllamaBackendConfig {
    /// Ollama host URL. Default: `http://localhost:11434`.
    pub host: Option<String>,
    /// Low-tier model. e.g. "llama3.2:3b"
    pub model_low: Option<String>,
    /// Medium-tier model. e.g. "llama3.3:70b"
    pub model_medium: Option<String>,
    /// High-tier model. e.g. "qwq:32b"
    pub model_high: Option<String>,
}

impl AiConfig {
    /// Overlay `other` on top of `self` — any `Some` field in `other` wins.
    pub fn overlay(&mut self, other: Self) {
        if other.backend.is_some() {
            self.backend = other.backend;
        }
        match (self.openrouter.as_mut(), other.openrouter) {
            (Some(existing), Some(incoming)) => existing.overlay(incoming),
            (None, Some(incoming)) => self.openrouter = Some(incoming),
            _ => {}
        }
        match (self.ollama.as_mut(), other.ollama) {
            (Some(existing), Some(incoming)) => existing.overlay(incoming),
            (None, Some(incoming)) => self.ollama = Some(incoming),
            _ => {}
        }
    }
}

impl OpenRouterBackendConfig {
    fn overlay(&mut self, other: Self) {
        if other.api_key_env.is_some() {
            self.api_key_env = other.api_key_env;
        }
        if other.model_low.is_some() {
            self.model_low = other.model_low;
        }
        if other.model_medium.is_some() {
            self.model_medium = other.model_medium;
        }
        if other.model_high.is_some() {
            self.model_high = other.model_high;
        }
    }
}

impl OllamaBackendConfig {
    fn overlay(&mut self, other: Self) {
        if other.host.is_some() {
            self.host = other.host;
        }
        if other.model_low.is_some() {
            self.model_low = other.model_low;
        }
        if other.model_medium.is_some() {
            self.model_medium = other.model_medium;
        }
        if other.model_high.is_some() {
            self.model_high = other.model_high;
        }
    }
}

#[derive(Deserialize, Default, Clone)]
pub struct NotificationsConfig {
    /// Master switch. If false, incoming notifications are silently dropped —
    /// apps still send them, but the modal never appears and the queue stays
    /// empty. Defaults to true.
    pub enabled: Option<bool>,
    /// Focus mode. When true, NO notification auto-surfaces regardless of
    /// priority. Everything queues silently; the user reviews via Cmd+Shift+A.
    /// Defaults to false.
    pub focus_mode: Option<bool>,
    /// Minimum priority that may auto-open the modal. Notifications below
    /// this value queue silently (badge ticks, Cmd+Shift+A reveals them).
    /// At or above it, arrival auto-opens the modal. Defaults to 100
    /// (`PRIORITY_HIGH`) — NORMAL and LOW are passive; HIGH and CRITICAL
    /// interrupt. Set to 0 to auto-open everything; set to 201 to match
    /// `focus_mode = true`.
    pub interrupt_threshold: Option<u32>,
}

#[derive(Deserialize, Default, Clone)]
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

#[derive(Deserialize, Default, Clone)]
pub struct BetaConfig {
    pub crt: Option<bool>,
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
    eprintln!(
        "profile init: seeded {} with {} apps",
        dir.display(),
        std::fs::read_dir(&apps_dir).map(|r| r.count()).unwrap_or(0)
    );

    // Embed the SDK so Python apps work on first launch without relying on the
    // bundle resource path, which can be absent on a cache-hit CI build.
    let sdk_dest = dir.join("sdk").join("plexi_sdk");
    if !sdk_dest.exists() {
        if let Err(e) = std::fs::create_dir_all(&sdk_dest) {
            eprintln!("profile init: failed to create sdk dir: {e}");
        } else {
            let embedded_sdk =
                include_dir::include_dir!("$CARGO_MANIFEST_DIR/sdk/python/plexi_sdk");
            if let Err(e) = embedded_sdk.extract(&sdk_dest) {
                eprintln!("profile init: failed to seed SDK: {e}");
            } else {
                log::info!("profile init: seeded SDK to {}", sdk_dest.display());
            }
        }
    }
}

/// Returns the config directory name.
/// Returns the build channel for this binary: `alpha`, `beta`, `pr-<N>`, `v3`, or `None` (stable).
pub fn build_channel() -> Option<String> {
    let binary = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))?;
    let name = binary.as_str();
    if name.contains("alpha") {
        Some("alpha".into())
    } else if name.contains("beta") {
        Some("beta".into())
    } else if name.contains("pr-") {
        Some(name.trim_start_matches("plexi-").to_string())
    } else if name.contains("v3") {
        Some("v3".into())
    } else {
        None
    }
}

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
        Some(name) if name.contains("pr-") => {
            let suffix = name.trim_start_matches("plexi-");
            format!(".plexi-{suffix}")
        }
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

# ── Theme ──────────────────────────────────────────────────────
# Pick a preset OR customize individual colors below.
# Presets: catppuccin-mocha, dracula, tokyo-night, gruvbox-dark, nord, solarized-dark
theme_preset = "catppuccin-mocha"

# ── Confirmation Dialogs ───────────────────────────────────────
# confirm_quit requires triple Cmd+Q to exit (safer than a single press).
# confirm_close shows a dialog before Cmd+W closes a pane.
confirm_quit  = true
confirm_close = false

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

# Focus mode. When true, NO notification auto-surfaces regardless of
# priority. Everything queues silently; open Cmd+Shift+A to review.
focus_mode = false

# Minimum priority that may auto-open the modal. Notifications below
# this value queue silently (badge ticks on the toolbar, Cmd+Shift+A
# reveals them). At or above it, arrival auto-opens the modal.
#
# Tiers (from plexi_sdk):
#   0   = PRIORITY_LOW       (background info)
#   50  = PRIORITY_NORMAL    (standard confirmations — "note saved")
#   100 = PRIORITY_HIGH      (needs attention soon)
#   200 = PRIORITY_CRITICAL  (interrupt-level)
#
# Default = 100: NORMAL and LOW queue silently, HIGH and CRITICAL
# interrupt. Set to 0 to auto-open everything. Set to 201 to match
# focus_mode = true (nothing auto-opens).
interrupt_threshold = 100

# Esc vs Enter on the modal:
#   Enter (or option-select / input-submit) = acknowledge. Notification
#     is removed from the queue and the app receives NotifyAction.
#   Esc = defer. Modal closes but the notification stays in the queue —
#     open Cmd+Shift+A later to come back to it. No NotifyAction dispatched.
#   Required notifications (required = true) cannot be Esc'd.

[theme]
# Uncomment any color below to override the preset value.
# accent = "#89b4fa"
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

# ── Plexi AI — brokered LLM calls (`ai.query` capability) ─────
# Apps that declare `ai.query` in their manifest can call tier-routed
# LLM models through the host broker. Two backends are supported:
# "openrouter" (default, cloud) and "ollama" (local).
#
# OpenRouter (default):
#   Export your key in ~/.zprofile or ~/.zshrc:
#     export OPENROUTER_API_KEY="sk-or-..."
#
[ai]
backend = "openrouter"

[ai.openrouter]
api_key_env  = "OPENROUTER_API_KEY"
model_low    = "google/gemini-2.0-flash-001"
model_medium = "anthropic/claude-sonnet-4-6"
model_high   = "anthropic/claude-opus-4-7"

# Ollama (local):
# [ai]
# backend = "ollama"
#
# [ai.ollama]
# host         = "http://localhost:11434"
# model_low    = "llama3.2:3b"
# model_medium = "llama3.3:70b"
# model_high   = "qwq:32b"

# ── Experimental Features ──────────────────────────────────────
# Flip any flag to true and restart to enable.
[beta]
# crt   = false    # Retro CRT scanlines + green phosphor tint
# ghost = false    # Unfocused panes render at reduced opacity

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

    if let Err(e) = std::process::Command::new("open").arg(&path).status() {
        log::error!("open_config_file: failed to open {}: {e}", path.display());
    }
}

impl PlexiConfig {
    /// Load the global config only — no project-level merge. Most call sites
    /// should prefer [`load_with_workspace`] so a workspace's
    /// `.plexi/config.toml` can override.
    pub fn load() -> Self {
        let path = config_path();
        if !path.exists() {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            match std::fs::write(&path, CONFIG_TEMPLATE) {
                Ok(()) => log::info!("config: created default config at {path:?}"),
                Err(e) => log::warn!("config: could not write default config to {path:?}: {e}"),
            }
        }
        Self::load_from_path(&path).unwrap_or_default()
    }

    /// Load `path` as a `PlexiConfig`. Returns `None` if the file is absent;
    /// returns `Some(default)` after logging if the file exists but fails to
    /// parse — matches the historical behavior of `load()`.
    fn load_from_path(path: &Path) -> Option<Self> {
        let data = match std::fs::read_to_string(path) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
            Err(e) => {
                log::warn!("Failed to read config file {}: {e}", path.display());
                return None;
            }
        };
        match toml::from_str::<Self>(&data) {
            Ok(cfg) => Some(cfg),
            Err(e) => {
                log::warn!("Failed to parse config file {}: {e}", path.display());
                Some(Self::default())
            }
        }
    }

    /// Load the global config and overlay `<workspace_root>/.plexi/config.toml`
    /// on top if it exists. Project-level values override globals on a
    /// per-field basis; unset project fields preserve the global value.
    pub fn load_with_workspace(workspace_root: Option<&Path>) -> Self {
        let mut merged = Self::load();
        let Some(root) = workspace_root else {
            return merged;
        };
        let project_path = root.join(".plexi").join("config.toml");
        if let Some(project) = Self::load_from_path(&project_path) {
            merged.overlay(project);
        }
        merged
    }

    /// Field-level overlay of `other` on top of `self`. Any `Some(_)` value in
    /// `other` replaces the corresponding field in `self`. Nested structs
    /// (theme, beta, log, notifications) are overlaid recursively.
    fn overlay(&mut self, other: Self) {
        if other.font_size.is_some() {
            self.font_size = other.font_size;
        }
        if other.theme_preset.is_some() {
            self.theme_preset = other.theme_preset;
        }
        if other.confirm_quit.is_some() {
            self.confirm_quit = other.confirm_quit;
        }
        if other.confirm_close.is_some() {
            self.confirm_close = other.confirm_close;
        }
        match (self.theme.as_mut(), other.theme) {
            (Some(existing), Some(incoming)) => existing.overlay(incoming),
            (None, Some(incoming)) => self.theme = Some(incoming),
            _ => {}
        }
        match (self.beta.as_mut(), other.beta) {
            (Some(existing), Some(incoming)) => existing.overlay(incoming),
            (None, Some(incoming)) => self.beta = Some(incoming),
            _ => {}
        }
        match (self.log.as_mut(), other.log) {
            (Some(existing), Some(incoming)) => existing.overlay(incoming),
            (None, Some(incoming)) => self.log = Some(incoming),
            _ => {}
        }
        match (self.notifications.as_mut(), other.notifications) {
            (Some(existing), Some(incoming)) => existing.overlay(incoming),
            (None, Some(incoming)) => self.notifications = Some(incoming),
            _ => {}
        }
        match (self.ai.as_mut(), other.ai) {
            (Some(existing), Some(incoming)) => existing.overlay(incoming),
            (None, Some(incoming)) => self.ai = Some(incoming),
            _ => {}
        }
    }
}

impl ThemeConfig {
    fn overlay(&mut self, other: Self) {
        macro_rules! overlay_field {
            ($field:ident) => {
                if other.$field.is_some() {
                    self.$field = other.$field;
                }
            };
        }
        overlay_field!(bg_darkest);
        overlay_field!(bg_sidebar);
        overlay_field!(bg_toolbar);
        overlay_field!(terminal_bg);
        overlay_field!(bg_hover);
        overlay_field!(bg_active);
        overlay_field!(text_primary);
        overlay_field!(text_dim);
        overlay_field!(text_section);
        overlay_field!(accent);
        overlay_field!(border);
        overlay_field!(foreground);
        overlay_field!(background);
        overlay_field!(black);
        overlay_field!(red);
        overlay_field!(green);
        overlay_field!(yellow);
        overlay_field!(blue);
        overlay_field!(magenta);
        overlay_field!(cyan);
        overlay_field!(white);
        overlay_field!(bright_black);
        overlay_field!(bright_red);
        overlay_field!(bright_green);
        overlay_field!(bright_yellow);
        overlay_field!(bright_blue);
        overlay_field!(bright_magenta);
        overlay_field!(bright_cyan);
        overlay_field!(bright_white);
        overlay_field!(bright_foreground);
    }
}

impl BetaConfig {
    fn overlay(&mut self, other: Self) {
        if other.crt.is_some() {
            self.crt = other.crt;
        }
        if other.ghost.is_some() {
            self.ghost = other.ghost;
        }
    }
}

impl LogConfig {
    fn overlay(&mut self, other: Self) {
        if other.level.is_some() {
            self.level = other.level;
        }
    }
}

impl NotificationsConfig {
    fn overlay(&mut self, other: Self) {
        if other.enabled.is_some() {
            self.enabled = other.enabled;
        }
        if other.focus_mode.is_some() {
            self.focus_mode = other.focus_mode;
        }
        if other.interrupt_threshold.is_some() {
            self.interrupt_threshold = other.interrupt_threshold;
        }
    }
}

// ── Adopted workspace root (set once by main when an explicit path arg is
// given) ─────────────────────────────────────────────────────────────────────

static ADOPTED_WORKSPACE_ROOT: OnceLock<Option<PathBuf>> = OnceLock::new();

/// Set the explicit workspace root adopted from a `plexi <path>` arg. Called
/// once from `main()` after CLI parsing. Subsequent calls are silently ignored
/// — the binary commits to one workspace per process.
pub fn set_adopted_workspace_root(root: Option<PathBuf>) {
    let _ = ADOPTED_WORKSPACE_ROOT.set(root);
}

/// Return the workspace root adopted via `plexi <path>` (the "open folder"
/// arg). When unset, callers should fall back to walking up from CWD via
/// [`crate::app_registry::resolve_workspace_root`].
pub fn adopted_workspace_root() -> Option<PathBuf> {
    ADOPTED_WORKSPACE_ROOT.get().and_then(|opt| opt.clone())
}

/// Convenience: the active workspace root for this process. Returns the
/// adopted root if set, otherwise walks up from the current working
/// directory looking for a `.plexi/` ancestor.
pub fn active_workspace_root() -> Option<PathBuf> {
    if let Some(adopted) = adopted_workspace_root() {
        return Some(adopted);
    }
    let cwd = std::env::current_dir().ok()?;
    crate::app_registry::resolve_workspace_root(&cwd)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn project_config_overrides_global() {
        let global_dir = tempfile::tempdir().unwrap();
        let global_path = global_dir.path().join("config.toml");
        write(
            &global_path,
            "font_size = 14.0\n[log]\nlevel = \"info\"\n[theme]\naccent = \"#aaaaaa\"\n",
        );

        let workspace = tempfile::tempdir().unwrap();
        let project_path = workspace.path().join(".plexi").join("config.toml");
        write(
            &project_path,
            "font_size = 18.0\n[log]\nlevel = \"debug\"\n",
        );

        let mut merged = PlexiConfig::load_from_path(&global_path).unwrap_or_default();
        if let Some(project) = PlexiConfig::load_from_path(&project_path) {
            merged.overlay(project);
        }
        assert_eq!(merged.font_size, Some(18.0));
        assert_eq!(
            merged.log.as_ref().and_then(|l| l.level.clone()),
            Some("debug".to_string())
        );
        // Theme accent untouched by project — global value must survive.
        assert_eq!(
            merged.theme.as_ref().and_then(|t| t.accent.clone()),
            Some("#aaaaaa".to_string())
        );
    }

    #[test]
    fn missing_project_config_keeps_global() {
        let global_dir = tempfile::tempdir().unwrap();
        let global_path = global_dir.path().join("config.toml");
        write(
            &global_path,
            "font_size = 12.0\n[log]\nlevel = \"warn\"\n[theme]\naccent = \"#bbbbbb\"\n",
        );

        // Workspace exists but has no .plexi/config.toml.
        let workspace = tempfile::tempdir().unwrap();
        let project_path = workspace.path().join(".plexi").join("config.toml");
        assert!(!project_path.exists());

        let mut merged = PlexiConfig::load_from_path(&global_path).unwrap_or_default();
        if let Some(project) = PlexiConfig::load_from_path(&project_path) {
            merged.overlay(project);
        }
        assert_eq!(merged.font_size, Some(12.0));
        assert_eq!(
            merged.log.as_ref().and_then(|l| l.level.clone()),
            Some("warn".to_string())
        );
        assert_eq!(
            merged.theme.as_ref().and_then(|t| t.accent.clone()),
            Some("#bbbbbb".to_string())
        );
    }

    #[test]
    fn project_partial_override_preserves_unset_global() {
        let global_dir = tempfile::tempdir().unwrap();
        let global_path = global_dir.path().join("config.toml");
        write(
            &global_path,
            "font_size = 14.0\nconfirm_close = true\n\
             [theme]\naccent = \"#cccccc\"\nbg_darkest = \"#000000\"\n\
             [log]\nlevel = \"info\"\n",
        );

        let workspace = tempfile::tempdir().unwrap();
        let project_path = workspace.path().join(".plexi").join("config.toml");
        // Only override [log] level. Everything else must remain global.
        write(&project_path, "[log]\nlevel = \"debug\"\n");

        let mut merged = PlexiConfig::load_from_path(&global_path).unwrap_or_default();
        if let Some(project) = PlexiConfig::load_from_path(&project_path) {
            merged.overlay(project);
        }
        // Project value wins.
        assert_eq!(
            merged.log.as_ref().and_then(|l| l.level.clone()),
            Some("debug".to_string())
        );
        // Globals preserved.
        assert_eq!(merged.font_size, Some(14.0));
        assert_eq!(merged.confirm_close, Some(true));
        assert_eq!(
            merged.theme.as_ref().and_then(|t| t.accent.clone()),
            Some("#cccccc".to_string())
        );
        assert_eq!(
            merged.theme.as_ref().and_then(|t| t.bg_darkest.clone()),
            Some("#000000".to_string())
        );
    }
}

