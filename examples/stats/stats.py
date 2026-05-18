#!/usr/bin/env python3
from __future__ import annotations

import json
import os
import sys
from datetime import datetime, timedelta, timezone
from pathlib import Path

sys.path.insert(0, os.path.join(os.path.dirname(__file__), '../../sdk/python'))

from plexi_sdk import App, RenderContext, FG, MUTED, ACCENT, HINT, CAPTION, BODY, dim

PALETTE = [
    "#f38ba8", "#a6e3a1", "#89b4fa", "#f9e2af",
    "#cba6f7", "#94e2d5", "#fab387", "#74c7ec",
]

BREADCRUMB_H = 24.0
STATS_BAR_H = 34.0
NODE_W = 120.0
NODE_H = 44.0
NODE_H_GAP = 28.0
NODE_V_GAP = 60.0
EDGE_COLOR = "#45475a"
HOME = str(Path.home())


def _shorten(path: str) -> str:
    if path == HOME:
        return "~"
    if path.startswith(HOME + "/"):
        return "~" + path[len(HOME):]
    return path


def _label(path: str) -> str:
    short = _shorten(path)
    parts = short.rstrip("/").split("/")
    return parts[-1] if parts[-1] else short


def _fmt_duration(secs: float) -> str:
    h = int(secs) // 3600
    m = (int(secs) % 3600) // 60
    if h > 0:
        return f"{h}h {m:02d}m"
    if m > 0:
        return f"{m}m"
    return f"{int(secs)}s"


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
    return sorted(events, key=lambda e: e["_ts"])


# --- DAG data structures ---

class FlowNode:
    __slots__ = ("path", "label", "total_duration", "visits", "color_index",
                 "layer", "col", "x", "y")

    def __init__(self, path: str):
        self.path = path
        self.label = _label(path)
        self.total_duration = 0.0
        self.visits = 0
        self.color_index = 0
        self.layer = 0
        self.col = 0
        self.x = 0.0
        self.y = 0.0


class FlowEdge:
    __slots__ = ("src", "dst", "count")

    def __init__(self, src: str, dst: str):
        self.src = src
        self.dst = dst
        self.count = 1


def _build_dag(events: list[dict]) -> tuple[dict[str, FlowNode], list[FlowEdge]]:
    nodes: dict[str, FlowNode] = {}
    edges: dict[tuple[str, str], FlowEdge] = {}
    color_map: dict[str, int] = {}
    color_counter = [0]

    def _color(path: str) -> int:
        top = path.replace(HOME, "").strip("/").split("/")[0] or "~"
        if top not in color_map:
            color_map[top] = color_counter[0] % len(PALETTE)
            color_counter[0] += 1
        return color_map[top]

    prev_cwd: str | None = None
    for ev in events:
        cwd = ev.get("cwd") or HOME
        dur = ev.get("duration_secs", 0.0)

        if cwd not in nodes:
            n = FlowNode(cwd)
            n.color_index = _color(cwd)
            nodes[cwd] = n
        nodes[cwd].total_duration += dur
        nodes[cwd].visits += 1

        if prev_cwd is not None and prev_cwd != cwd:
            key = (prev_cwd, cwd)
            if key in edges:
                edges[key].count += 1
            else:
                edges[key] = FlowEdge(prev_cwd, cwd)
        prev_cwd = cwd

    return nodes, list(edges.values())


def _layout_dag(
    nodes: dict[str, FlowNode],
    edges: list[FlowEdge],
    canvas_w: float,
    _canvas_h: float,
    scroll_x: float,
    scroll_y: float,
) -> None:
    if not nodes:
        return

    # Build adjacency for layer assignment
    out_edges: dict[str, list[str]] = {k: [] for k in nodes}
    in_degree: dict[str, int] = {k: 0 for k in nodes}
    for e in edges:
        if e.src in nodes and e.dst in nodes:
            out_edges[e.src].append(e.dst)
            in_degree[e.dst] = in_degree.get(e.dst, 0) + 1

    # BFS layer assignment from sources
    layer: dict[str, int] = {}
    queue = [k for k, d in in_degree.items() if d == 0]
    if not queue:
        queue = sorted(nodes.keys(), key=lambda k: -nodes[k].visits)

    while queue:
        nxt = queue.pop(0)
        if nxt in layer:
            continue
        max_pred = -1
        for e in edges:
            if e.dst == nxt and e.src in layer:
                max_pred = max(max_pred, layer[e.src])
        layer[nxt] = max_pred + 1
        for succ in out_edges.get(nxt, []):
            queue.append(succ)

    max_layer = max(layer.values()) if layer else 0
    for k in nodes:
        if k not in layer:
            layer[k] = max_layer + 1

    # Assign columns within each layer sorted by visits descending
    by_layer: dict[int, list[str]] = {}
    for k, l in layer.items():
        by_layer.setdefault(l, []).append(k)
    for l in by_layer:
        by_layer[l].sort(key=lambda k: -nodes[k].visits)

    # Compute positions centered horizontally per layer
    for l, keys in by_layer.items():
        row_count = len(keys)
        row_w = row_count * NODE_W + (row_count - 1) * NODE_H_GAP
        start_x = max(0.0, (canvas_w - row_w) / 2) + scroll_x
        for col, k in enumerate(keys):
            n = nodes[k]
            n.layer = l
            n.col = col
            n.x = start_x + col * (NODE_W + NODE_H_GAP)
            n.y = BREADCRUMB_H + 16 + l * (NODE_H + NODE_V_GAP) + scroll_y


# --- App ---

class StatsApp(App):

    async def on_init(self, ctx: RenderContext) -> None:
        self.nodes: dict[str, FlowNode] = {}
        self.edges: list[FlowEdge] = []
        self.highlighted: str | None = None
        self.highlight_chain: set[str] = set()
        self.total_time = 0.0
        self.total_visits = 0
        self.context_switches = 0
        self.dominant: str | None = None
        self.events_path: Path | None = None
        self.scroll_x = 0.0
        self.scroll_y = 0.0
        self._dragging = False
        self._drag_sx = 0.0
        self._drag_sy = 0.0
        self._drag_ox = 0.0
        self._drag_oy = 0.0
        self._layout_w = 0.0
        self._layout_h = 0.0
        self._load(ctx)
        self.emit.info("stats: initialized")

    def _load(self, ctx: RenderContext) -> None:
        self.events_path = _resolve_events_path()
        if not self.events_path:
            self.emit.warn("stats: no events.jsonl found")
            self.nodes = {}
            self.edges = []
            self.total_time = 0.0
            self.total_visits = 0
            self.context_switches = 0
            self.dominant = None
            return

        events = _parse_events(self.events_path)
        self.nodes, self.edges = _build_dag(events)
        self.total_time = sum(ev.get("duration_secs", 0.0) for ev in events)
        self.total_visits = len(events)
        self.context_switches = len(self.edges)
        if self.nodes:
            dom = max(self.nodes.values(), key=lambda n: n.total_duration)
            self.dominant = dom.label
        else:
            self.dominant = None

        self._layout_w = 0.0  # trigger relayout on next render
        self.highlighted = None
        self.highlight_chain = set()

        self.emit.info(
            f"stats: loaded {len(events)} events, "
            f"{len(self.nodes)} nodes, {len(self.edges)} edges"
        )
        ctx.status_summary(f"{_fmt_duration(self.total_time)} active · {self.context_switches} switches")

    def _ensure_layout(self, ctx: RenderContext) -> None:
        if self._layout_w != ctx.w or self._layout_h != ctx.h:
            _layout_dag(self.nodes, self.edges, ctx.w, ctx.h, self.scroll_x, self.scroll_y)
            self._layout_w = ctx.w
            self._layout_h = ctx.h

    def _relayout(self, ctx: RenderContext) -> None:
        _layout_dag(self.nodes, self.edges, ctx.w, ctx.h, self.scroll_x, self.scroll_y)
        self._layout_w = ctx.w
        self._layout_h = ctx.h

    def _build_chain(self, path: str) -> set[str]:
        chain: set[str] = {path}
        visited: set[str] = set()
        frontier = [path]
        while frontier:
            cur = frontier.pop()
            if cur in visited:
                continue
            visited.add(cur)
            for e in self.edges:
                if e.dst == cur and e.src not in chain:
                    chain.add(e.src)
                    frontier.append(e.src)
        visited = set()
        frontier = [path]
        while frontier:
            cur = frontier.pop()
            if cur in visited:
                continue
            visited.add(cur)
            for e in self.edges:
                if e.src == cur and e.dst not in chain:
                    chain.add(e.dst)
                    frontier.append(e.dst)
        return chain

    def on_render(self, ctx: RenderContext) -> None:
        ctx.clear("#11111b")
        w, h = ctx.w, ctx.h

        ctx.rect(0, 0, w, BREADCRUMB_H, fill="#181825")
        ctx.text(8, 5, "Focus Flow · 24h", size=HINT, color=MUTED, bold=True)
        ctx.text(w - 260, 5,
                 "r refresh · click highlight · esc clear · drag scroll",
                 size=HINT, color=dim(MUTED, 120))

        if not self.nodes:
            ctx.text(w / 2 - 80, h / 2, "No focus events found", size=BODY, color=MUTED)
            if self.events_path:
                ctx.text(w / 2 - 120, h / 2 + 24,
                         f"Path: {self.events_path}", size=HINT, color=MUTED)
            else:
                ctx.text(w / 2 - 120, h / 2 + 24,
                         "No events.jsonl found in any Plexi profile", size=HINT, color=MUTED)
            return

        self._ensure_layout(ctx)

        canvas_top = BREADCRUMB_H
        canvas_h = h - BREADCRUMB_H - STATS_BAR_H
        ctx.rect(0, canvas_top, w, canvas_h, fill="#11111b")

        max_count = max((e.count for e in self.edges), default=1)
        for edge in self.edges:
            if edge.src not in self.nodes or edge.dst not in self.nodes:
                continue
            src = self.nodes[edge.src]
            dst = self.nodes[edge.dst]
            in_chain = (
                bool(self.highlight_chain)
                and edge.src in self.highlight_chain
                and edge.dst in self.highlight_chain
            )
            sx = src.x + NODE_W / 2
            sy = src.y + NODE_H
            dx = dst.x + NODE_W / 2
            dy = dst.y
            if max(sy, dy) < canvas_top or min(sy, dy) > canvas_top + canvas_h:
                continue
            alpha = 180 if in_chain else 80
            width = 1.0 + 2.0 * (edge.count / max_count)
            color = "#89b4fa" if in_chain else EDGE_COLOR
            mx = (sx + dx) / 2
            my = (sy + dy) / 2
            ctx.line(sx, sy, mx, my, color=dim(color, alpha), width=width)
            ctx.line(mx, my, dx, dy, color=dim(color, alpha), width=width)

        for path, node in self.nodes.items():
            nx, ny = node.x, node.y
            if ny + NODE_H < canvas_top or ny > canvas_top + canvas_h:
                continue
            in_chain = bool(self.highlight_chain and path in self.highlight_chain)
            is_focal = path == self.highlighted
            base_color = PALETTE[node.color_index % len(PALETTE)]
            if is_focal:
                fill = base_color
                text_alpha = 255
            elif in_chain:
                fill = dim(base_color, 160)
                text_alpha = 200
            else:
                fill = dim(base_color, 60)
                text_alpha = 80
            ctx.rect(nx, ny, NODE_W, NODE_H, fill=fill, radius=6.0)
            label = node.label
            if len(label) > 14:
                label = label[:12] + ".."
            ctx.text(nx + NODE_W / 2, ny + 8, label,
                     size=CAPTION, color=dim(FG, text_alpha), bold=True, align="center")
            ctx.text(nx + NODE_W / 2, ny + 8 + CAPTION + 4,
                     _fmt_duration(node.total_duration),
                     size=HINT, color=dim(FG, max(40, text_alpha - 40)), align="center")
            if node.visits > 1:
                ctx.text(nx + NODE_W - 6, ny + 4,
                         f"{node.visits}x", size=HINT,
                         color=dim(FG, max(40, text_alpha - 40)), align="right")

        stats_y = h - STATS_BAR_H
        ctx.rect(0, stats_y, w, STATS_BAR_H, fill="#181825")
        ctx.line(0, stats_y, w, stats_y, color=dim(MUTED, 60), width=1.0)
        stats_text_y = stats_y + (STATS_BAR_H - CAPTION) / 2
        third = w / 3
        ctx.text(third * 0.5, stats_text_y,
                 f"{_fmt_duration(self.total_time)} active",
                 size=CAPTION, color=ACCENT, bold=True, align="center")
        ctx.text(third * 1.5, stats_text_y,
                 f"{self.context_switches} switches",
                 size=CAPTION, color="#a6e3a1", bold=True, align="center")
        dom_label = self.dominant or "—"
        ctx.text(third * 2.5, stats_text_y,
                 f"top: {dom_label}",
                 size=CAPTION, color="#f9e2af", bold=True, align="center")

    def on_key(self, ctx: RenderContext, key: str, _mods: dict) -> None:
        if key == "r":
            self._load(ctx)
            self._relayout(ctx)
            self.emit.info("stats: refreshed")
            ctx.emit.schedule_render(0)
        elif key in ("escape", "backspace"):
            self.highlighted = None
            self.highlight_chain = set()
            self.emit.info("stats: cleared highlight")
            ctx.emit.schedule_render(0)

    def on_click(self, ctx: RenderContext, x: float, y: float, _button: str) -> None:
        if y < BREADCRUMB_H:
            return
        for path, node in self.nodes.items():
            if node.x <= x <= node.x + NODE_W and node.y <= y <= node.y + NODE_H:
                if self.highlighted == path:
                    self.highlighted = None
                    self.highlight_chain = set()
                    self.emit.info("stats: cleared highlight")
                else:
                    self.highlighted = path
                    self.highlight_chain = self._build_chain(path)
                    self.emit.info(
                        f"stats: highlighted {node.label} "
                        f"({len(self.highlight_chain)} connected)"
                    )
                ctx.emit.schedule_render(0)
                return

    def on_mouse_down(self, _ctx: RenderContext, x: float, y: float, button: str) -> None:
        if button != "left" or y <= BREADCRUMB_H:
            return
        for _, node in self.nodes.items():
            if node.x <= x <= node.x + NODE_W and node.y <= y <= node.y + NODE_H:
                return
        self._dragging = True
        self._drag_sx = x
        self._drag_sy = y
        self._drag_ox = self.scroll_x
        self._drag_oy = self.scroll_y

    def on_mouse_up(self, _ctx: RenderContext, _x: float, _y: float, _button: str) -> None:
        self._dragging = False

    def on_mouse_move(self, ctx: RenderContext, x: float, y: float, _buttons: list) -> None:
        if not self._dragging:
            return
        self.scroll_x = self._drag_ox + (x - self._drag_sx)
        self.scroll_y = self._drag_oy + (y - self._drag_sy)
        self._relayout(ctx)
        ctx.emit.schedule_render(0)


if __name__ == "__main__":
    StatsApp().run()
