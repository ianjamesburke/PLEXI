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

type HarnessBuiltinFactory = fn(&Path, &[String], u64) -> Box<dyn crate::app::app_trait::App>;

fn assistant_harness_factory(
    workspace_root: &Path,
    _args: &[String],
    context_id: u64,
) -> Box<dyn crate::app::app_trait::App> {
    let broker: std::sync::Arc<dyn crate::plexi_ai::broker::AiBroker> =
        std::sync::Arc::new(crate::plexi_ai::broker::LiveAiBroker::new(None));
    Box::new(crate::assistant::AssistantApp::new(
        workspace_root.to_path_buf(),
        broker,
        workspace_root,
        context_id,
    ))
}

const HARNESS_BUILTIN_FACTORIES: &[(&str, HarnessBuiltinFactory)] =
    &[("assistant", assistant_harness_factory)];

fn harness_builtin_factory(
    id: &str,
    cwd: &Path,
    args: &[String],
    context_id: u64,
) -> Option<Box<dyn crate::app::app_trait::App>> {
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
        let profile_guard = crate::config::test_profile_dir_override().is_none()
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
    /// app content to be legible in the saved PNG.
    pub fn new_sized(width: f32, height: f32) -> Self {
        Self::new_sized_ppp(width, height, 1.0)
    }

    /// Create a harness with an explicit surface size and `pixels_per_point`.
    /// Use `ppp: 2.0` for HiDPI screenshot tests — pixel-grid regressions
    /// (half-resolution surfaces, off-grid galley origins) are invisible at
    /// the default ppp of 1.0.
    pub fn new_sized_ppp(width: f32, height: f32, ppp: f32) -> Self {
        let profile_dir = tempfile::tempdir().expect("create UI harness profile");
        let profile_guard = crate::config::test_profile_dir_override().is_none()
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

    /// Harness for a `phys_w` x `phys_h` PHYSICAL-pixel window at UI scale
    /// `scale`: the logical surface ends up `phys / scale` points and egui's
    /// zoom factor is `scale`.
    pub fn new_sized_scaled(phys_w: f32, phys_h: f32, scale: f32) -> Self {
        // Start at zoom 1.0 with the physical size; egui rescales the screen
        // rect to `phys / scale` on the pass that changes the zoom factor,
        // exactly as it does for a real window.
        let mut h = Self::new_sized_ppp(phys_w, phys_h, 1.0);
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
        let context_id = self.with_app(|app| app.windows[app.active_window].context_id);
        if let Some(app) = harness_builtin_factory(id, cwd, args, context_id) {
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
    use crate::app::permissions::AppPermissions;
    use crate::app::FocusKind;
    use crate::host::context::Window;
    use crate::host::pane::{AppPane, AppRuntime, Pane, PortalPane};
    use egui_kittest::kittest::Queryable;

    #[test]
    fn screenshot_card_gallery_all_states_and_surfaces() {
        crate::app::card_gallery::FORCE.with(|f| f.set(true));
        let mut h = PlexiUiHarness::new_sized(1440.0, 1400.0);
        h.run_steps(4);
        let path = evidence_png!();
        h.save_screenshot(&path).unwrap();
        assert!(std::path::Path::new(&path).exists());
    }

    /// Scale for the `screenshot_ui_scale_*` tests (run at 0.6 / 1.0 / 2.0).
    fn ui_scale_shot() -> f32 {
        crate::app::ui_scale::clamp_scale(
            std::env::var("PLEXI_UI_SCALE_SHOT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1.0),
        )
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
        h.with_app_mut(|app| {
            app.open_notes_picker();
            let mk = |title: &str, label: Option<&str>| crate::notes::NotePickerEntry {
                path: std::env::temp_dir().join(format!("plexi-ui-scale-{title}.md")),
                title: title.to_string(),
                preview: format!("{title} preview text for the picker row"),
                search_text: title.to_lowercase(),
                tier_label: label.map(|s| s.to_string()),
            };
            app.notes_picker_entries = (0..12)
                .map(|i| mk(&format!("note number {i}"), (i % 2 == 0).then_some("global")))
                .collect();
            app.notes_picker_selected = 0;
        });
    }

    /// At every UI scale, a 1440x900 physical window has a logical surface of
    /// `1440/scale x 900/scale`; the card gallery, atlas and the modals must
    /// all stay inside it.
    #[test]
    fn ui_scale_gallery_and_modals_fit_window() {
        for scale in [0.6_f32, 1.0, 2.0] {
            let mut h = PlexiUiHarness::new_sized_scaled(1440.0, 900.0, scale);
            add_focused_pane(&mut h);
            h.run_steps(3);
            let screen = h.with_app(|app| {
                assert!((app.ctx.zoom_factor() - scale).abs() < 1e-3, "zoom at {scale}");
                app.ctx.content_rect()
            });
            assert!(
                (screen.width() - 1440.0 / scale).abs() < 1.0
                    && (screen.height() - 900.0 / scale).abs() < 1.0,
                "logical screen {screen:?} at scale {scale}"
            );

            // Modals: open one at a time, assert its area, then dismiss.
            seed_note_picker(&mut h);
            h.run_steps(3);
            assert_inside("notes picker", scale, ui_scale_area_rect(&h, "notes_picker"), screen);
            h.harness().key_press(egui::Key::Escape);
            h.run_steps(3);

            h.with_app_mut(|app| app.show_command_palette = true);
            h.run_steps(3);
            assert_inside(
                "command palette",
                scale,
                ui_scale_area_rect(&h, "command_palette"),
                screen,
            );
            h.harness().key_press(egui::Key::Escape);
            h.run_steps(3);
            h.with_app_mut(|app| app.show_command_palette = false);

            h.with_app_mut(|app| app.open_quick_note_modal());
            h.run_steps(3);
            assert_inside(
                "quick note",
                scale,
                ui_scale_area_rect(&h, "quick_note_modal"),
                screen,
            );
            h.harness().key_press(egui::Key::Escape);
            h.run_steps(3);

            h.with_app_mut(|app| app.show_shortcuts = true);
            h.run_steps(3);
            assert_inside("shortcuts", scale, ui_scale_area_rect(&h, "shortcuts"), screen);
            h.with_app_mut(|app| app.show_shortcuts = false);

            // Atlas: every node and the hub pill lie inside the logical screen.
            h.with_app_mut(|app| {
                let root = app.create_context_without_pty("alpha", None);
                app.create_context_without_pty("beta", Some(root));
                app.create_context_without_pty("gamma", Some(root));
                app.create_context_without_pty("delta", None);
                let (nodes, _) = app.compute_atlas_graph(screen);
                assert!(!nodes.is_empty());
                for n in &nodes {
                    assert_inside(&format!("atlas node {}", n.name), scale, n.rect, screen);
                }
                for (i, a) in nodes.iter().enumerate() {
                    for b in &nodes[i + 1..] {
                        assert!(
                            !a.rect.intersects(b.rect),
                            "atlas nodes {} and {} overlap at scale {scale}",
                            a.name,
                            b.name
                        );
                    }
                }
                assert_inside(
                    "atlas hub",
                    scale,
                    crate::app::PlexiApp::atlas_hub_rect(screen),
                    screen,
                );
            });
        }
    }

    /// Cmd+= steps the one global scale: egui's zoom factor follows, and a
    /// terminal pane's effective pixel font size grows with it.
    #[test]
    fn ui_scale_up_key_scales_zoom_factor_and_terminal_pixels() {
        let mut h = PlexiUiHarness::new_sized(900.0, 600.0);
        h.step();
        let pane_id = h.with_app_mut(|app| {
            app.sidebar_visible = false;
            let workspace_root = app.router.active().root.clone();
            let id = app.host.alloc_pane_id();
            let terminal = crate::host::pane::TerminalPane::new(
                id,
                app.ctx.clone(),
                app.pty_event_tx.clone(),
                egui_term::BackendSettings {
                    shell: "/bin/sh".into(),
                    working_directory: Some(workspace_root),
                    ..Default::default()
                },
                14.0,
            )
            .expect("spawn terminal pane");
            let win = &mut app.windows[app.active_window];
            win.panes.insert(id, Pane::Terminal(Box::new(terminal)));
            let tile = win.tree.tiles.insert_pane(id);
            win.tree.root = Some(tile);
            win.focused_pane = Some(tile);
            id
        });
        h.run_steps(3);
        let pixel_font = |h: &PlexiUiHarness| {
            h.with_app(|app| {
                let pane = app.windows[app.active_window].panes.get(&pane_id).unwrap();
                let Pane::Terminal(t) = pane else { panic!("terminal pane") };
                t.font_size * app.ctx.zoom_factor()
            })
        };
        let before = pixel_font(&h);
        assert!((h.with_app(|a| a.ctx.zoom_factor()) - 1.0).abs() < 1e-4);

        h.harness()
            .key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Equals);
        h.run_steps(3);
        h.with_app(|app| {
            assert!((app.ui_scale - 1.1).abs() < 1e-4, "ui_scale {}", app.ui_scale);
            assert!(
                (app.ctx.zoom_factor() - 1.1).abs() < 1e-4,
                "zoom {}",
                app.ctx.zoom_factor()
            );
        });
        assert!(pixel_font(&h) > before, "terminal pixel font must grow");

        h.harness()
            .key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Minus);
        h.harness()
            .key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Minus);
        h.run_steps(3);
        h.with_app(|app| assert!((app.ui_scale - 0.9).abs() < 1e-4, "{}", app.ui_scale));
        assert!(pixel_font(&h) < before, "terminal pixel font must shrink");

        // Cmd+Shift+0 resets (and is not swallowed by Quick Note on Cmd+0).
        h.harness().key_press_modifiers(
            egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
            egui::Key::Num0,
        );
        h.run_steps(3);
        h.with_app(|app| {
            assert!((app.ui_scale - 1.0).abs() < 1e-4);
            assert!(
                !app.focus_stack
                    .contains(&crate::app::FocusKind::QuickNote),
                "reset chord must not open Quick Note"
            );
        });
    }

    #[test]
    fn screenshot_ui_scale_cards() {
        let scale = ui_scale_shot();
        crate::app::card_gallery::FORCE.with(|f| f.set(true));
        let mut h = PlexiUiHarness::new_sized_scaled(1440.0, 1400.0, scale);
        h.run_steps(4);
        let path = evidence_png!();
        h.save_screenshot(&path).unwrap();
        assert!(std::path::Path::new(&path).exists());
    }

    #[test]
    fn screenshot_ui_scale_atlas() {
        let scale = ui_scale_shot();
        let mut h = PlexiUiHarness::new_sized_scaled(1440.0, 900.0, scale);
        add_focused_pane(&mut h);
        h.with_app_mut(|app| {
            let root = app.create_context_without_pty("alpha", None);
            app.create_context_without_pty("beta", Some(root));
            app.create_context_without_pty("gamma", Some(root));
            app.create_context_without_pty("delta", None);
            app.set_zoom_level_with_transition(crate::app::zoom::ZoomLevel::TreeAtlas);
        });
        h.run_steps(6);
        let path = evidence_png!();
        h.save_screenshot(&path).unwrap();
        assert!(std::path::Path::new(&path).exists());
    }

    #[test]
    fn screenshot_ui_scale_note_picker() {
        let scale = ui_scale_shot();
        let mut h = PlexiUiHarness::new_sized_scaled(1440.0, 900.0, scale);
        add_focused_pane(&mut h);
        seed_note_picker(&mut h);
        h.run_steps(3);
        let path = evidence_png!();
        h.save_screenshot(&path).unwrap();
        assert!(std::path::Path::new(&path).exists());
    }

    #[test]
    fn screenshot_ui_scale_terminal() {
        let scale = ui_scale_shot();
        let mut h = PlexiUiHarness::new_sized_scaled(1440.0, 900.0, scale);
        h.with_app_mut(|app| {
            app.sidebar_visible = false;
            let workspace_root = app.router.active().root.clone();
            let id = app.host.alloc_pane_id();
            let terminal = crate::host::pane::TerminalPane::new(
                id,
                app.ctx.clone(),
                app.pty_event_tx.clone(),
                egui_term::BackendSettings {
                    shell: "/bin/sh".into(),
                    working_directory: Some(workspace_root),
                    ..Default::default()
                },
                14.0,
            )
            .expect("spawn terminal pane");
            let win = &mut app.windows[app.active_window];
            win.panes.insert(id, Pane::Terminal(Box::new(terminal)));
            let tile = win.tree.tiles.insert_pane(id);
            win.tree.root = Some(tile);
            win.focused_pane = Some(tile);
        });
        h.run_steps(6);
        let path = evidence_png!();
        h.save_screenshot(&path).unwrap();
        assert!(std::path::Path::new(&path).exists());
    }

    #[test]
    fn screenshot_agent_conjoined_strip_and_parent_pin() {
        let mut h = PlexiUiHarness::new_sized(1440.0, 900.0);
        h.step();
        let parent = h.with_app_mut(|app| {
            let (parent, _) = app.spawn_agent_worker_pane("lead", "pi", "printf 'Lead: review implementation and testing\\n'; sleep 120", "Coordinate review", None, false, None, None, None, None, false, None).unwrap();
            let (_, tile) = app.find_pane_in_any_window(parent).unwrap();
            app.windows[app.active_window].focused_pane = Some(tile);
            for role in ["implementer", "tester"] {
                let (child, _) = app.spawn_agent_worker_pane(role, "pi", "printf 'Worker: checking the requested changes\\n'; sleep 120", "Verify child call", Some("agent-tools-preview"), true, Some(parent), None, None, None, true, None).unwrap();
                app.agent_tool_children.insert(child, Some(parent));
            }
            parent
        });
        h.run_steps(4);
        h.save_screenshot(&evidence_png!("_strip")).unwrap();
        h.harness().key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::ArrowDown);
        h.run_steps(4);
        h.with_app(|app| {
            assert_eq!(app.conjoined_parent(), Some(parent));
            assert_eq!(app.visual_owner_pane(), Some(parent));
            assert_eq!(app.fleet_focused_pane(), Some(parent));
            let parent_rect = app.visible_pane_rect(parent).unwrap();
            let child_rect = app.windows[app.active_window].tree.root.and_then(|tile| app.windows[app.active_window].tree.tiles.rect(tile)).unwrap();
            let share = parent_rect.width() / (parent_rect.width() + child_rect.width());
            assert!((share - 0.35).abs() < 0.01, "parent share: {share}");
            assert!(!app.windows.iter().flat_map(|w| w.panes.values()).any(|p| matches!(p, Pane::Portal(_))));
        });
        h.save_screenshot(&evidence_png!("_view")).unwrap();
        h.harness().key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Escape);
        h.run_steps(2);
        // Cmd+Esc is the zoom-out ladder now: it leaves the column strip for
        // the sibling cards rather than popping the conjoined view.
        h.with_app(|app| assert_eq!(app.zoom.level, crate::app::zoom::ZoomLevel::SiblingCards));
    }

    fn wait_for_lineage_fixture(h: &mut PlexiUiHarness, panes: &[u64]) {
        let deadline = Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(20));
        loop {
            h.run_steps(2);
            let ready = h.with_app_mut(|app| panes.iter().all(|id| {
                let (win, _) = app.find_pane_in_any_window(*id).unwrap();
                app.windows[win].panes.get_mut(id).unwrap().as_terminal_mut().unwrap()
                    .backend.capture_lines(30).iter().any(|line| line.contains("Lead:") || line.contains("Researcher:"))
            }));
            if ready { break; }
            assert!(Instant::now() < deadline, "lineage terminal fixture timed out");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    // Agent lineage and pinned-parent input ownership cannot be seeded by scenes.
    #[test]
    fn screenshot_child_lineage_and_focus_outline() {
        let mut h = PlexiUiHarness::new_sized(1440.0, 900.0);
        h.step();
        let (parent, child) = h.with_app_mut(|app| {
            let (parent, _) = app.spawn_agent_worker_pane("lead", "pi", "printf 'Lead: review research\\n'; sleep 120", "Coordinate review", None, false, None, None, None, None, false, None).unwrap();
            let (child, _) = app.spawn_agent_worker_pane("researcher", "pi", "printf 'Researcher: checking source evidence\\n'; sleep 120", "Review evidence", Some("lineage-preview"), true, Some(parent), None, None, None, true, None).unwrap();
            assert!(app.dive_into_agent_children(parent));
            (parent, child)
        });
        wait_for_lineage_fixture(&mut h, &[parent, child]);
        h.focus_pane(child).unwrap();
        h.run_steps(3);
        let (rect, accent, gap) = h.with_app(|app| {
            assert_eq!(app.visual_owner_pane(), Some(child));
            assert_ne!(app.visual_owner_pane(), Some(parent));
            (app.visible_pane_rect(child).unwrap(), app.colors.accent, app.config.pane_gap.unwrap_or(4.0))
        });
        assert!(h.harness().output().shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::Shape::Rect(r) if r.rect == rect
                && r.stroke.color == accent && r.stroke.width == gap
                && r.stroke_kind == egui::StrokeKind::Outside)
        }), "focused child must have the ordinary outside accent outline");
        let mut texts = Vec::new();
        for shape in &h.harness().output().shapes {
            gallery_text_shapes(&shape.shape, &mut texts);
        }
        assert!(texts.iter().any(|(_, text)| text.contains("researcher") && text.contains("child of lead")), "{texts:?}");
        assert!(!texts.iter().any(|(_, text)| text.starts_with("↳ lead")));
        h.save_screenshot(&evidence_png!()).unwrap();
    }

    #[test]
    fn screenshot_portal_lineage_and_focus_outline() {
        let mut h = PlexiUiHarness::new_sized(1440.0, 900.0);
        h.step();
        let (parent, child, portal) = h.with_app_mut(|app| {
            let (parent, _) = app.spawn_agent_worker_pane("lead", "pi", "printf 'Lead: coordinate research\\n'; sleep 120", "Review", None, false, None, None, None, None, false, None).unwrap();
            let (child, context) = app.spawn_agent_worker_pane("researcher", "pi", "printf 'Researcher: working\\n'; sleep 120", "Research", Some("research"), true, Some(parent), None, None, None, false, None).unwrap();
            let portal = app.windows.iter().flat_map(|w| w.panes.iter())
                .find_map(|(&id, p)| (p.portal_target() == Some(context)).then_some(id)).unwrap();
            (parent, child, portal)
        });
        wait_for_lineage_fixture(&mut h, &[parent, child]);
        h.focus_pane(portal).unwrap();
        h.run_steps(3);
        let (rect, accent, gap) = h.with_app(|app| {
            assert!(!app.pane_lineage().contains_key(&parent));
            (app.visible_pane_rect(portal).unwrap(), app.colors.accent, app.config.pane_gap.unwrap_or(4.0))
        });
        assert!(h.harness().output().shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::Shape::Rect(r) if r.rect == rect
                && r.stroke.color == accent && r.stroke.width == gap
                && r.stroke_kind == egui::StrokeKind::Outside)
        }));
        assert!(h.harness().query_by_label("↳ research · child of lead").is_some());
        h.save_screenshot(&evidence_png!()).unwrap();
        h.with_app_mut(|app| {
            let (win, _) = app.find_pane_in_any_window(parent).unwrap();
            app.windows[win].panes.get_mut(&parent).unwrap().as_terminal_mut().unwrap().name = None;
            assert_eq!(app.pane_lineage().get(&portal), Some(&parent.to_string()));
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

    #[derive(Clone)]
    struct PressSnapshot {
        fill: egui::Color32,
        rect: egui::Rect,
        clicks: usize,
    }

    struct PressFixture {
        kind: u8,
        snapshot: Arc<std::sync::Mutex<PressSnapshot>>,
    }

    impl crate::app::app_trait::App for PressFixture {
        fn type_id(&self) -> &'static str {
            "press_fixture"
        }
        fn display_name(&self) -> String {
            "Button press feedback".into()
        }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
        fn ui(
            &mut self,
            ui: &mut egui::Ui,
            ctx: &crate::app::app_trait::AppRenderContext<'_>,
            _: Option<crate::host::pane::PendingPaneClick>,
        ) {
            let layer = ui.layer_id();
            let start = ui
                .ctx()
                .graphics(|g| g.get(layer).map_or(0, |p| p.all_entries().len()));
            let clicked = match self.kind {
                0 => crate::ui::button::chrome_button(
                    ui,
                    "Confirm changes",
                    crate::ui::button::ButtonKind::Accent,
                    ctx.colors,
                    160.0,
                )
                .clicked(),
                1 => {
                    crate::ui::button::choice_button(ui, "Keep this version", "1", true, ctx.colors)
                        .clicked()
                }
                _ => {
                    use crate::host::wasm_app::bindings::plexi::platform::types::ButtonNode;
                    use crate::host::wasm_app::{ButtonStyle, IndexedNode, UiNodeData, UiTree};
                    let tree = UiTree {
                        root: 0,
                        nodes: vec![IndexedNode {
                            id: 0,
                            key: "press".into(),
                            data: UiNodeData::Button(ButtonNode {
                                label: "Save document".into(),
                                on_click: "save".into(),
                                style: ButtonStyle::Primary,
                                disabled: false,
                            }),
                        }],
                    };
                    !crate::host::wasm_render::render_ui_tree_with_surface(
                        ui, &tree, ctx.colors, None, None, 0,
                    )
                    .actions
                    .is_empty()
                }
            };
            let painted = ui.ctx().graphics(|g| {
                g.get(layer).and_then(|list| {
                    list.all_entries().skip(start).find_map(|shape| {
                        if let egui::Shape::Rect(rect) = &shape.shape {
                            // The app background spans the pane; the button has rounded corners.
                            if rect.corner_radius != egui::CornerRadius::ZERO
                                && rect.fill != egui::Color32::TRANSPARENT
                            {
                                return Some((rect.rect, rect.fill));
                            }
                        }
                        None
                    })
                })
            });
            if let Some((rect, fill)) = painted {
                let mut snapshot = self.snapshot.lock().unwrap();
                snapshot.rect = rect;
                snapshot.fill = fill;
                snapshot.clicks += usize::from(clicked);
            }
        }
    }

    #[test]
    fn button_pointer_down_paints_active_before_release() {
        for kind in 0..3 {
            let mut h = PlexiUiHarness::new_sized(720.0, 420.0);
            let pane_id = add_focused_pane(&mut h);
            let snapshot = Arc::new(std::sync::Mutex::new(PressSnapshot {
                fill: egui::Color32::TRANSPARENT,
                rect: egui::Rect::NOTHING,
                clicks: 0,
            }));
            h.with_app_mut(|app| {
                if let Some(Pane::App(pane)) =
                    app.windows[app.active_window].panes.get_mut(&pane_id)
                {
                    pane.runtime = AppRuntime::Builtin(Box::new(PressFixture {
                        kind,
                        snapshot: snapshot.clone(),
                    }));
                }
            });
            h.run_steps(3);
            let idle = snapshot.lock().unwrap().clone();
            assert!(idle.rect.is_positive(), "kind={kind}");
            let pos = idle.rect.center();
            h.harness().input_mut().events.extend([
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
            h.run_steps(1);
            let pressed = snapshot.lock().unwrap().clone();
            assert_eq!(pressed.clicks, 0, "press must not activate kind={kind}");
            assert_ne!(pressed.fill, idle.fill, "active fill kind={kind}");
            assert!(
                (pressed.rect.width() / idle.rect.width() - 0.97).abs() < 0.001,
                "kind={kind}: {:?} -> {:?}",
                idle.rect,
                pressed.rect
            );
            assert!((pressed.rect.height() / idle.rect.height() - 0.97).abs() < 0.001);
            assert!(pressed.rect.center().distance(idle.rect.center()) < 0.01);
            h.save_screenshot(&evidence_png!(&format!("_{kind}")))
                .expect("press screenshot");
            h.run_steps(2);
            assert_eq!(snapshot.lock().unwrap().clicks, 0);
            h.harness()
                .input_mut()
                .events
                .push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                });
            h.run_steps(1);
            let released = snapshot.lock().unwrap().clone();
            assert_eq!(
                released.clicks, 1,
                "release activates exactly once kind={kind}"
            );
            assert!((released.rect.width() - idle.rect.width()).abs() < 0.01);
        }
    }

    #[test]
    fn screenshot_micro_motion_floating_shells() {
        let mut h = PlexiUiHarness::new_sized(1100.0, 900.0);
        add_focused_pane(&mut h);
        h.with_app_mut(|app| {
            app.show_ui_gallery = true;
            app.quit_press_count = 1;
            app.quit_last_press = None;
        });
        h.run_steps(3);
        assert!(h.harness().query_by_label("Host UI Gallery").is_some());
        assert!(
            h.harness()
                .query_by_label("⌘Q pressed 1 of 3 — press again to quit")
                .is_some()
        );
        h.save_screenshot(&evidence_png!())
            .expect("floating shells screenshot");
    }

    #[test]
    fn screenshot_file_browser_middle_elision_distinguishes_suffixes() {
        let dir = tempfile::tempdir().unwrap();
        let prefix = "Q3 Board Deck — FINAL (revised) v12 [approved by legal]";
        for suffix in ["-signed.pdf", "-draft.HEIC"] {
            std::fs::write(dir.path().join(format!("{prefix}{suffix}")), "fixture").unwrap();
        }
        let mut h = PlexiUiHarness::new_sized(520.0, 420.0);
        h.open_file_browser(dir.path().to_path_buf());
        h.run_steps(3);
        let mut labels = Vec::new();
        for shape in &h.harness().output().shapes {
            if let egui::Shape::Text(text) = &shape.shape {
                if text.galley.text().starts_with("Q3") {
                    labels.push(text.galley.text().to_owned());
                }
            }
        }
        assert!(
            labels
                .iter()
                .any(|s| s.contains('…') && s.ends_with("-signed.pdf")),
            "{labels:?}"
        );
        assert!(
            labels
                .iter()
                .any(|s| s.contains('…') && s.ends_with("-draft.HEIC")),
            "{labels:?}"
        );
        h.save_screenshot(&evidence_png!())
            .expect("filename screenshot");
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
            app.handle_pane_ipc_request(crate::protocol::AppRequest::KeyPane {
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
            .key_down_modifiers(egui::Modifiers::NONE, egui::Key::F2);
        h.step();
        h.harness()
            .key_up_modifiers(egui::Modifiers::NONE, egui::Key::F2);
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

    /// Stint 0750 evidence: the file-browser rename modal's action pair now
    /// renders through `chrome_button` (`Primary` / `Secondary`) at
    /// `BUTTON_H_MD`, instead of the bare `ui.button` it used to emit inside
    /// an otherwise correct `ModalShell`. Review artifact:
    /// /tmp/plexi-0750-file-browser-rename-modal.png.
    #[test]
    fn screenshot_file_browser_rename_modal_uses_kit_buttons() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("original.txt"), b"contents").expect("write fixture");

        let mut h = PlexiUiHarness::new_sized(720.0, 520.0);
        h.open_file_browser(dir.path().to_path_buf());
        h.run_steps(2);
        h.harness()
            .key_down_modifiers(egui::Modifiers::NONE, egui::Key::F2);
        h.step();
        h.harness()
            .key_up_modifiers(egui::Modifiers::NONE, egui::Key::F2);
        h.step();
        h.run_steps(3);

        h.save_screenshot("/tmp/plexi-0750-file-browser-rename-modal.png")
            .expect("render rename modal");
    }

    /// Stint 0788: a pane's `plexi secret get` shows on the desktop banner
    /// with the secret and its workspace, so the person knows what Allow once
    /// releases.
    #[test]
    fn screenshot_secret_get_banner_names_the_secret() {
        let profile = tempfile::tempdir().expect("profile");
        let _profile_guard = crate::config::set_test_profile_dir(profile.path().to_path_buf());
        let root = tempfile::tempdir().expect("workspace");
        let monitor =
            crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir());
        let scope = crate::workspace::secrets::SecretScope::Workspace {
            id: "ws-banner".to_string(),
            root: root.path().to_path_buf(),
        };
        let actor = crate::workspace::secrets::SecretActor {
            kind: crate::broker::ActorType::Agent,
            id: "pane:4".to_string(),
        };
        let store = crate::workspace::secrets::InMemoryKeychain::new();
        let read = crate::workspace::secrets::read_pane_secret(
            &monitor,
            &store,
            &actor,
            "OPENAI_API_KEY",
            &scope,
            profile.path(),
        )
        .expect("read admits");
        assert!(matches!(read, crate::workspace::secrets::ReadResult::PermissionRequired { .. }));

        let mut h = PlexiUiHarness::new_sized(1100.0, 300.0);
        h.run_steps(2);
        // The old approval banner is gone: the ask is an attention-queue item
        // that names the secret and its workspace.
        h.with_app_mut(|app| app.sync_attention_queue());
        let (summary, command_text) = h.with_app(|app| {
            let ask = app
                .attention_queue
                .items
                .iter()
                .find(|b| b.category == crate::app::attention_queue::QueueCategory::Ask)
                .expect("secret.get ask is queued");
            (ask.summary.clone(), ask.command_text.clone())
        });
        assert!(summary.contains("wants to use secret.get"), "summary: {summary}");
        assert!(command_text.contains("OPENAI_API_KEY"), "command: {command_text}");
        assert!(
            command_text.contains(&root.path().display().to_string()),
            "command: {command_text}"
        );
        // Arrival alone never captures keys or grants consent.
        h.harness().key_down_modifiers(egui::Modifiers::NONE, egui::Key::A);
        h.step();
        h.harness().key_up_modifiers(egui::Modifiers::NONE, egui::Key::A);
        h.step();
        assert_eq!(monitor.list_pending().len(), 1);
        h.save_screenshot(&evidence_png!()).expect("render banner");
    }

    /// An unreadable host seal key is recoverable from the desktop. The action
    /// is not a tool grant: it is the existing keychain retry/rotation path.
    #[test]
    fn screenshot_keychain_recovery_banner_is_actionable() {
        let profile = tempfile::tempdir().expect("profile");
        let _profile_guard = crate::config::set_test_profile_dir(profile.path().to_path_buf());
        let monitor =
            crate::broker::gate::PermissionMonitor::for_profile(&crate::config::config_dir());
        monitor
            .file_needs_you(crate::broker::gate::NeedsYouFile {
                kind: crate::broker::gate::NeedsYouKind::Keychain,
                actor: "host".to_string(),
                resource: "permission-mac".to_string(),
                summary: crate::broker::gate::KEYCHAIN_ACCESS_SUMMARY.to_string(),
                expires_at: None,
                run_tag: Some("keychain:permission-mac".to_string()),
            })
            .expect("file keychain recovery");

        let mut h = PlexiUiHarness::new_sized(1100.0, 360.0);
        h.run_steps(2);
        h.harness().get_by_label("Keychain access needed");
        h.harness().get_by_label("Saved permissions are unavailable.");
        h.harness().get_by_label("Continue");
        h.harness().get_by_label("Not now");
        h.save_screenshot(&evidence_png!())
            .expect("render keychain recovery");
    }

    /// Render the initial empty state and save a screenshot for visual inspection.
    /// File written to /tmp/plexi_init.png.
    #[test]
    fn screenshot_init() {
        let mut h = PlexiUiHarness::new();
        h.step();
        h.save_screenshot(&evidence_png!()).expect("render failed");
        println!("Screenshot saved to {}", evidence_png!());
    }

    /// Raw `.wasm` paths queue a host pre-launch review before spawning. The
    /// underlying launch path still fails closed until the modal persists the
    /// required link-time import grants.
    #[test]
    fn wasm_path_launch_queues_review_before_spawning() {
        let _experiments = crate::release::set_test_experiments(&[crate::release::ReleaseFeature::WasmApps]);
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
        let _experiments = crate::release::set_test_experiments(&[crate::release::ReleaseFeature::WasmApps]);
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
                name: "Notes".to_string().into(),
                root: editor_path.clone(),
                description: Some("Editing notes".to_string()),
                context_id: child_ctx_id,
                parent_id: Some(active_ctx_id),
                depth: 1,
                parked: false,
            });

            // Child window holding the text-editor pane the portal previews.
            let editor_pane_id = app.host.alloc_pane_id();
            let editor = AppPane {
                pip_status: Some(crate::protocol::PipStatus::Green),
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
        h.save_screenshot(&evidence_png!()).expect("render failed");
        assert!(
            h.with_app(|app| app.windows[app.active_window]
                .panes
                .values()
                .any(|p| p.portal_target().is_some())),
            "active window should contain a portal pane previewing the editor"
        );
        println!("Screenshot saved to {}", evidence_png!());
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
            pane_id: Some((x0 * 1000.0) as u64),
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
                    Some(crate::protocol::AgentState::Working),
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
            .save(evidence_png!())
            .expect("save screenshot");
    }

    #[test]
    fn screenshot_visual_escalation_panes_and_portals() {
        let mut h = PlexiUiHarness::new_sized_ppp(1440.0, 900.0, 2.0);
        let dir = tempfile::tempdir().expect("tempdir");
        h.step();

        let ws_root = h.workspace_root().to_path_buf();
        let master_pane = h
            .open_target(HarnessOpenTarget::Builtin {
                id: "file_browser",
                cwd: &ws_root,
                args: &[],
            })
            .expect("open initial master pane");
        h.step();

        let send_req = |h: &mut PlexiUiHarness, req: crate::protocol::AppRequest| {
            h.with_app_mut(|app| {
                app.handle_pane_ipc_request(req);
            });
            h.run_steps(2);
        };

        let spawn_in_subctx =
            |h: &mut PlexiUiHarness, subctx: &str, name: &str| -> (u64, u64) {
                let reply = dir.path().join(format!("spawn-{subctx}-{name}.json"));
                send_req(
                    h,
                    crate::protocol::AppRequest::AgentSpawn {
                        role: name.into(),
                        harness: "codex".into(),
                        command: "sleep 30".into(),
                        task: Some(format!("Task {name}")),
                        subcontext: Some(subctx.into()),
                        ephemeral: true,
                        parent_pane_id: Some(master_pane),
                        name: Some(name.into()),
                        cwd: None,
                        worktree: None,
                        result_schema: None,
                        initial_slots: vec![],
                        no_focus: true,
                        response_file: reply.to_string_lossy().into_owned(),
                        peer_ancestry: None,
                    },
                );
                let raw = std::fs::read_to_string(&reply).expect("read spawn response");
                let json: serde_json::Value = serde_json::from_str(&raw).expect("parse json");
                (
                    json["pane_id"].as_u64().expect("pane_id"),
                    json["portal_pane_id"].as_u64().expect("portal_pane_id"),
                )
            };

        // Subcontext 1 ("squad-alpha"): 2 WORKING · 1 BLOCKED -> amber PortalPane card.
        let (sq1_w1, portal_blocked) = spawn_in_subctx(&mut h, "squad-alpha", "worker-api");
        let (sq1_w2, _) = spawn_in_subctx(&mut h, "squad-alpha", "worker-db");
        let (sq1_b1, _) = spawn_in_subctx(&mut h, "squad-alpha", "worker-schema");
        for wid in [sq1_w1, sq1_w2] {
            send_req(
                &mut h,
                serde_json::from_value(serde_json::json!({
                    "type": "set_agent_state",
                    "pane_id": wid,
                    "state": "working",
                    "agent": "codex"
                }))
                .unwrap(),
            );
        }
        send_req(
            &mut h,
            serde_json::from_value(serde_json::json!({
                "type": "set_agent_state",
                "pane_id": sq1_b1,
                "state": "blocked",
                "agent": "codex",
                "detail": "Needs master decision on index"
            }))
            .unwrap(),
        );

        // Subcontext 2 ("squad-deploy"): 1 NEEDS YOU -> coral PortalPane card.
        let (sq2_ny, portal_needs_you) = spawn_in_subctx(&mut h, "squad-deploy", "release-gate");
        send_req(
            &mut h,
            serde_json::from_value(serde_json::json!({
                "type": "set_agent_state",
                "pane_id": sq2_ny,
                "state": "needs_you",
                "agent": "codex",
                "detail": "Approve production deploy"
            }))
            .unwrap(),
        );

        // Subcontext 3 ("squad-ci"): 1 ERROR -> danger PortalPane card.
        let (sq3_err, portal_error) = spawn_in_subctx(&mut h, "squad-ci", "ci-runner");
        h.with_app_mut(|app| {
            for win in &mut app.windows {
                if let Some(crate::host::pane::Pane::Terminal(t)) = win.panes.get_mut(&sq3_err) {
                    t.exited = true;
                    t.exit_status = Some(crate::host::pane_lifecycle::ExitStatus::Code(1));
                    t.agent = None;
                    t.observed_agent = None;
                    t.activity = None;
                }
            }
        });

        // Split visible terminal panes in the active window so Working, Blocked,
        // NeedsYou, and Error terminal headers/borders are all visible alongside
        // the three PortalPane cards and the sidebar.
        let split_pane = |h: &mut PlexiUiHarness, name: &str| -> u64 {
            let reply = dir.path().join(format!("split-{name}.json"));
            send_req(
                h,
                crate::protocol::AppRequest::AgentSpawn {
                    role: name.into(),
                    harness: "codex".into(),
                    command: "sleep 30".into(),
                    task: Some(format!("Task {name}")),
                    subcontext: None,
                    ephemeral: true,
                    parent_pane_id: Some(master_pane),
                    name: Some(name.into()),
                    cwd: None,
                    worktree: None,
                    result_schema: None,
                    initial_slots: vec![],
                    no_focus: true,
                    response_file: reply.to_string_lossy().into_owned(),
                    peer_ancestry: None,
                },
            );
            let raw = std::fs::read_to_string(&reply).expect("read split response");
            let json: serde_json::Value = serde_json::from_str(&raw).expect("parse json");
            json["pane_id"].as_u64().expect("pane_id")
        };

        let pane_working = split_pane(&mut h, "lane-working");
        let pane_blocked = split_pane(&mut h, "lane-blocked");
        let pane_needs_you = split_pane(&mut h, "lane-needs-you");
        let pane_error = split_pane(&mut h, "lane-error");

        // Set pane_working = Working, pane_blocked = Blocked, pane_needs_you = NeedsYou, pane_error = Error,
        // and arrange the active window into a clean 2-row layout:
        //   Top row:    [lane-working | lane-blocked | lane-needs-you | lane-error]
        //   Bottom row: [squad-alpha  | squad-deploy | squad-ci]
        let _ = h.close_pane(master_pane);
        h.with_app_mut(|app| {
            app.sidebar_visible = true;
            let win = &mut app.windows[app.active_window];
            let mut tiles = egui_tiles::Tiles::default();
            let t_work = tiles.insert_pane(pane_working);
            let t_blk = tiles.insert_pane(pane_blocked);
            let t_ny = tiles.insert_pane(pane_needs_you);
            let t_err = tiles.insert_pane(pane_error);
            let top_row = tiles.insert_horizontal_tile(vec![t_work, t_blk, t_ny, t_err]);

            let p_blk = tiles.insert_pane(portal_blocked);
            let p_ny = tiles.insert_pane(portal_needs_you);
            let p_err = tiles.insert_pane(portal_error);
            let bottom_row = tiles.insert_horizontal_tile(vec![p_blk, p_ny, p_err]);

            let root = tiles.insert_vertical_tile(vec![top_row, bottom_row]);
            win.tree = egui_tiles::Tree::new("escalation_grid", root, tiles);
            win.focused_pane = Some(t_work);
        });
        send_req(
            &mut h,
            serde_json::from_value(serde_json::json!({
                "type": "set_agent_state",
                "pane_id": pane_working,
                "state": "working",
                "agent": "codex"
            }))
            .unwrap(),
        );
        send_req(
            &mut h,
            serde_json::from_value(serde_json::json!({
                "type": "set_agent_state",
                "pane_id": pane_blocked,
                "state": "blocked",
                "agent": "codex",
                "detail": "Waiting on master"
            }))
            .unwrap(),
        );
        send_req(
            &mut h,
            serde_json::from_value(serde_json::json!({
                "type": "set_agent_state",
                "pane_id": pane_needs_you,
                "state": "needs_you",
                "agent": "codex",
                "detail": "Approve credentials"
            }))
            .unwrap(),
        );
        h.with_app_mut(|app| {
            if let Some(crate::host::pane::Pane::Terminal(t)) =
                app.windows[app.active_window].panes.get_mut(&pane_error)
            {
                t.exited = true;
                t.exit_status = Some(crate::host::pane_lifecycle::ExitStatus::Code(1));
                t.agent = None;
                t.observed_agent = None;
                t.activity = None;
            }
        });

        h.run_steps(4);

        h.with_app(|app| {
            let win = &app.windows[app.active_window];
            let p_blocked = win.panes[&portal_blocked]
                .as_portal()
                .expect("portal_blocked")
                .context_state
                .as_ref()
                .expect("context_state");
            assert_eq!(
                p_blocked.status,
                crate::host::context_state::ContextStatus::Blocked
            );
            assert_eq!(
                p_blocked.status_summary_badge().as_deref(),
                Some("[2 WORKING \u{b7} 1 BLOCKED]")
            );

            let p_needs_you = win.panes[&portal_needs_you]
                .as_portal()
                .expect("portal_needs_you")
                .context_state
                .as_ref()
                .expect("context_state");
            assert_eq!(
                p_needs_you.status,
                crate::host::context_state::ContextStatus::NeedsYou
            );
            assert_eq!(
                p_needs_you.status_summary_badge().as_deref(),
                Some("[1 NEEDS YOU]")
            );

            let p_error = win.panes[&portal_error]
                .as_portal()
                .expect("portal_error")
                .context_state
                .as_ref()
                .expect("context_state");
            assert_eq!(
                p_error.status,
                crate::host::context_state::ContextStatus::Error
            );
            assert_eq!(
                p_error.status_summary_badge().as_deref(),
                Some("[1 ERROR]")
            );

            // Named Working pane keeps its normal header bar without an escalation badge.
            let working_layout =
                crate::render::terminal_pane::last_rendered_tab_bar_layout(&app.ctx, pane_working)
                    .expect("named Working terminal pane still renders its name header bar");
            assert_eq!(
                working_layout.elided_label.as_deref(),
                Some("lane-working")
            );
        });

        h.save_screenshot(&evidence_png!()).expect("render failed");
    }

    #[test]
    fn unnamed_working_terminal_does_not_spawn_escalation_header_bar() {
        let mut h = PlexiUiHarness::new_sized(600.0, 400.0);
        h.step();
        let pane_id = h.with_app_mut(|app| {
            app.sidebar_visible = false;
            let workspace_root = app.router.active().root.clone();
            let id = app.host.alloc_pane_id();
            let mut terminal = crate::host::pane::TerminalPane::new(
                id,
                app.ctx.clone(),
                app.pty_event_tx.clone(),
                egui_term::BackendSettings {
                    shell: "/bin/sh".into(),
                    working_directory: Some(workspace_root.clone()),
                    ..Default::default()
                },
                14.0,
            )
            .expect("spawn unnamed terminal pane");
            terminal.outside_workspace_cached = false;
            terminal.outside_workspace_checked_at = Some(Instant::now());
            terminal.outside_workspace_root = Some(workspace_root);
            terminal.activity = Some(crate::protocol::AgentState::Working);

            let win = &mut app.windows[app.active_window];
            win.panes.insert(id, Pane::Terminal(Box::new(terminal)));
            let tile = win.tree.tiles.insert_pane(id);
            win.tree.root = Some(tile);
            win.focused_pane = Some(tile);
            id
        });

        h.run_steps(2);
        h.with_app(|app| {
            assert!(
                crate::render::terminal_pane::last_rendered_tab_bar_layout(&app.ctx, pane_id)
                    .is_none(),
                "unnamed single terminal pane in Working state must not render a 20px escalation header bar"
            );
        });

        // Escalating to Blocked on the same unnamed terminal pane MUST spawn the header bar with fallback "Terminal" title.
        h.with_app_mut(|app| {
            let term = app.windows[app.active_window]
                .panes
                .get_mut(&pane_id)
                .and_then(|p| p.as_terminal_mut())
                .expect("terminal pane");
            term.activity = Some(crate::protocol::AgentState::Blocked);
        });
        h.run_steps(2);
        h.with_app(|app| {
            let layout =
                crate::render::terminal_pane::last_rendered_tab_bar_layout(&app.ctx, pane_id)
                    .expect("Blocked state on unnamed terminal pane must spawn escalation header bar");
            assert_eq!(layout.elided_label.as_deref(), Some("Terminal"));
        });

        // Returning to Working removes the escalation header bar again.
        h.with_app_mut(|app| {
            let term = app.windows[app.active_window]
                .panes
                .get_mut(&pane_id)
                .and_then(|p| p.as_terminal_mut())
                .expect("terminal pane");
            term.activity = Some(crate::protocol::AgentState::Working);
        });
        h.run_steps(2);
        h.with_app(|app| {
            assert!(
                crate::render::terminal_pane::last_rendered_tab_bar_layout(&app.ctx, pane_id)
                    .is_none(),
                "returning to Working must clear the temporary escalation header bar"
            );
        });
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
                workspace_root: crate::testing::scratch_context_root("ui"),
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
        // via `split_note` in text_editor_app.rs). The file name and save
        // indicator share that header with a file opened from the browser.
        let file_name = path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        h.harness().get_by_label("Groceries");
        h.harness().get_by_label(&file_name);
        h.harness().get_by_label("Saved");
        h.save_screenshot(&evidence_png!()).expect("render failed");

        let _ = std::fs::remove_file(&path);
    }

    /// A markdown file opened outside the notes tier (the file-browser path)
    /// uses the same header as a note: the file name and the save indicator.
    #[test]
    fn text_editor_file_header_shows_file_name_and_save_indicator() {
        let mut h = PlexiUiHarness::new();
        h.step();

        let path =
            std::env::temp_dir().join(format!("plexi-ui-file-header-{}.md", std::process::id()));
        std::fs::write(&path, "# hello\n").expect("seed file");
        let file_name = path.file_name().unwrap().to_string_lossy().into_owned();

        h.with_app_mut(|app| {
            let pane_id = app.host.alloc_pane_id();
            let app_pane = AppPane {
                pip_status: None,
                id: pane_id,
                runtime: AppRuntime::Builtin(Box::new(
                    crate::app::text_editor_app::TextEditorApp::new(path.clone()),
                )),
                workspace_root: crate::testing::scratch_context_root("ui"),
                permissions: AppPermissions::builtin(),
                manifest_id: "text-editor".to_string(),
                name: file_name.clone(),
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

        h.harness().get_by_label(&file_name);
        h.harness().get_by_label("Saved");

        let _ = std::fs::remove_file(&path);
    }

    /// Stint 0786: a plain file shows the dirty dot, the close prompt and the
    /// conflict state in the header/prompt row; closing a dirty file through
    /// the host close path is vetoed until the user answers.
    #[test]
    fn text_editor_file_dirty_close_and_conflict_surfaces() {
        let mut h = PlexiUiHarness::new_sized(900.0, 320.0);
        h.step();
        let path = std::env::temp_dir().join(format!("plexi-ui-0786-{}.txt", std::process::id()));
        std::fs::write(&path, "fn main() {}\n").expect("seed file");
        let file_name = path.file_name().unwrap().to_string_lossy().into_owned();

        let pane_id = h.with_app_mut(|app| {
            let pane_id = app.host.alloc_pane_id();
            let app_pane = AppPane {
                pip_status: None,
                id: pane_id,
                runtime: AppRuntime::Builtin(Box::new(
                    crate::app::text_editor_app::TextEditorApp::new(path.clone()),
                )),
                workspace_root: crate::testing::scratch_context_root("ui"),
                permissions: AppPermissions::builtin(),
                manifest_id: "text-editor".to_string(),
                name: file_name.clone(),
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
            pane_id
        });
        let with_editor = |h: &mut PlexiUiHarness,
                           f: &mut dyn FnMut(&mut crate::app::text_editor_app::TextEditorApp)| {
            h.with_app_mut(|app| {
                let win = &mut app.windows[app.active_window];
                if let Some(Pane::App(a)) = win.panes.get_mut(&pane_id) {
                    if let AppRuntime::Builtin(b) = &mut a.runtime {
                        let ed = b
                            .as_any_mut()
                            .downcast_mut::<crate::app::text_editor_app::TextEditorApp>()
                            .expect("text editor");
                        f(ed);
                    }
                }
            });
        };
        h.run_steps(2);
        h.harness().get_by_label("Saved");

        // Dirty: the dot and the Unsaved label, and no autosave to disk.
        with_editor(&mut h, &mut |ed| {
            ed.test_set_unsaved("fn main() { println!(\"edited\"); }\n")
        });
        h.run_steps(2);
        h.harness().get_by_label("Unsaved");
        // The zoom path bar's column pips also render "●", so only require the dirty dot.
        assert!(h.harness().get_all_by_label("\u{25CF}").count() >= 1);
        h.save_screenshot(&evidence_png!("_dirty")).expect("render dirty");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "fn main() {}\n");

        // Host close path: vetoed, prompt shown, pane still there.
        h.with_app_mut(|app| {
            assert!(app.focused_app_vetoes_close(), "dirty plain file vetoes close");
            assert!(app.windows[app.active_window].panes.contains_key(&pane_id));
        });
        h.run_steps(2);
        h.harness().get_by_label("Unsaved changes.");
        h.save_screenshot(&evidence_png!("_close_prompt")).expect("render close prompt");

        // Cancel, then an agent writes the file: conflict state.
        with_editor(&mut h, &mut |ed| {
            use crate::app::app_trait::App as _;
            assert!(!ed.request_close());
        });
        std::fs::write(&path, "agent version\n").expect("external write");
        with_editor(&mut h, &mut |ed| {
            assert!(ed.test_check_disk());
        });
        h.run_steps(2);
        h.harness().get_by_label("Changed on disk");
        h.save_screenshot(&evidence_png!("_conflict")).expect("render conflict");

        with_editor(&mut h, &mut |ed| ed.test_discard_unsaved());
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
                workspace_root: crate::testing::scratch_context_root("ui"),
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
        h.save_screenshot(&evidence_png!()).expect("render failed");

        let _ = std::fs::remove_file(&path);
    }

    /// Visual review surface for tiered notes (stint 0746): the picker as a root
    /// context's "master view" — the active context's own tier unchipped, notes
    /// rolled up from nested tiers chipped with their relative path, and the
    /// global tier last. Geometry assertions cannot prove the chips read well, so
    /// this renders realistic seeded content.
    #[test]
    fn screenshot_notes_picker_tier_chips() {
        let mut h = PlexiUiHarness::new_sized(900.0, 760.0);
        h.step();
        add_focused_pane(&mut h);
        h.step();

        h.with_app_mut(|app| {
            app.open_notes_picker();
            let mk =
                |title: &str, preview: &str, label: Option<&str>| crate::notes::NotePickerEntry {
                    path: std::env::temp_dir().join(format!("plexi-tier-chip-{title}.md")),
                    title: title.to_string(),
                    preview: preview.to_string(),
                    search_text: title.to_lowercase(),
                    tier_label: label.map(|s| s.to_string()),
                };
            app.notes_picker_entries = vec![
                mk(
                    "north star — what v1 has to prove",
                    "one workspace you never leave",
                    None,
                ),
                mk("pricing notes", "seat-based vs usage, land on seats", None),
                mk(
                    "wasm wire format decisions",
                    "postcard over JSON for frame payloads",
                    Some("crates/runtime"),
                ),
                mk(
                    "font bootstrap ordering trap",
                    "ui-medium binds on the next frame, not this one",
                    Some("crates/runtime"),
                ),
                mk(
                    "onboarding copy pass",
                    "cut the second paragraph entirely",
                    Some("website"),
                ),
                mk(
                    "ask Sam about the staging cert",
                    "expires 2026-08-03, renewal is manual",
                    Some("global"),
                ),
                mk(
                    "release checklist",
                    "tag, changelog, notarize, publish",
                    Some("global"),
                ),
            ];
            app.notes_picker_selected = 0;
        });
        h.run_steps(2);

        let out = evidence_png!();
        h.save_screenshot(&out).expect("render failed");
        h.with_app(|app| {
            assert!(
                app.notes_picker_entries
                    .iter()
                    .any(|e| e.tier_label.as_deref() == Some("crates/runtime")),
                "nested-tier rows must reach the render with their relative label"
            );
            assert!(
                app.notes_picker_entries
                    .iter()
                    .any(|e| e.tier_label.as_deref() == Some("global")),
                "global-tier rows must reach the render chipped as global"
            );
        });
        println!("Screenshot saved to {out}");
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
            let mk = |name: &str, label: Option<&str>| crate::notes::NotePickerEntry {
                path: std::env::temp_dir().join(format!("plexi-ui-{name}.md")),
                title: name.to_string(),
                preview: format!("{name} preview"),
                search_text: name.to_lowercase(),
                tier_label: label.map(|s| s.to_string()),
            };
            app.notes_picker_entries =
                vec![mk("context-note", None), mk("global-note", Some("global"))];
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
        // ListRow titles are painted as galleys (not accessible labels); assert on
        // the tier section headers, which are real ui.label widgets. "Notes"
        // appears exactly once — the modal title. The primary tier gets no header
        // of its own, so it never duplicates the title.
        let notes_labels = h.harness().get_all_by_label("Notes").count();
        assert_eq!(notes_labels, 1, "the modal title, with no duplicate header");
        h.harness().get_by_label("global");
        h.save_screenshot(&evidence_png!()).expect("render failed");
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
        h.save_screenshot(&evidence_png!()).expect("render failed");
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
        h.save_screenshot(&evidence_png!()).expect("render failed");
    }

    #[test]
    fn parked_sidebar_dropdown_uses_shared_list_header() {
        let mut h = PlexiUiHarness::new_sized(900.0, 620.0);
        h.with_app_mut(|app| {
            app.sidebar_visible = true;
            app.parked_section_expanded = false;
            let mk = |id: u64, name: &str| crate::host::context::Context {
                name: name.to_string().into(),
                root: std::env::temp_dir().join(name),
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
        h.save_screenshot(&evidence_png!()).expect("render failed");

        // Needs-you banners and context rows can move the header. Click its
        // rendered hit target so those layout changes cannot miss the control.
        let click = h
            .harness()
            .ctx
            .read_response(egui::Id::new("parked_divider"))
            .expect("parked dropdown header should be rendered")
            .rect
            .center();
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
        h.save_screenshot(&evidence_png!()).expect("render failed");
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
                name: "second".to_string().into(),
                root: std::env::temp_dir().join("second"),
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
        h.save_screenshot(&evidence_png!()).expect("render failed");
    }

    /// Stint 0241: the sidebar is master-context navigation only. Every slot is
    /// a numbered, keyboard-addressable top-level destination, so a subcontext
    /// is never rendered and never consumes a number — it is reached from
    /// inside its parent through the Portal tile instead. Removing the row must
    /// not remove the *signal*: the child's state still rolls up to the parent.
    #[test]
    fn sidebar_renders_and_numbers_master_contexts_only() {
        let mut h = PlexiUiHarness::new_sized(900.0, 620.0);
        let child_ctx_id = h.with_app_mut(|app| {
            app.sidebar_visible = true;
            app.router.get_mut(0).name = "master-one".to_string().into();
            let parent_ctx_id = app.router.get(0).context_id;
            let child_ctx_id = parent_ctx_id + 9_000;

            // A subcontext of the first master, sitting *between* the two
            // masters in router order — the position that used to shift the
            // second master's sidebar number from 2 to 3.
            app.router.push(crate::host::context::Context {
                name: "child-of-one".to_string().into(),
                root: std::env::temp_dir().join("child-of-one"),
                description: None,
                context_id: child_ctx_id,
                parent_id: Some(parent_ctx_id),
                depth: 1,
                parked: false,
            });
            app.router.push(crate::host::context::Context {
                name: "master-two".to_string().into(),
                root: std::env::temp_dir().join("master-two"),
                description: None,
                context_id: 72_002,
                parent_id: None,
                depth: 0,
                parked: false,
            });

            // Give the child a window so it has real state to roll up.
            let child_window_id = app.next_window_id;
            app.next_window_id += 1;
            app.windows.push(Window {
                name: "child window".to_string(),
                path: crate::testing::scratch_context_root("ui"),
                tree: egui_tiles::Tree::empty("child window"),
                panes: std::collections::HashMap::new(),
                focused_pane: None,
                zoomed_pane: None,
                grid_x: 0,
                grid_y: 1,
                window_id: child_window_id,
                context_id: child_ctx_id,
            });
            child_ctx_id
        });
        let child_window_idx = h.with_app(|app| app.windows.len() - 1);
        add_focused_pane_to_window(&mut h, child_window_idx);
        h.run_steps(2);

        // Rendered sidebar: both masters present, the subcontext absent.
        assert!(
            h.host_has_label("master-one"),
            "top-level contexts must render in the sidebar"
        );
        assert!(
            h.host_has_label("master-two"),
            "top-level contexts must render in the sidebar"
        );
        assert!(
            !h.host_has_label("child-of-one"),
            "a subcontext must never render a sidebar row"
        );

        h.with_app(|app| {
            // Numbering: the sidebar enumerates only masters, so master-two
            // holds slot 2 (Cmd+2) despite the subcontext preceding it.
            let order = app.router.top_level_order();
            let names: Vec<&str> = order
                .iter()
                .map(|&i| app.router.get(i).name.displayed())
                .collect();
            assert_eq!(
                names,
                vec!["master-one", "master-two"],
                "subcontexts must not be enumerated or shift top-level numbering"
            );

            // Rollup intact: the child is still observable from its parent,
            // which is what the Portal tile paints.
            let parent_state = crate::host::context_state::ContextState::compute(
                app.router.get(0).context_id,
                app.router.as_slice(),
                &app.windows,
            );
            let child_state = parent_state
                .children
                .iter()
                .find(|c| c.context_id == child_ctx_id)
                .expect("parent must still roll up its subcontext");
            assert_eq!(child_state.label, "child-of-one");
            assert_eq!(
                child_state.pane_count, 1,
                "child pane activity must still surface through the parent"
            );
        });

        h.save_screenshot(&evidence_png!()).expect("render failed");
    }

    /// Stint 0749: the sidebar context list has no vertical scroll, so once
    /// enough contexts stack up to exceed the panel's height the overflow
    /// rows were unreachable — no scrollbar, no way to bring them into view.
    /// Push enough contexts to overflow a short sidebar, then drive the
    /// accessibility `ScrollIntoView` action (the same path a real scroll
    /// gesture or keyboard nav ends up using) on the last row and confirm it
    /// actually moves into the visible sidebar area.
    #[test]
    fn sidebar_scrolls_to_reach_contexts_past_the_panel_height() {
        let mut h = PlexiUiHarness::new_sized(900.0, 320.0);
        h.with_app_mut(|app| app.sidebar_visible = true);
        h.step();

        for n in 0..20 {
            push_sidebar_context(&mut h, &format!("ctx-{n}"), &format!("/tmp/ctx-{n}"));
        }
        h.run_steps(3);

        // The harness window is 320pt tall, so anything laid out well past
        // that is outside the visible sidebar area — proof this test is
        // actually exercising overflow, not a coincidentally-fitting list.
        let last_rect_before = h.harness().get_by_label("ctx-19").rect();
        assert!(
            last_rect_before.min.y > 320.0,
            "test setup: the last context should start out past the visible sidebar \
             area (rect={:?}), otherwise this test isn't exercising overflow",
            last_rect_before
        );

        h.harness().get_by_label("ctx-19").scroll_to_me();
        h.run_steps(3);

        let last_rect_after = h.harness().get_by_label("ctx-19").rect();
        assert!(
            last_rect_after.min.y < last_rect_before.min.y,
            "scrolling the sidebar must move an overflowed context row toward the \
             visible area (before={:?}, after={:?}) — a bare Panel with no ScrollArea \
             never moves row rects in response to a scroll action",
            last_rect_before,
            last_rect_after
        );
    }

    /// Stint 0715: visual review surface for the first-run sidebar — exactly
    /// one context, so the close zone is suppressed and the row is at its
    /// widest text budget. This is the state that used to ramp the panel to
    /// its `size_range` maximum, clip the row card, and leave a dead strip
    /// between the sidebar and the central pane.
    #[test]
    fn screenshot_single_context_sidebar() {
        let mut h = PlexiUiHarness::new_sized_ppp(1280.0, 800.0, 2.0);
        h.with_app_mut(|app| app.sidebar_visible = true);
        h.run_steps(6);
        h.with_app(|app| {
            assert_eq!(app.router.len(), 1, "first-run is a single context");
        });
        h.save_screenshot(&evidence_png!()).expect("render failed");
    }

    /// Stint 0388: visual proof that creating a context lands directly in the
    /// workspace with its auto-derived sidebar label and no rename overlay.
    #[test]
    fn screenshot_new_context_without_rename_modal() {
        let mut h = PlexiUiHarness::new_sized_ppp(1280.0, 800.0, 2.0);
        h.with_app_mut(|app| {
            app.sidebar_visible = true;
            let before = app.router.len();
            app.new_context();
            assert_eq!(app.router.len(), before + 1, "new context created");
            assert_eq!(app.renaming_window, None, "rename overlay stays closed");
        });
        h.run_steps(6);
        h.save_screenshot(&evidence_png!()).expect("render failed");
    }

    /// Stint 0726: visual proof that the sidebar shows the new project's
    /// basename immediately after an explicit root change, not the stale
    /// automatic name from before the move.
    #[test]
    fn screenshot_sidebar_auto_name_follows_root_change() {
        let mut h = PlexiUiHarness::new_sized_ppp(1280.0, 800.0, 2.0);
        let parent = tempfile::tempdir().expect("tempdir");
        let new_root = parent.path().join("plexi-target-project");
        std::fs::create_dir(&new_root).expect("target project dir");

        h.with_app_mut(|app| {
            app.sidebar_visible = true;
            let idx = app.router.active_idx();
            app.router.get_mut(idx).name = crate::host::context::ContextName::auto("stale-name");
            app.set_context_root(new_root.clone(), None);
            assert_eq!(
                app.router.get(idx).name.displayed(),
                "plexi-target-project",
                "sidebar label must resolve to the new root's basename"
            );
        });
        h.run_steps(6);
        h.save_screenshot(&evidence_png!()).expect("render failed");
    }

    /// Push a second spatial window onto `context_id` and return its index, so
    /// a sidebar row can be seeded with more than one pip capsule.
    fn push_window_for_context(
        h: &mut PlexiUiHarness,
        context_id: u64,
        grid_x: u32,
        path: &str,
    ) -> usize {
        h.with_app_mut(|app| {
            let window_id = app.next_window_id;
            app.next_window_id += 1;
            app.windows.push(crate::host::context::Window {
                name: String::new(),
                path: std::path::PathBuf::from(path),
                tree: egui_tiles::Tree::empty("sidebar-row-states"),
                panes: std::collections::HashMap::new(),
                focused_pane: None,
                zoomed_pane: None,
                grid_x,
                grid_y: 0,
                window_id,
                context_id,
            });
            app.windows.len() - 1
        })
    }

    /// Fill `window_index` with `count` app panes wired into one horizontal
    /// container, so the window's tree really reports every pane — inserting
    /// panes without attaching them to the root leaves them invisible to the
    /// spatial walk the sidebar pips are built from.
    fn fill_window_with_panes(h: &mut PlexiUiHarness, window_index: usize, count: usize) {
        let browser_root = h.workspace.path().to_path_buf();
        h.with_app_mut(|app| {
            let mut tiles = Vec::with_capacity(count);
            for _ in 0..count {
                let pane_id = app.host.alloc_pane_id();
                let app_pane = AppPane {
                    pip_status: None,
                    id: pane_id,
                    runtime: AppRuntime::Builtin(Box::new(
                        crate::file_browser::FileBrowserApp::new(browser_root.clone()),
                    )),
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
                tiles.push(win.tree.tiles.insert_pane(pane_id));
            }
            let win = &mut app.windows[window_index];
            let container = win.tree.tiles.insert_horizontal_tile(tiles.clone());
            win.tree.root = Some(container);
            win.focused_pane = tiles.first().copied();
        });
    }

    /// Seed a context with a name, a root path, and its own window, returning
    /// `(router index, context id, window index)`.
    fn push_sidebar_context(h: &mut PlexiUiHarness, name: &str, root: &str) -> (usize, u64, usize) {
        let (idx, context_id) = h.with_app_mut(|app| {
            let context_id = app.next_window_id;
            app.next_window_id += 1;
            app.router.push(crate::host::context::Context {
                name: name.to_string().into(),
                root: std::path::PathBuf::from(root),
                description: None,
                context_id,
                parent_id: None,
                depth: 0,
                parked: false,
            });
            (app.router.len() - 1, context_id)
        });
        let window_idx = push_window_for_context(h, context_id, 0, root);
        (idx, context_id, window_idx)
    }

    fn seeded_notification(
        notify_id: &str,
        source_context_id: u64,
        source_window_id: u64,
    ) -> crate::app::PendingNotification {
        crate::app::PendingNotification {
            notify_id: notify_id.to_string(),
            sender_pane_id: 0,
            dismiss_owner_pane_id: 0,
            source_context_id,
            source_window_id,
            title: "Preview badge".to_string(),
            body: "Seeded sidebar notification".to_string(),
            kind: crate::protocol::NotifyKind::Message,
            options: vec![],
            input_prompt: None,
            required: false,
            scope: crate::protocol::NotifyScope::Context,
            image_inline: None,
            image_pipe_id: None,
            response_file: None,
            timeout_secs: None,
            on_dismiss: None,
            enqueued_at: std::time::Instant::now(),
            tombstoned: false,
            deliver_after: None,
            origin_in_view: false,
        }
    }

    /// Stint 0717: the whole vocabulary of the rebuilt two-tier context row in
    /// one frame, at HiDPI. Identity tier — index number, context name, and
    /// the close action pinned top-right. Pane-state tier — per-window pip
    /// capsules with the return-target pin and agent activity colors, the
    /// overflow count, the notification badge, and the root path trailing.
    /// Read the PNG: the number must sit on the name's optical center; the
    /// badge must read as a different kind of thing from the collapsed pips
    /// beside it; the capsule's top edge must be unbroken except where the pin
    /// meets it; and nothing on either tier may touch its neighbour.
    #[test]
    fn screenshot_sidebar_row_states() {
        const SHOT_PPP: f32 = 2.0;
        let mut h = PlexiUiHarness::new_sized_ppp(1100.0, 700.0, SHOT_PPP);
        h.step();

        let (active_context_id, active_window_id) = h.with_app_mut(|app| {
            app.sidebar_visible = true;
            let idx = app.router.active_idx();
            app.router.get_mut(idx).name = "PLEXI Workspace".to_string().into();
            app.router.get_mut(idx).root = std::path::PathBuf::from("/projects/PLEXI");
            (
                app.router.get(idx).context_id,
                app.windows[app.active_window].window_id,
            )
        });

        // Active context: two spatial windows, so the row carries two capsules
        // and the pin marks the one activation will return to.
        let active_window = h.with_app(|app| app.active_window);
        fill_window_with_panes(&mut h, active_window, 3);
        let second_window =
            push_window_for_context(&mut h, active_context_id, 1, "/projects/PLEXI");
        fill_window_with_panes(&mut h, second_window, 2);

        // A long name that must elide, and enough panes to overflow the cap.
        let (_, narrator_id, narrator_window) = push_sidebar_context(
            &mut h,
            "Narrator Video Production Workspace",
            "/projects/narrator-site",
        );
        fill_window_with_panes(&mut h, narrator_window, 11);
        let narrator_window_id = h.with_app(|app| app.windows[narrator_window].window_id);

        let (_, _, scratch_window) = push_sidebar_context(&mut h, "scratch", "/tmp/scratch");
        fill_window_with_panes(&mut h, scratch_window, 1);

        // Agent activity across the pips, and badges on two rows.
        h.with_app_mut(|app| {
            let statuses = [
                crate::protocol::PipStatus::Yellow,
                crate::protocol::PipStatus::Green,
                crate::protocol::PipStatus::Red,
            ];
            for (i, pane) in app.windows[0].panes.values_mut().enumerate() {
                if let Pane::App(app_pane) = pane {
                    app_pane.pip_status = Some(statuses[i % statuses.len()]);
                }
            }
            app.pending_notifications.extend([
                seeded_notification("active-one", active_context_id, active_window_id),
                seeded_notification("active-two", active_context_id, active_window_id),
                seeded_notification("active-three", active_context_id, active_window_id),
                seeded_notification("narrator-one", narrator_id, narrator_window_id),
                seeded_notification("narrator-two", narrator_id, narrator_window_id),
            ]);
        });
        h.run_steps(3);

        // Hover the inactive "scratch" row so one row shows its close glyph
        // while the active row shows its badge — the two states that used to
        // share a single trailing zone.
        // kittest reports node rects in physical pixels; pointer events are in
        // points, so the HiDPI scale has to come back out.
        let scratch_row = h.harness().get_by_label("scratch").rect().center() / SHOT_PPP;
        h.harness()
            .input_mut()
            .events
            .push(egui::Event::PointerMoved(scratch_row));
        h.step();

        h.save_screenshot(&evidence_png!()).expect("render failed");
    }

    /// Stint 0605: `NextContext`/`PrevContext` cycle the same enumeration the
    /// sidebar draws. Against a live router carrying a real subcontext wedged
    /// between two masters, cycling steps master → master and wraps, so it can
    /// never activate a context with no row, no number, and no recorded depth
    /// entry for zoom-out to pop.
    #[test]
    fn context_cycling_visits_master_contexts_only() {
        let mut h = PlexiUiHarness::new_sized(900.0, 620.0);
        let child_ctx_id = h.with_app_mut(|app| {
            app.router.get_mut(0).name = "master-one".to_string().into();
            let parent_ctx_id = app.router.get(0).context_id;
            let child_ctx_id = parent_ctx_id + 9_000;

            // Subcontext at router index 1, between the two masters — the
            // index the old raw-index walk would have landed on.
            app.router.push(crate::host::context::Context {
                name: "child-of-one".to_string().into(),
                root: std::env::temp_dir().join("child-of-one"),
                description: None,
                context_id: child_ctx_id,
                parent_id: Some(parent_ctx_id),
                depth: 1,
                parked: false,
            });
            app.router.push(crate::host::context::Context {
                name: "master-two".to_string().into(),
                root: std::env::temp_dir().join("master-two"),
                description: None,
                context_id: 72_002,
                parent_id: None,
                depth: 0,
                parked: false,
            });
            child_ctx_id
        });
        h.run_steps(2);

        h.with_app(|app| {
            // From master-one (idx 0): forward skips the subcontext at idx 1.
            let next = app.router.cycle_top_level(true).expect("next exists");
            assert_eq!(
                app.router.get(next).name,
                "master-two",
                "cycling must skip the subcontext sitting between the masters"
            );
            assert_ne!(
                app.router.get(next).context_id,
                child_ctx_id,
                "cycling must never activate a subcontext"
            );

            // Backward from the first master wraps to the last master, not to
            // the subcontext that precedes it in raw router order.
            let prev = app.router.cycle_top_level(false).expect("prev exists");
            assert_eq!(
                app.router.get(prev).name,
                "master-two",
                "cycling backward from the first master wraps to the last"
            );
        });
    }

    /// Render after adding a test pane.
    #[test]
    fn screenshot_with_pane() {
        let mut h = PlexiUiHarness::new();
        h.step();
        add_focused_pane(&mut h);
        h.step();
        h.save_screenshot(&evidence_png!()).expect("render failed");
        println!("Screenshot saved to {}", evidence_png!());
    }

    /// Scenes cannot seed terminal PTY output. Exercise the real terminal
    /// widget with the same ANSI fixture used by installed-host validation.
    #[test]
    fn screenshot_terminal_bold_and_shades() {
        for ppp in [1.0, 2.0] {
            let mut h = PlexiUiHarness::new_sized_ppp(1000.0, 900.0, ppp);
            h.step();
            let id = add_focused_pane(&mut h);
            let workspace = h.workspace_root().to_path_buf();
            h.with_app_mut(|app| {
                let terminal = crate::host::pane::TerminalPane::new(
                    id,
                    app.ctx.clone(),
                    app.pty_event_tx.clone(),
                    egui_term::BackendSettings {
                        shell: "/bin/sh".into(),
                        args: vec![concat!(
                            env!("CARGO_MANIFEST_DIR"),
                            "/tests/fixtures/terminal-rendering.sh"
                        )
                        .into()],
                        working_directory: Some(workspace),
                        ..Default::default()
                    },
                    18.0,
                )
                .expect("start terminal fixture");
                app.windows[app.active_window]
                    .panes
                    .insert(id, Pane::Terminal(Box::new(terminal)));
                app.sidebar_visible = false;
            });
            let deadline = std::time::Instant::now()
                + crate::testing::load_aware_timeout(Duration::from_secs(10));
            loop {
                h.run_steps(2);
                let ready = h.with_app_mut(|app| {
                    app.windows[app.active_window]
                        .panes
                        .get_mut(&id)
                        .and_then(|pane| pane.as_terminal_mut())
                        .expect("fixture terminal")
                        .backend
                        .capture_lines(50)
                        .iter()
                        .any(|line| line.contains("RENDER_FIXTURE_READY"))
                });
                if ready {
                    // `capture_lines` reads the live PTY grid directly, which
                    // can advance between `run_steps(2)` and `capture_lines`.
                    // Pump a settling pass so `TerminalView::sync()` and egui's
                    // `FullOutput` incorporate the completed fixture before
                    // `h.render()` rasterizes the frame (stint 0820).
                    h.run_steps(2);
                    break;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "terminal fixture timed out"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
            let path = evidence_png!(&format!("_ppp{ppp}"));
            let image = h.render().expect("render terminal fixture");
            image.save(&path).expect("save terminal screenshot");
            assert!(
                image
                    .pixels()
                    .filter(|p| p.0 == [117, 223, 255, 255])
                    .count()
                    > 100,
                "bold RGB slider must retain its requested foreground; screenshot: {path}"
            );
        }
    }

    /// Exercise egui input ownership and the real terminal widget/PTY writer.
    /// Scene keys cannot express separate press, repeat and release events.
    #[test]
    fn terminal_kitty_press_repeat_release_reaches_pty_once() {
        let mut h = PlexiUiHarness::new_sized(900.0, 600.0);
        h.step();
        let id = add_focused_pane(&mut h);
        h.with_app_mut(|app| {
            let terminal = crate::host::pane::TerminalPane::new(
                id,
                app.ctx.clone(),
                app.pty_event_tx.clone(),
                egui_term::BackendSettings {
                    shell: "/bin/sh".into(),
                    args: vec![concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/tests/fixtures/kitty-keyboard-probe.sh"
                    )
                    .into()],
                    working_directory: Some(std::env::temp_dir()),
                    ..Default::default()
                },
                18.0,
            )
            .expect("start keyboard fixture");
            terminal.backend.enable_input_tap();
            app.windows[app.active_window]
                .panes
                .insert(id, Pane::Terminal(Box::new(terminal)));
        });
        let deadline = Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(10));
        loop {
            h.run_steps(2);
            let ready = h.with_app_mut(|app| {
                app.windows[app.active_window]
                    .panes
                    .get_mut(&id)
                    .unwrap()
                    .as_terminal_mut()
                    .unwrap()
                    .backend
                    .capture_lines(50)
                    .iter()
                    .any(|line| line.contains("KITTY_READY"))
            });
            if ready {
                break;
            }
            assert!(Instant::now() < deadline, "keyboard fixture timed out");
            std::thread::sleep(Duration::from_millis(20));
        }
        for (pressed, repeat) in [(true, false), (true, true), (false, false)] {
            h.harness().input_mut().events.push(egui::Event::Key {
                key: egui::Key::J,
                physical_key: Some(egui::Key::J),
                pressed,
                repeat,
                modifiers: egui::Modifiers::NONE,
            });
            if pressed {
                h.harness()
                    .input_mut()
                    .events
                    .push(egui::Event::Text("j".into()));
            }
            h.step();
        }
        let bytes = h.with_app_mut(|app| {
            app.windows[app.active_window]
                .panes
                .get_mut(&id)
                .unwrap()
                .as_terminal_mut()
                .unwrap()
                .backend
                .take_input_tap()
        });
        assert_eq!(
            String::from_utf8_lossy(&bytes),
            "\x1b[106;1:1u\x1b[106;1:2u\x1b[106;1:3u"
        );
        let deadline = Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(10));
        loop {
            h.run_steps(2);
            let received = h.with_app_mut(|app| {
                app.windows[app.active_window]
                    .panes
                    .get_mut(&id)
                    .unwrap()
                    .as_terminal_mut()
                    .unwrap()
                    .backend
                    .capture_lines(50)
                    .iter()
                    .any(|line| line.contains("KEY_EVENTS_OK"))
            });
            if received {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "child did not receive the key event sequence"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        h.save_screenshot(&evidence_png!())
            .expect("render keyboard fixture");
    }

    /// Optional consumer smoke test through crossterm, the real PTY and egui
    /// input ownership. PLEXI_NOOISE_BIN names the installed nooise executable.
    #[test]
    #[ignore = "requires installed nooise and audio device"]
    fn terminal_kitty_nooise_lead_supports_holds() {
        let binary = std::env::var("PLEXI_NOOISE_BIN").expect("set PLEXI_NOOISE_BIN");
        let mut h = PlexiUiHarness::new_sized(1500.0, 1000.0);
        h.step();
        let id = add_focused_pane(&mut h);
        let workspace = h.workspace_root().to_path_buf();
        h.with_app_mut(|app| {
            let terminal = crate::host::pane::TerminalPane::new(
                id,
                app.ctx.clone(),
                app.pty_event_tx.clone(),
                egui_term::BackendSettings {
                    shell: binary,
                    working_directory: Some(workspace),
                    ..Default::default()
                },
                16.0,
            )
            .expect("start nooise");
            app.windows[app.active_window]
                .panes
                .insert(id, Pane::Terminal(Box::new(terminal)));
            app.sidebar_visible = false;
        });
        let capture = |h: &mut PlexiUiHarness| {
            h.with_app_mut(|app| {
                app.windows[app.active_window]
                    .panes
                    .get_mut(&id)
                    .unwrap()
                    .as_terminal_mut()
                    .unwrap()
                    .backend
                    .capture_lines(100)
                    .join("\n")
            })
        };
        let wait_for = |h: &mut PlexiUiHarness, needle: &str| {
            let deadline =
                Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(15));
            loop {
                h.run_steps(2);
                let text = capture(h);
                if text.contains(needle) {
                    break;
                }
                assert!(Instant::now() < deadline, "missing {needle}: {text}");
                std::thread::sleep(Duration::from_millis(20));
            }
        };
        wait_for(&mut h, "BROWSE");
        let modes = h.with_app_mut(|app| {
            app.windows[app.active_window]
                .panes
                .get_mut(&id)
                .unwrap()
                .as_terminal_mut()
                .unwrap()
                .backend
                .sync()
                .terminal_mode
        });
        assert!(modes.contains(
            egui_term::TerminalMode::REPORT_EVENT_TYPES
                | egui_term::TerminalMode::REPORT_ALL_KEYS_AS_ESC
        ));
        let send = |h: &mut PlexiUiHarness, key, text: Option<&str>, pressed, repeat| {
            h.harness().input_mut().events.push(egui::Event::Key {
                key,
                physical_key: Some(key),
                pressed,
                repeat,
                modifiers: egui::Modifiers::NONE,
            });
            if let Some(text) = text {
                h.harness()
                    .input_mut()
                    .events
                    .push(egui::Event::Text(text.into()));
            }
            h.step();
        };
        send(&mut h, egui::Key::I, Some("i"), true, false);
        send(&mut h, egui::Key::I, None, false, false);
        wait_for(&mut h, "LEAD");
        // Adjust Level in Browse so this smoke test also works with nooise
        // versions predating arrow-key adjustments inside Lead mode.
        send(&mut h, egui::Key::Escape, None, true, false);
        send(&mut h, egui::Key::Escape, None, false, false);
        wait_for(&mut h, "BROWSE");
        for _ in 0..10 {
            send(&mut h, egui::Key::ArrowRight, None, true, false);
            send(&mut h, egui::Key::ArrowRight, None, false, false);
        }
        send(&mut h, egui::Key::I, Some("i"), true, false);
        send(&mut h, egui::Key::I, None, false, false);
        wait_for(&mut h, "LEAD");
        send(&mut h, egui::Key::J, Some("j"), true, false);
        for _ in 0..3 {
            std::thread::sleep(Duration::from_millis(40));
            send(&mut h, egui::Key::J, Some("j"), true, true);
        }
        send(&mut h, egui::Key::J, None, false, false);
        wait_for(&mut h, "Space");
        let text = capture(&mut h);
        assert!(
            !text.contains("no key-up"),
            "nooise fell back to taps: {text}"
        );
        assert!(
            !text.contains("level is 0"),
            "lead level must be audible: {text}"
        );
        h.save_screenshot(&evidence_png!())
            .expect("render nooise lead");
    }

    /// Visual review for the terminal reflow debounce (stint 0719). The
    /// debounce holds a resize while the layout is still moving and commits
    /// once the drag settles, so the pixel risk is a *stranded* commit: a
    /// terminal left wrapping to its pre-drag width inside a narrower pane.
    /// Three real terminals are dragged narrower one frame at a time, then
    /// held — the assertion proves the grid followed, the PNG proves the
    /// glyphs did too.
    #[test]
    fn screenshot_terminals_settle_after_window_drag() {
        const START_W: f32 = 1200.0;
        const HEIGHT: f32 = 700.0;
        let mut h = PlexiUiHarness::new_sized(START_W, HEIGHT);
        h.step();
        add_focused_pane(&mut h);
        h.step();
        for _ in 0..3 {
            h.with_app_mut(|app| app.split_focused(false, None, false, false, None));
            h.run_steps(2);
        }

        let terminals: Vec<crate::spatial::tiling::PaneId> = h.with_app(|app| {
            app.windows[app.active_window]
                .panes
                .iter()
                .filter(|(_, pane)| matches!(pane, Pane::Terminal(_)))
                .map(|(id, _)| *id)
                .collect()
        });
        assert!(
            terminals.len() >= 2,
            "need several real terminals for the drag to be meaningful"
        );
        let widest_before = h.with_app_mut(|app| max_terminal_cols(app, &terminals));
        seed_wrapping_text(&mut h, &terminals);

        // Drag the window edge in: one frame per intermediate width, exactly
        // as winit reports a live resize.
        for step in 1..=6 {
            h.harness()
                .set_size(egui::vec2(START_W - step as f32 * 60.0, HEIGHT));
            h.step();
        }
        h.run_steps(4);

        let widest_after = h.with_app_mut(|app| max_terminal_cols(app, &terminals));
        assert!(
            widest_after < widest_before,
            "the trailing-edge commit was stranded: terminals still {widest_before} cols \
             wide after dragging the window narrower (now {widest_after})"
        );

        h.save_screenshot(&evidence_png!()).expect("render failed");
        println!("Screenshot saved to {}", evidence_png!());
    }

    /// Fill each terminal with a line long enough to wrap, so the screenshot
    /// shows real reflowed content instead of an empty grid. Bounded: an
    /// unresponsive shell degrades the visual evidence, never the assertions,
    /// so this returns rather than failing the test.
    fn seed_wrapping_text(h: &mut PlexiUiHarness, terminals: &[crate::spatial::tiling::PaneId]) {
        let line = "the quick brown fox jumps over the lazy dog. ".repeat(6);
        h.with_app_mut(|app| {
            for id in terminals {
                if let Some(terminal) = app.windows[app.active_window]
                    .panes
                    .get_mut(id)
                    .and_then(|pane| pane.as_terminal_mut())
                {
                    terminal
                        .backend
                        .process_command(egui_term::BackendCommand::Write(
                            line.as_bytes().to_vec(),
                        ));
                }
            }
        });
        let deadline =
            std::time::Instant::now() + crate::testing::load_aware_timeout(Duration::from_secs(5));
        while std::time::Instant::now() < deadline {
            h.run_steps(2);
            let painted = h.with_app_mut(|app| {
                terminals.iter().all(|id| {
                    app.windows[app.active_window]
                        .panes
                        .get_mut(id)
                        .and_then(|pane| pane.as_terminal_mut())
                        .is_some_and(|terminal| {
                            terminal
                                .backend
                                .capture_lines(4)
                                .iter()
                                .any(|row| row.contains("quick brown fox"))
                        })
                })
            });
            if painted {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Widest committed PTY grid among `terminals`, in columns.
    fn max_terminal_cols(app: &mut PlexiApp, terminals: &[crate::spatial::tiling::PaneId]) -> u16 {
        terminals
            .iter()
            .filter_map(|id| {
                app.windows[app.active_window]
                    .panes
                    .get_mut(id)
                    .and_then(|pane| pane.as_terminal_mut())
                    .map(|terminal| terminal.backend.committed_grid().0)
            })
            .max()
            .expect("terminals must still be open")
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
        h.save_screenshot(&evidence_png!())
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
        h.save_screenshot(&evidence_png!()).expect("render failed");
    }

    struct GalleryRowsFixture {
        width: f32,
    }

    impl crate::app::app_trait::App for GalleryRowsFixture {
        fn type_id(&self) -> &'static str {
            "gallery_rows_fixture"
        }
        fn display_name(&self) -> String {
            "Gallery rows".into()
        }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
        fn ui(
            &mut self,
            ui: &mut egui::Ui,
            ctx: &crate::app::app_trait::AppRenderContext<'_>,
            _: Option<crate::host::pane::PendingPaneClick>,
        ) {
            ui.set_width(self.width);
            crate::overlays::ui_gallery::gallery_rows(
                ui,
                &crate::overlays::ui_gallery::UiGalleryDataMode::WorstCase.fixture(),
                ctx.colors,
            );
        }
    }

    #[test]
    fn ui_gallery_worst_case_rows() {
        for width in [760.0, 320.0] {
            let mut h = PlexiUiHarness::new_sized(900.0, 1000.0);
            let pane_id = add_focused_pane(&mut h);
            h.with_app_mut(|app| {
                if let Some(Pane::App(pane)) =
                    app.windows[app.active_window].panes.get_mut(&pane_id)
                {
                    pane.runtime = AppRuntime::Builtin(Box::new(GalleryRowsFixture { width }));
                }
            });
            h.run_steps(3);
            h.save_screenshot(&evidence_png!(&format!("_{width}")))
                .expect("all worst-case rows");
        }
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

    #[test]
    fn ui_gallery_worst_case_actions() {
        use crate::overlays::ui_gallery::UiGalleryDataMode;
        use crate::ui::{button, style, surface};
        for mode in UiGalleryDataMode::ALL {
            let data = mode.fixture();
            for width in [760.0, 320.0, 280.0] {
                for kind in 0..5 {
                    let ctx = egui::Context::default();
                    crate::ui::theme::setup_fonts(&ctx);
                    let _ = ctx.run_ui(Default::default(), |_| {});
                    let colors = crate::ui::theme::Colors::from_config(
                        &crate::ui::theme::preset_colors("catppuccin-mocha").unwrap(),
                    );
                    let mut bounds = egui::Rect::NOTHING;
                    let output = ctx.run_ui(Default::default(), |ui| {
                        ui.set_width(width);
                        let response = match kind {
                            0 => button::choice_button(ui, data.detail, "⌘Enter", false, &colors),
                            1 => button::chrome_button(
                                ui,
                                data.detail,
                                button::ButtonKind::Primary,
                                &colors,
                                80.0,
                            ),
                            2 => surface::copyable_command(
                                ui,
                                egui::Id::new("copy"),
                                data.command,
                                &colors,
                            ),
                            3 => surface::compact_copyable_command(
                                ui,
                                egui::Id::new("compact"),
                                data.command,
                                &colors,
                            ),
                            _ => surface::trust_decision_panel(
                                ui,
                                data.names.first().copied().unwrap_or(""),
                                data.detail,
                                surface::TrustTone::Warning,
                                &colors,
                            ),
                        };
                        bounds = response.rect;
                    });
                    assert!(
                        bounds.width() <= width + 0.1,
                        "action overflow {mode:?} {width} kind={kind}: {bounds:?}"
                    );
                    if kind < 2 {
                        assert_eq!(
                            bounds.height(),
                            if kind == 0 {
                                style::BUTTON_H_LG
                            } else {
                                style::BUTTON_H_MD
                            }
                        );
                    }
                    let mut texts = vec![];
                    for clipped in &output.shapes {
                        let painted = clipped.shape.visual_bounding_rect();
                        if painted.is_positive() {
                            assert!(
                                bounds.expand(1.0).contains_rect(painted),
                                "paint outside action {mode:?} {width} kind={kind}: {painted:?} in {bounds:?}"
                            );
                        }
                        gallery_text_shapes(&clipped.shape, &mut texts);
                    }
                    for (i, (rect, text)) in texts.iter().enumerate() {
                        for (other, label) in &texts[i + 1..] {
                            assert!(
                                !rect.shrink(0.5).intersects(other.shrink(0.5)),
                                "action text overlap {mode:?} {width} kind={kind}: {text:?} / {label:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn ui_gallery_worst_case() {
        use crate::overlays::ui_gallery::{UiGalleryDataMode, UiGalleryPreview, gallery_rows};
        for mode in UiGalleryDataMode::ALL {
            for width in [760.0, 320.0, 280.0] {
                // Inspect actual painted galleys, not copies of the layout arithmetic.
                let ctx = egui::Context::default();
                crate::ui::theme::setup_fonts(&ctx);
                let _ = ctx.run_ui(Default::default(), |_| {});
                let colors = crate::ui::theme::Colors::from_config(
                    &crate::ui::theme::preset_colors("catppuccin-mocha").unwrap(),
                );
                let mut bounds = egui::Rect::NOTHING;
                let mut height = 0.0;
                let output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1000.0, 2000.0),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        ui.set_width(width);
                        ui.spacing_mut().item_spacing.y = 0.0;
                        let start = ui.cursor().min;
                        gallery_rows(ui, &mode.fixture(), &colors);
                        height = ui.cursor().min.y - start.y;
                        bounds = egui::Rect::from_min_size(start, egui::vec2(width, height));
                    },
                );
                assert!(
                    (height
                        - mode.fixture().names.len() as f32
                            * (crate::ui::style::LIST_ROW_H + crate::ui::style::LIST_ROW_DENSE_H))
                        .abs()
                        < 0.1,
                    "row-height blowout: {mode:?} {width}: {height}"
                );
                let mut texts = vec![];
                for shape in &output.shapes {
                    gallery_text_shapes(&shape.shape, &mut texts);
                }
                assert_eq!(
                    texts
                        .iter()
                        .filter(|(_, text)| text == "Open" || text == "Delete")
                        .count(),
                    mode.fixture().names.len() * 2
                );
                for (i, (rect, label)) in texts.iter().enumerate() {
                    assert!(
                        bounds.expand(1.0).contains_rect(*rect),
                        "out of bounds {mode:?} {width}: {label:?} {rect:?} in {bounds:?}"
                    );
                    for (other, other_label) in &texts[i + 1..] {
                        assert!(
                            !rect.shrink(0.5).intersects(other.shrink(0.5)),
                            "text/chip overlap {mode:?} {width}: {label:?} and {other_label:?}"
                        );
                    }
                }
            }
            for narrow in [false, true] {
                let mut h = PlexiUiHarness::new_sized(1100.0, 1000.0);
                add_focused_pane(&mut h);
                h.with_app_mut(|app| {
                    app.show_ui_gallery = true;
                });
                h.run_steps(3);
                h.harness().get_by_label(mode.label()).click();
                h.run_steps(2);
                if narrow {
                    h.harness().get_by_label("Narrow 320px").click();
                    h.run_steps(2);
                }
                let preview = h.with_app(|app| {
                    app.ctx.data_mut(|d| {
                        d.get_temp::<UiGalleryPreview>(UiGalleryPreview::id())
                            .unwrap()
                    })
                });
                assert_eq!(preview.mode, mode);
                assert_eq!(preview.narrow, narrow);
                for label in [
                    "Demo",
                    "Worst case",
                    "Empty",
                    "1 item",
                    "Full width",
                    "Narrow 320px",
                ] {
                    assert!(
                        h.harness().get_all_by_label(label).count() > 0,
                        "missing {label}"
                    );
                }
                for section in [
                    "Code and toast surfaces",
                    "Trust and confirmations",
                    "Rows",
                    "Sidebar rows",
                    "Text fields",
                    "Buttons and chips",
                    "Empty states",
                ] {
                    h.harness().get_by_label(section).scroll_to_me();
                    h.run_steps(4);
                    let mut texts = vec![];
                    let mut in_gallery = false;
                    for clipped in &h.harness().output().shapes {
                        let mut painted = vec![];
                        gallery_text_shapes(&clipped.shape, &mut painted);
                        in_gallery |= painted.iter().any(|(_, text)| text == "Host UI Gallery");
                        if in_gallery {
                            texts.extend(
                                painted
                                    .into_iter()
                                    .filter(|(r, _)| clipped.clip_rect.contains_rect(*r)),
                            );
                        }
                    }
                    assert!(texts.len() > 5, "gallery paint guard must inspect content");
                    for (i, (rect, label)) in texts.iter().enumerate() {
                        for (other, other_label) in &texts[i + 1..] {
                            assert!(
                                !rect.shrink(1.0).intersects(other.shrink(1.0)),
                                "gallery overlap {mode:?} narrow={narrow}: {label:?} and {other_label:?}"
                            );
                        }
                    }
                    if mode == UiGalleryDataMode::WorstCase {
                        h.save_screenshot(&evidence_png!(&format!("_{narrow}_{section}")))
                            .expect("gallery screenshot");
                    }
                }
            }
        }
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
        assert_eq!(h.harness().get_all_by_label("plexi completions zsh").count(), 2, "both copy surfaces render the demo fixture");
        assert!(
            h.harness()
                .get_all_by_role(egui::accesskit::Role::MultilineTextInput)
                .count()
                > 0,
            "gallery should render the host TextArea primitive"
        );
        h.save_screenshot(&evidence_png!()).expect("render failed");
        assert!(
            h.with_app(|app| app.show_ui_gallery),
            "gallery should remain open for screenshot"
        );
        println!("Screenshot saved to {}", evidence_png!());
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
        h.save_screenshot(&evidence_png!()).expect("render failed");
        assert!(
            h.with_app(|app| app.show_changelog),
            "changelog should remain open for screenshot"
        );
        println!("Screenshot saved to {}", evidence_png!());
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
        h.save_screenshot(&evidence_png!()).expect("render failed");
        println!("Screenshot saved to {}", evidence_png!());
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
        tool.input_summary = Some(
            r#"{path: notes/log.md, body: line one line two, note: he said "hi"}"#.to_string(),
        );
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
        h.save_screenshot(&evidence_png!()).expect("render failed");
        println!(
            "Screenshot saved to {} — inspect: \
             no literal \\n or \\\" anywhere, /context output reads as a normal row.",
            evidence_png!()
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
        h.save_screenshot(&evidence_png!()).expect("render failed");
        println!(
            "Screenshot saved to {} — \
             inspect: user bubbles right-anchored + shrink-to-fit, assistant \
             bubbles left-anchored, no row renders full-width/identical.",
            evidence_png!()
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
        h.save_screenshot(&evidence_png!()).expect("render failed");

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
        h2.save_screenshot(&evidence_png!()).expect("render failed");
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
        h.save_screenshot(&evidence_png!()).expect("render failed");
        println!(
            "Screenshot saved to {} — inspect: \
             assistant segment above the tool rows, one caret row per call \
             (no 'N tool calls' summary), failed row in danger hue, final \
             reply as its own bubble below the tools.",
            evidence_png!()
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
        h.save_screenshot(&evidence_png!()).expect("render failed");
        println!(
            "Screenshot saved to {} — \
             inspect: 👋 😄 🛒 ✨ 🎮 render as distinct monochrome glyphs, \
             not tofu boxes.",
            evidence_png!()
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
        h.save_screenshot(&evidence_png!()).expect("render failed");
        println!("Screenshot saved to {}", evidence_png!());
    }

    #[test]
    fn assistant_pane_renders_without_crash() {
        let ws = tempfile::tempdir().unwrap();
        let mut h = PlexiUiHarness::new_sized(1280.0, 900.0);
        h.open_assistant(ws.path().to_path_buf());
        h.run_steps(5);
        h.save_screenshot(&evidence_png!()).expect("render failed");
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
        h.save_screenshot(&evidence_png!()).expect("render failed");
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

    #[test]
    fn screenshot_working_agent_quit_confirmation() {
        let mut h = PlexiUiHarness::new_sized(1168.0, 720.0);
        for name in ["release-checks.md", "implementation-plan.md", "test-results.json"] {
            std::fs::write(h.workspace.path().join(name), "Working agent release validation").unwrap();
        }
        let pane = add_focused_pane(&mut h);
        h.with_app_mut(|app| {
            app.windows[0].panes.get_mut(&pane).unwrap().set_agent(Some(crate::protocol::PaneAgentState {
                pane_id: pane, agent: "codex".into(), state: crate::protocol::AgentState::Working,
                detail: Some("Validating the release build".into()), session_id: None,
            }));
            assert!(!app.prepare_user_quit());
        });
        h.step();
        h.save_screenshot(&evidence_png!()).expect("render quit confirmation");
        assert!(h.with_app(|app| app.pending_quit && !app.quitting));
        println!("Screenshot saved to {}", evidence_png!());
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
        h.save_screenshot(&evidence_png!()).expect("render failed");
        assert!(
            h.with_app(|app| app.pending_context_close.is_some()),
            "confirm must still be pending — nothing consumed it"
        );
        println!("Screenshot saved to {}", evidence_png!());
    }

    /// A top-level context has no parent portal, so the modal must render
    /// exactly two action rows — Dissolve would early-return (stint 0542).
    #[test]
    fn screenshot_action_modal_context_close_top_level_has_no_dissolve() {
        let mut h = PlexiUiHarness::new_sized(1168.0, 720.0);
        add_focused_pane(&mut h);
        h.step();
        seed_context_close_confirm(&mut h, false);
        h.save_screenshot(&evidence_png!()).expect("render failed");
        assert!(
            h.with_app(|app| app
                .pending_context_close
                .as_ref()
                .is_some_and(|state| !state.can_dissolve)),
            "a top-level context must not advertise Dissolve"
        );
        println!("Screenshot saved to {}", evidence_png!());
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
                        search_text: title.to_lowercase(),
                        tier_label: None,
                    })
                    .collect();
                app.show_command_palette = true;
                app.sync_command_palette_focus();
            });
            h.run_steps(3);
            h.save_screenshot(&evidence_png!(format!("_palette_{tag}")))
                .expect("render failed");

            let mut h = PlexiUiHarness::new_sized_ppp(1168.0, 720.0, ppp);
            add_focused_pane(&mut h);
            h.step();
            h.with_app_mut(|app| {
                app.open_notes_picker();
                app.notes_picker_entries = [
                    "meeting notes 2026-07-24",
                    "deploy checklist",
                    "sprint 12 retro",
                ]
                .iter()
                .map(|title| crate::notes::NotePickerEntry {
                    path: std::env::temp_dir().join(format!("plexi-density-{title}.md")),
                    title: (*title).to_string(),
                    preview: format!("{title} — first body line of the note"),
                    search_text: title.to_lowercase(),
                    tier_label: None,
                })
                .collect();
                app.notes_picker_selected = 0;
            });
            h.run_steps(3);
            h.save_screenshot(&evidence_png!(format!("_picker_{tag}")))
                .expect("render failed");

            let mut h = PlexiUiHarness::new_sized_ppp(1168.0, 720.0, ppp);
            add_focused_pane(&mut h);
            h.step();
            seed_context_close_confirm(&mut h, true);
            h.save_screenshot(&evidence_png!(format!("_confirm_{tag}")))
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
            app.router.get_mut(ctx_idx).name = "Command Palette Metadata".to_string().into();
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
                path: crate::testing::scratch_context_root("ui"),
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
        h.save_screenshot(&evidence_png!()).expect("render failed");
        assert!(
            h.with_app(|app| app.show_command_palette
                && app.windows[app.active_window]
                    .panes
                    .contains_key(&first_pane_id)),
            "command palette should remain open over the populated context"
        );
        println!("Screenshot saved to {}", evidence_png!());
    }


    #[test]
    fn screenshot_state_presets_and_attention_styles() {
        use crate::protocol::AgentState;
        use crate::config::{AttentionConfig, AttentionStyle};
        let mut h = PlexiUiHarness::new_sized_ppp(1200.0, 800.0, 1.0);
        let root = h.workspace_root().to_path_buf();
        let mut ids = Vec::new();
        for (name, state) in [("Working: compiling",AgentState::Working), ("Waiting: ready",AgentState::Idle), ("Blocked: dependency",AgentState::Blocked), ("Needs you: review",AgentState::NeedsYou), ("Interrupted",AgentState::Interrupted), ("Error: failed test",AgentState::Failed)] {
            let id = add_focused_pane(&mut h);
            ids.push(id);
            h.with_app_mut(|app| {
                let mut term = crate::host::pane::TerminalPane::new(id, app.ctx.clone(), app.pty_event_tx.clone(),
                    egui_term::BackendSettings { shell: "/bin/sh".into(), args: vec!["-c".into(), "printf 'Agent workspace\\nReviewing changes and running tests.\\n'; sleep 300".into()], working_directory: Some(root.clone()), ..Default::default() }, 14.0).unwrap();
                term.name = Some(name.into());
                term.agent_role = Some(name.into());
                term.agent = Some(crate::protocol::PaneAgentState { pane_id:id, state, agent:"codex".into(), detail:None, session_id:None });
                app.windows[app.active_window].panes.insert(id, Pane::Terminal(Box::new(term)));
                app.sidebar_visible = true;
            });
        }
        h.with_app_mut(|app| {
            let win = &mut app.windows[app.active_window];
            let tiles = ids.iter().map(|id| win.tree.tiles.find_pane(id).unwrap()).collect();
            win.tree.root = Some(win.tree.tiles.insert_grid_tile(tiles));
        });
        for theme in ["plexi-night", "plexi-day", "tokyo-day", "solarized-light"] {
            for preset in crate::ui::theme::STATE_PRESETS {
                if matches!(theme, "tokyo-day" | "solarized-light") && *preset != "default" {
                    continue;
                }
                h.with_app_mut(|app| {
                    let mut cfg = crate::ui::theme::preset_colors(theme).unwrap();
                    cfg.state_preset = Some((*preset).into());
                    app.colors = crate::ui::theme::Colors::from_config(&cfg);
                    app.theme = crate::ui::theme::terminal_theme(&cfg);
                    app.config.attention = Some(AttentionConfig { style: Some(AttentionStyle::Subtle), ..Default::default() });
                    crate::ui::theme::setup_style(&app.ctx, &app.colors, theme == "plexi-night");
                });
                h.run_steps(3);
                h.save_screenshot(&evidence_png!(&format!("_{theme}_{preset}"))).unwrap();
                let mut simulated = h.render().unwrap();
                for pixel in simulated.pixels_mut() {
                    let c = crate::ui::theme::simulate_cvd(egui::Color32::from_rgb(pixel[0],pixel[1],pixel[2]),1);
                    pixel[0]=c.r(); pixel[1]=c.g(); pixel[2]=c.b();
                }
                simulated.save(evidence_png!(&format!("_{theme}_{preset}_deuteranopia"))).unwrap();
            }
        }
        for (name,style,intensity) in [("off",AttentionStyle::Off,1.0),("subtle",AttentionStyle::Subtle,1.0),("normal",AttentionStyle::Normal,1.0),("loud",AttentionStyle::Loud,1.0),("normal_half",AttentionStyle::Normal,0.5)] {
            h.with_app_mut(|app| {
                let cfg = crate::ui::theme::preset_colors("plexi-night").unwrap();
                app.colors = crate::ui::theme::Colors::from_config(&cfg);
                app.theme = crate::ui::theme::terminal_theme(&cfg);
                app.config.attention = Some(AttentionConfig { style:Some(style),intensity:Some(intensity),..Default::default() });
                crate::ui::theme::setup_style(&app.ctx,&app.colors,true);
            });
            h.run_steps(3);
            h.save_screenshot(&evidence_png!(&format!("_{name}"))).unwrap();
        }
    }

    #[test]
    fn screenshot_agents_pane_states_and_keys() {
        use crate::protocol::AgentState;
        let mut h = PlexiUiHarness::new_sized(1200.0, 850.0);
        let root = h.workspace_root().to_path_buf();
        let mut ids = Vec::new();
        for (role, state, detail) in [
            ("Planner", AgentState::Working, None),
            ("Implementer", AgentState::Blocked, Some("Which branch should receive the fix?")),
            ("Reviewer", AgentState::NeedsYou, Some("Approve the deployment plan")),
            ("Tester", AgentState::Completed, None),
            ("Researcher", AgentState::Failed, Some("harness exited with code 1 without agent done")),
            ("Writer", AgentState::Idle, None),
            ("Runner", AgentState::Working, None),
        ] {
            let id = add_focused_pane(&mut h);
            ids.push(id);
            h.with_app_mut(|app| {
                let mut term = crate::host::pane::TerminalPane::new(id, app.ctx.clone(), app.pty_event_tx.clone(),
                    egui_term::BackendSettings { shell: "/bin/sh".into(), args: vec!["-c".into(), "sleep 300".into()], working_directory: Some(root.clone()), ..Default::default() }, 14.0).unwrap();
                term.agent_role = Some(role.into());
                term.agent_harness = Some("pi".into());
                term.agent_completed = state == AgentState::Completed;
                term.tail_preview = Some("Inspected the affected files. Ready for the next step.".into());
                term.parent_pane_id = (role == "Implementer").then_some(ids[0]);
                term.agent = Some(crate::protocol::PaneAgentState { pane_id:id, state, agent:"pi".into(), detail:detail.map(str::to_string), session_id:None });
                app.windows[app.active_window].panes.insert(id, Pane::Terminal(Box::new(term)));
                app.sidebar_visible = false;
            });
        }
        h.with_app_mut(|app| {
            let win = &mut app.windows[app.active_window];
            let tiles = ids.iter().map(|id| win.tree.tiles.find_pane(id).unwrap()).collect();
            win.tree.root = Some(win.tree.tiles.insert_tab_tile(tiles));
        });
        h.with_app_mut(|app| {
            app.show_command_palette = true;
            app.palette_query = "agents".into();
        });
        h.run_steps(2);
        h.with_app(|app| assert_eq!(app.palette_agent_count_logged, Some(0)));
        h.save_screenshot(&evidence_png!("_before")).unwrap();
        h.with_app_mut(|app| app.show_command_palette = false);
        h.step();
        let agents_id = h.open_builtin_by_id("agents", &root, &[]).unwrap();
        h.run_steps(3);
        h.with_app_mut(|app| {
            let win = &mut app.windows[app.active_window];
            win.zoomed_pane = win.tree.tiles.find_pane(&agents_id);
        });
        h.run_steps(3);
        h.harness().get_by_label("Agents");
        h.save_screenshot(&evidence_png!("_after")).unwrap();
        h.harness().set_size(egui::vec2(520.0, 760.0));
        h.run_steps(2);
        h.save_screenshot(&evidence_png!("_narrow")).unwrap();
        h.harness().set_size(egui::vec2(1200.0, 850.0));
        h.run_steps(2);
        // The selection starts at human attention, then the parent-routed question.
        h.harness().key_press(egui::Key::J);
        h.step();
        h.harness().key_press(egui::Key::A);
        h.run_steps(2);
        h.harness().get_by_label("Reply to worker");
        h.harness().input_mut().events.push(egui::Event::Text("Use feature/0846".into()));
        h.step();
        h.harness().get_by_label("Send reply").click();
        h.run_steps(2);
        h.with_app(|app| {
            let view = app.agents_view(crate::agent::fleet::FleetFilter::All);
            let row = view.tasks.iter().flat_map(|t| &t.rows).find(|r| r.pane_id == ids[1]).unwrap();
            assert_eq!(row.display_state, crate::agent::fleet::DisplayState::Working);
            assert!(app.pane_mailboxes[&ids[1]].front().unwrap().payload.contains("feature/0846"));
        });
        h.harness().key_press(egui::Key::Enter);
        h.run_steps(2);
        h.with_app(|app| assert_eq!(app.fleet_focused_pane(), Some(ids[1])));
    }

    /// Visual review surface for stint 0565: the palette rendering an agent
    /// fleet spread across two contexts, including a stint-0568-style squad
    /// whose three panes share one title. Empty query shows the whole fleet;
    /// the typed query proves the squad stays separable by agent name.
    #[test]
    fn screenshot_command_palette_agent_fleet() {
        let mut h = PlexiUiHarness::new_sized(1168.0, 800.0);
        h.with_app_mut(|app| app.config.agents.get_or_insert_with(Default::default).palette_agent_results = Some(true));
        add_focused_pane(&mut h);
        let browser_root = h.workspace.path().to_path_buf();

        h.with_app_mut(|app| {
            let ctx_idx = app.router.active_idx();
            app.router.get_mut(ctx_idx).name = "plexi".to_string().into();

            let agent_pane = |app: &mut crate::app::PlexiApp,
                              pane_name: &str,
                              agent: &str,
                              state: crate::protocol::AgentState,
                              detail: Option<&str>| {
                let pane_id = app.host.alloc_pane_id();
                let pane = Pane::App(Box::new(AppPane {
                    pip_status: None,
                    id: pane_id,
                    runtime: AppRuntime::Builtin(Box::new(
                        crate::file_browser::FileBrowserApp::new(browser_root.clone()),
                    )),
                    workspace_root: browser_root.clone(),
                    permissions: AppPermissions::builtin(),
                    manifest_id: "terminal".to_string(),
                    name: pane_name.to_string(),
                    pane_group: None,
                    linked_pane_id: None,
                    overlay_replaced: None,
                    hidden: false,
                    agent: Some(crate::protocol::PaneAgentState {
                        pane_id,
                        state,
                        agent: agent.to_string(),
                        detail: detail.map(str::to_string),
                        session_id: None,
                    }),
                    slots: std::collections::HashMap::new(),
                    semantic_state: Default::default(),
                }));
                (pane_id, pane)
            };

            // One agent alongside the ordinary pane in the active context.
            let (solo_id, solo) = agent_pane(
                app,
                "codex",
                "reviewer",
                crate::protocol::AgentState::Working,
                Some("Read"),
            );
            let win = &mut app.windows[app.active_window];
            win.panes.insert(solo_id, solo);
            let solo_tile = win.tree.tiles.insert_pane(solo_id);
            if let Some(first_tile) = win.tree.root {
                let root = win
                    .tree
                    .tiles
                    .insert_horizontal_tile(vec![first_tile, solo_tile]);
                win.tree.root = Some(root);
                win.focused_pane = Some(first_tile);
            }

            // A three-pane squad in its own subcontext, all panes titled
            // "claude" — the stint 0568 shape this palette has to navigate.
            let squad_ctx_id = app.next_window_id;
            app.next_window_id += 1;
            app.router.push(crate::host::context::Context {
                name: "squad-alpha".to_string().into(),
                root: browser_root.clone(),
                description: None,
                context_id: squad_ctx_id,
                parent_id: None,
                depth: 0,
                parked: true,
            });

            let mut squad: Vec<(u64, Pane)> = [
                (
                    "claude-code",
                    crate::protocol::AgentState::Working,
                    Some("Edit"),
                ),
                ("claude-code", crate::protocol::AgentState::Idle, None),
                (
                    "claude-code",
                    crate::protocol::AgentState::Blocked,
                    Some("Bash(just pr-install)"),
                ),
            ]
            .into_iter()
            .map(|(agent, state, detail)| agent_pane(app, "claude", agent, state, detail))
            .collect();
            let ordinary_id = app.host.alloc_pane_id();
            squad.push((
                ordinary_id,
                Pane::App(Box::new(AppPane {
                    pip_status: None,
                    id: ordinary_id,
                    runtime: AppRuntime::Builtin(Box::new(
                        crate::file_browser::FileBrowserApp::new(browser_root.clone()),
                    )),
                    workspace_root: browser_root.clone(),
                    permissions: AppPermissions::builtin(),
                    manifest_id: "file-browser".to_string(),
                    name: "parked build log".to_string(),
                    pane_group: None,
                    linked_pane_id: None,
                    overlay_replaced: None,
                    hidden: false,
                    agent: None,
                    slots: std::collections::HashMap::new(),
                    semantic_state: Default::default(),
                })),
            ));

            let mut squad_panes = std::collections::HashMap::new();
            let mut squad_tiles = egui_tiles::Tiles::default();
            let mut squad_tile_ids = Vec::new();
            for (pane_id, pane) in squad {
                squad_panes.insert(pane_id, pane);
                squad_tile_ids.push(squad_tiles.insert_pane(pane_id));
            }
            let squad_focused = squad_tile_ids[0];
            let squad_root = squad_tiles.insert_horizontal_tile(squad_tile_ids);
            let squad_window_id = app.next_window_id;
            app.next_window_id += 1;
            app.windows.push(Window {
                name: "squad-alpha".to_string(),
                path: browser_root.clone(),
                tree: egui_tiles::Tree::new("squad-alpha", squad_root, squad_tiles),
                panes: squad_panes,
                focused_pane: Some(squad_focused),
                zoomed_pane: None,
                grid_x: 1,
                grid_y: 0,
                window_id: squad_window_id,
                context_id: squad_ctx_id,
            });
            app.context_active_window
                .insert(squad_ctx_id, squad_window_id);

            // A stale parked context is still a discoverable palette record,
            // but intentionally has no row action because there is no live
            // window to restore without desynchronizing router focus.
            app.router.push(crate::host::context::Context {
                name: "archived-without-window".to_string().into(),
                root: browser_root.clone(),
                description: None,
                context_id: app.next_window_id,
                parent_id: None,
                depth: 0,
                parked: true,
            });
            app.next_window_id += 1;

            app.show_command_palette = true;
            app.palette_selected = 0;
            app.sync_command_palette_focus();
        });

        h.run_steps(3);
        h.save_screenshot(&evidence_png!()).expect("render failed");

        h.with_app_mut(|app| {
            app.palette_query = "squad-alpha".to_string();
            app.palette_selected = 0;
            app.palette_selected_key = None;
        });
        h.run_steps(3);
        h.save_screenshot(&evidence_png!("_context"))
            .expect("render failed");

        h.with_app_mut(|app| {
            app.palette_query = "build log".to_string();
            app.palette_selected = 0;
            app.palette_selected_key = None;
        });
        h.run_steps(3);
        h.save_screenshot(&evidence_png!("_pane"))
            .expect("render failed");

        h.with_app_mut(|app| {
            app.palette_query = " blocked ".to_string();
            app.palette_selected = 0;
            app.palette_selected_key = None;
        });
        h.run_steps(3);
        h.save_screenshot(&evidence_png!("_agent"))
            .expect("render failed");

        // PAL-05 visual evidence uses the same seeded, live-agent corpus at
        // both responsive extremes. The narrow pass catches clipped headings,
        // chips, and search rows that a wide modal can hide.
        h.harness().set_size(egui::vec2(520.0, 400.0));
        h.run_steps(3);
        h.save_screenshot(&evidence_png!("_agent_narrow"))
            .expect("render failed");

        assert!(
            h.with_app(|app| app.show_command_palette && app.palette_agent_count_logged == Some(4)),
            "the palette should be open over all four agent panes"
        );
        println!("Screenshots saved to /tmp/plexi_command_palette_agent_fleet*.png (wide + narrow)");
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
        img.save(evidence_png!()).expect("save screenshot");
        // Probe the blank right gutter inside the modal. The image center can
        // land on a command description as the registered app list changes.
        let surface = img.get_pixel(img.width() * 3 / 4, img.height() / 2).0;
        let center_luma =
            (u16::from(surface[0]) + u16::from(surface[1]) + u16::from(surface[2])) / 3;
        assert!(
            center_luma > 180,
            "Catppuccin Latte palette surface should be light, got rgba={surface:?}"
        );
        println!("Screenshot saved to {}", evidence_png!());
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
                    steps: Vec::new(),
                    on_load: Vec::new(),
                    description: Some("Build the project".to_string()),
                    scope: crate::cli::UserCommandScope::Workspace,
                    secrets: Vec::new(),
                    split: false,
                },
                crate::cli::ResolvedUserCommand {
                    name: "test".to_string(),
                    run: Some("cargo test".to_string()),
                    steps: Vec::new(),
                    on_load: Vec::new(),
                    description: None,
                    scope: crate::cli::UserCommandScope::Workspace,
                    secrets: Vec::new(),
                    split: false,
                },
                crate::cli::ResolvedUserCommand {
                    name: "deploy".to_string(),
                    run: None,
                    steps: Vec::new(),
                    on_load: Vec::new(),
                    description: None,
                    scope: crate::cli::UserCommandScope::Global,
                    secrets: Vec::new(),
                    split: false,
                },
            ];
            app.show_command_palette = true;
            app.palette_selected = 0;
            app.sync_command_palette_focus();
        });
        h.run_steps(3);
        h.save_screenshot(&evidence_png!()).expect("render failed");
        assert!(
            h.with_app(|app| app.show_command_palette && app.palette_commands.len() == 3),
            "palette should be open with the injected user commands"
        );
        println!("Screenshot saved to {}", evidence_png!());
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
                    search_text: format!("palette note {idx:02} deterministic row"),
                    tier_label: None,
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

    /// Holding Control exposes every pane's centered ID, including a window
    /// with just one pane. The two-pane layout is the control case for the
    /// same production render path.
    #[test]
    fn ctrl_pane_id_overlay_renders_for_single_and_split_layouts() {
        fn pane_rect(h: &PlexiUiHarness, pane_id: PaneId) -> egui::Rect {
            h.with_app(|app| {
                let window = &app.windows[app.active_window];
                let tile_id = window
                    .tree
                    .tiles
                    .iter()
                    .find_map(|(tile_id, tile)| {
                        matches!(tile, egui_tiles::Tile::Pane(id) if *id == pane_id)
                            .then_some(tile_id)
                    })
                    .expect("pane tile");
                window.tree.tiles.rect(*tile_id).expect("painted pane rect")
            })
        }

        fn center_glyph_changed(
            without_ctrl: &image::RgbaImage,
            with_ctrl: &image::RgbaImage,
            pane_rect: egui::Rect,
        ) -> bool {
            let center = pane_rect.center();
            let x_start = (center.x - 20.0).max(0.0) as u32;
            let x_end = (center.x + 20.0).min(with_ctrl.width() as f32) as u32;
            let y_start = (center.y - 20.0).max(0.0) as u32;
            let y_end = (center.y + 20.0).min(with_ctrl.height() as f32) as u32;

            (y_start..y_end).any(|y| {
                (x_start..x_end).any(|x| without_ctrl.get_pixel(x, y) != with_ctrl.get_pixel(x, y))
            })
        }

        let screenshot_dir =
            std::env::temp_dir().join(format!("plexi-0630-pane-id-overlay-{}", std::process::id()));
        std::fs::create_dir_all(&screenshot_dir).expect("create screenshot directory");
        println!("pane ID overlay screenshots: {}", screenshot_dir.display());

        let mut single = PlexiUiHarness::new_sized(900.0, 600.0);
        single.step();
        let single_pane = add_focused_pane(&mut single);
        single.step();
        let single_off = single.render().expect("single Ctrl-off render");
        single.harness().input_mut().modifiers = egui::Modifiers::CTRL;
        single.step();
        let single_on = single.render().expect("single Ctrl-on render");
        single_off
            .save(screenshot_dir.join("single-ctrl-off.png"))
            .expect("save single Ctrl-off screenshot");
        single_on
            .save(screenshot_dir.join("single-ctrl-on.png"))
            .expect("save single Ctrl-on screenshot");
        assert!(
            center_glyph_changed(&single_off, &single_on, pane_rect(&single, single_pane)),
            "Control must paint the pane ID in a single-pane layout"
        );

        let mut split = PlexiUiHarness::new_sized(900.0, 600.0);
        split.step();
        let first_pane = add_focused_pane(&mut split);
        let second_pane = add_focused_pane(&mut split);
        split.with_app_mut(|app| {
            let window = &mut app.windows[app.active_window];
            let first_tile = window
                .tree
                .tiles
                .iter()
                .find_map(|(tile_id, tile)| {
                    matches!(tile, egui_tiles::Tile::Pane(id) if *id == first_pane)
                        .then_some(*tile_id)
                })
                .expect("first pane tile");
            let second_tile = window
                .tree
                .tiles
                .iter()
                .find_map(|(tile_id, tile)| {
                    matches!(tile, egui_tiles::Tile::Pane(id) if *id == second_pane)
                        .then_some(*tile_id)
                })
                .expect("second pane tile");
            window.tree.root = Some(
                window
                    .tree
                    .tiles
                    .insert_horizontal_tile(vec![first_tile, second_tile]),
            );
            window.focused_pane = Some(first_tile);
        });
        split.step();
        let split_off = split.render().expect("split Ctrl-off render");
        split.harness().input_mut().modifiers = egui::Modifiers::CTRL;
        split.step();
        let split_on = split.render().expect("split Ctrl-on render");
        split_off
            .save(screenshot_dir.join("split-ctrl-off.png"))
            .expect("save split Ctrl-off screenshot");
        split_on
            .save(screenshot_dir.join("split-ctrl-on.png"))
            .expect("save split Ctrl-on screenshot");

        for pane_id in [first_pane, second_pane] {
            assert!(
                center_glyph_changed(&split_off, &split_on, pane_rect(&split, pane_id)),
                "Control must paint pane {pane_id}'s ID in a split layout"
            );
        }
    }

    // ── Rename flow tests (require a real egui frame to process key events) ────

    /// Full rename flow: set up rename state → press Enter → verify commit.
    ///
    /// Visual review for the shared single-line rename modal. The pane and
    /// context rename overlays are one helper, so the pixels are what prove
    /// the extraction kept the top-hung scrim-less shell, its anchor, and the
    /// select-all-on-focus text field.
    #[test]
    fn screenshot_rename_pane_overlay() {
        let mut h = PlexiUiHarness::new();
        h.step();
        let pane_id = add_focused_pane(&mut h);
        h.step();
        h.with_app_mut(|app| {
            app.rename_buffer = "notes-scratch".to_string();
            app.renaming_pane = Some(pane_id);
            app.focus_stack.push(FocusKind::RenamePane);
        });
        h.step();
        h.save_screenshot("/tmp/plexi-0750-rename-pane-overlay.png")
            .expect("render failed");
        println!("Screenshot saved to /tmp/plexi-0750-rename-pane-overlay.png");
    }

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

    /// Empty rename buffer returns the context to an auto-derived name.
    #[test]
    fn context_rename_empty_buffer_reverts_to_auto() {
        let mut h = PlexiUiHarness::new();
        h.step();

        h.with_app_mut(|app| {
            let ctx_idx = app.router.active_idx();
            app.rename_buffer = "   ".to_string(); // whitespace-only trims to empty
            app.renaming_window = Some(ctx_idx);
            app.focus_stack.push(FocusKind::ContextRename);
        });

        h.harness().key_press(egui::Key::Enter);
        h.step();

        let name = h.with_app(|app| app.router.active().name.clone());
        assert!(
            matches!(name, crate::host::context::ContextName::Auto(ref value) if !value.is_empty()),
            "whitespace-only rename must restore a non-empty automatic name"
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
        h.save_screenshot(&evidence_png!())
            .expect("screenshot failed");
        println!("Screenshot saved to {}", evidence_png!());
        // Visual: open /tmp/plexi_tab_truncation.png and confirm the long label
        // ends with '…' on a single line and does not push the tab bar taller.
    }

    /// Stint 0755: when `outside_workspace` is active on a terminal pane,
    /// `render_name_bar_and_tabs` must reserve the `outside workspace` badge's
    /// width plus `SPACE_XS` on both single-tab and multi-tab paths and elide
    /// the tab label in the remaining space so the label galley and badge rect
    /// never intersect in narrow (~400px) tiles.
    #[test]
    fn outside_workspace_badge_does_not_overlap_tab_label_in_narrow_tile() {
        let long_cwd_label =
            "/Users/ianburke/Documents/GitHub/external-workspace/packages/core/src/components";

        // 1. Single-tab path in a 400px-wide terminal tile.
        {
            let mut h = PlexiUiHarness::new_sized(400.0, 260.0);
            h.step();
            let workspace_root = h.workspace_root().to_path_buf();
            let pane_id = h.with_app_mut(|app| {
                app.sidebar_visible = false;
                let id = app.host.alloc_pane_id();
                let mut terminal = crate::host::pane::TerminalPane::new(
                    id,
                    app.ctx.clone(),
                    app.pty_event_tx.clone(),
                    egui_term::BackendSettings {
                        shell: "/bin/sh".into(),
                        working_directory: Some(workspace_root.clone()),
                        ..Default::default()
                    },
                    14.0,
                )
                .expect("spawn terminal pane");
                terminal.name = Some(long_cwd_label.to_string());
                terminal.outside_workspace_cached = true;
                terminal.outside_workspace_checked_at = Some(Instant::now());
                terminal.outside_workspace_root = Some(workspace_root);

                let win = &mut app.windows[app.active_window];
                win.panes.insert(id, Pane::Terminal(Box::new(terminal)));
                let tile = win.tree.tiles.insert_pane(id);
                win.tree.root = Some(tile);
                win.focused_pane = Some(tile);
                id
            });

            h.run_steps(2);
            let single_shot = evidence_png!("_single_tab");
            h.save_screenshot(&single_shot)
                .expect("save single-tab outside_workspace screenshot");

            let layout = h.with_app(|app| {
                crate::render::terminal_pane::last_rendered_tab_bar_layout(&app.ctx, pane_id)
            })
            .expect("single-tab terminal pane must record tab-bar layout");
            let label_rect = layout.label_rect.expect("single-tab label_rect");
            let badge_rect = layout
                .outside_workspace_badge_rect
                .expect("single-tab outside_workspace_badge_rect");
            assert!(
                !label_rect.intersects(badge_rect),
                "single-tab label_rect ({label_rect:?}) must not intersect outside_workspace badge_rect ({badge_rect:?}); screenshot: {single_shot}"
            );
            assert!(
                label_rect.right() <= badge_rect.left() - crate::ui::style::SPACE_XS + 0.5,
                "single-tab label right ({}) must leave at least SPACE_XS before badge left ({}); screenshot: {single_shot}",
                label_rect.right(),
                badge_rect.left()
            );
            assert!(
                layout
                    .elided_label
                    .as_deref()
                    .unwrap_or_default()
                    .ends_with(crate::ui::text::ELLIPSIS),
                "single-tab long cwd label must elide with '…', got {:?}",
                layout.elided_label
            );
        }

        // 2. Multi-tab path in a 400px-wide terminal tile (rightmost tab active
        // directly adjacent to the right-pinned `outside workspace` badge).
        {
            let mut h = PlexiUiHarness::new_sized(400.0, 260.0);
            h.step();
            let workspace_root = h.workspace_root().to_path_buf();
            let pane_id_b = h.with_app_mut(|app| {
                app.sidebar_visible = false;
                let id_a = app.host.alloc_pane_id();
                let id_b = app.host.alloc_pane_id();
                let mut term_a = crate::host::pane::TerminalPane::new(
                    id_a,
                    app.ctx.clone(),
                    app.pty_event_tx.clone(),
                    egui_term::BackendSettings {
                        shell: "/bin/sh".into(),
                        working_directory: Some(workspace_root.clone()),
                        ..Default::default()
                    },
                    14.0,
                )
                .expect("spawn terminal pane A");
                term_a.name = Some("~/Documents/GitHub/PLEXI/crates/daw-engine".to_string());
                term_a.outside_workspace_cached = false;
                term_a.outside_workspace_checked_at = Some(Instant::now());
                term_a.outside_workspace_root = Some(workspace_root.clone());

                let mut term_b = crate::host::pane::TerminalPane::new(
                    id_b,
                    app.ctx.clone(),
                    app.pty_event_tx.clone(),
                    egui_term::BackendSettings {
                        shell: "/bin/sh".into(),
                        working_directory: Some(workspace_root.clone()),
                        ..Default::default()
                    },
                    14.0,
                )
                .expect("spawn terminal pane B");
                term_b.name = Some(long_cwd_label.to_string());
                term_b.outside_workspace_cached = true;
                term_b.outside_workspace_checked_at = Some(Instant::now());
                term_b.outside_workspace_root = Some(workspace_root);

                let win = &mut app.windows[app.active_window];
                win.panes.insert(id_a, Pane::Terminal(Box::new(term_a)));
                win.panes.insert(id_b, Pane::Terminal(Box::new(term_b)));
                let tile_a = win.tree.tiles.insert_pane(id_a);
                let tile_b = win.tree.tiles.insert_pane(id_b);
                let tab_tile = win.tree.tiles.insert_tab_tile(vec![tile_a, tile_b]);
                if let Some(egui_tiles::Tile::Container(egui_tiles::Container::Tabs(tabs))) =
                    win.tree.tiles.get_mut(tab_tile)
                {
                    tabs.set_active(tile_b);
                }
                win.tree.root = Some(tab_tile);
                win.focused_pane = Some(tile_b);
                id_b
            });

            h.run_steps(2);
            let multi_shot = evidence_png!("_multi_tab");
            h.save_screenshot(&multi_shot)
                .expect("save multi-tab outside_workspace screenshot");

            let layout = h.with_app(|app| {
                crate::render::terminal_pane::last_rendered_tab_bar_layout(&app.ctx, pane_id_b)
            })
            .expect("multi-tab active terminal pane must record tab-bar layout");
            let label_rect = layout.label_rect.expect("multi-tab label_rect");
            let badge_rect = layout
                .outside_workspace_badge_rect
                .expect("multi-tab outside_workspace_badge_rect");
            assert!(
                !label_rect.intersects(badge_rect),
                "multi-tab label_rect ({label_rect:?}) must not intersect outside_workspace badge_rect ({badge_rect:?}); screenshot: {multi_shot}"
            );
            assert!(
                label_rect.right() <= badge_rect.left() - crate::ui::style::SPACE_XS + 0.5,
                "multi-tab label right ({}) must leave at least SPACE_XS before badge left ({}); screenshot: {multi_shot}",
                label_rect.right(),
                badge_rect.left()
            );
            assert!(
                layout
                    .elided_label
                    .as_deref()
                    .unwrap_or_default()
                    .ends_with(crate::ui::text::ELLIPSIS),
                "multi-tab long cwd label must elide with '…', got {:?}",
                layout.elided_label
            );
        }
    }

    /// Stint 0713: at the minimum window width (400pt) and across standard
    /// widths (520pt, 800pt, 1280pt), the sidebar must clamp so that at least
    /// `style::SIDEBAR_CONTENT_MIN_W` (260pt) of usable content area remains.
    #[test]
    fn sidebar_preserves_content_area_floor_at_minimum_and_standard_window_widths() {
        for (window_w, expected_sidebar_w) in [
            (400.0, crate::ui::style::SIDEBAR_MIN_W),
            (520.0, crate::ui::style::SIDEBAR_DEFAULT_W),
            (800.0, crate::ui::style::SIDEBAR_DEFAULT_W),
            (1280.0, crate::ui::style::SIDEBAR_DEFAULT_W),
        ] {
            let mut h = PlexiUiHarness::new_sized(window_w, 320.0);
            h.step();
            add_focused_pane(&mut h);
            h.with_app_mut(|app| {
                app.sidebar_visible = true;
            });
            h.run_steps(2);
            if (window_w - 400.0).abs() < f32::EPSILON {
                let shot = evidence_png!("_400pt");
                h.save_screenshot(&shot)
                    .expect("save 400pt sidebar clamp screenshot");
            }
            let sidebar_w = h.with_app(|app| {
                egui::containers::panel::PanelState::load(&app.ctx, egui::Id::new("sidebar"))
                    .expect("sidebar panel state must exist")
                    .rect
                    .width()
            });
            assert!(
                (sidebar_w - expected_sidebar_w).abs() <= 1.0,
                "at window_w={window_w}, expected sidebar width {expected_sidebar_w}, got {sidebar_w}"
            );
            let content_w = window_w - sidebar_w;
            assert!(
                content_w + 0.5 >= crate::ui::style::SIDEBAR_CONTENT_MIN_W,
                "at window_w={window_w}, content width {content_w} must be >= SIDEBAR_CONTENT_MIN_W ({})",
                crate::ui::style::SIDEBAR_CONTENT_MIN_W
            );
        }
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
                workspace_root: crate::testing::scratch_context_root("ui"),
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
        editor_rows_evidence(1.0, &evidence_png!("_ppp1"));
    }

    /// Stint 0529 evidence: same document at ppp 2.0 (the HiDPI case that
    /// used to shimmer during scroll and show 17/17/16/17 leading).
    #[test]
    fn screenshot_editor_rows_pixel_grid_ppp2() {
        editor_rows_evidence(2.0, &evidence_png!("_ppp2"));
    }

    /// Stints 0528/0530 evidence: sidebar with seeded contexts (active +
    /// inactive names, path subtitles) and the spatial minimap overlay with a
    /// workspace name, at 1x and 2x. Review: inactive context names must meet
    /// the contrast floor, minimap name galley and status-dot capsules must
    /// sit on the pixel grid.
    fn render_sidebar_pane_dots_pixel_grid(ppp: f32, out: &str) {
        let mut h = PlexiUiHarness::new_sized_ppp(1100.0, 700.0, ppp);
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
                path: crate::testing::scratch_context_root("ui"),
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
                name: "PLEXI".to_string().into(),
                root: plexi_root.clone(),
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
                    name: name.to_string().into(),
                    root: root,
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
        // The active context's first two windows each need a pane so its
        // sidebar capsule draws the multi-window dot grouping this test
        // exists to review.
        add_focused_pane_to_window(&mut h, 1);
        add_focused_pane_to_window(&mut h, 2);
        h.with_app_mut(|app| {
            assert_eq!(app.windows[2].context_id, inactive_ctx_id);
        });
        h.run_steps(3);
        h.save_screenshot(out).expect("render failed");
        println!("Screenshot saved to {out}");
    }

    #[test]
    fn screenshot_sidebar_pane_dots_pixel_grid_ppp1() {
        render_sidebar_pane_dots_pixel_grid(1.0, &evidence_png!("_ppp1"));
    }

    #[test]
    fn screenshot_sidebar_pane_dots_pixel_grid_ppp2() {
        render_sidebar_pane_dots_pixel_grid(2.0, &evidence_png!("_ppp2"));
    }

    /// Stint 0700: the 220-point sidebar shows an ordinary context name in full
    /// while a genuinely long name ellipsizes. The hover makes the sole trailing
    /// close affordance visible in the evidence image.
    #[test]
    fn screenshot_sidebar_x_end_at_default_width() {
        let mut h = PlexiUiHarness::new_sized(1100.0, 700.0);
        h.step();
        let inactive_window_idx = h.with_app_mut(|app| {
            app.sidebar_visible = true;
            let active_context_idx = app.router.active_idx();
            let active_context_id = app.router.get(active_context_idx).context_id;
            let active_window_id = app.windows[app.active_window].window_id;
            app.router.get_mut(active_context_idx).name = "PLEXI Workspace".to_string().into();
            app.router.get_mut(active_context_idx).root =
                std::path::PathBuf::from("/projects/PLEXI");

            let inactive_context_id = app.next_window_id;
            app.next_window_id += 1;
            let inactive_root = std::path::PathBuf::from("/projects/narrator-site");
            app.router.push(crate::host::context::Context {
                // Deliberately longer than a normal workspace title: the
                // screenshot must exercise truncation without making the
                // ordinary active title a false-positive fixture.
                name: "Narrator Video Production Workspace".to_string().into(),
                root: inactive_root.clone(),
                description: None,
                context_id: inactive_context_id,
                parent_id: None,
                depth: 0,
                parked: false,
            });
            let inactive_window_id = app.next_window_id;
            app.next_window_id += 1;
            app.windows.push(crate::host::context::Window {
                name: String::new(),
                path: inactive_root,
                tree: egui_tiles::Tree::empty("sidebar-x-end-preview"),
                panes: std::collections::HashMap::new(),
                focused_pane: None,
                zoomed_pane: None,
                grid_x: 0,
                grid_y: 0,
                window_id: inactive_window_id,
                context_id: inactive_context_id,
            });

            let notification = |notify_id: &str, source_context_id, source_window_id| {
                crate::app::PendingNotification {
                    notify_id: notify_id.to_string(),
                    sender_pane_id: 0,
                    dismiss_owner_pane_id: 0,
                    source_context_id,
                    source_window_id,
                    title: "Preview badge".to_string(),
                    body: "Seeded sidebar notification".to_string(),
                    kind: crate::protocol::NotifyKind::Message,
                    options: vec![],
                    input_prompt: None,
                    required: false,
                    scope: crate::protocol::NotifyScope::Context,
                    image_inline: None,
                    image_pipe_id: None,
                    response_file: None,
                    timeout_secs: None,
                    on_dismiss: None,
                    enqueued_at: std::time::Instant::now(),
                    tombstoned: false,
                    deliver_after: None,
                    origin_in_view: false,
                }
            };
            app.pending_notifications.extend([
                notification("active-one", active_context_id, active_window_id),
                notification("active-two", active_context_id, active_window_id),
                notification("active-three", active_context_id, active_window_id),
                notification("inactive-one", inactive_context_id, inactive_window_id),
                notification("inactive-two", inactive_context_id, inactive_window_id),
            ]);
            app.windows.len() - 1
        });
        add_focused_pane(&mut h);
        add_focused_pane_to_window(&mut h, inactive_window_idx);
        h.run_steps(3);

        // Hover a sidebar row so the trailing close glyph is visible in the PNG.
        // The zoom path bar repeats the active context name; the sidebar row is
        // the leftmost match.
        let active_title = h
            .harness()
            .get_all_by_label("PLEXI Workspace")
            .map(|n| n.rect())
            .min_by(|a, b| a.min.x.total_cmp(&b.min.x))
            .expect("sidebar row")
            .center();
        h.harness()
            .get_by_label("Narrator Video Production Workspace");
        h.harness()
            .input_mut()
            .events
            .push(egui::Event::PointerMoved(active_title));
        h.step();
        h.save_screenshot(&evidence_png!())
            .expect("220-point sidebar render");
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
        h.save_screenshot(&evidence_png!()).expect("render failed");
        println!("Screenshot saved to {}", evidence_png!());
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
                        name: name.to_string().into(),
                        root: root,
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
                    workspace_root: crate::testing::scratch_context_root("ui"),
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
                let out = evidence_png!(format!("_sidebar_ppp{}", (ppp * 10.0) as u32));
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
                let out = evidence_png!(format!("_editor_ppp{}", (ppp * 10.0) as u32));
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
                let out = evidence_png!(format!("_filebrowser_ppp{}", (ppp * 10.0) as u32));
                h.save_screenshot(&out).expect("file browser render");
                println!("Screenshot saved to {out}");
            }
        }

        #[test]
        fn gate_render_wasm_text_ppp_1_and_2() {
        let _experiments = crate::release::set_test_experiments(&[crate::release::ReleaseFeature::WasmApps]);
            for ppp in GATE_PPP {
                let profile = tempfile::tempdir().expect("profile dir");
                let _guard = crate::config::set_test_profile_dir(profile.path().to_path_buf());
                let mut h = PlexiUiHarness::new_sized_ppp(900.0, 620.0, ppp);
                h.step();
                seed_wasm_text(&mut h, profile.path());
                h.run_steps(6);
                let out = evidence_png!(format!("_wasm_text_ppp{}", (ppp * 10.0) as u32));
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

    /// Stint 0720 visual review: the todo app's add form, autofocused and
    /// mid-draft. Drives the real app process through the production paths —
    /// `a` opens the form on the guest's raw KeyEvent path, `autofocus` hands
    /// the cursor to the host TextEdit with no click, and the typed draft is
    /// painted from the host-owned buffer while it round-trips to the guest.
    /// The assertion is the gate; the PNG is for a human/agent to look at.
    #[test]
    fn screenshot_todo_add_form_autofocused_draft() {
        let app_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("apps/todo");
        let mut h = PlexiUiHarness::new_sized(900.0, 620.0);
        let pane_id = h.open_app_at(&app_dir, &[]).expect("launch todo");
        h.wait_for_app_frame(pane_id, Duration::from_secs(30))
            .expect("todo renders its first frame");
        h.focus_pane(pane_id).expect("focus todo pane");
        h.run_steps(2);

        // `a` opens the add form (the guest's raw KeyEvent path — no text
        // surface is focused yet).
        h.harness().input_mut().events.push(egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
        h.step();

        let pane_text = |h: &PlexiUiHarness| -> String {
            h.with_app(|app| {
                app.windows[app.active_window]
                    .panes
                    .get(&pane_id)
                    .and_then(Pane::as_app)
                    .map(|pane| {
                        pane.semantic_state()
                            .nodes
                            .iter()
                            .flat_map(|n| {
                                n.label
                                    .clone()
                                    .into_iter()
                                    .chain(n.value.clone().map(|v| format!("value:{v}")))
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .unwrap_or_default()
            })
        };
        let start = Instant::now();
        while !pane_text(&h).contains("What needs doing?") {
            assert!(
                start.elapsed() < Duration::from_secs(30),
                "todo add form never appeared:\n{}",
                pane_text(&h)
            );
            h.step();
            std::thread::sleep(Duration::from_millis(25));
        }

        // Type without clicking: `autofocus` must already own the cursor. One
        // character per frame, faster than the guest can echo any of them.
        for ch in "Buy oat milk".chars() {
            h.harness()
                .input_mut()
                .events
                .push(egui::Event::Text(ch.to_string()));
            h.step();
        }
        let start = Instant::now();
        while !pane_text(&h).contains("value:Buy oat milk") {
            assert!(
                start.elapsed() < Duration::from_secs(30),
                "autofocused draft never round-tripped to the guest:\n{}",
                pane_text(&h)
            );
            h.step();
            std::thread::sleep(Duration::from_millis(25));
        }

        h.save_screenshot(&evidence_png!())
            .expect("render todo add form");
    }

    /// Stint 0581 visual + geometry review: `--agents 3` with `--layout`
    /// omitted arranges the squad as three equal full-height columns (not a
    /// 2×2 grid with a dead bottom-right quadrant) and labels each pane header
    /// `<name>-1`, `<name>-2`, `<name>-3`.
    #[test]
    fn screenshot_context_sub_three_agents_default_columns_and_names() {
        let mut h = PlexiUiHarness::new_sized(1280.0, 800.0);
        h.step();
        let anchor = add_focused_pane(&mut h);
        let root = tempfile::tempdir().expect("squad root");

        let pane_ids = h.with_app_mut(|app| {
            let parent_id = app.router.active().context_id;
            let layout = crate::pane_ops::default_squad_layout(3);
            let child = app
                .new_child_context(crate::pane_ops::ChildContextSpec {
                    parent_id: Some(parent_id),
                    parent_name: String::new(),
                    name: Some("agentsquad".to_string()),
                    path: root.path().to_path_buf(),
                    portal_vertical: true,
                    portal_first: false,
                    anchor_pane: Some(anchor),
                    panes: vec![None, None, None],
                    layout,
                })
                .expect("create 3-pane squad");
            let child_idx = app.router.len() - 1;
            app.switch_workspace(child_idx);
            child.pane_ids
        });
        h.run_steps(3);

        let rects: Vec<egui::Rect> = h.with_app(|app| {
            let window = &app.windows[app.active_window];
            pane_ids
                .iter()
                .map(|pane_id| {
                    let tile_id = window
                        .tree
                        .tiles
                        .iter()
                        .find_map(|(tid, tile)| {
                            matches!(tile, egui_tiles::Tile::Pane(id) if id == pane_id)
                                .then_some(*tid)
                        })
                        .expect("squad pane tile");
                    window.tree.tiles.rect(tile_id).expect("painted pane rect")
                })
                .collect()
        });
        assert_eq!(rects.len(), 3);
        for (i, rect) in rects.iter().enumerate() {
            assert!(
                (rect.min.y - rects[0].min.y).abs() < 2.0
                    && (rect.max.y - rects[0].max.y).abs() < 2.0,
                "pane {i} must span the same full vertical band as pane 0 (got {rect:?} vs {:?})",
                rects[0]
            );
            // Column widths follow the zoom strip presets (35% leading, 65%
            // following), so only a non-empty width is required here.
            assert!(rect.width() > 1.0, "pane {i} must have a painted width: {rect:?}");
        }
        assert!(
            rects[0].max.x <= rects[1].min.x + 2.0 && rects[1].max.x <= rects[2].min.x + 2.0,
            "the 3 squad panes must sit side-by-side left-to-right: {rects:?}"
        );

        let names: Vec<Option<String>> = h.with_app(|app| {
            let window = &app.windows[app.active_window];
            pane_ids
                .iter()
                .map(|id| {
                    window
                        .panes
                        .get(id)
                        .and_then(Pane::as_terminal)
                        .and_then(|t| t.name.clone())
                })
                .collect()
        });
        assert_eq!(
            names,
            vec![
                Some("agentsquad-1".to_string()),
                Some("agentsquad-2".to_string()),
                Some("agentsquad-3".to_string()),
            ]
        );

        h.save_screenshot(&evidence_png!())
            .expect("render 3-pane squad columns");
    }

    /// Item 4 regression: a context with several windows must lay out the ACTIVE
    /// window's columns from its own indices. Moving left/right across the window
    /// boundary used to feed the context-wide scroll offset into the window's tree and
    /// painted 6% slivers beside one oversized column (and overflowed the viewport).
    #[test]
    fn cross_window_navigation_keeps_every_pane_inside_viewport() {
        use crate::host::keys::Direction;
        const MIN_COLUMN_WIDTH: f32 = 80.0;
        let mut h = PlexiUiHarness::new_sized(1100.0, 700.0);
        let (ctx_id, first_window_panes, second_pane) = h.with_app_mut(|app| {
            app.sidebar_visible = false;
            let ctx_id = app.router.active().context_id;
            let p1 = app.add_test_pane().1;
            let p2 = app.add_test_pane().1;
            let p3 = app.add_test_pane().1;
            app.set_context_columns(ctx_id, vec![vec![p1], vec![p2], vec![p3]]);
            let win_id = app.next_window_id;
            app.next_window_id += 1;
            app.windows.push(crate::host::context::Window {
                name: String::new(),
                path: std::path::PathBuf::from("/tmp"),
                tree: egui_tiles::Tree::empty("plexi-second"),
                panes: std::collections::HashMap::new(),
                focused_pane: None,
                zoomed_pane: None,
                grid_x: 1,
                grid_y: 0,
                window_id: win_id,
                context_id: ctx_id,
            });
            let second_idx = app.windows.len() - 1;
            let (tile, p4) = app.add_app_pane_in_window(second_idx, "test");
            app.windows[second_idx].focused_pane = Some(tile);
            app.active_window = second_idx;
            app.context_active_window.insert(ctx_id, win_id);
            (ctx_id, vec![p1, p2, p3], p4)
        });
        h.run_steps(3);

        let check = |h: &mut PlexiUiHarness, step: &str| {
            h.run_steps(3);
            h.with_app(|app| {
                let viewport = app.ctx.content_rect();
                let window = &app.windows[app.active_window];
                let mut narrow = 0usize;
                let mut columns = 0usize;
                if let Some(root) = window.tree.root() {
                    let children = match window.tree.tiles.get(root) {
                        Some(egui_tiles::Tile::Container(c)) => c.children().copied().collect(),
                        _ => vec![root],
                    };
                    columns = children.len();
                    for child in children {
                        let rect = window.tree.tiles.rect(child).expect("painted column rect");
                        assert!(
                            viewport.expand(1.0).contains_rect(rect),
                            "{step}: column rect {rect:?} escapes the viewport {viewport:?}"
                        );
                        if rect.width() < MIN_COLUMN_WIDTH {
                            narrow += 1;
                        }
                    }
                }
                // Only the clipped neighbours of the strip may be narrow: with 3 columns
                // exactly one is clipped, never two.
                assert!(
                    narrow <= columns.saturating_sub(2),
                    "{step}: {narrow} of {columns} columns are narrower than {MIN_COLUMN_WIDTH}px"
                );
                let focused = app.active_focused_pane_id().expect("focused pane");
                let tile = window.tree.tiles.find_pane(&focused).expect("focused tile");
                let rect = window.tree.tiles.rect(tile).expect("focused rect");
                assert!(
                    rect.width() >= 200.0 && rect.height() >= 200.0,
                    "{step}: focused pane squeezed to {rect:?}"
                );
            });
        };

        check(&mut h, "start on second window");
        for (i, dir) in [
            Direction::Left,
            Direction::Left,
            Direction::Left,
            Direction::Right,
            Direction::Right,
            Direction::Right,
            Direction::Left,
        ]
        .into_iter()
        .enumerate()
        {
            h.with_app_mut(|app| {
                app.navigate(dir);
            });
            check(&mut h, &format!("move {i} {dir:?}"));
        }
        let _ = (ctx_id, first_window_panes, second_pane);
    }

    #[test]
    fn file_browser_responsive_table_and_font_scaling_screenshots() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let parent_dir = tmp.path().join("workspace-parent");
        let leaf_dir = parent_dir.join("active-leaf");
        std::fs::create_dir_all(&leaf_dir).expect("create dirs");
        std::fs::write(leaf_dir.join("README.md"), b"# Project\nOverview").expect("write");
        std::fs::write(
            leaf_dir.join("very_long_component_filename_that_must_elide_cleanly_in_narrow_columns.rs"),
            b"fn main() {}",
        )
        .expect("write");
        std::fs::create_dir_all(leaf_dir.join("subfolder")).expect("mkdir");
        std::fs::write(parent_dir.join("root-file.toml"), b"[package]").expect("write");

        let mut h = PlexiUiHarness::new_sized_ppp(440.0, 520.0, 2.0);
        let leaf_clone = leaf_dir.clone();
        h.with_app_mut(|app| {
            app.sidebar_visible = false;
            app.open_builtin_app_pane(
                Box::new(crate::file_browser::FileBrowserApp::new(leaf_clone.clone())),
                AppPermissions::builtin(),
                leaf_clone,
                None,
                None,
                None,
            );
        });
        h.run_steps(3);
        assert!(
            !h.host_has_label("compact") && !h.host_has_label("details"),
            "stray mode label must not be rendered in toolbar"
        );
        h.save_screenshot(&evidence_png!("narrow"))
            .expect("render narrow file_browser");

        // Resize to wide width and verify unified table renders with breadcrumbs.
        h.harness().set_size(egui::vec2(880.0, 520.0));
        h.run_steps(3);
        h.save_screenshot(&evidence_png!("wide"))
            .expect("render wide file_browser");

        // Scale font size up via adjust_focused_pane_font_size and verify render.
        h.with_app_mut(|app| {
            app.adjust_focused_pane_font_size(4.0);
        });
        h.run_steps(3);
        h.save_screenshot(&evidence_png!("scaled"))
            .expect("render scaled file_browser");

        // Click ancestor breadcrumb "workspace-parent" (converting from kittest's
        // 2x physical-pixel rect to points) and verify navigation to parent_dir.
        let crumb_pos = h.harness().get_by_label("workspace-parent").rect().center() / 2.0;
        h.harness()
            .input_mut()
            .events
            .push(egui::Event::PointerMoved(crumb_pos));
        h.harness()
            .input_mut()
            .events
            .push(egui::Event::PointerButton {
                pos: crumb_pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            });
        h.harness()
            .input_mut()
            .events
            .push(egui::Event::PointerButton {
                pos: crumb_pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            });
        h.run_steps(3);
        h.with_app_mut(|app| {
            let fb = app.windows[app.active_window]
                .panes
                .values_mut()
                .find_map(|p| {
                    let AppRuntime::Builtin(b) = &mut p.as_app_mut()?.runtime else {
                        return None;
                    };
                    b.as_any_mut()
                        .downcast_mut::<crate::file_browser::FileBrowserApp>()
                })
                .expect("FileBrowserApp pane");
            assert_eq!(fb.cwd, parent_dir, "clicking breadcrumb must navigate to ancestor");
        });
    }

    #[test]
    fn screenshot_file_browser_deep_search_narrow_and_wide() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().join("workspace-root");
        let src_dir = root.join("src").join("search");
        let docs_dir = root.join("docs");
        std::fs::create_dir_all(&src_dir).expect("mkdir src/search");
        std::fs::create_dir_all(&docs_dir).expect("mkdir docs");
        std::fs::write(
            src_dir.join("search_session.rs"),
            b"pub struct SearchSession;\n",
        )
        .expect("write search_session.rs");
        std::fs::write(
            src_dir.join("search_bar_widget.rs"),
            b"pub struct SearchBar;\n",
        )
        .expect("write search_bar_widget.rs");
        std::fs::write(
            docs_dir.join("architecture.md"),
            b"# Architecture\nUnified upstream search primitive for path and content discovery.\n",
        )
        .expect("write architecture.md");

        let mut h = PlexiUiHarness::new_sized_ppp(440.0, 480.0, 2.0);
        let root_clone = root.clone();
        h.with_app_mut(|app| {
            app.sidebar_visible = false;
            app.open_builtin_app_pane(
                Box::new(crate::file_browser::FileBrowserApp::new(root_clone.clone())),
                AppPermissions::builtin(),
                root_clone,
                None,
                None,
                None,
            );
        });
        h.run_steps(2);

        // Press Cmd+F to open the SearchBar and focus its TextInput, then type "search".
        h.harness().input_mut().events.push(egui::Event::Key {
            key: egui::Key::F,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers {
                command: true,
                ..Default::default()
            },
        });
        h.run_steps(2);
        h.harness()
            .input_mut()
            .events
            .push(egui::Event::Text("search".to_string()));
        h.run_steps(2);

        let deadline = std::time::Instant::now()
            + crate::testing::load_aware_timeout(std::time::Duration::from_secs(5));
        let mut found_all = false;
        while std::time::Instant::now() < deadline {
            h.step();
            let done = h.with_app_mut(|app| {
                app.windows[app.active_window]
                    .panes
                    .values_mut()
                    .find_map(|p| {
                        let AppRuntime::Builtin(b) = &mut p.as_app_mut()?.runtime else {
                            return None;
                        };
                        b.as_any_mut()
                            .downcast_mut::<crate::file_browser::FileBrowserApp>()
                    })
                    .is_some_and(|fb| fb.search_result_count() >= 3)
            });
            if done {
                found_all = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
        assert!(
            found_all,
            "Cmd+F + typing 'search' must enter search mode and populate deep path + content hits"
        );

        h.run_steps(2);
        h.save_screenshot(&evidence_png!("narrow"))
            .expect("render narrow deep search");

        h.harness().set_size(egui::vec2(880.0, 480.0));
        h.run_steps(2);
        h.save_screenshot(&evidence_png!("wide"))
            .expect("render wide deep search");
    }

    #[test]
    fn screenshot_file_browser_status_bar_shortcuts_wide_and_narrow() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().join("workspace-root");
        std::fs::create_dir_all(root.join("src")).expect("mkdir src");
        std::fs::create_dir_all(root.join("docs")).expect("mkdir docs");
        std::fs::write(root.join("Cargo.toml"), b"[package]\nname = \"plexi\"\n")
            .expect("write Cargo.toml");
        std::fs::write(root.join("README.md"), b"# Plexi\n").expect("write README.md");

        let mut h = PlexiUiHarness::new_sized_ppp(1100.0, 720.0, 2.0);
        let root_clone = root.clone();
        h.with_app_mut(|app| {
            app.sidebar_visible = false;
            app.open_builtin_app_pane(
                Box::new(crate::file_browser::FileBrowserApp::new(root_clone.clone())),
                AppPermissions::builtin(),
                root_clone,
                None,
                None,
                None,
            );
        });
        h.run_steps(3);
        h.with_app_mut(|app| {
            let fb = app.windows[app.active_window]
                .panes
                .values_mut()
                .find_map(|p| {
                    let AppRuntime::Builtin(b) = &mut p.as_app_mut()?.runtime else {
                        return None;
                    };
                    b.as_any_mut()
                        .downcast_mut::<crate::file_browser::FileBrowserApp>()
                })
                .expect("FileBrowserApp pane");
            let layout = fb
                .last_status_bar_layout
                .as_ref()
                .expect("wide status bar layout");
            assert_eq!(
                layout.label_rects.len(),
                7,
                "wide 1100x720 pane must fit all 7 status bar hint groups"
            );
            for label_rect in &layout.label_rects {
                assert!(
                    (label_rect.max.y - layout.item_count_rect.max.y).abs() <= 0.5,
                    "wide hint label baseline must match item count baseline"
                );
            }
            for chip_rect in &layout.chip_rects {
                assert!(
                    (chip_rect.center().y - layout.item_count_rect.center().y).abs() <= 0.5,
                    "wide chip vertical center must match item count vertical center"
                );
            }
            let last_label = layout.label_rects.last().unwrap();
            assert!(
                (layout.row_rect.max.x - last_label.max.x).abs() <= 2.0,
                "wide rightmost hint label must sit flush with status bar right edge"
            );
        });
        h.save_screenshot(&evidence_png!("_wide_1100x720"))
            .expect("render wide 1100x720 status bar");

        h.harness().set_size(egui::vec2(640.0, 520.0));
        h.run_steps(3);
        h.with_app_mut(|app| {
            let fb = app.windows[app.active_window]
                .panes
                .values_mut()
                .find_map(|p| {
                    let AppRuntime::Builtin(b) = &mut p.as_app_mut()?.runtime else {
                        return None;
                    };
                    b.as_any_mut()
                        .downcast_mut::<crate::file_browser::FileBrowserApp>()
                })
                .expect("FileBrowserApp pane");
            let layout = fb
                .last_status_bar_layout
                .as_ref()
                .expect("narrow status bar layout");
            assert!(
                !layout.label_rects.is_empty() && layout.label_rects.len() < 7,
                "narrow 640x520 pane must drop leading low-priority hints to fit cleanly"
            );
            for label_rect in &layout.label_rects {
                assert!(
                    (label_rect.max.y - layout.item_count_rect.max.y).abs() <= 0.5,
                    "narrow hint label baseline must match item count baseline"
                );
            }
            for chip_rect in &layout.chip_rects {
                assert!(
                    (chip_rect.center().y - layout.item_count_rect.center().y).abs() <= 0.5,
                    "narrow chip vertical center must match item count vertical center"
                );
            }
            let first_chip = layout.chip_rects.first().unwrap();
            let last_label = layout.label_rects.last().unwrap();
            assert!(
                first_chip.min.x >= layout.item_count_rect.max.x + crate::ui::style::SPACE_MD,
                "narrow first chip must not collide with item count label"
            );
            assert!(
                (layout.row_rect.max.x - last_label.max.x).abs() <= 2.0,
                "narrow rightmost hint label must sit flush with status bar right edge"
            );
        });
        h.save_screenshot(&evidence_png!("_narrow_640x520"))
            .expect("render narrow 640x520 status bar");
    }

    #[test]
    fn screenshot_cli_renderer_shared_chrome() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let desc_path = tmp.path().join("descriptor.json");
        std::fs::write(
            &desc_path,
            r#"{
                "plexi_version": "0.1",
                "name": "cargo",
                "version": "1.84.0",
                "description": "Rust's package manager and build tool",
                "default_view": "list",
                "commands": [
                    {
                        "name": "build",
                        "description": "Compile a local package and all of its dependencies",
                        "args": [
                            {"name": "package", "type": "string", "required": false, "placeholder": "plexi"}
                        ],
                        "flags": [
                            {"name": "--release", "type": "bool", "description": "Build artifacts in release mode"}
                        ]
                    },
                    {
                        "name": "test",
                        "description": "Execute all unit and integration tests"
                    }
                ]
            }"#,
        )
        .expect("write descriptor.json");

        let mut h = PlexiUiHarness::new_sized_ppp(720.0, 680.0, 2.0);
        let desc_str = desc_path.to_string_lossy().into_owned();
        let cwd = tmp.path().to_path_buf();
        h.with_app_mut(|app| {
            app.sidebar_visible = false;
            app.open_builtin_app_pane(
                Box::new(crate::render::cli_renderer_app::CliRendererApp::new(desc_str)),
                AppPermissions::builtin(),
                cwd,
                None,
                None,
                None,
            );
        });
        h.run_steps(3);
        h.with_app_mut(|app| {
            let cli = app.windows[app.active_window]
                .panes
                .values_mut()
                .find_map(|p| {
                    let AppRuntime::Builtin(b) = &mut p.as_app_mut()?.runtime else {
                        return None;
                    };
                    b.as_any_mut()
                        .downcast_mut::<crate::render::cli_renderer_app::CliRendererApp>()
                })
                .expect("CliRendererApp pane");
            assert!(cli.last_header_rect.is_some());
            assert!(cli.last_footer_chip_rects.len() >= 3);
        });
        h.save_screenshot(&evidence_png!("_list"))
            .expect("render cli_renderer list chrome");

        // Press Enter to open the `build` command form.
        h.harness().input_mut().events.push(egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
        h.run_steps(3);
        h.with_app_mut(|app| {
            let cli = app.windows[app.active_window]
                .panes
                .values_mut()
                .find_map(|p| {
                    let AppRuntime::Builtin(b) = &mut p.as_app_mut()?.runtime else {
                        return None;
                    };
                    b.as_any_mut()
                        .downcast_mut::<crate::render::cli_renderer_app::CliRendererApp>()
                })
                .expect("CliRendererApp pane");
            assert!(cli.last_header_rect.is_some());
            assert_eq!(cli.last_footer_chip_rects.len(), 2);
        });
        h.save_screenshot(&evidence_png!("_form"))
            .expect("render cli_renderer form chrome");
    }

    #[test]
    fn screenshot_file_browser_action_palette_and_intent_preview_chip() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().join("workspace-root");
        std::fs::create_dir_all(root.join("src")).expect("mkdir src");
        std::fs::create_dir_all(root.join("docs")).expect("mkdir docs");
        std::fs::write(root.join("Cargo.toml"), b"[package]\nname = \"plexi\"\n")
            .expect("write Cargo.toml");
        std::fs::write(root.join("main.rs"), b"fn main() { // todo: wire up }\n")
            .expect("write main.rs");

        let mut h = PlexiUiHarness::new_sized_ppp(880.0, 560.0, 2.0);
        let root_clone = root.clone();
        h.with_app_mut(|app| {
            app.sidebar_visible = false;
            app.open_builtin_app_pane(
                Box::new(crate::file_browser::FileBrowserApp::new(root_clone.clone())),
                AppPermissions::builtin(),
                root_clone,
                None,
                None,
                None,
            );
        });
        h.run_steps(3);

        let tap_key = |h: &mut PlexiUiHarness, key: egui::Key, modifiers: egui::Modifiers| {
            h.harness().input_mut().events.push(egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            });
            h.harness().input_mut().events.push(egui::Event::Key {
                key,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers,
            });
        };

        // Press `/` to open the inline action palette pre-filled with `/`.
        tap_key(&mut h, egui::Key::Slash, egui::Modifiers::NONE);
        h.run_steps(2);
        assert!(
            h.host_has_label("Type a command or use \u{2191}\u{2193} to choose an action \u{2014} Enter to run"),
            "bare `/` palette open must show neutral command-mode hint chip"
        );
        h.save_screenshot(&evidence_png!("_slash_open"))
            .expect("render file_browser action palette on `/`");

        // Type `sort by size` so `/sort by size` candidates and preview chip render.
        h.harness()
            .input_mut()
            .events
            .push(egui::Event::Text("sort by size".to_string()));
        h.run_steps(2);
        h.save_screenshot(&evidence_png!("_sort_by_size"))
            .expect("render file_browser action palette on `/sort by size`");

        // Escape and re-open `/` to type `new file notes.md` so the structured INTENT preview chip resolves `CreateFile`.
        tap_key(&mut h, egui::Key::Escape, egui::Modifiers::NONE);
        h.run_steps(2);
        tap_key(&mut h, egui::Key::Slash, egui::Modifiers::NONE);
        h.run_steps(2);
        h.harness()
            .input_mut()
            .events
            .push(egui::Event::Text("new file notes.md".to_string()));
        h.run_steps(2);
        assert!(
            h.host_has_label("Create file: notes.md \u{2014} open in editor [Enter]"),
            "structured intent preview chip must render resolved CreateFile preview"
        );
        h.save_screenshot(&evidence_png!("_resolved_intent"))
            .expect("render file_browser action palette with resolved intent preview chip");

        // Escape palette, open `Cmd+F` search, and type `todo`.
        tap_key(&mut h, egui::Key::Escape, egui::Modifiers::NONE);
        h.run_steps(2);
        tap_key(
            &mut h,
            egui::Key::F,
            egui::Modifiers {
                command: true,
                ..Default::default()
            },
        );
        h.run_steps(2);
        h.harness()
            .input_mut()
            .events
            .push(egui::Event::Text("todo".to_string()));
        h.run_steps(2);
        h.save_screenshot(&evidence_png!("_search_todo"))
            .expect("render file_browser search with `todo`");
    }

    #[test]
    fn screenshot_edit_commands_header_scope_banner() {
        let mut h = PlexiUiHarness::new_sized(1168.0, 820.0);
        add_focused_pane(&mut h);
        let ws_dir = tempfile::Builder::new()
            .prefix("plexi-ws-")
            .tempdir_in("/tmp")
            .unwrap_or_else(|_| tempfile::tempdir().unwrap());
        let ws_channel_dir = ws_dir.path().join(crate::config::workspace_channel_dir());
        std::fs::create_dir_all(&ws_channel_dir).unwrap();
        let ws_cmds = ws_channel_dir.join("commands.toml");
        std::fs::write(
            &ws_cmds,
            "[commands]\ngs = \"git status -sb\"\n\n[commands.play]\ndescription = \"Open mixer + editor\"\nsteps = [\n  \"pane new --right --name mixer -- echo mix\",\n  \"app open text-editor --split down\",\n]\n",
        )
        .unwrap();

        h.with_app_mut(|app| {
            app.palette_workspace_root = Some(ws_dir.path().to_path_buf());
            app.open_config_in_editor_with_cwd(ws_cmds.clone(), Some(ws_dir.path().to_path_buf()));
        });
        h.run_steps(3);
        assert!(
            h.host_has_label("WORKSPACE"),
            "Edit commands header must render WORKSPACE scope badge"
        );
        assert!(
            h.host_has_label("Switch to global"),
            "Edit commands header must render Switch to global action"
        );
        let out = evidence_png!();
        h.save_screenshot(&out)
            .expect("render edit_commands header scope banner");
        let _ = std::fs::remove_file(&out);
    }
}
