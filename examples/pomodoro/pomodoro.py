#!/usr/bin/env python3
"""
pomodoro — Plexi app
Focus timer using the Pomodoro Technique.

Controls:
  Space    Start / Pause
  r        Reset current timer
  b        Short break (5 min)
  l        Long break (15 min)
"""
from __future__ import annotations

import math
import os
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from plexi_sdk import App

# ---------------------------------------------------------------------------
# Catppuccin Mocha
# ---------------------------------------------------------------------------

C = {
    "bg":        "#1e1e2e",
    "surface":   "#313244",
    "overlay":   "#45475a",
    "text":      "#cdd6f4",
    "subtext":   "#6c7086",
    "accent":    "#89b4fa",
    "green":     "#a6e3a1",
    "yellow":    "#f9e2af",
    "red":       "#f38ba8",
    "mauve":     "#cba6f7",
    "peach":     "#fab387",
    "header":    "#181825",
    "flash_bg":  "#45475a",
}

# ---------------------------------------------------------------------------
# Timer state
# ---------------------------------------------------------------------------

STATE_IDLE  = "idle"
STATE_FOCUS = "focus"
STATE_SHORT = "short_break"
STATE_LONG  = "long_break"

DURATIONS = {
    STATE_IDLE:  25 * 60,
    STATE_FOCUS: 25 * 60,
    STATE_SHORT:  5 * 60,
    STATE_LONG:  15 * 60,
}

STATE_LABELS = {
    STATE_IDLE:  "Ready to focus",
    STATE_FOCUS: "Focus",
    STATE_SHORT: "Short Break",
    STATE_LONG:  "Long Break",
}

STATE_COLORS = {
    STATE_IDLE:  C["subtext"],
    STATE_FOCUS: C["red"],
    STATE_SHORT: C["green"],
    STATE_LONG:  C["accent"],
}

timer_state: str   = STATE_IDLE
running: bool      = False
remaining: float   = float(DURATIONS[STATE_IDLE])
last_tick: float   = 0.0
sessions: int      = 0   # completed focus pomodoros

flash_frames: int  = 0   # remaining frames to flash background

PADDING  = 20
HEADER_H = 44
ARC_SEGS = 60
ARC_R    = 90.0   # radius of progress ring (logical px)
ARC_W    = 10.0   # segment "width" in degrees

# ---------------------------------------------------------------------------
# App
# ---------------------------------------------------------------------------

app = App(app_id="pomodoro")


@app.on_render
def render(ctx):
    global remaining, last_tick, flash_frames, timer_state, running, sessions

    now = time.monotonic()

    # Tick the timer
    if running and last_tick > 0:
        delta = now - last_tick
        remaining -= delta
        if remaining <= 0:
            remaining = 0.0
            running = False
            flash_frames = 3
            _on_complete(ctx)
    last_tick = now if running else 0.0

    w = ctx.width
    h = ctx.height

    # Flash background on completion
    bg = C["flash_bg"] if flash_frames > 0 else C["bg"]
    if flash_frames > 0:
        flash_frames -= 1
    ctx.rect(0, 0, w, h, fill=bg)

    # Header
    ctx.rect(0, 0, w, HEADER_H, fill=C["header"])
    ctx.text(PADDING, 14, "Pomodoro", size=14, color=C["red"], bold=True)
    hint = "Space=start/pause  r=reset  b=break  l=long break"
    ctx.text(w - len(hint) * 6.8 - PADDING, 16, hint, size=10, color=C["subtext"])
    ctx.line(0, HEADER_H, w, HEADER_H, color=C["surface"], width=1.0)

    cx = w / 2
    cy = (h + HEADER_H) / 2 - 20

    total = float(DURATIONS.get(timer_state, DURATIONS[STATE_FOCUS]))
    progress = 1.0 - (remaining / total) if total > 0 else 0.0

    # Progress ring — 60 rect segments around a circle
    filled = int(progress * ARC_SEGS)
    seg_deg = 360.0 / ARC_SEGS
    ring_color = STATE_COLORS.get(timer_state, C["accent"])
    for i in range(ARC_SEGS):
        angle_rad = math.radians(i * seg_deg - 90)
        sx = cx + math.cos(angle_rad) * ARC_R
        sy = cy + math.sin(angle_rad) * ARC_R
        color = ring_color if i < filled else C["surface"]
        ctx.rect(sx - 4, sy - 4, 8, 8, fill=color, radius=2.0)

    # MM:SS countdown
    mins = int(remaining) // 60
    secs = int(remaining) % 60
    time_str = f"{mins:02d}:{secs:02d}"
    # Center the text (approximate: each char ~14px wide at size 40)
    tw = len(time_str) * 24
    ctx.text(cx - tw / 2, cy - 22, time_str, size=40, color=C["text"], bold=True, monospace=True)

    # State label
    label = STATE_LABELS.get(timer_state, "")
    lw = len(label) * 8
    lcolor = STATE_COLORS.get(timer_state, C["subtext"])
    ctx.text(cx - lw / 2, cy + 26, label, size=14, color=lcolor)

    # Paused / running indicator
    status = "▶ Running" if running else ("⏸ Paused" if timer_state != STATE_IDLE else "Press Space to start")
    sw = len(status) * 7
    ctx.text(cx - sw / 2, cy + 50, status, size=12, color=C["subtext"])

    # Session dots
    dot_y = cy + 80
    dot_spacing = 20.0
    total_dots = max(sessions + 1, 4)
    dots_w = total_dots * dot_spacing
    dot_start = cx - dots_w / 2
    for i in range(total_dots):
        dx = dot_start + i * dot_spacing
        color = C["red"] if i < sessions else C["overlay"]
        ctx.rect(dx, dot_y, 10, 10, fill=color, radius=5.0)

    ctx.text(cx - 30, dot_y + 18, f"{sessions} sessions", size=11, color=C["subtext"])


def _on_complete(ctx):
    global sessions, timer_state
    if timer_state == STATE_FOCUS:
        sessions += 1
        ctx.notification("Pomodoro complete!", f"Session #{sessions} done. Take a break.", priority=2)
    elif timer_state in (STATE_SHORT, STATE_LONG):
        ctx.notification("Break over", "Time to focus!", priority=2)
    timer_state = STATE_IDLE
    remaining_reset()


def remaining_reset():
    global remaining
    remaining = float(DURATIONS.get(timer_state, DURATIONS[STATE_FOCUS]))


@app.on_key
def on_key(key: str, _mods: dict, _emit):
    global running, timer_state, remaining, last_tick

    if key == " ":
        if timer_state == STATE_IDLE:
            timer_state = STATE_FOCUS
            remaining = float(DURATIONS[STATE_FOCUS])
        running = not running
        last_tick = time.monotonic() if running else 0.0

    elif key == "r":
        running = False
        last_tick = 0.0
        remaining = float(DURATIONS.get(timer_state, DURATIONS[STATE_FOCUS]))

    elif key == "b":
        running = False
        last_tick = 0.0
        timer_state = STATE_SHORT
        remaining = float(DURATIONS[STATE_SHORT])

    elif key == "l":
        running = False
        last_tick = 0.0
        timer_state = STATE_LONG
        remaining = float(DURATIONS[STATE_LONG])


app.run()
