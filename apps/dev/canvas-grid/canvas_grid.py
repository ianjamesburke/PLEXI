#!/usr/bin/env python3
"""Canvas Grid — POC for stint 0397 canvas coordinate hit-testing.

Click a cell in the grid. The canvas uses fit="contain" so its declared
360x360 coordinate space is scaled and letterboxed inside the pane — this
app only works if the host correctly inverts that fit transform before
delivering MouseEvent, since the app never sees the pane's pixel size.
"""

from __future__ import annotations

from plexi_sdk import log, state
from plexi_sdk.effects import SetState, SetStatus, SetTitle
from plexi_sdk.events import MouseEvent
from plexi_sdk.ui import AppBar, Canvas, CanvasLine, CanvasRect, CanvasText, Column, FooterKeys

CANVAS_W = 360.0
CANVAS_H = 360.0
GRID_COLS = 6
GRID_ROWS = 6
CELL_W = CANVAS_W / GRID_COLS
CELL_H = CANVAS_H / GRID_ROWS


def init(_size, _args) -> list:
    return [
        SetTitle("Canvas Grid"),
        SetState({"selected": None, "clicks": 0}),
    ]


def update(event) -> list:
    if isinstance(event, MouseEvent) and event.pressed:
        col = int(event.x // CELL_W)
        row = int(event.y // CELL_H)
        if not (0 <= col < GRID_COLS and 0 <= row < GRID_ROWS):
            return []
        clicks = state.get("clicks", 0) + 1
        log.info(
            f"canvas-grid: cell ({col},{row}) from canvas-space "
            f"({event.x:.1f}, {event.y:.1f})"
        )
        return [
            SetState({"selected": [col, row], "clicks": clicks}),
            SetStatus(f"cell ({col}, {row}) · {clicks} clicks"),
        ]
    return []


def view():
    selected = state.get("selected")
    clicks = state.get("clicks", 0)
    return Column(
        [
            AppBar("Canvas Grid", f"{clicks} clicks"),
            Canvas(_draw(selected), width=CANVAS_W, height=CANVAS_H, grow=True, fit="contain"),
            FooterKeys([("click", "select cell")]),
        ],
        padding=0,
        gap=0,
        grow=True,
    )


def _draw(selected) -> list:
    commands: list = [CanvasRect(0, 0, CANVAS_W, CANVAS_H, "#11111b")]
    if selected is not None:
        col, row = selected
        commands.append(
            CanvasRect(col * CELL_W, row * CELL_H, CELL_W, CELL_H, "#89b4fa55")
        )
    for c in range(1, GRID_COLS):
        x = c * CELL_W
        commands.append(CanvasLine(x, 0.0, x, CANVAS_H, "#45475a"))
    for r in range(1, GRID_ROWS):
        y = r * CELL_H
        commands.append(CanvasLine(0.0, y, CANVAS_W, y, "#45475a"))
    if selected is not None:
        col, row = selected
        commands.append(
            CanvasText(
                col * CELL_W + CELL_W / 2,
                row * CELL_H + CELL_H / 2,
                f"{col},{row}",
                size=14.0,
                color="#cdd6f4",
                align="center",
            )
        )
    return commands
