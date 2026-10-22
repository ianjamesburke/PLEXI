pub mod edit;
pub mod watcher;

use serde::{Deserialize, Serialize};
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
    DeprecatedSection {
        path: String,
        section: String,
        hint: String,
        /// true → error (config key completely removed); false → warning (migration path still works).
        error: bool,
    },
    InvalidKeyBinding {
        path: String,
        error: String,
    },
    InvalidValue { path: String, key: String, error: String },
    MisplacedTable {
        path: String,
        table: String,
        source_file: &'static str,
        target_config_path: String,
    },
}

impl ConfigDiagnostic {
    pub fn is_error(&self) -> bool {
        match self {
            Self::ReadError { .. }
            | Self::ParseError { .. }
            | Self::InvalidKeyBinding { .. }
            | Self::InvalidValue { .. } => true,
            Self::DeprecatedSection { error, .. } => *error,
            Self::UnknownKey { .. } | Self::MisplacedTable { .. } => false,
        }
    }
}

impl std::fmt::Display for ConfigDiagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ReadError { path, error } => write!(f, "{path}: read error: {error}"),
            Self::ParseError { path, error } => write!(f, "{path}: {error}"),
            Self::InvalidKeyBinding { path, error } => write!(f, "{path}: {error}"),
            Self::InvalidValue { path, key, error } => write!(f, "{path}: {key}: {error}"),
            Self::MisplacedTable {
                path,
                table,
                source_file,
                target_config_path,
            } => write!(
                f,
                "{path}: [{table}] is ignored in {source_file}; put it in {target_config_path}"
            ),
            Self::UnknownKey { path, table, key } => {
                if table.is_empty() {
                    write!(f, "{path}: unknown key `{key}`")
                } else {
                    write!(f, "{path}: unknown key `{key}` in [{table}]")
                }
            }
            Self::DeprecatedSection {
                path,
                section,
                hint,
                error,
            } => {
                let severity = if *error { "error" } else { "warning" };
                write!(f, "{path}: {severity}: `{section}` is deprecated. {hint}")
            }
        }
    }
}

const KNOWN_TOP_LEVEL: &[&str] = &[
    "config_version",
    "font_size",
    "theme_preset",
    "theme",
    "attention",
    "effects",
    "log",
    "notifications",
    "ai",
    "models",
    "confirm_quit",
    "confirm_close",
    "confirm_context_close",
    "keybindings",
    "keys",
    "focus_history_depth",
    "agents",
    "cli",
    "marketplace",
    "cloud",
    "permissions",
    "file_handlers",
    "experiments",
    "pane_gap",
    "pane_title_font_size",
    "osc_pane_title",
];
const KNOWN_AGENTS: &[&str] = &["low", "medium", "high", "restore_mode", "stalled_after_secs", "palette_agent_results", "os_notifications"];
const KNOWN_CLI: &[&str] = &["tips"];
const KNOWN_PERMISSIONS: &[&str] = &[
    "soft_grant_days",
    "default_posture",
    "allow",
    "ask",
    "deny",
];
const KNOWN_THEME: &[&str] = &[
    "pip_working", "pip_idle", "pip_blocked", "pip_dim",
    "state_preset",
    "state_working",
    "state_waiting",
    "state_blocked",
    "state_needs_you",
    "state_interrupted",
    "state_error",

    "preset",
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
const KNOWN_EFFECTS: &[&str] = &["crt", "ghost", "ghost_opacity"];
const KNOWN_LOG: &[&str] = &["level", "retention_days"];
const KNOWN_CLOUD: &[&str] = &["retain_local_history"];
const KNOWN_NOTIFICATIONS: &[&str] = &["enabled", "focus_mode", "sound"];
const KNOWN_AI: &[&str] = &["per_app_daily_usd", "global_daily_usd", "client"];
const KNOWN_MODE_NAMES: &[&str] = &["fast", "slow"];
const KNOWN_MODEL_MODE: &[&str] = &["provider", "model", "thinking"];
const KNOWN_MARKETPLACE: &[&str] = &[
    "registry_url",
    "cdn_url",
    "submit_url",
    "account_backend",
    "account_url",
    "account_email",
];
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
    "next_context",
    "prev_context",
    "move_context_up",
    "move_context_down",
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
    "open_agents",
    "next_blocked_agent",
    "open_file_browser",
    "open_quick_note",
    "open_config",
    "reload_config",
    "open_secrets_manager",
    "open_assistant",
    "force_reload_app",
    "toggle_notification_modal",
    "open_scratchpad",
    "set_context_root_from_cwd",
    "push_to_subcontext",
    "new_child_context",
    "hide_pane",
    "park_context",
    "open_notes_picker",
    "close_context",
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

    let parsed_cfg = match toml::from_str::<PlexiConfig>(&data) {
        Ok(cfg) => cfg,
        Err(e) => {
            diags.push(ConfigDiagnostic::ParseError {
                path: path_str,
                error: e.to_string(),
            });
            return diags;
        }
    };

    if let Some(ref user_keys) = parsed_cfg.keys {
        let bindings = crate::host::keys::build_key_bindings(parsed_cfg.keybindings.as_ref());
        for err in crate::host::keys::validate_user_keys(user_keys, &bindings) {
            diags.push(ConfigDiagnostic::InvalidKeyBinding {
                path: path_str.clone(),
                error: err,
            });
        }
    }

    if let Ok(value) = toml::from_str::<toml::Value>(&data) {
        if let Some(table) = value.as_table() {
            let target_commands_path = matching_commands_display_path(path);
            for key in table.keys() {
                if KNOWN_COMMANDS_TOP_LEVEL.contains(&key.as_str()) {
                    diags.push(ConfigDiagnostic::MisplacedTable {
                        path: path_str.clone(),
                        table: key.clone(),
                        source_file: "config.toml",
                        target_config_path: target_commands_path.clone(),
                    });
                } else if !KNOWN_TOP_LEVEL.contains(&key.as_str()) {
                    diags.push(ConfigDiagnostic::UnknownKey {
                        path: path_str.clone(),
                        table: String::new(),
                        key: key.clone(),
                    });
                }
            }

            if table.contains_key("experiments") {
                if let Err(error) = experiments_from_path(path) {
                    diags.push(ConfigDiagnostic::ParseError {
                        path: path_str.clone(),
                        error,
                    });
                }
            }

            if let Some(toml::Value::Table(t)) = table.get("attention") {
                check_unknown_keys(t, "attention", &["style", "intensity", "dock"], &path_str, &mut diags);
            }
            if let Some(toml::Value::Table(t)) = table.get("theme") {
                check_unknown_keys(t, "theme", KNOWN_THEME, &path_str, &mut diags);
            }
            if let Some(toml::Value::Table(t)) = table.get("effects") {
                check_unknown_keys(t, "effects", KNOWN_EFFECTS, &path_str, &mut diags);
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
                for key in legacy_ai_keys(t) {
                    diags.push(ConfigDiagnostic::InvalidValue {
                        path: path_str.clone(),
                        key,
                        error: "is no longer supported; use [models.fast] / [models.slow] instead".into(),
                    });
                }
            }
            if let Some(toml::Value::Table(t)) = table.get("models") {
                check_unknown_keys(t, "models", KNOWN_MODE_NAMES, &path_str, &mut diags);
                for mode in KNOWN_MODE_NAMES {
                    if let Some(toml::Value::Table(mode_table)) = t.get(*mode) {
                        check_unknown_keys(mode_table, &format!("models.{mode}"), KNOWN_MODEL_MODE, &path_str, &mut diags);
                    }
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
            if let Some(toml::Value::Table(t)) = table.get("marketplace") {
                check_unknown_keys(t, "marketplace", KNOWN_MARKETPLACE, &path_str, &mut diags);
            }
            if let Some(toml::Value::Table(t)) = table.get("cloud") {
                check_unknown_keys(t, "cloud", KNOWN_CLOUD, &path_str, &mut diags);
            }
            if let Some(toml::Value::Table(t)) = table.get("permissions") {
                check_unknown_keys(t, "permissions", KNOWN_PERMISSIONS, &path_str, &mut diags);
            }
            if table.contains_key("quick_note") {
                diags.push(ConfigDiagnostic::DeprecatedSection {
                    path: path_str.clone(),
                    section: "quick_note".to_string(),
                    hint: "Quick note now saves directly to notes/inbox/ — remove the [[quick_note.destinations]] section from your config. See https://plexiapp.com/docs/config".to_string(),
                    error: true,
                });
            }
            if table.contains_key("theme_preset") {
                diags.push(ConfigDiagnostic::DeprecatedSection {
                    path: path_str.clone(),
                    section: "theme_preset".to_string(),
                    hint: "Use [theme] preset = \"name\" instead. See https://plexiapp.com/docs/config".to_string(),
                    error: false, // still works via fallback; migrate at your convenience
                });
            }
        }
    }

    if let Some(models) = parsed_cfg.models.as_ref() {
        for (name, mode) in [("fast", models.fast.as_ref()), ("slow", models.slow.as_ref())] {
            if let Some(mode) = mode {
                for (field, value) in [("provider", mode.provider.as_deref()), ("model", mode.model.as_deref())] {
                    if value.is_some_and(|v| v.is_empty() || v.chars().any(char::is_whitespace)) {
                        diags.push(ConfigDiagnostic::InvalidValue { path: path_str.clone(), key: format!("models.{name}.{field}"), error: "must be a non-empty string without whitespace".into() });
                    }
                }
                if let Some(thinking) = mode.thinking.as_deref() {
                    if !THINKING_LEVELS.contains(&thinking) {
                        diags.push(ConfigDiagnostic::InvalidValue { path: path_str.clone(), key: format!("models.{name}.thinking"), error: format!("{thinking:?} is not one of {}", THINKING_LEVELS.join("|")) });
                    }
                }
            }
        }
    }

    diags
}

fn legacy_ai_keys(ai: &toml::map::Map<String, toml::Value>) -> Vec<String> {
    let mut keys = Vec::new();
    if ai.contains_key("backend") {
        keys.push("ai.backend".to_string());
    }
    for provider in ["openrouter", "ollama", "local"] {
        if let Some(value) = ai.get(provider) {
            if let Some(table) = value.as_table() {
                for key in table.keys() {
                    keys.push(format!("ai.{provider}.{key}"));
                }
            } else {
                keys.push(format!("ai.{provider}"));
            }
        }
    }
    keys
}

fn legacy_ai_error(data: &str) -> Option<String> {
    let value: toml::Value = toml::from_str(data).ok()?;
    let ai = value.get("ai")?.as_table()?;
    legacy_ai_keys(ai).into_iter().next().map(|key| {
        format!("{key} is no longer supported; use [models.fast] / [models.slow] instead")
    })
}

const KNOWN_COMMANDS_TOP_LEVEL: &[&str] = &["commands", "secrets"];

/// Format the matching `config.toml` display path for a `commands.toml` path:
/// `~/.plexi-<channel>/config.toml` for the global commands file, or
/// `<repo>/.plexi-<channel>/config.toml` for a workspace commands file.
pub fn matching_config_display_path(commands_path: &Path) -> String {
    if commands_path == crate::cli::global_commands_path() {
        format!("~/{}/config.toml", workspace_channel_dir())
    } else {
        commands_path
            .with_file_name("config.toml")
            .display()
            .to_string()
    }
}

/// Format the matching `commands.toml` display path for a `config.toml` path.
pub fn matching_commands_display_path(cfg_path: &Path) -> String {
    if cfg_path == config_path() {
        format!("~/{}/commands.toml", workspace_channel_dir())
    } else {
        cfg_path
            .with_file_name("commands.toml")
            .display()
            .to_string()
    }
}

pub fn validate_commands_from_path(path: &Path) -> Vec<ConfigDiagnostic> {
    let path_str = path.display().to_string();
    let data = match std::fs::read_to_string(path) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => {
            return vec![ConfigDiagnostic::ReadError {
                path: path_str,
                error: e.to_string(),
            }];
        }
    };
    validate_commands_from_str(&data, path)
}

pub fn validate_commands_from_str(data: &str, path: &Path) -> Vec<ConfigDiagnostic> {
    let mut diags = Vec::new();
    let path_str = path.display().to_string();

    if let Err(e) = toml::from_str::<crate::cli::PlexiCommands>(data) {
        diags.push(ConfigDiagnostic::ParseError {
            path: path_str,
            error: e.to_string(),
        });
        return diags;
    }

    if let Ok(value) = toml::from_str::<toml::Value>(data) {
        if let Some(table) = value.as_table() {
            let target_config_path = matching_config_display_path(path);
            for (key, val) in table {
                if KNOWN_COMMANDS_TOP_LEVEL.contains(&key.as_str()) {
                    continue;
                }
                if KNOWN_TOP_LEVEL.contains(&key.as_str()) || val.is_table() {
                    diags.push(ConfigDiagnostic::MisplacedTable {
                        path: path_str.clone(),
                        table: key.clone(),
                        source_file: "commands.toml",
                        target_config_path: target_config_path.clone(),
                    });
                } else {
                    diags.push(ConfigDiagnostic::UnknownKey {
                        path: path_str.clone(),
                        table: String::new(),
                        key: key.clone(),
                    });
                }
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
    pub next_context: Option<String>,
    pub prev_context: Option<String>,
    pub move_context_up: Option<String>,
    pub move_context_down: Option<String>,
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
    /// Open the Agents pane (default Cmd+Alt+A).
    pub open_agents: Option<String>,
    /// Next agent needing attention (default cmd+alt+b).
    pub next_blocked_agent: Option<String>,
    pub open_file_browser: Option<String>,
    pub open_quick_note: Option<String>,
    pub open_config: Option<String>,
    pub reload_config: Option<String>,
    pub open_secrets_manager: Option<String>,
    pub open_assistant: Option<String>,
    pub force_reload_app: Option<String>,
    pub toggle_notification_modal: Option<String>,
    pub open_scratchpad: Option<String>,
    pub push_to_subcontext: Option<String>,
    pub new_child_context: Option<String>,
    pub set_context_root_from_cwd: Option<String>,
    pub hide_pane: Option<String>,
    pub park_context: Option<String>,
    pub open_notes_picker: Option<String>,
    pub close_context: Option<String>,
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
        overlay_field!(next_context);
        overlay_field!(prev_context);
        overlay_field!(move_context_up);
        overlay_field!(move_context_down);
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
        overlay_field!(open_agents);
        overlay_field!(next_blocked_agent);
        overlay_field!(open_file_browser);
        overlay_field!(open_quick_note);
        overlay_field!(open_config);
        overlay_field!(reload_config);
        overlay_field!(open_secrets_manager);
        overlay_field!(open_assistant);
        overlay_field!(force_reload_app);
        overlay_field!(toggle_notification_modal);
        overlay_field!(open_scratchpad);
        overlay_field!(push_to_subcontext);
        overlay_field!(new_child_context);
        overlay_field!(set_context_root_from_cwd);
        overlay_field!(hide_pane);
        overlay_field!(park_context);
        overlay_field!(open_notes_picker);
        overlay_field!(close_context);
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
    /// Apply OSC 0/1/2 title sequences from terminal processes as pane names.
    /// Defaults to true; set false to keep terminal pane names manual-only.
    pub osc_pane_title: Option<bool>,
    pub theme_preset: Option<String>,
    pub theme: Option<ThemeConfig>,
    pub attention: Option<AttentionConfig>,
    pub effects: Option<EffectsConfig>,
    pub log: Option<LogConfig>,
    pub notifications: Option<NotificationsConfig>,
    pub ai: Option<AiConfig>,
    /// Pi FAST/SLOW targets (`[models.fast]` and `[models.slow]`). Every field
    /// is optional and resolves to the built-in target when omitted.
    pub models: Option<ModelModesConfig>,
    /// Set to false to quit immediately on Cmd+Q without triple-press confirmation (default: true).
    pub confirm_quit: Option<bool>,
    /// Set to false to close panes immediately on Cmd+W without a confirmation dialog (default: true).
    pub confirm_close: Option<bool>,
    /// Set to false to close contexts immediately on Cmd+Shift+W without a confirmation dialog (default: true).
    pub confirm_context_close: Option<bool>,
    pub keybindings: Option<KeybindingsConfig>,
    /// User-command shortcut bindings (`[keys] "<chord>" = "run:<command>"`).
    pub keys: Option<std::collections::HashMap<String, String>>,
    pub focus_history_depth: Option<usize>,
    pub agents: Option<AgentsConfig>,
    pub cli: Option<CliConfig>,
    pub marketplace: Option<MarketplaceConfig>,
    /// Optional cloud link (`[cloud]`). Local use never reads this. See
    /// [`CloudConfig`].
    pub cloud: Option<CloudConfig>,
    /// Permissions broker settings (`[permissions]`).
    pub permissions: Option<PermissionsConfig>,
    /// Per-extension file-open handlers (`[file_handlers]`). Keys are bare,
    /// lowercase extensions (`md`, not `.md`); values are `kind:value` handler
    /// specs resolved by [`crate::app::file_handlers::FileHandler`]:
    ///   `app:<id>` — open in a Plexi app by registry id
    ///   `os`       — hand to the OS default opener (`open` / `xdg-open`)
    ///   `cmd:<command>` — reserved; falls back to OS open until wired
    /// A user entry overrides any manifest-declared `file_types` association.
    /// Unmapped extensions fall through to manifest associations, then builtin
    /// media players, then the OS default.
    pub file_handlers: Option<std::collections::HashMap<String, String>>,
}

pub const THINKING_LEVELS: &[&str] = &["off", "minimal", "low", "medium", "high", "xhigh"];

/// FAST/SLOW target overrides for pi-plexi.
#[derive(Deserialize, Default, Clone)]
pub struct ModelModesConfig {
    /// FAST target override. Unset fields use the FAST defaults.
    pub fast: Option<ModelModeConfig>,
    /// SLOW target override. Unset fields use the SLOW defaults.
    pub slow: Option<ModelModeConfig>,
}

/// One pi-plexi target. Provider, model, and thinking can each be overridden independently.
#[derive(Deserialize, Default, Clone)]
pub struct ModelModeConfig {
    /// Model registry provider. Must be non-empty and contain no whitespace.
    pub provider: Option<String>,
    /// Provider model identifier. Must be non-empty and contain no whitespace.
    pub model: Option<String>,
    /// Pi thinking level: `off`, `minimal`, `low`, `medium`, `high`, or `xhigh`.
    pub thinking: Option<String>,
}

/// Fully resolved pi-plexi target returned by `plexi config get --json models`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ResolvedModelMode { pub provider: String, pub model: String, pub thinking: String }

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ResolvedModelModes { pub fast: ResolvedModelMode, pub slow: ResolvedModelMode }

impl ModelModeConfig {
    fn overlay(&mut self, other: Self) { if other.provider.is_some() { self.provider = other.provider; } if other.model.is_some() { self.model = other.model; } if other.thinking.is_some() { self.thinking = other.thinking; } }
    fn resolve(&self, default: ResolvedModelMode) -> ResolvedModelMode { ResolvedModelMode { provider: self.provider.clone().unwrap_or(default.provider), model: self.model.clone().unwrap_or(default.model), thinking: self.thinking.clone().unwrap_or(default.thinking) } }
}
impl ModelModesConfig {
    fn overlay(&mut self, other: Self) { match (self.fast.as_mut(), other.fast) { (Some(a), Some(b)) => a.overlay(b), (None, Some(b)) => self.fast = Some(b), _ => {} } match (self.slow.as_mut(), other.slow) { (Some(a), Some(b)) => a.overlay(b), (None, Some(b)) => self.slow = Some(b), _ => {} } }
    fn resolved(&self) -> ResolvedModelModes { ResolvedModelModes {
        fast: self.fast.as_ref().unwrap_or(&ModelModeConfig::default()).resolve(ResolvedModelMode { provider: "openrouter".into(), model: "deepseek/deepseek-v4.1-flash".into(), thinking: "low".into() }),
        slow: self.slow.as_ref().unwrap_or(&ModelModeConfig::default()).resolve(ResolvedModelMode { provider: "openrouter".into(), model: "openai/gpt-6-astra".into(), thinking: "high".into() }),
    } }
}

/// CLI behavior configuration.
#[derive(Deserialize, Default, Clone)]
pub struct CliConfig {
    /// Print contextual tips after CLI commands. Default: `true`. Set to `false` to suppress.
    pub tips: Option<bool>,
}

/// Hosted marketplace configuration (`[marketplace]`). All fields optional —
/// the hosted catalog/CDN default to `plexiapp.com` in code (see
/// `crate::app::marketplace::DEFAULT_REGISTRY_URL`), so an empty section uses
/// the official registry. `submit_url` and `account_backend` are intentionally
/// unset by default: publishing and accounts fail closed until they are wired
/// up.
#[derive(Deserialize, Default, Clone)]
pub struct MarketplaceConfig {
    /// Override the catalog index URL. Default: official `plexiapp.com` index.
    pub registry_url: Option<String>,
    /// Override the package CDN base. Default: official `plexiapp.com` CDN.
    pub cdn_url: Option<String>,
    /// Publisher submission endpoint. Unset = publishing prepared locally but
    /// not uploaded (fails closed with a clear message).
    pub submit_url: Option<String>,
    /// Account/auth backend selector. `"plexi"` enables plexiapp.com accounts;
    /// unset / `"none"` = stub (login fails closed). Selects the provider in
    /// `crate::app::account::account_provider()`.
    pub account_backend: Option<String>,
    /// Override the accounts service base URL. Default:
    /// `crate::app::account::DEFAULT_ACCOUNT_URL` (`plexiapp.com`).
    pub account_url: Option<String>,
    /// Default email pre-filled by `plexi account login`. Unset = prompt.
    pub account_email: Option<String>,
}

/// Cloud link settings (`[cloud]`).
///
/// An account is optional. Local use never requires one; a session only links
/// this desktop to relay and cloud features. This section does not change that.
/// The one flag here decides whether the retention job may also delete local
/// ledger rows and assistant conversations.
#[derive(Deserialize, Default, Clone)]
pub struct CloudConfig {
    /// When `true`, delete local `ai-ledger.jsonl` rows and assistant
    /// conversation files older than 30 days. Unset or `false` keeps them.
    /// Relay registry rows, queued-envelope metadata, and logs Plexi controls
    /// are pruned either way.
    pub retain_local_history: Option<bool>,
}

/// Default expiry window in days for soft permission grants (`--grant tool=allow:soft`).
pub const DEFAULT_SOFT_GRANT_DAYS: u32 = 30;

/// Permissions broker configuration (`[permissions]`).
#[derive(Deserialize, Default, Clone)]
pub struct PermissionsConfig {
    /// Expiry window in days for soft grants (`--grant tool=allow:soft`). After this window, a soft grant reverts to `ask`. Default: 30.
    pub soft_grant_days: Option<u32>,
}

impl PermissionsConfig {
    fn overlay(&mut self, other: Self) {
        if other.soft_grant_days.is_some() {
            self.soft_grant_days = other.soft_grant_days;
        }
    }

    pub fn effective_soft_grant_days(&self) -> u32 {
        self.soft_grant_days.unwrap_or(DEFAULT_SOFT_GRANT_DAYS)
    }
}

/// Non-model AI settings (`ai.query` capability).
#[derive(Deserialize, Default, Clone)]
pub struct AiConfig {
    /// Per-app daily spend cap in USD. Default $1.00.
    pub per_app_daily_usd: Option<f64>,
    /// Global daily spend cap across all apps in USD. Default $10.00.
    pub global_daily_usd: Option<f64>,
    /// Default ledger client tag for Assistant and agent runs that do not set
    /// one. Free-form (`narrative`, `du`, `personal`). A blank or missing
    /// value leaves the row's client null.
    pub client: Option<String>,
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

impl AiConfig {
    /// Overlay `other` on top of `self` — any `Some` field in `other` wins.
    pub fn overlay(&mut self, other: Self) {
        if other.per_app_daily_usd.is_some() {
            self.per_app_daily_usd = other.per_app_daily_usd;
        }
        if other.global_daily_usd.is_some() {
            self.global_daily_usd = other.global_daily_usd;
        }
        if other.client.as_ref().is_some_and(|client| !client.trim().is_empty()) {
            self.client = other.client;
        }
    }
}

/// Startup restoration policy for saved agent sessions (`[agents] restore_mode`).
#[derive(Deserialize, Debug, Default, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AgentRestoreMode {
    /// Active-context agent panes with a session ID auto-resume; background and
    /// sub-context agent panes restore into lightweight Standby until focused,
    /// pressed Enter, or sent input.
    #[default]
    Lazy,
    /// All restored agent panes with a session ID auto-resume immediately on startup.
    Eager,
    /// Agent panes restore as plain terminal shells without auto-resuming.
    Off,
}

/// Coding agent command templates for dispatch. Each field is a shell command template
/// where `{cmd}` is replaced with the prompt or slash command at dispatch time.
#[derive(Deserialize, Default, Clone)]
pub struct AgentsConfig {
    /// Raise silent macOS attention notifications while Plexi is unfocused (default true).
    /// Uses provisional authorization without a permission dialog; enable banners in macOS
    /// notification settings if desired. Disabled in tests; other platforms log only.
    pub os_notifications: Option<bool>,
    /// Idle ephemeral worker stall threshold in seconds (default 120).
    pub stalled_after_secs: Option<u64>,
    /// Show agent jump rows in the command palette (default false).
    pub palette_agent_results: Option<bool>,
    /// Fast/cheap tasks. Default: claude haiku with --dangerously-skip-permissions.
    pub low: Option<String>,
    /// Standard work. Default: claude sonnet with --dangerously-skip-permissions.
    pub medium: Option<String>,
    /// Complex/autonomous tasks. Default: claude with --dangerously-skip-permissions.
    pub high: Option<String>,
    /// Startup restoration mode for saved agent sessions: `"lazy"` (default), `"eager"`, or `"off"`.
    pub restore_mode: Option<AgentRestoreMode>,
}

impl AgentsConfig {
    fn overlay(&mut self, other: Self) {
        if other.os_notifications.is_some() { self.os_notifications = other.os_notifications; }
        if other.stalled_after_secs.is_some() { self.stalled_after_secs = other.stalled_after_secs; }
        if other.palette_agent_results.is_some() { self.palette_agent_results = other.palette_agent_results; }
        if other.low.is_some() {
            self.low = other.low;
        }
        if other.medium.is_some() {
            self.medium = other.medium;
        }
        if other.high.is_some() {
            self.high = other.high;
        }
        if other.restore_mode.is_some() {
            self.restore_mode = other.restore_mode;
        }
    }
}

#[derive(Deserialize, Default, Clone)]
pub struct NotificationsConfig {
    /// Master switch. If false, incoming notifications are silently dropped —
    /// apps still send them, but the modal never appears and the queue stays
    /// empty. Defaults to true.
    pub enabled: Option<bool>,
    /// Focus mode. When true, no notification auto-surfaces. Everything queues
    /// silently; the user reviews via Cmd+Shift+A.
    /// Defaults to false.
    pub focus_mode: Option<bool>,
    /// Path to an audio file (WAV, MP3, FLAC, OGG) played once when a
    /// notification arrives, so an agent waiting on input can pull a human
    /// back while Plexi is in the background. Unset (the default) means no
    /// sound; an explicitly empty string also means no sound, so a workspace
    /// overlay can set `sound = ""` to silence a globally configured cue.
    /// The cue only fires for a notification that may interrupt: `enabled =
    /// false` drops it before the cue, `focus_mode = true` suppresses it, and
    /// an invisible notification arrives silently.
    pub sound: Option<String>,
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
pub struct EffectsConfig {
    pub crt: Option<bool>,
    pub ghost: Option<bool>,
    /// Opacity for unfocused panes when ghost is enabled. Range 0.0–1.0; default 0.75.
    /// Setting this implies ghost = true unless `ghost = false` is explicit.
    pub ghost_opacity: Option<f32>,
}

#[derive(Deserialize, Debug, Default, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum AttentionStyle { Off, Subtle, #[default] Normal, Loud }

#[derive(Deserialize, Debug, Default, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum AttentionDock { None, #[default] Badge, BounceOnce, BounceUntilFocused }

#[derive(Deserialize, Debug, Default, Clone, Copy, PartialEq)]
pub struct AttentionConfig {
    /// Attention style: off, subtle, normal (default), or loud.
    pub style: Option<AttentionStyle>,
    /// Halo and border opacity multiplier, 0.0 to 1.0. Default: 1.0.
    #[serde(default, deserialize_with = "attention_intensity")]
    pub intensity: Option<f32>,
    /// Dock delivery: none, badge (default), bounce-once, bounce-until-focused.
    pub dock: Option<AttentionDock>,
}

fn attention_intensity<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<f32>, D::Error> {
    let value = Option::<f32>::deserialize(d)?;
    if value.is_some_and(|v| !v.is_finite() || !(0.0..=1.0).contains(&v)) {
        return Err(serde::de::Error::custom("attention.intensity must be between 0 and 1"));
    }
    Ok(value)
}

impl AttentionConfig {
    fn overlay(&mut self, other: Self) {
        if other.style.is_some() { self.style = other.style; }
        if other.intensity.is_some() { self.intensity = other.intensity; }
        if other.dock.is_some() { self.dock = other.dock; }
    }
}

fn state_preset<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let value = Option::<String>::deserialize(d)?;
    if value.as_deref().is_some_and(|v| !crate::ui::theme::STATE_PRESETS.contains(&v)) {
        return Err(serde::de::Error::custom("theme.state_preset must be default, okabe-ito, high-contrast, or mono"));
    }
    Ok(value)
}

#[derive(Deserialize, Default, Clone)]
pub struct ThemeConfig {
    /// Theme preset name. Applied first; individual color fields override the preset.
    pub preset: Option<String>,
    /// Agent states: default, okabe-ito, high-contrast, mono. Default: okabe-ito.
    #[serde(default, deserialize_with = "state_preset")]
    pub state_preset: Option<String>,
    pub state_working: Option<String>,
    pub state_waiting: Option<String>,
    pub state_blocked: Option<String>,
    pub state_needs_you: Option<String>,
    pub state_interrupted: Option<String>,
    pub state_error: Option<String>,

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
    // Activity pip color overrides — optional; fall back to success/warning/danger.
    pub pip_working: Option<String>,
    pub pip_idle: Option<String>,
    pub pip_blocked: Option<String>,
    // Opacity multiplier for unfocused pips. Default 0.72.
    pub pip_dim: Option<f32>,
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

/// Read-only freshness check shared by both sides of the initialization lock.
fn profile_initialization_needed() -> Result<bool, String> {
    let dir = config_dir();
    let sdk = dir.join("sdk");
    Ok(!dir.join("apps").is_dir()
        || !sdk.join("plexi_sdk").is_dir()
        || !sdk.join("pyproject.toml").is_file()
        || !std::fs::read_to_string(sdk.join(".sdk_version"))
            .is_ok_and(|version| version.trim() == env!("PLEXI_BUILD_ID"))
        || crate::cli::install_host::core_pack_needs_seeding(&dir.join("apps"))
        || !crate::cli::install_host::orphaned_pre_v3_first_party_apps(&dir.join("apps")).is_empty()
        || crate::distribution::profile_needs_seeding()?)
}

/// Fresh profiles need no writable files. Re-check after acquiring the lock:
/// another process may have completed initialization while we waited.
pub fn lock_profile_initialization() -> Result<Option<std::fs::File>, String> {
    if !profile_initialization_needed()? {
        return Ok(None);
    }
    let dir = config_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let lock = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false)
        .open(dir.join("initialization.lock")).map_err(|e| format!("open profile initialization lock: {e}"))?;
    lock.lock().map_err(|e| format!("lock profile initialization: {e}"))?;
    if !profile_initialization_needed()? {
        return Ok(None);
    }
    Ok(Some(lock))
}

/// Ensure the apps directory and embedded SDK exist. Caller holds the initialization lock.
/// Returns whether this is the first launch (no apps directory yet).
pub fn ensure_profile_initialized() -> bool {
    let dir = config_dir();
    // Install metadata may have created the profile before the first app launch.
    let is_new = !dir.join("apps").is_dir();
    if let Err(e) = std::fs::create_dir_all(dir.join("apps")) {
        eprintln!("profile init: failed to create apps directory: {e}");
        return false;
    }

    // Only re-extract when the embedded SDK version differs from what was last written.
    // This preserves manual edits and symlinks placed in the profile SDK dir during
    // normal launches while still ensuring upgrades land the correct version.
    let sdk_dir = dir.join("sdk");
    let sdk_dest = sdk_dir.join("plexi_sdk");
    let stamp_path = sdk_dir.join(".sdk_version");
    let current_version = env!("PLEXI_BUILD_ID");
    let needs_extract = !sdk_dest.exists()
        || !sdk_dir.join("pyproject.toml").is_file()
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
                if let Err(e) = std::fs::write(
                    sdk_dir.join("pyproject.toml"),
                    include_str!("../../sdk/python/pyproject.toml"),
                ) {
                    eprintln!("profile init: failed to seed SDK metadata: {e}");
                } else {
                    let _ = std::fs::write(&stamp_path, current_version);
                    log::info!(
                        "profile init: seeded SDK v{current_version} to {}",
                        sdk_dest.display()
                    );
                }
            }
        }
    } else {
        log::info!("profile init: SDK v{current_version} up to date, skipping extract");
    }

    is_new
}

/// Returns the build channel for this binary: the suffix after `plexi-`, or `None` for the bare `plexi` binary (main channel).
pub fn build_channel() -> Option<String> {
    #[cfg(test)]
    if let Some(channel) = test_channel_override() {
        return Some(channel);
    }
    channel_from_basename(&current_exe_basename()).map(str::to_string)
}

/// True when this binary was compiled for, and is currently running as, the
/// same disposable per-PR test channel (`pr-<N>`).
///
/// This is the *only* sanctioned way to ask "is this a throwaway test build?".
/// Runtime naming alone is not trusted: renaming a production binary to
/// `plexi-pr-1` must not unlock test-only behavior. `scripts/install.sh` embeds
/// the marker only while building an all-digit PR channel.
pub fn is_test_channel(channel: Option<&str>) -> bool {
    is_matching_test_channel(option_env!("PLEXI_BUILD_TEST_CHANNEL"), channel)
}

fn is_matching_test_channel(compiled: Option<&str>, runtime: Option<&str>) -> bool {
    let Some(compiled) = compiled else {
        return false;
    };
    compiled == runtime.unwrap_or_default()
        && compiled
            .strip_prefix("pr-")
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// Strip the Windows executable suffix before deriving a Plexi channel.
///
/// `current_exe()` includes `.exe` on Windows. Channel names are part of
/// persistent profile and named-pipe identities, so `plexi-alpha.exe` must
/// name the same channel as the Unix/macOS `plexi-alpha` binary. Accept the
/// suffix on every platform so this pure naming rule stays testable without a
/// Windows-only test build.
fn channel_binary_basename(basename: &str) -> &str {
    basename
        .strip_suffix(".exe")
        .or_else(|| basename.strip_suffix(".EXE"))
        .unwrap_or(basename)
}

/// Maps a binary basename to its config directory name.
/// `plexi` → `.plexi`; `plexi-<suffix>` → `.plexi-<suffix>`.
fn channel_suffix_from_basename(basename: &str) -> String {
    let basename = channel_binary_basename(basename);
    if let Some(suffix) = basename.strip_prefix("plexi-").filter(|s| !s.is_empty()) {
        format!(".plexi-{suffix}")
    } else {
        ".plexi".to_string()
    }
}

/// The channel named by the running binary itself, if any: `plexi-alpha` →
/// `Some("alpha")`, `plexi` → `None`.
pub(crate) fn channel_from_basename(basename: &str) -> Option<&str> {
    channel_binary_basename(basename)
        .strip_prefix("plexi-")
        .filter(|s| !s.is_empty())
}

/// Resolve the profile dir name (`.plexi`, `.plexi-alpha`, …) from the running
/// binary's basename and the ambient `PLEXI_CHANNEL`.
///
/// A channel-named binary is an *explicit address*: `plexi-alpha` always means
/// the alpha profile, even when invoked from inside a pane of another channel,
/// whose `PLEXI_CHANNEL` is inherited through the environment. Only the bare
/// `plexi` binary carries no channel of its own, so it adopts the surrounding
/// pane's `PLEXI_CHANNEL` when one is set (#2109) and falls back to `.plexi`
/// outside any pane.
///
/// See `src/cli/AGENTS.md` ("Environment classification (`self-identity` vs
/// `target-routing`)") for the canonical precedence table. Self-identity env
/// vars (`PLEXI_PANE_ID`, `PLEXI_CONTEXT_*`, `PLEXI_RUNNING`) are never
/// consulted here.
pub(crate) fn resolve_channel_dir(basename: &str, env_channel: Option<&str>) -> String {
    if channel_from_basename(basename).is_some() {
        return channel_suffix_from_basename(basename);
    }
    match env_channel.filter(|c| !c.is_empty()) {
        Some(channel) => format!(".plexi-{channel}"),
        None => channel_suffix_from_basename(basename),
    }
}

/// Basename of the running binary, defaulting to `plexi` when it cannot be read.
/// The single source for "what am I called?" — every channel, profile, and
/// self-update decision derives from this, never from its own `current_exe()`
/// call.
pub fn current_exe_basename() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "plexi".to_string())
}

/// Ambient `PLEXI_CHANNEL`, or `None` when unset or empty. Always `None` under
/// `cfg(test)`: unit tests must never resolve state from the channel of the
/// pane running them (tests use `set_test_channel`/`set_test_profile_dir`).
fn env_channel() -> Option<String> {
    #[cfg(test)]
    {
        None
    }
    #[cfg(not(test))]
    {
        std::env::var("PLEXI_CHANNEL")
            .ok()
            .filter(|c| !c.is_empty())
    }
}

/// The profile dir name for this process, resolved once and cached — both
/// inputs (binary basename, `PLEXI_CHANNEL`) are fixed for the process lifetime.
fn process_channel_dir() -> String {
    static CHANNEL_DIR: OnceLock<String> = OnceLock::new();
    CHANNEL_DIR
        .get_or_init(|| {
            let basename = current_exe_basename();
            let env = env_channel();
            let dir = resolve_channel_dir(&basename, env.as_deref());
            log::info!("channel dir resolved to {dir} (binary {basename}, PLEXI_CHANNEL={env:?})");
            dir
        })
        .clone()
}

#[cfg(test)]
thread_local! {
    /// Per-thread channel override for tests. Tests must use `set_test_channel`
    /// instead of mutating PLEXI_CHANNEL — env mutation races with parallel
    /// tests that read channel dirs.
    static TEST_CHANNEL_OVERRIDE: std::cell::RefCell<Option<String>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn test_channel_override() -> Option<String> {
    TEST_CHANNEL_OVERRIDE.with(|c| c.borrow().clone())
}

/// RAII guard that clears the per-thread test channel override on drop.
#[cfg(test)]
pub struct TestChannelGuard;

#[cfg(test)]
impl Drop for TestChannelGuard {
    fn drop(&mut self) {
        TEST_CHANNEL_OVERRIDE.with(|c| *c.borrow_mut() = None);
    }
}

/// Override the channel for the current test thread. Hold the returned guard
/// for the duration of the test.
#[cfg(test)]
pub fn set_test_channel(channel: &str) -> TestChannelGuard {
    TEST_CHANNEL_OVERRIDE.with(|c| *c.borrow_mut() = Some(channel.to_string()));
    TestChannelGuard
}

#[cfg(test)]
static NEXT_TEST_DIR_GUARD_TOKEN: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(1);

#[cfg(test)]
thread_local! {
    static TEST_PROFILE_DIR_OVERRIDE: std::cell::RefCell<Vec<(u64, std::path::PathBuf)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// RAII guard that removes its per-thread test profile dir override on drop.
#[cfg(test)]
pub struct TestProfileDirGuard {
    token: u64,
}

#[cfg(test)]
impl Drop for TestProfileDirGuard {
    fn drop(&mut self) {
        let token = self.token;
        TEST_PROFILE_DIR_OVERRIDE.with(|c| c.borrow_mut().retain(|(t, _)| *t != token));
    }
}

/// Override the full profile directory for the current test thread. The returned
/// guard must be held for the duration of the test; dropping it removes this
/// guard's override while preserving any outer or sibling live guard on the thread.
///
/// Prefer this over `set_test_channel` for new tests — it redirects `config_dir()`
/// and `config_path()` to a fully isolated tempdir rather than a subdir of $HOME.
#[cfg(test)]
pub fn set_test_profile_dir(path: std::path::PathBuf) -> TestProfileDirGuard {
    let token = NEXT_TEST_DIR_GUARD_TOKEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    TEST_PROFILE_DIR_OVERRIDE.with(|c| c.borrow_mut().push((token, path)));
    TestProfileDirGuard { token }
}

/// The isolated profile directory installed by [`set_test_profile_dir`], if any.
///
/// Notes-index persistence consults this instead of `config_dir()` so a test
/// that never installed an override cannot read or write a real channel profile
/// (`config_dir()` asserts when the channel dir is a real profile name).
#[cfg(test)]
pub(crate) fn test_profile_dir_override() -> Option<std::path::PathBuf> {
    TEST_PROFILE_DIR_OVERRIDE.with(|c| c.borrow().last().map(|(_, dir)| dir.clone()))
}

/// Returns the workspace channel directory name for the current binary.
/// This is the dot-prefixed dir used inside workspace roots to scope
/// workspace state per channel: `.plexi` (main), `.plexi-alpha`, `.plexi-beta`,
/// `.plexi-pr-N`, etc. This is the single source of truth for workspace
/// channel dirs — never derive it independently elsewhere.
pub fn workspace_channel_dir() -> String {
    #[cfg(test)]
    if let Some(channel) = test_channel_override() {
        return format!(".plexi-{channel}");
    }
    if let Some(Some(profile)) = PROFILE_OVERRIDE.get() {
        return format!(".plexi-{profile}");
    }
    let channel_dir = process_channel_dir();
    #[cfg(test)]
    assert_test_profile_is_isolated(&channel_dir);
    channel_dir
}

/// Returns the config directory name for this process. Same resolution as
/// [`workspace_channel_dir`] — the profile dir and the workspace channel dir
/// are always the same channel.
fn config_dir_name() -> String {
    workspace_channel_dir()
}

#[cfg(test)]
fn assert_test_profile_is_isolated(dir: &str) {
    let is_real_profile = matches!(
        dir,
        ".plexi" | ".plexi-alpha" | ".plexi-beta" | ".plexi-stable"
    ) || dir
        .strip_prefix(".plexi-pr-")
        .is_some_and(|suffix| !suffix.is_empty());
    assert!(
        !is_real_profile,
        "test resolved config/workspace state to real channel profile dir {dir:?}; use set_test_profile_dir for isolated test state"
    );
}

/// The enabled experiments from the global config, read-only (never writes the
/// default template). A missing file means nothing is enabled; an unknown
/// experiment name or a non-boolean value is an error.
pub fn load_global_experiments(
) -> Result<std::collections::BTreeSet<crate::release::ReleaseFeature>, String> {
    let path = config_path();
    experiments_from_path(&path)
}

pub fn experiments_from_path(
    path: &Path,
) -> Result<std::collections::BTreeSet<crate::release::ReleaseFeature>, String> {
    let data = match std::fs::read_to_string(path) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Default::default()),
        Err(e) => return Err(format!("could not read {}: {e}", path.display())),
    };
    // Parse as a bare value so an unrelated config error elsewhere in the file
    // cannot silently drop the experiment switches.
    let value: toml::Value = match toml::from_str(&data) {
        Ok(v) => v,
        Err(e) => {
            log::warn!(
                "experiments: {} does not parse ({e}); all experiments stay off",
                path.display()
            );
            return Ok(Default::default());
        }
    };
    let Some(table) = value.get("experiments") else {
        return Ok(Default::default());
    };
    let table: std::collections::BTreeMap<String, bool> = table
        .clone()
        .try_into()
        .map_err(|e| format!("[experiments] in {} must map names to true/false: {e}", path.display()))?;
    crate::release::parse_experiments(&table)
        .map_err(|e| format!("{}: {e}", path.display()))
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

pub fn config_dir() -> PathBuf {
    #[cfg(test)]
    {
        if let Some(dir) = test_profile_dir_override() {
            return dir;
        }
        #[cfg(unix)]
        let base = PathBuf::from("/tmp");
        #[cfg(not(unix))]
        let base = std::env::temp_dir();
        let fallback = base.join(format!("plexi-t-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&fallback);
        return fallback;
    }
    #[cfg(not(test))]
    {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(config_dir_name())
    }
}

/// Channel-neutral state directory, always `~/.plexi` regardless of which
/// channel's binary is running. For state that is global to the machine rather
/// than scoped to a release channel — anything a third party reads by absolute
/// path, where the last channel to write must not own it. Everything else
/// belongs in `config_dir()`.
pub fn shared_dir() -> PathBuf {
    #[cfg(test)]
    {
        let override_dir =
            TEST_SHARED_DIR_OVERRIDE.with(|c| c.borrow().last().map(|(_, dir)| dir.clone()));
        if let Some(dir) = override_dir {
            return dir;
        }
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(SHARED_DIR_NAME)
}

const SHARED_DIR_NAME: &str = ".plexi";

#[cfg(test)]
thread_local! {
    static TEST_SHARED_DIR_OVERRIDE: std::cell::RefCell<Vec<(u64, std::path::PathBuf)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// RAII guard that removes its per-thread test shared dir override on drop.
#[cfg(test)]
pub struct TestSharedDirGuard {
    token: u64,
}

#[cfg(test)]
impl Drop for TestSharedDirGuard {
    fn drop(&mut self) {
        let token = self.token;
        TEST_SHARED_DIR_OVERRIDE.with(|c| c.borrow_mut().retain(|(t, _)| *t != token));
    }
}

/// Redirect `shared_dir()` to an isolated tempdir for the current test thread.
/// The returned guard must be held for the duration of the test.
#[cfg(test)]
pub fn set_test_shared_dir(path: std::path::PathBuf) -> TestSharedDirGuard {
    let token = NEXT_TEST_DIR_GUARD_TOKEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    TEST_SHARED_DIR_OVERRIDE.with(|c| c.borrow_mut().push((token, path)));
    TestSharedDirGuard { token }
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
/// WASM launch and static render paths call this so the
/// priority is defined in exactly one place:
///   `PLEXI_SDK_PATH` override → debug checkout SDK → config-dir SDK → bundle SDK (when present)
pub fn build_pythonpath(bundle_sdk: Option<&std::path::Path>) -> String {
    let mut entries = Vec::new();
    if let Some(dev_sdk) = sdk_path_override() {
        entries.push(dev_sdk);
    }
    if let Some(source_sdk) = source_sdk_path() {
        entries.push(source_sdk);
    }
    let sdk_dir = config_dir().join("sdk");
    entries.push(sdk_dir);
    if let Some(bsdk) = bundle_sdk.filter(|p| p.exists()) {
        entries.push(bsdk.to_path_buf());
    }
    std::env::join_paths(entries)
        .expect("SDK paths must not contain the platform path separator")
        .to_string_lossy()
        .into_owned()
}

fn source_sdk_path() -> Option<PathBuf> {
    #[cfg(debug_assertions)]
    {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("sdk/python");
        if path.join("plexi_sdk").join("__init__.py").is_file() {
            return Some(path);
        }
    }
    None
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

/// The global config path, with its parent and a commented template written
/// on first use. The host opens the returned path in a Plexi text-editor pane.
pub fn ensure_config_file() -> PathBuf {
    ensure_config_file_at(config_path())
}

/// The workspace config path for `workspace_root`, with its parent and a
/// commented template written on first use. Global and workspace entries share
/// the same editor handoff.
pub fn ensure_workspace_config_file(workspace_root: &Path) -> PathBuf {
    ensure_config_file_at(workspace_config_path(workspace_root))
}

fn ensure_config_file_at(path: PathBuf) -> PathBuf {
    if !path.exists() {
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                log::error!(
                    "ensure_config_file: could not create {}: {e}",
                    parent.display()
                );
            }
        }
        if let Err(e) = std::fs::write(&path, CONFIG_TEMPLATE) {
            log::error!(
                "ensure_config_file: could not write template to {}: {e}",
                path.display()
            );
        }
    }
    path
}

/// Opens `path` with a fallback chain: VS Code → system default → TextEdit.
/// Returns `true` if any opener succeeded. Used by `plexi config edit` when
/// `$EDITOR` is unset; the in-host config entry points use the Plexi editor.
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

/// Linux opener chain: VS Code → the XDG default handler. `xdg-open` is the
/// desktop-agnostic entry point every Linux DE honours, so there is no third
/// candidate worth adding — a machine with no XDG handler has no configured
/// editor to fall back to, and the caller reports that honestly.
#[cfg(target_os = "linux")]
pub fn open_file_with_fallback(path: &std::path::Path) -> bool {
    for opener in ["code", "xdg-open"] {
        match std::process::Command::new(opener)
            .arg(path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
        {
            Ok(status) if status.success() => {
                log::info!(
                    "open_file_with_fallback: opened {} with {opener}",
                    path.display()
                );
                return true;
            }
            Ok(status) => log::warn!(
                "open_file_with_fallback: {opener} exited {status:?} for {}, trying next",
                path.display()
            ),
            Err(e) => log::warn!(
                "open_file_with_fallback: {opener} unavailable ({e}) for {}, trying next",
                path.display()
            ),
        }
    }
    log::warn!(
        "open_file_with_fallback: no opener succeeded for {}",
        path.display()
    );
    false
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn open_file_with_fallback(path: &std::path::Path) -> bool {
    log::warn!(
        "open_file_with_fallback: no platform implementation for {}",
        path.display()
    );
    false
}

impl PlexiConfig {
    /// Resolve pi-plexi FAST/SLOW targets with defaults applied field-by-field.
    pub fn resolved_model_modes(&self) -> ResolvedModelModes {
        self.models.as_ref().map(ModelModesConfig::resolved).unwrap_or_else(|| ModelModesConfig::default().resolved())
    }
    pub fn osc_pane_title_enabled(&self) -> bool {
        self.osc_pane_title.unwrap_or(true)
    }

    pub fn soft_grant_days(&self) -> u32 {
        self.permissions
            .as_ref()
            .map(PermissionsConfig::effective_soft_grant_days)
            .unwrap_or(DEFAULT_SOFT_GRANT_DAYS)
    }

    pub fn agent_restore_mode(&self) -> AgentRestoreMode {
        self.agents
            .as_ref()
            .and_then(|a| a.restore_mode)
            .unwrap_or_default()
    }

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
        if let Some(error) = legacy_ai_error(&data) {
            log::warn!("Failed to validate config file {}: {error}", path.display());
            return Some(Self::default());
        }
        match toml::from_str::<Self>(&data) {
            Ok(cfg) => Some(cfg),
            Err(e) => {
                log::warn!("Failed to parse config file {}: {e}", path.display());
                Some(Self::default())
            }
        }
    }

    /// CLI-only strict parser used by machine-readable config access.
    pub(crate) fn load_from_path_for_cli(path: &Path) -> Result<Option<Self>, String> {
        match std::fs::read_to_string(path) {
            Ok(data) => {
                if let Some(error) = legacy_ai_error(&data) {
                    Err(error)
                } else {
                    toml::from_str(&data).map(Some).map_err(|e| e.to_string())
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
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
    /// (theme, effects, log, notifications) are overlaid recursively.
    fn overlay(&mut self, other: Self) {
        if let Some(attention) = other.attention {
            self.attention.get_or_insert_with(Default::default).overlay(attention);
        }
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
        if other.confirm_context_close.is_some() {
            self.confirm_context_close = other.confirm_context_close;
        }
        if other.pane_gap.is_some() {
            self.pane_gap = other.pane_gap;
        }
        if other.pane_title_font_size.is_some() {
            self.pane_title_font_size = other.pane_title_font_size;
        }
        if other.osc_pane_title.is_some() {
            self.osc_pane_title = other.osc_pane_title;
        }
        match (self.theme.as_mut(), other.theme) {
            (Some(existing), Some(incoming)) => existing.overlay(incoming),
            (None, Some(incoming)) => self.theme = Some(incoming),
            _ => {}
        }
        match (self.effects.as_mut(), other.effects) {
            (Some(existing), Some(incoming)) => existing.overlay(incoming),
            (None, Some(incoming)) => self.effects = Some(incoming),
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
        match (self.models.as_mut(), other.models) {
            (Some(existing), Some(incoming)) => existing.overlay(incoming),
            (None, Some(incoming)) => self.models = Some(incoming),
            _ => {}
        }
        match (self.keybindings.as_mut(), other.keybindings) {
            (Some(existing), Some(incoming)) => existing.overlay(incoming),
            (None, Some(incoming)) => self.keybindings = Some(incoming),
            _ => {}
        }
        match (self.keys.as_mut(), other.keys) {
            (Some(existing), Some(incoming)) => existing.extend(incoming),
            (None, Some(incoming)) => self.keys = Some(incoming),
            _ => {}
        }
        match (self.agents.as_mut(), other.agents) {
            (Some(existing), Some(incoming)) => existing.overlay(incoming),
            (None, Some(incoming)) => self.agents = Some(incoming),
            _ => {}
        }
        match (self.marketplace.as_mut(), other.marketplace) {
            (Some(existing), Some(incoming)) => existing.overlay(incoming),
            (None, Some(incoming)) => self.marketplace = Some(incoming),
            _ => {}
        }
        match (self.cloud.as_mut(), other.cloud) {
            (Some(existing), Some(incoming)) => existing.overlay(incoming),
            (None, Some(incoming)) => self.cloud = Some(incoming),
            _ => {}
        }
        match (self.permissions.as_mut(), other.permissions) {
            (Some(existing), Some(incoming)) => existing.overlay(incoming),
            (None, Some(incoming)) => self.permissions = Some(incoming),
            _ => {}
        }
    }

    pub(crate) fn overlay_for_cli(&mut self, other: Self) { self.overlay(other); }
}

impl CloudConfig {
    fn overlay(&mut self, other: Self) {
        if other.retain_local_history.is_some() {
            self.retain_local_history = other.retain_local_history;
        }
    }

    /// Local ledger and conversation prune. Unset means keep local history.
    pub fn retain_local_history(&self) -> bool {
        self.retain_local_history.unwrap_or(false)
    }
}

impl MarketplaceConfig {
    fn overlay(&mut self, other: Self) {
        macro_rules! overlay_field {
            ($field:ident) => {
                if other.$field.is_some() {
                    self.$field = other.$field;
                }
            };
        }
        overlay_field!(registry_url);
        overlay_field!(cdn_url);
        overlay_field!(submit_url);
        overlay_field!(account_backend);
        overlay_field!(account_url);
        overlay_field!(account_email);
    }
}

/// Resolved marketplace settings for the active channel. Loads the global
/// config (workspace overlay does not apply to marketplace endpoints — they are
/// machine-level, not project-level). Returns the configured values, falling
/// back to in-code defaults at the call site.
fn marketplace_config() -> MarketplaceConfig {
    PlexiConfig::load().marketplace.unwrap_or_default()
}

/// Catalog index URL override, if the user set one.
pub fn marketplace_registry_url() -> Option<String> {
    marketplace_config().registry_url
}

/// Package CDN base override, if set.
pub fn marketplace_cdn_url() -> Option<String> {
    marketplace_config().cdn_url
}

/// Publisher submission endpoint, if configured. `None` = publishing fails
/// closed (prepared but not uploaded).
pub fn marketplace_submit_url() -> Option<String> {
    marketplace_config().submit_url
}

/// Account/auth backend selector (e.g. `"none"`, `"plexi"`). `None` = stub.
pub fn marketplace_account_backend() -> Option<String> {
    marketplace_config().account_backend
}

/// Accounts service base URL, defaulting to the product domain at the call site.
pub fn marketplace_account_url() -> String {
    marketplace_config()
        .account_url
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| crate::app::account::DEFAULT_ACCOUNT_URL.to_string())
}

/// Default email pre-filled by `plexi account login`, if set.
pub fn marketplace_account_email() -> Option<String> {
    marketplace_config().account_email
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
        overlay_field!(preset);
        overlay_field!(state_preset);
        overlay_field!(state_working);
        overlay_field!(state_waiting);
        overlay_field!(state_blocked);
        overlay_field!(state_needs_you);
        overlay_field!(state_interrupted);
        overlay_field!(state_error);

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
        overlay_field!(pip_working);
        overlay_field!(pip_idle);
        overlay_field!(pip_blocked);
        overlay_field!(pip_dim);
    }
}

impl EffectsConfig {
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
    }

    /// Resolved unfocused-pane opacity.
    /// `ghost = false` disables; otherwise ghost defaults on at 0.75 opacity.
    pub fn unfocused_opacity(&self) -> Option<f32> {
        if self.ghost == Some(false) {
            None
        } else {
            Some(self.ghost_opacity.unwrap_or(0.75).clamp(0.0, 1.0))
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
        if other.sound.is_some() {
            self.sound = other.sound;
        }
    }

    /// The configured arrival-cue path, with `sound = ""` normalized to unset.
    /// An explicitly empty value means "no sound" — never "open the empty path
    /// and log a playback error on every notification". Every reader of the
    /// cue path resolves it through here; nothing reads `sound` directly.
    pub fn cue_sound(&self) -> Option<String> {
        self.sound.clone().filter(|s| !s.is_empty())
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

#[cfg(any())]
mod tests {
    use super::*;
    use std::fs;

    /// Fix round 1 defect 3: a workspace overlay setting `[notifications]
    /// sound` must override the global value, not be silently discarded.
    #[test]
    fn notifications_overlay_carries_sound() {
        let mut base = NotificationsConfig {
            sound: Some("/global/cue.wav".to_string()),
            ..Default::default()
        };
        base.overlay(NotificationsConfig {
            sound: Some("/workspace/cue.wav".to_string()),
            ..Default::default()
        });
        assert_eq!(base.sound.as_deref(), Some("/workspace/cue.wav"));

        // An overlay that says nothing about sound keeps the global value.
        base.overlay(NotificationsConfig::default());
        assert_eq!(base.sound.as_deref(), Some("/workspace/cue.wav"));
    }

    /// `sound = ""` means "no sound" — never a `File::open("")` error logged
    /// on every notification. A workspace overlay uses it to silence a
    /// globally configured cue.
    #[test]
    fn empty_sound_normalizes_to_unset() {
        let cfg = NotificationsConfig {
            sound: Some(String::new()),
            ..Default::default()
        };
        assert_eq!(cfg.cue_sound(), None);

        let cfg = NotificationsConfig {
            sound: Some("/tmp/cue.wav".to_string()),
            ..Default::default()
        };
        assert_eq!(cfg.cue_sound().as_deref(), Some("/tmp/cue.wav"));
        assert_eq!(NotificationsConfig::default().cue_sound(), None);
    }

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
    fn windows_executable_suffix_does_not_become_part_of_the_channel() {
        assert_eq!(channel_suffix_from_basename("plexi-alpha.exe"), ".plexi-alpha");
        assert_eq!(channel_from_basename("plexi-alpha.exe"), Some("alpha"));
        assert_eq!(
            resolve_channel_dir("plexi-alpha.exe", Some("beta")),
            ".plexi-alpha"
        );
    }

    #[test]
    fn only_numbered_pr_channels_are_test_channels() {
        assert!(is_matching_test_channel(Some("pr-1"), Some("pr-1")));
        assert!(is_matching_test_channel(
            Some("pr-2493"),
            Some("pr-2493")
        ));

        for (compiled, runtime) in [
            (None, Some("pr-1")),
            (Some("pr-1"), None),
            (Some("pr-1"), Some("main")),
            (Some("pr-1"), Some("alpha")),
            (Some("pr-1"), Some("beta")),
            (Some("pr-1"), Some("pr-2")),
            (Some("pr-"), Some("pr-")),
            (Some("pr-evil"), Some("pr-evil")),
            (Some("pr-12a"), Some("pr-12a")),
            (Some("PR-1"), Some("PR-1")),
        ] {
            assert!(
                !is_matching_test_channel(compiled, runtime),
                "compiled={compiled:?}, runtime={runtime:?} must not be a test channel"
            );
        }
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

    #[test]
    fn config_dir_name_respects_test_channel_override() {
        let _guard = set_test_channel("pr-999");
        assert_eq!(config_dir_name(), ".plexi-pr-999");
    }

    #[test]
    fn workspace_channel_dir_respects_test_channel_override() {
        let _guard = set_test_channel("pr-999");
        assert_eq!(workspace_channel_dir(), ".plexi-pr-999");
    }

    #[test]
    fn ambient_channel_does_not_select_a_real_profile_in_tests_child() {
        if std::env::var_os("TEST_AMBIENT_CHANNEL_CHILD").is_none() {
            return;
        }
        assert_ne!(workspace_channel_dir(), ".plexi-beta");
        assert_ne!(config_dir_name(), ".plexi-beta");
    }

    #[test]
    fn ambient_channel_does_not_select_a_real_profile_in_tests() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "config::tests::ambient_channel_does_not_select_a_real_profile_in_tests_child",
            ])
            .env("PLEXI_CHANNEL", "beta")
            .env("TEST_AMBIENT_CHANNEL_CHILD", "1")
            .status()
            .unwrap();
        assert!(status.success());
    }

    #[test]
    fn channel_named_binary_wins_over_ambient_channel() {
        // A `plexi-alpha` shim invoked from inside a beta pane addresses alpha.
        assert_eq!(
            resolve_channel_dir("plexi-alpha", Some("beta")),
            ".plexi-alpha"
        );
        assert_eq!(
            resolve_channel_dir("plexi-pr-42", Some("beta")),
            ".plexi-pr-42"
        );
        assert_eq!(
            resolve_channel_dir("plexi-beta", Some("alpha")),
            ".plexi-beta"
        );
    }

    #[test]
    fn bare_binary_adopts_ambient_channel() {
        // The bare binary names no channel of its own, so the pane's channel
        // decides which profile it talks to (#2109).
        assert_eq!(
            resolve_channel_dir("plexi", Some("pr-999")),
            ".plexi-pr-999"
        );
        assert_eq!(resolve_channel_dir("plexi", Some("beta")), ".plexi-beta");
    }

    #[test]
    fn bare_binary_outside_a_pane_uses_stable_profile() {
        assert_eq!(resolve_channel_dir("plexi", None), ".plexi");
        assert_eq!(resolve_channel_dir("plexi", Some("")), ".plexi");
    }

    #[test]
    #[should_panic(expected = "real channel profile dir \".plexi-beta\"")]
    fn isolated_profile_guard_names_real_profile_dir() {
        assert_test_profile_is_isolated(".plexi-beta");
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
        fs::write(&path, "font_size = 14.0\n[theme]\npreset = \"dracula\"\n").unwrap();
        let diags = validate_from_path(&path);
        assert!(diags.is_empty(), "expected no diagnostics, got: {diags:?}");
    }

    #[test]
    fn model_modes_resolve_defaults_and_partial_overrides() {
        let defaults = PlexiConfig::default().resolved_model_modes();
        assert_eq!(defaults.fast.model, "deepseek/deepseek-v4.1-flash");
        assert_eq!(defaults.fast.thinking, "low");
        let cfg: PlexiConfig = toml::from_str("[models.fast]\nmodel = \"custom/flash\"\n").unwrap();
        let modes = cfg.resolved_model_modes();
        assert_eq!(modes.fast.provider, "openrouter");
        assert_eq!(modes.fast.model, "custom/flash");
        assert_eq!(modes.fast.thinking, "low");
    }

    #[test]
    fn model_modes_workspace_overlay_merges_fields() {
        let mut global: PlexiConfig = toml::from_str("[models.fast]\nprovider = \"global\"\nmodel = \"global/model\"\n").unwrap();
        global.overlay(toml::from_str("[models.fast]\nthinking = \"high\"\n").unwrap());
        let fast = global.resolved_model_modes().fast;
        assert_eq!((fast.provider.as_str(), fast.model.as_str(), fast.thinking.as_str()), ("global", "global/model", "high"));
    }

    #[test]
    fn resolved_model_modes_json_has_both_named_targets() {
        let value = serde_json::to_value(PlexiConfig::default().resolved_model_modes()).unwrap();
        assert_eq!(value["fast"]["provider"], "openrouter");
        assert_eq!(value["fast"]["thinking"], "low");
        assert_eq!(value["slow"]["model"], "openai/gpt-6-astra");
    }

    #[test]
    fn model_modes_validation_names_bad_paths() {
        let dir = tempfile::tempdir().unwrap(); let path = dir.path().join("config.toml");
        fs::write(&path, "[models.fast]\nthinking = \"turbo\"\nprovider = \"bad provider\"\n[models.turbo]\nmodel = \"x\"\n").unwrap();
        let rendered = validate_from_path(&path).into_iter().map(|d| d.to_string()).collect::<Vec<_>>().join("\n");
        assert!(rendered.contains("models.fast.thinking: \"turbo\" is not one of off|minimal|low|medium|high|xhigh"), "{rendered}");
        assert!(rendered.contains("models.fast.provider"), "{rendered}");
        assert!(rendered.contains("unknown key `turbo` in [models]"), "{rendered}");
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
    fn validate_ai_local_config_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            "[ai]\nbackend = \"local\"\n\
             [ai.local]\n\
             base_url = \"http://127.0.0.1:3456\"\n\
             api_key_env = \"MERIDIAN_API_KEY\"\n\
             model_low = \"claude-haiku-4-5\"\n\
             model_medium = \"claude-opus-5\"\n\
             model_high = \"claude-fable-5\"\n",
        )
        .unwrap();
        let diags = validate_from_path(&path);
        assert!(diags.is_empty(), "expected no diagnostics, got: {diags:?}");
    }

    #[test]
    fn validate_ai_local_unknown_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "[ai.local]\nhost = \"http://127.0.0.1:3456\"\n").unwrap();
        let diags = validate_from_path(&path);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].to_string().contains("host"));
        assert!(diags[0].to_string().contains("[ai.local]"));
    }

    #[test]
    fn ai_local_config_parses_round_trip() {
        let cfg: PlexiConfig = toml::from_str(
            "[ai]\nbackend = \"local\"\n\
             [ai.local]\n\
             base_url = \"http://127.0.0.1:3456\"\n\
             model_high = \"claude-fable-5\"\n",
        )
        .unwrap();
        let ai = cfg.ai.unwrap();
        assert_eq!(ai.backend.as_deref(), Some("local"));
        let local = ai.local.unwrap();
        assert_eq!(local.base_url.as_deref(), Some("http://127.0.0.1:3456"));
        assert_eq!(local.api_key_env, None);
        assert_eq!(local.tiers.model_high.as_deref(), Some("claude-fable-5"));
    }

    /// The tier triple is a flattened `ModelTiers`, so the keys must still sit
    /// directly in each backend table on disk — no `[ai.<backend>.tiers]`.
    #[test]
    fn flattened_model_tiers_parse_from_each_backend_section() {
        let cfg: PlexiConfig = toml::from_str(
            "[ai.openrouter]\n\
             api_key_env = \"OPENROUTER_API_KEY\"\n\
             model_low = \"or-low\"\n\
             [ai.ollama]\n\
             host = \"http://localhost:11434\"\n\
             model_medium = \"ol-medium\"\n\
             [ai.local]\n\
             base_url = \"http://127.0.0.1:3456\"\n\
             model_high = \"lo-high\"\n",
        )
        .unwrap();
        let ai = cfg.ai.unwrap();

        let or = ai.openrouter.unwrap();
        assert_eq!(or.api_key_env.as_deref(), Some("OPENROUTER_API_KEY"));
        assert_eq!(or.tiers.resolve(ModelTier::Low).as_deref(), Some("or-low"));
        assert_eq!(or.tiers.resolve(ModelTier::High), None);

        let ol = ai.ollama.unwrap();
        assert_eq!(ol.host.as_deref(), Some("http://localhost:11434"));
        assert_eq!(
            ol.tiers.resolve(ModelTier::Medium).as_deref(),
            Some("ol-medium")
        );

        let lo = ai.local.unwrap();
        assert_eq!(lo.base_url.as_deref(), Some("http://127.0.0.1:3456"));
        assert_eq!(
            lo.tiers.resolve(ModelTier::High).as_deref(),
            Some("lo-high")
        );
    }

    #[test]
    fn ai_local_overlay_merges_fields() {
        let mut base: AiConfig = toml::from_str(
            "backend = \"openrouter\"\n\
             [local]\n\
             base_url = \"http://127.0.0.1:3456\"\n\
             model_low = \"claude-haiku-4-5\"\n",
        )
        .unwrap();
        let project: AiConfig = toml::from_str(
            "backend = \"local\"\n\
             [local]\n\
             model_low = \"claude-opus-5\"\n\
             model_high = \"claude-fable-5\"\n",
        )
        .unwrap();
        base.overlay(project);
        assert_eq!(base.backend.as_deref(), Some("local"));
        let local = base.local.unwrap();
        // Project value wins; unset project fields keep the base value.
        assert_eq!(local.base_url.as_deref(), Some("http://127.0.0.1:3456"));
        assert_eq!(local.tiers.model_low.as_deref(), Some("claude-opus-5"));
        assert_eq!(local.tiers.model_high.as_deref(), Some("claude-fable-5"));
    }

    #[test]
    fn ai_local_overlay_adopts_section_when_base_absent() {
        let mut base = AiConfig::default();
        let project: AiConfig =
            toml::from_str("[local]\nbase_url = \"http://127.0.0.1:3456\"\n").unwrap();
        base.overlay(project);
        assert_eq!(
            base.local.unwrap().base_url.as_deref(),
            Some("http://127.0.0.1:3456")
        );
    }

    #[test]
    fn validate_marketplace_config_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            "[marketplace]\n\
             registry_url = \"https://plexiapp.com/registry/v1/index.json\"\n\
             cdn_url = \"https://plexiapp.com/registry/v1/packages\"\n\
             submit_url = \"https://plexiapp.com/registry/v1/submit\"\n\
             account_backend = \"plexi\"\n\
             account_url = \"https://plexiapp.com\"\n\
             account_email = \"publisher@example.com\"\n",
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
    fn validate_osc_pane_title_top_level_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "osc_pane_title = false\n").unwrap();
        let diags = validate_from_path(&path);
        assert!(diags.is_empty(), "expected no diagnostics, got: {diags:?}");
    }

    #[test]
    fn osc_pane_title_defaults_on() {
        let default_cfg = PlexiConfig::default();
        assert!(default_cfg.osc_pane_title_enabled());

        let top_level_disabled: PlexiConfig = toml::from_str("osc_pane_title = false\n").unwrap();
        assert!(!top_level_disabled.osc_pane_title_enabled());
    }

    #[test]
    fn validate_unknown_key_in_section() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "[effects]\ncrt = true\nfake_flag = true\n").unwrap();
        let diags = validate_from_path(&path);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].to_string().contains("fake_flag"));
        assert!(diags[0].to_string().contains("[effects]"));
    }

    #[test]
    fn validate_beta_section_is_unknown_after_effects_rename() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "[beta]\nghost = true\n").unwrap();
        let diags = validate_from_path(&path);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].to_string().contains("unknown key `beta`"));
    }

    #[test]
    fn effects_ghost_defaults_on_at_point_seven_five() {
        let cfg = EffectsConfig::default();
        assert_eq!(cfg.unfocused_opacity(), Some(0.75));

        let disabled: EffectsConfig = toml::from_str("ghost = false\n").unwrap();
        assert_eq!(disabled.unfocused_opacity(), None);

        let custom: EffectsConfig = toml::from_str("ghost_opacity = 0.6\n").unwrap();
        assert_eq!(custom.unfocused_opacity(), Some(0.6));
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

    #[test]
    fn config_docs_explain_channel_discipline() {
        for doc in [
            include_str!("../../docs/CONFIG.md"),
            include_str!("../../website/src/content/docs/config.md"),
        ] {
            assert!(doc.contains("Alpha config stays default"));
            assert!(doc.contains("Beta config is the staging ground"));
            assert!(doc.contains("PR builds are isolated"));
            assert!(doc.contains("~/.plexi-pr-<N>/config.toml"));
        }
    }

    /// Stint 0285 shipped the CPython-in-WASM sandbox and 0512 corrected the
    /// docs that still described the deleted native-subprocess model. This
    /// test used to assert the opposite — that public docs deny Python
    /// sandboxing — which kept a false security claim pinned in place. The
    /// invariant it guards now is the true one: no public doc may tell a
    /// reader that Python apps run unsandboxed as native processes.
    #[test]
    fn public_security_docs_do_not_deny_python_sandboxing() {
        for (name, doc) in [
            ("README.md", include_str!("../../README.md")),
            (
                "apps.md",
                include_str!("../../website/src/content/docs/apps.md"),
            ),
            (
                "sdk.md",
                include_str!("../../website/src/content/docs/sdk.md"),
            ),
            (
                "pgap.md",
                include_str!("../../website/src/content/docs/pgap.md"),
            ),
        ] {
            for claim in [
                "Python apps are native processes",
                "Python apps are native subprocesses",
                "not sandboxed",
                "not a Python process sandbox",
                "do not sandbox",
            ] {
                assert!(
                    !doc.contains(claim),
                    "{name} claims {claim:?}; Python apps run sandboxed through the \
                     CPython-in-WASM adapter (docs/wasm-runtime.md § Security Model)"
                );
            }
        }
    }
}

#[cfg(test)]
mod legacy_ai_rejection_tests {
    use super::*;

    #[test]
    fn removed_ai_keys_fail_validation_and_strict_load() {
        for key in ["backend", "openrouter.model_low", "openrouter.api_key_env", "ollama.host", "local.base_url"] {
            let (section, field) = key.split_once('.').map_or(("ai".to_string(), key.to_string()), |(section, field)| (format!("ai.{section}"), field.to_string()));
            let text = format!("[{section}]\n{field} = \"value\"\n");
            let path = Path::new("legacy-config.toml");
            let diagnostics = validate_from_str_for_test(&text, path);
            let expected = format!("ai.{key} is no longer supported; use [models.fast] / [models.slow] instead");
            assert!(diagnostics.iter().any(|diagnostic| diagnostic.to_string().contains(&expected)));
        }
    }

    fn validate_from_str_for_test(data: &str, path: &Path) -> Vec<ConfigDiagnostic> {
        let temp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(temp.path(), data).unwrap();
        validate_from_path(temp.path())
    }
}

#[cfg(test)]
mod distribution_profile_tests {
    #[test]
    fn initialized_read_only_profile_never_opens_initialization_lock() {
        let dir = tempfile::tempdir().unwrap();
        let _guard = super::set_test_profile_dir(dir.path().to_path_buf());
        super::ensure_profile_initialized();
        crate::cli::install_host::apply_core_pack_always(&crate::cli::install_host::GitCloner, &dir.path().join("apps"));
        // A directory at the lock path makes any read-write open fail, even as root.
        std::fs::create_dir(dir.path().join("initialization.lock")).unwrap();
        #[cfg(unix)]
        let original_permissions = {
            use std::os::unix::fs::PermissionsExt;
            let original = std::fs::metadata(dir.path()).unwrap().permissions();
            std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o555)).unwrap();
            original
        };
        let result = super::lock_profile_initialization();
        #[cfg(unix)]
        std::fs::set_permissions(dir.path(), original_permissions).unwrap();
        assert!(matches!(result, Ok(None)), "fresh profile attempted a write lock: {result:?}");
    }

    #[test]
    fn stale_profile_still_requires_exclusive_initialization() {
        let dir = tempfile::tempdir().unwrap();
        let _guard = super::set_test_profile_dir(dir.path().to_path_buf());
        assert!(super::lock_profile_initialization().unwrap().is_some());
        super::ensure_profile_initialized();
        crate::cli::install_host::apply_core_pack_always(&crate::cli::install_host::GitCloner, &dir.path().join("apps"));
        assert!(super::lock_profile_initialization().unwrap().is_none());
        std::fs::write(dir.path().join("sdk/.sdk_version"), "old-build").unwrap();
        assert!(super::lock_profile_initialization().unwrap().is_some());
    }

    #[test]
    fn installer_metadata_does_not_suppress_first_launch_seeding() {
        let dir = tempfile::tempdir().unwrap();
        let _guard = super::set_test_profile_dir(dir.path().to_path_buf());
        std::fs::write(dir.path().join("installed_tag"), "v0.3.4").unwrap();
        assert!(super::ensure_profile_initialized());
        assert!(!super::ensure_profile_initialized());
        assert_eq!(std::fs::read_to_string(dir.path().join("sdk/.sdk_version")).unwrap(), env!("PLEXI_BUILD_ID"));
    }
}
