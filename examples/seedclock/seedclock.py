#!/usr/bin/env python3
"""
seedclock — Plexi app
One-season farming game. Plant crops, manage weather, hit 50 coins before frost.

Controls:
  Arrow keys    Move cursor
  p             Plant seed at cursor
  h             Harvest ready crop
  m             Open/close market
  1-4           Buy seed in market (wheat/tomato/carrot/corn)
  Space         Advance day (debug)
  Escape        Close market / return to title
  Enter         Start / restart
"""
from __future__ import annotations

import os
import random
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from plexi_sdk import App

# ---------------------------------------------------------------------------
# Palette — Catppuccin Mocha
# ---------------------------------------------------------------------------
C = {
    "bg":       "#1e1e2e",
    "surface":  "#313244",
    "overlay":  "#45475a",
    "text":     "#cdd6f4",
    "subtext":  "#6c7086",
    "header":   "#181825",
    "green":    "#a6e3a1",
    "gold":     "#f9e2af",
    "red":      "#f38ba8",
    "orange":   "#fab387",
    "yellow":   "#f9e2af",
    "blue":     "#89b4fa",
    "mauve":    "#cba6f7",
    "seeded":   "#45475a",
    "growing":  "#a6e3a1",
}

# ---------------------------------------------------------------------------
# Crop definitions
# ---------------------------------------------------------------------------
CROPS = {
    "wheat":  {"days": 4, "price": 3, "seed_cost": 1, "color": "#f9e2af"},
    "tomato": {"days": 6, "price": 6, "seed_cost": 2, "color": "#f38ba8"},
    "carrot": {"days": 3, "price": 2, "seed_cost": 1, "color": "#fab387"},
    "corn":   {"days": 8, "price": 9, "seed_cost": 3, "color": "#f9e2af"},
}
CROP_ORDER = ["wheat", "tomato", "carrot", "corn"]

# Grid dimensions
GRID_COLS = 10
GRID_ROWS = 6
TOTAL_DAYS = 20
WIN_COINS = 50
DAY_SECONDS = 30.0  # real seconds per game day

# Growth stages
EMPTY   = 0
SEEDED  = 1
GROWING = 2
READY   = 3

# Weather
WEATHER_CLEAR   = "clear"
WEATHER_DROUGHT = "drought"
WEATHER_RAIN    = "rain"
WEATHER_FROST   = "frost"

WEATHER_COLORS = {
    WEATHER_CLEAR:   None,
    WEATHER_DROUGHT: "#fab387",
    WEATHER_RAIN:    "#89b4fa",
    WEATHER_FROST:   "#cba6f7",
}

WEATHER_LABELS = {
    WEATHER_DROUGHT: "DROUGHT — growth slowed 50%",
    WEATHER_RAIN:    "RAIN — growth sped up 50%",
    WEATHER_FROST:   "FROST WARNING — harvest ready crops soon!",
}

# States
STATE_TITLE   = "title"
STATE_PLAYING = "playing"
STATE_MARKET  = "market"
STATE_WIN     = "win"
STATE_LOSE    = "lose"


# ---------------------------------------------------------------------------
# Game model
# ---------------------------------------------------------------------------

class Cell:
    def __init__(self):
        self.stage: int = EMPTY
        self.crop: str | None = None
        self.days_planted: float = 0.0  # fractional days grown
        self.frost_timer: float = 0.0   # days until frost kills ready crop


class Game:
    def __init__(self):
        self.state = STATE_TITLE
        self.coins = 10
        self.day = 1
        self.cursor_x = 0
        self.cursor_y = 0
        self.selected_seed = "wheat"
        self.grid: list[list[Cell]] = [[Cell() for _ in range(GRID_COLS)] for _ in range(GRID_ROWS)]
        self.weather = WEATHER_CLEAR
        self.weather_days_left = 0
        self.next_weather_in = random.randint(3, 5)
        self.weather_banner_time = 0.0
        self._day_timer = 0.0
        self._last_tick = time.monotonic()

    # ---- Day progression ---------------------------------------------------

    def tick_time(self):
        """Called every render frame; advances fractional day timer."""
        now = time.monotonic()
        elapsed = now - self._last_tick
        self._last_tick = now

        speed = 1.0
        if self.weather == WEATHER_DROUGHT:
            speed = 0.5
        elif self.weather == WEATHER_RAIN:
            speed = 1.5

        day_fraction = (elapsed / DAY_SECONDS) * speed

        # Grow crops
        for row in self.grid:
            for cell in row:
                if cell.stage in (SEEDED, GROWING):
                    cell.days_planted += day_fraction
                    crop_days = CROPS[cell.crop]["days"]
                    if cell.stage == SEEDED and cell.days_planted >= 1.0:
                        cell.stage = GROWING
                    if cell.stage == GROWING and cell.days_planted >= crop_days:
                        cell.stage = READY
                        cell.frost_timer = 2.0  # frost grace period
                elif cell.stage == READY and self.weather == WEATHER_FROST:
                    cell.frost_timer -= day_fraction
                    if cell.frost_timer <= 0:
                        cell.stage = EMPTY
                        cell.crop = None
                        cell.days_planted = 0.0

        # Advance day counter (raw, unscaled by weather)
        self._day_timer += elapsed / DAY_SECONDS
        days_passed = int(self._day_timer)
        if days_passed > 0:
            self._day_timer -= days_passed
            self.advance_days(days_passed)

    def advance_days(self, count: int = 1):
        self.day += count
        self.next_weather_in -= count
        if self.weather_days_left > 0:
            self.weather_days_left -= count
            if self.weather_days_left <= 0:
                self.weather = WEATHER_CLEAR
        if self.next_weather_in <= 0:
            self._trigger_weather()
        if self.day > TOTAL_DAYS:
            self.state = STATE_LOSE

    def _trigger_weather(self):
        options = [WEATHER_DROUGHT, WEATHER_RAIN, WEATHER_FROST]
        self.weather = random.choice(options)
        self.weather_days_left = random.randint(2, 4)
        self.next_weather_in = random.randint(3, 5)
        self.weather_banner_time = time.monotonic()

    def advance_day_manual(self):
        self._last_tick = time.monotonic()
        self.advance_days(1)

    # ---- Actions -----------------------------------------------------------

    def plant(self):
        cell = self._cursor_cell()
        if cell.stage != EMPTY:
            return
        cost = CROPS[self.selected_seed]["seed_cost"]
        if self.coins < cost:
            return
        self.coins -= cost
        cell.stage = SEEDED
        cell.crop = self.selected_seed
        cell.days_planted = 0.0
        cell.frost_timer = 0.0

    def harvest(self):
        cell = self._cursor_cell()
        if cell.stage != READY:
            return
        self.coins += CROPS[cell.crop]["price"]
        cell.stage = EMPTY
        cell.crop = None
        cell.days_planted = 0.0
        if self.coins >= WIN_COINS:
            self.state = STATE_WIN

    def buy_seed(self, index: int):
        if index < 0 or index >= len(CROP_ORDER):
            return
        name = CROP_ORDER[index]
        cost = CROPS[name]["seed_cost"]
        if self.coins >= cost:
            self.coins -= cost
            self.selected_seed = name

    def _cursor_cell(self) -> Cell:
        return self.grid[self.cursor_y][self.cursor_x]

    def move_cursor(self, dx: int, dy: int):
        self.cursor_x = max(0, min(GRID_COLS - 1, self.cursor_x + dx))
        self.cursor_y = max(0, min(GRID_ROWS - 1, self.cursor_y + dy))


# ---------------------------------------------------------------------------
# App
# ---------------------------------------------------------------------------

game = Game()
app = App(app_id="seedclock")


@app.on_render
def render(ctx):
    if game.state in (STATE_PLAYING, STATE_MARKET):
        game.tick_time()

    ctx.rect(0, 0, ctx.width, ctx.height, fill=C["bg"])

    if game.state == STATE_TITLE:
        _render_title(ctx)
    elif game.state in (STATE_PLAYING, STATE_MARKET):
        _render_game(ctx)
        if game.state == STATE_MARKET:
            _render_market(ctx)
        _render_weather_banner(ctx)
    elif game.state == STATE_WIN:
        _render_game(ctx)
        _render_end(ctx, won=True)
    elif game.state == STATE_LOSE:
        _render_game(ctx)
        _render_end(ctx, won=False)


# ---------------------------------------------------------------------------
# Render helpers
# ---------------------------------------------------------------------------

HEADER_H = 40.0
CELL_PAD = 3.0


def _grid_layout(ctx):
    """Return (cell_w, cell_h, grid_x, grid_y)."""
    cell_w = ctx.width / GRID_COLS
    cell_h = (ctx.height - HEADER_H) / GRID_ROWS
    return cell_w, cell_h, 0.0, HEADER_H


def _render_header(ctx):
    ctx.rect(0, 0, ctx.width, HEADER_H, fill=C["header"])
    ctx.text(12, 10, f"Day {game.day}/{TOTAL_DAYS}", size=14, color=C["text"], bold=True)
    ctx.text(12, 24, f"Coins: {game.coins}c  |  Goal: {WIN_COINS}c", size=11, color=C["subtext"])
    seed_label = f"Seed: {game.selected_seed}  [m=market  p=plant  h=harvest  Space=+day]"
    ctx.text(ctx.width / 2 - len(seed_label) * 3.5, 14, seed_label, size=11, color=C["subtext"])


def _cell_color(cell: Cell) -> str:
    if cell.stage == EMPTY:
        return C["surface"]
    if cell.stage == SEEDED:
        return C["seeded"]
    if cell.stage == GROWING:
        return C["growing"]
    if cell.stage == READY:
        return CROPS[cell.crop]["color"]
    return C["surface"]


def _render_game(ctx):
    _render_header(ctx)
    cell_w, cell_h, gx, gy = _grid_layout(ctx)

    for row_i, row in enumerate(game.grid):
        for col_i, cell in enumerate(row):
            x = gx + col_i * cell_w + CELL_PAD
            y = gy + row_i * cell_h + CELL_PAD
            w = cell_w - CELL_PAD * 2
            h = cell_h - CELL_PAD * 2
            fill = _cell_color(cell)
            ctx.rect(x, y, w, h, fill=fill, radius=4.0)

            # Growth progress bar for seeded/growing
            if cell.stage in (SEEDED, GROWING):
                crop_days = CROPS[cell.crop]["days"]
                pct = min(1.0, cell.days_planted / crop_days)
                bar_h = 3.0
                ctx.rect(x, y + h - bar_h, w * pct, bar_h, fill=C["green"])

            # Stage indicators
            if cell.stage == SEEDED:
                ctx.text(x + w / 2 - 4, y + h / 2 - 6, "·", size=14, color=C["text"])
            elif cell.stage == READY:
                ctx.text(x + 4, y + 4, "!", size=11, color=C["header"], bold=True)

    # Cursor highlight
    cx = gx + game.cursor_x * cell_w + CELL_PAD
    cy = gy + game.cursor_y * cell_h + CELL_PAD
    cw = cell_w - CELL_PAD * 2
    ch = cell_h - CELL_PAD * 2
    ctx.rect(cx, cy, cw, ch, fill=C["blue"], radius=4.0)
    cursor_cell = game.grid[game.cursor_y][game.cursor_x]
    fill = _cell_color(cursor_cell)
    ctx.rect(cx + 3, cy + 3, cw - 6, ch - 6, fill=fill, radius=2.0)
    # Redraw progress bar on cursor cell
    if cursor_cell.stage in (SEEDED, GROWING):
        crop_days = CROPS[cursor_cell.crop]["days"]
        pct = min(1.0, cursor_cell.days_planted / crop_days)
        ctx.rect(cx + 3, cy + ch - 5, (cw - 6) * pct, 3.0, fill=C["green"])
    if cursor_cell.stage == READY:
        ctx.text(cx + 7, cy + 7, "!", size=11, color=C["header"], bold=True)


def _render_weather_banner(ctx):
    now = time.monotonic()
    if game.weather == WEATHER_CLEAR:
        return
    since = now - game.weather_banner_time
    if since > 5.0:
        return
    label = WEATHER_LABELS.get(game.weather, "")
    color = WEATHER_COLORS.get(game.weather, C["text"])
    bw = min(ctx.width * 0.6, 400.0)
    bx = (ctx.width - bw) / 2
    ctx.rect(bx, HEADER_H + 8, bw, 28, fill=C["header"], radius=6.0)
    ctx.text(bx + 10, HEADER_H + 14, label, size=12, color=color, bold=True)


def _render_market(ctx):
    panel_w = 320.0
    panel_h = 220.0
    px = (ctx.width - panel_w) / 2
    py = (ctx.height - panel_h) / 2
    ctx.rect(px, py, panel_w, panel_h, fill=C["header"], radius=10.0)
    ctx.text(px + panel_w / 2 - 36, py + 12, "MARKET", size=16, color=C["gold"], bold=True)
    ctx.text(px + 12, py + 38, f"Coins: {game.coins}c", size=12, color=C["text"])

    for i, name in enumerate(CROP_ORDER):
        c = CROPS[name]
        row_y = py + 65 + i * 36
        ctx.rect(px + 10, row_y, panel_w - 20, 30, fill=C["surface"], radius=4.0)
        ctx.text(px + 16, row_y + 7, f"[{i+1}]", size=12, color=C["subtext"], monospace=True)
        ctx.text(px + 48, row_y + 7, name.capitalize(), size=12, color=C["text"], bold=True)
        detail = f"seed={c['seed_cost']}c  sell={c['price']}c  {c['days']}d"
        ctx.text(px + 130, row_y + 9, detail, size=10, color=C["subtext"])
        ctx.rect(px + panel_w - 28, row_y + 7, 14, 14, fill=c["color"], radius=3.0)

    ctx.text(px + panel_w / 2 - 56, py + panel_h - 22,
             "Esc to close market", size=11, color=C["subtext"])


def _render_title(ctx):
    cx = ctx.width / 2
    cy = ctx.height / 2
    ctx.text(cx - 80, cy - 60, "SEEDCLOCK", size=28, color=C["gold"], bold=True)
    ctx.text(cx - 110, cy - 20,
             "Race to 50 coins before the frost.", size=13, color=C["text"])
    ctx.text(cx - 130, cy + 14,
             "Arrows=move  p=plant  h=harvest  m=market", size=11, color=C["subtext"])
    ctx.text(cx - 110, cy + 34,
             "Space=+day  1-4=select seed  Esc=quit", size=11, color=C["subtext"])
    ctx.text(cx - 68, cy + 70, "Press Enter to start", size=13, color=C["blue"])


def _render_end(ctx, won: bool):
    panel_w = 300.0
    panel_h = 140.0
    px = (ctx.width - panel_w) / 2
    py = (ctx.height - panel_h) / 2
    ctx.rect(px, py, panel_w, panel_h, fill=C["header"], radius=10.0)
    if won:
        ctx.text(px + panel_w / 2 - 36, py + 14, "YOU WIN!", size=20, color=C["green"], bold=True)
        ctx.text(px + panel_w / 2 - 80, py + 50,
                 f"Reached {game.coins} coins by day {game.day}.", size=12, color=C["text"])
    else:
        ctx.text(px + panel_w / 2 - 48, py + 14, "FROST WINS", size=20, color=C["mauve"], bold=True)
        ctx.text(px + panel_w / 2 - 76, py + 50,
                 f"Season ended with {game.coins} coins.", size=12, color=C["text"])
    ctx.text(px + panel_w / 2 - 94, py + 100,
             "Enter = play again   Esc = title", size=12, color=C["subtext"])


# ---------------------------------------------------------------------------
# Input
# ---------------------------------------------------------------------------

@app.on_key
def on_key(key, _mods, _emit):
    if game.state == STATE_TITLE:
        if key == "Enter":
            game.__init__()
            game.state = STATE_PLAYING

    elif game.state == STATE_PLAYING:
        if key == "ArrowUp":
            game.move_cursor(0, -1)
        elif key == "ArrowDown":
            game.move_cursor(0, 1)
        elif key == "ArrowLeft":
            game.move_cursor(-1, 0)
        elif key == "ArrowRight":
            game.move_cursor(1, 0)
        elif key == "p":
            game.plant()
        elif key == "h":
            game.harvest()
        elif key == "m":
            game.state = STATE_MARKET
        elif key == " ":
            game.advance_day_manual()
        elif key == "Escape":
            game.state = STATE_TITLE
        elif key in ("1", "2", "3", "4"):
            game.selected_seed = CROP_ORDER[int(key) - 1]

    elif game.state == STATE_MARKET:
        if key == "Escape":
            game.state = STATE_PLAYING
        elif key in ("1", "2", "3", "4"):
            game.buy_seed(int(key) - 1)
            game.state = STATE_PLAYING

    elif game.state in (STATE_WIN, STATE_LOSE):
        if key == "Enter":
            game.__init__()
            game.state = STATE_PLAYING
        elif key == "Escape":
            game.__init__()
            game.state = STATE_TITLE


app.run()
