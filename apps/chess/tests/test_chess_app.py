"""Chess app tests: tools and board share one domain path; events follow commits.

A tiny host stand-in applies `PersistState`/`SetState` effects to a dict and
re-installs it as the state snapshot, so successive calls see what the real
host would hand back.
"""

from __future__ import annotations

import json

import plexi_sdk as sdk
from plexi_sdk import _v3_state
from plexi_sdk.effects import (
    DeclareEventStreams,
    EmitEvent,
    ExposeTools,
    PersistState,
    SetState,
    ToolResult,
)
from plexi_sdk.events import KeyEvent, ToolCall

import chess
import chess_domain as domain

WHITE = "agent:chess-white"
BLACK = "agent:chess-opponent"


class Host:
    def __init__(self, values: dict | None = None) -> None:
        self.values: dict = dict(values or {})
        self.emitted: list[EmitEvent] = []
        self._install()

    def _install(self) -> None:
        _v3_state._state = sdk.StateSnapshot(self.values, {k: b"" for k in self.values})
        _v3_state._in_view = False

    def apply(self, effects: list, upto: int | None = None) -> list:
        """Apply effects in order; `upto` simulates a crash after N effects."""
        for effect in effects[:upto]:
            if isinstance(effect, (PersistState, SetState)):
                self.values.update(json.loads(json.dumps(effect.data)))
            elif isinstance(effect, EmitEvent):
                self.emitted.append(effect)
        self._install()
        return effects

    def init(self, args=()) -> list:
        return self.apply(chess.init((640.0, 640.0), list(args)))

    def call(self, name: str, caller: str = WHITE, **arguments) -> dict:
        effects = self.apply(chess.update(ToolCall("c1", name, json.dumps(arguments), caller)))
        result = next(e for e in effects if isinstance(e, ToolResult))
        if result.error:
            return {"error": result.error}
        return json.loads(result.output_json or "{}")

    def game(self) -> dict:
        return self.values["game"]


def _seated() -> Host:
    host = Host()
    host.init([f"--white={WHITE}", f"--black={BLACK}"])
    host.emitted.clear()
    return host


def test_init_declares_streams_exposes_tools_and_publishes_game_started():
    host = Host()
    effects = host.init([f"--white={WHITE}", f"--black={BLACK}"])
    decl = next(e for e in effects if isinstance(e, DeclareEventStreams))
    assert {s.name for s in decl.streams} == {domain.MOVE_COMMITTED, domain.GAME_STARTED}
    names = {t.name for t in next(e for e in effects if isinstance(e, ExposeTools)).tools}
    assert names == {"chess.state", "chess.legal_moves", "chess.play", "chess.new_game"}
    assert [e.event for e in host.emitted] == [domain.GAME_STARTED]
    assert host.game()["seats"] == {"white": WHITE, "black": BLACK}
    assert host.game()["outbox"] == []


def test_tool_play_commits_receipt_and_emits_scoped_event():
    host = _seated()
    receipt = host.call("chess.play", game_id="game-1", expected_revision=0,
                        operation_id="op-1", move="e2e4")
    assert receipt["revision_after"] == 1 and receipt["actor"] == WHITE
    (event,) = host.emitted
    assert event.event == domain.MOVE_COMMITTED
    assert event.resource_id == "game-1" and event.revision_after == "1"
    assert event.actor_id == WHITE and event.caused_by == "op-1"
    payload = json.loads(event.payload_json)
    assert payload["move"] == "e2e4" and payload["side_to_move"] == "black"
    assert host.game()["outbox"] == []
    assert host.call("chess.state")["revision"] == 1


def test_duplicate_stale_and_unauthorized_calls_do_not_mutate_or_emit():
    host = _seated()
    host.call("chess.play", game_id="game-1", expected_revision=0,
              operation_id="op-1", move="e2e4")
    host.emitted.clear()
    before = json.loads(json.dumps(host.game()))

    dup = host.call("chess.play", game_id="game-1", expected_revision=0,
                    operation_id="op-1", move="e2e4")
    assert dup["duplicate"] is True and dup["revision_after"] == 1
    stale = host.call("chess.play", caller=BLACK, game_id="game-1",
                      expected_revision=0, operation_id="op-2", move="e7e5")
    assert stale["error"].startswith("ChessError: stale_revision")
    intruder = host.call("chess.play", caller="agent:intruder", game_id="game-1",
                         expected_revision=1, operation_id="op-3", move="e7e5")
    assert intruder["error"].startswith("ChessError: unauthorized")
    wrong = host.call("chess.play", caller=WHITE, game_id="game-1",
                      expected_revision=1, operation_id="op-4", move="d2d4")
    assert wrong["error"].startswith("ChessError: wrong_side")
    assert host.game() == before
    assert host.emitted == []


def test_forged_caller_argument_is_ignored():
    host = _seated()
    out = host.call("chess.play", caller="agent:intruder", caller_id=WHITE,
                    game_id="game-1", expected_revision=0, operation_id="op-1",
                    move="e2e4")
    assert out["error"].startswith("ChessError: unauthorized")


def test_new_game_tool_is_privileged():
    host = _seated()
    out = host.call("chess.new_game", caller=WHITE, game_id="game-2", white="", black="")
    assert out["error"].startswith("ChessError: unauthorized")
    assert host.game()["game_id"] == "game-1"


def test_board_move_uses_the_same_domain_operation():
    host = _seated()
    host.apply(chess.update(KeyEvent("enter")))  # select e2 (cursor default)
    host.apply(chess.update(KeyEvent("up")))
    host.apply(chess.update(KeyEvent("up")))
    host.apply(chess.update(KeyEvent("enter")))  # drop on e4
    game = host.game()
    assert game["revision"] == 1
    assert game["moves"][0]["actor"] == domain.LOCAL_USER
    assert game["moves"][0]["uci"] == "e2e4"
    (event,) = host.emitted
    assert event.event == domain.MOVE_COMMITTED and event.actor == "user"
    # The agent's next move must now name revision 1.
    stale = host.call("chess.play", caller=BLACK, game_id="game-1",
                      expected_revision=0, operation_id="op-b", move="e7e5")
    assert "stale_revision" in stale["error"]


def test_crash_between_commit_and_publication_republishes_once_on_restart():
    host = _seated()
    effects = chess.update(ToolCall("c1", "chess.play", json.dumps({
        "game_id": "game-1", "expected_revision": 0, "operation_id": "op-1",
        "move": "e2e4"}), WHITE))
    # Apply only the ToolResult and the first PersistState (move + outbox).
    first_persist = next(i for i, e in enumerate(effects) if isinstance(e, PersistState))
    host.apply(effects, upto=first_persist + 1)
    assert host.emitted == []
    assert host.game()["revision"] == 1 and len(host.game()["outbox"]) == 1

    host.init()  # relaunch
    assert [json.loads(e.payload_json)["operation_id"] for e in host.emitted] == ["op-1"]
    assert host.game()["revision"] == 1, "restart must not re-apply the move"
    assert host.game()["outbox"] == []

    host.emitted.clear()
    host.init()  # a second restart has nothing pending
    assert host.emitted == []


def test_view_renders_revision_and_seats():
    host = _seated()
    host.call("chess.play", game_id="game-1", expected_revision=0,
              operation_id="op-1", move="e2e4")
    _v3_state._in_view = True
    try:
        tree = chess.view()
    finally:
        _v3_state._in_view = False
    text = json.dumps(tree, default=lambda o: getattr(o, "__dict__", str(o)))
    assert "rev 1" in text and WHITE in text
