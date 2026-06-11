"""Tests for the Flow node-canvas app.

Unit tests exercise the pure graph/exec logic directly; e2e tests boot the
real app subprocess through AppHarness and drive it with key, mouse, and
component events. The e2e graph state lives at /tmp/.plexi/flow.json (the
harness handshake pins workspace_root to /tmp), so each test seeds/cleans it.
"""
from __future__ import annotations

import asyncio
import importlib.util
import json
import pathlib
import sys
import time

import pytest

ROOT = pathlib.Path(__file__).resolve().parents[3]
SDK = ROOT / "sdk" / "python"
APP = ROOT / "apps" / "flow" / "main.py"

sys.path.insert(0, str(SDK))

from plexi_sdk.testing import AppHarness  # noqa: E402

HARNESS_FLOW_FILE = pathlib.Path("/tmp/.plexi/flow.json")


def _load_module():
    spec = importlib.util.spec_from_file_location("flow_app", APP)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


flow = _load_module()


def _bare_app(workspace_root: str = "/tmp"):
    """FlowApp with graph state only — skips App.__init__/event loop."""
    app = flow.FlowApp.__new__(flow.FlowApp)
    app.nodes = {}
    app.edges = []
    app.workspace_root = workspace_root
    return app


def _node(nid: int, kind: str = "prompt", text: str = ""):
    return flow.Node(nid, kind, 0.0, 0.0, text=text)


# ── unit: geometry ───────────────────────────────────────────────────────────

def test_bezier_hits_both_endpoints():
    pts = flow._bezier_points(10.0, 20.0, 300.0, 200.0)
    assert pts[0] == (10.0, 20.0)
    assert pts[-1] == pytest.approx((300.0, 200.0))


def test_dist_to_segment():
    assert flow._dist_to_segment(5, 5, 0, 0, 10, 0) == pytest.approx(5.0)
    assert flow._dist_to_segment(20, 0, 0, 0, 10, 0) == pytest.approx(10.0)
    assert flow._dist_to_segment(3, 4, 1, 1, 1, 1) == pytest.approx(((2**2 + 3**2) ** 0.5))


# ── unit: graph logic ────────────────────────────────────────────────────────

def test_topo_order_respects_edges():
    app = _bare_app()
    app.nodes = {i: _node(i) for i in (1, 2, 3)}
    app.edges = [(3, 2), (2, 1)]
    assert app._topo_order() == [3, 2, 1]


def test_topo_order_none_on_cycle():
    app = _bare_app()
    app.nodes = {1: _node(1), 2: _node(2)}
    app.edges = [(1, 2), (2, 1)]
    assert app._topo_order() is None


def test_creates_cycle():
    app = _bare_app()
    app.nodes = {i: _node(i) for i in (1, 2, 3)}
    app.nodes[4] = _node(4)
    app.edges = [(1, 2), (2, 3)]
    assert app._creates_cycle(3, 1) is True
    assert app._creates_cycle(3, 2) is True   # 2→3 exists, so 3→2 closes a loop
    assert app._creates_cycle(1, 3) is False
    assert app._creates_cycle(4, 1) is False  # disjoint node is always safe


def test_inputs_for_joins_upstream_outputs():
    app = _bare_app()
    app.nodes = {i: _node(i) for i in (1, 2, 3)}
    app.nodes[1].output = "alpha"
    app.nodes[2].output = "beta"
    app.edges = [(1, 3), (2, 3)]
    assert app._inputs_for(3) == "alpha\nbeta"


def test_save_load_round_trip(tmp_path):
    app = _bare_app(str(tmp_path))
    app.nodes = {7: flow.Node(7, "shell", 12.5, -40.0, title="Upper", text="tr a-z A-Z")}
    app.edges = [(7, 7)]  # shape only; load doesn't validate

    class _Emit:
        def error(self, msg):
            raise AssertionError(msg)
    app.emit = _Emit()
    app._save()

    app2 = _bare_app(str(tmp_path))
    app2.emit = _Emit()
    app2.next_id = 1
    app2._load()
    n = app2.nodes[7]
    assert (n.kind, n.x, n.y, n.title, n.text) == ("shell", 12.5, -40.0, "Upper", "tr a-z A-Z")
    assert app2.edges == [(7, 7)]
    assert app2.next_id == 8


# ── unit: node execution ─────────────────────────────────────────────────────

def _exec(app, node, input_text=""):
    return asyncio.run(app._exec_node(node, input_text))


def test_exec_prompt_substitutes_input():
    app = _bare_app()
    node = _node(1, "prompt", "Summarize: {{input}} — done")
    assert _exec(app, node, "the report") == "Summarize: the report — done"


def test_exec_ai_is_a_marked_stub():
    app = _bare_app()
    node = _node(1, "ai", "Improve: {{input}}")
    out = _exec(app, node, "draft")
    assert out.startswith("[ai stub")
    assert "Improve: draft" in out


def test_exec_file_read_write_round_trip(tmp_path):
    app = _bare_app(str(tmp_path))
    writer = _node(1, "file_write", "sub/out.txt")
    assert _exec(app, writer, "payload") == "payload"
    reader = _node(2, "file_read", "sub/out.txt")
    assert _exec(app, reader) == "payload"
    assert (tmp_path / "sub" / "out.txt").read_text() == "payload"


def test_exec_file_read_missing_path_raises():
    app = _bare_app()
    with pytest.raises(ValueError):
        _exec(app, _node(1, "file_read", "   "))


def test_exec_shell_pipes_stdin(tmp_path):
    app = _bare_app(str(tmp_path))
    node = _node(1, "shell", "tr a-z A-Z")
    assert _exec(app, node, "hello") == "HELLO"


def test_exec_shell_failure_raises(tmp_path):
    app = _bare_app(str(tmp_path))
    node = _node(1, "shell", "false")
    with pytest.raises(RuntimeError):
        _exec(app, node)


# ── e2e: AppHarness ──────────────────────────────────────────────────────────

@pytest.fixture
def clean_flow_file():
    HARNESS_FLOW_FILE.unlink(missing_ok=True)
    yield
    HARNESS_FLOW_FILE.unlink(missing_ok=True)


def _seed_graph(nodes: list[dict], edges: list[list[int]]) -> None:
    HARNESS_FLOW_FILE.parent.mkdir(parents=True, exist_ok=True)
    HARNESS_FLOW_FILE.write_text(json.dumps({"nodes": nodes, "edges": edges}))


def _find_text(cmds: list[dict], needle: str) -> dict | None:
    """Find `needle` in raw text draw commands (L0 canvas path)."""
    for c in cmds:
        if c.get("type") == "text" and needle in c.get("text", ""):
            return c
    return None


def _tree_contains(cmds: list[dict], needle: str) -> bool:
    """Find `needle` anywhere in a component_tree command (L1 view path)."""
    return any(c.get("type") == "component_tree" and needle in json.dumps(c)
               for c in cmds)


def test_e2e_boot_shows_empty_hint(clean_flow_file):
    with AppHarness(APP) as h:
        cmds = h.run(1)
    assert _find_text(cmds, "add your first node") is not None


def test_e2e_add_node_opens_editor_and_edits(clean_flow_file):
    with AppHarness(APP) as h:
        h.key("a")
        cmds = h.run(1)
        assert _tree_contains(cmds, "Prompt")  # add menu lists kinds

        h.key("1")  # add Prompt node → jumps straight into the editor
        cmds = h.run(1)
        assert _tree_contains(cmds, "{{input}}")  # kind hint visible
        assert _tree_contains(cmds, "body-1")     # editor TextEdit present

        h._send({"type": "component_event", "node_id": "body-1",
                 "event_type": "change", "payload": {"value": "hello {{input}}"}})
        h._send({"type": "component_event", "node_id": "title-1",
                 "event_type": "change", "payload": {"value": "Greeter"}})
        h.key("escape")  # back to canvas, saves
        cmds = h.run(1)
        assert _find_text(cmds, "Greeter") is not None
        assert _find_text(cmds, "hello {{input}}") is not None  # body preview

    saved = json.loads(HARNESS_FLOW_FILE.read_text())
    assert saved["nodes"][0]["title"] == "Greeter"
    assert saved["nodes"][0]["text"] == "hello {{input}}"


def test_e2e_drag_moves_node(clean_flow_file):
    _seed_graph([{"id": 1, "kind": "prompt", "x": 60.0, "y": 40.0,
                  "title": "DragMe", "text": ""}], [])
    with AppHarness(APP) as h:
        cmds = h.run(1)
        t = _find_text(cmds, "DragMe")
        assert t is not None
        sx, sy = t["x"], t["y"]  # title is drawn at node origin + (10, 6)

        grab = (sx + 30, sy + 40)  # inside the node body
        h._send({"type": "mouse_down", "x": grab[0], "y": grab[1], "button": "primary"})
        h._send({"type": "mouse_move", "x": grab[0] + 80, "y": grab[1] + 50,
                 "buttons": ["primary"]})
        h._send({"type": "mouse_up", "x": grab[0] + 80, "y": grab[1] + 50,
                 "button": "primary"})
        cmds = h.run(1)
        t2 = _find_text(cmds, "DragMe")
        assert t2 is not None
        assert (t2["x"] - sx, t2["y"] - sy) == pytest.approx((80.0, 50.0))


def test_e2e_wire_ports_with_mouse(clean_flow_file):
    _seed_graph([
        {"id": 1, "kind": "prompt", "x": 40.0, "y": 60.0, "title": "Src", "text": "x"},
        {"id": 2, "kind": "prompt", "x": 360.0, "y": 200.0, "title": "Dst", "text": "y"},
    ], [])
    with AppHarness(APP) as h:
        cmds = h.run(1)
        src = _find_text(cmds, "Src")
        dst = _find_text(cmds, "Dst")
        assert src is not None and dst is not None
        # node origin = title pos - (10, 6); ports sit at (±0/W, H/2)
        out_port = (src["x"] - 10 + flow.NODE_W, src["y"] - 6 + flow.NODE_H / 2)
        in_port = (dst["x"] - 10, dst["y"] - 6 + flow.NODE_H / 2)
        h._send({"type": "mouse_down", "x": out_port[0], "y": out_port[1],
                 "button": "primary"})
        h._send({"type": "mouse_move", "x": in_port[0], "y": in_port[1],
                 "buttons": ["primary"]})
        h._send({"type": "mouse_up", "x": in_port[0], "y": in_port[1],
                 "button": "primary"})
        h.run(1)

    saved = json.loads(HARNESS_FLOW_FILE.read_text())
    assert saved["edges"] == [[1, 2]]


def test_e2e_run_pipeline_writes_file(clean_flow_file, tmp_path):
    out_file = tmp_path / "result.txt"
    _seed_graph([
        {"id": 1, "kind": "prompt", "x": 0, "y": 0, "title": "P",
         "text": "hello flow"},
        {"id": 2, "kind": "shell", "x": 250, "y": 0, "title": "S",
         "text": "tr a-z A-Z"},
        {"id": 3, "kind": "file_write", "x": 500, "y": 0, "title": "W",
         "text": str(out_file)},
    ], [[1, 2], [2, 3]])
    with AppHarness(APP) as h:
        h.run(1)
        h.key("r")
        deadline = time.monotonic() + 10.0
        while time.monotonic() < deadline and not out_file.exists():
            h.run(1)
            time.sleep(0.1)
    assert out_file.read_text() == "HELLO FLOW"
