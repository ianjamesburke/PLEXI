#!/usr/bin/env python3
"""Snake — input + draw primitives proof for PGAP v3.

Written in Python (not Rust) to keep the example self-contained and avoid a
separate Rust sub-project. The spec allows either; Python is 80 lines vs a
new cargo crate + cross-compile setup. This is the intentional choice.
"""

from plexi_sdk import App, RenderContext, Arg, Pipe
from plexi_sdk.ui import Column, AppBar, FooterKeys, Component

CELL = 18.0          # cell size in logical px
TICK = 0.15          # seconds per game tick
COLS = 26
ROWS = 20

DIR_MAP = {
    "up":    (0, -1),
    "down":  (0, 1),
    "left":  (-1, 0),
    "right": (1, 0),
    "k": (0, -1),
    "j": (0, 1),
    "h": (-1, 0),
    "l": (1, 0),
}


class _SnakeCanvas(Component):
    """Grow-to-fill canvas that renders the grid, snake, food, and game-over overlay."""

    def __init__(self, app: "SnakeApp") -> None:
        self._app = app

    def is_grow(self) -> bool:
        return True

    def measure(self, _avail_w: float) -> float:
        return 0.0

    def render(self, ctx, x: float, y: float, w: float, h: float) -> None:
        app = self._app
        ox = x + (w - COLS * CELL) / 2
        oy = y + (h - ROWS * CELL) / 2
        # Grid border
        ctx.rect(ox - 2, oy - 2, COLS * CELL + 4, ROWS * CELL + 4,
                 fill=ctx.theme.muted, radius=2.0)
        ctx.rect(ox, oy, COLS * CELL, ROWS * CELL, fill=ctx.theme.bg_darkest)
        # Food
        fx, fy = app._food
        ctx.rect(ox + fx * CELL + 2, oy + fy * CELL + 2,
                 CELL - 4, CELL - 4, fill=ctx.theme.danger, radius=4.0)
        # Snake
        for i, (sx, sy) in enumerate(app._snake):
            color = ctx.theme.accent if i == 0 else ctx.theme.success
            ctx.rect(ox + sx * CELL + 1, oy + sy * CELL + 1,
                     CELL - 2, CELL - 2, fill=color, radius=2.0)
        # Game over overlay
        if app._dead:
            msg = "GAME OVER — press R to restart"
            ctx.rect(ox + COLS * CELL / 2 - 150, oy + ROWS * CELL / 2 - 18,
                     300, 36, fill=ctx.theme.bg, radius=6.0)
            ctx.text(ox + COLS * CELL / 2 - 140,
                     oy + ROWS * CELL / 2 - 8,
                     msg, size=13.0, color=ctx.theme.danger)


class SnakeApp(App):
    pipe_id: Arg[str | None] = Arg("--pipe", default=None)

    def on_init(self) -> None:
        self._pipe: "Pipe | None" = None
        if self.pipe_id:
            self._pipe = self.emit.pipe_open(self.pipe_id, mode="json", direction="out")
            self.emit.info(f"snake: pipe open pipe_id={self.pipe_id}")
        self._reset()
        self._canvas = _SnakeCanvas(self)

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
        self._tick_accum = 0.0

    def _advance(self, elapsed: float) -> None:
        if self._dead:
            return
        self._tick_accum += min(elapsed, TICK * 2.0)
        steps = 0
        while self._tick_accum >= TICK and steps < 2:
            self._step()
            self._tick_accum -= TICK
            steps += 1

    def _step(self) -> None:
        dx, dy = self._next_dir
        # Prevent 180 reversal
        if (dx, dy) != (-self._dir[0], -self._dir[1]):
            self._dir = self._next_dir
        hx, hy = self._snake[0]
        nx, ny = (hx + self._dir[0]) % COLS, (hy + self._dir[1]) % ROWS
        if (nx, ny) in self._snake:
            self._dead = True
            if self._pipe:
                self._pipe.send({"event": "game_over", "score": self._score})
                self.emit.info(f"snake: game_over score={self._score}")
            return
        self._snake.insert(0, (nx, ny))
        if (nx, ny) == self._food:
            self._score += 1
            if self._pipe:
                self._pipe.send({"event": "score", "score": self._score})
                self.emit.info(f"snake: score={self._score}")
            import random
            empty = [(c, r) for c in range(COLS) for r in range(ROWS)
                     if (c, r) not in self._snake]
            self._food = random.choice(empty) if empty else (-1, -1)
        else:
            self._snake.pop()

    def on_key(self, key: str, _mods: dict) -> None:
        if self._dead and key in ("r", "Enter", " "):
            self._reset()
            return
        if key in DIR_MAP:
            self._next_dir = DIR_MAP[key]

    def on_render(self, ctx: RenderContext) -> None:
        self._advance(ctx.elapsed)
        subtitle = "GAME OVER" if self._dead else f"Score: {self._score}"
        shortcuts = (
            [("h/j/k/l", "move"), ("r", "restart")]
            if self._dead
            else [("h/j/k/l", "move")]
        )
        ctx.render(Column([
            AppBar(title="Snake", subtitle=subtitle),
            self._canvas,
            FooterKeys(shortcuts),
        ]))
        if self._dead:
            ctx.emit.schedule_render(250)
        else:
            next_tick_ms = max(1, int((TICK - self._tick_accum) * 1000))
            ctx.emit.schedule_render(next_tick_ms)


if __name__ == "__main__":
    SnakeApp().run()
