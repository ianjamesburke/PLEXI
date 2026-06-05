use schemars::JsonSchema;
use serde::{Deserialize, Serialize};


/// On-the-wire shape of one MIDI port. Mirrors `midi::MidiPortInfo` but lives
/// on the protocol surface so SDKs in other languages can map it without
/// depending on the midi module.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq, Eq)]
pub struct MidiPortWire {
    pub id: String,
    pub name: String,
    pub default: bool,
}

impl From<crate::media::midi::MidiPortInfo> for MidiPortWire {
    fn from(info: crate::media::midi::MidiPortInfo) -> Self {
        Self {
            id: info.id,
            name: info.name,
            default: info.default,
        }
    }
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

impl From<crate::media::audio::AudioDeviceInfo> for AudioDeviceWire {
    fn from(info: crate::media::audio::AudioDeviceInfo) -> Self {
        Self {
            id: info.id,
            name: info.name,
            default: info.default,
        }
    }
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
/// Apps declare callable tools via `DrawCommand::ExposeTools`.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct AiTool {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    /// Maximum milliseconds to wait for this tool's response. Defaults to 30s
    /// when absent. The broker uses this to bound `ToolCall` round-trips.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

/// Coarse model tier requested by the app. The host maps each tier to a
/// concrete model identifier per backend (spec §ai.query):
///   - `Low`    → Haiku
///   - `Medium` → Sonnet
///   - `High`   → Opus
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelTier {
    Low,
    Medium,
    High,
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

/// Direction of a flex layout node.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
#[serde(rename_all = "snake_case")]
pub enum LayoutDirection {
    Row,
    Column,
    Stack,
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


pub fn default_compact_threshold() -> f32 { 280.0 }
pub fn default_regular_threshold() -> f32 { 480.0 }
