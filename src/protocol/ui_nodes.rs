use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use crate::protocol::commands::RenderCommand;

// ── Component tree wire types (PGAP v3.5) ────────────────────────────────────

fn default_true() -> bool {
    true
}

fn default_space_xl() -> f32 {
    24.0 // SPACE_XL — keep in sync with src/ui/style.rs
}

/// Single shortcut entry for `UiNode::FooterKeys`.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
pub struct FooterKeyEntry {
    pub keys: Vec<String>,
    pub description: String,
}

/// Item in a `UiNode::SelectList`.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
pub struct SelectListItem {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub leading: String,
    #[serde(default)]
    pub trailing: String,
}

/// Flex direction for a `UiNode::Stack`.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum StackDirection {
    Vertical,
    Horizontal,
}

impl Default for StackDirection {
    fn default() -> Self {
        Self::Vertical
    }
}

/// Per-side padding for a `UiNode::Stack`.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Default, PartialEq)]
pub struct UiPadding {
    #[serde(default)]
    pub top: f32,
    #[serde(default)]
    pub right: f32,
    #[serde(default)]
    pub bottom: f32,
    #[serde(default)]
    pub left: f32,
}

/// Which edge to pin a child against within a vertical Stack.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PinnedEdge {
    Bottom,
    Top,
    Left,
    Right,
}

/// Component tree node. L0 primitives compose into rich UI; L1 sugar variants
/// are rendered natively by the host.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UiNode {
    // ── L0 primitives ────────────────────────────────────────────────────
    /// Flex container — vertical or horizontal.
    Stack {
        #[serde(default)]
        direction: StackDirection,
        children: Vec<UiNode>,
        #[serde(default)]
        gap: f32,
        #[serde(default)]
        padding: UiPadding,
    },
    /// Scrollable single-child container.
    Scroll {
        child: Box<UiNode>,
        #[serde(default)]
        horizontal: bool,
    },
    /// Z-stack overlay — children rendered back-to-front at the same position.
    Layer { children: Vec<UiNode> },
    /// Inline text node.
    Text {
        text: String,
        /// 0.0 means inherit from context.
        #[serde(default)]
        size: f32,
        #[serde(default)]
        color: String,
        #[serde(default)]
        bold: bool,
        #[serde(default)]
        monospace: bool,
    },
    /// Interaction wrapper — host fires `ComponentEvent` for click/hover.
    Interactive {
        node_id: String,
        child: Box<UiNode>,
        #[serde(default)]
        on_click: bool,
        #[serde(default)]
        on_hover: bool,
    },
    /// Escape hatch: embed a flat `RenderCommand` inside the tree.
    Raw { command: Box<RenderCommand> },
    /// Future GPU surface placeholder — reserved, not yet rendered.
    Surface { id: String },
    /// Pinned layout wrapper. In a vertical Stack, `Pinned { edge: Bottom }` children
    /// are rendered flush to the available rect bottom regardless of body content height.
    Pinned {
        edge: PinnedEdge,
        child: Box<UiNode>,
    },
    /// Semantic column container with sticky-footer host contract.
    /// `padding` sets left/right/bottom margins (default SPACE_XL); `padding_top`
    /// sets the top margin independently. Emitted by `Column.to_node()` in the SDK.
    Column {
        children: Vec<UiNode>,
        #[serde(default)]
        gap: f32,
        #[serde(default)]
        padding_top: f32,
        #[serde(default = "default_space_xl")]
        padding: f32,
    },

    // ── L1 sugar ─────────────────────────────────────────────────────────
    /// Host-rendered button.
    Button {
        node_id: String,
        label: String,
        #[serde(default)]
        disabled: bool,
    },
    /// Host-rendered text input.
    Input {
        node_id: String,
        #[serde(default)]
        placeholder: String,
        value: String,
    },
    /// Host-rendered text editor with multiline and max_length support.
    ///
    /// The host maintains a persistent buffer keyed on `node_id`. On each
    /// frame the app sends its last-known `value`; the host seeds the buffer
    /// from it only when a new `node_id` appears. Typing fires
    /// `ComponentEvent { event_type: "change", payload: { value } }` and
    /// Enter (single-line) or Cmd+Enter (multiline) fires
    /// `ComponentEvent { event_type: "submit", payload: { value } }`.
    TextEdit {
        node_id: String,
        #[serde(default)]
        placeholder: String,
        #[serde(default)]
        value: String,
        #[serde(default)]
        multiline: bool,
        /// 0 means no limit.
        #[serde(default)]
        max_length: usize,
    },
    /// Host-rendered pill badge.
    Badge {
        label: String,
        #[serde(default)]
        fill: String,
        #[serde(default)]
        fg: String,
    },
    /// Coloured dot indicator.
    Dot {
        color: String,
        /// 0.0 means default size.
        #[serde(default)]
        size: f32,
    },

    // ── L1 layout components ────────────────────────────────────────────
    /// App title bar with optional subtitle.
    AppBar {
        title: String,
        #[serde(default)]
        subtitle: String,
    },
    /// Keyboard shortcut hints row at bottom of pane.
    FooterKeys {
        entries: Vec<FooterKeyEntry>,
        #[serde(default = "default_true")]
        divider: bool,
    },
    /// Single-line status footer with optional color.
    Footer {
        text: String,
        #[serde(default)]
        color: String,
    },
    /// Section header label (small, uppercase, with rule below).
    Section {
        title: String,
    },
    /// Themed text label with semantic tone.
    Label {
        text: String,
        #[serde(default)]
        size: f32,
        #[serde(default)]
        color: String,
        #[serde(default)]
        tone: String,
        #[serde(default)]
        bold: bool,
        #[serde(default)]
        monospace: bool,
        #[serde(default)]
        max_lines: usize,
    },
    /// Flexible space between siblings.
    Spacer {
        #[serde(default)]
        size: f32,
        #[serde(default)]
        grow: bool,
    },
    /// Horizontal divider rule.
    Divider {
        #[serde(default)]
        color: String,
    },
    /// Bordered card container.
    Card {
        children: Vec<UiNode>,
        #[serde(default)]
        padding: f32,
    },
    /// Keyboard-navigable scrollable list (selection managed by app).
    SelectList {
        items: Vec<SelectListItem>,
        #[serde(default)]
        selected_idx: usize,
    },
}

// Manual PartialEq impl — UiNode::Raw wraps RenderCommand which would require
// PartialEq on hundreds of transitive types. Raw nodes are structural escape
// hatches; we compare them via their serde round-trip JSON representation so
// equality is still meaningful without cascading the derive requirement.
impl PartialEq for UiNode {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (UiNode::Stack { direction: d1, children: c1, gap: g1, padding: p1 },
             UiNode::Stack { direction: d2, children: c2, gap: g2, padding: p2 }) => {
                d1 == d2 && c1 == c2 && g1 == g2 && p1 == p2
            }
            (UiNode::Scroll { child: c1, horizontal: h1 },
             UiNode::Scroll { child: c2, horizontal: h2 }) => c1 == c2 && h1 == h2,
            (UiNode::Layer { children: c1 }, UiNode::Layer { children: c2 }) => c1 == c2,
            (UiNode::Text { text: t1, size: s1, color: co1, bold: b1, monospace: m1 },
             UiNode::Text { text: t2, size: s2, color: co2, bold: b2, monospace: m2 }) => {
                t1 == t2 && s1 == s2 && co1 == co2 && b1 == b2 && m1 == m2
            }
            (UiNode::Interactive { node_id: n1, child: c1, on_click: oc1, on_hover: oh1 },
             UiNode::Interactive { node_id: n2, child: c2, on_click: oc2, on_hover: oh2 }) => {
                n1 == n2 && c1 == c2 && oc1 == oc2 && oh1 == oh2
            }
            // Raw: compare via serde JSON round-trip to avoid cascading PartialEq.
            (UiNode::Raw { command: cmd1 }, UiNode::Raw { command: cmd2 }) => {
                match (serde_json::to_string(cmd1.as_ref()), serde_json::to_string(cmd2.as_ref())) {
                    (Ok(s1), Ok(s2)) => s1 == s2,
                    _ => false,
                }
            }
            (UiNode::Surface { id: i1 }, UiNode::Surface { id: i2 }) => i1 == i2,
            (UiNode::Button { node_id: n1, label: l1, disabled: d1 },
             UiNode::Button { node_id: n2, label: l2, disabled: d2 }) => {
                n1 == n2 && l1 == l2 && d1 == d2
            }
            (UiNode::Input { node_id: n1, placeholder: ph1, value: v1 },
             UiNode::Input { node_id: n2, placeholder: ph2, value: v2 }) => {
                n1 == n2 && ph1 == ph2 && v1 == v2
            }
            (UiNode::TextEdit { node_id: n1, placeholder: ph1, value: v1, multiline: m1, max_length: ml1 },
             UiNode::TextEdit { node_id: n2, placeholder: ph2, value: v2, multiline: m2, max_length: ml2 }) => {
                n1 == n2 && ph1 == ph2 && v1 == v2 && m1 == m2 && ml1 == ml2
            }
            (UiNode::Badge { label: l1, fill: f1, fg: fg1 },
             UiNode::Badge { label: l2, fill: f2, fg: fg2 }) => {
                l1 == l2 && f1 == f2 && fg1 == fg2
            }
            (UiNode::Dot { color: c1, size: s1 },
             UiNode::Dot { color: c2, size: s2 }) => {
                c1 == c2 && s1 == s2
            }
            (UiNode::AppBar { title: t1, subtitle: s1 },
             UiNode::AppBar { title: t2, subtitle: s2 }) => t1 == t2 && s1 == s2,
            (UiNode::FooterKeys { entries: e1, divider: d1 },
             UiNode::FooterKeys { entries: e2, divider: d2 }) => e1 == e2 && d1 == d2,
            (UiNode::Footer { text: t1, color: c1 },
             UiNode::Footer { text: t2, color: c2 }) => t1 == t2 && c1 == c2,
            (UiNode::Section { title: t1 }, UiNode::Section { title: t2 }) => t1 == t2,
            (UiNode::Label { text: t1, size: s1, color: c1, tone: to1, bold: b1, monospace: m1, max_lines: ml1 },
             UiNode::Label { text: t2, size: s2, color: c2, tone: to2, bold: b2, monospace: m2, max_lines: ml2 }) => {
                t1 == t2 && s1 == s2 && c1 == c2 && to1 == to2 && b1 == b2 && m1 == m2 && ml1 == ml2
            }
            (UiNode::Spacer { size: s1, grow: g1 },
             UiNode::Spacer { size: s2, grow: g2 }) => s1 == s2 && g1 == g2,
            (UiNode::Divider { color: c1 }, UiNode::Divider { color: c2 }) => c1 == c2,
            (UiNode::Card { children: c1, padding: p1 },
             UiNode::Card { children: c2, padding: p2 }) => c1 == c2 && p1 == p2,
            (UiNode::SelectList { items: i1, selected_idx: s1 },
             UiNode::SelectList { items: i2, selected_idx: s2 }) => i1 == i2 && s1 == s2,
            (UiNode::Pinned { edge: e1, child: c1 }, UiNode::Pinned { edge: e2, child: c2 }) => {
                e1 == e2 && c1 == c2
            }
            (UiNode::Column { children: c1, gap: g1, padding_top: p1, padding: pa1 },
             UiNode::Column { children: c2, gap: g2, padding_top: p2, padding: pa2 }) => {
                c1 == c2 && g1 == g2 && p1 == p2 && pa1 == pa2
            }
            _ => false,
        }
    }
}

