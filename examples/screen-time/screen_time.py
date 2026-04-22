#!/usr/bin/env python3
"""Screen Time — pie chart of macOS app usage from the last 7 days (knowledgeC.db)."""
from __future__ import annotations

import math
import os
import sqlite3
import threading
import time

from plexi_sdk import (
    App, RenderContext,
    BG, FG, MUTED, ACCENT, SURFACE, HIGHLIGHT,
    BODY, CAPTION, HINT, HEADING, TITLE,
    PAD, HEADER_H,
)

# ── Color palette for pie slices (8 distinct colors) ─────────────────────────
SLICE_COLORS = [
    "#89b4fa",  # blue (ACCENT)
    "#a6e3a1",  # green
    "#f38ba8",  # red/pink
    "#fab387",  # peach
    "#f9e2af",  # yellow
    "#cba6f7",  # mauve/purple
    "#89dceb",  # sky
    "#f5c2e7",  # pink
    "#94e2d5",  # teal (overflow)
]
OTHER_COLOR = "#45475a"  # HIGHLIGHT / muted grey for "Other"

MAX_NAMED_SLICES = 8


def _short_name(bundle_id: str) -> str:
    """Strip bundle ID prefix; return last dot-component."""
    parts = bundle_id.rsplit(".", 1)
    name = parts[-1] if parts else bundle_id
    # Capitalise common patterns for readability
    return name[:20]  # cap length


def get_app_usage() -> list[tuple[str, float]]:
    """Return list of (bundle_id, seconds) sorted desc, last 7 days."""
    db_path = os.path.expanduser(
        "~/Library/Application Support/Knowledge/knowledgeC.db"
    )
    if not os.path.exists(db_path):
        return []

    cutoff = time.time() - 7 * 86400
    apple_cutoff = cutoff - 978307200  # Unix → Apple Core Data epoch offset

    try:
        con = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
        cur = con.execute(
            """
            SELECT ZVALUESTRING, SUM(ZENDDATE - ZSTARTDATE)
            FROM ZOBJECT
            WHERE ZSTREAMNAME = '/app/usage'
              AND ZSTARTDATE > ?
              AND ZVALUESTRING IS NOT NULL
            GROUP BY ZVALUESTRING
            ORDER BY 2 DESC
            LIMIT 20
            """,
            (apple_cutoff,),
        )
        rows = [
            (row[0], float(row[1]))
            for row in cur.fetchall()
            if row[1] and row[1] > 0
        ]
        con.close()
        return rows
    except Exception:
        return []


class ScreenTimeApp(App):
    def on_init(self, _ctx: RenderContext) -> None:
        self._rows: list[tuple[str, float]] = []   # raw (bundle_id, secs)
        self._slices: list[tuple[str, float, str]] = []  # (label, secs, color)
        self._total_secs: float = 0.0
        self._loading: bool = True
        self._error: str = ""
        self._spinner_frame: int = 0
        self._fetch()

    # ── Data loading ──────────────────────────────────────────────────────────

    def _fetch(self) -> None:
        self._loading = True
        self._error = ""
        self.emit.status_summary("Loading…")

        def run() -> None:
            try:
                rows = get_app_usage()
                self._rows = rows
                self._build_slices(rows)
            except Exception as e:
                self._error = f"Failed to load: {e}"
            finally:
                self._loading = False
                self.emit.status_summary("")
                self.emit.schedule_render(after_ms=0)

        threading.Thread(target=run, daemon=True).start()

    def _build_slices(self, rows: list[tuple[str, float]]) -> None:
        """Collapse into MAX_NAMED_SLICES + optional 'Other' bucket."""
        total = sum(s for _, s in rows)
        self._total_secs = total

        named = rows[:MAX_NAMED_SLICES]
        rest = rows[MAX_NAMED_SLICES:]

        slices: list[tuple[str, float, str]] = []
        for i, (bundle_id, secs) in enumerate(named):
            label = _short_name(bundle_id)
            color = SLICE_COLORS[i % len(SLICE_COLORS)]
            slices.append((label, secs, color))

        if rest:
            other_secs = sum(s for _, s in rest)
            slices.append(("Other", other_secs, OTHER_COLOR))

        self._slices = slices

    # ── Key handling ──────────────────────────────────────────────────────────

    def on_key(self, _ctx: RenderContext, key: str, _mods: dict) -> None:
        if key == "r":
            self._fetch()

    # ── Render ────────────────────────────────────────────────────────────────

    def on_render(self, ctx: RenderContext) -> None:
        ctx.clear(BG)
        self._draw_header(ctx)

        if self._loading:
            self._draw_spinner(ctx)
            ctx.emit.schedule_render(after_ms=120)  # animate spinner
            return

        if self._error:
            self._draw_error(ctx)
            return

        if not self._slices:
            self._draw_empty(ctx)
            return

        self._draw_chart(ctx)

    def _draw_header(self, ctx: RenderContext) -> None:
        ctx.rect(0, 0, ctx.w, HEADER_H, fill=SURFACE)
        ctx.text(PAD, 14, "Screen Time", size=HEADING, color=ACCENT, bold=True)

        if self._total_secs > 0:
            total_h = self._total_secs / 3600
            summary = f"Last 7 days  ·  {total_h:.1f} h total"
            ctx.text(PAD, 32, summary, size=HINT, color=MUTED)

        ctx.text(ctx.w - 80, 18, "r = refresh", size=HINT, color=MUTED)

    def _draw_spinner(self, ctx: RenderContext) -> None:
        frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]
        self._spinner_frame = (self._spinner_frame + 1) % len(frames)
        cy = ctx.h / 2
        cx = ctx.w / 2
        ctx.text(cx - 30, cy - 10, f"{frames[self._spinner_frame]} Loading…",
                 size=BODY, color=MUTED)

    def _draw_error(self, ctx: RenderContext) -> None:
        ctx.text(PAD, ctx.h / 2 - 10, self._error, size=BODY, color="#f38ba8")

    def _draw_empty(self, ctx: RenderContext) -> None:
        ctx.text(PAD, ctx.h / 2 - 10,
                 "No usage data found in knowledgeC.db",
                 size=BODY, color=MUTED)

    def _draw_chart(self, ctx: RenderContext) -> None:
        """Render pie chart + legend. Adapts to portrait vs landscape layout."""
        content_top = HEADER_H + PAD
        content_h = ctx.h - content_top
        content_w = ctx.w

        # Decide layout: wide pane → chart left / legend right
        # Narrow pane → chart top / legend below
        wide = ctx.w >= ctx.h * 0.85

        if wide:
            chart_area_w = content_w * 0.5
            legend_x = chart_area_w + PAD
            legend_w = content_w - legend_x - PAD
            chart_cx = chart_area_w / 2
            chart_cy = content_top + content_h / 2
            r = min(chart_area_w / 2, content_h / 2) * 0.72
        else:
            chart_area_h = content_h * 0.48
            chart_cx = content_w / 2
            chart_cy = content_top + chart_area_h / 2
            r = min(content_w / 2, chart_area_h / 2) * 0.80
            legend_x = PAD
            legend_w = content_w - 2 * PAD

        # Draw pie slices
        total = self._total_secs
        if total <= 0:
            return

        # Start from north (top), sweep clockwise
        angle = -math.pi / 2

        for label, secs, color in self._slices:
            fraction = secs / total
            end_angle = angle + fraction * 2 * math.pi
            ctx.arc(chart_cx, chart_cy, r, angle, end_angle, color)
            angle = end_angle

        # Draw a small center circle to make it a donut (optional — cleaner look)
        inner_r = r * 0.38
        ctx.circle(chart_cx, chart_cy, inner_r, BG)

        # Center label: total hours
        total_h = total / 3600
        label_txt = f"{total_h:.1f}h"
        # Approximate centering: 7 px per char at CAPTION size
        lw = len(label_txt) * 7
        ctx.text(chart_cx - lw / 2, chart_cy - 10, label_txt,
                 size=BODY, color=FG, bold=True)
        ctx.text(chart_cx - 14, chart_cy + 6, "total", size=HINT, color=MUTED)

        # Draw legend
        if wide:
            legend_top = content_top + (content_h - len(self._slices) * 28) / 2
        else:
            legend_top = content_top + content_h * 0.52

        self._draw_legend(ctx, legend_x, legend_top, legend_w, total)

    def _draw_legend(self, ctx: RenderContext,
                     x: float, y: float, w: float, total: float) -> None:
        row_h = 26.0
        dot_r = 5.0

        for label, secs, color in self._slices:
            pct = secs / total * 100 if total > 0 else 0
            hours = secs / 3600
            # Colored dot
            ctx.circle(x + dot_r, y + dot_r + 2, dot_r, color)
            # App name
            ctx.text(x + dot_r * 2 + 8, y, label, size=CAPTION, color=FG)
            # Percentage + hours — right-aligned within legend width
            stat = f"{pct:.1f}%  {hours:.1f}h"
            stat_w = len(stat) * 6.5
            ctx.text(x + w - stat_w, y, stat, size=CAPTION, color=MUTED)
            y += row_h


if __name__ == "__main__":
    ScreenTimeApp().run()
