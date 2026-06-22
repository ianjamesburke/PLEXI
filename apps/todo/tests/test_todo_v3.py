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
from plexi_sdk.events import KeyEvent, UiAction, UiValueChange  # noqa: E402

import todo  # noqa: E402


def _set_state(values: dict) -> None:
    raw = {key: b"" for key in values}
    _v3_state._state = sdk.StateSnapshot(values, raw)
    _v3_state._in_view = False


def _state_effect(effects: list) -> dict:
    effect = next(effect for effect in effects if isinstance(effect, SetState))
    return effect.data


def test_add_toggle_and_delete_item_with_v3_effects() -> None:
    _set_state(dict(todo.DEFAULT_TODO_STATE))

    effects = todo.update(UiAction("todo-add:start"))
    adding = _state_effect(effects)
    assert adding["adding"] is True

    _set_state(adding)
    effects = todo.update(UiValueChange("todo-add:change", "Write tests"))
    draft = _state_effect(effects)
    assert draft["draft"] == "Write tests"

    _set_state(draft)
    effects = todo.update(UiAction("todo-add:submit"))
    added = _state_effect(effects)
    assert added["items"] == [{"text": "Write tests", "done": False}]
    assert added["selected"] == 0
    assert added["adding"] is False

    _set_state(added)
    effects = todo.update(KeyEvent("space"))
    toggled = _state_effect(effects)
    assert toggled["items"][0]["done"] is True

    _set_state(toggled)
    effects = todo.update(KeyEvent("d"))
    deleted = _state_effect(effects)
    assert deleted["items"] == []
    assert deleted["selected"] == 0
