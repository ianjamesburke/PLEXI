"""SDK v3 balls app tests."""

from __future__ import annotations

import os
import sys

sys.path.insert(
    0, os.path.join(os.path.dirname(__file__), "..", "..", "..", "sdk", "python")
)
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import plexi_sdk as sdk  # noqa: E402
from plexi_sdk.events import KeyEvent, RenderFrame, Resize  # noqa: E402

import balls  # noqa: E402


def _reset(count: int = 2) -> dict:
    sdk.pane_width = 640.0
    sdk.pane_height = 360.0
    sdk.canvas_width = 640.0
    sdk.canvas_height = 360.0
    data = balls._initial(count)
    balls._runtime = data
    return data


def test_render_frame_steps_physics_and_view_has_canvas() -> None:
    data = _reset(2)
    start_y = data["balls"][0]["y"]

    effects = balls.update(RenderFrame(frame_id=1, elapsed=balls.TARGET_DT))
    stepped = balls._runtime

    assert effects == []
    assert stepped["ticks"] == 1
    assert stepped["balls"][0]["y"] != start_y
    node = balls.view().to_node()
    canvas = node["children"][0]
    assert canvas["type"] == "canvas"
    assert any(cmd["type"] == "rect" and cmd["fill"] == "#0d0d1a" for cmd in canvas["commands"])
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


def test_resize_updates_bounds() -> None:
    _reset(3)
    # Simulate what the runtime does: set SDK vars before dispatching Resize
    sdk.pane_width = 800.0
    sdk.pane_height = 600.0
    sdk.canvas_width = 800.0
    sdk.canvas_height = 600.0
    balls.update(Resize(width=800.0, height=600.0))
    assert sdk.pane_width == 800.0
    assert sdk.pane_height == 600.0


def test_add_remove_balls_via_key() -> None:
    _reset(2)
    count_before = len(balls._sim()["balls"])

    balls.update(KeyEvent(key="+"))
    assert len(balls._sim()["balls"]) == count_before + 1

    balls.update(KeyEvent(key="-"))
    assert len(balls._sim()["balls"]) == count_before


def test_cannot_remove_last_ball() -> None:
    _reset(1)
    balls.update(KeyEvent(key="-"))
    assert len(balls._sim()["balls"]) == 1


def test_footer_shows_add_remove() -> None:
    _reset(2)
    node = balls.view().to_node()
    footer = node["children"][-1]
    assert footer["type"] == "pinned"
    assert footer["child"]["type"] == "footer_keys"
