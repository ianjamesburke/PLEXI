---
title: "RenderContext"
description: "Drawing API for rendering pane content"
verified_version: "0.0.505"
---

# RenderContext

Passed to on_render. Accumulate draw commands; FrameDone is auto-emitted.

## Properties

| Property | Type | Description |
|----------|------|-------------|
| `x` | `float` | Pane origin x (usually 0) |
| `y` | `float` | Pane origin y (usually 0) |
| `w` | `float` | Pane width in logical pixels |
| `h` | `float` | Pane height in logical pixels |
| `width` | `float` | Alias for `w` |
| `height` | `float` | Alias for `h` |
| `frame_id` | `int` | Monotonically increasing render counter |
| `elapsed` | `float` | Seconds since previous on_render (0.0 on first frame) |
| `workspace_root` | `str` | Absolute path to the workspace root |
| `capabilities` | `list[str]` | Granted capability strings |
| `feature_flags` | `list[str]` | Enabled feature flag strings |
| `emit` | `Emitter` | Emitter instance for out-of-frame commands |

## Methods

### `declare_min_size`

```python
def declare_min_size(width: float, height: float) -> None
```

Override the manifest-declared minimum size at runtime.

Emits DrawCommand::SetMinSize. The host stores this and uses it as the
live effective minimum for this pane going forward, superseding the
manifest value. Use when a view mode needs more space than the default
(e.g. a detail view vs a list view).

---

### `clear`

```python
def clear(fill: str = '#000000') -> None
```

Fill the entire pane with a single color. Shorthand for a full-size rect.

---

### `rect`

```python
def rect(x: float, y: float, w: float, h: float, fill: str, radius: float = 0.0) -> None
```

Draw a filled rectangle.

Args:
    x: Left edge in logical pixels
    y: Top edge in logical pixels
    w: Width in logical pixels
    h: Height in logical pixels
    fill: Fill color as hex string (e.g., "#ff0000" or "#ff0000aa" with alpha)
    radius: Corner radius in pixels (0.0 = sharp corners, default)

---

### `circle`

```python
def circle(cx: float, cy: float, r: float, fill: str) -> None
```

Draw a filled circle.

Args:
    cx: Center X in logical pixels
    cy: Center Y in logical pixels
    r: Radius in logical pixels
    fill: Fill color as hex string; supports 8-digit hex (#rrggbbaa) for alpha

---

### `arc`

```python
def arc(cx: float, cy: float, r: float, start_angle: float, end_angle: float, fill: str) -> None
```

Draw a filled pie slice. Angles in radians, clockwise from east (right).
Full circle: start_angle=0, end_angle=6.2832 (2*pi).
Example pie slice: arc(cx, cy, r, 0, math.pi * 0.5, fill="#ff0000")

---

### `text`

```python
def text(x: float, y: float, text: str, size: float, color: str, monospace: bool = False, bold: bool = False, align: str = 'top_left', max_width: 'float | None' = None, elide: bool = True, selectable: bool = False, max_lines: 'int | None' = None) -> None
```

Draw text. `align` controls how `(x, y)` maps to the text box:

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

---

### `markdown`

```python
def markdown(x: float, y: float, w: float, text: str, base_size: float = 14.0, color: str = FG) -> None
```

Render markdown text via the host's egui_commonmark renderer.

The host creates a child Ui at (x, y) with width w and renders the
markdown with proper formatting (bold, italic, code blocks, lists, etc.).

`base_size` — body font size in pt (headers scale relative to this).
`color`     — default text colour (hex).

---

### `image`

```python
def image(src: str, x: float, y: float, w: float, h: float, fit: str = 'contain') -> None
```

Draw an image from a file path or URL.

`fit` controls scaling: "contain" (default, letterbox), "cover"
(crop to fill), or "fill" (stretch). Host must implement
DrawCommand::Image for this to render.

---

### `copy_to_clipboard`

```python
def copy_to_clipboard(text: str) -> None
```

Write `text` to the OS clipboard via the host.

Uses _emit (immediate write) rather than _queue so this works from
on_key and other hook handlers where frame_done() is never called.

---

### `badge`

```python
def badge(x: float, y_center: float, label: str, fill: str = ACCENT, fg: str = BG, font_size: float = 11.0, radius: float = 6.0) -> None
```

Render a host-measured pill badge.

The host measures the label with real egui font metrics, sizes the pill
(text_w + padding), and centres the text — no Python width math.

`x`        — left edge of the badge.
`y_center` — vertical centre of the badge.
`label`    — text to display.
`fill`     — pill background colour.
`fg`       — text colour.
`font_size`— label pt size.
`radius`   — corner radius (8.0 = fully rounded pill; 4.0 = tag chip).

---

### `button`

```python
def button(id: str, x: float, y: float, w: float, h: float, label: str, fill: str = '#313244', hover_fill: str = '#45475a', active_fill: str = '#585b70', text_color: str = '#cdd6f4', font_size: float = 13.0, radius: float = 6.0) -> bool
```

Draw a button and return True if it was clicked this frame.

Hover state is derived from the current mouse position (requires
mouse_tracking = true in the manifest, or degrades gracefully without it).
Click state is derived from buffered click events since the previous frame.

`id` is currently unused but reserved for future focus tracking.

---

### `key_chip_row`

```python
def key_chip_row(x: float, y: float, keys: 'list[str]', description: 'str | None' = None, font_size: float = 11.0) -> None
```

Render a row of keycap chips followed by an optional description.

The host measures each chip with real egui font metrics and flows them
left-to-right — no Python width math.

`x`, `y`     — origin of the chip row (left edge, top of chips).
`keys`       — list of key labels, e.g. ["⌘", "K"] for a chord.
`description`— optional trailing text label after the chips.
`font_size`  — chip label pt size.

For multi-pair shortcut rows with horizontal flow + multi-line
wrapping, use `ctx.shortcuts(...)` instead — that's the right
primitive for footer-style "[k] desc · [j] desc · …" layouts.

---

### `text_row`

```python
def text_row(x: float, y: float, items: 'list[dict]', gap: float = 8.0, align: str = 'left_center') -> None
```

Render multiple text segments horizontally with configurable spacing.

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

Example:

    ctx.text_row(
        x=24.0, y=200.0,
        items=[
            {"text": "12:34:56", "color": "#6c7086", "size": CAPTION, "monospace": True},
            {"text": "key pressed", "color": FG, "size": CAPTION},
        ],
        gap=12.0,
    )

---

### `shortcuts`

```python
def shortcuts(x: float, y: float, max_width: float, pairs: 'list[tuple]', font_size: float = 11.0) -> None
```

Render a multi-group shortcut row with host-measured layout.

The host owns ALL geometry: chip widths from real font metrics,
horizontal flow with inter-group spacing, multi-line wrapping
when the next group would exceed `max_width`. SDK callers send
one DrawCommand and trust the result — no Python width math,
no truncation, no overflow.

`pairs` is a list of `(keys, description)` tuples where `keys`
is either a single string or a list of strings (multi-key
chord). Example:

    ctx.shortcuts(
        x=24.0, y=12.0, max_width=ctx.w - 48.0,
        pairs=[
            (["[", "]"], "week"),
            ("t", "today"),
            (["j", "k"], "commit"),
            ("?", "help"),
        ],
    )

---

### `measure_text`

```python
async def measure_text(text: str, font_size: float, monospace: bool = False) -> 'tuple[float, float]'
```

Measure `text` at `font_size` using the host's real font metrics.

Sends a `MeasureText` DrawCommand and awaits `TextMeasured` from the
host. Returns `(width, height)` in logical pixels.

Use this only when layout depends on measured text width (e.g. flowing
multiple badges horizontally). Avoid on hot render paths — prefer
passing `max_width` on `ctx.text()` for simple truncation.

Must be called with ``await`` from an async hook.

---

### `measure_text_wrapped`

```python
async def measure_text_wrapped(text: str, font_size: float, max_width: float, max_lines: 'int | None' = None) -> float
```

Measure the height of `text` wrapped at `max_width` using real host font metrics.

If `max_lines` is set, clamps the result to that many rows.
Returns height in logical pixels.

Call at data-load time (once per item), not inside on_render() — each call
is an async IPC roundtrip.

---

### `avatar`

```python
def avatar(src: str, x: float, y_center: float, radius: float) -> None
```

Render a circular clipped image.

`src` accepts a handle from `emit.load_image()` or a local file path.
`x` is the left edge of the circle's bounding box (circle centre = x + radius).
`y_center` is the vertical centre of the circle.
`radius` is the circle radius in logical pixels.

---

### `skeleton`

```python
def skeleton(x: float, y: float, w: float, h: float, radius: float = 4.0) -> None
```

Render an animated shimmer placeholder rect for loading states.

The host drives a ~20fps pulsing animation — no app-side timer needed.

---

### `push_clip`

```python
def push_clip(x: float, y: float, w: float, h: float) -> None
```

Push a clip rect onto the host's clip stack.

All subsequent draws are clipped to the intersection of this rect with
the current top of the stack (or the pane rect if the stack is empty).
Must be balanced with a matching `pop_clip()`. Use `_render_clipped`
on Component subclasses instead of calling this directly.

---

### `pop_clip`

```python
def pop_clip() -> None
```

Pop the most recently pushed clip rect. Must balance a `push_clip()`.

---

### `line`

```python
def line(x1: float, y1: float, x2: float, y2: float, color: str, width: float = 1.0) -> None
```

Draw a line segment.

Args:
    x1: Start X in logical pixels
    y1: Start Y in logical pixels
    x2: End X in logical pixels
    y2: End Y in logical pixels
    color: Line color as hex string
    width: Line width in logical pixels (default: 1.0)

---

### `render`

```python
def render(tree: Any, fill: str = '#1e1e2e') -> None
```

Render a declarative UI tree (see `plexi_sdk.ui`). Clears the pane
to `fill` first, then lays out `tree` into the full pane rect.

Example:
    from plexi_sdk.ui import Column, Header, Footer, Spacer
    ctx.render(Column([
        Header("My App"),
        Spacer(grow=True),
        Footer("status line"),
    ]))

---

### `list_view`

```python
def list_view(items: 'list[dict]', selected: int = 0, item_height: float = 40.0, x: float = 0.0, y: float = 0.0, w: 'float | None' = None, h: 'float | None' = None) -> None
```

Draw a scrollable list of items with optional selection highlight.

Args:
    items: List of item dicts, each with keys "text" (required) and optional "disabled"
    selected: Index of the highlighted item (0-indexed)
    item_height: Height of each item in logical pixels
    x: Left edge of the list
    y: Top edge of the list
    w: Width of the list (defaults to full pane width)
    h: Height of the list (defaults to remaining pane height below y)

---

### `begin_scroll`

```python
def begin_scroll(id: str, x: float, y: float, w: float, h: float, content_height: float) -> None
```

Begin a host-managed vertical scroll region (#446).

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
than a counter. Typical pattern:

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

---

### `end_scroll`

```python
def end_scroll() -> None
```

Close the most recently opened scroll region. Must balance begin_scroll.

---

### `text_input`

```python
def text_input(id: str, x: float, y: float, w: float, placeholder: str = '', multiline: bool = False, h: float = 24.0) -> 'str | None'
```

Text input — host-owned buffer, submit-only.

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

Pattern (poll on every frame):

    submitted = ctx.text_input("note", x=12, y=12, w=300,
                               placeholder="Type a note…")
    if submitted is not None:
        save_note(submitted)

Real-time validation (per-keystroke access) is out of scope —
see issue #283.

---

### `log`

```python
def log(level: str, message: str) -> None
```

---

### `info`

```python
def info(message: str) -> None
```

---

### `warn`

```python
def warn(message: str) -> None
```

---

### `error`

```python
def error(message: str) -> None
```

---

### `debug`

```python
def debug(message: str) -> None
```

---

### `notify`

```python
def notify(title: str, priority: int, body: str = '', level: str = 'info', actions: 'list | None' = None, image_inline: 'dict | None' = None, image_pipe_id: 'str | None' = None, timeout_secs: 'int | None' = None, on_dismiss: 'str | None' = None) -> None
```

Post a message notification. See Emitter.notify.
`priority` is required (int, higher = more urgent).
`image_inline` / `image_pipe_id` (#74) optionally attach an image.
Scope is resolved from the app's manifest — not an argument.

---

### `notify_choice`

```python
async def notify_choice(title: str, options: list, priority: int, body: str = '', level: str = 'info', required: bool = False, image_inline: 'dict | None' = None, image_pipe_id: 'str | None' = None, timeout_secs: 'int | None' = None, on_dismiss: 'str | None' = None) -> str
```

Post a choice notification and await until the user picks.
`priority` is required (int, higher = more urgent).
`image_inline` / `image_pipe_id` (#74) optionally attach an image.
Returns the chosen option's value (or label if no value set),
or "__cancel__" if the user dismissed. Use with ``await``.

---

### `notify_with_image`

```python
async def notify_with_image(title: str, body: str, image_bytes: bytes, mime: str, priority: int, level: str = 'info', choices: 'list | None' = None) -> 'str | None'
```

Convenience: post a notification with an inline base64 image.
See Emitter.notify_with_image — handles base64 + 50KB cap. Use with ``await``.

---

### `notify_input`

```python
async def notify_input(title: str, priority: int, prompt: str = '', body: str = '', level: str = 'info', required: bool = False, timeout_secs: 'int | None' = None, on_dismiss: 'str | None' = None) -> str
```

Post an input notification and await until the user submits.
`priority` is required (int, higher = more urgent).
Returns the typed text, or "__cancel__" if dismissed. Use with ``await``.

---

### `notify_and_wait`

```python
async def notify_and_wait(title: str, priority: int, body: str = '', level: str = 'info', actions: 'list | None' = None, timeout_secs: 'int | None' = None, on_dismiss: 'str | None' = None) -> str
```

Post a message notification and await for acknowledge/cancel.
`priority` is required (int, higher = more urgent).
See Emitter.notify_and_wait. Use with ``await``.

---

### `notify_async`

```python
def notify_async(title: str, priority: int, body: str = '', level: str = 'info', actions: 'list | None' = None, image_inline: 'dict | None' = None, image_pipe_id: 'str | None' = None, timeout_secs: 'int | None' = None, on_dismiss: 'str | None' = None, on_response: 'Any' = None) -> str
```

Non-blocking notify_and_wait. Returns immediately with notify_id.
`priority` is required. See Emitter.notify_async.

---

### `notify_choice_async`

```python
def notify_choice_async(title: str, options: list, priority: int, body: str = '', level: str = 'info', required: bool = False, image_inline: 'dict | None' = None, image_pipe_id: 'str | None' = None, timeout_secs: 'int | None' = None, on_dismiss: 'str | None' = None, on_response: 'Any' = None) -> str
```

Non-blocking notify_choice. Returns immediately with notify_id.
`priority` is required. See Emitter.notify_choice_async.

---

### `notify_input_async`

```python
def notify_input_async(title: str, priority: int, prompt: str = '', body: str = '', level: str = 'info', required: bool = False, timeout_secs: 'int | None' = None, on_dismiss: 'str | None' = None, on_response: 'Any' = None) -> str
```

Non-blocking notify_input. Returns immediately with notify_id.
`priority` is required. See Emitter.notify_input_async.

---

### `notify_and_wait_async`

```python
def notify_and_wait_async(title: str, priority: int, body: str = '', level: str = 'info', actions: 'list | None' = None, timeout_secs: 'int | None' = None, on_dismiss: 'str | None' = None, on_response: 'Any' = None) -> str
```

Non-blocking notify_and_wait. Returns immediately with notify_id.
`priority` is required. See Emitter.notify_and_wait_async.

---

### `status_summary`

```python
def status_summary(text: str) -> None
```

---

### `set_timer`

```python
def set_timer(timer_id: str, after_ms: int) -> None
```

---

### `cancel_timer`

```python
def cancel_timer(timer_id: str) -> None
```

---

### `set_mouse_tracking`

```python
def set_mouse_tracking(enabled: bool) -> None
```

Enable or disable on_mouse_move delivery for this pane.
Call with True in on_init to start receiving mouse-move events.
Delegates to emit.set_mouse_tracking().

---

### `schedule_render`

```python
def schedule_render(after_ms: int = 16) -> None
```

Ask the host to send a new Render event after `after_ms` milliseconds.
Delegates to emit.schedule_render(). 16 ms ≈ 60 fps. 32 ms ≈ 30 fps.

---

### `get_secret`

```python
async def get_secret(key: str) -> 'str | None'
```

Request a secret by key. Alias for emit.get_secret(). Use with ``await``.

---

### `load_state`

```python
def load_state() -> dict
```

Return persisted app state loaded at startup. {} if nothing saved.

---

### `save_state`

```python
def save_state(state: dict) -> None
```

Persist state to workspace (if workspace active) or global storage.
Fire-and-forget — no acknowledgement from host.

---

### `http_request`

```python
async def http_request(url: str, method: str = 'GET', headers: 'dict[str, str] | None' = None, body: 'str | None' = None) -> str
```

HTTP request brokered through the host. Requires net.http capability. Use with ``await``.

---

### `ai_query`

```python
async def ai_query(model_tier: str, system: str, messages: 'list[dict]', tools: 'list[dict] | None' = None) -> object
```

AI broker call through the host. Requires ai.query capability. Use with ``await``.

---

### `spawn_pane`

```python
def spawn_pane(type_id: str, layout: str = 'split_v', args: 'list[str] | None' = None, pipe_id: 'str | None' = None, from_pane_id: 'int | None' = None, request_id: 'str | None' = None, target_context: 'int | None' = None) -> None
```

Request the host to open a new pane. Requires panes.spawn capability.

---

### `query_context_state`

```python
def query_context_state(context_id: int) -> None
```

Query the state of a context. Host responds via on_context_state.

---

### `badge_child`

```python
def badge_child(label: str, fill: str = '#89b4fa', fg: str = '#1e1e2e', font_size: float = 11.0, radius: float = 8.0) -> dict
```

Return a layout child node for a Badge. Use inside ctx.row/column/stack.

---

### `text_child`

```python
def text_child(text: str, size: float = 12.0, color: str = '#cdd6f4', monospace: bool = False, bold: bool = False, max_width: 'float | None' = None, elide: bool = False, selectable: bool = False) -> dict
```

Return a layout child node for Text. Use inside ctx.row/column/stack.

---

### `key_chip_child`

```python
def key_chip_child(label: str, font_size: float = 11.0) -> dict
```

Return a layout child node for a KeyChip. Use inside ctx.row/column/stack.

---

### `layout_node`

```python
def layout_node(direction: str, children: 'list[dict]', gap: float = 0.0) -> dict
```

Return a nested layout node for use inside ctx.row/column/stack.

---

### `row`

```python
def row(x: float, y: float, children: 'list[dict]', gap: float = 6.0) -> None
```

Emit a declarative flex-row layout tree.

The host resolves all child positions using taffy (flexbox) and real
egui font metrics. Children are layout child dicts from badge_child(),
text_child(), key_chip_child(), or layout_node().

`x`, `y`    — pane-relative top-left anchor.
`children`  — list of layout child dicts.
`gap`       — space between children in pixels (default 6.0).

Example:

    ctx.row(
        x=24.0, y=40.0,
        children=[
            ctx.badge_child("4 files"),
            ctx.text_child(" modified"),
        ],
        gap=6.0,
    )

---

### `column`

```python
def column(x: float, y: float, children: 'list[dict]', gap: float = 6.0) -> None
```

Emit a declarative flex-column layout tree.

Same as row() but stacks children top-to-bottom.

---

### `stack`

```python
def stack(x: float, y: float, children: 'list[dict]') -> None
```

Emit a declarative stack layout — all children rendered at the same origin.

Use for layering (background rect behind text, etc.).

---

### `responsive`

```python
def responsive(x: float, y: float, tiers: 'list[dict]') -> None
```

Emit a responsive layout that adapts to the available pane aspect ratio.

The host evaluates tiers in order and renders the first match:
- "landscape": pane is wider than tall (ratio > 1.2)
- "portrait": pane is taller than wide (ratio < 0.83)
- "square": fallback for balanced aspect ratios

Each tier dict has: aspect (str), direction (str), children (list), gap (float).

Example:

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

---

### `frame_done`

```python
def frame_done() -> None
```

Flush all buffered draw commands for this frame.

Called automatically after on_render returns. Only call this manually
if you're rendering in multiple phases and need intermediate updates.

---

