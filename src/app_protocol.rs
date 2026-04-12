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
///     Init { width, height } => { /* store dimensions */ }
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
    },
    /// Request a new frame. App should reply with DrawCommands + FrameDone.
    Render { width: f32, height: f32, delta_time: f32 },
    /// Surface was resized.
    Resize { width: f32, height: f32 },
    /// A key was pressed.
    Key {
        key: String,
        modifiers: Modifiers,
    },
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
    Scroll { x: f32, y: f32, delta_x: f32, delta_y: f32 },
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
    /// App is being closed. Process should exit.
    Shutdown,
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
    /// Raise a notification. Plexi records it to the notification log,
    /// increments the status-bar unread count, and surfaces it in the
    /// notification palette (Cmd+Shift+N).
    ///
    /// Priorities: `0` = info, `1` = normal, `2` = high, `3` = urgent.
    /// The MVP does not style by priority — the value is just logged.
    Notification {
        priority: u8,
        title: String,
        #[serde(default)]
        body: Option<String>,
        /// The `app_id` of the emitter (e.g. `"parallax"`).
        source_app: String,
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
    /// End of frame — Plexi will render everything queued since last FrameDone.
    FrameDone,
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
