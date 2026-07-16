#!/usr/bin/env python3
"""Canvas Sidebar -- POC for Canvas as a grow child of a horizontal HStack.

Demonstrates stint 0308: a growing Canvas takes the remaining width beside a
fixed-width `Sized` sidebar. The Canvas draws an NxN grid filling its allocated
rect using rect-relative coordinates, proving the host width-partitions the
horizontal Stack correctly.
"""

from __future__ import annotations

import plexi_sdk as sdk
from plexi_sdk import log, theme
from plexi_sdk.effects import SetStatus, SetTitle
from plexi_sdk.events import KeyEvent, Resize, UiAction
from plexi_sdk.ui import (
    Button,
    Canvas,
    CanvasLine,
    CanvasRect,
    Column,
    Divider,
    HStack,
    Text,
)

SIDEBAR_W = 160.0
GRID = 8

_n = GRID


def init(size, _args) -> list:
    global _n
    _n = GRID
    sdk.canvas_width, sdk.canvas_height = size
    log.info("canvas-sidebar: initialized HStack[Canvas grow + Sized sidebar]")
    return [SetTitle("Canvas Sidebar"), SetStatus(f"grid {_n}x{_n}")]


def update(event) -> list:
    global _n
    if isinstance(event, Resize):
        sdk.canvas_width = event.width
        sdk.canvas_height = event.height
        return []
    if isinstance(event, UiAction):
        if event.handler_id == "denser":
            _n = min(24, _n + 1)
            return [SetStatus(f"grid {_n}x{_n}")]
        if event.handler_id == "sparser":
            _n = max(2, _n - 1)
            return [SetStatus(f"grid {_n}x{_n}")]
        return []
    if isinstance(event, KeyEvent):
        if event.key in ("=", "+"):
            _n = min(24, _n + 1)
            return [SetStatus(f"grid {_n}x{_n}")]
        if event.key == "-":
            _n = max(2, _n - 1)
            return [SetStatus(f"grid {_n}x{_n}")]
    return []


def _grid_commands(w, h) -> list:
    cmds: list = [CanvasRect(0, 0, w, h, "#0d0d1a")]
    cell_w = w / _n
    cell_h = h / _n
    # Alternating filled cells (rect-relative coords fill the allocated rect).
    for row in range(_n):
        for col in range(_n):
            if (row + col) % 2 == 0:
                cmds.append(
                    CanvasRect(col * cell_w, row * cell_h, cell_w, cell_h, "#1e1e3a")
                )
    # Grid lines.
    for i in range(_n + 1):
        x = i * cell_w
        y = i * cell_h
        cmds.append(CanvasLine(x, 0, x, h, "#45475a", 1.0))
        cmds.append(CanvasLine(0, y, w, y, "#45475a", 1.0))
    return cmds


def view():
    # Sidebar renders first so egui's horizontal layout reserves its space
    # before the grow canvas claims the remainder (no fixed-width wrapper
    # node exists on the live CPython-WASM decode path — see stint 0394).
    sidebar = Column(
        [
            Text(text="SIDEBAR", size=10.0, color=theme.muted, bold=True),
            Divider(),
            Text(text=f"Grid: {_n} x {_n}"),
            Button(label="Denser (+)", on_click="denser"),
            Button(label="Sparser (-)", on_click="sparser"),
        ],
        padding=12,
        gap=8,
    )
    w = max(1.0, float(sdk.canvas_width or 480.0) - SIDEBAR_W - 12.0)
    h = max(1.0, float(sdk.canvas_height or 360.0))
    canvas = Canvas(_grid_commands(w, h), width=w, height=h, grow=True)
    return HStack([sidebar, canvas], gap=12)
