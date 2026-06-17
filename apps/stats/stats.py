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
    TEXT_HINT, TEXT_CAPTION, TEXT_HEADING, TEXT_TITLE,
)

HOME = str(Path.home())
IDLE_THRESHOLD_SECS = 15 * 60
IDLE_CLAMP_SECS = 60
RANK_COLORS = ["#f9e2af", "#bac2de", "#fab387"]  # 1st / 2nd / 3rd
PROJECT_PALETTE = ["#89b4fa", "#a6e3a1", "#f9e2af", "#cba6f7", "#fab387", "#94e2d5"]

# Earned rank titles by level threshold (light RPG flavor).
RANK_TITLES = [
    (1, "Novice"), (5, "Apprentice"), (10, "Adept"),
    (20, "Expert"), (35, "Master"), (55, "Grandmaster"),
]

# Section heights (logical px).
HERO_H = 96.0
TILES_H = 86.0
ROW_H = 26.0
CLOCK_MIN_H = 130.0


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

def _momentum(ctx, m: "Metrics") -> "tuple[str, str]":
    """Today vs a typical active day. Returns (text, color)."""
    if m.active_days <= 1 or m.avg_daily_secs <= 0:
        return "building your baseline", ctx.theme.muted
    pct = (m.active_secs / m.avg_daily_secs - 1.0) * 100.0
    if pct >= 10:
        return f"+{pct:.0f}% vs a typical day", ctx.theme.success
    if pct <= -10:
        return f"{pct:.0f}% vs a typical day", ctx.theme.warning
    return "on pace with a typical day", ctx.theme.muted


def _draw_hero(ctx, m: "Metrics", x, y, h) -> None:
    """Character card: rank medallion + earned title + today's momentum."""
    frac = m.xp_into / max(1.0, m.xp_need)
    accent = ctx.theme.accent
    r = min(h * 0.42, 38.0)
    cx = x + SPACE_LG + r
    cy = y + h / 2

    # Rank medallion: crisp disc + progress ring (no glow — keep the number legible).
    ctx.circle(cx, cy, r - 5, dim(accent, 26))
    ctx.arc_ring(cx, cy, r, 0, math.tau, dim(accent, 45), stroke_width=6.0)
    if frac > 0:
        ctx.arc_ring(cx, cy, r, -math.pi / 2, -math.pi / 2 + math.tau * frac,
                     accent, stroke_width=6.0)
    ctx.text(cx, cy, str(m.level), size=TEXT_TITLE, color=ctx.theme.fg,
             bold=True, align="center_center")

    tx = cx + r + SPACE_XL
    ctx.text(tx, y + SPACE_SM, _rank_title(m.level),
             size=TEXT_HEADING, color=ctx.theme.fg, bold=True)
    mom_text, mom_color = _momentum(ctx, m)
    ctx.text(tx, y + h * 0.45, mom_text, size=TEXT_CAPTION, color=mom_color, bold=True)
    mins_left = max(0, int((m.xp_need - m.xp_into) // 60))
    ctx.text(tx, y + h * 0.72,
             f"{_fmt_secs(m.lifetime_secs)} all-time  ·  Level {m.level + 1} in {mins_left}m",
             size=TEXT_HINT, color=ctx.theme.muted)


def _draw_tile(ctx, x, y, w, h, value, label, sub, color) -> None:
    ctx.rect(x, y, w, h, ctx.theme.surface, radius=RADIUS_MD,
             stroke=dim(color, 90), stroke_width=1.5)
    ctx.rect(x + RADIUS_MD, y, w - 2 * RADIUS_MD, 3.0, color)
    cxx = x + w / 2
    ctx.text(cxx, y + h * 0.30, value, size=TEXT_TITLE, color=color,
             bold=True, align="center_center")
    ctx.text(cxx, y + h * 0.62, label.upper(), size=TEXT_HINT,
             color=ctx.theme.muted, bold=True, align="center_center")
    if sub:
        ctx.text(cxx, y + h * 0.80, sub, size=TEXT_HINT,
                 color=dim(ctx.theme.muted, 150), align="center_center")


def _draw_tiles(ctx, m: "Metrics", x, y, w, h) -> None:
    gap = SPACE_SM
    tw = (w - gap * 3) / 4
    tiles = [
        (_fmt_secs(m.active_secs), "Active", "today", ctx.theme.accent),
        (f"{m.streak_days}d", "Streak", "current" if m.streak_days else "—", ctx.theme.warning),
        (str(m.session_count), "Sessions", "today", ctx.theme.success),
        (m.peak_hour_label, "Peak Hour", "most focus", ctx.theme.red),
    ]
    for i, (val, lbl, sub, color) in enumerate(tiles):
        _draw_tile(ctx, x + i * (tw + gap), y, tw, h, val, lbl, sub, color)


def _draw_projects(ctx, m: "Metrics", x, y, w) -> None:
    ctx.text(x, y, "TOP PROJECTS", size=TEXT_HINT, color=ctx.theme.muted, bold=True)
    if not m.projects:
        return
    bar_x = x + 132.0
    bar_w = w - 132.0 - 96.0
    bar_h = 13.0
    top = max(p["secs"] for p in m.projects)
    for i, p in enumerate(m.projects):
        ry = y + SPACE_LG + i * ROW_H
        cy = ry + bar_h / 2
        rank_color = RANK_COLORS[i] if i < 3 else dim(ctx.theme.muted, 160)
        ctx.text(x, cy, f"{i + 1}", size=TEXT_CAPTION, color=rank_color,
                 bold=True, align="left_center")
        ctx.text(x + SPACE_XL, cy, p["label"], size=TEXT_CAPTION,
                 color=ctx.theme.fg, bold=True, align="left_center", max_width=96.0)
        ctx.rect(bar_x, ry, bar_w, bar_h, dim(p["color"], 30), radius=bar_h / 2)
        frac = p["secs"] / top if top else 0.0
        if frac > 0:
            fw = max(bar_h, bar_w * frac)
            ctx.rect(bar_x, ry, fw, bar_h, "#00000000",
                     gradient={"from": dim(p["color"], 150), "to": p["color"], "dir": "h"},
                     glow_color=p["color"] if i < 3 else None,
                     glow_radius=5.0 if i < 3 else 0.0)
        ctx.text(x + w, cy, _fmt_secs(p["secs"]),
                 size=TEXT_CAPTION, color=ctx.theme.muted, align="right_center")


def _draw_clock(ctx, m: "Metrics", x, y, w, h) -> None:
    """24-hour radial dial: midnight at top, each hour heat-coloured by focus."""
    ctx.text(x, y, "ACTIVITY · 24H CLOCK", size=TEXT_HINT,
             color=ctx.theme.muted, bold=True)
    accent = ctx.theme.accent
    top = y + SPACE_LG
    avail_h = h - SPACE_LG
    cx = x + w / 2
    cy = top + avail_h / 2
    sw = 13.0
    r = min(avail_h / 2 - 14.0, w / 2 - 14.0, 58.0)
    seg = math.tau / 24.0
    gap = seg * 0.14
    peak = max(m.hourly) or 1.0
    base = math.pi / 2  # +y is down, so -pi/2 is top → midnight at top

    for hour in range(24):
        a0 = -base + hour * seg + gap / 2
        a1 = -base + (hour + 1) * seg - gap / 2
        secs = m.hourly[hour]
        if secs <= 0:
            ctx.arc_ring(cx, cy, r, a0, a1, dim(ctx.theme.muted, 26), stroke_width=sw)
        else:
            alpha = int(70 + 185 * min(1.0, secs / peak))
            ctx.arc_ring(cx, cy, r, a0, a1, dim(accent, alpha), stroke_width=sw)

    # Hour ticks at the quarters.
    lr = r + sw / 2 + 9.0
    for hh, lbl in ((0, "00"), (6, "06"), (12, "12"), (18, "18")):
        ang = -base + hh * seg
        ctx.text(cx + lr * math.cos(ang), cy + lr * math.sin(ang), lbl,
                 size=TEXT_HINT, color=dim(ctx.theme.muted, 140), align="center_center")

    # "now" marker dot on the ring.
    now = datetime.now().astimezone()
    nowf = now.hour + now.minute / 60.0
    ang = -base + nowf * seg
    ctx.circle(cx + r * math.cos(ang), cy + r * math.sin(ang), 4.0, ctx.theme.fg)

    # Centre focal: today's total.
    ctx.text(cx, cy - 7.0, _fmt_secs(m.active_secs), size=TEXT_CAPTION,
             color=ctx.theme.fg, bold=True, align="center_center")
    ctx.text(cx, cy + 9.0, "today", size=TEXT_HINT,
             color=ctx.theme.muted, align="center_center")


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
                     size=TEXT_HEADING, color=ctx.theme.muted, align="center_center")
            return
        pad = SPACE_MD
        cur = y + SPACE_SM
        _draw_hero(ctx, m, x + pad, cur, HERO_H)
        cur += HERO_H + SPACE_SM
        _draw_tiles(ctx, m, x + pad, cur, w - 2 * pad, TILES_H)
        cur += TILES_H + SPACE_MD
        projects_h = SPACE_LG + max(1, len(m.projects)) * ROW_H + SPACE_SM
        _draw_projects(ctx, m, x + pad, cur, w - 2 * pad)
        cur += projects_h + SPACE_SM
        clock_h = max(CLOCK_MIN_H, (y + h) - cur - SPACE_SM)
        _draw_clock(ctx, m, x + pad, cur, w - 2 * pad, clock_h)

    def on_render(self, ctx) -> None:
        m = self.m
        subtitle = (f"{_fmt_secs(m.active_secs)} active today · {m.streak_days}d streak"
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
