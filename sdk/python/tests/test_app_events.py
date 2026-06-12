"""Tests for emit.emit_event / emit.declare_event_streams — app event
contract (docs/prm/undo-and-app-events.md, Phase B)."""

import pytest
from unittest.mock import patch

from plexi_sdk import App


def _emitter():
    return App().emit


def test_declare_event_streams_emits_wire_shape():
    emitted = []
    with patch("plexi_sdk._emitter._emit", side_effect=lambda d: emitted.append(d)):
        _emitter().declare_event_streams([
            {"name": "move.played", "schema": {"type": "object"},
             "description": "a chess move"},
        ])
    assert emitted == [{
        "type": "declare_event_streams",
        "streams": [{"name": "move.played", "schema": {"type": "object"},
                     "description": "a chess move"}],
    }]


def test_declare_event_streams_validates_entries():
    e = _emitter()
    with pytest.raises(ValueError, match="non-empty"):
        e.declare_event_streams([])
    with pytest.raises(ValueError, match="name"):
        e.declare_event_streams([{"schema": {}}])
    with pytest.raises(ValueError, match="schema"):
        e.declare_event_streams([{"name": "x"}])


def test_emit_event_required_fields_only():
    emitted = []
    with patch("plexi_sdk._emitter._emit", side_effect=lambda d: emitted.append(d)):
        _emitter().emit_event("move.played", "user", "White played e4",
                              "game-abc", "rev-13")
    assert emitted == [{
        "type": "emit_event",
        "event": "move.played",
        "actor": "user",
        "summary": "White played e4",
        "resource_id": "game-abc",
        "revision_after": "rev-13",
        "changed_resources": [],
    }]


def test_emit_event_full_optional_fields():
    emitted = []
    with patch("plexi_sdk._emitter._emit", side_effect=lambda d: emitted.append(d)):
        _emitter().emit_event(
            "move.played", "agent", "Black played e5", "game-abc", "rev-14",
            payload={"san": "e5"},
            state_ref="chess://game/abc/rev/14",
            revision_before="rev-13",
            rollback_token="undo-xyz",
            changed_resources=["game-abc"],
            suggested_trigger="conversation",
            resource_scope="game",
            actor_id="chess-opponent",
        )
    msg = emitted[0]
    assert msg["payload"] == {"san": "e5"}
    assert msg["state_ref"] == "chess://game/abc/rev/14"
    assert msg["revision_before"] == "rev-13"
    assert msg["rollback_token"] == "undo-xyz"
    assert msg["changed_resources"] == ["game-abc"]
    assert msg["suggested_trigger"] == "conversation"
    assert msg["resource_scope"] == "game"
    assert msg["actor_id"] == "chess-opponent"


def test_emit_event_rejects_bad_actor_and_empty_required():
    e = _emitter()
    with pytest.raises(ValueError, match="actor"):
        e.emit_event("ev", "robot", "s", "r", "rev-1")
    for kwargs in (
        dict(event="", actor="user", summary="s", resource_id="r",
             revision_after="rev-1"),
        dict(event="ev", actor="user", summary="  ", resource_id="r",
             revision_after="rev-1"),
        dict(event="ev", actor="user", summary="s", resource_id="",
             revision_after="rev-1"),
        dict(event="ev", actor="user", summary="s", resource_id="r",
             revision_after=""),
    ):
        with pytest.raises(ValueError, match="non-empty"):
            e.emit_event(kwargs["event"], kwargs["actor"], kwargs["summary"],
                         kwargs["resource_id"], kwargs["revision_after"])


def test_emit_event_rejects_unknown_suggested_trigger():
    with pytest.raises(ValueError, match="suggested_trigger"):
        _emitter().emit_event("ev", "user", "s", "r", "rev-1",
                              suggested_trigger="sometimes")


def test_emit_event_stamps_caused_by_during_tool_call():
    """Events emitted while a tool handler runs carry the caller identity as
    caused_by; outside a tool call the field is absent."""
    from plexi_sdk._emitter import _current_tool_caller

    emitted = []
    with patch("plexi_sdk._emitter._emit", side_effect=lambda d: emitted.append(d)):
        e = _emitter()
        token = _current_tool_caller.set("agent:chess-opponent")
        try:
            e.emit_event("move.played", "agent", "Black played Nf6",
                         "game-1", "rev-2")
        finally:
            _current_tool_caller.reset(token)
        e.emit_event("move.played", "user", "White played e4",
                     "game-1", "rev-3")
    assert emitted[0]["caused_by"] == "agent:chess-opponent"
    assert "caused_by" not in emitted[1]
