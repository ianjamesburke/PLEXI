#!/usr/bin/env python3
"""Breakout -- SDK v3 classic brick-breaking arcade game."""

from __future__ import annotations

import math

import plexi_sdk as sdk
from plexi_sdk import log
from plexi_sdk.effects import SetSchedulerMode, SetStatus, SetTitle
from plexi_sdk.events import KeyEvent, RenderFrame, Resize
from plexi_sdk.ui import Canvas, CanvasCircle, CanvasRect, CanvasText, Column, FooterKeys

TARGET_FPS = 60
TARGET_DT = 1.0 / TARGET_FPS

BRICK_COLS = 10
BRICK_ROWS = 5
BRICK_GAP = 4.0

BRICK_COLORS = ["#f38ba8", "#fab387", "#f9e2af", "#a6e3a1", "#89b4fa"]

STARTING_LIVES = 3

_runtime: dict | None = None


def _brick_width() -> float:
    return (0.9 * sdk.canvas_width - (BRICK_COLS - 1) * BRICK_GAP) / BRICK_COLS


def _brick_height() -> float:
    return _brick_width() * 0.32


def _paddle_width() -> float:
    return sdk.canvas_width * 0.125


def _paddle_height() -> float:
    return sdk.canvas_height * 0.033


def _ball_radius() -> float:
    return min(sdk.canvas_width, sdk.canvas_height) * 0.0167


def _ball_speed() -> float:
    return min(sdk.canvas_width, sdk.canvas_height) * 0.833


def _paddle_speed() -> float:
    return sdk.canvas_width * 0.625


def _brick_top_margin() -> float:
    return sdk.canvas_height * 0.139


def _paddle_y_offset() -> float:
    return sdk.canvas_height * 0.083


def _make_bricks() -> list[dict]:
    bw = _brick_width()
    bh = _brick_height()
    total_width = BRICK_COLS * bw + (BRICK_COLS - 1) * BRICK_GAP
    offset_x = (sdk.canvas_width - total_width) / 2.0
    bricks = []
    for row in range(BRICK_ROWS):
        color = BRICK_COLORS[row % len(BRICK_COLORS)]
        for col in range(BRICK_COLS):
            bricks.append({
                "x": offset_x + col * (bw + BRICK_GAP),
                "y": _brick_top_margin() + row * (bh + BRICK_GAP),
                "w": bw,
                "h": bh,
                "color": color,
                "alive": True,
            })
    return bricks


def _rebuild_brick_positions(data: dict) -> None:
    bw = _brick_width()
    bh = _brick_height()
    total_width = BRICK_COLS * bw + (BRICK_COLS - 1) * BRICK_GAP
    offset_x = (sdk.canvas_width - total_width) / 2.0
    for i, brick in enumerate(data["bricks"]):
        row = i // BRICK_COLS
        col = i % BRICK_COLS
        brick["x"] = offset_x + col * (bw + BRICK_GAP)
        brick["y"] = _brick_top_margin() + row * (bh + BRICK_GAP)
        brick["w"] = bw
        brick["h"] = bh


def _initial() -> dict:
    pw = _paddle_width()
    paddle_x = (sdk.canvas_width - pw) / 2.0
    paddle_y = sdk.canvas_height - _paddle_y_offset()
    return {
        "paddle_x": paddle_x,
        "paddle_y": paddle_y,
        "ball_x": paddle_x + pw / 2.0,
        "ball_y": paddle_y - _ball_radius(),
        "ball_vx": 0.0,
        "ball_vy": 0.0,
        "ball_attached": True,
        "bricks": _make_bricks(),
        "score": 0,
        "lives": STARTING_LIVES,
        "state": "playing",
    }


def _sim() -> dict:
    global _runtime
    if _runtime is None:
        _runtime = _initial()
    return _runtime


def _launch_ball(data: dict) -> None:
    if not data["ball_attached"]:
        return
    data["ball_attached"] = False
    speed = _ball_speed()
    data["ball_vx"] = speed * 0.6
    data["ball_vy"] = -speed * 0.8


def _reset_ball(data: dict) -> None:
    data["ball_attached"] = True
    data["ball_vx"] = 0.0
    data["ball_vy"] = 0.0
    data["ball_x"] = data["paddle_x"] + _paddle_width() / 2.0
    data["ball_y"] = data["paddle_y"] - _ball_radius()


def _restart(data: dict) -> None:
    fresh = _initial()
    data.update(fresh)


def _alive_count(data: dict) -> int:
    return sum(1 for b in data["bricks"] if b["alive"])


def init(size, args) -> list:
    global _runtime
    sdk.keys_held.clear()
    _runtime = _initial()
    effects: list = [
        SetTitle("Breakout"),
        SetSchedulerMode("continuous", fps=TARGET_FPS),
        SetStatus(f"Score: 0 | Lives: {STARTING_LIVES}"),
    ]
    log.info("breakout: SDK v3 canvas initialized")
    return effects


def update(event) -> list:
    if isinstance(event, Resize):
        data = _sim()
        data["paddle_y"] = sdk.canvas_height - _paddle_y_offset()
        _rebuild_brick_positions(data)
        if data["ball_attached"]:
            _reset_ball(data)
        return []

    if isinstance(event, KeyEvent):
        return _handle_key(event)

    if isinstance(event, RenderFrame):
        data = _sim()
        if data["state"] != "playing":
            return []
        dt = event.elapsed if event.elapsed > 0 else TARGET_DT
        effects = _step(data, min(dt, TARGET_DT * 2.0))
        return effects

    return []


def _handle_key(event: KeyEvent) -> list:
    data = _sim()

    if event.key == "space" and event.pressed:
        if data["state"] in ("gameover", "win"):
            _restart(data)
            return [SetStatus(f"Score: 0 | Lives: {STARTING_LIVES}")]
        if data["ball_attached"]:
            _launch_ball(data)
        return []

    return []


def _step(data: dict, dt: float) -> list:
    effects: list = []
    pw = _paddle_width()
    ph = _paddle_height()
    br = _ball_radius()
    ps = _paddle_speed()

    dx = 0.0
    if "left" in sdk.keys_held or "a" in sdk.keys_held:
        dx -= ps * dt
    if "right" in sdk.keys_held or "d" in sdk.keys_held:
        dx += ps * dt

    if dx != 0.0:
        data["paddle_x"] = max(0.0, min(sdk.canvas_width - pw, data["paddle_x"] + dx))
        if data["ball_attached"]:
            data["ball_x"] = data["paddle_x"] + pw / 2.0

    if data["ball_attached"]:
        return effects

    bx = data["ball_x"] + data["ball_vx"] * dt
    by = data["ball_y"] + data["ball_vy"] * dt

    if bx - br <= 0:
        bx = br
        data["ball_vx"] = abs(data["ball_vx"])
    elif bx + br >= sdk.canvas_width:
        bx = sdk.canvas_width - br
        data["ball_vx"] = -abs(data["ball_vx"])

    if by - br <= 0:
        by = br
        data["ball_vy"] = abs(data["ball_vy"])

    if by + br >= sdk.canvas_height:
        data["lives"] -= 1
        if data["lives"] <= 0:
            data["state"] = "gameover"
            effects.append(SetStatus("GAME OVER"))
        else:
            _reset_ball(data)
            effects.append(SetStatus(f"Score: {data['score']} | Lives: {data['lives']}"))
        return effects

    paddle_x = data["paddle_x"]
    paddle_y = data["paddle_y"]
    if (
        data["ball_vy"] > 0
        and by + br >= paddle_y
        and by - br < paddle_y + ph
        and bx + br >= paddle_x
        and bx - br <= paddle_x + pw
    ):
        by = paddle_y - br
        hit_pos = (bx - paddle_x) / pw
        angle = -2.4 + (-0.7 - -2.4) * hit_pos
        speed = math.sqrt(data["ball_vx"] ** 2 + data["ball_vy"] ** 2)
        data["ball_vx"] = speed * math.cos(angle)
        data["ball_vy"] = speed * math.sin(angle)

    score_before = data["score"]
    for brick in data["bricks"]:
        if not brick["alive"]:
            continue
        if (
            bx + br > brick["x"]
            and bx - br < brick["x"] + brick["w"]
            and by + br > brick["y"]
            and by - br < brick["y"] + brick["h"]
        ):
            brick["alive"] = False
            data["score"] += 10

            overlap_left = (bx + br) - brick["x"]
            overlap_right = (brick["x"] + brick["w"]) - (bx - br)
            overlap_top = (by + br) - brick["y"]
            overlap_bottom = (brick["y"] + brick["h"]) - (by - br)
            min_overlap = min(overlap_left, overlap_right, overlap_top, overlap_bottom)
            if min_overlap == overlap_left or min_overlap == overlap_right:
                data["ball_vx"] = -data["ball_vx"]
            else:
                data["ball_vy"] = -data["ball_vy"]
            break

    data["ball_x"] = bx
    data["ball_y"] = by

    if data["score"] != score_before:
        if _alive_count(data) == 0:
            data["state"] = "win"
            effects.append(SetStatus(f"YOU WIN! Score: {data['score']}"))
        else:
            effects.append(SetStatus(f"Score: {data['score']} | Lives: {data['lives']}"))

    return effects


def view():
    data = _sim()
    w, h = sdk.canvas_width, sdk.canvas_height
    return Column(
        [
            Canvas(_draw(data, w, h), width=w, height=100.0, grow=True),
            FooterKeys([("←/→", "move"), ("space", "launch / restart")]),
        ],
        padding=0,
        gap=0,
        grow=True,
    )


def _draw(data: dict, w: float, h: float) -> list:
    pw = _paddle_width()
    ph = _paddle_height()
    br = _ball_radius()
    commands: list = [CanvasRect(0, 0, w, h, "#0d0d1a")]

    for brick in data["bricks"]:
        if brick["alive"]:
            commands.append(
                CanvasRect(brick["x"], brick["y"], brick["w"], brick["h"], brick["color"], radius=2.0)
            )

    commands.append(
        CanvasRect(data["paddle_x"], data["paddle_y"], pw, ph, "#cdd6f4", radius=4.0)
    )

    commands.append(
        CanvasCircle(data["ball_x"], data["ball_y"], br, "#f5e0dc")
    )

    commands.append(
        CanvasText(10.0, 14.0, f"Score: {data['score']}", size=14.0, color="#cdd6f4", bold=True)
    )
    lives_text = "❤ " * data["lives"]
    commands.append(
        CanvasText(w - 10.0, 14.0, lives_text.strip(), size=14.0, color="#f38ba8", align="right_top")
    )

    if data["state"] == "gameover":
        commands.append(
            CanvasText(w / 2, h / 2 - 10, "GAME OVER", size=32.0, color="#f38ba8", bold=True, align="center_center")
        )
        commands.append(
            CanvasText(w / 2, h / 2 + 25, "Press SPACE to restart", size=14.0, color="#a6adc8", align="center_center")
        )
    elif data["state"] == "win":
        commands.append(
            CanvasText(w / 2, h / 2 - 10, "YOU WIN!", size=32.0, color="#a6e3a1", bold=True, align="center_center")
        )
        commands.append(
            CanvasText(w / 2, h / 2 + 25, "Press SPACE to restart", size=14.0, color="#a6adc8", align="center_center")
        )
    elif data["ball_attached"]:
        commands.append(
            CanvasText(w / 2, h / 2, "Press SPACE to launch", size=14.0, color="#a6adc8", align="center_center")
        )

    return commands
