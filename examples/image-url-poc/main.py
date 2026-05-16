#!/usr/bin/env python3
"""POC for remote URL image loading (#1353).

Shows three images:
  - Left:   a real https:// URL (should fetch and render)
  - Center: https:// URL with a bad path (should show error placeholder)
  - Right:  a label confirming behaviour
"""
from plexi_sdk import App, RenderContext, BG  # type: ignore[attr-defined]

REAL_URL = "https://picsum.photos/id/237/200/200"
BAD_URL = "https://picsum.photos/nonexistent/404/image.jpg"


class ImageUrlPoc(App):
    def on_render(self, ctx: RenderContext) -> None:
        ctx.clear(BG)

        ctx.text(20, 10, "Remote URL image loading (#1353)", size=14, color="#ffffff",
                 align="top_left", elide=False, selectable=False)

        ctx.image(REAL_URL, 20, 40, 200, 200)
        ctx.text(20, 250, "real URL — should render", size=11, color="#44cc44",
                 align="top_left", elide=False, selectable=False)

        ctx.image(BAD_URL, 240, 40, 200, 200)
        ctx.text(240, 250, "bad URL — should show error", size=11, color="#cc4444",
                 align="top_left", elide=False, selectable=False)


if __name__ == "__main__":
    ImageUrlPoc().run()
