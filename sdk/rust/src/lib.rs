//! Plexi SDK for Rust
//!
//! Build Plexi apps in Rust by implementing the [`App`] trait and calling [`run`].
//!
//! # Example
//!
//! ```rust,no_run
//! use plexi_sdk::{App, ListItem, Modifiers, MouseButton, RenderContext, run};
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
//!             // egui serializes Key enum as PascalCase: Key::J → "J"
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
use std::io::{self, BufRead, Write};

// ── Inbound events (Plexi → app) ─────────────────────────────────────────────

#[derive(Deserialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PlexiEvent {
    Init { width: f32, height: f32, pixels_per_point: f32 },
    Render { width: f32, height: f32 },
    Resize { width: f32, height: f32 },
    Key { key: String, modifiers: Modifiers },
    Click { x: f32, y: f32, button: MouseButton },
    Command { text: String },
    Shutdown,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub cmd: bool,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case")]
pub enum MouseButton {
    Primary,
    Secondary,
}

// ── Outbound draw commands (app → Plexi) ─────────────────────────────────────

#[derive(Serialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
enum DrawCommand {
    Rect { x: f32, y: f32, w: f32, h: f32, fill: String, radius: f32 },
    Text { x: f32, y: f32, text: String, size: f32, color: String, monospace: bool, bold: bool },
    Line { x1: f32, y1: f32, x2: f32, y2: f32, color: String, width: f32 },
    List { items: Vec<ListItem>, selected: usize, item_height: f32 },
    RunInTerminal { command: String },
    Cd { path: String },
    FrameDone,
}

#[derive(Serialize, Debug, Clone)]
pub struct ListItem {
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secondary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
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

    pub fn dir(mut self) -> Self {
        self.is_dir = true;
        self
    }
}

// ── Emitter: write commands outside a render frame ───────────────────────────

pub struct Emitter;

impl Emitter {
    /// Execute a shell command in the linked terminal immediately.
    pub fn run_in_terminal(&self, command: &str) {
        let cmd = DrawCommand::RunInTerminal { command: command.to_string() };
        let stdout = io::stdout();
        let mut out = stdout.lock();
        let _ = writeln!(out, "{}", serde_json::to_string(&cmd).unwrap_or_default());
        let _ = out.flush();
    }

    /// Change the linked terminal's working directory immediately.
    pub fn cd(&self, path: &str) {
        let cmd = DrawCommand::Cd { path: path.to_string() };
        let stdout = io::stdout();
        let mut out = stdout.lock();
        let _ = writeln!(out, "{}", serde_json::to_string(&cmd).unwrap_or_default());
        let _ = out.flush();
    }
}

// ── RenderContext ─────────────────────────────────────────────────────────────

pub struct RenderContext {
    pub width: f32,
    pub height: f32,
    commands: Vec<DrawCommand>,
}

impl RenderContext {
    fn new(width: f32, height: f32) -> Self {
        Self { width, height, commands: Vec::new() }
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

    /// High-level scrollable list. Plexi handles layout, scrolling, and highlight.
    pub fn list(&mut self, items: Vec<ListItem>, selected: usize, item_height: f32) -> &mut Self {
        self.commands.push(DrawCommand::List { items, selected, item_height });
        self
    }

    /// Queue a terminal command to emit at end of this frame.
    pub fn run_in_terminal(&mut self, command: &str) -> &mut Self {
        self.commands.push(DrawCommand::RunInTerminal { command: command.to_string() });
        self
    }

    /// Queue a cd for the linked terminal at end of this frame.
    pub fn cd(&mut self, path: &str) -> &mut Self {
        self.commands.push(DrawCommand::Cd { path: path.to_string() });
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

/// Implement this trait to build a Plexi app. All methods have default no-op implementations.
pub trait App {
    fn on_render(&mut self, ctx: &mut RenderContext) { let _ = ctx; }
    fn on_key(&mut self, _key: &str, _mods: &Modifiers, _emit: &mut Emitter) {}
    fn on_click(&mut self, _x: f32, _y: f32, _button: &MouseButton, _emit: &mut Emitter) {}
    fn on_command(&mut self, _text: &str, _emit: &mut Emitter) {}
    fn on_resize(&mut self, _width: f32, _height: f32) {}
}

// ── Event loop ────────────────────────────────────────────────────────────────

/// Start the Plexi event loop. Blocks until Plexi sends `Shutdown`.
pub fn run(app: &mut dyn App) {
    let stdin = io::stdin();
    let mut emitter = Emitter;

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let line = line.trim();
        if line.is_empty() { continue; }

        let event: PlexiEvent = match serde_json::from_str(line) {
            Ok(e) => e,
            Err(_) => continue,
        };

        match event {
            PlexiEvent::Init { .. } => {}
            PlexiEvent::Resize { width, height } => {
                app.on_resize(width, height);
            }
            PlexiEvent::Render { width, height } => {
                let mut ctx = RenderContext::new(width, height);
                app.on_render(&mut ctx);
                ctx.flush();
            }
            PlexiEvent::Key { key, modifiers } => {
                app.on_key(&key, &modifiers, &mut emitter);
            }
            PlexiEvent::Click { x, y, button } => {
                app.on_click(x, y, &button, &mut emitter);
            }
            PlexiEvent::Command { text } => {
                app.on_command(&text, &mut emitter);
            }
            PlexiEvent::Shutdown => break,
        }
    }
}
