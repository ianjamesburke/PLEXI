import importlib.util
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]
SDK = ROOT / "sdk" / "python"
APP = ROOT / "apps" / "csv_viewer" / "csv_viewer.py"

sys.path.insert(0, str(SDK))

import plexi_sdk as sdk  # noqa: E402
from plexi_sdk import _v3_state  # noqa: E402
from plexi_sdk.effects import FileList, FileRead, RequestCapability, SetState  # noqa: E402
from plexi_sdk.events import CapabilityGranted, FileListEntry, FileListResult, FileReadResult  # noqa: E402


def _load_app_module():
    spec = importlib.util.spec_from_file_location("csv_viewer_app", APP)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def _set_state(values: dict) -> None:
    _v3_state._state = sdk.StateSnapshot(values, {})
    _v3_state._in_view = False


def _state_effect(effects: list) -> dict:
    return next(effect.data for effect in effects if isinstance(effect, SetState))


def test_init_requests_host_file_list_for_directory(tmp_path):
    app = _load_app_module()
    _set_state({})

    effects = app.init((500, 400), [str(tmp_path)])

    assert any(isinstance(effect, RequestCapability) and effect.name == "fs.read" for effect in effects)
    data = _state_effect(effects)
    assert data["pending_action"] == "list"

    _set_state(data)
    effects = app.update(CapabilityGranted("fs.read"))
    assert any(isinstance(effect, FileList) and effect.path == str(tmp_path) for effect in effects)


def test_file_list_result_populates_selectable_csv_rows(tmp_path):
    app = _load_app_module()
    data = dict(app.DEFAULT_STATE)
    data["pending_path"] = str(tmp_path)
    data["pending_action"] = "list"
    _set_state(data)

    effects = app.update(
        FileListResult(
            entries=[
                FileListEntry("sample.csv", str(tmp_path / "sample.csv"), False, 16),
                FileListEntry("notes.txt", str(tmp_path / "notes.txt"), False, 12),
            ],
            error=None,
        )
    )

    listed = _state_effect(effects)
    assert listed["files"] == [
        {"name": "sample.csv", "path": str(tmp_path / "sample.csv"), "description": "0.0 KB"}
    ]


def test_open_csv_uses_file_read_result():
    app = _load_app_module()
    data = dict(app.DEFAULT_STATE)
    data["pending_path"] = "/tmp/sample.csv"
    data["pending_action"] = "read"
    _set_state(data)

    effects = app.update(FileReadResult(content=b"name,count\nalpha,1\nbeta,2\n", error=None))
    opened = _state_effect(effects)

    assert opened["headers"] == ["name", "count"]
    assert opened["rows"] == [["alpha", "1"], ["beta", "2"]]
    assert opened["mode"] == "detail"


def test_enter_on_selected_file_requests_file_read(tmp_path):
    app = _load_app_module()
    path = tmp_path / "sample.csv"
    data = dict(app.DEFAULT_STATE)
    data["files"] = [{"name": "sample.csv", "path": str(path), "description": ""}]
    _set_state(data)

    effects = app.update(sdk.events.KeyEvent("return"))
    requested = _state_effect(effects)
    assert requested["pending_action"] == "read"

    _set_state(requested)
    effects = app.update(CapabilityGranted("fs.read"))
    assert any(isinstance(effect, FileRead) and effect.path == str(path) for effect in effects)


def test_detail_view_uses_native_table_inside_scroll():
    app = _load_app_module()
    data = dict(app.DEFAULT_STATE)
    data.update(
        {
            "mode": "detail",
            "path": "/tmp/sample.csv",
            "headers": ["name", "count"],
            "rows": [["alpha", "1"], ["beta", "2"]],
        }
    )
    _set_state(data)

    node = app.view().to_node()
    scroll = next(child for child in node["children"] if child["type"] == "scroll")

    assert scroll["child"]["type"] == "table"
    assert scroll["child"]["columns"] == ["name", "count"]
    assert scroll["child"]["rows"] == [["alpha", "1"], ["beta", "2"]]
