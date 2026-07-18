"""Tests for the Logs app.

Run with:  plexi app test          (from apps/logs)
       or:  uv run pytest tests/

Pure-function tests cover log parsing and the host-log result handling. The
AppHarness tests drive the real init -> read_host_log -> render lifecycle: the
harness plays the host side of the `read_host_log` effect (stint 0444), tailing
a fixture log the same way the live host tails `~/.plexi-<channel>/plexi.log`,
so the app is exercised end-to-end through the sanctioned capability path.
"""

import importlib.util
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
SDK = ROOT / "sdk" / "python"
APP = ROOT / "apps" / "logs" / "logs.py"

sys.path.insert(0, str(SDK))

from plexi_sdk.events import HostLogResult  # noqa: E402
from plexi_sdk.testing import AppHarness  # noqa: E402

THREE_LINE_LOG = (
    "[2026-07-02 01:46:18] [INFO] [plexi::config] loaded config\n"
    "[2026-07-02 01:46:19] [ERROR] [app::todo] save failed\n"
    "[2026-07-02 01:46:20] [WARN] [app::todo] retrying\n"
)


def _load_app_module():
    spec = importlib.util.spec_from_file_location("logs_app", APP)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def _last_status(h):
    statuses = [c for c in h._events_seen if c.get("type") == "status_summary"]
    return statuses[-1]["text"] if statuses else None


# ── Pure-function coverage ────────────────────────────────────────────────────


def test_parse_extracts_columns():
    app = _load_app_module()
    assert app._parse("[2026-06-22 10:11:12] [INFO] [app::todo] ready") == {
        "time": "10:11:12",
        "level": "INFO",
        "target": "app::todo",
        "message": "ready",
    }


def test_parse_rejects_non_log_line():
    app = _load_app_module()
    assert app._parse("not a log line") is None


def test_parse_log_reverses_to_newest_first():
    app = _load_app_module()
    lines = app._parse_log(THREE_LINE_LOG)
    assert [ln["message"] for ln in lines] == [
        "retrying",
        "save failed",
        "loaded config",
    ]


def test_filter_by_level_and_target():
    app = _load_app_module()
    data = dict(
        app.DEFAULT_STATE,
        lines=[
            {"time": "1", "level": "ERROR", "target": "app::todo", "message": "boom"},
            {"time": "2", "level": "INFO", "target": "app::todo", "message": "ok"},
            {"time": "3", "level": "INFO", "target": "plexi::config", "message": "load"},
        ],
    )
    data["filter"] = "INFO"
    data["query"] = "app::todo"
    filtered = app._filtered(data)
    assert [ln["message"] for ln in filtered] == ["ok"]


def test_apply_log_result_success_populates_lines():
    app = _load_app_module()
    data = dict(app.DEFAULT_STATE)
    app._apply_log_result(
        data,
        HostLogResult(content=THREE_LINE_LOG.encode(), path="/home/u/.plexi-alpha/plexi.log", error=None),
    )
    assert data["loaded"] is True
    assert data["error"] is None
    assert data["path"] == "/home/u/.plexi-alpha/plexi.log"
    assert [ln["message"] for ln in data["lines"]] == [
        "retrying",
        "save failed",
        "loaded config",
    ]


def test_apply_log_result_error_surfaces_not_blank():
    app = _load_app_module()
    data = dict(app.DEFAULT_STATE)
    app._apply_log_result(
        data,
        HostLogResult(content=None, path="/home/u/.plexi-alpha/plexi.log", error="open /x: denied"),
    )
    assert data["error"] == "open /x: denied"
    assert data["lines"] == []
    assert data["loaded"] is True
    text = app._visible_text(data)
    assert "Cannot read host log" in text
    assert "open /x: denied" in text


def test_visible_text_shows_path_when_reachable_but_empty():
    app = _load_app_module()
    text = app._visible_text(
        {**app.DEFAULT_STATE, "path": "/home/u/.plexi-alpha/plexi.log", "loaded": True}
    )
    assert "No log entries" in text
    assert "/home/u/.plexi-alpha/plexi.log" in text


# ── End-to-end lifecycle through the read_host_log effect ─────────────────────


def test_boots_and_populates_from_host_log(tmp_path):
    log_path = tmp_path / "plexi.log"
    log_path.write_text(THREE_LINE_LOG)
    with AppHarness(str(APP), width=800, height=600, host_log_path=str(log_path)) as h:
        h.run(2)  # frame 1 loads, frame 2 renders the resolved tail
        assert _last_status(h) == "3 lines"
        trees = [c for c in h._events_seen if c.get("type") == "component_tree"]
        assert "save failed" in json.dumps(trees[-1])


def test_level_key_narrows_status_count(tmp_path):
    log_path = tmp_path / "plexi.log"
    log_path.write_text(THREE_LINE_LOG)
    with AppHarness(str(APP), width=800, height=600, host_log_path=str(log_path)) as h:
        h.run(2)
        assert _last_status(h) == "3 lines"
        h.key("e")  # ERROR filter
        h.run(1)
        assert _last_status(h) == "1 lines"


def test_level_tab_click_narrows_filter(tmp_path):
    log_path = tmp_path / "plexi.log"
    log_path.write_text(THREE_LINE_LOG)
    with AppHarness(str(APP), width=800, height=600, host_log_path=str(log_path)) as h:
        h.run(2)
        assert _last_status(h) == "3 lines"
        h.ui_action("logs-level:1")  # ERROR is index 1 in LEVELS
        h.run(1)
        assert _last_status(h) == "1 lines"


def test_unreachable_log_renders_error_not_blank():
    # No host_log_path → the harness replies with an error result, mirroring a
    # channel log the sandbox cannot reach.
    with AppHarness(str(APP), width=800, height=600) as h:
        h.run(2)
        assert _last_status(h) == "log unavailable"
        trees = [c for c in h._events_seen if c.get("type") == "component_tree"]
        assert trees, "app must still render a tree when the log is unreachable"
        assert "Cannot read host log" in json.dumps(trees[-1])


def test_renders_without_overlap(tmp_path):
    log_path = tmp_path / "plexi.log"
    log_path.write_text(THREE_LINE_LOG)
    with AppHarness(str(APP), width=900, height=600, host_log_path=str(log_path)) as h:
        h.run(2)
        h.assert_no_overlap()
