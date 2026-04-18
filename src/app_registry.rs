//! App registry — discovers and loads Plexi apps from `~/.plexi/apps/`.
//!
//! # App directory layout
//!
//! ```
//! ~/.plexi/apps/
//!   my-pdf-viewer/
//!     manifest.toml
//!     bin/              # or just an executable named after the app id
//!       plexi-app       # the app binary (must be executable)
//!   file-browser/
//!     manifest.toml
//!     bin/plexi-app
//! ```
//!
//! # manifest.toml
//!
//! ```toml
//! [app]
//! id = "my-pdf-viewer"
//! name = "PDF Viewer"
//! version = "0.1.0"
//! description = "View PDF files inline"
//!
//! [app.capabilities]
//! file_types = ["pdf"]       # file extensions this app opens
//! keybinding = "cmd+shift+p" # optional global keybinding (not yet wired)
//! ```

use crate::app_trait::App;
use crate::process_app::ProcessApp;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Deserialize, Debug, Clone)]
pub struct AppManifest {
    pub app: AppManifestApp,
}

#[derive(Deserialize, Debug, Clone)]
pub struct AppManifestApp {
    pub id: String,
    pub name: String,
    pub entry: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub capabilities: AppCapabilities,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct AppCapabilities {
    #[serde(default)]
    pub file_types: Vec<String>,
    #[serde(default)]
    pub keybinding: Option<String>,
    /// v3 capability strings declared in manifest.toml.
    /// e.g. ["fs.read", "net.http", "audio.record"]
    /// Valid values: fs.read, fs.write, net.http, secrets.get,
    ///   audio.record, audio.playback, video.playback, pipe.open, spawn.app
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Pane group this app joins at spawn. When any pane in the group reports a
    /// CWD change, every member receives `PlexiEvent::PathChanged { cwd }`.
    /// Convention: "cwd" for generic directory-synced apps.
    #[serde(default)]
    pub group: Option<String>,
    /// If true, this app captures all keyboard input when focused. Host shortcuts
    /// (Cmd+HJKL, Cmd+Enter, etc.) are suppressed. Only Cmd+Q and Cmd+W remain.
    /// Declare in manifest as `keyboard_capture = true`.
    #[serde(default)]
    pub keyboard_capture: bool,
    /// Preferred pane layout: "split" (default, creates linked terminal) or "overlay"
    /// (takes the full pane, no terminal split). Declare as `layout_hint = "overlay"`.
    #[serde(default)]
    pub layout_hint: Option<String>,
}

impl AppCapabilities {
    /// Convert manifest-declared capabilities to runtime permissions.
    pub fn to_permissions(&self) -> crate::app_permissions::AppPermissions {
        crate::app_permissions::AppPermissions::from_capability_strings(&self.capabilities)
    }
}

/// A discovered but not-yet-launched app.
#[derive(Debug, Clone)]
pub struct InstalledApp {
    pub manifest: AppManifestApp,
    pub bin_path: PathBuf,
    pub app_dir: PathBuf,
}

pub struct AppRegistry {
    /// All discovered apps keyed by their id.
    apps: HashMap<String, InstalledApp>,
    /// Extension → app id mapping (first match wins).
    extension_map: HashMap<String, String>,
}

impl AppRegistry {
    /// Scan `~/.plexi/apps/` (global) plus `.plexi/apps/` directories found by
    /// walking up from `cwd` (local). Local apps override global ones with the same id.
    pub fn load(cwd: &std::path::Path) -> Self {
        let mut registry = Self {
            apps: HashMap::new(),
            extension_map: HashMap::new(),
        };

        // Global apps first (lowest priority).
        let global_dir = apps_dir();
        if !global_dir.exists() {
            if let Err(e) = std::fs::create_dir_all(&global_dir) {
                log::warn!("AppRegistry: could not create global apps dir: {e}");
            }
        } else {
            registry.scan_apps_dir(&global_dir);
        }

        // Local apps — walk up from cwd, deepest last so closest dir wins.
        for local_dir in collect_local_app_dirs(cwd).into_iter().rev() {
            registry.scan_apps_dir(&local_dir);
        }

        registry
    }

    /// Scan one `apps/` directory, inserting discovered apps.
    /// Later calls override earlier ones (local beats global).
    fn scan_apps_dir(&mut self, apps_dir: &std::path::Path) {
        let read_dir = match std::fs::read_dir(apps_dir) {
            Ok(d) => d,
            Err(e) => {
                log::warn!("AppRegistry: failed to read {:?}: {e}", apps_dir);
                return;
            }
        };

        for entry in read_dir.flatten() {
            let app_dir = entry.path();
            if !app_dir.is_dir() {
                continue;
            }

            match self.load_app(&app_dir) {
                Ok(installed) => {
                    log::info!("AppRegistry: loaded app '{}' from {:?}", installed.manifest.id, apps_dir);
                    for ext in &installed.manifest.capabilities.file_types {
                        // Plain insert — local apps override global for extension map too.
                        self.extension_map.insert(ext.to_lowercase(), installed.manifest.id.clone());
                    }
                    self.apps.insert(installed.manifest.id.clone(), installed);
                }
                Err(e) => {
                    log::warn!("AppRegistry: skipping {:?}: {e}", app_dir.file_name());
                }
            }
        }
    }

    fn load_app(&self, app_dir: &PathBuf) -> Result<InstalledApp, String> {
        let manifest_path = app_dir.join("manifest.toml");
        let manifest_str = std::fs::read_to_string(&manifest_path)
            .map_err(|e| format!("no manifest.toml: {e}"))?;

        let manifest: AppManifest = toml::from_str(&manifest_str)
            .map_err(|e| format!("invalid manifest: {e}"))?;

        let bin_path = resolve_entry(app_dir, &manifest.app.entry)?;

        Ok(InstalledApp {
            manifest: manifest.app,
            bin_path,
            app_dir: app_dir.clone(),
        })
    }

    /// List all installed apps.
    pub fn list(&self) -> Vec<&InstalledApp> {
        let mut apps: Vec<_> = self.apps.values().collect();
        apps.sort_by_key(|a| &a.manifest.name);
        apps
    }

    /// Find the app registered for a file extension.
    pub fn app_for_extension(&self, ext: &str) -> Option<&InstalledApp> {
        let id = self.extension_map.get(&ext.to_lowercase())?;
        self.apps.get(id)
    }

    /// Look up an app by id.
    pub fn app_by_id(&self, id: &str) -> Option<&InstalledApp> {
        self.apps.get(id)
    }

    /// Get the manifest-declared permissions for an app.
    pub fn permissions_for(&self, app_id: &str) -> Option<crate::app_permissions::AppPermissions> {
        self.apps.get(app_id).map(|app| app.manifest.capabilities.to_permissions())
    }

    /// Get the manifest-declared pane group for an app (None if unset).
    pub fn group_for(&self, app_id: &str) -> Option<String> {
        self.apps.get(app_id).and_then(|a| a.manifest.capabilities.group.clone())
    }

    /// Get the manifest-declared keyboard_capture flag for an app.
    pub fn keyboard_capture_for(&self, app_id: &str) -> bool {
        self.apps.get(app_id).map(|a| a.manifest.capabilities.keyboard_capture).unwrap_or(false)
    }

    /// Get the manifest-declared layout_hint for an app ("split" | "overlay").
    pub fn layout_hint_for(&self, app_id: &str) -> Option<String> {
        self.apps.get(app_id).and_then(|a| a.manifest.capabilities.layout_hint.clone())
    }

    /// Launch an app and return a boxed `App` trait object.
    pub fn launch(&self, id: &str, cwd: &PathBuf, args: &[String]) -> Option<Box<dyn App>> {
        let installed = self.apps.get(id)?;
        let perms = installed.manifest.capabilities.to_permissions();
        let caps = perms.capabilities.clone();
        let keyboard_capture = installed.manifest.capabilities.keyboard_capture;
        match ProcessApp::launch(
            installed.manifest.id.clone(),
            installed.manifest.name.clone(),
            installed.manifest.capabilities.file_types.to_vec(),
            &installed.bin_path,
            cwd,
            args,
            cwd.clone(),
            caps,
            keyboard_capture,
        ) {
            Ok(app) => {
                log::info!("AppRegistry: launched '{}' from {:?}", id, installed.bin_path);
                Some(Box::new(app))
            }
            Err(e) => {
                log::error!("AppRegistry: failed to launch '{}': {e}", id);
                None
            }
        }
    }

    /// Launch the app associated with a file extension, passing the file path as argv[1].
    pub fn launch_for_file(&self, file_path: &std::path::Path, cwd: &std::path::Path) -> Option<Box<dyn App>> {
        let ext = file_path.extension()?.to_string_lossy().to_lowercase();
        let id = self.extension_map.get(&ext)?.clone();
        let args = vec![file_path.display().to_string()];
        self.launch(&id, &cwd.to_path_buf(), &args)
    }
}

/// Returns the path to the global apps directory: `~/.plexi/apps/`.
pub fn apps_dir() -> PathBuf {
    crate::config::config_dir().join("apps")
}

/// Walk up from `cwd` toward home, collecting `.plexi/apps/` directories that exist.
/// Returns dirs ordered from home→cwd (deepest last), so callers that iterate in reverse
/// get the most-local directory applied last (highest priority).
fn collect_local_app_dirs(cwd: &std::path::Path) -> Vec<PathBuf> {
    let home = dirs::home_dir().unwrap_or_default();
    let mut dirs = Vec::new();
    let mut current = cwd;

    loop {
        let candidate = current.join(".plexi").join("apps");
        if candidate.is_dir() {
            dirs.push(candidate);
        }
        if current == home || current.parent().is_none() {
            break;
        }
        current = match current.parent() {
            Some(p) => p,
            None => break,
        };
    }

    dirs.reverse(); // home→cwd order; callers iterate .rev() to apply closest last
    dirs
}

/// Resolve the `entry` field from manifest.toml to an executable path.
/// Fails fast — no guessing, no fallbacks.
fn resolve_entry(app_dir: &PathBuf, entry: &str) -> Result<PathBuf, String> {
    let path = app_dir.join(entry);

    if !path.exists() {
        return Err(format!("entry '{entry}' not found in {:?}", app_dir));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path)
            .map(|m| m.permissions().mode())
            .unwrap_or(0);
        if mode & 0o111 == 0 {
            return Err(format!("entry '{entry}' exists but is not executable (run: chmod +x {entry})"));
        }
    }

    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_group_parses() {
        let toml = r#"
[app]
id = "todo"
name = "Todo"
entry = "todo.py"

[app.capabilities]
capabilities = ["fs.read", "fs.write"]
group = "cwd"
"#;
        let parsed: AppManifest = toml::from_str(toml).expect("parse");
        assert_eq!(parsed.app.capabilities.group.as_deref(), Some("cwd"));
    }

    #[test]
    fn manifest_group_defaults_to_none() {
        let toml = r#"
[app]
id = "snake"
name = "Snake"
entry = "snake.py"

[app.capabilities]
capabilities = []
"#;
        let parsed: AppManifest = toml::from_str(toml).expect("parse");
        assert_eq!(parsed.app.capabilities.group, None);
    }
}
