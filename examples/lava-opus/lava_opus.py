#!/usr/bin/env python3
"""Lava Opus — richer lava lamp with blob-blob soft repulsion and a broader palette.

Same buoyancy + temperature model as lava-lamp, but with:
- 10 blobs (vs 7), wider size range
- Soft blob-blob repulsion so they don't pile up
- Expanded warm palette with more color variety
- Slightly slower drag for a more viscous feel
"""
from __future__ import annotations

import math
import random
import sys
import os
sys.path.insert(0, os.path.join(os.path.dirname(__file__), '../../sdk/python'))

from plexi_sdk import App, RenderContext

# ── Constants ────────────────────────────────────────────────────────────────
NUM_BLOBS      = 10
MAX_DT         = 0.05
BUOYANCY       = 240.0
GRAVITY        = 170.0
DRAG           = 0.94       # thicker lava — more viscous than lava-lamp
WALL_REPEL     = 500.0
WALL_MARGIN    = 0.10
TEMP_RATE      = 0.5
TEMP_NOISE     = 0.035
MERGE_FACTOR   = 1.5
BRIDGE_STEPS   = 6
REPEL_STRENGTH = 300.0      # blob-blob soft repulsion (px/s²·px)

PALETTE = [
    "#ff4d00", "#ff8c00", "#ff2e7a", "#ff6347",
    "#e8003d", "#ff5500", "#cc2200", "#ff9933",
    "#ff1493", "#ff6600", "#dc143c", "#ff4500",
]


class Blob:
    __slots__ = ("x", "y", "vx", "vy", "r", "temp", "color")

    def __init__(self, x: float, y: float, r: float, color: str) -> None:
        self.x     = x
        self.y     = y
        self.vx    = 0.0
        self.vy    = 0.0
        self.r     = r
        self.temp  = random.random()
        self.color = color

    def target_temp(self, h: float) -> float:
        return self.y / h

    def update(self, dt: float, w: float, h: float, blobs: list["Blob"]) -> None:
        dt = min(dt, MAX_DT)

        target = self.target_temp(h)
        self.temp += (target - self.temp) * TEMP_RATE * dt
        self.temp += random.uniform(-TEMP_NOISE, TEMP_NOISE)
        self.temp  = max(0.0, min(1.0, self.temp))

        net_ay = GRAVITY * (1.0 - self.temp) - BUOYANCY * self.temp

        margin = w * WALL_MARGIN
        ax = 0.0
        if self.x < margin:
            ax += WALL_REPEL * (1.0 - self.x / margin)
        elif self.x > w - margin:
            ax -= WALL_REPEL * (1.0 - (w - self.x) / margin)

        # Soft blob-blob repulsion
        for other in blobs:
            if other is self:
                continue
            dx = self.x - other.x
            dy = self.y - other.y
            dist = math.sqrt(dx * dx + dy * dy) or 0.001
            touch = self.r + other.r
            if dist < touch * 1.8:
                strength = REPEL_STRENGTH * (1.0 - dist / (touch * 1.8))
                ax += (dx / dist) * strength
                net_ay += (dy / dist) * strength

        self.vx += ax * dt
        self.vy += net_ay * dt

        self.vx *= DRAG
        self.vy *= DRAG

        self.x += self.vx * dt
        self.y += self.vy * dt

        if self.y - self.r < 0:
            self.y  = self.r
            self.vy = abs(self.vy) * 0.3
        if self.y + self.r > h:
            self.y  = h - self.r
            self.vy = -abs(self.vy) * 0.3
        self.x = max(self.r, min(w - self.r, self.x))


def _hex_to_rgb(h: str) -> tuple[int, int, int]:
    h = h.lstrip("#")
    return int(h[0:2], 16), int(h[2:4], 16), int(h[4:6], 16)


def _dim_alpha(hex_color: str, alpha: int) -> str:
    r, g, b = _hex_to_rgb(hex_color)
    return f"#{r:02x}{g:02x}{b:02x}{alpha:02x}"


def _render_blob(ctx: RenderContext, b: Blob) -> None:
    ctx.circle(b.x, b.y, b.r * 1.6,  _dim_alpha(b.color, 18))
    ctx.circle(b.x, b.y, b.r * 1.3,  _dim_alpha(b.color, 38))
    ctx.circle(b.x, b.y, b.r * 1.05, _dim_alpha(b.color, 70))
    ctx.circle(b.x, b.y, b.r, b.color)
    hi_r = max(3.0, b.r * 0.22)
    ctx.circle(b.x - b.r * 0.28, b.y - b.r * 0.28, hi_r, _dim_alpha("#ffffff", 90))


def _render_bridge(ctx: RenderContext, a: Blob, b: Blob, proximity: float) -> None:
    base_alpha = int(proximity * 80)
    if base_alpha < 5:
        return

    for i in range(1, BRIDGE_STEPS):
        t   = i / BRIDGE_STEPS
        cx  = a.x + (b.x - a.x) * t
        cy  = a.y + (b.y - a.y) * t
        mid_boost = 1.0 - abs(t - 0.5) * 1.1
        r   = (a.r * (1 - t) + b.r * t) * (0.6 + mid_boost * 0.35)
        ra, ga, ba_ = _hex_to_rgb(a.color)
        rb, gb, bb_ = _hex_to_rgb(b.color)
        rc  = int(ra + (rb - ra) * t)
        gc  = int(ga + (gb - ga) * t)
        bc  = int(ba_ + (bb_ - ba_) * t)
        col = f"#{rc:02x}{gc:02x}{bc:02x}"
        ctx.circle(cx, cy, r, _dim_alpha(col, base_alpha))


class LavaOpusApp(App):

    def on_init(self, ctx: RenderContext) -> None:
        w, h = ctx.w, ctx.h
        self.blobs: list[Blob] = []
        for i in range(NUM_BLOBS):
            r     = random.uniform(18.0, 52.0)
            x     = random.uniform(w * 0.15, w * 0.85)
            y     = random.uniform(h * 0.15, h * 0.95)
            color = PALETTE[i % len(PALETTE)]
            self.blobs.append(Blob(x, y, r, color))

    def on_render(self, ctx: RenderContext) -> None:
        dt = ctx.elapsed
        w, h = ctx.w, ctx.h

        for blob in self.blobs:
            blob.update(dt, w, h, self.blobs)

        ctx.clear("#080814")

        blobs = self.blobs
        n = len(blobs)
        for i in range(n):
            for j in range(i + 1, n):
                a, b = blobs[i], blobs[j]
                dx   = b.x - a.x
                dy   = b.y - a.y
                dist = math.sqrt(dx * dx + dy * dy) or 0.001
                threshold = (a.r + b.r) * MERGE_FACTOR
                if dist < threshold:
                    proximity = 1.0 - (dist / threshold)
                    _render_bridge(ctx, a, b, proximity)

        for blob in blobs:
            _render_blob(ctx, blob)

        ctx.emit.schedule_render(16)

    def on_click(self, ctx: RenderContext, x: float, y: float, button: str) -> None:  # noqa: ARG002
        if not self.blobs:
            return
        nearest = min(
            self.blobs,
            key=lambda b: (b.x - x) ** 2 + (b.y - y) ** 2,
        )
        nearest.temp = min(1.0, nearest.temp + 0.6)
        nearest.vy  -= 100.0


if __name__ == "__main__":
    LavaOpusApp().run()
