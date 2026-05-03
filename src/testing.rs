//! HostHarness — headless egui test harness for host self-validation (issue #552).
//!
//! Layers:
//!   1. `HostHarness` — runs `PlexiApp` in a headless `egui::Context`. Input
//!      events are injected via `RawInput`; observable state is extracted into
//!      `HostSnapshot` after each frame.
//!   2. Protocol injection — `inject(pane_id, cmd)` feeds `DrawCommand`s
//!      directly into a `ProcessApp`'s channel so `route_command` executes
//!      without a subprocess. `effects_drain()` collects `outbound_events`.

use crate::app::PlexiApp;
use crate::app_permissions::AppPermissions;
use crate::app_protocol::{AiMessage, DrawCommand, ModelTier};
use crate::pane::{AppPane, AppRuntime, Pane};
use crate::process_app::ProcessApp;
use crate::tiling::PaneId;
use egui::RawInput;
use std::collections::HashMap;
use std::sync::{
    atomic::AtomicU64,
    mpsc::Sender,
    Arc,
};

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
                    Pane::Agent(_) => "Agent".to_string(),
                    Pane::AgentWorkspace(_) => "AgentWorkspace".to_string(),
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
}

impl HostHarness {
    /// Create a harness with an empty `PlexiApp` and a 1280×800 viewport.
    pub fn new() -> Self {
        let ctx = egui::Context::default();
        let frame_tick = Arc::new(AtomicU64::new(0));
        let app = PlexiApp::new_for_test(ctx.clone(), frame_tick);
        Self {
            app,
            ctx,
            inject_channels: HashMap::new(),
            next_pane_id: 100,
        }
    }

    // ── Pane management ──────────────────────────────────────────────────────

    /// Add a test `ProcessApp` pane. Returns the assigned `PaneId`.
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
        let _ = self.ctx.run(input, |ctx| {
            use eframe::App;
            app.update(ctx, &mut eframe::Frame::_new_kittest());
        });
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

    // ── State inspection ─────────────────────────────────────────────────────

    /// Snapshot observable state from the app after the last frame.
    pub fn state(&self) -> HostSnapshot {
        HostSnapshot::from_app(&self.app)
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
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Build a minimal `DrawCommand::AiQuery` for use in routing tests.
pub fn ai_query(request_id: &str) -> DrawCommand {
    DrawCommand::AiQuery {
        request_id: request_id.to_string(),
        model_tier: ModelTier::Low,
        system: String::new(),
        messages: vec![AiMessage {
            role: "user".to_string(),
            content: "hello".to_string(),
        }],
        tools: vec![],
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_protocol::PlexiEvent;

    // -- DrawCommand routing --------------------------------------------------

    /// Regression guard for PR #536: `AiQuery` was silently dropped into
    /// `pending_frame` (visual buffer) instead of being routed through
    /// `route_command`. Confirmed by the fact that `outbound_events` stays
    /// empty — the command was never dispatched.
    ///
    /// Uses a pane with no `ai.query` capability so the denial response lands
    /// synchronously on `outbound_events` — no background thread, no sleep.
    /// The capability-denied path still proves the regression: if AiQuery
    /// were dropped into `pending_frame` the denial would never happen and
    /// `outbound_events` would remain empty.
    #[test]
    fn ai_query_reaches_route_command_not_pending_frame() {
        let mut h = HostHarness::new();
        // No capabilities → AiQuery hits the synchronous denial path in route_command.
        let pane = h.add_test_pane_with_permissions(AppPermissions::from_capability_strings(&[]));

        h.inject(pane, ai_query("req-1"));

        // Drive routing directly via background_tick — no egui frames needed.
        {
            let win = &mut h.app.windows[0];
            let Some(Pane::App(app_pane)) = win.panes.get_mut(&pane) else {
                panic!("expected App pane");
            };
            let AppRuntime::Process(proc) = &mut app_pane.runtime else {
                panic!("expected Process runtime");
            };
            proc.background_tick();
        }

        let effects = h.effects_drain(pane);
        assert!(
            !effects.is_empty(),
            "AiQuery must produce an outbound event — got none. \
             This means it was silently dropped instead of being routed."
        );
        let has_ai_response = effects.iter().any(|e| matches!(e, PlexiEvent::AiResponse { .. }));
        assert!(has_ai_response, "Expected AiResponse in effects, got: {:?}", effects);
    }

    // -- State snapshot -------------------------------------------------------

    #[test]
    fn add_test_pane_appears_in_snapshot() {
        let mut h = HostHarness::new();
        assert!(h.state().open_panes.is_empty());

        let pane = h.add_test_pane();
        let snap = h.state();
        assert!(snap.open_panes.contains(&pane));
        assert_eq!(snap.pane_titles.get(&pane).map(|s| s.as_str()), Some("Test App"));
    }

    #[test]
    fn two_test_panes_have_distinct_ids() {
        let mut h = HostHarness::new();
        let p1 = h.add_test_pane();
        let p2 = h.add_test_pane();
        assert_ne!(p1, p2);
        let snap = h.state();
        assert_eq!(snap.open_panes.len(), 2);
    }

    // -- Nav stack ------------------------------------------------------------

    /// Regression guard for PR #392: `push_nav` and `pop_nav` commands must be
    /// processed so the host nav stack tracks depth correctly.
    #[test]
    fn push_nav_increments_nav_stack_depth() {
        let mut h = HostHarness::new();
        let pane = h.add_test_pane();

        h.inject(
            pane,
            DrawCommand::PushNav {
                view_id: "detail".to_string(),
                title: "Detail".to_string(),
            },
        );
        h.run_frames(2);

        let win = &h.app.windows[0];
        let Pane::App(app_pane) = win.panes.get(&pane).unwrap() else {
            panic!("expected App pane");
        };
        let AppRuntime::Process(proc) = &app_pane.runtime else {
            panic!("expected Process runtime");
        };
        assert_eq!(proc.nav_stack_depth(), 1, "push_nav should add one entry to the nav stack");
    }

    #[test]
    fn push_pop_nav_returns_to_zero() {
        let mut h = HostHarness::new();
        let pane = h.add_test_pane();

        h.inject(
            pane,
            DrawCommand::PushNav {
                view_id: "detail".to_string(),
                title: "Detail".to_string(),
            },
        );
        h.run_frames(1);

        h.inject(pane, DrawCommand::PopNav {});
        h.run_frames(1);

        let win = &h.app.windows[0];
        let Pane::App(app_pane) = win.panes.get(&pane).unwrap() else {
            panic!("expected App pane");
        };
        let AppRuntime::Process(proc) = &app_pane.runtime else {
            panic!("expected Process runtime");
        };
        assert_eq!(proc.nav_stack_depth(), 0, "pop_nav should empty the nav stack");
    }

    // -- Status summary ───────────────────────────────────────────────────────

    /// Verifies `DrawCommand::StatusSummary` is routed and stored on the pane,
    /// not discarded or dumped into the visual frame buffer.
    #[test]
    fn status_summary_stored_on_process_app() {
        let mut h = HostHarness::new();
        let pane = h.add_test_pane();

        h.inject(
            pane,
            DrawCommand::StatusSummary {
                text: "Working…".to_string(),
            },
        );
        h.run_frames(1);

        let win = &h.app.windows[0];
        let Pane::App(app_pane) = win.panes.get(&pane).unwrap() else {
            panic!("expected App pane");
        };
        let AppRuntime::Process(proc) = &app_pane.runtime else {
            panic!("expected Process runtime");
        };
        assert_eq!(
            proc.status_summary.as_deref(),
            Some("Working…"),
            "StatusSummary command must be routed to process_app.status_summary"
        );
    }
}
