"""Tests for ctx.render_tree() — epic #1897 B3."""

import json
from unittest.mock import MagicMock

import pytest

from plexi_sdk._render_context import RenderContext
from plexi_sdk.ui import Clickable, Tabs


# ── helpers ────────────────────────────────────────────────────────────────────


def _make_ctx() -> RenderContext:
    """Minimal RenderContext backed by a MagicMock app."""
    app = MagicMock()
    app._mx = 0.0
    app._my = 0.0
    app._btn_active_until = {}
    ctx = RenderContext(
        frame_id=0,
        rect={"x": 0.0, "y": 0.0, "w": 400.0, "h": 300.0},
        workspace_root="",
        capabilities=[],
        feature_flags=[],
        app=app,
        elapsed=0.0,
        clicks=[],
    )
    return ctx


def _buffered(ctx: RenderContext) -> list[dict]:
    """Return all buffered draw commands as parsed dicts (excludes frame_done)."""
    return [json.loads(line) for line in ctx._buf]


# ── tests ──────────────────────────────────────────────────────────────────────


def test_render_tree_with_dict() -> None:
    """render_tree(dict) queues a component_tree command wrapping the dict."""
    ctx = _make_ctx()
    node = {"type": "text", "text": "hi", "size": 14.0, "color": "#ffffff"}
    ctx.render_tree(node)

    cmds = _buffered(ctx)
    assert len(cmds) == 1
    cmd = cmds[0]
    assert cmd["type"] == "component_tree"
    assert cmd["root"]["type"] == "text"
    assert cmd["root"]["text"] == "hi"


def test_render_tree_with_has_to_node() -> None:
    """render_tree(HasToNode) calls to_node() and emits the result."""
    class _TextNode:
        def to_node(self) -> dict:
            return {"type": "text", "text": "from to_node", "size": 12.0, "color": "#cdd6f4"}

    ctx = _make_ctx()
    ctx.render_tree(_TextNode())

    cmds = _buffered(ctx)
    assert len(cmds) == 1
    cmd = cmds[0]
    assert cmd["type"] == "component_tree"
    assert cmd["root"]["text"] == "from to_node"


def test_render_tree_with_clickable_component() -> None:
    """render_tree accepts a Clickable (HasToNode from B2 components)."""
    inner = MagicMock()
    inner.to_node.return_value = {"type": "text", "text": "click me", "size": 14.0, "color": "#cdd6f4"}
    clickable = Clickable(node_id="btn-1", child=inner, on_click=True)

    ctx = _make_ctx()
    ctx.render_tree(clickable)

    cmds = _buffered(ctx)
    assert len(cmds) == 1
    cmd = cmds[0]
    assert cmd["type"] == "component_tree"
    assert cmd["root"]["type"] == "interactive"
    assert cmd["root"]["node_id"] == "btn-1"


def test_render_tree_rejects_invalid_type() -> None:
    """render_tree raises TypeError for objects without to_node() and non-dict."""
    ctx = _make_ctx()
    with pytest.raises(TypeError, match="render_tree"):
        ctx.render_tree(42)


def test_render_tree_rejects_string() -> None:
    """Strings have no to_node() and are not dicts — TypeError expected."""
    ctx = _make_ctx()
    with pytest.raises(TypeError, match="render_tree"):
        ctx.render_tree("not a node")
