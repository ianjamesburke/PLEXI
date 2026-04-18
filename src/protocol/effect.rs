use serde::{Deserialize, Serialize};

use crate::protocol::schema::{AppId, PipeId, RequestId, RunId, SecretKey};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AppEffect {
    CapabilityRequest {
        request_id: RequestId,
        capability: Capability,
    },
    SecretGet {
        key: SecretKey,
    },
    RunGet {
        intent: String,
        payload: serde_json::Value,
    },
    RunComplete {
        run_id: RunId,
        result: serde_json::Value,
    },
    Notify {
        level: NotifyLevel,
        title: String,
        body: String,
        actions: Vec<NotificationAction>,
    },
    AudioPlay {
        source: AudioSourceSpec,
        volume: f32,
        state: PlaybackState,
    },
    AudioCapture {
        pipe_id: PipeId,
        sample_rate: u32,
        buffer_size: u32,
    },
    PipeOpen {
        pipe_id: PipeId,
        mode: PipeMode,
        direction: PipeDirection,
    },
    PipeSend {
        pipe_id: PipeId,
        payload: serde_json::Value,
    },
    SpawnApp {
        app_id: AppId,
        layout: LayoutRequest,
        args: Vec<String>,
    },
    StatusSummary {
        text: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    FsRead,
    FsWrite,
    NetHttp,
    SecretsGet,
    AudioRecord,
    AudioPlayback,
    VideoPlayback,
    PipeOpen,
    SpawnApp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotifyLevel {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationAction {
    pub label: String,
    pub action_type: String,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AudioSourceSpec {
    File { path: String },
    Pipe { pipe_id: PipeId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackState {
    Play,
    Pause,
    Stop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PipeMode {
    Json,
    Binary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PipeDirection {
    In,
    Out,
    Duplex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayoutRequest {
    Below,
    Right,
    ReplaceFocused,
}
