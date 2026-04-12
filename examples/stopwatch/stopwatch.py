#!/usr/bin/env python3
from __future__ import annotations
"""
stopwatch — Plexi app
Precise stopwatch with lap recording.

Controls:
  Space   Start / Stop
  l       Record lap (while running)
  r       Reset (while stopped)
"""

import os
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from plexi_sdk import App

# ---------------------------------------------------------------------------
# Colors — Catppuccin Mocha
# ---------------------------------------------------------------------------

C = {
    "bg":      "#1e1e2e",
    "surface": "#313244",
    "text":    "#cdd6f4",
    "subtext": "#6c7086",
    "accent":  "#89b4fa",
    "green":   "#a6e3a1",
    "peach":   "#fab387",
    "red":     "#f38ba8",
    "header":  "#181825",
}

PADDING  = 16
MAX_LAPS = 10

# ---------------------------------------------------------------------------
# State
# ---------------------------------------------------------------------------

running:     bool  = False
start_time:  float = 0.0     # time.monotonic() when last started
elapsed:     float = 0.0     # accumulated seconds before the current run
laps:        list[float] = []  # absolute elapsed at each lap press (newest first)

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _current_elapsed() -> float:
    if running:
        return elapsed + (time.monotonic() - start_time)
    return elapsed


def _fmt(seconds: float) -> str:
    """Format seconds as MM:SS.mm"""
    total_ms = int(seconds * 100)
    mins = total_ms // 6000
    secs = (total_ms % 6000) // 100
    ms   = total_ms % 100
    return f"{mins:02d}:{secs:02d}.{ms:02d}"


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
    ctx.rect(0, 0, w, 40, fill=C["header"])
    ctx.text(PADDING, 12, "Stopwatch", size=13, color=C["accent"], bold=True)
    hint = "Space=start/stop  l=lap  r=reset"
    ctx.text(w - len(hint) * 7.2 - PADDING, 14, hint, size=11, color=C["subtext"])
    ctx.line(0, 40, w, 40, color=C["surface"], width=1.0)

    # Main timer — centered
    cur = _current_elapsed()
    time_str = _fmt(cur)
    # Large display: approx 9.5px per char at size=48
    char_w = 30.0
    tx = max(PADDING, (w - len(time_str) * char_w) / 2)
    ty = 80.0
    timer_color = C["green"] if running else C["text"]
    ctx.text(tx, ty, time_str, size=48, color=timer_color, monospace=True, bold=True)

    # Status label
    status = "RUNNING" if running else "STOPPED"
    status_color = C["green"] if running else C["peach"]
    sx = max(PADDING, (w - len(status) * 7.5) / 2)
    ctx.text(sx, ty + 64, status, size=12, color=status_color)

    # Lap count
    if laps:
        lc = f"{len(laps)} lap{'s' if len(laps) != 1 else ''}"
        ctx.text(w / 2 - len(lc) * 3.6, ty + 80, lc, size=11, color=C["subtext"])

    # Divider before laps
    divider_y = ty + 104
    ctx.line(PADDING, divider_y, w - PADDING, divider_y, color=C["surface"], width=1.0)

    # Lap list (newest first, max 10)
    lap_y = divider_y + 12
    lap_h = 24.0
    for i, lap_elapsed in enumerate(laps[:MAX_LAPS]):
        label_num = f"Lap {len(laps) - i}"
        label_time = _fmt(lap_elapsed)
        # lap delta
        if i < len(laps) - 1:
            delta = lap_elapsed - laps[i + 1]
        else:
            delta = lap_elapsed
        label_delta = f"+{_fmt(delta)}"

        row_y = lap_y + i * lap_h
        # alternating row bg
        if i % 2 == 0:
            ctx.rect(PADDING - 4, row_y - 2, w - PADDING * 2 + 8, lap_h, fill=C["surface"], radius=2.0)

        ctx.text(PADDING, row_y, label_num, size=12, color=C["subtext"])
        ctx.text(w / 2 - len(label_time) * 3.5, row_y, label_time, size=12, color=C["text"], monospace=True)
        ctx.text(w - PADDING - len(label_delta) * 7.0, row_y, label_delta, size=11, color=C["accent"], monospace=True)

    if not laps and not running:
        ctx.text(PADDING, lap_y, "No laps recorded.", size=12, color=C["subtext"])


@app.on_key
def on_key(key, _mods, _emit):
    global running, start_time, elapsed, laps

    if key == " ":
        if running:
            # Stop — accumulate elapsed
            elapsed += time.monotonic() - start_time
            running = False
        else:
            # Start
            start_time = time.monotonic()
            running = True

    elif key == "l" and running:
        laps.insert(0, _current_elapsed())
        if len(laps) > MAX_LAPS:
            laps = laps[:MAX_LAPS]

    elif key == "r" and not running:
        elapsed = 0.0
        laps = []


app.run()
