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
from plexi_sdk.effects import PersistState, SetTitle  # noqa: E402
from plexi_sdk.events import KeyEvent, UiAction, UiValueChange  # noqa: E402

import todo  # noqa: E402


def _set_state(values: dict) -> None:
    _v3_state._state = sdk.StateSnapshot(values, {})
    _v3_state._in_view = False


def _state_effect(effects: list) -> dict:
    return next(effect.data for effect in effects if isinstance(effect, PersistState))


def test_init_persists_normalized_default_state() -> None:
    _set_state({})

    effects = todo.init((480, 320), [])

    assert any(isinstance(effect, SetTitle) and effect.title == "Todo" for effect in effects)
    assert _state_effect(effects) == todo.DEFAULT_STATE


def test_add_toggle_with_space_enter_and_delete_item_with_v3_effects() -> None:
    _set_state(dict(todo.DEFAULT_STATE))

    adding = _state_effect(todo.update(KeyEvent("n")))
    assert adding["mode"] == "add"

    _set_state(adding)
    draft = _state_effect(todo.update(UiValueChange(todo.DRAFT_ID, "Write tests")))
    assert draft["draft"] == "Write tests"

    _set_state(draft)
    added = _state_effect(todo.update(UiAction("todo-add")))
    assert added["items"] == [{"text": "Write tests", "done": False}]
    assert added["selected"] == 0
    assert added["mode"] == "list"

    _set_state(added)
    toggled_by_space = _state_effect(todo.update(KeyEvent("space")))
    assert toggled_by_space["items"][0]["done"] is True

    _set_state(toggled_by_space)
    toggled_by_enter = _state_effect(todo.update(KeyEvent("enter")))
    assert toggled_by_enter["items"][0]["done"] is False

    _set_state(toggled_by_enter)
    deleted = _state_effect(todo.update(KeyEvent("d")))
    assert deleted["items"] == []
    assert deleted["selected"] == 0


def test_navigation_clamps_selection_with_j_k_and_arrows() -> None:
    data = {
        **todo.DEFAULT_STATE,
        "items": [
            {"text": "one", "done": False},
            {"text": "two", "done": False},
            {"text": "three", "done": False},
        ],
    }
    _set_state(data)

    selected = _state_effect(todo.update(KeyEvent("j")))
    assert selected["selected"] == 1

    _set_state(selected)
    selected = _state_effect(todo.update(KeyEvent("ArrowDown")))
    assert selected["selected"] == 2

    _set_state(selected)
    selected = _state_effect(todo.update(KeyEvent("j")))
    assert selected["selected"] == 2

    _set_state(selected)
    selected = _state_effect(todo.update(KeyEvent("k")))
    assert selected["selected"] == 1

    _set_state(selected)
    selected = _state_effect(todo.update(KeyEvent("ArrowUp")))
    assert selected["selected"] == 0


def test_delete_keeps_selection_on_next_available_item() -> None:
    data = {
        **todo.DEFAULT_STATE,
        "items": [
            {"text": "one", "done": False},
            {"text": "two", "done": False},
            {"text": "three", "done": False},
        ],
        "selected": 1,
    }
    _set_state(data)

    deleted = _state_effect(todo.update(UiAction("todo-delete")))

    assert deleted["items"] == [
        {"text": "one", "done": False},
        {"text": "three", "done": False},
    ]
    assert deleted["selected"] == 1


def test_delete_alias_keys_still_remove_selected_item() -> None:
    for key in ("x", "backspace", "delete"):
        data = {
            **todo.DEFAULT_STATE,
            "items": [
                {"text": "one", "done": False},
                {"text": "two", "done": False},
            ],
            "selected": 0,
        }
        _set_state(data)

        deleted = _state_effect(todo.update(KeyEvent(key)))

        assert deleted["items"] == [{"text": "two", "done": False}]
        assert deleted["selected"] == 0


def test_add_mode_cancel_and_blank_submit_return_to_list() -> None:
    data = {**todo.DEFAULT_STATE, "mode": "add", "draft": "  "}
    _set_state(data)

    submitted = _state_effect(todo.update(UiAction(todo.DRAFT_ID)))

    assert submitted["items"] == []
    assert submitted["mode"] == "list"
    assert submitted["draft"] == ""

    data = {**todo.DEFAULT_STATE, "mode": "add", "draft": "ignored"}
    _set_state(data)
    cancelled = _state_effect(todo.update(KeyEvent("escape")))

    assert cancelled["mode"] == "list"
    assert cancelled["draft"] == ""


def test_list_view_uses_action_bar_and_footer_keys() -> None:
    _set_state(
        {
            **todo.DEFAULT_STATE,
            "items": [{"text": "Write tests", "done": False}],
        }
    )

    node = todo.view().to_node()

    assert node["type"] == "column"
    app_bar, select_list, action_bar, footer = node["children"]
    assert app_bar == {"type": "app_bar", "title": "Todo", "subtitle": "0/1 done"}
    assert select_list["type"] == "select_list"
    assert select_list["selected_idx"] == 0
    assert action_bar["type"] == "stack"
    assert action_bar["direction"] == "horizontal"
    assert [button["label"] for button in action_bar["children"]] == [
        "New",
        "Toggle",
        "Delete",
    ]
    assert footer["type"] == "pinned"
    assert footer["edge"] == "bottom"
    assert footer["child"]["type"] == "footer_keys"


def test_add_view_uses_text_edit_action_bar_and_footer_keys() -> None:
    _set_state({**todo.DEFAULT_STATE, "mode": "add", "draft": "Write docs"})

    node = todo.view().to_node()

    app_bar, text_edit, action_bar, footer = node["children"]
    assert app_bar == {"type": "app_bar", "title": "Todo", "subtitle": "New item"}
    assert text_edit == {
        "type": "text_edit",
        "node_id": todo.DRAFT_ID,
        "placeholder": "What needs doing?",
        "value": "Write docs",
        "multiline": False,
        "max_length": 0,
    }
    assert action_bar["type"] == "stack"
    assert [button["label"] for button in action_bar["children"]] == ["Add", "Cancel"]
    assert footer["type"] == "pinned"
    assert footer["child"]["type"] == "footer_keys"
