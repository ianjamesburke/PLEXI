//! Host-owned identities for a node and the semantic scope that gives it meaning.
use super::pane::SemanticPaneState;
use super::wasm_view::{UiNodeData, UiTree};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug)]
pub(crate) struct NodeClick {
    pub reference: NodeRef,
    pub deadline: std::time::Instant,
    pub ticket: u64,
}

#[derive(Clone, Debug)]
pub(crate) struct NodeClickOutcome {
    pub ticket: u64,
    pub result: Result<(), String>,
}

pub(crate) struct NodeClickRequest {
    pub click: NodeClick,
    pub response_file: Option<String>,
}

pub(crate) fn check_click(
    click: Option<super::pane::PendingPaneClick>,
    state: &SemanticPaneState,
) -> (Option<super::pane::PendingPaneClick>, Option<NodeClickOutcome>) {
    if let Some(super::pane::PendingPaneClick { target: super::pane::PaneClickTarget::Node(node), .. }) = click {
        if let Err(error) = node.check(state) {
            log::info!("pane node click refused: {error}");
            return (None, Some(node.outcome(Err(error))));
        }
    }
    (click, None)
}

pub(crate) fn delivered(
    click: Option<super::pane::PendingPaneClick>,
    activated: bool,
) -> Option<NodeClickOutcome> {
    let super::pane::PaneClickTarget::Node(node) = click?.target else { return None };
    Some(node.outcome(if activated { Ok(()) } else {
        Err(format!("node {} was not activated (disabled or not painted)", node.reference.id))
    }))
}

impl NodeClick {
    pub(crate) fn check(self, state: &SemanticPaneState) -> Result<u64, String> {
        if std::time::Instant::now() >= self.deadline {
            return Err(format!("ref {} expired before delivery. Re-read pane state.", self.reference));
        }
        state.resolve_node_ref(&self.reference.to_string())
    }

    pub(crate) fn outcome(self, result: Result<(), String>) -> NodeClickOutcome {
        NodeClickOutcome { ticket: self.ticket, result }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NodeRef {
    pub id: u64,
    pub digest: u64,
}

impl std::fmt::Display for NodeRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{:016x}", self.id, self.digest)
    }
}

impl NodeRef {
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        let malformed = || format!("malformed ref {value:?}; read it from pane state");
        let (id, digest) = value.split_once('.').ok_or_else(malformed)?;
        if digest.len() != 16 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(malformed());
        }
        Ok(Self {
            id: id.parse().map_err(|_| malformed())?,
            digest: u64::from_str_radix(digest, 16).map_err(|_| malformed())?,
        })
    }
}

fn identity_key(key: &str) -> String {
    if let Some(meta) = super::wasm_render::parse_table_node_key(key) {
        return meta.identity_key;
    }
    if let Some(key) = key.strip_prefix(super::wasm_render::CARD_NODE_KEY_PREFIX) {
        return key.to_owned();
    }
    if let Some(key) = key.strip_prefix("__plexi_text_role__:") {
        if let Some((_, key)) = key.split_once(':') {
            return key.to_owned();
        }
    }
    key.to_owned()
}

/// Geometry and existing refs are deliberately absent. Child order is significant.
pub(crate) fn refs(
    state: &SemanticPaneState,
    tree: Option<&UiTree>,
    epoch: u128,
) -> HashMap<String, String> {
    let nodes: HashMap<_, _> = state.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let mut keys = HashMap::new();
    let mut actions = HashMap::new();
    if let Some(tree) = tree {
        for node in &tree.nodes {
            keys.insert(node.id.to_string(), identity_key(&node.key));
            let action = match &node.data {
                UiNodeData::Button(b) => serde_json::json!([b.on_click, b.disabled]),
                UiNodeData::TextInput(t) => serde_json::json!([t.on_change, t.on_submit]),
                UiNodeData::ListView(l) => serde_json::json!([l.on_select]),
                _ => serde_json::Value::Null,
            };
            // Host metadata includes table selection/actions and text roles.
            actions.insert(node.id.to_string(), serde_json::json!([node.key, action]));
        }
    }
    let mut parents = HashMap::new();
    for node in &state.nodes {
        for child in &node.children {
            parents.insert(child.as_str(), node.id.as_str());
        }
    }
    let keyed = |id: &str| -> bool {
        let Some(key) = keys.get(id) else { return false };
        let parent_key = parents.get(id).and_then(|parent| keys.get(*parent));
        key.strip_prefix(parent_key.map(String::as_str).unwrap_or(""))
            .is_some_and(|suffix| suffix.starts_with("/@"))
    };
    state.nodes.iter().map(|target| {
        let mut path = HashSet::new();
        let mut scope = target.id.as_str();
        while path.insert(scope) && !keyed(scope) {
            match parents.get(scope) {
                Some(parent) => scope = parent,
                None => break,
            }
        }
        let mut stack = if tree.is_none() {
            let mut ids: Vec<_> = nodes.keys().copied().collect();
            ids.sort_unstable();
            ids.reverse();
            ids
        } else {
            vec![scope]
        };
        let mut seen = HashSet::new();
        let mut digest = Sha256::new();
        digest.update(epoch.to_le_bytes());
        digest.update(keys.get(&target.id).map(String::as_bytes).unwrap_or(target.id.as_bytes()));
        while let Some(id) = stack.pop() {
            if !seen.insert(id) { continue; }
            let Some(node) = nodes.get(id) else { continue };
            let content = if keyed(id) && !path.contains(id) {
                serde_json::json!({"key": keys.get(id)})
            } else {
                if tree.is_some() {
                    stack.extend(node.children.iter().rev().map(String::as_str));
                }
                serde_json::json!({
                    "id": node.id, "role": node.role, "label": node.label,
                    "value": node.value, "children": node.children,
                    "disabled": node.disabled, "toggled": node.toggled, "numeric_value": node.numeric_value,
                    "canvas": node.canvas_commands, "action": actions.get(id),
                })
            };
            let bytes = content.to_string();
            digest.update((bytes.len() as u64).to_le_bytes());
            digest.update(bytes.as_bytes());
        }
        let hash = digest.finalize();
        let digest = u64::from_le_bytes(hash[..8].try_into().expect("SHA256 prefix"));
        (target.id.clone(), format!("{}.{digest:016x}", target.id))
    }).collect()
}
