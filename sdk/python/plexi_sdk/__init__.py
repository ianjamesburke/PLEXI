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

  ctx.list(items, selected=0, item_height=40.0, x=0, y=0, w=None, h=None)
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

ListItem  (each element of the items list passed to ctx.list):
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

Call MyApp().run() to start the PGAP event loop. This blocks until Shutdown.
"""
from __future__ import annotations

__version__ = "0.5.0"
SDK_ID = f"plexi-sdk-py/{__version__}"

import asyncio
import inspect
import json
import socket
import struct
import sys
import threading
import uuid
from dataclasses import dataclass
from typing import Any, Coroutine


# ── ai.query (#284) types ─────────────────────────────────────────────────────

@dataclass
class AiResponse:
    """Result of `Emitter.ai_query`. `tokens_in`/`tokens_out` are zero on error."""
    content: str
    tokens_in: int
    tokens_out: int


class CapabilityDeniedError(RuntimeError):
    """Raised when the host rejects a brokered call because the app's manifest
    didn't declare the required capability. Distinct from generic RuntimeError
    so apps can catch the gate-denial path explicitly."""


# ── CoreMIDI (#320) types ─────────────────────────────────────────────────────

@dataclass
class MidiPortInfo:
    """One MIDI port (input or output). Mirrors `MidiPortWire` on the wire."""
    id: str
    name: str
    default: bool


@dataclass
class MidiDeviceList:
    """Result of `Emitter.list_midi_devices`. Both lists may be empty."""
    inputs: list[MidiPortInfo]
    outputs: list[MidiPortInfo]


# ── Video substrate (#345) types ──────────────────────────────────────────────

@dataclass
class VideoHandle:
    """Result of `Emitter.open_video`. `handle_id` is opaque — pass it back to
    `set_video_state` and `close_video`. The associated `Pipe` delivers
    decoded RGBA8 frames (one frame per `pipe.read_frame()`) of length
    `width * height * 4`."""
    handle_id: int
    width: int
    height: int
    fps: float
    duration_ms: int
    pipe: "Pipe"


# ── agents.list (#286) types ──────────────────────────────────────────────────

@dataclass
class AgentInfo:
    """One row of the agent roster returned by `Emitter.agent_roster`.

    `pane_id` is the host's stable id for the agent pane; pass it to
    `Emitter.pipe_open_directed(pipe_id, pane_id)` to wire an inter-agent
    channel. `app_id` is the manifest id for subprocess agents (or the
    sentinel `"iq"` for the legacy in-process Cmd+I agent backend).
    """
    pane_id: int
    app_id: str
    name: str

# ── Theme constants ───────────────────────────────────────────────────────────
TITLE   = 22.0; HEADING = 18.0; BODY = 15.0; CAPTION = 13.0; HINT = 12.0
MONO_BODY = 14.0; MONO_SMALL = 12.0
PAD = 16.0; PAD_TIGHT = 8.0; HEADER_H = 48.0; STATUS_H = 44.0

BG        = "#1e1e2e"
SURFACE   = "#313244"
HIGHLIGHT = "#45475a"
ACCENT    = "#89b4fa"
MUTED     = "#6c7086"
FG        = "#cdd6f4"
RED       = "#f38ba8"
GREEN     = "#a6e3a1"
YELLOW    = "#f9e2af"

# ── Notification priority tiers ───────────────────────────────────────────────
# Higher = more urgent. Queue sorts priority DESC, arrival ASC. See the
# NOTIFICATIONS block in the module docstring for guidance on which tier to
# pick. Range 0..200 is the "app band" — stay inside it. A future release
# may reserve priorities above 200 for user overrides so apps can't yell
# themselves to the top; staying in-band today keeps you forward-compatible.
PRIORITY_LOW      = 0
PRIORITY_NORMAL   = 50
PRIORITY_HIGH     = 100
PRIORITY_CRITICAL = 200

# ── Color helpers ─────────────────────────────────────────────────────────────

def rgba(r: int, g: int, b: int, a: int = 255) -> str:
    """Return an 8-digit hex color string #rrggbbaa."""
    return f"#{r:02x}{g:02x}{b:02x}{a:02x}"

def dim(hex_color: str, alpha: int) -> str:
    """Return hex_color with the given alpha (0-255). Strips existing alpha."""
    h = hex_color.lstrip("#")[:6]
    return f"#{h}{alpha:02x}"

# ── Internal helpers ──────────────────────────────────────────────────────────
_LOCK = threading.Lock()

def _emit(obj: dict) -> None:
    """Thread-safe JSON line write to stdout."""
    with _LOCK:
        sys.stdout.write(json.dumps(obj) + "\n")
        sys.stdout.flush()


def _make_async_queue() -> asyncio.Queue:
    """Return a fresh asyncio.Queue for one pending request slot."""
    return asyncio.Queue(maxsize=1)


# ── Emitter (always available, even outside a frame) ─────────────────────────

class Emitter:
    """Emit out-of-frame commands to Plexi. Thread-safe."""

    def __init__(self, app: "App"):
        self._app = app

    # Logging
    def log(self, level: str, message: str) -> None:
        _emit({"type": "log", "level": level, "message": message})

    def info(self, message: str) -> None:  self.log("info", message)
    def warn(self, message: str) -> None:  self.log("warn", message)
    def error(self, message: str) -> None: self.log("error", message)
    def debug(self, message: str) -> None: self.log("debug", message)

    # Notifications — kind = "message" (plain text, one Acknowledge button).
    #
    # `priority` is REQUIRED on every notify call. Higher = more urgent.
    # The host uses it to pick the next front-most notification after
    # dismiss and to order the Cmd+] / Cmd+[ preview traversal. Typical
    # values: 0 (background info), 50 (normal), 100 (important), 200
    # (critical). No default — apps must pick deliberately.
    #
    # Scope (context vs. global) is NOT set by the app. It's a per-app
    # user-facing policy declared in manifest.toml:default_notification_scope
    # and resolved by the host at dispatch time. Apps just call notify();
    # the user chooses whether to be interrupted across contexts.
    def notify(self, title: str, body: str = "", level: str = "info",
               priority: int | None = None,
               actions: list | None = None,
               image_inline: dict | None = None,
               image_pipe_id: str | None = None) -> None:
        """Post a message notification. The modal shows title + body and a
        single Acknowledge button; Enter / Space acknowledge, Esc dismisses
        (unless required=True — use `notify_and_wait` for that flow).

        `priority` is required (int, higher = more urgent).
        `actions` is the legacy side-effect list (action_type =
        resume_run | open_intent | run_command). It does NOT render UI.
        `image_inline` (#74): {"mime": "image/png"|"image/jpeg",
        "base64": str}. Decoded host-side; >50KB triggers a placeholder.
        `image_pipe_id` (#74): pipe id of a binary ring carrying RGBA frames
        prefixed with width/height u32-LE. Mutually exclusive with
        `image_inline` — host warns + drops the pipe ref if both set.
        """
        if priority is None:
            raise TypeError("notify() requires 'priority' (int, higher = more urgent)")
        payload = {"type": "notify", "level": level, "title": title,
                   "body": body, "kind": "message",
                   "priority": int(priority),
                   "actions": actions or []}
        if image_inline is not None:
            payload["image_inline"] = image_inline
        if image_pipe_id is not None:
            payload["image_pipe_id"] = image_pipe_id
        _emit(payload)

    def run_sync(self, coro: "Any") -> Any:
        """Run a coroutine from a background thread and return its result.

        Use this when calling async Emitter methods from a non-async context
        (e.g. inside a ``threading.Thread`` callback)::

            def _worker(self) -> None:
                result = self.emit.run_sync(self.emit.http_get(url))

        Must NOT be called from within the event loop thread — it will deadlock.
        Raises ``RuntimeError`` if there is no running event loop to dispatch to
        (e.g. called before ``App.run()`` starts).
        """
        loop = self._app._loop
        if loop is None:
            raise RuntimeError(
                "emit.run_sync() called before the event loop started. "
                "Only call it after App.run() is running (e.g. from a "
                "background thread started inside on_init or later)."
            )
        future = asyncio.run_coroutine_threadsafe(coro, loop)
        return future.result()

    # kind = "choice"
    async def notify_choice(self, title: str, options: list, body: str = "",
                            level: str = "info", required: bool = False,
                            priority: int | None = None,
                            image_inline: dict | None = None,
                            image_pipe_id: str | None = None) -> str:
        """Post a choice notification and await until the user picks one.

        `options` is a list of dicts: {"label": str, "value": str (optional),
        "shortcut": str (optional, single char)}. If `value` is omitted, the
        label is returned. Issue #74's structured-choice spec maps directly:
        `key` → `shortcut`, `payload` → `value`.

        `priority` is required (int, higher = more urgent).
        `image_inline` (#74): {"mime", "base64"} — same shape as `notify`.
        `image_pipe_id` (#74): pipe id of a binary ring carrying RGBA frames.
        Returns the chosen option's value (or the label if no value set). If
        `required=False` the user may cancel with Esc — this returns the string
        `"__cancel__"`.

        Await this from async hooks. From background threads use
        ``self.emit.run_sync(self.emit.notify_choice(...))``.
        """
        if priority is None:
            raise TypeError("notify_choice() requires 'priority' (int, higher = more urgent)")
        notify_id = str(uuid.uuid4())
        q: asyncio.Queue[str] = _make_async_queue()
        self._app._pending_notify[notify_id] = q
        payload = {"type": "notify", "level": level, "title": title, "body": body,
                   "kind": "choice", "options": options, "required": required,
                   "priority": int(priority),
                   "notify_id": notify_id}
        if image_inline is not None:
            payload["image_inline"] = image_inline
        if image_pipe_id is not None:
            payload["image_pipe_id"] = image_pipe_id
        _emit(payload)
        return await q.get()

    # Convenience wrapper for the inline-image case (#74). Handles base64
    # encoding + size-cap enforcement client-side so we never spend wire
    # bytes on payloads the host will only reject.
    async def notify_with_image(self, title: str, body: str, image_bytes: bytes,
                                mime: str, level: str = "info",
                                priority: int | None = None,
                                choices: list | None = None) -> "str | None":
        """Post a notification with an inline base64-encoded image attachment.

        `image_bytes` must be ≤ 50_000 bytes — anything larger raises
        `ValueError` here so the app sees a fast, local failure instead of
        having the host silently render a placeholder. Use the pipe-ref
        path (`emit.notify(..., image_pipe_id=...)`) for larger images.

        `mime` must be `"image/png"` or `"image/jpeg"`.

        `choices` is optional. When None this posts a fire-and-forget
        message-kind notification (returns None). When set, this routes to
        `notify_choice` and awaits until the user picks one (returns the
        chosen value, or `"__cancel__"`).

        Await this from async hooks. From background threads use
        ``self.emit.run_sync(self.emit.notify_with_image(...))``.
        """
        if priority is None:
            raise TypeError("notify_with_image() requires 'priority' (int, higher = more urgent)")
        if mime not in ("image/png", "image/jpeg"):
            raise ValueError(f"notify_with_image() mime must be image/png or image/jpeg, got {mime!r}")
        # 50 KB cap: matches the host's `MAX_INLINE_IMAGE_BYTES`. Apps that
        # exceed this should use the pipe-ref path. Failing locally is the
        # least surprising behaviour.
        MAX_INLINE_BYTES = 50 * 1024
        if len(image_bytes) > MAX_INLINE_BYTES:
            raise ValueError(
                f"notify_with_image(): image is {len(image_bytes)} bytes, "
                f"exceeds {MAX_INLINE_BYTES}-byte cap. Use image_pipe_id for large images."
            )
        import base64
        encoded = base64.standard_b64encode(image_bytes).decode("ascii")
        image_inline = {"mime": mime, "base64": encoded}
        if choices is None:
            self.notify(title=title, body=body, level=level,
                        priority=priority, image_inline=image_inline)
            return None
        return await self.notify_choice(title=title, options=choices, body=body,
                                        level=level, priority=priority,
                                        image_inline=image_inline)

    # kind = "input"
    async def notify_input(self, title: str, prompt: str = "", body: str = "",
                           level: str = "info", required: bool = False,
                           priority: int | None = None) -> str:
        """Post an input notification and await until the user submits or
        cancels. Returns the typed text (possibly empty), or "__cancel__" if
        the user dismissed with Esc (only possible when required=False).

        `priority` is required (int, higher = more urgent).

        Await this from async hooks. From background threads use
        ``self.emit.run_sync(self.emit.notify_input(...))``.
        """
        if priority is None:
            raise TypeError("notify_input() requires 'priority' (int, higher = more urgent)")
        notify_id = str(uuid.uuid4())
        q: asyncio.Queue[str] = _make_async_queue()
        self._app._pending_notify[notify_id] = q
        _emit({"type": "notify", "level": level, "title": title, "body": body,
               "kind": "input", "input_prompt": prompt, "required": required,
               "priority": int(priority),
               "notify_id": notify_id})
        return await q.get()

    async def notify_and_wait(self, title: str, body: str = "", level: str = "info",
                              actions: list | None = None,
                              priority: int | None = None) -> str:
        """Post a message notification and await until the user acknowledges
        or cancels. Returns "acknowledge" on Enter/Space/button, "cancel" on Esc.

        For richer interaction, use `notify_choice` or `notify_input`.
        `priority` is required (int, higher = more urgent).
        `actions` is the legacy server-side side-effect list.

        Await this from async hooks. From background threads use
        ``self.emit.run_sync(self.emit.notify_and_wait(...))``.
        """
        if priority is None:
            raise TypeError("notify_and_wait() requires 'priority' (int, higher = more urgent)")
        notify_id = str(uuid.uuid4())
        q: asyncio.Queue[str] = _make_async_queue()
        self._app._pending_notify[notify_id] = q
        _emit({"type": "notify", "level": level, "title": title, "body": body,
               "kind": "message", "actions": actions or [],
               "priority": int(priority),
               "notify_id": notify_id})
        return await q.get()

    # Terminal commands (legacy back-compat)
    def run_in_terminal(self, command: str) -> None:
        _emit({"type": "run_in_terminal", "command": command})

    def cd(self, path: str) -> None:
        _emit({"type": "cd", "path": path})

    def status_summary(self, text: str) -> None:
        _emit({"type": "status_summary", "text": text})

    def cd_to(self, cwd: str) -> None:
        """Request the host to cd all terminals in the same pane group to `cwd`."""
        _emit({"type": "cd_request", "cwd": cwd})

    def copy_to_clipboard(self, text: str) -> None:
        """Write `text` to the OS clipboard via the host (issue #146).

        Routed through `egui::Context::copy_text` so the platform backend
        (NSPasteboard / X11 / Wayland / Win32) handles the actual write.
        Synchronous from the app's perspective — no acknowledgement event.
        No capability flag is required; clipboard writes are low-risk and
        the app already chooses when to fire (key handler, button, etc.).
        """
        _emit({"type": "copy_to_clipboard", "text": text})


    # ── Canvas Terminal Binding Primitives (#78) ────────────────────────────
    #
    # All five gate on the manifest capability `terminal.bindings`. Apps
    # without it get either a sentinel response (for the request/response
    # primitives) or a silent drop (for fire-and-forget primitives) — the
    # host logs `capability denied` either way so the bug is debuggable.
    #
    # The lifecycle: call `request_linked_terminal()` once at startup
    # (typically inside `on_init`) and stash the returned pane id. Pass it
    # to every subsequent `run_in_linked_terminal` / `insert_path_token` /
    # `request_command_preview` call.
    async def request_linked_terminal(
        self,
        cwd: "str | None" = None,
        label: "str | None" = None,
    ) -> int:
        """Ask the host to open a fresh terminal pane next to this app and
        return its `terminal_pane_id` (an integer). Awaits until the host
        emits `LinkedTerminalReady`.

        Returns 0 if the host denied the request — this happens when the
        manifest doesn't declare `terminal.bindings`. Apps should treat 0
        as a hard error (no terminal to drive). Raises `CapabilityDeniedError`
        in that case so the failure is loud.

        Await this from async hooks. From background threads use
        ``self.emit.run_sync(self.emit.request_linked_terminal(...))``.
        """
        req_id = str(uuid.uuid4())
        q: asyncio.Queue[int] = _make_async_queue()
        self._app._pending_linked_terminal[req_id] = q
        _emit({
            "type": "request_linked_terminal",
            "request_id": req_id,
            "cwd": cwd,
            "label": label,
        })
        pane_id = await q.get()
        if pane_id == 0:
            raise CapabilityDeniedError(
                "request_linked_terminal: capability 'terminal.bindings' "
                "not declared in manifest"
            )
        return int(pane_id)

    def run_in_linked_terminal(
        self,
        terminal_pane_id: int,
        command: str,
        echo: bool = True,
    ) -> None:
        """Run `command` in the linked terminal. With `echo=True` the user
        sees the command typed into the terminal; with `echo=False` the
        signalled intent is silent execution (PTY-level echo is shell-
        controlled — the flag is best-effort observational).

        Fire-and-forget — capability denial drops silently with a host
        log line."""
        _emit({
            "type": "run_in_linked_terminal",
            "terminal_pane_id": int(terminal_pane_id),
            "command": command,
            "echo": bool(echo),
        })

    def insert_path_token(
        self,
        terminal_pane_id: int,
        path: str,
        mode: str = "replace",
    ) -> None:
        """Inject `path` at the linked terminal's cursor.

        `mode = "replace"` — Ctrl-W (kill-word) before the path so the
                            shell readline removes the partial token.
        `mode = "append"`  — write the path verbatim.

        The host POSIX-quotes the path when it contains shell metacharacters.
        Fire-and-forget — capability denial drops silently with a host log
        line."""
        if mode not in ("replace", "append"):
            raise ValueError(
                f"insert_path_token: mode must be 'replace' or 'append', got {mode!r}"
            )
        _emit({
            "type": "insert_path_token",
            "terminal_pane_id": int(terminal_pane_id),
            "path": path,
            "mode": mode,
        })

    async def request_command_preview(
        self,
        terminal_pane_id: int,
        command: str,
    ) -> "tuple[str, str]":
        """Return `(command, would_run_in_cwd)` for `command` in the linked
        terminal. Doesn't execute. Useful for confirmation modals before
        destructive operations.

        If the host denies the request, `would_run_in_cwd` is empty string;
        the SDK does NOT raise — apps that want to be loud should check for
        empty cwd themselves (the most common failure mode is a missing
        capability, which the host already logs).

        Await this from async hooks. From background threads use
        ``self.emit.run_sync(self.emit.request_command_preview(...))``.
        """
        req_id = str(uuid.uuid4())
        q: asyncio.Queue[tuple[str, str]] = _make_async_queue()
        self._app._pending_command_preview[req_id] = q
        _emit({
            "type": "request_command_preview",
            "request_id": req_id,
            "terminal_pane_id": int(terminal_pane_id),
            "command": command,
        })
        return await q.get()

    def open_artifact(
        self,
        path: str,
        mode: str = "open_in_pane",
    ) -> None:
        """Open a workspace artifact via the host.

        Modes:
          - `"open_in_pane"`        — directories open the file browser
                                      next to the app; files open with
                                      the OS default app (v3.5).
          - `"reveal_in_finder"`    — `open -R <path>` on macOS.
          - `"open_with_default"`   — `open <path>` on macOS.

        Fire-and-forget. Capability denial drops silently with a host log
        line."""
        if mode not in ("open_in_pane", "reveal_in_finder", "open_with_default"):
            raise ValueError(
                f"open_artifact: mode must be one of "
                f"open_in_pane / reveal_in_finder / open_with_default, "
                f"got {mode!r}"
            )
        _emit({"type": "open_artifact", "path": path, "mode": mode})

    def schedule_render(self, after_ms: int = 16) -> None:
        """Ask the host to send a new Render event after `after_ms` milliseconds.
        Call at the end of on_render to sustain a game/animation loop.
        16 ms ≈ 60 fps.  32 ms ≈ 30 fps."""
        _emit({"type": "schedule_render", "after_ms": after_ms})

    # Runs
    def run_get(self, intent: str, payload: Any = None) -> str:
        import uuid
        run_id = str(uuid.uuid4())
        _emit({"type": "run_get", "intent": intent,
               "payload": payload or {}})
        return run_id

    # ── Async blocking helpers ────────────────────────────────────────────────
    # Each of these is a coroutine — await them from async hooks, or call
    # self.emit.run_sync(self.emit.method(...)) from a background thread.

    async def capability_request(self, capability: str) -> bool:
        """Await until host grants or denies the capability. Returns True if granted.

        Await from async hooks. From background threads:
        ``self.emit.run_sync(self.emit.capability_request(capability))``.
        """
        req_id = str(uuid.uuid4())
        q: asyncio.Queue[bool] = _make_async_queue()
        self._app._pending_capability[req_id] = q
        _emit({"type": "capability_request", "request_id": req_id,
               "capability": capability})
        return await q.get()

    async def secret_get(self, key: str) -> "str | None":
        """Await until host returns the secret value (or None if denied).

        From background threads:
        ``self.emit.run_sync(self.emit.secret_get(key))``.
        """
        q: asyncio.Queue[str | None] = _make_async_queue()
        self._app._pending_secret[key] = q
        _emit({"type": "secret_get", "key": key})
        return await q.get()

    async def get_secret(self, key: str) -> "str | None":
        """Alias for secret_get(). Preferred name going forward."""
        return await self.secret_get(key)

    async def http_get(self, url: str) -> str:
        """HTTP GET brokered through the host. Requires net.http capability.
        Raises RuntimeError on failure.

        From background threads:
        ``self.emit.run_sync(self.emit.http_get(url))``.
        """
        return await self.http_request(url)

    async def http_request(self, url: str, method: str = "GET",
                           headers: "dict[str, str] | None" = None,
                           body: "str | None" = None) -> str:
        """HTTP request brokered through the host. Requires net.http capability.
        Supports custom method, headers, and body. Raises RuntimeError on failure.

        From background threads:
        ``self.emit.run_sync(self.emit.http_request(...))``.
        """
        req_id = str(uuid.uuid4())
        q: asyncio.Queue[tuple[str, str]] = _make_async_queue()
        self._app._pending_http[req_id] = q
        payload: dict = {"type": "http_request", "request_id": req_id,
                         "method": method, "url": url}
        if headers:
            payload["headers"] = headers
        if body is not None:
            payload["body"] = body
        _emit(payload)
        status, value = await q.get()
        if status == "error":
            raise RuntimeError(f"http_request {url!r}: {value}")
        return value

    async def ai_query(self, model_tier: str, system: str,
                       messages: "list[dict]",
                       tools: "list[dict] | None" = None) -> AiResponse:
        """Call into the host's Plexi AI broker (#284). Requires the
        `ai.query` capability declared in `manifest.toml`.

        Args:
            model_tier: one of "low" | "medium" | "high" — the host maps these
                to Haiku / Sonnet / Opus respectively.
            system: system prompt (may be empty string).
            messages: list of `{"role": "user"|"assistant", "content": str}`
                dicts. At least one message is required.
            tools: reserved for v3.4 tool-use; pass `None` or `[]` today.

        Returns:
            AiResponse with `content`, `tokens_in`, `tokens_out`.

        Raises:
            CapabilityDeniedError: app didn't declare `ai.query` in its manifest
                (or the host returned any other "capability denied" error).
            RuntimeError: backend failed (e.g. missing API key, network error).

        From background threads:
        ``self.emit.run_sync(self.emit.ai_query(...))``.
        """
        if model_tier not in ("low", "medium", "high"):
            raise ValueError(
                f"ai_query: model_tier must be one of low|medium|high, got {model_tier!r}"
            )
        req_id = str(uuid.uuid4())
        q: asyncio.Queue[dict] = _make_async_queue()
        self._app._pending_ai[req_id] = q
        payload: dict = {
            "type": "ai_query",
            "request_id": req_id,
            "model_tier": model_tier,
            "system": system,
            "messages": messages,
            "tools": tools or [],
        }
        _emit(payload)
        try:
            ev = await asyncio.wait_for(q.get(), timeout=35.0)
        except asyncio.TimeoutError:
            self._app._pending_ai.pop(req_id, None)
            raise RuntimeError(
                f"ai_query timed out after 35s (req_id={req_id!r}) — "
                "check OPENROUTER_API_KEY and network connectivity"
            )
        error = ev.get("error")
        if error is not None:
            if "capability denied" in error:
                raise CapabilityDeniedError(error)
            raise RuntimeError(f"ai_query failed: {error}")
        return AiResponse(
            content=ev.get("content") or "",
            tokens_in=int(ev.get("tokens_in", 0) or 0),
            tokens_out=int(ev.get("tokens_out", 0) or 0),
        )

    async def list_midi_devices(self, timeout: float = 5.0) -> "MidiDeviceList":
        """Enumerate CoreMIDI input + output ports (#320).
        No capability gate — port names are publicly visible in Audio MIDI
        Setup. Requires `midi.in` / `midi.out` only on subsequent open/send.

        Returns a MidiDeviceList with `inputs` and `outputs` lists of
        MidiPortInfo (id, name, default).

        Raises RuntimeError on host enumeration failure or timeout.

        From background threads:
        ``self.emit.run_sync(self.emit.list_midi_devices())``.
        """
        req_id = str(uuid.uuid4())
        q: asyncio.Queue[dict] = _make_async_queue()
        self._app._pending_midi_devices[req_id] = q
        _emit({"type": "list_midi_devices", "request_id": req_id})
        try:
            ev = await asyncio.wait_for(q.get(), timeout=timeout)
        except asyncio.TimeoutError:
            self._app._pending_midi_devices.pop(req_id, None)
            raise RuntimeError(
                f"list_midi_devices timed out after {timeout}s — host did not respond"
            )
        if ev.get("error"):
            raise RuntimeError(f"list_midi_devices failed: {ev.get('error')}")
        return MidiDeviceList(
            inputs=[
                MidiPortInfo(
                    id=str(p.get("id", "")),
                    name=str(p.get("name", "")),
                    default=bool(p.get("default", False)),
                )
                for p in ev.get("inputs", [])
            ],
            outputs=[
                MidiPortInfo(
                    id=str(p.get("id", "")),
                    name=str(p.get("name", "")),
                    default=bool(p.get("default", False)),
                )
                for p in ev.get("outputs", [])
            ],
        )

    def open_midi_input(self, port_id: str, pipe_id: str) -> "Pipe":
        """Open a CoreMIDI input port and pipe its MIDI byte streams into a
        binary pipe. Requires `midi.in` capability declared in manifest.toml.

        Returns the Pipe handle. The host emits `PipeOpened` then
        `MidiInputOpened`; the Pipe auto-connects to its socket on
        `PipeOpened`. Apps then call `pipe.read_frame()` in a loop to
        receive 1–3 byte MIDI messages (channel-voice / system real-time).

        Each pipe frame is one MIDI 1.0 message — apps don't need to parse
        message boundaries themselves.
        """
        # Open the binary pipe FIRST so the App's `_pipes` registry has it
        # before PipeOpened arrives. The host emits PipeOpened then
        # MidiInputOpened in order; both events are received by the event
        # loop and forwarded to the Pipe / on_midi_input_opened handler.
        p = Pipe(pipe_id=pipe_id, mode="binary", direction="in", app=self._app)
        self._app._pipes[pipe_id] = p
        _emit({
            "type": "open_midi_input",
            "port_id": port_id,
            "pipe_id": pipe_id,
        })
        return p

    def close_midi_input(self, port_id: str) -> None:
        """Close a previously-opened MIDI input port. The associated binary
        pipe drains and closes; the app should drop its Pipe handle."""
        _emit({"type": "close_midi_input", "port_id": port_id})

    def send_midi(self, port_id: str, bytes_: "bytes | bytearray | list[int]") -> None:
        """Fire-and-forget send of one MIDI 1.0 byte stream to `port_id`.
        Requires `midi.out` capability. The host opens the output port lazily
        on the first send and reuses the handle afterwards.

        `bytes_` is a 1–3 byte sequence: NoteOn `[0x90+ch, note, vel]`,
        NoteOff `[0x80+ch, note, vel]`, CC `[0xB0+ch, num, val]`, clock
        pulse `[0xF8]`, etc. Use `plexi_sdk.midi` helpers to construct
        messages with named arguments.

        Successful sends produce no event; failures arrive as a logged
        `midi_send_error` warning. Apps that need explicit error handling
        should validate the destination via `list_midi_devices()` first.
        """
        if isinstance(bytes_, (bytes, bytearray)):
            data = list(bytes_)
        else:
            data = list(bytes_)
        # JSON wire shape: array of integers. The host accepts any ints in
        # 0..=255 and rejects empty arrays.
        _emit({"type": "send_midi", "port_id": port_id, "bytes": data})

    async def open_video(
        self,
        source: str,
        pipe_id: str,
        timeout: float = 10.0,
    ) -> "VideoHandle":
        """Open a video decoder against `source` and return a VideoHandle
        carrying the negotiated width/height/fps/duration_ms plus the
        attached Pipe (#345). Requires `video.playback` capability.

        Decoded frames flow as raw RGBA8 packets on the Pipe — one packet
        per video frame, length `width * height * 4`. Apps spin a reader
        thread that calls `handle.pipe.read_frame()` and either renders
        the bytes (if a frame-render API is available) or surfaces a
        frame counter.

        Source schemes:
          - `mock://gradient` — procedural mock decoder (always works).
          - `file:///...` — production decoder; returns NotImplemented
            until #346 lands AVFoundation backing.

        Raises CapabilityDeniedError if `video.playback` is not declared.
        Raises RuntimeError on any other failure (NotImplemented, bad
        source, decoder error, timeout).

        From background threads:
        ``self.emit.run_sync(self.emit.open_video(source, pipe_id))``.
        """
        # Open the binary pipe FIRST so the App's `_pipes` registry has it
        # before PipeOpened arrives. The host emits PipeOpened, then
        # VideoOpenAck. The Pipe auto-connects to its socket on PipeOpened.
        pipe = Pipe(pipe_id=pipe_id, mode="binary", direction="in", app=self._app)
        self._app._pipes[pipe_id] = pipe

        req_id = str(uuid.uuid4())
        q: asyncio.Queue[dict] = _make_async_queue()
        self._app._pending_video_open[req_id] = q
        _emit({
            "type": "open_video",
            "request_id": req_id,
            "source": source,
            "pipe_id": pipe_id,
        })
        try:
            ev = await asyncio.wait_for(q.get(), timeout=timeout)
        except asyncio.TimeoutError:
            self._app._pending_video_open.pop(req_id, None)
            raise RuntimeError(
                f"open_video timed out after {timeout}s — host did not respond"
            )
        error = ev.get("error")
        if error is not None:
            if "capability denied" in error:
                raise CapabilityDeniedError(error)
            raise RuntimeError(f"open_video failed: {error}")
        return VideoHandle(
            handle_id=int(ev.get("handle_id", 0)),
            width=int(ev.get("width", 0)),
            height=int(ev.get("height", 0)),
            fps=float(ev.get("fps", 0.0)),
            duration_ms=int(ev.get("duration_ms", 0)),
            pipe=pipe,
        )

    def set_video_state(self, handle_id: int, state: str, position_ms: int = 0) -> None:
        """Drive playback for a video opened with `open_video` (#345).
        `state` is one of `"play"`, `"pause"`, `"seek"`. For `"seek"`,
        `position_ms` is the absolute target position in milliseconds.
        Fire-and-forget — no response event."""
        if state == "seek":
            payload: dict[str, Any] = {"kind": "seek", "position_ms": int(position_ms)}
        elif state == "play":
            payload = {"kind": "play"}
        elif state == "pause":
            payload = {"kind": "pause"}
        else:
            raise ValueError(
                f"set_video_state: unknown state {state!r} (expected play|pause|seek)"
            )
        _emit({
            "type": "set_video_state",
            "handle_id": int(handle_id),
            "state": payload,
        })

    def close_video(self, handle_id: int) -> None:
        """Close a previously-opened video handle (#345). The host tears down
        the decoder and drains the binary pipe. No response event."""
        _emit({"type": "close_video", "handle_id": int(handle_id)})

    def audio_capture(
        self,
        pipe_id: str,
        sample_rate: int = 48000,
        buffer_size: int = 512,
        device_id: "str | None" = None,
    ) -> "Pipe":
        """Start mic capture (#277). PCM frames stream as raw f32 PCM on
        a binary pipe — interleaved per-channel, ``sample_rate`` per
        channel per second.

        Returns the binary Pipe immediately (negotiated values arrive
        async via ``PlexiEvent::AudioCaptureStarted`` and the host log;
        the app reads ``handle.read_frame()`` once it connects). Failure
        arrives as ``PlexiEvent::AudioCaptureError``.

        Requires ``audio.in`` capability. Raises ``CapabilityDeniedError``
        only when the gate fires synchronously at the wire layer; the
        usual TCC mic-permission denial surfaces async on the first frame
        attempt as an ``AudioCaptureError`` event.
        """
        # Open the binary pipe FIRST so PipeOpened can attach when it
        # arrives — same shape as ``open_video``.
        pipe = Pipe(pipe_id=pipe_id, mode="binary", direction="in", app=self._app)
        self._app._pipes[pipe_id] = pipe
        _emit({
            "type": "audio_capture",
            "pipe_id": pipe_id,
            "device_id": device_id,
            "sample_rate": int(sample_rate),
            "buffer_size": int(buffer_size),
        })
        return pipe

    def set_timer(self, timer_id: str, after_ms: int) -> None:
        """Fire PlexiEvent::Timer after after_ms milliseconds. Requires timer capability."""
        _emit({"type": "set_timer", "timer_id": timer_id, "after_ms": after_ms})

    def cancel_timer(self, timer_id: str) -> None:
        """Cancel a pending timer set with set_timer()."""
        _emit({"type": "cancel_timer", "timer_id": timer_id})

    # Binary pipe
    def pipe_open(self, pipe_id: str, mode: str = "binary",
                  direction: str = "in") -> "Pipe":
        """Open a typed pipe. Returns a Pipe handle. mode: json|binary. direction: in|out|duplex."""
        p = Pipe(pipe_id=pipe_id, mode=mode, direction=direction, app=self._app)
        self._app._pipes[pipe_id] = p
        _emit({"type": "pipe_open", "pipe_id": pipe_id,
               "mode": mode, "direction": direction})
        return p

    def pipe_open_directed(self, pipe_id: str, target_pane_id: int) -> "Pipe":
        """Open a *directed* JSON pipe to one specific target pane (#286).

        Mirrors `pipe_open` but the host scopes `PipeMessage` delivery so
        only the caller and `target_pane_id` see traffic on `pipe_id` —
        peers that coincidentally `pipe_open` the same id stay isolated.
        Always JSON-mode duplex; `pipe.send(payload)` is symmetric on
        either end. Used to wire inter-agent channels after discovering
        a target via `emit.agent_roster()`.

        Capability: `pipe.open` (same as `pipe_open`). The target pane
        does NOT need `agents.list` to be subscribed — only to be
        addressable via the roster.
        """
        p = Pipe(pipe_id=pipe_id, mode="json", direction="duplex", app=self._app)
        self._app._pipes[pipe_id] = p
        _emit({
            "type": "pipe_open_directed",
            "pipe_id": pipe_id,
            "target_pane_id": int(target_pane_id),
        })
        return p

    def pipe_send(self, pipe_id: str, payload: Any) -> None:
        """Send a JSON payload on an already-open pipe by id (#286).

        Use this when you don't own a `Pipe` handle locally — for example
        when replying on a directed pipe a *peer* opened (you're the target;
        the host subscribed you, but the SDK doesn't auto-build a handle).
        Sends a `pipe_send` DrawCommand; the host routes per the existing
        directed-pipe pair table.
        """
        _emit({"type": "pipe_send", "pipe_id": pipe_id, "payload": payload})

    async def agent_roster(self) -> "list[AgentInfo]":
        """Query for live agent panes in the workspace (#286).

        Returns a list of `AgentInfo` rows sorted by `pane_id` ascending.
        Each row carries `pane_id`, `app_id`, and `name`. Pass the
        `pane_id` back to `pipe_open_directed(...)` to address an
        inter-agent pipe.

        Capability: `agents.list`. Apps without it receive an EMPTY
        list (NOT an error) — the host returns an empty roster on the
        wire so the caller can't probe the workspace via this surface.

        From background threads:
        ``self.emit.run_sync(self.emit.agent_roster())``.
        """
        req_id = str(uuid.uuid4())
        q: asyncio.Queue[list[dict]] = _make_async_queue()
        self._app._pending_agent_roster[req_id] = q
        _emit({"type": "agent_roster_get", "request_id": req_id})
        rows = await q.get()
        return [
            AgentInfo(
                pane_id=int(r.get("pane_id", 0)),
                app_id=str(r.get("app_id", "")),
                name=str(r.get("name", "")),
            )
            for r in rows
        ]


# ── Pipe ──────────────────────────────────────────────────────────────────────

class Pipe:
    """Handle for a typed pipe. For binary mode, call connect() after PipeOpened."""

    def __init__(self, pipe_id: str, mode: str, direction: str, app: "App"):
        self.pipe_id = pipe_id
        self.mode = mode
        self.direction = direction
        self._app = app
        self._sock: socket.socket | None = None
        self._connected = threading.Event()

    def _on_opened(self, socket_path: str) -> None:
        """Called by App when PipeOpened arrives for this pipe_id."""
        if self.mode == "binary":
            try:
                sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                sock.connect(socket_path)
                self._sock = sock
                self._connected.set()
            except OSError as e:
                self._app.emit.error(f"pipe_open failed pipe_id={self.pipe_id}: {e}")

    def connect(self, timeout: float = 5.0) -> bool:
        """Wait for the pipe to be connected. Returns True on success."""
        return self._connected.wait(timeout=timeout)

    def read_frame(self) -> bytes | None:
        """Read one length-prefixed frame. Blocks. Returns None on EOF/error."""
        if not self._sock:
            return None
        try:
            header = self._recv_exact(4)
            if header is None:
                return None
            length = struct.unpack(">I", header)[0]
            return self._recv_exact(length)
        except OSError as e:
            self._app.emit.error(f"pipe read_frame error pipe_id={self.pipe_id}: {e}")
            return None

    def write_frame(self, data: bytes) -> None:
        """Write one length-prefixed frame."""
        if not self._sock:
            return
        try:
            header = struct.pack(">I", len(data))
            self._sock.sendall(header + data)
        except OSError as e:
            self._app.emit.error(f"pipe write_frame error pipe_id={self.pipe_id}: {e}")

    def send(self, payload: Any) -> None:
        """JSON-mode pipe send."""
        _emit({"type": "pipe_send", "pipe_id": self.pipe_id, "payload": payload})

    def _recv_exact(self, n: int) -> bytes | None:
        buf = b""
        while len(buf) < n:
            chunk = self._sock.recv(n - len(buf))  # type: ignore[union-attr]
            if not chunk:
                return None
            buf += chunk
        return buf

    def close(self) -> None:
        if self._sock:
            try:
                self._sock.close()
            except OSError:
                pass
            self._sock = None


# ── RenderContext ──────────────────────────────────────────────────────────────

class RenderContext:
    """Passed to on_render. Accumulate draw commands; FrameDone is auto-emitted."""

    def __init__(self, frame_id: int, rect: dict, workspace_root: str,
                 capabilities: list[str], feature_flags: list[str], app: "App",
                 elapsed: float = 0.0):
        self.frame_id = frame_id
        self.x: float = rect.get("x", 0.0)
        self.y: float = rect.get("y", 0.0)
        self.w: float = rect.get("w", 800.0)
        self.h: float = rect.get("h", 600.0)
        self.workspace_root = workspace_root
        self.capabilities = capabilities
        self.feature_flags = feature_flags
        self._app = app
        self.emit = app.emit
        # Seconds elapsed since the previous on_render call. 0.0 on first frame.
        # Use this for time-based game logic instead of calling time.time() yourself.
        self.elapsed: float = elapsed

    # ── Visual primitives ──
    def clear(self, fill: str) -> None:
        """Fill the entire pane with a single color. Shorthand for a full-size rect."""
        _emit({"type": "rect", "x": 0.0, "y": 0.0, "w": self.w, "h": self.h,
               "fill": fill, "radius": 0.0})

    def rect(self, x: float, y: float, w: float, h: float, fill: str,
             radius: float = 0.0) -> None:
        _emit({"type": "rect", "x": x, "y": y, "w": w, "h": h,
               "fill": fill, "radius": radius})

    def circle(self, cx: float, cy: float, r: float, fill: str) -> None:
        """Draw a filled circle. Alpha supported via 8-digit hex (#rrggbbaa) or dim()."""
        _emit({"type": "circle", "cx": cx, "cy": cy, "r": r, "fill": fill})

    def arc(self, cx: float, cy: float, r: float,
            start_angle: float, end_angle: float, fill: str) -> None:
        """Draw a filled pie slice. Angles in radians, clockwise from east (right).
        Full circle: start_angle=0, end_angle=6.2832 (2*pi).
        Example pie slice: arc(cx, cy, r, 0, math.pi * 0.5, fill="#ff0000")"""
        _emit({"type": "arc", "cx": cx, "cy": cy, "r": r,
               "start_angle": start_angle, "end_angle": end_angle, "fill": fill})

    def text(self, x: float, y: float, text: str, size: float, color: str,
             monospace: bool = False, bold: bool = False,
             align: str = "top_left",
             max_width: "float | None" = None,
             elide: bool = True,
             selectable: bool = False) -> None:
        """Draw text. `align` controls how `(x, y)` maps to the text box:

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
        """
        _emit({"type": "text", "x": x, "y": y, "text": text, "size": size,
               "color": color, "monospace": monospace, "bold": bold,
               "align": align, "max_width": max_width, "elide": elide,
               "selectable": selectable})

    def copy_to_clipboard(self, text: str) -> None:
        """Convenience shortcut for `emit.copy_to_clipboard(text)` (#146)."""
        _emit({"type": "copy_to_clipboard", "text": text})

    def badge(self, x: float, y_center: float, label: str,
              fill: str = ACCENT, fg: str = BG,
              font_size: float = 11.0, radius: float = 8.0) -> None:
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
        _emit({"type": "badge", "x": x, "y": y_center, "label": label,
               "fill": fill, "fg": fg, "font_size": font_size, "radius": radius})

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
        _emit({"type": "key_chip_row", "x": x, "y": y, "keys": keys,
               "description": description, "font_size": font_size})

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
        _emit({"type": "shortcuts", "x": x, "y": y,
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
        request_id = str(uuid.uuid4())
        q: asyncio.Queue[tuple[float, float]] = _make_async_queue()
        self._app._pending_measure_text[request_id] = q
        _emit({"type": "measure_text", "request_id": request_id,
               "text": text, "font_size": font_size, "monospace": monospace})
        return await q.get()

    def push_clip(self, x: float, y: float, w: float, h: float) -> None:
        """Push a clip rect onto the host's clip stack.

        All subsequent draws are clipped to the intersection of this rect with
        the current top of the stack (or the pane rect if the stack is empty).
        Must be balanced with a matching `pop_clip()`. Use `_render_clipped`
        on Component subclasses instead of calling this directly.
        """
        _emit({"type": "push_clip", "x": x, "y": y, "w": w, "h": h})

    def pop_clip(self) -> None:
        """Pop the most recently pushed clip rect. Must balance a `push_clip()`."""
        _emit({"type": "pop_clip"})

    def line(self, x1: float, y1: float, x2: float, y2: float,
             color: str, width: float = 1.0) -> None:
        _emit({"type": "line", "x1": x1, "y1": y1, "x2": x2, "y2": y2,
               "color": color, "width": width})

    # ── SDK v2 declarative UI entry point ──
    def render(self, tree, fill: str = "#1e1e2e") -> None:
        """Render a declarative UI tree (see `plexi_sdk.ui`). Clears the pane
        to `fill` first, then lays out `tree` into the full pane rect.

        Example:
            from plexi_sdk.ui import Column, Header, Footer, Spacer
            ctx.render(Column([
                Header("My App"),
                Spacer(grow=True),
                Footer("status line"),
            ]))
        """
        # Local import avoids a circular dependency at module load time
        # (ui.py references RenderContext only through duck-typed `ctx`).
        from plexi_sdk.ui import render_tree
        render_tree(self, tree, fill=fill)

    def list(self, items: list[dict], selected: int = 0,
             item_height: float = 40.0, x: float = 0.0, y: float = 0.0,
             w: float | None = None, h: float | None = None) -> None:
        _emit({"type": "list", "x": x, "y": y,
               "w": self.w if w is None else w,
               "h": self.h - y if h is None else h,
               "items": items, "selected": selected,
               "item_height": item_height})

    def text_input(self, id: str, x: float, y: float, w: float,
                   placeholder: str = "",
                   multiline: bool = False) -> "str | None":
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

        Real-time validation (per-keystroke access) is out of scope —
        see issue #283.
        """
        _emit({"type": "text_input", "id": id, "x": x, "y": y, "w": w,
               "placeholder": placeholder, "multiline": multiline})
        return self._app._take_text_submission(id)

    # Logging helpers (in-frame, forwarded to host logger)
    def log(self, level: str, message: str) -> None:
        _emit({"type": "log", "level": level, "message": message})

    def info(self, message: str) -> None:  self.log("info", message)
    def warn(self, message: str) -> None:  self.log("warn", message)
    def error(self, message: str) -> None: self.log("error", message)
    def debug(self, message: str) -> None: self.log("debug", message)

    def notify(self, title: str, body: str = "", level: str = "info",
               priority: int | None = None,
               actions: list | None = None,
               image_inline: dict | None = None,
               image_pipe_id: str | None = None) -> None:
        """Post a message notification. See Emitter.notify.
        `priority` is required (int, higher = more urgent).
        `image_inline` / `image_pipe_id` (#74) optionally attach an image.
        Scope is resolved from the app's manifest — not an argument."""
        self.emit.notify(title=title, body=body, level=level,
                         priority=priority, actions=actions,
                         image_inline=image_inline, image_pipe_id=image_pipe_id)

    async def notify_choice(self, title: str, options: list, body: str = "",
                            level: str = "info", required: bool = False,
                            priority: int | None = None,
                            image_inline: dict | None = None,
                            image_pipe_id: str | None = None) -> str:
        """Post a choice notification and await until the user picks.
        `priority` is required (int, higher = more urgent).
        `image_inline` / `image_pipe_id` (#74) optionally attach an image.
        Returns the chosen option's value (or label if no value set),
        or "__cancel__" if the user dismissed. Use with ``await``."""
        return await self.emit.notify_choice(title=title, options=options, body=body,
                                             level=level, required=required,
                                             priority=priority,
                                             image_inline=image_inline,
                                             image_pipe_id=image_pipe_id)

    async def notify_with_image(self, title: str, body: str, image_bytes: bytes,
                                mime: str, level: str = "info",
                                priority: int | None = None,
                                choices: list | None = None) -> "str | None":
        """Convenience: post a notification with an inline base64 image.
        See Emitter.notify_with_image — handles base64 + 50KB cap. Use with ``await``."""
        return await self.emit.notify_with_image(title=title, body=body,
                                                 image_bytes=image_bytes, mime=mime,
                                                 level=level, priority=priority,
                                                 choices=choices)

    async def notify_input(self, title: str, prompt: str = "", body: str = "",
                           level: str = "info", required: bool = False,
                           priority: int | None = None) -> str:
        """Post an input notification and await until the user submits.
        `priority` is required (int, higher = more urgent).
        Returns the typed text, or "__cancel__" if dismissed. Use with ``await``."""
        return await self.emit.notify_input(title=title, prompt=prompt, body=body,
                                            level=level, required=required,
                                            priority=priority)

    async def notify_and_wait(self, title: str, body: str = "", level: str = "info",
                              actions: list | None = None,
                              priority: int | None = None) -> str:
        """Post a message notification and await for acknowledge/cancel.
        `priority` is required (int, higher = more urgent).
        See Emitter.notify_and_wait. Use with ``await``."""
        return await self.emit.notify_and_wait(title=title, body=body, level=level,
                                               actions=actions, priority=priority)

    def status_summary(self, text: str) -> None:
        _emit({"type": "status_summary", "text": text})

    def set_timer(self, timer_id: str, after_ms: int) -> None:
        self.emit.set_timer(timer_id, after_ms)

    def cancel_timer(self, timer_id: str) -> None:
        self.emit.cancel_timer(timer_id)

    def schedule_render(self, after_ms: int = 16) -> None:
        """Ask the host to send a new Render event after `after_ms` milliseconds.
        Delegates to emit.schedule_render(). 16 ms ≈ 60 fps. 32 ms ≈ 30 fps."""
        self.emit.schedule_render(after_ms=after_ms)

    async def get_secret(self, key: str) -> "str | None":
        """Request a secret by key. Alias for emit.get_secret(). Use with ``await``."""
        return await self.emit.get_secret(key)

    async def http_request(self, url: str, method: str = "GET",
                           headers: "dict[str, str] | None" = None,
                           body: "str | None" = None) -> str:
        """HTTP request brokered through the host. Requires net.http capability. Use with ``await``."""
        return await self.emit.http_request(url, method=method, headers=headers, body=body)

    async def ai_query(self, model_tier: str, system: str,
                       messages: "list[dict]",
                       tools: "list[dict] | None" = None) -> AiResponse:
        """AI broker call through the host. Requires ai.query capability. Use with ``await``."""
        return await self.emit.ai_query(model_tier=model_tier, system=system,
                                        messages=messages, tools=tools)

    def frame_done(self) -> None:
        _emit({"type": "frame_done", "frame_id": self.frame_id})


# ── App base class ────────────────────────────────────────────────────────────

class App:
    """
    Base class for Plexi v3 apps. Subclass and override event handlers.

    Override any of:
        on_init(self, ctx)                            — after Init handshake
        on_render(self, ctx)                          — on each Render event
        on_key(self, ctx, key, mods)                  — on Key event
        on_click(self, ctx, x, y, button)             — on Click event
        on_command(self, ctx, text)                   — on Command event
        on_paste(self, ctx, text)                     — on Paste event (Cmd+V)
        on_pipe_message(self, ctx, pipe_id, payload)  — on PipeMessage (JSON mode)
        on_path_changed(self, ctx, cwd)               — on PathChanged broadcast
        on_suspend(self)                              — on Suspend
        on_resume(self)                               — on Resume
        on_shutdown(self)                             — on Shutdown (before exit)
    """

    def __init__(self) -> None:
        self.app_id: str = ""
        self.workspace_root: str = ""
        self.capabilities: list[str] = []
        self.feature_flags: list[str] = []
        self._rect: dict = {"x": 0.0, "y": 0.0, "w": 800.0, "h": 600.0}
        # The running asyncio event loop. Set by run() before hooks are called.
        # Background threads use this via emit.run_sync() to schedule coroutines.
        self._loop: "asyncio.AbstractEventLoop | None" = None
        # All pending-response maps now hold asyncio.Queue so the event loop
        # coroutine can await them without blocking the stdin reader.
        self._pending_capability: "dict[str, asyncio.Queue]" = {}
        self._pending_secret: "dict[str, asyncio.Queue]" = {}
        self._pending_http: "dict[str, asyncio.Queue]" = {}
        # v3.3 ai.query broker (#284): awaits PlexiEvent::AiResponse keyed
        # on request_id. Each entry is consumed by a single ai_query() call.
        self._pending_ai: "dict[str, asyncio.Queue]" = {}
        # v3.4 CoreMIDI (#320): awaits PlexiEvent::MidiDevicesListed keyed
        # on request_id. Each entry is consumed by a single list_midi_devices().
        self._pending_midi_devices: "dict[str, asyncio.Queue]" = {}
        # v3.4 video substrate (#345): awaits PlexiEvent::VideoOpenAck /
        # VideoOpenError keyed on request_id. Each entry is consumed by a
        # single open_video() call.
        self._pending_video_open: "dict[str, asyncio.Queue]" = {}
        # v3.3 P2 agents.list (#286): awaits PlexiEvent::AgentRoster keyed
        # on request_id. Each entry is consumed by a single agent_roster() call.
        self._pending_agent_roster: "dict[str, asyncio.Queue]" = {}
        self._pending_notify: "dict[str, asyncio.Queue]" = {}
        # v3.5 Canvas Terminal Binding Primitives (#78). Two response shapes:
        # `linked_terminal_ready` carries an int pane_id; `command_preview`
        # carries (command, would_run_in_cwd). Each async helper awaits
        # its own keyed queue.
        self._pending_linked_terminal: "dict[str, asyncio.Queue]" = {}
        self._pending_command_preview: "dict[str, asyncio.Queue]" = {}
        # RenderContext.measure_text: awaits PlexiEvent::TextMeasured keyed on request_id.
        self._pending_measure_text: "dict[str, asyncio.Queue]" = {}
        self._pipes: dict[str, Pipe] = {}
        self._last_render_time: float | None = None
        # Pending text-input submissions keyed on TextInput `id`. The
        # event-loop coroutine fills this when `PlexiEvent::TextSubmitted`
        # arrives; `RenderContext.text_input` drains it during render.
        # One pending value per id — a second submit before the app
        # consumes the first overwrites (apps poll every frame, so
        # this only matters in a perverse scheduling case).
        self._text_submissions: dict[str, str] = {}
        self.emit = Emitter(self)

    # ── Override these ──────────────────────────────────────────────────────
    # All hooks may be overridden as either `def` (sync) or `async def`.
    # _dispatch_hook detects the type at call time — both are valid.
    # Return type is `Coroutine[Any, Any, None] | None` so Pyright accepts
    # both sync (`def` → returns None) and async (`async def` → returns
    # Coroutine) overrides without reportIncompatibleMethodOverride.
    def on_init(self, _ctx: RenderContext) -> Coroutine[Any, Any, None] | None: return None
    def on_render(self, _ctx: RenderContext) -> None: pass
    def on_key(self, _ctx: RenderContext, _key: str, _mods: dict) -> Coroutine[Any, Any, None] | None: return None
    def on_click(self, _ctx: RenderContext, _x: float, _y: float, _button: str) -> Coroutine[Any, Any, None] | None: return None
    def on_command(self, _ctx: RenderContext, _text: str) -> Coroutine[Any, Any, None] | None: return None
    def on_paste(self, _ctx: RenderContext, _text: str) -> Coroutine[Any, Any, None] | None: return None
    def on_pipe_message(self, _ctx: RenderContext, _pipe_id: str, _payload: Any) -> Coroutine[Any, Any, None] | None: return None
    def on_path_changed(self, _ctx: RenderContext, _cwd: str) -> Coroutine[Any, Any, None] | None: return None
    def on_inject(self, _ctx: RenderContext, _payload: Any) -> Coroutine[Any, Any, None] | None: return None
    def on_app_spawned(self, _pane_id: int, _type_id: str) -> None: pass
    def on_timer(self, _ctx: RenderContext, _timer_id: str) -> Coroutine[Any, Any, None] | None: return None
    def on_midi_input_opened(
        self,
        _pipe_id: str,
        _port_id: str,
        _port_name: str,
    ) -> None:
        """Override to react to a successful OpenMidiInput. Apps that just
        want the byte stream typically read directly from the binary pipe
        opened alongside this event — Plexi sends `pipe_opened` first."""
        pass

    # ── Agent-as-app hooks (#338, type = "agent" manifests only) ────────────
    # `Agent` subclass wires these. Plain `App` subclasses get no-op defaults
    # so a misclassified manifest doesn't crash on the host's emit.
    def on_agent_init(self, _system_prompt: "str | None") -> None: pass
    def on_user_message(self, _ctx: "RenderContext", _text: str) -> None: pass
    def on_suspend(self) -> None: pass
    def on_resume(self) -> None: pass
    def on_shutdown(self) -> None: pass

    # ── Internal ────────────────────────────────────────────────────────────
    def _take_text_submission(self, id: str) -> "str | None":
        """Pop the most recent submission for `id` if one is queued, else None.

        Called by `RenderContext.text_input` to surface a buffered
        `TextSubmitted` value into the current frame's render call.
        """
        return self._text_submissions.pop(id, None)

    def _make_ctx(self, frame_id: int = 0, elapsed: float = 0.0) -> RenderContext:
        return RenderContext(
            frame_id=frame_id,
            rect=self._rect,
            workspace_root=self.workspace_root,
            capabilities=self.capabilities,
            feature_flags=self.feature_flags,
            app=self,
            elapsed=elapsed,
        )

    def run(self) -> None:
        """Start the PGAP v3 asyncio event loop. Blocks until Shutdown."""
        sys.stdout.reconfigure(line_buffering=True)  # type: ignore[attr-defined]
        asyncio.run(self._async_main())

    async def _async_main(self) -> None:
        """Asyncio entry point — two concurrent tasks to eliminate deadlocks.

        The root cause of the old single-loop design: when the dispatcher
        awaited a hook (e.g. on_init) that itself called a blocking helper
        (e.g. request_linked_terminal), the event loop had no concurrent
        stdin reader in flight. Nothing could deliver the response event
        while the hook was suspended, causing a permanent deadlock.

        Fix: split into two tasks that run concurrently on the same event loop.

          _reader  — always has a run_in_executor(readline) in flight.
                     Handles response events inline (put_nowait into pending
                     queues) so they can unblock awaiting hooks even while the
                     dispatcher is suspended.

          _dispatcher — drains a hook_q, dispatches hook events sequentially.
                        Can safely await hooks because _reader is always running
                        alongside it and will deliver response events.

        Response events MUST be handled inline in _reader — never enqueued —
        so that hooks awaiting on pending queues can be unblocked even when
        the dispatcher is suspended mid-hook.
        """
        loop = asyncio.get_running_loop()
        self._loop = loop
        hook_q: asyncio.Queue = asyncio.Queue()

        async def _reader() -> None:
            while True:
                raw = await loop.run_in_executor(None, sys.stdin.readline)
                if not raw:
                    # EOF — host closed stdin; signal dispatcher to shut down.
                    await hook_q.put({"type": "shutdown"})
                    return
                raw = raw.strip()
                if not raw:
                    continue
                try:
                    ev = json.loads(raw)
                except json.JSONDecodeError:
                    continue

                t = ev.get("type", "")

                # ── Response events: handled inline so they can unblock ──────
                # hooks suspended in the dispatcher. These must NEVER go on
                # hook_q — that would leave awaiting coroutines stuck forever.

                if t == "capability_decision":
                    req_id = ev.get("request_id", "")
                    granted = ev.get("granted", False)
                    q = self._pending_capability.pop(req_id, None)
                    if q:
                        q.put_nowait(granted)

                elif t == "secret_value":
                    key = ev.get("key", "")
                    value = ev.get("value")
                    q = self._pending_secret.pop(key, None)
                    if q:
                        q.put_nowait(value)

                elif t == "http_response":
                    req_id = ev.get("request_id", "")
                    q = self._pending_http.pop(req_id, None)
                    if q:
                        if ev.get("error"):
                            q.put_nowait(("error", ev["error"]))
                        else:
                            q.put_nowait(("ok", ev.get("body", "")))

                elif t == "ai_response":
                    # v3.3 ai.query broker (#284). Hand the whole event dict to
                    # `Emitter.ai_query` so it can split error vs success and
                    # attach token counts.
                    req_id = ev.get("request_id", "")
                    q = self._pending_ai.pop(req_id, None)
                    if q:
                        q.put_nowait(ev)
                    else:
                        import logging as _logging
                        _logging.warning(
                            f"ai_response: no pending request for req_id={req_id!r} — "
                            "response dropped (query may have timed out already)"
                        )

                elif t == "midi_devices_listed":
                    # v3.4 CoreMIDI (#320). Forward to Emitter.list_midi_devices.
                    req_id = ev.get("request_id", "")
                    q = self._pending_midi_devices.pop(req_id, None)
                    if q:
                        q.put_nowait(ev)

                elif t == "video_open_ack":
                    # v3.4 video substrate (#345). Forward to Emitter.open_video().
                    req_id = str(ev.get("request_id", ""))
                    q = self._pending_video_open.pop(req_id, None)
                    if q:
                        q.put_nowait(ev)

                elif t == "video_open_error":
                    # OpenVideo failed (capability denied, NotImplemented from the
                    # production stub, bad source). Forward the error event so
                    # `open_video()` can raise CapabilityDeniedError / RuntimeError.
                    req_id = str(ev.get("request_id", ""))
                    q = self._pending_video_open.pop(req_id, None)
                    if q:
                        q.put_nowait(ev)

                elif t == "linked_terminal_ready":
                    # v3.5 #78. Forward the terminal_pane_id (int) to the
                    # awaiting helper. 0 = capability denied — the helper
                    # raises CapabilityDeniedError when it sees that.
                    req_id = ev.get("request_id", "")
                    q = self._pending_linked_terminal.pop(req_id, None)
                    if q:
                        q.put_nowait(int(ev.get("terminal_pane_id", 0)))

                elif t == "command_preview":
                    # v3.5 #78. Forward (command, would_run_in_cwd) tuple to the
                    # awaiting helper. would_run_in_cwd is "" on capability denial.
                    req_id = ev.get("request_id", "")
                    q = self._pending_command_preview.pop(req_id, None)
                    if q:
                        q.put_nowait((
                            str(ev.get("command", "")),
                            str(ev.get("would_run_in_cwd", "")),
                        ))

                elif t == "agent_roster":
                    # v3.3 P2 agents.list (#286). The `agents` field is always
                    # a list (empty when the app lacks the `agents.list`
                    # capability — the host returns an empty roster, not an
                    # error). Forwarded as-is to the queue waiting in
                    # `Emitter.agent_roster`.
                    req_id = ev.get("request_id", "")
                    q = self._pending_agent_roster.pop(req_id, None)
                    if q:
                        q.put_nowait(ev.get("agents", []) or [])

                elif t == "notify_action":
                    # notify_choice / notify_input: put the value back.
                    # notify / notify_and_wait: put action_label back.
                    # Esc cancel: return "__cancel__" so callers can check easily.
                    notify_id = ev.get("notify_id", "")
                    action_label = ev.get("action_label", "")
                    value = ev.get("value")
                    q = self._pending_notify.pop(notify_id, None)
                    if q:
                        if action_label == "cancel":
                            q.put_nowait("__cancel__")
                        elif value is not None:
                            q.put_nowait(value)
                        else:
                            q.put_nowait(action_label or "acknowledge")

                elif t == "text_measured":
                    # Response to RenderContext.measure_text(). Forward (width, height)
                    # to the awaiting coroutine keyed on request_id.
                    req_id = ev.get("request_id", "")
                    q = self._pending_measure_text.pop(req_id, None)
                    if q:
                        q.put_nowait((
                            float(ev.get("width", 0.0)),
                            float(ev.get("height", 0.0)),
                        ))

                # ── Inline non-hook events (fast, no user code) ──────────────

                elif t == "pipe_opened":
                    pipe_id = ev.get("pipe_id", "")
                    socket_path = ev.get("socket_path", "")
                    p = self._pipes.get(pipe_id)
                    if p:
                        p._on_opened(socket_path)

                elif t == "pipe_overrun":
                    self.emit.warn(
                        f"pipe overrun pipe_id={ev.get('pipe_id')} "
                        f"dropped={ev.get('dropped_frames')}"
                    )

                elif t == "midi_input_error":
                    # OpenMidiInput failed (capability denied, port_id not found,
                    # CoreMIDI error). Apps log this; the typical recovery is to
                    # surface the error in-pane and let the user pick a different
                    # port from list_midi_devices.
                    self.emit.warn(
                        f"midi_input_error pipe_id={ev.get('pipe_id')} "
                        f"error={ev.get('error')}"
                    )

                elif t == "midi_send_error":
                    # SendMidi failed. Surfaces only on capability denial / open
                    # failure / coremidi error — successful sends produce no event.
                    self.emit.warn(
                        f"midi_send_error port_id={ev.get('port_id')} "
                        f"error={ev.get('error')}"
                    )

                elif t == "text_submitted":
                    # Host-owned text input: the user pressed Enter on a
                    # `DrawCommand::TextInput` field. Stash the value keyed
                    # on the input id; `RenderContext.text_input(...)` will
                    # drain it on the next frame the app polls.
                    tid = ev.get("id", "")
                    if tid:
                        self._text_submissions[tid] = ev.get("value", "")

                elif t == "run_update":
                    pass  # apps can override on_run_update if needed

                # ── Hook events: forwarded to the dispatcher ─────────────────
                else:
                    await hook_q.put(ev)

        async def _dispatcher() -> None:
            while True:
                ev = await hook_q.get()
                t = ev.get("type", "")

                if t == "init":
                    proto = ev.get("protocol", "")
                    if not proto.startswith("pgap/3"):
                        sys.stderr.write(
                            f"plexi_sdk: unsupported protocol {proto!r}, expected pgap/3\n"
                        )
                        sys.exit(1)
                    self.app_id = ev.get("app_id", "")
                    self.workspace_root = ev.get("workspace_root", "")
                    self.capabilities = ev.get("capabilities", [])
                    self.feature_flags = ev.get("feature_flags", [])
                    # Send Ready
                    features_used = [f for f in self.feature_flags
                                      if f in ("pane_groups_v1",)]
                    _emit({"type": "ready", "sdk": SDK_ID, "features_used": features_used})
                    await self._dispatch_hook(self.on_init, self._make_ctx())

                elif t == "render":
                    import time as _time
                    now = _time.monotonic()
                    elapsed = (now - self._last_render_time) if self._last_render_time is not None else 0.0
                    self._last_render_time = now
                    frame_id = ev.get("frame_id", 0)
                    if "rect" in ev:
                        self._rect = ev["rect"]
                    elif "width" in ev:
                        # legacy compat
                        self._rect = {"x": 0.0, "y": 0.0,
                                      "w": ev["width"], "h": ev["height"]}
                    ctx = self._make_ctx(frame_id, elapsed=elapsed)
                    try:
                        await self._dispatch_hook(self.on_render, ctx)
                    except Exception as e:
                        ctx.error(f"on_render exception: {e}")
                    ctx.frame_done()

                elif t == "key":
                    ctx = self._make_ctx()
                    await self._dispatch_hook(self.on_key, ctx, ev.get("key", ""), ev.get("modifiers", {}))

                elif t == "click":
                    ctx = self._make_ctx()
                    await self._dispatch_hook(self.on_click, ctx, ev.get("x", 0.0), ev.get("y", 0.0),
                                              ev.get("button", "primary"))

                elif t == "command":
                    ctx = self._make_ctx()
                    await self._dispatch_hook(self.on_command, ctx, ev.get("text", ""))

                elif t == "paste":
                    ctx = self._make_ctx()
                    await self._dispatch_hook(self.on_paste, ctx, ev.get("text", ""))

                elif t == "pipe_message":
                    ctx = self._make_ctx()
                    await self._dispatch_hook(self.on_pipe_message, ctx, ev.get("pipe_id", ""), ev.get("payload"))

                elif t == "path_changed":
                    ctx = self._make_ctx()
                    await self._dispatch_hook(self.on_path_changed, ctx, ev.get("cwd", ""))

                elif t == "suspend":
                    await self._dispatch_hook(self.on_suspend)

                elif t == "resume":
                    await self._dispatch_hook(self.on_resume)

                elif t == "shutdown":
                    # Cancel any pending ai_query waiters so their coroutines
                    # unblock immediately instead of waiting up to 35s for a
                    # response that will never arrive.
                    if self._pending_ai:
                        import logging as _logging
                        _logging.warning(
                            f"shutdown: cancelling {len(self._pending_ai)} in-flight "
                            f"ai_query request(s): {list(self._pending_ai.keys())}"
                        )
                        for _pending_q in self._pending_ai.values():
                            _pending_q.put_nowait(
                                {"error": "ai_query cancelled: app is shutting down"}
                            )
                        self._pending_ai.clear()
                    await self._dispatch_hook(self.on_shutdown)
                    return

                elif t == "inject_state":
                    ctx = self._make_ctx()
                    await self._dispatch_hook(self.on_inject, ctx, ev.get("payload", {}))

                elif t == "midi_input_opened":
                    # Confirms an OpenMidiInput call landed a CoreMIDI source.
                    # Apps that care about "the port is now wired to my pipe"
                    # see this event after the corresponding PipeOpened — they
                    # can override on_midi_input_opened to react.
                    try:
                        await self._dispatch_hook(
                            self.on_midi_input_opened,
                            str(ev.get("pipe_id", "")),
                            str(ev.get("port_id", "")),
                            str(ev.get("port_name", "")),
                        )
                    except Exception as e:
                        sys.stderr.write(f"on_midi_input_opened handler raised: {e}\n")

                elif t == "agent_init":
                    # v3.3 agent-as-app (#338): the host forwards the manifest's
                    # `[launch].system_prompt` once at startup. Apps that subclass
                    # `Agent` consume this in `_on_agent_init`; plain App
                    # subclasses can override `on_agent_init` to receive it.
                    try:
                        await self._dispatch_hook(self.on_agent_init, ev.get("system_prompt"))
                    except Exception as e:
                        sys.stderr.write(f"on_agent_init handler raised: {e}\n")

                elif t == "user_message":
                    # v3.3 agent-as-app (#338): the user submitted text in the
                    # host-rendered conversation input box. Forwarded to
                    # `on_user_message`. Only delivered to type=agent panes.
                    ctx = self._make_ctx()
                    try:
                        await self._dispatch_hook(self.on_user_message, ctx, ev.get("text", ""))
                    except Exception as e:
                        sys.stderr.write(f"on_user_message handler raised: {e}\n")

                elif t == "timer":
                    timer_id = ev.get("timer_id", "")
                    ctx = self._make_ctx()
                    await self._dispatch_hook(self.on_timer, ctx, timer_id)

                elif t == "app_spawned":
                    # Confirmation that a SpawnApp request succeeded. Apps that
                    # want to track the spawned pane can override on_app_spawned.
                    try:
                        await self._dispatch_hook(
                            self.on_app_spawned,
                            int(ev.get("pane_id", 0)),
                            str(ev.get("type_id", "")),
                        )
                    except Exception as e:
                        sys.stderr.write(f"on_app_spawned handler raised: {e}\n")

        reader_task = asyncio.create_task(_reader())
        try:
            await _dispatcher()
        finally:
            reader_task.cancel()
            # Ensure all pipes are closed cleanly
            for p in self._pipes.values():
                p.close()

    async def _dispatch_hook(self, hook: "Any", *args: Any) -> None:
        """Dispatch a lifecycle hook — async or sync — in the event loop.

        Async hooks are awaited directly, giving them access to blocking
        helpers via ``await self.emit.method()``.

        Sync hooks are called directly on the event loop thread. They are
        safe for pure-compute / draw-command hooks (the common case). If a
        sync hook needs to call a blocking Emitter helper, it must do so
        from a background thread via ``threading.Thread`` + ``emit.run_sync()``.
        """
        if inspect.iscoroutinefunction(hook):
            await hook(*args)
        else:
            hook(*args)


# ── Agent base class (issue #338) ────────────────────────────────────────────

class Agent(App):
    """Subclass for `type = "agent"` manifests. Wires the conversation loop.

    The host renders the conversation UI (history scrollback + input box).
    The agent owns the dialogue logic. The contract is symmetric:

        Host → Agent      PlexiEvent::AgentInit  { system_prompt }   (once)
        Host → Agent      PlexiEvent::UserMessage { text }            (per submit)
        Agent → Host      DrawCommand::AppendConversation { role, content }

    Author writes a single `on_user_message(text) -> str | None` callback.
    Returning a string auto-emits an assistant `AppendConversation`. Returning
    `None` means "I'll append manually" — useful for agents that emit
    multiple rows per turn (tool use, partial replies).

    Conversation history (`self.history`) is auto-built from `append_*`
    helpers — pass it directly to `emit.ai_query(...)` for multi-turn.

    Example:

        class JokeAgent(Agent):
            def on_user_message(self, text: str) -> str:
                resp = self.emit.ai_query(
                    model_tier="medium",
                    system=self.system_prompt or "",
                    messages=self.history,
                )
                return resp.content

        if __name__ == "__main__":
            JokeAgent().run()
    """

    def __init__(self) -> None:
        super().__init__()
        # Populated by AgentInit. None until the host emits it (or forever
        # if the manifest omits `[launch].system_prompt`).
        self.system_prompt: "str | None" = None
        # Conversation history in Anthropic Messages shape — built by the
        # `append_*` helpers and passed straight to `emit.ai_query`.
        self.history: list = []

    # ── Wire-up (do not override) ───────────────────────────────────────────
    def on_agent_init(self, system_prompt: "str | None") -> None:  # type: ignore[override]
        self.system_prompt = system_prompt
        self.emit.info(
            f"agent: AgentInit received (system_prompt={'set' if system_prompt else 'unset'})"
        )

    async def on_user_message(self, ctx: "RenderContext", text: str) -> None:  # type: ignore[override]
        # Append the user turn before invoking the override so `self.history`
        # already contains it when the override calls `emit.ai_query`.
        self.append_user_message(text)
        try:
            if inspect.iscoroutinefunction(self.respond):
                reply = await self.respond(text)  # type: ignore[misc]
            else:
                reply = self.respond(text)
        except Exception as e:
            self.emit.error(f"agent: respond() raised: {e}")
            self.append_system_message(f"Error: {e}")
            return
        # `None` means "I'll handle appends myself" — common for agents that
        # stream multiple rows (tool use, partial replies). A returned string
        # is the conventional one-shot reply path.
        if reply is not None:
            self.append_assistant_message(reply)

    # ── User override ───────────────────────────────────────────────────────
    def respond(self, _text: str) -> "str | None":
        """Override this. Called once per `user_message` event.

        May be a regular ``def`` or ``async def``. Return a string to
        auto-append as the assistant turn. Return ``None`` if you've already
        called ``append_assistant_message`` (or other ``append_*`` helpers)
        yourself — useful for tool-use loops.
        """
        raise NotImplementedError(
            "Agent subclasses must override `respond(text) -> str | None`"
        )

    # ── Conversation surface ────────────────────────────────────────────────
    def append_user_message(self, text: str) -> None:
        """Append a user row to the transcript. Updates `self.history` so the
        next `emit.ai_query` call sees the turn."""
        self.history.append({"role": "user", "content": text})
        _emit({"type": "append_conversation", "role": "user", "content": text})

    def append_assistant_message(self, text: str) -> None:
        """Append an assistant row. Mirrors into `self.history`."""
        self.history.append({"role": "assistant", "content": text})
        _emit({"type": "append_conversation", "role": "assistant", "content": text})

    def append_tool_message(self, text: str) -> None:
        """Append a tool-use status row. Tool messages are NOT mirrored into
        `self.history` (they're not part of the LLM-visible conversation)."""
        _emit({"type": "append_conversation", "role": "tool", "content": text})

    def append_system_message(self, text: str) -> None:
        """Append a system / error status row. NOT mirrored into history."""
        _emit({"type": "append_conversation", "role": "system", "content": text})

    # ── Inter-agent helpers (#286) ──────────────────────────────────────────
    async def list_agents(self) -> "list[AgentInfo]":
        """Return the workspace's live agent roster. Use with ``await``.

        Thin wrapper around ``await self.emit.agent_roster()``. Apps without
        the `agents.list` capability receive an EMPTY list (not an error).

        Pass an entry's `pane_id` to `open_pipe_to(pane_id, ...)` to start
        an inter-agent channel.
        """
        return await self.emit.agent_roster()

    def open_pipe_to(self, pane_id: int, pipe_id: "str | None" = None) -> Pipe:
        """Open a duplex JSON pipe to another agent pane (#286).

        `pipe_id` defaults to a fresh uuid4 — pass an explicit id only when
        both ends agreed on one out-of-band. The pipe is duplex; either
        side calls `pipe.send(payload)` and the other receives it via
        `on_pipe_message(ctx, pipe_id, payload)`.

        Capability: `pipe.open`.
        """
        pid = pipe_id or f"agent-{uuid.uuid4()}"
        return self.emit.pipe_open_directed(pid, int(pane_id))

    def on_pipe_message(self, ctx: RenderContext, pipe_id: str, payload: Any) -> Coroutine[Any, Any, None] | None:
        """Override to receive directed inter-agent messages.

        The default `App.on_pipe_message` is a no-op. Override on `Agent`
        subclasses that act as workers / fan-out targets to handle
        delegated requests.
        """
        # Same default as App.on_pipe_message — overridable by subclasses.
        del ctx, pipe_id, payload
        return None
