#!/usr/bin/env python3
"""
pyflow — Plexi app
Visual node graph editor. Drag nodes, wire ports, pan the canvas.

Controls:
  Drag node header        Move node
  Click output port       Begin wiring an edge
  Click input port        Complete edge (while wiring)
  Escape                  Cancel edge wiring
  Delete / Backspace      Delete selected node
  Drag empty space        Pan canvas
  Scroll                  Pan canvas
"""

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

NODE_W        = 200
NODE_HEADER_H = 32
PORT_ROW_H    = 26
PORT_RADIUS   = 6
PORT_PADDING  = 14   # x distance from node edge to port centre
TOOLBAR_H     = 44
EDGE_CURVE    = 80   # bezier handle length

# ---------------------------------------------------------------------------
# Data model
# ---------------------------------------------------------------------------


class Port:
    """Represents one input or output port on a node."""

    def __init__(self, name, type_label, is_output=False):
        self.name = name
        self.type_label = type_label
        self.is_output = is_output


class Node:
    """A single node on the canvas."""

    def __init__(self, node_id, title, inputs, outputs, x=100.0, y=100.0):
        self.id = node_id
        self.title = title
        self.inputs = inputs    # list of Port
        self.outputs = outputs  # list of Port
        self.x = x
        self.y = y

    @property
    def height(self):
        """Total rendered height of the node."""
        rows = max(len(self.inputs) + len(self.outputs), 1)
        return NODE_HEADER_H + rows * PORT_ROW_H + 8

    def input_port_pos(self, port_index):
        """Canvas-space centre of an input port."""
        y = self.y + NODE_HEADER_H + port_index * PORT_ROW_H + PORT_ROW_H / 2
        return (self.x + PORT_PADDING, y)

    def output_port_pos(self, port_index):
        """Canvas-space centre of an output port."""
        base = self.y + NODE_HEADER_H + len(self.inputs) * PORT_ROW_H
        y = base + port_index * PORT_ROW_H + PORT_ROW_H / 2
        return (self.x + NODE_W - PORT_PADDING, y)


class Edge:
    """A wired connection between an output port and an input port."""

    def __init__(self, src_node_id, src_port_index, dst_node_id, dst_port_index):
        self.src_node_id = src_node_id
        self.src_port_index = src_port_index
        self.dst_node_id = dst_node_id
        self.dst_port_index = dst_port_index


# ---------------------------------------------------------------------------
# Hard-coded example graph
# ---------------------------------------------------------------------------


def make_example_nodes():
    return [
        Node(
            "load_image", "Load Image",
            inputs=[],
            outputs=[Port("image", "Image")],
            x=60, y=80,
        ),
        Node(
            "blur", "Blur",
            inputs=[Port("image", "Image"), Port("radius", "float")],
            outputs=[Port("image", "Image")],
            x=340, y=60,
        ),
        Node(
            "threshold", "Threshold",
            inputs=[Port("image", "Image"), Port("value", "float")],
            outputs=[Port("mask", "Mask")],
            x=340, y=240,
        ),
        Node(
            "save", "Save",
            inputs=[Port("image", "Image"), Port("path", "str")],
            outputs=[],
            x=620, y=140,
        ),
    ]


def make_example_edges():
    return [
        Edge("load_image", 0, "blur", 0),
        Edge("blur", 0, "save", 0),
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

        self.selected_node_id = None

        # Edge wiring state
        self.wiring = False            # True while dragging a new edge
        self.wire_src_node_id = None
        self.wire_src_port_index = None
        self.wire_cursor_x = 0.0      # screen coords of cursor tip
        self.wire_cursor_y = 0.0

        # Which node is currently being dragged
        self.dragging_node_id = None

        # Pan drag tracking
        self.panning = False

    def node_by_id(self, node_id):
        for n in self.nodes:
            if n.id == node_id:
                return n
        return None

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

    def edge_exists(self, dst_node_id, dst_port_index):
        """Check if an input port already has an edge."""
        for e in self.edges:
            if e.dst_node_id == dst_node_id and e.dst_port_index == dst_port_index:
                return True
        return False


state = PyFlow()
app = App(app_id="pyflow")

# ---------------------------------------------------------------------------
# Hit-region ID helpers
# ---------------------------------------------------------------------------

# IDs are tuples so we can pattern-match on type easily.
# ("node", node_id)
# ("out_port", node_id, port_index)
# ("in_port", node_id, port_index)
# ("canvas_bg",)
# ("toolbar_btn", btn_name)


# ---------------------------------------------------------------------------
# Draw helpers
# ---------------------------------------------------------------------------


def draw_toolbar(ctx):
    """Fixed toolbar at top — not affected by canvas pan/zoom."""
    ctx.rect(0, 0, ctx.width, TOOLBAR_H, fill=C["mantle"], radius=0)

    # Title
    ctx.text(16, 13, "PyFlow", size=15, color=C["text"], bold=True)

    # Hint text
    hint = "Click output port to wire  |  Drag nodes  |  Drag canvas to pan"
    ctx.text(ctx.width / 2 - 160, 14, hint, size=11, color=C["overlay0"])


def draw_bezier_edge(ctx, sx, sy, ex, ey, color, width=2.0):
    """Approximate a cubic bezier with line segments (no bezier draw cmd yet)."""
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
    """Draw a filled or hollow port indicator using a small rect as circle proxy."""
    r = PORT_RADIUS
    ctx.rect(sx - r, sy - r, r * 2, r * 2, fill=color, radius=r)
    if not filled:
        # Punch an inner hole with the node body color
        ir = r - 2
        ctx.rect(sx - ir, sy - ir, ir * 2, ir * 2, fill=C["surface0"], radius=ir)


def port_is_connected_output(state, node_id, port_index):
    for e in state.edges:
        if e.src_node_id == node_id and e.src_port_index == port_index:
            return True
    return False


def port_is_connected_input(state, node_id, port_index):
    for e in state.edges:
        if e.dst_node_id == node_id and e.dst_port_index == port_index:
            return True
    return False


def draw_node(ctx, hit, node, selected, wiring_src_node, wiring_src_port):
    sx, sy = state.canvas.canvas_to_screen(node.x, node.y)
    sw = NODE_W * state.canvas.scale
    sh = node.height * state.canvas.scale

    # Body
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

    # Register header as draggable / selectable
    hit.register(("node", node.id), sx, sy, sw, NODE_HEADER_H * state.canvas.scale)

    # Input ports
    for i, port in enumerate(node.inputs):
        py = sy + (NODE_HEADER_H + i * PORT_ROW_H + PORT_ROW_H / 2) * state.canvas.scale
        px = sx + PORT_PADDING * state.canvas.scale
        connected = port_is_connected_input(state, node.id, i)
        # Highlight if we are wiring and this is a valid target
        highlight = state.wiring and not connected
        color = C["green"] if highlight else (C["blue"] if connected else C["overlay1"])
        draw_port_circle(ctx, px, py, connected, color)
        ctx.text(
            px + PORT_RADIUS + 4,
            py - 6 * state.canvas.scale,
            port.name + ": " + port.type_label,
            size=11 * state.canvas.scale,
            color=C["subtext0"],
        )
        hit.register(("in_port", node.id, i), px - PORT_RADIUS * 2, py - PORT_RADIUS * 2,
                     PORT_RADIUS * 4, PORT_RADIUS * 4)

    # Output ports
    base_row = len(node.inputs)
    for i, port in enumerate(node.outputs):
        py = sy + (NODE_HEADER_H + (base_row + i) * PORT_ROW_H + PORT_ROW_H / 2) * state.canvas.scale
        px = sx + (NODE_W - PORT_PADDING) * state.canvas.scale
        connected = port_is_connected_output(state, node.id, i)
        # Dim if we are currently wiring from this port
        is_src = wiring_src_node == node.id and wiring_src_port == i
        color = C["mauve"] if is_src else (C["blue"] if connected else C["overlay1"])
        draw_port_circle(ctx, px, py, True, color)
        label = "→ " + port.type_label
        ctx.text(
            px - PORT_RADIUS - len(label) * 7,
            py - 6 * state.canvas.scale,
            label,
            size=11 * state.canvas.scale,
            color=C["subtext0"],
        )
        hit.register(("out_port", node.id, i), px - PORT_RADIUS * 2, py - PORT_RADIUS * 2,
                     PORT_RADIUS * 4, PORT_RADIUS * 4)


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
    """Draw the in-progress edge while user is wiring."""
    if not state.wiring:
        return
    src = state.node_by_id(state.wire_src_node_id)
    if src is None:
        return
    scx, scy = src.output_port_pos(state.wire_src_port_index)
    ssx, ssy = state.canvas.canvas_to_screen(scx, scy)
    draw_bezier_edge(ctx, ssx, ssy, state.wire_cursor_x, state.wire_cursor_y,
                     color=C["mauve"], width=1.5)


# ---------------------------------------------------------------------------
# Render
# ---------------------------------------------------------------------------


@app.on_render
def render(ctx):
    hit = state.hit
    hit.clear()

    # Background
    ctx.rect(0, 0, ctx.width, ctx.height, fill=C["base"])

    # Register canvas background (below toolbar)
    hit.register(("canvas_bg",), 0, TOOLBAR_H, ctx.width, ctx.height - TOOLBAR_H)

    # Dot grid
    draw_dot_grid(ctx)

    # Edges
    draw_edges(ctx)

    # Wiring preview
    draw_wiring_preview(ctx)

    # Nodes (draw selected last so it renders on top)
    for node in state.nodes:
        if node.id != state.selected_node_id:
            draw_node(ctx, hit, node, False,
                      state.wire_src_node_id, state.wire_src_port_index)
    if state.selected_node_id:
        sel_node = state.node_by_id(state.selected_node_id)
        if sel_node:
            draw_node(ctx, hit, sel_node, True,
                      state.wire_src_node_id, state.wire_src_port_index)

    # Toolbar (drawn last — on top of canvas)
    draw_toolbar(ctx)

    # Cursor
    if state.wiring or state.node_drag.active:
        ctx.set_cursor("grabbing")
    elif state.panning:
        ctx.set_cursor("grabbing")


def draw_dot_grid(ctx):
    """Faint dot grid that moves with the canvas pan."""
    ox, oy = state.canvas.offset
    spacing = 32.0 * state.canvas.scale
    if spacing < 8:
        return

    # Compute first dot position mod spacing
    start_x = ox % spacing
    start_y = (oy - TOOLBAR_H) % spacing

    x = start_x
    while x < ctx.width:
        y = TOOLBAR_H + start_y
        while y < ctx.height:
            # Draw a tiny 2x2 rect as a dot
            ctx.rect(x - 1, y - 1, 2, 2, fill=C["surface1"])
            y += spacing
        x += spacing


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

    # Output port — start wiring
    if rid[0] == "out_port":
        _, node_id, port_idx = rid
        state.wiring = True
        state.wire_src_node_id = node_id
        state.wire_src_port_index = port_idx
        state.wire_cursor_x = x
        state.wire_cursor_y = y
        return

    # Input port — complete wiring if active
    if rid[0] == "in_port" and state.wiring:
        _, node_id, port_idx = rid
        # Don't connect to self
        if node_id != state.wire_src_node_id:
            # Remove existing edge to this input (one-in rule)
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

    # Node header — start drag / select
    if rid[0] == "node":
        _, node_id = rid
        if state.wiring:
            # Cancel wiring on click elsewhere
            state.wiring = False
            state.wire_src_node_id = None
            state.wire_src_port_index = None
            return
        state.selected_node_id = node_id
        state.dragging_node_id = node_id
        state.node_drag.start(x, y, payload=node_id)
        return

    # Canvas background — start pan
    if rid[0] == "canvas_bg":
        if state.wiring:
            state.wiring = False
            state.wire_src_node_id = None
            state.wire_src_port_index = None
            return
        state.selected_node_id = None
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
    # Update wiring cursor
    if state.wiring:
        state.wire_cursor_x = x
        state.wire_cursor_y = y

    # Node drag
    if state.node_drag._armed and state.dragging_node_id:
        dx, dy = state.node_drag.update(x, y)
        if state.node_drag.active and (dx != 0 or dy != 0):
            node = state.node_by_id(state.dragging_node_id)
            if node:
                # Convert screen delta to canvas delta
                node.x += dx / state.canvas.scale
                node.y += dy / state.canvas.scale

    # Canvas pan
    if state.panning and state.pan_drag._armed:
        dx, dy = state.pan_drag.update(x, y)
        if dx != 0 or dy != 0:
            ox, oy = state.canvas.offset
            state.canvas.offset = (ox + dx, oy + dy)


@app.on_scroll
def on_scroll(x, y, delta_x, delta_y, emit):
    # Pan the canvas with scroll
    if y < TOOLBAR_H:
        return
    ox, oy = state.canvas.offset
    state.canvas.offset = (ox - delta_x * 1.5, oy - delta_y * 1.5)


# ---------------------------------------------------------------------------
# Keyboard
# ---------------------------------------------------------------------------


@app.on_key
def on_key(key, mods, emit):
    if key in ("Delete", "Backspace"):
        state.delete_selected()
    elif key == "Escape":
        if state.wiring:
            state.wiring = False
            state.wire_src_node_id = None
            state.wire_src_port_index = None
        else:
            state.selected_node_id = None


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

app.run()
