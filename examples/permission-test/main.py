#!/usr/bin/env python3
"""POC app for first-run capability consent (#1455).

Declares panes.spawn and ai.query — both sensitive capabilities.
On first launch neither is pre-granted; this app requests each via
capability_request so the host consent modal is triggered.

Expected first-run flow:
  1. App launches, shows "Not yet granted" for both capabilities.
  2. User clicks "Request panes.spawn" → consent modal appears.
  3. User grants → status updates to "Granted".
  4. Same for ai.query.
  5. On second launch, both show "Granted" immediately (stored Green).
"""
from plexi_sdk import (
    App, RenderContext,
    BG, SURFACE, FG, ACCENT, MUTED, GREEN,
    CAPTION, HEADING, HINT,
    PAD,
)
from plexi_sdk.widgets import Button, ButtonStyle


class PermissionTest(App):
    async def on_init(self, _ctx: RenderContext) -> None:
        self.panes_spawn_state = "not granted"
        self.ai_query_state = "not granted"
        self.requesting = False
        self.emit.info("permission-test: initialized")

        btn_style = ButtonStyle(
            fill=SURFACE, hover_fill=ACCENT, active_fill=ACCENT,
            text_color=FG, font_size=CAPTION, radius=4.0,
        )
        self._btn_spawn = Button(
            "req-spawn", x=PAD, y=0, w=240, h=34,
            label="[ Request panes.spawn ]", style=btn_style,
        )
        self._btn_ai = Button(
            "req-ai", x=PAD, y=0, w=240, h=34,
            label="[ Request ai.query ]", style=btn_style,
        )

    def on_render(self, ctx: RenderContext) -> None:
        ctx.clear(BG)

        ctx.text(PAD, PAD, "Permission Consent Test", size=HEADING, color=ACCENT, bold=True)
        ctx.text(PAD, PAD + HEADING + 4,
                 "First launch: consent modal fires. Second launch: auto-granted.",
                 size=HINT, color=MUTED)

        y = PAD + HEADING + HINT + PAD * 2

        # panes.spawn row
        state_color = GREEN if self.panes_spawn_state == "granted" else MUTED
        ctx.text(PAD, y, f"panes.spawn: {self.panes_spawn_state}", size=CAPTION, color=state_color)
        y += CAPTION + 6
        self._btn_spawn.y = y
        if not self.requesting and self._btn_spawn.render(ctx):
            self.requesting = True
            self.emit.schedule_task(self._request("panes.spawn"))
        y += self._btn_spawn.h + PAD

        # ai.query row
        state_color = GREEN if self.ai_query_state == "granted" else MUTED
        ctx.text(PAD, y, f"ai.query:    {self.ai_query_state}", size=CAPTION, color=state_color)
        y += CAPTION + 6
        self._btn_ai.y = y
        if not self.requesting and self._btn_ai.render(ctx):
            self.requesting = True
            self.emit.schedule_task(self._request("ai.query"))

    async def _request(self, cap: str) -> None:
        self.emit.info(f"permission-test: requesting {cap}")
        granted = await self.emit.capability_request(cap)
        if cap == "panes.spawn":
            self.panes_spawn_state = "granted" if granted else "denied"
        else:
            self.ai_query_state = "granted" if granted else "denied"
        self.emit.info(f"permission-test: {cap} -> {'granted' if granted else 'denied'}")
        self.requesting = False


PermissionTest().run()
