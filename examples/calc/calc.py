#!/usr/bin/env python3
from __future__ import annotations
"""
calc — Plexi app
Calculator with expression evaluation and visual keypad reference.

Controls:
  0-9 . + - * /   Build expression
  Enter            Evaluate
  Backspace        Delete last character
  Escape / c       Clear
"""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from plexi_sdk import App

# ---------------------------------------------------------------------------
# Colors — Catppuccin Mocha
# ---------------------------------------------------------------------------

C = {
    "bg":      "#1e1e2e",
    "surface": "#313244",
    "surface1":"#45475a",
    "text":    "#cdd6f4",
    "subtext": "#6c7086",
    "accent":  "#89b4fa",
    "green":   "#a6e3a1",
    "peach":   "#fab387",
    "red":     "#f38ba8",
    "header":  "#181825",
}

PADDING  = 16
HEADER_H = 40

# ---------------------------------------------------------------------------
# State
# ---------------------------------------------------------------------------

expression: str = ""
result:     str = ""
is_error:   bool = False

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

_ALLOWED_CHARS = set("0123456789.+-*/() ")


def _evaluate(expr: str) -> tuple[str, bool]:
    """Evaluate expression safely. Returns (result_str, is_error)."""
    expr = expr.strip()
    if not expr:
        return ("", False)
    # Only allow safe characters
    for ch in expr:
        if ch not in _ALLOWED_CHARS:
            return (f"Error: invalid character '{ch}'", True)
    try:
        val = eval(expr, {"__builtins__": {}})  # noqa: S307
        # Format: remove trailing .0 for integers
        if isinstance(val, float) and val == int(val):
            return (str(int(val)), False)
        return (str(val), False)
    except ZeroDivisionError:
        return ("Error: division by zero", True)
    except SyntaxError:
        return ("Error: invalid expression", True)
    except Exception as e:
        return (f"Error: {e}", True)

# ---------------------------------------------------------------------------
# Keypad layout (visual reference only)
# ---------------------------------------------------------------------------

KEYPAD_ROWS = [
    ["7", "8", "9", "/"],
    ["4", "5", "6", "*"],
    ["1", "2", "3", "-"],
    ["0", ".", "=", "+"],
]

# ---------------------------------------------------------------------------
# App
# ---------------------------------------------------------------------------

app = App()


@app.on_render
def render(ctx):
    w = ctx.width
    h = ctx.height

    ctx.rect(0, 0, w, h, fill=C["bg"])

    # Header
    ctx.rect(0, 0, w, HEADER_H, fill=C["header"])
    ctx.text(PADDING, 12, "Calculator", size=13, color=C["accent"], bold=True)
    hint = "0-9 +-*/()  Enter=eval  Backspace=del  Esc/c=clear"
    ctx.text(w - len(hint) * 7.2 - PADDING, 14, hint, size=10, color=C["subtext"])
    ctx.line(0, HEADER_H, w, HEADER_H, color=C["surface"], width=1.0)

    # Expression display
    expr_y = HEADER_H + PADDING
    ctx.rect(PADDING, expr_y, w - PADDING * 2, 40, fill=C["surface"], radius=6.0)
    display_expr = expression if expression else "0"
    # right-align expression text
    expr_char_w = 10.0
    max_expr_chars = int((w - PADDING * 2 - 16) / expr_char_w)
    visible_expr = display_expr[-max_expr_chars:] if len(display_expr) > max_expr_chars else display_expr
    ctx.text(PADDING + 8, expr_y + 12, visible_expr, size=18, color=C["text"], monospace=True)

    # Result display
    result_y = expr_y + 52
    if result:
        res_color = C["peach"] if is_error else C["green"]
        # Large result
        rsize = 28 if not is_error else 14
        rx = PADDING + 8
        ctx.text(rx, result_y, result, size=rsize, color=res_color, monospace=True, bold=not is_error)
    else:
        ctx.text(PADDING + 8, result_y, "—", size=22, color=C["subtext"], monospace=True)

    # Divider before keypad
    kpad_top = result_y + 52
    ctx.line(PADDING, kpad_top, w - PADDING, kpad_top, color=C["surface"], width=1.0)

    # Keypad grid (visual reference)
    kpad_y = kpad_top + 12
    key_w = 56.0
    key_h = 44.0
    key_gap = 8.0
    total_kpad_w = 4 * key_w + 3 * key_gap
    kpad_x = (w - total_kpad_w) / 2

    for row_i, row in enumerate(KEYPAD_ROWS):
        for col_i, label in enumerate(row):
            kx = kpad_x + col_i * (key_w + key_gap)
            ky = kpad_y + row_i * (key_h + key_gap)
            # Different bg for operators
            is_op = label in ("/ * - + =".split())
            bg = C["accent"] if label == "=" else (C["surface1"] if is_op else C["surface"])
            txt = C["header"] if label == "=" else (C["accent"] if is_op else C["text"])
            ctx.rect(kx, ky, key_w, key_h, fill=bg, radius=6.0)
            lbl = "=" if label == "=" else label
            lx = kx + key_w / 2 - len(lbl) * 5.5
            ly = ky + key_h / 2 - 7
            ctx.text(lx, ly, lbl, size=14, color=txt, bold=True)

    # Bottom label
    ctx.text(
        w / 2 - 60, kpad_y + 4 * (key_h + key_gap) + 4,
        "visual reference only",
        size=10, color=C["subtext"],
    )


@app.on_key
def on_key(key, _mods, _emit):
    global expression, result, is_error

    if key in ("Escape", "c"):
        expression = ""
        result = ""
        is_error = False
    elif key == "Backspace":
        expression = expression[:-1]
        result = ""
        is_error = False
    elif key == "Enter":
        result, is_error = _evaluate(expression)
        if not is_error and result:
            # Keep result in expression so user can keep chaining
            expression = result
            result = ""
    elif len(key) == 1 and key in _ALLOWED_CHARS and key != " ":
        expression += key
        result = ""
        is_error = False


app.run()
