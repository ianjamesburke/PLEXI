use crate::cli::args::ConfigScope;
use std::path::PathBuf;

/// Static registry of all known config keys: (dotted_key, type_name, description).
pub const CONFIG_KEYS: &[(&str, &str, &str)] = &[
    (
        "config_version",
        "integer",
        "Config schema version (managed automatically)",
    ),
    ("font_size", "float", "UI font size in points"),
    (
        "pane_gap",
        "float",
        "Inter-pane gap width in pixels (clamped 0–20, default 4)",
    ),
    (
        "pane_title_font_size",
        "float",
        "Pane title bar font size (clamped 6–32, default 11)",
    ),
    (
        "osc_pane_title",
        "bool",
        "Apply OSC 0/1/2 title sequences as pane names (default true)",
    ),
    (
        "theme_preset",
        "string",
        "Active theme preset name (e.g. dracula, nord, solarized-dark)",
    ),
    (
        "confirm_quit",
        "bool",
        "Triple-press Cmd+Q confirmation before quitting (default true)",
    ),
    (
        "confirm_close",
        "bool",
        "Confirmation dialog before closing a pane (default true)",
    ),
    (
        "confirm_context_close",
        "bool",
        "Confirmation dialog before closing a context (default true)",
    ),
    (
        "focus_history_depth",
        "integer",
        "Number of focus history entries to retain",
    ),
    // theme.*
    (
        "theme.preset",
        "string",
        "Theme preset applied first; individual color fields override",
    ),
    (
        "theme.bg_darkest",
        "string",
        "Darkest background color (#rrggbb)",
    ),
    ("theme.bg_sidebar", "string", "Sidebar background color"),
    ("theme.bg_toolbar", "string", "Toolbar background color"),
    (
        "theme.terminal_bg",
        "string",
        "Terminal pane background color",
    ),
    ("theme.bg_hover", "string", "Hover state background color"),
    (
        "theme.bg_sidebar_hover",
        "string",
        "Sidebar hover background color",
    ),
    (
        "theme.bg_active",
        "string",
        "Active/selected element background color",
    ),
    ("theme.text_primary", "string", "Primary text color"),
    ("theme.text_dim", "string", "Dimmed text color"),
    ("theme.text_section", "string", "Section header text color"),
    ("theme.accent", "string", "Accent/highlight color"),
    ("theme.border", "string", "Border color"),
    (
        "theme.foreground",
        "string",
        "Terminal foreground (ANSI default)",
    ),
    (
        "theme.background",
        "string",
        "Terminal background (ANSI default)",
    ),
    ("theme.black", "string", "ANSI black"),
    ("theme.red", "string", "ANSI red"),
    ("theme.green", "string", "ANSI green"),
    ("theme.yellow", "string", "ANSI yellow"),
    ("theme.blue", "string", "ANSI blue"),
    ("theme.magenta", "string", "ANSI magenta"),
    ("theme.cyan", "string", "ANSI cyan"),
    ("theme.white", "string", "ANSI white"),
    ("theme.bright_black", "string", "ANSI bright black"),
    ("theme.bright_red", "string", "ANSI bright red"),
    ("theme.bright_green", "string", "ANSI bright green"),
    ("theme.bright_yellow", "string", "ANSI bright yellow"),
    ("theme.bright_blue", "string", "ANSI bright blue"),
    ("theme.bright_magenta", "string", "ANSI bright magenta"),
    ("theme.bright_cyan", "string", "ANSI bright cyan"),
    ("theme.bright_white", "string", "ANSI bright white"),
    (
        "theme.bright_foreground",
        "string",
        "ANSI bright foreground",
    ),
    (
        "theme.pip_working",
        "string",
        "Activity pip color when working",
    ),
    ("theme.pip_idle", "string", "Activity pip color when idle"),
    (
        "theme.pip_blocked",
        "string",
        "Activity pip color when blocked",
    ),
    (
        "theme.pip_dim",
        "float",
        "Opacity multiplier for unfocused pips (default 0.45)",
    ),
    // effects.*
    ("effects.crt", "bool", "CRT scanline overlay effect"),
    ("effects.ghost", "bool", "Ghost (unfocused pane dim) effect"),
    (
        "effects.ghost_opacity",
        "float",
        "Opacity for unfocused panes when ghost enabled (0–1, default 0.75)",
    ),
    // log.*
    ("log.level", "string", "Log level: error, warn, info, debug"),
    (
        "log.retention_days",
        "integer",
        "How many days to retain log files",
    ),
    // notifications.*
    (
        "notifications.enabled",
        "bool",
        "Master notification switch (default true)",
    ),
    (
        "notifications.focus_mode",
        "bool",
        "Suppress all notification auto-open when true (default false)",
    ),
    // agents.*
    ("agents.low", "string", "Command for low-tier agent tasks"),
    (
        "agents.medium",
        "string",
        "Command for medium-tier agent tasks",
    ),
    ("agents.high", "string", "Command for high-tier agent tasks"),
    // ai.*
    (
        "ai.backend",
        "string",
        "AI backend: openrouter (default) or ollama",
    ),
    (
        "ai.per_app_daily_usd",
        "float",
        "Per-app daily spend cap in USD (default 1.00)",
    ),
    (
        "ai.global_daily_usd",
        "float",
        "Global daily spend cap in USD (default 10.00)",
    ),
    (
        "ai.openrouter.api_key_env",
        "string",
        "Env var for OpenRouter API key (default OPENROUTER_API_KEY)",
    ),
    (
        "ai.openrouter.model_low",
        "string",
        "OpenRouter low-tier model",
    ),
    (
        "ai.openrouter.model_medium",
        "string",
        "OpenRouter medium-tier model",
    ),
    (
        "ai.openrouter.model_high",
        "string",
        "OpenRouter high-tier model",
    ),
    (
        "ai.ollama.host",
        "string",
        "Ollama host URL (default http://localhost:11434)",
    ),
    ("ai.ollama.model_low", "string", "Ollama low-tier model"),
    (
        "ai.ollama.model_medium",
        "string",
        "Ollama medium-tier model",
    ),
    ("ai.ollama.model_high", "string", "Ollama high-tier model"),
    // cli.*
    (
        "cli.tips",
        "bool",
        "Print contextual tips after CLI commands (default true)",
    ),
    // marketplace.*
    (
        "marketplace.registry_url",
        "string",
        "Override catalog index URL",
    ),
    (
        "marketplace.cdn_url",
        "string",
        "Override package CDN base URL",
    ),
    (
        "marketplace.submit_url",
        "string",
        "Publisher submission endpoint",
    ),
    (
        "marketplace.account_backend",
        "string",
        "Account/auth backend selector (\"plexi\" to enable)",
    ),
    (
        "marketplace.account_url",
        "string",
        "Accounts service base URL (default plexiapp.com)",
    ),
    (
        "marketplace.account_email",
        "string",
        "Default email for plexi account login",
    ),
];

pub fn config_check(scope: ConfigScope) -> i32 {
    log::info!("config_check: validating config files scope={scope:?}");
    let paths = match config_paths_for_scope(scope, false) {
        Ok(paths) => paths,
        Err(msg) => {
            eprintln!("error: {msg}");
            return 1;
        }
    };
    let mut diags = if scope == ConfigScope::Effective {
        crate::config::validate_all()
    } else {
        Vec::new()
    };
    if scope != ConfigScope::Effective {
        for path in &paths {
            diags.extend(crate::config::validate_from_path(path));
        }
    }
    if diags.is_empty() {
        for path in paths {
            if path.exists() {
                eprintln!("✓ {} is valid", path.display());
            }
        }
        return 0;
    }

    let has_errors = diags.iter().any(|d| d.is_error());
    for d in &diags {
        if d.is_error() {
            eprintln!("✗ {d}");
        } else {
            eprintln!("⚠ {d}");
        }
    }

    if has_errors {
        1
    } else {
        0
    }
}

pub fn config_edit(scope: ConfigScope) -> i32 {
    let path = match writable_config_path(scope) {
        Ok(path) => path,
        Err(msg) => {
            eprintln!("error: {msg}");
            return 1;
        }
    };
    log::info!(
        "config_edit: opening {} in editor scope={scope:?}",
        path.display()
    );

    if let Ok(editor_env) = std::env::var("EDITOR") {
        if editor_env.trim().is_empty() {
            log::warn!("config_edit: EDITOR is set but empty, falling through to system default");
        } else {
            let mut parts = editor_env.split_whitespace();
            if let Some(editor_bin) = parts.next() {
                let args: Vec<&str> = parts.collect();
                match std::process::Command::new(editor_bin)
                    .args(&args)
                    .arg(&path)
                    .status()
                {
                    Ok(status) if status.success() => return 0,
                    Ok(status) => {
                        eprintln!(
                            "error: editor {editor_bin:?} exited with status {}",
                            status.code().unwrap_or(1)
                        );
                        return status.code().unwrap_or(1);
                    }
                    Err(e) => {
                        eprintln!("error: could not launch editor {editor_bin:?}: {e}");
                        return 1;
                    }
                }
            }
        }
    }

    // No $EDITOR set — use the fallback chain: VS Code → system default → TextEdit
    if crate::config::open_file_with_fallback(&path) {
        0
    } else {
        eprintln!("error: could not open {:?} — install VS Code, set $EDITOR, or ensure a default text editor is configured", path);
        1
    }
}

pub fn config_get(key: &str, scope: ConfigScope) -> i32 {
    log::info!("config_get: resolving key={key} scope={scope:?}");

    // agents.* are special-cased because they have programmatic defaults
    // not stored in config.toml — always return via effective_*() for those.
    if scope == ConfigScope::Effective {
        match key {
            "agents.low" | "agents.medium" | "agents.high" => {
                let config = crate::config::PlexiConfig::load_with_workspace(
                    crate::config::active_workspace_root().as_deref(),
                );
                let agents = config.agents.as_ref();
                let value = match key {
                    "agents.low" => agents
                        .map(|a| a.effective_low())
                        .unwrap_or(crate::config::DEFAULT_AGENT_LOW),
                    "agents.medium" => agents
                        .map(|a| a.effective_medium())
                        .unwrap_or(crate::config::DEFAULT_AGENT_MEDIUM),
                    _ => agents
                        .map(|a| a.effective_high())
                        .unwrap_or(crate::config::DEFAULT_AGENT_HIGH),
                };
                println!("{value}");
                return 0;
            }
            _ => {}
        };
    }

    // Generic path: load config as a raw TOML value and walk the dot-separated key.
    // Effective scope overlays workspace config on top of the global config.
    let paths = match config_paths_for_scope(scope, false) {
        Ok(paths) => paths,
        Err(msg) => {
            eprintln!("error: {msg}");
            return 1;
        }
    };
    let mut root = toml::Value::Table(toml::map::Map::new());
    for path in paths {
        match std::fs::read_to_string(&path) {
            Ok(data) => match toml::from_str::<toml::Value>(&data) {
                Ok(val) => toml_merge(&mut root, val),
                Err(e) => {
                    eprintln!("error: could not parse {}: {e}", path.display());
                    return 1;
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if scope == ConfigScope::Workspace {
                    eprintln!(
                        "error: workspace config does not exist at {}",
                        path.display()
                    );
                    return 1;
                }
            }
            Err(e) => {
                eprintln!("error: could not read {}: {e}", path.display());
                return 1;
            }
        }
    }

    // Walk the dot-separated key path.
    let mut current = &root;
    for segment in key.split('.') {
        match current {
            toml::Value::Table(table) => match table.get(segment) {
                Some(v) => current = v,
                None => {
                    eprintln!(
                            "error: config key {key:?} not set (no value at {segment:?} in the current config)"
                        );
                    return 1;
                }
            },
            _ => {
                eprintln!("error: config key {key:?} not found (path traverses a non-table value)");
                return 1;
            }
        }
    }

    println!("{}", toml_value_to_string(current));
    0
}

/// Recursively merge `src` on top of `dst` — table keys in `src` override `dst`;
/// non-table values replace outright.
fn toml_merge(dst: &mut toml::Value, src: toml::Value) {
    match (dst, src) {
        (toml::Value::Table(dst_table), toml::Value::Table(src_table)) => {
            for (k, v) in src_table {
                let entry = dst_table
                    .entry(k)
                    .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
                toml_merge(entry, v);
            }
        }
        (dst, src) => *dst = src,
    }
}

/// Format a TOML value as a plain string for CLI output.
fn toml_value_to_string(v: &toml::Value) -> String {
    match v {
        toml::Value::String(s) => s.clone(),
        toml::Value::Integer(i) => i.to_string(),
        toml::Value::Float(f) => f.to_string(),
        toml::Value::Boolean(b) => b.to_string(),
        toml::Value::Datetime(dt) => dt.to_string(),
        toml::Value::Array(_) | toml::Value::Table(_) => v.to_string(),
    }
}

pub fn config_reset(scope: ConfigScope) -> i32 {
    let path = match writable_config_path(scope) {
        Ok(path) => path,
        Err(msg) => {
            eprintln!("error: {msg}");
            return 1;
        }
    };
    log::info!(
        "config_reset: writing default config to {} scope={scope:?}",
        path.display()
    );
    match write_default_config(&path) {
        Ok(()) => 0,
        Err(msg) => {
            eprintln!("error: {msg}");
            1
        }
    }
}

fn writable_config_path(scope: ConfigScope) -> Result<PathBuf, String> {
    match scope {
        ConfigScope::Effective | ConfigScope::Global => {
            let path = crate::config::ensure_config_exists();
            Ok(path)
        }
        ConfigScope::Workspace => {
            let root = crate::config::active_workspace_root()
                .ok_or_else(|| "not inside a Plexi workspace".to_string())?;
            let path = crate::config::workspace_config_path(&root);
            if !path.exists() {
                write_default_config(&path)?;
            }
            Ok(path)
        }
    }
}

fn config_paths_for_scope(
    scope: ConfigScope,
    include_missing_workspace: bool,
) -> Result<Vec<PathBuf>, String> {
    let global = crate::config::config_path();
    match scope {
        ConfigScope::Global => Ok(vec![global]),
        ConfigScope::Workspace => {
            let root = crate::config::active_workspace_root()
                .ok_or_else(|| "not inside a Plexi workspace".to_string())?;
            Ok(vec![crate::config::workspace_config_path(&root)])
        }
        ConfigScope::Effective => {
            let mut paths = vec![global];
            if let Some(root) = crate::config::active_workspace_root() {
                let workspace = crate::config::workspace_config_path(&root);
                if include_missing_workspace || workspace.exists() {
                    paths.push(workspace);
                }
            }
            Ok(paths)
        }
    }
}

fn write_default_config(path: &std::path::Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return Err(format!(
                "could not create config dir {}: {e}",
                parent.display()
            ));
        }
    }
    if path.exists() {
        let bak = path.with_extension("toml.bak");
        if let Err(e) = std::fs::copy(path, &bak) {
            return Err(format!(
                "could not back up config to {}: {e}",
                bak.display()
            ));
        }
        eprintln!("backed up existing config to {}", bak.display());
    }
    match std::fs::write(path, crate::config::CONFIG_TEMPLATE) {
        Ok(()) => {
            eprintln!("✓ wrote default config to {}", path.display());
            Ok(())
        }
        Err(e) => Err(format!("could not write config: {e}")),
    }
}

pub fn config_list(scope: ConfigScope, json: bool) -> i32 {
    log::info!("config_list: scope={scope:?} json={json}");
    let paths = match config_paths_for_scope(scope, false) {
        Ok(paths) => paths,
        Err(msg) => {
            eprintln!("error: {msg}");
            return 1;
        }
    };
    let mut root = toml::Value::Table(toml::map::Map::new());
    for path in paths {
        match std::fs::read_to_string(&path) {
            Ok(data) => match toml::from_str::<toml::Value>(&data) {
                Ok(val) => toml_merge(&mut root, val),
                Err(e) => {
                    eprintln!("error: could not parse {}: {e}", path.display());
                    return 1;
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                eprintln!("error: could not read {}: {e}", path.display());
                return 1;
            }
        }
    }

    if json {
        let mut arr = Vec::new();
        for (key, type_name, description) in CONFIG_KEYS {
            let value = resolve_dotted_key(&root, key)
                .map(toml_value_to_string)
                .unwrap_or_default();
            arr.push(serde_json::json!({
                "key": key,
                "type": type_name,
                "value": value,
                "description": description,
            }));
        }
        match serde_json::to_string_pretty(&arr) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("error: could not serialize JSON: {e}");
                return 1;
            }
        }
    } else {
        for (key, type_name, description) in CONFIG_KEYS {
            let value = resolve_dotted_key(&root, key)
                .map(toml_value_to_string)
                .unwrap_or_default();
            println!("{key}\t{type_name}\t{value}\t{description}");
        }
    }
    0
}

pub fn config_set(pairs: &[String], scope: ConfigScope) -> i32 {
    log::info!("config_set: {} pair(s) scope={scope:?}", pairs.len());

    // Resolve the target write path, defaulting to workspace if inside one.
    let path = match writable_config_path_defaulting_to_workspace(scope) {
        Ok(p) => p,
        Err(msg) => {
            eprintln!("error: {msg}");
            return 1;
        }
    };

    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            eprintln!("error: could not read {}: {e}", path.display());
            return 1;
        }
    };

    let mut doc: toml_edit::DocumentMut = match raw.parse() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: could not parse {}: {e}", path.display());
            return 1;
        }
    };

    for pair in pairs {
        let (key, raw_value) = match pair.split_once('=') {
            Some(kv) => kv,
            None => {
                eprintln!("error: argument {pair:?} is not in KEY=VALUE form");
                return 1;
            }
        };

        // Look up the expected type from the registry.
        let type_name = CONFIG_KEYS
            .iter()
            .find(|(k, _, _)| *k == key)
            .map(|(_, t, _)| *t);

        let toml_val = match parse_value_for_type(raw_value, type_name) {
            Ok(v) => v,
            Err(msg) => {
                eprintln!("error: {key}: {msg}");
                return 1;
            }
        };

        let segments: Vec<&str> = key.split('.').collect();
        if let Err(msg) = toml_edit_set(&mut doc, &segments, toml_val) {
            eprintln!("error: {key}: {msg}");
            return 1;
        }
        log::info!("config_set: set {key}={raw_value}");
    }

    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!(
                "error: could not create config dir {}: {e}",
                parent.display()
            );
            return 1;
        }
    }
    if let Err(e) = std::fs::write(&path, doc.to_string()) {
        eprintln!("error: could not write {}: {e}", path.display());
        return 1;
    }
    eprintln!("✓ wrote {}", path.display());
    0
}

/// Walk a dotted key path in a TOML value tree, returning a reference to the leaf or None.
fn resolve_dotted_key<'a>(root: &'a toml::Value, key: &str) -> Option<&'a toml::Value> {
    let mut current = root;
    for segment in key.split('.') {
        match current {
            toml::Value::Table(t) => current = t.get(segment)?,
            _ => return None,
        }
    }
    Some(current)
}

/// Parse a string value into the correct TOML edit value based on the key's declared type.
fn parse_value_for_type(raw: &str, type_name: Option<&str>) -> Result<toml_edit::Value, String> {
    match type_name {
        Some("bool") => raw
            .parse::<bool>()
            .map(toml_edit::Value::from)
            .map_err(|_| format!("expected bool (true/false), got {raw:?}")),
        Some("integer") => raw
            .parse::<i64>()
            .map(toml_edit::Value::from)
            .map_err(|_| format!("expected integer, got {raw:?}")),
        Some("float") => raw
            .parse::<f64>()
            .map(toml_edit::Value::from)
            .map_err(|_| format!("expected float, got {raw:?}")),
        // string or unknown key: store as string
        _ => Ok(toml_edit::Value::from(raw)),
    }
}

/// Navigate/create nested tables in a toml_edit document and set the leaf value.
fn toml_edit_set(
    doc: &mut toml_edit::DocumentMut,
    segments: &[&str],
    value: toml_edit::Value,
) -> Result<(), String> {
    let (leaf, parents) = segments
        .split_last()
        .ok_or_else(|| "empty key".to_string())?;

    // Walk/create nested [section] tables.
    let mut table: &mut toml_edit::Table = doc.as_table_mut();
    for seg in parents {
        if !table.contains_key(seg) {
            table.insert(seg, toml_edit::Item::Table(toml_edit::Table::new()));
        }
        table = table
            .get_mut(seg)
            .and_then(|i| i.as_table_mut())
            .ok_or_else(|| format!("{seg} is not a table"))?;
    }
    table.insert(leaf, toml_edit::Item::Value(value));
    Ok(())
}

/// Like `writable_config_path` but defaults to workspace when inside one
/// (scope=Effective resolves to workspace if available, global otherwise).
fn writable_config_path_defaulting_to_workspace(scope: ConfigScope) -> Result<PathBuf, String> {
    match scope {
        ConfigScope::Workspace => {
            let root = crate::config::active_workspace_root()
                .ok_or_else(|| "not inside a Plexi workspace".to_string())?;
            let path = crate::config::workspace_config_path(&root);
            if !path.exists() {
                write_default_config(&path)?;
            }
            Ok(path)
        }
        ConfigScope::Global => {
            let path = crate::config::ensure_config_exists();
            Ok(path)
        }
        ConfigScope::Effective => {
            // Default: workspace if available, global otherwise.
            if let Some(root) = crate::config::active_workspace_root() {
                let path = crate::config::workspace_config_path(&root);
                if !path.exists() {
                    write_default_config(&path)?;
                }
                Ok(path)
            } else {
                let path = crate::config::ensure_config_exists();
                Ok(path)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_set_marketplace_urls_keeps_default_config_parseable() {
        let profile = tempfile::tempdir().unwrap();
        let _profile_guard = crate::config::set_test_profile_dir(profile.path().to_path_buf());
        let pairs = vec![
            "marketplace.registry_url=http://127.0.0.1:8765/registry/v1/index.json".to_string(),
            "marketplace.cdn_url=http://127.0.0.1:8765/registry/v1/packages".to_string(),
        ];

        assert_eq!(config_set(&pairs, ConfigScope::Global), 0);

        let path = profile.path().join("config.toml");
        let text = std::fs::read_to_string(&path).unwrap();
        toml::from_str::<toml::Value>(&text).expect("config set must leave valid TOML");
        let diags = crate::config::validate_from_path(&path);
        assert!(diags.is_empty(), "expected no diagnostics, got: {diags:?}");
        assert!(text.contains("[marketplace]"));
        assert!(text.contains("registry_url = \"http://127.0.0.1:8765/registry/v1/index.json\""));
        assert!(text.contains("cdn_url = \"http://127.0.0.1:8765/registry/v1/packages\""));
    }
}
