import base64
import json
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import plexi_sdk as sdk
from plexi_sdk import _v3_state
from plexi_sdk import state
from plexi_sdk._adapter import _decode_event, _encode_effect, _encode_uitree
from plexi_sdk._adapter import call_lifecycle, load_app
from plexi_sdk.effects import SetState, SetTitle
from plexi_sdk.events import KeyEvent
from plexi_sdk.ui import Button, Column, Text


def test_effects_encode_dataclass_shape() -> None:
    assert _encode_effect(SetTitle("hello")) == {"type": "SetTitle", "title": "hello"}


def test_events_decode_key_event_with_modifiers() -> None:
    event = _decode_event({"type": "KeyEvent", "key": "q", "modifiers": {"meta": True}})
    assert isinstance(event, KeyEvent)
    assert event.key == "q"
    assert event.modifiers.meta is True


def test_ui_tree_flattens_v3_components() -> None:
    tree = _encode_uitree(Column([Text("hi"), Button("Run", "run")], padding=0))
    assert tree["root"] == 0
    assert len(tree["nodes"]) == 3
    assert tree["nodes"][0]["data"]["type"] == "Column"
    assert tree["nodes"][1]["data"]["type"] == "Text"
    assert tree["nodes"][2]["data"]["type"] == "button"


def test_state_proxy_reads_snapshot_and_set_returns_effect() -> None:
    payload = base64.b64encode(json.dumps(3).encode()).decode()
    _v3_state._state = sdk.StateSnapshot({"count": 3}, {"count": json.dumps(3).encode()})
    _v3_state._in_view = False
    assert state.get("count") == 3
    assert state.get("missing", 9) == 9
    assert state.set("count", 4) == SetState({"count": 4})

    _v3_state._in_view = True
    with pytest.raises(RuntimeError, match="inside view"):
        state.set("count", 5)
    _v3_state._in_view = False
    assert payload


def test_view_lifecycle_resets_view_guard(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    app_file = tmp_path / "sample_app.py"
    app_file.write_text(
        "from plexi_sdk.ui import Text\n"
        "def view():\n"
        "    return Text('ok')\n"
    )
    monkeypatch.syspath_prepend(str(tmp_path))

    load_app("sample_app")
    call_lifecycle("view", json.dumps({"state": {}}))

    assert _v3_state._in_view is False
