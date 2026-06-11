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

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::host::pane::{AppRuntime, Pane};
use crate::spatial::tiling::PaneId;
use crate::ui_tests::PlexiUiHarness;

pub const SCENE_REPORT_SCHEMA_VERSION: u32 = 1;

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
    /// Launch a real PGAP app process from a repo-relative app dir.
    /// `args` are forwarded in `PlexiEvent::Init` and surface as `ctx.args` —
    /// pass JSON state for deterministic scenes.
    OpenApp {
        open_app: String,
        #[serde(default)]
        args: Vec<String>,
    },
    /// Open the built-in file browser at a repo-relative (or absolute) dir.
    OpenFileBrowser { open_file_browser: String },
    /// Press a key combo, e.g. "cmd+b", "enter", "ctrl+shift+p".
    Key { key: String },
    /// Toggle the host sidebar.
    Sidebar { sidebar: bool },
    /// Switch to the context at this router index.
    SwitchContext { switch_context: usize },
    /// Push the focused pane into a new subcontext with this name.
    PushToSubcontext { push_to_subcontext: String },
    /// Block until the last-opened app commits its first real frame.
    WaitAppFrame { wait_app_frame: WaitSpec },
    /// Advance N harness frames.
    RunSteps { run_steps: usize },
    /// Structured assertions — every present key must match.
    Assert { assert: AssertSpec },
    /// Save a headless screenshot to `<out_dir>/<name>`.
    Shot { shot: String },
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct WaitSpec {
    pub timeout_s: f32,
}

#[derive(Deserialize, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct AssertSpec {
    pub pane_count: Option<usize>,
    pub window_count: Option<usize>,
    pub context_count: Option<usize>,
    /// Portal panes across all windows.
    pub portal_count: Option<usize>,
    pub sidebar: Option<bool>,
    /// Lifecycle of the last-opened app pane, lowercase (e.g. "running").
    pub lifecycle: Option<String>,
    /// Substring match against the last-opened app's serialized L1 tree.
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
    pub detail: Option<String>,
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

pub struct SceneRunner {
    h: PlexiUiHarness,
    last_app_pane: Option<PaneId>,
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
            return failed_report(scene_name, format!("read {}: {e}", scene_path.display()));
        }
    };
    let scene: Scene = match toml::from_str(&raw) {
        Ok(s) => s,
        Err(e) => {
            return failed_report(scene_name, format!("parse {}: {e}", scene_path.display()));
        }
    };

    std::fs::create_dir_all(out_dir).ok();

    let mut h = PlexiUiHarness::new_sized(scene.size[0], scene.size[1]);
    h.step();
    let mut runner = SceneRunner {
        h,
        last_app_pane: None,
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
            }),
            Err(e) => {
                steps.push(StepResult {
                    index,
                    step: label,
                    ok: false,
                    detail: Some(e),
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
        host: runner.host_state(),
        app: runner.app_state(),
    };
    let report_path = out_dir.join(format!("{scene_name}.json"));
    match serde_json::to_string_pretty(&report) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&report_path, json) {
                log::warn!("scene {scene_name}: failed to write report {}: {e}", report_path.display());
            }
        }
        Err(e) => log::warn!("scene {scene_name}: failed to serialize report: {e}"),
    }
    report
}

fn failed_report(scene: String, error: String) -> SceneReport {
    SceneReport {
        schema_version: SCENE_REPORT_SCHEMA_VERSION,
        scene,
        passed: false,
        steps: vec![StepResult {
            index: 0,
            step: "load".to_string(),
            ok: false,
            detail: Some(error),
        }],
        shots: Vec::new(),
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
        Step::OpenApp { open_app, .. } => format!("open_app {open_app}"),
        Step::OpenFileBrowser { open_file_browser } => {
            format!("open_file_browser {open_file_browser}")
        }
        Step::Key { key } => format!("key {key}"),
        Step::Sidebar { sidebar } => format!("sidebar {sidebar}"),
        Step::SwitchContext { switch_context } => format!("switch_context {switch_context}"),
        Step::PushToSubcontext { push_to_subcontext } => {
            format!("push_to_subcontext {push_to_subcontext}")
        }
        Step::WaitAppFrame { wait_app_frame } => {
            format!("wait_app_frame {}s", wait_app_frame.timeout_s)
        }
        Step::RunSteps { run_steps } => format!("run_steps {run_steps}"),
        Step::Assert { .. } => "assert".to_string(),
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

impl SceneRunner {
    fn exec(&mut self, step: &Step, shots: &mut Vec<String>) -> Result<Option<String>, String> {
        match step {
            Step::OpenApp { open_app, args } => {
                let pane_id = self.h.open_app_at(&resolve(open_app), args)?;
                self.last_app_pane = Some(pane_id);
                self.h.step();
                Ok(Some(format!("pane {pane_id}")))
            }
            Step::OpenFileBrowser { open_file_browser } => {
                self.h.open_file_browser(resolve(open_file_browser));
                self.h.step();
                Ok(None)
            }
            Step::Key { key } => {
                let (modifiers, k) = parse_key(key)?;
                self.h.harness().press_key_modifiers(modifiers, k);
                self.h.step();
                Ok(None)
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
                    return Err(format!("switch_context {idx}: only {len} contexts exist"));
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
                let pane_id = self
                    .last_app_pane
                    .ok_or("wait_app_frame: no app opened yet")?;
                self.h.wait_for_app_frame(
                    pane_id,
                    Duration::from_secs_f32(wait_app_frame.timeout_s),
                )?;
                Ok(None)
            }
            Step::RunSteps { run_steps } => {
                self.h.run_steps(*run_steps);
                Ok(None)
            }
            Step::Assert { assert } => self.check(assert).map(|()| None),
            Step::Shot { shot } => {
                if self.no_shots {
                    return Ok(Some("skipped (no-shots)".to_string()));
                }
                let path = self.out_dir.join(shot);
                self.h
                    .save_screenshot(&path.to_string_lossy())
                    .map_err(|e| format!("shot {shot}: {e}"))?;
                shots.push(path.display().to_string());
                Ok(Some(path.display().to_string()))
            }
        }
    }

    fn check(&mut self, spec: &AssertSpec) -> Result<(), String> {
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
            let app = self
                .app_state()
                .ok_or("assert lifecycle/tree_contains: no app opened yet")?;
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
            Err(failures.join("; "))
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
        self.h.with_app(|app| {
            for win in &app.windows {
                if let Some(Pane::App(app_pane)) = win.panes.get(&pane_id) {
                    if let AppRuntime::Process(p) = &app_pane.runtime {
                        return Some(AppState {
                            pane_id,
                            lifecycle: format!("{:?}", p.lifecycle.state()).to_lowercase(),
                            tree: serde_json::to_value(&p.frame)
                                .unwrap_or(serde_json::Value::Null),
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
    Ok((modifiers, key.ok_or_else(|| format!("no key in combo \"{combo}\""))?))
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn scenes_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("scenes")
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
            let scene: Scene = toml::from_str(&raw)
                .unwrap_or_else(|e| panic!("parse {}: {e}", path.display()));
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
        assert!(failures.is_empty(), "scenes failed:\n{}", failures.join("\n"));
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
}
