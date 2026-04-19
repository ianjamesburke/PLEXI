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
    #[serde(default)]
    pub launch: LaunchSection,
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
    /// Preferred pane layout. Default: `LayoutHint { side: "right", split: 0.5 }`.
    #[serde(default)]
    pub layout_hint: Option<LayoutHint>,
    /// If true, this app captures all keyboard input when focused. Host
    /// shortcuts (Cmd+HJKL, Cmd+Enter, etc.) are suppressed; only Cmd+Q and
    /// Cmd+W remain.
    #[serde(default)]
    pub keyboard_capture: bool,
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
        crate::app_permissions::AppPermissions::from_capability_strings(&self.capabilities)
    }
}

/// A discovered but not-yet-launched app.
#[derive(Debug, Clone)]
pub struct InstalledApp {
    pub manifest: AppManifestApp,
    pub launch: LaunchSection,
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
                    log::info!(
                        "AppRegistry: loaded app '{}' from {:?}",
                        installed.manifest.id,
                        apps_dir
                    );
                    for ext in &installed.manifest.capabilities.file_types {
                        // Plain insert — local apps override global for extension map too.
                        self.extension_map
                            .insert(ext.to_lowercase(), installed.manifest.id.clone());
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

        let manifest: AppManifest =
            toml::from_str(&manifest_str).map_err(|e| format!("invalid manifest: {e}"))?;

        // STEP-7: refuse to install an app whose declared capabilities include
        // any unknown string. Silent `→ FsRead` fallback was removed in STEP-2;
        // this replaces it with a loud install-time failure.
        if let Err(e) = crate::app_permissions::parse_capability_strings(
            &manifest.app.capabilities.capabilities,
        ) {
            return Err(format!(
                "manifest lists {e}; valid values: fs.read, fs.write, net.http, \
                 secrets.get, pipe.open, spawn.app, audio.record, audio.playback, \
                 video.playback"
            ));
        }

        // STEP-8: validate layout_hint.side now so bad manifests fail at
        // install rather than at first pane open.
        if let Some(hint) = &manifest.launch.layout_hint {
            match hint.side.as_str() {
                "right" | "below" | "overlay" => {}
                other => {
                    return Err(format!(
                        "layout_hint.side must be 'right', 'below', or 'overlay'; got '{other}'"
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

        Ok(InstalledApp {
            manifest: manifest.app,
            launch: manifest.launch,
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
        self.apps
            .get(app_id)
            .map(|app| app.manifest.capabilities.to_permissions())
    }

    /// Get the manifest-declared pane group (`[launch].join_group`).
    pub fn group_for(&self, app_id: &str) -> Option<String> {
        self.apps.get(app_id).and_then(|a| a.launch.join_group.clone())
    }

    /// Get the manifest-declared keyboard_capture flag (`[launch].keyboard_capture`).
    pub fn keyboard_capture_for(&self, app_id: &str) -> bool {
        self.apps
            .get(app_id)
            .map(|a| a.launch.keyboard_capture)
            .unwrap_or(false)
    }

    /// Get the launch-time layout side hint: "right" | "below" | "overlay".
    /// Internally mapped to the `split_v` / `split_h` strings pane_ops uses.
    pub fn layout_hint_for(&self, app_id: &str) -> Option<String> {
        self.apps
            .get(app_id)
            .and_then(|a| a.launch.layout_hint.as_ref())
            .map(|h| match h.side.as_str() {
                "below" => "split_h".to_string(),
                "overlay" => "overlay".to_string(),
                _ => "split_v".to_string(),
            })
    }

    /// Get the manifest-declared layout_hint.split fraction (None if unset).
    pub fn share_for(&self, app_id: &str) -> Option<f32> {
        self.apps
            .get(app_id)
            .and_then(|a| a.launch.layout_hint.as_ref())
            .map(|h| h.split)
    }

    /// Launch an app process for the given id.
    pub fn launch_process(&self, id: &str, cwd: &PathBuf, args: &[String]) -> Option<ProcessApp> {
        let installed = self.apps.get(id)?;
        let perms = installed.manifest.capabilities.to_permissions();
        let caps = perms.capabilities.clone();
        let keyboard_capture = installed.launch.keyboard_capture;
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
                log::info!(
                    "AppRegistry: launched '{}' from {:?}",
                    id,
                    installed.bin_path
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

    /// Launch the app associated with a file extension, passing the file path as argv[1].
    pub fn launch_for_file(
        &self,
        file_path: &std::path::Path,
        cwd: &std::path::Path,
    ) -> Option<Box<dyn App>> {
        let ext = file_path.extension()?.to_string_lossy().to_lowercase();
        let id = self.extension_map.get(&ext)?.clone();
        let args = vec![file_path.display().to_string()];
        self.launch(&id, &cwd.to_path_buf(), &args)
    }

    /// Launch a process app by file extension, passing file path as argv[1].
    pub fn launch_process_for_file(
        &self,
        file_path: &std::path::Path,
        cwd: &std::path::Path,
    ) -> Option<(String, ProcessApp)> {
        let ext = file_path.extension()?.to_string_lossy().to_lowercase();
        let id = self.extension_map.get(&ext)?.clone();
        let args = vec![file_path.display().to_string()];
        self.launch_process(&id, &cwd.to_path_buf(), &args)
            .map(|app| (id, app))
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
            return Err(format!(
                "entry '{entry}' exists but is not executable (run: chmod +x {entry})"
            ));
        }
    }

    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_launch_join_group_parses() {
        // STEP-8: join_group lives under [launch] now, not [app.capabilities].
        let toml = r#"
[app]
id = "todo"
name = "Todo"
entry = "todo.py"

[app.capabilities]
capabilities = ["fs.read", "fs.write"]

[launch]
join_group = "cwd"
"#;
        let parsed: AppManifest = toml::from_str(toml).expect("parse");
        assert_eq!(parsed.launch.join_group.as_deref(), Some("cwd"));
    }

    #[test]
    fn manifest_launch_join_group_defaults_to_none() {
        let toml = r#"
[app]
id = "snake"
name = "Snake"
entry = "snake.py"

[app.capabilities]
capabilities = []
"#;
        let parsed: AppManifest = toml::from_str(toml).expect("parse");
        assert_eq!(parsed.launch.join_group, None);
    }

    #[test]
    fn manifest_layout_hint_parses_structured() {
        // STEP-8: layout_hint is structured { side, split } under [launch].
        let toml = r#"
[app]
id = "todo"
name = "Todo"
entry = "todo.py"

[app.capabilities]
capabilities = []

[launch]
layout_hint = { side = "right", split = 0.4 }
"#;
        let parsed: AppManifest = toml::from_str(toml).expect("parse");
        let hint = parsed.launch.layout_hint.expect("layout_hint present");
        assert_eq!(hint.side, "right");
        assert_eq!(hint.split, 0.4);
    }

    #[test]
    fn app_registry_rejects_unknown_capability_in_manifest() {
        // STEP-7: install-time validation replaces the old silent
        // `unknown string → FsRead` fallback. A typo must fail loudly.
        use std::io::Write;
        let tmp = std::env::temp_dir().join(format!(
            "plexi-reg-{}-bogus",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let manifest_path = tmp.join("manifest.toml");
        let mut f = std::fs::File::create(&manifest_path).unwrap();
        writeln!(
            f,
            r#"[app]
id = "typo"
name = "Typo"
entry = "typo.py"

[app.capabilities]
capabilities = ["net.http_"]
"#
        )
        .unwrap();
        // Touch entry so resolve_entry doesn't fail first.
        let _ = std::fs::File::create(tmp.join("typo.py")).unwrap();

        let registry = AppRegistry {
            apps: HashMap::new(),
            extension_map: HashMap::new(),
        };
        let err = registry.load_app(&tmp).expect_err("bogus capability must fail");
        assert!(
            err.contains("net.http_"),
            "error must name the offending capability: {err}"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn app_registry_accepts_all_nine_spec_capabilities() {
        use std::io::Write;
        let tmp = std::env::temp_dir().join(format!(
            "plexi-reg-{}-nine",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let manifest_path = tmp.join("manifest.toml");
        let mut f = std::fs::File::create(&manifest_path).unwrap();
        writeln!(
            f,
            r#"[app]
id = "greedy"
name = "Greedy"
entry = "g.py"

[app.capabilities]
capabilities = [
    "fs.read", "fs.write", "net.http", "secrets.get", "pipe.open",
    "spawn.app", "audio.record", "audio.playback", "video.playback",
]
"#
        )
        .unwrap();
        let entry_path = tmp.join("g.py");
        let _ = std::fs::File::create(&entry_path).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&entry_path).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&entry_path, perms).unwrap();
        }

        let registry = AppRegistry {
            apps: HashMap::new(),
            extension_map: HashMap::new(),
        };
        let installed = registry.load_app(&tmp).expect("nine valid caps parse");
        assert_eq!(installed.manifest.capabilities.capabilities.len(), 9);

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
