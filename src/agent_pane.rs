/// Agent pane: conversation UI scaffolding (header / transcript / input)
/// backed by a subprocess agent (issue #338, part 2 of #285).
///
/// Layout (top→bottom):
///   Header: agent manifest id
///   Status: animated "working..." when in-flight (top of content, always visible)
///   Transcript: scrollable, sticks to bottom
///   Input: multiline — Enter sends, Shift+Enter inserts newline (no hint shown)
///
/// `AgentBackend` holds the `SubprocessAgent` — a `ProcessApp` running an
/// external binary that speaks PGAP. It receives `PlexiEvent::AgentInit` once
/// at startup, `PlexiEvent::UserMessage` on every submit, and emits
/// `DrawCommand::AppendConversation` rows that the host appends to the transcript.
///
/// Large pastes (>300 chars or multiline) appear collapsed in the transcript as
/// "You: [pasted text — N chars]" — the full text is still sent to the agent.
use crate::app_protocol::{AgentInfo, PlexiEvent};
use crate::pane::Pane;
use crate::process_app::ProcessApp;
use crate::theme::Colors;
use crate::tiling::PaneId;
use std::path::PathBuf;

// ── Display helpers ──────────────────────────────────────────────────────────

/// Render a user-typed message into a transcript line, collapsing oversized /
/// multiline pastes per the spec. Shared by both backends so the UI stays
/// consistent.
fn format_user_message(message: &str) -> String {
    if message.len() > 300 || message.contains('\n') {
        let lines = message.lines().count();
        if lines > 1 {
            format!("You: [pasted text — {lines} lines, {} chars]", message.len())
        } else {
            format!("You: [pasted text — {} chars]", message.len())
        }
    } else {
        format!("You: {message}")
    }
}

// ── Subprocess backend (PGAP-speaking external agent — issue #338) ───────────

/// Backend that wraps a `ProcessApp` running a `type = "agent"` manifest. The
/// host renders the conversation UI; the agent subprocess decides what to say
/// by emitting `DrawCommand::AppendConversation` rows in response to
/// `PlexiEvent::UserMessage` events the host forwards from the input box.
pub struct SubprocessAgent {
    pub process: Box<ProcessApp>,
    pub system_prompt: Option<String>,
    pub manifest_id: String,
    /// `true` once `AgentInit` has been forwarded to the subprocess. Sending
    /// happens lazily on the first drain after the subprocess marks Ready —
    /// the launch path doesn't have a synchronous "Ready" handshake.
    init_sent: bool,
    /// Tracks whether a turn is currently in flight (a UserMessage was sent
    /// and we're awaiting at least one AppendConversation back). Cleared
    /// when an assistant row arrives.
    in_flight: bool,
}

impl SubprocessAgent {
    pub fn new(process: Box<ProcessApp>, system_prompt: Option<String>, manifest_id: String) -> Self {
        Self {
            process,
            system_prompt,
            manifest_id,
            init_sent: false,
            in_flight: false,
        }
    }

    /// Send `PlexiEvent::AgentInit` if the subprocess has finished Init/Ready
    /// and we haven't already. Idempotent — safe to call every drain.
    fn maybe_send_init(&mut self) {
        if self.init_sent {
            return;
        }
        if !self.process.is_ready_for_agent_init() {
            return;
        }
        let event = PlexiEvent::AgentInit {
            system_prompt: self.system_prompt.clone(),
        };
        log::info!(
            "agent_pane[{}]: sending AgentInit (system_prompt={})",
            self.manifest_id,
            self.system_prompt
                .as_deref()
                .map(|s| format!("{} chars", s.len()))
                .unwrap_or_else(|| "unset".to_string())
        );
        self.process.queue_outbound_event_direct(event);
        self.init_sent = true;
    }
}

// ── AgentBackend ─────────────────────────────────────────────────────────────

/// Holds the subprocess agent backend. A separate enum is preserved here for
/// forward-compatibility — additional backend variants may be added in future
/// without changing the `AgentPane` public surface.
pub enum AgentBackend {
    /// Subprocess agent (manifest `type = "agent"` path, issue #338). Speaks
    /// PGAP, receives `UserMessage` / `AgentInit`, emits `AppendConversation`.
    Subprocess(SubprocessAgent),
}

// ── AgentPane ────────────────────────────────────────────────────────────────

pub struct AgentPane {
    pub id: PaneId,
    pub transcript: Vec<String>,
    pub input_buf: String,
    pub in_flight: bool,
    pub font_size: f32,
    needs_focus: bool,

    pub backend: AgentBackend,
}

impl AgentPane {
    /// Construct a subprocess-backed agent pane (#338). The `process` is a
    /// freshly launched `type = "agent"` ProcessApp; the host owns its
    /// lifetime and pumps it via `agent_tick` on every render.
    pub fn new_subprocess(
        id: PaneId,
        process: Box<ProcessApp>,
        system_prompt: Option<String>,
        manifest_id: String,
    ) -> Self {
        Self {
            id,
            transcript: Vec::new(),
            input_buf: String::new(),
            in_flight: false,
            font_size: 13.0,
            needs_focus: true,
            backend: AgentBackend::Subprocess(SubprocessAgent::new(
                process,
                system_prompt,
                manifest_id,
            )),
        }
    }

    /// Workspace `cwd` for this pane. Uses the subprocess `ProcessApp`'s
    /// `workspace_root`. Used by workspace persistence to round-trip the pane.
    pub fn cwd(&self) -> PathBuf {
        match &self.backend {
            AgentBackend::Subprocess(a) => a.process.workspace_root.clone(),
        }
    }

    fn dispatch_subprocess(&mut self, message: String) {
        log::info!("agent_pane {}: submit (subprocess) {:?}", self.id, message);
        self.transcript.push(format_user_message(&message));
        let AgentBackend::Subprocess(agent) = &mut self.backend;
        agent
            .process
            .queue_outbound_event_direct(PlexiEvent::UserMessage { text: message });
        agent.in_flight = true;
        self.in_flight = true;
    }

    pub fn submit_input(&mut self) {
        let message = self.input_buf.trim().to_string();
        if message.is_empty() {
            return;
        }
        self.input_buf.clear();
        // Subprocess agents don't support interruption today — the agent owns
        // the turn boundary. Submitting while in_flight simply queues another
        // UserMessage; the agent decides how to handle concurrent turns.
        self.dispatch_subprocess(message);
    }

    /// Drain backend events into the transcript. Returns true if caller should
    /// request a repaint (events arrived OR a turn is still running).
    pub fn drain_results(&mut self) -> bool {
        self.drain_subprocess()
    }

    fn drain_subprocess(&mut self) -> bool {
        let AgentBackend::Subprocess(ref mut agent) = self.backend;

        // 1. Forward AgentInit lazily once the subprocess is Ready.
        agent.maybe_send_init();

        // 2. Pump I/O + collect AppendConversation rows.
        let new_rows = agent.process.agent_tick();
        let changed = !new_rows.is_empty();
        // Capture whether the agent considers itself in-flight before we consume rows.
        let agent_in_flight_before = agent.in_flight;
        for (role, content) in &new_rows {
            // The agent finished a turn (or part of one) — clear in-flight on
            // any assistant row. Tool / system rows don't toggle the flag.
            if role == "assistant" {
                let AgentBackend::Subprocess(ref mut a) = self.backend;
                a.in_flight = false;
                self.in_flight = false;
                self.needs_focus = true;
            }
            self.transcript.push(format_conversation_row(role, content));
        }
        let _ = agent_in_flight_before;

        // Repaint while a turn is in progress so the working indicator animates.
        changed || self.in_flight
    }
}

/// Render a `(role, content)` pair from `AppendConversation` into a transcript
/// line that the existing UI styling matches on (You:/Agent:/  ↳ /Error:).
/// Unknown roles fall through as plain text per the spec — forward-compat for
/// future role kinds.
fn format_conversation_row(role: &str, content: &str) -> String {
    match role {
        "user" => format_user_message(content),
        "assistant" => format!("Agent: {content}"),
        "tool" => format!("  ↳ {content}"),
        "system" => format!("Error: {content}"),
        _ => content.to_string(),
    }
}

// ── Render ───────────────────────────────────────────────────────────────────

pub fn render(ui: &mut egui::Ui, pane: &mut AgentPane, colors: &Colors) {
    let bg = colors.terminal_bg;
    let text_color = colors.text_primary;
    let dim_color = colors.text_dim;
    let accent = colors.accent;
    let tool_color = egui::Color32::from_rgb(0x89, 0xb4, 0xfa);
    let error_color = egui::Color32::from_rgb(0xf3, 0x8b, 0xa8);

    egui::Frame::new()
        .fill(bg)
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            // ── Header ──────────────────────────────────────────────────────
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("AI").size(11.0).color(accent));
                let AgentBackend::Subprocess(agent) = &pane.backend;
                ui.label(
                    egui::RichText::new(format!("  {}", agent.manifest_id))
                        .size(10.0)
                        .color(dim_color),
                );
            });

            // ── Working status — top of content, animated ────────────────────
            if pane.in_flight {
                let t = ui.input(|i| i.time);
                let dot_count = (t * 1.5) as usize % 4;
                let dots = &"..."[..dot_count];
                let spaces = &"   "[..3 - dot_count];
                let status = format!("working{dots}{spaces}");
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(status)
                            .size(10.0)
                            .italics()
                            .color(dim_color),
                    );
                });
            }

            ui.add_space(4.0);

            // ── Transcript ───────────────────────────────────────────────────
            // Reserve space for separator + input at bottom.
            let input_reserve = pane.font_size * 3.0 + 32.0;
            let avail = ui.available_height() - input_reserve;
            egui::ScrollArea::vertical()
                .max_height(avail.max(40.0))
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for line in &pane.transcript {
                        let (color, size) = if line.starts_with("You: ") {
                            (text_color, pane.font_size)
                        } else if line.starts_with("Error:") {
                            (error_color, pane.font_size)
                        } else if line.starts_with("  ↳ ") {
                            (tool_color, pane.font_size - 1.5)
                        } else {
                            (dim_color, pane.font_size)
                        };
                        ui.label(egui::RichText::new(line).size(size).color(color));
                        ui.add_space(2.0);
                    }
                });

            ui.add_space(4.0);
            ui.separator();
            ui.add_space(4.0);

            // ── Input ────────────────────────────────────────────────────────
            // Right-to-left so the button claims its space first, then the
            // TextEdit expands into whatever remains.
            let prompt_color = if pane.in_flight { error_color } else { accent };
            let mut enter_pressed = false;
            let mut resp_opt: Option<egui::Response> = None;
            let mut button_clicked = false;

            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let send_label = if pane.in_flight { "Stop" } else { "Send" };
                    button_clicked = ui
                        .add_enabled(
                            !pane.input_buf.trim().is_empty() || pane.in_flight,
                            egui::Button::new(
                                egui::RichText::new(send_label).size(12.0).color(prompt_color),
                            ),
                        )
                        .clicked();

                    let input = egui::TextEdit::multiline(&mut pane.input_buf)
                        .desired_rows(2)
                        .desired_width(f32::INFINITY)
                        .hint_text("Message…")
                        .font(egui::FontId::monospace(pane.font_size))
                        .text_color(text_color)
                        .frame(false);
                    let resp = ui.add(input);
                    enter_pressed = resp.has_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift);
                    resp_opt = Some(resp);
                });
            });

            if let Some(resp) = resp_opt {
                let nothing_focused = ui.ctx().memory(|m| m.focused().is_none());
                if pane.needs_focus || nothing_focused {
                    resp.request_focus();
                    pane.needs_focus = false;
                }
            }

            if enter_pressed || button_clicked {
                pane.submit_input();
                pane.needs_focus = true;
            }
        });
}

pub fn render_and_drain(ui: &mut egui::Ui, pane: &mut AgentPane, colors: &Colors) -> bool {
    let needs_repaint = pane.drain_results();
    render(ui, pane, colors);
    needs_repaint
}

// ── Roster enumeration (#286) ────────────────────────────────────────────────

/// Walk a workspace's pane container and surface every `Pane::Agent` as an
/// `AgentInfo` row. Used by `DrawCommand::AgentRosterGet` routing.
///
/// `name` resolves to the agent's manifest id. Output ordering is `pane_id`
/// ascending so the snapshot is reproducible across calls (and across test runs).
pub fn enumerate_agents<'a, I>(panes: I) -> Vec<AgentInfo>
where
    I: IntoIterator<Item = &'a Pane>,
{
    let mut rows: Vec<AgentInfo> = panes
        .into_iter()
        .filter_map(|pane| pane.as_agent())
        .map(|agent| {
            let (app_id, name) = match &agent.backend {
                AgentBackend::Subprocess(sub) => {
                    (sub.manifest_id.clone(), sub.manifest_id.clone())
                }
            };
            AgentInfo {
                pane_id: agent.id,
                app_id,
                name,
            }
        })
        .collect();
    rows.sort_by_key(|a| a.pane_id);
    rows
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    //! Behavioural tests for the subprocess `AgentPane` backend (issue #338).
    //!
    //! Subprocess backend:
    //!   - `dispatch_subprocess` appends a "You: ..." row and queues a
    //!     `PlexiEvent::UserMessage` on the wrapped `ProcessApp`.
    //!   - When the subprocess emits an `AppendConversation` row, the host
    //!     extracts it via `agent_tick()` and appends to the transcript.
    //!
    //! Tests don't exercise the renderer (egui isn't easy to drive headless);
    //! we drive the data layer directly.
    use super::*;
    use crate::app_protocol::DrawCommand;
    use std::collections::HashSet;
    use std::path::PathBuf;

    /// Spawn `/bin/sh -c "sleep 1"` so the lifecycle threads are happy. We
    /// never actually exchange real PGAP traffic with this child — every test
    /// drives the data structures directly.
    fn make_process_app(type_id: &str) -> Option<Box<ProcessApp>> {
        let sh = ["/bin/sh", "/usr/bin/sh"]
            .iter()
            .find(|p| std::path::Path::new(p).exists())
            .map(PathBuf::from)?;
        let workspace_root = std::env::temp_dir();
        let app = ProcessApp::launch(
            type_id,
            "Agent Test",
            &sh,
            &workspace_root,
            &["-c".to_string(), "sleep 1".to_string()],
            workspace_root.clone(),
            HashSet::new(),
            false,
        )
        .ok()?;
        Some(Box::new(app))
    }

    #[test]
    fn subprocess_backend_appends_conversation_on_event() {
        // Given a subprocess-backed AgentPane, when an AppendConversation
        // row lands in the ProcessApp's pending_frame it should surface on
        // the transcript via `drain_results`.
        let Some(process) = make_process_app("agent_test_append") else {
            eprintln!("skipping: no /bin/sh available");
            return;
        };
        let mut pane = AgentPane::new_subprocess(
            42,
            process,
            Some("You are terse.".to_string()),
            "agent-test".to_string(),
        );
        // Inject the row directly into the test seam. `agent_tick()` will
        // pull it into the transcript on the next drain.
        let AgentBackend::Subprocess(agent) = &mut pane.backend;
        agent.process.test_inject_draw_command(DrawCommand::AppendConversation {
            role: "assistant".to_string(),
            content: "Hello!".to_string(),
        });

        pane.drain_results();

        assert!(
            pane.transcript.iter().any(|line| line.contains("Hello!")),
            "transcript must include the appended assistant content; got: {:?}",
            pane.transcript
        );
        assert!(
            pane.transcript.last().unwrap().starts_with("Agent: "),
            "assistant row must render with the 'Agent: ' prefix"
        );
    }

    #[test]
    fn subprocess_backend_forwards_user_message_to_subprocess() {
        // Given a subprocess-backed AgentPane, when the user submits text in
        // the input box, `submit_input` must (1) append a "You: ..." row to
        // the local transcript AND (2) queue a `PlexiEvent::UserMessage` on
        // the ProcessApp's outbound event queue so the host writes it to the
        // subprocess's stdin on the next flush.
        let Some(process) = make_process_app("agent_test_user_msg") else {
            eprintln!("skipping: no /bin/sh available");
            return;
        };
        let mut pane = AgentPane::new_subprocess(7, process, None, "agent-test".to_string());

        pane.input_buf = "Tell me a joke.".to_string();
        pane.submit_input();

        assert!(
            pane.transcript
                .iter()
                .any(|line| line == "You: Tell me a joke."),
            "transcript must include the user message"
        );
        assert!(pane.in_flight, "subprocess pane must be in_flight after submit");
        assert_eq!(pane.input_buf, "", "input buffer must clear after submit");

        // Inspect the outbound event queue on the underlying ProcessApp.
        let AgentBackend::Subprocess(agent) = &pane.backend;
        let outbound = agent.process.test_outbound_events();
        let queued: Vec<&PlexiEvent> = outbound
            .into_iter()
            .filter(|e| matches!(e, PlexiEvent::UserMessage { .. }))
            .collect();
        assert_eq!(queued.len(), 1, "exactly one UserMessage must be queued");
        match queued[0] {
            PlexiEvent::UserMessage { text } => assert_eq!(text, "Tell me a joke."),
            _ => unreachable!(),
        }
    }

    // ── Roster enumeration (#286) ────────────────────────────────────────

    #[test]
    fn enumerate_agents_empty_when_no_agents() {
        let panes: Vec<Pane> = vec![];
        let roster = enumerate_agents(panes.iter());
        assert!(roster.is_empty(), "no agents → empty roster");
    }
}
