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
        /// Protocol version negotiation.
        #[serde(default = "default_protocol_version")]
        protocol_version: u32,
        /// Structured spawn intent.
        #[serde(default)]
        open_intent: Option<OpenIntent>,
    },
    /// Request a new frame. App should reply with DrawCommands + FrameDone.
    Render { width: f32, height: f32 },
    /// Surface was resized.
    Resize { width: f32, height: f32 },
    /// A key was pressed.
    Key {
        key: String,
        modifiers: Modifiers,
    },
    /// Mouse click at logical coordinates within the app surface.
    Click { x: f32, y: f32, button: MouseButton },
    /// User submitted a command via the terminal command bar.
    Command { text: String },
    /// App is being closed. Process should exit.
    Shutdown,
    /// A Run was created; the run_id is returned to the requesting app.
    RunCreated { run_id: String },
    /// A bus event forwarded to a subscribed app.
    EventData { event: BusEvent },
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
    /// End of frame — Plexi will render everything queued since last FrameDone.
    FrameDone,
    /// Emit a notification.
    Notification {
        id: String,
        title: String,
        #[serde(default)]
        body: Option<String>,
        #[serde(default)]
        urgency: Option<String>,
        #[serde(default)]
        run_id: Option<String>,
        #[serde(default)]
        action: Option<NotificationAction>,
    },
    /// Write data to a named pipe channel.
    PipeWrite {
        channel: String,
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
    /// Subscribe to bus events.
    EventSubscribe {
        #[serde(default)]
        kinds: Vec<String>,
        #[serde(default)]
        scope: SubscribeScope,
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
