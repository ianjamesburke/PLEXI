"""Chess engine legality tests — the app is the move-legality authority
(docs/prm/chess-agent-poc.md), so the engine must be provably correct.

Run: `uv run pytest apps/chess/tests/` from sdk/python, or pytest directly.
"""

from __future__ import annotations

import os
import sys

import pytest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from chess_engine import Position, parse_sq, sq_name  # noqa: E402


def perft(pos: Position, depth: int) -> int:
    if depth == 0:
        return 1
    total = 0
    for move in pos.legal_moves():
        pos._apply(move)
        total += perft(pos, depth - 1)
        pos._undo_apply()
    return total


def test_perft_from_initial_position():
    # Canonical perft values — any generator bug (castling, en passant,
    # pins, promotions) diverges from these.
    pos = Position.initial()
    assert perft(pos, 1) == 20
    assert perft(pos, 2) == 400
    assert perft(pos, 3) == 8902


def test_initial_fen():
    assert Position.initial().fen().startswith(
        "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq -"
    )


def test_illegal_move_raises_with_legal_list():
    pos = Position.initial()
    with pytest.raises(ValueError) as exc:
        pos.make_move("Ke2")
    assert "illegal move" in str(exc.value)
    assert "e4" in str(exc.value), "error must list legal moves"


def test_san_and_uci_inputs_both_accepted():
    pos = Position.initial()
    assert pos.make_move("e4").uci == "e2e4"
    assert pos.make_move("e7e5").san == "e5"


def test_scholars_mate_ends_game_and_undo_reopens():
    pos = Position.initial()
    for mv in ["e4", "e5", "Qh5", "Nc6", "Bc4", "Nf6", "Qxf7#"]:
        pos.make_move(mv)
    assert pos.result() == "1-0"
    assert pos.legal_moves() == []
    undone = pos.undo_move()
    assert undone.san == "Qxf7#"
    assert pos.result() is None
    assert any(m.san == "Qxf7#" for m in pos.legal_moves())


def test_stalemate_is_draw():
    # Classic minimal stalemate: black king a8, white queen + king.
    pos = Position()
    pos.board = {
        parse_sq("a8"): "k",
        parse_sq("b6"): "Q",
        parse_sq("c6"): "K",
    }
    pos.white_to_move = False
    pos.castling = set()
    assert pos.result() == "1/2-1/2"


def test_castling_through_check_is_illegal():
    pos = Position()
    pos.board = {
        parse_sq("e1"): "K",
        parse_sq("h1"): "R",
        parse_sq("e8"): "k",
        parse_sq("f8"): "r",  # black rook covers f1 — O-O must be illegal
    }
    pos.castling = {"K"}
    assert not any(m.san.startswith("O-O") for m in pos.legal_moves())


def test_en_passant_capture_and_undo():
    pos = Position.initial()
    for mv in ["e4", "a6", "e5", "d5"]:
        pos.make_move(mv)
    sans = [m.san for m in pos.legal_moves()]
    assert "exd6+" in sans or "exd6" in sans
    fen_before = pos.fen()
    pos.make_move("exd6")
    assert pos.board.get(parse_sq("d5")) is None, "captured pawn must be removed"
    pos.undo_move()
    assert pos.fen() == fen_before


def test_promotion_generates_all_pieces():
    pos = Position()
    pos.board = {
        parse_sq("a7"): "P",
        parse_sq("e1"): "K",
        parse_sq("e8"): "k",
    }
    pos.castling = set()
    sans = {m.san for m in pos.legal_moves() if m.src == parse_sq("a7")}
    assert {"a8=Q+", "a8=R+", "a8=B", "a8=N"} <= sans


def test_disambiguation_in_san():
    pos = Position()
    pos.board = {
        parse_sq("b1"): "N",
        parse_sq("f1"): "N",
        parse_sq("a1"): "K",
        parse_sq("h8"): "k",
    }
    pos.castling = set()
    sans = {m.san for m in pos.legal_moves()}
    assert "Nbd2" in sans and "Nfd2" in sans


def test_pinned_piece_cannot_move():
    pos = Position()
    pos.board = {
        parse_sq("e1"): "K",
        parse_sq("e4"): "N",  # pinned by the rook on e8
        parse_sq("e8"): "r",
        parse_sq("a8"): "k",
    }
    pos.castling = set()
    assert not any(m.src == parse_sq("e4") for m in pos.legal_moves())


def test_sq_name_round_trip():
    for name in ("a1", "h8", "e4", "c7"):
        assert sq_name(parse_sq(name)) == name
