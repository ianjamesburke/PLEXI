from __future__ import annotations

import os
import sys

sys.path.insert(
    0,
    os.path.join(os.path.dirname(__file__), "..", "..", "..", "sdk", "python"),
)
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import json  # noqa: E402

import plexi_sdk as sdk  # noqa: E402
from plexi_sdk import _v3_state  # noqa: E402
from plexi_sdk.effects import ExposeTools, SetState, ToolResult  # noqa: E402
from plexi_sdk.events import KeyEvent, ToolCall, UiAction  # noqa: E402

import calc  # noqa: E402


def _set_state(values: dict) -> None:
    raw = {key: b"" for key in values}
    _v3_state._state = sdk.StateSnapshot(values, raw)
    _v3_state._in_view = False


def _state_effect(effects: list) -> dict:
    effect = next(effect for effect in effects if isinstance(effect, SetState))
    return effect.data


def test_keyboard_math_uses_v3_set_state_effects() -> None:
    _set_state(dict(calc.DEFAULT_STATE))

    for key in ["7", "+", "5", "enter"]:
        effects = calc.update(KeyEvent(key))
        data = _state_effect(effects)
        _set_state(data)

    assert data["display"] == "12"
    assert data["pending"] is None
    assert data["op"] is None
    assert data["fresh"] is True


def test_button_actions_decimal_backspace_and_clear() -> None:
    _set_state(dict(calc.DEFAULT_STATE))

    for action in [
        "calc:key:1",
        "calc:key:.",
        "calc:key:5",
        "calc:key:+/-",
        "calc:key:C",
    ]:
        effects = calc.update(UiAction(action))
        data = _state_effect(effects)
        _set_state(data)

    assert data == calc.DEFAULT_STATE

    _set_state(dict(calc.DEFAULT_STATE, display="123", fresh=False))
    effects = calc.update(KeyEvent("backspace"))
    assert _state_effect(effects)["display"] == "12"


def _call(name: str, payload: dict, call_id: str = "call-1") -> ToolResult:
    effects = calc.update(ToolCall(call_id, name, json.dumps(payload), "assistant"))
    assert len(effects) == 1, f"a tool call must produce exactly one effect: {effects}"
    result = effects[0]
    assert isinstance(result, ToolResult)
    assert result.call_id == call_id, "ToolResult must echo the inbound call_id"
    return result


def _output(result: ToolResult) -> dict:
    assert result.error is None, f"expected success, got error: {result.error}"
    assert result.output_json is not None
    return json.loads(result.output_json)


def test_init_exposes_read_only_tools_for_every_operation() -> None:
    _set_state(dict(calc.DEFAULT_STATE))

    declaration = next(e for e in calc.init(None, None) if isinstance(e, ExposeTools))
    names = {tool.name for tool in declaration.tools}

    assert names == {
        "calc.add",
        "calc.subtract",
        "calc.multiply",
        "calc.divide",
        "calc.evaluate",
    }
    assert all(tool.read_only for tool in declaration.tools), (
        "every calc tool is pure compute and must be read_only for auto-grant"
    )
    for tool in declaration.tools:
        assert tool.description, f"{tool.name} needs a description"
        assert tool.input_schema.get("required"), f"{tool.name} must require its inputs"
        assert tool.output_schema["properties"]["result"]["type"] == "number"


def test_binary_tool_calls_compute_results() -> None:
    _set_state(dict(calc.DEFAULT_STATE))

    assert _output(_call("calc.add", {"a": 7, "b": 5}))["result"] == 12
    assert _output(_call("calc.subtract", {"a": 7, "b": 5}))["result"] == 2
    assert _output(_call("calc.multiply", {"a": 7, "b": 5}))["result"] == 35
    assert _output(_call("calc.divide", {"a": 7, "b": 2}))["result"] == 3.5


def test_evaluate_parses_single_binary_expression() -> None:
    _set_state(dict(calc.DEFAULT_STATE))

    assert _output(_call("calc.evaluate", {"expression": "12 * 3.5"}))["result"] == 42
    assert _output(_call("calc.evaluate", {"expression": "-8/2"}))["result"] == -4
    assert _output(_call("calc.evaluate", {"expression": "1e3 - 1"}))["result"] == 999


def test_tool_calls_report_errors_instead_of_raising() -> None:
    _set_state(dict(calc.DEFAULT_STATE))

    for name, payload in [
        ("calc.divide", {"a": 1, "b": 0}),
        ("calc.add", {"a": 1}),
        ("calc.add", {"a": "seven", "b": 1}),
        ("calc.nope", {"a": 1, "b": 1}),
        ("calc.evaluate", {"expression": "banana"}),
        ("calc.evaluate", {"expression": 7}),
        # `inf`/`nan` are not valid JSON numbers, so they must never reach
        # output_json — they surface as a structured error instead.
        ("calc.add", {"a": 1e309, "b": 1}),
        ("calc.multiply", {"a": 1e308, "b": 1e308}),
        ("calc.evaluate", {"expression": "1e309 + 1"}),
    ]:
        result = _call(name, payload)
        assert result.error, f"{name}{payload} must return a ToolResult error"
        assert result.output_json is None, "never set both output_json and error"


def test_malformed_input_json_is_an_error_not_a_crash() -> None:
    _set_state(dict(calc.DEFAULT_STATE))

    effects = calc.update(ToolCall("call-9", "calc.add", "{not json", "assistant"))
    assert len(effects) == 1
    assert effects[0].error


def test_tool_calls_do_not_mutate_calculator_state() -> None:
    _set_state(dict(calc.DEFAULT_STATE, display="42", fresh=False))

    effects = calc.update(ToolCall("call-3", "calc.add", '{"a":1,"b":1}', "assistant"))
    assert not any(isinstance(e, SetState) for e in effects), (
        "read-only tools must never emit SetState"
    )
