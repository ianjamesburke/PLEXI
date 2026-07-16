#!/usr/bin/env python3
"""Canvas Click Probe — HostHarness click-injection e2e fixture (stint 0398).

`view()` returns a bare `Canvas` as the tree root, with no `AppBar`/`Column`/
`FooterKeys` chrome around it. That makes the canvas widget's on-screen rect
exactly equal the pane's own rect, so a test injecting a click at a known
PANE-PIXEL coordinate can predict the resulting canvas-space coordinate by
hand from the declared 360x440 `fit="contain"` transform — the same
declared size and fit mode stint 0397's `wasm_render.rs` unit test uses, so
this is the end-to-end counterpart of that test, not a new scenario.

On a `MouseEvent`, the last click's canvas-space coordinates are painted
back as `CanvasText` so a test can assert on them via the pane's semantic
`canvas_commands` (the host never exposes raw process state cross-process).
"""

from __future__ import annotations

from plexi_sdk import state
from plexi_sdk.effects import SetState, SetStatus, SetTitle
from plexi_sdk.events import MouseEvent
from plexi_sdk.ui import Canvas, CanvasRect, CanvasText

CANVAS_W = 360.0
CANVAS_H = 440.0


def init(_size, _args) -> list:
    return [SetTitle("Canvas Click Probe"), SetState({"last_click": None})]


def update(event) -> list:
    if isinstance(event, MouseEvent) and event.pressed:
        return [
            SetState({"last_click": [event.x, event.y]}),
            SetStatus(f"click {event.x:.2f},{event.y:.2f}"),
        ]
    return []


def view():
    last_click = state.get("last_click")
    commands: list = [CanvasRect(0, 0, CANVAS_W, CANVAS_H, "#11111b")]
    if last_click is not None:
        x, y = last_click
        commands.append(
            CanvasText(
                x,
                y,
                f"click:{x:.2f},{y:.2f}",
                size=14.0,
                color="#f38ba8",
                align="center",
            )
        )
    return Canvas(commands, width=CANVAS_W, height=CANVAS_H, grow=True, fit="contain")
