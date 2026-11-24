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
//! h.save_screenshot(&evidence_png!()).unwrap();
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

#[cfg(test)]
mod harness_hook;

use std::path::Path;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::app::PlexiApp;
use crate::host::pane::{Pane, ViewRuntime};
use crate::spatial::tiling::PaneId;

// ─── PlexiUiHarness ──────────────────────────────────────────────────────────

/// Generic plugin target used by the scene runner. Builtin ids resolve through
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

type HarnessBuiltinFactory = fn(&Path, &[String], u64) -> Box<dyn crate::app::view_trait::View>;

fn assistant_harness_factory(
    workspace_root: &Path,
    _args: &[String],
    context_id: u64,
) -> Box<dyn crate::app::view_trait::View> {
    let broker: std::sync::Arc<dyn crate::plexi_ai::broker::AiBroker> =
        std::sync::Arc::new(crate::plexi_ai::broker::LiveAiBroker::new(None));
    Box::new(crate::assistant::AssistantView::new(
        workspace_root.to_path_buf(),
        broker,
        workspace_root,
        context_id,
    ))
}

struct ButtonGalleryFixture;

impl crate::app::view_trait::View for ButtonGalleryFixture {
    fn type_id(&self) -> &'static str {
        "button-gallery-fixture"
    }
    fn display_name(&self) -> String {
        "Kit button gallery".into()
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn ui(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &crate::app::view_trait::ViewRenderContext<'_>,
    ) {
        // Padded so captions and the first column are never flush with the pane edge.
        egui::Frame::new()
            .inner_margin(egui::Margin::same(crate::ui::style::SPACE_XL as i8))
            .show(ui, |ui| {
                crate::overlays::ui_gallery::draw_button_sizes(ui, ctx.colors);
            });
    }
}

fn button_gallery_factory(_: &Path, _: &[String], _: u64) -> Box<dyn crate::app::view_trait::View> {
    Box::new(ButtonGalleryFixture)
}

struct DescriptorDropdownFixture {
    field: crate::app::plexi_descriptor::ArgSpec,
    values: std::collections::HashMap<String, String>,
}

impl crate::app::view_trait::View for DescriptorDropdownFixture {
    fn type_id(&self) -> &'static str {
        "descriptor-dropdown-fixture"
    }
    fn display_name(&self) -> String {
        "Deployment settings".into()
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn ui(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &crate::app::view_trait::ViewRenderContext<'_>,
    ) {
        crate::render::descriptor_widgets::form_fields(
            ui,
            ctx.colors,
            [&self.field],
            &mut self.values,
            false,
            ctx.pane_id,
        );
        ui.label(format!(
            "Selected environment: {}",
            self.values["Environment"]
        ));
    }
}

fn descriptor_dropdown_factory(
    _: &Path,
    _: &[String],
    _: u64,
) -> Box<dyn crate::app::view_trait::View> {
    Box::new(DescriptorDropdownFixture {
        field: serde_json::from_value(serde_json::json!({
            "name": "Environment", "type": "enum",
            "description": "Choose the deployment environment",
            "enum_values": ["Development", "Staging", "Production"]
        }))
        .unwrap(),
        values: std::collections::HashMap::from([("Environment".into(), "Development".into())]),
    })
}

// Seed the popup endpoint directly: generic scene pane gestures do not
// reproduce kittest's widget click routing. The descriptor and secrets pixel
// tests separately open the production callers through real widget clicks.
struct DropdownSceneFixture {
    selected: &'static str,
}
impl crate::app::view_trait::View for DropdownSceneFixture {
    fn type_id(&self) -> &'static str {
        "kit-dropdown-fixture"
    }
    fn display_name(&self) -> String {
        "Kit dropdown".into()
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn ui(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &crate::app::view_trait::ViewRenderContext<'_>,
    ) {
        let response = crate::ui::dropdown::select_value(
            ui,
            "scene_dropdown",
            self.selected,
            &mut self.selected,
            || {
                [
                    ("Development", "Development"),
                    ("Staging", "Staging"),
                    ("Production", "Production"),
                ]
            },
            ctx.colors,
        );
        if ui.input(|input| input.key_pressed(egui::Key::F6)) {
            egui::Popup::open_id(ui.ctx(), response.id.with("popup"));
        }
        ui.label(format!("Selected environment: {}", self.selected));
        ui.label(
            if egui::Popup::is_id_open(ui.ctx(), response.id.with("popup")) {
                "Options open"
            } else {
                "Options closed"
            },
        );
    }
}
fn dropdown_scene_factory(_: &Path, _: &[String], _: u64) -> Box<dyn crate::app::view_trait::View> {
    Box::new(DropdownSceneFixture {
        selected: "Development",
    })
}
const HARNESS_BUILTIN_FACTORIES: &[(&str, HarnessBuiltinFactory)] = &[
    ("assistant", assistant_harness_factory),
    ("button-gallery-fixture", button_gallery_factory),
    ("descriptor-dropdown-fixture", descriptor_dropdown_factory),
    ("kit-dropdown-fixture", dropdown_scene_factory),
];

fn harness_builtin_factory(
    id: &str,
    cwd: &Path,
    args: &[String],
    context_id: u64,
) -> Option<Box<dyn crate::app::view_trait::View>> {
    HARNESS_BUILTIN_FACTORIES
        .iter()
        .find(|(candidate, _)| *candidate == id)
        .map(|(_, factory)| factory(cwd, args, context_id))
}

/// Semantic UI test harness. Wraps egui_kittest's `Harness<PlexiApp>` using the
/// `new_eframe` constructor so PlexiApp gets the kittest-managed egui::Context
/// at construction time — no shared-state gymnastics needed.
pub struct PlexiUiHarness {
    inner: egui_kittest::Harness<'static, PlexiApp>,
    workspace: tempfile::TempDir,
    _profile_guard: Option<crate::config::TestProfileDirGuard>,
    _profile_dir: tempfile::TempDir,
    /// Isolates the channel-neutral shared dir (`~/.plexi`) for the harness
    /// lifetime, so a test that renders the notes picker reads a tempdir rather
    /// than the developer's real global notes tier.
    _shared_guard: crate::config::TestSharedDirGuard,
    _shared_dir: tempfile::TempDir,
}

impl PlexiUiHarness {
    /// Create a harness with a fresh, isolated PlexiApp.
    pub fn new() -> Self {
        let profile_dir = tempfile::tempdir().expect("create UI harness profile");
        let profile_guard = crate::config::test_profile_dir_override()
            .is_none()
            .then(|| crate::config::set_test_profile_dir(profile_dir.path().to_path_buf()));
        let workspace = tempfile::tempdir().expect("create UI harness workspace");
        let shared_dir = tempfile::tempdir().expect("create UI harness shared dir");
        let _shared_guard = crate::config::set_test_shared_dir(shared_dir.path().to_path_buf());
        let frame_tick = Arc::new(AtomicU64::new(0));
        let context_id = crate::testing::reserve_test_context_id();
        let pane_block = crate::testing::reserve_pane_id_block();
        let harness = egui_kittest::Harness::new_eframe(move |cc| {
            let (mut app, _ipc_tx) = PlexiApp::new_for_test(cc.egui_ctx.clone(), frame_tick);
            app.set_initial_context_id_for_test(context_id);
            app.host.seed_next_pane_id(pane_block);
            app
        });
        Self {
            inner: harness,
            workspace,
            _profile_guard: profile_guard,
            _profile_dir: profile_dir,
            _shared_guard,
            _shared_dir: shared_dir,
        }
    }

    /// Create a harness with an explicit surface size. Use for screenshot
    /// tests — the default kittest surface is too small for pane chrome and
    /// plugin content to be legible in the saved PNG.
    pub fn new_sized(width: f32, height: f32) -> Self {
        Self::new_sized_ppp(width, height, 1.0)
    }

    /// Create a harness with an explicit surface size and `pixels_per_point`.
    /// Use `ppp: 2.0` for HiDPI screenshot tests — pixel-grid regressions
    /// (half-resolution surfaces, off-grid galley origins) are invisible at
    /// the default ppp of 1.0.
    pub fn new_sized_ppp(width: f32, height: f32, ppp: f32) -> Self {
        Self::new_sized_config(width, height, ppp, Default::default())
    }

    fn new_sized_config(
        width: f32,
        height: f32,
        ppp: f32,
        config: crate::config::PlexiConfig,
    ) -> Self {
        let profile_dir = tempfile::tempdir().expect("create UI harness profile");
        let profile_guard = crate::config::test_profile_dir_override()
            .is_none()
            .then(|| crate::config::set_test_profile_dir(profile_dir.path().to_path_buf()));
        let workspace = tempfile::tempdir().expect("create UI harness workspace");
        let shared_dir = tempfile::tempdir().expect("create UI harness shared dir");
        let _shared_guard = crate::config::set_test_shared_dir(shared_dir.path().to_path_buf());
        let frame_tick = Arc::new(AtomicU64::new(0));
        let context_id = crate::testing::reserve_test_context_id();
        let pane_block = crate::testing::reserve_pane_id_block();
        let harness = egui_kittest::Harness::builder()
            .with_size(egui::Vec2::new(width, height))
            .with_pixels_per_point(ppp)
            .build_eframe(move |cc| {
                let (mut app, _ipc_tx) =
                    PlexiApp::new_for_test_with_config(cc.egui_ctx.clone(), frame_tick, config);
                app.set_initial_context_id_for_test(context_id);
                app.host.seed_next_pane_id(pane_block);
                app
            });
        Self {
            inner: harness,
            workspace,
            _profile_guard: profile_guard,
            _profile_dir: profile_dir,
            _shared_guard,
            _shared_dir: shared_dir,
        }
    }

    /// Harness for a `phys_w` x `phys_h` PHYSICAL-pixel window at UI scale
    /// `scale`: the logical surface ends up `phys / scale` points and egui's
    /// zoom factor is `scale`.
    pub fn new_sized_scaled(phys_w: f32, phys_h: f32, scale: f32) -> Self {
        Self::new_sized_scaled_with_config(phys_w, phys_h, scale, Default::default())
    }

    pub fn new_sized_scaled_with_config(
        phys_w: f32,
        phys_h: f32,
        scale: f32,
        config: crate::config::PlexiConfig,
    ) -> Self {
        // Start at zoom 1.0 with the physical size; egui rescales the screen
        // rect to `phys / scale` on the pass that changes the zoom factor,
        // exactly as it does for a real window.
        let mut h = Self::new_sized_config(phys_w, phys_h, 1.0, config);
        h.with_app_mut(|app| app.ui_scale = scale);
        h.run_steps(3);
        h
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
    /// omits `cwd`, keeping native plugin state out of the developer's workspace.
    pub fn workspace_root(&self) -> &Path {
        self.workspace.path()
    }

    // ── Real plugin / host plugin / portal drivers ─────────────────────────────────

    /// Open any supported scene target through its normal pane-opening path.
    pub fn open_target(&mut self, target: HarnessOpenTarget<'_>) -> Result<PaneId, String> {
        match target {
            HarnessOpenTarget::Process { path, args } => self.open_view_at(path, args),
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
        let context_id = self.with_app(|app| app.windows[app.active_window].context_id);
        if let Some(app) = harness_builtin_factory(id, cwd, args, context_id) {
            self.with_app_mut(|host| {
                host.open_builtin_view_pane(
                    app,
                    crate::app::permissions::PluginPermissions::builtin(),
                    cwd.to_path_buf(),
                    None,
                    Some("split_v"),
                    None,
                );
            });
        } else {
            self.with_app_mut(|host| {
                host.launch_view_by_id_with_layout(
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
                    Some(Pane::View(app)) if matches!(app.runtime, ViewRuntime::Builtin(_))
                )
            })
        });
        if !is_builtin {
            return Err(format!("plugin id '{id}' did not resolve to a builtin"));
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

    /// Launch a real PGAP plugin process from `plugin_dir` (a directory containing
    /// `manifest.toml`) — the same production path as `plexi view open <path>`.
    /// The child process, IPC threads, and L1 render pipeline are all real.
    /// Returns the new pane's id.
    ///
    /// For `.py` entries running outside an installed bundle, the repo SDK at
    /// `sdk/python` is exported via `PLEXI_SDK_PATH` so `plexi_sdk` imports
    /// resolve in the child.
    ///
    /// `args` are forwarded in `PlexiEvent::Init` and surface as `ctx.args` —
    /// pass JSON state for deterministic scenes.
    pub fn open_view_at(&mut self, plugin_dir: &Path, args: &[String]) -> Result<PaneId, String> {
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
        let path = plugin_dir.to_string_lossy().into_owned();
        self.with_app_mut(|app| {
            app.trust_plugin_path_for_test(&path);
            app.launch_view_by_path_with_layout(&path, Some("split_v".to_string()), None, args)
                .map(|_| ())
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

    /// Open a sandboxed WASM component plugin, forwarding `args` to the guest's
    /// `init` as its argv (the same path as `plexi view open <path> -- <args>`).
    pub fn open_wasm_at_with_args(
        &mut self,
        wasm_path: &Path,
        args: Vec<String>,
    ) -> Result<PaneId, String> {
        let plugin_id = wasm_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("wasm")
            .to_string();
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        self.with_app_mut(|app| {
            app.trust_plugin_path_for_test(wasm_path.to_string_lossy());
            app.open_wasm_view_pane(&plugin_id, wasm_path, cwd, args, None)
        })
    }

    /// Step frames until the real plugin process behind `pane_id` commits its
    /// first rendered frame (FrameDone observed), sleeping between steps to
    /// give the child process wall-clock time. Fails with the plugin's recent
    /// stderr on crash or timeout.
    pub fn wait_for_view_frame(
        &mut self,
        pane_id: PaneId,
        timeout: Duration,
    ) -> Result<(), String> {
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
                let Some(Pane::View(view_pane)) = win.panes.get(&pane_id) else {
                    return Probe::Dead(format!("pane {pane_id} is not a view pane"));
                };
                match &view_pane.runtime {
                    ViewRuntime::Python(p) if p.has_rendered_tree() => Probe::Rendered,
                    ViewRuntime::Python(p) => p
                        .error()
                        .map(|error| Probe::Dead(error.to_string()))
                        .unwrap_or(Probe::Waiting),
                    ViewRuntime::Wasm(p) if p.is_running() => Probe::Rendered,
                    _ => Probe::Dead(format!("pane {pane_id} is not a WASM plugin")),
                }
            });
            match probe {
                Probe::Rendered => return Ok(()),
                Probe::Dead(msg) => return Err(msg),
                Probe::Waiting => {}
            }
            if start.elapsed() > timeout {
                return Err(format!(
                    "timed out after {timeout:?} waiting for first plugin frame"
                ));
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    /// Open the built-in file browser host plugin rooted at `cwd`. Uses the
    /// production `open_builtin_view_pane` path with a `split_v` hint, which
    /// installs as the root pane when the context is empty (the ⌘O shortcut's
    /// `overlay` hint requires an existing focused pane).
    pub fn open_file_browser(&mut self, cwd: std::path::PathBuf) {
        self.with_app_mut(|app| {
            let fb: Box<dyn crate::app::view_trait::View> =
                Box::new(crate::file_browser::FileBrowserView::new(cwd.clone()));
            let perms = crate::app::permissions::PluginPermissions::builtin();
            app.open_builtin_view_pane(
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
        let assistant = crate::assistant::AssistantView::new(
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
        assistant: crate::assistant::AssistantView,
        workspace_root: std::path::PathBuf,
    ) {
        self.with_app_mut(|app| {
            let boxed: Box<dyn crate::app::view_trait::View> = Box::new(assistant);
            let perms = crate::app::permissions::PluginPermissions::builtin();
            app.open_builtin_view_pane(
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

/// Where this test's screenshot evidence goes: `/tmp/plexi_<test fn>.png`,
/// with the name taken from the enclosing function rather than typed out, so
/// a renamed test cannot leave a stale filename behind and every artifact
/// this file writes is reachable by one `rm /tmp/plexi_*.png` (the cleanup
/// TESTING.md's visual-review step prescribes). Pass a suffix to distinguish
/// several shots from one test: `evidence_png!("_ppp2")`.
///
/// Screenshots are a human review artifact, so the file deliberately survives
/// a passing run — the reviewer reads it, then deletes it.
#[cfg(test)]
macro_rules! evidence_png {
    () => {
        $crate::ui_tests::evidence_png_path($crate::ui_tests::enclosing_fn_name!(), "")
    };
    ($suffix:expr) => {
        $crate::ui_tests::evidence_png_path($crate::ui_tests::enclosing_fn_name!(), &$suffix)
    };
}
pub(crate) use evidence_png;

/// The unqualified name of the function this expands inside.
#[cfg(test)]
macro_rules! enclosing_fn_name {
    () => {{
        fn probe() {}
        fn name_of<T>(_: T) -> &'static str {
            std::any::type_name::<T>()
        }
        let path = name_of(probe);
        // `…::the_test::{{closure}}::probe` → `the_test`.
        path.strip_suffix("::probe")
            .unwrap_or(path)
            .trim_end_matches("::{{closure}}")
            .rsplit("::")
            .next()
            .unwrap_or(path)
    }};
}

#[cfg(test)]
pub(crate) use enclosing_fn_name;

#[cfg(test)]
pub(crate) fn evidence_png_path(test_name: &str, suffix: &str) -> String {
    format!("/tmp/plexi_{test_name}{suffix}.png")
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::permissions::PluginPermissions;
    use crate::app::FocusKind;
    use crate::host::context::Window;
    use crate::host::pane::{Pane, PortalPane, TerminalPane, ViewPane, ViewRuntime};
    use egui_kittest::kittest::Queryable;

    /// Queue `events` for the focused pane the way `plexi pane key` and
    /// `pane click` do, step, and prove `raw_input_hook` delivered the batch
    /// on a frame it marked synthetic, so a dialog that ignores it refused it
    /// rather than missed it.
    fn injected(h: &mut PlexiUiHarness, events: Vec<egui::Event>) {
        let pane = h
            .with_app(|app| app.active_focused_pane_id())
            .expect("pane IPC injects into a pane");
        h.with_app_mut(|app| {
            assert!(app.pane_navigate(pane), "pane {pane} could not be focused");
            app.pending_pane_inputs
                .entry(pane)
                .or_default()
                .push(egui::RawInput {
                    events,
                    ..Default::default()
                });
        });
        h.step();
        assert!(
            h.with_app(|app| {
                !app.pending_pane_inputs.contains_key(&pane) && app.synthetic_input_frame
            }),
            "the injected batch was not delivered as synthetic input"
        );
        h.run_steps(2);
    }

    fn key_tap(key: egui::Key) -> Vec<egui::Event> {
        [true, false]
            .map(|pressed| egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            })
            .into()
    }

    /// A primary click at the center of a kittest node rect (physical
    /// pixels), as `pane click` queues it.
    fn injected_click(h: &PlexiUiHarness, physical: egui::Rect) -> Vec<egui::Event> {
        let pos = physical.center() / h.with_app(|app| app.ctx.pixels_per_point());
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        vec![egui::Event::PointerMoved(pos), button(true), button(false)]
    }

    fn apply_fixture_theme(h: &mut PlexiUiHarness, theme: &str) {
        h.with_app_mut(|app| {
            let cfg = crate::ui::theme::preset_colors(theme).unwrap();
            app.colors = crate::ui::theme::Colors::from_config(&cfg);
            app.theme = crate::ui::theme::terminal_theme(
                &cfg,
                crate::config::DEFAULT_TERMINAL_MINIMUM_CONTRAST,
            );
            crate::ui::theme::setup_style(&app.ctx, &app.colors);
        });
    }

    fn assert_inside(what: &str, scale: f32, rect: egui::Rect, screen: egui::Rect) {
        let tol = 0.75;
        assert!(
            rect.min.x >= screen.min.x - tol
                && rect.min.y >= screen.min.y - tol
                && rect.max.x <= screen.max.x + tol
                && rect.max.y <= screen.max.y + tol,
            "{what} {rect:?} leaves logical screen {screen:?} at scale {scale}"
        );
    }

    fn ui_scale_area_rect(h: &PlexiUiHarness, id: &str) -> egui::Rect {
        h.with_app(|app| {
            app.ctx
                .memory(|m| m.area_rect(egui::Id::new(id).with("overlay")))
                .unwrap_or_else(|| panic!("modal area {id} not shown"))
        })
    }

    fn seed_note_picker(h: &mut PlexiUiHarness) {
        let realistic_notes = [
            (
                "Sprint Planning & Alpha Release Milestones",
                "## Sprint Objectives\n\n- Close P1 focus layer input leaks across modals\n- Validate Cmd+O Notes Picker layout, font and selection visibility\n- Cut next alpha tag with automated release checks\n\n### Open Items\n1. Review telemetry traces\n2. Verify keyboard navigation on dense note collections\n3. Confirm dirty-pane destructive confirmation flow",
                Some("active"),
            ),
            (
                "Architecture Decision: Unified Input Router",
                "Context:\nOverlay surfaces previously read raw input directly inside draw passes, leading to missed keys or desynchronized selection states.\n\nDecision:\nRoute all keyboard and paste events through `PlexiInput` before render dispatch.\n\nStatus: Accepted.",
                Some("active"),
            ),
            (
                "Quick Note Scratchpad",
                "Check out egui_term scrollback refactor when reviewing pull requests.\nRemember to test with ppp=2.0 on retina displays.",
                None,
            ),
            (
                "Database Migration Checklist",
                "- [x] Backup SQLite store\n- [x] Run schema version 4 migration\n- [ ] Vacuum indexes\n- [ ] Rebuild search FTS table",
                Some("global"),
            ),
            (
                "Short Note",
                "Plexi notes are markdown files stored in user scopes.",
                None,
            ),
            (
                "Terminal Font Tuning & Spacing Tokens",
                "Font: JetBrains Mono 14pt\nLine height: 1.25\nCell spacing: 0px horizontal, 2px vertical\nGlyph fallback: NotoEmoji + Nerd Font symbols.",
                Some("global"),
            ),
        ];

        // One instant, so equal ages tie and keep their seeded order.
        let now = std::time::SystemTime::now();
        // Yesterday's local midnight: at least 24 hours and one calendar day
        // back, so row 3 always reads "Yesterday".
        let since_yesterday = {
            let local = chrono::Local::now();
            let midnight = local
                .date_naive()
                .pred_opt()
                .and_then(|d| d.and_hms_opt(0, 0, 0))
                .and_then(|t| t.and_local_timezone(chrono::Local).earliest())
                .expect("yesterday's local midnight");
            (local - midnight).num_seconds() as u64
        };
        h.with_app_mut(|app| {
            app.open_notes_picker();
            let entries: Vec<crate::notes::NotePickerEntry> = realistic_notes
                .iter()
                .cycle()
                .take(30)
                .enumerate()
                .map(|(i, (title, body, label))| {
                    let path = std::env::temp_dir()
                        .join(format!("plexi-ui-scale-note-{}-{i}.md", std::process::id()));
                    let _ = std::fs::write(&path, body);
                    crate::notes::NotePickerEntry {
                        path,
                        title: title.to_string(),
                        preview: body.lines().next().unwrap_or("").to_string(),
                        search_text: format!("{title} {body}").to_lowercase(),
                        tier_label: label.map(|s| s.to_string()),
                        // Stint 1645: ages rise with the index, so the recency
                        // order keeps the seeded order and the first rows show
                        // every label form.
                        modified: Some(
                            now - std::time::Duration::from_secs(
                                [20, 7 * 60, 3 * 3600, since_yesterday, 4 * 86_400, 40 * 86_400]
                                    .get(i)
                                    .copied()
                                    .unwrap_or(i as u64 * 10 * 86_400),
                            ),
                        ),
                    }
                })
                .collect();
            app.notes_picker_entries = entries;
            app.notes_picker_selected = 0;
            app.notes_picker_last_drawn_selected = None;
        });
    }

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
            let view_pane = ViewPane {
                incarnation: Default::default(),
                pip_status: None,
                id: pane_id,
                runtime: ViewRuntime::Builtin(Box::new(crate::file_browser::FileBrowserView::new(
                    browser_root.clone(),
                ))),
                workspace_root: browser_root.clone(),
                permissions: PluginPermissions::builtin(),
                manifest_id: "test".to_string(),
                name: "Test App".to_string(),
                pane_group: None,
                linked_pane_id: None,
                overlay_replaced: None,
                agent: None,
                slots: std::collections::HashMap::new(),
                semantic_state: Default::default(),
            };
            let win = &mut app.windows[window_index];
            win.panes
                .insert(pane_id, Pane::View(Box::new(view_pane)))
                .unwrap();
            let tile_id = win.tree.tiles.insert_pane(pane_id);
            if win.tree.root.is_none() {
                win.tree.root = Some(tile_id);
            }
            win.focused_pane = Some(tile_id);
            pane_id
        })
    }

    fn gallery_text_shapes(shape: &egui::Shape, texts: &mut Vec<(egui::Rect, String)>) {
        match shape {
            egui::Shape::Text(text) if !text.galley.text().is_empty() => {
                texts.push((
                    text.galley.rect.translate(text.pos.to_vec2()),
                    text.galley.text().to_owned(),
                ));
            }
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    gallery_text_shapes(shape, texts);
                }
            }
            _ => {}
        }
    }

    /// Every non-empty text galley the last harness frame painted, with its
    /// rect. Galleys hold the text as painted, so an elided label reads with
    /// its `…` here even though its accessible label is the full string.
    fn painted_texts(h: &mut PlexiUiHarness) -> Vec<(egui::Rect, String)> {
        let mut texts = Vec::new();
        for shape in &h.harness().output().shapes {
            gallery_text_shapes(&shape.shape, &mut texts);
        }
        texts
    }

    /// The first painted galley containing `needle`, failing with every
    /// painted string so a miss shows what did render.
    fn painted_rect(texts: &[(egui::Rect, String)], needle: &str) -> egui::Rect {
        texts
            .iter()
            .find(|(_, text)| text.contains(needle))
            .map(|(rect, _)| *rect)
            .unwrap_or_else(|| panic!("no painted text contains {needle:?}: {texts:?}"))
    }

    fn assert_layout_surface(h: &PlexiUiHarness, surface: &str) {
        h.with_app(|app| {
            let reports = crate::ui::layout_report::snapshot(&app.ctx);
            assert!(
                reports.iter().any(|r| r.surface == surface),
                "missing {surface}"
            );
            let errors: Vec<_> = reports.iter().flat_map(|r| r.violations()).collect();
            assert!(errors.is_empty(), "{errors:?}");
        });
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
                win.panes
                    .insert(
                        portal_pane_id,
                        Pane::Portal(Box::new(PortalPane {
                            pane_id: portal_pane_id,
                            target_context_id: child_ctx_id,
                            context_state: None,
                        })),
                    )
                    .unwrap();
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
                    kind: "Plugin",
                    name: "File Browser".to_string(),
                },
            ];
            app.pending_context_close = Some(state);
            app.push_focus_layer(crate::app::FocusKind::ContextCloseConfirm);
        });
        // Two steps for the egui Area sizing pass.
        h.run_steps(2);
    }

    fn pane_chrome_fixture(scale: f32) -> (PlexiUiHarness, u64, u64) {
        pane_chrome_fixture_sized(scale, 1200.0)
    }

    fn pane_chrome_fixture_sized(scale: f32, width: f32) -> (PlexiUiHarness, u64, u64) {
        let mut h = PlexiUiHarness::new_sized_scaled(width, 700.0, scale);
        h.step();
        let first = add_focused_pane(&mut h);
        let second = add_focused_pane(&mut h);
        let root = h.workspace_root().to_path_buf();
        h.with_app_mut(|app| {
            app.sidebar_visible = false;
            let term = crate::host::pane::TerminalPane::fixture(
                first,
                app.ctx.clone(),
                app.pty_event_tx.clone(),
                egui_term::BackendSettings {
                    shell: "/bin/sh".into(),
                    working_directory: Some(root),
                    ..Default::default()
                },
                14.0,
            )
            .expect("spawn test terminal");
            let win = &mut app.windows[app.active_window];
            win.take_pane(first);
            win.panes
                .insert(first, Pane::Terminal(Box::new(term)))
                .unwrap();
            let a = win.tree.tiles.find_pane(&first).unwrap();
            let b = win.tree.tiles.find_pane(&second).unwrap();
            win.tree.root = Some(win.tree.tiles.insert_horizontal_tile(vec![a, b]));
        });
        (h, first, second)
    }

    /// Three laid-out columns `[a | b | c]` in the active window.
    fn three_column_fixture() -> (PlexiUiHarness, [u64; 3]) {
        let mut h = PlexiUiHarness::new_sized(1200.0, 600.0);
        h.step();
        let panes = [
            add_focused_pane(&mut h),
            add_focused_pane(&mut h),
            add_focused_pane(&mut h),
        ];
        h.with_app_mut(|app| {
            app.sidebar_visible = false;
            let context_id = app.windows[app.active_window].context_id;
            app.set_context_columns(context_id, panes.iter().map(|p| vec![*p]).collect());
        });
        h.run_steps(3);
        (h, panes)
    }

    /// The label of the widget holding keyboard focus, matched by rect
    /// against the labels a modal can show.
    fn focused_label(h: &mut PlexiUiHarness, labels: &[&str]) -> Option<String> {
        let focused = h.with_app(|app| {
            app.ctx
                .memory(|m| m.focused())
                .and_then(|id| app.ctx.read_response(id))
                .map(|r| r.rect)
        })?;
        labels
            .iter()
            .find(|label| {
                h.harness()
                    .query_by_label(label)
                    .is_some_and(|node| node.rect().intersects(focused))
            })
            .map(|label| label.to_string())
    }

    fn tab(h: &mut PlexiUiHarness, shift: bool) {
        let modifiers = if shift {
            egui::Modifiers::SHIFT
        } else {
            egui::Modifiers::NONE
        };
        h.harness().key_press_modifiers(modifiers, egui::Key::Tab);
        h.run_steps(2);
    }

    /// The `modal_action` hit rects of the open modal, in paint order.
    fn modal_action_hits(h: &mut PlexiUiHarness) -> Vec<egui::Rect> {
        h.with_app(|app| {
            crate::ui::layout_report::snapshot(&app.ctx)
                .into_iter()
                .filter(|r| r.surface == "modal_action")
                .flat_map(|r| r.actions)
                .map(|a| a.hit)
                .collect()
        })
    }

    /// Resize the window and let the modal settle; returns whether the
    /// action buttons stack (no two share a top edge).
    fn resize_settle(h: &mut PlexiUiHarness, width: f32) -> bool {
        h.harness().set_size(egui::vec2(width, 640.0));
        h.run_steps(3);
        let hits = modal_action_hits(h);
        assert!(hits.len() >= 2, "{hits:?}");
        hits.windows(2).all(|w| w[0].top() != w[1].top())
    }

    fn press_at(h: &mut PlexiUiHarness, pos: egui::Pos2) {
        for pressed in [true, false] {
            h.harness().input_mut().events.extend([
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
            h.step();
        }
        h.run_steps(2);
    }

    mod assistant;
    mod attention;
    mod consent;
    mod crispness_gate;
    mod dialogs;
    mod file_browser;
    mod kit;
    mod layout;
    mod notes;
    mod palette;
    mod palette_boundary;
    mod palette_widgets;
    mod pane_chrome;
    mod pane_selection;
    mod plugin_views;
    mod scale_and_shortcuts;
    mod sidebar;
    mod terminal;
    mod terminal_inset;
    mod titlebar;
    mod toasts;
    mod update_status;
}
