#!/usr/bin/env python3
"""
graph_layout.py — Layered auto-layout for MermaidGraph.

Algorithm: BFS from source nodes assigns each node a layer. Within each
layer, nodes are assigned a position index. Layers map to X (LR) or Y (TD),
positions to Y (LR) or X (TD). Output is a dict of NodeLayout objects with
pixel-space center coordinates and dimensions.
"""
from __future__ import annotations

from collections import deque
from dataclasses import dataclass
from typing import List


@dataclass
class NodeLayout:
    node_id: str
    x: float   # center x
    y: float   # center y
    w: float   # box width
    h: float   # box height


def layout(
    graph,  # MermaidGraph
    canvas_w: float,
    canvas_h: float,
    node_w: float = 140.0,
    node_h: float = 48.0,
    h_gap: float = 80.0,
    v_gap: float = 40.0,
) -> dict:
    """
    Compute NodeLayout for all nodes in the graph.

    Returns dict[str, NodeLayout].
    """
    nodes = graph.nodes
    edges = graph.edges

    if not nodes:
        return {}

    node_ids: List[str] = list(nodes.keys())

    # Build adjacency: outgoing edges per node
    outgoing: dict = {nid: [] for nid in node_ids}
    incoming: dict = {nid: [] for nid in node_ids}
    for e in edges:
        if e.from_id in outgoing and e.to_id in incoming:
            outgoing[e.from_id].append(e.to_id)
            incoming[e.to_id].append(e.from_id)

    # Find source nodes (no incoming edges)
    sources = [nid for nid in node_ids if not incoming[nid]]
    if not sources:
        # Cycle — pick first node
        sources = [node_ids[0]]

    # BFS to assign layers
    layer: dict = {}
    queue = deque()
    for src in sources:
        if src not in layer:
            layer[src] = 0
            queue.append(src)

    while queue:
        nid = queue.popleft()
        for neighbor in outgoing[nid]:
            new_layer = layer[nid] + 1
            if neighbor not in layer or layer[neighbor] < new_layer:
                layer[neighbor] = new_layer
                queue.append(neighbor)

    # Any node still not assigned (disconnected or in a cycle not reachable from sources)
    for nid in node_ids:
        if nid not in layer:
            layer[nid] = 0

    # Group nodes by layer
    max_layer = max(layer.values()) if layer else 0
    layers_map: dict = {i: [] for i in range(max_layer + 1)}
    for nid in node_ids:
        layers_map[layer[nid]].append(nid)

    num_layers = max_layer + 1
    max_per_layer = max(len(v) for v in layers_map.values()) if layers_map else 1

    # Compute total dimensions needed
    if graph.direction == "LR":
        total_w = num_layers * node_w + (num_layers - 1) * h_gap
        total_h = max_per_layer * node_h + (max_per_layer - 1) * v_gap
    else:  # TD
        total_w = max_per_layer * node_w + (max_per_layer - 1) * v_gap
        total_h = num_layers * node_h + (num_layers - 1) * h_gap

    # Center the diagram in the canvas
    origin_x = max(20.0, (canvas_w - total_w) / 2.0)
    origin_y = max(20.0, (canvas_h - total_h) / 2.0)

    result: dict = {}

    for layer_idx, layer_nodes in layers_map.items():
        count = len(layer_nodes)
        for pos_idx, nid in enumerate(layer_nodes):
            if graph.direction == "LR":
                # layer → X, position → Y
                cx = origin_x + layer_idx * (node_w + h_gap) + node_w / 2.0
                # Center the column within total_h
                col_h = count * node_h + (count - 1) * v_gap
                col_origin_y = origin_y + (total_h - col_h) / 2.0
                cy = col_origin_y + pos_idx * (node_h + v_gap) + node_h / 2.0
            else:
                # layer → Y, position → X
                cy = origin_y + layer_idx * (node_h + h_gap) + node_h / 2.0
                row_w = count * node_w + (count - 1) * v_gap
                row_origin_x = origin_x + (total_w - row_w) / 2.0
                cx = row_origin_x + pos_idx * (node_w + v_gap) + node_w / 2.0

            result[nid] = NodeLayout(
                node_id=nid,
                x=cx,
                y=cy,
                w=node_w,
                h=node_h,
            )

    return result
