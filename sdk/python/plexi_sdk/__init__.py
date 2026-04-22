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

    # Notifications
    def notify(self, title: str, body: str, level: str = "info",
               actions: list | None = None) -> None:
        _emit({"type": "notify", "level": level, "title": title,
               "body": body, "actions": actions or []})

    def notify_and_wait(self, title: str, body: str, level: str = "info",
                        actions: list | None = None) -> str:
        """Send a notification and block until the user responds.

        Returns the label of the action clicked, or "dismiss" if dismissed.
        Requires A5 notification panel to be active — blocks indefinitely otherwise.

        actions: list of {"label": str, "action_type": str, "payload": dict} dicts.
        If omitted, a single "Dismiss" action is added automatically.
        """
        import uuid
        notify_id = str(uuid.uuid4())
        q: "queue.Queue[str]" = queue.Queue()
        self._app._pending_notify[notify_id] = q
        actual_actions = actions or [{"label": "Dismiss", "action_type": "dismiss", "payload": {}}]
        _emit({"type": "notify", "level": level, "title": title, "body": body,
               "actions": actual_actions, "notify_id": notify_id})
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
        import uuid
        req_id = str(uuid.uuid4())
        q: "queue.Queue[tuple[str, str]]" = queue.Queue()
        self._app._pending_http[req_id] = q
        _emit({"type": "http_request", "request_id": req_id, "method": "GET", "url": url})
        status, value = q.get()
        if status == "error":
            raise RuntimeError(f"http_get {url!r}: {value}")
        return value

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
             monospace: bool = False, bold: bool = False) -> None:
        _emit({"type": "text", "x": x, "y": y, "text": text, "size": size,
               "color": color, "monospace": monospace, "bold": bold})

    def line(self, x1: float, y1: float, x2: float, y2: float,
             color: str, width: float = 1.0) -> None:
        _emit({"type": "line", "x1": x1, "y1": y1, "x2": x2, "y2": y2,
               "color": color, "width": width})

    def list(self, items: list[dict], selected: int = 0,
             item_height: float = 40.0, x: float = 0.0, y: float = 0.0,
             w: float | None = None, h: float | None = None) -> None:
        _emit({"type": "list", "x": x, "y": y,
               "w": self.w if w is None else w,
               "h": self.h - y if h is None else h,
               "items": items, "selected": selected,
               "item_height": item_height})

    # Logging helpers (in-frame, forwarded to host logger)
    def log(self, level: str, message: str) -> None:
        _emit({"type": "log", "level": level, "message": message})

    def info(self, message: str) -> None:  self.log("info", message)
    def warn(self, message: str) -> None:  self.log("warn", message)
    def error(self, message: str) -> None: self.log("error", message)
    def debug(self, message: str) -> None: self.log("debug", message)

    def notify(self, title: str, body: str, level: str = "info",
               actions: list | None = None) -> None:
        self.emit.notify(title=title, body=body, level=level, actions=actions)

    def notify_and_wait(self, title: str, body: str, level: str = "info",
                        actions: list | None = None) -> str:
        """Send a notification and block until the user responds. See Emitter.notify_and_wait."""
        return self.emit.notify_and_wait(title=title, body=body, level=level, actions=actions)

    def status_summary(self, text: str) -> None:
        _emit({"type": "status_summary", "text": text})

    def get_secret(self, key: str) -> str | None:
        """Request a secret by key. Alias for emit.get_secret(). Blocking."""
        return self.emit.get_secret(key)

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
        self._pending_notify: dict[str, queue.Queue] = {}
        self._pipes: dict[str, Pipe] = {}
        self._last_render_time: float | None = None
        self.emit = Emitter(self)

    # ── Override these ──────────────────────────────────────────────────────
    def on_init(self, _ctx: RenderContext) -> None: pass
    def on_render(self, _ctx: RenderContext) -> None: pass
    def on_key(self, _ctx: RenderContext, _key: str, _mods: dict) -> None: pass
    def on_click(self, _ctx: RenderContext, _x: float, _y: float, _button: str) -> None: pass
    def on_command(self, _ctx: RenderContext, _text: str) -> None: pass
    def on_pipe_message(self, _ctx: RenderContext, _pipe_id: str, _payload: Any) -> None: pass
    def on_path_changed(self, _ctx: RenderContext, _cwd: str) -> None: pass
    def on_inject(self, _ctx: RenderContext, _payload: Any) -> None: pass
    def on_app_spawned(self, _pane_id: int, _type_id: str) -> None: pass
    def on_suspend(self) -> None: pass
    def on_resume(self) -> None: pass
    def on_shutdown(self) -> None: pass

    # ── Internal ────────────────────────────────────────────────────────────
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

            elif t == "notify_action":
                notify_id = ev.get("notify_id", "")
                action_label = ev.get("action_label", "dismiss")
                q = self._pending_notify.pop(notify_id, None)
                if q:
                    q.put(action_label)

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
