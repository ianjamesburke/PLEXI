# plexi-sdk

Python SDK for building external apps that run inside [Plexi](https://github.com/ianjamesburke/PLEXI), a spatial terminal multiplexer.

Plexi apps are standalone Python (or Rust) programs that speak a newline-delimited JSON protocol over stdin/stdout. The host sends render/input events, the app responds with draw commands. No GUI toolkit, no framework lock-in — just stdlib and a few hundred lines of SDK glue.

## Status

Pre-release. The protocol is still evolving. Pin a version in production.

## Installation

```sh
pip install plexi-sdk
```

This installs `plexi_sdk` and `plexi_sdk_advanced` as top-level modules so your IDE, type checker, and linter understand Plexi apps during development.

At runtime, production Plexi apps do **not** import from the installed package — see [How apps are installed at runtime](#how-apps-are-installed-at-runtime) below.

## Quick start

A minimal app that draws a background and responds to keys:

```python
#!/usr/bin/env python3
"""hello_app.py — a minimal Plexi app."""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from plexi_sdk import App  # noqa: E402

app = App(app_id="hello-app")

counter = 0


@app.on_render
def render(ctx):
    ctx.rect(0, 0, ctx.width, ctx.height, fill="#1e1e2e")
    ctx.text(20, 20, "Hello Plexi!", size=18, color="#cdd6f4", bold=True)
    ctx.text(20, 50, f"counter = {counter}", size=14, color="#a6adc8")
    ctx.text(20, 80, "press space to increment, q to quit", size=11, color="#6c7086")


@app.on_key
def on_key(key, mods, emit):
    global counter
    if key == " ":
        counter += 1
    elif key == "q":
        emit.info("hello-app: quit requested")


if __name__ == "__main__":
    app.run()
```

Pair it with a `manifest.toml`:

```toml
id = "hello-app"
name = "Hello App"
entry = "hello_app.py"
runtime = "python"

[capabilities]
# declare what your app is allowed to do here
```

Install it into your local Plexi apps directory (`~/.plexi-alpha/apps/hello-app/` on alpha builds) and launch it from the Plexi command palette.

## Core concepts

- **`App`** — the top-level object. Register handlers with `@app.on_render`, `@app.on_key`, `@app.on_mouse`, `@app.on_command`, `@app.on_get_state`, `@app.on_set_state`, etc. Call `app.run()` to enter the event loop.
- **`RenderContext`** — passed to your `on_render` handler. Exposes `ctx.width`, `ctx.height`, plus draw primitives: `rect`, `text`, `image`, `video_thumbnail`, `file_grid`, `list`, and structured logging methods (`ctx.info`, `ctx.warn`, `ctx.error`, `ctx.debug`).
- **`Emitter`** — passed to input handlers. Lets you drive the host: `emit.run_in_terminal(cmd)`, `emit.cost_report(...)`, structured logging, and more.
- **Draw commands** — each draw call produces a JSON message. A frame is a stream of draw commands terminated by a `frame_done` marker; Plexi double-buffers and flushes atomically.
- **Events** — render, key, mouse, command, get_state, set_state. Only subscribe to what you need.
- **State buckets** — `user_state` (undoable), `derived` (recomputable), `session` (per-window), `persistent` (across restarts). Plexi handles undo/redo and serialization; you return dicts.
- **Capabilities** — declared in `manifest.toml`. The app sandbox grants filesystem, network, subprocess, and terminal-write access only when the manifest requests it.

See the [app protocol spec](../../docs/specs/app-infrastructure.md) for the full message reference.

## Breakpoints and minimum size

Most Plexi apps need to handle pane resizing gracefully. The SDK provides two
first-class primitives for this: **breakpoints** (pick a render function by
pane size) and **auto min-size fallback** (draw a built-in "too small" frame
when the pane is below a declared floor).

### Declaring a minimum size

Add an `[app.layout]` table to your `manifest.toml`:

```toml
[app]
id    = "my-app"
name  = "My App"
entry = "my_app.py"

[app.layout]
min_width  = 400   # logical pixels, default 0 (no floor)
min_height = 200   # logical pixels, default 0 (no floor)
```

When either dimension is non-zero and the pane is smaller than the declared
floor on that axis, the SDK draws a built-in "pane too small" frame and
bypasses your `on_render` handler entirely. The frame includes:

- A dark background rect
- A centered `min size: 400 x 200` label
- A directional arrow (`→`, `↓`, or `↘`) pointing at the axes that need to
  grow
- A dim `current: w x h` subtitle

No manual code required — the SDK owns the fallback.

You can also set the floor programmatically at app startup:

```python
app = App(app_id="my-app")
app.set_min_size(400, 200)
```

Opt out of the auto-fallback with `App(auto_min_size=False)` if you want to
handle the small-pane case yourself. Override the palette with
`app.set_min_size_colors(bg=..., fg=..., accent=...)`.

### Breakpoint dispatchers

Instead of hand-rolling an `if width < 400` branch inside a single
`on_render`, stack multiple `@app.breakpoint(...)` handlers:

```python
from plexi_sdk import App

app = App(app_id="dashboard")
app.set_min_size(320, 180)  # anything smaller gets the SDK fallback


@app.breakpoint(min_width=800, min_height=500)
def render_full(ctx):
    # Full sidebar + main + status bar
    ctx.rect(0, 0, ctx.width, ctx.height, fill="#1e1e2e")
    ctx.text(20, 20, "Dashboard (full)", size=18, color="#cdd6f4", bold=True)


@app.breakpoint(min_width=400)
def render_compact(ctx):
    # Narrow single-column view
    ctx.rect(0, 0, ctx.width, ctx.height, fill="#1e1e2e")
    ctx.text(20, 20, "Dashboard (compact)", size=14, color="#cdd6f4")


@app.breakpoint()  # fallback — (0, 0) always matches
def render_minimal(ctx):
    ctx.rect(0, 0, ctx.width, ctx.height, fill="#1e1e2e")
    ctx.text(10, 10, "·", size=12, color="#6c7086")


if __name__ == "__main__":
    app.run()
```

On each render event the SDK walks the registered breakpoints sorted by
`min_width × min_height` descending and calls the first one whose constraints
fit the current pane (`width >= min_width AND height >= min_height`). If none
match, the no-argument `@app.breakpoint()` fallback is used.

`@app.breakpoint(...)` and `@app.on_render` are **mutually exclusive** —
registering both raises a `RuntimeError` at `app.run()` startup.

## `plexi_sdk_advanced`

Higher-level helpers for canvas/game/interactive apps: pan-zoom `Canvas`, `HitTester`, `FrameTimer`, `Tween`, easing functions. Zero extra dependencies.

```python
from plexi_sdk import App
from plexi_sdk_advanced import Canvas, FrameTimer, Tween, ease_out_cubic
```

## Example apps

The [`examples/`](../../examples/) directory in the Plexi repo contains ~30 working apps covering terminals, canvases, file grids, git tooling, LLM integrations, games, and dashboards. Good places to start:

- `hello-app/` — minimal render + key handling + media draw commands
- `todo/` — list, persistent state, keyboard navigation
- `snake/`, `sandfall/` — game loops via `FrameTimer`
- `git-log/`, `github-issues/` — subprocess + list rendering
- `wikipedia/`, `hacker-news/` — network fetches, cost reporting

## How apps are installed at runtime

Plexi apps are **self-contained directories**. Each installed app looks like this:

```
~/.plexi-alpha/apps/hello-app/
    manifest.toml
    hello_app.py
    plexi_sdk.py          <-- vendored copy
    plexi_sdk_advanced.py <-- optional, vendored
```

At runtime, the app adds its own directory to `sys.path` and imports `plexi_sdk` from the file sitting next to it. It does **not** import from a globally installed `pip` package. This keeps apps hermetic — they work regardless of which Python the Plexi bundle uses, and they can't be broken by unrelated pip upgrades.

The `pip install plexi-sdk` package exists primarily for editor and type-checker support while developing a new app. When you ship, copy `plexi_sdk.py` (and `plexi_sdk_advanced.py` if you use it) next to your entry file.

The canonical copy lives at [`sdk/python/plexi_sdk.py`](./plexi_sdk.py) in the Plexi repo. The repo's `scripts/sync-sdk.py` keeps every `examples/*/plexi_sdk.py` in lockstep with it.

## License

MIT. See [LICENSE](./LICENSE).
