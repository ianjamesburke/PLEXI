//! Node refs: one control of a plugin pane's committed tree, as a caller read
//! it from `plexi pane state`.
//!
//! A ref is `<id>.<16 hex>`. The id is a lookup hint; the hex is the identity:
//! a hash of the pane's launch epoch, the node's key path, its place in its
//! scope, and every field of every node in that scope. A node's scope is its
//! nearest ancestor-or-self whose key adds an explicit `/@` segment, else the
//! whole tree. A keyed descendant inside a scope contributes only its key, so
//! a change inside one keyed row leaves the refs of its siblings alone.
//!
//! [`resolve`] and [`activation`] are the whole activation contract: a caller
//! resolves the ref on the tree it is about to deliver to, and delivers what
//! [`activation`] derives. The renderer derives a pointer click's action from
//! the same [`activation`], so the two can never mean different things.
use super::wasm_view::bindings::plexi::platform::types::{
    BadgeNode, ButtonNode, CanvasCircle, CanvasCommand, CanvasLine, CanvasNode, CanvasRect,
    CanvasText, Color, ColumnNode, FooterKeyEntry, FooterKeysNode, ListItemNode, ListNode,
    PaddingNode, PinnedNode, ProgressBarNode, RowNode, ScrollNode, SpaceNode, SpinnerNode,
    SurfaceHeaderNode, SurfaceNode, TableNode, TableRowNode, TextInputNode, TextNode,
};
use super::wasm_view::{IndexedNode, UiNodeData, UiTree};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NodeRef {
    pub id: u32,
    pub digest: u64,
}

impl std::fmt::Display for NodeRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{:016x}", self.id, self.digest)
    }
}

impl NodeRef {
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        let malformed = || {
            format!("malformed node ref {value:?}: expected <id>.<16 hex digits>; read it from pane state")
        };
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

/// What activating a node means to the pane that owns it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Activation {
    /// A guest UI action, exactly the handler id a pointer click emits.
    Action(String),
    /// Host focus on the text input with this arena id.
    Focus(u32),
}

/// Why an activation did not complete. A refusal is decided before anything
/// reaches the guest, so retrying is safe; a failure after delivery is not a
/// refusal, because the guest already has the action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ActivationFailure {
    Refused(String),
    DeliveredThenFailed(String),
}

impl From<String> for ActivationFailure {
    fn from(reason: String) -> Self {
        Self::Refused(reason)
    }
}

impl std::fmt::Display for ActivationFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(reason) => f.write_str(reason),
            Self::DeliveredThenFailed(reason) => write!(f, "delivered, then {reason}"),
        }
    }
}

/// The key a node is known by, without host-encoded presentation metadata.
fn identity_key(key: &str) -> String {
    if let Some(key) = key.strip_prefix(super::wasm_render::CARD_NODE_KEY_PREFIX) {
        return key.to_owned();
    }
    if let Some(rest) = key.strip_prefix("__plexi_text_role__:") {
        if let Some((_, key)) = rest.split_once(':') {
            return key.to_owned();
        }
    }
    key.to_owned()
}

/// The part of a tree a pane renders: nodes reachable from the root. A node
/// that names one control has exactly one parent, a unique arena id and a
/// unique key; anything else is ambiguous and gets no ref.
struct TreeIndex<'a> {
    nodes: HashMap<u32, &'a IndexedNode>,
    /// Every reachable parent of a node, in arena order.
    parents: HashMap<u32, Vec<u32>>,
    keys: HashMap<u32, String>,
    /// Nodes (and their descendants) that cannot name exactly one control.
    ambiguous: HashSet<u32>,
}

impl<'a> TreeIndex<'a> {
    fn new(root: u32, arena: &'a [IndexedNode]) -> Self {
        let mut by_id: HashMap<u32, &IndexedNode> = HashMap::new();
        let mut duplicate_ids = HashSet::new();
        for node in arena {
            if by_id.insert(node.id, node).is_some() {
                duplicate_ids.insert(node.id);
            }
        }
        let mut nodes = HashMap::new();
        let mut parents: HashMap<u32, Vec<u32>> = HashMap::new();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            let Some(node) = super::wasm_render::find_node(arena, id) else {
                continue;
            };
            if nodes.insert(id, node).is_some() {
                continue;
            }
            for child in super::wasm_view::children(&node.data) {
                parents.entry(*child).or_default().push(id);
                stack.push(*child);
            }
        }
        let keys: HashMap<u32, String> = nodes
            .iter()
            .map(|(id, node)| (*id, identity_key(&node.key)))
            .collect();
        // An empty key says nothing about identity (position does); two
        // nodes sharing a non-empty key cannot both be the control it names.
        let mut key_count: HashMap<&str, usize> = HashMap::new();
        for key in keys.values().filter(|key| !key.is_empty()) {
            *key_count.entry(key.as_str()).or_default() += 1;
        }
        let mut ambiguous: HashSet<u32> = nodes
            .keys()
            .copied()
            .filter(|id| {
                duplicate_ids.contains(id)
                    || parents.get(id).is_some_and(|p| p.len() > 1)
                    || (*id == root && parents.contains_key(id))
                    || key_count.get(keys[id].as_str()).is_some_and(|n| *n > 1)
            })
            .collect();
        // A node inside an ambiguous subtree is ambiguous too.
        let mut stack: Vec<u32> = ambiguous.iter().copied().collect();
        while let Some(id) = stack.pop() {
            if let Some(node) = nodes.get(&id) {
                for child in super::wasm_view::children(&node.data) {
                    if nodes.contains_key(child) && ambiguous.insert(*child) {
                        stack.push(*child);
                    }
                }
            }
        }
        Self {
            nodes,
            parents,
            keys,
            ambiguous,
        }
    }

    /// The node's single reachable parent, when it has exactly one.
    fn parent(&self, id: u32) -> Option<u32> {
        match self.parents.get(&id).map(Vec::as_slice) {
            Some([parent]) => Some(*parent),
            _ => None,
        }
    }

    /// True when this node's key adds an explicit `/@` segment to its
    /// parent's key path: it is a keyed node and roots its own scope.
    fn keyed(&self, id: u32) -> bool {
        let Some(key) = self.keys.get(&id) else {
            return false;
        };
        let parent_key = self
            .parent(id)
            .and_then(|parent| self.keys.get(&parent))
            .map(String::as_str)
            .unwrap_or("");
        key.strip_prefix(parent_key)
            .is_some_and(|suffix| suffix.starts_with("/@"))
    }

    /// The node's scope root and the child positions leading from it to the
    /// node. Only called for unambiguous nodes, whose parent chain is a path.
    fn scope(&self, id: u32) -> (u32, Vec<u32>) {
        let mut path = Vec::new();
        let mut current = id;
        while !self.keyed(current) {
            let Some(parent) = self.parent(current) else {
                break;
            };
            let position = self
                .nodes
                .get(&parent)
                .and_then(|node| {
                    super::wasm_view::children(&node.data)
                        .iter()
                        .position(|c| *c == current)
                })
                .unwrap_or(0);
            path.push(position as u32);
            current = parent;
        }
        path.reverse();
        (current, path)
    }

    fn scope_digest(&self, root: u32) -> [u8; 32] {
        let mut hasher = FieldHasher::default();
        self.hash_subtree(root, root, &mut hasher, &mut HashSet::new());
        hasher.0.finalize().into()
    }

    fn hash_subtree(&self, id: u32, root: u32, h: &mut FieldHasher, seen: &mut HashSet<u32>) {
        let Some(node) = self.nodes.get(&id) else {
            h.tag(b'M');
            return;
        };
        // A shared or cyclic child (an ambiguous node) is hashed once.
        if !seen.insert(id) {
            h.tag(b'R');
            h.u32(id);
            return;
        }
        if id != root && self.keyed(id) {
            h.tag(b'K');
            h.str(self.keys.get(&id).map(String::as_str).unwrap_or(""));
            return;
        }
        h.tag(b'N');
        let IndexedNode { id: _, key, data } = node;
        h.str(key);
        hash_data(h, data);
        let kids = super::wasm_view::children(data);
        h.u32(kids.len() as u32);
        for child in kids {
            self.hash_subtree(*child, root, h, seen);
        }
    }

    fn digest(&self, id: u32, epoch: u128, scopes: &mut HashMap<u32, [u8; 32]>) -> u64 {
        let (root, path) = self.scope(id);
        let scope = *scopes
            .entry(root)
            .or_insert_with(|| self.scope_digest(root));
        let mut h = FieldHasher::default();
        h.0.update(epoch.to_le_bytes());
        h.str(self.keys.get(&id).map(String::as_str).unwrap_or(""));
        h.u32(path.len() as u32);
        for step in &path {
            h.u32(*step);
        }
        h.0.update(scope);
        // A table row's cells mean what its table's columns say, even when
        // the row is keyed and roots its own scope.
        match self.table_columns(id) {
            Some(columns) => {
                h.tag(1);
                h.u32(columns.len() as u32);
                for column in columns {
                    h.str(column);
                }
            }
            None => h.tag(0),
        }
        let hash = h.0.finalize();
        let mut prefix = [0u8; 8];
        prefix.copy_from_slice(&hash[..8]);
        u64::from_le_bytes(prefix)
    }

    /// The columns of the table a table row sits in.
    fn table_columns(&self, id: u32) -> Option<&[String]> {
        if !matches!(self.nodes.get(&id)?.data, UiNodeData::TableRow(_)) {
            return None;
        }
        match &self.nodes.get(&self.parent(id)?)?.data {
            UiNodeData::Table(table) => Some(&table.columns),
            _ => None,
        }
    }

    /// Digest of every unambiguous reachable node.
    fn digests(&self, epoch: u128) -> HashMap<u32, u64> {
        let mut scopes = HashMap::new();
        self.nodes
            .keys()
            .filter(|id| !self.ambiguous.contains(id))
            .map(|id| (*id, self.digest(*id, epoch, &mut scopes)))
            .collect()
    }
}

/// Every node's ref on `tree`, minted for a pane whose launch epoch is
/// `epoch`. Only nodes the root reaches get one, and only when the ref names
/// exactly one control: an ambiguous node or a shared identity gets none.
pub(crate) fn refs(tree: &UiTree, epoch: u128) -> HashMap<u32, NodeRef> {
    let digests = TreeIndex::new(tree.root, &tree.nodes).digests(epoch);
    let mut count: HashMap<u64, usize> = HashMap::new();
    for digest in digests.values() {
        *count.entry(*digest).or_default() += 1;
    }
    digests
        .into_iter()
        .filter(|(_, digest)| count[digest] == 1)
        .map(|(id, digest)| (id, NodeRef { id, digest }))
        .collect()
}

/// The arena id `reference` names on `tree`, or why it names nothing there.
/// Identity decides; the id only explains a miss.
pub(crate) fn resolve(tree: &UiTree, epoch: u128, reference: &NodeRef) -> Result<u32, String> {
    let index = TreeIndex::new(tree.root, &tree.nodes);
    let digests = index.digests(epoch);
    let mut matches = digests
        .iter()
        .filter(|(_, digest)| **digest == reference.digest)
        .map(|(id, _)| *id);
    match (matches.next(), matches.next()) {
        (Some(id), None) => return Ok(id),
        (Some(_), Some(_)) => {
            return Err(format!(
                "ref {reference} is ambiguous: more than one node has its identity. Re-read pane state."
            ))
        }
        (None, _) => {}
    }
    let now = match index.nodes.get(&reference.id) {
        Some(_) if index.ambiguous.contains(&reference.id) => {
            "is ambiguous (shared key, id or parent) and has no ref".to_string()
        }
        Some(node) => {
            let semantic = super::pane::SemanticPaneNode::from_indexed(node);
            match semantic.label {
                Some(label) => format!("is now {} {label:?}", semantic.role),
                None => format!("is now {}", semantic.role),
            }
        }
        None => "is gone".to_string(),
    };
    Err(format!(
        "ref {reference} is stale: node {} {now}. Re-read pane state.",
        reference.id
    ))
}

/// What activating node `id` of the tree rooted at `root` delivers. Every
/// row describes itself: its activation is its own field, never derived from
/// a parent or a position.
pub(crate) fn activation(root: u32, nodes: &[IndexedNode], id: u32) -> Result<Activation, String> {
    let index = TreeIndex::new(root, nodes);
    let Some(node) = index.nodes.get(&id) else {
        return Err(format!("node {id} is not in the rendered tree"));
    };
    let own_action = |kind: &str, on_select: &Option<String>| match on_select
        .as_deref()
        .filter(|action| !action.is_empty())
    {
        Some(action) => Ok(Activation::Action(action.to_string())),
        None => Err(format!("node {id} is a {kind} with no on_select")),
    };
    match &node.data {
        UiNodeData::Button(button) if button.disabled => Err(format!(
            "node {id} is button {:?}, which is disabled",
            button.label
        )),
        UiNodeData::Button(button) => Ok(Activation::Action(button.on_click.clone())),
        UiNodeData::ListItem(item) => own_action("list item", &item.on_select),
        UiNodeData::TableRow(row) => own_action("table row", &row.on_select),
        UiNodeData::TextInput(_) => Ok(Activation::Focus(id)),
        _ => Err(format!(
            "node {id} is not interactive (role: {})",
            super::pane::SemanticPaneNode::from_indexed(node).role
        )),
    }
}

/// Feeds node fields into SHA-256 with lengths and tags, so no two different
/// field sequences hash the same bytes.
#[derive(Default)]
struct FieldHasher(Sha256);

impl FieldHasher {
    fn tag(&mut self, tag: u8) {
        self.0.update([tag]);
    }
    fn str(&mut self, value: &str) {
        self.0.update((value.len() as u64).to_le_bytes());
        self.0.update(value.as_bytes());
    }
    fn bool(&mut self, value: bool) {
        self.0.update([u8::from(value)]);
    }
    fn u32(&mut self, value: u32) {
        self.0.update(value.to_le_bytes());
    }
    fn f32(&mut self, value: f32) {
        self.0.update(value.to_bits().to_le_bytes());
    }
    fn color(&mut self, value: &Color) {
        let Color { r, g, b, a } = *value;
        self.0.update([r, g, b, a]);
    }
    fn opt<T>(&mut self, value: Option<T>, mut hash: impl FnMut(&mut Self, T)) {
        match value {
            Some(value) => {
                self.tag(1);
                hash(self, value);
            }
            None => self.tag(0),
        }
    }
}

/// Every field of the node's data. Each record is destructured without `..`,
/// so a new WIT field does not compile until it is hashed here. Child ids are
/// arena positions, not content: the subtree walk hashes children instead.
fn hash_data(h: &mut FieldHasher, data: &UiNodeData) {
    match data {
        UiNodeData::Empty => h.tag(0),
        UiNodeData::Text(TextNode {
            text,
            size,
            bold,
            color,
            truncate,
            align,
        }) => {
            h.tag(1);
            h.str(text);
            h.opt(*size, FieldHasher::f32);
            h.bool(*bold);
            h.opt(color.as_ref(), FieldHasher::color);
            h.bool(*truncate);
            h.tag(*align as u8);
        }
        UiNodeData::Button(ButtonNode {
            label,
            on_click,
            style,
            disabled,
        }) => {
            h.tag(2);
            h.str(label);
            h.str(on_click);
            h.tag(*style as u8);
            h.bool(*disabled);
        }
        UiNodeData::TextInput(TextInputNode {
            value,
            placeholder,
            on_change,
            on_submit,
            password,
            autofocus,
        }) => {
            h.tag(3);
            // A password value never enters a hash a caller can read.
            h.str(if *password { "" } else { value });
            h.str(placeholder);
            h.str(on_change);
            h.str(on_submit);
            h.bool(*password);
            h.bool(*autofocus);
        }
        UiNodeData::Row(RowNode {
            children: _,
            gap,
            align,
            justify,
            grow,
        }) => {
            h.tag(4);
            h.f32(*gap);
            h.tag(*align as u8);
            h.tag(*justify as u8);
            h.bool(*grow);
        }
        UiNodeData::Column(ColumnNode {
            children: _,
            gap,
            align,
            justify,
            grow,
        }) => {
            h.tag(5);
            h.f32(*gap);
            h.tag(*align as u8);
            h.tag(*justify as u8);
            h.bool(*grow);
        }
        UiNodeData::ProgressBar(ProgressBarNode {
            value,
            max,
            color,
            label,
        }) => {
            h.tag(6);
            h.f32(*value);
            h.f32(*max);
            h.opt(color.as_ref(), FieldHasher::color);
            h.opt(label.as_deref(), FieldHasher::str);
        }
        UiNodeData::Badge(BadgeNode { text, color }) => {
            h.tag(7);
            h.str(text);
            h.tag(*color as u8);
        }
        UiNodeData::ListView(ListNode { items: _ }) => h.tag(8),
        UiNodeData::ListItem(ListItemNode {
            child: _,
            selected,
            on_select,
        }) => {
            h.tag(19);
            h.bool(*selected);
            h.opt(on_select.as_deref(), FieldHasher::str);
        }
        UiNodeData::Table(TableNode { columns, rows: _ }) => {
            h.tag(20);
            h.u32(columns.len() as u32);
            for column in columns {
                h.str(column);
            }
        }
        UiNodeData::TableRow(TableRowNode {
            cells,
            selected,
            on_select,
        }) => {
            h.tag(21);
            h.u32(cells.len() as u32);
            for cell in cells {
                h.str(cell);
            }
            h.bool(*selected);
            h.opt(on_select.as_deref(), FieldHasher::str);
        }
        UiNodeData::Scroll(ScrollNode {
            child: _,
            horizontal,
        }) => {
            h.tag(9);
            h.bool(*horizontal);
        }
        UiNodeData::Padding(PaddingNode {
            child: _,
            top,
            right,
            bottom,
            left,
        }) => {
            h.tag(10);
            for side in [top, right, bottom, left] {
                h.f32(*side);
            }
        }
        UiNodeData::Canvas(CanvasNode {
            width,
            height,
            grow,
            commands,
        }) => {
            h.tag(11);
            h.f32(*width);
            h.f32(*height);
            h.bool(*grow);
            h.u32(commands.len() as u32);
            for command in commands {
                hash_canvas_command(h, command);
            }
        }
        UiNodeData::Divider => h.tag(12),
        UiNodeData::Space(SpaceNode { size, grow }) => {
            h.tag(13);
            h.f32(*size);
            h.bool(*grow);
        }
        UiNodeData::Surface(SurfaceNode {
            width,
            height,
            texture_handle,
        }) => {
            h.tag(14);
            h.u32(*width);
            h.u32(*height);
            h.opt(*texture_handle, |h, handle: u64| {
                h.0.update(handle.to_le_bytes())
            });
        }
        UiNodeData::SurfaceHeader(SurfaceHeaderNode { title, subtitle }) => {
            h.tag(15);
            h.str(title);
            h.str(subtitle);
        }
        UiNodeData::FooterKeys(FooterKeysNode { entries, divider }) => {
            h.tag(16);
            h.u32(entries.len() as u32);
            for FooterKeyEntry { keys, description } in entries {
                h.u32(keys.len() as u32);
                for key in keys {
                    h.str(key);
                }
                h.str(description);
            }
            h.bool(*divider);
        }
        UiNodeData::Pinned(PinnedNode { edge, child: _ }) => {
            h.tag(17);
            h.tag(*edge as u8);
        }
        UiNodeData::Spinner(SpinnerNode { label }) => {
            h.tag(18);
            h.str(label);
        }
    }
}

fn hash_canvas_command(h: &mut FieldHasher, command: &CanvasCommand) {
    match command {
        CanvasCommand::Rect(CanvasRect {
            x,
            y,
            width,
            height,
            fill,
            radius,
        }) => {
            h.tag(0);
            for value in [x, y, width, height] {
                h.f32(*value);
            }
            h.color(fill);
            h.f32(*radius);
        }
        CanvasCommand::Circle(CanvasCircle { x, y, radius, fill }) => {
            h.tag(1);
            for value in [x, y, radius] {
                h.f32(*value);
            }
            h.color(fill);
        }
        CanvasCommand::Line(CanvasLine {
            x1,
            y1,
            x2,
            y2,
            width,
            color,
        }) => {
            h.tag(2);
            for value in [x1, y1, x2, y2, width] {
                h.f32(*value);
            }
            h.color(color);
        }
        CanvasCommand::Text(CanvasText {
            x,
            y,
            text,
            size,
            color,
            bold,
            align,
        }) => {
            h.tag(3);
            h.f32(*x);
            h.f32(*y);
            h.str(text);
            h.f32(*size);
            h.color(color);
            h.bool(*bold);
            h.tag(*align as u8);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::wasm_view::bindings::plexi::platform::types::{
        Alignment, ButtonStyle, Justify,
    };

    fn node(id: u32, key: &str, data: UiNodeData) -> IndexedNode {
        IndexedNode {
            id,
            key: key.to_string(),
            data,
        }
    }

    fn column(children: Vec<u32>) -> UiNodeData {
        UiNodeData::Column(ColumnNode {
            children,
            gap: 4.0,
            align: Alignment::Start,
            justify: Justify::Start,
            grow: false,
        })
    }

    fn text(value: &str) -> UiNodeData {
        UiNodeData::Text(TextNode {
            text: value.to_string(),
            size: None,
            bold: false,
            color: None,
            truncate: false,
            align: Alignment::Start,
        })
    }

    fn button(label: &str, on_click: &str) -> UiNodeData {
        UiNodeData::Button(ButtonNode {
            label: label.to_string(),
            on_click: on_click.to_string(),
            style: ButtonStyle::Secondary,
            disabled: false,
        })
    }

    fn input() -> UiNodeData {
        UiNodeData::TextInput(TextInputNode {
            value: "draft".to_string(),
            placeholder: "Name".to_string(),
            on_change: "name_changed".to_string(),
            on_submit: "name_submitted".to_string(),
            password: false,
            autofocus: false,
        })
    }

    fn tree(nodes: Vec<IndexedNode>) -> UiTree {
        UiTree { root: 0, nodes }
    }

    fn ref_of(tree: &UiTree, id: u32) -> NodeRef {
        refs(tree, 7)[&id]
    }

    /// Keyed rows under an unkeyed toolbar, as the Python SDK keys them.
    fn rows(row_keys: &[&str]) -> UiTree {
        let mut nodes = vec![node(1, "0/0", button("Refresh", "refresh"))];
        let mut root_children = vec![1];
        let mut next = 2;
        for key in row_keys {
            let row = next;
            let action = next + 1;
            next += 2;
            nodes.push(node(row, &format!("0/@{key}"), column(vec![action])));
            nodes.push(node(
                action,
                &format!("0/@{key}/0"),
                button(&format!("Approve {key}"), &format!("approve:{key}")),
            ));
            root_children.push(row);
        }
        nodes.insert(0, node(0, "0", column(root_children)));
        tree(nodes)
    }

    fn action_ref(tree: &UiTree, key: &str) -> u64 {
        let id = tree
            .nodes
            .iter()
            .find(|n| n.key == format!("0/@{key}/0"))
            .expect("row action")
            .id;
        ref_of(tree, id).digest
    }

    #[test]
    fn a_replaced_control_at_the_same_id_and_key_gets_a_different_ref() {
        let approve = tree(vec![
            node(0, "0", column(vec![1])),
            node(1, "0/0", button("Approve A", "approve")),
        ]);
        let delete = tree(vec![
            node(0, "0", column(vec![1])),
            node(1, "0/0", button("Delete B", "delete")),
        ]);
        let reference = ref_of(&approve, 1);
        assert_ne!(reference, ref_of(&delete, 1));
        let error = resolve(&delete, 7, &reference).expect_err("stale ref");
        assert!(error.contains("stale"), "{error}");
        assert!(error.contains("button \"Delete B\""), "{error}");
        assert_eq!(resolve(&approve, 7, &reference), Ok(1));

        // Unkeyed siblings are part of the control's meaning.
        let allow_for = |subject: &str| {
            tree(vec![
                node(0, "0", column(vec![1, 2])),
                node(1, "0/0", text(subject)),
                node(2, "0/1", button("Allow", "allow")),
            ])
        };
        assert_ne!(
            ref_of(&allow_for("Text A"), 2),
            ref_of(&allow_for("Text B"), 2)
        );

        // Another pane (another epoch) never shares a ref.
        assert_ne!(refs(&approve, 7)[&1], refs(&approve, 8)[&1]);
    }

    #[test]
    fn keyed_rows_keep_their_refs_and_the_toolbar_tracks_the_row_set() {
        let ab = rows(&["a", "b"]);
        let mut ab_changed = rows(&["a", "b"]);
        for n in &mut ab_changed.nodes {
            if n.key == "0/@b/0" {
                n.data = button("Delete b", "delete:b");
            }
        }
        let abc = rows(&["a", "b", "c"]);
        let ca = rows(&["c", "a"]);
        let a = rows(&["a"]);
        let ba = rows(&["b", "a"]);

        for other in [&ab_changed, &abc, &ca, &a, &ba] {
            assert_eq!(action_ref(&ab, "a"), action_ref(other, "a"));
        }
        // Row a moved in the arena: the id hint misses, the identity finds it.
        let moved = ca.nodes.iter().find(|n| n.key == "0/@a/0").unwrap().id;
        let original = ab.nodes.iter().find(|n| n.key == "0/@a/0").unwrap().id;
        assert_ne!(moved, original);
        assert_eq!(resolve(&ca, 7, &ref_of(&ab, original)), Ok(moved));

        assert_eq!(
            ref_of(&ab, 1),
            ref_of(&ab_changed, 1),
            "row content is not toolbar scope"
        );
        for other in [&abc, &a, &ba] {
            assert_ne!(
                ref_of(&ab, 1),
                ref_of(other, 1),
                "added, removed or reordered rows"
            );
        }
    }

    #[test]
    fn every_field_of_every_node_in_scope_changes_the_ref() {
        let base = || {
            tree(vec![
                node(0, "0", column(vec![1, 2, 3])),
                node(1, "0/0", text("Subject")),
                node(2, "0/1", button("Approve", "approve")),
                node(3, "0/2", input()),
            ])
        };
        type Edit = fn(&mut UiNodeData);
        let edits: Vec<(&str, u32, Edit)> = vec![
            ("button label", 2, |d| {
                if let UiNodeData::Button(b) = d {
                    b.label.push('!')
                }
            }),
            ("button on_click", 2, |d| {
                if let UiNodeData::Button(b) = d {
                    b.on_click.push('!')
                }
            }),
            ("button style", 2, |d| {
                if let UiNodeData::Button(b) = d {
                    b.style = ButtonStyle::Primary
                }
            }),
            ("button disabled", 2, |d| {
                if let UiNodeData::Button(b) = d {
                    b.disabled = true
                }
            }),
            ("text text", 1, |d| {
                if let UiNodeData::Text(t) = d {
                    t.text.push('!')
                }
            }),
            ("text size", 1, |d| {
                if let UiNodeData::Text(t) = d {
                    t.size = Some(20.0)
                }
            }),
            ("text bold", 1, |d| {
                if let UiNodeData::Text(t) = d {
                    t.bold = true
                }
            }),
            ("text color", 1, |d| {
                if let UiNodeData::Text(t) = d {
                    t.color = Some(Color {
                        r: 1,
                        g: 2,
                        b: 3,
                        a: 4,
                    })
                }
            }),
            ("text truncate", 1, |d| {
                if let UiNodeData::Text(t) = d {
                    t.truncate = true
                }
            }),
            ("text align", 1, |d| {
                if let UiNodeData::Text(t) = d {
                    t.align = Alignment::End
                }
            }),
            ("input value", 3, |d| {
                if let UiNodeData::TextInput(t) = d {
                    t.value.push('!')
                }
            }),
            ("input placeholder", 3, |d| {
                if let UiNodeData::TextInput(t) = d {
                    t.placeholder.push('!')
                }
            }),
            ("input on_change", 3, |d| {
                if let UiNodeData::TextInput(t) = d {
                    t.on_change.push('!')
                }
            }),
            ("input on_submit", 3, |d| {
                if let UiNodeData::TextInput(t) = d {
                    t.on_submit.push('!')
                }
            }),
            ("input password", 3, |d| {
                if let UiNodeData::TextInput(t) = d {
                    t.password = true
                }
            }),
            ("input autofocus", 3, |d| {
                if let UiNodeData::TextInput(t) = d {
                    t.autofocus = true
                }
            }),
            ("column gap", 0, |d| {
                if let UiNodeData::Column(c) = d {
                    c.gap = 9.0
                }
            }),
            ("column align", 0, |d| {
                if let UiNodeData::Column(c) = d {
                    c.align = Alignment::Center
                }
            }),
            ("column justify", 0, |d| {
                if let UiNodeData::Column(c) = d {
                    c.justify = Justify::End
                }
            }),
            ("column grow", 0, |d| {
                if let UiNodeData::Column(c) = d {
                    c.grow = true
                }
            }),
        ];
        let before = ref_of(&base(), 2);
        for (field, id, edit) in edits {
            let mut changed = base();
            edit(&mut changed.nodes[id as usize].data);
            assert_ne!(before, ref_of(&changed, 2), "{field} must change the ref");
        }
        let mut rekeyed = base();
        rekeyed.nodes[1].key = "0/9".to_string();
        assert_ne!(
            before,
            ref_of(&rekeyed, 2),
            "a node key must change the ref"
        );

        // A password value never enters the hash.
        let secret = |value: &str| {
            let mut tree = base();
            if let UiNodeData::TextInput(t) = &mut tree.nodes[3].data {
                t.password = true;
                t.value = value.to_string();
            }
            tree
        };
        assert_eq!(ref_of(&secret("hunter2"), 2), ref_of(&secret("other"), 2));
        assert_eq!(ref_of(&secret("hunter2"), 3), ref_of(&secret("other"), 3));
    }

    fn list_item(child: u32, selected: bool, on_select: Option<&str>) -> UiNodeData {
        UiNodeData::ListItem(ListItemNode {
            child,
            selected,
            on_select: on_select.map(str::to_string),
        })
    }

    fn table_row(cells: &[&str], selected: bool, on_select: Option<&str>) -> UiNodeData {
        UiNodeData::TableRow(TableRowNode {
            cells: cells.iter().map(|c| c.to_string()).collect(),
            selected,
            on_select: on_select.map(str::to_string),
        })
    }

    #[test]
    fn activation_derives_the_action_of_each_interactive_kind() {
        let mut disabled = button("Off", "off");
        if let UiNodeData::Button(b) = &mut disabled {
            b.disabled = true;
        }
        let nodes = vec![
            node(0, "0", column(vec![1, 2, 7, 8, 9, 10])),
            node(1, "0/0", button("Go", "go")),
            node(
                2,
                "0/1",
                UiNodeData::ListView(ListNode {
                    items: vec![3, 4, 5],
                }),
            ),
            node(3, "0/1/@a", list_item(11, false, Some("pick:a"))),
            node(4, "0/1/@b", list_item(12, true, None)),
            node(5, "0/1/@c", list_item(13, false, Some("pick:c"))),
            node(
                7,
                "0/2",
                UiNodeData::Table(TableNode {
                    columns: vec![],
                    rows: vec![14],
                }),
            ),
            node(8, "0/3", input()),
            node(9, "0/4", disabled),
            node(10, "0/5", text("plain")),
            node(11, "0/1/@a/child", text("A")),
            node(12, "0/1/@b/child", text("B")),
            node(13, "0/1/@c/child", button("Edit C", "edit:c")),
            node(
                14,
                "0/2/0",
                table_row(&["only row"], false, Some("open:row-1")),
            ),
        ];
        let action = |s: &str| Ok(Activation::Action(s.to_string()));
        assert_eq!(activation(0, &nodes, 1), action("go"));
        // Rows deliver their own complete action; a headerless table is no
        // different from a headed one.
        assert_eq!(activation(0, &nodes, 3), action("pick:a"));
        assert_eq!(activation(0, &nodes, 14), action("open:row-1"));
        // A button inside a list row keeps its own action.
        assert_eq!(activation(0, &nodes, 13), action("edit:c"));
        assert_eq!(activation(0, &nodes, 8), Ok(Activation::Focus(8)));
        for (id, why) in [
            (4, "no on_select"),
            (9, "disabled"),
            (10, "not interactive"),
            (2, "not interactive"),
            (7, "not interactive"),
            (99, "not in the rendered tree"),
        ] {
            let error = activation(0, &nodes, id).expect_err("must refuse");
            assert!(error.contains(why), "node {id}: {error}");
        }
    }

    /// A list of keyed rows, as the SDK keys them: `(key, label, selected,
    /// on_select)` per row, ids assigned in order.
    fn keyed_list(rows: &[(&str, &str, bool, &str)]) -> UiTree {
        let n = rows.len() as u32;
        let mut nodes = vec![node(
            0,
            "0",
            UiNodeData::ListView(ListNode {
                items: (1..=n).collect(),
            }),
        )];
        for (i, (key, label, selected, on_select)) in rows.iter().enumerate() {
            let item = i as u32 + 1;
            let child = item + n;
            nodes.push(node(
                item,
                &format!("0/@{key}"),
                list_item(child, *selected, Some(on_select)),
            ));
            nodes.push(node(child, &format!("0/@{key}/child"), text(label)));
        }
        tree(nodes)
    }

    #[test]
    fn a_row_ref_covers_its_own_activation_and_survives_its_siblings() {
        let base = keyed_list(&[("a", "A", false, "pick:a"), ("b", "B", false, "pick:b")]);
        let reference = ref_of(&base, 1);
        let resolves = |tree: &UiTree| resolve(tree, 7, &reference);
        let row_a = |tree: &UiTree| tree.nodes.iter().find(|n| n.key == "0/@a").unwrap().id;
        for (what, other) in [
            (
                "sibling changed",
                keyed_list(&[("a", "A", false, "pick:a"), ("b", "B2", true, "pick:b")]),
            ),
            (
                "sibling added",
                keyed_list(&[
                    ("a", "A", false, "pick:a"),
                    ("b", "B", false, "pick:b"),
                    ("c", "C", false, "pick:c"),
                ]),
            ),
            (
                "sibling removed",
                keyed_list(&[("a", "A", false, "pick:a")]),
            ),
            (
                "reordered",
                keyed_list(&[("b", "B", false, "pick:b"), ("a", "A", false, "pick:a")]),
            ),
        ] {
            assert_eq!(resolves(&other), Ok(row_a(&other)), "{what}");
        }
        for (what, other) in [
            (
                "its action",
                keyed_list(&[("a", "A", false, "delete:a"), ("b", "B", false, "pick:b")]),
            ),
            (
                "its selection",
                keyed_list(&[("a", "A", true, "pick:a"), ("b", "B", false, "pick:b")]),
            ),
            (
                "what it paints",
                keyed_list(&[
                    ("a", "Delete A", false, "pick:a"),
                    ("b", "B", false, "pick:b"),
                ]),
            ),
        ] {
            let error = resolves(&other).expect_err(what);
            assert!(error.contains("stale"), "{what}: {error}");
        }

        // A table row's cells are what it paints.
        let table = |cells: &[&str], extra: bool| {
            let mut rows = vec![1];
            let mut nodes = vec![node(1, "0/@r1", table_row(cells, false, Some("open:r1")))];
            if extra {
                rows.push(2);
                nodes.push(node(
                    2,
                    "0/@r2",
                    table_row(&["other"], false, Some("open:r2")),
                ));
            }
            nodes.insert(
                0,
                node(
                    0,
                    "0",
                    UiNodeData::Table(TableNode {
                        columns: vec!["Name".into()],
                        rows,
                    }),
                ),
            );
            tree(nodes)
        };
        let row = ref_of(&table(&["Approve"], false), 1);
        assert_eq!(resolve(&table(&["Approve"], true), 7, &row), Ok(1));
        assert!(resolve(&table(&["Delete"], false), 7, &row).is_err());

        // Swapping only the headers changes what the row's cells mean.
        let headed = |columns: [&str; 2]| {
            tree(vec![
                node(
                    0,
                    "0",
                    UiNodeData::Table(TableNode {
                        columns: columns.iter().map(|c| c.to_string()).collect(),
                        rows: vec![1],
                    }),
                ),
                node(1, "0/@r1", table_row(&["a", "b"], false, Some("move:r1"))),
            ])
        };
        let moved = ref_of(&headed(["Source", "Destination"]), 1);
        let error = resolve(&headed(["Destination", "Source"]), 7, &moved)
            .expect_err("swapped headers change the row's meaning");
        assert!(error.contains("stale"), "{error}");
    }

    #[test]
    fn a_shared_identity_gets_no_ref_and_never_resolves() {
        // Row 1 is unique here; then a sibling takes the same key and content,
        // leaving row 1's own digest and id exactly as they were.
        let before = keyed_list(&[
            ("same", "Same", false, "pick"),
            ("other", "Same", false, "pick"),
        ]);
        let reference = ref_of(&before, 1);
        let two = keyed_list(&[
            ("same", "Same", false, "pick"),
            ("same", "Same", false, "pick"),
        ]);
        assert!(refs(&two, 7).get(&1).is_none() && refs(&two, 7).get(&2).is_none());
        // The exact-id hint does not bypass the ambiguity.
        let error = resolve(&two, 7, &reference).expect_err("ambiguous");
        assert!(error.contains("ambiguous"), "{error}");
    }

    #[test]
    fn only_nodes_the_root_reaches_once_get_refs() {
        let base = tree(vec![
            node(0, "0", column(vec![1])),
            node(1, "0/0", button("Approve", "approve")),
            node(2, "0/9", button("Orphan", "orphan")),
        ]);
        let minted = refs(&base, 7);
        assert!(minted.contains_key(&1));
        assert!(!minted.contains_key(&2), "unreachable nodes get no ref");
        assert!(activation(0, &base.nodes, 2).is_err());

        // A new root that no longer reaches the old subtree.
        let mut rerooted = tree(vec![
            node(0, "0", column(vec![1])),
            node(1, "0/0", button("Approve", "approve")),
            node(3, "1", column(vec![])),
        ]);
        rerooted.root = 3;
        assert!(resolve(&rerooted, 7, &minted[&1]).is_err());

        // One button under two parents names two controls.
        let shared = tree(vec![
            node(0, "0", column(vec![1, 2])),
            node(1, "0/0", column(vec![3])),
            node(2, "0/1", column(vec![3])),
            node(3, "0/x", button("Approve", "approve")),
        ]);
        assert!(!refs(&shared, 7).contains_key(&3));
        // A cycle is ambiguous and terminates.
        let cyclic = tree(vec![
            node(0, "0", column(vec![1])),
            node(1, "0/0", column(vec![0])),
        ]);
        assert!(refs(&cyclic, 7).is_empty());
    }

    #[test]
    fn a_failure_after_delivery_is_never_reported_as_a_refusal() {
        let refused = ActivationFailure::from("ref 1.00 is stale".to_string());
        assert_eq!(refused.to_string(), "ref 1.00 is stale");
        let failed = ActivationFailure::DeliveredThenFailed("the view failed".to_string());
        assert_eq!(failed.to_string(), "delivered, then the view failed");
    }

    #[test]
    fn a_ref_must_have_the_minted_shape() {
        assert_eq!(
            NodeRef::parse("12.00000000000000ff"),
            Ok(NodeRef {
                id: 12,
                digest: 255
            })
        );
        for bad in [
            "12",
            "12.ff",
            "x.00000000000000ff",
            "12.0000000000000zff",
            "",
        ] {
            let error = NodeRef::parse(bad).expect_err(bad);
            assert!(error.contains("malformed node ref"), "{error}");
        }
    }
}
