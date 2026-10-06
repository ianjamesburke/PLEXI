#!/usr/bin/env python3
"""Permissions — live view of the permission monitor.

Rows come from the host. This app does not keep a second grant store.
"""

from __future__ import annotations

from typing import Any

from plexi_sdk import log
from plexi_sdk import state
from plexi_sdk.effects import (
    PermissionDecision,
    ReadPermissionDecisions,
    SetState,
    SetStatus,
    SetTimer,
    SetTitle,
)
from plexi_sdk.events import KeyEvent, PermissionInventory, TimerFired, UiAction
from plexi_sdk.ui import Button, Column, FooterKeys, SelectList, Spacer, Text

POLL_TIMER = 1

DEFAULT_STATE: dict[str, Any] = {
    "entries": [],
    "selected": 0,
    "mode": "list",
    "notice": "",
}


def init(size, args) -> list:
    data = _state()
    log.info(f"permissions: opened with {len(data['entries'])} decisions")
    return [
        SetTitle("Permissions"),
        SetStatus(_status(data)),
        ReadPermissionDecisions(),
        SetTimer(id=POLL_TIMER, delay_ms=1000, repeat=True),
    ]


def update(event) -> list:
    if isinstance(event, TimerFired) and event.id == POLL_TIMER:
        return [ReadPermissionDecisions()]

    if isinstance(event, PermissionInventory):
        data = _state()
        data["entries"] = [_entry(row) for row in event.entries]
        data["selected"] = _clamp(data["selected"], len(data["entries"]))
        if event.notice:
            data["notice"] = event.notice
        elif event.status == "list":
            data["notice"] = ""
        log.info(
            f"permissions: inventory count={len(data['entries'])} status={event.status or 'list'}"
        )
        return _commit(data)

    data = _state()
    action = _action(event)
    if action is None:
        return []

    if action == "reload":
        log.info("permissions: reload")
        return [ReadPermissionDecisions()]

    if action == "detail" and data["entries"]:
        data["mode"] = "detail"
        data["notice"] = ""
        return _commit(data)

    if action == "back":
        data["mode"] = "list"
        return _commit(data)

    if action == "up":
        data["selected"] = _clamp(data["selected"] - 1, len(data["entries"]))
        return _commit(data)

    if action == "down":
        data["selected"] = _clamp(data["selected"] + 1, len(data["entries"]))
        return _commit(data)

    if action in {"revoke", "reset", "allow"} and data["entries"]:
        entry = data["entries"][data["selected"]]
        log.info(f"permissions: {action} {entry['id']}")
        return [PermissionDecision(id=entry["id"], action=action)]

    return []


def view():
    data = _state()
    if data["mode"] == "detail" and data["entries"]:
        return _detail_view(data)
    return _list_view(data)


def _state() -> dict:
    data = dict(DEFAULT_STATE)
    for key, value in DEFAULT_STATE.items():
        data[key] = state.get(key, value)
    data["entries"] = [_entry(row) for row in data.get("entries") or []]
    data["selected"] = _clamp(int(data.get("selected") or 0), len(data["entries"]))
    data["mode"] = data.get("mode") if data.get("mode") in {"list", "detail"} else "list"
    data["notice"] = str(data.get("notice") or "")
    return data


def _entry(row: dict) -> dict:
    return {
        "id": str(row.get("id") or ""),
        "kind": str(row.get("kind") or ""),
        "duration": str(row.get("duration") or ""),
        "actor_id": str(row.get("actor_id") or ""),
        "actor_type": str(row.get("actor_type") or ""),
        "tool": str(row.get("tool") or ""),
        "resource_id": str(row.get("resource_id") or ""),
        "when": str(row.get("when") or ""),
        "source": str(row.get("source") or ""),
        "workspace": str(row.get("workspace") or ""),
        "summary": str(row.get("summary") or ""),
    }


def _commit(data: dict) -> list:
    data["selected"] = _clamp(data["selected"], len(data["entries"]))
    return [SetState(data), SetStatus(_status(data))]


def _list_view(data: dict):
    rows = [
        {
            "name": f"{entry['tool']}  {entry['kind']}",
            "description": f"{entry['actor_id']}  {entry['id']}  {entry['when']}",
        }
        for entry in data["entries"]
    ]
    body = (
        SelectList(rows, selected_idx=data["selected"])
        if rows
        else Text("No permission decisions.", size=12.0)
    )
    return Column(
        [
            Text("Permissions", bold=True, size=15.0),
            Text("Live decisions from the permission monitor.", size=11.0),
            body,
            Text(data["notice"], size=11.0) if data["notice"] else Spacer(size=0.0),
            Spacer(grow=True),
            Button("Open", "permissions:detail", disabled=not rows),
            Button("Reload", "permissions:reload"),
            FooterKeys([("j/k", "select"), ("enter", "open"), ("r", "reload")]),
        ],
        gap=8.0,
        grow=True,
    )


def _detail_view(data: dict):
    entry = data["entries"][data["selected"]]
    kind = entry["kind"]
    resource = entry["resource_id"] or "any"
    buttons = []
    if kind in {"allow", "pending"}:
        buttons.append(Button("Revoke", "permissions:revoke", style="danger"))
    if kind == "deny":
        buttons.append(Button("Ask again", "permissions:reset"))
        buttons.append(Button("Allow", "permissions:allow"))
    if kind == "pending":
        buttons.append(Button("Allow", "permissions:allow"))
    buttons.append(Button("Back", "permissions:back"))
    return Column(
        [
            Text("Permission", bold=True, size=15.0),
            Text(f"{entry['tool']}  {kind}", bold=True, size=16.0),
            Text(f"Who: {entry['actor_id'] or '-'}", size=12.0),
            Text(f"What: {entry['tool']}  {entry['duration']}", size=12.0),
            Text(f"Resource: {resource}", size=12.0),
            Text(f"When: {entry['when'] or '-'}", size=12.0),
            Text(f"Id: {entry['id']}", size=12.0),
            Text(f"Source: {entry['source'] or '-'}", size=12.0),
            Text(f"Workspace: {entry['workspace'] or '-'}", size=12.0),
            Text(data["notice"], size=11.0) if data["notice"] else Spacer(size=0.0),
            Spacer(grow=True),
            *buttons,
            FooterKeys([("x", "revoke"), ("esc", "back")]),
        ],
        gap=8.0,
        grow=True,
    )


def _action(event) -> str | None:
    if isinstance(event, UiAction) and event.handler_id.startswith("permissions:"):
        return event.handler_id.removeprefix("permissions:")
    if not isinstance(event, KeyEvent) or not event.pressed:
        return None
    if event.key in {"up", "k", "ArrowUp"}:
        return "up"
    if event.key in {"down", "j", "ArrowDown"}:
        return "down"
    if event.key in {"return", "enter"}:
        return "detail"
    if event.key in {"escape", "h", "left", "ArrowLeft"}:
        return "back"
    if event.key == "r":
        return "reload"
    if event.key == "x":
        return "revoke"
    return None


def _status(data: dict) -> str:
    count = len(data["entries"])
    if data["mode"] == "detail" and count:
        entry = data["entries"][data["selected"]]
        return f"{entry['tool']} {entry['kind']}"
    return f"{count} decisions"


def _clamp(selected: int, total: int) -> int:
    if total <= 0:
        return 0
    return max(0, min(selected, total - 1))
