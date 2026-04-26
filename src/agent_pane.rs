/// Agent pane: conversation UI scaffolding (header / transcript / input)
/// backed by one of two interchangeable backends (issue #338, part 2 of #285).
///
/// Layout (top→bottom):
///   Header: "IQ  workspace-name"
///   Status: animated "working..." when in-flight (top of content, always visible)
///   Transcript: scrollable, sticks to bottom
///   Input: multiline — Enter sends, Shift+Enter inserts newline (no hint shown)
///
/// `AgentBackend` discriminates *who produces messages*. The render path is
/// identical for both — both backends own a `transcript: Vec<String>` and an
/// `in_flight: bool`, surfaced through `AgentPane::transcript()` /
/// `AgentPane::in_flight()`. Adding a new backend means one match arm in
/// `submit_input` / `drain_results` plus a new state struct.
///
///   - `InProcess` — legacy `claude -p --output-format stream-json` worker
///     thread. Ships unchanged; deletion tracked in #339. Used by Cmd+I.
///   - `Subprocess` — agent-as-app: a `ProcessApp` running an external binary
///     that speaks PGAP. Receives `PlexiEvent::AgentInit` once at startup,
///     `PlexiEvent::UserMessage` on every submit, and emits
///     `DrawCommand::AppendConversation` rows that the host appends to the
///     transcript.
///
/// Interruption: submitting while in-flight kills the subprocess and dispatches
/// the new message immediately. The cancelled turn is discarded silently.
/// (In-process backend only — subprocess backend has no concept of "kill the
/// turn", because the agent decides when its turn ends.)
///
/// Large pastes (>300 chars or multiline) appear collapsed in the transcript as
/// "You: [pasted text — N chars]" — the full text is still sent to the agent.
use crate::agent_turn::{self, WorkerEvent};
use crate::app_protocol::PlexiEvent;
use crate::process_app::ProcessApp;
use crate::theme::Colors;
use crate::tiling::PaneId;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::sync::mpsc;
use std::thread;

// ── Workspace resolution ─────────────────────────────────────────────────────

fn find_iq_dir(start: &Path) -> Option<PathBuf> {
    let mut dir = start.to_path_buf();
    loop {
        let candidate = dir.join(".plexi");
        if candidate.is_dir() {
            return Some(candidate.join("agents").join("iq"));
        }
        if !dir.pop() {
            return None;
        }
    }
}

// ── Soul / memory loading ────────────────────────────────────────────────────

const DEFAULT_SOUL: &str = "\
# Soul

You are the Plexi AI assistant — an expert developer embedded in the Plexi terminal environment.
You have full awareness of the project context from CLAUDE.md files in the workspace.
You are direct, technical, and concise.
You help with coding, architecture, debugging, and project management.
You remember what you have learned about this project and the user.
";

const DEFAULT_MEMORY: &str = "\
# Memory

(This file is updated by the agent to record important learned facts about the project and user.)
";

fn load_soul_context(iq_dir: &Path) -> String {
    let soul_path = iq_dir.join("SOUL.md");
    let memory_path = iq_dir.join("MEMORY.md");

    if let Err(e) = std::fs::create_dir_all(iq_dir) {
        log::warn!("agent_pane: could not create iq dir: {e}");
        return String::new();
    }

    let soul = if soul_path.exists() {
        std::fs::read_to_string(&soul_path).unwrap_or_else(|_| DEFAULT_SOUL.to_string())
    } else {
        let _ = std::fs::write(&soul_path, DEFAULT_SOUL);
        DEFAULT_SOUL.to_string()
    };

    let memory = if memory_path.exists() {
        std::fs::read_to_string(&memory_path).unwrap_or_else(|_| DEFAULT_MEMORY.to_string())
    } else {
        let _ = std::fs::write(&memory_path, DEFAULT_MEMORY);
        DEFAULT_MEMORY.to_string()
    };

    format!("{soul}\n\n{memory}\n\n---\n\n")
}

// ── Session persistence ──────────────────────────────────────────────────────

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct SessionFile {
    session_id: String,
    transcript: Vec<String>,
}

fn session_path(iq_dir: &Path, pane_id: PaneId) -> PathBuf {
    iq_dir.join("sessions").join(format!("{pane_id}.json"))
}

fn load_session(iq_dir: &Path, pane_id: PaneId) -> SessionFile {
    let path = session_path(iq_dir, pane_id);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => return SessionFile::default(),
    };
    serde_json::from_str(&text).unwrap_or_default()
}

fn save_session_file(iq_dir: &Path, pane_id: PaneId, file: &SessionFile) {
    let dir = iq_dir.join("sessions");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        log::error!("agent_pane: failed to create sessions dir: {e}");
        return;
    }
    if let Ok(json) = serde_json::to_string_pretty(file) {
        let _ = std::fs::write(session_path(iq_dir, pane_id), json);
    }
}

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

// ── In-process backend (legacy `claude -p` turn loop) ────────────────────────

struct WorkerMsg {
    session_id: String,
    soul_context: Option<String>,
    message: String,
    cwd: PathBuf,
}

/// Backend that drives the legacy `agent_turn::run_turn` loop on a worker
/// thread. Used by the Cmd+I "open agent pane" path. Will be retired in #339.
pub struct InProcessAgent {
    pub session_id: String,
    pub cwd: PathBuf,
    iq_dir: Option<PathBuf>,
    /// Message queued to send immediately after current turn completes.
    pending_message: Option<String>,
    /// True when the last transcript line is the live streaming agent response.
    streaming_active: bool,
    /// True when we killed the current turn intentionally — suppresses the error display.
    interrupting: bool,
    /// Shared with the worker so we can kill the subprocess to interrupt a turn.
    child_slot: Arc<Mutex<Option<std::process::Child>>>,

    turn_tx: Option<mpsc::SyncSender<WorkerMsg>>,
    event_rx: mpsc::Receiver<WorkerEvent>,
}

impl InProcessAgent {
    fn new(id: PaneId, cwd: PathBuf) -> (Self, Vec<String>) {
        let iq_dir = find_iq_dir(&cwd);

        let (session_id, transcript) = if let Some(ref dir) = iq_dir {
            let sf = load_session(dir, id);
            (sf.session_id, sf.transcript)
        } else {
            (String::new(), Vec::new())
        };

        let child_slot: Arc<Mutex<Option<std::process::Child>>> = Arc::new(Mutex::new(None));
        let child_slot_worker = Arc::clone(&child_slot);

        let (turn_tx, turn_rx) = mpsc::sync_channel::<WorkerMsg>(1);
        let (event_tx, event_rx) = mpsc::sync_channel::<WorkerEvent>(64);

        thread::Builder::new()
            .name(format!("agent-pane-{id}"))
            .spawn(move || {
                while let Ok(msg) = turn_rx.recv() {
                    log::info!("agent_pane {id}: turn start, session={:?}", msg.session_id);
                    if let Err(e) = agent_turn::run_turn(
                        &msg.session_id,
                        &msg.message,
                        &msg.cwd,
                        msg.soul_context,
                        event_tx.clone(),
                        Arc::clone(&child_slot_worker),
                    ) {
                        log::error!("agent_pane {id}: run_turn error: {e}");
                    }
                }
            })
            .unwrap_or_else(|e| panic!("failed to spawn agent worker: {e}"));

        let agent = Self {
            session_id,
            cwd,
            iq_dir,
            pending_message: None,
            streaming_active: false,
            interrupting: false,
            child_slot,
            turn_tx: Some(turn_tx),
            event_rx,
        };
        (agent, transcript)
    }

    /// Kill the current subprocess. Done(Err) will arrive; `interrupting` suppresses
    /// the error display. `pending_message` auto-dispatches on Done.
    fn cancel_current_turn(&mut self) {
        self.interrupting = true;
        if let Ok(mut slot) = self.child_slot.lock() {
            if let Some(ref mut child) = *slot {
                let _ = child.kill();
            }
        }
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

/// Discriminates which backend produces messages on the conversation surface.
/// Both backends share the transcript/input rendering on `AgentPane`. Adding a
/// backend means one match arm in `submit_input` and `drain_results`.
pub enum AgentBackend {
    /// Legacy in-process turn loop (Cmd+I path). Calls `claude -p` directly.
    InProcess(InProcessAgent),
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
    /// Construct an in-process (Cmd+I) agent pane. Spawns the worker thread
    /// and loads any persisted session for this `id`.
    pub fn new(id: PaneId, cwd: PathBuf) -> Self {
        let (agent, transcript) = InProcessAgent::new(id, cwd);
        Self {
            id,
            transcript,
            input_buf: String::new(),
            in_flight: false,
            font_size: 13.0,
            needs_focus: true,
            backend: AgentBackend::InProcess(agent),
        }
    }

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

    /// Workspace `cwd` for this pane. In-process backend tracks the workspace
    /// dir explicitly (it loads SOUL/MEMORY relative to it); subprocess
    /// backend uses the `ProcessApp`'s `workspace_root`. Used by workspace
    /// persistence to round-trip the pane.
    pub fn cwd(&self) -> PathBuf {
        match &self.backend {
            AgentBackend::InProcess(a) => a.cwd.clone(),
            AgentBackend::Subprocess(a) => a.process.workspace_root.clone(),
        }
    }

    /// `cwd` is only meaningful for the in-process backend (it points at the
    /// workspace whose `.plexi/agents/iq/` we persist sessions into). Returned
    /// here for the header's workspace label; subprocess panes return `None`.
    pub fn iq_dir(&self) -> Option<&Path> {
        match &self.backend {
            AgentBackend::InProcess(a) => a.iq_dir.as_deref(),
            AgentBackend::Subprocess(_) => None,
        }
    }

    pub fn pending_message_present(&self) -> bool {
        match &self.backend {
            AgentBackend::InProcess(a) => a.pending_message.is_some(),
            AgentBackend::Subprocess(_) => false,
        }
    }

    fn dispatch_in_process(&mut self, message: String) {
        log::info!("agent_pane {}: submit {:?}", self.id, message);

        let display = format_user_message(&message);
        self.transcript.push(display);

        let AgentBackend::InProcess(agent) = &mut self.backend else {
            return;
        };

        let soul_context = if agent.session_id.is_empty() {
            agent.iq_dir.as_deref().map(load_soul_context)
        } else {
            None
        };

        let msg = WorkerMsg {
            session_id: agent.session_id.clone(),
            soul_context,
            message,
            cwd: agent.cwd.clone(),
        };
        if let Some(tx) = &agent.turn_tx {
            match tx.try_send(msg) {
                Ok(_) => {
                    self.in_flight = true;
                    agent.streaming_active = false;
                }
                Err(e) => {
                    log::error!("agent_pane {}: channel send failed: {e}", self.id);
                    self.transcript.push("Error: failed to dispatch turn".into());
                }
            }
        }
    }

    fn dispatch_subprocess(&mut self, message: String) {
        log::info!("agent_pane {}: submit (subprocess) {:?}", self.id, message);
        self.transcript.push(format_user_message(&message));
        let AgentBackend::Subprocess(agent) = &mut self.backend else {
            return;
        };
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

        match &mut self.backend {
            AgentBackend::InProcess(agent) => {
                if self.in_flight {
                    // Interrupt current turn, queue new message — dispatches when Done arrives.
                    agent.cancel_current_turn();
                    agent.pending_message = Some(message);
                } else {
                    self.dispatch_in_process(message);
                }
            }
            AgentBackend::Subprocess(_) => {
                // Subprocess agents don't support interruption today — the
                // agent owns the turn boundary. Submitting while in_flight
                // simply queues another UserMessage; the agent decides how
                // to handle concurrent turns. Kept simple deliberately;
                // streaming + interruption are tracked under v3.3.5+.
                self.dispatch_subprocess(message);
            }
        }
    }

    /// Drain backend events into the transcript. Returns true if caller should
    /// request a repaint (events arrived OR a turn is still running).
    pub fn drain_results(&mut self) -> bool {
        match &mut self.backend {
            AgentBackend::InProcess(_) => self.drain_in_process(),
            AgentBackend::Subprocess(_) => self.drain_subprocess(),
        }
    }

    fn drain_in_process(&mut self) -> bool {
        let mut changed = false;
        // Captured outside the loop so we can release the `&mut self.backend`
        // borrow before calling `dispatch_in_process` (which needs `&mut self`).
        let mut to_dispatch: Option<String> = None;
        let AgentBackend::InProcess(agent) = &mut self.backend else {
            return false;
        };

        while let Ok(event) = agent.event_rx.try_recv() {
            changed = true;
            match event {
                WorkerEvent::Chunk(text) => {
                    if agent.streaming_active {
                        if let Some(last) = self.transcript.last_mut() {
                            *last = format!("Agent: {text}");
                        }
                    } else {
                        self.transcript.push(format!("Agent: {text}"));
                        agent.streaming_active = true;
                    }
                }
                WorkerEvent::ToolUse { name, input_preview } => {
                    let label = if input_preview.is_empty() {
                        format!("  ↳ {name}")
                    } else {
                        format!("  ↳ {name}: {input_preview}")
                    };
                    // Insert before the streaming response so tools stay above reply text.
                    if agent.streaming_active {
                        let idx = self.transcript.len().saturating_sub(1);
                        self.transcript.insert(idx, label);
                    } else {
                        self.transcript.push(label);
                    }
                }
                WorkerEvent::Done(result) => {
                    self.in_flight = false;
                    agent.streaming_active = false;
                    self.needs_focus = true;

                    let was_interrupting = agent.interrupting;
                    agent.interrupting = false;

                    match result {
                        Ok(turn) => {
                            log::info!(
                                "agent_pane {}: turn ok, session={:?}",
                                self.id, turn.session_id
                            );
                            if !turn.session_id.is_empty() && agent.session_id != turn.session_id {
                                agent.session_id = turn.session_id.clone();
                            }
                            // Push response text if no chunks arrived (e.g. very short reply).
                            if !self.transcript.last().map(|l| l.starts_with("Agent:")).unwrap_or(false) {
                                if !turn.response.is_empty() {
                                    self.transcript.push(format!("Agent: {}", turn.response));
                                }
                            }
                            if let Some(ref dir) = agent.iq_dir {
                                save_session_file(dir, self.id, &SessionFile {
                                    session_id: agent.session_id.clone(),
                                    transcript: self.transcript.clone(),
                                });
                            }
                        }
                        Err(e) => {
                            if was_interrupting {
                                log::info!("agent_pane {}: turn interrupted", self.id);
                            } else {
                                log::warn!("agent_pane {}: turn error: {e}", self.id);
                                self.transcript.push(format!("Error: {e}"));
                            }
                        }
                    }

                    // Auto-send queued message (from pending or interrupt).
                    // Stage outside the loop so we can drop the borrow on
                    // `agent` before calling `dispatch_in_process` (which
                    // re-borrows `self`).
                    if let Some(pending) = agent.pending_message.take() {
                        to_dispatch = Some(pending);
                    }
                }
            }
        }

        // Borrow on `agent` released — safe to dispatch.
        if let Some(message) = to_dispatch {
            self.dispatch_in_process(message);
        }

        changed || self.in_flight
    }

    fn drain_subprocess(&mut self) -> bool {
        let AgentBackend::Subprocess(agent) = &mut self.backend else {
            return false;
        };

        // 1. Forward AgentInit lazily once the subprocess is Ready.
        agent.maybe_send_init();

        // 2. Pump I/O + collect AppendConversation rows.
        let new_rows = agent.process.agent_tick();
        let changed = !new_rows.is_empty();
        for (role, content) in new_rows {
            // The agent finished a turn (or part of one) — clear in-flight on
            // any assistant row. Tool / system rows don't toggle the flag.
            if role == "assistant" {
                agent.in_flight = false;
                self.in_flight = false;
                self.needs_focus = true;
            }
            self.transcript.push(format_conversation_row(&role, &content));
        }

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
                ui.label(egui::RichText::new("IQ").size(11.0).color(accent));
                if let Some(dir) = pane.iq_dir() {
                    if let Some(ws) = dir.parent().and_then(|p| p.parent()).and_then(|p| p.file_name()) {
                        ui.label(
                            egui::RichText::new(format!("  {}", ws.to_string_lossy()))
                                .size(10.0)
                                .color(dim_color),
                        );
                    }
                } else if let AgentBackend::Subprocess(agent) = &pane.backend {
                    // Subprocess agents have no .plexi/ session dir — show the
                    // manifest id instead so the user knows which agent this is.
                    ui.label(
                        egui::RichText::new(format!("  {}", agent.manifest_id))
                            .size(10.0)
                            .color(dim_color),
                    );
                }
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
                    if pane.pending_message_present() {
                        ui.label(
                            egui::RichText::new("· next queued")
                                .size(10.0)
                                .color(dim_color),
                        );
                    }
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

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    //! Behavioural tests for the AgentBackend enum (issue #338, part 2 of #285).
    //!
    //! In-process backend regression: existing Cmd+I flow constructs an
    //! `AgentPane` with `AgentBackend::InProcess` and never panics during
    //! drain on a quiet event_rx.
    //!
    //! Subprocess backend (the new path):
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
        if let AgentBackend::Subprocess(agent) = &mut pane.backend {
            agent.process.test_inject_draw_command(DrawCommand::AppendConversation {
                role: "assistant".to_string(),
                content: "Hello!".to_string(),
            });
        } else {
            panic!("expected Subprocess backend");
        }

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
        let AgentBackend::Subprocess(agent) = &pane.backend else {
            panic!("expected Subprocess backend");
        };
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

    #[test]
    fn in_process_backend_unchanged_path_still_works() {
        // Regression guard: the legacy Cmd+I flow constructs an AgentPane
        // via `AgentPane::new(id, cwd)` and the backend must be the
        // `InProcess` variant. `drain_results` must be safe to call on a
        // freshly constructed pane that has done nothing yet (empty event_rx).
        let pane = AgentPane::new(99, std::env::temp_dir());
        assert!(matches!(pane.backend, AgentBackend::InProcess(_)));
        // empty drain — just confirm we don't panic
        let mut pane = pane;
        let _ = pane.drain_results();
        assert!(!pane.in_flight, "no turn submitted → no in_flight");
        assert!(pane.transcript.is_empty() || pane.transcript.iter().all(|l| !l.is_empty()));
    }
}
