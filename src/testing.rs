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
use crate::app_protocol::{AiMessage, DrawCommand, AppRequest, ModelTier};
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
            DrawCommand::Host(AppRequest::PushNav {
                view_id: "detail".to_string(),
                title: "Detail".to_string(),
            }),
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
            DrawCommand::Host(AppRequest::PushNav {
                view_id: "detail".to_string(),
                title: "Detail".to_string(),
            }),
        );
        h.run_frames(1);

        h.inject(pane, DrawCommand::Host(AppRequest::PopNav {}));
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
            DrawCommand::Host(AppRequest::StatusSummary {
                text: "Working…".to_string(),
            }),
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

    #[test]
    fn set_pane_title_unknown_pane_id_does_not_panic() {
        // Injects SetPaneTitle for a pane_id that doesn't exist.
        // Must run without panicking and log a warn — verifies the drain path is wired.
        let mut h = HostHarness::new();
        h.ipc_tx.send(AppRequest::SetPaneTitle { pane_id: 9999, name: "ghost".into() }).unwrap();
        h.run_frames(1); // must not panic; logs warn "not found"
    }

    /// Regression guard for issue #1018: dismissing the command palette must
    /// surrender egui focus from palette_search so AccessKit holds no stale
    /// focused node ID after the widget is gone. Without the fix, the next
    /// pane close triggers AccessKit's internal consistency check and panics.
    #[test]
    fn palette_close_surrenders_focus_before_pane_close() {
        let mut h = HostHarness::new();
        h.add_test_pane();

        // Open the palette — sync_command_palette_focus pushes the focus layer
        // and the per-frame code requests focus on palette_search.
        h.app.show_command_palette = true;
        h.run_frames(3);

        // Dismiss the palette — sync_command_palette_focus must pop the layer
        // AND surrender palette_search focus so AccessKit has no stale node.
        h.app.show_command_palette = false;
        h.run_frames(2);

        // Close a pane — triggers the AccessKit consistency check that panicked
        // when palette_search focus was not surrendered. Passes if no panic.
        h.app.execute_close_pane();
        h.run_frames(1);
    }

    // -- Shell execution security (issue #1177) ───────────────────────────────

    /// Regression guard: an app without `terminal.bindings` must never reach
    /// the `sh -c` spawn in `StreamProcess`. The host must return
    /// `StreamEnd { exit_code: 1 }` immediately and never spawn a subprocess.
    ///
    /// This is the only app-reachable `sh -c` path in the codebase. Any future
    /// app-reachable shell execution path must add a matching denial test here.
    /// See `docs/security/shell-execution-inventory.md` for the full audit.
    #[test]
    fn stream_process_denied_without_terminal_bindings() {
        use crate::app_protocol::{DrawCommand, AppRequest, PlexiEvent, StreamChannel};

        let mut h = HostHarness::new();
        let pane = h.add_test_pane_with_permissions(AppPermissions::from_capability_strings(&[]));

        h.inject(
            pane,
            DrawCommand::Host(AppRequest::StreamProcess {
                correlation_id: "sec-test-1".to_string(),
                terminal_pane_id: 0,
                command: "echo SHOULD_NOT_RUN".to_string(),
                channel: StreamChannel::Stdout,
            }),
        );

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
            "StreamProcess must produce an outbound event — got none. \
             This means the denial path was not reached."
        );
        let stream_end = effects.iter().find(|e| {
            matches!(e, PlexiEvent::StreamEnd { correlation_id, exit_code: 1 }
                if correlation_id == "sec-test-1")
        });
        assert!(
            stream_end.is_some(),
            "expected StreamEnd {{ exit_code: 1 }} for denied StreamProcess, got: {:?}",
            effects
        );
    }

    // -- Handle-key chain (issue #1764) ──────────────────────────────────────

    /// Notification modal handle_key always returns Consumed so poll_actions and
    /// dispatch_app_key_events are skipped while the modal is open.
    #[test]
    fn notification_modal_handle_key_returns_consumed() {
        use crate::app::FocusLayer;
        use crate::app_trait::KeyDisposition;
        let mut h = HostHarness::new();
        h.app.push_focus_layer(FocusLayer::NotificationModal);
        let ctx = h.app.ctx.clone();
        let disposition = h.app.notification_modal_handle_key(&ctx);
        assert_eq!(
            disposition,
            KeyDisposition::Consumed,
            "notification_modal_handle_key must return Consumed"
        );
    }

    // -- Context root CWD resolution ------------------------------------------

    /// Regression guard for #1534: `reset_active_context` (Cmd+N from welcome
    /// screen) must use `cwd_for_welcome_tab()` — which checks the context root
    /// — rather than hardcoding `home_dir()`.
    #[test]
    fn cwd_for_welcome_tab_returns_context_root_when_set() {
        let root = std::path::PathBuf::from("/tmp");
        let mut h = HostHarness::new();
        h.app.set_active_context_root(root.clone());
        assert_eq!(
            h.app.cwd_for_welcome_tab(),
            root,
            "cwd_for_welcome_tab must return the context root when one is set"
        );
    }

    /// Regression guard for #1534: without a context root, `cwd_for_welcome_tab`
    /// must fall back to the window launch path, not panic or return an arbitrary dir.
    #[test]
    fn cwd_for_welcome_tab_falls_back_to_window_path_when_no_root() {
        let h = HostHarness::new();
        let window_path = h.app.windows[0].path.clone();
        // No root set — fallback chain: None → window.path
        assert_eq!(
            h.app.cwd_for_welcome_tab(),
            window_path,
            "cwd_for_welcome_tab must fall back to window.path when no context root is set"
        );
    }

    // -- Capability modal focus layer -----------------------------------------

    /// Regression guard for #1596: Escape must reach the capability consent
    /// modal and fire deny_once. Before the fix, `dispatch_app_key_events` ran
    /// in step 3 of `update()`, the focused app consumed any key (returning
    /// `consumed = true`), which caused the host to steal Escape from the
    /// input state — the modal in step 4 saw nothing. After the fix the modal
    /// renders in step 2 with exclusive keyboard ownership.
    ///
    /// Observable invariants checked (all follow from a successful deny_once):
    /// 1. `CapabilityModal` is on the focus stack while a prompt is queued.
    /// 2. After Escape, `pending_prompts` is empty (prompt was popped).
    /// 3. After Escape, `CapabilityModal` is removed from the focus stack
    ///    (sync_capability_modal_focus pops it when the queue drains).
    #[test]
    fn capability_modal_escape_fires_deny_once() {
        use crate::app::FocusLayer;
        use crate::process_app::PendingPrompt;
        use crate::pane::AppRuntime;

        let mut h = HostHarness::new();
        let pane = h.add_test_pane();

        // Set the pane as the focused pane so sync_capability_modal_focus finds it.
        let tile_id = {
            let win = &h.app.windows[0];
            win.tree.tiles.iter()
                .find_map(|(id, tile)| match tile {
                    egui_tiles::Tile::Pane(p) if *p == pane => Some(id),
                    _ => None,
                })
                .expect("test pane must have a tile")
        };
        h.app.windows[0].focused_pane = Some(*tile_id);

        // Queue a capability prompt directly on the pane's ProcessApp.
        {
            let win = &mut h.app.windows[0];
            let Some(Pane::App(app_pane)) = win.panes.get_mut(&pane) else {
                panic!("expected App pane");
            };
            let AppRuntime::Process(ref mut proc) = app_pane.runtime else {
                panic!("expected Process runtime");
            };
            proc.pending_prompts.push_back(PendingPrompt::Capability {
                request_id: "req-cap-1".to_string(),
                capability: "net.http".to_string(),
            });
        }

        // One idle frame: sync_capability_modal_focus should push the layer.
        h.run_frames(1);
        assert!(
            h.app.focus_stack.iter().any(|l| *l == FocusLayer::CapabilityModal),
            "CapabilityModal must be on the focus stack when the focused pane has pending prompts"
        );

        // Send Escape. The modal must consume it and pop the prompt (deny_once).
        h.key(egui::Key::Escape, egui::Modifiers::NONE);

        // pending_prompts must be empty — Escape triggered deny_once, which pops
        // the front of the queue. If it's still non-empty, the modal didn't see Escape.
        let prompts_empty = {
            let win = &h.app.windows[0];
            let Some(Pane::App(app_pane)) = win.panes.get(&pane) else {
                panic!("expected App pane");
            };
            let AppRuntime::Process(ref proc) = app_pane.runtime else {
                panic!("expected Process runtime");
            };
            proc.pending_prompts.is_empty()
        };
        assert!(
            prompts_empty,
            "pending_prompts must be empty after Escape — deny_once must have consumed the prompt. \
             If non-empty, dispatch_app_key_events stole Escape before the modal could read it."
        );

        // CapabilityModal must have been popped from the focus stack since
        // sync_capability_modal_focus removes it when pending_prompts drains.
        assert!(
            !h.app.focus_stack.iter().any(|l| *l == FocusLayer::CapabilityModal),
            "CapabilityModal must be removed from the focus stack after deny_once drains the queue"
        );
    }

    /// A buried sync-managed layer must be removed even when another layer sits on top.
    ///
    /// Regression guard for the `pop_focus_layer` → `retain` fix: `pop_focus_layer` only
    /// removes the top entry, so a buried entry would survive until it accidentally
    /// became top again — causing a closed overlay to regain keyboard ownership.
    #[test]
    fn buried_stale_focus_layer_is_removed_by_sync() {
        use crate::app::FocusLayer;

        let mut h = HostHarness::new();

        // Push ConfirmClose by activating its source state and running sync.
        h.app.pending_close = true;
        h.app.sync_confirm_close_focus();
        assert!(
            h.app.focus_stack.iter().any(|l| *l == FocusLayer::ConfirmClose),
            "ConfirmClose must be pushed when pending_close is true"
        );

        // Push CommandPalette on top — now ConfirmClose is buried.
        h.app.show_command_palette = true;
        h.app.sync_command_palette_focus();
        assert_eq!(
            h.app.focus_stack.last(),
            Some(&FocusLayer::CommandPalette),
            "CommandPalette must be at the top after its source state becomes true"
        );
        assert!(
            h.app.focus_stack.iter().any(|l| *l == FocusLayer::ConfirmClose),
            "ConfirmClose must still be in the stack (buried beneath CommandPalette)"
        );

        // Clear ConfirmClose source state — sync must remove the buried entry.
        h.app.pending_close = false;
        h.app.sync_confirm_close_focus();

        assert!(
            !h.app.focus_stack.iter().any(|l| *l == FocusLayer::ConfirmClose),
            "ConfirmClose must be removed from the stack even though CommandPalette was on top. \
             If this fails, sync_confirm_close_focus used pop_focus_layer (top-only) instead of retain."
        );

        // The layer that was on top must still be present — we only removed ConfirmClose.
        assert!(
            h.app.focus_stack.iter().any(|l| *l == FocusLayer::CommandPalette),
            "CommandPalette must remain in the stack after removing the buried ConfirmClose layer"
        );
    }

    // -- QuickNote paste (#1637) -----------------------------------------------

    /// Regression guard for #1637: Cmd+V (paste) while QuickNote is open must
    /// land in `quick_note_text`, not fall through to the terminal pane behind
    /// the overlay.
    ///
    /// The fix: `draw_quick_note_modal` explicitly consumes `egui::Event::Paste`
    /// via `ctx.input_mut` and inserts the text at the cursor position, rather
    /// than relying on the TextEdit's internal event processing (which depends on
    /// focus being fully settled by the time the TextEdit renders).
    #[test]
    fn quick_note_paste_inserts_into_note_text() {
        let mut h = HostHarness::new();

        // Open QuickNote.
        h.app.push_focus_layer(crate::app::FocusLayer::QuickNote);

        // Run two idle frames so the overlay and focus fully settle.
        h.run_frames(2);

        // Inject a paste event in the next frame.
        h.frame(egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            events: vec![egui::Event::Paste("hello world".into())],
            ..Default::default()
        });

        assert_eq!(
            h.app.quick_note_text,
            "hello world",
            "Pasted text must appear in quick_note_text after a Paste event with QuickNote open"
        );
    }

    /// After a paste event with QuickNote open, `quick_note_text` must be
    /// updated AND the `Paste` event must not be present in the queue for
    /// downstream readers (terminal backends, poll_actions).
    #[test]
    fn quick_note_paste_consumed_from_queue() {
        let mut h = HostHarness::new();

        h.app.push_focus_layer(crate::app::FocusLayer::QuickNote);
        h.run_frames(2);

        // Inject paste + a non-input event that must survive.
        h.app.ctx.input_mut(|i| {
            i.events.push(egui::Event::Paste("test".into()));
            i.events.push(egui::Event::PointerMoved(egui::pos2(50.0, 50.0)));
        });

        // Run the overlay draw directly — draw_quick_note_modal consumes key events
        // via ctx.input_mut before the terminal backends run.
        let ctx = h.app.ctx.clone();
        h.app.draw_quick_note_modal(&ctx);

        // Paste event must be gone.
        h.app.ctx.input(|i| {
            assert!(
                !i.events.iter().any(|e| matches!(e, egui::Event::Paste(_))),
                "Paste event must be consumed before terminal backends see it"
            );
            // Non-input event must survive.
            assert!(
                i.events.iter().any(|e| matches!(e, egui::Event::PointerMoved(_))),
                "Non-input events must not be drained"
            );
        });
    }

    // -- Layer 2 post-CentralPanel focus guard (#1601) -------------------------

    /// Helper: simulate a CentralPanel pane widget stealing egui focus between two frames.
    /// Returns the egui Id of the "stealing" widget so callers can assert against it.
    fn steal_focus(h: &HostHarness) -> egui::Id {
        let steal_id = egui::Id::new("fake_pane_text_input_steal");
        h.ctx.memory_mut(|m| m.request_focus(steal_id));
        steal_id
    }

    /// Regression guard for #1601: rename-pane TextEdit must retain egui focus
    /// after CentralPanel renders. The between-frame steal simulates a pane TextInput
    /// calling request_focus during CentralPanel — the post-CentralPanel block must
    /// re-claim focus for `rename_pane_input` before the frame ends.
    #[test]
    fn rename_pane_overlay_focus_wins_after_central_panel_steal() {
        use crate::app::FocusLayer;
        let mut h = HostHarness::new();
        let pane = h.add_test_pane();
        h.app.renaming_pane = Some(pane);
        h.app.rename_buffer = "test name".to_string();
        h.app.push_focus_layer(FocusLayer::RenamePane);
        // Frame 1: one-shot fires, focus = rename_pane_input.
        h.run_frames(1);
        // Simulate CentralPanel pane widget stealing focus.
        steal_focus(&h);
        // Frame 2: post-CentralPanel block must reclaim rename_pane_input.
        h.run_frames(1);
        assert_eq!(
            h.ctx.memory(|m| m.focused()),
            Some(egui::Id::new("rename_pane_input")),
            "rename_pane_input must win focus back after a CentralPanel pane widget steals it"
        );
    }

    /// Regression guard for #1601: context-rename TextEdit (modal path, sidebar hidden)
    /// must retain egui focus after CentralPanel renders.
    #[test]
    fn context_rename_overlay_focus_wins_after_central_panel_steal() {
        use crate::app::FocusLayer;
        let mut h = HostHarness::new();
        h.app.renaming_window = Some(0);
        h.app.rename_buffer = "new context".to_string();
        h.app.sidebar_visible = false;
        h.app.push_focus_layer(FocusLayer::ContextRename);
        h.run_frames(1);
        steal_focus(&h);
        h.run_frames(1);
        assert_eq!(
            h.ctx.memory(|m| m.focused()),
            Some(egui::Id::new("rename_context_input")),
            "rename_context_input must win focus back after a CentralPanel pane widget steals it"
        );
    }

    /// Regression guard for #1601: edit-description TextEdit must retain egui focus
    /// after CentralPanel renders.
    #[test]
    fn edit_description_overlay_focus_wins_after_central_panel_steal() {
        use crate::app::FocusLayer;
        let mut h = HostHarness::new();
        h.app.editing_description = Some(0);
        h.app.description_buffer = "my description".to_string();
        h.app.push_focus_layer(FocusLayer::ContextDescription);
        h.run_frames(1);
        steal_focus(&h);
        h.run_frames(1);
        assert_eq!(
            h.ctx.memory(|m| m.focused()),
            Some(egui::Id::new("edit_description_input")),
            "edit_description_input must win focus back after a CentralPanel pane widget steals it"
        );
    }

    /// Regression guard for #1601: generic text-input overlay field must retain
    /// egui focus after CentralPanel renders.
    #[test]
    fn text_input_overlay_focus_wins_after_central_panel_steal() {
        use crate::app::{FocusLayer, TextInputOverlay, OverlayTarget};
        let mut h = HostHarness::new();
        h.app.text_overlay = Some((
            TextInputOverlay {
                label: "Root directory".to_string(),
                hint: "/path/to/project".to_string(),
                buffer: String::new(),
                focus_requested: false,
            },
            OverlayTarget::ContextRoot(0),
        ));
        h.app.push_focus_layer(FocusLayer::TextInput);
        h.run_frames(1);
        steal_focus(&h);
        h.run_frames(1);
        assert_eq!(
            h.ctx.memory(|m| m.focused()),
            Some(egui::Id::new("text_input_overlay_field")),
            "text_input_overlay_field must win focus back after a CentralPanel pane widget steals it"
        );
    }

    /// Regression guard for #1601: capability/secret prompt TextEdit must retain
    /// egui focus after CentralPanel renders. The secret TextEdit now carries a
    /// stable `capability_secret_input` Id so the post-CentralPanel block can
    /// target it by name.
    #[test]
    fn capability_secret_overlay_focus_wins_after_central_panel_steal() {
        use crate::app::FocusLayer;
        use crate::process_app::PendingPrompt;
        use crate::pane::AppRuntime;

        let mut h = HostHarness::new();
        let pane = h.add_test_pane();

        // Focus the pane so sync_capability_modal_focus can find it.
        let tile_id = {
            let win = &h.app.windows[0];
            win.tree.tiles.iter()
                .find_map(|(id, tile)| match tile {
                    egui_tiles::Tile::Pane(p) if *p == pane => Some(id),
                    _ => None,
                })
                .expect("test pane must have a tile")
        };
        h.app.windows[0].focused_pane = Some(*tile_id);

        // Queue a Secret prompt on the pane.
        {
            let win = &mut h.app.windows[0];
            let Some(Pane::App(app_pane)) = win.panes.get_mut(&pane) else {
                panic!("expected App pane");
            };
            let AppRuntime::Process(ref mut proc) = app_pane.runtime else {
                panic!("expected Process runtime");
            };
            proc.pending_prompts.push_back(PendingPrompt::Secret {
                key: "API_KEY".to_string(),
            });
        }

        // Frame 1: sync pushes CapabilityModal layer; modal renders; post-CentralPanel
        // requests focus on capability_secret_input.
        h.run_frames(1);
        assert!(
            h.app.focus_stack.iter().any(|l| *l == FocusLayer::CapabilityModal),
            "CapabilityModal must be on the focus stack when the focused pane has a pending Secret prompt"
        );

        // Simulate CentralPanel pane widget stealing focus.
        steal_focus(&h);

        // Frame 2: post-CentralPanel block must reclaim capability_secret_input.
        h.run_frames(1);
        assert_eq!(
            h.ctx.memory(|m| m.focused()),
            Some(egui::Id::new("capability_secret_input")),
            "capability_secret_input must win focus back after a CentralPanel pane widget steals it"
        );
    }

}
