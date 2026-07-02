from __future__ import annotations

import os
import sys

sys.path.insert(
    0,
    os.path.join(os.path.dirname(__file__), "..", "..", "..", "sdk", "python"),
)
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import plexi_sdk as sdk  # noqa: E402
from plexi_sdk import _v3_state  # noqa: E402
from plexi_sdk.effects import SetState  # noqa: E402
from plexi_sdk.events import KeyEvent, UiAction  # noqa: E402

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

    data: dict = dict(calc.DEFAULT_STATE)
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

    data: dict = dict(calc.DEFAULT_STATE)
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


def test_view_serializes_button_rows_as_action_bars() -> None:
    _set_state(dict(calc.DEFAULT_STATE))

    root = calc.view().to_node()
    assert root is not None

    def walk(node: dict) -> list[dict]:
        children = []
        for child in node.get("children", []):
            children.extend(walk(child))
        return [node, *children]

    nodes = walk(root)
    assert all(node.get("type") != "row" for node in nodes)
    assert any(node.get("type") == "action_bar" for node in nodes)
