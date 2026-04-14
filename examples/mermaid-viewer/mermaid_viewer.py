#!/usr/bin/env python3
"""
mermaid_viewer.py — Keyboard-driven mermaid diagram viewer and editor for Plexi.

Usage:
    mermaid_viewer.py [path/to/file.mmd]

When launched directly (not spawned), opens a file browser sidebar for .mmd files.
When launched with a path arg, loads and displays that diagram immediately.

Keybindings (VIEW mode):
    j/k/h/l or arrows  Navigate to nearest node
    Tab / Shift+Tab     Cycle through all nodes
    e or Enter          Edit selected node label
    a                   Add new node connected from selected
    c                   Connect mode — draw edge from selected to another node
    d                   Delete selected node + its edges
    x                   Delete an edge of selected node
    +/=                 Zoom in
    -                   Zoom out
    0                   Reset zoom and pan
    g                   Cycle layout direction (LR <-> TD)
    s or Ctrl+S         Save file
    o                   Open file browser sidebar
    Esc                 Deselect / cancel mode
"""
from __future__ import annotations

import math
import os
import sys
import time
from typing import List, Optional

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from plexi_sdk import App  # noqa: E402
from mermaid_parser import MermaidGraph, parse, serialize  # noqa: E402
from graph_layout import NodeLayout, layout as compute_layout  # noqa: E402

# ─── Colors ──────────────────────────────────────────────────────────────────

C_BG            = "#1e1e2e"
C_NODE_FILL     = "#313244"
C_NODE_BORDER   = "#45475a"
C_NODE_SEL_BG   = "#1e3a5f"
C_NODE_SEL_BR   = "#89b4fa"
C_NODE_CONN_BR  = "#a6e3a1"
C_TEXT          = "#cdd6f4"
C_EDGE          = "#6c7086"
C_EDGE_LABEL    = "#a6adc8"
C_DIAMOND_FILL  = "#2d2038"
C_CIRCLE_FILL   = "#1e3028"
C_STATUS_BG     = "#181825"
C_STATUS_TEXT   = "#a6adc8"
C_HEADER_BG     = "#181825"
C_ACCENT        = "#89b4fa"
C_DIM           = "#585b70"
C_WARN          = "#f38ba8"

HEADER_H = 36.0
STATUS_H = 28.0
NODE_W   = 140.0
NODE_H   = 48.0

# ─── App ─────────────────────────────────────────────────────────────────────

app = App()

# Mutable state dict (captured in closures)
state: dict = {
    "file_path": None,          # str | None
    "graph": None,              # MermaidGraph | None
    "node_layouts": {},         # dict[str, NodeLayout]
    "selected_node_id": None,   # str | None
    "mode": "view",             # "view" | "edit_label" | "add_node" | "connecting"
    "edit_buffer": "",          # text being edited
    "connect_from_id": None,    # source node when in connecting mode
    "connect_target_id": None,  # currently highlighted target in connecting mode
    "scroll_x": 0.0,
    "scroll_y": 0.0,
    "zoom": 1.0,
    "needs_sidebar_spawn": False,
    "status_message": "",
    "status_timer": 0.0,
    "file_mtime": None,         # float | None — last known mtime
    "edge_delete_idx": 0,       # which edge of selected node to delete with x
    "width": 800.0,
    "height": 600.0,
}


def _set_status(msg: str, duration: float = 3.0) -> None:
    state["status_message"] = msg
    state["status_timer"] = time.monotonic() + duration


def _load_file(path: str) -> None:
    """Load and parse a .mmd file into state."""
    try:
        with open(path, "r", encoding="utf-8") as f:
            source = f.read()
        state["graph"] = parse(source)
        state["file_path"] = path
        state["file_mtime"] = os.path.getmtime(path)
        _relayout()
        _set_status(f"Loaded {os.path.basename(path)}")
    except OSError as e:
        _set_status(f"Error loading file: {e}", 5.0)


def _save_file() -> None:
    """Serialize graph and save to file_path."""
    path = state["file_path"]
    graph = state["graph"]
    if not path or not graph:
        _set_status("No file to save")
        return
    try:
        source = serialize(graph)
        with open(path, "w", encoding="utf-8") as f:
            f.write(source)
        state["file_mtime"] = os.path.getmtime(path)
        _set_status(f"Saved {os.path.basename(path)}")
    except OSError as e:
        _set_status(f"Save failed: {e}", 5.0)


def _relayout() -> None:
    """Recompute node layouts from current graph and canvas size."""
    graph = state["graph"]
    if not graph:
        state["node_layouts"] = {}
        return
    w = state["width"]
    h = state["height"]
    canvas_w = w
    canvas_h = h - HEADER_H - STATUS_H
    state["node_layouts"] = compute_layout(
        graph, canvas_w, canvas_h,
        node_w=NODE_W, node_h=NODE_H, h_gap=80.0, v_gap=40.0,
    )


def _world_to_screen(x: float, y: float) -> tuple:
    """Convert world coords to screen coords accounting for scroll and zoom."""
    zoom = state["zoom"]
    sx = state["scroll_x"]
    sy = state["scroll_y"]
    return (x * zoom + sx, y * zoom + sy + HEADER_H)


def _screen_to_world(sx: float, sy: float) -> tuple:
    """Convert screen coords to world coords."""
    zoom = state["zoom"]
    ox = state["scroll_x"]
    oy = state["scroll_y"]
    return ((sx - ox) / zoom, (sy - HEADER_H - oy) / zoom)


def _node_center_screen(nl: NodeLayout) -> tuple:
    return _world_to_screen(nl.x, nl.y)


def _get_ordered_node_ids() -> List[str]:
    """Return node ids in a stable order."""
    graph = state["graph"]
    if not graph:
        return []
    return list(graph.nodes.keys())


def _select_nearest(dx: float, dy: float) -> None:
    """Move selection to the nearest node in direction (dx, dy)."""
    layouts = state["node_layouts"]
    sel = state["selected_node_id"]
    if not layouts:
        return
    if sel is None or sel not in layouts:
        ids = _get_ordered_node_ids()
        if ids:
            state["selected_node_id"] = ids[0]
        return

    cur = layouts[sel]
    cx, cy = cur.x, cur.y
    best_id = None
    best_dist = float("inf")

    for nid, nl in layouts.items():
        if nid == sel:
            continue
        # Direction filter: the candidate must be "ahead" in the requested direction
        ddx = nl.x - cx
        ddy = nl.y - cy
        dot = ddx * dx + ddy * dy
        if dot <= 0:
            continue
        dist = math.sqrt(ddx * ddx + ddy * ddy)
        if dist < best_dist:
            best_dist = dist
            best_id = nid

    if best_id:
        state["selected_node_id"] = best_id


def _cycle_selection(forward: bool = True) -> None:
    """Cycle through all nodes in order."""
    ids = _get_ordered_node_ids()
    if not ids:
        return
    sel = state["selected_node_id"]
    if sel not in ids:
        state["selected_node_id"] = ids[0]
        return
    idx = ids.index(sel)
    idx = (idx + (1 if forward else -1)) % len(ids)
    state["selected_node_id"] = ids[idx]


def _delete_selected_node() -> None:
    graph = state["graph"]
    sel = state["selected_node_id"]
    if not graph or not sel or sel not in graph.nodes:
        return
    del graph.nodes[sel]
    graph.edges = [e for e in graph.edges if e.from_id != sel and e.to_id != sel]
    state["selected_node_id"] = None
    _relayout()
    _set_status(f"Deleted node {sel}")


def _node_edges(sel: str) -> list:
    """Return edges involving the selected node."""
    graph = state["graph"]
    if not graph or not sel:
        return []
    return [e for e in graph.edges if e.from_id == sel or e.to_id == sel]


def _delete_edge_of_selected() -> None:
    graph = state["graph"]
    sel = state["selected_node_id"]
    if not graph or not sel:
        return
    edges = _node_edges(sel)
    if not edges:
        _set_status("No edges to delete")
        return
    idx = state["edge_delete_idx"] % len(edges)
    edge = edges[idx]
    graph.edges.remove(edge)
    state["edge_delete_idx"] = 0
    _set_status(f"Deleted edge {edge.from_id} -> {edge.to_id}")


def _add_node(label: str) -> None:
    """Add a new rect node connected from the currently selected node."""
    graph = state["graph"]
    if not graph:
        return
    # Generate a unique id
    existing = set(graph.nodes.keys())
    base = "".join(c for c in label[:8] if c.isalnum()) or "N"
    nid = base
    counter = 1
    while nid in existing:
        nid = f"{base}{counter}"
        counter += 1

    from mermaid_parser import MermaidNode, MermaidEdge  # noqa: E402
    graph.nodes[nid] = MermaidNode(id=nid, label=label, shape="rect")

    sel = state["selected_node_id"]
    if sel and sel in graph.nodes:
        graph.edges.append(MermaidEdge(from_id=sel, to_id=nid, label="", style="solid"))

    state["selected_node_id"] = nid
    _relayout()
    _set_status(f"Added node {nid}")


def _create_edge(from_id: str, to_id: str) -> None:
    """Create a solid edge between two nodes."""
    graph = state["graph"]
    if not graph:
        return
    from mermaid_parser import MermaidEdge  # noqa: E402
    # Avoid duplicate edges
    for e in graph.edges:
        if e.from_id == from_id and e.to_id == to_id:
            _set_status("Edge already exists")
            return
    graph.edges.append(MermaidEdge(from_id=from_id, to_id=to_id, label="", style="solid"))
    _set_status(f"Connected {from_id} -> {to_id}")


# ─── Rendering helpers ───────────────────────────────────────────────────────

def _draw_node(ctx, nl: NodeLayout, node, is_selected: bool, is_connect_target: bool) -> None:
    zoom = state["zoom"]
    sx, sy = _node_center_screen(nl)
    w = nl.w * zoom
    h = nl.h * zoom

    x0 = sx - w / 2.0
    y0 = sy - h / 2.0

    if is_selected:
        fill = C_NODE_SEL_BG
        border = C_NODE_SEL_BR
    elif is_connect_target:
        fill = "#1e3028"
        border = C_NODE_CONN_BR
    elif node.shape == "diamond":
        fill = C_DIAMOND_FILL
        border = C_NODE_BORDER
    elif node.shape == "circle":
        fill = C_CIRCLE_FILL
        border = C_NODE_BORDER
    else:
        fill = C_NODE_FILL
        border = C_NODE_BORDER

    if node.shape == "diamond":
        # Draw diamond: 4 lines connecting midpoints of bounding box
        mx = sx
        my = sy
        lx = x0
        rx = x0 + w
        ty = y0
        by = y0 + h
        # Fill with a rect (approximation) at smaller size
        ctx.rect(x0 + w * 0.15, y0 + h * 0.15, w * 0.7, h * 0.7, fill=fill, radius=0.0)
        ctx.line(mx, ty, rx, my, color=border, width=1.5)
        ctx.line(rx, my, mx, by, color=border, width=1.5)
        ctx.line(mx, by, lx, my, color=border, width=1.5)
        ctx.line(lx, my, mx, ty, color=border, width=1.5)
    elif node.shape == "circle":
        radius = min(w, h) / 2.0
        ctx.rect(x0, y0, w, h, fill=fill, radius=radius)
        # Border approximation using a slightly larger rect with no fill
        ctx.rect(x0, y0, w, h, fill=border + "00", radius=radius)
        ctx.line(x0, sy, x0 + w, sy, color=border, width=1.0)  # just draw outline lines
        ctx.rect(x0, y0, w, h, fill=fill, radius=radius)
        # Overdraw border as outline rect trick: draw 4 sides
        ctx.line(x0, y0 + radius, x0, y0 + h - radius, color=border, width=1.5)
        ctx.line(x0 + w, y0 + radius, x0 + w, y0 + h - radius, color=border, width=1.5)
        ctx.line(x0 + radius, y0, x0 + w - radius, y0, color=border, width=1.5)
        ctx.line(x0 + radius, y0 + h, x0 + w - radius, y0 + h, color=border, width=1.5)
    elif node.shape == "rounded":
        ctx.rect(x0, y0, w, h, fill=fill, radius=16.0 * zoom)
        ctx.line(x0, y0 + 16 * zoom, x0, y0 + h - 16 * zoom, color=border, width=1.5)
        ctx.line(x0 + w, y0 + 16 * zoom, x0 + w, y0 + h - 16 * zoom, color=border, width=1.5)
        ctx.line(x0 + 16 * zoom, y0, x0 + w - 16 * zoom, y0, color=border, width=1.5)
        ctx.line(x0 + 16 * zoom, y0 + h, x0 + w - 16 * zoom, y0 + h, color=border, width=1.5)
    else:
        # rect (default)
        ctx.rect(x0, y0, w, h, fill=fill, radius=8.0 * zoom)
        ctx.line(x0, y0 + 8 * zoom, x0, y0 + h - 8 * zoom, color=border, width=1.5)
        ctx.line(x0 + w, y0 + 8 * zoom, x0 + w, y0 + h - 8 * zoom, color=border, width=1.5)
        ctx.line(x0 + 8 * zoom, y0, x0 + w - 8 * zoom, y0, color=border, width=1.5)
        ctx.line(x0 + 8 * zoom, y0 + h, x0 + w - 8 * zoom, y0 + h, color=border, width=1.5)

    # Label
    if node.shape == "edit_label":
        label = state["edit_buffer"] + "|"
    else:
        label = node.label

    font_size = max(10.0, 13.0 * zoom)
    # Truncate label if too wide
    max_chars = max(4, int(w / (font_size * 0.6)))
    display = label if len(label) <= max_chars else label[: max_chars - 1] + "…"

    # Center text in node
    text_x = sx - len(display) * font_size * 0.3
    text_y = sy - font_size * 0.5
    ctx.text(text_x, text_y, display, size=font_size, color=C_TEXT)


def _draw_edge(ctx, from_nl: NodeLayout, to_nl: NodeLayout, edge) -> None:
    zoom = state["zoom"]

    # Source and dest centers
    x1, y1 = _node_center_screen(from_nl)
    x2, y2 = _node_center_screen(to_nl)

    if x1 == x2 and y1 == y2:
        return

    # Clip line to node borders (simple AABB clip)
    hw = from_nl.w * zoom / 2.0
    hh = from_nl.h * zoom / 2.0
    dx = x2 - x1
    dy = y2 - y1
    length = math.sqrt(dx * dx + dy * dy)
    if length < 1.0:
        return
    ndx = dx / length
    ndy = dy / length

    # Approximate edge start: move from center toward border
    t_from = max(hw * abs(ndx), hh * abs(ndy)) if (abs(ndx) > 0 or abs(ndy) > 0) else 0
    t_to_hw = to_nl.w * zoom / 2.0
    t_to_hh = to_nl.h * zoom / 2.0
    t_to = max(t_to_hw * abs(ndx), t_to_hh * abs(ndy)) if (abs(ndx) > 0 or abs(ndy) > 0) else 0

    sx1 = x1 + ndx * t_from
    sy1 = y1 + ndy * t_from
    sx2 = x2 - ndx * t_to
    sy2 = y2 - ndy * t_to

    color = C_EDGE
    ctx.line(sx1, sy1, sx2, sy2, color=color, width=1.5)

    # Arrowhead at destination
    arrow_size = 8.0 * zoom
    ax = sx2
    ay = sy2
    # Perpendicular
    px = -ndy
    py = ndx
    ctx.line(ax, ay, ax - ndx * arrow_size + px * arrow_size * 0.5,
             ay - ndy * arrow_size + py * arrow_size * 0.5, color=color, width=1.5)
    ctx.line(ax, ay, ax - ndx * arrow_size - px * arrow_size * 0.5,
             ay - ndy * arrow_size - py * arrow_size * 0.5, color=color, width=1.5)

    # Edge label
    if edge.label:
        mid_x = (sx1 + sx2) / 2.0
        mid_y = (sy1 + sy2) / 2.0
        font_size = max(9.0, 11.0 * zoom)
        ctx.rect(mid_x - len(edge.label) * font_size * 0.3 - 2, mid_y - font_size * 0.7,
                 len(edge.label) * font_size * 0.6 + 4, font_size + 4, fill=C_BG)
        ctx.text(mid_x - len(edge.label) * font_size * 0.3, mid_y - font_size * 0.5,
                 edge.label, size=font_size, color=C_EDGE_LABEL)


# ─── Event handlers ──────────────────────────────────────────────────────────

@app.on_render
def render(ctx) -> None:
    w = ctx.width
    h = ctx.height
    state["width"] = w
    state["height"] = h

    # Spawn sidebar on first render if needed
    if state["needs_sidebar_spawn"]:
        state["needs_sidebar_spawn"] = False
        ctx.spawn_app(
            "file-browser",
            args=["--filter=mermaid"],
            parent="self",
            layout={"kind": "cols", "slot": 0, "ratio": 0.3},
            lifecycle="cascade",
            linked=True,
        )

    # Check file mtime for hot-reload
    fp = state["file_path"]
    if fp:
        try:
            mtime = os.path.getmtime(fp)
            if state["file_mtime"] is not None and mtime != state["file_mtime"]:
                _load_file(fp)
        except OSError:
            pass

    # Clear status after timer
    if state["status_message"] and time.monotonic() > state["status_timer"]:
        state["status_message"] = ""

    # ── Background
    ctx.rect(0, 0, w, h, fill=C_BG)

    # ── Header
    ctx.rect(0, 0, w, HEADER_H, fill=C_HEADER_BG)
    title = "MERMAID VIEWER"
    filename = os.path.basename(state["file_path"]) if state["file_path"] else "No file"
    zoom_pct = f"{int(state['zoom'] * 100)}%"
    header_text = f"  {title}  [{filename}]  [zoom: {zoom_pct}]"
    ctx.text(8, HEADER_H / 2 - 7, header_text, size=13, color=C_ACCENT, bold=True)

    # ── Canvas area
    canvas_y = HEADER_H
    canvas_h = h - HEADER_H - STATUS_H

    graph = state["graph"]
    layouts = state["node_layouts"]
    mode = state["mode"]
    sel = state["selected_node_id"]
    connect_target = state["connect_target_id"]

    if not graph:
        # Empty state
        msg = "No file loaded — press O to open a .mmd file"
        ctx.text(w / 2 - len(msg) * 4, canvas_y + canvas_h / 2 - 8, msg, size=13, color=C_DIM)
    else:
        # Draw edges first (below nodes)
        for edge in graph.edges:
            from_nl = layouts.get(edge.from_id)
            to_nl = layouts.get(edge.to_id)
            if from_nl and to_nl:
                _draw_edge(ctx, from_nl, to_nl, edge)

        # Draw nodes
        for nid, nl in layouts.items():
            node = graph.nodes.get(nid)
            if not node:
                continue
            is_sel = (nid == sel)
            is_conn = (mode == "connecting" and nid == connect_target and nid != sel)

            # When editing label, show buffer in node
            if is_sel and mode == "edit_label":
                from mermaid_parser import MermaidNode as MN  # noqa: E402
                fake_node = MN(id=node.id, label=state["edit_buffer"] + "|", shape=node.shape)
                _draw_node(ctx, nl, fake_node, is_sel, is_conn)
            else:
                _draw_node(ctx, nl, node, is_sel, is_conn)

    # ── Status bar
    ctx.rect(0, h - STATUS_H, w, STATUS_H, fill=C_STATUS_BG)

    status_msg = state["status_message"]
    mode_label = {
        "view":       "VIEW",
        "edit_label": "EDIT LABEL",
        "add_node":   f"ADD NODE: {state['edit_buffer']}|",
        "connecting": "CONNECTING",
    }.get(mode, mode.upper())

    if mode == "add_node":
        hint = "Enter=confirm  Esc=cancel"
    elif mode == "edit_label":
        hint = "Enter=confirm  Esc=cancel"
    elif mode == "connecting":
        hint = "Tab=next node  Enter=connect  Esc=cancel"
    else:
        hint = "e=edit  a=add  c=connect  d=del  x=del-edge  s=save  o=open  0=reset"

    bar_text = f"  [{mode_label}]  {status_msg or hint}"
    ctx.text(8, h - STATUS_H + STATUS_H / 2 - 7, bar_text, size=11, color=C_STATUS_TEXT)


@app.on_resize
def on_resize(width, height) -> None:
    state["width"] = width
    state["height"] = height
    _relayout()


@app.on_key
def on_key(key: str, mods: dict, emit) -> None:
    mode = state["mode"]
    graph = state["graph"]
    sel = state["selected_node_id"]

    # ── EDIT_LABEL mode
    if mode == "edit_label":
        if key == "Escape":
            state["mode"] = "view"
            state["edit_buffer"] = ""
        elif key == "Enter":
            if graph and sel and sel in graph.nodes:
                new_label = state["edit_buffer"].strip()
                if new_label:
                    graph.nodes[sel].label = new_label
            state["mode"] = "view"
            state["edit_buffer"] = ""
        elif key == "Backspace":
            state["edit_buffer"] = state["edit_buffer"][:-1]
        elif len(key) == 1 and key.isprintable():
            state["edit_buffer"] += key
        return

    # ── ADD_NODE mode
    if mode == "add_node":
        if key == "Escape":
            state["mode"] = "view"
            state["edit_buffer"] = ""
        elif key == "Enter":
            label = state["edit_buffer"].strip() or "Node"
            _add_node(label)
            state["mode"] = "view"
            state["edit_buffer"] = ""
        elif key == "Backspace":
            state["edit_buffer"] = state["edit_buffer"][:-1]
        elif len(key) == 1 and key.isprintable():
            state["edit_buffer"] += key
        return

    # ── CONNECTING mode
    if mode == "connecting":
        if key == "Escape":
            state["mode"] = "view"
            state["connect_from_id"] = None
            state["connect_target_id"] = None
        elif key in ("Tab", "j", "k", "ArrowDown", "ArrowUp"):
            # Cycle through other nodes
            ids = [nid for nid in _get_ordered_node_ids() if nid != state["connect_from_id"]]
            if not ids:
                return
            ct = state["connect_target_id"]
            if ct not in ids:
                state["connect_target_id"] = ids[0]
            else:
                idx = ids.index(ct)
                forward = key in ("Tab", "j", "ArrowDown")
                idx = (idx + (1 if forward else -1)) % len(ids)
                state["connect_target_id"] = ids[idx]
        elif key == "Enter":
            from_id = state["connect_from_id"]
            to_id = state["connect_target_id"]
            if from_id and to_id and from_id != to_id:
                _create_edge(from_id, to_id)
            state["mode"] = "view"
            state["connect_from_id"] = None
            state["connect_target_id"] = None
        return

    # ── VIEW mode
    if key == "Escape":
        state["selected_node_id"] = None
        state["status_message"] = ""

    elif key in ("j", "ArrowDown"):
        _select_nearest(0.0, 1.0)
    elif key in ("k", "ArrowUp"):
        _select_nearest(0.0, -1.0)
    elif key in ("h", "ArrowLeft"):
        _select_nearest(-1.0, 0.0)
    elif key in ("l", "ArrowRight"):
        _select_nearest(1.0, 0.0)

    elif key == "Tab":
        _cycle_selection(forward=not mods.get("shift", False))

    elif key in ("e", "Enter"):
        if sel and graph and sel in graph.nodes:
            state["edit_buffer"] = graph.nodes[sel].label
            state["mode"] = "edit_label"

    elif key == "a":
        state["edit_buffer"] = ""
        state["mode"] = "add_node"

    elif key == "c":
        if sel and graph and sel in graph.nodes:
            state["connect_from_id"] = sel
            # Pick initial target
            others = [nid for nid in _get_ordered_node_ids() if nid != sel]
            state["connect_target_id"] = others[0] if others else None
            state["mode"] = "connecting"

    elif key == "d":
        _delete_selected_node()

    elif key == "x":
        _delete_edge_of_selected()

    elif key in ("+", "="):
        state["zoom"] = min(3.0, state["zoom"] + 0.1)
    elif key == "-":
        state["zoom"] = max(0.5, state["zoom"] - 0.1)
    elif key == "0":
        state["zoom"] = 1.0
        state["scroll_x"] = 0.0
        state["scroll_y"] = 0.0

    elif key == "g":
        if graph:
            graph.direction = "TD" if graph.direction == "LR" else "LR"
            _relayout()
            _set_status(f"Layout: {graph.direction}")

    elif key == "s" or (key == "s" and mods.get("command")):
        _save_file()

    elif key == "o":
        emit.spawn_app(
            "file-browser",
            args=["--filter=mermaid"],
            parent="self",
            layout={"kind": "cols", "slot": 0, "ratio": 0.3},
            lifecycle="cascade",
            linked=True,
        )


@app.on_scroll
def on_scroll(x: float, y: float, delta_x: float, delta_y: float, emit) -> None:
    state["scroll_x"] += delta_x * 2.0
    state["scroll_y"] += delta_y * 2.0


@app.on_command
def on_command(text: str, emit) -> None:
    """Handle command palette entries (e.g. file paths from file browser)."""
    text = text.strip()
    if text.endswith(".mmd") and os.path.isfile(text):
        _load_file(text)
    elif text == "save":
        _save_file()
    elif text.startswith("open "):
        path = text[5:].strip()
        if os.path.isfile(path):
            _load_file(path)


# ─── Init ────────────────────────────────────────────────────────────────────

def main() -> None:
    file_path = sys.argv[1] if len(sys.argv) > 1 else None
    launch_mode = os.environ.get("PLEXI_LAUNCH_MODE", "")

    if file_path:
        _load_file(file_path)
    elif launch_mode != "spawned":
        # Direct launch without a file: spawn sidebar on first frame
        state["needs_sidebar_spawn"] = True

    app.run()


if __name__ == "__main__":
    main()
