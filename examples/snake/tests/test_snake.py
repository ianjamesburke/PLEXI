"""Tests for the Snake PGAP app."""
from __future__ import annotations
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent.parent.parent.parent / "sdk" / "python"))
from plexi_test import Harness

APP = Path(__file__).parent.parent / "snake.py"


def test_ready_handshake():
    with Harness.for_app(APP) as h:
        ready = h.init()
        assert ready["type"] == "ready"


def test_renders_rects_on_first_frame():
    with Harness.for_app(APP) as h:
        h.init()
        cmds = h.render_frame(640, 480)
        assert h.rect_count(cmds) > 0, "snake must draw at least one rect"


def test_key_input_accepted():
    with Harness.for_app(APP) as h:
        h.init()
        h.render_frame(640, 480)
        h.send_key("ArrowUp")
        cmds = h.render_frame(640, 480)
        assert h.rect_count(cmds) > 0
