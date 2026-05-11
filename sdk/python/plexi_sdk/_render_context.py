from __future__ import annotations

import asyncio
import json
import sys
import time
from typing import TYPE_CHECKING, Any

from ._constants import FG, BG, ACCENT, BODY
from ._emitter import _emit, _LOCK

if TYPE_CHECKING:
    from ._app import App
    from ._emitter import Emitter


# ── RenderContext ──────────────────────────────────────────────────────────────

class RenderContext:
    """Passed to on_render. Accumulate draw commands; FrameDone is auto-emitted."""

    def __init__(self, frame_id: int, rect: dict, workspace_root: str,
                 capabilities: "list[str]", feature_flags: "list[str]", app: "App",
                 elapsed: float = 0.0,
                 clicks: "list[tuple[float, float]] | None" = None):
        self.frame_id = frame_id
        self.x: float = rect.get("x", 0.0)
        self.y: float = rect.get("y", 0.0)
        self.w: float = rect.get("w", 800.0)
        self.h: float = rect.get("h", 600.0)
        self.workspace_root = workspace_root
        self.capabilities = capabilities
        self.feature_flags = feature_flags
        self._app = app
        self.emit: "Emitter" = app.emit
        # Seconds elapsed since the previous on_render call. 0.0 on first frame.
        # Use this for time-based game logic instead of calling time.time() yourself.
        self.elapsed: float = elapsed
        # Buffered JSON lines — flushed as a single write in frame_done().
        self._buf: "list[str]" = []
        # Mouse state for ctx.button() hit-testing (#255).
        self._mx: float = app._mx
        self._my: float = app._my
        self._clicks: list[tuple[float, float]] = clicks or []
        # Shared reference to App's hold-timer map — mutations here persist
        # across frames (#1083). Prune expired entries each frame so the dict
        # stays bounded for apps that generate transient button IDs.
        self._btn_active_until: dict[str, float] = app._btn_active_until
        _now = time.monotonic()
        expired = [bid for bid, exp in self._btn_active_until.items() if exp <= _now]
        for bid in expired:
            del self._btn_active_until[bid]

    def _queue(self, obj: dict) -> None:
        """Buffer a render command for batch flush at frame_done()."""
        self._buf.append(json.dumps(obj) + "\n")

    @property
    def width(self) -> float:
        return self.w

    @property
    def height(self) -> float:
        return self.h

    # ── Visual primitives ──
    def clear(self, fill: str = "#000000") -> None:
        """Fill the entire pane with a single color. Shorthand for a full-size rect."""
        self._queue({"type": "rect", "x": 0.0, "y": 0.0, "w": self.w, "h": self.h,
                     "fill": fill, "radius": 0.0})

    def rect(self, x: float, y: float, w: float, h: float, fill: str,
             radius: float = 0.0) -> None:
        self._queue({"type": "rect", "x": x, "y": y, "w": w, "h": h,
                     "fill": fill, "radius": radius})

    def circle(self, cx: float, cy: float, r: float, fill: str) -> None:
        """Draw a filled circle. Alpha supported via 8-digit hex (#rrggbbaa) or dim()."""
        self._queue({"type": "circle", "cx": cx, "cy": cy, "r": r, "fill": fill})

    def arc(self, cx: float, cy: float, r: float,
            start_angle: float, end_angle: float, fill: str) -> None:
        """Draw a filled pie slice. Angles in radians, clockwise from east (right).
        Full circle: start_angle=0, end_angle=6.2832 (2*pi).
        Example pie slice: arc(cx, cy, r, 0, math.pi * 0.5, fill="#ff0000")"""
        self._queue({"type": "arc", "cx": cx, "cy": cy, "r": r,
                     "start_angle": start_angle, "end_angle": end_angle, "fill": fill})

    def text(self, x: float, y: float, text: str, size: float, color: str,
             monospace: bool = False, bold: bool = False,
             align: str = "top_left",
             max_width: "float | None" = None,
             elide: bool = True,
             selectable: bool = False) -> None:
        """Draw text. `align` controls how `(x, y)` maps to the text box:

          - "top_left" (default) — (x, y) is the top-left corner.
          - "center"              — (x, y) is the visual center.
          - "top_center"          — (x, y) is the top-center.
          - "right"               — (x, y) is the top-right corner.

        Use "center" when placing text inside a fixed-size container (a badge,
        a button, a pie-chart label) — the host uses real font metrics, which
        is noticeably more accurate than Python-side math with approximate
        character-width ratios.

        `max_width` — when set, the host clips the text at this pixel width.
        `elide`     — when True (default), a "…" is appended at the clip point;
                      when False, the text is hard-clipped with no marker.
        `selectable` — when True, the host renders the text as a real egui
                       label so the user can drag-select inside it and Cmd+C
                       copies the current selection. Default False (#200).

        `max_width`, `elide`, and `selectable` are sent explicitly on the wire
        so the host always has required fields (no serde defaults). The SDK
        fills in None / True / False when the caller omits them.
        """
        self._queue({"type": "text", "x": x, "y": y, "text": text, "size": size,
                     "color": color, "monospace": monospace, "bold": bold,
                     "align": align, "max_width": max_width, "elide": elide,
                     "selectable": selectable})

    def markdown(self, x: float, y: float, w: float, text: str,
                 base_size: float = 14.0, color: str = FG) -> None:
        """Render markdown text via the host's egui_commonmark renderer.

        The host creates a child Ui at (x, y) with width w and renders the
        markdown with proper formatting (bold, italic, code blocks, lists, etc.).

        `base_size` — body font size in pt (headers scale relative to this).
        `color`     — default text colour (hex).
        """
        self._queue({"type": "markdown", "x": x, "y": y, "w": w, "text": text,
                     "base_size": base_size, "color": color})

    def image(self, src: str, x: float, y: float, w: float, h: float,
              fit: str = "contain") -> None:
        """Draw an image from a file path or URL.

        `fit` controls scaling: "contain" (default, letterbox), "cover"
        (crop to fill), or "fill" (stretch). Host must implement
        DrawCommand::Image for this to render.
        """
        self._queue({"type": "image", "src": src, "x": x, "y": y, "w": w, "h": h,
                     "fit": fit})

    def copy_to_clipboard(self, text: str) -> None:
        """Convenience shortcut for `emit.copy_to_clipboard(text)` (#146)."""
        self._queue({"type": "copy_to_clipboard", "text": text})

    def badge(self, x: float, y_center: float, label: str,
              fill: str = ACCENT, fg: str = BG,
              font_size: float = 11.0, radius: float = 8.0) -> None:
        """Render a host-measured pill badge.

        The host measures the label with real egui font metrics, sizes the pill
        (text_w + padding), and centres the text — no Python width math.

        `x`        — left edge of the badge.
        `y_center` — vertical centre of the badge.
        `label`    — text to display.
        `fill`     — pill background colour.
        `fg`       — text colour.
        `font_size`— label pt size.
        `radius`   — corner radius (8.0 = fully rounded pill; 4.0 = tag chip).
        """
        self._queue({"type": "badge", "x": x, "y": y_center, "label": label,
                     "fill": fill, "fg": fg, "font_size": font_size, "radius": radius})

    def button(self, id: str, x: float, y: float, w: float, h: float,
               label: str,
               fill: str = "#313244",
               hover_fill: str = "#45475a",
               active_fill: str = "#585b70",
               text_color: str = "#cdd6f4",
               font_size: float = 13.0,
               radius: float = 6.0) -> bool:
        """Draw a button and return True if it was clicked this frame.

        Hover state is derived from the current mouse position (requires
        mouse_tracking = true in the manifest, or degrades gracefully without it).
        Click state is derived from buffered click events since the previous frame.

        `id` is currently unused but reserved for future focus tracking.
        """
        hovered = (x <= self._mx <= x + w) and (y <= self._my <= y + h)
        clicked = any(
            (x <= cx <= x + w) and (y <= cy <= y + h)
            for cx, cy in self._clicks
        )
        now = time.monotonic()
        if clicked:
            self._btn_active_until[id] = now + 0.15
            self.schedule_render(after_ms=150)
            self.info(f"button clicked: id={id!r}")
        active = now < self._btn_active_until.get(id, 0.0)
        bg = active_fill if (clicked or active) else (hover_fill if hovered else fill)
        self.rect(x, y, w, h, fill=bg, radius=radius)
        self.text(x + w / 2, y + h / 2, label, size=font_size,
                  color=text_color, align="center")
        return clicked

    def key_chip_row(self, x: float, y: float, keys: "list[str]",
                     description: "str | None" = None,
                     font_size: float = 11.0) -> None:
        """Render a row of keycap chips followed by an optional description.

        The host measures each chip with real egui font metrics and flows them
        left-to-right — no Python width math.

        `x`, `y`     — origin of the chip row (left edge, top of chips).
        `keys`       — list of key labels, e.g. ["⌘", "K"] for a chord.
        `description`— optional trailing text label after the chips.
        `font_size`  — chip label pt size.

        For multi-pair shortcut rows with horizontal flow + multi-line
        wrapping, use `ctx.shortcuts(...)` instead — that's the right
        primitive for footer-style "[k] desc · [j] desc · …" layouts.
        """
        self._queue({"type": "key_chip_row", "x": x, "y": y, "keys": keys,
                     "description": description, "font_size": font_size})

    def text_row(self, x: float, y: float, items: "list[dict]",
                 gap: float = 8.0, align: str = "left_center") -> None:
        """Render multiple text segments horizontally with configurable spacing.

        The host measures each text segment with real font metrics and flows
        them left-to-right — no Python width math.

        `x`, `y`    — origin of the text row.
        `items`     — list of text segment dicts, each with keys:
                      - `text`      — string to display (required)
                      - `color`     — color string (default: FG)
                      - `size`      — font size in pt (default: BODY)
                      - `monospace` — bool (default: False)
        `gap`       — spacing between items in pixels (default: 8.0).
        `align`     — vertical alignment; use "left_center" to center items
                      on the y-axis at their midline (default: "left_center").

        Example::

            ctx.text_row(
                x=24.0, y=200.0,
                items=[
                    {"text": "12:34:56", "color": "#6c7086", "size": CAPTION, "monospace": True},
                    {"text": "key pressed", "color": FG, "size": CAPTION},
                ],
                gap=12.0,
            )
        """
        # Validate and normalize items: each dict must have 'text', other keys optional.
        wire_items = []
        for item in items:
            if not isinstance(item, dict) or "text" not in item:
                raise ValueError("Each item in text_row must be a dict with 'text' key")
            wire_items.append({
                "text": item["text"],
                "color": item.get("color", FG),
                "size": item.get("size", BODY),
                "monospace": item.get("monospace", False),
            })
        self._queue({"type": "text_row", "x": x, "y": y, "items": wire_items,
                     "gap": gap, "align": align})

    def shortcuts(self, x: float, y: float, max_width: float,
                  pairs: "list[tuple]", font_size: float = 11.0) -> None:
        """Render a multi-group shortcut row with host-measured layout.

        The host owns ALL geometry: chip widths from real font metrics,
        horizontal flow with inter-group spacing, multi-line wrapping
        when the next group would exceed `max_width`. SDK callers send
        one DrawCommand and trust the result — no Python width math,
        no truncation, no overflow.

        `pairs` is a list of `(keys, description)` tuples where `keys`
        is either a single string or a list of strings (multi-key
        chord). Example::

            ctx.shortcuts(
                x=24.0, y=12.0, max_width=ctx.w - 48.0,
                pairs=[
                    (["[", "]"], "week"),
                    ("t", "today"),
                    (["j", "k"], "commit"),
                    ("?", "help"),
                ],
            )
        """
        # Normalise (keys-or-key, desc) → ShortcutPair on the wire.
        wire_pairs = []
        for keys_or_key, desc in pairs:
            if isinstance(keys_or_key, str):
                wire_keys = [keys_or_key]
            else:
                wire_keys = list(keys_or_key)
            wire_pairs.append({"keys": wire_keys, "description": desc or ""})
        self._queue({"type": "shortcuts", "x": x, "y": y,
                     "max_width": max_width, "pairs": wire_pairs,
                     "font_size": font_size})

    async def measure_text(self, text: str, font_size: float,
                           monospace: bool = False) -> "tuple[float, float]":
        """Measure `text` at `font_size` using the host's real font metrics.

        Sends a `MeasureText` DrawCommand and awaits `TextMeasured` from the
        host. Returns `(width, height)` in logical pixels.

        Use this only when layout depends on measured text width (e.g. flowing
        multiple badges horizontally). Avoid on hot render paths — prefer
        passing `max_width` on `ctx.text()` for simple truncation.

        Must be called with ``await`` from an async hook.
        """
        import uuid
        request_id = str(uuid.uuid4())
        from ._emitter import _make_async_queue
        q: asyncio.Queue[tuple[float, float]] = _make_async_queue()
        self._app._pending_measure_text[request_id] = q
        # Flush any buffered draw commands before the request so the host
        # processes prior frame content before responding to the measure.
        if self._buf:
            with _LOCK:
                sys.stdout.write("".join(self._buf))
                sys.stdout.flush()
            self._buf.clear()
        _emit({"type": "measure_text", "request_id": request_id,
               "text": text, "font_size": font_size, "monospace": monospace})
        return await q.get()

    def push_clip(self, x: float, y: float, w: float, h: float) -> None:
        """Push a clip rect onto the host's clip stack.

        All subsequent draws are clipped to the intersection of this rect with
        the current top of the stack (or the pane rect if the stack is empty).
        Must be balanced with a matching `pop_clip()`. Use `_render_clipped`
        on Component subclasses instead of calling this directly.
        """
        self._queue({"type": "push_clip", "x": x, "y": y, "w": w, "h": h})

    def pop_clip(self) -> None:
        """Pop the most recently pushed clip rect. Must balance a `push_clip()`."""
        self._queue({"type": "pop_clip"})

    def line(self, x1: float, y1: float, x2: float, y2: float,
             color: str, width: float = 1.0) -> None:
        self._queue({"type": "line", "x1": x1, "y1": y1, "x2": x2, "y2": y2,
                     "color": color, "width": width})

    # ── SDK v2 declarative UI entry point ──
    def render(self, tree: Any, fill: str = "#1e1e2e") -> None:
        """Render a declarative UI tree (see `plexi_sdk.ui`). Clears the pane
        to `fill` first, then lays out `tree` into the full pane rect.

        Example:
            from plexi_sdk.ui import Column, Header, Footer, Spacer
            ctx.render(Column([
                Header("My App"),
                Spacer(grow=True),
                Footer("status line"),
            ]))
        """
        # Local import avoids a circular dependency at module load time
        # (ui.py references RenderContext only through duck-typed `ctx`).
        from plexi_sdk.ui import render_tree
        render_tree(self, tree, fill=fill)

    def list_view(self, items: "list[dict]", selected: int = 0,
             item_height: float = 40.0, x: float = 0.0, y: float = 0.0,
             w: "float | None" = None, h: "float | None" = None) -> None:
        self._queue({"type": "list", "x": x, "y": y,
                     "w": self.w if w is None else w,
                     "h": self.h - y if h is None else h,
                     "items": items, "selected": selected,
                     "item_height": item_height})

    def begin_scroll(self, id: str, x: float, y: float, w: float, h: float,
                     content_height: float) -> None:
        """Begin a host-managed vertical scroll region (#446).

        Declares a viewport at (x, y, w, h) within this pane. This rect does
        two things: it clips all draw commands until the matching `end_scroll()`
        call, AND it defines where the host detects scroll gestures. The host
        only fires `on_scroll` when the pointer is inside this rect.

        **Common mistake:** if your scrollable list occupies only part of the
        pane (e.g. the right half), set x/w to cover the full pane width anyway
        so the user can scroll from anywhere — not just when hovering over the
        list. Clipping is only applied to content drawn *inside* the block, so
        content drawn before `begin_scroll` is unaffected regardless of x/w.

        The host tracks the scroll position across frames and calls
        `on_scroll(ctx, id, offset_y)` whenever the user scrolls so the app
        can re-render content translated by `offset_y`.

        `content_height` is the total virtual height of the scrollable content
        in logical pixels — the host uses this to size the scrollbar thumb.
        Pass the full height of all items even if most are off-screen.

        `id` must be stable across frames — use a descriptive string rather
        than a counter. Typical pattern::

            def on_scroll(self, ctx, id, offset_y):
                if id == "main-list":
                    self.scroll_y = offset_y

            def on_render(self, ctx):
                ctx.begin_scroll("main-list", 0, 48, ctx.w, ctx.h - 48,
                                 content_height=100 * ROW_H)
                for i, item in enumerate(self.items):
                    y = i * ROW_H - self.scroll_y
                    ctx.text(8, y, item, size=14, color=FG)
                ctx.end_scroll()
        """
        self._queue({"type": "begin_scroll", "id": id, "x": x, "y": y,
                     "w": w, "h": h, "content_height": content_height})

    def end_scroll(self) -> None:
        """Close the most recently opened scroll region. Must balance begin_scroll."""
        self._queue({"type": "end_scroll"})

    def text_input(self, id: str, x: float, y: float, w: float,
                   placeholder: str = "",
                   multiline: bool = False,
                   h: float = 24.0) -> "str | None":
        """Text input — host-owned buffer, submit-only.

        Emits a `DrawCommand::TextInput` and returns the most recently
        submitted value for `id` if any landed since the previous frame,
        else `None`. The host owns the buffer entirely — typed
        characters never reach the app between keystrokes. On Enter the
        host emits `PlexiEvent::TextSubmitted { id, value }` and clears
        its buffer.

        The host auto-focuses the widget on its first visible frame so
        the user can type immediately without clicking.

        When `multiline=True`, Enter submits and Shift+Enter inserts a
        newline. When `multiline=False` (default), Enter submits
        immediately.

        Pattern (poll on every frame)::

            submitted = ctx.text_input("note", x=12, y=12, w=300,
                                       placeholder="Type a note…")
            if submitted is not None:
                save_note(submitted)

        Real-time validation (per-keystroke access) is out of scope —
        see issue #283.
        """
        self._queue({"type": "text_input", "id": id, "x": x, "y": y, "w": w,
                     "h": h, "placeholder": placeholder, "multiline": multiline})
        return self._app._take_text_submission(id)

    # Logging helpers (in-frame, forwarded to host logger)
    def log(self, level: str, message: str) -> None:
        _emit({"type": "log", "level": level, "message": message})

    def info(self, message: str) -> None:  self.log("info", message)
    def warn(self, message: str) -> None:  self.log("warn", message)
    def error(self, message: str) -> None: self.log("error", message)
    def debug(self, message: str) -> None: self.log("debug", message)

    def notify(self, title: str, body: str = "", level: str = "info",
               priority: "int | None" = None,
               actions: "list | None" = None,
               image_inline: "dict | None" = None,
               image_pipe_id: "str | None" = None,
               timeout_secs: "int | None" = None,
               on_dismiss: "str | None" = None) -> None:
        """Post a message notification. See Emitter.notify.
        `priority` is required (int, higher = more urgent).
        `image_inline` / `image_pipe_id` (#74) optionally attach an image.
        Scope is resolved from the app's manifest — not an argument."""
        self.emit.notify(title=title, body=body, level=level,
                         priority=priority, actions=actions,
                         image_inline=image_inline, image_pipe_id=image_pipe_id,
                         timeout_secs=timeout_secs, on_dismiss=on_dismiss)

    async def notify_choice(self, title: str, options: list, body: str = "",
                            level: str = "info", required: bool = False,
                            priority: "int | None" = None,
                            image_inline: "dict | None" = None,
                            image_pipe_id: "str | None" = None,
                            timeout_secs: "int | None" = None,
                            on_dismiss: "str | None" = None) -> str:
        """Post a choice notification and await until the user picks.
        `priority` is required (int, higher = more urgent).
        `image_inline` / `image_pipe_id` (#74) optionally attach an image.
        Returns the chosen option's value (or label if no value set),
        or "__cancel__" if the user dismissed. Use with ``await``."""
        return await self.emit.notify_choice(title=title, options=options, body=body,
                                             level=level, required=required,
                                             priority=priority,
                                             image_inline=image_inline,
                                             image_pipe_id=image_pipe_id,
                                             timeout_secs=timeout_secs,
                                             on_dismiss=on_dismiss)

    async def notify_with_image(self, title: str, body: str, image_bytes: bytes,
                                mime: str, level: str = "info",
                                priority: "int | None" = None,
                                choices: "list | None" = None) -> "str | None":
        """Convenience: post a notification with an inline base64 image.
        See Emitter.notify_with_image — handles base64 + 50KB cap. Use with ``await``."""
        return await self.emit.notify_with_image(title=title, body=body,
                                                 image_bytes=image_bytes, mime=mime,
                                                 level=level, priority=priority,
                                                 choices=choices)

    async def notify_input(self, title: str, prompt: str = "", body: str = "",
                           level: str = "info", required: bool = False,
                           priority: "int | None" = None,
                           timeout_secs: "int | None" = None,
                           on_dismiss: "str | None" = None) -> str:
        """Post an input notification and await until the user submits.
        `priority` is required (int, higher = more urgent).
        Returns the typed text, or "__cancel__" if dismissed. Use with ``await``."""
        return await self.emit.notify_input(title=title, prompt=prompt, body=body,
                                            level=level, required=required,
                                            priority=priority,
                                            timeout_secs=timeout_secs,
                                            on_dismiss=on_dismiss)

    async def notify_and_wait(self, title: str, body: str = "", level: str = "info",
                              actions: "list | None" = None,
                              priority: "int | None" = None,
                              timeout_secs: "int | None" = None,
                              on_dismiss: "str | None" = None) -> str:
        """Post a message notification and await for acknowledge/cancel.
        `priority` is required (int, higher = more urgent).
        See Emitter.notify_and_wait. Use with ``await``."""
        return await self.emit.notify_and_wait(title=title, body=body, level=level,
                                               actions=actions, priority=priority,
                                               timeout_secs=timeout_secs,
                                               on_dismiss=on_dismiss)

    def status_summary(self, text: str) -> None:
        _emit({"type": "status_summary", "text": text})

    def set_timer(self, timer_id: str, after_ms: int) -> None:
        self.emit.set_timer(timer_id, after_ms)

    def cancel_timer(self, timer_id: str) -> None:
        self.emit.cancel_timer(timer_id)

    def set_mouse_tracking(self, enabled: bool) -> None:
        """Enable or disable on_mouse_move delivery for this pane.
        Call with True in on_init to start receiving mouse-move events.
        Delegates to emit.set_mouse_tracking()."""
        self.emit.set_mouse_tracking(enabled)

    def schedule_render(self, after_ms: int = 16) -> None:
        """Ask the host to send a new Render event after `after_ms` milliseconds.
        Delegates to emit.schedule_render(). 16 ms ≈ 60 fps. 32 ms ≈ 30 fps."""
        self.emit.schedule_render(after_ms=after_ms)

    async def get_secret(self, key: str) -> "str | None":
        """Request a secret by key. Alias for emit.get_secret(). Use with ``await``."""
        return await self.emit.get_secret(key)

    def load_state(self) -> dict:
        """Return persisted app state loaded at startup. {} if nothing saved."""
        return dict(self._app._app_state)

    def save_state(self, state: dict) -> None:
        """Persist state to workspace (if workspace active) or global storage.
        Fire-and-forget — no acknowledgement from host."""
        _emit({"type": "save_app_state", "payload": state})

    async def http_request(self, url: str, method: str = "GET",
                           headers: "dict[str, str] | None" = None,
                           body: "str | None" = None) -> str:
        """HTTP request brokered through the host. Requires net.http capability. Use with ``await``."""
        return await self.emit.http_request(url, method=method, headers=headers, body=body)

    async def ai_query(self, model_tier: str, system: str,
                       messages: "list[dict]",
                       tools: "list[dict] | None" = None) -> object:
        """AI broker call through the host. Requires ai.query capability. Use with ``await``."""
        return await self.emit.ai_query(model_tier=model_tier, system=system,
                                        messages=messages, tools=tools)

    def spawn_pane(self, type_id: str, layout: str = "split_v",
                   args: "list[str] | None" = None,
                   pipe_id: "str | None" = None,
                   from_pane_id: "int | None" = None,
                   request_id: "str | None" = None) -> None:
        """Request the host to open a new pane. Requires panes.spawn capability."""
        self.emit.spawn_pane(type_id, layout=layout, args=args, pipe_id=pipe_id,
                             from_pane_id=from_pane_id, request_id=request_id)

    def frame_done(self) -> None:
        self._buf.append(json.dumps({"type": "frame_done", "frame_id": self.frame_id}) + "\n")
        with _LOCK:
            sys.stdout.write("".join(self._buf))
            sys.stdout.flush()
        self._buf.clear()
