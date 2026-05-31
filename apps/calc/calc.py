#!/usr/bin/env python3
"""Calculator — reference implementation for ctx.button() (#255).

Demonstrates: ctx.button() hover + click detection, keyboard input handling.
"""
import os
import sys
sys.path.insert(0, os.path.join(os.path.dirname(__file__), '../../sdk/python'))

from plexi_sdk import App, RenderContext, PAD
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
        # Display
        ctx.rect(PAD, PAD, ctx.w - PAD * 2, 80.0, fill=ctx.theme.surface, radius=8.0)
        ctx.text(ctx.w - PAD - 8, PAD + 16, self.display,
                 size=32.0, color=ctx.theme.fg, align="right")
        if self.pending_op:
            ctx.text(PAD + 8, PAD + 8, self.pending_op, size=12.0, color=ctx.theme.muted)

        # Buttons
        for label, col, row in BUTTONS:
            x, y, w, h = _btn_rect(col, row)
            is_op = label in ("÷", "×", "+", "−", "=")
            is_clear = label == "C"
            fill = ctx.theme.accent if is_op else (ctx.theme.danger if is_clear else ctx.theme.surface)
            hover_fill = "#b9d0f7" if is_op else ("#f5a3b5" if is_clear else "#45475a")
            text_color = ctx.theme.bg if is_op else (ctx.theme.bg if is_clear else ctx.theme.fg)
            if ctx.button(label, x, y, w, h, label,
                          fill=fill, hover_fill=hover_fill,
                          text_color=text_color, radius=6.0):
                self._press(label)

        # Keyboard hint
        ctx.text(PAD, ctx.h - 24, "0-9  + - * /  Enter: equals  Backspace: delete  Esc: clear",
                 size=10.0, color=ctx.theme.muted)

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
