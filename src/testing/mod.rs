//! HostHarness — headless egui test harness for host self-validation (issue #552).
//!
//! Layers:
//!   1. `HostHarness` — runs `PlexiApp` in a headless `egui::Context`. Input
//!      events are injected via `RawInput`; observable state is extracted into
//!      `HostSnapshot` after each frame.
//!   2. Pane/IPC injection for host-model behavior without a display server.

use crate::app::permissions::AppPermissions;
use crate::app::PlexiApp;
use crate::app_protocol::AppRequest;
use crate::config::set_test_profile_dir;
use crate::host::pane::{AppPane, AppRuntime, Pane};
use crate::spatial::tiling::PaneId;
use egui::RawInput;
use std::collections::HashMap;
use std::sync::{atomic::AtomicU64, Arc};

// ─── HostSnapshot ────────────────────────────────────────────────────────────

/// Observable state extracted from `PlexiApp` after a frame. Returned by
/// `HostHarness::state()` — tests assert against this instead of reaching
/// into `PlexiApp` internals.
#[derive(Debug, Clone)]
pub struct HostSnapshot {
    pub open_panes: Vec<PaneId>,
    pub pane_titles: HashMap<PaneId, String>,
}

impl HostSnapshot {
    fn from_app(app: &PlexiApp) -> Self {
        let win = &app.windows[app.active_window];
        let open_panes: Vec<PaneId> = win.panes.keys().copied().collect();
        let pane_titles: HashMap<PaneId, String> = win
            .panes
            .iter()
            .map(|(id, pane)| {
                let title = match pane {
                    Pane::App(p) => p.name.clone(),
                    Pane::Terminal(_) => "Terminal".to_string(),
                    Pane::Portal(p) => format!("Portal({})", p.target_context_id),
                };
                (*id, title)
            })
            .collect();
        Self {
            open_panes,
            pane_titles,
        }
    }
}

// ─── HostHarness ─────────────────────────────────────────────────────────────

/// Headless egui test harness wrapping `PlexiApp`.
///
/// ```rust,no_run
/// let mut h = HostHarness::new();
/// let pane = h.add_test_pane();
/// h.inject_ipc(AppRequest::SetPipStatus {
///     pane_id: pane,
///     status: crate::app_protocol::PipStatus::Green,
/// });
/// h.run_frames(1);
/// ```
pub struct HostHarness {
    /// The app under test.
    pub app: PlexiApp,
    /// Shared egui context — same instance that `app.ctx` holds.
    ctx: egui::Context,
    /// Next pane id to assign.
    next_pane_id: u64,
    /// IPC sender for injecting AppRequests into the pane_ipc channel.
    pub ipc_tx: std::sync::mpsc::Sender<AppRequest>,
    /// Platform output from the most recently completed frame.
    /// Contains clipboard writes, open URLs, etc.
    pub last_platform_output: egui::PlatformOutput,
    /// Keeps the per-test tempdir alive for the harness lifetime; auto-deletes on drop.
    _profile_dir: tempfile::TempDir,
    /// Isolated workspace root for panes the harness creates. Held alive for the
    /// harness lifetime and auto-deleted on drop, so any channel dir a pane
    /// writes (e.g. the assistant's `AssistantStore`) lands here — never in the
    /// shared `std::env::temp_dir()` root, where it would leak into the walk-up
    /// path of every other test's tempdir.
    _workspace_dir: tempfile::TempDir,
    /// Keeps the thread-local profile dir override active for the harness lifetime.
    _profile_guard: crate::config::TestProfileDirGuard,
}

impl HostHarness {
    /// Create a harness with an empty `PlexiApp` and a 1280×800 viewport.
    pub fn new() -> Self {
        let profile_dir = tempfile::TempDir::new().expect("failed to create test profile tempdir");
        let workspace_dir =
            tempfile::TempDir::new().expect("failed to create test workspace tempdir");
        let profile_guard = set_test_profile_dir(profile_dir.path().to_path_buf());
        let ctx = egui::Context::default();
        let frame_tick = Arc::new(AtomicU64::new(0));
        let (app, ipc_tx) = PlexiApp::new_for_test(ctx.clone(), frame_tick);
        Self {
            app,
            ctx,
            next_pane_id: 100,
            ipc_tx,
            last_platform_output: egui::PlatformOutput::default(),
            _profile_dir: profile_dir,
            _workspace_dir: workspace_dir,
            _profile_guard: profile_guard,
        }
    }

    // ── Pane management ──────────────────────────────────────────────────────

    /// Add a builtin app pane (not a Terminal) for host testing.
    /// The pane has builtin permissions (all capability checks pass).
    pub fn add_test_pane(&mut self) -> PaneId {
        self.add_test_pane_with_permissions(AppPermissions::builtin())
    }

    /// Add a test app pane with the given permissions.
    pub fn add_test_pane_with_permissions(&mut self, permissions: AppPermissions) -> PaneId {
        let pane_id = self.next_pane_id;
        self.next_pane_id += 1;

        let app_pane = AppPane {
            pip_status: None,
            id: pane_id,
            runtime: AppRuntime::Builtin(Box::new(crate::file_browser::FileBrowserApp::new(
                std::env::temp_dir(),
            ))),
            workspace_root: std::env::temp_dir(),
            permissions,
            manifest_id: "test".to_string(),
            name: "Test App".to_string(),
            pane_group: None,
            linked_pane_id: None,
            overlay_replaced: None,
            hidden: false,
            agent: None,
            slots: HashMap::new(),
            semantic_state: Default::default(),
        };

        let win = &mut self.app.windows[0];
        win.panes.insert(pane_id, Pane::App(Box::new(app_pane)));
        let tile_id = win.tree.tiles.insert_pane(pane_id);
        if win.tree.root.is_none() {
            win.tree.root = Some(tile_id);
        }

        pane_id
    }

    /// Add a builtin Assistant app pane backed by an inert AI broker. Used by
    /// input-ownership tests that assert on the composer's observable state.
    pub fn add_assistant_pane(&mut self) -> PaneId {
        struct InertBroker;
        impl crate::plexi_ai::broker::AiBroker for InertBroker {
            fn dispatch(
                &self,
                request: crate::plexi_ai::broker::AiBrokerRequest,
                _on_delta: &mut dyn FnMut(crate::plexi_ai::turn_loop::TurnDelta<'_>),
            ) -> crate::plexi_ai::broker::AiBrokerResponse {
                log::warn!(
                    "testing: InertBroker dispatch for '{}' — no AI in harness",
                    request.app_id
                );
                crate::plexi_ai::broker::AiBrokerResponse {
                    content: None,
                    tokens_in: 0,
                    tokens_out: 0,
                    error: Some("no AI broker in HostHarness".to_string()),
                }
            }
        }

        let pane_id = self.next_pane_id;
        self.next_pane_id += 1;
        let workspace_root = self._workspace_dir.path().to_path_buf();
        let assistant = crate::assistant::AssistantApp::new(
            workspace_root.clone(),
            Arc::new(InertBroker),
            &crate::config::config_dir(),
        );
        let app_pane = AppPane {
            pip_status: None,
            id: pane_id,
            runtime: AppRuntime::Builtin(Box::new(assistant)),
            workspace_root,
            permissions: AppPermissions::builtin(),
            manifest_id: "assistant".to_string(),
            name: "Assistant".to_string(),
            pane_group: None,
            linked_pane_id: None,
            overlay_replaced: None,
            hidden: false,
            agent: None,
            slots: HashMap::new(),
            semantic_state: Default::default(),
        };
        let win = &mut self.app.windows[0];
        win.panes.insert(pane_id, Pane::App(Box::new(app_pane)));
        let tile_id = win.tree.tiles.insert_pane(pane_id);
        // Join the layout so this pane actually renders each frame — a tile
        // outside the root container would never reach the tiling behavior.
        match win.tree.root {
            None => win.tree.root = Some(tile_id),
            Some(old_root) => {
                let new_root = win
                    .tree
                    .tiles
                    .insert_horizontal_tile(vec![old_root, tile_id]);
                win.tree.root = Some(new_root);
            }
        }
        pane_id
    }

    /// Mutable access to an assistant pane's concrete app for state assertions.
    pub fn assistant_mut(&mut self, pane_id: PaneId) -> &mut crate::assistant::AssistantApp {
        let pane = self.app.windows[0]
            .panes
            .get_mut(&pane_id)
            .expect("assistant pane not found");
        let AppRuntime::Builtin(app) = &mut pane.as_app_mut().expect("not an app pane").runtime
        else {
            panic!("pane {pane_id} is not a builtin app");
        };
        app.as_any_mut()
            .downcast_mut::<crate::assistant::AssistantApp>()
            .expect("pane is not an AssistantApp")
    }

    // ── Frame pump ───────────────────────────────────────────────────────────

    /// Run one egui frame with the given raw input.
    pub fn frame(&mut self, mut input: RawInput) -> &mut Self {
        let app = &mut self.app;
        use eframe::App;
        app.raw_input_hook(&self.ctx, &mut input);
        let full_output = self.ctx.run_ui(input, |ui| {
            app.ui(ui, &mut eframe::Frame::_new_kittest());
        });
        self.last_platform_output = full_output.platform_output;
        self
    }

    /// Run `n` idle frames (no input events).
    pub fn run_frames(&mut self, n: u32) -> &mut Self {
        for _ in 0..n {
            self.frame(RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 800.0),
                )),
                ..Default::default()
            });
        }
        self
    }

    // ── State inspection ─────────────────────────────────────────────────────

    /// Snapshot observable state from the app after the last frame.
    pub fn state(&self) -> HostSnapshot {
        HostSnapshot::from_app(&self.app)
    }

    /// Number of panes in the active window.
    pub fn pane_count(&self) -> usize {
        self.app.windows[self.app.active_window].panes.len()
    }

    /// Number of windows in the active window list.
    pub fn window_count(&self) -> usize {
        self.app.windows.len()
    }

    /// Focus a pane by PaneId, looking up its TileId in the active window's
    /// tile tree. No-op if the pane has no corresponding tile.
    pub fn focus_pane(&mut self, pane_id: PaneId) -> &mut Self {
        let win = &mut self.app.windows[self.app.active_window];
        let tile_id = win.tree.tiles.iter().find_map(|(tid, tile)| {
            if let egui_tiles::Tile::Pane(pid) = tile {
                if *pid == pane_id {
                    Some(tid)
                } else {
                    None
                }
            } else {
                None
            }
        });
        win.focused_pane = tile_id.copied();
        self
    }

    /// Inject a `AppRequest` directly into the pane_ipc channel.
    pub fn inject_ipc(&self, cmd: AppRequest) -> &Self {
        let _ = self.ipc_tx.send(cmd);
        self
    }

    /// Inject a synthetic pointer click at pane-pixel coordinates (origin at
    /// the pane's top-left), through the real `AppRequest::ClickPane`
    /// dispatch path — the same host code `plexi pane click` drives. A real
    /// pointer move + press + release is delivered into the live production
    /// egui pass, so it exercises the pane's own `canvas_transform`
    /// inversion, never a parallel resolver. Call `run_frames` afterward to
    /// let the dispatch (and, for process apps, the async IPC round trip to
    /// the guest) take effect. `button` is `"left"`, `"right"`, or `"middle"`.
    pub fn inject_click(
        &self,
        pane_id: PaneId,
        x: f32,
        y: f32,
        button: &str,
        response_file: Option<String>,
    ) -> &Self {
        self.inject_ipc(AppRequest::ClickPane {
            pane_id,
            x,
            y,
            button: Some(button.to_string()),
            response_file,
        })
    }

    /// Inject a synthetic node-targeted click, through the real
    /// `AppRequest::ClickPaneNode` dispatch path — the same host code
    /// `plexi pane click <pane_id> --node <node_id>` drives. `node_id`
    /// matches `SemanticPaneNode.id`, the id `plexi pane state` reports for
    /// every node in the pane's tree. The host resolves the node's on-screen
    /// rect during the next render pass and delivers the same
    /// `PendingPaneClick` honest hit-test the pixel path uses. Call
    /// `run_frames` afterward to let the dispatch (and, for process apps,
    /// the async IPC round trip to the guest) take effect. `button` is
    /// `"left"`, `"right"`, or `"middle"`.
    pub fn inject_node_click(
        &self,
        pane_id: PaneId,
        node_id: &str,
        button: &str,
        response_file: Option<String>,
    ) -> &Self {
        self.inject_ipc(AppRequest::ClickPaneNode {
            pane_id,
            node_id: node_id.to_string(),
            button: Some(button.to_string()),
            response_file,
        })
    }
}

#[cfg(test)]
mod flow_tests;
#[cfg(test)]
mod harness_tests;

#[cfg(test)]
mod profile_isolation_tests {
    use super::HostHarness;

    #[test]
    fn host_harness_profile_dir_is_not_in_home() {
        let _h = HostHarness::new();
        let dir = crate::config::config_dir();
        let home = dirs::home_dir().expect("no home dir");
        assert!(
            !dir.starts_with(&home),
            "config_dir() must not point into $HOME inside HostHarness, got: {}",
            dir.display()
        );
    }
}
