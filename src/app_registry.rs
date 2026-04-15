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

/// Features supported by this build of the Plexi host.
pub const HOST_FEATURES: &[&str] = &[
    "core_v1",
    "open_intent_v1",
    "event_bus_v1",
    "runs_v1",
    "typed_pipes_v1",
    "ui_primitives_v1",
];

#[derive(Deserialize, Debug, Clone)]
pub struct AppManifest {
    pub app: AppManifestApp,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct AppProtocolSection {
    #[serde(default)]
    pub requires: Vec<String>,
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
    #[serde(default = "default_protocol_version_manifest")]
    pub protocol_version: u32,
    #[serde(default)]
    pub skill: Option<AppSkillSection>,
    #[serde(default)]
    pub agent: Option<AppAgentSection>,
    #[serde(default)]
    pub observes: Vec<String>,
    #[serde(default = "default_create_runs")]
    pub create_runs: bool,
    #[serde(default = "default_open_intent_kinds")]
    pub open_intent_kinds: Vec<String>,
    #[serde(default)]
    pub io: Option<AppIoSection>,
    #[serde(default)]
    pub protocol: Option<AppProtocolSection>,
}

fn default_protocol_version_manifest() -> u32 {
    1
}

fn default_create_runs() -> bool {
    true
}

fn default_open_intent_kinds() -> Vec<String> {
    vec!["file".into(), "url".into()]
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct AppSkillSection {
    pub description: String,
    #[serde(default)]
    pub invoke_phrase: Option<String>,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct AppAgentSection {
    pub system_prompt: String,
    #[serde(default)]
    pub tool_allowlist: Vec<String>,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct AppIoSection {
    #[serde(default)]
    pub inputs: Vec<PipeChannel>,
    #[serde(default)]
    pub outputs: Vec<PipeChannel>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct PipeChannel {
    pub kind: String,
    pub name: String,
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
    /// Can write secrets to Keychain via the API.
    #[serde(default)]
    pub secrets_write: bool,
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
            secrets_write: self.secrets_write,
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

    /// Launch an app and return a boxed `App` trait object.
    pub fn launch(&self, id: &str, cwd: &PathBuf, args: &[String]) -> Option<Box<dyn App>> {
        self.launch_with_intent(id, cwd, args, None, 0)
    }

    /// Launch an app with an OpenIntent and pane_id for bus events.
    pub fn launch_with_intent(
        &self,
        id: &str,
        cwd: &PathBuf,
        args: &[String],
        open_intent: Option<crate::app_protocol::OpenIntent>,
        pane_id: u64,
    ) -> Option<Box<dyn App>> {
        let installed = self.apps.get(id)?;

        // Feature negotiation: check that all required features are supported by this host.
        if let Some(protocol) = &installed.manifest.protocol {
            for required in &protocol.requires {
                if !HOST_FEATURES.contains(&required.as_str()) {
                    log::error!(
                        "AppRegistry: cannot launch '{}': host does not support '{}' (required by this app). Update Plexi to support this feature.",
                        id, required
                    );
                    return None;
                }
            }
        }

        let protocol_version = installed.manifest.protocol_version;
        match ProcessApp::launch_with_intent(
            installed.manifest.id.clone(),
            installed.manifest.name.clone(),
            installed.manifest.capabilities.file_types.iter().cloned().collect(),
            &installed.bin_path,
            cwd,
            args,
            open_intent,
            protocol_version,
            pane_id,
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
