use crate::protocol::commands::RenderCommand;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// ── Component tree wire types (PGAP v3.5) ────────────────────────────────────

fn default_true() -> bool {
    true
}

fn default_space_xl() -> f32 {
    24.0 // SPACE_XL — keep in sync with src/ui/style.rs
}

fn default_canvas_width() -> f32 {
    640.0
}

fn default_canvas_height() -> f32 {
    360.0
}

fn default_markdown_base_size() -> f32 {
    14.0
}

fn default_markdown_padding() -> f32 {
    12.0
}

fn default_slider_max() -> f32 {
    1.0
}

fn default_datetime_mode() -> String {
    "datetime".to_string()
}

fn default_skeleton_rows() -> usize {
    3
}

/// One row in a `UiNode::KeyValue` description list.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
pub struct KeyValueRow {
    pub key: String,
    pub value: String,
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
    /// Explicit-size layout wrapper. Constrains its single child to an exact
    /// `width` and/or `height`. A `null` axis means "inherit available size".
    /// Primary use: a fixed-width sibling (e.g. a sidebar) alongside a growing
    /// `Canvas` inside a horizontal `Stack`.
    Sized {
        #[serde(default)]
        width: Option<f32>,
        #[serde(default)]
        height: Option<f32>,
        child: Box<UiNode>,
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
    /// Host-rendered markdown block.
    Markdown {
        text: String,
        #[serde(default = "default_markdown_base_size")]
        base_size: f32,
        #[serde(default)]
        color: String,
        #[serde(default = "default_markdown_padding")]
        padding: f32,
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
    /// CPU canvas with local draw commands. The host allocates a layout rect
    /// for the node, then renders commands relative to that rect.
    Canvas {
        commands: Vec<RenderCommand>,
        #[serde(default = "default_canvas_width")]
        width: f32,
        #[serde(default = "default_canvas_height")]
        height: f32,
        #[serde(default = "default_true")]
        grow: bool,
    },
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
        #[serde(default)]
        style: String,
    },
    /// Host-owned contextual action strip. Children should be `Button` nodes;
    /// non-button children are ignored by the renderer.
    ActionBar {
        #[serde(alias = "children")]
        actions: Vec<UiNode>,
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
        #[serde(alias = "text")]
        label: String,
        #[serde(default, alias = "color")]
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
    Section { title: String },
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

    // ── L1 form controls (placeholder primitives — styling to be expanded) ─
    /// Boolean checkbox. Clicking fires `ComponentEvent { event_type: "change",
    /// payload: { value } }` with the toggled bool; the app owns the state.
    Checkbox {
        node_id: String,
        #[serde(default)]
        label: String,
        #[serde(default)]
        checked: bool,
        #[serde(default)]
        disabled: bool,
    },
    /// Single-select radio group. Clicking an option fires `ComponentEvent
    /// { event_type: "change", payload: { value: index } }`.
    Radio {
        node_id: String,
        options: Vec<String>,
        #[serde(default)]
        selected: usize,
        #[serde(default)]
        disabled: bool,
    },
    /// On/off switch. Fires `ComponentEvent { event_type: "change",
    /// payload: { value } }` with the toggled bool.
    Switch {
        node_id: String,
        #[serde(default)]
        label: String,
        #[serde(default)]
        on: bool,
        #[serde(default)]
        disabled: bool,
    },
    /// Horizontal value slider. Clicking along the track fires `ComponentEvent
    /// { event_type: "change", payload: { value } }`.
    Slider {
        node_id: String,
        #[serde(default)]
        value: f32,
        #[serde(default)]
        min: f32,
        #[serde(default = "default_slider_max")]
        max: f32,
        #[serde(default)]
        disabled: bool,
    },
    /// Dropdown/combobox trigger. Clicking fires a `click` `ComponentEvent`;
    /// the app owns the option popover.
    Select {
        node_id: String,
        options: Vec<String>,
        #[serde(default)]
        selected: usize,
        #[serde(default)]
        placeholder: String,
    },
    /// Date/time picker trigger. Clicking fires a `click` `ComponentEvent`;
    /// the app owns the picker popover.
    DateTimePicker {
        node_id: String,
        #[serde(default)]
        value: String,
        #[serde(default = "default_datetime_mode")]
        mode: String,
    },

    // ── L1 display / data primitives (styling to be expanded) ─────────────
    /// Progress bar. `value` is 0.0–1.0; set `indeterminate` for unknown work.
    Progress {
        #[serde(default)]
        value: f32,
        #[serde(default)]
        label: String,
        #[serde(default)]
        indeterminate: bool,
    },
    /// Loading spinner with an optional caption.
    Spinner {
        #[serde(default)]
        label: String,
    },
    /// Hover tooltip wrapping a single child.
    Tooltip {
        text: String,
        child: Box<UiNode>,
    },
    /// Circular avatar rendering initials (image variant deferred).
    Avatar {
        #[serde(default)]
        label: String,
        #[serde(default)]
        size: f32,
    },
    /// Named glyph icon. `name` is a semantic key; the placeholder renders the
    /// literal token until a real icon set is wired.
    Icon {
        name: String,
        #[serde(default)]
        size: f32,
        #[serde(default)]
        color: String,
    },
    /// Monospace code block with an optional language label.
    CodeBlock {
        code: String,
        #[serde(default)]
        language: String,
    },
    /// Static data table with column headers and string cells.
    Table {
        columns: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    /// Inline banner / callout with a semantic tone (info/success/warning/danger).
    Banner {
        text: String,
        #[serde(default)]
        tone: String,
        #[serde(default)]
        title: String,
    },
    /// Key/value description list.
    KeyValue {
        rows: Vec<KeyValueRow>,
    },
    /// Breadcrumb trail; the last item is the current location.
    Breadcrumb {
        items: Vec<String>,
    },
    /// Pagination control. Prev/next clicks fire `ComponentEvent
    /// { event_type: "change", payload: { value: page } }`.
    Pagination {
        node_id: String,
        #[serde(default)]
        page: usize,
        #[serde(default)]
        total: usize,
    },
    /// Disclosure/accordion. The header click fires a `click` `ComponentEvent`;
    /// the child renders when `open` (the app flips it).
    Accordion {
        node_id: String,
        title: String,
        #[serde(default)]
        open: bool,
        child: Box<UiNode>,
    },
    /// Tab strip (headers only). Selecting a tab fires `ComponentEvent
    /// { event_type: "change", payload: { value: index } }`; the app renders the body.
    Tabs {
        node_id: String,
        tabs: Vec<String>,
        #[serde(default)]
        active: usize,
    },
    /// Empty-state placeholder with a title, optional description and icon token.
    EmptyState {
        title: String,
        #[serde(default)]
        description: String,
        #[serde(default)]
        icon: String,
    },
    /// Skeleton loading placeholder — `rows` shimmer bars of the given height.
    Skeleton {
        #[serde(default = "default_skeleton_rows")]
        rows: usize,
        #[serde(default)]
        height: f32,
    },
    /// Modal dialog wrapping a child body. The close affordance fires a `click`
    /// `ComponentEvent`.
    Modal {
        node_id: String,
        title: String,
        child: Box<UiNode>,
    },
}

// Compared via serde round-trip. `UiNode::Raw` wraps `RenderCommand` (hundreds
// of transitive types without `PartialEq`), so we cannot derive; hand-writing a
// per-variant arm for every node was a maintenance landmine as the component
// vocabulary grows. JSON equality is exact for these value trees and keeps
// adding a new variant a single-location change. Used only by tests.
impl PartialEq for UiNode {
    fn eq(&self, other: &Self) -> bool {
        match (serde_json::to_string(self), serde_json::to_string(other)) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::UiNode;

    #[test]
    fn badge_accepts_python_sdk_field_aliases() {
        let json = r##"{"type":"badge","text":"Ready","color":"#89b4fa"}"##;

        let node: UiNode = serde_json::from_str(json).expect("SDK badge aliases should parse");

        assert_eq!(
            node,
            UiNode::Badge {
                label: "Ready".into(),
                fill: "#89b4fa".into(),
                fg: String::new(),
            }
        );
    }

    /// Every placeholder primitive's Python `to_node()` JSON must parse into the
    /// matching `UiNode` variant. Guards the SDK↔host wire contract (stint 0328).
    #[test]
    fn sdk_placeholder_primitives_parse_from_python_wire_json() {
        let cases: &[(&str, fn(&UiNode) -> bool)] = &[
            (
                r#"{"type":"checkbox","node_id":"c","label":"Agree","checked":true,"disabled":false}"#,
                |n| matches!(n, UiNode::Checkbox { checked: true, .. }),
            ),
            (
                r#"{"type":"radio","node_id":"r","options":["a","b"],"selected":1,"disabled":false}"#,
                |n| matches!(n, UiNode::Radio { selected: 1, .. }),
            ),
            (
                r#"{"type":"switch","node_id":"s","label":"","on":true,"disabled":false}"#,
                |n| matches!(n, UiNode::Switch { on: true, .. }),
            ),
            (
                r#"{"type":"slider","node_id":"sl","value":0.5,"min":0.0,"max":1.0,"disabled":false}"#,
                |n| matches!(n, UiNode::Slider { .. }),
            ),
            (
                r#"{"type":"select","node_id":"se","options":["x"],"selected":0,"placeholder":"Pick"}"#,
                |n| matches!(n, UiNode::Select { .. }),
            ),
            (
                r#"{"type":"date_time_picker","node_id":"d","value":"","mode":"date"}"#,
                |n| matches!(n, UiNode::DateTimePicker { .. }),
            ),
            (
                r#"{"type":"progress","value":0.3,"label":"Load","indeterminate":false}"#,
                |n| matches!(n, UiNode::Progress { .. }),
            ),
            (r#"{"type":"spinner","label":"Wait"}"#, |n| {
                matches!(n, UiNode::Spinner { .. })
            }),
            (
                r#"{"type":"tooltip","text":"Hi","child":{"type":"text","text":"x"}}"#,
                |n| matches!(n, UiNode::Tooltip { .. }),
            ),
            (r#"{"type":"avatar","label":"Ian Burke","size":0.0}"#, |n| {
                matches!(n, UiNode::Avatar { .. })
            }),
            (r#"{"type":"icon","name":"gear","size":0.0,"color":""}"#, |n| {
                matches!(n, UiNode::Icon { .. })
            }),
            (
                r#"{"type":"code_block","code":"x=1","language":"python"}"#,
                |n| matches!(n, UiNode::CodeBlock { .. }),
            ),
            (
                r#"{"type":"table","columns":["a"],"rows":[["1"]]}"#,
                |n| matches!(n, UiNode::Table { .. }),
            ),
            (
                r#"{"type":"banner","text":"Saved","tone":"success","title":""}"#,
                |n| matches!(n, UiNode::Banner { .. }),
            ),
            (
                r#"{"type":"key_value","rows":[{"key":"Env","value":"prod"}]}"#,
                |n| matches!(n, UiNode::KeyValue { .. }),
            ),
            (r#"{"type":"breadcrumb","items":["Home","Docs"]}"#, |n| {
                matches!(n, UiNode::Breadcrumb { .. })
            }),
            (
                r#"{"type":"pagination","node_id":"p","page":1,"total":5}"#,
                |n| matches!(n, UiNode::Pagination { .. }),
            ),
            (
                r#"{"type":"accordion","node_id":"a","title":"More","open":true,"child":{"type":"text","text":"x"}}"#,
                |n| matches!(n, UiNode::Accordion { open: true, .. }),
            ),
            (
                r#"{"type":"tabs","node_id":"t","tabs":["A","B"],"active":0}"#,
                |n| matches!(n, UiNode::Tabs { .. }),
            ),
            (
                r#"{"type":"empty_state","title":"Nothing","description":"","icon":""}"#,
                |n| matches!(n, UiNode::EmptyState { .. }),
            ),
            (r#"{"type":"skeleton","rows":3,"height":0.0}"#, |n| {
                matches!(n, UiNode::Skeleton { .. })
            }),
            (
                r#"{"type":"modal","node_id":"m","title":"Confirm","child":{"type":"text","text":"x"}}"#,
                |n| matches!(n, UiNode::Modal { .. }),
            ),
        ];

        for (json, check) in cases {
            let node: UiNode =
                serde_json::from_str(json).unwrap_or_else(|e| panic!("parse {json}: {e}"));
            assert!(check(&node), "wrong variant for {json}");
        }
        assert_eq!(cases.len(), 22, "all 22 placeholder primitives covered");
    }
}
