"""Rows are self-describing nodes (stint 1636).

Each list or table row carries its own selection and complete action, and a
table header is ``columns``, never a child. The encoded trees are pinned to
golden JSON files the host's Rust tests decode and click, so both sides of the
bridge agree on one shape.
"""

import json
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from plexi_sdk._adapter import _encode_uitree
from plexi_sdk._v3_delta import child_refs
from plexi_sdk.ui import Button, DataTable, Keyed, Label, SelectList

GOLDEN = Path(__file__).resolve().parent / "golden" / "rows"

CASES = {
    "table_headerless": lambda: DataTable(rows=[["A"], ["B"]], on_select="pick"),
    "table_headered": lambda: DataTable(columns=["Name"], rows=[["A"], ["B"]], on_select="pick"),
    "list_button_row": lambda: SelectList([Button("Run", "run")], on_select="pick"),
    "list_button_row_no_handler": lambda: SelectList([Button("Run", "run")]),
}


def _data(tree: dict, node_id: int) -> dict:
    return next(node["data"] for node in tree["nodes"] if node["id"] == node_id)


@pytest.mark.parametrize("name", sorted(CASES))
def test_encoding_matches_golden_fixture_shared_with_host(name: str) -> None:
    encoded = _encode_uitree(CASES[name]())
    golden = json.loads((GOLDEN / f"{name}.json").read_text())
    assert encoded == golden


@pytest.mark.parametrize("columns", [[], ["Name"]])
def test_data_table_rows_are_exactly_its_table_row_children(columns: list[str]) -> None:
    tree = _encode_uitree(DataTable(columns=columns, rows=[["A"], ["B"]], on_select="pick"))
    table = _data(tree, tree["root"])
    assert table["type"] == "Table"
    assert table["columns"] == columns
    assert "children" not in table
    rows = [_data(tree, row_id) for row_id in table["rows"]]
    assert [row["type"] for row in rows] == ["TableRow", "TableRow"]
    assert [row["cells"] for row in rows] == [["A"], ["B"]]
    assert [row["on_select"] for row in rows] == ["pick:0", "pick:1"]


def test_data_table_selection_is_a_flag_on_the_row() -> None:
    tree = _encode_uitree(DataTable(rows=[["A"], ["B"]], selected=1, on_select="pick"))
    table = _data(tree, tree["root"])
    assert [_data(tree, row_id)["selected"] for row_id in table["rows"]] == [False, True]


def test_select_list_rows_carry_selection_and_full_action() -> None:
    tree = _encode_uitree(SelectList(["a", "b"], selected_idx=1, on_select="pick"))
    listing = _data(tree, tree["root"])
    assert set(listing) == {"type", "items"}
    rows = [_data(tree, row_id) for row_id in listing["items"]]
    assert [row["type"] for row in rows] == ["ListItem", "ListItem"]
    assert [row["selected"] for row in rows] == [False, True]
    assert [row["on_select"] for row in rows] == ["pick:0", "pick:1"]
    assert [_data(tree, row["child"])["text"] for row in rows] == ["a", "b"]


def test_select_list_without_handler_rows_have_no_action() -> None:
    tree = _encode_uitree(SelectList(["a"]))
    row = _data(tree, _data(tree, tree["root"])["items"][0])
    assert row["on_select"] is None


def test_keyed_item_key_moves_onto_its_row() -> None:
    tree = _encode_uitree(SelectList([Keyed(Label("x"), key="k1"), "y"], on_select="pick"))
    by_id = {node["id"]: node for node in tree["nodes"]}
    first_row = by_id[_data(tree, tree["root"])["items"][0]]
    assert first_row["key"] == "0/@k1"
    assert by_id[first_row["data"]["child"]]["key"] == "0/@k1/0"


def test_delta_child_refs_follow_rows() -> None:
    assert child_refs({"type": "Table", "columns": [], "rows": [3, 4]}) == [3, 4]
    assert child_refs({"type": "ListItem", "child": 7}) == [7]
    assert child_refs({"type": "TableRow", "cells": ["a"]}) == []
