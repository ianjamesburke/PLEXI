#!/usr/bin/env python3
"""
mermaid_parser.py — Parse and serialize a subset of mermaid flowchart syntax.

Supports: flowchart/graph LR/TD, node shapes (rect/circle/diamond/rounded),
edge types (solid/dashed), edge labels, comments, and subgraph headers (ignored).
"""
from __future__ import annotations

import re
from dataclasses import dataclass, field
from typing import List


@dataclass
class MermaidNode:
    id: str
    label: str   # display label
    shape: str   # "rect", "circle", "diamond", "rounded"


@dataclass
class MermaidEdge:
    from_id: str
    to_id: str
    label: str   # "" if no label
    style: str   # "solid", "dashed"


@dataclass
class MermaidGraph:
    direction: str                   # "LR" or "TD"
    nodes: dict                      # dict[str, MermaidNode]
    edges: List[MermaidEdge]
    raw_lines: List[str]             # original source lines for reference


# ─── Regex patterns ──────────────────────────────────────────────────────────

# Node definition patterns (standalone or as edge endpoint)
# A[Label], A(Rounded), A((Circle)), A{Diamond}
_RE_NODE_BRACKET   = re.compile(r'^([A-Za-z0-9_-]+)\[([^\]]*)\]$')
_RE_NODE_ROUNDED   = re.compile(r'^([A-Za-z0-9_-]+)\(([^)]*)\)$')
_RE_NODE_CIRCLE    = re.compile(r'^([A-Za-z0-9_-]+)\(\(([^)]*)\)\)$')
_RE_NODE_DIAMOND   = re.compile(r'^([A-Za-z0-9_-]+)\{([^}]*)\}$')
_RE_NODE_PLAIN     = re.compile(r'^([A-Za-z0-9_-]+)$')

# Edge patterns
# A --> B, A -- label --> B, A --label--> B, A --- B, A -.-> B
_RE_EDGE_LABELED   = re.compile(
    r'^([A-Za-z0-9_-]+(?:\[[^\]]*\]|\([^)]*\)|\(\([^)]*\)\)|\{[^}]*\})?)'
    r'\s*--([^->]+?)-->\s*'
    r'([A-Za-z0-9_-]+(?:\[[^\]]*\]|\([^)]*\)|\(\([^)]*\)\)|\{[^}]*\})?)$'
)
_RE_EDGE_ARROW     = re.compile(
    r'^([A-Za-z0-9_-]+(?:\[[^\]]*\]|\([^)]*\)|\(\([^)]*\)\)|\{[^}]*\})?)'
    r'\s*-->\s*'
    r'([A-Za-z0-9_-]+(?:\[[^\]]*\]|\([^)]*\)|\(\([^)]*\)\)|\{[^}]*\})?)$'
)
_RE_EDGE_PLAIN     = re.compile(
    r'^([A-Za-z0-9_-]+(?:\[[^\]]*\]|\([^)]*\)|\(\([^)]*\)\)|\{[^}]*\})?)'
    r'\s*---\s*'
    r'([A-Za-z0-9_-]+(?:\[[^\]]*\]|\([^)]*\)|\(\([^)]*\)\)|\{[^}]*\})?)$'
)
_RE_EDGE_DASHED    = re.compile(
    r'^([A-Za-z0-9_-]+(?:\[[^\]]*\]|\([^)]*\)|\(\([^)]*\)\)|\{[^}]*\})?)'
    r'\s*-\.->?\s*'
    r'([A-Za-z0-9_-]+(?:\[[^\]]*\]|\([^)]*\)|\(\([^)]*\)\)|\{[^}]*\})?)$'
)


def _parse_node_token(token: str) -> tuple[str, str, str]:
    """
    Parse a node token like 'A[Label]' or 'B((Circle))' or just 'C'.
    Returns (node_id, label, shape).
    """
    token = token.strip()

    m = _RE_NODE_CIRCLE.match(token)
    if m:
        return m.group(1), m.group(2), "circle"

    m = _RE_NODE_BRACKET.match(token)
    if m:
        return m.group(1), m.group(2), "rect"

    m = _RE_NODE_ROUNDED.match(token)
    if m:
        return m.group(1), m.group(2), "rounded"

    m = _RE_NODE_DIAMOND.match(token)
    if m:
        return m.group(1), m.group(2), "diamond"

    m = _RE_NODE_PLAIN.match(token)
    if m:
        node_id = m.group(1)
        return node_id, node_id, "rect"

    # Fallback: treat whole token as id
    return token, token, "rect"


def _ensure_node(nodes: dict, node_id: str, label: str, shape: str) -> None:
    """Add node if not already present; update label/shape only if it's a new node."""
    if node_id not in nodes:
        nodes[node_id] = MermaidNode(id=node_id, label=label, shape=shape)


def parse(source: str) -> MermaidGraph:
    """Parse mermaid flowchart source into a MermaidGraph."""
    raw_lines = source.splitlines()
    direction = "LR"
    nodes: dict = {}
    edges: List[MermaidEdge] = []

    for line in raw_lines:
        stripped = line.strip()

        # Skip empty lines and comments
        if not stripped or stripped.startswith("%%"):
            continue

        # Header
        if stripped.lower().startswith(("flowchart ", "graph ")):
            parts = stripped.split(None, 1)
            if len(parts) == 2:
                dir_hint = parts[1].strip().upper()
                if dir_hint in ("LR", "RL", "TD", "TB", "BT"):
                    direction = "TD" if dir_hint in ("TD", "TB") else "LR"
            continue

        # Subgraph / end — skip
        if stripped.lower().startswith("subgraph") or stripped.lower() == "end":
            continue

        # Try edge with label: A -- label --> B
        m = _RE_EDGE_LABELED.match(stripped)
        if m:
            from_tok, lbl, to_tok = m.group(1), m.group(2).strip(), m.group(3)
            fid, flabel, fshape = _parse_node_token(from_tok)
            tid, tlabel, tshape = _parse_node_token(to_tok)
            _ensure_node(nodes, fid, flabel, fshape)
            _ensure_node(nodes, tid, tlabel, tshape)
            edges.append(MermaidEdge(from_id=fid, to_id=tid, label=lbl, style="solid"))
            continue

        # Try dashed edge: A -.-> B
        m = _RE_EDGE_DASHED.match(stripped)
        if m:
            from_tok, to_tok = m.group(1), m.group(2)
            fid, flabel, fshape = _parse_node_token(from_tok)
            tid, tlabel, tshape = _parse_node_token(to_tok)
            _ensure_node(nodes, fid, flabel, fshape)
            _ensure_node(nodes, tid, tlabel, tshape)
            edges.append(MermaidEdge(from_id=fid, to_id=tid, label="", style="dashed"))
            continue

        # Try arrow edge: A --> B
        m = _RE_EDGE_ARROW.match(stripped)
        if m:
            from_tok, to_tok = m.group(1), m.group(2)
            fid, flabel, fshape = _parse_node_token(from_tok)
            tid, tlabel, tshape = _parse_node_token(to_tok)
            _ensure_node(nodes, fid, flabel, fshape)
            _ensure_node(nodes, tid, tlabel, tshape)
            edges.append(MermaidEdge(from_id=fid, to_id=tid, label="", style="solid"))
            continue

        # Try plain edge: A --- B
        m = _RE_EDGE_PLAIN.match(stripped)
        if m:
            from_tok, to_tok = m.group(1), m.group(2)
            fid, flabel, fshape = _parse_node_token(from_tok)
            tid, tlabel, tshape = _parse_node_token(to_tok)
            _ensure_node(nodes, fid, flabel, fshape)
            _ensure_node(nodes, tid, tlabel, tshape)
            edges.append(MermaidEdge(from_id=fid, to_id=tid, label="", style="solid"))
            continue

        # Try standalone node definition
        nid, nlabel, nshape = _parse_node_token(stripped)
        if nid and re.match(r'^[A-Za-z0-9_-]+$', nid):
            _ensure_node(nodes, nid, nlabel, nshape)

    return MermaidGraph(
        direction=direction,
        nodes=nodes,
        edges=edges,
        raw_lines=raw_lines,
    )


def _node_to_mermaid(node: MermaidNode) -> str:
    """Serialize a single node definition."""
    label = node.label
    nid = node.id
    if node.shape == "rect":
        return f"    {nid}[{label}]"
    elif node.shape == "rounded":
        return f"    {nid}({label})"
    elif node.shape == "circle":
        return f"    {nid}(({label}))"
    elif node.shape == "diamond":
        return f"    {nid}{{{label}}}"
    return f"    {nid}[{label}]"


def serialize(graph: MermaidGraph) -> str:
    """Serialize a MermaidGraph back to mermaid syntax."""
    lines = [f"flowchart {graph.direction}"]

    # Write node definitions (only nodes not already covered by edges)
    edge_node_ids: set = set()
    for e in graph.edges:
        edge_node_ids.add(e.from_id)
        edge_node_ids.add(e.to_id)

    for node in graph.nodes.values():
        if node.id not in edge_node_ids:
            lines.append(_node_to_mermaid(node))

    # Write edges
    for edge in graph.edges:
        from_node = graph.nodes.get(edge.from_id)
        to_node = graph.nodes.get(edge.to_id)
        from_tok = _node_to_mermaid(from_node).strip() if from_node else edge.from_id
        to_tok = _node_to_mermaid(to_node).strip() if to_node else edge.to_id

        if edge.style == "dashed":
            lines.append(f"    {from_tok} -.-> {to_tok}")
        elif edge.label:
            lines.append(f"    {from_tok} -- {edge.label} --> {to_tok}")
        else:
            lines.append(f"    {from_tok} --> {to_tok}")

    return "\n".join(lines) + "\n"
