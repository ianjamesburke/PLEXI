#!/usr/bin/env python3
"""Spawn Pane POC — demonstrates from_pane_id + request_id correlation (#752).

Step 1: Click "Spawn snake (split_h)" — spawns a snake pane to the right with
        request_id="req-1". Log shows the echoed request_id.
Step 2: Click "Spawn from first pane (split_v)" — splits relative to the
        first snake pane (not this coordinator) using from_pane_id + request_id="req-2".
"""
from __future__ import annotations

from plexi_sdk import App, RenderContext, FG, MUTED, SURFACE, ACCENT, BG, BODY, CAPTION  # type: ignore[attr-defined]

PAD = 20.0
BTN_W = 260.0
BTN_H = 36.0


class SpawnPanePoc(App):
    def on_init(self, ctx: RenderContext) -> None:
        self._log: list[str] = []
        self._first_pane_id: int | None = None
        self._btn1_y = 0.0
        self._btn2_y = 0.0

    def on_render(self, ctx: RenderContext) -> None:
        ctx.clear(BG)
        pad = PAD
        y = pad

        ctx.text(pad, y, "Spawn Pane POC (#752)", size=16.0, color=FG, bold=True)
        y += 32.0
        ctx.text(pad, y, "Tests from_pane_id + request_id correlation.", size=BODY, color=MUTED)
        y += 28.0

        # Button 1 — spawn snake to the right, capture pane_id
        ctx.rect(pad, y, BTN_W, BTN_H, fill=ACCENT, radius=6.0)
        ctx.text(pad + 12, y + 10, "1. Spawn snake (split_h, req-1)", size=BODY, color=BG)
        self._btn1_y = y
        y += BTN_H + 12.0

        # Button 2 — spawn below first pane using from_pane_id
        has_first = self._first_pane_id is not None
        btn2_fill = SURFACE if has_first else MUTED
        label2 = f"2. Spawn below pane {self._first_pane_id} (req-2)" if has_first else "2. (spawn step 1 first)"
        ctx.rect(pad, y, BTN_W, BTN_H, fill=btn2_fill, radius=6.0)
        ctx.text(pad + 12, y + 10, label2, size=BODY, color=FG)
        self._btn2_y = y
        y += BTN_H + 20.0

        ctx.rect(pad, y, ctx.w - pad * 2, 1, fill=SURFACE, radius=0.0)
        y += 12.0
        ctx.text(pad, y, "Log:", size=CAPTION, color=MUTED)
        y += 20.0
        for line in self._log[-8:]:
            ctx.text(pad + 8, y, line, size=CAPTION, color=FG, monospace=True)
            y += 18.0

    def on_click(self, ctx: RenderContext, x: float, y: float, button: str) -> None:
        if button != "primary":
            return
        if PAD <= x <= PAD + BTN_W:
            if self._btn1_y <= y <= self._btn1_y + BTN_H:
                ctx.emit.spawn_pane("snake", layout="split_h", request_id="req-1")
                self._log.append("→ spawn snake split_h request_id=req-1")
            elif self._btn2_y <= y <= self._btn2_y + BTN_H and self._first_pane_id is not None:
                ctx.emit.spawn_pane(
                    "snake",
                    layout="split_v",
                    from_pane_id=self._first_pane_id,
                    request_id="req-2",
                )
                self._log.append(f"→ spawn snake split_v from_pane_id={self._first_pane_id} request_id=req-2")

    def on_pane_spawned(self, pane_id: int, request_id: str | None = None) -> None:
        self._log.append(f"✓ pane_id={pane_id} request_id={request_id}")
        if self._first_pane_id is None:
            self._first_pane_id = pane_id

    def on_pane_spawn_error(self, reason: str, request_id: str | None = None) -> None:
        self._log.append(f"✗ error={reason!r} request_id={request_id}")


if __name__ == "__main__":
    SpawnPanePoc().run()
