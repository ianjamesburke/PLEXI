from __future__ import annotations
#!/usr/bin/env python3
"""
lichen — Plexi app
Cellular automaton art toy. Watch moss colonies bloom, branch, and crumble.
"""

import os
import random
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from plexi_sdk import App

# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

COLS = 100
ROWS = 60

# Cell states
DEAD = 0
YOUNG = 1
MATURE = 2
OLD = 3
SPORE = 4

# Colors
BG = "#1e1e2e"
COLOR = {
    YOUNG:  "#a6e3a1",
    MATURE: "#40a02b",
    OLD:    "#6c7086",
    SPORE:  "#cba6f7",
}

# Decay speed tick-count multipliers
DECAY_MULTS = {"slow": 2.0, "normal": 1.0, "fast": 0.5}

TICK_INTERVAL = 0.10  # seconds between simulation steps (~10 tps)

# ---------------------------------------------------------------------------
# Simulation state
# ---------------------------------------------------------------------------

# grid[r][c] = (state, age, spore_life)
# spore_life counts down from 8, only meaningful for SPORE state
_grid: list[list[list[int]]] = []
_cursor_r = ROWS // 2
_cursor_c = COLS // 2
_paused = False
_birth_threshold = 3      # 2–4
_spore_rate = 10          # 0–30 (%)
_decay_speed = "normal"   # slow | normal | fast
_last_tick = 0.0
_initialized = False

# Neighbour offsets (8-connected)
_NBRS = [(-1,-1),(-1,0),(-1,1),(0,-1),(0,1),(1,-1),(1,0),(1,1)]


def _make_grid() -> list[list[list[int]]]:
    return [[[DEAD, 0, 0] for _ in range(COLS)] for _ in range(ROWS)]


def _init():
    global _grid, _initialized, _last_tick
    _grid = _make_grid()
    _last_tick = time.monotonic()
    _initialized = True


def _age_thresholds() -> tuple[int, int, int]:
    """Return (young_ticks, mature_ticks, old_ticks) adjusted for decay speed."""
    m = DECAY_MULTS[_decay_speed]
    return int(3 * m), int(6 * m), int(3 * m)


def _tick():
    global _grid
    young_ticks, mature_ticks, old_ticks = _age_thresholds()
    next_grid = _make_grid()

    for r in range(ROWS):
        for c in range(COLS):
            state, age, sl = _grid[r][c]

            if state == SPORE:
                # Drift in a random direction
                dr, dc = random.choice(_NBRS)
                nr, nc = (r + dr) % ROWS, (c + dc) % COLS
                new_sl = sl - 1
                if new_sl <= 0:
                    # Spore expires
                    pass
                elif _grid[nr][nc][0] == DEAD and next_grid[nr][nc][0] == DEAD:
                    if random.random() < 0.20:
                        next_grid[nr][nc] = [YOUNG, 0, 0]
                    else:
                        next_grid[nr][nc] = [SPORE, 0, new_sl]
                else:
                    # Landing spot occupied — just die
                    pass
                continue

            if state == DEAD:
                # Count Young + Mature neighbours
                live = sum(
                    1 for dr, dc in _NBRS
                    if _grid[(r+dr) % ROWS][(c+dc) % COLS][0] in (YOUNG, MATURE)
                )
                if live >= _birth_threshold:
                    next_grid[r][c] = [YOUNG, 0, 0]
                # else stays dead
                continue

            if state == YOUNG:
                new_age = age + 1
                if new_age >= young_ticks:
                    next_grid[r][c] = [MATURE, 0, 0]
                else:
                    next_grid[r][c] = [YOUNG, new_age, 0]
                continue

            if state == MATURE:
                new_age = age + 1
                if new_age >= mature_ticks:
                    next_grid[r][c] = [OLD, 0, 0]
                else:
                    next_grid[r][c] = [MATURE, new_age, 0]
                continue

            if state == OLD:
                new_age = age + 1
                # Possibly emit spore to random empty neighbour
                if random.randint(1, 100) <= _spore_rate:
                    candidates = [
                        ((r+dr) % ROWS, (c+dc) % COLS)
                        for dr, dc in _NBRS
                        if _grid[(r+dr) % ROWS][(c+dc) % COLS][0] == DEAD
                    ]
                    if candidates:
                        sr, sc = random.choice(candidates)
                        if next_grid[sr][sc][0] == DEAD:
                            next_grid[sr][sc] = [SPORE, 0, 8]

                if new_age >= old_ticks:
                    next_grid[r][c] = [DEAD, 0, 0]
                else:
                    next_grid[r][c] = [OLD, new_age, 0]
                continue

    _grid = next_grid


# ---------------------------------------------------------------------------
# Controls
# ---------------------------------------------------------------------------

def _seed_cluster(r: int, c: int, radius: int = 2):
    """Plant a 5x5 cluster of Young cells at (r, c)."""
    for dr in range(-radius, radius + 1):
        for dc in range(-radius, radius + 1):
            nr, nc = (r + dr) % ROWS, (c + dc) % COLS
            _grid[nr][nc] = [YOUNG, 0, 0]


def _randomize():
    global _grid
    _grid = _make_grid()
    count = int(COLS * ROWS * 0.10)
    for _ in range(count):
        r = random.randrange(ROWS)
        c = random.randrange(COLS)
        _grid[r][c] = [YOUNG, 0, 0]


# ---------------------------------------------------------------------------
# App
# ---------------------------------------------------------------------------

app = App(app_id="lichen")


@app.on_render
def render(ctx):
    global _last_tick, _initialized, _paused

    if not _initialized:
        _init()

    now = time.monotonic()

    # Advance simulation
    if not _paused and now - _last_tick >= TICK_INTERVAL:
        _tick()
        _last_tick = now

    w = ctx.width
    h = ctx.height

    # Cell pixel dimensions
    cw = w / COLS
    ch = (h - 26) / ROWS  # leave 26px for status bar

    # Background — fill once
    ctx.rect(0, 0, w, h, fill=BG)

    # Draw live cells only
    for r in range(ROWS):
        for c in range(COLS):
            state = _grid[r][c][0]
            if state == DEAD:
                continue
            color = COLOR[state]
            px = c * cw
            py = r * ch
            if state == SPORE:
                # Slightly smaller dot for spores
                pad = max(0.5, cw * 0.2)
                ctx.rect(px + pad, py + pad, cw - pad * 2, ch - pad * 2, fill=color)
            else:
                ctx.rect(px, py, cw + 0.5, ch + 0.5, fill=color)

    # Cursor — draw a small outline box
    cx_px = _cursor_c * cw
    cy_px = _cursor_r * ch
    ctx.rect(cx_px, cy_px, cw, ch, fill="#ffffff26")
    ctx.line(cx_px, cy_px, cx_px + cw, cy_px, color="#cdd6f4", width=1.0)
    ctx.line(cx_px, cy_px + ch, cx_px + cw, cy_px + ch, color="#cdd6f4", width=1.0)
    ctx.line(cx_px, cy_px, cx_px, cy_px + ch, color="#cdd6f4", width=1.0)
    ctx.line(cx_px + cw, cy_px, cx_px + cw, cy_px + ch, color="#cdd6f4", width=1.0)

    # Status bar
    bar_y = h - 24
    ctx.rect(0, bar_y, w, 24, fill="#181825")

    pause_label = "PAUSED" if _paused else "playing"
    status = (
        f"  Birth≥{_birth_threshold}  Spore:{_spore_rate}%  "
        f"Decay:{_decay_speed}  [{pause_label}]  "
        f"Spc:seed  c:clear  r:rand  p:pause  [/]:birth  ,/.:spore  1/2/3:decay"
    )
    ctx.text(4, bar_y + 5, status, size=11, color="#6c7086", monospace=True)


@app.on_key
def on_key(key: str, mods: dict, emit):
    global _cursor_r, _cursor_c, _paused, _birth_threshold, _spore_rate, _decay_speed

    if key == "ArrowUp":
        _cursor_r = (_cursor_r - 1) % ROWS
    elif key == "ArrowDown":
        _cursor_r = (_cursor_r + 1) % ROWS
    elif key == "ArrowLeft":
        _cursor_c = (_cursor_c - 1) % COLS
    elif key == "ArrowRight":
        _cursor_c = (_cursor_c + 1) % COLS
    elif key == " ":
        _seed_cluster(_cursor_r, _cursor_c)
    elif key == "c":
        global _grid
        _grid = _make_grid()
    elif key == "r":
        _randomize()
    elif key == "p":
        _paused = not _paused
    elif key == "[":
        _birth_threshold = max(2, _birth_threshold - 1)
    elif key == "]":
        _birth_threshold = min(4, _birth_threshold + 1)
    elif key == ",":
        _spore_rate = max(0, _spore_rate - 5)
    elif key == ".":
        _spore_rate = min(30, _spore_rate + 5)
    elif key == "1":
        _decay_speed = "slow"
    elif key == "2":
        _decay_speed = "normal"
    elif key == "3":
        _decay_speed = "fast"


@app.on_resize
def on_resize(width: float, height: float):
    # No reinit needed — grid is independent of pane size
    pass


app.run()
