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


__version__ = "0.1.0"

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
        self.delta_time: float = 0.0
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

    def drop_target(
        self,
        id: str,
        x: float,
        y: float,
        w: float,
        h: float,
        accept: Optional[list] = None,
        label: Optional[str] = None,
    ):
        """
        Declare a region that can accept dropped files from outside Plexi.

        Drop targets are stateless — you must re-emit this on every render
        frame for the region to remain active.

        Args:
            id:     App-local identifier for this drop zone. Echoed back in
                    the on_drop handler so you can tell which zone received
                    the drop when you declare multiple.
            x, y, w, h: Region in the app's local coordinate space.
            accept: List of file extensions (lowercase, no dot) to accept,
                    e.g. ["png", "jpg", "mp4"]. Empty or None = accept
                    anything. Paths that don't match are filtered out by
                    Plexi before your on_drop handler is called.
            label:  Optional hint text shown by Plexi over the target while
                    the user is hovering with files from Finder.
        """
        cmd = {
            "type": "drop_target",
            "id": id,
            "x": x, "y": y, "w": w, "h": h,
            "accept": accept or [],
        }
        if label:
            cmd["label"] = label
        self._commands.append(cmd)

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

    def image(self, path: str, x: float, y: float, w: float, h: float,
              fit: str = "contain", rounding: float = 0.0):
        """
        Draw an image from a file on disk.

        Plexi decodes and caches the texture (keyed by absolute path + mtime).
        `path` may be absolute or relative to the app's cwd.

        `fit` controls how the image is placed in the rect:
            "contain" — fit inside, preserve aspect, letterbox (default)
            "cover"   — fill the rect, preserve aspect, crop overflow
            "fill"    — stretch to the rect, ignoring aspect ratio

        `rounding` is the corner radius in logical pixels.
        """
        cmd: dict = {
            "type": "image",
            "path": path,
            "x": x, "y": y, "w": w, "h": h,
        }
        if fit != "contain":
            cmd["fit"] = fit
        if rounding > 0:
            cmd["rounding"] = rounding
        self._commands.append(cmd)

    def video_thumbnail(self, path: str, x: float, y: float, w: float, h: float,
                        show_play_button: bool = True,
                        timestamp_seconds: float = 0.0):
        """
        Draw a thumbnail for a video file.

        Plexi extracts a single frame at `timestamp_seconds` using ffmpeg and
        caches the result under ~/.cache/plexi/thumbnails/. Extraction runs on
        a background thread so the first render returns a loading placeholder
        and the real thumbnail appears a frame later.

        If `show_play_button` is True (default), a centered play triangle is
        overlaid. Clicking the rect opens the original video with the system
        default player (`open` on macOS).
        """
        self._commands.append({
            "type": "video_thumbnail",
            "path": path,
            "x": x, "y": y, "w": w, "h": h,
            "show_play_button": show_play_button,
            "timestamp_seconds": timestamp_seconds,
        })

    def file_grid(self, x: float, y: float, w: float, h: float,
                  path: Optional[str] = None,
                  filter: Optional[list] = None,
                  paths: Optional[list] = None,
                  item_size: float = 96.0,
                  columns: Optional[int] = None,
                  show_labels: bool = True):
        """
        Draw a grid of files with auto-generated thumbnails.

        Two modes — exactly one must be provided:
            path  + optional filter  — walk a directory non-recursively
            paths                    — explicit list of file paths to show

        `filter` accepts glob patterns or bare extensions, e.g.
        ["*.png", "*.jpg"] or ["png", "mp4"].

        Image files render via the shared image texture cache; video files
        (mp4/mov/webm/mkv/m4v/avi) render via the video thumbnail cache.
        Other file types show a generic icon with the extension label.

        Clicking an item opens it with the system default handler.
        """
        if path is None and paths is None:
            raise ValueError("file_grid requires either 'path' or 'paths'")
        cmd: dict = {
            "type": "file_grid",
            "x": x, "y": y, "w": w, "h": h,
            "item_size": item_size,
            "show_labels": show_labels,
        }
        if path is not None:
            cmd["path"] = path
            if filter:
                cmd["filter"] = filter
        if paths is not None:
            cmd["paths"] = paths
        if columns is not None:
            cmd["columns"] = columns
        self._commands.append(cmd)

    def set_cursor(self, cursor: str):
        """Set the cursor icon for the app pane for this frame.

        Values: 'default', 'pointer', 'grab', 'grabbing', 'crosshair', 'text'.
        Must be re-emitted each frame to persist (cursor resets to 'default' each frame).
        """
        self._commands.append({"type": "set_cursor", "cursor": cursor})

    def mouse_tracking(self, enabled: bool):
        """Enable or disable mouse-move event delivery.

        When enabled, Plexi sends MouseMove events to this app on every frame.
        Off by default — enable only when needed to avoid flooding the pipe.
        This setting persists until changed; you do not need to re-emit each frame.
        """
        self._commands.append({"type": "mouse_tracking", "enabled": enabled})

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
        @app.on_render        fn(ctx: RenderContext)
        @app.on_key           fn(key: str, mods: dict, emit: Emitter)
        @app.on_click         fn(x: float, y: float, button: str, emit: Emitter)
        @app.on_command       fn(text: str, emit: Emitter)
        @app.on_resize        fn(width: float, height: float)
        @app.on_get_state     fn() -> dict with keys: user_state, derived, session, persistent
        @app.on_set_state     fn(state: dict) — restore app from state buckets
        @app.on_drop          fn(target_id: str, paths: list[str], emit: Emitter)
        @app.on_mouse_down    fn(x: float, y: float, button: str, emit: Emitter)
        @app.on_mouse_up      fn(x: float, y: float, button: str, emit: Emitter)
        @app.on_mouse_move    fn(x: float, y: float, emit: Emitter)
        @app.on_scroll        fn(x: float, y: float, delta_x: float, delta_y: float, emit: Emitter)
    """

    def __init__(self, app_id: str = ""):
        self.width: float = 800.0
        self.height: float = 600.0
        self.delta_time: float = 0.0
        self._on_render: Optional[Callable] = None
        self._on_key: Optional[Callable] = None
        self._on_click: Optional[Callable] = None
        self._on_command: Optional[Callable] = None
        self._on_resize: Optional[Callable] = None
        self._on_get_state: Optional[Callable] = None
        self._on_set_state: Optional[Callable] = None
        self._on_drop: Optional[Callable] = None
        self._on_mouse_down: Optional[Callable] = None
        self._on_mouse_up: Optional[Callable] = None
        self._on_mouse_move: Optional[Callable] = None
        self._on_scroll: Optional[Callable] = None
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

    def on_drop(self, fn: Callable) -> Callable:
        """Register a handler for file drops onto declared drop targets.

        The handler receives (target_id: str, paths: list[str], emit: Emitter).
        `target_id` matches the id you passed to ctx.drop_target(). `paths`
        are absolute host filesystem paths already filtered by the target's
        accept list.
        """
        self._on_drop = fn
        return fn

    def on_mouse_down(self, fn: Callable) -> Callable:
        """Register a handler for mouse button presses.

        The handler receives (x: float, y: float, button: str, emit: Emitter).
        `button` is one of 'left', 'right', 'middle'.
        """
        self._on_mouse_down = fn
        return fn

    def on_mouse_up(self, fn: Callable) -> Callable:
        """Register a handler for mouse button releases.

        The handler receives (x: float, y: float, button: str, emit: Emitter).
        `button` is one of 'left', 'right', 'middle'.
        """
        self._on_mouse_up = fn
        return fn

    def on_mouse_move(self, fn: Callable) -> Callable:
        """Register a handler for mouse movement.

        The handler receives (x: float, y: float, emit: Emitter).
        Only called when mouse_tracking = true in manifest capabilities or after
        the app emits a MouseTracking draw command.
        """
        self._on_mouse_move = fn
        return fn

    def on_scroll(self, fn: Callable) -> Callable:
        """Register a handler for scroll wheel / trackpad scroll events.

        The handler receives (x: float, y: float, delta_x: float, delta_y: float, emit: Emitter).
        `x, y` is the cursor position; `delta_x, delta_y` is the scroll amount in logical pixels.
        """
        self._on_scroll = fn
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
                self.delta_time = event.get("delta_time", 0.0)
                ctx = RenderContext(self.width, self.height, app_id=self._emitter._app_id)
                ctx.delta_time = self.delta_time
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

            elif event_type == "drop":
                if self._on_drop:
                    self._on_drop(
                        event.get("target_id", ""),
                        event.get("paths", []),
                        self._emitter,
                    )

            elif event_type == "mouse_down":
                if self._on_mouse_down:
                    self._on_mouse_down(
                        event.get("x", 0.0),
                        event.get("y", 0.0),
                        event.get("button", "left"),
                        self._emitter,
                    )

            elif event_type == "mouse_up":
                if self._on_mouse_up:
                    self._on_mouse_up(
                        event.get("x", 0.0),
                        event.get("y", 0.0),
                        event.get("button", "left"),
                        self._emitter,
                    )

            elif event_type == "mouse_move":
                if self._on_mouse_move:
                    self._on_mouse_move(
                        event.get("x", 0.0),
                        event.get("y", 0.0),
                        self._emitter,
                    )

            elif event_type == "scroll":
                if self._on_scroll:
                    self._on_scroll(
                        event.get("x", 0.0),
                        event.get("y", 0.0),
                        event.get("delta_x", 0.0),
                        event.get("delta_y", 0.0),
                        self._emitter,
                    )

            elif event_type == "shutdown":
                break
