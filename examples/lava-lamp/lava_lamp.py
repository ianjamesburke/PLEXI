#!/usr/bin/env python3
"""Lava Lamp — buoyancy-driven blobs with fake-metaball blending.

Blobs rise when hot, sink when cool. Nearby blobs render translucent bridge
circles between them to simulate the characteristic merging-and-splitting look.
"""
from __future__ import annotations

import math
import random
import sys
import os
sys.path.insert(0, os.path.join(os.path.dirname(__file__), '../../sdk/python'))

from plexi_sdk import App, RenderContext

# ── Constants ────────────────────────────────────────────────────────────────
NUM_BLOBS      = 7
MAX_DT         = 0.05       # cap dt to avoid tunnelling on first frame
BUOYANCY       = 220.0      # upward acceleration when hot (px/s²)
GRAVITY        = 160.0      # downward acceleration when cool (px/s²)
DRAG           = 0.96       # velocity damping per frame
WALL_REPEL     = 400.0      # lateral repulsion from walls (px/s²)
WALL_MARGIN    = 0.12       # fraction of width to keep blobs away from walls
TEMP_RATE      = 0.6        # how fast temperature tracks target (0–1 blend/s)
TEMP_NOISE     = 0.04       # random walk amplitude on temperature per frame
MERGE_FACTOR   = 1.5        # blob center distance / (r_a + r_b) to start merging
BRIDGE_STEPS   = 6          # translucent circles per bridge

# Warm lava palette (fill colors)
PALETTE = [
    "#ff4d00", "#ff8c00", "#ff2e7a", "#ff6347",
    "#e8003d", "#ff5500", "#cc2200",
]


class Blob:
    __slots__ = ("x", "y", "vx", "vy", "r", "temp", "color")

    def __init__(self, x: float, y: float, r: float, color: str) -> None:
        self.x     = x
        self.y     = y
        self.vx    = 0.0
        self.vy    = 0.0
        self.r     = r
        self.temp  = random.random()   # 0.0 = cold, 1.0 = hot
        self.color = color

    def target_temp(self, h: float) -> float:
        """Blobs near the bottom should be hot; near the top, cold."""
        # y=h → bottom → hot (1.0); y=0 → top → cold (0.0)
        return self.y / h

    def update(self, dt: float, w: float, h: float) -> None:
        """Advance physics by dt seconds."""
        dt = min(dt, MAX_DT)

        # Temperature: blend toward target + small random walk
        target = self.target_temp(h)
        self.temp += (target - self.temp) * TEMP_RATE * dt
        self.temp += random.uniform(-TEMP_NOISE, TEMP_NOISE)
        self.temp  = max(0.0, min(1.0, self.temp))

        # Net vertical acceleration: hot = rise, cold = fall
        # temp=1 → fully hot → buoyancy wins (upward, negative y)
        # temp=0 → fully cold → gravity wins (downward, positive y)
        net_ay = GRAVITY * (1.0 - self.temp) - BUOYANCY * self.temp

        # Lateral wall repulsion keeps blobs off the edges
        margin = w * WALL_MARGIN
        ax = 0.0
        if self.x < margin:
            ax += WALL_REPEL * (1.0 - self.x / margin)
        elif self.x > w - margin:
            ax -= WALL_REPEL * (1.0 - (w - self.x) / margin)

        self.vx += ax * dt
        self.vy += net_ay * dt

        # Viscous drag (lava is thick)
        self.vx *= DRAG
        self.vy *= DRAG

        self.x += self.vx * dt
        self.y += self.vy * dt

        # Hard clamp to bounds (soft bounce off top/bottom)
        if self.y - self.r < 0:
            self.y  = self.r
            self.vy = abs(self.vy) * 0.3
        if self.y + self.r > h:
            self.y  = h - self.r
            self.vy = -abs(self.vy) * 0.3
        self.x = max(self.r, min(w - self.r, self.x))


def _hex_to_rgb(h: str) -> tuple[int, int, int]:
    """Parse #rrggbb → (r, g, b)."""
    h = h.lstrip("#")
    return int(h[0:2], 16), int(h[2:4], 16), int(h[4:6], 16)


def _dim_alpha(hex_color: str, alpha: int) -> str:
    """Return hex_color with a given alpha (0–255) as #rrggbbaa."""
    r, g, b = _hex_to_rgb(hex_color)
    return f"#{r:02x}{g:02x}{b:02x}{alpha:02x}"


def _render_blob(ctx: RenderContext, b: Blob) -> None:
    """Draw a blob as layered circles: glow → body → specular highlight."""
    # Outer glow (large, very translucent)
    ctx.circle(b.x, b.y, b.r * 1.55, _dim_alpha(b.color, 22))
    # Mid halo
    ctx.circle(b.x, b.y, b.r * 1.25, _dim_alpha(b.color, 45))
    # Solid core
    ctx.circle(b.x, b.y, b.r, b.color)
    # Specular highlight
    hi_r = max(3.0, b.r * 0.25)
    ctx.circle(b.x - b.r * 0.30, b.y - b.r * 0.30, hi_r, _dim_alpha("#ffffff", 80))


def _render_bridge(ctx: RenderContext, a: Blob, b: Blob, proximity: float) -> None:
    """Draw translucent circles along the line between two nearby blobs.

    proximity: 0.0 = just touching threshold, 1.0 = centers overlapping.
    """
    base_alpha = int(proximity * 70)
    if base_alpha < 5:
        return

    for i in range(1, BRIDGE_STEPS):
        t   = i / BRIDGE_STEPS
        cx  = a.x + (b.x - a.x) * t
        cy  = a.y + (b.y - a.y) * t
        mid_boost = 1.0 - abs(t - 0.5) * 1.2
        r   = (a.r * (1 - t) + b.r * t) * (0.55 + mid_boost * 0.35)
        ra, ga, ba_ = _hex_to_rgb(a.color)
        rb, gb, bb_ = _hex_to_rgb(b.color)
        rc  = int(ra + (rb - ra) * t)
        gc  = int(ga + (gb - ga) * t)
        bc  = int(ba_ + (bb_ - ba_) * t)
        col = f"#{rc:02x}{gc:02x}{bc:02x}"
        ctx.circle(cx, cy, r, _dim_alpha(col, base_alpha))

class LavaLampApp(App):

    def on_init(self, ctx: RenderContext) -> None:
        w, h = ctx.w, ctx.h
        self.blobs: list[Blob] = []
        for i in range(NUM_BLOBS):
            r     = random.uniform(22.0, 48.0)
            x     = random.uniform(w * 0.2, w * 0.8)
            y     = random.uniform(h * 0.2, h * 0.9)
            color = PALETTE[i % len(PALETTE)]
            self.blobs.append(Blob(x, y, r, color))

    def on_render(self, ctx: RenderContext) -> None:
        dt = ctx.elapsed
        w, h = ctx.w, ctx.h

        for blob in self.blobs:
            blob.update(dt, w, h)

        ctx.clear("#0a0a1a")

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

    def on_click(self, ctx: RenderContext, x: float, y: float, button: str) -> None:
        if not self.blobs:
            return
        nearest = min(
            self.blobs,
            key=lambda b: (b.x - x) ** 2 + (b.y - y) ** 2,
        )
        nearest.temp = min(1.0, nearest.temp + 0.5)
        nearest.vy  -= 80.0


if __name__ == "__main__":
    LavaLampApp().run()
