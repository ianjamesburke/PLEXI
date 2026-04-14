"""Tests for mermaid parser — run with: python3 -m pytest examples/mermaid-viewer/tests/ -v"""
from __future__ import annotations

import sys
import os
sys.path.insert(0, os.path.join(os.path.dirname(__file__), '..'))
from mermaid_parser import parse, serialize


def test_basic_nodes():
    src = "flowchart LR\n    A[Hello] --> B[World]\n"
    g = parse(src)
    assert "A" in g.nodes
    assert "B" in g.nodes
    assert g.nodes["A"].label == "Hello"
    assert g.nodes["B"].shape == "rect"
    assert len(g.edges) == 1
    assert g.edges[0].from_id == "A"
    assert g.edges[0].to_id == "B"


def test_direction():
    g = parse("graph TD\n    X --> Y\n")
    assert g.direction == "TD"


def test_diamond_shape():
    src = "flowchart LR\n    A{Decision}\n"
    g = parse(src)
    assert g.nodes["A"].shape == "diamond"


def test_edge_label():
    src = "flowchart LR\n    A -- yes --> B\n"
    g = parse(src)
    assert g.edges[0].label == "yes"


def test_serialize_roundtrip():
    src = "flowchart LR\n    A[Hello] --> B[World]\n"
    g = parse(src)
    out = serialize(g)
    g2 = parse(out)
    assert set(g2.nodes.keys()) == set(g.nodes.keys())


def test_circle_shape():
    src = "flowchart LR\n    A((Circle))\n"
    g = parse(src)
    assert g.nodes["A"].shape == "circle"
    assert g.nodes["A"].label == "Circle"


def test_rounded_shape():
    src = "flowchart LR\n    A(Rounded)\n"
    g = parse(src)
    assert g.nodes["A"].shape == "rounded"
    assert g.nodes["A"].label == "Rounded"


def test_dashed_edge():
    src = "flowchart LR\n    A -.-> B\n"
    g = parse(src)
    assert len(g.edges) == 1
    assert g.edges[0].style == "dashed"


def test_comments_ignored():
    src = "flowchart LR\n    %% This is a comment\n    A --> B\n"
    g = parse(src)
    assert len(g.edges) == 1


def test_multiple_edges():
    src = "flowchart TD\n    A --> B\n    A --> C\n    B --> D\n"
    g = parse(src)
    assert g.direction == "TD"
    assert len(g.edges) == 3
    from_ids = [e.from_id for e in g.edges]
    assert from_ids.count("A") == 2
