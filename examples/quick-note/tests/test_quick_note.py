"""Tests for the Quick Note PGAP app."""
from __future__ import annotations
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent.parent.parent.parent / "sdk" / "python"))
from plexi_test import Harness

APP = Path(__file__).parent.parent / "quick_note.py"


def test_ready_handshake():
    with Harness.for_app(APP) as h:
        ready = h.init()
        assert ready["type"] == "ready"


def test_renders_frame():
    with Harness.for_app(APP) as h:
        h.init()
        cmds = h.render_frame(800, 600)
        types = [c.get("type") for c in cmds]
        assert "frame_done" in types
