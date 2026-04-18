use serde::{Deserialize, Serialize};

use crate::protocol::schema::{FrameId, RequestId, RunId, SecretKey, WorkspaceRoot};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PlexiEvent {
    Init {
        protocol: String,
        app_id: String,
        workspace_root: String,
        capabilities: Vec<String>,
        feature_flags: Vec<String>,
    },
    Render {
        frame_id: FrameId,
        rect: Rect,
    },
    Input {
        key: Option<String>,
        text: Option<String>,
    },
    CapabilityDecision {
        request_id: RequestId,
        granted: bool,
    },
    SecretValue {
        key: SecretKey,
        value: Option<String>,
        denied: bool,
    },
    RunUpdate {
        run_id: RunId,
        status: String,
        payload: serde_json::Value,
    },
    PathChanged {
        cwd: String,
    },
    Suspend,
    Resume,
    Shutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InitPayload {
    pub app_id: String,
    pub workspace_root: WorkspaceRoot,
    pub capabilities: Vec<String>,
    pub feature_flags: Vec<String>,
}
