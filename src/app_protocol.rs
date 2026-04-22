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
    NotifyAction {
        notify_id: String,
        /// The label of the action the user clicked, or "dismiss" if dismissed.
        action_label: String,
    },
    /// Fired when a SetTimer timer expires.
    Timer { timer_id: String },
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
        #[serde(default)]
        actions: Vec<NotificationAction>,
        /// If set, host sends PlexiEvent::NotifyAction when the user responds.
        #[serde(default)]
        notify_id: Option<String>,
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
}

/// An action attached to a Notify command.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct NotificationAction {
    pub label: String,
    /// One of: "resume_run" | "open_intent" | "run_command"
    pub action_type: String,
    pub payload: serde_json::Value,
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
