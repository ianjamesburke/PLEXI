"""
plexi_sdk — Plexi external app SDK (Python), PGAP v3

Spec: docs/specs/releases/plexi-v3.0.md §3 (PGAP v3), §7 (typed pipes).
Zero dependencies, pure stdlib.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
QUICK START
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

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
            # Pure-sync render hooks work unchanged — no await needed here.
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
            ctx.notify("Clicked", f"({x:.0f}, {y:.0f}) {button}")

    CounterApp().run()


━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
PROTOCOL OVERVIEW (PGAP v3)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Newline-delimited JSON over stdin/stdout. Binary data travels on typed Unix
socket pipes, not stdio.

PlexiEvent  (host → app):
  Init               — handshake; delivers app_id, workspace_root, capabilities,
                        feature_flags, and protocol version string ("pgap/3.x")
  Render             — draw a new frame; carries frame_id and rect {x,y,w,h}
  Key                — keypress; carries key string and modifiers dict
  Click              — pointer event; carries x, y, button string
  Command            — command-palette entry submitted by the user; carries text
  CapabilityDecision — response to a CapabilityRequest; carries request_id and granted bool
  SecretValue        — response to SecretGet; carries key and value (str or null)
  HttpResponse       — response to HttpRequest; carries request_id, body, and optional error
  RunUpdate          — streaming update from a RunGet job; carries run_id and payload
  PipeMessage        — JSON-mode pipe message; carries pipe_id and payload
  PipeOpened         — binary pipe ready; carries pipe_id and socket_path (Unix socket)
  PipeOverrun        — host dropped frames on a pipe; carries pipe_id and dropped_frames count
  PathChanged        — terminal cwd broadcast; carries cwd string
  AppSpawned         — confirmation that a SpawnApp request completed; carries pane_id and type_id
  PaneSpawned        — confirmation that a SpawnPane request completed; carries pane_id
  PaneSpawnError     — SpawnPane could not be fulfilled; carries reason
  InjectState        — host-initiated state injection; carries payload dict
  Suspend            — app is being hidden/backgrounded
  Resume             — app is visible again
  Shutdown           — app should clean up and exit

DrawCommand (app → host):
  Rect          — filled rectangle with optional corner radius
  Circle        — filled circle
  Text          — text label with font size, color, monospace/bold flags
  Line          — straight line segment
  List          — scrollable item list (see ListItem shape below)
  Image         — display a raster image by path or data URI
  VideoPlayer   — embed a video player widget
  AudioMeter    — display a real-time audio level meter
  AudioPlay     — play audio from a file or pipe
  AudioCapture  — open an audio capture stream
  FrameDone     — signals end of a render frame (auto-sent by SDK; do not call manually)
  Log           — structured log line forwarded to the host log
  Notify        — trigger a system notification
  CapabilityRequest — request a runtime capability; host may prompt the user
  SecretGet     — request a secret by key from the host secrets store
  HttpRequest   — broker an HTTP request through the host (requires net.http capability)
  RunGet        — dispatch an intent-based AI/agent job
  RunComplete   — mark a RunGet job as finished
  PipeOpen      — open a typed pipe (json or binary, in/out/duplex)
  PipeSend      — send a JSON payload on a json-mode pipe
  StatusSummary — set the status bar summary text for this pane
  ScheduleRender — ask the host to send a Render event after N milliseconds
  SpawnApp      — request the host to open a new pane with a given app type
  SpawnPane     — request the host to open a pane with given app, layout, args, and optional pipe_id
  CdRequest     — request the host to cd all terminals in the pane group to a path
  Ready         — sent automatically after Init; do not emit manually


━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
THEME CONSTANTS
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Font sizes (float, points):
  TITLE      = 22.0   — primary heading
  HEADING    = 18.0   — section heading
  BODY       = 15.0   — default body text
  CAPTION    = 13.0   — secondary label
  HINT       = 12.0   — muted hint text
  MONO_BODY  = 14.0   — monospace body (code)
  MONO_SMALL = 12.0   — monospace small (log output)

Layout (float, pixels):
  PAD        = 16.0   — standard outer padding
  PAD_TIGHT  =  8.0   — tight/inner padding
  HEADER_H   = 48.0   — standard header bar height
  STATUS_H   = 44.0   — status bar height

Colors (hex strings, Catppuccin Mocha):
  BG        = "#1e1e2e"   — main background
  SURFACE   = "#313244"   — elevated surface / card
  HIGHLIGHT = "#45475a"   — hover / selection highlight
  ACCENT    = "#89b4fa"   — primary accent (blue)
  MUTED     = "#6c7086"   — muted / disabled text
  FG        = "#cdd6f4"   — primary foreground text
  RED       = "#f38ba8"   — error / destructive
  GREEN     = "#a6e3a1"   — success / positive
  YELLOW    = "#f9e2af"   — warning / caution

Color helpers:
  rgba(r, g, b, a=255) -> str  — build an 8-digit hex string #rrggbbaa
  dim(hex_color, alpha) -> str — apply alpha (0-255) to an existing hex color


━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
NOTIFICATIONS
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Four kinds. All are posted via ctx.notify* and render in the central
notification modal (ctx.notify(...) returns immediately; the other three
block the calling thread until the user responds — run them on a worker
thread if the app needs to stay interactive).

  ctx.notify(title, body="", level="info", priority=PRIORITY_NORMAL)
      Fire-and-forget message. Enter / Space acknowledge, Esc dismisses.

  ctx.notify_and_wait(title, body="", priority=...)  -> str
      Same as notify() but blocks. Returns "acknowledge" or "cancel".

  ctx.notify_choice(title, options, body="", required=False, priority=...) -> str
      Blocking choice picker. options = [{"label":..., "value":...,
      "shortcut":...}]. Returns chosen value (or label if no value),
      or "__cancel__" if dismissed.

  ctx.notify_input(title, prompt="", body="", required=False, priority=...) -> str
      Blocking text input. Returns the typed string, or "__cancel__".

  ctx.notify_with_image(title, body, image_bytes, mime, priority=...,
                        choices=None) -> str | None
      Convenience wrapper that handles base64 encoding + 50 KB cap.
      `image_bytes` > 50 KB raises ValueError locally. With `choices=None`
      this is fire-and-forget (returns None); with `choices` set it routes
      to notify_choice and blocks for the user's pick. `mime` must be
      "image/png" or "image/jpeg".

Image attachments (#74) — pass `image_inline={"mime", "base64"}` to any of
the notify* methods, or use `notify_with_image` for the convenience wrap.
Inline images cap at 50 KB decoded; oversized images render a placeholder
badge instead of the bitmap. The `image_pipe_id` field is reserved for a
future host-side rendering primitive — apps cannot publish frames through
it today. Use the inline path for now.

Priority — required kwarg on every call. Use the named constants:

  PRIORITY_LOW      = 0     Background info. Stacks at the bottom of the queue.
  PRIORITY_NORMAL   = 50    Standard confirmations — "note saved", "done", etc.
  PRIORITY_HIGH     = 100   Needs attention soon — not blocking but noticeable.
  PRIORITY_CRITICAL = 200   Interrupt-level. Use sparingly; reserve for user
                            decisions the app genuinely depends on. If every
                            notification is CRITICAL, none is.

(A future version may reserve a user-only priority band above CRITICAL so
a misbehaving app can't yell itself to the top of someone's queue. Apps
should stay under 200 regardless; 0..200 is the app band.)

Queue model:

- Notifications pile into a single priority-sorted queue (priority DESC,
  arrival ASC). The front-most is pinned by id — new notifications
  arriving NEVER change what's on screen, only the total count.
- On dismiss, the next front-most is chosen dynamically from whatever's
  in the queue right now — not from a pre-frozen snapshot.
- Cmd+] / Cmd+[ preview other queued notifications without acknowledging.
  Cmd+Shift+A toggles the modal on/off.

Scope — context vs global — is NOT a runtime choice. It's declared per-app
in the app's manifest.toml::default_notification_scope:

  "context"  — notification is visible only when its source context is active
               (default; safe — local confirmations stay local).
  "global"   — notification is visible in all contexts (use for genuinely
               cross-context things like stand-up reminders).

The user controls which scope a given app uses by editing its manifest.
Apps do not see or set scope at the SDK level.

Round-trip response — notify_choice / notify_input / notify_and_wait all
block for the user's answer. For fire-and-forget notifications that still
need a response, set notify_id explicitly and handle PlexiEvent::NotifyAction
in your event loop. The blocking helpers handle this plumbing internally.


━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
MANIFEST REFERENCE (examples/<app>/manifest.toml)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Required:
  [app]
  id = "my-app"             # stable identifier — used for launch slot, log
                            # target "app::<id>", install dir, pack refs
  name = "My App"           # human-readable display name
  version = "0.1.0"
  description = "…"
  entry = "my_app.py"       # executable entry point, relative to manifest

Optional:
  default_notification_scope = "context"  # "context" | "global" (default "context")

  [app.capabilities]
  capabilities = []         # e.g. ["net.http", "audio.record"]
                            # apps must declare what they use; host prompts
                            # on install (future) and gates at runtime

  [launch]
  layout_hint = { side = "above", split = 0.5 }  # preferred split direction
                                                  # + size when spawned


━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
RenderContext  (ctx passed to on_init, on_render, on_key, on_click, …)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Attributes:
  ctx.x, ctx.y          — pane origin in logical pixels (usually 0, 0)
  ctx.w, ctx.h          — pane width and height in logical pixels
  ctx.frame_id          — monotonically increasing render counter
  ctx.elapsed           — seconds since previous on_render (0.0 on first frame)
  ctx.workspace_root    — absolute path to the workspace root directory
  ctx.capabilities      — list of granted capability strings
  ctx.feature_flags     — list of enabled feature flag strings
  ctx.emit              — Emitter instance (same as self.emit on App)

Drawing methods:
  ctx.clear(fill)
      Fill the entire pane with a solid color. Equivalent to ctx.rect(0, 0, w, h, fill).

  ctx.rect(x, y, w, h, fill, radius=0.0)
      Draw a filled rectangle. radius > 0 rounds the corners.

  ctx.circle(cx, cy, r, fill)
      Draw a filled circle centered at (cx, cy) with radius r.

  ctx.text(x, y, text, size, color, monospace=False, bold=False)
      Draw a text label. x, y are the top-left origin of the text block.

  ctx.line(x1, y1, x2, y2, color, width=1.0)
      Draw a straight line segment.

  ctx.list_view(items, selected=0, item_height=40.0, x=0, y=0, w=None, h=None)
      Draw a scrollable list. w defaults to ctx.w; h defaults to ctx.h - y.
      Each item is a dict — see ListItem shape below.

Notification / logging (usable inside or outside a frame):
  ctx.notify(title, body, level="info", actions=None)
      Trigger a system notification. actions: list of NotifyAction dicts (see below).

  ctx.status_summary(text)
      Set the status bar summary text for this pane.

  ctx.log(level, message)  /  ctx.info(msg)  /  ctx.warn(msg)
  ctx.error(msg)           /  ctx.debug(msg)
      Forward a log line to the host logger, tagged with this app's ID.


━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
Emitter  (self.emit — available at all times, including background threads)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

All methods are thread-safe (protected by a global write lock).

  emit.notify(title, body, level="info", actions=None)
      Trigger a system notification outside of a render frame.

  emit.log(level, message)  /  emit.info(msg)  /  emit.warn(msg)
  emit.error(msg)           /  emit.debug(msg)
      Write a structured log line to the host log.

  emit.status_summary(text)
      Set the status bar summary text for this pane.

  emit.schedule_render(after_ms=16)
      Ask the host to send a Render event after after_ms milliseconds.
      Use at the end of on_render to drive a continuous animation loop.
      16 ms ≈ 60 fps  |  32 ms ≈ 30 fps.

  emit.secret_get(key) -> str | None        [BLOCKING]
      Request a secret by key from the host secrets store. Blocks until the
      host responds. Returns the secret string, or None if denied/not found.

  emit.http_get(url) -> str                 [BLOCKING]
      Broker an HTTP GET through the host. Requires the net.http capability.
      Blocks until the response arrives. Raises RuntimeError on failure.
      Call from a background thread to avoid stalling the render loop.

  emit.ai_query(model_tier, system, messages, tools=None) -> AiResponse  [BLOCKING]
      Plexi AI broker call (#284). Requires the `ai.query` capability declared
      in manifest.toml. `model_tier` is "low" | "medium" | "high"
      (Haiku / Sonnet / Opus). Returns an AiResponse with content, tokens_in,
      tokens_out. Raises CapabilityDeniedError if the manifest didn't grant
      `ai.query`, or RuntimeError on any other backend failure. Call from a
      background thread — the host may take seconds to reply.

  emit.capability_request(capability) -> bool  [BLOCKING]
      Request a runtime capability (e.g. "net.http", "fs.write"). The host
      may show a permission prompt to the user. Blocks until granted or denied.
      Returns True if granted. Call once at startup, not on every render.

  emit.cd_to(cwd)
      Request the host to cd all terminals in the same pane group to cwd.

  emit.run_get(intent, payload=None) -> str
      Dispatch an intent-based AI/agent job. Returns a run_id. Progress arrives
      via RunUpdate PlexiEvents; handle them in on_run_update if needed.

  emit.pipe_open(pipe_id, mode="binary", direction="in") -> Pipe
      Open a typed pipe and return a Pipe handle.
      mode: "json" | "binary"    direction: "in" | "out" | "duplex"
      For binary mode, call pipe.connect() and wait for PipeOpened before I/O.


━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
STRUCTURED ARGUMENT SHAPES
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

mods  (passed to on_key):
    {"shift": bool, "ctrl": bool, "alt": bool, "meta": bool}

ListItem  (each element of the items list passed to ctx.list_view):
    {
        "title":    str,          # primary label (required)
        "subtitle": str,          # secondary label (optional)
        "icon":     str,          # SF Symbol name or emoji (optional)
        "color":    str,          # override title color (optional hex)
        "tag":      str,          # right-aligned badge text (optional)
    }

NotifyAction  (each element of the actions list passed to notify):
    {
        "label": str,             # button label shown in the notification
        "key":   str,             # identifier sent back in a Command event
    }

Pipe  (returned by emit.pipe_open):
    pipe.connect(timeout=5.0) -> bool    — wait for the socket to be ready
    pipe.read_frame()         -> bytes | None  — read one length-prefixed frame
    pipe.write_frame(data)               — write one length-prefixed frame
    pipe.send(payload)                   — JSON-mode send (dict/list/scalar)
    pipe.close()                         — release the socket


━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
App EVENT HANDLERS (override in your subclass)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

  on_init(self, ctx)                            — after Init handshake completes
  on_render(self, ctx)                          — on each Render event; auto-sends FrameDone
  on_key(self, ctx, key, mods)                  — on Key event
  on_click(self, ctx, x, y, button)             — on Click event
  on_command(self, ctx, text)                   — on Command event (command palette)
  on_pipe_message(self, ctx, pipe_id, payload)  — on PipeMessage (json-mode pipe)
  on_path_changed(self, ctx, cwd)               — on PathChanged broadcast
  on_inject(self, ctx, payload)                 — on InjectState from the host
  on_app_spawned(self, pane_id, type_id)        — on AppSpawned confirmation
  on_suspend(self)                              — on Suspend (app hidden/backgrounded)
  on_resume(self)                               — on Resume (app visible again)
  on_shutdown(self)                             — on Shutdown (clean up before exit)

All handlers except on_suspend, on_resume, and on_shutdown receive a
RenderContext as their first argument. on_render is the only handler that
auto-emits FrameDone; all others must NOT emit FrameDone.

ASYNC HANDLERS AND BLOCKING I/O (#393)

Input-driven hooks (on_key, on_click, on_command, on_paste, on_pipe_message,
on_path_changed, on_inject, on_timer) are dispatched as asyncio tasks — the
event loop does NOT wait for them to finish before processing the next event.
This means a slow handler never stalls the stdin reader or delays a Render.

Rules:

  1. Declare handlers ``async def`` whenever they need to do any I/O or call
     ``await``-able Emitter helpers:

         async def on_key(self, ctx, key, mods):
             result = await self.emit.http_get(url)   # non-blocking — fine

  2. Never call blocking operations directly from a handler. These freeze the
     event loop thread and starve all other tasks:

         def on_key(self, ctx, key, mods):
             time.sleep(1)          # BAD — blocks event loop thread
             requests.get(url)      # BAD — blocks event loop thread

     Instead, use ``asyncio.to_thread`` from an async handler, or kick off a
     ``threading.Thread`` and bridge back with ``emit.run_sync()``.

  3. ``on_render`` is awaited directly — all draw commands must complete before
     FrameDone is sent. Keep on_render free of I/O; use on_render to read state
     that background tasks have already fetched and stored.

  4. ``on_init`` and ``on_shutdown`` are also awaited (startup / teardown
     ordering). Blocking I/O in on_init should use ``await`` Emitter helpers.

Call MyApp().run() to start the PGAP event loop. This blocks until Shutdown.
"""

__version__ = "0.5.0"
SDK_ID = f"plexi-sdk-py/{__version__}"

from ._constants import (
    TITLE, HEADING, BODY, CAPTION, HINT, MONO_BODY, MONO_SMALL,
    PAD, PAD_TIGHT, HEADER_H, STATUS_H,
    BG, SURFACE, HIGHLIGHT, ACCENT, MUTED, FG, RED, GREEN, YELLOW,
    PRIORITY_LOW, PRIORITY_NORMAL, PRIORITY_HIGH, PRIORITY_CRITICAL,
    rgba, dim,
)
from ._types import (
    CapabilityDeniedError, VideoHandle, AgentInfo,
    RectCommand, TextCommand, BadgeCommand, TextInputSpec, ShortcutPair, NotifyOption,
)
from ._protocol import AiResponse, MidiPortInfo, MidiDeviceList, PROTOCOL_VERSION
from ._emitter import Emitter, _emit, _make_async_queue, _LOCK
from ._pipe import Pipe
from ._render_context import RenderContext
from ._app import App, Agent
