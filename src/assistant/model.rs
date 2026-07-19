//! Pure state for the host Assistant — testable without egui.
//!
//! `AssistantModel` owns the active conversation, composer text, and
//! streaming state. State transitions return `AssistantEffect`s; the pane
//! shell (`AssistantApp`) executes them.

use super::commands::{self, ParsedCommand};
use crate::app_protocol::ModelTier;
use crate::plexi_ai::broker::ReasoningEffort;

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
    /// Reasoning tokens streamed during this turn, kept for `TurnRole::
    /// Assistant` rows so `/thoughts` can reveal them after the turn ends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thoughts: Option<String>,
    /// Render payload for `TurnRole::Tool` rows — the unified diff a
    /// `host.files.edit`/`host.files.write` reported (stint 0421). `None`
    /// for every other role and for tools with no visual payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Compact summary of the tool call's input, for `TurnRole::Tool` rows
    /// (stint 0455). Shown inside the row's caret dropdown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_summary: Option<String>,
    /// Bounded preview of the tool call's output, for `TurnRole::Tool` rows
    /// (stint 0455). Shown inside the row's caret dropdown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_preview: Option<String>,
}

impl Turn {
    pub fn now(role: TurnRole, text: impl Into<String>) -> Self {
        Self {
            role,
            text: text.into(),
            created_at: crate::host::event_log::now_timestamp(),
            status: None,
            thoughts: None,
            detail: None,
            input_summary: None,
            output_preview: None,
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
    /// Compact summary of the call's input, shown on the running row.
    pub input_summary: String,
}

/// Everything the model needs to commit a completed tool call as a
/// transcript row (stint 0455). Built by the pane shell from the tool-flow
/// event; the model never parses tool payloads itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinishedToolCall {
    pub tool: String,
    /// `None` = success; `Some(reason)` renders as a failed row.
    pub error: Option<String>,
    /// Unified diff payload for file edits (stint 0421).
    pub detail: Option<String>,
    /// Compact summary of the call's input.
    pub input_summary: Option<String>,
    /// Bounded preview of the call's output.
    pub output_preview: Option<String>,
}

/// A permission sheet awaiting the user's decision (renderable state only —
/// the worker reply channel lives in `AssistantApp`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingPermission {
    pub tool: String,
    pub input_summary: String,
    /// Keyboard cursor over `PermissionChoice::ORDER`, so Tab/arrow nav can
    /// move it and Enter can activate whatever it lands on.
    pub selected: usize,
}

/// What the user chose on the permission sheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionChoice {
    AllowOnce,
    AllowSession,
    AllowAlways,
    Deny,
}

impl PermissionChoice {
    /// Left-to-right order the sheet renders its actions in — also the
    /// Tab/arrow traversal order.
    pub const ORDER: [PermissionChoice; 4] = [
        PermissionChoice::AllowOnce,
        PermissionChoice::AllowSession,
        PermissionChoice::AllowAlways,
        PermissionChoice::Deny,
    ];
}

/// Live state of an in-flight model turn. Reasoning deltas are carried
/// separately from answer text so the renderer can show a collapsed
/// thinking section above the streaming answer.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StreamingState {
    pub in_flight: bool,
    pub partial_answer: String,
    pub partial_reasoning: String,
    /// Answer text already committed to the transcript mid-turn — segments
    /// flushed when a tool call started (stint 0455). `finish_turn` strips
    /// this prefix from the broker's final text so only the tail commits.
    pub committed_answer: String,
}

/// Observable lifecycle for `/compact`. This deliberately records feedback,
/// not compaction mechanics: raw history and checkpoint format remain owned
/// by the store layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompactionState {
    Idle,
    Compacting,
    Completed {
        compacted_turns: usize,
        checkpoint_id: String,
    },
}

impl Default for CompactionState {
    fn default() -> Self {
        Self::Idle
    }
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
    SessionWrite {
        conversation_id: String,
    },
    /// `/tools`: list discovered app connector tools with broker decisions.
    ListTools,
    /// `/apps`: list running apps and their exposed connectors.
    ListApps,
    ListSkills,
    ShowContext,
    ShowHooks,
    InvokeSkill {
        name: String,
        args: String,
    },
    /// `/permissions` and `/revoke` (no args): open the interactive
    /// permissions manager overlay over the composer.
    OpenPermissionsManager,
    /// `/revoke <target_id>`: remove persisted grants for one target.
    RevokeGrant {
        target_id: String,
    },
    /// `/audit`: show recent audit events.
    ShowAudit,
    /// `/settings` and `/config`: show the resolved Assistant settings.
    ShowSettings,
    /// `/model` (no args): open the interactive model/agent picker overlay.
    OpenModelPicker,
    /// `/model low|medium|high`: override the model tier for this session.
    SetSessionModel(ModelTier),
    ListAgents,
    SwitchAgent(String),
    InspectAgent(String),
    CreateAgent(String),
    EditAgent(String),
    ShowEffort,
    SetSessionEffort(Option<ReasoningEffort>),
    ListConversations,
    ResumeConversation(String),
    ShowHistory,
    RewindConversation(String),
    CompactConversation,
    ExportConversation,
    /// `/thoughts`: persist the flipped show-thoughts preference.
    PersistShowThoughts(bool),
    /// `/clear` or `/new` ran while a turn was in flight: unblock any worker
    /// thread waiting on a permission reply. The late outcome is dropped by
    /// the stale-conversation guard in `finish_turn`.
    CancelTurn,
}

/// One agent the model/agent picker can switch to.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentChoice {
    pub id: String,
    pub display_name: String,
}

/// One editable row in the permissions manager: a persisted grant target and
/// the decision currently selected for it (cycled in-place, applied on Enter).
#[derive(Debug, Clone, PartialEq)]
pub struct GrantRow {
    pub target_id: String,
    pub decision: crate::broker::Decision,
}

/// A modal overlay the composer renders above itself. Distinct from the
/// filter-as-you-type slash-command picker (`picker_selected`/`picker_active`),
/// which has its own text-driven lifecycle.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum AssistantOverlay {
    #[default]
    None,
    /// `/model`: pick a session model tier or switch the active agent. The
    /// flattened selection runs over `tiers` first, then `agents`.
    ModelPicker {
        selected: usize,
        current_tier: ModelTier,
        active_agent_id: String,
        tiers: Vec<ModelTier>,
        agents: Vec<AgentChoice>,
    },
    /// `/permissions` and `/revoke`: review persisted grants and re-decide
    /// each one (Space cycles Allow/Ask/Block), applied on Enter.
    PermissionsManager {
        selected: usize,
        grants: Vec<GrantRow>,
    },
}

fn new_conversation_id() -> String {
    format!("conv-{}", uuid::Uuid::new_v4())
}

/// Pure Assistant state: one active conversation + composer + streaming.
#[derive(Debug)]
pub struct AssistantModel {
    pub conversation_id: String,
    /// User-visible name for this session, persisted across restarts.
    pub session_name: Option<String>,
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
    /// Transcript index where the in-flight turn's rows belong: just after
    /// the user message that started it. Tool rows and the final reply
    /// insert here (advancing it), so output appended mid-turn — slash-view
    /// rows, queued messages — stays below the reply it chronologically
    /// follows. `None` while no turn is in flight.
    pub turn_anchor: Option<usize>,
    /// Show reasoning ("thoughts") sections in the transcript. Toggled by
    /// `/thoughts`, persisted in the assistant store's `state.toml`.
    pub show_thoughts: bool,
    pub active_agent_id: String,
    pub effort_override: Option<ReasoningEffort>,
    /// `/compact` lifecycle exposed to the renderer and injected into the
    /// next model turn so the Assistant can answer status questions.
    pub compaction: CompactionState,
    /// Shell-style composer history cursor over prior user turns. `None`
    /// means normal editing; `Some(0)` is the newest user message.
    history_cursor: Option<usize>,
    /// Modal picker/manager overlay currently open over the composer.
    pub overlay: AssistantOverlay,
}

impl AssistantModel {
    /// Resume an existing conversation.
    pub fn resume(conversation_id: String, turns: Vec<Turn>) -> Self {
        Self {
            conversation_id,
            session_name: None,
            turns,
            composer: String::new(),
            streaming: StreamingState::default(),
            picker_selected: 0,
            active_tools: Vec::new(),
            pending_permission: None,
            queued_user_turns: 0,
            turn_anchor: None,
            show_thoughts: false,
            active_agent_id: "default".to_string(),
            effort_override: None,
            compaction: CompactionState::Idle,
            history_cursor: None,
            overlay: AssistantOverlay::None,
        }
    }

    /// Set the user-visible session name. Blank/whitespace-only clears it.
    pub fn set_session_name(&mut self, name: &str) {
        self.session_name = if name.trim().is_empty() {
            None
        } else {
            Some(name.trim().to_string())
        };
    }

    /// Start a brand-new conversation.
    pub fn fresh() -> Self {
        Self::resume(new_conversation_id(), Vec::new())
    }

    pub fn begin_compaction(&mut self) {
        self.compaction = CompactionState::Compacting;
    }

    pub fn complete_compaction(&mut self, compacted_turns: usize, checkpoint_id: String) {
        self.compaction = CompactionState::Completed {
            compacted_turns,
            checkpoint_id,
        };
    }

    pub fn clear_compaction(&mut self) {
        self.compaction = CompactionState::Idle;
    }

    pub fn compaction_status(&self) -> String {
        match &self.compaction {
            CompactionState::Idle => "No compaction is running.".to_string(),
            CompactionState::Compacting => "Compaction is in progress.".to_string(),
            CompactionState::Completed {
                compacted_turns,
                checkpoint_id,
            } => format!(
                "Compaction completed: {compacted_turns} turns into checkpoint {checkpoint_id}."
            ),
        }
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

    /// True while a modal picker/manager overlay is open over the composer.
    pub fn overlay_active(&self) -> bool {
        !matches!(self.overlay, AssistantOverlay::None)
    }

    /// Open the model/agent picker. `current_tier` is the resolved session
    /// tier; the cursor lands on its row. Callers pass the full tier list and
    /// the current agent roster.
    pub fn open_model_picker(
        &mut self,
        current_tier: ModelTier,
        tiers: Vec<ModelTier>,
        agents: Vec<AgentChoice>,
    ) {
        let selected = tiers.iter().position(|t| *t == current_tier).unwrap_or(0);
        log::info!(
            "assistant[{}]: model picker opened, current tier={current_tier:?}, {} tier(s), {} agent(s)",
            self.conversation_id,
            tiers.len(),
            agents.len()
        );
        self.overlay = AssistantOverlay::ModelPicker {
            selected,
            current_tier,
            active_agent_id: self.active_agent_id.clone(),
            tiers,
            agents,
        };
    }

    /// Open the permissions manager over the persisted grants for the actor.
    pub fn open_permissions_manager(&mut self, grants: Vec<GrantRow>) {
        log::info!(
            "assistant[{}]: permissions manager opened, {} grant(s)",
            self.conversation_id,
            grants.len()
        );
        self.overlay = AssistantOverlay::PermissionsManager {
            selected: 0,
            grants,
        };
    }

    /// Total selectable rows in the open overlay.
    pub fn overlay_len(&self) -> usize {
        match &self.overlay {
            AssistantOverlay::None => 0,
            AssistantOverlay::ModelPicker { tiers, agents, .. } => tiers.len() + agents.len(),
            AssistantOverlay::PermissionsManager { grants, .. } => grants.len(),
        }
    }

    /// The overlay's current cursor row (0 when no overlay is open).
    pub fn overlay_selected(&self) -> usize {
        match &self.overlay {
            AssistantOverlay::None => 0,
            AssistantOverlay::ModelPicker { selected, .. }
            | AssistantOverlay::PermissionsManager { selected, .. } => *selected,
        }
    }

    /// Move the cursor up one row (clamped at the top).
    pub fn overlay_move_up(&mut self) {
        match &mut self.overlay {
            AssistantOverlay::ModelPicker { selected, .. }
            | AssistantOverlay::PermissionsManager { selected, .. } => {
                *selected = selected.saturating_sub(1);
            }
            AssistantOverlay::None => {}
        }
    }

    /// Move the cursor down one row (clamped at the last row).
    pub fn overlay_move_down(&mut self) {
        let len = self.overlay_len();
        match &mut self.overlay {
            AssistantOverlay::ModelPicker { selected, .. }
            | AssistantOverlay::PermissionsManager { selected, .. } => {
                if *selected + 1 < len {
                    *selected += 1;
                }
            }
            AssistantOverlay::None => {}
        }
    }

    /// Space in the permissions manager: cycle the selected grant's decision
    /// Allow → Ask → Block → Allow. No-op for other overlays.
    pub fn overlay_cycle_decision(&mut self) {
        use crate::broker::Decision;
        if let AssistantOverlay::PermissionsManager { selected, grants } = &mut self.overlay {
            if let Some(row) = grants.get_mut(*selected) {
                row.decision = match row.decision {
                    Decision::Allow => Decision::Ask,
                    Decision::Ask => Decision::Deny,
                    Decision::Deny => Decision::Allow,
                };
            }
        }
    }

    /// Close the overlay without applying anything (Esc).
    pub fn cancel_overlay(&mut self) {
        if self.overlay_active() {
            log::info!("assistant[{}]: overlay cancelled", self.conversation_id);
            self.overlay = AssistantOverlay::None;
        }
    }

    /// Submit the composer. Returns the effects to execute. Blank input is a
    /// no-op. Slash commands execute immediately — even mid-turn (`/clear`
    /// and `/new` interrupt the in-flight turn). Plain messages submitted
    /// while a turn is in flight are appended to the transcript and queued
    /// for one follow-up turn.
    pub fn submit(&mut self) -> Vec<AssistantEffect> {
        let input = self.composer.trim().to_string();
        if input.is_empty() {
            self.composer.clear();
            return Vec::new();
        }
        self.composer.clear();
        self.history_cursor = None;
        self.picker_selected = 0;
        if let Some(cmd) = commands::parse_slash_command(&input) {
            return self.execute_command(&cmd);
        }
        self.submit_prompt(input)
    }

    /// Submit text as a user prompt without slash-command reparsing. Used
    /// after an installed skill command has already consumed its command name.
    pub fn submit_prompt(&mut self, input: String) -> Vec<AssistantEffect> {
        if self.streaming.in_flight {
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
        log::info!(
            "assistant[{}]: turn start ({} chars)",
            self.conversation_id,
            input.len()
        );
        self.turns.push(Turn::now(TurnRole::User, input.clone()));
        self.turn_anchor = Some(self.turns.len());
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

    /// Recall previous user messages into the composer. The first Up starts
    /// from the latest submitted user turn; repeated Up walks older turns.
    /// A non-empty draft is left untouched unless history recall is already
    /// active, so one accidental Up never overwrites in-progress typing.
    pub fn recall_previous_user_message(&mut self) -> bool {
        if self.history_cursor.is_none() && !self.composer.trim().is_empty() {
            return false;
        }
        let history: Vec<&str> = self
            .turns
            .iter()
            .rev()
            .filter(|t| t.role == TurnRole::User && !t.text.trim().is_empty())
            .map(|t| t.text.as_str())
            .collect();
        if history.is_empty() {
            return false;
        }
        let next = self
            .history_cursor
            .map(|idx| (idx + 1).min(history.len() - 1))
            .unwrap_or(0);
        self.history_cursor = Some(next);
        self.composer = history[next].to_string();
        self.picker_selected = 0;
        log::info!(
            "assistant[{}]: recalled composer history entry {} of {}",
            self.conversation_id,
            next + 1,
            history.len()
        );
        true
    }

    pub fn reset_history_recall(&mut self) {
        self.history_cursor = None;
    }

    /// Abandon the in-flight turn for a conversation switch: reset all
    /// streaming state and return the `CancelTurn` effect so the pane shell
    /// unblocks any worker waiting on a permission reply. The worker's late
    /// outcome is dropped by the stale-conversation guard in `finish_turn`.
    fn interrupt_turn(&mut self) -> Vec<AssistantEffect> {
        if !self.streaming.in_flight {
            return Vec::new();
        }
        log::info!(
            "assistant[{}]: in-flight turn interrupted by conversation switch",
            self.conversation_id
        );
        self.streaming = StreamingState::default();
        self.active_tools.clear();
        self.pending_permission = None;
        self.queued_user_turns = 0;
        self.turn_anchor = None;
        vec![AssistantEffect::CancelTurn]
    }

    pub(crate) fn switch_conversation(
        &mut self,
        conversation_id: String,
        turns: Vec<Turn>,
    ) -> Vec<AssistantEffect> {
        let effects = self.interrupt_turn();
        self.conversation_id = conversation_id;
        self.turns = turns;
        self.composer.clear();
        self.history_cursor = None;
        self.picker_selected = 0;
        effects
    }

    /// Insert a row that belongs to the in-flight turn at the turn anchor,
    /// keeping it above anything appended mid-turn (slash-view output,
    /// queued messages). Falls back to a plain append when no turn is in
    /// flight.
    fn push_flight_turn(&mut self, turn: Turn) {
        match self.turn_anchor {
            Some(at) => {
                let at = at.min(self.turns.len());
                self.turns.insert(at, turn);
                self.turn_anchor = Some(at + 1);
            }
            None => self.turns.push(turn),
        }
    }

    /// Execute a parsed slash command. Every built-in has a real handler
    /// below; unknown names answer with an error row.
    fn execute_command(&mut self, cmd: &ParsedCommand) -> Vec<AssistantEffect> {
        log::info!(
            "assistant[{}]: command /{} args_len={}",
            self.conversation_id,
            cmd.name,
            cmd.args.len()
        );
        match cmd.name.as_str() {
            // Fresh context in a new conversation; the prior transcript
            // stays on disk and is resumable.
            "clear" => {
                let mut effects = self.interrupt_turn();
                let prior = self.conversation_id.clone();
                self.conversation_id = new_conversation_id();
                self.session_name = None;
                self.turns.clear();
                log::info!(
                    "assistant: /clear — new conversation {} (prior {prior} resumable)",
                    self.conversation_id
                );
                effects.push(AssistantEffect::SessionWrite {
                    conversation_id: self.conversation_id.clone(),
                });
                effects
            }
            // New named conversation; current one is kept.
            "new" => {
                let mut effects = self.interrupt_turn();
                self.conversation_id = new_conversation_id();
                self.turns.clear();
                self.set_session_name(&cmd.args);
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
                effects.push(AssistantEffect::SessionWrite {
                    conversation_id: self.conversation_id.clone(),
                });
                effects
            }
            "help" => {
                self.turns
                    .push(Turn::now(TurnRole::Assistant, commands::help_text()));
                vec![AssistantEffect::SessionWrite {
                    conversation_id: self.conversation_id.clone(),
                }]
            }
            // Toggle reasoning visibility for every turn, past and future.
            "thoughts" => {
                self.show_thoughts = !self.show_thoughts;
                self.turns.push(Turn::now(
                    TurnRole::Assistant,
                    if self.show_thoughts {
                        "Thoughts are now shown. Run /thoughts again to hide them."
                    } else {
                        "Thoughts are now hidden. Run /thoughts again to show them."
                    },
                ));
                log::info!(
                    "assistant: /thoughts — show_thoughts={}",
                    self.show_thoughts
                );
                vec![
                    AssistantEffect::PersistShowThoughts(self.show_thoughts),
                    AssistantEffect::SessionWrite {
                        conversation_id: self.conversation_id.clone(),
                    },
                ]
            }
            // Real Phase 2 views: the pane shell reads the broker/audit state
            // and answers with an info row.
            "tools" => vec![AssistantEffect::ListTools],
            "apps" => vec![AssistantEffect::ListApps],
            "skills" => vec![AssistantEffect::ListSkills],
            "context" => vec![AssistantEffect::ShowContext],
            "hooks" => vec![AssistantEffect::ShowHooks],
            "permissions" => vec![AssistantEffect::OpenPermissionsManager],
            "audit" => vec![AssistantEffect::ShowAudit],
            "settings" | "config" => vec![AssistantEffect::ShowSettings],
            "resume" if cmd.args.is_empty() => vec![AssistantEffect::ListConversations],
            "resume" => vec![AssistantEffect::ResumeConversation(cmd.args.clone())],
            "history" => vec![AssistantEffect::ShowHistory],
            "rewind" if cmd.args.is_empty() => {
                self.turns.push(Turn::now(
                    TurnRole::Error,
                    "Usage: /rewind <turn:N | checkpoint:ID>. Conversation context only; files and apps are untouched.",
                ));
                vec![AssistantEffect::SessionWrite {
                    conversation_id: self.conversation_id.clone(),
                }]
            }
            "rewind" => vec![AssistantEffect::RewindConversation(cmd.args.clone())],
            "compact" => {
                self.begin_compaction();
                vec![AssistantEffect::CompactConversation]
            }
            "export" => vec![AssistantEffect::ExportConversation],
            "model" if cmd.args.is_empty() => vec![AssistantEffect::OpenModelPicker],
            "model" => {
                let tier = match cmd.args.as_str() {
                    "low" => Some(ModelTier::Low),
                    "medium" => Some(ModelTier::Medium),
                    "high" => Some(ModelTier::High),
                    _ => None,
                };
                match tier {
                    Some(tier) => vec![AssistantEffect::SetSessionModel(tier)],
                    None => {
                        self.turns.push(Turn::now(
                            TurnRole::Error,
                            format!(
                                "Invalid model tier '{}'. Expected low | medium | high.",
                                cmd.args
                            ),
                        ));
                        vec![AssistantEffect::SessionWrite {
                            conversation_id: self.conversation_id.clone(),
                        }]
                    }
                }
            }
            "agent" if cmd.args.is_empty() => vec![AssistantEffect::ListAgents],
            "agent" => {
                let mut parts = cmd.args.split_whitespace();
                let first = parts.next().unwrap_or("");
                let second = parts.next();
                match (first, second, parts.next()) {
                    ("inspect", Some(id), None) => {
                        vec![AssistantEffect::InspectAgent(id.to_string())]
                    }
                    ("create", Some(id), None) => {
                        vec![AssistantEffect::CreateAgent(id.to_string())]
                    }
                    ("edit", Some(id), None) => vec![AssistantEffect::EditAgent(id.to_string())],
                    (id, None, None) => vec![AssistantEffect::SwitchAgent(id.to_string())],
                    _ => {
                        self.turns.push(Turn::now(
                            TurnRole::Error,
                            "Usage: /agent [<id> | inspect <id> | create <id> | edit <id>].",
                        ));
                        vec![AssistantEffect::SessionWrite {
                            conversation_id: self.conversation_id.clone(),
                        }]
                    }
                }
            }
            "effort" if cmd.args.is_empty() => vec![AssistantEffect::ShowEffort],
            "effort" => match cmd.args.as_str() {
                "auto" => vec![AssistantEffect::SetSessionEffort(None)],
                "low" => vec![AssistantEffect::SetSessionEffort(Some(
                    ReasoningEffort::Low,
                ))],
                "medium" => vec![AssistantEffect::SetSessionEffort(Some(
                    ReasoningEffort::Medium,
                ))],
                "high" => vec![AssistantEffect::SetSessionEffort(Some(
                    ReasoningEffort::High,
                ))],
                _ => {
                    self.turns.push(Turn::now(
                        TurnRole::Error,
                        format!(
                            "Invalid effort '{}'. Expected auto | low | medium | high.",
                            cmd.args
                        ),
                    ));
                    vec![AssistantEffect::SessionWrite {
                        conversation_id: self.conversation_id.clone(),
                    }]
                }
            },
            "revoke" if cmd.args.is_empty() => vec![AssistantEffect::OpenPermissionsManager],
            "revoke" => vec![AssistantEffect::RevokeGrant {
                target_id: cmd.args.clone(),
            }],
            name => vec![AssistantEffect::InvokeSkill {
                name: name.to_string(),
                args: cmd.args.clone(),
            }],
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

    pub fn push_error(&mut self, text: impl Into<String>) -> Vec<AssistantEffect> {
        self.turns.push(Turn::now(TurnRole::Error, text));
        vec![AssistantEffect::SessionWrite {
            conversation_id: self.conversation_id.clone(),
        }]
    }

    /// A tool call started running inside the in-flight turn. Any answer
    /// text streamed so far commits as its own transcript row first, so the
    /// tool row lands *below* the text that preceded it — the transcript
    /// stays chronological instead of the whole reply committing at turn end
    /// above nothing and below every tool call (stint 0455).
    pub fn tool_call_started(&mut self, tool: &str, input_summary: &str) {
        self.flush_streamed_segment();
        self.active_tools.push(ActiveToolCall {
            tool: tool.to_string(),
            input_summary: input_summary.to_string(),
        });
    }

    /// Commit the streamed-but-uncommitted answer text as an Assistant turn.
    /// No-op outside an in-flight turn or when nothing has streamed.
    fn flush_streamed_segment(&mut self) {
        if !self.streaming.in_flight || self.streaming.partial_answer.is_empty() {
            return;
        }
        let segment = std::mem::take(&mut self.streaming.partial_answer);
        self.streaming.committed_answer.push_str(&segment);
        // `committed_answer` tracks the exact streamed text for the
        // prefix-strip in `finish_turn`; the transcript row gets a trimmed
        // copy so inter-segment newlines don't render as blank bubbles.
        let display = segment.trim();
        if display.is_empty() {
            // Whitespace-only segment: nothing to show. Any streamed
            // reasoning stays pending and attaches to the next commit.
            return;
        }
        let mut turn = Turn::now(TurnRole::Assistant, display);
        if !self.streaming.partial_reasoning.is_empty() {
            turn.thoughts = Some(std::mem::take(&mut self.streaming.partial_reasoning));
        }
        self.push_flight_turn(turn);
    }

    /// A tool call finished: drop its running row and append a completed
    /// tool turn. Returns the persistence effect.
    pub fn tool_call_finished(&mut self, call: FinishedToolCall) -> Vec<AssistantEffect> {
        if let Some(pos) = self.active_tools.iter().position(|t| t.tool == call.tool) {
            self.active_tools.remove(pos);
        }
        let mut turn = match &call.error {
            None => Turn::tool(call.tool.clone(), ToolStatus::Succeeded),
            Some(e) => Turn::tool(format!("{} — {e}", call.tool), ToolStatus::Failed),
        };
        turn.detail = call.detail;
        turn.input_summary = call.input_summary;
        turn.output_preview = call.output_preview;
        self.push_flight_turn(turn);
        vec![AssistantEffect::SessionWrite {
            conversation_id: self.conversation_id.clone(),
        }]
    }

    /// The in-flight turn hit an ask-gated tool: show the permission sheet.
    /// The cursor starts on `Deny` (`ORDER`'s last slot) — the safe default
    /// if the user reflexively hits Enter without looking.
    pub fn permission_requested(&mut self, tool: &str, input_summary: &str) {
        self.pending_permission = Some(PendingPermission {
            tool: tool.to_string(),
            input_summary: input_summary.to_string(),
            selected: PermissionChoice::ORDER.len() - 1,
        });
    }

    /// Move the permission sheet's keyboard cursor one action right,
    /// wrapping from the last action back to the first.
    pub fn permission_move_next(&mut self) {
        if let Some(pending) = self.pending_permission.as_mut() {
            pending.selected = (pending.selected + 1) % PermissionChoice::ORDER.len();
        }
    }

    /// Move the permission sheet's keyboard cursor one action left,
    /// wrapping from the first action back to the last.
    pub fn permission_move_prev(&mut self) {
        if let Some(pending) = self.pending_permission.as_mut() {
            let len = PermissionChoice::ORDER.len();
            pending.selected = (pending.selected + len - 1) % len;
        }
    }

    /// The action the permission sheet's keyboard cursor currently sits on.
    pub fn permission_selected_choice(&self) -> Option<PermissionChoice> {
        self.pending_permission
            .as_ref()
            .map(|pending| PermissionChoice::ORDER[pending.selected])
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
                self.push_flight_turn(Turn::tool(
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
            // Touch nothing: `interrupt_turn` already reset all streaming
            // state when the conversation switched, so any in-flight state
            // now belongs to a newer turn this late outcome must not clobber.
            log::info!(
                "assistant: dropping turn outcome for stale conversation {conversation_id} (active {})",
                self.conversation_id
            );
            return Vec::new();
        }
        match outcome {
            Ok(text) => {
                log::info!(
                    "assistant[{}]: turn end ({} chars, {} committed mid-turn, {} reasoning chars)",
                    self.conversation_id,
                    text.len(),
                    self.streaming.committed_answer.len(),
                    self.streaming.partial_reasoning.len()
                );
                // Segments flushed when tool calls started (stint 0455) are
                // already in the transcript — commit only the remaining tail
                // of the broker's final text. A prefix mismatch means the
                // final text diverged from the streamed deltas; fall back to
                // the full text so nothing the model said is lost.
                let tail = match text.strip_prefix(self.streaming.committed_answer.as_str()) {
                    Some(tail) => tail.to_string(),
                    None => {
                        if !self.streaming.committed_answer.is_empty() {
                            log::warn!(
                                "assistant[{}]: final text does not start with the {} chars committed mid-turn; committing full text",
                                self.conversation_id,
                                self.streaming.committed_answer.len()
                            );
                        }
                        text
                    }
                };
                // An empty tail with no reasoning means nothing new was
                // produced — e.g. a turn cancelled before any text streamed,
                // or a reply that ended on a tool call. Don't commit an
                // empty assistant bubble.
                let tail = tail.trim();
                if !tail.is_empty() || !self.streaming.partial_reasoning.is_empty() {
                    let mut turn = Turn::now(TurnRole::Assistant, tail);
                    if !self.streaming.partial_reasoning.is_empty() {
                        turn.thoughts = Some(std::mem::take(&mut self.streaming.partial_reasoning));
                    }
                    self.push_flight_turn(turn);
                }
            }
            Err(e) => {
                log::warn!("assistant[{}]: turn failed: {e}", self.conversation_id);
                self.push_flight_turn(Turn::now(TurnRole::Error, e));
            }
        }
        self.streaming = StreamingState::default();
        self.active_tools.clear();
        self.pending_permission = None;
        self.turn_anchor = None;
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

    fn tool_started(m: &mut AssistantModel, tool: &str) {
        m.tool_call_started(tool, "{}");
    }

    fn tool_finished(
        m: &mut AssistantModel,
        tool: &str,
        error: Option<String>,
        detail: Option<String>,
    ) -> Vec<AssistantEffect> {
        m.tool_call_finished(FinishedToolCall {
            tool: tool.to_string(),
            error,
            detail,
            input_summary: Some("{}".to_string()),
            output_preview: None,
        })
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

        // View commands execute immediately mid-turn without touching the
        // in-flight stream.
        let effects = submitted(&mut m, "/help");
        assert!(m.composer.is_empty());
        assert!(m.streaming.in_flight, "in-flight turn unaffected by /help");
        assert_eq!(m.turns.last().unwrap().role, TurnRole::Assistant);
        assert!(m.turns.last().unwrap().text.contains("/clear"));
        assert!(matches!(effects[0], AssistantEffect::SessionWrite { .. }));
    }

    #[test]
    fn up_history_recalls_previous_user_turns_without_overwriting_draft() {
        let mut m = AssistantModel::fresh();
        submitted(&mut m, "first");
        m.finish_turn(&m.conversation_id.clone(), Ok("one".to_string()));
        submitted(&mut m, "second");
        m.finish_turn(&m.conversation_id.clone(), Ok("two".to_string()));

        assert!(m.recall_previous_user_message());
        assert_eq!(m.composer, "second");
        assert!(m.recall_previous_user_message());
        assert_eq!(m.composer, "first");
        assert!(m.recall_previous_user_message());
        assert_eq!(m.composer, "first", "history clamps at oldest entry");

        m.composer = "draft".to_string();
        m.reset_history_recall();
        assert!(!m.recall_previous_user_message());
        assert_eq!(m.composer, "draft");
    }

    #[test]
    fn clear_mid_turn_interrupts_and_cancels() {
        let mut m = AssistantModel::fresh();
        submitted(&mut m, "question");
        let old_id = m.conversation_id.clone();
        assert!(m.streaming.in_flight);
        tool_started(&mut m, "csv.read_range");
        m.permission_requested("csv.write_range", "{}");

        let effects = submitted(&mut m, "/clear");
        assert!(effects.contains(&AssistantEffect::CancelTurn));
        assert!(!m.streaming.in_flight);
        assert!(m.active_tools.is_empty());
        assert!(m.pending_permission.is_none());
        assert_eq!(m.queued_user_turns, 0);
        assert_ne!(m.conversation_id, old_id);
        assert!(m.turns.is_empty());

        // The interrupted worker's late outcome lands stale and is dropped.
        assert!(m.finish_turn(&old_id, Ok("late".to_string())).is_empty());
        assert!(m.turns.is_empty());
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
        assert!(
            matches!(&effects[0], AssistantEffect::SessionWrite { conversation_id } if *conversation_id == id)
        );
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
    fn mid_turn_command_output_lands_below_the_final_reply() {
        let mut m = AssistantModel::fresh();
        submitted(&mut m, "hi");
        // View command runs while the turn is still in flight.
        submitted(&mut m, "/help");
        let id = m.conversation_id.clone();
        m.finish_turn(&id, Ok("hello there".to_string()));
        assert_eq!(m.turns[0].text, "hi");
        assert_eq!(
            m.turns[1].text, "hello there",
            "reply commits at its anchor"
        );
        assert!(m.turns[2].text.contains("Built-in commands"));
        assert_eq!(m.turn_anchor, None);
    }

    #[test]
    fn tool_rows_stay_with_their_turn_above_mid_turn_output() {
        let mut m = AssistantModel::fresh();
        submitted(&mut m, "do it");
        tool_started(&mut m, "csv.read_range");
        tool_finished(&mut m, "csv.read_range", None, None);
        submitted(&mut m, "/help");
        let id = m.conversation_id.clone();
        m.finish_turn(&id, Ok("done".to_string()));
        assert_eq!(m.turns[0].role, TurnRole::User);
        assert_eq!(m.turns[1].role, TurnRole::Tool);
        assert_eq!(m.turns[2].text, "done");
        assert!(m.turns[3].text.contains("Built-in commands"));
    }

    #[test]
    fn queued_message_stays_below_the_reply_it_followed() {
        let mut m = AssistantModel::fresh();
        submitted(&mut m, "first");
        submitted(&mut m, "second");
        let id = m.conversation_id.clone();
        m.finish_turn(&id, Ok("reply one".to_string()));
        let texts: Vec<&str> = m.turns.iter().map(|t| t.text.as_str()).collect();
        assert_eq!(texts, ["first", "reply one", "second"]);
    }

    #[test]
    fn stale_outcome_does_not_clobber_a_newer_turn() {
        let mut m = AssistantModel::fresh();
        submitted(&mut m, "q1");
        let old_id = m.conversation_id.clone();
        submitted(&mut m, "/clear");
        submitted(&mut m, "q2");
        m.apply_answer_delta("fresh");
        let effects = m.finish_turn(&old_id, Ok("late".to_string()));
        assert!(effects.is_empty());
        assert!(m.streaming.in_flight, "newer turn's streaming must survive");
        assert_eq!(m.streaming.partial_answer, "fresh");
        assert_eq!(m.turns.len(), 1, "only q2's user row");
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
        assert!(
            m.turns.is_empty(),
            "stale outcome must not land in the cleared conversation"
        );
    }

    #[test]
    fn clear_starts_fresh_conversation_with_new_id() {
        let mut m = AssistantModel::fresh();
        m.active_agent_id = "writer".to_string();
        m.effort_override = Some(ReasoningEffort::High);
        let old_id = m.conversation_id.clone();
        submitted(&mut m, "remember this");
        m.streaming = StreamingState::default();
        let effects = submitted(&mut m, "/clear");
        assert_ne!(m.conversation_id, old_id);
        assert!(m.turns.is_empty());
        assert_eq!(m.active_agent_id, "writer");
        assert_eq!(m.effort_override, Some(ReasoningEffort::High));
        assert!(
            matches!(&effects[0], AssistantEffect::SessionWrite { conversation_id }
            if *conversation_id == m.conversation_id)
        );
    }

    #[test]
    fn new_creates_named_conversation() {
        let mut m = AssistantModel::fresh();
        m.active_agent_id = "writer".to_string();
        m.effort_override = Some(ReasoningEffort::High);
        let old_id = m.conversation_id.clone();
        submitted(&mut m, "/new project notes");
        assert_ne!(m.conversation_id, old_id);
        assert_eq!(m.turns.len(), 1);
        assert!(m.turns[0].text.contains("project notes"));
        assert_eq!(m.active_agent_id, "writer");
        assert_eq!(m.effort_override, Some(ReasoningEffort::High));
    }

    #[test]
    fn help_lists_builtins() {
        let mut m = AssistantModel::fresh();
        submitted(&mut m, "/help");
        assert_eq!(m.turns.len(), 1);
        assert!(m.turns[0].text.contains("/clear"));
    }

    #[test]
    fn skills_context_and_hooks_route_to_real_effects() {
        let mut m = AssistantModel::fresh();
        assert_eq!(
            submitted(&mut m, "/skills"),
            vec![AssistantEffect::ListSkills]
        );
        assert_eq!(
            submitted(&mut m, "/context"),
            vec![AssistantEffect::ShowContext]
        );
        assert_eq!(
            submitted(&mut m, "/hooks"),
            vec![AssistantEffect::ShowHooks]
        );
    }

    #[test]
    fn phase2_views_route_to_their_effects() {
        let mut m = AssistantModel::fresh();
        assert_eq!(
            submitted(&mut m, "/tools"),
            vec![AssistantEffect::ListTools]
        );
        assert_eq!(
            submitted(&mut m, "/permissions"),
            vec![AssistantEffect::OpenPermissionsManager]
        );
        assert_eq!(
            submitted(&mut m, "/audit"),
            vec![AssistantEffect::ShowAudit]
        );
        assert_eq!(
            submitted(&mut m, "/revoke app.csv.write_range"),
            vec![AssistantEffect::RevokeGrant {
                target_id: "app.csv.write_range".to_string()
            }]
        );
        // Bare /revoke opens the permissions manager (same as /permissions),
        // not a usage error.
        assert_eq!(
            submitted(&mut m, "/revoke"),
            vec![AssistantEffect::OpenPermissionsManager]
        );
        assert!(m.turns.is_empty(), "views answer via the pane shell");
    }

    #[test]
    fn settings_and_config_commands_are_equivalent() {
        let mut settings_model = AssistantModel::fresh();
        let mut config_model = AssistantModel::fresh();

        let settings_effects = submitted(&mut settings_model, "/settings");
        let config_effects = submitted(&mut config_model, "/config");

        assert_eq!(settings_effects, config_effects);
        assert_eq!(settings_effects, vec![AssistantEffect::ShowSettings]);
    }

    #[test]
    fn agent_and_effort_commands_emit_typed_effects() {
        let mut model = AssistantModel::fresh();
        assert_eq!(
            submitted(&mut model, "/agent"),
            vec![AssistantEffect::ListAgents]
        );
        assert_eq!(
            submitted(&mut model, "/agent writer"),
            vec![AssistantEffect::SwitchAgent("writer".to_string())]
        );
        assert_eq!(
            submitted(&mut model, "/agent inspect writer"),
            vec![AssistantEffect::InspectAgent("writer".to_string())]
        );
        assert_eq!(
            submitted(&mut model, "/agent create writer"),
            vec![AssistantEffect::CreateAgent("writer".to_string())]
        );
        assert_eq!(
            submitted(&mut model, "/agent edit writer"),
            vec![AssistantEffect::EditAgent("writer".to_string())]
        );
        assert_eq!(
            submitted(&mut model, "/effort high"),
            vec![AssistantEffect::SetSessionEffort(Some(
                ReasoningEffort::High
            ))]
        );
        assert_eq!(
            submitted(&mut model, "/effort auto"),
            vec![AssistantEffect::SetSessionEffort(None)]
        );
    }

    #[test]
    fn model_without_args_opens_the_picker() {
        let mut model = AssistantModel::fresh();

        let effects = submitted(&mut model, "/model");

        assert_eq!(effects, vec![AssistantEffect::OpenModelPicker]);
    }

    #[test]
    fn model_picker_overlay_navigates_and_cancels() {
        let mut model = AssistantModel::fresh();
        model.open_model_picker(
            ModelTier::Medium,
            vec![ModelTier::Low, ModelTier::Medium, ModelTier::High],
            vec![AgentChoice {
                id: "default".to_string(),
                display_name: "Plexi Assistant".to_string(),
            }],
        );
        // Cursor lands on the current tier (medium = index 1).
        assert_eq!(model.overlay_selected(), 1);
        assert_eq!(model.overlay_len(), 4); // 3 tiers + 1 agent

        model.overlay_move_down();
        assert_eq!(model.overlay_selected(), 2); // high
        model.overlay_move_down();
        assert_eq!(model.overlay_selected(), 3); // agent row
        model.overlay_move_down();
        assert_eq!(model.overlay_selected(), 3, "clamps at the last row");

        model.overlay_move_up();
        model.overlay_move_up();
        model.overlay_move_up();
        model.overlay_move_up();
        assert_eq!(model.overlay_selected(), 0, "clamps at the first row");

        model.cancel_overlay();
        assert!(!model.overlay_active());
    }

    #[test]
    fn permissions_overlay_cycles_decisions() {
        use crate::broker::Decision;
        let mut model = AssistantModel::fresh();
        model.open_permissions_manager(vec![
            GrantRow {
                target_id: "app.a.read".to_string(),
                decision: Decision::Allow,
            },
            GrantRow {
                target_id: "app.b.write".to_string(),
                decision: Decision::Ask,
            },
        ]);
        assert_eq!(model.overlay_selected(), 0);

        // Space cycles the selected row: Allow -> Ask -> Deny -> Allow.
        model.overlay_cycle_decision();
        model.overlay_cycle_decision();
        let AssistantOverlay::PermissionsManager { grants, .. } = &model.overlay else {
            panic!("expected permissions manager");
        };
        assert_eq!(grants[0].decision, Decision::Deny);
        assert_eq!(grants[1].decision, Decision::Ask, "other rows untouched");

        model.overlay_move_down();
        model.overlay_cycle_decision();
        let AssistantOverlay::PermissionsManager { grants, .. } = &model.overlay else {
            panic!("expected permissions manager");
        };
        assert_eq!(grants[1].decision, Decision::Deny);
    }

    #[test]
    fn model_with_valid_tier_requests_a_session_override() {
        let mut model = AssistantModel::fresh();

        let effects = submitted(&mut model, "/model high");

        assert_eq!(
            effects,
            vec![AssistantEffect::SetSessionModel(
                crate::app_protocol::ModelTier::High
            )]
        );
    }

    #[test]
    fn model_rejects_invalid_tier_without_requesting_a_mutation() {
        let mut model = AssistantModel::fresh();

        let effects = submitted(&mut model, "/model turbo");

        assert!(
            !effects
                .iter()
                .any(|effect| matches!(effect, AssistantEffect::SetSessionModel(_))),
            "invalid tier must not request a settings mutation: {effects:?}"
        );
        assert_eq!(model.turns.last().unwrap().role, TurnRole::Error);
        assert!(model
            .turns
            .last()
            .unwrap()
            .text
            .contains("low | medium | high"));
    }

    #[test]
    fn unknown_command_routes_to_skill_registry_fallback() {
        let mut m = AssistantModel::fresh();
        assert_eq!(
            submitted(&mut m, "/frobnicate now"),
            vec![AssistantEffect::InvokeSkill {
                name: "frobnicate".to_string(),
                args: "now".to_string(),
            }]
        );
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

        tool_started(&mut m, "csv.read_range");
        assert_eq!(m.active_tools.len(), 1);
        assert_eq!(m.active_tools[0].tool, "csv.read_range");

        let effects = tool_finished(&mut m, "csv.read_range", None, None);
        assert!(m.active_tools.is_empty());
        let row = m.turns.last().unwrap();
        assert_eq!(row.role, TurnRole::Tool);
        assert_eq!(row.status, Some(ToolStatus::Succeeded));
        assert!(matches!(&effects[0], AssistantEffect::SessionWrite { .. }));

        tool_started(&mut m, "csv.write_range");
        tool_finished(&mut m, "csv.write_range", Some("tool_timeout".to_string()), None);
        let row = m.turns.last().unwrap();
        assert_eq!(row.status, Some(ToolStatus::Failed));
        assert!(row.text.contains("tool_timeout"));
    }

    /// Stint 0421: a render payload (file-edit diff) rides on the completed
    /// tool turn and survives serde.
    #[test]
    fn tool_turn_detail_attaches_and_round_trips() {
        let mut m = AssistantModel::fresh();
        submitted(&mut m, "edit the app");
        tool_started(&mut m, "host.files.edit");
        tool_finished(
            &mut m,
            "host.files.edit",
            None,
            Some("--- a/x\n+++ b/x\n".to_string()),
        );
        let diff_turn = m.turns.last().unwrap();
        assert_eq!(diff_turn.detail.as_deref(), Some("--- a/x\n+++ b/x\n"));
        let json = serde_json::to_string(diff_turn).unwrap();
        let back: Turn = serde_json::from_str(&json).unwrap();
        assert_eq!(back.detail, diff_turn.detail);
    }

    /// Stint 0416: even when a follow-up message is queued mid-turn while
    /// several tool calls complete, every row must land in strict
    /// chronological order — both tool calls before the reply they belong
    /// to, and the queued message after it. Tool calls must never end up
    /// stacked above (or after) the wrong turn's message.
    #[test]
    fn tool_calls_and_queued_message_stay_in_chronological_order() {
        let mut m = AssistantModel::fresh();
        submitted(&mut m, "build me an app");

        tool_started(&mut m, "host.files.write");
        tool_finished(&mut m, "host.files.write", None, None);

        // A follow-up sent while the turn is still streaming is queued, not
        // dispatched immediately.
        submitted(&mut m, "also add a footer");
        assert_eq!(m.queued_user_turns, 1);

        tool_started(&mut m, "plexi.app_check");
        tool_finished(&mut m, "plexi.app_check", None, None);

        let id = m.conversation_id.clone();
        m.finish_turn(&id, Ok("Done — app scaffolded.".to_string()));

        let roles: Vec<_> = m.turns.iter().map(|t| (t.role, t.text.clone())).collect();
        assert_eq!(
            roles,
            vec![
                (TurnRole::User, "build me an app".to_string()),
                (TurnRole::Tool, "host.files.write".to_string()),
                (TurnRole::Tool, "plexi.app_check".to_string()),
                (TurnRole::Assistant, "Done — app scaffolded.".to_string()),
                (TurnRole::User, "also add a footer".to_string()),
            ],
            "tool calls and the reply must commit before the queued follow-up, in the order they happened"
        );
    }

    /// Stint 0377: the permission sheet's keyboard cursor starts on Deny
    /// (the safe default), Tab/arrow-right cycles forward with wraparound,
    /// and Shift-Tab/arrow-left cycles backward with wraparound.
    #[test]
    fn permission_sheet_keyboard_cursor_cycles_with_wraparound() {
        let mut m = AssistantModel::fresh();
        m.permission_requested("csv.write_range", "{}");
        assert_eq!(
            m.permission_selected_choice(),
            Some(PermissionChoice::Deny),
            "cursor starts on the safe default"
        );

        m.permission_move_next();
        assert_eq!(
            m.permission_selected_choice(),
            Some(PermissionChoice::AllowOnce),
            "wraps forward from the last action to the first"
        );

        m.permission_move_prev();
        assert_eq!(m.permission_selected_choice(), Some(PermissionChoice::Deny));

        m.permission_move_prev();
        assert_eq!(
            m.permission_selected_choice(),
            Some(PermissionChoice::AllowAlways),
            "wraps backward from the first action to the last"
        );

        m.permission_move_next();
        m.permission_move_next();
        assert_eq!(
            m.permission_selected_choice(),
            Some(PermissionChoice::AllowOnce)
        );
    }

    #[test]
    fn permission_sheet_cursor_navigation_is_noop_with_nothing_pending() {
        let mut m = AssistantModel::fresh();
        assert!(m.pending_permission.is_none());
        m.permission_move_next();
        m.permission_move_prev();
        assert!(m.permission_selected_choice().is_none());
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
        tool_started(&mut m, "csv.read_range");
        m.permission_requested("csv.write_range", "{}");
        let id = m.conversation_id.clone();
        m.finish_turn(&id, Err("worker died".to_string()));
        assert!(m.active_tools.is_empty());
        assert!(m.pending_permission.is_none());
    }

    #[test]
    fn finish_turn_attaches_streamed_thoughts() {
        let mut m = AssistantModel::fresh();
        submitted(&mut m, "q");
        m.apply_reasoning_delta("step 1, ");
        m.apply_reasoning_delta("step 2");
        let id = m.conversation_id.clone();
        m.finish_turn(&id, Ok("answer".to_string()));
        let turn = m.turns.last().unwrap();
        assert_eq!(turn.thoughts.as_deref(), Some("step 1, step 2"));

        // No reasoning streamed → no thoughts attached.
        submitted(&mut m, "q2");
        m.finish_turn(&id, Ok("answer 2".to_string()));
        assert_eq!(m.turns.last().unwrap().thoughts, None);
    }

    #[test]
    fn thoughts_command_toggles_and_persists() {
        let mut m = AssistantModel::fresh();
        assert!(!m.show_thoughts);
        let effects = submitted(&mut m, "/thoughts");
        assert!(m.show_thoughts);
        assert!(effects.contains(&AssistantEffect::PersistShowThoughts(true)));
        assert!(m.turns.last().unwrap().text.contains("shown"));

        let effects = submitted(&mut m, "/thoughts");
        assert!(!m.show_thoughts);
        assert!(effects.contains(&AssistantEffect::PersistShowThoughts(false)));
        assert!(m.turns.last().unwrap().text.contains("hidden"));
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

    /// Stint 0455: answer text streamed before a tool call commits as its
    /// own Assistant turn the moment the tool starts, so the transcript
    /// reads chronologically — text, then the tool row below it, then the
    /// final reply as a separate bubble.
    #[test]
    fn streamed_text_flushes_before_tool_rows_and_tail_commits_at_finish() {
        let mut m = AssistantModel::fresh();
        submitted(&mut m, "build the app");

        m.apply_answer_delta("Let me build this for you.");
        m.apply_reasoning_delta("plan the scaffold");
        tool_started(&mut m, "host.terminals.run");
        assert_eq!(
            m.turns.last().unwrap().role,
            TurnRole::Assistant,
            "streamed segment commits when the tool call starts"
        );
        assert_eq!(m.turns.last().unwrap().text, "Let me build this for you.");
        assert_eq!(
            m.turns.last().unwrap().thoughts.as_deref(),
            Some("plan the scaffold"),
            "reasoning streamed before the flush rides on the segment"
        );
        tool_finished(&mut m, "host.terminals.run", None, None);

        m.apply_answer_delta("\n\nNow the code.");
        tool_started(&mut m, "host.files.write");
        tool_finished(&mut m, "host.files.write", None, None);

        let id = m.conversation_id.clone();
        m.finish_turn(
            &id,
            Ok("Let me build this for you.\n\nNow the code.\n\nDone — try it!".to_string()),
        );

        let rows: Vec<_> = m.turns.iter().map(|t| (t.role, t.text.clone())).collect();
        assert_eq!(
            rows,
            vec![
                (TurnRole::User, "build the app".to_string()),
                (TurnRole::Assistant, "Let me build this for you.".to_string()),
                (TurnRole::Tool, "host.terminals.run".to_string()),
                (TurnRole::Assistant, "Now the code.".to_string()),
                (TurnRole::Tool, "host.files.write".to_string()),
                (TurnRole::Assistant, "Done — try it!".to_string()),
            ],
            "segments, tool rows, and the tail commit in chronological order"
        );
        assert_eq!(m.streaming, StreamingState::default());
    }

    /// Stint 0455: when the broker's final text diverges from the streamed
    /// deltas, the full text commits (nothing the model said is lost), at
    /// the cost of repeating the flushed segments.
    #[test]
    fn finish_turn_prefix_mismatch_falls_back_to_full_text() {
        let mut m = AssistantModel::fresh();
        submitted(&mut m, "q");
        m.apply_answer_delta("streamed prefix");
        tool_started(&mut m, "host.files.read");
        tool_finished(&mut m, "host.files.read", None, None);
        let id = m.conversation_id.clone();
        m.finish_turn(&id, Ok("completely different final".to_string()));
        assert_eq!(m.turns.last().unwrap().text, "completely different final");
    }

    /// Stint 0455: a reply that ends on a tool call (no text after it)
    /// commits no trailing empty bubble.
    #[test]
    fn finish_turn_with_fully_flushed_text_commits_no_empty_bubble() {
        let mut m = AssistantModel::fresh();
        submitted(&mut m, "q");
        m.apply_answer_delta("Working on it.");
        tool_started(&mut m, "host.files.write");
        tool_finished(&mut m, "host.files.write", None, None);
        let id = m.conversation_id.clone();
        m.finish_turn(&id, Ok("Working on it.".to_string()));
        let rows: Vec<_> = m.turns.iter().map(|t| (t.role, t.text.clone())).collect();
        assert_eq!(
            rows,
            vec![
                (TurnRole::User, "q".to_string()),
                (TurnRole::Assistant, "Working on it.".to_string()),
                (TurnRole::Tool, "host.files.write".to_string()),
            ],
            "no empty assistant bubble after the last tool row"
        );
    }

    /// Stint 0455: the caret-dropdown payloads ride on the tool turn and
    /// survive serde; rows persisted before the fields existed still load.
    #[test]
    fn tool_turn_dropdown_payloads_round_trip() {
        let mut m = AssistantModel::fresh();
        submitted(&mut m, "q");
        m.tool_call_started("host.files.grep", r#"{"pattern": "draft"}"#);
        assert_eq!(m.active_tools[0].input_summary, r#"{"pattern": "draft"}"#);
        m.tool_call_finished(FinishedToolCall {
            tool: "host.files.grep".to_string(),
            error: None,
            detail: None,
            input_summary: Some(r#"{"pattern": "draft"}"#.to_string()),
            output_preview: Some(r#"{"matches": 3}"#.to_string()),
        });
        let row = m.turns.last().unwrap();
        assert_eq!(row.input_summary.as_deref(), Some(r#"{"pattern": "draft"}"#));
        assert_eq!(row.output_preview.as_deref(), Some(r#"{"matches": 3}"#));
        let json = serde_json::to_string(row).unwrap();
        let back: Turn = serde_json::from_str(&json).unwrap();
        assert_eq!(back.input_summary, row.input_summary);
        assert_eq!(back.output_preview, row.output_preview);
        let legacy: Turn = serde_json::from_str(
            r#"{"role":"tool","text":"csv.read_range","created_at":"2026-01-01T00:00:00Z","status":"succeeded"}"#,
        )
        .unwrap();
        assert_eq!(legacy.input_summary, None);
        assert_eq!(legacy.output_preview, None);
    }

    #[test]
    fn conversation_commands_emit_store_backed_effects() {
        let mut model = AssistantModel::fresh();
        assert_eq!(
            submitted(&mut model, "/resume"),
            vec![AssistantEffect::ListConversations]
        );
        assert_eq!(
            submitted(&mut model, "/resume 2"),
            vec![AssistantEffect::ResumeConversation("2".to_string())]
        );
        assert_eq!(
            submitted(&mut model, "/history"),
            vec![AssistantEffect::ShowHistory]
        );
        assert_eq!(
            submitted(&mut model, "/rewind turn:2"),
            vec![AssistantEffect::RewindConversation("turn:2".to_string())]
        );
        assert_eq!(
            submitted(&mut model, "/compact"),
            vec![AssistantEffect::CompactConversation]
        );
        assert_eq!(model.compaction, CompactionState::Compacting);
        assert_eq!(
            submitted(&mut model, "/export"),
            vec![AssistantEffect::ExportConversation]
        );
    }

    #[test]
    fn rewind_requires_an_explicit_selector() {
        let mut model = AssistantModel::fresh();
        let effects = submitted(&mut model, "/rewind");
        assert!(matches!(
            effects.as_slice(),
            [AssistantEffect::SessionWrite { .. }]
        ));
        assert!(model.turns.last().unwrap().text.contains("Usage: /rewind"));
    }
}
