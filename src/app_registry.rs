/// App registry — discovers and loads Plexi apps from `~/.plexi/apps/`.
///
/// # App directory layout
///
/// ```
/// ~/.plexi/apps/
///   my-pdf-viewer/
///     manifest.toml
///     bin/              # or just an executable named after the app id
///       plexi-app       # the app binary (must be executable)
///   file-browser/
///     manifest.toml
///     bin/plexi-app
/// ```
///
/// # manifest.toml
///
/// ```toml
/// [app]
/// id = "my-pdf-viewer"
/// name = "PDF Viewer"
/// version = "0.1.0"
/// description = "View PDF files inline"
///
/// [app.capabilities]
/// file_types = ["pdf"]       # file extensions this app opens
/// keybinding = "cmd+shift+p" # optional global keybinding (not yet wired)
/// ```

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
    /// Can send commands to the linked terminal PTY.
    #[serde(default)]
    pub terminal_write: bool,
    /// Filesystem access: "none", "read_only", "read_write". Default: "read_only".
    #[serde(default = "default_fs_permission")]
    pub filesystem: String,
    /// Can read .env / credentials files.
    #[serde(default)]
    pub env_file_access: bool,
    /// Can make network requests.
    #[serde(default)]
    pub network: bool,
}

fn default_fs_permission() -> String {
    "read_only".to_string()
}

impl AppCapabilities {
    /// Convert manifest-declared capabilities to runtime permissions.
    pub fn to_permissions(&self) -> crate::app_permissions::AppPermissions {
        use crate::app_permissions::{AppPermissions, FsPermission, TrustLevel};
        AppPermissions {
            trust_level: TrustLevel::Sandboxed, // manifest apps are always sandboxed
            terminal_write: self.terminal_write,
            filesystem: match self.filesystem.as_str() {
                "none" => FsPermission::None,
                "read_write" => FsPermission::ReadWrite,
                _ => FsPermission::ReadOnly,
            },
            env_file_access: self.env_file_access,
            network: self.network,
        }
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
    /// Scan `~/.plexi/apps/` and load all valid app manifests.
    pub fn load() -> Self {
        let mut registry = Self {
            apps: HashMap::new(),
            extension_map: HashMap::new(),
        };

        let apps_dir = apps_dir();
        if !apps_dir.exists() {
            if let Err(e) = std::fs::create_dir_all(&apps_dir) {
                log::warn!("AppRegistry: could not create apps dir: {e}");
            }
            return registry;
        }

        let read_dir = match std::fs::read_dir(&apps_dir) {
            Ok(d) => d,
            Err(e) => {
                log::warn!("AppRegistry: failed to read apps dir: {e}");
                return registry;
            }
        };

        for entry in read_dir.flatten() {
            let app_dir = entry.path();
            if !app_dir.is_dir() {
                continue;
            }

            match registry.load_app(&app_dir) {
                Ok(installed) => {
                    log::info!("AppRegistry: loaded app '{}'", installed.manifest.id);
                    for ext in &installed.manifest.capabilities.file_types {
                        registry
                            .extension_map
                            .entry(ext.to_lowercase())
                            .or_insert_with(|| installed.manifest.id.clone());
                    }
                    registry.apps.insert(installed.manifest.id.clone(), installed);
                }
                Err(e) => {
                    log::warn!("AppRegistry: skipping {:?}: {e}", app_dir.file_name());
                }
            }
        }

        registry
    }

    fn load_app(&self, app_dir: &PathBuf) -> Result<InstalledApp, String> {
        let manifest_path = app_dir.join("manifest.toml");
        let manifest_str = std::fs::read_to_string(&manifest_path)
            .map_err(|e| format!("no manifest.toml: {e}"))?;

        let manifest: AppManifest = toml::from_str(&manifest_str)
            .map_err(|e| format!("invalid manifest: {e}"))?;

        let bin_path = find_bin(app_dir, &manifest.app.id)?;

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

    /// Launch an app and return a boxed `App` trait object.
    pub fn launch(&self, id: &str, cwd: &PathBuf, args: &[String]) -> Option<Box<dyn App>> {
        let installed = self.apps.get(id)?;
        match ProcessApp::launch(
            installed.manifest.id.clone(),
            installed.manifest.name.clone(),
            installed.manifest.capabilities.file_types.iter().cloned().collect(),
            &installed.bin_path,
            cwd,
            args,
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
    pub fn launch_for_file(&self, file_path: &PathBuf, cwd: &PathBuf) -> Option<Box<dyn App>> {
        let ext = file_path.extension()?.to_string_lossy().to_lowercase();
        let id = self.extension_map.get(&ext)?.clone();
        let args = vec![file_path.display().to_string()];
        self.launch(&id, cwd, &args)
    }
}

/// Returns the path to the apps directory: `~/.plexi/apps/`.
pub fn apps_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".plexi")
        .join("apps")
}

/// Find the executable binary inside an app directory.
///
/// Checks in order:
///   1. `<app_dir>/bin/plexi-app`
///   2. `<app_dir>/bin/<id>`
///   3. `<app_dir>/plexi-app`
///   4. `<app_dir>/<id>`
fn find_bin(app_dir: &PathBuf, id: &str) -> Result<PathBuf, String> {
    let candidates = [
        app_dir.join("bin").join("plexi-app"),
        app_dir.join("bin").join(id),
        app_dir.join("plexi-app"),
        app_dir.join(id),
    ];

    for candidate in &candidates {
        if candidate.exists() {
            // Check it's executable on unix.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = std::fs::metadata(candidate)
                    .map(|m| m.permissions().mode())
                    .unwrap_or(0);
                if mode & 0o111 == 0 {
                    return Err(format!("{:?} exists but is not executable", candidate));
                }
            }
            return Ok(candidate.clone());
        }
    }

    Err(format!(
        "no executable found in {:?} (tried bin/plexi-app, bin/{}, plexi-app, {})",
        app_dir, id, id
    ))
}
