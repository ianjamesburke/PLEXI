use super::primitives::*;
use super::ui_nodes::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A child inside a layout tree — either a nested layout node or a leaf draw command.
///
/// Leaf commands must be host-measured primitives (`Badge`, `Text`, `KeyChip`).
/// The `x` and `y` fields on leaf commands are ignored — positions come from taffy.
///
/// `JsonSchema` is implemented manually to avoid schemars recursion issues
/// (RenderCommand → LayoutChild → RenderCommand).
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LayoutChild {
    /// A nested layout node.
    Node {
        direction: LayoutDirection,
        children: Vec<LayoutChild>,
        #[serde(default)]
        gap: f32,
    },
    /// A leaf draw command positioned by taffy.
    Leaf { command: Box<RenderCommand> },
}

// Manual JsonSchema impl to avoid schemars recursion (RenderCommand ↔ LayoutChild).
impl schemars::JsonSchema for LayoutChild {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "LayoutChild".into()
    }
    fn json_schema(_gen: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({})
    }
}

// ── Commands sent FROM the app TO Plexi ──────────────────────────────────────

/// Render primitives — go to `pending_frame` → drawn to screen.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RenderCommand {
    /// Push a clip rect onto the host's clip stack.
    ///
    /// The effective clip rect is the intersection of the new rect with the
    /// current top of the stack (or the pane rect if the stack is empty).
    /// All subsequent draw commands are clipped to this intersection until
    /// a matching `PopClip` rebalances the stack.
    ///
    /// Imbalanced push/pop is a hard error logged at `warn` level. The host
    /// resets to zero depth at frame end so a bug in one app cannot corrupt
    /// subsequent frames.
    PushClip { x: f32, y: f32, w: f32, h: f32 },

    /// Pop the most recently pushed clip rect from the stack.
    ///
    /// If the stack is already empty, logs a `warn` and is a no-op.
    PopClip,

    /// Fill a rectangle.
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        fill: String,
        #[serde(default)]
        radius: f32,
    },
    /// Draw text at a position.
    ///
    /// `align` controls how the `(x, y)` point maps to the text box:
    ///   - `"top_left"` (default) — `(x, y)` is the top-left corner.
    ///   - `"center"`              — `(x, y)` is the visual center of the text.
    ///
    /// Centering uses the host's real font metrics, which matters for small
    /// badges / buttons where a 0.1em difference is visible. Prefer `center`
    /// for anything inside a fixed-size container.
    ///
    /// `max_width` — when `Some(w)`, the text is clipped at `w` pixels.
    /// `elide` — when `true`, a `…` is appended at the clip point; when
    ///           `false`, the text is hard-clipped with no marker.
    /// `selectable` — when `true`, the host renders this text as a
    ///                selectable egui label so the user can drag-select
    ///                inside it and Cmd+C copies the selection. When
    ///                `false` (default for pre-#200 callers), the text
    ///                paints via the painter and cannot be selected.
    ///                Required field — no `serde(default)`. Apps must
    ///                set it explicitly; omitting it fails deserialisation.
    Text {
        x: f32,
        y: f32,
        text: String,
        size: f32,
        color: String,
        #[serde(default)]
        monospace: bool,
        #[serde(default)]
        bold: bool,
        #[serde(default = "default_text_align")]
        align: String,
        max_width: Option<f32>,
        elide: bool,
        selectable: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_lines: Option<u32>,
    },
    /// Draw a line segment.
    Line {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        color: String,
        #[serde(default = "default_stroke_width")]
        width: f32,
    },
    /// Draw a filled circle. Alpha is supported via 8-digit hex fill (#rrggbbaa).
    Circle {
        cx: f32,
        cy: f32,
        r: f32,
        fill: String,
    },

    /// Draw a filled arc / pie slice.
    /// `start_angle` and `end_angle` are in radians, measured clockwise from the right (east).
    /// A full circle is 0.0 to std::f32::consts::TAU.
    Arc {
        cx: f32,
        cy: f32,
        r: f32,
        start_angle: f32,
        end_angle: f32,
        fill: String,
    },

    /// High-level scrollable list — host handles layout and scrolling.
    List {
        #[serde(default)]
        x: f32,
        #[serde(default)]
        y: f32,
        #[serde(default)]
        w: f32,
        #[serde(default)]
        h: f32,
        items: Vec<ListItem>,
        selected: usize,
        #[serde(default)]
        item_height: f32,
    },

    /// Host-native scrollable list with j/k navigation, typed row slots,
    /// and built-in loading/error/empty states.
    /// Host emits `PlexiEvent::ListSelect` / `PlexiEvent::ListActivate` for callbacks.
    ListView {
        id: String,
        #[serde(default)]
        x: f32,
        #[serde(default)]
        y: f32,
        /// 0.0 = full pane width
        #[serde(default)]
        w: f32,
        /// 0.0 = remaining pane height below y
        #[serde(default)]
        h: f32,
        items: Vec<ListViewItem>,
        #[serde(default)]
        selected: usize,
        #[serde(default)]
        loading: bool,
        #[serde(default)]
        error: Option<String>,
    },

    // ── Host-measured layout primitives ──────────────────────────────────
    //
    // These commands delegate text measurement and pill geometry entirely to
    // the host. The SDK emits intent; the host measures with real egui font
    // metrics and renders. No Python-side width estimation.
    /// Host-rendered pill badge. The host measures the label with real font
    /// metrics, sizes the pill (text_w + padding), and centres the text
    /// both horizontally and vertically. No width math in the SDK.
    ///
    /// `x`      — left edge of the badge.
    /// `y`      — vertical centre (the badge is drawn centred on this y).
    /// `label`  — text to display inside the pill.
    /// `fill`   — pill background colour (hex).
    /// `fg`     — text colour (hex).
    /// `font_size` — label font size in pt.
    /// `radius` — pill corner radius. Use a large value (e.g. font_size) for
    ///            a fully-rounded pill, or RADIUS_SM (4.0) for tag chips.
    Badge {
        x: f32,
        y: f32,
        label: String,
        fill: String,
        fg: String,
        font_size: f32,
        radius: f32,
    },

    /// Host-rendered keycap chip. The host measures the label with real
    /// monospace font metrics, sizes the chip, and centres the text inside.
    ///
    /// `x`         — left edge of the chip (top-left, matching ctx.text).
    /// `y`         — top edge of the chip.
    /// `label`     — key label (e.g. "⌘", "[", "Enter").
    /// `font_size` — label font size in pt.
    KeyChip {
        x: f32,
        y: f32,
        label: String,
        font_size: f32,
    },

    /// A horizontal row of keycap chips. The host flows them left-to-right
    /// with a fixed 2px gap between chips, sizes each chip from real font
    /// metrics, and places an optional trailing description label after the
    /// last chip.
    ///
    /// `x`, `y`      — top-left origin of the row.
    /// `keys`        — ordered list of key labels to render as chips.
    /// `description` — optional label rendered after the last chip.
    /// `font_size`   — applies to all chips and the description.
    KeyChipRow {
        x: f32,
        y: f32,
        keys: Vec<String>,
        description: Option<String>,
        font_size: f32,
    },

    /// A multi-group shortcut row. The host owns all layout — chip widths
    /// from real font metrics, flow horizontally with configurable inter-
    /// group gap, wrap to a new line when the next group would exceed
    /// `max_width`. Returns nothing; correct by construction.
    ///
    /// `x`, `y`      — top-left origin.
    /// `max_width`   — wrap budget. Wrap to a new line when the next group
    ///                 would exceed it. Caller passes the available pane
    ///                 width minus its own padding.
    /// `pairs`       — ordered list of `(chip-labels, description)` groups.
    ///                 Each pair renders as one or more chips followed by
    ///                 the description text.
    /// `font_size`   — applies to all chips and descriptions.
    Shortcuts {
        x: f32,
        y: f32,
        max_width: f32,
        pairs: Vec<ShortcutPair>,
        font_size: f32,
    },

    /// Multiple text segments rendered horizontally with host-measured layout.
    /// The host measures each segment with real font metrics and flows them
    /// left-to-right with configurable gap spacing.
    ///
    /// `x`, `y`  — origin of the text row.
    /// `items`   — list of text segments, each with text, color, size, monospace.
    /// `gap`     — spacing between items in pixels.
    /// `align`   — vertical alignment (e.g., "left_center").
    TextRow {
        x: f32,
        y: f32,
        items: Vec<TextRowItem>,
        gap: f32,
        align: String,
    },

    /// Render markdown text using the host's `egui_commonmark` renderer.
    ///
    /// The host creates a child `Ui` at `(x, y)` with width `w` and renders
    /// the markdown with proper formatting (bold, italic, code blocks, etc.).
    /// `base_size` controls the body font size; `color` sets the default text
    /// colour.
    Markdown {
        x: f32,
        y: f32,
        w: f32,
        text: String,
        base_size: f32,
        color: String,
    },

    /// Draw an image from a workspace-scoped path or data URL.
    Image {
        src: String,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        /// One of: "contain" | "cover" | "fill". Default: "contain".
        #[serde(default = "default_image_fit")]
        fit: String,
    },

    /// Circular clipped image. `src` accepts a load_image handle (UUID) or local path.
    /// `cx`, `cy` are the circle centre in pane-local coordinates.
    /// `radius` is the circle radius in logical pixels.
    Avatar {
        src: String,
        cx: f32,
        cy: f32,
        radius: f32,
    },

    /// Animated shimmer placeholder rect for loading states.
    /// The host drives a subtle pulsing animation at ~20fps using request_repaint_after.
    Skeleton {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        #[serde(default = "default_skeleton_radius")]
        radius: f32,
    },

    /// Render an amplitude meter reading from a binary pipe.
    AudioMeter { rect: Rect, pipe_id: String },

    /// Text input field (host-owned buffer, submit-only).
    ///
    /// Emitted by the app each frame at `(x, y)` with width `w`. The host
    /// owns the underlying buffer keyed on `id` — typed characters never
    /// reach the app between frames. On Enter the host emits
    /// `PlexiEvent::TextSubmitted { id, value }` and clears its buffer.
    ///
    /// When `multiline` is `false` (the default), the host renders a
    /// single-line `TextEdit` and Enter submits. When `multiline` is `true`,
    /// the host renders a multi-line `TextEdit`; Enter still submits but
    /// Shift+Enter inserts a newline.
    ///
    /// Real-time validation (per-keystroke value access) is intentionally
    /// out of scope — see issue #283 option A. Apps that need it must
    /// wait for a future protocol revision.
    TextInput {
        id: String,
        x: f32,
        y: f32,
        w: f32,
        /// Height of the input widget in pixels. Defaults to 24.0 for
        /// backwards compatibility with older SDKs that don't send `h`.
        #[serde(default = "default_text_input_h")]
        h: f32,
        placeholder: String,
        /// When `true`, render as a multi-line editor. Enter submits;
        /// Shift+Enter inserts a newline. Defaults to `false` so existing
        /// draw commands without this field continue to work.
        #[serde(default)]
        multiline: bool,
    },

    // ── Host-managed scroll regions (#446) ───────────────────────────────
    /// Begin a host-managed vertical scroll region.
    ///
    /// All draw commands between this and the matching `EndScroll` are clipped
    /// to the viewport rect `(x, y, w, h)`. The host tracks the scroll offset
    /// for `id` across frames and emits `PlexiEvent::ScrollOffset { id, offset_y }`
    /// whenever the user scrolls (mouse wheel / drag). The app translates its
    /// content coordinates by `offset_y` before emitting draw commands.
    ///
    /// `content_height` is the total virtual height of the scrollable content
    /// in logical pixels. The host uses this to size the scrollbar thumb.
    ///
    /// `id` must be stable across frames — use a meaningful string (e.g. the
    /// region name) rather than a counter that may change.
    BeginScroll {
        id: String,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        content_height: f32,
    },

    /// Close the most recently opened scroll region.
    ///
    /// Must be balanced with a preceding `BeginScroll`. Imbalanced pairs are
    /// logged at `warn` level and the stack is reset at frame end.
    EndScroll,

    /// Declarative flex layout tree. The host resolves all positions using
    /// taffy (flexbox) and real egui font metrics before painting.
    ///
    /// `x`, `y` — pane-relative top-left anchor of the layout tree.
    /// `direction` — flex direction of the root node.
    /// `children` — the layout tree children.
    /// `gap` — space between children in the root node (pixels).
    Layout {
        x: f32,
        y: f32,
        direction: LayoutDirection,
        children: Vec<LayoutChild>,
        #[serde(default)]
        gap: f32,
    },

    /// Responsive layout — host picks the matching tier based on available rect aspect ratio.
    ///
    /// Tiers are evaluated in order. First match wins. The aspect condition is one of:
    /// - `"landscape"` — width > height
    /// - `"portrait"` — height > width
    /// - `"square"` — within ~20% ratio (0.83..1.2), or used as a fallback
    ///
    /// The winning tier's layout tree is rendered through the existing taffy pipeline.
    Responsive {
        x: f32,
        y: f32,
        tiers: Vec<ResponsiveTier>,
    },

    /// Component tree — host renders via render_component_tree(). Added in PGAP v3.5.
    /// Host logs a warning and drops if it doesn't know this variant.
    ComponentTree { root: UiNode },
}

/// A single tier in a responsive layout. Contains an aspect-ratio condition
/// and the layout tree to render when that condition matches.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct ResponsiveTier {
    /// One of: "landscape", "portrait", "square"
    pub aspect: String,
    /// Flex direction for this tier's root node.
    pub direction: LayoutDirection,
    /// Children of this tier's layout tree.
    pub children: Vec<LayoutChild>,
    /// Gap between children in pixels.
    #[serde(default)]
    pub gap: f32,
}

/// Agent state reported by a hook script.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    Working,
    Blocked,
    Idle,
}

/// Per-pane agent state stored in PlexiApp.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PaneAgentState {
    pub pane_id: u64,
    pub state: AgentState,
    pub agent: String,
    pub session_id: Option<String>,
}

/// App-to-host requests — go to `route_command`.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AppRequest {
    /// Request a runtime capability prompt. Host shows modal; responds with CapabilityDecision.
    CapabilityRequest {
        request_id: String,
        /// v3 capability string, e.g. "net.http"
        capability: String,
    },
    /// Request a workspace-scoped secret. Scoped to Init.workspace_root automatically.
    SecretGet { key: String },
    /// Save app state. Host writes to workspace or global JSON file.
    SaveAppState { payload: serde_json::Value },
    /// Request to start a run. Host surfaces in Run palette (Cmd+R).
    RunGet {
        intent: String,
        payload: serde_json::Value,
    },
    /// Signal that a run the app owns has finished.
    RunComplete {
        run_id: String,
        result: serde_json::Value,
    },
    /// Post a notification. All three action_types must dispatch correctly (no TODO).
    Notify {
        /// One of: "info" | "warn" | "error"
        level: String,
        title: String,
        body: String,
        /// The notification shape. Defaults to `message` for back-compat with
        /// existing apps. Determines how the modal renders and interacts.
        #[serde(default)]
        kind: NotifyKind,
        /// Choice options (only meaningful for `kind = "choice"`). The host shows
        /// these as keyboard-navigable buttons; Enter selects the focused one.
        #[serde(default)]
        options: Vec<NotifyOption>,
        /// Placeholder / hint for the text input (only for `kind = "input"`).
        #[serde(default)]
        input_prompt: Option<String>,
        /// If true, the user cannot dismiss with Esc — they must pick an option
        /// or submit input. Intended for decisions the app depends on.
        #[serde(default)]
        required: bool,
        #[serde(default)]
        actions: Vec<NotificationAction>,
        /// If set, host sends PlexiEvent::NotifyAction when the user responds.
        #[serde(default)]
        notify_id: Option<String>,
        /// Higher = more urgent. REQUIRED — no `#[serde(default)]`. Apps must
        /// set this explicitly; omitting it fails deserialisation (forces SDK
        /// upgrade, no silent defaults). Ties broken by arrival order.
        /// Typical values: 0 (background info), 50 (normal), 100 (important),
        /// 200 (critical/required).
        priority: u32,
        /// Inline image attachment (PNG / JPEG, base64-encoded). Rendered above
        /// the action buttons. Decoded size > 50 KB triggers a placeholder.
        /// `Option` is the natural empty/missing wire shape — no
        /// `#[serde(default)]` shim. Mutually exclusive with `image_pipe_id`;
        /// when both set, inline wins and the pipe is ignored (logged warn).
        #[serde(skip_serializing_if = "Option::is_none")]
        image_inline: Option<NotificationImage>,
        /// Pipe-referenced image. Host drains the binary ring lazily when the
        /// notification is visible. Payload format: `width: u32 LE`,
        /// `height: u32 LE`, then `width * height * 4` bytes of RGBA.
        #[serde(skip_serializing_if = "Option::is_none")]
        image_pipe_id: Option<String>,
        /// Auto-dismiss after this many seconds. On expiry, host delivers
        /// `PlexiEvent::NotifyAction` with `value = on_dismiss` (or "timeout" if
        /// `on_dismiss` is None). Omit for notifications that should persist.
        #[serde(default)]
        timeout_secs: Option<u64>,
        /// Payload delivered to the app when the notification is dismissed without
        /// an explicit choice (timeout, Esc, or tombstone dismiss). Defaults to
        /// "timeout" when omitted.
        #[serde(default)]
        on_dismiss: Option<String>,
        /// CLI-only: path to a file the host writes the chosen key into when
        /// the user responds. Used by `plexi notify --choice` to return the
        /// result to the blocking CLI process. Never set by app SDK.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        response_file: Option<String>,
        /// CLI-only: notification visibility scope. Apps never set this — their
        /// scope comes from `manifest.toml::[launch] notification_scope`.
        /// Omit for backwards-compatible global behaviour.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scope: Option<NotifyScope>,
    },
    /// Report agent state for a pane. Called by hook scripts via `plexi agent report`.
    SetAgentState {
        pane_id: u64,
        state: AgentState,
        agent: String,
        #[serde(default)]
        session_id: Option<String>,
    },
    /// Get all tracked pane agent states. Writes JSON array to response_file.
    GetAgentStates {
        response_file: String,
    },
    /// Open a typed pipe.
    /// mode: "json" | "binary"
    /// direction: "in" | "out" | "duplex"
    PipeOpen {
        pipe_id: String,
        /// One of: "json" | "binary"
        mode: String,
        /// One of: "in" | "out" | "duplex"
        direction: String,
    },
    /// Open a *directed* JSON pipe to a specific target pane (#286).
    ///
    /// Mirrors `PipeOpen` but the host scopes `PipeMessage` delivery so only
    /// the caller pane and the pane identified by `target_pane_id` can
    /// receive messages on this pipe — peer apps are NOT subscribed even
    /// if they call `pipe_open` with the same `pipe_id`.
    ///
    /// Created bidirectionally: both panes receive the other's `PipeSend`
    /// payloads as `PlexiEvent::PipeMessage`. Either side can close.
    ///
    /// Capability gate: caller needs `pipe.open` (the same as `PipeOpen`).
    /// The target does NOT need `agents.list` — it just needs to be
    /// addressable. The target also doesn't need to opt in; the host
    /// subscribes it on this pipe id.
    ///
    /// Direction is always `duplex` on directed pipes — there's no use case
    /// for a one-way agent-to-agent channel today, so the field is omitted.
    /// Mode is always `json`; binary directed pipes are out of scope until
    /// a real use case arrives.
    PipeOpenDirected {
        pipe_id: String,
        target_pane_id: u64,
    },
    /// Send a JSON-mode pipe message (not for binary pipes).
    PipeSend {
        pipe_id: String,
        payload: serde_json::Value,
    },
    /// Update the status text shown in the parent pane chrome.
    StatusSummary { text: String },

    /// Request the host to spawn a new app pane. Requires `spawn.app` capability.
    /// `layout`: "split_h" (new pane right), "split_v" (default, new pane below),
    ///           or "overlay" (full pane, no split).
    /// `args`: argv passed to the child process (appended after the binary path).
    /// Host responds with `PlexiEvent::AppSpawned { pane_id }` on success.
    SpawnApp {
        type_id: String,
        #[serde(default)]
        layout: Option<String>,
        #[serde(default)]
        args: Vec<String>,
    },

    /// Unified pane spawn primitive (#592). Supersedes SpawnApp for new apps.
    /// Requires `panes.spawn` capability.
    /// `layout`: one of "split_h", "split_right", "split_v", "split_below", "split_above", "split_left", "overlay",
    ///   "new_window" (terminal only — creates a new spatial grid window to the right
    ///   of the current context row instead of splitting the active pane),
    ///   "tab" (terminal only — adds a new tab alongside the focused pane, wrapping
    ///   both in a Tabs container if needed; use after `pane focus` to target a window).
    ///   "overlay_pane" and "background" are reserved but not yet implemented.
    /// `pipe_id`: when set, the host appends `--pipe=<pipe_id>` to args before launch
    ///   so the spawned app can reply via PipeSend on completion.
    /// Host responds: `PlexiEvent::PaneSpawned { pane_id }` on success,
    ///               `PlexiEvent::PaneSpawnError { reason }` on failure.
    SpawnPane {
        type_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        layout: Option<String>,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pipe_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from_pane_id: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        response_file: Option<String>,
        #[serde(default, skip_serializing_if = "is_false")]
        ephemeral: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
        #[serde(default, skip_serializing_if = "is_false")]
        no_focus: bool,
        /// When set, the host launches the app directly from this filesystem path
        /// rather than looking it up in the registry by type_id.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        /// When set, use this path as the workspace root for app state scoping
        /// instead of defaulting to the app directory. Sent by `plexi open github:`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspace_root: Option<String>,
        /// When set, spawn the pane into this child context instead of the
        /// requesting app's own context. The target context must exist and be
        /// a descendant of the requesting app's context (#1518).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target_context: Option<u64>,
        /// Inline pane name — applied immediately after spawn so the pane
        /// starts with a human-readable label instead of the default shell title.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },

    /// Set the title displayed on a terminal pane's tab. Sent by `plexi pane set-title`
    /// over PLEXI_SOCKET.
    SetPaneTitle { pane_id: u64, name: String },

    /// List all open panes. Host writes a JSON array to `response_file`. Sent by `plexi pane list`.
    ListPanes {
        response_file: String,
        /// When Some, only panes in this context are returned.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        context_id: Option<u64>,
    },

    /// List all open contexts. Host writes a JSON array to `response_file`.
    /// Sent by `plexi context list`.
    ListContexts { response_file: String },

    /// Query info for a specific pane by ID. Host writes JSON object to `response_file`.
    /// Sent by `plexi pane info`.
    GetPaneInfo { pane_id: u64, response_file: String },

    /// Query info for the previously focused pane. Host walks `pane_focus_history`
    /// from the end, finds the last entry whose tile resolves to a live pane, and
    /// writes the same JSON shape as `GetPaneInfo` to `response_file`.
    /// Returns `{"error":"..."}` if history is empty or no valid pane found.
    /// Sent by `plexi pane info --previous`.
    GetPreviousPaneInfo { response_file: String },

    /// Move UI focus to a pane by PaneId. Sent by `plexi pane focus`. Fire-and-forget.
    FocusPane { pane_id: u64 },

    /// Close a pane by PaneId. Sent by `plexi pane close`. Fire-and-forget.
    ClosePane { pane_id: u64 },

    /// Write text to a running pane's PTY stdin. Sent by `plexi pane send`.
    /// `\n` in text (literal backslash-n) is interpreted as Enter.
    /// Host writes `{"ok":true}` or `{"error":"..."}` to `response_file` when set.
    SendToPane {
        pane_id: u64,
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        response_file: Option<String>,
    },

    /// Deliver a synthetic key event to any pane. Sent by `plexi pane key`.
    /// For terminal panes, the key is translated to PTY bytes.
    /// For PGAP app panes, a `PlexiEvent::Key` is delivered.
    /// Host writes `{"ok":true}` or `{"error":"..."}` to `response_file` when set.
    KeyPane {
        pane_id: u64,
        /// Key string: single char ("h"), named key ("enter", "escape", "up", "down",
        /// "left", "right", "space", "backspace"), or chord ("ctrl+c", "ctrl+d").
        key: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        response_file: Option<String>,
    },

    /// Read the last N lines from a terminal pane's PTY scrollback buffer.
    /// Sent by `plexi pane capture`. Host writes a JSON array of strings to `response_file`.
    CapturePane {
        pane_id: u64,
        /// Number of lines to capture from the end of the scrollback.
        lines: usize,
        response_file: String,
        /// When true, preserve trailing empty lines. Defaults to false (strip them).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        full_output: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from_cursor: Option<u64>,
    },

    /// Query the last-rendered UI state of a pane. Sent by `plexi pane state`.
    /// For app panes: host writes a JSON object with a `frame` array of RenderCommands.
    /// For terminal panes: host writes a simple status object.
    /// Host writes `{"error":"..."}` if the pane is not found.
    GetPaneState { pane_id: u64, response_file: String },

    /// Dispatch a semantic action to an app pane. Sent by `plexi app action <pane_id> <action> [args...]`.
    /// Host delivers `PlexiEvent::Action { action, args }` to the target app pane.
    /// Writes `{"ok":true}` or `{"error":"..."}` to `response_file`.
    SendAppAction {
        pane_id: u64,
        /// Action name, e.g. "refresh", "navigate-to".
        action: String,
        /// Optional extra arguments forwarded from the CLI.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        args: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        response_file: Option<String>,
    },

    /// Create a new context. Sent by `plexi context new` over PLEXI_SOCKET.
    CreateContext {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        root: Option<std::path::PathBuf>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_name: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        windows: Vec<String>,
    },
    /// Focus existing context by root, or create one. Sent by `plexi context open`.
    FocusContext { root: std::path::PathBuf },
    /// Set/update the root of the active context. Sent by `plexi context set-root`.
    SetContextRoot { root: std::path::PathBuf },
    /// Set/update the description of the active context. Sent by `plexi context describe`.
    SetContextDescription { description: String },
    /// Zoom into a sub-context. Pushes depth stack. Sent by `plexi context zoom`.
    ZoomIntoContext { context_id: u64 },
    /// Zoom out of a sub-context. Pops depth stack. Sent by `plexi context zoom-out`.
    ZoomOutOfContext,
    /// Push the focused pane into a new sub-context. Sent by `plexi context push`.
    PushPaneToSubcontext {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },

    /// Query the rolled-up `ContextState` for a context (#1518).
    /// The requesting app must be in an ancestor (or the same) context.
    /// Host responds with `PlexiEvent::ContextStateResponse`.
    QueryContextState { context_id: u64 },

    // ── Media + HTTP primitives ──────────────────────────────────────────
    /// Host-brokered HTTP request. Requires `net.http` capability.
    /// Host replies with `PlexiEvent::HttpResponse { request_id, ... }`.
    HttpRequest {
        request_id: String,
        url: String,
        #[serde(default = "default_http_method")]
        method: String,
        #[serde(default)]
        headers: std::collections::HashMap<String, String>,
        #[serde(default)]
        body: Option<String>,
    },
    /// v3.3 brokered AI call. Requires `ai.query` capability.
    ///
    /// The host routes this to the active Plexi AI backend, appends an
    /// `AgentTurn` row to `ai-ledger.jsonl`, and replies with
    /// `PlexiEvent::AiResponse { request_id, content, tokens_in, tokens_out, error }`.
    ///
    /// All fields are required — no `serde(default)`. `tools` may be empty;
    /// non-empty `tools` causes the broker to dispatch through a tool loop
    /// (#399) — the AI can call tools on any pane that exposed them via
    /// `DrawCommand::ExposeTools`.
    AiQuery {
        request_id: String,
        model_tier: ModelTier,
        system: String,
        messages: Vec<AiMessage>,
        tools: Vec<AiTool>,
    },
    /// v3.7 tool protocol (#398). App declares its callable tools to the host.
    /// The host registers these in the global tool registry so the broker can
    /// route `PlexiEvent::ToolCall` events to this pane during AI turns that
    /// include tool-use.
    ///
    /// May be sent at any time (including on_init). Replaces any prior
    /// registration for this pane — send the full current set each time.
    ExposeTools { tools: Vec<AiTool> },
    /// v3.7 tool protocol (#399). App returns the result of a `PlexiEvent::ToolCall`
    /// invocation. `call_id` must match the `call_id` from the `ToolCall` event.
    ///
    /// Either `output_json` or `error` must be set — not both.
    ToolResult {
        call_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output_json: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    /// App returns the result of a `PlexiEvent::McpToolCall` invocation.
    /// `call_id` must match the `call_id` from the `McpToolCall` event.
    /// Either `result` or `error` must be set.
    McpToolResult {
        call_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result: Option<serde_json::Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    /// Host-owned audio playback via `rodio`.
    AudioPlay {
        #[serde(default)]
        source: Option<String>,
        #[serde(default)]
        pipe_id: Option<String>,
        #[serde(default = "default_volume")]
        volume: f32,
        /// One of: "playing" | "paused" | "stopped".
        state: String,
    },
    /// Host-owned audio capture: mic PCM delivered on a binary pipe.
    /// `device_id` selects which input device (from `ListAudioDevices`); when
    /// `None` the host opens the OS default input. The negotiated parameters
    /// arrive in `PlexiEvent::AudioCaptureStarted`; failures arrive in
    /// `PlexiEvent::AudioCaptureError`.
    AudioCapture {
        pipe_id: String,
        #[serde(default)]
        device_id: Option<String>,
        sample_rate: u32,
        buffer_size: u32,
    },
    /// Request enumeration of audio devices (#277). Host responds with
    /// `PlexiEvent::AudioDevicesListed { request_id, inputs, outputs }`. No
    /// capability gate — enumeration discloses only device names already
    /// visible to any macOS app.
    ListAudioDevices { request_id: String },

    /// Request enumeration of MIDI ports (#320). Host responds with
    /// `PlexiEvent::MidiDevicesListed { request_id, inputs, outputs }`. No
    /// capability gate — enumeration discloses only port names already
    /// visible in Audio MIDI Setup.
    ListMidiDevices { request_id: String },
    /// Open a MIDI input port and forward every incoming message as a binary
    /// pipe frame on `pipe_id`. Each frame is a single MIDI 1.0 byte stream
    /// (1–3 bytes for channel-voice / system real-time). Requires `midi.in`.
    /// Host emits `PlexiEvent::PipeOpened` then `PlexiEvent::MidiInputOpened`
    /// on success, or `PlexiEvent::MidiInputError` on failure (port not found,
    /// capability denied, CoreMIDI error).
    OpenMidiInput { port_id: String, pipe_id: String },

    /// Close the MIDI input previously opened on `port_id`. The host
    /// disconnects from the port and closes the associated binary pipe.
    /// No-op if the port is not currently open. No response event.
    CloseMidiInput { port_id: String },

    /// Send one MIDI 1.0 byte stream to `port_id`. Fire-and-forget — the host
    /// only emits `PlexiEvent::MidiSendError` if the send failed (port not
    /// open, CoreMIDI error). Requires `midi.out`. The host opens the output
    /// port lazily on the first `SendMidi` call and keeps it open until the
    /// app exits.
    SendMidi { port_id: String, bytes: Vec<u8> },

    /// Open a video decoder (#345). The host responds with
    /// `PlexiEvent::VideoOpenAck { request_id, handle_id, width, height,
    /// fps, duration_ms }` on success or `PlexiEvent::VideoOpenError
    /// { request_id, error }` on failure (capability denied, source not
    /// found, decoder error, NotImplemented from the production stub).
    /// Decoded RGBA8 frames flow over the binary pipe at `pipe_id`; one
    /// pipe frame = one video frame, packed `[R,G,B,A,...]` of length
    /// `width * height * 4`.
    ///
    /// Requires `video.playback` capability. All fields required —
    /// no `serde(default)`.
    OpenVideo {
        request_id: String,
        source: String,
        pipe_id: String,
    },
    /// Drive playback state for a previously-opened video handle (#345).
    /// `handle_id` is the value returned in `VideoOpenAck`. State variants:
    /// `play`, `pause`, or `seek` to an absolute position in milliseconds.
    SetVideoState {
        handle_id: u64,
        state: crate::media::video::VideoState,
    },
    /// Close a previously-opened video handle (#345). Tears down the
    /// decoder thread and the associated binary pipe drains. No response
    /// event — fire-and-forget.
    CloseVideo { handle_id: u64 },

    /// Request the host to cd all terminals in the same pane group to `cwd`.
    /// Terminals receive `cd <cwd>\n` written to their PTY.
    CdRequest { cwd: String },

    /// Request a one-shot timer. Requires `timer` capability.
    /// Host fires `PlexiEvent::Timer { timer_id }` after `after_ms` milliseconds.
    SetTimer { timer_id: String, after_ms: u64 },
    /// Cancel a pending timer. No-op if the timer has already fired or doesn't exist.
    CancelTimer { timer_id: String },
    /// Async image fetch brokered through the host. Requires `net.http` capability.
    /// Host fetches `src`, caches under `handle`, and emits `PlexiEvent::ImageLoaded`
    /// when done. App then renders via `DrawCommand::Image { src: handle, .. }`.
    LoadImage { handle: String, src: String },

    // ── Canvas Terminal Binding Primitives (#78) ─────────────────────────
    //
    // The binding-primitive surface that lets a Canvas app drive a linked
    // terminal pane. All commands require the `terminal.bindings` capability
    // — apps without it get capability-denied responses on each call.
    //
    // The lifecycle: app emits `RequestLinkedTerminal` once at startup; host
    // opens a fresh `Pane::Terminal` next to the app and replies with
    // `PlexiEvent::LinkedTerminalReady { request_id, terminal_pane_id }`.
    // From then on every primitive references the terminal via
    // `terminal_pane_id`.
    /// Ask the host to open a fresh terminal pane next to this Canvas app
    /// and link it. Host responds with `PlexiEvent::LinkedTerminalReady
    /// { request_id, terminal_pane_id }`. `cwd` defaults to the app's
    /// workspace root when `None`. `label` is reserved for future pane-
    /// chrome decoration; currently unused by the host but pinned on the
    /// wire so apps and the host evolve together.
    ///
    /// Capability: `terminal.bindings`. Required fields — no `serde(default)`.
    RequestLinkedTerminal {
        request_id: String,
        cwd: Option<String>,
        label: Option<String>,
    },

    /// Execute `command` in a linked terminal pane.
    ///
    /// `echo: true` — the command is typed into the terminal so the user
    /// sees it (followed by a newline so the shell executes it). This is
    /// the default behaviour for click-driven UIs where the user wants to
    /// know what just ran.
    /// `echo: false` — the command is still written to the PTY, but the
    /// caller is signalling intent that the terminal is being driven
    /// programmatically (the terminal still echoes input by default at
    /// the PTY level — silent execution is best-effort).
    ///
    /// Capability: `terminal.bindings`. All fields required.
    RunInLinkedTerminal {
        terminal_pane_id: u64,
        command: String,
        echo: bool,
    },

    /// Insert `path` into the linked terminal at the cursor position.
    ///
    /// `Replace` mode: write a Ctrl-W (kill-word, shell readline default)
    /// before the path — replaces the partial word the user is typing.
    /// `Append` mode: write the path verbatim — handy for completing
    /// drag-and-drop / file-picker style flows where the user is composing
    /// a command.
    ///
    /// The host wraps the path in single quotes (POSIX) when it contains
    /// shell metacharacters so the shell doesn't expand or split it.
    ///
    /// Capability: `terminal.bindings`. All fields required.
    InsertPathToken {
        terminal_pane_id: u64,
        path: String,
        mode: PathTokenMode,
    },

    /// Ask the host to compute the command that *would* run for a given
    /// command string in the linked terminal. Doesn't execute. Used for
    /// confirmation modals before destructive operations.
    ///
    /// Host responds with `PlexiEvent::CommandPreview { request_id, command,
    /// would_run_in_cwd }`. `would_run_in_cwd` is the terminal's current
    /// working directory at request time — useful for "rm -rf .git in
    /// /tmp/foo" style confirmations.
    ///
    /// Capability: `terminal.bindings`. All fields required.
    RequestCommandPreview {
        request_id: String,
        terminal_pane_id: u64,
        command: String,
    },

    /// Open a workspace artifact (file or directory) via the host.
    ///
    /// Modes:
    ///   - `OpenInPane`        — open the path in a new app pane, e.g. a
    ///                           file browser at a directory or a text
    ///                           editor on a file. Implementation: routes
    ///                           through the file-browser app for
    ///                           directories; falls through to the OS
    ///                           default for files in this PR.
    ///   - `RevealInFinder`    — `open -R <path>` on macOS.
    ///   - `OpenWithDefault`   — `open <path>` on macOS — uses Launch
    ///                           Services to pick the registered app for
    ///                           the file's UTI.
    ///
    /// Capability: `terminal.bindings`. All fields required.
    OpenArtifact {
        path: String,
        mode: ArtifactOpenMode,
    },

    // ── Navigation stack ─────────────────────────────────────────────────
    /// App signals it has pushed a navigation level. The host appends the
    /// entry to its per-pane nav stack. While the stack has entries, the pane
    /// chrome shows a back arrow + the current view's title, and Cmd+[
    /// emits `PlexiEvent::NavBack` to the app instead of cycling tabs.
    ///
    /// `view_id` must be a stable identifier for this view (e.g. `"detail"`);
    /// `title` is shown in the pane chrome while this view is active.
    PushNav { view_id: String, title: String },
    /// App signals it has popped a navigation level. The host removes the top
    /// entry from the per-pane nav stack (saturating — no-op on empty stack).
    PopNav {},

    /// Enable or disable `PlexiEvent::MouseMove` delivery for this pane.
    ///
    /// Off by default to avoid flooding apps that don't need continuous pointer
    /// tracking. Send `{ enabled: true }` after `Ready` to start receiving
    /// `on_mouse_move` callbacks. Send `{ enabled: false }` to stop.
    SetMouseTracking { enabled: bool },

    // ── Process streaming (#358) ──────────────────────────────────────────────
    /// Spawn `command` via `sh -c` and stream its output back to the app.
    ///
    /// The host pipes stdout and stderr, batches available bytes at ~30 Hz,
    /// and delivers each batch as `PlexiEvent::StreamChunk`. When the child
    /// exits (or the app calls `CancelProcess`) the host sends
    /// `PlexiEvent::StreamEnd`. `channel` selects which output stream to
    /// forward; v1 treats `structured` identically to `stdout`.
    ///
    /// `terminal_pane_id` links the stream to an existing linked terminal
    /// for capability gating and future display contexts.
    ///
    /// Capability: `terminal.bindings`. All fields required.
    StreamProcess {
        correlation_id: String,
        terminal_pane_id: u64,
        command: String,
        channel: StreamChannel,
    },

    /// Cancel an in-flight `StreamProcess`. The host sends SIGTERM to the
    /// child, waits up to 1s, then SIGKILL. A `PlexiEvent::StreamEnd` is
    /// always delivered after cancellation.
    ///
    /// Cancelling an already-exited stream is a no-op.
    ///
    /// Capability: `terminal.bindings`. All fields required.
    CancelProcess { correlation_id: String },

    // ── File picker (#514) ────────────────────────────────────────────────────
    /// Show a native macOS file picker dialog. Requires `fs.pick` capability.
    ///
    /// `filter` is a list of file extensions without leading dots
    /// (e.g. `["mp4", "mov"]`). Empty list = accept all files.
    ///
    /// `multiple` allows selecting more than one file.
    ///
    /// Host responds with `PlexiEvent::FilePicked` (paths) or
    /// `PlexiEvent::FilePickCancelled` (user dismissed / capability denied).
    OpenFilePicker {
        request_id: String,
        filter: Vec<String>,
        multiple: bool,
    },
}

/// Inline-handled commands (processed directly in `ui()` or `background_tick()`).
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ControlCommand {
    /// SDK ready handshake. Sent once by the app after receiving Init.
    /// Host captures sdk and features_used; the message is otherwise a no-op.
    Ready {
        #[serde(default)]
        sdk: String,
        #[serde(default)]
        features_used: Vec<String>,
    },
    /// End of frame. Host renders everything queued since last FrameDone.
    FrameDone {
        /// Must match the frame_id from the triggering Render event.
        frame_id: u64,
    },
    /// Forward a log message into Plexi's logger (tagged with app_id).
    Log {
        /// One of: "error" | "warn" | "info" | "debug"
        level: String,
        message: String,
    },
    /// Ask the host to trigger a new Render event after `after_ms` milliseconds.
    /// Intended for game loops and animations — emit once per frame to sustain a
    /// tick rate without relying on egui's unconditional repaint cadence.
    /// Apps that do not emit this will still repaint on keyboard/inject events.
    ScheduleRender { after_ms: u32 },
    /// Write `text` to the OS clipboard.
    ///
    /// Routed through `egui::Context::copy_text`, which handles platform-
    /// specific clipboard backends (NSPasteboard / X11 / Wayland / Win32).
    /// Synchronous from the app's perspective — no acknowledgement event.
    /// No capability flag required: clipboard write is low-risk and the
    /// app already controls when it fires (key handler, button, etc.).
    CopyToClipboard { text: String },
    /// Request a one-shot text measurement. The host measures `text` at
    /// `font_size` with the proportional font and replies immediately with
    /// `PlexiEvent::TextMeasured { request_id, width, height }`.
    ///
    /// Use this only when layout genuinely depends on measured text width
    /// (e.g. flowing multiple badges horizontally). Avoid on hot render paths.
    MeasureText {
        request_id: String,
        text: String,
        font_size: f32,
        #[serde(default)]
        monospace: bool,
    },
    /// Measure the height of `text` wrapped at `max_width` using real host font metrics.
    /// If `max_lines` is Some(n), clamps the measurement to n rows.
    /// Replies with `PlexiEvent::TextWrappedMeasured { request_id, height }`.
    MeasureTextWrapped {
        request_id: String,
        text: String,
        font_size: f32,
        max_width: f32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_lines: Option<u32>,
    },
    /// Override the manifest-declared minimum size at runtime.
    /// Stored by the host and used as the live effective minimum from this
    /// point forward, superseding the manifest `[launch]` values.
    SetMinSize { width: f32, height: f32 },
    /// Request the host to close this app's pane gracefully.
    /// The host closes the pane on the next frame via the wants_close path.
    /// Use instead of sys.exit() to avoid triggering crash-restart on watched panes.
    CloseSelf,
}

/// Top-level wire type. The `type` field is globally unique across all three
/// inner enums, so `#[serde(untagged)]` deserializes unambiguously.
/// The JSON `{"type":"rect",...}` still works; `{"type":"ai_query",...}` still works.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
#[serde(untagged)]
pub enum DrawCommand {
    Render(RenderCommand),
    Host(AppRequest),
    Control(ControlCommand),
}

/// Replace-vs-append behaviour for `DrawCommand::InsertPathToken`.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PathTokenMode {
    /// Send Ctrl-W (kill-word) before the path so the shell's readline
    /// removes the partial word the user was typing, then write the path.
    Replace,
    /// Write the path verbatim at the cursor position.
    Append,
}

/// Routing target for `DrawCommand::OpenArtifact`.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactOpenMode {
    /// Open the path in a new Plexi pane (file browser for directories;
    /// OS default for files in v3.5).
    OpenInPane,
    /// `open -R <path>` — reveal in Finder.
    RevealInFinder,
    /// `open <path>` — Launch Services default app.
    OpenWithDefault,
}

/// One text segment inside a `DrawCommand::TextRow`.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct TextRowItem {
    pub text: String,
    pub color: String,
    pub size: f32,
    pub monospace: bool,
}

/// One group inside a `DrawCommand::Shortcuts`. Renders as `keys` chips
/// followed by `description` text.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct ShortcutPair {
    pub keys: Vec<String>,
    pub description: String,
}

/// Visibility scope for a notification.
///
/// - `Context` — visible only when the source context is the active context.
/// - `Global`  — always visible, regardless of which context is active.
///
/// Host-side enum. Apps do NOT emit this on the wire — scope is a per-app
/// user-facing policy declared in `manifest.toml::[launch] notification_scope`,
/// resolved by the host at dispatch time. Apps never think about it.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NotifyScope {
    /// Only visible when the source window is the active window. Default.
    /// In the current single-window-per-context model this is equivalent to
    /// `Context`; the distinction matters when multi-window contexts land.
    Window,
    /// Visible whenever the source context is the active context (sidebar
    /// item), regardless of which window page is showing.
    Context,
    /// Always visible regardless of which context is active.
    Global,
}

/// An action attached to a Notify command.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct NotificationAction {
    pub label: String,
    /// One of: "resume_run" | "open_intent" | "run_command"
    pub action_type: String,
    pub payload: serde_json::Value,
}

/// The shape / interaction model of a notification.
///
/// - `Message` — title + body, single Acknowledge button.
/// - `Choice`  — title + body + N options; Enter picks the focused one, ↑↓/j-k
///               cycles, 1-9 direct-selects, optional per-option `shortcut` key.
/// - `Input`   — title + body + a text field; Enter submits.
///
/// Future kinds (image / audio / video / rich) will land here without breaking
/// existing apps — `#[serde(default)]` on the field means missing `kind`
/// deserializes to `Message`.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NotifyKind {
    #[default]
    Message,
    Choice,
    Input,
}

/// Inline image attachment on a Notify command. `mime` is the MIME type
/// (`image/png` or `image/jpeg`), `base64` is the raw image bytes
/// base64-encoded. Decoded size cap (50 KB) is enforced host-side at render
/// time; oversized images render a placeholder badge instead of decoding.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq, Eq)]
pub struct NotificationImage {
    pub mime: String,
    pub base64: String,
}

/// One option in a `kind = "choice"` notification.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct NotifyOption {
    /// Visible label on the button.
    pub label: String,
    /// Value returned to the app in `PlexiEvent::NotifyAction.value`. If empty,
    /// the label is used.
    #[serde(default)]
    pub value: String,
    /// Optional single-char hotkey (e.g. "y", "n"). Case-insensitive.
    /// Reserved keys that conflict with navigation are stripped at ingestion:
    /// `j`, `k`, `h`, `l` (navigation), `1`–`9` (digit-select).
    /// Recommended safe set: letters excluding navigation keys; `y`/`n` for yes/no.
    #[serde(default)]
    pub shortcut: Option<String>,
    /// Optional host-side action to execute synchronously at click time.
    /// Format: `"action_type:action_arg"` (e.g. `"pane_focus:123"`).
    /// If set, the host executes the action before writing the response file.
    /// If not set, behavior is unchanged from before this field existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_action: Option<String>,
}

/// Returns `true` if `key` is reserved by the notification overlay for navigation.
/// Reserved: `j`, `k`, `h`, `l` (navigation) and `1`–`9` (digit-select).
pub fn is_reserved_shortcut(key: &str) -> bool {
    let Some(c) = key.chars().next() else {
        return false;
    };
    if key.chars().count() != 1 {
        return false;
    }
    matches!(c.to_ascii_lowercase(), 'j' | 'k' | 'h' | 'l') || c.is_ascii_digit() && c != '0'
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct ListItem {
    pub label: String,
    #[serde(default)]
    pub secondary: Option<String>,
    #[serde(default)]
    pub icon: Option<String>, // reserved for future use
    #[serde(default)]
    pub is_dir: bool,
}

/// Leading slot for a ListView row.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
#[serde(tag = "variant", rename_all = "snake_case")]
pub enum ListViewLeading {
    Badge { label: String, color: String },
    Avatar { handle: String },
    Icon { name: String },
    None,
}

/// One chip label on a ListView row.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct ListViewChip {
    pub label: String,
    pub color: String,
}

/// A generic typed row for a ListView.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct ListViewRowDescriptor {
    pub id: String,
    #[serde(default)]
    pub leading: Option<ListViewLeading>,
    pub primary: String,
    #[serde(default)]
    pub secondary: Option<String>,
    #[serde(default)]
    pub chips: Vec<ListViewChip>,
    #[serde(default)]
    pub trailing: Option<String>,
}

/// An item inside a ListView — either a typed row or a custom cell.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ListViewItem {
    Row(ListViewRowDescriptor),
    CustomCell {
        id: String,
        #[serde(default = "default_custom_cell_height")]
        height_hint: f32,
    },
}

impl ListViewItem {
    pub const ROW_H_BASE: f32 = 40.0;
    pub const ROW_H_WITH_SEC: f32 = 56.0;

    pub fn height(&self) -> f32 {
        match self {
            ListViewItem::Row(r) => {
                if r.secondary.is_some() {
                    Self::ROW_H_WITH_SEC
                } else {
                    Self::ROW_H_BASE
                }
            }
            ListViewItem::CustomCell { height_hint, .. } => *height_hint,
        }
    }
}

fn default_custom_cell_height() -> f32 {
    40.0
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn default_stroke_width() -> f32 {
    1.0
}

fn default_text_align() -> String {
    "top_left".to_string()
}

fn default_http_method() -> String {
    "GET".to_string()
}

fn default_image_fit() -> String {
    "contain".to_string()
}

fn default_volume() -> f32 {
    1.0
}

fn default_text_input_h() -> f32 {
    24.0
}

fn default_skeleton_radius() -> f32 {
    4.0
}

#[cfg(test)]
mod tests {
    //! Wire-format round-trip tests for the v3.2 clipboard / paste / selectable
    //! text additions (#200 + #146). These pin the on-the-wire shape — every
    //! field is required and must be present. No `#[serde(default)]` papering
    //! over missing fields.
    use super::*;
    use crate::protocol::events::PlexiEvent;

    #[test]
    fn paste_event_round_trips_serde() {
        let json = r#"{"type":"paste","text":"hello world"}"#;
        let event: PlexiEvent = serde_json::from_str(json).expect("deserialise");
        match &event {
            PlexiEvent::Paste { text } => assert_eq!(text, "hello world"),
            other => panic!("expected Paste, got {other:?}"),
        }
        let serialised = serde_json::to_string(&event).expect("serialise");
        assert!(
            serialised.contains(r#""type":"paste""#),
            "wire tag missing: {serialised}"
        );
        assert!(
            serialised.contains(r#""text":"hello world""#),
            "text missing: {serialised}"
        );
    }

    #[test]
    fn copy_to_clipboard_round_trips_serde() {
        let json = r#"{"type":"copy_to_clipboard","text":"snippet"}"#;
        let cmd: DrawCommand = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            DrawCommand::Control(ControlCommand::CopyToClipboard { text }) => {
                assert_eq!(text, "snippet")
            }
            other => panic!("expected CopyToClipboard, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""type":"copy_to_clipboard""#),
            "wire tag missing: {serialised}"
        );
    }

    #[test]
    fn text_drawcommand_with_selectable_round_trips() {
        let json = r##"{"type":"text","x":1.0,"y":2.0,"text":"hi","size":14.0,"color":"#fff","monospace":false,"bold":false,"align":"top_left","max_width":null,"elide":true,"selectable":true}"##;
        let cmd: DrawCommand = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            DrawCommand::Render(RenderCommand::Text {
                text, selectable, ..
            }) => {
                assert_eq!(text, "hi");
                assert!(*selectable, "selectable should be true");
            }
            other => panic!("expected Text, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""selectable":true"#),
            "selectable flag missing on wire: {serialised}"
        );
    }

    #[test]
    fn text_drawcommand_missing_selectable_fails_deserialise() {
        // No `selectable` field — must fail because the field is required
        // (no `#[serde(default)]` on it).
        let json = r##"{"type":"text","x":0.0,"y":0.0,"text":"x","size":14.0,"color":"#fff","max_width":null,"elide":true}"##;
        let result: Result<DrawCommand, _> = serde_json::from_str(json);
        assert!(
            result.is_err(),
            "deserialise should fail without `selectable` field"
        );
    }

    // ── v3.3 iq.query wire shape (#284) ──────────────────────────────────
    // Pin the on-the-wire shape for AiQuery / AiResponse. All fields are
    // required — no `serde(default)`.

    #[test]
    fn ai_query_drawcommand_round_trips_serde() {
        let json = r#"{"type":"ai_query","request_id":"req-1","model_tier":"medium","system":"You are helpful.","messages":[{"role":"user","content":"hi"}],"tools":[]}"#;
        let cmd: DrawCommand = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            DrawCommand::Host(AppRequest::AiQuery {
                request_id,
                model_tier,
                system,
                messages,
                tools,
            }) => {
                assert_eq!(request_id, "req-1");
                assert_eq!(*model_tier, ModelTier::Medium);
                assert_eq!(system, "You are helpful.");
                assert_eq!(messages.len(), 1);
                assert_eq!(messages[0].role, "user");
                assert_eq!(messages[0].content, "hi");
                assert!(tools.is_empty());
            }
            other => panic!("expected AiQuery, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""type":"ai_query""#),
            "wire tag missing: {serialised}"
        );
        assert!(
            serialised.contains(r#""model_tier":"medium""#),
            "model_tier missing: {serialised}"
        );
    }

    #[test]
    fn ai_response_round_trips_serde() {
        let json = r#"{"type":"ai_response","request_id":"req-1","content":"Hello!","tokens_in":12,"tokens_out":4,"error":null}"#;
        let event: PlexiEvent = serde_json::from_str(json).expect("deserialise");
        match &event {
            PlexiEvent::AiResponse {
                request_id,
                content,
                tokens_in,
                tokens_out,
                error,
            } => {
                assert_eq!(request_id, "req-1");
                assert_eq!(content.as_deref(), Some("Hello!"));
                assert_eq!(*tokens_in, 12);
                assert_eq!(*tokens_out, 4);
                assert!(error.is_none());
            }
            other => panic!("expected AiResponse, got {other:?}"),
        }
        let serialised = serde_json::to_string(&event).expect("serialise");
        assert!(
            serialised.contains(r#""type":"ai_response""#),
            "wire tag missing: {serialised}"
        );
    }

    #[test]
    fn ai_response_with_error_serde() {
        let json = r#"{"type":"ai_response","request_id":"req-2","content":null,"tokens_in":0,"tokens_out":0,"error":"capability denied: ai.query not declared in manifest"}"#;
        let event: PlexiEvent = serde_json::from_str(json).expect("deserialise");
        match &event {
            PlexiEvent::AiResponse {
                content,
                error,
                tokens_in,
                tokens_out,
                ..
            } => {
                assert!(content.is_none(), "content must be None on error");
                assert_eq!(
                    error.as_deref(),
                    Some("capability denied: ai.query not declared in manifest")
                );
                assert_eq!(*tokens_in, 0);
                assert_eq!(*tokens_out, 0);
            }
            other => panic!("expected AiResponse, got {other:?}"),
        }
    }

    #[test]
    fn ai_query_missing_required_field_fails_deserialise() {
        // No `tools` field — must fail because the field is required
        // (no `#[serde(default)]` on it).
        let json =
            r#"{"type":"ai_query","request_id":"r","model_tier":"low","system":"","messages":[]}"#;
        let result: Result<DrawCommand, _> = serde_json::from_str(json);
        assert!(
            result.is_err(),
            "deserialise should fail without `tools` field"
        );
    }

    // ── v3.4 audio capture wire shape (#277) ─────────────────────────────
    // Pin the on-the-wire shape for ListAudioDevices / AudioDevicesListed /
    // AudioCapture. Required fields fail to deserialise when absent.

    #[test]
    fn list_audio_devices_drawcommand_round_trips_serde() {
        let json = r#"{"type":"list_audio_devices","request_id":"req-9"}"#;
        let cmd: DrawCommand = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            DrawCommand::Host(AppRequest::ListAudioDevices { request_id }) => {
                assert_eq!(request_id, "req-9");
            }
            other => panic!("expected ListAudioDevices, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""type":"list_audio_devices""#),
            "wire tag missing: {serialised}"
        );
    }

    #[test]
    fn audio_devices_listed_event_round_trips_serde() {
        let json = r#"{"type":"audio_devices_listed","request_id":"req-9","inputs":[{"id":"abc","name":"Built-in Mic","default":true}],"outputs":[{"id":"def","name":"Built-in Speakers","default":true}]}"#;
        let event: PlexiEvent = serde_json::from_str(json).expect("deserialise");
        match &event {
            PlexiEvent::AudioDevicesListed {
                request_id,
                inputs,
                outputs,
                error,
            } => {
                assert_eq!(request_id, "req-9");
                assert_eq!(inputs.len(), 1);
                assert_eq!(inputs[0].id, "abc");
                assert!(inputs[0].default);
                assert_eq!(outputs.len(), 1);
                assert_eq!(outputs[0].name, "Built-in Speakers");
                assert!(error.is_none());
            }
            other => panic!("expected AudioDevicesListed, got {other:?}"),
        }
        let serialised = serde_json::to_string(&event).expect("serialise");
        assert!(
            serialised.contains(r#""type":"audio_devices_listed""#),
            "wire tag missing: {serialised}"
        );
    }

    // ── v3.4 CoreMIDI wire shape (#320) ─────────────────────────────────
    // Pin the on-the-wire shape for ListMidiDevices / MidiDevicesListed /
    // OpenMidiInput / SendMidi. Required fields fail to deserialise when absent.

    #[test]
    fn list_midi_devices_drawcommand_round_trips_serde() {
        let json = r#"{"type":"list_midi_devices","request_id":"req-m1"}"#;
        let cmd: DrawCommand = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            DrawCommand::Host(AppRequest::ListMidiDevices { request_id }) => {
                assert_eq!(request_id, "req-m1");
            }
            other => panic!("expected ListMidiDevices, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""type":"list_midi_devices""#),
            "wire tag missing: {serialised}"
        );
    }

    #[test]
    fn midi_devices_listed_event_round_trips_serde() {
        let json = r#"{"type":"midi_devices_listed","request_id":"req-m1","inputs":[{"id":"123","name":"Mock Controller","default":true}],"outputs":[{"id":"456","name":"Mock Synth","default":true}]}"#;
        let event: PlexiEvent = serde_json::from_str(json).expect("deserialise");
        match &event {
            PlexiEvent::MidiDevicesListed {
                request_id,
                inputs,
                outputs,
                error,
            } => {
                assert_eq!(request_id, "req-m1");
                assert_eq!(inputs.len(), 1);
                assert_eq!(inputs[0].id, "123");
                assert!(inputs[0].default);
                assert_eq!(outputs.len(), 1);
                assert_eq!(outputs[0].name, "Mock Synth");
                assert!(error.is_none());
            }
            other => panic!("expected MidiDevicesListed, got {other:?}"),
        }
        let serialised = serde_json::to_string(&event).expect("serialise");
        assert!(
            serialised.contains(r#""type":"midi_devices_listed""#),
            "wire tag missing: {serialised}"
        );
    }

    #[test]
    fn send_midi_drawcommand_round_trips() {
        // SendMidi carries a Vec<u8> on the wire. JSON encoding is a JSON
        // array of numbers — the human-readable shape (no base64) keeps the
        // wire debuggable and side-steps SDK plumbing for binary payloads.
        let json = r#"{"type":"send_midi","port_id":"123","bytes":[144,60,100]}"#;
        let cmd: DrawCommand = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            DrawCommand::Host(AppRequest::SendMidi { port_id, bytes }) => {
                assert_eq!(port_id, "123");
                assert_eq!(bytes, &vec![0x90u8, 0x3C, 0x64]);
            }
            other => panic!("expected SendMidi, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""type":"send_midi""#),
            "wire tag missing: {serialised}"
        );

        // Required-field discipline: dropping `bytes` fails deserialisation.
        let bad = r#"{"type":"send_midi","port_id":"123"}"#;
        assert!(
            serde_json::from_str::<DrawCommand>(bad).is_err(),
            "must fail without required `bytes` field"
        );
    }

    #[test]
    fn pipe_open_directed_drawcommand_round_trips_serde() {
        let json =
            r#"{"type":"pipe_open_directed","pipe_id":"coord-to-worker","target_pane_id":42}"#;
        let cmd: DrawCommand = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            DrawCommand::Host(AppRequest::PipeOpenDirected {
                pipe_id,
                target_pane_id,
            }) => {
                assert_eq!(pipe_id, "coord-to-worker");
                assert_eq!(*target_pane_id, 42);
            }
            other => panic!("expected PipeOpenDirected, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""type":"pipe_open_directed""#),
            "wire tag missing: {serialised}"
        );
        assert!(
            serialised.contains(r#""target_pane_id":42"#),
            "target_pane_id missing: {serialised}"
        );
    }

    #[test]
    fn pipe_open_directed_missing_target_pane_id_fails_deserialise() {
        // No `target_pane_id` — must fail. Required.
        let json = r#"{"type":"pipe_open_directed","pipe_id":"x"}"#;
        let result: Result<DrawCommand, _> = serde_json::from_str(json);
        assert!(
            result.is_err(),
            "deserialise should fail without `target_pane_id` field"
        );
    }

    #[test]
    fn audio_capture_drawcommand_required_fields() {
        // `sample_rate` and `buffer_size` are now required (no serde-default).
        // `device_id` is the only optional field.
        let bad = r#"{"type":"audio_capture","pipe_id":"mic","device_id":null}"#;
        assert!(
            serde_json::from_str::<DrawCommand>(bad).is_err(),
            "must fail without required sample_rate/buffer_size"
        );
        let good = r#"{"type":"audio_capture","pipe_id":"mic","device_id":null,"sample_rate":48000,"buffer_size":512}"#;
        let cmd: DrawCommand = serde_json::from_str(good).expect("deserialise");
        match &cmd {
            DrawCommand::Host(AppRequest::AudioCapture {
                pipe_id,
                device_id,
                sample_rate,
                buffer_size,
            }) => {
                assert_eq!(pipe_id, "mic");
                assert!(device_id.is_none());
                assert_eq!(*sample_rate, 48_000);
                assert_eq!(*buffer_size, 512);
            }
            other => panic!("expected AudioCapture, got {other:?}"),
        }
    }

    // ── v3.4 video substrate (#345) ────────────────────────────────────────
    // Pin the on-the-wire shape for OpenVideo / SetVideoState / CloseVideo
    // and the matching VideoOpenAck / VideoOpenError events. All fields
    // required — no `serde(default)`.

    #[test]
    fn open_video_drawcommand_round_trips_serde() {
        let json = r#"{"type":"open_video","request_id":"req-1","source":"mock://gradient","pipe_id":"video-stream"}"#;
        let cmd: DrawCommand = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            DrawCommand::Host(AppRequest::OpenVideo {
                request_id,
                source,
                pipe_id,
            }) => {
                assert_eq!(request_id, "req-1");
                assert_eq!(source, "mock://gradient");
                assert_eq!(pipe_id, "video-stream");
            }
            other => panic!("expected OpenVideo, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""type":"open_video""#),
            "wire tag missing: {serialised}"
        );

        // Required-field discipline — dropping any field fails.
        let bad = r#"{"type":"open_video","source":"mock://gradient","pipe_id":"video-stream"}"#;
        assert!(
            serde_json::from_str::<DrawCommand>(bad).is_err(),
            "must fail without required `request_id`"
        );
        let bad = r#"{"type":"open_video","request_id":"r","pipe_id":"p"}"#;
        assert!(
            serde_json::from_str::<DrawCommand>(bad).is_err(),
            "must fail without required `source`"
        );
        let bad = r#"{"type":"open_video","request_id":"r","source":"mock://x"}"#;
        assert!(
            serde_json::from_str::<DrawCommand>(bad).is_err(),
            "must fail without required `pipe_id`"
        );
    }

    #[test]
    fn set_video_state_drawcommand_round_trips_serde() {
        let play_json = r#"{"type":"set_video_state","handle_id":7,"state":{"kind":"play"}}"#;
        let cmd: DrawCommand = serde_json::from_str(play_json).expect("deserialise play");
        match &cmd {
            DrawCommand::Host(AppRequest::SetVideoState { handle_id, state }) => {
                assert_eq!(*handle_id, 7);
                assert_eq!(*state, crate::media::video::VideoState::Play);
            }
            other => panic!("expected SetVideoState, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""type":"set_video_state""#),
            "wire tag missing: {serialised}"
        );

        let pause_json = r#"{"type":"set_video_state","handle_id":7,"state":{"kind":"pause"}}"#;
        let cmd: DrawCommand = serde_json::from_str(pause_json).expect("deserialise pause");
        if let DrawCommand::Host(AppRequest::SetVideoState { state, .. }) = &cmd {
            assert_eq!(*state, crate::media::video::VideoState::Pause);
        } else {
            panic!("expected SetVideoState pause, got {cmd:?}");
        }

        let seek_json = r#"{"type":"set_video_state","handle_id":7,"state":{"kind":"seek","position_ms":1500}}"#;
        let cmd: DrawCommand = serde_json::from_str(seek_json).expect("deserialise seek");
        if let DrawCommand::Host(AppRequest::SetVideoState { state, .. }) = &cmd {
            assert_eq!(
                *state,
                crate::media::video::VideoState::Seek { position_ms: 1500 }
            );
        } else {
            panic!("expected SetVideoState seek, got {cmd:?}");
        }
    }

    #[test]
    fn close_video_drawcommand_round_trips_serde() {
        let json = r#"{"type":"close_video","handle_id":42}"#;
        let cmd: DrawCommand = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            DrawCommand::Host(AppRequest::CloseVideo { handle_id }) => assert_eq!(*handle_id, 42),
            other => panic!("expected CloseVideo, got {other:?}"),
        }
        let bad = r#"{"type":"close_video"}"#;
        assert!(
            serde_json::from_str::<DrawCommand>(bad).is_err(),
            "must fail without required `handle_id`"
        );
    }

    #[test]
    fn video_open_ack_round_trips_serde() {
        let json = r#"{"type":"video_open_ack","request_id":"req-1","handle_id":3,"width":640,"height":360,"fps":30.0,"duration_ms":12000}"#;
        let event: PlexiEvent = serde_json::from_str(json).expect("deserialise");
        match &event {
            PlexiEvent::VideoOpenAck {
                request_id,
                handle_id,
                width,
                height,
                fps,
                duration_ms,
            } => {
                assert_eq!(request_id, "req-1");
                assert_eq!(*handle_id, 3);
                assert_eq!(*width, 640);
                assert_eq!(*height, 360);
                assert!((*fps - 30.0).abs() < 0.01);
                assert_eq!(*duration_ms, 12_000);
            }
            other => panic!("expected VideoOpenAck, got {other:?}"),
        }
        let serialised = serde_json::to_string(&event).expect("serialise");
        assert!(
            serialised.contains(r#""type":"video_open_ack""#),
            "wire tag missing: {serialised}"
        );

        // Required-field discipline.
        let bad = r#"{"type":"video_open_ack","handle_id":3,"width":1,"height":1,"fps":30.0,"duration_ms":0}"#;
        assert!(
            serde_json::from_str::<PlexiEvent>(bad).is_err(),
            "must fail without required `request_id`"
        );
    }

    // ── v3.5 Canvas Terminal Binding Primitives (#78) ─────────────────────
    // Pin the on-the-wire shape for the new primitives. All fields are
    // required — no `serde(default)`. Enums round-trip as snake_case.

    #[test]
    fn request_linked_terminal_drawcommand_round_trips_serde() {
        let json = r#"{"type":"request_linked_terminal","request_id":"req-1","cwd":"/tmp/foo","label":"bindings demo"}"#;
        let cmd: DrawCommand = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            DrawCommand::Host(AppRequest::RequestLinkedTerminal {
                request_id,
                cwd,
                label,
            }) => {
                assert_eq!(request_id, "req-1");
                assert_eq!(cwd.as_deref(), Some("/tmp/foo"));
                assert_eq!(label.as_deref(), Some("bindings demo"));
            }
            other => panic!("expected RequestLinkedTerminal, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""type":"request_linked_terminal""#),
            "wire tag missing: {serialised}"
        );

        // Required-field discipline — no `serde(default)` on `request_id`.
        let bad = r#"{"type":"request_linked_terminal","cwd":null,"label":null}"#;
        assert!(
            serde_json::from_str::<DrawCommand>(bad).is_err(),
            "must fail without required `request_id`"
        );

        // Optional fields: explicit null deserialises to None.
        let null_json =
            r#"{"type":"request_linked_terminal","request_id":"r2","cwd":null,"label":null}"#;
        let cmd: DrawCommand = serde_json::from_str(null_json).expect("deserialise null");
        match &cmd {
            DrawCommand::Host(AppRequest::RequestLinkedTerminal { cwd, label, .. }) => {
                assert!(cwd.is_none());
                assert!(label.is_none());
            }
            other => panic!("expected RequestLinkedTerminal, got {other:?}"),
        }
    }

    #[test]
    fn linked_terminal_ready_event_round_trips_serde() {
        let json = r#"{"type":"linked_terminal_ready","request_id":"req-1","terminal_pane_id":42}"#;
        let event: PlexiEvent = serde_json::from_str(json).expect("deserialise");
        match &event {
            PlexiEvent::LinkedTerminalReady {
                request_id,
                terminal_pane_id,
            } => {
                assert_eq!(request_id, "req-1");
                assert_eq!(*terminal_pane_id, 42);
            }
            other => panic!("expected LinkedTerminalReady, got {other:?}"),
        }
        let serialised = serde_json::to_string(&event).expect("serialise");
        assert!(
            serialised.contains(r#""type":"linked_terminal_ready""#),
            "wire tag missing: {serialised}"
        );
        let bad = r#"{"type":"linked_terminal_ready","request_id":"r"}"#;
        assert!(
            serde_json::from_str::<PlexiEvent>(bad).is_err(),
            "must fail without required `terminal_pane_id`"
        );
    }

    #[test]
    fn run_in_linked_terminal_round_trips_serde() {
        let json = r#"{"type":"run_in_linked_terminal","terminal_pane_id":42,"command":"ls -la","echo":true}"#;
        let cmd: DrawCommand = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            DrawCommand::Host(AppRequest::RunInLinkedTerminal {
                terminal_pane_id,
                command,
                echo,
            }) => {
                assert_eq!(*terminal_pane_id, 42);
                assert_eq!(command, "ls -la");
                assert!(*echo);
            }
            other => panic!("expected RunInLinkedTerminal, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""type":"run_in_linked_terminal""#),
            "wire tag missing: {serialised}"
        );

        // Required-field discipline — `echo` has no default.
        let bad = r#"{"type":"run_in_linked_terminal","terminal_pane_id":1,"command":"ls"}"#;
        assert!(
            serde_json::from_str::<DrawCommand>(bad).is_err(),
            "must fail without required `echo`"
        );
    }

    #[test]
    fn insert_path_token_mode_enum_serde() {
        let replace_json =
            r#"{"type":"insert_path_token","terminal_pane_id":7,"path":"/tmp/x","mode":"replace"}"#;
        let cmd: DrawCommand = serde_json::from_str(replace_json).expect("deserialise replace");
        match &cmd {
            DrawCommand::Host(AppRequest::InsertPathToken {
                mode,
                path,
                terminal_pane_id,
            }) => {
                assert_eq!(*mode, PathTokenMode::Replace);
                assert_eq!(path, "/tmp/x");
                assert_eq!(*terminal_pane_id, 7);
            }
            other => panic!("expected InsertPathToken, got {other:?}"),
        }

        let append_json =
            r#"{"type":"insert_path_token","terminal_pane_id":7,"path":"/tmp/y","mode":"append"}"#;
        let cmd: DrawCommand = serde_json::from_str(append_json).expect("deserialise append");
        if let DrawCommand::Host(AppRequest::InsertPathToken { mode, .. }) = &cmd {
            assert_eq!(*mode, PathTokenMode::Append);
        } else {
            panic!("expected InsertPathToken, got {cmd:?}");
        }

        // Round-trip serialise → snake_case on the wire.
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""mode":"append""#),
            "mode must serialise as snake_case: {serialised}"
        );

        // Bad mode rejected loudly.
        let bad =
            r#"{"type":"insert_path_token","terminal_pane_id":1,"path":"/x","mode":"INSERT"}"#;
        assert!(
            serde_json::from_str::<DrawCommand>(bad).is_err(),
            "unknown mode must fail to deserialise"
        );
    }

    #[test]
    fn request_command_preview_round_trips_serde() {
        let json = r#"{"type":"request_command_preview","request_id":"req-9","terminal_pane_id":3,"command":"rm -rf .git"}"#;
        let cmd: DrawCommand = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            DrawCommand::Host(AppRequest::RequestCommandPreview {
                request_id,
                terminal_pane_id,
                command,
            }) => {
                assert_eq!(request_id, "req-9");
                assert_eq!(*terminal_pane_id, 3);
                assert_eq!(command, "rm -rf .git");
            }
            other => panic!("expected RequestCommandPreview, got {other:?}"),
        }

        let preview_json = r#"{"type":"command_preview","request_id":"req-9","command":"rm -rf .git","would_run_in_cwd":"/tmp/foo"}"#;
        let event: PlexiEvent = serde_json::from_str(preview_json).expect("deserialise event");
        match &event {
            PlexiEvent::CommandPreview {
                request_id,
                command,
                would_run_in_cwd,
            } => {
                assert_eq!(request_id, "req-9");
                assert_eq!(command, "rm -rf .git");
                assert_eq!(would_run_in_cwd, "/tmp/foo");
            }
            other => panic!("expected CommandPreview, got {other:?}"),
        }
    }

    #[test]
    fn open_artifact_mode_enum_serde() {
        let cases = [
            ("open_in_pane", ArtifactOpenMode::OpenInPane),
            ("reveal_in_finder", ArtifactOpenMode::RevealInFinder),
            ("open_with_default", ArtifactOpenMode::OpenWithDefault),
        ];
        for (wire, expected) in cases {
            let json = format!(r#"{{"type":"open_artifact","path":"/tmp/x","mode":"{wire}"}}"#);
            let cmd: DrawCommand = serde_json::from_str(&json).expect("deserialise");
            match &cmd {
                DrawCommand::Host(AppRequest::OpenArtifact { path, mode }) => {
                    assert_eq!(path, "/tmp/x");
                    assert_eq!(*mode, expected, "wire {wire} → {expected:?}");
                }
                other => panic!("expected OpenArtifact, got {other:?}"),
            }
        }

        // Round-trip serialise → snake_case on the wire.
        let cmd = DrawCommand::Host(AppRequest::OpenArtifact {
            path: "/tmp/x".to_string(),
            mode: ArtifactOpenMode::RevealInFinder,
        });
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""mode":"reveal_in_finder""#),
            "snake_case missing: {serialised}"
        );
    }

    #[test]
    fn video_open_error_round_trips_serde() {
        let json = r#"{"type":"video_open_error","request_id":"req-1","error":"video decoder not implemented"}"#;
        let event: PlexiEvent = serde_json::from_str(json).expect("deserialise");
        match &event {
            PlexiEvent::VideoOpenError { request_id, error } => {
                assert_eq!(request_id, "req-1");
                assert!(error.contains("not implemented"));
            }
            other => panic!("expected VideoOpenError, got {other:?}"),
        }
        let bad = r#"{"type":"video_open_error","error":"x"}"#;
        assert!(
            serde_json::from_str::<PlexiEvent>(bad).is_err(),
            "must fail without required `request_id`"
        );
    }

    // ── v3.5 P2 rich notification panel (#74) ─────────────────────────────
    // Image attachments — inline base64 + pipe reference. Both are optional;
    // missing them deserialises to None via serde's natural Option handling
    // (no `#[serde(default)]` shim). Mutually exclusive at render time.

    #[test]
    fn notify_choice_action_round_trips_serde() {
        // Choice action shape: each NotifyOption carries a label, a value
        // (the structured `payload` from issue #74), and an optional
        // single-char hotkey (`shortcut`, the `key` from issue #74).
        let json = r#"{"type":"notify","level":"info","title":"Pick","body":"choose","kind":"choice","options":[{"label":"A","value":"sidebar","shortcut":"1"},{"label":"B","value":"fullwidth","shortcut":"2"}],"priority":100}"#;
        let cmd: DrawCommand = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            DrawCommand::Host(AppRequest::Notify {
                kind,
                options,
                priority,
                image_inline,
                image_pipe_id,
                ..
            }) => {
                assert_eq!(*kind, NotifyKind::Choice);
                assert_eq!(options.len(), 2);
                assert_eq!(options[0].value, "sidebar");
                assert_eq!(options[0].shortcut.as_deref(), Some("1"));
                assert_eq!(options[1].value, "fullwidth");
                assert_eq!(*priority, 100);
                assert!(
                    image_inline.is_none(),
                    "image_inline should default to None"
                );
                assert!(
                    image_pipe_id.is_none(),
                    "image_pipe_id should default to None"
                );
            }
            other => panic!("expected Notify, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""kind":"choice""#),
            "kind missing: {serialised}"
        );
        assert!(
            serialised.contains(r#""value":"sidebar""#),
            "payload missing: {serialised}"
        );
    }

    #[test]
    fn notify_with_inline_image_round_trips_serde() {
        // 4-byte base64 payload — well under the 50 KB cap. The host will
        // attempt to decode + render; tiny or invalid images render a
        // placeholder, never crash.
        let json = r#"{"type":"notify","level":"info","title":"Pic","body":"see image","kind":"message","priority":50,"image_inline":{"mime":"image/png","base64":"AAAA"}}"#;
        let cmd: DrawCommand = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            DrawCommand::Host(AppRequest::Notify {
                image_inline,
                image_pipe_id,
                ..
            }) => {
                let img = image_inline.as_ref().expect("inline image present");
                assert_eq!(img.mime, "image/png");
                assert_eq!(img.base64, "AAAA");
                assert!(image_pipe_id.is_none());
            }
            other => panic!("expected Notify, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""image_inline":{"mime":"image/png","base64":"AAAA"}"#),
            "image_inline missing: {serialised}"
        );
        // Skip-serializing-if-none means image_pipe_id is absent on the wire
        // when None — keeps existing notifications byte-identical.
        assert!(
            !serialised.contains("image_pipe_id"),
            "absent fields should not appear: {serialised}"
        );
    }

    #[test]
    fn notify_with_image_pipe_id_round_trips_serde() {
        let json = r#"{"type":"notify","level":"info","title":"Pic","body":"piped","kind":"message","priority":50,"image_pipe_id":"render-out"}"#;
        let cmd: DrawCommand = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            DrawCommand::Host(AppRequest::Notify {
                image_pipe_id,
                image_inline,
                ..
            }) => {
                assert_eq!(image_pipe_id.as_deref(), Some("render-out"));
                assert!(image_inline.is_none());
            }
            other => panic!("expected Notify, got {other:?}"),
        }
    }

    #[test]
    fn notify_without_image_fields_round_trips_serde() {
        // Existing apps that never set image fields must continue to work.
        // Missing `image_inline` / `image_pipe_id` deserialise as None.
        let json = r#"{"type":"notify","level":"info","title":"Plain","body":"no image","kind":"message","priority":50}"#;
        let cmd: DrawCommand = serde_json::from_str(json).expect("deserialise");
        match cmd {
            DrawCommand::Host(AppRequest::Notify {
                image_inline,
                image_pipe_id,
                ..
            }) => {
                assert!(image_inline.is_none());
                assert!(image_pipe_id.is_none());
            }
            other => panic!("expected Notify, got {other:?}"),
        }
    }

    // ── v3.5 StreamProcess / CancelProcess / StreamChunk / StreamEnd (#358) ──

    #[test]
    fn stream_channel_serializes_as_snake_case() {
        assert_eq!(
            serde_json::to_string(&StreamChannel::Stdout).unwrap(),
            r#""stdout""#
        );
        assert_eq!(
            serde_json::to_string(&StreamChannel::Stderr).unwrap(),
            r#""stderr""#
        );
        assert_eq!(
            serde_json::to_string(&StreamChannel::Structured).unwrap(),
            r#""structured""#
        );
        let parsed: StreamChannel = serde_json::from_str(r#""stdout""#).unwrap();
        assert_eq!(parsed, StreamChannel::Stdout);
    }

    #[test]
    fn stream_process_drawcommand_round_trips_serde() {
        let json = r#"{"type":"stream_process","correlation_id":"cid-1","terminal_pane_id":42,"command":"ls -la","channel":"stdout"}"#;
        let cmd: DrawCommand = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            DrawCommand::Host(AppRequest::StreamProcess {
                correlation_id,
                terminal_pane_id,
                command,
                channel,
            }) => {
                assert_eq!(correlation_id, "cid-1");
                assert_eq!(*terminal_pane_id, 42);
                assert_eq!(command, "ls -la");
                assert_eq!(*channel, StreamChannel::Stdout);
            }
            other => panic!("expected StreamProcess, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""type":"stream_process""#),
            "wire tag missing: {serialised}"
        );

        let bad =
            r#"{"type":"stream_process","terminal_pane_id":42,"command":"ls","channel":"stdout"}"#;
        assert!(
            serde_json::from_str::<DrawCommand>(bad).is_err(),
            "must fail without required `correlation_id`"
        );
    }

    #[test]
    fn cancel_process_drawcommand_round_trips_serde() {
        let json = r#"{"type":"cancel_process","correlation_id":"cid-2"}"#;
        let cmd: DrawCommand = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            DrawCommand::Host(AppRequest::CancelProcess { correlation_id }) => {
                assert_eq!(correlation_id, "cid-2");
            }
            other => panic!("expected CancelProcess, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""type":"cancel_process""#),
            "wire tag missing: {serialised}"
        );

        let bad = r#"{"type":"cancel_process"}"#;
        assert!(
            serde_json::from_str::<DrawCommand>(bad).is_err(),
            "must fail without required `correlation_id`"
        );
    }

    #[test]
    fn stream_chunk_event_round_trips_serde() {
        let json = r#"{"type":"stream_chunk","correlation_id":"cid-1","channel":"stderr","bytes":[72,101,108,108,111]}"#;
        let event: PlexiEvent = serde_json::from_str(json).expect("deserialise");
        match &event {
            PlexiEvent::StreamChunk {
                correlation_id,
                channel,
                bytes,
            } => {
                assert_eq!(correlation_id, "cid-1");
                assert_eq!(*channel, StreamChannel::Stderr);
                assert_eq!(bytes, &[72u8, 101, 108, 108, 111]);
            }
            other => panic!("expected StreamChunk, got {other:?}"),
        }
        let serialised = serde_json::to_string(&event).expect("serialise");
        assert!(
            serialised.contains(r#""type":"stream_chunk""#),
            "wire tag missing: {serialised}"
        );

        let bad = r#"{"type":"stream_chunk","channel":"stdout","bytes":[1]}"#;
        assert!(
            serde_json::from_str::<PlexiEvent>(bad).is_err(),
            "must fail without required `correlation_id`"
        );
    }

    #[test]
    fn stream_end_event_round_trips_serde() {
        let json = r#"{"type":"stream_end","correlation_id":"cid-1","exit_code":0}"#;
        let event: PlexiEvent = serde_json::from_str(json).expect("deserialise");
        match &event {
            PlexiEvent::StreamEnd {
                correlation_id,
                exit_code,
            } => {
                assert_eq!(correlation_id, "cid-1");
                assert_eq!(*exit_code, 0);
            }
            other => panic!("expected StreamEnd, got {other:?}"),
        }
        let serialised = serde_json::to_string(&event).expect("serialise");
        assert!(
            serialised.contains(r#""type":"stream_end""#),
            "wire tag missing: {serialised}"
        );

        let json_nonzero = r#"{"type":"stream_end","correlation_id":"cid-2","exit_code":127}"#;
        let event: PlexiEvent = serde_json::from_str(json_nonzero).expect("deserialise nonzero");
        match &event {
            PlexiEvent::StreamEnd { exit_code, .. } => assert_eq!(*exit_code, 127),
            other => panic!("expected StreamEnd, got {other:?}"),
        }

        let bad = r#"{"type":"stream_end","exit_code":0}"#;
        assert!(
            serde_json::from_str::<PlexiEvent>(bad).is_err(),
            "must fail without required `correlation_id`"
        );
    }

    #[test]
    fn test_notify_timeout_fields() {
        let json = r#"{"type":"notify","level":"info","title":"T","body":"B","priority":50,"timeout_secs":30,"on_dismiss":"user_ignored"}"#;
        let cmd: DrawCommand = serde_json::from_str(json).expect("deserialise");
        match cmd {
            DrawCommand::Host(AppRequest::Notify {
                timeout_secs,
                on_dismiss,
                ..
            }) => {
                assert_eq!(timeout_secs, Some(30));
                assert_eq!(on_dismiss.as_deref(), Some("user_ignored"));
            }
            other => panic!("expected AppRequest::Notify, got {other:?}"),
        }
    }

    #[test]
    fn test_notify_timeout_fields_default() {
        let json = r#"{"type":"notify","level":"info","title":"T","body":"B","priority":50}"#;
        let cmd: DrawCommand = serde_json::from_str(json).expect("deserialise");
        match cmd {
            DrawCommand::Host(AppRequest::Notify {
                timeout_secs,
                on_dismiss,
                ..
            }) => {
                assert!(timeout_secs.is_none());
                assert!(on_dismiss.is_none());
            }
            other => panic!("expected AppRequest::Notify, got {other:?}"),
        }
    }

    #[test]
    fn set_pane_title_deserializes() {
        let json = r#"{"type":"set_pane_title","pane_id":42,"name":"my label"}"#;
        let cmd: DrawCommand = serde_json::from_str(json).unwrap();
        assert!(matches!(
            cmd,
            DrawCommand::Host(AppRequest::SetPaneTitle { pane_id: 42, .. })
        ));
    }

    #[test]
    fn spawn_pane_drawcommand_round_trips_serde() {
        let json = r#"{"type":"spawn_pane","type_id":"snake","layout":"split_v","args":["--foo"],"pipe_id":"p1"}"#;
        let cmd: DrawCommand = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            DrawCommand::Host(AppRequest::SpawnPane {
                type_id,
                layout,
                args,
                pipe_id,
                from_pane_id,
                request_id,
                ..
            }) => {
                assert_eq!(type_id, "snake");
                assert_eq!(layout.as_deref(), Some("split_v"));
                assert_eq!(args, &["--foo"]);
                assert_eq!(pipe_id.as_deref(), Some("p1"));
                assert!(from_pane_id.is_none());
                assert!(request_id.is_none());
            }
            other => panic!("expected SpawnPane, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""type":"spawn_pane""#),
            "wire tag missing: {serialised}"
        );

        // defaults: layout is None (absent from wire), args to [], pipe_id absent
        let minimal = r#"{"type":"spawn_pane","type_id":"snake"}"#;
        let cmd2: DrawCommand = serde_json::from_str(minimal).expect("deserialise minimal");
        match &cmd2 {
            DrawCommand::Host(AppRequest::SpawnPane {
                layout,
                args,
                pipe_id,
                from_pane_id,
                request_id,
                ..
            }) => {
                assert!(layout.is_none(), "absent layout must deserialise to None");
                assert!(args.is_empty());
                assert!(pipe_id.is_none());
                assert!(from_pane_id.is_none());
                assert!(request_id.is_none());
            }
            other => panic!("expected SpawnPane, got {other:?}"),
        }
    }

    #[test]
    fn pane_spawned_event_round_trips_serde() {
        let json = r#"{"type":"pane_spawned","pane_id":99}"#;
        let event: PlexiEvent = serde_json::from_str(json).expect("deserialise");
        match &event {
            PlexiEvent::PaneSpawned { pane_id, .. } => {
                assert_eq!(*pane_id, 99);
            }
            other => panic!("expected PaneSpawned, got {other:?}"),
        }
        // request_id is None when not present
        assert!(matches!(
            event,
            PlexiEvent::PaneSpawned {
                request_id: None,
                ..
            }
        ));
        let serialised = serde_json::to_string(&event).expect("serialise");
        assert!(
            serialised.contains(r#""type":"pane_spawned""#),
            "wire tag missing: {serialised}"
        );

        let bad = r#"{"type":"pane_spawned"}"#;
        assert!(
            serde_json::from_str::<PlexiEvent>(bad).is_err(),
            "must fail without pane_id"
        );
    }

    #[test]
    fn pane_spawn_error_event_round_trips_serde() {
        let json = r#"{"type":"pane_spawn_error","reason":"capability denied"}"#;
        let event: PlexiEvent = serde_json::from_str(json).expect("deserialise");
        match &event {
            PlexiEvent::PaneSpawnError { reason, .. } => assert_eq!(reason, "capability denied"),
            other => panic!("expected PaneSpawnError, got {other:?}"),
        }
        let serialised = serde_json::to_string(&event).expect("serialise");
        assert!(
            serialised.contains(r#""type":"pane_spawn_error""#),
            "wire tag missing: {serialised}"
        );
    }

    #[test]
    fn spawn_pane_with_new_fields_round_trips_serde() {
        let json = r#"{"type":"spawn_pane","type_id":"snake","layout":"split_v","from_pane_id":42,"request_id":"req-1"}"#;
        let cmd: DrawCommand = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            DrawCommand::Host(AppRequest::SpawnPane {
                from_pane_id,
                request_id,
                ..
            }) => {
                assert_eq!(*from_pane_id, Some(42u64));
                assert_eq!(request_id.as_deref(), Some("req-1"));
            }
            other => panic!("expected SpawnPane, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""from_pane_id":42"#),
            "from_pane_id missing: {serialised}"
        );
        assert!(
            serialised.contains(r#""request_id":"req-1""#),
            "request_id missing: {serialised}"
        );
    }

    #[test]
    fn spawn_pane_no_focus_round_trips_serde() {
        let json = r#"{"type":"spawn_pane","type_id":"terminal","no_focus":true}"#;
        let cmd: DrawCommand = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            DrawCommand::Host(AppRequest::SpawnPane { no_focus, .. }) => {
                assert!(*no_focus, "no_focus should be true");
            }
            other => panic!("expected SpawnPane, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""no_focus":true"#),
            "no_focus missing: {serialised}"
        );

        // default (false) should be omitted from serialised output
        let json_default = r#"{"type":"spawn_pane","type_id":"terminal"}"#;
        let cmd_default: DrawCommand =
            serde_json::from_str(json_default).expect("deserialise default");
        match &cmd_default {
            DrawCommand::Host(AppRequest::SpawnPane { no_focus, .. }) => {
                assert!(!*no_focus, "no_focus should default to false");
            }
            other => panic!("expected SpawnPane, got {other:?}"),
        }
        let serialised_default = serde_json::to_string(&cmd_default).expect("serialise default");
        assert!(
            !serialised_default.contains("no_focus"),
            "no_focus should be omitted when false: {serialised_default}"
        );
    }

    #[test]
    fn spawn_pane_with_workspace_root_round_trips_serde() {
        let json =
            r#"{"type":"spawn_pane","type_id":"terminal","workspace_root":"/tmp/github-repo"}"#;
        let cmd: DrawCommand = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            DrawCommand::Host(AppRequest::SpawnPane { workspace_root, .. }) => {
                assert_eq!(workspace_root.as_deref(), Some("/tmp/github-repo"));
            }
            other => panic!("expected SpawnPane, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""workspace_root":"/tmp/github-repo""#),
            "workspace_root missing: {serialised}"
        );

        // None should be omitted from serialised output.
        let json_absent = r#"{"type":"spawn_pane","type_id":"terminal"}"#;
        let cmd_absent: DrawCommand =
            serde_json::from_str(json_absent).expect("deserialise absent");
        match &cmd_absent {
            DrawCommand::Host(AppRequest::SpawnPane { workspace_root, .. }) => {
                assert!(
                    workspace_root.is_none(),
                    "absent workspace_root must deserialise to None"
                );
            }
            other => panic!("expected SpawnPane, got {other:?}"),
        }
        let serialised_absent = serde_json::to_string(&cmd_absent).expect("serialise absent");
        assert!(
            !serialised_absent.contains("workspace_root"),
            "workspace_root should be omitted when None: {serialised_absent}"
        );
    }

    #[test]
    fn spawn_pane_name_round_trips_serde() {
        let json = r#"{"type":"spawn_pane","type_id":"terminal","name":"dev server"}"#;
        let cmd: DrawCommand = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            DrawCommand::Host(AppRequest::SpawnPane { name, .. }) => {
                assert_eq!(name.as_deref(), Some("dev server"));
            }
            other => panic!("expected SpawnPane, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""name":"dev server""#),
            "name missing: {serialised}"
        );

        let json_absent = r#"{"type":"spawn_pane","type_id":"terminal"}"#;
        let cmd_absent: DrawCommand =
            serde_json::from_str(json_absent).expect("deserialise absent");
        match &cmd_absent {
            DrawCommand::Host(AppRequest::SpawnPane { name, .. }) => {
                assert!(name.is_none(), "absent name must deserialise to None");
            }
            other => panic!("expected SpawnPane, got {other:?}"),
        }
        let serialised_absent = serde_json::to_string(&cmd_absent).expect("serialise absent");
        assert!(
            !serialised_absent.contains("name"),
            "name should be omitted when None: {serialised_absent}"
        );
    }

    #[test]
    fn pane_spawned_with_request_id_round_trips_serde() {
        let json = r#"{"type":"pane_spawned","pane_id":99,"request_id":"req-abc"}"#;
        let event: PlexiEvent = serde_json::from_str(json).expect("deserialise");
        match &event {
            PlexiEvent::PaneSpawned {
                pane_id,
                request_id,
            } => {
                assert_eq!(*pane_id, 99);
                assert_eq!(request_id.as_deref(), Some("req-abc"));
            }
            other => panic!("expected PaneSpawned, got {other:?}"),
        }
        let serialised = serde_json::to_string(&event).expect("serialise");
        assert!(
            serialised.contains(r#""request_id":"req-abc""#),
            "request_id missing: {serialised}"
        );
        // Omitting request_id → None
        let no_req: PlexiEvent =
            serde_json::from_str(r#"{"type":"pane_spawned","pane_id":1}"#).unwrap();
        assert!(matches!(
            no_req,
            PlexiEvent::PaneSpawned {
                request_id: None,
                ..
            }
        ));
    }

    #[test]
    fn pane_spawn_error_with_request_id_round_trips_serde() {
        let json = r#"{"type":"pane_spawn_error","reason":"denied","request_id":"req-xyz"}"#;
        let event: PlexiEvent = serde_json::from_str(json).expect("deserialise");
        match &event {
            PlexiEvent::PaneSpawnError { reason, request_id } => {
                assert_eq!(reason, "denied");
                assert_eq!(request_id.as_deref(), Some("req-xyz"));
            }
            other => panic!("expected PaneSpawnError, got {other:?}"),
        }
        let serialised = serde_json::to_string(&event).expect("serialise");
        assert!(
            serialised.contains(r#""request_id":"req-xyz""#),
            "request_id missing: {serialised}"
        );
        // Omitting request_id → None
        let no_req: PlexiEvent =
            serde_json::from_str(r#"{"type":"pane_spawn_error","reason":"x"}"#).unwrap();
        assert!(matches!(
            no_req,
            PlexiEvent::PaneSpawnError {
                request_id: None,
                ..
            }
        ));
    }

    #[test]
    fn is_reserved_shortcut_covers_expected_set() {
        // Navigation keys — reserved
        assert!(is_reserved_shortcut("j"));
        assert!(is_reserved_shortcut("k"));
        assert!(is_reserved_shortcut("h"));
        assert!(is_reserved_shortcut("l"));
        assert!(is_reserved_shortcut("J")); // case-insensitive
                                            // Digit-select keys — reserved
        assert!(is_reserved_shortcut("1"));
        assert!(is_reserved_shortcut("9"));
        // 0 is NOT reserved (1-9 only)
        assert!(!is_reserved_shortcut("0"));
        // Safe keys — not reserved
        assert!(!is_reserved_shortcut("y"));
        assert!(!is_reserved_shortcut("n"));
        assert!(!is_reserved_shortcut("a"));
        // Multi-char is not a valid shortcut — treated as not reserved
        assert!(!is_reserved_shortcut("jk"));
        // Empty is not reserved
        assert!(!is_reserved_shortcut(""));
    }

    #[test]
    fn key_pane_drawcommand_round_trips_serde() {
        let json =
            r#"{"type":"key_pane","pane_id":42,"key":"enter","response_file":"result.json"}"#;
        let cmd: DrawCommand = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            DrawCommand::Host(AppRequest::KeyPane {
                pane_id,
                key,
                response_file,
            }) => {
                assert_eq!(*pane_id, 42);
                assert_eq!(key, "enter");
                assert_eq!(response_file.as_deref(), Some("result.json"));
            }
            other => panic!("expected KeyPane, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""type":"key_pane""#),
            "wire tag missing: {serialised}"
        );

        // Optional field: response_file absent → None
        let minimal = r#"{"type":"key_pane","pane_id":1,"key":"h"}"#;
        let cmd2: DrawCommand = serde_json::from_str(minimal).expect("deserialise minimal");
        match &cmd2 {
            DrawCommand::Host(AppRequest::KeyPane { response_file, .. }) => {
                assert!(
                    response_file.is_none(),
                    "absent response_file must deserialise to None"
                );
            }
            other => panic!("expected KeyPane, got {other:?}"),
        }

        // Required-field discipline: missing key field must fail
        let bad = r#"{"type":"key_pane","pane_id":1}"#;
        assert!(
            serde_json::from_str::<DrawCommand>(bad).is_err(),
            "must fail without required key field"
        );
    }

    #[test]
    fn capture_pane_command_round_trips_serde() {
        let json =
            r#"{"type":"capture_pane","pane_id":7,"lines":20,"response_file":"capture.json"}"#;
        let cmd: DrawCommand = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            DrawCommand::Host(AppRequest::CapturePane {
                pane_id,
                lines,
                response_file,
                full_output,
                from_cursor,
            }) => {
                assert_eq!(*pane_id, 7);
                assert_eq!(*lines, 20);
                assert_eq!(response_file, "capture.json");
                assert!(!full_output, "full_output should default to false");
                assert!(from_cursor.is_none(), "from_cursor should default to None");
            }
            other => panic!("expected CapturePane, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""type":"capture_pane""#),
            "wire tag missing: {serialised}"
        );
        assert!(
            !serialised.contains("full_output"),
            "full_output=false should be omitted from wire format: {serialised}"
        );

        // full_output=true should round-trip
        let json_full = r#"{"type":"capture_pane","pane_id":7,"lines":20,"response_file":"capture.json","full_output":true}"#;
        let cmd_full: DrawCommand =
            serde_json::from_str(json_full).expect("deserialise full_output");
        match &cmd_full {
            DrawCommand::Host(AppRequest::CapturePane { full_output, .. }) => {
                assert!(*full_output, "full_output should be true when set");
            }
            other => panic!("expected CapturePane, got {other:?}"),
        }

        // from_cursor should round-trip
        let json_cursor = r#"{"type":"capture_pane","pane_id":7,"lines":20,"response_file":"capture.json","from_cursor":42}"#;
        let cmd_cursor: DrawCommand =
            serde_json::from_str(json_cursor).expect("deserialise from_cursor");
        match &cmd_cursor {
            DrawCommand::Host(AppRequest::CapturePane { from_cursor, .. }) => {
                assert_eq!(*from_cursor, Some(42), "from_cursor should be Some(42)");
            }
            other => panic!("expected CapturePane, got {other:?}"),
        }
        // from_cursor absent → None
        let cmd_no_cursor: DrawCommand = serde_json::from_str(json).expect("deserialise no cursor");
        match &cmd_no_cursor {
            DrawCommand::Host(AppRequest::CapturePane { from_cursor, .. }) => {
                assert!(from_cursor.is_none(), "from_cursor should default to None");
            }
            other => panic!("expected CapturePane, got {other:?}"),
        }
        // from_cursor=0 should be omitted from wire format
        let cmd_zero: DrawCommand = serde_json::from_str(
            r#"{"type":"capture_pane","pane_id":7,"lines":5,"response_file":"f.json"}"#,
        )
        .unwrap();
        let s = serde_json::to_string(&cmd_zero).unwrap();
        assert!(
            !s.contains("from_cursor"),
            "absent from_cursor must not appear on wire: {s}"
        );

        // Missing required fields must fail
        let bad = r#"{"type":"capture_pane","pane_id":1}"#;
        assert!(
            serde_json::from_str::<DrawCommand>(bad).is_err(),
            "must fail without lines and response_file"
        );
    }

    #[test]
    fn layout_command_round_trips_serde() {
        let json = r##"{"type":"layout","x":10.0,"y":20.0,"direction":"row","gap":6.0,"children":[{"type":"leaf","command":{"type":"badge","x":0.0,"y":0.0,"label":"4 files","fill":"#89b4fa","fg":"#1e1e2e","font_size":11.0,"radius":8.0}},{"type":"leaf","command":{"type":"text","x":0.0,"y":0.0,"text":"modified","size":12.0,"color":"#cdd6f4","monospace":false,"bold":false,"align":"top_left","elide":false,"selectable":false}}]}"##;
        let cmd: DrawCommand = serde_json::from_str(json).expect("deserialise layout command");
        match &cmd {
            DrawCommand::Render(RenderCommand::Layout {
                direction,
                children,
                gap,
                ..
            }) => {
                assert!(matches!(direction, LayoutDirection::Row));
                assert_eq!(children.len(), 2);
                assert!((gap - 6.0).abs() < 0.01);
            }
            other => panic!("expected Layout, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""type":"layout""#),
            "wire tag missing: {serialised}"
        );
    }

    #[test]
    fn set_min_size_round_trips_serde() {
        let json = r#"{"type":"set_min_size","width":200.0,"height":100.0}"#;
        let cmd: DrawCommand = serde_json::from_str(json).unwrap();
        match cmd {
            DrawCommand::Control(ControlCommand::SetMinSize { width, height }) => {
                assert_eq!(width, 200.0);
                assert_eq!(height, 100.0);
            }
            other => panic!("expected SetMinSize, got {other:?}"),
        }
        let serialised = serde_json::to_string(&DrawCommand::Control(ControlCommand::SetMinSize {
            width: 200.0,
            height: 100.0,
        }))
        .unwrap();
        assert!(
            serialised.contains(r#""type":"set_min_size""#),
            "got: {serialised}"
        );
    }

    #[test]
    fn set_context_description_round_trips_serde() {
        let json = r#"{"type":"set_context_description","description":"Main project workspace"}"#;
        let cmd: DrawCommand = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            DrawCommand::Host(AppRequest::SetContextDescription { description }) => {
                assert_eq!(description, "Main project workspace");
            }
            other => panic!("expected SetContextDescription, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""type":"set_context_description""#),
            "wire tag missing: {serialised}"
        );

        let bad = r#"{"type":"set_context_description"}"#;
        assert!(
            serde_json::from_str::<DrawCommand>(bad).is_err(),
            "must fail without required description field"
        );
    }

    // ── feed-quality primitives wire shape (#1607) ────────────────────────
    // Pin wire contracts for TextWrappedMeasured, MeasureTextWrapped, Avatar,
    // Skeleton, and Text::max_lines.

    #[test]
    fn text_wrapped_measured_event_round_trips_serde() {
        let json = r#"{"type":"text_wrapped_measured","request_id":"req-wrap-1","height":48.5}"#;
        let event: PlexiEvent = serde_json::from_str(json).expect("deserialise");
        match &event {
            PlexiEvent::TextWrappedMeasured { request_id, height } => {
                assert_eq!(request_id, "req-wrap-1");
                assert!((height - 48.5).abs() < 0.001);
            }
            other => panic!("expected TextWrappedMeasured, got {other:?}"),
        }
        let serialised = serde_json::to_string(&event).expect("serialise");
        assert!(
            serialised.contains(r#""type":"text_wrapped_measured""#),
            "wire tag missing: {serialised}"
        );
        assert!(
            serialised.contains(r#""request_id":"req-wrap-1""#),
            "request_id missing: {serialised}"
        );
        assert!(
            serialised.contains(r#""height":"#),
            "height missing: {serialised}"
        );
    }

    #[test]
    fn text_wrapped_measured_missing_height_fails_deserialise() {
        let json = r#"{"type":"text_wrapped_measured","request_id":"req-wrap-2"}"#;
        let result: Result<PlexiEvent, _> = serde_json::from_str(json);
        assert!(result.is_err(), "must fail without required height field");
    }

    #[test]
    fn measure_text_wrapped_command_round_trips_serde() {
        let json = r#"{"type":"measure_text_wrapped","request_id":"req-mw-1","text":"hello world","font_size":14.0,"max_width":300.0,"max_lines":3}"#;
        let cmd: DrawCommand = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            DrawCommand::Control(ControlCommand::MeasureTextWrapped {
                request_id,
                text,
                font_size,
                max_width,
                max_lines,
            }) => {
                assert_eq!(request_id, "req-mw-1");
                assert_eq!(text, "hello world");
                assert!((font_size - 14.0).abs() < 0.001);
                assert!((max_width - 300.0).abs() < 0.001);
                assert_eq!(*max_lines, Some(3));
            }
            other => panic!("expected MeasureTextWrapped, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""type":"measure_text_wrapped""#),
            "wire tag missing: {serialised}"
        );
    }

    #[test]
    fn measure_text_wrapped_without_max_lines_round_trips_serde() {
        let json = r#"{"type":"measure_text_wrapped","request_id":"req-mw-2","text":"hi","font_size":12.0,"max_width":200.0}"#;
        let cmd: DrawCommand = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            DrawCommand::Control(ControlCommand::MeasureTextWrapped { max_lines, .. }) => {
                assert_eq!(*max_lines, None, "max_lines should be absent");
            }
            other => panic!("expected MeasureTextWrapped, got {other:?}"),
        }
    }

    #[test]
    fn measure_text_wrapped_missing_required_field_fails_deserialise() {
        // No `max_width` field — required, must fail.
        let json =
            r#"{"type":"measure_text_wrapped","request_id":"r","text":"hi","font_size":12.0}"#;
        let result: Result<DrawCommand, _> = serde_json::from_str(json);
        assert!(
            result.is_err(),
            "must fail without required max_width field"
        );
    }

    #[test]
    fn avatar_render_command_round_trips_serde() {
        let json = r#"{"type":"avatar","src":"abc-handle","cx":24.0,"cy":36.0,"radius":20.0}"#;
        let cmd: DrawCommand = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            DrawCommand::Render(RenderCommand::Avatar {
                src,
                cx,
                cy,
                radius,
            }) => {
                assert_eq!(src, "abc-handle");
                assert!((cx - 24.0).abs() < 0.001);
                assert!((cy - 36.0).abs() < 0.001);
                assert!((radius - 20.0).abs() < 0.001);
            }
            other => panic!("expected Avatar, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""type":"avatar""#),
            "wire tag missing: {serialised}"
        );
        assert!(
            serialised.contains(r#""src":"abc-handle""#),
            "src missing: {serialised}"
        );
    }

    #[test]
    fn avatar_missing_required_field_fails_deserialise() {
        // No `radius` — required, must fail.
        let json = r#"{"type":"avatar","src":"h","cx":0.0,"cy":0.0}"#;
        let result: Result<DrawCommand, _> = serde_json::from_str(json);
        assert!(result.is_err(), "must fail without required radius field");
    }

    #[test]
    fn skeleton_render_command_round_trips_serde() {
        let json = r#"{"type":"skeleton","x":10.0,"y":20.0,"w":200.0,"h":16.0}"#;
        let cmd: DrawCommand = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            DrawCommand::Render(RenderCommand::Skeleton { x, y, w, h, radius }) => {
                assert!((x - 10.0).abs() < 0.001);
                assert!((y - 20.0).abs() < 0.001);
                assert!((w - 200.0).abs() < 0.001);
                assert!((h - 16.0).abs() < 0.001);
                assert!((radius - 4.0).abs() < 0.001, "default radius should be 4.0");
            }
            other => panic!("expected Skeleton, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""type":"skeleton""#),
            "wire tag missing: {serialised}"
        );
    }

    #[test]
    fn skeleton_with_explicit_radius_round_trips_serde() {
        let json = r#"{"type":"skeleton","x":0.0,"y":0.0,"w":100.0,"h":8.0,"radius":8.0}"#;
        let cmd: DrawCommand = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            DrawCommand::Render(RenderCommand::Skeleton { radius, .. }) => {
                assert!((radius - 8.0).abs() < 0.001);
            }
            other => panic!("expected Skeleton, got {other:?}"),
        }
    }

    #[test]
    fn text_max_lines_present_round_trips_serde() {
        let json = r##"{"type":"text","x":0.0,"y":0.0,"text":"hello","size":14.0,"color":"#fff","monospace":false,"bold":false,"align":"top_left","max_width":null,"elide":false,"selectable":false,"max_lines":3}"##;
        let cmd: DrawCommand = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            DrawCommand::Render(RenderCommand::Text {
                max_lines, text, ..
            }) => {
                assert_eq!(*max_lines, Some(3));
                assert_eq!(text, "hello");
            }
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn text_max_lines_absent_deserialises_to_none() {
        // max_lines is #[serde(default)] so absence → None, not an error.
        let json = r##"{"type":"text","x":0.0,"y":0.0,"text":"hi","size":14.0,"color":"#fff","monospace":false,"bold":false,"align":"top_left","max_width":null,"elide":false,"selectable":false}"##;
        let cmd: DrawCommand = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            DrawCommand::Render(RenderCommand::Text { max_lines, .. }) => {
                assert_eq!(*max_lines, None, "absent max_lines should be None");
            }
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn list_view_command_round_trips_serde() {
        let json = r#"{"type":"list_view","id":"feed","x":0.0,"y":48.0,"w":0.0,"h":0.0,"items":[{"type":"row","id":"r1","primary":"Hello","secondary":null,"leading":{"variant":"none"},"chips":[],"trailing":null}],"selected":0,"loading":false,"error":null}"#;
        let cmd: DrawCommand = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            DrawCommand::Render(RenderCommand::ListView {
                id,
                selected,
                loading,
                error,
                items,
                ..
            }) => {
                assert_eq!(id, "feed");
                assert_eq!(*selected, 0);
                assert!(!loading);
                assert!(error.is_none());
                assert_eq!(items.len(), 1);
            }
            other => panic!("expected ListView, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""type":"list_view""#),
            "wire tag missing: {serialised}"
        );
        assert!(
            serialised.contains(r#""id":"feed""#),
            "id missing: {serialised}"
        );
    }

    #[test]
    fn list_view_loading_state_round_trips_serde() {
        let json = r#"{"type":"list_view","id":"issues","x":0.0,"y":0.0,"w":0.0,"h":0.0,"items":[],"selected":0,"loading":true,"error":null}"#;
        let cmd: DrawCommand = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            DrawCommand::Render(RenderCommand::ListView { loading, error, .. }) => {
                assert!(loading, "loading should be true");
                assert!(error.is_none());
            }
            other => panic!("expected ListView, got {other:?}"),
        }
    }

    #[test]
    fn list_view_error_state_round_trips_serde() {
        let json = r#"{"type":"list_view","id":"issues","x":0.0,"y":0.0,"w":0.0,"h":0.0,"items":[],"selected":0,"loading":false,"error":"network timeout"}"#;
        let cmd: DrawCommand = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            DrawCommand::Render(RenderCommand::ListView { error, .. }) => {
                assert_eq!(error.as_deref(), Some("network timeout"));
            }
            other => panic!("expected ListView, got {other:?}"),
        }
    }

    #[test]
    fn list_view_row_badge_leading_round_trips_serde() {
        let json = r##"{"type":"list_view","id":"issues","x":0.0,"y":0.0,"w":0.0,"h":0.0,"items":[{"type":"row","id":"r1","primary":"Fix bug","secondary":"subtitle","leading":{"variant":"badge","label":"#42","color":"accent"},"chips":[{"label":"P1","color":"red"}],"trailing":"3m ago"}],"selected":0,"loading":false,"error":null}"##;
        let cmd: DrawCommand = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            DrawCommand::Render(RenderCommand::ListView { items, .. }) => {
                assert_eq!(items.len(), 1);
                match &items[0] {
                    ListViewItem::Row(row) => {
                        assert_eq!(row.primary, "Fix bug");
                        assert_eq!(row.secondary.as_deref(), Some("subtitle"));
                        assert_eq!(row.chips.len(), 1);
                        assert_eq!(row.trailing.as_deref(), Some("3m ago"));
                        match &row.leading {
                            Some(ListViewLeading::Badge { label, color }) => {
                                assert_eq!(label, "#42");
                                assert_eq!(color, "accent");
                            }
                            other => panic!("expected Badge leading, got {other:?}"),
                        }
                    }
                    other => panic!("expected Row item, got {other:?}"),
                }
            }
            other => panic!("expected ListView, got {other:?}"),
        }
    }

    #[test]
    fn list_select_event_round_trips_serde() {
        let json = r#"{"type":"list_select","id":"feed","index":3}"#;
        let event: PlexiEvent = serde_json::from_str(json).expect("deserialise");
        match &event {
            PlexiEvent::ListSelect { id, index } => {
                assert_eq!(id, "feed");
                assert_eq!(*index, 3);
            }
            other => panic!("expected ListSelect, got {other:?}"),
        }
        let serialised = serde_json::to_string(&event).expect("serialise");
        assert!(
            serialised.contains(r#""type":"list_select""#),
            "wire tag missing: {serialised}"
        );
        assert!(
            serialised.contains(r#""index":3"#),
            "index missing: {serialised}"
        );
    }

    #[test]
    fn list_activate_event_round_trips_serde() {
        let json = r#"{"type":"list_activate","id":"issues","index":0}"#;
        let event: PlexiEvent = serde_json::from_str(json).expect("deserialise");
        match &event {
            PlexiEvent::ListActivate { id, index } => {
                assert_eq!(id, "issues");
                assert_eq!(*index, 0);
            }
            other => panic!("expected ListActivate, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod ui_node_tests {
    use super::*;
    use crate::protocol::events::PlexiEvent;

    #[test]
    fn stack_with_text_child_serializes_correctly() {
        let node = UiNode::Stack {
            direction: StackDirection::Vertical,
            children: vec![UiNode::Text {
                text: "hello".into(),
                size: 0.0,
                color: String::new(),
                bold: false,
                monospace: false,
            }],
            gap: 0.0,
            padding: UiPadding::default(),
        };
        let json = serde_json::to_string(&node).expect("serialize");
        assert!(
            json.contains(r#""type":"stack""#),
            "stack tag missing: {json}"
        );
        assert!(
            json.contains(r#""type":"text""#),
            "text tag missing: {json}"
        );
        assert!(
            json.contains(r#""text":"hello""#),
            "text content missing: {json}"
        );
    }

    #[test]
    fn draw_command_render_component_tree_deserializes() {
        let root = UiNode::Text {
            text: "hi".into(),
            size: 14.0,
            color: "#fff".into(),
            bold: false,
            monospace: false,
        };
        let root_json = serde_json::to_string(&root).expect("serialize root");
        let json = format!(r#"{{"type":"component_tree","root":{root_json}}}"#);
        let cmd: DrawCommand = serde_json::from_str(&json).expect("deserialize DrawCommand");
        match cmd {
            DrawCommand::Render(RenderCommand::ComponentTree { root: r }) => match r {
                UiNode::Text { text, .. } => assert_eq!(text, "hi"),
                other => panic!("expected Text root, got {other:?}"),
            },
            other => panic!("expected ComponentTree, got {other:?}"),
        }
    }

    #[test]
    fn component_event_serializes_correctly() {
        let event = PlexiEvent::ComponentEvent {
            node_id: "btn1".into(),
            event_type: "click".into(),
            payload: None,
        };
        let json = serde_json::to_string(&event).expect("serialize");
        assert!(
            json.contains(r#""type":"component_event""#),
            "event tag missing: {json}"
        );
        assert!(
            json.contains(r#""node_id":"btn1""#),
            "node_id missing: {json}"
        );
        assert!(
            json.contains(r#""event_type":"click""#),
            "event_type missing: {json}"
        );
        // payload:None should be omitted (skip_serializing_if)
        assert!(
            !json.contains("payload"),
            "payload should be absent: {json}"
        );

        // Some(payload) case — payload key and value must appear.
        let event_with_payload = PlexiEvent::ComponentEvent {
            node_id: "inp1".into(),
            event_type: "change".into(),
            payload: Some(serde_json::json!({"value": "hello"})),
        };
        let json2 = serde_json::to_string(&event_with_payload).unwrap();
        assert!(
            json2.contains(r#""payload""#),
            "payload Some case must include payload key: {json2}"
        );
        assert!(
            json2.contains(r#""value""#),
            "payload Some case must include value: {json2}"
        );
    }

    #[test]
    fn surface_node_round_trips_serde() {
        // Surface is reserved for the GPU surface layer (A2). Verify it serializes
        // and deserializes correctly so the variant is exercised and not silently dead.
        let node = UiNode::Surface {
            id: "canvas".into(),
        };
        let json = serde_json::to_string(&node).expect("serialize");
        assert!(json.contains(r#""type":"surface""#), "surface tag: {json}");
        assert!(json.contains(r#""id":"canvas""#), "id missing: {json}");
        let round_tripped: UiNode = serde_json::from_str(&json).expect("deserialize");
        match round_tripped {
            UiNode::Surface { id } => assert_eq!(id, "canvas"),
            other => panic!("expected Surface, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod ai_stream_chunk_tests {
    use super::*;
    use crate::protocol::events::PlexiEvent;

    /// AiStreamChunk round-trips through serde with all fields present.
    #[test]
    fn ai_stream_chunk_round_trips_serde() {
        let event = PlexiEvent::AiStreamChunk {
            request_id: "req-123".to_string(),
            delta: "Hello, ".to_string(),
            done: false,
        };
        let json = serde_json::to_string(&event).expect("serialize");
        assert!(
            json.contains(r#""type":"ai_stream_chunk""#),
            "type tag missing: {json}"
        );
        assert!(
            json.contains(r#""request_id":"req-123""#),
            "request_id missing: {json}"
        );
        assert!(
            json.contains(r#""delta":"Hello, ""#),
            "delta missing: {json}"
        );

        let round_tripped: PlexiEvent = serde_json::from_str(&json).expect("deserialize");
        match round_tripped {
            PlexiEvent::AiStreamChunk {
                request_id,
                delta,
                done,
            } => {
                assert_eq!(request_id, "req-123");
                assert_eq!(delta, "Hello, ");
                assert!(!done);
            }
            other => panic!("expected AiStreamChunk, got {other:?}"),
        }
    }

    /// done=true round-trips correctly.
    #[test]
    fn ai_stream_chunk_done_true_round_trips() {
        let event = PlexiEvent::AiStreamChunk {
            request_id: "req-456".to_string(),
            delta: String::new(),
            done: true,
        };
        let json = serde_json::to_string(&event).expect("serialize");
        let round_tripped: PlexiEvent = serde_json::from_str(&json).expect("deserialize");
        match round_tripped {
            PlexiEvent::AiStreamChunk { done, .. } => assert!(done, "done must be true"),
            other => panic!("expected AiStreamChunk, got {other:?}"),
        }
    }

    /// done defaults to false when absent from JSON (serde(default)).
    #[test]
    fn ai_stream_chunk_done_defaults_to_false() {
        let json = r#"{"type":"ai_stream_chunk","request_id":"r1","delta":"hi"}"#;
        let event: PlexiEvent = serde_json::from_str(json).expect("deserialize");
        match event {
            PlexiEvent::AiStreamChunk { done, delta, .. } => {
                assert!(!done, "done should default to false when absent");
                assert_eq!(delta, "hi");
            }
            other => panic!("expected AiStreamChunk, got {other:?}"),
        }
    }

    #[test]
    fn l1_app_bar_round_trips_serde() {
        let json = r#"{"type":"app_bar","title":"Todo","subtitle":"3 items"}"#;
        let node: UiNode = serde_json::from_str(json).expect("deserialize");
        assert_eq!(
            node,
            UiNode::AppBar {
                title: "Todo".into(),
                subtitle: "3 items".into()
            }
        );
    }

    #[test]
    fn l1_footer_keys_round_trips_serde() {
        let json = r#"{"type":"footer_keys","entries":[{"keys":["j","k"],"description":"scroll"}],"divider":true}"#;
        let node: UiNode = serde_json::from_str(json).expect("deserialize");
        match node {
            UiNode::FooterKeys { entries, divider } => {
                assert_eq!(entries.len(), 1);
                assert_eq!(entries[0].keys, vec!["j", "k"]);
                assert_eq!(entries[0].description, "scroll");
                assert!(divider);
            }
            other => panic!("expected FooterKeys, got {other:?}"),
        }
    }

    #[test]
    fn l1_select_list_round_trips_serde() {
        let json = r#"{"type":"select_list","items":[{"name":"Item 1"},{"name":"Item 2","description":"details"}],"selected_idx":1}"#;
        let node: UiNode = serde_json::from_str(json).expect("deserialize");
        match node {
            UiNode::SelectList {
                items,
                selected_idx,
            } => {
                assert_eq!(items.len(), 2);
                assert_eq!(items[0].name, "Item 1");
                assert_eq!(items[1].description, "details");
                assert_eq!(selected_idx, 1);
            }
            other => panic!("expected SelectList, got {other:?}"),
        }
    }

    #[test]
    fn l1_full_tree_from_sdk_deserializes() {
        let json = r#"{"type":"component_tree","root":{"type":"stack","direction":"vertical","children":[{"type":"app_bar","title":"Todo","subtitle":""},{"type":"section","title":"Items"},{"type":"select_list","items":[{"name":"Buy milk"}],"selected_idx":0},{"type":"spacer","size":12.0,"grow":false},{"type":"footer_keys","entries":[{"keys":["space"],"description":"toggle"}],"divider":true}],"gap":12.0,"padding":{"top":8.0,"right":24.0,"bottom":24.0,"left":24.0}}}"#;
        let cmd: DrawCommand = serde_json::from_str(json).expect("deserialize");
        match cmd {
            DrawCommand::Render(RenderCommand::ComponentTree { root }) => {
                if let UiNode::Stack { children, .. } = &root {
                    assert_eq!(children.len(), 5);
                } else {
                    panic!("expected Stack root, got {root:?}");
                }
            }
            other => panic!("expected ComponentTree, got {other:?}"),
        }
    }

    /// Wire-format round-trip for GetPreviousPaneInfo.
    #[test]
    fn get_previous_pane_info_round_trips_serde() {
        let json = r#"{"type":"get_previous_pane_info","response_file":"/tmp/prev.json"}"#;
        let req: AppRequest = serde_json::from_str(json).expect("deserialise");
        match &req {
            AppRequest::GetPreviousPaneInfo { response_file } => {
                assert_eq!(response_file, "/tmp/prev.json");
            }
            other => panic!("expected GetPreviousPaneInfo, got {other:?}"),
        }
        let serialised = serde_json::to_string(&req).expect("serialise");
        assert!(
            serialised.contains(r#""type":"get_previous_pane_info""#),
            "wire tag missing: {serialised}"
        );
        assert!(
            serialised.contains(r#""response_file":"/tmp/prev.json""#),
            "response_file missing: {serialised}"
        );
    }

    /// Missing response_file must fail deserialization.
    #[test]
    fn get_previous_pane_info_missing_response_file_fails() {
        let json = r#"{"type":"get_previous_pane_info"}"#;
        assert!(
            serde_json::from_str::<AppRequest>(json).is_err(),
            "missing required response_file must fail"
        );
    }
}
