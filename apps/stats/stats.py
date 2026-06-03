#!/usr/bin/env python3
from __future__ import annotations

import json
import os
from datetime import datetime, timedelta, timezone
from pathlib import Path

from plexi_sdk import App, RenderContext, dim
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
HOME = str(Path.home())


def _shorten(path: str) -> str:
    if path == HOME:
        return "~"
    if path.startswith(HOME + "/"):
        return "~" + path[len(HOME):]
    return path


def _fmt_duration(secs: float) -> str:
    h = int(secs) // 3600
    m = (int(secs) % 3600) // 60
    if h > 0:
        return f"{h}h {m:02d}m"
    return f"{m}m"


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
    by_cwd: dict[str, tuple[float, int]] = {}
    for ev in events:
        cwd = ev.get("cwd") or HOME
        dur = ev.get("duration_secs", 0)
        prev = by_cwd.get(cwd, (0.0, 0))
        by_cwd[cwd] = (prev[0] + dur, prev[1] + 1)

    root = TreeNode(HOME, "~")
    nodes: dict[str, TreeNode] = {HOME: root}

    for cwd in sorted(by_cwd.keys()):
        parts = cwd.split("/")
        for i in range(1, len(parts) + 1):
            prefix = "/".join(parts[:i]) or "/"
            if prefix not in nodes:
                parent_path = "/".join(parts[:i - 1]) or "/"
                parent = nodes.get(parent_path, root)
                node = TreeNode(prefix, parts[i - 1] if i > 0 else "/")
                nodes[prefix] = node
                parent.children.append(node)

    for cwd, (dur, visits) in by_cwd.items():
        node = nodes.get(cwd)
        if node:
            node.self_duration = dur
            node.visits = visits

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

    def render(self, ctx: RenderContext, x: float, y: float, w: float, h: float) -> None:
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
            color = PALETTE[node.color_index % len(PALETTE)]
            is_highlighted = app.highlight_path and node.path == app.highlight_path
            fill = color if not is_highlighted else "#ffffff"
            ctx.rect(cx + 1, cy + 1, max(0, cw - 2), max(0, ch - 2),
                     fill=dim(fill, 180 if not is_highlighted else 255), radius=3.0)

            if cw >= MIN_CELL_W and ch >= MIN_CELL_H:
                parent_label = _shorten(os.path.dirname(node.path))
                if parent_label and parent_label != _shorten(vr.path):
                    ctx.text(cx + 6, cy + ch - 14 - TEXT_BODY - TEXT_HINT - 2,
                             parent_label, size=TEXT_HINT, color=dim(ctx.theme.fg, 100))

                ctx.text(cx + 6, cy + ch - 14 - TEXT_BODY,
                         node.label, size=TEXT_BODY, color=ctx.theme.fg, bold=True)
                ctx.text(cx + 6, cy + ch - 14,
                         _fmt_duration(node.total_duration), size=TEXT_CAPTION,
                         color=dim(ctx.theme.fg, 180))

                if node.visits > 0:
                    ctx.text(cx + cw - 40, cy + 6,
                             f"{node.visits}x", size=TEXT_HINT, color=dim(ctx.theme.fg, 100))

        # stats bar
        stats_y = y + treemap_h
        ctx.rect(x, stats_y, w, STATS_BAR_H, fill="#181825")
        ctx.line(x, stats_y, x + w, stats_y, color=dim(ctx.theme.muted, 60), width=1.0)
        ctx.line(x, stats_y + STATS_BAR_H, x + w, stats_y + STATS_BAR_H,
                 color=dim(ctx.theme.muted, 60), width=1.0)

        stats_text_y = stats_y + (STATS_BAR_H - TEXT_CAPTION) / 2
        third = w / 3
        ctx.text(x + third * 0.5, stats_text_y,
                 f"{_fmt_duration(app.total_time)} active",
                 size=TEXT_CAPTION, color=ctx.theme.accent, bold=True, align="center")
        ctx.text(x + third * 1.5, stats_text_y,
                 f"{app.total_visits} visits",
                 size=TEXT_CAPTION, color="#a6e3a1", bold=True, align="center")
        ctx.text(x + third * 2.5, stats_text_y,
                 f"{app.total_projects} projects",
                 size=TEXT_CAPTION, color="#f9e2af", bold=True, align="center")

        # timeline strip
        tl_y = stats_y + STATS_BAR_H
        ctx.rect(x, tl_y, w, TIMELINE_H, fill="#11111b")

        bar_y = tl_y + 4
        bar_h = TIMELINE_H - 20
        for start_frac, end_frac, _, color_idx in app.timeline:
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

    async def on_init(self, ctx: RenderContext) -> None:
        self.root: TreeNode | None = None
        self.view_root: TreeNode | None = None
        self.view_stack: list[TreeNode] = []
        self.cells: list[tuple[TreeNode, float, float, float, float]] = []
        self.timeline: list[tuple[float, float, str, int]] = []
        self.total_time = 0.0
        self.total_visits = 0
        self.total_projects = 0
        self.events_path: Path | None = None
        self.highlight_path: str | None = None
        self._canvas_y: float = 0.0
        self._canvas = _StatsCanvas(self)
        self._load(ctx)
        self.emit.info("stats: initialized")

    def _load(self, ctx: RenderContext) -> None:
        self.events_path = _resolve_events_path()
        if not self.events_path:
            self.emit.warn("stats: no events.jsonl found")
            self.root = None
            self.view_root = None
            self.view_stack = []
            self.timeline = []
            self.total_time = 0.0
            self.total_visits = 0
            self.total_projects = 0
            return

        events = _parse_events(self.events_path)
        self.emit.info(f"stats: loaded {len(events)} focus events from {self.events_path}")

        self.root = _build_tree(events)
        self.view_root = self.root
        self.view_stack = []

        self.total_time = sum(ev.get("duration_secs", 0) for ev in events)
        self.total_visits = len(events)
        self.total_projects = len({ev.get("context_name", "") for ev in events})

        now = datetime.now(timezone.utc)
        start_window = now - timedelta(hours=24)
        self.timeline = []
        for ev in events:
            ts = ev["_ts"]
            dur = ev.get("duration_secs", 0)
            cwd = ev.get("cwd") or HOME
            end_frac = (ts - start_window).total_seconds() / 86400.0
            start_frac = ((ts - timedelta(seconds=dur)) - start_window).total_seconds() / 86400.0
            start_frac = max(0.0, min(1.0, start_frac))
            end_frac = max(0.0, min(1.0, end_frac))
            parts = cwd.replace(HOME, "").strip("/").split("/")
            top_dir = parts[0] if parts and parts[0] else "~"
            color_idx = 0
            if self.root:
                for i, child in enumerate(self.root.children):
                    if child.label == top_dir or child.path.endswith("/" + top_dir):
                        color_idx = i
                        break
            self.timeline.append((start_frac, end_frac, cwd, color_idx))

        ctx.status_summary(f"{_fmt_duration(self.total_time)} active")

    def on_render(self, ctx: RenderContext) -> None:
        subtitle = (
            f"{_fmt_duration(self.total_time)} active · "
            f"{self.total_visits} visits · "
            f"{self.total_projects} projects"
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

    def on_key(self, ctx: RenderContext, key: str, _mods: dict) -> None:
        if key == "r":
            self._load(ctx)
            self.emit.info("stats: refreshed")
            ctx.emit.schedule_render(0)
        elif key in ("escape", "backspace"):
            if self.view_stack:
                self.view_root = self.view_stack.pop()
                self.emit.info(f"stats: navigated up to {self.view_root.path}")
                ctx.emit.schedule_render(0)

    def on_click(self, ctx: RenderContext, x: float, y: float, _button: str) -> None:
        w = ctx.w
        # Translate global y to canvas-local y
        local_y = y - self._canvas_y

        # Determine timeline bounds in canvas-local space
        canvas_h = ctx.h - self._canvas_y
        # canvas renders: treemap then stats bar then timeline at the bottom
        tl_local_y = canvas_h - TIMELINE_H

        if local_y >= tl_local_y:
            bar_local_y = tl_local_y + 4
            bar_h = TIMELINE_H - 20
            if bar_local_y <= local_y <= bar_local_y + bar_h:
                frac = x / w if w > 0 else 0
                for start_frac, end_frac, cwd, _ in self.timeline:
                    if start_frac <= frac <= end_frac:
                        self.highlight_path = cwd
                        self.emit.info(f"stats: highlighted {cwd}")
                        ctx.emit.schedule_render(0)
                        break
            return

        # Treemap cells are stored with global coordinates from canvas render
        for node, cx, cy, cw, ch in self.cells:
            if cx <= x <= cx + cw and cy <= y <= cy + ch:
                if node.children and self.view_root is not None:
                    self.view_stack.append(self.view_root)
                    self.view_root = node
                    self.emit.info(f"stats: drilled into {node.path}")
                    ctx.emit.schedule_render(0)
                break


if __name__ == "__main__":
    StatsApp().run()
