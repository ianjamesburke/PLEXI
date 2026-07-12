#!/usr/bin/env python3
"""Calculator — SDK v3 runtime-state four-function calculator."""

from __future__ import annotations

from plexi_sdk import log, state
from plexi_sdk.effects import SetState, SetStatus, SetTitle
from plexi_sdk.events import KeyEvent, UiAction
from plexi_sdk.ui import Button, Column, Component, Spacer, Text

BUTTON_ROWS = [
    ["C", "+/-", "%", "/"],
    ["7", "8", "9", "*"],
    ["4", "5", "6", "-"],
    ["1", "2", "3", "+"],
    ["0", ".", "="],
]

DEFAULT_STATE = {
    "display": "0",
    "pending": None,
    "op": None,
    "fresh": True,
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
    effects: list = [SetTitle("Calculator"), SetStatus(_status(data))]
    if missing:
        effects.append(SetState(missing))
    return effects


def update(event) -> list:
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
            Text(
                "0-9 input. Operators queue. Enter equals. Backspace deletes.",
                size=11.0,
            ),
        ],
        gap=8.0,
        grow=True,
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
