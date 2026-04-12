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

State management (Plexi handles undo/redo/save):

    @app.on_get_state
    def get_state():
        return {
            "user_state": {"cursor": 0, "selected": None},
            "derived": {},
            "session": {"scroll_offset": 120},
            "persistent": {"bookmarks": [1, 5, 9]},
        }

    @app.on_set_state
    def set_state(state):
        cursor = state["user_state"].get("cursor", 0)
        # ... restore your app's internal state from the buckets

Cost reporting (for apps that call LLM APIs):

    emit.cost_report(
        service="anthropic", model="claude-sonnet-4-20250514",
        input_tokens=1500, output_tokens=500, cost_usd=0.01,
    )
"""

import json
import sys
import uuid
from datetime import datetime, timezone
from typing import Callable, Optional


class Emitter:
    """Emit commands to Plexi immediately (outside a render frame)."""

    def __init__(self, app_id: str = ""):
        self._app_id = app_id

    def run_in_terminal(self, command: str):
        """Execute a shell command in the linked terminal."""
        print(json.dumps({"type": "run_in_terminal", "command": command}), flush=True)

    def cd(self, path: str):
        """Change the linked terminal's working directory."""
        print(json.dumps({"type": "cd", "path": path}), flush=True)

    def log(self, level: str, message: str):
        """Forward a log message to Plexi's logger (level: error|warn|info|debug)."""
        print(json.dumps({"type": "log", "level": level, "message": message}), flush=True)

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

    def cost_report(
        self,
        service: str,
        model: str,
        input_tokens: int,
        output_tokens: int,
        cost_usd: float,
        operation_id: Optional[str] = None,
    ):
        """Report LLM API cost to Plexi for logging and tracking."""
        print(json.dumps({
            "type": "cost_report",
            "app_id": self._app_id,
            "service": service,
            "model": model,
            "input_tokens": input_tokens,
            "output_tokens": output_tokens,
            "cost_usd": cost_usd,
            "operation_id": operation_id or str(uuid.uuid4()),
            "timestamp": datetime.now(timezone.utc).isoformat(),
        }), flush=True)

    def notification(
        self,
        title: str,
        body: Optional[str] = None,
        priority: int = 1,
    ):
        """
        Raise a notification to Plexi's notification log.

        Priority: 0 = info, 1 = normal, 2 = high, 3 = urgent.
        The notification is recorded to ~/.plexi-alpha/notifications.jsonl,
        increments the status-bar unread count, and appears in the
        notification palette (Cmd+Shift+N).
        """
        cmd = {
            "type": "notification",
            "priority": priority,
            "title": title,
            "source_app": self._app_id,
        }
        if body:
            cmd["body"] = body
        print(json.dumps(cmd), flush=True)


class RenderContext:
    """
    Passed to the on_render handler. Accumulates draw commands, then flushes.

    All coordinates are in logical pixels within the app surface.
    """

    def __init__(self, width: float, height: float, app_id: str = ""):
        self.width = width
        self.height = height
        self._app_id = app_id
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

    def notification(
        self,
        title: str,
        body: Optional[str] = None,
        priority: int = 1,
    ):
        """
        Raise a notification from inside a render frame.

        Priority: 0 = info, 1 = normal, 2 = high, 3 = urgent.
        """
        cmd = {
            "type": "notification",
            "priority": priority,
            "title": title,
            "source_app": self._app_id,
        }
        if body:
            cmd["body"] = body
        self._commands.append(cmd)

    def _flush(self):
        for cmd in self._commands:
            print(json.dumps(cmd), flush=True)
        print(json.dumps({"type": "frame_done"}), flush=True)
        self._commands.clear()


class App:
    """
    Base class for Plexi apps. Register event handlers via decorators.

    Handlers:
        @app.on_render      fn(ctx: RenderContext)
        @app.on_key         fn(key: str, mods: dict, emit: Emitter)
        @app.on_click       fn(x: float, y: float, button: str, emit: Emitter)
        @app.on_command     fn(text: str, emit: Emitter)
        @app.on_resize      fn(width: float, height: float)
        @app.on_get_state   fn() -> dict with keys: user_state, derived, session, persistent
        @app.on_set_state   fn(state: dict) — restore app from state buckets
    """

    def __init__(self, app_id: str = ""):
        self.width: float = 800.0
        self.height: float = 600.0
        self._on_render: Optional[Callable] = None
        self._on_key: Optional[Callable] = None
        self._on_click: Optional[Callable] = None
        self._on_command: Optional[Callable] = None
        self._on_resize: Optional[Callable] = None
        self._on_get_state: Optional[Callable] = None
        self._on_set_state: Optional[Callable] = None
        self._emitter = Emitter(app_id=app_id)

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

    def on_get_state(self, fn: Callable) -> Callable:
        """Register handler for get_state requests. Should return a dict with
        keys: user_state, derived, session, persistent."""
        self._on_get_state = fn
        return fn

    def on_set_state(self, fn: Callable) -> Callable:
        """Register handler for set_state requests. Receives a dict with
        keys: user_state, derived, session, persistent."""
        self._on_set_state = fn
        return fn

    def _handle_get_state(self):
        """Respond to a get_state request from Plexi."""
        if self._on_get_state:
            state = self._on_get_state()
        else:
            state = {}
        # Ensure all four buckets exist.
        result = {
            "type": "state",
            "user_state": state.get("user_state", {}),
            "derived": state.get("derived", {}),
            "session": state.get("session", {}),
            "persistent": state.get("persistent", {}),
        }
        print(json.dumps(result), flush=True)

    def _handle_set_state(self, event: dict):
        """Handle a set_state request from Plexi."""
        if self._on_set_state:
            self._on_set_state({
                "user_state": event.get("user_state", {}),
                "derived": event.get("derived", {}),
                "session": event.get("session", {}),
                "persistent": event.get("persistent", {}),
            })

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

            if event_type in ("init", "resize"):
                self.width = event.get("width", self.width)
                self.height = event.get("height", self.height)
                if event_type == "resize" and self._on_resize:
                    self._on_resize(self.width, self.height)

            elif event_type == "render":
                self.width = event.get("width", self.width)
                self.height = event.get("height", self.height)
                ctx = RenderContext(self.width, self.height, app_id=self._emitter._app_id)
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

            elif event_type == "get_state":
                self._handle_get_state()

            elif event_type == "set_state":
                self._handle_set_state(event)

            elif event_type == "shutdown":
                break
