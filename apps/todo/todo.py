#!/usr/bin/env python3
"""Todo - SDK v3 persisted list."""

from __future__ import annotations

from plexi_sdk import log, state
from plexi_sdk.effects import PersistState, SetStatus, SetTitle
from plexi_sdk.events import KeyEvent, UiAction, UiValueChange
from plexi_sdk.ui import AppBar, Button, Column, FooterKeys, SelectList, Spacer, Text, TextEdit

DEFAULT_ITEMS: list[dict] = []
DEFAULT_SELECTED = 0
DEFAULT_MODE = "list"
DEFAULT_DRAFT = ""


def init(size, args) -> list:
    data = _load()
    log.info(f"todo: initialized, {len(data['items'])} items")
    return [SetTitle("Todo"), SetStatus(_status(data)), PersistState(data)]


def update(event) -> list:
    data = _load()

    if isinstance(event, UiValueChange) and event.handler_id == "draft":
        data["draft"] = event.value
        return _save(data)

    if isinstance(event, UiAction):
        return _handle_action(data, event.handler_id)

    if not isinstance(event, KeyEvent) or not event.pressed:
        return []

    return _handle_key(data, event.key)


def view():
    data = _load()

    if data["mode"] == "add":
        return _add_view(data)

    return _list_view(data)


def _handle_action(data: dict, handler_id: str) -> list:
    if handler_id == "new":
        data["mode"] = "add"
        data["draft"] = ""
        return _save(data)
    if handler_id == "draft":
        return _commit_draft(data)
    if handler_id == "cancel":
        data["mode"] = "list"
        data["draft"] = ""
        return _save(data)
    if handler_id == "toggle":
        return _toggle(data)
    if handler_id == "delete":
        return _delete(data)
    return []


def _handle_key(data: dict, key: str) -> list:
    if data["mode"] == "add":
        if key == "escape":
            data["mode"] = "list"
            data["draft"] = ""
            return _save(data)
        return []

    items = data["items"]
    if key in ("j", "down"):
        data["selected"] = _clamp(data["selected"] + 1, len(items))
    elif key in ("k", "up"):
        data["selected"] = _clamp(data["selected"] - 1, len(items))
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


def _toggle(data: dict) -> list:
    items = data["items"]
    if not items:
        return []
    idx = data["selected"]
    items[idx] = {"text": items[idx]["text"], "done": not items[idx]["done"]}
    log.info(f"todo: toggled item {idx} -> done={items[idx]['done']}")
    return _save(data)


def _delete(data: dict) -> list:
    if not data["items"]:
        return []
    removed = data["items"].pop(data["selected"])
    log.info(f"todo: deleted '{removed['text']}'")
    return _save(data)


def _commit_draft(data: dict) -> list:
    text = data["draft"].strip()
    if text:
        data["items"].append({"text": text, "done": False})
        data["selected"] = len(data["items"]) - 1
        log.info(f"todo: added '{text}'")
    data["mode"] = "list"
    data["draft"] = ""
    return _save(data)


def _list_view(data: dict) -> Column:
    items = data["items"]
    if items:
        rows = [
            {
                "name": it["text"],
                "description": "done" if it["done"] else "open",
                "leading": "[x]" if it["done"] else "[ ]",
            }
            for it in items
        ]
        body = SelectList(rows, selected_idx=data["selected"])
    else:
        body = Column(
            [Spacer(size=24), Text("No items yet", size=16.0, bold=True), Text("Press n to add one.", size=12.0)],
            grow=True,
            padding=16,
        )

    return Column(
        [
            AppBar("Todo", _status(data)),
            body,
            Button("New", "new", style="primary"),
            Button("Toggle", "toggle", disabled=not items),
            Button("Delete", "delete", style="danger", disabled=not items),
            FooterKeys([("j/k", "navigate"), ("space", "toggle"), ("n", "new"), ("d", "delete")]),
        ],
        grow=True,
        padding=0,
    )


def _add_view(data: dict) -> Column:
    return Column(
        [
            AppBar("Todo", "New item"),
            TextEdit("draft", value=data["draft"], placeholder="What needs doing?"),
            Button("Add", "draft", style="primary", disabled=not data["draft"].strip()),
            Button("Cancel", "cancel", style="ghost"),
            FooterKeys([("enter", "add"), ("esc", "cancel")]),
        ],
        grow=True,
        padding=0,
    )


def _load() -> dict:
    items = state.get("items", DEFAULT_ITEMS)
    if not isinstance(items, list):
        items = []
    items = [_normalize(it) for it in items]
    selected = _clamp(int(state.get("selected", DEFAULT_SELECTED) or 0), len(items))
    mode = state.get("mode", DEFAULT_MODE)
    if mode != "add":
        mode = "list"
    draft = str(state.get("draft", DEFAULT_DRAFT) or "")
    return {"items": items, "selected": selected, "mode": mode, "draft": draft}


def _normalize(item) -> dict:
    if isinstance(item, dict):
        return {"text": str(item.get("text") or ""), "done": bool(item.get("done"))}
    return {"text": str(item), "done": False}


def _save(data: dict) -> list:
    data["selected"] = _clamp(data["selected"], len(data["items"]))
    return [PersistState(data), SetStatus(_status(data))]


def _status(data: dict) -> str:
    total = len(data["items"])
    done = sum(1 for it in data["items"] if it["done"])
    return f"{done}/{total} done"


def _clamp(idx: int, total: int) -> int:
    if total <= 0:
        return 0
    return max(0, min(idx, total - 1))
