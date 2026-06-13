from plexi_sdk import App, RenderContext
from plexi_sdk.widgets import KeyMap


class KeyMapProbe(App):
    def on_init(self) -> None:
        self._last_action = "none"
        self._km = KeyMap()
        self._km.bind("z", "bare-z")
        self._km.bind("z", "ctrl-z", mod="ctrl")
        self._km.bind("return", "submit")

    def on_render(self, ctx: RenderContext) -> None:
        ctx.text(10, 10, f"last_action={self._last_action}", size=14, color="#ffffff",
                 selectable=False)
        ctx.status_summary(f"last_action={self._last_action}")

    def on_key(self, key: str, mods: dict) -> None:
        action = self._km.handle(key, mods)
        if action is not None:
            self._last_action = action
            self.emit.schedule_render()


if __name__ == "__main__":
    KeyMapProbe().run()
