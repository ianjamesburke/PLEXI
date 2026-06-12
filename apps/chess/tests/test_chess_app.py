"""Headless app-level tests for the chess app: event streams, reversible
move events, tool handlers, and rollback verify/apply
(docs/prm/chess-agent-poc.md "App Contract")."""

from __future__ import annotations

import os
import sys
import time
from pathlib import Path

import pytest

sys.path.insert(
    0,
    os.path.join(os.path.dirname(__file__), "..", "..", "..", "sdk", "python"),
)
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from plexi_sdk.testing import AppHarness  # noqa: E402

APP = Path(__file__).resolve().parent.parent / "chess.py"


def _events(h: AppHarness, type_: str) -> list[dict]:
    return [e for e in h._events_seen if e.get("type") == type_]


def _emitted(h: AppHarness, stream: str) -> list[dict]:
    return [e for e in _events(h, "emit_event") if e.get("event") == stream]


def _wait_for_tool_result(h: AppHarness, call_id: str, timeout: float = 5.0) -> dict:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        h.run(1)
        for ev in _events(h, "tool_result"):
            if ev.get("call_id") == call_id:
                return ev
    raise AssertionError(f"no tool_result for {call_id!r}")


@pytest.fixture
def chess():
    with AppHarness(APP, timeout=5.0) as h:
        h.run(1)
        yield h


def test_declares_streams_and_starts_game(chess):
    declares = _events(chess, "declare_event_streams")
    assert declares, "app must declare event streams on init"
    names = {s["name"] for s in declares[0]["streams"]}
    assert names == {"game.started", "turn.ready", "move.played",
                     "move.undone", "game.ended"}
    assert _emitted(chess, "game.started"), "game.started must fire on init"
    turn = _emitted(chess, "turn.ready")
    assert turn and turn[0]["payload"]["side_to_move"] == "white"
    assert "e4" in turn[0]["payload"]["legal_moves"]
    tools = _events(chess, "expose_tools")
    assert tools, "app must expose tools"
    # The @tool decorator re-emits the cumulative list per registration —
    # the last expose_tools event carries the full set.
    tool_names = {t["name"] for t in tools[-1]["tools"]}
    assert tool_names == {"chess.current_state", "chess.legal_moves",
                          "chess.make_move", "chess.undo_move", "chess.resign"}


def test_user_move_emits_reversible_event_and_turn_ready(chess):
    chess.text_submit("chess-move", "e4")
    chess.run(2)
    played = _emitted(chess, "move.played")
    assert played, "submitting a move must emit move.played"
    ev = played[0]
    assert ev["actor"] == "user"
    assert ev["rollback_token"] == "move-1"
    assert ev["revision_before"] == "rev-0"
    assert ev["revision_after"] == "rev-1"
    assert ev["payload"]["san"] == "e4"
    turns = _emitted(chess, "turn.ready")
    assert turns[-1]["payload"]["side_to_move"] == "black"


def test_illegal_user_move_emits_nothing(chess):
    chess.text_submit("chess-move", "Ke2")
    chess.run(2)
    assert not _emitted(chess, "move.played")


def test_undo_command_emits_move_undone(chess):
    chess.text_submit("chess-move", "e4")
    chess.run(2)
    chess.text_submit("chess-move", "undo")
    chess.run(2)
    undone = _emitted(chess, "move.undone")
    assert undone and undone[0]["payload"]["san"] == "e4"
    assert undone[0]["revision_after"] == "rev-0"


def test_make_move_tool_validates_and_returns_spec_shape(chess):
    import json
    game_id = _emitted(chess, "game.started")[0]["payload"]["game_id"]

    chess._send({"type": "tool_call", "call_id": "c1", "name": "chess.make_move",
                 "input_json": json.dumps({"game_id": game_id, "move": "e4",
                                           "notation": "san"})})
    result = json.loads(_wait_for_tool_result(chess, "c1")["output_json"])
    assert result["ok"] is True
    assert result["summary"] == "White played e4"
    assert result["rollback_token"] == "move-1"
    assert result["revision_before"] == "rev-0"
    assert result["revision_after"] == "rev-1"
    assert result["changed_resources"] == [game_id]
    agent_moves = [e for e in _emitted(chess, "move.played") if e["actor"] == "agent"]
    assert agent_moves, "tool move must emit move.played with actor=agent"

    # Illegal move → ok: False with the reason; nothing emitted.
    chess._send({"type": "tool_call", "call_id": "c2", "name": "chess.make_move",
                 "input_json": json.dumps({"game_id": game_id, "move": "e9"})})
    result = json.loads(_wait_for_tool_result(chess, "c2")["output_json"])
    assert result["ok"] is False and "illegal move" in result["error"]

    # Wrong game id → error.
    chess._send({"type": "tool_call", "call_id": "c3", "name": "chess.make_move",
                 "input_json": json.dumps({"game_id": "nope", "move": "e5"})})
    result = json.loads(_wait_for_tool_result(chess, "c3")["output_json"])
    assert result["ok"] is False and "unknown game_id" in result["error"]


def test_current_state_and_legal_moves_tools(chess):
    import json
    game_id = _emitted(chess, "game.started")[0]["payload"]["game_id"]
    chess._send({"type": "tool_call", "call_id": "s1",
                 "name": "chess.current_state", "input_json": "{}"})
    state = json.loads(_wait_for_tool_result(chess, "s1")["output_json"])
    assert state["game_id"] == game_id
    assert state["side_to_move"] == "white"
    assert state["revision_id"] == "rev-0"
    assert state["result"] is None
    assert "e4" in state["legal_moves"]

    chess._send({"type": "tool_call", "call_id": "s2",
                 "name": "chess.legal_moves", "input_json": "{}"})
    legal = json.loads(_wait_for_tool_result(chess, "s2")["output_json"])
    assert len(legal["legal_moves"]) == 20


def test_resign_tool_ends_game(chess):
    import json
    game_id = _emitted(chess, "game.started")[0]["payload"]["game_id"]
    chess._send({"type": "tool_call", "call_id": "r1", "name": "chess.resign",
                 "input_json": json.dumps({"game_id": game_id})})
    result = json.loads(_wait_for_tool_result(chess, "r1")["output_json"])
    assert result == {"ok": True, "result": "0-1"}, "white to move resigns -> 0-1"
    ended = _emitted(chess, "game.ended")
    assert ended and ended[0]["payload"]["by"] == "resignation"


def test_rollback_verify_and_apply_round_trip(chess):
    game_id = _emitted(chess, "game.started")[0]["payload"]["game_id"]
    chess.text_submit("chess-move", "e4")
    chess.run(2)

    # Host asks: still at rev-1? App answers with its current revision.
    chess._send({"type": "rollback_verify", "checkpoint_id": "ckpt-1",
                 "resource_id": game_id, "expected_revision": "rev-1"})
    deadline = time.monotonic() + 5.0
    answer = None
    while time.monotonic() < deadline and answer is None:
        chess.run(1)
        results = _events(chess, "rollback_verify_result")
        answer = results[0] if results else None
    assert answer is not None
    assert answer["checkpoint_id"] == "ckpt-1"
    assert answer["current_revision"] == "rev-1"

    # Stale game id → empty revision (never matches; host blocks).
    chess._send({"type": "rollback_verify", "checkpoint_id": "ckpt-2",
                 "resource_id": "old-game", "expected_revision": "rev-1"})
    deadline = time.monotonic() + 5.0
    stale = None
    while time.monotonic() < deadline and stale is None:
        chess.run(1)
        stale = next((e for e in _events(chess, "rollback_verify_result")
                      if e["checkpoint_id"] == "ckpt-2"), None)
    assert stale is not None and stale["current_revision"] == ""

    # Verified apply → the move is undone and move.undone emitted.
    chess._send({"type": "rollback_apply", "checkpoint_id": "ckpt-1",
                 "resource_id": game_id, "rollback_token": "move-1"})
    deadline = time.monotonic() + 5.0
    undone = []
    while time.monotonic() < deadline and not undone:
        chess.run(1)
        undone = _emitted(chess, "move.undone")
    assert undone and undone[0]["payload"]["san"] == "e4"
    assert undone[0]["actor"] == "system"
