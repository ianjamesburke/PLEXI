#!/usr/bin/env python3
"""Calculator — L1 component-based rendering.

Demonstrates: ButtonRow click detection, AppBar, Heading, FooterKeys.
"""
import os
import sys
sys.path.insert(0, os.path.join(os.path.dirname(__file__), '../../sdk/python'))

from plexi_sdk import App, RenderContext
from plexi_sdk.ui import (
    Column, Card, AppBar, Heading, FooterKeys, ButtonRow, Spacer, Component,
    SPACE_SM, SPACE_MD,
)

# Grid geometry — fixed per-button size; gap matches Card inner gap default.
BTN_W = 64.0
BTN_H = 48.0
BTN_GAP = 8.0
COLS = 4

# Button layout: (label, col, row)  — same as original
BUTTONS = [
    ("C",  0, 0), ("±",  1, 0), ("%",  2, 0), ("÷",  3, 0),
    ("7",  0, 1), ("8",  1, 1), ("9",  2, 1), ("×",  3, 1),
    ("4",  0, 2), ("5",  1, 2), ("6",  2, 2), ("−",  3, 2),
    ("1",  0, 3), ("2",  1, 3), ("3",  2, 3), ("+",  3, 3),
    ("0",  0, 4), (".",  2, 4), ("=",  3, 4),
]

ROWS = 5  # 0-indexed rows 0..4


def _is_op(label: str) -> bool:
    return label in ("÷", "×", "+", "−", "=")


def _is_clear(label: str) -> bool:
    return label == "C"


class ButtonGrid(Component):
    """Renders pre-created ButtonRow widgets in a fixed 4-column calculator grid.

    Stateful ButtonRow instances are created once in on_init and passed in here.
    Each render call positions them according to the BUTTONS layout and delegates
    to ButtonRow.render() so click state is captured correctly.
    """

    def __init__(self, buttons: dict[str, ButtonRow]) -> None:
        # buttons: label -> ButtonRow
        self._buttons = buttons

    def measure(self, _avail_w: float) -> float:
        return ROWS * BTN_H + (ROWS - 1) * BTN_GAP

    def render(self, ctx, x: float, y: float, _w: float, _h: float) -> None:
        # "0" spans columns 0-1; everything else is single-column.
        for label, col, row in BUTTONS:
            spans = 2 if (label == "0") else 1
            bx = x + col * (BTN_W + BTN_GAP)
            by = y + row * (BTN_H + BTN_GAP)
            bw = BTN_W * spans + BTN_GAP * (spans - 1)
            btn = self._buttons[label]
            btn.render(ctx, bx, by, bw, BTN_H)


class CalcApp(App):
    def on_init(self, ctx: RenderContext) -> None:
        self.display = "0"
        self.pending_op: str | None = None
        self.pending_val: float | None = None
        self.fresh = True  # next digit starts a new number

        ctx.emit.set_mouse_tracking(True)

        # Create all ButtonRow widgets once; stable across renders.
        self._btns: dict[str, ButtonRow] = {}
        for label, _, _ in BUTTONS:
            if label in self._btns:
                continue  # "." only appears once, but guard for safety
            if _is_op(label):
                fill = ctx.theme.accent
                hover_fill = ctx.theme.highlight
                text_color = ctx.theme.bg
            elif _is_clear(label):
                fill = ctx.theme.danger
                hover_fill = ctx.theme.surface
                text_color = ctx.theme.bg
            else:
                fill = ctx.theme.surface
                hover_fill = ctx.theme.highlight
                text_color = ctx.theme.fg

            self._btns[label] = ButtonRow(
                id=f"btn_{label}",
                label=label,
                fill=fill,
                hover_fill=hover_fill,
                text_color=text_color,
                radius=6.0,
                height=BTN_H,
            )

        self._grid = ButtonGrid(self._btns)
        ctx.info("CalcApp ready (L1)")

    def _press(self, label: str) -> None:
        if label.isdigit():
            if self.fresh:
                self.display = label
                self.fresh = False
            else:
                self.display = (self.display + label).lstrip("0") or "0"
        elif label == ".":
            if self.fresh:
                self.display = "0."
                self.fresh = False
            elif "." not in self.display:
                self.display += "."
        elif label == "C":
            self.display = "0"
            self.pending_op = None
            self.pending_val = None
            self.fresh = True
        elif label == "±":
            val = float(self.display)
            self.display = str(-val) if val != 0 else "0"
        elif label == "%":
            self.display = str(float(self.display) / 100)
            self.fresh = True
        elif label in ("÷", "×", "+", "−"):
            self.pending_val = float(self.display)
            self.pending_op = label
            self.fresh = True
        elif label == "=":
            if self.pending_op and self.pending_val is not None:
                cur = float(self.display)
                op = self.pending_op
                if op == "+":
                    result = self.pending_val + cur
                elif op == "−":
                    result = self.pending_val - cur
                elif op == "×":
                    result = self.pending_val * cur
                elif op == "÷":
                    result = self.pending_val / cur if cur != 0 else float("inf")
                else:
                    result = cur
                # Trim unnecessary decimal
                self.display = (
                    str(int(result))
                    if result == int(result) and abs(result) < 1e15
                    else str(result)
                )
                self.pending_op = None
                self.pending_val = None
                self.fresh = True

    def on_render(self, ctx: RenderContext) -> None:
        subtitle = f"op: {self.pending_op}" if self.pending_op else None
        ctx.render(Column([
            AppBar("Calculator", subtitle=subtitle),
            Heading(self.display, level=1),
            Card([self._grid], padding=SPACE_MD, gap=BTN_GAP),
            Spacer(grow=True),
            FooterKeys([
                ("0-9", "digit"),
                ("+ - * /", "operator"),
                ("Enter", "equals"),
                ("Backspace", "delete"),
                ("Esc", "clear"),
            ]),
        ], padding=SPACE_MD, gap=SPACE_SM))

        # Check button clicks after render
        for label, _, _ in BUTTONS:
            if self._btns[label].clicked:
                self._press(label)

    def _backspace(self) -> None:
        if self.fresh or self.display == "0":
            return
        self.display = self.display[:-1] or "0"

    def on_key(self, ctx: RenderContext, key: str, _mods: dict) -> None:
        if key in "0123456789":
            self._press(key)
        elif key == ".":
            self._press(".")
        elif key == "+":
            self._press("+")
        elif key == "-":
            self._press("−")
        elif key == "*":
            self._press("×")
        elif key == "/":
            self._press("÷")
        elif key in ("=", "return", "enter"):
            self._press("=")
        elif key == "escape":
            self._press("C")
        elif key == "backspace":
            self._backspace()
        else:
            ctx.debug(f"calc: unhandled key {key!r}")


CalcApp().run()
