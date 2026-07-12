import importlib.util
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]
SDK = ROOT / "sdk" / "python"
APP = ROOT / "apps" / "csv_viewer" / "csv_viewer.py"

sys.path.insert(0, str(SDK))


def _load_app_module():
    spec = importlib.util.spec_from_file_location("csv_viewer_app", APP)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def test_open_csv_reads_headers_and_rows(tmp_path):
    app = _load_app_module()
    csv_path = tmp_path / "sample.csv"
    csv_path.write_text("name,count\nalpha,1\nbeta,2\n")

    data = app._open_csv(csv_path)

    assert data["headers"] == ["name", "count"]
    assert data["rows"] == [["alpha", "1"], ["beta", "2"]]
    assert data["error"] == ""
