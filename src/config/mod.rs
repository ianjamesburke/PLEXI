pub mod watcher;

use serde::Deserialize;
use std::path::{Path, PathBuf};

// ── Config validation (#1116) ───────────────────────────────────────────────

#[derive(Debug)]
pub enum ConfigDiagnostic {
    ReadError {
        path: String,
        error: String,
    },
    ParseError {
        path: String,
        error: String,
    },
    UnknownKey {
        path: String,
        table: String,
        key: String,
    },
}

impl ConfigDiagnostic {
    pub fn is_error(&self) -> bool {
        matches!(self, Self::ReadError { .. } | Self::ParseError { .. })
    }
}

impl std::fmt::Display for ConfigDiagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ReadError { path, error } => write!(f, "{path}: read error: {error}"),
            Self::ParseError { path, error } => write!(f, "{path}: {error}"),
            Self::UnknownKey { path, table, key } => {
                if table.is_empty() {
                    write!(f, "{path}: unknown key `{key}`")
                } else {
                    write!(f, "{path}: unknown key `{key}` in [{table}]")
                }
            }
        }
    }
}

const KNOWN_TOP_LEVEL: &[&str] = &[
    "config_version",
    "font_size",
    "theme_preset",
    "theme",
    "beta",
    "log",
    "notifications",
    "ai",
    "confirm_quit",
    "confirm_close",
    "keybindings",
    "quick_note",
    "focus_history_depth",
    "agents",
    "cli",
    "pane_gap",
    "pane_title_font_size",
];
const KNOWN_AGENTS: &[&str] = &["low", "medium", "high"];
const KNOWN_CLI: &[&str] = &["tips"];
const KNOWN_THEME: &[&str] = &[
    "bg_darkest",
    "bg_sidebar",
    "bg_toolbar",
    "terminal_bg",
    "bg_hover",
    "bg_sidebar_hover",
    "bg_active",
    "text_primary",
    "text_dim",
    "text_section",
    "accent",
    "border",
    "foreground",
    "background",
    "black",
    "red",
    "green",
    "yellow",
    "blue",
    "magenta",
    "cyan",
    "white",
    "bright_black",
    "bright_red",
    "bright_green",
    "bright_yellow",
    "bright_blue",
    "bright_magenta",
    "bright_cyan",
    "bright_white",
    "bright_foreground",
];
const KNOWN_BETA: &[&str] = &["crt", "ghost", "ghost_opacity", "osc_pane_title"];
const KNOWN_LOG: &[&str] = &["level", "retention_days"];
const KNOWN_NOTIFICATIONS: &[&str] = &["enabled", "focus_mode", "interrupt_threshold"];
const KNOWN_AI: &[&str] = &[
    "backend",
    "openrouter",
    "ollama",
    "per_app_daily_usd",
    "global_daily_usd",
];
const KNOWN_AI_OPENROUTER: &[&str] = &["api_key_env", "model_low", "model_medium", "model_high"];
const KNOWN_AI_OLLAMA: &[&str] = &["host", "model_low", "model_medium", "model_high"];
const KNOWN_KEYBINDINGS: &[&str] = &[
    "quit",
    "close_pane",
    "toggle_command_palette",
    "split_horizontal",
    "split_vertical",
    "split_right",
    "split_down",
    "swap_pane_left",
    "swap_pane_down",
    "swap_pane_up",
    "swap_pane_right",
    "send_pane_left",
    "send_pane_down",
    "send_pane_up",
    "send_pane_right",
    "navigate_left",
    "navigate_down",
    "navigate_up",
    "navigate_right",
    "new_tab",
    "next_tab",
    "prev_tab",
    "first_tab",
    "last_tab",
    "nav_back",
    "focus_history_forward",
    "toggle_sidebar",
    "toggle_zoom",
    "toggle_shortcuts",
    "rename_context",
    "rename_pane",
    "new_context",
    "new_page_right",
    "toggle_minimap",
    "scroll_up",
    "scroll_down",
    "increase_font_size",
    "decrease_font_size",
    "open_file_browser",
    "open_quick_note",
    "open_config",
    "reload_config",
    "open_secrets_manager",
    "force_reload_app",
    "toggle_notification_modal",
    "open_scratchpad",
    "set_context_root_from_cwd",
    "push_to_subcontext",
    "new_child_context",
    "hide_pane",
    "park_context",
    "open_notes_picker",
];

pub fn validate_from_path(path: &Path) -> Vec<ConfigDiagnostic> {
    let mut diags = Vec::new();
    let path_str = path.display().to_string();

    let data = match std::fs::read_to_string(path) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return diags,
        Err(e) => {
            diags.push(ConfigDiagnostic::ReadError {
                path: path_str,
                error: e.to_string(),
            });
            return diags;
        }
    };

    if let Err(e) = toml::from_str::<PlexiConfig>(&data) {
        diags.push(ConfigDiagnostic::ParseError {
            path: path_str,
            error: e.to_string(),
        });
        return diags;
    }

    if let Ok(value) = toml::from_str::<toml::Value>(&data) {
        if let Some(table) = value.as_table() {
            check_unknown_keys(table, "", KNOWN_TOP_LEVEL, &path_str, &mut diags);

            if let Some(toml::Value::Table(t)) = table.get("theme") {
                check_unknown_keys(t, "theme", KNOWN_THEME, &path_str, &mut diags);
            }
            if let Some(toml::Value::Table(t)) = table.get("beta") {
                check_unknown_keys(t, "beta", KNOWN_BETA, &path_str, &mut diags);
            }
            if let Some(toml::Value::Table(t)) = table.get("log") {
                check_unknown_keys(t, "log", KNOWN_LOG, &path_str, &mut diags);
            }
            if let Some(toml::Value::Table(t)) = table.get("notifications") {
                check_unknown_keys(
                    t,
                    "notifications",
                    KNOWN_NOTIFICATIONS,
                    &path_str,
                    &mut diags,
                );
            }
            if let Some(toml::Value::Table(t)) = table.get("ai") {
                check_unknown_keys(t, "ai", KNOWN_AI, &path_str, &mut diags);
                if let Some(toml::Value::Table(or)) = t.get("openrouter") {
                    check_unknown_keys(
                        or,
                        "ai.openrouter",
                        KNOWN_AI_OPENROUTER,
                        &path_str,
                        &mut diags,
                    );
                }
                if let Some(toml::Value::Table(ol)) = t.get("ollama") {
                    check_unknown_keys(ol, "ai.ollama", KNOWN_AI_OLLAMA, &path_str, &mut diags);
                }
            }
            if let Some(toml::Value::Table(t)) = table.get("keybindings") {
                check_unknown_keys(t, "keybindings", KNOWN_KEYBINDINGS, &path_str, &mut diags);
            }
            if let Some(toml::Value::Table(t)) = table.get("agents") {
                check_unknown_keys(t, "agents", KNOWN_AGENTS, &path_str, &mut diags);
            }
            if let Some(toml::Value::Table(t)) = table.get("cli") {
                check_unknown_keys(t, "cli", KNOWN_CLI, &path_str, &mut diags);
            }
        }
    }

    diags
}

fn check_unknown_keys(
    table: &toml::map::Map<String, toml::Value>,
    section: &str,
    known: &[&str],
    path: &str,
    diags: &mut Vec<ConfigDiagnostic>,
) {
    for key in table.keys() {
        if !known.contains(&key.as_str()) {
            diags.push(ConfigDiagnostic::UnknownKey {
                path: path.to_string(),
                table: section.to_string(),
                key: key.clone(),
            });
        }
    }
}

pub fn validate_all() -> Vec<ConfigDiagnostic> {
    let mut diags = validate_from_path(&config_path());
    if let Some(root) = active_workspace_root() {
        let project_path = workspace_config_path(&root);
        diags.extend(validate_from_path(&project_path));
    }
    diags
}

/// Per-action keybinding overrides. Each field is the name of an action;
/// the value is a key combo string like `"cmd+d"` or `"cmd+shift+d"`.
/// Omitting a field preserves the default binding for that action.
/// Format: `[modifier+]...[modifier+]key` (case-insensitive).
/// Modifiers: cmd/command, shift, ctrl/control, alt/opt/option.
/// Keys: a-z, 0-9, enter, escape, tab, space, backspace, delete, up, down,
///   left, right, open_bracket, close_bracket, backslash, slash, comma,
///   period, equals, minus (and symbol aliases like "[", "]", "\\", etc.).
#[derive(Deserialize, Default, Clone)]
pub struct KeybindingsConfig {
    pub quit: Option<String>,
    pub close_pane: Option<String>,
    pub toggle_command_palette: Option<String>,
    pub split_horizontal: Option<String>,
    pub split_vertical: Option<String>,
    pub split_right: Option<String>,
    pub split_down: Option<String>,
    pub swap_pane_left: Option<String>,
    pub swap_pane_down: Option<String>,
    pub swap_pane_up: Option<String>,
    pub swap_pane_right: Option<String>,
    pub send_pane_left: Option<String>,
    pub send_pane_down: Option<String>,
    pub send_pane_up: Option<String>,
    pub send_pane_right: Option<String>,
    pub navigate_left: Option<String>,
    pub navigate_down: Option<String>,
    pub navigate_up: Option<String>,
    pub navigate_right: Option<String>,
    pub new_tab: Option<String>,
    pub next_tab: Option<String>,
    pub prev_tab: Option<String>,
    pub first_tab: Option<String>,
    pub last_tab: Option<String>,
    pub nav_back: Option<String>,
    pub focus_history_forward: Option<String>,
    pub toggle_sidebar: Option<String>,
    pub toggle_zoom: Option<String>,
    pub toggle_shortcuts: Option<String>,
    pub rename_context: Option<String>,
    pub rename_pane: Option<String>,
    pub new_context: Option<String>,
    pub new_page_right: Option<String>,
    pub toggle_minimap: Option<String>,
    pub scroll_up: Option<String>,
    pub scroll_down: Option<String>,
    pub increase_font_size: Option<String>,
    pub decrease_font_size: Option<String>,
    pub open_file_browser: Option<String>,
    pub open_quick_note: Option<String>,
    pub open_config: Option<String>,
    pub reload_config: Option<String>,
    pub open_secrets_manager: Option<String>,
    pub force_reload_app: Option<String>,
    pub toggle_notification_modal: Option<String>,
    pub open_scratchpad: Option<String>,
    pub push_to_subcontext: Option<String>,
    pub new_child_context: Option<String>,
    pub set_context_root_from_cwd: Option<String>,
    pub hide_pane: Option<String>,
    pub park_context: Option<String>,
    pub open_notes_picker: Option<String>,
}

impl KeybindingsConfig {
    fn overlay(&mut self, other: Self) {
        macro_rules! overlay_field {
            ($field:ident) => {
                if other.$field.is_some() {
                    self.$field = other.$field;
                }
            };
        }
        overlay_field!(quit);
        overlay_field!(close_pane);
        overlay_field!(toggle_command_palette);
        overlay_field!(split_horizontal);
        overlay_field!(split_vertical);
        overlay_field!(split_right);
        overlay_field!(split_down);
        overlay_field!(swap_pane_left);
        overlay_field!(swap_pane_down);
        overlay_field!(swap_pane_up);
        overlay_field!(swap_pane_right);
        overlay_field!(send_pane_left);
        overlay_field!(send_pane_down);
        overlay_field!(send_pane_up);
        overlay_field!(send_pane_right);
        overlay_field!(navigate_left);
        overlay_field!(navigate_down);
        overlay_field!(navigate_up);
        overlay_field!(navigate_right);
        overlay_field!(new_tab);
        overlay_field!(next_tab);
        overlay_field!(prev_tab);
        overlay_field!(first_tab);
        overlay_field!(last_tab);
        overlay_field!(nav_back);
        overlay_field!(focus_history_forward);
        overlay_field!(toggle_sidebar);
        overlay_field!(toggle_zoom);
        overlay_field!(toggle_shortcuts);
        overlay_field!(rename_context);
        overlay_field!(rename_pane);
        overlay_field!(new_context);
        overlay_field!(new_page_right);
        overlay_field!(toggle_minimap);
        overlay_field!(scroll_up);
        overlay_field!(scroll_down);
        overlay_field!(increase_font_size);
        overlay_field!(decrease_font_size);
        overlay_field!(open_file_browser);
        overlay_field!(open_quick_note);
        overlay_field!(open_config);
        overlay_field!(reload_config);
        overlay_field!(open_secrets_manager);
        overlay_field!(force_reload_app);
        overlay_field!(toggle_notification_modal);
        overlay_field!(open_scratchpad);
        overlay_field!(push_to_subcontext);
        overlay_field!(new_child_context);
        overlay_field!(set_context_root_from_cwd);
        overlay_field!(hide_pane);
        overlay_field!(park_context);
        overlay_field!(open_notes_picker);
    }
}

#[derive(Deserialize, Default, Clone)]
pub struct PlexiConfig {
    pub config_version: Option<u32>,
    pub font_size: Option<f32>,
    /// Inter-pane gap width in pixels. Default: 4.0. Clamped to [0.0, 20.0].
    pub pane_gap: Option<f32>,
    /// Pane title bar font size. Default: 11.0. Clamped to [6.0, 32.0].
    pub pane_title_font_size: Option<f32>,
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
    pub keybindings: Option<KeybindingsConfig>,
    pub quick_note: Option<QuickNoteConfig>,
    pub focus_history_depth: Option<usize>,
    pub agents: Option<AgentsConfig>,
    pub cli: Option<CliConfig>,
}

/// CLI behavior configuration.
#[derive(Deserialize, Default, Clone)]
pub struct CliConfig {
    /// Print contextual tips after CLI commands. Default: `true`. Set to `false` to suppress.
    pub tips: Option<bool>,
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
    /// Per-app daily spend cap in USD. Default $1.00.
    pub per_app_daily_usd: Option<f64>,
    /// Global daily spend cap across all apps in USD. Default $10.00.
    pub global_daily_usd: Option<f64>,
}

impl AiConfig {
    /// Resolved per-app daily spend cap. Falls back to $1.00 if unset.
    pub fn effective_per_app_daily_usd(&self) -> f64 {
        self.per_app_daily_usd.unwrap_or(1.0)
    }

    /// Resolved global daily spend cap. Falls back to $10.00 if unset.
    pub fn effective_global_daily_usd(&self) -> f64 {
        self.global_daily_usd.unwrap_or(10.0)
    }
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
        if other.per_app_daily_usd.is_some() {
            self.per_app_daily_usd = other.per_app_daily_usd;
        }
        if other.global_daily_usd.is_some() {
            self.global_daily_usd = other.global_daily_usd;
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

pub const DEFAULT_AGENT_LOW: &str =
    "claude --model claude-haiku-4-5 --dangerously-skip-permissions '{cmd}'";
pub const DEFAULT_AGENT_MEDIUM: &str =
    "claude --model claude-sonnet-4-6 --dangerously-skip-permissions '{cmd}'";
pub const DEFAULT_AGENT_HIGH: &str = "claude --dangerously-skip-permissions '{cmd}'";

/// Coding agent command templates for dispatch. Each field is a shell command template
/// where `{cmd}` is replaced with the prompt or slash command at dispatch time.
#[derive(Deserialize, Default, Clone)]
pub struct AgentsConfig {
    /// Fast/cheap tasks. Default: claude haiku with --dangerously-skip-permissions.
    pub low: Option<String>,
    /// Standard work. Default: claude sonnet with --dangerously-skip-permissions.
    pub medium: Option<String>,
    /// Complex/autonomous tasks. Default: claude with --dangerously-skip-permissions.
    pub high: Option<String>,
}

impl AgentsConfig {
    pub fn effective_low(&self) -> &str {
        self.low.as_deref().unwrap_or(DEFAULT_AGENT_LOW)
    }
    pub fn effective_medium(&self) -> &str {
        self.medium.as_deref().unwrap_or(DEFAULT_AGENT_MEDIUM)
    }
    pub fn effective_high(&self) -> &str {
        self.high.as_deref().unwrap_or(DEFAULT_AGENT_HIGH)
    }
    fn overlay(&mut self, other: Self) {
        if other.low.is_some() {
            self.low = other.low;
        }
        if other.medium.is_some() {
            self.medium = other.medium;
        }
        if other.high.is_some() {
            self.high = other.high;
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
    pub retention_days: Option<u32>,
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
    /// Opacity for unfocused panes when ghost is enabled. Range 0.0–1.0; default 0.9.
    /// Setting this implies ghost = true regardless of the `ghost` flag.
    pub ghost_opacity: Option<f32>,
    pub osc_pane_title: Option<bool>,
}

#[derive(Deserialize, Default, Clone)]
pub struct ThemeConfig {
    // UI chrome
    pub bg_darkest: Option<String>,
    pub bg_sidebar: Option<String>,
    pub bg_toolbar: Option<String>,
    pub terminal_bg: Option<String>,
    pub bg_hover: Option<String>,
    pub bg_sidebar_hover: Option<String>,
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

use std::sync::{Mutex, OnceLock};

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

/// Ensure the profile directory exists and the Python SDK is up to date.
/// Returns `true` if the profile directory was newly created (first launch),
/// `false` if it already existed.
///
/// The embedded SDK is extracted only when missing or when the binary version
/// changed since the last extract, so manual edits and symlinks in the profile
/// SDK dir survive normal app launches.
pub fn ensure_profile_initialized() -> bool {
    let dir = config_dir();
    let is_new = if !dir.exists() {
        if let Err(e) = std::fs::create_dir_all(&dir) {
            eprintln!("profile init: failed to create {}: {e}", dir.display());
            return false;
        }
        let apps_dir = dir.join("apps");
        if let Err(e) = std::fs::create_dir_all(&apps_dir) {
            eprintln!("profile init: failed to create apps dir: {e}");
            return false;
        }
        log::info!("profile init: created new profile at {}", dir.display());
        true
    } else {
        false
    };

    // Only re-extract when the embedded SDK version differs from what was last written.
    // This preserves manual edits and symlinks placed in the profile SDK dir during
    // normal launches while still ensuring upgrades land the correct version.
    let sdk_dir = dir.join("sdk");
    let sdk_dest = sdk_dir.join("plexi_sdk");
    let stamp_path = sdk_dir.join(".sdk_version");
    let current_version = env!("CARGO_PKG_VERSION");
    let needs_extract = !sdk_dest.exists()
        || std::fs::read_to_string(&stamp_path)
            .map(|v| v.trim() != current_version)
            .unwrap_or(true);

    if needs_extract {
        let _ = std::fs::remove_dir_all(&sdk_dest);
        if let Err(e) = std::fs::create_dir_all(&sdk_dest) {
            eprintln!("profile init: failed to create sdk dir: {e}");
        } else {
            let embedded_sdk =
                include_dir::include_dir!("$CARGO_MANIFEST_DIR/sdk/python/plexi_sdk");
            if let Err(e) = embedded_sdk.extract(&sdk_dest) {
                eprintln!("profile init: failed to seed SDK: {e}");
            } else {
                let _ = std::fs::create_dir_all(&sdk_dir);
                let _ = std::fs::write(&stamp_path, current_version);
                log::info!(
                    "profile init: seeded SDK v{current_version} to {}",
                    sdk_dest.display()
                );
            }
        }
    } else {
        log::info!("profile init: SDK v{current_version} up to date, skipping extract");
    }

    is_new
}

/// Returns the build channel for this binary: the suffix after `plexi-`, or `None` for the bare `plexi` binary (main channel).
pub fn build_channel() -> Option<String> {
    let basename = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))?;
    let name = basename.as_str();
    if let Some(suffix) = name.strip_prefix("plexi-").filter(|s| !s.is_empty()) {
        Some(suffix.to_string())
    } else {
        None
    }
}

/// Maps a binary basename to its config directory name.
/// `plexi` → `.plexi`; `plexi-<suffix>` → `.plexi-<suffix>`.
fn channel_suffix_from_basename(basename: &str) -> String {
    if let Some(suffix) = basename.strip_prefix("plexi-").filter(|s| !s.is_empty()) {
        format!(".plexi-{suffix}")
    } else {
        ".plexi".to_string()
    }
}

/// Returns the workspace channel directory name for the current binary.
/// This is the dot-prefixed dir used inside workspace roots to scope
/// workspace state per channel: `.plexi` (main), `.plexi-alpha`, `.plexi-beta`,
/// `.plexi-pr-N`, etc. This is the single source of truth for workspace
/// channel dirs — never derive it independently elsewhere.
pub fn workspace_channel_dir() -> String {
    if let Some(Some(profile)) = PROFILE_OVERRIDE.get() {
        return format!(".plexi-{profile}");
    }
    static CHANNEL_DIR: OnceLock<String> = OnceLock::new();
    CHANNEL_DIR
        .get_or_init(|| {
            let basename = std::env::current_exe()
                .ok()
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                .unwrap_or_else(|| "plexi".to_string());
            channel_suffix_from_basename(&basename)
        })
        .clone()
}

/// Returns the config directory name based on the running binary basename.
fn config_dir_name() -> String {
    if let Some(Some(profile)) = PROFILE_OVERRIDE.get() {
        return format!(".plexi-{profile}");
    }
    let basename = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "plexi".to_string());
    channel_suffix_from_basename(&basename)
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

pub fn workspace_config_path(workspace_root: &Path) -> PathBuf {
    workspace_config_path_for_channel(workspace_root, &workspace_channel_dir())
}

fn workspace_config_path_for_channel(workspace_root: &Path, channel_dir: &str) -> PathBuf {
    workspace_root.join(channel_dir).join("config.toml")
}

/// Dev override: when `PLEXI_SDK_PATH` is set, Python apps import the SDK from
/// that directory (which must contain the `plexi_sdk` package) instead of the
/// binary-embedded copy. This enables live SDK iteration — edit
/// `sdk/python/plexi_sdk/*.py`, re-render, no rebuild. Returns the path only if
/// it exists and actually contains a `plexi_sdk` package; otherwise `None` so a
/// stale/typo'd value fails loudly via the missing-import rather than silently.
pub fn sdk_path_override() -> Option<PathBuf> {
    let raw = std::env::var_os("PLEXI_SDK_PATH")?;
    let path = PathBuf::from(raw);
    if path.join("plexi_sdk").join("__init__.py").is_file() {
        Some(path)
    } else {
        eprintln!(
            "PLEXI_SDK_PATH={} does not contain a plexi_sdk package (expected {}/plexi_sdk/__init__.py) — ignoring",
            path.display(),
            path.display()
        );
        None
    }
}

/// Builds the `PYTHONPATH` value for Python app subprocesses.
///
/// This is the single source for how PYTHONPATH is constructed. Both
/// `ProcessApp::launch` and the static render path must call this so the
/// priority is defined in exactly one place:
///   `PLEXI_SDK_PATH` override → config-dir SDK → bundle SDK (when present)
pub fn build_pythonpath(bundle_sdk: Option<&std::path::Path>) -> String {
    let sdk_dir = config_dir().join("sdk");
    let mut pythonpath = sdk_dir.to_string_lossy().into_owned();
    if let Some(bsdk) = bundle_sdk.filter(|p| p.exists()) {
        pythonpath.push(':');
        pythonpath.push_str(&bsdk.to_string_lossy());
    }
    if let Some(dev_sdk) = sdk_path_override() {
        pythonpath = format!("{}:{}", dev_sdk.to_string_lossy(), pythonpath);
    }
    pythonpath
}

pub const CONFIG_TEMPLATE: &str = include_str!("../../scripts/default-config.toml");

pub const CURRENT_CONFIG_VERSION: u32 = 1;

/// Migrates an existing config file forward to `CURRENT_CONFIG_VERSION`.
///
/// Uses line-based text manipulation to preserve comments. Each version bump
/// adds a migration arm keyed by the target version. After all migrations run,
/// the `config_version` line is written back so subsequent calls are no-ops.
pub fn migrate_config(path: &Path) {
    let data = match std::fs::read_to_string(path) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
        Err(e) => {
            log::warn!("config migrate: failed to read {}: {e}", path.display());
            return;
        }
    };

    let current_version = toml::from_str::<PlexiConfig>(&data)
        .ok()
        .and_then(|c| c.config_version)
        .unwrap_or(0);

    if current_version >= CURRENT_CONFIG_VERSION {
        return;
    }

    log::info!(
        "config migrate: {} at v{current_version}, migrating to v{CURRENT_CONFIG_VERSION}",
        path.display()
    );

    let mut lines: Vec<String> = data.lines().map(str::to_owned).collect();
    let mut version = current_version;

    // v0 → v1: add config_version field after the opening comment header
    if version < 1 {
        if !lines.iter().any(|l| l.starts_with("config_version")) {
            let insert_at = lines
                .iter()
                .position(|l| !l.starts_with('#') && !l.trim().is_empty())
                .unwrap_or(0);
            lines.insert(insert_at, String::new());
            lines.insert(insert_at, "config_version = 1".to_string());
        } else {
            for l in lines.iter_mut() {
                if l.starts_with("config_version") {
                    *l = "config_version = 1".to_string();
                    break;
                }
            }
        }
        version = 1;
    }

    let trailing_newline = data.ends_with('\n');
    let mut updated = lines.join("\n");
    if trailing_newline {
        updated.push('\n');
    }

    match std::fs::write(path, &updated) {
        Ok(()) => log::info!("config migrate: {} migrated to v{version}", path.display()),
        Err(e) => log::error!("config migrate: failed to write {}: {e}", path.display()),
    }
}

/// Ensures the config file exists, creating it from the default template if not.
/// Returns the config file path.
pub fn ensure_config_exists() -> std::path::PathBuf {
    let path = config_path();
    if !path.exists() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&path, CONFIG_TEMPLATE);
    }
    path
}

pub fn open_config_file() {
    let path = config_path();

    if !path.exists() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&path, CONFIG_TEMPLATE);
    }

    if !open_file_with_fallback(&path) {
        log::error!(
            "open_config_file: could not open {} with any available editor",
            path.display()
        );
    }
}

/// Opens `path` with a fallback chain: VS Code → system default → TextEdit.
/// Returns `true` if any opener succeeded.
#[cfg(target_os = "macos")]
pub fn open_file_with_fallback(path: &std::path::Path) -> bool {
    let candidates: &[&[&str]] = &[&["-a", "Visual Studio Code"], &["-t"], &["-a", "TextEdit"]];
    for args in candidates {
        let ok = std::process::Command::new("open")
            .args(*args)
            .arg(path)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            log::info!(
                "open_file_with_fallback: opened {} (args: {:?})",
                path.display(),
                args
            );
            return true;
        }
        let opener_desc = if args.is_empty() {
            "system default".to_string()
        } else {
            format!("{:?}", args)
        };
        log::warn!(
            "open_file_with_fallback: opener {} failed for {}, trying next",
            opener_desc,
            path.display()
        );
    }
    false
}

#[cfg(not(target_os = "macos"))]
pub fn open_file_with_fallback(path: &std::path::Path) -> bool {
    log::warn!(
        "open_file_with_fallback: no platform implementation for {}",
        path.display()
    );
    false
}

impl PlexiConfig {
    /// Load the global config only — no project-level merge. Most call sites
    /// should prefer [`load_with_workspace`] so a workspace's
    /// channel-scoped config can override.
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
        migrate_config(&path);
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

    /// Load the global config and overlay the workspace's channel-scoped
    /// config on top if it exists. Project-level values override globals on a
    /// per-field basis; unset project fields preserve the global value.
    pub fn load_with_workspace(workspace_root: Option<&Path>) -> Self {
        let mut merged = Self::load();
        let Some(root) = workspace_root else {
            return merged;
        };
        let project_path = workspace_config_path(root);
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
        if other.pane_gap.is_some() {
            self.pane_gap = other.pane_gap;
        }
        if other.pane_title_font_size.is_some() {
            self.pane_title_font_size = other.pane_title_font_size;
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
        match (self.keybindings.as_mut(), other.keybindings) {
            (Some(existing), Some(incoming)) => existing.overlay(incoming),
            (None, Some(incoming)) => self.keybindings = Some(incoming),
            _ => {}
        }
        match (self.quick_note.as_mut(), other.quick_note) {
            (Some(existing), Some(incoming)) => existing.overlay(incoming),
            (None, Some(incoming)) => self.quick_note = Some(incoming),
            _ => {}
        }
        match (self.agents.as_mut(), other.agents) {
            (Some(existing), Some(incoming)) => existing.overlay(incoming),
            (None, Some(incoming)) => self.agents = Some(incoming),
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
        overlay_field!(bg_sidebar_hover);
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
        if other.ghost_opacity.is_some() {
            self.ghost_opacity = other.ghost_opacity;
        }
        if other.osc_pane_title.is_some() {
            self.osc_pane_title = other.osc_pane_title;
        }
    }

    /// Resolved unfocused-pane opacity.
    /// `ghost_opacity` set → use it; `ghost = true` → 0.9; else → None (no dimming).
    pub fn unfocused_opacity(&self) -> Option<f32> {
        if let Some(opacity) = self.ghost_opacity {
            Some(opacity)
        } else if self.ghost.unwrap_or(false) {
            Some(0.9)
        } else {
            None
        }
    }
}

impl LogConfig {
    fn overlay(&mut self, other: Self) {
        if other.level.is_some() {
            self.level = other.level;
        }
        if other.retention_days.is_some() {
            self.retention_days = other.retention_days;
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

/// Quick Note modal configuration.
#[derive(Deserialize, Default, Clone)]
pub struct QuickNoteConfig {
    #[serde(default)]
    pub destinations: Vec<QuickNoteNode>,
}

impl QuickNoteConfig {
    fn overlay(&mut self, other: Self) {
        if !other.destinations.is_empty() {
            self.destinations = other.destinations;
            warn_deprecated_quick_note_fields(&self.destinations);
        }
    }
}

fn warn_deprecated_quick_note_fields(nodes: &[QuickNoteNode]) {
    for node in nodes {
        if node.dest_type.as_deref() == Some("pane") || node.position.is_some() {
            log::warn!(
                "QuickNote: destination '{}' uses deprecated 'type = \"pane\"' or 'position'. \
                 Migrate to a bare 'command' string. See config docs for the new schema.",
                node.label
            );
        }
        // Warn if command wraps tokens in quotes — tokens are already shell-escaped
        if let Some(cmd) = &node.command {
            for token in ["{note}", "{cwd}", "{context_root}"] {
                let single = format!("'{token}'");
                let double = format!("\"{token}\"");
                if cmd.contains(&single) || cmd.contains(&double) {
                    log::warn!(
                        "QuickNote: destination '{}' wraps {token} in quotes — \
                         tokens are already shell-escaped, extra quotes will break substitution",
                        node.label
                    );
                }
            }
        }
        if let Some(children) = &node.children_cmd {
            for token in ["{note}", "{cwd}", "{context_root}"] {
                let single = format!("'{token}'");
                let double = format!("\"{token}\"");
                if children.contains(&single) || children.contains(&double) {
                    log::warn!(
                        "QuickNote: destination '{}' children_cmd wraps {token} in quotes — \
                         tokens are already shell-escaped, extra quotes will break substitution",
                        node.label
                    );
                }
            }
        }
        if let Some(opts) = &node.options {
            warn_deprecated_quick_note_fields(opts);
        }
    }
}

/// A node in the quick-note destination tree. Used at every level — root destinations,
/// static submenu children (`options`), and dynamic children (`children_cmd` output).
#[derive(Deserialize, Clone)]
pub struct QuickNoteNode {
    pub key: u8,
    pub label: String,
    /// Shell command template. Tokens: {note}, {cwd}, {context_root}. Required on leaves.
    pub command: Option<String>,
    /// If true, run command without a visible terminal pane (fire-and-forget background spawn).
    #[serde(default)]
    pub hidden: bool,
    /// Keep spawned pane alive after command exits. Defaults to false.
    pub stay_alive: Option<bool>,
    /// Static child nodes (submenu). Mutually exclusive with `command` at this node.
    pub options: Option<Vec<QuickNoteNode>>,
    /// Shell command whose stdout populates children dynamically. Format: one entry per
    /// line as `key|label|command`. {note} and {cwd} in output are substituted before execution.
    pub children_cmd: Option<String>,
    // ── Deprecated ────────────────────────────────────────────────────────────────────────
    // These fields are parsed for backward compatibility and to emit a migration warning,
    // but are no longer used for dispatch.
    #[serde(rename = "type")]
    pub dest_type: Option<String>,
    pub position: Option<String>,
    pub path: Option<String>,
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
/// [`crate::app::registry::resolve_workspace_root`].
pub fn adopted_workspace_root() -> Option<PathBuf> {
    ADOPTED_WORKSPACE_ROOT.get().and_then(|opt| opt.clone())
}

// ── Adopted context path (set once by main when `plexi <path>` targets a
// directory without a `.plexi/` workspace) ──────────────────────────────────

static ADOPTED_CONTEXT_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Store a directory path for context-only opening (no workspace).
/// Called from `parse_workspace_path_arg` when the target dir has no `.plexi/`.
pub fn set_adopted_context_path(path: PathBuf) {
    if let Ok(mut guard) = ADOPTED_CONTEXT_PATH.lock() {
        *guard = Some(path);
    }
}

/// Consume the adopted context path. Returns `Some` at most once.
pub fn take_adopted_context_path() -> Option<PathBuf> {
    ADOPTED_CONTEXT_PATH
        .lock()
        .ok()
        .and_then(|mut guard| guard.take())
}

/// Convenience: the active workspace root for this process. Returns the
/// adopted root if set, otherwise walks up from the current working
/// directory looking for a `.plexi/` ancestor.
pub fn active_workspace_root() -> Option<PathBuf> {
    if let Some(adopted) = adopted_workspace_root() {
        return Some(adopted);
    }
    let cwd = std::env::current_dir().ok()?;
    crate::app::registry::resolve_workspace_root(&cwd)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn config_dir_name_bare_plexi() {
        let result = channel_suffix_from_basename("plexi");
        assert_eq!(result, ".plexi");
    }

    #[test]
    fn config_dir_name_alpha() {
        assert_eq!(channel_suffix_from_basename("plexi-alpha"), ".plexi-alpha");
    }

    #[test]
    fn config_dir_name_gpui() {
        assert_eq!(channel_suffix_from_basename("plexi-gpui"), ".plexi-gpui");
    }

    #[test]
    fn config_dir_name_pr() {
        assert_eq!(
            channel_suffix_from_basename("plexi-pr-817"),
            ".plexi-pr-817"
        );
    }

    #[test]
    fn config_dir_name_path_with_alpha_component() {
        // A binary at /Users/alpha/bin/plexi must NOT resolve to .plexi-alpha.
        // The extractor must use basename only, not the full path.
        assert_eq!(channel_suffix_from_basename("plexi"), ".plexi");
    }

    fn write(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn validate_valid_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "font_size = 14.0\ntheme_preset = \"dracula\"\n").unwrap();
        let diags = validate_from_path(&path);
        assert!(diags.is_empty(), "expected no diagnostics, got: {diags:?}");
    }

    #[test]
    fn validate_known_keybindings_cover_pane_hide_and_context_park() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            "[keybindings]\nhide_pane = \"cmd+u\"\npark_context = \"cmd+shift+u\"\n",
        )
        .unwrap();
        let diags = validate_from_path(&path);
        assert!(diags.is_empty(), "expected no diagnostics, got: {diags:?}");
    }

    #[test]
    fn validate_parse_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "font_size = not_a_number\n").unwrap();
        let diags = validate_from_path(&path);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].is_error());
        assert!(diags[0].to_string().contains("config.toml"));
    }

    #[test]
    fn validate_unknown_key_top_level() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "font_size = 14.0\nfoobar = true\n").unwrap();
        let diags = validate_from_path(&path);
        assert_eq!(diags.len(), 1);
        assert!(!diags[0].is_error());
        assert!(diags[0].to_string().contains("foobar"));
    }

    #[test]
    fn validate_unknown_key_in_section() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "[beta]\ncrt = true\nfake_flag = true\n").unwrap();
        let diags = validate_from_path(&path);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].to_string().contains("fake_flag"));
        assert!(diags[0].to_string().contains("[beta]"));
    }

    #[test]
    fn validate_missing_file_no_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nonexistent.toml");
        let diags = validate_from_path(&path);
        assert!(diags.is_empty());
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
    fn workspace_config_path_uses_channel_dir() {
        let workspace = tempfile::tempdir().unwrap();

        assert_eq!(
            workspace_config_path_for_channel(workspace.path(), ".plexi"),
            workspace.path().join(".plexi").join("config.toml")
        );
        assert_eq!(
            workspace_config_path_for_channel(workspace.path(), ".plexi-alpha"),
            workspace.path().join(".plexi-alpha").join("config.toml")
        );
        assert_eq!(
            workspace_config_path_for_channel(workspace.path(), ".plexi-pr-817"),
            workspace.path().join(".plexi-pr-817").join("config.toml")
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

        // Workspace exists but has no main-channel config.toml.
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

    #[test]
    fn migrate_config_adds_version_to_v0() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "font_size = 14.0\n").unwrap();
        migrate_config(&path);
        let result = fs::read_to_string(&path).unwrap();
        assert!(
            result.contains("config_version = 1"),
            "version should be added: {result}"
        );
        assert!(
            result.contains("font_size = 14.0"),
            "font_size should be preserved: {result}"
        );
    }

    #[test]
    fn migrate_config_no_op_when_current() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let content = "config_version = 1\nfont_size = 14.0\n";
        fs::write(&path, content).unwrap();
        migrate_config(&path);
        let result = fs::read_to_string(&path).unwrap();
        assert_eq!(
            result, content,
            "should not modify an already-current config"
        );
    }

    #[test]
    fn migrate_config_preserves_comments() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let content = "# My comment\nfont_size = 14.0\n";
        fs::write(&path, content).unwrap();
        migrate_config(&path);
        let result = fs::read_to_string(&path).unwrap();
        assert!(
            result.contains("# My comment"),
            "comments should be preserved: {result}"
        );
        assert!(
            result.contains("config_version = 1"),
            "version should be added: {result}"
        );
    }
}
