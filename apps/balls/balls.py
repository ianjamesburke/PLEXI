#!/usr/bin/env python3
"""Balls — elastic circle collision physics demo.

Runs against live pane dimensions each frame: walls track pane resizes, so
balls fall into freed space when a pane below closes. Absolute pixel radii,
1:1 canvas (fill mode with live width/height), click to add a ball at the
cursor, click a ball to remove it.
"""

from __future__ import annotations

import math
import random

from plexi_sdk import AppPalette, dim, log, theme
from plexi_sdk.effects import SetSchedulerMode, SetStatus, SetTitle
from plexi_sdk.events import MouseEvent, RenderFrame, Resize
from plexi_sdk.ui import AppBar, Canvas, CanvasCircle, CanvasRect, Column, FooterKeys

TARGET_FPS = 60
TARGET_DT = 1.0 / TARGET_FPS
MAX_DT = 0.05  # cap to avoid tunnelling on first frame / tab-back
GRAVITY = 300.0  # px/s^2
DAMPING = 0.78  # energy retained on wall bounce
FRICTION = 0.995  # per-frame horizontal friction on floor contact
MAX_BALLS = 50

BALL_PALETTE = AppPalette(
    dark={
        "c0": "#f7768e",
        "c1": "#9ece6a",
        "c2": "#7aa2f7",
        "c3": "#e0af68",
        "c4": "#bb9af7",
        "c5": "#7dcfff",
        "c6": "#ff9e64",
        "c7": "#2ac3de",
        "c8": "#b4f9f8",
        "c9": "#db4b4b",
        "c10": "#73daca",
        "c11": "#c0caf5",
    },
    light={
        "c0": "#f52a65",
        "c1": "#587539",
        "c2": "#2e7de9",
        "c3": "#8c6c3e",
        "c4": "#7847bd",
        "c5": "#007197",
        "c6": "#b15c00",
        "c7": "#188092",
        "c8": "#387068",
        "c9": "#c64343",
        "c10": "#118c74",
        "c11": "#3760bf",
    },
)


def _palette() -> list[str]:
    pal = BALL_PALETTE.resolve(theme)
    return [pal[f"c{i}"] for i in range(12)]


_runtime: dict | None = None


def _new_ball(
    x: float,
    y: float,
    idx: int,
    vx: float | None = None,
    vy: float | None = None,
) -> dict:
    radius = random.uniform(14.0, 44.0)
    colors = _palette()
    return {
        "x": x,
        "y": y,
        "vx": random.uniform(-180.0, 180.0) if vx is None else vx,
        "vy": random.uniform(-250.0, -60.0) if vy is None else vy,
        "r": radius,
        "color": colors[idx % len(colors)],
    }


def _initial(w: float, h: float, count: int) -> dict:
    count = max(1, min(count, MAX_BALLS))
    balls = []
    for idx in range(count):
        ball = _new_ball(0.0, 0.0, idx)
        ball["x"] = random.uniform(ball["r"], max(ball["r"], w - ball["r"]))
        ball["y"] = random.uniform(ball["r"], max(ball["r"], h * 0.6))
        balls.append(ball)
    return {"balls": balls, "w": w, "h": h}


def _sim() -> dict:
    global _runtime
    if _runtime is None:
        _runtime = _initial(640.0, 360.0, 10)
    return _runtime


def init(size, args) -> list:
    global _runtime
    w, h = size
    count = 10
    if args:
        try:
            count = int(args[0])
        except (TypeError, ValueError):
            count = 10
    _runtime = _initial(w, h, count)
    effects: list = [
        SetTitle("Balls"),
        SetSchedulerMode("continuous", fps=TARGET_FPS),
        SetStatus(f"{len(_runtime['balls'])} balls"),
    ]
    log.info(f"balls: init complete, spawned {len(_runtime['balls'])} balls")
    return effects


def update(event) -> list:
    data = _sim()

    if isinstance(event, Resize):
        data["w"] = event.width
        data["h"] = event.height
        return []

    if isinstance(event, MouseEvent) and event.pressed:
        return _click(data, event.x, event.y)

    if isinstance(event, RenderFrame):
        dt = event.elapsed if event.elapsed > 0 else TARGET_DT
        _step(data, min(dt, MAX_DT))
        return []

    return []


def _click(data: dict, x: float, y: float) -> list:
    balls = data["balls"]
    # Topmost ball is drawn last, so hit-test from the end.
    for i in range(len(balls) - 1, -1, -1):
        ball = balls[i]
        if (x - ball["x"]) ** 2 + (y - ball["y"]) ** 2 <= ball["r"] ** 2:
            balls.pop(i)
            log.info(f"balls: removed ball at ({x:.0f}, {y:.0f}), count={len(balls)}")
            return [SetStatus(f"{len(balls)} balls")]
    if len(balls) < MAX_BALLS:
        balls.append(_new_ball(x, y, len(balls), vy=random.uniform(-300.0, -120.0)))
        log.info(f"balls: spawned ball at ({x:.0f}, {y:.0f}), count={len(balls)}")
        return [SetStatus(f"{len(balls)} balls")]
    return []


def _step(data: dict, dt: float) -> None:
    balls = data["balls"]
    w = data["w"]
    h = data["h"]

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

    n = len(balls)
    for i in range(n):
        for j in range(i + 1, n):
            _collide(balls[i], balls[j])


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
    am = a["r"] * a["r"]  # mass proportional to area (uniform density)
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
    w = data["w"]
    h = data["h"]
    balls = data["balls"]
    return Column(
        [
            AppBar("Balls", f"{len(balls)} balls"),
            Canvas(_draw(data), width=w, height=h, grow=True),
            FooterKeys([("click", "add ball"), ("click ball", "remove")]),
        ],
        gap=0,
        grow=True,
    )


def _draw(data: dict) -> list:
    w = data["w"]
    h = data["h"]
    balls = data["balls"]
    commands: list = [CanvasRect(0.0, 0.0, w, h, theme.bg_darkest)]
    shadow = dim(theme.bg_darkest, 160)
    glint = dim(theme.fg, 90)
    for ball in balls:
        commands.append(
            CanvasCircle(ball["x"] + 3.0, ball["y"] + 4.0, ball["r"], shadow)
        )
    for ball in balls:
        commands.append(CanvasCircle(ball["x"], ball["y"], ball["r"], ball["color"]))
        commands.append(
            CanvasCircle(
                ball["x"] - ball["r"] * 0.28,
                ball["y"] - ball["r"] * 0.28,
                max(3.0, ball["r"] * 0.28),
                glint,
            )
        )
    return commands

