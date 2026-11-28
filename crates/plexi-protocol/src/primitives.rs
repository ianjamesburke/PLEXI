use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Playback state for a video handle. Encoded on the wire as
/// `{"play": null}` / `{"pause": null}` / `{"seek": <ms>}` via serde's
/// default `untagged`-friendly encoding. The PGAP wire serialises this as a
/// nested struct under `state` in `DrawCommand::SetVideoState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VideoState {
    Play,
    Pause,
    /// Absolute position in milliseconds from the start of the video.
    Seek {
        position_ms: u64,
    },
}

/// Parse a wire string into a protocol enum, naming the field it came from.
///
/// The enum's own `#[serde(rename_all = "snake_case")]` is the single source of
/// truth for which spellings are accepted, and serde's `unknown variant …,
/// expected one of …` already lists them — so a hand-written match arm per
/// variant can only drift from the wire contract it claims to enforce.
pub fn parse_enum<T: serde::de::DeserializeOwned>(field: &str, raw: &str) -> Result<T, String> {
    serde_json::from_value(serde_json::Value::String(raw.to_string()))
        .map_err(|e| format!("invalid {field} '{raw}': {e}"))
}

/// On-the-wire shape of one MIDI port. Mirrors `midi::MidiPortInfo` but lives
/// on the protocol surface so SDKs in other languages can map it without
/// depending on the midi module.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq, Eq)]
pub struct MidiPortWire {
    pub id: String,
    pub name: String,
    pub default: bool,
}

/// On-the-wire shape of one audio device. Mirrors `audio::AudioDeviceInfo`
/// but lives on the protocol surface so SDKs in other languages can map it
/// without depending on the audio module.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq, Eq)]
pub struct AudioDeviceWire {
    pub id: String,
    pub name: String,
    pub default: bool,
}

/// One message in an `AiQuery` conversation. Wire shape mirrors Anthropic
/// Messages API: `role` ∈ {"user", "assistant"}, `content` is plain text.
/// (Multimodal content blocks are future-scope.)
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq, Eq)]
pub struct AiMessage {
    pub role: String,
    pub content: String,
}

/// Tool definition for the tool-use turn loop (#398).
/// Plugins declare callable tools via `DrawCommand::ExposeTools`.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq, Eq)]
pub struct AiTool {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    pub output_schema: serde_json::Value,
    /// Maximum milliseconds to wait for this tool's response. Defaults to 30s
    /// when absent. The broker uses this to bound `ToolCall` round-trips.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    /// App-declared hint that the tool only reads state. This is not authority:
    /// the host permission monitor still admits and audits every call.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub read_only: bool,
}

/// Coarse model tier requested by the plugin. The host maps each tier to a
/// concrete model identifier per backend (spec §ai.query):
///   - `Low`    → Haiku
///   - `Medium` → Sonnet
///   - `High`   → Opus
#[derive(
    Serialize, Deserialize, JsonSchema, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord,
)]
#[serde(rename_all = "snake_case")]
pub enum ModelTier {
    Low,
    Medium,
    High,
}

impl ModelTier {
    /// Canonical lowercase name, matching the serde representation.
    pub fn as_str(self) -> &'static str {
        match self {
            ModelTier::Low => "low",
            ModelTier::Medium => "medium",
            ModelTier::High => "high",
        }
    }
}

/// A simple rectangle (logical coordinates).
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Default)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub cmd: bool,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
#[serde(rename_all = "snake_case")]
pub enum MouseButton {
    Primary,
    Secondary,
}

/// Output channel selector for `DrawCommand::StreamProcess`.
/// v1: `structured` emits the same bytes as `stdout`.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StreamChannel {
    Stdout,
    Stderr,
    /// Reserved for future structured-progress framing. v1: identical to `stdout`.
    Structured,
}

/// Dialog mode for `DrawCommand::OpenFilePicker`.
///
/// - `Open` — pick one existing file (or several when `multiple` is true).
/// - `Folder` — pick one existing directory; the grant covers the subtree.
/// - `Save` — pick a save destination; the file may not exist yet.
///
/// `multiple` is only meaningful for `Open`; `Folder` and `Save` always
/// return exactly one path.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum FilePickerMode {
    #[default]
    Open,
    Folder,
    Save,
}

pub fn default_compact_threshold() -> f32 {
    280.0
}
pub fn default_regular_threshold() -> f32 {
    480.0
}

/// Rolled-up status for a single context, including recursive children.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct ContextState {
    pub context_id: u64,
    pub label: String,
    pub pane_count: u32,
    pub active_agents: u32,
    pub status: ContextStatus,
    pub pane_summaries: Vec<PaneSummary>,
    pub children: Vec<ContextState>,
}

/// Overall status of a context, derived from its pane summaries and child contexts.
/// Precedence: `NeedsYou > Blocked > Error > Working > Idle`.
#[derive(
    Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
pub enum ContextStatus {
    Idle,
    Working,
    Error,
    Blocked,
    NeedsYou,
    Done,
}

/// Summary of a single pane within a context.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct PaneSummary {
    pub pane_id: u64,
    pub label: String,
    pub status: PaneSummaryStatus,
}

/// Status of an individual pane.
/// Precedence: `NeedsYou > Blocked > Error > Active > Idle`.
#[derive(
    Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
pub enum PaneSummaryStatus {
    Idle,
    Active,
    Error,
    Blocked,
    NeedsYou,
}

impl ContextState {
    /// Count of panes currently in `Active` (`Working`) state across this context and its children.
    pub fn working_count(&self) -> u32 {
        let own = self
            .pane_summaries
            .iter()
            .filter(|s| s.status == PaneSummaryStatus::Active)
            .count() as u32;
        let children: u32 = self.children.iter().map(Self::working_count).sum();
        (own + children).max(self.active_agents)
    }

    /// Count of panes currently in `Blocked` (Tier 1, master-owned) state across this context and its children.
    pub fn blocked_count(&self) -> u32 {
        let own = self
            .pane_summaries
            .iter()
            .filter(|s| s.status == PaneSummaryStatus::Blocked)
            .count() as u32;
        let children: u32 = self.children.iter().map(Self::blocked_count).sum();
        own + children
    }

    /// Count of panes currently in `NeedsYou` (Tier 2, human-owned) state across this context and its children.
    pub fn needs_you_count(&self) -> u32 {
        let own = self
            .pane_summaries
            .iter()
            .filter(|s| s.status == PaneSummaryStatus::NeedsYou)
            .count() as u32;
        let children: u32 = self.children.iter().map(Self::needs_you_count).sum();
        own + children
    }

    /// Count of panes currently in `Error` state across this context and its children.
    pub fn error_count(&self) -> u32 {
        let own = self
            .pane_summaries
            .iter()
            .filter(|s| s.status == PaneSummaryStatus::Error)
            .count() as u32;
        let children: u32 = self.children.iter().map(Self::error_count).sum();
        own + children
    }

    /// Format the rolled-up bracketed escalation badge for `PortalPane` cards
    /// (e.g. `"[2 WORKING · 1 BLOCKED]"`, `"[1 NEEDS YOU]"`, `"[1 ERROR]"`).
    pub fn status_summary_badge(&self) -> Option<String> {
        let working = self.working_count();
        let blocked = self.blocked_count();
        let needs_you = self.needs_you_count();
        let errors = self.error_count();

        let mut parts = Vec::new();
        if working > 0 {
            parts.push(format!("{working} WORKING"));
        }
        if blocked > 0 {
            parts.push(format!("{blocked} BLOCKED"));
        }
        if needs_you > 0 {
            parts.push(format!("{needs_you} NEEDS YOU"));
        }
        if errors > 0 {
            parts.push(format!("{errors} ERROR"));
        }
        if !parts.is_empty() {
            return Some(format!("[{}]", parts.join(" \u{b7} ")));
        }

        match self.status {
            ContextStatus::NeedsYou => Some("[1 NEEDS YOU]".to_string()),
            ContextStatus::Blocked => Some("[1 BLOCKED]".to_string()),
            ContextStatus::Error => Some("[1 ERROR]".to_string()),
            ContextStatus::Working => Some(format!("[{} WORKING]", self.active_agents.max(1))),
            ContextStatus::Done => Some("[DONE]".to_string()),
            ContextStatus::Idle => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::AiTool;

    #[test]
    fn ai_tool_wire_requires_typed_input_and_output_schemas() {
        let tool = AiTool {
            name: "counter.read".to_string(),
            description: "Read the counter".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
            output_schema: serde_json::json!({
                "type": "object",
                "properties": {"count": {"type": "integer"}},
                "required": ["count"]
            }),
            timeout_ms: Some(2_000),
            read_only: true,
        };
        let wire = serde_json::to_value(&tool).unwrap();
        assert_eq!(wire["input_schema"]["type"], "object");
        assert_eq!(
            wire["output_schema"]["properties"]["count"]["type"],
            "integer"
        );
        assert_eq!(serde_json::from_value::<AiTool>(wire).unwrap(), tool);

        let missing_output = serde_json::json!({
            "name": "counter.read",
            "description": "Read the counter",
            "input_schema": {"type": "object"}
        });
        assert!(serde_json::from_value::<AiTool>(missing_output).is_err());
    }
}
