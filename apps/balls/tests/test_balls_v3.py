"""SDK v3 balls app tests."""

from __future__ import annotations

import os
import sys

sys.path.insert(
    0, os.path.join(os.path.dirname(__file__), "..", "..", "..", "sdk", "python")
)
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from plexi_sdk.events import TimerFired  # noqa: E402

import balls  # noqa: E402


def test_timer_steps_physics_and_view_has_canvas() -> None:
    data = balls._initial(2)
    start_y = data["balls"][0]["y"]
    balls._runtime = data

    effects = balls.update(TimerFired(balls.TIMER_ID))
    stepped = balls._runtime

    assert effects == []
    assert stepped["ticks"] == 1
    assert stepped["balls"][0]["y"] != start_y
    canvas = balls.view().children[1].to_node()
    assert canvas["type"] == "canvas"
    assert {"type": "rect", "x": 0, "y": 0, "w": balls.CANVAS_W, "h": balls.CANVAS_H,
            "fill": "#0d0d1a", "radius": 0.0} in canvas["commands"]
    assert any(cmd["type"] == "circle" and "cx" in cmd and "r" in cmd for cmd in canvas["commands"])
