import importlib.util
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
APP = ROOT / "apps" / "logs" / "logs.py"


def _load_app_module():
    spec = importlib.util.spec_from_file_location("logs_app", APP)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def test_parse_log_line_extracts_columns():
    app = _load_app_module()

    line = app._parse("[2026-06-22 10:11:12] [INFO] [app::todo] ready")

    assert line == {
        "time": "10:11:12",
        "level": "INFO",
        "target": "app::todo",
        "message": "ready",
    }
