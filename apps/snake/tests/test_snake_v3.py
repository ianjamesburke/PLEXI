"""SDK v3 snake app tests."""

from __future__ import annotations

import os
import sys

sys.path.insert(
    0, os.path.join(os.path.dirname(__file__), "..", "..", "..", "sdk", "python")
)
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import plexi_sdk as sdk
from plexi_sdk import StateSnapshot, _v3_state  # noqa: E402
from plexi_sdk.effects import SetState  # noqa: E402
from plexi_sdk.events import KeyEvent, Resize, TimerFired  # noqa: E402

import snake  # noqa: E402


def _with_state(values: dict) -> None:
    _v3_state._state = StateSnapshot(values, {})
    _v3_state._in_view = False


def _state_effect(effects: list) -> dict:
    for effect in effects:
        if isinstance(effect, SetState):
            return effect.data
    raise AssertionError(f"no SetState effect in {effects!r}")


def test_timer_advances_snake() -> None:
    data = snake._initial()
    _with_state(data)

    advanced = _state_effect(snake.update(TimerFired(snake.TIMER_ID)))

    assert advanced["snake"][0] == [snake.COLS // 2 + 1, snake.ROWS // 2]
    assert advanced["score"] == 0
    assert advanced["alive"] is True


def test_key_changes_direction_and_view_has_canvas() -> None:
    _with_state(snake._initial())

    changed = _state_effect(snake.update(KeyEvent("down")))
    _with_state(changed)

    assert changed["next_direction"] == [0, 1]
    canvas = snake.view().children[0].to_node()
    assert canvas["type"] == "canvas"
    assert any(cmd["type"] == "rect" and "w" in cmd and "h" in cmd for cmd in canvas["commands"])


def test_resize_updates_pane_and_scales_grid() -> None:
    _with_state(snake._initial())
    sdk.pane_width = 640.0
    sdk.pane_height = 360.0
    sdk.canvas_width = 640.0
    sdk.canvas_height = 360.0

    snake.update(Resize(width=1200.0, height=800.0))
    sdk.pane_width = 1200.0
    sdk.pane_height = 800.0
    sdk.canvas_width = 1200.0
    sdk.canvas_height = 800.0

    cell = snake._cell_size()
    assert cell > 18.0
