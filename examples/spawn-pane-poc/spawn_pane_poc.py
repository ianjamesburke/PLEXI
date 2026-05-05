#!/usr/bin/env python3
"""Spawn Pane POC — demonstrates DrawCommand::SpawnPane (#527).

Two buttons: one spawns a terminal pane to the right, one spawns the snake
app. The pane_id returned by PaneSpawned is displayed for verification.
"""
from __future__ import annotations

from plexi_sdk import App, RenderContext, FG, MUTED, SURFACE, ACCENT, BG, BODY, CAPTION  # type: ignore[attr-defined]

PAD = 20.0
BTN_W = 220.0
BTN_H = 36.0


class SpawnPanePoc(App):
    def on_init(self, ctx: RenderContext) -> None:
        self._log: list[str] = []

    def on_render(self, ctx: RenderContext) -> None:
        ctx.clear(BG)
        pad = PAD
        y = pad

        ctx.text(pad, y, "Spawn Pane POC", size=16.0, color=FG, bold=True)
        y += 32.0
        ctx.text(pad, y, "Click a button to spawn a pane via panes.spawn.", size=BODY, color=MUTED)
        y += 28.0

        # Button 1 — terminal
        ctx.rect(pad, y, BTN_W, BTN_H, fill=ACCENT, radius=6.0)
        ctx.text(pad + 12, y + 10, "Spawn terminal (split_h)", size=BODY, color=BG)
        self._btn1_y = y
        y += BTN_H + 12.0

        # Button 2 — snake app
        ctx.rect(pad, y, BTN_W, BTN_H, fill=SURFACE, radius=6.0)
        ctx.text(pad + 12, y + 10, "Spawn snake (split_v)", size=BODY, color=FG)
        self._btn2_y = y
        y += BTN_H + 20.0

        ctx.rect(pad, y, ctx.w - pad * 2, 1, fill=SURFACE, radius=0.0)
        y += 12.0
        ctx.text(pad, y, "Log:", size=CAPTION, color=MUTED)
        y += 20.0
        for line in self._log[-6:]:
            ctx.text(pad + 8, y, line, size=CAPTION, color=FG, monospace=True)
            y += 18.0

    def on_click(self, ctx: RenderContext, x: float, y: float, button: str) -> None:
        if button != "primary":
            return
        if PAD <= x <= PAD + BTN_W:
            if self._btn1_y <= y <= self._btn1_y + BTN_H:
                ctx.emit.spawn_pane("terminal", layout="split_h")
                self._log.append("→ spawn_pane terminal split_h")
            elif self._btn2_y <= y <= self._btn2_y + BTN_H:
                ctx.emit.spawn_pane("snake", layout="split_v")
                self._log.append("→ spawn_pane snake split_v")

    def on_pane_spawned(self, pane_id: int) -> None:
        self._log.append(f"✓ pane_spawned pane_id={pane_id}")

    def on_pane_spawn_error(self, reason: str) -> None:
        self._log.append(f"✗ pane_spawn_error: {reason}")


if __name__ == "__main__":
    SpawnPanePoc().run()
