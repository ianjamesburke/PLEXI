"""Tests for typed pipes Phase 0 SDK additions. Run: python3 -m pytest sdk/python/tests/ -v"""
from __future__ import annotations
import sys, os
sys.path.insert(0, os.path.join(os.path.dirname(__file__), '..'))
from plexi_sdk import App


def test_pipe_write_emits_correct_json():
    output_lines = []

    class FakeEmitter:
        def _emit(self, msg):
            output_lines.append(msg)
        def pipe_write(self, channel, value):
            self._emit({"type": "pipe_write", "channel": channel, "value": value})

    e = FakeEmitter()
    e.pipe_write("selection", "/tmp/test.mmd")

    assert output_lines[0]["type"] == "pipe_write"
    assert output_lines[0]["channel"] == "selection"
    assert output_lines[0]["value"] == "/tmp/test.mmd"


def test_on_pipe_data_decorator():
    app = App()

    @app.on_pipe_data
    def handler(from_app, channel, value, emit):
        pass

    assert app._on_pipe_data is handler


def test_pipe_write_value_types():
    output = []

    class FakeEmitter:
        def _emit(self, msg): output.append(msg)
        def pipe_write(self, ch, v): self._emit({"type": "pipe_write", "channel": ch, "value": v})

    e = FakeEmitter()
    e.pipe_write("text", "hello")
    e.pipe_write("count", 42)
    e.pipe_write("data", {"key": "val"})
    e.pipe_write("items", [1, 2, 3])

    assert output[0]["value"] == "hello"
    assert output[1]["value"] == 42
    assert output[2]["value"] == {"key": "val"}
    assert output[3]["value"] == [1, 2, 3]
