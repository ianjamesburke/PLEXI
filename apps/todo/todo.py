#!/usr/bin/env python3
"""Todo — SDK v3 persisted list."""

from __future__ import annotations

from typing import Any

from plexi_sdk import state
from plexi_sdk.effects import PersistState, SetStatus, SetTitle
from plexi_sdk.events import KeyEvent, UiAction, UiValueChange
from plexi_sdk.ui import AppBar, Button, Column, FooterKeys, SelectList, Spacer, Text, TextInput

DEFAULT_TODO_STATE: dict[str, Any] = {
    "items": [],
    "selected": 0,
    "adding": False,
    "cwd": "",
    "draft": "",
}


def _app_state() -> dict:
    data = dict(DEFAULT_TODO_STATE)
    for key in data:
        data[key] = state.get(key, data[key])
    data["items"] = list(data.get("items") or [])
    data["selected"] = _clamp_selected(data["items"], int(data.get("selected") or 0))
    data["adding"] = bool(data.get("adding"))
    data["draft"] = str(data.get("draft") or "")
    data["cwd"] = str(data.get("cwd") or "")
    return data


def _set(data: dict) -> list[PersistState]:
    data["selected"] = _clamp_selected(data["items"], int(data.get("selected") or 0))
    return [PersistState(data)]


def _clamp_selected(items: list, selected: int) -> int:
    if not items:
        return 0
    return max(0, min(selected, len(items) - 1))


def _add_item(data: dict) -> list[PersistState]:
    text = data["draft"].strip()
    if text:
        data["items"].append({"text": text, "done": False})
        data["selected"] = len(data["items"]) - 1
    data["adding"] = False
    data["draft"] = ""
    return _set(data)


def init(size, args) -> list:
    data = _app_state()
    effects: list = [SetTitle("Todo"), SetStatus(f"{len(data['items'])} todo items")]
    missing = {
        key: value
        for key, value in DEFAULT_TODO_STATE.items()
        if state.get(key, None) is None
    }
    if missing:
        effects.append(PersistState(missing))
    return effects


def update(event) -> list:
    data = _app_state()

    if isinstance(event, UiValueChange) and event.handler_id == "todo-add:change":
        data["draft"] = event.value
        return _set(data)

    if isinstance(event, UiAction):
        if event.handler_id == "todo-add:start":
            data["adding"] = True
            data["draft"] = ""
            return _set(data)
        if event.handler_id == "todo-add:submit":
            return _add_item(data)
        if event.handler_id == "todo-add:cancel":
            data["adding"] = False
            data["draft"] = ""
            return _set(data)
        if event.handler_id == "todo-toggle":
            return _toggle_selected(data)
        if event.handler_id == "todo-delete":
            return _delete_selected(data)

    if not isinstance(event, KeyEvent) or not event.pressed:
        return []

    key = event.key
    if data["adding"]:
        if key == "escape":
            data["adding"] = False
            data["draft"] = ""
            return _set(data)
        return []

    if key in ("up", "k", "ArrowUp"):
        data["selected"] = _clamp_selected(data["items"], data["selected"] - 1)
        return _set(data)
    if key in ("down", "j", "ArrowDown"):
        data["selected"] = _clamp_selected(data["items"], data["selected"] + 1)
        return _set(data)
    if key == "space":
        return _toggle_selected(data)
    if key == "a":
        data["adding"] = True
        data["draft"] = ""
        return _set(data)
    if key == "d":
        return _delete_selected(data)
    return []


def _toggle_selected(data: dict) -> list:
    if not data["items"]:
        return []
    item = dict(data["items"][data["selected"]])
    item["done"] = not bool(item.get("done"))
    data["items"][data["selected"]] = item
    return _set(data)


def _delete_selected(data: dict) -> list:
    if not data["items"]:
        return []
    data["items"].pop(data["selected"])
    data["selected"] = _clamp_selected(data["items"], data["selected"])
    return _set(data)


def view():
    data = _app_state()
    if data["adding"]:
        return Column([
            AppBar("Todo", "Add item"),
            TextInput(
                "todo-add",
                value=data["draft"],
                placeholder="New item",
                on_change="todo-add:change",
                on_submit="todo-add:submit",
            ),
            Button("Add", "todo-add:submit", style="primary", disabled=not data["draft"].strip()),
            Button("Cancel", "todo-add:cancel", style="ghost"),
            Spacer(grow=True),
            FooterKeys([("↩", "save"), ("esc", "cancel")]),
        ], grow=True)

    rows = [
        {
            "name": f"{'[x]' if item.get('done') else '[ ]'} {item.get('text', '')}",
        }
        for item in data["items"]
    ]
    list_area = (
        SelectList(rows, selected_idx=data["selected"])
        if rows
        else Text("No items. Press 'a' to add.", size=12.0)
    )
    return Column([
        AppBar("Todo", f"{len(rows)} items"),
        list_area,
        Spacer(grow=True),
        Button("Add item", "todo-add:start", style="primary"),
        Button("Toggle selected", "todo-toggle", disabled=not rows),
        Button("Delete selected", "todo-delete", style="danger", disabled=not rows),
        FooterKeys([("↑/↓", "select"), ("space", "toggle"), ("a", "add"), ("d", "delete")]),
    ], grow=True)
