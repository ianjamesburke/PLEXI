from __future__ import annotations

import plexi_sdk as sdk
from plexi_sdk import _v3_state
from plexi_sdk.effects import PermissionDecision, ReadPermissionDecisions, SetState
from plexi_sdk.events import KeyEvent, PermissionInventory, UiAction

import main as permissions


def _set_state(values: dict) -> None:
    raw = {key: b"" for key in values}
    _v3_state._state = sdk.StateSnapshot(values, raw)
    _v3_state._in_view = False


def _state_effect(effects: list) -> dict:
    effect = next(effect for effect in effects if isinstance(effect, SetState))
    return effect.data


def _sample_state() -> dict:
    return {
        **permissions.DEFAULT_STATE,
        "entries": [
            {
                "id": "grant-allow",
                "kind": "allow",
                "duration": "until",
                "actor_id": "pane:1",
                "tool": "chess.state",
                "resource_id": "game-1",
                "when": "2026-10-06 00:00:00 UTC",
                "source": "user",
                "workspace": "/workspace",
            },
            {
                "id": "grant-deny",
                "kind": "deny",
                "duration": "always",
                "actor_id": "agent:assistant:2",
                "tool": "chess.legal_moves",
                "resource_id": "game-1",
                "when": "2026-10-06 00:01:00 UTC",
                "source": "user",
                "workspace": "/workspace",
            },
        ],
    }


def test_inventory_replaces_entries() -> None:
    _set_state(dict(permissions.DEFAULT_STATE))

    effects = permissions.update(
        PermissionInventory(
            entries=[
                {
                    "id": "grant-deny",
                    "kind": "deny",
                    "actor_id": "pane:4",
                    "tool": "chess.state",
                    "when": "2026-10-06 01:00:00 UTC",
                }
            ],
            notice="",
            status="list",
        )
    )
    data = _state_effect(effects)
    assert data["entries"][0]["id"] == "grant-deny"
    assert data["entries"][0]["kind"] == "deny"
    assert data["entries"][0]["tool"] == "chess.state"


def test_revoke_reset_and_allow_ask_the_host() -> None:
    _set_state(_sample_state())

    effects = permissions.update(KeyEvent("down"))
    data = _state_effect(effects)
    assert data["selected"] == 1

    _set_state(data)
    effects = permissions.update(UiAction("permissions:reset"))
    decision = next(effect for effect in effects if isinstance(effect, PermissionDecision))
    assert decision.action == "reset"
    assert decision.id == "grant-deny"

    _set_state(_sample_state())
    effects = permissions.update(UiAction("permissions:revoke"))
    decision = next(effect for effect in effects if isinstance(effect, PermissionDecision))
    assert decision.action == "revoke"
    assert decision.id == "grant-allow"

    _set_state(data)
    effects = permissions.update(UiAction("permissions:allow"))
    decision = next(effect for effect in effects if isinstance(effect, PermissionDecision))
    assert decision.action == "allow"
    assert decision.id == "grant-deny"


def test_init_reads_the_monitor() -> None:
    _set_state(_sample_state())
    effects = permissions.init((800, 600), [])
    assert any(isinstance(effect, ReadPermissionDecisions) for effect in effects)


def _texts(node) -> list[str]:
    raw = node.to_node() if hasattr(node, "to_node") else node
    found: list[str] = []

    def walk(value) -> None:
        if isinstance(value, dict):
            if value.get("type") == "text" and isinstance(value.get("text"), str):
                found.append(value["text"])
            for child in value.values():
                walk(child)
        elif isinstance(value, list):
            for child in value:
                walk(child)

    walk(raw)
    return found


def test_unchanged_inventory_does_not_repaint() -> None:
    _set_state(dict(permissions.DEFAULT_STATE))
    event = PermissionInventory(entries=[], notice="", status="list")
    first = permissions.update(event)
    assert any(isinstance(effect, SetState) for effect in first)
    _set_state(_state_effect(first))
    second = permissions.update(PermissionInventory(entries=[], notice="", status="list"))
    assert second == []


def test_empty_inventory_is_honest() -> None:
    _set_state(dict(permissions.DEFAULT_STATE))
    waiting = _texts(permissions.view())
    assert "Waiting for permission decisions…" in waiting
    assert "No permission decisions." not in waiting

    effects = permissions.update(PermissionInventory(entries=[], notice="", status="list"))
    _set_state(_state_effect(effects))
    empty = _texts(permissions.view())
    assert "No permission decisions." in empty
    assert "Waiting for permission decisions…" not in empty


def test_seeded_entries_show_before_inventory() -> None:
    _set_state(_sample_state())
    texts = _texts(permissions.view())
    assert "Waiting for permission decisions…" not in texts
    assert "No permission decisions." not in texts
