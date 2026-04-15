#!/usr/bin/env python3
"""
pyflow — Plexi app
Visual Python node graph editor. Each node has an editable Python code body.
Run the graph to execute nodes in topological order, passing outputs as inputs.

Controls:
  Drag node header        Move node
  Click output port       Begin wiring an edge
  Click input port        Complete edge (while wiring)
  Click node body         Select node + open code panel
  Escape                  Cancel edge wiring / close panel
  Delete / Backspace      Delete selected node (when editor not focused)
  Drag empty space        Pan canvas
  Cmd+R / toolbar Run     Execute the graph
"""
from __future__ import annotations

import math
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from plexi_sdk import App
from plexi_sdk_advanced import Canvas, HitTester, DragHandler

# ---------------------------------------------------------------------------
# Catppuccin Mocha palette
# ---------------------------------------------------------------------------

C = {
    "base":     "#1e1e2e",
    "mantle":   "#181825",
    "crust":    "#11111b",
    "surface0": "#313244",
    "surface1": "#45475a",
    "surface2": "#585b70",
    "overlay0": "#6c7086",
    "overlay1": "#7f849c",
    "text":     "#cdd6f4",
    "subtext0": "#a6adc8",
    "subtext1": "#bac2de",
    "blue":     "#89b4fa",
    "lavender": "#b4befe",
    "sapphire": "#74c7ec",
    "sky":      "#89dceb",
    "teal":     "#94e2d5",
    "green":    "#a6e3a1",
    "yellow":   "#f9e2af",
    "peach":    "#fab387",
    "maroon":   "#eba0ac",
    "red":      "#f38ba8",
    "mauve":    "#cba6f7",
    "pink":     "#f5c2e7",
    "flamingo": "#f2cdcd",
}

# ---------------------------------------------------------------------------
# Layout constants
# ---------------------------------------------------------------------------

NODE_W          = 200
NODE_HEADER_H   = 32
PORT_ROW_H      = 26
PORT_RADIUS     = 6
PORT_PADDING    = 14
TOOLBAR_H       = 44
EDGE_CURVE      = 80
CODE_PANEL_W    = 360
NODE_BODY_H     = 32   # preview row below ports

# ---------------------------------------------------------------------------
# Data model
# ---------------------------------------------------------------------------


class Port:
    """One input or output port on a node."""

    def __init__(self, name: str, type_label: str, is_output: bool = False):
        self.name = name
        self.type_label = type_label
        self.is_output = is_output


class Node:
    """A single node on the canvas."""

    def __init__(self, node_id: str, title: str, inputs: list, outputs: list,
                 x: float = 100.0, y: float = 100.0, code: str = ""):
        self.id = node_id
        self.title = title
        self.inputs: list = inputs
        self.outputs: list = outputs
        self.x = x
        self.y = y
        self.code: str = code
        self.result = None   # last execution outputs dict
        self.error: str | None = None

    @property
    def height(self) -> float:
        rows = max(len(self.inputs) + len(self.outputs), 1)
        return NODE_HEADER_H + rows * PORT_ROW_H + NODE_BODY_H + 8

    def input_port_pos(self, port_index: int):
        y = self.y + NODE_HEADER_H + port_index * PORT_ROW_H + PORT_ROW_H / 2
        return (self.x + PORT_PADDING, y)

    def output_port_pos(self, port_index: int):
        base = self.y + NODE_HEADER_H + len(self.inputs) * PORT_ROW_H
        y = base + port_index * PORT_ROW_H + PORT_ROW_H / 2
        return (self.x + NODE_W - PORT_PADDING, y)

    def preview_text(self) -> tuple:
        """Returns (text, color) for the node body preview line."""
        if self.error:
            return (self.error.split("\n")[0][:40], C["red"])
        if self.result is not None:
            r = repr(self.result)
            return (r[:40] + ("…" if len(r) > 40 else ""), C["green"])
        first_line = self.code.split("\n")[0].strip()[:40] if self.code else ""
        return (first_line, C["overlay0"])


class Edge:
    """A wired connection between an output port and an input port."""

    def __init__(self, src_node_id: str, src_port_index: int,
                 dst_node_id: str, dst_port_index: int):
        self.src_node_id = src_node_id
        self.src_port_index = src_port_index
        self.dst_node_id = dst_node_id
        self.dst_port_index = dst_port_index


# ---------------------------------------------------------------------------
# Graph execution
# ---------------------------------------------------------------------------


def execute_graph(nodes: list, edges: list) -> dict:
    """Run nodes in topological order. Returns {node_id: result_or_error}."""
    import math as _math

    node_map = {n.id: n for n in nodes}
    # Build adjacency: node_id -> list of successor node_ids
    deps: dict = {n.id: set() for n in nodes}  # n.id -> set of node_ids it depends on
    for e in edges:
        deps[e.dst_node_id].add(e.src_node_id)

    # Kahn's algorithm
    in_degree = {n.id: len(deps[n.id]) for n in nodes}
    queue = [n.id for n in nodes if in_degree[n.id] == 0]
    order = []
    visited = set()
    while queue:
        nid = queue.pop(0)
        order.append(nid)
        visited.add(nid)
        # Find successors (nodes that depend on nid)
        for n in nodes:
            if nid in deps[n.id]:
                in_degree[n.id] -= 1
                if in_degree[n.id] == 0:
                    queue.append(n.id)

    results: dict = {}  # node_id -> outputs dict

    for nid in order:
        node = node_map[nid]
        # Build inputs dict from upstream edges
        inputs_dict: dict = {}
        for e in edges:
            if e.dst_node_id == nid:
                src_node = node_map.get(e.src_node_id)
                if src_node and src_node.result is not None:
                    src_port = src_node.outputs[e.src_port_index]
                    dst_port = node.inputs[e.dst_port_index]
                    # Map by destination port name
                    inputs_dict[dst_port.name] = src_node.result.get(src_port.name)

        outputs_dict: dict = {}
        try:
            local_ns: dict = {
                "inputs": inputs_dict,
                "outputs": outputs_dict,
                "math": _math,
            }
            exec(node.code, {"__builtins__": __builtins__}, local_ns)
            node.result = local_ns.get("outputs", {})
            node.error = None
        except Exception as exc:
            node.result = None
            node.error = str(exc)

        results[nid] = node.result if node.error is None else node.error

    return results


# ---------------------------------------------------------------------------
# Example graph
# ---------------------------------------------------------------------------


def make_example_nodes() -> list:
    return [
        Node(
            "load_data", "Load Data",
            inputs=[],
            outputs=[Port("data", "list")],
            x=60, y=80,
            code='outputs["data"] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]',
        ),
        Node(
            "filter", "Filter > 5",
            inputs=[Port("data", "list")],
            outputs=[Port("filtered", "list")],
            x=340, y=60,
            code='outputs["filtered"] = [x for x in inputs.get("data", []) if x > 5]',
        ),
        Node(
            "sum_node", "Sum",
            inputs=[Port("data", "list")],
            outputs=[Port("total", "int")],
            x=340, y=240,
            code='outputs["total"] = sum(inputs.get("data", []))',
        ),
        Node(
            "display", "Display",
            inputs=[Port("value", "any"), Port("label", "str")],
            outputs=[],
            x=620, y=160,
            code='label = inputs.get("label", "result")\nvalue = inputs.get("value", None)\noutputs["display"] = f"{label}: {value}"',
        ),
    ]


def make_example_edges() -> list:
    return [
        Edge("load_data", 0, "filter", 0),
        Edge("load_data", 0, "sum_node", 0),
        Edge("filter", 0, "display", 0),
    ]


# ---------------------------------------------------------------------------
# App state
# ---------------------------------------------------------------------------


class PyFlow:
    def __init__(self):
        self.canvas = Canvas(offset=(20.0, TOOLBAR_H + 10.0), scale=1.0)
        self.hit = HitTester()
        self.node_drag = DragHandler(threshold=3.0)
        self.pan_drag = DragHandler(threshold=3.0)

        self.nodes = make_example_nodes()
        self.edges = make_example_edges()

        self.selected_node_id: str | None = None

        # Edge wiring state
        self.wiring: bool = False
        self.wire_src_node_id: str | None = None
        self.wire_src_port_index: int | None = None
        self.wire_cursor_x: float = 0.0
        self.wire_cursor_y: float = 0.0

        self.dragging_node_id: str | None = None
        self.panning: bool = False

        # Code panel state
        self.show_code_panel: bool = False
        self.editor_lines: list = [""]
        self.editor_cursor_line: int = 0
        self.editor_cursor_col: int = 0

    def node_by_id(self, node_id: str) -> Node | None:
        for n in self.nodes:
            if n.id == node_id:
                return n
        return None

    def select_node(self, node_id: str | None):
        self.selected_node_id = node_id
        if node_id is not None:
            node = self.node_by_id(node_id)
            if node:
                self.editor_lines = node.code.split("\n") if node.code else [""]
                self.editor_cursor_line = 0
                self.editor_cursor_col = 0
                self.show_code_panel = True
        else:
            self.show_code_panel = False

    def sync_code_to_node(self, new_lines: list, new_cl: int, new_cc: int):
        node = self.node_by_id(self.selected_node_id) if self.selected_node_id else None
        if node:
            node.code = "\n".join(new_lines)
        self.editor_lines = new_lines
        self.editor_cursor_line = new_cl
        self.editor_cursor_col = new_cc

    def delete_selected(self):
        if self.selected_node_id is None:
            return
        nid = self.selected_node_id
        self.nodes = [n for n in self.nodes if n.id != nid]
        self.edges = [
            e for e in self.edges
            if e.src_node_id != nid and e.dst_node_id != nid
        ]
        self.selected_node_id = None
        self.show_code_panel = False

    def edge_exists(self, dst_node_id: str, dst_port_index: int) -> bool:
        for e in self.edges:
            if e.dst_node_id == dst_node_id and e.dst_port_index == dst_port_index:
                return True
        return False

    def canvas_width(self, total_w: float) -> float:
        """Canvas area width — shrinks when code panel is open."""
        if self.show_code_panel:
            return max(100.0, total_w - CODE_PANEL_W)
        return total_w


state = PyFlow()
app = App()

# ---------------------------------------------------------------------------
# Draw helpers
# ---------------------------------------------------------------------------


def draw_toolbar(ctx):
    ctx.rect(0, 0, ctx.width, TOOLBAR_H, fill=C["mantle"], radius=0)
    ctx.text(16, 13, "PyFlow", size=15, color=C["text"], bold=True)
    hint = "Click output port to wire  |  Drag nodes  |  Drag canvas to pan  |  Cmd+R to run"
    ctx.text(ctx.width / 2 - 200, 14, hint, size=11, color=C["overlay0"])

    # Run button
    btn_x = ctx.width - CODE_PANEL_W - 90 if state.show_code_panel else ctx.width - 90
    ctx.rect(btn_x, 8, 76, 28, fill=C["blue"], radius=6)
    ctx.text(btn_x + 10, 15, "▶ Run", size=13, color=C["crust"], bold=True)
    state.hit.register(("toolbar_btn", "run"), btn_x, 8, 76, 28)


def draw_bezier_edge(ctx, sx, sy, ex, ey, color, width=2.0):
    cx1 = sx + EDGE_CURVE
    cy1 = sy
    cx2 = ex - EDGE_CURVE
    cy2 = ey
    steps = 24
    prev_x, prev_y = sx, sy
    for i in range(1, steps + 1):
        t = i / steps
        t2 = t * t
        t3 = t2 * t
        mt = 1.0 - t
        mt2 = mt * mt
        mt3 = mt2 * mt
        x = mt3 * sx + 3 * mt2 * t * cx1 + 3 * mt * t2 * cx2 + t3 * ex
        y = mt3 * sy + 3 * mt2 * t * cy1 + 3 * mt * t2 * cy2 + t3 * ey
        ctx.line(prev_x, prev_y, x, y, color=color, width=width)
        prev_x, prev_y = x, y


def draw_port_circle(ctx, sx, sy, filled, color):
    r = PORT_RADIUS
    ctx.rect(sx - r, sy - r, r * 2, r * 2, fill=color, radius=r)
    if not filled:
        ir = r - 2
        ctx.rect(sx - ir, sy - ir, ir * 2, ir * 2, fill=C["surface0"], radius=ir)


def port_is_connected_output(node_id, port_index):
    return any(e.src_node_id == node_id and e.src_port_index == port_index
               for e in state.edges)


def port_is_connected_input(node_id, port_index):
    return any(e.dst_node_id == node_id and e.dst_port_index == port_index
               for e in state.edges)


def draw_node(ctx, hit, node, selected):
    sx, sy = state.canvas.canvas_to_screen(node.x, node.y)
    sw = NODE_W * state.canvas.scale
    sh = node.height * state.canvas.scale

    # Body + border
    border_color = C["blue"] if selected else C["surface1"]
    ctx.rect(sx - 2, sy - 2, sw + 4, sh + 4, fill=border_color, radius=8)
    ctx.rect(sx, sy, sw, sh, fill=C["surface0"], radius=7)

    # Header
    ctx.rect(sx, sy, sw, NODE_HEADER_H * state.canvas.scale, fill=C["surface1"], radius=7)
    ctx.text(
        sx + sw / 2 - len(node.title) * 4,
        sy + 9 * state.canvas.scale,
        node.title,
        size=13 * state.canvas.scale,
        color=C["text"],
        bold=True,
    )
    hit.register(("node", node.id), sx, sy, sw, NODE_HEADER_H * state.canvas.scale)

    # Input ports
    for i, port in enumerate(node.inputs):
        py = sy + (NODE_HEADER_H + i * PORT_ROW_H + PORT_ROW_H / 2) * state.canvas.scale
        px = sx + PORT_PADDING * state.canvas.scale
        connected = port_is_connected_input(node.id, i)
        highlight = state.wiring and not connected
        color = C["green"] if highlight else (C["blue"] if connected else C["overlay1"])
        draw_port_circle(ctx, px, py, connected, color)
        ctx.text(px + PORT_RADIUS + 4, py - 6 * state.canvas.scale,
                 port.name + ": " + port.type_label, size=11 * state.canvas.scale,
                 color=C["subtext0"])
        hit.register(("in_port", node.id, i), px - PORT_RADIUS * 2, py - PORT_RADIUS * 2,
                     PORT_RADIUS * 4, PORT_RADIUS * 4)

    # Output ports
    base_row = len(node.inputs)
    for i, port in enumerate(node.outputs):
        py = sy + (NODE_HEADER_H + (base_row + i) * PORT_ROW_H + PORT_ROW_H / 2) * state.canvas.scale
        px = sx + (NODE_W - PORT_PADDING) * state.canvas.scale
        connected = port_is_connected_output(node.id, i)
        is_src = state.wire_src_node_id == node.id and state.wire_src_port_index == i
        color = C["mauve"] if is_src else (C["blue"] if connected else C["overlay1"])
        draw_port_circle(ctx, px, py, True, color)
        label = "→ " + port.type_label
        ctx.text(px - PORT_RADIUS - len(label) * 7, py - 6 * state.canvas.scale,
                 label, size=11 * state.canvas.scale, color=C["subtext0"])
        hit.register(("out_port", node.id, i), px - PORT_RADIUS * 2, py - PORT_RADIUS * 2,
                     PORT_RADIUS * 4, PORT_RADIUS * 4)

    # Body preview row
    ports_bottom = sy + (NODE_HEADER_H + (len(node.inputs) + len(node.outputs)) * PORT_ROW_H) * state.canvas.scale
    preview_text, preview_color = node.preview_text()
    if preview_text:
        ctx.text(sx + 8, ports_bottom + 6, preview_text,
                 size=10 * state.canvas.scale, color=preview_color, monospace=True)

    # Register node body for selection (below header)
    body_y = sy + NODE_HEADER_H * state.canvas.scale
    body_h = sh - NODE_HEADER_H * state.canvas.scale
    hit.register(("node_body", node.id), sx, body_y, sw, body_h)


def draw_edges(ctx):
    for edge in state.edges:
        src = state.node_by_id(edge.src_node_id)
        dst = state.node_by_id(edge.dst_node_id)
        if src is None or dst is None:
            continue
        scx, scy = src.output_port_pos(edge.src_port_index)
        dcx, dcy = dst.input_port_pos(edge.dst_port_index)
        ssx, ssy = state.canvas.canvas_to_screen(scx, scy)
        dsx, dsy = state.canvas.canvas_to_screen(dcx, dcy)
        draw_bezier_edge(ctx, ssx, ssy, dsx, dsy, color=C["blue"], width=2.0)


def draw_wiring_preview(ctx):
    if not state.wiring:
        return
    if state.wire_src_node_id is None or state.wire_src_port_index is None:
        return
    src = state.node_by_id(state.wire_src_node_id)
    if src is None:
        return
    scx, scy = src.output_port_pos(state.wire_src_port_index)
    ssx, ssy = state.canvas.canvas_to_screen(scx, scy)
    draw_bezier_edge(ctx, ssx, ssy, state.wire_cursor_x, state.wire_cursor_y,
                     color=C["mauve"], width=1.5)


def draw_dot_grid(ctx, canvas_w):
    ox, oy = state.canvas.offset
    spacing = 32.0 * state.canvas.scale
    if spacing < 8:
        return
    start_x = ox % spacing
    start_y = (oy - TOOLBAR_H) % spacing
    x = start_x
    while x < canvas_w:
        y = TOOLBAR_H + start_y
        while y < ctx.height:
            ctx.rect(x - 1, y - 1, 2, 2, fill=C["surface1"])
            y += spacing
        x += spacing


def draw_code_panel(ctx):
    """Right-side code editor panel for the selected node."""
    if not state.show_code_panel:
        return
    node = state.node_by_id(state.selected_node_id) if state.selected_node_id else None
    if node is None:
        return

    px = ctx.width - CODE_PANEL_W
    py = TOOLBAR_H

    # Panel background
    ctx.rect(px, py, CODE_PANEL_W, ctx.height - py, fill=C["mantle"], radius=0)
    ctx.line(px, py, px, ctx.height, color=C["surface1"], width=1.0)

    # Header
    ctx.rect(px, py, CODE_PANEL_W, 36, fill=C["surface0"], radius=0)
    ctx.text(px + 12, py + 10, node.title, size=14, color=C["text"], bold=True)
    ctx.text(px + 12 + len(node.title) * 8 + 8, py + 11, "— Code", size=12, color=C["overlay0"])

    # Error / result banner
    banner_y = py + 36
    banner_h = 0
    if node.error:
        banner_h = 28
        ctx.rect(px, banner_y, CODE_PANEL_W, banner_h, fill="#3b1f2b", radius=0)
        ctx.text(px + 8, banner_y + 7, "Error: " + node.error.split("\n")[0][:46],
                 size=11, color=C["red"])
    elif node.result is not None:
        banner_h = 28
        ctx.rect(px, banner_y, CODE_PANEL_W, banner_h, fill="#1e3b2b", radius=0)
        result_text = repr(node.result)[:50]
        ctx.text(px + 8, banner_y + 7, "Result: " + result_text,
                 size=11, color=C["green"])

    # Code editor
    editor_y = banner_y + banner_h + 8
    editor_h = ctx.height - editor_y - 48  # leave room for buttons
    ctx.code_editor(
        editor_id="node_code",
        lines=state.editor_lines,
        cursor_line=state.editor_cursor_line,
        cursor_col=state.editor_cursor_col,
        on_change=state.sync_code_to_node,
        x=px + 4,
        y=editor_y,
        w=CODE_PANEL_W - 8,
        h=editor_h,
        focused=True,
    )

    # Buttons
    btn_y = ctx.height - 36
    ctx.rect(px + 8, btn_y, 100, 28, fill=C["blue"], radius=6)
    ctx.text(px + 18, btn_y + 7, "▶ Run Graph", size=12, color=C["crust"], bold=True)
    state.hit.register(("panel_btn", "run"), px + 8, btn_y, 100, 28)

    ctx.rect(px + 116, btn_y, 60, 28, fill=C["surface1"], radius=6)
    ctx.text(px + 128, btn_y + 7, "Clear", size=12, color=C["text"])
    state.hit.register(("panel_btn", "clear"), px + 116, btn_y, 60, 28)


# ---------------------------------------------------------------------------
# Render
# ---------------------------------------------------------------------------


@app.on_render
def render(ctx):
    hit = state.hit
    hit.clear()

    canvas_w = state.canvas_width(ctx.width)

    ctx.rect(0, 0, ctx.width, ctx.height, fill=C["base"])
    hit.register(("canvas_bg",), 0, TOOLBAR_H, canvas_w, ctx.height - TOOLBAR_H)

    draw_dot_grid(ctx, canvas_w)
    draw_edges(ctx)
    draw_wiring_preview(ctx)

    for node in state.nodes:
        if node.id != state.selected_node_id:
            draw_node(ctx, hit, node, False)
    if state.selected_node_id:
        sel_node = state.node_by_id(state.selected_node_id)
        if sel_node:
            draw_node(ctx, hit, sel_node, True)

    draw_code_panel(ctx)
    draw_toolbar(ctx)


# ---------------------------------------------------------------------------
# Mouse events
# ---------------------------------------------------------------------------


@app.on_mouse_down
def on_mouse_down(x, y, button, emit):
    if y < TOOLBAR_H:
        return

    region = state.hit.test(x, y)
    if region is None:
        return

    rid = region.id

    # Toolbar button
    if rid[0] == "toolbar_btn":
        if rid[1] == "run":
            execute_graph(state.nodes, state.edges)
        return

    # Panel button
    if rid[0] == "panel_btn":
        if rid[1] == "run":
            execute_graph(state.nodes, state.edges)
        elif rid[1] == "clear":
            node = state.node_by_id(state.selected_node_id) if state.selected_node_id else None
            if node:
                node.code = ""
                node.result = None
                node.error = None
                state.editor_lines = [""]
                state.editor_cursor_line = 0
                state.editor_cursor_col = 0
        return

    # Output port — start wiring
    if rid[0] == "out_port":
        _, node_id, port_idx = rid
        state.wiring = True
        state.wire_src_node_id = node_id
        state.wire_src_port_index = port_idx
        state.wire_cursor_x = x
        state.wire_cursor_y = y
        return

    # Input port — complete wiring
    if rid[0] == "in_port" and state.wiring:
        _, node_id, port_idx = rid
        if (node_id != state.wire_src_node_id
                and state.wire_src_node_id is not None
                and state.wire_src_port_index is not None):
            state.edges = [
                e for e in state.edges
                if not (e.dst_node_id == node_id and e.dst_port_index == port_idx)
            ]
            state.edges.append(Edge(
                state.wire_src_node_id, state.wire_src_port_index,
                node_id, port_idx,
            ))
        state.wiring = False
        state.wire_src_node_id = None
        state.wire_src_port_index = None
        return

    # Node header — drag/select
    if rid[0] == "node":
        _, node_id = rid
        if state.wiring:
            state.wiring = False
            state.wire_src_node_id = None
            state.wire_src_port_index = None
            return
        state.select_node(node_id)
        state.dragging_node_id = node_id
        state.node_drag.start(x, y, payload=node_id)
        return

    # Node body — select + open code panel
    if rid[0] == "node_body":
        _, node_id = rid
        if not state.wiring:
            state.select_node(node_id)
        return

    # Canvas background — pan
    if rid[0] == "canvas_bg":
        if state.wiring:
            state.wiring = False
            state.wire_src_node_id = None
            state.wire_src_port_index = None
            return
        state.select_node(None)
        state.panning = True
        state.pan_drag.start(x, y)


@app.on_mouse_up
def on_mouse_up(x, y, button, emit):
    if state.node_drag._armed:
        state.node_drag.end()
        state.dragging_node_id = None
    if state.panning:
        state.pan_drag.end()
        state.panning = False


@app.on_mouse_move
def on_mouse_move(x, y, emit):
    if state.wiring:
        state.wire_cursor_x = x
        state.wire_cursor_y = y

    if state.node_drag._armed and state.dragging_node_id:
        dx, dy = state.node_drag.update(x, y)
        if state.node_drag.active and (dx != 0 or dy != 0):
            node = state.node_by_id(state.dragging_node_id)
            if node:
                node.x += dx / state.canvas.scale
                node.y += dy / state.canvas.scale

    if state.panning and state.pan_drag._armed:
        dx, dy = state.pan_drag.update(x, y)
        if dx != 0 or dy != 0:
            ox, oy = state.canvas.offset
            state.canvas.offset = (ox + dx, oy + dy)


@app.on_scroll
def on_scroll(x, y, delta_x, delta_y, emit):
    if y < TOOLBAR_H:
        return
    ox, oy = state.canvas.offset
    state.canvas.offset = (ox - delta_x * 1.5, oy - delta_y * 1.5)


# ---------------------------------------------------------------------------
# Keyboard
# ---------------------------------------------------------------------------


@app.on_key
def on_key(key, mods, emit):
    cmd = mods.get("command", False) or mods.get("ctrl", False)
    if cmd and key == "r":
        execute_graph(state.nodes, state.edges)
        return
    if key in ("Delete", "Backspace"):
        state.delete_selected()
    elif key == "Escape":
        if state.wiring:
            state.wiring = False
            state.wire_src_node_id = None
            state.wire_src_port_index = None
        else:
            state.show_code_panel = False
            state.select_node(None)


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

app.run()
