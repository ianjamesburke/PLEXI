"""POC: async image loading via emit.load_image() — issue #1354."""
from plexi_sdk import App, RenderContext

class ImageLoaderApp(App):
    def __init__(self):
        super().__init__()
        self._handle: str | None = None
        self._error: str | None = None
        self._loading = False

    async def on_init(self, _ctx: RenderContext) -> None:
        self.emit.info("image-loader: starting")
        self.emit.schedule_task(self._fetch())

    async def _fetch(self) -> None:
        self._loading = True
        try:
            self._handle = await self.emit.load_image(
                "https://picsum.photos/400/300"
            )
            self.emit.info(f"image-loader: loaded handle={self._handle}")
        except RuntimeError as e:
            self._error = str(e)
            self.emit.info(f"image-loader: error={self._error}")
        self._loading = False

    def on_render(self, ctx: RenderContext) -> None:
        w, h = ctx.width, ctx.height
        ctx.rect(0, 0, w, h, fill="#1e1e2e")
        ctx.text(20, 20, "Async Image Loader POC (#1354)", size=16, color="#cdd6f4")

        if self._error:
            ctx.text(20, 60, f"Error: {self._error}", size=13, color="#f38ba8")
        elif self._handle:
            ctx.image(self._handle, 20, 60, w - 40, h - 80, fit="contain")
        else:
            ctx.text(20, 60, "Loading…", size=13, color="#a6adc8")

if __name__ == "__main__":
    ImageLoaderApp().run()
