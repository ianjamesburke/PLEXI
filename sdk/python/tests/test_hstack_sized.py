"""Tests for stint 0308 layout components: HStack and Sized.

Sized has no to_node() (see its docstring in ui.py: there is no host-native
fixed-size wrapper node) and is not exported from ui.__all__. It remains
usable in canvas mode only (measure()/render()/is_grow()).
"""

import pytest

from plexi_sdk.ui import Canvas, HStack, Sized, Text


def test_hstack_to_node_horizontal_stack() -> None:
    node = HStack([Text(text="a"), Text(text="b")], gap=12.0).to_node()
    assert node is not None
    assert node["type"] == "row"
    assert node["gap"] == 12.0
    assert len(node["children"]) == 2


def test_hstack_fails_loud_when_a_child_has_no_tree_node() -> None:
    """A Sized child can't produce a UiNode; HStack must propagate that as
    None (matching Column/Card/Scrollable) rather than emit a partial tree."""
    sidebar = Sized(Text(text="side"), width=160.0)
    node = HStack([Text(text="a"), sidebar], gap=12.0).to_node()
    assert node is None


def test_hstack_canvas_grow_plus_sized_sidebar_layout() -> None:
    """The target pattern: growing Canvas beside a fixed-width Sized sidebar,
    in canvas mode (measure()/render()), not the declarative tree."""
    canvas = Canvas([], grow=True)
    sidebar = Sized(Text(text="side"), width=160.0)
    assert canvas.is_grow() is True
    assert sidebar.is_grow() is False
    assert sidebar.measure(400.0) == Text(text="side").measure(160.0)


def test_sized_is_not_grow() -> None:
    assert Sized(Text(text="x"), width=80.0).is_grow() is False


def test_hstack_rejects_non_component_children() -> None:
    with pytest.raises(TypeError):
        HStack([{"type": "text"}])
