#!/usr/bin/env python3
"""
snake — Plexi app
Classic Snake game. Keyboard controlled, fixed-tick game loop.

Controls:
  Arrow keys / WASD   Change direction
  Enter               Start / restart
  Escape              Return to title
  p                   Pause / unpause
"""

import math
import os
import random
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from plexi_sdk import App

# ---------------------------------------------------------------------------
# Colors — Catppuccin Mocha
# ---------------------------------------------------------------------------

C = {
    "bg":       "#1e1e2e",
    "surface":  "#313244",
    "overlay":  "#45475a",
    "text":     "#cdd6f4",
    "subtext":  "#6c7086",
    "accent":   "#89b4fa",  # snake head
    "tail":     "#45475a",  # snake tail end
    "food":     "#f38ba8",  # food
    "green":    "#a6e3a1",
    "yellow":   "#f9e2af",
    "header":   "#181825",
}

# ---------------------------------------------------------------------------
# Game constants
# ---------------------------------------------------------------------------

GRID_COLS = 20
GRID_ROWS = 20
BASE_INTERVAL = 0.15   # seconds per tick at speed 1
MIN_INTERVAL  = 0.07   # fastest possible tick
SPEED_STEP    = 5      # eat this many foods to increase speed

# Direction vectors
UP    = (0, -1)
DOWN  = (0,  1)
LEFT  = (-1, 0)
RIGHT = (1,  0)
OPPOSITE = {UP: DOWN, DOWN: UP, LEFT: RIGHT, RIGHT: LEFT}

# ---------------------------------------------------------------------------
# State machine
# ---------------------------------------------------------------------------

STATE_TITLE    = "title"
STATE_PLAYING  = "playing"
STATE_PAUSED   = "paused"
STATE_GAMEOVER = "gameover"


class SnakeGame:
    def __init__(self):
        self.state = STATE_TITLE
        self.score = 0
        self.high_score = 0

        # Snake body: list of (col, row) tuples, head first
        self.body = []
        self.direction = RIGHT
        self.input_queue = []   # buffered direction changes

        self.food = (0, 0)
        self.tick_interval = BASE_INTERVAL
        self._last_tick = time.monotonic()

    def start(self):
        mid = GRID_COLS // 2
        self.body = [(mid, GRID_ROWS // 2), (mid - 1, GRID_ROWS // 2), (mid - 2, GRID_ROWS // 2)]
        self.direction = RIGHT
        self.input_queue = []
        self.score = 0
        self.tick_interval = BASE_INTERVAL
        self._last_tick = time.monotonic()
        self._spawn_food()
        self.state = STATE_PLAYING

    def _spawn_food(self):
        body_set = set(self.body)
        candidates = [
            (c, r)
            for c in range(GRID_COLS)
            for r in range(GRID_ROWS)
            if (c, r) not in body_set
        ]
        if candidates:
            self.food = random.choice(candidates)

    def tick(self):
        """Advance one game step. Returns True if the game is still running."""
        if self.input_queue:
            candidate = self.input_queue.pop(0)
            if candidate != OPPOSITE.get(self.direction, None):
                self.direction = candidate

        dx, dy = self.direction
        hx, hy = self.body[0]
        new_head = (hx + dx, hy + dy)

        # Wall collision
        nx, ny = new_head
        if nx < 0 or nx >= GRID_COLS or ny < 0 or ny >= GRID_ROWS:
            self._die()
            return False

        # Self collision (check against all but the tail, which will move)
        if new_head in set(self.body[:-1]):
            self._die()
            return False

        ate = new_head == self.food
        self.body.insert(0, new_head)
        if ate:
            self.score += 1
            self._spawn_food()
            # Speed up every SPEED_STEP foods
            speed_level = self.score // SPEED_STEP
            self.tick_interval = max(MIN_INTERVAL, BASE_INTERVAL - speed_level * 0.01)
        else:
            self.body.pop()

        return True

    def _die(self):
        if self.score > self.high_score:
            self.high_score = self.score
        self.state = STATE_GAMEOVER

    def queue_direction(self, new_dir):
        # Don't add a duplicate of the last queued direction
        last = self.input_queue[-1] if self.input_queue else self.direction
        if new_dir == last or new_dir == OPPOSITE.get(last, None):
            return
        self.input_queue.append(new_dir)


game = SnakeGame()

# ---------------------------------------------------------------------------
# App
# ---------------------------------------------------------------------------

app = App(app_id="snake")


@app.on_render
def render(ctx):
    now = time.monotonic()

    # Tick the game loop at fixed interval
    if game.state == STATE_PLAYING:
        elapsed = now - game._last_tick
        if elapsed >= game.tick_interval:
            game._last_tick = now
            game.tick()

    # Background
    ctx.rect(0, 0, ctx.width, ctx.height, fill=C["bg"])

    if game.state == STATE_TITLE:
        _render_title(ctx, now)
    elif game.state in (STATE_PLAYING, STATE_PAUSED):
        _render_game(ctx, now)
        if game.state == STATE_PAUSED:
            _render_overlay(ctx, "PAUSED", "Press p to resume", now)
    elif game.state == STATE_GAMEOVER:
        _render_game(ctx, now)
        _render_gameover(ctx)


def _cell_layout(ctx):
    """Return (cell_size, offset_x, offset_y) for the current pane size."""
    cell_size = max(8, min(ctx.width // GRID_COLS, ctx.height // GRID_ROWS))
    offset_x = (ctx.width - cell_size * GRID_COLS) / 2.0
    offset_y = (ctx.height - cell_size * GRID_ROWS) / 2.0
    return cell_size, offset_x, offset_y


def _render_title(ctx, now):
    w = ctx.width
    h = ctx.height
    cx = w / 2.0
    cy = h / 2.0

    ctx.text(cx - 60, cy - 60, "SNAKE", size=32, color=C["accent"], bold=True)
    ctx.text(cx - 90, cy, "Press Enter to start", size=14, color=C["text"])

    if game.high_score > 0:
        hs_label = f"High score: {game.high_score}"
        ctx.text(cx - len(hs_label) * 4, cy + 30, hs_label, size=13, color=C["subtext"])

    ctx.text(cx - 110, cy + 70,
             "Arrow keys / WASD  |  p = pause  |  Esc = menu",
             size=11, color=C["subtext"])


def _render_game(ctx, now):
    cell_size, ox, oy = _cell_layout(ctx)
    cs = cell_size

    # Grid lines
    for c in range(GRID_COLS + 1):
        x = ox + c * cs
        ctx.line(x, oy, x, oy + GRID_ROWS * cs, color=C["surface"], width=1.0)
    for r in range(GRID_ROWS + 1):
        y = oy + r * cs
        ctx.line(ox, y, ox + GRID_COLS * cs, y, color=C["surface"], width=1.0)

    # Snake body (tail → head, so head draws on top)
    body_len = len(game.body)
    for i, (col, row) in enumerate(reversed(game.body)):
        ratio = i / max(1, body_len - 1)
        # Interpolate from tail color to head color
        r_head, g_head, b_head = 0x89, 0xb4, 0xfa
        r_tail, g_tail, b_tail = 0x45, 0x47, 0x5a
        ri = int(r_tail + (r_head - r_tail) * (1.0 - ratio))
        gi = int(g_tail + (g_head - g_tail) * (1.0 - ratio))
        bi = int(b_tail + (b_head - b_tail) * (1.0 - ratio))
        color = f"#{ri:02x}{gi:02x}{bi:02x}"
        pad = max(1, cs // 8)
        bx = ox + col * cs + pad
        by = oy + row * cs + pad
        bw = cs - pad * 2
        bh = cs - pad * 2
        radius = max(1.0, cs / 5.0)
        ctx.rect(bx, by, bw, bh, fill=color, radius=radius)

    # Food — pulsing scale via sine
    pulse = 0.85 + 0.15 * math.sin(now * 4.0)
    fc, fr = game.food
    food_cs = cs * pulse
    food_pad = (cs - food_cs) / 2.0
    fx = ox + fc * cs + food_pad
    fy = oy + fr * cs + food_pad
    ctx.rect(fx, fy, food_cs, food_cs, fill=C["food"], radius=food_cs / 3.0)

    # Score
    score_label = f"Score: {game.score}"
    ctx.text(ctx.width - len(score_label) * 8 - 8, 8, score_label,
             size=13, color=C["text"], bold=True)

    speed_level = game.score // SPEED_STEP + 1
    level_label = f"Spd {speed_level}"
    ctx.text(ctx.width - len(level_label) * 8 - 8, 26, level_label,
             size=11, color=C["subtext"])


def _render_overlay(ctx, title, subtitle, now):
    w = ctx.width
    h = ctx.height
    # Semi-transparent dark panel
    panel_w = 260.0
    panel_h = 100.0
    px = (w - panel_w) / 2.0
    py = (h - panel_h) / 2.0
    ctx.rect(px, py, panel_w, panel_h, fill=C["header"], radius=10.0)
    ctx.text(px + panel_w / 2 - len(title) * 9, py + 18, title,
             size=20, color=C["yellow"], bold=True)
    ctx.text(px + panel_w / 2 - len(subtitle) * 5, py + 56, subtitle,
             size=13, color=C["subtext"])


def _render_gameover(ctx):
    w = ctx.width
    h = ctx.height
    panel_w = 280.0
    panel_h = 130.0
    px = (w - panel_w) / 2.0
    py = (h - panel_h) / 2.0
    ctx.rect(px, py, panel_w, panel_h, fill=C["header"], radius=10.0)
    ctx.text(px + panel_w / 2 - 72, py + 14, "GAME OVER",
             size=22, color=C["food"], bold=True)
    score_line = f"Score: {game.score}"
    ctx.text(px + panel_w / 2 - len(score_line) * 5, py + 50,
             score_line, size=14, color=C["text"])
    ctx.text(px + panel_w / 2 - 90, py + 80,
             "Enter = restart   Esc = menu", size=12, color=C["subtext"])


@app.on_key
def on_key(key, _mods, _emit):
    if game.state == STATE_TITLE:
        if key == "Enter":
            game.start()

    elif game.state == STATE_PLAYING:
        if key in ("ArrowUp", "w", "W"):
            game.queue_direction(UP)
        elif key in ("ArrowDown", "s", "S"):
            game.queue_direction(DOWN)
        elif key in ("ArrowLeft", "a", "A"):
            game.queue_direction(LEFT)
        elif key in ("ArrowRight", "d", "D"):
            game.queue_direction(RIGHT)
        elif key == "p":
            game.state = STATE_PAUSED
        elif key == "Escape":
            game.state = STATE_TITLE

    elif game.state == STATE_PAUSED:
        if key == "p":
            game.state = STATE_PLAYING
            game._last_tick = time.monotonic()
        elif key == "Escape":
            game.state = STATE_TITLE

    elif game.state == STATE_GAMEOVER:
        if key == "Enter":
            game.start()
        elif key == "Escape":
            game.state = STATE_TITLE


app.run()
