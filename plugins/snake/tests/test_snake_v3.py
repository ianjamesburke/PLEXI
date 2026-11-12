"""SDK v3 snake app tests."""

from __future__ import annotations

from plexi_sdk import StateSnapshot, _v3_state
from plexi_sdk.effects import SetState
from plexi_sdk.events import KeyEvent, TimerFired

import snake


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
    from plexi_sdk import theme

    _with_state(snake._initial())

    changed = _state_effect(snake.update(KeyEvent("down")))
    _with_state(changed)

    assert changed["next_direction"] == [0, 1]
    view_node = snake.view().to_node()
    assert view_node.get("padding") is None
    canvas = snake.view().children[1].to_node()
    assert canvas["type"] == "canvas"
    assert any(cmd["type"] == "rect" and cmd["fill"] == theme.bg_darkest for cmd in canvas["commands"])
    assert any(cmd["type"] == "rect" and cmd["fill"] == theme.danger for cmd in canvas["commands"])
    assert any(cmd["type"] == "rect" and cmd["fill"] == theme.accent for cmd in canvas["commands"])
    assert any(cmd["type"] == "rect" and cmd["fill"] == theme.success for cmd in canvas["commands"])


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


def _node_types(node: dict) -> list[str]:
    types = [node["type"]]
    for child in node.get("children", []):
        types.extend(_node_types(child))
    return types


def test_view_is_canvas_only_with_in_canvas_key_hint() -> None:
    """Stint 1266: a game shows the game, not its own name band. Status goes
    through SetStatus and the key hint is drawn inside the canvas."""
    _with_state(snake._initial())
    node = snake.view().to_node()
    types = _node_types(node)
    assert "surface_header" not in types
    assert "footer_keys" not in types
    assert node["type"] == "canvas"
    texts = [cmd["text"] for cmd in node["commands"] if cmd["type"] == "text"]
    assert any("move" in text for text in texts), texts

    dead = snake._initial()
    dead["alive"] = False
    _with_state(dead)
    texts = [cmd["text"] for cmd in snake.view().to_node()["commands"] if cmd["type"] == "text"]
    assert any("restart" in text for text in texts), texts
