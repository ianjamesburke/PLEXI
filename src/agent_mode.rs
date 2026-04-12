use std::path::PathBuf;
use std::time::SystemTime;

use crate::agent_llm::{LlmRequest, LlmResponse, LlmWorker};

/// Plexi's user-facing CLI app id. Secrets stored via the secrets manager
/// or `plexi secret set` use this namespace, and agent mode reads from it.
const SECRETS_APP_ID: &str = "plexi-run";

/// Key the user must set to enable the LLM backend.
const API_KEY_NAME: &str = "ANTHROPIC_API_KEY";

/// State machine for agent mode within a terminal pane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentModeState {
    /// Agent mode is not active — normal shell mode.
    Inactive,
    /// User is typing their prompt to the agent.
    WaitingForInput,
    /// Agent is processing (LLM call in progress).
    Processing,
    /// Agent is streaming a response back.
    #[allow(dead_code)]
    Responding,
}

/// A single message in the agent conversation.
pub struct AgentMessage {
    pub role: MessageRole,
    pub content: String,
    pub timestamp: SystemTime,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageRole {
    User,
    Agent,
    System,
}

/// Per-pane agent mode state. Each terminal pane has its own AgentMode instance.
pub struct AgentMode {
    pub state: AgentModeState,
    pub input_buffer: String,
    pub conversation: Vec<AgentMessage>,
    pub directory_scope: PathBuf,
    /// Lazily-initialized LLM worker. `None` until activate() runs once
    /// successfully; stays `None` if the API key is missing (we surface
    /// an inline error message in the conversation instead).
    llm: Option<LlmWorker>,
}

impl AgentMode {
    pub fn new(directory_scope: PathBuf) -> Self {
        Self {
            state: AgentModeState::Inactive,
            input_buffer: String::new(),
            conversation: Vec::new(),
            directory_scope,
            llm: None,
        }
    }

    /// Activate agent mode. Lazily spawns the LLM worker on first activation.
    /// If `ANTHROPIC_API_KEY` is missing, we still enter agent mode but push
    /// a system message explaining how to fix it.
    pub fn activate(&mut self) {
        if self.state != AgentModeState::Inactive {
            return;
        }
        self.state = AgentModeState::WaitingForInput;
        self.input_buffer.clear();
        log::info!("Agent mode activated in {}", self.directory_scope.display());

        // Lazy-init the LLM worker. Only do this the first time we activate.
        if self.llm.is_none() {
            self.try_spawn_worker();
        }
    }

    /// Attempt to load the API key from the Plexi secrets store and spawn a
    /// worker thread. On success, sets `self.llm`. On failure, pushes a
    /// friendly system message into the conversation telling the user how
    /// to set it.
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
                self.push_system_message(format!(
                    "No {API_KEY_NAME} found. Open the Secrets manager (from the sidebar) and \
add a secret with key \"{API_KEY_NAME}\" to enable the agent."
                ));
            }
        }
    }

    /// Deactivate agent mode. Called on Escape or when the agent finishes.
    pub fn deactivate(&mut self) {
        self.state = AgentModeState::Inactive;
        self.input_buffer.clear();
        log::info!("Agent mode deactivated");
    }

    /// Submit the current input buffer as a user message and dispatch it to
    /// the LLM worker. Returns the submitted text if non-empty, None otherwise.
    pub fn submit(&mut self) -> Option<String> {
        let text = self.input_buffer.trim().to_string();
        if text.is_empty() {
            return None;
        }
        self.conversation.push(AgentMessage {
            role: MessageRole::User,
            content: text.clone(),
            timestamp: SystemTime::now(),
        });
        self.input_buffer.clear();

        // No worker → surface an error inline and stay in waiting state so
        // the user can edit secrets and try again.
        let Some(worker) = self.llm.as_ref() else {
            self.push_system_message(format!(
                "Cannot reach the LLM: {API_KEY_NAME} is not set. \
Add it in the Secrets manager, then press Escape and Ctrl+/ again."
            ));
            self.state = AgentModeState::WaitingForInput;
            return Some(text);
        };

        let system = build_system_prompt(&self.directory_scope);
        let request = LlmRequest::Complete {
            prompt: text.clone(),
            system,
        };
        match worker.send(request) {
            Ok(()) => {
                self.state = AgentModeState::Processing;
            }
            Err(err) => {
                log::error!("agent_mode: failed to dispatch LLM request: {err}");
                self.push_system_message(format!("Failed to send request to LLM worker: {err}"));
                self.state = AgentModeState::WaitingForInput;
            }
        }

        Some(text)
    }

    /// Drain any pending responses from the LLM worker and apply them to
    /// the conversation. Call from the UI render loop every frame. Returns
    /// `true` if any state changed (so the caller can request a repaint).
    pub fn poll_llm(&mut self) -> bool {
        if self.llm.is_none() {
            return false;
        }
        // Drain responses up front so we don't hold an immutable borrow of
        // `self.llm` across mutable calls like `push_system_message`.
        let mut pending: Vec<LlmResponse> = Vec::new();
        if let Some(worker) = self.llm.as_ref() {
            while let Some(resp) = worker.try_recv() {
                pending.push(resp);
            }
        }
        let changed = !pending.is_empty();
        for resp in pending {
            match resp {
                LlmResponse::Complete(text) => {
                    self.conversation.push(AgentMessage {
                        role: MessageRole::Agent,
                        content: text,
                        timestamp: SystemTime::now(),
                    });
                    self.state = AgentModeState::WaitingForInput;
                }
                LlmResponse::Token(chunk) => {
                    // Streaming not wired up yet, but if a token arrives,
                    // append it to the last agent message or start a new one.
                    match self.conversation.last_mut() {
                        Some(msg) if msg.role == MessageRole::Agent => {
                            msg.content.push_str(&chunk);
                        }
                        _ => {
                            self.conversation.push(AgentMessage {
                                role: MessageRole::Agent,
                                content: chunk,
                                timestamp: SystemTime::now(),
                            });
                        }
                    }
                    self.state = AgentModeState::Responding;
                }
                LlmResponse::Error(err) => {
                    self.push_system_message(format!("LLM error: {err}"));
                    self.state = AgentModeState::WaitingForInput;
                }
            }
        }
        changed
    }

    fn push_system_message(&mut self, content: String) {
        self.conversation.push(AgentMessage {
            role: MessageRole::System,
            content,
            timestamp: SystemTime::now(),
        });
    }

    pub fn is_active(&self) -> bool {
        self.state != AgentModeState::Inactive
    }
}

fn build_system_prompt(cwd: &std::path::Path) -> String {
    format!(
        "You are an assistant inside Plexi, a terminal multiplexer. \
The user is currently working in directory: {}. Respond concisely.",
        cwd.display()
    )
}
