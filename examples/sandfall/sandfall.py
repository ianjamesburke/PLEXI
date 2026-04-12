from __future__ import annotations

#!/usr/bin/env python3
"""
sandfall — Plexi app
Falling-sand physics toy. Pour sand, water, fire, ice, stone, and smoke.

Controls:
  1-6        Select material (sand/water/fire/ice/stone/smoke)
  Arrows     Move cursor
  Space      Place material at cursor
  Mouse      Move cursor; click/drag to place
  c          Clear grid
  p          Pause / resume
"""

import os
import random
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from plexi_sdk import App

# ---------------------------------------------------------------------------
# Materials
# ---------------------------------------------------------------------------

EMPTY = 0
SAND  = 1
WATER = 2
FIRE  = 3
ICE   = 4
STONE = 5
SMOKE = 6

MAT_NAMES = {EMPTY: "Empty", SAND: "Sand", WATER: "Water",
             FIRE: "Fire", ICE: "Ice", STONE: "Stone", SMOKE: "Smoke"}

MAT_KEYS  = {"1": SAND, "2": WATER, "3": FIRE, "4": ICE, "5": STONE, "6": SMOKE}

# Base colors (R, G, B)
COL = {
    EMPTY: (0x1e, 0x1e, 0x2e),
    SAND:  (0xc9, 0xa8, 0x5c),
    WATER: (0x4a, 0x9e, 0xff),
    FIRE:  (0xff, 0x6b, 0x35),
    ICE:   (0xa8, 0xd8, 0xf0),
    STONE: (0x6c, 0x70, 0x86),
    SMOKE: (0x93, 0x99, 0xb2),
}
FIRE_ALT = (0xff, 0xaa, 0x44)
BG_HEX   = "#1e1e2e"

# ---------------------------------------------------------------------------
# Physics constants
# ---------------------------------------------------------------------------

TICK_RATE  = 20
TICK_DT    = 1.0 / TICK_RATE
FIRE_LIFE  = 10    # ticks before fire turns to smoke
SMOKE_LIFE = 20    # ticks before smoke disappears
BRUSH_R    = 2     # radius in cells (gives 5x5 brush)

# ---------------------------------------------------------------------------
# Grid
# ---------------------------------------------------------------------------

COLS = 80
ROWS = 50
_SIZE = COLS * ROWS

_mat: list[int] = [EMPTY] * _SIZE
_age: list[int] = [0]     * _SIZE
_upd: list[bool]= [False] * _SIZE  # per-tick updated flag


def _idx(c: int, r: int) -> int:
    return r * COLS + c

def _in(c: int, r: int) -> bool:
    return 0 <= c < COLS and 0 <= r < ROWS

def _get(c: int, r: int) -> int:
    return _mat[_idx(c, r)] if _in(c, r) else STONE  # OOB = solid

def _set(c: int, r: int, m: int, age: int = 0):
    if _in(c, r):
        i = _idx(c, r)
        _mat[i] = m
        _age[i] = age

def clear_grid():
    for i in range(_SIZE):
        _mat[i] = EMPTY
        _age[i] = 0

def _swap(c1: int, r1: int, c2: int, r2: int):
    i1, i2 = _idx(c1, r1), _idx(c2, r2)
    _mat[i1], _mat[i2] = _mat[i2], _mat[i1]
    _age[i1], _age[i2] = _age[i2], _age[i1]
    _upd[i1] = _upd[i2] = True

# ---------------------------------------------------------------------------
# Physics
# ---------------------------------------------------------------------------

def _lighter_than_water(m: int) -> bool:
    return m in (EMPTY, SMOKE)

def tick_physics():
    global _upd
    _upd = [False] * _SIZE
    for r in range(ROWS - 1, -1, -1):
        cols = list(range(COLS))
        random.shuffle(cols)
        for c in cols:
            i = _idx(c, r)
            if _upd[i]:
                continue
            m = _mat[i]
            if   m == SAND:  _tick_sand(c, r)
            elif m == WATER: _tick_water(c, r)
            elif m == FIRE:  _tick_fire(c, r)
            elif m == SMOKE: _tick_smoke(c, r)
            elif m == ICE:   _tick_ice(c, r)


def _tick_sand(c: int, r: int):
    if _in(c, r + 1) and _get(c, r + 1) in (EMPTY, WATER):
        _swap(c, r, c, r + 1); return
    dirs = [-1, 1]; random.shuffle(dirs)
    for dx in dirs:
        if _in(c + dx, r + 1) and _get(c + dx, r + 1) in (EMPTY, WATER):
            _swap(c, r, c + dx, r + 1); return


def _tick_water(c: int, r: int):
    if _in(c, r + 1) and _lighter_than_water(_get(c, r + 1)):
        _swap(c, r, c, r + 1); return
    dirs = [-1, 1]; random.shuffle(dirs)
    for dx in dirs:
        if _in(c + dx, r) and _lighter_than_water(_get(c + dx, r)):
            _swap(c, r, c + dx, r); return
    for dx in dirs:
        if _in(c + dx, r + 1) and _lighter_than_water(_get(c + dx, r + 1)):
            _swap(c, r, c + dx, r + 1); return


def _tick_fire(c: int, r: int):
    i = _idx(c, r)
    _age[i] += 1
    for dc, dr in ((-1, 0), (1, 0), (0, -1), (0, 1)):
        nc, nr = c + dc, r + dr
        if not _in(nc, nr): continue
        nb = _get(nc, nr)
        if nb == ICE:
            _set(nc, nr, WATER)
            _set(c, r, SMOKE, 0)
            return
        if nb == WATER:
            _set(nc, nr, SMOKE, 0)
            _set(c, r, SMOKE, 0)
            return
    if _age[i] >= FIRE_LIFE:
        _set(c, r, SMOKE, 0); return
    if _in(c, r - 1) and _get(c, r - 1) in (EMPTY, SMOKE):
        _swap(c, r, c, r - 1); return
    dx = random.choice((-1, 1))
    if _in(c + dx, r - 1) and _get(c + dx, r - 1) in (EMPTY, SMOKE):
        _swap(c, r, c + dx, r - 1)


def _tick_smoke(c: int, r: int):
    i = _idx(c, r)
    _age[i] += 1
    if _age[i] >= SMOKE_LIFE:
        _mat[i] = EMPTY; _age[i] = 0; return
    dx = random.choice((-1, 0, 0, 1))
    nc, nr = c + dx, r - 1
    if _in(nc, nr) and _get(nc, nr) == EMPTY:
        _swap(c, r, nc, nr); return
    if dx != 0 and _in(c, r - 1) and _get(c, r - 1) == EMPTY:
        _swap(c, r, c, r - 1)


def _tick_ice(c: int, r: int):
    for dc, dr in ((-1, 0), (1, 0), (0, -1), (0, 1)):
        if _get(c + dc, r + dr) == FIRE:
            _set(c, r, WATER); return

# ---------------------------------------------------------------------------
# Color helpers
# ---------------------------------------------------------------------------

def _hex(r: int, g: int, b: int) -> str:
    return f"#{r:02x}{g:02x}{b:02x}"

def _cell_color(m: int, age: int) -> str:
    if m == FIRE:
        t = random.random()
        fr, fg, fb = COL[FIRE]
        ar, ag, ab = FIRE_ALT
        return _hex(int(fr + (ar - fr) * t), int(fg + (ag - fg) * t), int(fb + (ab - fb) * t))
    if m == SMOKE:
        fade = max(0.0, 1.0 - age / SMOKE_LIFE)
        sr, sg, sb = COL[SMOKE]
        er, eg, eb = COL[EMPTY]
        return _hex(int(er + (sr - er) * fade), int(eg + (sg - eg) * fade), int(eb + (sb - eb) * fade))
    if m == SAND:
        v = random.randint(-12, 12)
        r, g, b = COL[SAND]
        return _hex(max(0, min(255, r + v)), max(0, min(255, g + v)), max(0, min(255, b + v)))
    return _hex(*COL[m])

# ---------------------------------------------------------------------------
# App state
# ---------------------------------------------------------------------------

selected_mat = SAND
cursor_c     = COLS // 2
cursor_r     = ROWS // 2
paused       = False
placing      = False

_last_tick   = time.monotonic()

app = App(app_id="sandfall")

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _place_brush(c: int, r: int):
    for dc in range(-BRUSH_R, BRUSH_R + 1):
        for dr in range(-BRUSH_R, BRUSH_R + 1):
            if _in(c + dc, r + dr):
                _set(c + dc, r + dr, selected_mat)

def _cursor_from_px(x: float, y: float):
    global cursor_c, cursor_r
    HUD_H = 22.0
    sim_h = app.height - HUD_H
    cw = app.width / COLS
    ch = sim_h / ROWS
    cursor_c = max(BRUSH_R, min(COLS - BRUSH_R - 1, int(x / cw)))
    cursor_r = max(BRUSH_R, min(ROWS - BRUSH_R - 1, int(y / ch)))

# ---------------------------------------------------------------------------
# Render
# ---------------------------------------------------------------------------

@app.on_render
def render(ctx):
    global _last_tick

    now = time.monotonic()
    if not paused:
        while now - _last_tick >= TICK_DT:
            _last_tick += TICK_DT
            if placing:
                _place_brush(cursor_c, cursor_r)
            tick_physics()

    w, h = ctx.width, ctx.height
    HUD_H = 22.0
    sim_h = h - HUD_H
    cw = w / COLS
    ch = sim_h / ROWS

    # Enable mouse tracking every frame (stateless protocol)
    ctx.mouse_tracking(True)

    # Background
    ctx.rect(0, 0, w, sim_h, fill=BG_HEX)

    # Cells (skip empty)
    for r in range(ROWS):
        for c in range(COLS):
            i = _idx(c, r)
            m = _mat[i]
            if m == EMPTY:
                continue
            ctx.rect(c * cw, r * ch, cw + 0.5, ch + 0.5, fill=_cell_color(m, _age[i]))

    # Cursor border
    bsz = BRUSH_R * 2 + 1
    cx_px = (cursor_c - BRUSH_R) * cw
    cy_px = (cursor_r - BRUSH_R) * ch
    bw, bh = bsz * cw, bsz * ch
    ctx.rect(cx_px - 1, cy_px - 1, bw + 2, bh + 2, fill="#ffffff11", radius=2.0)
    ctx.line(cx_px - 1, cy_px - 1,    cx_px + bw + 1, cy_px - 1,    color="#ffffff", width=1.5)
    ctx.line(cx_px - 1, cy_px + bh + 1, cx_px + bw + 1, cy_px + bh + 1, color="#ffffff", width=1.5)
    ctx.line(cx_px - 1, cy_px - 1,    cx_px - 1, cy_px + bh + 1,    color="#ffffff", width=1.5)
    ctx.line(cx_px + bw + 1, cy_px - 1, cx_px + bw + 1, cy_px + bh + 1, color="#ffffff", width=1.5)

    # HUD bar
    ctx.rect(0, sim_h, w, HUD_H, fill="#181825")
    mat_label = f"[{MAT_NAMES[selected_mat]}]"
    hint = "1-6:mat  Arrows/Mouse:move  Space/Click:place  c:clear  p:pause"
    ctx.text(6, sim_h + 4, mat_label, size=12, color="#cdd6f4", bold=True)
    ctx.text(len(mat_label) * 8 + 10, sim_h + 4, hint, size=11, color="#6c7086")
    if paused:
        ctx.text(w - 70, sim_h + 4, "PAUSED", size=11, color="#f9e2af", bold=True)

# ---------------------------------------------------------------------------
# Input
# ---------------------------------------------------------------------------

@app.on_key
def on_key(key, _mods, _emit):
    global selected_mat, cursor_c, cursor_r, paused, placing
    if key in MAT_KEYS:
        selected_mat = MAT_KEYS[key]
    elif key == "c":
        clear_grid()
    elif key == "p":
        paused = not paused
    elif key == "ArrowLeft":
        cursor_c = max(BRUSH_R, cursor_c - 1)
    elif key == "ArrowRight":
        cursor_c = min(COLS - BRUSH_R - 1, cursor_c + 1)
    elif key == "ArrowUp":
        cursor_r = max(BRUSH_R, cursor_r - 1)
    elif key == "ArrowDown":
        cursor_r = min(ROWS - BRUSH_R - 1, cursor_r + 1)
    elif key == " ":
        placing = True
        _place_brush(cursor_c, cursor_r)


@app.on_mouse_down
def on_mouse_down(x, y, button, _emit):
    global placing
    if button == "left":
        placing = True
        _cursor_from_px(x, y)
        _place_brush(cursor_c, cursor_r)


@app.on_mouse_up
def on_mouse_up(x, y, button, _emit):
    global placing
    if button == "left":
        placing = False


@app.on_mouse_move
def on_mouse_move(x, y, _emit):
    _cursor_from_px(x, y)


app.run()
