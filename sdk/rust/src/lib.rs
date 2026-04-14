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
use std::fs::{create_dir_all, OpenOptions};
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
    FrameDone,
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
            monospace: false, bold: false,
        });
        self
    }

    /// Draw monospace text (uses terminal font).
    pub fn text_mono(&mut self, x: f32, y: f32, text: &str, size: f32, color: &str) -> &mut Self {
        self.commands.push(DrawCommand::Text {
            x, y, text: text.to_string(), size, color: color.to_string(),
            monospace: true, bold: false,
        });
        self
    }

    /// Draw bold text.
    pub fn text_bold(&mut self, x: f32, y: f32, text: &str, size: f32, color: &str) -> &mut Self {
        self.commands.push(DrawCommand::Text {
            x, y, text: text.to_string(), size, color: color.to_string(),
            monospace: false, bold: true,
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
}

// ── Event loop ────────────────────────────────────────────────────────────────

/// Start the Plexi event loop. Blocks until Plexi sends `Shutdown`.
pub fn run(app: &mut dyn App) {
    let stdin = io::stdin();
    let mut emitter = Emitter::new();
    let app_id = emitter.app_id().to_string();

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
}
