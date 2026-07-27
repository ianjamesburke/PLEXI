//! Semantic UI regression tests using egui_kittest.
//!
//! # Architecture
//!
//! `PlexiUiHarness` wraps `PlexiApp` inside egui_kittest's `Harness` using the
//! `new_eframe` constructor — PlexiApp receives the egui::Context from kittest's
//! `CreationContext`, which is the same context used for every subsequent frame.
//!
//! After each `step()`, the accessibility tree is updated and semantic queries
//! are available via `h.harness().get_by_label("...")`. State assertions use
//! `h.harness().state()` which returns `&PlexiApp`.
//!
//! # Screenshot support
//!
//! `render()` returns an `image::RgbaImage` via wgpu Metal offscreen rendering.
//! On macOS this is fully headless — no display required. Save with:
//! ```
//! h.render().unwrap().save("/tmp/out.png").unwrap();
//! ```
//!
//! # State isolation
//!
//! `PlexiApp::new_for_test` uses `PlexiConfig::default()` (no disk reads) and
//! `temp_dir()` as the workspace root. `save_workspace()` writes to
//! `~/.plexi-<binary-hash>/` — a throwaway dir separate from `~/.plexi-alpha/`.
//!
//! # PTY requirement
//!
//! `split_focused` and `new_context` spawn real terminal panes. Pass on macOS
//! local dev; tag `#[ignore]` for headless CI without PTY support.

use std::path::Path;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::app::PlexiApp;
use crate::host::pane::{AppRuntime, Pane};
use crate::spatial::tiling::PaneId;

// ─── PlexiUiHarness ──────────────────────────────────────────────────────────

/// Generic app target used by the scene runner. Builtin ids resolve through
/// the production factory plus the test-only registry below.
pub(crate) enum HarnessOpenTarget<'a> {
    Process {
        path: &'a Path,
        args: &'a [String],
    },
    Wasm {
        path: &'a Path,
        args: &'a [String],
    },
    Builtin {
        id: &'a str,
        cwd: &'a Path,
        args: &'a [String],
    },
}

type HarnessBuiltinFactory = fn(&Path, &[String]) -> Box<dyn crate::app::app_trait::App>;

fn assistant_harness_factory(
    workspace_root: &Path,
    _args: &[String],
) -> Box<dyn crate::app::app_trait::App> {
    let broker: std::sync::Arc<dyn crate::plexi_ai::broker::AiBroker> =
        std::sync::Arc::new(crate::plexi_ai::broker::LiveAiBroker::new(None));
    Box::new(crate::assistant::AssistantApp::new(
        workspace_root.to_path_buf(),
        broker,
        workspace_root,
        1,
    ))
}

const HARNESS_BUILTIN_FACTORIES: &[(&str, HarnessBuiltinFactory)] =
    &[("assistant", assistant_harness_factory)];

fn harness_builtin_factory(
    id: &str,
    cwd: &Path,
    args: &[String],
) -> Option<Box<dyn crate::app::app_trait::App>> {
    HARNESS_BUILTIN_FACTORIES
        .iter()
        .find(|(candidate, _)| *candidate == id)
        .map(|(_, factory)| factory(cwd, args))
}

/// Semantic UI test harness. Wraps egui_kittest's `Harness<PlexiApp>` using the
/// `new_eframe` constructor so PlexiApp gets the kittest-managed egui::Context
/// at construction time — no shared-state gymnastics needed.
pub struct PlexiUiHarness {
    inner: egui_kittest::Harness<'static, PlexiApp>,
    workspace: tempfile::TempDir,
}

impl PlexiUiHarness {
    /// Create a harness with a fresh, isolated PlexiApp.
    pub fn new() -> Self {
        let workspace = tempfile::tempdir().expect("create UI harness workspace");
        let frame_tick = Arc::new(AtomicU64::new(0));
        let harness = egui_kittest::Harness::new_eframe(move |cc| {
            let (app, _ipc_tx) = PlexiApp::new_for_test(cc.egui_ctx.clone(), frame_tick);
            app
        });
        Self {
            inner: harness,
            workspace,
        }
    }

    /// Create a harness with an explicit surface size. Use for screenshot
    /// tests — the default kittest surface is too small for pane chrome and
    /// app content to be legible in the saved PNG.
    pub fn new_sized(width: f32, height: f32) -> Self {
        Self::new_sized_ppp(width, height, 1.0)
    }

    /// Create a harness with an explicit surface size and `pixels_per_point`.
    /// Use `ppp: 2.0` for HiDPI screenshot tests — pixel-grid regressions
    /// (half-resolution surfaces, off-grid galley origins) are invisible at
    /// the default ppp of 1.0.
    pub fn new_sized_ppp(width: f32, height: f32, ppp: f32) -> Self {
        let workspace = tempfile::tempdir().expect("create UI harness workspace");
        let frame_tick = Arc::new(AtomicU64::new(0));
        let harness = egui_kittest::Harness::builder()
            .with_size(egui::Vec2::new(width, height))
            .with_pixels_per_point(ppp)
            .build_eframe(move |cc| {
                let (app, _ipc_tx) = PlexiApp::new_for_test(cc.egui_ctx.clone(), frame_tick);
                app
            });
        Self {
            inner: harness,
            workspace,
        }
    }

    /// Advance one frame. Uses `step()` — PlexiApp continuously requests
    /// repaints (animations), which would cause `run()` to exceed its 4-step
    /// stability limit.
    pub fn step(&mut self) -> &mut Self {
        self.inner.step();
        self
    }

    /// Advance `n` frames.
    pub fn run_steps(&mut self, n: usize) -> &mut Self {
        self.inner.run_steps(n);
        self
    }

    /// Access the kittest Harness directly for semantic queries:
    /// `h.harness().get_by_label("Split")`, `h.harness().state()`, etc.
    pub fn harness(&mut self) -> &mut egui_kittest::Harness<'static, PlexiApp> {
        &mut self.inner
    }

    /// Render the current frame to an RGBA image via wgpu Metal offscreen
    /// rendering. Fully headless on macOS — no display needed.
    pub fn render(&mut self) -> Result<image::RgbaImage, String> {
        self.inner.render()
    }

    /// Convenience: render and save to a PNG file.
    pub fn save_screenshot(&mut self, path: &str) -> Result<(), String> {
        let img = self.render()?;
        img.save(path).map_err(|e| e.to_string())
    }

    // ── Typed state helpers ───────────────────────────────────────────────────

    /// Number of panes in the active window.
    pub fn pane_count(&self) -> usize {
        let app = self.inner.state();
        app.windows[app.active_window].panes.len()
    }

    /// Number of windows (spatial grid entries) in the app.
    pub fn window_count(&self) -> usize {
        self.inner.state().windows.len()
    }

    /// Mutably access PlexiApp to set up state before assertions.
    pub fn with_app_mut<F, R>(&mut self, f: F) -> R
    where
        F: FnOnce(&mut PlexiApp) -> R,
    {
        f(self.inner.state_mut())
    }

    /// Read-only access to PlexiApp state for assertions.
    pub fn with_app<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&PlexiApp) -> R,
    {
        f(self.inner.state())
    }

    /// Fresh directory owned by this harness. Scenes use it when an open step
    /// omits `cwd`, keeping native app state out of the developer's workspace.
    pub fn workspace_root(&self) -> &Path {
        self.workspace.path()
    }

    // ── Real app / host app / portal drivers ─────────────────────────────────

    /// Open any supported scene target through its normal pane-opening path.
    pub fn open_target(&mut self, target: HarnessOpenTarget<'_>) -> Result<PaneId, String> {
        match target {
            HarnessOpenTarget::Process { path, args } => self.open_app_at(path, args),
            HarnessOpenTarget::Wasm { path, args } => {
                self.open_wasm_at_with_args(path, args.to_vec())
            }
            HarnessOpenTarget::Builtin { id, cwd, args } => self.open_builtin_by_id(id, cwd, args),
        }
    }

    fn open_builtin_by_id(
        &mut self,
        id: &str,
        cwd: &Path,
        args: &[String],
    ) -> Result<PaneId, String> {
        let before: std::collections::HashSet<PaneId> = self.with_app(|app| {
            app.windows
                .iter()
                .flat_map(|window| window.panes.keys())
                .copied()
                .collect()
        });
        if let Some(app) = harness_builtin_factory(id, cwd, args) {
            self.with_app_mut(|host| {
                host.open_builtin_app_pane(
                    app,
                    crate::app::permissions::AppPermissions::builtin(),
                    cwd.to_path_buf(),
                    None,
                    Some("split_v"),
                    None,
                );
            });
        } else {
            self.with_app_mut(|host| {
                host.launch_app_by_id_with_layout(
                    id,
                    Some("split_v".to_string()),
                    args,
                    Some(cwd.to_path_buf()),
                )
            })?;
        }
        let pane_id = self.with_app(|host| {
            host.windows
                .iter()
                .flat_map(|window| window.panes.keys())
                .find(|pane_id| !before.contains(pane_id))
                .copied()
                .ok_or_else(|| format!("no new pane appeared after opening builtin '{id}'"))
        })?;
        let is_builtin = self.with_app(|host| {
            host.windows.iter().any(|window| {
                matches!(
                    window.panes.get(&pane_id),
                    Some(Pane::App(app)) if matches!(app.runtime, AppRuntime::Builtin(_))
                )
            })
        });
        if !is_builtin {
            return Err(format!("app id '{id}' did not resolve to a builtin"));
        }
        Ok(pane_id)
    }

    /// Focus a pane through the same navigation path used by host commands.
    pub fn focus_pane(&mut self, pane_id: PaneId) -> Result<(), String> {
        if self.with_app_mut(|app| app.pane_navigate(pane_id)) {
            Ok(())
        } else {
            Err(format!("pane {pane_id} no longer exists"))
        }
    }

    /// Close a pane through the same lifecycle path used by `plexi pane close`.
    pub fn close_pane(&mut self, pane_id: PaneId) -> Result<(), String> {
        let exists = self.with_app(|app| {
            app.windows
                .iter()
                .any(|window| window.panes.contains_key(&pane_id))
        });
        if !exists {
            return Err(format!("pane {pane_id} no longer exists"));
        }
        self.with_app_mut(|app| app.close_pane_by_id(pane_id));
        Ok(())
    }

    /// Exact semantic-label lookup scoped to one rendered pane rectangle.
    pub fn pane_has_label(&mut self, pane_id: PaneId, label: &str) -> bool {
        use egui_kittest::kittest::Queryable;

        let pane_rect = self.with_app(|host| {
            host.windows.iter().find_map(|window| {
                if !window.panes.contains_key(&pane_id) {
                    return None;
                }
                let tile_id = window.tree.tiles.iter().find_map(|(tile_id, tile)| {
                    matches!(tile, egui_tiles::Tile::Pane(id) if *id == pane_id).then_some(tile_id)
                })?;
                window.tree.tiles.rect(*tile_id)
            })
        });
        let Some(pane_rect) = pane_rect else {
            return false;
        };

        self.inner
            .query_all_by_label(label)
            .any(|node| pane_rect.contains(node.rect().center()))
    }

    /// Exact semantic-label lookup against the whole headless host tree.
    pub fn host_has_label(&mut self, label: &str) -> bool {
        use egui_kittest::kittest::Queryable;

        self.inner.query_all_by_label(label).next().is_some()
    }

    /// Launch a real PGAP app process from `app_dir` (a directory containing
    /// `manifest.toml`) — the same production path as `plexi app open <path>`.
    /// The child process, IPC threads, and L1 render pipeline are all real.
    /// Returns the new pane's id.
    ///
    /// For `.py` entries running outside an installed bundle, the repo SDK at
    /// `sdk/python` is exported via `PLEXI_SDK_PATH` so `plexi_sdk` imports
    /// resolve in the child.
    ///
    /// `args` are forwarded in `PlexiEvent::Init` and surface as `ctx.args` —
    /// pass JSON state for deterministic scenes.
    pub fn open_app_at(&mut self, app_dir: &Path, args: &[String]) -> Result<PaneId, String> {
        if std::env::var_os("PLEXI_SDK_PATH").is_none() {
            let dev_sdk = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("sdk")
                .join("python");
            if dev_sdk.join("plexi_sdk").join("__init__.py").is_file() {
                std::env::set_var("PLEXI_SDK_PATH", &dev_sdk);
            }
        }
        let before: std::collections::HashSet<PaneId> = self.with_app(|app| {
            app.windows[app.active_window]
                .panes
                .keys()
                .copied()
                .collect()
        });
        let path = app_dir.to_string_lossy().into_owned();
        self.with_app_mut(|app| {
            app.launch_app_by_path_with_layout(&path, Some("split_v".to_string()), None, args)
        })?;
        self.with_app(|app| {
            app.windows[app.active_window]
                .panes
                .keys()
                .find(|id| !before.contains(id))
                .copied()
                .ok_or_else(|| format!("no new pane appeared after launching {path}"))
        })
    }

    /// Open a sandboxed WASM component app, forwarding `args` to the guest's
    /// `init` as its argv (the same path as `plexi app open <path> -- <args>`).
    pub fn open_wasm_at_with_args(
        &mut self,
        wasm_path: &Path,
        args: Vec<String>,
    ) -> Result<PaneId, String> {
        let app_id = wasm_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("wasm")
            .to_string();
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        self.with_app_mut(|app| app.open_wasm_app_pane(&app_id, wasm_path, cwd, args))
    }

    /// Step frames until the real app process behind `pane_id` commits its
    /// first rendered frame (FrameDone observed), sleeping between steps to
    /// give the child process wall-clock time. Fails with the app's recent
    /// stderr on crash or timeout.
    pub fn wait_for_app_frame(&mut self, pane_id: PaneId, timeout: Duration) -> Result<(), String> {
        enum Probe {
            Rendered,
            Waiting,
            Dead(String),
        }
        let start = Instant::now();
        loop {
            self.step();
            let probe = self.with_app(|app| {
                let win = &app.windows[app.active_window];
                let Some(Pane::App(app_pane)) = win.panes.get(&pane_id) else {
                    return Probe::Dead(format!("pane {pane_id} is not an app pane"));
                };
                match &app_pane.runtime {
                    AppRuntime::Python(p) if p.has_rendered_tree() => Probe::Rendered,
                    AppRuntime::Python(p) => p
                        .error()
                        .map(|error| Probe::Dead(error.to_string()))
                        .unwrap_or(Probe::Waiting),
                    AppRuntime::Wasm(p) if p.is_running() => Probe::Rendered,
                    _ => Probe::Dead(format!("pane {pane_id} is not a WASM app")),
                }
            });
            match probe {
                Probe::Rendered => return Ok(()),
                Probe::Dead(msg) => return Err(msg),
                Probe::Waiting => {}
            }
            if start.elapsed() > timeout {
                return Err(format!(
                    "timed out after {timeout:?} waiting for first app frame"
                ));
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    /// Open the built-in file browser host app rooted at `cwd`. Uses the
    /// production `open_builtin_app_pane` path with a `split_v` hint, which
    /// installs as the root pane when the context is empty (the ⌘O shortcut's
    /// `overlay` hint requires an existing focused pane).
    pub fn open_file_browser(&mut self, cwd: std::path::PathBuf) {
        self.with_app_mut(|app| {
            let fb: Box<dyn crate::app::app_trait::App> =
                Box::new(crate::file_browser::FileBrowserApp::new(cwd.clone()));
            let perms = crate::app::permissions::AppPermissions::builtin();
            app.open_builtin_app_pane(
                fb,
                perms,
                cwd,
                Some("cwd".to_string()),
                Some("split_v"),
                None,
            );
        });
    }

    /// Convert the focused pane into a subcontext portal — same path as the
    /// push-pane-to-subcontext host shortcut and CLI command.
    pub fn push_focused_pane_to_subcontext(&mut self, name: Option<String>) {
        self.with_app_mut(|app| app.push_pane_to_subcontext(name, None));
    }

    /// Open the host Assistant pane rooted at `workspace_root` — same builtin
    /// pane path as `open_assistant_pane`, but with an explicit (temp)
    /// workspace root so test transcripts never land in a real workspace.
    /// `LiveAiBroker::new(None)` fails fast if a turn is ever dispatched.
    pub fn open_assistant(&mut self, workspace_root: std::path::PathBuf) {
        let broker: std::sync::Arc<dyn crate::plexi_ai::broker::AiBroker> =
            std::sync::Arc::new(crate::plexi_ai::broker::LiveAiBroker::new(None));
        // Scope the store to the active window's context — same as the
        // production open path.
        let context_id = self.with_app_mut(|app| app.windows[app.active_window].context_id);
        // Grants/audit also stay inside the temp workspace, never the real
        // channel profile dir.
        let assistant = crate::assistant::AssistantApp::new(
            workspace_root.clone(),
            broker,
            &workspace_root,
            context_id,
        );
        self.open_assistant_built(assistant, workspace_root);
    }

    /// Open a pre-built Assistant (lets tests seed model state — e.g. a
    /// pending permission sheet) through the same builtin pane path.
    pub fn open_assistant_built(
        &mut self,
        assistant: crate::assistant::AssistantApp,
        workspace_root: std::path::PathBuf,
    ) {
        self.with_app_mut(|app| {
            let boxed: Box<dyn crate::app::app_trait::App> = Box::new(assistant);
            let perms = crate::app::permissions::AppPermissions::builtin();
            app.open_builtin_app_pane(
                boxed,
                perms,
                workspace_root.clone(),
                None,
                Some("split_v"),
                None,
            );
        });
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::permissions::AppPermissions;
    use crate::app::FocusKind;
    use crate::host::context::Window;
    use crate::host::pane::{AppPane, AppRuntime, Pane, PortalPane};
    use egui_kittest::kittest::Queryable;

    fn add_focused_pane(h: &mut PlexiUiHarness) -> crate::spatial::tiling::PaneId {
        let active_window = h.with_app(|app| app.active_window);
        add_focused_pane_to_window(h, active_window)
    }

    fn add_focused_pane_to_window(
        h: &mut PlexiUiHarness,
        window_index: usize,
    ) -> crate::spatial::tiling::PaneId {
        // Root the file browser in the harness's empty workspace — never the
        // real temp_dir(), whose ~50k dev-machine entries balloon rendering.
        let browser_root = h.workspace.path().to_path_buf();
        h.with_app_mut(|app| {
            let pane_id = app.host.alloc_pane_id();
            let app_pane = AppPane {
                pip_status: None,
                id: pane_id,
                runtime: AppRuntime::Builtin(Box::new(crate::file_browser::FileBrowserApp::new(
                    browser_root.clone(),
                ))),
                workspace_root: browser_root.clone(),
                permissions: AppPermissions::builtin(),
                manifest_id: "test".to_string(),
                name: "Test App".to_string(),
                pane_group: None,
                linked_pane_id: None,
                overlay_replaced: None,
                hidden: false,
                agent: None,
                slots: std::collections::HashMap::new(),
                semantic_state: Default::default(),
            };
            let win = &mut app.windows[window_index];
            win.panes.insert(pane_id, Pane::App(Box::new(app_pane)));
            let tile_id = win.tree.tiles.insert_pane(pane_id);
            if win.tree.root.is_none() {
                win.tree.root = Some(tile_id);
            }
            win.focused_pane = Some(tile_id);
            pane_id
        })
    }

    /// Regression for stint 0351: `AppRequest::KeyPane` (the `plexi pane key`
    /// socket path) must drive a native app pane through its real
    /// `App::handle_key` — not the PGAP event queue, which native apps never
    /// read. Enter on the File Explorer's selected directory must run the
    /// app's own Activate flow (navigate_into), and the response file must
    /// report the disposition so driving agents can detect ignored keys.
    #[test]
    fn pane_key_ipc_drives_native_file_browser_handle_key() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(dir.path().join("inner")).expect("mkdir");

        let mut h = PlexiUiHarness::new();
        h.open_file_browser(dir.path().to_path_buf());
        h.step();

        let pane_id = h.with_app(|app| {
            app.windows[app.active_window]
                .panes
                .iter()
                .find_map(|(id, p)| p.as_app().map(|_| *id))
                .expect("file browser pane must exist after open_file_browser")
        });

        let rf_dir = tempfile::tempdir().expect("tempdir");
        let rf = rf_dir.path().join("key-response.json");
        h.with_app_mut(|app| {
            app.handle_pane_ipc_request(crate::app_protocol::AppRequest::KeyPane {
                pane_id,
                key: "enter".to_string(),
                response_file: Some(rf.to_string_lossy().into_owned()),
            })
        });

        let content = std::fs::read_to_string(&rf).expect("KeyPane must write a response file");
        let v: serde_json::Value = serde_json::from_str(&content).expect("response must be JSON");
        assert_eq!(v["ok"], true, "response must be ok; got {content}");
        assert_eq!(
            v["disposition"], "consumed",
            "File Explorer must consume Enter; got {content}"
        );

        // Enter on the selected directory (dirs sort first, selected=0) runs
        // the app's real Activate flow — cwd must now be the subdirectory.
        let cwd = h.with_app(|app| {
            app.windows[app.active_window]
                .panes
                .get(&pane_id)
                .and_then(|p| p.as_app())
                .and_then(|a| a.runtime.serialize_state())
                .and_then(|s| s["cwd"].as_str().map(str::to_string))
                .expect("file browser must serialize cwd")
        });
        assert!(
            cwd.ends_with("inner"),
            "Enter must navigate into the selected directory; cwd={cwd}"
        );
    }

    #[test]
    fn file_browser_rename_text_field_commits_on_enter() {
        let dir = tempfile::tempdir().expect("tempdir");
        let original = dir.path().join("original.txt");
        let renamed = dir.path().join("renamed.txt");
        std::fs::write(&original, b"contents").expect("write fixture");

        let mut h = PlexiUiHarness::new_sized(720.0, 520.0);
        h.open_file_browser(dir.path().to_path_buf());
        h.run_steps(2);
        h.harness()
            .key_down_modifiers(egui::Modifiers::COMMAND, egui::Key::R);
        h.step();
        h.harness()
            .key_up_modifiers(egui::Modifiers::COMMAND, egui::Key::R);
        h.step();
        h.run_steps(3);

        h.harness()
            .key_down_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.step();
        h.harness()
            .key_up_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.step();
        h.harness()
            .input_mut()
            .events
            .push(egui::Event::Text("renamed.txt".to_string()));
        h.step();
        h.harness()
            .key_down_modifiers(egui::Modifiers::NONE, egui::Key::Enter);
        h.step();
        h.harness()
            .key_up_modifiers(egui::Modifiers::NONE, egui::Key::Enter);
        h.step();
        h.run_steps(2);

        assert!(!original.exists(), "Enter must rename the original path");
        assert!(renamed.exists(), "Enter must create the renamed path");
        assert_eq!(
            std::fs::read(&renamed).expect("read renamed file"),
            b"contents"
        );
    }

    /// Render the initial empty state and save a screenshot for visual inspection.
    /// File written to /tmp/plexi_init.png.
    #[test]
    fn screenshot_init() {
        let mut h = PlexiUiHarness::new();
        h.step();
        h.save_screenshot("/tmp/plexi_init.png")
            .expect("render failed");
        println!("Screenshot saved to /tmp/plexi_init.png");
    }

    /// Raw `.wasm` paths queue a host pre-launch review before spawning. The
    /// underlying launch path still fails closed until the modal persists the
    /// required link-time import grants.
    #[test]
    fn wasm_path_launch_queues_review_before_spawning() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/wasm-fixtures/sysmon.wasm");
        let config_dir = tempfile::tempdir().expect("temp config dir");
        let _profile_guard = crate::config::set_test_profile_dir(config_dir.path().to_path_buf());
        let mut h = PlexiUiHarness::new_sized(900.0, 620.0);

        let result = h.with_app_mut(|app| {
            app.launch_app_by_path_with_layout(
                &fixture.to_string_lossy(),
                Some("split_v".to_string()),
                None,
                &[],
            )
        });

        result.expect("unreviewed wasm launch should queue review");
        h.step();
        let queued = h.with_app(|app| app.pending_raw_wasm_launches.len());
        assert_eq!(queued, 1, "raw .wasm path launch should queue one review");
        let has_review_focus =
            h.with_app(|app| matches!(app.focus_stack.last(), Some(FocusKind::RawWasmReview)));
        assert!(has_review_focus, "raw WASM review should own focus");
        let has_wasm = h.with_app(|app| {
            app.windows[app.active_window]
                .panes
                .values()
                .any(|p| matches!(p, Pane::App(a) if matches!(a.runtime, AppRuntime::Wasm(_))))
        });
        assert!(
            !has_wasm,
            "unreviewed .wasm path launch must not spawn a WASM pane"
        );

        h.harness().key_press(egui::Key::Enter);
        h.step();
        let queued = h.with_app(|app| app.pending_raw_wasm_launches.len());
        assert_eq!(queued, 0, "approval should drain the raw WASM review queue");
        let has_wasm = h.with_app(|app| {
            app.windows[app.active_window]
                .panes
                .values()
                .any(|p| matches!(p, Pane::App(a) if matches!(a.runtime, AppRuntime::Wasm(_))))
        });
        assert!(
            has_wasm,
            "approving raw WASM review should spawn an AppRuntime::Wasm pane"
        );
    }

    /// Once raw `.wasm` imports are reviewed and persisted for the path scope,
    /// the same path-launch entry point must still spawn an `AppRuntime::Wasm`
    /// pane rather than attempting a manifest/process launch.
    #[test]
    fn wasm_path_launch_opens_wasm_pane_after_review() {
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/wasm-fixtures/sysmon.wasm");
        let config_dir = tempfile::tempdir().expect("temp config dir");
        let _profile_guard = crate::config::set_test_profile_dir(config_dir.path().to_path_buf());
        let app_id = fixture
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("wasm");
        let workspace_root = fixture.parent().expect("fixture parent");
        let grants = crate::host::wasm_app::WasmApp::inspect_required_grants(&fixture)
            .expect("inspect fixture grants");
        let mut store =
            crate::app::permissions::PermissionStore::load_or_default(config_dir.path());
        for capability_id in grants.capability_ids() {
            store.set_wasm(
                app_id,
                workspace_root,
                &capability_id,
                crate::app::permissions::PermissionState::Green,
            );
        }
        store.save();

        let mut h = PlexiUiHarness::new_sized(900.0, 620.0);
        h.with_app_mut(|app| {
            app.launch_app_by_path_with_layout(
                &fixture.to_string_lossy(),
                Some("split_v".to_string()),
                None,
                &[],
            )
        })
        .expect("reviewed wasm launch should succeed");
        h.step();
        let has_wasm = h.with_app(|app| {
            app.windows[app.active_window]
                .panes
                .values()
                .any(|p| matches!(p, Pane::App(a) if matches!(a.runtime, AppRuntime::Wasm(_))))
        });
        assert!(
            has_wasm,
            "a reviewed .wasm path launch should produce an AppRuntime::Wasm pane"
        );
    }

    /// Subcontext portal whose child context holds a `text-editor` pane: the
    /// minimap must render the document glyph (accent text rules, folded corner)
    /// instead of the generic app grid, with a status pip sized to match the
    /// sidebar. Zoom the portal so the icon is legible in the PNG.
    #[test]
    fn screenshot_portal_text_editor_icon() {
        // Never point the harnessed file browser at the real temp_dir(): a dev
        // machine's temp dir can hold ~50k entries and rendering them balloons
        // this test past 30 GB RSS. An empty scoped tempdir renders instantly.
        let editor_root = tempfile::tempdir().expect("tempdir");
        let editor_path = editor_root.path().to_path_buf();
        let mut h = PlexiUiHarness::new_sized(900.0, 640.0);
        let _base = add_focused_pane(&mut h);
        h.with_app_mut(|app| {
            let active_ctx_id = app.router.active().context_id;
            let child_ctx_id = active_ctx_id + 5000;
            // Register the child context so the portal header shows a real name.
            app.router.push(crate::host::context::Context {
                name: "Notes".to_string(),
                path: editor_path.clone(),
                root: None,
                description: Some("Editing notes".to_string()),
                context_id: child_ctx_id,
                parent_id: Some(active_ctx_id),
                depth: 1,
                parked: false,
            });

            // Child window holding the text-editor pane the portal previews.
            let editor_pane_id = app.host.alloc_pane_id();
            let editor = AppPane {
                pip_status: Some(crate::app_protocol::PipStatus::Green),
                id: editor_pane_id,
                runtime: AppRuntime::Builtin(Box::new(crate::file_browser::FileBrowserApp::new(
                    editor_path.clone(),
                ))),
                workspace_root: editor_path.clone(),
                permissions: AppPermissions::builtin(),
                manifest_id: "text-editor".to_string(),
                name: "note.md".to_string(),
                pane_group: None,
                linked_pane_id: None,
                overlay_replaced: None,
                hidden: false,
                agent: None,
                slots: std::collections::HashMap::new(),
                semantic_state: Default::default(),
            };
            let mut child_panes = std::collections::HashMap::new();
            child_panes.insert(editor_pane_id, Pane::App(Box::new(editor)));
            let mut child_tiles = egui_tiles::Tiles::default();
            let child_tile = child_tiles.insert_pane(editor_pane_id);
            let child_window_id = app.next_window_id;
            app.next_window_id += 1;
            app.windows.push(Window {
                name: "notes window".to_string(),
                path: editor_path.clone(),
                tree: egui_tiles::Tree::new("notes window", child_tile, child_tiles),
                panes: child_panes,
                focused_pane: Some(child_tile),
                zoomed_pane: None,
                grid_x: 0,
                grid_y: 0,
                window_id: child_window_id,
                context_id: child_ctx_id,
            });

            // Portal pane in the active window targeting the child context.
            let portal_pane_id = app.host.alloc_pane_id();
            let active_win = app.active_window;
            let win = &mut app.windows[active_win];
            win.panes.insert(
                portal_pane_id,
                Pane::Portal(Box::new(PortalPane {
                    pane_id: portal_pane_id,
                    target_context_id: child_ctx_id,
                    context_state: None,
                    hidden: false,
                })),
            );
            let portal_tile = win.tree.tiles.insert_pane(portal_pane_id);
            if let Some(root) = win.tree.root {
                let new_root = win
                    .tree
                    .tiles
                    .insert_horizontal_tile(vec![root, portal_tile]);
                win.tree.root = Some(new_root);
            }
            // Zoom the portal so its minimap fills the window.
            win.zoom_to(portal_tile);
        });
        h.run_steps(3);
        h.save_screenshot("/tmp/plexi_portal_text_editor_icon.png")
            .expect("render failed");
        assert!(
            h.with_app(|app| app.windows[app.active_window]
                .panes
                .values()
                .any(|p| p.portal_target().is_some())),
            "active window should contain a portal pane previewing the editor"
        );
        println!("Screenshot saved to /tmp/plexi_portal_text_editor_icon.png");
    }

    /// Direct paint of `paint_portal_minimap` with hand-built `MiniPane`s — no
    /// PTY needed — so every glyph arm (terminal `>_`, text-editor document,
    /// generic app grid) and the status pip render in one screenshot for size
    /// verification. The full-app test above cannot fabricate a Terminal pane
    /// because that needs a real `TerminalBackend`; this bypasses that by
    /// setting `PaneKind` directly.
    #[test]
    fn screenshot_portal_minimap_glyphs() {
        use crate::spatial::tiling::{paint_portal_minimap, MiniPane, MiniWindow, PaneKind};

        let colors = crate::ui::theme::Colors::from_config(
            &crate::ui::theme::preset_colors("catppuccin-mocha").expect("preset"),
        );
        let mk = |kind, focused, activity, x0: f32, x1: f32| MiniPane {
            norm_rect: egui::Rect::from_min_max(egui::pos2(x0, 0.0), egui::pos2(x1, 1.0)),
            kind,
            focused,
            has_content: true,
            title: None,
            active: true,
            activity,
        };
        let windows = vec![MiniWindow {
            grid_x: 0,
            grid_y: 0,
            panes: vec![
                mk(PaneKind::Terminal, true, None, 0.0, 0.34),
                mk(
                    PaneKind::TextEditor,
                    false,
                    Some(crate::app_protocol::AgentState::Working),
                    0.34,
                    0.67,
                ),
                mk(PaneKind::App, false, None, 0.67, 1.0),
            ],
        }];

        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::Vec2::new(540.0, 320.0))
            .build_ui(move |ui| {
                crate::ui::theme::setup_fonts(ui.ctx());
                egui::CentralPanel::default()
                    .frame(egui::Frame::default().fill(colors.bg_darkest))
                    .show_inside(ui, |ui| {
                        paint_portal_minimap(ui.painter(), ui.max_rect(), &windows, &colors, 0.0);
                    });
            });
        harness.run();
        harness
            .render()
            .expect("render failed")
            .save("/tmp/plexi_portal_minimap_glyphs.png")
            .expect("save screenshot");
        println!("Screenshot saved to /tmp/plexi_portal_minimap_glyphs.png");
    }

    // Visual smoke coverage lives in `tests/scenes/*.toml`, executed by
    // `scenes::tests::scene_suite`. Run one ad hoc with `just scene <file>`.

    /// Note editor pane: frontmatter is hidden from the buffer and the
    /// frontmatter title renders as a header row.
    #[test]
    fn text_editor_note_hides_frontmatter_and_shows_title() {
        let mut h = PlexiUiHarness::new();
        h.step();

        let path =
            std::env::temp_dir().join(format!("plexi-ui-note-title-{}.md", std::process::id()));
        std::fs::write(
            &path,
            "---\ntitle: \"Groceries\"\nsource: \"scratchpad\"\n---\nmilk and eggs\n",
        )
        .expect("seed note");

        h.with_app_mut(|app| {
            let pane_id = app.host.alloc_pane_id();
            let app_pane = AppPane {
                pip_status: None,
                id: pane_id,
                runtime: AppRuntime::Builtin(Box::new(
                    crate::app::text_editor_app::TextEditorApp::new_for_test_note(path.clone()),
                )),
                workspace_root: std::env::temp_dir(),
                permissions: AppPermissions::builtin(),
                manifest_id: "text-editor".to_string(),
                name: "Text Editor".to_string(),
                pane_group: None,
                linked_pane_id: None,
                overlay_replaced: None,
                hidden: false,
                agent: None,
                slots: std::collections::HashMap::new(),
                semantic_state: Default::default(),
            };
            let win = &mut app.windows[app.active_window];
            win.panes.insert(pane_id, Pane::App(Box::new(app_pane)));
            let tile_id = win.tree.tiles.insert_pane(pane_id);
            if win.tree.root.is_none() {
                win.tree.root = Some(tile_id);
            }
            win.focused_pane = Some(tile_id);
        });
        h.run_steps(2);

        // Title row is a real ui.label; the YAML block lives only in
        // `note_header` and never reaches the TextEdit buffer (unit-tested
        // via `split_note` in text_editor_app.rs).
        h.harness().get_by_label("Groceries");
        h.save_screenshot("/tmp/plexi_note_editor.png")
            .expect("render failed");

        let _ = std::fs::remove_file(&path);
    }

    /// Find/replace bar: opening it (Cmd+F) renders the Replace/All buttons
    /// through the embedded-bar primitive, whose safe insets keep the button
    /// bottoms off the pane's clip edge (stint 0506 — the bar used to clip).
    #[test]
    fn text_editor_find_bar_buttons_render_unclipped() {
        let mut h = PlexiUiHarness::new_sized(900.0, 400.0);
        h.step();

        let path = std::env::temp_dir().join(format!("plexi-ui-findbar-{}.md", std::process::id()));
        std::fs::write(&path, "the quick brown fox\nthe lazy dog\n").expect("seed note");

        h.with_app_mut(|app| {
            let pane_id = app.host.alloc_pane_id();
            let app_pane = AppPane {
                pip_status: None,
                id: pane_id,
                runtime: AppRuntime::Builtin(Box::new(
                    crate::app::text_editor_app::TextEditorApp::new_for_test_note(path.clone()),
                )),
                workspace_root: std::env::temp_dir(),
                permissions: AppPermissions::builtin(),
                manifest_id: "text-editor".to_string(),
                name: "Text Editor".to_string(),
                pane_group: None,
                linked_pane_id: None,
                overlay_replaced: None,
                hidden: false,
                agent: None,
                slots: std::collections::HashMap::new(),
                semantic_state: Default::default(),
            };
            let win = &mut app.windows[app.active_window];
            win.panes.insert(pane_id, Pane::App(Box::new(app_pane)));
            let tile_id = win.tree.tiles.insert_pane(pane_id);
            if win.tree.root.is_none() {
                win.tree.root = Some(tile_id);
            }
            win.focused_pane = Some(tile_id);
        });
        // Settle focus so the editor owns input, then open the find bar and
        // type a query so the Replace/All buttons are enabled.
        h.run_steps(3);
        h.harness()
            .key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::F);
        h.run_steps(2);
        h.harness().key_press(egui::Key::T);
        h.run_steps(2);

        // The Replace/All buttons are real Button widgets — their presence in
        // the accessibility tree proves the bar laid them out.
        h.harness().get_by_label("Replace");
        h.harness().get_by_label("All");
        h.save_screenshot("/tmp/plexi_note_find_bar.png")
            .expect("render failed");

        let _ = std::fs::remove_file(&path);
    }

    /// Notes picker overlay: open → step → still visible and renders.
    #[test]
    fn notes_picker_overlay_smoke() {
        let mut h = PlexiUiHarness::new();
        h.step();
        add_focused_pane(&mut h);
        h.step();

        h.with_app_mut(|app| {
            app.open_notes_picker();
            // Deterministic entries — don't depend on the machine's notes dir.
            let mk = |name: &str, inbox: bool| crate::notes::NotePickerEntry {
                path: std::env::temp_dir().join(format!("plexi-ui-{name}.md")),
                title: name.to_string(),
                preview: format!("{name} preview"),
                inbox,
                search_text: name.to_lowercase(),
            };
            app.notes_picker_entries = vec![mk("inbox-note", true), mk("kept-note", false)];
            app.notes_picker_selected = 0;
        });
        // Two steps: egui Areas spend their first frame on an invisible
        // sizing pass; the modal is visible from the second frame.
        h.run_steps(2);

        h.with_app(|app| {
            assert!(app
                .focus_stack
                .contains(&crate::app::FocusKind::NotesPicker));
        });
        // ListRow titles are painted as galleys (not accessible labels);
        // assert on the section headers, which are real ui.label widgets.
        h.harness().get_by_label("Inbox (1)");
        // "Notes" appears as both the modal title and the kept-notes header.
        let notes_labels = h.harness().get_all_by_label("Notes").count();
        assert_eq!(notes_labels, 2, "modal title + kept-notes section header");
        h.save_screenshot("/tmp/plexi_notes_picker.png")
            .expect("render failed");
    }

    #[test]
    fn screenshot_ui_gallery_hint_bar_stays_centered_at_wide_width() {
        let mut h = PlexiUiHarness::new_sized(900.0, 640.0);
        h.step();
        h.with_app_mut(|app| {
            app.show_ui_gallery = true;
        });
        // Two steps: egui Areas spend their first frame on an invisible sizing
        // pass; the modal is visible from the second frame.
        h.run_steps(2);

        // Modal title and the first section header are real ui.label widgets.
        h.harness().get_by_label("Host UI Gallery");
        h.harness().get_by_label("Chrome primitives");
        h.harness().get_by_label("palette");
        h.harness().get_by_label("help");
        h.harness().get_by_label("dismiss");
        h.save_screenshot("/tmp/plexi_ui_gallery_hint_bar_wide.png")
            .expect("render failed");
    }

    #[test]
    fn screenshot_ui_gallery_hint_bar_wraps_at_narrow_width() {
        let mut h = PlexiUiHarness::new_sized(290.0, 640.0);
        h.step();
        h.harness().set_size(egui::vec2(290.0, 640.0));
        h.with_app_mut(|app| {
            app.show_ui_gallery = true;
        });
        h.run_steps(2);

        h.harness().get_by_label("Host UI Gallery");
        h.harness().get_by_label("palette");
        h.harness().get_by_label("help");
        h.harness().get_by_label("dismiss");
        h.save_screenshot("/tmp/plexi_ui_gallery_hint_bar_narrow.png")
            .expect("render failed");
    }

    #[test]
    fn parked_sidebar_dropdown_uses_shared_list_header() {
        let mut h = PlexiUiHarness::new_sized(900.0, 620.0);
        h.with_app_mut(|app| {
            app.sidebar_visible = true;
            app.parked_section_expanded = false;
            let mk = |id: u64, name: &str| crate::host::context::Context {
                name: name.to_string(),
                path: std::env::temp_dir().join(name),
                root: None,
                description: None,
                context_id: id,
                parent_id: None,
                depth: 0,
                parked: true,
            };
            app.router.push(mk(70_001, "parked-alpha"));
            app.router.push(mk(70_002, "parked-beta"));
        });
        h.run_steps(2);
        h.with_app(|app| {
            assert_eq!(
                app.router.iter().filter(|ctx| ctx.parked).count(),
                2,
                "test setup should render a parked dropdown"
            );
            assert!(!app.parked_section_expanded);
        });
        h.save_screenshot("/tmp/plexi_parked_dropdown_collapsed.png")
            .expect("render failed");

        // The parked header sits under the "Contexts" header and active row;
        // click its shared fixed-height hit target near the chevron.
        let click = egui::pos2(28.0, 126.0);
        h.harness()
            .input_mut()
            .events
            .push(egui::Event::PointerMoved(click));
        h.harness()
            .input_mut()
            .events
            .push(egui::Event::PointerButton {
                pos: click,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            });
        h.harness()
            .input_mut()
            .events
            .push(egui::Event::PointerButton {
                pos: click,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            });
        h.run_steps(2);

        h.with_app(|app| {
            assert!(
                app.parked_section_expanded,
                "shared dropdown header should toggle the parked list"
            );
        });
        h.save_screenshot("/tmp/plexi_parked_dropdown_expanded.png")
            .expect("render failed");
    }

    /// While a context is being dragged, the Parked header renders even with
    /// zero parked contexts so the drag has a drop target. Smoke test: set up
    /// an in-progress drag and confirm the sidebar renders without panicking.
    #[test]
    fn dragging_context_shows_parked_drop_target() {
        let mut h = PlexiUiHarness::new_sized(900.0, 620.0);
        h.with_app_mut(|app| {
            app.sidebar_visible = true;
            app.parked_section_expanded = false;
            // Add a second active context so there is a drag source + a list.
            app.router.push(crate::host::context::Context {
                name: "second".to_string(),
                path: std::env::temp_dir().join("second"),
                root: None,
                description: None,
                context_id: 71_001,
                parent_id: None,
                depth: 0,
                parked: false,
            });
            // Simulate an in-progress drag of the first context.
            app.drag_context = Some(0);
        });
        // Hover low in the sidebar where the (empty) Parked header renders.
        let pos = egui::pos2(28.0, 150.0);
        h.harness()
            .input_mut()
            .events
            .push(egui::Event::PointerMoved(pos));
        h.run_steps(2);
        h.with_app(|app| {
            assert_eq!(app.router.iter().filter(|c| c.parked).count(), 0);
            assert_eq!(app.drag_context, Some(0));
        });
        h.save_screenshot("/tmp/plexi_drag_park_drop_target.png")
            .expect("render failed");
    }

    /// Notes triage overlay: renders a note, and emptying the inbox returns
    /// to the picker instead of closing outright.
    #[test]
    fn notes_triage_overlay_smoke_returns_to_picker() {
        let mut h = PlexiUiHarness::new();
        h.step();
        add_focused_pane(&mut h);
        h.step();

        h.with_app_mut(|app| {
            app.notes_triage_notes = vec![crate::notes::InboxNote {
                path: std::env::temp_dir().join("plexi-ui-triage.md"),
                frontmatter: crate::notes::NoteFrontmatter::default(),
                body: "triage me\n".to_string(),
            }];
            app.notes_triage_actions = Vec::new();
            app.notes_triage_index = 0;
            app.push_focus_layer(crate::app::FocusKind::NotesTriage);
        });
        // Two steps for the egui Area sizing pass (see picker smoke test).
        h.run_steps(2);
        h.with_app(|app| {
            assert!(app
                .focus_stack
                .contains(&crate::app::FocusKind::NotesTriage));
        });
        h.harness().get_by_label("Inbox Triage (1/1)");
        h.harness().get_by_label("triage me");
        h.save_screenshot("/tmp/plexi_notes_triage.png")
            .expect("render failed");

        h.with_app_mut(|app| app.notes_triage_advance());
        h.step();
        h.with_app(|app| {
            assert!(!app
                .focus_stack
                .contains(&crate::app::FocusKind::NotesTriage));
            assert!(app
                .focus_stack
                .contains(&crate::app::FocusKind::NotesPicker));
        });
    }

    /// Render after adding a test pane.
    #[test]
    fn screenshot_with_pane() {
        let mut h = PlexiUiHarness::new();
        h.step();
        add_focused_pane(&mut h);
        h.step();
        h.save_screenshot("/tmp/plexi_with_pane.png")
            .expect("render failed");
        println!("Screenshot saved to /tmp/plexi_with_pane.png");
    }

    /// Visual review for the diffed WASM tree path (stint 0438): a real
    /// CPython-in-WASM app (breakout) opened and stepped past its first frame,
    /// so frame 2+ arrive as `tree_delta`s the decoder thread reconstructs and
    /// the paint path renders. Proves the delta pipeline paints a coherent
    /// frame, not just that the assertions pass.
    #[test]
    fn screenshot_python_app_delta_frames_paint() {
        let mut h = PlexiUiHarness::new_sized(900.0, 700.0);
        let app_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("apps/breakout");
        let pane_id = h.open_app_at(&app_dir, &[]).expect("open breakout");
        h.wait_for_app_frame(pane_id, Duration::from_secs(30))
            .expect("breakout first frame");
        // Advance host frames so the guest emits delta frames that the decoder
        // reconstructs and commits, then screenshot the painted result.
        for _ in 0..30 {
            h.step();
            std::thread::sleep(Duration::from_millis(8));
        }
        h.save_screenshot("/tmp/plexi_python_delta_frames.png")
            .expect("render delta frame");
    }

    /// Regression: zoomed app panes must render through the shared
    /// `render::app_pane::render` path (full-rect fill, no collapsing
    /// `egui::Frame`) and exercise the overtake-bar chrome. Guards against
    /// the black-square / missing-chrome zoom bugs (zoom overlay previously
    /// duplicated pane rendering inline and called `runtime.ui()` directly).
    #[test]
    fn zoomed_app_pane_renders_via_shared_path() {
        let mut h = PlexiUiHarness::new();
        let pane_id = add_focused_pane(&mut h);
        h.step();
        h.with_app_mut(|app| {
            let win = &mut app.windows[app.active_window];
            // Set overlay_replaced so the zoom path renders the overtake bar
            // ("← <replaced> / Esc return") via render::app_pane::render.
            if let Some(Pane::App(app_pane)) = win.panes.get_mut(&pane_id) {
                app_pane.overlay_replaced = Some(Box::new(Pane::Portal(Box::new(PortalPane {
                    pane_id: pane_id + 1_000,
                    target_context_id: 42,
                    context_state: None,
                    hidden: false,
                }))));
            }
            let tile_id = win
                .tree
                .tiles
                .find_pane(&pane_id)
                .expect("test pane tile missing");
            win.zoom_to(tile_id);
        });
        h.run_steps(3);
        h.render()
            .expect("zoomed app pane frame should render without panic");
        assert!(
            h.with_app(|app| app.windows[app.active_window].zoomed_pane.is_some()),
            "pane should remain zoomed after stepping frames"
        );
        h.save_screenshot("/tmp/plexi_zoomed_app_pane.png")
            .expect("render failed");
    }

    #[test]
    fn screenshot_host_ui_gallery_trust_states() {
        let mut h = PlexiUiHarness::new_sized(1280.0, 900.0);
        add_focused_pane(&mut h);
        h.step();
        h.with_app_mut(|app| {
            app.show_ui_gallery = true;
        });
        h.run_steps(2);
        h.harness().get_by_label("Code and toast surfaces");
        h.harness().get_by_label("Shell completions aren't set up.");
        h.harness().get_by_label("plexi completions zsh");
        assert!(
            h.harness()
                .get_all_by_role(egui::accesskit::Role::MultilineTextInput)
                .count()
                > 0,
            "gallery should render the host TextArea primitive"
        );
        h.save_screenshot("/tmp/plexi_host_ui_gallery_trust.png")
            .expect("render failed");
        assert!(
            h.with_app(|app| app.show_ui_gallery),
            "gallery should remain open for screenshot"
        );
        println!("Screenshot saved to /tmp/plexi_host_ui_gallery_trust.png");
    }

    #[test]
    fn screenshot_changelog_modal_uses_host_chrome() {
        let mut h = PlexiUiHarness::new_sized(1024.0, 640.0);
        add_focused_pane(&mut h);
        h.step();
        h.with_app_mut(|app| {
            app.show_changelog = true;
        });
        h.run_steps(2);

        h.harness().get_by_label("Changelog");
        assert!(
            h.harness().get_all_by_label("Changes").count() > 0,
            "changelog body should render release section labels"
        );
        h.save_screenshot("/tmp/plexi_changelog_modal.png")
            .expect("render failed");
        assert!(
            h.with_app(|app| app.show_changelog),
            "changelog should remain open for screenshot"
        );
        println!("Screenshot saved to /tmp/plexi_changelog_modal.png");
    }

    /// Host Assistant pane smoke: open → step → assert visible (epic Phase D).
    #[test]
    fn assistant_pane_opens_and_renders() {
        let ws = tempfile::tempdir().unwrap();
        let mut h = PlexiUiHarness::new_sized(1000.0, 720.0);
        h.step();
        h.open_assistant(ws.path().to_path_buf());
        h.run_steps(2);

        let assistant_panes = h.with_app(|app| {
            app.windows[app.active_window]
                .panes
                .values()
                .filter(|p| {
                    p.as_app()
                        .map(|a| a.runtime.type_id() == "assistant")
                        .unwrap_or(false)
                })
                .count()
        });
        assert_eq!(assistant_panes, 1, "assistant pane should be open");
        h.save_screenshot("/tmp/plexi_assistant_pane.png")
            .expect("render failed");
        println!("Screenshot saved to /tmp/plexi_assistant_pane.png");
    }

    /// Visual review for stints 0466/0380: a tool row whose input/output carry
    /// nested JSON strings must show decoded text (real newlines, real quotes)
    /// rather than `\n` / `\"` literals, and a shared slash-command row must
    /// render like an ordinary assistant row.
    #[test]
    fn screenshot_assistant_decoded_tool_rows_and_command_output() {
        use crate::assistant::model::{Turn, TurnRole};

        let ws = tempfile::tempdir().unwrap();
        let broker: std::sync::Arc<dyn crate::plexi_ai::broker::AiBroker> =
            std::sync::Arc::new(crate::plexi_ai::broker::LiveAiBroker::new(None));
        let mut assistant =
            crate::assistant::AssistantApp::new(ws.path().to_path_buf(), broker, ws.path(), 1);
        let row = |role: TurnRole, text: &str, at: &str| Turn {
            role,
            text: text.to_string(),
            created_at: at.to_string(),
            status: None,
            thoughts: None,
            detail: None,
            input_summary: None,
            output_preview: None,
        };
        let mut tool = row(TurnRole::Tool, "host.files.write", "2026-07-27T00:00:02Z");
        tool.status = Some(crate::assistant::model::ToolStatus::Succeeded);
        tool.input_summary =
            Some(r#"{path: notes/log.md, body: line one line two, note: he said "hi"}"#.to_string());
        tool.output_preview = Some("stdout: wrote 2 lines\nline one\nline two".to_string());

        assistant.model.turns = vec![
            row(TurnRole::User, "/context", "2026-07-27T00:00:00Z"),
            row(
                TurnRole::Command,
                "Assistant context:\n- Estimated transcript tokens: ~180 (4 turns)\n- Enabled tools: host.files.write, host.build.run",
                "2026-07-27T00:00:01Z",
            ),
            tool,
            row(
                TurnRole::Assistant,
                "Wrote both lines to `notes/log.md`.",
                "2026-07-27T00:00:03Z",
            ),
        ];

        let mut h = PlexiUiHarness::new_sized(1000.0, 720.0);
        h.step();
        h.open_assistant_built(assistant, ws.path().to_path_buf());
        h.run_steps(3);
        h.save_screenshot("/tmp/plexi_assistant_decoded_rows.png")
            .expect("render failed");
        println!(
            "Screenshot saved to /tmp/plexi_assistant_decoded_rows.png — inspect: \
             no literal \\n or \\\" anywhere, /context output reads as a normal row."
        );
    }

    /// Visual regression for stints 0435/0442: a real multi-turn conversation
    /// (short + long messages on both sides) rendered end-to-end through the
    /// actual assistant pane path, screenshotted for human/agent eyeball
    /// review — not just the isolated `draw_turn_row` geometry assertions in
    /// `assistant::render::tests`. Catches whole-conversation regressions
    /// (e.g. every row rendering identically) that a single-turn unit test
    /// cannot.
    #[test]
    fn screenshot_assistant_conversation_bubbles() {
        use crate::assistant::model::{Turn, TurnRole};

        let ws = tempfile::tempdir().unwrap();
        let broker: std::sync::Arc<dyn crate::plexi_ai::broker::AiBroker> =
            std::sync::Arc::new(crate::plexi_ai::broker::LiveAiBroker::new(None));
        let mut assistant =
            crate::assistant::AssistantApp::new(ws.path().to_path_buf(), broker, ws.path(), 1);
        assistant.model.turns = vec![
            Turn {
                role: TurnRole::User,
                text: "hi there big boii".to_string(),
                created_at: "2026-07-18T00:00:00Z".to_string(),
                status: None,
                thoughts: None,
                detail: None,
                input_summary: None,
                output_preview: None,
            },
            Turn {
                role: TurnRole::Assistant,
                text: "Hey hey! What's up? 👋".to_string(),
                created_at: "2026-07-18T00:00:01Z".to_string(),
                status: None,
                thoughts: None,
                detail: None,
                input_summary: None,
                output_preview: None,
            },
            Turn {
                role: TurnRole::User,
                text: "can you use markdown in your responses, just as an example so I can see."
                    .to_string(),
                created_at: "2026-07-18T00:00:02Z".to_string(),
                status: None,
                thoughts: None,
                detail: None,
                input_summary: None,
                output_preview: None,
            },
            Turn {
                role: TurnRole::Assistant,
                text: "Sure — here's **bold**, *italic*, and a list:\n\n- one\n- two\n- three"
                    .to_string(),
                created_at: "2026-07-18T00:00:03Z".to_string(),
                status: None,
                thoughts: None,
                detail: None,
                input_summary: None,
                output_preview: None,
            },
        ];

        let mut h = PlexiUiHarness::new_sized(1000.0, 720.0);
        h.step();
        h.open_assistant_built(assistant, ws.path().to_path_buf());
        h.run_steps(3);
        h.save_screenshot("/tmp/plexi_assistant_conversation_bubbles.png")
            .expect("render failed");
        println!(
            "Screenshot saved to /tmp/plexi_assistant_conversation_bubbles.png — \
             inspect: user bubbles right-anchored + shrink-to-fit, assistant \
             bubbles left-anchored, no row renders full-width/identical."
        );
    }

    /// Stint 0467, never-frozen rule: while a turn is in flight the
    /// transcript always shows an animated element. Two screenshots: (1) the
    /// model streamed a sentence and is now generating a tool call — the
    /// generation row (`⠹ writing host.files.write · 3.4k chars`) renders
    /// under the text; (2) same state with no generation info — the thinking
    /// dots render under the text instead of the pre-0467 frozen bubble.
    #[test]
    fn screenshot_assistant_streaming_never_frozen() {
        use crate::assistant::model::{Turn, TurnRole};

        let ws = tempfile::tempdir().unwrap();
        let broker: std::sync::Arc<dyn crate::plexi_ai::broker::AiBroker> =
            std::sync::Arc::new(crate::plexi_ai::broker::LiveAiBroker::new(None));
        let mut assistant =
            crate::assistant::AssistantApp::new(ws.path().to_path_buf(), broker, ws.path(), 1);
        assistant.model.turns = vec![Turn {
            role: TurnRole::User,
            text: "build me a tictactoe app".to_string(),
            created_at: "2026-07-19T00:00:00Z".to_string(),
            status: None,
            thoughts: None,
            detail: None,
            input_summary: None,
            output_preview: None,
        }];
        assistant.model.streaming.in_flight = true;
        assistant.model.streaming.partial_answer =
            "The SDK uses state.get() from plexi_sdk. Let me fix all the issues.".to_string();
        assistant
            .model
            .apply_tool_progress(Some("host.files.write".to_string()), 3400);

        let mut h = PlexiUiHarness::new_sized(1000.0, 720.0);
        h.step();
        h.open_assistant_built(assistant, ws.path().to_path_buf());
        h.run_steps(3);
        h.save_screenshot("/tmp/plexi_assistant_streaming_tool_progress.png")
            .expect("render failed");

        // State 2: same turn with no generation info — a fresh pane, since
        // the harness has no post-open mutable assistant accessor. The dots
        // must take over under the text.
        let broker2: std::sync::Arc<dyn crate::plexi_ai::broker::AiBroker> =
            std::sync::Arc::new(crate::plexi_ai::broker::LiveAiBroker::new(None));
        let mut assistant2 =
            crate::assistant::AssistantApp::new(ws.path().to_path_buf(), broker2, ws.path(), 1);
        assistant2.model.streaming.in_flight = true;
        assistant2.model.streaming.partial_answer =
            "The SDK uses state.get() from plexi_sdk. Let me fix all the issues.".to_string();
        let mut h2 = PlexiUiHarness::new_sized(1000.0, 720.0);
        h2.step();
        h2.open_assistant_built(assistant2, ws.path().to_path_buf());
        h2.run_steps(2);
        h2.save_screenshot("/tmp/plexi_assistant_streaming_dots_after_text.png")
            .expect("render failed");
        println!(
            "Screenshots saved to /tmp/plexi_assistant_streaming_tool_progress.png \
             (spinner + 'writing host.files.write · 3.4k chars' under the text \
             bubble) and /tmp/plexi_assistant_streaming_dots_after_text.png \
             (thinking dots under the text bubble) — neither state may render \
             as static text alone."
        );
    }

    /// Stint 0455: an app-build turn's transcript renders chronologically —
    /// user message, first assistant segment, individual caret-dropdown tool
    /// rows (never a collapsed "N tool calls" trail), a failed row in the
    /// danger hue, and the final reply as its own bubble below the tools.
    /// Screenshot is for eyeball review of the caret rows and ordering.
    #[test]
    fn screenshot_assistant_transcript_with_tool_rows() {
        use crate::assistant::model::{ToolStatus, Turn, TurnRole};

        let ws = tempfile::tempdir().unwrap();
        let broker: std::sync::Arc<dyn crate::plexi_ai::broker::AiBroker> =
            std::sync::Arc::new(crate::plexi_ai::broker::LiveAiBroker::new(None));
        let mut assistant =
            crate::assistant::AssistantApp::new(ws.path().to_path_buf(), broker, ws.path(), 1);

        let mut write_tool = Turn::tool("host.files.write", ToolStatus::Succeeded);
        write_tool.input_summary = Some("path: main.py".to_string());
        write_tool.output_preview = Some("{\n  \"ok\": true,\n  \"bytes\": 512\n}".to_string());
        write_tool.detail = Some(
            "--- a/main.py\n+++ b/main.py\n@@ -1,1 +1,2 @@\n-pass\n+print(\"hi\")\n".to_string(),
        );
        let mut check_tool = Turn::tool("plexi.app_check", ToolStatus::Succeeded);
        check_tool.input_summary = Some(r#"{"app": "guessing-game"}"#.to_string());
        check_tool.output_preview = Some(r#"{"ok": true}"#.to_string());
        let mut failed_tool = Turn::tool("host.build.run — exit 1: mypy error", ToolStatus::Failed);
        failed_tool.input_summary = Some(r#"{"cmd": "app check"}"#.to_string());
        // Multi-line output preview (stint 0460): real newlines render as a
        // framed block, so a command's output reads as the lines it printed.
        failed_tool.output_preview = Some(
            "stdout: ✓ manifest — fish-game\n✗ types — mypy found errors:\nmain.py:12: error: \"UiValueChange\" has no attribute \"payload\"\n✗ app check failed — 1 error(s)".to_string(),
        );

        assistant.model.turns = vec![
            Turn::now(
                TurnRole::User,
                "Build a simple number guessing game as a Plexi app.",
            ),
            Turn::now(
                TurnRole::Assistant,
                "Let me build this for you. Starting with scaffolding the app.",
            ),
            write_tool,
            check_tool,
            failed_tool,
            Turn::now(
                TurnRole::Assistant,
                "The Guessing Game app is open! Type a number and press Enter.",
            ),
        ];
        // An in-flight tool call so the animated running row (spinner +
        // elapsed seconds, stint 0460) is on the screenshot too.
        assistant.model.streaming.in_flight = true;
        assistant.model.tool_call_started(
            "host.build.run",
            r#"{"args": ["app", "check", "fish-game"]}"#,
        );

        let mut h = PlexiUiHarness::new_sized(1000.0, 720.0);
        h.step();
        h.open_assistant_built(assistant, ws.path().to_path_buf());
        h.run_steps(3);
        h.save_screenshot("/tmp/plexi_assistant_tool_rows.png")
            .expect("render failed");
        println!(
            "Screenshot saved to /tmp/plexi_assistant_tool_rows.png — inspect: \
             assistant segment above the tool rows, one caret row per call \
             (no 'N tool calls' summary), failed row in danger hue, final \
             reply as its own bubble below the tools."
        );
    }

    /// Stint 0451: emoji in an assistant reply must rasterize as monochrome
    /// glyphs, not tofu boxes. Before the Noto Emoji fallback was bound in
    /// `theme::font_definitions`, none of the bundled fonts covered the emoji
    /// planes, so these codepoints drew as empty boxes. Screenshot is for
    /// eyeball review — confirm each emoji draws a distinct shape.
    #[test]
    fn screenshot_assistant_emoji_render() {
        use crate::assistant::model::{Turn, TurnRole};

        let ws = tempfile::tempdir().unwrap();
        let broker: std::sync::Arc<dyn crate::plexi_ai::broker::AiBroker> =
            std::sync::Arc::new(crate::plexi_ai::broker::LiveAiBroker::new(None));
        let mut assistant =
            crate::assistant::AssistantApp::new(ws.path().to_path_buf(), broker, ws.path(), 1);
        assistant.model.turns = vec![
            Turn {
                role: TurnRole::User,
                text: "show me some emoji".to_string(),
                created_at: "2026-07-18T00:00:00Z".to_string(),
                status: None,
                thoughts: None,
                detail: None,
                input_summary: None,
                output_preview: None,
            },
            Turn {
                role: TurnRole::Assistant,
                // Exact transcript codepoints from stint 0451: U+1F44B U+1F604
                // U+1F6D2 U+2728 U+1F3AE.
                text: "Here you go: 👋 😄 🛒 ✨ 🎮".to_string(),
                created_at: "2026-07-18T00:00:01Z".to_string(),
                status: None,
                thoughts: None,
                detail: None,
                input_summary: None,
                output_preview: None,
            },
        ];

        let mut h = PlexiUiHarness::new_sized(1000.0, 720.0);
        h.step();
        h.open_assistant_built(assistant, ws.path().to_path_buf());
        h.run_steps(3);
        h.save_screenshot("/tmp/plexi_assistant_emoji_render.png")
            .expect("render failed");
        println!(
            "Screenshot saved to /tmp/plexi_assistant_emoji_render.png — \
             inspect: 👋 😄 🛒 ✨ 🎮 render as distinct monochrome glyphs, \
             not tofu boxes."
        );
    }

    #[test]
    fn semantic_label_lookup_is_scoped_to_the_target_pane() {
        let mut h = PlexiUiHarness::new_sized(1000.0, 720.0);
        let workspace = h.workspace_root().to_path_buf();
        let assistant = h
            .open_target(HarnessOpenTarget::Builtin {
                id: "assistant",
                cwd: &workspace,
                args: &[],
            })
            .expect("open Assistant");
        h.step();
        let files = h
            .open_target(HarnessOpenTarget::Builtin {
                id: "file_browser",
                cwd: &workspace,
                args: &[],
            })
            .expect("open File Browser");
        h.run_steps(2);

        // Stint 0424 removed the header bar (which used to carry "Assistant"/
        // the session name); the default footer hint bar's "commands" label
        // is the stable, assistant-only text left to scope against.
        assert!(h.pane_has_label(assistant, "commands"));
        assert!(!h.pane_has_label(files, "commands"));
    }

    #[test]
    fn native_builtin_pane_state_retains_committed_semantics() {
        let mut h = PlexiUiHarness::new_sized(1000.0, 720.0);
        let workspace = h.workspace_root().to_path_buf();
        let assistant = h
            .open_target(HarnessOpenTarget::Builtin {
                id: "assistant",
                cwd: &workspace,
                args: &[],
            })
            .expect("open Assistant");
        let files = h
            .open_target(HarnessOpenTarget::Builtin {
                id: "file_browser",
                cwd: &workspace,
                args: &[],
            })
            .expect("open File Browser");
        h.run_steps(2);

        let (assistant_state, files_state) = h.with_app(|host| {
            let state = |pane_id| {
                host.windows
                    .iter()
                    .find_map(|window| window.panes.get(&pane_id))
                    .and_then(Pane::as_app)
                    .map(|pane| pane.semantic_state())
                    .expect("builtin pane state")
            };
            (state(assistant), state(files))
        });

        assert_eq!(assistant_state.runtime_kind, "builtin");
        // Stint 0424 removed the header bar (which used to carry "Assistant"
        // as a committed semantic node); the footer hint bar's "commands"
        // label is the stable text left to check committed-semantics against.
        assert!(assistant_state.nodes.iter().any(|node| {
            node.label.as_deref() == Some("commands") || node.value.as_deref() == Some("commands")
        }));
        assert_eq!(files_state.runtime_kind, "builtin");
        assert!(!files_state.nodes.is_empty());
    }

    /// Assistant pane with a pending permission sheet renders (Phase D2).
    #[test]
    fn assistant_permission_sheet_renders() {
        let ws = tempfile::tempdir().unwrap();
        let mut h = PlexiUiHarness::new_sized(1000.0, 720.0);
        h.step();

        let broker: std::sync::Arc<dyn crate::plexi_ai::broker::AiBroker> =
            std::sync::Arc::new(crate::plexi_ai::broker::LiveAiBroker::new(None));
        let mut assistant =
            crate::assistant::AssistantApp::new(ws.path().to_path_buf(), broker, ws.path(), 1);
        assistant
            .model
            .permission_requested("csv.write_range", r#"{"range": "A1:B2"}"#);
        h.open_assistant_built(assistant, ws.path().to_path_buf());
        h.run_steps(2);

        let assistant_panes = h.with_app(|app| {
            app.windows[app.active_window]
                .panes
                .values()
                .filter(|p| {
                    p.as_app()
                        .map(|a| a.runtime.type_id() == "assistant")
                        .unwrap_or(false)
                })
                .count()
        });
        assert_eq!(assistant_panes, 1, "assistant pane should be open");
        h.save_screenshot("/tmp/plexi_assistant_permission_sheet.png")
            .expect("render failed");
        println!("Screenshot saved to /tmp/plexi_assistant_permission_sheet.png");
    }

    #[test]
    fn assistant_pane_renders_without_crash() {
        let ws = tempfile::tempdir().unwrap();
        let mut h = PlexiUiHarness::new_sized(1280.0, 900.0);
        h.open_assistant(ws.path().to_path_buf());
        h.run_steps(5);
        h.save_screenshot("/tmp/plexi-render-2216-assistant.png")
            .expect("render failed");
    }

    /// Seeded transcript (user bubble right, assistant plain left) with the
    /// slash-command picker open above the bottom-pinned composer.
    #[test]
    fn assistant_transcript_and_picker_render() {
        let ws = tempfile::tempdir().unwrap();
        let mut h = PlexiUiHarness::new_sized(1280.0, 900.0);
        h.step();

        let broker: std::sync::Arc<dyn crate::plexi_ai::broker::AiBroker> =
            std::sync::Arc::new(crate::plexi_ai::broker::LiveAiBroker::new(None));
        let mut assistant =
            crate::assistant::AssistantApp::new(ws.path().to_path_buf(), broker, ws.path(), 1);
        let turn = |role, text: &str| crate::assistant::model::Turn {
            role,
            text: text.to_string(),
            created_at: "2026-06-12T00:00:00Z".to_string(),
            status: None,
            thoughts: None,
            detail: None,
            input_summary: None,
            output_preview: None,
        };
        assistant.model.turns = vec![
            turn(
                crate::assistant::model::TurnRole::User,
                "Summarize the open PRs for this repo, please.",
            ),
            turn(
                crate::assistant::model::TurnRole::Assistant,
                "There are **3 open PRs**:\n\n\
                 - [#2224](https://github.com/x/x/pull/2224) overhauls the assistant UI\n\
                 - #2220 fixes `IPC` socket recovery\n\
                 - #2218 regenerates docs",
            ),
            turn(crate::assistant::model::TurnRole::User, "Thanks!"),
            // A tool row carrying a file-edit diff payload (stint 0421) —
            // exercises draw_diff_block through the real transcript path.
            crate::assistant::model::Turn {
                status: Some(crate::assistant::model::ToolStatus::Succeeded),
                detail: Some(
                    "--- a/main.py\n+++ b/main.py\n@@ -1,2 +1,2 @@\n-count = 0\n+count = 5\n print(count)\n"
                        .to_string(),
                ),
                ..turn(crate::assistant::model::TurnRole::Tool, "host.files.edit")
            },
        ];
        assistant.model.composer = "/".to_string();
        h.open_assistant_built(assistant, ws.path().to_path_buf());
        h.run_steps(10);
        h.save_screenshot("/tmp/plexi-render-2216-assistant-picker.png")
            .expect("render failed");
    }

    /// Seed a context-close confirm with a realistic pane inventory. When
    /// `portal_backed`, the active window gets a Portal tile targeting the
    /// context, which is what unlocks the Dissolve action (stint 0542).
    fn seed_context_close_confirm(h: &mut PlexiUiHarness, portal_backed: bool) {
        h.with_app_mut(|app| {
            let ctx_id = if portal_backed {
                let child_ctx_id = app.router.active().context_id + 4_000;
                let portal_pane_id = app.host.alloc_pane_id();
                let win = &mut app.windows[app.active_window];
                win.panes.insert(
                    portal_pane_id,
                    Pane::Portal(Box::new(PortalPane {
                        pane_id: portal_pane_id,
                        target_context_id: child_ctx_id,
                        context_state: None,
                        hidden: false,
                    })),
                );
                child_ctx_id
            } else {
                app.router.active().context_id
            };
            let mut state = app.build_context_close_state(ctx_id);
            state.context_name = "api-server".to_string();
            state.items = vec![
                crate::app::ContextCloseItem {
                    kind: "Terminal",
                    name: "cargo watch".to_string(),
                },
                crate::app::ContextCloseItem {
                    kind: "Terminal",
                    name: "psql plexi_dev".to_string(),
                },
                crate::app::ContextCloseItem {
                    kind: "App",
                    name: "File Browser".to_string(),
                },
            ];
            app.pending_context_close = Some(state);
            app.push_focus_layer(crate::app::FocusKind::ContextCloseConfirm);
        });
        // Two steps for the egui Area sizing pass.
        h.run_steps(2);
    }

    /// ActionModal defaults: shortcut chips lead every action row, all rows
    /// share the modal's left edge, and the destructive action is distinct
    /// without a filled button (stint 0542).
    #[test]
    fn screenshot_action_modal_context_close_confirm() {
        let mut h = PlexiUiHarness::new_sized(1168.0, 720.0);
        add_focused_pane(&mut h);
        h.step();
        seed_context_close_confirm(&mut h, true);
        h.save_screenshot("/tmp/plexi_action_modal_context_close.png")
            .expect("render failed");
        assert!(
            h.with_app(|app| app.pending_context_close.is_some()),
            "confirm must still be pending — nothing consumed it"
        );
        println!("Screenshot saved to /tmp/plexi_action_modal_context_close.png");
    }

    /// A top-level context has no parent portal, so the modal must render
    /// exactly two action rows — Dissolve would early-return (stint 0542).
    #[test]
    fn screenshot_action_modal_context_close_top_level_has_no_dissolve() {
        let mut h = PlexiUiHarness::new_sized(1168.0, 720.0);
        add_focused_pane(&mut h);
        h.step();
        seed_context_close_confirm(&mut h, false);
        h.save_screenshot("/tmp/plexi_action_modal_context_close_top_level.png")
            .expect("render failed");
        assert!(
            h.with_app(|app| app
                .pending_context_close
                .as_ref()
                .is_some_and(|state| !state.can_dissolve)),
            "a top-level context must not advertise Dissolve"
        );
        println!("Screenshot saved to /tmp/plexi_action_modal_context_close_top_level.png");
    }

    /// Density review surface for the modal padding tokens (stint 0543):
    /// palette, notes picker, and a confirm modal rendered with realistic
    /// seeded content at the given `pixels_per_point`.
    #[test]
    #[ignore = "density review artifact — run explicitly when tuning MODAL_PADDING_*"]
    fn screenshot_modal_density_review() {
        for ppp in [1.0_f32, 2.0] {
            let tag = if ppp == 1.0 { "1x" } else { "2x" };

            let mut h = PlexiUiHarness::new_sized_ppp(1168.0, 720.0, ppp);
            add_focused_pane(&mut h);
            h.step();
            h.with_app_mut(|app| {
                app.palette_notes = ["deploy checklist", "sprint 12 retro", "api rate limits"]
                    .iter()
                    .map(|title| crate::notes::NotePickerEntry {
                        path: std::env::temp_dir().join(format!("plexi-density-{title}.md")),
                        title: (*title).to_string(),
                        preview: format!("{title} — first body line of the note"),
                        inbox: false,
                        search_text: title.to_lowercase(),
                    })
                    .collect();
                app.show_command_palette = true;
                app.sync_command_palette_focus();
            });
            h.run_steps(3);
            h.save_screenshot(&format!("/tmp/plexi_density_palette_{tag}.png"))
                .expect("render failed");

            let mut h = PlexiUiHarness::new_sized_ppp(1168.0, 720.0, ppp);
            add_focused_pane(&mut h);
            h.step();
            h.with_app_mut(|app| {
                app.open_notes_picker();
                app.notes_picker_entries = [
                    ("meeting notes 2026-07-24", true),
                    ("deploy checklist", false),
                    ("sprint 12 retro", false),
                ]
                .iter()
                .map(|(title, inbox)| crate::notes::NotePickerEntry {
                    path: std::env::temp_dir().join(format!("plexi-density-{title}.md")),
                    title: (*title).to_string(),
                    preview: format!("{title} — first body line of the note"),
                    inbox: *inbox,
                    search_text: title.to_lowercase(),
                })
                .collect();
                app.notes_picker_selected = 0;
            });
            h.run_steps(3);
            h.save_screenshot(&format!("/tmp/plexi_density_picker_{tag}.png"))
                .expect("render failed");

            let mut h = PlexiUiHarness::new_sized_ppp(1168.0, 720.0, ppp);
            add_focused_pane(&mut h);
            h.step();
            seed_context_close_confirm(&mut h, true);
            h.save_screenshot(&format!("/tmp/plexi_density_confirm_{tag}.png"))
                .expect("render failed");
        }
    }

    #[test]
    fn screenshot_command_palette_metadata_lane() {
        let mut h = PlexiUiHarness::new_sized(1168.0, 720.0);
        let first_pane_id = add_focused_pane(&mut h);
        let browser_root = h.workspace.path().to_path_buf();
        h.with_app_mut(|app| {
            let second_pane_id = app.host.alloc_pane_id();
            let app_pane = AppPane {
                pip_status: None,
                id: second_pane_id,
                runtime: AppRuntime::Builtin(Box::new(crate::file_browser::FileBrowserApp::new(
                    browser_root.clone(),
                ))),
                workspace_root: browser_root.clone(),
                permissions: AppPermissions::builtin(),
                manifest_id: "hidden-test".to_string(),
                name: "Hidden Test App".to_string(),
                pane_group: None,
                linked_pane_id: None,
                overlay_replaced: None,
                hidden: true,
                agent: None,
                slots: std::collections::HashMap::new(),
                semantic_state: Default::default(),
            };
            let win = &mut app.windows[app.active_window];
            win.panes
                .insert(second_pane_id, Pane::App(Box::new(app_pane)));
            let second_tile = win.tree.tiles.insert_pane(second_pane_id);
            if let Some(first_tile) = win.tree.root {
                let root = win
                    .tree
                    .tiles
                    .insert_horizontal_tile(vec![first_tile, second_tile]);
                win.tree.root = Some(root);
                win.focused_pane = Some(first_tile);
            }
            let ctx_idx = app.router.active_idx();
            app.router.get_mut(ctx_idx).name = "Command Palette Metadata".to_string();
            let ctx_id = app.router.get(ctx_idx).context_id;
            let extra_a = app.host.alloc_pane_id();
            let extra_b = app.host.alloc_pane_id();
            let mut extra_panes = std::collections::HashMap::new();
            extra_panes.insert(
                extra_a,
                Pane::Portal(Box::new(PortalPane {
                    pane_id: extra_a,
                    target_context_id: extra_a + 10_000,
                    context_state: None,
                    hidden: false,
                })),
            );
            extra_panes.insert(
                extra_b,
                Pane::Portal(Box::new(PortalPane {
                    pane_id: extra_b,
                    target_context_id: extra_b + 10_000,
                    context_state: None,
                    hidden: true,
                })),
            );
            let mut extra_tiles = egui_tiles::Tiles::default();
            let extra_tile_a = extra_tiles.insert_pane(extra_a);
            let extra_tile_b = extra_tiles.insert_pane(extra_b);
            let extra_root = extra_tiles.insert_horizontal_tile(vec![extra_tile_a, extra_tile_b]);
            let extra_window_id = app.next_window_id;
            app.next_window_id += 1;
            app.windows.push(Window {
                name: "palette metadata extra".to_string(),
                path: std::env::temp_dir(),
                tree: egui_tiles::Tree::new("palette metadata extra", extra_root, extra_tiles),
                panes: extra_panes,
                focused_pane: Some(extra_tile_a),
                zoomed_pane: None,
                grid_x: 1,
                grid_y: 0,
                window_id: extra_window_id,
                context_id: ctx_id,
            });
            app.show_command_palette = true;
            app.sync_command_palette_focus();
        });
        h.run_steps(3);
        h.save_screenshot("/tmp/plexi_command_palette_metadata_lane.png")
            .expect("render failed");
        assert!(
            h.with_app(|app| app.show_command_palette
                && app.windows[app.active_window]
                    .panes
                    .contains_key(&first_pane_id)),
            "command palette should remain open over the populated context"
        );
        println!("Screenshot saved to /tmp/plexi_command_palette_metadata_lane.png");
    }

    #[test]
    fn screenshot_catppuccin_latte_command_palette() {
        let mut h = PlexiUiHarness::new_sized(1168.0, 720.0);
        add_focused_pane(&mut h);
        h.with_app_mut(|app| {
            let cfg = crate::ui::theme::preset_colors("catppuccin-latte")
                .expect("catppuccin latte preset must exist");
            app.colors = crate::ui::theme::Colors::from_config(&cfg);
            app.theme = crate::ui::theme::terminal_theme(&cfg);
            crate::ui::theme::setup_style(&app.ctx, &app.colors, false);
            app.show_command_palette = true;
            app.sync_command_palette_focus();
        });
        h.run_steps(3);
        let img = h.render().expect("render failed");
        img.save("/tmp/plexi_catppuccin_latte_palette.png")
            .expect("save screenshot");
        // Probe the blank right gutter inside the modal. The image center can
        // land on a command description as the registered app list changes.
        let surface = img.get_pixel(img.width() * 3 / 4, img.height() / 2).0;
        let center_luma =
            (u16::from(surface[0]) + u16::from(surface[1]) + u16::from(surface[2])) / 3;
        assert!(
            center_luma > 180,
            "Catppuccin Latte palette surface should be light, got rgba={surface:?}"
        );
        println!("Screenshot saved to /tmp/plexi_catppuccin_latte_palette.png");
    }

    #[test]
    fn screenshot_command_palette_user_commands() {
        let mut h = PlexiUiHarness::new_sized(1168.0, 1000.0);
        add_focused_pane(&mut h);
        h.with_app_mut(|app| {
            app.palette_commands = vec![
                crate::cli::ResolvedUserCommand {
                    name: "build".to_string(),
                    run: Some("cargo build".to_string()),
                    description: Some("Build the project".to_string()),
                    scope: crate::cli::UserCommandScope::Workspace,
                },
                crate::cli::ResolvedUserCommand {
                    name: "test".to_string(),
                    run: Some("cargo test".to_string()),
                    description: None,
                    scope: crate::cli::UserCommandScope::Workspace,
                },
                crate::cli::ResolvedUserCommand {
                    name: "deploy".to_string(),
                    run: None,
                    description: None,
                    scope: crate::cli::UserCommandScope::Global,
                },
            ];
            app.show_command_palette = true;
            app.palette_selected = 0;
            app.sync_command_palette_focus();
        });
        h.run_steps(3);
        h.save_screenshot("/tmp/plexi_command_palette_user_commands.png")
            .expect("render failed");
        assert!(
            h.with_app(|app| app.show_command_palette && app.palette_commands.len() == 3),
            "palette should be open with the injected user commands"
        );
        println!("Screenshot saved to /tmp/plexi_command_palette_user_commands.png");
    }

    #[test]
    fn command_palette_keeps_search_focus_after_scrolled_down_nav() {
        let mut h = PlexiUiHarness::new_sized(1168.0, 720.0);
        add_focused_pane(&mut h);
        h.with_app_mut(|app| {
            app.palette_notes = (0..40)
                .map(|idx| crate::notes::NotePickerEntry {
                    path: std::env::temp_dir().join(format!("palette-note-{idx}.md")),
                    title: format!("Palette note {idx:02}"),
                    preview: "Deterministic row for palette scrolling".to_string(),
                    inbox: false,
                    search_text: format!("palette note {idx:02} deterministic row"),
                })
                .collect();
            app.show_command_palette = true;
            app.palette_selected = 0;
            app.sync_command_palette_focus();
        });

        h.run_steps(2);
        for _ in 0..18 {
            h.harness().key_press(egui::Key::ArrowDown);
            h.step();
        }
        h.run_steps(2);

        let (open, selected, focused) = h.with_app(|app| {
            (
                app.show_command_palette,
                app.palette_selected,
                app.ctx.memory(|m| m.focused()),
            )
        });
        assert!(
            open,
            "command palette should remain open during keyboard nav"
        );
        assert!(
            selected >= 18,
            "ArrowDown should advance selection into the scrolled list"
        );
        assert_eq!(
            focused,
            Some(egui::Id::new("palette_search")),
            "palette search must keep focus after scrolling down past the first page"
        );
    }

    // ── Regression: layout flows ──────────────────────────────────────────────

    #[test]
    fn split_vertical_adds_pane() {
        let mut h = PlexiUiHarness::new();
        h.step();
        add_focused_pane(&mut h);
        h.step();
        assert_eq!(h.pane_count(), 1);

        h.with_app_mut(|app| app.split_focused(true, None, false, false, None));
        h.step();

        assert_eq!(h.pane_count(), 2);
    }

    #[test]
    fn split_horizontal_adds_pane() {
        let mut h = PlexiUiHarness::new();
        h.step();
        add_focused_pane(&mut h);
        h.step();
        assert_eq!(h.pane_count(), 1);

        h.with_app_mut(|app| app.split_focused(false, None, false, false, None));
        h.step();

        assert_eq!(h.pane_count(), 2);
    }

    #[test]
    fn new_context_adds_window() {
        let mut h = PlexiUiHarness::new();
        h.step();
        assert_eq!(h.window_count(), 1);

        h.with_app_mut(|app| app.new_context());
        h.step();

        assert_eq!(h.window_count(), 2);
    }

    // ── Rename flow tests (require a real egui frame to process key events) ────

    /// Full rename flow: set up rename state → press Enter → verify commit.
    ///
    /// The rename commit happens inside `draw_rename_context_overlay`, which reads
    /// Enter from `ctx.input_mut()` during the egui draw pass. This cannot be
    /// tested with HostHarness — it requires PlexiUiHarness + a real frame.
    #[test]
    fn context_rename_commits_on_enter() {
        let mut h = PlexiUiHarness::new();
        h.step();

        // Simulate what Action::RenameContext does: populate rename state and push focus layer.
        h.with_app_mut(|app| {
            let ctx_idx = app.router.active_idx();
            app.rename_buffer = "My Project".to_string();
            app.renaming_window = Some(ctx_idx);
            app.focus_stack.push(FocusKind::ContextRename);
        });

        // Queue Enter for the next frame — processed by draw_rename_context_overlay.
        h.harness().key_press(egui::Key::Enter);
        h.step();

        let (name, still_renaming) =
            h.with_app(|app| (app.router.active().name.clone(), app.renaming_window));
        assert_eq!(name, "My Project");
        assert!(
            still_renaming.is_none(),
            "renaming_window must be cleared after commit"
        );
    }

    /// Escape discards the rename buffer; the context name must be unchanged.
    #[test]
    fn context_rename_cancels_on_escape() {
        let mut h = PlexiUiHarness::new();
        h.step();

        let original_name = h.with_app(|app| app.router.active().name.clone());

        h.with_app_mut(|app| {
            let ctx_idx = app.router.active_idx();
            app.rename_buffer = "Discarded Name".to_string();
            app.renaming_window = Some(ctx_idx);
            app.focus_stack.push(FocusKind::ContextRename);
        });

        h.harness().key_press(egui::Key::Escape);
        h.step();

        let (name, still_renaming) =
            h.with_app(|app| (app.router.active().name.clone(), app.renaming_window));
        assert_eq!(name, original_name, "name must be unchanged after Escape");
        assert!(
            still_renaming.is_none(),
            "renaming_window must be cleared after Escape"
        );
    }

    /// Empty rename buffer must not overwrite the existing name.
    #[test]
    fn context_rename_ignores_empty_buffer() {
        let mut h = PlexiUiHarness::new();
        h.step();

        let original_name = h.with_app(|app| app.router.active().name.clone());

        h.with_app_mut(|app| {
            let ctx_idx = app.router.active_idx();
            app.rename_buffer = "   ".to_string(); // whitespace-only trims to empty
            app.renaming_window = Some(ctx_idx);
            app.focus_stack.push(FocusKind::ContextRename);
        });

        h.harness().key_press(egui::Key::Enter);
        h.step();

        let name = h.with_app(|app| app.router.active().name.clone());
        assert_eq!(
            name, original_name,
            "whitespace-only rename must be discarded"
        );
    }

    /// Regression (stint 0387 follow-up): the inline sidebar context-rename
    /// TextEdit must receive typed characters even while a terminal pane is the
    /// focused tile. The terminal-input ownership transfer
    /// (`take_focused_terminal_input`) runs before the render pass and, before
    /// the fix, stole every Key/Text event out of `ctx` whenever the focused
    /// tile was a terminal — starving the sidebar rename box (which is not a
    /// focus layer when the sidebar is visible). With the fix, the steal is
    /// gated on egui keyboard focus, so the box (which holds egui focus) keeps
    /// its keystrokes.
    #[test]
    fn sidebar_inline_rename_receives_text_with_focused_terminal() {
        let mut h = PlexiUiHarness::new_sized(1000.0, 700.0);
        h.step();

        // Need a real focused terminal pane. Seed an app pane, then split — the
        // split spawns a real TerminalPane and focuses it.
        add_focused_pane(&mut h);
        h.step();
        h.with_app_mut(|app| app.split_focused(true, None, false, false, None));
        h.run_steps(2);

        let focused_is_terminal = h.with_app(|app| {
            let win = &app.windows[app.active_window];
            win.focused_pane
                .and_then(|t| match win.tree.tiles.get(t) {
                    Some(egui_tiles::Tile::Pane(pid)) => win.panes.get(pid),
                    _ => None,
                })
                .map(|p| matches!(p, Pane::Terminal(_)))
                .unwrap_or(false)
        });
        assert!(
            focused_is_terminal,
            "focused pane must be a real terminal for this regression to be meaningful"
        );

        // Enter inline sidebar rename: sidebar visible → sync_context_rename_focus
        // leaves should_own=false, so NO ContextRename focus layer is pushed and
        // the inline TextEdit (not an overlay) owns keyboard input.
        h.with_app_mut(|app| {
            app.sidebar_visible = true;
            let ctx_idx = app.router.active_idx();
            app.rename_buffer.clear();
            app.renaming_window = Some(ctx_idx);
        });
        // Let the TextEdit mount and gain egui focus: the reconciler (stint
        // 0429) derives InputOwner::Overlay(SidebarRename) and grants the
        // rename box focus, surrendering the terminal's surface.
        h.run_steps(3);

        // Type into the rename box.
        h.harness()
            .input_mut()
            .events
            .push(egui::Event::Text("x".to_string()));
        h.step();
        h.harness()
            .input_mut()
            .events
            .push(egui::Event::Text("y".to_string()));
        h.step();

        let buffer = h.with_app(|app| app.rename_buffer.clone());
        assert_eq!(
            buffer, "xy",
            "inline sidebar rename box must receive typed characters even while a terminal is the focused tile"
        );
    }

    /// Regression: tab titles with long names must stay on a single line and show
    /// a trailing ellipsis rather than wrapping or overflowing the tab bar rect.
    #[test]
    fn tab_titles_truncate_to_single_line() {
        let mut h = PlexiUiHarness::new();
        h.step();

        let browser_root = h.workspace.path().to_path_buf();
        h.with_app_mut(|app| {
            let pane_id_a = app.host.alloc_pane_id();
            let pane_id_b = app.host.alloc_pane_id();
            for (pane_id, name) in [
                (pane_id_a, "A very long tab title that should not wrap onto a second line under any circumstances"),
                (pane_id_b, "Short"),
            ] {
                let app_pane = AppPane {
                    pip_status: None,
                    id: pane_id,
                    runtime: AppRuntime::Builtin(Box::new(
                        crate::file_browser::FileBrowserApp::new(browser_root.clone()),
                    )),
                    workspace_root: browser_root.clone(),
                    permissions: AppPermissions::builtin(),
                    manifest_id: "test".to_string(),
                    name: name.to_string(),
                    pane_group: None,
                    linked_pane_id: None,
                    overlay_replaced: None,
                    hidden: false,
                    agent: None,
                    slots: std::collections::HashMap::new(),
                    semantic_state: Default::default(),
                };
                let win = &mut app.windows[app.active_window];
                win.panes.insert(pane_id, Pane::App(Box::new(app_pane)));
            }
            let win = &mut app.windows[app.active_window];
            let tile_a = win.tree.tiles.insert_pane(pane_id_a);
            let tile_b = win.tree.tiles.insert_pane(pane_id_b);
            let tab_tile = win.tree.tiles.insert_tab_tile(vec![tile_a, tile_b]);
            win.tree.root = Some(tab_tile);
            win.focused_pane = Some(tile_a);
        });

        h.run_steps(2);
        h.render()
            .expect("tab bar with long title must render without panic");
        h.save_screenshot("/tmp/plexi_tab_truncation.png")
            .expect("screenshot failed");
        println!("Screenshot saved to /tmp/plexi_tab_truncation.png");
        // Visual: open /tmp/plexi_tab_truncation.png and confirm the long label
        // ends with '…' on a single line and does not push the tab bar taller.
    }

    // ── s8 pixel-grid + typography evidence (stints 0527–0530) ──────────────

    /// Seeds a Notes pane with a multi-line document and screenshots it at the
    /// given display scale. The ppp-2.0 capture is the one that shows editor
    /// pixel-grid regressions (uneven leading, off-grid rows).
    fn editor_rows_evidence(ppp: f32, out: &str) {
        let mut h = PlexiUiHarness::new_sized_ppp(760.0, 520.0, ppp);
        h.step();

        let path = std::env::temp_dir().join(format!(
            "plexi-ui-evidence-rows-{}-{}.md",
            (ppp * 10.0) as u32,
            std::process::id()
        ));
        let mut body = String::from("---\ntitle: \"Pixel Grid\"\n---\n");
        for i in 1..=26 {
            body.push_str(&format!(
                "Line {i:02}: the quick brown fox jumps over the lazy dog 0123456789\n"
            ));
        }
        std::fs::write(&path, &body).expect("seed note");

        h.with_app_mut(|app| {
            let pane_id = app.host.alloc_pane_id();
            let app_pane = AppPane {
                pip_status: None,
                id: pane_id,
                runtime: AppRuntime::Builtin(Box::new(
                    crate::app::text_editor_app::TextEditorApp::new_for_test_note(path.clone()),
                )),
                workspace_root: std::env::temp_dir(),
                permissions: AppPermissions::builtin(),
                manifest_id: "text-editor".to_string(),
                name: "Text Editor".to_string(),
                pane_group: None,
                linked_pane_id: None,
                overlay_replaced: None,
                hidden: false,
                agent: None,
                slots: std::collections::HashMap::new(),
                semantic_state: Default::default(),
            };
            let win = &mut app.windows[app.active_window];
            win.panes.insert(pane_id, Pane::App(Box::new(app_pane)));
            let tile_id = win.tree.tiles.insert_pane(pane_id);
            if win.tree.root.is_none() {
                win.tree.root = Some(tile_id);
            }
            win.focused_pane = Some(tile_id);
        });
        h.run_steps(3);
        h.save_screenshot(out).expect("render failed");
        println!("Screenshot saved to {out}");

        let _ = std::fs::remove_file(&path);
    }

    /// Stint 0529 evidence: editor rows at ppp 1.0 — every painted row top on
    /// the pixel grid with uniform leading.
    #[test]
    fn screenshot_editor_rows_pixel_grid_ppp1() {
        editor_rows_evidence(1.0, "/tmp/plexi-0529-editor-rows-ppp1.png");
    }

    /// Stint 0529 evidence: same document at ppp 2.0 (the HiDPI case that
    /// used to shimmer during scroll and show 17/17/16/17 leading).
    #[test]
    fn screenshot_editor_rows_pixel_grid_ppp2() {
        editor_rows_evidence(2.0, "/tmp/plexi-0529-editor-rows-ppp2.png");
    }

    /// Stints 0528/0530 evidence: sidebar with seeded contexts (active +
    /// inactive names, path subtitles) and the spatial minimap overlay with a
    /// workspace name, at ppp 2.0. Review: inactive context names must meet
    /// the contrast floor, minimap name galley must sit on the pixel grid.
    #[test]
    fn screenshot_sidebar_and_minimap_typography_ppp2() {
        let mut h = PlexiUiHarness::new_sized_ppp(1100.0, 700.0, 2.0);
        h.step();
        add_focused_pane(&mut h);
        let inactive_ctx_id = h.with_app_mut(|app| {
            app.sidebar_visible = true;
            // The minimap overlay only draws when the active workspace has a
            // second window — seed an empty one on the grid.
            let ws_id = app.router.active().context_id;
            let win_id = app.next_window_id;
            app.next_window_id += 1;
            app.windows.push(crate::host::context::Window {
                name: String::new(),
                path: std::env::temp_dir(),
                tree: egui_tiles::Tree::empty("plexi"),
                panes: std::collections::HashMap::new(),
                focused_pane: None,
                zoomed_pane: None,
                grid_x: 1,
                grid_y: 0,
                window_id: win_id,
                context_id: ws_id,
            });
            app.minimap = crate::render::minimap::MinimapState::with_visible(true);
            // Two extra inactive contexts with realistic names + root paths.
            // The first has a persisted return window so the sidebar screenshot
            // covers its inactive focus capsule + pane pin.
            let plexi_ctx_id = app.next_window_id;
            app.next_window_id += 1;
            let plexi_root = std::env::temp_dir().join("GitHub/PLEXI");
            app.router.push(crate::host::context::Context {
                name: "PLEXI".to_string(),
                path: plexi_root.clone(),
                root: Some(plexi_root.clone()),
                description: None,
                context_id: plexi_ctx_id,
                parent_id: None,
                depth: 0,
                parked: false,
            });
            let plexi_window_id = app.next_window_id;
            app.next_window_id += 1;
            app.windows.push(crate::host::context::Window {
                name: String::new(),
                path: plexi_root,
                tree: egui_tiles::Tree::empty("plexi-sidebar-focus-preview"),
                panes: std::collections::HashMap::new(),
                focused_pane: None,
                zoomed_pane: None,
                grid_x: 0,
                grid_y: 0,
                window_id: plexi_window_id,
                context_id: plexi_ctx_id,
            });
            app.context_active_window
                .insert(plexi_ctx_id, plexi_window_id);
            for (name, dir) in [("videos", "Documents/videos")] {
                let ctx_id = app.next_window_id;
                app.next_window_id += 1;
                let root = std::env::temp_dir().join(dir);
                app.router.push(crate::host::context::Context {
                    name: name.to_string(),
                    path: root.clone(),
                    root: Some(root),
                    description: None,
                    context_id: ctx_id,
                    parent_id: None,
                    depth: 0,
                    parked: false,
                });
            }
            plexi_ctx_id
        });
        // Index 2 is the inactive PLEXI context's only window: indexes 0 and
        // 1 are the active context's two spatial windows seeded above.
        add_focused_pane_to_window(&mut h, 2);
        h.with_app_mut(|app| {
            assert_eq!(app.windows[2].context_id, inactive_ctx_id);
        });
        h.run_steps(3);
        h.save_screenshot("/tmp/plexi-0528-sidebar-minimap-ppp2.png")
            .expect("render failed");
        println!("Screenshot saved to /tmp/plexi-0528-sidebar-minimap-ppp2.png");
    }

    /// Stint 0528 evidence: file browser rows + preview panel at ppp 2.0 —
    /// integer point sizes only (was 9.5/10.5).
    #[test]
    fn screenshot_file_browser_typography_ppp2() {
        let dir = tempfile::tempdir().expect("seed dir");
        for name in ["notes.md", "render.rs", "todo.txt"] {
            std::fs::write(
                dir.path().join(name),
                "alpha\nbravo\ncharlie\ndelta\necho\n",
            )
            .expect("seed file");
        }
        std::fs::create_dir(dir.path().join("assets")).expect("seed subdir");

        let mut h = PlexiUiHarness::new_sized_ppp(900.0, 560.0, 2.0);
        h.step();
        h.with_app_mut(|app| {
            let pane_id = app.host.alloc_pane_id();
            let app_pane = AppPane {
                pip_status: None,
                id: pane_id,
                runtime: AppRuntime::Builtin(Box::new(crate::file_browser::FileBrowserApp::new(
                    dir.path().to_path_buf(),
                ))),
                workspace_root: dir.path().to_path_buf(),
                permissions: AppPermissions::builtin(),
                manifest_id: "file-browser".to_string(),
                name: "Files".to_string(),
                pane_group: None,
                linked_pane_id: None,
                overlay_replaced: None,
                hidden: false,
                agent: None,
                slots: std::collections::HashMap::new(),
                semantic_state: Default::default(),
            };
            let win = &mut app.windows[app.active_window];
            win.panes.insert(pane_id, Pane::App(Box::new(app_pane)));
            let tile_id = win.tree.tiles.insert_pane(pane_id);
            if win.tree.root.is_none() {
                win.tree.root = Some(tile_id);
            }
            win.focused_pane = Some(tile_id);
        });
        h.run_steps(3);
        h.save_screenshot("/tmp/plexi-0528-filebrowser-ppp2.png")
            .expect("render failed");
        println!("Screenshot saved to /tmp/plexi-0528-filebrowser-ppp2.png");
    }

    // ── Text-crispness gate (stint 0531) ────────────────────────────────────
    //
    // Freezes the s8 pixel-grid + typography wins (stints 0527–0530) behind one
    // runnable gate. Two halves, per `src/testing/TESTING.md`:
    //
    //   1. Seeded renders of the key text surfaces at BOTH display scales
    //      (ppp 1.0 and 2.0) — a human/agent Reads the PNGs to confirm the
    //      pixels look crisp, then deletes them. Non-scope: no golden-image
    //      diffing (brittle across font-raster changes).
    //   2. Mechanical assertions on the composited host state that stay green
    //      without a human: app-surface texture dims scale with ppp (0527),
    //      the editor's row metric is integer-physical after a real render
    //      (0529), chrome labels clear the WCAG AA contrast floor (0528), and
    //      hand-painted chrome galley origins land on the physical pixel grid
    //      (0530). Each calls the production code path — it never re-implements
    //      the invariant.
    //
    // The `paint_app_bar` title+subtitle band is painted only from
    // `wasm_render`; rather than depend on a fixture app declaring app-chrome,
    // its two crispness invariants are frozen mechanically below — the subtitle
    // color clears the contrast floor on `bg_toolbar`, and its `font_medium`
    // galleys snap to the grid.
    mod crispness_gate {
        use super::*;
        use crate::app::permissions::{PermissionState, PermissionStore};
        use crate::app::text_editor_app::TextEditorApp;
        use crate::host::wasm_app::{StateSnapshot, StateStore, WasmApp};
        use crate::host::wasm_pane::{SysinfoStats, WasmPane};

        /// The two display scales every text surface must stay crisp at: 1.0
        /// (integer) and 2.0 (HiDPI, where off-grid galleys and half-resolution
        /// app surfaces surface as fuzz).
        const GATE_PPP: [f32; 2] = [1.0, 2.0];

        fn wasm_fixture(name: &str) -> std::path::PathBuf {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/wasm-fixtures")
                .join(name)
        }

        /// Reach the first `text-editor` pane in any window and run `f` against
        /// it. Returns `None` if no editor pane is open.
        fn with_editor<R>(
            h: &mut PlexiUiHarness,
            f: impl FnOnce(&mut TextEditorApp) -> R,
        ) -> Option<R> {
            let mut f = Some(f);
            h.with_app_mut(|app| {
                for win in &mut app.windows {
                    for pane in win.panes.values_mut() {
                        if let Pane::App(a) = pane {
                            if let AppRuntime::Builtin(b) = &mut a.runtime {
                                if let Some(ed) = b.as_any_mut().downcast_mut::<TextEditorApp>() {
                                    return Some((f.take().expect("f consumed once"))(ed));
                                }
                            }
                        }
                    }
                }
                None
            })
        }

        // ── Seeding ─────────────────────────────────────────────────────────

        /// Sidebar shown with several long context names + path subtitles —
        /// the surface where `text_secondary` context labels and their galley
        /// snapping are most visible.
        fn seed_sidebar(h: &mut PlexiUiHarness) {
            add_focused_pane(h);
            h.with_app_mut(|app| {
                app.sidebar_visible = true;
                for (name, dir) in [
                    ("Very Long Project Context Name", "GitHub/PLEXI/src/render"),
                    ("videos", "Documents/videos/2026/exports"),
                    ("scratch", "tmp/scratch/wip"),
                ] {
                    let ctx_id = app.next_window_id;
                    app.next_window_id += 1;
                    let root = std::env::temp_dir().join(dir);
                    app.router.push(crate::host::context::Context {
                        name: name.to_string(),
                        path: root.clone(),
                        root: Some(root),
                        description: None,
                        context_id: ctx_id,
                        parent_id: None,
                        depth: 0,
                        parked: false,
                    });
                }
            });
        }

        /// A Notes pane holding a multi-line document, parked at a fractional
        /// mid-scroll offset so unsnapped rows would show off-grid.
        fn seed_editor_mid_scroll(h: &mut PlexiUiHarness) {
            let path = h.workspace.path().join("crispness-note.md");
            let mut body = String::from("---\ntitle: \"Pixel Grid\"\n---\n");
            for i in 1..=60 {
                body.push_str(&format!(
                    "Line {i:02}: the quick brown fox jumps over the lazy dog 0123456789\n"
                ));
            }
            std::fs::write(&path, &body).expect("seed note");
            h.with_app_mut(|app| {
                let pane_id = app.host.alloc_pane_id();
                let app_pane = AppPane {
                    pip_status: None,
                    id: pane_id,
                    runtime: AppRuntime::Builtin(Box::new(TextEditorApp::new_for_test_note(
                        path.clone(),
                    ))),
                    workspace_root: std::env::temp_dir(),
                    permissions: AppPermissions::builtin(),
                    manifest_id: "text-editor".to_string(),
                    name: "Text Editor".to_string(),
                    pane_group: None,
                    linked_pane_id: None,
                    overlay_replaced: None,
                    hidden: false,
                    agent: None,
                    slots: std::collections::HashMap::new(),
                    semantic_state: Default::default(),
                };
                let win = &mut app.windows[app.active_window];
                win.panes.insert(pane_id, Pane::App(Box::new(app_pane)));
                let tile_id = win.tree.tiles.insert_pane(pane_id);
                if win.tree.root.is_none() {
                    win.tree.root = Some(tile_id);
                }
                win.focused_pane = Some(tile_id);
            });
            // One pass so the widget lays out (sets line_height / viewport_height),
            // then park at a fractional mid-document offset and render again.
            h.run_steps(2);
            with_editor(h, |ed| {
                let line_height = ed.test_view_line_height().max(1.0);
                ed.test_set_scroll_y(line_height * 18.0 + 0.5);
            });
        }

        /// A file browser rooted at a small seeded dir (never `temp_dir()`,
        /// whose dev-machine entry count balloons rendering).
        fn seed_file_browser(h: &mut PlexiUiHarness) {
            let root = h.workspace.path().join("files");
            std::fs::create_dir_all(&root).expect("seed dir");
            for name in ["notes.md", "render.rs", "todo.txt", "README.md"] {
                std::fs::write(root.join(name), "alpha\nbravo\ncharlie\ndelta\necho\n")
                    .expect("seed file");
            }
            std::fs::create_dir_all(root.join("assets")).expect("seed subdir");
            h.with_app_mut(|app| {
                let pane_id = app.host.alloc_pane_id();
                let app_pane = AppPane {
                    pip_status: None,
                    id: pane_id,
                    runtime: AppRuntime::Builtin(Box::new(
                        crate::file_browser::FileBrowserApp::new(root.clone()),
                    )),
                    workspace_root: root.clone(),
                    permissions: AppPermissions::builtin(),
                    manifest_id: "file-browser".to_string(),
                    name: "Files".to_string(),
                    pane_group: None,
                    linked_pane_id: None,
                    overlay_replaced: None,
                    hidden: false,
                    agent: None,
                    slots: std::collections::HashMap::new(),
                    semantic_state: Default::default(),
                };
                let win = &mut app.windows[app.active_window];
                win.panes.insert(pane_id, Pane::App(Box::new(app_pane)));
                let tile_id = win.tree.tiles.insert_pane(pane_id);
                if win.tree.root.is_none() {
                    win.tree.root = Some(tile_id);
                }
                win.focused_pane = Some(tile_id);
            });
        }

        /// Launch the sysmon WASM fixture through the reviewed-path production
        /// launcher: text-heavy L1 body plus its `paint_app_bar` title band.
        /// Grants are pre-persisted into the harness profile so the review
        /// auto-approves. The returned guard must outlive the harness.
        fn seed_wasm_text(h: &mut PlexiUiHarness, profile: &std::path::Path) {
            let fixture = wasm_fixture("sysmon.wasm");
            let app_id = fixture
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("wasm");
            let workspace_root = fixture.parent().expect("fixture parent");
            let grants = WasmApp::inspect_required_grants(&fixture).expect("inspect grants");
            let mut store = PermissionStore::load_or_default(profile);
            for capability_id in grants.capability_ids() {
                store.set_wasm(
                    app_id,
                    workspace_root,
                    &capability_id,
                    PermissionState::Green,
                );
            }
            store.save();
            h.with_app_mut(|app| {
                app.launch_app_by_path_with_layout(
                    &fixture.to_string_lossy(),
                    Some("split_v".to_string()),
                    None,
                    &[],
                )
            })
            .expect("reviewed wasm launch should succeed");
        }

        // ── Rendered fixtures at ppp 1.0 and 2.0 (visual review) ────────────

        #[test]
        fn gate_render_sidebar_ppp_1_and_2() {
            for ppp in GATE_PPP {
                let mut h = PlexiUiHarness::new_sized_ppp(1100.0, 700.0, ppp);
                h.step();
                seed_sidebar(&mut h);
                h.run_steps(3);
                let out = format!("/tmp/plexi-0531-sidebar-ppp{}.png", (ppp * 10.0) as u32);
                h.save_screenshot(&out).expect("sidebar render");
                println!("Screenshot saved to {out}");
            }
        }

        #[test]
        fn gate_render_editor_mid_scroll_ppp_1_and_2() {
            for ppp in GATE_PPP {
                let mut h = PlexiUiHarness::new_sized_ppp(820.0, 560.0, ppp);
                h.step();
                seed_editor_mid_scroll(&mut h);
                h.run_steps(3);
                let out = format!("/tmp/plexi-0531-editor-ppp{}.png", (ppp * 10.0) as u32);
                h.save_screenshot(&out).expect("editor render");
                println!("Screenshot saved to {out}");
            }
        }

        #[test]
        fn gate_render_file_browser_ppp_1_and_2() {
            for ppp in GATE_PPP {
                let mut h = PlexiUiHarness::new_sized_ppp(900.0, 560.0, ppp);
                h.step();
                seed_file_browser(&mut h);
                h.run_steps(3);
                let out = format!("/tmp/plexi-0531-filebrowser-ppp{}.png", (ppp * 10.0) as u32);
                h.save_screenshot(&out).expect("file browser render");
                println!("Screenshot saved to {out}");
            }
        }

        #[test]
        fn gate_render_wasm_text_ppp_1_and_2() {
            for ppp in GATE_PPP {
                let profile = tempfile::tempdir().expect("profile dir");
                let _guard = crate::config::set_test_profile_dir(profile.path().to_path_buf());
                let mut h = PlexiUiHarness::new_sized_ppp(900.0, 620.0, ppp);
                h.step();
                seed_wasm_text(&mut h, profile.path());
                h.run_steps(6);
                let out = format!("/tmp/plexi-0531-wasm-text-ppp{}.png", (ppp * 10.0) as u32);
                h.save_screenshot(&out).expect("wasm text render");
                println!("Screenshot saved to {out}");
            }
        }

        // ── Mechanical assertions on composited state ───────────────────────

        /// Stint 0527: a GPU app pane allocates its surface at physical
        /// resolution (`logical × ppp`), so at ppp 2.0 the texture is exactly
        /// twice the ppp-1.0 texture in each dimension — full-res on HiDPI, no
        /// bilinear upscaling. Drives the production `WasmPane` surface path.
        #[test]
        fn gate_wasm_app_surface_scales_with_ppp() {
            fn surface_at(ppp: f32) -> (u32, u32) {
                let app = WasmApp::load_ephemeral_run(
                    "pong-crispness",
                    &wasm_fixture("pong.wasm"),
                    StateStore::ephemeral(),
                )
                .expect("load pong fixture");
                let mut pane = WasmPane::new(app, Box::new(SysinfoStats::new()));
                pane.set_pixels_per_point(ppp);
                pane.init(&StateSnapshot { entries: vec![] }, (480.0, 360.0), 0, &[])
                    .expect("init pong");
                pane.surface_size().expect("surface allocated after init")
            }
            let (w1, h1) = surface_at(1.0);
            let (w2, h2) = surface_at(2.0);
            assert!(w1 > 0 && h1 > 0, "ppp-1.0 surface must be non-empty");
            assert_eq!(
                (w2, h2),
                (w1 * 2, h1 * 2),
                "surface texture must scale 1:1 with display scale \
                 (ppp1={w1}x{h1}, ppp2={w2}x{h2})"
            );
        }

        /// Stint 0529: after a real host render the editor's row metric is
        /// quantized to the physical pixel grid, so `line_height × ppp` is an
        /// integer and every painted row top is uniformly spaced — at both
        /// display scales, and mid-scroll.
        #[test]
        fn gate_editor_row_metric_is_integer_physical() {
            for ppp in GATE_PPP {
                let mut h = PlexiUiHarness::new_sized_ppp(820.0, 560.0, ppp);
                h.step();
                seed_editor_mid_scroll(&mut h);
                h.run_steps(3);
                let line_height =
                    with_editor(&mut h, |ed| ed.test_view_line_height()).expect("editor pane open");
                let phys = line_height * ppp;
                assert!(
                    line_height > 0.0 && (phys - phys.round()).abs() < 1e-2,
                    "quantized line_height {line_height} not integer-physical at ppp {ppp} \
                     (phys {phys})"
                );
                // Uniform, on-grid row tops for the metric the widget committed.
                let view = crate::editor::ViewState {
                    line_height,
                    ..crate::editor::ViewState::default()
                };
                let expected = (line_height * ppp).round();
                let mut prev = view.line_top(0) * ppp;
                for line in 1..120 {
                    let top = view.line_top(line) * ppp;
                    let spacing = top - prev;
                    assert!(
                        (spacing - expected).abs() < 1e-2,
                        "row spacing {spacing} != {expected} phys px (ppp {ppp}, line {line})"
                    );
                    prev = top;
                }
            }
        }

        /// Stint 0528: the labels the host hand-paints in `text_secondary`
        /// (sidebar context names, app-bar subtitles, pane-name bars) clear the
        /// WCAG AA 4.5:1 floor against every background they actually sit on —
        /// mirroring the contrast check apps get. Reuses the production WCAG
        /// math and the default theme.
        #[test]
        fn gate_chrome_labels_meet_wcag_aa_floor() {
            use crate::ui::theme::{contrast, luminance};
            let colors =
                crate::ui::theme::colors_from_config(&crate::config::PlexiConfig::default());
            let backgrounds = [
                ("bg_sidebar", colors.bg_sidebar),
                ("bg_toolbar", colors.bg_toolbar),
                ("pane_header_bg", colors.pane_header_bg()),
                ("bg_darkest", colors.bg_darkest),
            ];
            for (name, bg) in backgrounds {
                let fg = colors.text_secondary(bg);
                let ratio = contrast(luminance(fg), luminance(bg));
                assert!(
                    ratio >= 4.5,
                    "chrome label on {name} has contrast {ratio:.2} < 4.5 AA floor"
                );
            }
        }

        /// Stint 0530: the hand-painted-text snapping helper lands every galley
        /// origin on the physical pixel grid, for the whole UI text size range
        /// and from fractional layout positions, at both display scales. Drives
        /// the production `snap::text_snapped` path all chrome text routes
        /// through.
        #[test]
        fn gate_chrome_galley_origins_snap_to_physical_grid() {
            for ppp in GATE_PPP {
                let ctx = egui::Context::default();
                // Bind the real UI faces so `font_medium` ("ui-medium") resolves.
                crate::ui::theme::setup_fonts(&ctx);
                ctx.set_pixels_per_point(ppp);
                let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                    let painter = ui.painter();
                    for size in 8..=16 {
                        let rect = crate::ui::snap::text_snapped(
                            painter,
                            egui::pos2(12.3, 9.7),
                            egui::Align2::LEFT_TOP,
                            "context / subtitle",
                            crate::ui::theme::font_medium(size as f32),
                            egui::Color32::WHITE,
                        );
                        for coord in [rect.min.x, rect.min.y] {
                            let phys = coord * ppp;
                            assert!(
                                (phys - phys.round()).abs() < 1e-3,
                                "galley origin {coord} off the grid at ppp {ppp} (size {size})"
                            );
                        }
                    }
                });
            }
        }
    }
}
