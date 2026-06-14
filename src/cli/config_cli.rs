use crate::cli::args::ConfigScope;
use std::path::PathBuf;

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
        if let Err(e) = std::fs::copy(&path, &bak) {
            return Err(format!(
                "could not back up config to {}: {e}",
                bak.display()
            ));
        }
        eprintln!("backed up existing config to {}", bak.display());
    }
    match std::fs::write(&path, crate::config::CONFIG_TEMPLATE) {
        Ok(()) => {
            eprintln!("✓ wrote default config to {}", path.display());
            Ok(())
        }
        Err(e) => Err(format!("could not write config: {e}")),
    }
}
