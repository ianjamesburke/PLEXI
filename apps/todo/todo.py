#!/usr/bin/env python3
"""Todo — the reference PGAP app: every need is an SDK primitive, none is local.

Items live in this context's `[state]` document — a markdown checklist a
human or agent can edit directly (`plexi app state set todo`) while the pane
repaints. Selection, the add-form's open/closed flag, and its draft text are
process-local UI state and are never written to disk.
"""

from __future__ import annotations

from typing import Any

from plexi_sdk import log, state, tools
from plexi_sdk.effects import PersistState, SetState, SetStatus, SetTitle
from plexi_sdk.events import KeyEvent, StateChanged, UiAction, UiValueChange
from plexi_sdk.state_format import ChecklistItem, parse_checklist, render_checklist
from plexi_sdk.ui import (
    Actions,
    AppBar,
    Button,
    Column,
    FooterKeys,
    FormField,
    SelectList,
    Spacer,
    Text,
)

DEFAULT_UI_STATE: dict[str, Any] = {
    "selected": 0,
    "adding": False,
    "draft": "",
}

ADD = "todo:add"
CANCEL = "todo:cancel"
DRAFT = "todo:draft"
START = "todo:start"
TOGGLE = "todo:toggle"
DELETE = "todo:delete"


def _items() -> list[dict]:
    document = str(state.get("document", "") or "")
    return [{"text": item.text, "done": item.done} for item in parse_checklist(document)]


def _data() -> dict:
    items = _items()
    return {
        "items": items,
        "selected": _clamp(items, int(state.get("selected", 0) or 0)),
        "adding": bool(state.get("adding", False)),
        "draft": str(state.get("draft", "") or ""),
    }


def _clamp(items: list, selected: int) -> int:
    return max(0, min(selected, len(items) - 1)) if items else 0


def _status(data: dict) -> str:
    open_count = sum(1 for item in data["items"] if not item["done"])
    return f"{open_count} open · {len(data['items'])} total"


def _ui_effects(data: dict) -> list:
    """Selection/form change only — process-local, no disk write."""
    data["selected"] = _clamp(data["items"], data["selected"])
    return [
        SetState({"selected": data["selected"], "adding": data["adding"], "draft": data["draft"]}),
        SetStatus(_status(data)),
    ]


def _item_effects(data: dict) -> list:
    """The items changed: re-render the checklist and persist it to disk."""
    data["selected"] = _clamp(data["items"], data["selected"])
    document = render_checklist(
        [ChecklistItem(item["text"], item["done"]) for item in data["items"]]
    )
    return [
        PersistState({"document": document}),
        SetState({"selected": data["selected"], "adding": data["adding"], "draft": data["draft"]}),
        SetStatus(_status(data)),
    ]


def _add(data: dict, text: str) -> dict:
    text = text.strip()
    if text:
        data["items"].append({"text": text, "done": False})
        data["selected"] = len(data["items"]) - 1
        log.info(f"todo: added item ({len(data['items'])} total)")
    data["adding"] = False
    data["draft"] = ""
    return data


@tools.tool("todo.list", "List every todo item with its done state.", read_only=True)
def _tool_list() -> dict:
    data = _data()
    return {"items": data["items"], "open": sum(1 for i in data["items"] if not i["done"])}


@tools.tool("todo.add", "Add a todo item to this context's list.", {"text": str})
def _tool_add(text: str) -> tools.Reply:
    data = _add(_data(), text)
    return tools.Reply({"count": len(data["items"])}, _item_effects(data))


@tools.tool("todo.set_done", "Mark the todo item at `index` done or not done.",
            {"index": int, "done": bool})
def _tool_set_done(index: int, done: bool) -> tools.Reply:
    data = _data()
    if not 0 <= index < len(data["items"]):
        raise IndexError(f"no todo item at index {index} ({len(data['items'])} items)")
    data["items"][index]["done"] = done
    log.info(f"todo: item {index} done={done} via assistant")
    return tools.Reply({"text": data["items"][index]["text"], "done": done}, _item_effects(data))


@tools.tool("todo.remove", "Remove the todo item at `index`.", {"index": int})
def _tool_remove(index: int) -> tools.Reply:
    data = _data()
    if not 0 <= index < len(data["items"]):
        raise IndexError(f"no todo item at index {index} ({len(data['items'])} items)")
    removed = data["items"].pop(index)
    log.info(f"todo: removed item {index} via assistant")
    return tools.Reply({"removed": removed["text"]}, _item_effects(data))


def init(size, args) -> list:
    data = _data()
    log.info("todo: ready")
    return [SetTitle("Todo"), SetStatus(_status(data)), tools.expose()]


def update(event) -> list:
    handled = tools.dispatch(event)
    if handled is not None:
        return handled

    if isinstance(event, StateChanged):
        if event.error:
            log.warn(f"todo: state file error: {event.error}")
            return [SetStatus(f"todo: {event.error}")]
        log.info("todo: external write to the state file repainted the list")
        return [SetStatus(_status(_data()))]

    data = _data()

    if isinstance(event, UiValueChange) and event.handler_id == DRAFT:
        data["draft"] = event.value
        return [SetState({"draft": data["draft"]})]

    action = event.handler_id if isinstance(event, UiAction) else None
    key = event.key if isinstance(event, KeyEvent) and event.pressed else None

    if action == ADD:
        return _item_effects(_add(data, data["draft"]))
    if action == CANCEL or (data["adding"] and key == "escape"):
        data["adding"] = False
        data["draft"] = ""
        return _ui_effects(data)
    if data["adding"]:
        return []

    if action == START or key == "a":
        data["adding"] = True
        data["draft"] = ""
        return _ui_effects(data)
    if action == TOGGLE or key == "space":
        if not data["items"]:
            return []
        item = data["items"][data["selected"]]
        item["done"] = not item["done"]
        return _item_effects(data)
    if action == DELETE or key == "d":
        if not data["items"]:
            return []
        data["items"].pop(data["selected"])
        return _item_effects(data)
    if key in ("up", "k"):
        data["selected"] = _clamp(data["items"], data["selected"] - 1)
        return _ui_effects(data)
    if key in ("down", "j"):
        data["selected"] = _clamp(data["items"], data["selected"] + 1)
        return _ui_effects(data)
    return []


def view():
    data = _data()
    if data["adding"]:
        return Column([
            AppBar("Todo", "New item"),
            # autofocus: the field takes the cursor the frame the form appears.
            FormField("todo-draft", "Item", placeholder="What needs doing?",
                      value=data["draft"], autofocus=True,
                      on_change=DRAFT, on_submit=ADD),
            Actions([Button("Add", ADD, style="primary", disabled=not data["draft"].strip()),
                     Button("Cancel", CANCEL, style="ghost")]),
            Spacer(grow=True),
            FooterKeys([("↩", "add"), ("esc", "cancel")]),
        ], grow=True)

    rows = [
        {"name": item["text"], "leading": "✓" if item["done"] else "○"}
        for item in data["items"]
    ]
    return Column([
        AppBar("Todo", _status(data)),
        SelectList(rows, selected_idx=data["selected"]) if rows
        else Text("Nothing yet — press a to add an item.", size=13.0),
        Spacer(grow=True),
        Actions([Button("Add item", START, style="primary"),
                 Button("Toggle", TOGGLE, disabled=not rows),
                 Button("Delete", DELETE, style="danger", disabled=not rows)]),
        FooterKeys([("↑/↓", "select"), ("space", "toggle"), ("a", "add"), ("d", "delete")]),
    ], grow=True)
