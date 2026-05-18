//! App registry — discovers and loads Plexi apps and agents.
//!
//! # Discovery order (later entries shadow earlier ones)
//!
//! 1. `~/.plexi-<channel>/apps/<id>/manifest.toml` — global apps (lowest priority)
//! 2. `<workspace_root>/.plexi/apps/<id>/manifest.toml` — local app (overrides global)
//! 3. `<workspace_root>/.plexi/agents/<id>/manifest.toml` — local agent (overrides above)
//! 4. `<linked_path>/manifest.toml` — linked apps (`.plexi/links.toml`)
//!
//! `workspace_root` is the nearest ancestor of the current working directory that
//! contains a `.plexi/` directory; if none is found, only global apps are loaded.
//! When ids collide, the local entry wins and the shadow is logged at info level.
//!
//! # App directory layout
//!
//! ```
//! ~/.plexi/apps/
//!   my-pdf-viewer/
//!     manifest.toml
//!     bin/              # or just an executable named after the app id
//!       plexi-app       # the app binary (.py entries are launched via python3)
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
use std::path::{Path, PathBuf};

/// The current manifest schema version. Bumped whenever a required field is
/// added/removed/renamed so older or newer manifests fail loud at install
/// rather than silently behave wrong. (Issue #308 Phase 2.)
pub const MANIFEST_SCHEMA_VERSION: u32 = 1;

#[derive(Deserialize, Debug, Clone)]
pub struct AppManifest {
    /// Required schema version — refuses to load if greater than
    /// `MANIFEST_SCHEMA_VERSION`. No serde-default: a manifest that omits
    /// `schema_version` is rejected so contributors notice the contract.
    pub schema_version: u32,
    pub app: AppManifestApp,
    #[serde(default)]
    pub launch: LaunchSection,
    /// Canonical secret names this app reads via `ctx.secret(...)`. The host
    /// uses this to validate workspace routes at launch and to surface the
    /// missing-secret modal proactively. Empty when omitted.
    #[serde(default)]
    pub secrets: HashMap<String, SecretDecl>,
}

/// A `[secrets]` table entry from manifest.toml. `required` is **required**
/// (no serde default) — apps must explicitly state whether the host should
/// block on the missing-secret modal at launch.
#[derive(Deserialize, Debug, Clone)]
pub struct SecretDecl {
    pub required: bool,
    #[serde(default)]
    pub description: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct AppManifestApp {
    pub id: String,
    pub name: String,
    pub entry: String,
    /// Required. Selects host rendering / pane behaviour:
    ///
    /// - `App` (`type = "app"`): the host renders the app's draw canvas. The
    ///   default for nearly every Plexi app — UI is freeform, the app emits
    ///   `Rect`/`Text`/etc.
    ///
    /// No `serde(default)` — every manifest must declare its type explicitly.
    /// Discipline matches `schema_version` (issue #308 Phase 2).
    #[serde(rename = "type")]
    pub manifest_type: ManifestType,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub capabilities: AppCapabilities,
    /// Hot-reload opt-in (#83). When true AND the app was discovered from a
    /// workspace-local `.plexi/apps/`, the host watches the app dir and
    /// reloads the subprocess on save. Off by default — global installs
    /// never auto-reload regardless of this field.
    ///
    /// Modelled as `Option<bool>` (no `serde(default)`): missing → `None`,
    /// which the host treats as `false`. Standard pattern for an explicitly
    /// optional field whose absence has a meaningful default. Distinct from
    /// `serde(default)`, which would conflate "absent" with "false" at the
    /// type level and lose the diagnostic that the field was never set.
    pub watch: Option<bool>,
    /// Optional `[app.mcp]` section. When present the host starts an HTTP MCP server
    /// on a dynamic port and injects PLEXI_MCP_PORT into the app's environment.
    #[serde(default)]
    pub mcp: Option<McpSection>,
}

/// Manifest `[app] type` field — chooses the host rendering surface for
/// the pane. Required; no `serde(default)`.
#[derive(Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManifestType {
    /// Standard draw-canvas app. Host renders whatever the app draws.
    App,
}

/// Newtype for `[launch] notification_scope` in manifest.toml. Deserialises
/// from `"window"` | `"context"` | `"global"`. Default is `window` — the most
/// restrictive scope and the pre-525 behaviour for apps that don't declare it.
#[derive(Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "snake_case")]
pub enum DefaultNotifyScope {
    #[default]
    Window,
    Context,
    Global,
}

impl From<DefaultNotifyScope> for crate::app_protocol::NotifyScope {
    fn from(d: DefaultNotifyScope) -> Self {
        match d {
            DefaultNotifyScope::Window => crate::app_protocol::NotifyScope::Window,
            DefaultNotifyScope::Context => crate::app_protocol::NotifyScope::Context,
            DefaultNotifyScope::Global => crate::app_protocol::NotifyScope::Global,
        }
    }
}

/// Manifest `[app.mcp.tools]` entry — one tool the app exposes to external MCP clients.
#[derive(Deserialize, Debug, Clone)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// Manifest `[app.mcp]` section. Presence means the host starts an HTTP MCP server.
#[derive(Deserialize, Debug, Clone)]
pub struct McpSection {
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tools: Vec<McpTool>,
}

/// v3 capability section — `[app.capabilities]`. Holds only the capability
/// string list and the optional `file_types` extension map. Launch-time
/// layout + grouping moved to `[launch]` (see `LaunchSection`).
#[derive(Deserialize, Debug, Clone, Default)]
pub struct AppCapabilities {
    #[serde(default)]
    pub file_types: Vec<String>,
    /// v3 capability strings. Valid values: fs.read, fs.write, net.http,
    /// secrets.get, pipe.open, spawn.app, audio.record, audio.playback,
    /// video.playback. Unknown values cause install to fail (STEP-7).
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Hosts this app is allowed to reach via net.http.
    /// Empty list = unrestricted (allow any host).
    /// Patterns: exact hostname ("api.github.com") or wildcard ("*.wikipedia.org").
    #[serde(default)]
    pub allowed_hosts: Vec<String>,
}

/// v3 launch section — `[launch]`. Controls pane placement, share, grouping,
/// and keyboard capture when the host spawns this app. All fields optional.
#[derive(Deserialize, Debug, Clone, Default)]
pub struct LaunchSection {
    /// Pane group this app joins at spawn. When any pane in the group reports
    /// a CWD change, every member receives `PlexiEvent::PathChanged { cwd }`.
    /// Convention: "cwd" for generic directory-synced apps.
    #[serde(default)]
    pub join_group: Option<String>,
    /// Preferred pane layout. When absent, the host defaults to `overlay` (full pane takeover).
    #[serde(default)]
    pub layout_hint: Option<LayoutHint>,
    /// If true, this app captures all keyboard input when focused. Host
    /// shortcuts (Cmd+HJKL, Cmd+Enter, etc.) are suppressed; only Cmd+Q and
    /// Cmd+W remain.
    #[serde(default)]
    pub keyboard_capture: bool,
    /// If true, this app's process survives pane close and can be re-attached.
    /// The host will not kill the subprocess when the pane is closed;
    /// instead it parks the process in a background registry keyed by app_id.
    #[serde(default)]
    pub background: bool,
    /// Where this app's notifications surface. `"window"` (default) shows
    /// notifications only when the app's window is active. `"context"` shows
    /// them whenever the user is in the same sidebar context. `"global"` always
    /// surfaces them regardless of active context — use for stand-up reminders,
    /// timers, and monitoring dashboards.
    #[serde(default)]
    pub notification_scope: DefaultNotifyScope,
}

/// Structured layout hint. `side` ∈ {`"right"`, `"below"`, `"overlay"`}.
/// `split` is the fraction of the parent container given to the new pane
/// on open — must be in (0.0, 1.0). Default: 0.5.
#[derive(Deserialize, Debug, Clone)]
pub struct LayoutHint {
    pub side: String,
    #[serde(default = "default_split")]
    pub split: f32,
}

fn default_split() -> f32 {
    0.5
}

impl AppCapabilities {
    /// Convert manifest-declared capabilities to runtime permissions.
    pub fn to_permissions(&self) -> crate::app_permissions::AppPermissions {
        let mut perms = crate::app_permissions::AppPermissions::from_capability_strings(&self.capabilities);
        perms.allowed_hosts = self.allowed_hosts.clone();
        perms
    }
}

/// Where a discovered registry entry came from. Used for shadow-logging at
/// `info` level so users can trace which copy of an id won discovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistrySource {
    /// `~/.plexi-<channel>/apps/<id>/`
    Global,
    /// `<workspace_root>/.plexi/apps/<id>/`
    LocalApp,
    /// `<workspace_root>/.plexi/agents/<id>/`
    LocalAgent,
    /// Linked via `.plexi/links.toml` — absolute path registered by `plexi app link`
    Linked,
}

impl RegistrySource {
    fn label(self) -> &'static str {
        match self {
            RegistrySource::Global => "global",
            RegistrySource::LocalApp => "local-app",
            RegistrySource::LocalAgent => "local-agent",
            RegistrySource::Linked => "linked",
        }
    }
}

/// A discovered but not-yet-launched app.
#[derive(Debug, Clone)]
pub struct InstalledApp {
    pub manifest: AppManifestApp,
    pub launch: LaunchSection,
    /// Canonical secret names this app declares in its `[secrets]` table
    /// (issue #322). Used by the host to validate workspace routes and
    /// surface the missing-secret prompt at first launch.
    pub secrets: HashMap<String, SecretDecl>,
    pub bin_path: PathBuf,
    /// Which discovery layer this entry came from. Set by `scan_dir`; the
    /// value returned by `load_app` is a placeholder and is overwritten at
    /// insert time.
    pub source: RegistrySource,
}

pub struct AppRegistry {
    /// All discovered apps keyed by their id.
    apps: HashMap<String, InstalledApp>,
    /// Extension → app id mapping (first match wins).
    extension_map: HashMap<String, String>,
}

impl AppRegistry {
    /// Scan global apps then walk up from `cwd` looking for a `.plexi/` directory;
    /// if found, also scan its `apps/` and `agents/` subdirs. Local entries shadow
    /// global ones with the same id (a single info-level log line per shadow).
    pub fn load(cwd: &Path) -> Self {
        let global_dir = apps_dir();
        if !global_dir.exists() {
            if let Err(e) = std::fs::create_dir_all(&global_dir) {
                log::warn!("AppRegistry: could not create global apps dir: {e}");
            }
        }
        Self::load_with_global(cwd, &global_dir)
    }

    /// Same as [`load`], but with an explicit global apps directory. Used by
    /// tests so they can stage a fake `~/.plexi-<channel>/apps/` without
    /// touching the real one.
    pub fn load_with_global(cwd: &Path, global_dir: &Path) -> Self {
        let mut registry = Self {
            apps: HashMap::new(),
            extension_map: HashMap::new(),
        };

        if global_dir.is_dir() {
            registry.scan_dir(global_dir, RegistrySource::Global);
        }

        // Local apps + agents — only scanned when a workspace root exists.
        // `.plexi/apps/` is scanned first, then `.plexi/agents/`; both shadow
        // global, and a colliding id between local apps and local agents lets
        // the agent win (scanned later).
        if let Some(root) = resolve_workspace_root(cwd) {
            let local_apps = root.join(".plexi").join("apps");
            if local_apps.is_dir() {
                registry.scan_dir(&local_apps, RegistrySource::LocalApp);
            }
            let local_agents = root.join(".plexi").join("agents");
            if local_agents.is_dir() {
                registry.scan_dir(&local_agents, RegistrySource::LocalAgent);
            }
            // Linked apps — registered via `plexi app link`, stored as absolute paths in
            // `.plexi/links.toml`. Scanned last — linked entries shadow all other sources.
            let links_path = root.join(".plexi").join("links.toml");
            if links_path.exists() {
                registry.scan_links(&links_path);
            }
        }

        registry
    }

    /// Look up an installed entry by id (returns `None` if not discovered).
    pub fn get(&self, id: &str) -> Option<&InstalledApp> {
        self.apps.get(id)
    }

    /// Scan one directory of manifest-bearing subdirs, inserting discovered
    /// entries. Calls made later in `load()` shadow earlier ones; on shadow
    /// the displaced entry's source is logged so users can debug discovery.
    fn scan_dir(&mut self, dir: &Path, source: RegistrySource) {
        let read_dir = match std::fs::read_dir(dir) {
            Ok(d) => d,
            Err(e) => {
                log::warn!("AppRegistry: failed to read {:?}: {e}", dir);
                return;
            }
        };

        for entry in read_dir.flatten() {
            let entry_dir = entry.path();
            if !entry_dir.is_dir() {
                continue;
            }

            match self.load_app(&entry_dir) {
                Ok(installed) => {
                    let id = installed.manifest.id.clone();
                    if let Some(existing) = self.apps.get(&id) {
                        if source != existing.source {
                            log::warn!(
                                "AppRegistry: '{}' — {} entry (from {:?}) shadows {} entry (from {:?})",
                                id,
                                source.label(),
                                entry_dir,
                                existing.source.label(),
                                existing.bin_path.parent().unwrap_or(&existing.bin_path),
                            );
                        } else {
                            log::debug!(
                                "AppRegistry: {} entry '{}' (from {:?}) shadows {} entry from {:?}",
                                source.label(),
                                id,
                                entry_dir,
                                existing.source.label(),
                                existing.bin_path.parent().unwrap_or(&existing.bin_path),
                            );
                        }
                    } else {
                        log::debug!(
                            "AppRegistry: loaded {} entry '{}' from {:?}",
                            source.label(),
                            id,
                            entry_dir,
                        );
                    }
                    for ext in &installed.manifest.capabilities.file_types {
                        // Plain insert — local entries override global for extension map too.
                        self.extension_map.insert(ext.to_lowercase(), id.clone());
                    }
                    self.apps.insert(id, InstalledApp { source, ..installed });
                }
                Err(e) => {
                    log::warn!("AppRegistry: skipping {:?}: {e}", entry_dir.file_name());
                }
            }
        }
    }

    /// Parse `.plexi/links.toml` and load each listed absolute path as a linked app.
    /// links.toml format:
    /// ```toml
    /// links = ["/abs/path/to/app1", "/abs/path/to/app2"]
    /// ```
    fn scan_links(&mut self, links_path: &Path) {
        let content = match std::fs::read_to_string(links_path) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("AppRegistry: failed to read {:?}: {e}", links_path);
                return;
            }
        };
        #[derive(serde::Deserialize)]
        struct LinksFile {
            #[serde(default)]
            links: Vec<String>,
        }
        let parsed: LinksFile = match toml::from_str(&content) {
            Ok(p) => p,
            Err(e) => {
                log::warn!("AppRegistry: failed to parse {:?}: {e}", links_path);
                return;
            }
        };
        // Profile dir — linked paths must not point inside it. A path inside
        // the global config dir would silently override a managed install with
        // arbitrary code from a local workspace's links.toml.
        // Canonicalize so symlinks (e.g. macOS /var → /private/var) don't
        // allow a bypass via the real resolved path.
        let raw_profile = crate::config::config_dir();
        let profile_dir = raw_profile.canonicalize().unwrap_or(raw_profile);

        for raw_path in &parsed.links {
            let app_dir = PathBuf::from(raw_path);
            if !app_dir.is_absolute() {
                log::warn!("AppRegistry: skipping relative path in links.toml: {:?} (must be absolute)", raw_path);
                continue;
            }
            // Canonicalize to resolve symlinks before the profile-dir check.
            let canonical = match app_dir.canonicalize() {
                Ok(p) => p,
                Err(e) => {
                    log::warn!("AppRegistry: skipping linked path {:?}: cannot canonicalize: {e}", raw_path);
                    continue;
                }
            };
            if canonical.starts_with(&profile_dir) {
                log::warn!(
                    "AppRegistry: skipping linked path {:?}: resolves inside profile dir {:?} — use global install instead",
                    raw_path,
                    profile_dir,
                );
                continue;
            }
            match self.load_app(&canonical) {
                Ok(installed) => {
                    let id = installed.manifest.id.clone();
                    if let Some(existing) = self.apps.get(&id) {
                        log::warn!(
                            "AppRegistry: '{}' — linked entry (from {:?}) shadows {} entry (from {:?})",
                            id,
                            canonical,
                            existing.source.label(),
                            existing.bin_path.parent().unwrap_or(&existing.bin_path),
                        );
                    } else {
                        log::info!(
                            "AppRegistry: loaded linked entry '{}' from {:?}",
                            id,
                            canonical,
                        );
                    }
                    for ext in &installed.manifest.capabilities.file_types {
                        self.extension_map.insert(ext.to_lowercase(), id.clone());
                    }
                    self.apps.insert(id, InstalledApp { source: RegistrySource::Linked, ..installed });
                }
                Err(e) => {
                    log::warn!("AppRegistry: skipping linked path {:?}: {e}", canonical);
                }
            }
        }
    }

    pub(crate) fn load_app(&self, app_dir: &PathBuf) -> Result<InstalledApp, String> {
        let manifest_path = app_dir.join("manifest.toml");
        let manifest_str = std::fs::read_to_string(&manifest_path)
            .map_err(|e| format!("no manifest.toml: {e}"))?;

        let manifest: AppManifest =
            toml::from_str(&manifest_str).map_err(|e| format!("invalid manifest: {e}"))?;

        if manifest.schema_version > MANIFEST_SCHEMA_VERSION {
            return Err(format!(
                "manifest schema_version = {} is newer than this Plexi build supports (max {}); \
                 update Plexi to install this app",
                manifest.schema_version, MANIFEST_SCHEMA_VERSION
            ));
        }

        // STEP-7: refuse to install an app whose declared capabilities include
        // any unknown string. Silent `→ FsRead` fallback was removed in STEP-2;
        // this replaces it with a loud install-time failure.
        if let Err(e) = crate::app_permissions::parse_capability_strings(
            &manifest.app.capabilities.capabilities,
        ) {
            return Err(format!(
                "manifest lists {e}; valid values: {}",
                crate::app_permissions::Capability::all_str_values().join(", ")
            ));
        }

        // STEP-8: validate layout_hint.side now so bad manifests fail at
        // install rather than at first pane open.
        if let Some(hint) = &manifest.launch.layout_hint {
            match hint.side.as_str() {
                "right" | "below" | "above" | "overlay" => {}
                other => {
                    return Err(format!(
                        "layout_hint.side must be 'right', 'below', 'above', or 'overlay'; got '{other}'"
                    ));
                }
            }
            if !(0.0 < hint.split && hint.split < 1.0) {
                return Err(format!(
                    "layout_hint.split must be in (0.0, 1.0); got {}",
                    hint.split
                ));
            }
        }

        let bin_path = resolve_entry(app_dir, &manifest.app.entry)?;

        for (name, decl) in &manifest.secrets {
            log::debug!(
                "AppRegistry: {} declares secret '{name}' (required={}, description=\"{}\")",
                manifest.app.id,
                decl.required,
                decl.description,
            );
        }

        Ok(InstalledApp {
            manifest: manifest.app,
            launch: manifest.launch,
            secrets: manifest.secrets,
            bin_path,
            // Placeholder — `scan_dir` overwrites this with the real source.
            source: RegistrySource::Global,
        })
    }


    /// List all installed apps.
    pub fn list(&self) -> Vec<&InstalledApp> {
        let mut apps: Vec<_> = self.apps.values().collect();
        apps.sort_by_key(|a| &a.manifest.name);
        apps
    }

    /// Returns true if the app's process should survive pane close (`[launch].background`).
    pub fn is_background(&self, app_id: &str) -> bool {
        self.apps.get(app_id).map(|a| a.launch.background).unwrap_or(false)
    }

    /// Get the manifest-declared pane group (`[launch].join_group`).
    pub fn group_for(&self, app_id: &str) -> Option<String> {
        self.apps.get(app_id).and_then(|a| a.launch.join_group.clone())
    }

    /// Get the launch-time layout side hint: "right" | "below" | "above" | "overlay".
    /// Internally mapped to the `split_h` / `split_v` / `split_above` strings pane_ops uses.
    pub fn layout_hint_for(&self, app_id: &str) -> Option<String> {
        self.apps
            .get(app_id)
            .and_then(|a| a.launch.layout_hint.as_ref())
            .map(|h| match h.side.as_str() {
                "below" => "split_v".to_string(),
                "above" => "split_above".to_string(),
                "overlay" => "overlay".to_string(),
                _ => "split_h".to_string(),
            })
    }

    /// Returns true when the app's manifest sets `[app] watch = true`.
    /// The file watcher only needs the resolved app directory — discovery
    /// location is irrelevant.
    pub fn watch_eligible(&self, app_id: &str) -> bool {
        self.apps
            .get(app_id)
            .map(|a| a.manifest.watch.unwrap_or(false))
            .unwrap_or(false)
    }

    /// Path to the app's installation directory (parent of `bin_path`).
    /// Used by the file watcher; returns `None` when the app id is unknown.
    pub fn app_dir_for(&self, app_id: &str) -> Option<PathBuf> {
        self.apps
            .get(app_id)
            .and_then(|a| a.bin_path.parent().map(Path::to_path_buf))
    }

    /// Get the manifest-declared layout_hint.split fraction (None if unset).
    pub fn share_for(&self, app_id: &str) -> Option<f32> {
        self.apps
            .get(app_id)
            .and_then(|a| a.launch.layout_hint.as_ref())
            .map(|h| h.split)
    }

    /// Return the manifest-declared notification scope for an app.
    /// Defaults to `Window` when the manifest omits `[launch] notification_scope`.
    pub fn default_notification_scope_for(
        &self,
        app_id: &str,
    ) -> crate::app_protocol::NotifyScope {
        self.apps
            .get(app_id)
            .map(|a| a.launch.notification_scope.clone().into())
            .unwrap_or(crate::app_protocol::NotifyScope::Window)
    }

    /// Check whether an app's declared capabilities are satisfiable by the
    /// current config. Returns human-readable missing-capability descriptions.
    /// Empty = all satisfied. Does not spawn the process.
    pub fn check_config_capabilities(
        &self,
        id: &str,
        config: &crate::config::PlexiConfig,
    ) -> Vec<String> {
        let Some(installed) = self.apps.get(id) else {
            return vec![];
        };
        let mut missing = Vec::new();
        for cap_str in &installed.manifest.capabilities.capabilities {
            let Ok(cap) = crate::app_permissions::Capability::try_from(cap_str.as_str()) else {
                continue;
            };
            if let Some(reason) = cap.config_missing_reason(config) {
                missing.push(reason);
            }
        }
        missing
    }

    /// Launch an app process for the given id.
    pub fn launch_process(&self, id: &str, cwd: &PathBuf, args: &[String]) -> Option<ProcessApp> {
        let installed = self.apps.get(id)?;
        let perms = installed.manifest.capabilities.to_permissions();
        let caps = perms.capabilities.clone();
        let keyboard_capture = installed.launch.keyboard_capture;
        let default_scope = self.default_notification_scope_for(id);

        log::info!(
            "AppRegistry: launching '{}' as type={:?} source={}",
            id,
            installed.manifest.manifest_type,
            installed.source.label(),
        );

        // Issue #322: log declared-but-routed status for visibility. The
        // missing-secret prompt fires lazily on first `ctx.secret(...)` call —
        // this just makes the manifest's contract observable in the host log.
        if !installed.secrets.is_empty() {
            let required: Vec<&str> = installed
                .secrets
                .iter()
                .filter(|(_, d)| d.required)
                .map(|(k, _)| k.as_str())
                .collect();
            log::info!(
                "AppRegistry: launching '{id}' with declared secrets {:?} (required: {:?})",
                installed.secrets.keys().collect::<Vec<_>>(),
                required,
            );
        }
        match ProcessApp::launch(
            installed.manifest.id.clone(),
            installed.manifest.name.clone(),
            &installed.bin_path,
            cwd,
            args,
            cwd.clone(),
            caps,
            keyboard_capture,
            installed.manifest.mcp.as_ref(),
        ) {
            Ok(mut app) => {
                app.permissions.allowed_hosts = perms.allowed_hosts;
                log::info!(
                    "AppRegistry: launched '{}' from {:?} (notification_scope={:?}, allowed_hosts={:?})",
                    id,
                    installed.bin_path,
                    default_scope,
                    app.permissions.allowed_hosts,
                );
                Some(app)
            }
            Err(e) => {
                log::error!("AppRegistry: failed to launch '{}': {e}", id);
                None
            }
        }
    }

    /// Launch an app and return a boxed `App` trait object.
    pub fn launch(&self, id: &str, cwd: &PathBuf, args: &[String]) -> Option<Box<dyn App>> {
        self.launch_process(id, cwd, args)
            .map(|app| Box::new(app) as Box<dyn App>)
    }
}

/// Returns the path to the global apps directory: `~/.plexi/apps/`.
pub fn apps_dir() -> PathBuf {
    crate::config::config_dir().join("apps")
}

/// Walk up from `start` toward the filesystem root looking for the nearest
/// ancestor that contains a `.plexi/` directory. Returns that ancestor (the
/// workspace root), or `None` if no `.plexi/` is found before the root.
///
/// The home directory is **not** treated as a workspace root unless it
/// contains a `.plexi/` itself — `~/.plexi-<channel>/` is the global config
/// dir, which lives next to `~`, not inside it.
pub fn resolve_workspace_root(start: &Path) -> Option<PathBuf> {
    let home = dirs::home_dir();
    let mut current = start.to_path_buf();
    loop {
        // Home dir is never a workspace root. Check this BEFORE .plexi so that
        // ~/.plexi/ (the stable global config dir) doesn't trigger a false positive
        // when the focused pane is at ~/. Without this guard ordering, PR/alpha builds
        // would silently load stable app code from ~/.plexi/apps/ instead of their own
        // channel's apps whenever the terminal cwd is ~/. (issue #1064)
        if let Some(ref h) = home {
            if current == *h {
                return None;
            }
        }
        if current.join(".plexi").is_dir() {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}

/// Resolve the `entry` field from manifest.toml to a path.
/// Fails fast — no guessing, no fallbacks.
pub(crate) fn resolve_entry(app_dir: &PathBuf, entry: &str) -> Result<PathBuf, String> {
    let path = app_dir.join(entry);

    if !path.exists() {
        return Err(format!("entry '{entry}' not found in {:?}", app_dir));
    }

    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Create a manifest and entry script under `dir/<id>/`.
    /// `name` is what shows up in the manifest's `[app].name` so tests can
    /// distinguish two entries with the same id but different content.
    fn write_app(dir: &Path, id: &str, name: &str) {
        write_app_with_type(dir, id, name, "app");
    }

    fn write_app_with_type(dir: &Path, id: &str, name: &str, manifest_type: &str) {
        let app_dir = dir.join(id);
        fs::create_dir_all(&app_dir).unwrap();
        let manifest = format!(
            "schema_version = 1\n\n[app]\nid = \"{id}\"\ntype = \"{manifest_type}\"\nname = \"{name}\"\nversion = \"0.0.1\"\nentry = \"run.sh\"\n"
        );
        fs::write(app_dir.join("manifest.toml"), manifest).unwrap();
        let entry = app_dir.join("run.sh");
        fs::write(&entry, "#!/bin/sh\nexit 0\n").unwrap();
    }

    #[test]
    fn local_app_shadows_global_with_same_id() {
        let global = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let local_apps = workspace.path().join(".plexi").join("apps");
        fs::create_dir_all(&local_apps).unwrap();

        write_app(global.path(), "foo", "Global Foo");
        write_app(&local_apps, "foo", "Local Foo");

        let registry = AppRegistry::load_with_global(workspace.path(), global.path());
        let foo = registry.get("foo").expect("foo should be discovered");
        assert_eq!(foo.manifest.name, "Local Foo");
        assert_eq!(foo.source, RegistrySource::LocalApp);
    }

    #[test]
    fn py_entry_loads_without_executable_bit() {
        let global = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let app_dir = global.path().join("myapp");
        fs::create_dir_all(&app_dir).unwrap();
        let manifest = "schema_version = 1\n\n[app]\nid = \"myapp\"\ntype = \"app\"\nname = \"My App\"\nversion = \"0.0.1\"\nentry = \"app.py\"\n";
        fs::write(app_dir.join("manifest.toml"), manifest).unwrap();
        // Write entry without shebang or executable bit.
        fs::write(app_dir.join("app.py"), "import plexi\n").unwrap();

        let registry = AppRegistry::load_with_global(workspace.path(), global.path());
        let app = registry.get("myapp").expect("app.py entry must load without chmod+x");
        assert!(app.bin_path.ends_with("app.py"));
    }

    #[test]
    fn global_only_app_still_discovered_when_workspace_open() {
        let global = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        // Workspace exists (.plexi/ dir) but has no local apps/agents.
        fs::create_dir_all(workspace.path().join(".plexi")).unwrap();

        write_app(global.path(), "global-only", "Global Only");

        let registry = AppRegistry::load_with_global(workspace.path(), global.path());
        let entry = registry.get("global-only").expect("global app should appear");
        assert_eq!(entry.source, RegistrySource::Global);
    }

    #[test]
    fn workspace_root_resolved_from_deep_descendant() {
        let workspace = tempfile::tempdir().unwrap();
        fs::create_dir_all(workspace.path().join(".plexi")).unwrap();
        let deep = workspace.path().join("a").join("b").join("c");
        fs::create_dir_all(&deep).unwrap();

        let resolved = resolve_workspace_root(&deep).expect("should find ancestor");
        // Canonicalize both sides — tempfile paths on macOS go through /var → /private/var.
        assert_eq!(
            resolved.canonicalize().unwrap(),
            workspace.path().canonicalize().unwrap()
        );
    }

    #[test]
    fn workspace_root_returns_none_when_no_dot_plexi() {
        let bare = tempfile::tempdir().unwrap();
        assert!(resolve_workspace_root(bare.path()).is_none());
    }

    #[test]
    fn home_with_dot_plexi_is_not_a_workspace_root() {
        // Regression for issue #1064: ~/.plexi/ (stable profile dir) must NOT cause
        // home to be returned as a workspace root. The home-dir stop must fire before
        // the .plexi check, not after.
        use std::sync::Mutex;
        static ENV_LOCK: Mutex<()> = Mutex::new(());
        let _guard = ENV_LOCK.lock().unwrap();

        let fake_home = tempfile::tempdir().unwrap();
        fs::create_dir_all(fake_home.path().join(".plexi")).unwrap();
        let original = std::env::var("HOME").ok();
        // SAFETY: serialised via ENV_LOCK
        unsafe { std::env::set_var("HOME", fake_home.path()) };
        let result = resolve_workspace_root(fake_home.path());
        match original {
            Some(h) => unsafe { std::env::set_var("HOME", h) },
            None => unsafe { std::env::remove_var("HOME") },
        }
        assert!(result.is_none(), "home dir must not be a workspace root even when .plexi exists");
    }

    #[test]
    fn agents_directory_is_discovered() {
        let global = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let local_agents = workspace.path().join(".plexi").join("agents");
        fs::create_dir_all(&local_agents).unwrap();

        write_app(&local_agents, "code-reviewer", "Code Reviewer");

        let registry = AppRegistry::load_with_global(workspace.path(), global.path());
        let agent = registry
            .get("code-reviewer")
            .expect("agent should be discovered");
        assert_eq!(agent.source, RegistrySource::LocalAgent);
    }

    #[test]
    fn manifest_with_schema_version_loads() {
        let global = tempfile::tempdir().unwrap();
        let bare = tempfile::tempdir().unwrap();
        write_app(global.path(), "ok-app", "Ok App");

        let registry = AppRegistry::load_with_global(bare.path(), global.path());
        assert!(registry.get("ok-app").is_some());
    }

    #[test]
    fn manifest_with_future_schema_version_refuses() {
        let global = tempfile::tempdir().unwrap();
        let app_dir = global.path().join("future-app");
        fs::create_dir_all(&app_dir).unwrap();
        let manifest = format!(
            "schema_version = {}\n\n[app]\nid = \"future-app\"\ntype = \"app\"\nname = \"Future\"\nversion = \"0.0.1\"\nentry = \"run.sh\"\n",
            MANIFEST_SCHEMA_VERSION + 1
        );
        fs::write(app_dir.join("manifest.toml"), manifest).unwrap();
        let entry = app_dir.join("run.sh");
        fs::write(&entry, "#!/bin/sh\nexit 0\n").unwrap();

        let bare = tempfile::tempdir().unwrap();
        let registry = AppRegistry::load_with_global(bare.path(), global.path());
        assert!(
            registry.get("future-app").is_none(),
            "future schema_version manifest must be skipped"
        );
    }

    #[test]
    fn manifest_missing_schema_version_errors() {
        // Direct unit test of the manifest deserialiser: a manifest without
        // `schema_version` must fail to parse — no serde-default fallback.
        let raw = r#"
[app]
id = "no-version"
name = "No Version"
version = "0.0.1"
entry = "run.sh"
"#;
        let parsed: Result<AppManifest, _> = toml::from_str(raw);
        assert!(
            parsed.is_err(),
            "manifest missing schema_version must be rejected, got: {parsed:?}"
        );
    }

    // ── v3.3 agent-as-app manifest type field (#285) ─────────────────────

    #[test]
    fn manifest_with_type_app_loads() {
        let global = tempfile::tempdir().unwrap();
        let bare = tempfile::tempdir().unwrap();
        write_app_with_type(global.path(), "regular-app", "Regular", "app");

        let registry = AppRegistry::load_with_global(bare.path(), global.path());
        let entry = registry
            .get("regular-app")
            .expect("type=app manifest should load");
        assert_eq!(entry.manifest.manifest_type, ManifestType::App);
    }

    #[test]
    fn manifest_missing_type_field_errors() {
        // No `type` field — must fail to parse. Required field, no
        // `serde(default)`. Discipline matches `schema_version`.
        let raw = r#"
schema_version = 1

[app]
id = "no-type"
name = "No Type"
version = "0.0.1"
entry = "run.sh"
"#;
        let parsed: Result<AppManifest, _> = toml::from_str(raw);
        assert!(
            parsed.is_err(),
            "manifest missing `type` must be rejected, got: {parsed:?}"
        );
    }

    #[test]
    fn manifest_with_unknown_type_errors() {
        // `type = "wizard"` — must fail to parse. Only `app` is valid; `agent`
        // and other values should not silently fall back.
        let raw = r#"
schema_version = 1

[app]
id = "unknown-type"
type = "wizard"
name = "Wizard"
version = "0.0.1"
entry = "run.sh"
"#;
        let parsed: Result<AppManifest, _> = toml::from_str(raw);
        assert!(
            parsed.is_err(),
            "manifest with unknown type variant must be rejected, got: {parsed:?}"
        );
    }

    // ── #83 hot reload — manifest `[app] watch` field ────────────────────

    #[test]
    fn manifest_with_watch_true_loads() {
        let global = tempfile::tempdir().unwrap();
        let bare = tempfile::tempdir().unwrap();
        let app_dir = global.path().join("watching-app");
        fs::create_dir_all(&app_dir).unwrap();
        let manifest = "\
schema_version = 1

[app]
id = \"watching-app\"
type = \"app\"
name = \"Watching\"
version = \"0.0.1\"
entry = \"run.sh\"
watch = true
";
        fs::write(app_dir.join("manifest.toml"), manifest).unwrap();
        let entry = app_dir.join("run.sh");
        fs::write(&entry, "#!/bin/sh\nexit 0\n").unwrap();

        let registry = AppRegistry::load_with_global(bare.path(), global.path());
        let app = registry.get("watching-app").expect("manifest with watch=true should load");
        assert_eq!(app.manifest.watch, Some(true));
    }

    #[test]
    fn manifest_with_watch_field_absent_treats_as_false() {
        // The default `write_app` helper omits `watch` — exercise that path.
        let global = tempfile::tempdir().unwrap();
        let bare = tempfile::tempdir().unwrap();
        write_app(global.path(), "no-watch", "No Watch");

        let registry = AppRegistry::load_with_global(bare.path(), global.path());
        let app = registry.get("no-watch").expect("default manifest should load");
        assert_eq!(app.manifest.watch, None);
        // And `watch_eligible` returns false (also exercises the absent
        // path on the public API).
        assert!(!registry.watch_eligible("no-watch"));
    }

    #[test]
    fn watch_field_engages_for_any_source_when_opted_in() {
        // Any app with `watch = true` in the manifest is eligible for hot reload
        // regardless of whether it was discovered globally or workspace-locally.
        let global = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let local_apps = workspace.path().join(".plexi").join("apps");
        fs::create_dir_all(&local_apps).unwrap();

        // Global copy with watch=true.
        let g_dir = global.path().join("dual-install");
        fs::create_dir_all(&g_dir).unwrap();
        let manifest_g = "\
schema_version = 1

[app]
id = \"global-watcher\"
type = \"app\"
name = \"Global Watcher\"
version = \"0.0.1\"
entry = \"run.sh\"
watch = true
";
        fs::write(g_dir.join("manifest.toml"), manifest_g).unwrap();
        let entry_g = g_dir.join("run.sh");
        fs::write(&entry_g, "#!/bin/sh\nexit 0\n").unwrap();

        // Local copy with watch=true (different id so they don't shadow).
        let l_dir = local_apps.join("local-watcher");
        fs::create_dir_all(&l_dir).unwrap();
        let manifest_l = "\
schema_version = 1

[app]
id = \"local-watcher\"
type = \"app\"
name = \"Local Watcher\"
version = \"0.0.1\"
entry = \"run.sh\"
watch = true
";
        fs::write(l_dir.join("manifest.toml"), manifest_l).unwrap();
        let entry_l = l_dir.join("run.sh");
        fs::write(&entry_l, "#!/bin/sh\nexit 0\n").unwrap();

        let registry = AppRegistry::load_with_global(workspace.path(), global.path());
        assert!(
            registry.watch_eligible("global-watcher"),
            "global install with watch=true must be eligible"
        );
        assert!(
            registry.watch_eligible("local-watcher"),
            "workspace-local install with watch=true must be eligible"
        );
    }

    #[test]
    fn no_workspace_means_only_global_apps_load() {
        let global = tempfile::tempdir().unwrap();
        let bare = tempfile::tempdir().unwrap();

        write_app(global.path(), "g", "Global");

        let registry = AppRegistry::load_with_global(bare.path(), global.path());
        assert!(registry.get("g").is_some());
        assert_eq!(registry.list().len(), 1);
    }

    // ── #525 notification scope — `[launch] notification_scope` ─────────────

    #[test]
    fn manifest_without_notification_scope_defaults_to_window() {
        // Apps that omit `notification_scope` must default to `Window` —
        // the pre-525 behaviour (no change for existing apps).
        let global = tempfile::tempdir().unwrap();
        let bare = tempfile::tempdir().unwrap();
        write_app(global.path(), "no-scope", "No Scope");

        let registry = AppRegistry::load_with_global(bare.path(), global.path());
        let scope = registry.default_notification_scope_for("no-scope");
        assert_eq!(scope, crate::app_protocol::NotifyScope::Window);
    }

    #[test]
    fn manifest_with_notification_scope_global_loads() {
        let global = tempfile::tempdir().unwrap();
        let bare = tempfile::tempdir().unwrap();
        let app_dir = global.path().join("stand-up");
        fs::create_dir_all(&app_dir).unwrap();
        let manifest = "\
schema_version = 1

[app]
id = \"stand-up\"
type = \"app\"
name = \"Stand Up\"
version = \"0.0.1\"
entry = \"run.sh\"

[launch]
notification_scope = \"global\"
";
        fs::write(app_dir.join("manifest.toml"), manifest).unwrap();
        let entry = app_dir.join("run.sh");
        fs::write(&entry, "#!/bin/sh\nexit 0\n").unwrap();

        let registry = AppRegistry::load_with_global(bare.path(), global.path());
        let scope = registry.default_notification_scope_for("stand-up");
        assert_eq!(scope, crate::app_protocol::NotifyScope::Global);
    }

    #[test]
    fn linked_app_appears_in_registry() {
        let global = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        // Create workspace .plexi dir
        fs::create_dir_all(workspace.path().join(".plexi")).unwrap();
        // Create app directory outside .plexi/ — write_app creates <parent>/<id>/ subdir
        let apps_parent = workspace.path().join("apps");
        fs::create_dir_all(&apps_parent).unwrap();
        write_app(&apps_parent, "linked-app", "Linked App");
        let app_dir = apps_parent.join("linked-app");
        // Write links.toml
        let links_toml = format!("links = [{:?}]\n", app_dir.to_string_lossy());
        fs::write(workspace.path().join(".plexi").join("links.toml"), links_toml).unwrap();

        let registry = AppRegistry::load_with_global(workspace.path(), global.path());
        let app = registry.get("linked-app").expect("linked app must appear in registry");
        assert_eq!(app.source, RegistrySource::Linked);
    }

    #[test]
    fn linked_app_shadows_global_with_same_id() {
        let global = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        // Global install of "my-app"
        write_app(global.path(), "my-app", "Global My App");
        // Workspace .plexi dir
        fs::create_dir_all(workspace.path().join(".plexi")).unwrap();
        // Linked version of same id — write_app creates <parent>/<id>/ subdir
        let apps_parent = workspace.path().join("apps");
        fs::create_dir_all(&apps_parent).unwrap();
        write_app(&apps_parent, "my-app", "Linked My App");
        let linked_app = apps_parent.join("my-app");
        let links_toml = format!("links = [{:?}]\n", linked_app.to_string_lossy());
        fs::write(workspace.path().join(".plexi").join("links.toml"), links_toml).unwrap();

        let registry = AppRegistry::load_with_global(workspace.path(), global.path());
        let app = registry.get("my-app").expect("my-app must be in registry");
        assert_eq!(app.source, RegistrySource::Linked, "linked must shadow global");
        assert_eq!(app.manifest.name, "Linked My App");
    }

    #[test]
    fn local_agent_shadows_local_app_with_same_id() {
        let global = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let local_apps = workspace.path().join(".plexi").join("apps");
        let local_agents = workspace.path().join(".plexi").join("agents");
        fs::create_dir_all(&local_apps).unwrap();
        fs::create_dir_all(&local_agents).unwrap();

        write_app(&local_apps, "tool", "Tool (app)");
        write_app(&local_agents, "tool", "Tool (agent)");

        let registry = AppRegistry::load_with_global(workspace.path(), global.path());
        let entry = registry.get("tool").expect("tool must be discovered");
        assert_eq!(entry.source, RegistrySource::LocalAgent, "agent must shadow local app");
        assert_eq!(entry.manifest.name, "Tool (agent)");
    }

    #[test]
    fn linked_app_relative_path_rejected() {
        let global = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        fs::create_dir_all(workspace.path().join(".plexi")).unwrap();

        // A relative path in links.toml must be skipped — only absolute paths allowed.
        let bad_links = "links = [\"relative/path/to/app\"]\n";
        fs::write(workspace.path().join(".plexi").join("links.toml"), bad_links).unwrap();

        let registry = AppRegistry::load_with_global(workspace.path(), global.path());
        assert!(registry.get("anything").is_none(), "relative linked path must be rejected");
    }

    #[test]
    fn linked_app_with_nonexistent_path_skipped() {
        let global = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        fs::create_dir_all(workspace.path().join(".plexi")).unwrap();

        let bad_links = "links = [\"/nonexistent/path/to/app\"]\n";
        fs::write(workspace.path().join(".plexi").join("links.toml"), bad_links).unwrap();

        // Must not panic — skips with a warn
        let registry = AppRegistry::load_with_global(workspace.path(), global.path());
        assert!(registry.get("anything").is_none());
    }

    #[test]
    fn manifest_with_notification_scope_context_loads() {
        let global = tempfile::tempdir().unwrap();
        let bare = tempfile::tempdir().unwrap();
        let app_dir = global.path().join("ctx-scoped");
        fs::create_dir_all(&app_dir).unwrap();
        let manifest = "\
schema_version = 1

[app]
id = \"ctx-scoped\"
type = \"app\"
name = \"Context Scoped\"
version = \"0.0.1\"
entry = \"run.sh\"

[launch]
notification_scope = \"context\"
";
        fs::write(app_dir.join("manifest.toml"), manifest).unwrap();
        let entry = app_dir.join("run.sh");
        fs::write(&entry, "#!/bin/sh\nexit 0\n").unwrap();

        let registry = AppRegistry::load_with_global(bare.path(), global.path());
        let scope = registry.default_notification_scope_for("ctx-scoped");
        assert_eq!(scope, crate::app_protocol::NotifyScope::Context);
    }
}
