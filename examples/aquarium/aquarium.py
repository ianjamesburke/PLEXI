#!/usr/bin/env python3
"""
aquarium — Plexi app
Ambient underwater scene with procedurally animated fish and plants.
No user input required — pure ambient animation.
"""

import math
import os
import random
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from plexi_sdk import App

# ---------------------------------------------------------------------------
# Colors — Catppuccin Mocha + aquatic palette
# ---------------------------------------------------------------------------

C = {
    "water_top":    "#2a2a4e",
    "water_bottom": "#1e1e3e",
    "sand":         "#585b70",
    "sand_light":   "#6c7086",
    "plant_green":  "#a6e3a1",
    "plant_teal":   "#94e2d5",
    "bubble":       "#cdd6f4",
    "text":         "#cdd6f4",
    "subtext":      "#6c7086",
}

# Fish color palette — vivid but not garish
FISH_COLORS = [
    "#89b4fa",  # blue
    "#f38ba8",  # pink/red
    "#fab387",  # peach/orange
    "#f9e2af",  # yellow
    "#a6e3a1",  # green
    "#94e2d5",  # teal
    "#cba6f7",  # mauve/purple
    "#89dceb",  # sky
]

SAND_H = 28       # height of sand strip at bottom
NUM_FISH = 12
NUM_PLANTS = 5
MAX_BUBBLES = 15

# ---------------------------------------------------------------------------
# Entity classes
# ---------------------------------------------------------------------------


class Fish:
    def __init__(self, width, height):
        self.reset(width, height, initial=True)

    def reset(self, width, height, initial=False):
        self.size = random.uniform(20.0, 55.0)
        self.speed = random.uniform(25.0, 75.0)
        self.direction = random.choice([-1, 1])
        self.base_y = random.uniform(height * 0.1, height - SAND_H - self.size * 1.2)
        self.amplitude = random.uniform(5.0, 18.0)
        self.freq = random.uniform(0.4, 1.2)
        self.phase = random.uniform(0.0, math.pi * 2)
        self.color = random.choice(FISH_COLORS)
        self.depth = random.uniform(0.3, 1.0)   # 0=background,1=foreground
        self.tail_phase = random.uniform(0.0, math.pi * 2)
        self.bubble_timer = random.uniform(0.5, 4.0)

        # Position
        if initial:
            self.x = random.uniform(0, width)
        else:
            if self.direction > 0:
                self.x = -self.size * 2
            else:
                self.x = width + self.size * 2
        self.y = self.base_y


class Plant:
    def __init__(self, x, height):
        self.x = x
        self.segments = random.randint(3, 5)
        self.seg_len = random.uniform(20.0, 35.0)
        self.height = self.segments * self.seg_len
        self.sway_amp = random.uniform(6.0, 14.0)
        self.sway_freq = random.uniform(0.4, 0.8)
        self.phase = random.uniform(0.0, math.pi * 2)
        self.color = random.choice([C["plant_green"], C["plant_teal"]])
        self.base_y = height - SAND_H


class Bubble:
    def __init__(self, x, y):
        self.x = x
        self.y = y
        self.r = random.uniform(2.0, 5.0)
        self.speed = random.uniform(18.0, 40.0)
        self.drift_freq = random.uniform(0.8, 2.0)
        self.drift_phase = random.uniform(0.0, math.pi * 2)
        self.drift_amp = random.uniform(4.0, 10.0)
        self.opacity = random.uniform(0.5, 0.9)
        self.birth_time = time.monotonic()

    @property
    def alive(self):
        return self.opacity > 0.05 and self.y > 0


# ---------------------------------------------------------------------------
# Scene state
# ---------------------------------------------------------------------------

_start_time = time.monotonic()
_last_time = _start_time
_fish = []
_plants = []
_bubbles = []
_initialized = False


def _init_scene(width, height):
    global _fish, _plants, _bubbles, _initialized
    _fish = [Fish(width, height) for _ in range(NUM_FISH)]
    plant_xs = []
    for i in range(NUM_PLANTS):
        px = width * (i + 0.5) / NUM_PLANTS + random.uniform(-20, 20)
        plant_xs.append(max(20.0, min(width - 20.0, px)))
    _plants = [Plant(px, height) for px in plant_xs]
    _bubbles = []
    _initialized = True


# ---------------------------------------------------------------------------
# Render helpers
# ---------------------------------------------------------------------------


def _lerp_color(c1_hex, c2_hex, t):
    """Interpolate between two hex colors, return hex string."""
    def parse(h):
        h = h.lstrip("#")
        return int(h[0:2], 16), int(h[2:4], 16), int(h[4:6], 16)
    r1, g1, b1 = parse(c1_hex)
    r2, g2, b2 = parse(c2_hex)
    r = int(r1 + (r2 - r1) * t)
    g = int(g1 + (g2 - g1) * t)
    b = int(b1 + (b2 - b1) * t)
    return f"#{r:02x}{g:02x}{b:02x}"


def _hex_with_alpha(hex_color, opacity):
    """Return hex color with alpha channel (as 8-char hex including opacity byte)."""
    # Plexi rect fill takes a standard hex — approximate opacity by blending with water color
    # using _lerp_color toward the background. This avoids needing true RGBA support.
    alpha_hex = f"#{hex_color.lstrip('#')}{int(opacity * 255):02x}"
    return alpha_hex


def _draw_background(ctx, now):
    w = ctx.width
    h = ctx.height
    # Water gradient — bands from bottom (dark) to top (slightly lighter)
    bands = 10
    band_h = (h - SAND_H) / bands
    for i in range(bands):
        t = i / bands  # 0=bottom,1=top
        color = _lerp_color(C["water_bottom"], C["water_top"], t)
        by = h - SAND_H - (i + 1) * band_h
        ctx.rect(0, by, w, band_h + 1.0, fill=color)

    # Sand strip
    ctx.rect(0, h - SAND_H, w, SAND_H, fill=C["sand"])
    # Sand highlight line
    ctx.rect(0, h - SAND_H, w, 3.0, fill=C["sand_light"])


def _draw_plants(ctx, now, plants):
    for plant in plants:
        sway = math.sin(now * plant.sway_freq + plant.phase) * plant.sway_amp
        # Draw segments from base upward, each slightly offset by sway
        x = plant.x
        y = plant.base_y
        for seg in range(plant.segments):
            seg_sway = sway * (seg + 1) / plant.segments
            nx = plant.x + seg_sway
            ny = y - plant.seg_len
            ctx.line(x, y, nx, ny, color=plant.color, width=3.0)
            x, y = nx, ny


def _draw_fish(ctx, now, fish, width, height):
    for f in fish:
        # Depth affects opacity approximation — blend toward bg color
        opacity = 0.4 + 0.6 * f.depth
        body_color = _lerp_color(C["water_bottom"], f.color, opacity)

        s = f.size
        cx = f.x
        cy = f.y
        flip = f.direction  # 1=right, -1=left

        # Body — rounded rect approximation
        bw = s
        bh = s * 0.45
        bx = cx - bw / 2.0
        by = cy - bh / 2.0
        ctx.rect(bx, by, bw, bh, fill=body_color, radius=bh * 0.45)

        # Tail — triangle drawn as two lines converging at a tip
        tail_wag = math.sin(now * 3.5 + f.tail_phase) * (s * 0.15)
        tail_base_x = cx - flip * bw * 0.45
        tail_tip_x = tail_base_x - flip * s * 0.4
        tail_tip_y = cy + tail_wag
        tail_top_y = cy - bh * 0.35
        tail_bot_y = cy + bh * 0.35
        ctx.line(tail_base_x, tail_top_y, tail_tip_x, tail_tip_y,
                 color=body_color, width=2.5)
        ctx.line(tail_base_x, tail_bot_y, tail_tip_x, tail_tip_y,
                 color=body_color, width=2.5)

        # Eye — small white dot, positioned toward the head
        eye_x = cx + flip * bw * 0.28
        eye_y = cy - bh * 0.1
        eye_r = max(2.0, s * 0.07)
        ctx.rect(eye_x - eye_r, eye_y - eye_r, eye_r * 2, eye_r * 2,
                 fill="#ffffff", radius=eye_r)
        # Pupil
        pupil_r = max(1.0, eye_r * 0.55)
        ctx.rect(eye_x - pupil_r + flip * pupil_r * 0.3,
                 eye_y - pupil_r,
                 pupil_r * 2, pupil_r * 2,
                 fill="#000000", radius=pupil_r)


def _draw_bubbles(ctx, now, bubbles):
    for b in bubbles:
        if not b.alive:
            continue
        drift = math.sin(now * b.drift_freq + b.drift_phase) * b.drift_amp
        draw_x = b.x + drift
        draw_y = b.y
        # Approximate opacity by blending bubble color with water
        bubble_color = _lerp_color(C["water_top"], C["bubble"], b.opacity * 0.6)
        d = b.r * 2
        ctx.rect(draw_x - b.r, draw_y - b.r, d, d, fill=bubble_color, radius=b.r)


# ---------------------------------------------------------------------------
# Update logic
# ---------------------------------------------------------------------------


def _update(now, dt, width, height):
    # Update fish
    for f in _fish:
        f.x += f.speed * f.direction * dt * f.depth
        f.y = f.base_y + math.sin(now * f.freq + f.phase) * f.amplitude

        # Wrap around screen
        margin = f.size * 2
        if f.direction > 0 and f.x > width + margin:
            f.reset(width, height)
        elif f.direction < 0 and f.x < -margin:
            f.reset(width, height)

        # Random bubble emission
        f.bubble_timer -= dt
        if f.bubble_timer <= 0 and len(_bubbles) < MAX_BUBBLES:
            mouth_x = f.x + f.direction * f.size * 0.45
            mouth_y = f.y - f.size * 0.05
            _bubbles.append(Bubble(mouth_x, mouth_y))
            f.bubble_timer = random.uniform(2.0, 6.0)

    # Emit bubbles from plant tips occasionally
    if len(_bubbles) < MAX_BUBBLES and random.random() < 0.02:
        plant = random.choice(_plants) if _plants else None
        if plant:
            sway = math.sin(now * plant.sway_freq + plant.phase) * plant.sway_amp
            tip_x = plant.x + sway
            tip_y = plant.base_y - plant.height
            _bubbles.append(Bubble(tip_x, tip_y))

    # Update bubbles
    for b in _bubbles:
        b.y -= b.speed * dt
        age = now - b.birth_time
        # Fade out over 3 seconds
        b.opacity = max(0.0, 0.9 - age / 3.0)
        # Grow slightly as they rise
        b.r = min(b.r + 0.2 * dt, 8.0)

    # Remove dead bubbles
    _bubbles[:] = [b for b in _bubbles if b.alive]


# ---------------------------------------------------------------------------
# App
# ---------------------------------------------------------------------------

app = App(app_id="aquarium")


@app.on_render
def render(ctx):
    global _last_time, _initialized

    now = time.monotonic() - _start_time
    wall_now = time.monotonic()
    dt = min(0.1, wall_now - _last_time)
    _last_time = wall_now

    if not _initialized or ctx.width < 10 or ctx.height < 10:
        _init_scene(ctx.width, ctx.height)

    # Update positions before drawing
    _update(now, dt, ctx.width, ctx.height)

    # Draw layers bottom-to-top
    _draw_background(ctx, now)
    _draw_plants(ctx, now, _plants)

    # Sort fish by depth so background fish draw first
    sorted_fish = sorted(_fish, key=lambda f: f.depth)
    _draw_fish(ctx, now, sorted_fish, ctx.width, ctx.height)

    _draw_bubbles(ctx, now, _bubbles)


@app.on_resize
def on_resize(width, height):
    _init_scene(width, height)


app.run()
