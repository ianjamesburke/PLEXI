//! Pane lifecycle facts carried on the existing app event timeline.
use crate::protocol::{AgentBlockedReason, AgentState};
use schemars::JsonSchema;
use serde::Serialize;

pub(crate) const PUBLISHER: &str = "plexi.host.panes";
pub(crate) const STREAM: &str = "pane.lifecycle";

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Source {
    Hook,
    LegacyReport,
    HostObservation,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub(crate) struct Provenance {
    pub source: Source,
    /// Human-facing provider label, never an entity key.
    pub agent_label: String,
    pub session_id: Option<String>,
    pub raw_event: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ExitStatus {
    /// The PTY adapter does not currently carry a kernel exit status.
    Unknown,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum PaneLifecycleEvent {
    Spawned,
    AgentBooted { provenance: Provenance },
    AgentIdle { provenance: Provenance },
    AgentWorking { provenance: Provenance },
    AgentBlocked { reason: AgentBlockedReason, provenance: Provenance },
    TurnFinished { provenance: Provenance },
    TurnFailed { provenance: Provenance },
    SessionStarted { provenance: Provenance },
    SessionEnded { provenance: Provenance },
    AgentReported { provenance: Provenance },
    SlotChanged { name: String, value: Vec<u8> },
    Exited { status: ExitStatus },
}

#[derive(Debug, Default)]
pub(crate) struct PaneLifecycleState {
    pub context_id: u64,
    pub booted: bool,
}

impl PaneLifecycleEvent {
    pub(crate) fn from_report(
        state: &AgentState,
        provenance: Provenance,
        reason: Option<AgentBlockedReason>,
    ) -> Self {
        match provenance.raw_event.as_deref() {
            Some("Stop" | "agent_end") => Self::TurnFinished { provenance },
            Some("StopFailure") => Self::TurnFailed { provenance },
            Some("SessionEnd" | "session_shutdown") => Self::SessionEnded { provenance },
            Some("SessionStart" | "session_start") => Self::SessionStarted { provenance },
            Some("PermissionRequest") => Self::AgentBlocked {
                reason: AgentBlockedReason::PermissionPrompt, provenance,
            },
            Some("UsageLimit") => Self::AgentBlocked {
                reason: AgentBlockedReason::UsageLimit, provenance,
            },
            Some("BootFailure") => Self::AgentBlocked {
                reason: AgentBlockedReason::BootFailure, provenance,
            },
            Some("UserPromptSubmit" | "agent_start" | "before_agent_start" | "PreToolUse" |
                "tool_call" | "PostToolUse" | "PostToolBatch" | "tool_result") =>
                Self::AgentWorking { provenance },
            Some(_) => Self::AgentReported { provenance },
            None => match state {
                AgentState::Idle => Self::AgentIdle { provenance },
                AgentState::Working => Self::AgentWorking { provenance },
                AgentState::Blocked => Self::AgentBlocked {
                    reason: reason.unwrap_or(AgentBlockedReason::Unknown), provenance,
                },
            },
        }
    }
}
