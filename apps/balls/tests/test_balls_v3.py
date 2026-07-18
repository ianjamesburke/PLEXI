"""SDK v3 balls app tests."""

from __future__ import annotations

import os
import sys

sys.path.insert(
    0, os.path.join(os.path.dirname(__file__), "..", "..", "..", "sdk", "python")
)
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from plexi_sdk.events import MouseEvent, RenderFrame, Resize  # noqa: E402

import balls  # noqa: E402


def _one_ball(x: float, y: float, r: float) -> dict:
    return {"x": x, "y": y, "vx": 0.0, "vy": 0.0, "r": r, "color": "#ffffff"}


def test_render_frame_steps_physics_and_view_is_bare_canvas() -> None:
    data = balls._initial(640.0, 360.0, 2)
    start_y = data["balls"][0]["y"]
    balls._runtime = data

    effects = balls.update(RenderFrame(frame_id=1, elapsed=balls.TARGET_DT))
    stepped = balls._runtime

    assert effects == []
    assert stepped["balls"][0]["y"] != start_y
    canvas = balls.view().to_node()
    assert canvas["type"] == "canvas"
    assert canvas["fit"] == "fill"
    assert canvas["width"] == 640.0 and canvas["height"] == 360.0
    assert any(cmd["type"] == "circle" for cmd in canvas["commands"])


def test_init_uses_continuous_scheduler_not_timers() -> None:
    effects = balls.init((640.0, 360.0), [])

    assert any(
        type(effect).__name__ == "SetSchedulerMode"
        and effect.mode == "continuous"
        and effect.fps == balls.TARGET_FPS
        for effect in effects
    )
    assert not any(type(effect).__name__ == "SetTimer" for effect in effects)


def test_click_empty_adds_ball_at_cursor_click_ball_removes() -> None:
    balls._runtime = {"balls": [_one_ball(100.0, 100.0, 20.0)], "w": 640.0, "h": 360.0}

    # Click empty space -> add a ball at the exact cursor position.
    balls.update(MouseEvent(x=400.0, y=300.0, pressed=True))
    assert len(balls._runtime["balls"]) == 2
    added = balls._runtime["balls"][-1]
    assert (added["x"], added["y"]) == (400.0, 300.0)

    # Click inside the original ball's circle hitbox -> remove it.
    balls.update(MouseEvent(x=100.0, y=100.0, pressed=True))
    remaining = balls._runtime["balls"]
    assert len(remaining) == 1
    assert (remaining[0]["x"], remaining[0]["y"]) == (400.0, 300.0)


def test_resize_moves_walls_to_live_dims() -> None:
    balls._runtime = {"balls": [_one_ball(100.0, 100.0, 20.0)], "w": 640.0, "h": 360.0}
    balls.update(Resize(width=1000.0, height=800.0))
    assert balls._runtime["w"] == 1000.0
    assert balls._runtime["h"] == 800.0


def test_floor_bounce_uses_live_height() -> None:
    # A ball resting past a shrunken floor is pushed back onto the new floor.
    balls._runtime = {"balls": [_one_ball(100.0, 350.0, 20.0)], "w": 640.0, "h": 360.0}
    balls._runtime["balls"][0]["vy"] = 100.0
    balls.update(Resize(width=640.0, height=200.0))
    balls.update(RenderFrame(frame_id=1, elapsed=balls.TARGET_DT))
    ball = balls._runtime["balls"][0]
    assert ball["y"] <= 200.0 - ball["r"] + 1e-6
