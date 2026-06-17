# Plexi SDK v2 — Developer Reference

SDK v2 is the Python authoring API. PGAP v3 (`pgap/3`) is the host/app wire
protocol the SDK speaks. They are different version lines: SDK v2 apps normally
emit PGAP v3 component trees.

## The Pattern (memorize this)

```python
from plexi_sdk import App
from plexi_sdk.ui import Column, AppBar, Label, Spacer, FooterKeys

class CounterApp(App):
    def on_init(self) -> None:
        self.count = self.state.get("count", 0)

    def view(self):
        return Column([
            AppBar("Counter"),
            Spacer(grow=True),
            Label(str(self.count), bold=True),
            Spacer(grow=True),
            FooterKeys([("+", "increment"), ("-", "decrement")]),
        ])

    def on_key(self, key: str, mods: dict) -> None:
        if key == "plus":    self.count += 1
        elif key == "minus": self.count -= 1
        self.state.save({"count": self.count})
        self.emit.schedule_render()

CounterApp().run()
```

## Lifecycle hooks

| Hook | Signature | When called | Notes |
|------|-----------|-------------|-------|
| `on_init` | `def on_init(self)` | Once after handshake | No ctx. Use `self.state.get()` for state. |
| `view` | `def view(self)` | Every Render event | Must return a Component. No I/O. |
| `on_key` | `def on_key(self, key, mods)` | Keypress | `key`: "plus", "minus", "return", "escape", "up", "down", "left", "right", "space", "a"-"z", "f1"-"f12". `mods`: `{"shift", "ctrl", "alt", "meta"}` booleans. |
| `on_click` | `def on_click(self, x, y, button)` | Pointer event | `button`: "primary", "secondary", "middle". |
| `on_path_changed` | `def on_path_changed(self, cwd)` | Workspace dir changed | `cwd`: absolute path string. |
| `on_escape` | `def on_escape(self) -> bool` | Escape key | Return `True` if handled (e.g. dismissed a modal). Return `False` to let host close the app. |
| `on_shutdown` | `def on_shutdown(self)` | Host closing app | Cleanup only. No emit after this. |

All hooks may be `async def`. Async hooks can `await self.emit.*` helpers.

## State

```python
self.state.get("key", default)  # read one value
self.state.all()                 # get full dict
self.state.save({"key": value}) # write and persist full dict
```

State is persisted by the host across app restarts. Call `self.emit.schedule_render()` after mutations.

## Reactive fields (optional convenience)

```python
from plexi_sdk import App, State

class MyApp(App):
    count = State(0)  # auto-calls schedule_render() on assignment
```

`State` descriptors are complementary to `self.state` (host persistence). Use `State` for fields that trigger re-render on change; use `self.state.save()` to persist across restarts.

## I/O (self.emit)

```python
self.emit.notify(title, priority, body="")       # fire notification
self.emit.schedule_render(after_ms=16)           # request next frame
self.emit.info/warn/error(msg)                   # structured logging
await self.emit.http_get(url)                    # HTTP GET (requires net.http capability)
await self.emit.ai_query(tier, system, messages) # LLM call (requires ai.query capability)
await self.emit.secret_get("API_KEY")            # read from secrets store
```

## Pane dimensions

`self.w`, `self.h` — current pane width/height in logical pixels. Updated each frame. Use in `view()` for responsive layouts.

## App with async I/O

```python
class WikiApp(App):
    def on_init(self) -> None:
        self.result = ""

    def view(self):
        return Column([
            AppBar("Wikipedia"),
            Label(self.result or "Press s to search"),
        ])

    async def on_key(self, key, mods):
        if key == "s":
            self.result = "Searching..."
            self.emit.schedule_render()
            self.result = await self.emit.http_get("https://en.wikipedia.org/...")
            self.emit.schedule_render()
```

## Canvas drawing reference (games, animation, data viz)

Override `on_render(self, ctx)` instead of `view()` for pixel-level control.
**Never override both `view()` and `on_render()`.**

### Primitives

**`ctx.rect(x, y, w, h, fill, *, radius=0.0, stroke=None, stroke_width=1.0, glow_color=None, glow_radius=0.0, gradient=None)`**
Fill a rectangle. `fill` is a hex string; supports 8-digit `#rrggbbaa` alpha.
- `stroke` — outline color hex; `stroke_width` — outline pixels (default 1.0).
- `glow_color` / `glow_radius` — soft halo painted behind the fill.
- `gradient` — dict `{"from": "#hex", "to": "#hex", "dir": "h"}` replaces the solid fill.
  `dir` is `"h"` (left→right) or `"v"` (top→bottom). Corner radius is not applied to the mesh.

```python
ctx.rect(10, 10, 200, 80, "#313244", radius=8.0,
         stroke="#89b4fa", stroke_width=1.5,
         glow_color="#89b4fa80", glow_radius=12.0)

ctx.rect(10, 100, 200, 60, "#000000",
         gradient={"from": "#1e1e2e", "to": "#313244", "dir": "h"})
```

**`ctx.circle(cx, cy, r, fill, *, stroke=None, stroke_width=1.0, glow_color=None, glow_radius=0.0)`**
Fill a circle. Same color rules as `rect`.
- `stroke` — outline color hex; `stroke_width` — outline pixels.
- `glow_color` / `glow_radius` — concentric-ring halo with linear alpha falloff.

```python
ctx.circle(cx, cy, 32, "#a6e3a1", stroke="#ffffff", stroke_width=2.0,
           glow_color="#a6e3a140", glow_radius=16.0)
```

**`ctx.arc(cx, cy, r, start_angle, end_angle, fill)`**
Filled pie slice. Angles in radians, clockwise from east. Full circle: `0` to `6.2832`.

**`ctx.arc_ring(cx, cy, r, start_angle, end_angle, color, stroke_width=2.0)`**
Stroked ring/donut arc — hollow, not filled. Same angle convention as `arc`.

```python
import math
ctx.arc_ring(cx, cy, 40, 0, math.pi * 1.5, "#89b4fa", stroke_width=4.0)
```

**`ctx.line(x1, y1, x2, y2, color, width=1.0)`**
Line segment.

**`ctx.text(x, y, text, size, color, *, monospace=False, bold=False, align="left_top", max_width=None, elide=True, selectable=False, max_lines=None)`**
Draw text. `align` values: `"left_top"`, `"center_center"`, `"right_bottom"`, etc. (9 anchors).

### Theme tokens

Access via `ctx.theme.<role>`:

| Token | Role |
|---|---|
| `bg` | Darkest background (terminal / pane chrome) |
| `bg_darkest` | Even darker background |
| `surface` | Card / surface background |
| `highlight` | Subtle highlight fill |
| `border` | Border / separator color |
| `fg` | Primary text |
| `muted` | Muted / secondary text |
| `text_section` | Section header text |
| `accent` | Brand blue |
| `danger` / `red` | Error / destructive |
| `success` / `green` | Success |
| `warning` / `yellow` | Warning |

### Color helpers

`dim(hex, alpha)` — module-level helper (`from plexi_sdk import dim`); returns `hex` with the given alpha (0–255) injected as `#rrggbbaa`. Example: `dim(ctx.theme.accent, 120)`.

### Example

```python
import math
from plexi_sdk import App

class GaugeApp(App):
    def on_render(self, ctx):
        cx, cy, r = ctx.w / 2, ctx.h / 2, min(ctx.w, ctx.h) * 0.35
        # Background ring
        ctx.arc_ring(cx, cy, r, 0, math.tau, ctx.theme.border, stroke_width=8.0)
        # Filled arc proportional to value
        ctx.arc_ring(cx, cy, r, -math.pi / 2,
                     -math.pi / 2 + math.tau * self.value,
                     ctx.theme.accent, stroke_width=8.0)
        # Glowing centre dot
        ctx.circle(cx, cy, 6, ctx.theme.accent,
                   glow_color=ctx.theme.accent + "60", glow_radius=10.0)
        ctx.text(cx, cy + r * 0.5, f"{self.value:.0%}",
                 size=20.0, color=ctx.theme.fg, align="center_center")
```

## Layout components (plexi_sdk.ui)

| Component | Usage |
|-----------|-------|
| `Column([...])` | Vertical stack. The root container for all apps. |
| `AppBar(title, subtitle=None)` | Top bar. Host shows Esc-to-close in the chrome. |
| `Spacer(grow=True)` | Fills remaining space. Put exactly one before FooterKeys. |
| `Label(text, tone="body", bold=False)` | Text. `tone`: "body", "caption", "hint". |
| `Section(title)` | Section header with divider line. |
| `FooterKeys(shortcuts)` | Bottom key hint row. `shortcuts`: list of `(key, description)` tuples. |
| `SelectList(items, selected_idx=0)` | Keyboard-navigable list. |
| `TextInput(id, placeholder="")` | Single-line text input. |
| `TextEdit(node_id, placeholder="", value="", multiline=False, max_length=0, height=48.0)` | Host-rendered text editor. Change/submit events via `on_component_event`. |
| `Divider()` | Horizontal rule. |

## Agent dev loop (testing and automation)

Agents drive Plexi apps the same way Playwright drives browsers:

```bash
# Render app with a given state — get UiNode tree as JSON
plexi app render apps/counter/counter.py --state '{"count": 5}'

# Read live app state from a running pane
plexi pane state <pane_id>

# Send a key event to a live pane
plexi pane key <pane_id> plus
plexi pane key <pane_id> minus
```

Use this loop for testing: render -> observe tree -> send key -> render again -> assert change.
