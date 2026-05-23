#!/usr/bin/env python3
"""Balls — elastic circle collision physics demo.

Demonstrates DrawCommand::Circle, ctx.elapsed for frame-rate-independent
physics, ctx.clear(), and on_click to spawn new balls.

Physics model: constant gravity, elastic circle-circle collisions with
impulse resolution, wall bounce with damping.
"""
from __future__ import annotations

import math
import random
import sys
import os
sys.path.insert(0, os.path.join(os.path.dirname(__file__), '../../sdk/python'))

from plexi_sdk import App, RenderContext, dim

GRAVITY = 300.0      # px/s²
DAMPING = 0.78       # energy retained on wall bounce
FRICTION = 0.995     # per-frame horizontal friction on floor contact
MAX_BALLS = 50
MAX_DT = 0.05        # cap to avoid tunnelling on first frame / tab-back

PALETTE = [
    "#f38ba8", "#a6e3a1", "#89b4fa", "#f9e2af", "#cba6f7",
    "#94e2d5", "#fab387", "#74c7ec", "#b4befe", "#eba0ac",
    "#a6adc8", "#cdd6f4",
]


class Ball:
    __slots__ = ("x", "y", "vx", "vy", "r", "mass", "color")

    def __init__(self, x: float, y: float, r: float, color: str,
                 vx: float = 0.0, vy: float = 0.0):
        self.x = x
        self.y = y
        self.vx = vx
        self.vy = vy
        self.r = r
        self.mass = r * r   # mass ∝ area (uniform density)
        self.color = color


def _new_ball(x: float, y: float, idx: int,
              vx: float | None = None, vy: float | None = None) -> Ball:
    r = random.uniform(14.0, 44.0)
    color = PALETTE[idx % len(PALETTE)]
    return Ball(
        x, y, r, color,
        vx=random.uniform(-180.0, 180.0) if vx is None else vx,
        vy=random.uniform(-250.0, -60.0) if vy is None else vy,
    )


class BallsApp(App):
    # Override the default dark background with the physics demo's deep-space color.
    default_background = "#0d0d1a"

    def on_init(self, ctx: RenderContext) -> None:
        count = int(self.args[0]) if self.args else 10
        count = max(1, min(count, MAX_BALLS))
        self.balls: list[Ball] = []
        w, h = ctx.w, ctx.h
        for i in range(count):
            ball = _new_ball(0, 0, i)
            ball.x = random.uniform(ball.r, w - ball.r)
            ball.y = random.uniform(ball.r, h * 0.6)
            self.balls.append(ball)
        ctx.emit.info(f"balls: init complete, spawned {count} balls")

    def on_render(self, ctx: RenderContext) -> None:
        dt = min(ctx.elapsed, MAX_DT)
        w, h = ctx.w, ctx.h

        # ── Physics ───────────────────────────────────────────────────────────
        for b in self.balls:
            b.vy += GRAVITY * dt
            b.x += b.vx * dt
            b.y += b.vy * dt

        # Wall collisions
        for b in self.balls:
            if b.x - b.r < 0:
                b.x = b.r
                b.vx = abs(b.vx) * DAMPING
            elif b.x + b.r > w:
                b.x = w - b.r
                b.vx = -abs(b.vx) * DAMPING
            if b.y - b.r < 0:
                b.y = b.r
                b.vy = abs(b.vy) * DAMPING
            elif b.y + b.r > h:
                b.y = h - b.r
                b.vy = -abs(b.vy) * DAMPING
                b.vx *= FRICTION   # floor friction

        # Circle-circle elastic collisions (broad + narrow phase)
        balls = self.balls
        n = len(balls)
        for i in range(n):
            for j in range(i + 1, n):
                a, b = balls[i], balls[j]
                dx = b.x - a.x
                dy = b.y - a.y
                dist_sq = dx * dx + dy * dy
                min_dist = a.r + b.r
                if dist_sq >= min_dist * min_dist:
                    continue

                if dist_sq > 0:
                    dist = math.sqrt(dist_sq)
                    nx, ny = dx / dist, dy / dist
                else:
                    dist = 0.0
                    nx, ny = 1.0, 0.0  # default normal for exact overlap

                # Separate overlapping circles (mass-weighted push)
                overlap = min_dist - dist
                total = a.mass + b.mass
                push_a = overlap * b.mass / total
                push_b = overlap * a.mass / total
                a.x -= nx * push_a
                a.y -= ny * push_a
                b.x += nx * push_b
                b.y += ny * push_b

                # Elastic impulse — only if approaching
                rel_vx = a.vx - b.vx
                rel_vy = a.vy - b.vy
                approach = rel_vx * nx + rel_vy * ny
                if approach <= 0:
                    continue
                impulse = 2.0 * approach / total
                a.vx -= impulse * b.mass * nx
                a.vy -= impulse * b.mass * ny
                b.vx += impulse * a.mass * nx
                b.vy += impulse * a.mass * ny

        # ── Render ────────────────────────────────────────────────────────────
        # Draw shadows first so they don't overlap other balls' bodies
        for ball in self.balls:
            ctx.circle(ball.x + 3, ball.y + 4, ball.r, dim("#000000", 60))

        for ball in self.balls:
            ctx.circle(ball.x, ball.y, ball.r, ball.color)
            hi_r = max(3.0, ball.r * 0.28)
            hi_x = ball.x - ball.r * 0.28
            hi_y = ball.y - ball.r * 0.28
            ctx.circle(hi_x, hi_y, hi_r, dim("#ffffff", 90))

        # HUD
        count = len(self.balls)
        ctx.text(10, 10, f"{count} balls — click to add · click ball to remove",
                 size=12, color="#45475a")

        ctx.emit.schedule_render(16)   # 60 fps

    def on_click(self, ctx: RenderContext, x: float, y: float, _button: str) -> None:
        # Click on an existing ball → remove it (check largest/topmost first)
        for i in range(len(self.balls) - 1, -1, -1):
            b = self.balls[i]
            if (x - b.x) ** 2 + (y - b.y) ** 2 <= b.r ** 2:
                self.balls.pop(i)
                ctx.emit.info(f"balls: removed ball at ({x:.0f}, {y:.0f}), count={len(self.balls)}")
                return
        # Click on empty space → spawn at cursor
        if len(self.balls) < MAX_BALLS:
            self.balls.append(_new_ball(x, y, len(self.balls), vy=random.uniform(-300.0, -120.0)))
            ctx.emit.info(f"balls: spawned ball at ({x:.0f}, {y:.0f}), count={len(self.balls)}")


if __name__ == "__main__":
    BallsApp().run()
