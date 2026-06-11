#!/usr/bin/env python3
"""Stats — usage dashboard showing active time, visits, and project treemap."""
from __future__ import annotations

import json
import os
from datetime import datetime, timedelta, timezone
from pathlib import Path

from plexi_sdk import App, dim
from plexi_sdk.ui import (
    Column, AppBar, FooterKeys, Component,
    TEXT_HINT, TEXT_CAPTION, TEXT_BODY,
)

PALETTE = [
    "#f38ba8", "#a6e3a1", "#89b4fa", "#f9e2af",
    "#cba6f7", "#94e2d5", "#fab387", "#74c7ec",
]

STATS_BAR_H = 34.0
TIMELINE_H = 50.0
MIN_CELL_W = 60.0
MIN_CELL_H = 40.0
IDLE_THRESHOLD_SECS = 15 * 60
IDLE_CLAMP_SECS = 60
HOME = str(Path.home())
UNKNOWN_CONTEXT = "Unknown"


def _shorten(path: str) -> str:
    if path.startswith("context:"):
        return path.removeprefix("context:")
    if path == HOME:
        return "~"
    if path.startswith(HOME + "/"):
        return "~" + path[len(HOME):]
    return path


def _clean_text(value: object) -> str | None:
    if not isinstance(value, str):
        return None
    value = value.strip()
    if not value or value == "(none)":
        return None
    return value


def _project_root(path: str | None) -> str | None:
    if not path:
        return None
    parts = Path(path).parts
    if "GitHub" in parts:
        idx = parts.index("GitHub")
        if idx + 1 < len(parts):
            return str(Path(*parts[:idx + 2]))
    return path


def _project_label(path: str | None) -> str:
    if not path:
        return "No CWD"
    if path == HOME:
        return "~"
    parts = Path(path).parts
    if "GitHub" in parts:
        idx = parts.index("GitHub")
        if idx + 1 < len(parts):
            return parts[idx + 1]
    name = Path(path).name
    return name or _shorten(path)


def _cwd_label(path: str) -> str:
    if path == HOME:
        return "~"
    name = Path(path).name
    return name or _shorten(path)


def _event_context_identity(ev: dict) -> tuple[str, str]:
    context_root = _clean_text(ev.get("context_root"))
    if context_root:
        root = _project_root(context_root) or context_root
        return root, _project_label(root)

    context_name = _clean_text(ev.get("context_name"))
    if context_name:
        return f"context:{context_name}", context_name

    cwd = _clean_text(ev.get("cwd"))
    root = _project_root(cwd)
    if root:
        return root, _project_label(root)

    return "context:unknown", UNKNOWN_CONTEXT


def _fmt_duration(secs: float) -> str:
    h = int(secs) // 3600
    m = (int(secs) % 3600) // 60
    if h > 0:
        return f"{h}h {m:02d}m"
    return f"{m}m"


def _event_duration(ev: dict, key: str = "duration_secs") -> float:
    try:
        return max(0.0, float(ev.get(key, 0) or 0))
    except (TypeError, ValueError):
        return 0.0


def _event_ts(ev: dict) -> datetime:
    ts = ev.get("_ts")
    if isinstance(ts, datetime):
        return ts
    return datetime.min.replace(tzinfo=timezone.utc)


def _normalize_focus_events(events: list[dict]) -> tuple[list[dict], dict[str, float | int]]:
    normalized: list[dict] = []
    stats: dict[str, float | int] = {
        "raw_secs": 0.0,
        "counted_secs": 0.0,
        "clamped_secs": 0.0,
        "skipped_secs": 0.0,
        "counted_events": 0,
        "clamped_events": 0,
        "skipped_events": 0,
    }
    idle_stream = False

    for ev in sorted(events, key=_event_ts):
        raw = _event_duration(ev)
        reason = ev.get("reason") or ""
        counted = raw
        idle_state = "active"
        clamped = 0.0
        skipped = 0.0

        if reason == "pane_switch" and raw < IDLE_THRESHOLD_SECS:
            idle_stream = False
        elif raw >= IDLE_THRESHOLD_SECS:
            if idle_stream:
                counted = 0.0
                skipped = raw
                idle_state = "skipped"
            else:
                counted = min(raw, float(IDLE_CLAMP_SECS))
                clamped = max(0.0, raw - counted)
                idle_state = "clamped"
                idle_stream = True
        elif idle_stream and reason != "pane_switch":
            counted = 0.0
            skipped = raw
            idle_state = "skipped"

        normalized_ev = dict(ev)
        normalized_ev["_raw_duration_secs"] = raw
        normalized_ev["_clamped_secs"] = clamped
        normalized_ev["_skipped_secs"] = skipped
        normalized_ev["_idle_state"] = idle_state
        normalized_ev["duration_secs"] = counted
        normalized.append(normalized_ev)

        stats["raw_secs"] = float(stats["raw_secs"]) + raw
        stats["counted_secs"] = float(stats["counted_secs"]) + counted
        stats["clamped_secs"] = float(stats["clamped_secs"]) + clamped
        stats["skipped_secs"] = float(stats["skipped_secs"]) + skipped
        if counted > 0:
            stats["counted_events"] = int(stats["counted_events"]) + 1
        if clamped > 0:
            stats["clamped_events"] = int(stats["clamped_events"]) + 1
        if skipped > 0:
            stats["skipped_events"] = int(stats["skipped_events"]) + 1

    return normalized, stats


def _timeline_fractions(
    ts: datetime,
    counted_secs: float,
    raw_secs: float,
    idle_state: str,
    start_window: datetime,
) -> tuple[float, float, float, float]:
    raw_start = ts - timedelta(seconds=raw_secs)
    raw_end = ts
    if idle_state == "clamped":
        counted_start = raw_start
        counted_end = raw_start + timedelta(seconds=counted_secs)
    else:
        counted_start = ts - timedelta(seconds=counted_secs)
        counted_end = ts

    def frac(moment: datetime) -> float:
        return max(0.0, min(1.0, (moment - start_window).total_seconds() / 86400.0))

    return frac(raw_start), frac(raw_end), frac(counted_start), frac(counted_end)


def _resolve_events_path() -> Path | None:
    sock = os.environ.get("PLEXI_SOCKET", "")
    if sock:
        profile_dir = Path(sock).parent
        p = profile_dir / "events.jsonl"
        if p.exists():
            return p
    candidates = sorted(Path.home().glob(".plexi*/events.jsonl"),
                        key=lambda p: p.stat().st_mtime, reverse=True)
    return candidates[0] if candidates else None


def _parse_events(path: Path, hours: int = 24) -> list[dict]:
    cutoff = datetime.now(timezone.utc) - timedelta(hours=hours)
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
                ts_str = ev.get("timestamp", "")
                try:
                    ts = datetime.fromisoformat(ts_str)
                    if ts.tzinfo is None:
                        ts = ts.replace(tzinfo=timezone.utc)
                except ValueError:
                    continue
                if ts >= cutoff:
                    ev["_ts"] = ts
                    events.append(ev)
    except OSError:
        pass
    return events


class TreeNode:
    __slots__ = ("path", "label", "self_duration", "total_duration", "visits",
                 "children", "color_index")

    def __init__(self, path: str, label: str):
        self.path = path
        self.label = label
        self.self_duration = 0.0
        self.total_duration = 0.0
        self.visits = 0
        self.children: list[TreeNode] = []
        self.color_index = 0


def _build_tree(events: list[dict]) -> TreeNode:
    by_context: dict[str, tuple[str, float, int]] = {}
    by_context_cwd: dict[tuple[str, str], tuple[str, float, int]] = {}

    for ev in events:
        context_path, context_label = _event_context_identity(ev)
        cwd = _clean_text(ev.get("cwd"))
        dur = ev.get("duration_secs", 0)
        prev_context = by_context.get(context_path, (context_label, 0.0, 0))
        by_context[context_path] = (
            context_label,
            prev_context[1],
            prev_context[2] + 1,
        )

        if cwd:
            leaf_key = (context_path, cwd)
            prev_leaf = by_context_cwd.get(leaf_key, (_cwd_label(cwd), 0.0, 0))
            by_context_cwd[leaf_key] = (
                prev_leaf[0],
                prev_leaf[1] + dur,
                prev_leaf[2] + 1,
            )
        else:
            by_context[context_path] = (
                context_label,
                prev_context[1] + dur,
                prev_context[2] + 1,
            )

    root = TreeNode("contexts", "Contexts")
    context_nodes: dict[str, TreeNode] = {}

    for context_path, (context_label, dur, visits) in sorted(by_context.items()):
        node = TreeNode(context_path, context_label)
        node.self_duration = dur
        node.visits = visits
        root.children.append(node)
        context_nodes[context_path] = node

    for (context_path, cwd), (label, dur, visits) in sorted(by_context_cwd.items()):
        parent = context_nodes.get(context_path)
        if not parent:
            continue
        node = TreeNode(cwd, label)
        node.self_duration = dur
        node.visits = visits
        parent.children.append(node)

    def rollup(n: TreeNode) -> float:
        child_total = sum(rollup(c) for c in n.children)
        n.total_duration = n.self_duration + child_total
        n.children.sort(key=lambda c: c.total_duration, reverse=True)
        return n.total_duration

    rollup(root)

    for i, child in enumerate(root.children):
        _assign_colors(child, i)

    return root


def _assign_colors(node: TreeNode, base_index: int) -> None:
    node.color_index = base_index
    for child in node.children:
        _assign_colors(child, base_index)


def _squarify(items: list[tuple[TreeNode, float]], rect: tuple[float, float, float, float]) -> list[tuple[TreeNode, float, float, float, float]]:
    if not items:
        return []
    x, y, w, h = rect
    if w <= 0 or h <= 0:
        return []

    total = sum(v for _, v in items)
    if total <= 0:
        return []

    result: list[tuple[TreeNode, float, float, float, float]] = []

    remaining = list(items)
    rx, ry, rw, rh = x, y, w, h

    while remaining:
        rem_total = sum(v for _, v in remaining)
        if rem_total <= 0:
            break

        short_side = min(rw, rh)
        if short_side <= 0:
            break

        row: list[tuple[TreeNode, float]] = []
        row_area = 0.0
        best_ratio = float("inf")

        for node, val in remaining:
            test_row = row + [(node, val)]
            test_area = row_area + val
            area_frac = (test_area / rem_total) * rw * rh
            long_side = area_frac / short_side if short_side > 0 else 0

            worst = 0.0
            for _, rv in test_row:
                cell_frac = rv / test_area if test_area > 0 else 0
                cell_long = long_side * cell_frac
                cell_short = area_frac / long_side if long_side > 0 else 0
                if cell_long > 0 and cell_short > 0:
                    ratio = max(cell_long / cell_short, cell_short / cell_long)
                    worst = max(worst, ratio)

            if worst <= best_ratio:
                row = test_row
                row_area = test_area
                best_ratio = worst
            else:
                break

        row_frac = row_area / rem_total if rem_total > 0 else 0
        if rw >= rh:
            row_w = rw * row_frac
            cy = ry
            for node, val in row:
                cell_frac = val / row_area if row_area > 0 else 0
                cell_h = rh * cell_frac
                result.append((node, rx, cy, row_w, cell_h))
                cy += cell_h
            rx += row_w
            rw -= row_w
        else:
            row_h = rh * row_frac
            cx = rx
            for node, val in row:
                cell_frac = val / row_area if row_area > 0 else 0
                cell_w = rw * cell_frac
                result.append((node, cx, ry, cell_w, row_h))
                cx += cell_w
            ry += row_h
            rh -= row_h

        remaining = remaining[len(row):]

    return result


class _StatsCanvas(Component):
    """Canvas component that renders treemap, stats bar, and timeline strip."""

    def __init__(self, app: "StatsApp") -> None:
        self._app = app

    def is_grow(self) -> bool:
        return True

    def measure(self, _avail_w: float) -> float:
        return 0.0

    def render(self, ctx, x: float, y: float, w: float, h: float) -> None:
        app = self._app
        # Store canvas y-offset so on_click can translate global coords
        app._canvas_y = y

        if not app.root or not app.view_root:
            ctx.text(x + w / 2 - 80, y + h / 2, "No events found",
                     size=TEXT_BODY, color=ctx.theme.muted)
            if app.events_path:
                ctx.text(x + w / 2 - 120, y + h / 2 + 24,
                         f"Path: {app.events_path}", size=TEXT_HINT, color=ctx.theme.muted)
            else:
                ctx.text(x + w / 2 - 120, y + h / 2 + 24,
                         "No events.jsonl found", size=TEXT_HINT, color=ctx.theme.muted)
            return

        vr = app.view_root

        # treemap region (above stats bar and timeline)
        treemap_h = h - STATS_BAR_H - TIMELINE_H
        if treemap_h < 10:
            treemap_h = 10

        children = vr.children if vr.children else [vr]
        items = [(c, c.total_duration) for c in children if c.total_duration > 0]
        app.cells = _squarify(items, (x, y, w, treemap_h))

        for node, cx, cy, cw, ch in app.cells:
            draw_x = max(x, cx + 1)
            draw_y = max(y, cy + 1)
            draw_r = min(x + w, cx + max(0, cw - 1))
            draw_b = min(y + treemap_h, cy + max(0, ch - 1))
            draw_w = max(0.0, draw_r - draw_x)
            draw_h = max(0.0, draw_b - draw_y)
            if draw_w <= 0 or draw_h <= 0:
                continue

            color = PALETTE[node.color_index % len(PALETTE)]
            is_highlighted = app.highlight_path and node.path == app.highlight_path
            fill = color if not is_highlighted else "#ffffff"
            ctx.rect(draw_x, draw_y, draw_w, draw_h,
                     fill=dim(fill, 180 if not is_highlighted else 255), radius=3.0)

            if draw_w >= MIN_CELL_W and draw_h >= MIN_CELL_H:
                parent_label = _shorten(os.path.dirname(node.path))
                if parent_label and parent_label != _shorten(vr.path):
                    ctx.text(draw_x + 5, draw_y + draw_h - 14 - TEXT_BODY - TEXT_HINT - 2,
                             parent_label, size=TEXT_HINT, color=dim(ctx.theme.fg, 100))

                ctx.text(draw_x + 5, draw_y + draw_h - 14 - TEXT_BODY,
                         node.label, size=TEXT_BODY, color=ctx.theme.fg, bold=True)
                ctx.text(draw_x + 5, draw_y + draw_h - 14,
                         _fmt_duration(node.total_duration), size=TEXT_CAPTION,
                         color=dim(ctx.theme.fg, 180))

                if node.visits > 0:
                    ctx.text(draw_x + draw_w - 40, draw_y + 5,
                             f"{node.visits}x", size=TEXT_HINT, color=dim(ctx.theme.fg, 100))

        # stats bar
        stats_y = y + treemap_h
        ctx.rect(x, stats_y, w, STATS_BAR_H, fill=ctx.theme.surface)
        ctx.line(x, stats_y, x + w, stats_y, color=dim(ctx.theme.muted, 60), width=1.0)
        ctx.line(x, stats_y + STATS_BAR_H, x + w, stats_y + STATS_BAR_H,
                 color=dim(ctx.theme.muted, 60), width=1.0)

        stats_text_y = stats_y + (STATS_BAR_H - TEXT_CAPTION) / 2
        quarter = w / 4
        compact = w < 620
        active_label = f"{_fmt_duration(app.total_time)} {'act' if compact else 'active'}"
        idle_label = f"{_fmt_duration(app.idle_skipped_time)} {'idle' if compact else 'idle skipped'}"
        clamped_label = f"{_fmt_duration(app.idle_clamped_time)} {'clamp' if compact else 'clamped'}"
        visits_label = (
            f"{app.total_visits}v · {app.total_projects}p"
            if compact
            else f"{app.total_visits} visits · {app.total_projects} projects"
        )
        ctx.text(x + quarter * 0.5, stats_text_y,
                 active_label,
                 size=TEXT_CAPTION, color=ctx.theme.accent, bold=True, align="center")
        ctx.text(x + quarter * 1.5, stats_text_y,
                 idle_label,
                 size=TEXT_CAPTION, color=ctx.theme.muted, bold=True, align="center")
        ctx.text(x + quarter * 2.5, stats_text_y,
                 clamped_label,
                 size=TEXT_CAPTION, color=ctx.theme.warning, bold=True, align="center")
        ctx.text(x + quarter * 3.5, stats_text_y,
                 visits_label,
                 size=TEXT_CAPTION, color=ctx.theme.success, bold=True, align="center")

        # timeline strip
        tl_y = stats_y + STATS_BAR_H
        ctx.rect(x, tl_y, w, TIMELINE_H, fill=ctx.theme.bg_darkest)

        bar_y = tl_y + 4
        bar_h = TIMELINE_H - 20
        idle_y = bar_y + bar_h * 0.58
        idle_h = max(4.0, bar_h * 0.32)
        for raw_start, raw_end, start_frac, end_frac, _, _, color_idx, state in app.timeline:
            if state in {"clamped", "skipped"} and raw_end > raw_start:
                bx = x + raw_start * w
                bw = max(2.0, (raw_end - raw_start) * w)
                fill = ctx.theme.warning if state == "clamped" else ctx.theme.muted
                ctx.rect(bx, idle_y, bw, idle_h, fill=dim(fill, 90), radius=2.0)
            if end_frac <= start_frac:
                continue
            bx = x + start_frac * w
            bw = max(2.0, (end_frac - start_frac) * w)
            color = PALETTE[color_idx % len(PALETTE)]
            ctx.rect(bx, bar_y, bw, bar_h, fill=dim(color, 200), radius=2.0)

        # hour markers (dynamic, aligned to rolling 24h window)
        marker_y = tl_y + TIMELINE_H - 14
        now = datetime.now(timezone.utc)
        start_time = now - timedelta(hours=24)
        labels: list[tuple[float, str]] = []
        for tick in range(7):
            frac = tick / 6
            t = start_time + timedelta(hours=24 * frac)
            if tick == 6:
                lbl = "now"
            else:
                hr = t.astimezone().hour
                suffix = "a" if hr < 12 else "p"
                h12 = hr % 12 or 12
                lbl = f"{h12}{suffix}"
            labels.append((frac, lbl))
        for frac, lbl in labels:
            mx = x + frac * w
            ctx.text(mx, marker_y, lbl, size=TEXT_HINT,
                     color=dim(ctx.theme.muted, 120), align="center")

        app.highlight_path = None


class StatsApp(App):

    async def on_init(self) -> None:
        self.root: TreeNode | None = None
        self.view_root: TreeNode | None = None
        self.view_stack: list[TreeNode] = []
        self.cells: list[tuple[TreeNode, float, float, float, float]] = []
        self.timeline: list[tuple[float, float, float, float, str, str, int, str]] = []
        self.total_time = 0.0
        self.raw_time = 0.0
        self.idle_skipped_time = 0.0
        self.idle_clamped_time = 0.0
        self.total_visits = 0
        self.total_projects = 0
        self.events_path: Path | None = None
        self.highlight_path: str | None = None
        self._canvas_y: float = 0.0
        self._canvas = _StatsCanvas(self)
        self._load()
        self.emit.info("stats: initialized")

    def _load(self) -> None:
        self.events_path = _resolve_events_path()
        if not self.events_path:
            self.emit.warn("stats: no events.jsonl found")
            self.root = None
            self.view_root = None
            self.view_stack = []
            self.timeline = []
            self.total_time = 0.0
            self.raw_time = 0.0
            self.idle_skipped_time = 0.0
            self.idle_clamped_time = 0.0
            self.total_visits = 0
            self.total_projects = 0
            return

        events = _parse_events(self.events_path)
        self.emit.info(f"stats: loaded {len(events)} focus events from {self.events_path}")
        normalized_events, idle_stats = _normalize_focus_events(events)
        counted_events = [ev for ev in normalized_events if _event_duration(ev) > 0]
        self.emit.info(
            "stats: normalized focus events "
            f"raw={len(events)} counted={idle_stats['counted_events']} "
            f"clamped={idle_stats['clamped_events']} skipped={idle_stats['skipped_events']} "
            f"raw_secs={idle_stats['raw_secs']:.0f} "
            f"counted_secs={idle_stats['counted_secs']:.0f} "
            f"clamped_secs={idle_stats['clamped_secs']:.0f} "
            f"skipped_secs={idle_stats['skipped_secs']:.0f}"
        )

        self.root = _build_tree(counted_events)
        self.view_root = self.root
        self.view_stack = []

        self.total_time = float(idle_stats["counted_secs"])
        self.raw_time = float(idle_stats["raw_secs"])
        self.idle_clamped_time = float(idle_stats["clamped_secs"])
        self.idle_skipped_time = float(idle_stats["skipped_secs"])
        self.total_visits = int(idle_stats["counted_events"])
        self.total_projects = len({_event_context_identity(ev)[0] for ev in counted_events})

        now = datetime.now(timezone.utc)
        start_window = now - timedelta(hours=24)
        self.timeline = []
        context_colors = {}
        if self.root:
            context_colors = {child.path: i for i, child in enumerate(self.root.children)}
        for ev in normalized_events:
            ts = ev["_ts"]
            dur = _event_duration(ev)
            raw_dur = _event_duration(ev, "_raw_duration_secs")
            cwd = _clean_text(ev.get("cwd")) or ""
            context_path, _ = _event_context_identity(ev)
            idle_state = str(ev.get("_idle_state") or "active")
            raw_start_frac, raw_end_frac, start_frac, end_frac = _timeline_fractions(
                ts,
                dur,
                raw_dur,
                idle_state,
                start_window,
            )
            color_idx = context_colors.get(context_path, 0)
            self.timeline.append((
                raw_start_frac,
                raw_end_frac,
                start_frac,
                end_frac,
                context_path,
                cwd,
                color_idx,
                idle_state,
            ))

        self.emit.status_summary(f"{_fmt_duration(self.total_time)} active")

    def on_render(self, ctx) -> None:
        subtitle = (
            f"{_fmt_duration(self.total_time)} active · "
            f"{_fmt_duration(self.idle_skipped_time)} idle skipped · "
            f"{self.total_visits} visits"
        )
        ctx.render(Column(
            [
                AppBar(title="Stats", subtitle=subtitle),
                self._canvas,
                FooterKeys([("r", "refresh"), ("esc", "back"), ("click", "drill down")]),
            ],
            padding=0,
            gap=0,
        ))

    def on_escape(self):
        if self.view_stack:
            self.view_root = self.view_stack.pop()
            self.emit.info(f"stats: navigated up to {self.view_root.path}")
            self.emit.schedule_render(0)
            return True
        return False

    def on_key(self, key: str, _mods: dict) -> None:
        if key == "r":
            self._load()
            self.emit.info("stats: refreshed")
            self.emit.schedule_render(0)
        elif key == "backspace":
            if self.view_stack:
                self.view_root = self.view_stack.pop()
                self.emit.info(f"stats: navigated up to {self.view_root.path}")
                self.emit.schedule_render(0)

    def on_click(self, x: float, y: float, _button: str) -> None:
        w = self.w
        # Translate global y to canvas-local y
        local_y = y - self._canvas_y

        # Determine timeline bounds in canvas-local space
        canvas_h = self.h - self._canvas_y
        # canvas renders: treemap then stats bar then timeline at the bottom
        tl_local_y = canvas_h - TIMELINE_H

        if local_y >= tl_local_y:
            bar_local_y = tl_local_y + 4
            bar_h = TIMELINE_H - 20
            if bar_local_y <= local_y <= bar_local_y + bar_h:
                frac = x / w if w > 0 else 0
                for _, _, start_frac, end_frac, context_path, cwd, _, state in self.timeline:
                    if state == "skipped":
                        continue
                    if start_frac <= frac <= end_frac:
                        if self.view_root and self.view_root.path == context_path and cwd:
                            self.highlight_path = cwd
                        else:
                            self.highlight_path = context_path
                        self.emit.info(f"stats: highlighted {self.highlight_path}")
                        self.emit.schedule_render(0)
                        break
            return

        # Treemap cells are stored with global coordinates from canvas render
        for node, cx, cy, cw, ch in self.cells:
            if cx <= x <= cx + cw and cy <= y <= cy + ch:
                if node.children and self.view_root is not None:
                    self.view_stack.append(self.view_root)
                    self.view_root = node
                    self.emit.info(f"stats: drilled into {node.path}")
                    self.emit.schedule_render(0)
                break


if __name__ == "__main__":
    StatsApp().run()
