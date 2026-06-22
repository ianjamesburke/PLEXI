//! Local cloud-runtime proof-of-concept loop.

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::cloud_protocol::{
    apply, decode_frame, diff, encode_frame, CloudFrame, CloudPayload, CloudUiNode, FrameType,
    InputEventFrame, ProtocolError, StateSyncFrame, ViewRequest,
};

#[derive(Debug, Error)]
pub enum CloudRuntimeError {
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
    #[error("unexpected cloud frame: expected {expected}, got {actual:?}")]
    UnexpectedFrame {
        expected: &'static str,
        actual: FrameType,
    },
    #[error("cloud input event JSON decode failed: {0}")]
    InvalidInputEvent(String),
    #[error("cloud client has no root tree for patch")]
    MissingClientTree,
}

pub trait CloudLifecycle {
    fn app_id(&self) -> &str;
    fn update(&mut self, event_json: &str) -> Result<(), CloudRuntimeError>;
    fn view(&self) -> CloudUiNode;
    fn snapshot_json(&self) -> String;

    fn state_hash(&self) -> String {
        hex_sha256(self.snapshot_json().as_bytes())
    }
}

pub struct LocalCloudRuntime<L> {
    lifecycle: L,
    previous_tree: Option<CloudUiNode>,
}

impl<L: CloudLifecycle> LocalCloudRuntime<L> {
    pub fn new(lifecycle: L) -> Self {
        Self {
            lifecycle,
            previous_tree: None,
        }
    }

    pub fn handle_frame_bytes(&mut self, bytes: &[u8]) -> Result<Vec<Vec<u8>>, CloudRuntimeError> {
        let frame = decode_frame(bytes)?;
        let responses = self.handle_frame(frame)?;
        responses
            .iter()
            .map(encode_frame)
            .collect::<Result<Vec<_>, _>>()
            .map_err(CloudRuntimeError::Protocol)
    }

    pub fn handle_frame(
        &mut self,
        frame: CloudFrame,
    ) -> Result<Vec<CloudFrame>, CloudRuntimeError> {
        match frame.payload {
            CloudPayload::ViewRequest(request) => {
                log::info!(
                    "cloud_runtime: view request app={}",
                    self.lifecycle.app_id()
                );
                Ok(self.render_response_frames(request))
            }
            CloudPayload::InputEvent(input) => {
                log::info!("cloud_runtime: input event app={}", self.lifecycle.app_id());
                self.lifecycle.update(&input.event_json)?;
                Ok(self.render_response_frames(ViewRequest {
                    last_state_hash: None,
                }))
            }
            _ => Err(CloudRuntimeError::UnexpectedFrame {
                expected: "ViewRequest or InputEvent",
                actual: frame.frame_type,
            }),
        }
    }

    fn render_response_frames(&mut self, request: ViewRequest) -> Vec<CloudFrame> {
        let next_tree = self.lifecycle.view();
        let patch = diff(self.previous_tree.as_ref(), &next_tree);
        self.previous_tree = Some(next_tree);

        let mut frames = vec![CloudFrame {
            frame_type: FrameType::UiNodePatch,
            payload: CloudPayload::UiNodePatch(patch),
        }];

        let state_hash = self.lifecycle.state_hash();
        if request.last_state_hash.as_deref() != Some(state_hash.as_str()) {
            frames.push(CloudFrame {
                frame_type: FrameType::StateSync,
                payload: CloudPayload::StateSync(StateSyncFrame {
                    app_id: self.lifecycle.app_id().to_string(),
                    state_hash,
                    snapshot_json: self.lifecycle.snapshot_json(),
                }),
            });
        }

        frames
    }
}

pub struct LocalCloudClient<L> {
    runtime: LocalCloudRuntime<L>,
    tree: Option<CloudUiNode>,
    state_hash: Option<String>,
    snapshot_json: Option<String>,
}

impl<L: CloudLifecycle> LocalCloudClient<L> {
    pub fn new(runtime: LocalCloudRuntime<L>) -> Self {
        Self {
            runtime,
            tree: None,
            state_hash: None,
            snapshot_json: None,
        }
    }

    pub fn request_view(&mut self) -> Result<Vec<CloudFrame>, CloudRuntimeError> {
        let frame = CloudFrame {
            frame_type: FrameType::ViewRequest,
            payload: CloudPayload::ViewRequest(ViewRequest {
                last_state_hash: self.state_hash.clone(),
            }),
        };
        self.exchange(frame)
    }

    pub fn send_input(
        &mut self,
        event_json: impl Into<String>,
    ) -> Result<Vec<CloudFrame>, CloudRuntimeError> {
        let frame = CloudFrame {
            frame_type: FrameType::InputEvent,
            payload: CloudPayload::InputEvent(InputEventFrame {
                event_json: event_json.into(),
            }),
        };
        self.exchange(frame)
    }

    pub fn cached_tree(&self) -> Option<&CloudUiNode> {
        self.tree.as_ref()
    }

    pub fn state_hash(&self) -> Option<&str> {
        self.state_hash.as_deref()
    }

    pub fn snapshot_json(&self) -> Option<&str> {
        self.snapshot_json.as_deref()
    }

    fn exchange(&mut self, frame: CloudFrame) -> Result<Vec<CloudFrame>, CloudRuntimeError> {
        let request = encode_frame(&frame)?;
        let response_bytes = self.runtime.handle_frame_bytes(&request)?;
        let mut responses = Vec::with_capacity(response_bytes.len());
        for bytes in response_bytes {
            let response = decode_frame(&bytes)?;
            self.apply_response(&response)?;
            responses.push(response);
        }
        Ok(responses)
    }

    fn apply_response(&mut self, frame: &CloudFrame) -> Result<(), CloudRuntimeError> {
        match &frame.payload {
            CloudPayload::UiNodePatch(patch) => {
                if self.tree.is_none() {
                    let root = patch.ops.iter().find_map(|op| match op {
                        crate::cloud_protocol::PatchOp::Replace { node, .. } => Some(node.clone()),
                        _ => None,
                    });
                    self.tree = root;
                }
                let tree = self
                    .tree
                    .as_mut()
                    .ok_or(CloudRuntimeError::MissingClientTree)?;
                apply(tree, patch);
                Ok(())
            }
            CloudPayload::StateSync(state) => {
                self.state_hash = Some(state.state_hash.clone());
                self.snapshot_json = Some(state.snapshot_json.clone());
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

pub struct CounterCloudLifecycle {
    app_id: String,
    count: u64,
    last_event: Option<String>,
}

impl CounterCloudLifecycle {
    pub fn new(app_id: impl Into<String>) -> Self {
        Self {
            app_id: app_id.into(),
            count: 0,
            last_event: None,
        }
    }
}

impl CloudLifecycle for CounterCloudLifecycle {
    fn app_id(&self) -> &str {
        &self.app_id
    }

    fn update(&mut self, event_json: &str) -> Result<(), CloudRuntimeError> {
        let event: Value = serde_json::from_str(event_json)
            .map_err(|e| CloudRuntimeError::InvalidInputEvent(e.to_string()))?;
        self.last_event = Some(event_json.to_string());
        if event.get("type").and_then(Value::as_str) == Some("click")
            && event.get("target").and_then(Value::as_str) == Some("increment")
        {
            self.count += 1;
        }
        Ok(())
    }

    fn view(&self) -> CloudUiNode {
        let mut root = CloudUiNode::new("root", "column");

        let mut count = CloudUiNode::new("count-label", "text");
        count
            .props
            .insert("text".to_string(), format!("Count: {}", self.count));
        root.children.push(count);

        let mut button = CloudUiNode::new("increment", "button");
        button
            .props
            .insert("label".to_string(), "Increment".to_string());
        root.children.push(button);

        root
    }

    fn snapshot_json(&self) -> String {
        json!({
            "count": self.count,
            "last_event": self.last_event,
        })
        .to_string()
    }
}

fn hex_sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloud_protocol::PatchOp;

    #[test]
    fn local_cloud_runtime_round_trips_view_input_and_reconnect() {
        let runtime = LocalCloudRuntime::new(CounterCloudLifecycle::new("poc.counter"));
        let mut client = LocalCloudClient::new(runtime);

        let initial = client.request_view().expect("initial view should succeed");
        assert!(initial
            .iter()
            .any(|frame| matches!(frame.payload, CloudPayload::UiNodePatch(_))));
        assert_eq!(
            client
                .cached_tree()
                .and_then(|tree| tree.children.first())
                .and_then(|node| node.props.get("text"))
                .map(String::as_str),
            Some("Count: 0")
        );
        let initial_hash = client
            .state_hash()
            .expect("initial state sync should set hash")
            .to_string();

        let updated = client
            .send_input(r#"{"type":"click","target":"increment"}"#)
            .expect("input event should update runtime");
        let updated_patch = updated
            .iter()
            .find_map(|frame| match &frame.payload {
                CloudPayload::UiNodePatch(patch) => Some(patch),
                _ => None,
            })
            .expect("input should return a patch");
        assert!(updated_patch.ops.iter().any(|op| matches!(
            op,
            PatchOp::Replace { key, .. } if key == "count-label"
        )));
        assert_eq!(
            client
                .cached_tree()
                .and_then(|tree| tree.children.first())
                .and_then(|node| node.props.get("text"))
                .map(String::as_str),
            Some("Count: 1")
        );
        assert_ne!(
            client.state_hash(),
            Some(initial_hash.as_str()),
            "input event should update state hash"
        );
        assert_eq!(
            client.snapshot_json(),
            Some(r#"{"count":1,"last_event":"{\"type\":\"click\",\"target\":\"increment\"}"}"#)
        );

        let reconnect = client
            .request_view()
            .expect("matching reconnect state hash should succeed");
        assert!(
            !reconnect
                .iter()
                .any(|frame| matches!(frame.payload, CloudPayload::StateSync(_))),
            "matching state hash should suppress redundant StateSync"
        );
    }
}
