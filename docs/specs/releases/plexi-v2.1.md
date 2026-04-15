# Plexi Protocol v2.1 — UI Primitives

**Status:** Draft
**Last updated:** 2026-04-14
**Owner:** plexi-core
**Target:** Ships after Plexi 2.0 (v2.0 ship order in `protocol-v2.md`)
**Depends on:** `protocol-v2.md` §10 (version negotiation), `sdk/python/plexi_sdk.py` components layer (shipped)

---

## TL;DR

Plexi 2.0 ships the orchestration primitives: OpenIntent, Runs, event bus, typed pipes, capabilities, Plexi IQ. **It ships zero new UI primitives.** The Python SDK components layer (`ctx.header`, `ctx.scrollable_list`, `ctx.scrollable_text`, etc.) covers list-shaped apps completely, but leaves a category of apps unreachable because they need rendering primitives that don't exist yet:

- **Viewers** — mermaid diagrams, images, PDFs, maps. Need zoom/pan/transform.
- **Editors** — text editor, code editor, forms. Need cursor, selection, IME.
- **Dashboards** — calculators, grids, spreadsheets. Need grid layout and cell focus.
- **Advanced UI** — tabbed interfaces, forms with validation, modal dialogs.

v2.1 is **additive**, not breaking. It ships:

1. **Two new draw commands** — `PushTransform` / `PopTransform` (affine 2D transform stack) and an exact `MeasureText` round-trip (replaces the SDK's approximation).
2. **A components expansion** — `ctx.viewport()`, `ctx.text_input()`, `ctx.tabs()`, `ctx.grid()`, `ctx.modal()`, and `ctx.measure_text_exact()`.
3. **A manifest feature-negotiation surface** — apps declare `[app.protocol.requires = ["ui_primitives_v1"]]` so the host can refuse installs/launches cleanly when features are missing.

Protocol version stays at `2` — new commands are JSON-forward-compatible and apps negotiate by declared capabilities, not by version number. The proof-of-concept app in §8 is a rewritten `mermaid-viewer` that uses every primitive end-to-end.

---

## 1. Scope and non-goals

### In scope for v2.1

- `DrawCommand::PushTransform` / `PopTransform` — 2D affine stack (translate, scale, rotate)
- `DrawCommand::MeasureText` / `PlexiEvent::TextMetrics` — exact measurement round-trip
- `ctx.viewport(viewport_id, content_fn, zoom, pan, ...)` — pan/zoom container
- `ctx.text_input(input_id, value, on_change, ...)` — single-line text field with cursor
- `ctx.tabs(tab_id, tabs, selected, on_change)` — top-bar tab switcher
- `ctx.grid(grid_id, cols, rows, render_cell)` — static grid layout
- `ctx.modal(modal_id, visible, content_fn, ...)` — centered modal overlay with backdrop
- `ctx.measure_text_exact(text, size, monospace)` — exact measurement using `MeasureText` round-trip (SDK caches results per frame)
- `[app.protocol]` manifest section for feature negotiation

### Explicitly deferred to v2.2+

- **Proper `plexi-sdk` Python package on PyPI.** Today the SDK is one file at `sdk/python/plexi_sdk.py`, symlinked into each example dir for dev-tree cleanliness and dereferenced at install time by `just install-alpha`. Plexi 2.0 will add a `PYTHONPATH` resolution path in `ProcessApp::launch()` so installed apps share one SDK copy at `~/.plexi-alpha/sdk/plexi_sdk.py` instead of bundling (see v2 tracking issue). Post-2.0, when third-party app authors start shipping apps and want semver + pinned versions, the canonical file becomes a real packaged module: `pip install plexi-sdk`, manifest declares `plexi_sdk>=0.4.0`, version mismatches fail cleanly on install. Not valuable today — no external contributors yet — but the three-phase progression (vendored → shared → packaged) is the long-term path.
- **Rich text runs** — multiple fonts/colors in one text block (needed for syntax highlighting). Can be composed from multiple `Text` commands today at minor cost.
- **Clip regions** — clipping to a rect is useful for scrollable subregions. Parked until a concrete app needs it.
- **SVG primitives** — path, curve, polygon. Parked. Apps that need vector output rasterize to an image.
- **Canvas drawing / per-pixel access** — deliberately out of scope. Apps must compose from the primitive set.
- **Rotation around arbitrary points** — `PushTransform` supports rotation, but UX around pivot points is tricky. Rotation ships as advanced feature with sensible defaults.
- **IME / composing text** — text input in v2.1 is ASCII-first with `Backspace`, arrow keys, `Home`/`End`. Full IME composition is v2.2+.

### Non-goals — will never be in the core protocol

- Direct GPU / shader access. The protocol stays portable (WASM endgame).
- Multi-window apps. One pane, one surface.
- Native OS widgets (NSButton, etc.). Everything renders through draw commands.
- Per-frame animation primitives (spring/ease/interpolation). Apps do their own timing using `delta_time` from the Render event.

---

## 2. Why these primitives, in this order

The criteria for adding a primitive in v2.1:

1. **A concrete Tier 3 app needs it.** Every addition below unlocks ≥1 specific app that can't be written with the current components layer.
2. **It's the smallest change that covers its category.** `PushTransform` unlocks every viewer (mermaid, image, PDF, map). `text_input` unlocks every form. `grid` unlocks dashboards and calculators.
3. **It composes with v2.0 primitives.** No new capability system, no new lifecycle rules. Transform commands live in the same draw stream as `Rect`/`Text`/`Image`.

The five apps that unblock:

| App | Primitive needed | Component |
|---|---|---|
| `mermaid-viewer` | 2D transform + exact measure | `ctx.viewport` |
| `text-editor` | Cursor / selection rendering | `ctx.text_input` (single-line) + future multiline |
| `calc` | Fixed grid layout | `ctx.grid` |
| `markdown-preview` | Exact text measurement for wrap | `ctx.measure_text_exact` |
| `weather` (tabbed dashboards) | Tab switcher | `ctx.tabs` |

Everything else Tier 3 falls out of these five.

---

## 3. New draw commands

### 3.1 `PushTransform` / `PopTransform`

```rust
pub enum DrawCommand {
    // ... existing variants ...

    /// Push a 2D affine transform onto the stack. All subsequent draw commands
    /// (until the matching PopTransform) are rendered in the transformed
    /// coordinate space. Transforms compose: a scale inside a translate first
    /// translates, then scales around the translated origin.
    ///
    /// `origin` controls where rotation/scale are anchored (in the current
    /// pre-transform coordinate space). Defaults to (0, 0).
    PushTransform {
        #[serde(default = "one")]
        scale_x: f32,
        #[serde(default = "one")]
        scale_y: f32,
        #[serde(default)]
        translate_x: f32,
        #[serde(default)]
        translate_y: f32,
        #[serde(default)]
        rotate: f32,            // radians
        #[serde(default)]
        origin_x: f32,
        #[serde(default)]
        origin_y: f32,
    },
    PopTransform,
}
```

**Semantics:**

- Transforms are a stack. `PushTransform` × N must be matched with `PopTransform` × N in the same frame. Mismatch is a rendering error logged at `warn` level, frame discarded.
- The host maintains the stack per-frame and resets to identity at `FrameDone`.
- Rendering commands (`Rect`, `Text`, `Line`, `Image`, etc.) are transformed by the current stack product.
- Hit-testing for mouse events is also transformed: `PlexiEvent::Click { x, y }` is delivered in **pre-transform** coordinates (the app's logical space) so app code can use the same math it used to draw. The host maintains the stack during event delivery.

**Rationale for push/pop over nested:** matches every graphics API conventional (canvas, egui, wgpu). Flat JSON stream stays flat. No recursive types.

**Implementation:** `src/process_app.rs` adds a `Vec<egui::emath::TSTransform>` on `ProcessApp`, mul-composed on push, popped on pop. Each draw call applies `painter().with_clip_rect(...)`... actually egui uses `TSTransform` which has `*` operator for composition. The painter doesn't natively take transforms — it takes transformed positions. So: the host maintains `current_transform: TSTransform`, and every draw call does `current_transform.mul_pos(pos)` before painting.

### 3.2 `MeasureText` / `TextMetrics`

```rust
pub enum DrawCommand {
    // ...
    /// Ask the host to measure `text` at `size` and reply with exact metrics.
    /// The reply arrives as PlexiEvent::TextMetrics with the matching request_id.
    /// Apps should cache the result per-frame — the host charges one measurement
    /// per call, but the Python SDK wrapper caches within a single render pass.
    MeasureText {
        request_id: u32,
        text: String,
        size: f32,
        #[serde(default)]
        monospace: bool,
        #[serde(default)]
        bold: bool,
    },
}

pub enum PlexiEvent {
    // ...
    TextMetrics {
        request_id: u32,
        width: f32,
        height: f32,    // cap_height including descent
        ascent: f32,
    },
}
```

**Semantics:**

- `MeasureText` is a non-rendering command. It doesn't produce pixels; it produces a reply event on the stdin channel back to the app.
- The app must wait for the matching reply before continuing its current render pass if it needs the exact measurement to lay out. In practice, the SDK wraps this in a synchronous `ctx.measure_text_exact()` that blocks until the reply arrives (typically < 1ms since it's an in-process call through egui's `fonts()`).
- `request_id` is app-local; the host echoes it verbatim.
- **Replaces** the current approximation-only `ctx.measure_text()`. The approximation remains as `ctx.measure_text_approx()` for apps that need a non-blocking estimate.

**Implementation cost:** ~40 lines in `process_app.rs`. The host calls `ctx.fonts(|f| f.layout_no_wrap(text, font_id, color)).rect.size()`. That's the egui text layout engine — exact.

---

## 4. New SDK components

All additions to `sdk/python/plexi_sdk.py`. Still one file, still stdlib-only, still vendored.

### 4.1 `ctx.viewport(viewport_id, content_fn, zoom=None, pan=None, min_zoom=0.1, max_zoom=10.0, on_pan=None, on_zoom=None, x=None, y=None, w=None, h=None)`

A pan/zoom container. `content_fn(ctx)` is called with a transformed context — rect/text/image commands drawn inside are transformed by the current zoom/pan.

```python
def render(ctx):
    ctx.header("Mermaid")
    ctx.viewport(
        viewport_id="diagram",
        content_fn=lambda c: c.image(0, 0, image_w, image_h, path="/tmp/diagram.png"),
        zoom=diagram_zoom,
        pan=diagram_pan,
        x=0, y=HEADER_H, w=ctx.width, h=ctx.height - HEADER_H - 30,
    )
    ctx.status_bar([...])
```

- Zoom/pan state is owned by the app (passed in each frame).
- SDK emits `PushTransform` with `scale_x=zoom, scale_y=zoom, translate_x=pan.x, translate_y=pan.y`, runs `content_fn(ctx)`, emits `PopTransform`.
- Mouse wheel → pinch-zoom, mouse drag → pan, `+`/`-` → zoom, arrow keys → pan. Plexi host handles these when the viewport has focus (via `ctx.viewport_focus(viewport_id)` — see "viewport input handling").
- `on_pan` / `on_zoom` callbacks let the app react to state changes. The SDK writes back to the app's state directly if the app passes mutable refs (using a small `Viewport` dataclass).

### 4.2 `ctx.text_input(input_id, value, on_change, cursor=None, placeholder=None, max_length=None, size=BODY, x=0, y=0, w=None, bg=None, fg=None, focused=True)`

A single-line editable text field. Uses `PushTransform` internally? No — single-line text doesn't need transforms. It uses direct draw calls.

- `value: str` — the app's current value (app owns state, passes in each frame)
- `cursor: int` — character index (defaults to end)
- `on_change: fn(new_value, new_cursor)` — called when the SDK's keyboard handling mutates the value
- The SDK handles: character insertion, `Backspace`, `Delete`, `Home`, `End`, `ArrowLeft`/`ArrowRight` (with optional `Shift` for selection, deferred to v2.2).
- Draws a rounded rect background, the text, a blinking cursor (using `delta_time` for animation), and a placeholder when empty.
- **Keyboard routing:** when an app has a focused `text_input`, it marks `ctx.input_focus(input_id)` during render. The SDK intercepts key events for that input before dispatching to the app's `@app.on_key` handler. Apps can also opt a text input out of focus via the `focused` parameter.

### 4.3 `ctx.tabs(tab_id, tabs, selected, on_change=None, height=36, x=0, y=HEADER_H, w=None)`

A horizontal tab switcher directly under the header.

- `tabs: list[tuple[str, str]]` — `[(key, label), ...]`. `key` is app-local, `label` is displayed.
- `selected: str` — currently-selected key.
- `on_change: fn(new_key)` — called when the user clicks a tab or presses Tab/Shift+Tab.
- Draws each tab's label centered in its slot, the selected tab gets a bottom underline in `accent` color, hover states are live via `MouseMove`.

### 4.4 `ctx.grid(grid_id, cols, rows, render_cell, x=None, y=None, w=None, h=None, gap=PAD_TIGHT)`

A fixed grid. No virtualization. Good for calculators, color pickers, keyboard layouts.

- `render_cell: fn(ctx, col, row, x, y, cell_w, cell_h)` — app draws into the provided cell rect.
- Grid sizes cells uniformly: `cell_w = (w - (cols-1)*gap) / cols`. Same for rows.
- No built-in selection; apps track their own "focused cell" and highlight in `render_cell`.

### 4.5 `ctx.modal(modal_id, visible, content_fn, width=400, height=200, backdrop_alpha=128)`

A centered modal overlay with a dimmed backdrop. For confirmations, name prompts, inline errors.

- `visible: bool` — show or hide.
- `content_fn(ctx, x, y, w, h)` — app draws the modal body into the provided rect (relative to pane).
- SDK draws the backdrop rect covering the whole pane, then the modal background with rounded corners and drop shadow (shadow is two offset rects).
- Keyboard: Esc dismisses (on_dismiss callback), Tab/Shift+Tab cycle focused inputs inside the modal. Cmd+W still closes the whole app (host-level).

### 4.6 `ctx.measure_text_exact(text, size, monospace=False, bold=False) -> TextMetrics`

Blocking round-trip via `DrawCommand::MeasureText`. Returns `(width, height, ascent)`. Cached per-frame in `RenderContext` keyed by `(text, size, monospace, bold)`.

For apps that need exact layout math (markdown preview word-wrap, text input cursor positioning). Non-blocking approximation stays as `ctx.measure_text(...)`.

---

## 5. Manifest feature negotiation

New optional section:

```toml
[app.protocol]
requires = ["ui_primitives_v1"]
```

- `requires: list[str]` — feature flags the host must support. Unknown flags fail app load with a clear error.
- v2.1 hosts declare `ui_primitives_v1` in their capability set.
- v2.0 hosts don't, so an app with `requires = ["ui_primitives_v1"]` refuses to load on them with `error: host does not support ui_primitives_v1 (required by app X).`
- Apps that don't use v2.1 commands leave this section empty and stay v2.0-compatible.

Feature flags are additive. Future flags: `rich_text_runs`, `clipping`, `svg_primitives`.

---

## 6. Ship order (after v2.0)

v2.1 is ~3 weeks of work once v2.0 is done.

**Week 1 — Protocol additions**
1. `PushTransform` / `PopTransform` in `app_protocol.rs`, transform stack in `process_app.rs`, hit-test transform in event delivery. Tests: draw a rect inside a `scale(2,2)` transform, confirm it renders at 2× size and a click in that area reports pre-transform coordinates.
2. `MeasureText` / `TextMetrics` round-trip. Tests: SDK `measure_text_exact("Hello", 16.0)` returns the same width egui would give internally.

**Week 2 — SDK components**
3. `ctx.viewport` (depends on `PushTransform`). Mouse-wheel zoom, click-drag pan, keyboard zoom/pan when focused.
4. `ctx.text_input` (depends on `measure_text_exact` for cursor positioning and `delta_time` for blink).
5. `ctx.tabs`, `ctx.grid`, `ctx.modal` — pure layout components, no new protocol dependency.

**Week 3 — Reference apps + polish**
6. Rewrite `mermaid-viewer` using `ctx.viewport` + exact measure (see §7).
7. Rewrite `calc` using `ctx.grid`.
8. Rewrite `text-editor` for single-line prompts using `ctx.text_input`. Multi-line text editor stays deferred to v2.2.
9. Port `weather` to `ctx.tabs` for its multi-city view.
10. Manifest feature negotiation end-to-end test: install an app with `requires = ["ui_primitives_v1"]` on a v2.0 host, confirm clean refusal.

---

## 7. Feature-negotiation details

The `requires` check runs at:

1. **Install time** (`plexi install`) — warn + offer to upgrade the host if a required feature isn't present; don't block install (user may upgrade later).
2. **Launch time** (`AppRegistry::launch` in `src/app_registry.rs`) — refuse to launch with a clear error message. User sees: `Cannot launch mermaid-viewer: host does not support ui_primitives_v1. Update Plexi to v2.1+ to run this app.`

The host publishes its feature set via `HOST_FEATURES` const in `src/app_registry.rs`:

```rust
pub const HOST_FEATURES: &[&str] = &[
    "core_v1",           // v1 protocol baseline
    "open_intent_v1",    // v2.0 §3
    "event_bus_v1",      // v2.0 §4
    "runs_v1",           // v2.0 §5
    "typed_pipes_v1",    // typed-pipes.md Phase 1
    "ui_primitives_v1",  // v2.1 (this doc)
];
```

Features are strings, not flags. New features append. Dropped features (rare) become deprecated but remain in the list for one major version.

---

## 8. Worked example — `mermaid-viewer` with v2.1 primitives

The current `mermaid-viewer` (shipped in #206) renders a mermaid diagram to a PNG and displays it full-pane with no zoom/pan. Users can view a diagram but can't inspect it — a complex flowchart is unreadable at pane resolution.

The v2.1 rewrite is the canonical proof-of-concept app for the new primitive set: every component below exercises something only v2.1 makes possible.

### 8.1 Manifest

```toml
[app]
id = "mermaid-viewer"
name = "Mermaid Viewer"
entry = "mermaid_viewer.py"
version = "0.2.0"
description = "Zoomable mermaid diagram viewer"
protocol_version = 2

[app.launch]
mode = "fullscreen"
companion = "none"

[app.protocol]
requires = ["ui_primitives_v1"]

[app.capabilities]
file_types = ["mmd"]
filesystem = "read_only"

[app.open_intent]
# v2.0 OpenIntent — caller passes the .mmd file path to open.
accepts = ["file"]
```

### 8.2 Python app (~100 lines)

```python
from __future__ import annotations
"""
mermaid_viewer.py — Zoomable mermaid diagram viewer.

A v2.1 reference app. Exercises:
  ctx.header / ctx.status_bar — components layer
  ctx.viewport              — v2.1 pan/zoom primitive
  ctx.measure_text_exact    — v2.1 exact measurement
  ctx.modal                 — v2.1 modal overlay
  OpenIntent                — v2.0 launch intent
"""

import pathlib
import subprocess
from plexi_sdk import (
    App, load_manifest, THEME,
    TITLE, BODY, CAPTION, HINT, PAD, HEADER_H,
)

app = App("mermaid-viewer")

# State
file_path: pathlib.Path | None = None
image_path: pathlib.Path | None = None
image_size: tuple[int, int] = (0, 0)
zoom: float = 1.0
pan: tuple[float, float] = (0.0, 0.0)
error: str | None = None
show_help: bool = False


@app.on_init
def init(ev):
    global file_path, image_path, image_size, error
    intent = ev.open_intent
    if intent and intent.kind == "file":
        file_path = pathlib.Path(intent.payload["path"])
        image_path = _render_mermaid(file_path)
        if image_path and image_path.exists():
            image_size = _probe_png_size(image_path)
        else:
            error = "Failed to render diagram"
    else:
        error = "No diagram file provided"


def _render_mermaid(src: pathlib.Path) -> pathlib.Path | None:
    out = pathlib.Path("/tmp") / f"{src.stem}.png"
    try:
        subprocess.run(
            ["mmdc", "-i", str(src), "-o", str(out), "-b", "transparent"],
            check=True, capture_output=True,
        )
        return out
    except Exception:
        return None


def _probe_png_size(p: pathlib.Path) -> tuple[int, int]:
    # Stdlib PNG dimension read (no Pillow dependency).
    with p.open("rb") as f:
        f.read(8)   # signature
        f.read(8)   # IHDR chunk length + type
        width = int.from_bytes(f.read(4), "big")
        height = int.from_bytes(f.read(4), "big")
    return (width, height)


def _fit_to_pane(pane_w: float, pane_h: float) -> float:
    iw, ih = image_size
    if iw == 0 or ih == 0:
        return 1.0
    return min(pane_w / iw, pane_h / ih, 1.0)


@app.on_render
def render(ctx):
    global zoom, pan

    ctx.rect(0, 0, ctx.width, ctx.height, fill=THEME.bg)

    title = f"Mermaid  ·  {file_path.name if file_path else '(no file)'}"
    ctx.header(title)

    content_y = HEADER_H + PAD
    content_h = ctx.height - content_y - 30 - PAD

    if error:
        ctx.empty_state("Render failed", error, icon_color=THEME.red)
    elif image_path:
        # Centre the image at zoom 1.0.
        iw, ih = image_size
        base_x = (ctx.width - iw) / 2
        base_y = content_y + (content_h - ih) / 2

        def draw_diagram(c):
            c.image(base_x + pan[0], base_y + pan[1], iw, ih, path=str(image_path))

        ctx.viewport(
            viewport_id="diagram",
            content_fn=draw_diagram,
            zoom=zoom,
            pan=(0.0, 0.0),   # pan is applied to the image rect, not the transform
            x=0, y=content_y, w=ctx.width, h=content_h,
            min_zoom=0.1, max_zoom=10.0,
        )

    zoom_pct = f"{int(zoom * 100)}%"
    ctx.status_bar(
        [
            ("+/-", "zoom"),
            ("arrows", "pan"),
            ("f", "fit"),
            ("0", "reset"),
            ("?", "help"),
            ("⌘W", "close"),
        ],
        status_msg=zoom_pct,
        status_color=THEME.accent,
    )

    if show_help:
        def help_body(c, mx, my, mw, mh):
            lines = [
                ("Mermaid Viewer", TITLE, THEME.accent),
                ("", BODY, THEME.fg),
                ("+/-   zoom in/out", BODY, THEME.fg),
                ("arrows   pan", BODY, THEME.fg),
                ("f   fit to pane", BODY, THEME.fg),
                ("0   reset zoom + pan", BODY, THEME.fg),
                ("?   toggle this help", BODY, THEME.muted),
                ("⌘W   close", BODY, THEME.muted),
            ]
            cy = my + PAD
            for text, size, color in lines:
                c.text(mx + PAD, cy, text, size=size, color=color)
                cy += size + 6

        ctx.modal("help", visible=True, content_fn=help_body, width=360, height=220)


@app.on_key
def on_key(key, mods, emit):
    global zoom, pan, show_help

    if show_help:
        if key in ("?", "Escape"):
            show_help = False
        return

    pan_step = 40.0 / zoom

    if key in ("+", "="):
        zoom = min(zoom * 1.25, 10.0)
    elif key == "-":
        zoom = max(zoom / 1.25, 0.1)
    elif key == "0":
        zoom = 1.0
        pan = (0.0, 0.0)
    elif key == "f":
        zoom = _fit_to_pane(float(app.width), float(app.height) - HEADER_H - 30 - PAD * 2)
        pan = (0.0, 0.0)
    elif key == "ArrowLeft":
        pan = (pan[0] + pan_step, pan[1])
    elif key == "ArrowRight":
        pan = (pan[0] - pan_step, pan[1])
    elif key == "ArrowUp":
        pan = (pan[0], pan[1] + pan_step)
    elif key == "ArrowDown":
        pan = (pan[0], pan[1] - pan_step)
    elif key == "?":
        show_help = True


@app.on_scroll
def on_scroll(dx, dy, mods, emit):
    # Mouse-wheel zoom. The host already provides dy in logical pixels.
    global zoom
    factor = 1.0 + (dy * 0.002)
    zoom = max(0.1, min(zoom * factor, 10.0))


app.run()
```

### 8.3 What this app demonstrates

| Feature | How | Couldn't do in v2.0 |
|---|---|---|
| **Header + status bar + empty state** | `ctx.header`, `ctx.status_bar`, `ctx.empty_state` | Could do in v2.0 (components layer) |
| **Zoom/pan container** | `ctx.viewport(zoom=, content_fn=...)` emits `PushTransform` / `PopTransform` around the nested `image` call | ✗ — no transform primitive |
| **Launch intent** | Receives the `.mmd` path via `open_intent.payload["path"]` from palette / file-browser / agent mode | ✗ in v1; v2.0 OpenIntent |
| **Modal help overlay** | `ctx.modal("help", visible=show_help, content_fn=...)` draws over the whole pane with backdrop | ✗ — no modal primitive |
| **Mouse wheel zoom** | `@app.on_scroll` handler + state mutation | Partially — protocol has Scroll event; v2.1 adds viewport-native integration |
| **Feature negotiation** | `[app.protocol] requires = ["ui_primitives_v1"]` — refuses to load on v2.0 host | ✗ — no negotiation surface |

Every primitive has a purpose; the app isn't a feature demo. It's a real viewer users would use, and the line count is roughly 100 — comparable to a well-written v2.0 app of similar complexity. The components layer + protocol additions let the app stay focused on its domain (mermaid rendering, zoom math) instead of re-implementing zoom/pan/scrollbar infrastructure.

---

## 9. What v2.1 does NOT solve

The following Tier 3 apps are still blocked after v2.1 ships:

| App | What's still missing |
|---|---|
| `text-editor` (multi-line) | Selection primitive, line-break handling, IME — all v2.2+ |
| `diff-viewer` | Rich text runs (add/remove highlighting mid-line) |
| `pyflow` | Node graph primitive (custom connectors) — likely app-level, not protocol |
| `map` / `globe` | Tile streaming, viewport culling — v3+ |
| `video-editor` | Frame-accurate seek, waveform rendering — v3+ |

These are filed in backlog with the specific primitive each one needs. Don't build them speculatively; build them when a real app wants to ship.

---

## 10. Open questions

1. **Does `ctx.viewport` own keyboard focus?** Proposal: yes — when the viewport has focus, the SDK intercepts `+`/`-`/arrows/wheel for zoom/pan and never dispatches to `@app.on_key`. Apps that want custom keybindings inside a viewport opt out via `on_key_passthrough=True`. Decide before shipping 4.1.
2. **Should `ctx.modal` dismiss on backdrop click?** Proposal: yes, unless `dismiss_on_backdrop=False`.
3. **Text input max value length: runtime or manifest?** Proposal: runtime (`max_length` parameter). Manifest constraints are for host enforcement; this is app UX.
4. **Grid component — virtualized rows?** Proposal: no. Grid is fixed-size for v2.1 (calculators, color pickers). Virtual grids are a separate primitive when a spreadsheet app needs one.
5. **`PushTransform` stack depth limit?** Proposal: 16. Deeper nesting is almost certainly a bug; 16 is enough for any real case.

---

## 11. Cross-references

- **`protocol-v2.md`** — v2.0 orchestration layer; this doc depends on version negotiation (§10) and OpenIntent (§3).
- **`proposals/core-advanced-ui-sdk.md`** — original draft of tabs/carousels/grid; this doc formalizes and scopes what's actually in v2.1.
- **`proposals/core-text-editor-primitive.md`** — original draft of text input; multi-line editor is still deferred but single-line ships here.
- **`proposals/core-layout-presets.md`** — layout presets; grid component here supersedes the static preset model for static grids.
- **`sdk/python/plexi_sdk.py`** — target file for all SDK additions.
- **`src/app_protocol.rs`** — target file for all draw command additions.
- **`src/process_app.rs`** — target for transform stack + measure_text round-trip.
- **`src/app_registry.rs`** — target for feature negotiation.

---

**End of spec.** Protocol v2.1 is additive, the SDK stays one file, the reference app works end-to-end. Changes require a `Last updated` bump and a DEV_LOG entry.
