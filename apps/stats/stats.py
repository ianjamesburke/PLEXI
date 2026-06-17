#!/usr/bin/env python3
"""Stats — an activity dashboard for your work.

Reads Plexi focus events and renders a circular activity clock — a 24h dial
whose amplitude is your pane-switch velocity through the day and whose colour
is the context you were in, with today's focus total at the centre — plus stat
tiles and a top-contexts list. Built on the native canvas primitives — see
docs/sdk-v2.md.

Data notes (what is and isn't accurately measured):
  * Every focus change is one `focus_changed` event with a microsecond
    `timestamp` and an integer `duration_secs` (1-second resolution). Switch
    counts and timing are exact — that is what drives the waveform amplitude.
  * "Focus time" is the sum of focus dwell. Dwell can't see idle-at-keyboard,
    so very long single-pane spans (e.g. a pane left focused overnight) are
    treated as idle and clamped. Focus time is therefore approximate; switch
    velocity is not.
"""
from __future__ import annotations

import json
import math
import os
from datetime import datetime, timedelta, timezone
from pathlib import Path

from plexi_sdk import App, dim
from plexi_sdk.ui import (
    AppBar, Canvas, Column, FooterKeys,
    RADIUS_MD,
    SPACE_SM, SPACE_LG,
    TEXT_HINT, TEXT_CAPTION, TEXT_HEADING,
)

HOME = str(Path.home())
DAY_START_HOUR = 4              # a "day" runs 4 AM → 4 AM, not midnight
IDLE_THRESHOLD_SECS = 15 * 60  # a single focus span longer than this is treated as idle
IDLE_CLAMP_SECS = 60
WAVE_BUCKETS = 120             # angular resolution of the activity waveform (~12 min each)

RANK_COLORS = ["#f9e2af", "#bac2de", "#fab387"]  # 1st / 2nd / 3rd
CONTEXT_PALETTE = ["#89b4fa", "#a6e3a1", "#f9e2af", "#cba6f7", "#fab387", "#94e2d5"]
TEXT_SOFT = "#a6adc8"  # readable secondary text (~3.7:1 on bg); never use theme.muted for text

# Section heights (logical px).
TILES_H = 70.0
ROW_H = 30.0
DIAL_MIN_H = 180.0
DIAL_MAX_R = 240.0   # the dial keeps growing with the pane up to this radius
TOP_CONTEXTS = 4     # named rows; the rest collapse into one "Other" row
CONTEXT_ROWS = TOP_CONTEXTS + 1  # fixed row count so the layout never jumps
OTHER_COLOR = "#585b70"          # neutral track colour for the "Other" bucket


# ── Data layer ───────────────────────────────────────────────────────────────

def _clean_text(value: object) -> "str | None":
    if not isinstance(value, str):
        return None
    value = value.strip()
    if not value or value in ("(none)", "Default"):
        return None
    return value


def _project_label(path: "str | None") -> str:
    if not path:
        return "No CWD"
    if path == HOME:
        return "~"
    parts = Path(path).parts
    if "GitHub" in parts:
        idx = parts.index("GitHub")
        if idx + 1 < len(parts):
            return parts[idx + 1]
    return Path(path).name or path


def _context_label(ev: dict) -> str:
    """Prefer the Plexi context name; fall back to the working directory."""
    name = _clean_text(ev.get("context_name"))
    if name:
        return name
    root = _clean_text(ev.get("context_root"))
    if root:
        return _project_label(root)
    cwd = _clean_text(ev.get("cwd"))
    if cwd:
        return _project_label(cwd)
    return "Untitled"


def _fmt_secs(secs: float) -> str:
    h = int(secs) // 3600
    m = (int(secs) % 3600) // 60
    if h > 0:
        return f"{h}h {m:02d}m"
    return f"{m}m"


def _event_duration(ev: dict) -> float:
    try:
        return max(0.0, float(ev.get("duration_secs", 0) or 0))
    except (TypeError, ValueError):
        return 0.0


def _day_start(local_dt: datetime) -> datetime:
    start = local_dt.replace(hour=DAY_START_HOUR, minute=0, second=0, microsecond=0)
    if local_dt.hour < DAY_START_HOUR:
        start -= timedelta(days=1)
    return start


def _logical_date(local_dt: datetime):
    """Calendar date of the 4 AM-anchored day this timestamp belongs to."""
    return (local_dt - timedelta(hours=DAY_START_HOUR)).date()


def _resolve_events_path() -> "Path | None":
    # Explicit override (testing / cross-channel demos): point at any events.jsonl.
    override = os.environ.get("PLEXI_STATS_EVENTS", "")
    if override:
        p = Path(override).expanduser()
        if p.exists():
            return p
    sock = os.environ.get("PLEXI_SOCKET", "")
    if sock:
        p = Path(sock).parent / "events.jsonl"
        if p.exists():
            return p
    candidates = sorted(Path.home().glob(".plexi*/events.jsonl"),
                        key=lambda p: p.stat().st_mtime, reverse=True)
    return candidates[0] if candidates else None


def _parse_focus_events(path: Path) -> "list[dict]":
    events = []
    try:
        with open(path, encoding="utf-8") as f:
            for line in f:
                line = line.strip()
                if not line:
                    continue
                try:
                    ev = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if ev.get("kind") != "focus_changed":
                    continue
                try:
                    ts = datetime.fromisoformat(ev.get("timestamp", ""))
                except ValueError:
                    continue
                if ts.tzinfo is None:
                    ts = ts.replace(tzinfo=timezone.utc)
                ev["_ts"] = ts
                events.append(ev)
    except OSError:
        pass
    return events


def _counted_duration(ev: dict, idle_stream: "list[bool]") -> float:
    """Idle-aware dwell: long spans after the first are dropped as away-time."""
    raw = _event_duration(ev)
    reason = ev.get("reason") or ""
    if reason == "pane_switch" and raw < IDLE_THRESHOLD_SECS:
        idle_stream[0] = False
        return raw
    if raw >= IDLE_THRESHOLD_SECS:
        if idle_stream[0]:
            return 0.0
        idle_stream[0] = True
        return float(IDLE_CLAMP_SECS)
    if idle_stream[0] and reason != "pane_switch":
        return 0.0
    return raw


# ── Metrics ──────────────────────────────────────────────────────────────────

class Metrics:
    def __init__(self) -> None:
        self.has_data = False
        self.active_secs = 0.0       # focus time since today's 4 AM
        self.lifetime_secs = 0.0
        self.streak_days = 0
        self.switches_today = 0
        self.contexts_today = 0
        self.active_days = 0
        self.avg_daily_secs = 0.0
        self.peak_hour_label = "—"
        self.contexts: "list[dict]" = []
        self.other_secs = 0.0
        self.other_count = 0
        self.wave: "list[tuple[int, str]]" = [(0, TEXT_SOFT)] * WAVE_BUCKETS
        self.wave_peak = 1
        self.now_frac = 0.0          # fraction of the clock day elapsed (now marker)
        self.is_today = True
        self.selected_date = None    # date of the day window being shown
        self.day_offset = 0

    @classmethod
    def load(cls, events_path: "Path | None", day_offset: int = 0) -> "Metrics":
        m = cls()
        m.day_offset = max(0, day_offset)
        if not events_path:
            return m
        events = _parse_focus_events(events_path)
        if not events:
            return m
        events.sort(key=lambda e: e["_ts"])
        m.has_data = True

        now = datetime.now(timezone.utc)
        now_local = now.astimezone()
        # Window = the 4 AM-anchored day, shifted back by day_offset.
        day_start = _day_start(now_local) - timedelta(days=m.day_offset)
        day_end = day_start + timedelta(days=1)
        m.is_today = m.day_offset == 0
        m.selected_date = _logical_date(day_start + timedelta(hours=DAY_START_HOUR))
        midnight = now_local.replace(hour=0, minute=0, second=0, microsecond=0)
        m.now_frac = min(1.0, max(0.0, (now_local - midnight).total_seconds() / 86400.0))
        bucket_secs = 86400.0 / WAVE_BUCKETS

        idle = [False]
        by_ctx: "dict[str, float]" = {}
        active_dates: "set" = set()
        hourly = [0.0] * 24
        wave_count = [0] * WAVE_BUCKETS
        wave_ctx: "list[dict[str, float]]" = [dict() for _ in range(WAVE_BUCKETS)]

        for ev in events:
            secs = _counted_duration(ev, idle)
            m.lifetime_secs += secs
            local = ev["_ts"].astimezone()
            if secs > 0:
                active_dates.add(_logical_date(local))
            if local < day_start or local >= day_end:
                continue
            # Within the selected 4 AM-anchored window.
            m.switches_today += 1
            label = _context_label(ev)
            offset = (local - day_start).total_seconds()
            if 0 <= offset < 86400.0:
                b = min(WAVE_BUCKETS - 1, int(offset / bucket_secs))
                wave_count[b] += 1
                wave_ctx[b][label] = wave_ctx[b].get(label, 0.0) + max(secs, 1.0)
            if secs > 0:
                m.active_secs += secs
                hourly[local.hour] += secs
                by_ctx[label] = by_ctx.get(label, 0.0) + secs

        m.active_days = len(active_dates)
        m.avg_daily_secs = m.lifetime_secs / max(1, m.active_days)
        m.contexts_today = len(by_ctx)

        # Rank contexts by today's focus and assign each a stable colour.
        ranked = sorted(by_ctx.items(), key=lambda kv: kv[1], reverse=True)
        ctx_color = {name: CONTEXT_PALETTE[i % len(CONTEXT_PALETTE)]
                     for i, (name, _) in enumerate(ranked)}
        m.contexts = [{"label": name, "secs": secs, "color": ctx_color[name]}
                      for name, secs in ranked[:TOP_CONTEXTS]]
        rest = ranked[TOP_CONTEXTS:]
        m.other_count = len(rest)
        m.other_secs = sum(secs for _, secs in rest)

        # Waveform: amplitude = switch count, colour = dominant context that bucket.
        m.wave_peak = max(wave_count) or 1
        wave = []
        for b in range(WAVE_BUCKETS):
            if wave_ctx[b]:
                dom = max(wave_ctx[b].items(), key=lambda kv: kv[1])[0]
                color = ctx_color.get(dom, TEXT_SOFT)
            else:
                color = TEXT_SOFT
            wave.append((wave_count[b], color))
        m.wave = wave

        # Streak: consecutive 4 AM-days ending today (or yesterday).
        today = _logical_date(now_local)
        d = today if today in active_dates else today - timedelta(days=1)
        while d in active_dates:
            m.streak_days += 1
            d -= timedelta(days=1)

        if any(hourly):
            peak = max(range(24), key=lambda h: hourly[h])
            ampm = "AM" if peak < 12 else "PM"
            h12 = peak % 12 or 12
            nxt = (peak + 1) % 12 or 12
            m.peak_hour_label = f"{h12}–{nxt} {ampm}"
        return m


# ── Rendering ────────────────────────────────────────────────────────────────
#
# Contrast rule: text uses ctx.theme.fg (high contrast) or TEXT_SOFT (readable
# subtext). theme.muted (#6c7086) is ~2.6:1 on bg and fails WCAG — never use it
# for text. dim()/hues are for fills, tracks, and the waveform only.

def _momentum(ctx, m: "Metrics") -> "tuple[str, str]":
    """Today vs a typical active day. Returns (text, color)."""
    if m.active_days <= 1 or m.avg_daily_secs <= 0:
        return "building your baseline", TEXT_SOFT
    pct = (m.active_secs / m.avg_daily_secs - 1.0) * 100.0
    if pct >= 10:
        return f"+{pct:.0f}% vs a typical day", ctx.theme.success
    if pct <= -10:
        return f"{pct:.0f}% vs a typical day", ctx.theme.warning
    return "on pace with a typical day", TEXT_SOFT


def _draw_dial(ctx, m: "Metrics", x, y, w, h) -> None:
    """Hero: a circular activity clock for the selected day.

    Angle = clock time (12 AM at top, clockwise). Radial amplitude = pane-switch
    velocity in that slice. Colour = the context you were in. A bright needle
    marks the current time (today only). Hub = the day's focus total; the inner
    ring fills toward a typical day. Grows with its container up to DIAL_MAX_R.
    """
    accent = ctx.theme.accent
    cap_h = 30.0                 # caption strip (momentum) under the circle
    ch = h - cap_h
    cx = x + w / 2
    cy = y + ch / 2
    outer = max(48.0, min(w / 2, ch / 2, DIAL_MAX_R) - 22.0)  # room for tick labels
    amp = outer * 0.42
    r0 = outer - amp
    base = math.pi / 2  # -base = top → 12 AM at top
    n = len(m.wave)
    # Bucket i covers the day from DAY_START_HOUR; map it to its clock angle so
    # midnight sits at the top regardless of the 4 AM data window.
    bucket_secs = 86400.0 / n

    def clock_ang(i: float) -> float:
        clock_secs = (DAY_START_HOUR * 3600 + i * bucket_secs) % 86400.0
        return -base + clock_secs / 86400.0 * math.tau

    # Smooth the switch counts into a flowing amplitude (3-bucket moving average),
    # compressed so frequent-but-small bursts stay visible next to big spikes.
    counts = [c for c, _ in m.wave]
    smooth = [(counts[(i - 1) % n] + counts[i] + counts[(i + 1) % n]) / 3.0
              for i in range(n)]
    peak = max(smooth) or 1.0

    # Visible baseline band the waveform rises from (keeps the ring continuous).
    sw = max(2.5, math.tau * r0 / n * 1.6)
    ctx.arc_ring(cx, cy, r0, 0, math.tau, dim(accent, 70), stroke_width=max(3.0, sw))

    # Waveform spokes — wide enough to touch, coloured by the dominant context.
    for i in range(n):
        a = smooth[i]
        if a <= 0:
            continue
        color = m.wave[i][1]
        ang = clock_ang(i + 0.5)
        rr = r0 + amp * (min(1.0, a / peak) ** 0.6)
        ctx.line(cx + math.cos(ang) * r0, cy + math.sin(ang) * r0,
                 cx + math.cos(ang) * rr, cy + math.sin(ang) * rr,
                 color=color, width=sw)

    # Inner ring: progress toward a typical day's focus.
    pr = r0 - 7.0
    frac = min(1.0, m.active_secs / m.avg_daily_secs) if m.avg_daily_secs > 0 else 0.0
    ctx.arc_ring(cx, cy, pr, 0, math.tau, dim(accent, 40), stroke_width=3.0)
    if frac > 0:
        ctx.arc_ring(cx, cy, pr, -base, -base + math.tau * frac, accent, stroke_width=3.0)

    # "Now" needle — a bright clock hand at the current time (today only).
    if m.is_today:
        nang = -base + m.now_frac * math.tau
        ctx.line(cx + math.cos(nang) * (pr - 2.0), cy + math.sin(nang) * (pr - 2.0),
                 cx + math.cos(nang) * (outer + 4.0), cy + math.sin(nang) * (outer + 4.0),
                 color=ctx.theme.fg, width=2.5)
        ctx.circle(cx + math.cos(nang) * (outer + 4.0), cy + math.sin(nang) * (outer + 4.0),
                   3.5, ctx.theme.fg)

    # Quarter-day ticks (12 AM at top, clockwise).
    lr = outer + 12.0
    for tfrac, lbl in ((0.0, "12a"), (0.25, "6a"), (0.5, "12p"), (0.75, "6p")):
        ang = -base + tfrac * math.tau
        ctx.text(cx + lr * math.cos(ang), cy + lr * math.sin(ang), lbl,
                 size=TEXT_HINT, color=TEXT_SOFT, align="center_center")

    # Hub: the day's focus total — the headline the clock measures.
    num_size = max(22.0, min(r0 * 0.52, 40.0))
    ctx.text(cx, cy - num_size * 0.26, _fmt_secs(m.active_secs), size=num_size,
             color=ctx.theme.fg, bold=True, align="center_center")
    ctx.text(cx, cy + num_size * 0.62, "focused today" if m.is_today else "focused",
             size=TEXT_HINT, color=TEXT_SOFT, align="center_center")

    # Caption: this day vs a typical day.
    ry = y + ch + 12.0
    mom_text, mom_color = _momentum(ctx, m)
    ctx.text(cx, ry, mom_text, size=TEXT_CAPTION, color=mom_color,
             bold=True, align="center_center")


def _draw_tile(ctx, x, y, w, h, value, label, color) -> None:
    ctx.rect(x, y, w, h, ctx.theme.surface, radius=RADIUS_MD)
    cxx = x + w / 2
    ctx.text(cxx, y + h * 0.34, value, size=TEXT_HEADING, color=ctx.theme.fg,
             bold=True, align="center_center")
    uw = w * 0.34
    ctx.rect(cxx - uw / 2, y + h * 0.56, uw, 2.0, color, radius=1.0)
    ctx.text(cxx, y + h * 0.74, label.upper(), size=TEXT_HINT, color=TEXT_SOFT,
             bold=True, align="center_center")


def _draw_tiles(ctx, m: "Metrics", x, y, w, h) -> None:
    gap = SPACE_SM
    tw = (w - gap * 3) / 4
    tiles = [
        (f"{m.streak_days}d", "Streak", ctx.theme.warning),
        (str(m.contexts_today), "Contexts", ctx.theme.success),
        (str(m.switches_today), "Switches", ctx.theme.accent),
        (m.peak_hour_label, "Peak Hour", ctx.theme.red),
    ]
    for i, (val, lbl, color) in enumerate(tiles):
        _draw_tile(ctx, x + i * (tw + gap), y, tw, h, val, lbl, color)


def _draw_contexts(ctx, m: "Metrics", x, y, w) -> None:
    ctx.text(x, y, "TOP CONTEXTS", size=TEXT_HINT, color=TEXT_SOFT, bold=True)
    name_w = w * 0.46          # generous room for context names
    time_w = 56.0
    bar_x = x + name_w
    bar_w = w - name_w - time_w
    bar_h = 13.0

    # Always render TOP_CONTEXTS named rows + one "Other" row so the section
    # height is constant — the layout never jumps as the day's mix changes.
    rows = list(m.contexts)
    if m.other_secs > 0 or m.other_count > 0:
        rows.append({"label": f"Other ({m.other_count})", "secs": m.other_secs,
                     "color": OTHER_COLOR, "other": True})
    top = max((r["secs"] for r in rows), default=0.0)

    for i in range(CONTEXT_ROWS):
        ry = y + SPACE_LG + i * ROW_H
        if i >= len(rows):
            continue
        c = rows[i]
        is_other = c.get("other", False)
        cyy = ry + bar_h / 2
        rank_color = RANK_COLORS[i] if i < 3 and not is_other else TEXT_SOFT
        marker = "·" if is_other else f"{i + 1}"
        ctx.text(x, cyy, marker, size=TEXT_CAPTION, color=rank_color,
                 bold=True, align="left_center")
        ctx.text(x + SPACE_LG, cyy, c["label"], size=TEXT_CAPTION,
                 color=TEXT_SOFT if is_other else ctx.theme.fg, bold=True,
                 align="left_center", max_width=name_w - SPACE_LG - SPACE_SM)
        ctx.rect(bar_x, ry, bar_w, bar_h, ctx.theme.surface, radius=bar_h / 2)
        frac = c["secs"] / top if top else 0.0
        if frac > 0:
            fw = max(bar_h, bar_w * frac)
            glow = (not is_other) and i < 3
            ctx.rect(bar_x, ry, fw, bar_h, "#00000000",
                     gradient={"from": dim(c["color"], 160), "to": c["color"], "dir": "h"},
                     glow_color=c["color"] if glow else None,
                     glow_radius=5.0 if glow else 0.0)
        ctx.text(x + w, cyy, _fmt_secs(c["secs"]),
                 size=TEXT_CAPTION, color=TEXT_SOFT, align="right_center")


class StatsApp(App):

    async def on_init(self) -> None:
        self.events_path = _resolve_events_path()
        self.day_offset = 0
        self.m = Metrics.load(self.events_path, self.day_offset)
        self.emit.info(
            f"stats: loaded data={self.m.has_data} "
            f"active_today={int(self.m.active_secs)}s switches={self.m.switches_today} "
            f"contexts={self.m.contexts_today} lifetime={int(self.m.lifetime_secs)}s")
        if self.m.has_data:
            self.emit.status_summary(f"{_fmt_secs(self.m.active_secs)} active")

    def _draw(self, ctx, x, y, w, h) -> None:
        ctx.rect(x, y, w, h, ctx.theme.bg)
        m = self.m
        if not m.has_data:
            ctx.text(x + w / 2, y + h / 2, "No focus events yet",
                     size=TEXT_HEADING, color=TEXT_SOFT, align="center_center")
            return
        pad = SPACE_LG
        inner_w = w - 2 * pad
        contexts_h = SPACE_LG + CONTEXT_ROWS * ROW_H  # fixed → layout never jumps
        # Dial is the hero; it absorbs leftover vertical space (and is capped
        # inside _draw_dial so it never overruns its slot).
        bottom = y + h - SPACE_LG
        cur = y + SPACE_SM
        dial_h = max(DIAL_MIN_H, bottom - cur - TILES_H - contexts_h - 2 * SPACE_LG)
        _draw_dial(ctx, m, x + pad, cur, inner_w, dial_h)
        cur += dial_h + SPACE_LG
        _draw_tiles(ctx, m, x + pad, cur, inner_w, TILES_H)
        cur += TILES_H + SPACE_LG
        _draw_contexts(ctx, m, x + pad, cur, inner_w)

    def _subtitle(self) -> str:
        m = self.m
        if not m.has_data or m.selected_date is None:
            return "no data"
        d = m.selected_date
        label = f"{d:%A, %B} {d.day}"
        if m.day_offset == 1:
            return f"{label} · yesterday"
        if m.day_offset > 1:
            return f"{label} · {m.day_offset} days ago"
        return label

    def on_render(self, ctx) -> None:
        ctx.render(Column(
            [
                AppBar(title="Stats", subtitle=self._subtitle()),
                Canvas(draw=self._draw, grow=True),
                FooterKeys([("←/→", "day"), ("t", "today"),
                            ("r", "refresh"), ("esc", "close")]),
            ],
            padding=0, gap=0,
        ))

    def _reload(self) -> None:
        self.m = Metrics.load(self.events_path, self.day_offset)
        self.emit.info(f"stats: day_offset={self.day_offset}")
        self.emit.schedule_render(0)

    def on_key(self, key: str, _mods: dict) -> None:
        if key == "left":
            self.day_offset += 1            # older
            self._reload()
        elif key == "right":
            if self.day_offset > 0:
                self.day_offset -= 1        # newer
                self._reload()
        elif key == "t":
            if self.day_offset != 0:
                self.day_offset = 0
                self._reload()
        elif key == "r":
            self._reload()


if __name__ == "__main__":
    StatsApp().run()
