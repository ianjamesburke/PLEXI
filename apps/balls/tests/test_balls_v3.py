"""SDK v3 balls app tests."""

from __future__ import annotations

import os
import sys

sys.path.insert(
    0, os.path.join(os.path.dirname(__file__), "..", "..", "..", "sdk", "python")
)
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from plexi_sdk import StateSnapshot, _v3_state  # noqa: E402
from plexi_sdk.effects import SetState  # noqa: E402
from plexi_sdk.events import TimerFired  # noqa: E402

import balls  # noqa: E402


def _with_state(values: dict) -> None:
    _v3_state._state = StateSnapshot(values, {})
    _v3_state._in_view = False


def _state_effect(effects: list) -> dict:
    for effect in effects:
        if isinstance(effect, SetState):
            return effect.data
    raise AssertionError(f"no SetState effect in {effects!r}")


def test_timer_steps_physics_and_view_has_canvas() -> None:
    data = balls._initial(2)
    _with_state(data)

    stepped = _state_effect(balls.update(TimerFired(balls.TIMER_ID)))
    _with_state(stepped)

    assert stepped["ticks"] == 1
    assert stepped["balls"][0]["y"] != data["balls"][0]["y"]
    canvas = balls.view().children[1].to_node()
    assert canvas["type"] == "canvas"
    assert {"type": "rect", "x": 0, "y": 0, "w": balls.CANVAS_W, "h": balls.CANVAS_H,
            "fill": "#0d0d1a", "radius": 0.0} in canvas["commands"]
    assert any(cmd["type"] == "circle" and "cx" in cmd and "r" in cmd for cmd in canvas["commands"])
