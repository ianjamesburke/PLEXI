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
# Badge advance: widest label "WARN"/"INFO"/"ERRO" at 10pt ~30 px glyph width
# + 8 px pad each side = ~46 px. Use 50 for a consistent gutter.
BADGE_ADV = 50.0

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

ROW_ALT     = "#1a1a2a"
COPY_ROW_BG = "#1e2d1e"   # subtle green tint for copy-mode cursor row
COPY_ROW_FG = "#a6e3a1"   # soft green text for selected row

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

    def as_text(self) -> str:
        return f"[{self.time}] [{self.level}] [{self.target}] {self.message}"


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
        self._lines:       list[LogLine] = []
        self._filter_idx:  int   = 0
        self._scroll:      float = 0.0
        self._viewport_h:  float = 400.0
        # search
        self._search_mode: bool = False
        self._search_q:    str  = ""
        # copy mode
        self._copy_mode:   bool = False
        self._copy_row:    int  = 0   # index into filtered list
        ctx.emit.set_mouse_tracking(True)
        ctx.status_summary("Logs")
        ctx.set_timer(TIMER_ID, 50)
        self.emit.info(f"logs: ready — {LOG_PATH}")

    def on_timer(self, ctx: RenderContext, timer_id: str) -> None:
        if timer_id != TIMER_ID:
            return
        self._lines = _read_log()
        ctx.set_timer(TIMER_ID, POLL_MS)

    def on_text_submitted(self, _ctx: RenderContext, id: str, text: str) -> None:  # noqa: ARG002
        if id == "search":
            self._search_q    = text.strip()
            self._search_mode = False
            self._scroll      = 0.0
            self._copy_row    = 0

    def on_key(self, ctx: RenderContext, key: str, _mods: dict) -> None:  # noqa: ARG002
        # ── search mode: only Esc is handled here; host owns the text field ──
        if self._search_mode:
            if key == "escape":
                self._search_mode = False
                self._search_q    = ""
                self._scroll      = 0.0
                self._copy_row    = 0
            return

        # ── copy mode ─────────────────────────────────────────────────────
        if self._copy_mode:
            filtered = self._filtered()
            if key == "escape":
                self._copy_mode = False
            elif key in ("j", "down"):
                self._copy_row = min(len(filtered) - 1, self._copy_row + 1)
                self._ensure_copy_row_visible()
            elif key in ("k", "up"):
                self._copy_row = max(0, self._copy_row - 1)
                self._ensure_copy_row_visible()
            elif key == "y":
                if filtered and 0 <= self._copy_row < len(filtered):
                    ctx.copy_to_clipboard(filtered[self._copy_row].as_text())
                self._copy_mode = False
            return

        # ── normal mode ───────────────────────────────────────────────────
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
            self._copy_row = 0
        elif key == "/":
            self._search_mode = True
        elif key == "escape" and self._search_q:
            self._search_q = ""
            self._scroll   = 0.0
            self._copy_row = 0
        elif key == "y":
            filtered = self._filtered()
            if filtered:
                self._copy_mode = True
                self._copy_row  = min(self._copy_row, len(filtered) - 1)

    def _ensure_copy_row_visible(self) -> None:
        row_top = self._copy_row * ROW_H
        row_bot = row_top + ROW_H
        if row_top < self._scroll:
            self._scroll = row_top
        elif row_bot > self._scroll + self._viewport_h:
            self._scroll = row_bot - self._viewport_h

    def _clamp(self) -> None:
        filtered = self._filtered()
        max_s = max(0.0, len(filtered) * ROW_H - self._viewport_h)
        self._scroll = min(self._scroll, max_s)

    def _filtered(self) -> list[LogLine]:
        level = FILTERS[self._filter_idx]
        lines = self._lines if level == "ALL" else [
            ll for ll in self._lines if ll.level == level
        ]
        if not self._search_q:
            return lines
        q = self._search_q.lower()
        return [ll for ll in lines if q in ll.target.lower() or q in ll.message.lower()]

    def on_render(self, ctx: RenderContext) -> None:
        w, h = ctx.w, ctx.h
        filtered = self._filtered()

        # ── background ──────────────────────────────────────────────────────
        ctx.rect(0, 0, w, h, BG)

        # ── top bar ─────────────────────────────────────────────────────────
        ctx.rect(0, 0, w, BAR_H, SURFACE)

        if self._search_mode:
            ctx.text(PAD, BAR_H / 2 - TEXT_CAPTION / 2, "/",
                     size=TEXT_CAPTION, color=ACCENT, bold=True)
            search_x = PAD + 14.0
            submitted = ctx.text_input(
                "search",
                x=search_x, y=4.0,
                w=w - search_x - PAD,
                placeholder="filter by target or message…",
                h=BAR_H - 8.0,
            )
            if submitted is not None:
                self._search_q    = submitted.strip()
                self._search_mode = False
                self._scroll      = 0.0
                self._copy_row    = 0
        else:
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
                    self._copy_row = 0
                chip_x += CHIP_W + CHIP_GAP

            if self._search_q:
                ctx.text(w - PAD, BAR_H / 2 - TEXT_HINT / 2,
                         f"/{self._search_q}",
                         size=TEXT_HINT, color=ACCENT, align="right")

        # ── footer ──────────────────────────────────────────────────────────
        foot_y = h - FOOT_H
        ctx.rect(0, foot_y, w, FOOT_H, SURFACE)

        if self._copy_mode:
            ctx.shortcuts(PAD, foot_y + 5.0, w - PAD * 2, [
                (["j", "k"], "move"),
                (["y"], "copy line"),
                (["esc"], "exit"),
            ], font_size=10.0)
            ctx.text(w - PAD, foot_y + FOOT_H / 2 - TEXT_HINT / 2,
                     "COPY MODE", size=TEXT_HINT, color=COPY_ROW_FG,
                     align="right")
        elif self._search_mode:
            ctx.shortcuts(PAD, foot_y + 5.0, w - PAD * 2, [
                (["enter"], "apply"),
                (["esc"], "cancel"),
            ], font_size=10.0)
            ctx.text(w - PAD, foot_y + FOOT_H / 2 - TEXT_HINT / 2,
                     "SEARCH", size=TEXT_HINT, color=ACCENT,
                     align="right")
        else:
            ctx.shortcuts(PAD, foot_y + 5.0, w - PAD * 2, [
                (["a", "e", "w", "i", "d"], "filter"),
                (["j", "k"], "scroll"),
                (["g", "G"], "top/btm"),
                (["/"], "search"),
                (["y"], "copy"),
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

            is_copy_cursor = self._copy_mode and i == self._copy_row

            if is_copy_cursor:
                ctx.rect(0, row_y, w, ROW_H, COPY_ROW_BG)
            elif i % 2 == 0:
                ctx.rect(0, row_y, w, ROW_H, ROW_ALT)

            x      = PAD
            text_y = row_y + ROW_H / 2 - TEXT_HINT / 2
            dim_fg = COPY_ROW_FG if is_copy_cursor else MUTED
            msg_fg = COPY_ROW_FG if is_copy_cursor else FG

            # timestamp
            ctx.text(x, text_y, ll.time,
                     size=TEXT_HINT, color=dim_fg, monospace=True)
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
                     size=TEXT_HINT, color=dim_fg,
                     max_width=target_w, elide=True)
            x += target_w + PAD

            # message
            ctx.text(x, text_y, ll.message,
                     size=TEXT_HINT, color=msg_fg,
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
