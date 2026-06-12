from __future__ import annotations

import asyncio
import json
import sys
import time
from typing import TYPE_CHECKING, Any

from ._constants import BODY
from ._theme import theme as _sdk_theme
from ._emitter import _emit, _LOCK

if TYPE_CHECKING:
    from ._app import App
    from ._emitter import Emitter

COMPACT_DEFAULT: float = 280.0
REGULAR_DEFAULT: float = 480.0

_VALID_ALIGN = frozenset({
    "left_top", "center_top", "right_top",
    "left_center", "center_center", "right_center",
    "left_bottom", "center_bottom", "right_bottom",
})

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
        self.args: list[str] = app.launch_args
        # Active host theme (light/dark + user overrides). Read colors via
        # ctx.theme.<role>, e.g. ctx.theme.accent / ctx.theme.bg / ctx.theme.danger.
        from ._theme import theme as _theme
        self.theme = _theme
        self.colors = _theme
        self._app = app
        self.emit: "Emitter" = app.emit
        # Seconds elapsed since the previous on_render call. 0.0 on first frame.
        # Use this for time-based game logic instead of calling time.time() yourself.
        self.elapsed: float = elapsed
        self._compact_threshold: float = COMPACT_DEFAULT
        self._regular_threshold: float = REGULAR_DEFAULT
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
        # Components that declare scroll interest by calling register_scroll_consumer()
        # during render. The App saves this list after each frame and routes
        # PlexiEvent::Scroll to these consumers automatically (#1802).
        self._scroll_consumers: list = []

    def register_scroll_consumer(self, component: Any) -> None:
        """Declare that `component` wants to receive scroll events this frame.

        Call from a component's render() method. The SDK will automatically
        call component.handle_scroll(delta_y) on incoming scroll events and
        schedule a re-render — no on_scroll_delta override needed in the app.
        """
        self._scroll_consumers.append(component)

    def _queue(self, obj: dict) -> None:
        """Buffer a render command for batch flush at frame_done()."""
        self._buf.append(json.dumps(obj) + "\n")

    @property
    def width(self) -> float:
        return self.w

    @property
    def height(self) -> float:
        return self.h

    @property
    def size_class(self) -> str:
        """Current size class based on pane width vs declared thresholds.

        Returns 'compact', 'regular', or 'full'.
        Thresholds come from the manifest [launch] section (or SDK defaults).
        """
        if self.w < self._compact_threshold:
            return "compact"
        elif self.w < self._regular_threshold:
            return "regular"
        return "full"

    def declare_min_size(self, width: float, height: float) -> None:
        """Override the manifest-declared minimum size at runtime.

        Emits DrawCommand::SetMinSize. The host stores this and uses it as the
        live effective minimum for this pane going forward, superseding the
        manifest value. Use when a view mode needs more space than the default
        (e.g. a detail view vs a list view).
        """
        self._queue({"type": "set_min_size", "width": width, "height": height})

    # ── Visual primitives ──
    def clear(self, fill: str = "#000000") -> None:
        """Fill the entire pane with a single color. Shorthand for a full-size rect."""
        self._queue({"type": "rect", "x": 0.0, "y": 0.0, "w": self.w, "h": self.h,
                     "fill": fill, "radius": 0.0})

    def rect(self, x: float, y: float, w: float, h: float, fill: str,
             radius: float = 0.0) -> None:
        """Draw a filled rectangle.

        Args:
            x: Left edge in logical pixels
            y: Top edge in logical pixels
            w: Width in logical pixels
            h: Height in logical pixels
            fill: Fill color as hex string (e.g., "#ff0000" or "#ff0000aa" with alpha)
            radius: Corner radius in pixels (0.0 = sharp corners, default)
        """
        if not isinstance(fill, str):
            raise TypeError(
                f"ctx.rect() fill must be a hex color string (e.g. \"#ff0000\"), "
                f"got {type(fill).__name__}: {fill!r}"
            )
        self._queue({"type": "rect", "x": x, "y": y, "w": w, "h": h,
                     "fill": fill, "radius": radius})

    def circle(self, cx: float, cy: float, r: float, fill: str) -> None:
        """Draw a filled circle.

        Args:
            cx: Center X in logical pixels
            cy: Center Y in logical pixels
            r: Radius in logical pixels
            fill: Fill color as hex string; supports 8-digit hex (#rrggbbaa) for alpha
        """
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
             align: str = "left_top",
             max_width: "float | None" = None,
             elide: bool = True,
             selectable: bool = False,
             max_lines: "int | None" = None) -> None:
        """Draw text. `align` controls how `(x, y)` maps to the text box.

        Valid values (horizontal_vertical):
          - "left_top" (default)   — (x, y) is the top-left corner.
          - "center_top"           — (x, y) is the top-center.
          - "right_top"            — (x, y) is the top-right corner.
          - "left_center"          — (x, y) is the left midline.
          - "center_center"        — (x, y) is the visual center.
          - "right_center"         — (x, y) is the right midline.
          - "left_bottom"          — (x, y) is the bottom-left corner.
          - "center_bottom"        — (x, y) is the bottom-center.
          - "right_bottom"         — (x, y) is the bottom-right corner.

        Use "center_center" when placing text inside a fixed-size container
        (a badge, a button, a pie-chart label) — the host uses real font
        metrics, which is noticeably more accurate than Python-side math.

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
        if not isinstance(text, str):
            raise TypeError(
                f"ctx.text() text must be a string, got {type(text).__name__}: {text!r}. "
                f"Convert with str(): ctx.text(x, y, str(value), ...)"
            )
        if not isinstance(color, str):
            raise TypeError(
                f"ctx.text() color must be a hex color string (e.g. \"#cdd6f4\"), "
                f"got {type(color).__name__}: {color!r}"
            )
        if not isinstance(size, (int, float)):
            raise TypeError(
                f"ctx.text() size must be a number (e.g. 14.0), "
                f"got {type(size).__name__}: {size!r}. "
                f"Use a font constant: from plexi_sdk import BODY, CAPTION, HINT"
            )
        if align not in _VALID_ALIGN:
            raise ValueError(
                f"ctx.text() align={align!r} is not valid. "
                f"Valid values: {sorted(_VALID_ALIGN)}"
            )
        d: dict = {"type": "text", "x": x, "y": y, "text": text, "size": size,
                   "color": color, "monospace": monospace, "bold": bold,
                   "align": align, "max_width": max_width, "elide": elide,
                   "selectable": selectable}
        if max_lines is not None:
            d["max_lines"] = max_lines
        self._queue(d)

    def markdown(self, x: float, y: float, w: float, text: str,
                 base_size: float = 14.0, color: "str | None" = None) -> None:
        """Render markdown text via the host's egui_commonmark renderer.

        The host creates a child Ui at (x, y) with width w and renders the
        markdown with proper formatting (bold, italic, code blocks, lists, etc.).

        `base_size` — body font size in pt (headers scale relative to this).
        `color`     — body text colour (hex); defaults to the active theme fg.
        """
        self._queue({"type": "markdown", "x": x, "y": y, "w": w, "text": text,
                     "base_size": base_size, "color": color or self.theme.fg})

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
        """Write `text` to the OS clipboard via the host.

        Uses _emit (immediate write) rather than _queue so this works from
        on_key and other hook handlers where frame_done() is never called.
        """
        _emit({"type": "copy_to_clipboard", "text": text})

    def badge(self, x: float, y_center: float, label: str,
              fill: "str | None" = None, fg: "str | None" = None,
              font_size: float = 11.0, radius: float = 6.0) -> None:
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
                     "fill": fill or _sdk_theme.accent, "fg": fg or _sdk_theme.bg, "font_size": font_size, "radius": radius})

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
                      - `color`     — color string (default: theme.fg)
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
                    {"text": "key pressed", "size": CAPTION},
                ],
                gap=12.0,
            )
        """
        if align not in _VALID_ALIGN:
            raise ValueError(
                f"ctx.text_row() align={align!r} is not valid. "
                f"Valid values: {sorted(_VALID_ALIGN)}"
            )
        # Validate and normalize items: each dict must have 'text', other keys optional.
        wire_items = []
        for item in items:
            if not isinstance(item, dict) or "text" not in item:
                raise ValueError("Each item in text_row must be a dict with 'text' key")
            wire_items.append({
                "text": item["text"],
                "color": item.get("color") or _sdk_theme.fg,
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

    async def measure_text_wrapped(self, text: str, font_size: float,
                                    max_width: float,
                                    max_lines: "int | None" = None) -> float:
        """Measure the height of `text` wrapped at `max_width` using real host font metrics.

        If `max_lines` is set, clamps the result to that many rows.
        Returns height in logical pixels.

        Call at data-load time (once per item), not inside on_render() — each call
        is an async IPC roundtrip.
        """
        import uuid as _uuid
        request_id = str(_uuid.uuid4())
        from ._emitter import _make_async_queue
        q: asyncio.Queue[float] = _make_async_queue()
        self._app._pending_measure_text_wrapped[request_id] = q
        if self._buf:
            with _LOCK:
                sys.stdout.write("".join(self._buf))
                sys.stdout.flush()
            self._buf.clear()
        payload: dict = {"type": "measure_text_wrapped", "request_id": request_id,
                         "text": text, "font_size": font_size, "max_width": max_width}
        if max_lines is not None:
            payload["max_lines"] = max_lines
        _emit(payload)
        try:
            return await asyncio.wait_for(q.get(), timeout=10.0)
        except asyncio.TimeoutError:
            self._app._pending_measure_text_wrapped.pop(request_id, None)
            raise RuntimeError(
                f"measure_text_wrapped timed out after 10s (request_id={request_id!r})"
            )

    def avatar(self, src: str, x: float, y_center: float, radius: float) -> None:
        """Render a circular clipped image.

        `src` accepts a handle from `emit.load_image()` or a local file path.
        `x` is the left edge of the circle's bounding box (circle centre = x + radius).
        `y_center` is the vertical centre of the circle.
        `radius` is the circle radius in logical pixels.
        """
        self._queue({"type": "avatar", "src": src,
                     "cx": x + radius, "cy": y_center, "radius": radius})

    def skeleton(self, x: float, y: float, w: float, h: float,
                 radius: float = 4.0) -> None:
        """Render an animated shimmer placeholder rect for loading states.

        The host drives a ~20fps pulsing animation — no app-side timer needed.
        """
        self._queue({"type": "skeleton", "x": x, "y": y, "w": w, "h": h,
                     "radius": radius})

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
        """Draw a line segment.

        Args:
            x1: Start X in logical pixels
            y1: Start Y in logical pixels
            x2: End X in logical pixels
            y2: End Y in logical pixels
            color: Line color as hex string
            width: Line width in logical pixels (default: 1.0)
        """
        self._queue({"type": "line", "x1": x1, "y1": y1, "x2": x2, "y2": y2,
                     "color": color, "width": width})

    # ── SDK v2 declarative UI entry point ──
    def render(self, tree: Any, fill: "str | None" = None) -> None:
        """Render a declarative UI tree (see `plexi_sdk.ui`). Clears the pane
        to `fill` first, then lays out `tree` into the full pane rect.

        `fill` defaults to the active host theme background (`ctx.theme.bg`).

        Example:
            from plexi_sdk.ui import Column, Header, Footer, Spacer
            ctx.render(Column([
                Header("My App"),
                Spacer(grow=True),
                Footer("status line"),
            ]))
        """
        if tree is None:
            raise TypeError(
                "ctx.render() received None. Pass a Component tree, e.g. "
                "ctx.render(Column([Label(text='hello')]))"
            )
        if isinstance(tree, str):
            raise TypeError(
                f"ctx.render() received a string {tree!r}. Pass a Component tree, not raw text. "
                "To display text, use: ctx.render(Column([Label(text='hello')]))"
            )
        if isinstance(tree, (list, tuple)):
            raise TypeError(
                f"ctx.render() received a {type(tree).__name__}. Wrap children in a container: "
                "ctx.render(Column([child1, child2, ...])) instead of ctx.render([child1, child2])"
            )
        # Local import avoids a circular dependency at module load time
        # (ui.py references RenderContext only through duck-typed `ctx`).
        from plexi_sdk.ui import render_tree
        render_tree(self, tree, fill=fill)

    def simple_list(self, items: "list[dict]", selected: int = 0,
             item_height: float = 40.0, x: float = 0.0, y: float = 0.0,
             w: "float | None" = None, h: "float | None" = None) -> None:
        """Draw a scrollable list of items with optional selection highlight.

        Args:
            items: List of item dicts, each with keys "text" (required) and optional "disabled"
            selected: Index of the highlighted item (0-indexed)
            item_height: Height of each item in logical pixels
            x: Left edge of the list
            y: Top edge of the list
            w: Width of the list (defaults to full pane width)
            h: Height of the list (defaults to remaining pane height below y)
        """
        self._queue({"type": "list", "x": x, "y": y,
                     "w": self.w if w is None else w,
                     "h": self.h - y if h is None else h,
                     "items": items, "selected": selected,
                     "item_height": item_height})

    def list_view(
        self,
        list_id: str,
        items: "list[dict]",
        selected: int = 0,
        loading: bool = False,
        error: "str | None" = None,
        x: float = 0.0,
        y: float = 0.0,
        w: float = 0.0,
        h: float = 0.0,
    ) -> None:
        """Host-native scrollable list with j/k navigation and typed row slots.

        Items must be dicts with ``"type": "row"`` or ``"type": "custom_cell"``.
        Use :class:`plexi_sdk.ui.ListRow` to build row descriptors.

        The host handles: j/k selection, scroll-to-selected, skeleton loading
        rows (when ``loading=True``), error text, and empty state.
        The app receives :meth:`on_list_select` / :meth:`on_list_activate`
        callbacks instead of managing scroll state.

        Args:
            list_id: Stable identifier for this list (must not change across frames).
            items: List of row/custom_cell dicts. Use ``ListRow(...).to_dict()``.
            selected: Index of the currently highlighted item.
            loading: When True, renders skeleton rows instead of items.
            error: When set, renders an error message instead of items.
            x: Left edge in pane-local coordinates (0 = left edge).
            y: Top edge in pane-local coordinates (0 = top of pane).
            w: Width (0 = full pane width).
            h: Height (0 = remaining height below y).
        """
        self._queue({
            "type": "list_view",
            "id": list_id,
            "x": x,
            "y": y,
            "w": w,
            "h": h,
            "items": items,
            "selected": selected,
            "loading": loading,
            "error": error,
        })

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
                    ctx.text(8, y, item, size=14, color=ctx.theme.fg)
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
                   h: float = 24.0,
                   value: "str | None" = None) -> "str | None":
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

        If `value` is provided, the host applies it as a one-shot buffer
        replacement for completions or controlled corrections.
        """
        cmd = {"type": "text_input", "id": id, "x": x, "y": y, "w": w,
               "h": h, "placeholder": placeholder, "multiline": multiline}
        if value is not None:
            cmd["value"] = value
        self._queue(cmd)
        return self._app._take_text_submission(id)

    # Logging helpers (in-frame, forwarded to host logger)
    def log(self, level: str, message: str) -> None:
        _emit({"type": "log", "level": level, "message": message})

    def info(self, message: str) -> None:  self.log("info", message)
    def warn(self, message: str) -> None:  self.log("warn", message)
    def error(self, message: str) -> None: self.log("error", message)
    def debug(self, message: str) -> None: self.log("debug", message)

    def notify(self, title: str, priority: int, body: str = "",
               level: str = "info",
               actions: "list | None" = None,
               image_inline: "dict | None" = None,
               image_pipe_id: "str | None" = None,
               timeout_secs: "int | None" = None,
               on_dismiss: "str | None" = None) -> None:
        """Post a message notification. See Emitter.notify.
        `priority` is required (int, higher = more urgent).
        `image_inline` / `image_pipe_id` (#74) optionally attach an image.
        Scope is resolved from the app's manifest — not an argument."""
        self.emit.notify(title=title, priority=priority, body=body, level=level,
                         actions=actions,
                         image_inline=image_inline, image_pipe_id=image_pipe_id,
                         timeout_secs=timeout_secs, on_dismiss=on_dismiss)

    async def notify_choice(self, title: str, options: list, priority: int,
                            body: str = "",
                            level: str = "info", required: bool = False,
                            image_inline: "dict | None" = None,
                            image_pipe_id: "str | None" = None,
                            timeout_secs: "int | None" = None,
                            on_dismiss: "str | None" = None) -> str:
        """Post a choice notification and await until the user picks.
        `priority` is required (int, higher = more urgent).
        `image_inline` / `image_pipe_id` (#74) optionally attach an image.
        Returns the chosen option's value (or label if no value set),
        or "__cancel__" if the user dismissed. Use with ``await``."""
        return await self.emit.notify_choice(title=title, options=options,
                                             priority=priority, body=body,
                                             level=level, required=required,
                                             image_inline=image_inline,
                                             image_pipe_id=image_pipe_id,
                                             timeout_secs=timeout_secs,
                                             on_dismiss=on_dismiss)

    async def notify_with_image(self, title: str, body: str, image_bytes: bytes,
                                mime: str, priority: int,
                                level: str = "info",
                                choices: "list | None" = None) -> "str | None":
        """Convenience: post a notification with an inline base64 image.
        See Emitter.notify_with_image — handles base64 + 50KB cap. Use with ``await``."""
        return await self.emit.notify_with_image(title=title, body=body,
                                                 image_bytes=image_bytes, mime=mime,
                                                 priority=priority, level=level,
                                                 choices=choices)

    async def notify_input(self, title: str, priority: int,
                           prompt: str = "", body: str = "",
                           level: str = "info", required: bool = False,
                           timeout_secs: "int | None" = None,
                           on_dismiss: "str | None" = None) -> str:
        """Post an input notification and await until the user submits.
        `priority` is required (int, higher = more urgent).
        Returns the typed text, or "__cancel__" if dismissed. Use with ``await``."""
        return await self.emit.notify_input(title=title, priority=priority,
                                            prompt=prompt, body=body,
                                            level=level, required=required,
                                            timeout_secs=timeout_secs,
                                            on_dismiss=on_dismiss)

    async def notify_and_wait(self, title: str, priority: int,
                              body: str = "", level: str = "info",
                              actions: "list | None" = None,
                              timeout_secs: "int | None" = None,
                              on_dismiss: "str | None" = None) -> str:
        """Post a message notification and await for acknowledge/cancel.
        `priority` is required (int, higher = more urgent).
        See Emitter.notify_and_wait. Use with ``await``."""
        return await self.emit.notify_and_wait(title=title, priority=priority,
                                               body=body, level=level,
                                               actions=actions,
                                               timeout_secs=timeout_secs,
                                               on_dismiss=on_dismiss)

    def notify_async(self, title: str, priority: int, body: str = "",
                     level: str = "info",
                     actions: "list | None" = None,
                     image_inline: "dict | None" = None,
                     image_pipe_id: "str | None" = None,
                     timeout_secs: "int | None" = None,
                     on_dismiss: "str | None" = None,
                     on_response: "Any" = None) -> str:
        """Non-blocking notify_and_wait. Returns immediately with notify_id.
        `priority` is required. See Emitter.notify_async."""
        return self.emit.notify_async(title=title, priority=priority, body=body,
                                      level=level, actions=actions,
                                      image_inline=image_inline,
                                      image_pipe_id=image_pipe_id,
                                      timeout_secs=timeout_secs,
                                      on_dismiss=on_dismiss,
                                      on_response=on_response)

    def notify_choice_async(self, title: str, options: list, priority: int,
                            body: str = "",
                            level: str = "info", required: bool = False,
                            image_inline: "dict | None" = None,
                            image_pipe_id: "str | None" = None,
                            timeout_secs: "int | None" = None,
                            on_dismiss: "str | None" = None,
                            on_response: "Any" = None) -> str:
        """Non-blocking notify_choice. Returns immediately with notify_id.
        `priority` is required. See Emitter.notify_choice_async."""
        return self.emit.notify_choice_async(title=title, options=options,
                                             priority=priority, body=body,
                                             level=level, required=required,
                                             image_inline=image_inline,
                                             image_pipe_id=image_pipe_id,
                                             timeout_secs=timeout_secs,
                                             on_dismiss=on_dismiss,
                                             on_response=on_response)

    def notify_input_async(self, title: str, priority: int,
                           prompt: str = "", body: str = "",
                           level: str = "info", required: bool = False,
                           timeout_secs: "int | None" = None,
                           on_dismiss: "str | None" = None,
                           on_response: "Any" = None) -> str:
        """Non-blocking notify_input. Returns immediately with notify_id.
        `priority` is required. See Emitter.notify_input_async."""
        return self.emit.notify_input_async(title=title, priority=priority,
                                            prompt=prompt, body=body,
                                            level=level, required=required,
                                            timeout_secs=timeout_secs,
                                            on_dismiss=on_dismiss,
                                            on_response=on_response)

    def notify_and_wait_async(self, title: str, priority: int,
                              body: str = "", level: str = "info",
                              actions: "list | None" = None,
                              timeout_secs: "int | None" = None,
                              on_dismiss: "str | None" = None,
                              on_response: "Any" = None) -> str:
        """Non-blocking notify_and_wait. Returns immediately with notify_id.
        `priority` is required. See Emitter.notify_and_wait_async."""
        return self.emit.notify_and_wait_async(title=title, priority=priority,
                                               body=body, level=level,
                                               actions=actions,
                                               timeout_secs=timeout_secs,
                                               on_dismiss=on_dismiss,
                                               on_response=on_response)

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
                   request_id: "str | None" = None,
                   target_context: "int | None" = None) -> None:
        """Request the host to open a new pane. Requires panes.spawn capability."""
        self.emit.spawn_pane(type_id, layout=layout, args=args, pipe_id=pipe_id,
                             from_pane_id=from_pane_id, request_id=request_id,
                             target_context=target_context)

    def query_context_state(self, context_id: int) -> None:
        """Query the state of a context. Host responds via on_context_state."""
        self.emit.query_context_state(context_id)

    # ── Component tree emission (epic #1897 B3) ────────────────────────────────

    def render_tree(self, node: "dict | Any") -> None:
        """Emit a component tree command to the host.

        node can be any object with a to_node() method (HasToNode), or a raw
        dict matching the UiNode wire format.

        Emitted as: {"type": "component_tree", "root": <UiNode dict>}
        """
        if node is None:
            raise TypeError(
                "ctx.render_tree() received None. Pass a UiNode dict or an object "
                "with a to_node() method (e.g. Tabs, Grid, Toggle, Clickable)."
            )
        if isinstance(node, dict):
            root = node
        elif hasattr(node, "to_node"):
            root = node.to_node()
        else:
            raise TypeError(
                f"ctx.render_tree() expected a dict or object with to_node(), "
                f"got {type(node).__name__}. "
                f"For L0 components (Column, Card, etc.), use ctx.render(tree) instead. "
                f"For L1 node trees (Tabs, Grid, Toggle), pass the object directly."
            )
        self._queue({"type": "component_tree", "root": root})

    # ── Declarative layout ──────────────────────────────────────────────────────

    def badge_child(self, label: str, fill: "str | None" = None, fg: "str | None" = None,
                    font_size: float = 11.0, radius: float = 8.0) -> dict:
        """Return a layout child node for a Badge. Use inside ctx.row/column/stack."""
        return {"type": "leaf", "command": {
            "type": "badge", "x": 0.0, "y": 0.0,
            "label": label, "fill": fill or _sdk_theme.accent, "fg": fg or _sdk_theme.bg,
            "font_size": font_size, "radius": radius,
        }}

    def text_child(self, text: str, size: float = 12.0, color: str = "#cdd6f4",
                   monospace: bool = False, bold: bool = False,
                   max_width: "float | None" = None, elide: bool = False,
                   selectable: bool = False) -> dict:
        """Return a layout child node for Text. Use inside ctx.row/column/stack."""
        return {"type": "leaf", "command": {
            "type": "text", "x": 0.0, "y": 0.0,
            "text": text, "size": size, "color": color,
            "monospace": monospace, "bold": bold,
            "align": "top_left", "max_width": max_width,
            "elide": elide, "selectable": selectable,
        }}

    def key_chip_child(self, label: str, font_size: float = 11.0) -> dict:
        """Return a layout child node for a KeyChip. Use inside ctx.row/column/stack."""
        return {"type": "leaf", "command": {
            "type": "key_chip", "x": 0.0, "y": 0.0,
            "label": label, "font_size": font_size,
        }}

    def layout_node(self, direction: str, children: "list[dict]", gap: float = 0.0) -> dict:
        """Return a nested layout node for use inside ctx.row/column/stack."""
        return {"type": "node", "direction": direction, "children": children, "gap": gap}

    def row(self, x: float, y: float, children: "list[dict]", gap: float = 6.0) -> None:
        """Emit a declarative flex-row layout tree.

        The host resolves all child positions using taffy (flexbox) and real
        egui font metrics. Children are layout child dicts from badge_child(),
        text_child(), key_chip_child(), or layout_node().

        `x`, `y`    — pane-relative top-left anchor.
        `children`  — list of layout child dicts.
        `gap`       — space between children in pixels (default 6.0).

        Example::

            ctx.row(
                x=24.0, y=40.0,
                children=[
                    ctx.badge_child("4 files"),
                    ctx.text_child(" modified"),
                ],
                gap=6.0,
            )
        """
        self._queue({
            "type": "layout",
            "x": x, "y": y,
            "direction": "row",
            "children": children,
            "gap": gap,
        })

    def column(self, x: float, y: float, children: "list[dict]", gap: float = 6.0) -> None:
        """Emit a declarative flex-column layout tree.

        Same as row() but stacks children top-to-bottom.
        """
        self._queue({
            "type": "layout",
            "x": x, "y": y,
            "direction": "column",
            "children": children,
            "gap": gap,
        })

    def stack(self, x: float, y: float, children: "list[dict]") -> None:
        """Emit a declarative stack layout — all children rendered at the same origin.

        Use for layering (background rect behind text, etc.).
        """
        self._queue({
            "type": "layout",
            "x": x, "y": y,
            "direction": "stack",
            "children": children,
            "gap": 0.0,
        })

    def responsive(
        self,
        x: float,
        y: float,
        tiers: "list[dict]",
    ) -> None:
        """Emit a responsive layout that adapts to the available pane aspect ratio.

        The host evaluates tiers in order and renders the first match:
        - "landscape": pane is wider than tall (ratio > 1.2)
        - "portrait": pane is taller than wide (ratio < 0.83)
        - "square": fallback for balanced aspect ratios

        Each tier dict has: aspect (str), direction (str), children (list), gap (float).

        Example::

            ctx.responsive(
                x=0.0, y=0.0,
                tiers=[
                    {"aspect": "landscape", "direction": "row",
                     "children": [ctx.text_child("wide view")], "gap": 6.0},
                    {"aspect": "portrait", "direction": "column",
                     "children": [ctx.text_child("tall view")], "gap": 4.0},
                    {"aspect": "square", "direction": "column",
                     "children": [ctx.text_child("default")], "gap": 4.0},
                ],
            )
        """
        self._queue({
            "type": "responsive",
            "x": x, "y": y,
            "tiers": tiers,
        })

    def frame_done(self) -> None:
        """Flush all buffered draw commands for this frame.

        Called automatically after on_render returns. Only call this manually
        if you're rendering in multiple phases and need intermediate updates.
        """
        self._buf.append(json.dumps({"type": "frame_done", "frame_id": self.frame_id}) + "\n")
        with _LOCK:
            sys.stdout.write("".join(self._buf))
            sys.stdout.flush()
        self._buf.clear()
