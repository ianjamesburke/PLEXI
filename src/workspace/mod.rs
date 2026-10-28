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
    #[serde(default)]
    pub strips: HashMap<u64, crate::strip::Strip>,
    #[serde(default)]
    pub conjoined_contexts: HashMap<u64, PaneId>,
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

#[derive(Serialize, Deserialize, Clone)]
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
    /// `AppRuntime::type_id()` at save time (`"python-wasm"` / `"wasm"` /
    /// a builtin's own type id). Distinct from `app_id`, which is the app's
    /// *manifest* identity — the runtime kind alone cannot relaunch anything.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_kind: Option<String>,
    /// Enough launch context to relaunch a Python/WASM app pane through the
    /// normal open path on restore. `None` for builtins (restored via
    /// `restore_builtin_app_pane`) and for saves written before this field
    /// existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch: Option<SavedAppLaunch>,
    /// Named file-backed pane slots (`slot_name -> path`) persisted across restarts.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub slots: HashMap<String, PathBuf>,
    /// Saved agent session state for agent terminal panes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_session: Option<SavedAgentSession>,
}

/// Queued structured result from a child agent pane waiting to be delivered
/// to an interactive parent pane once the parent transitions to Idle.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct QueuedChildResult {
    pub child_pane_id: u64,
    pub child_agent: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub result_json: String,
    pub finished_at_epoch_secs: u64,
}

/// Persisted agent session state for an agent terminal pane.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct SavedAgentSession {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fleet_result: Option<serde_json::Value>,
    #[serde(default)]
    pub fleet_seen: bool,
    #[serde(default)]
    pub fleet_dismissed: bool,
    #[serde(default)]
    pub fleet_done_unseen: bool,
    #[serde(default)]
    pub fleet_since: u64,
    pub harness: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_posture: Option<crate::broker::PermissionPosture>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub last_state: crate::protocol::AgentState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_blocked_reason: Option<crate::protocol::AgentBlockedReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_pane_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawned_by: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_child_results: Vec<QueuedChildResult>,
    #[serde(default)]
    pub ephemeral: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tail_preview: Option<String>,
    /// Saved terminal scrollback shown while a lazily restored pane is in Standby.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub standby_scrollback_tail: Option<Vec<String>>,
}

/// Closed-run history ledger record written to `~/.plexi-<channel>/agent_history.jsonl`
/// whenever an agent pane or ephemeral squad worker closes.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct AgentHistoryRecord {
    #[serde(default)]
    pub fleet_seen: bool,
    #[serde(default)]
    pub fleet_dismissed: bool,
    pub run_id: String,
    pub pane_id: u64,
    pub context_id: u64,
    pub context_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_pane_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawned_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane_name: Option<String>,
    pub harness: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_posture: Option<crate::broker::PermissionPosture>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub cwd: PathBuf,
    #[serde(default)]
    pub ephemeral: bool,
    pub last_state: crate::protocol::AgentState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_blocked_reason: Option<crate::protocol::AgentBlockedReason>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tail_preview: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub slots: HashMap<String, PathBuf>,
    pub closed_at: u64,
}

impl SavedPane {
    /// True for a pre-0680 save of a Python/WASM app pane: `app_id` holds the
    /// *runtime kind* (`AppRuntime::type_id()`), not the app's manifest id,
    /// and `runtime_kind`/`launch` are absent because the field didn't exist
    /// yet. There is no recoverable manifest identity in this record, so
    /// restore renders a launch-failed pane naming the runtime kind rather
    /// than the (unknown) app id — it must never silently substitute a
    /// terminal.
    pub fn is_legacy_runtime_kind_id(&self) -> bool {
        self.runtime_kind.is_none()
            && matches!(self.app_id.as_deref(), Some("python-wasm" | "wasm"))
    }
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

/// Launch context saved alongside a Python/WASM `SavedPane`, sufficient to
/// relaunch the same app through the normal open path on restore. Never
/// carries granted capabilities — those live in `PermissionStore` and are
/// re-derived at restore time, not persisted into the workspace file.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(tag = "runtime", rename_all = "snake_case")]
pub enum SavedAppLaunch {
    Python {
        app_dir: PathBuf,
        #[serde(default)]
        args: Vec<String>,
    },
    Wasm {
        wasm_path: PathBuf,
        #[serde(default)]
        args: Vec<String>,
    },
}

pub fn workspace_path_in(profile_dir: &Path) -> PathBuf {
    profile_dir.join("workspaces").join("default.json")
}

pub fn agent_history_path_in(profile_dir: &Path) -> PathBuf {
    profile_dir.join("agent_history.jsonl")
}

pub fn append_agent_history(record: &AgentHistoryRecord) -> io::Result<()> {
    append_agent_history_to(&crate::config::config_dir(), record)
}

pub fn append_agent_history_to(profile_dir: &Path, record: &AgentHistoryRecord) -> io::Result<()> {
    let path = agent_history_path_in(profile_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut line = serde_json::to_string(record).map_err(io::Error::other)?;
    line.push('\n');
    let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
    file.write_all(line.as_bytes())?;
    log::info!(
        "agent_history: recorded closed run run_id={} pane_id={} harness={} session_id={:?} state={:?}",
        record.run_id,
        record.pane_id,
        record.harness,
        record.session_id,
        record.last_state
    );
    Ok(())
}

pub fn load_agent_history() -> Vec<AgentHistoryRecord> {
    load_agent_history_from(&crate::config::config_dir())
}

pub fn load_agent_history_from(profile_dir: &Path) -> Vec<AgentHistoryRecord> {
    let path = agent_history_path_in(profile_dir);
    let data = match std::fs::read_to_string(&path) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };
    let records = data.lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| match serde_json::from_str::<AgentHistoryRecord>(line) {
            Ok(rec) => Some(rec),
            Err(e) => {
                log::warn!("agent_history: skipping malformed line in {}: {e}", path.display());
                None
            }
        })
        .map(|r| (r.run_id.clone(), r))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut records: Vec<_> = records.into_values().collect();
    records.sort_by_key(|r| r.closed_at);
    records
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
        self.save_to(&crate::config::config_dir())
    }

    pub fn save_to(&self, profile_dir: &Path) -> io::Result<()> {
        self.save_to_with_migration_source(profile_dir, None)
    }

    fn save_to_with_migration_source(
        &self,
        profile_dir: &Path,
        migration_source: Option<&[u8]>,
    ) -> io::Result<()> {
        self.validate()?;
        let path = workspace_path_in(profile_dir);
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
            match std::fs::read(&path) {
                Ok(data) => {
                    if migration_source.is_some_and(|source| source != data) {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "workspace changed while migrating; preserving original",
                        ));
                    }
                    let mut existing: Self = serde_json::from_slice(&data).map_err(|error| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("refusing to overwrite invalid workspace {}: {error}", path.display()),
                        )
                    })?;
                    if !matches!(existing.version, 2 | 3) {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("refusing to overwrite workspace {} with unsupported version {}", path.display(), existing.version),
                        ));
                    }
                    if migration_source.is_some() && existing.version == 2 && self.version == 3 {
                        existing.remove_legacy_deleted_context_focus();
                    }
                    existing.validate()?;
                    if existing.version == 2 && self.version == 3 {
                        preserve_v2_rollback(&path, &data)?;
                    }
                    if existing.version == 3 && self.version == 2 {
                        return Err(io::Error::new(io::ErrorKind::InvalidData, "refusing to downgrade a version 3 workspace"));
                    }
                }
                Err(error)
                    if error.kind() == io::ErrorKind::NotFound && migration_source.is_none() => {}
                Err(error) => return Err(error),
            }
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
        Self::load_from(&crate::config::config_dir())
    }

    pub fn load_from(profile_dir: &Path) -> Option<Self> {
        let path = workspace_path_in(profile_dir);
        let data = match std::fs::read_to_string(&path) {
            Ok(data) => data,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return None,
            Err(error) => {
                log::error!("workspace_load: cannot read {}: {error}", path.display());
                return None;
            }
        };
        let mut ws: Self = match serde_json::from_str(&data) {
            Ok(ws) => ws,
            Err(e) => {
                log::warn!("Failed to parse workspace file: {e}");
                return None;
            }
        };
        if !matches!(ws.version, 2 | 3) {
            log::info!(
                "Unsupported workspace file version {}; preserving original",
                ws.version
            );
            return None;
        }
        if ws.version == 2 {
            ws.remove_legacy_deleted_context_focus();
        }
        if let Err(error) = ws.validate() {
            log::error!("workspace_load: invalid workspace {}: {error}; preserving original", path.display());
            return None;
        }
        if ws.version == 2 {
            match ws.assign_legacy_window_ids().and_then(|()| ws.convert_legacy_strips()).and_then(|strips| {
                ws.strips = strips;
                ws.version = 3;
                ws.save_to_with_migration_source(profile_dir, Some(data.as_bytes()))
            }) {
                Ok(()) => log::info!("workspace migration: converted version 2 to version 3; legacy conjoined views were not recorded in version 2"),
                Err(error) => {
                    log::error!("workspace migration: {}: {error}; preserving original", path.display());
                    return None;
                }
            }
        }
        Some(ws)
    }

    /// Version 2 could persist focus entries after a context was deleted.
    /// Remove only entries whose context and window are both absent; retain
    /// all other invalid references for validation to reject.
    fn remove_legacy_deleted_context_focus(&mut self) {
        let contexts: std::collections::HashSet<_> = self
            .contexts
            .iter()
            .map(|context| context.context_id)
            .collect();
        let windows: std::collections::HashSet<_> = self
            .windows
            .iter()
            .map(|window| window.window_id)
            .collect();
        self.context_active_window.retain(|context, window| {
            let orphaned = !contexts.contains(context) && !windows.contains(window);
            if orphaned {
                log::info!(
                    "workspace migration: removed stale focus entry context={context} window={window}"
                );
            }
            !orphaned
        });
    }

    fn assign_legacy_window_ids(&mut self) -> io::Result<()> {
        let mut used = std::collections::HashSet::new();
        for window in &self.windows {
            if window.window_id != 0 && !used.insert(window.window_id) {
                return Err(io::Error::new(io::ErrorKind::InvalidData, format!("duplicate window identity {}", window.window_id)));
            }
        }
        let mut next = 1_u64;
        let mut assigned: HashMap<u64, Vec<u64>> = HashMap::new();
        for window in &mut self.windows {
            if window.window_id != 0 { continue; }
            while used.contains(&next) {
                next = next.checked_add(1).ok_or_else(|| io::Error::other("window identity space exhausted"))?;
            }
            window.window_id = next;
            used.insert(next);
            assigned.entry(window.context_id).or_default().push(next);
        }
        for (context, window_id) in &mut self.context_active_window {
            if *window_id != 0 { continue; }
            match assigned.get(context).map(Vec::as_slice) {
                Some([id]) => *window_id = *id,
                _ => return Err(io::Error::new(io::ErrorKind::InvalidData, format!("context {context} has an ambiguous legacy active window"))),
            }
        }
        Ok(())
    }

    fn convert_legacy_strips(&self) -> io::Result<HashMap<u64, crate::strip::Strip>> {
        use crate::app::zoom::ColumnWidthPreset;
        use crate::strip::{Column, ColumnWidth, Strip};
        let mut strips = HashMap::new();
        for context in &self.contexts {
            let columns = saved_legacy_columns(&self.windows, context.context_id)?;
            let count = columns.len();
            let mut strip = Strip::new(context.context_id);
            for (index, (panes, saved_width, heights)) in columns.into_iter().enumerate() {
                let width = saved_width.unwrap_or(ColumnWidth::Preset(if count > 1 && index == 0 { ColumnWidthPreset::Parent35 } else { ColumnWidthPreset::Child65 }));
                let id = strip.alloc_column_id();
                let mut column = Column::new_stacked(id, panes, width);
                column.heights = heights;
                strip.push_column(column);
            }
            let focused = self.context_active_window.get(&context.context_id)
                .and_then(|id| self.windows.iter().find(|window| window.window_id == *id && window.context_id == context.context_id))
                .or_else(|| self.windows.iter().find(|window| window.context_id == context.context_id))
                .and_then(|window| window.focused_pane.and_then(|tile| match window.tree.tiles.get(tile) {
                    Some(egui_tiles::Tile::Pane(pane)) => Some(*pane), _ => None,
                }));
            if let Some(focused) = focused { strip.focus_pane(focused); }
            strips.insert(context.context_id, strip);
        }
        Ok(strips)
    }

    fn validate(&self) -> io::Result<()> {
        let invalid = |message: String| io::Error::new(io::ErrorKind::InvalidData, message);
        let context_ids: std::collections::HashSet<_> = self.contexts.iter().map(|c| c.context_id).collect();
        if context_ids.len() != self.contexts.len() {
            return Err(invalid("duplicate context identity".into()));
        }
        if !self.contexts.is_empty() && self.active_context >= self.contexts.len() {
            return Err(invalid(format!("invalid active context index {}", self.active_context)));
        }
        let parents: HashMap<_, _> = self.contexts.iter().map(|context| (context.context_id, context.parent_id)).collect();
        for context in &self.contexts {
            let mut seen = std::collections::HashSet::new();
            let mut next = Some(context.context_id);
            while let Some(id) = next {
                if !seen.insert(id) { return Err(invalid(format!("context hierarchy contains a cycle at {id}"))); }
                next = *parents.get(&id).ok_or_else(|| invalid(format!("context hierarchy references missing context {id}")))?;
            }
        }
        let mut owners = HashMap::new();
        let mut window_ids = std::collections::HashSet::new();
        for window in &self.windows {
            if self.version == 3 && (window.window_id == 0 || !window_ids.insert(window.window_id)) {
                return Err(invalid(format!("invalid or duplicate window identity {}", window.window_id)));
            }
            if !context_ids.contains(&window.context_id) {
                return Err(invalid(format!("window {} references missing context {}", window.window_id, window.context_id)));
            }
            for pane in &window.panes {
                if matches!(pane.kind, SavedPaneKind::App) && pane.app_id.is_none() {
                    return Err(invalid(format!("app pane {} has no app identity", pane.id)));
                }
                if let SavedPaneKind::Portal { context_id } = pane.kind {
                    if !context_ids.contains(&context_id) { return Err(invalid(format!("portal {} references missing context {context_id}", pane.id))); }
                }
                if owners.insert(pane.id, window.context_id).is_some() {
                    return Err(invalid(format!("duplicate pane identity {}", pane.id)));
                }
            }
        }
        for (context, window_id) in &self.context_active_window {
            let matches = self.windows.iter().filter(|window| window.context_id == *context && window.window_id == *window_id).count();
            if !context_ids.contains(context) || matches != 1 {
                return Err(invalid(format!("context {context} references invalid active window {window_id}")));
            }
        }
        for window in &self.windows {
            if let Some(focused) = window.focused_pane {
                if !matches!(window.tree.tiles.get(focused), Some(egui_tiles::Tile::Pane(_))) {
                    return Err(invalid(format!("window {} references invalid focused tile", window.window_id)));
                }
            }
        }
        if owners.keys().any(|id| *id >= self.next_pane_id) {
            return Err(invalid(format!("next pane identity {} is not greater than every saved pane", self.next_pane_id)));
        }
        if self.version == 3 && self.strips.len() != context_ids.len() {
            return Err(invalid("version 3 must have one strip per context".into()));
        }
        for (child, parent_pane) in &self.conjoined_contexts {
            let parent_context = self.contexts.iter().find(|context| context.context_id == *child).and_then(|context| context.parent_id);
            if parent_context.is_none() || owners.get(parent_pane).copied() != parent_context {
                return Err(invalid(format!("invalid conjoined parent {parent_pane} for context {child}")));
            }
        }
        for window in &self.windows {
            if let Some(root) = window.tree.root() {
                validate_saved_tree(window, root, &owners, &mut std::collections::HashSet::new())?;
            }
        }
        if self.version != 3 {
            return Ok(());
        }
        let mut membership = std::collections::HashSet::new();
        for (context, strip) in &self.strips {
            if !context_ids.contains(context) || strip.context_id != *context {
                return Err(invalid(format!("strip references invalid context {context}")));
            }
            if !strip.scroll_offset_px.is_finite() || strip.scroll_offset_px < 0.0 {
                return Err(invalid(format!("strip {context} has invalid scroll offset")));
            }
            let mut column_ids = std::collections::HashSet::new();
            for column in &strip.columns {
                if !column_ids.insert(column.id) || column.panes.is_empty() || column.panes.len() != column.heights.len() {
                    return Err(invalid(format!("strip {context} has invalid column {:?}", column.id)));
                }
                let valid_width = match column.width {
                    crate::strip::ColumnWidth::Preset(_) => true,
                    crate::strip::ColumnWidth::CustomRatio(value) => value.is_finite() && (0.05..=0.95).contains(&value),
                    crate::strip::ColumnWidth::CustomPixels(value) | crate::strip::ColumnWidth::FixedPx(value) => value.is_finite() && value > 0.0,
                };
                if !valid_width { return Err(invalid(format!("strip {context} has invalid column width"))); }
                if column.heights.iter().any(|h| !h.is_finite() || *h <= 0.0) {
                    return Err(invalid(format!("strip {context} has invalid stack shares")));
                }
                if column.last_focused.is_some_and(|pane| !column.panes.contains(&pane)) {
                    return Err(invalid(format!("strip {context} has invalid column focus")));
                }
                for pane in &column.panes {
                    if owners.get(pane) != Some(context) || !membership.insert(*pane) {
                        return Err(invalid(format!("strip {context} has invalid or duplicate pane {pane}")));
                    }
                }
            }
            if strip.focused_pane.is_some_and(|pane| !strip.contains_pane(pane)) {
                return Err(invalid(format!("strip {context} has invalid focus")));
            }
            let expected = saved_strip_columns(&self.windows, *context)?;
            let actual: Vec<_> = strip.columns.iter().map(|column| column.panes.clone()).collect();
            if actual != expected {
                return Err(invalid(format!("strip {context} does not match saved window tree membership")));
            }
        }
        Ok(())
    }
}

fn validate_saved_tree(
    window: &SavedWindow,
    tile_id: TileId,
    owners: &HashMap<PaneId, u64>,
    visited: &mut std::collections::HashSet<TileId>,
) -> io::Result<()> {
    if !visited.insert(tile_id) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("window {} contains a repeated tile", window.window_id)));
    }
    let tile = window.tree.tiles.get(tile_id).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!("window {} references a missing tile", window.window_id)))?;
    match tile {
        egui_tiles::Tile::Pane(id) if !owners.contains_key(id) => {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("window {} references missing pane {id}", window.window_id)));
        }
        egui_tiles::Tile::Pane(_) => {}
        egui_tiles::Tile::Container(egui_tiles::Container::Grid(_)) => {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("window {} uses an unsupported grid layout", window.window_id)));
        }
        egui_tiles::Tile::Container(egui_tiles::Container::Linear(linear)) if linear.dir == egui_tiles::LinearDir::Horizontal && window.tree.root() != Some(tile_id) => {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("window {} uses an unsupported nested horizontal split", window.window_id)));
        }
        egui_tiles::Tile::Container(container) => {
            for &child in container.children() {
                validate_saved_tree(window, child, owners, visited)?;
            }
        }
    }
    Ok(())
}

type LegacyColumn = (Vec<PaneId>, Option<crate::strip::ColumnWidth>, Vec<f32>);

fn saved_legacy_columns(windows: &[SavedWindow], context_id: u64) -> io::Result<Vec<LegacyColumn>> {
    use egui_tiles::{Container, LinearDir, Tile};
    let mut windows: Vec<_> = windows.iter().enumerate().filter(|(_, w)| w.context_id == context_id).collect();
    windows.sort_by_key(|(index, w)| (w.grid_x, w.grid_y, *index));
    let mut columns = Vec::new();
    for (_, window) in windows {
        let Some(root) = window.tree.root() else { continue };
        let children: Vec<_> = match window.tree.tiles.get(root) {
            Some(Tile::Container(Container::Linear(linear))) if linear.dir == LinearDir::Horizontal => {
                let shares: Vec<_> = linear.children.iter().map(|id| linear.shares[*id]).collect();
                if shares.iter().any(|share| !share.is_finite() || *share <= 0.0) {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, format!("window {} has invalid column shares", window.window_id)));
                }
                let total: f32 = shares.iter().sum();
                if !total.is_finite() {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, format!("window {} has invalid column share total", window.window_id)));
                }
                // Egui's default shares sum to the number of children; divider
                // drags preserve that total. Strip projection stores pixel widths.
                let relative = total <= shares.len() as f32 + 0.0001;
                linear.children.iter().zip(shares.iter()).map(|(&id, &share)| {
                    let width = if relative {
                        Some(crate::strip::ColumnWidth::CustomRatio(share / total))
                    } else {
                        Some(crate::strip::ColumnWidth::CustomPixels(share))
                    };
                    (id, width)
                }).collect()
            }
            Some(Tile::Container(Container::Grid(_))) => {
                return Err(io::Error::new(io::ErrorKind::InvalidData, format!("window {} uses an unsupported legacy grid layout", window.window_id)));
            }
            _ => vec![(root, None)],
        };
        let mut visited = std::collections::HashSet::new();
        for (child, width) in children {
            let mut panes = Vec::new();
            collect_saved_strip_panes(window, child, &mut visited, &mut panes)?;
            if panes.is_empty() { continue; }
            let mut heights = vec![1.0; panes.len()];
            if let Some(Tile::Container(Container::Linear(linear))) = window.tree.tiles.get(child) {
                if linear.dir == LinearDir::Vertical && linear.children.len() == panes.len() {
                    let mut rows = Vec::new();
                    for &row in &linear.children {
                        let mut row_panes = Vec::new();
                        collect_saved_strip_panes(window, row, &mut std::collections::HashSet::new(), &mut row_panes)?;
                        rows.push((row_panes, linear.shares[row]));
                    }
                    if rows.iter().zip(&panes).all(|((row, share), pane)| row.as_slice() == [*pane] && share.is_finite() && *share > 0.0) {
                        heights = rows.into_iter().map(|(_, share)| share).collect();
                    }
                }
            }
            columns.push((panes, width, heights));
        }
    }
    Ok(columns)
}

fn saved_strip_columns(windows: &[SavedWindow], context_id: u64) -> io::Result<Vec<Vec<PaneId>>> {
    use egui_tiles::{Container, LinearDir, Tile};
    let mut windows: Vec<_> = windows.iter().enumerate().filter(|(_, w)| w.context_id == context_id).collect();
    windows.sort_by_key(|(index, w)| (w.grid_x, w.grid_y, *index));
    let mut columns = Vec::new();
    for (_, window) in windows {
        let Some(root) = window.tree.root() else { continue };
        let children: Vec<_> = match window.tree.tiles.get(root) {
            Some(Tile::Container(Container::Linear(linear))) if linear.dir == LinearDir::Horizontal => linear.children.clone(),
            Some(Tile::Container(Container::Grid(grid))) => grid.children().copied().collect(),
            _ => vec![root],
        };
        let mut visited = std::collections::HashSet::new();
        for child in children {
            let mut panes = Vec::new();
            collect_saved_strip_panes(window, child, &mut visited, &mut panes)?;
            if !panes.is_empty() { columns.push(panes); }
        }
    }
    Ok(columns)
}

fn collect_saved_strip_panes(
    window: &SavedWindow,
    tile_id: TileId,
    visited: &mut std::collections::HashSet<TileId>,
    panes: &mut Vec<PaneId>,
) -> io::Result<()> {
    use egui_tiles::{Container, Tile};
    if !visited.insert(tile_id) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("window {} has a repeated tile", window.window_id)));
    }
    let tile = window.tree.tiles.get(tile_id).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!("window {} references a missing tile", window.window_id)))?;
    match tile {
        Tile::Pane(id) => {
            if window.panes.iter().any(|pane| pane.id == *id && !pane.hidden && !matches!(pane.kind, SavedPaneKind::Portal { .. })) {
                panes.push(*id);
            }
        }
        Tile::Container(Container::Tabs(tabs)) => {
            if let Some(active) = tabs.active.or_else(|| tabs.children.first().copied()) {
                collect_saved_strip_panes(window, active, visited, panes)?;
            }
        }
        Tile::Container(container) => {
            for &child in container.children() {
                collect_saved_strip_panes(window, child, visited, panes)?;
            }
        }
    }
    Ok(())
}

/// Export the pre-migration workspace into a new, isolated profile.
pub fn export_v2_rollback(source_profile: &Path, destination_profile: &Path) -> io::Result<()> {
    let source = workspace_path_in(source_profile);
    let current: WorkspaceFile = serde_json::from_slice(&std::fs::read(&source)?)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, format!("invalid current workspace: {error}")))?;
    if current.version != 3 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "current workspace is not version 3"));
    }
    current.validate()?;
    let backup = source.with_extension("v2.rollback.json");
    let bytes = std::fs::read(&backup)?;
    let mut legacy: WorkspaceFile = serde_json::from_slice(&bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, format!("invalid rollback workspace: {error}")))?;
    if legacy.version != 2 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "rollback workspace is not version 2"));
    }
    legacy.remove_legacy_deleted_context_focus();
    legacy.validate()?;
    std::fs::create_dir(destination_profile)?;
    let result = (|| {
        let destination = workspace_path_in(destination_profile);
        let parent = destination.parent().ok_or_else(|| io::Error::other("rollback destination has no parent"))?;
        std::fs::create_dir(parent)?;
        let temp = unique_temp_path(&destination);
        let write = (|| {
            let mut file = OpenOptions::new().write(true).create_new(true).open(&temp)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            std::fs::hard_link(&temp, &destination)?;
            File::open(parent)?.sync_all()?;
            File::open(destination_profile)?.sync_all()
        })();
        if let Err(error) = std::fs::remove_file(&temp) {
            if error.kind() != io::ErrorKind::NotFound {
                log::warn!("workspace rollback: could not remove temporary file {}: {error}", temp.display());
            }
        }
        write
    })();
    if result.is_err() {
        log::error!("workspace rollback export failed for {}: {:?}", destination_profile.display(), result.as_ref().err());
    } else {
        log::info!("workspace rollback exported to {}", destination_profile.display());
    }
    result
}

fn preserve_v2_rollback(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let backup = path.with_extension("v2.rollback.json");
    let temp = unique_temp_path(&backup);
    let result = (|| {
        let mut file = OpenOptions::new().write(true).create_new(true).open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        match std::fs::hard_link(&temp, &backup) {
            Ok(()) => {
                File::open(path.parent().ok_or_else(|| io::Error::other("workspace has no parent"))?)?.sync_all()?;
                log::info!("workspace migration: preserved version 2 rollback copy at {}", backup.display());
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                if std::fs::read(&backup)? != bytes {
                    return Err(io::Error::new(io::ErrorKind::AlreadyExists, "existing version 2 rollback copy differs; refusing migration"));
                }
            }
            Err(error) => return Err(error),
        }
        Ok(())
    })();
    if let Err(error) = std::fs::remove_file(&temp) {
        if error.kind() != io::ErrorKind::NotFound {
            log::warn!("workspace migration: could not remove temporary file {}: {error}", temp.display());
        }
    }
    result
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
    fn invalid_workspace_load_preserves_original_file() {
        let profile = tempfile::tempdir().expect("temporary profile");
        let path = workspace_path_in(profile.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let original = b"{invalid workspace";
        std::fs::write(&path, original).unwrap();
        assert!(WorkspaceFile::load_from(profile.path()).is_none());
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }

    #[test]
    fn save_rejects_unreadable_existing_workspace() {
        let profile = tempfile::tempdir().expect("temporary profile");
        let path = workspace_path_in(profile.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let original = b"{invalid workspace";
        std::fs::write(&path, original).unwrap();
        let workspace: WorkspaceFile = serde_json::from_str(
            r#"{"version":2,"active_context":0,"sidebar_visible":true,"next_pane_id":1,"contexts":[],"windows":[]}"#,
        )
        .unwrap();
        assert!(workspace.save_to(profile.path()).is_err());
        assert_eq!(std::fs::read(path).unwrap(), original);
    }

    #[test]
    fn migration_preserves_original_v2_and_refuses_stale_downgrade() {
        let profile = tempfile::tempdir().unwrap();
        let path = workspace_path_in(profile.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let original = br#"{"version":2,"active_context":0,"sidebar_visible":true,"next_pane_id":1,"contexts":[],"windows":[]}"#;
        std::fs::write(&path, original).unwrap();
        let mut workspace: WorkspaceFile = serde_json::from_slice(original).unwrap();
        workspace.version = 3;
        workspace.save_to(profile.path()).unwrap();
        let backup = path.with_extension("v2.rollback.json");
        assert_eq!(std::fs::read(&backup).unwrap(), original);
        workspace.next_pane_id = 42;
        workspace.save_to(profile.path()).unwrap();
        let current = std::fs::read(&path).unwrap();
        assert_eq!(std::fs::read(&backup).unwrap(), original);
        workspace.version = 2;
        assert!(workspace.save_to(profile.path()).is_err());
        assert_eq!(std::fs::read(path).unwrap(), current);
    }

    #[test]
    fn rollback_export_preserves_current_workspace_and_refuses_existing_profile() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        std::fs::create_dir(&source).unwrap();
        let path = workspace_path_in(&source);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let original = br#"{"version":2,"active_context":0,"sidebar_visible":true,"next_pane_id":1,"contexts":[],"windows":[]}"#;
        std::fs::write(&path, original).unwrap();
        let mut current: WorkspaceFile = serde_json::from_slice(original).unwrap();
        current.version = 3;
        current.save_to(&source).unwrap();
        current.next_pane_id = 7;
        current.save_to(&source).unwrap();
        let newer = std::fs::read(&path).unwrap();
        let destination = root.path().join("isolated");
        export_v2_rollback(&source, &destination).unwrap();
        assert_eq!(std::fs::read(workspace_path_in(&destination)).unwrap(), original);
        assert_eq!(std::fs::read(&path).unwrap(), newer);
        assert!(export_v2_rollback(&source, &destination).is_err());
        assert_eq!(std::fs::read(workspace_path_in(&destination)).unwrap(), original);
    }

    #[test]
    fn migration_rejects_a_stale_rollback_copy() {
        let profile = tempfile::tempdir().unwrap();
        let path = workspace_path_in(profile.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let original = br#"{"version":2,"active_context":0,"sidebar_visible":true,"next_pane_id":1,"contexts":[],"windows":[]}"#;
        std::fs::write(&path, original).unwrap();
        std::fs::write(path.with_extension("v2.rollback.json"), b"stale").unwrap();
        let mut workspace: WorkspaceFile = serde_json::from_slice(original).unwrap();
        workspace.version = 3;
        assert!(workspace.save_to(profile.path()).is_err());
        assert_eq!(std::fs::read(path).unwrap(), original);
    }

    #[test]
    fn invalid_v3_strip_does_not_load_or_get_overwritten() {
        let profile = tempfile::tempdir().unwrap();
        let path = workspace_path_in(profile.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut workspace: WorkspaceFile = serde_json::from_str(
            r#"{"version":3,"active_context":0,"sidebar_visible":true,"next_pane_id":1,"contexts":[],"windows":[]}"#,
        ).unwrap();
        workspace.strips.insert(42, crate::strip::Strip::new(42));
        let original = serde_json::to_vec(&workspace).unwrap();
        std::fs::write(&path, &original).unwrap();
        assert!(WorkspaceFile::load_from(profile.path()).is_none());
        workspace.strips.clear();
        assert!(workspace.save_to(profile.path()).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }

    #[test]
    fn invalid_context_cycle_is_not_migrated() {
        let profile = tempfile::tempdir().unwrap();
        let path = workspace_path_in(profile.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let original = br#"{"version":2,"active_context":0,"sidebar_visible":true,"next_pane_id":1,"contexts":[{"name":"a","root":"/tmp","context_id":1,"parent_id":2},{"name":"b","root":"/tmp","context_id":2,"parent_id":1}],"windows":[]}"#;
        std::fs::write(&path, original).unwrap();
        assert!(WorkspaceFile::load_from(profile.path()).is_none());
        assert_eq!(std::fs::read(path).unwrap(), original);
    }

    #[test]
    fn invalid_active_window_reference_is_not_migrated() {
        let profile = tempfile::tempdir().unwrap();
        let path = workspace_path_in(profile.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let original = br#"{"version":2,"active_context":0,"sidebar_visible":true,"next_pane_id":1,"contexts":[{"name":"a","root":"/tmp","context_id":1}],"windows":[],"context_active_window":{"1":88}}"#;
        std::fs::write(&path, original).unwrap();
        assert!(WorkspaceFile::load_from(profile.path()).is_none());
        assert_eq!(std::fs::read(path).unwrap(), original);
    }

    #[test]
    fn invalid_next_pane_id_is_not_migrated() {
        let profile = tempfile::tempdir().unwrap();
        let path = workspace_path_in(profile.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let tree: Tree<PaneId> = Tree::empty("counter-fixture");
        let original = serde_json::json!({"version":2, "active_context":0, "sidebar_visible":true, "next_pane_id":1,
            "contexts":[{"name":"a", "root":"/tmp", "context_id":1}],
            "windows":[{"name":"a", "path":"/tmp", "tree":tree, "panes":[{"id":1, "cwd":"/tmp"}], "focused_pane":null, "window_id":1, "context_id":1}]}).to_string();
        std::fs::write(&path, &original).unwrap();
        assert!(WorkspaceFile::load_from(profile.path()).is_none());
        assert_eq!(std::fs::read(path).unwrap(), original.as_bytes());
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
                runtime_kind: None,
                launch: None,
                slots: HashMap::new(),
                agent_session: None,
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
            runtime_kind: None,
            launch: None,
            slots: HashMap::new(),
            agent_session: None,
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

    #[test]
    fn saved_pane_round_trips_slots_and_agent_session_and_append_load_agent_history() {
        use crate::protocol::{AgentBlockedReason, AgentState};
        let mut slots = HashMap::new();
        slots.insert("summary".to_string(), PathBuf::from("/tmp/slot_summary"));
        let pane = SavedPane {
            id: 42,
            kind: SavedPaneKind::Terminal,
            cwd: PathBuf::from("/tmp/work"),
            name: Some("worker-1".to_string()),
            app_id: None,
            app_state: None,
            hidden: false,
            heartbeat: None,
            runtime_kind: None,
            launch: None,
            slots: slots.clone(),
            agent_session: Some(SavedAgentSession {
                fleet_result: None,
                fleet_seen: false,
                fleet_dismissed: false,
                fleet_done_unseen: false,
                fleet_since: 0,
                harness: "claude".to_string(),
                config_name: Some("implementer".to_string()),
                permission_posture: None,
                session_id: Some("sess-42".to_string()),
                last_state: AgentState::Blocked,
                last_blocked_reason: Some(AgentBlockedReason::PermissionPrompt),
                last_detail: Some("Bash: git push".to_string()),
                parent_pane_id: Some(1),
                spawned_by: Some("agent:coordinator".to_string()),
                pending_child_results: Vec::new(),
                ephemeral: true,
                tail_preview: Some("Waiting for permission...".to_string()),
                standby_scrollback_tail: Some(vec!["Waiting for permission...".to_string()]),
            }),
        };
        let json = serde_json::to_string(&pane).expect("serialize SavedPane");
        let restored: SavedPane = serde_json::from_str(&json).expect("deserialize SavedPane");
        assert_eq!(restored.slots, slots);
        let session = restored.agent_session.expect("restored agent_session");
        assert_eq!(session.harness, "claude");
        assert_eq!(session.config_name.as_deref(), Some("implementer"));
        assert_eq!(session.session_id.as_deref(), Some("sess-42"));
        assert_eq!(session.last_state, AgentState::Blocked);
        assert_eq!(
            session.last_blocked_reason,
            Some(AgentBlockedReason::PermissionPrompt)
        );
        assert_eq!(session.parent_pane_id, Some(1));
        assert!(session.ephemeral);
        assert_eq!(
            session.tail_preview.as_deref(),
            Some("Waiting for permission...")
        );
        assert_eq!(
            session.standby_scrollback_tail,
            Some(vec!["Waiting for permission...".to_string()])
        );

        let mut legacy_json: serde_json::Value = serde_json::from_str(&json).expect("saved pane JSON");
        legacy_json["agent_session"]
            .as_object_mut()
            .expect("agent session object")
            .remove("standby_scrollback_tail");
        let legacy: SavedPane = serde_json::from_value(legacy_json).expect("legacy SavedPane");
        assert_eq!(
            legacy
                .agent_session
                .expect("legacy agent session")
                .standby_scrollback_tail,
            None,
            "old workspaces without a Standby scrollback tail remain readable"
        );

        let dir = tempfile::tempdir().expect("tempdir");
        let rec = AgentHistoryRecord {
            fleet_seen: false,
            fleet_dismissed: false,
            run_id: "run-42-100".to_string(),
            pane_id: 42,
            pane_name: Some("worker-1".to_string()),
            context_id: 2,
            context_name: "squad-ctx".to_string(),
            cwd: PathBuf::from("/tmp/work"),
            harness: "claude".to_string(),
            config_name: Some("implementer".to_string()),
            permission_posture: None,
            session_id: Some("sess-42".to_string()),
            last_state: AgentState::Completed,
            last_blocked_reason: None,
            last_detail: Some("Done".to_string()),
            parent_pane_id: Some(1),
            spawned_by: Some("agent:coordinator".to_string()),
            ephemeral: true,
            closed_at: 100,
            summary: Some("Implemented feature".to_string()),
            result: None,
            slots,
            tail_preview: Some("All tests passed.".to_string()),
        };
        append_agent_history_to(dir.path(), &rec).expect("append history");
        let loaded = load_agent_history_from(dir.path());
        assert_eq!(loaded, vec![rec]);
    }
}

/// Preserve an unseen completion before restore discards its finished ephemeral PTY.
pub(crate) fn archive_saved_completion(profile: &Path, pane: &SavedPane, context_id: u64, context_name: &str) {
    let Some(session) = &pane.agent_session else { return };
    if load_agent_history_from(profile).iter().any(|r| r.pane_id == pane.id && r.session_id == session.session_id) { return; }
    let result = session.fleet_result.clone().or_else(|| pane.slots.get("result").and_then(|path| {
        match std::fs::read_to_string(path).and_then(|data| serde_json::from_str(&data).map_err(io::Error::other)) {
            Ok(value) => Some(value),
            Err(error) => { log::warn!("agents: restore completion result {}: {error}", path.display()); None }
        }
    }));
    let record = AgentHistoryRecord {
        fleet_seen: session.fleet_seen,
        fleet_dismissed: session.fleet_dismissed,
        run_id: format!("run-{}-{}", pane.id, session.fleet_since),
        pane_id: pane.id, context_id, context_name: context_name.into(), parent_pane_id: session.parent_pane_id,
        spawned_by: session.spawned_by.clone(),
        pane_name: pane.name.clone(), harness: session.harness.clone(), config_name: session.config_name.clone(),
        permission_posture: session.permission_posture.clone(),
        session_id: session.session_id.clone(), cwd: pane.cwd.clone(), ephemeral: session.ephemeral,
        last_state: crate::protocol::AgentState::Completed, last_blocked_reason: None,
        last_detail: session.last_detail.clone(), summary: session.last_detail.clone(), result,
        tail_preview: session.tail_preview.clone(), slots: pane.slots.clone(), closed_at: session.fleet_since,
    };
    if let Err(error) = append_agent_history_to(profile, &record) {
        log::error!("agents: archive restored completion pane={}: {error}", pane.id);
    }
}
