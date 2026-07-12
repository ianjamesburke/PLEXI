#!/usr/bin/env python3
"""Permissions — SDK v3 runtime-state permission grant browser."""

from __future__ import annotations

from pathlib import Path

from plexi_sdk import log, state
from plexi_sdk.effects import RequestCapability, SetState, SetStatus, SetTitle
from plexi_sdk.events import CapabilityDenied, CapabilityGranted, KeyEvent, UiAction
from plexi_sdk.ui import Button, Column, SelectList, Spacer, Text

DEFAULT_STATE = {
    "grants": [],
    "selected": 0,
    "path": "",
    "mode": "list",
    "notice": "",
    "can_manage": False,
}

STATE_LABELS = {
    "green": "allow",
    "yellow": "ask",
    "red": "block",
    "revoked": "revoked",
}


def init(size, args) -> list:
    data = _state()
    if not data["path"]:
        data["path"] = str(Path.cwd())
    missing = {key: data[key] for key in DEFAULT_STATE if state.get(key, None) is None}
    log.info("permissions: SDK v3 initialized")
    effects: list = [
        SetTitle("Permissions"),
        SetStatus(_status(data)),
        RequestCapability("permissions.manage"),
    ]
    if missing:
        effects.append(SetState(missing))
    return effects


def update(event) -> list:
    data = _state()

    if isinstance(event, CapabilityGranted) and event.name == "permissions.manage":
        data["can_manage"] = True
        data["notice"] = "permissions.manage granted. Waiting for host grant inventory."
        return _commit(data)

    if isinstance(event, CapabilityDenied) and event.name == "permissions.manage":
        data["can_manage"] = False
        data["notice"] = "permissions.manage denied."
        return _commit(data)

    action = _action(event)
    if action is None:
        return []

    if action == "reload":
        data["notice"] = "Reload requested. SDK v3 has no list-permissions effect yet."
        log.info("permissions: reload requested without v3 host list effect")
        return _commit(data)

    if action == "detail" and data["grants"]:
        data["mode"] = "detail"
        data["notice"] = ""
        return _commit(data)

    if action == "back":
        data["mode"] = "list"
        return _commit(data)

    if action == "revoke":
        return _revoke_selected(data)

    if action == "up":
        data["selected"] = _clamp(data["selected"] - 1, len(data["grants"]))
        return _commit(data)

    if action == "down":
        data["selected"] = _clamp(data["selected"] + 1, len(data["grants"]))
        return _commit(data)

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
    data["grants"] = [_grant(row) for row in data.get("grants") or []]
    data["selected"] = _clamp(int(data.get("selected") or 0), len(data["grants"]))
    data["path"] = str(data.get("path") or "")
    data["mode"] = (
        data.get("mode") if data.get("mode") in {"list", "detail"} else "list"
    )
    data["notice"] = str(data.get("notice") or "")
    data["can_manage"] = bool(data.get("can_manage"))
    return data


def _grant(row: dict) -> dict:
    return {
        "app_id": str(row.get("app_id") or row.get("app") or "?"),
        "capability": str(row.get("capability") or "?"),
        "state": str(row.get("state") or "yellow"),
        "workspace": str(row.get("workspace") or row.get("path") or ""),
        "description": str(row.get("description") or ""),
        "sensitive": bool(row.get("sensitive")),
        "stored": bool(row.get("stored", True)),
    }


def _commit(data: dict) -> list:
    data["selected"] = _clamp(data["selected"], len(data["grants"]))
    return [SetState(data), SetStatus(_status(data))]


def _revoke_selected(data: dict) -> list:
    if not data["grants"]:
        return []
    grant = dict(data["grants"][data["selected"]])
    grant["state"] = "revoked"
    data["grants"][data["selected"]] = grant
    data["notice"] = (
        "Revoke modeled locally. SDK v3 host revoke effect is not available."
    )
    log.info(
        "permissions: modeled revoke for "
        f"{grant['app_id']}/{grant['capability']} workspace={grant['workspace']!r}"
    )
    return _commit(data)


def _list_view(data: dict):
    rows = [
        {
            "name": f"{grant['app_id']}  {grant['capability']}",
            "description": _grant_summary(grant),
        }
        for grant in data["grants"]
    ]
    body = (
        SelectList(rows, selected_idx=data["selected"])
        if rows
        else Text("No permission grants in state.", size=12.0)
    )
    return Column(
        [
            Text("Permissions", bold=True, size=15.0),
            Text(data["path"] or "workspace unknown", size=11.0),
            body,
            Text(data["notice"], size=11.0) if data["notice"] else Spacer(size=0.0),
            Spacer(grow=True),
            Button("Open", "permissions:detail", disabled=not rows),
            Button("Reload", "permissions:reload"),
            Text("j/k selects. Enter opens. r reloads.", size=11.0),
        ],
        gap=8.0,
        grow=True,
    )


def _detail_view(data: dict):
    grant = data["grants"][data["selected"]] if data["grants"] else _grant({})
    return Column(
        [
            Text("Permission", bold=True, size=15.0),
            Text(f"{grant['app_id']} / {grant['capability']}", bold=True, size=16.0),
            Text(
                f"State: {STATE_LABELS.get(grant['state'], grant['state'])}", size=12.0
            ),
            Text(f"Workspace: {grant['workspace'] or data['path'] or '-'}", size=12.0),
            Text(f"Description: {grant['description'] or '-'}", size=12.0),
            Text(f"Stored: {'yes' if grant['stored'] else 'live only'}", size=12.0),
            Text(f"Sensitive: {'yes' if grant['sensitive'] else 'no'}", size=12.0),
            Text(data["notice"], size=11.0) if data["notice"] else Spacer(size=0.0),
            Spacer(grow=True),
            Button(
                "Revoke",
                "permissions:revoke",
                style="danger",
                disabled=not data["can_manage"],
            ),
            Button("Back", "permissions:back"),
            Text("x revokes. Esc returns.", size=11.0),
        ],
        gap=8.0,
        grow=True,
    )


def _grant_summary(grant: dict) -> str:
    state_label = STATE_LABELS.get(grant["state"], grant["state"])
    bits = [state_label]
    if grant["workspace"]:
        bits.append(grant["workspace"])
    if grant["sensitive"]:
        bits.append("sensitive")
    if not grant["stored"]:
        bits.append("live")
    return " | ".join(bits)


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
    grant_count = len(data["grants"])
    if data["mode"] == "detail" and grant_count:
        grant = data["grants"][data["selected"]]
        return f"{grant['app_id']} {grant['capability']}"
    return f"{grant_count} grants"


def _clamp(selected: int, total: int) -> int:
    if total <= 0:
        return 0
    return max(0, min(selected, total - 1))
