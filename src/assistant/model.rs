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
    /// An app event delivered through a granted subscription (Phase D3).
    Event,
}

/// Final state of a tool-call transcript row. In-flight states (pending,
/// running) live in `AssistantModel::active_tools`; only completed calls are
/// persisted as turns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolStatus {
    Succeeded,
    Failed,
}

/// One row in the conversation transcript.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Turn {
    pub role: TurnRole,
    pub text: String,
    /// RFC 3339.
    pub created_at: String,
    /// Final status for `TurnRole::Tool` rows; `None` for every other role.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<ToolStatus>,
}

impl Turn {
    pub fn now(role: TurnRole, text: impl Into<String>) -> Self {
        Self {
            role,
            text: text.into(),
            created_at: crate::host::event_log::now_timestamp(),
            status: None,
        }
    }

    /// A completed tool-call row.
    pub fn tool(text: impl Into<String>, status: ToolStatus) -> Self {
        Self {
            status: Some(status),
            ..Self::now(TurnRole::Tool, text)
        }
    }
}

/// One tool call currently running inside the in-flight turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveToolCall {
    pub tool: String,
}

/// A permission sheet awaiting the user's decision (renderable state only —
/// the worker reply channel lives in `AssistantApp`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingPermission {
    pub tool: String,
    pub input_summary: String,
}

/// What the user chose on the permission sheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionChoice {
    AllowOnce,
    AllowSession,
    AllowAlways,
    Deny,
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

/// Side effects the model requests from the pane shell.
#[derive(Debug, Clone, PartialEq)]
pub enum AssistantEffect {
    /// Run a model turn for `prompt` in `conversation_id`.
    AiQuery {
        conversation_id: String,
        prompt: String,
    },
    /// Persist unwritten turns and the active conversation id to disk.
    SessionWrite { conversation_id: String },
    /// `/tools`: list discovered app connector tools with broker decisions.
    ListTools,
    /// `/permissions`: list persisted grants for the assistant actor.
    ListPermissions,
    /// `/revoke <target_id>`: remove persisted grants for one target.
    RevokeGrant { target_id: String },
    /// `/audit`: show recent audit events.
    ShowAudit,
    /// Phase 3: host pane control (open/focus/read).
    PaneAction { action: String },
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
    /// Tool calls currently running inside the in-flight turn.
    pub active_tools: Vec<ActiveToolCall>,
    /// Permission sheet awaiting a decision; the in-flight turn's worker
    /// thread is blocked until it resolves.
    pub pending_permission: Option<PendingPermission>,
    /// User messages submitted while a turn was in flight. They are already
    /// appended to `turns`; this count tells the pump to dispatch one
    /// follow-up turn that folds them in.
    pub queued_user_turns: usize,
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
            active_tools: Vec::new(),
            pending_permission: None,
            queued_user_turns: 0,
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

    /// Submit the composer. Returns the effects to execute. Blank input is a
    /// no-op. While a turn is in flight, plain messages are appended to the
    /// transcript and queued for one follow-up turn; slash commands stay
    /// no-ops until the turn finishes.
    pub fn submit(&mut self) -> Vec<AssistantEffect> {
        if self.streaming.in_flight {
            let input = self.composer.trim().to_string();
            if input.is_empty() || commands::parse_slash_command(&input).is_some() {
                return Vec::new();
            }
            self.composer.clear();
            self.picker_selected = 0;
            log::info!(
                "assistant[{}]: message queued — turn in flight ({} chars)",
                self.conversation_id,
                input.len()
            );
            self.turns.push(Turn::now(TurnRole::User, input));
            self.queued_user_turns += 1;
            return vec![AssistantEffect::SessionWrite {
                conversation_id: self.conversation_id.clone(),
            }];
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
            // Real Phase 2 views: the pane shell reads the broker/audit state
            // and answers with an info row.
            "tools" => vec![AssistantEffect::ListTools],
            "permissions" => vec![AssistantEffect::ListPermissions],
            "audit" => vec![AssistantEffect::ShowAudit],
            "revoke" => {
                if cmd.args.is_empty() {
                    self.turns.push(Turn::now(
                        TurnRole::Error,
                        "Usage: /revoke <target_id> — see /permissions for target ids.",
                    ));
                    vec![AssistantEffect::SessionWrite {
                        conversation_id: self.conversation_id.clone(),
                    }]
                } else {
                    vec![AssistantEffect::RevokeGrant {
                        target_id: cmd.args.clone(),
                    }]
                }
            }
            name if commands::is_builtin(name) => {
                self.turns.push(Turn::now(
                    TurnRole::Assistant,
                    format!("/{name} is not yet implemented."),
                ));
                let mut effects = vec![AssistantEffect::SessionWrite {
                    conversation_id: self.conversation_id.clone(),
                }];
                // Route the remaining Phase 3 surface through its future
                // effect shape so the executor seam stays exercised.
                if name == "apps" {
                    effects.push(AssistantEffect::PaneAction {
                        action: "list_app_connectors".to_string(),
                    });
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

    /// Push an informational assistant row (slash-view output). Returns the
    /// persistence effect.
    pub fn push_info(&mut self, text: impl Into<String>) -> Vec<AssistantEffect> {
        self.turns.push(Turn::now(TurnRole::Assistant, text));
        vec![AssistantEffect::SessionWrite {
            conversation_id: self.conversation_id.clone(),
        }]
    }

    /// A tool call started running inside the in-flight turn.
    pub fn tool_call_started(&mut self, tool: &str) {
        self.active_tools.push(ActiveToolCall {
            tool: tool.to_string(),
        });
    }

    /// A tool call finished: drop its running row and append a completed
    /// tool turn. Returns the persistence effect.
    pub fn tool_call_finished(
        &mut self,
        tool: &str,
        error: Option<String>,
    ) -> Vec<AssistantEffect> {
        if let Some(pos) = self.active_tools.iter().position(|t| t.tool == tool) {
            self.active_tools.remove(pos);
        }
        let turn = match error {
            None => Turn::tool(tool.to_string(), ToolStatus::Succeeded),
            Some(e) => Turn::tool(format!("{tool} — {e}"), ToolStatus::Failed),
        };
        self.turns.push(turn);
        vec![AssistantEffect::SessionWrite {
            conversation_id: self.conversation_id.clone(),
        }]
    }

    /// The in-flight turn hit an ask-gated tool: show the permission sheet.
    pub fn permission_requested(&mut self, tool: &str, input_summary: &str) {
        self.pending_permission = Some(PendingPermission {
            tool: tool.to_string(),
            input_summary: input_summary.to_string(),
        });
    }

    /// The user decided the pending permission sheet. A denial appends a
    /// failed tool row; allows append nothing (the running/finished rows
    /// follow from the call itself). Returns the persistence effects.
    pub fn permission_resolved(&mut self, choice: PermissionChoice) -> Vec<AssistantEffect> {
        let Some(pending) = self.pending_permission.take() else {
            return Vec::new();
        };
        match choice {
            PermissionChoice::Deny => {
                self.turns.push(Turn::tool(
                    format!("{} — denied by user", pending.tool),
                    ToolStatus::Failed,
                ));
                vec![AssistantEffect::SessionWrite {
                    conversation_id: self.conversation_id.clone(),
                }]
            }
            PermissionChoice::AllowOnce
            | PermissionChoice::AllowSession
            | PermissionChoice::AllowAlways => Vec::new(),
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
            self.active_tools.clear();
            self.pending_permission = None;
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
        self.active_tools.clear();
        self.pending_permission = None;
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
    fn submit_queues_message_while_streaming_and_skips_blank() {
        let mut m = AssistantModel::fresh();
        assert!(submitted(&mut m, "   ").is_empty());
        assert!(m.turns.is_empty());

        submitted(&mut m, "question");
        assert!(m.streaming.in_flight);

        // A plain message mid-turn lands in the transcript and queues for a
        // follow-up turn, persisting but not dispatching.
        let effects = submitted(&mut m, "second question");
        assert_eq!(effects.len(), 1);
        assert!(matches!(effects[0], AssistantEffect::SessionWrite { .. }));
        assert_eq!(m.turns.len(), 2);
        assert_eq!(m.turns[1].text, "second question");
        assert_eq!(m.queued_user_turns, 1);
        assert!(m.composer.is_empty());
        assert!(m.streaming.in_flight, "in-flight turn unaffected");

        // Slash commands stay no-ops mid-turn.
        assert!(submitted(&mut m, "/clear").is_empty());
        assert_eq!(m.turns.len(), 2);
        assert_eq!(m.queued_user_turns, 1);
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
        submitted(&mut m, "/skills");
        assert_eq!(m.turns[0].role, TurnRole::Assistant);
        assert!(m.turns[0].text.contains("not yet implemented"));
    }

    #[test]
    fn phase2_views_route_to_their_effects() {
        let mut m = AssistantModel::fresh();
        assert_eq!(submitted(&mut m, "/tools"), vec![AssistantEffect::ListTools]);
        assert_eq!(
            submitted(&mut m, "/permissions"),
            vec![AssistantEffect::ListPermissions]
        );
        assert_eq!(submitted(&mut m, "/audit"), vec![AssistantEffect::ShowAudit]);
        assert_eq!(
            submitted(&mut m, "/revoke app.csv.write_range"),
            vec![AssistantEffect::RevokeGrant {
                target_id: "app.csv.write_range".to_string()
            }]
        );
        assert!(m.turns.is_empty(), "views answer via the pane shell");

        // Bare /revoke is a usage error row, not an effect.
        let effects = submitted(&mut m, "/revoke");
        assert!(matches!(&effects[0], AssistantEffect::SessionWrite { .. }));
        assert_eq!(m.turns.last().unwrap().role, TurnRole::Error);
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

    #[test]
    fn tool_call_rows_track_running_then_completed_states() {
        let mut m = AssistantModel::fresh();
        submitted(&mut m, "do work");

        m.tool_call_started("csv.read_range");
        assert_eq!(m.active_tools.len(), 1);
        assert_eq!(m.active_tools[0].tool, "csv.read_range");

        let effects = m.tool_call_finished("csv.read_range", None);
        assert!(m.active_tools.is_empty());
        let row = m.turns.last().unwrap();
        assert_eq!(row.role, TurnRole::Tool);
        assert_eq!(row.status, Some(ToolStatus::Succeeded));
        assert!(matches!(&effects[0], AssistantEffect::SessionWrite { .. }));

        m.tool_call_started("csv.write_range");
        m.tool_call_finished("csv.write_range", Some("tool_timeout".to_string()));
        let row = m.turns.last().unwrap();
        assert_eq!(row.status, Some(ToolStatus::Failed));
        assert!(row.text.contains("tool_timeout"));
    }

    #[test]
    fn permission_sheet_state_round_trips() {
        let mut m = AssistantModel::fresh();
        submitted(&mut m, "write the sheet");

        m.permission_requested("csv.write_range", "{\"range\": \"A1:B2\"}");
        let pending = m.pending_permission.as_ref().unwrap();
        assert_eq!(pending.tool, "csv.write_range");

        // Allow: sheet clears, nothing appended (the call rows follow).
        let effects = m.permission_resolved(PermissionChoice::AllowOnce);
        assert!(m.pending_permission.is_none());
        assert!(effects.is_empty());

        // Deny: sheet clears and a failed tool row is appended.
        m.permission_requested("csv.write_range", "{}");
        let effects = m.permission_resolved(PermissionChoice::Deny);
        assert!(m.pending_permission.is_none());
        let row = m.turns.last().unwrap();
        assert_eq!(row.status, Some(ToolStatus::Failed));
        assert!(row.text.contains("denied by user"));
        assert!(matches!(&effects[0], AssistantEffect::SessionWrite { .. }));

        // Resolving with no pending sheet is a no-op.
        assert!(m.permission_resolved(PermissionChoice::Deny).is_empty());
    }

    #[test]
    fn finish_turn_clears_tool_and_permission_state() {
        let mut m = AssistantModel::fresh();
        submitted(&mut m, "q");
        m.tool_call_started("csv.read_range");
        m.permission_requested("csv.write_range", "{}");
        let id = m.conversation_id.clone();
        m.finish_turn(&id, Err("worker died".to_string()));
        assert!(m.active_tools.is_empty());
        assert!(m.pending_permission.is_none());
    }

    #[test]
    fn tool_turn_serde_round_trips_status() {
        let turn = Turn::tool("csv.write_range", ToolStatus::Failed);
        let json = serde_json::to_string(&turn).unwrap();
        let back: Turn = serde_json::from_str(&json).unwrap();
        assert_eq!(back.status, Some(ToolStatus::Failed));
        // Pre-Phase-2 rows without a status field still load.
        let legacy: Turn = serde_json::from_str(
            r#"{"role":"user","text":"hi","created_at":"2026-01-01T00:00:00Z"}"#,
        )
        .unwrap();
        assert_eq!(legacy.status, None);
    }
}
