#!/usr/bin/env python3
"""scroll-list — POC for host-managed scroll regions (#446).

Renders 100 rows using DrawCommand::BeginScroll / EndScroll. The host owns
the scroll offset; this app just receives ScrollOffset events via on_scroll
and re-renders the list at the updated position.

Verification checklist:
  1. Launch the app — a list of 100 items should appear, most off-screen.
  2. Hover over the list and scroll with the mouse wheel — items should scroll.
  3. Items outside the declared viewport height must be clipped (not visible).
  4. The host log should show no "clip stack not empty" warnings.
"""

import sys
import os

# Prefer the SDK from the repo tree over any installed version.
_sdk_path = os.path.join(os.path.dirname(__file__), "..", "..", "sdk", "python")
if os.path.isdir(_sdk_path):
    sys.path.insert(0, _sdk_path)

from plexi_sdk import App, BG, SURFACE, HIGHLIGHT, FG, MUTED, ACCENT, BODY, CAPTION, PAD

ITEM_COUNT = 100
ROW_H = 36.0
HEADER_H = 48.0
SCROLL_ID = "main-list"


class ScrollListApp(App):
    def on_init(self, ctx):
        self.scroll_y = 0.0

    def on_scroll(self, ctx, id, offset_y):
        if id == SCROLL_ID:
            self.scroll_y = offset_y

    def on_render(self, ctx):
        # Background
        ctx.clear(BG)

        # Header bar
        ctx.rect(0, 0, ctx.w, HEADER_H, fill=SURFACE)
        ctx.text(PAD, HEADER_H / 2, "Host-Managed Scroll — 100 Items",
                 size=BODY, color=FG, align="left_center")
        offset_label = f"offset: {self.scroll_y:.1f}px"
        ctx.text(ctx.w - PAD, HEADER_H / 2, offset_label,
                 size=CAPTION, color=MUTED, align="right_center")

        # Total content height for all 100 rows
        content_height = ITEM_COUNT * ROW_H

        # Scroll viewport fills the pane below the header
        viewport_y = HEADER_H
        viewport_h = ctx.h - HEADER_H

        ctx.begin_scroll(SCROLL_ID, 0, viewport_y, ctx.w, viewport_h,
                         content_height=content_height)

        # Render only the rows that are (potentially) visible to avoid
        # painting thousands of off-screen items. This is optional —
        # the host will clip anything outside the viewport — but it
        # keeps the draw-command stream lean.
        first_visible = max(0, int(self.scroll_y / ROW_H))
        last_visible = min(ITEM_COUNT, int((self.scroll_y + viewport_h) / ROW_H) + 2)

        for i in range(first_visible, last_visible):
            row_y = viewport_y + i * ROW_H - self.scroll_y
            is_even = i % 2 == 0
            bg = SURFACE if is_even else BG
            ctx.rect(0, row_y, ctx.w, ROW_H, fill=bg)

            # Row number badge
            badge_label = f"{i + 1:>3}"
            ctx.text(PAD, row_y + ROW_H / 2, badge_label,
                     size=CAPTION, color=ACCENT, monospace=True, align="left_center")

            # Item label
            ctx.text(PAD + 40, row_y + ROW_H / 2,
                     f"Item {i + 1:03d} — host scroll region",
                     size=BODY, color=FG, align="left_center")

            # Right-side hint
            ctx.text(ctx.w - PAD, row_y + ROW_H / 2,
                     f"y={i * ROW_H:.0f}",
                     size=CAPTION, color=MUTED, align="right_center")

            # Separator line
            ctx.line(0, row_y + ROW_H - 1, ctx.w, row_y + ROW_H - 1,
                     color=HIGHLIGHT, width=0.5)

        ctx.end_scroll()

        # Footer hint
        footer_y = ctx.h - 28
        ctx.text(ctx.w / 2, footer_y,
                 "Scroll with mouse wheel  •  100 items  •  host clips viewport",
                 size=CAPTION, color=MUTED, align="center")


if __name__ == "__main__":
    ScrollListApp().run()
