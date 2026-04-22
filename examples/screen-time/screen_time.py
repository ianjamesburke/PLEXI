#!/usr/bin/env python3
"""Screen Time — clock, daily, and monthly views from daily_log *_screen.jsonl files."""
from __future__ import annotations

import datetime
import json
import math
import os
import threading

from plexi_sdk import (
    App, RenderContext,
    BG, FG, MUTED, ACCENT, SURFACE, HIGHLIGHT,
    CAPTION, HINT, HEADING, BODY,
    PAD, PAD_TIGHT, HEADER_H,
)

DEFAULT_LOG_DIR = os.path.expanduser("~/Documents/github/daily_log")

SLICE_COLORS = [
    "#89b4fa",  # blue
    "#a6e3a1",  # green
    "#f38ba8",  # red/pink
    "#fab387",  # peach
    "#f9e2af",  # yellow
    "#cba6f7",  # mauve
    "#89dceb",  # sky
    "#f5c2e7",  # pink
    "#94e2d5",  # teal
    "#b4befe",  # lavender
    "#cba6f7",  # mauve 2
    "#eba0ac",  # maroon
]
OTHER_COLOR = "#45475a"
EMPTY_HOUR_COLOR = "#1e1e2e"

# Consistent app→color mapping so the same app always gets the same color
_app_color_cache: dict[str, str] = {}
_color_idx = 0


def app_color(name: str) -> str:
    global _color_idx
    if name not in _app_color_cache:
        _app_color_cache[name] = SLICE_COLORS[_color_idx % len(SLICE_COLORS)]
        _color_idx += 1
    return _app_color_cache[name]


# ── Data loading ──────────────────────────────────────────────────────────────

def load_day(log_dir: str, date: datetime.date) -> list[dict]:
    """Load all records for a single date from its screen.jsonl file."""
    path = os.path.join(log_dir, f"{date.isoformat()}_screen.jsonl")
    records = []
    try:
        with open(path) as fh:
            for line in fh:
                line = line.strip()
                if not line:
                    continue
                try:
                    rec = json.loads(line)
                    if rec.get("app") and rec.get("secs", 0) > 0:
                        records.append(rec)
                except (json.JSONDecodeError, ValueError):
                    continue
    except OSError:
        pass
    return records


def aggregate_day(records: list[dict]) -> dict[str, float]:
    """Sum seconds per app for a day's records."""
    totals: dict[str, float] = {}
    for rec in records:
        app = rec["app"]
        totals[app] = totals.get(app, 0) + float(rec.get("secs", 0))
    return totals


def aggregate_hours(records: list[dict]) -> dict[int, dict[str, float]]:
    """Group usage by hour-of-day → {hour: {app: secs}}."""
    hours: dict[int, dict[str, float]] = {h: {} for h in range(24)}
    for rec in records:
        try:
            t = datetime.datetime.fromisoformat(rec["t"])
            hour = t.hour
            app = rec["app"]
            secs = float(rec.get("secs", 0))
            hours[hour][app] = hours[hour].get(app, 0) + secs
        except (KeyError, ValueError):
            continue
    return hours


def slices_from_totals(totals: dict[str, float], max_named: int = 8
                        ) -> list[tuple[str, float, str]]:
    """Build (name, secs, color) list sorted desc, with Other bucket."""
    rows = sorted(totals.items(), key=lambda x: x[1], reverse=True)
    result: list[tuple[str, float, str]] = []
    for name, secs in rows[:max_named]:
        result.append((name[:22], secs, app_color(name)))
    rest_secs = sum(s for _, s in rows[max_named:])
    if rest_secs > 0:
        result.append(("Other", rest_secs, OTHER_COLOR))
    return result


# ── App ───────────────────────────────────────────────────────────────────────

MODE_CLOCK   = "clock"
MODE_MONTH   = "month"
MODE_SETTINGS = "settings"

SPINNER_FRAMES = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]


class ScreenTimeApp(App):

    def on_init(self, _ctx: RenderContext) -> None:
        self._log_dir = DEFAULT_LOG_DIR
        self._mode = MODE_CLOCK
        self._view_date = datetime.date.today()

        # Clock view state
        self._hour_data: dict[int, dict[str, float]] = {}
        self._day_slices: list[tuple[str, float, str]] = []
        self._day_total: float = 0.0

        # Monthly view state
        self._month_data: list[tuple[datetime.date, list[tuple[str, float, str]], float]] = []

        self._loading = True
        self._spinner = 0
        self._error = ""
        self._settings_buf = DEFAULT_LOG_DIR

        self._fetch()

    # ── Fetching ─────────────────────────────────────────────────────────────

    def _fetch(self) -> None:
        self._loading = True
        self._error = ""
        self.emit.status_summary("Loading…")
        log_dir = self._log_dir
        view_date = self._view_date

        def run() -> None:
            try:
                # Clock view: load selected day
                records = load_day(log_dir, view_date)
                self._hour_data = aggregate_hours(records)
                day_totals = aggregate_day(records)
                self._day_slices = slices_from_totals(day_totals)
                self._day_total = sum(day_totals.values())

                # Monthly view: last 30 days
                today = datetime.date.today()
                month_data = []
                for i in range(30):
                    d = today - datetime.timedelta(days=29 - i)
                    recs = load_day(log_dir, d)
                    totals = aggregate_day(recs)
                    slices = slices_from_totals(totals, max_named=6)
                    total = sum(totals.values())
                    month_data.append((d, slices, total))
                self._month_data = month_data
            except Exception as e:
                self._error = str(e)
            finally:
                self._loading = False
                self.emit.status_summary("")
                self.emit.schedule_render(after_ms=0)

        threading.Thread(target=run, daemon=True).start()

    # ── Input ─────────────────────────────────────────────────────────────────

    def on_key(self, _ctx: RenderContext, key: str, _mods: dict) -> None:
        if self._mode == MODE_SETTINGS:
            self._settings_key(key)
            return

        if key == "c":
            self._mode = MODE_CLOCK
        elif key == "m":
            self._mode = MODE_MONTH
        elif key == "s":
            self._mode = MODE_SETTINGS
            self._settings_buf = self._log_dir
        elif key == "r":
            self._fetch()
        elif key in ("left", "h") and self._mode == MODE_CLOCK:
            self._view_date -= datetime.timedelta(days=1)
            self._fetch()
        elif key in ("right", "l") and self._mode == MODE_CLOCK:
            if self._view_date < datetime.date.today():
                self._view_date += datetime.timedelta(days=1)
                self._fetch()

    def _settings_key(self, key: str) -> None:
        if key == "escape":
            self._mode = MODE_CLOCK
        elif key == "return":
            self._log_dir = os.path.expanduser(self._settings_buf.strip())
            self._mode = MODE_CLOCK
            self._fetch()
        elif key == "backspace":
            self._settings_buf = self._settings_buf[:-1]
        elif key == "space":
            self._settings_buf += " "
        elif len(key) == 1:
            self._settings_buf += key

    # ── Render ────────────────────────────────────────────────────────────────

    def on_render(self, ctx: RenderContext) -> None:
        ctx.clear(BG)
        self._draw_header(ctx)

        if self._loading:
            self._spinner = (self._spinner + 1) % len(SPINNER_FRAMES)
            ctx.text(PAD, ctx.h / 2 - 10,
                     f"{SPINNER_FRAMES[self._spinner]} Loading…",
                     size=BODY, color=MUTED)
            ctx.emit.schedule_render(after_ms=120)
            return

        if self._error:
            ctx.text(PAD, ctx.h / 2 - 10, f"Error: {self._error}",
                     size=BODY, color="#f38ba8")
            return

        if self._mode == MODE_SETTINGS:
            self._draw_settings(ctx)
        elif self._mode == MODE_MONTH:
            self._draw_month(ctx)
        else:
            self._draw_clock(ctx)

    # ── Header ────────────────────────────────────────────────────────────────

    def _draw_header(self, ctx: RenderContext) -> None:
        ctx.rect(0, 0, ctx.w, HEADER_H, fill=SURFACE)
        ctx.text(PAD, 12, "Screen Time", size=HEADING, color=ACCENT, bold=True)
        mode_label = "Clock" if self._mode == MODE_CLOCK else (
            "Month" if self._mode == MODE_MONTH else "Settings")
        ctx.text(PAD, 30, mode_label, size=HINT, color=MUTED)
        ctx.text(ctx.w - 180, 20, "c=clock  m=month  s=dir  r=refresh",
                 size=HINT, color=MUTED)

    # ── Clock view ────────────────────────────────────────────────────────────

    def _draw_clock(self, ctx: RenderContext) -> None:
        top = HEADER_H + PAD

        # Date nav
        date_str = self._view_date.strftime("%A, %B %-d")
        ctx.text(PAD, top + 4, date_str, size=HEADING, color=FG, bold=True)
        ctx.text(ctx.w - 120, top + 4, "← h  l →", size=HINT, color=MUTED)

        top += 28

        # Decide layout
        available_h = ctx.h - top
        wide = ctx.w > ctx.h * 0.9
        if wide:
            clock_area_w = ctx.w * 0.55
            legend_x = clock_area_w + PAD
            legend_w = ctx.w - legend_x - PAD
        else:
            clock_area_w = ctx.w
            legend_x = PAD
            legend_w = ctx.w - PAD * 2

        r_outer = min(clock_area_w / 2, available_h / 2) * 0.78
        r_inner = r_outer * 0.55
        cx = clock_area_w / 2
        cy = top + (available_h / 2 if wide else available_h * 0.42)

        self._draw_clock_ring(ctx, cx, cy, r_outer, r_inner)
        self._draw_clock_legend(ctx, legend_x, top + (4 if wide else available_h * 0.86),
                                legend_w, wide)

    def _draw_clock_ring(self, ctx: RenderContext,
                         cx: float, cy: float,
                         r_outer: float, r_inner: float) -> None:
        now_hour = datetime.datetime.now().hour if self._view_date == datetime.date.today() else -1

        for hour in range(24):
            start = -math.pi / 2 + hour * (2 * math.pi / 24)
            end = start + (2 * math.pi / 24) - 0.02  # small gap between segments

            hour_apps = self._hour_data.get(hour, {})
            if hour_apps:
                dominant = max(hour_apps, key=lambda k: hour_apps[k])
                color = app_color(dominant)
            else:
                color = EMPTY_HOUR_COLOR

            # Draw outer ring segment
            ctx.arc(cx, cy, r_outer, start, end, color)

        # Draw inner circle to create donut
        ctx.circle(cx, cy, r_inner, BG)

        # Center text
        total_h = self._day_total / 3600
        if total_h > 0:
            lbl = f"{total_h:.1f}h"
            ctx.text(cx - len(lbl) * 4, cy - 12, lbl, size=HEADING, color=FG, bold=True)
        ctx.text(cx - 12, cy + 6, "today" if self._view_date == datetime.date.today() else
                 self._view_date.strftime("%-m/%-d"), size=HINT, color=MUTED)

        # Hour tick marks at 12, 3, 6, 9 (and labels)
        for hour in [0, 6, 12, 18]:
            angle = -math.pi / 2 + hour * (2 * math.pi / 24)
            label = {0: "12a", 6: "6a", 12: "12p", 18: "6p"}[hour]
            label_r = r_outer + 16
            lx = cx + math.cos(angle) * label_r
            ly = cy + math.sin(angle) * label_r
            ctx.text(lx - len(label) * 3, ly - 6, label, size=HINT, color=MUTED)

        # Current-hour highlight: bright tick on ring edge
        if 0 <= now_hour < 24:
            angle = -math.pi / 2 + now_hour * (2 * math.pi / 24) + math.pi / 24
            tick_r = r_outer + 4
            ctx.circle(
                cx + math.cos(angle) * tick_r,
                cy + math.sin(angle) * tick_r,
                3, ACCENT,
            )

    def _draw_clock_legend(self, ctx: RenderContext,
                           x: float, y: float, w: float, wide: bool) -> None:
        row_h = 22.0
        cols = 2 if not wide else 1
        col_w = w / cols
        for i, (name, secs, color) in enumerate(self._day_slices):
            col = i % cols
            row = i // cols
            lx = x + col * col_w
            ly = y + row * row_h
            if ly + row_h > ctx.h - PAD:
                break
            ctx.circle(lx + 5, ly + 8, 5, color)
            ctx.text(lx + 14, ly + 2, name[:18], size=CAPTION, color=FG)
            stat = f"{secs/3600:.1f}h"
            ctx.text(lx + col_w - len(stat) * 6.5, ly + 2, stat,
                     size=CAPTION, color=MUTED)

    # ── Monthly view ─────────────────────────────────────────────────────────

    def _draw_month(self, ctx: RenderContext) -> None:
        top = HEADER_H + PAD
        available_w = ctx.w - PAD * 2
        available_h = ctx.h - top - PAD

        cols = 6
        rows = 5
        cell_w = available_w / cols
        cell_h = available_h / rows

        for i, (date, slices, total) in enumerate(self._month_data):
            col = i % cols
            row = i // cols
            cx = PAD + col * cell_w + cell_w / 2
            cy = top + row * cell_h + cell_h * 0.45
            r = min(cell_w, cell_h) * 0.30

            # Cell background for today
            if date == datetime.date.today():
                ctx.rect(PAD + col * cell_w + 2, top + row * cell_h + 2,
                         cell_w - 4, cell_h - 4, fill=SURFACE, radius=6.0)

            if slices and total > 0:
                angle = -math.pi / 2
                for _name, secs, color in slices:
                    end = angle + secs / total * 2 * math.pi
                    ctx.arc(cx, cy, r, angle, end, color)
                    angle = end
                ctx.circle(cx, cy, r * 0.4, BG)
            else:
                ctx.circle(cx, cy, r, HIGHLIGHT)

            # Day number label
            day_label = str(date.day)
            lx = cx - len(day_label) * 3.5
            ctx.text(lx, cy - r - 14, day_label, size=HINT,
                     color=ACCENT if date == datetime.date.today() else MUTED)

            # Hours label below pie
            if total > 0:
                h_lbl = f"{total/3600:.0f}h"
                ctx.text(cx - len(h_lbl) * 3, cy + r + 4, h_lbl,
                         size=HINT, color=MUTED)

    # ── Settings ─────────────────────────────────────────────────────────────

    def _draw_settings(self, ctx: RenderContext) -> None:
        top = HEADER_H + 48
        ctx.text(PAD, top, "Log directory", size=HEADING, color=FG, bold=True)
        ctx.text(PAD, top + 24, "Folder containing *_screen.jsonl files",
                 size=CAPTION, color=MUTED)
        box_y = top + 56
        ctx.rect(PAD, box_y, ctx.w - PAD * 2, 44, fill=SURFACE, radius=8.0)
        max_chars = max(1, int((ctx.w - PAD * 2 - PAD_TIGHT * 2) / 8))
        display = self._settings_buf[-max_chars:] + "▌"
        ctx.text(PAD + PAD_TIGHT, box_y + 13, display,
                 size=BODY, color=FG, monospace=True)
        ctx.text(PAD, box_y + 56, "Return to confirm · Esc to cancel",
                 size=HINT, color=MUTED)


if __name__ == "__main__":
    ScreenTimeApp().run()
