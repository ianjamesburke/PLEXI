#!/usr/bin/env python3
"""Logs — live tail of the Plexi host log, newest-first, color-coded by level."""

import os
import re

from plexi_sdk import App, RenderContext
from plexi_sdk.ui import (
    TEXT_HINT, TEXT_CAPTION,
    SPACE_SM,
    AppBar, FooterKeys,
)

# ── Constants ──────────────────────────────────────────────────────────────────

POLL_MS  = 2_000
TIMER_ID = "poll"

ROW_H    = 20.0
BAR_H    = 34.0   # AppBar single-line band height
FOOT_H   = 32.0   # FooterKeys band height (TOP_GAP + 1 + TOP_GAP + ROW_H + TOP_GAP)
PAD      = SPACE_SM
CHIP_W    = 54.0   # fixed-width filter buttons — consistent across all labels
CHIP_GAP  = 4.0
COL_GAP   = SPACE_SM  # uniform inter-column gap for all table columns
# Level badge: fixed pixel width so all badges (ERROR/WARN/INFO/DEBUG) render
# in a uniform column. We draw a rect + centred text rather than ctx.badge()
# which auto-sizes to label width and would produce ragged column edges.
BADGE_W   = 46.0   # fixed badge width (px)
BADGE_H   = 14.0   # fixed badge height (px); fits comfortably inside ROW_H=20
BADGE_R   = 3.0    # corner radius
# Total column advance: badge + gap before next column
BADGE_ADV = BADGE_W + COL_GAP

FILTERS    = ["ALL", "ERROR", "WARN", "INFO", "DEBUG"]
FILTER_KEY = {"a": 0, "e": 1, "w": 2, "i": 3, "d": 4}

APP_FILTER_KEY = "t"   # cycle through unique targets with 't'

ROW_ALT        = "#1a1a2a"
COPY_ROW_BG    = "#1e2d1e"   # subtle green tint for copy-mode selected rows
COPY_CURSOR_BG = "#253525"   # slightly brighter for the active cursor row
COPY_ROW_FG    = "#a6e3a1"   # soft green text for selected row

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

# AppBar.BAND_H for single-line = 34.0; DIVIDER_H = 1.0 → total = 35.0
_APPBAR_H = AppBar.BAND_H + AppBar.DIVIDER_H


class LogsApp(App):
    def on_init(self, ctx: RenderContext) -> None:
        self._lines:       list[LogLine] = []
        self._filter_idx:  int   = 0
        self._scroll:      float = 0.0
        self._viewport_h:  float = 400.0
        # search
        self._search_mode: bool = False
        self._search_q:    str  = ""
        # app/target filter
        self._targets:     list[str] = ["ALL"]   # ["ALL", ...unique targets...]
        self._target_idx:  int       = 0         # index into _targets
        # copy mode
        self._copy_mode:   bool           = False
        self._copy_row:    int            = 0    # active cursor / drag end
        self._copy_anchor: "int | None"   = None  # selection start; None = single row
        self._is_dragging: bool           = False
        ctx.emit.set_mouse_tracking(True)
        ctx.status_summary("Logs")
        ctx.set_timer(TIMER_ID, 50)
        self.emit.info(f"logs: ready — {LOG_PATH}")

    def on_timer(self, ctx: RenderContext, timer_id: str) -> None:
        if timer_id != TIMER_ID:
            return
        self._lines = _read_log()
        self._refresh_targets()
        ctx.set_timer(TIMER_ID, POLL_MS)

    def _refresh_targets(self) -> None:
        """Rebuild the unique target list; preserve current selection if still valid."""
        seen: list[str] = []
        for ll in self._lines:
            if ll.target not in seen:
                seen.append(ll.target)
        new_targets = ["ALL"] + seen
        current = self._targets[self._target_idx] if self._target_idx < len(self._targets) else "ALL"
        self._targets = new_targets
        # Preserve selection if the target is still present; otherwise reset to ALL.
        try:
            self._target_idx = new_targets.index(current)
        except ValueError:
            self._target_idx = 0

    def on_text_submitted(self, _ctx: RenderContext, id: str, text: str) -> None:
        if id == "search":
            self._search_q    = text.strip()
            self._search_mode = False
            self._scroll      = 0.0
            self._copy_row    = 0
            self._copy_anchor = None

    def on_key(self, ctx: RenderContext, key: str, mods: dict) -> None:
        shift = mods.get("shift", False)

        # ── search mode: only Esc is handled here; host owns the text field ──
        if self._search_mode:
            if key == "escape":
                self._search_mode = False
                self._search_q    = ""
                self._scroll      = 0.0
                self._copy_row    = 0
                self._copy_anchor = None
            return

        # ── copy mode ─────────────────────────────────────────────────────
        if self._copy_mode:
            filtered = self._filtered()
            if key == "escape":
                self._copy_mode   = False
                self._copy_anchor = None
            elif key in ("j", "down", "k", "up"):
                if shift and self._copy_anchor is None:
                    self._copy_anchor = self._copy_row
                elif not shift:
                    self._copy_anchor = None
                delta = 1 if key in ("j", "down") else -1
                self._copy_row = max(0, min(len(filtered) - 1, self._copy_row + delta))
                self._ensure_copy_row_visible()
            elif key == "y":
                if filtered:
                    lo, hi = self._copy_range(len(filtered))
                    text = "\n".join(filtered[i].as_text() for i in range(lo, hi + 1))
                    ctx.copy_to_clipboard(text)
                    self.emit.info(f"logs: copied {hi - lo + 1} line(s) to clipboard")
                self._copy_mode   = False
                self._copy_anchor = None
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
            self._scroll      = 0.0
            self._copy_row    = 0
            self._copy_anchor = None
        elif key == APP_FILTER_KEY:
            if len(self._targets) > 1:
                self._target_idx = (self._target_idx + 1) % len(self._targets)
                self._scroll      = 0.0
                self._copy_row    = 0
                self._copy_anchor = None
        elif key == "/":
            self._search_mode = True
        elif key == "escape" and self._search_q:
            self._search_q    = ""
            self._scroll      = 0.0
            self._copy_row    = 0
            self._copy_anchor = None
        elif key == "y":
            filtered = self._filtered()
            if filtered:
                self._copy_mode   = True
                self._copy_row    = min(self._copy_row, len(filtered) - 1)
                self._copy_anchor = None

    def on_mouse_down(self, _ctx: RenderContext, _x: float, y: float, button: str, mods: dict = {}) -> None:
        if button not in ("left", "primary"):
            return
        row = self._row_at_y(y)
        if row is None:
            return
        self._copy_mode   = True
        self._is_dragging = True
        if mods.get("shift") and self._copy_anchor is not None:
            self._copy_row = row
            self.emit.info(f"logs: shift-click extend selection to row {row}")
        else:
            self._copy_anchor = row
            self._copy_row    = row
            self.emit.info(f"logs: mouse select started at row {row}")

    def on_mouse_move(self, _ctx: RenderContext, _x: float, y: float, buttons: list, _mods: dict = {}) -> None:
        if not self._is_dragging or not any(b in buttons for b in ("left", "primary")):
            self._is_dragging = False
            return
        row = self._row_at_y(y)
        if row is not None:
            self._copy_row = row
            self._ensure_copy_row_visible()

    def on_mouse_up(self, _ctx: RenderContext, _x: float, _y: float, button: str, _mods: dict = {}) -> None:
        if button in ("left", "primary"):
            self._is_dragging = False

    def _row_at_y(self, y: float) -> "int | None":
        list_y = _APPBAR_H
        if y < list_y or y > list_y + self._viewport_h:
            return None
        row = int((y - list_y + self._scroll) / ROW_H)
        filtered = self._filtered()
        if 0 <= row < len(filtered):
            return row
        return None

    def _copy_range(self, total: int) -> "tuple[int, int]":
        """Return (lo, hi) inclusive row indices for the current selection."""
        if total == 0:
            return 0, -1
        if self._copy_anchor is None:
            r = max(0, min(total - 1, self._copy_row))
            return r, r
        lo = max(0, min(self._copy_anchor, self._copy_row))
        hi = min(total - 1, max(self._copy_anchor, self._copy_row))
        return lo, hi

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
        target = self._targets[self._target_idx] if self._target_idx < len(self._targets) else "ALL"
        if target != "ALL":
            lines = [ll for ll in lines if ll.target == target]
        if not self._search_q:
            return lines
        q = self._search_q.lower()
        return [ll for ll in lines if q in ll.target.lower() or q in ll.message.lower()]

    def _subtitle(self) -> str:
        """Build AppBar subtitle showing active filter / target / search query."""
        parts: list[str] = []
        if self._filter_idx != 0:
            parts.append(FILTERS[self._filter_idx])
        target = self._targets[self._target_idx] if self._target_idx < len(self._targets) else "ALL"
        if target != "ALL":
            parts.append(f"@{target}")
        if self._search_q:
            parts.append(f"/{self._search_q}")
        return "  ".join(parts) if parts else LOG_PATH

    def _footer_shortcuts(self) -> list[tuple]:
        if self._copy_mode:
            return [
                (["j", "k"], "move"),
                (["⇧j", "⇧k"], "extend"),
                (["y"], "copy"),
                (["esc"], "exit"),
            ]
        if self._search_mode:
            return [
                (["enter"], "apply"),
                (["esc"], "cancel"),
            ]
        return [
            (["a", "e", "w", "i", "d"], "level"),
            (["t"], "target"),
            (["j", "k"], "scroll"),
            (["g", "G"], "top/btm"),
            (["/"], "search"),
            (["y"], "copy"),
        ]

    def on_render(self, ctx: RenderContext) -> None:
        w, h = ctx.w, ctx.h
        filtered = self._filtered()

        level_badge_fill = {
            "ERROR": ctx.theme.danger,
            "WARN":  ctx.theme.warning,
            "INFO":  ctx.theme.accent,
            "DEBUG": ctx.theme.text_section,
            "TRACE": ctx.theme.highlight,
        }

        # ── L1 header ───────────────────────────────────────────────────────
        appbar = AppBar(
            title="Logs",
            subtitle=self._subtitle() if not self._search_mode else None,
        )
        appbar_h = appbar.render_into(ctx, 0.0, 0.0, w)

        # ── Search input (replaces the bar when search mode is active) ──────
        if self._search_mode:
            # Draw "/" prefix then the text input inline in the bar area.
            bar_y = 0.0
            ctx.rect(0, bar_y, w, _APPBAR_H, ctx.theme.surface)
            ctx.text(PAD, _APPBAR_H / 2 - TEXT_CAPTION / 2, "/",
                     size=TEXT_CAPTION, color=ctx.theme.accent, bold=True)
            search_x = PAD + 14.0
            submitted = ctx.text_input(
                "search",
                x=search_x, y=4.0,
                w=w - search_x - PAD,
                placeholder="filter by target or message…",
                h=_APPBAR_H - 8.0,
            )
            ctx.rect(0, _APPBAR_H - 1.0, w, 1.0, ctx.theme.highlight)
            if submitted is not None:
                self._search_q    = submitted.strip()
                self._search_mode = False
                self._scroll      = 0.0
                self._copy_row    = 0
                self._copy_anchor = None
        else:
            # ── filter chip buttons (rendered below the AppBar divider) ─────
            # They live inside the AppBar band — rendered on top after AppBar.
            chip_x = 56.0   # leave room for "Logs" title
            for i, label in enumerate(FILTERS):
                active = i == self._filter_idx
                if ctx.button(
                    f"filter_{i}", chip_x, 5.0, CHIP_W, AppBar.BAND_H - 10.0, label,
                    fill=ctx.theme.accent if active else ctx.theme.highlight,
                    hover_fill="#a6c5f5" if active else "#45475a",
                    active_fill="#6ea8f5" if active else "#585b70",
                    text_color=ctx.theme.bg if active else ctx.theme.muted,
                    font_size=12.0,
                    radius=5.0,
                ):
                    self._filter_idx = i
                    self._scroll      = 0.0
                    self._copy_row    = 0
                    self._copy_anchor = None
                chip_x += CHIP_W + CHIP_GAP

            # right-side indicators: active target and/or search query
            right_parts: list[str] = []
            active_target = self._targets[self._target_idx] if self._target_idx < len(self._targets) else "ALL"
            if active_target != "ALL":
                right_parts.append(f"@{active_target}")
            if self._search_q:
                right_parts.append(f"/{self._search_q}")
            if right_parts:
                ctx.text(w - PAD, AppBar.BAND_H / 2 - TEXT_HINT / 2,
                         "  ".join(right_parts),
                         size=TEXT_HINT, color=ctx.theme.accent, align="right")

        # ── L1 footer ───────────────────────────────────────────────────────
        footer = FooterKeys(self._footer_shortcuts())
        foot_h = footer.measure(w)
        foot_y = h - foot_h
        footer.render(ctx, 0.0, foot_y, w, foot_h)

        # Copy-mode status label (right side of footer)
        if self._copy_mode:
            lo, hi = self._copy_range(len(filtered))
            n_sel = hi - lo + 1
            label = f"COPY — {n_sel} line{'s' if n_sel != 1 else ''}"
            ctx.text(w - PAD, foot_y + foot_h / 2 - TEXT_HINT / 2,
                     label, size=TEXT_HINT, color=COPY_ROW_FG, align="right")
        elif self._search_mode:
            ctx.text(w - PAD, foot_y + foot_h / 2 - TEXT_HINT / 2,
                     "SEARCH", size=TEXT_HINT, color=ctx.theme.accent, align="right")
        else:
            ctx.text(w - PAD, foot_y + foot_h / 2 - TEXT_HINT / 2,
                     f"{len(filtered)} lines", size=TEXT_HINT, color=ctx.theme.muted,
                     align="right")

        # ── log rows ────────────────────────────────────────────────────────
        # Custom pixel-level rendering: timestamp column, colored level badges,
        # target column, message column, copy-mode row highlighting.
        # No L1 equivalent exists for this dense columnar layout with per-row
        # background changes and inline badge rendering.
        list_y = appbar_h
        list_h = foot_y - list_y
        self._viewport_h = list_h

        if not filtered:
            ctx.text(PAD, list_y + 16, "no log entries",
                     size=TEXT_CAPTION, color=ctx.theme.muted)
            return

        ctx.push_clip(0, list_y, w, list_h)

        n        = len(filtered)
        time_w   = 66.0
        target_w = 140.0
        first    = int(self._scroll / ROW_H)
        count    = int(list_h / ROW_H) + 2
        sel_lo, sel_hi = self._copy_range(n)

        for i in range(first, min(first + count, n)):
            ll    = filtered[i]
            row_y = list_y + i * ROW_H - self._scroll

            in_selection  = self._copy_mode and sel_lo <= i <= sel_hi
            is_cursor_row = self._copy_mode and i == self._copy_row

            if is_cursor_row:
                ctx.rect(0, row_y, w, ROW_H, COPY_CURSOR_BG)
            elif in_selection:
                ctx.rect(0, row_y, w, ROW_H, COPY_ROW_BG)
            elif i % 2 == 0:
                ctx.rect(0, row_y, w, ROW_H, ROW_ALT)

            x      = PAD
            text_y = row_y + ROW_H / 2 - TEXT_HINT / 2
            dim_fg = COPY_ROW_FG if in_selection else ctx.theme.muted
            msg_fg = COPY_ROW_FG if in_selection else ctx.theme.fg

            # timestamp
            ctx.text(x, text_y, ll.time,
                     size=TEXT_HINT, color=dim_fg, monospace=True)
            x += time_w + COL_GAP

            # level badge — fixed-width rect + centred text so all level labels
            # (ERROR/WARN/INFO/DEBUG) occupy an identical column width.
            badge_fill = level_badge_fill.get(ll.level, ctx.theme.highlight)
            badge_fg   = ctx.theme.bg if ll.level in ("ERROR", "WARN", "INFO") else ctx.theme.fg
            badge_y    = row_y + (ROW_H - BADGE_H) / 2
            ctx.rect(x, badge_y, BADGE_W, BADGE_H, badge_fill, radius=BADGE_R)
            ctx.text(x + BADGE_W / 2, badge_y + BADGE_H / 2,
                     ll.level[:4], size=10.0, color=badge_fg,
                     bold=True, monospace=True, align="center")
            x += BADGE_ADV

            # target
            ctx.text(x, text_y, ll.target,
                     size=TEXT_HINT, color=dim_fg,
                     max_width=target_w, elide=True)
            x += target_w + COL_GAP

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
            ctx.rect(w - 4, list_y, 4, list_h, ctx.theme.highlight)
            ctx.rect(w - 4, thumb_y, 4, thumb_h, ctx.theme.muted)


LogsApp().run()
