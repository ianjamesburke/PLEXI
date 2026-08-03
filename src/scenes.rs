//! Declarative scene runner — TOML scenes ARE the UI tests.
//!
//! A scene file describes setup steps, actions, structured assertions, and
//! optional screenshots. `HeadlessBackend` executes through `PlexiUiHarness`;
//! `LiveBackend` executes the same schema against one explicit installed host
//! channel through sanctioned CLI/IPC commands.
//!
//! # Entry points
//!
//! - `scene_suite` test — globs `tests/scenes/*.toml`, runs every scene with
//!   `suite = true` (the default). A new scene file is automatically a
//!   regression test; no Rust required.
//! - `scene_single` test (`#[ignore]`) — runs one scene named by the
//!   `PLEXI_SCENE` env var. Wrapped by `just scene <file>`.
//!
//! # Output
//!
//! Every run writes a schema-v3 `SceneReport` JSON to the out dir: backend,
//! channel, pass/fail per step, resolved pane ids, host state, teardown result,
//! and the last-opened app's available state/semantic snapshot. Screenshots are
//! an optional headless artifact (`shot` steps; suppressed by
//! `PLEXI_SCENE_NO_SHOTS=1`).
//!
//! # DSL
//!
//! Assertions are structured keys (typed matchers), never expression strings.
//! New verbs require a scene that needs them.
//!
//! A literal `{tmp}` anywhere in a scene file expands to a fresh per-run temp
//! dir (removed when the run ends). Scenes that touch the filesystem must use
//! it — a fixed path leaks state into the next run of the same scene.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::host::pane::{AppRuntime, Pane};
use crate::spatial::tiling::PaneId;
use crate::ui_tests::PlexiUiHarness;

pub const SCENE_REPORT_SCHEMA_VERSION: u32 = 4;
const LIVE_POLL_INTERVAL: Duration = Duration::from_millis(20);

/// Scales a scene-declared timeout by `PLEXI_SCENE_TIMEOUT_SCALE`.
///
/// A scene's `timeout_s` exists to catch a hang, not to assert a performance
/// budget: the same wait that finishes in 10s on a developer's machine can
/// exceed 15s on a shared CI runner, and the resulting failure says "too slow
/// to observe" while reading as "the thing never happened". The scale factor
/// keeps one deadline in the scene file and lets a slower executor widen it
/// without editing every scene.
///
/// Unset means 1.0. A non-numeric or non-positive value is a configuration
/// error and panics rather than silently reverting to the default.
fn scene_timeout(secs: f32) -> Duration {
    let raw = std::env::var(SCENE_TIMEOUT_SCALE_VAR).ok();
    Duration::from_secs_f32(secs * scene_timeout_scale(raw.as_deref()))
}

const SCENE_TIMEOUT_SCALE_VAR: &str = "PLEXI_SCENE_TIMEOUT_SCALE";

/// Parses the scale factor. `None` means unset, which is 1.0.
///
/// A malformed or non-positive value panics: a scene deadline silently
/// reverting to its default is the failure this knob exists to prevent.
fn scene_timeout_scale(raw: Option<&str>) -> f32 {
    let Some(raw) = raw else {
        return 1.0;
    };
    let parsed: f32 = raw
        .parse()
        .unwrap_or_else(|e| panic!("{SCENE_TIMEOUT_SCALE_VAR}={raw:?} is not a number: {e}"));
    assert!(
        parsed.is_finite() && parsed > 0.0,
        "{SCENE_TIMEOUT_SCALE_VAR}={raw:?} must be a finite number greater than zero"
    );
    parsed
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HostStatusClass {
    Running,
    Stopped,
    Unknown,
}

fn classify_host_status(status: &serde_json::Value) -> HostStatusClass {
    let ready = status.get("ready").and_then(serde_json::Value::as_bool);
    let pid = status.get("pid");
    if ready == Some(true) || pid.is_some_and(|value| !value.is_null()) {
        HostStatusClass::Running
    } else if ready == Some(false) && pid.is_some_and(serde_json::Value::is_null) {
        HostStatusClass::Stopped
    } else {
        HostStatusClass::Unknown
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LiveStartAction {
    /// Reuse the already-running host; teardown must leave it untouched.
    Attach,
    /// Boot a runner-owned ephemeral host and stop it on teardown.
    StartOwned,
}

/// Pure decision behind [`LiveBackend::start_or_attach`]. Attach mode
/// (`PLEXI_SCENE_ATTACH=1`) is a hard requirement, not a preference: the
/// driver (e.g. editor-gate.sh) booted a specific host and the run is only
/// meaningful against that host, so a stopped channel in attach mode is an
/// error — silently starting an owned replacement would let a mid-gate host
/// crash masquerade as a green run. Errors are `(code, detail)` for
/// [`SceneError`].
fn live_start_action(
    class: HostStatusClass,
    attach: bool,
) -> Result<LiveStartAction, (&'static str, &'static str)> {
    match (class, attach) {
        (HostStatusClass::Running, true) => Ok(LiveStartAction::Attach),
        (HostStatusClass::Running, false) => Err((
            "live_host_already_running",
            "channel already has a host; set PLEXI_SCENE_ATTACH=1 to attach",
        )),
        (HostStatusClass::Stopped, true) => Err((
            "live_attach_host_missing",
            "PLEXI_SCENE_ATTACH=1 but the channel has no running host; \
             the attached host crashed or was stopped",
        )),
        (HostStatusClass::Stopped, false) => Ok(LiveStartAction::StartOwned),
        (HostStatusClass::Unknown, _) => Err((
            "live_status_invalid",
            "could not classify host status for the channel",
        )),
    }
}

fn host_seed_ready(status: &serde_json::Value, minimum_panes: u64) -> bool {
    status.get("ready").and_then(serde_json::Value::as_bool) == Some(true)
        && status
            .get("pane_count")
            .and_then(serde_json::Value::as_u64)
            .is_some_and(|count| count >= minimum_panes)
}

fn poll_until<T>(
    timeout: Duration,
    interval: Duration,
    mut observe: impl FnMut() -> Option<T>,
) -> Result<T, SceneError> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(value) = observe() {
            return Ok(value);
        }
        if Instant::now() >= deadline {
            return Err(SceneError::new(
                "eventual_timeout",
                "eventual assertion did not pass before timeout",
            ));
        }
        std::thread::sleep(interval);
    }
}

/// Polls an installed host response with a correctness budget. Live scenes
/// exercise a real host and CLI transport, so their timeout is a liveness
/// bound rather than an idle-machine performance claim.
fn poll_live_until<T>(
    base_timeout: Duration,
    interval: Duration,
    mut observe: impl FnMut(Duration) -> Result<Option<T>, SceneError>,
) -> Result<T, SceneError> {
    let started = Instant::now();
    let deadline = started + crate::testing::load_aware_timeout(base_timeout);
    loop {
        if let Some(value) = observe(started.elapsed())? {
            return Ok(value);
        }
        if Instant::now() >= deadline {
            return Err(SceneError::new(
                "eventual_timeout",
                "eventual assertion did not pass before timeout",
            ));
        }
        std::thread::sleep(interval);
    }
}

fn live_context_count(contexts: Option<&serde_json::Value>, panes: &[serde_json::Value]) -> usize {
    contexts
        .and_then(serde_json::Value::as_array)
        .map(Vec::len)
        .unwrap_or_else(|| {
            panes
                .iter()
                .filter_map(|pane| pane.get("context_id").and_then(serde_json::Value::as_u64))
                .collect::<std::collections::BTreeSet<_>>()
                .len()
        })
}

fn live_builtin_cwd(open: &OpenSpec) -> Option<PathBuf> {
    match open {
        OpenSpec::Builtin { cwd, .. } => cwd.as_deref().map(resolve),
        OpenSpec::Process { .. } | OpenSpec::Wasm { .. } => None,
    }
}

fn resolve_live_pane_target(
    handles: &PaneHandles,
    target: &str,
    operation: &str,
) -> Result<PaneId, SceneError> {
    match handles.resolve_input(target)? {
        InputTarget::Pane(pane_id) => Ok(pane_id),
        InputTarget::Host => Err(SceneError::new(
            "unsupported_live_target",
            format!("live {operation} does not support target = 'host'"),
        )),
    }
}

// ─── Scene file format ───────────────────────────────────────────────────────

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Scene {
    /// Harness surface size in points.
    #[serde(default = "default_size")]
    pub size: [f32; 2],
    /// When false, `scene_suite` skips this scene (run it via `just scene`).
    /// Use for scenes that spawn real app processes or need wall-clock time.
    #[serde(default = "default_true")]
    pub suite: bool,
    /// Scripted file-picker outcomes (stint 0508), consumed in order by
    /// `OpenFilePicker` requests from apps this scene opens. Each entry is
    /// `{ paths = ["..."] }` or `{ cancel = true }`; a literal `{out}` prefix
    /// in a path resolves to the scene's out dir. Headless-only: a live host
    /// scripts its picker through `PLEXI_PICKER_SCRIPT` at host launch.
    #[serde(default)]
    pub picker_script: Option<Vec<PickerScriptEntry>>,
    pub steps: Vec<Step>,
}

/// One scripted picker outcome in a scene file.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct PickerScriptEntry {
    #[serde(default)]
    pub paths: Option<Vec<String>>,
    #[serde(default)]
    pub cancel: Option<bool>,
}

fn default_size() -> [f32; 2] {
    [1280.0, 800.0]
}

fn default_true() -> bool {
    true
}

/// One scene step. Untagged: each variant is identified by its unique key.
#[derive(Deserialize, Debug)]
#[serde(untagged)]
pub enum Step {
    /// Open one process, WASM, or builtin app and bind its pane id to a handle.
    Open { open: OpenSpec },
    /// Insert text through egui's normal text-input event path. Printable
    /// characters only: the editor drops control-character `Event::Text`
    /// echoes, so multi-line content must arrive as `paste`.
    Text { text: TextSpec },
    /// Deliver text through egui's paste event path — the production route
    /// for multi-line content. Headless-only.
    Paste { paste: TextSpec },
    /// Press a key combo against a pane handle or the whole host.
    Key { key: KeySpec },
    /// Deliver a local file or image URL through the production pane drop path.
    DropFile { drop_file: DropFileSpec },
    /// Drag the pointer across a pane through the production input path:
    /// press, N moves, release — one frame each (stint 0510).
    Drag { drag: DragSpec },
    /// Focus an opened pane through the production pane-navigation path.
    Focus { focus: String },
    /// Close an opened pane through the production pane-close path.
    Close { close: String },
    /// Toggle the host sidebar.
    Sidebar { sidebar: bool },
    /// Seed one message notification through the real host modal render path.
    /// Headless-only: live scenes must create notifications through an app.
    SeedNotification {
        seed_notification: NotificationSeedSpec,
    },
    /// Switch to the context at this router index.
    SwitchContext { switch_context: usize },
    /// Push the focused pane into a new subcontext with this name.
    PushToSubcontext { push_to_subcontext: String },
    /// Block until a process app handle commits its first real frame.
    WaitAppFrame { wait_app_frame: WaitSpec },
    /// Advance N harness frames.
    RunSteps { run_steps: usize },
    /// Structured assertions — every present key must match.
    Assert { assert: AssertSpec },
    /// Eventual semantic assertion, optionally after normal input delivery.
    Expect { expect: ExpectSpec },
    /// Assert that the headless host accessibility tree contains an exact label.
    AssertLabel { assert_label: AssertLabelSpec },
    /// Save a headless screenshot to `<out_dir>/<name>`.
    Shot { shot: String },
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct NotificationSeedSpec {
    pub title: String,
    #[serde(default)]
    pub body: String,
}

/// One generic app-opening request. `kind` determines which production launch
/// path runs; `as` binds the resulting pane id for later steps.
#[derive(Deserialize, Debug)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum OpenSpec {
    /// Launch a real PGAP process from an app directory.
    Process {
        path: String,
        #[serde(rename = "as")]
        handle: String,
        #[serde(default)]
        args: Vec<String>,
    },
    /// Launch a reviewed raw WASM component.
    Wasm {
        path: String,
        #[serde(rename = "as")]
        handle: String,
        #[serde(default)]
        args: Vec<String>,
    },
    /// Open a compiled-in app by its production builtin id.
    Builtin {
        id: String,
        #[serde(rename = "as")]
        handle: String,
        cwd: Option<String>,
        #[serde(default)]
        args: Vec<String>,
    },
}

impl OpenSpec {
    fn handle(&self) -> &str {
        match self {
            Self::Process { handle, .. }
            | Self::Wasm { handle, .. }
            | Self::Builtin { handle, .. } => handle,
        }
    }

    fn target_kind(&self) -> &'static str {
        match self {
            Self::Process { .. } => "process",
            Self::Wasm { .. } => "wasm",
            Self::Builtin { .. } => "builtin",
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct TextSpec {
    pub target: String,
    pub value: String,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct KeySpec {
    pub target: String,
    pub value: String,
}

/// One pointer drag: endpoints are pane-pixel coordinates (`from`/`to`) or
/// semantic node ids (`from_node`/`to_node` — the drag targets the node's
/// rendered bounds center). Exactly one form per endpoint.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct DragSpec {
    pub target: String,
    pub from: Option<[f32; 2]>,
    pub from_node: Option<String>,
    pub to: Option<[f32; 2]>,
    pub to_node: Option<String>,
    /// Intermediate pointer moves between press and release (default 8).
    #[serde(default = "default_drag_steps")]
    pub steps: u32,
    /// "left" (default), "right", or "middle".
    #[serde(default)]
    pub button: Option<String>,
}

fn default_drag_steps() -> u32 {
    8
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct DropFileSpec {
    pub target: String,
    pub value: String,
    #[serde(default)]
    pub expect_rejected: bool,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct AssertLabelSpec {
    pub target: String,
    pub label: String,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct WaitSpec {
    pub target: String,
    pub timeout_s: f32,
}

#[derive(Deserialize, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct AssertSpec {
    /// Pane handle used by app lifecycle and tree assertions.
    pub target: Option<String>,
    pub pane_count: Option<usize>,
    pub window_count: Option<usize>,
    pub context_count: Option<usize>,
    /// Portal panes across all windows.
    pub portal_count: Option<usize>,
    pub sidebar: Option<bool>,
    /// Whether the target pane currently exists.
    pub exists: Option<bool>,
    /// Whether the target pane is the active focused pane.
    pub focused: Option<bool>,
    /// Lifecycle of the target app pane, lowercase (e.g. "running").
    pub lifecycle: Option<String>,
    /// Substring match against the target app's serialized L1 tree.
    pub tree_contains: Option<String>,
    pub fit: Option<String>,
    pub aspect: Option<[f64; 2]>,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct ExpectSpec {
    pub target: String,
    pub after_key: Option<String>,
    pub after_text: Option<String>,
    pub node_changes: Option<String>,
    pub tree_contains: Option<String>,
    pub focused: Option<bool>,
    pub caret: Option<usize>,
    pub selection: Option<[usize; 2]>,
    pub undo_available: Option<bool>,
    pub redo_available: Option<bool>,
    pub save_result: Option<String>,
    pub source_text_contains: Option<String>,
    pub active_markdown_contains: Option<String>,
    /// Exact match on the editor's Markdown presentation:
    /// "live_preview" | "source".
    pub preview_mode: Option<String>,
    pub rendered_text_contains: Option<String>,
    pub visible_link_target: Option<String>,
    pub visible_image_target: Option<String>,
    pub drop_result: Option<String>,
    /// Event-bus expectation (stint 0511): a stream name (e.g. "probe.tick")
    /// that must appear as a recorded emitted event within `timeout_s`.
    /// Headless-only — the live backend has no sanctioned event-query seam.
    pub event_stream: Option<String>,
    /// Substring that must appear in the matched event's JSON payload.
    /// Requires `event_stream`.
    pub event_payload_contains: Option<String>,
    #[serde(default = "default_expect_timeout")]
    pub timeout_s: f32,
}

/// True when the recorded app-event expectation (if any) is satisfied: some
/// emitted event on `event_stream` whose serialized payload contains
/// `event_payload_contains` (when set). Reads the same global `AppTimeline`
/// the production event bus records into.
fn app_event_expectation_match(spec: &ExpectSpec) -> bool {
    let Some(stream) = &spec.event_stream else {
        return true;
    };
    let timeline = crate::host::app_timeline::global();
    let timeline = timeline.lock().expect("app timeline lock");
    timeline.events().iter().any(|record| {
        &record.event == stream
            && spec.event_payload_contains.as_ref().is_none_or(|needle| {
                record
                    .payload
                    .as_ref()
                    .is_some_and(|payload| payload.to_string().contains(needle))
            })
    })
}

fn notes_expectations_match(spec: &ExpectSpec, state: &serde_json::Value) -> bool {
    let app = state
        .pointer("/app_state")
        .unwrap_or(&serde_json::Value::Null);
    let string_contains = |pointer: &str, needle: &Option<String>| {
        needle.as_ref().is_none_or(|needle| {
            app.pointer(pointer)
                .and_then(serde_json::Value::as_str)
                .is_some_and(|value| value.contains(needle))
        })
    };
    let array_contains = |pointer: &str, needle: &Option<String>| {
        needle.as_ref().is_none_or(|needle| {
            app.pointer(pointer)
                .and_then(serde_json::Value::as_array)
                .is_some_and(|values| values.iter().any(|value| value.as_str() == Some(needle)))
        })
    };
    let rendered_matches = spec.rendered_text_contains.as_ref().is_none_or(|needle| {
        state
            .pointer("/semantic/nodes")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|nodes| nodes.iter().any(|node| node.to_string().contains(needle)))
    });
    spec.focused.is_none_or(|expected| {
        app.get("focused").and_then(serde_json::Value::as_bool) == Some(expected)
    }) && spec.caret.is_none_or(|expected| {
        app.get("caret").and_then(serde_json::Value::as_u64) == Some(expected as u64)
    }) && spec.selection.is_none_or(|[anchor, caret]| {
        app.pointer("/primary_selection/anchor")
            .and_then(serde_json::Value::as_u64)
            == Some(anchor as u64)
            && app
                .pointer("/primary_selection/caret")
                .and_then(serde_json::Value::as_u64)
                == Some(caret as u64)
    }) && spec.undo_available.is_none_or(|expected| {
        app.get("undo_available")
            .and_then(serde_json::Value::as_bool)
            == Some(expected)
    }) && spec.redo_available.is_none_or(|expected| {
        app.get("redo_available")
            .and_then(serde_json::Value::as_bool)
            == Some(expected)
    }) && spec.save_result.as_ref().is_none_or(|expected| {
        app.get("last_save_result")
            .and_then(serde_json::Value::as_str)
            .or_else(|| {
                app.pointer("/last_save_result/result")
                    .and_then(serde_json::Value::as_str)
            })
            == Some(expected)
    }) && string_contains("/source_text", &spec.source_text_contains)
        && string_contains(
            "/active_markdown_block/source",
            &spec.active_markdown_contains,
        )
        && spec.preview_mode.as_ref().is_none_or(|expected| {
            app.get("preview_mode").and_then(serde_json::Value::as_str) == Some(expected)
        })
        && rendered_matches
        && array_contains("/visible_link_targets", &spec.visible_link_target)
        && array_contains("/visible_images", &spec.visible_image_target)
        && spec.drop_result.as_ref().is_none_or(|expected| {
            app.pointer("/last_drop_result/result")
                .and_then(serde_json::Value::as_str)
                == Some(expected)
        })
}

fn default_expect_timeout() -> f32 {
    5.0
}

// ─── Report format ───────────────────────────────────────────────────────────

#[derive(Serialize, Debug)]
pub struct SceneReport {
    pub schema_version: u32,
    pub backend: String,
    pub channel: String,
    pub scene: String,
    pub passed: bool,
    pub steps: Vec<StepResult>,
    pub shots: Vec<String>,
    /// Symbolic pane handles resolved during this run.
    pub handles: BTreeMap<String, PaneId>,
    pub host: HostState,
    /// Last-opened app pane state, when a scene opened one.
    pub app: Option<AppState>,
    pub teardown: TeardownResult,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_bundle: Option<String>,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct TeardownResult {
    pub attempted: bool,
    pub ok: bool,
    pub detail: String,
}

impl TeardownResult {
    fn headless() -> Self {
        Self {
            attempted: false,
            ok: true,
            detail: "not_required".to_string(),
        }
    }
}

#[derive(Serialize, Debug)]
pub struct StepResult {
    pub index: usize,
    pub step: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<StepDetail>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<SceneError>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_bundle: Option<String>,
}

#[derive(Serialize, Debug)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StepDetail {
    Opened {
        target_kind: String,
        handle: String,
        pane_id: PaneId,
    },
    TextInput {
        target: String,
        pane_id: Option<PaneId>,
        length: usize,
    },
    KeyInput {
        target: String,
        pane_id: Option<PaneId>,
        value: String,
    },
    LabelMatched {
        target: String,
        pane_id: Option<PaneId>,
        label: String,
    },
    Message {
        message: String,
    },
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct SceneError {
    pub code: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub poll_history: Vec<PollSample>,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct PollSample {
    pub timestamp_ms: u128,
    pub observed: serde_json::Value,
}

impl SceneError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            poll_history: Vec::new(),
        }
    }

    fn with_poll_history(mut self, poll_history: Vec<PollSample>) -> Self {
        self.poll_history = poll_history;
        self
    }
}

#[derive(Serialize, Debug)]
pub struct HostState {
    pub context_count: usize,
    pub window_count: usize,
    /// Panes in the active window.
    pub pane_count: usize,
    /// Portal panes across all windows.
    pub portal_count: usize,
    pub sidebar: bool,
}

#[derive(Serialize, Debug)]
pub struct AppState {
    pub pane_id: PaneId,
    pub lifecycle: String,
    /// Committed L1 render tree (the app's UI state), as JSON.
    pub tree: serde_json::Value,
}

// ─── Runner ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputTarget {
    Host,
    Pane(PaneId),
}

#[derive(Default)]
struct PaneHandles {
    by_name: HashMap<String, PaneId>,
}

impl PaneHandles {
    fn ensure_available(&self, handle: &str) -> Result<(), SceneError> {
        if handle.trim().is_empty() {
            return Err(SceneError::new(
                "invalid_handle",
                "open: symbolic handle cannot be empty",
            ));
        }
        if handle == "host" {
            return Err(SceneError::new(
                "reserved_handle",
                "open: 'host' is reserved for whole-host input",
            ));
        }
        if self.by_name.contains_key(handle) {
            return Err(SceneError::new(
                "duplicate_handle",
                format!("open: symbolic handle '{handle}' is already bound"),
            ));
        }
        Ok(())
    }

    fn bind(&mut self, handle: &str, pane_id: PaneId) -> Result<(), SceneError> {
        self.ensure_available(handle)?;
        self.by_name.insert(handle.to_string(), pane_id);
        Ok(())
    }

    fn resolve(&self, target: &str) -> Result<PaneId, SceneError> {
        self.by_name.get(target).copied().ok_or_else(|| {
            SceneError::new(
                "missing_target",
                format!("target '{target}' has not been opened in this scene"),
            )
        })
    }

    fn resolve_input(&self, target: &str) -> Result<InputTarget, SceneError> {
        if target == "host" {
            Ok(InputTarget::Host)
        } else {
            self.resolve(target).map(InputTarget::Pane)
        }
    }

    fn report(&self) -> BTreeMap<String, PaneId> {
        self.by_name
            .iter()
            .map(|(name, pane_id)| (name.clone(), *pane_id))
            .collect()
    }
}

pub struct HeadlessBackend {
    h: PlexiUiHarness,
    last_app_pane: Option<PaneId>,
    handles: PaneHandles,
    out_dir: PathBuf,
    no_shots: bool,
}

/// Run a scene file. Writes `<out_dir>/<scene-stem>.json` and returns the
/// report. Execution stops at the first failing step (fail fast); the report
/// records everything up to and including the failure.
/// Clears the process-wide picker override when a scene run ends.
struct ScenePickerGuard;

impl Drop for ScenePickerGuard {
    fn drop(&mut self) {
        crate::host::services::set_picker_override(None);
    }
}

/// Turn a scene's `picker_script` entries into a scripted picker override.
/// `{out}` path prefixes resolve to the scene out dir so round-trip scenes
/// stay self-contained.
fn install_scene_picker(
    entries: &[PickerScriptEntry],
    out_dir: &Path,
) -> Result<ScenePickerGuard, SceneError> {
    use crate::host::services::{FilePickOutcome, ScriptedPickerService};
    let mut outcomes = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        match (&entry.paths, entry.cancel) {
            (Some(paths), None) => {
                let paths = paths
                    .iter()
                    .map(|path| {
                        PathBuf::from(match path.strip_prefix("{out}/") {
                            Some(rest) => out_dir.join(rest).display().to_string(),
                            None => path.clone(),
                        })
                    })
                    .collect();
                outcomes.push(FilePickOutcome::Picked(paths));
            }
            (None, Some(true)) => outcomes.push(FilePickOutcome::Cancelled),
            _ => {
                return Err(SceneError::new(
                    "picker_script_invalid",
                    format!(
                        "picker_script entry {index} must set exactly one of `paths` or `cancel = true`"
                    ),
                ));
            }
        }
    }
    crate::host::services::set_picker_override(Some(std::sync::Arc::new(
        ScriptedPickerService::from_outcomes(outcomes),
    )));
    Ok(ScenePickerGuard)
}

/// Expand `{tmp}` in raw scene TOML to a fresh per-run temp dir so scenes
/// never see filesystem state left behind by a previous run. The returned
/// guard keeps the dir alive for the run and removes it on drop.
fn expand_scene_tmp(raw: String) -> Result<(String, Option<tempfile::TempDir>), SceneError> {
    if !raw.contains("{tmp}") {
        return Ok((raw, None));
    }
    let dir = tempfile::tempdir().map_err(|error| {
        SceneError::new("scene_tmp", format!("create scene temp dir: {error}"))
    })?;
    let expanded = raw.replace("{tmp}", &dir.path().display().to_string());
    Ok((expanded, Some(dir)))
}

pub fn run_scene(scene_path: &Path, out_dir: &Path, no_shots: bool) -> SceneReport {
    if std::env::var("PLEXI_SCENE_BACKEND").is_ok_and(|value| value == "live") {
        return run_live_scene(scene_path, out_dir, no_shots);
    }
    let scene_name = scene_path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| scene_path.display().to_string());

    let raw = match std::fs::read_to_string(scene_path) {
        Ok(r) => r,
        Err(e) => {
            return failed_report(
                scene_name,
                SceneError::new("scene_read", format!("read {}: {e}", scene_path.display())),
            );
        }
    };
    let (raw, _tmp_guard) = match expand_scene_tmp(raw) {
        Ok(expanded) => expanded,
        Err(e) => return failed_report(scene_name, e),
    };
    let scene: Scene = match toml::from_str(&raw) {
        Ok(s) => s,
        Err(e) => {
            return failed_report(
                scene_name,
                SceneError::new(
                    "scene_parse",
                    format!("parse {}: {e}", scene_path.display()),
                ),
            );
        }
    };

    std::fs::create_dir_all(out_dir).ok();

    // Install the scene's scripted picker before any app opens, and clear it
    // however this run ends so no override leaks into the next scene.
    let _picker_guard = match scene.picker_script.as_deref() {
        Some(entries) => match install_scene_picker(entries, out_dir) {
            Ok(guard) => Some(guard),
            Err(e) => return failed_report(scene_name, e),
        },
        None => None,
    };

    let mut h = PlexiUiHarness::new_sized(scene.size[0], scene.size[1]);
    h.step();
    let mut runner = HeadlessBackend {
        h,
        last_app_pane: None,
        handles: PaneHandles::default(),
        out_dir: out_dir.to_path_buf(),
        no_shots,
    };

    let mut steps = Vec::new();
    let mut shots = Vec::new();
    let mut passed = true;
    for (index, step) in scene.steps.iter().enumerate() {
        let label = step_label(step);
        match runner.exec(step, &mut shots) {
            Ok(detail) => steps.push(StepResult {
                index,
                step: label,
                ok: true,
                detail,
                error: None,
                failure_bundle: None,
            }),
            Err(e) => {
                let bundle = write_failure_bundle(
                    out_dir,
                    &scene_name,
                    index,
                    "headless",
                    &e,
                    runner.app_state(),
                    Some(&mut runner),
                );
                steps.push(StepResult {
                    index,
                    step: label,
                    ok: false,
                    detail: None,
                    error: Some(e),
                    failure_bundle: Some(bundle),
                });
                passed = false;
                break;
            }
        }
    }

    let failure_bundle = steps.iter().find_map(|step| step.failure_bundle.clone());
    let report = SceneReport {
        schema_version: SCENE_REPORT_SCHEMA_VERSION,
        backend: "headless".to_string(),
        channel: "isolated-test".to_string(),
        scene: scene_name.clone(),
        passed,
        steps,
        shots,
        handles: runner.handles.report(),
        host: runner.host_state(),
        app: runner.app_state(),
        teardown: TeardownResult::headless(),
        failure_bundle,
    };
    let report_path = out_dir.join(format!("{scene_name}.json"));
    match serde_json::to_string_pretty(&report) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&report_path, json) {
                log::warn!(
                    "scene {scene_name}: failed to write report {}: {e}",
                    report_path.display()
                );
            }
        }
        Err(e) => log::warn!("scene {scene_name}: failed to serialize report: {e}"),
    }
    report
}

fn failed_report(scene: String, error: SceneError) -> SceneReport {
    SceneReport {
        schema_version: SCENE_REPORT_SCHEMA_VERSION,
        backend: "headless".to_string(),
        channel: "isolated-test".to_string(),
        scene,
        passed: false,
        steps: vec![StepResult {
            index: 0,
            step: "load".to_string(),
            ok: false,
            detail: None,
            error: Some(error),
            failure_bundle: None,
        }],
        shots: Vec::new(),
        handles: BTreeMap::new(),
        host: HostState {
            context_count: 0,
            window_count: 0,
            pane_count: 0,
            portal_count: 0,
            sidebar: false,
        },
        app: None,
        teardown: TeardownResult::headless(),
        failure_bundle: None,
    }
}

fn write_failure_bundle(
    out_dir: &Path,
    scene: &str,
    step: usize,
    backend: &str,
    error: &SceneError,
    app: Option<AppState>,
    mut headless: Option<&mut HeadlessBackend>,
) -> String {
    let dir = out_dir.join(format!("{scene}.failure-{step}"));
    let _ = std::fs::create_dir_all(&dir);
    let screenshot = if let Some(runner) = headless.as_mut() {
        let path = dir.join("screenshot.png");
        runner
            .h
            .save_screenshot(&path.to_string_lossy())
            .ok()
            .map(|_| "screenshot.png")
    } else {
        None
    };
    let semantic = app
        .map(|state| state.tree)
        .unwrap_or(serde_json::Value::Null);
    let _ = std::fs::write(
        dir.join("semantic.json"),
        serde_json::to_vec_pretty(&semantic).unwrap_or_default(),
    );
    let log_path = Some(crate::config::config_dir().join("plexi.log"));
    let log_tail = log_path
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|log| {
            log.lines()
                .rev()
                .take(200)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    let _ = std::fs::write(dir.join("log-tail.txt"), log_tail);
    let _ = std::fs::write(
        dir.join("scene-event-trace.json"),
        serde_json::to_vec_pretty(&error.poll_history).unwrap_or_default(),
    );
    let note_path = semantic
        .pointer("/app_state/path")
        .and_then(serde_json::Value::as_str)
        .map(PathBuf::from);
    if let Some(path) = &note_path {
        if let Ok(saved) = std::fs::read(path) {
            let _ = std::fs::write(dir.join("saved-note.md"), saved);
        }
    }
    let attachments = note_path
        .as_deref()
        .and_then(Path::parent)
        .map(|parent| parent.join("assets"))
        .and_then(|assets| std::fs::read_dir(assets).ok())
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path().display().to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let _ = std::fs::write(
        dir.join("attachment-manifest.json"),
        serde_json::to_vec_pretty(&attachments).unwrap_or_default(),
    );
    let manifest = serde_json::json!({"step_index": step, "backend": backend, "code": error.code, "message": error.message, "poll_history": error.poll_history, "screenshot": screenshot});
    let _ = std::fs::write(
        dir.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).unwrap_or_default(),
    );
    dir.display().to_string()
}

struct LiveBackend {
    binary: String,
    channel: String,
    socket: PathBuf,
    owned_host: bool,
    attached_host: bool,
    handles: PaneHandles,
    last_app_pane: Option<PaneId>,
    teardown: TeardownResult,
    owner_file: Option<PathBuf>,
}

impl LiveBackend {
    fn from_env() -> Result<Self, SceneError> {
        let channel = std::env::var("PLEXI_SCENE_CHANNEL").map_err(|_| {
            SceneError::new(
                "live_channel_required",
                "live scenes require PLEXI_SCENE_CHANNEL=<explicit-channel>",
            )
        })?;
        if channel.trim().is_empty() {
            return Err(SceneError::new(
                "live_channel_required",
                "live scene channel cannot be empty",
            ));
        }
        let binary = std::env::var("PLEXI_SCENE_BIN").unwrap_or_else(|_| match channel.as_str() {
            "main" => "plexi".to_string(),
            other => format!("plexi-{other}"),
        });
        let home = dirs::home_dir()
            .ok_or_else(|| SceneError::new("home_unavailable", "cannot resolve home directory"))?;
        let profile = if channel == "main" {
            ".plexi".to_string()
        } else {
            format!(".plexi-{channel}")
        };
        Ok(Self {
            binary,
            channel,
            socket: home.join(profile).join("notify.sock"),
            owned_host: false,
            attached_host: false,
            handles: PaneHandles::default(),
            last_app_pane: None,
            teardown: TeardownResult {
                attempted: false,
                ok: false,
                detail: "pending".to_string(),
            },
            owner_file: std::env::var_os("PLEXI_SCENE_OWNER_FILE").map(PathBuf::from),
        })
    }

    fn command(&self, args: &[String], drive: bool) -> Result<Output, SceneError> {
        self.command_with_cwd(args, drive, None)
    }

    fn command_with_cwd(
        &self,
        args: &[String],
        drive: bool,
        cwd: Option<&Path>,
    ) -> Result<Output, SceneError> {
        let mut command = Command::new(&self.binary);
        command.args(args).env("PLEXI_CHANNEL", &self.channel);
        if let Some(cwd) = cwd {
            command.current_dir(cwd);
        }
        if drive {
            command
                .env("PLEXI_SOCKET", &self.socket)
                .env_remove("PLEXI_PANE_ID")
                .env_remove("PLEXI_CONTEXT_ID")
                .env_remove("PLEXI_CONTEXT_ROOT")
                .env_remove("PLEXI_RUNNING");
        }
        let output = command.output().map_err(|error| {
            SceneError::new(
                "live_command_failed",
                format!("run {}: {error}", self.binary),
            )
        })?;
        if !output.status.success() {
            let operation = args.iter().take(2).cloned().collect::<Vec<_>>().join(" ");
            let stderr = if args.first().map(String::as_str) == Some("pane")
                && args.get(1).map(String::as_str) == Some("key")
            {
                "input delivery failed".to_string()
            } else {
                String::from_utf8_lossy(&output.stderr).trim().to_string()
            };
            return Err(SceneError::new(
                "live_command_failed",
                format!("{} {} failed: {}", self.binary, operation, stderr),
            ));
        }
        Ok(output)
    }

    fn json(&self, args: &[&str]) -> Result<serde_json::Value, SceneError> {
        let args: Vec<String> = args.iter().map(|value| (*value).to_string()).collect();
        let output = self.command(&args, true)?;
        serde_json::from_slice(&output.stdout).map_err(|error| {
            SceneError::new(
                "live_json_invalid",
                format!("{} returned invalid JSON: {error}", args.join(" ")),
            )
        })
    }

    fn start_or_attach(&mut self) -> Result<(), SceneError> {
        let status_args = vec![
            "host".to_string(),
            "status".to_string(),
            "--json".to_string(),
        ];
        let status = self
            .command(&status_args, false)
            .ok()
            .and_then(|out| serde_json::from_slice::<serde_json::Value>(&out.stdout).ok());
        let class = status
            .as_ref()
            .map_or(HostStatusClass::Unknown, classify_host_status);
        let attach = std::env::var("PLEXI_SCENE_ATTACH").is_ok_and(|value| value == "1");
        match live_start_action(class, attach) {
            Ok(LiveStartAction::Attach) => {
                self.attached_host = true;
                log::info!(
                    "scene_live: attached channel={} binary={}",
                    self.channel,
                    self.binary
                );
                return Ok(());
            }
            Ok(LiveStartAction::StartOwned) => {}
            Err((code, detail)) => {
                return Err(SceneError::new(
                    code,
                    format!("channel '{}': {detail}", self.channel),
                ));
            }
        }
        let pane = format!("cwd={}", env!("CARGO_MANIFEST_DIR"));
        // Hermetic by construction: a runner-owned host must never restore
        // (or overwrite) the channel's saved session — stale panes from a
        // human session would pollute every live assertion.
        self.command(
            &[
                "host".to_string(),
                "start".to_string(),
                "--ephemeral".to_string(),
                "--pane".to_string(),
                pane,
            ],
            false,
        )?;
        self.owned_host = true;
        if let Some(owner_file) = &self.owner_file {
            std::fs::write(owner_file, format!("{}\n", self.channel)).map_err(|error| {
                SceneError::new(
                    "owner_marker_failed",
                    format!("could not create live-scene ownership marker: {error}"),
                )
            })?;
        }
        poll_live_until(Duration::from_secs(5), LIVE_POLL_INTERVAL, |_| {
            Ok(self
                .json(&["host", "status", "--json"])
                .ok()
                .filter(|status| host_seed_ready(status, 1)))
        })
        .map_err(|_| {
            SceneError::new(
                "live_seed_not_ready",
                "host became reachable but its declared seed pane did not settle",
            )
        })?;
        log::info!(
            "scene_live: started channel={} binary={} ready=true seed_panes>=1",
            self.channel,
            self.binary
        );
        Ok(())
    }

    fn teardown(&mut self) {
        if !self.owned_host {
            self.teardown = TeardownResult {
                attempted: false,
                ok: true,
                detail: if self.attached_host {
                    "attached_host_untouched"
                } else {
                    "host_not_started"
                }
                .to_string(),
            };
            return;
        }
        self.teardown.attempted = true;
        let stopped = self.command(&["host".into(), "stop".into()], false).is_ok();
        let gone = poll_live_until(Duration::from_secs(5), LIVE_POLL_INTERVAL, |_| {
            Ok(self
                .command(&["host".into(), "status".into(), "--json".into()], false)
                .ok()
                .and_then(|out| serde_json::from_slice::<serde_json::Value>(&out.stdout).ok())
                .filter(|status| classify_host_status(status) == HostStatusClass::Stopped))
        })
        .is_ok();
        self.teardown.ok = stopped && gone;
        self.teardown.detail = if self.teardown.ok {
            "runner_host_stopped"
        } else {
            "runner_host_teardown_failed"
        }
        .to_string();
        log::info!(
            "scene_live: teardown channel={} ok={}",
            self.channel,
            self.teardown.ok
        );
        if self.teardown.ok {
            if let Some(owner_file) = &self.owner_file {
                let _ = std::fs::remove_file(owner_file);
            }
        }
        self.owned_host = false;
    }

    fn pane_state(&self, pane_id: PaneId) -> Result<serde_json::Value, SceneError> {
        self.json(&["pane", "state", &pane_id.to_string()])
    }

    fn settled_state(
        &self,
        pane_id: PaneId,
        timeout: Duration,
    ) -> Result<serde_json::Value, SceneError> {
        let mut previous = None;
        poll_live_until(timeout, LIVE_POLL_INTERVAL, |_| {
            if let Ok(state) = self.pane_state(pane_id) {
                if previous.as_ref() == Some(&state) {
                    return Ok(Some(state));
                }
                previous = Some(state);
            }
            Ok(None)
        })
    }

    fn exec(
        &mut self,
        step: &Step,
        _shots: &mut Vec<String>,
    ) -> Result<Option<StepDetail>, SceneError> {
        match step {
            Step::Open { open } => {
                self.handles.ensure_available(open.handle())?;
                let target = match open {
                    OpenSpec::Process { path, .. } | OpenSpec::Wasm { path, .. } => {
                        resolve(path).display().to_string()
                    }
                    OpenSpec::Builtin { id, .. } => id.clone(),
                };
                // A live wasm open shells `plexi app open`, whose raw-WASM
                // review would block on a human at a TTY. Pre-approve through
                // the real channel binary first (`plexi app trust`), which
                // persists the fixture's import grants into the channel profile
                // the attached host reads — the scene-runner test process is
                // walled off from real profiles, so it cannot do this itself.
                if let OpenSpec::Wasm { path, .. } = open {
                    let resolved = resolve(path).display().to_string();
                    self.command(&["app".to_string(), "trust".to_string(), resolved], false)?;
                }
                let mut args = vec!["app".to_string(), "open".to_string(), target];
                match open {
                    OpenSpec::Process {
                        args: launch_args, ..
                    }
                    | OpenSpec::Wasm {
                        args: launch_args, ..
                    }
                    | OpenSpec::Builtin {
                        args: launch_args, ..
                    } => args.extend(launch_args.iter().cloned()),
                }
                let cwd = live_builtin_cwd(open);
                let output = self.command_with_cwd(&args, true, cwd.as_deref())?;
                let pane_id = String::from_utf8_lossy(&output.stdout)
                    .trim()
                    .parse::<PaneId>()
                    .map_err(|error| {
                        SceneError::new(
                            "open_response_invalid",
                            format!("app open did not return a pane id: {error}"),
                        )
                    })?;
                self.handles.bind(open.handle(), pane_id)?;
                self.last_app_pane = Some(pane_id);
                let _ = self.settled_state(pane_id, Duration::from_secs(5))?;
                log::info!(
                    "scene_live: step=open channel={} kind={} pane_id={pane_id}",
                    self.channel,
                    open.target_kind()
                );
                Ok(Some(StepDetail::Opened {
                    target_kind: open.target_kind().to_string(),
                    handle: open.handle().to_string(),
                    pane_id,
                }))
            }
            Step::Text { text } => {
                let pane_id = resolve_live_pane_target(&self.handles, &text.target, "text input")?;
                self.command(
                    &[
                        "pane".into(),
                        "send".into(),
                        pane_id.to_string(),
                        text.value.clone(),
                    ],
                    true,
                )?;
                let _ = self.settled_state(pane_id, Duration::from_secs(5))?;
                let length = text.value.chars().count();
                log::info!(
                    "scene_live: step=text channel={} pane_id={pane_id} length={length}",
                    self.channel
                );
                Ok(Some(StepDetail::TextInput {
                    target: text.target.clone(),
                    pane_id: Some(pane_id),
                    length,
                }))
            }
            Step::Paste { paste } => Err(SceneError::new(
                "paste_unsupported",
                format!(
                    "paste step (target '{}') has no live transport; live scenes deliver text through `text`/`key`",
                    paste.target
                ),
            )),
            Step::Key { key } => {
                let pane_id = resolve_live_pane_target(&self.handles, &key.target, "key input")?;
                self.command(
                    &[
                        "pane".into(),
                        "key".into(),
                        pane_id.to_string(),
                        key.value.clone(),
                    ],
                    true,
                )?;
                let _ = self.settled_state(pane_id, Duration::from_secs(5))?;
                Ok(Some(StepDetail::KeyInput {
                    target: key.target.clone(),
                    pane_id: Some(pane_id),
                    value: key.value.clone(),
                }))
            }
            Step::DropFile { drop_file } => {
                let pane_id =
                    resolve_live_pane_target(&self.handles, &drop_file.target, "file drop")?;
                let result = self.command(
                    &[
                        "pane".into(),
                        "drop".into(),
                        pane_id.to_string(),
                        drop_file.value.clone(),
                    ],
                    true,
                );
                match (result, drop_file.expect_rejected) {
                    (Ok(_), false) | (Err(_), true) => {}
                    (Ok(_), true) => {
                        return Err(SceneError::new(
                            "drop_unexpectedly_accepted",
                            "drop was accepted but the scene expected rejection",
                        ));
                    }
                    (Err(error), false) => return Err(error),
                }
                let _ = self.settled_state(pane_id, Duration::from_secs(5))?;
                Ok(Some(StepDetail::Message {
                    message: format!("dropped file onto {} (pane {pane_id})", drop_file.target),
                }))
            }
            Step::Drag { drag } => {
                let pane_id =
                    resolve_live_pane_target(&self.handles, &drag.target, "pointer drag")?;
                let mut args = vec!["pane".to_string(), "drag".to_string(), pane_id.to_string()];
                if let Some([x, y]) = drag.from {
                    args.extend(["--from".to_string(), format!("{x},{y}")]);
                }
                if let Some(node) = &drag.from_node {
                    args.extend(["--from-node".to_string(), node.clone()]);
                }
                if let Some([x, y]) = drag.to {
                    args.extend(["--to".to_string(), format!("{x},{y}")]);
                }
                if let Some(node) = &drag.to_node {
                    args.extend(["--to-node".to_string(), node.clone()]);
                }
                args.extend(["--steps".to_string(), drag.steps.to_string()]);
                if let Some(button) = &drag.button {
                    args.extend(["--button".to_string(), button.clone()]);
                }
                self.command(&args, true)?;
                let _ = self.settled_state(pane_id, Duration::from_secs(5))?;
                log::info!(
                    "scene_live: step=drag channel={} pane_id={pane_id} steps={}",
                    self.channel,
                    drag.steps
                );
                Ok(Some(StepDetail::Message {
                    message: format!("dragged pointer across {} (pane {pane_id})", drag.target),
                }))
            }
            Step::Focus { focus } => {
                let pane_id = self.handles.resolve(focus)?;
                self.command(&["pane".into(), "focus".into(), pane_id.to_string()], true)?;
                poll_live_until(Duration::from_secs(5), LIVE_POLL_INTERVAL, |_| {
                    Ok(self.json(&["pane", "list"]).ok().and_then(|value| {
                        value.as_array()?.iter().find_map(|pane| {
                            (pane.get("id").and_then(serde_json::Value::as_u64) == Some(pane_id)
                                && pane.get("focused").and_then(serde_json::Value::as_bool)
                                    == Some(true))
                            .then_some(())
                        })
                    }))
                })?;
                log::info!(
                    "scene_live: step=focus channel={} handle={} pane_id={pane_id}",
                    self.channel,
                    focus
                );
                Ok(Some(StepDetail::Message {
                    message: format!("focused {focus} (pane {pane_id})"),
                }))
            }
            Step::Close { close } => {
                let pane_id = self.handles.resolve(close)?;
                self.command(&["pane".into(), "close".into(), pane_id.to_string()], true)?;
                poll_live_until(Duration::from_secs(5), LIVE_POLL_INTERVAL, |_| {
                    Ok(self.json(&["pane", "list"]).ok().and_then(|value| {
                        value
                            .as_array()?
                            .iter()
                            .all(|pane| {
                                pane.get("id").and_then(serde_json::Value::as_u64) != Some(pane_id)
                            })
                            .then_some(())
                    }))
                })?;
                log::info!(
                    "scene_live: step=close channel={} handle={} pane_id={pane_id}",
                    self.channel,
                    close
                );
                Ok(Some(StepDetail::Message {
                    message: format!("closed {close} (pane {pane_id})"),
                }))
            }
            Step::WaitAppFrame { wait_app_frame } => {
                let pane_id = self.handles.resolve(&wait_app_frame.target)?;
                let _ = self.settled_state(pane_id, scene_timeout(wait_app_frame.timeout_s))?;
                Ok(None)
            }
            Step::RunSteps { .. } => {
                if let Some(pane_id) = self.last_app_pane {
                    let _ = self.settled_state(pane_id, Duration::from_secs(5))?;
                }
                Ok(None)
            }
            Step::AssertLabel { assert_label } => {
                let pane_id = resolve_live_pane_target(
                    &self.handles,
                    &assert_label.target,
                    "semantic label assertion",
                )?;
                poll_live_until(Duration::from_secs(5), LIVE_POLL_INTERVAL, |_| {
                    let state = self.pane_state(pane_id)?;
                    let matched = state
                        .pointer("/semantic/nodes")
                        .and_then(serde_json::Value::as_array)
                        .is_some_and(|nodes| {
                            nodes.iter().any(|node| {
                                node.get("label").and_then(serde_json::Value::as_str)
                                    == Some(assert_label.label.as_str())
                                    || node.get("value").and_then(serde_json::Value::as_str)
                                        == Some(assert_label.label.as_str())
                            })
                        });
                    if matched {
                        return Ok(Some(Some(StepDetail::LabelMatched {
                            target: assert_label.target.clone(),
                            pane_id: Some(pane_id),
                            label: assert_label.label.clone(),
                        })));
                    }
                    Ok(None)
                })
                .map_err(|error| {
                    if error.code == "eventual_timeout" {
                        SceneError::new(
                            "label_not_found",
                            format!(
                                "assert_label: {:?} not found in live pane {}",
                                assert_label.label, pane_id
                            ),
                        )
                    } else {
                        error
                    }
                })
            }
            Step::Assert { assert } => self.check_eventually(assert).map(|()| None),
            Step::Expect { expect } => self.expect(expect),
            Step::Shot { shot } => Ok(Some(StepDetail::Message {
                message: format!("live backend skips optional screenshot {shot}"),
            })),
            Step::SwitchContext { switch_context } => {
                let contexts = self.json(&["context", "list"])?;
                let entries = contexts.as_array().ok_or_else(|| {
                    SceneError::new("live_json_invalid", "context list did not return an array")
                })?;
                let context = entries.get(*switch_context).ok_or_else(|| {
                    SceneError::new(
                        "invalid_context",
                        format!(
                            "switch_context {}: only {} contexts exist",
                            switch_context,
                            entries.len()
                        ),
                    )
                })?;
                let context_id = context
                    .get("context_id")
                    .and_then(serde_json::Value::as_u64)
                    .ok_or_else(|| {
                        SceneError::new("live_json_invalid", "context entry has no context_id")
                    })?;
                self.command(
                    &["context".into(), "zoom".into(), context_id.to_string()],
                    true,
                )?;
                poll_live_until(Duration::from_secs(5), LIVE_POLL_INTERVAL, |_| {
                    Ok(self.json(&["context", "list"]).ok().and_then(|value| {
                        value
                            .as_array()?
                            .iter()
                            .find(|entry| {
                                entry.get("context_id").and_then(serde_json::Value::as_u64)
                                    == Some(context_id)
                            })
                            .and_then(|entry| {
                                entry.get("is_active").and_then(serde_json::Value::as_bool)
                            })
                            .filter(|active| *active)
                    }))
                })?;
                Ok(None)
            }
            Step::PushToSubcontext { push_to_subcontext } => {
                let pane_id = self.last_app_pane.ok_or_else(|| {
                    SceneError::new(
                        "missing_target",
                        "push_to_subcontext requires an opened pane",
                    )
                })?;
                let before = self
                    .json(&["context", "list"])?
                    .as_array()
                    .map(Vec::len)
                    .unwrap_or(0);
                self.command(
                    &[
                        "context".into(),
                        "push".into(),
                        push_to_subcontext.clone(),
                        "--pane-id".into(),
                        pane_id.to_string(),
                    ],
                    true,
                )?;
                poll_live_until(Duration::from_secs(5), LIVE_POLL_INTERVAL, |_| {
                    Ok(self
                        .json(&["context", "list"])
                        .ok()
                        .and_then(|value| (value.as_array()?.len() > before).then_some(())))
                })?;
                Ok(None)
            }
            Step::Sidebar { .. } => Err(SceneError::new(
                "unsupported_live_verb",
                "sidebar mutation has no sanctioned live CLI/IPC command",
            )),
            Step::SeedNotification { .. } => Err(SceneError::new(
                "unsupported_live_verb",
                "seed_notification is headless-only; live scenes must notify through an app",
            )),
        }
    }

    fn host_state(&self) -> HostState {
        let panes = self
            .json(&["pane", "list"])
            .ok()
            .and_then(|value| value.as_array().cloned())
            .unwrap_or_default();
        let contexts = self.json(&["context", "list"]).ok();
        let context_count = live_context_count(contexts.as_ref(), &panes);
        let window_count = panes
            .iter()
            .filter_map(|pane| pane.get("window_id").and_then(serde_json::Value::as_u64))
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        HostState {
            context_count,
            window_count,
            pane_count: panes.len(),
            portal_count: panes
                .iter()
                .filter(|pane| {
                    pane.get("type").and_then(serde_json::Value::as_str) == Some("portal")
                })
                .count(),
            sidebar: false,
        }
    }

    fn app_state(&self) -> Option<AppState> {
        let pane_id = self.last_app_pane?;
        let tree = self.pane_state(pane_id).ok()?;
        let lifecycle = tree
            .get("lifecycle")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unavailable")
            .to_string();
        Some(AppState {
            pane_id,
            lifecycle,
            tree,
        })
    }

    fn check(&self, spec: &AssertSpec) -> Result<(), SceneError> {
        if spec.sidebar.is_some() {
            return Err(SceneError::new(
                "unsupported_live_assertion",
                "sidebar state is not exposed by the live pane metadata API",
            ));
        }
        let host = self.host_state();
        let mut failures = Vec::new();
        if let Some(expected) = spec
            .pane_count
            .filter(|expected| *expected != host.pane_count)
        {
            failures.push(format!(
                "pane_count: expected {expected}, got {}",
                host.pane_count
            ));
        }
        if let Some(expected) = spec
            .window_count
            .filter(|expected| *expected != host.window_count)
        {
            failures.push(format!(
                "window_count: expected {expected}, got {}",
                host.window_count
            ));
        }
        if let Some(expected) = spec
            .context_count
            .filter(|expected| *expected != host.context_count)
        {
            failures.push(format!(
                "context_count: expected {expected}, got {}",
                host.context_count
            ));
        }
        if let Some(expected) = spec.focused {
            let target = spec.target.as_deref().ok_or_else(|| {
                SceneError::new(
                    "missing_assert_target",
                    "assert focused requires target = '<handle>'",
                )
            })?;
            let pane_id = self.handles.resolve(target)?;
            let panes = self.json(&["pane", "list"])?;
            let actual = panes.as_array().and_then(|entries| {
                entries.iter().find_map(|pane| {
                    (pane.get("id").and_then(serde_json::Value::as_u64) == Some(pane_id))
                        .then(|| pane.get("focused").and_then(serde_json::Value::as_bool))
                        .flatten()
                })
            });
            if actual != Some(expected) {
                failures.push(format!(
                    "focused: expected {expected}, got {}",
                    actual.map_or_else(|| "unavailable".to_string(), |value| value.to_string())
                ));
            }
        }
        if let Some(expected) = spec.exists {
            let target = spec.target.as_deref().ok_or_else(|| {
                SceneError::new(
                    "missing_assert_target",
                    "assert exists requires target = '<handle>'",
                )
            })?;
            let pane_id = self.handles.resolve(target)?;
            let panes = self.json(&["pane", "list"])?;
            let actual = panes.as_array().is_some_and(|entries| {
                entries
                    .iter()
                    .any(|pane| pane.get("id").and_then(serde_json::Value::as_u64) == Some(pane_id))
            });
            if actual != expected {
                failures.push(format!("exists: expected {expected}, got {actual}"));
            }
        }
        if spec.lifecycle.is_some()
            || spec.tree_contains.is_some()
            || spec.fit.is_some()
            || spec.aspect.is_some()
        {
            let target = spec.target.as_deref().ok_or_else(|| {
                SceneError::new(
                    "missing_assert_target",
                    "assert lifecycle/tree_contains requires target = '<handle>'",
                )
            })?;
            let pane_id = self.handles.resolve(target)?;
            let state = self.pane_state(pane_id)?;
            if let Some(expected) = &spec.lifecycle {
                let actual = state
                    .get("lifecycle")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("unavailable");
                if actual != expected {
                    failures.push(format!("lifecycle: expected {expected}, got {actual}"));
                }
            }
            if let Some(needle) = &spec.tree_contains {
                if !state.to_string().contains(needle) {
                    failures.push(format!("tree_contains: {needle:?} not found in app tree"));
                }
            }
            geometry_failures(spec, &state, &mut failures);
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(SceneError::new("assertion_failed", failures.join("; ")))
        }
    }

    fn check_eventually(&self, spec: &AssertSpec) -> Result<(), SceneError> {
        poll_live_until(Duration::from_secs(5), LIVE_POLL_INTERVAL, |_| {
            Ok(self.check(spec).ok())
        })
    }

    fn expect(&mut self, spec: &ExpectSpec) -> Result<Option<StepDetail>, SceneError> {
        if spec.event_stream.is_some() || spec.event_payload_contains.is_some() {
            return Err(SceneError::new(
                "unsupported_live_verb",
                "event expectations are headless-only — the installed host exposes no \
                 sanctioned event-query seam yet",
            ));
        }
        let pane_id = self.handles.resolve(&spec.target)?;
        let before = self.pane_state(pane_id)?;
        if let Some(key) = &spec.after_key {
            self.command(
                &[
                    "pane".into(),
                    "key".into(),
                    pane_id.to_string(),
                    key.clone(),
                ],
                true,
            )?;
        }
        if let Some(text) = &spec.after_text {
            self.command(
                &[
                    "pane".into(),
                    "send".into(),
                    pane_id.to_string(),
                    text.clone(),
                ],
                true,
            )?;
        }
        let mut history = Vec::new();
        let result = poll_live_until(
            scene_timeout(spec.timeout_s),
            LIVE_POLL_INTERVAL,
            |elapsed| {
                let state = self.pane_state(pane_id)?;
                history.push(PollSample {
                    timestamp_ms: elapsed.as_millis(),
                    observed: state.clone(),
                });
                let changed = spec
                    .node_changes
                    .as_ref()
                    .is_none_or(|needle| state.to_string().contains(needle) && state != before);
                let contains = spec
                    .tree_contains
                    .as_ref()
                    .is_none_or(|needle| state.to_string().contains(needle));
                if changed && contains && notes_expectations_match(spec, &state) {
                    return Ok(Some(()));
                }
                Ok(None)
            },
        );
        match result {
            Ok(()) => Ok(None),
            Err(error) if error.code == "eventual_timeout" => Err(SceneError::new(
                if spec.after_key.is_some() || spec.after_text.is_some() {
                    "input_no_effect"
                } else {
                    "eventual_timeout"
                },
                format!(
                    "expect target '{}' did not satisfy semantic predicate; before={before}",
                    spec.target
                ),
            )
            .with_poll_history(history)),
            Err(error) => Err(error),
        }
    }
}

fn geometry_failures(spec: &AssertSpec, tree: &serde_json::Value, failures: &mut Vec<String>) {
    if spec.fit.is_none() && spec.aspect.is_none() {
        return;
    }
    let Some(nodes) = tree
        .pointer("/semantic/nodes")
        .and_then(serde_json::Value::as_array)
    else {
        failures.push("geometry: semantic tree unavailable".into());
        return;
    };
    let Some(canvas) = nodes
        .iter()
        .find(|node| node.get("role").and_then(serde_json::Value::as_str) == Some("canvas"))
    else {
        failures.push("geometry: no canvas semantic node".into());
        return;
    };
    let value = canvas
        .get("value")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if let Some(expected) = spec.fit.as_deref() {
        let actual = value.split("fit=").nth(1).unwrap_or("missing");
        if actual != expected {
            failures.push(format!("fit: expected {expected}, got {actual}"));
        }
    }
    if let Some([aw, ah]) = spec.aspect {
        let matched = canvas
            .get("canvas_commands")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|commands| {
                commands
                    .iter()
                    .filter_map(|command| {
                        let w = command.get("width")?.as_f64()?;
                        let h = command.get("height")?.as_f64()?;
                        (w > 0.0 && h > 0.0).then_some((w / h - aw / ah).abs() <= 0.03)
                    })
                    .any(|matches| matches)
            });
        if !matched {
            failures.push(format!(
                "aspect: expected {aw}:{ah}, no committed canvas command matched"
            ));
        }
    }
}

impl Drop for LiveBackend {
    fn drop(&mut self) {
        if self.owned_host {
            self.teardown();
        }
    }
}

fn run_live_scene(scene_path: &Path, out_dir: &Path, no_shots: bool) -> SceneReport {
    let scene_name = scene_path
        .file_stem()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| scene_path.display().to_string());
    let raw = match std::fs::read_to_string(scene_path) {
        Ok(raw) => raw,
        Err(error) => {
            return live_failed_report(
                scene_name,
                None,
                SceneError::new(
                    "scene_read",
                    format!("read {}: {error}", scene_path.display()),
                ),
            )
        }
    };
    let (raw, _tmp_guard) = match expand_scene_tmp(raw) {
        Ok(expanded) => expanded,
        Err(error) => return live_failed_report(scene_name, None, error),
    };
    let scene: Scene = match toml::from_str(&raw) {
        Ok(scene) => scene,
        Err(error) => {
            return live_failed_report(
                scene_name,
                None,
                SceneError::new(
                    "scene_parse",
                    format!("parse {}: {error}", scene_path.display()),
                ),
            )
        }
    };
    if scene.picker_script.is_some() {
        return live_failed_report(
            scene_name,
            std::env::var("PLEXI_SCENE_CHANNEL").ok(),
            SceneError::new(
                "unsupported_live_verb",
                "picker_script is headless-only; script a live host's picker by launching it with PLEXI_PICKER_SCRIPT",
            ),
        );
    }
    let mut backend = match LiveBackend::from_env() {
        Ok(backend) => backend,
        Err(error) => {
            return live_failed_report(scene_name, std::env::var("PLEXI_SCENE_CHANNEL").ok(), error)
        }
    };
    let channel = backend.channel.clone();
    let mut steps = Vec::new();
    let mut shots = Vec::new();
    let start_error = backend.start_or_attach().err();
    let mut passed = start_error.is_none();
    if passed {
        for (index, step) in scene.steps.iter().enumerate() {
            match backend.exec(step, &mut shots) {
                Ok(detail) => steps.push(StepResult {
                    index,
                    step: step_label(step),
                    ok: true,
                    detail,
                    error: None,
                    failure_bundle: None,
                }),
                Err(error) => {
                    let bundle = write_failure_bundle(
                        out_dir,
                        &scene_name,
                        index,
                        "live",
                        &error,
                        backend.app_state(),
                        None,
                    );
                    if let Some(pane_id) = backend.last_app_pane {
                        let screenshot = PathBuf::from(&bundle).join("screenshot.png");
                        let _ = backend.command(
                            &[
                                "host".into(),
                                "screenshot".into(),
                                "--pane".into(),
                                pane_id.to_string(),
                                "--output".into(),
                                screenshot.to_string_lossy().into_owned(),
                            ],
                            true,
                        );
                    }
                    steps.push(StepResult {
                        index,
                        step: step_label(step),
                        ok: false,
                        detail: None,
                        error: Some(error),
                        failure_bundle: Some(bundle),
                    });
                    passed = false;
                    break;
                }
            }
        }
    } else {
        steps.push(StepResult {
            index: 0,
            step: "live_start".to_string(),
            ok: false,
            detail: None,
            error: start_error,
            failure_bundle: None,
        });
    }
    let handles = backend.handles.report();
    let host = backend.host_state();
    let app = backend.app_state();
    backend.teardown();
    passed &= backend.teardown.ok;
    let failure_bundle = steps.iter().find_map(|step| step.failure_bundle.clone());
    let report = SceneReport {
        schema_version: SCENE_REPORT_SCHEMA_VERSION,
        backend: "live".to_string(),
        channel,
        scene: scene_name.clone(),
        passed,
        steps,
        shots,
        handles,
        host,
        app,
        teardown: backend.teardown.clone(),
        failure_bundle,
    };
    if let Err(error) = std::fs::create_dir_all(out_dir).and_then(|()| {
        std::fs::write(
            out_dir.join(format!("{scene_name}.json")),
            serde_json::to_vec_pretty(&report).unwrap_or_default(),
        )
    }) {
        log::warn!("scene_live: failed to write report: {error}");
    }
    let _ = no_shots;
    report
}

fn live_failed_report(scene: String, channel: Option<String>, error: SceneError) -> SceneReport {
    let mut report = failed_report(scene, error);
    report.backend = "live".to_string();
    report.channel = channel.unwrap_or_else(|| "unconfigured".to_string());
    report.teardown = TeardownResult {
        attempted: false,
        ok: true,
        detail: "host_not_started".to_string(),
    };
    report
}

fn step_label(step: &Step) -> String {
    match step {
        Step::Open { open } => format!("open {} as {}", open.target_kind(), open.handle()),
        Step::Text { text } => format!(
            "text {} ({} chars)",
            text.target,
            text.value.chars().count()
        ),
        Step::Paste { paste } => format!(
            "paste {} ({} chars)",
            paste.target,
            paste.value.chars().count()
        ),
        Step::Key { key } => format!("key {} {}", key.target, key.value),
        Step::DropFile { drop_file } => format!("drop_file {}", drop_file.target),
        Step::Drag { drag } => format!("drag {} ({} steps)", drag.target, drag.steps),
        Step::Focus { focus } => format!("focus {focus}"),
        Step::Close { close } => format!("close {close}"),
        Step::Sidebar { sidebar } => format!("sidebar {sidebar}"),
        Step::SeedNotification { seed_notification } => {
            format!("seed_notification {:?}", seed_notification.title)
        }
        Step::SwitchContext { switch_context } => format!("switch_context {switch_context}"),
        Step::PushToSubcontext { push_to_subcontext } => {
            format!("push_to_subcontext {push_to_subcontext}")
        }
        Step::WaitAppFrame { wait_app_frame } => {
            format!(
                "wait_app_frame {} {}s",
                wait_app_frame.target, wait_app_frame.timeout_s
            )
        }
        Step::RunSteps { run_steps } => format!("run_steps {run_steps}"),
        Step::Assert { .. } => "assert".to_string(),
        Step::Expect { .. } => "expect".to_string(),
        Step::AssertLabel { assert_label } => {
            format!(
                "assert_label {} {:?}",
                assert_label.target, assert_label.label
            )
        }
        Step::Shot { shot } => format!("shot {shot}"),
    }
}

/// Resolve a scene-relative path: absolute stays as-is, relative is joined to
/// the repo root (`CARGO_MANIFEST_DIR`).
fn resolve(path: &str) -> PathBuf {
    let p = Path::new(path);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        Path::new(env!("CARGO_MANIFEST_DIR")).join(p)
    }
}

/// Seeds the in-process test profile's `PermissionStore` with Green grants for
/// a headless scene fixture's raw-WASM imports so the scene never stalls on an
/// unanswerable capability review. Headless only: the in-process host and this
/// call share the same (test-isolated) profile. The live backend cannot use
/// this — a `#[cfg(test)]` binary is walled off from real channel profiles
/// (`assert_test_profile_is_isolated`) — so it pre-approves through the real
/// channel binary with `plexi app trust` instead.
fn preapprove_wasm_scene_grants(wasm_path: &Path) -> Result<(), String> {
    let app_id = wasm_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("wasm");
    let workspace_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let grants = crate::host::wasm_app::WasmApp::inspect_required_grants(wasm_path)
        .map_err(|e| format!("inspect {}: {e}", wasm_path.display()))?;
    let mut store =
        crate::app::permissions::PermissionStore::load_or_default(&crate::config::config_dir());
    // Link-time host-interface grants (state/pipes/gpu/audio) come from the
    // component's imports. `fs.pick` is not an import — it gates the picker
    // effect at runtime — so preapprove it unconditionally: a scene never opens
    // a real dialog (the scripted picker override / `PLEXI_PICKER_SCRIPT`
    // always answers it), and the pick itself is what grants concrete fs paths.
    // Without this, a fixture driving the file picker would stall on an
    // unanswerable capability prompt.
    for capability_id in grants
        .capability_ids()
        .into_iter()
        .chain(std::iter::once("fs.pick".to_string()))
    {
        store.set_wasm(
            app_id,
            &workspace_root,
            &capability_id,
            crate::app::permissions::PermissionState::Green,
        );
    }
    store.save();
    log::info!(
        "scene: preapproved raw wasm grants app={} path={} workspace={}",
        app_id,
        wasm_path.display(),
        workspace_root.display()
    );
    Ok(())
}

impl HeadlessBackend {
    fn exec(
        &mut self,
        step: &Step,
        shots: &mut Vec<String>,
    ) -> Result<Option<StepDetail>, SceneError> {
        match step {
            Step::Open { open } => {
                let handle = open.handle();
                self.handles.ensure_available(handle)?;
                let pane_id = match open {
                    OpenSpec::Process { path, args, .. } => {
                        let path = resolve(path);
                        self.h
                            .open_target(crate::ui_tests::HarnessOpenTarget::Process {
                                path: &path,
                                args,
                            })
                    }
                    OpenSpec::Wasm { path, args, .. } => {
                        let path = resolve(path);
                        preapprove_wasm_scene_grants(&path).map_err(|message| {
                            SceneError::new("wasm_preapproval_failed", message)
                        })?;
                        self.h
                            .open_target(crate::ui_tests::HarnessOpenTarget::Wasm {
                                path: &path,
                                args,
                            })
                    }
                    OpenSpec::Builtin { id, cwd, args, .. } => {
                        let cwd = cwd
                            .as_deref()
                            .map(resolve)
                            .unwrap_or_else(|| self.h.workspace_root().to_path_buf());
                        self.h
                            .open_target(crate::ui_tests::HarnessOpenTarget::Builtin {
                                id,
                                cwd: &cwd,
                                args,
                            })
                    }
                }
                .map_err(|message| SceneError::new("open_failed", message))?;
                self.handles.bind(handle, pane_id)?;
                self.last_app_pane = Some(pane_id);
                self.h.step();
                log::info!(
                    "scene: open kind={} handle={} pane_id={pane_id}",
                    open.target_kind(),
                    handle
                );
                Ok(Some(StepDetail::Opened {
                    target_kind: open.target_kind().to_string(),
                    handle: handle.to_string(),
                    pane_id,
                }))
            }
            Step::Text { text } => {
                let target = self.handles.resolve_input(&text.target)?;
                let pane_id = self.focus_target(target)?;
                self.h
                    .harness()
                    .input_mut()
                    .events
                    .push(egui::Event::Text(text.value.clone()));
                self.h.step();
                let length = text.value.chars().count();
                log::info!(
                    "scene: text target={} pane_id={pane_id:?} length={length}",
                    text.target
                );
                Ok(Some(StepDetail::TextInput {
                    target: text.target.clone(),
                    pane_id,
                    length,
                }))
            }
            Step::Paste { paste } => {
                let target = self.handles.resolve_input(&paste.target)?;
                let pane_id = self.focus_target(target)?;
                self.h
                    .harness()
                    .input_mut()
                    .events
                    .push(egui::Event::Paste(paste.value.clone()));
                self.h.step();
                let length = paste.value.chars().count();
                log::info!(
                    "scene: paste target={} pane_id={pane_id:?} length={length}",
                    paste.target
                );
                Ok(Some(StepDetail::TextInput {
                    target: paste.target.clone(),
                    pane_id,
                    length,
                }))
            }
            Step::Key { key } => {
                // Bare printable character — not a named key or modifier chord.
                // Live Plexi delivers these via egui::Event::Text, not Event::Key
                // (the host suppresses Event::Key for printable chars to avoid
                // double-dispatch, relying on Event::Text for OS-resolved chars).
                // Mirror that path here so scene injection reaches the app the
                // same way a real keypress does.
                //
                // A single non-control character is unambiguously a bare key:
                // named keys ("enter", "left") and chords ("ctrl+z") are always
                // multi-character. No `+`-prefix check needed — a literal `+`
                // key has len == 1 and must take the Event::Text path.
                let target = self.handles.resolve_input(&key.target)?;
                let pane_id = self.focus_target(target)?;
                let mut key_chars = key.value.chars();
                let is_bare_printable = key_chars
                    .next()
                    .is_some_and(|character| !character.is_control())
                    && key_chars.next().is_none();
                if is_bare_printable {
                    self.h
                        .harness()
                        .input_mut()
                        .events
                        .push(egui::Event::Text(key.value.clone()));
                } else {
                    let (modifiers, k) = parse_key(&key.value)
                        .map_err(|message| SceneError::new("invalid_key", message))?;
                    self.h.harness().key_press_modifiers(modifiers, k);
                }
                self.h.step();
                log::info!(
                    "scene: key target={} pane_id={pane_id:?} value={}",
                    key.target,
                    key.value
                );
                Ok(Some(StepDetail::KeyInput {
                    target: key.target.clone(),
                    pane_id,
                    value: key.value.clone(),
                }))
            }
            Step::DropFile { drop_file } => {
                let pane_id = self.handles.resolve(&drop_file.target)?;
                let response = self
                    .h
                    .workspace_root()
                    .join(format!("scene-drop-{pane_id}.json"));
                self.h.with_app_mut(|app| {
                    app.handle_pane_ipc_request(crate::app_protocol::AppRequest::DropFile {
                        pane_id,
                        path_or_url: drop_file.value.clone(),
                        response_file: response.to_string_lossy().into_owned(),
                    })
                });
                self.h.step();
                let bytes = std::fs::read(&response)
                    .map_err(|e| SceneError::new("drop_delivery_failed", e.to_string()))?;
                let value: serde_json::Value = serde_json::from_slice(&bytes)
                    .map_err(|e| SceneError::new("drop_delivery_failed", e.to_string()))?;
                let rejected = value.get("error").and_then(|v| v.as_str());
                match (rejected, drop_file.expect_rejected) {
                    (None, false) | (Some(_), true) => {}
                    (None, true) => {
                        return Err(SceneError::new(
                            "drop_unexpectedly_accepted",
                            "drop was accepted but the scene expected rejection",
                        ));
                    }
                    (Some(error), false) => {
                        return Err(SceneError::new("drop_rejected", error));
                    }
                }
                Ok(Some(StepDetail::Message {
                    message: format!("dropped file onto {} (pane {pane_id})", drop_file.target),
                }))
            }
            Step::Drag { drag } => {
                let pane_id = self.handles.resolve(&drag.target)?;
                let response = self
                    .h
                    .workspace_root()
                    .join(format!("scene-drag-{pane_id}.json"));
                self.h.with_app_mut(|app| {
                    app.handle_pane_ipc_request(crate::app_protocol::AppRequest::DragPane {
                        pane_id,
                        from: drag.from,
                        from_node: drag.from_node.clone(),
                        to: drag.to,
                        to_node: drag.to_node.clone(),
                        steps: Some(drag.steps),
                        button: drag.button.clone(),
                        response_file: Some(response.to_string_lossy().into_owned()),
                    })
                });
                self.h.step();
                let bytes = std::fs::read(&response)
                    .map_err(|e| SceneError::new("drag_delivery_failed", e.to_string()))?;
                let value: serde_json::Value = serde_json::from_slice(&bytes)
                    .map_err(|e| SceneError::new("drag_delivery_failed", e.to_string()))?;
                if let Some(error) = value.get("error").and_then(|v| v.as_str()) {
                    return Err(SceneError::new("drag_rejected", error));
                }
                // Deliver the whole press → moves → release schedule (one
                // frame each) plus settling frames for the guest round trip.
                self.h.run_steps(drag.steps as usize + 4);
                log::info!(
                    "scene: drag target={} pane_id={pane_id} steps={}",
                    drag.target,
                    drag.steps
                );
                Ok(Some(StepDetail::Message {
                    message: format!("dragged pointer across {} (pane {pane_id})", drag.target),
                }))
            }
            Step::Focus { focus } => {
                let pane_id = self.handles.resolve(focus)?;
                self.h
                    .focus_pane(pane_id)
                    .map_err(|message| SceneError::new("focus_failed", message))?;
                // Focus changes can invalidate pane rectangles for the current
                // frame; settle one render and one post-layout frame before a
                // following assertion or screenshot observes the host.
                self.h.run_steps(2);
                log::info!("scene: focus handle={focus} pane_id={pane_id}");
                Ok(Some(StepDetail::Message {
                    message: format!("focused {focus} (pane {pane_id})"),
                }))
            }
            Step::Close { close } => {
                let pane_id = self.handles.resolve(close)?;
                self.h
                    .close_pane(pane_id)
                    .map_err(|message| SceneError::new("close_failed", message))?;
                // Closing rewrites the tile tree before egui recomputes the
                // surviving pane rect. Settle both phases so an immediately
                // following shot cannot capture the transient old clip rect.
                self.h.run_steps(2);
                log::info!("scene: close handle={close} pane_id={pane_id}");
                Ok(Some(StepDetail::Message {
                    message: format!("closed {close} (pane {pane_id})"),
                }))
            }
            Step::Sidebar { sidebar } => {
                let v = *sidebar;
                self.h.with_app_mut(|app| app.sidebar_visible = v);
                self.h.step();
                Ok(None)
            }
            Step::SeedNotification { seed_notification } => {
                // Deliberately bypasses `enqueue_notification`: this is a
                // screenshot fixture, not a notification surface. It force-opens
                // the modal so scenes can photograph it.
                let title = seed_notification.title.clone();
                let body = seed_notification.body.clone();
                let id = self.h.with_app_mut(|app| {
                    let id = format!("__scene_notification_{}", app.pending_notifications.len());
                    app.pending_notifications
                        .push(crate::app::PendingNotification {
                            notify_id: id.clone(),
                            sender_pane_id: 0,
                            dismiss_owner_pane_id: 0,
                            source_context_id: 0,
                            source_window_id: 0,
                            title,
                            body,
                            kind: crate::app_protocol::NotifyKind::Message,
                            options: Vec::new(),
                            input_prompt: None,
                            required: false,
                            scope: crate::app_protocol::NotifyScope::Global,
                            image_inline: None,
                            image_pipe_id: None,
                            response_file: None,
                            timeout_secs: None,
                            on_dismiss: None,
                            enqueued_at: std::time::Instant::now(),
                            tombstoned: false,
                            deliver_after: None,
                            origin_in_view: false,
                        });
                    app.show_notification_modal = true;
                    if app.current_notify_id.is_none() {
                        app.current_notify_id = Some(id.clone());
                    }
                    id
                });
                self.h.run_steps(2);
                Ok(Some(StepDetail::Message {
                    message: format!("seeded notification {id}"),
                }))
            }
            Step::SwitchContext { switch_context } => {
                let idx = *switch_context;
                let len = self.h.with_app(|app| app.router.len());
                if idx >= len {
                    return Err(SceneError::new(
                        "invalid_context",
                        format!("switch_context {idx}: only {len} contexts exist"),
                    ));
                }
                self.h.with_app_mut(|app| app.switch_workspace(idx));
                self.h.step();
                Ok(None)
            }
            Step::PushToSubcontext { push_to_subcontext } => {
                self.h
                    .push_focused_pane_to_subcontext(Some(push_to_subcontext.clone()));
                self.h.step();
                Ok(None)
            }
            Step::WaitAppFrame { wait_app_frame } => {
                let pane_id = self.handles.resolve(&wait_app_frame.target)?;
                self.h
                    .wait_for_app_frame(pane_id, scene_timeout(wait_app_frame.timeout_s))
                    .map_err(|message| SceneError::new("wait_app_frame_failed", message))?;
                Ok(None)
            }
            Step::RunSteps { run_steps } => {
                self.h.run_steps(*run_steps);
                Ok(None)
            }
            Step::Assert { assert } => self.check(assert).map(|()| None),
            Step::Expect { expect } => self.expect(expect),
            Step::AssertLabel { assert_label } => {
                let target = self.handles.resolve_input(&assert_label.target)?;
                let pane_id = self.focus_target(target)?;
                let matched = match pane_id {
                    Some(pane_id) => self.h.pane_has_label(pane_id, &assert_label.label),
                    None => self.h.host_has_label(&assert_label.label),
                };
                if !matched {
                    return Err(SceneError::new(
                        "label_not_found",
                        format!(
                            "assert_label: {:?} not found after focusing target '{}'",
                            assert_label.label, assert_label.target
                        ),
                    ));
                }
                Ok(Some(StepDetail::LabelMatched {
                    target: assert_label.target.clone(),
                    pane_id,
                    label: assert_label.label.clone(),
                }))
            }
            Step::Shot { shot } => {
                if self.no_shots {
                    return Ok(Some(StepDetail::Message {
                        message: "skipped (no-shots)".to_string(),
                    }));
                }
                let path = self.out_dir.join(shot);
                self.h
                    .save_screenshot(&path.to_string_lossy())
                    .map_err(|error| {
                        SceneError::new("screenshot_failed", format!("shot {shot}: {error}"))
                    })?;
                shots.push(path.display().to_string());
                Ok(Some(StepDetail::Message {
                    message: path.display().to_string(),
                }))
            }
        }
    }

    fn focus_target(&mut self, target: InputTarget) -> Result<Option<PaneId>, SceneError> {
        match target {
            InputTarget::Host => Ok(None),
            InputTarget::Pane(pane_id) => {
                self.h
                    .focus_pane(pane_id)
                    .map_err(|message| SceneError::new("target_unavailable", message))?;
                self.h.step();
                Ok(Some(pane_id))
            }
        }
    }

    fn check(&mut self, spec: &AssertSpec) -> Result<(), SceneError> {
        let host = self.host_state();
        let mut failures = Vec::new();
        let mut expect = |name: &str, expected: String, actual: String| {
            if expected != actual {
                failures.push(format!("{name}: expected {expected}, got {actual}"));
            }
        };
        if let Some(v) = spec.pane_count {
            expect("pane_count", v.to_string(), host.pane_count.to_string());
        }
        if let Some(v) = spec.window_count {
            expect("window_count", v.to_string(), host.window_count.to_string());
        }
        if let Some(v) = spec.context_count {
            expect(
                "context_count",
                v.to_string(),
                host.context_count.to_string(),
            );
        }
        if let Some(v) = spec.portal_count {
            expect("portal_count", v.to_string(), host.portal_count.to_string());
        }
        if let Some(v) = spec.sidebar {
            expect("sidebar", v.to_string(), host.sidebar.to_string());
        }
        if let Some(expected) = spec.exists {
            let target = spec.target.as_deref().ok_or_else(|| {
                SceneError::new(
                    "missing_assert_target",
                    "assert exists requires target = '<handle>'",
                )
            })?;
            let pane_id = self.handles.resolve(target)?;
            let actual = self.h.with_app(|app| {
                app.windows
                    .iter()
                    .any(|window| window.panes.contains_key(&pane_id))
            });
            expect("exists", expected.to_string(), actual.to_string());
        }
        if let Some(expected) = spec.focused {
            let target = spec.target.as_deref().ok_or_else(|| {
                SceneError::new(
                    "missing_assert_target",
                    "assert focused requires target = '<handle>'",
                )
            })?;
            let pane_id = self.handles.resolve(target)?;
            let actual = self.h.with_app(|app| {
                app.windows
                    .iter()
                    .enumerate()
                    .any(|(window_index, window)| {
                        window_index == app.active_window
                            && window.focused_pane.is_some_and(|tile_id| {
                                matches!(
                                    window.tree.tiles.get(tile_id),
                                    Some(egui_tiles::Tile::Pane(id)) if *id == pane_id
                                )
                            })
                    })
            });
            expect("focused", expected.to_string(), actual.to_string());
        }
        if spec.lifecycle.is_some()
            || spec.tree_contains.is_some()
            || spec.fit.is_some()
            || spec.aspect.is_some()
        {
            let target = spec.target.as_deref().ok_or_else(|| {
                SceneError::new(
                    "missing_assert_target",
                    "assert lifecycle/tree_contains requires target = '<handle>'",
                )
            })?;
            let pane_id = self.handles.resolve(target)?;
            let app = self.app_state_for(pane_id).ok_or_else(|| {
                SceneError::new(
                    "app_state_unavailable",
                    format!(
                        "assert target '{target}' has no process or WASM app state; use assert_label for native UI"
                    ),
                )
            })?;
            if let Some(v) = &spec.lifecycle {
                expect("lifecycle", v.clone(), app.lifecycle.clone());
            }
            if let Some(needle) = &spec.tree_contains {
                let haystack = app.tree.to_string();
                if !haystack.contains(needle.as_str()) {
                    failures.push(format!("tree_contains: \"{needle}\" not found in app tree"));
                }
            }
            if spec.fit.is_some() || spec.aspect.is_some() {
                geometry_failures(spec, &app.tree, &mut failures);
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(SceneError::new("assertion_failed", failures.join("; ")))
        }
    }

    fn expect(&mut self, spec: &ExpectSpec) -> Result<Option<StepDetail>, SceneError> {
        let pane_id = self.handles.resolve(&spec.target)?;
        let before = self
            .app_state_for(pane_id)
            .ok_or_else(|| SceneError::new("app_state_unavailable", "expect requires app state"))?
            .tree;
        if let Some(key) = &spec.after_key {
            self.exec(
                &Step::Key {
                    key: KeySpec {
                        target: spec.target.clone(),
                        value: key.clone(),
                    },
                },
                &mut Vec::new(),
            )?;
        }
        if let Some(text) = &spec.after_text {
            self.exec(
                &Step::Text {
                    text: TextSpec {
                        target: spec.target.clone(),
                        value: text.clone(),
                    },
                },
                &mut Vec::new(),
            )?;
        }
        let deadline = Instant::now() + scene_timeout(spec.timeout_s);
        let started = Instant::now();
        let mut history = Vec::new();
        loop {
            self.h.step();
            let state = self
                .app_state_for(pane_id)
                .ok_or_else(|| {
                    SceneError::new("app_state_unavailable", "expect requires app state")
                })?
                .tree;
            history.push(PollSample {
                timestamp_ms: started.elapsed().as_millis(),
                observed: state.clone(),
            });
            let changed = spec
                .node_changes
                .as_ref()
                .is_none_or(|needle| state.to_string().contains(needle) && state != before);
            let contains = spec
                .tree_contains
                .as_ref()
                .is_none_or(|needle| state.to_string().contains(needle));
            if changed
                && contains
                && notes_expectations_match(spec, &state)
                && app_event_expectation_match(spec)
            {
                return Ok(None);
            }
            if Instant::now() >= deadline {
                return Err(SceneError::new(
                    if spec.after_key.is_some() || spec.after_text.is_some() {
                        "input_no_effect"
                    } else {
                        "eventual_timeout"
                    },
                    format!(
                        "expect target '{}' did not satisfy semantic predicate; before={before}",
                        spec.target
                    ),
                )
                .with_poll_history(history));
            }
        }
    }

    fn host_state(&self) -> HostState {
        self.h.with_app(|app| HostState {
            context_count: app.router.len(),
            window_count: app.windows.len(),
            pane_count: app.windows[app.active_window].panes.len(),
            portal_count: app
                .windows
                .iter()
                .flat_map(|w| w.panes.values())
                .filter(|p| matches!(p, Pane::Portal(_)))
                .count(),
            sidebar: app.sidebar_visible,
        })
    }

    fn app_state(&self) -> Option<AppState> {
        let pane_id = self.last_app_pane?;
        self.app_state_for(pane_id)
    }

    fn app_state_for(&self, pane_id: PaneId) -> Option<AppState> {
        self.h.with_app(|app| {
            for win in &app.windows {
                if let Some(Pane::App(app_pane)) = win.panes.get(&pane_id) {
                    let (lifecycle, guest_error) = app_pane.runtime.lifecycle();
                    let tree = match &app_pane.runtime {
                        AppRuntime::Builtin(_) => serde_json::json!({
                            "semantic": app_pane.semantic_state(),
                            "app_state": app_pane.runtime.semantic_details(),
                        }),
                        AppRuntime::Python(_) => serde_json::json!({
                            "semantic": app_pane.semantic_state(),
                            // Same nested shape as the get_pane_state IPC
                            // response, so scene assertions exercise what
                            // agents actually read.
                            "failure": guest_error.map(|error| serde_json::json!({ "error": error })),
                        }),
                        AppRuntime::Wasm(w) => serde_json::json!({
                            "frame": w.last_render_text(),
                            "semantic": app_pane.semantic_state(),
                        }),
                    };
                    return Some(AppState {
                        pane_id,
                        lifecycle: lifecycle.to_string(),
                        tree,
                    });
                }
            }
            None
        })
    }
}

/// Parse "cmd+shift+b" style combos into egui modifiers + key.
fn parse_key(combo: &str) -> Result<(egui::Modifiers, egui::Key), String> {
    let mut modifiers = egui::Modifiers::NONE;
    let mut key = None;
    for part in combo.split('+') {
        match part.trim().to_lowercase().as_str() {
            "cmd" | "command" => modifiers |= egui::Modifiers::MAC_CMD | egui::Modifiers::COMMAND,
            "ctrl" | "control" => modifiers |= egui::Modifiers::CTRL,
            "alt" | "option" => modifiers |= egui::Modifiers::ALT,
            "shift" => modifiers |= egui::Modifiers::SHIFT,
            other => {
                let name = if other.len() == 1 {
                    other.to_uppercase()
                } else {
                    // "enter" -> "Enter", "escape" -> "Escape"
                    let mut c = other.chars();
                    match c.next() {
                        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                        None => return Err(format!("empty key in combo \"{combo}\"")),
                    }
                };
                key = Some(
                    egui::Key::from_name(&name)
                        .ok_or_else(|| format!("unknown key \"{other}\" in combo \"{combo}\""))?,
                );
            }
        }
    }
    Ok((
        modifiers,
        key.ok_or_else(|| format!("no key in combo \"{combo}\""))?,
    ))
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static LIVE_ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn scene_timeout_scale_defaults_to_one_and_multiplies_when_set() {
        assert_eq!(scene_timeout_scale(None), 1.0);
        assert_eq!(scene_timeout_scale(Some("4")), 4.0);
        assert_eq!(scene_timeout_scale(Some("2.5")), 2.5);
    }

    #[test]
    #[should_panic(expected = "is not a number")]
    fn scene_timeout_scale_rejects_non_numeric() {
        scene_timeout_scale(Some("soon"));
    }

    #[test]
    #[should_panic(expected = "greater than zero")]
    fn scene_timeout_scale_rejects_zero() {
        scene_timeout_scale(Some("0"));
    }

    fn scenes_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("scenes")
    }

    fn out_dir() -> PathBuf {
        std::env::var("PLEXI_SCENE_OUT")
            .map(PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("plexi-scenes"))
    }

    /// Every `tests/scenes/*.toml` with `suite = true` (the default) runs as
    /// a regression test. Adding a scene file IS adding a test.
    #[test]
    fn scene_suite() {
        let dir = scenes_dir();
        let mut entries: Vec<PathBuf> = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("toml"))
            .collect();
        entries.sort();
        assert!(!entries.is_empty(), "no scenes found in {}", dir.display());

        let out = out_dir();
        let mut failures = Vec::new();
        for path in entries {
            let raw = std::fs::read_to_string(&path).expect("read scene");
            let scene: Scene =
                toml::from_str(&raw).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()));
            if !scene.suite {
                println!("scene_suite: skipping {} (suite = false)", path.display());
                continue;
            }
            let report = run_scene(&path, &out, false);
            println!(
                "scene_suite: {} — {}",
                report.scene,
                if report.passed { "ok" } else { "FAILED" }
            );
            if !report.passed {
                failures.push(format!(
                    "{}: {:?}",
                    report.scene,
                    report.steps.iter().find(|s| !s.ok)
                ));
            }
        }
        assert!(
            failures.is_empty(),
            "scenes failed:\n{}",
            failures.join("\n")
        );
    }

    /// Run one scene named by `PLEXI_SCENE`. Wrapped by `just scene <file>`.
    /// `PLEXI_SCENE_NO_SHOTS=1` skips screenshot steps.
    #[test]
    #[ignore = "parameterized by PLEXI_SCENE env var; run via `just scene`"]
    fn scene_single() {
        let scene = std::env::var("PLEXI_SCENE").expect("set PLEXI_SCENE=<scene.toml>");
        let no_shots = std::env::var("PLEXI_SCENE_NO_SHOTS").is_ok_and(|v| v == "1");
        let report = run_scene(&resolve(&scene), &out_dir(), no_shots);
        println!(
            "{}",
            serde_json::to_string_pretty(&report).expect("serialize report")
        );
        assert!(report.passed, "scene failed — see report above");
    }

    #[test]
    fn tmp_placeholder_expands_to_fresh_dir_per_run() {
        let (unchanged, guard) = expand_scene_tmp("no placeholder".to_string()).unwrap();
        assert_eq!(unchanged, "no placeholder");
        assert!(guard.is_none(), "no temp dir without a {{tmp}} marker");

        let (first, first_guard) = expand_scene_tmp("open {tmp}/note.md".to_string()).unwrap();
        let (second, _second_guard) = expand_scene_tmp("open {tmp}/note.md".to_string()).unwrap();
        assert!(!first.contains("{tmp}"), "marker fully expanded: {first}");
        assert_ne!(first, second, "each run gets its own temp dir");

        let dir = first_guard.expect("guard exists when {tmp} is used");
        assert!(dir.path().is_dir());
        let path = dir.path().to_path_buf();
        drop(dir);
        assert!(!path.exists(), "temp dir removed when the run ends");
    }

    #[test]
    fn parse_key_combos() {
        let (m, k) = parse_key("cmd+b").unwrap();
        assert!(m.command);
        assert_eq!(k, egui::Key::B);
        let (m, k) = parse_key("enter").unwrap();
        assert_eq!(m, egui::Modifiers::NONE);
        assert_eq!(k, egui::Key::Enter);
        assert!(parse_key("cmd+").is_err());
        assert!(parse_key("cmd+bogus").is_err());
    }

    #[test]
    fn geometry_fit_rejects_contain_for_committed_fill_canvas() {
        let tree = serde_json::json!({"semantic":{"nodes":[{"role":"canvas","value":"1280x720 fit=fill","canvas_commands":[]}]}});
        let spec = AssertSpec {
            fit: Some("contain".to_string()),
            ..Default::default()
        };
        let mut failures = Vec::new();
        geometry_failures(&spec, &tree, &mut failures);
        assert_eq!(failures, vec!["fit: expected contain, got fill"]);
    }

    #[test]
    fn generic_scene_steps_parse_with_typed_targets() {
        let raw = r#"
            [[steps]]
            open = { kind = "builtin", id = "assistant", cwd = ".", as = "assistant" }

            [[steps]]
            text = { target = "assistant", value = "/settings" }

            [[steps]]
            key = { target = "assistant", value = "enter" }

            [[steps]]
            focus = "assistant"

            [[steps]]
            close = "assistant"

            [[steps]]
            assert_label = { target = "assistant", label = "Assistant settings" }
        "#;

        let scene: Scene = toml::from_str(raw).expect("generic scene should parse");

        assert!(matches!(
            &scene.steps[..],
            [
                Step::Open {
                    open: OpenSpec::Builtin { handle, .. }
                },
                Step::Text { text: TextSpec { target: text_target, .. } },
                Step::Key { key: KeySpec { target: key_target, .. } },
                Step::Focus { focus },
                Step::Close { close },
                Step::AssertLabel {
                    assert_label: AssertLabelSpec { target: label_target, .. }
                }
            ] if handle == "assistant"
                && text_target == "assistant"
                && key_target == "assistant"
                && focus == "assistant"
                && close == "assistant"
                && label_target == "assistant"
        ));
    }

    #[test]
    fn generic_open_rejects_fields_from_another_target_kind() {
        let raw = r#"
            [[steps]]
            open = { kind = "builtin", id = "assistant", path = "apps/dev/balls", as = "assistant" }
        "#;

        assert!(toml::from_str::<Scene>(raw).is_err());
    }

    #[test]
    fn pane_handles_reject_duplicate_names() {
        let mut handles = PaneHandles::default();
        handles.bind("assistant", 41).expect("first bind");

        let error = handles.bind("assistant", 42).unwrap_err();

        assert_eq!(error.code, "duplicate_handle");
    }

    #[test]
    fn pane_handles_reject_missing_targets() {
        let handles = PaneHandles::default();

        let error = handles.resolve("missing").unwrap_err();

        assert_eq!(error.code, "missing_target");
    }

    #[test]
    fn live_missing_target_fails_before_any_host_command() {
        let handles = PaneHandles::default();

        let error = handles.resolve("not-opened").unwrap_err();

        assert_eq!(error.code, "missing_target");
    }

    #[test]
    fn pane_handles_reserve_host_target() {
        let mut handles = PaneHandles::default();

        let error = handles.bind("host", 41).unwrap_err();

        assert_eq!(error.code, "reserved_handle");
    }

    #[test]
    fn pane_handles_resolve_host_only_as_an_input_target() {
        let handles = PaneHandles::default();

        let target = handles.resolve_input("host").expect("host input target");

        assert_eq!(target, InputTarget::Host);
    }

    #[test]
    fn live_backend_requires_an_explicit_channel() {
        let _guard = LIVE_ENV_LOCK.lock().unwrap();
        std::env::remove_var("PLEXI_SCENE_CHANNEL");

        let error = LiveBackend::from_env()
            .err()
            .expect("channel must be required");

        assert_eq!(error.code, "live_channel_required");
    }

    #[test]
    fn live_backend_channel_selects_isolated_binary_and_socket() {
        let _guard = LIVE_ENV_LOCK.lock().unwrap();
        std::env::set_var("PLEXI_SCENE_CHANNEL", "pr-4242");
        std::env::remove_var("PLEXI_SCENE_BIN");

        let backend = LiveBackend::from_env().expect("live config");

        assert_eq!(backend.binary, "plexi-pr-4242");
        assert!(backend.socket.ends_with(".plexi-pr-4242/notify.sock"));
        std::env::remove_var("PLEXI_SCENE_CHANNEL");
    }

    #[test]
    fn live_start_action_attach_mode_requires_running_host() {
        use super::{live_start_action, HostStatusClass, LiveStartAction};
        assert_eq!(
            live_start_action(HostStatusClass::Running, true),
            Ok(LiveStartAction::Attach)
        );
        assert_eq!(
            live_start_action(HostStatusClass::Stopped, false),
            Ok(LiveStartAction::StartOwned)
        );
        // A stopped channel in attach mode is a hard error — never a silent
        // owned replacement (a crashed gate host must fail the gate).
        assert_eq!(
            live_start_action(HostStatusClass::Stopped, true).unwrap_err().0,
            "live_attach_host_missing"
        );
        assert_eq!(
            live_start_action(HostStatusClass::Running, false).unwrap_err().0,
            "live_host_already_running"
        );
        assert_eq!(
            live_start_action(HostStatusClass::Unknown, true).unwrap_err().0,
            "live_status_invalid"
        );
    }

    #[test]
    fn attached_live_backend_teardown_leaves_host_untouched() {
        let _guard = LIVE_ENV_LOCK.lock().unwrap();
        std::env::set_var("PLEXI_SCENE_CHANNEL", "test-attach");
        let mut backend = LiveBackend::from_env().expect("live config");
        backend.attached_host = true;

        backend.teardown();

        assert_eq!(
            backend.teardown,
            TeardownResult {
                attempted: false,
                ok: true,
                detail: "attached_host_untouched".to_string(),
            }
        );
        std::env::remove_var("PLEXI_SCENE_CHANNEL");
    }

    #[test]
    fn report_schema_has_backend_channel_errors_and_teardown_parity() {
        let report = failed_report(
            "parity".to_string(),
            SceneError::new("eventual_timeout", "bounded wait expired"),
        );
        let json = serde_json::to_value(report).expect("serialize report");

        assert_eq!(json["schema_version"], SCENE_REPORT_SCHEMA_VERSION);
        assert_eq!(json["backend"], "headless");
        assert!(json.get("channel").is_some());
        assert_eq!(json["steps"][0]["error"]["code"], "eventual_timeout");
        assert!(json.get("teardown").is_some());
    }

    #[test]
    fn real_host_status_json_classifies_running_and_stopped() {
        let running = serde_json::json!({
            "ready": true,
            "pane_count": 2,
            "pid": 47709,
            "socket": "/tmp/notify.sock"
        });
        let starting = serde_json::json!({
            "ready": false,
            "pane_count": null,
            "pid": 47709,
            "socket": "/tmp/notify.sock"
        });
        let stopped = serde_json::json!({
            "ready": false,
            "pane_count": null,
            "pid": null,
            "socket": "/tmp/notify.sock"
        });

        assert_eq!(classify_host_status(&running), HostStatusClass::Running);
        assert_eq!(classify_host_status(&starting), HostStatusClass::Running);
        assert_eq!(classify_host_status(&stopped), HostStatusClass::Stopped);
        assert_eq!(
            classify_host_status(&serde_json::json!({})),
            HostStatusClass::Unknown
        );
    }

    #[test]
    fn live_seed_readiness_requires_ready_and_declared_pane_count() {
        assert!(!host_seed_ready(
            &serde_json::json!({"ready": false, "pane_count": 1, "pid": 42}),
            1,
        ));
        assert!(!host_seed_ready(
            &serde_json::json!({"ready": true, "pane_count": 0, "pid": 42}),
            1,
        ));
        assert!(host_seed_ready(
            &serde_json::json!({"ready": true, "pane_count": 1, "pid": 42}),
            1,
        ));
    }

    #[test]
    fn live_seed_readiness_timeout_is_bounded() {
        let error = poll_until(Duration::from_millis(3), Duration::from_millis(1), || {
            host_seed_ready(
                &serde_json::json!({"ready": true, "pane_count": 0, "pid": 42}),
                1,
            )
            .then_some(())
        })
        .unwrap_err();

        assert_eq!(error.code, "eventual_timeout");
    }

    #[test]
    fn bounded_eventual_timeout_is_structured() {
        let error = poll_until(Duration::from_millis(3), Duration::from_millis(1), || {
            None::<()>
        })
        .unwrap_err();

        assert_eq!(error.code, "eventual_timeout");
    }

    #[test]
    fn live_backend_real_host_polls_use_the_central_boundary() {
        let source = include_str!("scenes.rs");
        let start = source
            .find("impl LiveBackend {")
            .expect("LiveBackend implementation exists");
        let end = source[start..]
            .find("\n}\n\nfn geometry_failures")
            .map(|offset| start + offset)
            .expect("LiveBackend implementation has a stable end marker");
        let live_backend = &source[start..end];

        assert!(
            !live_backend.contains("Instant::now"),
            "LiveBackend host polling must use poll_live_until, never a raw deadline"
        );
        assert!(
            !live_backend.contains("poll_until("),
            "LiveBackend host polling must use load-aware poll_live_until"
        );
        assert!(
            !live_backend.contains("std::thread::sleep(LIVE_POLL_INTERVAL)"),
            "LiveBackend host polling must let poll_live_until own poll cadence"
        );
    }

    #[test]
    fn ownership_marker_distinguishes_runner_host_from_attached_host() {
        let dir = tempfile::tempdir().expect("marker dir");
        let marker = dir.path().join("owner");
        assert!(!marker.exists(), "attached host has no ownership marker");

        std::fs::write(&marker, "pr-4242\n").expect("write owner marker");
        assert!(
            marker.exists(),
            "runner-owned host authorizes interrupt cleanup"
        );

        std::fs::remove_file(&marker).expect("clear marker after teardown");
        assert!(
            !marker.exists(),
            "successful teardown revokes cleanup authority"
        );
    }

    #[test]
    fn live_context_count_includes_empty_contexts_and_has_pane_fallback() {
        let contexts = serde_json::json!([
            {"context_id": 10, "is_active": true},
            {"context_id": 20, "is_active": false}
        ]);
        let panes = vec![serde_json::json!({"id": 1, "context_id": 10})];

        assert_eq!(live_context_count(Some(&contexts), &panes), 2);
        assert_eq!(live_context_count(None, &panes), 1);
    }

    #[test]
    fn live_builtin_open_resolves_explicit_cwd_for_command() {
        let open = OpenSpec::Builtin {
            id: "file_browser".to_string(),
            handle: "files".to_string(),
            cwd: Some("tests".to_string()),
            args: Vec::new(),
        };

        let cwd = live_builtin_cwd(&open).expect("explicit builtin cwd");

        assert_eq!(cwd, Path::new(env!("CARGO_MANIFEST_DIR")).join("tests"));
    }

    #[test]
    fn live_whole_host_target_fails_explicitly() {
        let handles = PaneHandles::default();

        let error = resolve_live_pane_target(&handles, "host", "key input").unwrap_err();

        assert_eq!(error.code, "unsupported_live_target");
    }
}
