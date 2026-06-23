"""SDK v3 balls app tests."""

from __future__ import annotations

import os
import sys

sys.path.insert(
    0, os.path.join(os.path.dirname(__file__), "..", "..", "..", "sdk", "python")
)
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from plexi_sdk.events import RenderFrame  # noqa: E402

import balls  # noqa: E402


def test_render_frame_steps_physics_and_view_has_canvas() -> None:
    data = balls._initial(2)
    start_y = data["balls"][0]["y"]
    balls._runtime = data

    effects = balls.update(RenderFrame(frame_id=1, elapsed=balls.TARGET_DT))
    stepped = balls._runtime

    assert effects == []
    assert stepped["ticks"] == 1
    assert stepped["balls"][0]["y"] != start_y
    canvas = balls.view().children[1].to_node()
    assert canvas["type"] == "canvas"
    assert {"type": "rect", "x": 0, "y": 0, "w": balls.CANVAS_W, "h": balls.CANVAS_H,
            "fill": "#0d0d1a", "radius": 0.0} in canvas["commands"]
    assert any(cmd["type"] == "circle" and "cx" in cmd and "r" in cmd for cmd in canvas["commands"])


def test_init_uses_continuous_scheduler_not_timers() -> None:
    effects = balls.init((640.0, 360.0), [])

    assert any(
        type(effect).__name__ == "SetSchedulerMode"
        and effect.mode == "continuous"
        and effect.fps == balls.TARGET_FPS
        for effect in effects
    )
    assert not any(type(effect).__name__ == "SetTimer" for effect in effects)
