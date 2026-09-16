//! Pane lifecycle facts carried on the existing app event timeline.
//!
//! Publisher `plexi.host.panes` is reserved to the host. The `pane.lifecycle`
//! stream uses schema_version=1, pane/context IDs, and the tagged payload below.
//! Subscriptions retain the bus's context and resource filters. Pane IDs are
//! temporary handles; provider labels are display attributes, not agent IDs.
//! Hook/legacy reports are claims, while detector and PTY events are host
//! observations. A normal turn stop is not a task-completion verdict, and a
//! provider failure is not an OS crash. PTY status is explicitly unknown until
//! the adapter carries it. Slot values are lossless byte arrays after the write.
//! Records have the timeline's in-memory lifetime; this is not a durable replay
//! or restart contract. Wait/follow CLI consumers belong to the remaining stint.
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
    AgentBooted {
        provenance: Provenance,
    },
    AgentIdle {
        provenance: Provenance,
    },
    AgentWorking {
        provenance: Provenance,
    },
    AgentBlocked {
        reason: AgentBlockedReason,
        provenance: Provenance,
    },
    TurnFinished {
        provenance: Provenance,
    },
    TurnFailed {
        provenance: Provenance,
    },
    SessionStarted {
        provenance: Provenance,
    },
    SessionEnded {
        provenance: Provenance,
    },
    AgentReported {
        provenance: Provenance,
    },
    SlotChanged {
        name: String,
        value: Vec<u8>,
    },
    Exited {
        status: ExitStatus,
    },
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
                reason: AgentBlockedReason::PermissionPrompt,
                provenance,
            },
            Some("UsageLimit") => Self::AgentBlocked {
                reason: AgentBlockedReason::UsageLimit,
                provenance,
            },
            Some("BootFailure") => Self::AgentBlocked {
                reason: AgentBlockedReason::BootFailure,
                provenance,
            },
            Some(
                "UserPromptSubmit" | "agent_start" | "before_agent_start" | "PreToolUse"
                | "tool_call" | "PostToolUse" | "PostToolBatch" | "tool_result",
            ) => Self::AgentWorking { provenance },
            Some(_) => Self::AgentReported { provenance },
            None => match state {
                AgentState::Idle => Self::AgentIdle { provenance },
                AgentState::Working => Self::AgentWorking { provenance },
                AgentState::Blocked => Self::AgentBlocked {
                    reason: reason.unwrap_or(AgentBlockedReason::Unknown),
                    provenance,
                },
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broker::{ActorType, GrantDuration};
    use crate::host::app_timeline::{AppTimeline, EmittedEvent, SubscriptionRecord};
    use crate::protocol::{AppEventActor, EventStreamDecl, PayloadMode, TriggerMode};

    #[test]
    fn pane_lifecycle_uses_scoped_resource_filtered_bus_and_reserves_publisher() {
        let mut timeline = AppTimeline::default();
        timeline
            .record_pane_lifecycle(41, 7, &PaneLifecycleEvent::Spawned)
            .unwrap();
        for (name, context, resource) in [
            ("same-pane", 41, "7"),
            ("other-pane", 41, "8"),
            ("other-context", 42, "7"),
        ] {
            timeline.add_subscription(SubscriptionRecord {
                subscription_id: name.into(),
                subscriber_id: name.into(),
                subscriber_type: ActorType::Agent,
                app_id: PUBLISHER.into(),
                event_names: vec![STREAM.into()],
                resource_id: Some(resource.into()),
                subscriber_context_id: context,
                payload_mode: PayloadMode::Full,
                trigger_mode: TriggerMode::Conversation,
                duration: GrantDuration::Session,
                created_at: "test".into(),
            });
        }
        let outcome = timeline
            .record_pane_lifecycle(
                41,
                7,
                &PaneLifecycleEvent::Exited {
                    status: ExitStatus::Unknown,
                },
            )
            .unwrap();
        assert_eq!(outcome.deliveries_queued, 1);
        let deliveries = timeline.take_deliveries_for(ActorType::Agent, "same-pane");
        assert_eq!(deliveries.len(), 1);
        let payload = deliveries[0].payload.as_ref().unwrap();
        assert_eq!(payload["kind"], "exited");
        assert_eq!(payload["status"], "unknown");
        assert_eq!(payload["schema_version"], 1);
        assert_eq!(deliveries[0].actor, AppEventActor::System);
        assert!(timeline
            .take_deliveries_for(ActorType::Agent, "other-pane")
            .is_empty());
        assert!(timeline
            .take_deliveries_for(ActorType::Agent, "other-context")
            .is_empty());
        assert!(timeline
            .declare_streams(
                41,
                PUBLISHER,
                vec![EventStreamDecl {
                    name: STREAM.into(),
                    schema: serde_json::json!({}),
                    description: None,
                }]
            )
            .unwrap_err()
            .contains("reserved"));
        assert!(timeline
            .record_event(
                41,
                PUBLISHER,
                7,
                EmittedEvent {
                    event: STREAM.into(),
                    actor: AppEventActor::System,
                    actor_id: Some(PUBLISHER.into()),
                    caused_by: None,
                    summary: "forged clean exit".into(),
                    resource_id: "7".into(),
                    resource_scope: Some("pane".into()),
                    revision_after: "3".into(),
                    payload: Some(serde_json::json!({"kind":"exited", "status":"clean"})),
                    state_ref: None,
                    revision_before: None,
                    rollback_token: None,
                    changed_resources: vec![],
                    suggested_trigger: None,
                }
            )
            .unwrap_err()
            .contains("reserved"));
        assert_eq!(
            timeline.events().len(),
            2,
            "rejected publications cannot enter the bus"
        );
    }
}
