#!/usr/bin/env python3
"""Snake — SDK v3 runtime-state canvas game."""

from __future__ import annotations

import plexi_sdk as sdk
from plexi_sdk import log, state
from plexi_sdk.effects import SetState, SetStatus, SetTimer, SetTitle
from plexi_sdk.events import KeyEvent, Resize, TimerFired
from plexi_sdk.ui import (
    Canvas,
    CanvasRect,
    CanvasText,
    Column,
    FooterKeys,
)

COLS = 26
ROWS = 20
GRID_MARGIN = 48.0
TIMER_ID = 1
TICK_MS = 150

def _cell_size() -> float:
    return min((sdk.canvas_width - GRID_MARGIN) / COLS, (sdk.canvas_height - GRID_MARGIN) / ROWS)


def _grid_origin() -> tuple[float, float]:
    cell = _cell_size()
    ox = (sdk.canvas_width - COLS * cell) / 2
    oy = (sdk.canvas_height - ROWS * cell) / 2
    return ox, oy

DIRS = {
    "up": [0, -1],
    "k": [0, -1],
    "down": [0, 1],
    "j": [0, 1],
    "left": [-1, 0],
    "h": [-1, 0],
    "right": [1, 0],
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
    if isinstance(event, Resize):
        return []

    data = _game()
    if isinstance(event, TimerFired) and event.id == TIMER_ID:
        if not data["alive"]:
            return []
        return _set(_advance(data))

    if not isinstance(event, KeyEvent) or not event.pressed:
        return []

    if event.key == "r" and not data["alive"]:
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
        data["food"] = _next_food(data["snake"], data["score"])
        log.info(f"snake: score={data['score']}")
    else:
        data["snake"].pop()
    return data


def _next_food(snake: list[list[int]], score: int) -> list[int]:
    occupied = {tuple(cell) for cell in snake}
    start = (score * 17 + len(snake) * 7) % (COLS * ROWS)
    for offset in range(COLS * ROWS):
        idx = (start + offset) % (COLS * ROWS)
        cell = [idx % COLS, idx // COLS]
        if tuple(cell) not in occupied:
            return cell
    return [-1, -1]


def view():
    data = _game()
    commands = _draw(data)
    w = sdk.canvas_width
    keys = (
        [("h/j/k/l", "move"), ("r", "restart")]
        if not data["alive"]
        else [("h/j/k/l", "move")]
    )
    return Column(
        [
            Canvas(commands, width=w, height=100.0, grow=True),
            FooterKeys(keys),
        ],
        padding=0,
        gap=0,
        grow=True,
    )


def _draw(data: dict) -> list:
    cell = _cell_size()
    ox, oy = _grid_origin()
    grid_w = COLS * cell
    grid_h = ROWS * cell
    commands: list = [
        CanvasRect(0, 0, sdk.canvas_width, sdk.canvas_height, "#0d0d1a"),
        CanvasRect(ox, oy, grid_w, grid_h, "#11111b", radius=2.0,
                   border_color="#6c7086", border_width=2.0),
    ]
    fx, fy = data["food"]
    commands.append(
        CanvasRect(
            ox + fx * cell + 2,
            oy + fy * cell + 2,
            cell - 4,
            cell - 4,
            "#f38ba8",
            radius=4.0,
        )
    )
    for idx, (sx, sy) in enumerate(data["snake"]):
        fill = "#89b4fa" if idx == 0 else "#a6e3a1"
        commands.append(
            CanvasRect(
                ox + sx * cell + 1,
                oy + sy * cell + 1,
                cell - 2,
                cell - 2,
                fill,
                radius=2.0,
            )
        )
    if not data["alive"]:
        cx = sdk.canvas_width / 2
        cy = sdk.canvas_height / 2
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
