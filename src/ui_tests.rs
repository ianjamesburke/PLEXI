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

/// Semantic UI test harness. Wraps egui_kittest's `Harness<PlexiApp>` using the
/// `new_eframe` constructor so PlexiApp gets the kittest-managed egui::Context
/// at construction time — no shared-state gymnastics needed.
pub struct PlexiUiHarness {
    inner: egui_kittest::Harness<'static, PlexiApp>,
}

impl PlexiUiHarness {
    /// Create a harness with a fresh, isolated PlexiApp.
    pub fn new() -> Self {
        let frame_tick = Arc::new(AtomicU64::new(0));
        let harness = egui_kittest::Harness::new_eframe(move |cc| {
            let (app, _ipc_tx) = PlexiApp::new_for_test(cc.egui_ctx.clone(), frame_tick.clone());
            app
        });
        Self { inner: harness }
    }

    /// Create a harness with an explicit surface size. Use for screenshot
    /// tests — the default kittest surface is too small for pane chrome and
    /// app content to be legible in the saved PNG.
    pub fn new_sized(width: f32, height: f32) -> Self {
        let frame_tick = Arc::new(AtomicU64::new(0));
        let harness = egui_kittest::Harness::builder()
            .with_size(egui::Vec2::new(width, height))
            .build_eframe(move |cc| {
                let (app, _ipc_tx) =
                    PlexiApp::new_for_test(cc.egui_ctx.clone(), frame_tick.clone());
                app
            });
        Self { inner: harness }
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

    // ── Real app / host app / portal drivers ─────────────────────────────────

    /// Launch a real PGAP app process from `app_dir` (a directory containing
    /// `manifest.toml`) — the same production path as `plexi app open <path>`.
    /// The child process, IPC threads, and L1 render pipeline are all real.
    /// Returns the new pane's id.
    ///
    /// For `.py` entries running outside an installed bundle, the repo SDK at
    /// `sdk/python` is exported via `PLEXI_SDK_PATH` so `plexi_sdk` imports
    /// resolve in the child.
    pub fn open_app_at(&mut self, app_dir: &Path) -> Result<PaneId, String> {
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
            app.launch_app_by_path_with_layout(&path, Some("split_v".to_string()), None)
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
        self.with_app_mut(|app| app.push_pane_to_subcontext(name));
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::permissions::AppPermissions;
    use crate::app::FocusLayer;
    use crate::host::pane::{AppPane, AppRuntime, Pane};
    use crate::process_app::ProcessApp;

    fn add_focused_pane(h: &mut PlexiUiHarness) -> crate::spatial::tiling::PaneId {
        h.with_app_mut(|app| {
            let pane_id = app.host.alloc_pane_id();
            let (process_app, _draw_tx) =
                ProcessApp::new_for_test(pane_id, AppPermissions::builtin());
            let app_pane = AppPane {
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

    // ── Visual smoke: host app, portal, real PGAP app ─────────────────────────

    /// Headless screenshot of the built-in file browser host app.
    #[test]
    fn shot_file_browser() {
        let mut h = PlexiUiHarness::new_sized(1280.0, 800.0);
        h.step();
        h.open_file_browser(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")));
        h.run_steps(3);
        assert_eq!(h.pane_count(), 1, "file browser pane must exist");
        h.save_screenshot("/tmp/plexi_shot_file_browser.png")
            .expect("render failed");
        println!("Screenshot saved to /tmp/plexi_shot_file_browser.png");
    }

    /// Headless screenshot of the file browser with the host sidebar open.
    /// `new_for_test` defaults `sidebar_visible` to false; flip it on to
    /// capture the full host chrome (sidebar + pane).
    #[test]
    fn shot_file_browser_with_sidebar() {
        let mut h = PlexiUiHarness::new_sized(1280.0, 800.0);
        h.step();
        h.open_file_browser(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")));
        h.with_app_mut(|app| app.sidebar_visible = true);
        h.run_steps(3);
        h.save_screenshot("/tmp/plexi_shot_file_browser_sidebar.png")
            .expect("render failed");
        println!("Screenshot saved to /tmp/plexi_shot_file_browser_sidebar.png");
    }

    /// Headless screenshot of a subcontext portal: open a file browser pane,
    /// push it into a subcontext, render the resulting portal card.
    #[test]
    fn shot_subcontext_portal() {
        let mut h = PlexiUiHarness::new_sized(1280.0, 800.0);
        h.step();
        h.open_file_browser(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")));
        h.run_steps(2);
        let parent_idx = h.with_app(|app| app.router.active_idx());
        h.push_focused_pane_to_subcontext(Some("Subcontext".to_string()));
        // push_pane_to_subcontext navigates into the child context; switch
        // back to the parent so the portal card is the visible pane.
        h.with_app_mut(|app| app.switch_workspace(parent_idx));
        h.run_steps(3);
        let has_portal = h.with_app(|app| {
            app.windows[app.active_window]
                .panes
                .values()
                .any(|p| matches!(p, Pane::Portal(_)))
        });
        assert!(has_portal, "expected a portal pane after push_pane_to_subcontext");
        h.save_screenshot("/tmp/plexi_shot_subcontext_portal.png")
            .expect("render failed");
        println!("Screenshot saved to /tmp/plexi_shot_subcontext_portal.png");
    }

    /// Headless screenshot of the real `apps/balls` PGAP app: spawns the
    /// actual Python process, waits for its first committed frame, renders.
    #[test]
    #[ignore = "spawns a real Python app process — run by name with --ignored"]
    fn shot_balls_app() {
        let mut h = PlexiUiHarness::new_sized(1280.0, 800.0);
        h.step();
        let app_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("apps")
            .join("balls");
        let pane_id = h.open_app_at(&app_dir).expect("launch balls app");
        h.wait_for_app_frame(pane_id, std::time::Duration::from_secs(15))
            .expect("balls app first frame");
        h.run_steps(5);
        h.save_screenshot("/tmp/plexi_shot_balls.png")
            .expect("render failed");
        println!("Screenshot saved to /tmp/plexi_shot_balls.png");
    }

    /// Generic screenshot driver for any PGAP app directory — no code change
    /// needed. Run:
    /// `PLEXI_SHOT_APP=apps/foo PLEXI_SHOT_OUT=/tmp/foo.png \
    ///  cargo test --bin plexi shot_app_from_env -- --ignored --nocapture`
    #[test]
    #[ignore = "parameterized by PLEXI_SHOT_APP; spawns a real app process"]
    fn shot_app_from_env() {
        let app_dir = std::env::var("PLEXI_SHOT_APP")
            .expect("set PLEXI_SHOT_APP=<path to app dir containing manifest.toml>");
        let out = std::env::var("PLEXI_SHOT_OUT")
            .unwrap_or_else(|_| "/tmp/plexi_shot_app.png".to_string());
        let app_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(app_dir);
        let mut h = PlexiUiHarness::new_sized(1280.0, 800.0);
        h.step();
        let pane_id = h.open_app_at(&app_dir).expect("launch app");
        h.wait_for_app_frame(pane_id, std::time::Duration::from_secs(15))
            .expect("app first frame");
        h.run_steps(5);
        h.save_screenshot(&out).expect("render failed");
        println!("Screenshot saved to {out}");
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
}
