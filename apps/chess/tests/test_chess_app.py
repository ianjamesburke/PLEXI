"""SDK v3 chess app tests."""

from __future__ import annotations

import os
import sys

sys.path.insert(
    0, os.path.join(os.path.dirname(__file__), "..", "..", "..", "sdk", "python")
)
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from plexi_sdk import StateSnapshot, _v3_state  # noqa: E402
import plexi_sdk as sdk  # noqa: E402
from plexi_sdk.effects import SetMouseTracking, SetState  # noqa: E402
from plexi_sdk.events import KeyEvent, MouseEvent  # noqa: E402

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
    assert any(isinstance(effect, SetMouseTracking) and effect.enabled for effect in effects)


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
    sdk.canvas_width = 640.0
    sdk.canvas_height = 360.0

    tree = chess.view()
    node = tree.children[1]

    canvas = node.to_node()
    assert canvas["type"] == "canvas"
    assert canvas["commands"]
    assert any(
        cmd["type"] == "text" and cmd.get("align") == "center_center"
        for cmd in canvas["commands"]
    )
    assert canvas["width"] == 640.0
    assert canvas["height"] == 360.0


def test_drag_moves_piece_using_pane_sized_board() -> None:
    _with_state(chess._initial())
    sdk.canvas_width = 640.0
    sdk.canvas_height = 360.0
    ox, oy, cell, _board = chess._board_geometry()

    def center(file_idx: int, rank: int) -> tuple[float, float]:
        return ox + (file_idx + 0.5) * cell, oy + (7 - rank + 0.5) * cell

    x1, y1 = center(4, 1)
    selected = chess.update(MouseEvent(x=x1, y=y1, button="left", pressed=True))
    data = _state_effect(selected)
    assert data["selected"] == [4, 1]

    _with_state(data)
    x2, y2 = center(4, 3)
    moved = chess.update(MouseEvent(x=x2, y=y2, button="left", pressed=False))
    data = _state_effect(moved)
    assert data["last_move"] == "e4"
    assert data["selected"] is None
