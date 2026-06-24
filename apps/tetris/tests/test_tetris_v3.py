"""SDK v3 tetris app tests."""

from __future__ import annotations

import os
import sys

sys.path.insert(
    0, os.path.join(os.path.dirname(__file__), "..", "..", "..", "sdk", "python")
)
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from plexi_sdk import StateSnapshot, _v3_state  # noqa: E402
from plexi_sdk.effects import SetState  # noqa: E402
from plexi_sdk.events import KeyEvent, TimerFired  # noqa: E402

import tetris  # noqa: E402


def _with_state(values: dict) -> None:
    _v3_state._state = StateSnapshot(values, {})
    _v3_state._in_view = False


def _state_effect(effects: list) -> dict:
    for effect in effects:
        if isinstance(effect, SetState):
            return effect.data
    raise AssertionError(f"no SetState effect in {effects!r}")


def test_key_moves_piece_and_view_has_canvas() -> None:
    data = tetris._initial()
    _with_state(data)

    moved = _state_effect(tetris.update(KeyEvent("left")))
    _with_state(moved)

    assert moved["current"]["col"] == data["current"]["col"] - 1
    canvas = tetris.view().children[1].to_node()
    assert canvas["type"] == "canvas"
    assert any(cmd["type"] == "rect" and "w" in cmd and "h" in cmd for cmd in canvas["commands"])


def test_timer_drops_piece_after_threshold() -> None:
    data = tetris._initial()
    data["fall_ticks"] = tetris._drop_ticks(data) - 1
    _with_state(data)

    dropped = _state_effect(tetris.update(TimerFired(tetris.TIMER_ID)))

    assert dropped["current"]["row"] == 1
    assert dropped["fall_ticks"] == 0
