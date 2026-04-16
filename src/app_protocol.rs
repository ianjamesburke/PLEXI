/// Plexi external app protocol — JSON lines over stdin/stdout.
///
/// # Overview
///
/// An external Plexi app is any executable that speaks this protocol.
/// Plexi spawns it as a subprocess and communicates via stdin/stdout.
/// Each message is a single JSON object on one line (newline-delimited JSON).
///
/// # Flow
///
/// 1. Plexi spawns the app binary.
/// 2. Plexi sends an `Init` event with initial dimensions.
/// 3. Each frame, Plexi sends a `Render` request; the app responds with
///    a sequence of `DrawCommand`s followed by `FrameDone`.
/// 4. Plexi forwards key/click/command events as they occur.
/// 5. On close, Plexi sends `Shutdown` and waits briefly for the process to exit.
///
/// # Example app (pseudocode)
///
/// ```
/// loop {
///   let event = read_json_line(stdin);
///   match event {
///     Init { width, height, .. } => { /* store dimensions */ }
///     Render => {
///       write_json(DrawCommand::Rect { x:0, y:0, w:width, h:height, fill:"#1e1e2e" });
///       write_json(DrawCommand::Text { x:20, y:20, text:"Hello Plexi!", size:14.0, color:"#cdd6f4" });
///       write_json(DrawCommand::FrameDone);
///     }
///     Key { key, .. } => { /* handle navigation */ }
///     _ => {}
///   }
/// }
/// ```
use serde::{Deserialize, Serialize};

/// The protocol version this host speaks. Sent in every `Init` event.
/// Apps that require features from a specific version compare this to
/// their declared minimum and exit cleanly if incompatible.
pub const HOST_PROTOCOL_VERSION: u32 = 2;

// ── Serde default helpers ─────────────────────────────────────────────────────

fn notification_default_urgency() -> String {
    "low".to_string()
}

fn notification_default_action_type() -> String {
    "dismiss".to_string()
}

// ── Events sent FROM Plexi TO the app ────────────────────────────────────────

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PlexiEvent {
    /// Sent once on startup with initial surface dimensions.
    Init {
        width: f32,
        height: f32,
        /// Logical pixels per point (display scale factor).
        pixels_per_point: f32,
        /// Protocol version spoken by the host. Apps should compare this
        /// against their declared minimum and exit if incompatible.
        /// `#[serde(default)]` ensures v1 apps (no field) deserialize to 0,
        /// which the host treats as "version 1" (legacy, deprecation warned).
        #[serde(default)]
        protocol_version: u32,
        /// Structured spawn intent.
        #[serde(default)]
        open_intent: Option<OpenIntent>,
        /// Optional capability manifest for nested instances. Hosts may leave
        /// this unset until capability enforcement is fully wired up.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        capability_manifest: Option<CapabilityManifest>,
    },
    /// Request a new frame. App should reply with DrawCommands + FrameDone.
    Render {
        width: f32,
        height: f32,
        delta_time: f32,
        #[serde(default)]
        mode: RenderMode,
    },
    /// Temporarily pause active work and render loops if possible.
    Suspend,
    /// Resume active work and render loops.
    Resume,
    /// Surface was resized.
    Resize { width: f32, height: f32 },
    /// A key was pressed.
    Key { key: String, modifiers: Modifiers },
    /// Mouse click at logical coordinates within the app surface.
    Click { x: f32, y: f32, button: MouseButton },
    /// Mouse button pressed (distinct from Click which fires on release).
    MouseDown { x: f32, y: f32, button: String },
    /// Mouse button released.
    MouseUp { x: f32, y: f32, button: String },
    /// Mouse moved over the app surface. Only sent when `mouse_tracking = true`
    /// in the app's manifest capabilities.
    MouseMove { x: f32, y: f32 },
    /// Scroll wheel / trackpad scroll over the app surface.
    Scroll {
        x: f32,
        y: f32,
        delta_x: f32,
        delta_y: f32,
    },
    /// User submitted a command via the terminal command bar.
    Command { text: String },
    /// Files were dropped on a registered drop target region.
    ///
    /// `target_id` matches the `id` of the `DropTarget` draw command that
    /// declared the region. Paths are absolute paths in the host filesystem.
    /// Paths that don't match the target's `accept` list are filtered out
    /// by Plexi before this event is sent.
    Drop {
        target_id: String,
        paths: Vec<String>,
    },
    /// Request the app's current state (for undo/redo/save).
    GetState,
    /// Restore app state from a previous snapshot.
    SetState {
        #[serde(default)]
        user_state: serde_json::Value,
        #[serde(default)]
        derived: serde_json::Value,
        #[serde(default)]
        session: serde_json::Value,
        #[serde(default)]
        persistent: serde_json::Value,
    },
    /// Response to a prior `DrawCommand::SecretGet` request. The SDK's
    /// `get_secret` helper reads stdin until it sees this event with a
    /// matching `name`, stashing any other events that arrive in the
    /// meantime so the main loop processes them on its next pass.
    ///
    /// `value` is `None` if the secret is missing or resolution failed.
    SecretResponse { name: String, value: Option<String> },
    /// A host event matching an active `DrawCommand::EventSubscribe` subscription.
    ///
    /// `kind` is the event's tag (e.g. `"app_spawned"`, `"pipe_write"`).
    /// `payload` is the full event JSON, minus the `source` field.
    EventData {
        kind: String,
        payload: serde_json::Value,
    },
    /// App is being closed. Process should exit.
    Shutdown,
    /// A Run was created; the run_id is returned to the requesting app.
    RunCreated { run_id: String },
    /// A value arrived on a named pipe channel from another app.
    ///
    /// Sent by the host when a connected app (parent or child via spawn
    /// relationship) emits a `DrawCommand::PipeWrite` on the named channel.
    PipeData {
        /// The `app_id` (type_id) of the sending app.
        from_app: String,
        /// The channel name the value was written to.
        channel: String,
        /// The JSON value written by the sender.
        value: serde_json::Value,
    },
    /// Response to a MeasureText request.
    TextMetrics {
        request_id: u32,
        width: f32,
        height: f32,
        ascent: f32,
    },
}

fn default_protocol_version() -> u32 {
    2
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub cmd: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case")]
pub enum MouseButton {
    Primary,
    Secondary,
}

// ── OpenIntent ────────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct OpenIntent {
    pub kind: OpenKind,
    #[serde(default)]
    pub caller: Option<Caller>,
    #[serde(default)]
    pub payload: Option<serde_json::Value>,
    #[serde(default)]
    pub run_id: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OpenKind {
    File {
        path: String,
        #[serde(default)]
        range: Option<TextRange>,
    },
    Url { url: String },
    Prompt {
        text: String,
        #[serde(default)]
        model_hint: Option<String>,
    },
    Resume { snapshot_key: String },
    Bare,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct TextRange {
    pub start_line: u32,
    pub end_line: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Caller {
    pub app_id: String,
    #[serde(default)]
    pub pane_id: Option<u64>,
    pub source: CallerSource,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "snake_case")]
pub enum CallerSource {
    #[default]
    Palette,
    Cli,
    Spawn,
    Notification,
    AgentMode,
    ApiCall,
}

// ── Run primitive ─────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Run {
    pub id: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub status: RunStatus,
    pub head_task: String,
    pub initiator: Caller,
    pub scope: RunScope,
    #[serde(default)]
    pub notification_id: Option<String>,
    #[serde(default)]
    pub parent_run_id: Option<String>,
    #[serde(default)]
    pub payload: serde_json::Value,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RunStatus {
    Pending,
    Running,
    BlockedOnUser {
        prompt: String,
        resume_intent: OpenIntent,
    },
    BlockedOnChild {
        child_run_id: String,
    },
    Complete,
    Failed { error: String },
    Cancelled,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum RunOutcome {
    Success,
    Failed { error: String },
    Cancelled,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "snake_case")]
pub enum RunScope {
    #[default]
    Global,
    Workspace { path: String },
}

// ── Notification actions ──────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NotificationAction {
    Focus {
        pane_id: u64,
        fullscreen: bool,
    },
    Confirm {
        confirm_text: String,
        cancel_text: String,
        on_confirm: Box<NotificationAction>,
    },
    TextInput {
        prompt: String,
        #[serde(default)]
        placeholder: Option<String>,
        on_submit: Box<NotificationAction>,
    },
    Dismiss,
    ResumeRun {
        run_id: String,
    },
    OpenIntent {
        open_intent: OpenIntent,
    },
    RunCommand {
        app_id: String,
        command: String,
        args: Vec<String>,
    },
    ExternalUrl {
        url: String,
    },
}

// ── Event bus ─────────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BusEvent {
    pub id: u64,
    pub ts: i64,
    pub scope: RunScope,
    pub kind: BusEventKind,
    #[serde(default)]
    pub caller: Option<Caller>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum BusEventKind {
    AppSpawned {
        #[serde(default)]
        parent_pane: Option<u64>,
        child_pane: u64,
        app_id: String,
        #[serde(default)]
        open_intent: Option<OpenIntent>,
    },
    AppClosed {
        pane: u64,
        app_id: String,
        reason: String,
    },
    PipeWrite {
        from: u64,
        channel: String,
        bytes: u32,
    },
    NotificationEmitted {
        id: String,
        app_id: String,
        urgency: String,
        #[serde(default)]
        run_id: Option<String>,
    },
    NotificationActioned {
        id: String,
        action: String,
    },
    ApiCall {
        app_id: String,
        method: String,
        ok: bool,
    },
    AgentTurn {
        agent_id: String,
        #[serde(default)]
        run_id: Option<String>,
        tokens_in: u32,
        tokens_out: u32,
        model: String,
    },
    RunCreated {
        run_id: String,
        head_task: String,
        initiator: Caller,
    },
    RunUpdated {
        run_id: String,
        status_tag: String,
        head_task: String,
    },
    RunCompleted {
        run_id: String,
        outcome: RunOutcome,
    },
    PermissionPrompted {
        app_id: String,
        capability: String,
        decision: String,
    },
    CostReported {
        app_id: String,
        usd: f64,
        model: String,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(rename_all = "snake_case")]
pub enum SubscribeScope {
    #[default]
    Workspace,
    Pane,
    Group,
}
/// How a render request should behave.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum RenderMode {
    #[default]
    Full,
    Preview,
}

/// Current health for a pane or depth summary.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Health {
    #[default]
    Running,
    Idle,
    Error,
}

/// Summary for one pane in a depth.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct PaneSummary {
    pub pane_id: u64,
    pub cwd: String,
    #[serde(default)]
    pub status_text: Option<String>,
    #[serde(default)]
    pub last_activity_unix_ms: Option<i64>,
    #[serde(default)]
    pub health: Health,
}

/// Lightweight render/status metadata used for preview summaries and rollups.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct StatusSummary {
    pub uptime_seconds: f64,
    pub process_count: u32,
    #[serde(default)]
    pub last_activity_unix_ms: Option<i64>,
    #[serde(default)]
    pub summary_text: Option<String>,
    #[serde(default)]
    pub health: Health,
    #[serde(default)]
    pub panes: Vec<PaneSummary>,
}

/// Capability allowlist passed to nested instances via `Init`.
///
/// This is intentionally additive-only at the wire layer. Enforcement stays
/// host-side in the process/app/api path once the rollout is ready.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
pub struct CapabilityManifest {
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub secrets: Vec<String>,
    #[serde(default)]
    pub network: Vec<String>,
    #[serde(default)]
    pub fs_read: Vec<String>,
    #[serde(default)]
    pub fs_write: Vec<String>,
    #[serde(default)]
    pub hardware: Vec<String>,
    #[serde(default)]
    pub spawn: Vec<String>,
    #[serde(default)]
    pub pipe: Vec<String>,
    #[serde(default)]
    pub ttl_seconds: Option<u64>,
}

// ── Commands sent FROM the app TO Plexi ──────────────────────────────────────

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DrawCommand {
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
    /// `align` controls horizontal anchoring of the text relative to `x`:
    /// - `"left"` (default) — `x` is the left edge of the text
    /// - `"center"` — `x` is the horizontal center
    /// - `"right"` — `x` is the right edge of the text
    /// Vertical anchoring is always top (`y` = top of text cell).
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
        #[serde(default)]
        align: Option<String>,
    },
    /// Draw a horizontal line.
    Line {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        color: String,
        #[serde(default = "default_stroke_width")]
        width: f32,
    },
    /// High-level scrollable list — Plexi handles layout and scrolling.
    List {
        items: Vec<ListItem>,
        selected: usize,
        #[serde(default)]
        item_height: f32,
    },
    /// Draw an image from a file on disk. Plexi decodes and caches the texture.
    ///
    /// `path` may be absolute or relative to the app's cwd. `fit` controls how
    /// the source image is placed inside the `w`×`h` rect:
    ///   - "contain" (default): fit inside the rect, preserve aspect, may letterbox.
    ///   - "cover":             fill the rect, preserve aspect, crop overflow.
    ///   - "fill":              stretch to the rect, ignoring aspect ratio.
    ///
    /// If decoding fails, Plexi draws a placeholder rect with an error marker.
    Image {
        path: String,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        #[serde(default)]
        fit: Option<String>,
        #[serde(default)]
        rounding: Option<f32>,
    },
    /// Draw a thumbnail for a video file. Plexi extracts a single frame via
    /// ffmpeg and caches the result under ~/.cache/plexi/thumbnails/.
    ///
    /// Extraction happens on a worker thread; the first render after a cache
    /// miss shows a "loading" placeholder and triggers a repaint when ready.
    ///
    /// If `show_play_button` is true (default), a centered play triangle is
    /// overlaid on the thumbnail. Clicking the rect opens the video in the
    /// system default player (`open` on macOS).
    VideoThumbnail {
        path: String,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        #[serde(default)]
        show_play_button: Option<bool>,
        #[serde(default)]
        timestamp_seconds: Option<f32>,
    },
    /// Draw a grid of files with thumbnails. Two modes:
    ///   - `path` + optional `filter`: walk the directory (non-recursive),
    ///     filter by glob patterns or extensions.
    ///   - `paths`: explicit list of file paths, rendered in order.
    ///
    /// Image files render via the image texture cache; video files
    /// (mp4/mov/webm/mkv) render via the video thumbnail cache. Other file
    /// types show a generic icon with the extension label.
    FileGrid {
        #[serde(default)]
        path: Option<String>,
        #[serde(default)]
        filter: Option<Vec<String>>,
        #[serde(default)]
        paths: Option<Vec<String>>,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        #[serde(default)]
        item_size: Option<f32>,
        #[serde(default)]
        columns: Option<u32>,
        #[serde(default)]
        show_labels: Option<bool>,
    },
    /// Emit a command back to Plexi to run in the linked terminal.
    RunInTerminal { command: String },
    /// Tell Plexi to cd the linked terminal to this path.
    Cd { path: String },
    /// Forward a log message to Plexi's logger. Tagged with the app's id.
    Log {
        /// One of: "error" | "warn" | "info" | "debug"
        level: String,
        message: String,
    },
    /// App's state snapshot (response to GetState).
    State {
        #[serde(default)]
        user_state: serde_json::Value,
        #[serde(default)]
        derived: serde_json::Value,
        #[serde(default)]
        session: serde_json::Value,
        #[serde(default)]
        persistent: serde_json::Value,
    },
    /// Report LLM API costs for logging.
    CostReport {
        app_id: String,
        service: String,
        model: String,
        input_tokens: u64,
        output_tokens: u64,
        cost_usd: f64,
        #[serde(default)]
        operation_id: Option<String>,
        #[serde(default)]
        timestamp: Option<String>,
    },
    /// Declare a drop target region. Stateless per frame: apps must re-emit
    /// this on every render frame for the drop zone to remain active.
    ///
    /// When the user drags files from outside Plexi over this region, Plexi
    /// will draw a subtle highlight and the optional `label`. When files are
    /// dropped, Plexi filters paths by extension against `accept` (empty =
    /// accept anything) and sends a `PlexiEvent::Drop` to the app with the
    /// matching `id`.
    DropTarget {
        id: String,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        /// File extensions (lowercase, no dot) or MIME types to accept.
        /// Empty list means accept anything.
        #[serde(default)]
        accept: Vec<String>,
        /// Optional hint text shown over the target while hovering with files.
        #[serde(default)]
        label: Option<String>,
    },
    /// Set the cursor icon for the app pane.
    ///
    /// Values: `"default"`, `"pointer"`, `"grab"`, `"grabbing"`, `"crosshair"`, `"text"`.
    /// The cursor reverts to `"default"` at the start of each frame; apps must
    /// re-emit this on every render frame where they want a non-default cursor.
    SetCursor { cursor: String },
    /// Enable or disable mouse-move event delivery. When disabled (default),
    /// Plexi does not send `MouseMove` events to avoid flooding the pipe.
    /// This command is stateful — the setting persists until changed.
    MouseTracking { enabled: bool },
    /// Summary-only metadata for a parent depth preview or tree rollup.
    StatusSummary { summary: StatusSummary },
    /// Ask Plexi to spawn another app and place it in a layout slot relative
    /// to the emitting pane. This is the composition primitive: a file
    /// browser pressing Enter on a .txt file emits `spawn_app` to bring up
    /// a text editor in a 50/50 right split, lifecycle-bonded to itself.
    ///
    /// Field semantics:
    /// - `app_id`       — id of the app to spawn (must exist in the registry).
    /// - `args`         — command-line args forwarded to the child as argv[1..].
    /// - `parent`       — anchor for the new pane: the spawner (`self`), the
    ///                    top-level root, or a named layout mark (reserved).
    /// - `layout`       — how to position the new pane relative to the parent.
    /// - `lifecycle`    — what happens to the child when the parent closes
    ///                    (`cascade` closes together, `orphan` detaches,
    ///                    `prompt` asks the user — stubbed to `orphan` in v1).
    /// - `linked`       — join the parent's linked-pane group. When true the
    ///                    child shares terminal-linking with the parent.
    /// - `wire_channels`— pre-wire typed-pipe channel names. Stored on the
    ///                    spawn relationship for the typed-pipes spec to
    ///                    consume; not emitted over the wire in v1.
    ///
    /// Authorization: the host checks the target app's `[app.spawnable]`
    /// manifest table. If `allow_callers` doesn't match the spawner's id, or
    /// `allow_lifecycle` doesn't include the requested lifecycle, the spawn
    /// is refused and a notification is delivered to the caller.
    SpawnApp {
        app_id: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        parent: SpawnParent,
        #[serde(default)]
        layout: SpawnLayout,
        #[serde(default)]
        lifecycle: SpawnLifecycle,
        #[serde(default = "default_spawn_linked")]
        linked: bool,
        #[serde(default)]
        wire_channels: Vec<String>,
    },
    /// Subscribe to a named input channel.
    ///
    /// **Phase 0 no-op:** this command is accepted for forward compatibility
    /// with Phase 1 manifest wiring but performs no action in Phase 0. Apps
    /// may re-emit it each frame; the host silently discards it.
    PipeSubscribe { channel: String },
    /// Request a secret by name. Plexi resolves against the Keychain, walking
    /// up from the app's launch directory to home, and sends the result back
    /// as a `PlexiEvent::SecretResponse` with the same `name`.
    ///
    /// Missing secrets and resolution failures both return `value: None` — the
    /// SDK should not crash the app on a failed lookup.
    SecretGet { name: String },
    /// Subscribe to host events. Matching events will be forwarded to the app
    /// as `PlexiEvent::EventData`.
    ///
    /// `kinds` — list of event kind strings to subscribe to (e.g. `"app_spawned"`,
    /// `"pipe_write"`). An empty list means subscribe to all events.
    /// `scope` — one of `"workspace"`, `"pane"`, or `"global"`.
    ///
    /// The subscription persists until the app is closed or a new
    /// `EventSubscribe` replaces it. Emitting this command with an empty
    /// `kinds` list and scope `"global"` subscribes to all host events.
    ///
    /// **Phase 0 implementation note:** subscription tracking and event
    /// forwarding are stubbed. The command is accepted for forward compatibility
    /// but the host does not yet deliver `EventData` events to the app's stdin.
    /// Full delivery will land in a follow-up PR.
    EventSubscribe {
        /// Empty = all event kinds.
        #[serde(default)]
        kinds: Vec<String>,
        #[serde(default)]
        scope: SubscribeScope,
    },
    /// End of frame — Plexi will render everything queued since last FrameDone.
    FrameDone,
    /// Emit a notification. Plexi records it to the notification log,
    /// increments the status-bar unread count, and surfaces it in the
    /// notification palette (Cmd+Shift+N).
    ///
    /// Uses structured `NotificationAction` for the action field (v2.0+).
    /// `source_app` is the `app_id` of the emitter.
    Notification {
        id: String,
        title: String,
        #[serde(default)]
        body: Option<String>,
        /// The `app_id` of the emitter (e.g. `"parallax"`).
        #[serde(default)]
        source_app: Option<String>,
        /// Urgency level: "low" | "medium" | "high". Defaults to "low".
        #[serde(default = "notification_default_urgency")]
        urgency: String,
        /// Unix timestamp (seconds). Host drops the notification if this is
        /// already past at ingestion time.
        #[serde(default)]
        expires_at: Option<i64>,
        /// Unix timestamp (seconds). Defer rendering in the palette until then.
        #[serde(default)]
        visible_after: Option<i64>,
        #[serde(default)]
        run_id: Option<String>,
        /// Structured action triggered when the user interacts with this notification.
        #[serde(default)]
        action: Option<NotificationAction>,
    },
    /// Write data to a named pipe channel.
    ///
    /// The host routes this to all connected apps: if the emitter is a child
    /// pane, its parent app receives a `PlexiEvent::PipeData`; if the emitter
    /// is a parent pane, all its child apps receive `PlexiEvent::PipeData`.
    /// Routing is bidirectional — parent→children and child→parent.
    PipeWrite {
        /// Channel name (e.g. "selection", "result", "data").
        channel: String,
        /// Any JSON-representable value.
        value: serde_json::Value,
    },
    /// Create a Run.
    RunCreate {
        head_task: String,
        #[serde(default)]
        payload: serde_json::Value,
        #[serde(default)]
        parent_run_id: Option<String>,
        #[serde(default)]
        notification_title: Option<String>,
    },
    /// Update a Run's status.
    RunUpdate {
        run_id: String,
        status: RunStatus,
        #[serde(default)]
        head_task: Option<String>,
        #[serde(default)]
        payload: Option<serde_json::Value>,
    },
    /// Complete a Run.
    RunComplete {
        run_id: String,
        outcome: RunOutcome,
    },
    /// List active pipe wires.
    PipeListWires,
    /// Push a transform onto the transform stack.
    PushTransform {
        #[serde(default = "default_one_f32")]
        scale_x: f32,
        #[serde(default = "default_one_f32")]
        scale_y: f32,
        #[serde(default)]
        translate_x: f32,
        #[serde(default)]
        translate_y: f32,
        #[serde(default)]
        rotate: f32,
        #[serde(default)]
        origin_x: f32,
        #[serde(default)]
        origin_y: f32,
    },
    /// Pop the top transform off the stack.
    PopTransform,
    /// Request exact text measurement. Plexi responds with a TextMetrics event.
    MeasureText {
        request_id: u32,
        text: String,
        size: f32,
        #[serde(default)]
        monospace: bool,
        #[serde(default)]
        bold: bool,
    },
}

/// Anchor for a `SpawnApp` layout — where the new pane is positioned relative
/// to. Serializes as a bare string on the wire for ergonomics:
///
/// - `"self"` (default) — the pane that emitted the spawn.
/// - `"root"`           — top-level (ignore the emitter's location).
/// - `"mark:<name>"`    — reserved for a future named-layout system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpawnParent {
    SelfPane,
    Root,
    NamedMark(String),
}

impl Default for SpawnParent {
    fn default() -> Self {
        SpawnParent::SelfPane
    }
}

impl Serialize for SpawnParent {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        match self {
            SpawnParent::SelfPane => ser.serialize_str("self"),
            SpawnParent::Root => ser.serialize_str("root"),
            SpawnParent::NamedMark(name) => ser.serialize_str(&format!("mark:{name}")),
        }
    }
}

impl<'de> Deserialize<'de> for SpawnParent {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(de)?;
        match raw.as_str() {
            "self" | "self_pane" => Ok(SpawnParent::SelfPane),
            "root" => Ok(SpawnParent::Root),
            other if other.starts_with("mark:") => {
                Ok(SpawnParent::NamedMark(other[5..].to_string()))
            }
            other => Err(serde::de::Error::custom(format!(
                "invalid spawn parent: {other:?} (expected \"self\", \"root\", or \"mark:<name>\")"
            ))),
        }
    }
}

/// How a `SpawnApp` call positions the new pane relative to its parent.
///
/// - `Fill`        — replace or cover the parent slot (default).
/// - `Cols`        — split horizontally; `slot=0` is left, `slot=1` is right;
///                   `ratio` is the fraction allocated to `slot`.
/// - `Rows`        — split vertically; `slot=0` is top, `slot=1` is bottom.
/// - `Grid2x2`     — 2×2 grid; `slot` in 0..4 (row-major). Reserved — the v1
///                   host falls back to Fill and logs a warning.
/// - `Custom`      — arbitrary layout spec forwarded through as JSON. Stored
///                   for forward-compat; v1 treats it as Fill.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SpawnLayout {
    Fill,
    Cols {
        slot: u8,
        #[serde(default = "default_layout_ratio")]
        ratio: f32,
    },
    Rows {
        slot: u8,
        #[serde(default = "default_layout_ratio")]
        ratio: f32,
    },
    Grid2x2 {
        slot: u8,
    },
    Custom {
        spec: serde_json::Value,
    },
}

impl Default for SpawnLayout {
    fn default() -> Self {
        SpawnLayout::Fill
    }
}

/// What happens to a spawned child when its parent closes.
///
/// - `Cascade` — the child is closed together with the parent (default).
///               Cascades recursively: a child that owns its own grandchildren
///               closes them too.
/// - `Orphan`  — the relationship is dropped and the child stays alive as a
///               top-level pane.
/// - `Prompt`  — v1 stub: falls back to `Orphan` with a warning. Reserved
///               for a future interactive confirmation.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SpawnLifecycle {
    Cascade,
    Orphan,
    Prompt,
}

impl Default for SpawnLifecycle {
    fn default() -> Self {
        SpawnLifecycle::Cascade
    }
}

fn default_spawn_linked() -> bool {
    true
}
fn default_layout_ratio() -> f32 {
    0.5
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ListItem {
    pub label: String,
    #[serde(default)]
    pub secondary: Option<String>,
    #[serde(default)]
    pub icon: Option<String>, // reserved for future use
    #[serde(default)]
    pub is_dir: bool,
}

fn default_stroke_width() -> f32 {
    1.0
}

fn default_one_f32() -> f32 {
    1.0
}

/// A wired connection between two app panes on a named channel.
#[derive(Debug, Clone)]
pub struct PipeWire {
    pub from_pane: u64,
    pub to_pane: u64,
    pub channel: String,
}

// ── Spawn-relationship host bookkeeping ──────────────────────────────────────

/// Record of one parent → child pane spawn, produced when the host honors a
/// `DrawCommand::SpawnApp`. Used to drive cascade/orphan/prompt semantics when
/// a pane closes. Not serialized — purely an in-memory registry.
///
/// Consumed by the spawn dispatcher (to be added on `PlexiApp`) — `#[allow
/// (dead_code)]` is kept until that wiring lands in a follow-up commit so the
/// types live in the crate without polluting the warning baseline.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct SpawnRelationship {
    pub parent_pane: crate::tiling::PaneId,
    pub child_pane: crate::tiling::PaneId,
    pub lifecycle: SpawnLifecycle,
    /// Typed-pipe channel names the caller asked to pre-wire. The linking
    /// matrix is a future spec; v1 just stores the list.
    pub wire_channels: Vec<String>,
}

/// In-memory registry of every active spawn relationship. Owned by the host
/// (added to `PlexiApp` when the spawn dispatcher lands).
#[allow(dead_code)]
#[derive(Debug, Default)]
pub struct SpawnRelationships {
    relations: Vec<SpawnRelationship>,
}

#[allow(dead_code)]
impl SpawnRelationships {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a new parent → child relationship.
    pub fn add(
        &mut self,
        parent_pane: crate::tiling::PaneId,
        child_pane: crate::tiling::PaneId,
        lifecycle: SpawnLifecycle,
        wire_channels: Vec<String>,
    ) {
        self.relations.push(SpawnRelationship {
            parent_pane,
            child_pane,
            lifecycle,
            wire_channels,
        });
    }

    /// All direct children of `parent`.
    pub fn children_of(&self, parent: crate::tiling::PaneId) -> Vec<&SpawnRelationship> {
        self.relations
            .iter()
            .filter(|r| r.parent_pane == parent)
            .collect()
    }

    /// Look up the parent pane (if any) of a child.
    pub fn parent_of(&self, child: crate::tiling::PaneId) -> Option<crate::tiling::PaneId> {
        self.relations
            .iter()
            .find(|r| r.child_pane == child)
            .map(|r| r.parent_pane)
    }

    /// Remove all relationships mentioning `pane` (as parent OR child). Called
    /// when a pane is actually closed so the table stays tight.
    pub fn remove_pane(&mut self, pane: crate::tiling::PaneId) {
        self.relations
            .retain(|r| r.parent_pane != pane && r.child_pane != pane);
    }

    /// Total number of active relationships (for tests / telemetry).
    pub fn len(&self) -> usize {
        self.relations.len()
    }

    pub fn is_empty(&self) -> bool {
        self.relations.is_empty()
    }
}

/// A spawn request drained from a `ProcessApp` between frames. The host
/// dispatcher walks this list, validates each request against the registry
/// and target app's manifest, and turns it into a real pane + child process.
#[derive(Debug, Clone)]
pub struct PendingSpawn {
    pub app_id: String,
    pub args: Vec<String>,
    pub parent: SpawnParent,
    pub layout: SpawnLayout,
    pub lifecycle: SpawnLifecycle,
    pub linked: bool,
    pub wire_channels: Vec<String>,
}

#[cfg(test)]
mod spawn_tests {
    use super::*;

    #[test]
    fn spawn_app_roundtrip_minimal() {
        let json = r#"{"type":"spawn_app","app_id":"text-editor"}"#;
        let cmd: DrawCommand = serde_json::from_str(json).unwrap();
        match cmd {
            DrawCommand::SpawnApp {
                app_id,
                args,
                parent,
                layout,
                lifecycle,
                linked,
                wire_channels,
            } => {
                assert_eq!(app_id, "text-editor");
                assert!(args.is_empty());
                assert_eq!(parent, SpawnParent::SelfPane);
                assert_eq!(layout, SpawnLayout::Fill);
                assert_eq!(lifecycle, SpawnLifecycle::Cascade);
                assert!(linked);
                assert!(wire_channels.is_empty());
            }
            _ => panic!("expected SpawnApp"),
        }
    }

    #[test]
    fn spawn_app_roundtrip_cols_split() {
        let json = r#"{
            "type": "spawn_app",
            "app_id": "text-editor",
            "args": ["/tmp/notes.txt"],
            "parent": "self",
            "layout": { "kind": "cols", "slot": 1, "ratio": 0.5 },
            "lifecycle": "cascade",
            "linked": true,
            "wire_channels": ["file_buffer"]
        }"#;
        let cmd: DrawCommand = serde_json::from_str(json).unwrap();
        let re = serde_json::to_value(&cmd).unwrap();
        assert_eq!(re["type"], "spawn_app");
        assert_eq!(re["app_id"], "text-editor");
        assert_eq!(re["args"][0], "/tmp/notes.txt");
        assert_eq!(re["parent"], "self");
        assert_eq!(re["layout"]["kind"], "cols");
        assert_eq!(re["layout"]["slot"], 1);
        assert_eq!(re["layout"]["ratio"], 0.5);
        assert_eq!(re["lifecycle"], "cascade");
        assert_eq!(re["linked"], true);
        assert_eq!(re["wire_channels"][0], "file_buffer");
    }

    #[test]
    fn spawn_app_roundtrip_root_fill() {
        let json = r#"{
            "type": "spawn_app",
            "app_id": "permissions",
            "parent": "root",
            "layout": { "kind": "fill" },
            "lifecycle": "orphan",
            "linked": false
        }"#;
        let cmd: DrawCommand = serde_json::from_str(json).unwrap();
        match cmd {
            DrawCommand::SpawnApp {
                parent,
                layout,
                lifecycle,
                linked,
                ..
            } => {
                assert_eq!(parent, SpawnParent::Root);
                assert!(matches!(layout, SpawnLayout::Fill));
                assert_eq!(lifecycle, SpawnLifecycle::Orphan);
                assert!(!linked);
            }
            _ => panic!("expected SpawnApp"),
        }
    }

    #[test]
    fn spawn_app_rows_with_ratio_default() {
        let json = r#"{
            "type": "spawn_app",
            "app_id": "log-viewer",
            "layout": { "kind": "rows", "slot": 1 }
        }"#;
        let cmd: DrawCommand = serde_json::from_str(json).unwrap();
        match cmd {
            DrawCommand::SpawnApp { layout, .. } => match layout {
                SpawnLayout::Rows { slot, ratio } => {
                    assert_eq!(slot, 1);
                    assert_eq!(ratio, 0.5);
                }
                _ => panic!("expected Rows"),
            },
            _ => panic!("expected SpawnApp"),
        }
    }

    #[test]
    fn spawn_parent_named_mark_roundtrip() {
        let v: SpawnParent = serde_json::from_str(r#""mark:file-tree""#).unwrap();
        assert_eq!(v, SpawnParent::NamedMark("file-tree".to_string()));
        let s = serde_json::to_string(&v).unwrap();
        assert_eq!(s, r#""mark:file-tree""#);
    }

    #[test]
    fn spawn_relationships_cascade_children_lookup() {
        let mut rels = SpawnRelationships::new();
        rels.add(1, 2, SpawnLifecycle::Cascade, vec![]);
        rels.add(1, 3, SpawnLifecycle::Orphan, vec![]);
        rels.add(2, 4, SpawnLifecycle::Cascade, vec![]);
        assert_eq!(rels.children_of(1).len(), 2);
        assert_eq!(rels.children_of(2).len(), 1);
        assert_eq!(rels.parent_of(4), Some(2));
        rels.remove_pane(2);
        // pane 2 gone → its children lose their parent edge, and pane 2 as a
        // child of pane 1 is also gone.
        assert_eq!(rels.children_of(1).len(), 1);
        assert_eq!(rels.children_of(2).len(), 0);
        assert_eq!(rels.parent_of(4), None);
    }
}

#[cfg(test)]
mod protocol_tests {
    use super::*;

    #[test]
    fn render_defaults_mode_to_full_for_old_json() {
        let json = r#"{"type":"render","width":120,"height":40,"delta_time":0.016}"#;
        let event: PlexiEvent = serde_json::from_str(json).unwrap();
        match event {
            PlexiEvent::Render {
                mode,
                width,
                height,
                delta_time,
            } => {
                assert_eq!(mode, RenderMode::Full);
                assert_eq!(width, 120.0);
                assert_eq!(height, 40.0);
                assert_eq!(delta_time, 0.016);
            }
            other => panic!("expected render, got {other:?}"),
        }
    }

    #[test]
    fn init_defaults_capability_manifest_to_none_for_old_json() {
        let json = r#"{
            "type":"init",
            "width":800,
            "height":600,
            "pixels_per_point":2,
            "protocol_version":2
        }"#;
        let event: PlexiEvent = serde_json::from_str(json).unwrap();
        match event {
            PlexiEvent::Init {
                capability_manifest,
                protocol_version,
                ..
            } => {
                assert_eq!(protocol_version, 2);
                assert!(capability_manifest.is_none());
            }
            other => panic!("expected init, got {other:?}"),
        }
    }

    #[test]
    fn capability_manifest_roundtrips() {
        let manifest = CapabilityManifest {
            cwd: Some("/project/agents/scraper".to_string()),
            secrets: vec!["GITHUB_TOKEN".to_string()],
            network: vec!["api.github.com".to_string()],
            fs_read: vec!["/project/data".to_string()],
            fs_write: vec!["/project/output".to_string()],
            hardware: vec!["microphone".to_string()],
            spawn: vec!["text-editor".to_string()],
            pipe: vec!["events".to_string()],
            ttl_seconds: Some(3600),
        };

        let event = PlexiEvent::Init {
            width: 800.0,
            height: 600.0,
            pixels_per_point: 2.0,
            protocol_version: 2,
            capability_manifest: Some(manifest.clone()),
        };

        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(
            json["capability_manifest"]["cwd"],
            "/project/agents/scraper"
        );
        assert_eq!(json["capability_manifest"]["ttl_seconds"], 3600);

        let de: PlexiEvent = serde_json::from_value(json).unwrap();
        match de {
            PlexiEvent::Init {
                capability_manifest: Some(roundtrip),
                ..
            } => assert_eq!(roundtrip, manifest),
            other => panic!("expected init with capability manifest, got {other:?}"),
        }
    }

    #[test]
    fn existing_events_still_deserialize() {
        let key: PlexiEvent = serde_json::from_str(
            r#"{"type":"key","key":"enter","modifiers":{"shift":false,"ctrl":true,"alt":false,"cmd":false}}"#,
        )
        .unwrap();
        assert!(matches!(
            key,
            PlexiEvent::Key { key, modifiers } if key == "enter" && modifiers.ctrl
        ));

        let shutdown: PlexiEvent = serde_json::from_str(r#"{"type":"shutdown"}"#).unwrap();
        assert!(matches!(shutdown, PlexiEvent::Shutdown));

        let suspend: PlexiEvent = serde_json::from_str(r#"{"type":"suspend"}"#).unwrap();
        assert!(matches!(suspend, PlexiEvent::Suspend));

        let resume: PlexiEvent = serde_json::from_str(r#"{"type":"resume"}"#).unwrap();
        assert!(matches!(resume, PlexiEvent::Resume));
    }

    #[test]
    fn status_summary_roundtrips() {
        let summary = StatusSummary {
            uptime_seconds: 12.5,
            process_count: 3,
            last_activity_unix_ms: Some(1234),
            summary_text: Some("idle".to_string()),
            health: Health::Idle,
            panes: vec![PaneSummary {
                pane_id: 7,
                cwd: "/tmp".to_string(),
                status_text: Some("waiting".to_string()),
                last_activity_unix_ms: None,
                health: Health::Running,
            }],
        };

        let cmd = DrawCommand::StatusSummary {
            summary: summary.clone(),
        };
        let json = serde_json::to_value(&cmd).unwrap();
        assert_eq!(json["type"], "status_summary");
        assert_eq!(json["summary"]["health"], "idle");
        assert_eq!(json["summary"]["panes"][0]["pane_id"], 7);

        let de: DrawCommand = serde_json::from_value(json).unwrap();
        match de {
            DrawCommand::StatusSummary { summary: roundtrip } => {
                assert_eq!(roundtrip, summary);
            }
            other => panic!("expected status summary, got {other:?}"),
        }
    }
}
