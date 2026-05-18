#!/usr/bin/env python3
"""Logs — live tail of the Plexi host log, filterable by level and substring."""

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

POLL_MS  = 2_000
TIMER_ID = "poll"

ROW_H    = 20.0
BAR_H    = 32.0
FOOT_H   = 22.0
PAD      = SPACE_SM

TIME_W   = 66.0   # "HH:MM:SS" at hint size
BADGE_W  = 38.0   # "WARN" widest label
TARGET_W = 150.0

WIDTH_WIDE = 400.0
WIDTH_TINY = 100.0

FILTERS = ["ALL", "ERROR", "WARN", "INFO", "DEBUG"]

LEVEL_COLOR: dict[str, str] = {
    "ERROR": RED,
    "WARN":  YELLOW,
    "INFO":  ACCENT,
    "DEBUG": MUTED,
    "TRACE": "#585b70",
}

LEVEL_BG: dict[str, str] = {
    "ERROR": "#f38ba833",
    "WARN":  "#f9e2af33",
    "INFO":  "#89b4fa33",
    "DEBUG": "#6c708633",
    "TRACE": "#58597033",
}

ROW_ALT = "#1a1a2a"

_LOG_RE = re.compile(
    r"^\[(\d{4}-\d{2}-\d{2} (\d{2}:\d{2}:\d{2}))\] \[(\w+)\] \[([^\]]+)\] (.*)$"
)


def _detect_log_path() -> str:
    """Resolve the active channel's log file path.

    Prefers PLEXI_CONFIG_DIR if injected by the host, then picks the most
    recently modified log among known channel paths.
    """
    config_dir = os.environ.get("PLEXI_CONFIG_DIR")
    if config_dir:
        return os.path.join(config_dir, "plexi.log")
    candidates = [
        "~/.plexi-alpha/plexi.log",
        "~/.plexi/plexi.log",
        "~/.plexi-beta/plexi.log",
    ]
    best_path: str | None = None
    best_mtime = 0.0
    for p in candidates:
        expanded = os.path.expanduser(p)
        try:
            mtime = os.path.getmtime(expanded)
            if mtime > best_mtime:
                best_mtime = mtime
                best_path = expanded
        except OSError:
            pass
    return best_path or os.path.expanduser("~/.plexi-alpha/plexi.log")


LOG_PATH = _detect_log_path()

# ── Data ───────────────────────────────────────────────────────────────────────

class LogLine:
    __slots__ = ("time", "level", "target", "message", "raw")

    def __init__(self, time: str, level: str, target: str, message: str, raw: str) -> None:
        self.time    = time
        self.level   = level
        self.target  = target
        self.message = message
        self.raw     = raw


def _parse(raw: str) -> LogLine | None:
    m = _LOG_RE.match(raw.rstrip())
    if not m:
        return None
    _, time, level, target, message = m.groups()
    return LogLine(time, level, target, message, raw.rstrip())


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
        self._search:     str   = ""
        self._searching:  bool  = False
        ctx.status_summary("Logs")
        ctx.info(f"logs: watching {LOG_PATH}")
        ctx.set_timer(TIMER_ID, 50)

    def on_timer(self, ctx: RenderContext, timer_id: str) -> None:
        if timer_id != TIMER_ID:
            return
        self._lines = _read_log()
        ctx.set_timer(TIMER_ID, POLL_MS)

    def on_key(self, _ctx: RenderContext, key: str, _mods: dict) -> None:
        if self._searching:
            if key == "escape":
                self._searching = False
                self._search = ""
                self._scroll = 0.0
            elif key == "return":
                self._searching = False
            elif key == "backspace":
                self._search = self._search[:-1]
                self._scroll = 0.0
            elif len(key) == 1:
                self._search += key
                self._scroll = 0.0
            return

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
        elif key in ("1", "2", "3", "4", "5"):
            self._filter_idx = int(key) - 1
            self._scroll = 0.0
        elif key == "/":
            self._searching = True
            self._search = ""
            self._scroll = 0.0

    def _clamp(self) -> None:
        filtered = self._filtered()
        max_s = max(0.0, len(filtered) * ROW_H - self._viewport_h)
        self._scroll = min(self._scroll, max_s)

    def _filtered(self) -> list[LogLine]:
        level = FILTERS[self._filter_idx]
        lines = self._lines if level == "ALL" else [l for l in self._lines if l.level == level]
        if self._search:
            q = self._search.lower()
            lines = [l for l in lines if q in l.raw.lower()]
        return lines

    def on_render(self, ctx: RenderContext) -> None:
        w, h = ctx.w, ctx.h
        filtered = self._filtered()

        ctx.rect(0, 0, w, h, BG)

        # ── tiny mode: just an error/warn count badge ────────────────────────
        if w <= WIDTH_TINY:
            ctx.rect(0, 0, w, h, SURFACE)
            error_count = sum(1 for l in self._lines if l.level == "ERROR")
            warn_count  = sum(1 for l in self._lines if l.level == "WARN")
            if error_count:
                label = f"E{error_count}"
                color = RED
            elif warn_count:
                label = f"W{warn_count}"
                color = YELLOW
            else:
                label = "OK"
                color = MUTED
            ctx.text(PAD, h / 2 - TEXT_CAPTION / 2, label,
                     size=TEXT_CAPTION, color=color, bold=True)
            return

        # ── top bar ─────────────────────────────────────────────────────────
        ctx.rect(0, 0, w, BAR_H, SURFACE)

        if self._searching:
            ctx.text(PAD, BAR_H / 2 - TEXT_CAPTION / 2, "/ ",
                     size=TEXT_CAPTION, color=ACCENT, bold=True)
            ctx.text(PAD + 16, BAR_H / 2 - TEXT_CAPTION / 2,
                     f"{self._search}▌",
                     size=TEXT_CAPTION, color=FG)
        else:
            ctx.text(PAD, BAR_H / 2 - TEXT_CAPTION / 2, "Logs",
                     size=TEXT_CAPTION, color=FG, bold=True)

            if w >= WIDTH_WIDE:
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

            if self._search:
                search_text = f"/{self._search}"
                ctx.text(w - len(search_text) * 6.5 - PAD,
                         BAR_H / 2 - TEXT_HINT / 2,
                         search_text, size=TEXT_HINT, color=ACCENT)

        # ── footer ──────────────────────────────────────────────────────────
        foot_y = h - FOOT_H
        ctx.rect(0, foot_y, w, FOOT_H, SURFACE)
        if self._searching:
            ctx.text(PAD, foot_y + FOOT_H / 2 - TEXT_HINT / 2,
                     "Enter apply  Esc cancel",
                     size=TEXT_HINT, color=MUTED)
        elif w >= WIDTH_WIDE:
            ctx.text(PAD, foot_y + FOOT_H / 2 - TEXT_HINT / 2,
                     f"j/k scroll · g/G top/bottom · 1–5 filter · / search · {len(filtered)} lines",
                     size=TEXT_HINT, color=MUTED)
        else:
            ctx.text(PAD, foot_y + FOOT_H / 2 - TEXT_HINT / 2,
                     f"j/k · / search · {len(filtered)} lines",
                     size=TEXT_HINT, color=MUTED)

        # ── log rows ────────────────────────────────────────────────────────
        list_y = BAR_H
        list_h = foot_y - list_y
        self._viewport_h = list_h

        if not filtered:
            msg = "no matches" if (self._search or self._filter_idx) else "no log entries"
            ctx.text(PAD, list_y + 16, msg, size=TEXT_CAPTION, color=MUTED)
            return

        ctx.push_clip(0, list_y, w, list_h)

        first = int(self._scroll / ROW_H)
        count = int(list_h / ROW_H) + 2
        wide  = w >= WIDTH_WIDE

        for i in range(first, min(first + count, len(filtered))):
            ll    = filtered[i]
            row_y = list_y + i * ROW_H - self._scroll

            if i % 2 == 0:
                ctx.rect(0, row_y, w, ROW_H, ROW_ALT)

            lc = LEVEL_COLOR.get(ll.level, MUTED)
            lb = LEVEL_BG.get(ll.level, "#6c708633")
            text_y = row_y + ROW_H / 2 - TEXT_HINT / 2
            x = PAD

            if wide:
                ctx.text(x, text_y, ll.time,
                         size=TEXT_HINT, color=MUTED, monospace=True)
                x += TIME_W

            ctx.rect(x, row_y + 3, BADGE_W, ROW_H - 6, lb, radius=RADIUS_SM)
            ctx.text(x + 4, row_y + 3 + (ROW_H - 6) / 2 - TEXT_HINT / 2,
                     ll.level[:4], size=TEXT_HINT, color=lc, bold=True, monospace=True)
            x += BADGE_W + PAD

            if wide:
                ctx.text(x, text_y, ll.target,
                         size=TEXT_HINT, color=MUTED,
                         max_width=TARGET_W, elide=True)
                x += TARGET_W + PAD

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
