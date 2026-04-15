from __future__ import annotations

"""
pulse — Plexi app
Visual beat sequencer. 8 instruments x 16 steps. Build rhythms, watch them pulse.

Controls:
  Arrow keys    Move cursor
  Space/Enter   Toggle cell
  [/]           Decrease/Increase BPM
  m             Mute row
  r             Randomize row
  c             Clear all
  p             Pause/resume
"""

import math
import os
import random
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from plexi_sdk import App

# ---------------------------------------------------------------------------
# Audio setup — graceful degradation if pygame/numpy missing
# ---------------------------------------------------------------------------

AUDIO_OK = False
sounds: dict[int, object] = {}

try:
    import numpy as np
    import pygame
    import pygame.sndarray

    pygame.mixer.pre_init(frequency=44100, size=-16, channels=1, buffer=512)
    pygame.mixer.init()
    SAMPLE_RATE = 44100

    def _make_sound(samples: "np.ndarray") -> "pygame.mixer.Sound":
        samples = np.clip(samples, -1.0, 1.0)
        pcm = (samples * 32767).astype(np.int16)
        return pygame.sndarray.make_sound(pcm)

    def _sine(freq: float, dur: float, env_decay: float = 1.0) -> "np.ndarray":
        t = np.linspace(0, dur, int(SAMPLE_RATE * dur), endpoint=False)
        env = np.exp(-t * env_decay / dur * 10)
        return np.sin(2 * np.pi * freq * t) * env

    def _noise(dur: float, env_decay: float = 1.0) -> "np.ndarray":
        n = int(SAMPLE_RATE * dur)
        t = np.linspace(0, dur, n, endpoint=False)
        env = np.exp(-t * env_decay / dur * 10)
        return (np.random.uniform(-1, 1, n) * env)

    def _sweep(f0: float, f1: float, dur: float) -> "np.ndarray":
        n = int(SAMPLE_RATE * dur)
        t = np.linspace(0, dur, n, endpoint=False)
        freqs = np.linspace(f0, f1, n)
        phase = np.cumsum(freqs) / SAMPLE_RATE * 2 * np.pi
        env = np.exp(-t * 8)
        return np.sin(phase) * env

    # Row index → sound generator
    _generators = [
        lambda: _sweep(80, 30, 0.15),                          # 0 Kick
        lambda: _noise(0.10, 1.0) * 0.6 + _sine(180, 0.10),   # 1 Snare
        lambda: _noise(0.05, 1.2),                             # 2 Hi-Hat
        lambda: _noise(0.08, 1.5) * 0.7 + _sine(220, 0.08),   # 3 Clap
        lambda: _sweep(120, 60, 0.12),                         # 4 Tom
        lambda: _sine(300, 0.08) * 0.9,                        # 5 Rim
        lambda: _sine(562, 0.20) * 0.8,                        # 6 Cowbell
        lambda: _sweep(55, 40, 0.20),                          # 7 Bass
    ]

    for i, gen in enumerate(_generators):
        try:
            sounds[i] = _make_sound(gen())
        except Exception:
            pass

    AUDIO_OK = True
except Exception:
    pass


def play_sound(row: int):
    if not AUDIO_OK:
        return
    snd = sounds.get(row)
    if snd is not None:
        try:
            snd.play()
        except Exception:
            pass


# ---------------------------------------------------------------------------
# Colors — Catppuccin Mocha
# ---------------------------------------------------------------------------

BG          = "#1e1e2e"
SURFACE     = "#313244"
OVERLAY     = "#45475a"
TEXT        = "#cdd6f4"
SUBTEXT     = "#6c7086"
HEADER      = "#181825"
PLAYHEAD    = "#ffffff"

ROW_COLORS = [
    "#f38ba8",  # Kick
    "#fab387",  # Snare
    "#f9e2af",  # Hi-Hat
    "#a6e3a1",  # Clap
    "#89b4fa",  # Tom
    "#94e2d5",  # Rim
    "#cba6f7",  # Cowbell
    "#eba0ac",  # Bass
]

ROW_NAMES = ["Kick", "Snare", "Hi-Hat", "Clap", "Tom", "Rim", "Cowbell", "Bass"]

ROWS = 8
COLS = 16
BPM_MIN = 60
BPM_MAX = 200

# ---------------------------------------------------------------------------
# State
# ---------------------------------------------------------------------------

grid: list[list[bool]] = [[False] * COLS for _ in range(ROWS)]
muted: list[bool] = [False] * ROWS
cursor_row = 0
cursor_col = 0
bpm = 120
paused = False

# Playhead
current_step = 0
_last_step_time = time.monotonic()

# Active pulses: list of (row, col, start_time)
pulses: list[tuple[int, int, float]] = []

PULSE_DURATION = 0.3  # seconds


def step_interval() -> float:
    return 60.0 / bpm / 4  # 16th notes


def _hex_to_rgb(h: str) -> tuple[int, int, int]:
    h = h.lstrip("#")
    return int(h[0:2], 16), int(h[2:4], 16), int(h[4:6], 16)


def _rgb_with_alpha(color: str, alpha: float) -> str:
    r, g, b = _hex_to_rgb(color)
    ri = int(r * alpha + int(BG.lstrip("#")[0:2], 16) * (1 - alpha))
    gi = int(g * alpha + int(BG.lstrip("#")[2:4], 16) * (1 - alpha))
    bi = int(b * alpha + int(BG.lstrip("#")[4:6], 16) * (1 - alpha))
    return f"#{ri:02x}{gi:02x}{bi:02x}"


# ---------------------------------------------------------------------------
# App
# ---------------------------------------------------------------------------

app = App(app_id="pulse")


@app.on_render
def render(ctx):
    global current_step, _last_step_time, paused

    now = time.monotonic()

    # Advance playhead
    if not paused:
        elapsed = now - _last_step_time
        if elapsed >= step_interval():
            _last_step_time = now
            current_step = (current_step + 1) % COLS
            # Fire sounds and pulses for active cells in this column
            for row in range(ROWS):
                if grid[row][current_step] and not muted[row]:
                    play_sound(row)
                    pulses.append((row, current_step, now))

    # Expire old pulses
    pulses[:] = [(r, c, t) for (r, c, t) in pulses if now - t < PULSE_DURATION]

    # ---- Layout ----
    header_h = 36.0
    footer_h = 24.0
    label_w = 56.0
    pad = 8.0

    grid_x = pad + label_w
    grid_y = header_h + pad
    grid_w = ctx.width - grid_x - pad
    grid_h = ctx.height - header_h - footer_h - pad * 2

    cell_w = grid_w / COLS
    cell_h = grid_h / ROWS

    # Background
    ctx.rect(0, 0, ctx.width, ctx.height, fill=BG)

    # Header
    ctx.rect(0, 0, ctx.width, header_h, fill=HEADER)
    state_label = "PAUSED" if paused else f"BPM {bpm}"
    ctx.text(pad, 10, "PULSE", size=15, color=TEXT, bold=True)
    ctx.text(pad + 70, 11, state_label, size=13, color=SUBTEXT)
    step_label = f"Step {current_step + 1:02d}/16"
    ctx.text(ctx.width - len(step_label) * 7.5 - pad, 11, step_label, size=12, color=SUBTEXT)
    hint = "[ ] bpm  m mute  r rand  c clear  p pause"
    ctx.text(ctx.width / 2 - len(hint) * 3.5, 11, hint, size=10, color=SUBTEXT)

    # Row labels + mute indicators
    for row in range(ROWS):
        color = ROW_COLORS[row]
        ry = grid_y + row * cell_h
        label_color = SUBTEXT if muted[row] else color
        ctx.text(pad, ry + cell_h / 2 - 6, ROW_NAMES[row], size=10, color=label_color)
        if muted[row]:
            ctx.text(pad, ry + cell_h / 2 + 4, "mute", size=8, color=OVERLAY)

    # Playhead band (behind cells)
    ph_x = grid_x + current_step * cell_w
    ctx.rect(ph_x, grid_y, cell_w, grid_h, fill=_rgb_with_alpha(PLAYHEAD, 0.08))

    # Grid cells
    cell_pad = 2.0
    for row in range(ROWS):
        color = ROW_COLORS[row]
        dim_color = _rgb_with_alpha(color, 0.18)
        for col in range(COLS):
            cx = grid_x + col * cell_w + cell_pad
            cy = grid_y + row * cell_h + cell_pad
            cw = cell_w - cell_pad * 2
            ch = cell_h - cell_pad * 2
            radius = 3.0

            if grid[row][col]:
                fill = _rgb_with_alpha(color, 0.85) if muted[row] else color
            else:
                fill = dim_color

            ctx.rect(cx, cy, cw, ch, fill=fill, radius=radius)

            # Cursor border (draw as four thin rects)
            if row == cursor_row and col == cursor_col:
                bw = 2.0
                ctx.rect(cx, cy, cw, bw, fill="#ffffff")
                ctx.rect(cx, cy + ch - bw, cw, bw, fill="#ffffff")
                ctx.rect(cx, cy, bw, ch, fill="#ffffff")
                ctx.rect(cx + cw - bw, cy, bw, ch, fill="#ffffff")

    # Pulse rings
    for (pr, pc, pt) in pulses:
        age = now - pt
        t = age / PULSE_DURATION        # 0..1
        alpha = max(0.0, 1.0 - t)
        max_expand = min(cell_w, cell_h) * 1.2
        expand = t * max_expand

        color = ROW_COLORS[pr]
        ring_x = grid_x + pc * cell_w + cell_w / 2
        ring_y = grid_y + pr * cell_h + cell_h / 2
        hw = cell_w / 2 + expand
        hh = cell_h / 2 + expand
        bw = max(1.0, 3.0 * (1.0 - t))

        rw = hw * 2
        rh = hh * 2
        rx = ring_x - hw
        ry = ring_y - hh
        ring_color = _rgb_with_alpha(color, alpha)

        ctx.rect(rx, ry, rw, bw, fill=ring_color)
        ctx.rect(rx, ry + rh - bw, rw, bw, fill=ring_color)
        ctx.rect(rx, ry, bw, rh, fill=ring_color)
        ctx.rect(rx + rw - bw, ry, bw, rh, fill=ring_color)

    # Footer
    fy = ctx.height - footer_h
    ctx.rect(0, fy, ctx.width, footer_h, fill=HEADER)
    audio_label = "audio: on" if AUDIO_OK else "audio: off (install pygame + numpy)"
    ctx.text(pad, fy + 7, audio_label, size=10, color=SUBTEXT)


@app.on_key
def on_key(key, _mods, _emit):
    global cursor_row, cursor_col, bpm, paused, current_step, _last_step_time

    if key == "ArrowUp":
        cursor_row = (cursor_row - 1) % ROWS
    elif key == "ArrowDown":
        cursor_row = (cursor_row + 1) % ROWS
    elif key == "ArrowLeft":
        cursor_col = (cursor_col - 1) % COLS
    elif key == "ArrowRight":
        cursor_col = (cursor_col + 1) % COLS

    elif key in ("Enter", " "):
        grid[cursor_row][cursor_col] = not grid[cursor_row][cursor_col]

    elif key == "[":
        bpm = max(BPM_MIN, bpm - 5)
    elif key == "]":
        bpm = min(BPM_MAX, bpm + 5)

    elif key == "m":
        muted[cursor_row] = not muted[cursor_row]

    elif key == "r":
        for col in range(COLS):
            grid[cursor_row][col] = random.random() < 0.35

    elif key == "c":
        for row in range(ROWS):
            for col in range(COLS):
                grid[row][col] = False
        pulses.clear()

    elif key == "p":
        paused = not paused
        if not paused:
            _last_step_time = time.monotonic()


app.run()
