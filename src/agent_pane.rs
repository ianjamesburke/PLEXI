/// Agent pane: a first-class Plexi pane that chats with `claude -p --resume`.
///
/// Each `AgentPane` owns a background thread that drives `claude` as a
/// subprocess. The UI thread sends messages in and receives responses out
/// via synchronous mpsc channels, so the egui render loop is never blocked.
use crate::agent_turn;
use crate::theme::Colors;
use crate::tiling::PaneId;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;

// ── Session persistence ──────────────────────────────────────────────────────

/// Path to the per-build sessions file.
fn sessions_path() -> PathBuf {
    crate::config::config_dir().join("sessions.toml")
}

/// Load all agent session IDs from disk.
/// File format:
/// ```toml
/// [agents]
/// "42" = "claude-session-abc123"
/// ```
fn load_sessions() -> HashMap<String, String> {
    let path = sessions_path();
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => return HashMap::new(),
    };
    let table: toml::Value = match toml::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            log::warn!("agent_pane: failed to parse sessions.toml: {e}");
            return HashMap::new();
        }
    };
    let mut out = HashMap::new();
    if let Some(agents) = table.get("agents").and_then(|v| v.as_table()) {
        for (k, v) in agents {
            if let Some(s) = v.as_str() {
                out.insert(k.clone(), s.to_string());
            }
        }
    }
    out
}

/// Persist a single agent's session ID to disk.
fn save_session(pane_id: PaneId, session_id: &str) {
    let mut sessions = load_sessions();
    sessions.insert(pane_id.to_string(), session_id.to_string());

    let mut body = String::from("[agents]\n");
    for (k, v) in &sessions {
        body.push_str(&format!("{:?} = {:?}\n", k, v));
    }

    let path = sessions_path();
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            log::error!("agent_pane: failed to create sessions dir: {e}");
            return;
        }
    }
    if let Err(e) = std::fs::write(&path, body) {
        log::error!("agent_pane: failed to write sessions.toml: {e}");
    }
}

// ── AgentPane ────────────────────────────────────────────────────────────────

/// Message sent to the background worker thread.
struct WorkerMsg {
    session_id: String,
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
    /// True until the input bar has received its initial auto-focus on first render.
    needs_focus: bool,

    // Channel pair — kept on the pane so the background thread stays alive.
    turn_tx: Option<mpsc::SyncSender<WorkerMsg>>,
    pub result_rx: mpsc::Receiver<WorkerResult>,
}

impl AgentPane {
    /// Create a new `AgentPane`, spawning its background worker thread.
    ///
    /// Looks up a persisted session ID for `id` in `sessions.toml` and
    /// restores it if found.
    pub fn new(id: PaneId, cwd: PathBuf) -> Self {
        // Restore session ID if one was persisted for this pane.
        let sessions = load_sessions();
        let session_id = sessions.get(&id.to_string()).cloned().unwrap_or_default();

        // Synchronous channel: worker blocks until the UI drains results,
        // which is fine — we only ever have one turn in flight at a time.
        let (turn_tx, turn_rx) = mpsc::sync_channel::<WorkerMsg>(1);
        let (result_tx, result_rx) = mpsc::sync_channel::<WorkerResult>(8);

        thread::Builder::new()
            .name(format!("agent-pane-{id}"))
            .spawn(move || {
                while let Ok(msg) = turn_rx.recv() {
                    log::info!("agent_pane {id}: running turn, session={:?}", msg.session_id);
                    let result = agent_turn::run_turn(&msg.session_id, &msg.message, &msg.cwd);
                    match &result {
                        Ok(t) => log::info!("agent_pane {id}: turn ok, session={:?}, response={:?}", t.session_id, t.response),
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
            transcript: Vec::new(),
            input_buf: String::new(),
            in_flight: false,
            cwd,
            needs_focus: true,
            turn_tx: Some(turn_tx),
            result_rx,
        }
    }

    /// Submit the current `input_buf` as a user turn.
    ///
    /// Pushes the message to `transcript`, sends it to the background thread,
    /// and sets `in_flight = true`. No-op if a turn is already in flight or
    /// the input is empty.
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

        let msg = WorkerMsg {
            session_id: self.session_id.clone(),
            message,
            cwd: self.cwd.clone(),
        };
        if let Some(tx) = &self.turn_tx {
            match tx.try_send(msg) {
                Ok(_) => {
                    self.in_flight = true;
                }
                Err(e) => {
                    log::error!("agent_pane {}: failed to send to worker: {e}", self.id);
                    self.transcript
                        .push("Error: failed to dispatch turn to worker thread".into());
                }
            }
        }
    }

    /// Drain completed turn results from the background thread.
    ///
    /// Returns `true` if any results were received (caller should request a
    /// repaint so the transcript updates are visible immediately).
    pub fn drain_results(&mut self) -> bool {
        let mut got_any = false;
        while let Ok(result) = self.result_rx.try_recv() {
            got_any = true;
            self.in_flight = false;
            self.needs_focus = true; // re-focus input after response arrives
            match result.response {
                Ok(turn) => {
                    // Persist the (possibly new) session ID.
                    if self.session_id != turn.session_id && !turn.session_id.is_empty() {
                        self.session_id = turn.session_id.clone();
                        save_session(self.id, &self.session_id);
                    }
                    self.transcript.push(format!("Agent: {}", turn.response));
                }
                Err(e) => {
                    self.transcript.push(format!("Error: {e}"));
                }
            }
        }
        // Keep requesting repaints while a turn is in flight so the result
        // surfaces as soon as the background thread delivers it, without
        // requiring a mouse/keyboard event to trigger a frame.
        got_any || self.in_flight
    }
}

// ── Render ───────────────────────────────────────────────────────────────────

/// Render the agent pane UI into `ui`.
///
/// Called from `PlexiBehavior::pane_ui` when the pane ID maps to an `AgentPane`.
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
                ui.label(
                    egui::RichText::new("Agent")
                        .size(11.0)
                        .color(accent),
                );
                if !pane.session_id.is_empty() {
                    ui.label(
                        egui::RichText::new(format!("  session: {}", &pane.session_id[..pane.session_id.len().min(12)]))
                            .size(10.0)
                            .color(dim_color),
                    );
                }
                if pane.in_flight {
                    ui.label(
                        egui::RichText::new(" ●")
                            .size(10.0)
                            .color(accent),
                    );
                }
            });

            ui.add_space(4.0);

            // Transcript scroll area
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
                            egui::Color32::from_rgb(0xf3, 0x8b, 0xa8) // soft red
                        } else {
                            dim_color
                        };
                        ui.label(egui::RichText::new(line).size(13.0).color(color));
                        ui.add_space(2.0);
                    }
                    if pane.in_flight {
                        ui.label(
                            egui::RichText::new("…thinking")
                                .size(12.0)
                                .italics()
                                .color(dim_color),
                        );
                    }
                });

            ui.add_space(4.0);
            ui.separator();
            ui.add_space(4.0);

            // Input bar
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(">").size(12.0).color(accent));
                let input = egui::TextEdit::singleline(&mut pane.input_buf)
                    .desired_width(ui.available_width() - 60.0)
                    .hint_text("Ask Claude…")
                    .font(egui::TextStyle::Monospace)
                    .text_color(text_color)
                    .frame(false);
                let resp = ui.add(input);

                // Auto-focus the input bar on first render so typing works immediately.
                if pane.needs_focus {
                    resp.request_focus();
                    pane.needs_focus = false;
                }

                let enter_pressed = resp.lost_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter));
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

/// Render the agent pane, also draining any pending results.
///
/// Returns true if a repaint was needed (caller should call
/// `ui.ctx().request_repaint()`).
pub fn render_and_drain(ui: &mut egui::Ui, pane: &mut AgentPane, colors: &Colors) -> bool {
    let needs_repaint = pane.drain_results();
    render(ui, pane, colors);
    needs_repaint
}
