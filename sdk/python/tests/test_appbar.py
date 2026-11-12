"""Tests for SurfaceHeader declarative component."""

from plexi_sdk._adapter import _encode_uitree
from plexi_sdk.ui import SurfaceHeader


def test_appbar_to_node_and_wire_encoding() -> None:
    bar = SurfaceHeader("Dashboard", "live metrics")
    assert bar.to_node() == {
        "type": "surface_header",
        "title": "Dashboard",
        "subtitle": "live metrics",
    }
    tree = _encode_uitree(bar)
    assert tree["nodes"][tree["root"]]["data"] == {
        "type": "SurfaceHeader",
        "title": "Dashboard",
        "subtitle": "live metrics",
    }
