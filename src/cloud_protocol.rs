//! Shared cloud-runtime wire protocol primitives.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("frame too short")]
    FrameTooShort,
    #[error("unknown frame type {0}")]
    UnknownFrameType(u8),
    #[error("payload length mismatch: header says {expected}, actual {actual}")]
    LengthMismatch { expected: usize, actual: usize },
    #[error("payload decode failed: {0}")]
    Decode(String),
    #[error("payload encode failed: {0}")]
    Encode(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum FrameType {
    ViewRequest = 0x01,
    UiNodePatch = 0x02,
    InputEvent = 0x03,
    EffectRequest = 0x04,
    EffectResult = 0x05,
    StateSync = 0x06,
    PaymentRequest = 0x07,
    PaymentResult = 0x08,
}

impl TryFrom<u8> for FrameType {
    type Error = ProtocolError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x01 => Ok(Self::ViewRequest),
            0x02 => Ok(Self::UiNodePatch),
            0x03 => Ok(Self::InputEvent),
            0x04 => Ok(Self::EffectRequest),
            0x05 => Ok(Self::EffectResult),
            0x06 => Ok(Self::StateSync),
            0x07 => Ok(Self::PaymentRequest),
            0x08 => Ok(Self::PaymentResult),
            other => Err(ProtocolError::UnknownFrameType(other)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudFrame {
    pub frame_type: FrameType,
    pub payload: CloudPayload,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CloudPayload {
    ViewRequest(ViewRequest),
    UiNodePatch(UiNodePatch),
    InputEvent(InputEventFrame),
    EffectRequest(EffectRequestFrame),
    EffectResult(EffectResultFrame),
    StateSync(StateSyncFrame),
    PaymentRequest(PaymentRequestFrame),
    PaymentResult(PaymentResultFrame),
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ViewRequest {
    pub last_state_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputEventFrame {
    pub event_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectRequestFrame {
    pub request_id: u64,
    pub effect_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectResultFrame {
    pub request_id: u64,
    pub result_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateSyncFrame {
    pub app_id: String,
    pub state_hash: String,
    pub snapshot_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaymentRequestFrame {
    pub app_id: String,
    pub price_usd_cents: u64,
    pub model: String,
    pub payment_endpoint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaymentResultFrame {
    pub approved: bool,
    pub session_token: Option<String>,
    pub error: Option<String>,
}

pub fn encode_frame(frame: &CloudFrame) -> Result<Vec<u8>, ProtocolError> {
    let payload =
        bincode::serialize(&frame.payload).map_err(|e| ProtocolError::Encode(e.to_string()))?;
    let mut out = Vec::with_capacity(5 + payload.len());
    out.push(frame.frame_type as u8);
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(&payload);
    Ok(out)
}

pub fn decode_frame(bytes: &[u8]) -> Result<CloudFrame, ProtocolError> {
    if bytes.len() < 5 {
        return Err(ProtocolError::FrameTooShort);
    }
    let frame_type = FrameType::try_from(bytes[0])?;
    let expected = u32::from_be_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]) as usize;
    let payload = &bytes[5..];
    if payload.len() != expected {
        return Err(ProtocolError::LengthMismatch {
            expected,
            actual: payload.len(),
        });
    }
    let decoded: CloudPayload =
        bincode::deserialize(payload).map_err(|e| ProtocolError::Decode(e.to_string()))?;
    Ok(CloudFrame {
        frame_type,
        payload: decoded,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudUiNode {
    pub key: String,
    pub kind: String,
    #[serde(default)]
    pub props: BTreeMap<String, String>,
    #[serde(default)]
    pub children: Vec<CloudUiNode>,
}

impl CloudUiNode {
    pub fn new(key: impl Into<String>, kind: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            kind: kind.into(),
            props: BTreeMap::new(),
            children: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiNodePatch {
    pub ops: Vec<PatchOp>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PatchOp {
    Replace { key: String, node: CloudUiNode },
    Remove { key: String },
    Skip { key: String },
}

pub fn diff(prev: Option<&CloudUiNode>, next: &CloudUiNode) -> UiNodePatch {
    let mut ops = Vec::new();
    diff_node(prev, Some(next), "0", &mut ops);
    UiNodePatch { ops }
}

fn diff_node(
    prev: Option<&CloudUiNode>,
    next: Option<&CloudUiNode>,
    implicit_key: &str,
    ops: &mut Vec<PatchOp>,
) {
    match (prev, next) {
        (Some(old), Some(new)) => {
            let key = if new.key.is_empty() {
                implicit_key.to_string()
            } else {
                new.key.clone()
            };
            if old == new {
                ops.push(PatchOp::Skip { key });
                return;
            }
            if old.kind != new.kind || old.props != new.props || old.key != new.key {
                ops.push(PatchOp::Replace {
                    key,
                    node: new.clone(),
                });
                return;
            }
            let old_keys: Vec<&str> = old
                .children
                .iter()
                .map(|child| child.key.as_str())
                .collect();
            let new_keys: Vec<&str> = new
                .children
                .iter()
                .map(|child| child.key.as_str())
                .collect();
            if old_keys != new_keys {
                ops.push(PatchOp::Replace {
                    key,
                    node: new.clone(),
                });
                return;
            }
            let max_len = old.children.len().max(new.children.len());
            for idx in 0..max_len {
                diff_node(
                    old.children.get(idx),
                    new.children.get(idx),
                    &format!("{implicit_key}/{idx}"),
                    ops,
                );
            }
        }
        (None, Some(new)) => {
            let key = if new.key.is_empty() {
                implicit_key.to_string()
            } else {
                new.key.clone()
            };
            ops.push(PatchOp::Replace {
                key,
                node: new.clone(),
            });
        }
        (Some(old), None) => {
            let key = if old.key.is_empty() {
                implicit_key.to_string()
            } else {
                old.key.clone()
            };
            ops.push(PatchOp::Remove { key });
        }
        (None, None) => {}
    }
}

pub fn apply(tree: &mut CloudUiNode, patch: &UiNodePatch) {
    for op in &patch.ops {
        match op {
            PatchOp::Skip { .. } => {}
            PatchOp::Replace { key, node } => {
                if tree.key == *key {
                    *tree = node.clone();
                } else {
                    replace_child(tree, key, node);
                }
            }
            PatchOp::Remove { key } => {
                remove_child(tree, key);
            }
        }
    }
}

fn replace_child(tree: &mut CloudUiNode, key: &str, node: &CloudUiNode) -> bool {
    for child in &mut tree.children {
        if child.key == key {
            *child = node.clone();
            return true;
        }
        if replace_child(child, key, node) {
            return true;
        }
    }
    false
}

fn remove_child(tree: &mut CloudUiNode, key: &str) -> bool {
    let before = tree.children.len();
    tree.children.retain(|child| child.key != key);
    if tree.children.len() != before {
        return true;
    }
    tree.children
        .iter_mut()
        .any(|child| remove_child(child, key))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payloads() -> Vec<CloudFrame> {
        vec![
            CloudFrame {
                frame_type: FrameType::ViewRequest,
                payload: CloudPayload::ViewRequest(ViewRequest::default()),
            },
            CloudFrame {
                frame_type: FrameType::UiNodePatch,
                payload: CloudPayload::UiNodePatch(UiNodePatch { ops: vec![] }),
            },
            CloudFrame {
                frame_type: FrameType::InputEvent,
                payload: CloudPayload::InputEvent(InputEventFrame {
                    event_json: "{}".into(),
                }),
            },
            CloudFrame {
                frame_type: FrameType::EffectRequest,
                payload: CloudPayload::EffectRequest(EffectRequestFrame {
                    request_id: 1,
                    effect_json: "{}".into(),
                }),
            },
            CloudFrame {
                frame_type: FrameType::EffectResult,
                payload: CloudPayload::EffectResult(EffectResultFrame {
                    request_id: 1,
                    result_json: "{}".into(),
                }),
            },
            CloudFrame {
                frame_type: FrameType::StateSync,
                payload: CloudPayload::StateSync(StateSyncFrame {
                    app_id: "app".into(),
                    state_hash: "hash".into(),
                    snapshot_json: "{}".into(),
                }),
            },
            CloudFrame {
                frame_type: FrameType::PaymentRequest,
                payload: CloudPayload::PaymentRequest(PaymentRequestFrame {
                    app_id: "app".into(),
                    price_usd_cents: 5,
                    model: "per-run".into(),
                    payment_endpoint: "https://pay".into(),
                }),
            },
            CloudFrame {
                frame_type: FrameType::PaymentResult,
                payload: CloudPayload::PaymentResult(PaymentResultFrame {
                    approved: true,
                    session_token: Some("tok".into()),
                    error: None,
                }),
            },
        ]
    }

    #[test]
    fn wire_protocol_round_trips_all_frame_types() {
        for frame in payloads() {
            let encoded = encode_frame(&frame).unwrap();
            assert_eq!(encoded[0], frame.frame_type as u8);
            let decoded = decode_frame(&encoded).unwrap();
            assert_eq!(decoded, frame);
        }
    }

    #[test]
    fn diff_patch_round_trip_for_large_tree() {
        let mut prev = CloudUiNode::new("root", "column");
        for idx in 0..200 {
            let mut child = CloudUiNode::new(format!("row-{idx}"), "text");
            child.props.insert("text".into(), format!("old {idx}"));
            prev.children.push(child);
        }
        let mut next = prev.clone();
        for idx in [4, 18, 42, 99, 175] {
            next.children[idx]
                .props
                .insert("text".into(), format!("new {idx}"));
        }
        next.children.remove(150);
        next.children.push(CloudUiNode::new("row-new", "button"));

        let patch = diff(Some(&prev), &next);
        let changed = patch
            .ops
            .iter()
            .filter(|op| !matches!(op, PatchOp::Skip { .. }))
            .count();
        assert!(changed <= 20, "changed ops={changed}, patch={patch:?}");

        let mut applied = prev;
        apply(&mut applied, &patch);
        assert_eq!(applied, next);
    }
}
