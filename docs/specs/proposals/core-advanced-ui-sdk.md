# Advanced UI SDK

**Status:** Spec  
**Last updated:** 2026-04-11  
**Depends on:** Simple SDK (current `plexi_sdk.py`), text editor primitive  
**Blocks:** PyFlow, Snake, Aquarium, any canvas/interactive app

---

## Summary

A second-tier SDK for apps that need stateful, interactive UIs — node canvases, games, drag-and-drop interfaces, layered modals. Separate from the simple SDK, which stays dead-simple for list/text/rect apps.

The simple SDK is declarative: "here's what I want on screen." The Advanced SDK is stateful: "here's how I handle a click at (x,y) given my current mode."

Both SDKs speak the same JSON draw protocol. The Advanced SDK is a Python (and eventually Rust) library that provides higher-level abstractions over the raw commands — coordinate transforms, hit testing, scene graphs, animation timing — so apps don't reimplement these patterns.

---

## Why Two SDKs

The simple SDK (`plexi_sdk.py`) is 247 lines. An app like git-log or a file browser is ~200 lines of app code. That's the right ratio — the SDK disappears, the app is the code.

Canvas apps are structurally different:
- They need coordinate spaces (pan/zoom transforms applied to all draw calls)
- They need hit testing (which node did the user click?)
- They need drag state machines (mousedown → move → mouseup with snapping)
- They need focus routing (which widget gets keystrokes?)
- They need z-ordering (modals on top of canvas on top of background)
- They need animation (smooth transitions, particle systems, game loops)

Mixing these into the simple SDK would make every simple app author wade through abstractions they don't need. Keeping them separate means simple apps stay simple and complex apps get real tools.

---

## Architecture

```
┌──────────────────────────────────────┐
│  App Code (PyFlow, Snake, etc.)      │
├──────────────────────────────────────┤
│  Advanced SDK                        │
│  ┌──────────┐ ┌──────────┐ ┌──────┐ │
│  │  Canvas   │ │  Input   │ │ Anim │ │
│  │  (coords, │ │  (hit,   │ │ (dt, │ │
│  │   scene,  │ │   drag,  │ │ tween│ │
│  │   layers) │ │   focus) │ │ ease)│ │
│  └──────────┘ └──────────┘ └──────┘ │
├──────────────────────────────────────┤
│  Simple SDK (plexi_sdk.py)           │
│  App, RenderContext, Emitter         │
├──────────────────────────────────────┤
│  JSON draw protocol (stdin/stdout)   │
└──────────────────────────────────────┘
```

The Advanced SDK imports and extends the simple SDK. `RenderContext` gains new methods. `App` gains new event types. The wire protocol may need a few new draw commands (see New Draw Commands below).

---

## Module: Canvas

A transformed coordinate space for drawing. All draw calls within a canvas are offset and scaled by the canvas's current transform.

### API

```python
from plexi_sdk_advanced import Canvas

class MyApp:
    def __init__(self):
        self.canvas = Canvas()
        # canvas.offset = (0, 0)  — pan offset in screen pixels
        # canvas.scale = 1.0      — zoom level
        # canvas.bounds = None    — optional content bounds for clamping

    def on_render(self, ctx):
        # Background
        ctx.rect(0, 0, ctx.width, ctx.height, fill="#1e1e2e")

        # Enter canvas coordinate space
        with self.canvas.transform(ctx):
            # All coordinates here are in canvas space
            ctx.rect(100, 200, 150, 80, fill="#313244")  # a node at canvas (100, 200)
            ctx.text(110, 220, "my_func", size=14, color="#cdd6f4")

        # Back to screen space — draw fixed UI (toolbars, etc.)
        ctx.rect(0, 0, ctx.width, 40, fill="#181825")
```

### Pan & Zoom

The canvas handles pan/zoom input automatically when enabled:

- **Pan:** Middle-click drag, or two-finger trackpad scroll
- **Zoom:** Cmd+scroll, pinch (if Plexi forwards gesture events)
- **Zoom-to-fit:** Programmatic — `canvas.zoom_to_fit(content_bounds)`

The app can also set `canvas.offset` and `canvas.scale` directly for programmatic control.

### Coordinate Conversion

```python
# Screen pixel → canvas coordinate
canvas_pos = self.canvas.screen_to_canvas(screen_x, screen_y)

# Canvas coordinate → screen pixel
screen_pos = self.canvas.canvas_to_screen(canvas_x, canvas_y)
```

---

## Module: Input

Hit testing, drag state, and focus management.

### Hit Testing

```python
from plexi_sdk_advanced import HitRegion, HitTester

class MyApp:
    def __init__(self):
        self.hit = HitTester()
        self.nodes = [...]

    def on_render(self, ctx):
        self.hit.clear()
        for node in self.nodes:
            ctx.rect(node.x, node.y, node.w, node.h, fill=node.color)
            self.hit.register(node.id, node.x, node.y, node.w, node.h)

    def on_click(self, x, y, button, emit):
        canvas_pos = self.canvas.screen_to_canvas(x, y)
        hit = self.hit.test(canvas_pos.x, canvas_pos.y)
        if hit:
            self.selected_node = hit.id
```

### Drag State Machine

```python
from plexi_sdk_advanced import DragHandler

class MyApp:
    def __init__(self):
        self.drag = DragHandler(
            threshold=4,  # pixels before drag activates (prevents accidental drags)
        )

    def on_click(self, x, y, button, emit):
        hit = self.hit.test(...)
        if hit:
            self.drag.start(x, y, payload=hit.id)

    def on_mouse_move(self, x, y, emit):
        if self.drag.active:
            dx, dy = self.drag.delta(x, y)
            self.nodes[self.drag.payload].x += dx
            self.nodes[self.drag.payload].y += dy

    def on_mouse_up(self, x, y, button, emit):
        if self.drag.active:
            self.drag.end()
```

> **Protocol note:** This requires Plexi to send `mouse_move` and `mouse_up` events to apps. Currently only `click` is in the protocol. New events needed:
> - `{"type": "mouse_move", "x": ..., "y": ...}` — sent while mouse button is held (drag), or always if app opts in
> - `{"type": "mouse_up", "x": ..., "y": ..., "button": "primary"}` — sent on button release
> - `{"type": "mouse_down", "x": ..., "y": ..., "button": "primary"}` — replaces or supplements `click` for drag-aware apps

### Focus Manager

Routes keyboard input to the correct widget.

```python
from plexi_sdk_advanced import FocusManager

class MyApp:
    def __init__(self):
        self.focus = FocusManager()
        # focus.set("node_editor")
        # focus.current → "node_editor" | None

    def on_key(self, key, mods, emit):
        if self.focus.current == "search_bar":
            self.handle_search_key(key, mods)
        elif self.focus.current == "canvas":
            self.handle_canvas_key(key, mods)
        else:
            self.handle_global_key(key, mods)
```

---

## Module: Animation

Frame-rate-aware animation helpers. Plexi sends a `delta_time` field on render events (seconds since last frame).

> **Protocol note:** This requires a new field on the render event:
> `{"type": "render", "width": 800, "height": 600, "delta_time": 0.016}`

### Tweens

```python
from plexi_sdk_advanced import Tween, ease_out_cubic

class MyApp:
    def __init__(self):
        self.node_x = Tween(start=100, end=300, duration=0.3, easing=ease_out_cubic)

    def on_render(self, ctx):
        x = self.node_x.value(ctx.time)  # interpolated value at current time
        ctx.rect(x, 100, 150, 80, fill="#313244")
```

### Easing Functions

Standard set: `linear`, `ease_in`, `ease_out`, `ease_in_out`, `ease_out_cubic`, `ease_out_bounce`, `ease_out_elastic`.

### Frame Timer

For game loops that need consistent tick rates:

```python
from plexi_sdk_advanced import FrameTimer

class SnakeGame:
    def __init__(self):
        self.tick = FrameTimer(interval=0.15)  # snake moves every 150ms

    def on_render(self, ctx):
        if self.tick.ready(ctx.delta_time):
            self.move_snake()
        self.draw(ctx)
```

---

## Module: Layers (Z-ordering)

Draw commands are painter's model — later commands draw on top. The Layers module provides named layers to organize draw order without manual sorting.

```python
from plexi_sdk_advanced import LayerStack

class MyApp:
    def __init__(self):
        self.layers = LayerStack(["background", "canvas", "connections", "nodes", "modal"])

    def on_render(self, ctx):
        with self.layers.draw("background", ctx):
            ctx.rect(0, 0, ctx.width, ctx.height, fill="#1e1e2e")

        with self.layers.draw("nodes", ctx):
            for node in self.nodes:
                self.draw_node(ctx, node)

        with self.layers.draw("connections", ctx):
            for edge in self.edges:
                self.draw_edge(ctx, edge)

        if self.modal_open:
            with self.layers.draw("modal", ctx):
                # semi-transparent overlay
                ctx.rect(0, 0, ctx.width, ctx.height, fill="#00000088")
                self.draw_modal(ctx)

        self.layers.flush(ctx)  # flushes all layers in declared order
```

---

## New Draw Commands (Protocol Extensions)

The Advanced SDK needs a few new draw commands added to Plexi's renderer:

| Command | Fields | Description |
|---------|--------|-------------|
| `image` | `x, y, w, h, path` (or `data` as base64) | Render an image (PNG/SVG). For sprites, game assets, icons. |
| `circle` | `cx, cy, r, fill, stroke, stroke_width` | Circle primitive. Cheaper than approximating with rect+radius. |
| `arc` | `cx, cy, r, start_angle, end_angle, color, width` | Arc/curve for edge routing in node graphs. |
| `bezier` | `x1, y1, cx1, cy1, cx2, cy2, x2, y2, color, width` | Cubic bezier curve. Essential for node graph edges. |
| `clip` | `x, y, w, h` | Set a clipping rectangle — subsequent commands only draw within this rect. |
| `clip_end` | — | Remove the current clip. |
| `opacity` | `value` (0.0–1.0) | Set global opacity for subsequent commands until `opacity_end`. |
| `opacity_end` | — | Reset opacity to 1.0. |

These are all standard egui operations — `Painter::circle`, `CubicBezierShape`, `clip_rect`, etc. Exposing them as draw commands is straightforward.

### New Input Events

| Event | Fields | Description |
|-------|--------|-------------|
| `mouse_down` | `x, y, button` | Mouse button pressed. |
| `mouse_up` | `x, y, button` | Mouse button released. |
| `mouse_move` | `x, y` | Mouse moved (opt-in via manifest: `mouse_tracking = true`). |
| `scroll` | `x, y, dx, dy` | Scroll/trackpad event at position. |

The `render` event gains:
- `delta_time` (float, seconds since last frame)
- `time` (float, seconds since app start)

---

## SDK Distribution

### Python

```
sdk/
  python/
    plexi_sdk.py              # simple SDK (unchanged)
    plexi_sdk_advanced.py     # advanced SDK (imports plexi_sdk)
```

Apps copy `plexi_sdk_advanced.py` (which imports `plexi_sdk`) into their directory, same pattern as today. One file, zero dependencies.

### Rust

A `plexi-sdk-advanced` crate that depends on `plexi-sdk`. Provides typed canvas/input/animation abstractions over the same JSON protocol.

---

## MVP Scope

1. **Canvas** — transform context, pan/zoom, coordinate conversion. This unblocks PyFlow.
2. **Hit testing** — register regions, test point. This unblocks click interaction on nodes.
3. **New draw commands** — `bezier` (for node edges), `circle` (for ports), `clip` (for scroll containers). Requires Plexi renderer changes.
4. **New input events** — `mouse_down`, `mouse_up`, `mouse_move`. Requires Plexi event forwarding changes.
5. **delta_time on render** — unblocks animation and game loops.

**Defer:** Tweens, easing, LayerStack (apps can manage draw order manually for now), image/sprite rendering, Rust SDK.

---

## Relationship to Simple SDK

The simple SDK is not deprecated. It remains the right choice for 80% of apps. The advanced SDK:

- Imports and extends the simple SDK (not a fork)
- Adds no new draw commands that simple apps need to know about
- Can be ignored entirely by apps that don't need canvas/drag/animation

An app can start with the simple SDK and upgrade to advanced when it needs to. The migration is: `from plexi_sdk_advanced import App` instead of `from plexi_sdk import App` — everything else stays the same.
