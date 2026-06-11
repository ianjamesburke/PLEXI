//! HostHarness — headless egui test harness for host self-validation (issue #552).
//!
//! Layers:
//!   1. `HostHarness` — runs `PlexiApp` in a headless `egui::Context`. Input
//!      events are injected via `RawInput`; observable state is extracted into
//!      `HostSnapshot` after each frame.
//!   2. Protocol injection — `inject(pane_id, cmd)` feeds `DrawCommand`s
//!      directly into a `ProcessApp`'s channel so `route_command` executes
//!      without a subprocess. `effects_drain()` collects `outbound_events`.

use crate::app::permissions::AppPermissions;
use crate::app::PlexiApp;
use crate::app_protocol::{AiMessage, AppRequest, DrawCommand, ModelTier};
use crate::host::pane::{AppPane, AppRuntime, Pane};
use crate::process_app::ProcessApp;
use crate::spatial::tiling::PaneId;
use egui::RawInput;
use std::collections::HashMap;
use std::sync::{atomic::AtomicU64, mpsc::Sender, Arc};

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
/// h.inject(pane, DrawCommand::StatusSummary { text: "hello".into() });
/// h.run_frames(1);
/// assert!(h.effects_drain(pane).is_empty()); // StatusSummary has no outbound event
/// ```
pub struct HostHarness {
    /// The app under test.
    pub app: PlexiApp,
    /// Shared egui context — same instance that `app.ctx` holds.
    ctx: egui::Context,
    /// Inject channels keyed by pane id. Populated by `add_test_pane`.
    inject_channels: HashMap<PaneId, Sender<DrawCommand>>,
    /// Next pane id to assign.
    next_pane_id: u64,
    /// IPC sender for injecting AppRequests into the pane_ipc channel.
    pub ipc_tx: std::sync::mpsc::Sender<AppRequest>,
    /// Platform output from the most recently completed frame.
    /// Contains clipboard writes, open URLs, etc.
    pub last_platform_output: egui::PlatformOutput,
}

impl HostHarness {
    /// Create a harness with an empty `PlexiApp` and a 1280×800 viewport.
    pub fn new() -> Self {
        let ctx = egui::Context::default();
        let frame_tick = Arc::new(AtomicU64::new(0));
        let (app, ipc_tx) = PlexiApp::new_for_test(ctx.clone(), frame_tick);
        Self {
            app,
            ctx,
            inject_channels: HashMap::new(),
            next_pane_id: 100,
            ipc_tx,
            last_platform_output: egui::PlatformOutput::default(),
        }
    }

    // ── Pane management ──────────────────────────────────────────────────────

    /// Add a `ProcessApp` pane (not a Terminal) for protocol testing.
    /// The pane has builtin permissions (all capability checks pass).
    pub fn add_test_pane(&mut self) -> PaneId {
        self.add_test_pane_with_permissions(AppPermissions::builtin())
    }

    /// Add a test `ProcessApp` pane with the given permissions.
    pub fn add_test_pane_with_permissions(&mut self, permissions: AppPermissions) -> PaneId {
        let pane_id = self.next_pane_id;
        self.next_pane_id += 1;

        let (process_app, draw_tx) = ProcessApp::new_for_test(pane_id, permissions.clone());
        let app_pane = AppPane {
            id: pane_id,
            runtime: AppRuntime::Process(Box::new(process_app)),
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
        };

        let win = &mut self.app.windows[0];
        win.panes.insert(pane_id, Pane::App(Box::new(app_pane)));
        let tile_id = win.tree.tiles.insert_pane(pane_id);
        if win.tree.root.is_none() {
            win.tree.root = Some(tile_id);
        }

        self.inject_channels.insert(pane_id, draw_tx);
        pane_id
    }

    // ── Frame pump ───────────────────────────────────────────────────────────

    /// Run one egui frame with the given raw input.
    pub fn frame(&mut self, input: RawInput) -> &mut Self {
        let app = &mut self.app;
        let full_output = self.ctx.run(input, |ctx| {
            use eframe::App;
            app.update(ctx, &mut eframe::Frame::_new_kittest());
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

    /// Inject a pointer click at `pos` on the next frame.
    pub fn click(&mut self, pos: impl Into<egui::Pos2>) -> &mut Self {
        let pos = pos.into();
        self.frame(RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            events: vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                },
            ],
            ..Default::default()
        })
    }

    /// Inject a key press on the next frame.
    pub fn key(&mut self, key: egui::Key, modifiers: egui::Modifiers) -> &mut Self {
        self.frame(RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            events: vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            }],
            ..Default::default()
        })
    }

    // ── Context management ────────────────────────────────────────────────────

    /// Add a sub-context under the given parent context in the host model.
    pub fn add_sub_context(&mut self, context_id: u64, parent_id: u64) {
        self.app.host.add_context(context_id, Some(parent_id));
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

    // ── Protocol injection ───────────────────────────────────────────────────

    /// Inject a `DrawCommand` into the given pane's channel. The command will
    /// be processed during the next `run_frames()` call, following the same
    /// `route_command` path as a real subprocess.
    pub fn inject(&mut self, pane_id: PaneId, cmd: DrawCommand) -> &mut Self {
        if let Some(tx) = self.inject_channels.get(&pane_id) {
            let _ = tx.send(cmd);
        }
        self
    }

    /// Inject a `AppRequest` directly into the pane_ipc channel.
    pub fn inject_ipc(&self, cmd: AppRequest) -> &Self {
        let _ = self.ipc_tx.send(cmd);
        self
    }

    /// Drain and return all `outbound_events` queued by the given pane's
    /// `ProcessApp` since the last call. These are the `PlexiEvent`s the host
    /// would normally send back to the subprocess — in tests they're assertions
    /// that the command was routed and produced a response.
    pub fn effects_drain(&mut self, pane_id: PaneId) -> Vec<crate::app_protocol::PlexiEvent> {
        let win = &mut self.app.windows[0];
        let Some(Pane::App(app_pane)) = win.panes.get_mut(&pane_id) else {
            return vec![];
        };
        let AppRuntime::Process(process_app) = &mut app_pane.runtime else {
            return vec![];
        };
        process_app.outbound_events.drain(..).collect()
    }

    /// Take the latest Render payload queued for a test `ProcessApp`.
    ///
    /// Test apps do not spawn the stdin writer thread, so this helper also
    /// resets `render_in_queue` to simulate that writer consuming the slot.
    pub fn render_payload_take(&mut self, pane_id: PaneId) -> Option<String> {
        let win = &mut self.app.windows[0];
        let Some(Pane::App(app_pane)) = win.panes.get_mut(&pane_id) else {
            return None;
        };
        let AppRuntime::Process(process_app) = &mut app_pane.runtime else {
            return None;
        };
        let payload = process_app.render_slot.lock().unwrap().take();
        process_app
            .render_in_queue
            .store(false, std::sync::atomic::Ordering::Relaxed);
        payload
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Build a minimal `DrawCommand::Host(AppRequest::AiQuery)` for use in routing tests.
pub fn ai_query(request_id: &str) -> DrawCommand {
    DrawCommand::Host(AppRequest::AiQuery {
        request_id: request_id.to_string(),
        model_tier: ModelTier::Low,
        system: String::new(),
        messages: vec![AiMessage {
            role: "user".to_string(),
            content: "hello".to_string(),
        }],
        tools: vec![],
    })
}

#[cfg(test)]
mod flow_tests;
#[cfg(test)]
mod harness_tests;
