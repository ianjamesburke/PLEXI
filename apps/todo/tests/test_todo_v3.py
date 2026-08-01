from __future__ import annotations

import json
import os
import sys

sys.path.insert(
    0,
    os.path.join(os.path.dirname(__file__), "..", "..", "..", "sdk", "python"),
)
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import plexi_sdk as sdk  # noqa: E402
from plexi_sdk import _v3_state  # noqa: E402
from plexi_sdk.effects import ExposeTools, PersistState, SetState, ToolResult  # noqa: E402
from plexi_sdk.events import KeyEvent, ToolCall, UiAction, UiValueChange  # noqa: E402

import todo  # noqa: E402


def _set_state(values: dict) -> None:
    raw = {key: b"" for key in values}
    _v3_state._state = sdk.StateSnapshot(values, raw)
    _v3_state._in_view = False


def _persisted(effects: list) -> dict:
    effect = next(effect for effect in effects if isinstance(effect, PersistState))
    return effect.data


def _transient(effects: list) -> dict:
    effect = next(effect for effect in effects if isinstance(effect, SetState))
    return effect.data


def _tool(name: str, **arguments) -> ToolCall:
    return ToolCall("call-1", name, json.dumps(arguments), "assistant")


def _result(effects: list) -> dict:
    effect = next(effect for effect in effects if isinstance(effect, ToolResult))
    assert effect.error is None, effect.error
    assert effect.output_json is not None
    return json.loads(effect.output_json)


def _flatten(component) -> list[dict]:
    """Every node in the component's tree, parents before children."""
    out: list[dict] = []

    def walk(node) -> None:
        if not isinstance(node, dict):
            return
        out.append(node)
        for child in node.get("children") or []:
            walk(child)

    walk(component.to_node())
    return out


def _seeded(items: list) -> dict:
    return {**todo.DEFAULT_TODO_STATE, "items": items}


def test_add_toggle_and_delete_through_ui_actions() -> None:
    _set_state(dict(todo.DEFAULT_TODO_STATE))

    adding = _persisted(todo.update(UiAction(todo.START)))
    assert adding["adding"] is True

    _set_state(adding)
    draft_effects = todo.update(UiValueChange(todo.DRAFT, "Write tests"))
    assert not any(isinstance(effect, PersistState) for effect in draft_effects)
    draft = _transient(draft_effects)
    assert draft["draft"] == "Write tests"

    _set_state(draft)
    added = _persisted(todo.update(UiAction(todo.ADD)))
    assert added["items"] == [{"text": "Write tests", "done": False}]
    assert added["adding"] is False and added["draft"] == ""

    _set_state(added)
    toggled = _persisted(todo.update(KeyEvent("space")))
    assert toggled["items"][0]["done"] is True

    _set_state(toggled)
    assert _persisted(todo.update(KeyEvent("d")))["items"] == []


def test_form_submit_is_the_only_enter_path_and_escape_cancels() -> None:
    _set_state({**todo.DEFAULT_TODO_STATE, "adding": True, "draft": "milk"})
    assert todo.update(KeyEvent("enter")) == []
    assert _persisted(todo.update(UiAction(todo.ADD)))["items"] == [
        {"text": "milk", "done": False}
    ]

    _set_state({**todo.DEFAULT_TODO_STATE, "adding": True, "draft": "milk"})
    cancelled = _persisted(todo.update(KeyEvent("escape")))
    assert cancelled["items"] == [] and cancelled["adding"] is False


def test_selection_stays_in_range_after_delete() -> None:
    _set_state({
        **_seeded([{"text": "a", "done": False}, {"text": "b", "done": False}]),
        "selected": 1,
    })
    assert _persisted(todo.update(KeyEvent("d")))["selected"] == 0


def test_add_form_field_autofocuses_so_typing_starts_immediately() -> None:
    _set_state({**todo.DEFAULT_TODO_STATE, "adding": True, "draft": ""})
    inputs = [n for n in _flatten(todo.view()) if n.get("type") == "TextInput"]
    assert [n["autofocus"] for n in inputs] == [True]


def test_top_chrome_is_present_in_list_and_add_views() -> None:
    for adding in (False, True):
        _set_state({**todo.DEFAULT_TODO_STATE, "adding": adding})
        root = todo.view().to_node()
        assert root["children"][0]["type"] == "app_bar"
        assert root["children"][0]["title"] == "Todo"


def test_add_and_cancel_render_as_one_action_row_not_stacked() -> None:
    _set_state({**todo.DEFAULT_TODO_STATE, "adding": True, "draft": "x"})
    labels = [
        child["label"]
        for node in _flatten(todo.view())
        if node.get("type") == "row"
        for child in node["children"]
        if child.get("type") == "button"
    ]
    assert labels == ["Add", "Cancel"]


def test_assistant_tools_are_exposed_at_init() -> None:
    _set_state(dict(todo.DEFAULT_TODO_STATE))
    declared = next(e for e in todo.init((800, 600), []) if isinstance(e, ExposeTools))
    assert {tool.name for tool in declared.tools} == {
        "todo.list",
        "todo.add",
        "todo.set_done",
        "todo.remove",
    }
    assert {tool.name for tool in declared.tools if tool.read_only} == {"todo.list"}


def test_assistant_can_read_and_write_the_list() -> None:
    _set_state(dict(todo.DEFAULT_TODO_STATE))
    effects = todo.update(_tool("todo.add", text="from the assistant"))
    assert _result(effects) == {"count": 1}
    added = _persisted(effects)
    assert added["items"] == [{"text": "from the assistant", "done": False}]

    _set_state(added)
    effects = todo.update(_tool("todo.set_done", index=0, done=True))
    assert _result(effects)["done"] is True

    _set_state(_persisted(effects))
    assert _result(todo.update(_tool("todo.list"))) == {
        "items": [{"text": "from the assistant", "done": True}],
        "open": 0,
    }

    effects = todo.update(_tool("todo.remove", index=0))
    assert _result(effects) == {"removed": "from the assistant"}
    assert _persisted(effects)["items"] == []


def test_out_of_range_tool_index_is_an_error_not_a_crash() -> None:
    _set_state(dict(todo.DEFAULT_TODO_STATE))
    effect = next(
        e
        for e in todo.update(_tool("todo.remove", index=3))
        if isinstance(e, ToolResult)
    )
    assert effect.output_json is None
    assert "no todo item at index 3" in (effect.error or "")
