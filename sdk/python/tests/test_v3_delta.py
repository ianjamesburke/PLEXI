"""Diffed component-tree protocol (stint 0438).

Drives the real `plexi_sdk._v3_process` subprocess and inspects the raw wire
framing — full `component_tree` vs `tree_delta` — because the whole point of the
feature is *which* message the guest chooses to emit. Reconstruction is tested
separately against `_v3_delta.apply_delta` / `TreeReconstructor`.
"""

import json
import subprocess
import sys
import textwrap
import time
from pathlib import Path

import pytest

from plexi_sdk._v3_delta import (
    DeltaApplyError,
    TreeReconstructor,
    apply_delta,
    node_patch,
)

REPO_ROOT = Path(__file__).resolve().parents[3]
SDK_PATH = str(REPO_ROOT / "sdk" / "python")


def _make_env():
    import os

    env = dict(os.environ)
    env["PYTHONPATH"] = SDK_PATH
    return env


def _spawn(app_file: Path):
    return subprocess.Popen(
        [sys.executable, "-m", "plexi_sdk._v3_process", str(app_file)],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        env=_make_env(),
    )


def _send(proc, event: dict):
    proc.stdin.write(json.dumps(event) + "\n")
    proc.stdin.flush()


def _collect_raw(proc, target_type="frame_done", timeout=3.0) -> list[dict]:
    """Collect RAW wire events (no delta reconstruction) until target_type."""
    seen = []
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        line = proc.stdout.readline()
        if not line:
            break
        ev = json.loads(line)
        seen.append(ev)
        if ev.get("type") == target_type:
            return seen
    return seen


def _init(proc, state=None):
    msg = {
        "type": "init",
        "app_id": "delta-test",
        "workspace_root": "/tmp",
        "capabilities": [],
        "feature_flags": [],
        "width": 640.0,
        "height": 360.0,
    }
    if state is not None:
        msg["state"] = state
    _send(proc, msg)
    return _collect_raw(proc, "ready")


def _render(proc, frame_id: int) -> list[dict]:
    _send(proc, {
        "type": "render",
        "frame_id": frame_id,
        "rect": {"x": 0.0, "y": 0.0, "w": 640.0, "h": 360.0},
    })
    return _collect_raw(proc, "frame_done")


def _only(events: list[dict], event_type: str) -> list[dict]:
    return [e for e in events if e.get("type") == event_type]


COUNTER_APP = textwrap.dedent("""
    from plexi_sdk import state
    from plexi_sdk.effects import SetState
    from plexi_sdk.events import KeyEvent
    from plexi_sdk.ui import Text

    def init(size, args):
        return [SetState({"n": 0})]

    def update(event):
        if isinstance(event, KeyEvent) and event.pressed:
            return [SetState({"n": state.get("n", 0) + 1})]
        return []

    def view():
        return Text(f"n={state.get('n', 0)}")
""").lstrip()


CANVAS_APP = textwrap.dedent("""
    from plexi_sdk import state
    from plexi_sdk.effects import SetState
    from plexi_sdk.events import KeyEvent
    from plexi_sdk.ui import Canvas, CanvasRect

    def init(size, args):
        return [SetState({"x": 0.0})]

    def update(event):
        if isinstance(event, KeyEvent) and event.pressed:
            return [SetState({"x": state.get("x", 0.0) + 10.0})]
        return []

    def view():
        x = state.get("x", 0.0)
        return Canvas([
            CanvasRect(0, 0, 640, 360, "#000000"),
            CanvasRect(x, 100, 20, 20, "#ff0000"),
        ], width=640, height=360)
""").lstrip()


FULL_MOTION_APP = textwrap.dedent("""
    from plexi_sdk import state
    from plexi_sdk.effects import SetState
    from plexi_sdk.events import KeyEvent
    from plexi_sdk.ui import Canvas, CanvasCircle

    def init(size, args):
        return [SetState({"t": 0.0})]

    def update(event):
        if isinstance(event, KeyEvent) and event.pressed:
            return [SetState({"t": state.get("t", 0.0) + 1.0})]
        return []

    def view():
        t = state.get("t", 0.0)
        return Canvas([
            CanvasCircle(i * 7.0 + t, i * 3.0 + t, 12.0, "#0000003c")
            for i in range(50)
        ], width=640, height=360)
""").lstrip()


PARTIAL_MOTION_APP = textwrap.dedent("""
    from plexi_sdk import state
    from plexi_sdk.effects import SetState
    from plexi_sdk.events import KeyEvent
    from plexi_sdk.ui import Canvas, CanvasCircle

    def init(size, args):
        return [SetState({"t": 0.0})]

    def update(event):
        if isinstance(event, KeyEvent) and event.pressed:
            return [SetState({"t": state.get("t", 0.0) + 1.0})]
        return []

    def view():
        t = state.get("t", 0.0)
        return Canvas([
            CanvasCircle(i * 7.0 + (t if i == 1 else 0.0), i * 3.0, 12.0, "#0000003c")
            for i in range(50)
        ], width=640, height=360)
""").lstrip()


STRUCTURAL_APP = textwrap.dedent("""
    from plexi_sdk import state
    from plexi_sdk.effects import SetState
    from plexi_sdk.events import KeyEvent
    from plexi_sdk.ui import Column, Text

    def init(size, args):
        return [SetState({"rows": 1})]

    def update(event):
        if isinstance(event, KeyEvent) and event.pressed:
            return [SetState({"rows": state.get("rows", 1) + 1})]
        return []

    def view():
        rows = state.get("rows", 1)
        return Column([Text(f"row-{i}") for i in range(rows)])
""").lstrip()


def _key(proc, key="x"):
    _send(proc, {"type": "key", "key": key, "pressed": True, "modifiers": {}})


class TestWireFraming:
    def test_first_frame_is_full_tree(self, tmp_path):
        app = tmp_path / "a.py"
        app.write_text(COUNTER_APP)
        proc = _spawn(app)
        try:
            _init(proc)
            events = _render(proc, 1)
            assert _only(events, "component_tree"), events
            assert not _only(events, "tree_delta")
        finally:
            proc.kill()

    def test_noop_frame_emits_empty_delta(self, tmp_path):
        app = tmp_path / "a.py"
        app.write_text(COUNTER_APP)
        proc = _spawn(app)
        try:
            _init(proc)
            _render(proc, 1)
            events = _render(proc, 2)  # nothing changed
            deltas = _only(events, "tree_delta")
            assert deltas, events
            assert deltas[0]["changed"] == []
            assert not _only(events, "component_tree")
        finally:
            proc.kill()

    def test_single_property_change_emits_minimal_delta(self, tmp_path):
        app = tmp_path / "a.py"
        app.write_text(COUNTER_APP)
        proc = _spawn(app)
        try:
            _init(proc)
            first = _render(proc, 1)
            full_tree = _only(first, "component_tree")[0]["tree"]
            _key(proc)
            events = _render(proc, 2)
            deltas = _only(events, "tree_delta")
            assert deltas, events
            changed = deltas[0]["changed"]
            # Exactly the one text node changed; it is a full-node replacement.
            assert len(changed) == 1
            assert "n=1" in json.dumps(changed[0])
            # Reconstruction reproduces the same arena the guest would have sent.
            rebuilt = apply_delta(full_tree, changed)
            assert "n=1" in json.dumps(rebuilt)
            assert len(rebuilt["nodes"]) == len(full_tree["nodes"])
        finally:
            proc.kill()

    def test_canvas_command_mutation_emits_commands_changed(self, tmp_path):
        app = tmp_path / "a.py"
        app.write_text(CANVAS_APP)
        proc = _spawn(app)
        try:
            _init(proc)
            first = _render(proc, 1)
            full_tree = _only(first, "component_tree")[0]["tree"]
            _key(proc)  # moves the red rect: same command count
            events = _render(proc, 2)
            deltas = _only(events, "tree_delta")
            assert deltas, events
            changed = deltas[0]["changed"]
            assert len(changed) == 1
            patch = changed[0]
            assert "commands_changed" in patch, patch
            # Only the moved rect (index 1) changed, not the background (index 0).
            indices = [entry[0] for entry in patch["commands_changed"]]
            assert indices == [1], patch
            rebuilt = apply_delta(full_tree, changed)
            moved = rebuilt["nodes"][patch["id"]]["data"]["commands"][1]
            assert moved["x"] == 10.0
        finally:
            proc.kill()

    def test_full_motion_canvas_falls_back_to_the_smaller_full_frame(self, tmp_path):
        # Every command moves every frame, so each one is re-sent wrapped in its
        # own index: the delta comes out larger than the frame it replaces.
        app = tmp_path / "a.py"
        app.write_text(FULL_MOTION_APP)
        proc = _spawn(app)
        try:
            _init(proc)
            first = _render(proc, 1)
            full_tree = _only(first, "component_tree")[0]["tree"]
            _key(proc)  # moves all commands at once
            events = _render(proc, 2)
            assert not _only(events, "tree_delta"), events
            full = _only(events, "component_tree")
            assert full, events
            # The fallback is the existing full-frame shape, not a new one, and
            # it really is smaller than the delta it replaced.
            nodes = full[0]["tree"]["nodes"]
            assert len(nodes) == len(full_tree["nodes"])
            changed = [
                node_patch(node, old)
                for node, old in zip(nodes, full_tree["nodes"])
                if node != old
            ]
            delta_bytes = len(json.dumps(
                {"type": "tree_delta", "frame_id": 2, "changed": changed},
                separators=(",", ":"),
            ))
            full_bytes = len(json.dumps(full[0], separators=(",", ":")))
            assert full_bytes <= delta_bytes, (full_bytes, delta_bytes)
        finally:
            proc.kill()

    def test_partial_motion_canvas_still_emits_the_smaller_delta(self, tmp_path):
        # The other direction: when only some commands move, the delta wins and
        # must still be chosen.
        app = tmp_path / "a.py"
        app.write_text(PARTIAL_MOTION_APP)
        proc = _spawn(app)
        try:
            _init(proc)
            _render(proc, 1)
            _key(proc)
            events = _render(proc, 2)
            deltas = _only(events, "tree_delta")
            assert deltas, events
            assert not _only(events, "component_tree")
            indices = [e[0] for e in deltas[0]["changed"][0]["commands_changed"]]
            assert indices == [1], deltas
        finally:
            proc.kill()

    def test_structural_change_forces_full_tree(self, tmp_path):
        app = tmp_path / "a.py"
        app.write_text(STRUCTURAL_APP)
        proc = _spawn(app)
        try:
            _init(proc)
            _render(proc, 1)
            _key(proc)  # rows 1 -> 2: node count changes
            events = _render(proc, 2)
            assert _only(events, "component_tree"), events
            assert not _only(events, "tree_delta")
        finally:
            proc.kill()

    def test_request_full_tree_forces_next_frame_full(self, tmp_path):
        app = tmp_path / "a.py"
        app.write_text(COUNTER_APP)
        proc = _spawn(app)
        try:
            _init(proc)
            _render(proc, 1)
            _render(proc, 2)  # would be a delta
            _send(proc, {"type": "request_full_tree"})
            # The handler schedules a render; drain any scheduled emissions and
            # then explicitly render to observe the forced-full frame.
            events = _render(proc, 3)
            assert _only(events, "component_tree"), events
            assert not _only(events, "tree_delta")
        finally:
            proc.kill()


class TestReconstruction:
    def test_apply_delta_replaces_full_node(self):
        prev = {"root": 0, "nodes": [
            {"id": 0, "key": "0", "data": {"type": "Text", "text": "a"}},
        ]}
        changed = [{"id": 0, "key": "0", "data": {"type": "Text", "text": "b"}}]
        out = apply_delta(prev, changed)
        assert out["nodes"][0]["data"]["text"] == "b"
        assert prev["nodes"][0]["data"]["text"] == "a"  # base untouched

    def test_apply_delta_patches_canvas_commands(self):
        prev = {"root": 0, "nodes": [
            {"id": 0, "key": "0", "data": {
                "type": "canvas",
                "commands": [{"type": "rect", "x": 0}, {"type": "rect", "x": 5}],
            }},
        ]}
        changed = [{"id": 0, "key": "0", "commands_changed": [[1, {"type": "rect", "x": 99}]]}]
        out = apply_delta(prev, changed)
        assert out["nodes"][0]["data"]["commands"][1]["x"] == 99
        assert out["nodes"][0]["data"]["commands"][0]["x"] == 0
        assert prev["nodes"][0]["data"]["commands"][1]["x"] == 5  # base untouched

    def test_apply_delta_rejects_out_of_range_id(self):
        prev = {"root": 0, "nodes": [{"id": 0, "key": "0", "data": {"type": "Empty"}}]}
        with pytest.raises(DeltaApplyError):
            apply_delta(prev, [{"id": 5, "key": "5", "data": {"type": "Empty"}}])

    def test_apply_delta_rejects_out_of_range_command(self):
        prev = {"root": 0, "nodes": [
            {"id": 0, "key": "0", "data": {"type": "canvas", "commands": [{"type": "rect"}]}},
        ]}
        with pytest.raises(DeltaApplyError):
            apply_delta(prev, [{"id": 0, "key": "0", "commands_changed": [[9, {"type": "rect"}]]}])

    def test_reconstructor_delta_before_full_raises(self):
        r = TreeReconstructor()
        with pytest.raises(DeltaApplyError):
            r.ingest({"type": "tree_delta", "frame_id": 1, "changed": []})

    def test_reconstructor_passes_through_non_tree_events(self):
        r = TreeReconstructor()
        ev = {"type": "frame_done", "frame_id": 1}
        assert r.ingest(ev) is ev

    def test_node_patch_falls_back_to_full_node_on_command_count_change(self):
        old = {"id": 0, "key": "0", "data": {"type": "canvas", "commands": [{"a": 1}]}}
        new = {"id": 0, "key": "0", "data": {"type": "canvas", "commands": [{"a": 1}, {"a": 2}]}}
        assert node_patch(new, old) is new  # full node, not commands_changed
