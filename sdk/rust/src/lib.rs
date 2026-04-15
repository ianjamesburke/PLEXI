//! Plexi SDK for Rust
//!
//! Build Plexi apps in Rust by implementing the [`App`] trait and calling [`run`].
//!
//! # Example
//!
//! ```rust,no_run
//! use plexi_sdk::{App, Emitter, Modifiers, MouseButton, RenderContext, run};
//!
//! struct Counter { count: u32 }
//!
//! impl App for Counter {
//!     fn on_render(&mut self, ctx: &mut RenderContext) {
//!         ctx.rect(0.0, 0.0, ctx.width, ctx.height, "#1e1e2e");
//!         ctx.text(20.0, 20.0, &format!("Count: {}", self.count), 16.0, "#cdd6f4");
//!     }
//!
//!     fn on_key(&mut self, key: &str, _mods: &Modifiers, _emit: &mut Emitter) {
//!         match key {
//!             "J" | "ArrowDown" => self.count += 1,
//!             "K" | "ArrowUp"   => self.count = self.count.saturating_sub(1),
//!             _ => {}
//!         }
//!     }
//! }
//!
//! fn main() { run(&mut Counter { count: 0 }); }
//! ```

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::env;
use std::fs::{create_dir_all, read_to_string, OpenOptions};
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

// ── Inbound events (Plexi → app) ─────────────────────────────────────────────

/// Events sent from Plexi to the app over stdin.
///
/// Mirror of `crate::app_protocol::PlexiEvent` in the Plexi host. Uses
/// `#[serde(tag = "type", rename_all = "snake_case")]` so the JSON shape on
/// the wire matches the host exactly.
#[derive(Deserialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PlexiEvent {
    Init {
        width: f32,
        height: f32,
        pixels_per_point: f32,
    },
    Render {
        width: f32,
        height: f32,
        #[serde(default)]
        delta_time: f32,
    },
    Resize {
        width: f32,
        height: f32,
    },
    Key {
        key: String,
        modifiers: Modifiers,
    },
    Click {
        x: f32,
        y: f32,
        button: MouseButton,
    },
    MouseDown {
        x: f32,
        y: f32,
        button: String,
    },
    MouseUp {
        x: f32,
        y: f32,
        button: String,
    },
    MouseMove {
        x: f32,
        y: f32,
    },
    Scroll {
        x: f32,
        y: f32,
        delta_x: f32,
        delta_y: f32,
    },
    Command {
        text: String,
    },
    Drop {
        target_id: String,
        paths: Vec<String>,
    },
    GetState,
    SetState {
        #[serde(default)]
        user_state: Value,
        #[serde(default)]
        derived: Value,
        #[serde(default)]
        session: Value,
        #[serde(default)]
        persistent: Value,
    },
    Shutdown,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub cmd: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MouseButton {
    Primary,
    Secondary,
}

// ── Outbound draw commands (app → Plexi) ─────────────────────────────────────

/// Commands sent from the app to Plexi over stdout.
///
/// Mirror of `crate::app_protocol::DrawCommand` in the Plexi host.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DrawCommand {
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        fill: String,
        #[serde(default)]
        radius: f32,
    },
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
    Line {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        color: String,
        #[serde(default = "default_stroke_width")]
        width: f32,
    },
    List {
        items: Vec<ListItem>,
        selected: usize,
        #[serde(default)]
        item_height: f32,
    },
    Image {
        path: String,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fit: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rounding: Option<f32>,
    },
    VideoThumbnail {
        path: String,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        show_play_button: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timestamp_seconds: Option<f32>,
    },
    FileGrid {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filter: Option<Vec<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        paths: Option<Vec<String>>,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        item_size: Option<f32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        columns: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        show_labels: Option<bool>,
    },
    RunInTerminal {
        command: String,
    },
    Cd {
        path: String,
    },
    Log {
        level: String,
        message: String,
    },
    State {
        #[serde(default)]
        user_state: Value,
        #[serde(default)]
        derived: Value,
        #[serde(default)]
        session: Value,
        #[serde(default)]
        persistent: Value,
    },
    CostReport {
        app_id: String,
        service: String,
        model: String,
        input_tokens: u64,
        output_tokens: u64,
        cost_usd: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        operation_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timestamp: Option<String>,
    },
    Notification {
        priority: u8,
        title: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        body: Option<String>,
        source_app: String,
    },
    DropTarget {
        id: String,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        #[serde(default)]
        accept: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    SetCursor {
        cursor: String,
    },
    MouseTracking {
        enabled: bool,
    },
    /// Ask Plexi to spawn another app and place it in a layout slot relative
    /// to the emitting pane. See [`Emitter::spawn_app`] and [`SpawnLayout`]
    /// for the full field reference.
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
    FrameDone,
}

/// Anchor for a `SpawnApp` layout — where the new pane sits relative to.
/// Serializes as a bare string: `"self"`, `"root"`, or `"mark:<name>"`.
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
                "invalid spawn parent: {other:?}"
            ))),
        }
    }
}

/// How a `SpawnApp` call positions the new pane relative to the parent.
///
/// Serialized as an internally tagged object with `kind` as the discriminator:
///     {"kind":"fill"}
///     {"kind":"cols","slot":1,"ratio":0.5}
///     {"kind":"rows","slot":1,"ratio":0.4}
///     {"kind":"grid_2x2","slot":0}
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
        spec: Value,
    },
}

impl Default for SpawnLayout {
    fn default() -> Self {
        SpawnLayout::Fill
    }
}

/// What happens to a spawned child when its parent closes.
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secondary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default)]
    pub is_dir: bool,
}

impl ListItem {
    pub fn new(label: impl Into<String>) -> Self {
        Self { label: label.into(), secondary: None, icon: None, is_dir: false }
    }

    pub fn secondary(mut self, s: impl Into<String>) -> Self {
        self.secondary = Some(s.into());
        self
    }

    pub fn icon(mut self, s: impl Into<String>) -> Self {
        self.icon = Some(s.into());
        self
    }

    pub fn dir(mut self) -> Self {
        self.is_dir = true;
        self
    }
}

fn default_stroke_width() -> f32 {
    1.0
}

// ── Snapshot returned from on_get_state ──────────────────────────────────────

/// Buckets of state returned from [`App::on_get_state`].
///
/// Each bucket is an opaque `serde_json::Value` mapped 1:1 to the
/// `state` draw command's fields. Use [`StateSnapshot::default`] for empty
/// state, then assign whichever buckets your app needs.
#[derive(Debug, Clone, Default)]
pub struct StateSnapshot {
    pub user_state: Value,
    pub derived: Value,
    pub session: Value,
    pub persistent: Value,
}

impl StateSnapshot {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_user_state(mut self, v: Value) -> Self {
        self.user_state = v;
        self
    }

    pub fn with_derived(mut self, v: Value) -> Self {
        self.derived = v;
        self
    }

    pub fn with_session(mut self, v: Value) -> Self {
        self.session = v;
        self
    }

    pub fn with_persistent(mut self, v: Value) -> Self {
        self.persistent = v;
        self
    }
}

// ── Emitter: write commands outside a render frame ───────────────────────────

/// Sends draw commands to Plexi outside of a render frame.
///
/// Each call writes a single newline-delimited JSON object to stdout.
/// `app_id` is set automatically by the runtime from the `PLEXI_APP_ID` env
/// var so cost reports and notifications are attributed correctly.
pub struct Emitter {
    app_id: String,
}

impl Emitter {
    fn new() -> Self {
        Self { app_id: env::var("PLEXI_APP_ID").unwrap_or_default() }
    }

    /// The app id this emitter was constructed with (from `PLEXI_APP_ID`).
    pub fn app_id(&self) -> &str {
        &self.app_id
    }

    fn write(&self, cmd: &DrawCommand) {
        let stdout = io::stdout();
        let mut out = stdout.lock();
        if let Ok(s) = serde_json::to_string(cmd) {
            let _ = writeln!(out, "{}", s);
        }
        let _ = out.flush();
    }

    /// Execute a shell command in the linked terminal immediately.
    pub fn run_in_terminal(&self, command: &str) {
        self.write(&DrawCommand::RunInTerminal { command: command.to_string() });
    }

    /// Change the linked terminal's working directory immediately.
    pub fn cd(&self, path: &str) {
        self.write(&DrawCommand::Cd { path: path.to_string() });
    }

    /// Forward a log message to Plexi's logger (`error`|`warn`|`info`|`debug`).
    pub fn log(&self, level: &str, message: &str) {
        self.write(&DrawCommand::Log {
            level: level.to_string(),
            message: message.to_string(),
        });
    }

    /// Log at info level.
    pub fn info(&self, message: &str) {
        self.log("info", message);
    }

    /// Log at warn level.
    pub fn warn(&self, message: &str) {
        self.log("warn", message);
    }

    /// Log at error level.
    pub fn error(&self, message: &str) {
        self.log("error", message);
    }

    /// Log at debug level.
    pub fn debug(&self, message: &str) {
        self.log("debug", message);
    }

    /// Report LLM API costs to Plexi for logging and tracking.
    ///
    /// `app_id` and `timestamp` are filled automatically. `operation_id`
    /// defaults to a fresh UUID-shaped string if not supplied.
    pub fn cost_report(
        &self,
        service: &str,
        model: &str,
        input_tokens: u64,
        output_tokens: u64,
        cost_usd: f64,
        operation_id: Option<&str>,
    ) {
        self.write(&DrawCommand::CostReport {
            app_id: self.app_id.clone(),
            service: service.to_string(),
            model: model.to_string(),
            input_tokens,
            output_tokens,
            cost_usd,
            operation_id: Some(
                operation_id.map(|s| s.to_string()).unwrap_or_else(generate_operation_id),
            ),
            timestamp: Some(now_rfc3339()),
        });
    }

    /// Raise a notification to Plexi's notification log.
    ///
    /// Priorities: `0 = info`, `1 = normal`, `2 = high`, `3 = urgent`.
    pub fn notification(&self, priority: u8, title: &str, body: Option<&str>) {
        self.write(&DrawCommand::Notification {
            priority,
            title: title.to_string(),
            body: body.map(|s| s.to_string()),
            source_app: self.app_id.clone(),
        });
    }

    /// Ask Plexi to spawn another app and place it in a layout slot relative
    /// to this pane. This is the composition primitive: a file browser
    /// pressing Enter on a `.txt` file can emit
    /// `spawn_app("text-editor", &["/tmp/notes.txt"], SpawnParent::SelfPane,
    /// SpawnLayout::Cols { slot: 1, ratio: 0.5 }, SpawnLifecycle::Cascade,
    /// true, &[])` to bring up a text editor in a 50/50 right split,
    /// lifecycle-bonded to itself.
    ///
    /// Authorization: the target app's `[app.spawnable]` manifest table is
    /// consulted. If `allow_callers` doesn't include this app, or the
    /// requested lifecycle isn't in `allow_lifecycle`, the spawn is refused
    /// and a notification is delivered back to this app.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_app(
        &self,
        app_id: &str,
        args: &[&str],
        parent: SpawnParent,
        layout: SpawnLayout,
        lifecycle: SpawnLifecycle,
        linked: bool,
        wire_channels: &[&str],
    ) {
        self.write(&DrawCommand::SpawnApp {
            app_id: app_id.to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            parent,
            layout,
            lifecycle,
            linked,
            wire_channels: wire_channels.iter().map(|s| s.to_string()).collect(),
        });
    }

    /// Submit user feedback for this app. Writes to `feedback.jsonl` in the
    /// app's install directory — this is a **client-side helper**, not a
    /// draw command.
    ///
    /// Reads `PLEXI_APP_ID` and `PLEXI_APPS_DIR` env vars set by the host.
    /// Falls back to `~/.plexi/apps/<app_id>/feedback.jsonl` if missing.
    pub fn submit_feedback(&self, text: &str, rating: Option<u8>, category: Option<&str>) {
        let app_id = if self.app_id.is_empty() {
            env::var("PLEXI_APP_ID").unwrap_or_else(|_| "unknown".to_string())
        } else {
            self.app_id.clone()
        };
        let apps_dir = env::var("PLEXI_APPS_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                let home = env::var("HOME").unwrap_or_default();
                PathBuf::from(home).join(".plexi").join("apps")
            });
        let dir = apps_dir.join(&app_id);
        let path = dir.join("feedback.jsonl");

        let mut entry = serde_json::Map::new();
        entry.insert("ts".to_string(), Value::String(now_rfc3339()));
        entry.insert("text".to_string(), Value::String(text.to_string()));
        if let Some(r) = rating {
            entry.insert("rating".to_string(), Value::Number(r.into()));
        }
        if let Some(c) = category {
            entry.insert("category".to_string(), Value::String(c.to_string()));
        }
        let entry_value = Value::Object(entry);

        let write_result = (|| -> io::Result<()> {
            create_dir_all(&dir)?;
            let mut f = OpenOptions::new().create(true).append(true).open(&path)?;
            writeln!(f, "{}", entry_value)?;
            Ok(())
        })();

        if let Err(e) = write_result {
            self.warn(&format!(
                "submit_feedback: could not write to {}: {}",
                path.display(),
                e
            ));
        }
        let preview: String = text.chars().take(80).collect();
        self.info(&format!("feedback submitted: {}", preview));
    }
}

// ── RenderContext ─────────────────────────────────────────────────────────────

/// Drawing surface passed to [`App::on_render`]. Accumulates draw commands
/// and flushes them as one frame when `on_render` returns.
pub struct RenderContext {
    pub width: f32,
    pub height: f32,
    pub delta_time: f32,
    app_id: String,
    commands: Vec<DrawCommand>,
}

impl RenderContext {
    fn new(width: f32, height: f32, delta_time: f32, app_id: String) -> Self {
        Self { width, height, delta_time, app_id, commands: Vec::new() }
    }

    /// Fill a rectangle.
    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, fill: &str) -> &mut Self {
        self.commands.push(DrawCommand::Rect { x, y, w, h, fill: fill.to_string(), radius: 0.0 });
        self
    }

    /// Fill a rounded rectangle.
    pub fn rect_rounded(&mut self, x: f32, y: f32, w: f32, h: f32, fill: &str, radius: f32) -> &mut Self {
        self.commands.push(DrawCommand::Rect { x, y, w, h, fill: fill.to_string(), radius });
        self
    }

    /// Draw text.
    pub fn text(&mut self, x: f32, y: f32, text: &str, size: f32, color: &str) -> &mut Self {
        self.commands.push(DrawCommand::Text {
            x, y, text: text.to_string(), size, color: color.to_string(),
            monospace: false, bold: false, align: None,
        });
        self
    }

    /// Draw monospace text (uses terminal font).
    pub fn text_mono(&mut self, x: f32, y: f32, text: &str, size: f32, color: &str) -> &mut Self {
        self.commands.push(DrawCommand::Text {
            x, y, text: text.to_string(), size, color: color.to_string(),
            monospace: true, bold: false, align: None,
        });
        self
    }

    /// Draw bold text.
    pub fn text_bold(&mut self, x: f32, y: f32, text: &str, size: f32, color: &str) -> &mut Self {
        self.commands.push(DrawCommand::Text {
            x, y, text: text.to_string(), size, color: color.to_string(),
            monospace: false, bold: true, align: None,
        });
        self
    }

    /// Draw right-aligned text (anchor at `x` = right edge of text).
    pub fn text_right(&mut self, x: f32, y: f32, text: &str, size: f32, color: &str) -> &mut Self {
        self.commands.push(DrawCommand::Text {
            x, y, text: text.to_string(), size, color: color.to_string(),
            monospace: false, bold: false, align: Some("right".to_string()),
        });
        self
    }

    /// Draw center-aligned text (anchor at `x` = horizontal center of text).
    pub fn text_center(&mut self, x: f32, y: f32, text: &str, size: f32, color: &str) -> &mut Self {
        self.commands.push(DrawCommand::Text {
            x, y, text: text.to_string(), size, color: color.to_string(),
            monospace: false, bold: false, align: Some("center".to_string()),
        });
        self
    }

    /// Draw a line.
    pub fn line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, color: &str) -> &mut Self {
        self.commands.push(DrawCommand::Line {
            x1, y1, x2, y2, color: color.to_string(), width: 1.0,
        });
        self
    }

    /// Draw a line with explicit stroke width.
    pub fn line_width(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, color: &str, width: f32) -> &mut Self {
        self.commands.push(DrawCommand::Line {
            x1, y1, x2, y2, color: color.to_string(), width,
        });
        self
    }

    /// High-level scrollable list. Plexi handles layout, scrolling, and highlight.
    ///
    /// **Warning:** the `list` primitive is full-pane only. It has no `x/y/w/h`
    /// parameters and will overlap any other draw calls in the same frame.
    /// Render lists manually with `text` + `rect` if you need positioned layout.
    pub fn list(&mut self, items: Vec<ListItem>, selected: usize, item_height: f32) -> &mut Self {
        self.commands.push(DrawCommand::List { items, selected, item_height });
        self
    }

    /// Draw an image from a file on disk. `fit` is one of `"contain"`,
    /// `"cover"`, `"fill"`. `rounding` in logical pixels.
    pub fn image(
        &mut self,
        path: &str,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        fit: Option<&str>,
        rounding: Option<f32>,
    ) -> &mut Self {
        self.commands.push(DrawCommand::Image {
            path: path.to_string(),
            x, y, w, h,
            fit: fit.map(|s| s.to_string()),
            rounding,
        });
        self
    }

    /// Draw a video thumbnail (extracted by Plexi via ffmpeg, cached on disk).
    pub fn video_thumbnail(
        &mut self,
        path: &str,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        show_play_button: Option<bool>,
        timestamp_seconds: Option<f32>,
    ) -> &mut Self {
        self.commands.push(DrawCommand::VideoThumbnail {
            path: path.to_string(),
            x, y, w, h,
            show_play_button,
            timestamp_seconds,
        });
        self
    }

    /// Draw a grid of files with auto-generated thumbnails. Provide either
    /// `path` (directory walk) or `paths` (explicit list).
    #[allow(clippy::too_many_arguments)]
    pub fn file_grid(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        path: Option<&str>,
        filter: Option<Vec<String>>,
        paths: Option<Vec<String>>,
        item_size: Option<f32>,
        columns: Option<u32>,
        show_labels: Option<bool>,
    ) -> &mut Self {
        self.commands.push(DrawCommand::FileGrid {
            path: path.map(|s| s.to_string()),
            filter,
            paths,
            x, y, w, h,
            item_size,
            columns,
            show_labels,
        });
        self
    }

    /// Declare a region that accepts dropped files from outside Plexi.
    /// Stateless per frame — re-emit on every render.
    pub fn drop_target(
        &mut self,
        id: &str,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        accept: Vec<String>,
        label: Option<&str>,
    ) -> &mut Self {
        self.commands.push(DrawCommand::DropTarget {
            id: id.to_string(),
            x, y, w, h,
            accept,
            label: label.map(|s| s.to_string()),
        });
        self
    }

    /// Set the cursor icon for this frame.
    ///
    /// Values: `"default"`, `"pointer"`, `"grab"`, `"grabbing"`,
    /// `"crosshair"`, `"text"`. Resets to `"default"` each frame.
    pub fn set_cursor(&mut self, cursor: &str) -> &mut Self {
        self.commands.push(DrawCommand::SetCursor { cursor: cursor.to_string() });
        self
    }

    /// Enable or disable mouse-move event delivery. Stateful — persists
    /// until changed.
    pub fn mouse_tracking(&mut self, enabled: bool) -> &mut Self {
        self.commands.push(DrawCommand::MouseTracking { enabled });
        self
    }

    /// Queue a terminal command to emit at end of this frame.
    pub fn run_in_terminal(&mut self, command: &str) -> &mut Self {
        self.commands.push(DrawCommand::RunInTerminal { command: command.to_string() });
        self
    }

    /// Queue a `cd` for the linked terminal at end of this frame.
    pub fn cd(&mut self, path: &str) -> &mut Self {
        self.commands.push(DrawCommand::Cd { path: path.to_string() });
        self
    }

    /// Forward a log line to Plexi's logger from inside a render frame.
    pub fn log(&mut self, level: &str, message: &str) -> &mut Self {
        self.commands.push(DrawCommand::Log {
            level: level.to_string(),
            message: message.to_string(),
        });
        self
    }

    /// Log at info level.
    pub fn info(&mut self, message: &str) -> &mut Self {
        self.log("info", message)
    }

    /// Log at warn level.
    pub fn warn(&mut self, message: &str) -> &mut Self {
        self.log("warn", message)
    }

    /// Log at error level.
    pub fn error(&mut self, message: &str) -> &mut Self {
        self.log("error", message)
    }

    /// Log at debug level.
    pub fn debug(&mut self, message: &str) -> &mut Self {
        self.log("debug", message)
    }

    /// Raise a notification from inside a render frame.
    pub fn notification(&mut self, priority: u8, title: &str, body: Option<&str>) -> &mut Self {
        self.commands.push(DrawCommand::Notification {
            priority,
            title: title.to_string(),
            body: body.map(|s| s.to_string()),
            source_app: self.app_id.clone(),
        });
        self
    }

    /// Queue a `spawn_app` request at end of this frame. Mirrors
    /// [`Emitter::spawn_app`] — see that method's docs for full semantics.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_app(
        &mut self,
        app_id: &str,
        args: &[&str],
        parent: SpawnParent,
        layout: SpawnLayout,
        lifecycle: SpawnLifecycle,
        linked: bool,
        wire_channels: &[&str],
    ) -> &mut Self {
        self.commands.push(DrawCommand::SpawnApp {
            app_id: app_id.to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            parent,
            layout,
            lifecycle,
            linked,
            wire_channels: wire_channels.iter().map(|s| s.to_string()).collect(),
        });
        self
    }

    fn flush(self) {
        let stdout = io::stdout();
        let mut out = stdout.lock();
        for cmd in self.commands {
            let _ = writeln!(out, "{}", serde_json::to_string(&cmd).unwrap_or_default());
        }
        let _ = writeln!(out, "{}", serde_json::to_string(&DrawCommand::FrameDone).unwrap());
        let _ = out.flush();
    }
}

// ── Breakpoints ───────────────────────────────────────────────────────────────

/// A render function selected based on a minimum pane size.
///
/// Breakpoints are a declarative alternative to a hand-rolled
/// `if width < 400 { render_compact() } else { render_full() }` branch
/// inside `on_render`. Collect a set of them in a [`BreakpointSet`] and
/// call [`BreakpointSet::dispatch`] from `on_render`, or use the free
/// helper [`pick_breakpoint`] to pick the winning index against an array
/// of `(min_width, min_height)` tuples and then branch to your own
/// `&mut self` methods.
///
/// The render closure receives only a `&mut RenderContext`, so the
/// stateless-closure form is best for pure-drawing helpers. Apps that
/// need to touch `&mut self` should use [`pick_breakpoint`] instead and
/// dispatch manually — see the README for an example.
pub struct Breakpoint {
    pub min_width: f32,
    pub min_height: f32,
    render: Box<dyn FnMut(&mut RenderContext)>,
}

impl Breakpoint {
    /// Build a new breakpoint with a minimum pane size in logical pixels.
    pub fn new<F>(min_width: f32, min_height: f32, render: F) -> Self
    where
        F: FnMut(&mut RenderContext) + 'static,
    {
        Self { min_width, min_height, render: Box::new(render) }
    }

    /// Convenience for a `(0, 0)` breakpoint that always matches. Use
    /// this as the last entry in a [`BreakpointSet`] to guarantee
    /// something always renders when no larger breakpoint fits.
    pub fn default_fallback<F>(render: F) -> Self
    where
        F: FnMut(&mut RenderContext) + 'static,
    {
        Self::new(0.0, 0.0, render)
    }
}

/// A stack of [`Breakpoint`]s that picks the most-specific match per frame.
///
/// On each call to [`dispatch`](Self::dispatch), the set walks its
/// entries sorted by `min_width * min_height` descending and invokes the
/// first one whose bounds fit the current pane. If none match, a
/// `(0, 0)` fallback entry is invoked if present; otherwise nothing is
/// drawn for that frame.
pub struct BreakpointSet {
    entries: Vec<Breakpoint>,
}

impl Default for BreakpointSet {
    fn default() -> Self {
        Self::new()
    }
}

impl BreakpointSet {
    pub fn new() -> Self {
        Self { entries: Vec::new() }
    }

    /// Add a breakpoint. Builder-style.
    pub fn add(mut self, bp: Breakpoint) -> Self {
        self.entries.push(bp);
        self
    }

    /// Add a breakpoint via the tuple-of-mins + closure form. Builder-style.
    pub fn breakpoint<F>(mut self, min_width: f32, min_height: f32, render: F) -> Self
    where
        F: FnMut(&mut RenderContext) + 'static,
    {
        self.entries.push(Breakpoint::new(min_width, min_height, render));
        self
    }

    /// Add the `(0, 0)` fallback breakpoint. Builder-style.
    pub fn fallback<F>(mut self, render: F) -> Self
    where
        F: FnMut(&mut RenderContext) + 'static,
    {
        self.entries.push(Breakpoint::default_fallback(render));
        self
    }

    /// Pick and run the best-matching breakpoint for the given pane size.
    /// Returns `true` if a breakpoint fired, `false` if none matched and
    /// no `(0, 0)` fallback was registered.
    pub fn dispatch(&mut self, ctx: &mut RenderContext) -> bool {
        let (w, h) = (ctx.width, ctx.height);
        let mut best: Option<usize> = None;
        let mut best_area: f32 = -1.0;
        for (i, bp) in self.entries.iter().enumerate() {
            if w >= bp.min_width && h >= bp.min_height {
                let area = bp.min_width * bp.min_height;
                // Strict `>` so later equal-area entries don't override earlier ones.
                if area > best_area {
                    best_area = area;
                    best = Some(i);
                }
            }
        }
        if let Some(i) = best {
            (self.entries[i].render)(ctx);
            return true;
        }
        // Fall back to an explicit (0, 0) entry if one exists.
        for bp in self.entries.iter_mut() {
            if bp.min_width == 0.0 && bp.min_height == 0.0 {
                (bp.render)(ctx);
                return true;
            }
        }
        false
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Pick the most specific breakpoint index from an array of
/// `(min_width, min_height)` tuples for a given pane size.
///
/// Returns the index of the largest-area entry whose bounds fit the
/// pane, or `None` if nothing matches (in which case the caller should
/// fall back to whichever entry they treat as the floor — usually the
/// last one).
///
/// This is the stateless form: it lets you dispatch to your own
/// `&mut self` methods from inside `on_render` without the closures
/// having to capture `self`.
///
/// ```ignore
/// fn on_render(&mut self, ctx: &mut RenderContext) {
///     const BPS: &[(f32, f32)] = &[(800.0, 500.0), (400.0, 0.0), (0.0, 0.0)];
///     match pick_breakpoint(ctx.width, ctx.height, BPS).unwrap_or(BPS.len() - 1) {
///         0 => self.render_full(ctx),
///         1 => self.render_compact(ctx),
///         _ => self.render_fallback(ctx),
///     }
/// }
/// ```
pub fn pick_breakpoint(width: f32, height: f32, breakpoints: &[(f32, f32)]) -> Option<usize> {
    let mut best: Option<usize> = None;
    let mut best_area: f32 = -1.0;
    for (i, (mw, mh)) in breakpoints.iter().enumerate() {
        if width >= *mw && height >= *mh {
            let area = mw * mh;
            if area > best_area {
                best_area = area;
                best = Some(i);
            }
        }
    }
    best
}

// ── App trait ─────────────────────────────────────────────────────────────────

/// Implement this trait to build a Plexi app. All methods have default
/// no-op implementations — override only what you need.
pub trait App {
    fn on_render(&mut self, ctx: &mut RenderContext) {
        let _ = ctx;
    }
    fn on_key(&mut self, _key: &str, _mods: &Modifiers, _emit: &mut Emitter) {}
    fn on_click(&mut self, _x: f32, _y: f32, _button: &MouseButton, _emit: &mut Emitter) {}
    fn on_command(&mut self, _text: &str, _emit: &mut Emitter) {}
    fn on_resize(&mut self, _width: f32, _height: f32) {}
    fn on_mouse_down(&mut self, _x: f32, _y: f32, _button: &str, _emit: &mut Emitter) {}
    fn on_mouse_up(&mut self, _x: f32, _y: f32, _button: &str, _emit: &mut Emitter) {}
    fn on_mouse_move(&mut self, _x: f32, _y: f32, _emit: &mut Emitter) {}
    fn on_scroll(&mut self, _x: f32, _y: f32, _delta_x: f32, _delta_y: f32, _emit: &mut Emitter) {}
    fn on_drop(&mut self, _target_id: &str, _paths: &[String], _emit: &mut Emitter) {}

    /// Return the app's current state in response to a `GetState` event.
    /// Default is empty buckets.
    fn on_get_state(&mut self) -> StateSnapshot {
        StateSnapshot::default()
    }

    /// Restore the app's state from a previous snapshot.
    fn on_set_state(
        &mut self,
        _user_state: &Value,
        _derived: &Value,
        _session: &Value,
        _persistent: &Value,
    ) {
    }

    /// Minimum pane size (logical pixels) below which the SDK auto-fallback
    /// frame is rendered instead of calling [`on_render`].
    ///
    /// Defaults to `(0.0, 0.0)` (no floor). When either dimension is
    /// non-zero, the SDK draws a built-in "pane too small" frame — a
    /// background rect, centered `"min size: W x H"` label, directional
    /// arrow indicating which axes need to grow, and a dim `"current: w x h"`
    /// subtitle — bypassing `on_render` entirely.
    ///
    /// If this method returns `(0.0, 0.0)` and the app's `manifest.toml`
    /// declares `[app.layout]`, the manifest values are used instead.
    /// Return a non-zero tuple to override the manifest at runtime.
    fn min_size(&self) -> (f32, f32) {
        (0.0, 0.0)
    }
}

// ── Manifest layout reader ────────────────────────────────────────────────────

/// Read `min_width` / `min_height` from the app's `manifest.toml`
/// `[app.layout]` table, if present.
///
/// Lookup order:
///  1. `$PLEXI_APPS_DIR/$PLEXI_APP_ID/manifest.toml` (set by the host)
///  2. A `manifest.toml` sitting next to the running executable
///  3. `./manifest.toml` (cwd)
///
/// Returns `(0.0, 0.0)` if no manifest is found or the table is absent.
/// Parsing is best-effort and hand-rolled (no extra deps) — only a flat
/// `[app.layout]` table with scalar `min_width` / `min_height` is
/// recognized.
pub fn load_manifest_layout() -> (f32, f32) {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let (Ok(dir), Ok(id)) = (env::var("PLEXI_APPS_DIR"), env::var("PLEXI_APP_ID")) {
        candidates.push(PathBuf::from(dir).join(id).join("manifest.toml"));
    }
    if let Ok(exe) = env::current_exe() {
        if let Some(parent) = exe.parent() {
            candidates.push(parent.join("manifest.toml"));
        }
    }
    candidates.push(PathBuf::from("manifest.toml"));

    for path in candidates {
        if let Ok(text) = read_to_string(&path) {
            if let Some(layout) = parse_app_layout_table(&text) {
                return layout;
            }
        }
    }
    (0.0, 0.0)
}

/// Minimal TOML scanner for the `[app.layout]` table. Recognizes
/// `min_width` and `min_height` as scalar numbers (int or float). All
/// other lines and tables are ignored. Returns `None` if the table is
/// not present.
fn parse_app_layout_table(text: &str) -> Option<(f32, f32)> {
    let mut in_layout = false;
    let mut found = false;
    let mut mw: f32 = 0.0;
    let mut mh: f32 = 0.0;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let name = line[1..line.len() - 1].trim();
            in_layout = name == "app.layout";
            if in_layout {
                found = true;
            }
            continue;
        }
        if !in_layout {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let key = k.trim();
            // Strip inline comments and quotes; keep only the numeric prefix.
            let mut val = v.trim().to_string();
            if let Some(idx) = val.find('#') {
                val.truncate(idx);
            }
            let val = val.trim().trim_matches('"').trim();
            if let Ok(n) = val.parse::<f32>() {
                match key {
                    "min_width" => mw = n,
                    "min_height" => mh = n,
                    _ => {}
                }
            }
        }
    }
    if found { Some((mw, mh)) } else { None }
}

/// Emit a built-in "pane too small" fallback frame directly to stdout.
/// Skips [`App::on_render`] entirely so user code never sees an
/// undersized pane.
fn render_min_size_fallback(width: f32, height: f32, min_w: f32, min_h: f32) {
    let bg = "#0a0a0a";
    let fg = "#888888";
    let accent = "#ffffff";

    let needs_w = width < min_w;
    let needs_h = height < min_h;
    let arrow = if needs_w && needs_h {
        "\u{2198}" // ↘
    } else if needs_w {
        "\u{2192}" // →
    } else if needs_h {
        "\u{2193}" // ↓
    } else {
        ""
    };

    let label = format!("min size: {} x {}", min_w as i32, min_h as i32);
    let current = format!("current: {} x {}", width as i32, height as i32);

    // Rough centering — no text metrics available in the SDK, so estimate
    // a per-glyph advance of 0.55 * font_size.
    let label_size: f32 = 14.0;
    let current_size: f32 = 11.0;
    let arrow_size: f32 = 32.0;
    let label_w = label.chars().count() as f32 * label_size * 0.55;
    let current_w = current.chars().count() as f32 * current_size * 0.55;
    let arrow_w = arrow.chars().count() as f32 * arrow_size * 0.55;

    let cx = width / 2.0;
    let cy = height / 2.0;
    let label_x = (cx - label_w / 2.0).max(0.0);
    let current_x = (cx - current_w / 2.0).max(0.0);
    let arrow_x = (cx - arrow_w / 2.0).max(0.0);
    let label_y = (cy - 30.0).max(0.0);
    let arrow_y = cy;
    let current_y = cy + 40.0;

    let mut cmds: Vec<DrawCommand> = Vec::with_capacity(5);
    cmds.push(DrawCommand::Rect {
        x: 0.0,
        y: 0.0,
        w: width,
        h: height,
        fill: bg.to_string(),
        radius: 0.0,
    });
    cmds.push(DrawCommand::Text {
        x: label_x,
        y: label_y,
        text: label,
        size: label_size,
        color: fg.to_string(),
        monospace: false,
        bold: true,
        align: None,
    });
    if !arrow.is_empty() {
        cmds.push(DrawCommand::Text {
            x: arrow_x,
            y: arrow_y,
            text: arrow.to_string(),
            size: arrow_size,
            color: accent.to_string(),
            monospace: false,
            bold: false,
            align: None,
        });
    }
    cmds.push(DrawCommand::Text {
        x: current_x,
        y: current_y,
        text: current,
        size: current_size,
        color: fg.to_string(),
        monospace: false,
        bold: false,
        align: None,
    });
    cmds.push(DrawCommand::FrameDone);

    let stdout = io::stdout();
    let mut out = stdout.lock();
    for cmd in &cmds {
        if let Ok(s) = serde_json::to_string(cmd) {
            let _ = writeln!(out, "{}", s);
        }
    }
    let _ = out.flush();
}

// ── Event loop ────────────────────────────────────────────────────────────────

/// Start the Plexi event loop. Blocks until Plexi sends `Shutdown`.
pub fn run(app: &mut dyn App) {
    let stdin = io::stdin();
    let mut emitter = Emitter::new();
    let app_id = emitter.app_id().to_string();

    // Resolve effective min-size: programmatic override wins over manifest.
    let (min_w, min_h) = {
        let (tw, th) = app.min_size();
        if tw > 0.0 || th > 0.0 {
            (tw, th)
        } else {
            load_manifest_layout()
        }
    };

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let event: PlexiEvent = match serde_json::from_str(line) {
            Ok(e) => e,
            Err(_) => continue,
        };

        match event {
            PlexiEvent::Init { .. } => {}
            PlexiEvent::Resize { width, height } => {
                app.on_resize(width, height);
            }
            PlexiEvent::Render { width, height, delta_time } => {
                // Auto min-size fallback: if the pane is smaller than the
                // declared floor on either axis, draw the built-in "too
                // small" frame and skip user rendering entirely.
                if (min_w > 0.0 || min_h > 0.0) && (width < min_w || height < min_h) {
                    render_min_size_fallback(width, height, min_w, min_h);
                    continue;
                }
                let mut ctx = RenderContext::new(width, height, delta_time, app_id.clone());
                app.on_render(&mut ctx);
                ctx.flush();
            }
            PlexiEvent::Key { key, modifiers } => {
                app.on_key(&key, &modifiers, &mut emitter);
            }
            PlexiEvent::Click { x, y, button } => {
                app.on_click(x, y, &button, &mut emitter);
            }
            PlexiEvent::MouseDown { x, y, button } => {
                app.on_mouse_down(x, y, &button, &mut emitter);
            }
            PlexiEvent::MouseUp { x, y, button } => {
                app.on_mouse_up(x, y, &button, &mut emitter);
            }
            PlexiEvent::MouseMove { x, y } => {
                app.on_mouse_move(x, y, &mut emitter);
            }
            PlexiEvent::Scroll { x, y, delta_x, delta_y } => {
                app.on_scroll(x, y, delta_x, delta_y, &mut emitter);
            }
            PlexiEvent::Command { text } => {
                app.on_command(&text, &mut emitter);
            }
            PlexiEvent::Drop { target_id, paths } => {
                app.on_drop(&target_id, &paths, &mut emitter);
            }
            PlexiEvent::GetState => {
                let snap = app.on_get_state();
                let cmd = DrawCommand::State {
                    user_state: snap.user_state,
                    derived: snap.derived,
                    session: snap.session,
                    persistent: snap.persistent,
                };
                let stdout = io::stdout();
                let mut out = stdout.lock();
                if let Ok(s) = serde_json::to_string(&cmd) {
                    let _ = writeln!(out, "{}", s);
                }
                let _ = out.flush();
            }
            PlexiEvent::SetState { user_state, derived, session, persistent } => {
                app.on_set_state(&user_state, &derived, &session, &persistent);
            }
            PlexiEvent::Shutdown => break,
        }
    }
}

// ── Tiny helpers (no extra deps) ─────────────────────────────────────────────

/// Best-effort RFC 3339 / ISO 8601 UTC timestamp without pulling in `chrono`.
fn now_rfc3339() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let dur = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = dur.as_secs() as i64;
    let micros = dur.subsec_micros();
    // Civil date conversion (Howard Hinnant algorithm).
    let z = secs.div_euclid(86_400) + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    let secs_of_day = secs.rem_euclid(86_400) as u32;
    let hour = secs_of_day / 3_600;
    let minute = (secs_of_day % 3_600) / 60;
    let second = secs_of_day % 60;
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:06}+00:00",
        y, m, d, hour, minute, second, micros
    )
}

/// Generate a UUID-shaped string. Not cryptographically random — uses
/// nanoseconds + process id for uniqueness within an app session.
fn generate_operation_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id() as u128;
    let mix = nanos.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(pid);
    format!(
        "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
        (mix >> 96) as u32,
        ((mix >> 80) & 0xFFFF) as u16,
        ((mix >> 64) & 0xFFFF) as u16,
        ((mix >> 48) & 0xFFFF) as u16,
        (mix & 0xFFFF_FFFF_FFFF) as u64,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn deserializes_scroll_event() {
        let json = r#"{"type":"scroll","x":10.0,"y":20.0,"delta_x":1.5,"delta_y":-3.0}"#;
        let event: PlexiEvent = serde_json::from_str(json).unwrap();
        match event {
            PlexiEvent::Scroll { x, y, delta_x, delta_y } => {
                assert_eq!(x, 10.0);
                assert_eq!(y, 20.0);
                assert_eq!(delta_x, 1.5);
                assert_eq!(delta_y, -3.0);
            }
            _ => panic!("expected Scroll"),
        }
    }

    #[test]
    fn deserializes_mouse_down_event() {
        let json = r#"{"type":"mouse_down","x":5.0,"y":6.0,"button":"left"}"#;
        let event: PlexiEvent = serde_json::from_str(json).unwrap();
        match event {
            PlexiEvent::MouseDown { x, y, button } => {
                assert_eq!(x, 5.0);
                assert_eq!(y, 6.0);
                assert_eq!(button, "left");
            }
            _ => panic!("expected MouseDown"),
        }
    }

    #[test]
    fn deserializes_drop_event() {
        let json =
            r#"{"type":"drop","target_id":"zone-1","paths":["/a.png","/b.png"]}"#;
        let event: PlexiEvent = serde_json::from_str(json).unwrap();
        match event {
            PlexiEvent::Drop { target_id, paths } => {
                assert_eq!(target_id, "zone-1");
                assert_eq!(paths, vec!["/a.png".to_string(), "/b.png".to_string()]);
            }
            _ => panic!("expected Drop"),
        }
    }

    #[test]
    fn deserializes_get_state_event() {
        let json = r#"{"type":"get_state"}"#;
        let event: PlexiEvent = serde_json::from_str(json).unwrap();
        assert!(matches!(event, PlexiEvent::GetState));
    }

    #[test]
    fn deserializes_set_state_event() {
        let json = r#"{"type":"set_state","user_state":{"cursor":3},"derived":{},"session":{},"persistent":{}}"#;
        let event: PlexiEvent = serde_json::from_str(json).unwrap();
        match event {
            PlexiEvent::SetState { user_state, .. } => {
                assert_eq!(user_state, json!({"cursor": 3}));
            }
            _ => panic!("expected SetState"),
        }
    }

    #[test]
    fn serializes_cost_report() {
        let cmd = DrawCommand::CostReport {
            app_id: "demo".to_string(),
            service: "anthropic".to_string(),
            model: "claude-sonnet-4".to_string(),
            input_tokens: 100,
            output_tokens: 50,
            cost_usd: 0.001,
            operation_id: Some("op-1".to_string()),
            timestamp: Some("2026-01-01T00:00:00+00:00".to_string()),
        };
        let v: Value = serde_json::to_value(&cmd).unwrap();
        assert_eq!(v["type"], "cost_report");
        assert_eq!(v["app_id"], "demo");
        assert_eq!(v["service"], "anthropic");
        assert_eq!(v["input_tokens"], 100);
        assert_eq!(v["cost_usd"], 0.001);
        assert_eq!(v["operation_id"], "op-1");
    }

    #[test]
    fn serializes_notification() {
        let cmd = DrawCommand::Notification {
            priority: 2,
            title: "Build done".to_string(),
            body: Some("ok".to_string()),
            source_app: "ci".to_string(),
        };
        let v: Value = serde_json::to_value(&cmd).unwrap();
        assert_eq!(v["type"], "notification");
        assert_eq!(v["priority"], 2);
        assert_eq!(v["title"], "Build done");
        assert_eq!(v["body"], "ok");
        assert_eq!(v["source_app"], "ci");
    }

    #[test]
    fn serializes_state_response() {
        let cmd = DrawCommand::State {
            user_state: json!({"cursor": 0}),
            derived: json!({}),
            session: json!({"scroll": 12}),
            persistent: json!({}),
        };
        let v: Value = serde_json::to_value(&cmd).unwrap();
        assert_eq!(v["type"], "state");
        assert_eq!(v["user_state"]["cursor"], 0);
        assert_eq!(v["session"]["scroll"], 12);
    }

    #[test]
    fn serializes_set_cursor_and_mouse_tracking() {
        let cur = serde_json::to_value(&DrawCommand::SetCursor {
            cursor: "pointer".to_string(),
        })
        .unwrap();
        assert_eq!(cur["type"], "set_cursor");
        assert_eq!(cur["cursor"], "pointer");

        let mt = serde_json::to_value(&DrawCommand::MouseTracking { enabled: true }).unwrap();
        assert_eq!(mt["type"], "mouse_tracking");
        assert_eq!(mt["enabled"], true);
    }

    #[test]
    fn serializes_drop_target() {
        let cmd = DrawCommand::DropTarget {
            id: "zone".to_string(),
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 50.0,
            accept: vec!["png".to_string(), "jpg".to_string()],
            label: Some("drop here".to_string()),
        };
        let v: Value = serde_json::to_value(&cmd).unwrap();
        assert_eq!(v["type"], "drop_target");
        assert_eq!(v["id"], "zone");
        assert_eq!(v["accept"][0], "png");
        assert_eq!(v["label"], "drop here");
    }

    #[test]
    fn serializes_spawn_app_cols_split() {
        let cmd = DrawCommand::SpawnApp {
            app_id: "text-editor".to_string(),
            args: vec!["/tmp/notes.txt".to_string()],
            parent: SpawnParent::SelfPane,
            layout: SpawnLayout::Cols { slot: 1, ratio: 0.5 },
            lifecycle: SpawnLifecycle::Cascade,
            linked: true,
            wire_channels: vec!["file_buffer".to_string()],
        };
        let v = serde_json::to_value(&cmd).unwrap();
        assert_eq!(v["type"], "spawn_app");
        assert_eq!(v["app_id"], "text-editor");
        assert_eq!(v["args"][0], "/tmp/notes.txt");
        assert_eq!(v["parent"], "self");
        assert_eq!(v["layout"]["kind"], "cols");
        assert_eq!(v["layout"]["slot"], 1);
        assert_eq!(v["layout"]["ratio"], 0.5);
        assert_eq!(v["lifecycle"], "cascade");
        assert_eq!(v["linked"], true);
        assert_eq!(v["wire_channels"][0], "file_buffer");
    }

    #[test]
    fn deserializes_spawn_app_minimal() {
        let json = r#"{"type":"spawn_app","app_id":"permissions"}"#;
        let cmd: DrawCommand = serde_json::from_str(json).unwrap();
        match cmd {
            DrawCommand::SpawnApp {
                app_id,
                parent,
                layout,
                lifecycle,
                linked,
                ..
            } => {
                assert_eq!(app_id, "permissions");
                assert_eq!(parent, SpawnParent::SelfPane);
                assert!(matches!(layout, SpawnLayout::Fill));
                assert_eq!(lifecycle, SpawnLifecycle::Cascade);
                assert!(linked);
            }
            _ => panic!("expected SpawnApp"),
        }
    }

    #[test]
    fn spawn_parent_mark_roundtrip() {
        let v: SpawnParent = serde_json::from_str(r#""mark:tree""#).unwrap();
        assert_eq!(v, SpawnParent::NamedMark("tree".to_string()));
        let s = serde_json::to_string(&v).unwrap();
        assert_eq!(s, r#""mark:tree""#);
    }

    #[test]
    fn serializes_log() {
        let v = serde_json::to_value(&DrawCommand::Log {
            level: "warn".to_string(),
            message: "uh oh".to_string(),
        })
        .unwrap();
        assert_eq!(v["type"], "log");
        assert_eq!(v["level"], "warn");
        assert_eq!(v["message"], "uh oh");
    }

    // ── Breakpoint dispatcher ────────────────────────────────────────────

    fn make_ctx(width: f32, height: f32) -> RenderContext {
        RenderContext::new(width, height, 0.0, String::new())
    }

    #[test]
    fn pick_breakpoint_picks_largest_match() {
        let bps: &[(f32, f32)] = &[(800.0, 500.0), (400.0, 0.0), (0.0, 0.0)];
        assert_eq!(pick_breakpoint(1200.0, 800.0, bps), Some(0));
        assert_eq!(pick_breakpoint(600.0, 600.0, bps), Some(1));
        assert_eq!(pick_breakpoint(200.0, 200.0, bps), Some(2));
    }

    #[test]
    fn pick_breakpoint_returns_none_when_nothing_fits() {
        let bps: &[(f32, f32)] = &[(1000.0, 1000.0), (800.0, 600.0)];
        assert_eq!(pick_breakpoint(400.0, 300.0, bps), None);
    }

    #[test]
    fn breakpoint_set_dispatch_picks_largest_match() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let fired: Rc<RefCell<Vec<&'static str>>> = Rc::new(RefCell::new(Vec::new()));
        let tag = |s: &'static str| {
            let fired = Rc::clone(&fired);
            move |_ctx: &mut RenderContext| fired.borrow_mut().push(s)
        };

        let mut set = BreakpointSet::new()
            .breakpoint(800.0, 500.0, tag("full"))
            .breakpoint(400.0, 0.0, tag("compact"))
            .fallback(tag("fallback"));

        let mut ctx = make_ctx(1200.0, 800.0);
        assert!(set.dispatch(&mut ctx));
        assert_eq!(&*fired.borrow(), &["full"]);
        fired.borrow_mut().clear();

        let mut ctx = make_ctx(600.0, 600.0);
        assert!(set.dispatch(&mut ctx));
        assert_eq!(&*fired.borrow(), &["compact"]);
        fired.borrow_mut().clear();

        let mut ctx = make_ctx(200.0, 200.0);
        assert!(set.dispatch(&mut ctx));
        assert_eq!(&*fired.borrow(), &["fallback"]);
    }

    #[test]
    fn breakpoint_set_uses_zero_fallback_when_no_match() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let fired: Rc<RefCell<bool>> = Rc::new(RefCell::new(false));
        let fired_clone = Rc::clone(&fired);

        let mut set = BreakpointSet::new()
            .breakpoint(1000.0, 800.0, |_| {})
            .fallback(move |_| *fired_clone.borrow_mut() = true);

        let mut ctx = make_ctx(500.0, 400.0);
        assert!(set.dispatch(&mut ctx));
        assert!(*fired.borrow());
    }

    #[test]
    fn breakpoint_set_returns_false_when_nothing_fits_and_no_fallback() {
        let mut set = BreakpointSet::new().breakpoint(1000.0, 800.0, |_| {});
        let mut ctx = make_ctx(500.0, 400.0);
        assert!(!set.dispatch(&mut ctx));
    }

    // ── Manifest layout parser ───────────────────────────────────────────

    #[test]
    fn parses_app_layout_table() {
        let toml = r#"
[app]
id = "demo"

[app.layout]
min_width = 400
min_height = 200
"#;
        assert_eq!(parse_app_layout_table(toml), Some((400.0, 200.0)));
    }

    #[test]
    fn parses_app_layout_with_comments_and_floats() {
        let toml = r#"
[app.layout]
min_width  = 640.5  # wider than the sidebar
min_height = 360    # golden ratio of something
"#;
        assert_eq!(parse_app_layout_table(toml), Some((640.5, 360.0)));
    }

    #[test]
    fn parse_app_layout_returns_none_when_absent() {
        let toml = r#"
[app]
id = "demo"

[app.capabilities]
network = false
"#;
        assert_eq!(parse_app_layout_table(toml), None);
    }
}
