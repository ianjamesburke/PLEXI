use std::path::Path;

/// `plexi agent init <name>` — scaffold an agent app with ai.query capability.
pub fn agent_init(name: &str, from_pane_id: Option<u64>) -> i32 {
    log::info!("agent_init:cli: name={name}");
    crate::cli::app::app_init(name, "python_agent", false, false, from_pane_id)
}

/// `plexi agent add <name>` — copy AGENT.md from global registry into workspace.
pub fn agent_add(name: &str) -> i32 {
    log::info!("agent_add:cli: name={name}");

    let global_dir = crate::config::config_dir().join("agents").join(name);
    let global_agent_md = global_dir.join("AGENT.md");

    if !global_agent_md.is_file() {
        eprintln!(
            "error: agent '{name}' not found in global registry.\n  \
             Expected: {}\n  \
             Create the file and try again.",
            global_agent_md.display()
        );
        return 1;
    }

    let workspace_root = match resolve_workspace_cwd() {
        Ok(root) => root,
        Err(code) => return code,
    };

    let channel_dir = crate::config::workspace_channel_dir();
    let ws_agent_dir = workspace_root.join(&channel_dir).join("agents").join(name);
    let ws_agent_md = ws_agent_dir.join("AGENT.md");

    if ws_agent_md.exists() {
        eprintln!(
            "error: agent '{name}' already installed in this workspace.\n  \
             Use `plexi agent update {name}` to refresh the definition."
        );
        return 1;
    }

    if let Err(e) = install_agent(&global_agent_md, &ws_agent_dir) {
        eprintln!("error: {e}");
        return 1;
    }

    log::info!(
        "agent_add:cli: installed {name} to {}",
        ws_agent_dir.display()
    );
    println!("Installed agent '{name}' into .plexi/agents/{name}/");
    println!("  AGENT.md   copied from global registry");
    println!("  memory/    created (workspace-scoped agent context)");
    println!("  logs/      created (workspace-scoped agent logs)");
    super::print_tip("memory/ and logs/ are git-ignored by default. Run `plexi workspace init` if .gitignore is missing agent entries.");
    0
}

/// `plexi agent update <name>` — overwrite AGENT.md from global, preserve memory/logs.
pub fn agent_update(name: &str) -> i32 {
    log::info!("agent_update:cli: name={name}");

    let global_dir = crate::config::config_dir().join("agents").join(name);
    let global_agent_md = global_dir.join("AGENT.md");

    if !global_agent_md.is_file() {
        eprintln!(
            "error: agent '{name}' not found in global registry.\n  \
             Expected: {}",
            global_agent_md.display()
        );
        return 1;
    }

    let workspace_root = match resolve_workspace_cwd() {
        Ok(root) => root,
        Err(code) => return code,
    };

    let channel_dir = crate::config::workspace_channel_dir();
    let ws_agent_dir = workspace_root.join(&channel_dir).join("agents").join(name);
    let ws_agent_md = ws_agent_dir.join("AGENT.md");

    if !ws_agent_md.exists() {
        eprintln!(
            "error: agent '{name}' is not installed in this workspace.\n  \
             Use `plexi agent add {name}` first."
        );
        return 1;
    }

    // Read source
    let content = match std::fs::read_to_string(&global_agent_md) {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "error: could not read {}: {e}",
                global_agent_md.display()
            );
            return 1;
        }
    };

    // Overwrite only AGENT.md, leave memory/ and logs/ untouched
    if let Err(e) = std::fs::write(&ws_agent_md, &content) {
        eprintln!("error: could not write {}: {e}", ws_agent_md.display());
        return 1;
    }

    // Ensure memory/ and logs/ exist (in case they were deleted)
    for subdir in &["memory", "logs"] {
        let path = ws_agent_dir.join(subdir);
        if let Err(e) = std::fs::create_dir_all(&path) {
            eprintln!("warning: could not create {}: {e}", path.display());
        }
    }

    log::info!(
        "agent_update:cli: updated {name} at {}",
        ws_agent_dir.display()
    );
    println!("Updated agent '{name}' — AGENT.md refreshed, memory/ and logs/ preserved.");
    0
}

/// `plexi agent list` — list agents installed in the current workspace.
pub fn agent_list() -> i32 {
    log::info!("agent_list:cli");

    let workspace_root = match resolve_workspace_cwd() {
        Ok(root) => root,
        Err(code) => return code,
    };

    let channel_dir = crate::config::workspace_channel_dir();
    let agents_dir = workspace_root.join(&channel_dir).join("agents");
    if !agents_dir.is_dir() {
        println!("No agents installed in this workspace.");
        return 0;
    }

    let mut names: Vec<String> = Vec::new();
    match std::fs::read_dir(&agents_dir) {
        Ok(entries) => {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() && path.join("AGENT.md").is_file() {
                    if let Some(name) = entry.file_name().to_str() {
                        names.push(name.to_string());
                    }
                }
            }
        }
        Err(e) => {
            eprintln!("error: could not read {}: {e}", agents_dir.display());
            return 1;
        }
    }

    names.sort();

    if names.is_empty() {
        println!("No agents installed in this workspace.");
    } else {
        println!("Installed agents:");
        for name in &names {
            println!("  {name}");
        }
    }
    0
}

/// Resolve the workspace root from the current directory.
fn resolve_workspace_cwd() -> Result<std::path::PathBuf, i32> {
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: could not determine current directory: {e}");
            return Err(1);
        }
    };
    match crate::app::registry::resolve_workspace_root(&cwd) {
        Some(root) => Ok(root),
        None => {
            eprintln!(
                "error: no .plexi/ workspace found at or above {}.\n  \
                 Run `plexi workspace init` first.",
                cwd.display()
            );
            Err(1)
        }
    }
}

/// Copy AGENT.md and create memory/ + logs/ subdirectories.
fn install_agent(global_agent_md: &Path, ws_agent_dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(ws_agent_dir)
        .map_err(|e| format!("create {}: {e}", ws_agent_dir.display()))?;

    let content = std::fs::read_to_string(global_agent_md)
        .map_err(|e| format!("read {}: {e}", global_agent_md.display()))?;

    let dest = ws_agent_dir.join("AGENT.md");
    std::fs::write(&dest, &content)
        .map_err(|e| format!("write {}: {e}", dest.display()))?;

    for subdir in &["memory", "logs"] {
        let path = ws_agent_dir.join(subdir);
        std::fs::create_dir_all(&path)
            .map_err(|e| format!("create {}: {e}", path.display()))?;
    }

    Ok(())
}

#[cfg(test)]
mod agent_tests {
    use std::fs;
    use tempfile::TempDir;

    /// Set up a fake global config dir with an agent, and a workspace with .plexi/.
    /// Returns (global_tempdir, workspace_tempdir).
    fn setup_dirs(agent_name: &str, agent_content: &str) -> (TempDir, TempDir) {
        let global = tempfile::tempdir().unwrap();
        let agent_dir = global.path().join("agents").join(agent_name);
        fs::create_dir_all(&agent_dir).unwrap();
        fs::write(agent_dir.join("AGENT.md"), agent_content).unwrap();

        let workspace = tempfile::tempdir().unwrap();
        fs::create_dir_all(workspace.path().join(".plexi")).unwrap();

        (global, workspace)
    }

    #[test]
    fn install_agent_creates_all_paths() {
        let (global, workspace) = setup_dirs("test-agent", "name = \"test-agent\"");
        let global_md = global.path().join("agents/test-agent/AGENT.md");
        let ws_dir = workspace.path().join(".plexi/agents/test-agent");

        super::install_agent(&global_md, &ws_dir).unwrap();

        assert!(ws_dir.join("AGENT.md").is_file());
        assert!(ws_dir.join("memory").is_dir());
        assert!(ws_dir.join("logs").is_dir());

        let content = fs::read_to_string(ws_dir.join("AGENT.md")).unwrap();
        assert_eq!(content, "name = \"test-agent\"");
    }

    #[test]
    fn install_agent_fails_on_missing_source() {
        let workspace = tempfile::tempdir().unwrap();
        let fake_md = workspace.path().join("nonexistent/AGENT.md");
        let ws_dir = workspace.path().join(".plexi/agents/missing");

        let result = super::install_agent(&fake_md, &ws_dir);
        assert!(result.is_err());
    }
}
