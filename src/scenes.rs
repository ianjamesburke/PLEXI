//! Declarative scene runner — TOML scenes ARE the UI tests.
//!
//! A scene file describes setup steps, actions, structured assertions, and
//! optional screenshots. One engine (`PlexiUiHarness`) executes every scene:
//! real host chrome, real PGAP app processes, headless wgpu rendering.
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
//! Every run writes a `SceneReport` JSON (schema-versioned) to the out dir:
//! pass/fail per step, host state snapshot, and the committed L1 render tree
//! of the last-opened app. Agents read the report; screenshots are an
//! optional artifact (`shot` steps; suppressed by `PLEXI_SCENE_NO_SHOTS=1`).
//!
//! # DSL
//!
//! Assertions are structured keys (typed matchers), never expression strings.
//! New verbs require a scene that needs them.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::host::pane::{AppRuntime, Pane};
use crate::spatial::tiling::PaneId;
use crate::ui_tests::PlexiUiHarness;

pub const SCENE_REPORT_SCHEMA_VERSION: u32 = 2;

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
    pub steps: Vec<Step>,
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
    /// Insert text through egui's normal text-input event path.
    Text { text: TextSpec },
    /// Press a key combo against a pane handle or the whole host.
    Key { key: KeySpec },
    /// Toggle the host sidebar.
    Sidebar { sidebar: bool },
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
    /// Assert that the headless host accessibility tree contains an exact label.
    AssertLabel { assert_label: AssertLabelSpec },
    /// Save a headless screenshot to `<out_dir>/<name>`.
    Shot { shot: String },
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
    /// Lifecycle of the target app pane, lowercase (e.g. "running").
    pub lifecycle: Option<String>,
    /// Substring match against the target app's serialized L1 tree.
    pub tree_contains: Option<String>,
}

// ─── Report format ───────────────────────────────────────────────────────────

#[derive(Serialize, Debug)]
pub struct SceneReport {
    pub schema_version: u32,
    pub scene: String,
    pub passed: bool,
    pub steps: Vec<StepResult>,
    pub shots: Vec<String>,
    /// Symbolic pane handles resolved during this run.
    pub handles: BTreeMap<String, PaneId>,
    pub host: HostState,
    /// Last-opened app pane state, when a scene opened one.
    pub app: Option<AppState>,
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
}

impl SceneError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
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

pub struct SceneRunner {
    h: PlexiUiHarness,
    last_app_pane: Option<PaneId>,
    handles: PaneHandles,
    out_dir: PathBuf,
    no_shots: bool,
}

/// Run a scene file. Writes `<out_dir>/<scene-stem>.json` and returns the
/// report. Execution stops at the first failing step (fail fast); the report
/// records everything up to and including the failure.
pub fn run_scene(scene_path: &Path, out_dir: &Path, no_shots: bool) -> SceneReport {
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

    let mut h = PlexiUiHarness::new_sized(scene.size[0], scene.size[1]);
    h.step();
    let mut runner = SceneRunner {
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
            }),
            Err(e) => {
                steps.push(StepResult {
                    index,
                    step: label,
                    ok: false,
                    detail: None,
                    error: Some(e),
                });
                passed = false;
                break;
            }
        }
    }

    let report = SceneReport {
        schema_version: SCENE_REPORT_SCHEMA_VERSION,
        scene: scene_name.clone(),
        passed,
        steps,
        shots,
        handles: runner.handles.report(),
        host: runner.host_state(),
        app: runner.app_state(),
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
        scene,
        passed: false,
        steps: vec![StepResult {
            index: 0,
            step: "load".to_string(),
            ok: false,
            detail: None,
            error: Some(error),
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
    }
}

fn step_label(step: &Step) -> String {
    match step {
        Step::Open { open } => format!("open {} as {}", open.target_kind(), open.handle()),
        Step::Text { text } => format!(
            "text {} ({} chars)",
            text.target,
            text.value.chars().count()
        ),
        Step::Key { key } => format!("key {} {}", key.target, key.value),
        Step::Sidebar { sidebar } => format!("sidebar {sidebar}"),
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
    for capability_id in grants.capability_ids() {
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

impl SceneRunner {
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
                    self.h.harness().press_key_modifiers(modifiers, k);
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
            Step::Sidebar { sidebar } => {
                let v = *sidebar;
                self.h.with_app_mut(|app| app.sidebar_visible = v);
                self.h.step();
                Ok(None)
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
                    .wait_for_app_frame(pane_id, Duration::from_secs_f32(wait_app_frame.timeout_s))
                    .map_err(|message| SceneError::new("wait_app_frame_failed", message))?;
                Ok(None)
            }
            Step::RunSteps { run_steps } => {
                self.h.run_steps(*run_steps);
                Ok(None)
            }
            Step::Assert { assert } => self.check(assert).map(|()| None),
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
        if spec.lifecycle.is_some() || spec.tree_contains.is_some() {
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
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(SceneError::new("assertion_failed", failures.join("; ")))
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
                    if let AppRuntime::Process(p) = &app_pane.runtime {
                        return Some(AppState {
                            pane_id,
                            lifecycle: format!("{:?}", p.lifecycle.state()).to_lowercase(),
                            tree: serde_json::to_value(&p.frame).unwrap_or(serde_json::Value::Null),
                        });
                    }
                    if let AppRuntime::Wasm(w) = &app_pane.runtime {
                        return Some(AppState {
                            pane_id,
                            lifecycle: if w.is_running() { "running" } else { "exited" }
                                .to_string(),
                            tree: serde_json::Value::String(w.last_render_text().to_string()),
                        });
                    }
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
    fn generic_scene_steps_parse_with_typed_targets() {
        let raw = r#"
            [[steps]]
            open = { kind = "builtin", id = "assistant", cwd = ".", as = "assistant" }

            [[steps]]
            text = { target = "assistant", value = "/settings" }

            [[steps]]
            key = { target = "assistant", value = "enter" }

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
                Step::AssertLabel {
                    assert_label: AssertLabelSpec { target: label_target, .. }
                }
            ] if handle == "assistant"
                && text_target == "assistant"
                && key_target == "assistant"
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
}
