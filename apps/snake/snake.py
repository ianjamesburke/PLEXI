#!/usr/bin/env python3
"""Snake — SDK v3 runtime-state canvas game."""

from __future__ import annotations

import random

from plexi_sdk import log, state
from plexi_sdk.effects import SetState, SetStatus, SetTimer, SetTitle
from plexi_sdk.events import KeyEvent, TimerFired
from plexi_sdk.ui import (
    AppBar,
    Canvas,
    CanvasRect,
    CanvasText,
    Column,
    FooterKeys,
    Spacer,
)

COLS = 26
ROWS = 20
CELL = 18.0
CANVAS_W = COLS * CELL + 48.0
CANVAS_H = ROWS * CELL + 48.0
TIMER_ID = 1
TICK_MS = 150

DIRS = {
    "up": [0, -1],
    "ArrowUp": [0, -1],
    "k": [0, -1],
    "down": [0, 1],
    "ArrowDown": [0, 1],
    "j": [0, 1],
    "left": [-1, 0],
    "ArrowLeft": [-1, 0],
    "h": [-1, 0],
    "right": [1, 0],
    "ArrowRight": [1, 0],
    "l": [1, 0],
}


def _initial() -> dict:
    mid_r, mid_c = ROWS // 2, COLS // 2
    return {
        "snake": [[mid_c, mid_r], [mid_c - 1, mid_r], [mid_c - 2, mid_r]],
        "direction": [1, 0],
        "next_direction": [1, 0],
        "food": [COLS - 3, mid_r],
        "score": 0,
        "alive": True,
    }


def _game() -> dict:
    data = _initial()
    for key, default in data.items():
        data[key] = state.get(key, default)
    data["snake"] = [list(cell) for cell in data["snake"]]
    data["direction"] = list(data["direction"])
    data["next_direction"] = list(data["next_direction"])
    data["food"] = list(data["food"])
    data["score"] = int(data["score"])
    data["alive"] = bool(data["alive"])
    return data


def _set(data: dict) -> list:
    return [SetState(data), SetStatus(_status(data))]


def _status(data: dict) -> str:
    return (
        f"Score: {data['score']}"
        if data["alive"]
        else f"Game over - score {data['score']}"
    )


def init(size, args) -> list:
    data = _game()
    missing = {
        key: value for key, value in _initial().items() if state.get(key, None) is None
    }
    effects: list = [
        SetTitle("Snake"),
        SetStatus(_status(data)),
        SetTimer(TIMER_ID, TICK_MS, repeat=True),
    ]
    if missing:
        effects.append(SetState(missing))
    log.info("snake: SDK v3 canvas initialized")
    return effects


def update(event) -> list:
    data = _game()
    if isinstance(event, TimerFired) and event.id == TIMER_ID:
        if not data["alive"]:
            return []
        return _set(_advance(data))

    if not isinstance(event, KeyEvent) or not event.pressed:
        return []

    if event.key in ("r", "R") and not data["alive"]:
        data = _initial()
        log.info("snake: restarted")
        return _set(data)

    next_dir = DIRS.get(event.key)
    if next_dir is None:
        return []
    cur = data["direction"]
    if next_dir != [-cur[0], -cur[1]]:
        data["next_direction"] = next_dir
        return _set(data)
    return []


def _advance(data: dict) -> dict:
    direction = data["next_direction"]
    cur = data["direction"]
    if direction != [-cur[0], -cur[1]]:
        data["direction"] = direction

    hx, hy = data["snake"][0]
    dx, dy = data["direction"]
    head = [(hx + dx) % COLS, (hy + dy) % ROWS]
    if head in data["snake"]:
        data["alive"] = False
        log.info(f"snake: game_over score={data['score']}")
        return data

    data["snake"].insert(0, head)
    if head == data["food"]:
        data["score"] += 1
        data["food"] = _next_food(data["snake"])
        log.info(f"snake: score={data['score']}")
    else:
        data["snake"].pop()
    return data


def _next_food(snake: list[list[int]]) -> list[int]:
    occupied = {tuple(cell) for cell in snake}
    free = [
        [c, r]
        for r in range(ROWS)
        for c in range(COLS)
        if (c, r) not in occupied
    ]
    if not free:
        return [-1, -1]
    return random.choice(free)


def view():
    data = _game()
    commands = _draw(data)
    subtitle = _status(data)
    keys = (
        [("h/j/k/l", "move"), ("r", "restart")]
        if not data["alive"]
        else [("h/j/k/l", "move")]
    )
    return Column(
        [
            AppBar("Snake", subtitle),
            Canvas(commands, width=CANVAS_W, height=CANVAS_H, grow=True, fit="contain"),
            Spacer(8.0),
            FooterKeys(keys),
        ],
        padding=0,
        gap=0,
        grow=True,
    )


def _draw(data: dict) -> list:
    ox = 24.0
    oy = 24.0
    commands: list = [
        CanvasRect(
            ox - 2, oy - 2, COLS * CELL + 4, ROWS * CELL + 4, "#6c7086", radius=2.0
        ),
        CanvasRect(ox, oy, COLS * CELL, ROWS * CELL, "#11111b"),
    ]
    fx, fy = data["food"]
    commands.append(
        CanvasRect(
            ox + fx * CELL + 2,
            oy + fy * CELL + 2,
            CELL - 4,
            CELL - 4,
            "#f38ba8",
            radius=4.0,
        )
    )
    for idx, (sx, sy) in enumerate(data["snake"]):
        fill = "#89b4fa" if idx == 0 else "#a6e3a1"
        commands.append(
            CanvasRect(
                ox + sx * CELL + 1,
                oy + sy * CELL + 1,
                CELL - 2,
                CELL - 2,
                fill,
                radius=2.0,
            )
        )
    if not data["alive"]:
        cx = CANVAS_W / 2
        cy = CANVAS_H / 2
        commands.extend(
            [
                CanvasRect(cx - 150, cy - 20, 300, 40, "#11111bcc", radius=6.0),
                CanvasText(
                    cx,
                    cy,
                    "GAME OVER - press R",
                    size=14.0,
                    color="#f38ba8",
                    bold=True,
                    align="center_center",
                ),
            ]
        )
    return commands
