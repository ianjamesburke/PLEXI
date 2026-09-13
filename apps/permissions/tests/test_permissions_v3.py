from __future__ import annotations

import plexi_sdk as sdk
from plexi_sdk import _v3_state
from plexi_sdk.effects import SetState
from plexi_sdk.events import CapabilityGranted, KeyEvent, UiAction

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
        "path": "/workspace",
        "can_manage": True,
        "grants": [
            {
                "app_id": "todo",
                "capability": "fs.read",
                "state": "green",
                "workspace": "/workspace",
                "description": "Read files",
                "sensitive": False,
                "stored": True,
            },
            {
                "app_id": "kraken",
                "capability": "net.http",
                "state": "yellow",
                "workspace": "/workspace",
                "description": "Fetch prices",
                "sensitive": True,
                "stored": False,
            },
        ],
    }


def test_selection_detail_and_revoke_are_state_effects() -> None:
    _set_state(_sample_state())

    effects = permissions.update(KeyEvent("down"))
    data = _state_effect(effects)
    assert data["selected"] == 1

    _set_state(data)
    effects = permissions.update(KeyEvent("enter"))
    data = _state_effect(effects)
    assert data["mode"] == "detail"

    _set_state(data)
    effects = permissions.update(UiAction("permissions:revoke"))
    data = _state_effect(effects)
    assert data["grants"][1]["state"] == "revoked"
    assert "host revoke effect is not available" in data["notice"]


def test_capability_granted_updates_manage_state() -> None:
    _set_state(dict(permissions.DEFAULT_STATE))

    effects = permissions.update(CapabilityGranted("permissions.manage"))
    data = _state_effect(effects)

    assert data["can_manage"] is True
    assert "granted" in data["notice"]
