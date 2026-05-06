#!/usr/bin/env python3
"""Speak Tester — POC for emit.speak() TTS primitive.

Press Enter to fire a speak command. Watch plexi.log for TTS latency.
"""
from plexi_sdk import App, RenderContext


class SpeakTester(App):
    def __init__(self) -> None:
        super().__init__()
        self._count = 0

    def on_render(self, ctx: RenderContext) -> None:
        ctx.text(10, 20, "Speak Tester", size=16, color="#cdd6f4", bold=True)
        ctx.text(10, 50, f"Press Enter to speak. Count: {self._count}", size=13, color="#a6adc8")
        ctx.text(10, 75, "Requires GOOGLE_API_KEY to be set.", size=12, color="#6c7086")

    def on_key(self, _ctx: RenderContext, key: str, _mods: dict) -> None:
        if key == "Return":
            self._count += 1
            self.emit.speak(f"Hello from Plexi. This is message {self._count}.")
            self.emit.info(f"speak fired: count={self._count}")


SpeakTester().run()
