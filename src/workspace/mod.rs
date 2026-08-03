pub mod router;
pub mod secrets;

use crate::host::context::Context;
use crate::spatial::tiling::PaneId;
use egui_tiles::{TileId, Tree};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

static TEMP_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Serialize, Deserialize)]
pub struct WorkspaceFile {
    pub version: u32,
    /// Active context index (sidebar item).
    pub active_context: usize,
    pub sidebar_visible: bool,
    pub next_pane_id: u64,
    pub contexts: Vec<Context>,
    pub windows: Vec<SavedWindow>,
    /// context_id → last active window_id for that context.
    #[serde(default)]
    pub context_active_window: HashMap<u64, u64>,
}

#[derive(Serialize, Deserialize)]
pub struct SavedWindow {
    pub name: String,
    pub path: PathBuf,
    pub tree: Tree<PaneId>,
    pub panes: Vec<SavedPane>,
    pub focused_pane: Option<TileId>,
    #[serde(default)]
    pub grid_x: u32,
    #[serde(default)]
    pub grid_y: u32,
    #[serde(default)]
    pub window_id: u64,
    #[serde(default)]
    pub context_id: u64,
}

#[derive(Serialize, Deserialize)]
pub struct SavedPane {
    pub id: u64,
    #[serde(default)]
    pub kind: SavedPaneKind,
    pub cwd: PathBuf,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub app_id: Option<String>,
    #[serde(default)]
    pub app_state: Option<serde_json::Value>,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub heartbeat: Option<SavedPaneHeartbeat>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct SavedPaneHeartbeat {
    pub every_ms: u64,
    pub text: String,
    pub while_idle_only: bool,
}

#[derive(Serialize, Deserialize, Clone, Default, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum SavedPaneKind {
    #[default]
    Terminal,
    App,
    #[serde(alias = "sub_context")]
    Portal {
        context_id: u64,
    },
}

fn workspace_path() -> PathBuf {
    crate::config::config_dir()
        .join("workspaces")
        .join("default.json")
}

/// Env switch for a hermetic host session, set by `plexi host start
/// --ephemeral`. When present, [`WorkspaceFile::load`] restores nothing and
/// [`WorkspaceFile::save`] persists nothing, so automated runs (scene
/// runners, release gates) neither inherit nor clobber the channel's saved
/// session. This is the sanctioned isolation affordance — gate any future
/// automated-host workflow on the same variable instead of stashing
/// `workspaces/default.json` by hand.
pub const EPHEMERAL_SESSION_ENV: &str = "PLEXI_EPHEMERAL_SESSION";

fn ephemeral_session() -> bool {
    ephemeral_session_from(std::env::var_os(EPHEMERAL_SESSION_ENV).as_deref())
}

/// Pure predicate behind [`ephemeral_session`], testable without mutating
/// process env: set (non-empty) means hermetic.
fn ephemeral_session_from(value: Option<&std::ffi::OsStr>) -> bool {
    value.is_some_and(|value| !value.is_empty())
}

/// Boot-mode switch set only by an explicit `plexi host start --background`.
/// It is process-local launch state, not persisted configuration.
pub const BACKGROUND_SESSION_ENV: &str = "PLEXI_BACKGROUND_HOST";

/// Owned process boot state captured before launch-only environment markers
/// are removed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostBootState {
    background: bool,
}

impl HostBootState {
    pub fn is_background(self) -> bool {
        self.background
    }
}

static HOST_BOOT_STATE: std::sync::OnceLock<HostBootState> = std::sync::OnceLock::new();

/// Snapshot and consume launch-only host state exactly once.
///
/// Production must call this as the first action in `main`, before any thread
/// can read or inherit the process environment. Removing an environment
/// variable while another thread accesses the environment is not safe.
pub fn consume_host_boot_state() -> HostBootState {
    *HOST_BOOT_STATE.get_or_init(|| {
        let background =
            background_session_from(std::env::var_os(BACKGROUND_SESSION_ENV).as_deref());
        // SAFETY: `main` calls this before config initialization, logging,
        // shell probes, the heartbeat, or any other thread creation.
        unsafe {
            std::env::remove_var(BACKGROUND_SESSION_ENV);
        }
        HostBootState { background }
    })
}

fn background_session_from(value: Option<&std::ffi::OsStr>) -> bool {
    value.is_some_and(|value| !value.is_empty())
}

impl WorkspaceFile {
    pub fn save(&self) -> io::Result<()> {
        if ephemeral_session() {
            log::info!("workspace_save: skipped (ephemeral session)");
            return Ok(());
        }
        let path = workspace_path();
        let temp_path = unique_temp_path(&path);
        let started = Instant::now();
        let result = (|| {
            let parent = path.parent().ok_or_else(|| {
                save_error(
                    "resolve workspace directory",
                    &temp_path,
                    io::Error::new(io::ErrorKind::InvalidInput, "workspace path has no parent"),
                )
            })?;
            std::fs::create_dir_all(parent)
                .map_err(|error| save_error("create workspace directory", &temp_path, error))?;
            let json = serde_json::to_string_pretty(self).map_err(|error| {
                save_error("serialize workspace", &temp_path, io::Error::other(error))
            })?;
            atomic_save(&path, &temp_path, json.as_bytes(), |file, bytes| {
                file.write_all(bytes)
            })?;
            Ok(json.len())
        })();
        match result {
            Ok(byte_count) => {
                let pane_count: usize = self.windows.iter().map(|window| window.panes.len()).sum();
                log::info!(
                    "workspace_save: saved bytes={} windows={} panes={} elapsed_ms={}",
                    byte_count,
                    self.windows.len(),
                    pane_count,
                    started.elapsed().as_millis()
                );
                Ok(())
            }
            Err(error) => {
                log::error!("workspace_save: {error}");
                Err(error)
            }
        }
    }

    pub fn load() -> Option<Self> {
        if ephemeral_session() {
            log::info!("workspace_load: skipped (ephemeral session)");
            return None;
        }
        let path = workspace_path();
        let data = match std::fs::read_to_string(&path) {
            Ok(d) => d,
            Err(_) => return None,
        };
        let ws: Self = match serde_json::from_str(&data) {
            Ok(ws) => ws,
            Err(e) => {
                log::warn!("Failed to parse workspace file: {e}");
                let backup = path.with_extension(format!(
                    "backup-{}.json",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0)
                ));
                let _ = std::fs::rename(&path, &backup);
                return None;
            }
        };
        if ws.version != 2 {
            log::info!(
                "Ignoring old workspace file version {}; starting fresh",
                ws.version
            );
            let backup = path.with_extension(format!(
                "backup-v{}-{}.json",
                ws.version,
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0)
            ));
            let _ = std::fs::rename(&path, &backup);
            return None;
        }
        Some(ws)
    }
}

fn atomic_save<F>(path: &Path, temp_path: &Path, bytes: &[u8], write_temp: F) -> io::Result<()>
where
    F: FnOnce(&mut File, &[u8]) -> io::Result<()>,
{
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("workspace path has no parent: {}", path.display()),
        )
    })?;
    let result = (|| {
        let mut temp = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(temp_path)
            .map_err(|error| save_error("create temp file", temp_path, error))?;
        write_temp(&mut temp, bytes)
            .map_err(|error| save_error("write temp file", temp_path, error))?;
        temp.sync_all()
            .map_err(|error| save_error("sync temp file", temp_path, error))?;

        if path.exists() {
            let previous_path = path.with_extension("json.prev");
            let previous_temp_path = unique_temp_path(&previous_path);
            let backup_result = (|| {
                let mut source = File::open(path)
                    .map_err(|error| save_error("open previous workspace", temp_path, error))?;
                let mut previous_temp = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&previous_temp_path)
                    .map_err(|error| save_error("create previous temp file", temp_path, error))?;
                io::copy(&mut source, &mut previous_temp)
                    .map_err(|error| save_error("copy previous workspace", temp_path, error))?;
                previous_temp
                    .sync_all()
                    .map_err(|error| save_error("sync previous workspace", temp_path, error))?;
                std::fs::rename(&previous_temp_path, &previous_path)
                    .map_err(|error| save_error("replace previous workspace", temp_path, error))
            })();
            if backup_result.is_err() {
                let _ = std::fs::remove_file(&previous_temp_path);
            }
            backup_result?;
        }

        std::fs::rename(temp_path, path)
            .map_err(|error| save_error("replace workspace", temp_path, error))?;
        File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| save_error("sync workspace directory", temp_path, error))
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(temp_path);
    }
    result
}

fn unique_temp_path(path: &Path) -> PathBuf {
    let sequence = TEMP_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("workspace");
    path.with_file_name(format!("{file_name}.tmp-{}-{sequence}", std::process::id()))
}

fn save_error(operation: &str, temp_path: &Path, error: io::Error) -> io::Error {
    io::Error::new(
        error.kind(),
        format!(
            "{operation} failed (temp_path={}): {error}",
            temp_path.display()
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_atomic_save<F>(path: &Path, bytes: &[u8], write_temp: F) -> io::Result<()>
    where
        F: FnOnce(&mut File, &[u8]) -> io::Result<()>,
    {
        atomic_save(path, &unique_temp_path(path), bytes, write_temp)
    }

    #[test]
    fn ephemeral_session_predicate_requires_nonempty_value() {
        use std::ffi::OsStr;
        assert!(!super::ephemeral_session_from(None));
        assert!(!super::ephemeral_session_from(Some(OsStr::new(""))));
        assert!(super::ephemeral_session_from(Some(OsStr::new("1"))));
    }

    #[test]
    fn background_session_predicate_requires_nonempty_value() {
        use std::ffi::OsStr;
        assert!(!super::background_session_from(None));
        assert!(!super::background_session_from(Some(OsStr::new(""))));
        assert!(super::background_session_from(Some(OsStr::new("1"))));
    }

    #[test]
    fn background_launch_marker_does_not_reach_descendants() {
        const PROBE_ENV: &str = "PLEXI_TEST_BACKGROUND_MARKER_PROBE";

        if std::env::var_os(PROBE_ENV).is_some() {
            let boot_state = super::consume_host_boot_state();
            assert!(boot_state.is_background());
            let output = std::process::Command::new("/usr/bin/printenv")
                .arg(super::BACKGROUND_SESSION_ENV)
                .output()
                .expect("run inherited-env probe");
            assert!(
                !output.status.success() && output.stdout.is_empty(),
                "background marker leaked to child: status={:?} stdout={:?}",
                output.status,
                String::from_utf8_lossy(&output.stdout)
            );
            return;
        }

        let output = std::process::Command::new(
            std::env::current_exe().expect("resolve current test binary"),
        )
        .args([
            "--exact",
            "workspace::tests::background_launch_marker_does_not_reach_descendants",
            "--test-threads=1",
        ])
        .env(PROBE_ENV, "1")
        .env(super::BACKGROUND_SESSION_ENV, "1")
        .output()
        .expect("run isolated background marker probe");

        assert!(
            output.status.success(),
            "isolated marker probe failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn atomic_save_replaces_workspace_without_temp_residue() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("default.json");

        test_atomic_save(&path, b"new workspace", |file, bytes| file.write_all(bytes))
            .expect("atomic save");

        assert_eq!(
            std::fs::read(&path).expect("read workspace"),
            b"new workspace"
        );
        let entries: Vec<_> = std::fs::read_dir(directory.path())
            .expect("read workspace directory")
            .map(|entry| entry.expect("directory entry").file_name())
            .collect();
        assert_eq!(entries, [std::ffi::OsString::from("default.json")]);
    }

    #[test]
    fn atomic_save_rotates_one_previous_generation() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("default.json");
        let previous_path = directory.path().join("default.json.prev");

        test_atomic_save(&path, b"first", |file, bytes| file.write_all(bytes)).expect("first save");
        assert!(
            !previous_path.exists(),
            "first save has no prior generation"
        );

        test_atomic_save(&path, b"second", |file, bytes| file.write_all(bytes))
            .expect("second save");
        assert_eq!(
            std::fs::read(&previous_path).expect("read first backup"),
            b"first"
        );

        test_atomic_save(&path, b"third", |file, bytes| file.write_all(bytes)).expect("third save");
        assert_eq!(std::fs::read(&path).expect("read current"), b"third");
        assert_eq!(
            std::fs::read(&previous_path).expect("read rolling backup"),
            b"second"
        );
    }

    #[test]
    fn failed_partial_temp_write_preserves_loadable_workspace() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("default.json");
        let prior = br#"{"version":2,"active_context":0,"sidebar_visible":true,"next_pane_id":1,"contexts":[],"windows":[],"context_active_window":{}}"#;
        std::fs::write(&path, prior).expect("seed prior workspace");

        let error = test_atomic_save(&path, b"replacement", |file, bytes| {
            file.write_all(&bytes[..4])?;
            Err(io::Error::new(
                io::ErrorKind::Other,
                "injected write failure",
            ))
        })
        .expect_err("partial write must fail");

        assert!(error.to_string().contains("write temp file failed"));
        assert!(error.to_string().contains("temp_path="));
        assert_eq!(std::fs::read(&path).expect("read prior workspace"), prior);
        let loaded: WorkspaceFile =
            serde_json::from_slice(&std::fs::read(&path).expect("read JSON"))
                .expect("prior workspace remains loadable");
        assert_eq!(loaded.version, 2);
        assert!(
            std::fs::read_dir(directory.path())
                .expect("read workspace directory")
                .all(|entry| !entry
                    .expect("directory entry")
                    .file_name()
                    .to_string_lossy()
                    .contains(".tmp-"))
        );
    }

    /// `SavedPaneKind` must serialise every Pane variant produced by mirror-split
    /// (Cmd+N / Cmd+Shift+N): Terminal, App. Round-tripping through JSON
    /// preserves the kind so a workspace saved after a split restores the same
    /// pane type on next launch.
    #[test]
    fn workspace_save_restore_preserves_split_panes() {
        let kinds = [SavedPaneKind::Terminal, SavedPaneKind::App];
        for kind in kinds {
            let pane = SavedPane {
                id: 42,
                kind: kind.clone(),
                cwd: PathBuf::from("/tmp"),
                name: Some("split-pane".to_string()),
                app_id: matches!(kind, SavedPaneKind::App).then(|| "snake".to_string()),
                app_state: None,
                hidden: false,
                heartbeat: None,
            };
            let json = serde_json::to_string(&pane).expect("serialize");
            let restored: SavedPane = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(restored.kind, kind, "kind {kind:?} must round-trip");
            assert_eq!(restored.id, 42);
            assert_eq!(restored.cwd, PathBuf::from("/tmp"));
        }
        // Portal round-trips with embedded context_id.
        let portal_pane = SavedPane {
            id: 99,
            kind: SavedPaneKind::Portal { context_id: 42 },
            cwd: PathBuf::new(),
            name: None,
            app_id: None,
            app_state: None,
            hidden: false,
            heartbeat: None,
        };
        let json = serde_json::to_string(&portal_pane).expect("serialize portal");
        let restored: SavedPane = serde_json::from_str(&json).expect("deserialize portal");
        assert!(
            matches!(restored.kind, SavedPaneKind::Portal { context_id: 42 }),
            "Portal kind must round-trip with correct context_id"
        );
        assert_eq!(restored.id, 99);

        // Backward compat: old "sub_context" JSON still deserializes to Portal.
        let legacy_json = r#"{"id":99,"kind":{"sub_context":{"context_id":42}},"cwd":"","name":null,"app_id":null,"app_state":null}"#;
        let legacy: SavedPane =
            serde_json::from_str(legacy_json).expect("deserialize legacy sub_context");
        assert!(
            matches!(legacy.kind, SavedPaneKind::Portal { context_id: 42 }),
            "legacy sub_context must deserialize to Portal"
        );
    }

    /// SavedWindow omits `grid_x` / `grid_y` defaults cleanly.
    #[test]
    fn grid_coords_default_to_zero_on_load() {
        use egui_tiles::Tree;

        let tree: Tree<PaneId> = Tree::empty("test");
        let tree_json = serde_json::to_string(&tree).expect("tree must serialize");

        let json = format!(
            r#"{{
                "name": "Default",
                "path": "/tmp",
                "tree": {},
                "panes": [],
                "focused_pane": null
            }}"#,
            tree_json
        );

        let restored: SavedWindow = serde_json::from_str(&json)
            .expect("SavedWindow without grid_x/grid_y must deserialize");
        assert_eq!(restored.grid_x, 0, "grid_x must default to 0");
        assert_eq!(restored.grid_y, 0, "grid_y must default to 0");
    }

    /// Legacy workspace files without optional fields still deserialize cleanly.
    #[test]
    fn legacy_saved_pane_deserializes_cleanly() {
        let legacy_json = r#"{
            "id": 1,
            "kind": "terminal",
            "cwd": "/tmp"
        }"#;
        let restored: SavedPane =
            serde_json::from_str(legacy_json).expect("legacy SavedPane must deserialize");
        assert_eq!(restored.kind, SavedPaneKind::Terminal);
    }

    /// Context (formerly SavedContext) round-trips through JSON and handles
    /// legacy files missing optional fields (root, description, parent_id, depth).
    #[test]
    fn context_serde_round_trip() {
        let ctx = Context {
            name: "dev".to_string().into(),
            root: PathBuf::from("/projects/dev/src"),
            description: Some("main workspace".to_string()),
            context_id: 42,
            parent_id: Some(1),
            depth: 1,
            parked: false,
        };
        let json = serde_json::to_string(&ctx).expect("serialize");
        let restored: Context = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored.name, "dev");
        assert_eq!(restored.context_id, 42);
        assert_eq!(restored.parent_id, Some(1));
        assert_eq!(restored.depth, 1);
        assert_eq!(restored.root, PathBuf::from("/projects/dev/src"));
    }

    /// Legacy workspace JSON written before the path→root collapse: a
    /// required `path` and no `root`. The anchor folds to `path`.
    #[test]
    fn legacy_context_with_only_path_deserializes() {
        let legacy_json = r#"{
            "name": "old-ctx",
            "path": "/tmp",
            "context_id": 7
        }"#;
        let restored: Context =
            serde_json::from_str(legacy_json).expect("legacy Context must deserialize");
        assert_eq!(restored.name, "old-ctx");
        assert_eq!(restored.context_id, 7);
        assert_eq!(restored.parent_id, None);
        assert_eq!(restored.depth, 0);
        assert_eq!(restored.root, PathBuf::from("/tmp"));
        assert_eq!(restored.description, None);
    }

    /// Legacy workspace JSON carrying both `path` and an explicit `root`
    /// keeps the root — it was the field `set-root` wrote and the only one
    /// that affected behavior.
    #[test]
    fn legacy_context_with_path_and_root_prefers_root() {
        let legacy_json = r#"{
            "name": "old-ctx",
            "path": "/tmp",
            "root": "/projects/dev",
            "context_id": 7
        }"#;
        let restored: Context =
            serde_json::from_str(legacy_json).expect("legacy Context must deserialize");
        assert_eq!(restored.root, PathBuf::from("/projects/dev"));
    }

    /// A context with neither `root` nor legacy `path` is corrupt and must
    /// fail loudly (naming the context), never half-load a layout.
    #[test]
    fn context_without_root_or_path_fails_loudly() {
        let corrupt_json = r#"{
            "name": "broken-ctx",
            "context_id": 9
        }"#;
        let error = serde_json::from_str::<Context>(corrupt_json)
            .expect_err("context without root or path must not deserialize");
        assert!(error.to_string().contains("broken-ctx"), "{error}");
    }
}
