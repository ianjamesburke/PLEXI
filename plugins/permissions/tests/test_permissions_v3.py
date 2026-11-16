from __future__ import annotations

import plexi_sdk as sdk
from plexi_sdk import _v3_state
from plexi_sdk.effects import PermissionDecision, ReadPermissionDecisions, SetState
from plexi_sdk.events import KeyEvent, PermissionInventory, UiAction

import main as permissions


def test_back_node_survives_inventory_rebuild_after_revoke() -> None:
    from plexi_sdk._adapter import _encode_uitree
    from plexi_sdk._v3_runtime import PluginRuntime
    from plexi_sdk.events import TimerFired

    runtime = PluginRuntime.__new__(PluginRuntime)
    runtime._last_tree = None
    runtime._next_tree_node_id = 0
    pending = {"id": "ask", "kind": "pending", "tool": "probe.pending"}
    timer = {"id": "timer", "kind": "allow", "tool": "timer"}
    _set_state({**permissions.DEFAULT_STATE, "mode": "detail", "entries": [pending, timer]})
    before = runtime._stabilize_tree_ids(_encode_uitree(permissions.view()))
    runtime._last_tree = before
    back = next(node for node in before["nodes"] if node["data"].get("label") == "Back")

    # Force the poll's response between resolving Back and dispatching its id.
    assert isinstance(permissions.update(TimerFired(id=permissions.POLL_TIMER))[0], ReadPermissionDecisions)
    _set_state(_state_effect(permissions.update(PermissionInventory(entries=[timer]))))
    after = runtime._stabilize_tree_ids(_encode_uitree(permissions.view()))
    current = next(node for node in after["nodes"] if node["data"].get("label") == "Back")
    assert back["id"] == current["id"], (back, current)
    old_actions = {node["id"] for node in before["nodes"]
                   if node["data"].get("on_click") in {"permissions:allow", "permissions:revoke"}}
    assert old_actions.isdisjoint(node["id"] for node in after["nodes"])

    # A later poll can add a denial before the timer grant. Back still denotes
    # navigation, while none of the prior decision's actions may be retargeted.
    runtime._last_tree = after
    denial = {"id": "denial", "kind": "deny", "tool": "probe.denied"}
    _set_state(_state_effect(permissions.update(PermissionInventory(entries=[denial, timer]))))
    rebuilt = runtime._stabilize_tree_ids(_encode_uitree(permissions.view()))
    assert next(node for node in rebuilt["nodes"] if node["id"] == back["id"])["data"]["on_click"] == "permissions:back"


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

    effects = permissions.update(UiAction("permissions:select:1"))
    data = _state_effect(effects)
    assert data["selected"] == 1

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


def test_list_and_detail_views_use_app_bar_and_actions() -> None:
    from plexi_sdk.ui import Actions, SurfaceHeader

    _set_state(_sample_state())
    list_view = permissions.view()
    assert isinstance(list_view.children[0], SurfaceHeader)
    assert any(isinstance(child, Actions) for child in list_view.children)
    list_node = list_view.to_node()
    header = list_node["children"][0]
    assert header["type"] == "surface_header"
    assert header["title"] == "Permissions"

    _set_state({**_sample_state(), "mode": "detail"})
    detail_view = permissions.view()
    assert isinstance(detail_view.children[0], SurfaceHeader)
    assert any(isinstance(child, Actions) for child in detail_view.children)
    detail_node = detail_view.to_node()
    detail_header = detail_node["children"][0]
    assert detail_header["type"] == "surface_header"
    assert detail_header["title"] == "Permissions"

