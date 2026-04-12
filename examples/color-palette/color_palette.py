#!/usr/bin/env python3
from __future__ import annotations
"""
color-palette — Plexi app
Catppuccin Mocha color reference. Navigate the grid, press Enter/Space to copy hex.

Controls:
  j / ↓   Move down
  k / ↑   Move up
  h / ←   Move left
  l / →   Move right
  Enter   Copy hex value to clipboard (macOS pbcopy)
  Space   Copy hex value to clipboard (macOS pbcopy)
"""

import os
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from plexi_sdk import App

# ---------------------------------------------------------------------------
# Catppuccin Mocha palette
# ---------------------------------------------------------------------------

PALETTE: list[tuple[str, str]] = [
    ("Rosewater", "#f5e0dc"),
    ("Flamingo",  "#f2cdcd"),
    ("Pink",      "#f5c2e7"),
    ("Mauve",     "#cba6f7"),
    ("Red",       "#f38ba8"),
    ("Maroon",    "#eba0ac"),
    ("Peach",     "#fab387"),
    ("Yellow",    "#f9e2af"),
    ("Green",     "#a6e3a1"),
    ("Teal",      "#94e2d5"),
    ("Sky",       "#89dceb"),
    ("Sapphire",  "#74c7ec"),
    ("Blue",      "#89b4fa"),
    ("Lavender",  "#b4befe"),
    ("Text",      "#cdd6f4"),
    ("Subtext1",  "#bac2de"),
    ("Subtext0",  "#a6adc8"),
    ("Overlay2",  "#7f849c"),
    ("Overlay1",  "#6c7086"),
    ("Overlay0",  "#585b70"),
    ("Surface2",  "#45475a"),
    ("Surface1",  "#313244"),
    ("Surface0",  "#1e1e2e"),
    ("Base",      "#1e1e2e"),
    ("Mantle",    "#181825"),
    ("Crust",     "#11111b"),
]

# ---------------------------------------------------------------------------
# Colors (UI chrome)
# ---------------------------------------------------------------------------

C = {
    "bg":      "#1e1e2e",
    "surface": "#313244",
    "text":    "#cdd6f4",
    "subtext": "#6c7086",
    "accent":  "#89b4fa",
    "green":   "#a6e3a1",
    "header":  "#181825",
}

HEADER_H   = 40
PADDING    = 16
SWATCH_W   = 80
SWATCH_H   = 56
GAP        = 12
COLS       = 4    # default; recalculated in render

# ---------------------------------------------------------------------------
# State
# ---------------------------------------------------------------------------

selected:     int   = 0
copied_at:    float = 0.0
copied_hex:   str   = ""

# ---------------------------------------------------------------------------
# App
# ---------------------------------------------------------------------------

app = App()


def _copy_to_clipboard(text: str):
    try:
        subprocess.run(["pbcopy"], input=text.encode(), check=True, timeout=2)
    except Exception:
        pass  # silently skip on non-macOS


@app.on_render
def render(ctx):
    global selected

    w = ctx.width
    h = ctx.height

    ctx.rect(0, 0, w, h, fill=C["bg"])

    # Header
    ctx.rect(0, 0, w, HEADER_H, fill=C["header"])
    ctx.text(PADDING, 12, "Color Palette — Catppuccin Mocha", size=13, color=C["accent"], bold=True)
    hint = "hjkl/arrows=navigate  Enter=copy hex"
    ctx.text(w - len(hint) * 7.2 - PADDING, 14, hint, size=11, color=C["subtext"])
    ctx.line(0, HEADER_H, w, HEADER_H, color=C["surface"], width=1.0)

    # Compute grid dimensions
    available_w = w - PADDING * 2
    cols = max(1, int((available_w + GAP) / (SWATCH_W + GAP)))
    rows = (len(PALETTE) + cols - 1) // cols

    grid_x0 = PADDING
    grid_y0 = HEADER_H + PADDING

    for i, (name, hex_val) in enumerate(PALETTE):
        col = i % cols
        row = i // cols
        sx = grid_x0 + col * (SWATCH_W + GAP)
        sy = grid_y0 + row * (SWATCH_H + GAP + 28)  # 28 extra for two text lines

        is_sel = (i == selected)

        # Selection outline
        if is_sel:
            ctx.rect(sx - 3, sy - 3, SWATCH_W + 6, SWATCH_H + 6,
                     fill=C["accent"], radius=6.0)

        # Swatch
        ctx.rect(sx, sy, SWATCH_W, SWATCH_H, fill=hex_val, radius=4.0)

        # Name
        ctx.text(sx, sy + SWATCH_H + 4, name, size=11, color=C["text"])
        # Hex
        ctx.text(sx, sy + SWATCH_H + 18, hex_val, size=10, color=C["subtext"])

    # "Copied!" flash
    now = time.monotonic()
    if copied_hex and (now - copied_at) < 1.0:
        msg = f"Copied {copied_hex}!"
        msg_w = len(msg) * 7.5 + 16
        ctx.rect(w / 2 - msg_w / 2, h - 36, msg_w, 24, fill=C["surface"], radius=4.0)
        ctx.text(w / 2 - len(msg) * 3.75, h - 28, msg, size=12, color=C["green"])


@app.on_key
def on_key(key, _mods, _emit):
    global selected, copied_at, copied_hex

    # Recalculate cols based on last known width (stored on app object)
    w = app.width
    available_w = w - PADDING * 2
    cols = max(1, int((available_w + GAP) / (SWATCH_W + GAP)))
    n = len(PALETTE)

    if key in ("j", "ArrowDown"):
        selected = min(selected + cols, n - 1)
    elif key in ("k", "ArrowUp"):
        selected = max(selected - cols, 0)
    elif key in ("h", "ArrowLeft"):
        selected = max(selected - 1, 0)
    elif key in ("l", "ArrowRight"):
        selected = min(selected + 1, n - 1)
    elif key in ("Enter", " "):
        _, hex_val = PALETTE[selected]
        _copy_to_clipboard(hex_val)
        copied_hex = hex_val
        copied_at = time.monotonic()


app.run()
