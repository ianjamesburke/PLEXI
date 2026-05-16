#!/usr/bin/env python3
"""node-canvas — interactive n8n-style node graph editor (PGAP POC).

No execution engine, no persistence. Demonstrates mouse tracking, coordinate
math, and frame-rate performance with complex interactive canvas UIs.

Keys:
  n          — add node at center
  Delete / Backspace — remove selected node
  Escape     — cancel in-progress connection

Mouse:
  Left-drag node body     — move node
  Left-drag output port   — start connection; release on input port to connect
  Right-drag background   — pan canvas
"""
import math
import uuid
from dataclasses import dataclass, field

from plexi_sdk import (
    App, RenderContext,
    BG, SURFACE, FG, ACCENT, MUTED,
    CAPTION, HINT,
    GREEN,
)

# ── Design constants ──────────────────────────────────────────────────────────

NODE_W        = 160.0
NODE_HEADER_H = 28.0
PORT_H        = 22.0
PORT_R        = 6.0
PORT_LABEL_X  = 10.0
GRID_STEP     = 40.0

C_NODE_BG     = SURFACE
C_NODE_BORDER = "#3a3a5c"
C_NODE_SEL    = ACCENT
C_HEADER_BG   = "#2a2a4a"
C_PORT_IN     = GREEN
C_PORT_OUT    = ACCENT
C_CONN        = "#8888cc"
C_CONN_LIVE   = "#ccccff"
C_GRID        = "#1e1e2e"
C_GRID_DOT    = "#2a2a40"


# ── Data model ────────────────────────────────────────────────────────────────

@dataclass
class Port:
    id:   str
    name: str


@dataclass
class Node:
    id:      str
    x:       float
    y:       float
    title:   str
    inputs:  list[Port] = field(default_factory=list)
    outputs: list[Port] = field(default_factory=list)

    @property
    def w(self) -> float:
        return NODE_W

    @property
    def h(self) -> float:
        return NODE_HEADER_H + max(len(self.inputs), len(self.outputs)) * PORT_H + 8.0


@dataclass
class Connection:
    from_node: str
    from_port: str
    to_node:   str
    to_port:   str


# ── Seed data ─────────────────────────────────────────────────────────────────

def _make_node(title: str, x: float, y: float,
               inputs: list[str], outputs: list[str]) -> Node:
    return Node(
        id=str(uuid.uuid4())[:8],
        x=x, y=y, title=title,
        inputs=[Port(id=str(uuid.uuid4())[:8], name=n) for n in inputs],
        outputs=[Port(id=str(uuid.uuid4())[:8], name=n) for n in outputs],
    )


def _seed() -> tuple[list[Node], list[Connection]]:
    trigger = _make_node("HTTP Trigger", 60,  80,  [],              ["response"])
    parse   = _make_node("Parse JSON",  280, 80,  ["body"],         ["data", "error"])
    filter_ = _make_node("Filter",      500, 40,  ["items"],        ["pass", "fail"])
    send    = _make_node("Send Email",  500, 180, ["payload"],       ["sent"])
    log_    = _make_node("Log",         720, 100, ["msg"],           [])

    nodes = [trigger, parse, filter_, send, log_]
    conns = [
        Connection(trigger.id, trigger.outputs[0].id, parse.id, parse.inputs[0].id),
        Connection(parse.id,   parse.outputs[0].id,   filter_.id, filter_.inputs[0].id),
        Connection(parse.id,   parse.outputs[0].id,   send.id,    send.inputs[0].id),
        Connection(filter_.id, filter_.outputs[0].id, log_.id,    log_.inputs[0].id),
    ]
    return nodes, conns


# ── App ───────────────────────────────────────────────────────────────────────

class NodeCanvas(App):
    async def on_init(self, ctx: RenderContext) -> None:
        self._nodes, self._conns = _seed()
        self._cam_x = 0.0
        self._cam_y = 0.0
        self._selected: str | None = None

        # interaction state
        self._drag_node:   tuple[str, float, float] | None = None  # (id, off_x, off_y)
        self._drag_conn:   tuple[str, str, float, float] | None = None  # (from_node, from_port, mx, my)
        self._pan_anchor:  tuple[float, float, float, float] | None = None  # (mx, my, cam_x, cam_y)

        ctx.status_summary("Node Canvas — n:add  Del:remove  drag:connect/pan")
        self.emit.info(f"node-canvas init: {len(self._nodes)} nodes seeded")

    # ── Coordinate helpers ────────────────────────────────────────────────────

    def _w2s(self, wx: float, wy: float) -> tuple[float, float]:
        return wx + self._cam_x, wy + self._cam_y

    def _s2w(self, sx: float, sy: float) -> tuple[float, float]:
        return sx - self._cam_x, sy - self._cam_y

    def _port_screen_pos(self, node: Node, port: Port, is_output: bool) -> tuple[float, float]:
        ports = node.outputs if is_output else node.inputs
        idx = next((i for i, p in enumerate(ports) if p.id == port.id), 0)
        sx, sy = self._w2s(node.x, node.y)
        px = sx + node.w if is_output else sx
        py = sy + NODE_HEADER_H + idx * PORT_H + PORT_H / 2
        return px, py

    def _node_by_id(self, nid: str) -> Node | None:
        return next((n for n in self._nodes if n.id == nid), None)

    # ── Hit testing ───────────────────────────────────────────────────────────

    def _port_at(self, sx: float, sy: float) -> tuple[str, str, bool] | None:
        """Return (node_id, port_id, is_output) or None."""
        for node in self._nodes:
            for port in node.outputs:
                px, py = self._port_screen_pos(node, port, True)
                if math.hypot(sx - px, sy - py) <= PORT_R * 2:
                    return node.id, port.id, True
            for port in node.inputs:
                px, py = self._port_screen_pos(node, port, False)
                if math.hypot(sx - px, sy - py) <= PORT_R * 2:
                    return node.id, port.id, False
        return None

    def _node_at(self, sx: float, sy: float) -> str | None:
        for node in reversed(self._nodes):
            nx, ny = self._w2s(node.x, node.y)
            if nx <= sx <= nx + node.w and ny <= sy <= ny + node.h:
                return node.id
        return None

    # ── Rendering ─────────────────────────────────────────────────────────────

    def on_render(self, ctx: RenderContext) -> None:
        ctx.clear(BG)
        self._draw_grid(ctx)
        self._draw_connections(ctx)
        self._draw_live_connection(ctx)
        self._draw_nodes(ctx)
        self._draw_hint(ctx)

    def _draw_grid(self, ctx: RenderContext) -> None:
        off_x = self._cam_x % GRID_STEP
        off_y = self._cam_y % GRID_STEP
        x = off_x
        while x < ctx.w:
            y = off_y
            while y < ctx.h:
                ctx.circle(x, y, 1.5, C_GRID_DOT)
                y += GRID_STEP
            x += GRID_STEP

    def _draw_connections(self, ctx: RenderContext) -> None:
        for conn in self._conns:
            fn = self._node_by_id(conn.from_node)
            tn = self._node_by_id(conn.to_node)
            if not fn or not tn:
                continue
            fp = next((p for p in fn.outputs if p.id == conn.from_port), None)
            tp = next((p for p in tn.inputs  if p.id == conn.to_port),   None)
            if not fp or not tp:
                continue
            x1, y1 = self._port_screen_pos(fn, fp, True)
            x2, y2 = self._port_screen_pos(tn, tp, False)
            ctx.line(x1, y1, x2, y2, C_CONN, width=2.0)

    def _draw_live_connection(self, ctx: RenderContext) -> None:
        if not self._drag_conn:
            return
        from_node_id, from_port_id, mx, my = self._drag_conn
        fn = self._node_by_id(from_node_id)
        if not fn:
            return
        fp = next((p for p in fn.outputs if p.id == from_port_id), None)
        if not fp:
            return
        x1, y1 = self._port_screen_pos(fn, fp, True)
        ctx.line(x1, y1, mx, my, C_CONN_LIVE, width=2.0)
        ctx.circle(mx, my, PORT_R, C_CONN_LIVE)

    def _draw_nodes(self, ctx: RenderContext) -> None:
        for node in self._nodes:
            self._draw_node(ctx, node)

    def _draw_node(self, ctx: RenderContext, node: Node) -> None:
        sx, sy = self._w2s(node.x, node.y)
        selected = node.id == self._selected
        border = C_NODE_SEL if selected else C_NODE_BORDER

        # shadow
        ctx.rect(sx + 3, sy + 3, node.w, node.h, "#00000044", radius=6.0)
        # body
        ctx.rect(sx, sy, node.w, node.h, C_NODE_BG, radius=6.0)
        # header
        ctx.rect(sx, sy, node.w, NODE_HEADER_H, C_HEADER_BG, radius=6.0)
        ctx.rect(sx, sy + NODE_HEADER_H - 4, node.w, 4, C_HEADER_BG)  # square bottom of header
        # border
        ctx.rect(sx, sy, node.w, 2, border)
        ctx.rect(sx, sy + node.h - 2, node.w, 2, border)
        ctx.rect(sx, sy, 2, node.h, border)
        ctx.rect(sx + node.w - 2, sy, 2, node.h, border)
        ctx.rect(sx + 2, sy + 2, node.w - 4, node.h - 4, "#00000000", radius=4.0)

        # title
        ctx.text(sx + PORT_LABEL_X, sy + (NODE_HEADER_H - CAPTION) / 2,
                 node.title, size=CAPTION, color=FG, bold=True,
                 max_width=node.w - PORT_LABEL_X * 2, elide=True)

        # input ports
        for i, port in enumerate(node.inputs):
            py = sy + NODE_HEADER_H + i * PORT_H + PORT_H / 2
            ctx.circle(sx, py, PORT_R, C_PORT_IN)
            ctx.text(sx + PORT_R + 4, py - HINT / 2, port.name,
                     size=HINT, color=MUTED, max_width=node.w / 2 - PORT_R - 8, elide=True)

        # output ports
        for i, port in enumerate(node.outputs):
            py = sy + NODE_HEADER_H + i * PORT_H + PORT_H / 2
            ctx.circle(sx + node.w, py, PORT_R, C_PORT_OUT)
            ctx.text(sx + node.w - PORT_R - 4, py - HINT / 2, port.name,
                     size=HINT, color=MUTED, align="right",
                     max_width=node.w / 2 - PORT_R - 8, elide=True)

    def _draw_hint(self, ctx: RenderContext) -> None:
        hints = "n: add node   Del: remove   drag output→input: connect   right-drag: pan"
        ctx.text(8, ctx.h - HINT - 6, hints, size=HINT, color=MUTED)

    # ── Mouse events ──────────────────────────────────────────────────────────

    def on_mouse_down(self, _ctx: RenderContext, x: float, y: float, button: str) -> None:
        if button == "secondary":
            self._pan_anchor = (x, y, self._cam_x, self._cam_y)
            self.emit.info("node-canvas: pan start")
            return

        if button != "primary":
            return

        # port hit first
        hit_port = self._port_at(x, y)
        if hit_port:
            node_id, port_id, is_output = hit_port
            if is_output:
                self._drag_conn = (node_id, port_id, x, y)
                self.emit.info(f"node-canvas: connection drag start from {node_id}:{port_id}")
                return

        # then node body
        hit_node = self._node_at(x, y)
        if hit_node:
            self._selected = hit_node
            node = self._node_by_id(hit_node)
            if node:
                sx, sy = self._w2s(node.x, node.y)
                self._drag_node = (hit_node, x - sx, y - sy)
                self.emit.info(f"node-canvas: drag node start {hit_node}")
        else:
            self._selected = None

    def on_mouse_move(self, _ctx: RenderContext, x: float, y: float, _buttons: list) -> None:
        if self._pan_anchor:
            ax, ay, cam_x, cam_y = self._pan_anchor
            self._cam_x = cam_x + (x - ax)
            self._cam_y = cam_y + (y - ay)
            self.emit.schedule_render()
            return

        if self._drag_node:
            nid, off_x, off_y = self._drag_node
            node = self._node_by_id(nid)
            if node:
                wx, wy = self._s2w(x - off_x, y - off_y)
                node.x = wx
                node.y = wy
                self.emit.schedule_render()

        if self._drag_conn:
            fn, fp, _, _ = self._drag_conn
            self._drag_conn = (fn, fp, x, y)
            self.emit.schedule_render()

    def on_mouse_up(self, _ctx: RenderContext, x: float, y: float, button: str) -> None:
        if button == "secondary":
            if self._pan_anchor:
                self.emit.info("node-canvas: pan stop")
            self._pan_anchor = None
            return

        if button != "primary":
            return

        if self._drag_conn:
            from_node_id, from_port_id, _, _ = self._drag_conn
            self._drag_conn = None

            hit = self._port_at(x, y)
            if hit:
                to_node_id, to_port_id, is_output = hit
                if not is_output and to_node_id != from_node_id:
                    already = any(
                        c.from_port == from_port_id and c.to_port == to_port_id
                        for c in self._conns
                    )
                    if not already:
                        self._conns.append(Connection(
                            from_node=from_node_id, from_port=from_port_id,
                            to_node=to_node_id,   to_port=to_port_id,
                        ))
                        self.emit.info(
                            f"node-canvas: connection created "
                            f"{from_node_id}:{from_port_id} → {to_node_id}:{to_port_id}"
                        )
            else:
                self.emit.info("node-canvas: connection drag cancelled (no target port)")

            self.emit.schedule_render()

        if self._drag_node:
            self.emit.info(f"node-canvas: drag node end {self._drag_node[0]}")
            self._drag_node = None

    # ── Key events ────────────────────────────────────────────────────────────

    def on_key(self, ctx: RenderContext, key: str, _mods: dict) -> None:
        if key == "n":
            cx, cy = self._s2w(ctx.w / 2, ctx.h / 2)
            node = _make_node(
                f"Node {len(self._nodes) + 1}",
                cx - NODE_W / 2, cy - 40,
                ["input"], ["output"],
            )
            self._nodes.append(node)
            self._selected = node.id
            self.emit.info(f"node-canvas: node added id={node.id} pos=({node.x:.0f},{node.y:.0f})")
            self.emit.schedule_render()

        elif key in ("delete", "backspace"):
            if self._selected:
                nid = self._selected
                self._nodes = [n for n in self._nodes if n.id != nid]
                self._conns = [c for c in self._conns
                               if c.from_node != nid and c.to_node != nid]
                self._selected = None
                self.emit.info(f"node-canvas: node removed id={nid}")
                self.emit.schedule_render()

        elif key == "escape":
            if self._drag_conn:
                self._drag_conn = None
                self.emit.info("node-canvas: connection drag cancelled via Escape")
                self.emit.schedule_render()


NodeCanvas().run()
