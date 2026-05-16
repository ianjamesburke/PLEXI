#!/usr/bin/env python3
from plexi_sdk import (
    App, RenderContext,
    BG, SURFACE, FG, ACCENT, MUTED, GREEN,
    CAPTION, HEADING, HINT,
    PAD, PAD_TIGHT,
)
from plexi_sdk.widgets import Button, ButtonStyle


class AiTest(App):
    async def on_init(self, _ctx: RenderContext) -> None:
        self.response = ""
        self.status = "Press the button to query AI"
        self.waiting = False
        self.tokens_in = 0
        self.tokens_out = 0
        self.emit.info("ai-test initialized")

        self._btn = Button(
            "ask", x=PAD, y=0, w=200, h=34, label="[ Ask AI something ]",
            style=ButtonStyle(
                fill=SURFACE, hover_fill=ACCENT, active_fill=ACCENT,
                text_color=FG, font_size=CAPTION, radius=4.0,
            ),
        )

    def on_render(self, ctx: RenderContext) -> None:
        ctx.clear(BG)

        ctx.text(PAD, PAD, "AI Query Test", size=HEADING, color=ACCENT, bold=True)
        ctx.text(PAD, PAD + HEADING + PAD_TIGHT, self.status, size=CAPTION, color=MUTED)

        self._btn.y = PAD + HEADING + CAPTION + PAD * 2
        if self._btn.render(ctx):
            if not self.waiting:
                self.waiting = True
                self.status = "Querying AI (low tier / Gemini Flash)..."
                self.response = ""
                self.emit.info("button clicked, dispatching ai_query")
                self.emit.schedule_task(self._do_query())

        text_y = self._btn.y + self._btn.h + PAD
        if self.response:
            ctx.text(PAD, text_y, f"tokens in: {self.tokens_in}  out: {self.tokens_out}",
                     size=HINT, color=GREEN, monospace=True)
            ctx.markdown(PAD, text_y + HINT + PAD_TIGHT,
                         ctx.w - PAD * 2, self.response)

    async def _do_query(self) -> None:
        try:
            resp = await self.emit.ai_query(
                model_tier="low",
                system="You are a helpful assistant. Keep answers under 2 sentences.",
                messages=[{"role": "user", "content": "What is the coolest thing about terminal-based apps?"}],
            )
            self.response = resp.content or "(empty response)"
            self.tokens_in = resp.tokens_in
            self.tokens_out = resp.tokens_out
            self.status = "Done!"
            self.emit.info(f"ai_query success: {resp.tokens_in} in, {resp.tokens_out} out")
        except Exception as e:
            self.response = f"Error: {e}"
            self.status = "Failed"
            self.emit.error(f"ai_query failed: {e}")
        finally:
            self.waiting = False


AiTest().run()
