import importlib.util
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]
SDK = ROOT / "sdk" / "python"
APP = ROOT / "apps" / "logs" / "logs.py"

sys.path.insert(0, str(SDK))


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
