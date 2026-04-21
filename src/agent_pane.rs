/// Agent pane: a first-class Plexi pane that chats with `claude -p --resume`.
///
/// Each `AgentPane` owns a background thread that drives `claude` as a
/// subprocess. The UI thread sends messages in and receives responses out
/// via synchronous mpsc channels, so the egui render loop is never blocked.
///
/// Soul and memory files live in the nearest `.plexi/agents/iq/` directory
/// (found by walking up from cwd). Sessions are stored as per-pane JSON files
/// in `.plexi/agents/iq/sessions/<pane-id>.json` so transcripts survive restarts.
use crate::agent_turn;
use crate::theme::Colors;
use crate::tiling::PaneId;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;

// ── Workspace resolution ─────────────────────────────────────────────────────

/// Walk up from `start` to find the nearest `.plexi/` directory.
/// Returns the `.plexi/agents/iq/` path if a workspace is found.
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

/// Load SOUL.md and MEMORY.md from the iq dir, creating defaults if absent.
/// Returns the combined context string to prepend to the first message.
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
        if let Err(e) = std::fs::write(&soul_path, DEFAULT_SOUL) {
            log::warn!("agent_pane: could not write default SOUL.md: {e}");
        }
        DEFAULT_SOUL.to_string()
    };

    let memory = if memory_path.exists() {
        std::fs::read_to_string(&memory_path).unwrap_or_else(|_| DEFAULT_MEMORY.to_string())
    } else {
        if let Err(e) = std::fs::write(&memory_path, DEFAULT_MEMORY) {
            log::warn!("agent_pane: could not write default MEMORY.md: {e}");
        }
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
    serde_json::from_str(&text).unwrap_or_else(|e| {
        log::warn!("agent_pane: failed to parse session {pane_id}: {e}");
        SessionFile::default()
    })
}

fn save_session_file(iq_dir: &Path, pane_id: PaneId, file: &SessionFile) {
    let dir = iq_dir.join("sessions");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        log::error!("agent_pane: failed to create sessions dir: {e}");
        return;
    }
    let path = session_path(iq_dir, pane_id);
    match serde_json::to_string_pretty(file) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                log::error!("agent_pane: failed to write session {pane_id}: {e}");
            }
        }
        Err(e) => log::error!("agent_pane: failed to serialize session {pane_id}: {e}"),
    }
}

// ── AgentPane ────────────────────────────────────────────────────────────────

/// Message sent to the background worker thread.
struct WorkerMsg {
    session_id: String,
    /// Soul + memory context prepended to the first message of a new session.
    soul_context: Option<String>,
    message: String,
    cwd: PathBuf,
}

/// Response received from the background worker thread.
pub struct WorkerResult {
    pub response: Result<agent_turn::TurnResult, String>,
}

/// A pane that provides an in-process chat interface backed by `claude -p`.
pub struct AgentPane {
    pub id: PaneId,
    /// Current Claude session ID. Empty until the first turn completes.
    pub session_id: String,
    /// Rendered conversation lines, alternating user and assistant turns.
    pub transcript: Vec<String>,
    /// Text currently being composed in the input bar.
    pub input_buf: String,
    /// True while a turn is in flight on the background thread.
    pub in_flight: bool,
    /// Working directory scoped to this pane (used for the claude subprocess).
    pub cwd: PathBuf,
    /// Resolved `.plexi/agents/iq/` dir for this pane's workspace, if found.
    iq_dir: Option<PathBuf>,
    /// Font size — adjustable with Cmd+= / Cmd+-.
    pub font_size: f32,
    /// True until the input bar has received its initial auto-focus on first render.
    needs_focus: bool,

    // Channel pair — kept on the pane so the background thread stays alive.
    turn_tx: Option<mpsc::SyncSender<WorkerMsg>>,
    pub result_rx: mpsc::Receiver<WorkerResult>,
}

impl AgentPane {
    /// Create a new `AgentPane`, spawning its background worker thread.
    /// Restores session ID and transcript from the workspace session file if found.
    pub fn new(id: PaneId, cwd: PathBuf) -> Self {
        let iq_dir = find_iq_dir(&cwd);

        let (session_id, transcript) = if let Some(ref dir) = iq_dir {
            let sf = load_session(dir, id);
            (sf.session_id, sf.transcript)
        } else {
            (String::new(), Vec::new())
        };

        let (turn_tx, turn_rx) = mpsc::sync_channel::<WorkerMsg>(1);
        let (result_tx, result_rx) = mpsc::sync_channel::<WorkerResult>(8);

        thread::Builder::new()
            .name(format!("agent-pane-{id}"))
            .spawn(move || {
                while let Ok(msg) = turn_rx.recv() {
                    log::info!("agent_pane {id}: running turn, session={:?}", msg.session_id);
                    let result = agent_turn::run_turn(
                        &msg.session_id,
                        &msg.message,
                        &msg.cwd,
                        msg.soul_context,
                    );
                    match &result {
                        Ok(t) => log::info!(
                            "agent_pane {id}: turn ok, session={:?}, response={:?}",
                            t.session_id,
                            t.response
                        ),
                        Err(e) => log::error!("agent_pane {id}: turn error: {e}"),
                    }
                    if result_tx.send(WorkerResult { response: result }).is_err() {
                        break;
                    }
                }
            })
            .unwrap_or_else(|e| panic!("failed to spawn agent worker thread: {e}"));

        Self {
            id,
            session_id,
            transcript,
            input_buf: String::new(),
            in_flight: false,
            cwd,
            iq_dir,
            font_size: 13.0,
            needs_focus: true,
            turn_tx: Some(turn_tx),
            result_rx,
        }
    }

    /// Submit the current `input_buf` as a user turn.
    pub fn submit_input(&mut self) {
        if self.in_flight {
            return;
        }
        let message = self.input_buf.trim().to_string();
        if message.is_empty() {
            return;
        }
        log::info!("agent_pane {}: submit: {:?}", self.id, message);
        self.input_buf.clear();
        self.transcript.push(format!("You: {message}"));

        // On the first turn of a new session, load and prepend soul context.
        let soul_context = if self.session_id.is_empty() {
            self.iq_dir.as_deref().map(load_soul_context)
        } else {
            None
        };

        let msg = WorkerMsg {
            session_id: self.session_id.clone(),
            soul_context,
            message,
            cwd: self.cwd.clone(),
        };
        if let Some(tx) = &self.turn_tx {
            match tx.try_send(msg) {
                Ok(_) => self.in_flight = true,
                Err(e) => {
                    log::error!("agent_pane {}: failed to send to worker: {e}", self.id);
                    self.transcript
                        .push("Error: failed to dispatch turn to worker thread".into());
                }
            }
        }
    }

    /// Drain completed turn results from the background thread.
    /// Returns `true` if the caller should request a repaint.
    pub fn drain_results(&mut self) -> bool {
        let mut got_any = false;
        while let Ok(result) = self.result_rx.try_recv() {
            got_any = true;
            self.in_flight = false;
            self.needs_focus = true;
            match result.response {
                Ok(turn) => {
                    if !turn.session_id.is_empty() && self.session_id != turn.session_id {
                        self.session_id = turn.session_id.clone();
                    }
                    self.transcript.push(format!("Agent: {}", turn.response));
                    // Persist session ID + full transcript to workspace.
                    if let Some(ref dir) = self.iq_dir {
                        save_session_file(
                            dir,
                            self.id,
                            &SessionFile {
                                session_id: self.session_id.clone(),
                                transcript: self.transcript.clone(),
                            },
                        );
                    }
                }
                Err(e) => {
                    self.transcript.push(format!("Error: {e}"));
                }
            }
        }
        got_any || self.in_flight
    }
}

// ── Render ───────────────────────────────────────────────────────────────────

pub fn render(ui: &mut egui::Ui, pane: &mut AgentPane, colors: &Colors) {
    let bg = colors.terminal_bg;
    let text_color = colors.text_primary;
    let dim_color = colors.text_dim;
    let accent = colors.accent;

    egui::Frame::new()
        .fill(bg)
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            // Header
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("IQ").size(11.0).color(accent));
                if let Some(ref dir) = pane.iq_dir {
                    if let Some(workspace) = dir.parent().and_then(|p| p.parent()).and_then(|p| p.file_name()) {
                        ui.label(
                            egui::RichText::new(format!("  {}", workspace.to_string_lossy()))
                                .size(10.0)
                                .color(dim_color),
                        );
                    }
                }
                if pane.in_flight {
                    ui.label(egui::RichText::new(" ●").size(10.0).color(accent));
                }
            });

            ui.add_space(4.0);

            let input_height = 60.0;
            let available = ui.available_height() - input_height - 8.0;
            egui::ScrollArea::vertical()
                .max_height(available.max(40.0))
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for line in &pane.transcript {
                        let color = if line.starts_with("You: ") {
                            text_color
                        } else if line.starts_with("Error:") {
                            egui::Color32::from_rgb(0xf3, 0x8b, 0xa8)
                        } else {
                            dim_color
                        };
                        ui.label(egui::RichText::new(line).size(pane.font_size).color(color));
                        ui.add_space(2.0);
                    }
                    if pane.in_flight {
                        ui.label(
                            egui::RichText::new("working…")
                                .size(pane.font_size - 1.0)
                                .italics()
                                .color(dim_color),
                        );
                    }
                });

            ui.add_space(4.0);
            ui.separator();
            ui.add_space(4.0);

            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(">").size(12.0).color(accent));
                let input = egui::TextEdit::singleline(&mut pane.input_buf)
                    .desired_width(ui.available_width() - 60.0)
                    .hint_text("Ask Claude…")
                    .font(egui::FontId::monospace(pane.font_size))
                    .text_color(text_color)
                    .frame(false);
                let resp = ui.add(input);

                if pane.needs_focus {
                    resp.request_focus();
                    pane.needs_focus = false;
                }

                let enter_pressed =
                    resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let button_clicked = ui
                    .add_enabled(
                        !pane.in_flight && !pane.input_buf.trim().is_empty(),
                        egui::Button::new(
                            egui::RichText::new("Send").size(11.0).color(accent),
                        ),
                    )
                    .clicked();

                if (enter_pressed || button_clicked) && !pane.in_flight {
                    pane.submit_input();
                }
            });
        });
}

pub fn render_and_drain(ui: &mut egui::Ui, pane: &mut AgentPane, colors: &Colors) -> bool {
    let needs_repaint = pane.drain_results();
    render(ui, pane, colors);
    needs_repaint
}
