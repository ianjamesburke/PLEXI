#!/usr/bin/env python3
"""Clipboard Demo — exercises #200 (paste + selection) and #146 (clipboard write).

Three things on screen:
  1. A header label "clipboard demo".
  2. A multi-paragraph block of selectable text. Drag to select, Cmd+C to copy.
  3. A button area: press `c` to copy the current time to the clipboard.
  4. A status line showing the most recent paste payload.

Cmd+V (or right-click → Paste) anywhere in the pane fires `on_paste`.
"""

import time

from plexi_sdk import App, RenderContext, BG, FG, ACCENT, MUTED, SURFACE, HIGHLIGHT


SAMPLE_TEXT = (
    "Lorem ipsum dolor sit amet, consectetur adipiscing elit. "
    "Sed do eiusmod tempor incididunt ut labore et dolore magna aliqua.\n\n"
    "Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris "
    "nisi ut aliquip ex ea commodo consequat.\n\n"
    "Duis aute irure dolor in reprehenderit in voluptate velit esse cillum "
    "dolore eu fugiat nulla pariatur."
)


class ClipboardTestApp(App):
    def on_init(self, ctx: RenderContext) -> None:
        self._last_paste: str = ""
        self._copy_log: str = ""
        ctx.status_summary("Clipboard Demo — ready")
        self.emit.info("clipboard-test started")

    def on_render(self, ctx: RenderContext) -> None:
        w, h = ctx.w, ctx.h

        # Background.
        ctx.rect(0, 0, w, h, BG)

        # Header.
        ctx.text(
            x=24, y=24, text="clipboard demo",
            size=22.0, color=FG, bold=True,
        )

        # Subtitle / instructions.
        ctx.text(
            x=24, y=58, text="drag to select  ·  Cmd+C copies  ·  Cmd+V pastes  ·  press `c` to copy current time",
            size=12.0, color=MUTED,
        )

        # Selectable region: card background + selectable text.
        card_x, card_y = 24, 92
        card_w = max(w - 48, 100)
        card_h = 200
        ctx.rect(card_x, card_y, card_w, card_h, SURFACE, radius=6.0)
        ctx.text(
            x=card_x + 16, y=card_y + 16,
            text=SAMPLE_TEXT,
            size=14.0, color=FG,
            max_width=card_w - 32,
            elide=False,
            selectable=True,
        )

        # Copy-button strip.
        btn_y = card_y + card_h + 16
        ctx.rect(card_x, btn_y, card_w, 44, HIGHLIGHT, radius=6.0)
        btn_label = self._copy_log or "press `c` to copy current time to clipboard"
        ctx.text(
            x=card_x + card_w / 2, y=btn_y + 22,
            text=btn_label,
            size=13.0, color=ACCENT,
            align="center",
        )

        # Status line: most recent paste.
        status_y = btn_y + 60
        ctx.text(
            x=24, y=status_y, text="Last paste received:",
            size=12.0, color=MUTED,
        )
        paste_display = self._last_paste if self._last_paste else "(none yet — try Cmd+V)"
        ctx.text(
            x=24, y=status_y + 18, text=paste_display,
            size=14.0, color=FG, monospace=True,
            max_width=w - 48, elide=True,
        )

    def on_key(self, ctx: RenderContext, key: str, mods: dict) -> None:
        # Press `c` (no modifier) to copy the current time to the clipboard.
        if key == "c" and not any(mods.values()):
            now = time.strftime("%Y-%m-%d %H:%M:%S")
            payload = f"copied at {now}"
            self.emit.copy_to_clipboard(payload)
            self._copy_log = f"copied: {payload}"
            self.emit.info(f"clipboard-test: copy_to_clipboard({payload!r})")
            self.emit.schedule_render(after_ms=16)

    def on_paste(self, ctx: RenderContext, text: str) -> None:
        self._last_paste = text
        self.emit.info(f"clipboard-test: on_paste len={len(text)}")
        self.emit.schedule_render(after_ms=16)


if __name__ == "__main__":
    ClipboardTestApp().run()
