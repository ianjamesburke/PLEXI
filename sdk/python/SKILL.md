# Plexi Python App — Agent Guide

SDK version: **0.3.0 legacy** (`plexi_sdk.py`). Protocol: newline-delimited JSON over stdin/stdout.
This doc is the fast path for current apps, but Plexi v2 is being rewritten around recursive `.plexi` instance boundaries. Prefer the spec index and fractal roadmap for new architecture work: `docs/specs/README.md` and `docs/specs/roadmaps/fractal-pgap/`.

Existing example apps are disposable protocol probes. It is acceptable to delete or rewrite them during v2 if they do not validate recursive instances, capability manifests, typed pipes, depth notifications, or Plexi IQ.

The Python SDK now exposes the recursive probes the v2 apps need: `protocol_version`, `open_intent`, `render_mode`, `capability_manifest`, `status_summary`, and `Suspend`/`Resume`.

---

## Quick-Start

Minimal skeleton that actually runs:

```python
#!/usr/bin/env python3
from __future__ import annotations  # REQUIRED — see gotchas

import os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from plexi_sdk import App

app = App()

@app.on_render
def render(ctx):
    ctx.rect(0, 0, ctx.width, ctx.height, fill="#1e1e2e")
    ctx.text(20, 20, "Hello Plexi!", size=16, color="#cdd6f4")

app.run()
```

Pair it with a `manifest.toml` (see below) and make the file executable (`chmod +x`).

---

## App Lifecycle

Plexi sends JSON events on stdin. `app.run()` reads them in a loop. Register handlers via decorators before calling `run()`.

| Event | Decorator | Signature | When it fires |
|---|---|---|---|
| `init` | `@app.on_init` | `fn()` | First event; inspect `app.protocol_version`, `app.open_intent`, `app.capability_manifest`, and `app.render_mode` |
| `render` | `@app.on_render` | `fn(ctx: RenderContext)` | Every frame — draw here; inspect `ctx.render_mode` for `full` vs `preview` |
| `suspend` | `@app.on_suspend` | `fn()` | Host is pausing work for this depth |
| `resume` | `@app.on_resume` | `fn()` | Host is resuming work for this depth |
| `resize` | `@app.on_resize` | `fn(width, height)` | Pane size changed (before next render) |
| `key` | `@app.on_key` | `fn(key: str, mods: dict, emit: Emitter)` | Key pressed while app has focus |
| `click` | `@app.on_click` | `fn(x, y, button: str, emit: Emitter)` | Mouse click in app surface |
| `mouse_down` | `@app.on_mouse_down` | `fn(x, y, button, emit)` | Mouse button pressed |
| `mouse_up` | `@app.on_mouse_up` | `fn(x, y, button, emit)` | Mouse button released |
| `mouse_move` | `@app.on_mouse_move` | `fn(x, y, emit)` | Mouse moved (opt-in only) |
| `scroll` | `@app.on_scroll` | `fn(x, y, delta_x, delta_y, emit)` | Trackpad/wheel scroll |
| `command` | `@app.on_command` | `fn(text: str, emit: Emitter)` | Command palette entry dispatched to app |
| `drop` | `@app.on_drop` | `fn(target_id, paths: list[str], emit)` | Files dropped on a declared drop target |
| `get_state` | `@app.on_get_state` | `fn() -> dict` | Plexi requests state snapshot |
| `set_state` | `@app.on_set_state` | `fn(state: dict)` | Plexi restores state from snapshot |
| `shutdown` | _(none)_ | — | Process should exit — `run()` returns |

**What to do in render:** Always draw a full-pane background rect first. Build your frame by calling `ctx.*` methods. Frame is committed automatically when the handler returns (the SDK calls `ctx._flush()` which writes `frame_done`).

**Key handler notes:** `key` is a string like `"Enter"`, `"Backspace"`, `"ArrowUp"`, `"j"`, etc. `mods` is `{"shift": bool, "command": bool, "alt": bool, "ctrl": bool}`. Single printable chars come through directly (`"a"`, `"1"`, `" "`).

**State buckets** (for `get_state`/`set_state`):
- `user_state` — undo-managed user data
- `derived` — computed, not saved
- `session` — per-session (scroll offsets, etc.)
- `persistent` — survives app close (bookmarks, prefs)

---

## Drawing Primitives

All coordinates are **logical pixels**. Origin `(0, 0)` is **top-left**. `ctx.width` / `ctx.height` are the current pane dimensions.

### `ctx.rect(x, y, w, h, fill, radius=0.0)`
Filled rectangle. `fill` is a hex color string (`"#rrggbb"` or `"#rrggbbaa"`). `radius` rounds corners.

### `ctx.text(x, y, text, size, color, monospace=False, bold=False)`
Draw text at `(x, y)`. `size` is the font size in logical pixels. Use `monospace=True` for code; `bold=True` for emphasis. No built-in wrapping — wrap text manually before calling.

Convenience aliases: `ctx.text(..., bold=True)` and `ctx.text(..., monospace=True)` — there are no separate `text_bold` / `text_mono` methods; pass the flags.

### `ctx.line(x1, y1, x2, y2, color, width=1.0)`
Draw a line segment. `width` in logical pixels.

### `ctx.image(path, x, y, w, h, fit="contain", rounding=0.0)`
Render an image from disk. `path` may be absolute or relative to the app's cwd. Plexi caches the decoded texture by path+mtime. `fit`: `"contain"` (letterbox), `"cover"` (crop), `"fill"` (stretch). `rounding` rounds corners.

### `ctx.video_thumbnail(path, x, y, w, h, show_play_button=True, timestamp_seconds=0.0)`
Render a video thumbnail extracted at `timestamp_seconds`. Extraction is async (first frame shows a placeholder). Clicking opens the video with the system default player.

### `ctx.file_grid(x, y, w, h, path=None, filter=None, paths=None, item_size=96.0, columns=None, show_labels=True)`
Grid of files with auto thumbnails. Exactly one of `path` (directory walk) or `paths` (explicit list) must be provided. `filter` accepts glob patterns or bare extensions. Clicking an item opens it with the system default handler.

### `ctx.drop_target(id, x, y, w, h, accept=None, label=None)`
Declare a drop zone for Finder file drops. Re-emit every frame to keep it active. `accept` is a list of lowercase extensions without dots (e.g. `["png", "jpg"]`); empty = accept anything. The `id` is echoed back to `@app.on_drop`.

### `ctx.list(items, selected=0, item_height=40.0)`
**WARNING — full-pane only, no position parameters.** Renders at the pane origin with implicit full-pane layout. Cannot be offset. Do NOT use in apps that have a header, sidebar, or any layout smaller than the full viewport — it will overlap. Use `ctx.text` + `ctx.rect` for positioned lists instead. See gotchas.

Each item dict: `{"label": str, "secondary": str | None, "is_dir": bool}`.

### `ctx.set_cursor(cursor)`
Set the cursor icon for this frame. Values: `'default'`, `'pointer'`, `'grab'`, `'grabbing'`, `'crosshair'`, `'text'`. Must be re-emitted each frame (resets to `'default'` per frame).

### `ctx.mouse_tracking(enabled)`
Enable/disable continuous `mouse_move` events. Persists until changed; do not re-emit each frame.

---

## Emitter Commands

`emit` is passed to every non-render handler. It also exists as `ctx.*` equivalents inside render frames (queued with draw commands).

| Method | When to use |
|---|---|
| `emit.run_in_terminal(command)` | Run a shell command in the linked terminal pane. Requires `terminal_write = true` in manifest capabilities. |
| `emit.cd(path)` | Change the linked terminal's cwd. Requires `terminal_write = true`. |
| `emit.notification(title, body=None, priority=1)` | Post to Plexi's notification log (Cmd+Shift+N). Priority: 0=info, 1=normal, 2=high, 3=urgent. |
| `emit.cost_report(service, model, input_tokens, output_tokens, cost_usd)` | Report LLM API cost for tracking. Use after every API call. |
| `emit.status_summary(summary)` | Emit lightweight status/preview metadata for depth trees and recursive panes. |
| `emit.info(msg)` / `.warn(msg)` / `.error(msg)` / `.debug(msg)` | Forward log messages into `~/.plexi-alpha/plexi.log` tagged `app::<app_id>`. |
| `emit.log(level, msg)` | Generic log; level is `"error"`, `"warn"`, `"info"`, or `"debug"`. |
| `emit.spawn_app(app_id, ..., open_intent=None)` | Spawn another app (see below). |
| `emit.submit_feedback(text, rating=None, category=None)` | Append user feedback to the app's `feedback.jsonl`. |

---

## Breakpoints

Register multiple render functions that activate at different pane sizes. Mutually exclusive with `@app.on_render` — use one or the other, not both.

```python
@app.breakpoint(min_width=800, min_height=500)
def render_full(ctx):
    ...  # rich layout for large panes

@app.breakpoint(min_width=400)
def render_compact(ctx):
    ...  # stripped layout

@app.breakpoint()  # 0x0 — always matches; required as fallback
def render_tiny(ctx):
    ...
```

The SDK picks the most specific matching breakpoint (largest `min_width * min_height` area that satisfies `width >= min_width AND height >= min_height`). If no registered breakpoints match, it falls back to the `0x0` entry.

**Min-size auto-fallback:** When `app.set_min_size(w, h)` is called (or `[app.layout] min_width/min_height` is set in the manifest), and the pane is smaller than the floor on either axis, the SDK draws a built-in "too small" frame with a directional arrow and skips all user render functions entirely.

```python
app.set_min_size(400, 300)  # programmatic — overrides manifest
```

---

## spawn_app

Compose apps by spawning a child pane. Call from any handler via `emit.spawn_app(...)` or from inside a render frame via `ctx.spawn_app(...)`.

```python
emit.spawn_app(
    app_id="text-editor",          # must be in the app registry
    args=["/path/to/file.py"],     # forwarded as argv[1..]
    parent="self",                 # "self" | "root"
    layout={"kind": "cols", "slot": 1, "ratio": 0.5},  # right 50% split
    lifecycle="cascade",           # "cascade" | "orphan" | "prompt"
    linked=True,                   # share terminal link group
    wire_channels=None,            # typed-pipe channel names (future)
    open_intent=None,              # OpenIntent.file(...) / .prompt(...) / .resume(...)
)
```

Layout kinds: `"fill"`, `"cols"` (slot 0=left/1=right), `"rows"` (slot 0=top/1=bottom), `"grid_2x2"` (stub, falls back to fill).

The target app's `[app.spawnable]` manifest table controls who may spawn it and which lifecycles it accepts. Refused spawns send an error notification back to the caller. When the host starts the child, the SDK surfaces the launch context on `app.open_intent` and the depth-scoped permission set on `app.capability_manifest`.

---

## Manifest File

Place `manifest.toml` next to the entry point. The registry refuses to load apps without it.

```toml
[app]
id          = "my-app"          # required — unique, used in logs and composition
name        = "My App"          # required — shown in app switcher
entry       = "my_app.py"       # required — must exist and have +x bit set
protocol_version = 2            # required for v2 apps
version     = "0.1.0"           # optional
description = "Does a thing"    # optional

[app.capabilities]
terminal_write = false   # required to use run_in_terminal / cd
network        = false   # required for urllib / http calls
filesystem     = "read_only"  # "none" | "read_only" | "read_write"
mouse_tracking = false   # opt-in to continuous mouse_move events

[app.layout]             # optional — SDK reads this, not the host
min_width  = 400
min_height = 200

[app.launch]             # optional
companion          = "terminal"   # "none" | "terminal"
companion_position = "bottom"     # "bottom" | "right"
companion_size     = 0.25

[app.spawnable]          # optional — composition policy
allow_callers   = ["*"]
allow_lifecycle = ["cascade", "orphan"]
```

Required fields: `[app].id`, `[app].name`, `[app].entry`. Everything else is optional with sensible defaults.

---

## Key Gotchas

**`ctx.list()` has no position parameters.** It renders at the pane origin with full-pane layout. Using it alongside a header or sidebar causes overlap — there is no workaround except manually rendering the list with `ctx.text` + `ctx.rect`. This is a known trap, not an oversight.

**`from __future__ import annotations` must be the first line** of every app `.py` file. macOS GUI bundles do not inherit the user's shell PATH. Plexi probes for a Homebrew Python 3.10+, but the `X | Y` union syntax requires either Python 3.10+ or the `__future__` import. Always add it — it's safe on all 3.10+ versions.

**`Optional[List[str]]` not `Optional[list]`.** The `list` method exists on `RenderContext`. If you write a type annotation `Optional[list]`, pyright/mypy will shadow the method name at class scope. Use `from typing import List, Optional` and write `Optional[List[str]]`.

**Entry point must have `+x` bit.** The registry checks the Unix executable bit and refuses to launch entries without it. After `just install-alpha`, run: `chmod +x ~/.plexi-alpha/apps/*/*.py`

**Vendored SDK must stay in sync.** Each app ships its own copy of `plexi_sdk.py`. After editing the canonical at `sdk/python/plexi_sdk.py`, run `python3 scripts/sync-sdk.py` to propagate the changes to all example apps. Do not hand-edit vendored copies.

**`@app.on_render` and `@app.breakpoint()` are mutually exclusive.** Registering both raises a `RuntimeError` at `app.run()`.

**Background threads and `result_queue`.** Plexi doesn't call your render handler on a separate thread. Use `threading.Thread(daemon=True)` for async work (network, file I/O) and a `queue.Queue` to ferry results back. Drain the queue at the top of `on_render`. See `wikipedia.py` for the canonical pattern.

---

## Install Path

Apps live under the build-specific config directory:

| Build | Install path |
|---|---|
| Alpha | `~/.plexi-alpha/apps/<id>/` |
| Beta | `~/.plexi-beta/apps/<id>/` |
| Stable | `~/.plexi/apps/<id>/` |

Minimal app directory:
```
~/.plexi-alpha/apps/my-app/
  manifest.toml
  my_app.py         # must be executable (+x)
  plexi_sdk.py      # vendored SDK copy
```

**Install command for alpha branch:** `just install-alpha`. This syncs app files but does NOT set executable bits — run `chmod +x ~/.plexi-alpha/apps/*/*.py` after install.

**Local app directories:** Plexi also walks up from the pane's cwd collecting `.plexi/apps/` directories. Local apps override global ones with the same `id`. Useful for project-scoped apps.

---

## Testing

Use `AppTestHarness` from `sdk/python/plexi_test.py` (vendored alongside the SDK):

```python
from plexi_test import AppTestHarness

def test_renders_title():
    h = AppTestHarness("my_app.py")
    h.send_init()
    frames = h.send_render(width=800, height=600)
    h.assert_text_visible("My App Title", frames)
    h.shutdown()
```

`AppTestHarness` spawns the app as a subprocess, sends JSON events on stdin, reads draw commands from stdout, and provides assertion helpers. Run with:

```sh
python3 -m pytest tests/
```

Not bare `python3 tests/test_foo.py` — pytest must be invoked as a module to pick up the test runner correctly.

For the full harness API (`send_key`, `send_click`, `assert_rect_exists`, `get_stderr`, etc.) see `sdk/python/plexi_test.py` directly.
