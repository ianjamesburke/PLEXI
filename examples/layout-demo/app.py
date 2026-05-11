#!/usr/bin/env python3
"""layout-demo — POC for the declarative layout model (issue #1080).

Renders a row of [badge + text] pairs with varying label lengths to verify
that host-side flexbox correctly spaces them without hard-coded offsets.
"""
from plexi_sdk import App, RenderContext, FG, BG, ACCENT, BODY, CAPTION  # type: ignore[attr-defined]

PAIRS = [
    ("1 file", "modified"),
    ("42 files", "staged"),
    ("3 files", "untracked"),
]


class LayoutDemoApp(App):
    def on_render(self, ctx: RenderContext) -> None:
        ctx.clear(BG)
        ctx.text(10, 10, "Layout Demo — host-side flexbox (issue #1080)", size=BODY, color=FG,
                 align="top_left", elide=False, selectable=False)

        y = 40.0
        for label, desc in PAIRS:
            ctx.row(
                x=10.0, y=y,
                children=[
                    ctx.badge_child(label, fill=ACCENT, fg=BG),
                    ctx.text_child(f"  {desc}", size=BODY, color=FG),
                ],
                gap=6.0,
            )
            y += 28.0

        # Also demonstrate a column
        ctx.text(10, y + 10, "Column:", size=CAPTION, color=FG,
                 align="top_left", elide=False, selectable=False)
        ctx.column(
            x=10.0, y=y + 28.0,
            children=[ctx.badge_child(lbl) for lbl, _ in PAIRS],
            gap=4.0,
        )


if __name__ == "__main__":
    LayoutDemoApp().run()
