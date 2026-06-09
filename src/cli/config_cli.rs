pub fn config_check() -> i32 {
    log::info!("config_check: validating config files");
    let diags = crate::config::validate_all();

    if diags.is_empty() {
        let path = crate::config::config_path();
        eprintln!("✓ {} is valid", path.display());
        if let Some(root) = crate::config::active_workspace_root() {
            let project_path = crate::config::workspace_config_path(&root);
            if project_path.exists() {
                eprintln!("✓ {} is valid", project_path.display());
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

pub fn config_edit() -> i32 {
    let path = crate::config::ensure_config_exists();
    log::info!("config_edit: opening {} in editor", path.display());

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

pub fn config_get(key: &str) -> i32 {
    log::info!("config_get: resolving key={key}");

    // agents.* are special-cased because they have programmatic defaults
    // not stored in config.toml — always return via effective_*() for those.
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
    }

    // Generic path: load config as a raw TOML value and walk the dot-separated key.
    // Workspace config (if present) overlays the global config at the TOML level.
    let global_path = crate::config::config_path();
    let mut root: toml::Value = match std::fs::read_to_string(&global_path) {
        Ok(data) => match toml::from_str(&data) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("error: could not parse {}: {e}", global_path.display());
                return 1;
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            toml::Value::Table(toml::map::Map::new())
        }
        Err(e) => {
            eprintln!("error: could not read {}: {e}", global_path.display());
            return 1;
        }
    };

    // Overlay workspace config on top if one exists.
    if let Some(workspace_root) = crate::config::active_workspace_root() {
        let project_path = crate::config::workspace_config_path(&workspace_root);
        if let Ok(data) = std::fs::read_to_string(&project_path) {
            if let Ok(project_val) = toml::from_str::<toml::Value>(&data) {
                toml_merge(&mut root, project_val);
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

pub fn config_reset() -> i32 {
    let path = crate::config::config_path();
    log::info!("config_reset: writing default config to {}", path.display());
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!(
                "error: could not create config dir {}: {e}",
                parent.display()
            );
            return 1;
        }
    }
    if path.exists() {
        let bak = path.with_extension("toml.bak");
        if let Err(e) = std::fs::copy(&path, &bak) {
            eprintln!("error: could not back up config to {}: {e}", bak.display());
            return 1;
        }
        eprintln!("backed up existing config to {}", bak.display());
    }
    match std::fs::write(&path, crate::config::CONFIG_TEMPLATE) {
        Ok(()) => {
            eprintln!("✓ wrote default config to {}", path.display());
            0
        }
        Err(e) => {
            eprintln!("error: could not write config: {e}");
            1
        }
    }
}
