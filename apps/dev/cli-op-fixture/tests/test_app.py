"""Tests for Cli Op Fixture.

Run with:  plexi plugin test          (from the app directory)
       or:  uv run --no-project --with pytest pytest tests/

PluginHarness spawns this app as a real subprocess and speaks the PGAP protocol
over stdin/stdout, with no running Plexi host or display required.
"""
from pathlib import Path

import pytest

from plexi_sdk.testing import PluginHarness

APP = Path(__file__).resolve().parent.parent / "main.py"
FORBIDDEN = ("executable", "argv", "env", "cwd", "command", "shell", "workspace")


@pytest.mark.parametrize("size", [(300, 200), (800, 600)])
def test_renders_without_overlap(size):
    width, height = size
    with PluginHarness(str(APP), width=width, height=height) as h:
        h.run(1)
        h.assert_no_overlap()


@pytest.mark.parametrize(
    ("key", "operation_id", "values"),
    [
        ("r", "fixture.echo", {"word": "hello"}),
        ("m", "fixture.set", {"priority": "p1", "task": "1175"}),
        ("s", "fixture.slow", {"ms": 8000}),
    ],
)
def test_key_requests_one_typed_operation_and_nothing_else(key, operation_id, values):
    with PluginHarness(str(APP)) as h:
        h.run(1)
        h.key(key)
        sent = [e for e in h.run(1) if e.get("type") == "cli_operation"]
    assert len(sent) == 1
    assert sent[0]["operation_id"] == operation_id
    assert sent[0]["values"] == values
    assert sent[0]["request_id"] == "req-1"
    assert not any(name in sent[0] for name in FORBIDDEN)


def test_a_second_request_waits_for_the_first_and_cancel_targets_it():
    with PluginHarness(str(APP)) as h:
        h.run(1)
        h.key("s")
        h.run(1)
        h.key("r")
        again = [e for e in h.run(1) if e.get("type") == "cli_operation"]
        assert again == [], "one request in flight at a time"
        h.key("c")
        cancels = [e for e in h.run(1) if e.get("type") == "cli_operation_cancel"]
    assert cancels == [{"type": "cli_operation_cancel", "request_id": "req-1"}]


def test_result_is_shown_and_frees_the_slot():
    with PluginHarness(str(APP)) as h:
        h.run(1)
        h.key("r")
        h.run(1)
        h._send({
            "type": "cli_operation_result", "request_id": "req-1",
            "operation_id": "fixture.echo", "ok": True, "data": {"argv": ["hello"]},
        })
        shown = str(h.run(1))
        h.key("r")
        next_request = [e for e in h.run(1) if e.get("type") == "cli_operation"]
    assert "req-1 ok" in shown
    assert next_request and next_request[0]["request_id"] == "req-2"


def test_denial_is_shown_without_a_result():
    with PluginHarness(str(APP)) as h:
        h.run(1)
        h.key("m")
        h.run(1)
        h._send({
            "type": "cli_operation_result", "request_id": "req-1",
            "operation_id": "fixture.set", "ok": False,
            "error_code": "denied", "error": "the approval was denied",
        })
        shown = str(h.run(1))
    assert "failed: denied" in shown
