#!/usr/bin/env python3
"""POC for RenderCommand::Image (#1144).

Shows two images side by side:
  - Left:  logo.png (real image, must exist next to this script)
  - Right: missing.png (non-existent — tests the placeholder / error state)
"""

from plexi_sdk import App, RenderContext, BG  # type: ignore[attr-defined]


class ImageRenderPoc(App):
    def on_render(self, ctx: RenderContext) -> None:
        ctx.clear(BG)

        # Left: real image (contain fit)
        ctx.image("logo.png", 20, 20, 200, 200)
        ctx.text(20, 230, "logo.png (contain)", size=12, color="#888888",
                 align="top_left", elide=False, selectable=False)

        # Right: missing image — should show placeholder with error text
        ctx.image("missing.png", 240, 20, 200, 200)
        ctx.text(240, 230, "missing.png (placeholder)", size=12, color="#888888",
                 align="top_left", elide=False, selectable=False)


if __name__ == "__main__":
    ImageRenderPoc().run()
