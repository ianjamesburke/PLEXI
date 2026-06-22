#!/usr/bin/env python3
"""Tetris — SDK v3 state-backed canvas game."""

from __future__ import annotations

import random

from plexi_sdk import log, state
from plexi_sdk.effects import SetState, SetStatus, SetTimer, SetTitle
from plexi_sdk.events import KeyEvent, TimerFired
from plexi_sdk.ui import AppBar, Canvas, CanvasRect, CanvasText, Column, FooterKeys

ROWS = 20
COLS = 10
CELL = 20.0
CANVAS_W = 360.0
CANVAS_H = 440.0
TIMER_ID = 1
TICK_MS = 100

PIECES = {
    "I": {
        "color": "#94e2d5",
        "shapes": [
            [(1, 0), (1, 1), (1, 2), (1, 3)],
            [(0, 2), (1, 2), (2, 2), (3, 2)],
            [(2, 0), (2, 1), (2, 2), (2, 3)],
            [(0, 1), (1, 1), (2, 1), (3, 1)],
        ],
    },
    "O": {"color": "#f9e2af", "shapes": [[(0, 0), (0, 1), (1, 0), (1, 1)]] * 4},
    "T": {
        "color": "#cba6f7",
        "shapes": [
            [(0, 1), (1, 0), (1, 1), (1, 2)],
            [(0, 1), (1, 1), (1, 2), (2, 1)],
            [(1, 0), (1, 1), (1, 2), (2, 1)],
            [(0, 1), (1, 0), (1, 1), (2, 1)],
        ],
    },
    "S": {
        "color": "#a6e3a1",
        "shapes": [
            [(0, 1), (0, 2), (1, 0), (1, 1)],
            [(0, 1), (1, 1), (1, 2), (2, 2)],
            [(1, 1), (1, 2), (2, 0), (2, 1)],
            [(0, 0), (1, 0), (1, 1), (2, 1)],
        ],
    },
    "Z": {
        "color": "#f38ba8",
        "shapes": [
            [(0, 0), (0, 1), (1, 1), (1, 2)],
            [(0, 2), (1, 1), (1, 2), (2, 1)],
            [(1, 0), (1, 1), (2, 1), (2, 2)],
            [(0, 1), (1, 0), (1, 1), (2, 0)],
        ],
    },
    "J": {
        "color": "#89b4fa",
        "shapes": [
            [(0, 0), (1, 0), (1, 1), (1, 2)],
            [(0, 1), (0, 2), (1, 1), (2, 1)],
            [(1, 0), (1, 1), (1, 2), (2, 2)],
            [(0, 1), (1, 1), (2, 0), (2, 1)],
        ],
    },
    "L": {
        "color": "#fab387",
        "shapes": [
            [(0, 2), (1, 0), (1, 1), (1, 2)],
            [(0, 1), (1, 1), (2, 1), (2, 2)],
            [(1, 0), (1, 1), (1, 2), (2, 0)],
            [(0, 0), (0, 1), (1, 1), (2, 1)],
        ],
    },
}
PIECE_KEYS = list(PIECES)


def _spawn(key: str | None = None) -> dict:
    return {"key": key or random.choice(PIECE_KEYS), "rot": 0, "row": 0, "col": 3}


def _initial() -> dict:
    current = _spawn()
    return {
        "board": [[None] * COLS for _ in range(ROWS)],
        "current": current,
        "next": _spawn()["key"],
        "score": 0,
        "lines": 0,
        "alive": True,
        "paused": False,
        "fall_ticks": 0,
    }


def _game() -> dict:
    data = _initial()
    for key, default in data.items():
        data[key] = state.get(key, default)
    data["board"] = [list(row) for row in data["board"]]
    data["current"] = dict(data["current"])
    data["next"] = str(data["next"])
    for key in ("score", "lines", "fall_ticks"):
        data[key] = int(data[key])
    data["alive"] = bool(data["alive"])
    data["paused"] = bool(data["paused"])
    return data


def init(size, args) -> list:
    data = _game()
    missing = {
        key: value for key, value in _initial().items() if state.get(key, None) is None
    }
    effects: list = [
        SetTitle("Tetris"),
        SetStatus(_status(data)),
        SetTimer(TIMER_ID, TICK_MS, repeat=True),
    ]
    if missing:
        effects.append(SetState(missing))
    log.info("tetris: SDK v3 canvas initialized")
    return effects


def update(event) -> list:
    data = _game()
    if isinstance(event, TimerFired) and event.id == TIMER_ID:
        if not data["alive"] or data["paused"]:
            return []
        data["fall_ticks"] += 1
        if data["fall_ticks"] >= _drop_ticks(data):
            data["fall_ticks"] = 0
            data = _soft_drop(data, score=False)
        return _set(data)

    if not isinstance(event, KeyEvent) or not event.pressed:
        return []
    key = event.key
    if key in ("r", "R"):
        data = _initial()
        log.info("tetris: restarted")
        return _set(data)
    if key == "p":
        data["paused"] = not data["paused"]
        return _set(data)
    if not data["alive"] or data["paused"]:
        return []
    if key in ("left", "ArrowLeft", "h"):
        return _set(_move(data, dc=-1))
    if key in ("right", "ArrowRight", "l"):
        return _set(_move(data, dc=1))
    if key in ("down", "ArrowDown", "j"):
        return _set(_soft_drop(data, score=True))
    if key in ("up", "ArrowUp", "k", "x"):
        return _set(_rotate(data))
    if key in ("space", "enter", "return", "Enter"):
        while _valid(data, _shift(data["current"], dr=1)):
            data["current"]["row"] += 1
            data["score"] += 2
        return _set(_lock(data))
    return []


def _set(data: dict) -> list:
    return [SetState(data), SetStatus(_status(data))]


def _status(data: dict) -> str:
    if not data["alive"]:
        return f"Game over - score {data['score']}"
    return f"Score {data['score']} - lines {data['lines']}"


def _drop_ticks(data: dict) -> int:
    level = data["lines"] // 10
    return max(1, 10 - level)


def _cells(piece: dict) -> list[tuple[int, int]]:
    shape = PIECES[piece["key"]]["shapes"][int(piece["rot"]) % 4]
    return [(int(piece["row"]) + r, int(piece["col"]) + c) for r, c in shape]


def _shift(piece: dict, dr: int = 0, dc: int = 0, rot: int | None = None) -> dict:
    out = dict(piece)
    out["row"] = int(out["row"]) + dr
    out["col"] = int(out["col"]) + dc
    if rot is not None:
        out["rot"] = rot % 4
    return out


def _valid(data: dict, piece: dict) -> bool:
    for row, col in _cells(piece):
        if col < 0 or col >= COLS or row >= ROWS:
            return False
        if row >= 0 and data["board"][row][col] is not None:
            return False
    return True


def _move(data: dict, dc: int) -> dict:
    moved = _shift(data["current"], dc=dc)
    if _valid(data, moved):
        data["current"] = moved
    return data


def _rotate(data: dict) -> dict:
    current = data["current"]
    for dc in (0, -1, 1, -2, 2):
        rotated = _shift(current, dc=dc, rot=int(current["rot"]) + 1)
        if _valid(data, rotated):
            data["current"] = rotated
            break
    return data


def _soft_drop(data: dict, score: bool) -> dict:
    moved = _shift(data["current"], dr=1)
    if _valid(data, moved):
        data["current"] = moved
        if score:
            data["score"] += 1
    else:
        data = _lock(data)
    return data


def _lock(data: dict) -> dict:
    color = PIECES[data["current"]["key"]]["color"]
    for row, col in _cells(data["current"]):
        if row >= 0:
            data["board"][row][col] = color
    full = [row for row in range(ROWS) if all(data["board"][row])]
    for row in reversed(full):
        del data["board"][row]
        data["board"].insert(0, [None] * COLS)
    if full:
        data["lines"] += len(full)
        data["score"] += [0, 100, 300, 500, 800][len(full)] * (data["lines"] // 10 + 1)
        log.info(f"tetris: cleared {len(full)} lines score={data['score']}")
    data["current"] = _spawn(data["next"])
    data["next"] = _spawn()["key"]
    if not _valid(data, data["current"]):
        data["alive"] = False
        log.info(f"tetris: game_over score={data['score']}")
    return data


def view():
    data = _game()
    if not data["alive"]:
        keys = [("r", "restart")]
    elif data["paused"]:
        keys = [("p", "resume"), ("r", "restart")]
    else:
        keys = [("arrows", "move"), ("space", "drop"), ("p", "pause")]
    return Column(
        [
            AppBar("Tetris", _status(data)),
            Canvas(_draw(data), width=CANVAS_W, height=CANVAS_H, grow=True),
            FooterKeys(keys),
        ],
        padding=0,
        gap=0,
        grow=True,
    )


def _draw(data: dict) -> list:
    bx = 28.0
    by = 20.0
    commands: list = [
        CanvasRect(
            bx - 2, by - 2, COLS * CELL + 4, ROWS * CELL + 4, "#11111b", radius=2.0
        )
    ]
    for row in range(ROWS):
        for col in range(COLS):
            fill = data["board"][row][col]
            if fill:
                _cell(commands, bx, by, row, col, fill)
    if data["alive"]:
        for row, col in _cells(data["current"]):
            if row >= 0:
                _cell(
                    commands, bx, by, row, col, PIECES[data["current"]["key"]]["color"]
                )
    px = bx + COLS * CELL + 28.0
    commands.extend(
        [
            CanvasText(px, by, "SCORE", size=10.0, color="#a6adc8"),
            CanvasText(
                px, by + 18.0, str(data["score"]), size=18.0, color="#cdd6f4", bold=True
            ),
            CanvasText(px, by + 52.0, "LINES", size=10.0, color="#a6adc8"),
            CanvasText(px, by + 70.0, str(data["lines"]), size=16.0, color="#cdd6f4"),
            CanvasText(px, by + 108.0, "NEXT", size=10.0, color="#a6adc8"),
        ]
    )
    for row, col in PIECES[data["next"]]["shapes"][0]:
        commands.append(
            CanvasRect(
                px + col * 14.0,
                by + 126.0 + row * 14.0,
                12.0,
                12.0,
                PIECES[data["next"]]["color"],
                radius=2.0,
            )
        )
    if not data["alive"]:
        commands.extend(
            [
                CanvasRect(bx + 12.0, by + 160.0, 176.0, 54.0, "#000000cc", radius=4.0),
                CanvasText(
                    bx + 100.0,
                    by + 187.0,
                    "GAME OVER",
                    size=18.0,
                    color="#f38ba8",
                    bold=True,
                    align="center",
                ),
            ]
        )
    elif data["paused"]:
        commands.extend(
            [
                CanvasRect(bx + 34.0, by + 176.0, 132.0, 40.0, "#000000cc", radius=4.0),
                CanvasText(
                    bx + 100.0,
                    by + 196.0,
                    "PAUSED",
                    size=16.0,
                    color="#f9e2af",
                    bold=True,
                    align="center",
                ),
            ]
        )
    return commands


def _cell(commands: list, bx: float, by: float, row: int, col: int, fill: str) -> None:
    x = bx + col * CELL + 1.0
    y = by + row * CELL + 1.0
    commands.append(CanvasRect(x, y, CELL - 2.0, CELL - 2.0, fill, radius=2.0))
    commands.append(CanvasRect(x + 1.5, y + 1.5, CELL - 5.0, 2.0, "#ffffff44"))
