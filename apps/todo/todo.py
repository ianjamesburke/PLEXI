#!/usr/bin/env python3
"""Todo — the reference PGAP app: every need is an SDK primitive, none is local.

State is context-scoped and persisted, so a pane reopened in the same context
shows the same list. The Assistant reads and writes that list through the tools
declared below whether or not the app is on screen.
"""

from __future__ import annotations

from typing import Any

from plexi_sdk import log, state, tools
from plexi_sdk.effects import PersistState, SetStatus, SetTitle
from plexi_sdk.events import KeyEvent, UiAction, UiValueChange
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

DEFAULT_TODO_STATE: dict[str, Any] = {
    "items": [],
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


def _data() -> dict:
    data = {key: state.get(key, value) for key, value in DEFAULT_TODO_STATE.items()}
    data["items"] = [dict(item) for item in data["items"] or []]
    data["selected"] = _clamp(data["items"], int(data["selected"] or 0))
    data["adding"] = bool(data["adding"])
    data["draft"] = str(data["draft"] or "")
    return data


def _clamp(items: list, selected: int) -> int:
    return max(0, min(selected, len(items) - 1)) if items else 0


def _save(data: dict) -> list:
    """Persist the whole snapshot. `PersistState` is the durable, context-scoped
    write — the same keys `view()` reads back after a restart."""
    data["selected"] = _clamp(data["items"], data["selected"])
    return [PersistState(data), SetStatus(_status(data))]


def _status(data: dict) -> str:
    open_count = sum(1 for item in data["items"] if not item["done"])
    return f"{open_count} open · {len(data['items'])} total"


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
    return tools.Reply({"count": len(data["items"])}, _save(data))


@tools.tool("todo.set_done", "Mark the todo item at `index` done or not done.",
            {"index": int, "done": bool})
def _tool_set_done(index: int, done: bool) -> tools.Reply:
    data = _data()
    if not 0 <= index < len(data["items"]):
        raise IndexError(f"no todo item at index {index} ({len(data['items'])} items)")
    data["items"][index]["done"] = done
    log.info(f"todo: item {index} done={done} via assistant")
    return tools.Reply({"text": data["items"][index]["text"], "done": done}, _save(data))


@tools.tool("todo.remove", "Remove the todo item at `index`.", {"index": int})
def _tool_remove(index: int) -> tools.Reply:
    data = _data()
    if not 0 <= index < len(data["items"]):
        raise IndexError(f"no todo item at index {index} ({len(data['items'])} items)")
    removed = data["items"].pop(index)
    log.info(f"todo: removed item {index} via assistant")
    return tools.Reply({"removed": removed["text"]}, _save(data))


def init(size, args) -> list:
    data = _data()
    log.info("todo: ready")
    effects: list = [SetTitle("Todo"), SetStatus(_status(data)), tools.expose()]
    missing = {k: v for k, v in DEFAULT_TODO_STATE.items() if state.get(k, None) is None}
    if missing:
        effects.append(PersistState(missing))
    return effects


def update(event) -> list:
    handled = tools.dispatch(event)
    if handled is not None:
        return handled

    data = _data()

    if isinstance(event, UiValueChange) and event.handler_id == DRAFT:
        data["draft"] = event.value
        return _save(data)

    action = event.handler_id if isinstance(event, UiAction) else None
    key = event.key if isinstance(event, KeyEvent) and event.pressed else None

    if action == ADD or (data["adding"] and key == "enter"):
        return _save(_add(data, data["draft"]))
    if action == CANCEL or (data["adding"] and key == "escape"):
        data["adding"] = False
        data["draft"] = ""
        return _save(data)
    if data["adding"]:
        return []

    if action == START or key == "a":
        data["adding"] = True
        data["draft"] = ""
        return _save(data)
    if action == TOGGLE or key == "space":
        if not data["items"]:
            return []
        item = data["items"][data["selected"]]
        item["done"] = not item["done"]
        return _save(data)
    if action == DELETE or key == "d":
        if not data["items"]:
            return []
        data["items"].pop(data["selected"])
        return _save(data)
    if key in ("up", "k"):
        data["selected"] = _clamp(data["items"], data["selected"] - 1)
        return _save(data)
    if key in ("down", "j"):
        data["selected"] = _clamp(data["items"], data["selected"] + 1)
        return _save(data)
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
