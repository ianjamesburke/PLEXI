from __future__ import annotations
#!/usr/bin/env python3
"""apiary — Plexi beehive management simulation.

Migrated to the Phase 1 SDK components layer: THEME palette, named size
constants, ctx.header, ctx.status_bar, text_center/text_right. Honey-specific
cell colors (brood/honey/pollen/queen/infected/cold) aren't part of THEME and
stay as an app-local palette.
"""

import math
import os
import random
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from plexi_sdk import (
    App,
    THEME,
    TITLE, HEADING, BODY, CAPTION, HINT,
    PAD, HEADER_H,
)

# App-local cell palette — these hues aren't in THEME and are semantic to the
# beehive sim, so we keep them as plain constants rather than a dict lookup.
CELL_EMPTY    = THEME.surface
CELL_BROOD    = "#f5f5e8"
CELL_HONEY    = "#f9a825"
CELL_POLLEN   = "#fab387"
CELL_QUEEN    = "#cba6f7"
CELL_INFECTED = "#f38ba8"
CELL_COLD     = "#45475a"
CURSOR_COLOR  = "#ffffff"

COLS = 6
ROWS = 8
TICK_INTERVAL = 2.0   # real seconds per auto-tick
WIN_JARS = 30
MAX_TICKS = 20

EMPTY    = "empty"
BROOD    = "brood"
HONEY    = "honey"
POLLEN   = "pollen"
QUEEN    = "queen"
INFECTED = "infected"
COLD     = "cold"

STATE_PLAYING = "playing"
STATE_WIN     = "win"
STATE_LOSE    = "lose"


class Hive:
    def __init__(self) -> None:
        self.cells: list[list[str]] = [[EMPTY] * COLS for _ in range(ROWS)]
        self.brood_age: list[list[int]] = [[0] * COLS for _ in range(ROWS)]
        self.queen_pos: tuple[int, int] = (ROWS // 2, COLS // 2)
        self.cursor: tuple[int, int] = (ROWS // 2, COLS // 2)
        self.honey_jars: int = 0
        self.colony_size: int = 20
        self.queen_health: int = 100
        self.supers: int = 1
        self.tick: int = 0
        self.state: str = STATE_PLAYING
        self._last_tick_time: float = time.monotonic()
        self._forager_phase: float = 0.0
        self._forager_x: float = 0.0
        self._forager_active: bool = False
        self._forager_timer: float = 0.0
        self._status_msg: str = ""
        self._status_timer: float = 0.0
        self._setup_initial()

    def _setup_initial(self) -> None:
        # Place queen at center
        qr, qc = self.queen_pos
        self.cells[qr][qc] = QUEEN
        # Seed some honey and brood
        for r, c in [(2, 2), (2, 3), (3, 1), (3, 4)]:
            self.cells[r][c] = HONEY
        for r, c in [(4, 2), (4, 3), (5, 2)]:
            self.cells[r][c] = BROOD
        for r, c in [(1, 1), (1, 4)]:
            self.cells[r][c] = POLLEN

    def max_honey_cells(self) -> int:
        return self.supers * 8

    def honey_count(self) -> int:
        count = 0
        for row in self.cells:
            for cell in row:
                if cell == HONEY:
                    count += 1
        return count

    def do_tick(self) -> None:
        if self.state != STATE_PLAYING:
            return
        self.tick += 1
        self._simulate()
        self._check_end()

    def _is_outer(self, r: int, c: int) -> bool:
        return r == 0 or r == ROWS - 1 or c == 0 or c == COLS - 1

    def _simulate(self) -> None:
        is_winter = self.tick >= 15
        honey_cap = self.max_honey_cells()
        current_honey = self.honey_count()

        # Queen health degrades slightly each tick
        qr, qc = self.queen_pos
        if self.cells[qr][qc] != QUEEN:
            self.queen_health = max(0, self.queen_health - 10)
        else:
            self.queen_health = min(100, self.queen_health + 2)

        new_cells = [row[:] for row in self.cells]

        for r in range(ROWS):
            for c in range(COLS):
                cell = self.cells[r][c]

                if cell == QUEEN:
                    continue

                # Winter cold spreads to outer cells
                if is_winter and self._is_outer(r, c) and cell not in (QUEEN,):
                    new_cells[r][c] = COLD
                    continue

                if cell == BROOD:
                    self.brood_age[r][c] += 1
                    if self.brood_age[r][c] >= 4:
                        new_cells[r][c] = EMPTY
                        self.brood_age[r][c] = 0
                        self.colony_size += 1

                elif cell == HONEY:
                    # 5% infection chance
                    if random.random() < 0.05:
                        new_cells[r][c] = INFECTED

                elif cell == EMPTY or cell == COLD:
                    # Workers forage: fill empty cells with honey if queen healthy
                    if (not is_winter and cell == EMPTY
                            and self.queen_health > 30
                            and current_honey < honey_cap
                            and random.random() < 0.15):
                        new_cells[r][c] = HONEY
                        current_honey += 1
                    # Brood laying near queen
                    elif (cell == EMPTY
                          and self.queen_health > 50
                          and random.random() < 0.08):
                        qr2, qc2 = self.queen_pos
                        dist = abs(r - qr2) + abs(c - qc2)
                        if dist <= 2:
                            new_cells[r][c] = BROOD
                            self.brood_age[r][c] = 0

        self.cells = new_cells
        # Trigger forager animation
        self._forager_active = True
        self._forager_timer = 0.0

    def _check_end(self) -> None:
        if self.honey_jars >= WIN_JARS:
            self.state = STATE_WIN
        elif self.tick >= MAX_TICKS:
            self.state = STATE_LOSE

    def move_cursor(self, dr: int, dc: int) -> None:
        r, c = self.cursor
        self.cursor = (max(0, min(ROWS - 1, r + dr)),
                       max(0, min(COLS - 1, c + dc)))

    def place_queen(self) -> None:
        r, c = self.cursor
        if self.cells[r][c] in (INFECTED, COLD):
            self._msg("Can't place queen there")
            return
        qr, qc = self.queen_pos
        self.cells[qr][qc] = EMPTY
        self.queen_pos = (r, c)
        self.cells[r][c] = QUEEN

    def harvest(self) -> None:
        r, c = self.cursor
        if self.cells[r][c] == HONEY:
            self.cells[r][c] = EMPTY
            self.honey_jars += 1
            self._msg(f"+1 jar! Total: {self.honey_jars}")
        else:
            self._msg("No honey here")

    def treat(self) -> None:
        r, c = self.cursor
        if self.cells[r][c] == INFECTED:
            if self.honey_jars >= 2:
                self.honey_jars -= 2
                self.cells[r][c] = EMPTY
                self._msg("Cell treated (-2 jars)")
            else:
                self._msg("Need 2 jars to treat")
        else:
            self._msg("No infection here")

    def add_super(self) -> None:
        self.supers = min(5, self.supers + 1)
        self._msg(f"Supers: {self.supers}")

    def remove_super(self) -> None:
        self.supers = max(1, self.supers - 1)
        self._msg(f"Supers: {self.supers}")

    def _msg(self, text: str) -> None:
        self._status_msg = text
        self._status_timer = 2.5

    def update(self, now: float, dt: float) -> None:
        if self.state != STATE_PLAYING:
            return
        elapsed = now - self._last_tick_time
        if elapsed >= TICK_INTERVAL:
            self._last_tick_time = now
            self.do_tick()
        # Status message decay
        if self._status_timer > 0:
            self._status_timer = max(0.0, self._status_timer - dt)
        # Forager animation
        if self._forager_active:
            self._forager_timer += dt
            if self._forager_timer > 1.2:
                self._forager_active = False

hive = Hive()
app = App(app_id="apiary")
_start = time.monotonic()
_last_t = _start

SIDEBAR_W = 180.0
CELL_W = 52.0
CELL_H = 40.0
H_OFFSET = CELL_W * 0.5

# Layout chrome — header at top, status bar at bottom, sidebar on the right.
STATUS_BAR_H = 30.0


def _play_area() -> tuple[float, float]:
    """Vertical bounds of the playfield (between header and status bar)."""
    top = HEADER_H
    bot_h = STATUS_BAR_H
    return top, bot_h


def grid_origin(ctx_w: float, ctx_h: float) -> tuple[float, float]:
    top, bot_h = _play_area()
    grid_w = COLS * CELL_W + H_OFFSET
    grid_h = ROWS * CELL_H
    avail_w = ctx_w - SIDEBAR_W
    avail_h = ctx_h - top - bot_h
    ox = (avail_w - grid_w) / 2.0 + PAD / 2.0
    oy = top + (avail_h - grid_h) / 2.0
    return ox, oy


def cell_xy(r: int, c: int, ox: float, oy: float) -> tuple[float, float]:
    offset = H_OFFSET if r % 2 == 1 else 0.0
    return ox + c * CELL_W + offset, oy + r * CELL_H


CELL_COLORS = {
    EMPTY: CELL_EMPTY, BROOD: CELL_BROOD, HONEY: CELL_HONEY,
    POLLEN: CELL_POLLEN, QUEEN: CELL_QUEEN,
    INFECTED: CELL_INFECTED, COLD: CELL_COLD,
}


@app.on_render
def render(ctx) -> None:
    global _last_t
    now = time.monotonic() - _start
    wall_now = time.monotonic()
    dt = min(0.1, wall_now - _last_t)
    _last_t = wall_now

    hive.update(now, dt)

    ctx.rect(0, 0, ctx.width, ctx.height, fill=THEME.bg)

    if hive.state in (STATE_WIN, STATE_LOSE):
        _render_endscreen(ctx)
        return

    # Header with title + current season day.
    season_day = hive.tick + 1
    ctx.header(
        f"Apiary  ·  tick {season_day}/{MAX_TICKS}",
        subtitle=f"{hive.honey_jars}/{WIN_JARS} jars  ·  {hive.colony_size} bees",
    )

    _render_hive(ctx, now)
    _render_sidebar(ctx, now)
    _render_foragers(ctx, now)

    # Status bar — replaces the old inline sidebar controls + status popup.
    show_msg = bool(hive._status_msg and hive._status_timer > 0)
    ctx.status_bar(
        [
            ("←↑↓→", "move"),
            ("q", "queen"),
            ("h", "harvest"),
            ("t", "treat"),
            ("+/-", "supers"),
            ("space", "tick"),
            ("⌘W", "close"),
        ],
        status_msg=hive._status_msg if show_msg else None,
        status_color=THEME.yellow if show_msg else None,
        height=STATUS_BAR_H,
    )


def _render_hive(ctx, now: float) -> None:
    ox, oy = grid_origin(ctx.width, ctx.height)
    cr, cc = hive.cursor
    pad = 3.0
    for r in range(ROWS):
        for c in range(COLS):
            x, y = cell_xy(r, c, ox, oy)
            state = hive.cells[r][c]
            fill = CELL_COLORS.get(state, CELL_EMPTY)

            if state == INFECTED:
                p = 0.7 + 0.3 * math.sin(now * 5.0)
                fill = f"#{int(0xf3*p):02x}{int(0x8b*p):02x}{int(0xa8*p):02x}"
            cw, ch = CELL_W - pad * 2, CELL_H - pad * 2
            if r == cr and c == cc:
                ctx.rect(x + pad - 2.5, y + pad - 2.5, cw + 5, ch + 5,
                         fill=CURSOR_COLOR, radius=8.0)
            ctx.rect(x + pad, y + pad, cw, ch, fill=fill, radius=6.0)

            # Label inside cell
            label = {
                BROOD:    "B",
                HONEY:    "H",
                POLLEN:   "P",
                QUEEN:    "Q",
                INFECTED: "!",
                COLD:     "~",
            }.get(state, "")
            if label:
                lx = x + CELL_W / 2.0 - 5.0
                ly = y + CELL_H / 2.0 - 7.0
                txt_color = (THEME.bg if state in (HONEY, BROOD, POLLEN, QUEEN)
                             else THEME.fg)
                ctx.text(lx, ly, label, size=CAPTION, color=txt_color, bold=True)


def _render_sidebar(ctx, now: float) -> None:
    top, bot_h = _play_area()
    sx = ctx.width - SIDEBAR_W
    sy = top
    sh = ctx.height - top - bot_h
    ctx.rect(sx, sy, SIDEBAR_W, sh, fill=THEME.surface, radius=0.0)

    y = sy + PAD
    ctx.text(sx + PAD, y, "STATS", size=CAPTION, color=THEME.accent, bold=True)
    y += 24

    def row(label: str, val: str, color: str = THEME.fg) -> None:
        nonlocal y
        ctx.text(sx + PAD, y, label, size=HINT, color=THEME.muted)
        ctx.text(sx + PAD, y + 14, val, size=CAPTION, color=color, bold=True)
        y += 36

    row("HONEY JARS", f"{hive.honey_jars} / {WIN_JARS}",
        color=CELL_HONEY if hive.honey_jars > 0 else THEME.muted)
    row("COLONY SIZE", str(hive.colony_size))

    # Queen health bar
    ctx.text(sx + PAD, y, "QUEEN HEALTH", size=HINT, color=THEME.muted)
    y += 14
    bar_w = SIDEBAR_W - PAD * 2
    ctx.rect(sx + PAD, y, bar_w, 8, fill=THEME.highlight, radius=4.0)
    qh_ratio = hive.queen_health / 100.0
    qh_color = (THEME.green if qh_ratio > 0.5
                else (CELL_HONEY if qh_ratio > 0.25 else THEME.red))
    ctx.rect(sx + PAD, y, bar_w * qh_ratio, 8, fill=qh_color, radius=4.0)
    y += 16
    ctx.text_right(sx + SIDEBAR_W - PAD, y, f"{hive.queen_health}%",
                   size=HINT, color=THEME.muted)
    y += 24

    # Supers
    ctx.text(sx + PAD, y, "SUPERS", size=HINT, color=THEME.muted)
    y += 14
    for i in range(5):
        box_x = sx + PAD + i * 22
        box_fill = CELL_HONEY if i < hive.supers else THEME.highlight
        ctx.rect(box_x, y, 18, 14, fill=box_fill, radius=3.0)
    y += 28

    # Honey capacity
    cap = hive.max_honey_cells()
    cur_h = hive.honey_count()
    ctx.text(sx + PAD, y, f"HONEY CELLS", size=HINT, color=THEME.muted)
    ctx.text_right(sx + SIDEBAR_W - PAD, y, f"{cur_h}/{cap}",
                   size=HINT, color=THEME.fg)


def _render_foragers(ctx, now: float) -> None:
    if not hive._forager_active:
        return
    ox, oy = grid_origin(ctx.width, ctx.height)
    # Entrance is bottom-center of hive
    grid_w = COLS * CELL_W + H_OFFSET
    entrance_x = ox + grid_w / 2.0
    entrance_y = oy + ROWS * CELL_H + 6.0

    progress = hive._forager_timer / 1.2
    # Draw 3 animated dots moving from entrance outward
    for i in range(3):
        t = (progress + i * 0.33) % 1.0
        dot_x = entrance_x + math.sin(t * math.pi * 2 + i) * 20.0 * t
        dot_y = entrance_y + 10.0 * t
        alpha = max(0, 1.0 - t)
        r_v = int(0xf9 * alpha)
        g_v = int(0xa8 * alpha)
        b_v = int(0x25 * alpha)
        dot_color = f"#{r_v:02x}{g_v:02x}{b_v:02x}"
        ctx.rect(dot_x - 3, dot_y - 3, 6, 6, fill=dot_color, radius=3.0)


def _render_endscreen(ctx) -> None:
    ctx.rect(0, 0, ctx.width, ctx.height, fill=THEME.bg)
    cx = ctx.width / 2.0
    cy = ctx.height / 2.0

    if hive.state == STATE_WIN:
        ctx.text_center(cx, cy - 50, "YOU WIN!", size=TITLE + 6,
                        color=CELL_HONEY, bold=True)
        msg = f"Harvested {hive.honey_jars} jars in {hive.tick} ticks"
    else:
        ctx.text_center(cx, cy - 50, "SEASON OVER", size=TITLE + 2,
                        color=THEME.red, bold=True)
        msg = f"Only {hive.honey_jars} / {WIN_JARS} jars harvested"

    ctx.text_center(cx, cy, msg, size=BODY, color=THEME.fg)
    ctx.text_center(cx, cy + 40, "Press R to restart",
                    size=CAPTION, color=THEME.muted)

    ctx.status_bar(
        [("R", "restart"), ("⌘W", "close")],
        height=STATUS_BAR_H,
    )


# ---------------------------------------------------------------------------
# Key handler
# ---------------------------------------------------------------------------

@app.on_key
def on_key(key: str, _mods: dict, _emit) -> None:
    if hive.state in (STATE_WIN, STATE_LOSE):
        if key in ("r", "R"):
            _restart()
        return

    if key == "ArrowUp":
        hive.move_cursor(-1, 0)
    elif key == "ArrowDown":
        hive.move_cursor(1, 0)
    elif key == "ArrowLeft":
        hive.move_cursor(0, -1)
    elif key == "ArrowRight":
        hive.move_cursor(0, 1)
    elif key == "q":
        hive.place_queen()
    elif key == "h":
        hive.harvest()
    elif key == "t":
        hive.treat()
    elif key in ("+", "="):
        hive.add_super()
    elif key == "-":
        hive.remove_super()
    elif key == " ":
        hive._last_tick_time = 0.0  # force next update to tick


def _restart() -> None:
    global hive
    hive = Hive()


app.run()
