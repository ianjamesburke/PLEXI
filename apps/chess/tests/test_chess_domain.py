"""Pure tests for the chess domain operations (P1 gate invariants).

Every case asserts both the outcome and that a rejection left the game
untouched — a failed operation must never mutate.
"""

from __future__ import annotations

import copy
import os
import sys

import pytest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import chess_domain as d  # noqa: E402

WHITE = "agent:chess-white"
BLACK = "agent:chess-opponent"


def _game(**kw) -> dict:
    g = d.new_game("game-1", {"white": WHITE, "black": BLACK}, **kw)
    g, _ = d.drain_outbox(g)
    return g


def _auth(actor, game_id="game-1", side=None):
    if actor == d.LOCAL_USER:
        return {
            "schema_version": 1, "actor": actor, "package": "chess",
            "game_id": game_id, "grant_id": "human", "human_interaction": True,
        }
    body = {
        "schema_version": 1, "actor": actor, "package": "chess",
        "game_id": game_id, "grant_id": "grant-test", "human_interaction": False,
    }
    if side:
        body["side"] = side
    return body


def _play(game, actor, rev, op, move, game_id="game-1", authorization="auto", side=None):
    if authorization == "auto":
        authorization = _auth(actor, game_id, side)
    return d.play(game, actor=actor, game_id=game_id, expected_revision=rev,
                  operation_id=op, move=move, authorization=authorization)


def _rejects(code, game, *args, **kw):
    before = copy.deepcopy(game)
    with pytest.raises(d.ChessError) as err:
        _play(game, *args, **kw)
    assert err.value.code == code, err.value
    assert game == before, "a rejected operation mutated the game"
    return err.value


def test_new_game_is_revision_zero_with_started_event():
    g = d.new_game("game-1", {"white": WHITE, "black": BLACK})
    assert g["revision"] == 0
    assert [e["event"] for e in g["outbox"]] == [d.GAME_STARTED]
    snap = d.snapshot(g)
    assert snap["side_to_move"] == "white" and snap["status"] == "ongoing"


def test_legal_move_commits_receipt_revision_and_event():
    g = _game()
    out = _play(g, WHITE, 0, "op-1", "e2e4")
    assert out.game["revision"] == 1
    assert out.receipt["revision_before"] == 0 and out.receipt["revision_after"] == 1
    assert out.receipt["actor"] == WHITE and out.receipt["side"] == "white"
    assert out.receipt["san"] == "e4"
    assert len(out.events) == 1
    ev = out.events[0]
    assert ev["event"] == d.MOVE_COMMITTED
    assert ev["operation_id"] == "op-1" and ev["side_to_move"] == "black"
    # Receipt, board, and outbox commit together in one state value.
    assert out.game["receipts"]["op-1"]["move"] == "e2e4"
    assert out.game["outbox"] == [ev]
    pos = d.position(out.game)
    assert pos.board.get((4, 3)) == "P" and (4, 1) not in pos.board
    assert g["revision"] == 0, "play() must not mutate its input"


def test_illegal_move_rejected():
    _rejects("illegal_move", _game(), WHITE, 0, "op-1", "e2e5")


def test_san_is_rejected_in_favor_of_uci():
    err = _rejects("illegal_move", _game(), WHITE, 0, "op-1", "e4")
    assert "e2e4" in err.message


def test_wrong_side_rejected():
    _rejects("wrong_side", _game(), BLACK, 0, "op-1", "e7e5", side="black")


def test_host_approved_actor_needs_no_seat():
    out = _play(_game(), "agent:intruder", 0, "op-1", "e2e4")
    assert out.receipt["actor"] == "agent:intruder"


def test_missing_envelope_is_denied():
    _rejects("permission_denied", _game(), "agent:intruder", 0, "op-1", "e2e4", authorization=None)


def test_stale_revision_wins_over_wrong_side():
    g = _play(_game(), WHITE, 0, "op-1", "e2e4").game
    _rejects("stale_revision", g, BLACK, 0, "op-2", "e7e5", side="white")


def test_empty_identity_rejected():
    _rejects("permission_denied", _game(), "", 0, "op-1", "e2e4")


def test_local_user_may_move_either_side():
    g = _play(_game(), d.LOCAL_USER, 0, "ui-1", "e2e4").game
    g = _play(g, d.LOCAL_USER, 1, "ui-2", "e7e5").game
    assert g["revision"] == 2


def test_stale_revision_rejected_with_current_revision():
    g = _play(_game(), WHITE, 0, "op-1", "e2e4").game
    err = _rejects("stale_revision", g, BLACK, 0, "op-2", "e7e5")
    assert err.detail["current_revision"] == 1


def test_unknown_game_rejected_without_disclosure():
    err = _rejects("unknown_game", _game(), WHITE, 0, "op-1", "e2e4", game_id="other")
    assert "game-1" not in err.message


def test_concurrent_commands_one_wins():
    g = _game()
    first = _play(g, WHITE, 0, "op-a", "e2e4")
    # The human clicked a different move against the same revision.
    _rejects("stale_revision", first.game, d.LOCAL_USER, 0, "op-b", "d2d4")


def test_duplicate_operation_returns_original_receipt_without_mutation():
    g = _play(_game(), WHITE, 0, "op-1", "e2e4").game
    g, _ = d.drain_outbox(g)
    before = copy.deepcopy(g)
    again = _play(g, WHITE, 0, "op-1", "e2e4")
    assert again.duplicate and again.receipt["duplicate"] is True
    assert again.events == []
    assert again.game == before
    assert again.receipt["revision_after"] == 1


def test_reused_operation_id_with_different_move_conflicts():
    g = _play(_game(), WHITE, 0, "op-1", "e2e4").game
    _rejects("operation_conflict", g, WHITE, 0, "op-1", "d2d4")


def test_reused_operation_id_by_other_actor_conflicts():
    g = _play(_game(), WHITE, 0, "op-1", "e2e4").game
    _rejects("operation_conflict", g, BLACK, 1, "op-1", "e7e5")


def test_promotion_requires_piece_and_commits_queen():
    g = _game(start_fen="8/P7/8/8/8/8/k7/4K3 w - - 0 1")
    _rejects("illegal_move", g, WHITE, 0, "op-1", "a7a8")
    out = _play(g, WHITE, 0, "op-1", "a7a8q")
    assert d.position(out.game).board[(0, 7)] == "Q"
    assert out.receipt["san"].startswith("a8=Q")


def test_castling_kingside():
    g = _game(start_fen="r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1")
    out = _play(g, WHITE, 0, "op-1", "e1g1")
    pos = d.position(out.game)
    assert pos.board[(6, 0)] == "K" and pos.board[(5, 0)] == "R"
    assert out.receipt["san"] == "O-O"


def test_en_passant():
    g = _game(start_fen="4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 1")
    out = _play(g, WHITE, 0, "op-1", "e5d6")
    pos = d.position(out.game)
    assert pos.board[(3, 5)] == "P" and (3, 4) not in pos.board


def test_checkmate_ends_game_and_blocks_further_moves():
    g = _game()
    moves = [(WHITE, "f2f3"), (BLACK, "e7e5"), (WHITE, "g2g4"), (BLACK, "d8h4")]
    for rev, (actor, mv) in enumerate(moves):
        out = _play(g, actor, rev, f"op-{rev}", mv)
        g = out.game
    assert out.receipt["status"] == "checkmate" and out.receipt["result"] == "0-1"
    _rejects("game_over", g, WHITE, 4, "op-x", "e2e4")


def test_stalemate_detected():
    g = _game(start_fen="7k/8/6Q1/8/8/8/8/K7 w - - 0 1")
    out = _play(g, WHITE, 0, "op-1", "g6f7")
    assert out.receipt["status"] == "stalemate"


def test_reset_is_privileged_and_invalidates_old_game():
    g = _play(_game(), WHITE, 0, "op-1", "e2e4").game
    with pytest.raises(d.ChessError) as err:
        d.start_new_game(g, actor=WHITE, game_id="game-2")
    assert err.value.code == "permission_denied"
    fresh = d.start_new_game(g, actor=d.LOCAL_USER, game_id="game-2",
                             seats={"white": WHITE, "black": BLACK})
    assert fresh["revision"] == 0 and fresh["receipts"] == {}
    _rejects("unknown_game", fresh, WHITE, 1, "op-2", "d2d4", game_id="game-1")


def test_outbox_survives_until_drained():
    g = _play(_game(), WHITE, 0, "op-1", "e2e4").game
    # A crash here leaves the move committed and its event still pending.
    restored = copy.deepcopy(g)
    assert d.position(restored).board.get((4, 3)) == "P"
    drained, pending = d.drain_outbox(restored)
    assert [e["operation_id"] for e in pending] == ["op-1"]
    assert drained["outbox"] == [] and drained["revision"] == 1


def test_legal_moves_requires_current_revision():
    g = _game()
    assert "e2e4" in d.legal_moves(g, "game-1", 0)["moves"]
    with pytest.raises(d.ChessError) as err:
        d.legal_moves(g, "game-1", 3)
    assert err.value.code == "stale_revision"
