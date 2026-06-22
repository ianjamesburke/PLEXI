import base64
import json
import sys
from pathlib import Path

import pytest
from dataclasses import dataclass

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import plexi_sdk as sdk
from plexi_sdk import _v3_state
from plexi_sdk import state
from plexi_sdk._adapter import _decode_event, _encode_effect, _encode_uitree
from plexi_sdk._adapter import call_lifecycle, load_app
from plexi_sdk.effects import SetState, SetTitle
from plexi_sdk.events import KeyEvent, PipeMessage, PipePayload, SystemStatsResult
from plexi_sdk.ui import Button, Canvas, CanvasRect, Column, SelectList, Text, TextInput


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
    assert tree["nodes"][2]["data"]["type"] == "Button"


def test_ui_tree_serializes_text_input_and_select_list() -> None:
    tree = _encode_uitree(
        Column(
            [
                TextInput(
                    "todo-add",
                    value="draft",
                    placeholder="New item",
                    on_change="todo-add:change",
                    on_submit="todo-add:submit",
                ),
                SelectList(
                    [{"name": "one"}, {"name": "two", "description": "done"}],
                    selected_idx=1,
                ),
            ],
            padding=0,
        )
    )

    nodes = tree["nodes"]
    assert nodes[1]["data"] == {
        "type": "TextInput",
        "value": "draft",
        "placeholder": "New item",
        "on_change": "todo-add:change",
        "on_submit": "todo-add:submit",
        "password": False,
    }
    assert nodes[2]["data"]["type"] == "ListView"
    assert nodes[2]["data"]["selected"] == 1
    assert nodes[2]["data"]["items"] == [3, 4]


def test_ui_tree_serializes_canvas_commands() -> None:
    tree = _encode_uitree(
        Canvas(
            [
                CanvasRect(1.0, 2.0, 30.0, 40.0, "#112233", radius=2.0),
            ],
            width=320.0,
            height=180.0,
        )
    )

    node = tree["nodes"][0]["data"]
    assert node["type"] == "canvas"
    assert node["width"] == 320.0
    assert node["height"] == 180.0
    assert node["commands"] == [
        {
            "type": "rect",
            "x": 1.0,
            "y": 2.0,
            "width": 30.0,
            "height": 40.0,
            "fill": "#112233",
            "radius": 2.0,
        }
    ]


def test_effects_reject_unknown_dataclass() -> None:
    @dataclass
    class NotAnEffect:
        value: int

    with pytest.raises(TypeError, match="Unknown effect type"):
        _encode_effect(NotAnEffect(1))


def test_events_decode_nested_payloads() -> None:
    stats = _decode_event(
        {
            "type": "SystemStatsResult",
            "stats": {
                "cpu_usage_pct": 1.0,
                "memory_used_bytes": 2,
                "memory_total_bytes": 3,
                "disk_read_bps": 4,
                "disk_write_bps": 5,
                "net_rx_bps": 6,
                "net_tx_bps": 7,
                "uptime_secs": 8,
                "load_avg_one_min": 9.0,
            },
        }
    )
    pipe = _decode_event(
        {"type": "PipeMessage", "handle": 1, "payload": {"json": "{}"}}
    )

    assert isinstance(stats, SystemStatsResult)
    assert stats.stats.memory_total_bytes == 3
    assert isinstance(pipe, PipeMessage)
    assert pipe.payload == PipePayload(json="{}")


def test_state_proxy_reads_snapshot_and_set_returns_effect() -> None:
    payload = base64.b64encode(json.dumps(3).encode()).decode()
    _v3_state._state = sdk.StateSnapshot(
        {"count": 3}, {"count": json.dumps(3).encode()}
    )
    _v3_state._in_view = False
    assert state.get("count") == 3
    assert state.get("missing", 9) == 9
    assert state.set("count", 4) == SetState({"count": 4})

    _v3_state._in_view = True
    with pytest.raises(RuntimeError, match="inside view"):
        state.set("count", 5)
    _v3_state._in_view = False
    assert payload


def test_view_lifecycle_resets_view_guard(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    app_file = tmp_path / "sample_app.py"
    app_file.write_text(
        "from plexi_sdk.ui import Text\n" "def view():\n" "    return Text('ok')\n"
    )
    monkeypatch.syspath_prepend(str(tmp_path))

    load_app("sample_app")
    call_lifecycle("view", json.dumps({"state": {}}))

    assert _v3_state._in_view is False
