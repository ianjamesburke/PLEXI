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


# ---------------------------------------------------------------------------
# Module-level syntax colorizers
# ---------------------------------------------------------------------------

_SYN_KW = frozenset({
    'def', 'class', 'return', 'import', 'from', 'if', 'else', 'elif',
    'for', 'while', 'in', 'not', 'and', 'or', 'True', 'False', 'None',
    'try', 'except', 'finally', 'with', 'as', 'pass', 'raise', 'yield',
    'lambda', 'global', 'nonlocal', 'break', 'continue', 'is', 'del',
})
_COL_KW    = "#569cd6"
_COL_STR   = "#ce9178"
_COL_CMT   = "#6a9955"
_COL_NUM   = "#b5cea8"
_COL_PLAIN = "#d4d4d4"


def _python_colorize(line: str):
    """
    Tokenize a single Python source line into (segment, color) pairs.
    Returns a list — never None.
    """
    # Full-line comment fast path
    stripped = line.lstrip()
    if stripped.startswith("#"):
        return [(line, _COL_CMT)]

    segments: list = []
    i = 0
    in_str = False
    str_char = ""
    tok_start = 0

    def flush_plain(end: int):
        nonlocal tok_start
        chunk = line[tok_start:end]
        if chunk:
            buf = ""
            for ch in chunk:
                if ch.isalnum() or ch in ("_", "."):
                    buf += ch
                else:
                    if buf:
                        if buf in _SYN_KW:
                            color = _COL_KW
                        elif buf.replace(".", "", 1).isdigit():
                            color = _COL_NUM
                        else:
                            color = _COL_PLAIN
                        segments.append((buf, color))
                        buf = ""
                    segments.append((ch, _COL_PLAIN))
            if buf:
                if buf in _SYN_KW:
                    color = _COL_KW
                elif buf.replace(".", "", 1).isdigit():
                    color = _COL_NUM
                else:
                    color = _COL_PLAIN
                segments.append((buf, color))
        tok_start = end

    while i < len(line):
        ch = line[i]
        if in_str:
            if ch == str_char and (i == 0 or line[i - 1] != "\\"):
                str_token = line[tok_start:i + 1]
                segments.append((str_token, _COL_STR))
                in_str = False
                tok_start = i + 1
            i += 1
            continue
        if ch in ('"', "'"):
            flush_plain(i)
            in_str = True
            str_char = ch
            tok_start = i
            i += 1
            continue
        if ch == "#":
            flush_plain(i)
            segments.append((line[i:], _COL_CMT))
            i = len(line)
            tok_start = i
            continue
        i += 1

    flush_plain(len(line))
    return segments


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
        self._app: Optional["App"] = None  # set by App.run() before on_render

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
        _ = viewport_id; _ = on_pan; _ = on_zoom
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
        _ = input_id; _ = on_change; _ = max_length
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
        _ = tab_id; _ = on_change
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
        _ = grid_id
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
        _ = modal_id; _ = on_dismiss
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

    def multiline_text_input(self, editor_id: str, lines: list, cursor_line: int,
                             cursor_col: int, on_change, x: float, y: float,
                             w: float, h: float, focused: bool = True,
                             bg: Optional[str] = None, fg: Optional[str] = None,
                             font_size: float = 14.0,
                             line_colorizer=None) -> dict:
        """
        Base multi-line text input primitive.

        lines: list[str] — content, one string per line (app owns state)
        cursor_line, cursor_col — cursor position (app owns)
        on_change(new_lines, new_cursor_line, new_cursor_col) — called on edits
        focused — whether this editor captures key events
        bg, fg — background/foreground colors (defaults to dark theme)
        font_size — font size in logical pixels
        line_colorizer — optional callable(line_str) -> list[(segment, color)] | None
                         If None, each line is drawn in fg. If provided, each line is
                         drawn segment-by-segment with per-segment colors.

        Keyboard handling is routed through App._editor_states when focused=True.
        Requires ctx._app to be set (done automatically by App.run()).
        Returns bounding rect dict.
        """
        _app = self._app
        bg_color = bg or "#1e1e2e"
        fg_color = fg or "#cdd6f4"

        # Register with App for key routing
        if _app is not None and focused:
            _app._editor_states[editor_id] = {
                "lines": lines,
                "cursor_line": cursor_line,
                "cursor_col": cursor_col,
                "on_change": on_change,
            }
            _app._focused_editor_id = editor_id

        line_h = font_size + 4.0
        visible_lines = max(1, int(h / line_h))
        scroll_line = _app._editor_scroll.get(editor_id, 0) if _app else 0

        # Clamp scroll
        max_scroll = max(0, len(lines) - visible_lines)
        scroll_line = min(scroll_line, max_scroll)
        if _app:
            _app._editor_scroll[editor_id] = scroll_line

        # Background
        self.rect(x, y, w, h, fill=bg_color, radius=4.0)

        # Render visible lines
        for rel_idx in range(visible_lines):
            line_idx = scroll_line + rel_idx
            if line_idx >= len(lines):
                break
            ly = y + rel_idx * line_h

            # Current line highlight
            if focused and line_idx == cursor_line:
                self.rect(x, ly, w, line_h, fill="#2a2a3e", radius=0.0)

            line_text = lines[line_idx]
            if line_colorizer is not None:
                segments = line_colorizer(line_text)
                if segments:
                    char_w = font_size * 0.6
                    cx = x + 4
                    for tok, color in segments:
                        self.text(cx, ly + 2, tok, size=font_size, color=color, monospace=True)
                        cx += len(tok) * char_w
                else:
                    self.text(x + 4, ly + 2, line_text, size=font_size, color=fg_color, monospace=True)
            else:
                self.text(x + 4, ly + 2, line_text, size=font_size, color=fg_color, monospace=True)

        # Cursor (blink)
        if focused and self._time % 1.0 < 0.5:
            rel = cursor_line - scroll_line
            if 0 <= rel < visible_lines:
                char_w = font_size * 0.6
                cur_x = x + 4 + cursor_col * char_w
                cur_y = y + rel * line_h
                self.rect(cur_x, cur_y + 1, 2.0, line_h - 2, fill=fg_color)

        return {"x": x, "y": y, "w": w, "h": h}

    def code_editor(self, editor_id: str, lines: list, cursor_line: int,
                    cursor_col: int, on_change, x: float, y: float,
                    w: float, h: float, focused: bool = True,
                    language: str = "python") -> dict:
        """
        Multi-line code editor with line-number gutter and syntax highlighting.

        Wraps multiline_text_input — draws a 40px line-number gutter, then delegates
        to multiline_text_input with x offset by 40px and w reduced by 40px.
        For language="python", passes _python_colorize as the line_colorizer.

        lines: list[str] — code content, one string per line (app owns state)
        cursor_line, cursor_col — cursor position (app owns)
        on_change(new_lines, new_cursor_line, new_cursor_col) — called on edits
        focused — whether this editor captures key events
        language — syntax highlighting language ("python" supported; others = plain)

        Keyboard handling is routed through App._editor_states when focused=True.
        Requires ctx._app to be set (done automatically by App.run()).
        Returns bounding rect dict.
        """
        _app = self._app
        font_size = 13.0
        line_h = font_size + 4.0
        line_num_w = 40.0
        visible_lines = max(1, int(h / line_h))
        scroll_line = _app._editor_scroll.get(editor_id, 0) if _app else 0
        max_scroll = max(0, len(lines) - visible_lines)
        scroll_line = min(scroll_line, max_scroll)

        # Gutter background
        self.rect(x, y, line_num_w, h, fill="#181825", radius=0.0)

        # Right-aligned line numbers
        for rel_idx in range(visible_lines):
            line_idx = scroll_line + rel_idx
            if line_idx >= len(lines):
                break
            ly = y + rel_idx * line_h
            line_num_str = str(line_idx + 1)
            lnx = x + line_num_w - len(line_num_str) * 7 - 4
            self.text(lnx, ly + 2, line_num_str, size=font_size,
                      color="#45475a", monospace=True)

        # Wrap on_change to forward unchanged
        def _wrapped_on_change(new_lines, new_cl, new_cc):
            on_change(new_lines, new_cl, new_cc)

        colorizer = _python_colorize if language == "python" else None

        return self.multiline_text_input(
            editor_id, lines, cursor_line, cursor_col, _wrapped_on_change,
            x=x + line_num_w, y=y, w=w - line_num_w, h=h,
            focused=focused, font_size=font_size, line_colorizer=colorizer,
        )

    def _draw_syntax_line(self, x: float, y: float, line: str, size: float):
        """Draw a single code line with Python syntax highlighting. Delegates to _python_colorize."""
        char_w = size * 0.6
        cx = x
        for tok, color in _python_colorize(line):
            self.text(cx, y, tok, size=size, color=color, monospace=True)
            cx += len(tok) * char_w

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
        @app.on_click         fn(x: float, y: float, button: str, emit: Emitter)
        @app.on_mouse_down    fn(x: float, y: float, button: str, emit: Emitter)
        @app.on_mouse_up      fn(x: float, y: float, button: str, emit: Emitter)
        @app.on_mouse_move    fn(x: float, y: float, emit: Emitter)
        @app.on_scroll        fn(x: float, y: float, delta_x: float, delta_y: float, emit: Emitter)
        @app.on_command       fn(text: str, emit: Emitter)
        @app.on_resize        fn(width: float, height: float)
        @app.on_event         fn(event: dict, emit: Emitter)       — v2: bus events
        @app.on_run_created   fn(run_id: str, emit: Emitter)       — v2: run lifecycle
    """

    def __init__(self):
        self.width: float = 800.0
        self.height: float = 600.0
        self.protocol_version: int = 1
        self.open_intent: Optional[OpenIntent] = None
        self._render_time: float = 0.0
        self._measure_req_id: int = 0
        self._on_init: Optional[Callable] = None
        self._on_render: Optional[Callable] = None
        self._on_key: Optional[Callable] = None
        self._on_click: Optional[Callable] = None
        self._on_mouse_down: Optional[Callable] = None
        self._on_mouse_up: Optional[Callable] = None
        self._on_mouse_move: Optional[Callable] = None
        self._on_scroll: Optional[Callable] = None
        self._on_command: Optional[Callable] = None
        self._on_resize: Optional[Callable] = None
        self._on_event: Optional[Callable] = None
        self._on_run_created: Optional[Callable] = None
        self._emitter = Emitter()
        # Code editor state — keyed by editor_id
        self._editor_states: dict = {}
        self._editor_scroll: dict = {}   # editor_id -> scroll_line (int)
        self._focused_editor_id: Optional[str] = None

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

    def on_mouse_down(self, fn: Callable) -> Callable:
        self._on_mouse_down = fn
        return fn

    def on_mouse_up(self, fn: Callable) -> Callable:
        self._on_mouse_up = fn
        return fn

    def on_mouse_move(self, fn: Callable) -> Callable:
        self._on_mouse_move = fn
        return fn

    def on_scroll(self, fn: Callable) -> Callable:
        self._on_scroll = fn
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

    def _route_key_to_editor(self, editor_id: str, es: dict, key: str, mods: dict) -> bool:
        """Handle a key event for a focused code editor. Returns True if consumed."""
        lines = [l for l in es["lines"]]  # shallow copy to mutate
        cl = es["cursor_line"]
        cc = es["cursor_col"]
        on_change = es["on_change"]
        scroll = self._editor_scroll.get(editor_id, 0)

        def clamp_col():
            nonlocal cc
            if cl < len(lines):
                cc = min(cc, len(lines[cl]))

        consumed = True
        shift = mods.get("shift", False)

        if key == "Enter":
            current = lines[cl]
            lines[cl] = current[:cc]
            lines.insert(cl + 1, current[cc:])
            cl += 1
            cc = 0
        elif key == "Backspace":
            if cc > 0:
                lines[cl] = lines[cl][:cc - 1] + lines[cl][cc:]
                cc -= 1
            elif cl > 0:
                prev_len = len(lines[cl - 1])
                lines[cl - 1] = lines[cl - 1] + lines[cl]
                del lines[cl]
                cl -= 1
                cc = prev_len
        elif key == "Delete":
            if cc < len(lines[cl]):
                lines[cl] = lines[cl][:cc] + lines[cl][cc + 1:]
            elif cl < len(lines) - 1:
                lines[cl] = lines[cl] + lines[cl + 1]
                del lines[cl + 1]
        elif key == "ArrowLeft":
            if cc > 0:
                cc -= 1
            elif cl > 0:
                cl -= 1
                cc = len(lines[cl])
        elif key == "ArrowRight":
            if cl < len(lines) and cc < len(lines[cl]):
                cc += 1
            elif cl < len(lines) - 1:
                cl += 1
                cc = 0
        elif key == "ArrowUp":
            if cl > 0:
                cl -= 1
                clamp_col()
                if cl < scroll:
                    scroll = cl
                    self._editor_scroll[editor_id] = scroll
        elif key == "ArrowDown":
            if cl < len(lines) - 1:
                cl += 1
                clamp_col()
        elif key == "Home":
            cc = 0
        elif key == "End":
            if cl < len(lines):
                cc = len(lines[cl])
        elif key == "Tab":
            lines[cl] = lines[cl][:cc] + "    " + lines[cl][cc:]
            cc += 4
        elif key == "a" and mods.get("command", False):
            cl = 0
            cc = 0
        elif len(key) == 1:
            lines[cl] = lines[cl][:cc] + key + lines[cl][cc:]
            cc += 1
        else:
            consumed = False

        if consumed:
            clamp_col()
            # Auto-scroll to keep cursor visible
            # (visible_lines unknown here; use a default 20 — app will re-clamp)
            if cl < scroll:
                self._editor_scroll[editor_id] = cl
            elif cl >= scroll + 20:
                self._editor_scroll[editor_id] = cl - 19
            on_change(lines, cl, cc)

        return consumed

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
                delta_time = event.get("delta_time", 0.016)
                self._render_time += delta_time
                # Clear focused editor each frame — re-registered by code_editor() calls
                self._focused_editor_id = None
                self._editor_states.clear()
                ctx = RenderContext(self.width, self.height)
                ctx._time = self._render_time
                ctx._measure_cache = {}  # clear per-frame
                ctx._measure_req_id = self._measure_req_id
                ctx._app = self  # allow code_editor to register state
                if self._on_render:
                    self._on_render(ctx)
                self._measure_req_id = ctx._measure_req_id
                ctx._flush()

            elif event_type == "key":
                key = event.get("key", "")
                mods = event.get("modifiers", {})
                # Route to focused editor first, unless a Cmd-key combo
                cmd_held = mods.get("command", False) or mods.get("ctrl", False)
                editor_consumed = False
                if self._focused_editor_id and not cmd_held:
                    eid = self._focused_editor_id
                    es = self._editor_states.get(eid)
                    if es:
                        editor_consumed = self._route_key_to_editor(eid, es, key, mods)
                if not editor_consumed and self._on_key:
                    self._on_key(key, mods, self._emitter)

            elif event_type == "click":
                if self._on_click:
                    self._on_click(
                        event.get("x", 0.0),
                        event.get("y", 0.0),
                        event.get("button", "primary"),
                        self._emitter,
                    )

            elif event_type == "mouse_down":
                if self._on_mouse_down:
                    self._on_mouse_down(
                        event.get("x", 0.0),
                        event.get("y", 0.0),
                        event.get("button", "primary"),
                        self._emitter,
                    )

            elif event_type == "mouse_up":
                if self._on_mouse_up:
                    self._on_mouse_up(
                        event.get("x", 0.0),
                        event.get("y", 0.0),
                        event.get("button", "primary"),
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
