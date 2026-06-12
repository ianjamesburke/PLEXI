//! Pure state for the host Assistant — testable without egui.
//!
//! `AssistantModel` owns the active conversation, composer text, and
//! streaming state. State transitions return `AssistantEffect`s; the pane
//! shell (`AssistantApp`) executes them.

use super::commands::{self, ParsedCommand};

/// Who produced a transcript row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnRole {
    User,
    Assistant,
    Tool,
    Error,
}

/// One row in the conversation transcript.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Turn {
    pub role: TurnRole,
    pub text: String,
    /// RFC 3339.
    pub created_at: String,
}

impl Turn {
    pub fn now(role: TurnRole, text: impl Into<String>) -> Self {
        Self {
            role,
            text: text.into(),
            created_at: crate::host::event_log::now_timestamp(),
        }
    }
}

/// Live state of an in-flight model turn. Reasoning deltas are carried
/// separately from answer text so the renderer can show a collapsed
/// thinking section above the streaming answer.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StreamingState {
    pub in_flight: bool,
    pub partial_answer: String,
    pub partial_reasoning: String,
}

/// Side effects the model requests from the pane shell. Phase 1 implements
/// `AiQuery` and `SessionWrite`; the rest are correctly-shaped stubs for the
/// Phase 2 host-tool work (executed as logged no-ops, never panics).
#[derive(Debug, Clone, PartialEq)]
pub enum AssistantEffect {
    /// Run a model turn for `prompt` in `conversation_id`.
    AiQuery {
        conversation_id: String,
        prompt: String,
    },
    /// Persist unwritten turns and the active conversation id to disk.
    SessionWrite { conversation_id: String },
    /// Phase 2: typed app-connector tool call.
    ToolCall {
        conversation_id: String,
        tool: String,
        input_json: String,
    },
    /// Phase 2: host pane control (open/focus/read).
    PaneAction { action: String },
    /// Phase 2: permission sheet for a tool or connector grant.
    PermissionPrompt { target: String },
}

fn new_conversation_id() -> String {
    format!("conv-{}", uuid::Uuid::new_v4())
}

/// Pure Assistant state: one active conversation + composer + streaming.
#[derive(Debug)]
pub struct AssistantModel {
    pub conversation_id: String,
    pub turns: Vec<Turn>,
    pub composer: String,
    pub streaming: StreamingState,
    /// Selected row in the slash-command picker (clamped by the renderer).
    pub picker_selected: usize,
}

impl AssistantModel {
    /// Resume an existing conversation.
    pub fn resume(conversation_id: String, turns: Vec<Turn>) -> Self {
        Self {
            conversation_id,
            turns,
            composer: String::new(),
            streaming: StreamingState::default(),
            picker_selected: 0,
        }
    }

    /// Start a brand-new conversation.
    pub fn fresh() -> Self {
        Self::resume(new_conversation_id(), Vec::new())
    }

    /// True while the slash-command picker should be visible.
    pub fn picker_active(&self) -> bool {
        commands::picker_active(&self.composer)
    }

    /// The picker filter query: text after the leading `/`.
    pub fn picker_query(&self) -> String {
        self.composer
            .trim_start()
            .strip_prefix('/')
            .unwrap_or("")
            .to_string()
    }

    /// Submit the composer. Returns the effects to execute. No-op while a
    /// turn is in flight or when the composer is blank.
    pub fn submit(&mut self) -> Vec<AssistantEffect> {
        if self.streaming.in_flight {
            return Vec::new();
        }
        let input = self.composer.trim().to_string();
        if input.is_empty() {
            self.composer.clear();
            return Vec::new();
        }
        self.composer.clear();
        self.picker_selected = 0;
        if let Some(cmd) = commands::parse_slash_command(&input) {
            return self.execute_command(&cmd);
        }
        log::info!(
            "assistant[{}]: turn start ({} chars)",
            self.conversation_id,
            input.len()
        );
        self.turns.push(Turn::now(TurnRole::User, input.clone()));
        self.streaming = StreamingState {
            in_flight: true,
            ..Default::default()
        };
        vec![
            AssistantEffect::SessionWrite {
                conversation_id: self.conversation_id.clone(),
            },
            AssistantEffect::AiQuery {
                conversation_id: self.conversation_id.clone(),
                prompt: input,
            },
        ]
    }

    /// Execute a parsed slash command. `/clear`, `/new`, and `/help` are
    /// real; other built-ins answer with a stub row; unknown names error.
    fn execute_command(&mut self, cmd: &ParsedCommand) -> Vec<AssistantEffect> {
        log::info!(
            "assistant[{}]: command /{} args='{}'",
            self.conversation_id,
            cmd.name,
            cmd.args
        );
        match cmd.name.as_str() {
            // Fresh context in a new conversation; the prior transcript
            // stays on disk and is resumable.
            "clear" => {
                let prior = self.conversation_id.clone();
                self.conversation_id = new_conversation_id();
                self.turns.clear();
                log::info!(
                    "assistant: /clear — new conversation {} (prior {prior} resumable)",
                    self.conversation_id
                );
                vec![AssistantEffect::SessionWrite {
                    conversation_id: self.conversation_id.clone(),
                }]
            }
            // New named conversation; current one is kept.
            "new" => {
                self.conversation_id = new_conversation_id();
                self.turns.clear();
                if !cmd.args.is_empty() {
                    self.turns.push(Turn::now(
                        TurnRole::Assistant,
                        format!("Started conversation '{}'.", cmd.args),
                    ));
                }
                log::info!(
                    "assistant: /new — conversation {} ('{}')",
                    self.conversation_id,
                    cmd.args
                );
                vec![AssistantEffect::SessionWrite {
                    conversation_id: self.conversation_id.clone(),
                }]
            }
            "help" => {
                self.turns
                    .push(Turn::now(TurnRole::Assistant, commands::help_text()));
                vec![AssistantEffect::SessionWrite {
                    conversation_id: self.conversation_id.clone(),
                }]
            }
            name if commands::is_builtin(name) => {
                self.turns.push(Turn::now(
                    TurnRole::Assistant,
                    format!("/{name} is not yet implemented."),
                ));
                let mut effects = vec![AssistantEffect::SessionWrite {
                    conversation_id: self.conversation_id.clone(),
                }];
                // Route the Phase 2 surfaces through their future effect
                // shapes so the executor seam is exercised today (the
                // executors are logged no-ops until Phase 2).
                match name {
                    "permissions" => effects.push(AssistantEffect::PermissionPrompt {
                        target: "assistant".to_string(),
                    }),
                    "tools" => effects.push(AssistantEffect::ToolCall {
                        conversation_id: self.conversation_id.clone(),
                        tool: "host.tools.list".to_string(),
                        input_json: "{}".to_string(),
                    }),
                    "apps" => effects.push(AssistantEffect::PaneAction {
                        action: "list_app_connectors".to_string(),
                    }),
                    _ => {}
                }
                effects
            }
            name => {
                self.turns.push(Turn::now(
                    TurnRole::Error,
                    format!("Unknown command /{name}. Type /help for the list."),
                ));
                vec![AssistantEffect::SessionWrite {
                    conversation_id: self.conversation_id.clone(),
                }]
            }
        }
    }

    /// Append an answer text delta from the streaming turn.
    pub fn apply_answer_delta(&mut self, chunk: &str) {
        if self.streaming.in_flight {
            self.streaming.partial_answer.push_str(chunk);
        }
    }

    /// Append a reasoning delta from the streaming turn.
    pub fn apply_reasoning_delta(&mut self, chunk: &str) {
        if self.streaming.in_flight {
            self.streaming.partial_reasoning.push_str(chunk);
        }
    }

    /// Complete the in-flight turn with the broker outcome. Returns the
    /// persistence effects. Outcomes for a conversation that was cleared
    /// mid-turn are dropped.
    pub fn finish_turn(
        &mut self,
        conversation_id: &str,
        outcome: Result<String, String>,
    ) -> Vec<AssistantEffect> {
        if conversation_id != self.conversation_id {
            log::info!(
                "assistant: dropping turn outcome for stale conversation {conversation_id} (active {})",
                self.conversation_id
            );
            self.streaming = StreamingState::default();
            return Vec::new();
        }
        match outcome {
            Ok(text) => {
                log::info!(
                    "assistant[{}]: turn end ({} chars)",
                    self.conversation_id,
                    text.len()
                );
                self.turns.push(Turn::now(TurnRole::Assistant, text));
            }
            Err(e) => {
                log::warn!("assistant[{}]: turn failed: {e}", self.conversation_id);
                self.turns.push(Turn::now(TurnRole::Error, e));
            }
        }
        self.streaming = StreamingState::default();
        vec![AssistantEffect::SessionWrite {
            conversation_id: self.conversation_id.clone(),
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn submitted(model: &mut AssistantModel, text: &str) -> Vec<AssistantEffect> {
        model.composer = text.to_string();
        model.submit()
    }

    #[test]
    fn submit_pushes_user_turn_and_requests_query() {
        let mut m = AssistantModel::fresh();
        let effects = submitted(&mut m, "hello there");
        assert_eq!(m.turns.len(), 1);
        assert_eq!(m.turns[0].role, TurnRole::User);
        assert_eq!(m.turns[0].text, "hello there");
        assert!(m.streaming.in_flight);
        assert!(m.composer.is_empty());
        assert_eq!(effects.len(), 2);
        assert!(matches!(effects[0], AssistantEffect::SessionWrite { .. }));
        assert!(
            matches!(&effects[1], AssistantEffect::AiQuery { conversation_id, prompt }
                if *conversation_id == m.conversation_id && prompt == "hello there")
        );
    }

    #[test]
    fn submit_is_noop_while_streaming_or_blank() {
        let mut m = AssistantModel::fresh();
        assert!(submitted(&mut m, "   ").is_empty());
        assert!(m.turns.is_empty());

        submitted(&mut m, "question");
        assert!(m.streaming.in_flight);
        m.composer = "second question".to_string();
        assert!(m.submit().is_empty());
        assert_eq!(m.turns.len(), 1, "no turn appended while in flight");
    }

    #[test]
    fn streaming_deltas_accumulate_separately() {
        let mut m = AssistantModel::fresh();
        submitted(&mut m, "q");
        m.apply_reasoning_delta("thinking ");
        m.apply_reasoning_delta("hard");
        m.apply_answer_delta("an");
        m.apply_answer_delta("swer");
        assert_eq!(m.streaming.partial_reasoning, "thinking hard");
        assert_eq!(m.streaming.partial_answer, "answer");
    }

    #[test]
    fn finish_turn_appends_assistant_row_and_resets_streaming() {
        let mut m = AssistantModel::fresh();
        submitted(&mut m, "q");
        m.apply_answer_delta("partial");
        let id = m.conversation_id.clone();
        let effects = m.finish_turn(&id, Ok("final answer".to_string()));
        assert_eq!(m.streaming, StreamingState::default());
        assert_eq!(m.turns.last().unwrap().role, TurnRole::Assistant);
        assert_eq!(m.turns.last().unwrap().text, "final answer");
        assert!(matches!(&effects[0], AssistantEffect::SessionWrite { conversation_id } if *conversation_id == id));
    }

    #[test]
    fn finish_turn_error_appends_error_row() {
        let mut m = AssistantModel::fresh();
        submitted(&mut m, "q");
        let id = m.conversation_id.clone();
        m.finish_turn(&id, Err("api_key_missing".to_string()));
        assert_eq!(m.turns.last().unwrap().role, TurnRole::Error);
        assert!(!m.streaming.in_flight);
    }

    #[test]
    fn finish_turn_for_stale_conversation_is_dropped() {
        let mut m = AssistantModel::fresh();
        submitted(&mut m, "q");
        let old_id = m.conversation_id.clone();
        // Simulate the turn racing a /clear: streaming flag cleared by the
        // command path, conversation id changed, then the old outcome lands.
        m.streaming = StreamingState::default();
        submitted(&mut m, "/clear");
        let effects = m.finish_turn(&old_id, Ok("late answer".to_string()));
        assert!(effects.is_empty());
        assert!(m.turns.is_empty(), "stale outcome must not land in the cleared conversation");
    }

    #[test]
    fn clear_starts_fresh_conversation_with_new_id() {
        let mut m = AssistantModel::fresh();
        let old_id = m.conversation_id.clone();
        submitted(&mut m, "remember this");
        m.streaming = StreamingState::default();
        let effects = submitted(&mut m, "/clear");
        assert_ne!(m.conversation_id, old_id);
        assert!(m.turns.is_empty());
        assert!(matches!(&effects[0], AssistantEffect::SessionWrite { conversation_id }
            if *conversation_id == m.conversation_id));
    }

    #[test]
    fn new_creates_named_conversation() {
        let mut m = AssistantModel::fresh();
        let old_id = m.conversation_id.clone();
        submitted(&mut m, "/new project notes");
        assert_ne!(m.conversation_id, old_id);
        assert_eq!(m.turns.len(), 1);
        assert!(m.turns[0].text.contains("project notes"));
    }

    #[test]
    fn help_lists_builtins() {
        let mut m = AssistantModel::fresh();
        submitted(&mut m, "/help");
        assert_eq!(m.turns.len(), 1);
        assert!(m.turns[0].text.contains("/clear"));
    }

    #[test]
    fn stubbed_builtin_answers_not_yet_implemented() {
        let mut m = AssistantModel::fresh();
        submitted(&mut m, "/permissions");
        assert_eq!(m.turns[0].role, TurnRole::Assistant);
        assert!(m.turns[0].text.contains("not yet implemented"));
    }

    #[test]
    fn unknown_command_answers_with_error_row() {
        let mut m = AssistantModel::fresh();
        submitted(&mut m, "/frobnicate");
        assert_eq!(m.turns[0].role, TurnRole::Error);
        assert!(m.turns[0].text.contains("/frobnicate"));
    }

    #[test]
    fn slash_mid_text_is_a_normal_prompt() {
        let mut m = AssistantModel::fresh();
        let effects = submitted(&mut m, "what is in /etc/hosts");
        assert!(matches!(&effects[1], AssistantEffect::AiQuery { .. }));
        assert_eq!(m.turns[0].role, TurnRole::User);
    }

    /// Phase 2 effect variants stay correctly shaped — constructing and
    /// matching them is the contract until real executors land.
    #[test]
    fn phase2_effect_stubs_construct() {
        let effects = vec![
            AssistantEffect::ToolCall {
                conversation_id: "c".to_string(),
                tool: "csv.read_range".to_string(),
                input_json: "{}".to_string(),
            },
            AssistantEffect::PaneAction {
                action: "focus:1".to_string(),
            },
            AssistantEffect::PermissionPrompt {
                target: "app_connector:app.csv.write_range".to_string(),
            },
        ];
        assert_eq!(effects.len(), 3);
    }
}
