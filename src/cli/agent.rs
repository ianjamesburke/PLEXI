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
            eprintln!("error: could not read {}: {e}", global_agent_md.display());
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
    std::fs::write(&dest, &content).map_err(|e| format!("write {}: {e}", dest.display()))?;

    for subdir in &["memory", "logs"] {
        let path = ws_agent_dir.join(subdir);
        std::fs::create_dir_all(&path).map_err(|e| format!("create {}: {e}", path.display()))?;
    }

    Ok(())
}

/// `plexi agent report --state <state>` — report agent state to host via socket.
pub fn agent_report_cli(state: &str, agent: &str, session_id: Option<&str>) -> i32 {
    log::info!("agent_report:cli: state={state} agent={agent}");
    let normalized = match state.to_lowercase().as_str() {
        "working" => "working",
        "blocked" => "blocked",
        "idle" => "idle",
        other => {
            eprintln!("error: unknown state '{other}'; expected working, blocked, or idle");
            return 1;
        }
    };
    let pane_id: u64 = match std::env::var("PLEXI_PANE_ID")
        .ok()
        .and_then(|v| v.parse().ok())
    {
        Some(id) => id,
        None => {
            eprintln!("error: PLEXI_PANE_ID is not set — run this inside a Plexi terminal pane");
            return 1;
        }
    };
    let mut payload = serde_json::json!({
        "type": "set_agent_state",
        "pane_id": pane_id,
        "state": normalized,
        "agent": agent,
    });
    if let Some(sid) = session_id {
        payload["session_id"] = serde_json::Value::String(sid.to_string());
    }
    super::send_to_socket(payload)
}

/// `plexi agent status` — query agent states for all panes.
pub fn agent_status_cli(blocked: bool, working: bool, idle: bool) -> i32 {
    log::info!("agent_status:cli: blocked={blocked} working={working} idle={idle}");
    let tmp_path =
        std::env::temp_dir().join(format!("plexi-agent-states-{}.json", uuid::Uuid::new_v4()));
    let tmp_path_str = tmp_path.to_string_lossy().to_string();
    let code = super::send_to_socket(serde_json::json!({
        "type": "get_agent_states",
        "response_file": tmp_path_str,
    }));
    if code != 0 {
        return code;
    }
    // Poll for response (host writes async); check before sleeping to avoid artificial latency
    let response = (|| {
        for _ in 0..250 {
            if let Ok(content) = std::fs::read_to_string(&tmp_path) {
                if !content.is_empty() {
                    return Some(content);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        None
    })();
    // Clean up temp file.
    let _ = std::fs::remove_file(&tmp_path);
    let content = match response {
        Some(c) => c,
        None => {
            eprintln!("error: timed out waiting for agent states response");
            return 1;
        }
    };
    let states: Vec<serde_json::Value> = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: could not parse agent states: {e}");
            return 1;
        }
    };
    let filter_active = blocked || working || idle;
    let filtered: Vec<&serde_json::Value> = states
        .iter()
        .filter(|s| {
            if !filter_active {
                return true;
            }
            let st = s["state"].as_str().unwrap_or("");
            (blocked && st == "blocked") || (working && st == "working") || (idle && st == "idle")
        })
        .collect();
    if filtered.is_empty() {
        println!("No agent states tracked.");
        return 0;
    }
    println!(
        "{:<12} {:<16} {:<12} {}",
        "PANE_ID", "AGENT", "STATE", "SESSION_ID"
    );
    for s in &filtered {
        let pane_id = s["pane_id"].as_u64().unwrap_or(0);
        let agent = s["agent"].as_str().unwrap_or("unknown");
        let state = s["state"].as_str().unwrap_or("unknown");
        let session_id = s.get("session_id").and_then(|v| v.as_str()).unwrap_or("-");
        println!("{:<12} {:<16} {:<12} {}", pane_id, agent, state, session_id);
    }
    0
}

/// `plexi agent hook install --claude-code` — write hook script + patch ~/.claude/settings.json.
pub fn agent_hook_install_cli(claude_code: bool) -> i32 {
    if !claude_code {
        eprintln!("error: specify --claude-code");
        return 1;
    }
    log::info!("agent_hook_install:cli: claude-code");

    let binary = std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "plexi".to_string());

    // Write a dynamic hook script that reads event name + session_id from stdin.
    // Static per-event commands can't surface session_id; the script approach can.
    let hooks_dir = crate::config::config_dir().join("hooks");
    if let Err(e) = std::fs::create_dir_all(&hooks_dir) {
        eprintln!("error: could not create hooks dir: {e}");
        return 1;
    }
    let script_path = hooks_dir.join("claude-code-agent-state.sh");
    let script_content = format!(
        r#"#!/usr/bin/env bash
set -euo pipefail
if [ -z "${{PLEXI_SOCKET:-}}" ] || [ -z "${{PLEXI_PANE_ID:-}}" ]; then
    exit 0
fi
INPUT=$(cat)
EVENT=$(jq -r '.hook_event_name // empty' <<< "$INPUT" 2>/dev/null || true)
case "$EVENT" in
    SessionStart|UserPromptSubmit) STATE="working" ;;
    PermissionRequest)             STATE="blocked" ;;
    Stop|StopFailure|SessionEnd)   STATE="idle" ;;
    SubagentStop)                  exit 0 ;;
    *)                             exit 0 ;;
esac
SESSION_ID=$(jq -r '.session_id // empty' <<< "$INPUT" 2>/dev/null || true)
ARGS=(agent report --state "$STATE" --agent claude-code)
[ -n "$SESSION_ID" ] && ARGS+=(--session-id "$SESSION_ID")
"{binary}" "${{ARGS[@]}}" >/dev/null 2>&1 || true
exit 0
"#
    );
    if let Err(e) = std::fs::write(&script_path, &script_content) {
        eprintln!("error: could not write hook script: {e}");
        return 1;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) =
            std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755))
        {
            eprintln!("error: could not chmod hook script: {e}");
            return 1;
        }
    }
    let script_str = script_path.to_string_lossy().to_string();
    log::info!("agent_hook_install: wrote hook script to {script_str}");

    let settings_path = claude_settings_path();
    let mut settings = read_claude_settings(&settings_path);

    if settings.get("hooks").is_none() {
        settings["hooks"] = serde_json::json!({});
    }

    let events = [
        "SessionStart",
        "UserPromptSubmit",
        "PermissionRequest",
        "Stop",
        "StopFailure",
        "SessionEnd",
    ];

    for event in &events {
        let event_hooks = settings["hooks"][*event]
            .as_array()
            .cloned()
            .unwrap_or_default();

        // Idempotency: skip if this script or any PLEXI agent-report command is already registered.
        let already_registered = event_hooks.iter().any(|h| {
            h.get("hooks")
                .and_then(|arr| arr.as_array())
                .map(|arr| {
                    arr.iter().any(|entry| {
                        entry
                            .get("command")
                            .and_then(|c| c.as_str())
                            .map(|c| {
                                c.contains("claude-code-agent-state.sh")
                                    || c.contains("plexi agent report")
                            })
                            .unwrap_or(false)
                    })
                })
                .unwrap_or(false)
        });
        if already_registered {
            continue;
        }

        let new_entry = serde_json::json!({
            "matcher": "",
            "hooks": [{"type": "command", "command": script_str}]
        });
        match settings["hooks"][*event].as_array_mut() {
            Some(a) => {
                a.push(new_entry);
            }
            None => {
                settings["hooks"][*event] = serde_json::json!([new_entry]);
            }
        }
    }

    let code = write_claude_settings(&settings_path, &settings);
    if code == 0 {
        println!("Hook script: {script_str}");
        println!(
            "Registered in {} for 6 lifecycle events.",
            settings_path.display()
        );
    }
    code
}

/// `plexi agent hook uninstall --claude-code` — remove PLEXI hook entries from ~/.claude/settings.json.
pub fn agent_hook_uninstall_cli(claude_code: bool) -> i32 {
    if !claude_code {
        eprintln!("error: specify --claude-code");
        return 1;
    }
    log::info!("agent_hook_uninstall:cli: claude-code");

    let settings_path = claude_settings_path();
    if !settings_path.exists() {
        println!(
            "Nothing to uninstall — {} does not exist.",
            settings_path.display()
        );
        return 0;
    }

    let mut settings = read_claude_settings(&settings_path);

    let events = [
        "SessionStart",
        "UserPromptSubmit",
        "PermissionRequest",
        "Stop",
        "StopFailure",
        "SessionEnd",
    ];
    let mut removed = 0usize;

    if let Some(hooks_map) = settings.get_mut("hooks").and_then(|h| h.as_object_mut()) {
        for event in &events {
            if let Some(event_arr) = hooks_map.get_mut(*event).and_then(|v| v.as_array_mut()) {
                let before = event_arr.len();
                event_arr.retain(|entry| {
                    !entry
                        .get("hooks")
                        .and_then(|h| h.as_array())
                        .map(|arr| {
                            arr.iter().any(|h| {
                                h.get("command")
                                    .and_then(|c| c.as_str())
                                    .map(|c| {
                                        c.contains("claude-code-agent-state.sh")
                                            || c.contains("plexi agent report")
                                    })
                                    .unwrap_or(false)
                            })
                        })
                        .unwrap_or(false)
                });
                removed += before - event_arr.len();
            }
        }
    }

    if removed == 0 {
        println!(
            "No PLEXI hook entries found in {}.",
            settings_path.display()
        );
        return 0;
    }

    let code = write_claude_settings(&settings_path, &settings);
    if code == 0 {
        println!(
            "Removed {removed} PLEXI hook entr{} from {}.",
            if removed == 1 { "y" } else { "ies" },
            settings_path.display()
        );
    }
    code
}

fn claude_settings_path() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".claude")
        .join("settings.json")
}

fn read_claude_settings(path: &std::path::Path) -> serde_json::Value {
    if path.exists() {
        match std::fs::read_to_string(path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_else(|e| {
                log::warn!(
                    "agent_hook: could not parse {}: {e} — treating as empty",
                    path.display()
                );
                serde_json::json!({})
            }),
            Err(e) => {
                log::warn!("agent_hook: could not read {}: {e}", path.display());
                serde_json::json!({})
            }
        }
    } else {
        serde_json::json!({})
    }
}

fn write_claude_settings(path: &std::path::Path, settings: &serde_json::Value) -> i32 {
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            eprintln!("error: could not create {}: {e}", parent.display());
            return 1;
        }
    }
    let json = match serde_json::to_string_pretty(settings) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: could not serialize settings: {e}");
            return 1;
        }
    };
    let tmp_path = path.with_extension("json.tmp");
    if let Err(e) = std::fs::write(&tmp_path, &json) {
        eprintln!("error: could not write temp settings: {e}");
        return 1;
    }
    if let Err(e) = std::fs::rename(&tmp_path, path) {
        eprintln!("error: could not rename temp settings: {e}");
        let _ = std::fs::remove_file(&tmp_path);
        return 1;
    }
    log::info!("agent_hook: wrote {}", path.display());
    0
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
