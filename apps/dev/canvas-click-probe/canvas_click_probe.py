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

It also records pointer-drag trajectories (stint 0510): a press with a
button starts a record, button-less moves extend it, and a button release
completes it — painted back as a `drag:` `CanvasText` line so drag-injection
tests can assert press/move-count/release in canvas space.
"""

from __future__ import annotations

from plexi_sdk import state
from plexi_sdk.effects import SetState, SetStatus, SetTitle
from plexi_sdk.events import MouseEvent
from plexi_sdk.ui import Canvas, CanvasRect, CanvasText

CANVAS_W = 360.0
CANVAS_H = 440.0


def init(_size, _args) -> list:
    return [SetTitle("Canvas Click Probe"), SetState({"last_click": None, "drag": None})]


def update(event) -> list:
    if not isinstance(event, MouseEvent):
        return []
    if event.pressed and event.button is not None:
        return [
            SetState({
                "last_click": [event.x, event.y],
                "drag": {"press": [event.x, event.y], "moves": 0, "release": None},
            }),
            SetStatus(f"click {event.x:.2f},{event.y:.2f}"),
        ]
    drag = state.get("drag")
    if drag is None or drag.get("release") is not None:
        return []
    if event.button is None:
        moved = dict(drag, moves=drag["moves"] + 1)
        return [SetState({"drag": moved})]
    done = dict(drag, release=[event.x, event.y])
    return [
        SetState({"drag": done}),
        SetStatus(f"drag release {event.x:.2f},{event.y:.2f}"),
    ]


def view():
    last_click = state.get("last_click")
    drag = state.get("drag")
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
    if drag is not None and drag.get("release") is not None:
        px, py = drag["press"]
        rx, ry = drag["release"]
        commands.append(
            CanvasText(
                CANVAS_W / 2.0,
                CANVAS_H - 20.0,
                f"drag:press:{px:.2f},{py:.2f} moves:{drag['moves']} "
                f"release:{rx:.2f},{ry:.2f}",
                size=12.0,
                color="#a6e3a1",
                align="center",
            )
        )
    return Canvas(commands, width=CANVAS_W, height=CANVAS_H, grow=True, fit="contain")
