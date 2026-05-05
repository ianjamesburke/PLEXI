#!/usr/bin/env python3
"""Tetris — full game using DrawCommand::ScheduleRender for a 60 fps loop.

Demonstrates: time-based game logic in on_render, ghost piece, wall kicks,
hard drop, level progression, and the ScheduleRender protocol extension.
"""

import random
import sys
import os
sys.path.insert(0, os.path.join(os.path.dirname(__file__), '../../sdk/python'))

import time
from plexi_sdk import App, RenderContext

ROWS, COLS = 20, 10

# Standard SRS tetromino shapes: list of 4 rotations, each a list of (row, col) offsets.
PIECES: dict[str, dict] = {
    "I": {
        "color": "#00f0f0",
        "shapes": [
            [(1, 0), (1, 1), (1, 2), (1, 3)],
            [(0, 2), (1, 2), (2, 2), (3, 2)],
            [(2, 0), (2, 1), (2, 2), (2, 3)],
            [(0, 1), (1, 1), (2, 1), (3, 1)],
        ],
    },
    "O": {
        "color": "#f0f000",
        "shapes": [[(0, 0), (0, 1), (1, 0), (1, 1)]] * 4,
    },
    "T": {
        "color": "#a000f0",
        "shapes": [
            [(0, 1), (1, 0), (1, 1), (1, 2)],
            [(0, 1), (1, 1), (1, 2), (2, 1)],
            [(1, 0), (1, 1), (1, 2), (2, 1)],
            [(0, 1), (1, 0), (1, 1), (2, 1)],
        ],
    },
    "S": {
        "color": "#00f000",
        "shapes": [
            [(0, 1), (0, 2), (1, 0), (1, 1)],
            [(0, 1), (1, 1), (1, 2), (2, 2)],
            [(1, 1), (1, 2), (2, 0), (2, 1)],
            [(0, 0), (1, 0), (1, 1), (2, 1)],
        ],
    },
    "Z": {
        "color": "#f00000",
        "shapes": [
            [(0, 0), (0, 1), (1, 1), (1, 2)],
            [(0, 2), (1, 1), (1, 2), (2, 1)],
            [(1, 0), (1, 1), (2, 1), (2, 2)],
            [(0, 1), (1, 0), (1, 1), (2, 0)],
        ],
    },
    "J": {
        "color": "#0000f0",
        "shapes": [
            [(0, 0), (1, 0), (1, 1), (1, 2)],
            [(0, 1), (0, 2), (1, 1), (2, 1)],
            [(1, 0), (1, 1), (1, 2), (2, 2)],
            [(0, 1), (1, 1), (2, 0), (2, 1)],
        ],
    },
    "L": {
        "color": "#f0a000",
        "shapes": [
            [(0, 2), (1, 0), (1, 1), (1, 2)],
            [(0, 1), (1, 1), (2, 1), (2, 2)],
            [(1, 0), (1, 1), (1, 2), (2, 0)],
            [(0, 0), (0, 1), (1, 1), (2, 1)],
        ],
    },
}

PIECE_KEYS = list(PIECES.keys())

CLEAR_FLASH_DURATION = 0.35  # seconds — rows flash before collapsing

# Wall-kick offsets tried in order for non-I pieces
KICKS = [(0, 0), (0, -1), (0, 1), (0, -2), (0, 2)]
# I-piece has wider kick table
KICKS_I = [(0, 0), (0, -2), (0, 1), (0, -2), (0, 1)]


def _drop_interval(level: int) -> float:
    return max(0.05, 1.0 - (level - 1) * 0.085)


def _score_for_lines(n: int, level: int) -> int:
    return ([0, 100, 300, 500, 800])[n] * level


class Piece:
    __slots__ = ("key", "rot", "row", "col")

    def __init__(self, key: str, rot: int = 0, row: int = 0, col: int = 3):
        self.key = key
        self.rot = rot
        self.row = row
        self.col = col

    def copy(self) -> "Piece":
        return Piece(self.key, self.rot, self.row, self.col)

    def cells(self) -> list[tuple[int, int]]:
        shape = PIECES[self.key]["shapes"][self.rot]
        return [(self.row + r, self.col + c) for r, c in shape]

    def color(self) -> str:
        return PIECES[self.key]["color"]


class TetrisApp(App):
    def on_init(self, ctx: RenderContext) -> None:
        self._new_game()

    def _new_game(self) -> None:
        self.board: list[list[str | None]] = [[None] * COLS for _ in range(ROWS)]
        self.score = 0
        self.lines = 0
        self.level = 1
        self.game_over = False
        self.paused = False
        self.current = self._spawn()
        self.next = self._spawn()
        self.last_drop = time.time()
        self.lock_delay: float | None = None  # time when current piece should lock
        self.clear_anim: tuple[list[int], float] | None = None  # (rows, start_time)

    def _spawn(self) -> Piece:
        return Piece(random.choice(PIECE_KEYS), row=0, col=3)

    # ── Collision ──────────────────────────────────────────────────────────────

    def _valid(self, p: Piece) -> bool:
        for r, c in p.cells():
            if c < 0 or c >= COLS or r >= ROWS:
                return False
            if r >= 0 and self.board[r][c] is not None:
                return False
        return True

    def _ghost(self) -> Piece:
        g = self.current.copy()
        while True:
            g.row += 1
            if not self._valid(g):
                g.row -= 1
                return g

    # ── Piece lifecycle ────────────────────────────────────────────────────────

    def _try_rotate(self) -> None:
        p = self.current.copy()
        p.rot = (p.rot + 1) % 4
        kicks = KICKS_I if p.key == "I" else KICKS
        for dr, dc in kicks:
            p.row += dr
            p.col += dc
            if self._valid(p):
                self.current = p
                return
            p.row -= dr
            p.col -= dc

    def _lock(self) -> None:
        color = self.current.color()
        for r, c in self.current.cells():
            if r >= 0:
                self.board[r][c] = color
        self.lock_delay = None
        full = [r for r in range(ROWS) if all(cell is not None for cell in self.board[r])]
        if full:
            self.clear_anim = (full, time.time())
            # Piece advance deferred until flash completes in _finish_clear
        else:
            self._advance_piece()

    def _finish_clear(self, rows: list[int]) -> None:
        n = len(rows)
        for r in sorted(rows, reverse=True):
            del self.board[r]
            self.board.insert(0, [None] * COLS)
        self.score += _score_for_lines(n, self.level)
        self.lines += n
        self.level = self.lines // 10 + 1
        self._advance_piece()

    def _advance_piece(self) -> None:
        self.current = self.next
        self.next = self._spawn()
        self.last_drop = time.time()
        if not self._valid(self.current):
            self.game_over = True

    def _hard_drop(self) -> None:
        g = self._ghost()
        self.score += max(0, g.row - self.current.row) * 2
        self.current = g
        self._lock()

    def _soft_drop(self) -> None:
        moved = self.current.copy()
        moved.row += 1
        if self._valid(moved):
            self.current = moved
            self.score += 1
            self.last_drop = time.time()
        else:
            self._lock()

    # ── Rendering ─────────────────────────────────────────────────────────────

    def on_render(self, ctx: RenderContext) -> None:
        now = time.time()

        # Tick line-clear animation
        if self.clear_anim is not None:
            rows, anim_start = self.clear_anim
            if now - anim_start >= CLEAR_FLASH_DURATION:
                self.clear_anim = None
                self._finish_clear(rows)

        if not self.game_over and not self.paused and self.clear_anim is None:
            # Gravity
            if now - self.last_drop >= _drop_interval(self.level):
                moved = self.current.copy()
                moved.row += 1
                if self._valid(moved):
                    self.current = moved
                    self.last_drop = now
                else:
                    self._lock()

        # Layout: board on left, info panel on right
        cell = min(ctx.w * 0.58 / COLS, ctx.h * 0.92 / ROWS)
        board_w = cell * COLS
        board_h = cell * ROWS
        margin = cell * 0.5
        bx = margin
        by = (ctx.h - board_h) / 2

        # Background
        ctx.rect(0, 0, ctx.w, ctx.h, fill="#0d0d1a")

        # Board background
        ctx.rect(bx - 1, by - 1, board_w + 2, board_h + 2, fill="#111122", radius=2.0)

        # Subtle grid
        for row in range(ROWS + 1):
            ctx.line(bx, by + row * cell, bx + board_w, by + row * cell,
                     color="#1a1a33", width=0.5)
        for col in range(COLS + 1):
            ctx.line(bx + col * cell, by, bx + col * cell, by + board_h,
                     color="#1a1a33", width=0.5)

        # Placed cells
        clearing_rows: set[int] = set()
        flash_on = False
        if self.clear_anim is not None:
            rows, anim_start = self.clear_anim
            clearing_rows = set(rows)
            t = (now - anim_start) / CLEAR_FLASH_DURATION
            flash_on = (int(t * 8) % 2) == 0  # 4 white/dark cycles

        for row in range(ROWS):
            for col in range(COLS):
                if row in clearing_rows:
                    if flash_on:
                        self._cell(ctx, bx + col * cell, by + row * cell, cell, "#ffffff")
                    # else: leave dark (show grid background) for blink effect
                elif color := self.board[row][col]:
                    self._cell(ctx, bx + col * cell, by + row * cell, cell, color)

        if not self.game_over and self.clear_anim is None:
            # Ghost piece (dim outline)
            ghost = self._ghost()
            gc = self.current.color()
            for r, c in ghost.cells():
                if r >= 0:
                    x = bx + c * cell + 1
                    y = by + r * cell + 1
                    s = cell - 2
                    ctx.rect(x, y, s, s, fill=gc + "28", radius=2.0)

            # Active piece
            for r, c in self.current.cells():
                if r >= 0:
                    self._cell(ctx, bx + c * cell, by + r * cell, cell,
                               self.current.color())

        # ── Info panel ────────────────────────────────────────────────────────
        px = bx + board_w + cell
        py = by

        ctx.text(px, py, "TETRIS", size=22, color="#89b4fa", bold=True)
        py += 36

        ctx.text(px, py, "SCORE", size=10, color="#6c7086")
        py += 16
        ctx.text(px, py, f"{self.score:,}", size=20, color="#cdd6f4", bold=True)
        py += 32

        ctx.text(px, py, "LINES", size=10, color="#6c7086")
        py += 16
        ctx.text(px, py, str(self.lines), size=16, color="#cdd6f4")
        py += 28

        ctx.text(px, py, "LEVEL", size=10, color="#6c7086")
        py += 16
        ctx.text(px, py, str(self.level), size=16, color="#cdd6f4")
        py += 36

        ctx.text(px, py, "NEXT", size=10, color="#6c7086")
        py += 16
        nc = self.next.color()
        ps = cell * 0.72
        for r, c in PIECES[self.next.key]["shapes"][0]:
            self._cell(ctx, px + c * ps, py + r * ps, ps, nc)
        py += ps * 4 + 24

        # Controls
        for line in ["← → / H L  move", "↑ / K  rotate", "↓ / J  soft drop",
                     "SPC/↵ hard drop", "P     pause", "R     restart"]:
            ctx.text(px, py, line, size=10, color="#45475a")
            py += 14

        # Overlays
        if self.game_over:
            ow = board_w
            oh = 80.0
            ox = bx
            oy = by + (board_h - oh) / 2
            ctx.rect(ox, oy, ow, oh, fill="#000000cc", radius=4.0)
            ctx.text(ox + ow / 2 - 52, oy + 16, "GAME OVER",
                     size=22, color="#f38ba8", bold=True)
            ctx.text(ox + ow / 2 - 42, oy + 46, "R to restart",
                     size=13, color="#cdd6f4")

        elif self.paused:
            ow = board_w
            oh = 50.0
            ctx.rect(bx, by + (board_h - oh) / 2, ow, oh, fill="#000000cc", radius=4.0)
            ctx.text(bx + ow / 2 - 28, by + (board_h - oh) / 2 + 14,
                     "PAUSED", size=18, color="#f9e2af", bold=True)

        # Controls hint below board
        hint_y = by + board_h + 6
        ctx.text(bx, hint_y, "← → / H L move  ↑ / K rotate  ↓ / J soft  SPC/↵ hard  P pause",
                 size=10, color="#313244")

        # Drive the game loop — ~60 fps when active or animating, ~10 fps when paused/over.
        active = not self.game_over and (not self.paused or self.clear_anim is not None)
        ctx.emit.schedule_render(16 if active else 100)

    def _cell(self, ctx: RenderContext, x: float, y: float, size: float,
               color: str) -> None:
        s = size - 2
        ctx.rect(x + 1, y + 1, s, s, fill=color, radius=2.0)
        # Highlight strip for pseudo-3D look
        ctx.rect(x + 2, y + 2, s - 2, 3, fill="#ffffff35", radius=0.0)

    # ── Input ─────────────────────────────────────────────────────────────────

    def on_key(self, ctx: RenderContext, key: str, mods: dict) -> None:
        # TODO: add sound effects once AudioPlay is exposed in the Python SDK
        #   - line clear: short synth chord (pitch scales with number of lines cleared)
        #   - hard drop: thud / impact tone
        #   - game over: descending tone

        if self.game_over:
            if key == "r":
                self._new_game()
            return

        if key == "p":
            self.paused = not self.paused
            return

        if self.paused or self.clear_anim is not None:
            return

        if key in ("left", "h"):
            moved = self.current.copy()
            moved.col -= 1
            if self._valid(moved):
                self.current = moved

        elif key in ("right", "l"):
            moved = self.current.copy()
            moved.col += 1
            if self._valid(moved):
                self.current = moved

        elif key in ("down", "j"):
            self._soft_drop()

        elif key in ("up", "k", "x"):
            self._try_rotate()

        elif key in (" ", "Enter"):
            self._hard_drop()

        elif key == "r":
            self._new_game()


if __name__ == "__main__":
    TetrisApp().run()
