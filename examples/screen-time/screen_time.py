#!/usr/bin/env python3
"""Screen Time — clock + monthly views from daily_log *_screen.jsonl files.

Layout:
  - Top bar chrome (AppBar, KeyRow hints, Footer) uses SDK v2 components.
  - The clock face + month grid are drawn with primitive ctx.circle/arc/text —
    they're data surfaces, not chrome.
"""

import datetime
import json
import math
import os

from plexi_sdk import App, RenderContext
from plexi_sdk.ui import (
    BG, FG, MUTED, ACCENT, SURFACE, HIGHLIGHT,
    TEXT_HINT, TEXT_CAPTION, TEXT_BODY, TEXT_HEADING, TEXT_TITLE, TEXT_TITLE_XL,
    SPACE_XS, SPACE_SM, SPACE_MD, SPACE_LG, SPACE_XL,
    RADIUS_MD,
    Column, AppBar, Footer, FooterKeys, KeyRow, Spacer,
)

DEFAULT_LOG_DIR = os.path.expanduser("~/Documents/github/daily_log")

# 15-minute bucket granularity → 96 cells around the clock face.
BUCKET_MIN = 15
BUCKETS_PER_DAY = (24 * 60) // BUCKET_MIN  # 96

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
    "#eba0ac",  # maroon
    "#74c7ec",  # sapphire
]
OTHER_COLOR = "#45475a"
EMPTY_CELL_COLOR = "#252538"

# Consistent app→color mapping so the same app always gets the same color.
_app_color_cache: dict[str, str] = {}
_color_idx = 0


def app_color(name: str) -> str:
    global _color_idx
    if name not in _app_color_cache:
        _app_color_cache[name] = SLICE_COLORS[_color_idx % len(SLICE_COLORS)]
        _color_idx += 1
    return _app_color_cache[name]


# ── Data loading ──────────────────────────────────────────────────────────────


def _parse_ts(s: str) -> datetime.datetime | None:
    """Parse an ISO timestamp from the jsonl. Entries are written as naive
    local times (e.g. ``2026-04-22T20:37:14``), so we treat them as local.
    If a tz offset is present we convert to local wall-clock time before use.
    """
    try:
        t = datetime.datetime.fromisoformat(s)
    except (TypeError, ValueError):
        return None
    if t.tzinfo is not None:
        # Convert aware timestamp to local wall-clock, then drop the tz so
        # all downstream math treats values uniformly as naive local.
        t = t.astimezone().replace(tzinfo=None)
    return t


def load_day(log_dir: str, date: datetime.date) -> list[dict]:
    """Load records for ``date`` and filter to ones whose local timestamp
    actually falls on that calendar day.

    The ``YYYY-MM-DD_screen.jsonl`` file is written by the collector when
    records are flushed, so its name doesn't guarantee every contained
    record lands on that date. Late-night sessions routinely produce a
    first entry timestamped "yesterday evening" that the collector only
    flushed after local midnight — without this filter those seconds
    bleed into the following day's totals.
    """
    path = os.path.join(log_dir, f"{date.isoformat()}_screen.jsonl")
    records: list[dict] = []
    try:
        with open(path) as fh:
            for line in fh:
                line = line.strip()
                if not line:
                    continue
                try:
                    rec = json.loads(line)
                except (json.JSONDecodeError, ValueError):
                    continue
                if not rec.get("app") or rec.get("secs", 0) <= 0:
                    continue
                ts = _parse_ts(rec.get("t", ""))
                if ts is None or ts.date() != date:
                    continue
                rec["_ts"] = ts  # cached parsed timestamp
                records.append(rec)
    except OSError:
        pass
    return records


def aggregate_day(records: list[dict]) -> dict[str, float]:
    """Sum seconds per app."""
    totals: dict[str, float] = {}
    for rec in records:
        totals[rec["app"]] = totals.get(rec["app"], 0) + float(rec.get("secs", 0))
    return totals


def aggregate_buckets(records: list[dict]) -> list[dict[str, float]]:
    """Group usage into 15-minute buckets → list[96] of {app: secs}."""
    buckets: list[dict[str, float]] = [dict() for _ in range(BUCKETS_PER_DAY)]
    for rec in records:
        ts: datetime.datetime | None = rec.get("_ts")
        if ts is None:
            continue
        idx = (ts.hour * 60 + ts.minute) // BUCKET_MIN
        if 0 <= idx < BUCKETS_PER_DAY:
            app = rec["app"]
            secs = float(rec.get("secs", 0))
            buckets[idx][app] = buckets[idx].get(app, 0) + secs
    return buckets


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


# ── Sanity test (runs at import time in debug; cheap, deterministic) ─────────

def _selftest_day_boundary() -> None:
    """An 11:55pm local entry must land in the final bucket of that day, and
    a 12:05am entry the next day must land in the first bucket of the next
    day — never the other way around."""
    late = {"t": "2026-04-22T23:55:00", "app": "X", "secs": 60}
    early = {"t": "2026-04-23T00:05:00", "app": "X", "secs": 60}

    parsed_late = _parse_ts(late["t"]) ; assert parsed_late is not None
    parsed_early = _parse_ts(early["t"]) ; assert parsed_early is not None
    assert parsed_late.date() == datetime.date(2026, 4, 22)
    assert parsed_early.date() == datetime.date(2026, 4, 23)

    late["_ts"] = parsed_late
    early["_ts"] = parsed_early

    late_buckets = aggregate_buckets([late])
    early_buckets = aggregate_buckets([early])
    assert late_buckets[BUCKETS_PER_DAY - 1].get("X") == 60
    assert early_buckets[0].get("X") == 60


_selftest_day_boundary()


# ── App ───────────────────────────────────────────────────────────────────────

MODE_CLOCK = "clock"
MODE_MONTH = "month"
MODE_SETTINGS = "settings"


class ScreenTimeApp(App):

    def on_init(self, _ctx: RenderContext) -> None:
        self._log_dir = DEFAULT_LOG_DIR
        self._mode = MODE_CLOCK
        self._view_date = datetime.date.today()

        # Clock view state
        self._buckets: list[dict[str, float]] = [dict() for _ in range(BUCKETS_PER_DAY)]
        self._day_slices: list[tuple[str, float, str]] = []
        self._day_total: float = 0.0

        # Monthly view state
        self._month_data: list[tuple[datetime.date, list[tuple[str, float, str]], float]] = []

        self._error = ""
        self._settings_buf = DEFAULT_LOG_DIR

        self._load_all()

    # ── Loading (synchronous — files are tiny) ──────────────────────────────
    #
    # Every jsonl file is <100KB. Reading 30 of them from local disk is a few
    # ms — not worth a background thread, and doing it inline avoids the
    # "loading spinner flash" on H/L day-nav that the thread-based design
    # produced.

    def _load_all(self) -> None:
        self._error = ""
        try:
            records = load_day(self._log_dir, self._view_date)
            self._buckets = aggregate_buckets(records)
            day_totals = aggregate_day(records)
            self._day_slices = slices_from_totals(day_totals)
            self._day_total = sum(day_totals.values())

            today = datetime.date.today()
            month_data = []
            for i in range(30):
                d = today - datetime.timedelta(days=29 - i)
                recs = load_day(self._log_dir, d)
                totals = aggregate_day(recs)
                slices = slices_from_totals(totals, max_named=6)
                total = sum(totals.values())
                month_data.append((d, slices, total))
            self._month_data = month_data
        except Exception as e:  # noqa: BLE001 — surfaced to UI
            self._error = f"{type(e).__name__}: {e}"
            self.emit.warn(f"screen-time load failed: {e}")

    def _load_day_only(self) -> None:
        """Reload just the current day — used for H/L nav so we don't pay
        for 30 file reads on every arrow press."""
        try:
            records = load_day(self._log_dir, self._view_date)
            self._buckets = aggregate_buckets(records)
            day_totals = aggregate_day(records)
            self._day_slices = slices_from_totals(day_totals)
            self._day_total = sum(day_totals.values())
        except Exception as e:  # noqa: BLE001
            self._error = f"{type(e).__name__}: {e}"
            self.emit.warn(f"screen-time day-reload failed: {e}")

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
            self._load_all()
        elif key in ("left", "h") and self._mode == MODE_CLOCK:
            self._view_date -= datetime.timedelta(days=1)
            self._load_day_only()
        elif key in ("right", "l") and self._mode == MODE_CLOCK:
            if self._view_date < datetime.date.today():
                self._view_date += datetime.timedelta(days=1)
                self._load_day_only()

    def _settings_key(self, key: str) -> None:
        if key == "escape":
            self._mode = MODE_CLOCK
        elif key == "return":
            self._log_dir = os.path.expanduser(self._settings_buf.strip())
            self._mode = MODE_CLOCK
            self._load_all()
        elif key == "backspace":
            self._settings_buf = self._settings_buf[:-1]
        elif key == "space":
            self._settings_buf += " "
        elif len(key) == 1:
            self._settings_buf += key

    # ── Render ────────────────────────────────────────────────────────────────

    def on_render(self, ctx: RenderContext) -> None:
        # Chrome first via the SDK v2 declarative tree — this clears the
        # pane, draws the title/subtitle/divider (Header) and the footer
        # caption. We then paint the data surface (clock/grid/settings) on
        # top of the middle spacer the Column reserved.
        ctx.render(self._chrome_tree())

        if self._error:
            ctx.text(SPACE_XL, ctx.h / 2, f"Error: {self._error}",
                     size=TEXT_BODY, color="#f38ba8", align="left_center")
            return

        if self._mode == MODE_SETTINGS:
            self._draw_settings(ctx)
        elif self._mode == MODE_MONTH:
            self._draw_month(ctx)
        else:
            self._draw_clock(ctx)

    # ── Chrome (SDK v2) ─────────────────────────────────────────────────────

    # Padding/gap used by the chrome Column — exposed so the content-area
    # geometry stays in sync with what the Column reserves.
    _CHROME_PAD = SPACE_XL
    _CHROME_GAP = SPACE_MD

    def _chrome_tree(self) -> Column:
        if self._mode == MODE_CLOCK:
            hints = [
                KeyRow("h/l", "previous / next day"),
                KeyRow("m", "month grid"),
                KeyRow("s", "set log directory"),
                KeyRow("r", "refresh"),
            ]
            footer_text = "15-minute buckets · local-time · today only shows clock hand"
        elif self._mode == MODE_MONTH:
            hints = [
                KeyRow("c", "clock view"),
                KeyRow("s", "set log directory"),
                KeyRow("r", "refresh"),
            ]
            footer_text = "30-day grid · pie = per-app share · bold border = today"
        else:
            hints = [
                KeyRow("↵", "save & close"),
                KeyRow("esc", "cancel"),
            ]
            footer_text = "Path is expanded (~ ok)."

        # Chrome is intentionally minimal: Header + grow-spacer + footer chrome.
        # Context text (Footer) and key chips (FooterKeys) are stacked so the
        # content area (clock / month grid / settings form) owns the middle.
        footer_components: list
        if self._mode == MODE_SETTINGS:
            # Settings: only context text, no key chips (shortcuts shown inline)
            footer_components = [Footer(footer_text)]
        else:
            footer_components = [
                Footer(footer_text),
                FooterKeys([(kr.key, kr.description) for kr in hints]),
            ]
        children: list = [
            AppBar("Screen Time", accent=ACCENT),
            Spacer(grow=True),
            *footer_components,
        ]
        return Column(children, padding=self._CHROME_PAD, gap=self._CHROME_GAP)

    def _content_top(self) -> float:
        """Y-coordinate where the primitive content layer starts, just below
        the Header component (which the Column renders at top padding)."""
        # Mirror the Header component's measure(): title + gap + subtitle
        # + gap + divider. Keep this in sync with plexi_sdk.ui.Header.
        header_h = TEXT_TITLE_XL + 6.0 + TEXT_HINT + 14.0 + 1.0
        return self._CHROME_PAD + header_h + self._CHROME_GAP

    def _content_bottom(self, ctx: RenderContext) -> float:
        """Y-coordinate where footer chrome begins.

        Non-settings modes have two footer components stacked (Footer + FooterKeys).
        Footer height formula: TOP_GAP + 1 + TOP_GAP + lines * LINE_H.
        FooterKeys height formula: TOP_GAP + 1 + TOP_GAP + ROW_H.
        Be conservative (2 lines for Footer) to match the worst-case wrap.
        """
        line_h = TEXT_HINT + 5.0
        footer_h = SPACE_MD + 1.0 + SPACE_MD + 2 * line_h
        if self._mode != MODE_SETTINGS:
            from plexi_sdk.ui import FooterKeys as _FK
            footer_keys_h = _FK.TOP_GAP + 1.0 + _FK.TOP_GAP + _FK.ROW_H
            footer_h += self._CHROME_GAP + footer_keys_h
        return ctx.h - self._CHROME_PAD - footer_h - self._CHROME_GAP

    # ── Clock view ────────────────────────────────────────────────────────────

    def _draw_clock(self, ctx: RenderContext) -> None:
        top = self._content_top()
        bottom = self._content_bottom(ctx)
        available_h = bottom - top

        wide = ctx.w > ctx.h * 0.9
        if wide:
            clock_area_w = ctx.w * 0.6
            legend_x = clock_area_w + SPACE_MD
            legend_w = ctx.w - legend_x - SPACE_XL
        else:
            clock_area_w = ctx.w
            legend_x = SPACE_XL
            legend_w = ctx.w - SPACE_XL * 2

        r_outer = min(clock_area_w / 2, available_h / 2) * 0.78
        r_inner = r_outer * 0.55
        cx = clock_area_w / 2
        cy = top + (available_h / 2 if wide else available_h * 0.42)

        self._draw_clock_ring(ctx, cx, cy, r_outer, r_inner)
        legend_y = top + (SPACE_MD if wide else available_h * 0.82)
        self._draw_clock_legend(ctx, legend_x, legend_y, legend_w, bottom, wide)

    def _draw_clock_ring(self, ctx: RenderContext,
                         cx: float, cy: float,
                         r_outer: float, r_inner: float) -> None:
        now = datetime.datetime.now()
        is_today = self._view_date == datetime.date.today()

        seg_span = 2 * math.pi / BUCKETS_PER_DAY
        # Gap scales with segment size — thin at 15-min granularity.
        gap = seg_span * 0.08

        for idx in range(BUCKETS_PER_DAY):
            start = -math.pi / 2 + idx * seg_span
            end = start + seg_span - gap

            bucket = self._buckets[idx]
            if bucket:
                dominant = max(bucket, key=lambda k: bucket[k])
                color = app_color(dominant)
            else:
                color = EMPTY_CELL_COLOR

            ctx.arc(cx, cy, r_outer, start, end, color)

        # Inner circle creates the donut.
        ctx.circle(cx, cy, r_inner, BG)

        # ── Center label (total hours + date) ───────────────────────────
        total_h = self._day_total / 3600
        lbl = f"{total_h:.1f}h" if total_h > 0 else "—"
        ctx.text(cx, cy - 4, lbl,
                 size=TEXT_TITLE, color=FG, bold=True, align="center")
        sublbl = "today" if is_today else self._view_date.strftime("%-m/%-d")
        ctx.text(cx, cy + 14, sublbl,
                 size=TEXT_HINT, color=MUTED, align="center")

        # ── Hour labels at 12 / 3 / 6 / 9 o'clock ───────────────────────
        for hour in (0, 6, 12, 18):
            angle = -math.pi / 2 + hour * (2 * math.pi / 24)
            label = {0: "12a", 6: "6a", 12: "12p", 18: "6p"}[hour]
            label_r = r_outer + 16
            lx = cx + math.cos(angle) * label_r
            ly = cy + math.sin(angle) * label_r
            ctx.text(lx, ly, label,
                     size=TEXT_HINT, color=MUTED, align="center")

        # ── Elapsed arc + clock hand + now-dot (today only) ─────────────
        if is_today:
            elapsed_min = now.hour * 60 + now.minute
            elapsed_frac = elapsed_min / (24 * 60)
            now_angle = -math.pi / 2 + elapsed_frac * 2 * math.pi

            # Thin elapsed-time ring just inside the donut inner edge.
            # We render it as a sequence of very thin arc segments so it
            # visually reads as a ring rather than a pie wedge.
            ring_r = r_inner - 6
            if ring_r > 8 and elapsed_frac > 0:
                steps = max(1, int(elapsed_frac * BUCKETS_PER_DAY))
                sweep = elapsed_frac * 2 * math.pi
                step_span = sweep / steps
                for i in range(steps):
                    s = -math.pi / 2 + i * step_span
                    e = s + step_span
                    ctx.arc(cx, cy, ring_r, s, e, ACCENT)
                ctx.circle(cx, cy, ring_r - 2, BG)

            # Clock hand — line from center to the ring edge at `now`.
            hand_r = r_outer * 0.95
            hx = cx + math.cos(now_angle) * hand_r
            hy = cy + math.sin(now_angle) * hand_r
            ctx.line(cx, cy, hx, hy, ACCENT, width=2.0)

            # Current-time dot, just outside the ring.
            tick_r = r_outer + 4
            ctx.circle(
                cx + math.cos(now_angle) * tick_r,
                cy + math.sin(now_angle) * tick_r,
                4, ACCENT,
            )

    def _draw_clock_legend(self, ctx: RenderContext,
                           x: float, y: float, w: float,
                           bottom: float, wide: bool) -> None:
        row_h = 22.0
        cols = 1 if wide else 2
        col_w = w / cols
        for i, (name, secs, color) in enumerate(self._day_slices):
            col = i % cols
            row = i // cols
            lx = x + col * col_w
            ly = y + row * row_h
            if ly + row_h > bottom:
                break
            ctx.circle(lx + 5, ly + 10, 5, color)
            ctx.text(lx + 16, ly + row_h / 2, name[:18],
                     size=TEXT_CAPTION, color=FG, align="left_center")
            stat = f"{secs/3600:.1f}h"
            ctx.text(lx + col_w - SPACE_SM, ly + row_h / 2, stat,
                     size=TEXT_CAPTION, color=MUTED, align="right_center")

    # ── Monthly view ─────────────────────────────────────────────────────────

    def _draw_month(self, ctx: RenderContext) -> None:
        top = self._content_top()
        bottom = self._content_bottom(ctx)
        available_w = ctx.w - SPACE_XL * 2
        available_h = bottom - top

        cols = 6
        rows = 5
        cell_w = available_w / cols
        cell_h = available_h / rows

        for i, (date, slices, total) in enumerate(self._month_data):
            col = i % cols
            row = i // cols
            cx = SPACE_XL + col * cell_w + cell_w / 2
            cy = top + row * cell_h + cell_h * 0.45
            r = min(cell_w, cell_h) * 0.30

            if date == datetime.date.today():
                ctx.rect(SPACE_XL + col * cell_w + 2, top + row * cell_h + 2,
                         cell_w - 4, cell_h - 4, SURFACE, radius=RADIUS_MD)

            if slices and total > 0:
                angle = -math.pi / 2
                for _name, secs, color in slices:
                    end = angle + secs / total * 2 * math.pi
                    ctx.arc(cx, cy, r, angle, end, color)
                    angle = end
                ctx.circle(cx, cy, r * 0.4, BG)
            else:
                ctx.circle(cx, cy, r, HIGHLIGHT)

            day_label = str(date.day)
            ctx.text(cx, cy - r - 8, day_label,
                     size=TEXT_HINT,
                     color=ACCENT if date == datetime.date.today() else MUTED,
                     align="center")

            if total > 0:
                h_lbl = f"{total/3600:.0f}h"
                ctx.text(cx, cy + r + 10, h_lbl,
                         size=TEXT_HINT, color=MUTED, align="center")

    # ── Settings ─────────────────────────────────────────────────────────────

    def _draw_settings(self, ctx: RenderContext) -> None:
        top = self._content_top() + SPACE_LG
        ctx.text(SPACE_XL, top, "Log directory",
                 size=TEXT_HEADING, color=FG, bold=True)
        ctx.text(SPACE_XL, top + TEXT_HEADING + SPACE_XS,
                 "Folder containing *_screen.jsonl files",
                 size=TEXT_CAPTION, color=MUTED)

        box_y = top + TEXT_HEADING + SPACE_XS + TEXT_CAPTION + SPACE_MD
        ctx.rect(SPACE_XL, box_y, ctx.w - SPACE_XL * 2, 44, SURFACE, radius=RADIUS_MD)
        max_chars = max(1, int((ctx.w - SPACE_XL * 2 - SPACE_MD * 2) / 8))
        display = self._settings_buf[-max_chars:] + "▌"
        ctx.text(SPACE_XL + SPACE_MD, box_y + 22, display,
                 size=TEXT_BODY, color=FG, monospace=True, align="left_center")
        ctx.text(SPACE_XL, box_y + 56, "Return to confirm · Esc to cancel",
                 size=TEXT_HINT, color=MUTED)


if __name__ == "__main__":
    ScreenTimeApp().run()
