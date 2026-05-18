#!/usr/bin/env python3
"""Logs — live tail of ~/.plexi-beta/plexi.log, newest-first, color-coded by level."""

import os
import re

from plexi_sdk import App, RenderContext
from plexi_sdk.ui import (
    BG, SURFACE, HIGHLIGHT, ACCENT, MUTED, FG, RED, YELLOW,
    TEXT_HINT, TEXT_CAPTION,
    SPACE_SM,
    RADIUS_SM,
)

# ── Constants ──────────────────────────────────────────────────────────────────

LOG_PATH = os.path.expanduser("~/.plexi-beta/plexi.log")
POLL_MS  = 2_000
TIMER_ID = "poll"

ROW_H    = 20.0
BAR_H    = 32.0
FOOT_H   = 22.0
PAD      = SPACE_SM

TIME_W   = 66.0   # "HH:MM:SS" at hint size
BADGE_W  = 38.0   # "WARN" widest label
TARGET_W = 150.0

FILTERS = ["ALL", "ERROR", "WARN", "INFO", "DEBUG"]

LEVEL_COLOR: dict[str, str] = {
    "ERROR": RED,
    "WARN":  YELLOW,
    "INFO":  ACCENT,
    "DEBUG": MUTED,
    "TRACE": "#585b70",
}

# "#rrggbb33" — 20% alpha tint for badge background
LEVEL_BG: dict[str, str] = {
    "ERROR": "#f38ba833",
    "WARN":  "#f9e2af33",
    "INFO":  "#89b4fa33",
    "DEBUG": "#6c708633",
    "TRACE": "#58597033",
}

ROW_ALT = "#1a1a2a"   # alternating row tint

_LOG_RE = re.compile(
    r"^\[(\d{4}-\d{2}-\d{2} (\d{2}:\d{2}:\d{2}))\] \[(\w+)\] \[([^\]]+)\] (.*)$"
)

# ── Data ───────────────────────────────────────────────────────────────────────

class LogLine:
    __slots__ = ("time", "level", "target", "message")

    def __init__(self, time: str, level: str, target: str, message: str) -> None:
        self.time    = time
        self.level   = level
        self.target  = target
        self.message = message


def _parse(raw: str) -> LogLine | None:
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
        ctx.status_summary("Logs")
        ctx.set_timer(TIMER_ID, 50)   # near-immediate first load

    def on_timer(self, ctx: RenderContext, timer_id: str) -> None:
        if timer_id != TIMER_ID:
            return
        self._lines = _read_log()
        ctx.set_timer(TIMER_ID, POLL_MS)

    def on_key(self, _ctx: RenderContext, key: str, _mods: dict) -> None:
        step = ROW_H * 4
        if key in ("j", "down", "down"):
            self._scroll += step
            self._clamp()
        elif key in ("k", "up", "up"):
            self._scroll = max(0.0, self._scroll - step)
        elif key == "g":
            self._scroll = 0.0
        elif key == "G":
            self._scroll = 999_999.0
            self._clamp()
        elif key in ("1", "2", "3", "4", "5"):
            self._filter_idx = int(key) - 1
            self._scroll = 0.0

    def _clamp(self) -> None:
        filtered = self._filtered()
        max_s = max(0.0, len(filtered) * ROW_H - self._viewport_h)
        self._scroll = min(self._scroll, max_s)

    def _filtered(self) -> list[LogLine]:
        level = FILTERS[self._filter_idx]
        if level == "ALL":
            return self._lines
        return [l for l in self._lines if l.level == level]

    def on_render(self, ctx: RenderContext) -> None:
        w, h = ctx.w, ctx.h
        filtered = self._filtered()
        compact = ctx.size_class == "compact"
        # In compact mode, omit the target column to save horizontal space.
        target_w = 0.0 if compact else TARGET_W

        # ── background ──────────────────────────────────────────────────────
        ctx.rect(0, 0, w, h, BG)

        # ── top bar ─────────────────────────────────────────────────────────
        ctx.rect(0, 0, w, BAR_H, SURFACE)
        ctx.text(PAD, BAR_H / 2 - TEXT_CAPTION / 2, "Logs",
                 size=TEXT_CAPTION, color=FG, bold=True)

        chip_x = 48.0
        for i, label in enumerate(FILTERS):
            active  = i == self._filter_idx
            chip_w  = len(label) * 6.5 + PAD * 2
            chip_bg = ACCENT if active else HIGHLIGHT
            chip_fg = BG     if active else MUTED
            ctx.rect(chip_x, 6, chip_w, BAR_H - 12, chip_bg, radius=RADIUS_SM)
            ctx.text(chip_x + PAD, 6 + (BAR_H - 12) / 2 - TEXT_HINT / 2,
                     label, size=TEXT_HINT, color=chip_fg, bold=active)
            chip_x += chip_w + 4

        # ── footer ──────────────────────────────────────────────────────────
        foot_y = h - FOOT_H
        ctx.rect(0, foot_y, w, FOOT_H, SURFACE)
        ctx.text(PAD, foot_y + FOOT_H / 2 - TEXT_HINT / 2,
                 f"j/k scroll · g/G top/bottom · 1–5 filter · {len(filtered)} lines",
                 size=TEXT_HINT, color=MUTED)

        # ── log rows ────────────────────────────────────────────────────────
        list_y = BAR_H
        list_h = foot_y - list_y
        self._viewport_h = list_h

        if not filtered:
            ctx.text(PAD, list_y + 16, "no log entries", size=TEXT_CAPTION, color=MUTED)
            return

        ctx.push_clip(0, list_y, w, list_h)

        first = int(self._scroll / ROW_H)
        count = int(list_h / ROW_H) + 2

        for i in range(first, min(first + count, len(filtered))):
            ll  = filtered[i]
            row_y = list_y + i * ROW_H - self._scroll

            # alternating stripe
            if i % 2 == 0:
                ctx.rect(0, row_y, w, ROW_H, ROW_ALT)

            x = PAD
            text_y = row_y + ROW_H / 2 - TEXT_HINT / 2

            # timestamp
            ctx.text(x, text_y, ll.time,
                     size=TEXT_HINT, color=MUTED, monospace=True)
            x += TIME_W

            # level badge
            lc = LEVEL_COLOR.get(ll.level, MUTED)
            lb = LEVEL_BG.get(ll.level, "#6c708633")
            ctx.rect(x, row_y + 3, BADGE_W, ROW_H - 6, lb, radius=RADIUS_SM)
            ctx.text(x + 4, row_y + 3 + (ROW_H - 6) / 2 - TEXT_HINT / 2,
                     ll.level[:4], size=TEXT_HINT, color=lc, bold=True, monospace=True)
            x += BADGE_W + PAD

            # target (hidden in compact mode)
            if not compact:
                ctx.text(x, text_y, ll.target,
                         size=TEXT_HINT, color=MUTED,
                         max_width=target_w, elide=True)
            x += target_w + (PAD if not compact else 0.0)

            # message
            ctx.text(x, text_y, ll.message,
                     size=TEXT_HINT, color=FG,
                     max_width=w - x - PAD, elide=True)

        ctx.pop_clip()

        # scrollbar
        total_h = len(filtered) * ROW_H
        if total_h > list_h and list_h > 0:
            thumb_ratio = list_h / total_h
            thumb_h = max(20.0, list_h * thumb_ratio)
            thumb_y = list_y + (self._scroll / total_h) * list_h
            thumb_y = min(thumb_y, list_y + list_h - thumb_h)
            ctx.rect(w - 4, list_y, 4, list_h, HIGHLIGHT)
            ctx.rect(w - 4, thumb_y, 4, thumb_h, MUTED)


LogsApp().run()
