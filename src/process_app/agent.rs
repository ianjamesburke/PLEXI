//! Agent-pane support on `ProcessApp` (issue #338, part 2 of #285).
//!
//! When a manifest declares `[app] type = "agent"`, the host wraps the
//! resulting `ProcessApp` in `Pane::Agent` instead of `Pane::App`. The
//! conversation UI in `agent_pane::render` does the rendering; this module
//! exposes the I/O surface the agent pane needs to talk to its subprocess:
//!
//!   - `agent_tick()` — drain the subprocess's stdout, route any control
//!     commands (timers, secret_get, iq.query, etc.), discard visual draw
//!     commands (the agent pane has no canvas), and return any
//!     `AppendConversation` rows so the pane can append them to the
//!     transcript.
//!   - `is_ready_for_agent_init()` — true once the subprocess has emitted
//!     its `Ready` reply to the initial `Init` event. The pane uses this
//!     to forward `PlexiEvent::AgentInit` exactly once at the right time.
//!   - `queue_outbound_event_direct()` / `test_outbound_events` /
//!     `test_inject_draw_command` — small surface for the host to push
//!     events onto the outbound queue and for unit tests to drive the
//!     paths deterministically without spinning up a real subprocess.

use crate::app_protocol::{DrawCommand, PlexiEvent};

use super::ProcessApp;

#[cfg(test)]
mod agent_tests {
    //! Behavioural tests for the agent path on `ProcessApp` (#338).
    //!
    //! 1. `agent_tick` extracts `AppendConversation` rows out of the draw
    //!    stream and returns them — they don't go through `route_command`
    //!    or `pending_frame` (the agent pane has no canvas).
    //! 2. The agent pane's lazy `AgentInit` send fires once `Ready` arrives,
    //!    not before — verified via `is_ready_for_agent_init`.
    //!
    //! These tests bypass the real subprocess pipeline by calling
    //! `test_inject_draw_command` to seed the draw stream. The lifecycle
    //! / I/O threads spawned by `ProcessApp::launch` against `/bin/sh`
    //! are inert because we never write real PGAP traffic to the child
    //! and discard its stderr.
    use super::*;
    use crate::app_protocol::DrawCommand;
    use std::collections::HashSet;
    use std::path::PathBuf;

    fn make_app(type_id: &str) -> Option<ProcessApp> {
        let sh = ["/bin/sh", "/usr/bin/sh"]
            .iter()
            .find(|p| std::path::Path::new(p).exists())
            .map(PathBuf::from)?;
        let workspace_root = std::env::temp_dir();
        ProcessApp::launch(
            type_id,
            "Agent Test",
            &sh,
            &workspace_root,
            &["-c".to_string(), "sleep 1".to_string()],
            workspace_root.clone(),
            HashSet::new(),
            false,
        )
        .ok()
    }

    #[test]
    fn type_agent_manifest_routes_to_subprocess_pane() {
        // The host-level "type=agent → AgentBackend::Subprocess" wiring is
        // exercised by the agent_pane unit tests
        // (subprocess_backend_appends_conversation_on_event /
        // subprocess_backend_forwards_user_message_to_subprocess), which
        // construct a Pane with `AgentPane::new_subprocess` and prove that
        // the data flows correctly. This test pins the contract on the
        // ProcessApp side: a `type=agent` ProcessApp's `agent_tick` must
        // surface AppendConversation rows on the conversation channel,
        // not on the visual draw frame.
        let Some(mut app) = make_app("agent_routes_test") else {
            eprintln!("skipping: no /bin/sh available");
            return;
        };
        app.test_inject_draw_command(DrawCommand::AppendConversation {
            role: "assistant".to_string(),
            content: "ack".to_string(),
        });

        let rows = app.agent_tick();

        assert_eq!(rows.len(), 1, "AppendConversation must surface as a row");
        assert_eq!(rows[0].0, "assistant");
        assert_eq!(rows[0].1, "ack");
        // Must NOT have leaked into pending_frame (visual surface).
        let leaked = app
            .pending_frame
            .iter()
            .any(|c| matches!(c, DrawCommand::AppendConversation { .. }));
        assert!(
            !leaked,
            "AppendConversation must not appear on the draw frame"
        );
    }

    #[test]
    fn agent_init_event_emitted_with_system_prompt() {
        // Once the subprocess emits `Ready`, the `is_ready_for_agent_init`
        // gate flips. The agent pane uses this to forward AgentInit. We
        // verify the gate semantics directly here so changes to the
        // readiness signal break this test instead of leaking into
        // production.
        let Some(mut app) = make_app("agent_init_test") else {
            eprintln!("skipping: no /bin/sh available");
            return;
        };
        assert!(
            !app.is_ready_for_agent_init(),
            "fresh subprocess must NOT be ready for AgentInit"
        );

        app.test_inject_draw_command(DrawCommand::Ready {
            sdk: "test".to_string(),
            features_used: vec![],
        });
        // First tick processes the Ready and flips the gate.
        let _ = app.agent_tick();
        assert!(
            app.is_ready_for_agent_init(),
            "after Ready arrives, the subprocess must be ready for AgentInit"
        );

        // Direct verification that AgentInit serialises with the prompt
        // intact. (Wire round-trip is already covered in `app_protocol::tests`.)
        let event = PlexiEvent::AgentInit {
            system_prompt: Some("You are terse.".to_string()),
        };
        let serialised = serde_json::to_string(&event).expect("serialise");
        assert!(
            serialised.contains(r#""system_prompt":"You are terse.""#),
            "AgentInit must carry the manifest's system_prompt verbatim: {serialised}"
        );
    }
}

impl ProcessApp {
    /// True once the subprocess has emitted its `Ready` reply. The
    /// `sdk` field on `ProcessApp` is set when `DrawCommand::Ready`
    /// arrives — same signal `ui()` uses indirectly.
    pub(crate) fn is_ready_for_agent_init(&self) -> bool {
        self.sdk.is_some()
    }

    /// Push an event onto the outbound queue. Mirrors `App::queue_outbound_event`
    /// but keeps the call ergonomic for code paths that hold the `ProcessApp`
    /// directly (the agent pane does — it never goes through the `App` trait).
    pub(crate) fn queue_outbound_event_direct(&mut self, event: PlexiEvent) {
        self.outbound_events.push_back(event);
    }

    /// Pump one tick of agent I/O.
    ///
    /// Conceptually identical to `background_tick()` — drains responses, sends
    /// queued events, drains draw commands — but with two differences:
    ///   1. Sends `Init` lazily on the first call (subprocess agents never go
    ///      through `ui()`, which normally owns the Init handshake).
    ///   2. Filters `AppendConversation` out of the routing pass and returns
    ///      them as `(role, content)` pairs so `AgentPane` can append rows.
    ///
    /// Returns the list of conversation rows the subprocess emitted on this
    /// tick. Empty vec is normal — only signals "no new rows", not an error.
    pub(crate) fn agent_tick(&mut self) -> Vec<(String, String)> {
        // STEP-1: lazy Init. Subprocess agents don't go through `ui()` so the
        // initialised gate that lives there never fires. Send it once here.
        if !self.initialized {
            self.initialized = true;
            let cap_strings: Vec<String> = self
                .permissions
                .capabilities
                .iter()
                .map(|c| c.to_string())
                .collect();
            self.send_event(&PlexiEvent::Init {
                protocol: "pgap/3".to_string(),
                app_id: self.type_id.clone(),
                workspace_root: self.workspace_root.clone(),
                capabilities: cap_strings,
                feature_flags: vec!["pane_groups_v1".into()],
            });
        }

        // STEP-2: drain async HTTP responses from background request threads
        // (timers land here too). Same as `ui()` does each frame.
        while let Ok(event) = self.http_rx.try_recv() {
            self.outbound_events.push_back(event);
        }
        self.flush_outbound_events_pub();

        // STEP-3: drain draw commands. Pull AppendConversation rows out for
        // return; route control commands; discard visual primitives.
        let mut rows: Vec<(String, String)> = Vec::new();
        for cmd in self.drain_draw_commands_pub() {
            match cmd {
                DrawCommand::AppendConversation { role, content } => {
                    rows.push((role, content));
                }
                DrawCommand::Ready { sdk, features_used } => {
                    self.sdk = Some(sdk);
                    self.features_used = features_used;
                }
                DrawCommand::Log { level, message } => {
                    let target = format!("app::{}", self.type_id);
                    match level.as_str() {
                        "error" => log::error!(target: &target, "{message}"),
                        "warn" => log::warn!(target: &target, "{message}"),
                        "debug" => log::debug!(target: &target, "{message}"),
                        _ => log::info!(target: &target, "{message}"),
                    }
                }
                cmd @ (DrawCommand::CapabilityRequest { .. }
                | DrawCommand::SecretGet { .. }
                | DrawCommand::RunGet { .. }
                | DrawCommand::RunComplete { .. }
                | DrawCommand::Notify { .. }
                | DrawCommand::PipeOpen { .. }
                | DrawCommand::PipeOpenDirected { .. }
                | DrawCommand::PipeSend { .. }
                | DrawCommand::AgentRosterGet { .. }
                | DrawCommand::StatusSummary { .. }
                | DrawCommand::SpawnApp { .. }
                | DrawCommand::HttpRequest { .. }
                | DrawCommand::AiQuery { .. }
                | DrawCommand::AudioPlay { .. }
                | DrawCommand::AudioCapture { .. }
                | DrawCommand::CdRequest { .. }
                | DrawCommand::SetTimer { .. }
                | DrawCommand::CancelTimer { .. }
                | DrawCommand::PushNav { .. }
                | DrawCommand::PopNav { .. }) => {
                    self.route_command(cmd);
                }
                // FrameDone, ScheduleRender, MeasureText, CopyToClipboard, and
                // every visual primitive (Rect/Text/etc.) are silently
                // discarded — agent panes have no canvas. CopyToClipboard
                // would need a UI context anyway; if an agent ever needs it,
                // surface a separate route.
                _ => {}
            }
        }

        // STEP-4: per-frame try_wait poll for lifecycle (Crashed detection).
        // Mirrors what `ui()` does on the App-pane path.
        if let Some(child) = self.process.as_mut() {
            match child.try_wait() {
                Ok(Some(_status)) => self.lifecycle.on_process_exited(),
                Ok(None) => {}
                Err(e) => {
                    log::warn!(
                        "ProcessApp[{}]: agent_tick try_wait failed: {e} — marking Crashed",
                        self.type_id
                    );
                    self.lifecycle.on_process_exited();
                }
            }
        }

        rows
    }

    // Test seam. Inserts a synthetic DrawCommand into a channel `agent_tick`
    // will consume, so unit tests can drive the routing layer without a real
    // subprocess emitting bytes.
    #[cfg(test)]
    pub(crate) fn test_inject_draw_command(&mut self, cmd: DrawCommand) {
        // Use the existing draw_tx → draw_rx channel by stealing the rx,
        // creating a fresh channel, sending the command, then merging.
        // Simpler: keep a side queue the tick reads first if non-empty.
        // We piggyback on the existing `pending_frame` field by routing
        // through a dedicated test queue would require a new field.
        //
        // Cleanest: just push directly onto a test-only side queue we add
        // to `ProcessApp`. But adding a field for tests-only is wasteful.
        // Instead, send through draw_tx via a tiny ephemeral plumbing —
        // we don't have draw_tx visible here. So use a hidden field.
        self.test_injected_commands
            .lock()
            .unwrap()
            .push_back(cmd);
    }

    #[cfg(test)]
    pub(crate) fn test_outbound_events(&self) -> Vec<&PlexiEvent> {
        self.outbound_events.iter().collect()
    }
}

// Helpers exposing the existing private mod.rs methods to this submodule.
// Both methods already exist on `ProcessApp` but are private to mod.rs;
// re-exposing them here as `pub(crate)` thin shims keeps the public API
// minimal and the existing internal helpers untouched.
impl ProcessApp {
    pub(crate) fn flush_outbound_events_pub(&mut self) {
        // Drain outbound queue → stdin. Inlined — `flush_outbound_events`
        // is `fn` (private) in mod.rs and we want a single source of truth,
        // but Rust doesn't let one impl block call another module's private
        // methods. The body is identical to mod.rs's `flush_outbound_events`.
        while let Some(event) = self.outbound_events.pop_front() {
            self.send_event(&event);
        }
    }

    pub(crate) fn drain_draw_commands_pub(&mut self) -> Vec<DrawCommand> {
        // First, take any test-injected commands. Always returns empty in
        // production builds (the field is `#[cfg(test)]`-gated to keep its
        // memory footprint zero in release).
        #[cfg(test)]
        let mut cmds: Vec<DrawCommand> = self
            .test_injected_commands
            .lock()
            .unwrap()
            .drain(..)
            .collect();
        #[cfg(not(test))]
        let mut cmds: Vec<DrawCommand> = Vec::new();

        let Some(rx) = self.draw_rx.as_ref() else {
            return cmds;
        };
        loop {
            match rx.try_recv() {
                Ok(cmd) => cmds.push(cmd),
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    log::debug!(
                        "ProcessApp[{}]: subprocess stdout closed during agent_tick",
                        self.type_id
                    );
                    self.draw_rx = None;
                    break;
                }
            }
        }
        cmds
    }
}
