"""Tests for the Logs app.

Run with:  plexi app test          (from apps/logs)
       or:  uv run pytest tests/

Pure-function tests cover log parsing and channel/path resolution; the
AppHarness test drives the real init -> render lifecycle against a fixture
log and asserts the pane status surfaces the active channel + path.
"""

import importlib.util
import sys
from pathlib import Path

import pytest


ROOT = Path(__file__).resolve().parents[3]
SDK = ROOT / "sdk" / "python"
APP = ROOT / "apps" / "logs" / "logs.py"

sys.path.insert(0, str(SDK))

from plexi_sdk.testing import AppHarness  # noqa: E402


def _load_app_module():
    spec = importlib.util.spec_from_file_location("logs_app", APP)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


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


def test_detect_prefers_config_dir(monkeypatch):
    app = _load_app_module()
    monkeypatch.setenv("PLEXI_CONFIG_DIR", "/home/u/.plexi-alpha")
    monkeypatch.delenv("PLEXI_CHANNEL", raising=False)
    path, channel = app._detect()
    assert path == "/home/u/.plexi-alpha/plexi.log"
    assert channel == "alpha"


def test_detect_falls_back_to_channel_env(monkeypatch):
    app = _load_app_module()
    monkeypatch.delenv("PLEXI_CONFIG_DIR", raising=False)
    monkeypatch.setenv("PLEXI_CHANNEL", "beta")
    path, channel = app._detect()
    assert path.endswith("/.plexi-beta/plexi.log")
    assert channel == "beta"


def test_detect_default_channel(monkeypatch):
    app = _load_app_module()
    monkeypatch.setenv("PLEXI_CONFIG_DIR", "/home/u/.plexi")
    path, channel = app._detect()
    assert path == "/home/u/.plexi/plexi.log"
    assert channel == "default"


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
    data["level"] = "INFO"
    data["target"] = "app::todo"
    filtered = app._filtered(data)
    assert [ln["message"] for ln in filtered] == ["ok"]


def test_sort_order_reverses():
    app = _load_app_module()
    data = dict(
        app.DEFAULT_STATE,
        lines=[
            {"time": "3", "level": "INFO", "target": "x", "message": "c"},
            {"time": "2", "level": "INFO", "target": "x", "message": "b"},
            {"time": "1", "level": "INFO", "target": "x", "message": "a"},
        ],
    )
    assert [ln["time"] for ln in app._filtered(data)] == ["3", "2", "1"]
    data["order"] = "oldest"
    assert [ln["time"] for ln in app._filtered(data)] == ["1", "2", "3"]


@pytest.mark.parametrize("size", [(320, 240), (900, 600)])
def test_renders_without_overlap(size, tmp_path, monkeypatch):
    (tmp_path / "plexi.log").write_text(
        "[2026-07-02 01:46:18] [INFO] [plexi::config] loaded config\n"
        "[2026-07-02 01:46:19] [ERROR] [app::todo] save failed\n"
        "[2026-07-02 01:46:20] [WARN] [app::todo] retrying\n"
    )
    monkeypatch.setenv("PLEXI_CONFIG_DIR", str(tmp_path))
    width, height = size
    with AppHarness(str(APP), width=width, height=height) as h:
        h.run(1)
        h.assert_no_overlap()


def test_level_key_narrows_status_count(tmp_path, monkeypatch):
    config_dir = tmp_path / ".plexi-alpha"
    config_dir.mkdir()
    (config_dir / "plexi.log").write_text(
        "[2026-07-02 01:46:18] [INFO] [plexi::config] loaded config\n"
        "[2026-07-02 01:46:19] [ERROR] [app::todo] save failed\n"
        "[2026-07-02 01:46:20] [WARN] [app::todo] retrying\n"
    )
    monkeypatch.setenv("PLEXI_CONFIG_DIR", str(config_dir))

    def _last_status(h):
        return [c for c in h._events_seen if c.get("type") == "status_summary"][-1]["text"]

    with AppHarness(str(APP), width=800, height=600) as h:
        h.run(1)
        assert "3 lines" in _last_status(h)
        h.key("e")  # ERROR filter
        h.run(1)
        assert "1 lines" in _last_status(h)


def test_status_surfaces_channel_and_path(tmp_path, monkeypatch):
    config_dir = tmp_path / ".plexi-alpha"
    config_dir.mkdir()
    log_path = config_dir / "plexi.log"
    log_path.write_text("[2026-07-02 01:46:18] [INFO] [plexi::config] loaded config\n")
    monkeypatch.setenv("PLEXI_CONFIG_DIR", str(config_dir))
    monkeypatch.delenv("PLEXI_CHANNEL", raising=False)
    with AppHarness(str(APP), width=800, height=600) as h:
        h.run(1)
        statuses = [c for c in h._events_seen if c.get("type") == "status_summary"]
        assert statuses, f"no status_summary emitted; saw {[c.get('type') for c in h._events_seen]}"
        text = statuses[-1].get("text", "")
        assert str(log_path) in text
        assert "alpha" in text
