"""
plexi_sdk.py — Plexi external app SDK (Python)

Zero dependencies, pure stdlib. Copy this file into your app directory.

Protocol: newline-delimited JSON over stdin/stdout.
  - Plexi sends PlexiEvent objects  {"type": "render", "width": ..., "height": ...}
  - App responds with DrawCommand objects {"type": "rect", ...} + {"type": "frame_done"}

Usage:

    from plexi_sdk import App

    app = App()

    @app.on_render
    def render(ctx):
        ctx.rect(0, 0, ctx.width, ctx.height, fill="#1e1e2e")
        ctx.text(20, 20, "Hello Plexi!", size=16, color="#cdd6f4")

    app.run()

Key handlers can emit terminal commands via the Emitter passed as the second arg:

    @app.on_key
    def on_key(key, mods, emit):
        if key == "Enter":
            emit.run_in_terminal("echo hello")

State management (Plexi handles undo/redo/save):

    @app.on_get_state
    def get_state():
        return {
            "user_state": {"cursor": 0, "selected": None},
            "derived": {},
            "session": {"scroll_offset": 120},
            "persistent": {"bookmarks": [1, 5, 9]},
        }

    @app.on_set_state
    def set_state(state):
        cursor = state["user_state"].get("cursor", 0)
        # ... restore your app's internal state from the buckets

Cost reporting (for apps that call LLM APIs):

    emit.cost_report(
        service="anthropic", model="claude-sonnet-4-20250514",
        input_tokens=1500, output_tokens=500, cost_usd=0.01,
    )
"""


from __future__ import annotations

__version__ = "0.3.0"

import json
import os
import pathlib
import shutil
import sys
import uuid
from dataclasses import asdict, dataclass, field, is_dataclass
from datetime import datetime, timezone
from typing import Callable, List, Optional, Tuple


# ─── Text size constants ──────────────────────────────────────────────────────
# Use these instead of arbitrary numbers. They give consistent legibility
# across apps and survive future scale tuning.
#
#   TITLE      — page/app title, one per screen
#   HEADING    — section headings
#   BODY       — default body text; what you should pick 90% of the time
#   CAPTION    — secondary labels, timestamps, file metadata
#   HINT       — keyboard shortcut bars, status lines; smallest legible tier
#   MONO_BODY  — default monospace body (code, paths, JSON)
#   MONO_SMALL — smallest monospace tier (inline diff markers, tight tables)
#
# Pixel sizes were chosen so that at typical macOS Retina scale (2.0x),
# the smallest tier (HINT) still has ≥12 logical pixels of cap height.

TITLE      = 22.0
HEADING    = 18.0
BODY       = 15.0
CAPTION    = 13.0
HINT       = 12.0
MONO_BODY  = 14.0
MONO_SMALL = 12.0

# ─── Safe-area constants ──────────────────────────────────────────────────────
# Recommended padding + chrome heights so app layouts compose uniformly.
#
#   PAD         — default outer padding (use for all edge insets)
#   PAD_TIGHT   — use only inside dense lists/grids
#   HEADER_H    — recommended header bar height (fits TITLE + vertical breathing)
#   STATUS_H    — recommended status/hint bar height at the bottom

PAD         = 16.0
PAD_TIGHT   = 8.0
HEADER_H    = 48.0
STATUS_H    = 44.0


# ─── Theme ────────────────────────────────────────────────────────────────────
# Default Catppuccin palette. Apps can `from plexi_sdk import THEME` and
# reference `THEME.accent`, etc., or construct their own `Theme(...)` and
# pass it into component calls (`ctx.header(..., theme=my_theme)`).

# ─── v2.1 measurement type ────────────────────────────────────────────────────

@dataclass
class TextMetrics:
    """Result of a MeasureText request — exact font metrics from Plexi."""
    width: float
    height: float
    ascent: float


# ─── v2.0 OpenIntent ─────────────────────────────────────────────────────────

class OpenIntent:
    """Structured spawn intent passed at app startup via the Init event."""

    def __init__(self, kind_type: str, **kwargs):
        self.kind_type = kind_type
        self.kwargs = kwargs

    @classmethod
    def file(cls, path: str, start_line: Optional[int] = None, end_line: Optional[int] = None) -> "OpenIntent":
        """Open a file, optionally at a specific line range."""
        r: dict = {"kind": "file", "path": path}
        if start_line is not None:
            r["range"] = {"start_line": start_line, "end_line": end_line or start_line}
        return cls("file", **r)

    @classmethod
    def prompt(cls, text: str, model_hint: Optional[str] = None) -> "OpenIntent":
        """Open with a text prompt."""
        d: dict = {"kind": "prompt", "text": text}
        if model_hint:
            d["model_hint"] = model_hint
        return cls("prompt", **d)

    @classmethod
    def bare(cls) -> "OpenIntent":
        """Open with no structured intent."""
        return cls("bare", kind="bare")

    @classmethod
    def resume(cls, snapshot_key: str) -> "OpenIntent":
        """Resume from a snapshot key."""
        return cls("resume", kind="resume", snapshot_key=snapshot_key)

    @classmethod
    def from_dict(cls, d: dict) -> "OpenIntent":
        """Deserialize from the Init event payload."""
        kind_type = d.get("kind", "bare")
        return cls(kind_type, **d)

    def to_dict(self) -> dict:
        return self.kwargs


@dataclass
class Theme:
    bg:        str = "#1e1e2e"
    surface:   str = "#313244"
    highlight: str = "#45475a"
    accent:    str = "#89b4fa"
    muted:     str = "#6c7086"
    fg:        str = "#cdd6f4"
    red:       str = "#f38ba8"
    green:     str = "#a6e3a1"
    yellow:    str = "#f9e2af"


THEME = Theme()


class RenderMode:
    """Render modes surfaced by protocol v2."""

    FULL = "full"
    PREVIEW = "preview"


@dataclass
class OpenIntent:
    """Structured launch context supplied by the host on Init."""

    kind: str = "bare"
    caller: Optional[dict] = None
    payload: Optional[dict] = None
    run_id: Optional[str] = None

    @classmethod
    def bare(cls) -> "OpenIntent":
        return cls(kind="bare")

    @classmethod
    def file(cls, path: str, text_range: Optional[dict] = None) -> "OpenIntent":
        payload: dict = {"path": path}
        if text_range is not None:
            payload["range"] = text_range
        return cls(kind="file", payload=payload)

    @classmethod
    def url(cls, url: str) -> "OpenIntent":
        return cls(kind="url", payload={"url": url})

    @classmethod
    def prompt(cls, text: str, model_hint: Optional[str] = None) -> "OpenIntent":
        payload: dict = {"text": text}
        if model_hint is not None:
            payload["model_hint"] = model_hint
        return cls(kind="prompt", payload=payload)

    @classmethod
    def resume(cls, snapshot_key: str) -> "OpenIntent":
        return cls(kind="resume", payload={"snapshot_key": snapshot_key})

    def to_dict(self) -> dict:
        return asdict(self)

    @classmethod
    def from_value(cls, value) -> Optional["OpenIntent"]:
        if isinstance(value, cls):
            return value
        if not isinstance(value, dict):
            return None
        return cls(
            kind=str(value.get("kind", "bare")),
            caller=value.get("caller"),
            payload=value.get("payload"),
            run_id=value.get("run_id"),
        )


@dataclass
class Health:
    """Compact status indicator surfaced in StatusSummary."""

    status: str = "running"


@dataclass
class PaneSummary:
    """Serializable one-line summary for a pane/depth node."""

    pane_id: int
    cwd: str
    status_text: Optional[str] = None
    last_activity_unix_ms: Optional[int] = None
    health: Health = field(default_factory=Health)

    def to_dict(self) -> dict:
        return {
            "pane_id": self.pane_id,
            "cwd": self.cwd,
            "status_text": self.status_text,
            "last_activity_unix_ms": self.last_activity_unix_ms,
            "health": self.health.status,
        }


@dataclass
class StatusSummary:
    """Serializable cheap-status payload for preview renders."""

    uptime_seconds: float = 0.0
    process_count: int = 0
    summary_text: Optional[str] = None
    health: Health = field(default_factory=Health)
    last_activity_unix_ms: Optional[int] = None
    panes: List[PaneSummary] = field(default_factory=list)

    def to_dict(self) -> dict:
        return {
            "uptime_seconds": self.uptime_seconds,
            "process_count": self.process_count,
            "last_activity_unix_ms": self.last_activity_unix_ms,
            "summary_text": self.summary_text,
            "health": self.health.status,
            "panes": [_payload_dict(pane) for pane in self.panes],
        }


def _payload_dict(value):
    """Serialize dataclasses and helper objects to plain JSON values."""
    if value is None:
        return None
    if hasattr(value, "to_dict"):
        return value.to_dict()
    if is_dataclass(value):
        return asdict(value)
    return value


# ─── Filesystem utilities ─────────────────────────────────────────────────────
def safe_move(src: pathlib.Path, dest_dir: pathlib.Path) -> str:
    """
    Move `src` into `dest_dir`, creating the directory if needed.

    Returns a short status string suitable for display in a status bar:
      - "Moved <name>"      on success
      - "Error: <message>"  on failure

    If `dest_dir/src.name` already exists, appends a timestamp suffix to the
    destination filename so nothing is clobbered.
    """
    try:
        dest_dir.mkdir(parents=True, exist_ok=True)
        dest = dest_dir / src.name
        if dest.exists():
            stem = src.stem
            suffix = src.suffix
            ts = datetime.now().strftime("%Y%m%d-%H%M%S")
            dest = dest_dir / f"{stem}.{ts}{suffix}"
        shutil.move(str(src), str(dest))
        return f"Moved {src.name}"
    except OSError as e:
        return f"Error: {e}"


class Emitter:
    """Emit commands to Plexi immediately (outside a render frame)."""

    def __init__(self, app_id: str = ""):
        self._app_id = app_id

    def run_in_terminal(self, command: str):
        """Execute a shell command in the linked terminal."""
        print(json.dumps({"type": "run_in_terminal", "command": command}), flush=True)

    def cd(self, path: str):
        """Change the linked terminal's working directory."""
        print(json.dumps({"type": "cd", "path": path}), flush=True)

    def log(self, level: str, message: str):
        """Forward a log message to Plexi's logger (level: error|warn|info|debug)."""
        print(json.dumps({"type": "log", "level": level, "message": message}), flush=True)

    def info(self, message: str):
        """Log at info level."""
        self.log("info", message)

    def warn(self, message: str):
        """Log at warn level."""
        self.log("warn", message)

    def error(self, message: str):
        """Log at error level."""
        self.log("error", message)

    def debug(self, message: str):
        """Log at debug level."""
        self.log("debug", message)

    def cost_report(
        self,
        service: str,
        model: str,
        input_tokens: int,
        output_tokens: int,
        cost_usd: float,
        operation_id: Optional[str] = None,
    ):
        """Report LLM API cost to Plexi for logging and tracking."""
        print(json.dumps({
            "type": "cost_report",
            "app_id": self._app_id,
            "service": service,
            "model": model,
            "input_tokens": input_tokens,
            "output_tokens": output_tokens,
            "cost_usd": cost_usd,
            "operation_id": operation_id or str(uuid.uuid4()),
            "timestamp": datetime.now(timezone.utc).isoformat(),
        }), flush=True)

    def notify(
        self,
        title: str,
        body: Optional[str] = None,
        urgency: str = "low",
        action_type: str = "dismiss",
        action_payload: Optional[dict] = None,
        expires_at: Optional[int] = None,
        visible_after: Optional[int] = None,
    ):
        """
        Raise a notification to Plexi's notification log.

        urgency: "low" | "medium" | "high"
        action_type: "dismiss" | "focus" | "confirm" | "text_input"
        action_payload: type-dependent dict; for "focus": {"pane_id": int, "fullscreen": bool}
        expires_at: unix timestamp (seconds); notification is dropped if already past
        visible_after: unix timestamp (seconds); defer display until this time

        The notification is recorded to ~/.plexi-alpha/notifications.jsonl,
        increments the status-bar unread count, and appears in the
        notification palette (Cmd+Shift+N).
        """
        cmd: dict = {
            "type": "notification",
            "title": title,
            "source_app": self._app_id,
            "urgency": urgency,
            "action_type": action_type,
        }
        if body is not None:
            cmd["body"] = body
        if action_payload is not None:
            cmd["action_payload"] = action_payload
        if expires_at is not None:
            cmd["expires_at"] = expires_at
        if visible_after is not None:
            cmd["visible_after"] = visible_after
        print(json.dumps(cmd), flush=True)

    def notification(
        self,
        title: str,
        body: Optional[str] = None,
        priority: int = 1,
    ):
        """Deprecated: use notify() instead. Kept for backwards compatibility."""
        urgency = {0: "low", 1: "low", 2: "medium", 3: "high"}.get(priority, "low")
        self.notify(title=title, body=body, urgency=urgency)

    def spawn_app(
        self,
        app_id: str,
        args: Optional[List[str]] = None,
        parent: str = "self",
        layout: Optional[dict] = None,
        lifecycle: str = "cascade",
        linked: bool = True,
        wire_channels: Optional[List[str]] = None,
        open_intent: Optional[OpenIntent] = None,
    ):
        """
        Ask Plexi to spawn another app and place it in a layout slot relative to this one.

        This is the composition primitive: a file browser pressing Enter on a
        .txt file can emit `spawn_app("text-editor", args=[path], layout={"kind":"cols","slot":1,"ratio":0.5})`
        to bring up a text editor in a 50/50 right split, lifecycle-bonded to
        the file browser (so closing the file browser closes the editor).

        Args:
            app_id:
                The id of the app to spawn (must be in Plexi's app registry).
            args:
                Command-line args forwarded to the spawned app as argv[1..].
                Defaults to no args.
            parent:
                Anchor for the new pane's position. One of:
                    "self"  (default) — the pane emitting this call
                    "root"            — top-level (ignores the emitter's location)
                    "mark:<name>"     — reserved for a future named-layout system
            layout:
                How to position the new pane relative to the parent. A dict
                with a "kind" key. Examples:
                    {"kind": "fill"}
                    {"kind": "cols", "slot": 1, "ratio": 0.5}
                    {"kind": "rows", "slot": 1, "ratio": 0.4}
                    {"kind": "grid_2x2", "slot": 0}  # v1 stub, falls back to fill
                Defaults to {"kind": "fill"}.
            lifecycle:
                What happens to the spawned app when this app closes:
                    "cascade" (default) — close together
                    "orphan"            — detach, stay alive as a top-level pane
                    "prompt"            — ask the user (v1 stub, falls back to orphan)
            linked:
                When True (default), the new pane joins this pane's linked
                group so terminal-linking is shared.
            wire_channels:
                Typed-pipe channel names to pre-wire (e.g. ["file_buffer"]).
                Stored on the spawn relationship for the typed-pipes spec;
                defaults to no pre-wired channels.
            open_intent:
                Optional structured launch context for the child. Use
                OpenIntent.file(...), OpenIntent.prompt(...), or
                OpenIntent.resume(...) when the child should open with
                depth-aware context.

        Authorization: the target app's `[app.spawnable]` manifest table is
        consulted — if `allow_callers` doesn't include this app, or the
        requested `lifecycle` isn't in `allow_lifecycle`, the spawn is
        refused and a notification is delivered back to this app.
        """
        cmd = {
            "type": "spawn_app",
            "app_id": app_id,
            "args": args or [],
            "parent": parent,
            "layout": layout or {"kind": "fill"},
            "lifecycle": lifecycle,
            "linked": linked,
            "wire_channels": wire_channels or [],
            "open_intent": _payload_dict(open_intent),
        }
        print(json.dumps(cmd), flush=True)

    def status_summary(self, summary) -> None:
        """Emit a structured preview/status summary outside a frame."""
        print(json.dumps({
            "type": "status_summary",
            "summary": _payload_dict(summary),
        }), flush=True)

    def pipe_write(self, channel: str, value) -> None:
        """Write a value to a named output pipe channel.

        The host routes this to all connected apps (parent or children).
        Call from any event handler or from on_render via ctx.pipe_write().

        Args:
            channel: Channel name (e.g. "selection", "result", "data")
            value: Any JSON-serializable value (str, int, float, dict, list, etc.)
        """
        print(json.dumps({"type": "pipe_write", "channel": channel, "value": value}), flush=True)

    def submit_feedback(
        self,
        text: str,
        rating: Optional[int] = None,
        category: Optional[str] = None,
    ):
        """
        Submit user feedback about this app. Appended to feedback.jsonl in the app's
        install directory (~/.plexi-alpha/apps/<id>/feedback.jsonl).

        Requires PLEXI_APP_ID and PLEXI_APPS_DIR env vars, which Plexi sets automatically
        when launching apps. If missing, falls back to ~/.plexi/apps/<app_id>/.

        Args:
            text:     Free-form feedback message.
            rating:   Optional 1–5 star rating.
            category: Optional tag (e.g. "bug", "feature", "praise").
        """
        app_id = self._app_id or os.environ.get("PLEXI_APP_ID", "unknown")
        apps_dir = os.environ.get(
            "PLEXI_APPS_DIR",
            str(pathlib.Path.home() / ".plexi" / "apps"),
        )
        feedback_file = pathlib.Path(apps_dir) / app_id / "feedback.jsonl"
        entry: dict = {
            "ts": datetime.now(timezone.utc).isoformat(),
            "text": text,
        }
        if rating is not None:
            entry["rating"] = rating
        if category is not None:
            entry["category"] = category
        try:
            feedback_file.parent.mkdir(parents=True, exist_ok=True)
            with open(feedback_file, "a") as f:
                f.write(json.dumps(entry) + "\n")
        except OSError as e:
            self.warn(f"submit_feedback: could not write to {feedback_file}: {e}")
        self.info(f"feedback submitted: {text[:80]}")

    def event_subscribe(self, kinds=None, scope="workspace"):
        """Subscribe to host events. Matched events arrive via on_event handler.

        Args:
            kinds: List of event kind strings to subscribe to, e.g.
                   ["app_spawned", "pipe_write"]. Pass None or [] for all events.
            scope: One of "workspace" (default), "pane", or "global".
                   Controls which events are filtered by proximity. Global
                   subscriptions are reserved for apps granted the observes
                   capability by the host.

        Note: Phase 0 — the subscribe command is accepted by the host but
        EventData delivery is not yet implemented. Full forwarding lands in
        a follow-up PR.
        """
        print(json.dumps({
            "type": "event_subscribe",
            "kinds": kinds or [],
            "scope": scope,
        }), flush=True)

    def run_create(self, head_task: str, payload: Optional[dict] = None,
                   parent_run_id: Optional[str] = None,
                   notification_title: Optional[str] = None):
        """Create a Run. Plexi will respond with a RunCreated event containing the run_id."""
        cmd: dict = {"type": "run_create", "head_task": head_task, "payload": payload or {}}
        if parent_run_id:
            cmd["parent_run_id"] = parent_run_id
        if notification_title:
            cmd["notification_title"] = notification_title
        print(json.dumps(cmd), flush=True)

    def run_update(self, run_id: str, status: dict, head_task: Optional[str] = None,
                   payload: Optional[dict] = None):
        """Update a Run's status. status is a dict with a 'status' key matching RunStatus."""
        cmd: dict = {"type": "run_update", "run_id": run_id, "status": status}
        if head_task:
            cmd["head_task"] = head_task
        if payload:
            cmd["payload"] = payload
        print(json.dumps(cmd), flush=True)

    def run_complete(self, run_id: str, outcome: str = "success", error: Optional[str] = None):
        """Complete a Run. outcome: 'success' | 'failed' | 'cancelled'."""
        o: dict = {"outcome": outcome}
        if error:
            o["error"] = error
        print(json.dumps({"type": "run_complete", "run_id": run_id, "outcome": o}), flush=True)


def load_manifest(app_file: str = __file__) -> dict:
    """
    Read and return the manifest.toml for the app that calls this.

    Usage:
        manifest = load_manifest(__file__)
        version = manifest.get("app", {}).get("version", "0.0.0")

    Falls back to an empty dict if the manifest is missing or unparseable.
    """
    manifest_path = pathlib.Path(app_file).parent / "manifest.toml"
    if not manifest_path.exists():
        return {}
    text = manifest_path.read_text()
    # Minimal TOML parser for flat [app] tables — avoids a toml dep.
    result: dict = {}
    current_section: dict = result
    for line in text.splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("[") and line.endswith("]"):
            keys = line[1:-1].split(".")
            current_section = result
            for k in keys:
                current_section = current_section.setdefault(k, {})
            continue
        if "=" in line:
            k, _, v = line.partition("=")
            k = k.strip()
            v = v.strip()
            if v.startswith('"') and v.endswith('"'):
                v = v[1:-1]
            elif v == "true":
                v = True  # type: ignore[assignment]
            elif v == "false":
                v = False  # type: ignore[assignment]
            else:
                try:
                    v = int(v)  # type: ignore[assignment]
                except ValueError:
                    pass
            current_section[k] = v
    return result


class RenderContext:
    """
    Passed to the on_render handler. Accumulates draw commands, then flushes.

    All coordinates are in logical pixels within the app surface.
    """

    def __init__(self, width: float, height: float, app_id: str = "",
                 app_state: Optional[dict] = None,
                 render_mode: str = RenderMode.FULL):
        self.width = width
        self.height = height
        self.delta_time: float = 0.0
        self.render_mode: str = render_mode
        self._app_id = app_id
        self._commands: list = []
        # Shared mutable state owned by the App instance (survives across
        # render frames, unlike the RenderContext itself which is recreated
        # per frame). Used by scrollable_list() and other stateful components.
        self._app_state: dict = app_state if app_state is not None else {}
        self._time: float = 0.0
        self._app: Optional["App"] = None  # set by App.run() before on_render

    def rect(self, x: float, y: float, w: float, h: float, fill: str, radius: float = 0.0):
        """Fill a rectangle."""
        self._commands.append({
            "type": "rect", "x": x, "y": y, "w": w, "h": h,
            "fill": fill, "radius": radius,
        })

    def text(self, x: float, y: float, text: str, size: float, color: str,
             monospace: bool = False, bold: bool = False,
             align: str = "left"):
        """
        Draw text at a position.

        `align` controls horizontal anchoring of the text relative to `x`:
          - "left"   (default) — `x` is the left edge of the text
          - "center"           — `x` is the horizontal center
          - "right"            — `x` is the right edge of the text

        Vertical anchoring is always top (`y` = top of the text cell).

        Use the named size constants (TITLE, BODY, CAPTION, HINT, MONO_BODY,
        MONO_SMALL) instead of arbitrary numbers — they give consistent
        legibility across apps and survive future scale tuning.
        """
        cmd = {
            "type": "text", "x": x, "y": y, "text": text, "size": size,
            "color": color, "monospace": monospace, "bold": bold,
        }
        if align and align != "left":
            cmd["align"] = align
        self._commands.append(cmd)

    def text_right(self, x: float, y: float, text: str, size: float, color: str,
                   monospace: bool = False, bold: bool = False):
        """Draw right-aligned text: `x` is the right edge.

        Use for status-bar right columns, header shortcuts, version labels —
        anything anchored to the right side of the pane. Internally sets
        `align="right"` so the host measures exact text width; never do
        `ctx.width - guessed_pixels` yourself.
        """
        self.text(x, y, text, size, color, monospace=monospace, bold=bold, align="right")

    def text_center(self, x: float, y: float, text: str, size: float, color: str,
                    monospace: bool = False, bold: bool = False):
        """Draw center-aligned text: `x` is the horizontal center."""
        self.text(x, y, text, size, color, monospace=monospace, bold=bold, align="center")

    def measure_text(self, text: str, size: float, monospace: bool = False) -> float:
        """
        Approximate the pixel width of `text` at `size`.

        This is an *approximation* — the host (egui) has exact font metrics,
        but the Python SDK can't reach them. For perfect alignment, prefer
        `text_right` / `text_center` (which let the host measure exactly).
        Use `measure_text` only when you need to *reserve* horizontal space
        for padding or wrap a block into a width budget.

        Accuracy target: within ~10% of actual for ASCII text.
        """
        if monospace:
            return len(text) * size * 0.60
        # Proportional: rough average char width ≈ 0.52 * size for mixed ASCII.
        return len(text) * size * 0.52

    # ── Phase 1 components ────────────────────────────────────────────────
    # Composable layout primitives that produce only standard draw commands
    # (rect, text, text_right, text_center). Apps can use these directly or
    # ignore them and draw everything manually.

    def header(
        self,
        title: str,
        subtitle: Optional[str] = None,
        height: float = HEADER_H,
        theme: Optional[Theme] = None,
    ) -> None:
        """
        Draw a surface-filled header bar across the top of the pane.

        Renders `title` at TITLE size in accent bold, left-padded by PAD,
        vertically centered in the bar. If `subtitle` is given, it's drawn
        below the title at HINT size in muted. Returns nothing — call your
        list/body rendering after this.
        """
        t = theme or THEME
        self.rect(0, 0, self.width, height, fill=t.surface)
        if subtitle:
            # Stack title + subtitle, centered as a group.
            block_h = TITLE + 2 + HINT
            title_y = (height - block_h) / 2
            sub_y = title_y + TITLE + 2
            self.text(PAD, title_y, title, size=TITLE, color=t.accent, bold=True)
            self.text(PAD, sub_y, subtitle, size=HINT, color=t.muted)
        else:
            title_y = (height - TITLE) / 2
            self.text(PAD, title_y, title, size=TITLE, color=t.accent, bold=True)

    def status_bar(
        self,
        shortcuts: List[Tuple[str, str]],
        status_msg: Optional[str] = None,
        status_color: Optional[str] = None,
        height: float = 30.0,
        theme: Optional[Theme] = None,
    ) -> None:
        """
        Draw a surface-filled status bar across the bottom of the pane.

        `shortcuts` is a list of (keys, label) tuples. When `status_msg` is
        set, the shortcut row is replaced by the message in `status_color`
        (defaults to theme.green). Default `height` is 30 — STATUS_H=44 is
        too tall for a keyboard-shortcut-only bar.
        """
        t = theme or THEME
        self.rect(0, self.height - height, self.width, height, fill=t.surface)
        text_y = self.height - height + (height - HINT) / 2
        if status_msg:
            color = status_color or t.green
            self.text_center(self.width / 2, text_y, status_msg,
                             size=HINT, color=color)
            return
        if not shortcuts:
            return
        hint = "   ".join(f"{keys}  {label}" for keys, label in shortcuts)
        self.text_center(self.width / 2, text_y, hint, size=HINT, color=t.muted)

    def scrollable_list(
        self,
        list_id: str,
        items: list,
        selected: int,
        row_height: float,
        render_row: Callable,
        x: Optional[float] = None,
        y: Optional[float] = None,
        w: Optional[float] = None,
        h: Optional[float] = None,
        theme: Optional[Theme] = None,
    ) -> None:
        """
        Render a scrollable list with persistent scroll state and a scrollbar.

        Persistent scroll offset is stored on the owning App instance, keyed
        by `list_id`. Symmetric clamping keeps `selected` visible with
        minimal scroll movement (file-explorer behavior).

        `render_row` is called for each visible row with:
            render_row(ctx, item, absolute_index, x, y_row, w, is_selected)
        The callback draws whatever it wants within the row rect.

        Bounding rect defaults (when x/y/w/h are None) assume a standard
        header + status_bar layout: fills the area between the header and
        the status bar with a small top gap.
        """
        t = theme or THEME
        # Defaults: below header, above 30px status bar, full width.
        if x is None:
            x = 0.0
        if w is None:
            w = self.width
        if y is None:
            y = HEADER_H + PAD / 2
        if h is None:
            h = self.height - HEADER_H - 30.0 - PAD

        if h <= 0 or row_height <= 0:
            return

        visible = max(1, int(h / row_height))
        state = self._app_state.setdefault("scroll", {})
        scroll_off = state.get(list_id, 0)

        # Symmetric clamp — scroll in the direction `selected` is pushing.
        if selected < scroll_off:
            scroll_off = selected
        elif selected >= scroll_off + visible:
            scroll_off = selected - visible + 1
        scroll_off = max(0, min(scroll_off, max(0, len(items) - visible)))
        state[list_id] = scroll_off

        # Reserve a small gutter on the right for the scrollbar.
        scroll_gutter = 8.0
        row_w = w - scroll_gutter if len(items) > visible else w

        for i in range(visible):
            idx = scroll_off + i
            if idx >= len(items):
                break
            y_row = y + i * row_height
            is_sel = idx == selected
            render_row(self, items[idx], idx, x, y_row, row_w, is_sel)

        # Scrollbar.
        if len(items) > visible:
            track_x = x + w - scroll_gutter / 2 - 1.5
            track_h = row_height * visible
            thumb_h = max(24.0, track_h * visible / len(items))
            denom = max(1, len(items) - visible)
            thumb_y = y + (track_h - thumb_h) * (scroll_off / denom)
            self.rect(track_x, y, 3, track_h, fill=t.surface, radius=2)
            self.rect(track_x, thumb_y, 3, thumb_h, fill=t.muted, radius=2)

    def scrollable_text(
        self,
        text_id: str,
        lines: List[str],
        scroll_offset: Optional[int] = None,
        line_height: float = MONO_BODY + 4,
        size: Optional[float] = None,
        x: Optional[float] = None,
        y: Optional[float] = None,
        w: Optional[float] = None,
        h: Optional[float] = None,
        monospace: bool = False,
        theme: Optional[Theme] = None,
        color: Optional[str] = None,
    ) -> int:
        """
        Render a scrollable block of pre-wrapped text lines with a right-edge
        scrollbar. Returns the clamped scroll offset considered "current"
        after this render pass.

        Persistent scroll offset is stored on the owning App instance, keyed
        by `text_id`. This shares the same dict namespace as
        `scrollable_list` — a `list_id="notes"` and `text_id="notes"` would
        collide. Choose unique keys per scrollable thing in the app.

        `lines` must be pre-wrapped by the caller (use `ctx.wrap_text()`
        first if you have an unwrapped string). If `scroll_offset` is None
        the persisted value is used; if passed explicitly the persisted
        value is updated to match (so callers can reset it on mode change).

        Bounding rect defaults (when x/y/w/h are None) assume a standard
        header + status_bar layout.
        """
        t = theme or THEME
        txt_color = color or t.fg
        font_size = size if size is not None else max(1.0, line_height - 4)

        if x is None:
            x = PAD
        if w is None:
            w = self.width - PAD * 2
        if y is None:
            y = HEADER_H + PAD
        if h is None:
            h = self.height - 30.0 - PAD - y

        if h <= 0 or line_height <= 0:
            return 0

        state = self._app_state.setdefault("scroll", {})
        if scroll_offset is None:
            offset = state.get(text_id, 0)
        else:
            offset = scroll_offset

        visible_lines = max(1, int(h / line_height))
        max_offset = max(0, len(lines) - visible_lines)
        offset = max(0, min(offset, max_offset))
        state[text_id] = offset

        for i in range(visible_lines):
            idx = offset + i
            if idx >= len(lines):
                break
            self.text(
                x, y + i * line_height, lines[idx],
                size=font_size,
                color=txt_color, monospace=monospace,
            )

        # Scrollbar — matches scrollable_list styling.
        if len(lines) > visible_lines:
            track_x = x + w - PAD / 2 - 3
            track_h = line_height * visible_lines
            thumb_h = max(24.0, track_h * visible_lines / len(lines))
            denom = max(1, len(lines) - visible_lines)
            thumb_y = y + (track_h - thumb_h) * (offset / denom)
            self.rect(track_x, y, 3, track_h, fill=t.surface, radius=2)
            self.rect(track_x, thumb_y, 3, thumb_h, fill=t.muted, radius=2)

        return offset

    def empty_state(
        self,
        title: str,
        subtitle: Optional[str] = None,
        icon_color: Optional[str] = None,
        theme: Optional[Theme] = None,
    ) -> None:
        """
        Draw a centered two-line empty-state message near the pane center.

        `title` renders at BODY size in `icon_color` (defaults to theme.green).
        `subtitle` (optional) renders below at CAPTION size in muted, with
        a 6px gap. The combined block's midpoint is exactly `self.height / 2`.
        """
        t = theme or THEME
        color = icon_color or t.green
        cx = self.width / 2
        if subtitle:
            block_h = BODY + 6 + CAPTION
            block_top = (self.height - block_h) / 2
            title_y = block_top
            sub_y = block_top + BODY + 6
            self.text_center(cx, title_y, title, size=BODY, color=color)
            self.text_center(cx, sub_y, subtitle, size=CAPTION, color=t.muted)
        else:
            title_y = (self.height - BODY) / 2
            self.text_center(cx, title_y, title, size=BODY, color=color)

    def wrap_text(
        self,
        text: str,
        max_width_px: float,
        size: float,
        monospace: bool = False,
    ) -> List[str]:
        """
        Greedy word-wrap `text` into a list of lines that fit within
        `max_width_px` at the given font `size`.

        Uses `measure_text` to decide when to break. Preserves explicit
        newlines as hard breaks. A single word longer than `max_width_px`
        is hard-truncated at the character level (computed against the
        same width factor used by `measure_text`: 0.60 for monospace,
        0.52 for proportional).
        """
        if max_width_px <= 0 or size <= 0:
            return [text]

        factor = 0.60 if monospace else 0.52
        max_chars = max(1, int(max_width_px / (size * factor)))

        out: List[str] = []
        # Preserve explicit newlines as hard breaks.
        for src_line in text.splitlines() or [""]:
            if not src_line:
                out.append("")
                continue
            words = src_line.split(" ")
            current = ""
            for word in words:
                # Hard-truncate any single word that's wider than the budget.
                while self.measure_text(word, size, monospace=monospace) > max_width_px:
                    out.append(word[:max_chars])
                    word = word[max_chars:]
                if not word:
                    continue
                candidate = word if not current else current + " " + word
                if self.measure_text(candidate, size, monospace=monospace) <= max_width_px:
                    current = candidate
                else:
                    if current:
                        out.append(current)
                    current = word
            if current:
                out.append(current)
        return out

    def line(self, x1: float, y1: float, x2: float, y2: float,
             color: str, width: float = 1.0):
        """Draw a horizontal or diagonal line."""
        self._commands.append({
            "type": "line", "x1": x1, "y1": y1, "x2": x2, "y2": y2,
            "color": color, "width": width,
        })

    def drop_target(
        self,
        id: str,
        x: float,
        y: float,
        w: float,
        h: float,
        accept: Optional[list] = None,
        label: Optional[str] = None,
    ):
        """
        Declare a region that can accept dropped files from outside Plexi.

        Drop targets are stateless — you must re-emit this on every render
        frame for the region to remain active.

        Args:
            id:     App-local identifier for this drop zone. Echoed back in
                    the on_drop handler so you can tell which zone received
                    the drop when you declare multiple.
            x, y, w, h: Region in the app's local coordinate space.
            accept: List of file extensions (lowercase, no dot) to accept,
                    e.g. ["png", "jpg", "mp4"]. Empty or None = accept
                    anything. Paths that don't match are filtered out by
                    Plexi before your on_drop handler is called.
            label:  Optional hint text shown by Plexi over the target while
                    the user is hovering with files from Finder.
        """
        cmd = {
            "type": "drop_target",
            "id": id,
            "x": x, "y": y, "w": w, "h": h,
            "accept": accept or [],
        }
        if label:
            cmd["label"] = label
        self._commands.append(cmd)

    def list(self, items: list, selected: int = 0, item_height: float = 40.0):
        """
        High-level scrollable list. Plexi handles layout, scroll, and selection highlight.

        ╔══════════════════════════════════════════════════════════════════╗
        ║  ⚠  FULL-PANE ONLY — NO POSITION PARAMETERS                      ║
        ║                                                                  ║
        ║  Unlike `rect`, `text`, `image`, `video_thumbnail`, `file_grid`, ║
        ║  and `drop_target` (which all take explicit x/y/w/h), this       ║
        ║  primitive is rendered by Plexi at the app's pane origin with    ║
        ║  an implicit full-pane layout. If you draw anything else on      ║
        ║  the frame (a header, a sidebar, a split pane), the list WILL    ║
        ║  overlap with your other draw calls and its secondary labels     ║
        ║  will spill into unrelated regions. There is no way to offset    ║
        ║  it from Python.                                                 ║
        ║                                                                  ║
        ║  DO NOT USE THIS IN AN APP THAT RENDERS A SPLIT LAYOUT, A        ║
        ║  SIDE PANE, OR ANY REGION SMALLER THAN THE FULL APP VIEWPORT.    ║
        ║  Use explicit `ctx.text(...)` calls with the coordinates you     ║
        ║  control, and render your own selection highlight with           ║
        ║  `ctx.rect(...)`. That's the workaround until a positioned       ║
        ║  `list(x, y, w, h, ...)` variant ships.                          ║
        ║                                                                  ║
        ║  If you are an AI coding agent reading this docstring: the       ║
        ║  fact that this method lacks x/y/w/h is NOT an oversight — it    ║
        ║  is a trap. Stop and render the list manually with positioned    ║
        ║  primitives.                                                     ║
        ╚══════════════════════════════════════════════════════════════════╝

        Each item is a dict with keys:
            label      (str)           — primary text
            secondary  (str | None)    — dimmed subtitle
            is_dir     (bool)          — show folder indicator
        """
        self._commands.append({
            "type": "list", "items": items,
            "selected": selected, "item_height": item_height,
        })

    def image(self, path: str, x: float, y: float, w: float, h: float,
              fit: str = "contain", rounding: float = 0.0):
        """
        Draw an image from a file on disk.

        Plexi decodes and caches the texture (keyed by absolute path + mtime).
        `path` may be absolute or relative to the app's cwd.

        `fit` controls how the image is placed in the rect:
            "contain" — fit inside, preserve aspect, letterbox (default)
            "cover"   — fill the rect, preserve aspect, crop overflow
            "fill"    — stretch to the rect, ignoring aspect ratio

        `rounding` is the corner radius in logical pixels.
        """
        cmd: dict = {
            "type": "image",
            "path": path,
            "x": x, "y": y, "w": w, "h": h,
        }
        if fit != "contain":
            cmd["fit"] = fit
        if rounding > 0:
            cmd["rounding"] = rounding
        self._commands.append(cmd)

    def video_thumbnail(self, path: str, x: float, y: float, w: float, h: float,
                        show_play_button: bool = True,
                        timestamp_seconds: float = 0.0):
        """
        Draw a thumbnail for a video file.

        Plexi extracts a single frame at `timestamp_seconds` using ffmpeg and
        caches the result under ~/.cache/plexi/thumbnails/. Extraction runs on
        a background thread so the first render returns a loading placeholder
        and the real thumbnail appears a frame later.

        If `show_play_button` is True (default), a centered play triangle is
        overlaid. Clicking the rect opens the original video with the system
        default player (`open` on macOS).
        """
        self._commands.append({
            "type": "video_thumbnail",
            "path": path,
            "x": x, "y": y, "w": w, "h": h,
            "show_play_button": show_play_button,
            "timestamp_seconds": timestamp_seconds,
        })

    def file_grid(self, x: float, y: float, w: float, h: float,
                  path: Optional[str] = None,
                  filter: Optional[List[str]] = None,
                  paths: Optional[List[str]] = None,
                  item_size: float = 96.0,
                  columns: Optional[int] = None,
                  show_labels: bool = True):
        """
        Draw a grid of files with auto-generated thumbnails.

        Two modes — exactly one must be provided:
            path  + optional filter  — walk a directory non-recursively
            paths                    — explicit list of file paths to show

        `filter` accepts glob patterns or bare extensions, e.g.
        ["*.png", "*.jpg"] or ["png", "mp4"].

        Image files render via the shared image texture cache; video files
        (mp4/mov/webm/mkv/m4v/avi) render via the video thumbnail cache.
        Other file types show a generic icon with the extension label.

        Clicking an item opens it with the system default handler.
        """
        if path is None and paths is None:
            raise ValueError("file_grid requires either 'path' or 'paths'")
        cmd: dict = {
            "type": "file_grid",
            "x": x, "y": y, "w": w, "h": h,
            "item_size": item_size,
            "show_labels": show_labels,
        }
        if path is not None:
            cmd["path"] = path
            if filter:
                cmd["filter"] = filter
        if paths is not None:
            cmd["paths"] = paths
        if columns is not None:
            cmd["columns"] = columns
        self._commands.append(cmd)

    def set_cursor(self, cursor: str):
        """Set the cursor icon for the app pane for this frame.

        Values: 'default', 'pointer', 'grab', 'grabbing', 'crosshair', 'text'.
        Must be re-emitted each frame to persist (cursor resets to 'default' each frame).
        """
        self._commands.append({"type": "set_cursor", "cursor": cursor})

    def mouse_tracking(self, enabled: bool):
        """Enable or disable mouse-move event delivery.

        When enabled, Plexi sends MouseMove events to this app on every frame.
        Off by default — enable only when needed to avoid flooding the pipe.
        This setting persists until changed; you do not need to re-emit each frame.
        """
        self._commands.append({"type": "mouse_tracking", "enabled": enabled})

    def run_in_terminal(self, command: str):
        """Queue a terminal command to run at end of this frame."""
        self._commands.append({"type": "run_in_terminal", "command": command})

    def cd(self, path: str):
        """Queue a cd command for the linked terminal at end of this frame."""
        self._commands.append({"type": "cd", "path": path})

    def log(self, level: str, message: str):
        """Forward a log message to Plexi's logger (level: error|warn|info|debug)."""
        self._commands.append({"type": "log", "level": level, "message": message})

    def info(self, message: str):
        """Log at info level."""
        self.log("info", message)

    def warn(self, message: str):
        """Log at warn level."""
        self.log("warn", message)

    def error(self, message: str):
        """Log at error level."""
        self.log("error", message)

    def debug(self, message: str):
        """Log at debug level."""
        self.log("debug", message)

    def notify(
        self,
        title: str,
        body: Optional[str] = None,
        urgency: str = "low",
        action_type: str = "dismiss",
        action_payload: Optional[dict] = None,
        expires_at: Optional[int] = None,
        visible_after: Optional[int] = None,
    ):
        """
        Raise a notification from inside a render frame.

        urgency: "low" | "medium" | "high"
        action_type: "dismiss" | "focus" | "confirm" | "text_input"
        action_payload: type-dependent dict; for "focus": {"pane_id": int, "fullscreen": bool}
        expires_at: unix timestamp (seconds); notification is dropped if already past
        visible_after: unix timestamp (seconds); defer display until this time
        """
        cmd: dict = {
            "type": "notification",
            "title": title,
            "source_app": self._app_id,
            "urgency": urgency,
            "action_type": action_type,
        }
        if body is not None:
            cmd["body"] = body
        if action_payload is not None:
            cmd["action_payload"] = action_payload
        if expires_at is not None:
            cmd["expires_at"] = expires_at
        if visible_after is not None:
            cmd["visible_after"] = visible_after
        self._commands.append(cmd)

    def notification(
        self,
        title: str,
        body: Optional[str] = None,
        priority: int = 1,
    ):
        """Deprecated: use notify() instead. Kept for backwards compatibility."""
        urgency = {0: "low", 1: "low", 2: "medium", 3: "high"}.get(priority, "low")
        self.notify(title=title, body=body, urgency=urgency)

    def spawn_app(
        self,
        app_id: str,
        args: Optional[List[str]] = None,
        parent: str = "self",
        layout: Optional[dict] = None,
        lifecycle: str = "cascade",
        linked: bool = True,
        wire_channels: Optional[List[str]] = None,
        open_intent: Optional[OpenIntent] = None,
    ):
        """
        Ask Plexi to spawn another app at end of this frame.

        Same shape as `Emitter.spawn_app` — see that method's docstring for
        full field semantics. Use this variant when you need the spawn to
        happen as part of a render pass (queued alongside the frame's draw
        commands); use `emit.spawn_app` from a key/command/mouse handler.
        """
        self._commands.append({
            "type": "spawn_app",
            "app_id": app_id,
            "args": args or [],
            "parent": parent,
            "layout": layout or {"kind": "fill"},
            "lifecycle": lifecycle,
            "linked": linked,
            "wire_channels": wire_channels or [],
            "open_intent": _payload_dict(open_intent),
        })

    def pipe_write(self, channel: str, value) -> None:
        """Write to a named output pipe channel (same as Emitter.pipe_write).

        The host routes this to all connected apps (parent or children).
        Queued with the frame's draw commands and flushed at frame end.

        Args:
            channel: Channel name (e.g. "selection", "result", "data")
            value: Any JSON-serializable value (str, int, float, dict, list, etc.)
        """
        self._commands.append({"type": "pipe_write", "channel": channel, "value": value})

    def status_summary(self, summary) -> None:
        """Emit a structured preview/status summary for the current frame."""
        self._commands.append({
            "type": "status_summary",
            "summary": _payload_dict(summary),
        })

    def code_editor(self, editor_id: str, lines: list, cursor_line: int,
                    cursor_col: int, on_change, x: float, y: float,
                    w: float, h: float, focused: bool = True) -> dict:
        """
        Draw a multi-line Python code editor with syntax highlighting.

        lines: list[str] — code content, one string per line (app owns state)
        cursor_line, cursor_col — cursor position (app owns)
        on_change(new_lines, new_cursor_line, new_cursor_col) — called on edits
        focused — whether this editor captures key events

        Keyboard handling is routed through App._editor_states when focused=True.
        Requires ctx._app to be set (done automatically by App.run()).
        """
        _app = self._app

        # Register with App for key routing
        if _app is not None and focused:
            _app._editor_states[editor_id] = {
                "lines": lines,
                "cursor_line": cursor_line,
                "cursor_col": cursor_col,
                "on_change": on_change,
            }
            _app._focused_editor_id = editor_id

        font_size = 13.0
        line_h = font_size + 4.0
        line_num_w = 40.0
        visible_lines = max(1, int(h / line_h))
        scroll_line = _app._editor_scroll.get(editor_id, 0) if _app else 0

        # Clamp scroll
        max_scroll = max(0, len(lines) - visible_lines)
        scroll_line = min(scroll_line, max_scroll)
        if _app:
            _app._editor_scroll[editor_id] = scroll_line

        # Background
        self.rect(x, y, w, h, fill="#1e1e2e", radius=4.0)

        # Line number column bg
        self.rect(x, y, line_num_w, h, fill="#181825", radius=0.0)

        # Render visible lines
        for rel_idx in range(visible_lines):
            line_idx = scroll_line + rel_idx
            if line_idx >= len(lines):
                break
            ly = y + rel_idx * line_h

            # Current line highlight
            if focused and line_idx == cursor_line:
                self.rect(x, ly, w, line_h, fill="#2a2a3e", radius=0.0)

            # Line number
            line_num_str = str(line_idx + 1)
            lnx = x + line_num_w - len(line_num_str) * 7 - 4
            self.text(lnx, ly + 2, line_num_str, size=font_size,
                      color="#45475a", monospace=True)

            # Syntax-highlighted code
            line_text = lines[line_idx]
            self._draw_syntax_line(x + line_num_w + 4, ly + 2, line_text,
                                   font_size)

        # Cursor (blink)
        if focused and self._time % 1.0 < 0.5:
            rel = cursor_line - scroll_line
            if 0 <= rel < visible_lines:
                # Approximate char width for monospace
                char_w = font_size * 0.6
                cur_x = x + line_num_w + 4 + cursor_col * char_w
                cur_y = y + rel * line_h
                self.rect(cur_x, cur_y + 1, 2.0, line_h - 2, fill="#cdd6f4")

        return {"x": x, "y": y, "w": w, "h": h}

    # Syntax color constants
    _KW = frozenset({
        'def', 'class', 'return', 'import', 'from', 'if', 'else', 'elif',
        'for', 'while', 'in', 'not', 'and', 'or', 'True', 'False', 'None',
        'try', 'except', 'finally', 'with', 'as', 'pass', 'raise', 'yield',
        'lambda', 'global', 'nonlocal', 'break', 'continue', 'is', 'del',
    })
    _COL_KW    = "#569cd6"
    _COL_STR   = "#ce9178"
    _COL_CMT   = "#6a9955"
    _COL_NUM   = "#b5cea8"
    _COL_PLAIN = "#d4d4d4"

    def _draw_syntax_line(self, x: float, y: float, line: str, size: float):
        """Draw a single code line with basic syntax highlighting."""
        char_w = size * 0.6

        # Detect full-line comment
        stripped = line.lstrip()
        if stripped.startswith("#"):
            self.text(x, y, line, size=size, color=self._COL_CMT, monospace=True)
            return

        # Split into (token, color) segments
        segments: list = []
        i = 0
        in_str = False
        str_char = ""
        tok_start = 0

        def flush_plain(end: int):
            nonlocal tok_start
            chunk = line[tok_start:end]
            if chunk:
                buf = ""
                for ch in chunk:
                    if ch.isalnum() or ch in ("_", "."):
                        buf += ch
                    else:
                        if buf:
                            if buf in self._KW:
                                color = self._COL_KW
                            elif buf.replace(".", "", 1).isdigit():
                                color = self._COL_NUM
                            else:
                                color = self._COL_PLAIN
                            segments.append((buf, color))
                            buf = ""
                        segments.append((ch, self._COL_PLAIN))
                if buf:
                    if buf in self._KW:
                        color = self._COL_KW
                    elif buf.replace(".", "", 1).isdigit():
                        color = self._COL_NUM
                    else:
                        color = self._COL_PLAIN
                    segments.append((buf, color))
            tok_start = end

        while i < len(line):
            ch = line[i]
            if in_str:
                if ch == str_char and (i == 0 or line[i - 1] != "\\"):
                    str_token = line[tok_start:i + 1]
                    segments.append((str_token, self._COL_STR))
                    in_str = False
                    tok_start = i + 1
                i += 1
                continue
            if ch in ('"', "'"):
                flush_plain(i)
                in_str = True
                str_char = ch
                tok_start = i
                i += 1
                continue
            if ch == "#":
                flush_plain(i)
                segments.append((line[i:], self._COL_CMT))
                i = len(line)
                tok_start = i
                continue
            i += 1

        flush_plain(len(line))

        cx = x
        for tok, color in segments:
            self.text(cx, y, tok, size=size, color=color, monospace=True)
            cx += len(tok) * char_w

    def _flush(self):
        for cmd in self._commands:
            print(json.dumps(cmd), flush=True)
        print(json.dumps({"type": "frame_done"}), flush=True)
        self._commands.clear()

    # ─── v2.1 UI primitives ────────────────────────────────────────────────────

    def measure_text_exact(self, text: str, size: float, monospace: bool = False, bold: bool = False) -> TextMetrics:
        """
        Request exact text measurement from Plexi. Blocking — waits for TextMetrics reply.
        Results are cached per-frame (cache clears each frame).
        """
        if not hasattr(self, "_measure_cache"):
            self._measure_cache: dict = {}
        if not hasattr(self, "_measure_req_id"):
            self._measure_req_id: int = 0
        if not hasattr(self, "_pending_events"):
            self._pending_events: list = []

        cache_key = (text, size, monospace, bold)
        if cache_key in self._measure_cache:
            return self._measure_cache[cache_key]

        self._measure_req_id += 1
        req_id = self._measure_req_id

        # Flush current commands first so measure request is sent in-band.
        for cmd in self._commands:
            print(json.dumps(cmd), flush=True)
        self._commands.clear()

        print(json.dumps({
            "type": "measure_text",
            "request_id": req_id,
            "text": text,
            "size": size,
            "monospace": monospace,
            "bold": bold,
        }), flush=True)

        # Read stdin until we get the matching TextMetrics reply.
        for line in sys.stdin:
            line = line.strip()
            if not line:
                continue
            try:
                event = json.loads(line)
            except json.JSONDecodeError:
                continue
            if event.get("type") == "text_metrics" and event.get("request_id") == req_id:
                metrics = TextMetrics(
                    width=event.get("width", 0.0),
                    height=event.get("height", 0.0),
                    ascent=event.get("ascent", 0.0),
                )
                self._measure_cache[cache_key] = metrics
                return metrics
            else:
                self._pending_events.append(event)

        return TextMetrics(width=size * len(text) * 0.6, height=size, ascent=size * 0.8)

    def viewport(self, viewport_id: str, content_fn: Callable, zoom: float = 1.0,
                 pan: Optional[tuple] = None, x: Optional[float] = None,
                 y: Optional[float] = None, w: Optional[float] = None,
                 h: Optional[float] = None, min_zoom: float = 0.1,
                 max_zoom: float = 10.0, on_pan=None, on_zoom=None):
        """Render content inside a transformed viewport with zoom + pan."""
        zoom = max(min_zoom, min(max_zoom, zoom))
        tx = pan[0] if pan else 0.0
        ty = pan[1] if pan else 0.0
        self._commands.append({
            "type": "push_transform",
            "scale_x": zoom,
            "scale_y": zoom,
            "translate_x": tx,
            "translate_y": ty,
        })
        content_fn(self)
        self._commands.append({"type": "pop_transform"})
        return {"x": x or 0, "y": y or 0, "w": w or self.width, "h": h or self.height}

    def text_input(self, input_id: str, value: str, on_change: Callable,
                   cursor: Optional[int] = None, placeholder: Optional[str] = None,
                   max_length: Optional[int] = None, size: Optional[float] = None,
                   x: float = 0, y: float = 0, w: Optional[float] = None,
                   bg: Optional[str] = None, fg: Optional[str] = None,
                   focused: bool = True) -> dict:
        """Draw a single-line text input widget. Returns the bounding rect dict."""
        font_size = size or 14.0
        width = w or (self.width - x * 2)
        height = font_size + 12.0
        bg_color = bg or "#2a2a3e"
        fg_color = fg or "#cdd6f4"
        placeholder_color = "#6c7086"

        self.rect(x, y, width, height, fill=bg_color, radius=4.0)
        display_text = value if value else (placeholder or "")
        display_color = fg_color if value else placeholder_color
        self.text(x + 6, y + (height - font_size) / 2, display_text, size=font_size, color=display_color)

        if focused and hasattr(self, "_time") and self._time % 1.0 < 0.5:
            cur_pos = cursor if cursor is not None else len(value)
            cursor_x = x + 6 + cur_pos * font_size * 0.6
            cursor_h = font_size + 2
            cursor_y = y + (height - cursor_h) / 2
            self.rect(cursor_x, cursor_y, 2.0, cursor_h, fill=fg_color)

        return {"x": x, "y": y, "w": width, "h": height}

    def tabs(self, tab_id: str, tabs: list, selected: str,
             on_change=None, height: float = 36, x: float = 0,
             y: Optional[float] = None, w: Optional[float] = None) -> dict:
        """Draw a tab bar. tabs is a list of (key, label) tuples. Returns bounding rect."""
        y_pos = y if y is not None else 0.0
        total_w = w or self.width
        tab_w = total_w / max(len(tabs), 1)
        accent = "#89b4fa"
        bg_selected = "#313244"
        bg_normal = "#1e1e2e"
        fg = "#cdd6f4"

        for i, (key, label) in enumerate(tabs):
            tx = x + i * tab_w
            is_sel = key == selected
            self.rect(tx, y_pos, tab_w, height, fill=bg_selected if is_sel else bg_normal)
            self.text(tx + tab_w / 2 - len(label) * 4, y_pos + (height - 14) / 2, label, size=14.0, color=fg)
            if is_sel:
                self.rect(tx, y_pos + height - 2, tab_w, 2.0, fill=accent)

        return {"x": x, "y": y_pos, "w": total_w, "h": height}

    def grid(self, grid_id: str, cols: int, rows: int, render_cell: Callable,
             x: Optional[float] = None, y: Optional[float] = None,
             w: Optional[float] = None, h: Optional[float] = None,
             gap: float = 4.0):
        """Draw a uniform grid. render_cell(ctx, col, row, cx, cy, cw, ch) is called per cell."""
        gx = x or 0.0
        gy = y or 0.0
        gw = w or self.width
        gh = h or self.height
        cell_w = (gw - gap * (cols - 1)) / max(cols, 1)
        cell_h = (gh - gap * (rows - 1)) / max(rows, 1)

        for row in range(rows):
            for col in range(cols):
                cx = gx + col * (cell_w + gap)
                cy = gy + row * (cell_h + gap)
                render_cell(self, col, row, cx, cy, cell_w, cell_h)

    def modal(self, modal_id: str, visible: bool, content_fn: Callable,
              width: float = 400, height: float = 200,
              backdrop_alpha: int = 128, on_dismiss=None):
        """Draw a centered modal dialog with a semi-transparent backdrop."""
        if not visible:
            return
        alpha_hex = format(backdrop_alpha, '02x')
        self.rect(0, 0, self.width, self.height, fill=f"#000000{alpha_hex}")
        mx = (self.width - width) / 2
        my = (self.height - height) / 2
        self.rect(mx, my, width, height, fill="#1e1e2e", radius=8.0)
        content_fn(self, mx, my, width, height)


class App:
    """
    Base class for Plexi apps. Register event handlers via decorators.

    Handlers:
        @app.on_init         fn() — inspect app.protocol_version, open_intent,
                               capability_manifest, and render_mode
        @app.on_render        fn(ctx: RenderContext)
        @app.on_key           fn(key: str, mods: dict, emit: Emitter)
        @app.on_suspend       fn() — host is pausing the app
        @app.on_resume        fn() — host is resuming the app
        @app.on_click         fn(x: float, y: float, button: str, emit: Emitter)
        @app.on_command       fn(text: str, emit: Emitter)
        @app.on_resize        fn(width: float, height: float)
        @app.on_get_state     fn() -> dict with keys: user_state, derived, session, persistent
        @app.on_set_state     fn(state: dict) — restore app from state buckets
        @app.on_drop          fn(target_id: str, paths: list[str], emit: Emitter)
        @app.on_mouse_down    fn(x: float, y: float, button: str, emit: Emitter)
        @app.on_mouse_up      fn(x: float, y: float, button: str, emit: Emitter)
        @app.on_mouse_move    fn(x: float, y: float, emit: Emitter)
        @app.on_scroll        fn(x: float, y: float, delta_x: float, delta_y: float, emit: Emitter)
    """

    def __init__(self, app_id: str = "", auto_min_size: bool = True,
                 min_protocol_version: int = 0):
        self.width: float = 800.0
        self.height: float = 600.0
        self.delta_time: float = 0.0
        self.protocol_version: int = 0
        """Protocol version sent by the host in the Init event. 0 means the
        host did not send a version (v1 host). Set once at startup."""
        self.render_mode: str = RenderMode.FULL
        self.open_intent: Optional[OpenIntent] = None
        self.capability_manifest: dict = {}
        self._min_protocol_version: int = min_protocol_version
        self._on_init: Optional[Callable] = None
        self._on_render: Optional[Callable] = None
        self._on_key: Optional[Callable] = None
        self._on_suspend: Optional[Callable] = None
        self._on_resume: Optional[Callable] = None
        self._on_click: Optional[Callable] = None
        self._on_command: Optional[Callable] = None
        self._on_resize: Optional[Callable] = None
        self._on_get_state: Optional[Callable] = None
        self._on_set_state: Optional[Callable] = None
        self._on_drop: Optional[Callable] = None
        self._on_mouse_down: Optional[Callable] = None
        self._on_mouse_up: Optional[Callable] = None
        self._on_mouse_move: Optional[Callable] = None
        self._on_scroll: Optional[Callable] = None
        self._on_pipe_data: Optional[Callable] = None
        self._on_event: Optional[Callable] = None
        self._on_run_created: Optional[Callable] = None
        self.open_intent: Optional[OpenIntent] = None
        self._emitter = Emitter(app_id=app_id)
        # Breakpoints: list of (min_width, min_height, render_fn).
        # Walked in descending area order on each render to pick the
        # most specific match whose constraints fit the current pane.
        self._breakpoints: List[Tuple[float, float, Callable]] = []
        # Minimum pane size before the SDK draws its built-in
        # "too small" fallback frame (bypassing the user render fn).
        # Values of 0 mean "no floor on this axis".
        self._min_width: float = 0.0
        self._min_height: float = 0.0
        self._auto_min_size: bool = auto_min_size
        # Palette used by the auto min-size fallback. Apps can override
        # per-instance via `set_min_size_colors` if they care.
        self._min_size_bg: str = "#0a0a0a"
        self._min_size_fg: str = "#888888"
        self._min_size_accent: str = "#ffffff"
        # Persistent scroll state for SDK-level components (e.g.
        # ctx.scrollable_list). Keyed by the app-supplied list_id. Lives on
        # the App because RenderContext is recreated per frame.
        self._scroll_state: dict[str, int] = {}
        # Code editor state — keyed by editor_id
        self._editor_states: dict = {}
        self._editor_scroll: dict = {}   # editor_id -> scroll_line (int)
        self._focused_editor_id: Optional[str] = None
        self._render_time: float = 0.0

    def on_render(self, fn: Callable) -> Callable:
        self._on_render = fn
        return fn

    def on_init(self, fn: Callable) -> Callable:
        """Register a handler for the Init event (called once at startup)."""
        self._on_init = fn
        return fn

    def breakpoint(
        self, min_width: float = 0.0, min_height: float = 0.0
    ) -> Callable:
        """
        Register a render function for a specific minimum pane size.

        Apps can stack multiple `@app.breakpoint(...)` handlers; on each
        render event the SDK picks the most specific breakpoint whose
        constraints fit the current pane (largest min_width * min_height
        area that still satisfies `width >= min_width AND height >= min_height`).
        If none match, a breakpoint registered with no constraints (the
        default 0x0 fallback) is called.

        Example:

            @app.breakpoint(min_width=800, min_height=500)
            def render_full(ctx):
                ...

            @app.breakpoint(min_width=400)
            def render_compact(ctx):
                ...

            @app.breakpoint()  # fallback — always matches
            def render_fallback(ctx):
                ...

        Mutually exclusive with `@app.on_render`. Registering both raises
        a RuntimeError at app startup.
        """
        def decorator(fn: Callable) -> Callable:
            self._breakpoints.append((float(min_width), float(min_height), fn))
            return fn
        return decorator

    def set_min_size(self, min_width: float, min_height: float) -> None:
        """
        Set the minimum pane size programmatically.

        When the pane is smaller than this on either axis and `auto_min_size`
        is enabled, the SDK draws a built-in "too small" fallback frame
        (background + label + directional arrow + current size) instead of
        calling any user render function.

        Useful for apps that compute their min size at runtime (e.g. based
        on font metrics) rather than declaring it in manifest.toml.
        """
        self._min_width = float(min_width)
        self._min_height = float(min_height)

    def set_min_size_colors(
        self,
        bg: Optional[str] = None,
        fg: Optional[str] = None,
        accent: Optional[str] = None,
    ) -> None:
        """
        Override the palette used by the auto min-size fallback frame.

        The SDK has no access to live host theme tokens, so defaults are
        hardcoded (`#0a0a0a` background, `#888888` label, `#ffffff` arrow).
        Apps that care about matching a custom theme can override any
        subset of these.
        """
        if bg is not None:
            self._min_size_bg = bg
        if fg is not None:
            self._min_size_fg = fg
        if accent is not None:
            self._min_size_accent = accent

    def _load_manifest_layout(self) -> None:
        """
        Read `[app.layout]` from manifest.toml (if present) and populate
        `_min_width` / `_min_height` unless already set programmatically.
        Best-effort — missing or unparseable manifests are silently ignored.
        """
        if self._min_width or self._min_height:
            return  # explicit set_min_size wins over manifest
        try:
            # Walk up from the caller's file to find manifest.toml.
            main_file = getattr(sys.modules.get("__main__"), "__file__", None)
            if not main_file:
                return
            manifest = load_manifest(main_file)
            layout = manifest.get("app", {}).get("layout", {})
            if isinstance(layout, dict):
                mw = layout.get("min_width", 0)
                mh = layout.get("min_height", 0)
                try:
                    self._min_width = float(mw)
                    self._min_height = float(mh)
                except (TypeError, ValueError):
                    pass
        except Exception:
            # Never let manifest errors kill the app — log-only fallback.
            pass

    def _render_min_size_fallback(self, width: float, height: float) -> None:
        """
        Emit the built-in "pane too small" frame directly as JSON draw
        commands. Does NOT go through RenderContext (the user render fn
        is never called in this code path).

        Draws: background rect, centered "min size: W x H" label,
        directional arrow(s) pointing toward the axes that need to grow,
        and a dim "current: w x h" subtitle.
        """
        bg = self._min_size_bg
        fg = self._min_size_fg
        accent = self._min_size_accent
        min_w = self._min_width
        min_h = self._min_height

        needs_width = width < min_w
        needs_height = height < min_h

        if needs_width and needs_height:
            arrow = "\u2198"  # ↘
        elif needs_width:
            arrow = "\u2192"  # →
        elif needs_height:
            arrow = "\u2193"  # ↓
        else:
            arrow = ""

        label = f"min size: {int(min_w)} x {int(min_h)}"
        current = f"current: {int(width)} x {int(height)}"

        # Rough centering — we don't have text metrics, so estimate with
        # a fixed per-char width of 0.55 * font_size. Good enough for a
        # fallback that only shows when the pane is too small anyway.
        label_size = 14.0
        current_size = 11.0
        arrow_size = 32.0
        label_w = len(label) * label_size * 0.55
        current_w = len(current) * current_size * 0.55
        arrow_w = len(arrow) * arrow_size * 0.55

        cx = width / 2.0
        cy = height / 2.0
        label_x = max(0.0, cx - label_w / 2.0)
        current_x = max(0.0, cx - current_w / 2.0)
        arrow_x = max(0.0, cx - arrow_w / 2.0)

        label_y = max(0.0, cy - 30.0)
        arrow_y = cy
        current_y = cy + 40.0

        cmds = [
            {"type": "rect", "x": 0.0, "y": 0.0, "w": width, "h": height,
             "fill": bg, "radius": 0.0},
            {"type": "text", "x": label_x, "y": label_y, "text": label,
             "size": label_size, "color": fg,
             "monospace": False, "bold": True},
        ]
        if arrow:
            cmds.append({
                "type": "text", "x": arrow_x, "y": arrow_y, "text": arrow,
                "size": arrow_size, "color": accent,
                "monospace": False, "bold": False,
            })
        cmds.append({
            "type": "text", "x": current_x, "y": current_y, "text": current,
            "size": current_size, "color": fg,
            "monospace": False, "bold": False,
        })
        cmds.append({"type": "frame_done"})
        for cmd in cmds:
            print(json.dumps(cmd), flush=True)

    def _pick_breakpoint(self, width: float, height: float) -> Optional[Callable]:
        """
        Walk breakpoints sorted by (min_width * min_height) descending and
        return the first one whose `width >= min_width AND height >= min_height`.
        Falls back to a zero-constraint breakpoint if present. Returns None
        if no breakpoints are registered at all.
        """
        if not self._breakpoints:
            return None
        ordered = sorted(
            self._breakpoints,
            key=lambda b: (b[0] * b[1], b[0], b[1]),
            reverse=True,
        )
        for min_w, min_h, fn in ordered:
            if width >= min_w and height >= min_h:
                return fn
        # No match — look for a 0x0 fallback explicitly.
        for min_w, min_h, fn in self._breakpoints:
            if min_w == 0.0 and min_h == 0.0:
                return fn
        return None

    def on_key(self, fn: Callable) -> Callable:
        self._on_key = fn
        return fn

    def on_suspend(self, fn: Callable) -> Callable:
        """Register a handler for host Suspend events."""
        self._on_suspend = fn
        return fn

    def on_resume(self, fn: Callable) -> Callable:
        """Register a handler for host Resume events."""
        self._on_resume = fn
        return fn

    def on_click(self, fn: Callable) -> Callable:
        self._on_click = fn
        return fn

    def on_command(self, fn: Callable) -> Callable:
        self._on_command = fn
        return fn

    def on_resize(self, fn: Callable) -> Callable:
        self._on_resize = fn
        return fn

    def on_get_state(self, fn: Callable) -> Callable:
        """Register handler for get_state requests. Should return a dict with
        keys: user_state, derived, session, persistent."""
        self._on_get_state = fn
        return fn

    def on_set_state(self, fn: Callable) -> Callable:
        """Register handler for set_state requests. Receives a dict with
        keys: user_state, derived, session, persistent."""
        self._on_set_state = fn
        return fn

    def on_drop(self, fn: Callable) -> Callable:
        """Register a handler for file drops onto declared drop targets.

        The handler receives (target_id: str, paths: list[str], emit: Emitter).
        `target_id` matches the id you passed to ctx.drop_target(). `paths`
        are absolute host filesystem paths already filtered by the target's
        accept list.
        """
        self._on_drop = fn
        return fn

    def on_mouse_down(self, fn: Callable) -> Callable:
        """Register a handler for mouse button presses.

        The handler receives (x: float, y: float, button: str, emit: Emitter).
        `button` is one of 'left', 'right', 'middle'.
        """
        self._on_mouse_down = fn
        return fn

    def on_mouse_up(self, fn: Callable) -> Callable:
        """Register a handler for mouse button releases.

        The handler receives (x: float, y: float, button: str, emit: Emitter).
        `button` is one of 'left', 'right', 'middle'.
        """
        self._on_mouse_up = fn
        return fn

    def on_mouse_move(self, fn: Callable) -> Callable:
        """Register a handler for mouse movement.

        The handler receives (x: float, y: float, emit: Emitter).
        Only called when mouse_tracking = true in manifest capabilities or after
        the app emits a MouseTracking draw command.
        """
        self._on_mouse_move = fn
        return fn

    def on_scroll(self, fn: Callable) -> Callable:
        """Register a handler for scroll wheel / trackpad scroll events.

        The handler receives (x: float, y: float, delta_x: float, delta_y: float, emit: Emitter).
        `x, y` is the cursor position; `delta_x, delta_y` is the scroll amount in logical pixels.
        """
        self._on_scroll = fn
        return fn

    def on_pipe_data(self, fn: Callable) -> Callable:
        """Register a handler for pipe data received from connected apps.

        Called when another app (parent or child) writes to a channel that this
        app is connected to via a spawn relationship.

        Handler signature: fn(from_app: str, channel: str, value, emit: Emitter)

        Args:
            from_app: The app_id of the app that wrote the value.
            channel:  The channel name the value was written to.
            value:    The JSON value (str, int, float, dict, list, or None).
            emit:     An Emitter to respond or trigger further actions.
        """
        self._on_pipe_data = fn
        return fn

    def on_event(self, fn: Callable) -> Callable:
        """Register a handler for host events received after EventSubscribe.

        Called when the host delivers an EventData event matching an active
        subscription (initiated via emit.event_subscribe(...)).

        Handler signature: fn(kind: str, payload: dict, emit: Emitter)

        Args:
            kind:    The event kind string (e.g. "app_spawned", "pipe_write").
            payload: The full event payload as a dict.
            emit:    An Emitter to respond or trigger further actions.

        Note: Phase 0 — delivery requires a prior EventSubscribe call. The host
        does not yet forward EventData events; full delivery lands in a follow-up PR.
        """
        self._on_event = fn
        return fn

    def on_run_created(self, fn: Callable) -> Callable:
        """Register a handler for RunCreated responses. fn(run_id: str, emit: Emitter)."""
        self._on_run_created = fn
        return fn

    def _route_key_to_editor(self, editor_id: str, es: dict, key: str, mods: dict) -> bool:
        """Handle a key event for a focused code editor. Returns True if consumed."""
        lines = [l for l in es["lines"]]  # shallow copy to mutate
        cl = es["cursor_line"]
        cc = es["cursor_col"]
        on_change = es["on_change"]
        scroll = self._editor_scroll.get(editor_id, 0)

        def clamp_col():
            nonlocal cc
            if cl < len(lines):
                cc = min(cc, len(lines[cl]))

        consumed = True
        shift = mods.get("shift", False)

        if key == "Enter":
            current = lines[cl]
            lines[cl] = current[:cc]
            lines.insert(cl + 1, current[cc:])
            cl += 1
            cc = 0
        elif key == "Backspace":
            if cc > 0:
                lines[cl] = lines[cl][:cc - 1] + lines[cl][cc:]
                cc -= 1
            elif cl > 0:
                prev_len = len(lines[cl - 1])
                lines[cl - 1] = lines[cl - 1] + lines[cl]
                del lines[cl]
                cl -= 1
                cc = prev_len
        elif key == "Delete":
            if cc < len(lines[cl]):
                lines[cl] = lines[cl][:cc] + lines[cl][cc + 1:]
            elif cl < len(lines) - 1:
                lines[cl] = lines[cl] + lines[cl + 1]
                del lines[cl + 1]
        elif key == "ArrowLeft":
            if cc > 0:
                cc -= 1
            elif cl > 0:
                cl -= 1
                cc = len(lines[cl])
        elif key == "ArrowRight":
            if cl < len(lines) and cc < len(lines[cl]):
                cc += 1
            elif cl < len(lines) - 1:
                cl += 1
                cc = 0
        elif key == "ArrowUp":
            if cl > 0:
                cl -= 1
                clamp_col()
                if cl < scroll:
                    scroll = cl
                    self._editor_scroll[editor_id] = scroll
        elif key == "ArrowDown":
            if cl < len(lines) - 1:
                cl += 1
                clamp_col()
        elif key == "Home":
            cc = 0
        elif key == "End":
            if cl < len(lines):
                cc = len(lines[cl])
        elif key == "Tab":
            lines[cl] = lines[cl][:cc] + "    " + lines[cl][cc:]
            cc += 4
        elif key == "a" and mods.get("command", False):
            cl = 0
            cc = 0
        elif len(key) == 1:
            lines[cl] = lines[cl][:cc] + key + lines[cl][cc:]
            cc += 1
        else:
            consumed = False

        if consumed:
            clamp_col()
            # Auto-scroll to keep cursor visible
            # (visible_lines unknown here; use a default 20 — app will re-clamp)
            if cl < scroll:
                self._editor_scroll[editor_id] = cl
            elif cl >= scroll + 20:
                self._editor_scroll[editor_id] = cl - 19
            on_change(lines, cl, cc)

        return consumed

    def _handle_get_state(self):
        """Respond to a get_state request from Plexi."""
        if self._on_get_state:
            state = self._on_get_state()
        else:
            state = {}
        # Ensure all four buckets exist.
        result = {
            "type": "state",
            "user_state": state.get("user_state", {}),
            "derived": state.get("derived", {}),
            "session": state.get("session", {}),
            "persistent": state.get("persistent", {}),
        }
        print(json.dumps(result), flush=True)

    def _handle_set_state(self, event: dict):
        """Handle a set_state request from Plexi."""
        if self._on_set_state:
            self._on_set_state({
                "user_state": event.get("user_state", {}),
                "derived": event.get("derived", {}),
                "session": event.get("session", {}),
                "persistent": event.get("persistent", {}),
            })

    def run(self):
        """Start the event loop. Blocks until Plexi sends Shutdown."""
        sys.stdout.reconfigure(line_buffering=True)  # type: ignore[attr-defined]

        # Validate mutually exclusive render registration.
        if self._on_render is not None and self._breakpoints:
            raise RuntimeError(
                "plexi_sdk: @app.on_render and @app.breakpoint(...) are "
                "mutually exclusive — use one or the other, not both."
            )

        # Pull min-size from the manifest (if set_min_size was not called).
        self._load_manifest_layout()

        for line in sys.stdin:
            line = line.strip()
            if not line:
                continue

            try:
                event = json.loads(line)
            except json.JSONDecodeError:
                continue

            event_type = event.get("type", "")

            if event_type == "init":
                self.width = event.get("width", self.width)
                self.height = event.get("height", self.height)
                self.protocol_version = event.get("protocol_version", 0)
                raw_intent = event.get("open_intent")
                if raw_intent:
                    self.open_intent = OpenIntent.from_dict(raw_intent)
                self.render_mode = event.get("mode", event.get("render_mode", RenderMode.FULL))
                self.open_intent = OpenIntent.from_value(event.get("open_intent"))
                capability_manifest = event.get("capability_manifest", {})
                self.capability_manifest = (
                    capability_manifest if isinstance(capability_manifest, dict) else {}
                )
                if self._min_protocol_version > 0 and self.protocol_version < self._min_protocol_version:
                    print(
                        f"plexi_sdk: host protocol version {self.protocol_version} is below "
                        f"this app's minimum required version {self._min_protocol_version}. "
                        f"Please update your Plexi host.",
                        file=sys.stderr,
                    )
                    sys.exit(1)
                if self._on_init:
                    self._on_init()

            elif event_type == "resize":
                self.width = event.get("width", self.width)
                self.height = event.get("height", self.height)
                if self._on_resize:
                    self._on_resize(self.width, self.height)

            elif event_type == "render":
                self.width = event.get("width", self.width)
                self.height = event.get("height", self.height)
                self.delta_time = event.get("delta_time", 0.0)
                self.render_mode = event.get("mode", event.get("render_mode", self.render_mode))
                # Auto min-size fallback: if the pane is smaller than
                # our declared floor on either axis, draw the built-in
                # "too small" frame and skip user rendering entirely.
                if (
                    self._auto_min_size
                    and (self._min_width > 0 or self._min_height > 0)
                    and (
                        self.width < self._min_width
                        or self.height < self._min_height
                    )
                ):
                    self._render_min_size_fallback(self.width, self.height)
                    continue
                self._render_time += self.delta_time
                # Clear focused editor each frame — re-registered by code_editor() calls
                self._focused_editor_id = None
                self._editor_states.clear()
                ctx = RenderContext(
                    self.width, self.height,
                    app_id=self._emitter._app_id,
                    app_state={"scroll": self._scroll_state},
                    render_mode=self.render_mode,
                )
                ctx.delta_time = self.delta_time
                ctx._time = self._render_time
                ctx._app = self  # allow code_editor to register state
                # Breakpoint dispatch (if registered) overrides on_render.
                if self._breakpoints:
                    fn = self._pick_breakpoint(self.width, self.height)
                    if fn is not None:
                        fn(ctx)
                elif self._on_render:
                    self._on_render(ctx)
                ctx._flush()

            elif event_type == "key":
                key = event.get("key", "")
                mods = event.get("modifiers", {})
                # Route to focused editor first, unless a Cmd-key combo
                cmd_held = mods.get("command", False) or mods.get("ctrl", False)
                editor_consumed = False
                if self._focused_editor_id and not cmd_held:
                    eid = self._focused_editor_id
                    es = self._editor_states.get(eid)
                    if es:
                        editor_consumed = self._route_key_to_editor(eid, es, key, mods)
                if not editor_consumed and self._on_key:
                    self._on_key(key, mods, self._emitter)

            elif event_type == "suspend":
                if self._on_suspend:
                    self._on_suspend()

            elif event_type == "resume":
                if self._on_resume:
                    self._on_resume()

            elif event_type == "click":
                if self._on_click:
                    self._on_click(
                        event.get("x", 0.0),
                        event.get("y", 0.0),
                        event.get("button", "primary"),
                        self._emitter,
                    )

            elif event_type == "command":
                if self._on_command:
                    self._on_command(event.get("text", ""), self._emitter)

            elif event_type == "get_state":
                self._handle_get_state()

            elif event_type == "set_state":
                self._handle_set_state(event)

            elif event_type == "drop":
                if self._on_drop:
                    self._on_drop(
                        event.get("target_id", ""),
                        event.get("paths", []),
                        self._emitter,
                    )

            elif event_type == "mouse_down":
                if self._on_mouse_down:
                    self._on_mouse_down(
                        event.get("x", 0.0),
                        event.get("y", 0.0),
                        event.get("button", "left"),
                        self._emitter,
                    )

            elif event_type == "mouse_up":
                if self._on_mouse_up:
                    self._on_mouse_up(
                        event.get("x", 0.0),
                        event.get("y", 0.0),
                        event.get("button", "left"),
                        self._emitter,
                    )

            elif event_type == "mouse_move":
                if self._on_mouse_move:
                    self._on_mouse_move(
                        event.get("x", 0.0),
                        event.get("y", 0.0),
                        self._emitter,
                    )

            elif event_type == "scroll":
                if self._on_scroll:
                    self._on_scroll(
                        event.get("x", 0.0),
                        event.get("y", 0.0),
                        event.get("delta_x", 0.0),
                        event.get("delta_y", 0.0),
                        self._emitter,
                    )

            elif event_type == "pipe_data":
                if self._on_pipe_data:
                    self._on_pipe_data(
                        event.get("from_app", ""),
                        event.get("channel", ""),
                        event.get("value"),
                        self._emitter,
                    )

            elif event_type == "event_data":
                if self._on_event:
                    self._on_event(
                        event.get("kind", ""),
                        event.get("payload", {}),
                        self._emitter,
                    )

            elif event_type == "run_created":
                if self._on_run_created:
                    self._on_run_created(event.get("run_id", ""), self._emitter)

            elif event_type == "shutdown":
                break
