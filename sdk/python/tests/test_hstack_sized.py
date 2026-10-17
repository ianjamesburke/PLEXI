"""Tests for horizontal layout containers: HStack and Row alias."""

import pytest

from plexi_sdk.ui import Column, Component, HStack, Row, Text


def test_row_is_hstack_alias() -> None:
    assert Row is HStack


def test_hstack_to_node_horizontal_stack() -> None:
    node = HStack([Text(text="a"), Text(text="b")], gap=12.0).to_node()
    assert node is not None
    assert node["type"] == "row"
    assert node["gap"] == 12.0
    assert len(node["children"]) == 2


def test_containers_omit_gap_when_unset_but_keep_explicit_zero() -> None:
    """Good-by-default spacing (stint 0445): an unset gap is omitted from the
    wire so the host applies its default; an explicit ``gap=0`` is preserved so
    the host packs children flush. The two must stay distinguishable."""
    assert "gap" not in HStack([Text(text="a")]).to_node()
    assert HStack([Text(text="a")], gap=0.0).to_node()["gap"] == 0.0

    col_default = Column([Text(text="a")]).to_node()
    assert col_default is not None
    assert "gap" not in col_default
    col_flush = Column([Text(text="a")], gap=0.0).to_node()
    assert col_flush is not None
    assert col_flush["gap"] == 0.0


def test_hstack_fails_loud_when_a_child_has_no_tree_node() -> None:
    """A bare Component base instance returns None from to_node(); HStack must
    propagate that as None rather than emit a partial tree."""
    empty_child = Component()
    node = HStack([Text(text="a"), empty_child], gap=12.0).to_node()
    assert node is None


def test_hstack_rejects_non_component_children() -> None:
    with pytest.raises(TypeError):
        HStack([{"type": "text"}])  # type: ignore[list-item]
