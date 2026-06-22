"""SDK v3 chess app tests."""

from __future__ import annotations

import os
import sys

sys.path.insert(
    0, os.path.join(os.path.dirname(__file__), "..", "..", "..", "sdk", "python")
)
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from plexi_sdk import StateSnapshot, _v3_state  # noqa: E402
from plexi_sdk.effects import SetState  # noqa: E402
from plexi_sdk.events import KeyEvent  # noqa: E402

import chess  # noqa: E402


def _with_state(values: dict) -> None:
    _v3_state._state = StateSnapshot(values, {})
    _v3_state._in_view = False


def _state_effect(effects: list) -> dict:
    for effect in effects:
        if isinstance(effect, SetState):
            return effect.data
    raise AssertionError(f"no SetState effect in {effects!r}")


def test_init_sets_default_game_state() -> None:
    _with_state({})

    effects = chess.init((480.0, 480.0), [])
    data = _state_effect(effects)

    assert data["fen"].startswith("rnbqkbnr")
    assert data["cursor"] == [4, 1]
    assert data["selected"] is None


def test_keyboard_selects_and_moves_piece() -> None:
    _with_state(chess._initial())

    selected = _state_effect(chess.update(KeyEvent("enter")))
    assert selected["selected"] == [4, 1]

    _with_state(selected)
    moved = _state_effect(chess.update(KeyEvent("up")))
    _with_state(moved)
    moved = _state_effect(chess.update(KeyEvent("up")))
    _with_state(moved)
    moved = _state_effect(chess.update(KeyEvent("enter")))

    assert " w " not in moved["fen"]
    assert moved["last_move"] == "e4"
    assert moved["selected"] is None


def test_view_returns_canvas_tree() -> None:
    _with_state(chess._initial())

    tree = chess.view()
    node = tree.children[1]

    assert node.to_node()["type"] == "canvas"
    assert node.to_node()["commands"]
