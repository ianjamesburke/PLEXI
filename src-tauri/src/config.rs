use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Returns ~/.plexi, creating it and the workspaces/ subdirectory if missing.
fn plexi_dir() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or("Could not determine home directory")?;
    let base = home.join(".plexi");
    let workspaces = base.join("workspaces");

    if !base.exists() {
        fs::create_dir_all(&base).map_err(|e| format!("Failed to create ~/.plexi: {e}"))?;
    }
    if !workspaces.exists() {
        fs::create_dir_all(&workspaces)
            .map_err(|e| format!("Failed to create ~/.plexi/workspaces: {e}"))?;
    }

    Ok(base)
}

#[derive(Serialize)]
pub struct PlexiPaths {
    pub base: String,
    pub config: String,
    pub workspaces: String,
}

#[derive(Serialize, Deserialize)]
pub struct WorkspaceEntry {
    pub name: String,
    pub path: String,
}

#[tauri::command]
pub fn get_plexi_paths() -> Result<PlexiPaths, String> {
    let base = plexi_dir()?;
    Ok(PlexiPaths {
        base: base.display().to_string(),
        config: base.join("config.json").display().to_string(),
        workspaces: base.join("workspaces").display().to_string(),
    })
}

#[tauri::command]
pub fn read_config() -> Result<Option<String>, String> {
    let base = plexi_dir()?;
    let path = base.join("config.json");

    if !path.exists() {
        return Ok(None);
    }

    fs::read_to_string(&path).map(Some).map_err(|e| format!("Failed to read config: {e}"))
}

#[tauri::command]
pub fn write_config(contents: String) -> Result<(), String> {
    let base = plexi_dir()?;
    let path = base.join("config.json");

    fs::write(&path, contents).map_err(|e| format!("Failed to write config: {e}"))
}

#[tauri::command]
pub fn read_workspace(name: String) -> Result<Option<String>, String> {
    let base = plexi_dir()?;
    let safe_name = sanitize_workspace_name(&name)?;
    let path = base.join("workspaces").join(format!("{safe_name}.json"));

    if !path.exists() {
        return Ok(None);
    }

    fs::read_to_string(&path)
        .map(Some)
        .map_err(|e| format!("Failed to read workspace '{name}': {e}"))
}

/// Renames <name>.json to <name>.backup-<timestamp>.json so a corrupt file
/// isn't overwritten by the next save. Returns the backup filename (not full
/// path) so the frontend can show the user where to find it.
#[tauri::command]
pub fn backup_workspace(name: String) -> Result<String, String> {
    let base = plexi_dir()?;
    let safe_name = sanitize_workspace_name(&name)?;
    let src = base.join("workspaces").join(format!("{safe_name}.json"));

    if !src.exists() {
        return Err(format!("Workspace '{name}' does not exist"));
    }

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let backup_name = format!("{safe_name}.backup-{timestamp}.json");
    let dst = base.join("workspaces").join(&backup_name);

    fs::rename(&src, &dst).map_err(|e| format!("Failed to back up workspace '{name}': {e}"))?;

    Ok(backup_name)
}

#[tauri::command]
pub fn write_workspace(name: String, contents: String) -> Result<(), String> {
    let base = plexi_dir()?;
    let safe_name = sanitize_workspace_name(&name)?;
    let path = base.join("workspaces").join(format!("{safe_name}.json"));

    fs::write(&path, contents).map_err(|e| format!("Failed to write workspace '{name}': {e}"))
}

#[tauri::command]
pub fn list_workspaces() -> Result<Vec<WorkspaceEntry>, String> {
    let base = plexi_dir()?;
    let dir = base.join("workspaces");

    let mut entries = Vec::new();

    let listing = fs::read_dir(&dir).map_err(|e| format!("Failed to list workspaces: {e}"))?;

    for entry in listing {
        let entry = entry.map_err(|e| format!("Failed to read directory entry: {e}"))?;
        let path = entry.path();

        if path.extension().and_then(|e| e.to_str()) == Some("json") {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                entries.push(WorkspaceEntry {
                    name: stem.to_string(),
                    path: path.display().to_string(),
                });
            }
        }
    }

    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

/// Only allow alphanumeric, dash, underscore, space, and dot in workspace names.
fn sanitize_workspace_name(name: &str) -> Result<String, String> {
    let trimmed = name.trim();

    if trimmed.is_empty() {
        return Err("Workspace name cannot be empty".to_string());
    }

    if trimmed.contains("..") || trimmed.contains('/') || trimmed.contains('\\') {
        return Err("Workspace name contains invalid characters".to_string());
    }

    let safe: String = trimmed
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_' || *c == ' ' || *c == '.')
        .collect();

    if safe.is_empty() {
        return Err("Workspace name contains only invalid characters".to_string());
    }

    Ok(safe)
}
