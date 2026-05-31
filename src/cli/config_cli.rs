pub fn config_check() -> i32 {
    log::info!("config_check: validating config files");
    let diags = crate::config::validate_all();

    if diags.is_empty() {
        let path = crate::config::config_path();
        eprintln!("✓ {} is valid", path.display());
        if let Some(root) = crate::config::active_workspace_root() {
            let project_path = root.join(".plexi").join("config.toml");
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

    if has_errors { 1 } else { 0 }
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
                match std::process::Command::new(editor_bin).args(&args).arg(&path).status() {
                    Ok(status) if status.success() => return 0,
                    Ok(status) => {
                        eprintln!("error: editor {editor_bin:?} exited with status {}", status.code().unwrap_or(1));
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
    let config = crate::config::PlexiConfig::load_with_workspace(
        crate::config::active_workspace_root().as_deref(),
    );
    let agents = config.agents.as_ref();
    let value = match key {
        "agents.low" => agents.map(|a| a.effective_low()).unwrap_or(crate::config::DEFAULT_AGENT_LOW),
        "agents.medium" => agents.map(|a| a.effective_medium()).unwrap_or(crate::config::DEFAULT_AGENT_MEDIUM),
        "agents.high" => agents.map(|a| a.effective_high()).unwrap_or(crate::config::DEFAULT_AGENT_HIGH),
        _ => {
            eprintln!("error: unknown config key {key:?} — supported keys: agents.low, agents.medium, agents.high");
            return 1;
        }
    };
    println!("{value}");
    0
}

pub fn config_reset() -> i32 {
    let path = crate::config::config_path();
    log::info!("config_reset: writing default config to {}", path.display());
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("error: could not create config dir {}: {e}", parent.display());
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

