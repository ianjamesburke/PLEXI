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
        let workspace = tempfile::tempdir().expect("create UI harness workspace");
        let frame_tick = Arc::new(AtomicU64::new(0));
        let harness = egui_kittest::Harness::builder()
            .with_size(egui::Vec2::new(width, height))
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

    /// Exact semantic-label lookup scoped to one rendered pane rectangle.
    pub fn pane_has_label(&mut self, pane_id: PaneId, label: &str) -> bool {
        use egui_kittest::kittest::Queryable;

        let pane_rect = self.with_app(|host| {
            host.windows.iter().find_map(|window| {
                if !window.panes.contains_key(&pane_id) {
                    return None;
                }
                let tile_id = window.tree.tiles.iter().find_map(|(tile_id, tile)| {
                    matches!(tile, egui_tiles::Tile::Pane(id) if *id == pane_id)
                        .then_some(tile_id)
                })?;
                window.tree.tiles.rect(*tile_id)
            })
        });
        let Some(pane_rect) = pane_rect else {
            return false;
        };

        self.inner.query_all_by_label(label).any(|node| {
            node.raw_bounds().is_some_and(|bounds| {
                let center = egui::pos2(
                    ((bounds.x0 + bounds.x1) / 2.0) as f32,
                    ((bounds.y0 + bounds.y1) / 2.0) as f32,
                );
                pane_rect.contains(center)
            })
        })
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
                let AppRuntime::Process(p) = &app_pane.runtime else {
                    return Probe::Dead(format!("pane {pane_id} is not a process app"));
                };
                if !p.frame.is_empty() {
                    return Probe::Rendered;
                }
                let state = p.lifecycle.state();
                if state.is_terminal() {
                    return Probe::Dead(format!(
                        "app reached {state:?} before first frame; stderr:\n{}",
                        Self::drain_stderr(p)
                    ));
                }
                Probe::Waiting
            });
            match probe {
                Probe::Rendered => return Ok(()),
                Probe::Dead(msg) => return Err(msg),
                Probe::Waiting => {}
            }
            if start.elapsed() > timeout {
                let stderr = self.with_app(|app| {
                    let win = &app.windows[app.active_window];
                    match win.panes.get(&pane_id) {
                        Some(Pane::App(app_pane)) => match &app_pane.runtime {
                            AppRuntime::Process(p) => Self::drain_stderr(p),
                            _ => String::new(),
                        },
                        _ => String::new(),
                    }
                });
                return Err(format!(
                    "timed out after {timeout:?} waiting for first app frame; stderr:\n{stderr}"
                ));
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn drain_stderr(p: &crate::process_app::ProcessApp) -> String {
        p.recent_stderr
            .lock()
            .map(|q| q.iter().cloned().collect::<Vec<_>>().join("\n"))
            .unwrap_or_default()
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
        // Grants/audit also stay inside the temp workspace, never the real
        // channel profile dir.
        let assistant =
            crate::assistant::AssistantApp::new(workspace_root.clone(), broker, &workspace_root);
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
    use crate::app::FocusLayer;
    use crate::host::context::Window;
    use crate::host::pane::{AppPane, AppRuntime, Pane, PortalPane};
    use crate::process_app::ProcessApp;
    use egui_kittest::kittest::Queryable;

    fn add_focused_pane(h: &mut PlexiUiHarness) -> crate::spatial::tiling::PaneId {
        h.with_app_mut(|app| {
            let pane_id = app.host.alloc_pane_id();
            let (process_app, _draw_tx) =
                ProcessApp::new_for_test(pane_id, AppPermissions::builtin());
            let app_pane = AppPane {
                pip_status: None,
                id: pane_id,
                runtime: AppRuntime::Process(Box::new(process_app)),
                workspace_root: std::env::temp_dir(),
                permissions: AppPermissions::builtin(),
                manifest_id: "test".to_string(),
                name: "Test App".to_string(),
                pane_group: None,
                linked_pane_id: None,
                overlay_replaced: None,
                hidden: false,
                agent: None,
                slots: std::collections::HashMap::new(),
            };
            let win = &mut app.windows[app.active_window];
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

    /// Stint 0207: render the four new canvas styling primitives (rect gradient,
    /// rect glow + stroke, circle glow + stroke, arc_ring) directly through
    /// `render_draw_commands` and save a PNG for visual inspection.
    /// File written to /tmp/plexi_canvas_primitives.png.
    #[test]
    fn screenshot_canvas_primitives() {
        use crate::protocol::commands::{Gradient, GradientDir, RenderCommand};

        let commands = vec![
            // Dark backdrop so glows are visible.
            RenderCommand::Rect {
                x: 0.0,
                y: 0.0,
                w: 440.0,
                h: 320.0,
                fill: "#11111b".to_string(),
                radius: 0.0,
                stroke: None,
                stroke_width: 1.0,
                glow_color: None,
                glow_radius: 0.0,
                gradient: None,
                hit_region: None,
            },
            // 1. Horizontal gradient fill.
            RenderCommand::Rect {
                x: 24.0,
                y: 24.0,
                w: 180.0,
                h: 80.0,
                fill: "#000000".to_string(),
                radius: 8.0,
                stroke: None,
                stroke_width: 1.0,
                glow_color: None,
                glow_radius: 0.0,
                gradient: Some(Gradient {
                    from: "#f38ba8".to_string(),
                    to: "#89b4fa".to_string(),
                    dir: GradientDir::Horizontal,
                }),
                hit_region: None,
            },
            // 2. Glow + stroke rect.
            RenderCommand::Rect {
                x: 24.0,
                y: 128.0,
                w: 180.0,
                h: 80.0,
                fill: "#1e1e2e".to_string(),
                radius: 10.0,
                stroke: Some("#a6e3a1".to_string()),
                stroke_width: 2.0,
                glow_color: Some("#a6e3a1".to_string()),
                glow_radius: 18.0,
                gradient: None,
                hit_region: None,
            },
            // 3. Glow + stroke circle.
            RenderCommand::Circle {
                cx: 320.0,
                cy: 80.0,
                r: 38.0,
                fill: "#cba6f7".to_string(),
                stroke: Some("#ffffff".to_string()),
                stroke_width: 2.0,
                glow_color: Some("#cba6f7".to_string()),
                glow_radius: 22.0,
            },
            // 4. Arc ring (three-quarter sweep).
            RenderCommand::ArcRing {
                cx: 320.0,
                cy: 210.0,
                r: 42.0,
                start_angle: 0.0,
                end_angle: std::f32::consts::PI * 1.5,
                color: "#fab387".to_string(),
                stroke_width: 7.0,
            },
        ];

        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(440.0, 320.0))
            .build_ui(move |ui| {
                let colors =
                    crate::ui::theme::colors_from_config(&crate::config::PlexiConfig::default());
                let mut commonmark = egui_commonmark::CommonMarkCache::default();
                let audio: std::collections::HashMap<String, f32> = Default::default();
                let mut image_cache = crate::process_app::image_cache::ImageCache::new();
                let ws = std::env::temp_dir();
                let mut lv_off = std::collections::HashMap::new();
                let mut lv_sel = std::collections::HashMap::new();
                let mut events = Vec::new();
                let mut te_buf = std::collections::HashMap::new();
                let mut te_focus = crate::render::components::TextEditFocusCtx::new();
                let mut canvas_width = 0.0;
                let mut canvas_height = 0.0;
                let rect = ui.max_rect();
                crate::process_app::render::render_draw_commands(
                    ui,
                    rect,
                    &commands,
                    &colors,
                    &mut commonmark,
                    &audio,
                    &mut image_cache,
                    &ws,
                    false,
                    &mut lv_off,
                    &mut lv_sel,
                    &mut events,
                    &mut te_buf,
                    &mut te_focus,
                    &mut canvas_width,
                    &mut canvas_height,
                    &mut Vec::new(),
                );
            });
        harness.run();
        let img = harness.render().expect("render failed");
        img.save("/tmp/plexi_canvas_primitives.png")
            .expect("save failed");
        println!("Screenshot saved to /tmp/plexi_canvas_primitives.png");
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
            h.with_app(|app| matches!(app.focus_stack.last(), Some(FocusLayer::RawWasmReview)));
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

        h.harness().press_key(egui::Key::Enter);
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
        let mut h = PlexiUiHarness::new_sized(900.0, 640.0);
        let _base = add_focused_pane(&mut h);
        h.with_app_mut(|app| {
            let active_ctx_id = app.router.active().context_id;
            let child_ctx_id = active_ctx_id + 5000;
            // Register the child context so the portal header shows a real name.
            app.router.push(crate::host::context::Context {
                name: "Notes".to_string(),
                path: std::env::temp_dir(),
                root: None,
                description: Some("Editing notes".to_string()),
                context_id: child_ctx_id,
                parent_id: Some(active_ctx_id),
                depth: 1,
                parked: false,
            });

            // Child window holding the text-editor pane the portal previews.
            let editor_pane_id = app.host.alloc_pane_id();
            let (process_app, _tx) =
                ProcessApp::new_for_test(editor_pane_id, AppPermissions::builtin());
            let editor = AppPane {
                pip_status: Some(crate::app_protocol::PipStatus::Green),
                id: editor_pane_id,
                runtime: AppRuntime::Process(Box::new(process_app)),
                workspace_root: std::env::temp_dir(),
                permissions: AppPermissions::builtin(),
                manifest_id: "text-editor".to_string(),
                name: "note.md".to_string(),
                pane_group: None,
                linked_pane_id: None,
                overlay_replaced: None,
                hidden: false,
                agent: None,
                slots: std::collections::HashMap::new(),
            };
            let mut child_panes = std::collections::HashMap::new();
            child_panes.insert(editor_pane_id, Pane::App(Box::new(editor)));
            let mut child_tiles = egui_tiles::Tiles::default();
            let child_tile = child_tiles.insert_pane(editor_pane_id);
            let child_window_id = app.next_window_id;
            app.next_window_id += 1;
            app.windows.push(Window {
                name: "notes window".to_string(),
                path: std::env::temp_dir(),
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
            .build(move |ctx| {
                crate::ui::theme::setup_fonts(ctx);
                egui::CentralPanel::default()
                    .frame(egui::Frame::default().fill(colors.bg_darkest))
                    .show(ctx, |ui| {
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
                .contains(&crate::app::FocusLayer::NotesPicker));
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
    fn ui_gallery_overlay_smoke() {
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
        h.save_screenshot("/tmp/plexi_ui_gallery.png")
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
            app.push_focus_layer(crate::app::FocusLayer::NotesTriage);
        });
        // Two steps for the egui Area sizing pass (see picker smoke test).
        h.run_steps(2);
        h.with_app(|app| {
            assert!(app
                .focus_stack
                .contains(&crate::app::FocusLayer::NotesTriage));
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
                .contains(&crate::app::FocusLayer::NotesTriage));
            assert!(app
                .focus_stack
                .contains(&crate::app::FocusLayer::NotesPicker));
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

    /// Renders the markdown-demo POC app through the real process + render
    /// path and screenshots it. Visual regression for themed markdown chrome:
    /// fenced code-block cards, inline-code tint, blockquote tint, link color.
    /// Inspect /tmp/plexi_markdown_demo.png after running.
    #[test]
    fn markdown_demo_styling() {
        // Narrow window so code blocks wrap like a real side pane — this is the
        // width where copy-icon placement and block padding actually show.
        let mut h = PlexiUiHarness::new_sized(560.0, 940.0);
        let app_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("apps/dev/markdown-demo");
        let pane_id = h
            .open_app_at(&app_dir, &[])
            .expect("markdown-demo should launch");
        h.wait_for_app_frame(pane_id, Duration::from_secs(20))
            .expect("markdown-demo should render its first frame");
        h.run_steps(3);
        // Resting state: copy buttons are hover-only, so blocks render clean
        // with no icon overlapping the code text.
        h.save_screenshot("/tmp/plexi_markdown_demo.png")
            .expect("render failed");

        // Hover state: move the pointer over a code block and confirm the copy
        // button appears pinned to the block's top-right corner. The Python
        // block sits low in the narrow layout; hover its interior.
        h.harness()
            .input_mut()
            .events
            .push(egui::Event::PointerMoved(egui::pos2(150.0, 485.0)));
        h.run_steps(3);
        h.save_screenshot("/tmp/plexi_markdown_demo_hover.png")
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

        assert!(h.pane_has_label(assistant, "Assistant"));
        assert!(h.pane_has_label(assistant, "default"));
        assert!(!h.pane_has_label(files, "Assistant"));
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
            crate::assistant::AssistantApp::new(ws.path().to_path_buf(), broker, ws.path());
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
            crate::assistant::AssistantApp::new(ws.path().to_path_buf(), broker, ws.path());
        let turn = |role, text: &str| crate::assistant::model::Turn {
            role,
            text: text.to_string(),
            created_at: "2026-06-12T00:00:00Z".to_string(),
            status: None,
            thoughts: None,
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
        ];
        assistant.model.composer = "/".to_string();
        h.open_assistant_built(assistant, ws.path().to_path_buf());
        h.run_steps(10);
        h.save_screenshot("/tmp/plexi-render-2216-assistant-picker.png")
            .expect("render failed");
    }

    #[test]
    fn screenshot_permission_prompt_uses_host_chrome() {
        let mut h = PlexiUiHarness::new_sized(1000.0, 720.0);
        let pane_id = add_focused_pane(&mut h);
        h.step();
        h.with_app_mut(|app| {
            let win = &mut app.windows[app.active_window];
            let Some(Pane::App(app_pane)) = win.panes.get_mut(&pane_id) else {
                panic!("test pane missing");
            };
            let AppRuntime::Process(process) = &mut app_pane.runtime else {
                panic!("test pane is not process app");
            };
            process
                .pending_prompts
                .push_back(crate::process_app::PendingPrompt::Capability {
                    request_id: "test-permission".to_string(),
                    capability: "fs.read".to_string(),
                });
            app.focus_stack.push(FocusLayer::CapabilityModal);
        });
        h.run_steps(2);
        h.save_screenshot("/tmp/plexi_permission_prompt_chrome.png")
            .expect("render failed");
        assert!(
            h.with_app(|app| matches!(app.focus_stack.last(), Some(FocusLayer::CapabilityModal))),
            "capability modal should own focus for screenshot"
        );
        println!("Screenshot saved to /tmp/plexi_permission_prompt_chrome.png");
    }

    #[test]
    fn screenshot_command_palette_metadata_lane() {
        let mut h = PlexiUiHarness::new_sized(1168.0, 720.0);
        let first_pane_id = add_focused_pane(&mut h);
        h.with_app_mut(|app| {
            let second_pane_id = app.host.alloc_pane_id();
            let (process_app, _draw_tx) =
                ProcessApp::new_for_test(second_pane_id, AppPermissions::builtin());
            let app_pane = AppPane {
                pip_status: None,
                id: second_pane_id,
                runtime: AppRuntime::Process(Box::new(process_app)),
                workspace_root: std::env::temp_dir(),
                permissions: AppPermissions::builtin(),
                manifest_id: "hidden-test".to_string(),
                name: "Hidden Test App".to_string(),
                pane_group: None,
                linked_pane_id: None,
                overlay_replaced: None,
                hidden: true,
                agent: None,
                slots: std::collections::HashMap::new(),
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
        let center = img.get_pixel(img.width() / 2, img.height() / 2).0;
        let center_luma = (u16::from(center[0]) + u16::from(center[1]) + u16::from(center[2])) / 3;
        assert!(
            center_luma > 180,
            "Catppuccin Latte palette center should be light, got rgba={center:?}"
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
            h.harness().press_key(egui::Key::ArrowDown);
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
            app.focus_stack.push(FocusLayer::ContextRename);
        });

        // Queue Enter for the next frame — processed by draw_rename_context_overlay.
        h.harness().press_key(egui::Key::Enter);
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
            app.focus_stack.push(FocusLayer::ContextRename);
        });

        h.harness().press_key(egui::Key::Escape);
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
            app.focus_stack.push(FocusLayer::ContextRename);
        });

        h.harness().press_key(egui::Key::Enter);
        h.step();

        let name = h.with_app(|app| app.router.active().name.clone());
        assert_eq!(
            name, original_name,
            "whitespace-only rename must be discarded"
        );
    }

    /// Regression: tab titles with long names must stay on a single line and show
    /// a trailing ellipsis rather than wrapping or overflowing the tab bar rect.
    #[test]
    fn tab_titles_truncate_to_single_line() {
        let mut h = PlexiUiHarness::new();
        h.step();

        h.with_app_mut(|app| {
            let pane_id_a = app.host.alloc_pane_id();
            let pane_id_b = app.host.alloc_pane_id();
            for (pane_id, name) in [
                (pane_id_a, "A very long tab title that should not wrap onto a second line under any circumstances"),
                (pane_id_b, "Short"),
            ] {
                let (process_app, _draw_tx) =
                    ProcessApp::new_for_test(pane_id, AppPermissions::builtin());
                let app_pane = AppPane {
                    pip_status: None,
                    id: pane_id,
                    runtime: AppRuntime::Process(Box::new(process_app)),
                    workspace_root: std::env::temp_dir(),
                    permissions: AppPermissions::builtin(),
                    manifest_id: "test".to_string(),
                    name: name.to_string(),
                    pane_group: None,
                    linked_pane_id: None,
                    overlay_replaced: None,
                    hidden: false,
                    agent: None,
                    slots: std::collections::HashMap::new(),
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
}
