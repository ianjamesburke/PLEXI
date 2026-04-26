//! Plexi external app protocol — PGAP v3 (newline-delimited JSON over stdin/stdout).
//!
//! # Protocol overview
//!
//! Binary data (audio PCM, video frames, raw bytes) travels on typed pipes — not stdio.
//! The PGAP wire carries only JSON control/draw messages.
//!
//! # Handshake
//!
//! 1. Host spawns the app binary.
//! 2. Host sends exactly one `Init` event.
//! 3. App sends `DrawCommand::Ready` once after receiving `Init`.
//! 4. Each frame: host sends `Render`; app replies with `DrawCommand`s + `FrameDone`.
//! 5. Input events (`Key`, `Click`, `Command`) arrive between frames as they occur.
//! 6. Out-of-frame draw commands (`CapabilityRequest`, `SecretGet`, `Notify`, etc.)
//!    may arrive at any time, including mid-frame; host processes them immediately.
//! 7. On close: host sends `Shutdown`; app must exit cleanly within a short timeout.
//!
//! # Example app (pseudocode)
//!
//! ```
//! let init = read_json_line(stdin);  // PlexiEvent::Init
//! write_json(DrawCommand::Ready { sdk: "my-sdk/1.0.0".into(), features_used: vec![] });
//! loop {
//!   let event = read_json_line(stdin);
//!   match event {
//!     PlexiEvent::Render { frame_id, .. } => {
//!       write_json(DrawCommand::Rect { x:0, y:0, w:800, h:600, fill:"#1e1e2e", radius:0.0 });
//!       write_json(DrawCommand::Text { x:20, y:20, text:"Hello v3!", size:14.0, color:"#cdd6f4", monospace:false, bold:false });
//!       write_json(DrawCommand::FrameDone { frame_id });
//!     }
//!     PlexiEvent::Key { key, .. } => { /* navigate */ }
//!     _ => {}
//!   }
//! }
//! ```

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ── Events sent FROM Plexi TO the app ────────────────────────────────────────

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PlexiEvent {
    /// Sent exactly once on startup. App must reply with DrawCommand::Ready.
    Init {
        /// Protocol version string, e.g. "pgap/3". App must refuse unknown versions.
        protocol: String,
        /// Stable identifier for this app instance, e.g. "audio-recorder".
        app_id: String,
        /// The workspace root this app was launched from.
        /// Hard invariant: all SecretGet calls are scoped to this directory.
        workspace_root: PathBuf,
        /// Capabilities granted to this app (declared in manifest or runtime-prompted).
        /// e.g. ["audio.record", "fs.read"]
        capabilities: Vec<String>,
        /// Additive feature flags. Unknown flags are ignored.
        /// e.g. ["media_v1", "pane_groups_v1"]
        feature_flags: Vec<String>,
    },
    /// Request a new frame. App replies with DrawCommands terminated by FrameDone.
    Render {
        frame_id: u64,
        /// Current surface rect the app should draw into.
        rect: Rect,
    },
    /// Surface was resized. App should re-layout and request a new frame.
    Resize { width: f32, height: f32 },
    /// User input event.
    Key { key: String, modifiers: Modifiers },
    /// Mouse click at logical coordinates within the app surface.
    Click { x: f32, y: f32, button: MouseButton },
    /// User submitted a command via the command bar.
    Command { text: String },
    /// Response to a runtime CapabilityRequest.
    CapabilityDecision { request_id: String, granted: bool },
    /// Secret broker response. value is None when denied.
    SecretValue { key: String, value: Option<String> },
    /// Run lifecycle update from the host.
    RunUpdate {
        run_id: String,
        /// One of: "pending" | "running" | "blocked_on_user" | "completed" | "failed"
        status: String,
        payload: serde_json::Value,
    },
    /// Typed pipe message (JSON mode only; binary mode travels on the side channel).
    PipeMessage {
        pipe_id: String,
        payload: serde_json::Value,
    },
    /// Pane group CWD broadcast. Apps in the same group receive this when any
    /// member's CWD changes.
    PathChanged { cwd: PathBuf },
    /// App is being backgrounded (host window losing focus, app no longer visible).
    Suspend,
    /// App is being foregrounded again.
    Resume,
    /// App is being closed. Process must exit within a short timeout.
    Shutdown,
    /// Confirmation that a SpawnApp request succeeded.
    AppSpawned {
        /// The pane_id of the newly spawned app pane.
        pane_id: u64,
        type_id: String,
    },
    /// Binary pipe opened — app connects to `socket_path` as a unix socket client.
    PipeOpened {
        pipe_id: String,
        socket_path: String,
    },
    /// Binary pipe backpressure — host dropped `dropped_frames` frames from the ring.
    PipeOverrun {
        pipe_id: String,
        dropped_frames: u64,
    },
    /// Harness-only: drop a JSON payload into the app's `on_inject` hook.
    /// Used by `pgap_test_harness` to seed deterministic state without
    /// round-tripping through real inputs.
    InjectState { payload: serde_json::Value },
    /// Host broker response to a `DrawCommand::HttpRequest`. `error` is present
    /// when the request failed; `body` may still carry a partial response.
    HttpResponse {
        request_id: String,
        status: u16,
        body: String,
        #[serde(default)]
        error: Option<String>,
    },
    /// Sent when an LlmRequest completes.
    LlmResponse {
        request_id: String,
        /// The text content of the first choice, or empty on error.
        content: String,
        /// Set if the call failed.
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    /// Sent when the user responds to a notification that included a notify_id.
    ///
    /// - `action_label`: what was clicked — "acknowledge" for the default,
    ///   the option label for a choice, "submit" for input, or "cancel" if
    ///   dismissed with Esc (only possible when `required = false`).
    /// - `value`: option `value` for choice kind, typed text for input kind,
    ///   absent otherwise.
    NotifyAction {
        notify_id: String,
        action_label: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<String>,
    },
    /// Fired when a SetTimer timer expires.
    Timer { timer_id: String },
    /// Response to a `DrawCommand::MeasureText` request.
    /// `width` and `height` are in logical pixels at the requested font size.
    TextMeasured {
        request_id: String,
        width: f32,
        height: f32,
    },
    /// User pressed Enter inside a `DrawCommand::TextInput` field.
    ///
    /// `id` matches the `id` the app supplied on the `TextInput` command.
    /// `value` is the buffered text at submission time. The host clears its
    /// buffer for `id` after emitting this event so the field is empty for
    /// the next input.
    TextSubmitted { id: String, value: String },
    /// Sent once at agent startup to deliver the manifest's `[launch].system_prompt`.
    ///
    /// Only delivered to apps whose manifest declares `[app] type = "agent"`. Apps
    /// of `type = "app"` will never receive this event. The agent decides how to
    /// use the prompt — typically by passing it as the `system` field on the
    /// first `iq.query` call. Apps that don't need a system prompt may simply
    /// ignore the event.
    ///
    /// `system_prompt` is `None` when the manifest omits the `[launch].system_prompt`
    /// field; the host serialises it as JSON `null` so the value is explicit on
    /// the wire. Agents must handle the `None` case (no prompt set).
    AgentInit { system_prompt: Option<String> },
    /// User submitted text in the host-rendered conversation input box of an
    /// agent pane.
    ///
    /// Only delivered to apps whose manifest declares `[app] type = "agent"`.
    /// The agent receives the raw user text and decides how to respond
    /// (typically by appending it to its conversation history and dispatching
    /// an `iq.query`). The host owns the input widget; the agent owns the
    /// conversation logic.
    UserMessage { text: String },
    /// Clipboard paste forwarded into the focused app pane.
    ///
    /// Emitted whenever the host observes `egui::Event::Paste(text)` while an
    /// app pane has keyboard focus. The text is the OS clipboard contents at
    /// paste time, already decoded to UTF-8 by egui. Apps receive this both
    /// for Cmd+V chords and for OS-menu / right-click → Paste actions.
    ///
    /// Pre-#200 apps shelled out to `pbpaste` for this; that workaround is
    /// macOS-only and races with focus changes. This event is the portable
    /// path.
    Paste { text: String },
    /// Response to a `DrawCommand::IqQuery`. Either `content` is `Some` (success)
    /// or `error` is `Some` (failure) — the two are mutually exclusive. Token
    /// counts are zero on error.
    ///
    /// `error` is set when:
    ///   - the app does not declare the `iq.query` capability ("capability denied")
    ///   - the host backend cannot be reached (e.g. missing API key, claude CLI
    ///     not installed)
    ///   - the upstream backend returned an error mid-stream
    IqResponse {
        request_id: String,
        content: Option<String>,
        tokens_in: u32,
        tokens_out: u32,
        error: Option<String>,
    },
}

/// One message in an `IqQuery` conversation. Wire shape mirrors Anthropic
/// Messages API: `role` ∈ {"user", "assistant"}, `content` is plain text.
/// (Multimodal content blocks are future-scope.)
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct IqMessage {
    pub role: String,
    pub content: String,
}

/// Tool definition for a future tool-use turn loop. The current broker only
/// supports text-only turns; if `tools` is non-empty the broker returns
/// an error response. Reserved on the wire so that v3.4+ can add tool
/// dispatch without changing the protocol.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct IqTool {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// Coarse model tier requested by the app. The host maps each tier to a
/// concrete model identifier per backend (spec §iq.query):
///   - `Low`    → Haiku
///   - `Medium` → Sonnet
///   - `High`   → Opus
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelTier {
    Low,
    Medium,
    High,
}

/// A simple rectangle (logical coordinates).
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
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

// ── Commands sent FROM the app TO Plexi ──────────────────────────────────────

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DrawCommand {
    // ── Visual primitives (frame-scoped) ─────────────────────────────────

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
    /// End of frame. Host renders everything queued since last FrameDone.
    FrameDone {
        /// Must match the frame_id from the triggering Render event.
        frame_id: u64,
    },

    // ── Out-of-frame commands ─────────────────────────────────────────────
    /// Forward a log message into Plexi's logger (tagged with app_id).
    Log {
        /// One of: "error" | "warn" | "info" | "debug"
        level: String,
        message: String,
    },
    /// Request a runtime capability prompt. Host shows modal; responds with CapabilityDecision.
    CapabilityRequest {
        request_id: String,
        /// v3 capability string, e.g. "net.http"
        capability: String,
    },
    /// Request a workspace-scoped secret. Scoped to Init.workspace_root automatically.
    SecretGet { key: String },
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
    /// Send a JSON-mode pipe message (not for binary pipes).
    PipeSend {
        pipe_id: String,
        payload: serde_json::Value,
    },
    /// Update the status text shown in the parent pane chrome.
    StatusSummary { text: String },

    /// Request the host to spawn a new app pane. Requires `spawn.app` capability.
    /// `layout`: "split_v" (default, new pane below), "split_h" (new pane right),
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
    /// Host-brokered LLM call. Requires `llm` capability.
    /// Calls Anthropic Claude with the given prompt and optional system message.
    /// Host replies with `PlexiEvent::LlmResponse { request_id, content, error }`.
    LlmRequest {
        request_id: String,
        prompt: String,
        #[serde(default = "default_llm_model")]
        model: String,
        #[serde(default)]
        system: Option<String>,
    },
    /// v3.3 brokered LLM call. Requires `iq.query` capability.
    ///
    /// The host routes this to the active Plexi IQ backend, appends an
    /// `AgentTurn` row to `ledger.jsonl`, and replies with
    /// `PlexiEvent::IqResponse { request_id, content, tokens_in, tokens_out, error }`.
    ///
    /// All fields are required — no `serde(default)`. `tools` may be empty;
    /// non-empty `tools` is accepted on the wire but rejected at the broker
    /// (returns an error response) until v3.4 adds tool dispatch.
    IqQuery {
        request_id: String,
        model_tier: ModelTier,
        system: String,
        messages: Vec<IqMessage>,
        tools: Vec<IqTool>,
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
    /// Host-owned video decoder: emits frames on a binary pipe.
    VideoPlayer {
        source: String,
        rect: Rect,
        /// One of: "playing" | "paused" | "stopped".
        state: String,
    },
    /// Render an amplitude meter reading from a binary pipe.
    AudioMeter { rect: Rect, pipe_id: String },
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
    AudioCapture {
        pipe_id: String,
        #[serde(default = "default_sample_rate")]
        sample_rate: u32,
        #[serde(default = "default_buffer_size")]
        buffer_size: u32,
    },

    /// SDK ready handshake. Sent once by the app after receiving Init.
    /// Host captures sdk and features_used; the message is otherwise a no-op.
    Ready {
        #[serde(default)]
        sdk: String,
        #[serde(default)]
        features_used: Vec<String>,
    },
    /// Request the host to cd all terminals in the same pane group to `cwd`.
    /// Terminals receive `cd <cwd>\n` written to their PTY.
    CdRequest { cwd: String },

    /// Ask the host to trigger a new Render event after `after_ms` milliseconds.
    /// Intended for game loops and animations — emit once per frame to sustain a
    /// tick rate without relying on egui's unconditional repaint cadence.
    /// Apps that do not emit this will still repaint on keyboard/inject events.
    ScheduleRender { after_ms: u32 },

    /// Request a one-shot timer. Requires `timer` capability.
    /// Host fires `PlexiEvent::Timer { timer_id }` after `after_ms` milliseconds.
    SetTimer { timer_id: String, after_ms: u64 },
    /// Cancel a pending timer. No-op if the timer has already fired or doesn't exist.
    CancelTimer { timer_id: String },

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

    /// Write `text` to the OS clipboard.
    ///
    /// Routed through `egui::Context::copy_text`, which handles platform-
    /// specific clipboard backends (NSPasteboard / X11 / Wayland / Win32).
    /// Synchronous from the app's perspective — no acknowledgement event.
    /// No capability flag required: clipboard write is low-risk and the
    /// app already controls when it fires (key handler, button, etc.).
    CopyToClipboard { text: String },

    /// Append a row to the host-owned conversation history of an agent pane.
    ///
    /// Only meaningful for panes whose manifest declares `[app] type = "agent"`.
    /// The host renders the conversation history as the pane's primary surface;
    /// the agent emits one of these per logical turn boundary (user echo,
    /// assistant reply, tool result, system note).
    ///
    /// `role` is one of `"user"` | `"assistant"` | `"tool"` | `"system"`. Other
    /// values are accepted at the wire level (forward-compatibility) but the
    /// host renders unknown roles as plain text. Required field — no
    /// `serde(default)`.
    ///
    /// `content` is the plain-text body of the row. Required field. Empty
    /// strings are valid (e.g. a placeholder turn while a stream is in flight)
    /// but discouraged — emit on completion, not on dispatch.
    AppendConversation { role: String, content: String },

    /// Single-line text input field (host-owned buffer, submit-only).
    ///
    /// Emitted by the app each frame at `(x, y)` with width `w`. The host
    /// owns the underlying buffer keyed on `id` — typed characters never
    /// reach the app between frames. On Enter the host emits
    /// `PlexiEvent::TextSubmitted { id, value }` and clears its buffer.
    ///
    /// Real-time validation (per-keystroke value access) is intentionally
    /// out of scope — see issue #283 option A. Apps that need it must
    /// wait for a future protocol revision.
    TextInput {
        id: String,
        x: f32,
        y: f32,
        w: f32,
        placeholder: String,
    },
}

/// One group inside a `DrawCommand::Shortcuts`. Renders as `keys` chips
/// followed by `description` text.
#[derive(Serialize, Deserialize, Debug, Clone)]
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
/// user-facing policy declared in `manifest.toml::default_notification_scope`,
/// resolved by the host at dispatch time. Apps never think about it.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NotifyScope {
    /// Only visible when the source context is the active context.
    Context,
    /// Always visible regardless of which context is active.
    Global,
}

/// An action attached to a Notify command.
#[derive(Serialize, Deserialize, Debug, Clone)]
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
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NotifyKind {
    #[default]
    Message,
    Choice,
    Input,
}

/// One option in a `kind = "choice"` notification.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct NotifyOption {
    /// Visible label on the button.
    pub label: String,
    /// Value returned to the app in `PlexiEvent::NotifyAction.value`. If empty,
    /// the label is used.
    #[serde(default)]
    pub value: String,
    /// Optional single-char hotkey (e.g. "y", "n"). Case-insensitive.
    #[serde(default)]
    pub shortcut: Option<String>,
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

fn default_sample_rate() -> u32 {
    48_000
}

fn default_buffer_size() -> u32 {
    1024
}

fn default_llm_model() -> String {
    "claude-haiku-4-5-20251001".to_string()
}

#[cfg(test)]
mod tests {
    //! Wire-format round-trip tests for the v3.2 clipboard / paste / selectable
    //! text additions (#200 + #146). These pin the on-the-wire shape — every
    //! field is required and must be present. No `#[serde(default)]` papering
    //! over missing fields.
    use super::*;

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
            DrawCommand::CopyToClipboard { text } => assert_eq!(text, "snippet"),
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
            DrawCommand::Text {
                text, selectable, ..
            } => {
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
    // Pin the on-the-wire shape for IqQuery / IqResponse. All fields are
    // required — no `serde(default)`.

    #[test]
    fn iq_query_drawcommand_round_trips_serde() {
        let json = r#"{"type":"iq_query","request_id":"req-1","model_tier":"medium","system":"You are helpful.","messages":[{"role":"user","content":"hi"}],"tools":[]}"#;
        let cmd: DrawCommand = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            DrawCommand::IqQuery {
                request_id,
                model_tier,
                system,
                messages,
                tools,
            } => {
                assert_eq!(request_id, "req-1");
                assert_eq!(*model_tier, ModelTier::Medium);
                assert_eq!(system, "You are helpful.");
                assert_eq!(messages.len(), 1);
                assert_eq!(messages[0].role, "user");
                assert_eq!(messages[0].content, "hi");
                assert!(tools.is_empty());
            }
            other => panic!("expected IqQuery, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""type":"iq_query""#),
            "wire tag missing: {serialised}"
        );
        assert!(
            serialised.contains(r#""model_tier":"medium""#),
            "model_tier missing: {serialised}"
        );
    }

    #[test]
    fn iq_response_round_trips_serde() {
        let json = r#"{"type":"iq_response","request_id":"req-1","content":"Hello!","tokens_in":12,"tokens_out":4,"error":null}"#;
        let event: PlexiEvent = serde_json::from_str(json).expect("deserialise");
        match &event {
            PlexiEvent::IqResponse {
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
            other => panic!("expected IqResponse, got {other:?}"),
        }
        let serialised = serde_json::to_string(&event).expect("serialise");
        assert!(
            serialised.contains(r#""type":"iq_response""#),
            "wire tag missing: {serialised}"
        );
    }

    #[test]
    fn iq_response_with_error_serde() {
        let json = r#"{"type":"iq_response","request_id":"req-2","content":null,"tokens_in":0,"tokens_out":0,"error":"capability denied: iq.query not declared in manifest"}"#;
        let event: PlexiEvent = serde_json::from_str(json).expect("deserialise");
        match &event {
            PlexiEvent::IqResponse {
                content,
                error,
                tokens_in,
                tokens_out,
                ..
            } => {
                assert!(content.is_none(), "content must be None on error");
                assert_eq!(
                    error.as_deref(),
                    Some("capability denied: iq.query not declared in manifest")
                );
                assert_eq!(*tokens_in, 0);
                assert_eq!(*tokens_out, 0);
            }
            other => panic!("expected IqResponse, got {other:?}"),
        }
    }

    // ── v3.3 agent-as-app wire shape (#285) ──────────────────────────────
    // Pin the on-the-wire shape for the three new variants. All fields are
    // required — no `serde(default)`.

    #[test]
    fn user_message_event_round_trips_serde() {
        let json = r#"{"type":"user_message","text":"tell me a joke"}"#;
        let event: PlexiEvent = serde_json::from_str(json).expect("deserialise");
        match &event {
            PlexiEvent::UserMessage { text } => assert_eq!(text, "tell me a joke"),
            other => panic!("expected UserMessage, got {other:?}"),
        }
        let serialised = serde_json::to_string(&event).expect("serialise");
        assert!(
            serialised.contains(r#""type":"user_message""#),
            "wire tag missing: {serialised}"
        );
        assert!(
            serialised.contains(r#""text":"tell me a joke""#),
            "text missing: {serialised}"
        );
    }

    #[test]
    fn user_message_missing_text_fails_deserialise() {
        // No `text` field — must fail because the field is required (no
        // `serde(default)`).
        let json = r#"{"type":"user_message"}"#;
        let result: Result<PlexiEvent, _> = serde_json::from_str(json);
        assert!(
            result.is_err(),
            "deserialise should fail without `text` field"
        );
    }

    #[test]
    fn agent_init_event_round_trips_serde() {
        // With a system prompt set.
        let json = r#"{"type":"agent_init","system_prompt":"You are helpful."}"#;
        let event: PlexiEvent = serde_json::from_str(json).expect("deserialise");
        match &event {
            PlexiEvent::AgentInit { system_prompt } => {
                assert_eq!(system_prompt.as_deref(), Some("You are helpful."));
            }
            other => panic!("expected AgentInit, got {other:?}"),
        }
        let serialised = serde_json::to_string(&event).expect("serialise");
        assert!(
            serialised.contains(r#""type":"agent_init""#),
            "wire tag missing: {serialised}"
        );

        // Null system_prompt — agent manifests without a `[launch].system_prompt`.
        let json = r#"{"type":"agent_init","system_prompt":null}"#;
        let event: PlexiEvent = serde_json::from_str(json).expect("deserialise");
        match &event {
            PlexiEvent::AgentInit { system_prompt } => {
                assert!(system_prompt.is_none(), "system_prompt must be None");
            }
            other => panic!("expected AgentInit, got {other:?}"),
        }
    }

    #[test]
    fn append_conversation_drawcommand_round_trips() {
        let json =
            r#"{"type":"append_conversation","role":"assistant","content":"Hello!"}"#;
        let cmd: DrawCommand = serde_json::from_str(json).expect("deserialise");
        match &cmd {
            DrawCommand::AppendConversation { role, content } => {
                assert_eq!(role, "assistant");
                assert_eq!(content, "Hello!");
            }
            other => panic!("expected AppendConversation, got {other:?}"),
        }
        let serialised = serde_json::to_string(&cmd).expect("serialise");
        assert!(
            serialised.contains(r#""type":"append_conversation""#),
            "wire tag missing: {serialised}"
        );
        assert!(
            serialised.contains(r#""role":"assistant""#),
            "role missing: {serialised}"
        );
    }

    #[test]
    fn append_conversation_missing_content_fails_deserialise() {
        // No `content` field — must fail. Required.
        let json = r#"{"type":"append_conversation","role":"user"}"#;
        let result: Result<DrawCommand, _> = serde_json::from_str(json);
        assert!(
            result.is_err(),
            "deserialise should fail without `content` field"
        );
    }

    #[test]
    fn iq_query_missing_required_field_fails_deserialise() {
        // No `tools` field — must fail because the field is required
        // (no `#[serde(default)]` on it).
        let json = r#"{"type":"iq_query","request_id":"r","model_tier":"low","system":"","messages":[]}"#;
        let result: Result<DrawCommand, _> = serde_json::from_str(json);
        assert!(
            result.is_err(),
            "deserialise should fail without `tools` field"
        );
    }
}
