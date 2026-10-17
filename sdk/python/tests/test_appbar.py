"""Tests for AppBar declarative component."""

from plexi_sdk._adapter import _encode_uitree
from plexi_sdk.ui import AppBar


def test_appbar_to_node_and_wire_encoding() -> None:
    bar = AppBar("Dashboard", "live metrics")
    assert bar.to_node() == {
        "type": "app_bar",
        "title": "Dashboard",
        "subtitle": "live metrics",
    }
    tree = _encode_uitree(bar)
    assert tree["nodes"][tree["root"]]["data"] == {
        "type": "AppBar",
        "title": "Dashboard",
        "subtitle": "live metrics",
    }
