/// Agent pane: chats with `claude -p` using streaming JSON for real-time output.
///
/// Streaming lets the UI show text as it arrives and surface tool use in-line.
/// A pending_message queue lets the user type and submit the next message while
/// a turn is still in flight — it auto-sends when the current turn completes.
use crate::agent_turn::{self, WorkerEvent};
use crate::theme::Colors;
use crate::tiling::PaneId;
use std::path::{Path, PathBuf};
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

// ── AgentPane ────────────────────────────────────────────────────────────────

struct WorkerMsg {
    session_id: String,
    soul_context: Option<String>,
    message: String,
    cwd: PathBuf,
}

pub struct AgentPane {
    pub id: PaneId,
    pub session_id: String,
    pub transcript: Vec<String>,
    pub input_buf: String,
    pub in_flight: bool,
    pub cwd: PathBuf,
    iq_dir: Option<PathBuf>,
    pub font_size: f32,
    needs_focus: bool,
    /// Message queued while a turn is in flight — auto-sends when turn completes.
    pending_message: Option<String>,
    /// True when the last transcript line is a streaming in-progress agent line.
    streaming_active: bool,

    turn_tx: Option<mpsc::SyncSender<WorkerMsg>>,
    event_rx: mpsc::Receiver<WorkerEvent>,
}

impl AgentPane {
    pub fn new(id: PaneId, cwd: PathBuf) -> Self {
        let iq_dir = find_iq_dir(&cwd);

        let (session_id, transcript) = if let Some(ref dir) = iq_dir {
            let sf = load_session(dir, id);
            (sf.session_id, sf.transcript)
        } else {
            (String::new(), Vec::new())
        };

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
                    ) {
                        log::error!("agent_pane {id}: run_turn error: {e}");
                    }
                }
            })
            .unwrap_or_else(|e| panic!("failed to spawn agent worker: {e}"));

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
            pending_message: None,
            streaming_active: false,
            turn_tx: Some(turn_tx),
            event_rx,
        }
    }

    fn dispatch(&mut self, message: String) {
        log::info!("agent_pane {}: submit {:?}", self.id, message);
        self.transcript.push(format!("You: {message}"));

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
                Ok(_) => {
                    self.in_flight = true;
                    self.streaming_active = false;
                }
                Err(e) => {
                    log::error!("agent_pane {}: channel send failed: {e}", self.id);
                    self.transcript.push("Error: failed to dispatch turn".into());
                }
            }
        }
    }

    pub fn submit_input(&mut self) {
        let message = self.input_buf.trim().to_string();
        if message.is_empty() {
            return;
        }
        self.input_buf.clear();

        if self.in_flight {
            // Queue it — will auto-send when current turn completes.
            self.pending_message = Some(message);
        } else {
            self.dispatch(message);
        }
    }

    /// Drain streaming events from the background thread.
    /// Returns true if the caller should request a repaint.
    pub fn drain_results(&mut self) -> bool {
        let mut changed = false;

        while let Ok(event) = self.event_rx.try_recv() {
            changed = true;
            match event {
                WorkerEvent::Chunk(text) => {
                    if self.streaming_active {
                        // Update the last line in place.
                        if let Some(last) = self.transcript.last_mut() {
                            *last = format!("Agent: {text}");
                        }
                    } else {
                        self.transcript.push(format!("Agent: {text}"));
                        self.streaming_active = true;
                    }
                }
                WorkerEvent::ToolUse { name, input_preview } => {
                    let label = if input_preview.is_empty() {
                        format!("  ↳ {name}")
                    } else {
                        format!("  ↳ {name}: {input_preview}")
                    };
                    // Insert tool-use line before the streaming agent line so it
                    // stays visually above the response text.
                    if self.streaming_active {
                        let idx = self.transcript.len().saturating_sub(1);
                        self.transcript.insert(idx, label);
                    } else {
                        self.transcript.push(label);
                    }
                }
                WorkerEvent::Done(result) => {
                    self.in_flight = false;
                    self.streaming_active = false;
                    self.needs_focus = true;

                    match result {
                        Ok(turn) => {
                            if !turn.session_id.is_empty() && self.session_id != turn.session_id {
                                self.session_id = turn.session_id.clone();
                            }
                            // If no chunks arrived (e.g. error path), push the result text.
                            if !self.transcript.last().map(|l| l.starts_with("Agent:")).unwrap_or(false) {
                                if !turn.response.is_empty() {
                                    self.transcript.push(format!("Agent: {}", turn.response));
                                }
                            }
                            if let Some(ref dir) = self.iq_dir {
                                save_session_file(dir, self.id, &SessionFile {
                                    session_id: self.session_id.clone(),
                                    transcript: self.transcript.clone(),
                                });
                            }
                        }
                        Err(e) => {
                            self.transcript.push(format!("Error: {e}"));
                        }
                    }

                    // Auto-send queued message.
                    if let Some(pending) = self.pending_message.take() {
                        self.dispatch(pending);
                    }
                }
            }
        }

        changed || self.in_flight
    }
}

// ── Render ───────────────────────────────────────────────────────────────────

pub fn render(ui: &mut egui::Ui, pane: &mut AgentPane, colors: &Colors) {
    let bg = colors.terminal_bg;
    let text_color = colors.text_primary;
    let dim_color = colors.text_dim;
    let accent = colors.accent;
    let tool_color = egui::Color32::from_rgb(0x89, 0xb4, 0xfa); // blue

    egui::Frame::new()
        .fill(bg)
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            // Header
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("IQ").size(11.0).color(accent));
                if let Some(ref dir) = pane.iq_dir {
                    if let Some(ws) = dir.parent().and_then(|p| p.parent()).and_then(|p| p.file_name()) {
                        ui.label(
                            egui::RichText::new(format!("  {}", ws.to_string_lossy()))
                                .size(10.0)
                                .color(dim_color),
                        );
                    }
                }
                if pane.in_flight {
                    let dots = match (ui.input(|i| i.time) * 1.5) as usize % 4 {
                        0 => "   ", 1 => ".  ", 2 => ".. ", _ => "...",
                    };
                    ui.label(
                        egui::RichText::new(format!("  working{dots}"))
                            .size(10.0)
                            .italics()
                            .color(dim_color),
                    );
                }
                if pane.pending_message.is_some() {
                    ui.label(egui::RichText::new("  [queued]").size(10.0).color(dim_color));
                }
            });

            ui.add_space(4.0);

            let input_height = 48.0;
            let available = ui.available_height() - input_height - 8.0;
            egui::ScrollArea::vertical()
                .max_height(available.max(40.0))
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for line in &pane.transcript {
                        let (color, size) = if line.starts_with("You: ") {
                            (text_color, pane.font_size)
                        } else if line.starts_with("Error:") {
                            (egui::Color32::from_rgb(0xf3, 0x8b, 0xa8), pane.font_size)
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

            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(">").size(12.0).color(accent));
                let input = egui::TextEdit::singleline(&mut pane.input_buf)
                    .desired_width(ui.available_width() - 50.0)
                    .hint_text("Message…")
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
                let send_label = if pane.in_flight { "Queue" } else { "Send" };
                let button_clicked = ui
                    .add_enabled(
                        !pane.input_buf.trim().is_empty(),
                        egui::Button::new(
                            egui::RichText::new(send_label).size(11.0).color(accent),
                        ),
                    )
                    .clicked();

                if enter_pressed || button_clicked {
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
