use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::SystemTime;

use crate::agent_ansi::markdown_to_ansi;
use crate::agent_llm::{LlmRequest, LlmResponse, LlmWorker};

/// Cyan foreground for the `>>> ` prompt indicator.
const PROMPT_ANSI: &str = "\r\n\x1b[36m>>> \x1b[0m";
/// Dim italics for the "thinking..." indicator.
const THINKING_ANSI: &str = "\r\n\x1b[2;3magent thinking...\x1b[0m\r\n";
/// Bright red prefix for error lines.
const ERROR_ANSI_PREFIX: &str = "\r\n\x1b[31m! ";
const ERROR_ANSI_SUFFIX: &str = "\x1b[0m\r\n";
/// Dim gray prefix for system notices.
const SYSTEM_ANSI_PREFIX: &str = "\r\n\x1b[2m# ";
const SYSTEM_ANSI_SUFFIX: &str = "\x1b[0m\r\n";
/// Dim gray line emitted when the user aborts an in-flight LLM request with
/// Ctrl+C. Written inline into the terminal grid so it becomes part of
/// scrollback, same as every other agent-mode line.
const INTERRUPTED_ANSI: &str = "\r\n\x1b[2;90m^C — interrupted\x1b[0m\r\n";

/// State machine for agent mode within a terminal pane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentModeState {
    /// Agent mode is not active — normal shell mode.
    Inactive,
    /// User is typing their prompt to the agent.
    WaitingForInput,
    /// Agent is processing (LLM call in progress).
    Processing,
}

/// Per-pane agent mode state. Each terminal pane has its own AgentMode instance.
///
/// Output flow: state mutations append ANSI byte chunks to `pending_output`.
/// The tiling layer drains that queue each frame and writes it to the pane's
/// `TerminalBackend` via `write_agent_bytes`, so agent turns render into the
/// terminal's normal scrollback grid (Warp-style) rather than a separate panel.
pub struct AgentMode {
    pub state: AgentModeState,
    pub input_buffer: String,
    pub directory_scope: PathBuf,
    /// Conversation transcript — kept so future multi-turn context can send
    /// prior messages to the LLM. Not rendered directly (the terminal grid is
    /// the source of truth for what the user sees).
    #[allow(dead_code)]
    pub conversation: Vec<AgentMessage>,
    /// ANSI bytes to be written into the pane's terminal grid on the next
    /// frame. The render layer drains this and forwards it to the backend.
    pending_output: VecDeque<Vec<u8>>,
    /// Lazily-initialized LLM worker. `None` until activate() runs once
    /// successfully; reused across turns for the session lifetime.
    llm: Option<LlmWorker>,
    /// Session ID returned by `claude -p` on the first turn.
    /// Passed as `--resume <id>` on all subsequent turns to maintain context.
    session_id: Option<String>,
    /// Buffer accumulating streamed token chunks for the current turn.
    /// Flushed to `conversation` when `LlmResponse::Complete` arrives.
    current_response: String,
}

/// A single message in the agent conversation. Kept around for future
/// multi-turn context support — not used for rendering in the inline flow.
#[allow(dead_code)]
pub struct AgentMessage {
    pub role: MessageRole,
    pub content: String,
    pub timestamp: SystemTime,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageRole {
    User,
    Agent,
}

impl AgentMode {
    pub fn new(directory_scope: PathBuf) -> Self {
        Self {
            state: AgentModeState::Inactive,
            input_buffer: String::new(),
            conversation: Vec::new(),
            pending_output: VecDeque::new(),
            directory_scope,
            llm: None,
            session_id: None,
            current_response: String::new(),
        }
    }

    /// Activate agent mode. Lazily spawns the LLM worker on first activation,
    /// then enqueues the initial `>>> ` prompt indicator bytes so the render
    /// layer will draw it into the terminal grid on the next frame.
    pub fn activate(&mut self) {
        if self.state != AgentModeState::Inactive {
            return;
        }
        self.state = AgentModeState::WaitingForInput;
        self.input_buffer.clear();
        log::info!("Agent mode activated in {}", self.directory_scope.display());

        if self.llm.is_none() {
            log::info!("agent_mode: spawning LLM worker (claude -p subprocess)");
            self.llm = Some(LlmWorker::spawn());
        }

        self.enqueue(PROMPT_ANSI.as_bytes().to_vec());
    }

    /// Deactivate agent mode. Emits a trailing newline so the shell's next
    /// output lands cleanly on its own line.
    pub fn deactivate(&mut self) {
        self.state = AgentModeState::Inactive;
        self.input_buffer.clear();
        self.enqueue(b"\r\n".to_vec());
        log::info!("Agent mode deactivated");
    }

    pub fn is_active(&self) -> bool {
        self.state != AgentModeState::Inactive
    }

    /// Abort the current in-flight LLM request (Ctrl+C from the user).
    ///
    /// Kills the spawned `claude` subprocess, drops any tokens that were
    /// mid-flight in the worker channel, writes a dim gray "^C — interrupted"
    /// line into the terminal grid, resets the partial response buffer, and
    /// returns the agent to the ready state so the next prompt can be entered.
    ///
    /// No-op if the worker isn't initialized or no request is in flight.
    pub fn cancel_llm(&mut self) {
        if self.state != AgentModeState::Processing {
            return;
        }
        if let Some(worker) = self.llm.as_ref() {
            worker.cancel();
            // Drop any streamed tokens still sitting in the channel so they
            // don't leak into the next turn.
            worker.drain_pending();
        }
        log::info!("agent_mode: LLM request cancelled by user (Ctrl+C)");
        self.current_response.clear();
        self.enqueue(INTERRUPTED_ANSI.as_bytes().to_vec());
        self.state = AgentModeState::WaitingForInput;
        self.enqueue(PROMPT_ANSI.as_bytes().to_vec());
    }

    /// Drain all pending ANSI output. The render layer calls this every frame
    /// and pipes the bytes into `TerminalBackend::write_agent_bytes`.
    pub fn drain_output(&mut self) -> Vec<Vec<u8>> {
        self.pending_output.drain(..).collect()
    }

    fn enqueue(&mut self, bytes: Vec<u8>) {
        if !bytes.is_empty() {
            self.pending_output.push_back(bytes);
        }
    }

    fn enqueue_system(&mut self, message: &str) {
        let mut out = String::new();
        out.push_str(SYSTEM_ANSI_PREFIX);
        out.push_str(message);
        out.push_str(SYSTEM_ANSI_SUFFIX);
        self.enqueue(out.into_bytes());
    }

    fn enqueue_error(&mut self, message: &str) {
        let mut out = String::new();
        out.push_str(ERROR_ANSI_PREFIX);
        out.push_str(message);
        out.push_str(ERROR_ANSI_SUFFIX);
        self.enqueue(out.into_bytes());
    }

    /// Handle a keyboard event routed from the pane input layer.
    ///
    /// Returns `true` if the event was consumed (so the caller should NOT
    /// forward it to the PTY).
    ///
    /// This path replaces the old egui `TextEdit` submit handler that was the
    /// root cause of #123 — `TextEdit::singleline` was consuming Enter before
    /// the app-level submit check could see it. Now keys are captured directly
    /// from the event stream before any terminal widget renders.
    pub fn handle_key_event(&mut self, event: &egui::Event) -> bool {
        // Escape always exits, even mid-processing.
        if let egui::Event::Key { key: egui::Key::Escape, pressed: true, .. } = event {
            self.deactivate();
            return true;
        }

        // Ctrl+C during an in-flight LLM request aborts it and returns to the
        // prompt. Outside of Processing we deliberately let Ctrl+C fall through
        // so the shell/terminal handles it normally (SIGINT to foreground job).
        if self.state == AgentModeState::Processing {
            if let egui::Event::Key {
                key: egui::Key::C,
                pressed: true,
                modifiers,
                ..
            } = event
            {
                if modifiers.ctrl {
                    self.cancel_llm();
                    return true;
                }
            }
        }

        if self.state != AgentModeState::WaitingForInput {
            // While processing, swallow other key/text events so they don't
            // leak to the PTY and disturb whatever shell is running.
            return matches!(event, egui::Event::Text(_) | egui::Event::Key { .. });
        }

        match event {
            egui::Event::Text(text) => {
                for ch in text.chars() {
                    if !ch.is_control() {
                        self.input_buffer.push(ch);
                        // Echo into the terminal grid so the user sees it.
                        let mut buf = [0u8; 4];
                        self.enqueue(ch.encode_utf8(&mut buf).as_bytes().to_vec());
                    }
                }
                true
            }
            egui::Event::Key { key, pressed: true, modifiers, .. } => {
                match key {
                    egui::Key::Enter => {
                        if modifiers.shift {
                            // Shift+Enter inserts a newline (multi-line).
                            self.input_buffer.push('\n');
                            self.enqueue(b"\r\n".to_vec());
                        } else {
                            self.submit();
                        }
                        true
                    }
                    egui::Key::Backspace => {
                        if self.input_buffer.pop().is_some() {
                            // Erase one cell: move back, space, move back.
                            self.enqueue(b"\x08 \x08".to_vec());
                        }
                        true
                    }
                    // Swallow other keys so they don't reach the PTY. Future
                    // follow-ups can add arrow-key buffer editing and history.
                    _ => true,
                }
            }
            _ => false,
        }
    }

    /// Commit the current input buffer as a user message and dispatch to the
    /// LLM worker.
    fn submit(&mut self) {
        let text = self.input_buffer.trim().to_string();
        if text.is_empty() {
            // Empty Enter just redraws the prompt on a fresh line.
            self.enqueue(PROMPT_ANSI.as_bytes().to_vec());
            return;
        }

        self.conversation.push(AgentMessage {
            role: MessageRole::User,
            content: text.clone(),
            timestamp: SystemTime::now(),
        });
        self.input_buffer.clear();
        self.current_response.clear();

        // Close the prompt line before the thinking indicator / reply.
        self.enqueue(b"\r\n".to_vec());

        let Some(worker) = self.llm.as_ref() else {
            self.enqueue_error(
                "Cannot reach the LLM: agent worker not initialized. Restart agent mode."
            );
            self.enqueue(PROMPT_ANSI.as_bytes().to_vec());
            return;
        };

        let system = build_system_prompt(&self.directory_scope);
        let request = LlmRequest::Complete {
            prompt: text,
            system,
            session_id: self.session_id.clone(),
        };
        match worker.send(request) {
            Ok(()) => {
                self.state = AgentModeState::Processing;
                self.enqueue(THINKING_ANSI.as_bytes().to_vec());
            }
            Err(err) => {
                log::error!("agent_mode: failed to dispatch LLM request: {err}");
                self.enqueue_error(&format!("Failed to send request to LLM worker: {err}"));
                self.enqueue(PROMPT_ANSI.as_bytes().to_vec());
            }
        }
    }

    /// Drain any pending responses from the LLM worker and convert them to
    /// ANSI output. Call once per frame from the render layer. Returns `true`
    /// if any output was produced (so the caller can request a repaint).
    pub fn poll_llm(&mut self) -> bool {
        let Some(worker) = self.llm.as_ref() else {
            return false;
        };
        let mut pending: Vec<LlmResponse> = Vec::new();
        while let Some(resp) = worker.try_recv() {
            pending.push(resp);
        }
        if pending.is_empty() {
            return false;
        }

        for resp in pending {
            match resp {
                LlmResponse::Complete(text, session_id) => {
                    // Store session ID for the next turn.
                    log::info!("agent_mode: turn complete, session_id={session_id}");
                    self.session_id = Some(session_id);

                    // If no tokens were streamed (e.g. non-streaming path),
                    // render the full text now. Otherwise, the terminal grid
                    // already shows the streamed output — just push the newline
                    // separator and the next prompt.
                    if self.current_response.is_empty() && !text.is_empty() {
                        let ansi = markdown_to_ansi(&text);
                        let mut bytes = b"\r\n".to_vec();
                        bytes.extend_from_slice(ansi.as_bytes());
                        self.enqueue(bytes);
                    } else {
                        // Streaming mode: tokens already rendered inline.
                        // Emit a trailing newline so the next prompt starts
                        // on a clean line.
                        self.enqueue(b"\r\n".to_vec());
                    }

                    self.conversation.push(AgentMessage {
                        role: MessageRole::Agent,
                        content: text,
                        timestamp: SystemTime::now(),
                    });
                    self.current_response.clear();
                    self.state = AgentModeState::WaitingForInput;
                    self.enqueue(PROMPT_ANSI.as_bytes().to_vec());
                }
                LlmResponse::Token(chunk) => {
                    // Streaming chunk — render it directly into the terminal grid.
                    // The chunks arrive as plain text (not markdown), so we emit
                    // them verbatim. Markdown rendering is applied to the full
                    // text on Complete only when there was no streaming.
                    self.current_response.push_str(&chunk);
                    // Convert newlines to CRLF for the terminal emulator.
                    let bytes = chunk.replace('\n', "\r\n").into_bytes();
                    self.enqueue(bytes);
                }
                LlmResponse::Error(err) => {
                    log::error!("agent_mode: LLM error: {err}");
                    self.enqueue_error(&format!("LLM error: {err}"));
                    self.current_response.clear();
                    self.state = AgentModeState::WaitingForInput;
                    self.enqueue(PROMPT_ANSI.as_bytes().to_vec());
                }
            }
        }
        true
    }
}

fn build_system_prompt(cwd: &std::path::Path) -> String {
    format!(
        "You are an assistant inside Plexi, a terminal multiplexer. \
The user is currently working in directory: {}. Respond concisely.",
        cwd.display()
    )
}
