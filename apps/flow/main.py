#!/usr/bin/env python3
"""Flow — node-based workflow canvas (n8n-style POC).

Drag nodes on a pannable canvas, wire outputs into inputs, expand any node
into a full text editor, then run the graph: prompts template into other
prompts, file nodes read/write the local filesystem, shell nodes pipe text
through commands. The AI node is a correctly-shaped stub for a later broker.

Graph persists to <workspace_root>/.plexi/flow.json.
"""
from __future__ import annotations

import asyncio
import json
import pathlib
import subprocess
import time

from plexi_sdk import App, dim
from plexi_sdk.ui import AppBar, Column, Component, FooterKeys, Label, Spacer, TextEdit

FLOW_FILE = ".plexi/flow.json"
SHELL_TIMEOUT = 15.0

NODE_W = 190.0
NODE_H = 76.0
HEADER_H = 26.0
PORT_R = 6.0
PORT_HIT_R = 14.0
EDGE_HIT_DIST = 7.0
DOUBLE_CLICK_S = 0.35
CLICK_SLOP = 4.0

BG = "#11111b"
GRID_DOT = "#313244"
SURFACE = "#1e1e2e"
SURFACE_SEL = "#2a2a40"
BORDER = "#45475a"
ACCENT = "#89b4fa"
TEXT = "#cdd6f4"
TEXT_DIM = "#7f849c"
OK = "#a6e3a1"
ERR = "#f38ba8"
WIRE = "#6c7086"

KINDS = {
    "prompt":     {"label": "Prompt",     "color": "#89b4fa",
                   "hint": "Template text. {{input}} is replaced by upstream output."},
    "file_read":  {"label": "File Read",  "color": "#a6e3a1",
                   "hint": "Path to read (relative to workspace root or absolute)."},
    "file_write": {"label": "File Write", "color": "#f9e2af",
                   "hint": "Path to write. Upstream output becomes the file contents."},
    "shell":      {"label": "Shell",      "color": "#fab387",
                   "hint": "Command to run. Upstream output is piped to stdin."},
    "ai":         {"label": "AI",         "color": "#cba6f7",
                   "hint": "Prompt template ({{input}} substituted). Stub — no model wired yet."},
}
KIND_ORDER = list(KINDS)


class Node:
    def __init__(self, nid: int, kind: str, x: float, y: float,
                 title: str = "", text: str = "") -> None:
        self.id = nid
        self.kind = kind
        self.x = x
        self.y = y
        self.title = title or KINDS[kind]["label"]
        self.text = text
        self.status = "idle"   # idle | running | ok | error
        self.output = ""
        self.error = ""

    def to_dict(self) -> dict:
        return {"id": self.id, "kind": self.kind, "x": self.x, "y": self.y,
                "title": self.title, "text": self.text}

    @staticmethod
    def from_dict(d: dict) -> "Node":
        return Node(d["id"], d["kind"], d["x"], d["y"], d["title"], d["text"])


def _bezier_points(x1: float, y1: float, x2: float, y2: float,
                   steps: int = 18) -> list[tuple[float, float]]:
    """Horizontal-tangent cubic bezier between two ports, as a polyline."""
    dx = max(40.0, abs(x2 - x1) * 0.5)
    c1x, c1y = x1 + dx, y1
    c2x, c2y = x2 - dx, y2
    pts: list[tuple[float, float]] = []
    for i in range(steps + 1):
        t = i / steps
        mt = 1.0 - t
        px = (mt ** 3) * x1 + 3 * (mt ** 2) * t * c1x + 3 * mt * (t ** 2) * c2x + (t ** 3) * x2
        py = (mt ** 3) * y1 + 3 * (mt ** 2) * t * c1y + 3 * mt * (t ** 2) * c2y + (t ** 3) * y2
        pts.append((px, py))
    return pts


def _dist_to_segment(px: float, py: float, x1: float, y1: float,
                     x2: float, y2: float) -> float:
    vx, vy = x2 - x1, y2 - y1
    seg_sq = vx * vx + vy * vy
    if seg_sq <= 0.0:
        return ((px - x1) ** 2 + (py - y1) ** 2) ** 0.5
    t = max(0.0, min(1.0, ((px - x1) * vx + (py - y1) * vy) / seg_sq))
    cx, cy = x1 + t * vx, y1 + t * vy
    return ((px - cx) ** 2 + (py - cy) ** 2) ** 0.5


class _FlowCanvas(Component):
    """World-space node canvas. Records its layout rect for hit-testing."""

    def __init__(self, app: "FlowApp") -> None:
        self._app = app
        self.rect = (0.0, 0.0, 0.0, 0.0)

    def is_grow(self) -> bool:
        return True

    def measure(self, _avail_w: float) -> float:
        return 0.0

    # world <-> screen
    def to_screen(self, wx: float, wy: float) -> tuple[float, float]:
        x, y, _, _ = self.rect
        return (wx + self._app.ox + x, wy + self._app.oy + y)

    def to_world(self, sx: float, sy: float) -> tuple[float, float]:
        x, y, _, _ = self.rect
        return (sx - x - self._app.ox, sy - y - self._app.oy)

    def out_port(self, n: Node) -> tuple[float, float]:
        return self.to_screen(n.x + NODE_W, n.y + NODE_H / 2)

    def in_port(self, n: Node) -> tuple[float, float]:
        return self.to_screen(n.x, n.y + NODE_H / 2)

    def render(self, ctx, x: float, y: float, w: float, h: float) -> None:
        self.rect = (x, y, w, h)
        app = self._app
        ctx.push_clip(x, y, w, h)
        ctx.rect(x, y, w, h, BG)

        # dot grid, aligned to pan offset
        step = 28.0
        gy = y + (app.oy % step)
        while gy < y + h:
            gx = x + (app.ox % step)
            while gx < x + w:
                ctx.circle(gx, gy, 1.0, GRID_DOT)
                gx += step
            gy += step

        # edges
        for ei, (a_id, b_id) in enumerate(app.edges):
            a, b = app.nodes.get(a_id), app.nodes.get(b_id)
            if a is None or b is None:
                continue
            x1, y1 = self.out_port(a)
            x2, y2 = self.in_port(b)
            color = ACCENT if app.selected_edge == ei else WIRE
            width = 2.5 if app.selected_edge == ei else 1.8
            pts = _bezier_points(x1, y1, x2, y2)
            for (px, py), (qx, qy) in zip(pts, pts[1:]):
                ctx.line(px, py, qx, qy, color, width)

        # pending wire follows the cursor
        if app.drag and app.drag["type"] == "wire":
            src = app.nodes.get(app.drag["node"])
            if src is not None:
                mx, my = app.mouse
                if app.drag["dir"] == "out":
                    x1, y1 = self.out_port(src)
                    pts = _bezier_points(x1, y1, mx, my)
                else:
                    x2, y2 = self.in_port(src)
                    pts = _bezier_points(mx, my, x2, y2)
                for (px, py), (qx, qy) in zip(pts, pts[1:]):
                    ctx.line(px, py, qx, qy, ACCENT, 1.8)

        # nodes
        for n in app.nodes.values():
            sx, sy = self.to_screen(n.x, n.y)
            if sx + NODE_W < x or sx > x + w or sy + NODE_H < y or sy > y + h:
                continue
            kind = KINDS[n.kind]
            selected = app.selected_node == n.id
            ctx.rect(sx + 3, sy + 4, NODE_W, NODE_H, dim("#000000", 70), radius=8.0)
            ctx.rect(sx - 1, sy - 1, NODE_W + 2, NODE_H + 2,
                     ACCENT if selected else BORDER, radius=9.0)
            ctx.rect(sx, sy, NODE_W, NODE_H, SURFACE_SEL if selected else SURFACE,
                     radius=8.0)
            ctx.rect(sx, sy, NODE_W, HEADER_H, dim(kind["color"], 90), radius=8.0)
            ctx.rect(sx, sy + HEADER_H - 4, NODE_W, 4, dim(kind["color"], 90))
            ctx.text(sx + 10, sy + 6, n.title, 12.0, TEXT, bold=True,
                     max_width=NODE_W - 62)
            ctx.text(sx + NODE_W - 10, sy + 7, kind["label"].lower(), 9.0,
                     dim(kind["color"], 220), align="right")

            preview = n.text.strip().splitlines()[0] if n.text.strip() \
                else "(empty — ⏎ to edit)"
            ctx.text(sx + 10, sy + HEADER_H + 8, preview, 10.0,
                     TEXT_DIM if n.text.strip() else dim(TEXT_DIM, 150),
                     monospace=True, max_width=NODE_W - 20)

            # status row
            status_color = {"ok": OK, "error": ERR, "running": ACCENT}.get(n.status)
            if status_color:
                ctx.circle(sx + 14, sy + NODE_H - 14, 4.0, status_color)
                msg = n.error if n.status == "error" else \
                    (n.output.strip().splitlines()[0] if n.output.strip() else n.status)
                ctx.text(sx + 24, sy + NODE_H - 20, msg, 9.0, status_color,
                         max_width=NODE_W - 34)

            # ports
            for px, py in (self.in_port(n), self.out_port(n)):
                ctx.circle(px, py, PORT_R + 1.5, BORDER)
                ctx.circle(px, py, PORT_R, BG)
                ctx.circle(px, py, PORT_R - 2.5, kind["color"])

        if not app.nodes:
            ctx.text(x + w / 2, y + h / 2, "Press  a  to add your first node",
                     14.0, TEXT_DIM, align="center")
        ctx.pop_clip()


class FlowApp(App):
    default_background = BG

    # ── lifecycle ────────────────────────────────────────────────────────────
    def on_init(self) -> None:
        self.nodes: dict[int, Node] = {}
        self.edges: list[tuple[int, int]] = []
        self.next_id = 1
        self.ox = 0.0
        self.oy = 0.0
        self.mode = "canvas"           # canvas | edit | add
        self.edit_node: int | None = None
        self.selected_node: int | None = None
        self.selected_edge: int | None = None
        self.drag: dict | None = None
        self.mouse = (0.0, 0.0)
        self._down_pos = (0.0, 0.0)
        self._last_click: tuple[float, int] = (0.0, -1)
        self._running = False
        self._run_task: asyncio.Future | None = None
        self._canvas = _FlowCanvas(self)
        self.emit.set_mouse_tracking(True)
        self._load()
        self.emit.info(f"flow: init — {len(self.nodes)} nodes, {len(self.edges)} edges")

    def _path(self) -> pathlib.Path:
        return pathlib.Path(self.workspace_root) / FLOW_FILE

    def _load(self) -> None:
        try:
            p = self._path()
            if not p.exists():
                return
            data = json.loads(p.read_text())
            self.nodes = {n["id"]: Node.from_dict(n) for n in data.get("nodes", [])}
            self.edges = [tuple(e) for e in data.get("edges", [])]
            self.next_id = max(self.nodes, default=0) + 1
        except Exception as e:
            self.emit.error(f"flow: load failed from {self._path()}: {e}")

    def _save(self) -> None:
        try:
            p = self._path()
            p.parent.mkdir(parents=True, exist_ok=True)
            p.write_text(json.dumps({
                "nodes": [n.to_dict() for n in self.nodes.values()],
                "edges": [list(e) for e in self.edges],
            }, indent=2))
        except Exception as e:
            self.emit.error(f"flow: save failed to {self._path()}: {e}")

    # ── views ────────────────────────────────────────────────────────────────
    def view(self):
        if self.mode == "edit":
            return self._edit_view()
        if self.mode == "add":
            return self._add_view()
        return self._canvas_view()

    def _canvas_view(self):
        n, e = len(self.nodes), len(self.edges)
        subtitle = f"{n} node{'s' if n != 1 else ''} · {e} wire{'s' if e != 1 else ''}"
        if self._running:
            subtitle += " · running…"
        return Column([
            AppBar(title="Flow", subtitle=subtitle),
            self._canvas,
            FooterKeys([
                ("a", "add"), ("⏎", "edit"), ("drag port", "wire"),
                ("r", "run"), ("⌫", "delete"),
            ]),
        ])

    def _add_view(self):
        rows: list = [AppBar(title="Flow", subtitle="add node"), Spacer(12.0)]
        for i, kind in enumerate(KIND_ORDER, start=1):
            k = KINDS[kind]
            rows.append(Label(f"{i}   {k['label']}  —  {k['hint']}"))
            rows.append(Spacer(6.0))
        rows.append(Spacer(grow=True))
        rows.append(FooterKeys([("1-5", "choose"), ("esc", "back")]))
        return Column(rows)

    def _edit_view(self):
        node = self.nodes.get(self.edit_node) if self.edit_node is not None else None
        if node is None:
            self.mode = "canvas"
            return self._canvas_view()
        kind = KINDS[node.kind]
        rows: list = [
            AppBar(title=node.title, subtitle=kind["label"]),
            Spacer(8.0),
            Label("Title"),
            TextEdit(f"title-{node.id}", value=node.title, height=40.0,
                     placeholder=kind["label"]),
            Spacer(8.0),
            Label(kind["hint"]),
            TextEdit(f"body-{node.id}", value=node.text, multiline=True,
                     height=220.0, placeholder="…"),
        ]
        if node.status == "error":
            rows += [Spacer(8.0), Label(f"error: {node.error}")]
        elif node.output:
            out = node.output if len(node.output) <= 2000 else node.output[:2000] + "…"
            rows += [Spacer(8.0), Label("Last output"), Label(out)]
        rows += [Spacer(grow=True),
                 FooterKeys([("esc", "back to canvas"), ("⌘⏎", "apply")])]
        return Column(rows)

    # ── input: keyboard ──────────────────────────────────────────────────────
    def on_key(self, key: str, _mods: dict) -> None:
        if self.mode == "add":
            if key in tuple(str(i) for i in range(1, len(KIND_ORDER) + 1)):
                self._add_node(KIND_ORDER[int(key) - 1])
            return
        if self.mode == "edit":
            return  # TextEdit owns input; esc handled by on_escape
        if key == "a":
            self.mode = "add"
        elif key == "r":
            if not self._running:
                self._run_task = asyncio.ensure_future(self._run_graph())
        elif key == "return" and self.selected_node is not None:
            self._open_editor(self.selected_node)
        elif key == "backspace":
            self._delete_selection()
        elif key == "s":
            self._save()
            self.emit.info("flow: saved")
        self.emit.schedule_render()

    def on_escape(self) -> bool:
        if self.mode != "canvas":
            if self.mode == "edit":
                self._save()
            self.mode = "canvas"
            self.edit_node = None
            self.emit.schedule_render()
            return True
        if self.selected_node is not None or self.selected_edge is not None:
            self.selected_node = None
            self.selected_edge = None
            self.emit.schedule_render()
            return True
        return False

    # ── input: mouse ─────────────────────────────────────────────────────────
    def on_mouse_down(self, x: float, y: float, button: str, _mods: dict = {}) -> None:
        if self.mode != "canvas" or button not in ("left", "primary"):
            return
        self.mouse = (x, y)
        self._down_pos = (x, y)
        # ports first
        for n in self.nodes.values():
            ox, oy = self._canvas.out_port(n)
            if (x - ox) ** 2 + (y - oy) ** 2 <= PORT_HIT_R ** 2:
                self.drag = {"type": "wire", "node": n.id, "dir": "out"}
                return
            ix, iy = self._canvas.in_port(n)
            if (x - ix) ** 2 + (y - iy) ** 2 <= PORT_HIT_R ** 2:
                self.drag = {"type": "wire", "node": n.id, "dir": "in"}
                return
        node = self._node_at(x, y)
        if node is not None:
            self.selected_node = node.id
            self.selected_edge = None
            nsx, nsy = self._canvas.to_screen(node.x, node.y)
            self.drag = {"type": "node", "node": node.id, "grab": (x - nsx, y - nsy)}
            self.emit.schedule_render()
            return
        edge = self._edge_at(x, y)
        if edge is not None:
            self.selected_edge = edge
            self.selected_node = None
            self.emit.schedule_render()
            return
        self.drag = {"type": "pan", "start": (x, y), "origin": (self.ox, self.oy)}
        if self.selected_node is not None or self.selected_edge is not None:
            self.selected_node = None
            self.selected_edge = None
            self.emit.schedule_render()

    def on_mouse_move(self, x: float, y: float, _buttons: list,
                      _mods: dict = {}) -> None:
        self.mouse = (x, y)
        if self.drag is None:
            return
        d = self.drag
        if d["type"] == "node":
            node = self.nodes.get(d["node"])
            if node is not None:
                gx, gy = d["grab"]
                node.x, node.y = self._canvas.to_world(x - gx, y - gy)
        elif d["type"] == "pan":
            sx, sy = d["start"]
            ox0, oy0 = d["origin"]
            self.ox = ox0 + (x - sx)
            self.oy = oy0 + (y - sy)
        self.emit.schedule_render()

    def on_mouse_up(self, x: float, y: float, button: str, _mods: dict = {}) -> None:
        if self.mode != "canvas" or button not in ("left", "primary"):
            return
        d, self.drag = self.drag, None
        if d is None:
            return
        if d["type"] == "wire":
            self._finish_wire(d, x, y)
        elif d["type"] == "node":
            moved = abs(x - self._down_pos[0]) + abs(y - self._down_pos[1])
            if moved <= CLICK_SLOP:
                now = time.monotonic()
                last_t, last_id = self._last_click
                if last_id == d["node"] and now - last_t <= DOUBLE_CLICK_S:
                    self._open_editor(d["node"])
                self._last_click = (now, d["node"])
            else:
                self._save()
        self.emit.schedule_render()

    def on_scroll_delta(self, delta_y: float) -> None:
        if self.mode == "canvas":
            self.oy += delta_y
            self.emit.schedule_render()

    # ── component events (editor) ────────────────────────────────────────────
    def on_component_event(self, node_id: str, event_type: str, payload) -> None:
        if event_type not in ("change", "submit"):
            return
        try:
            field, nid_s = node_id.rsplit("-", 1)
            node = self.nodes.get(int(nid_s))
        except ValueError:
            return
        if node is None:
            return
        value = payload.get("value", "") if isinstance(payload, dict) else str(payload)
        if field == "title":
            node.title = value.strip() or KINDS[node.kind]["label"]
        elif field == "body":
            node.text = value
        if event_type == "submit":
            self._save()
            self.mode = "canvas"
            self.edit_node = None
            self.emit.schedule_render()

    # ── graph mutations ──────────────────────────────────────────────────────
    def _add_node(self, kind: str) -> None:
        cx, cy, w, h = self._canvas.rect
        wx, wy = self._canvas.to_world(cx + max(w, 400) / 2, cy + max(h, 300) / 2)
        offset = (len(self.nodes) % 5) * 24.0   # stagger stacked adds
        node = Node(self.next_id, kind,
                    wx - NODE_W / 2 + offset, wy - NODE_H / 2 + offset)
        self.nodes[node.id] = node
        self.next_id += 1
        self.selected_node = node.id
        self._save()
        self.emit.info(f"flow: added {kind} node {node.id}")
        self._open_editor(node.id)

    def _delete_selection(self) -> None:
        if self.selected_node is not None:
            nid = self.selected_node
            self.nodes.pop(nid, None)
            self.edges = [e for e in self.edges if nid not in e]
            self.selected_node = None
            self.emit.info(f"flow: deleted node {nid}")
        elif self.selected_edge is not None and 0 <= self.selected_edge < len(self.edges):
            a, b = self.edges.pop(self.selected_edge)
            self.selected_edge = None
            self.emit.info(f"flow: deleted wire {a}->{b}")
        else:
            return
        self._save()

    def _finish_wire(self, d: dict, x: float, y: float) -> None:
        target = self._node_at(x, y, port_slop=True)
        src = self.nodes.get(d["node"])
        if target is None or src is None or target.id == src.id:
            return
        a, b = (src.id, target.id) if d["dir"] == "out" else (target.id, src.id)
        if (a, b) in self.edges:
            return
        if self._creates_cycle(a, b):
            self.emit.warn(f"flow: refused wire {a}->{b} — would create a cycle")
            return
        self.edges.append((a, b))
        self._save()
        self.emit.info(f"flow: wired {a}->{b}")

    def _creates_cycle(self, a: int, b: int) -> bool:
        # adding a->b cycles iff a is reachable from b
        stack, seen = [b], set()
        while stack:
            cur = stack.pop()
            if cur == a:
                return True
            if cur in seen:
                continue
            seen.add(cur)
            stack.extend(dst for s, dst in self.edges if s == cur)
        return False

    def _open_editor(self, nid: int) -> None:
        self.edit_node = nid
        self.mode = "edit"
        self.emit.schedule_render()

    # ── hit testing ──────────────────────────────────────────────────────────
    def _node_at(self, x: float, y: float, port_slop: bool = False) -> Node | None:
        slop = PORT_HIT_R if port_slop else 0.0
        for n in reversed(list(self.nodes.values())):
            sx, sy = self._canvas.to_screen(n.x, n.y)
            if sx - slop <= x <= sx + NODE_W + slop \
                    and sy - slop <= y <= sy + NODE_H + slop:
                return n
        return None

    def _edge_at(self, x: float, y: float) -> int | None:
        for ei, (a_id, b_id) in enumerate(self.edges):
            a, b = self.nodes.get(a_id), self.nodes.get(b_id)
            if a is None or b is None:
                continue
            x1, y1 = self._canvas.out_port(a)
            x2, y2 = self._canvas.in_port(b)
            pts = _bezier_points(x1, y1, x2, y2)
            for (px, py), (qx, qy) in zip(pts, pts[1:]):
                if _dist_to_segment(x, y, px, py, qx, qy) <= EDGE_HIT_DIST:
                    return ei
        return None

    # ── execution ────────────────────────────────────────────────────────────
    def _topo_order(self) -> list[int] | None:
        indeg = {nid: 0 for nid in self.nodes}
        for _, b in self.edges:
            if b in indeg:
                indeg[b] += 1
        queue = sorted(nid for nid, d in indeg.items() if d == 0)
        order = []
        while queue:
            cur = queue.pop(0)
            order.append(cur)
            for s, dst in self.edges:
                if s == cur and dst in indeg:
                    indeg[dst] -= 1
                    if indeg[dst] == 0:
                        queue.append(dst)
        return order if len(order) == len(self.nodes) else None

    def _inputs_for(self, nid: int) -> str:
        parts = [self.nodes[a].output for a, b in self.edges
                 if b == nid and a in self.nodes]
        return "\n".join(p for p in parts if p)

    def _resolve_path(self, raw: str) -> pathlib.Path:
        p = pathlib.Path(raw.strip())
        return p if p.is_absolute() else pathlib.Path(self.workspace_root) / p

    async def _run_graph(self) -> None:
        order = self._topo_order()
        if order is None:
            self.emit.error("flow: run aborted — graph has a cycle")
            return
        self._running = True
        for n in self.nodes.values():
            n.status = "idle"
            n.output = ""
            n.error = ""
        self.emit.info(f"flow: running {len(order)} nodes")
        self.emit.schedule_render()
        try:
            for nid in order:
                node = self.nodes[nid]
                failed_upstream = any(
                    self.nodes[a].status == "error"
                    for a, b in self.edges if b == nid and a in self.nodes
                )
                if failed_upstream:
                    node.status = "error"
                    node.error = "upstream failed"
                    continue
                node.status = "running"
                self.emit.schedule_render()
                try:
                    node.output = await self._exec_node(node, self._inputs_for(nid))
                    node.status = "ok"
                    self.emit.info(f"flow: node {nid} ({node.kind}) ok "
                                   f"({len(node.output)} chars)")
                except Exception as e:
                    node.status = "error"
                    node.error = str(e)
                    self.emit.error(f"flow: node {nid} ({node.kind}) failed: {e}")
                self.emit.schedule_render()
        finally:
            self._running = False
            self.emit.schedule_render()

    async def _exec_node(self, node: Node, input_text: str) -> str:
        kind = node.kind
        if kind == "prompt":
            return node.text.replace("{{input}}", input_text)
        if kind == "ai":
            rendered = node.text.replace("{{input}}", input_text)
            # stub: a later broker call replaces this return
            return f"[ai stub — no model wired]\n{rendered}"
        if kind == "file_read":
            if not node.text.strip():
                raise ValueError("no path set")
            path = self._resolve_path(node.text)
            return await asyncio.to_thread(path.read_text)
        if kind == "file_write":
            if not node.text.strip():
                raise ValueError("no path set")
            path = self._resolve_path(node.text)

            def _write() -> None:
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(input_text)
            await asyncio.to_thread(_write)
            return input_text
        if kind == "shell":
            if not node.text.strip():
                raise ValueError("no command set")

            def _run() -> str:
                proc = subprocess.run(
                    node.text, shell=True, input=input_text,
                    capture_output=True, text=True, timeout=SHELL_TIMEOUT,
                    cwd=self.workspace_root or None,
                )
                if proc.returncode != 0:
                    raise RuntimeError(
                        (proc.stderr.strip() or f"exit {proc.returncode}")[:200])
                return proc.stdout
            return await asyncio.to_thread(_run)
        raise ValueError(f"unknown node kind: {kind}")


if __name__ == "__main__":
    FlowApp().run()
