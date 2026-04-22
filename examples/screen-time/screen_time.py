#!/usr/bin/env python3
"""Screen Time — pie chart of app usage from daily_log *_screen.jsonl files."""
from __future__ import annotations

import datetime
import glob
import json
import math
import os
import threading

from plexi_sdk import (
    App, RenderContext,
    BG, FG, MUTED, ACCENT, SURFACE,
    CAPTION, HINT, HEADING, BODY,
    PAD, PAD_TIGHT, HEADER_H,
)

DEFAULT_LOG_DIR = os.path.expanduser("~/Documents/github/daily_log")
DAYS = 7

SLICE_COLORS = [
    "#89b4fa",  # blue
    "#a6e3a1",  # green
    "#f38ba8",  # red/pink
    "#fab387",  # peach
    "#f9e2af",  # yellow
    "#cba6f7",  # mauve/purple
    "#89dceb",  # sky
    "#f5c2e7",  # pink
    "#94e2d5",  # teal
]
OTHER_COLOR = "#45475a"
MAX_NAMED = 8


def get_app_usage(log_dir: str) -> list[tuple[str, float]]:
    cutoff = datetime.date.today() - datetime.timedelta(days=DAYS)
    totals: dict[str, float] = {}

    for path in glob.glob(os.path.join(log_dir, "*_screen.jsonl")):
        fname = os.path.basename(path)
        try:
            date_str = fname.split("_screen")[0]
            day = datetime.date.fromisoformat(date_str)
        except ValueError:
            continue
        if day < cutoff:
            continue
        try:
            with open(path) as fh:
                for line in fh:
                    line = line.strip()
                    if not line:
                        continue
                    try:
                        rec = json.loads(line)
                        app = rec.get("app", "")
                        secs = float(rec.get("secs", 0))
                        if app and secs > 0:
                            totals[app] = totals.get(app, 0) + secs
                    except (json.JSONDecodeError, ValueError):
                        continue
        except OSError:
            continue

    return sorted(totals.items(), key=lambda x: x[1], reverse=True)


class ScreenTimeApp(App):
    def on_init(self, _ctx: RenderContext) -> None:
        self._log_dir: str = DEFAULT_LOG_DIR
        self._slices: list[tuple[str, float, str]] = []
        self._total_secs: float = 0.0
        self._loading: bool = True
        self._error: str = ""
        self._spinner_frame: int = 0
        self._settings_mode: bool = False
        self._settings_buf: str = DEFAULT_LOG_DIR
        self._fetch()

    # ── Data ─────────────────────────────────────────────────────────────────

    def _fetch(self) -> None:
        self._loading = True
        self._error = ""
        self.emit.status_summary("Loading…")
        log_dir = self._log_dir

        def run() -> None:
            try:
                rows = get_app_usage(log_dir)
                self._build_slices(rows)
            except Exception as e:
                self._error = str(e)
            finally:
                self._loading = False
                self.emit.status_summary("")
                self.emit.schedule_render(after_ms=0)

        threading.Thread(target=run, daemon=True).start()

    def _build_slices(self, rows: list[tuple[str, float]]) -> None:
        total = sum(s for _, s in rows)
        self._total_secs = total
        named = rows[:MAX_NAMED]
        rest = rows[MAX_NAMED:]
        slices: list[tuple[str, float, str]] = []
        for i, (name, secs) in enumerate(named):
            slices.append((name[:22], secs, SLICE_COLORS[i % len(SLICE_COLORS)]))
        if rest:
            slices.append(("Other", sum(s for _, s in rest), OTHER_COLOR))
        self._slices = slices

    # ── Input ─────────────────────────────────────────────────────────────────

    def on_key(self, _ctx: RenderContext, key: str, _mods: dict) -> None:
        if self._settings_mode:
            self._handle_settings_key(key)
        else:
            if key == "r":
                self._fetch()
            elif key == "s":
                self._settings_mode = True
                self._settings_buf = self._log_dir

    def _handle_settings_key(self, key: str) -> None:
        if key == "escape":
            self._settings_mode = False
        elif key == "return":
            path = self._settings_buf.strip()
            if path:
                self._log_dir = os.path.expanduser(path)
                self._settings_mode = False
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

        if self._settings_mode:
            self._draw_settings(ctx)
            return

        if self._loading:
            self._spinner_frame = (self._spinner_frame + 1) % 10
            frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]
            ctx.text(PAD, ctx.h / 2 - 10,
                     f"{frames[self._spinner_frame]} Loading…", size=BODY, color=MUTED)
            ctx.emit.schedule_render(after_ms=120)
            return

        if self._error:
            ctx.text(PAD, ctx.h / 2 - 10, f"Error: {self._error}",
                     size=BODY, color="#f38ba8")
            return

        if not self._slices:
            ctx.text(PAD, ctx.h / 2 - 20,
                     "No screen.jsonl files found in:", size=BODY, color=MUTED)
            ctx.text(PAD, ctx.h / 2 + 4, self._log_dir, size=CAPTION, color=HINT)
            ctx.text(PAD, ctx.h / 2 + 24, "Press s to change directory",
                     size=HINT, color=MUTED)
            return

        self._draw_chart(ctx)

    def _draw_header(self, ctx: RenderContext) -> None:
        ctx.rect(0, 0, ctx.w, HEADER_H, fill=SURFACE)
        ctx.text(PAD, 14, "Screen Time", size=HEADING, color=ACCENT, bold=True)
        if self._total_secs > 0:
            ctx.text(PAD, 32, f"Last {DAYS} days · {self._total_secs/3600:.1f}h total",
                     size=HINT, color=MUTED)
        ctx.text(ctx.w - 100, 18, "r=refresh  s=dir", size=HINT, color=MUTED)

    def _draw_settings(self, ctx: RenderContext) -> None:
        top = HEADER_H + 48
        ctx.text(PAD, top, "Log directory", size=HEADING, color=FG, bold=True)
        ctx.text(PAD, top + 24, "Path to folder containing *_screen.jsonl files",
                 size=CAPTION, color=MUTED)
        box_y = top + 56
        ctx.rect(PAD, box_y, ctx.w - PAD * 2, 44, fill=SURFACE, radius=8.0)
        max_chars = max(1, int((ctx.w - PAD * 2 - PAD_TIGHT * 2) / 8))
        display = self._settings_buf[-max_chars:] + "▌"
        ctx.text(PAD + PAD_TIGHT, box_y + 13, display, size=BODY, color=FG, monospace=True)
        ctx.text(PAD, box_y + 56, "Return to confirm · Esc to cancel",
                 size=HINT, color=MUTED)

    def _draw_chart(self, ctx: RenderContext) -> None:
        top = HEADER_H + PAD
        ch = ctx.h - top

        wide = ctx.w >= ctx.h * 0.85
        if wide:
            aw = ctx.w * 0.5
            cx, cy = aw / 2, top + ch / 2
            r = min(aw / 2, ch / 2) * 0.72
            legend_x, legend_w = aw + PAD, ctx.w - aw - PAD * 2
        else:
            ah = ch * 0.48
            cx, cy = ctx.w / 2, top + ah / 2
            r = min(ctx.w / 2, ah / 2) * 0.80
            legend_x, legend_w = PAD, ctx.w - PAD * 2

        total = self._total_secs
        angle = -math.pi / 2
        for _name, secs, color in self._slices:
            end = angle + secs / total * 2 * math.pi
            ctx.arc(cx, cy, r, angle, end, color)
            angle = end

        # Donut hole + center label
        ctx.circle(cx, cy, r * 0.38, BG)
        lbl = f"{total/3600:.1f}h"
        ctx.text(cx - len(lbl) * 3.5, cy - 10, lbl, size=BODY, color=FG, bold=True)
        ctx.text(cx - 14, cy + 6, "total", size=HINT, color=MUTED)

        legend_top = (top + (ch - len(self._slices) * 28) / 2) if wide else (top + ch * 0.52)
        y = legend_top
        for name, secs, color in self._slices:
            pct = secs / total * 100
            ctx.circle(legend_x + 5, y + 7, 5, color)
            ctx.text(legend_x + 14, y, name, size=CAPTION, color=FG)
            stat = f"{pct:.1f}%  {secs/3600:.1f}h"
            ctx.text(legend_x + legend_w - len(stat) * 6.5, y, stat,
                     size=CAPTION, color=MUTED)
            y += 28


if __name__ == "__main__":
    ScreenTimeApp().run()
