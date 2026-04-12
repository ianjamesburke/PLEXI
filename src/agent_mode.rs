use std::path::PathBuf;
use std::time::SystemTime;

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
}

impl AgentMode {
    pub fn new(directory_scope: PathBuf) -> Self {
        Self {
            state: AgentModeState::Inactive,
            input_buffer: String::new(),
            conversation: Vec::new(),
            directory_scope,
        }
    }

    /// Activate agent mode. Called when `/` is pressed at an empty prompt.
    pub fn activate(&mut self) {
        if self.state == AgentModeState::Inactive {
            self.state = AgentModeState::WaitingForInput;
            self.input_buffer.clear();
            log::info!("Agent mode activated in {}", self.directory_scope.display());
        }
    }

    /// Deactivate agent mode. Called on Escape or when the agent finishes.
    pub fn deactivate(&mut self) {
        self.state = AgentModeState::Inactive;
        self.input_buffer.clear();
        log::info!("Agent mode deactivated");
    }

    /// Submit the current input buffer as a user message.
    /// Returns the submitted text if non-empty, None otherwise.
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
        self.state = AgentModeState::Processing;
        Some(text)
    }

    /// Push a stub agent response. Placeholder for real LLM integration.
    pub fn push_stub_response(&mut self, content: String) {
        self.conversation.push(AgentMessage {
            role: MessageRole::Agent,
            content,
            timestamp: SystemTime::now(),
        });
        self.state = AgentModeState::WaitingForInput;
    }

    pub fn is_active(&self) -> bool {
        self.state != AgentModeState::Inactive
    }
}
