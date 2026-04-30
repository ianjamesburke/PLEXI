#!/usr/bin/env python3
"""Parallax DAW — visual proof-of-concept Techno loop DAW.

Loop-based 8-channel step sequencer with physics-driven modulation balls,
a 32-bar arrangement strip, and a transport bar. Purely visual — no audio.

Channels: Kick, Snare, Hi-Hat, Clap, Bass, Lead, Pad, FX.
Grid: 8 rows × 64 steps (4 bars × 16 steps each).
Physics balls float over the grid, bouncing off its walls, providing
visual modulation cues (which column region is "hot").
"""

import time
import threading

from plexi_sdk import App, RenderContext  # type: ignore[import-untyped]

# ── Palette ───────────────────────────────────────────────────────────────────
BG          = "#0a0a0f"
PANEL_BG    = "#12121a"
BORDER      = "#2a2a3a"
TRANSPORT   = "#1a1a2e"
TEXT        = "#e0e0ff"
TEXT_DIM    = "#6060a0"
CELL_EMPTY  = "#1a1a1a"
CELL_BORDER = "#2a2a3a"
ACCENT_BEAT = "#00ffff"

# ── Layout ────────────────────────────────────────────────────────────────────
TRANSPORT_H  = 50.0
CHANNEL_W    = 140.0
ARRANGE_H    = 80.0
CELL_W       = 14.0
CELL_H       = 20.0
CELL_GAP     = 1.0
NUM_CHANNELS = 8
NUM_STEPS    = 64
NUM_BARS     = 32

# Derived step cell pitch (width + gap)
STEP_PITCH = CELL_W + CELL_GAP


# ── Channel and ball config ───────────────────────────────────────────────────

CHANNELS: list[dict] = [
    {"name": "KICK",   "color": "#ff4444"},
    {"name": "SNARE",  "color": "#ff8844"},
    {"name": "HI-HAT", "color": "#ffcc44"},
    {"name": "CLAP",   "color": "#44ff88"},
    {"name": "BASS",   "color": "#44aaff"},
    {"name": "LEAD",   "color": "#aa44ff"},
    {"name": "PAD",    "color": "#ff44aa"},
    {"name": "FX",     "color": "#44ffff"},
]

# Channel mock volume levels (0.0–1.0, purely visual)
CHANNEL_VOL = [0.85, 0.70, 0.60, 0.55, 0.90, 0.75, 0.50, 0.65]

BALL_DEFS: list[dict] = [
    {"x": 200.0, "y": 200.0, "vx": 1.8,  "vy": 1.3,  "color": "#00ffff", "r": 18.0},
    {"x": 400.0, "y": 150.0, "vx": -2.1, "vy": 0.9,  "color": "#ff00ff", "r": 14.0},
    {"x": 600.0, "y": 300.0, "vx": 1.4,  "vy": -1.7, "color": "#ffff00", "r": 16.0},
    {"x": 300.0, "y": 400.0, "vx": -1.2, "vy": 2.0,  "color": "#00ff88", "r": 12.0},
    {"x": 500.0, "y": 250.0, "vx": 2.3,  "vy": -1.1, "color": "#ff8800", "r": 20.0},
]


def _hex_alpha(color: str, alpha_pct: int) -> str:
    """Return color with alpha as an 8-char hex string. alpha_pct 0–100."""
    h = color.lstrip("#")[:6]
    a = int(alpha_pct * 255 / 100)
    return f"#{h}{a:02x}"


class ParallaxDAW(App):

    def on_init(self, ctx: RenderContext) -> None:
        self.steps: list[list[bool]] = [[False] * NUM_STEPS for _ in range(NUM_CHANNELS)]
        self.playing: bool = False
        self.bpm: int = 138
        self.current_beat: float = 0.0
        self.current_bar: int = 0
        self.arrangement: list[bool] = [False] * NUM_BARS
        self.arrangement[0] = True

        # Deep-copy ball defs so we mutate local state
        self.balls: list[dict] = [dict(b) for b in BALL_DEFS]

        # Seed a few steps for visual interest
        kick_pattern = [0, 4, 8, 12, 16, 20, 24, 28, 32, 36, 40, 44, 48, 52, 56, 60]
        for s in kick_pattern:
            self.steps[0][s] = True
        snare_pattern = [4, 12, 20, 28, 36, 44, 52, 60]
        for s in snare_pattern:
            self.steps[1][s] = True
        hihat_pattern = [i for i in range(NUM_STEPS) if i % 2 == 0]
        for s in hihat_pattern:
            self.steps[2][s] = True

        # Playback clock
        self._t0: float = 0.0
        self._beat_thread_running: bool = False

        ctx.status_summary("Parallax DAW · 138 BPM · stopped")

    # ── Play control ──────────────────────────────────────────────────────────

    def _start_playback(self) -> None:
        self._t0 = time.monotonic()
        self.playing = True
        self._beat_thread_running = True

        def _tick() -> None:
            beats_per_sec = self.bpm / 60.0
            while self._beat_thread_running:
                time.sleep(0.033)
                elapsed = time.monotonic() - self._t0
                self.current_beat = (elapsed * beats_per_sec * 4) % NUM_STEPS
                self.emit.schedule_render(after_ms=0)

        threading.Thread(target=_tick, daemon=True).start()

    def _stop_playback(self) -> None:
        self._beat_thread_running = False
        self.playing = False

    def _toggle_play(self) -> None:
        if self.playing:
            self._stop_playback()
        else:
            self._start_playback()

    # ── Input ─────────────────────────────────────────────────────────────────

    def on_key(self, ctx: RenderContext, key: str, _mods: dict) -> None:
        if key == " ":
            self._toggle_play()
        elif key == "+" or key == "=":
            self.bpm = min(200, self.bpm + 1)
        elif key == "-":
            self.bpm = max(60, self.bpm - 1)
        ctx.status_summary(
            f"Parallax DAW · {self.bpm} BPM · {'playing' if self.playing else 'stopped'}"
        )
        self.emit.schedule_render(after_ms=16)

    def on_click(self, ctx: RenderContext, x: float, y: float, _button: str) -> None:
        W, H = ctx.w, ctx.h
        grid_x, grid_y, grid_w, grid_h = self._grid_rect(W, H)
        arrange_y = H - ARRANGE_H

        # Transport bar
        if y < TRANSPORT_H:
            self._handle_transport_click(x, y, W)
            return

        # Arrangement strip
        if y >= arrange_y:
            self._handle_arrange_click(x, y, W, H, arrange_y)
            return

        # Loop grid
        if (grid_x <= x < grid_x + grid_w) and (grid_y <= y < grid_y + grid_h):
            col = int((x - grid_x) / STEP_PITCH)
            row = int((y - grid_y) / (CELL_H + CELL_GAP))
            if 0 <= row < NUM_CHANNELS and 0 <= col < NUM_STEPS:
                self.steps[row][col] = not self.steps[row][col]

        self.emit.schedule_render(after_ms=16)

    def _handle_transport_click(self, x: float, y: float, W: float) -> None:
        mid = W / 2.0
        cy = TRANSPORT_H / 2.0
        # Play/Stop button at center
        if abs(x - mid) < 20 and abs(y - cy) < 16:
            self._toggle_play()
            return
        # BPM - button (left of center)
        if abs(x - (mid - 80)) < 12 and abs(y - cy) < 12:
            self.bpm = max(60, self.bpm - 1)
            return
        # BPM + button (right of center)
        if abs(x - (mid + 80)) < 12 and abs(y - cy) < 12:
            self.bpm = min(200, self.bpm + 1)
            return
        self.emit.schedule_render(after_ms=16)

    def _handle_arrange_click(
        self, x: float, _y: float, W: float, _H: float, _arrange_y: float
    ) -> None:
        bar_area_x = CHANNEL_W + 8.0
        bar_area_w = W - bar_area_x - 8.0
        bar_pitch = bar_area_w / NUM_BARS
        if x >= bar_area_x:
            idx = int((x - bar_area_x) / bar_pitch)
            if 0 <= idx < NUM_BARS:
                self.current_bar = idx
                self.arrangement[idx] = True
        self.emit.schedule_render(after_ms=16)

    # ── Geometry helpers ──────────────────────────────────────────────────────

    def _grid_rect(self, _W: float, H: float) -> tuple[float, float, float, float]:
        """Return (x, y, w, h) of the step-grid content area."""
        gx = CHANNEL_W
        gy = TRANSPORT_H
        gw = NUM_STEPS * STEP_PITCH
        gh = H - TRANSPORT_H - ARRANGE_H
        return gx, gy, gw, gh

    # ── Render ────────────────────────────────────────────────────────────────

    def on_render(self, ctx: RenderContext) -> None:
        W, H = ctx.w, ctx.h

        # Background
        ctx.clear(BG)

        # Draw all sections
        self._draw_transport(ctx, W)
        self._draw_channel_strips(ctx, W, H)
        self._draw_grid(ctx, W, H)
        self._advance_and_draw_balls(ctx, W, H)
        self._draw_arrangement(ctx, W, H)

        ctx.status_summary(
            f"Parallax DAW · {self.bpm} BPM · {'playing' if self.playing else 'stopped'}"
        )
        self.emit.schedule_render(after_ms=33)  # ~30 fps for physics

    # ── Transport bar ─────────────────────────────────────────────────────────

    def _draw_transport(self, ctx: RenderContext, W: float) -> None:
        ctx.rect(0, 0, W, TRANSPORT_H, fill=TRANSPORT)
        ctx.line(0, TRANSPORT_H, W, TRANSPORT_H, color=BORDER, width=1.0)

        cy = TRANSPORT_H / 2.0
        mid = W / 2.0

        # App title
        ctx.text(14, cy - 7, "PARALLAX", size=11.0, color=TEXT_DIM, bold=True)
        ctx.text(14, cy + 5, "DAW", size=9.0, color=ACCENT_BEAT)

        # BPM display and +/- buttons
        bpm_x = mid - 120.0
        ctx.rect(bpm_x - 6, cy - 13, 80, 26, fill=PANEL_BG, radius=4.0)
        ctx.text(bpm_x + 34, cy - 1, f"{self.bpm}", size=18.0, color=TEXT, bold=True)
        ctx.text(bpm_x + 6, cy - 1, "BPM", size=9.0, color=TEXT_DIM)

        # Minus button
        minus_x = bpm_x + 85.0
        ctx.rect(minus_x - 10, cy - 10, 20, 20, fill=PANEL_BG, radius=3.0)
        ctx.text(minus_x, cy - 1, "–", size=13.0, color=TEXT)

        # Plus button
        plus_x = minus_x + 26.0
        ctx.rect(plus_x - 10, cy - 10, 20, 20, fill=PANEL_BG, radius=3.0)
        ctx.text(plus_x, cy - 1, "+", size=13.0, color=TEXT)

        # Play/Stop button
        is_playing = self.playing
        play_fill = "#00cc88" if is_playing else "#2255cc"
        ctx.rect(mid - 22, cy - 16, 44, 32, fill=play_fill, radius=6.0)
        label = "■" if is_playing else "▶"
        ctx.text(mid, cy, label, size=14.0, color="#ffffff", bold=True)

        # Bar counter
        bar_x = mid + 42.0
        ctx.text(bar_x, cy - 6, f"BAR  {self.current_bar + 1} / {NUM_BARS}", size=11.0, color=TEXT)

        # Beat indicator: a small dot moving across a thin strip
        strip_x = bar_x
        strip_w = W - bar_x - 16.0
        strip_y = cy + 8.0
        ctx.rect(strip_x, strip_y, strip_w, 4.0, fill=PANEL_BG, radius=2.0)
        beat_frac = (self.current_beat % 16) / 16.0
        dot_x = strip_x + beat_frac * strip_w
        ctx.circle(dot_x, strip_y + 2.0, 4.0, fill=ACCENT_BEAT)

    # ── Channel strips ────────────────────────────────────────────────────────

    def _draw_channel_strips(self, ctx: RenderContext, _W: float, H: float) -> None:
        strip_h = H - TRANSPORT_H - ARRANGE_H
        row_h = strip_h / NUM_CHANNELS
        x0 = 0.0
        y0 = TRANSPORT_H

        ctx.rect(x0, y0, CHANNEL_W, strip_h, fill=PANEL_BG)
        ctx.line(CHANNEL_W, y0, CHANNEL_W, y0 + strip_h, color=BORDER, width=1.0)

        for i, ch in enumerate(CHANNELS):
            ry = y0 + i * row_h
            ctx.line(x0, ry + row_h, CHANNEL_W, ry + row_h, color=BORDER, width=1.0)

            # Color swatch
            ctx.rect(x0 + 8, ry + 8, 6, row_h - 16, fill=ch["color"], radius=2.0)

            # Channel name
            ctx.text(x0 + 22, ry + row_h / 2.0 - 5, ch["name"], size=10.0, color=TEXT, bold=True)

            # Volume bar (thin vertical strip on the right side of the strip)
            vol = CHANNEL_VOL[i]
            bar_x = CHANNEL_W - 18.0
            bar_total_h = row_h - 12.0
            bar_filled = bar_total_h * vol
            ctx.rect(bar_x, ry + 6, 6, bar_total_h, fill=CELL_EMPTY, radius=1.0)
            ctx.rect(
                bar_x,
                ry + 6 + bar_total_h - bar_filled,
                6,
                bar_filled,
                fill=_hex_alpha(ch["color"], 70),
                radius=1.0,
            )

    # ── Step grid ─────────────────────────────────────────────────────────────

    def _draw_grid(self, ctx: RenderContext, W: float, H: float) -> None:
        gx, gy, gw, gh = self._grid_rect(W, H)
        row_h = gh / NUM_CHANNELS

        ctx.rect(gx, gy, gw, gh, fill=PANEL_BG)

        # Collect ball x-positions to compute per-column hotness
        ball_xs = [b["x"] for b in self.balls]

        for row in range(NUM_CHANNELS):
            ry = gy + row * row_h
            ch = CHANNELS[row]

            for col in range(NUM_STEPS):
                cx = gx + col * STEP_PITCH
                cy_cell = ry + (row_h - CELL_H) / 2.0

                # Compute hotness: how close is any ball's x to this column's center
                cell_cx = cx + CELL_W / 2.0
                hotness = 0.0
                for bx in ball_xs:
                    dist = abs(bx - cell_cx)
                    if dist < 60.0:
                        hotness = max(hotness, 1.0 - dist / 60.0)

                active = self.steps[row][col]

                if active:
                    fill = _hex_alpha(ch["color"], 82)
                    # Boost brightness near current beat
                    beat_col = int(self.current_beat) % NUM_STEPS
                    if col == beat_col:
                        fill = ch["color"]
                else:
                    # Empty: dark, with slight glow if a ball is nearby
                    if hotness > 0.15:
                        glow_a = int(15 + hotness * 30)
                        fill = _hex_alpha(ch["color"], glow_a)
                    else:
                        fill = CELL_EMPTY

                ctx.rect(cx, cy_cell, CELL_W, CELL_H, fill=fill, radius=1.5)

            # Row separator
            ctx.line(gx, ry + row_h, gx + gw, ry + row_h, color=BORDER, width=1.0)

        # Bar division lines (every 16 steps)
        for bar in range(1, NUM_STEPS // 16):
            lx = gx + bar * 16 * STEP_PITCH
            ctx.line(lx, gy, lx, gy + gh, color="#44446688", width=1.5)

        # Playhead column
        if self.playing:
            beat_col = int(self.current_beat) % NUM_STEPS
            px = gx + beat_col * STEP_PITCH + CELL_W / 2.0
            ctx.line(px, gy, px, gy + gh, color=_hex_alpha(ACCENT_BEAT, 40), width=2.0)

    # ── Physics balls ─────────────────────────────────────────────────────────

    def _advance_and_draw_balls(self, ctx: RenderContext, W: float, H: float) -> None:
        gx, gy, gw, gh = self._grid_rect(W, H)

        for ball in self.balls:
            # Advance
            ball["x"] += ball["vx"]
            ball["y"] += ball["vy"]

            # Bounce off grid walls
            r: float = ball["r"]
            if ball["x"] - r < gx:
                ball["x"] = gx + r
                ball["vx"] = abs(ball["vx"])
            elif ball["x"] + r > gx + gw:
                ball["x"] = gx + gw - r
                ball["vx"] = -abs(ball["vx"])
            if ball["y"] - r < gy:
                ball["y"] = gy + r
                ball["vy"] = abs(ball["vy"])
            elif ball["y"] + r > gy + gh:
                ball["y"] = gy + gh - r
                ball["vy"] = -abs(ball["vy"])

            # Draw: outer halo (20% alpha), inner solid
            color: str = ball["color"]
            halo_r = r * 2.8
            ctx.circle(ball["x"], ball["y"], halo_r, fill=_hex_alpha(color, 20))
            ctx.circle(ball["x"], ball["y"], r, fill=_hex_alpha(color, 90))
            # Bright core highlight
            ctx.circle(ball["x"] - r * 0.25, ball["y"] - r * 0.25, r * 0.3, fill=_hex_alpha("#ffffff", 60))

    # ── Arrangement strip ─────────────────────────────────────────────────────

    def _draw_arrangement(self, ctx: RenderContext, W: float, H: float) -> None:
        ay = H - ARRANGE_H
        ctx.rect(0, ay, W, ARRANGE_H, fill=PANEL_BG)
        ctx.line(0, ay, W, ay, color=BORDER, width=1.0)

        # Section label
        ctx.text(CHANNEL_W / 2.0, ay + ARRANGE_H / 2.0 - 5, "ARRANGE", size=9.0, color=TEXT_DIM, bold=True)
        ctx.text(CHANNEL_W / 2.0, ay + ARRANGE_H / 2.0 + 8, "32 BARS", size=8.0, color=TEXT_DIM)
        ctx.line(CHANNEL_W, ay, CHANNEL_W, H, color=BORDER, width=1.0)

        bar_area_x = CHANNEL_W + 8.0
        bar_area_w = W - bar_area_x - 8.0
        bar_pitch = bar_area_w / NUM_BARS
        bar_h = ARRANGE_H - 20.0
        bar_y = ay + 10.0

        for i in range(NUM_BARS):
            bx = bar_area_x + i * bar_pitch
            bw = bar_pitch - 2.0

            is_active = i == self.current_bar
            has_content = self.arrangement[i]

            if is_active:
                fill = "#334466"
                border_col = ACCENT_BEAT
            elif has_content:
                fill = "#223344"
                border_col = "#4488aa"
            else:
                fill = "#151520"
                border_col = BORDER

            ctx.rect(bx, bar_y, bw, bar_h, fill=fill, radius=2.0)
            ctx.line(bx, bar_y, bx + bw, bar_y, color=border_col, width=1.0)
            ctx.line(bx, bar_y + bar_h, bx + bw, bar_y + bar_h, color=border_col, width=1.0)

            # Bar number label every 4
            if i % 4 == 0:
                ctx.text(bx + bw / 2.0, bar_y + bar_h / 2.0 - 4, str(i + 1), size=8.0, color=TEXT_DIM)

        # Progress marker (current_bar position)
        marker_x = bar_area_x + self.current_bar * bar_pitch + bar_pitch / 2.0
        ctx.circle(marker_x, bar_y - 4.0, 3.0, fill=ACCENT_BEAT)


if __name__ == "__main__":
    ParallaxDAW().run()
