#!/usr/bin/env python3
"""Todo - persisted SDK v3 app."""

from __future__ import annotations

from plexi_sdk import log, state
from plexi_sdk.effects import PersistState, SetStatus, SetTitle
from plexi_sdk.events import KeyEvent, UiAction, UiValueChange
from plexi_sdk.ui import ActionBar, AppBar, Button, Column, FooterKeys, SelectList, TextEdit

MODE_LIST = "list"
MODE_ADD = "add"

DRAFT_ID = "todo-draft"

DEFAULT_STATE = {
    "items": [],
    "selected": 0,
    "mode": MODE_LIST,
    "draft": "",
}


def init(size, args) -> list:
    data = _read_state()
    log.info(f"todo: initialized items={len(data['items'])} mode={data['mode']}")
    return [SetTitle("Todo"), SetStatus(_status(data)), PersistState(data)]


def update(event) -> list:
    data = _read_state()

    if isinstance(event, UiValueChange) and event.handler_id == DRAFT_ID:
        data["draft"] = event.value
        return _persist(data, "draft")

    if isinstance(event, UiAction):
        if event.handler_id == "todo-new":
            return _start_add(data)
        if event.handler_id in ("todo-add", DRAFT_ID):
            return _add_item(data)
        if event.handler_id == "todo-cancel":
            return _cancel_add(data)
        if event.handler_id == "todo-toggle":
            return _toggle_selected(data)
        if event.handler_id == "todo-delete":
            return _delete_selected(data)

    if not isinstance(event, KeyEvent) or not event.pressed:
        return []

    key = _key(event.key)
    if data["mode"] == MODE_ADD:
        if key == "escape":
            return _cancel_add(data)
        if key == "enter":
            return _add_item(data)
        return []

    if key in ("j", "down"):
        return _move_selection(data, 1)
    if key in ("k", "up"):
        return _move_selection(data, -1)
    if key in ("n", "a"):
        return _start_add(data)
    if key in ("space", "enter"):
        return _toggle_selected(data)
    if key in ("d", "x", "backspace", "delete"):
        return _delete_selected(data)
    return []


def view():
    data = _read_state()
    if data["mode"] == MODE_ADD:
        return Column(
            [
                AppBar("Todo", "New item"),
                TextEdit(DRAFT_ID, value=data["draft"], placeholder="What needs doing?"),
                ActionBar(
                    [
                        Button(
                            "Add",
                            "todo-add",
                            style="primary",
                            disabled=not data["draft"].strip(),
                        ),
                        Button("Cancel", "todo-cancel", style="ghost"),
                    ]
                ),
                FooterKeys([("enter", "add"), ("esc", "cancel")]),
            ],
            grow=True,
            padding=0,
        )

    rows = _rows(data["items"])
    return Column(
        [
            AppBar("Todo", _status(data)),
            SelectList(rows, selected_idx=data["selected"]),
            ActionBar(
                [
                    Button("New", "todo-new", style="primary"),
                    Button("Toggle", "todo-toggle", disabled=not rows),
                    Button("Delete", "todo-delete", style="danger", disabled=not rows),
                ]
            ),
            FooterKeys(
                [
                    ("j/k", "select"),
                    (["space", "enter"], "toggle"),
                    ("n", "new"),
                    ("d", "delete"),
                ]
            ),
        ],
        grow=True,
        padding=0,
    )


def _read_state() -> dict:
    data = dict(DEFAULT_STATE)
    for key, fallback in DEFAULT_STATE.items():
        data[key] = state.get(key, fallback)

    items = [_normalize_item(item) for item in data.get("items") or []]
    data["items"] = [item for item in items if item["text"]]
    data["selected"] = _clamp_index(data.get("selected"), len(data["items"]))
    data["mode"] = MODE_ADD if data.get("mode") == MODE_ADD else MODE_LIST
    data["draft"] = str(data.get("draft") or "")
    return data


def _normalize_item(item) -> dict:
    if isinstance(item, dict):
        text = str(item.get("text") or "").strip()
        done = bool(item.get("done"))
    else:
        text = str(item).strip()
        done = False
    return {"text": text, "done": done}


def _rows(items: list[dict]) -> list[dict]:
    return [
        {
            "name": item["text"],
            "description": "done" if item["done"] else "open",
            "leading": "[x]" if item["done"] else "[ ]",
        }
        for item in items
    ]


def _start_add(data: dict) -> list:
    data["mode"] = MODE_ADD
    data["draft"] = ""
    return _persist(data, "start_add")


def _cancel_add(data: dict) -> list:
    data["mode"] = MODE_LIST
    data["draft"] = ""
    return _persist(data, "cancel_add")


def _add_item(data: dict) -> list:
    text = data["draft"].strip()
    if text:
        data["items"].append({"text": text, "done": False})
        data["selected"] = len(data["items"]) - 1
    data["mode"] = MODE_LIST
    data["draft"] = ""
    return _persist(data, "add")


def _move_selection(data: dict, delta: int) -> list:
    if not data["items"]:
        return []
    data["selected"] = _clamp_index(data["selected"] + delta, len(data["items"]))
    return _persist(data, "select")


def _toggle_selected(data: dict) -> list:
    selected = data["selected"]
    if not data["items"] or selected >= len(data["items"]):
        return []
    item = dict(data["items"][selected])
    item["done"] = not item["done"]
    data["items"][selected] = item
    return _persist(data, "toggle")


def _delete_selected(data: dict) -> list:
    selected = data["selected"]
    if not data["items"] or selected >= len(data["items"]):
        return []
    del data["items"][selected]
    data["selected"] = _clamp_index(selected, len(data["items"]))
    return _persist(data, "delete")


def _persist(data: dict, action: str) -> list:
    data["selected"] = _clamp_index(data["selected"], len(data["items"]))
    log.info(
        f"todo: {action} items={len(data['items'])} selected={data['selected']} mode={data['mode']}"
    )
    return [PersistState(data), SetStatus(_status(data))]


def _status(data: dict) -> str:
    total = len(data["items"])
    done = sum(1 for item in data["items"] if item["done"])
    return f"{done}/{total} done"


def _clamp_index(value, total: int) -> int:
    if total <= 0:
        return 0
    try:
        selected = int(value)
    except (TypeError, ValueError):
        selected = 0
    return max(0, min(selected, total - 1))


def _key(key: str) -> str:
    normalized = str(key or "").lower()
    return {
        "arrowdown": "down",
        "arrowup": "up",
        "return": "enter",
        "esc": "escape",
        " ": "space",
    }.get(normalized, normalized)
