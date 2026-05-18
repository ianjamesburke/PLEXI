#!/usr/bin/env python3
"""Logs — live tail of the Plexi host log, newest-first, color-coded by level."""

import os
import re

from plexi_sdk import App, RenderContext
from plexi_sdk.ui import (
    BG, SURFACE, HIGHLIGHT, ACCENT, MUTED, FG, RED, YELLOW,
    TEXT_HINT, TEXT_CAPTION,
    SPACE_SM,
)

# ── Constants ──────────────────────────────────────────────────────────────────

POLL_MS  = 2_000
TIMER_ID = "poll"

ROW_H    = 20.0
BAR_H    = 32.0
FOOT_H   = 24.0
PAD      = SPACE_SM
CHIP_W   = 54.0   # fixed-width filter buttons — consistent across all labels
CHIP_GAP = 4.0
BADGE_ADV = 44.0  # fixed x-advance after a 4-char badge at 10pt

FILTERS    = ["ALL", "ERROR", "WARN", "INFO", "DEBUG"]
FILTER_KEY = {"a": 0, "e": 1, "w": 2, "i": 3, "d": 4}

# Solid fill colours for ctx.badge() — readable on dark rows
LEVEL_BADGE_FILL: dict[str, str] = {
    "ERROR": RED,
    "WARN":  YELLOW,
    "INFO":  ACCENT,
    "DEBUG": "#585b70",
    "TRACE": "#45475a",
}

ROW_ALT = "#1a1a2a"

_LOG_RE = re.compile(
    r"^\[(\d{4}-\d{2}-\d{2} (\d{2}:\d{2}:\d{2}))\] \[(\w+)\] \[([^\]]+)\] (.*)$"
)

# ── Log path detection ─────────────────────────────────────────────────────────

def _detect_log_path() -> str:
    env = os.environ.get("PLEXI_CONFIG_DIR")
    if env:
        return os.path.join(env, "plexi.log")
    candidates = [
        os.path.expanduser(p) for p in (
            "~/.plexi-alpha/plexi.log",
            "~/.plexi-beta/plexi.log",
            "~/.plexi/plexi.log",
        )
    ]
    existing = [(os.path.getmtime(p), p) for p in candidates if os.path.exists(p)]
    return max(existing)[1] if existing else candidates[0]


LOG_PATH = _detect_log_path()

# ── Data ───────────────────────────────────────────────────────────────────────

class LogLine:
    __slots__ = ("time", "level", "target", "message")

    def __init__(self, time: str, level: str, target: str, message: str) -> None:
        self.time    = time
        self.level   = level
        self.target  = target
        self.message = message


def _parse(raw: str) -> "LogLine | None":
    m = _LOG_RE.match(raw.rstrip())
    if not m:
        return None
    _, time, level, target, message = m.groups()
    return LogLine(time, level, target, message)


def _read_log(max_lines: int = 5_000) -> list[LogLine]:
    try:
        with open(LOG_PATH) as f:
            tail = f.readlines()[-max_lines:]
    except OSError:
        return []
    out: list[LogLine] = []
    for raw in reversed(tail):
        ll = _parse(raw)
        if ll:
            out.append(ll)
    return out

# ── App ────────────────────────────────────────────────────────────────────────

class LogsApp(App):
    def on_init(self, ctx: RenderContext) -> None:
        self._lines:      list[LogLine] = []
        self._filter_idx: int   = 0
        self._scroll:     float = 0.0
        self._viewport_h: float = 400.0
        ctx.emit.set_mouse_tracking(True)
        ctx.status_summary("Logs")
        ctx.set_timer(TIMER_ID, 50)
        self.emit.info(f"logs: ready — {LOG_PATH}")

    def on_timer(self, ctx: RenderContext, timer_id: str) -> None:
        if timer_id != TIMER_ID:
            return
        self._lines = _read_log()
        ctx.set_timer(TIMER_ID, POLL_MS)

    def on_key(self, _ctx: RenderContext, key: str, _mods: dict) -> None:
        step = ROW_H * 4
        if key in ("j", "down"):
            self._scroll += step
            self._clamp()
        elif key in ("k", "up"):
            self._scroll = max(0.0, self._scroll - step)
        elif key == "g":
            self._scroll = 0.0
        elif key == "G":
            self._scroll = 999_999.0
            self._clamp()
        elif key in FILTER_KEY:
            self._filter_idx = FILTER_KEY[key]
            self._scroll = 0.0

    def _clamp(self) -> None:
        filtered = self._filtered()
        max_s = max(0.0, len(filtered) * ROW_H - self._viewport_h)
        self._scroll = min(self._scroll, max_s)

    def _filtered(self) -> list[LogLine]:
        level = FILTERS[self._filter_idx]
        if level == "ALL":
            return self._lines
        return [ll for ll in self._lines if ll.level == level]

    def on_render(self, ctx: RenderContext) -> None:
        w, h = ctx.w, ctx.h
        filtered = self._filtered()

        # ── background ──────────────────────────────────────────────────────
        ctx.rect(0, 0, w, h, BG)

        # ── top bar ─────────────────────────────────────────────────────────
        ctx.rect(0, 0, w, BAR_H, SURFACE)
        ctx.text(PAD, BAR_H / 2 - TEXT_CAPTION / 2, "Logs",
                 size=TEXT_CAPTION, color=FG, bold=True)

        chip_x = 50.0
        for i, label in enumerate(FILTERS):
            active = i == self._filter_idx
            if ctx.button(
                f"filter_{i}", chip_x, 5.0, CHIP_W, BAR_H - 10.0, label,
                fill=ACCENT if active else HIGHLIGHT,
                hover_fill="#a6c5f5" if active else "#45475a",
                active_fill="#6ea8f5" if active else "#585b70",
                text_color=BG if active else MUTED,
                font_size=12.0,
                radius=5.0,
            ):
                self._filter_idx = i
                self._scroll = 0.0
            chip_x += CHIP_W + CHIP_GAP

        # ── footer ──────────────────────────────────────────────────────────
        foot_y = h - FOOT_H
        ctx.rect(0, foot_y, w, FOOT_H, SURFACE)
        ctx.shortcuts(PAD, foot_y + 5.0, w - PAD * 2, [
            (["a", "e", "w", "i", "d"], "filter"),
            (["j", "k"], "scroll"),
            (["g", "G"], "top/btm"),
        ], font_size=10.0)
        ctx.text(w - PAD, foot_y + FOOT_H / 2 - TEXT_HINT / 2,
                 f"{len(filtered)} lines", size=TEXT_HINT, color=MUTED,
                 align="right")

        # ── log rows ────────────────────────────────────────────────────────
        list_y = BAR_H
        list_h = foot_y - list_y
        self._viewport_h = list_h

        if not filtered:
            ctx.text(PAD, list_y + 16, "no log entries", size=TEXT_CAPTION, color=MUTED)
            return

        ctx.push_clip(0, list_y, w, list_h)

        time_w   = 66.0
        target_w = 140.0
        first    = int(self._scroll / ROW_H)
        count    = int(list_h / ROW_H) + 2

        for i in range(first, min(first + count, len(filtered))):
            ll    = filtered[i]
            row_y = list_y + i * ROW_H - self._scroll

            if i % 2 == 0:
                ctx.rect(0, row_y, w, ROW_H, ROW_ALT)

            x      = PAD
            text_y = row_y + ROW_H / 2 - TEXT_HINT / 2

            # timestamp
            ctx.text(x, text_y, ll.time,
                     size=TEXT_HINT, color=MUTED, monospace=True)
            x += time_w

            # level badge — solid fill, host-measured, readable at any size
            badge_fill = LEVEL_BADGE_FILL.get(ll.level, "#45475a")
            badge_fg   = BG if ll.level in ("ERROR", "WARN", "INFO") else FG
            ctx.badge(x, row_y + ROW_H / 2, ll.level[:4],
                      fill=badge_fill, fg=badge_fg,
                      font_size=10.0, radius=4.0)
            x += BADGE_ADV

            # target
            ctx.text(x, text_y, ll.target,
                     size=TEXT_HINT, color=MUTED,
                     max_width=target_w, elide=True)
            x += target_w + PAD

            # message
            ctx.text(x, text_y, ll.message,
                     size=TEXT_HINT, color=FG,
                     max_width=w - x - PAD, elide=True)

        ctx.pop_clip()

        # scrollbar
        total_h = len(filtered) * ROW_H
        if total_h > list_h and list_h > 0:
            thumb_ratio = list_h / total_h
            thumb_h     = max(20.0, list_h * thumb_ratio)
            thumb_y     = list_y + (self._scroll / total_h) * list_h
            thumb_y     = min(thumb_y, list_y + list_h - thumb_h)
            ctx.rect(w - 4, list_y, 4, list_h, HIGHLIGHT)
            ctx.rect(w - 4, thumb_y, 4, thumb_h, MUTED)


LogsApp().run()
