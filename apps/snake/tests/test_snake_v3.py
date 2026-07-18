"""SDK v3 snake app tests."""

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
    canvas = snake.view().children[1].to_node()
    assert canvas["type"] == "canvas"
    assert any(cmd["type"] == "rect" and "w" in cmd and "h" in cmd for cmd in canvas["commands"])


def test_next_food_is_random_and_lands_on_free_cell() -> None:
    body = [[13, 10], [12, 10], [11, 10]]
    occupied = {tuple(cell) for cell in body}
    results = set()
    for _ in range(50):
        food = snake._next_food(body)
        assert 0 <= food[0] < snake.COLS
        assert 0 <= food[1] < snake.ROWS
        assert tuple(food) not in occupied
        results.add(tuple(food))
    # The old deterministic spawn returned one fixed cell for a fixed board;
    # uniform random over ~500 free cells must produce more than one result.
    assert len(results) > 1


def test_next_food_full_board_returns_sentinel() -> None:
    full = [[c, r] for r in range(snake.ROWS) for c in range(snake.COLS)]
    assert snake._next_food(full) == [-1, -1]
