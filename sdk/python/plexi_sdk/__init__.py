"""
plexi_sdk.py — Plexi external app SDK (Python), PGAP v3

Spec: docs/specs/releases/plexi-v3.0.md §3 (PGAP v3), §7 (typed pipes).
Zero dependencies, pure stdlib. Copy into your app directory and import.

Protocol: newline-delimited JSON over stdin/stdout.
  PlexiEvent  (host → app): Init, Render, Key, Click, Command, CapabilityDecision,
              SecretValue, RunUpdate, PipeMessage, PathChanged, Suspend, Resume,
              Shutdown, PipeOpened, PipeOverrun
  DrawCommand (app → host): Rect, Text, Line, List, FrameDone, Log,
              CapabilityRequest, SecretGet, RunGet, RunComplete, Notify,
              PipeOpen, PipeSend, StatusSummary, RunInTerminal, Cd

Usage:

    class MyApp(App):
        def on_init(self, ctx):
            pass  # called after Init handshake completes

        def on_render(self, ctx):
            ctx.rect(0, 0, ctx.w, ctx.h, fill="#1e1e2e")
            ctx.text(20, 20, "Hello v3!", size=14, color="#cdd6f4")

        def on_key(self, ctx, key, mods):
            if key == "q":
                pass

    MyApp().run()
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
                 capabilities: list[str], feature_flags: list[str], app: "App"):
        self.frame_id = frame_id
        self.x: float = rect.get("x", 0.0)
        self.y: float = rect.get("y", 0.0)
        self.w: float = rect.get("w", 800.0)
        self.h: float = rect.get("h", 600.0)
        self.workspace_root = workspace_root
        self.capabilities = capabilities
        self.feature_flags = feature_flags
        self._app = app
        # Convenience alias
        self.emit = app.emit

    # ── Visual primitives ──
    def rect(self, x: float, y: float, w: float, h: float, fill: str,
             radius: float = 0.0) -> None:
        _emit({"type": "rect", "x": x, "y": y, "w": w, "h": h,
               "fill": fill, "radius": radius})

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

    def status_summary(self, text: str) -> None:
        _emit({"type": "status_summary", "text": text})

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
        self._pipes: dict[str, Pipe] = {}
        self.emit = Emitter(self)

    # ── Override these ──────────────────────────────────────────────────────
    def on_init(self, ctx: RenderContext) -> None: pass
    def on_render(self, ctx: RenderContext) -> None: pass
    def on_key(self, ctx: RenderContext, key: str, mods: dict) -> None: pass
    def on_click(self, ctx: RenderContext, x: float, y: float, button: str) -> None: pass
    def on_command(self, ctx: RenderContext, text: str) -> None: pass
    def on_pipe_message(self, ctx: RenderContext, pipe_id: str, payload: Any) -> None: pass
    def on_path_changed(self, ctx: RenderContext, cwd: str) -> None: pass
    def on_inject(self, ctx: "RenderContext", payload: Any) -> None: pass
    def on_app_spawned(self, pane_id: int, type_id: str) -> None: pass
    def on_suspend(self) -> None: pass
    def on_resume(self) -> None: pass
    def on_shutdown(self) -> None: pass

    # ── Internal ────────────────────────────────────────────────────────────
    def _make_ctx(self, frame_id: int = 0) -> RenderContext:
        return RenderContext(
            frame_id=frame_id,
            rect=self._rect,
            workspace_root=self.workspace_root,
            capabilities=self.capabilities,
            feature_flags=self.feature_flags,
            app=self,
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
                frame_id = ev.get("frame_id", 0)
                if "rect" in ev:
                    self._rect = ev["rect"]
                elif "width" in ev:
                    # legacy compat
                    self._rect = {"x": 0.0, "y": 0.0,
                                  "w": ev["width"], "h": ev["height"]}
                ctx = self._make_ctx(frame_id)
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
