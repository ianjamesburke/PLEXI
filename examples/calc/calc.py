#!/usr/bin/env python3
"""Calculator — reference implementation for ctx.button() (#255).

Demonstrates: ctx.button() hover + click detection, KeyMap shortcuts.
"""
import os
import sys
sys.path.insert(0, os.path.join(os.path.dirname(__file__), '../../sdk/python'))

from plexi_sdk import App, RenderContext
from plexi_sdk.widgets.keymap import KeyMap

BG = "#1e1e2e"
SURFACE = "#313244"
FG = "#cdd6f4"
ACCENT = "#89b4fa"
RED = "#f38ba8"
MUTED = "#6c7086"

PAD = 16.0
BTN_W = 64.0
BTN_H = 48.0
BTN_GAP = 8.0

# Button layout: (label, col, row)
BUTTONS = [
    ("C",  0, 0), ("±",  1, 0), ("%",  2, 0), ("÷",  3, 0),
    ("7",  0, 1), ("8",  1, 1), ("9",  2, 1), ("×",  3, 1),
    ("4",  0, 2), ("5",  1, 2), ("6",  2, 2), ("−",  3, 2),
    ("1",  0, 3), ("2",  1, 3), ("3",  2, 3), ("+",  3, 3),
    ("0",  0, 4), (".",  2, 4), ("=",  3, 4),
]

def _btn_rect(col: int, row: int) -> tuple[float, float, float, float]:
    x = PAD + col * (BTN_W + BTN_GAP)
    y = 120.0 + row * (BTN_H + BTN_GAP)
    w = BTN_W * 2 + BTN_GAP if col == 0 and row == 4 else BTN_W  # 0 spans 2 cols
    return x, y, w, BTN_H


class CalcApp(App):
    def on_init(self, ctx: RenderContext) -> None:
        self.display = "0"
        self.pending_op: str | None = None
        self.pending_val: float | None = None
        self.fresh = True  # next digit starts a new number
        self._km = KeyMap()
        self._km.bind("return", "equals")
        self._km.bind("enter", "equals")
        self._km.bind("escape", "clear")
        self._km.bind("*", "multiply")
        self._km.bind("/", "divide")
        self._km.bind("-", "minus")
        ctx.emit.set_mouse_tracking(True)
        ctx.info("CalcApp ready")

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
                self.display = str(int(result)) if result == int(result) and abs(result) < 1e15 else str(result)
                self.pending_op = None
                self.pending_val = None
                self.fresh = True

    def on_render(self, ctx: RenderContext) -> None:
        ctx.clear(BG)

        # Display
        ctx.rect(PAD, PAD, ctx.w - PAD * 2, 80.0, fill=SURFACE, radius=8.0)
        ctx.text(ctx.w - PAD - 8, PAD + 16, self.display,
                 size=32.0, color=FG, align="right")
        if self.pending_op:
            ctx.text(PAD + 8, PAD + 8, self.pending_op, size=12.0, color=MUTED)

        # Buttons
        for label, col, row in BUTTONS:
            x, y, w, h = _btn_rect(col, row)
            is_op = label in ("÷", "×", "+", "−", "=")
            is_clear = label == "C"
            fill = ACCENT if is_op else (RED if is_clear else SURFACE)
            hover_fill = "#b9d0f7" if is_op else ("#f5a3b5" if is_clear else "#45475a")
            text_color = BG if is_op else (BG if is_clear else FG)
            if ctx.button(label, x, y, w, h, label,
                          fill=fill, hover_fill=hover_fill,
                          text_color=text_color, radius=6.0):
                self._press(label)

        # Keyboard hint
        ctx.text(PAD, ctx.h - 24, "keyboard: 0-9  + - * /  Enter=  Escape=C",
                 size=10.0, color=MUTED)

    def on_key(self, ctx: RenderContext, key: str, mods: dict) -> None:
        _ACTION_TO_LABEL = {
            "equals": "=", "clear": "C",
            "multiply": "×", "divide": "÷", "minus": "−",
        }
        action = self._km.handle(key, mods)
        label = _ACTION_TO_LABEL.get(action, key) if action else key
        valid = set("0123456789.±%÷×+−=C")
        if label in valid:
            self._press(label)
        else:
            ctx.debug(f"calc: unhandled key {key!r}")


CalcApp().run()
