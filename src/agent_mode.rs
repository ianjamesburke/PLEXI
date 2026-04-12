use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::SystemTime;

use crate::agent_ansi::markdown_to_ansi;
use crate::agent_llm::{LlmRequest, LlmResponse, LlmWorker};

/// Plexi's user-facing CLI app id. Secrets stored via the secrets manager
/// or `plexi secret set` use this namespace, and agent mode reads from it.
const SECRETS_APP_ID: &str = "plexi-run";

/// Key the user must set to enable the LLM backend.
const API_KEY_NAME: &str = "ANTHROPIC_API_KEY";

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
    /// successfully; stays `None` if the API key is missing.
    llm: Option<LlmWorker>,
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
            self.try_spawn_worker();
        }

        self.enqueue(PROMPT_ANSI.as_bytes().to_vec());
    }

    /// Attempt to load the API key from the Plexi secrets store and spawn a
    /// worker thread. On failure, pushes an inline error into the terminal.
    fn try_spawn_worker(&mut self) {
        let dir_str = self.directory_scope.to_string_lossy().to_string();
        match crate::secrets::resolve_secret(API_KEY_NAME, SECRETS_APP_ID, &dir_str) {
            Some(key) => {
                log::info!("agent_mode: loaded {API_KEY_NAME} from secrets, spawning LLM worker");
                let worker = LlmWorker::spawn(key.to_string());
                self.llm = Some(worker);
            }
            None => {
                log::warn!("agent_mode: {API_KEY_NAME} not found in secrets (app_id={SECRETS_APP_ID})");
                self.enqueue_system(&format!(
                    "No {API_KEY_NAME} found. Open the Secrets manager and add a secret with key \"{API_KEY_NAME}\" to enable the agent."
                ));
            }
        }
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

        // Close the prompt line before the thinking indicator / reply.
        self.enqueue(b"\r\n".to_vec());

        let Some(worker) = self.llm.as_ref() else {
            self.enqueue_error(&format!(
                "Cannot reach the LLM: {API_KEY_NAME} is not set. Add it in the Secrets manager, then restart agent mode."
            ));
            self.enqueue(PROMPT_ANSI.as_bytes().to_vec());
            return;
        };

        let system = build_system_prompt(&self.directory_scope);
        let request = LlmRequest::Complete { prompt: text, system };
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
                LlmResponse::Complete(text) => {
                    let ansi = markdown_to_ansi(&text);
                    self.conversation.push(AgentMessage {
                        role: MessageRole::Agent,
                        content: text,
                        timestamp: SystemTime::now(),
                    });
                    let mut bytes = b"\r\n".to_vec();
                    bytes.extend_from_slice(ansi.as_bytes());
                    self.enqueue(bytes);
                    self.state = AgentModeState::WaitingForInput;
                    self.enqueue(PROMPT_ANSI.as_bytes().to_vec());
                }
                LlmResponse::Token(chunk) => {
                    // Streaming not wired yet — if a token arrives, dump it
                    // inline. The final Complete follows.
                    self.enqueue(chunk.into_bytes());
                }
                LlmResponse::Error(err) => {
                    self.enqueue_error(&format!("LLM error: {err}"));
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
