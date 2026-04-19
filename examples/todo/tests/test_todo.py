"""Tests for the Todo PGAP app."""
from __future__ import annotations
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent.parent.parent.parent / "sdk" / "python"))
from plexi_test import Harness

APP = Path(__file__).parent.parent / "todo.py"


def test_ready_handshake():
    with Harness.for_app(APP) as h:
        ready = h.init()
        assert ready["type"] == "ready"


def test_renders_todo_header():
    with Harness.for_app(APP) as h:
        h.init()
        cmds = h.render_frame(800, 600)
        assert h.has_text(cmds, "Todo"), f"got: {h.texts(cmds)}"


def test_path_changed_updates_cwd():
    with Harness.for_app(APP) as h:
        h.init()
        h.render_frame(800, 600)
        h.send_path_changed("/tmp/plexi-test-cwd")
        cmds = h.render_frame(800, 600)
        assert h.has_text(cmds, "/tmp/plexi-test-cwd"), f"got: {h.texts(cmds)}"
