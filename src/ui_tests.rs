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
    use crate::host::context::Window;
    use crate::host::pane::{AppPane, AppRuntime, Pane, PortalPane};
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

    // Visual smoke coverage lives in `tests/scenes/*.toml`, executed by
    // `scenes::tests::scene_suite`. Run one ad hoc with `just scene <file>`.

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
                app_pane.overlay_replaced =
                    Some(Box::new(Pane::Portal(Box::new(PortalPane {
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
        h.save_screenshot("/tmp/plexi_host_ui_gallery_trust.png")
            .expect("render failed");
        assert!(
            h.with_app(|app| app.show_ui_gallery),
            "gallery should remain open for screenshot"
        );
        println!("Screenshot saved to /tmp/plexi_host_ui_gallery_trust.png");
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
