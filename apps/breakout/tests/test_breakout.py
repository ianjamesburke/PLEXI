"""SDK v3 breakout app tests."""

from __future__ import annotations

import os
import sys

sys.path.insert(
    0, os.path.join(os.path.dirname(__file__), "..", "..", "..", "sdk", "python")
)
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from plexi_sdk.events import KeyEvent, RenderFrame, Resize  # noqa: E402

import breakout  # noqa: E402


def _reset() -> dict:
    breakout._canvas_width = 640.0
    breakout._canvas_height = 360.0
    breakout._keys_held.clear()
    data = breakout._initial()
    breakout._runtime = data
    return data


def test_init_uses_continuous_scheduler() -> None:
    effects = breakout.init((640.0, 360.0), [])
    assert any(
        type(e).__name__ == "SetSchedulerMode"
        and e.mode == "continuous"
        and e.fps == breakout.TARGET_FPS
        for e in effects
    )
    assert not any(type(e).__name__ == "SetTimer" for e in effects)


def test_resize_updates_bounds() -> None:
    _reset()
    breakout.update(Resize(width=800.0, height=600.0))
    assert breakout._canvas_width == 800.0
    assert breakout._canvas_height == 600.0


def test_ball_launch_via_space() -> None:
    data = _reset()
    assert data["ball_attached"] is True

    breakout.update(KeyEvent(key="space", pressed=True))
    assert data["ball_attached"] is False
    assert data["ball_vy"] < 0


def test_paddle_movement_via_arrow_keys() -> None:
    data = _reset()
    start_x = data["paddle_x"]

    breakout.update(KeyEvent(key="right", pressed=True))
    breakout.update(RenderFrame(frame_id=1, elapsed=breakout.TARGET_DT))
    assert data["paddle_x"] > start_x

    moved_x = data["paddle_x"]
    breakout.update(KeyEvent(key="right", pressed=False))
    breakout.update(KeyEvent(key="left", pressed=True))
    breakout.update(RenderFrame(frame_id=2, elapsed=breakout.TARGET_DT))
    assert data["paddle_x"] < moved_x
    breakout._keys_held.clear()


def test_arrow_tap_moves_once_and_release_does_not_stick() -> None:
    data = _reset()
    start_x = data["paddle_x"]

    breakout.update(KeyEvent(key="right", pressed=True))
    breakout.update(KeyEvent(key="right", pressed=False))
    moved_x = data["paddle_x"]
    assert moved_x > start_x

    breakout.update(RenderFrame(frame_id=1, elapsed=breakout.TARGET_DT))
    assert data["paddle_x"] == moved_x


def test_brick_collision_removes_brick_and_scores() -> None:
    data = _reset()
    breakout._launch_ball(data)

    first_brick = next(b for b in data["bricks"] if b["alive"])
    data["ball_x"] = first_brick["x"] + first_brick["w"] / 2
    data["ball_y"] = first_brick["y"] + first_brick["h"] + breakout._ball_radius() + 1
    data["ball_vx"] = 0.0
    data["ball_vy"] = -breakout._ball_speed()

    breakout._step(data, breakout.TARGET_DT)
    assert data["score"] == 10
    assert first_brick["alive"] is False


def test_ball_displacement_per_second_is_tick_rate_independent() -> None:
    """Physics is dt-based, so 1 real second of travel covers the same distance
    whether ticked at 30 fps or the old 60 fps. Guards the 0446 fps lock against
    a future regression that reintroduces fixed per-tick increments."""

    def _travel(fps: int) -> tuple[float, float]:
        breakout._canvas_width = 2000.0
        breakout._canvas_height = 2000.0
        breakout._keys_held.clear()
        data = breakout._initial()
        breakout._runtime = data
        for brick in data["bricks"]:
            brick["alive"] = False  # remove collisions; measure free flight
        data["ball_attached"] = False
        data["ball_x"] = data["ball_y"] = 1000.0
        data["ball_vx"] = 100.0
        data["ball_vy"] = -100.0
        start_x, start_y = data["ball_x"], data["ball_y"]
        dt = 1.0 / fps
        for _ in range(fps):  # fps frames of 1/fps == exactly one real second
            breakout._step(data, dt)
        return data["ball_x"] - start_x, data["ball_y"] - start_y

    dx_30, dy_30 = _travel(30)
    dx_60, dy_60 = _travel(60)

    # vx=100, vy=-100 px/s over 1.0 s -> (+100, -100) at either tick rate.
    assert abs(dx_30 - 100.0) < 1e-6 and abs(dy_30 + 100.0) < 1e-6
    assert abs(dx_30 - dx_60) < 1e-6 and abs(dy_30 - dy_60) < 1e-6


def test_view_returns_canvas_with_correct_structure() -> None:
    _reset()
    node = breakout.view().to_node()
    assert node is not None
    canvas = node["children"][0]
    assert canvas["type"] == "canvas"
    assert canvas["width"] == 640.0
    assert canvas["height"] == 360.0
    assert any(cmd["type"] == "rect" and cmd["fill"] == "#0d0d1a" for cmd in canvas["commands"])
    assert any(cmd["type"] == "circle" for cmd in canvas["commands"])
    footer = node["children"][-1]
    assert footer["type"] == "pinned"
    assert footer["child"]["type"] == "footer_keys"


def test_initial_frame_batches_contiguous_bricks() -> None:
    data = _reset()
    commands = breakout._draw(data, breakout._canvas_width, breakout._canvas_height)

    brick_colors = set(breakout.BRICK_COLORS)
    brick_commands = [
        command
        for command in commands
        if command.to_command().get("fill") in brick_colors
    ]
    assert len(brick_commands) == breakout.BRICK_ROWS
    assert len(commands) <= 20


def test_destroyed_brick_remains_a_hole_in_batched_row() -> None:
    data = _reset()
    missing = data["bricks"][4]
    missing["alive"] = False

    commands = breakout._draw(data, breakout._canvas_width, breakout._canvas_height)
    row_color = breakout.BRICK_COLORS[0]
    row_rects = [
        command.to_command()
        for command in commands
        if command.to_command().get("fill") == row_color
    ]

    assert len(row_rects) == 2
    assert all(
        rect["x"] + rect["w"] <= missing["x"]
        or rect["x"] >= missing["x"] + missing["w"]
        for rect in row_rects
    )


def test_game_over_on_lives_exhausted() -> None:
    data = _reset()
    data["lives"] = 1
    breakout._launch_ball(data)
    data["ball_x"] = breakout._canvas_width / 2
    data["ball_y"] = breakout._canvas_height - breakout._ball_radius() - 1
    data["ball_vx"] = 0.0
    data["ball_vy"] = breakout._ball_speed()

    breakout._step(data, breakout.TARGET_DT)
    assert data["state"] == "gameover"
    assert data["lives"] == 0


def test_win_condition() -> None:
    data = _reset()
    breakout._launch_ball(data)
    for brick in data["bricks"]:
        brick["alive"] = False
    last_brick = data["bricks"][-1]
    last_brick["alive"] = True

    data["ball_x"] = last_brick["x"] + last_brick["w"] / 2
    data["ball_y"] = last_brick["y"] + last_brick["h"] + breakout._ball_radius() + 1
    data["ball_vx"] = 0.0
    data["ball_vy"] = -breakout._ball_speed()

    breakout._step(data, breakout.TARGET_DT)
    assert data["state"] == "win"
    assert last_brick["alive"] is False


def test_restart_after_game_over() -> None:
    data = _reset()
    data["state"] = "gameover"
    data["lives"] = 0
    data["score"] = 420

    breakout.update(KeyEvent(key="space", pressed=True))
    assert data["state"] == "playing"
    assert data["lives"] == breakout.STARTING_LIVES
    assert data["score"] == 0
