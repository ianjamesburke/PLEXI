use std::path::Path;

/// Lazily-discovered context about what the agent can see.
pub struct AgentContext {
    pub apps: Vec<AppInfo>,
    pub project_files: Vec<String>,
}

/// Metadata about an installed Plexi app, relevant to the agent.
pub struct AppInfo {
    pub id: String,
    pub name: String,
    /// Content of agents.md if present in the app directory.
    pub agents_md: Option<String>,
}

impl AgentContext {
    /// Discover apps and project files for the given directory scope.
    /// This is intentionally lazy — called once when agent mode activates.
    pub fn discover(scope: &Path) -> Self {
        let apps = Self::discover_apps();
        let project_files = Self::discover_project_files(scope);
        Self {
            apps,
            project_files,
        }
    }

    /// Scan installed apps for agent-facing metadata.
    fn discover_apps() -> Vec<AppInfo> {
        let apps_dir = match crate::config::config_dir() {
            d if d.join("apps").is_dir() => d.join("apps"),
            _ => return Vec::new(),
        };

        let mut apps = Vec::new();
        let entries = match std::fs::read_dir(&apps_dir) {
            Ok(e) => e,
            Err(e) => {
                log::warn!("Failed to read apps directory: {e}");
                return Vec::new();
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let id = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();

            let manifest_path = path.join("manifest.toml");
            let name = if manifest_path.exists() {
                Self::read_app_name(&manifest_path).unwrap_or_else(|| id.clone())
            } else {
                id.clone()
            };

            let agents_md_path = path.join("agents.md");
            let agents_md = if agents_md_path.exists() {
                std::fs::read_to_string(&agents_md_path).ok()
            } else {
                None
            };

            apps.push(AppInfo {
                id,
                name,
                agents_md,
            });
        }

        apps
    }

    /// Read the app name from manifest.toml.
    fn read_app_name(manifest_path: &Path) -> Option<String> {
        let content = std::fs::read_to_string(manifest_path).ok()?;
        let table: toml::Table = content.parse().ok()?;
        table
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }

    /// List key files in the scoped directory (shallow, non-recursive).
    fn discover_project_files(scope: &Path) -> Vec<String> {
        let entries = match std::fs::read_dir(scope) {
            Ok(e) => e,
            Err(_) => return Vec::new(),
        };

        let mut files: Vec<String> = entries
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                // Skip hidden files
                if name.starts_with('.') {
                    return None;
                }
                Some(name)
            })
            .collect();

        files.sort();
        files.truncate(50); // cap discovery to avoid noise
        files
    }
}
