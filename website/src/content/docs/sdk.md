---
title: "SDK Overview"
description: "Python SDK for building Plexi pane apps"
verified_version: "0.0.505"
---

Python SDK for building Plexi pane apps. Zero dependencies, pure stdlib. Implements PGAP v3 over newline-delimited JSON on stdin/stdout.

## Quick Start

```python
from plexi_sdk import App, BG, FG, BODY, ACCENT

class CounterApp(App):
    async def on_init(self, ctx):
        # Called once after the host completes the Init handshake.
        # ctx.workspace_root, ctx.capabilities, ctx.feature_flags are set.
        # Hooks may be async def (to use await) or plain def (fire-and-forget).
        self.count = 0
        self.emit.info("CounterApp ready")
        # Blocking helpers are coroutines — await them directly:
        # api_key = await self.emit.secret_get("MY_API_KEY")

    def on_render(self, ctx):
        # ctx.w / ctx.h are the current pane dimensions.
        # ctx.elapsed is seconds since the previous render (0.0 on first frame).
        ctx.clear(BG)
        ctx.rect(20, 20, ctx.w - 40, 60, fill="#313244", radius=8.0)
        ctx.text(36, 42, f"Count: {self.count}", size=BODY, color=FG)
        ctx.text(36, 72, "Press +/- to change  •  q to quit", size=12.0, color="#6c7086")

    def on_key(self, ctx, key, mods):
        # key is a string: "a"-"z", "up", "down", "left", "right",
        # "return", "escape", "backspace", "tab", "space", "f1"…"f12", etc.
        # mods shape: {"shift": bool, "ctrl": bool, "alt": bool, "meta": bool}
        if key == "+" or (key == "=" and mods.get("shift")):
            self.count += 1
        elif key == "-":
            self.count -= 1
        elif key == "q":
            pass  # host handles quit; apps cannot self-exit

    def on_click(self, ctx, x, y, button):
        # button: "primary" | "secondary" | "middle"
        # x, y are pixel coordinates within the pane
        ctx.notify("Clicked", priority=50, body=f"({x:.0f}, {y:.0f}) {button}")

CounterApp().run()
```

## Protocol Overview

Newline-delimited JSON over stdin/stdout. Binary data travels on typed Unix socket pipes, not stdio.

**Events the host sends to the app:**

| Event | Description |
|---|---|
| `Init` | Handshake — delivers app_id, workspace_root, capabilities, feature_flags, and protocol version |
| `Render` | Draw a new frame; carries frame_id and rect {x,y,w,h} |
| `Key` | Keypress; carries key string and modifiers dict |
| `Click` | Pointer event; carries x, y, button string |
| `Command` | Command-palette entry submitted by the user; carries text |
| `CapabilityDecision` | Response to a CapabilityRequest; carries request_id and granted bool |
| `SecretValue` | Response to SecretGet; carries key and value (str or null) |
| `HttpResponse` | Response to HttpRequest; carries request_id, body, and optional error |
| `RunUpdate` | Streaming update from a RunGet job; carries run_id and payload |
| `PipeMessage` | JSON-mode pipe message; carries pipe_id and payload |
| `PipeOpened` | Binary pipe ready; carries pipe_id and socket_path |
| `PipeOverrun` | Host dropped frames on a pipe; carries pipe_id and dropped_frames count |
| `PathChanged` | Terminal cwd broadcast; carries cwd string |
| `PaneSpawned` | Confirmation that a SpawnPane request completed; carries pane_id |
| `PaneSpawnError` | SpawnPane could not be fulfilled; carries reason |
| `InjectState` | Host-initiated state injection; carries payload dict |
| `Suspend` | App is being hidden/backgrounded |
| `Resume` | App is visible again |
| `Shutdown` | App should clean up and exit |

**Commands the app sends to the host:**

| Command | Description |
|---|---|
| `Rect` | Filled rectangle with optional corner radius |
| `Circle` | Filled circle |
| `Text` | Text label with font size, color, monospace/bold flags |
| `Line` | Straight line segment |
| `List` | Scrollable item list |
| `Image` | Display a raster image by path or data URI |
| `VideoPlayer` | Embed a video player widget |
| `AudioMeter` | Display a real-time audio level meter |
| `AudioPlay` | Play audio from a file or pipe |
| `AudioCapture` | Open an audio capture stream |
| `FrameDone` | Signals end of a render frame (auto-sent by SDK; do not call manually) |
| `Log` | Structured log line forwarded to the host log |
| `Notify` | Trigger a system notification |
| `CapabilityRequest` | Request a runtime capability; host may prompt the user |
| `SecretGet` | Request a secret by key from the host secrets store |
| `HttpRequest` | Broker an HTTP request through the host (requires net.http capability) |
| `RunGet` | Dispatch an intent-based AI/agent job |
| `RunComplete` | Mark a RunGet job as finished |
| `PipeOpen` | Open a typed pipe (json or binary, in/out/duplex) |
| `PipeSend` | Send a JSON payload on a json-mode pipe |
| `StatusSummary` | Set the status bar summary text for this pane |
| `ScheduleRender` | Ask the host to send a Render event after N milliseconds |
| `SpawnPane` | Request the host to open a pane with given app, layout, args, and optional pipe_id |
| `CdRequest` | Request the host to cd all terminals in the pane group to a path |
| `Ready` | Sent automatically after Init; do not emit manually |

## Theme Constants

**Font sizes** (float, points):

| Constant | Value | Use |
|---|---|---|
| `TITLE` | 22.0 | Primary heading |
| `HEADING` | 18.0 | Section heading |
| `BODY` | 15.0 | Default body text |
| `CAPTION` | 13.0 | Secondary label |
| `HINT` | 12.0 | Muted hint text |
| `MONO_BODY` | 14.0 | Monospace body (code) |
| `MONO_SMALL` | 12.0 | Monospace small (log output) |

**Layout** (float, pixels):

| Constant | Value | Use |
|---|---|---|
| `PAD` | 16.0 | Standard outer padding |
| `PAD_TIGHT` | 8.0 | Tight/inner padding |
| `HEADER_H` | 48.0 | Standard header bar height |
| `STATUS_H` | 44.0 | Status bar height |

**Colors** (hex strings, Catppuccin Mocha):

| Constant | Value | Use |
|---|---|---|
| `BG` | `#1e1e2e` | Main background |
| `SURFACE` | `#313244` | Elevated surface / card |
| `HIGHLIGHT` | `#45475a` | Hover / selection highlight |
| `ACCENT` | `#89b4fa` | Primary accent (blue) |
| `MUTED` | `#6c7086` | Muted / disabled text |
| `FG` | `#cdd6f4` | Primary foreground text |
| `RED` | `#f38ba8` | Error / destructive |
| `GREEN` | `#a6e3a1` | Success / positive |
| `YELLOW` | `#f9e2af` | Warning / caution |

**Color helpers:**

```python
rgba(r, g, b, a=255) -> str   # build an 8-digit hex string #rrggbbaa
dim(hex_color, alpha) -> str  # apply alpha (0–255) to an existing hex color
```

## Notifications

Eight methods across two groups: blocking (await) and non-blocking (callback).

**Blocking** — await these from async hooks, or call via `emit.run_sync()` from threads:

```python
ctx.notify(title, priority, body="", level="info")
# Fire-and-forget message. Enter/Space acknowledge, Esc dismisses.

ctx.notify_and_wait(title, priority, body="") -> str
# Same as notify() but blocks. Returns "acknowledge" or "cancel".

ctx.notify_choice(title, options, priority, body="", required=False) -> str
# Blocking choice picker. options = [{"label":..., "value":..., "shortcut":...}]
# Returns chosen value (or label if no value), or "__cancel__" if dismissed.

ctx.notify_input(title, priority, prompt="", body="", required=False) -> str
# Blocking text input. Returns the typed string, or "__cancel__".

ctx.notify_with_image(title, body, image_bytes, mime, priority,
                      level="info", choices=None) -> str | None
# Convenience wrapper that handles base64 encoding + 50 KB cap.
# image_bytes > 50 KB raises ValueError locally.
# With choices=None this is fire-and-forget (returns None);
# with choices set it routes to notify_choice and blocks.
# mime must be "image/png" or "image/jpeg".
```

**Non-blocking** — return immediately with `notify_id`; `on_response` callback fires on the event thread when the user responds:

```python
ctx.notify_async(title, priority, body="", on_response=None) -> str
ctx.notify_and_wait_async(title, priority, body="", on_response=None) -> str
ctx.notify_choice_async(title, options, priority, body="", on_response=None) -> str
ctx.notify_input_async(title, priority, prompt="", body="", on_response=None) -> str
```

**Priority constants:**

| Constant | Value | Use |
|---|---|---|
| `PRIORITY_LOW` | 0 | Background info |
| `PRIORITY_NORMAL` | 50 | Standard confirmations |
| `PRIORITY_HIGH` | 100 | Needs attention soon |
| `PRIORITY_CRITICAL` | 200 | Interrupt-level — use sparingly |

**Queue model:** Notifications pile into a single priority-sorted queue (priority DESC, arrival ASC). The front-most is pinned by id — new notifications arriving never change what's on screen, only the total count. On dismiss, the next front-most is chosen dynamically. `Cmd+]` / `Cmd+[` preview other queued notifications without acknowledging. `Cmd+Shift+A` toggles the modal.

**Scope** is declared in `manifest.toml`, not set at runtime:

```toml
[launch]
notification_scope = "global"   # "window" (default) | "context" | "global"
```

- `"window"` — visible only when the app's window is active.
- `"context"` — visible whenever the user is in the same sidebar project.
- `"global"` — always visible across all contexts (timers, monitoring dashboards).

## Manifest Reference

```toml
# Required
[app]
id = "my-app"          # stable identifier — used for launch slot, logs, install dir
name = "My App"        # human-readable display name
version = "0.1.0"
description = "…"
entry = "my_app.py"    # executable entry point, relative to manifest

# Optional
[launch]
notification_scope = "context"              # "window" (default) | "context" | "global"
layout_hint = { side = "above", split = 0.5 }

[app.capabilities]
capabilities = []      # e.g. ["net.http", "audio.record"]
                       # host gates at runtime; apps must declare what they use
```

## RenderContext

`ctx` is passed to `on_init`, `on_render`, `on_key`, `on_click`, and all other handlers.

**Attributes:**

```python
ctx.x, ctx.y        # pane origin in logical pixels (usually 0, 0)
ctx.w, ctx.h        # pane width and height in logical pixels
ctx.frame_id        # monotonically increasing render counter
ctx.elapsed         # seconds since previous on_render (0.0 on first frame)
ctx.workspace_root  # absolute path to the workspace root directory
ctx.capabilities    # list of granted capability strings
ctx.feature_flags   # list of enabled feature flag strings
ctx.emit            # Emitter instance (same as self.emit on App)
```

**Drawing methods:**

```python
ctx.clear(fill)
# Fill the entire pane with a solid color.

ctx.rect(x, y, w, h, fill, radius=0.0)
# Draw a filled rectangle. radius > 0 rounds the corners.

ctx.circle(cx, cy, r, fill)
# Draw a filled circle centered at (cx, cy) with radius r.

ctx.text(x, y, text, size, color, monospace=False, bold=False)
# Draw a text label. x, y are the top-left origin of the text block.

ctx.line(x1, y1, x2, y2, color, width=1.0)
# Draw a straight line segment.

ctx.list_view(items, selected=0, item_height=40.0, x=0, y=0, w=None, h=None)
# Draw a scrollable list. w defaults to ctx.w; h defaults to ctx.h - y.
# Each item is a dict — see Structured Shapes below.
```

**Notification and logging** (usable inside or outside a frame):

```python
ctx.notify(title, priority, body="", level="info", actions=None)
ctx.status_summary(text)
ctx.log(level, message)
ctx.info(msg) / ctx.warn(msg) / ctx.error(msg) / ctx.debug(msg)
```

## Emitter

`self.emit` is available at all times, including background threads. All methods are thread-safe.

```python
emit.notify(title, priority, body="", level="info", actions=None)
emit.log(level, message)
emit.info(msg) / emit.warn(msg) / emit.error(msg) / emit.debug(msg)
emit.status_summary(text)

emit.schedule_render(after_ms=16)
# Ask the host to send a Render event after after_ms milliseconds.
# 16 ms ≈ 60 fps  |  32 ms ≈ 30 fps

emit.secret_get(key) -> str | None        # [BLOCKING]
# Request a secret by key. Blocks until host responds.

emit.http_get(url) -> str                 # [BLOCKING]
# Broker an HTTP GET through the host. Requires net.http capability.
# Call from a background thread to avoid stalling the render loop.

emit.ai_query(model_tier, system, messages, tools=None) -> AiResponse  # [BLOCKING]
# model_tier: "low" | "medium" | "high" (Haiku / Sonnet / Opus)
# Requires the ai.query capability in manifest.toml.
# Call from a background thread — the host may take seconds to reply.

emit.capability_request(capability) -> None  # [BLOCKING]
# Request a runtime capability. Raises CapabilityDeniedError if denied.
# Call once at startup, not on every render.

emit.cd_to(cwd)
# Request the host to cd all terminals in the same pane group to cwd.

emit.run_get(intent, payload=None) -> str
# Dispatch an intent-based AI/agent job. Returns a run_id.
# Progress arrives via RunUpdate events; handle in on_run_update.

emit.pipe_open(pipe_id, mode="binary", direction="in") -> Pipe
# Open a typed pipe and return a Pipe handle.
# mode: "json" | "binary"    direction: "in" | "out" | "duplex"
```

## Structured Shapes

**`mods`** (passed to `on_key`):

```python
{"shift": bool, "ctrl": bool, "alt": bool, "meta": bool}
```

**`ListItem`** (each element passed to `ctx.list_view`):

```python
{
    "title":    str,   # primary label (required)
    "subtitle": str,   # secondary label (optional)
    "icon":     str,   # SF Symbol name or emoji (optional)
    "color":    str,   # override title color — hex (optional)
    "tag":      str,   # right-aligned badge text (optional)
}
```

**`NotifyAction`** (each element of the `actions` list passed to `notify`):

```python
{
    "label": str,   # button label shown in the notification
    "key":   str,   # identifier sent back in a Command event
}
```

**`Pipe`** (returned by `emit.pipe_open`):

```python
pipe.connect(timeout=5.0) -> bool    # wait for the socket to be ready
pipe.read_frame()         -> bytes | None  # read one length-prefixed frame
pipe.write_frame(data)               # write one length-prefixed frame
pipe.send(payload)                   # JSON-mode send (dict/list/scalar)
pipe.close()                         # release the socket
```

## Event Handlers

Override these methods in your `App` subclass:

```python
on_init(self, ctx)                            # after Init handshake completes
on_render(self, ctx)                          # on each Render event; auto-sends FrameDone
on_key(self, ctx, key, mods)                  # on Key event
on_click(self, ctx, x, y, button)             # on Click event
on_command(self, ctx, text)                   # on Command event (command palette)
on_pipe_message(self, ctx, pipe_id, payload)  # on PipeMessage (json-mode pipe)
on_path_changed(self, ctx, cwd)               # on PathChanged broadcast
on_inject(self, ctx, payload)                 # on InjectState from the host
on_suspend(self)                              # on Suspend (app hidden/backgrounded)
on_resume(self)                               # on Resume (app visible again)
on_shutdown(self)                             # on Shutdown (clean up before exit)
```

All handlers except `on_suspend`, `on_resume`, and `on_shutdown` receive a `RenderContext` as their first argument. `on_render` is the only handler that auto-emits `FrameDone`; all others must not emit it.

### Async handlers and blocking I/O

Input-driven hooks (`on_key`, `on_click`, `on_command`, `on_pipe_message`, `on_path_changed`, `on_inject`) are dispatched as asyncio tasks — the event loop does not wait for them to finish before processing the next event.

**Declare handlers `async def` whenever they need I/O:**

```python
async def on_key(self, ctx, key, mods):
    result = await self.emit.http_get(url)   # non-blocking — fine
```

**Never call blocking operations directly from a handler:**

```python
def on_key(self, ctx, key, mods):
    time.sleep(1)       # BAD — blocks event loop thread
    requests.get(url)   # BAD — blocks event loop thread
```

Instead, use `asyncio.to_thread` from an async handler, or kick off a `threading.Thread` and bridge back with `emit.run_sync()`.

`on_render`, `on_init`, and `on_shutdown` are awaited directly. Keep `on_render` free of I/O — use it to read state that background tasks have already fetched.

Call `MyApp().run()` to start the PGAP event loop. This blocks until Shutdown.
