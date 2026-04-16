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
    /// Protocol version the app was written against. Missing or < 2 triggers
    /// a deprecation warning at load time — the app still runs, but is
    /// flagged as using the v1 protocol. Apps should declare
    /// `protocol_version = 2` in their manifest.
    #[serde(default = "default_protocol_version_manifest")]
    pub protocol_version: u32,
    #[serde(default)]
    pub capabilities: AppCapabilities,
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
    /// Optional companion-pane launch configuration. When present, Plexi splits
    /// the launching pane and starts the companion in the secondary slot.
    #[serde(default)]
    pub launch: Option<AppLaunchConfig>,
    /// Declares how (and by whom) this app may be spawned as a child of
    /// another app via `DrawCommand::SpawnApp`. Missing = permissive defaults
    /// (any caller, any lifecycle, fill layout). Consumed by the spawn
    /// dispatcher once it lands in a follow-up commit.
    #[serde(default)]
    #[allow(dead_code)]
    pub spawnable: AppSpawnable,
    #[serde(default)]
    pub protocol: Option<AppProtocolSection>,
    /// Marks this app as a trusted orchestrator (e.g. Plexi IQ). Orchestrators
    /// are granted all capabilities at install time and never see capability prompts.
    #[serde(default)]
    pub is_orchestrator: bool,
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

/// Launch configuration declared under `[app.launch]` in `manifest.toml`.
/// All fields optional; see defaults on each.
///
/// In v1 this controls both the companion-pane model (the original use case)
/// and the top-level launch mode the spawn dispatcher uses when an app is
/// opened as a child via `DrawCommand::SpawnApp` with no explicit layout:
/// `fullscreen` fills the slot, `windowed` reserved for future floating
/// windows, `companion` triggers the companion-pane split below.
#[derive(Deserialize, Debug, Clone)]
pub struct AppLaunchConfig {
    /// How the app occupies its pane when launched:
    /// `"fullscreen"` (default) | `"windowed"` (reserved) | `"companion"`.
    /// The v1 host treats any value other than `"companion"` as fullscreen;
    /// `"companion"` keeps the existing companion-split behavior.
    #[serde(default = "default_launch_mode")]
    #[allow(dead_code)]
    pub mode: String,
    /// What to run in the companion pane. `"none"` (default) disables the
    /// auto-split; `"terminal"` keeps the v1 behavior.
    #[serde(default = "default_companion")]
    #[allow(dead_code)]
    pub companion: String,
    /// Where the companion sits relative to the app pane:
    /// `"bottom"` (vertical split, default) or `"right"` (horizontal split).
    #[serde(default = "default_companion_position")]
    pub companion_position: String,
    /// Fraction of the split allocated to the companion (0.0..1.0).
    #[serde(default = "default_companion_size")]
    pub companion_size: f32,
    /// Working directory for the companion. Supported template:
    /// `{launch_dir}` — resolves to the app's launch directory.
    #[serde(default = "default_companion_cwd")]
    pub companion_cwd: String,
    /// Optional message written into the linked terminal's scrollback grid
    /// when the app launches. Rendered in dim italics so it reads as a
    /// system-emitted notice rather than real shell output. Not echoed to
    /// the shell's PTY — the shell has no idea these bytes exist, same as
    /// agent-mode output.
    #[serde(default)]
    pub startup_message: Option<String>,
}

impl Default for AppLaunchConfig {
    fn default() -> Self {
        Self {
            mode: default_launch_mode(),
            companion: default_companion(),
            companion_position: default_companion_position(),
            companion_size: default_companion_size(),
            companion_cwd: default_companion_cwd(),
            startup_message: None,
        }
    }
}

fn default_launch_mode() -> String { "fullscreen".to_string() }
fn default_companion() -> String { "none".to_string() }
fn default_companion_position() -> String { "bottom".to_string() }
fn default_companion_size() -> f32 { 0.25 }
fn default_companion_cwd() -> String { "{launch_dir}".to_string() }

/// `[app.spawnable]` — controls which other apps are allowed to spawn this
/// one and the layout/lifecycle defaults the dispatcher applies when the
/// caller omits them. Missing table = permissive defaults.
#[allow(dead_code)]
#[derive(Deserialize, Debug, Clone)]
pub struct AppSpawnable {
    /// Who is allowed to spawn this app via `DrawCommand::SpawnApp`.
    /// `["*"]` (default) accepts any caller; otherwise must contain the
    /// spawner's `app_id` exactly.
    #[serde(default = "default_allow_callers")]
    pub allow_callers: Vec<String>,
    /// Layout used when the caller does not specify one. Serialized the same
    /// way `SpawnLayout` is on the wire (`{ kind = "fill" }` etc).
    #[serde(default = "default_default_layout")]
    pub default_layout: crate::app_protocol::SpawnLayout,
    /// Lifecycles this app accepts. The spawn is refused if the caller asks
    /// for a lifecycle not listed here. Default: all three allowed.
    #[serde(default = "default_allow_lifecycle")]
    pub allow_lifecycle: Vec<String>,
}

impl Default for AppSpawnable {
    fn default() -> Self {
        Self {
            allow_callers: default_allow_callers(),
            default_layout: default_default_layout(),
            allow_lifecycle: default_allow_lifecycle(),
        }
    }
}

#[allow(dead_code)]
impl AppSpawnable {
    /// Returns true if `caller_app_id` is allowed to spawn this app.
    pub fn caller_allowed(&self, caller_app_id: &str) -> bool {
        self.allow_callers.iter().any(|c| c == "*" || c == caller_app_id)
    }

    /// Returns true if this app permits the requested lifecycle.
    pub fn lifecycle_allowed(&self, lifecycle: crate::app_protocol::SpawnLifecycle) -> bool {
        let name = match lifecycle {
            crate::app_protocol::SpawnLifecycle::Cascade => "cascade",
            crate::app_protocol::SpawnLifecycle::Orphan => "orphan",
            crate::app_protocol::SpawnLifecycle::Prompt => "prompt",
        };
        self.allow_lifecycle.iter().any(|l| l == name)
    }
}

fn default_allow_callers() -> Vec<String> {
    vec!["*".to_string()]
}
fn default_default_layout() -> crate::app_protocol::SpawnLayout {
    crate::app_protocol::SpawnLayout::Fill
}
fn default_allow_lifecycle() -> Vec<String> {
    vec!["cascade".to_string(), "orphan".to_string(), "prompt".to_string()]
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
    /// Opt-in to receiving `MouseMove` events. Off by default to avoid pipe flooding.
    #[serde(default)]
    pub mouse_tracking: bool,
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

        // Warn when manifest lacks an explicit protocol_version or declares v1.
        if manifest.app.protocol_version < 2 {
            log::warn!(
                "AppRegistry: app '{}' declares protocol_version = {} (< 2). \
                 Update the manifest to `protocol_version = 2` to suppress this warning.",
                manifest.app.id, manifest.app.protocol_version
            );
        }

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
        self.launch_inner(id, cwd, args, open_intent, pane_id, None)
    }

    /// Launch an app that was spawned by another app. Sets PLEXI_LAUNCH_MODE=spawned
    /// and PLEXI_PARENT_APP_ID=<parent_app_id> in the child's environment.
    pub fn launch_as_child(
        &self,
        id: &str,
        cwd: &PathBuf,
        args: &[String],
        parent_app_id: &str,
    ) -> Option<Box<dyn App>> {
        self.launch_inner(id, cwd, args, None, 0, Some(parent_app_id))
    }

    fn launch_inner(
        &self,
        id: &str,
        cwd: &PathBuf,
        args: &[String],
        open_intent: Option<crate::app_protocol::OpenIntent>,
        pane_id: u64,
        parent_app_id: Option<&str>,
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
        let mouse_tracking = installed.manifest.capabilities.mouse_tracking;
        let observes = installed.manifest.observes.clone();
        let create_runs = installed.manifest.create_runs;
        let open_intent_kinds = installed.manifest.open_intent_kinds.clone();
        let is_orchestrator = installed.manifest.is_orchestrator;
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
            mouse_tracking,
            parent_app_id,
        ) {
            Ok(mut app) => {
                app.set_manifest_capabilities(observes, create_runs, open_intent_kinds, is_orchestrator);
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
