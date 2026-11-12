"""Wire contract of the approved CLI operation effect (stint 1175).

The app names an operation and typed values. The wire must never carry an
executable, argv, environment, working directory or shell text.
"""

import json
import textwrap
from pathlib import Path

from test_v3_runtime_regression import (
    _collect_until,
    _find_events,
    _init_and_render,
    _init_app,
    _send_event,
    _spawn_v3_app,
)


def _app(tmp_path: Path, init_effects: str) -> Path:
    app = tmp_path / "app.py"
    app.write_text(textwrap.dedent(f"""
        from plexi_sdk.effects import *
        from plexi_sdk.events import *
        from plexi_sdk.ui import Text

        def init(size, args):
            return [{init_effects}]

        def update(event):
            if isinstance(event, CliOperationResult):
                if event.ok:
                    return [SetTitle("ok " + event.request_id + " " + json.dumps(event.data))]
                return [SetTitle("fail " + event.request_id + " " + event.error_code
                                 + " " + str(event.exit_code))]
            return []

        def view():
            return Text("ok")
    """).lstrip().replace("from plexi_sdk.ui import Text", "import json\nfrom plexi_sdk.ui import Text"))
    return app


def test_cli_operation_wire_carries_only_id_operation_and_values(tmp_path):
    app = _app(
        tmp_path,
        "CliOperation(request_id='r1', operation_id='task.set', values={'priority': 'p1', 'task': '0042'})",
    )
    proc = _spawn_v3_app(app)
    try:
        events = _init_and_render(proc)
        sent = _find_events(events, "cli_operation")
        assert sent == [{
            "type": "cli_operation",
            "request_id": "r1",
            "operation_id": "task.set",
            "values": {"priority": "p1", "task": "0042"},
        }]
        for forbidden in ("executable", "argv", "env", "cwd", "command", "shell", "workspace"):
            assert forbidden not in sent[0]
    finally:
        proc.kill()


def test_cli_operation_cancel_wire(tmp_path):
    app = _app(tmp_path, "CliOperationCancel(request_id='r1')")
    proc = _spawn_v3_app(app)
    try:
        events = _init_and_render(proc)
        assert _find_events(events, "cli_operation_cancel") == [
            {"type": "cli_operation_cancel", "request_id": "r1"}
        ]
    finally:
        proc.kill()


def test_cli_operation_result_success_and_failure_reach_the_app(tmp_path):
    proc = _spawn_v3_app(_app(tmp_path, ""))
    try:
        _init_app(proc, capabilities=["cli.operations"])
        _send_event(proc, {
            "type": "cli_operation_result", "request_id": "a", "operation_id": "x",
            "ok": True, "data": {"tasks": [1, 2]},
        })
        titles = [e["title"] for e in _find_events(_collect_until(proc, "schedule_render"), "set_title")]
        assert titles == ['ok a {"tasks": [1, 2]}']
        _send_event(proc, {
            "type": "cli_operation_result", "request_id": "b", "operation_id": "x",
            "ok": False, "error_code": "nonzero_exit", "error": "exited with status 7", "exit_code": 7,
        })
        titles = [e["title"] for e in _find_events(_collect_until(proc, "schedule_render"), "set_title")]
        assert titles == ["fail b nonzero_exit 7"]
        _send_event(proc, {
            "type": "cli_operation_result", "request_id": "c", "operation_id": "x",
            "ok": False, "error_code": "denied", "error": "the approval was denied",
        })
        titles = [e["title"] for e in _find_events(_collect_until(proc, "schedule_render"), "set_title")]
        assert titles == ["fail c denied None"]
    finally:
        proc.kill()


def test_cli_operation_rejects_non_dict_values_before_emitting(tmp_path):
    app = _app(tmp_path, "CliOperation(request_id='r', operation_id='x', values=['--flag'])")
    proc = _spawn_v3_app(app)
    try:
        _init_app(proc)
        events = _collect_until(proc, "frame_done", timeout=1.5)
        assert _find_events(events, "cli_operation") == []
    finally:
        proc.kill()


def test_nonzero_exit_failure_carries_structured_data_and_stderr(tmp_path):
    proc = _spawn_v3_app(_app(tmp_path, ""))
    try:
        _init_app(proc, capabilities=["cli.operations"])
        _send_event(proc, {
            "type": "cli_operation_result", "request_id": "a", "operation_id": "x",
            "ok": False, "error_code": "nonzero_exit", "error": "exited with status 1: conflict",
            "exit_code": 1, "stderr": "conflict", "data_available": True,
            "data": {"error": "priority_conflict"},
        })
        titles = [e["title"] for e in _find_events(_collect_until(proc, "schedule_render"), "set_title")]
        assert titles == ["fail a nonzero_exit 1"]
    finally:
        proc.kill()
