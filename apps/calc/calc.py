#!/usr/bin/env python3
"""Calculator — SDK v3 runtime-state four-function calculator."""

from __future__ import annotations

import json
import math
from typing import Any, Callable

from plexi_sdk import log, state
from plexi_sdk.effects import AiTool, ExposeTools, SetState, SetStatus, SetTitle, ToolResult
from plexi_sdk.events import KeyEvent, ToolCall, UiAction
from plexi_sdk.ui import Button, Column, Component, FooterKeys, Spacer, Text

BUTTON_ROWS = [
    ["C", "+/-", "%", "/"],
    ["7", "8", "9", "*"],
    ["4", "5", "6", "-"],
    ["1", "2", "3", "+"],
    ["0", ".", "="],
]

DEFAULT_STATE: dict[str, Any] = {
    "display": "0",
    "pending": None,
    "op": None,
    "fresh": True,
}

# Connector tools. Every one is pure compute over its own arguments — none reads
# or writes the calculator's display state — so all are read_only and the
# assistant can run them without a write-grant prompt.
BINARY_OPS: dict[str, Callable[[float, float], float]] = {
    "add": lambda a, b: a + b,
    "subtract": lambda a, b: a - b,
    "multiply": lambda a, b: a * b,
    "divide": lambda a, b: a / b,
}

_NUMBER_PAIR_SCHEMA = {
    "type": "object",
    "properties": {
        "a": {"type": "number", "description": "Left operand."},
        "b": {"type": "number", "description": "Right operand."},
    },
    "required": ["a", "b"],
}

_RESULT_SCHEMA = {
    "type": "object",
    "properties": {"result": {"type": "number"}},
    "required": ["result"],
}


class ButtonRow(Component):
    def __init__(self, labels: list[str]) -> None:
        self.labels = labels

    def to_node(self) -> dict:
        return {
            "type": "row",
            "children": [
                Button(label, f"calc:key:{label}", style=_button_style(label)).to_node()
                for label in self.labels
            ],
            "gap": 8.0,
            "align": "start",
            "grow": False,
        }


def init(size, args) -> list:
    data = _state()
    missing = {
        key: value
        for key, value in DEFAULT_STATE.items()
        if state.get(key, None) is None
    }
    log.info("calc: SDK v3 initialized")
    effects: list = [
        SetTitle("Calculator"),
        SetStatus(_status(data)),
        ExposeTools(_tools()),
    ]
    log.info(f"calc: exposed {len(BINARY_OPS) + 1} connector tools")
    if missing:
        effects.append(SetState(missing))
    return effects


def _tools() -> list[AiTool]:
    tools = [
        AiTool(
            name=f"calc.{name}",
            description=f"{name.capitalize()} two numbers and return the result.",
            input_schema=_NUMBER_PAIR_SCHEMA,
            output_schema=_RESULT_SCHEMA,
            read_only=True,
        )
        for name in BINARY_OPS
    ]
    tools.append(
        AiTool(
            name="calc.evaluate",
            description=(
                "Evaluate a single binary expression of the form '<number> <op> <number>', "
                "where op is one of + - * /."
            ),
            input_schema={
                "type": "object",
                "properties": {
                    "expression": {
                        "type": "string",
                        "description": "For example '12 * 3.5'.",
                    }
                },
                "required": ["expression"],
            },
            output_schema=_RESULT_SCHEMA,
            read_only=True,
        )
    )
    return tools


def update(event) -> list:
    if isinstance(event, ToolCall):
        return [_handle_tool_call(event)]
    label = _event_label(event)
    if label is None:
        return []
    data = _state()
    _press(data, label)
    return [SetState(data), SetStatus(_status(data))]


def view():
    data = _state()
    subtitle = f"{_format_number(data['pending'])} {data['op']}" if data["op"] else ""
    return Column(
        [
            Text("Calculator", bold=True, size=15.0),
            Text(subtitle or "ready", size=11.0),
            Text(data["display"], size=28.0, bold=True, align="end", truncate=True),
            *[ButtonRow(row) for row in BUTTON_ROWS],
            Spacer(grow=True),
            FooterKeys(
                [
                    ("0-9", "input"),
                    ("+ - * /", "operators"),
                    ("↩", "equals"),
                    ("⌫", "delete"),
                    ("esc", "clear"),
                ]
            ),
        ],
        gap=8.0,
        grow=True,
    )


def _handle_tool_call(call: ToolCall) -> ToolResult:
    log.info(f"calc: tool call {call.name} from {call.caller_id}")
    try:
        payload = json.loads(call.input_json or "{}")
    except ValueError as e:
        return ToolResult(call.call_id, error=f"input_json is not valid JSON: {e}")
    if not isinstance(payload, dict):
        return ToolResult(call.call_id, error="input must be a JSON object")

    try:
        if call.name == "calc.evaluate":
            result = _evaluate(payload.get("expression"))
        else:
            op = BINARY_OPS.get(call.name.removeprefix("calc."))
            if op is None or not call.name.startswith("calc."):
                return ToolResult(call.call_id, error=f"unknown tool '{call.name}'")
            result = op(_operand(payload, "a"), _operand(payload, "b"))
    except (ValueError, ZeroDivisionError, OverflowError) as e:
        log.warn(f"calc: tool call {call.name} failed: {e}")
        return ToolResult(call.call_id, error=str(e))

    # `inf`/`nan` serialize as bare `Infinity`/`NaN`, which is not valid JSON and
    # breaks the declared number-typed output schema. Report them as errors.
    if not math.isfinite(result):
        log.warn(f"calc: tool call {call.name} produced a non-finite result")
        return ToolResult(call.call_id, error=f"result is not a finite number: {result}")

    return ToolResult(call.call_id, output_json=json.dumps({"result": result}))


def _operand(payload: dict, key: str) -> float:
    if key not in payload:
        raise ValueError(f"missing required operand '{key}'")
    value = payload[key]
    if isinstance(value, bool) or not isinstance(value, (int, float, str)):
        raise ValueError(f"operand '{key}' must be a number")
    try:
        number = float(value)
    except (ValueError, OverflowError):
        raise ValueError(f"operand '{key}' is not a number: {value!r}") from None
    if not math.isfinite(number):
        raise ValueError(f"operand '{key}' must be finite, got {value!r}")
    return number


def _evaluate(expression: Any) -> float:
    """Parse and compute one `<number> <op> <number>` expression.

    Deliberately not a general expression evaluator — the four binary ops are the
    whole contract, and hand-parsing keeps `eval` well away from assistant input.
    """
    if not isinstance(expression, str):
        raise ValueError("'expression' must be a string")
    symbols = {"+": "add", "-": "subtract", "*": "multiply", "/": "divide"}
    # Scan from the right so a leading sign stays attached to its operand.
    for index in range(len(expression) - 1, 0, -1):
        name = symbols.get(expression[index])
        if name is None:
            continue
        left, right = expression[:index].strip(), expression[index + 1 :].strip()
        if not left or not right:
            continue
        try:
            a, b = float(left), float(right)
        except ValueError:
            continue
        if not (math.isfinite(a) and math.isfinite(b)):
            raise ValueError(f"operands in '{expression}' must be finite")
        return BINARY_OPS[name](a, b)
    raise ValueError(
        f"could not parse '{expression}' as '<number> <op> <number>' with op in + - * /"
    )


def _state() -> dict:
    data = dict(DEFAULT_STATE)
    for key, value in DEFAULT_STATE.items():
        data[key] = state.get(key, value)
    data["display"] = str(data.get("display") or "0")
    data["op"] = data.get("op") if data.get("op") in {"+", "-", "*", "/"} else None
    data["pending"] = _coerce_pending(data.get("pending"))
    data["fresh"] = bool(data.get("fresh", True))
    return data


def _event_label(event) -> str | None:
    if isinstance(event, UiAction) and event.handler_id.startswith("calc:key:"):
        return event.handler_id.removeprefix("calc:key:")
    if not isinstance(event, KeyEvent) or not event.pressed:
        return None
    key = event.key
    if key in "0123456789":
        return key
    if key in {".", "+", "-", "*", "/", "%"}:
        return key
    if key in {"=", "return", "enter"}:
        return "="
    if key == "backspace":
        return "backspace"
    if key == "escape":
        return "C"
    return None


def _press(data: dict, label: str) -> None:
    if label.isdigit():
        if data["fresh"] or data["display"] == "0":
            data["display"] = label
        else:
            data["display"] += label
        data["fresh"] = False
        return

    if label == ".":
        if data["fresh"]:
            data["display"] = "0."
            data["fresh"] = False
        elif "." not in data["display"]:
            data["display"] += "."
        return

    if label == "backspace":
        if not data["fresh"]:
            data["display"] = data["display"][:-1] or "0"
        return

    if label == "C":
        data.update(dict(DEFAULT_STATE))
        return

    if label == "+/-":
        value = -_display_value(data)
        data["display"] = _format_number(value)
        return

    if label == "%":
        data["display"] = _format_number(_display_value(data) / 100.0)
        data["fresh"] = True
        return

    if label in {"+", "-", "*", "/"}:
        _apply_pending(data)
        data["pending"] = _display_value(data)
        data["op"] = label
        data["fresh"] = True
        return

    if label == "=":
        _apply_pending(data)


def _apply_pending(data: dict) -> None:
    if data["op"] is None or data["pending"] is None:
        return
    current = _display_value(data)
    pending = float(data["pending"])
    if data["op"] == "+":
        result = pending + current
    elif data["op"] == "-":
        result = pending - current
    elif data["op"] == "*":
        result = pending * current
    elif data["op"] == "/":
        result = pending / current if current != 0 else float("inf")
    else:
        result = current
    data["display"] = _format_number(result)
    data["pending"] = None
    data["op"] = None
    data["fresh"] = True


def _display_value(data: dict) -> float:
    try:
        return float(data["display"])
    except ValueError:
        return 0.0


def _coerce_pending(value) -> float | None:
    if value is None:
        return None
    try:
        return float(value)
    except (TypeError, ValueError):
        return None


def _format_number(value: float | None) -> str:
    if value is None:
        return "0"
    if value == float("inf"):
        return "Infinity"
    if value == float("-inf"):
        return "-Infinity"
    if value == int(value) and abs(value) < 1e15:
        return str(int(value))
    return f"{value:.12g}"


def _button_style(label: str) -> str:
    if label in {"+", "-", "*", "/", "="}:
        return "primary"
    if label == "C":
        return "danger"
    return "secondary"


def _status(data: dict) -> str:
    if data["op"] and data["pending"] is not None:
        return f"{_format_number(data['pending'])} {data['op']}"
    return data["display"]
