#!/usr/bin/env python3
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

SDK version: 0.5.0
Protocol version: 2 (v2.1 — ui_primitives_v1)
"""
from __future__ import annotations

import json
import sys
from dataclasses import dataclass
from typing import Callable, Optional


@dataclass
class TextMetrics:
    """Result of a MeasureText request — exact font metrics from Plexi."""
    width: float
    height: float
    ascent: float


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
        self._measure_cache: dict = {}
        self._measure_req_id: int = 0
        self._time: float = 0.0
        self._pending_events: list = []

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

    def measure_text_exact(self, text: str, size: float, monospace: bool = False, bold: bool = False) -> TextMetrics:
        """
        Request exact text measurement from Plexi. Blocking — waits for TextMetrics reply.
        Results are cached per-frame (cache clears each frame).
        """
        cache_key = (text, size, monospace, bold)
        if cache_key in self._measure_cache:
            return self._measure_cache[cache_key]

        self._measure_req_id += 1
        req_id = self._measure_req_id

        # Flush current commands first so measure request is sent in-band.
        for cmd in self._commands:
            print(json.dumps(cmd), flush=True)
        self._commands.clear()

        print(json.dumps({
            "type": "measure_text",
            "request_id": req_id,
            "text": text,
            "size": size,
            "monospace": monospace,
            "bold": bold,
        }), flush=True)

        # Read stdin until we get the matching TextMetrics reply.
        # Any non-metric events that arrive during this wait are buffered
        # in _pending_events and replayed by the main loop after we return,
        # so they are never discarded.
        for line in sys.stdin:
            line = line.strip()
            if not line:
                continue
            try:
                event = json.loads(line)
            except json.JSONDecodeError:
                continue
            if event.get("type") == "text_metrics" and event.get("request_id") == req_id:
                metrics = TextMetrics(
                    width=event.get("width", 0.0),
                    height=event.get("height", 0.0),
                    ascent=event.get("ascent", 0.0),
                )
                self._measure_cache[cache_key] = metrics
                return metrics
            else:
                # Buffer this event — it will be replayed after measurement returns.
                self._pending_events.append(event)

        # Fallback if stdin closed (should not happen in normal operation).
        return TextMetrics(width=size * len(text) * 0.6, height=size, ascent=size * 0.8)

    def viewport(self, viewport_id: str, content_fn: Callable, zoom: float = 1.0,
                 pan: Optional[tuple] = None, x: Optional[float] = None,
                 y: Optional[float] = None, w: Optional[float] = None,
                 h: Optional[float] = None, min_zoom: float = 0.1,
                 max_zoom: float = 10.0, on_pan=None, on_zoom=None):
        """
        Render content inside a transformed viewport with zoom + pan.
        content_fn receives this RenderContext.
        """
        zoom = max(min_zoom, min(max_zoom, zoom))
        tx = pan[0] if pan else 0.0
        ty = pan[1] if pan else 0.0
        self._commands.append({
            "type": "push_transform",
            "scale_x": zoom,
            "scale_y": zoom,
            "translate_x": tx,
            "translate_y": ty,
        })
        content_fn(self)
        self._commands.append({"type": "pop_transform"})
        return {"x": x or 0, "y": y or 0, "w": w or self.width, "h": h or self.height}

    def text_input(self, input_id: str, value: str, on_change: Callable,
                   cursor: Optional[int] = None, placeholder: Optional[str] = None,
                   max_length: Optional[int] = None, size: Optional[float] = None,
                   x: float = 0, y: float = 0, w: Optional[float] = None,
                   bg: Optional[str] = None, fg: Optional[str] = None,
                   focused: bool = True) -> dict:
        """
        Draw a single-line text input widget. Returns the bounding rect dict.
        Cursor blinks at 1Hz based on accumulated _time.
        """
        font_size = size or 14.0
        width = w or (self.width - x * 2)
        height = font_size + 12.0
        bg_color = bg or "#2a2a3e"
        fg_color = fg or "#cdd6f4"
        placeholder_color = "#6c7086"

        # Background
        self.rect(x, y, width, height, fill=bg_color, radius=4.0)

        display_text = value if value else (placeholder or "")
        display_color = fg_color if value else placeholder_color
        self.text(x + 6, y + (height - font_size) / 2, display_text, size=font_size, color=display_color)

        # Cursor blink
        if focused and self._time % 1.0 < 0.5:
            cur_pos = cursor if cursor is not None else len(value)
            # Approximate cursor x position.
            cursor_x = x + 6 + cur_pos * font_size * 0.6
            cursor_h = font_size + 2
            cursor_y = y + (height - cursor_h) / 2
            self.rect(cursor_x, cursor_y, 2.0, cursor_h, fill=fg_color)

        return {"x": x, "y": y, "w": width, "h": height}

    def tabs(self, tab_id: str, tabs: list, selected: str,
             on_change=None, height: float = 36, x: float = 0,
             y: Optional[float] = None, w: Optional[float] = None) -> dict:
        """
        Draw a tab bar. tabs is a list of (key, label) tuples.
        selected is the key of the active tab. Returns bounding rect dict.
        """
        y_pos = y if y is not None else 0.0
        total_w = w or self.width
        tab_w = total_w / max(len(tabs), 1)
        accent = "#89b4fa"
        bg_selected = "#313244"
        bg_normal = "#1e1e2e"
        fg = "#cdd6f4"

        for i, (key, label) in enumerate(tabs):
            tx = x + i * tab_w
            is_sel = key == selected
            self.rect(tx, y_pos, tab_w, height, fill=bg_selected if is_sel else bg_normal)
            self.text(tx + tab_w / 2 - len(label) * 4, y_pos + (height - 14) / 2, label, size=14.0, color=fg)
            if is_sel:
                self.rect(tx, y_pos + height - 2, tab_w, 2.0, fill=accent)

        return {"x": x, "y": y_pos, "w": total_w, "h": height}

    def grid(self, grid_id: str, cols: int, rows: int, render_cell: Callable,
             x: Optional[float] = None, y: Optional[float] = None,
             w: Optional[float] = None, h: Optional[float] = None,
             gap: float = 4.0):
        """
        Draw a uniform grid. render_cell(ctx, col, row, cx, cy, cw, ch) is called per cell.
        """
        gx = x or 0.0
        gy = y or 0.0
        gw = w or self.width
        gh = h or self.height
        cell_w = (gw - gap * (cols - 1)) / max(cols, 1)
        cell_h = (gh - gap * (rows - 1)) / max(rows, 1)

        for row in range(rows):
            for col in range(cols):
                cx = gx + col * (cell_w + gap)
                cy = gy + row * (cell_h + gap)
                render_cell(self, col, row, cx, cy, cell_w, cell_h)

    def modal(self, modal_id: str, visible: bool, content_fn: Callable,
              width: float = 400, height: float = 200,
              backdrop_alpha: int = 128, on_dismiss=None):
        """
        Draw a centered modal dialog with a semi-transparent backdrop.
        content_fn(ctx, modal_x, modal_y, width, height) is called if visible.
        """
        if not visible:
            return

        # Backdrop
        alpha_hex = format(backdrop_alpha, '02x')
        self.rect(0, 0, self.width, self.height, fill=f"#000000{alpha_hex}")

        # Centered modal rect
        mx = (self.width - width) / 2
        my = (self.height - height) / 2
        self.rect(mx, my, width, height, fill="#1e1e2e", radius=8.0)

        content_fn(self, mx, my, width, height)

    def _flush(self):
        for cmd in self._commands:
            print(json.dumps(cmd), flush=True)
        print(json.dumps({"type": "frame_done"}), flush=True)
        self._commands.clear()
        # Do NOT clear _measure_cache or _time here — App.run() manages those.


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
        self._render_time: float = 0.0
        self._measure_req_id: int = 0
        self._pending_events: list = []
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

    def _dispatch_event(self, event: dict) -> bool:
        """Dispatch a single parsed event. Returns True if loop should break (shutdown)."""
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
            delta_time = event.get("delta_time", 0.016)
            self._render_time += delta_time
            ctx = RenderContext(self.width, self.height)
            ctx._time = self._render_time
            ctx._measure_cache = {}  # clear per-frame
            ctx._measure_req_id = self._measure_req_id
            ctx._pending_events = self._pending_events
            if self._on_render:
                self._on_render(ctx)
            self._measure_req_id = ctx._measure_req_id
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
            return True

        return False

    def run(self):
        """Start the event loop. Blocks until Plexi sends Shutdown."""
        sys.stdout.reconfigure(line_buffering=True)  # type: ignore[attr-defined]

        for line in sys.stdin:
            # Drain events buffered during any measure_text_exact call first.
            while self._pending_events:
                buffered = self._pending_events.pop(0)
                if self._dispatch_event(buffered):
                    return

            line = line.strip()
            if not line:
                continue

            try:
                event = json.loads(line)
            except json.JSONDecodeError:
                continue

            if self._dispatch_event(event):
                break
