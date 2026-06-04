# SDK Quickstart: Your First Plexi App

**Audience:** a coding agent building its first PGAP app.
**Goal:** running counter app in 50 lines of Python.
**Deeper reference:** [`docs/PGAP_REFERENCE.md`](PGAP_REFERENCE.md)

---

## 1. Prerequisites

```
plexi-alpha app list   # confirms Plexi Alpha is running
```

The SDK is pure stdlib — no pip install needed. It ships alongside the host
at `sdk/python/plexi_sdk/`.

---

## 2. Scaffold the app

```bash
cd ~/my-apps
plexi app init counter
cd counter
```

`plexi app init` creates:
```
counter/
  manifest.toml   # app identity and launch config
  counter.py      # entry point (empty shell)
```

**Never hand-write `manifest.toml`.** `plexi app init` produces the correct
`schema_version` and required fields. Edit only what you need to change.

---

## 3. manifest.toml

The generated manifest looks like this:

```toml
schema_version = 1

[app]
id = "counter"
type = "app"
name = "Counter"
version = "0.1.0"
description = "A simple counter demo."
entry = "counter.py"

[app.capabilities]
capabilities = []

[launch]
```

Fields:
- `id` — stable slug; used for the log target (`app::counter`), install dir, pack refs.
- `entry` — path to the Python entry point, relative to the manifest.
- `capabilities` — list capabilities you need (e.g. `["net.http"]`). Leave empty if none.
- `[launch]` — optional. Add `notification_scope = "global"` for always-visible notifications.

---

## 4. Write the app (counter.py)

```python
#!/usr/bin/env python3
import os, sys
sys.path.insert(0, os.path.join(os.path.dirname(__file__), '../../sdk/python'))

from plexi_sdk import App, RenderContext, BODY, CAPTION, PAD

class CounterApp(App):
    def on_init(self, ctx: RenderContext) -> None:
        self.count = 0
        ctx.info("CounterApp ready")

    def on_render(self, ctx: RenderContext) -> None:
        ctx.clear(ctx.theme.bg)
        # Card background
        ctx.rect(PAD, PAD, ctx.w - PAD * 2, 80, ctx.theme.surface, radius=8.0)
        # Count value
        ctx.text(PAD * 2, PAD + 12, f"Count: {self.count}", size=BODY, color=ctx.theme.fg)
        # Hint row
        ctx.text(PAD * 2, PAD + 44, "+ / -  change   q  quit",
                 size=CAPTION, color=ctx.theme.muted)

    def on_key(self, ctx: RenderContext, key: str, mods: dict) -> None:
        if key == "+" or (key == "=" and mods.get("shift")):
            self.count += 1
        elif key == "-":
            self.count -= 1
        # q is handled by the host — apps cannot self-exit

CounterApp().run()
```

That's 37 lines including imports.

---

## 5. Open the app in Plexi

```bash
plexi app open ~/my-apps/counter
```

The app opens in a new pane. Plexi calls `on_render` whenever the pane needs
repainting and `on_key` for every keypress.

---

## 6. Key concepts

### on_render

Called on every frame. Must be fast and free of I/O. Reads state written by
other handlers; never fetches data inline.

```python
def on_render(self, ctx: RenderContext) -> None:
    ctx.clear(ctx.theme.bg)          # fill background
    ctx.rect(x, y, w, h, fill, radius=0.0)   # filled rectangle
    ctx.text(x, y, "label", size=BODY, color=ctx.theme.fg)
    ctx.line(x1, y1, x2, y2, color, width=1.0)
    ctx.circle(cx, cy, r, fill)
```

`ctx.w` and `ctx.h` are the current pane dimensions. Use them for
responsive layout: `ctx.w - PAD * 2` gives full-width minus margins.

### Theme colors

Always use theme colors — never hardcode hex values for semantic roles.

| Token | Use |
|---|---|
| `ctx.theme.bg` | pane background |
| `ctx.theme.surface` | card / panel fill |
| `ctx.theme.fg` | primary text |
| `ctx.theme.muted` | secondary / hint text |
| `ctx.theme.accent` | interactive highlight |
| `ctx.theme.danger` | error / destructive |
| `ctx.theme.success` | confirmation |
| `ctx.theme.border` | dividers, outlines |

`ctx.theme.is_dark` is `True` on dark themes. For app-defined palettes that
auto-switch, use `AppPalette` (see `docs/PGAP_REFERENCE.md`).

### Font size constants

```python
from plexi_sdk import TITLE, HEADING, BODY, CAPTION, HINT, MONO_BODY
# 22.0   18.0     15.0  13.0    12.0  14.0
```

### Keyboard handling

```python
def on_key(self, ctx, key: str, mods: dict) -> None:
    # key: "a"-"z", "up", "down", "left", "right",
    #      "return", "escape", "backspace", "tab", "space", "f1"…"f12"
    # mods: {"shift": bool, "ctrl": bool, "alt": bool, "meta": bool}
```

### Logging

```python
ctx.info("message")    # inside a handler
self.emit.info("msg")  # outside a handler / from threads
```

Logs appear in `~/.plexi-alpha/plexi.log` tagged `app::counter`.

### Notifications

```python
ctx.notify("Done", priority=50, body="Counter reset")
```

Use the named priority constants: `PRIORITY_LOW=0`, `PRIORITY_NORMAL=50`,
`PRIORITY_HIGH=100`, `PRIORITY_CRITICAL=200`.

---

## 7. Next steps

- **UI tree layout** (Column, Card, Header, Footer): `docs/sdk-ui-guide.md`
- **Full draw command reference**: `docs/PGAP_REFERENCE.md` § DrawCommand
- **Capabilities** (HTTP, secrets, AI queries): `docs/PGAP_REFERENCE.md` § Capabilities
- **Example apps**: `apps/calc/`, `apps/backlog/`, `apps/todo/`
