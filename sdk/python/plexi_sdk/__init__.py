"""
plexi_sdk — Plexi external app SDK (Python), PGAP v3

Spec: docs/specs/releases/plexi-v3.0.md §3 (PGAP v3), §7 (typed pipes).
Zero dependencies, pure stdlib.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
QUICK START
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

    from plexi_sdk import App, BG, FG, BODY, ACCENT

    class CounterApp(App):
        def on_init(self, ctx):
            # Called once after the host completes the Init handshake.
            # ctx.workspace_root, ctx.capabilities, ctx.feature_flags are set.
            self.count = 0
            self.emit.info("CounterApp ready")

        def on_render(self, ctx):
            # Called on every frame. Must not block — use threads for I/O.
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

__version__ = "0.4.0"
SDK_ID = f"plexi-sdk-py/{__version__}"

import json
import queue
import socket
import struct
import sys
import threading
from typing import Any

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
               actions: list | None = None) -> None:
        """Post a message notification. The modal shows title + body and a
        single Acknowledge button; Enter / Space acknowledge, Esc dismisses
        (unless required=True — use `notify_and_wait` for that flow).

        `priority` is required (int, higher = more urgent).
        `actions` is the legacy side-effect list (action_type =
        resume_run | open_intent | run_command). It does NOT render UI.
        """
        if priority is None:
            raise TypeError("notify() requires 'priority' (int, higher = more urgent)")
        _emit({"type": "notify", "level": level, "title": title,
               "body": body, "kind": "message",
               "priority": int(priority),
               "actions": actions or []})

    # kind = "choice"
    def notify_choice(self, title: str, options: list, body: str = "",
                      level: str = "info", required: bool = False,
                      priority: int | None = None) -> str:
        """Post a choice notification and block until the user picks one.

        `options` is a list of dicts: {"label": str, "value": str (optional),
        "shortcut": str (optional, single char)}. If `value` is omitted, the
        label is returned.

        `priority` is required (int, higher = more urgent).
        Returns the chosen option's value (or the label if no value set). If
        `required=False` the user may cancel with Esc — this returns the string
        `"__cancel__"`.
        """
        if priority is None:
            raise TypeError("notify_choice() requires 'priority' (int, higher = more urgent)")
        import uuid
        notify_id = str(uuid.uuid4())
        q: "queue.Queue[str]" = queue.Queue()
        self._app._pending_notify[notify_id] = q
        _emit({"type": "notify", "level": level, "title": title, "body": body,
               "kind": "choice", "options": options, "required": required,
               "priority": int(priority),
               "notify_id": notify_id})
        return q.get()

    # kind = "input"
    def notify_input(self, title: str, prompt: str = "", body: str = "",
                     level: str = "info", required: bool = False,
                     priority: int | None = None) -> str:
        """Post an input notification and block until the user submits or
        cancels. Returns the typed text (possibly empty), or "__cancel__" if
        the user dismissed with Esc (only possible when required=False).

        `priority` is required (int, higher = more urgent).
        """
        if priority is None:
            raise TypeError("notify_input() requires 'priority' (int, higher = more urgent)")
        import uuid
        notify_id = str(uuid.uuid4())
        q: "queue.Queue[str]" = queue.Queue()
        self._app._pending_notify[notify_id] = q
        _emit({"type": "notify", "level": level, "title": title, "body": body,
               "kind": "input", "input_prompt": prompt, "required": required,
               "priority": int(priority),
               "notify_id": notify_id})
        return q.get()

    def notify_and_wait(self, title: str, body: str = "", level: str = "info",
                        actions: list | None = None,
                        priority: int | None = None) -> str:
        """Post a message notification and block until the user acknowledges
        or cancels. Returns "acknowledge" on Enter/Space/button, "cancel" on Esc.

        For richer interaction, use `notify_choice` or `notify_input`.
        `priority` is required (int, higher = more urgent).
        `actions` is the legacy server-side side-effect list.
        """
        if priority is None:
            raise TypeError("notify_and_wait() requires 'priority' (int, higher = more urgent)")
        import uuid
        notify_id = str(uuid.uuid4())
        q: "queue.Queue[str]" = queue.Queue()
        self._app._pending_notify[notify_id] = q
        _emit({"type": "notify", "level": level, "title": title, "body": body,
               "kind": "message", "actions": actions or [],
               "priority": int(priority),
               "notify_id": notify_id})
        return q.get()

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

    # Blocking helpers — waits for host response on the stdin reader thread
    def capability_request(self, capability: str) -> bool:
        """Block until host grants or denies the capability. Returns True if granted."""
        import uuid
        req_id = str(uuid.uuid4())
        q: "queue.Queue[bool]" = queue.Queue()
        self._app._pending_capability[req_id] = q
        _emit({"type": "capability_request", "request_id": req_id,
               "capability": capability})
        return q.get()

    def secret_get(self, key: str) -> str | None:
        """Block until host returns the secret value (or None if denied)."""
        q: "queue.Queue[str | None]" = queue.Queue()
        self._app._pending_secret[key] = q
        _emit({"type": "secret_get", "key": key})
        return q.get()

    def get_secret(self, key: str) -> str | None:
        """Alias for secret_get(). Preferred name going forward."""
        return self.secret_get(key)

    def http_get(self, url: str) -> str:
        """Blocking HTTP GET brokered through the host. Requires net.http capability.
        Call from any thread (background threads included). Raises RuntimeError on failure."""
        return self.http_request(url)

    def http_request(self, url: str, method: str = "GET",
                     headers: "dict[str, str] | None" = None,
                     body: "str | None" = None) -> str:
        """Blocking HTTP request brokered through the host. Requires net.http capability.
        Supports custom method, headers, and body. Raises RuntimeError on failure."""
        import uuid
        req_id = str(uuid.uuid4())
        q: "queue.Queue[tuple[str, str]]" = queue.Queue()
        self._app._pending_http[req_id] = q
        payload: dict = {"type": "http_request", "request_id": req_id,
                         "method": method, "url": url}
        if headers:
            payload["headers"] = headers
        if body is not None:
            payload["body"] = body
        _emit(payload)
        status, value = q.get()
        if status == "error":
            raise RuntimeError(f"http_request {url!r}: {value}")
        return value

    def llm(self, prompt: str, model: str = "claude-haiku-4-5-20251001",
            system: str | None = None) -> str:
        """Blocking LLM call brokered through the host. Requires llm capability.
        Uses ANTHROPIC_API_KEY from the Plexi secrets store.
        Returns the text response. Raises RuntimeError on failure."""
        import uuid
        req_id = str(uuid.uuid4())
        q: "queue.Queue[tuple[str, str]]" = queue.Queue()
        self._app._pending_llm[req_id] = q
        payload: dict = {"type": "llm_request", "request_id": req_id,
                         "prompt": prompt, "model": model}
        if system is not None:
            payload["system"] = system
        _emit(payload)
        status, value = q.get()
        if status == "error":
            raise RuntimeError(f"llm call failed: {value}")
        return value

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

    def measure_text(self, text: str, font_size: float,
                     monospace: bool = False) -> "tuple[float, float]":
        """Measure `text` at `font_size` using the host's real font metrics.

        Sends a `MeasureText` DrawCommand and blocks until the host responds
        with `TextMeasured`. Returns `(width, height)` in logical pixels.

        Use this only when layout depends on measured text width (e.g. flowing
        multiple badges horizontally). Avoid on hot render paths — prefer
        passing `max_width` on `ctx.text()` for simple truncation.
        """
        import uuid
        request_id = str(uuid.uuid4())
        _emit({"type": "measure_text", "request_id": request_id,
               "text": text, "font_size": font_size, "monospace": monospace})
        # Block until the matching TextMeasured response arrives on stdin.
        # The App event loop reads from stdin; we need to read directly here
        # because we're inside a frame callback. Use the shared stdin lock.
        import sys as _sys
        for line in _sys.stdin:
            line = line.strip()
            if not line:
                continue
            try:
                import json as _json
                event = _json.loads(line)
            except Exception:
                continue
            if (event.get("type") == "text_measured"
                    and event.get("request_id") == request_id):
                return float(event.get("width", 0.0)), float(event.get("height", 0.0))
            # Stash any non-matching events so the main loop sees them.
            # NOTE: This is a best-effort flush for the common case (no other
            # events expected mid-frame). A full async approach would require
            # a separate thread; apps that need that should use DrawCommand::MeasureText
            # with an explicit NotifyAction-style async pattern instead.

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
                   placeholder: str = "") -> "str | None":
        """Single-line text input — host-owned buffer, submit-only.

        Emits a `DrawCommand::TextInput` and returns the most recently
        submitted value for `id` if any landed since the previous frame,
        else `None`. The host owns the buffer entirely — typed
        characters never reach the app between keystrokes. On Enter the
        host emits `PlexiEvent::TextSubmitted { id, value }` and clears
        its buffer.

        Pattern (poll on every frame)::

            submitted = ctx.text_input("note", x=12, y=12, w=300,
                                       placeholder="Type a note…")
            if submitted is not None:
                save_note(submitted)

        Real-time validation (per-keystroke access) is out of scope —
        see issue #283. Use `TextArea` for multi-line app-managed
        editors instead.
        """
        _emit({"type": "text_input", "id": id, "x": x, "y": y, "w": w,
               "placeholder": placeholder})
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
               actions: list | None = None) -> None:
        """Post a message notification. See Emitter.notify.
        `priority` is required (int, higher = more urgent).
        Scope is resolved from the app's manifest — not an argument."""
        self.emit.notify(title=title, body=body, level=level,
                         priority=priority, actions=actions)

    def notify_choice(self, title: str, options: list, body: str = "",
                      level: str = "info", required: bool = False,
                      priority: int | None = None) -> str:
        """Post a choice notification and block until the user picks.
        `priority` is required (int, higher = more urgent).
        Returns the chosen option's value (or label if no value set),
        or "__cancel__" if the user dismissed."""
        return self.emit.notify_choice(title=title, options=options, body=body,
                                       level=level, required=required,
                                       priority=priority)

    def notify_input(self, title: str, prompt: str = "", body: str = "",
                     level: str = "info", required: bool = False,
                     priority: int | None = None) -> str:
        """Post an input notification and block until the user submits.
        `priority` is required (int, higher = more urgent).
        Returns the typed text, or "__cancel__" if dismissed."""
        return self.emit.notify_input(title=title, prompt=prompt, body=body,
                                      level=level, required=required,
                                      priority=priority)

    def notify_and_wait(self, title: str, body: str = "", level: str = "info",
                        actions: list | None = None,
                        priority: int | None = None) -> str:
        """Post a message notification and block for acknowledge/cancel.
        `priority` is required (int, higher = more urgent).
        See Emitter.notify_and_wait."""
        return self.emit.notify_and_wait(title=title, body=body, level=level,
                                         actions=actions, priority=priority)

    def status_summary(self, text: str) -> None:
        _emit({"type": "status_summary", "text": text})

    def set_timer(self, timer_id: str, after_ms: int) -> None:
        self.emit.set_timer(timer_id, after_ms)

    def cancel_timer(self, timer_id: str) -> None:
        self.emit.cancel_timer(timer_id)

    def get_secret(self, key: str) -> str | None:
        """Request a secret by key. Alias for emit.get_secret(). Blocking."""
        return self.emit.get_secret(key)

    def http_request(self, url: str, method: str = "GET",
                     headers: "dict[str, str] | None" = None,
                     body: "str | None" = None) -> str:
        """Blocking HTTP request with optional method, headers, body. Requires net.http capability."""
        return self.emit.http_request(url, method=method, headers=headers, body=body)

    def llm(self, prompt: str, model: str = "claude-haiku-4-5-20251001",
            system: str | None = None) -> str:
        """Blocking LLM call brokered through the host. Requires llm capability.
        Uses ANTHROPIC_API_KEY from the Plexi secrets store."""
        return self.emit.llm(prompt=prompt, model=model, system=system)

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
        self._pending_capability: dict[str, queue.Queue] = {}
        self._pending_secret: dict[str, queue.Queue] = {}
        self._pending_http: dict[str, queue.Queue] = {}
        self._pending_llm: dict[str, queue.Queue] = {}
        self._pending_notify: dict[str, queue.Queue] = {}
        self._pipes: dict[str, Pipe] = {}
        self._last_render_time: float | None = None
        # Pending text-input submissions keyed on TextInput `id`. The
        # event-loop thread fills this when `PlexiEvent::TextSubmitted`
        # arrives; `RenderContext.text_input` drains it during render.
        # One pending value per id — a second submit before the app
        # consumes the first overwrites (apps poll every frame, so
        # this only matters in a perverse scheduling case).
        self._text_submissions: dict[str, str] = {}
        self.emit = Emitter(self)

    # ── Override these ──────────────────────────────────────────────────────
    def on_init(self, _ctx: RenderContext) -> None: pass
    def on_render(self, _ctx: RenderContext) -> None: pass
    def on_key(self, _ctx: RenderContext, _key: str, _mods: dict) -> None: pass
    def on_click(self, _ctx: RenderContext, _x: float, _y: float, _button: str) -> None: pass
    def on_command(self, _ctx: RenderContext, _text: str) -> None: pass
    def on_paste(self, _ctx: RenderContext, _text: str) -> None: pass
    def on_pipe_message(self, _ctx: RenderContext, _pipe_id: str, _payload: Any) -> None: pass
    def on_path_changed(self, _ctx: RenderContext, _cwd: str) -> None: pass
    def on_inject(self, _ctx: RenderContext, _payload: Any) -> None: pass
    def on_app_spawned(self, _pane_id: int, _type_id: str) -> None: pass
    def on_timer(self, _ctx: "RenderContext", _timer_id: str) -> None: pass
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
        """Start the PGAP v3 event loop. Blocks until Shutdown."""
        sys.stdout.reconfigure(line_buffering=True)  # type: ignore[attr-defined]

        for raw in sys.stdin:
            raw = raw.strip()
            if not raw:
                continue
            try:
                ev = json.loads(raw)
            except json.JSONDecodeError:
                continue

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
                self.on_init(self._make_ctx())

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
                    self.on_render(ctx)
                except Exception as e:
                    ctx.error(f"on_render exception: {e}")
                ctx.frame_done()

            elif t == "key":
                ctx = self._make_ctx()
                self.on_key(ctx, ev.get("key", ""), ev.get("modifiers", {}))

            elif t == "click":
                ctx = self._make_ctx()
                self.on_click(ctx, ev.get("x", 0.0), ev.get("y", 0.0),
                              ev.get("button", "primary"))

            elif t == "command":
                ctx = self._make_ctx()
                self.on_command(ctx, ev.get("text", ""))

            elif t == "paste":
                ctx = self._make_ctx()
                self.on_paste(ctx, ev.get("text", ""))

            elif t == "capability_decision":
                req_id = ev.get("request_id", "")
                granted = ev.get("granted", False)
                q = self._pending_capability.pop(req_id, None)
                if q:
                    q.put(granted)

            elif t == "secret_value":
                key = ev.get("key", "")
                value = ev.get("value")
                q = self._pending_secret.pop(key, None)
                if q:
                    q.put(value)

            elif t == "pipe_message":
                ctx = self._make_ctx()
                self.on_pipe_message(ctx, ev.get("pipe_id", ""), ev.get("payload"))

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

            elif t == "path_changed":
                ctx = self._make_ctx()
                self.on_path_changed(ctx, ev.get("cwd", ""))

            elif t == "suspend":
                self.on_suspend()

            elif t == "resume":
                self.on_resume()

            elif t == "shutdown":
                self.on_shutdown()
                break

            elif t == "inject_state":
                ctx = self._make_ctx()
                self.on_inject(ctx, ev.get("payload", {}))

            elif t == "http_response":
                req_id = ev.get("request_id", "")
                q = self._pending_http.pop(req_id, None)
                if q:
                    if ev.get("error"):
                        q.put(("error", ev["error"]))
                    else:
                        q.put(("ok", ev.get("body", "")))

            elif t == "llm_response":
                req_id = ev.get("request_id", "")
                q = self._pending_llm.pop(req_id, None)
                if q:
                    if ev.get("error"):
                        q.put(("error", ev["error"]))
                    else:
                        q.put(("ok", ev.get("content", "")))

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
                        q.put("__cancel__")
                    elif value is not None:
                        q.put(value)
                    else:
                        q.put(action_label or "acknowledge")

            elif t == "text_submitted":
                # Host-owned text input: the user pressed Enter on a
                # `DrawCommand::TextInput` field. Stash the value keyed
                # on the input id; `RenderContext.text_input(...)` will
                # drain it on the next frame the app polls.
                tid = ev.get("id", "")
                if tid:
                    self._text_submissions[tid] = ev.get("value", "")

            elif t == "timer":
                timer_id = ev.get("timer_id", "")
                ctx = self._make_ctx()
                self.on_timer(ctx, timer_id)

            elif t in ("run_update",):
                pass  # apps can override on_run_update if needed

            elif t == "app_spawned":
                # Confirmation that a SpawnApp request succeeded. Apps that
                # want to track the spawned pane can override on_app_spawned.
                try:
                    self.on_app_spawned(
                        int(ev.get("pane_id", 0)),
                        str(ev.get("type_id", "")),
                    )
                except Exception as e:
                    sys.stderr.write(f"on_app_spawned handler raised: {e}\n")

        # Ensure all pipes are closed cleanly
        for p in self._pipes.values():
            p.close()
