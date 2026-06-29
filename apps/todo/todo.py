#!/usr/bin/env python3
"""Todo - SDK v3 persisted list."""

from __future__ import annotations

from plexi_sdk import log, state
from plexi_sdk.effects import PersistState, SetStatus, SetTitle
from plexi_sdk.events import KeyEvent, UiAction, UiValueChange
from plexi_sdk.ui import (
    ActionBar,
    AppBar,
    Button,
    Column,
    FooterKeys,
    SelectList,
    Spacer,
    Text,
    TextEdit,
)

DEFAULT_STATE = {
    "items": [],
    "selected": 0,
    "mode": "list",
    "draft": "",
}


def init(size, args) -> list:
    data = _state()
    effects: list = [SetTitle("Todo"), SetStatus(_status(data))]
    if not state.get("items", None):
        effects.append(PersistState({"items": data["items"]}))
    log.info("todo: sdk v3 app initialized")
    return effects


def update(event) -> list:
    data = _state()

    if isinstance(event, UiValueChange) and event.handler_id == "todo-draft":
        data["draft"] = event.value
        return _save(data)

    if isinstance(event, UiAction):
        if event.handler_id == "todo-new":
            data["mode"] = "add"
            data["draft"] = ""
            return _save(data)
        if event.handler_id == "todo-draft":
            return _add(data)
        if event.handler_id == "todo-cancel":
            data["mode"] = "list"
            data["draft"] = ""
            return _save(data)
        if event.handler_id == "todo-toggle":
            return _toggle(data)
        if event.handler_id == "todo-delete":
            return _delete(data)

    if not isinstance(event, KeyEvent) or not event.pressed:
        return []

    key = event.key
    if data["mode"] == "add":
        if key == "escape":
            data["mode"] = "list"
            data["draft"] = ""
            return _save(data)
        return []

    if key in ("j", "down"):
        data["selected"] = _clamp(data["selected"] + 1, len(data["items"]))
    elif key in ("k", "up"):
        data["selected"] = _clamp(data["selected"] - 1, len(data["items"]))
    elif key in ("n", "a"):
        data["mode"] = "add"
        data["draft"] = ""
    elif key in ("space", "enter", "return"):
        return _toggle(data)
    elif key in ("d", "x", "backspace", "delete"):
        return _delete(data)
    else:
        return []
    return _save(data)


def view():
    data = _state()
    if data["mode"] == "add":
        return Column(
            [
                AppBar("Todo", "New item"),
                TextEdit(
                    "todo-draft", value=data["draft"], placeholder="What needs doing?"
                ),
                ActionBar(
                    [
                        Button(
                            "Add",
                            "todo-draft",
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

    rows = [
        {
            "name": item["text"],
            "description": "done" if item["done"] else "open",
            "leading": "[x]" if item["done"] else "[ ]",
        }
        for item in data["items"]
    ]
    body = SelectList(rows, selected_idx=data["selected"]) if rows else _empty()
    return Column(
        [
            AppBar("Todo", _status(data)),
            body,
            ActionBar(
                [
                    Button("New", "todo-new", style="primary"),
                    Button("Toggle", "todo-toggle", disabled=not rows),
                    Button("Delete", "todo-delete", style="danger", disabled=not rows),
                ]
            ),
            FooterKeys(
                [("j/k", "select"), ("enter", "toggle"), ("n", "new"), ("d", "delete")]
            ),
        ],
        grow=True,
        padding=0,
    )


def _state() -> dict:
    data = dict(DEFAULT_STATE)
    for key, value in DEFAULT_STATE.items():
        data[key] = state.get(key, value)
    data["items"] = [_normalize_item(item) for item in list(data.get("items") or [])]
    data["selected"] = _clamp(int(data.get("selected") or 0), len(data["items"]))
    data["mode"] = "add" if data.get("mode") == "add" else "list"
    data["draft"] = str(data.get("draft") or "")
    return data


def _normalize_item(item) -> dict:
    if isinstance(item, dict):
        return {"text": str(item.get("text") or ""), "done": bool(item.get("done"))}
    return {"text": str(item), "done": False}


def _save(data: dict) -> list:
    data["selected"] = _clamp(data["selected"], len(data["items"]))
    log.info(f"todo: state mode={data['mode']} items={len(data['items'])}")
    return [PersistState(data), SetStatus(_status(data))]


def _add(data: dict) -> list:
    text = data["draft"].strip()
    if text:
        data["items"].append({"text": text, "done": False})
        data["selected"] = len(data["items"]) - 1
    data["mode"] = "list"
    data["draft"] = ""
    return _save(data)


def _toggle(data: dict) -> list:
    if not data["items"]:
        return []
    item = dict(data["items"][data["selected"]])
    item["done"] = not item["done"]
    data["items"][data["selected"]] = item
    return _save(data)


def _delete(data: dict) -> list:
    if not data["items"]:
        return []
    data["items"].pop(data["selected"])
    return _save(data)


def _empty():
    return Column(
        [
            Spacer(size=24),
            Text("No todo items", size=16.0, bold=True),
            Text("Press n or click New.", size=12.0),
        ],
        grow=True,
        padding=16,
    )


def _status(data: dict) -> str:
    total = len(data["items"])
    done = sum(1 for item in data["items"] if item["done"])
    return f"{done}/{total} done"


def _clamp(selected: int, total: int) -> int:
    if total <= 0:
        return 0
    return max(0, min(selected, total - 1))
