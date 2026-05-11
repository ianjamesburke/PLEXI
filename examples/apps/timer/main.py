#!/usr/bin/env python3
import math
import time

from plexi_sdk import (
    App, RenderContext,
    BG, SURFACE, ACCENT, MUTED, FG, GREEN,
    TITLE, CAPTION,
    PRIORITY_HIGH,
)
from plexi_sdk.ui import Column, AppBar, FooterKeys, Component, Label

DURATIONS = [
    (15, "15 sec"),
    (30, "30 sec"),
    (60, "1 min"),
    (2 * 60, "2 min"),
    (3 * 60, "3 min"),
    (5 * 60, "5 min"),
    (10 * 60, "10 min"),
    (15 * 60, "15 min"),
    (20 * 60, "20 min"),
    (30 * 60, "30 min"),
    (45 * 60, "45 min"),
    (60 * 60, "60 min"),
]

DEFAULT_IDX = 4  # 5 min
DEFAULT_MSG = "Timer done!"
TICK_ID = "tick"
TWO_PI = 2 * math.pi


def _fmt(secs: float) -> str:
    secs = int(max(0, secs))
    m, s = divmod(secs, 60)
    h, m = divmod(m, 60)
    if h:
        return f"{h}:{m:02d}:{s:02d}"
    return f"{m:02d}:{s:02d}"


class _Face(Component):
    """Centered arc ring with time text. Grows to fill remaining column space."""

    def __init__(self, app: "TimerApp") -> None:
        self._app = app

    def measure(self, avail_w: float) -> float:
        return 0.0

    def is_grow(self) -> bool:
        return True

    def render(self, ctx, x: float, y: float, w: float, h: float) -> None:
        app = self._app
        cx = x + w / 2
        cy = y + h / 2
        r = min(w, h) * 0.36

        # Ring geometry: 2px outer rim, 12px track, BG center punch-out.
        # ctx.arc draws filled pie slices from center, so we layer circles
        # to build a donut: outer circle → track circle → progress arc → hole.
        rim = r - 2    # inner edge of outer rim / outer edge of track
        hole = r - 14  # center hole (track width = rim - hole = 12px)

        if app._state == "running":
            total = app._total_secs
            elapsed = time.time() - app._start_at
            remaining = max(0.0, total - elapsed)
            frac = remaining / total if total > 0 else 0.0
            ctx.circle(cx, cy, r, SURFACE)
            ctx.circle(cx, cy, rim, MUTED)
            if frac > 0:
                start_angle = -math.pi / 2
                ctx.arc(cx, cy, rim, start_angle, start_angle + frac * TWO_PI, ACCENT)
            ctx.circle(cx, cy, hole, BG)
            label = _fmt(remaining)
            sub = app._msg.strip() or DEFAULT_MSG
            color = ACCENT

        elif app._state == "done":
            ctx.circle(cx, cy, r, SURFACE)
            ctx.circle(cx, cy, rim, GREEN)
            ctx.circle(cx, cy, hole, BG)
            label = "✓"
            sub = app._msg.strip() or DEFAULT_MSG
            color = GREEN

        else:  # setup
            ctx.circle(cx, cy, r, SURFACE)
            ctx.circle(cx, cy, rim, MUTED)
            ctx.circle(cx, cy, hole, BG)
            label = _fmt(DURATIONS[app._dur_idx][0])
            if app._editing_msg:
                sub = f"{app._msg}▌" if app._msg else "▌"
            elif app._msg:
                sub = app._msg
            else:
                sub = "press m to set message"
            color = FG

        ctx.text(cx, cy - 14, label, size=TITLE + 8, color=color, bold=True, align="center")
        ctx.text(cx, cy + 22, sub, size=CAPTION, color=MUTED, align="center",
                 max_width=r * 1.7)


class TimerApp(App):

    def on_init(self, ctx: RenderContext) -> None:
        self._state: str = "setup"
        self._dur_idx: int = DEFAULT_IDX
        self._msg: str = ""
        self._start_at: float = 0.0
        self._total_secs: int = DURATIONS[DEFAULT_IDX][0]
        self._editing_msg: bool = False
        ctx.status_summary("Timer")
        self.emit.info("Timer ready")

    # ── timer tick ────────────────────────────────────────────────────────

    def on_timer(self, ctx: RenderContext, timer_id: str) -> None:
        if timer_id != TICK_ID:
            return
        elapsed = time.time() - self._start_at
        if elapsed >= self._total_secs:
            self._state = "done"
            msg = self._msg.strip() or DEFAULT_MSG
            ctx.notify(title="Timer", body=msg, level="info", priority=PRIORITY_HIGH)
            ctx.status_summary("Timer — done")
            self.emit.info(f"Timer fired: {msg!r}")
        else:
            ctx.set_timer(TICK_ID, 1000)
        self.emit.schedule_render(after_ms=50)

    # ── keys ──────────────────────────────────────────────────────────────

    def on_key(self, ctx: RenderContext, key: str, mods: dict) -> None:
        if self._state == "setup":
            self._setup_key(ctx, key)
        elif self._state == "running":
            if key in ("escape", "space", "q"):
                ctx.cancel_timer(TICK_ID)
                self._reset(ctx)
                self.emit.info("Timer cancelled")
        elif self._state == "done":
            self._reset(ctx)

    def _setup_key(self, ctx, key: str) -> None:
        if self._editing_msg:
            if key in ("return", "escape"):
                self._editing_msg = False
                self.emit.info(f"Message set: {self._msg!r}")
            elif key == "backspace":
                self._msg = self._msg[:-1]
            elif key == "space":
                self._msg += " "
            elif len(key) == 1:
                self._msg += key
            return

        if key == "k":
            if self._dur_idx < len(DURATIONS) - 1:
                self._dur_idx += 1
                self.emit.info(f"Duration → {DURATIONS[self._dur_idx][1]}")
        elif key == "j":
            if self._dur_idx > 0:
                self._dur_idx -= 1
                self.emit.info(f"Duration → {DURATIONS[self._dur_idx][1]}")
        elif key in ("m", "e"):
            self._editing_msg = True
            self._msg = ""
        elif key in ("return", "space"):
            self._total_secs = DURATIONS[self._dur_idx][0]
            self._start_at = time.time()
            self._state = "running"
            ctx.set_timer(TICK_ID, 1000)
            ctx.status_summary(f"Timer — {DURATIONS[self._dur_idx][1]}")
            self.emit.info(f"Timer started: {DURATIONS[self._dur_idx][1]}")
            self.emit.schedule_render(after_ms=50)

    def _reset(self, ctx) -> None:
        self._state = "setup"
        self._editing_msg = False
        ctx.status_summary("Timer")
        self.emit.schedule_render(after_ms=50)

    # ── render ────────────────────────────────────────────────────────────

    def on_render(self, ctx: RenderContext) -> None:
        ctx.clear(BG)
        face = _Face(self)

        if self._state == "setup":
            if self._editing_msg:
                status = Label(f"message: {self._msg}▌", tone="caption", color=MUTED)
                footer = FooterKeys([(["↵", "esc"], "done")])
            else:
                status = Label(DURATIONS[self._dur_idx][1], tone="caption", color=MUTED)
                footer = FooterKeys([
                    (["j", "k"], "duration"),
                    (["m"], "message"),
                    (["↵"], "start"),
                ])
            ctx.render(Column([AppBar(title="Timer"), status, face, footer]))

        elif self._state == "running":
            ctx.render(Column([
                AppBar(title="Timer"),
                Label(DURATIONS[self._dur_idx][1], tone="caption", color=MUTED),
                face,
                FooterKeys([(["esc"], "cancel")]),
            ]))
            self.emit.schedule_render(after_ms=1000)

        elif self._state == "done":
            ctx.render(Column([
                AppBar(title="Timer"),
                Label("done", tone="caption", color=GREEN),
                face,
                FooterKeys([(["any key"], "reset")]),
            ]))


TimerApp().run()
