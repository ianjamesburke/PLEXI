#!/usr/bin/env python3
"""Permissions — live view of the permission monitor.

Rows come from the host. This app does not keep a second grant store.
"""

from __future__ import annotations

import json
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
from plexi_sdk.ui import Actions, SurfaceHeader, Button, Column, FooterKeys, Keyed, SelectList, Spacer, Text

POLL_TIMER = 1

DEFAULT_STATE: dict[str, Any] = {
    "entries": [],
    "selected": 0,
    "selected_id": None,
    "mode": "list",
    "notice": "",
    "inventory_seen": False,
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
        entries, invalid_notice = _entries(event.entries)
        selected_id = data["selected_id"]
        if not data["inventory_seen"] and selected_id is None and entries:
            selected_id = entries[0]["id"]
        if not any(row["id"] == selected_id for row in entries):
            selected_id = None
        if event.notice:
            notice = event.notice
        elif event.status == "list":
            notice = ""
        else:
            notice = data["notice"]
        if invalid_notice:
            notice = invalid_notice
        log.debug(
            f"permissions: inventory count={len(entries)} status={event.status or 'list'}"
        )
        if (
            data["inventory_seen"]
            and entries == data["entries"]
            and selected_id == data["selected_id"]
            and notice == data["notice"]
        ):
            return []
        data["entries"] = entries
        data["selected_id"] = selected_id
        data["notice"] = notice
        data["inventory_seen"] = True
        return _commit(data)

    data = _state()
    action = _action(event)
    if action is None:
        return []

    if action == "reload":
        log.info("permissions: reload")
        return [ReadPermissionDecisions()]

    if action == "detail" or action.startswith("detail:"):
        entry_id = data["selected_id"] if isinstance(event, KeyEvent) else action.removeprefix("detail:")
        if not any(row["id"] == entry_id for row in data["entries"]):
            data["notice"] = "That permission entry is no longer available. Choose an entry again."
            return _commit(data)
        data["selected_id"] = entry_id
        data["mode"] = "detail"
        data["notice"] = ""
        return _commit(data)

    if action == "back":
        data["mode"] = "list"
        return _commit(data)

    if action.startswith("select:"):
        # SelectList appends a row index; resolve it in the rendered snapshot,
        # never in an inventory that may have changed since that click.
        try:
            snapshot, idx = action.removeprefix("select:").rsplit(":", 1)
            entry_id = json.loads(snapshot)[int(idx)]
        except (ValueError, IndexError, TypeError):
            return []
        if not any(row["id"] == entry_id for row in data["entries"]):
            data["notice"] = "That permission entry is no longer available. Choose an entry again."
            return _commit(data)
        data["selected_id"] = entry_id
        log.info(f"permissions: selected entry {entry_id}")
        return _commit(data)

    if action in {"up", "down"}:
        idx = _clamp(data["selected"] + (-1 if action == "up" else 1), len(data["entries"]))
        data["selected_id"] = data["entries"][idx]["id"] if data["entries"] else None
        return _commit(data)

    decision, _, entry_id = action.partition(":")
    if decision in {"revoke", "reset", "allow"}:
        # Keyboard shortcuts address the current selection. Button events must
        # carry the identity captured when rendered, never a positional fallback.
        if isinstance(event, KeyEvent):
            entry_id = data["selected_id"]
        entry = next((row for row in data["entries"] if row["id"] == entry_id), None)
        if entry is None:
            log.info(f"permissions: refused {decision}; entry no longer available")
            data["notice"] = "That permission entry is no longer available. Reload and choose an entry again."
            return _commit(data)
        if decision == "allow" and entry["kind"] not in {"deny", "pending"}:
            log.info(f"permissions: refused allow for kind {entry['kind']}")
            data["notice"] = "Allow is only available for denied or pending entries."
            return _commit(data)
        log.info(f"permissions: {decision} {entry['id']}")
        return [PermissionDecision(id=entry["id"], action=decision)]

    return []


def view():
    data = _state()
    if data["mode"] == "detail":
        return _detail_view(data)
    return _list_view(data)


def _state() -> dict:
    data = dict(DEFAULT_STATE)
    for key, value in DEFAULT_STATE.items():
        data[key] = state.get(key, value)
    data["entries"], invalid_notice = _entries(data.get("entries") or [])
    # Migrate old state once; a cleared selection after a poll stays cleared.
    if data["selected_id"] is None and not data["inventory_seen"] and data["entries"]:
        idx = _clamp(int(data.get("selected") or 0), len(data["entries"]))
        data["selected_id"] = data["entries"][idx]["id"]
    _sync_selection(data)
    data["mode"] = data.get("mode") if data.get("mode") in {"list", "detail"} else "list"
    data["notice"] = str(data.get("notice") or "")
    data["inventory_seen"] = bool(data.get("inventory_seen"))
    if invalid_notice:
        data["notice"] = invalid_notice
    return data


def _entries(rows: list) -> tuple[list[dict], str]:
    entries = [entry for row in rows if (entry := _entry(row)) is not None]
    seen: set[str] = set()
    duplicates: set[str] = set()
    for entry in entries:
        if entry["id"] in seen:
            duplicates.add(entry["id"])
        seen.add(entry["id"])
    if duplicates:
        log.info("permissions: rejected duplicate entry ids")
        return [entry for entry in entries if entry["id"] not in duplicates], "Duplicate permission entry ids were rejected. Reload the inventory."
    return entries, ""


def _sync_selection(data: dict) -> None:
    data["selected"] = next((idx for idx, row in enumerate(data["entries"])
                             if row["id"] == data["selected_id"]), -1)
    if data["selected"] == -1:
        data["selected_id"] = None



def _entry(row: dict) -> dict | None:
    entry_id = row.get("id")
    if not isinstance(entry_id, str) or not entry_id.strip():
        log.info("permissions: rejected entry without a valid id")
        return None
    return {
        "id": entry_id,
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
    _sync_selection(data)
    return [SetState(data), SetStatus(_status(data))]


def _list_view(data: dict):
    rows = [
        {
            "name": f"{entry['tool']}  {entry['kind']}",
            "description": f"{entry['actor_id']}  {entry['id']}  {entry['when']}",
        }
        for entry in data["entries"]
    ]
    if rows:
        body: Any = SelectList(
            rows,
            selected_idx=data["selected"],
            on_select=f"permissions:select:{json.dumps([entry['id'] for entry in data['entries']])}",
        )
    elif not data["inventory_seen"]:
        body = Text("Waiting for permission decisions…", size=12.0)
    else:
        body = Text("No permission decisions.", size=12.0)
    return Column(
        [
            SurfaceHeader("Permissions", "Live decisions from the permission monitor."),
            body,
            Text(data["notice"], size=11.0) if data["notice"] else Spacer(size=0.0),
            Spacer(grow=True),
            Actions(
                [
                    Button("Open", f"permissions:detail:{data['selected_id'] or ''}", disabled=data["selected_id"] is None),
                    Button("Reload", "permissions:reload"),
                ]
            ),
            FooterKeys([("j/k", "select"), ("enter", "open"), ("r", "reload")]),
        ],
        gap=8.0,
        grow=True,
    )


def _detail_view(data: dict):
    if data["selected_id"] is None:
        return Column(
            [
                SurfaceHeader("Permissions", "The selected entry is no longer available."),
                Text("Choose another entry from the list.", size=12.0),
                Text(data["notice"], size=11.0) if data["notice"] else Spacer(size=0.0),
                Spacer(grow=True),
                Keyed(Actions([Keyed(Button("Back", "permissions:back"), key="back")]), key="actions"),
                FooterKeys([("esc", "back")]),
            ],
            gap=8.0,
            grow=True,
        )
    entry = data["entries"][data["selected"]]
    kind = entry["kind"]
    resource = entry["resource_id"] or "any"
    buttons = []
    if kind in {"allow", "pending"}:
        buttons.append(Button("Revoke", f"permissions:revoke:{entry['id']}", style="danger"))
    if kind == "deny":
        buttons.append(Button("Ask again", f"permissions:reset:{entry['id']}"))
        buttons.append(Button("Allow", f"permissions:allow:{entry['id']}"))
    if kind == "pending":
        buttons.append(Button("Allow", f"permissions:allow:{entry['id']}"))
    # Inventory polling can replace the selected decision and change the action
    # count. Decision actions belong to that entry; Back belongs to this view.
    buttons = [Keyed(button, key=f"{entry['id']}:{button.on_click}") for button in buttons]
    buttons.append(Keyed(Button("Back", "permissions:back"), key="back"))
    return Column(
        [
            SurfaceHeader("Permissions", f"{entry['tool']}  {kind}"),
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
            Keyed(Actions(buttons), key="actions"),
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
    if data["mode"] == "detail" and data["selected_id"] is not None:
        entry = data["entries"][data["selected"]]
        return f"{entry['tool']} {entry['kind']}"
    return f"{count} decisions"


def _clamp(selected: int, total: int) -> int:
    if total <= 0:
        return 0
    return max(0, min(selected, total - 1))
