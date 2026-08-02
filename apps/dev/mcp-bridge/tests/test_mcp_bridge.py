"""MCP bridge tests, driven entirely by real captured server traffic.

Every `tools/list` body under `src/host/testdata/mcp/` was captured from a real
MCP server over stdio. Nothing here invents a response shape: a fixture built
from an assumed schema would validate the assumption instead of the protocol.
"""

from __future__ import annotations

import json
import os
import sys

sys.path.insert(
    0,
    os.path.join(os.path.dirname(__file__), "..", "..", "..", "..", "sdk", "python"),
)
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import plexi_sdk as sdk  # noqa: E402
from plexi_sdk import _v3_state  # noqa: E402
from plexi_sdk.effects import (  # noqa: E402
    CancelTimer,
    ExposeTools,
    McpConnect,
    McpDisconnect,
    McpSend,
    SetState,
    SetTimer,
    ToolResult,
)
from plexi_sdk.events import (  # noqa: E402
    KeyEvent,
    McpClosed,
    McpConnected,
    McpMessage,
    TimerFired,
    ToolCall,
)

import mcp_bridge  # noqa: E402

FIXTURES = os.path.join(
    os.path.dirname(__file__), "..", "..", "..", "..", "src", "host", "testdata", "mcp"
)


def _captured(name: str) -> dict:
    with open(os.path.join(FIXTURES, name), encoding="utf-8") as handle:
        return json.load(handle)


def _set_state(values: dict) -> None:
    raw = {key: b"" for key in values}
    _v3_state._state = sdk.StateSnapshot(values, raw)
    _v3_state._in_view = False


def _apply(effects: list) -> dict:
    """Fold every SetState in `effects` into live state, as the host would."""
    data: dict = dict(_v3_state._state.values) if _v3_state._state else {}
    for effect in effects:
        if isinstance(effect, SetState):
            data.update(effect.data)
    _set_state(data)
    return data


def _of(effects: list, kind) -> list:
    return [effect for effect in effects if isinstance(effect, kind)]


def _drive_to_ready(server_id: str = "filesystem") -> list:
    """Run the full connect → initialize → tools/list handshake."""
    _apply(mcp_bridge.init((800.0, 600.0), [server_id]))
    _apply(mcp_bridge.update(McpConnected(request_id=f"connect:{server_id}", server_id=server_id, error=None)))
    initialize = _captured("filesystem_initialize.json")
    _apply(
        mcp_bridge.update(
            McpMessage(server_id=server_id, message=initialize, raw=json.dumps(initialize))
        )
    )
    tools_list = _captured("filesystem_tools_list.json")
    effects = mcp_bridge.update(
        McpMessage(server_id=server_id, message=tools_list, raw=json.dumps(tools_list))
    )
    _apply(effects)
    return effects


# ── Handshake ────────────────────────────────────────────────────────────────


def test_init_connects_to_each_named_server() -> None:
    effects = mcp_bridge.init((800.0, 600.0), ["filesystem", "git"])
    connects = _of(effects, McpConnect)
    assert [c.server_id for c in connects] == ["filesystem", "git"]


def test_init_without_a_server_id_fails_visibly() -> None:
    effects = mcp_bridge.init((800.0, 600.0), [])
    data = _apply(effects)
    assert not _of(effects, McpConnect)
    assert "mcp_servers.toml" in data["fatal"]


def test_connect_then_initialize_then_tools_list() -> None:
    _apply(mcp_bridge.init((800.0, 600.0), ["filesystem"]))

    effects = mcp_bridge.update(
        McpConnected(request_id="connect:filesystem", server_id="filesystem", error=None)
    )
    _apply(effects)
    sends = _of(effects, McpSend)
    assert len(sends) == 1
    assert sends[0].message["method"] == "initialize"
    assert sends[0].message["params"]["protocolVersion"] == mcp_bridge.PROTOCOL_VERSION

    initialize = _captured("filesystem_initialize.json")
    effects = mcp_bridge.update(
        McpMessage(server_id="filesystem", message=initialize, raw=json.dumps(initialize))
    )
    _apply(effects)
    methods = [send.message["method"] for send in _of(effects, McpSend)]
    assert methods == ["notifications/initialized", "tools/list"]


def test_failed_connect_records_the_reason_and_sends_nothing() -> None:
    _apply(mcp_bridge.init((800.0, 600.0), ["filesystem"]))
    effects = mcp_bridge.update(
        McpConnected(
            request_id="connect:filesystem",
            server_id="filesystem",
            error="no MCP server named 'filesystem' in mcp_servers.toml (configured: none)",
        )
    )
    data = _apply(effects)
    assert not _of(effects, McpSend)
    assert data["servers"]["filesystem"]["status"] == "failed"
    assert "mcp_servers.toml" in data["servers"]["filesystem"]["error"]


# ── Schema translation ───────────────────────────────────────────────────────


def test_real_filesystem_tools_are_exposed_and_attributed() -> None:
    effects = _drive_to_ready()
    declarations = _of(effects, ExposeTools)
    assert len(declarations) == 1
    tools = declarations[0].tools

    captured = _captured("filesystem_tools_list.json")["result"]["tools"]
    assert len(tools) == len(captured), "every real filesystem tool must translate"

    names = {tool.name for tool in tools}
    assert "mcp.filesystem.read_text_file" in names
    assert "mcp.filesystem.write_file" in names
    # Every name carries its server, so two servers exposing `read_file` are
    # two separately-approvable tools.
    assert all(name.startswith("mcp.filesystem.") for name in names)


def test_no_bridged_tool_is_ever_declared_read_only() -> None:
    """The everything server advertises `readOnlyHint: true` on most tools.

    Honouring an external server's mutability claim would let a less-trusted
    tool source turn the host's default Ask into an automatic allow. Every
    bridged tool is ask-gated regardless of what the server says.
    """
    captured = _captured("everything_tools_list.json")["result"]["tools"]
    hinted = [t for t in captured if t.get("annotations", {}).get("readOnlyHint") is True]
    assert hinted, "fixture must actually carry readOnlyHint to make this meaningful"

    translated, _ = mcp_bridge.translate_tools("everything", captured)
    assert translated
    assert all(tool.read_only is False for tool in translated)


def test_description_is_attributed_to_the_server() -> None:
    captured = _captured("filesystem_tools_list.json")["result"]["tools"]
    translated, _ = mcp_bridge.translate_tools("filesystem", captured)
    assert all(tool.description.startswith("[MCP server 'filesystem'] ") for tool in translated)


def test_draft_schema_key_is_stripped_but_constraints_survive() -> None:
    captured = _captured("filesystem_tools_list.json")["result"]["tools"]
    assert any("$schema" in t["inputSchema"] for t in captured), "fixture must carry $schema"

    translated, _ = mcp_bridge.translate_tools("filesystem", captured)
    read_multiple = next(t for t in translated if t.name.endswith("read_multiple_files"))
    assert "$schema" not in read_multiple.input_schema
    assert read_multiple.input_schema["type"] == "object"
    assert read_multiple.input_schema["properties"]["paths"]["type"] == "array"
    assert read_multiple.input_schema["required"] == ["paths"]


def test_declared_output_schema_is_preserved() -> None:
    captured = _captured("everything_tools_list.json")["result"]["tools"]
    declared = next(t for t in captured if isinstance(t.get("outputSchema"), dict))
    translated, _ = mcp_bridge.translate_tools("everything", [declared])
    assert translated[0].output_schema["type"] == "object"
    assert "$schema" not in translated[0].output_schema


def test_tool_without_output_schema_declares_the_mcp_content_envelope() -> None:
    # Most of the everything server's tools declare no outputSchema, so the
    # bridge describes MCP's real reply envelope rather than claiming a shape
    # the server never promised.
    captured = _captured("everything_tools_list.json")["result"]["tools"]
    undeclared = [t for t in captured if not isinstance(t.get("outputSchema"), dict)]
    assert undeclared, "fixture must carry a tool with no outputSchema"

    translated, _ = mcp_bridge.translate_tools("everything", undeclared)
    schema = translated[0].output_schema
    assert schema["properties"]["content"]["type"] == "array"
    assert schema["properties"]["isError"]["type"] == "boolean"


def test_untranslatable_tools_are_skipped_loudly_never_dropped() -> None:
    real = _captured("filesystem_tools_list.json")["result"]["tools"][0]
    unmappable = [
        {"name": "no_schema", "description": "d"},
        {"name": "wrong_type", "description": "d", "inputSchema": {"type": "string"}},
        {"name": "bad_props", "description": "d", "inputSchema": {"type": "object", "properties": []}},
        {"description": "nameless", "inputSchema": {"type": "object", "properties": {}}},
        real,
        dict(real),  # duplicate name
    ]
    translated, skipped = mcp_bridge.translate_tools("mixed", unmappable)

    assert [tool.name for tool in translated] == [f"mcp.mixed.{real['name']}"]
    skipped_names = [name for name, _ in skipped]
    assert skipped_names == ["no_schema", "wrong_type", "bad_props", "None", real["name"]]
    # Every skip carries a reason — a silent drop is the failure mode this guards.
    assert all(reason for _, reason in skipped)
    assert "requires a top-level object" in dict(skipped)["wrong_type"]


# ── Tool proxying ────────────────────────────────────────────────────────────


def test_tool_call_is_proxied_as_tools_call() -> None:
    _drive_to_ready()
    effects = mcp_bridge.update(
        ToolCall(
            call_id="call-1",
            name="mcp.filesystem.read_text_file",
            input_json=json.dumps({"path": "/tmp/x.txt"}),
            caller_id="agent:assistant",
        )
    )
    _apply(effects)
    sends = _of(effects, McpSend)
    assert len(sends) == 1
    assert sends[0].message["method"] == "tools/call"
    assert sends[0].message["params"]["name"] == "read_text_file"
    assert sends[0].message["params"]["arguments"] == {"path": "/tmp/x.txt"}


def test_tool_result_is_returned_against_the_originating_call_id() -> None:
    _drive_to_ready()
    effects = mcp_bridge.update(
        ToolCall(
            call_id="call-7",
            name="mcp.filesystem.read_text_file",
            input_json=json.dumps({"path": "/tmp/x.txt"}),
            caller_id="agent:assistant",
        )
    )
    _apply(effects)
    request_id = _of(effects, McpSend)[0].message["id"]

    reply = {
        "jsonrpc": "2.0",
        "id": request_id,
        "result": {"content": [{"type": "text", "text": "hello"}]},
    }
    effects = mcp_bridge.update(
        McpMessage(server_id="filesystem", message=reply, raw=json.dumps(reply))
    )
    _apply(effects)
    results = _of(effects, ToolResult)
    assert len(results) == 1
    assert results[0].call_id == "call-7"
    assert json.loads(results[0].output_json)["content"][0]["text"] == "hello"


def test_jsonrpc_error_fails_the_call_rather_than_stranding_it() -> None:
    _drive_to_ready()
    effects = mcp_bridge.update(
        ToolCall(
            call_id="call-9",
            name="mcp.filesystem.read_text_file",
            input_json="{}",
            caller_id="agent:assistant",
        )
    )
    _apply(effects)
    request_id = _of(effects, McpSend)[0].message["id"]

    error = {
        "jsonrpc": "2.0",
        "id": request_id,
        "error": {"code": -32602, "message": "path is required"},
    }
    effects = mcp_bridge.update(
        McpMessage(server_id="filesystem", message=error, raw=json.dumps(error))
    )
    _apply(effects)
    results = _of(effects, ToolResult)
    assert len(results) == 1
    assert results[0].call_id == "call-9"
    assert results[0].output_json is None
    assert "path is required" in results[0].error


def test_call_for_an_unknown_tool_fails_immediately() -> None:
    _drive_to_ready()
    effects = mcp_bridge.update(
        ToolCall(call_id="c", name="mcp.filesystem.nope", input_json="{}", caller_id="a")
    )
    results = _of(effects, ToolResult)
    assert len(results) == 1
    assert "unknown tool" in results[0].error


def test_invalid_input_json_fails_the_call_and_sends_nothing() -> None:
    _drive_to_ready()
    effects = mcp_bridge.update(
        ToolCall(
            call_id="c",
            name="mcp.filesystem.read_text_file",
            input_json="{not json",
            caller_id="a",
        )
    )
    assert not _of(effects, McpSend)
    assert "invalid tool input JSON" in _of(effects, ToolResult)[0].error


# ── Server loss ──────────────────────────────────────────────────────────────


def test_close_fails_in_flight_calls_and_withdraws_the_tools() -> None:
    _drive_to_ready()
    effects = mcp_bridge.update(
        ToolCall(
            call_id="call-inflight",
            name="mcp.filesystem.read_text_file",
            input_json="{}",
            caller_id="a",
        )
    )
    _apply(effects)

    effects = mcp_bridge.update(
        McpClosed(server_id="filesystem", reason="server closed stdout")
    )
    data = _apply(effects)

    results = _of(effects, ToolResult)
    assert [r.call_id for r in results] == ["call-inflight"]
    assert "closed before replying" in results[0].error
    # The declaration is re-sent without the dead server's tools, so the broker
    # snapshot cannot keep offering them.
    assert _of(effects, ExposeTools)[0].tools == []
    assert data["servers"]["filesystem"]["status"] == "closed"


def test_notifications_are_ignored_without_touching_state() -> None:
    _drive_to_ready()
    notification = {"jsonrpc": "2.0", "method": "notifications/tools/list_changed"}
    effects = mcp_bridge.update(
        McpMessage(server_id="filesystem", message=notification, raw=json.dumps(notification))
    )
    assert effects == []


def test_init_arms_the_handshake_deadline_timer() -> None:
    effects = mcp_bridge.init((800.0, 600.0), ["filesystem"])
    _apply(effects)
    timers = _of(effects, SetTimer)
    assert [t.id for t in timers] == [mcp_bridge._HANDSHAKE_TIMER_ID]
    assert timers[0].delay_ms == mcp_bridge.HANDSHAKE_TIMEOUT_MS


def test_handshake_deadline_fails_a_stuck_server_and_disconnects_it() -> None:
    # Server answers the connect but never answers initialize: without the
    # deadline it would sit at "initializing" forever, alive and invisible.
    _apply(mcp_bridge.init((800.0, 600.0), ["filesystem"]))
    _apply(
        mcp_bridge.update(
            McpConnected(request_id="connect:filesystem", server_id="filesystem", error=None)
        )
    )
    effects = mcp_bridge.update(TimerFired(id=mcp_bridge._HANDSHAKE_TIMER_ID))
    data = _apply(effects)
    server = data["servers"]["filesystem"]
    assert server["status"] == "failed"
    assert "handshake not completed" in server["error"]
    disconnects = _of(effects, McpDisconnect)
    assert [d.server_id for d in disconnects] == ["filesystem"]


def test_handshake_deadline_keeps_the_failure_reason_across_the_close() -> None:
    # The disconnect above makes the host reply McpClosed("disconnected by
    # app"); that close must not overwrite the deadline diagnosis.
    _apply(mcp_bridge.init((800.0, 600.0), ["filesystem"]))
    _apply(
        mcp_bridge.update(
            McpConnected(request_id="connect:filesystem", server_id="filesystem", error=None)
        )
    )
    _apply(mcp_bridge.update(TimerFired(id=mcp_bridge._HANDSHAKE_TIMER_ID)))
    data = _apply(
        mcp_bridge.update(
            McpClosed(server_id="filesystem", reason="disconnected by app")
        )
    )
    server = data["servers"]["filesystem"]
    assert server["status"] == "failed"
    assert "handshake not completed" in server["error"]


def test_handshake_deadline_is_a_no_op_once_ready() -> None:
    _drive_to_ready()
    assert mcp_bridge.update(TimerFired(id=mcp_bridge._HANDSHAKE_TIMER_ID)) == []


def test_completed_handshake_cancels_the_deadline_timer() -> None:
    effects = _drive_to_ready()
    cancels = _of(effects, CancelTimer)
    assert [c.id for c in cancels] == [mcp_bridge._HANDSHAKE_TIMER_ID]


def test_reconnect_rearms_the_handshake_deadline() -> None:
    _drive_to_ready()
    effects = mcp_bridge.update(KeyEvent(key="r", pressed=True))
    _apply(effects)
    timers = _of(effects, SetTimer)
    assert [t.id for t in timers] == [mcp_bridge._HANDSHAKE_TIMER_ID]
