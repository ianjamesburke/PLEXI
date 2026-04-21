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

from plexi_sdk import App, RenderContext, dim

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

if __name__ == "__main__":
    pass  # entry point wired in Task 4
