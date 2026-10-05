//! Versioned intake, agent, app-instance, and receipt shapes.
//!
//! Clients submit [`IntakeEnvelope`]. Tenant, actor authority, and grants are
//! not fields of that envelope; [`crate::cloud_assistant::session::Principal`]
//! is attached by the host before the parser's value is acted on.

use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;
pub const MAX_TEXT_CHARS: usize = 8_192;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractError {
    pub code: &'static str,
    pub detail: String,
}

impl ContractError {
    fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntakeEnvelope {
    pub schema_version: u32,
    pub request_id: String,
    pub host_id: String,
    pub agent_id: String,
    pub conversation_id: String,
    pub expires_at: String,
    pub content: Vec<ContentPart>,
    #[serde(default)]
    pub target: Option<IntakeTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentPart {
    #[serde(rename = "type")]
    pub kind: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntakeTarget {
    pub resource_id: String,
    pub expected_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptPhase {
    AcceptedAtEdge,
    DeliveredToHost,
    Queued,
    Running,
    WaitingForPermission,
    Terminal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalKind {
    Succeeded,
    Failed,
    Cancelled,
    Expired,
    OutcomeUnknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntakeReceipt {
    pub schema_version: u32,
    pub request_id: String,
    pub owner_id: String,
    pub phase: ReceiptPhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<TerminalKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentRecord {
    pub schema_version: u32,
    pub agent_id: String,
    pub conversation_id: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundPolicy {
    Continue,
    StopOnLastViewClose,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ViewState {
    Attached { device: String, observed_at: String },
    Hidden { device: String, observed_at: String },
    InactiveContext { device: String, observed_at: String },
    NoView { observed_at: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppInstanceRecord {
    pub schema_version: u32,
    pub instance_id: String,
    pub app_id: String,
    pub resource_id: String,
    pub background_policy: BackgroundPolicy,
    pub view: ViewState,
}

/// Production parser for a client turn. Unknown fields, including any
/// client-supplied role or tenant, fail closed.
pub fn parse_intake(raw: &str) -> Result<IntakeEnvelope, ContractError> {
    if raw.len() > MAX_TEXT_CHARS * 4 {
        return Err(ContractError::new(
            "oversized",
            "intake body exceeds the byte cap",
        ));
    }
    let envelope: IntakeEnvelope = serde_json::from_str(raw)
        .map_err(|error| ContractError::new("invalid_input", error.to_string()))?;
    validate_envelope(&envelope)?;
    Ok(envelope)
}

pub fn validate_envelope(envelope: &IntakeEnvelope) -> Result<(), ContractError> {
    if envelope.schema_version != SCHEMA_VERSION {
        return Err(ContractError::new(
            "unsupported_schema",
            format!(
                "schema_version {} is not supported",
                envelope.schema_version
            ),
        ));
    }
    if envelope.request_id.trim().is_empty()
        || envelope.host_id.trim().is_empty()
        || envelope.agent_id.trim().is_empty()
        || envelope.conversation_id.trim().is_empty()
    {
        return Err(ContractError::new(
            "invalid_input",
            "request, host, agent, and conversation ids are required",
        ));
    }
    if chrono::DateTime::parse_from_rfc3339(&envelope.expires_at).is_err() {
        return Err(ContractError::new(
            "invalid_input",
            "expires_at must be RFC 3339",
        ));
    }
    if envelope.content.is_empty() {
        return Err(ContractError::new("invalid_input", "content is empty"));
    }
    let mut chars = 0usize;
    for part in &envelope.content {
        if part.kind != "text" {
            return Err(ContractError::new(
                "unsupported_schema",
                format!("content type {} is not supported", part.kind),
            ));
        }
        if part.text.is_empty() {
            return Err(ContractError::new("invalid_input", "text content is empty"));
        }
        chars = chars.saturating_add(part.text.chars().count());
    }
    if chars > MAX_TEXT_CHARS {
        return Err(ContractError::new(
            "oversized",
            format!("content is {chars} chars; cap is {MAX_TEXT_CHARS}"),
        ));
    }
    Ok(())
}

pub fn fingerprint(envelope: &IntakeEnvelope) -> String {
    serde_json::to_string(envelope).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_intake_round_trips() {
        let raw = include_str!("../../fixtures/cloud-assistant/intake-turn.json");
        let envelope = parse_intake(raw).expect("fixture parses");
        assert_eq!(envelope.schema_version, SCHEMA_VERSION);
        assert_eq!(envelope.request_id, "req-fixture-e4");
        assert_eq!(envelope.target.as_ref().unwrap().expected_revision, 0);
        let again = serde_json::to_string(&envelope).unwrap();
        let parsed = parse_intake(&again).unwrap();
        assert_eq!(parsed, envelope);
    }

    #[test]
    fn fixture_agent_and_instance_round_trip() {
        let agent: AgentRecord =
            serde_json::from_str(include_str!("../../fixtures/cloud-assistant/agent.json"))
                .unwrap();
        assert_eq!(agent.agent_id, "assistant-fixture");
        assert_eq!(agent.conversation_id, "conversation-fixture");
        let instance: AppInstanceRecord = serde_json::from_str(include_str!(
            "../../fixtures/cloud-assistant/app-instance.json"
        ))
        .unwrap();
        assert_eq!(instance.app_id, "chess");
        assert_eq!(instance.background_policy, BackgroundPolicy::Continue);
        assert!(matches!(instance.view, ViewState::Attached { .. }));
        let receipt: IntakeReceipt =
            serde_json::from_str(include_str!("../../fixtures/cloud-assistant/receipt.json"))
                .unwrap();
        assert_eq!(receipt.phase, ReceiptPhase::Queued);
        assert_eq!(receipt.outcome, None);
    }

    #[test]
    fn rejects_client_supplied_authority_and_unknown_schema() {
        let raw = include_str!("../../fixtures/cloud-assistant/intake-turn.json");
        let mut value: serde_json::Value = serde_json::from_str(raw).unwrap();
        value["role"] = serde_json::json!("admin");
        value["tenant_id"] = serde_json::json!("other-tenant");
        let error = parse_intake(&value.to_string()).unwrap_err();
        assert_eq!(error.code, "invalid_input");

        value = serde_json::from_str(raw).unwrap();
        value["schema_version"] = serde_json::json!(99);
        let error = parse_intake(&value.to_string()).unwrap_err();
        assert_eq!(error.code, "unsupported_schema");
    }

    #[test]
    fn rejects_oversized_text_before_a_record_exists() {
        let mut envelope: IntakeEnvelope = parse_intake(include_str!(
            "../../fixtures/cloud-assistant/intake-turn.json"
        ))
        .unwrap();
        envelope.content[0].text = "x".repeat(MAX_TEXT_CHARS + 1);
        let error = validate_envelope(&envelope).unwrap_err();
        assert_eq!(error.code, "oversized");
    }
}
