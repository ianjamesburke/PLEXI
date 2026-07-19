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
    text = app._placeholder_text(data)
    assert "Cannot read host log" in text
    assert "open /x: denied" in text


def test_placeholder_text_shows_path_when_reachable_but_empty():
    app = _load_app_module()
    text = app._placeholder_text(
        {**app.DEFAULT_STATE, "path": "/home/u/.plexi-alpha/plexi.log", "loaded": True}
    )
    assert "No log entries" in text
    assert "/home/u/.plexi-alpha/plexi.log" in text


# ── Row rendering ─────────────────────────────────────────────────────────────


def test_parseable_row_carries_level_colored_badge():
    app = _load_app_module()
    row = app._row(
        {"time": "10:11:12", "level": "ERROR", "target": "app::todo", "message": "boom"}
    ).to_node()
    badges = [c for c in row["children"] if c.get("type") == "badge"]
    assert len(badges) == 1
    assert badges[0]["text"] == "ERROR"
    assert badges[0]["color"] == "danger"
    texts = [c["text"] for c in row["children"] if c.get("type") == "text"]
    assert texts == ["10:11:12", "app::todo", "boom"]


def test_each_level_gets_a_distinct_badge_color():
    app = _load_app_module()
    colors = {lvl: app.LEVEL_COLOR[lvl] for lvl in ("ERROR", "WARN", "INFO", "DEBUG")}
    assert len(set(colors.values())) == 4


def test_unparseable_line_kept_as_full_width_message_row():
    app = _load_app_module()
    lines = app._parse_log("this is not a log line\n")
    assert lines == [{"time": "", "level": "", "target": "", "message": "this is not a log line"}]
    row = app._row(lines[0]).to_node()
    assert not [c for c in row["children"] if c.get("type") == "badge"]
    assert [c["text"] for c in row["children"] if c.get("type") == "text"] == [
        "this is not a log line"
    ]


# ── Follow-freeze ─────────────────────────────────────────────────────────────


def _newer_log():
    return (
        "[2026-07-02 01:46:18] [INFO] [plexi::config] loaded config\n"
        "[2026-07-02 01:46:19] [ERROR] [app::todo] save failed\n"
        "[2026-07-02 01:46:20] [WARN] [app::todo] retrying\n"
        "[2026-07-02 01:46:21] [INFO] [app::todo] recovered\n"
    )


def test_follow_off_freezes_tail_dropping_new_results():
    app = _load_app_module()
    data = dict(app.DEFAULT_STATE)
    app._apply_log_result(
        data, HostLogResult(content=THREE_LINE_LOG.encode(), path="/x/plexi.log", error=None)
    )
    frozen = [dict(ln) for ln in data["lines"]]
    data["follow"] = False
    effects = app._apply_log_result(
        data, HostLogResult(content=_newer_log().encode(), path="/x/plexi.log", error=None)
    )
    assert effects == []  # fresh tail dropped
    assert data["lines"] == frozen  # visible tail unchanged


def test_forced_refresh_bypasses_freeze_once():
    app = _load_app_module()
    data = dict(app.DEFAULT_STATE)
    app._apply_log_result(
        data, HostLogResult(content=THREE_LINE_LOG.encode(), path="/x/plexi.log", error=None)
    )
    data["follow"] = False
    data["pending_force"] = True  # what pressing `r` sets
    app._apply_log_result(
        data, HostLogResult(content=_newer_log().encode(), path="/x/plexi.log", error=None)
    )
    assert data["pending_force"] is False  # consumed
    assert "recovered" in [ln["message"] for ln in data["lines"]]


def test_follow_on_keeps_applying_new_results():
    app = _load_app_module()
    data = dict(app.DEFAULT_STATE)  # follow defaults True
    app._apply_log_result(
        data, HostLogResult(content=THREE_LINE_LOG.encode(), path="/x/plexi.log", error=None)
    )
    app._apply_log_result(
        data, HostLogResult(content=_newer_log().encode(), path="/x/plexi.log", error=None)
    )
    assert "recovered" in [ln["message"] for ln in data["lines"]]


def test_timer_poll_suppressed_while_frozen():
    app = _load_app_module()
    from plexi_sdk import _v3_state
    from plexi_sdk._v3_state import StateSnapshot
    from plexi_sdk.effects import ReadHostLog
    from plexi_sdk.events import TimerFired

    # follow off → no ReadHostLog effect on the poll tick (tail is frozen).
    _v3_state._state = StateSnapshot({**app.DEFAULT_STATE, "follow": False, "loaded": True}, {})
    assert app.update(TimerFired(id=app.TIMER_ID)) == []
    # follow on → the poll re-reads the tail.
    _v3_state._state = StateSnapshot({**app.DEFAULT_STATE, "follow": True, "loaded": True}, {})
    effects = app.update(TimerFired(id=app.TIMER_ID))
    assert any(isinstance(e, ReadHostLog) for e in effects)


# ── End-to-end lifecycle through the read_host_log effect ─────────────────────


def test_boots_and_populates_from_host_log(tmp_path):
    log_path = tmp_path / "plexi.log"
    log_path.write_text(THREE_LINE_LOG)
    with AppHarness(str(APP), width=800, height=600, host_log_path=str(log_path)) as h:
        h.run(2)  # frame 1 loads, frame 2 renders the resolved tail
        assert _last_status(h) == "3 lines"
        trees = [c for c in h._events_seen if c.get("type") == "component_tree"]
        assert "save failed" in json.dumps(trees[-1])


def test_boots_renders_colored_level_badges(tmp_path):
    log_path = tmp_path / "plexi.log"
    log_path.write_text(THREE_LINE_LOG)
    with AppHarness(str(APP), width=800, height=600, host_log_path=str(log_path)) as h:
        h.run(2)
        trees = [c for c in h._events_seen if c.get("type") == "component_tree"]
        blob = json.dumps(trees[-1])
        assert '"badge"' in blob
        assert '"danger"' in blob  # ERROR row badge
        assert '"warning"' in blob  # WARN row badge


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


def test_search_replaces_level_tabs_instead_of_stacking(tmp_path):
    # Stint 0463: search must swap in for the level TabBar row rather than
    # adding a third stacked header row (AppBar + TabBar + TextInput), which
    # read as a heavy inset / "missing header" vs Snake.
    log_path = tmp_path / "plexi.log"
    log_path.write_text(THREE_LINE_LOG)
    with AppHarness(str(APP), width=800, height=600, host_log_path=str(log_path)) as h:
        h.run(2)
        before = json.dumps(h._events_seen)
        assert "logs-level:0" in before, "TabBar present before search"
        assert '"TextInput"' not in before

        h.key("/")
        h.run(1)
        trees = [c for c in h._events_seen if c.get("type") == "component_tree"]
        latest = json.dumps(trees[-1])
        assert '"TextInput"' in latest, "search input must replace the TabBar row"
        assert "logs-level:0" not in latest, "TabBar must not stack alongside search"


def test_renders_without_overlap(tmp_path):
    log_path = tmp_path / "plexi.log"
    log_path.write_text(THREE_LINE_LOG)
    with AppHarness(str(APP), width=900, height=600, host_log_path=str(log_path)) as h:
        h.run(2)
        h.assert_no_overlap()
