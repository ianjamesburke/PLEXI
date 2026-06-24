#!/usr/bin/env python3
"""Balls -- SDK v3 runtime-state canvas physics demo."""

from __future__ import annotations

import math
import random

import plexi_sdk as sdk
from plexi_sdk import log
from plexi_sdk.effects import SetSchedulerMode, SetStatus, SetTitle
from plexi_sdk.events import KeyEvent, MouseEvent, RenderFrame, Resize, UiAction
from plexi_sdk.ui import Canvas, CanvasCircle, CanvasRect, CanvasText, Column, FooterKeys

TARGET_FPS = 60
TARGET_DT = 1.0 / TARGET_FPS
GRAVITY = 300.0
DAMPING = 0.78
FRICTION = 0.995
MAX_BALLS = 50
PALETTE = [
    "#f38ba8",
    "#a6e3a1",
    "#89b4fa",
    "#f9e2af",
    "#cba6f7",
    "#94e2d5",
    "#fab387",
    "#74c7ec",
]

_runtime: dict | None = None


def _initial(count: int = 10) -> dict:
    w, h = sdk.canvas_width, sdk.canvas_height
    rng = random.Random(7)
    balls = []
    for idx in range(max(1, min(count, MAX_BALLS))):
        radius = rng.uniform(14.0, 36.0)
        balls.append(
            {
                "x": rng.uniform(radius, max(radius + 1, w - radius)),
                "y": rng.uniform(radius, max(radius + 1, h * 0.55)),
                "vx": rng.uniform(-180.0, 180.0),
                "vy": rng.uniform(-250.0, -60.0),
                "r": radius,
                "color": PALETTE[idx % len(PALETTE)],
            }
        )
    return {"balls": balls, "ticks": 0}


def _sim() -> dict:
    global _runtime
    if _runtime is None:
        _runtime = _initial()
    return _runtime


def _add_ball(x: float | None = None, y: float | None = None) -> None:
    data = _sim()
    if len(data["balls"]) >= MAX_BALLS:
        return
    rng = random.Random()
    w, h = sdk.canvas_width, sdk.canvas_height
    radius = rng.uniform(14.0, 36.0)
    bx = x if x is not None else rng.uniform(radius, max(radius + 1, w - radius))
    by = y if y is not None else rng.uniform(radius, max(radius + 1, h * 0.3))
    data["balls"].append(
        {
            "x": bx,
            "y": by,
            "vx": rng.uniform(-180.0, 180.0),
            "vy": rng.uniform(-250.0, -60.0),
            "r": radius,
            "color": PALETTE[len(data["balls"]) % len(PALETTE)],
        }
    )


def _remove_ball() -> None:
    data = _sim()
    if len(data["balls"]) > 1:
        data["balls"].pop()


def init(size, args) -> list:
    global _runtime
    count = 10
    if args:
        try:
            count = int(args[0])
        except (TypeError, ValueError):
            count = 10
    _runtime = _initial(count)
    effects: list = [
        SetTitle("Balls"),
        SetSchedulerMode("continuous", fps=TARGET_FPS),
        SetStatus(f"{len(_runtime['balls'])} balls"),
    ]
    log.info("balls: SDK v3 canvas initialized")
    return effects


def update(event) -> list:
    if isinstance(event, Resize):
        return []

    if isinstance(event, MouseEvent):
        data = _sim()
        for i, ball in enumerate(data["balls"]):
            dx = event.x - ball["x"]
            dy = event.y - ball["y"]
            if dx * dx + dy * dy <= ball["r"] * ball["r"]:
                if len(data["balls"]) > 1:
                    data["balls"].pop(i)
                return [SetStatus(f"{len(data['balls'])} balls")]
        _add_ball(event.x, event.y)
        return [SetStatus(f"{len(_sim()['balls'])} balls")]

    if isinstance(event, KeyEvent):
        if event.key == "=" or event.key == "+":
            _add_ball()
            return [SetStatus(f"{len(_sim()['balls'])} balls")]
        if event.key == "-":
            _remove_ball()
            return [SetStatus(f"{len(_sim()['balls'])} balls")]
        return []

    if isinstance(event, UiAction):
        if event.handler_id == "add_ball":
            _add_ball()
            return [SetStatus(f"{len(_sim()['balls'])} balls")]
        if event.handler_id == "remove_ball":
            _remove_ball()
            return [SetStatus(f"{len(_sim()['balls'])} balls")]
        return []

    if not isinstance(event, RenderFrame):
        return []
    dt = event.elapsed if event.elapsed > 0 else TARGET_DT
    _step(_sim(), min(dt, TARGET_DT * 2.0))
    return []


def _step(data: dict, dt: float = TARGET_DT) -> dict:
    w, h = sdk.canvas_width, sdk.canvas_height
    balls = data["balls"]
    for ball in balls:
        ball["vy"] += GRAVITY * dt
        ball["x"] += ball["vx"] * dt
        ball["y"] += ball["vy"] * dt

    for ball in balls:
        radius = ball["r"]
        if ball["x"] - radius < 0:
            ball["x"] = radius
            ball["vx"] = abs(ball["vx"]) * DAMPING
        elif ball["x"] + radius > w:
            ball["x"] = w - radius
            ball["vx"] = -abs(ball["vx"]) * DAMPING
        if ball["y"] - radius < 0:
            ball["y"] = radius
            ball["vy"] = abs(ball["vy"]) * DAMPING
        elif ball["y"] + radius > h:
            ball["y"] = h - radius
            ball["vy"] = -abs(ball["vy"]) * DAMPING
            ball["vx"] *= FRICTION

    for i in range(len(balls)):
        for j in range(i + 1, len(balls)):
            _collide(balls[i], balls[j])

    data["ticks"] += 1
    return data


def _collide(a: dict, b: dict) -> None:
    dx = b["x"] - a["x"]
    dy = b["y"] - a["y"]
    min_dist = a["r"] + b["r"]
    dist_sq = dx * dx + dy * dy
    if dist_sq >= min_dist * min_dist:
        return
    if dist_sq > 0:
        dist = math.sqrt(dist_sq)
        nx, ny = dx / dist, dy / dist
    else:
        dist = 0.0
        nx, ny = 1.0, 0.0
    overlap = min_dist - dist
    am = a["r"] * a["r"]
    bm = b["r"] * b["r"]
    total = am + bm
    a["x"] -= nx * overlap * bm / total
    a["y"] -= ny * overlap * bm / total
    b["x"] += nx * overlap * am / total
    b["y"] += ny * overlap * am / total

    rel_vx = a["vx"] - b["vx"]
    rel_vy = a["vy"] - b["vy"]
    approach = rel_vx * nx + rel_vy * ny
    if approach <= 0:
        return
    impulse = 2.0 * approach / total
    a["vx"] -= impulse * bm * nx
    a["vy"] -= impulse * bm * ny
    b["vx"] += impulse * am * nx
    b["vy"] += impulse * am * ny


def view():
    data = _sim()
    w, h = sdk.canvas_width, sdk.canvas_height
    return Column(
        [
            Canvas(_draw(data, w, h), width=w, height=100.0, grow=True),
            FooterKeys([("click", "add/remove"), ("+/-", "add/remove")]),
        ],
        padding=0,
        gap=0,
        grow=True,
    )


def _draw(data: dict, w: float, h: float) -> list:
    commands: list = [CanvasRect(0, 0, w, h, "#0d0d1a")]
    for ball in data["balls"]:
        commands.append(
            CanvasCircle(ball["x"] + 3.0, ball["y"] + 4.0, ball["r"], "#00000055")
        )
    for ball in data["balls"]:
        commands.append(CanvasCircle(ball["x"], ball["y"], ball["r"], ball["color"]))
        commands.append(
            CanvasCircle(
                ball["x"] - ball["r"] * 0.28,
                ball["y"] - ball["r"] * 0.28,
                max(3.0, ball["r"] * 0.28),
                "#ffffff66",
            )
        )
    count = len(data["balls"])
    commands.append(
        CanvasText(12.0, 18.0, f"{count} balls | tick {data['ticks']}", size=11.0, color="#a6adc8")
    )
    return commands
