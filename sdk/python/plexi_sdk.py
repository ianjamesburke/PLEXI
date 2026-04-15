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


from __future__ import annotations

__version__ = "0.3.0"

import json
import os
import pathlib
import sys
import uuid
from collections import deque
from datetime import datetime, timezone
from typing import Callable, Deque, List, Optional, Tuple


class Emitter:
    """Emit commands to Plexi immediately (outside a render frame).

    The emitter also owns the synchronous request/response path used by
    `get_secret()`. It shares a pending-event deque with the parent `App`
    so that any events read from stdin while waiting for a response (and
    which are unrelated to that response) get processed by the main loop
    on its next iteration instead of being dropped.
    """

    def __init__(self, app_id: str = "", pending_events: Optional[Deque[dict]] = None):
        self._app_id = app_id
        # Shared with App.run(); None only in cases where the emitter is
        # constructed standalone (e.g. legacy tests).
        self._pending_events: Deque[dict] = pending_events if pending_events is not None else deque()

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

    def get_secret(self, name: str) -> Optional[str]:
        """
        Fetch a secret by name from Plexi's Keychain-backed store.

        Resolution walks up from the app's launch directory to the user's
        home, returning the first matching value. Returns None if the
        secret is missing, Keychain is unavailable, or any other
        resolution error occurred (the host logs the failure; the app
        should not crash on a missing secret).

        Blocks until Plexi responds. Safe to call from `on_render`,
        `on_key`, `on_click`, or `on_command` handlers. Any unrelated
        events that arrive while waiting for the response are queued and
        processed by the main loop on its next iteration.
        """
        # Fire the request.
        print(json.dumps({"type": "secret_get", "name": name}), flush=True)

        # Block until we see the matching secret_response. Other events
        # get stashed for the main loop so nothing is lost.
        while True:
            line = sys.stdin.readline()
            if not line:
                # stdin closed — treat as host death; no secret available.
                return None
            line = line.strip()
            if not line:
                continue
            try:
                event = json.loads(line)
            except json.JSONDecodeError:
                continue
            if event.get("type") == "secret_response" and event.get("name") == name:
                value = event.get("value")
                return value if isinstance(value, str) else None
            # Unrelated event — stash for the main loop.
            self._pending_events.append(event)

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

    def notify(
        self,
        title: str,
        body: Optional[str] = None,
        urgency: str = "low",
        action_type: str = "dismiss",
        action_payload: Optional[dict] = None,
        expires_at: Optional[int] = None,
        visible_after: Optional[int] = None,
    ):
        """
        Raise a notification to Plexi's notification log.

        urgency: "low" | "medium" | "high"
        action_type: "dismiss" | "focus" | "confirm" | "text_input"
        action_payload: type-dependent dict; for "focus": {"pane_id": int, "fullscreen": bool}
        expires_at: unix timestamp (seconds); notification is dropped if already past
        visible_after: unix timestamp (seconds); defer display until this time

        The notification is recorded to ~/.plexi-alpha/notifications.jsonl,
        increments the status-bar unread count, and appears in the
        notification palette (Cmd+Shift+N).
        """
        cmd: dict = {
            "type": "notification",
            "title": title,
            "source_app": self._app_id,
            "urgency": urgency,
            "action_type": action_type,
        }
        if body is not None:
            cmd["body"] = body
        if action_payload is not None:
            cmd["action_payload"] = action_payload
        if expires_at is not None:
            cmd["expires_at"] = expires_at
        if visible_after is not None:
            cmd["visible_after"] = visible_after
        print(json.dumps(cmd), flush=True)

    def notification(
        self,
        title: str,
        body: Optional[str] = None,
        priority: int = 1,
    ):
        """Deprecated: use notify() instead. Kept for backwards compatibility."""
        urgency = {0: "low", 1: "low", 2: "medium", 3: "high"}.get(priority, "low")
        self.notify(title=title, body=body, urgency=urgency)

    def spawn_app(
        self,
        app_id: str,
        args: Optional[List[str]] = None,
        parent: str = "self",
        layout: Optional[dict] = None,
        lifecycle: str = "cascade",
        linked: bool = True,
        wire_channels: Optional[List[str]] = None,
    ):
        """
        Ask Plexi to spawn another app and place it in a layout slot relative to this one.

        This is the composition primitive: a file browser pressing Enter on a
        .txt file can emit `spawn_app("text-editor", args=[path], layout={"kind":"cols","slot":1,"ratio":0.5})`
        to bring up a text editor in a 50/50 right split, lifecycle-bonded to
        the file browser (so closing the file browser closes the editor).

        Args:
            app_id:
                The id of the app to spawn (must be in Plexi's app registry).
            args:
                Command-line args forwarded to the spawned app as argv[1..].
                Defaults to no args.
            parent:
                Anchor for the new pane's position. One of:
                    "self"  (default) — the pane emitting this call
                    "root"            — top-level (ignores the emitter's location)
                    "mark:<name>"     — reserved for a future named-layout system
            layout:
                How to position the new pane relative to the parent. A dict
                with a "kind" key. Examples:
                    {"kind": "fill"}
                    {"kind": "cols", "slot": 1, "ratio": 0.5}
                    {"kind": "rows", "slot": 1, "ratio": 0.4}
                    {"kind": "grid_2x2", "slot": 0}  # v1 stub, falls back to fill
                Defaults to {"kind": "fill"}.
            lifecycle:
                What happens to the spawned app when this app closes:
                    "cascade" (default) — close together
                    "orphan"            — detach, stay alive as a top-level pane
                    "prompt"            — ask the user (v1 stub, falls back to orphan)
            linked:
                When True (default), the new pane joins this pane's linked
                group so terminal-linking is shared.
            wire_channels:
                Typed-pipe channel names to pre-wire (e.g. ["file_buffer"]).
                Stored on the spawn relationship for the typed-pipes spec;
                defaults to no pre-wired channels.

        Authorization: the target app's `[app.spawnable]` manifest table is
        consulted — if `allow_callers` doesn't include this app, or the
        requested `lifecycle` isn't in `allow_lifecycle`, the spawn is
        refused and a notification is delivered back to this app.
        """
        cmd = {
            "type": "spawn_app",
            "app_id": app_id,
            "args": args or [],
            "parent": parent,
            "layout": layout or {"kind": "fill"},
            "lifecycle": lifecycle,
            "linked": linked,
            "wire_channels": wire_channels or [],
        }
        print(json.dumps(cmd), flush=True)

    def pipe_write(self, channel: str, value) -> None:
        """Write a value to a named output pipe channel.

        The host routes this to all connected apps (parent or children).
        Call from any event handler or from on_render via ctx.pipe_write().

        Args:
            channel: Channel name (e.g. "selection", "result", "data")
            value: Any JSON-serializable value (str, int, float, dict, list, etc.)
        """
        print(json.dumps({"type": "pipe_write", "channel": channel, "value": value}), flush=True)

    def submit_feedback(
        self,
        text: str,
        rating: Optional[int] = None,
        category: Optional[str] = None,
    ):
        """
        Submit user feedback about this app. Appended to feedback.jsonl in the app's
        install directory (~/.plexi-alpha/apps/<id>/feedback.jsonl).

        Requires PLEXI_APP_ID and PLEXI_APPS_DIR env vars, which Plexi sets automatically
        when launching apps. If missing, falls back to ~/.plexi/apps/<app_id>/.

        Args:
            text:     Free-form feedback message.
            rating:   Optional 1–5 star rating.
            category: Optional tag (e.g. "bug", "feature", "praise").
        """
        app_id = self._app_id or os.environ.get("PLEXI_APP_ID", "unknown")
        apps_dir = os.environ.get(
            "PLEXI_APPS_DIR",
            str(pathlib.Path.home() / ".plexi" / "apps"),
        )
        feedback_file = pathlib.Path(apps_dir) / app_id / "feedback.jsonl"
        entry: dict = {
            "ts": datetime.now(timezone.utc).isoformat(),
            "text": text,
        }
        if rating is not None:
            entry["rating"] = rating
        if category is not None:
            entry["category"] = category
        try:
            feedback_file.parent.mkdir(parents=True, exist_ok=True)
            with open(feedback_file, "a") as f:
                f.write(json.dumps(entry) + "\n")
        except OSError as e:
            self.warn(f"submit_feedback: could not write to {feedback_file}: {e}")
        self.info(f"feedback submitted: {text[:80]}")


def load_manifest(app_file: str = __file__) -> dict:
    """
    Read and return the manifest.toml for the app that calls this.

    Usage:
        manifest = load_manifest(__file__)
        version = manifest.get("app", {}).get("version", "0.0.0")

    Falls back to an empty dict if the manifest is missing or unparseable.
    """
    manifest_path = pathlib.Path(app_file).parent / "manifest.toml"
    if not manifest_path.exists():
        return {}
    text = manifest_path.read_text()
    # Minimal TOML parser for flat [app] tables — avoids a toml dep.
    result: dict = {}
    current_section: dict = result
    for line in text.splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("[") and line.endswith("]"):
            keys = line[1:-1].split(".")
            current_section = result
            for k in keys:
                current_section = current_section.setdefault(k, {})
            continue
        if "=" in line:
            k, _, v = line.partition("=")
            k = k.strip()
            v = v.strip()
            if v.startswith('"') and v.endswith('"'):
                v = v[1:-1]
            elif v == "true":
                v = True  # type: ignore[assignment]
            elif v == "false":
                v = False  # type: ignore[assignment]
            else:
                try:
                    v = int(v)  # type: ignore[assignment]
                except ValueError:
                    pass
            current_section[k] = v
    return result


class RenderContext:
    """
    Passed to the on_render handler. Accumulates draw commands, then flushes.

    All coordinates are in logical pixels within the app surface.
    """

    def __init__(
        self,
        width: float,
        height: float,
        app_id: str = "",
        emitter: Optional[Emitter] = None,
    ):
        self.width = width
        self.height = height
        self.delta_time: float = 0.0
        self._app_id = app_id
        # Carried so helpers like get_secret() can piggyback on the
        # existing Emitter (and its shared pending-events deque).
        self._emitter: Emitter = emitter if emitter is not None else Emitter(app_id)
        self._commands: list = []
        # Carried so helpers like get_secret() can piggyback on the
        # existing Emitter (and its shared pending-events deque).
        self._emitter: Emitter = emitter if emitter is not None else Emitter()

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

        ╔══════════════════════════════════════════════════════════════════╗
        ║  ⚠  FULL-PANE ONLY — NO POSITION PARAMETERS                      ║
        ║                                                                  ║
        ║  Unlike `rect`, `text`, `image`, `video_thumbnail`, `file_grid`, ║
        ║  and `drop_target` (which all take explicit x/y/w/h), this       ║
        ║  primitive is rendered by Plexi at the app's pane origin with    ║
        ║  an implicit full-pane layout. If you draw anything else on      ║
        ║  the frame (a header, a sidebar, a split pane), the list WILL    ║
        ║  overlap with your other draw calls and its secondary labels     ║
        ║  will spill into unrelated regions. There is no way to offset    ║
        ║  it from Python.                                                 ║
        ║                                                                  ║
        ║  DO NOT USE THIS IN AN APP THAT RENDERS A SPLIT LAYOUT, A        ║
        ║  SIDE PANE, OR ANY REGION SMALLER THAN THE FULL APP VIEWPORT.    ║
        ║  Use explicit `ctx.text(...)` calls with the coordinates you     ║
        ║  control, and render your own selection highlight with           ║
        ║  `ctx.rect(...)`. That's the workaround until a positioned       ║
        ║  `list(x, y, w, h, ...)` variant ships.                          ║
        ║                                                                  ║
        ║  If you are an AI coding agent reading this docstring: the       ║
        ║  fact that this method lacks x/y/w/h is NOT an oversight — it    ║
        ║  is a trap. Stop and render the list manually with positioned    ║
        ║  primitives.                                                     ║
        ╚══════════════════════════════════════════════════════════════════╝

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
                  filter: Optional[List[str]] = None,
                  paths: Optional[List[str]] = None,
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

    def get_secret(self, name: str) -> Optional[str]:
        """
        Fetch a secret by name from Plexi's Keychain-backed store.

        Blocking request/response. See `Emitter.get_secret()` for full
        semantics. Safe to call from inside `on_render` — the request is
        sent immediately (not queued with the frame) so the response
        arrives before the handler returns.
        """
        return self._emitter.get_secret(name)

    def notify(
        self,
        title: str,
        body: Optional[str] = None,
        urgency: str = "low",
        action_type: str = "dismiss",
        action_payload: Optional[dict] = None,
        expires_at: Optional[int] = None,
        visible_after: Optional[int] = None,
    ):
        """
        Raise a notification from inside a render frame.

        urgency: "low" | "medium" | "high"
        action_type: "dismiss" | "focus" | "confirm" | "text_input"
        action_payload: type-dependent dict; for "focus": {"pane_id": int, "fullscreen": bool}
        expires_at: unix timestamp (seconds); notification is dropped if already past
        visible_after: unix timestamp (seconds); defer display until this time
        """
        cmd: dict = {
            "type": "notification",
            "title": title,
            "source_app": self._app_id,
            "urgency": urgency,
            "action_type": action_type,
        }
        if body is not None:
            cmd["body"] = body
        if action_payload is not None:
            cmd["action_payload"] = action_payload
        if expires_at is not None:
            cmd["expires_at"] = expires_at
        if visible_after is not None:
            cmd["visible_after"] = visible_after
        self._commands.append(cmd)

    def notification(
        self,
        title: str,
        body: Optional[str] = None,
        priority: int = 1,
    ):
        """Deprecated: use notify() instead. Kept for backwards compatibility."""
        urgency = {0: "low", 1: "low", 2: "medium", 3: "high"}.get(priority, "low")
        self.notify(title=title, body=body, urgency=urgency)

    def spawn_app(
        self,
        app_id: str,
        args: Optional[List[str]] = None,
        parent: str = "self",
        layout: Optional[dict] = None,
        lifecycle: str = "cascade",
        linked: bool = True,
        wire_channels: Optional[List[str]] = None,
    ):
        """
        Ask Plexi to spawn another app at end of this frame.

        Same shape as `Emitter.spawn_app` — see that method's docstring for
        full field semantics. Use this variant when you need the spawn to
        happen as part of a render pass (queued alongside the frame's draw
        commands); use `emit.spawn_app` from a key/command/mouse handler.
        """
        self._commands.append({
            "type": "spawn_app",
            "app_id": app_id,
            "args": args or [],
            "parent": parent,
            "layout": layout or {"kind": "fill"},
            "lifecycle": lifecycle,
            "linked": linked,
            "wire_channels": wire_channels or [],
        })

    def pipe_write(self, channel: str, value) -> None:
        """Write to a named output pipe channel (same as Emitter.pipe_write).

        The host routes this to all connected apps (parent or children).
        Queued with the frame's draw commands and flushed at frame end.

        Args:
            channel: Channel name (e.g. "selection", "result", "data")
            value: Any JSON-serializable value (str, int, float, dict, list, etc.)
        """
        self._commands.append({"type": "pipe_write", "channel": channel, "value": value})

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

    def __init__(self, app_id: str = "", auto_min_size: bool = True):
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
        self._on_pipe_data: Optional[Callable] = None
        # Events that arrived on stdin while a blocking request (e.g.
        # get_secret) was waiting for its response. Drained at the top of
        # each run-loop iteration before reading a fresh line from stdin.
        self._pending_events: Deque[dict] = deque()
        self._emitter = Emitter(app_id=app_id, pending_events=self._pending_events)
        # Breakpoints: list of (min_width, min_height, render_fn).
        # Walked in descending area order on each render to pick the
        # most specific match whose constraints fit the current pane.
        self._breakpoints: List[Tuple[float, float, Callable]] = []
        # Minimum pane size before the SDK draws its built-in
        # "too small" fallback frame (bypassing the user render fn).
        # Values of 0 mean "no floor on this axis".
        self._min_width: float = 0.0
        self._min_height: float = 0.0
        self._auto_min_size: bool = auto_min_size
        # Palette used by the auto min-size fallback. Apps can override
        # per-instance via `set_min_size_colors` if they care.
        self._min_size_bg: str = "#0a0a0a"
        self._min_size_fg: str = "#888888"
        self._min_size_accent: str = "#ffffff"

    def on_render(self, fn: Callable) -> Callable:
        self._on_render = fn
        return fn

    def breakpoint(
        self, min_width: float = 0.0, min_height: float = 0.0
    ) -> Callable:
        """
        Register a render function for a specific minimum pane size.

        Apps can stack multiple `@app.breakpoint(...)` handlers; on each
        render event the SDK picks the most specific breakpoint whose
        constraints fit the current pane (largest min_width * min_height
        area that still satisfies `width >= min_width AND height >= min_height`).
        If none match, a breakpoint registered with no constraints (the
        default 0x0 fallback) is called.

        Example:

            @app.breakpoint(min_width=800, min_height=500)
            def render_full(ctx):
                ...

            @app.breakpoint(min_width=400)
            def render_compact(ctx):
                ...

            @app.breakpoint()  # fallback — always matches
            def render_fallback(ctx):
                ...

        Mutually exclusive with `@app.on_render`. Registering both raises
        a RuntimeError at app startup.
        """
        def decorator(fn: Callable) -> Callable:
            self._breakpoints.append((float(min_width), float(min_height), fn))
            return fn
        return decorator

    def set_min_size(self, min_width: float, min_height: float) -> None:
        """
        Set the minimum pane size programmatically.

        When the pane is smaller than this on either axis and `auto_min_size`
        is enabled, the SDK draws a built-in "too small" fallback frame
        (background + label + directional arrow + current size) instead of
        calling any user render function.

        Useful for apps that compute their min size at runtime (e.g. based
        on font metrics) rather than declaring it in manifest.toml.
        """
        self._min_width = float(min_width)
        self._min_height = float(min_height)

    def set_min_size_colors(
        self,
        bg: Optional[str] = None,
        fg: Optional[str] = None,
        accent: Optional[str] = None,
    ) -> None:
        """
        Override the palette used by the auto min-size fallback frame.

        The SDK has no access to live host theme tokens, so defaults are
        hardcoded (`#0a0a0a` background, `#888888` label, `#ffffff` arrow).
        Apps that care about matching a custom theme can override any
        subset of these.
        """
        if bg is not None:
            self._min_size_bg = bg
        if fg is not None:
            self._min_size_fg = fg
        if accent is not None:
            self._min_size_accent = accent

    def _load_manifest_layout(self) -> None:
        """
        Read `[app.layout]` from manifest.toml (if present) and populate
        `_min_width` / `_min_height` unless already set programmatically.
        Best-effort — missing or unparseable manifests are silently ignored.
        """
        if self._min_width or self._min_height:
            return  # explicit set_min_size wins over manifest
        try:
            # Walk up from the caller's file to find manifest.toml.
            main_file = getattr(sys.modules.get("__main__"), "__file__", None)
            if not main_file:
                return
            manifest = load_manifest(main_file)
            layout = manifest.get("app", {}).get("layout", {})
            if isinstance(layout, dict):
                mw = layout.get("min_width", 0)
                mh = layout.get("min_height", 0)
                try:
                    self._min_width = float(mw)
                    self._min_height = float(mh)
                except (TypeError, ValueError):
                    pass
        except Exception:
            # Never let manifest errors kill the app — log-only fallback.
            pass

    def _render_min_size_fallback(self, width: float, height: float) -> None:
        """
        Emit the built-in "pane too small" frame directly as JSON draw
        commands. Does NOT go through RenderContext (the user render fn
        is never called in this code path).

        Draws: background rect, centered "min size: W x H" label,
        directional arrow(s) pointing toward the axes that need to grow,
        and a dim "current: w x h" subtitle.
        """
        bg = self._min_size_bg
        fg = self._min_size_fg
        accent = self._min_size_accent
        min_w = self._min_width
        min_h = self._min_height

        needs_width = width < min_w
        needs_height = height < min_h

        if needs_width and needs_height:
            arrow = "\u2198"  # ↘
        elif needs_width:
            arrow = "\u2192"  # →
        elif needs_height:
            arrow = "\u2193"  # ↓
        else:
            arrow = ""

        label = f"min size: {int(min_w)} x {int(min_h)}"
        current = f"current: {int(width)} x {int(height)}"

        # Rough centering — we don't have text metrics, so estimate with
        # a fixed per-char width of 0.55 * font_size. Good enough for a
        # fallback that only shows when the pane is too small anyway.
        label_size = 14.0
        current_size = 11.0
        arrow_size = 32.0
        label_w = len(label) * label_size * 0.55
        current_w = len(current) * current_size * 0.55
        arrow_w = len(arrow) * arrow_size * 0.55

        cx = width / 2.0
        cy = height / 2.0
        label_x = max(0.0, cx - label_w / 2.0)
        current_x = max(0.0, cx - current_w / 2.0)
        arrow_x = max(0.0, cx - arrow_w / 2.0)

        label_y = max(0.0, cy - 30.0)
        arrow_y = cy
        current_y = cy + 40.0

        cmds = [
            {"type": "rect", "x": 0.0, "y": 0.0, "w": width, "h": height,
             "fill": bg, "radius": 0.0},
            {"type": "text", "x": label_x, "y": label_y, "text": label,
             "size": label_size, "color": fg,
             "monospace": False, "bold": True},
        ]
        if arrow:
            cmds.append({
                "type": "text", "x": arrow_x, "y": arrow_y, "text": arrow,
                "size": arrow_size, "color": accent,
                "monospace": False, "bold": False,
            })
        cmds.append({
            "type": "text", "x": current_x, "y": current_y, "text": current,
            "size": current_size, "color": fg,
            "monospace": False, "bold": False,
        })
        cmds.append({"type": "frame_done"})
        for cmd in cmds:
            print(json.dumps(cmd), flush=True)

    def _pick_breakpoint(self, width: float, height: float) -> Optional[Callable]:
        """
        Walk breakpoints sorted by (min_width * min_height) descending and
        return the first one whose `width >= min_width AND height >= min_height`.
        Falls back to a zero-constraint breakpoint if present. Returns None
        if no breakpoints are registered at all.
        """
        if not self._breakpoints:
            return None
        ordered = sorted(
            self._breakpoints,
            key=lambda b: (b[0] * b[1], b[0], b[1]),
            reverse=True,
        )
        for min_w, min_h, fn in ordered:
            if width >= min_w and height >= min_h:
                return fn
        # No match — look for a 0x0 fallback explicitly.
        for min_w, min_h, fn in self._breakpoints:
            if min_w == 0.0 and min_h == 0.0:
                return fn
        return None

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

    def on_pipe_data(self, fn: Callable) -> Callable:
        """Register a handler for pipe data received from connected apps.

        Called when another app (parent or child) writes to a channel that this
        app is connected to via a spawn relationship.

        Handler signature: fn(from_app: str, channel: str, value, emit: Emitter)

        Args:
            from_app: The app_id of the app that wrote the value.
            channel:  The channel name the value was written to.
            value:    The JSON value (str, int, float, dict, list, or None).
            emit:     An Emitter to respond or trigger further actions.
        """
        self._on_pipe_data = fn
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

        # Validate mutually exclusive render registration.
        if self._on_render is not None and self._breakpoints:
            raise RuntimeError(
                "plexi_sdk: @app.on_render and @app.breakpoint(...) are "
                "mutually exclusive — use one or the other, not both."
            )

        # Pull min-size from the manifest (if set_min_size was not called).
        self._load_manifest_layout()

        while True:
            # Drain any events that were queued by a blocking helper (e.g.
            # get_secret) during a prior handler. These were read off stdin
            # while waiting for a specific response, so they need to be
            # replayed here before pulling more.
            if self._pending_events:
                event = self._pending_events.popleft()
            else:
                line = sys.stdin.readline()
                if not line:
                    break
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
                # Auto min-size fallback: if the pane is smaller than
                # our declared floor on either axis, draw the built-in
                # "too small" frame and skip user rendering entirely.
                if (
                    self._auto_min_size
                    and (self._min_width > 0 or self._min_height > 0)
                    and (
                        self.width < self._min_width
                        or self.height < self._min_height
                    )
                ):
                    self._render_min_size_fallback(self.width, self.height)
                    continue
                ctx = RenderContext(
                    self.width,
                    self.height,
                    app_id=self._emitter._app_id,
                    emitter=self._emitter,
                )
                ctx.delta_time = self.delta_time
                # Breakpoint dispatch (if registered) overrides on_render.
                if self._breakpoints:
                    fn = self._pick_breakpoint(self.width, self.height)
                    if fn is not None:
                        fn(ctx)
                elif self._on_render:
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

            elif event_type == "pipe_data":
                if self._on_pipe_data:
                    self._on_pipe_data(
                        event.get("from_app", ""),
                        event.get("channel", ""),
                        event.get("value"),
                        self._emitter,
                    )

            elif event_type == "secret_response":
                # Should normally be consumed by the blocking get_secret()
                # helper. If one arrives at top level (e.g. a late response
                # after timeout), there's nothing sensible to do — drop it.
                pass

            elif event_type == "shutdown":
                break
