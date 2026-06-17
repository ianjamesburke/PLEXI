#!/usr/bin/env python3
"""Stats — an activity dashboard for your work.

Reads Plexi focus events and renders a focus-level progress badge, stat
tiles, ranked projects, and a 24h activity heatmap. Built on the native
canvas styling primitives (rect glow/gradient/stroke, arc_ring) — see
docs/sdk-v2.md.
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
    SPACE_SM, SPACE_MD, SPACE_LG, SPACE_XL,
    TEXT_HINT, TEXT_CAPTION, TEXT_HEADING,
)

HOME = str(Path.home())
IDLE_THRESHOLD_SECS = 15 * 60
IDLE_CLAMP_SECS = 60
RANK_COLORS = ["#f9e2af", "#bac2de", "#fab387"]  # 1st / 2nd / 3rd
PROJECT_PALETTE = ["#89b4fa", "#a6e3a1", "#f9e2af", "#cba6f7", "#fab387", "#94e2d5"]
TEXT_SOFT = "#a6adc8"  # readable secondary text (~3.7:1 on bg); never use theme.muted for text

# Earned rank titles by level threshold (light RPG flavor).
RANK_TITLES = [
    (1, "Novice"), (5, "Apprentice"), (10, "Adept"),
    (20, "Expert"), (35, "Master"), (55, "Grandmaster"),
]

# Section heights (logical px).
TILES_H = 76.0
ROW_H = 27.0
DIAL_MIN_H = 190.0


def _rank_title(level: int) -> str:
    title = RANK_TITLES[0][1]
    for threshold, name in RANK_TITLES:
        if level >= threshold:
            title = name
    return title


# ── Data layer ───────────────────────────────────────────────────────────────

def _clean_text(value: object) -> "str | None":
    if not isinstance(value, str):
        return None
    value = value.strip()
    if not value or value == "(none)":
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


def _project_identity(ev: dict) -> "tuple[str, str]":
    root = _clean_text(ev.get("context_root"))
    if root:
        return root, _project_label(root)
    name = _clean_text(ev.get("context_name"))
    if name:
        return f"context:{name}", name
    cwd = _clean_text(ev.get("cwd"))
    if cwd:
        return cwd, _project_label(cwd)
    return "context:unknown", "Unknown"


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
    """Idle-aware duration: long gaps after the first are dropped."""
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


def _level_for(xp_secs: float) -> "tuple[int, float, float]":
    """Triangular XP curve: level n costs n hours. Returns (level, into, need)."""
    hours = xp_secs / 3600.0
    level = 0
    need_total = 0.0
    while need_total + (level + 1) <= hours:
        level += 1
        need_total += level
    into = (hours - need_total) * 3600.0
    need = (level + 1) * 3600.0
    return max(1, level), into, need


# ── Metrics ──────────────────────────────────────────────────────────────────

class Metrics:
    def __init__(self) -> None:
        self.has_data = False
        self.active_secs = 0.0
        self.lifetime_secs = 0.0
        self.level = 1
        self.xp_into = 0.0
        self.xp_need = 3600.0
        self.streak_days = 0
        self.session_count = 0
        self.peak_hour_label = "—"
        self.projects: "list[dict]" = []
        self.hourly = [0.0] * 24
        self.active_days = 0
        self.avg_daily_secs = 0.0

    @classmethod
    def load(cls, events_path: "Path | None") -> "Metrics":
        m = cls()
        if not events_path:
            return m
        events = _parse_focus_events(events_path)
        if not events:
            return m
        events.sort(key=lambda e: e["_ts"])
        m.has_data = True

        now = datetime.now(timezone.utc)
        day_cutoff = now - timedelta(hours=24)

        idle = [False]
        by_project: "dict[str, dict]" = {}
        active_dates: "set[str]" = set()
        for ev in events:
            secs = _counted_duration(ev, idle)
            m.lifetime_secs += secs
            ts = ev["_ts"]
            local = ts.astimezone()
            if secs > 0:
                active_dates.add(local.strftime("%Y-%m-%d"))
            if ts < day_cutoff:
                continue
            # today (rolling 24h)
            m.active_secs += secs
            if secs > 0:
                m.session_count += 1
                m.hourly[local.hour] += secs
                _, label = _project_identity(ev)
                p = by_project.setdefault(label, {"label": label, "secs": 0.0})
                p["secs"] += secs

        m.level, m.xp_into, m.xp_need = _level_for(m.lifetime_secs)
        m.active_days = len(active_dates)
        m.avg_daily_secs = m.lifetime_secs / max(1, m.active_days)

        # streak: consecutive local dates ending today (or yesterday).
        today = now.astimezone().date()
        d = today if today.strftime("%Y-%m-%d") in active_dates else today - timedelta(days=1)
        while d.strftime("%Y-%m-%d") in active_dates:
            m.streak_days += 1
            d = d - timedelta(days=1)

        ranked = sorted(by_project.values(), key=lambda p: p["secs"], reverse=True)[:6]
        for i, p in enumerate(ranked):
            p["color"] = PROJECT_PALETTE[i % len(PROJECT_PALETTE)]
        m.projects = ranked

        if any(m.hourly):
            peak = max(range(24), key=lambda h: m.hourly[h])
            ampm = "AM" if peak < 12 else "PM"
            h12 = peak % 12 or 12
            nxt = (peak + 1) % 12 or 12
            m.peak_hour_label = f"{h12}–{nxt} {ampm}"
        return m


# ── Rendering ────────────────────────────────────────────────────────────────
#
# Contrast rule: text uses ctx.theme.fg (high contrast) or TEXT_SOFT (readable
# subtext). theme.muted (#6c7086) is ~2.6:1 on bg and fails WCAG — never use it
# for text. dim()/hues are for fills, tracks, and dial heat only.

def _lerp_hex(a: str, b: str, t: float) -> str:
    a, b = a.lstrip("#"), b.lstrip("#")
    ar, ag, ab = int(a[0:2], 16), int(a[2:4], 16), int(a[4:6], 16)
    br, bg, bb = int(b[0:2], 16), int(b[2:4], 16), int(b[4:6], 16)
    r = round(ar + (br - ar) * t)
    g = round(ag + (bg - ag) * t)
    bl = round(ab + (bb - ab) * t)
    return f"#{r:02x}{g:02x}{bl:02x}"


def _heat(frac: float, stops: "list[str]") -> str:
    """Map 0..1 to a multi-stop heat ramp (cool → warm), full opacity."""
    frac = max(0.0, min(1.0, frac))
    span = len(stops) - 1
    pos = frac * span
    i = min(int(pos), span - 1)
    return _lerp_hex(stops[i], stops[i + 1], pos - i)


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
    """Hero: a big 24h activity clock whose hub IS your character.

    Outer ring = 24h focus heat (midnight at top). Inner ring = level progress.
    Hub = level number, rank, today's momentum. Scales with its container.
    """
    accent = ctx.theme.accent
    cx = x + w / 2
    cy = y + h / 2
    tick_margin = 22.0
    r = max(46.0, min(w / 2, h / 2) - tick_margin)
    sw = max(11.0, r * 0.17)
    seg = math.tau / 24.0
    gap = seg * 0.16
    base = math.pi / 2  # -base = top → midnight at top
    peak = max(m.hourly) or 1.0
    heat_stops = [ctx.theme.highlight, accent, ctx.theme.warning]

    # 24h heat ring.
    for hour in range(24):
        a0 = -base + hour * seg + gap / 2
        a1 = -base + (hour + 1) * seg - gap / 2
        secs = m.hourly[hour]
        if secs <= 0:
            ctx.arc_ring(cx, cy, r, a0, a1, ctx.theme.surface, stroke_width=sw)
        else:
            ctx.arc_ring(cx, cy, r, a0, a1, _heat(secs / peak, heat_stops),
                         stroke_width=sw)

    # Inner level-progress ring.
    pr = r - sw - 6.0
    frac = m.xp_into / max(1.0, m.xp_need)
    ctx.arc_ring(cx, cy, pr, 0, math.tau, dim(accent, 40), stroke_width=3.0)
    if frac > 0:
        ctx.arc_ring(cx, cy, pr, -base, -base + math.tau * frac, accent, stroke_width=3.0)

    # Hour ticks at the quarters.
    lr = r + sw / 2 + 11.0
    for hh, lbl in ((0, "00"), (6, "06"), (12, "12"), (18, "18")):
        ang = -base + hh * seg
        ctx.text(cx + lr * math.cos(ang), cy + lr * math.sin(ang), lbl,
                 size=TEXT_HINT, color=TEXT_SOFT, align="center_center")

    # "now" marker.
    now = datetime.now().astimezone()
    nowf = now.hour + now.minute / 60.0
    ang = -base + nowf * seg
    ctx.circle(cx + r * math.cos(ang), cy + r * math.sin(ang), max(4.0, sw * 0.3),
               ctx.theme.fg)

    # Hub: the character.
    num_size = max(30.0, min(r * 0.85, 64.0))
    ctx.text(cx, cy - num_size * 0.18, str(m.level), size=num_size,
             color=ctx.theme.fg, bold=True, align="center_center")
    ctx.text(cx, cy + num_size * 0.42, _rank_title(m.level).upper(),
             size=TEXT_CAPTION, color=accent, bold=True, align="center_center")
    mom_text, mom_color = _momentum(ctx, m)
    ctx.text(cx, cy + num_size * 0.42 + 16.0, mom_text, size=TEXT_HINT,
             color=mom_color, align="center_center")


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
        (_fmt_secs(m.active_secs), "Active Today", ctx.theme.accent),
        (f"{m.streak_days}d", "Day Streak", ctx.theme.warning),
        (str(m.session_count), "Sessions", ctx.theme.success),
        (m.peak_hour_label, "Peak Hour", ctx.theme.red),
    ]
    for i, (val, lbl, color) in enumerate(tiles):
        _draw_tile(ctx, x + i * (tw + gap), y, tw, h, val, lbl, color)


def _draw_projects(ctx, m: "Metrics", x, y, w) -> None:
    ctx.text(x, y, "TOP PROJECTS", size=TEXT_HINT, color=TEXT_SOFT, bold=True)
    if not m.projects:
        return
    bar_x = x + 140.0
    bar_w = w - 140.0 - 84.0
    bar_h = 13.0
    top = max(p["secs"] for p in m.projects)
    for i, p in enumerate(m.projects):
        ry = y + SPACE_LG + i * ROW_H
        cy = ry + bar_h / 2
        rank_color = RANK_COLORS[i] if i < 3 else TEXT_SOFT
        ctx.text(x, cy, f"{i + 1}", size=TEXT_CAPTION, color=rank_color,
                 bold=True, align="left_center")
        ctx.text(x + SPACE_XL, cy, p["label"], size=TEXT_CAPTION,
                 color=ctx.theme.fg, bold=True, align="left_center", max_width=104.0)
        # Neutral track + coloured fill (track is NOT the project hue).
        ctx.rect(bar_x, ry, bar_w, bar_h, ctx.theme.surface, radius=bar_h / 2)
        frac = p["secs"] / top if top else 0.0
        if frac > 0:
            fw = max(bar_h, bar_w * frac)
            ctx.rect(bar_x, ry, fw, bar_h, "#00000000",
                     gradient={"from": dim(p["color"], 160), "to": p["color"], "dir": "h"},
                     glow_color=p["color"] if i < 3 else None,
                     glow_radius=5.0 if i < 3 else 0.0)
        ctx.text(x + w, cy, _fmt_secs(p["secs"]),
                 size=TEXT_CAPTION, color=TEXT_SOFT, align="right_center")


class StatsApp(App):

    async def on_init(self) -> None:
        self.events_path = _resolve_events_path()
        self.m = Metrics.load(self.events_path)
        self.emit.info(
            f"stats: loaded data={self.m.has_data} level={self.m.level} "
            f"active={int(self.m.active_secs)}s sessions={self.m.session_count} "
            f"projects={len(self.m.projects)}")
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
        projects_h = SPACE_LG + max(1, len(m.projects)) * ROW_H
        # Dial is the hero — it absorbs the leftover vertical space, so it scales
        # up on tall panes instead of capping at a tiny fixed size.
        bottom = y + h - SPACE_LG
        cur = y + SPACE_MD
        dial_h = max(DIAL_MIN_H,
                     bottom - cur - TILES_H - projects_h - 2 * SPACE_LG)
        _draw_dial(ctx, m, x + pad, cur, inner_w, dial_h)
        cur += dial_h + SPACE_LG
        _draw_tiles(ctx, m, x + pad, cur, inner_w, TILES_H)
        cur += TILES_H + SPACE_LG
        _draw_projects(ctx, m, x + pad, cur, inner_w)

    def on_render(self, ctx) -> None:
        m = self.m
        subtitle = (f"{_fmt_secs(m.lifetime_secs)} focused all-time · {m.active_days} active days"
                    if m.has_data else "no data")
        ctx.render(Column(
            [
                AppBar(title="Stats", subtitle=subtitle),
                Canvas(draw=self._draw, grow=True),
                FooterKeys([("r", "refresh"), ("esc", "close")]),
            ],
            padding=0, gap=0,
        ))

    def on_key(self, key: str, _mods: dict) -> None:
        if key == "r":
            self.m = Metrics.load(self.events_path)
            self.emit.info("stats: refreshed")
            self.emit.schedule_render(0)


if __name__ == "__main__":
    StatsApp().run()
