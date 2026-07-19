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
from plexi_sdk.effects import (
    AiTool,
    ExposeTools,
    HttpFetch,
    SetState,
    SetTitle,
    SubscribeEventStreams,
    ToolResult,
    UnsubscribeEventStreams,
)
from plexi_sdk.events import (
    HttpResponse,
    KeyEvent,
    PipeMessage,
    PipePayload,
    SystemStatsResult,
    ToolCall,
)
from plexi_sdk.ui import (
    Button,
    Canvas,
    CanvasRect,
    Column,
    FooterKeys,
    Scrollable,
    SelectList,
    Text,
    TextInput,
)


def test_effects_encode_dataclass_shape() -> None:
    assert _encode_effect(SetTitle("hello")) == {"type": "SetTitle", "title": "hello"}


def test_effects_encode_http_fetch_bytes_as_json_list() -> None:
    assert _encode_effect(HttpFetch("https://example.test", body=b"ok")) == {
        "type": "HttpFetch",
        "url": "https://example.test",
        "method": "GET",
        "headers": {},
        "body": [111, 107],
    }


def test_tool_effects_encode_rust_wire_shape() -> None:
    tool = AiTool(
        name="csv.describe_table",
        description="Describe the current table.",
        input_schema={"type": "object", "properties": {}},
        output_schema={"type": "object", "properties": {"rows": {"type": "integer"}}},
        timeout_ms=1_500,
        read_only=True,
    )

    assert _encode_effect(ExposeTools([tool])) == {
        "type": "ExposeTools",
        "tools": [{
            "name": "csv.describe_table",
            "description": "Describe the current table.",
            "input_schema": {"type": "object", "properties": {}},
            "output_schema": {"type": "object", "properties": {"rows": {"type": "integer"}}},
            "timeout_ms": 1_500,
            "read_only": True,
        }],
    }


def test_subscription_effects_encode_rust_wire_shape() -> None:
    assert _encode_effect(SubscribeEventStreams(
        request_id="subscribe-1",
        app_id="notes",
        event_names=["note.saved"],
    )) == {
        "type": "SubscribeEventStreams",
        "request_id": "subscribe-1",
        "app_id": "notes",
        "event_names": ["note.saved"],
        "payload_mode": "full",
        "trigger_mode": "conversation",
        "resource_id": None,
    }
    assert _encode_effect(UnsubscribeEventStreams("unsubscribe-1", "sub-1")) == {
        "type": "UnsubscribeEventStreams",
        "request_id": "unsubscribe-1",
        "subscription_id": "sub-1",
    }
    assert _encode_effect(ToolResult("call-7", output_json='{"rows":3}')) == {
        "type": "ToolResult",
        "call_id": "call-7",
        "output_json": '{"rows":3}',
        "error": None,
    }


def test_events_decode_tool_call_rust_wire_shape() -> None:
    event = _decode_event({
        "type": "ToolCall",
        "call_id": "call-7",
        "name": "csv.describe_table",
        "input_json": "{}",
        "caller_id": "assistant",
    })

    assert event == ToolCall(
        call_id="call-7",
        name="csv.describe_table",
        input_json="{}",
        caller_id="assistant",
    )


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
    assert tree["nodes"][1]["data"]["type"] == "text"
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
    assert "size" not in nodes[5]["data"]


def test_text_input_handlers_default_to_its_id() -> None:
    # Stint 0456: the natural app pattern (`event.handler_id == "<input-id>"`)
    # must work without wiring on_change/on_submit explicitly — typing
    # delivers UiValueChange(handler_id=id), Enter delivers UiAction(id).
    node = TextInput("guess", value="42").to_node()
    assert node["on_change"] == "guess"
    assert node["on_submit"] == "guess"


def test_ui_tree_flattens_scrollable_child() -> None:
    tree = _encode_uitree(Scrollable(Text("log line")))

    assert tree["nodes"][0]["data"] == {
        "type": "Scroll",
        "child": 1,
        "horizontal": False,
    }
    assert tree["nodes"][1]["data"]["text"] == "log line"


def test_ui_tree_serializes_row_children() -> None:
    tree = _encode_uitree(
        {
            "type": "row",
            "children": [Button("One", "one"), Button("Two", "two")],
            "gap": 8.0,
        }
    )

    nodes = tree["nodes"]
    assert nodes[0]["data"] == {
        "type": "Row",
        "children": [1, 2],
        "gap": 8.0,
        "align": "start",
        "grow": False,
    }
    assert nodes[1]["data"]["type"] == "Button"
    assert nodes[2]["data"]["type"] == "Button"


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
    assert node["fit"] == "fill"
    assert node["commands"] == [
        {
            "type": "rect",
            "x": 1.0,
            "y": 2.0,
            "w": 30.0,
            "h": 40.0,
            "fill": "#112233",
            "radius": 2.0,
        }
    ]


def test_canvas_contain_fit_is_explicit_and_validated() -> None:
    tree = _encode_uitree(Canvas([], fit="contain"))
    assert tree["nodes"][0]["data"]["fit"] == "contain"

    with pytest.raises(ValueError, match="fit must be"):
        Canvas([], fit="stretch")


def test_ui_tree_passes_pinned_footer_keys_through_as_wit_nodes() -> None:
    """Stint 0389: pinned/footer_keys are live WIT node kinds — the SDK must
    pass them through (flattening the pinned child into the arena) rather
    than downgrading to a flattened Text node."""
    tree = _encode_uitree(FooterKeys([("j", "down"), (["g", "G"], "ends")]))

    root = tree["nodes"][tree["root"]]
    assert root["data"]["type"] == "Pinned"
    assert root["data"]["edge"] == "bottom"

    child = tree["nodes"][root["data"]["child"]]
    assert child["data"] == {
        "type": "FooterKeys",
        "entries": [
            {"keys": ["j"], "description": "down"},
            {"keys": ["g", "G"], "description": "ends"},
        ],
        "divider": True,
    }


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
    http = _decode_event(
        {"type": "HttpResponse", "status": 200, "headers": [], "body": [111, 107]}
    )

    assert isinstance(stats, SystemStatsResult)
    assert stats.stats.memory_total_bytes == 3
    assert isinstance(pipe, PipeMessage)
    assert pipe.payload == PipePayload(json="{}")
    assert isinstance(http, HttpResponse)
    assert http.body == b"ok"


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


def test_encode_uitree_rejects_view_returning_effect_tuple() -> None:
    from plexi_sdk.effects import SetTimer
    from plexi_sdk.ui import Text

    with pytest.raises(TypeError, match=r"view\(\) must return a single component tree"):
        _encode_uitree((Text("hi"), SetTimer(id=1, delay_ms=500)))

    with pytest.raises(TypeError, match=r"view\(\) returned the effect SetTimer"):
        _encode_uitree(SetTimer(id=1, delay_ms=500))
