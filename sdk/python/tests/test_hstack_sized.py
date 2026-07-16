"""Tests for stint 0308 layout components: HStack and Sized."""

import pytest

from plexi_sdk.ui import Canvas, HStack, Sized, Text


def test_hstack_to_node_horizontal_stack() -> None:
    node = HStack([Text(text="a"), Text(text="b")], gap=12.0).to_node()
    assert node is not None
    assert node["type"] == "row"
    assert node["gap"] == 12.0
    assert len(node["children"]) == 2


def test_sized_to_node_wraps_child_with_dimensions() -> None:
    node = Sized(Text(text="side"), width=160.0).to_node()
    assert node is not None
    assert node["type"] == "sized"
    assert node["width"] == 160.0
    assert node["height"] is None
    assert node["child"]["type"] == "text"


def test_hstack_canvas_grow_plus_sized_sidebar() -> None:
    """The target pattern: growing Canvas beside a fixed-width Sized sidebar."""
    canvas = Canvas([], grow=True)
    sidebar = Sized(Text(text="side"), width=160.0)
    node = HStack([canvas, sidebar], gap=12.0).to_node()
    assert node is not None
    assert node["children"][0]["type"] == "canvas"
    assert node["children"][0]["grow"] is True
    assert node["children"][1]["type"] == "sized"
    assert node["children"][1]["width"] == 160.0


def test_sized_is_not_grow() -> None:
    assert Sized(Text(text="x"), width=80.0).is_grow() is False


def test_hstack_rejects_non_component_children() -> None:
    with pytest.raises(TypeError):
        HStack([{"type": "text"}])
