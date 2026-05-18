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
                    Pane::SubContext { context_id, .. } => format!("SubContext({context_id})"),
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

    // -- Input drain (issue #1236) ────────────────────────────────────────────

    /// For every FocusLayer variant × every input-intent event type, asserts
    /// that `drain_captured_keyboard_input` removes the event from the buffer.
    /// Non-input events (pointer, scroll) must survive.
    ///
    /// Tests the drain function directly by injecting events into ctx.input_mut
    /// and calling drain_captured_keyboard_input, then reading back what remains.
    #[test]
    fn drain_drops_all_input_intent_events_when_overlay_active() {
        use crate::app::FocusLayer;
        use crate::input_intent;

        let focus_layers = vec![
            FocusLayer::NotificationModal,
            FocusLayer::ConfirmClose,
            FocusLayer::CommandPalette,
            FocusLayer::RenamePane,
            FocusLayer::ContextRename,
            FocusLayer::QuickNote,
            FocusLayer::QuickNoteDestination,
            FocusLayer::QuickNoteSubDestination(vec![0]),
            FocusLayer::CliSetupPrompt,
            FocusLayer::ContextInspector,
        ];

        let input_events = vec![
            egui::Event::Text("hello".into()),
            egui::Event::Paste("pasted".into()),
            egui::Event::Copy,
            egui::Event::Cut,
            egui::Event::Ime(egui::ImeEvent::Commit("日本語".into())),
            egui::Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            },
        ];

        let non_input_event = egui::Event::PointerMoved(egui::pos2(100.0, 100.0));

        for layer in &focus_layers {
            for input_event in &input_events {
                assert!(
                    input_intent::classify(input_event).is_some(),
                    "test bug: {input_event:?} should classify as input intent"
                );

                let mut h = HostHarness::new();
                h.app.push_focus_layer(layer.clone());

                h.app.ctx.input_mut(|i| {
                    i.events.push(input_event.clone());
                    i.events.push(non_input_event.clone());
                });

                h.app.drain_captured_keyboard_input(&h.app.ctx.clone());

                h.app.ctx.input(|i| {
                    for remaining in &i.events {
                        assert!(
                            input_intent::classify(remaining).is_none(),
                            "input-intent event leaked through drain with {layer:?}: {remaining:?}"
                        );
                    }
                    assert!(
                        i.events.iter().any(|e| matches!(e, egui::Event::PointerMoved(_))),
                        "non-input event was incorrectly drained with {layer:?}"
                    );
                });
            }
        }
    }

    /// Verify that the global allowlist keys survive the drain even when an
    /// overlay is active — users must always be able to quit or dismiss.
    #[test]
    fn drain_preserves_global_allowlist_keys() {
        use crate::app::FocusLayer;

        let allowlist_keys = vec![
            (egui::Key::Q, false),  // Cmd+Q
            (egui::Key::W, false),  // Cmd+W
            (egui::Key::A, true),   // Cmd+Shift+A
            (egui::Key::L, true),   // Cmd+Shift+L
            (egui::Key::H, true),   // Cmd+Shift+H
        ];

        for (key, shift) in &allowlist_keys {
            let mut h = HostHarness::new();
            h.app.push_focus_layer(FocusLayer::QuickNote);

            let event = egui::Event::Key {
                key: *key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers {
                    command: true,
                    shift: *shift,
                    ..Default::default()
                },
            };

            h.app.ctx.input_mut(|i| {
                i.events.push(event.clone());
            });

            h.app.drain_captured_keyboard_input(&h.app.ctx.clone());

            let mut found = false;
            h.app.ctx.input(|i| {
                found = i.events.iter().any(|e| matches!(
                    e,
                    egui::Event::Key { key: k, modifiers, .. }
                        if *k == *key && modifiers.command
                ));
            });
            assert!(
                found,
                "allowlist key {key:?} (shift={shift}) was incorrectly drained"
            );
        }
    }

    /// Cmd+0 (quick note) must pass through the drain when any non-quick-note
    /// overlay is active so poll_actions can open the quick note modal.
    #[test]
    fn drain_allows_quick_note_key_through_other_overlays() {
        use crate::app::FocusLayer;

        let non_quick_note_layers = vec![
            FocusLayer::ContextInspector,
            FocusLayer::CommandPalette,
            FocusLayer::NotificationModal,
            FocusLayer::ConfirmClose,
            FocusLayer::RenamePane,
            FocusLayer::TextInput,
            FocusLayer::ContextRename,
            FocusLayer::CliSetupPrompt,
        ];

        for layer in non_quick_note_layers {
            let mut h = HostHarness::new();
            h.app.push_focus_layer(layer.clone());

            let event = egui::Event::Key {
                key: egui::Key::Num0,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers {
                    command: true,
                    ..Default::default()
                },
            };

            h.app.ctx.input_mut(|i| {
                i.events.push(event);
            });

            h.app.drain_captured_keyboard_input(&h.app.ctx.clone());

            let mut found = false;
            h.app.ctx.input(|i| {
                found = i.events.iter().any(|e| matches!(
                    e,
                    egui::Event::Key { key: egui::Key::Num0, modifiers, .. }
                        if modifiers.command && !modifiers.shift
                ));
            });
            assert!(
                found,
                "Cmd+0 was incorrectly drained when {layer:?} was active — quick note should be reachable"
            );
        }
    }

    /// Cmd+0 must be drained when quick note is already the active overlay to
    /// prevent resetting the modal state mid-edit. This includes when another
    /// overlay is stacked on top of QuickNote (e.g. a notification pushed above it).
    #[test]
    fn drain_blocks_quick_note_key_when_quick_note_active() {
        use crate::app::FocusLayer;

        let quick_note_layers = vec![
            FocusLayer::QuickNote,
            FocusLayer::QuickNoteDestination,
            FocusLayer::QuickNoteSubDestination(vec![0]),
        ];

        // Also verify that a non-QuickNote layer on top of QuickNote still blocks Cmd+0.
        {
            let mut h = HostHarness::new();
            h.app.push_focus_layer(FocusLayer::QuickNote);
            h.app.push_focus_layer(FocusLayer::NotificationModal);

            let event = egui::Event::Key {
                key: egui::Key::Num0,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers {
                    command: true,
                    ..Default::default()
                },
            };

            h.app.ctx.input_mut(|i| {
                i.events.push(event);
            });

            h.app.drain_captured_keyboard_input(&h.app.ctx.clone());

            let mut found = false;
            h.app.ctx.input(|i| {
                found = i.events.iter().any(|e| matches!(
                    e,
                    egui::Event::Key { key: egui::Key::Num0, modifiers, .. }
                        if modifiers.command
                ));
            });
            assert!(
                !found,
                "Cmd+0 leaked through drain when NotificationModal is on top of QuickNote"
            );
        }

        for layer in quick_note_layers {
            let mut h = HostHarness::new();
            h.app.push_focus_layer(layer.clone());

            let event = egui::Event::Key {
                key: egui::Key::Num0,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers {
                    command: true,
                    ..Default::default()
                },
            };

            h.app.ctx.input_mut(|i| {
                i.events.push(event);
            });

            h.app.drain_captured_keyboard_input(&h.app.ctx.clone());

            let mut found = false;
            h.app.ctx.input(|i| {
                found = i.events.iter().any(|e| matches!(
                    e,
                    egui::Event::Key { key: egui::Key::Num0, modifiers, .. }
                        if modifiers.command
                ));
            });
            assert!(
                !found,
                "Cmd+0 leaked through drain when {layer:?} was active — quick note should stay open"
            );
        }
    }

}
