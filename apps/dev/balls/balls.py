#!/usr/bin/env python3
"""Balls — SDK v3 runtime-state canvas physics demo."""

from __future__ import annotations

import math
import random

from plexi_sdk import log
from plexi_sdk.effects import SetSchedulerMode, SetStatus, SetTitle
from plexi_sdk.events import KeyEvent, RenderFrame
from plexi_sdk.ui import (
    AppBar,
    Canvas,
    CanvasCircle,
    CanvasRect,
    CanvasText,
    Column,
    FooterKeys,
)

CANVAS_W = 640.0
CANVAS_H = 360.0
TARGET_FPS = 60
TARGET_DT = 1.0 / TARGET_FPS
GRAVITY = 300.0
DAMPING = 0.78
FRICTION = 0.995
MAX_BALLS = 50
_runtime: dict | None = None
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


def _initial(count: int = 10) -> dict:
    rng = random.Random(7)
    balls = []
    for idx in range(max(1, min(count, MAX_BALLS))):
        radius = rng.uniform(14.0, 36.0)
        balls.append(
            {
                "x": rng.uniform(radius, CANVAS_W - radius),
                "y": rng.uniform(radius, CANVAS_H * 0.55),
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
    if isinstance(event, KeyEvent):
        if event.key in ("+", "plus", "equals"):
            _sim()["balls"].extend(_initial(1)["balls"])
        elif event.key in ("-", "minus") and len(_sim()["balls"]) > 1:
            _sim()["balls"].pop()
        return []
    if not isinstance(event, RenderFrame):
        return []
    dt = event.elapsed if event.elapsed > 0 else TARGET_DT
    _step(_sim(), min(dt, TARGET_DT * 2.0))
    return []


def _step(data: dict, dt: float = TARGET_DT) -> dict:
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
        elif ball["x"] + radius > CANVAS_W:
            ball["x"] = CANVAS_W - radius
            ball["vx"] = -abs(ball["vx"]) * DAMPING
        if ball["y"] - radius < 0:
            ball["y"] = radius
            ball["vy"] = abs(ball["vy"]) * DAMPING
        elif ball["y"] + radius > CANVAS_H:
            ball["y"] = CANVAS_H - radius
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
    count = len(data["balls"])
    return Column(
        [
            AppBar("Balls", f"{count} balls"),
            Canvas(_draw(data), width=CANVAS_W, height=CANVAS_H, grow=True),
            FooterKeys([("+", "add"), ("-", "remove")]),
        ],
        padding=0,
        gap=0,
        grow=True,
    )


def _draw(data: dict) -> list:
    commands: list = [CanvasRect(0, 0, CANVAS_W, CANVAS_H, "#0d0d1a")]
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
    commands.append(
        CanvasText(12.0, 18.0, f"ticks {data['ticks']}", size=11.0, color="#a6adc8")
    )
    return commands
