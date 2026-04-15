"""
plexi_sdk.py — Plexi external app SDK (Python)

Zero dependencies, pure stdlib. Copy this file into your app directory.

Protocol: newline-delimited JSON over stdin/stdout.
  - Plexi sends PlexiEvent objects  {"type": "render", "width": ..., "height": ...}
  - App responds with DrawCommand objects {"type": "rect", ...} + {"type": "frame_done"}

Usage:

    from plexi_sdk import App

    app = App()

    @app.on_render
    def render(ctx):
        ctx.rect(0, 0, ctx.width, ctx.height, fill="#1e1e2e")
        ctx.text(20, 20, "Hello Plexi!", size=16, color="#cdd6f4")

    app.run()

Key handlers can emit terminal commands via the Emitter passed as the second arg:

    @app.on_key
    def on_key(key, mods, emit):
        if key == "Enter":
            emit.run_in_terminal("echo hello")

SDK version: 0.4.0
Protocol version: 2
"""
from __future__ import annotations

import json
import sys
from typing import Callable, Optional


class OpenIntent:
    """Structured spawn intent passed at app startup via the Init event."""

    def __init__(self, kind_type: str, **kwargs):
        self.kind_type = kind_type
        self.kwargs = kwargs

    @classmethod
    def file(cls, path: str, start_line: Optional[int] = None, end_line: Optional[int] = None) -> "OpenIntent":
        """Open a file, optionally at a specific line range."""
        r: dict = {"kind": "file", "path": path}
        if start_line is not None:
            r["range"] = {"start_line": start_line, "end_line": end_line or start_line}
        return cls("file", **r)

    @classmethod
    def prompt(cls, text: str, model_hint: Optional[str] = None) -> "OpenIntent":
        """Open with a text prompt."""
        d: dict = {"kind": "prompt", "text": text}
        if model_hint:
            d["model_hint"] = model_hint
        return cls("prompt", **d)

    @classmethod
    def bare(cls) -> "OpenIntent":
        """Open with no structured intent."""
        return cls("bare", kind="bare")

    @classmethod
    def resume(cls, snapshot_key: str) -> "OpenIntent":
        """Resume from a snapshot key."""
        return cls("resume", kind="resume", snapshot_key=snapshot_key)

    @classmethod
    def from_dict(cls, d: dict) -> "OpenIntent":
        """Deserialize from the Init event payload."""
        kind_type = d.get("kind", "bare")
        return cls(kind_type, **d)

    def to_dict(self) -> dict:
        return self.kwargs


class Emitter:
    """Emit commands to Plexi immediately (outside a render frame)."""

    def _write(self, cmd: dict):
        print(json.dumps(cmd), flush=True)

    def run_in_terminal(self, command: str):
        """Execute a shell command in the linked terminal."""
        self._write({"type": "run_in_terminal", "command": command})

    def cd(self, path: str):
        """Change the linked terminal's working directory."""
        self._write({"type": "cd", "path": path})

    def log(self, level: str, message: str):
        """Forward a log message to Plexi's logger (level: error|warn|info|debug)."""
        self._write({"type": "log", "level": level, "message": message})

    def info(self, message: str):
        """Log at info level."""
        self.log("info", message)

    def warn(self, message: str):
        """Log at warn level."""
        self.log("warn", message)

    def error(self, message: str):
        """Log at error level."""
        self.log("error", message)

    def debug(self, message: str):
        """Log at debug level."""
        self.log("debug", message)

    def run_create(self, head_task: str, payload: Optional[dict] = None,
                   parent_run_id: Optional[str] = None,
                   notification_title: Optional[str] = None):
        """Create a Run. Plexi will respond with a RunCreated event containing the run_id."""
        cmd: dict = {"type": "run_create", "head_task": head_task, "payload": payload or {}}
        if parent_run_id:
            cmd["parent_run_id"] = parent_run_id
        if notification_title:
            cmd["notification_title"] = notification_title
        self._write(cmd)

    def run_update(self, run_id: str, status: dict, head_task: Optional[str] = None,
                   payload: Optional[dict] = None):
        """Update a Run's status. status is a dict with a 'status' key matching RunStatus."""
        cmd: dict = {"type": "run_update", "run_id": run_id, "status": status}
        if head_task:
            cmd["head_task"] = head_task
        if payload:
            cmd["payload"] = payload
        self._write(cmd)

    def run_complete(self, run_id: str, outcome: str = "success", error: Optional[str] = None):
        """Complete a Run. outcome: 'success' | 'failed' | 'cancelled'."""
        o: dict = {"outcome": outcome}
        if error:
            o["error"] = error
        self._write({"type": "run_complete", "run_id": run_id, "outcome": o})

    def event_subscribe(self, kinds: Optional[list] = None, scope: str = "workspace"):
        """Subscribe to bus events. Handler registered via @app.on_event."""
        self._write({"type": "event_subscribe", "kinds": kinds or [], "scope": scope})

    def notify(self, id: str, title: str, body: Optional[str] = None,
               urgency: Optional[str] = None, run_id: Optional[str] = None,
               action: Optional[dict] = None):
        """Emit a notification."""
        cmd: dict = {"type": "notification", "id": id, "title": title}
        if body:
            cmd["body"] = body
        if urgency:
            cmd["urgency"] = urgency
        if run_id:
            cmd["run_id"] = run_id
        if action:
            cmd["action"] = action
        self._write(cmd)

    def pipe_write(self, channel: str, value):
        """Write data to a named pipe channel."""
        self._write({"type": "pipe_write", "channel": channel, "value": value})


class RenderContext:
    """
    Passed to the on_render handler. Accumulates draw commands, then flushes.

    All coordinates are in logical pixels within the app surface.
    """

    def __init__(self, width: float, height: float):
        self.width = width
        self.height = height
        self._commands: list = []

    def rect(self, x: float, y: float, w: float, h: float, fill: str, radius: float = 0.0):
        """Fill a rectangle."""
        self._commands.append({
            "type": "rect", "x": x, "y": y, "w": w, "h": h,
            "fill": fill, "radius": radius,
        })

    def text(self, x: float, y: float, text: str, size: float, color: str,
             monospace: bool = False, bold: bool = False):
        """Draw text at a position."""
        self._commands.append({
            "type": "text", "x": x, "y": y, "text": text, "size": size,
            "color": color, "monospace": monospace, "bold": bold,
        })

    def line(self, x1: float, y1: float, x2: float, y2: float,
             color: str, width: float = 1.0):
        """Draw a horizontal or diagonal line."""
        self._commands.append({
            "type": "line", "x1": x1, "y1": y1, "x2": x2, "y2": y2,
            "color": color, "width": width,
        })

    def list(self, items: list, selected: int = 0, item_height: float = 40.0):
        """
        High-level scrollable list. Plexi handles layout, scroll, and selection highlight.

        Each item is a dict with keys:
            label      (str)           — primary text
            secondary  (str | None)    — dimmed subtitle
            is_dir     (bool)          — show folder indicator
        """
        self._commands.append({
            "type": "list", "items": items,
            "selected": selected, "item_height": item_height,
        })

    def run_in_terminal(self, command: str):
        """Queue a terminal command to run at end of this frame."""
        self._commands.append({"type": "run_in_terminal", "command": command})

    def cd(self, path: str):
        """Queue a cd command for the linked terminal at end of this frame."""
        self._commands.append({"type": "cd", "path": path})

    def log(self, level: str, message: str):
        """Forward a log message to Plexi's logger (level: error|warn|info|debug)."""
        self._commands.append({"type": "log", "level": level, "message": message})

    def info(self, message: str):
        """Log at info level."""
        self.log("info", message)

    def warn(self, message: str):
        """Log at warn level."""
        self.log("warn", message)

    def error(self, message: str):
        """Log at error level."""
        self.log("error", message)

    def debug(self, message: str):
        """Log at debug level."""
        self.log("debug", message)

    def _flush(self):
        for cmd in self._commands:
            print(json.dumps(cmd), flush=True)
        print(json.dumps({"type": "frame_done"}), flush=True)
        self._commands.clear()


class App:
    """
    Base class for Plexi apps. Register event handlers via decorators.

    Handlers:
        @app.on_init     fn(init_data: dict, emit: Emitter)   — v2: receives open_intent
        @app.on_render   fn(ctx: RenderContext)
        @app.on_key      fn(key: str, mods: dict, emit: Emitter)
        @app.on_click    fn(x: float, y: float, button: str, emit: Emitter)
        @app.on_command  fn(text: str, emit: Emitter)
        @app.on_resize   fn(width: float, height: float)
        @app.on_event    fn(event: dict, emit: Emitter)       — v2: bus events
        @app.on_run_created fn(run_id: str, emit: Emitter)    — v2: run lifecycle
    """

    def __init__(self):
        self.width: float = 800.0
        self.height: float = 600.0
        self.protocol_version: int = 1
        self.open_intent: Optional[OpenIntent] = None
        self._on_init: Optional[Callable] = None
        self._on_render: Optional[Callable] = None
        self._on_key: Optional[Callable] = None
        self._on_click: Optional[Callable] = None
        self._on_command: Optional[Callable] = None
        self._on_resize: Optional[Callable] = None
        self._on_event: Optional[Callable] = None
        self._on_run_created: Optional[Callable] = None
        self._emitter = Emitter()

    def on_init(self, fn: Callable) -> Callable:
        self._on_init = fn
        return fn

    def on_render(self, fn: Callable) -> Callable:
        self._on_render = fn
        return fn

    def on_key(self, fn: Callable) -> Callable:
        self._on_key = fn
        return fn

    def on_click(self, fn: Callable) -> Callable:
        self._on_click = fn
        return fn

    def on_command(self, fn: Callable) -> Callable:
        self._on_command = fn
        return fn

    def on_resize(self, fn: Callable) -> Callable:
        self._on_resize = fn
        return fn

    def on_event(self, fn: Callable) -> Callable:
        """Register a handler for bus events (EventData). fn(event: dict, emit: Emitter)."""
        self._on_event = fn
        return fn

    def on_run_created(self, fn: Callable) -> Callable:
        """Register a handler for RunCreated responses. fn(run_id: str, emit: Emitter)."""
        self._on_run_created = fn
        return fn

    def spawn_app(self, app_id: str, open_intent: Optional[OpenIntent] = None):
        """Spawn another Plexi app. Emits a spawn_app draw command."""
        cmd: dict = {"type": "spawn_app", "app_id": app_id}
        if open_intent is not None:
            cmd["open_intent"] = open_intent.to_dict()
        self._emitter._write(cmd)

    def run(self):
        """Start the event loop. Blocks until Plexi sends Shutdown."""
        sys.stdout.reconfigure(line_buffering=True)  # type: ignore[attr-defined]

        for line in sys.stdin:
            line = line.strip()
            if not line:
                continue

            try:
                event = json.loads(line)
            except json.JSONDecodeError:
                continue

            event_type = event.get("type", "")

            if event_type == "init":
                self.width = event.get("width", self.width)
                self.height = event.get("height", self.height)
                self.protocol_version = event.get("protocol_version", 1)
                raw_intent = event.get("open_intent")
                if raw_intent:
                    self.open_intent = OpenIntent.from_dict(raw_intent)
                if self._on_init:
                    self._on_init(event, self._emitter)

            elif event_type == "resize":
                self.width = event.get("width", self.width)
                self.height = event.get("height", self.height)
                if self._on_resize:
                    self._on_resize(self.width, self.height)

            elif event_type == "render":
                self.width = event.get("width", self.width)
                self.height = event.get("height", self.height)
                ctx = RenderContext(self.width, self.height)
                if self._on_render:
                    self._on_render(ctx)
                ctx._flush()

            elif event_type == "key":
                if self._on_key:
                    self._on_key(
                        event.get("key", ""),
                        event.get("modifiers", {}),
                        self._emitter,
                    )

            elif event_type == "click":
                if self._on_click:
                    self._on_click(
                        event.get("x", 0.0),
                        event.get("y", 0.0),
                        event.get("button", "primary"),
                        self._emitter,
                    )

            elif event_type == "command":
                if self._on_command:
                    self._on_command(event.get("text", ""), self._emitter)

            elif event_type == "run_created":
                if self._on_run_created:
                    self._on_run_created(event.get("run_id", ""), self._emitter)

            elif event_type == "event_data":
                if self._on_event:
                    self._on_event(event.get("event", {}), self._emitter)

            elif event_type == "shutdown":
                break
