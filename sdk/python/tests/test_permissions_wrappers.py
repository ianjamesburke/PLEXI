"""Tests for the permissions.manage SDK wrappers (stint 0017).

Fakes the host: patches `plexi_sdk._emitter._emit` to capture the emitted
command and write the JSON response file the wrapper polls for.
"""

import asyncio
import json
from unittest.mock import patch

import pytest

from plexi_sdk._emitter import CAPABILITY_REGISTRY
from plexi_sdk._types import CapabilityDeniedError


def _make_app():
    from plexi_sdk._app import App

    app = App.__new__(App)
    App.__init__(app)
    return app


def _fake_host(reply: object):
    """Return (emitted_commands, _emit side effect) writing `reply` to the
    response file of every captured command, like the host would."""
    emitted: list[dict] = []

    def side_effect(cmd: dict) -> None:
        emitted.append(cmd)
        with open(cmd["response_file"], "w") as f:
            f.write(json.dumps(reply))

    return emitted, side_effect


# ── list_permissions ──────────────────────────────────────────────────────────

def test_list_permissions_emits_and_returns_host_reply():
    app = _make_app()
    reply = {
        "permissions": [{
            "app_id": "snake",
            "workspace": "/tmp/ws",
            "capability": "panes.read",
            "state": "green",
            "stored": True,
            "sensitive": True,
            "description": "Read pane state",
        }],
        "running": ["snake"],
    }
    emitted, side_effect = _fake_host(reply)
    with patch("plexi_sdk._emitter._emit", side_effect=side_effect):
        result = asyncio.run(app.emit.list_permissions())

    assert result == reply
    assert len(emitted) == 1
    assert emitted[0]["type"] == "list_permissions"
    assert emitted[0]["response_file"].endswith(".json")


def test_list_permissions_raises_on_capability_denial():
    app = _make_app()
    denial = {"capability": "permissions.manage", "error": "denied by user"}
    _, side_effect = _fake_host(denial)
    with patch("plexi_sdk._emitter._emit", side_effect=side_effect):
        with pytest.raises(CapabilityDeniedError, match="denied by user"):
            asyncio.run(app.emit.list_permissions())


# ── set_permission ────────────────────────────────────────────────────────────

def test_set_permission_ok_with_workspace():
    app = _make_app()
    emitted, side_effect = _fake_host({"ok": True})
    with patch("plexi_sdk._emitter._emit", side_effect=side_effect):
        result = asyncio.run(app.emit.set_permission(
            "snake", "panes.read", "red", workspace="/tmp/ws"))

    assert result == {"ok": True}
    assert len(emitted) == 1
    cmd = emitted[0]
    assert cmd["type"] == "set_permission"
    assert cmd["app_id"] == "snake"
    assert cmd["capability"] == "panes.read"
    assert cmd["state"] == "red"
    assert cmd["workspace"] == "/tmp/ws"


def test_set_permission_omits_workspace_when_none():
    app = _make_app()
    emitted, side_effect = _fake_host({"ok": True})
    with patch("plexi_sdk._emitter._emit", side_effect=side_effect):
        asyncio.run(app.emit.set_permission("snake", "panes.read", "green"))

    assert "workspace" not in emitted[0]


def test_set_permission_raises_runtime_error_on_host_error():
    app = _make_app()
    _, side_effect = _fake_host({"error": "unknown capability 'nope'"})
    with patch("plexi_sdk._emitter._emit", side_effect=side_effect):
        with pytest.raises(RuntimeError, match="unknown capability"):
            asyncio.run(app.emit.set_permission("snake", "nope", "green"))


def test_set_permission_raises_on_capability_denial():
    app = _make_app()
    denial = {"capability": "permissions.manage", "error": "denied by user"}
    _, side_effect = _fake_host(denial)
    with patch("plexi_sdk._emitter._emit", side_effect=side_effect):
        with pytest.raises(CapabilityDeniedError, match="denied by user"):
            asyncio.run(app.emit.set_permission("snake", "panes.read", "red"))


# ── registry ──────────────────────────────────────────────────────────────────

def test_capability_registry_maps_permission_wrappers():
    assert CAPABILITY_REGISTRY["list_permissions"] == "permissions.manage"
    assert CAPABILITY_REGISTRY["set_permission"] == "permissions.manage"
