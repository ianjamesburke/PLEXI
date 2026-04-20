#!/usr/bin/env python3
"""Snake — input + draw primitives proof for PGAP v3.

Written in Python (not Rust) to keep the example self-contained and avoid a
separate Rust sub-project. The spec allows either; Python is 80 lines vs a
new cargo crate + cross-compile setup. This is the intentional choice.
"""
from __future__ import annotations

import sys
import os

import threading
import time
from plexi_sdk import App, RenderContext, BG, ACCENT, FG, RED, GREEN, MUTED

CELL = 20.0          # cell size in logical px
TICK = 0.15          # seconds per game tick
COLS = 20
ROWS = 16

DIR_MAP = {
    "ArrowUp":    (0, -1),
    "ArrowDown":  (0, 1),
    "ArrowLeft":  (-1, 0),
    "ArrowRight": (1, 0),
    "k": (0, -1),
    "j": (0, 1),
    "h": (-1, 0),
    "l": (1, 0),
}


class SnakeApp(App):
    def on_init(self, ctx: RenderContext) -> None:
        self._reset()
        self._timer_thread = threading.Thread(
            target=self._tick_loop, daemon=True
        )
        self._timer_thread.start()

    def _reset(self) -> None:
        mid_r, mid_c = ROWS // 2, COLS // 2
        self._snake: list[tuple[int, int]] = [
            (mid_c, mid_r), (mid_c - 1, mid_r), (mid_c - 2, mid_r)
        ]
        self._dir = (1, 0)
        self._next_dir = (1, 0)
        self._food = (COLS - 3, ROWS // 2)
        self._score = 0
        self._dead = False
        self._running = True

    def _tick_loop(self) -> None:
        while self._running:
            time.sleep(TICK)
            if not self._dead:
                self._step()

    def _step(self) -> None:
        dx, dy = self._next_dir
        # Prevent 180 reversal
        if (dx, dy) != (-self._dir[0], -self._dir[1]):
            self._dir = self._next_dir
        hx, hy = self._snake[0]
        nx, ny = (hx + self._dir[0]) % COLS, (hy + self._dir[1]) % ROWS
        if (nx, ny) in self._snake:
            self._dead = True
            return
        self._snake.insert(0, (nx, ny))
        if (nx, ny) == self._food:
            self._score += 1
            import random
            empty = [(c, r) for c in range(COLS) for r in range(ROWS)
                     if (c, r) not in self._snake]
            self._food = random.choice(empty) if empty else (-1, -1)
        else:
            self._snake.pop()

    def on_key(self, ctx: RenderContext, key: str, mods: dict) -> None:
        if self._dead and key in ("r", "Enter", " "):
            self._reset()
            return
        if key in DIR_MAP:
            self._next_dir = DIR_MAP[key]

    def on_render(self, ctx: RenderContext) -> None:
        ox = (ctx.w - COLS * CELL) / 2
        oy = (ctx.h - ROWS * CELL) / 2
        # Background
        ctx.rect(0, 0, ctx.w, ctx.h, fill=BG)
        # Grid border
        ctx.rect(ox - 2, oy - 2, COLS * CELL + 4, ROWS * CELL + 4,
                 fill=MUTED, radius=2.0)
        ctx.rect(ox, oy, COLS * CELL, ROWS * CELL, fill="#11111b")
        # Food
        fx, fy = self._food
        ctx.rect(ox + fx * CELL + 2, oy + fy * CELL + 2,
                 CELL - 4, CELL - 4, fill=RED, radius=4.0)
        # Snake
        for i, (sx, sy) in enumerate(self._snake):
            color = ACCENT if i == 0 else GREEN
            ctx.rect(ox + sx * CELL + 1, oy + sy * CELL + 1,
                     CELL - 2, CELL - 2, fill=color, radius=2.0)
        # Score
        ctx.text(ox, oy - 28, f"Score: {self._score}", size=14.0, color=FG)
        # Game over
        if self._dead:
            msg = "GAME OVER — press R to restart"
            ctx.rect(ox + COLS * CELL / 2 - 150, oy + ROWS * CELL / 2 - 18,
                     300, 36, fill="#1e1e2e", radius=6.0)
            ctx.text(ox + COLS * CELL / 2 - 140,
                     oy + ROWS * CELL / 2 - 8,
                     msg, size=13.0, color=RED)

    def on_shutdown(self) -> None:
        self._running = False


if __name__ == "__main__":
    SnakeApp().run()
